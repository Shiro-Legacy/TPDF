/**
 * Save As of an Untitled or filled document rebinds the window to the file written (final review): main.ts took
 * that file's identity by inspecting `dest` again after saveCopy returned, so a link retargeted in between bound
 * the window to a file it never wrote, and the next Save would overwrite it. saveCopy (and writeNew) now return
 * the identity taken right after the rename from the path actually written, as saveInPlace does, and main.ts
 * binds to it. Style of tests/m10-save-failure.test.mjs.
 */
import assert from 'node:assert/strict';
import { mkdir, mkdtemp as makeTempDir, readdir, readFile, rename, rm, stat, symlink, unlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { after } from 'node:test';
import { blankPdf } from '../build/blank-pdf.js';
import { inspectRegularFile, saveCopy, saveInPlace, writeNew } from '../build/safe-write.js';

const root = fileURLToPath(new URL('..', import.meta.url));
const madeDirs = [];
const mkdtemp = async prefix => { const dir = await makeTempDir(prefix); madeDirs.push(dir); return dir; };
after(async () => { await Promise.all(madeDirs.map(dir => rm(dir, { recursive: true, force: true, maxRetries: 5 }))); });

const edited = Buffer.from('%PDF-edited-bytes-longer');
const precious = Buffer.from('%PDF-precious-second');

/** A symlink, or false where the platform will not make one (Windows without Developer Mode). */
async function trySymlink(target, path) {
  try { await symlink(target, path); return true; } catch { return false; }
}

async function temps(dir) {
  return (await readdir(dir)).filter(name => name.startsWith('.') && name.endsWith('.tmp'));
}

/** An Untitled document as main.ts's newBlank makes it: Untitled.pdf in a fresh folder; and a folder to save into. */
async function untitled() {
  const dir = await mkdtemp(join(tmpdir(), 'tpdf-save-as-'));
  const fresh = join(dir, 'pdf4all-new-x');
  await mkdir(fresh);
  const path = join(fresh, 'Untitled.pdf');
  const bytes = Buffer.from(blankPdf(612, 792));
  await writeFile(path, bytes);
  const journalPath = join(await mkdtemp(join(tmpdir(), 'tpdf-save-as-journal-')), 'pending-saves.json');
  return { dir, path, bytes, source: await inspectRegularFile(path), journalPath };
}

for (const hook of ['rename', 'dirSync']) {
  test(`Save As of an Untitled document onto a link retargeted inside the ${hook} hook binds to the file written`, async t => {
    const { dir, path, bytes, source, journalPath } = await untitled();
    const first = join(dir, 't1.pdf');
    const second = join(dir, 't2.pdf');
    await writeFile(first, Buffer.from('%PDF-old-first'));
    await writeFile(second, precious);
    const alias = join(dir, 'alias.pdf');
    if (!await trySymlink(first, alias)) return t.skip('no symlinks here');
    const retarget = async () => { await unlink(alias); await symlink(second, alias); };
    const inject = hook === 'rename'
      ? { rename: async (from, to) => { await rename(from, to); await retarget(); } }
      : { dirSync: async fh => { await retarget(); await fh?.sync(); } };
    const result = await saveCopy({ dest: alias, bytes: edited, source, inject, journalPath });
    assert.equal(result.ok, true, JSON.stringify(result));
    assert.deepEqual(await readFile(first), edited);
    assert.deepEqual(await readFile(second), precious);
    assert.deepEqual(await readFile(path), bytes, 'the Untitled file is untouched');
    // main.ts rebinds the window to `result.file`: the file written, under the path chosen.
    assert.equal(result.file?.path, alias);
    assert.equal(result.file?.ino, (await stat(first)).ino, 'the identity of the file written');
    assert.equal(result.file?.size, edited.byteLength);
    // So the next Save (in place, through the rebound identity) refuses the file the link now names.
    const next = await saveInPlace({ bytes: Buffer.from('%PDF-next-save'), source: result.file, journalPath });
    assert.deepEqual(next, { ok: false, reason: 'source-changed' });
    assert.deepEqual(await readFile(second), precious);
    assert.deepEqual(await temps(dir), []);
  });
}

test('in-place Save: a link retargeted inside the dirSync hook still reports the file written', async t => {
  const { dir, journalPath } = await untitled();
  const first = join(dir, 't1.pdf');
  const second = join(dir, 't2.pdf');
  await writeFile(first, Buffer.from('%PDF-original-bytes'));
  await writeFile(second, precious);
  const alias = join(dir, 'alias.pdf');
  if (!await trySymlink(first, alias)) return t.skip('no symlinks here');
  const inject = { dirSync: async fh => { await unlink(alias); await symlink(second, alias); await fh?.sync(); } };
  const result = await saveInPlace({ bytes: edited, source: await inspectRegularFile(alias), inject, journalPath });
  assert.equal(result.ok, true, JSON.stringify(result));
  assert.equal(result.file?.ino, (await stat(first)).ino);
  assert.deepEqual(await readFile(second), precious);
});

test('saveCopy and writeNew report the written file for a plain path; main.ts binds Save As to it', async () => {
  const { dir, source, journalPath } = await untitled();
  const dest = join(dir, 'made.pdf');
  const copied = await saveCopy({ dest, bytes: edited, source, journalPath });
  assert.equal(copied.ok, true, JSON.stringify(copied));
  const now = await stat(dest);
  assert.deepEqual([copied.file?.path, copied.file?.ino, copied.file?.dev, copied.file?.size], [dest, now.ino, now.dev, now.size]);
  const made = join(dir, 'new.pdf');
  const written = await writeNew({ dest: made, bytes: edited, journalPath });
  assert.equal(written.ok, true);
  assert.equal(written.file?.ino, (await stat(made)).ino);
  // The Save As branch binds to saveCopy's identity and never re-inspects `dest` (the race), nor sends it to the renderer.
  const main = await readFile(join(root, 'src/main.ts'), 'utf8');
  const branch = main.slice(main.indexOf('const copied = await saveCopy('), main.indexOf("ipcMain.on('dirty'"));
  assert.match(branch, /const file = copied\.file;/);
  assert.doesNotMatch(branch, /inspectRegularFile\(dest\)/);
  assert.match(branch, /const \{ file: _file, \.\.\.rest \} = copied;/);
});
