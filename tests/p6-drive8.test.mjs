/**
 * P6 U7 drive (contract-p6-unit7.md §4.2 as amended by §9.49, §10.10, §10.13, §10.45): tests/drive/direct-edit8.jsonl holds the exact-value
 * expects; the geometry is checked here, ±0.5 pt, all in view space (y down): objects by `content.objects[].view` via all(), runs by
 * `content.bounds[].bounds`, fields by `fields.defs[name].rect`, links by `links[i].rect`, marks by `redact.rects[i].rect`.
 * Steps are addressed by index (identical lines repeat across documents): S(n) is the n-th command of the script.
 * drive()/pick()/near()/all() are copies of tests/p6-drive7.test.mjs (not shared on purpose).
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
const SCRIPT = 'tests/drive/direct-edit8.jsonl';
const FIXTURES = {
  'fixtures/p6/objects6.pdf': 'ca0a5988eb81cd6b35916918695cd8c9881b8ebba82ae590e38cdf8a97fb2ab8',
  'fixtures/p6/direct3.pdf': 'e3ae5a43d9fb17e5b164c00b529d5be50ba4d9178de6c11b66252cc240297466',
  'fixtures/p3/field-defs.pdf': '43782ca55aa415cc9fc09406d95e9300326ad281fac5a0176b14c88a295f4250',
  'fixtures/p6/links.pdf': '0ac413b0fa9b42062542afbc970f0245bceacab0a789017c90a413733962d0bc',
  'fixtures/hello.pdf': '229ca9488a139d3f32b864ba24055152fe7f0faf670c9db6491144f23d275b94',
};

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
const red = steps => steps.filter(step => step.expect !== undefined && !step.ok);
const brief = step => ({ cmd: step.cmd, contents: step.contents, direct: step.state?.direct && { active: step.state.direct.active, selection: step.state.direct.selection, menu: step.state.direct.menu, clipboard: step.state.direct.clipboard, objects: step.state.direct.objects }, edited: step.state?.content?.edited, revision: step.state?.revision, dirty: step.state?.dirty, error: step.state?.error, announcement: step.state?.announcement });

const ID = { swap: false, sx: 1, sy: 1 };
/** `name` has exactly one box, `view` ±0.5; `orient` deep-equal when given. */
function one(step, name, view, orient, label) {
  const got = all(step, name);
  assert.equal(got.length, 1, `${label}: ${name} count ${JSON.stringify(got)}`);
  near(got[0], view, 0.5, `${label}: ${name} view`);
  if (orient) assert.deepEqual(step.state.content.objects.find(o => o.name === name).orient, orient, `${label}: ${name} orient`);
}
/** The one entry of `rects` within ±0.5 of `want` (list of boxes). */
function has(rects, want, label) {
  const hits = rects.filter(r => Array.isArray(r) && r.length === 4 && r.every((v, i) => Math.abs(v - want[i]) <= 0.5));
  assert.equal(hits.length, 1, `${label}: one box at ${JSON.stringify(want)} in ${JSON.stringify(rects)}`);
}
const runs = step => step.state.content.bounds.map(b => b.bounds);
const linkRects = step => step.state.links.map(l => l.rect);
const markRects = step => step.state.redact.rects.map(m => m.rect);
const MOVE_ME = [72, 376.95, 129.57, 396.19];
const IM1 = [72, 142, 172, 192];

