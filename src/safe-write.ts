import { createHash, randomBytes, timingSafeEqual } from 'node:crypto';
import { createReadStream, constants } from 'node:fs';
import { access, chmod, open, readFile, rename, stat, lstat, realpath, unlink, type FileHandle } from 'node:fs/promises';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';
import { Readable } from 'node:stream';
import { MAX_SAVE_BYTES } from './policy.js';

const MIN_PDF_BYTES = 5;
const TEMP_NAME = /^\.[^/]+\.[A-Za-z0-9_-]+\.tmp$/;

type JournalEntry = { temp: string; dest: string; started: number };

export type FileId = { path: string; size: number; mtimeMs: number; dev: number; ino: number };
export type SaveWarning = 'source-changed' | 'durability-uncertain';
export type SaveFail =
  | 'cancelled' | 'too-large' | 'disk-full' | 'permission' | 'unreadable'
  | 'same-file' | 'document-changed' | 'busy' | 'write-failed' | 'source-changed' | 'broken-link';
export type SaveResult =
  | { ok: true; name: string; warning?: SaveWarning; file?: FileId; sameFile?: boolean }
  | { ok: false; reason: SaveFail };

/** Tiny fault seam for tests. Real fs is the default. */
export type WriteInject = {
  writeAll?: (fh: FileHandle, bytes: Uint8Array) => Promise<void>;
  reopen?: (tmp: string) => Promise<Uint8Array>;
  rename?: (from: string, to: string) => Promise<void>;
  dirSync?: (fh: FileHandle) => Promise<void>;
  /** Replaces the journal's temp write (debt1 row 25). */
  journalWrite?: (fh: FileHandle, text: string) => Promise<void>;
};

export function copyBasename(name: string): string {
  const base = basename(name);
  return base.toLowerCase().endsWith('.pdf') ? `${base.slice(0, -4)}-copy.pdf` : `${base}-copy.pdf`;
}

export function classifyFsError(err: unknown): 'disk-full' | 'permission' | 'write-failed' {
  const code = (err as NodeJS.ErrnoException | undefined)?.code;
  if (code === 'ENOSPC' || code === 'EDQUOT') return 'disk-full';
  if (code === 'EACCES' || code === 'EPERM' || code === 'EROFS') return 'permission';
  return 'write-failed';
}

export async function inspectRegularFile(path: string): Promise<FileId | undefined> {
  if (!path) return;
  try {
    const st = await stat(path);
    if (!st.isFile()) return;
    return { path, size: st.size, mtimeMs: st.mtimeMs, dev: st.dev, ino: st.ino };
  } catch {
    return;
  }
}

export async function canonicalDest(dest: string): Promise<string> {
  return join(await realpath(dirname(dest)), basename(dest));
}

export async function destConflictsSource(dest: string, source: FileId): Promise<boolean> {
  let destPath: string;
  try { destPath = await canonicalDest(dest); }
  catch { return false; }
  let srcPath: string;
  try { srcPath = await realpath(source.path); }
  catch { srcPath = resolve(source.path); }
  if (destPath === srcPath) return true;
  try {
    const lst = await lstat(destPath);
    if (lst.isSymbolicLink()) return true;
    if (lst.dev === source.dev && lst.ino === source.ino) return true;
  } catch { /* dest missing */ }
  return false;
}

export function capByteStream(stream: ReadableStream<Uint8Array>, size: number): ReadableStream<Uint8Array> {
  let seen = 0;
  return stream.pipeThrough(new TransformStream<Uint8Array, Uint8Array>({
    transform(chunk, controller) {
      if (seen >= size) return;
      const room = size - seen;
      if (chunk.byteLength > room) {
        controller.enqueue(chunk.subarray(0, room));
        seen = size;
        controller.terminate();
        return;
      }
      seen += chunk.byteLength;
      controller.enqueue(chunk);
    },
    flush(controller) {
      if (seen !== size) controller.error(Object.assign(new Error('truncated'), { code: 'EIO' }));
    },
  }));
}

export function cappedReadStream(path: string, size: number): ReadableStream<Uint8Array> {
  const node = createReadStream(path, { start: 0, end: size - 1 });
  return capByteStream(Readable.toWeb(node) as ReadableStream<Uint8Array>, size);
}

