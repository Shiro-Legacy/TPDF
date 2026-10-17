/**
 * The request bounds and fixed sizes the renderer shares with the engine, and the
 * renderer helpers that used to live in `engine.ts`. Moved out unchanged so the
 * renderer loads no engine code. The MuPDF test oracle
 * (engine/oracle/app-engine/engine.ts) re-exports all of it, and so does the test
 * harness (tests/rust-engine.mjs).
 */

export const MAX_PIXEL_EDGE = 4096;
export const MIN_SCALE = 0.25;
/** Thumbnail long edge bounds; the sidebar cache bounds live here too so tests can assert them. */
export const THUMB_MIN_EDGE = 32;
export const THUMB_MAX_EDGE = 256;
export const THUMB_CACHE_ENTRIES = 64;
export const THUMB_CACHE_BYTES = 24 * 1024 * 1024;
export const MAX_SCALE = 4;

/** A sticky note's fixed size, in PDF points (the point is its top-left). */
export const NOTE_SIZE = 20;

/** A shape's keyboard box (the search hit, else the page centre) and the signature's own. */
export const SHAPE_SIZE: [number, number] = [160, 100];
export const SIGNATURE_SIZE: [number, number] = [200, 60];
/** Ink bounds, mirrored by the renderer's drag recorder. */
export const MAX_INK_STROKES = 64;
export const MIN_INK_POINTS = 2;
export const MAX_INK_POINTS = 2048;
/** A stamp or signature image's ceiling, the bound the picker enforces on its side. */
export const MAX_STAMP_IMAGE_BYTES = 4 * 1024 * 1024;
/**
 * A signature appearance is a picture: `/Subj` says so and the contents say who and
 * when. It is never a `/Sig` field and carries no `/ByteRange` (contract-unit9).
 */
export const SIGNATURE_SUBJECT = "Signature appearance";
/** draw-signature D1: the `/Subj` of a drawn signature (a black Ink, no `/T`, no `/Contents`). */
export const DRAWN_SIGNATURE_SUBJECT = "Drawn signature";

/** Bounds the annotation contents string on the worker seam (IPC is one message). */
export const MAX_NOTE_CHARS = 4096;
/** A bookmark title is user text like a note, so it shares the same ceiling. */
export const MAX_BOOKMARK_CHARS = 4096;
/** A link URI bound; the preload side bounds `open-link` at the same 2048. */
export const MAX_URI_CHARS = 2048;

/** Password entry bound: long enough for any real password, short enough to bound the IPC message. */
export const MAX_PASSWORD_CHARS = 1024;
export const MAX_HITS_DEFAULT = 500;
export const MAX_HITS_CAP = 2000;

/**
 * U6 (bounded form designer): every bound `field-add`/`field-props`/`field-delete`
 * enforce, in one place (contract-p3-unit6.md §2.1, §7.10, archived in
 * .quad/backup/history-20260924.tgz). A name is 1..128 chars;
 * the uniqueness walk visits at most 4096 nodes and descends at most 8 levels.
 */
export const MAX_FIELD_NAME_CHARS = 128;
export const MAX_FIELD_NODES = 4096;
export const MAX_FIELD_DEPTH = 8;
export const MAX_FIELD_KIDS = 32;
export const MAX_FIELD_OPTIONS = 256;
export const MAX_FIELD_OPTION_CHARS = 256;
export const MAX_FIELD_KID_NAME = 64;
export const MAX_FIELD_TEXT_CHARS = 4096;
/** U7: the longest accessible label (`/TU`), contract-p3-unit7.md §2.2 (archived in .quad/backup/history-20260924.tgz). */
export const MAX_FIELD_LABEL = 256;
/** The smallest authored widget, in points (contract §2.1). */
export const MIN_FIELD_RECT_PT = 4;

/**
 * Latest-wins guard for async hand-offs. `begin()` returns a token and only the
 * newest token is current, so a slow fetch that resolves after a newer one is
 * discarded instead of replacing it.
 */
export function createLatestGuard() {
  let current = 0;
  return {
    begin: () => ++current,
    isCurrent: (token: number) => token === current,
  };
}

/**
 * One-slot gate for work queued behind a long worker job (save/validate). Waiters
 * only post after `release()`, so the watchdog timer they then start is not spent
 * queued behind a 120 s rewrite. Single holder by convention.
 */
