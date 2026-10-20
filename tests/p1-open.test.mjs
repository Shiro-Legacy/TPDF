import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { lstat, mkdtemp as makeTempDir, open as fopen, readFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';
import { appBinary } from './app-bin.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));

/** Every temp dir this file makes (housekeeping B): removed after all its tests, never before their asserts. */
const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));
const skipApp = process.env.TANDEM_NO_APP ? 'TANDEM_NO_APP' : false;
/** The open ceiling, 120 MiB (crates/tandem-protocol limits; the open caps are Rust tests: crates/tandem-app/tests/safe_write.rs). */
const MAX_PDF_BYTES = 120 * 1024 * 1024;

async function runSmoke(args, timeoutMs = 60_000) {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-p1-'));
  const reportPath = join(dir, 'report.json');
  const child = spawn(appBinary, [`--user-data-dir=${join(dir, 'udata')}`, '--smoke', `--report=${reportPath}`, ...args], {
    cwd: root,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let stdout = '';
  let stderr = '';
  child.stdout.setEncoding('utf8');
  child.stderr.setEncoding('utf8');
  child.stdout.on('data', chunk => { stdout += chunk; });
  child.stderr.on('data', chunk => { stderr += chunk; });
  let timedOut = false;
  const timer = setTimeout(() => {
    timedOut = true;
    child.kill();
  }, timeoutMs);
  const code = await new Promise(resolve => child.on('close', resolve));
  clearTimeout(timer);
  let report = null;
  try { report = JSON.parse(await readFile(reportPath, 'utf8')); } catch { /* no report */ }
  return { code, stdout, stderr, report, timedOut };
}

test('smoke --open missing and oversized files fail without a renderer crash', { skip: skipApp, timeout: 60_000 }, async () => {
  const missing = await runSmoke([`--open=${join(root, 'fixtures/no-such.pdf')}`]);
  assert.equal(missing.timedOut, false);
  assert.equal(missing.code, 1);
  assert.equal(missing.report?.ok, false);
  assert.equal(missing.report?.openError?.reason, 'unreadable');
  assert.notEqual(missing.report?.error, 'renderer process exited');
  assert.equal(missing.stdout.includes('render-process-gone'), false);

  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-p1-sparse-'));
  const sparse = join(dir, 'huge.pdf');
  const fh = await fopen(sparse, 'w');
  await fh.truncate(MAX_PDF_BYTES + 1);
  await fh.close();
  const huge = await runSmoke([`--open=${sparse}`]);
  assert.equal(huge.timedOut, false);
  assert.equal(huge.code, 1);
  assert.equal(huge.report?.ok, false);
  assert.equal(huge.report?.openError?.reason, 'too-large');
  assert.equal(huge.report?.openError?.limit, MAX_PDF_BYTES);
  assert.notEqual(huge.report?.error, 'renderer process exited');
  assert.equal(huge.stdout.includes('render-process-gone'), false);
});

test('smoke --open hello --print-to writes a 1-page PDF', { skip: skipApp, timeout: 130_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-print-'));
  const dest = join(dir, 'hello-print.pdf');
  const { code, report, timedOut } = await runSmoke(
    [`--open=${join(root, 'fixtures/hello.pdf')}`, `--print-to=${dest}`],
    125_000,
  );
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.print?.pages, 1);
  const pdf = await readFile(dest);
  // The app prints the page itself, vector (hello.pdf's page is under 1 KB).
  assert.equal(pdf.subarray(0, 5).toString(), '%PDF-');
  assert.equal([...pdf.toString('latin1').matchAll(/\/Type\s*\/Page(?!s)/g)].length, 1);
});

test('smoke --open large --print-to --print-range=1-5 reports 5 pages', { skip: skipApp, timeout: 130_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-print-'));
  const dest = join(dir, 'large-print.pdf');
  const { code, report, timedOut } = await runSmoke(
    [`--open=${join(root, 'fixtures/large.pdf')}`, `--print-to=${dest}`, '--print-range=1-5'],
    125_000,
  );
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.open?.pageCount, 500);
  assert.equal(report?.error, undefined);
  assert.equal(report?.print?.pages, 5);
});

test('second smoke against the same userData exits 3 and does not touch the journal', { skip: skipApp, timeout: 130_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-single-'));
  const udata = join(dir, 'udata');
  const report1 = join(dir, 'r1.json');
  const report2 = join(dir, 'r2.json');
  // The first instance is a live drive (`--drive=-`) that holds the lock until its stdin closes below.
  const first = spawn(appBinary, [
    `--user-data-dir=${udata}`, '--smoke', `--report=${report1}`,
    `--open=${join(root, 'fixtures/large.pdf')}`, '--drive=-',
  ], { cwd: root, stdio: ['pipe', 'pipe', 'pipe'] });
  try {
    const start = Date.now();
    for (;;) {
      // The single-instance lock file (crates/tandem-app/src/host/mod.rs SingletonLock), on every OS.
      try { await lstat(join(udata, 'SingletonLock')); break; }
      catch {
        if (Date.now() - start > 20_000) throw new Error('first instance did not take SingletonLock');
        await new Promise(r => setTimeout(r, 50));
      }
    }
    const journalPath = join(udata, 'pending-saves.json');
    const journalWait = Date.now();
    let journalBefore = null;
    while (Date.now() - journalWait < 5_000) {
      // A 0-byte read is the file mid-write: keep polling until the journal has content.
      try { journalBefore = await readFile(journalPath); if (journalBefore.length) break; }
      catch { await new Promise(r => setTimeout(r, 50)); }
    }
    const second = spawn(appBinary, [
      `--user-data-dir=${udata}`, '--smoke', `--report=${report2}`,
      `--open=${join(root, 'fixtures/hello.pdf')}`,
    ], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
    const code2 = await new Promise(resolve => second.on('close', resolve));
    assert.equal(code2, 3);
    await assert.rejects(stat(report2), { code: 'ENOENT' });
    let journalAfter = null;
    try { journalAfter = await readFile(join(udata, 'pending-saves.json')); } catch { /* absent */ }
    // The first instance's drive saves nothing, so the journal is as it was: the second instance touched nothing.
    assert.deepEqual(journalAfter, journalBefore);
    first.stdin.end();
    const code1 = await new Promise(resolve => first.on('close', resolve));
    assert.equal(code1, 0);
    assert.equal(JSON.parse(await readFile(report1, 'utf8')).ok, true);
  } finally {
    first.kill();
  }
});