export async function collectCapped(path: string, size: number): Promise<Uint8Array> {
  const meta = await inspectRegularFile(path);
  if (!meta) throw Object.assign(new Error('unreadable'), { code: 'EINVAL' });
  if (meta.size < size) throw Object.assign(new Error('truncated'), { code: 'EIO' });
  const reader = cappedReadStream(path, size).getReader();
  const chunks: Uint8Array[] = [];
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
  }
  let total = 0;
  for (const chunk of chunks) total += chunk.byteLength;
  const out = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return out;
}

export async function saveCopy(input: {
  dest: string | undefined;
  bytes: Uint8Array;
  source: FileId;
  inject?: WriteInject;
  /** Rechecked immediately before rename so Open B during write cannot commit. */
  isCurrent?: () => boolean | Promise<boolean>;
  journalPath?: string;
}): Promise<SaveResult> {
  if (input.dest === undefined) return { ok: false, reason: 'cancelled' };
  if (input.bytes.byteLength > MAX_SAVE_BYTES) return { ok: false, reason: 'too-large' };
  if (input.bytes.byteLength < MIN_PDF_BYTES) return { ok: false, reason: 'unreadable' };
  if (!isAbsolute(input.dest)) return { ok: false, reason: 'unreadable' };
  return writeCopy(input.dest, input.bytes, input.source, input.inject, input.isCurrent, input.journalPath, 'copy');
}

export async function writeNew(input: {
  dest: string | undefined;
  bytes: Uint8Array;
  inject?: WriteInject;
  journalPath?: string;
}): Promise<SaveResult> {
  if (input.dest === undefined) return { ok: false, reason: 'cancelled' };
  if (input.bytes.byteLength > MAX_SAVE_BYTES) return { ok: false, reason: 'too-large' };
  if (input.bytes.byteLength < 1) return { ok: false, reason: 'unreadable' };
  if (!isAbsolute(input.dest)) return { ok: false, reason: 'unreadable' };
  return writeCopy(input.dest, input.bytes, undefined, input.inject, undefined, input.journalPath, 'new');
}

export async function saveInPlace(input: {
  bytes: Uint8Array;
  source: FileId;
  inject?: WriteInject;
  isCurrent?: () => boolean | Promise<boolean>;
  journalPath?: string;
}): Promise<SaveResult> {
  if (input.bytes.byteLength > MAX_SAVE_BYTES) return { ok: false, reason: 'too-large' };
  if (input.bytes.byteLength < MIN_PDF_BYTES) return { ok: false, reason: 'unreadable' };
  if (!input.source.path || !isAbsolute(input.source.path)) return { ok: false, reason: 'unreadable' };
  return writeCopy(input.source.path, input.bytes, input.source, input.inject, input.isCurrent, input.journalPath, 'in-place');
}

