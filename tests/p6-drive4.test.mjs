/**
 * P6 U4 drive (contract-p6-unit4.md §6.2, §9.8, §9.12, §9.18): tests/drive/direct-edit4.jsonl (links + /Tabs) and
 * tests/drive/direct-edit5.jsonl (outlines, plain-arrow nudge, unarmed redact drag) hold the exact-value expects; the
 * geometry every gesture produced (moves, resizes, nudges, drawn rects, the rotated-page move, persistence) is checked
 * here through pick(). drive()/pick()/savedBytes()/near() are copies of tests/p6-drive3.test.mjs (not shared on purpose).
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
const NO_APP = process.env.TANDEM_NO_APP ? 'TANDEM_NO_APP' : false;

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

/** §9.18: the four fixtures the two scripts open; hashes before == after == pinned. */
const U4_FIXTURES = {
  'fixtures/p6/links.pdf': '0ac413b0fa9b42062542afbc970f0245bceacab0a789017c90a413733962d0bc',
  'fixtures/p6/tabs.pdf': 'b90c5db2ed9d4b1221b453bc9df02682fd8a6c678724263738a68d24bc047326',
  'fixtures/p6/direct3.pdf': 'e3ae5a43d9fb17e5b164c00b529d5be50ba4d9178de6c11b66252cc240297466',
  'fixtures/form2.pdf': 'f2d77d007f947e7daab0c2b9800b0a91df3ee843ef32f183b53b200b9d941164',
};
const sha = async path => createHash('sha256').update(await readFile(join(root, path))).digest('hex');
const hashes = async () => Object.fromEntries(await Promise.all(Object.keys(U4_FIXTURES).map(async path => [path, await sha(path)])));

/** Every number within ±tol (pt). */
function near(got, want, tol, label) {
  assert.ok(Array.isArray(got) && got.length === want.length && got.every((value, i) => Math.abs(value - want[i]) <= tol),
    `${label}: ${JSON.stringify(got)} vs ${JSON.stringify(want)} ±${tol}`);
}
const shift = (rect, dx, dy) => [rect[0] + dx, rect[1] + dy, rect[2] + dx, rect[3] + dy];
const red = steps => steps.filter(step => step.expect !== undefined && !step.ok);
const brief = step => ({ cmd: step.cmd, contents: step.contents, direct: step.state?.direct, links: step.state?.links, nav: step.state?.nav?.last, linkDraft: step.state?.nav?.linkDraft, fields: step.state?.fields && { tabs: step.state.fields.tabs, tabsOrder: step.state.fields.tabsOrder, order: step.state.fields.order }, redact: step.state?.redact?.rects, revision: step.state?.revision, error: step.state?.error });