test('drive P6 U7: cut / copy / paste and multi-select of runs, fields, links and marks', { skip: CI_NO_ELECTRON, timeout: 300_000 }, async () => {
  for (const [path, want] of Object.entries(FIXTURES)) assert.equal(await sha(path), want, `${path} before`);
  // §10.10: the counts come from the script header, re-counted from the final file.
  const text = await readFile(join(root, SCRIPT), 'utf8');
  const header = text.match(/(\d+) steps, (\d+) click lines/);
  assert.ok(header, 'script header pins the step and click counts');
  const lines = text.split('\n').filter(line => line.trim() && !line.trim().startsWith('#'));
  assert.deepEqual([Number(header[1]), Number(header[2])], [115, 26]);
  assert.equal(lines.length, 115);
  const t0 = Date.now();
  const { code, report } = await drive(SCRIPT, 240_000);
  console.log(`direct-edit8 wall ${Date.now() - t0} ms`);
  const steps = report.steps;
  assert.equal(steps.length, 115, report.error ?? '');
  assert.equal(steps.filter(step => step.cmd === 'click').length, 26);
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);
  const S = n => steps[n - 1];

  // §9.46: a Shift-click never opens an editor.
  for (const step of steps.filter(s => s.cmd === 'click' && s.contents?.shift)) assert.equal(step.state.direct.active?.draft, undefined, `Shift-click ${JSON.stringify(step.contents)} opened an editor`);

  // objects6 p0: paste onto the source page lands one Duplicate step off (+10,+10 view); Im1 untouched.
  assert.equal(S(8).state.doc.name, 'objects6.pdf');
  one(S(8), 'PA1', [82, 152, 182, 202], ID, 'same-page paste');
  one(S(8), 'Im1', IM1, ID, 'same-page paste');
  // Cut Im2, page-menu paste on p1 at (300,400): the union's top-left at the click point, Im2's rotation kept; p1's own Im1 untouched.
  assert.equal(S(14).state.page, 1);
  one(S(14), 'PA1', [300, 400, 350, 500], { swap: true, sx: 1, sy: -1 }, 'page-1 paste');
  one(S(14), 'Im1', IM1, ID, 'page-1 own Im1');
  // Fm1 copy + paste (§9.4: no child, objects 7).
  assert.equal(S(20).state.content.objects.length, 7);
  one(S(20), 'PA1', [82, 502, 382, 702], null, 'Fm1 paste');
  assert.equal(S(20).state.content.objects.find(o => o.name === 'PA1').kind, 'form');
  one(S(21), 'Fm1', [72, 492, 372, 692], ID, 'Fm1 paste undo');
  assert.equal(all(S(21), 'PA1').length, 0);

  // direct3: the cross-document form paste lands at its source box (no occupant).
  assert.equal(S(25).state.doc.name, 'direct3.pdf');
  one(S(25), 'PA1', [72, 492, 372, 692], null, 'cross-document paste');
  // Mixed run + image nudge: both +1 pt.
  one(S(32), 'Im1', [73, 142, 173, 192], ID, 'mixed ArrowRight');
  has(runs(S(32)), [73, 376.95, 130.57, 396.19], 'mixed ArrowRight "Move me"');
  one(S(33), 'Im1', IM1, ID, 'mixed undo');
  has(runs(S(33)), MOVE_ME, 'mixed undo "Move me"');
  // Text paste on p2: page menu at (300,500) → the run's top-left there; ⌘V → the source box (§10.36).
  assert.equal(S(41).state.content.bounds.length, 1);
  has(runs(S(43)), [300, 500, 357.57, 519.24], 'page-menu text paste');
  const pasted = S(43).state.content.bounds.find(b => Math.abs(b.bounds[0] - 300) <= 0.5);
  assert.match(pasted.targetId, /\.6c51e7e2$/, 'pasted run keeps the payload hash');
  assert.equal(S(44).state.content.bounds.length, 1);
  has(runs(S(45)), MOVE_ME, '⌘V text paste');
  // Debt sweep 2 §9.12 (T-5): the ⌘V run keeps the payload hash too.
  const pasted45 = S(45).state.content.bounds.find(b => b.bounds.every((v, i) => Math.abs(v - MOVE_ME[i]) <= 0.5));
  assert.match(pasted45?.targetId ?? '', /\.6c51e7e2$/, '⌘V pasted run keeps the payload hash');

  // field-defs: drag nom + plain by (20,0).
  near(S(52).state.fields.defs.nom.rect, [92, 112, 272, 142], 0.5, 'nom after group drag');
  near(S(52).state.fields.defs.plain.rect, [92, 247, 212, 272], 0.5, 'plain after group drag');
  near(S(53).state.fields.defs.nom.rect, [72, 112, 252, 142], 0.5, 'nom after undo');
  near(S(53).state.fields.defs.plain.rect, [72, 247, 192, 272], 0.5, 'plain after undo');
  assert.equal(S(57).state.fields.defs.nom, undefined);
  assert.equal(S(57).state.fields.defs.plain, undefined);
  near(S(62).state.fields.defs['plain copy'].rect, [82, 257, 202, 282], 0.5, 'plain copy');
  near(S(62).state.fields.defs.plain.rect, [72, 247, 192, 272], 0.5, 'plain after paste');
  assert.equal(S(63).state.fields.defs['plain copy'], undefined);

  // links: ArrowDown moves L1 + L2 by 1 pt; L3 stays.
  const l69 = S(69).state.links;
  near(l69.find(l => l.annotId === 10).rect, [72, 73, 200, 93], 0.5, 'L1 ArrowDown');
  near(l69.find(l => l.annotId === 11).rect, [72, 123, 200, 143], 0.5, 'L2 ArrowDown');
  near(l69.find(l => l.annotId === 13).rect, [72, 172, 200, 192], 0.5, 'L3 unchanged');
  // Paste of L1 + L2 on their own page: one Duplicate step.
  assert.equal(S(80).state.links.length, 5);
  has(linkRects(S(80)), [82, 82, 210, 102], 'pasted L1');
  has(linkRects(S(80)), [82, 132, 210, 152], 'pasted L2');
  has(linkRects(S(80)), [72, 72, 200, 92], 'L1 stays');
  // hello: L1 pasted at the page-menu point, L2 skipped.
  assert.equal(S(86).state.doc.name, 'hello.pdf');
  near(S(86).state.links[0].rect, [300, 400, 428, 420], 0.5, 'hello pasted L1');

  // marks: drawn, nudged +1, copied, pasted one Duplicate step off.
  const m90 = markRects(S(90));
  assert.equal(m90.length, 2);
  has(m90, [300, 500, 400, 530], 'mark 1 drawn');
  has(m90, [300, 600, 400, 630], 'mark 2 drawn');
  has(markRects(S(93)), [301, 500, 401, 530], 'mark 1 ArrowRight');
  has(markRects(S(93)), [301, 600, 401, 630], 'mark 2 ArrowRight');
  has(markRects(S(96)), [300, 500, 400, 530], 'mark 1 after undo ×2');
  has(markRects(S(96)), [300, 600, 400, 630], 'mark 2 after undo ×2');
  const m100 = markRects(S(100));
  assert.equal(m100.length, 3);
  has(m100, [310, 510, 410, 540], 'pasted mark');
  has(m100, [300, 500, 400, 530], 'source mark stays');

  // Review F1 tail (direct3, S105-S115): the mixed [run, Im1] selection moves +10 (Shift+ArrowRight) then +30 (drag on the run), both members kept.
  assert.equal(S(105).state.doc.name, 'direct3.pdf');
  one(S(109), 'Im1', [82, 142, 182, 192], ID, 'F1 Shift+ArrowRight');
  has(runs(S(109)), [82, 376.95, 139.57, 396.19], 'F1 Shift+ArrowRight "Move me"');
  one(S(110), 'Im1', [112, 142, 212, 192], ID, 'F1 drag');
  has(runs(S(110)), [112, 376.95, 169.57, 396.19], 'F1 drag "Move me"');
  for (const n of [109, 110, 111, 112]) assert.deepEqual(S(n).state.direct.selection.map(m => m.kind), ['text', 'image'], `S${n} keeps both members`);
  assert.equal(S(111).state.content.objects.length, 3, 'F1 Delete refused: Im1 kept');
  one(S(114), 'Im1', IM1, ID, 'F1 undo ×2');
  has(runs(S(114)), MOVE_ME, 'F1 undo ×2 "Move me"');

  for (const [path, want] of Object.entries(FIXTURES)) assert.equal(await sha(path), want, `${path} after`);
});

