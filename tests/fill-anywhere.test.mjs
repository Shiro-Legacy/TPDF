/**
 * fill-anywhere (contract-fill-anywhere.md D8): (a) the engine's fitted FreeText (`annot-add`/`annot-set`
 * `fit`) and `annot-move`; (b) the flat-form drive; (c) the form drive (fields still win; a filled
 * document saves as a copy). Engine harness and drive()/red()/brief() copied from tests/bugfix1.test.mjs
 * (not shared on purpose); verifyFields copied from tests/p1-save.test.mjs.
 */
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { mkdtemp as makeTempDir, readFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';
import mupdf from 'mupdf';
import { createEngine } from './rust-engine.mjs';
import { blankPdf } from '../build/blank-pdf.js';

const root = fileURLToPath(new URL('..', import.meta.url));
const engine = createEngine(mupdf);
const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;
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
const boxes = s => request(s, { op: 'annots', page: 0 }).items.filter(item => item.type === 'FreeText');
const fitAdd = (s, point, contents, extra = { fit: true }) => request(s, { op: 'annot-add', page: 0, type: 'FreeText', point, contents, ...extra });
/** The saved `/AP /N` stream of every FreeText on page 0: how many lines it draws (one Tj each). */
function savedLines(s) {
  const saved = request(s, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const doc = mupdf.Document.openDocument(new Uint8Array(saved.bytes), 'application/pdf');
  return doc.loadPage(0).getAnnotations().filter(annot => annot.getType() === 'FreeText')
    .map(annot => (annot.getObject().get('AP').get('N').readStream().asString().match(/\bTj\b/g) ?? []).length);
}
const near = (actual, expected, label, tolerance = 0.5) => assert.ok(Math.abs(actual - expected) <= tolerance, `${label}: ${actual} vs ${expected}`);

test('(a) annot-add fit: left edge and first-line centre on the point, one drawn line per typed line, clamped inside the page', () => {
  const s = blank();
  const added = fitAdd(s, [72, 300], 'Jane Doe');
  assert.equal(added.ok, true, added.error);
  const [box] = boxes(s);
  near(box.rect[0], 72, 'left edge');
  near(box.rect[1] + 7.2, 300, 'first-line centre (line height 14.4)');
  near(box.rect[3] - box.rect[1], 14.4, 'one line tall');
  assert.ok(box.rect[2] - box.rect[0] < 60, `fits "Jane Doe": ${box.rect}`);
  assert.deepEqual(savedLines(s), [1]);

  const long = blank();
  assert.equal(fitAdd(long, [72, 300], 'abcdefghij'.repeat(8)).ok, true);
  const [wide] = boxes(long);
  assert.ok(wide.rect[2] <= 612 && wide.rect[0] >= 0, `inside the page: ${wide.rect}`);
  assert.deepEqual(savedLines(long), [1], 'an 80-character line that fits the page draws on one line');

  // Review F4: a line wider than the page is refused before any write (it would wrap and be clipped).
  const tooWide = blank();
  const refusedWide = fitAdd(tooWide, [72, 300], 'abcdefghij'.repeat(20));
  assert.equal(refusedWide.ok, false);
  assert.equal(refusedWide.detail, 'overflow');
  assert.equal(boxes(tooWide).length, 0);
  assert.equal(refusedWide.dirty, false);
  assert.equal(refusedWide.revision, 0);

  const two = blank();
  assert.equal(fitAdd(two, [72, 300], 'a\nb').ok, true);
  near(boxes(two)[0].rect[3] - boxes(two)[0].rect[1], 28.8, 'two lines tall');
  assert.deepEqual(savedLines(two), [2]);

  const edge = blank();
  assert.equal(fitAdd(edge, [600, 300], 'Jane Doe').ok, true);
  const [clamped] = boxes(edge);
  assert.ok(clamped.rect[2] <= 612 + 1e-6 && clamped.rect[0] < 600, `moved inside the page: ${clamped.rect}`);
  assert.deepEqual(savedLines(edge), [1]);
});

test('(a) annot-add fit refuses a non-WinAnsi character before any write; without fit the box is the fixed 160x40', () => {
  const s = blank();
  const refused = fitAdd(s, [72, 300], 'Omega Ω');
  assert.equal(refused.ok, false);
  assert.equal(refused.detail, 'unsupported-encoding', JSON.stringify(refused));
  assert.deepEqual(refused.missing, ['Ω']);
  assert.equal(refused.dirty, false);
  assert.equal(refused.revision, 0);
  assert.equal(boxes(s).length, 0);

  const plain = blank();
  assert.equal(fitAdd(plain, [72, 300], 'Jane Doe', {}).ok, true);
  assert.deepEqual(boxes(plain)[0].rect.map(value => Math.round(value * 100) / 100), [72, 300, 232, 340]);
});

test('(a) annot-set fit re-fits keeping the left edge; annot-move moves, clamps, no-ops and refuses a note; undo reverts each', () => {
  const s = blank();
  assert.equal(fitAdd(s, [72, 300], 'Jane Doe').ok, true);
  const [first] = boxes(s);
  const set = request(s, { op: 'annot-set', page: 0, annotId: first.id, contents: 'Jane Quincy Public', fit: true });
  assert.equal(set.ok, true, set.error);
  const [refit] = boxes(s);
  near(refit.rect[0], first.rect[0], 'left edge kept', 0.01);
  near(refit.rect[1], first.rect[1], 'first line kept', 0.01);
  assert.ok(refit.rect[2] > first.rect[2] + 20, `wider: ${refit.rect} vs ${first.rect}`);
  assert.deepEqual(savedLines(s), [1]);

  const width = refit.rect[2] - refit.rect[0], height = refit.rect[3] - refit.rect[1];
  const moved = request(s, { op: 'annot-move', page: 0, annotId: first.id, point: [100, 400] });
  assert.equal(moved.ok, true, moved.error);
  assert.equal(moved.revision, 3);
  const [there] = boxes(s);
  near(there.rect[0], 100, 'moved x', 0.01);
  near(there.rect[1], 400, 'moved y', 0.01);
  near(there.rect[2] - there.rect[0], width, 'same width', 0.01);
  near(there.rect[3] - there.rect[1], height, 'same height', 0.01);

  const same = request(s, { op: 'annot-move', page: 0, annotId: first.id, point: [there.rect[0], there.rect[1]] });
  assert.equal(same.ok, true, same.error);
  assert.equal(same.noop, true);
  assert.equal(same.revision, 3, 'a no-op move journals nothing');

  assert.equal(request(s, { op: 'annot-move', page: 0, annotId: first.id, point: [600, 790] }).ok, true);
  const [edge] = boxes(s);
  near(edge.rect[2], 612, 'clamped at the right edge', 0.01);
  near(edge.rect[3], 792, 'clamped at the bottom edge', 0.01);

  assert.equal(request(s, { op: 'annot-add', page: 0, type: 'Text', point: [300, 100], contents: 'note' }).ok, true);
  const note = request(s, { op: 'annots', page: 0 }).items.find(item => item.type === 'Text');
  const refused = request(s, { op: 'annot-move', page: 0, annotId: note.id, point: [10, 10] });
  assert.equal(refused.ok, false);
  assert.equal(refused.error, 'annot-move: only FreeText annotations can be moved');
  assert.equal(refused.revision, 5);

  assert.equal(request(s, { op: 'undo' }).ok, true); // the note
  assert.equal(request(s, { op: 'undo' }).ok, true); // the clamped move
  near(boxes(s)[0].rect[0], 100, 'undo reverts the clamped move', 0.01);
  assert.equal(request(s, { op: 'undo' }).ok, true); // the move
  near(boxes(s)[0].rect[0], 72, 'undo reverts the move', 0.01);
  assert.equal(request(s, { op: 'undo' }).ok, true); // the refit
  near(boxes(s)[0].rect[2], first.rect[2], 'undo reverts the refit', 0.01);
  assert.equal(request(s, { op: 'undo' }).ok, true); // the add
  assert.equal(boxes(s).length, 0);
});

test('(a) review: annot-set fit leaves a FreeText that is not Helv 12 alone (rect kept, no WinAnsi refusal)', () => {
  const pdf = new mupdf.PDFDocument(blankPdf(612, 792));
  const made = pdf.loadPage(0).createAnnotation('FreeText');
  made.setRect([100, 100, 300, 160]);
  made.setDefaultAppearance('TiRo', 20, [1, 0, 0]);
  made.setContents('Foreign');
  made.update();
  const s = openBytes(pdf.saveToBuffer('').asUint8Array());
  const [foreign] = boxes(s);
  const set = request(s, { op: 'annot-set', page: 0, annotId: foreign.id, contents: 'Foreign text, longer, with Ω', fit: true });
  assert.equal(set.ok, true, set.error);
  assert.deepEqual(boxes(s)[0].rect, [100, 100, 300, 160], 'a Times 20 box keeps its rect');
  assert.equal(boxes(s)[0].contents, 'Foreign text, longer, with Ω');
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
const brief = step => ({ cmd: step.cmd, save: step.state?.save, name: step.state?.doc?.name, revision: step.state?.revision, dirty: step.state?.dirty, error: step.state?.error ?? step.error, active: step.state?.direct?.active, fill: step.state?.fill });
async function header(script, steps, clicks) {
  const text = await readFile(join(root, script), 'utf8');
  const found = text.match(/(\d+) steps, (\d+) click lines/);
  assert.ok(found, 'script header pins the step and click counts');
  assert.deepEqual([Number(found[1]), Number(found[2])], [steps, clicks]);
}
const freeTexts = doc => doc.loadPage(0).getAnnotations().filter(annot => annot.getType() === 'FreeText');

test('(b) drive: type on a flat form in Fill forms; a filled document saves as "(filled)"; a rotated page refuses', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  const SCRIPT = 'tests/drive/fill-anywhere.jsonl';
  await header(SCRIPT, 34, 4);
  const { code, report, dir } = await drive(SCRIPT, 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 34, report.error ?? '');
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);

  const opened = steps[4].state.direct.active;
  near(opened.point[0], 72, 'editor x', 1);
  near(opened.point[1], 300, 'editor y', 1);
  const added = steps[6].state.fill;
  assert.equal(added.boxes.length, 1);
  assert.equal(added.boxes[0].chars, 8);
  near(added.boxes[0].rect[0], 72, 'box left', 1);
  near(added.boxes[0].rect[1] + 7.2, 300, 'box first-line centre', 1);
  assert.equal(added.last.id, added.boxes[0].id);
  assert.equal(steps[7].state.direct.active.annotId, added.boxes[0].id, 'a click on the box re-opens it');
  const edited = steps[9].state.fill.boxes[0];
  assert.equal(edited.chars, 14);
  assert.ok(edited.rect[2] > added.boxes[0].rect[2] + 20, `wider after the edit: ${edited.rect}`);
  near(edited.rect[0], added.boxes[0].rect[0], 'same left edge', 0.01);
  const dragged = steps[10].state.fill.boxes[0];
  near(dragged.rect[0], edited.rect[0] + 100, 'dragged x', 1.5);
  near(dragged.rect[1], edited.rect[1] + 50, 'dragged y', 1.5);
  assert.deepEqual(steps[10].state.fill.last.rect, dragged.rect);
  const nudged = steps[11].state.fill.boxes[0];
  near(nudged.rect[0], dragged.rect[0] + 1, 'ArrowRight nudges 1 pt', 0.011);
  near(nudged.rect[1], dragged.rect[1], 'ArrowRight keeps y', 0.011);
  assert.equal(steps[14].state.fill.boxes.length, 2);
  assert.equal(steps[16].state.fill.boxes.length, 1);
  assert.equal(steps[17].state.fill.boxes.length, 2);

  // flat.pdf is exactly what save-to wrote: the filled save never touched it.
  const flat = await readFile(join(dir, 'flat.pdf'));
  assert.equal(flat.byteLength, steps[1].state.save.bytes);
  assert.equal(freeTexts(mupdf.Document.openDocument(flat, 'application/pdf')).length, 0);
  const filled = mupdf.Document.openDocument(await readFile(join(dir, 'flat (filled).pdf')), 'application/pdf');
  assert.deepEqual(freeTexts(filled).map(annot => annot.getContents()).sort(), ['Jane Q. Public', 'Second box']);
  for (const annot of freeTexts(filled)) assert.ok(annot.getObject().get('T').isNull(), 'no author on the text box');
});

function verifyFields(pdf, expected) {
  const checked = spawnSync(join(root, process.platform === 'win32' ? '.venv-qa/Scripts/python.exe' : '.venv-qa/bin/python3'), [
    join(root, 'scripts/verify-roundtrip.py'), pdf, '--fields', JSON.stringify(expected), '--appearance', 'Field:name',
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  const verdict = JSON.parse(checked.stdout);
  assert.equal(verdict.ok, true);
  return verdict;
}

test('(c) drive: fields still win in Fill forms; Escape writes nothing; the filled form saves as "(filled)" and form.pdf is unchanged', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  const SCRIPT = 'tests/drive/fill-anywhere-form.jsonl';
  await header(SCRIPT, 14, 2);
  const { code, report, dir } = await drive(SCRIPT, 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 14, report.error ?? '');
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);
  // The assertions moved from tests/p1-save.test.mjs (smoke --edit=form --save-in-place): the copy holds them.
  const copy = join(dir, 'form (filled).pdf');
  assert.equal(steps[12].state.save.validated, true);
  const verdict = verifyFields(copy, { name: 'PDF4All P2 form', agree: '/Yes', size: '/M', choice: 'two' });
  assert.ok(verdict.appearance.inside_frac > 0.01);
  assert.equal(freeTexts(mupdf.Document.openDocument(await readFile(copy), 'application/pdf')).length, 0);
  const original = join(dir, 'form.pdf');
  assert.equal((await stat(original)).size, steps[1].state.save.bytes, 'form.pdf is what save-to wrote');
  const form = mupdf.Document.openDocument(await readFile(original), 'application/pdf');
  const name = form.loadPage(0).getWidgets().find(widget => widget.getName() === 'name');
  assert.equal(name.getValue(), '', 'the original keeps its empty field');
});
