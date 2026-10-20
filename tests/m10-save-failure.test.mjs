/**
 * M10 save failure and recovery (engine/design/m10-fuzz.md, "Save failures"), in the app: every case checks
 * that the original file is byte-identical, that the document stays dirty and that STATE `error` shows the
 * catalogue's plain text. The file-level cases (read-only targets, swaps and links during the write, a crash
 * between write and rename) are Rust tests: crates/tandem-app/tests/safe_write.rs.
 */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import {
  chmod, copyFile, mkdir, mkdtemp as makeTempDir, readFile, readdir, realpath, rename, rm, stat, unlink, writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import test, { after } from 'node:test';
import { appBinary } from './app-bin.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const WIN = process.platform === 'win32';
/** root ignores permission bits, so a read-only file or folder stays writable for it. */
const ROOT = typeof process.getuid === 'function' && process.getuid() === 0;
const skipPerms = WIN || ROOT ? 'needs POSIX permission bits and a non-root user' : false;
const skipApp = process.env.TANDEM_NO_APP ? 'TANDEM_NO_APP' : false;
const skipAppPerms = skipApp || skipPerms;

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
  return { dir, sourcePath };
}

async function temps(dir) {
  return (await readdir(dir)).filter(name => name.startsWith('.') && name.endsWith('.tmp'));
}

async function journal(path) {
  return JSON.parse(await readFile(path, 'utf8'));
}

/** The catalogue's save-failure texts the window shows (crates/tandem-app/src/messages.rs `save::reason`). */
const TEXT = {
  permission: 'The file or its folder is read-only, or Tandem PDF is not allowed to write there. The file on disk was not changed; use Save As… to save somewhere else.',
  'source-changed': 'The file changed on disk since you opened it, so it was not overwritten. Use Save As… to save your version under another name.',
};
const shown = reason => TEXT[reason];

// ------------------------------------------------------------------ a crash between write and rename

/**
 * What a save that crashed between its write and its rename leaves: the written temp beside the file
 * (`.<name>.<random>.tmp`) and its journal entry in `<userData>/pending-saves.json`, the original untouched.
 */
async function crashedSave(sourcePath, journalPath) {
  const source = await realpath(sourcePath);
  const temp = join(source, '..', `.doc.pdf.${'0123456789abcdef01234567'}.tmp`);
  await writeFile(temp, edited);
  await writeFile(journalPath, JSON.stringify([{ temp, dest: source, started: Date.now() }]));
}

test('the app removes a crashed save\'s temp on its next start', { skip: skipApp, timeout: 90_000 }, async () => {
  const { dir, sourcePath } = await scratch();
  const home = await mkdtemp(join(tmpdir(), 'tpdf-m10-udata-'));
  const userData = join(home, 'udata');
  const journalPath = join(userData, 'pending-saves.json');
  await mkdir(userData, { recursive: true });
  await crashedSave(sourcePath, journalPath);
  assert.equal((await temps(dir)).length, 1);

  const report = join(home, 'report.json');
  const child = spawn(appBinary, [`--user-data-dir=${userData}`, '--smoke', `--report=${report}`, `--open=${join(root, 'fixtures/hello.pdf')}`], { cwd: root, stdio: 'ignore' });
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

test('app: Save after the file was replaced or deleted on disk shows source-changed and stays dirty', { skip: skipApp, timeout: 120_000 }, async () => {
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

test('app: an export into a read-only folder shows the permission text', { skip: skipAppPerms, timeout: 120_000 }, async () => {
  const { dir, path } = await appScratch();
  await chmod(dir, 0o555);
  const live = await session();
  try {
    assert.equal((await live.step({ cmd: 'open', path })).state?.doc?.name, 'hello.pdf');
    const state = (await live.step({ cmd: 'export-png', to: join(dir, 'hello.png') })).state;
    assert.match(state.error, /^The PNG was not saved\. .*read-only.*Choose another folder\.$/);
    assert.doesNotMatch(state.error, /Save As/);
    assert.deepEqual(await readdir(dir), ['hello.pdf']);
  } finally {
    await live.close().catch(() => undefined);
    await chmod(dir, 0o755);
  }
});
