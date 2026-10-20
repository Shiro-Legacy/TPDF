/**
 * Debt sweep 1 (.quad/shared/debt/scope-debt-1.md rows 1, 2, 13-15, 19, 24): tests/drive/debt1.jsonl holds the exact-value expects and the
 * geometry the sig-field drag produced is checked here through pick(); tests/drive/refusals.jsonl (A2 refusal regression) and
 * tests/drive/nonascii-label.jsonl (P3 U7) are the salvaged drives; fixtures/p3/field-defs.pdf is the --field-defs self-check input.
 * drive()/pick()/near() are copies of tests/p6-drive4.test.mjs (not shared on purpose).
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn, spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
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
const py = join(root, process.platform === 'win32' ? '.venv-qa/Scripts/python.exe' : '.venv-qa/bin/python');
const NO_PY = existsSync(py) ? false : 'no .venv-qa';

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
const red = steps => steps.filter(step => step.expect !== undefined && !step.ok);
const brief = step => ({ cmd: step.cmd, path: step.path, contents: step.contents, announcement: step.state?.announcement, error: step.state?.error, revision: step.state?.revision, dirty: step.state?.dirty, links: step.state?.links, signatures: step.state?.signatures, sign: step.state?.sign, fieldsLast: step.state?.fields?.last, content: step.state?.content && { edited: step.state.content.edited, refused: step.state.content.refused }, nav: step.state?.nav?.last, annotLast: step.state?.annotLast, decorate: step.state?.decorate, assemble: step.state?.assemble });

/** Fixtures the three scripts open, pinned by value; hashes before == after == pinned. */
const FIXTURES = {
  'fixtures/hello.pdf': '229ca9488a139d3f32b864ba24055152fe7f0faf670c9db6491144f23d275b94',
  'fixtures/p5/empty-sigfield.pdf': '07e46851060fc7aa179c0f89a7d08ebdac26f682f4322edf20c57dad0523a206',
  'fixtures/p6/direct3.pdf': 'e3ae5a43d9fb17e5b164c00b529d5be50ba4d9178de6c11b66252cc240297466',
  'fixtures/p6/links.pdf': '0ac413b0fa9b42062542afbc970f0245bceacab0a789017c90a413733962d0bc',
  'fixtures/p3/field-defs.pdf': '43782ca55aa415cc9fc09406d95e9300326ad281fac5a0176b14c88a295f4250',
};
const sha = async path => createHash('sha256').update(await readFile(join(root, path))).digest('hex');
const hashes = async () => Object.fromEntries(await Promise.all(Object.keys(FIXTURES).map(async path => [path, await sha(path)])));
async function unchanged(before) {
  const after = await hashes();
  for (const [path, hash] of Object.entries(FIXTURES)) {
    assert.equal(before[path], hash, `${path} pinned`);
    assert.equal(after[path], hash, `${path} unchanged`);
  }
}

/** One steady result per script: exit 0, no red expect, the pinned step count. */
async function green(script, count) {
  const run = await drive(script, 150_000);
  assert.equal(run.report.steps.length, count, run.report.error ?? '');
  assert.equal(red(run.report.steps).length, 0, JSON.stringify(red(run.report.steps).map(brief)));
  assert.equal(run.code, 0);
  return run;
}

function fieldDefs(file, spec) {
  const run = spawnSync(py, ['scripts/verify-roundtrip.py', file, '--field-defs', JSON.stringify(spec)], { cwd: root, encoding: 'utf8' });
  assert.equal(run.status, 0, `${file}: ${run.stdout}${run.stderr}`);
  return JSON.parse(run.stdout);
}

test('drive debt sweep 1: announcements, link set by page, sig-field body drag, stale error, content reset', { skip: NO_APP, timeout: 180_000 }, async () => {
  const before = await hashes();
  const { report } = await green('tests/drive/debt1.jsonl', 22);
  const steps = report.steps;
  const prev = step => steps[steps.indexOf(step) - 1];
  // Row 13: the body drag moves Signature1 by dx 20 pt (±0.5), one op; the click that follows is at the moved centre.
  const moved = pick(report, 'drag', { 'contents.dx': 20 });
  near(prev(moved).state.signatures[0].rect, [72, 620, 340, 680], 0.01, 'Signature1 before the drag');
  near(moved.state.signatures[0].rect, [92, 620, 360, 680], 0.5, 'Signature1 moved by the body drag');
  // Row 14: the armed draw added Signature4 at the drag (±0.5).
  const drawn = pick(report, 'drag', { 'contents.dx': 220 });
  assert.equal(prev(drawn).cmd, 'arm', 'the draw is armed');
  near(drawn.state.signatures.find(item => item.field === 'Signature4')?.rect, [72, 300, 292, 360], 0.5, 'drawn Signature4');
  await unchanged(before);
});

test('drive A2 refusals: every refused command leaves the document clean', { skip: NO_APP, timeout: 180_000 }, async () => {
  const before = await hashes();
  await green('tests/drive/refusals.jsonl', 29);
  await unchanged(before);
});

test('drive P3 U7: a non-ASCII /TU label round-trips through the app', { skip: NO_APP, timeout: 180_000 }, async () => {
  const before = await hashes();
  const { dir } = await green('tests/drive/nonascii-label.jsonl', 4);
  if (!NO_PY) fieldDefs(join(dir, 'nonascii.pdf'), { n1: { ft: '/Tx', tu: 'Nom (é)', ff: 2, dv: 'Année', rect: [72, 468, 252, 492], page: 0 } });
  await unchanged(before);
});

test('fixtures/p3/field-defs.pdf: provenance matches the bytes and the generator', async () => {
  const provenance = JSON.parse(await readFile(join(root, 'fixtures/p3/field-defs.provenance.json'), 'utf8'));
  const bytes = await readFile(join(root, 'fixtures/p3/field-defs.pdf'));
  assert.equal(provenance.file, 'fixtures/p3/field-defs.pdf');
  assert.equal(provenance.bytes, bytes.length);
  assert.equal(provenance.sha256, createHash('sha256').update(bytes).digest('hex'));
  assert.equal(provenance.sha256, FIXTURES['fixtures/p3/field-defs.pdf']);
  assert.equal(provenance.generator.sha256, await sha(provenance.generator.path));
});

test('verify-roundtrip.py --field-defs reads tu/ff/dv on fixtures/p3/field-defs.pdf', { skip: NO_PY }, () => {
  const verdict = fieldDefs('fixtures/p3/field-defs.pdf', {
    nom: { ft: '/Tx', tu: 'Nom (é)', ff: 2, dv: 'défaut', rect: [72, 650, 252, 680], page: 0 },
    ok: { ft: '/Btn', dv: 'Yes' },
    color: { ft: '/Btn', radio: true, ff: 32768, dv: 'B', kids: 2, ap: ['A', 'B', 'Off'] },
    plain: { ft: '/Tx', tu: null, ff: null, dv: null },
  });
  assert.equal(verdict.ok, true);
  // The checker bites: a wrong /TU is reported, not ignored.
  const wrong = spawnSync(py, ['scripts/verify-roundtrip.py', 'fixtures/p3/field-defs.pdf', '--field-defs', JSON.stringify({ nom: { ft: '/Tx', tu: 'Nom (e)' } })], { cwd: root, encoding: 'utf8' });
  assert.equal(wrong.status, 1, wrong.stdout);
});
