/**
 * P6 U6 drive (contract-p6-unit6.md §4.2 as amended by §9.29, §9.31): tests/drive/direct-edit7.jsonl holds the exact-value
 * expects; the geometry (rotated / flipped / cropped boxes, group resize and rotate, snapped drags) is checked here, ±0.5 pt on
 * `view`/`full` (y-down), and every `orient` by deep-equal on the object found by name and box (after a group rotate the
 * members tie on x0, so objects are picked by view, never by name alone).
 * drive()/pick()/near()/all() are copies of tests/p6-drive6.test.mjs (not shared on purpose).
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { mkdtemp as makeTempDir, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';

const root = fileURLToPath(new URL('..', import.meta.url));

/** Every temp dir this file makes (housekeeping B): removed after all its tests, never before their asserts. */
const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));
const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;
const SCRIPT = 'tests/drive/direct-edit7.jsonl';
const OBJECTS6 = 'fixtures/p6/objects6.pdf';
const OBJECTS6_SHA = 'ca0a5988eb81cd6b35916918695cd8c9881b8ebba82ae590e38cdf8a97fb2ab8';
const DIRECT3 = 'fixtures/p6/direct3.pdf';
const DIRECT3_SHA = 'e3ae5a43d9fb17e5b164c00b529d5be50ba4d9178de6c11b66252cc240297466';

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

function at(object, path) {
  return path.split('.').reduce((value, key) => (value == null ? undefined : value[key]), object);
}

/** The step whose cmd and line keys match; `want` may use dotted paths into the state. */
function pick(report, cmd, want = {}) {
  const hits = report.steps.filter(step => step.cmd === cmd
    && Object.entries(want).every(([key, value]) => JSON.stringify(at(step, key)) === JSON.stringify(value)));
  assert.ok(hits.length >= 1, `no ${cmd} step matching ${JSON.stringify(want)}`);
  return hits[0];
}

/** Every number within ±tol (pt). */
function near(got, want, tol, label) {
  assert.ok(Array.isArray(got) && got.length === want.length && got.every((value, i) => Math.abs(value - want[i]) <= tol),
    `${label}: ${JSON.stringify(got)} vs ${JSON.stringify(want)} ±${tol}`);
}
const sha = async path => createHash('sha256').update(await readFile(join(root, path))).digest('hex');
/** Every `view` box of one name, sorted by x then y. */
const all = (step, name) => step.state.content.objects.filter(o => o.name === name).map(o => o.view).sort((a, b) => a[0] - b[0] || a[1] - b[1]);

const ID = { swap: false, sx: 1, sy: 1 };
/**
 * `name` has exactly one box, `view` ±0.5 (via all()); the object at that view carries that name, `orient` deep-equal,
 * and `full` ±0.5 or absent (`full === null`).
 */
function obj(step, name, view, orient, full, label) {
  const got = all(step, name);
  assert.equal(got.length, 1, `${label}: ${name} count ${JSON.stringify(got)}`);
  near(got[0], view, 0.5, `${label}: ${name} view`);
  const hits = step.state.content.objects.filter(o => Array.isArray(o.view) && o.view.every((v, i) => Math.abs(v - view[i]) <= 0.5));
  assert.equal(hits.length, 1, `${label}: one object at ${JSON.stringify(view)}`);
  assert.equal(hits[0].name, name, `${label}: object at ${JSON.stringify(view)}`);
  assert.deepEqual(hits[0].orient, orient, `${label}: ${name} orient`);
  if (full === null) assert.equal(hits[0].full, undefined, `${label}: ${name} full absent`);
  else near(hits[0].full, full, 0.5, `${label}: ${name} full`);
}
const IM1 = [72, 142, 172, 192];
const IM3 = [200, 142, 300, 192];
const pristine = (step, label) => {
  assert.equal(step.state.content.objects.length, 6, `${label}: object count`);
  obj(step, 'Im1', IM1, ID, null, label);
  obj(step, 'Im2', [400, 42, 450, 142], { swap: true, sx: 1, sy: -1 }, null, label);
  obj(step, 'Im3', IM3, { swap: false, sx: -1, sy: 1 }, null, label);
  obj(step, 'Im4', [72, 352, 132, 392], ID, [72, 342, 172, 392], label);
  obj(step, 'Fm1', [72, 492, 372, 692], ID, null, label);
  obj(step, 'Im5', [300, 622, 350, 672], ID, null, label);
};
const red = steps => steps.filter(step => step.expect !== undefined && !step.ok);
const brief = step => ({ cmd: step.cmd, contents: step.contents, direct: step.state?.direct && { active: step.state.direct.active, selection: step.state.direct.selection, menu: step.state.direct.menu, crop: step.state.direct.crop, objects: step.state.direct.objects }, edited: step.state?.content?.edited, revision: step.state?.revision, dirty: step.state?.dirty, error: step.state?.error });