/** Header counts of a drive script, re-counted from its lines. */
async function counted(script, steps, clicks) {
  const text = await readFile(join(root, script), 'utf8');
  const header = text.match(/(\d+) steps, (\d+) click lines/);
  assert.ok(header, 'script header pins the step and click counts');
  assert.deepEqual([Number(header[1]), Number(header[2])], [steps, clicks]);
  const lines = text.split('\n').filter(line => line.trim() && !line.trim().startsWith('#'));
  assert.equal(lines.length, steps);
  assert.equal(lines.filter(line => line.includes('"cmd":"click"')).length, clicks);
}
async function run(script, steps, clicks) {
  await counted(script, steps, clicks);
  const { code, report } = await drive(script, 200_000);
  assert.equal(report.steps.length, steps, report.error ?? '');
  assert.equal(red(report.steps).length, 0, JSON.stringify(red(report.steps).map(brief)));
  assert.equal(code, 0);
  return n => report.steps[n - 1];
}

// Review T-2 (contract §0, §9.53): every pasted kind that is document content survives save-to → reopen with the same geometry.
test('drive P6 U7: pasted image, text run, field and links survive save-to and reopen', { skip: CI_NO_ELECTRON, timeout: 300_000 }, async () => {
  for (const [path, want] of Object.entries(FIXTURES)) assert.equal(await sha(path), want, `${path} before`);
  const S = await run('tests/drive/direct-edit8-save.jsonl', 42, 5);
  const strip = step => step.state.content.objects.map(({ targetId, ...rest }) => rest);
  // objects6: PA1 on page 0 (+10 off Im1) and on page 1 at the page-menu point.
  assert.equal(S(11).state.doc.name, 'o6p.pdf');
  one(S(5), 'PA1', [82, 152, 182, 202], ID, 'p0 paste');
  assert.deepEqual(strip(S(11)), strip(S(5)), 'o6p page 0 after reopen');
  one(S(11), 'PA1', [82, 152, 182, 202], ID, 'p0 paste after reopen');
  one(S(8), 'PA1', [300, 400, 400, 450], ID, 'p1 paste');
  assert.deepEqual(strip(S(12)), strip(S(8)), 'o6p page 1 after reopen');
  // direct3 p2: the run at the page-menu point (drive8 S43's box).
  assert.equal(S(23).state.doc.name, 'd3p.pdf');
  has(runs(S(19)), [300, 500, 357.57, 519.24], 'text paste');
  assert.deepEqual(S(23).state.content.bounds, S(19).state.content.bounds, 'd3p page 2 after reopen');
  // field-defs: "plain copy" one Duplicate step off plain (drive8 S62's box).
  assert.equal(S(32).state.doc.name, 'fdp.pdf');
  near(S(29).state.fields.defs['plain copy'].rect, [82, 257, 202, 282], 0.5, 'plain copy');
  assert.deepEqual(S(32).state.fields.defs, S(29).state.fields.defs, 'fdp defs after reopen');
  // links: L1 + L2 pasted one Duplicate step off (drive8 S80's boxes).
  assert.equal(S(42).state.doc.name, 'lp.pdf');
  has(linkRects(S(39)), [82, 82, 210, 102], 'pasted L1');
  has(linkRects(S(39)), [82, 132, 210, 152], 'pasted L2');
  assert.deepEqual(S(42).state.links, S(39).state.links, 'lp links after reopen');
  for (const [path, want] of Object.entries(FIXTURES)) assert.equal(await sha(path), want, `${path} after`);
});

