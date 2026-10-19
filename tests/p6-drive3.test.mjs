/**
 * P6 U3 drive (contract-p6-unit3.md §6.2, §9.12): tests/drive/direct-edit3.jsonl holds the exact-value
 * expects; the geometry (boxes ±0.05 pt, the 2:1 corner ratio, nudges, the 40 pt mark move) is checked
 * here through pick(). drive()/pick()/savedBytes() are copies of tests/p3-drive.test.mjs (not shared on purpose).
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { mkdtemp as makeTempDir, readFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';

const root = fileURLToPath(new URL('..', import.meta.url));

/** Every temp dir this file makes (housekeeping B): removed after all its tests, never before their asserts. */
const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));
const CI_NO_ELECTRON = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;

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

async function savedBytes(dir, name) {
  return (await stat(join(dir, name))).size;
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

const DIRECT3_FIXTURES = {
  'fixtures/p6/direct3.pdf': 'e3ae5a43d9fb17e5b164c00b529d5be50ba4d9178de6c11b66252cc240297466',
  'fixtures/img-rgb.png': null, // read by pick-images on the replace step; only before == after
};
const sha = async path => createHash('sha256').update(await readFile(join(root, path))).digest('hex');

/** Every number within ±tol (pt). */
function near(got, want, tol, label) {
  assert.ok(Array.isArray(got) && got.length === want.length && got.every((value, i) => Math.abs(value - want[i]) <= tol),
    `${label}: ${JSON.stringify(got)} vs ${JSON.stringify(want)} ±${tol}`);
}
const object = (step, name) => step.state.content.objects.find(item => item.name === name);

test('drive P6 U3: objects, add text and redaction marks on the page', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  const before = Object.fromEntries(await Promise.all(Object.keys(DIRECT3_FIXTURES).map(async path => [path, await sha(path)])));
  const { code, report, dir } = await drive('tests/drive/direct-edit3.jsonl', 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 79, report.error ?? '');
  assert.equal(steps.filter(step => step.cmd === 'click').length, 15);
  const red = steps.filter(step => step.expect !== undefined && !step.ok);
  assert.equal(red.length, 0, JSON.stringify(red.map(step => ({ cmd: step.cmd, contents: step.contents, direct: step.state?.direct, content: step.state?.content?.edited, revision: step.state?.revision, error: step.state?.error, redact: step.state?.redact }))));
  assert.equal(code, 0);
  const next = step => steps[steps.indexOf(step) + 1];

  // §1.5 Im2 grip move dx 50 dy 30 (y-down) → y-up box +50/−30; undo restores it.
  const grip = pick(report, 'handle', { 'contents.x': 430, 'contents.handle': 'grip' });
  near(object(grip, 'Im2').box, [450, 670, 510, 710], 0.05, 'Im2 moved (y-up)');
  near(object(grip, 'Im2').view, [450, 82, 510, 122], 0.05, 'Im2 moved (y-down)');
  near(object(next(grip), 'Im2').box, [400, 700, 460, 740], 0.05, 'Im2 after undo');

  // §9.8 corner keeps the picture's ratio: Im1 100×50 se dx 20 dy 20 → 140×70, anchor (72, 650) fixed.
  const resize = pick(report, 'handle', { 'contents.x': 122, 'contents.handle': 'se' });
  const im1 = object(resize, 'Im1').box;
  near(im1, [72, 580, 212, 650], 0.05, 'Im1 corner resize');
  assert.ok(Math.abs((im1[2] - im1[0]) - 2 * (im1[3] - im1[1])) <= 0.1, `Im1 keeps 2:1: ${JSON.stringify(im1)}`);
  const escaped = pick(report, 'handle', { 'contents.escape': true });
  assert.deepEqual(object(escaped, 'Im1').box, im1, 'Esc mid-drag leaves the box');
  assert.equal(escaped.state.revision, resize.state.revision);

  // Fm1: body drag, then a free corner resize (Forms do not keep the ratio), both undone.
  const formMove = pick(report, 'drag', { 'contents.x': 450 });
  near(object(formMove, 'Fm1').box, [380, 500, 480, 530], 0.05, 'Fm1 body drag');
  const formResize = pick(report, 'handle', { 'contents.x': 430, 'contents.handle': 'se' });
  near(object(formResize, 'Fm1').box, [380, 480, 490, 530], 0.05, 'Fm1 free corner resize');
  near(object(next(next(formResize)), 'Fm1').box, [400, 500, 500, 530], 0.05, 'Fm1 after two undos');

  // §9.11 direct.active for a focused object; targetId is a string (cm rewrites change it).
  const focused = pick(report, 'click', { 'contents.x': 430, 'contents.y': 72 }).state.direct.active;
  assert.equal(typeof focused.targetId, 'string');
  near(focused.placement, [400, 700, 460, 740], 0.05, 'focused Im2 placement');
  assert.ok(focused.ops.includes('delete') && focused.ops.includes('replace'), JSON.stringify(focused.ops));
  assert.ok(['left', 'top', 'width', 'height'].every(key => typeof focused.box[key] === 'number'), JSON.stringify(focused.box));
  const deleted = pick(report, 'key', { 'contents.key': 'Delete', 'state.content.edited.action': 'delete' });
  assert.equal(deleted.state.content.objects.length, 2);
  assert.equal(object(deleted, 'Im2'), undefined);

  // §1.6 two ⌥→ lines total exactly 2 pt; ⌥⌘→ widens by 1 pt.
  const [nudge1, nudge2] = steps.filter(step => step.cmd === 'key' && step.contents?.key === 'Alt+ArrowRight');
  near(object(nudge1, 'Im2').box, [401, 700, 461, 740], 0.05, 'first nudge');
  near(object(nudge2, 'Im2').box, [402, 700, 462, 740], 0.05, 'second nudge');
  near(object(pick(report, 'key', { 'contents.key': 'Alt+Meta+ArrowRight' }), 'Im2').box, [402, 700, 463, 740], 0.05, 'width +1');

  // §2 add text where you click: the editor sits at the click point; one run lands there.
  const addClick = pick(report, 'click', { 'contents.x': 300, 'contents.y': 500 });
  // The drive click lands on a whole CSS pixel, so the point is within half a pixel (< 0.5 pt at ≥ 1 px/pt).
  near(addClick.state.direct.active.point, [300, 500], 0.5, 'add point');
  const added = pick(report, 'key', { 'contents.key': 'Meta+Enter' }).state;
  assert.ok(typeof added.content.lastAdd?.targetId === 'string' && added.content.lastAdd.targetId !== '', JSON.stringify(added.content.lastAdd));
  const run = added.content.bounds.find(item => item.targetId === added.content.lastAdd.targetId);
  assert.ok(run && Math.abs(run.bounds[0] - 300) <= 0.5 && run.bounds[1] >= 490 && run.bounds[1] <= 505, JSON.stringify(run));
  assert.equal(added.content.bounds.length, addClick.state.content.bounds.length + 1);
  const cancelled = pick(report, 'key', { 'contents.key': 'Escape', 'state.revision': 15 });
  assert.equal(cancelled.state.content.bounds.length, added.content.bounds.length, 'Escape wrote nothing');
  const rotated = pick(report, 'click', { 'contents.page': 1 }).state;
  assert.deepEqual(rotated.content.refused.at(-1), { text: '', detail: 'unsupported-transform' });

  // §3 marks: the area mark moves 40 pt with its width unchanged; the search mark becomes an area mark.
  const area = pick(report, 'redact', { 'contents.page': 0 }).state.redact.rects[0].rect;
  near(area, [300, 600, 400, 650], 0.05, 'area mark');
  const markMove = pick(report, 'handle', { 'contents.x': 350 });
  const moved = markMove.state.redact.rects[0].rect;
  near(moved, [area[0] + 40, area[1], area[2] + 40, area[3]], 0.05, 'area mark moved 40 pt');
  assert.ok(Math.abs((moved[2] - moved[0]) - (area[2] - area[0])) <= 0.05, 'width unchanged');
  const searchBefore = markMove.state.redact.rects[1].rect;
  const searchAfter = pick(report, 'handle', { 'contents.x': 100.78 }).state.redact.rects[1].rect;
  near(searchAfter, [searchBefore[0], searchBefore[1] + 100, searchBefore[2], searchBefore[3] + 100], 0.05, 'search mark moved');
  const markFocus = pick(report, 'click', { 'contents.x': 390 }).state.direct.active;
  assert.equal(typeof markFocus.annotId, 'number');
  near(markFocus.rect, moved, 0.05, 'focused mark rect');
  const markNudge = steps.filter(step => step.cmd === 'key' && step.contents?.key === 'Alt+ArrowRight')[2];
  near(markNudge.state.redact.rects[0].rect, [moved[0] + 1, moved[1], moved[2] + 1, moved[3]], 0.05, 'mark nudge 1 pt');

  // Persistence: save → reopen → Edit text shows the edited placements.
  const save = pick(report, 'save-to').state.save;
  assert.ok(save.bytes > 0);
  assert.equal(await savedBytes(dir, 'direct3.pdf'), save.bytes);
  const reopened = next(steps.find(step => step.cmd === 'open' && step.path !== 'fixtures/p6/direct3.pdf' && basename(step.path) === 'direct3.pdf'));
  assert.equal(reopened.cmd, 'mode', 'the Edit text step follows the reopen');
  assert.equal(reopened.state.content.objects.length, 3);
  near(object(reopened, 'Im1').box, [72, 580, 212, 650], 0.05, 'Im1 persisted');
  near(object(reopened, 'Im2').box, [402, 700, 463, 740], 0.05, 'Im2 persisted');
  near(object(reopened, 'Fm1').box, [400, 500, 500, 530], 0.05, 'Fm1 persisted');
  assert.equal((await readFile(join(dir, 'direct3 redacted.pdf'))).subarray(0, 5).toString(), '%PDF-');

  // Second pass (fresh open): §2.3 a refused add keeps the editor and draft and writes nothing; undo of an add.
  const second = steps.findLastIndex(step => step.cmd === 'open' && step.path === 'fixtures/p6/direct3.pdf');
  assert.ok(second > 0, 'second pass opens the fixture again');
  const tail = steps.slice(second);
  const refusedAdd = tail.find(step => step.cmd === 'key' && step.contents?.key === 'Meta+Enter');
  assert.deepEqual(refusedAdd.state.content.refused.at(-1), { text: '', detail: 'unsupported-encoding' });
  assert.equal(refusedAdd.state.direct.active.draft, '漢字');
  const committed = tail.filter(step => step.cmd === 'key' && step.contents?.key === 'Meta+Enter')[1];
  assert.equal(committed.state.content.bounds.length, refusedAdd.state.content.bounds.length + 1, 'the add wrote one run');
  assert.equal(next(committed).state.content.bounds.length, refusedAdd.state.content.bounds.length, 'undo took the add back');
  // §3.2 mark se handle, Esc mid-drag, ⌥⇧→ 10 pt.
  const markResize = tail.find(step => step.cmd === 'handle' && step.contents?.handle === 'se' && !step.contents.escape);
  near(markResize.state.redact.rects[0].rect, [300, 600, 410, 660], 0.05, 'mark se resize');
  near(next(markResize).state.redact.rects[0].rect, [300, 600, 410, 660], 0.05, 'mark Esc mid-drag');
  near(tail.find(step => step.contents?.key === 'Alt+Shift+ArrowRight').state.redact.rects[0].rect, [310, 600, 420, 660], 0.05, 'mark nudge 10 pt');

  // §9.8 Shift frees the aspect ratio: Im1 se dx 20 dy 0 → 120×50 (review F1).
  near(object(pick(report, 'handle', { 'contents.shift': true }), 'Im1').box, [72, 600, 192, 650], 0.05, 'Shift frees the ratio');

  assert.equal(before['fixtures/p6/direct3.pdf'], DIRECT3_FIXTURES['fixtures/p6/direct3.pdf']);
  for (const [path, hash] of Object.entries(before)) assert.equal(await sha(path), hash, path);
});
