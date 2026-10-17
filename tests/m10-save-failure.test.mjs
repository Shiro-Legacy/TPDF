/**
 * M10 save failure and recovery (engine/design/m10-fuzz.md, "Save failures"). Every case checks that
 * the original file is byte-identical and that the refusal has plain text in the renderer's
 * SAVE_REASON_TEXT (the renderer marks the document clean only on `result.ok`). The app cases drive
 * the built window and read its STATE: `dirty` stays true and `error` shows that text.
 */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { readFileSync } from 'node:fs';
import {
  chmod, copyFile, link, mkdir, mkdtemp as makeTempDir, readFile, readdir, rename, rm, stat, unlink, writeFile,
} from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import test, { after } from 'node:test';
import { MAX_PDF_BYTES, MAX_SAVE_BYTES } from '../build/renderer/limits.js';
import { cleanupOrphans, inspectRegularFile, saveCopy, saveInPlace } from '../build/safe-write.js';

const root = fileURLToPath(new URL('..', import.meta.url));
const WIN = process.platform === 'win32';
/** root ignores permission bits, so a read-only file or folder stays writable for it. */
const ROOT = typeof process.getuid === 'function' && process.getuid() === 0;
const skipPerms = WIN || ROOT ? 'needs POSIX permission bits and a non-root user' : false;
const skipElectron = process.env.CI_NO_ELECTRON ? 'CI_NO_ELECTRON' : false;
const skipAppPerms = skipElectron || skipPerms;

const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(async () => {
  for (const dir of madeDirs) await chmod(dir, 0o755).catch(() => undefined);
  await Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 })));
});

const original = Buffer.from('%PDF-original-bytes');
const edited = Buffer.from('%PDF-edited-bytes-longer');

async function scratch() {
  const dir = await mkdtemp(join(tmpdir(), 'tpdf-m10-save-'));
  const sourcePath = join(dir, 'doc.pdf');
  await writeFile(sourcePath, original);
  return { dir, sourcePath, journalPath: join(await mkdtemp(join(tmpdir(), 'tpdf-m10-journal-')), 'pending-saves.json') };
}

async function temps(dir) {
  return (await readdir(dir)).filter(name => name.startsWith('.') && name.endsWith('.tmp'));
}

async function journal(path) {
  return JSON.parse(await readFile(path, 'utf8'));
}

// ------------------------------------------------------------------ the renderer's reason texts

