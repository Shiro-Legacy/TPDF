/**
 * The start screen's recent files (UX audit X4): the last few PDFs this person opened, newest first.
 * Main owns the list and the paths; the renderer only ever sees a file name, its folder's name and an
 * opaque id per entry, and opens an entry by that id (an index would name another file once an entry
 * is dropped). A smoke or drive run keeps the list in memory only (path null).
 */
import { randomUUID } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { rename, writeFile } from 'node:fs/promises';
import { basename, dirname, isAbsolute } from 'node:path';

export const MAX_RECENT = 8;

export type RecentEntry = { id: string; name: string; folder: string };

export type RecentStore = {
  /** What the start screen shows, newest first. */
  list(): RecentEntry[];
  /** The path behind entry `id`, or undefined (an id from an older list whose entry is gone). */
  path(id: string): string | undefined;
  /** Moves `path` to the front (adds it when new); the oldest entry past MAX_RECENT drops off. */
  add(path: string): Promise<void>;
  /** Forgets `path` (a file that could not be opened any more). */
  remove(path: string): Promise<void>;
  /** Forgets every entry. */
  clear(): Promise<void>;
};

export function createRecentStore(file: string | null): RecentStore {
  let paths: string[] = [];
  if (file) {
    try {
      const parsed: unknown = JSON.parse(readFileSync(file, 'utf8'));
      if (Array.isArray(parsed)) paths = parsed.filter((item): item is string => typeof item === 'string' && isAbsolute(item)).slice(0, MAX_RECENT);
    } catch { /* No list yet, or an unreadable one: start empty. */ }
  }
  // Ids live for this run only: a path keeps its id while it stays in the list.
  const ids = new Map<string, string>();
  const idOf = (path: string): string => { let id = ids.get(path); if (!id) { id = randomUUID(); ids.set(path, id); } return id; };
  let writing: Promise<void> = Promise.resolve();
  const persist = (): Promise<void> => {
    if (!file) return Promise.resolve();
    const body = JSON.stringify(paths);
    writing = writing.then(async () => {
      const temp = `${file}.${process.pid}.tmp`;
      try {
        await writeFile(temp, body, { mode: 0o600 });
        await rename(temp, file);
      } catch { /* The list is a convenience: a failed write loses nothing else. */ }
    });
    return writing;
  };
  return {
    list: () => paths.map(path => ({ id: idOf(path), name: basename(path), folder: basename(dirname(path)) })),
    path: id => paths.find(path => ids.get(path) === id),
    add(path) {
      if (!isAbsolute(path)) return Promise.resolve();
      paths = [path, ...paths.filter(item => item !== path)].slice(0, MAX_RECENT);
      return persist();
    },
    remove(path) {
      const before = paths.length;
      paths = paths.filter(item => item !== path);
      ids.delete(path);
      return paths.length === before ? Promise.resolve() : persist();
    },
    clear() {
      paths = [];
      ids.clear();
      return persist();
    },
  };
}
