/**
 * P6 U5 drive (contract-p6-unit5.md §4.2 as amended by §9.29, §9.30, §9.33): tests/drive/direct-edit6.jsonl holds the
 * exact-value expects; the geometry (group moves, the page clamp, duplicate offsets) is checked here, ±0.5 pt on `view`
 * (y-down) boxes. A duplicated name is never looked up with find: all() returns every box of that name, sorted.
 * drive()/pick()/near() are copies of tests/p6-drive3.test.mjs (not shared on purpose).
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
const NO_APP = process.env.TANDEM_NO_APP ? 'TANDEM_NO_APP' : false;
const SCRIPT = 'tests/drive/direct-edit6.jsonl';
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
/** §9.30: every `view` box of one name, sorted by x then y (never find: a duplicate shares the name). */
const all = (step, name) => step.state.content.objects.filter(o => o.name === name).map(o => o.view).sort((a, b) => a[0] - b[0] || a[1] - b[1]);
/** Exactly `want.length` boxes of `name`, each within ±0.5 pt. */
function boxes(step, name, want, label) {
  const got = all(step, name);
  assert.equal(got.length, want.length, `${label}: ${name} count ${JSON.stringify(got)}`);
  want.forEach((box, i) => near(got[i], box, 0.5, `${label}: ${name}[${i}]`));
}
const IM1 = [72, 142, 172, 192];
const IM2 = [400, 52, 460, 92];
const FM1 = [400, 262, 500, 292];
const pristine = (step, label) => {
  boxes(step, 'Im1', [IM1], label);
  boxes(step, 'Im2', [IM2], label);
  boxes(step, 'Fm1', [FM1], label);
};
/** §2.3/§9.18: the selection is the copies, not the originals: each member's targetId is the object whose view is a copy box. */
function selectsCopies(step, copies, label) {
  const ids = copies.map(box => {
    const hit = step.state.content.objects.filter(o => o.view.every((v, i) => Math.abs(v - box[i]) <= 0.5));
    assert.equal(hit.length, 1, `${label}: one object at ${JSON.stringify(box)}`);
    return hit[0].targetId;
  });
  assert.deepEqual(step.state.direct.selection.map(member => member.targetId).sort(), ids.sort(), `${label}: selection is the copies`);
}
const red = steps => steps.filter(step => step.expect !== undefined && !step.ok);
const brief = step => ({ cmd: step.cmd, contents: step.contents, direct: step.state?.direct && { active: step.state.direct.active, selection: step.state.direct.selection, menu: step.state.direct.menu, objects: step.state.direct.objects }, edited: step.state?.content?.edited, revision: step.state?.revision, dirty: step.state?.dirty, error: step.state?.error });

