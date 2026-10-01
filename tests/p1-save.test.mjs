import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { chmod, copyFile, link, lstat, mkdir, mkdtemp as makeTempDir, readFile, readdir, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { createRequire } from 'node:module';
import { basename, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';
import { MAX_PDF_BYTES, MAX_SAVE_BYTES, pdfWithinCeiling, saveWithinCeiling } from '../build/policy.js';
import {
  classifyFsError, cleanupOrphans, copyBasename, inspectRegularFile, saveCopy, saveInPlace, writeNew,
} from '../build/safe-write.js';
import { makeThreePagePdf } from '../fixtures/textpages.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));

/** Every temp dir this file makes (housekeeping B): removed after all its tests, never before their asserts. */
const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(() => Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))));
const electron = createRequire(import.meta.url)('electron');
/** Windows has no POSIX permission bits (stat reports 0666 or 0444) and needs Developer Mode or elevation for a symlink. */
const WIN = process.platform === 'win32';
const python = join(root, WIN ? '.venv-qa/Scripts/python.exe' : '.venv-qa/bin/python3');
async function trySymlink(target, path) {
  try { await symlink(target, path); return true; }
  catch (err) { if (WIN && err.code === 'EPERM') return false; throw err; }
}
const skipElectron = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;
const rendererSrc = readFileSync(join(root, 'src/renderer/renderer.ts'), 'utf8');
const skipP2Edit = skipElectron || !rendererSrc.includes('P2_EDIT');
const skipP2Assemble = skipP2Edit || !rendererSrc.includes('assemble:');
const skipP2AssembleUndo = skipP2Assemble || !rendererSrc.includes('"insert pages"');
const skipP2Export = skipP2Edit || !rendererSrc.includes('P2_NEW');
const skipP2Decorate = skipP2Edit || !rendererSrc.includes('decorate:');
const skipP2Shapes = skipP2Edit || !rendererSrc.includes('op === "shapes"');
const skipP2Encrypt = skipP2Edit || !rendererSrc.includes('smokeEncrypt');
const bytes = Buffer.from('%PDF-n');

function fsError(code) {
  return Object.assign(new Error(code), { code });
}

async function scratch() {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-save-'));
  const sourcePath = join(dir, 'src.pdf');
  await writeFile(sourcePath, bytes);
  const source = await inspectRegularFile(sourcePath);
  assert.ok(source);
  return { dir, sourcePath, source, dest: join(dir, 'out.pdf') };
}

async function tmpCount(dir) {
  return (await readdir(dir)).filter(name => name.startsWith('.') && name.endsWith('.tmp')).length;
}

test('copyBasename appends -copy before .pdf', () => {
  assert.equal(copyBasename('hello.pdf'), 'hello-copy.pdf');
  assert.equal(copyBasename('/tmp/Hello.PDF'), 'Hello-copy.pdf');
});

test('classifyFsError maps ENOSPC/EACCES and the rest to write-failed', () => {
  assert.equal(classifyFsError(fsError('ENOSPC')), 'disk-full');
  assert.equal(classifyFsError(fsError('EACCES')), 'permission');
  assert.equal(classifyFsError(fsError('EIO')), 'write-failed');
});

test('success replaces a preexisting copy and leaves the source', async () => {
  const { dir, sourcePath, source, dest } = await scratch();
  await writeFile(dest, Buffer.from('old-copy-bytes'));
  const before = await inspectRegularFile(sourcePath);
  const result = await saveCopy({ dest, bytes, source });
  assert.equal(result.ok, true);
  if (result.ok) assert.equal(result.name, 'out.pdf');
  assert.equal(result.ok && result.warning, undefined);
  assert.deepEqual(await readFile(dest), bytes);
  assert.deepEqual(await readFile(sourcePath), bytes);
  const after = await inspectRegularFile(sourcePath);
  assert.equal(after?.mtimeMs, before?.mtimeMs);
  assert.equal(after?.ino, before?.ino);
  const mask = process.umask(0o022);
  process.umask(mask);
  assert.equal((await stat(dest)).mode & 0o777, 0o666 & ~mask);
  assert.equal(await tmpCount(dir), 0);
});

test('cancel writes nothing', async () => {
  const { dir, source, dest } = await scratch();
  const result = await saveCopy({ dest: undefined, bytes, source });
  assert.deepEqual(result, { ok: false, reason: 'cancelled' });
  await assert.rejects(stat(dest), { code: 'ENOENT' });
  assert.equal(await tmpCount(dir), 0);
});

test('inject failures clean temp and leave dest/source', async () => {
  for (const mode of ['copy', 'in-place']) {
    for (const [label, inject, reason, destPreexists] of [
      ['ENOSPC', { writeAll: async () => { throw fsError('ENOSPC'); } }, 'disk-full', true],
      ['EACCES', { writeAll: async () => { throw fsError('EACCES'); } }, 'permission', false],
      ['reopen hash mismatch', { reopen: async () => Buffer.alloc(bytes.length, 7) }, 'write-failed', true],
      ['commit rename failure', { rename: async () => { throw fsError('EIO'); } }, 'write-failed', true],
    ]) {
      const tag = `${mode} ${label}`;
      const { dir, sourcePath, source, dest } = await scratch();
      if (mode === 'copy' && destPreexists) await writeFile(dest, Buffer.from('keep-me-please'));
      const result = mode === 'in-place'
        ? await saveInPlace({ bytes, source, inject })
        : await saveCopy({ dest, bytes, source, inject });
      assert.deepEqual(result, { ok: false, reason }, tag);
      if (mode === 'copy') {
        if (destPreexists) assert.equal(await readFile(dest, 'utf8'), 'keep-me-please', tag);
        else await assert.rejects(stat(dest), { code: 'ENOENT' });
      }
      assert.deepEqual(await readFile(sourcePath), bytes, tag);
      assert.equal(await tmpCount(dir), 0, tag);
    }
  }
});