test('drive P6 U4: links on the page and /Tabs order', { skip: NO_APP, timeout: 180_000 }, async () => {
  const before = await hashes();
  const { code, report, dir } = await drive('tests/drive/direct-edit4.jsonl', 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 71, report.error ?? '');
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);
  const prev = step => steps[steps.indexOf(step) - 1];
  const next = step => steps[steps.indexOf(step) + 1];
  const L1 = [72, 72, 200, 92];
  const link0 = step => step.state.links[0].rect;

  // §1.4 grip move dx 30 dy 40 (y-down page pt) → one link-set; undo restores the fixture rect.
  const grip = pick(report, 'handle', { 'contents.page': 0, 'contents.handle': 'grip' });
  near(link0(prev(grip)), L1, 0.05, 'L1 before the move');
  near(link0(grip), shift(L1, 30, 40), 0.05, 'L1 grip move');
  near(link0(next(grip)), L1, 0.05, 'L1 after undo');
  // se handle dx 20 dy 10: the far corner moves, the near one stays.
  const resize = pick(report, 'handle', { 'contents.handle': 'se' });
  near(link0(resize), [72, 72, 220, 102], 0.05, 'L1 se resize');
  // §3.1 plain → 1 pt, ⇧↓ 10 pt, each a delta from the previous step.
  const right = pick(report, 'key', { 'contents.key': 'ArrowRight' });
  near(link0(right), shift(link0(prev(right)), 1, 0), 0.05, 'L1 → 1 pt');
  const down = pick(report, 'key', { 'contents.key': 'Shift+ArrowDown' });
  near(link0(down), shift(link0(prev(down)), 0, 10), 0.05, 'L1 ⇧↓ 10 pt');
  // The retarget and the refused scheme leave the rect alone.
  const refused = pick(report, 'key', { 'state.nav.last.detail': 'scheme' });
  near(link0(refused), link0(down), 0.05, 'refusal wrote nothing');
  // R1 (§9.6): a mode change drops the changed editor and commits silently — refused, no #error text, rect untouched.
  const silent = pick(report, 'wait', { 'state.nav.last.detail': 'scheme' });
  assert.equal(prev(silent).cmd, 'mode', 'the silent commit follows a mode change');
  assert.equal(silent.state.error, '', 'the refusal never reaches #error');
  near(link0(silent), link0(down), 0.05, 'silent refusal wrote nothing');

  // §1.6 drawn links: the draft is the drag ±0.5 pt, and link-add writes exactly the draft.
  const draw1 = pick(report, 'drag', { 'contents.y': 400 });
  near(draw1.state.nav.linkDraft.rect, [300, 400, 420, 420], 0.5, 'first draft');
  near(draw1.state.nav.last.rect, draw1.state.nav.linkDraft.rect, 0.01, 'link-draw names the draft');
  near(next(draw1).state.links[3].rect, draw1.state.nav.linkDraft.rect, 0.05, 'first drawn link');
  const draw2 = pick(report, 'drag', { 'contents.y': 450 });
  near(draw2.state.nav.linkDraft.rect, [300, 450, 420, 470], 0.5, 'second draft');
  near(next(draw2).state.links[4].rect, draw2.state.nav.linkDraft.rect, 0.05, 'second drawn link');

  // Rotated page 1: grip dx 10 moves the page-space rect 10 pt right.
  const rotated = pick(report, 'handle', { 'contents.page': 1 });
  near(link0(prev(rotated)), [700, 72, 720, 200], 0.05, 'L4 before');
  near(link0(rotated), [710, 72, 730, 200], 0.05, 'L4 moved on the /Rotate 90 page');

  // Save → reopen: page 0 and page 1 rects equal the last pre-save STATE of each page (±0.01).
  const save = pick(report, 'save-to').state.save;
  assert.equal(await savedBytes(dir, 'links.pdf'), save.bytes);
  const page0Before = pick(report, 'link', { 'contents.action': 'set' }).state.links;
  const reopen = steps.findIndex(step => step.cmd === 'open' && basename(step.path ?? '') === 'links.pdf' && step.path !== 'fixtures/p6/links.pdf');
  assert.ok(reopen > 0, 'the saved copy is reopened');
  const page0After = steps[reopen + 1].state.links;
  assert.equal(page0After.length, page0Before.length);
  page0After.forEach((item, i) => near(item.rect, page0Before[i].rect, 0.01, `page 0 link ${i} persisted`));
  near(steps[reopen + 2].state.links[0].rect, link0(rotated), 0.01, 'L4 move persisted');

  const after = await hashes();
  for (const [path, hash] of Object.entries(U4_FIXTURES)) {
    assert.equal(before[path], hash, `${path} pinned`);
    assert.equal(after[path], hash, `${path} unchanged`);
  }
});

