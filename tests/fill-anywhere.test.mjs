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
import { createEngine, fieldValue, latin1, rectOf } from './rust-engine.mjs';
import { blankPdf } from './blank-pdf.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const engine = createEngine();
const NO_APP = process.env.TANDEM_NO_APP ? 'TANDEM_NO_APP' : false;
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
/**
 * Every FreeText on page 0 of the saved file: how many text lines its appearance draws inside its box, read
 * from the /AP /N stream (the file's values): each non-blank Tj or TJ whose origin, through the stream's
 * translations (cm, Td, Tm), lies inside the form's /BBox.
 */
function savedLines(s) {
  const saved = request(s, { op: 'save' });
  assert.equal(saved.ok, true, saved.error);
  const doc = engine.reopen(saved.bytes);
  try {
    return doc.annotsOf(0, 'FreeText').map(annot => {
      const ap = annot.dict('AP').get('N');
      const [x0, y0, x1, y1] = rectOf(ap.array('BBox'));
      let origin = [0, 0], line = [0, 0], count = 0;
      for (const op of latin1(ap.stream()).matchAll(/([-\d.\s]+?)\s(cm|Td|Tm)\b|BT\b|(\((?:\\.|[^\\)])*\)|\[[^\]]*\])\s*(Tj|TJ)\b/g)) {
        if (op[0] === 'BT') { line = [0, 0]; continue; }
        if (op[2]) {
          const n = op[1].trim().split(/\s+/).map(Number);
          if (op[2] === 'cm') origin = [origin[0] + n[4], origin[1] + n[5]];
          else if (op[2] === 'Tm') line = [n[4], n[5]];
          else line = [line[0] + n[0], line[1] + n[1]];
          continue;
        }
        const shown = op[3].startsWith('[') ? [...op[3].matchAll(/\(((?:\\.|[^\\)])*)\)/g)].map(m => m[1]).join('') : op[3].slice(1, -1);
        const [x, y] = [origin[0] + line[0], origin[1] + line[1]];
        if (shown.trim() !== '' && x >= x0 - 0.5 && x <= x1 + 0.5 && y >= y0 - 0.5 && y <= y1 + 0.5) count++;
      }
      return count;
    });
  } finally {
    engine.closeDocument(doc);
  }
}
/** The height of a one-line fit box: the line height the fit uses. */
function oneLineHeight() {
  const s = blank();
  assert.equal(fitAdd(s, [72, 300], 'Jane Doe').ok, true);
  const [box] = boxes(s);
  return box.rect[3] - box.rect[1];
}
const near = (actual, expected, label, tolerance = 0.5) => assert.ok(Math.abs(actual - expected) <= tolerance, `${label}: ${actual} vs ${expected}`);

test('(a) annot-add fit: left edge and first-line centre on the point, one drawn line per typed line, clamped inside the page', () => {
  const s = blank();
  const added = fitAdd(s, [72, 300], 'Jane Doe');
  assert.equal(added.ok, true, added.error);
  const [box] = boxes(s);
  const H = box.rect[3] - box.rect[1];
  assert.ok(H >= 12 && H <= 24, `one line of 12 pt text: ${H}`);
  near(box.rect[0], 72, 'left edge');
  near(box.rect[1] + H / 2, 300, 'first-line centre');
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
  near(boxes(two)[0].rect[3] - boxes(two)[0].rect[1], 2 * H, 'two lines tall');
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
  assert.equal(refused.error, 'annot-move: only FreeText, Square, Circle, Line, Ink annotations and signatures can be moved');
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
  // A FreeText from elsewhere, written by hand: Times 20 red at page-space [100, 100, 300, 160] with its appearance.
  const s = openBytes(pdfOf([
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Contents 4 0 R /Annots [5 0 R] >>',
    '<< /Length 0 >>\nstream\n\nendstream',
    '<< /Type /Annot /Subtype /FreeText /Rect [100 632 300 692] /F 4 /Contents (Foreign) /DA (/TiRo 20 Tf 1 0 0 rg) /AP << /N 6 0 R >> >>',
    '<< /Type /XObject /Subtype /Form /BBox [100 632 300 692] /Resources << /Font << /TiRo << /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >> >> >> /Length 50 >>\nstream\nBT 1 0 0 rg /TiRo 20 Tf 102 670 Td (Foreign) Tj ET\nendstream',
  ]));
  const [foreign] = boxes(s);
  const set = request(s, { op: 'annot-set', page: 0, annotId: foreign.id, contents: 'Foreign text, longer, with Ω', fit: true });
  assert.equal(set.ok, true, set.error);
  assert.deepEqual(boxes(s)[0].rect, [100, 100, 300, 160], 'a Times 20 box keeps its rect');
  assert.equal(boxes(s)[0].contents, 'Foreign text, longer, with Ω');
});

/** The bytes of a PDF of `objects` (numbered from 1, the first the catalog), every xref offset computed. */
function pdfOf(objects) {
  let text = '%PDF-1.7\n';
  const offsets = [];
  objects.forEach((body, i) => {
    offsets.push(text.length);
    text += `${i + 1} 0 obj\n${body}\nendobj\n`;
  });
  const xref = text.length;
  text += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets) text += `${String(offset).padStart(10, '0')} 00000 n \n`;
  text += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(text, 'latin1');
}

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
/** The FreeText annotations of saved bytes: their /Contents text and their /T. */
function freeTexts(bytes) {
  const doc = engine.reopen(bytes);
  try {
    return doc.annotsOf(0, 'FreeText').map(annot => ({ contents: annot.string('Contents') ?? '', author: annot.get('T') }));
  } finally {
    engine.closeDocument(doc);
  }
}

test('(b) drive: type on a flat form in Fill forms; a filled document saves as "(filled)"; a rotated page refuses', { skip: NO_APP, timeout: 180_000 }, async () => {
  const SCRIPT = 'tests/drive/fill-anywhere.jsonl';
  await header(SCRIPT, 34, 4);
  const H = oneLineHeight();
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
  near(added.boxes[0].rect[1] + H / 2, 300, 'box first-line centre', 1);
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
  assert.equal(freeTexts(flat).length, 0);
  const filled = freeTexts(await readFile(join(dir, 'flat (filled).pdf')));
  assert.deepEqual(filled.map(annot => annot.contents).sort(), ['Jane Q. Public', 'Second box']);
  for (const annot of filled) assert.equal(annot.author, null, 'no author on the text box');
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

test('(c) drive: fields still win in Fill forms; Escape writes nothing; the filled form saves as "(filled)" and form.pdf is unchanged', { skip: NO_APP, timeout: 180_000 }, async () => {
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
  assert.equal(freeTexts(await readFile(copy)).length, 0);
  const original = join(dir, 'form.pdf');
  assert.equal((await stat(original)).size, steps[1].state.save.bytes, 'form.pdf is what save-to wrote');
  const form = engine.reopen(await readFile(original));
  try {
    assert.equal(fieldValue(form.widget(0, 'name')) ?? '', '', 'the original keeps its empty field');
  } finally {
    engine.closeDocument(form);
  }
});
