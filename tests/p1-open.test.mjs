import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { lstat, mkdir, mkdtemp as makeTempDir, open as fopen, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { createRequire } from 'node:module';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';
import { CSP, MAX_PDF_BYTES, localAsset, openToken, securityPolicy } from '../build/policy.js';
import { collectCapped, inspectRegularFile } from '../build/safe-write.js';
import { ALT_APP, appBinary, appEntry } from './app-bin.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));

/** Every temp dir this file makes (housekeeping B): removed after all its tests, never before their asserts. */
const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));
const electron = appBinary;
const skipElectron = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;

test('preload path must sit inside the app bundle', () => {
  assert.ok(securityPolicy('/app', '/app/build/preload.cjs'));
  assert.equal(securityPolicy('/app', '/tmp/preload.cjs'), undefined);
  assert.equal(securityPolicy('/app', '/app/../etc/preload.cjs'), undefined);
});

test('CSP img-src is self and data only; no other directive changed', () => {
  const img = /(?:^|; )img-src ([^;]*)(?:;|$)/.exec(`; ${CSP}`)?.[1];
  assert.equal(img, "'self' data:");
  assert.equal(
    CSP,
    "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; worker-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
  );
});

test('open stream caps at issued size and rejects truncated or non-file sources', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-p1-stream-'));
  const file = join(dir, 'doc.pdf');
  const payload = Buffer.alloc(100, 7);
  await writeFile(file, payload);
  const capped = await collectCapped(file, 40);
  assert.equal(capped.byteLength, 40);
  assert.deepEqual(Buffer.from(capped), payload.subarray(0, 40));
  const exact = await collectCapped(file, 100);
  assert.deepEqual(Buffer.from(exact), payload);
  await assert.rejects(collectCapped(file, 150), { code: 'EIO' });
  const nested = join(dir, 'subdir');
  await mkdir(nested);
  assert.equal(await inspectRegularFile(nested), undefined);
  assert.equal(await inspectRegularFile(join(dir, 'missing.pdf')), undefined);
  const meta = await inspectRegularFile(file);
  assert.equal(meta?.size, 100);
});

test('openToken accepts a UUID path and rejects the rest', () => {
  const token = '01234567-89ab-cdef-0123-456789abcdef';
  assert.equal(openToken(`/open/${token}`), token);
  assert.equal(openToken('/open/'), undefined);
  assert.equal(openToken('/open/../index.html'), undefined);
  assert.equal(openToken(`/open/${token.toUpperCase()}`), undefined);
  assert.equal(openToken('/open/01234567-89ab-cdef-0123-456789abcde'), undefined);
  assert.equal(localAsset(`app://pdf4all/open/${token}`), undefined);
});

async function runSmoke(args, timeoutMs = 60_000) {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-p1-'));
  const reportPath = join(dir, 'report.json');
  const child = spawn(electron, [`--user-data-dir=${join(dir, 'udata')}`, ...appEntry, '--smoke', `--report=${reportPath}`, ...args], {
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

test('smoke --open missing and oversized files fail without a renderer crash', { skip: skipElectron, timeout: 60_000 }, async () => {
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

test('smoke --open hello --print-to writes a 1-page PDF', { skip: skipElectron, timeout: 130_000 }, async () => {
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
  // Chromium's printToPDF output; a native binary prints the page itself, vector (hello.pdf's page is under 1 KB).
  if (!ALT_APP) assert.ok(pdf.byteLength >= 1024);
  assert.equal(pdf.subarray(0, 5).toString(), '%PDF-');
  assert.equal([...pdf.toString('latin1').matchAll(/\/Type\s*\/Page(?!s)/g)].length, 1);
});

test('smoke --open large --print-to --print-range=1-5 reports 5 pages', { skip: skipElectron, timeout: 130_000 }, async () => {
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

test('second smoke against the same userData exits 3 and does not touch the journal', { skip: skipElectron, timeout: 130_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-single-'));
  const udata = join(dir, 'udata');
  const report1 = join(dir, 'r1.json');
  const report2 = join(dir, 'r2.json');
  const printed = join(dir, 'out.pdf');
  // Electron's 20-page print keeps the first instance (and its lock) alive while the second starts. A native
  // binary finishes it in under 0.1 s, so there the first instance is a live drive (`--drive=-`) that holds the
  // lock until its stdin closes below.
  const first = spawn(electron, [
    `--user-data-dir=${udata}`, ...appEntry, '--smoke', `--report=${report1}`,
    `--open=${join(root, 'fixtures/large.pdf')}`,
    ...(ALT_APP ? ['--drive=-'] : [`--print-to=${printed}`, '--print-range=1-20']),
  ], { cwd: root, stdio: [ALT_APP ? 'pipe' : 'ignore', 'pipe', 'pipe'] });
  try {
    const start = Date.now();
    for (;;) {
      // Chromium's single-instance lock: a `SingletonLock` symlink on POSIX, a `lockfile` on Windows.
      try { await lstat(join(udata, process.platform === 'win32' ? 'lockfile' : 'SingletonLock')); break; }
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
    const second = spawn(electron, [
      `--user-data-dir=${udata}`, ...appEntry, '--smoke', `--report=${report2}`,
      `--open=${join(root, 'fixtures/hello.pdf')}`,
    ], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
    const code2 = await new Promise(resolve => second.on('close', resolve));
    assert.equal(code2, 3);
    await assert.rejects(stat(report2), { code: 'ENOENT' });
    let journalAfter = null;
    try { journalAfter = await readFile(join(udata, 'pending-saves.json')); } catch { /* absent */ }
    // Electron's 20-page print outlasts the second instance; a native binary's finishes in about 0.1 s, so its own
    // journal clear can land before this read (the second instance exiting 3 above is what touches nothing).
    if (!ALT_APP) assert.deepEqual(journalAfter, journalBefore);
    if (ALT_APP) first.stdin.end();
    const code1 = await new Promise(resolve => first.on('close', resolve));
    assert.equal(code1, 0);
    assert.equal(JSON.parse(await readFile(report1, 'utf8')).ok, true);
  } finally {
    first.kill();
  }
});