test('directory fsync failure is success with durability-uncertain', async () => {
  const { dir, source, dest } = await scratch();
  const result = await saveCopy({
    dest, bytes, source,
    inject: { dirSync: async () => { throw fsError('EIO'); } },
  });
  assert.deepEqual(result, { ok: true, name: 'out.pdf', warning: 'durability-uncertain' });
  assert.deepEqual(await readFile(dest), bytes);
  assert.equal(await tmpCount(dir), 0);
});

test('source change still writes the copy with a warning', async () => {
  const { sourcePath, source, dest } = await scratch();
  await writeFile(sourcePath, Buffer.concat([bytes, Buffer.from('x')]));
  const result = await saveCopy({ dest, bytes, source });
  assert.deepEqual(result, { ok: true, name: 'out.pdf', warning: 'source-changed' });
  assert.deepEqual(await readFile(dest), bytes);
  assert.deepEqual(await readFile(sourcePath), Buffer.concat([bytes, Buffer.from('x')]));
});

test('same path, hardlink, and dest symlink are same-file', async () => {
  const { dir, sourcePath, source } = await scratch();
  assert.deepEqual(await saveCopy({ dest: sourcePath, bytes, source }), { ok: false, reason: 'same-file' });

  const hard = join(dir, 'hard.pdf');
  await link(sourcePath, hard);
  assert.deepEqual(await saveCopy({ dest: hard, bytes, source }), { ok: false, reason: 'same-file' });
  assert.deepEqual(await readFile(hard), bytes);

  const other = join(dir, 'other.pdf');
  await writeFile(other, Buffer.from('not-the-source'));
  const alias = join(dir, 'alias.pdf');
  if (!await trySymlink(other, alias)) return;
  assert.deepEqual(await saveCopy({ dest: alias, bytes, source }), { ok: false, reason: 'same-file' });
  assert.equal(await readFile(other, 'utf8'), 'not-the-source');

  const selfLink = join(dir, 'self.pdf');
  await symlink(sourcePath, selfLink);
  assert.deepEqual(await saveCopy({ dest: selfLink, bytes, source }), { ok: false, reason: 'same-file' });
  assert.deepEqual(await readFile(sourcePath), bytes);
});

test('Open B during write aborts before rename and leaves dest', async () => {
  const { dir, source, dest } = await scratch();
  await writeFile(dest, Buffer.from('B-source-bytes'));
  let current = true;
  const result = await saveCopy({
    dest, bytes, source,
    isCurrent: () => current,
    inject: {
      writeAll: async (fh, data) => {
        await fh.writeFile(data);
        current = false;
      },
    },
  });
  assert.deepEqual(result, { ok: false, reason: 'document-changed' });
  assert.equal(await readFile(dest, 'utf8'), 'B-source-bytes');
  assert.equal(await tmpCount(dir), 0);
});

test('save ceiling is 50 MiB; open stays 120 MiB', () => {
  assert.equal(MAX_SAVE_BYTES, 50 * 1024 * 1024);
  assert.equal(MAX_PDF_BYTES, 120 * 1024 * 1024);
  assert.equal(saveWithinCeiling(MAX_SAVE_BYTES), true);
  assert.equal(saveWithinCeiling(MAX_SAVE_BYTES + 1), false);
  assert.equal(pdfWithinCeiling(MAX_PDF_BYTES), true);
  assert.equal(pdfWithinCeiling(MAX_PDF_BYTES + 1), false);
});

test('relative dest is unreadable; too-large uses the save ceiling', async () => {
  const { source } = await scratch();
  assert.deepEqual(await saveCopy({ dest: 'out.pdf', bytes, source }), { ok: false, reason: 'unreadable' });
  const huge = new Uint8Array(MAX_SAVE_BYTES + 1);
  assert.deepEqual(await saveCopy({ dest: join(tmpdir(), 'x.pdf'), bytes: huge, source }), { ok: false, reason: 'too-large' });
});

test('writeNew refuses a relative dest and oversize, writes 0644-ish on success', async () => {
  assert.deepEqual(await writeNew({ dest: 'out.pdf', bytes }), { ok: false, reason: 'unreadable' });
  assert.deepEqual(await writeNew({ dest: join(tmpdir(), 'x.pdf'), bytes: new Uint8Array(MAX_SAVE_BYTES + 1) }), { ok: false, reason: 'too-large' });
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-write-new-'));
  const dest = join(dir, 'new.pdf');
  const result = await writeNew({ dest, bytes });
  assert.equal(result.ok, true);
  assert.deepEqual(await readFile(dest), bytes);
  const mask = process.umask(0o022);
  process.umask(mask);
  assert.equal((await stat(dest)).mode & 0o777, 0o666 & ~mask);
  const failed = await writeNew({
    dest: join(dir, 'fail.pdf'), bytes,
    inject: { writeAll: async () => { throw fsError('ENOSPC'); } },
  });
  assert.deepEqual(failed, { ok: false, reason: 'disk-full' });
  assert.equal(await tmpCount(dir), 0);
});

