/**
 * P5 U3 drive (contract-p5-unit3.md §4.2 as amended by §9.27, §9.29, §9.32, §9.35, §9.37): tests/drive/sign-field.jsonl holds
 * the exact-value expects; the drawn rect, the password redaction, the fixture hashes and pyHanko on the signed outputs are
 * checked here. drive()/pick()/near()/savedBytes() are copies of tests/p6-drive4.test.mjs (not shared on purpose).
 */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn, spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
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
const py = join(root, process.platform === 'win32' ? '.venv-qa/Scripts/python.exe' : '.venv-qa/bin/python');

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

/** §9.37: the two fixtures the script opens, pinned by value; hashes before == after == pinned. */
const U3_FIXTURES = {
  'fixtures/hello.pdf': '229ca9488a139d3f32b864ba24055152fe7f0faf670c9db6491144f23d275b94',
  'fixtures/p5/empty-sigfield.pdf': '07e46851060fc7aa179c0f89a7d08ebdac26f682f4322edf20c57dad0523a206',
};
const sha = async path => createHash('sha256').update(await readFile(join(root, path))).digest('hex');
const hashes = async () => Object.fromEntries(await Promise.all(Object.keys(U3_FIXTURES).map(async path => [path, await sha(path)])));

/** Every number within ±tol (pt). */
function near(got, want, tol, label) {
  assert.ok(Array.isArray(got) && got.length === want.length && got.every((value, i) => Math.abs(value - want[i]) <= tol),
    `${label}: ${JSON.stringify(got)} vs ${JSON.stringify(want)} ±${tol}`);
}
const red = steps => steps.filter(step => step.expect !== undefined && !step.ok);
const brief = step => ({ cmd: step.cmd, to: step.to, path: step.path, signatures: step.state?.signatures, sign: step.state?.sign, fieldsLast: step.state?.fields?.last, revision: step.state?.revision, error: step.state?.error });

/** §9.32: scripts/verify-sign.py on one output; exit 0 and every wanted line present. */
function pyhanko(file, n, lines) {
  const run = spawnSync(py, ['scripts/verify-sign.py', file, '--ca', join(root, 'fixtures/p0b/.ca/ca.pem'), '--expect-signatures', String(n)], { cwd: root, encoding: 'utf8' });
  assert.equal(run.status, 0, `${file}: ${run.stdout}${run.stderr}`);
  const got = run.stdout.split('\n').map(line => line.trim());
  for (const line of lines) assert.ok(got.includes(line), `${file}: missing "${line}" in\n${run.stdout}`);
}

let outDir; // the drive run's --out, shared with the pyHanko test

test('drive P5 U3: sign on the page', { skip: CI_NO_ELECTRON, timeout: 180_000 }, async () => {
  const before = await hashes();
  const { code, report, dir, stdout } = await drive('tests/drive/sign-field.jsonl', 150_000);
  outDir = dir;
  const steps = report.steps;
  assert.equal(steps.length, 45, report.error ?? '');
  assert.equal(red(steps).length, 0, JSON.stringify(red(steps).map(brief)));
  assert.equal(code, 0);
  const prev = step => steps[steps.indexOf(step) - 1];

  // §9.35 the drawn field is the drag ±0.5 pt, and the signed reopen keeps it (±0.01).
  const drawn = pick(report, 'drag', { 'contents.dx': 220 });
  assert.equal(prev(drawn).cmd, 'arm', 'the drag is armed');
  near(drawn.state.signatures[2].rect, [72, 300, 292, 360], 0.5, 'drawn Signature4');
  const reopened = steps.find(step => step.cmd === 'open' && basename(step.path ?? '') === 'field-signed.pdf');
  assert.ok(reopened, 'field-signed.pdf is reopened');
  near(reopened.state.signatures[2].rect, drawn.state.signatures[2].rect, 0.01, 'Signature4 persisted');

  // §2.6/§9.27 secrets: the password never appears as a JSON value; every sign step is [redacted] and has no password key.
  assert.ok(!stdout.includes('"password"'), 'no password key in the report');
  assert.ok(!stdout.includes('"pdf4all"'), 'no password value in the report');
  const signs = steps.filter(step => step.cmd === 'sign');
  assert.equal(signs.length, 6);
  for (const step of signs) {
    assert.equal(step.contents, '[redacted]', `sign → ${step.to}`);
    assert.ok(!('password' in step), `sign → ${step.to} has no password key`);
  }

  // Outputs exist exactly for the successful signs; a refused sign writes nothing.
  for (const name of ['field-signed.pdf', 'field-signed2.pdf', 'field-signed3.pdf', 'hello-field.pdf']) assert.ok(await savedBytes(dir, name) > 0, name);
  for (const name of ['refused-signed.pdf', 'refused-nope.pdf']) assert.ok(!existsSync(join(dir, name)), `${name} not written`);

  const after = await hashes();
  for (const [path, hash] of Object.entries(U3_FIXTURES)) {
    assert.equal(before[path], hash, `${path} pinned`);
    assert.equal(after[path], hash, `${path} unchanged`);
  }
});

