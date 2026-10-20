/**
 * Housekeeping B (contract-housekeeping-b.md T3): a drive run leaves no `pdf4all-*` dir in the
 * temp directory, and no test lets openssl rewrite the project's CA serial file
 * (fixtures/p0b/.ca/ca.srl). Helpers are local on purpose (not shared with other test files).
 */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const root = fileURLToPath(new URL('..', import.meta.url));
const NO_APP = process.env.TANDEM_NO_APP ? 'TANDEM_NO_APP' : false;

test('drive.mjs removes its pdf4all-drive-* userData dir after the run', { skip: NO_APP, timeout: 120_000 }, async t => {
  // A private TMPDIR: parallel test files cannot add or remove entries in it.
  const tmp = await mkdtemp(join(tmpdir(), 'tmp-cleanup-'));
  t.after(() => rm(tmp, { recursive: true, force: true }));
  const script = join(tmp, 'open.jsonl');
  await writeFile(script, `${JSON.stringify({ cmd: 'open', path: join(root, 'fixtures/hello.pdf'), expect: { revision: 0 } })}\n`);
  const child = spawn(process.execPath, [join(root, 'scripts/drive.mjs'), script], { cwd: root, env: { ...process.env, TMPDIR: tmp }, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '';
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', chunk => { stdout += chunk; });
  child.stderr.resume();
  const timer = setTimeout(() => child.kill(), 90_000);
  const code = await new Promise(resolve => child.on('close', resolve));
  clearTimeout(timer);
  assert.equal(code, 0, stdout);
  assert.equal(JSON.parse(stdout).steps.length, 1);
  assert.deepEqual((await readdir(tmp)).filter(name => name.startsWith('pdf4all-')), []);
});

test('no test runs openssl with -CAcreateserial unless -CAserial keeps the serial out of fixtures/', async () => {
  const dir = join(root, 'tests');
  const offenders = [];
  for (const name of (await readdir(dir)).filter(n => n.endsWith('.test.mjs'))) {
    const text = await readFile(join(dir, name), 'utf8');
    if (text.includes("'-CAcreateserial'") && !text.includes("'-CAserial'")) offenders.push(name);
  }
  assert.deepEqual(offenders, []);
});