function verifyTextNote(pdf) {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-roundtrip.py'), pdf, '--text-notes', '1', '--contents', 'PDF4All P2 note',
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  const verdict = JSON.parse(checked.stdout);
  assert.equal(verdict.ok, true);
  assert.equal(verdict.count, 1);
  assert.equal(verdict.no_author, true);
}

async function runSmoke(args, timeoutMs = 60_000) {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-p1-save-'));
  const reportPath = join(dir, 'report.json');
  const child = spawn(electron, [`--user-data-dir=${join(dir, 'udata')}`, '.', '--smoke', `--report=${reportPath}`, ...args], {
    cwd: root,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let stdout = '';
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', chunk => { stdout += chunk; });
  let timedOut = false;
  const timer = setTimeout(() => { timedOut = true; child.kill(); }, timeoutMs);
  const code = await new Promise(resolve => child.on('close', resolve));
  clearTimeout(timer);
  let report = null;
  try { report = JSON.parse(await readFile(reportPath, 'utf8')); } catch { /* no report */ }
  return { code, stdout, report, timedOut, dir };
}

test('journal entry is present before rename; success leaves empty 0600 journal', async () => {
  const { dir, source, dest } = await scratch();
  const journalPath = join(dir, 'pending-saves.json');
  let during;
  const failed = await saveCopy({
    dest, bytes, source, journalPath,
    inject: {
      rename: async () => {
        during = JSON.parse(await readFile(journalPath, 'utf8'));
        throw fsError('EIO');
      },
    },
  });
  assert.deepEqual(failed, { ok: false, reason: 'write-failed' });
  assert.equal(during.length, 1);
  assert.equal(basename(during[0].dest), 'out.pdf');
  assert.equal(dirname(during[0].temp), dirname(during[0].dest));
  assert.match(basename(during[0].temp), /^\.[^/]+\.[A-Za-z0-9_-]+\.tmp$/);
  assert.equal(typeof during[0].started, 'number');
  assert.equal(await tmpCount(dir), 0);
  assert.deepEqual(JSON.parse(await readFile(journalPath, 'utf8')), []);

  const result = await saveCopy({ dest, bytes, source, journalPath });
  assert.equal(result.ok, true);
  assert.deepEqual(JSON.parse(await readFile(journalPath, 'utf8')), []);
  if (!WIN) assert.equal((await stat(journalPath)).mode & 0o777, 0o600);
  assert.equal(await tmpCount(dir), 0);
});

test('cleanupOrphans unlinks a matching temp, ignores the rest, treats corrupt as empty', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-orphan-'));
  const dest = join(dir, 'out.pdf');
  const journalPath = join(dir, 'pending-saves.json');
  const orphan = join(dir, '.out.pdf.abc123.tmp');
  await writeFile(orphan, 'leftover');
  const target = join(dir, 'real.pdf');
  await writeFile(target, 'keep-link-target');
  const linked = join(dir, '.out.pdf.symlink.tmp');
  const symlinked = await trySymlink(target, linked);
  const nested = join(dir, '.out.pdf.dir.tmp');
  await mkdir(nested);
  const other = join(dir, 'not-a-temp.pdf');
  await writeFile(other, 'keep-other');
  const outsideDir = await mkdtemp(join(tmpdir(), 'pdf4all-orphan-out-'));
  const outside = join(outsideDir, '.out.pdf.abc456.tmp');
  await writeFile(outside, 'keep-outside');
  const missing = join(dir, '.gone.pdf.abc.tmp');
  await writeFile(journalPath, JSON.stringify([
    { temp: orphan, dest, started: 1 },
    ...(symlinked ? [{ temp: linked, dest, started: 2 }] : []),
    { temp: nested, dest, started: 3 },
    { temp: other, dest, started: 4 },
    { temp: outside, dest, started: 5 },
    { temp: missing, dest, started: 6 },
  ]));

  await cleanupOrphans(journalPath);

  await assert.rejects(stat(orphan), { code: 'ENOENT' });
  assert.equal(await readFile(target, 'utf8'), 'keep-link-target');
  if (symlinked) assert.equal((await lstat(linked)).isSymbolicLink(), true);
  assert.equal((await stat(nested)).isDirectory(), true);
  assert.equal(await readFile(other, 'utf8'), 'keep-other');
  assert.equal(await readFile(outside, 'utf8'), 'keep-outside');
  const remain = JSON.parse(await readFile(journalPath, 'utf8')).map(entry => entry.temp).sort();
  assert.deepEqual(remain, [...(symlinked ? [linked] : []), nested, other, outside].sort());

  await writeFile(journalPath, '{not json');
  await cleanupOrphans(journalPath);
  assert.deepEqual(JSON.parse(await readFile(journalPath, 'utf8')), []);
});

test('saveInPlace rewrites the source, preserves 0644, journal empty', async () => {
  const { dir, sourcePath } = await scratch();
  await chmod(sourcePath, 0o644);
  const id = await inspectRegularFile(sourcePath);
  assert.ok(id);
  const journalPath = join(dir, 'pending-saves.json');
  const payload = Buffer.from('%PDF-in-place');
  const result = await saveInPlace({ bytes: payload, source: id, journalPath });
  assert.equal(result.ok, true);
  if (result.ok) {
    assert.equal(result.name, 'src.pdf');
    assert.equal(result.file?.size, payload.byteLength);
  }
  assert.deepEqual(await readFile(sourcePath), payload);
  if (!WIN) assert.equal((await stat(sourcePath)).mode & 0o777, 0o644);
  assert.deepEqual(JSON.parse(await readFile(journalPath, 'utf8')), []);
  assert.equal(await tmpCount(dir), 0);
});

test('saveInPlace source-changed refuses before rename', async () => {
  const { dir, sourcePath, source } = await scratch();
  const changed = Buffer.concat([bytes, Buffer.from('x')]);
  await writeFile(sourcePath, changed);
  const result = await saveInPlace({ bytes, source });
  assert.deepEqual(result, { ok: false, reason: 'source-changed' });
  assert.deepEqual(await readFile(sourcePath), changed);
  assert.equal(await tmpCount(dir), 0);
});

test('smoke --open hello --save-to writes a copy and leaves the source', { skip: skipElectron, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/hello.pdf');
  const before = await stat(fixture);
  const original = await readFile(fixture);
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-save-to-'));
  const dest = join(dir, 'hello-copy.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, `--save-to=${dest}`]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.save?.name, 'hello-copy.pdf');
  assert.equal(report?.save?.validated, true);
  assert.equal(report?.sourceUnchanged, true);
  const written = await readFile(dest);
  assert.equal(written.byteLength, report?.save?.bytes);
  assert.ok(written.byteLength >= 5);
  const after = await stat(fixture);
  assert.equal(after.mtimeMs, before.mtimeMs);
  assert.deepEqual(await readFile(fixture), original);
});

test('smoke --edit=note --save-to writes one Text note and no /T', { skip: skipP2Edit, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/hello.pdf');
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-save-'));
  const dest = join(dir, 'hello-note.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=note', `--save-to=${dest}`]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.notes, 1);
  assert.equal(report?.save?.validated, true);
  verifyTextNote(dest);
});

test('smoke --edit=note --save-in-place rewrites a scratch copy', { skip: skipP2Edit, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-in-place-'));
  const scratchPath = join(dir, 'hello.pdf');
  await copyFile(join(root, 'fixtures/hello.pdf'), scratchPath);
  await chmod(scratchPath, 0o644);
  const before = await stat(scratchPath);
  const beforeBytes = await readFile(scratchPath);
  const { code, report, timedOut } = await runSmoke([`--open=${scratchPath}`, '--edit=note', '--save-in-place']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.save?.validated, true);
  const after = await stat(scratchPath);
  const afterBytes = await readFile(scratchPath);
  assert.notDeepEqual(afterBytes, beforeBytes);
  assert.equal(after.mode & 0o777, before.mode & 0o777);
  verifyTextNote(scratchPath);
});

test('smoke --edit=note then undo reports notes 0', { skip: skipP2Edit, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/hello.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=note']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.undo?.notes, 0);
});

function verifyAnnots(pdf, map, contents, type) {
  const args = [join(root, 'scripts/verify-roundtrip.py'), pdf, '--annots', JSON.stringify(map)];
  if (contents) args.push('--contents', contents);
  if (type) args.push('--appearance', type);
  const checked = spawnSync(python, args, { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  const verdict = JSON.parse(checked.stdout);
  assert.equal(verdict.ok, true);
  return verdict;
}

test('smoke --edit=highlight --save-to writes one Highlight', { skip: skipP2Edit, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/hello.pdf');
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-hl-'));
  const dest = join(dir, 'hello-highlight.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=highlight', `--save-to=${dest}`]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.annots?.Highlight, 1);
  assert.equal(report?.save?.validated, true);
  const verdict = verifyAnnots(dest, { Highlight: 1 }, undefined, 'Highlight');
  assert.ok(verdict.appearance.inside_frac > 0.3);
});

test('smoke --edit=freetext --save-in-place rewrites a scratch copy', { skip: skipP2Edit, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-ft-'));
  const scratchPath = join(dir, 'hello.pdf');
  await copyFile(join(root, 'fixtures/hello.pdf'), scratchPath);
  await chmod(scratchPath, 0o644);
  const before = await stat(scratchPath);
  const beforeBytes = await readFile(scratchPath);
  const { code, report, timedOut } = await runSmoke([`--open=${scratchPath}`, '--edit=freetext', '--save-in-place']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.save?.validated, true);
  const after = await stat(scratchPath);
  const afterBytes = await readFile(scratchPath);
  assert.notDeepEqual(afterBytes, beforeBytes);
  assert.equal(after.mode & 0o777, before.mode & 0o777);
  const verdict = verifyAnnots(scratchPath, { FreeText: 1 }, 'PDF4All P2 freetext', 'FreeText');
  assert.ok(verdict.appearance.inside_frac > 0.01);
});

test('smoke --edit=highlight then undo reports Highlight 0', { skip: skipP2Edit, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/hello.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=highlight']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.undo?.annots?.Highlight, 0);
});

const PAGES_AFTER = { count: 2, rotate: [0, 90], order: ['three', 'one'] };

async function makePagesPdf(dest) {
  await writeFile(dest, makeThreePagePdf());
}

function verifyPages(pdf, expected) {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-roundtrip.py'), pdf, '--pages', JSON.stringify(expected),
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  const verdict = JSON.parse(checked.stdout);
  assert.equal(verdict.ok, true);
  return verdict;
}

function tabPeakMiB(report) {
  const tabs = (report?.memory ?? []).filter(m => m.type === 'Tab');
  const rss = (tabs.length ? tabs : report?.memory ?? []).map(m => m.peakWorkingSetSize ?? m.workingSetSize ?? 0);
  return Math.max(0, ...rss) / 1024; // Electron MemoryInfo is KiB
}

test('smoke --edit=pages --save-in-place rewrites a 3-page scratch', { skip: skipP2Edit, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-pages-'));
  const scratchPath = join(dir, 'pages.pdf');
  await makePagesPdf(scratchPath);
  await chmod(scratchPath, 0o644);
  const { code, report, timedOut } = await runSmoke([`--open=${scratchPath}`, '--edit=pages', '--save-in-place']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.pages?.count, 2);
  assert.deepEqual(report?.edit?.pages?.rotate, [0, 90]);
  assert.equal(report?.save?.validated, true);
  verifyPages(scratchPath, PAGES_AFTER);
});

test('smoke --edit=pages then undo restores count 3', { skip: skipP2Edit, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-pages-undo-'));
  const scratchPath = join(dir, 'pages.pdf');
  await makePagesPdf(scratchPath);
  const { code, report, timedOut } = await runSmoke([`--open=${scratchPath}`, '--edit=pages']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.pages?.count, 2);
  assert.equal(report?.undo?.pages?.count, 3);
});

test('smoke --edit=pages --save-to on large.pdf leaves 499 pages', { skip: skipP2Edit, timeout: 120_000 }, async () => {
  const fixture = join(root, 'fixtures/large.pdf');
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-pages-large-'));
  const dest = join(dir, 'large-pages.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=pages', `--save-to=${dest}`]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.pages?.count, 499);
  assert.equal(report?.save?.validated, true);
  console.log(`Tab peak ${tabPeakMiB(report).toFixed(0)} MiB`);
});

// fill-anywhere D7: a filled document's Save writes a "(filled)" copy, so a smoke save-in-place
// (no path to give) is cancelled and the scratch is never rewritten. The old field/appearance
// assertions moved to tests/fill-anywhere.test.mjs (c), where the copy holds them.
test('smoke --edit=form --save-in-place is cancelled on a filled form2 scratch (Save writes a copy)', { skip: skipP2Edit, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-form-'));
  const scratchPath = join(dir, 'form2.pdf');
  await copyFile(join(root, 'fixtures/form2.pdf'), scratchPath);
  await chmod(scratchPath, 0o644);
  const before = await readFile(scratchPath);
  const { report, timedOut } = await runSmoke([`--open=${scratchPath}`, '--edit=form', '--save-in-place']);
  assert.equal(timedOut, false);
  assert.equal(report?.ok, false);
  assert.match(String(report?.error), /P1_SAVE_ERROR .*cancelled/);
  assert.deepEqual(await readFile(scratchPath), before, 'the scratch bytes are unchanged');
  assert.deepEqual((await readdir(dir)).sort(), ['form2.pdf'], 'nothing else was written');
});

test('smoke --edit=form then undo reverts size', { skip: skipP2Edit, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/form2.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=form']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.fields?.values?.size, 'M');
  assert.equal(report?.undo?.fields?.values?.size, '');
});

const NAV_OUTLINE_BEFORE = [['One', 0], ['Two', 1], ['Two-A', 1], ['Three', 2]];
const NAV_LINKS_BEFORE = {
  0: [{ page: 1 }, { uri: 'https://example.com/' }],
  1: [],
  2: [{ page: 1 }, { blocked: 'file' }],
};
const NAV_OUTLINE_AFTER = [['One', 0], ['Two', null], ['Two-A', null], ['Three', 1], ['PDF4All P2 nav', 1]];
const NAV_LINKS_AFTER = {
  0: [{ uri: 'https://example.com/' }, { page: 1 }],
  1: [{ blocked: 'file' }],
};

function verifyNav(pdf, flag, expected) {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-roundtrip.py'), pdf, flag, JSON.stringify(expected),
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  const verdict = JSON.parse(checked.stdout);
  assert.equal(verdict.ok, true);
  return verdict;
}

test('nav.pdf --outline/--links match the contract fixture', () => {
  const fixture = join(root, 'fixtures/nav.pdf');
  verifyNav(fixture, '--outline', NAV_OUTLINE_BEFORE);
  verifyNav(fixture, '--links', NAV_LINKS_BEFORE);
});

test('smoke --edit=nav --save-in-place rewrites a nav scratch', { skip: skipP2Edit, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-nav-'));
  const scratchPath = join(dir, 'nav.pdf');
  await copyFile(join(root, 'fixtures/nav.pdf'), scratchPath);
  await chmod(scratchPath, 0o644);
  const before = await stat(scratchPath);
  const { code, report, timedOut, stdout } = await runSmoke([`--open=${scratchPath}`, '--edit=nav', '--save-in-place']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.pages?.count, 2);
  const last = report?.edit?.nav?.outline?.at(-1);
  assert.equal(last?.title, 'PDF4All P2 nav');
  assert.equal(last?.page, 1);
  assert.equal(report?.save?.validated, true);
  const after = await stat(scratchPath);
  assert.equal(after.mode & 0o777, before.mode & 0o777);
  verifyNav(scratchPath, '--outline', NAV_OUTLINE_AFTER);
  verifyNav(scratchPath, '--links', NAV_LINKS_AFTER);
  const links = [...stdout.matchAll(/P2_LINK ({.*})/g)].map(m => JSON.parse(m[1]));
  assert.ok(links.some(item => item.allowed === false && String(item.uri).startsWith('file:')), stdout);
  console.log(`Tab peak ${tabPeakMiB(report).toFixed(0)} MiB`);
});

test('smoke --edit=nav then undo restores page 2', { skip: skipP2Edit, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/nav.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=nav']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.pages?.count, 2);
  assert.equal(report?.undo?.pages?.count, 3);
  assert.equal(report?.undo?.nav?.outline?.find(item => item.title === 'Two')?.page, 1);
});

function verifyFieldsUnique(pdf) {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-roundtrip.py'), pdf, '--fields-unique',
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  const verdict = JSON.parse(checked.stdout);
  assert.equal(verdict.ok, true);
  return verdict;
}

test('form2.pdf --fields-unique reports 9 top-level names', () => {
  const verdict = verifyFieldsUnique(join(root, 'fixtures/form2.pdf'));
  assert.equal(verdict.keys, 9);
  assert.equal(verdict.top, 9);
});

test('smoke --edit=assemble --insert-from=form2 --save-in-place merges unique fields', { skip: skipP2Assemble, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-assemble-'));
  const scratchPath = join(dir, 'form2.pdf');
  const source = join(root, 'fixtures/form2.pdf');
  await copyFile(source, scratchPath);
  await chmod(scratchPath, 0o644);
  const before = await stat(scratchPath);
  const { code, report, timedOut } = await runSmoke([
    `--open=${scratchPath}`, '--edit=assemble', `--insert-from=${source}`, '--save-in-place',
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.pages?.count, 2);
  assert.equal(report?.edit?.assemble?.inserted, 1);
  assert.equal(new Set(report?.edit?.assemble?.fieldNames ?? []).size, 18);
  assert.equal(report?.save?.validated, true);
  const after = await stat(scratchPath);
  assert.equal(after.mode & 0o777, before.mode & 0o777);
  verifyPages(scratchPath, { count: 2 });
  const unique = verifyFieldsUnique(scratchPath);
  assert.equal(unique.keys, 18);
  console.log(`Tab peak ${tabPeakMiB(report).toFixed(0)} MiB`);
});

test('smoke --edit=assemble then undo restores page count', { skip: skipP2AssembleUndo, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/form2.pdf');
  const { code, report, timedOut } = await runSmoke([
    `--open=${fixture}`, '--edit=assemble', `--insert-from=${fixture}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.pages?.count, 2);
  assert.equal(report?.undo?.pages?.count, 1);
});

test('smoke --edit=extract --save-to writes page 2 of nav', { skip: skipP2Assemble, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/nav.pdf');
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-extract-'));
  const dest = join(dir, 'two.pdf');
  const { code, report, timedOut } = await runSmoke([
    `--open=${fixture}`, '--edit=extract', `--save-to=${dest}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.deepEqual(report?.edit?.extract?.pages, [1]);
  verifyPages(dest, { count: 1, rotate: [0], order: ['two'] });
  console.log(`Tab peak ${tabPeakMiB(report).toFixed(0)} MiB`);
});

function pngSize(buf) {
  if (buf.length < 24 || buf[0] !== 0x89 || buf[1] !== 0x50) return null;
  return { width: buf.readUInt32BE(16), height: buf.readUInt32BE(20) };
}

function verifyImagePage(pdf, expected) {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-roundtrip.py'), pdf, '--image-page', JSON.stringify(expected),
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  const verdict = JSON.parse(checked.stdout);
  assert.equal(verdict.ok, true);
  return verdict;
}

test('smoke --edit=export-png --save-to writes a PNG of hello page 1', { skip: skipP2Export, timeout: 60_000 }, async () => {
  const dest = join(await mkdtemp(join(tmpdir(), 'pdf4all-export-png-')), 'hello.png');
  const { code, report, timedOut } = await runSmoke([
    `--open=${join(root, 'fixtures/hello.pdf')}`, '--edit=export-png', `--save-to=${dest}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  const buf = await readFile(dest);
  assert.equal(buf[0], 0x89);
  assert.equal(buf[1], 0x50);
  const size = pngSize(buf);
  assert.ok(size && size.width >= 600 && size.height >= 700, JSON.stringify(size));
  console.log(`Tab peak ${tabPeakMiB(report).toFixed(0)} MiB`);
});

test('smoke --edit=export-jpeg --save-to writes a JPEG of hello page 1', { skip: skipP2Export, timeout: 60_000 }, async () => {
  const dest = join(await mkdtemp(join(tmpdir(), 'pdf4all-export-jpeg-')), 'hello.jpg');
  const { code, report, timedOut } = await runSmoke([
    `--open=${join(root, 'fixtures/hello.pdf')}`, '--edit=export-jpeg', `--save-to=${dest}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  const buf = await readFile(dest);
  assert.equal(buf[0], 0xff);
  assert.equal(buf[1], 0xd8);
});

test('smoke --edit=export-txt --save-to writes hello text', { skip: skipP2Export, timeout: 60_000 }, async () => {
  const dest = join(await mkdtemp(join(tmpdir(), 'pdf4all-export-txt-')), 'hello.txt');
  const { code, report, timedOut } = await runSmoke([
    `--open=${join(root, 'fixtures/hello.pdf')}`, '--edit=export-txt', `--save-to=${dest}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  const text = await readFile(dest, 'utf8');
  assert.match(text, /PDF4All/);
});

test('smoke --new-from-images writes a 2-page PDF', { skip: skipP2Export, timeout: 60_000 }, async () => {
  const dest = join(await mkdtemp(join(tmpdir(), 'pdf4all-from-images-')), 'images.pdf');
  const rgb = join(root, 'fixtures/img-rgb.png');
  const gray = join(root, 'fixtures/img-gray.png');
  const { code, report, timedOut } = await runSmoke([
    '--new-from-images', `--images=${rgb},${gray}`, `--save-to=${dest}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.new?.pages, 2);
  verifyImagePage(dest, { page: 0, width: 300, height: 200 });
  verifyImagePage(dest, { page: 1, width: 200, height: 300 });
  console.log(`Tab peak ${tabPeakMiB(report).toFixed(0)} MiB`);
});

test('export JPEG of the images PDF then new-from-images that JPEG', { skip: skipP2Export, timeout: 90_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-jpeg-round-'));
  const pdf = join(dir, 'images.pdf');
  const jpg = join(dir, 'page.jpg');
  const back = join(dir, 'from-jpeg.pdf');
  const rgb = join(root, 'fixtures/img-rgb.png');
  const gray = join(root, 'fixtures/img-gray.png');
  const first = await runSmoke(['--new-from-images', `--images=${rgb},${gray}`, `--save-to=${pdf}`]);
  assert.equal(first.code, 0);
  const exported = await runSmoke([`--open=${pdf}`, '--edit=export-jpeg', `--save-to=${jpg}`]);
  assert.equal(exported.code, 0);
  const again = await runSmoke(['--new-from-images', `--images=${jpg}`, `--save-to=${back}`]);
  assert.equal(again.code, 0);
  assert.equal(again.report?.new?.pages, 1);
});

function verifyDecorate(pdf, flag, expected) {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-roundtrip.py'), pdf, flag, JSON.stringify(expected),
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  const verdict = JSON.parse(checked.stdout);
  assert.equal(verdict.ok, true);
  return verdict;
}

test('layers.pdf --layers lists Base and Notes both on', () => {
  verifyDecorate(join(root, 'fixtures/layers.pdf'), '--layers', [
    { name: 'Base', on: true }, { name: 'Notes', on: true },
  ]);
});

test('smoke --edit=decorate --save-in-place stamps nav.pdf', { skip: skipP2Decorate, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-decorate-'));
  const scratchPath = join(dir, 'nav.pdf');
  await copyFile(join(root, 'fixtures/nav.pdf'), scratchPath);
  await chmod(scratchPath, 0o644);
  const { code, report, timedOut } = await runSmoke([
    `--open=${scratchPath}`, '--edit=decorate', '--save-in-place',
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.decorate?.stamps?.watermark, 3);
  assert.equal(report?.edit?.decorate?.stamps?.header, 3);
  assert.equal(report?.edit?.decorate?.labels?.[0], 'i');
  assert.equal(report?.save?.validated, true);
  verifyDecorate(scratchPath, '--labels', ['i', 'ii', 'iii']);
  verifyDecorate(scratchPath, '--text-has', { 0: ['PDF4All P2 decorate', 'Page 1 of 3'] });
  console.log(`Tab peak ${tabPeakMiB(report).toFixed(0)} MiB`);
});

test('smoke --edit=decorate --save-in-place hides Notes on layers.pdf', { skip: skipP2Decorate, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-layers-'));
  const scratchPath = join(dir, 'layers.pdf');
  await copyFile(join(root, 'fixtures/layers.pdf'), scratchPath);
  await chmod(scratchPath, 0o644);
  const { code, report, timedOut } = await runSmoke([
    `--open=${scratchPath}`, '--edit=decorate', '--save-in-place',
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.decorate?.layers?.[1]?.visible, false);
  if (report?.edit?.decorate?.layersSaved) {
    verifyDecorate(scratchPath, '--layers', [
      { name: 'Base', on: true }, { name: 'Notes', on: false },
    ]);
  }
});

test('smoke --edit=decorate then undo reverts the last op', { skip: skipP2Decorate, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/nav.pdf');
  const { code, report, timedOut } = await runSmoke([`--open=${fixture}`, '--edit=decorate']);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.undo?.decorate?.labels?.[0], '1');
});

function verifyShapes(pdf) {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-roundtrip.py'), pdf,
    '--annots', JSON.stringify({ Square: 1, Circle: 1, Line: 1, Ink: 1, Stamp: 2 }),
    '--appearance', 'Stamp:2',
    '--no-sig',
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  const verdict = JSON.parse(checked.stdout);
  assert.equal(verdict.ok, true);
  assert.equal(verdict.no_sig?.ok, true);
  assert.ok(verdict.appearance.inside_frac > 0.15);
  return verdict;
}

test('--no-sig on nav.pdf (no certificate signature)', () => {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-roundtrip.py'), join(root, 'fixtures/nav.pdf'), '--no-sig',
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  assert.equal(JSON.parse(checked.stdout).ok, true);
});

test('smoke --edit=shapes --save-in-place writes six annots and no /Sig', { skip: skipP2Shapes, timeout: 60_000 }, async () => {
  const dir = await mkdtemp(join(tmpdir(), 'pdf4all-edit-shapes-'));
  const scratchPath = join(dir, 'nav.pdf');
  await copyFile(join(root, 'fixtures/nav.pdf'), scratchPath);
  await chmod(scratchPath, 0o644);
  const image = join(root, 'fixtures/img-gray.png');
  const { code, report, timedOut } = await runSmoke([
    `--open=${scratchPath}`, '--edit=shapes', `--images=${image}`, '--save-in-place',
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.annots?.Square, 1);
  assert.equal(report?.edit?.annots?.Circle, 1);
  assert.equal(report?.edit?.annots?.Line, 1);
  assert.equal(report?.edit?.annots?.Ink, 1);
  assert.equal(report?.edit?.annots?.Stamp, 2);
  assert.equal(report?.edit?.sign?.subj, 'Signature appearance');
  assert.equal(report?.save?.validated, true);
  verifyShapes(scratchPath);
  console.log(`Tab peak ${tabPeakMiB(report).toFixed(0)} MiB`);
});

test('smoke --edit=shapes then undo drops the signature stamp', { skip: skipP2Shapes, timeout: 60_000 }, async () => {
  const fixture = join(root, 'fixtures/nav.pdf');
  const image = join(root, 'fixtures/img-gray.png');
  const { code, report, timedOut } = await runSmoke([
    `--open=${fixture}`, '--edit=shapes', `--images=${image}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0);
  assert.equal(report?.ok, true);
  assert.equal(report?.undo?.annots?.Stamp, 1);
});

function verifyEncryption(pdf, password) {
  const checked = spawnSync(python, [
    join(root, 'scripts/verify-save.py'), '--encryption', pdf, '--password', password,
  ], { encoding: 'utf8' });
  assert.equal(checked.status, 0, checked.stdout + checked.stderr);
  return JSON.parse(checked.stdout);
}

function p2EditLine(stdout) {
  const line = stdout.split('\n').find(row => row.includes('P2_EDIT '));
  return line ?? '';
}

test('verify-save --encryption reports RC4 and AES-256 fixtures', () => {
  const rc4 = verifyEncryption(join(root, 'fixtures/encrypted.pdf'), 'pdf4all-user');
  assert.equal(rc4.encrypted, true);
  assert.equal(rc4.userOpens, true);
  assert.equal(rc4.emptyOpens, false);
  const aes = verifyEncryption(join(root, 'fixtures/encrypted-aes256.pdf'), 'pdf4all-user');
  assert.equal(aes.encrypted, true);
  assert.equal(aes.userOpens, true);
  assert.equal(aes.emptyOpens, false);
  const noedit = verifyEncryption(join(root, 'fixtures/encrypted-noedit.pdf'), 'pdf4all-user');
  assert.equal(noedit.permissions.edit, false);
  assert.equal(noedit.permissions.annotate, false);
});

test('smoke --edit=encrypt --new-passwords sets AES-256 print+annotate', { skip: skipP2Encrypt, timeout: 60_000 }, async () => {
  const dest = join(await mkdtemp(join(tmpdir(), 'pdf4all-encrypt-')), 'out.pdf');
  const user = 'p2u-user';
  const owner = 'p2u-owner';
  const { code, report, timedOut, stdout } = await runSmoke([
    `--open=${join(root, 'fixtures/hello.pdf')}`, '--edit=encrypt', `--new-passwords=${user}:${owner}`, `--save-to=${dest}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0, stdout);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.security?.mode, 'aes-256');
  const line = p2EditLine(stdout);
  assert.equal(line.includes(user), false, line);
  assert.equal(line.includes(owner), false, line);
  const enc = verifyEncryption(dest, user);
  assert.equal(enc.encrypted, true);
  assert.equal(enc.userOpens, true);
  assert.equal(enc.emptyOpens, false);
});

test('smoke --edit=encrypt-keep keeps RC4 and logs no password', { skip: skipP2Encrypt, timeout: 60_000 }, async () => {
  const dest = join(await mkdtemp(join(tmpdir(), 'pdf4all-encrypt-keep-')), 'out.pdf');
  const pw = 'pdf4all-user';
  const { code, report, timedOut, stdout } = await runSmoke([
    `--open=${join(root, 'fixtures/encrypted.pdf')}`, '--edit=encrypt-keep', `--password=${pw}`, `--save-to=${dest}`,
  ]);
  assert.equal(timedOut, false);
  assert.equal(code, 0, stdout);
  assert.equal(report?.ok, true);
  assert.equal(report?.edit?.security?.mode, 'keep');
  const line = p2EditLine(stdout);
  assert.equal(line.includes(pw), false, line);
  const enc = verifyEncryption(dest, pw);
  assert.equal(enc.encrypted, true);
  assert.equal(enc.userOpens, true);
});

test('debt1 row 25: the journal is written to a .tmp sibling and renamed; a failed write keeps the old bytes', async () => {
  const { dir, source, dest } = await scratch();
  const journalPath = join(dir, 'pending-saves.json');
  const previous = JSON.stringify([{ temp: join(dir, '.old.aaaa.tmp'), dest: join(dir, 'old.pdf'), started: 1 }]);
  await writeFile(journalPath, previous, { mode: 0o600 });
  const ok = await saveCopy({ dest, bytes, source, journalPath });
  assert.equal(ok.ok, true);
  assert.equal(await readFile(journalPath, 'utf8'), previous, 'journalAdd + journalDrop leave the previous entry');
  assert.equal((await readdir(dir)).includes('pending-saves.json.tmp'), false);
  if (!WIN) assert.equal((await stat(journalPath)).mode & 0o777, 0o600);

  const failed = await saveCopy({
    dest, bytes, source, journalPath,
    inject: {
      journalWrite: async (fh, text) => {
        await fh.write(text.slice(0, 10));
        throw fsError('ENOSPC');
      },
    },
  });
  assert.deepEqual(failed, { ok: false, reason: 'disk-full' });
  assert.equal(await readFile(journalPath, 'utf8'), previous, 'a write that throws mid-way leaves the previous journal intact');
  assert.equal((await readdir(dir)).includes('pending-saves.json.tmp'), false);
  assert.equal(await tmpCount(dir), 0);
});