export function createGate() {
  let held: Promise<void> | null = null;
  let release: (() => void) | null = null;
  return {
    hold(): () => void {
      held = new Promise<void>(resolve => { release = resolve; });
      let done = false;
      return () => {
        if (done) return;
        done = true;
        held = null;
        release?.();
        release = null;
      };
    },
    async wait(): Promise<void> {
      while (held) await held;
    },
  };
}

/**
 * Bounded render cache. Entry count alone is not enough: a page at the 4096 px
 * edge is ~64 MiB of RGBA, so the cache also tracks total pixel bytes.
 */
export type CachedRender = { pixels: ArrayBuffer };

export function createRenderCache<T extends CachedRender>(limitEntries: number, limitBytes: number) {
  const entries = new Map<string, T>();
  let bytes = 0;
  return {
    get(key: string): T | undefined {
      return entries.get(key);
    },
    size(): number {
      return entries.size;
    },
    totalBytes(): number {
      return bytes;
    },
    /** Evicts oldest-first; the newest entry always stays, even alone over the byte budget. */
    set(key: string, value: T) {
      const previous = entries.get(key);
      if (previous) {
        bytes -= sizeOfRender(previous);
        entries.delete(key);
      }
      entries.set(key, value);
      bytes += sizeOfRender(value);
      while (entries.size > 1 && (entries.size > limitEntries || bytes > limitBytes)) {
        const oldest = entries.keys().next().value as string;
        const evicted = entries.get(oldest);
        entries.delete(oldest);
        if (evicted) bytes -= sizeOfRender(evicted);
      }
    },
    clear() {
      entries.clear();
      bytes = 0;
    },
  };
}

function sizeOfRender(value: CachedRender): number {
  return value.pixels?.byteLength ?? 0;
}

/**
 * Half-open `[start, end)` index range of a vertical list visible in a scroll
 * container, expanded by `buffer` items on each side and clamped to the list.
 * `tops[i]`/`heights[i]` are the item's top offset and height.
 */
export function visibleRange(
  scrollTop: number,
  viewportHeight: number,
  tops: number[],
  heights: number[],
  buffer: number,
): { start: number; end: number } {
  const count = Math.min(tops.length, heights.length);
  const top = Number.isFinite(scrollTop) ? scrollTop : 0;
  const height = Number.isFinite(viewportHeight) && viewportHeight > 0 ? viewportHeight : 0;
  const bottom = top + height;
  let first = -1;
  let last = -1;
  for (let index = 0; index < count; index++) {
    if (tops[index] < bottom && tops[index] + heights[index] > top) {
      if (first < 0) first = index;
      last = index;
    }
  }
  if (first < 0) return { start: 0, end: 0 };
  const pad = Math.max(0, Math.trunc(buffer) || 0);
  return { start: Math.max(0, first - pad), end: Math.min(count, last + 1 + pad) };
}

/**
 * Print page range: comma-separated `a`, `a-b`, `a-` (to the end), `-b` (from the
 * start), 1-based, spaces allowed, `a > b` read as reversed. Empty means every
 * page. Duplicates collapse and the result is ascending; `null` means "not a
 * range I can use" - the caller refuses rather than guessing.
 */
export function parsePageRange(text: string, pageCount: number): number[] | null {
  if (!Number.isInteger(pageCount) || pageCount < 0) return null;
  const trimmed = text.trim();
  if (!trimmed) return Array.from({ length: pageCount }, (_, page) => page);
  const wanted = new Set<number>();
  for (const item of trimmed.split(",")) {
    const token = item.trim();
    const range = /^(\d*)\s*-\s*(\d*)$/.exec(token);
    let first: number;
    let last: number;
    if (range && (range[1] || range[2])) {
      first = range[1] ? Number(range[1]) : 1;
      last = range[2] ? Number(range[2]) : pageCount;
    } else if (/^\d+$/.test(token)) {
      first = last = Number(token);
    } else {
      return null;
    }
    if (first > last) [first, last] = [last, first];
    if (first < 1 || last > pageCount) return null;
    for (let page = first; page <= last; page++) wanted.add(page - 1);
  }
  return [...wanted].sort((a, b) => a - b);
}

/** Compare §9.7: trim and collapse runs of whitespace; the LCS compares these. */
export function normaliseLine(text: string): string {
  return text.trim().replace(/\s+/g, " ");
}
