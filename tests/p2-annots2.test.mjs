/**
 * P2 unit 9 §A — Square, Circle, Line, Ink and Stamp annotations, plus the
 * signature appearance. Runs against the app engine (tests/rust-engine.mjs) exactly as the
 * worker does.
 *
 * The unit's gate (`.stage/gate-u9.mjs`, table in `p2-annots2.md`): `update()`
 * synthesises `/AP` for all six shapes on MuPDF 1.28.1 wasm, each is one journal
 * step, and `setStampImage` keeps the caller's rect while `setIcon` resizes it
 * to the icon's own aspect around the same centre. Line and Ink carry no `/Rect`
 * in this build, so their box is read from `getLine()`/`getInkList()`.
 */
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import mupdf from 'mupdf';
import {
  createEngine, MAX_INK_POINTS, MAX_INK_STROKES, MAX_SIGN_NAME_CHARS, MAX_STAMP_IMAGE_BYTES,
  SIGNATURE_SUBJECT,
} from './rust-engine.mjs';
import { assertWithin } from './timing-budget.mjs';

const engine = createEngine(mupdf);

function toArrayBuffer(buffer) {
  return buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength);
}

async function open(id = 'hello', session = engine.createSession()) {
  const bytes = await readFile(`fixtures/${id}.pdf`);
  const reply = engine.dispatch({ id: 1, op: 'open', name: `${id}.pdf`, pdf: toArrayBuffer(bytes) }, session).reply;
  assert.equal(reply.ok, true, reply.error);
  return session;
}

function op(session, request) {
  return engine.dispatch({ id: 900 + (request.id ?? 0), ...request }, session).reply;
}

function items(session, page = 0) {
  const reply = op(session, { op: 'annots', page });
  assert.equal(reply.ok, true, reply.error);
  return reply.items;
}

/** The live annotations on the page, by object number; the page stays open for the caller. */
function liveAnnots(session, pageIndex = 0) {
  const page = session.doc.loadPage(pageIndex);
  const map = new Map(page.getAnnotations().map(annot => [annot.getObject().asIndirect(), annot]));
  return { map, close: () => page.destroy() };
}

const GRAY_IMAGE = toArrayBuffer(await readFile('fixtures/img-gray.png'));

/** One valid add per new type, in the order the shapes smoke uses them. */
function validAdds(image = GRAY_IMAGE) {
  return [
    ['add square', { op: 'annot-add', page: 0, type: 'Square', rect: [100, 100, 260, 200], contents: '' }],
    ['add circle', { op: 'annot-add', page: 0, type: 'Circle', rect: [100, 250, 260, 350], contents: '' }],
    ['add line', { op: 'annot-add', page: 0, type: 'Line', points: [[100, 400], [300, 400]], contents: '' }],
    ['add ink', { op: 'annot-add', page: 0, type: 'Ink', strokes: [[[100, 450], [150, 500], [200, 450]]], contents: '' }],
    ['add stamp', { op: 'annot-add', page: 0, type: 'Stamp', rect: [100, 550, 260, 610], icon: 'Draft', contents: '' }],
    ['add stamp', { op: 'annot-add', page: 0, type: 'Stamp', rect: [100, 650, 300, 710], image: { name: 'img-gray.png', bytes: image }, contents: '' }],
  ];
}

test('the gate: every new type gets an /AP from update(), one journal step and one revision each', async () => {
  const session = await open();
  let count = 0;
  for (const [label, request] of validAdds()) {
    const reply = op(session, { id: count, ...request });
    assert.equal(reply.ok, true, `${request.type}: ${reply.error}`);
    count += 1;
    assert.equal(reply.revision, count, request.type);
    const list = items(session);
    assert.equal(list.length, count, `${request.type} added one annotation`);
    const added = list[list.length - 1];
    assert.equal(added.type, request.type, label);
    const live = liveAnnots(session);
    try {
      const annot = live.map.get(added.id);
      assert.ok(annot, request.type);
      assert.equal(annot.getObject().get('AP').isNull(), false, `${request.type} has an appearance stream`);
    } finally {
      live.close();
    }
  }
  assert.deepEqual(items(session).map(item => item.type),
    ['Square', 'Circle', 'Line', 'Ink', 'Stamp', 'Stamp'], 'the six shapes are listed in order');
  assert.equal(items(session).every(item => item.editable), true, 'every new type is editable');
  engine.closeDocument(session.doc);
});