// Review T-1 / T-3 (§9.22, §10.8, §10.21, §10.38, §10.42): Cut of non-object selections; the menu read on an unselected member per mode.
test('drive P6 U7: Cut of links, runs, run + image and a radio; the member menu in Annotate, Fill forms and Redact', { skip: CI_NO_ELECTRON, timeout: 300_000 }, async () => {
  for (const [path, want] of Object.entries(FIXTURES)) assert.equal(await sha(path), want, `${path} before`);
  const S = await run('tests/drive/direct-edit8-cut.jsonl', 51, 11);
  for (let n = 1; n <= 51; n++) if (S(n).cmd === 'click' && S(n).contents?.shift) assert.equal(S(n).state.direct.active?.draft, undefined, `S${n} Shift-click opened an editor`);
  // links: Cut L1 + L2 leaves L3; ⌘V puts them back at their source boxes; a single-link Cut removes L1 only.
  assert.deepEqual(linkRects(S(9)), [[72, 172, 200, 192]]);
  has(linkRects(S(10)), [72, 72, 200, 92], 'L1 pasted back');
  has(linkRects(S(10)), [72, 122, 200, 142], 'L2 pasted back');
  assert.deepEqual(S(15).state.links.map(l => l.annotId), [11, 13]);
  // direct3: the paragraph is not selectable; a runs-only Cut writes nothing; run + Im1 removes Im1 and keeps the run.
  assert.equal(S(22).state.content.bounds.length, 5);
  has(runs(S(22)), MOVE_ME, 'runs-only Cut keeps "Move me"');
  assert.equal(all(S(25), 'Im1').length, 0, 'Im1 cut');
  has(runs(S(25)), MOVE_ME, 'run stays after run + Im1 Cut');
  one(S(26), 'PA1', [82, 152, 182, 202], ID, '⌘V after run + Im1 Cut');
  one(S(28), 'Im1', IM1, ID, 'Im1 after undo ×2');
  // field-defs: the radio group's two kids go and come back under "color"; undo restores the original defs exactly.
  assert.equal(S(37).state.fields.defs.color, undefined);
  assert.equal(S(37).state.fields.defs['color.A'], undefined);
  assert.equal(S(38).state.fields.count, 5);
  assert.deepEqual(S(40).state.fields.defs, S(36).state.fields.defs, 'radio Cut + paste undone');
  // hello: the right-click selected the second mark alone and added none.
  assert.notEqual(S(47).state.direct.selection[0].targetId, S(46).state.direct.selection[0].targetId, 'menu target replaces the selection');
  assert.deepEqual(markRects(S(47)), [[300, 500, 400, 530], [300, 600, 400, 630]]);
  for (const [path, want] of Object.entries(FIXTURES)) assert.equal(await sha(path), want, `${path} after`);
});

