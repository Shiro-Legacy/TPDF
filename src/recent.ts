/**
 * The start screen's recent files (UX audit X4): the last few PDFs this person opened, newest first.
 * Main owns the list and the paths; the renderer only ever sees a file name and its folder's name,
 * and opens an entry by its index. A smoke or drive run keeps the list in memory only (path null).
 */
import { readFileSync } from 'node:fs';
import { rename, writeFile } from 'node:fs/promises';
import { basename, dirname, isAbsolute } from 'node:path';

export const MAX_RECENT = 8;

export type RecentEntry = { name: string; folder: string };

export type RecentStore = {
  /** What the start screen shows, newest first. */
  list(): RecentEntry[];
  /** The path behind entry `index`, or undefined. */
  path(index: number): string | undefined;
  /** Moves `path` to the front (adds it when new); the oldest entry past MAX_RECENT drops off. */
  add(path: string): Promise<void>;
  /** Forgets `path` (a file that could not be opened any more). */
  remove(path: string): Promise<void>;
};

export function createRecentStore(file: string | null): RecentStore {
  let paths: string[] = [];
  if (file) {
    try {
      const parsed: unknown = JSON.parse(readFileSync(file, 'utf8'));
      if (Array.isArray(parsed)) paths = parsed.filter((item): item is string => typeof item === 'string' && isAbsolute(item)).slice(0, MAX_RECENT);
    } catch { /* No list yet, or an unreadable one: start empty. */ }
  }
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
    list: () => paths.map(path => ({ name: basename(path), folder: basename(dirname(path)) })),
    path: index => (Number.isInteger(index) && index >= 0 ? paths[index] : undefined),
    add(path) {
      if (!isAbsolute(path)) return Promise.resolve();
      paths = [path, ...paths.filter(item => item !== path)].slice(0, MAX_RECENT);
      return persist();
    },
    remove(path) {
      const before = paths.length;
      paths = paths.filter(item => item !== path);
      return paths.length === before ? Promise.resolve() : persist();
    },
  };
}