test('each new type round-trips add, undo, redo and delete, and the step names are the journal labels', async () => {
  const session = await open();
  for (const [, request] of validAdds()) {
    const added = op(session, { id: 1, ...request });
    assert.equal(added.ok, true, `${request.type}: ${added.error}`);
  }
  // Undo walks the six adds back out; each reply names the step it reverted.
  for (const name of ['add stamp', 'add stamp', 'add ink', 'add line', 'add circle', 'add square']) {
    const undone = op(session, { op: 'undo' });
    assert.equal(undone.ok, true, undone.error);
    assert.equal(undone.step, name);
  }
  assert.deepEqual(items(session), [], 'six undos leave no annotations');
  assert.equal(op(session, { op: 'undo' }).ok, false, 'nothing left to undo');
  // Redo brings them all back in order, then delete removes each one.
  for (const name of ['add square', 'add circle', 'add line', 'add ink', 'add stamp', 'add stamp']) {
    const redone = op(session, { op: 'redo' });
    assert.equal(redone.ok, true, redone.error);
    assert.equal(redone.step, name);
  }
  assert.equal(items(session).length, 6, 'redo restores all six');
  for (const item of items(session)) {
    const deleted = op(session, { op: 'annot-delete', page: 0, annotId: item.id });
    assert.equal(deleted.ok, true, `${item.type}: ${deleted.error}`);
  }
  assert.deepEqual(items(session), [], 'every new type deletes');
  engine.closeDocument(session.doc);
});