test('drive P6 U6: rotate / flip, crop mode, group resize, alignment guides, Form pass-through', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  const before6 = await sha(OBJECTS6);
  const before3 = await sha(DIRECT3);
  assert.equal(before6, OBJECTS6_SHA);
  assert.equal(before3, DIRECT3_SHA);
  // §9.31: the counts come from the script header ("66 steps, 18 click lines").
  const header = (await readFile(join(root, SCRIPT), 'utf8')).match(/(\d+) steps, (\d+) click lines/);
  assert.ok(header, 'script header pins the step and click counts');
  const { code, report } = await drive(SCRIPT, 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 66, report.error ?? '');
  assert.equal(steps.length, Number(header[1]));
  assert.equal(steps.filter(step => step.cmd === 'click').length, 18);
  assert.equal(Number(header[2]), 18);
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);
  const next = step => steps[steps.indexOf(step) + 1];
  const edit = pick(report, 'mode', { 'contents.mode': 'edit' });
  pristine(edit, 'edit mode');

  // Single rotate / flip (§1.2 orients; view space y down).
  const cw = pick(report, 'menu', { 'contents.item': 'rotate-cw', 'state.revision': 1 });
  assert.equal(cw.state.doc.name, 'objects6.pdf');
  obj(cw, 'Im1', [97, 117, 147, 217], { swap: true, sx: 1, sy: -1 }, null, 'Im1 rotate-cw');
  pristine(next(cw), 'Im1 rotate-cw undo');
  const flipH = pick(report, 'menu', { 'contents.item': 'flip-h', 'state.revision': 3 });
  obj(flipH, 'Im1', IM1, { swap: false, sx: -1, sy: 1 }, null, 'Im1 flip-h');
  pristine(next(flipH), 'Im1 flip-h undo');
  const im2cw = pick(report, 'menu', { 'contents.item': 'rotate-cw', 'state.revision': 5 });
  obj(im2cw, 'Im2', [375, 67, 475, 117], { swap: false, sx: -1, sy: -1 }, null, 'Im2 rotate-cw');
  pristine(next(im2cw), 'Im2 rotate-cw undo');

  // Crop mode: Im1 east handle −40 (x1 132), then the grip +20 moves the window inside full.
  const cropE = pick(report, 'handle', { 'contents.handle': 'e', 'state.revision': 7 });
  obj(cropE, 'Im1', [72, 142, 132, 192], ID, IM1, 'crop e');
  const cropGrip = pick(report, 'handle', { 'contents.handle': 'grip', 'state.revision': 8 });
  obj(cropGrip, 'Im1', [92, 142, 152, 192], ID, IM1, 'crop grip');
  // Im4: the west handle cannot move (silent no-op); ne +40,−10 reaches the full box, so the clip is removed.
  const cropW = pick(report, 'handle', { 'contents.handle': 'w' });
  obj(cropW, 'Im4', [72, 352, 132, 392], ID, [72, 342, 172, 392], 'crop w no-op');
  const cropNe = pick(report, 'handle', { 'contents.handle': 'ne', 'state.revision': 9 });
  obj(cropNe, 'Im4', [72, 342, 172, 392], ID, null, 'crop ne to full');
  pristine(pick(report, 'undo', { 'state.revision': 12 }), 'crop undo ×3');

  // Group-box east handle +228: the union [72,300] → [72,528]; members map by the affine.
  const groupResize = pick(report, 'handle', { 'contents.handle': 'e', 'state.revision': 13 });
  obj(groupResize, 'Im1', [72, 142, 272, 192], ID, null, 'group resize');
  obj(groupResize, 'Im3', [328, 142, 528, 192], { swap: false, sx: -1, sy: 1 }, null, 'group resize');
  pristine(next(groupResize), 'group resize undo');

  // §9.29 group rotate-cw about the union centre (186,167): picked by view (the members tie on x0).
  const groupCw = pick(report, 'menu', { 'contents.item': 'rotate-cw', 'state.revision': 15 });
  obj(groupCw, 'Im1', [161, 53, 211, 153], { swap: true, sx: 1, sy: -1 }, null, 'group rotate-cw');
  obj(groupCw, 'Im3', [161, 181, 211, 281], { swap: true, sx: 1, sy: 1 }, null, 'group rotate-cw');
  pristine(next(groupCw), 'group rotate-cw undo');

  // §9.19 guides: +326 snaps x0 to Im2's 400; with meta the drag lands at 398.
  const snapped = pick(report, 'drag', { 'state.revision': 17 });
  assert.equal(snapped.contents.meta, undefined);
  obj(snapped, 'Im1', [400, 142, 500, 192], ID, null, 'snapped drag');
  pristine(next(snapped), 'snapped drag undo');
  const free = pick(report, 'drag', { 'contents.meta': true });
  assert.equal(free.state.revision, 19);
  obj(free, 'Im1', [398, 142, 498, 192], ID, null, 'meta drag');
  pristine(next(free), 'meta drag undo');

  // rotate-ccw on Im1, flip-v on Im3.
  const ccw = pick(report, 'menu', { 'contents.item': 'rotate-ccw', 'state.revision': 21 });
  obj(ccw, 'Im1', [97, 117, 147, 217], { swap: true, sx: -1, sy: 1 }, null, 'Im1 rotate-ccw');
  pristine(next(ccw), 'rotate-ccw undo');
  const flipV = pick(report, 'menu', { 'contents.item': 'flip-v', 'state.revision': 23 });
  obj(flipV, 'Im3', IM3, { swap: false, sx: -1, sy: -1 }, null, 'Im3 flip-v');
  obj(flipV, 'Im1', IM1, ID, null, 'Im3 flip-v');
  pristine(next(flipV), 'flip-v undo');

  // §9.21 group ⌥⌘→: union x1 300 → 301 (scale 229/228), keyboard path, no snap.
  const widen = pick(report, 'key', { 'contents.key': 'Alt+Meta+ArrowRight' });
  obj(widen, 'Im1', [72, 142, 172.4386, 192], ID, null, 'group ⌥⌘→');
  obj(widen, 'Im3', [200.5614, 142, 301, 192], { swap: false, sx: -1, sy: 1 }, null, 'group ⌥⌘→');
  pristine(next(widen), 'group ⌥⌘→ undo');

  // Group flip-h about x = 186: the members swap places.
  const groupFlip = pick(report, 'menu', { 'contents.item': 'flip-h', 'state.revision': 27 });
  obj(groupFlip, 'Im1', IM3, { swap: false, sx: -1, sy: 1 }, null, 'group flip-h');
  obj(groupFlip, 'Im3', IM1, ID, null, 'group flip-h');
  pristine(next(groupFlip), 'group flip-h undo');

  // §9.22 rotate-fit: the group menu disables Crop; Im1 stretched to 638 pt tall cannot rotate (the menu items are exact in the script).
  const groupMenu = pick(report, 'menu', { 'state.direct.menu.count': 2 });
  assert.equal(groupMenu.state.direct.menu.items.crop, false, 'group menu: crop disabled');
  const tall = pick(report, 'handle', { 'contents.handle': 's', 'state.revision': 29 });
  obj(tall, 'Im1', [72, 142, 172, 780], ID, null, 'Im1 s +588');
  const tallMenu = next(tall);
  assert.deepEqual([tallMenu.state.direct.menu.items.rotateCw, tallMenu.state.direct.menu.items.rotateCcw], [false, false], 'rotate does not fit');
  pristine(pick(report, 'undo', { 'state.revision': 30 }), 'Im1 s undo');

  // Pass-through and read mode leave the document untouched.
  pristine(pick(report, 'click', { 'contents.x': 74, 'contents.y': 600 }), 'Fm1 band click');
  pristine(pick(report, 'mode', { 'contents.mode': 'read' }), 'read mode');

  // direct3.pdf: one rotate + undo on a fixture without the new features.
  const d3 = pick(report, 'menu', { 'contents.item': 'rotate-cw', 'state.doc.name': 'direct3.pdf' });
  assert.equal(d3.state.revision, 1);
  const d3Im1 = d3.state.content.objects.find(o => o.name === 'Im1');
  near(d3Im1.view, [97, 117, 147, 217], 0.5, 'direct3 Im1 rotate-cw');
  assert.deepEqual(d3Im1.orient, { swap: true, sx: 1, sy: -1 });
  near(next(d3).state.content.objects.find(o => o.name === 'Im1').view, IM1, 0.5, 'direct3 undo');
  assert.deepEqual(next(d3).state.content.objects.find(o => o.name === 'Im1').orient, ID);

  assert.equal(await sha(OBJECTS6), before6, 'objects6.pdf unchanged');
  assert.equal(await sha(DIRECT3), before3, 'direct3.pdf unchanged');
});

