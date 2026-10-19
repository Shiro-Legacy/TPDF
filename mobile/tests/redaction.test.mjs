// The mobile Redact tool's logic (src/redact.ts) and the engine wire it uses: one in-memory box over the visible
// secret of fixtures/redact.pdf, inspected and applied as `regions`, with the copy read back by the mupdf npm API.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { build } from 'esbuild';
import { rustEngine } from './rust.mjs';

const bundled = await build({ entryPoints: ['src/redact.ts'], bundle: true, format: 'esm', write: false });
const redact = await import(`data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].text).toString('base64')}`);
const { Boxes, boxAt, boxFrom, redactedName, flattenQuestion, refusalMessage, SANITIZE_LABELS, pagesNamed } = redact;
const SECRET = 'P0B-SECRET-7731', BOX = [66, 34, 300, 60];

test('a drag makes a box clamped to the page; a tap makes none', () => {
  assert.deepEqual(boxFrom([300, 60], [66, 34], 612, 792), [66, 34, 300, 60]);
  assert.deepEqual(boxFrom([-20, -5], [700, 900], 612, 792), [0, 0, 612, 792]);
  assert.equal(boxFrom([10, 10], [12, 40], 612, 792), null);
});

test('boxes undo and redo adding and removing, in place', () => {
  const boxes = new Boxes();
  const a = boxes.add(0, [0, 0, 10, 10]), b = boxes.add(1, [0, 0, 20, 20]), c = boxes.add(0, [30, 30, 50, 50]);
  assert.equal(boxes.count, 3);
  assert.ok(boxes.remove(b.id));
  assert.deepEqual(boxes.all.map(x => x.id), [a.id, c.id]);
  assert.equal(boxes.undo().id, b.id);
  assert.deepEqual(boxes.all.map(x => x.id), [a.id, b.id, c.id], 'an undone removal goes back where it was');
  assert.equal(boxes.undo().id, c.id); assert.equal(boxes.count, 2);
  assert.equal(boxes.redo().id, c.id); assert.equal(boxes.count, 3);
  assert.equal(boxes.canRedo, true, 'the undone removal can be redone too');
  boxes.add(2, [0, 0, 5, 5]); assert.equal(boxes.canRedo, false, 'a new box clears redo');
  assert.deepEqual(boxes.byPage().map(g => [g.page, g.boxes.length]), [[0, 2], [1, 1], [2, 1]]);
  assert.deepEqual(boxes.regions()[0], { page: 0, rect: [0, 0, 10, 10] });
  assert.equal(boxAt(boxes.all, 0, [40, 40]).id, c.id);
  assert.equal(boxAt(boxes.all, 0, [12, 12]), null);
  assert.equal(boxAt(boxes.all, 0, [12, 12], 4).id, a.id, 'a fingertip slop reaches a box edge');
  boxes.clear(); assert.equal(boxes.count, 0); assert.equal(boxes.canUndo, false);
});

test('the copy gets a new name and never takes an existing one', () => {
  assert.equal(redactedName('contract.pdf', []), 'contract redacted.pdf');
  assert.equal(redactedName('contract.PDF', ['contract redacted.pdf']), 'contract redacted 2.pdf');
  assert.equal(redactedName('.pdf', []), 'Document redacted.pdf');
});

test('the words are plain and never the engine message', () => {
  assert.equal(SANITIZE_LABELS.length, 9);
  assert.deepEqual(SANITIZE_LABELS.map(([k]) => k).sort(), ['annots', 'attachments', 'fields', 'info', 'layers', 'links', 'outlines', 'struct', 'xmp']);
  assert.equal(pagesNamed([3, 1, 1]), 'pages 2 and 4');
  assert.deepEqual(flattenQuestion([3]), {
    title: "Page 4 contains artwork that can't be cut precisely.",
    text: 'Redact page 4 by turning it into an image? Its text will no longer be selectable.',
    action: 'Turn page 4 into an image',
  });
  assert.match(flattenQuestion([0, 2]).text, /^Redact pages 1 and 3 by turning them into images\? Their text/);
  for (const reason of ['verify-failed', 'too-large', 'too-many', 'scan-budget', 'invalid', 'read-only', 'shared-content', 'no-marks', 'other']) {
    const { title, text } = refusalMessage(reason, false);
    assert.ok(title && text);
    assert.doesNotMatch(`${title} ${text}`, /MiB|refus|redact-|regions|verify-failed|\bop\b/);
  }
  assert.match(refusalMessage('verify-failed', false).text, /Nothing was saved/);
});

test('one box over the visible secret: inspect shows it, apply removes it, the original stays', async () => {
  const bytes = await readFile('../fixtures/redact.pdf');
  const engine = rustEngine(), session = engine.createSession(); let id = 0;
  const send = request => engine.dispatch({ ...request, id: ++id }, session).reply;
  assert.equal(send({ op: 'open', pdf: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength), name: 'redact.pdf' }).ok, true);
  const inspected = send({ op: 'redact-inspect', regions: [{ page: 0, rect: BOX }] });
  assert.equal(inspected.ok, true, JSON.stringify(inspected));
  assert.match(inspected.regions[0].text, new RegExp(SECRET));
  assert.ok(inspected.regions[0].glyphs.length > 0);
  const sanitize = Object.fromEntries(SANITIZE_LABELS.map(([k]) => [k, true]));
  const applied = send({ op: 'redact-apply', sanitize, regions: [{ page: 0, rect: BOX }] });
  assert.equal(applied.ok, true, JSON.stringify({ ...applied, bytes: undefined }));
  assert.equal(applied.marks, 1);
  const mupdf = await import('mupdf');
  const lines = pdf => JSON.parse(mupdf.Document.openDocument(pdf, 'application/pdf').loadPage(0).toStructuredText().asJSON())
    .blocks.flatMap(b => b.lines ?? []).filter(l => l.bbox.y < BOX[3] && l.bbox.y + l.bbox.h > BOX[1] && l.bbox.x < BOX[2]).map(l => l.text);
  assert.ok(lines(bytes).some(t => t.includes(SECRET)));
  assert.ok(!lines(new Uint8Array(applied.bytes)).some(t => t.includes(SECRET)), 'the secret is gone from the boxed line of the copy');
  // Nothing is written to the session: no marks, nothing to undo.
  const preview = send({ op: 'redact-preview' });
  assert.equal(preview.marks, 0);
});