async function writeCopy(
  dest: string,
  bytes: Uint8Array,
  source: FileId | undefined,
  inject?: WriteInject,
  isCurrent?: () => boolean | Promise<boolean>,
  journalPath?: string,
  mode: 'copy' | 'in-place' | 'new' = 'copy',
): Promise<SaveResult> {
  let destPath: string;
  try {
    const target = await resolveTarget(dest, mode);
    if (typeof target !== 'string') return { ok: false, reason: target.reason };
    destPath = target;
  } catch (err) {
    return { ok: false, reason: classifyFsError(err) };
  }
  if (mode === 'copy' && source && await destConflictsSource(destPath, source)) return { ok: false, reason: 'same-file' };

  let warning: SaveWarning | undefined = mode === 'copy' && source ? await sourceChanged(source) : undefined;
  const expected = sha256(bytes);
  const dir = dirname(destPath);
  let tmp: string | undefined;
  let fh: FileHandle | undefined;
  let dirFh: FileHandle | undefined;
  let committed = false;
  // M10: every journalled name, so a temp whose create failed (EACCES in a read-only folder) leaves no entry behind.
  const journaled: string[] = [];
  try {
    ({ path: tmp, fh } = await exclusiveTemp(dir, basename(destPath), async path => {
      if (!journalPath) return;
      await journalAdd(journalPath, { temp: path, dest: destPath, started: Date.now() }, inject);
      journaled.push(path);
    }));
    if (inject?.writeAll) await inject.writeAll(fh, bytes);
    else await fh.writeFile(bytes);
    await fh.sync();
    await fh.close();
    fh = undefined;

    const disk = inject?.reopen ? await inject.reopen(tmp) : await readFile(tmp);
    if (disk.byteLength !== bytes.byteLength || !timingSafeEqual(sha256(disk), expected)) {
      return { ok: false, reason: 'write-failed' };
    }

    if (mode === 'copy' && source && await destConflictsSource(destPath, source)) return { ok: false, reason: 'same-file' };
    if (mode === 'in-place' && source) {
      let srcMode: number;
      try {
        const now = await stat(destPath);
        if (!now.isFile() || now.size !== source.size || now.mtimeMs !== source.mtimeMs || now.dev !== source.dev || now.ino !== source.ino) {
          return { ok: false, reason: 'source-changed' };
        }
        srcMode = now.mode;
      } catch {
        return { ok: false, reason: 'source-changed' };
      }
      await chmod(tmp, srcMode & 0o777);
    } else {
      // Replacing an existing file keeps its permission bits (a 0600 file stays private);
      // a new file gets the usual 0666 minus the umask.
      let existing: number | undefined;
      try {
        const now = await stat(destPath);
        if (now.isFile()) existing = now.mode & 0o777;
      } catch { /* no file there yet */ }
      if (existing !== undefined) {
        await chmod(tmp, existing);
      } else {
        const mask = process.umask(0o022);
        process.umask(mask);
        await chmod(tmp, 0o666 & ~mask);
      }
      if (mode === 'copy' && source) warning = await sourceChanged(source) ?? warning;
    }
    if (isCurrent && !await isCurrent()) return { ok: false, reason: 'document-changed' };
    // M10: rename(2) replaces a read-only (0444) file whenever its folder is writable, so Save would
    // silently overwrite a file someone marked read-only. Refuse; Save As can still write elsewhere.
    if (await readOnlyTarget(destPath)) return { ok: false, reason: 'permission' };

    if (inject?.rename) await inject.rename(tmp, destPath);
    else await rename(tmp, destPath);
    committed = true;
    // The identity of the file written, taken right after the rename from the path actually written (a link
    // retargeted meanwhile names another file), under the path the caller gave. Every mode: Save As of an
    // Untitled or filled document rebinds the window to it (final review: it re-inspected `dest` later).
    const written = await inspectRegularFile(destPath);

    try {
      // Windows cannot open a directory to fsync it (the rename is journalled by NTFS itself); the seam still runs for tests.
      if (process.platform !== 'win32') dirFh = await open(dir, constants.O_RDONLY);
      if (inject?.dirSync) await inject.dirSync(dirFh as FileHandle);
      else await dirFh?.sync();
    } catch {
      warning = 'durability-uncertain';
    }
    const file = written ? { ...written, path: dest } : undefined;
    return { ok: true, name: basename(dest), ...(warning ? { warning } : {}), ...(file ? { file } : {}) };
  } catch (err) {
    return { ok: false, reason: classifyFsError(err) };
  } finally {
    if (fh) await fh.close().catch(() => undefined);
    if (dirFh) await dirFh.close().catch(() => undefined);
    if (tmp && !committed) await unlink(tmp).catch(() => undefined);
    if (journalPath && journaled.length) await journalDrop(journalPath, journaled).catch(() => undefined);
  }
}

/** True when `path` exists but this process may not write it (absent, or any other error: false, the rename decides). */
async function readOnlyTarget(path: string): Promise<boolean> {
  try {
    await access(path, constants.W_OK);
    return false;
  } catch (err) {
    const code = (err as NodeJS.ErrnoException).code;
    return code === 'EACCES' || code === 'EPERM' || code === 'EROFS';
  }
}

/**
 * M10 review: the file a save replaces, through any symlinks, so the temp is made beside it and
 * renamed over it; a link keeps being a link. In place, a path that no longer resolves means the
 * file changed (it was deleted); for Save As and new files a link to nothing is refused, and a
 * path that does not exist yet is the new file's (its folder resolved).
 */
