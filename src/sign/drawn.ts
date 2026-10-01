import { mkdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';

/** draw-signature D3: the one drawn signature, in pad CSS px with its bounds' top-left at the origin. */
export type DrawnSignature = { v: 1; width: number; height: number; strokes: [number, number][][] };

/** sign-merge D2: an image signature — the file's PNG/JPEG bytes as canonical base64 and its decoded pixel size. Never a file name. */
export type ImageSignature = { v: 1; kind: 'image'; width: number; height: number; data: string };
export type SavedSignature = DrawnSignature | ImageSignature;

/** sign-merge D2: base64 of a 4 MiB image (MAX_STAMP_IMAGE_BYTES) is about 5.6 MB, so the read cap is 8 MiB. */
const MAX_FILE_BYTES = 8 * 1024 * 1024;
const MAX_IMAGE_BYTES = 4 * 1024 * 1024; // = engine MAX_STAMP_IMAGE_BYTES; change both together
const round2 = (value: number) => Math.round(value * 100) / 100;
const finite = (value: unknown): value is number => typeof value === 'number' && Number.isFinite(value);

/** draw-signature D3: a valid signature (coordinates rounded to 2 decimals), else null. Used on read and on write. */
export function checkDrawnSignature(value: unknown): DrawnSignature | null {
  if (typeof value !== 'object' || value === null) return null;
  const { v, width, height, strokes } = value as Record<string, unknown>;
  if (v !== 1 || !finite(width) || !finite(height) || width < 8 || width > 4096 || height < 0 || height > 4096) return null;
  if (!Array.isArray(strokes) || strokes.length < 1 || strokes.length > 64) return null;
  const out: [number, number][][] = [];
  let total = 0; // 4096 points in all: mupdf's Ink appearance is quadratic in the point count (the pad stops there too)
  for (const stroke of strokes) {
    if (!Array.isArray(stroke) || stroke.length < 2 || stroke.length > 2048 || (total += stroke.length) > 4096) return null;
    const points: [number, number][] = [];
    for (const point of stroke) {
      if (!Array.isArray(point) || point.length !== 2 || !finite(point[0]) || !finite(point[1])) return null;
      const [x, y] = point as [number, number];
      if (x < -0.01 || y < -0.01 || x > width + 0.01 || y > height + 0.01) return null;
      points.push([round2(x), round2(y)]);
    }
    out.push(points);
  }
  return { v: 1, width: round2(width), height: round2(height), strokes: out };
}

const PNG_MAGIC = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], JPEG_MAGIC = [0xff, 0xd8, 0xff];
const pixels = (value: unknown): value is number => Number.isInteger(value) && (value as number) >= 1 && (value as number) <= 16384;

/** sign-merge D2: a drawing (no `kind`, draw-signature's format) or an image (`kind: "image"`, only its five keys kept), else null. */
export function checkSavedSignature(value: unknown): SavedSignature | null {
  if (typeof value !== 'object' || value === null) return null;
  const { kind } = value as Record<string, unknown>;
  if (kind === undefined) return checkDrawnSignature(value);
  if (kind !== 'image') return null;
  const { v, width, height, data } = value as Record<string, unknown>;
  if (v !== 1 || !pixels(width) || !pixels(height) || typeof data !== 'string' || data.length > Math.ceil(MAX_IMAGE_BYTES / 3) * 4) return null;
  const bytes = Buffer.from(data, 'base64');
  if (bytes.length < 1 || bytes.length > MAX_IMAGE_BYTES || bytes.toString('base64') !== data) return null;
  const starts = (magic: number[]) => magic.every((byte, index) => bytes[index] === byte);
  if (!starts(PNG_MAGIC) && !starts(JPEG_MAGIC)) return null;
  return { v: 1, kind: 'image', width, height, data };
}

/** draw-signature D3: path === null keeps it in memory (smoke/drive/tests). File: temp + rename, 0600, 8 MiB read cap (sign-merge D2). Never logged. */
export function createSignatureStore(path: string | null): {
  get: () => SavedSignature | null;
  set: (value: SavedSignature | null) => void;
} {
  let memory: SavedSignature | null = null;
  return {
    get() {
      if (path === null) return memory;
      try {
        // Review: a regular file only — a FIFO or device at the path would block or flood the main process.
        const info = statSync(path);
        if (!info.isFile() || info.size > MAX_FILE_BYTES) return null;
        return checkSavedSignature(JSON.parse(readFileSync(path, 'utf8')));
      } catch { return null; }
    },
    set(value) {
      const checked = value === null ? null : checkSavedSignature(value);
      if (value !== null && checked === null) throw new Error('signature: not a valid drawing or image');
      if (path === null) { memory = checked; return; }
      if (checked === null) { rmSync(path, { force: true }); return; }
      const tmp = `${path}.${process.pid}.tmp`;
      mkdirSync(dirname(path), { recursive: true });
      // Review: a stale temp keeps its own mode (`mode` applies only on create), and a failed write leaves no temp behind.
      try {
        rmSync(tmp, { force: true });
        writeFileSync(tmp, JSON.stringify(checked), { mode: 0o600, flag: 'wx' });
        renameSync(tmp, path);
      } catch (err) {
        rmSync(tmp, { force: true });
        throw err;
      }
    },
  };
}