test('drive P5 U3: pyHanko on the signed outputs', { skip: CI_NO_ELECTRON || (existsSync(py) ? false : 'no .venv-qa'), timeout: 60_000 }, () => {
  assert.ok(outDir, 'the drive test ran first');
  pyhanko(join(outDir, 'field-signed2.pdf'), 2, [
    'Signature4 INTACT ENTIRE_REVISION trusted docmdp-ok',
    'Signature1 INTACT ENTIRE_FILE trusted docmdp-ok',
  ]);
  pyhanko(join(outDir, 'field-signed3.pdf'), 3, [
    'Signature4 INTACT ENTIRE_REVISION trusted docmdp-ok',
    'Signature1 INTACT ENTIRE_REVISION trusted docmdp-ok',
    'Signature3 INTACT ENTIRE_FILE trusted docmdp-ok',
  ]);
  pyhanko(join(outDir, 'hello-field.pdf'), 1, ['S1 INTACT ENTIRE_FILE trusted docmdp-ok']);
});

// Debt sweep 1 row 16b: the run's field-signed3.pdf reopens through the engine harness (as tests/p5-verify.test.mjs does)
// and Verify reads three intact signatures, one per signed field.
test('drive P5 U3: field-signed3.pdf reopens with 3 valid signatures', { skip: CI_NO_ELECTRON, timeout: 60_000 }, async () => {
  assert.ok(outDir, 'the drive test ran first');
  const { X509Certificate } = await import('node:crypto');
  const { default: mupdf } = await import('mupdf');
  const { createEngine } = await import('./rust-engine.mjs');
  const { verifySignature } = await import('../build/sign/verify.js');
  const engine = createEngine(mupdf);
  const bytes = await readFile(join(outDir, 'field-signed3.pdf'));
  const session = engine.createSession();
  const opened = engine.dispatch({ id: 1, op: 'open', name: 'field-signed3.pdf', pdf: bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) }, session).reply;
  assert.equal(opened.ok, true, opened.error);
  const listed = engine.dispatch({ id: 2, op: 'sign-list' }, session).reply;
  assert.equal(listed.ok, true, listed.error);
  const signed = listed.signatures.filter(entry => !entry.empty);
  assert.deepEqual(signed.map(entry => entry.field).sort(), ['Signature1', 'Signature3', 'Signature4']);
  assert.deepEqual(listed.signatures.filter(entry => entry.empty).map(entry => entry.field), ['Signature2']);
  assert.equal(listed.versions, 3);
  const ca = new X509Certificate(await readFile(join(root, 'fixtures/p0b/.ca/ca.pem')));
  for (const entry of signed) {
    const verdict = verifySignature(Buffer.from(listed.bytes), entry, [ca]);
    assert.equal(verdict.status, 'valid', entry.field);
    assert.equal(verdict.trust, 'trusted', entry.field);
  }
});