async function resolveTarget(dest: string, mode: 'copy' | 'in-place' | 'new'): Promise<string | { reason: SaveFail }> {
  if (mode === 'in-place') {
    try { return await realpath(dest); }
    catch (err) {
      if ((err as NodeJS.ErrnoException).code === 'ENOENT') return { reason: 'source-changed' };
      throw err;
    }
  }
  let link = false;
  try { link = (await lstat(dest)).isSymbolicLink(); }
  catch (err) { if ((err as NodeJS.ErrnoException).code !== 'ENOENT') throw err; }
  if (!link) return canonicalDest(dest);
  try { return await realpath(dest); }
  catch (err) {
    const code = (err as NodeJS.ErrnoException).code;
    if (code === 'ENOENT' || code === 'ELOOP') return { reason: 'broken-link' };
    throw err;
  }
}

async function sourceChanged(source: FileId): Promise<SaveWarning | undefined> {
  const now = await inspectRegularFile(source.path);
  if (!now) return 'source-changed';
  if (now.size !== source.size || now.mtimeMs !== source.mtimeMs || now.dev !== source.dev || now.ino !== source.ino) {
    return 'source-changed';
  }
}

async function exclusiveTemp(
  dir: string,
  base: string,
  beforeCreate?: (path: string) => Promise<void>,
): Promise<{ path: string; fh: FileHandle }> {
  const safe = base.replace(/[^A-Za-z0-9._-]/g, '_').slice(0, 80) || 'pdf';
  for (let i = 0; i < 8; i++) {
    const path = join(dir, `.${safe}.${randomBytes(12).toString('hex')}.tmp`);
    if (beforeCreate) await beforeCreate(path);
    try {
      return { path, fh: await open(path, constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY, 0o600) };
    } catch (err) {
      if ((err as NodeJS.ErrnoException).code !== 'EEXIST') throw err;
    }
  }
  throw Object.assign(new Error('temp'), { code: 'EEXIST' });
}

async function readJournal(path: string): Promise<JournalEntry[]> {
  try {
    const data = JSON.parse(await readFile(path, 'utf8'));
    if (!Array.isArray(data)) return [];
    return data.filter((entry): entry is JournalEntry =>
      entry && typeof entry.temp === 'string' && typeof entry.dest === 'string' && typeof entry.started === 'number');
  } catch {
    return [];
  }
}

/** Write `${path}.tmp` (0600, fsync), then rename over `path`: a crash never leaves a truncated journal. */
async function writeJournal(path: string, entries: JournalEntry[], inject?: WriteInject): Promise<void> {
  const tmp = `${path}.tmp`;
  const fh = await open(tmp, constants.O_WRONLY | constants.O_CREAT | constants.O_TRUNC, 0o600);
  try {
    await fh.chmod(0o600);
    if (inject?.journalWrite) await inject.journalWrite(fh, JSON.stringify(entries));
    else await fh.writeFile(JSON.stringify(entries));
    await fh.sync();
  } catch (err) {
    await fh.close().catch(() => undefined);
    await unlink(tmp).catch(() => undefined);
    throw err;
  }
  await fh.close();
  await rename(tmp, path);
}

async function journalAdd(path: string, entry: JournalEntry, inject?: WriteInject): Promise<void> {
  const entries = await readJournal(path);
  entries.push(entry);
  await writeJournal(path, entries, inject);
}

async function journalDrop(path: string, temps: string[]): Promise<void> {
  await writeJournal(path, (await readJournal(path)).filter(entry => !temps.includes(entry.temp)));
}

export async function cleanupOrphans(journalPath: string): Promise<void> {
  try {
    const remain: JournalEntry[] = [];
    for (const entry of await readJournal(journalPath)) {
      try {
        const st = await lstat(entry.temp);
        if (st.isSymbolicLink() || !st.isFile() || !TEMP_NAME.test(basename(entry.temp)) || dirname(entry.temp) !== dirname(entry.dest)) {
          remain.push(entry);
          continue;
        }
        try { await unlink(entry.temp); }
        catch { remain.push(entry); }
      } catch (err) {
        if ((err as NodeJS.ErrnoException).code !== 'ENOENT') remain.push(entry);
      }
    }
    await writeJournal(journalPath, remain);
  } catch { /* startup must not crash on a bad journal */ }
}

function sha256(data: Uint8Array): Buffer {
  return createHash('sha256').update(data).digest();
}
