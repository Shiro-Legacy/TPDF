/**
 * draw-signature (contract-draw-signature.md D9): (a) the engine's drawn-signature Ink (`annot-add`
 * `signature`) and its move/resize through `annot-move`; (b) the signature store is a Rust test now
 * (crates/tandem-app/tests/sign_store.rs);
 * (c) the Fill-forms drive (draw, place, move, resize, delete, armed click/Enter/Escape, "(filled)" save)
 * and the rotated-page refusal. Engine harness and drive()/red()/brief()/header() copied from
 * tests/fill-anywhere.test.mjs (not shared on purpose).
 */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp as makeTempDir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from './rust-engine.mjs';
import { blankPdf } from './blank-pdf.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const engine = createEngine(mupdf);
const NO_APP = process.env.TANDEM_NO_APP ? 'TANDEM_NO_APP' : false;
/** Windows has no POSIX permission bits (the store sits under the user's own profile there) and no FIFO files. */
const WIN = process.platform === 'win32';
const buffer = bytes => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);

const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));

function openBytes(bytes, name = 'b.pdf') {
  const s = engine.createSession();
  const reply = engine.dispatch({ id: 1, op: 'open', name, pdf: buffer(bytes) }, s).reply;
  assert.equal(reply.ok, true, reply.error);
  return s;
}
const request = (s, req) => engine.dispatch({ id: 2, ...req }, s).reply;
const blank = () => openBytes(blankPdf(612, 792));
const near = (actual, expected, label, tolerance = 0.01) => assert.ok(Math.abs(actual - expected) <= tolerance, `${label}: ${actual} vs ${expected}`);
const inks = s => request(s, { op: 'annots', page: 0 }).items.filter(item => item.type === 'Ink');
/** Two strokes whose bounds are 200 x 50 pt at [100,100]. */
const STROKES = [[[100, 100], [150, 150], [200, 110]], [[220, 120], [260, 140], [300, 100]]];
const addSig = (s, strokes = STROKES, extra = { signature: true }) => request(s, { op: 'annot-add', page: 0, type: 'Ink', strokes, contents: '', ...extra });
/** The saved file's Ink annotations, read back by mupdf. */
function savedInks(s) {
  const saved = request(s, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const doc = mupdf.Document.openDocument(new Uint8Array(saved.bytes), 'application/pdf');
  return doc.loadPage(0).getAnnotations().filter(annot => annot.getType() === 'Ink');
}
const flat = strokes => strokes.flat();

test('(a) annot-add signature: black, width from the bounds, the drawn-signature subject, no /T, no /Contents, the strokes as sent', () => {
  const s = blank();
  const added = addSig(s);
  assert.equal(added.ok, true, added.error);
  assert.equal(added.revision, 1);
  const [listed] = inks(s);
  assert.equal(listed.subject, 'Drawn signature');
  assert.deepEqual(listed.rect.map(v => Math.round(v * 100) / 100), [100, 100, 300, 150]);
  const [ink] = savedInks(s);
  assert.deepEqual(ink.getColor(), [0, 0, 0]);
  near(ink.getBorderWidth(), 1.5, 'border width (200 pt x 0.0075)');
  assert.equal(ink.getSubject(), 'Drawn signature');
  assert.ok(ink.getObject().get('T').isNull(), 'no /T');
  assert.ok(ink.getObject().get('Contents').isNull(), 'no /Contents');
  const list = ink.getInkList();
  assert.equal(list.length, 2);
  flat(list).forEach((point, index) => { near(point[0], flat(STROKES)[index][0], `x ${index}`); near(point[1], flat(STROKES)[index][1], `y ${index}`); });

  // Without `signature` the Ink is today's: blue, 1.5, no subject.
  const plain = blank();
  assert.equal(addSig(plain, STROKES, {}).ok, true);
  const [blue] = savedInks(plain);
  assert.deepEqual(blue.getColor(), [0, 0, 1]);
  near(blue.getBorderWidth(), 1.5, 'plain ink width');
  assert.equal(blue.getSubject(), '');

  // `signature` on another type, or a non-boolean, is refused before any write.
  for (const extra of [{ type: 'Square', rect: [100, 100, 200, 200], signature: true }, { type: 'Ink', strokes: STROKES, signature: 'yes' }]) {
    const t = blank();
    const refused = request(t, { op: 'annot-add', page: 0, contents: '', ...extra });
    assert.equal(refused.ok, false, JSON.stringify(extra));
    assert.equal(refused.dirty, false);
    assert.equal(refused.revision, 0);
    assert.equal(request(t, { op: 'annots', page: 0 }).items.length, 0);
  }
});

test('(a) annot-move on a drawn signature: moves every point, resizes keeping the aspect, clamps, no-ops; refusals; undo reverts each', () => {
  const s = blank();
  assert.equal(addSig(s).ok, true);
  const [sig] = inks(s);
  const moved = request(s, { op: 'annot-move', page: 0, annotId: sig.id, point: [150, 200] });
  assert.equal(moved.ok, true, moved.error);
  assert.equal(moved.revision, 2);
  const [afterMove] = savedInks(s);
  flat(afterMove.getInkList()).forEach((point, index) => { near(point[0], flat(STROKES)[index][0] + 50, `moved x ${index}`); near(point[1], flat(STROKES)[index][1] + 100, `moved y ${index}`); });

  const resized = request(s, { op: 'annot-move', page: 0, annotId: sig.id, point: [150, 200], width: 100 });
  assert.equal(resized.ok, true, resized.error);
  assert.equal(resized.revision, 3);
  const [small] = inks(s);
  near(small.rect[0], 150, 'left kept');
  near(small.rect[1], 200, 'top kept');
  near(small.rect[2] - small.rect[0], 100, 'width 100');
  near(small.rect[3] - small.rect[1], 25, 'aspect kept (200x50 -> 100x25)');
  near(savedInks(s)[0].getBorderWidth(), 0.75, 'border width follows the width');

  assert.equal(request(s, { op: 'annot-move', page: 0, annotId: sig.id, point: [150, 200], width: 5 }).ok, true);
  near(inks(s)[0].rect[2] - inks(s)[0].rect[0], 24, 'width clamped to 24');

  assert.equal(request(s, { op: 'annot-move', page: 0, annotId: sig.id, point: [600, 790] }).ok, true);
  const [edge] = inks(s);
  assert.ok(edge.rect[2] <= 612 + 0.01 && edge.rect[3] <= 792 + 0.01 && edge.rect[0] >= 0 && edge.rect[1] >= 0, `inside the page: ${edge.rect}`);
  assert.equal(edge.rect.every(Number.isFinite), true);

  const same = request(s, { op: 'annot-move', page: 0, annotId: sig.id, point: [edge.rect[0], edge.rect[1]], width: edge.rect[2] - edge.rect[0] });
  assert.equal(same.ok, true, same.error);
  assert.equal(same.noop, true);
  assert.equal(same.revision, 5, 'a no-op journals nothing');

  // An ordinary Ink keeps today's refusal; width on a FreeText is refused; a bad width is refused.
  assert.equal(addSig(s, [[[300, 300], [350, 350]]], {}).ok, true);
  const plain = inks(s).find(item => item.subject !== 'Drawn signature');
  const refused = request(s, { op: 'annot-move', page: 0, annotId: plain.id, point: [10, 10] });
  assert.equal(refused.ok, false);
  assert.equal(refused.error, 'annot-move: only FreeText annotations can be moved');
  assert.equal(request(s, { op: 'annot-add', page: 0, type: 'FreeText', point: [72, 500], contents: 'box', fit: true }).ok, true);
  const box = request(s, { op: 'annots', page: 0 }).items.find(item => item.type === 'FreeText');
  const widthOnBox = request(s, { op: 'annot-move', page: 0, annotId: box.id, point: [80, 500], width: 50 });
  assert.equal(widthOnBox.ok, false);
  assert.equal(widthOnBox.error, 'annot-move: width applies to drawn signatures only');
  assert.equal(widthOnBox.revision, 7);
  const badWidth = request(s, { op: 'annot-move', page: 0, annotId: sig.id, point: [10, 10], width: 'wide' });
  assert.equal(badWidth.ok, false);
  assert.equal(badWidth.revision, 7);

  for (let i = 0; i < 2; i++) assert.equal(request(s, { op: 'undo' }).ok, true); // the box, the plain ink
  assert.equal(request(s, { op: 'undo' }).ok, true); // the edge move
  near(inks(s)[0].rect[2] - inks(s)[0].rect[0], 24, 'undo keeps the 24 width');
  near(inks(s)[0].rect[0], 150, 'undo reverts the edge move');
  assert.equal(request(s, { op: 'undo' }).ok, true); // the clamp to 24
  near(inks(s)[0].rect[2] - inks(s)[0].rect[0], 100, 'undo reverts the 24 resize');
  assert.equal(request(s, { op: 'undo' }).ok, true); // the 100 resize
  near(inks(s)[0].rect[2] - inks(s)[0].rect[0], 200, 'undo reverts the resize');
  assert.equal(request(s, { op: 'undo' }).ok, true); // the move
  near(inks(s)[0].rect[0], 100, 'undo reverts the move');
  near(inks(s)[0].rect[1], 100, 'undo reverts the move (y)');
});

test('(a) annot-move refuses a width on a signature narrower than 1 pt', () => {
  const s = blank();
  assert.equal(addSig(s, [[[100, 100], [100.5, 200]]]).ok, true);
  const [sig] = inks(s);
  const refused = request(s, { op: 'annot-move', page: 0, annotId: sig.id, point: [100, 100], width: 50 });
  assert.equal(refused.ok, false);
  assert.equal(refused.revision, 1);
  const moved = request(s, { op: 'annot-move', page: 0, annotId: sig.id, point: [120, 100] });
  assert.equal(moved.ok, true, moved.error);
  near(inks(s)[0].rect[0], 120, 'a plain move still works');
});

async function drive(script, timeoutMs = 90_000) {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-drive-test-'));
  const report = join(dir, 'report.json');
  const child = spawn(process.execPath, [join(root, 'scripts/drive.mjs'), script, `--out=${dir}`, `--report=${report}`], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '';
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', chunk => { stdout += chunk; });
  const timer = setTimeout(() => child.kill(), timeoutMs);
  const code = await new Promise(resolve => child.on('close', resolve));
  clearTimeout(timer);
  return { code, report: JSON.parse(stdout), dir, stdout };
}
const red = steps => steps.filter(step => step.expect !== undefined && !step.ok);
const brief = step => ({ cmd: step.cmd, save: step.state?.save, revision: step.state?.revision, dirty: step.state?.dirty, error: step.state?.error ?? step.error, announcement: step.state?.announcement, fill: step.state?.fill });
async function header(script, steps, clicks) {
  const text = await readFile(join(root, script), 'utf8');
  const found = text.match(/(\d+) steps, (\d+) click lines/);
  assert.ok(found, 'script header pins the step and click counts');
  assert.deepEqual([Number(found[1]), Number(found[2])], [steps, clicks]);
}
const savedSigs = doc => doc.loadPage(0).getAnnotations().filter(annot => annot.getType() === 'Ink');

test('(c) drive: draw a signature, place it in Fill forms, move/resize/delete it, armed click/Enter/Escape, and save a "(filled)" copy', { skip: NO_APP, timeout: 180_000 }, async () => {
  const SCRIPT = 'tests/drive/draw-signature.jsonl';
  await header(SCRIPT, 25, 1);
  const { code, report, dir } = await drive(SCRIPT, 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 25, report.error ?? '');
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);

  const drawn = steps[4].state.fill.signature;
  assert.deepEqual(drawn, { saved: true, kind: 'drawn', strokes: 2, points: 7, armed: false, pad: false });
  assert.ok(!JSON.stringify(steps[4].state).includes('[[['), 'STATE never carries the stroke points');
  const placed = steps[5].state.fill;
  assert.equal(placed.signatures.length, 1);
  const [x0, y0, x1, y1] = placed.signatures[0].rect;
  near((x0 + x1) / 2, 300, 'centred x', 0.05);
  near((y0 + y1) / 2, 650, 'centred y', 0.05);
  assert.ok(x1 - x0 <= 200.01 && y1 - y0 <= 60.01, `inside 200x60: ${placed.signatures[0].rect}`);
  near(x1 - x0, 200, 'fills the 200 pt width (200x40 drawing)', 0.05);
  assert.deepEqual(placed.lastSignature, { op: 'add', id: placed.signatures[0].id, rect: placed.signatures[0].rect });
  const dragged = steps[6].state.fill.signatures[0].rect;
  near(dragged[0], x0 + 50, 'dragged x', 1.5);
  near(dragged[1], y0 - 100, 'dragged y', 1.5);
  near(dragged[2] - dragged[0], x1 - x0, 'a drag keeps the width', 0.05);
  const nudged = steps[7].state.fill.signatures[0].rect;
  near(nudged[0], dragged[0] + 1, 'ArrowRight nudges 1 pt', 0.02);
  near(nudged[1], dragged[1], 'ArrowRight keeps y', 0.02);
  const grown = steps[8].state.fill.signatures[0].rect;
  near(grown[2] - grown[0], (nudged[2] - nudged[0]) * 1.1, '+ grows the width by 10 %', 0.05);
  near(grown[3] - grown[1], (nudged[3] - nudged[1]) * 1.1, '+ keeps the aspect', 0.05);
  near(grown[0], nudged[0], '+ keeps the left edge', 0.02);
  near(grown[1], nudged[1], '+ keeps the top edge', 0.02);
  const handled = steps[9].state.fill.signatures[0].rect;
  assert.ok(handled[2] - handled[0] > grown[2] - grown[0] + 10, `the se handle grows it: ${handled} vs ${grown}`);
  near((handled[3] - handled[1]) / (handled[2] - handled[0]), (grown[3] - grown[1]) / (grown[2] - grown[0]), 'the handle keeps the aspect', 0.01);
  near(handled[0], grown[0], 'the se handle keeps the left edge', 0.02);
  assert.equal(steps[10].state.fill.signatures.length, 0);
  assert.equal(steps[11].state.fill.signatures.length, 1);
  const clicked = steps[13].state.fill.signatures.find(item => item.id === steps[13].state.fill.lastSignature.id).rect;
  near((clicked[0] + clicked[2]) / 2, 150, 'placed by the click x', 1);
  near((clicked[1] + clicked[3]) / 2, 200, 'placed by the click y', 1);
  const entered = steps[15].state.fill.signatures.find(item => item.id === steps[15].state.fill.lastSignature.id).rect;
  near((entered[0] + entered[2]) / 2, 306, 'Enter places at the page centre x', 0.05);
  near((entered[1] + entered[3]) / 2, 396, 'Enter places at the page centre y', 0.05);

  // sig.pdf is what save-to wrote; the filled copy holds both signatures, black, no author.
  const original = await readFile(join(dir, 'sig.pdf'));
  assert.equal(original.byteLength, steps[1].state.save.bytes);
  assert.equal(savedSigs(mupdf.Document.openDocument(original, 'application/pdf')).length, 0);
  const filled = savedSigs(mupdf.Document.openDocument(await readFile(join(dir, 'sig (filled).pdf')), 'application/pdf'));
  assert.equal(filled.length, 2);
  for (const ink of filled) {
    assert.deepEqual(ink.getColor(), [0, 0, 0]);
    assert.equal(ink.getSubject(), 'Drawn signature');
    assert.ok(ink.getObject().get('T').isNull(), 'no author on the signature');
    assert.ok(ink.getObject().get('Contents').isNull(), 'no contents on the signature');
  }
});

test('(c) drive: a rotated page refuses a signature and stays clean', { skip: NO_APP, timeout: 180_000 }, async () => {
  const SCRIPT = 'tests/drive/draw-signature-rotated.jsonl';
  await header(SCRIPT, 9, 0);
  const { code, report } = await drive(SCRIPT, 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 9, report.error ?? '');
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);
});