/** The quoted names of the first `reason:` union after `marker` (or of the union right after it), sorted. */
function unionOf(source, marker, field = '') {
  let at = source.indexOf(marker);
  assert.ok(at >= 0, `${marker} not found`);
  at += marker.length;
  if (field) at = source.indexOf(field, at) + field.length;
  const body = source.slice(at, source.indexOf(';', at));
  return [...body.matchAll(/['"]([a-z-]+)['"]/g)].map(match => match[1]).sort();
}

const rendererSrc = readFileSync(join(root, 'src/renderer/renderer.ts'), 'utf8');
const SAVE_FAIL = unionOf(readFileSync(join(root, 'src/safe-write.ts'), 'utf8'), 'export type SaveFail =');

/** SAVE_REASON_TEXT, evaluated from the renderer source with the limits it interpolates. */
function saveReasonText() {
  const head = 'const SAVE_REASON_TEXT: Record<string, string> = ';
  const at = rendererSrc.indexOf(head);
  assert.ok(at >= 0, 'SAVE_REASON_TEXT not found');
  const literal = rendererSrc.slice(at + head.length, rendererSrc.indexOf('\n};\n', at) + 2)
    .replace(/\bsatisfies Record<\w+, string>/g, '');
  return new Function('MAX_PDF_BYTES', 'MAX_SAVE_BYTES', `return (${literal});`)(MAX_PDF_BYTES, MAX_SAVE_BYTES);
}
const TEXT = saveReasonText();

/** The text the window shows for a refusal (and the test's own check that there is one). */
function shown(reason) {
  const text = TEXT[reason];
  assert.equal(typeof text, 'string', `no SAVE_REASON_TEXT for ${reason}`);
  assert.ok(text.trim().length > 0, `empty SAVE_REASON_TEXT for ${reason}`);
  return text;
}

test('every SaveFail reason has plain, non-empty text, and the three SaveResult copies agree', () => {
  assert.ok(SAVE_FAIL.length >= 10, SAVE_FAIL.join());
  assert.deepEqual(unionOf(rendererSrc, 'type SaveResult =', 'reason:'), SAVE_FAIL);
  assert.deepEqual(unionOf(readFileSync(join(root, 'src/preload.cts'), 'utf8'), 'type SaveResult =', 'reason:'), SAVE_FAIL);
  for (const reason of SAVE_FAIL) {
    const text = shown(reason);
    assert.match(text, /^[A-Z][^<>{}`$]*[.]$/, `${reason}: one plain sentence or more, no markup or template left`);
    assert.doesNotMatch(text, /\bundefined\b|\bNaN\b|\bnull\b/, reason);
    assert.doesNotMatch(text, /[A-Z]{2,}[_-][A-Z]|E[A-Z]{3,}\b/, `${reason}: no error codes`);
  }
  // In-place Save and Save As share the texts: none of the write failures may speak of "the copy" only.
  for (const reason of ['disk-full', 'permission', 'write-failed', 'unreadable']) assert.doesNotMatch(TEXT[reason], /\bcopy\b/, reason);
  assert.match(TEXT.permission, /read-only/);
});

// ------------------------------------------------------------------ read-only targets

test('in-place Save over a read-only (0444) file is refused and leaves it intact', { skip: skipPerms }, async () => {
  const { dir, sourcePath, journalPath } = await scratch();
  await chmod(sourcePath, 0o444);
  const source = await inspectRegularFile(sourcePath);
  const result = await saveInPlace({ bytes: edited, source, journalPath });
  assert.deepEqual(result, { ok: false, reason: 'permission' });
  shown(result.reason);
  assert.deepEqual(await readFile(sourcePath), original);
  assert.equal((await stat(sourcePath)).mode & 0o777, 0o444);
  assert.deepEqual(await temps(dir), []);
  assert.deepEqual(await journal(journalPath), []);
});

test('Save As over an existing read-only file is refused and leaves both files intact', { skip: skipPerms }, async () => {
  const { dir, sourcePath, journalPath } = await scratch();
  const source = await inspectRegularFile(sourcePath);
  const dest = join(dir, 'other.pdf');
  await writeFile(dest, Buffer.from('%PDF-someone-else'));
  await chmod(dest, 0o444);
  const result = await saveCopy({ dest, bytes: edited, source, journalPath });
  assert.deepEqual(result, { ok: false, reason: 'permission' });
  shown(result.reason);
  assert.deepEqual(await readFile(dest), Buffer.from('%PDF-someone-else'));
  assert.deepEqual(await readFile(sourcePath), original);
  assert.deepEqual(await temps(dir), []);
  assert.deepEqual(await journal(journalPath), []);
});

test('a writable file in a read-only folder: in-place Save and Save As into it are refused', { skip: skipPerms }, async () => {
  const { dir, sourcePath, journalPath } = await scratch();
  const source = await inspectRegularFile(sourcePath);
  await chmod(dir, 0o555);
  try {
    const inPlace = await saveInPlace({ bytes: edited, source, journalPath });
    assert.deepEqual(inPlace, { ok: false, reason: 'permission' });
    const other = await scratch();
    const copy = await saveCopy({ dest: join(dir, 'new.pdf'), bytes: edited, source: await inspectRegularFile(other.sourcePath), journalPath });
    assert.deepEqual(copy, { ok: false, reason: 'permission' });
    shown('permission');
    assert.deepEqual(await readFile(sourcePath), original);
    assert.deepEqual(await readdir(dir), ['doc.pdf']);
    assert.deepEqual(await journal(journalPath), []);
  } finally {
    await chmod(dir, 0o755);
  }
});

test('the target made read-only while the bytes are written is still refused', { skip: skipPerms }, async () => {
  const { dir, sourcePath, journalPath } = await scratch();
  const source = await inspectRegularFile(sourcePath);
  const dest = join(dir, 'other.pdf');
  await writeFile(dest, Buffer.from('%PDF-someone-else'));
  const result = await saveCopy({
    dest, bytes: edited, source, journalPath,
    inject: { writeAll: async (fh, bytes) => { await chmod(dest, 0o444); await fh.writeFile(bytes); } },
  });
  assert.deepEqual(result, { ok: false, reason: 'permission' });
  assert.deepEqual(await readFile(dest), Buffer.from('%PDF-someone-else'));
  assert.deepEqual(await readFile(sourcePath), original);
  assert.deepEqual(await temps(dir), []);
});

// ------------------------------------------------------------------ the target changes during the write

test('in-place Save: the file deleted during the write is refused and not recreated', async () => {
  const { dir, sourcePath, journalPath } = await scratch();
  const source = await inspectRegularFile(sourcePath);
  const result = await saveInPlace({
    bytes: edited, source, journalPath,
    inject: { writeAll: async (fh, bytes) => { await unlink(sourcePath); await fh.writeFile(bytes); } },
  });
  assert.deepEqual(result, { ok: false, reason: 'source-changed' });
  shown(result.reason);
  await assert.rejects(stat(sourcePath), { code: 'ENOENT' });
  assert.deepEqual(await temps(dir), []);
  assert.deepEqual(await journal(journalPath), []);
});

test('in-place Save: the file replaced by another during the write is refused; the other file is kept', async () => {
  const { dir, sourcePath, journalPath } = await scratch();
  const source = await inspectRegularFile(sourcePath);
  const other = Buffer.from('%PDF-replacement-from-another-app');
  const result = await saveInPlace({
    bytes: edited, source, journalPath,
    inject: {
      writeAll: async (fh, bytes) => {
        await writeFile(join(dir, 'incoming'), other);
        await rename(join(dir, 'incoming'), sourcePath);
        await fh.writeFile(bytes);
      },
    },
  });
  assert.deepEqual(result, { ok: false, reason: 'source-changed' });
  shown(result.reason);
  assert.deepEqual(await readFile(sourcePath), other);
  assert.deepEqual(await temps(dir), []);
});

test('Save As: the target replaced by a hard link to the open file during the write is refused', async () => {
  const { dir, sourcePath, journalPath } = await scratch();
  const source = await inspectRegularFile(sourcePath);
  const dest = join(dir, 'copy.pdf');
  const result = await saveCopy({
    dest, bytes: edited, source, journalPath,
    inject: { writeAll: async (fh, bytes) => { await link(sourcePath, dest); await fh.writeFile(bytes); } },
  });
  assert.deepEqual(result, { ok: false, reason: 'same-file' });
  shown(result.reason);
  assert.deepEqual(await readFile(sourcePath), original);
  assert.deepEqual(await temps(dir), []);
});

test('Save As: the old target deleted during the write still gets the new copy; the open file is untouched', async () => {
  const { dir, sourcePath, journalPath } = await scratch();
  const source = await inspectRegularFile(sourcePath);
  const dest = join(dir, 'copy.pdf');
  await writeFile(dest, Buffer.from('%PDF-old-copy'));
  const result = await saveCopy({
    dest, bytes: edited, source, journalPath,
    inject: { writeAll: async (fh, bytes) => { await unlink(dest); await fh.writeFile(bytes); } },
  });
  assert.equal(result.ok, true);
  assert.deepEqual(await readFile(dest), edited);
  assert.deepEqual(await readFile(sourcePath), original);
  assert.deepEqual(await temps(dir), []);
});

// ------------------------------------------------------------------ a crash between write and rename

/** Runs saveInPlace in a child node that SIGKILLs itself inside the rename hook. */
function crashInRename(sourcePath, journalPath) {
  const module = pathToFileURL(join(root, 'build/safe-write.js')).href;
  const script = `
    const { inspectRegularFile, saveInPlace } = await import(${JSON.stringify(module)});
    const source = await inspectRegularFile(${JSON.stringify(sourcePath)});
    await saveInPlace({
      bytes: Buffer.from(${JSON.stringify(edited.toString())}), source, journalPath: ${JSON.stringify(journalPath)},
      inject: { rename: async () => { process.kill(process.pid, 'SIGKILL'); await new Promise(() => {}); } },
    });
    process.exit(3);`;
  const child = spawn(process.execPath, ['--input-type=module', '-e', script], { stdio: ['ignore', 'ignore', 'pipe'] });
  let stderr = '';
  child.stderr.setEncoding('utf8');
  child.stderr.on('data', chunk => { stderr += chunk; });
  return new Promise(resolve => child.on('close', (code, signal) => resolve({ code, signal, stderr })));
}

test('a crash between write and rename leaves the original intact; cleanupOrphans removes the temp', { skip: WIN ? 'no SIGKILL to self on Windows' : false }, async () => {
  const { dir, sourcePath, journalPath } = await scratch();
  const crashed = await crashInRename(sourcePath, journalPath);
  assert.equal(crashed.signal, 'SIGKILL', crashed.stderr);
  assert.deepEqual(await readFile(sourcePath), original);
  const left = await temps(dir);
  assert.equal(left.length, 1, 'the fsynced temp is left behind');
  assert.deepEqual(await readFile(join(dir, left[0])), edited);
  const pending = await journal(journalPath);
  assert.deepEqual(pending.map(entry => basename(entry.temp)), left, 'the journal names the temp (as a realpath)');

  await cleanupOrphans(journalPath);
  assert.deepEqual(await temps(dir), []);
  assert.deepEqual(await journal(journalPath), []);
  assert.deepEqual(await readFile(sourcePath), original);
});

test('the app removes a crashed save\'s temp on its next start', { skip: skipElectron || (WIN ? 'no SIGKILL to self on Windows' : false), timeout: 90_000 }, async () => {
  const { dir, sourcePath } = await scratch();
  const home = await mkdtemp(join(tmpdir(), 'tpdf-m10-udata-'));
  const userData = join(home, 'udata');
  const journalPath = join(userData, 'pending-saves.json');
  await mkdir(userData, { recursive: true });
  const crashed = await crashInRename(sourcePath, journalPath);
  assert.equal(crashed.signal, 'SIGKILL', crashed.stderr);
  assert.equal((await temps(dir)).length, 1);

  const electron = createRequire(import.meta.url)('electron');
  const report = join(home, 'report.json');
  const child = spawn(electron, [`--user-data-dir=${userData}`, '.', '--smoke', `--report=${report}`, `--open=${join(root, 'fixtures/hello.pdf')}`], { cwd: root, stdio: 'ignore' });
  const timer = setTimeout(() => child.kill(), 60_000);
  await new Promise(resolve => child.on('close', resolve));
  clearTimeout(timer);
  assert.equal(JSON.parse(await readFile(report, 'utf8')).ok, true);
  assert.deepEqual(await temps(dir), []);
  assert.deepEqual(await journal(journalPath), []);
  assert.deepEqual(await readFile(sourcePath), original);
});

// ------------------------------------------------------------------ the window: still dirty, text shown

async function session() {
  const { createSession } = await import(pathToFileURL(join(root, 'scripts/session.mjs')).href);
  return createSession();
}

/** Opens `path` in a driven window, adds a note (dirty), runs `save`, returns the STATE after it. */
async function failedSave(path, save, between = async () => {}) {
  const live = await session();
  try {
    const opened = await live.step({ cmd: 'open', path });
    assert.equal(opened.state?.doc?.name, path.split(/[\\/]/).pop(), JSON.stringify(opened));
    const noted = await live.step({ cmd: 'note', contents: 'M10 unsaved note' });
    assert.equal(noted.state.dirty, true);
    await between();
    return (await live.step(save)).state;
  } finally {
    await live.close().catch(() => undefined);
  }
}

function assertRefused(state, reason) {
  assert.deepEqual(state.save, { ok: false, reason });
  assert.equal(state.dirty, true, 'the document stays dirty');
  assert.equal(state.error, shown(reason));
  assert.equal(state.status, 'Not saved');
}

async function appScratch() {
  const dir = await mkdtemp(join(tmpdir(), 'tpdf-m10-app-'));
  const path = join(dir, 'hello.pdf');
  await copyFile(join(root, 'fixtures/hello.pdf'), path);
  await chmod(path, 0o644);
  return { dir, path, bytes: await readFile(path) };
}

test('app: Save on a read-only file shows the permission text and stays dirty', { skip: skipAppPerms, timeout: 120_000 }, async () => {
  const { dir, path, bytes } = await appScratch();
  await chmod(path, 0o444);
  assertRefused(await failedSave(path, { cmd: 'save-in-place' }), 'permission');
  assert.deepEqual(await readFile(path), bytes);
  assert.deepEqual(await temps(dir), []);
});

test('app: Save and Save As in a read-only folder show the permission text and stay dirty', { skip: skipAppPerms, timeout: 120_000 }, async () => {
  const { dir, path, bytes } = await appScratch();
  await chmod(dir, 0o555);
  try {
    assertRefused(await failedSave(path, { cmd: 'save-in-place' }), 'permission');
    assertRefused(await failedSave(path, { cmd: 'save-to', path: join(dir, 'copy.pdf') }), 'permission');
    assert.deepEqual(await readFile(path), bytes);
    assert.deepEqual(await readdir(dir), ['hello.pdf']);
  } finally {
    await chmod(dir, 0o755);
  }
});

test('app: Save after the file was replaced or deleted on disk shows source-changed and stays dirty', { skip: skipElectron, timeout: 120_000 }, async () => {
  const replaced = await appScratch();
  const other = Buffer.from('%PDF-1.4\n% another app wrote this\n');
  assertRefused(await failedSave(replaced.path, { cmd: 'save-in-place' }, async () => {
    await writeFile(join(replaced.dir, 'incoming'), other);
    await rename(join(replaced.dir, 'incoming'), replaced.path);
  }), 'source-changed');
  assert.deepEqual(await readFile(replaced.path), other);
  assert.deepEqual(await temps(replaced.dir), []);

  const deleted = await appScratch();
  assertRefused(await failedSave(deleted.path, { cmd: 'save-in-place' }, () => unlink(deleted.path)), 'source-changed');
  await assert.rejects(stat(deleted.path), { code: 'ENOENT' });
  assert.deepEqual(await temps(deleted.dir), []);
});
