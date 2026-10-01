/**
 * P6 U8 drive (contract-p6-unit8.md §3.1 as amended by §9.10, §9.13): tests/drive/direct-edit9.jsonl holds the exact-value expects;
 * here, by step index, the two Twin runs keep their own ids and rects, and the added "Move me" copy is a target of its own.
 * drive()/pick()/near() are copies of tests/p6-drive8.test.mjs (not shared on purpose).
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
const SCRIPT = 'tests/drive/direct-edit9.jsonl';
const DIRECT3 = 'fixtures/p6/direct3.pdf';
const DIRECT3_SHA = 'e3ae5a43d9fb17e5b164c00b529d5be50ba4d9178de6c11b66252cc240297466';
const ORIGINAL = 'p0.c0.314.6c51e7e2';

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

/** Every number within ±tol (pt). */
function near(got, want, tol, label) {
  assert.ok(Array.isArray(got) && got.length === want.length && got.every((value, i) => Math.abs(value - want[i]) <= tol),
    `${label}: ${JSON.stringify(got)} vs ${JSON.stringify(want)} ±${tol}`);
}
const sha = async path => createHash('sha256').update(await readFile(join(root, path))).digest('hex');
const red = steps => steps.filter(step => step.expect !== undefined && !step.ok);
const brief = step => ({ cmd: step.cmd, direct: step.state?.direct && { active: step.state.direct.active, targets: step.state.direct.targets }, revision: step.state?.revision, error: step.state?.error, announcement: step.state?.announcement });
const TWIN_A = [299.88, 396.16, 325.21, 412.65];
const TWIN_B = [299.88, 296.3, 325.21, 312.79];
const COPY = [299.88, 196.44, 349.22, 212.93];

test('drive P6 U8: duplicate text on the page stays editable per run', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  assert.equal(await sha(DIRECT3), DIRECT3_SHA, 'direct3 before');
  const text = await readFile(join(root, SCRIPT), 'utf8');
  const header = text.match(/(\d+) steps, (\d+) click lines/);
  assert.ok(header, 'script header pins the step and click counts');
  const lines = text.split('\n').filter(line => line.trim() && !line.trim().startsWith('#'));
  assert.deepEqual([Number(header[1]), Number(header[2])], [43, 12]);
  assert.equal(lines.length, 43);
  assert.equal(lines.filter(line => line.includes('"cmd":"click"')).length, 12);
  const { code, report } = await drive(SCRIPT, 150_000);
  const steps = report.steps;
  assert.equal(steps.length, 43, report.error ?? '');
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);
  const S = n => steps[n - 1];
  const active = n => S(n).state.direct.active;

  // The second add names a run with on-page bounds (null before U8).
  near(S(8).state.content.lastAdd.bounds, TWIN_B, 0.5, 'second Twin lastAdd bounds');
  // A (S9, S16) and B (S11, S14) are two targets; A is untouched by B's edit, B's id changes with its bytes.
  assert.notEqual(active(9).targetId, active(11).targetId, 'the two Twins have distinct ids');
  assert.equal(active(16).targetId, active(9).targetId, 'A keeps its id after B is edited');
  assert.notEqual(active(14).targetId, active(11).targetId, 'B is rewritten');
  near(active(9).rect, TWIN_A, 0.5, 'A rect');
  near(active(16).rect, TWIN_A, 0.5, 'A rect after B edit');
  near(active(11).rect, TWIN_B, 0.5, 'B rect');
  // "Move me" copy (added before any undo): the original keeps its id and target, the copy is its own target.
  assert.notEqual(S(20).state.content.lastAdd.targetId, ORIGINAL, 'lastAdd is the copy');
  near(S(20).state.content.lastAdd.bounds, COPY, 0.5, 'copy lastAdd bounds');
  assert.equal(active(21).targetId, ORIGINAL);
  near(active(21).rect, [72, 376.95, 129.57, 396.19], 0.5, 'original rect');
  assert.notEqual(active(23).targetId, ORIGINAL, 'the copy opens its own editor');
  near(active(23).rect, COPY, 0.5, 'copy rect');
  // §9.13: a grip move of the copy keeps the editor on the copy (refound near old + d), not on the same-text original.
  assert.notEqual(active(24).targetId, ORIGINAL, 'moved editor stays on the copy');
  near(active(24).rect, [COPY[0], COPY[1] + 20, COPY[2], COPY[3] + 20], 0.5, 'copy rect after move');
  // T8-N2: redo back to rev 12 (before B's edit) -> B is "Twin" again with its pre-edit id and rect.
  assert.equal(active(33).targetId, active(11).targetId, 'B id restored before its edit is redone');
  near(active(33).rect, TWIN_B, 0.5, 'B rect before its edit is redone');
  // Saved at rev 15 and reopened: the moved copy and the edited B keep their own targets.
  assert.notEqual(active(41).targetId, ORIGINAL, 'reopened copy is its own target');
  near(active(41).rect, [COPY[0], COPY[1] + 20, COPY[2], COPY[3] + 20], 0.5, 'reopened copy rect');
  assert.equal(active(43).targetId, active(14).targetId, 'reopened B keeps its edited id');
  near(active(43).rect, active(14).rect, 0.5, 'reopened B rect');
  assert.equal(await sha(DIRECT3), DIRECT3_SHA, 'direct3 after');
});