// ---------------------------------------------------------------- review (review-draw-signature.md)

test('(review) undo and redo of a resize restore the border width as well as the ink list', () => {
  const t = blank();
  assert.equal(addSig(t).ok, true);
  const [small] = inks(t);
  assert.equal(request(t, { op: 'annot-move', page: 0, annotId: small.id, point: [100, 100], width: 100 }).ok, true);
  near(savedInks(t)[0].getBorderWidth(), 0.75, 'resized width');
  assert.equal(request(t, { op: 'undo' }).ok, true);
  near(savedInks(t)[0].getBorderWidth(), 1.5, 'undo restores the border width');
  assert.equal(request(t, { op: 'redo' }).ok, true);
  near(savedInks(t)[0].getBorderWidth(), 0.75, 'redo re-applies it');
  near(inks(t)[0].rect[2] - inks(t)[0].rect[0], 100, 'redo re-applies the ink list');
});

test('(review) drive: a realistic drawing over 4096 characters, a store-sized drawing, an over-cap and a two-key sign-draw, and a page under 200 pt', { skip: NO_APP, timeout: 180_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-sig-review-'));
  // 600 points with long decimals: about 15 000 characters of JSON, 300 x 100 px.
  const realistic = [0, 1].map(k => Array.from({ length: 300 }, (_, i) => [i * 300 / 299, (i + k) % 2 * 100]));
  // 64 x 2048 points 2 px apart: over the 4096-point cap (the engine would take minutes), refused. 2 x 2048 is the most that fits.
  const store = Array.from({ length: 64 }, (_, k) => Array.from({ length: 2048 }, (_, i) => [i * 2, k * 60]));
  const most = store.slice(0, 2);
  const lines = [
    { cmd: 'new', contents: { size: [144, 144] }, expect: { doc: { name: 'Untitled.pdf' }, revision: 0, error: '' } },
    { cmd: 'sign-draw', contents: { strokes: store }, expect: { error: 'sign-draw: a signature can have at most 4096 points (points under 2 px apart are skipped)', fill: { signature: { saved: false, points: 0, pad: false } }, dirty: false } },
    { cmd: 'sign-draw', contents: { strokes: most }, expect: { error: '', fill: { signature: { saved: true, strokes: 2, points: 4096, pad: false } }, dirty: false } },
    // A tap is a dot (two points 0.5 px apart), and samples under 2 px from the last are skipped.
    { cmd: 'sign-draw', contents: { strokes: [[[0, 0], [1, 0], [1.9, 0], [3, 0], [40, 10]], [[20, 20]]] }, expect: { error: '', fill: { signature: { saved: true, strokes: 2, points: 5 } }, dirty: false } },
    { cmd: 'sign-draw', contents: { strokes: realistic }, expect: { error: '', fill: { signature: { saved: true, strokes: 2, points: 600 } }, dirty: false } },
    { cmd: 'sign-draw', contents: { strokes: [[[0, 0], [50, 10]]], pad: 'x'.repeat(4 * 1024 * 1024) }, expect: { error: 'sign-draw: contents are empty or larger than 4 MiB', fill: { signature: { saved: true, points: 600 } } } },
    { cmd: 'sign-draw', contents: { clear: true, strokes: [[[0, 0], [50, 10]]] }, expect: { error: 'sign-draw: give one of strokes, image or clear', fill: { signature: { saved: true, points: 600 } } } },
    { cmd: 'sign-place', contents: { page: 0, x: 72, y: 72 }, expect: { error: '', annots: { Ink: 1 }, revision: 1 } },
    // D5: an armed click lands over a text box (placement outranks the box); the box is untouched.
    { cmd: 'fill-text', contents: { page: 0, x: 10, y: 20, text: 'Box' }, expect: { error: '', annots: { Ink: 1, FreeText: 1 }, revision: 2 } },
    { cmd: 'sign-place', expect: { fill: { signature: { armed: true } }, revision: 2, error: '' } },
    { cmd: 'click', page: 0, x: 15, y: 20, expect: { annots: { Ink: 2, FreeText: 1 }, fill: { signature: { armed: false }, lastSignature: { op: 'add' } }, revision: 3, error: '' } },
  ];
  const script = join(dir, 'review.jsonl');
  await writeFile(script, lines.map(line => JSON.stringify(line)).join('\n') + '\n');
  const { code, report } = await drive(script, 150_000);
  const steps = report.steps;
  assert.equal(steps.length, lines.length, report.error ?? '');
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);
  for (const step of steps.filter(item => item.cmd === 'sign-draw')) assert.equal(step.contents, '[redacted]', 'the report never carries the stroke points');
  const [rect] = steps[7].state.fill.signatures.map(item => item.rect);
  assert.ok(rect[0] >= -0.01 && rect[1] >= -0.01 && rect[2] <= 144.01 && rect[3] <= 144.01, `inside the 144 pt page: ${rect}`);
  near((rect[2] - rect[0]) / (rect[3] - rect[1]), 3, 'the 300 x 100 drawing keeps its aspect on a small page', 0.02);
  near(rect[2] - rect[0], 144, 'fits the page width', 0.05);
});