test('drive P6 U5: Shift multi-select, group move/delete, ⌥-drag duplicate, context menu', { skip: NO_APP, timeout: 180_000 }, async () => {
  const before = await sha(DIRECT3);
  assert.equal(before, DIRECT3_SHA);
  // §9.33: the counts come from the script header ("59 steps, 22 click lines").
  const header = (await readFile(join(root, SCRIPT), 'utf8')).match(/(\d+) steps, (\d+) click lines/);
  assert.ok(header, 'script header pins the step and click counts');
  const { code, report } = await drive(SCRIPT, 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 59, report.error ?? '');
  assert.equal(steps.length, Number(header[1]));
  assert.equal(steps.filter(step => step.cmd === 'click').length, 22);
  assert.equal(Number(header[2]), 22);
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);
  const next = step => steps[steps.indexOf(step) + 1];
  pristine(steps[1], 'edit mode');

  // §4.2 group body drag +30,+20: both members move, Fm1 stays; undo restores both in one step.
  const moved = pick(report, 'drag', { 'contents.dx': 30, 'contents.dy': 20 });
  boxes(moved, 'Im1', [[102, 162, 202, 212]], 'group drag');
  boxes(moved, 'Im2', [[430, 72, 490, 112]], 'group drag');
  boxes(moved, 'Fm1', [FM1], 'group drag');
  pristine(next(moved), 'group drag undo');

  // §4.2 dy -80 clamps the union at the top: shared delta -52.
  const clamped = pick(report, 'drag', { 'contents.dy': -80 });
  boxes(clamped, 'Im2', [[400, 0, 460, 40]], 'clamp');
  boxes(clamped, 'Im1', [[72, 90, 172, 140]], 'clamp');
  pristine(next(clamped), 'clamp undo');

  // Group Delete removes Im1 and Fm1 only.
  const deleted = pick(report, 'key', { 'contents.key': 'Delete' });
  boxes(deleted, 'Im1', [], 'group delete');
  boxes(deleted, 'Fm1', [], 'group delete');
  boxes(deleted, 'Im2', [IM2], 'group delete');
  pristine(next(deleted), 'group delete undo');

  // §9.30 group Alt+ArrowRight: +1 pt each.
  const nudged = pick(report, 'key', { 'contents.key': 'Alt+ArrowRight' });
  boxes(nudged, 'Im1', [[73, 142, 173, 192]], 'group nudge');
  boxes(nudged, 'Im2', [[401, 52, 461, 92]], 'group nudge');
  pristine(next(nudged), 'group nudge undo');

  // Group Enter changes nothing.
  pristine(pick(report, 'key', { 'contents.key': 'Enter' }), 'group Enter');

  // ⌥-drag Im2 +60,+40: the copy lands at the drop point, the original stays.
  const altDrag = pick(report, 'drag', { 'contents.alt': true, 'contents.dx': 60 });
  boxes(altDrag, 'Im2', [IM2, [460, 92, 520, 132]], '⌥-drag');
  boxes(altDrag, 'Im1', [IM1], '⌥-drag');
  selectsCopies(altDrag, [[460, 92, 520, 132]], '⌥-drag');
  pristine(next(altDrag), '⌥-drag undo');

  // Menu duplicate on Im2: +10,+10.
  const menuDup = pick(report, 'menu', { 'contents.item': 'duplicate', 'state.revision': 11 });
  boxes(menuDup, 'Im2', [IM2, [410, 62, 470, 102]], 'menu duplicate');
  selectsCopies(menuDup, [[410, 62, 470, 102]], 'menu duplicate');
  pristine(next(menuDup), 'menu duplicate undo');

  // A refused menu item writes nothing; the menu delete on Fm1 removes only Fm1.
  pristine(pick(report, 'menu', { 'contents.item': 'replace' }), 'menu replace refused');
  const menuDel = pick(report, 'menu', { 'contents.item': 'delete' });
  boxes(menuDel, 'Fm1', [], 'menu delete');
  boxes(menuDel, 'Im1', [IM1], 'menu delete');
  boxes(menuDel, 'Im2', [IM2], 'menu delete');
  pristine(next(menuDel), 'menu delete undo');

  // §9.30 group menu duplicate: each member copied +10,+10.
  const groupDup = pick(report, 'menu', { 'contents.item': 'duplicate', 'state.revision': 15 });
  boxes(groupDup, 'Im1', [IM1, [82, 152, 182, 202]], 'group menu duplicate');
  boxes(groupDup, 'Im2', [IM2, [410, 62, 470, 102]], 'group menu duplicate');
  boxes(groupDup, 'Fm1', [FM1], 'group menu duplicate');
  selectsCopies(groupDup, [[82, 152, 182, 202], [410, 62, 470, 102]], 'group menu duplicate');
  // §9.28: undo re-resolves the members to the originals.
  const groupDupUndo = next(groupDup);
  assert.deepEqual(groupDupUndo.state.direct.selection.map(m => m.targetId).sort(),
    groupDupUndo.state.content.objects.filter(o => o.name === 'Im1' || o.name === 'Im2').map(o => o.targetId).sort(), 'undo selects the originals');
  pristine(next(groupDup), 'group menu duplicate undo');

  // §9.29 group ⌥-drag +20,+20.
  const groupAlt = pick(report, 'drag', { 'contents.alt': true, 'contents.dx': 20 });
  boxes(groupAlt, 'Im1', [IM1, [92, 162, 192, 212]], 'group ⌥-drag');
  boxes(groupAlt, 'Im2', [IM2, [420, 72, 480, 112]], 'group ⌥-drag');
  selectsCopies(groupAlt, [[92, 162, 192, 212], [420, 72, 480, 112]], 'group ⌥-drag');
  pristine(next(groupAlt), 'group ⌥-drag undo');

  // §9.17 keyboard menu: Shift+F10, ArrowDown, Enter duplicates Im1 +10,+10 and selects the copy.
  const keyDup = pick(report, 'key', { 'contents.key': 'Enter', 'state.revision': 19 });
  boxes(keyDup, 'Im1', [IM1, [82, 152, 182, 202]], 'keyboard menu duplicate');
  boxes(keyDup, 'Im2', [IM2], 'keyboard menu duplicate');
  selectsCopies(keyDup, [[82, 152, 182, 202]], 'keyboard menu duplicate');
  pristine(next(keyDup), 'keyboard menu duplicate undo');

  assert.equal(await sha(DIRECT3), before, 'direct3.pdf unchanged');
});