// Debt sweep 2 T-4 (contract-debt2.md §3.1, §9.11): ⌘C, ⌘X and ⌘V as drive key lines; the Cut announcement agrees in number (§9.10).
test('drive debt2: ⌘C / ⌘X / ⌘V chords on objects6 and a run + image ⌘X on direct3', { skip: CI_NO_ELECTRON, timeout: 300_000 }, async () => {
  for (const [path, want] of Object.entries(FIXTURES)) assert.equal(await sha(path), want, `${path} before`);
  const S = await run('tests/drive/direct-edit8-chords.jsonl', 19, 5);
  for (let n = 1; n <= 19; n++) if (S(n).cmd === 'click' && S(n).contents?.shift) assert.equal(S(n).state.direct.active?.draft, undefined, `S${n} Shift-click opened an editor`);
  one(S(5), 'PA1', [82, 152, 182, 202], ID, '⌘V after ⌘C');
  assert.equal(S(10).state.announcement, 'Cut image Im2');
  assert.equal(all(S(10), 'Im2').length, 0, 'Im2 cut');
  one(S(11), 'Im2', [400, 42, 450, 142], { swap: true, sx: 1, sy: -1 }, 'Im2 after undo');
  assert.equal(S(17).state.announcement, 'Cut 1 item; 1 text run stays (it cannot be deleted)');
  assert.equal(all(S(17), 'Im1').length, 0, 'Im1 cut');
  has(runs(S(17)), MOVE_ME, 'run stays after run + Im1 ⌘X');
  one(S(18), 'Im1', IM1, ID, 'Im1 after undo');
  for (const [path, want] of Object.entries(FIXTURES)) assert.equal(await sha(path), want, `${path} after`);
});