test('the shapes carry the fixed looks and the geometry the request named', async () => {
  const session = await open();
  for (const [, request] of validAdds().slice(0, 5)) {
    assert.equal(op(session, { id: 1, ...request }).ok, true);
  }
  const by = Object.fromEntries(items(session).map(item => [item.type, item]));
  const page = session.doc.loadPage(0);
  try {
    const live = new Map(page.getAnnotations().map(annot => [annot.getObject().asIndirect(), annot]));
    const square = live.get(by.Square.id);
    const circle = live.get(by.Circle.id);
    const line = live.get(by.Line.id);
    const ink = live.get(by.Ink.id);
    const stamp = live.get(by.Stamp.id);
    assert.deepEqual([...square.getRect()], [100, 100, 260, 200], 'a Square keeps its rect');
    assert.deepEqual([...square.getColor()], [1, 0, 0], 'Square is red');
    assert.equal(square.getBorderWidth(), 1);
    assert.deepEqual([...circle.getColor()], [1, 0, 0], 'Circle is red');
    assert.deepEqual(line.getLine().map(point => [...point]), [[100, 400], [300, 400]], 'a Line keeps its endpoints');
    assert.deepEqual([...line.getColor()], [1, 0, 0], 'Line is red');
    assert.deepEqual(ink.getInkList().map(stroke => stroke.map(point => [...point])),
      [[[100, 450], [150, 500], [200, 450]]], 'Ink keeps its stroke');
    assert.deepEqual([...ink.getColor()], [0, 0, 1], 'Ink is blue');
    assert.equal(ink.getBorderWidth(), 1.5);
    assert.equal(stamp.getIcon(), 'Draft', 'the icon stamp keeps its icon');
  } finally {
    page.destroy();
  }
  assert.equal(by.Line.rect.length, 4, 'Line reports a box from its geometry, since it has no /Rect');
  assert.equal(by.Ink.rect.length, 4, 'Ink reports a box from its geometry too');

  // The five shapes survive save -> validate -> reopen, Line and Ink included (without /Rect they
  // must still get one from the writer, or the round trip would drop them).
  const saved = op(session, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const validated = op(session, { op: 'validate', pdf: saved.bytes });
  assert.equal(validated.ok, true, validated.error);
  const reopened = engine.createSession();
  assert.equal(engine.dispatch({ id: 2, op: 'open', name: 'shapes.pdf', pdf: validated.bytes }, reopened).reply.ok, true);
  try {
    const roundTripped = Object.fromEntries(items(reopened).map(item => [item.type, item]));
    assert.deepEqual(Object.keys(roundTripped).sort(), ['Circle', 'Ink', 'Line', 'Square', 'Stamp']);
    const page = reopened.doc.loadPage(0);
    try {
      const byType = Object.fromEntries(page.getAnnotations().map(annot => [annot.getType(), annot]));
      assert.equal(byType.Line.getLine().length, 2, 'the Line keeps its two endpoints');
      assert.equal(byType.Ink.getInkList()[0].length, 3, 'the Ink keeps its three points');
      assert.equal(byType.Stamp.getIcon(), 'Draft');
      closeTo(byType.Square.getRect()[2], 260, 'the Square keeps its rect');
    } finally {
      page.destroy();
    }
  } finally {
    engine.closeDocument(reopened.doc);
  }
  engine.closeDocument(session.doc);
});

test('an icon stamp keeps its centre but takes the icon aspect; an image stamp keeps the rect', async () => {
  const session = await open();
  assert.equal(op(session, { id: 1, op: 'annot-add', page: 0, type: 'Stamp', rect: [100, 300, 260, 400], icon: 'Final', contents: '' }).ok, true);
  const icon = items(session)[0];
  closeTo(icon.rect[0], 100, 'icon rect x0');
  closeTo(icon.rect[2], 260, 'icon rect x1');
  closeTo((icon.rect[1] + icon.rect[3]) / 2, 350, 'the icon stamp keeps the rect centre');
  assert.ok(icon.rect[3] - icon.rect[1] < 100, 'the icon aspect is shorter than the 100 pt request');
  assert.equal(op(session, { id: 2, op: 'annot-add', page: 0, type: 'Stamp', rect: [100, 500, 300, 560], image: { name: 'img-gray.png', bytes: GRAY_IMAGE }, contents: '' }).ok, true);
  const image = items(session).find(item => item.id !== icon.id);
  assertClose(image.rect, [100, 500, 300, 560], 'the image stamp keeps its 200x60 rect');
  engine.closeDocument(session.doc);
});

test('a signature appearance is a picture: /Subj, a dated contents line, no /Sig and no /ByteRange', async () => {
  const session = await open();
  const added = op(session, {
    id: 1, op: 'annot-add', page: 0, type: 'Stamp', rect: [72, 72, 272, 132],
    image: { name: 'img-gray.png', bytes: GRAY_IMAGE }, sign: { name: 'Ada Lovelace' }, contents: '',
  });
  assert.equal(added.ok, true, added.error);
  const [item] = items(session);
  assert.equal(item.type, 'Stamp');
  assert.equal(item.subject, SIGNATURE_SUBJECT, 'the row is told apart from an icon stamp by /Subj');
  const now = new Date();
  const pad = value => String(value).padStart(2, '0');
  const today = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
  assert.equal(item.contents, `Signed by Ada Lovelace on ${today}`);
  const live = liveAnnots(session);
  try {
    const annot = live.map.get(item.id);
    assert.equal(annot.getSubject(), SIGNATURE_SUBJECT);
    assertClose([...annot.getRect()], [72, 72, 272, 132], 'the signature keeps its own 200x60 box');
    assert.equal(annot.getObject().get('Sig').isNull(), true, 'the annotation itself is no /Sig');
  } finally {
    live.close();
  }

  const saved = op(session, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const validated = op(session, { op: 'validate', pdf: saved.bytes });
  assert.equal(validated.ok, true, validated.error);
  const raw = Buffer.from(new Uint8Array(validated.bytes)).toString('latin1');
  assert.equal(raw.includes('/ByteRange'), false, 'no /ByteRange is written');
  assert.equal(raw.includes('/SigFlags'), false, 'no /SigFlags is written');

  const reopened = engine.createSession();
  const opened = engine.dispatch({ id: 2, op: 'open', name: 'signed.pdf', pdf: validated.bytes }, reopened).reply;
  assert.equal(opened.ok, true, opened.error);
  assert.equal(opened.info.signatures, 0, 'the reopened document has no signature field');
  try {
    const page = reopened.doc.loadPage(0);
    const stamps = page.getAnnotations().filter(annot => annot.getType() === 'Stamp');
    assert.equal(stamps.length, 1, 'the signature appearance survives the round trip');
    assert.equal(stamps[0].getSubject(), SIGNATURE_SUBJECT);
    assert.equal(stamps[0].getContents(), `Signed by Ada Lovelace on ${today}`);
    assert.equal(stamps[0].getObject().get('AP').isNull(), false, 'its appearance stream survives too');
    page.destroy();
  } finally {
    engine.closeDocument(reopened.doc);
  }
  engine.closeDocument(session.doc);
});

test('bad geometry, Ink bounds, icons and images are refused without dirtying the document', async () => {
  const session = await open();
  const big = new Uint8Array(MAX_STAMP_IMAGE_BYTES + 1).buffer;
  const manyStrokes = Array.from({ length: MAX_INK_STROKES + 1 }, () => [[10, 10], [20, 20]]);
  const longStroke = Array.from({ length: MAX_INK_POINTS + 1 }, (_value, index) => [10 + (index % 100), 10 + (index % 100)]);
  const rejected = [
    ['off-page rect', { op: 'annot-add', page: 0, type: 'Square', rect: [100, 100, 700, 200], contents: '' }],
    ['negative rect', { op: 'annot-add', page: 0, type: 'Circle', rect: [-1, 100, 200, 200], contents: '' }],
    ['zero-area rect', { op: 'annot-add', page: 0, type: 'Square', rect: [100, 100, 100, 200], contents: '' }],
    ['reversed rect', { op: 'annot-add', page: 0, type: 'Circle', rect: [200, 200, 100, 100], contents: '' }],
    ['no rect', { op: 'annot-add', page: 0, type: 'Square', contents: '' }],
    ['one-point line', { op: 'annot-add', page: 0, type: 'Line', points: [[100, 100]], contents: '' }],
    ['identical line points', { op: 'annot-add', page: 0, type: 'Line', points: [[100, 100], [100, 100]], contents: '' }],
    ['off-page line', { op: 'annot-add', page: 0, type: 'Line', points: [[100, 100], [700, 100]], contents: '' }],
    ['no strokes', { op: 'annot-add', page: 0, type: 'Ink', contents: '' }],
    ['one-point stroke', { op: 'annot-add', page: 0, type: 'Ink', strokes: [[[100, 100]]], contents: '' }],
    ['65 strokes', { op: 'annot-add', page: 0, type: 'Ink', strokes: manyStrokes, contents: '' }],
    ['2049 points', { op: 'annot-add', page: 0, type: 'Ink', strokes: [longStroke], contents: '' }],
    ['off-page stroke', { op: 'annot-add', page: 0, type: 'Ink', strokes: [[[100, 100], [700, 200]]], contents: '' }],
    ['bad icon', { op: 'annot-add', page: 0, type: 'Stamp', rect: [100, 100, 200, 200], icon: 'Nope', contents: '' }],
    ['no icon and no image', { op: 'annot-add', page: 0, type: 'Stamp', rect: [100, 100, 200, 200], contents: '' }],
    ['icon and image', { op: 'annot-add', page: 0, type: 'Stamp', rect: [100, 100, 200, 200], icon: 'Draft', image: { name: 'img-gray.png', bytes: GRAY_IMAGE }, contents: '' }],
    ['5 MiB image', { op: 'annot-add', page: 0, type: 'Stamp', rect: [100, 100, 300, 160], image: { name: 'big.png', bytes: big }, contents: '' }],
    ['non-image bytes', { op: 'annot-add', page: 0, type: 'Stamp', rect: [100, 100, 300, 160], image: { name: 'not.png', bytes: new Uint8Array([1, 2, 3, 4]).buffer }, contents: '' }],
    ['nameless image', { op: 'annot-add', page: 0, type: 'Stamp', rect: [100, 100, 300, 160], image: { name: '', bytes: GRAY_IMAGE }, contents: '' }],
    ['signature without an image', { op: 'annot-add', page: 0, type: 'Stamp', rect: [100, 100, 300, 160], sign: { name: 'Ada' }, contents: '' }],
    ['signature without a name', { op: 'annot-add', page: 0, type: 'Stamp', rect: [100, 100, 300, 160], image: { name: 'img-gray.png', bytes: GRAY_IMAGE }, sign: { name: '' }, contents: '' }],
    ['201-char signature name', { op: 'annot-add', page: 0, type: 'Stamp', rect: [100, 100, 300, 160], image: { name: 'img-gray.png', bytes: GRAY_IMAGE }, sign: { name: 'x'.repeat(MAX_SIGN_NAME_CHARS + 1) }, contents: '' }],
  ];
  for (const [index, [label, request]] of rejected.entries()) {
    const reply = op(session, { id: index, ...request });
    assert.equal(reply.ok, false, label);
    assert.equal(typeof reply.error, 'string', label);
  }
  assert.deepEqual(items(session), [], 'nothing was created');
  assert.equal(session.revision, 0, 'no revision bump for a refused add');
  assert.equal(engine.journalState(session).dirty, false, 'a refused add leaves the document clean');
  engine.closeDocument(session.doc);
});

test('a read-only document refuses the new types before touching them', async () => {
  const bytes = await readFile('fixtures/encrypted.pdf');
  const session = engine.createSession();
  assert.equal(engine.dispatch({ id: 1, op: 'open', name: 'encrypted.pdf', pdf: toArrayBuffer(bytes) }, session).reply.ok, true);
  const refused = op(session, { op: 'annot-add', page: 0, type: 'Square', rect: [10, 10, 100, 100], contents: '' });
  assert.equal(refused.ok, false);
  assert.equal(refused.reason, 'read-only');
  engine.closeDocument(session.doc);
});

test('a 2048-point Ink on large.pdf lands inside the unit budget', async () => {
  const session = await open('large');
  const stroke = Array.from({ length: MAX_INK_POINTS }, (_value, index) => [72 + (index % 400), 72 + Math.floor(index / 4)]);
  const started = performance.now();
  const added = op(session, { id: 1, op: 'annot-add', page: 0, type: 'Ink', strokes: [stroke], contents: '' });
  const elapsed = performance.now() - started;
  assert.equal(added.ok, true, added.error);
  assert.equal(items(session, 0).filter(item => item.type === 'Ink').length, 1);
  assertWithin(elapsed, 2000, 'a 2048-point Ink');
  console.log(`P2_INK_COST ${JSON.stringify({ points: MAX_INK_POINTS, pages: session.doc.countPages(), ms: Math.round(elapsed) })}`);
  engine.closeDocument(session.doc);
});

function assertClose(actual, expected, label, epsilon = 0.01) {
  assert.equal(actual.length, expected.length, label);
  actual.forEach((value, index) => {
    assert.ok(Math.abs(value - expected[index]) <= epsilon, `${label}[${index}]: ${value} vs ${expected[index]}`);
  });
}

function closeTo(actual, expected, label, epsilon = 0.01) {
  assert.ok(Math.abs(actual - expected) <= epsilon, `${label}: ${actual} vs ${expected}`);
}