test('drive P6 U4: outlines, plain-arrow nudge, unarmed redact drag', { skip: NO_APP, timeout: 180_000 }, async () => {
  const before = await hashes();
  const { code, report } = await drive('tests/drive/direct-edit5.jsonl', 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 35, report.error ?? '');
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);
  const prev = step => steps[steps.indexOf(step) - 1];
  const next = step => steps[steps.indexOf(step) + 1];
  const im2 = step => step.state.content.objects.find(item => item.name === 'Im2').box; // y-up

  // §3.1 plain → +1 pt x, ⇧↓ 10 pt down (y-up box: y −10); two undos restore it.
  const right = pick(report, 'key', { 'contents.key': 'ArrowRight', 'state.doc.name': 'direct3.pdf' });
  near(im2(prev(right)), [400, 700, 460, 740], 0.05, 'Im2 before');
  near(im2(right), shift(im2(prev(right)), 1, 0), 0.05, 'Im2 → 1 pt');
  const down = pick(report, 'key', { 'contents.key': 'Shift+ArrowDown' });
  near(im2(down), shift(im2(right), 0, -10), 0.05, 'Im2 ⇧↓ 10 pt');
  near(im2(next(next(down))), [400, 700, 460, 740], 0.05, 'Im2 after two undos');

  // §9.12 checkbox ↑ in Forms mode: y-down rect −1 pt; undo restores it.
  const up = pick(report, 'key', { 'contents.key': 'ArrowUp' });
  near(prev(up).state.fields.defs.agree.rect, [160, 184, 178, 202], 0.05, 'agree before');
  near(up.state.fields.defs.agree.rect, [160, 183, 178, 201], 0.05, 'agree ↑ 1 pt');
  near(next(up).state.fields.defs.agree.rect, [160, 184, 178, 202], 0.05, 'agree after undo');
  // A radio's plain → moves the selection and no kid's rect.
  const radio = pick(report, 'key', { 'contents.key': 'ArrowRight', 'state.doc.name': 'form2.pdf' });
  for (const kid of ['size.S', 'size.M', 'size.L']) assert.deepEqual(radio.state.fields.defs[kid].rect, prev(radio).state.fields.defs[kid].rect, kid);

  // §3.3 the unarmed drag marks the dragged area (±0.5 pt, a drive point lands on a whole CSS px).
  const mark = pick(report, 'drag', { 'contents.y': 500 });
  near(mark.state.redact.rects[0].rect, [300, 500, 400, 530], 0.5, 'area mark');
  near(next(mark).state.redact.rects[0].rect, mark.state.redact.rects[0].rect, 0.01, 'a 1 px drag moves nothing');
  // §3.1 plain → on a focused mark: +1 pt x; the second undo puts it back. §3.3 armed Mark area: 1 px marks nothing, 100×30 marks it.
  const nudge = pick(report, 'key', { 'contents.key': 'ArrowRight', 'state.direct.active.kind': 'redact' });
  near(nudge.state.redact.rects[0].rect, shift(prev(nudge).state.redact.rects[0].rect, 1, 0), 0.01, 'mark → 1 pt');
  const armedTiny = pick(report, 'drag', { 'contents.y': 300, 'contents.dx': 1, 'state.revision': 2 });
  assert.equal(prev(armedTiny).cmd, 'arm', 'the 1 px drag is armed');
  assert.deepEqual(armedTiny.state.redact.rects, prev(armedTiny).state.redact.rects, 'an armed 1 px drag marks nothing');
  const armed = pick(report, 'drag', { 'contents.y': 300, 'contents.dx': 100 });
  assert.equal(prev(armed).cmd, 'arm', 'the 100 px drag is armed');
  near(armed.state.redact.rects[1].rect, [300, 300, 400, 330], 0.5, 'armed area mark');
  near(next(next(armed)).state.redact.rects[0].rect, mark.state.redact.rects[0].rect, 0.01, 'nudge undone');

  const after = await hashes();
  for (const [path, hash] of Object.entries(U4_FIXTURES)) {
    assert.equal(before[path], hash, `${path} pinned`);
    assert.equal(after[path], hash, `${path} unchanged`);
  }
});