// contract-p6-unit7.md §4.5 / §9.50 (U6 review TS4): the save→reopen tail as its own script; content.objects (targetId stripped)
// before save-to (step 16) deep-equals the reopened document's (step 19).
test('drive P6 U6: crop, flip and group rotate survive save-to and reopen', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  const SAVE = 'tests/drive/direct-edit7-save.jsonl';
  const before6 = await sha(OBJECTS6);
  assert.equal(before6, OBJECTS6_SHA);
  const header = (await readFile(join(root, SAVE), 'utf8')).match(/(\d+) steps, (\d+) click lines/);
  assert.ok(header, 'script header pins the step and click counts');
  assert.deepEqual([Number(header[1]), Number(header[2])], [19, 5]);
  const { code, report } = await drive(SAVE, 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 19, report.error ?? '');
  assert.equal(steps.filter(step => step.cmd === 'click').length, 5);
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);
  const strip = step => step.state.content.objects.map(({ targetId, ...rest }) => rest);
  const saved = steps[15], reopened = steps[18];
  assert.equal(saved.cmd, 'key');
  assert.equal(reopened.cmd, 'mode');
  assert.equal(reopened.state.doc.name, 'o6.pdf');
  assert.equal(saved.state.content.objects.length, 6);
  assert.deepEqual(strip(reopened), strip(saved));
  const im1 = all(reopened, 'Im1');
  assert.equal(im1.length, 1, 'reopened: one Im1');
  near(im1[0], [161, 53, 211, 113], 0.5, 'reopened Im1 view (cropped, then group rotate-cw)');
  assert.deepEqual(reopened.state.content.objects.find(o => o.name === 'Im1').orient, { swap: true, sx: 1, sy: -1 });
  assert.equal(await sha(OBJECTS6), before6, 'objects6.pdf unchanged');
});
