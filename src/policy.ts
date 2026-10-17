import { resolve, sep } from 'node:path';
import { MAX_PDF_BYTES, MAX_SAVE_BYTES } from './renderer/limits.js';
import { ENGINE } from './engine-build.js';

export const CSP = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; worker-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

// The ceilings are defined in `renderer/limits.ts` (A1 step 5) so the worker can load
// the exact same numbers over `app://`; re-exported so `main.ts` and tests are unchanged.
export { MAX_PDF_BYTES, MAX_SAVE_BYTES };

export function pdfWithinCeiling(size: number): boolean {
  return size <= MAX_PDF_BYTES;
}

export function saveWithinCeiling(size: number): boolean {
  return size <= MAX_SAVE_BYTES;
}

/** One-shot document token path. `undefined` unless `/open/<uuid>` exactly. */
export function openToken(pathname: string): string | undefined {
  return /^\/open\/([0-9a-f-]{36})$/.exec(pathname)?.[1];
}

export function securityPolicy(appPath: string, preload: string): { preload: string } | undefined {
  const root = resolve(appPath);
  const file = resolve(preload);
  const prefix = root.endsWith(sep) ? root : root + sep;
  if (!file.startsWith(prefix)) return;
  return { preload: file };
}

// Only the engine this build ships (scripts/build.mjs, TPDF_ENGINE): for MuPDF the TypeScript engine
// over it too, which a Rust build neither loads nor serves.
const ENGINE_ASSETS: Record<typeof ENGINE, [string, string][]> = {
  rust: [
    ['/engine-rust.js', 'text/javascript'],
    ['/vendor/tandem_rust_workflow.js', 'text/javascript'],
    ['/vendor/tandem_rust_workflow_bg.wasm', 'application/wasm'],
    // The CJK fallback font the worker reads when a page draws CJK text whose font is not embedded.
    ['/fonts/DroidSansFallback.ttf', 'font/ttf'],
  ],
  mupdf: [
    ['/engine.js', 'text/javascript'],
    ['/content-edit.js', 'text/javascript'],
    ['/redact.js', 'text/javascript'],
    ['/sign-prepare.js', 'text/javascript'],
    ['/vendor/mupdf.js', 'text/javascript'],
    ['/vendor/mupdf-wasm.js', 'text/javascript'],
    ['/vendor/mupdf-wasm.wasm', 'application/wasm'],
  ],
};

const assets = new Map<string, string>([
  ['/index.html', 'text/html; charset=utf-8'],
  ['/renderer.js', 'text/javascript'],
  ['/worker.js', 'text/javascript'],
  ['/engine-core.js', 'text/javascript'],
  ['/engine-shared.js', 'text/javascript'],
  ['/limits.js', 'text/javascript'],
  ['/recovery.js', 'text/javascript'],
  ['/worker-engine.js', 'text/javascript'],
  ['/styles.css', 'text/css'],
  ...ENGINE_ASSETS[ENGINE],
  ['/fixtures/hello.pdf', 'application/pdf'],
  ['/fixtures/form.pdf', 'application/pdf'],
  ['/fixtures/large.pdf', 'application/pdf'],
]);

export function isFixture(name: string): boolean {
  return assets.get(`/fixtures/${name}`) === 'application/pdf';
}

// Exact names only: renderer input never becomes an arbitrary filesystem path.
export function localAsset(input: string): { path: string; type: string } | undefined {
  try {
    const url = new URL(input);
    if (url.protocol !== 'app:' || url.host !== 'pdf4all' || url.username || url.password || url.search || url.hash) return;
    const type = assets.get(url.pathname);
    if (type) return { path: url.pathname.slice(1), type };
  } catch { /* Malformed URLs are denied, like unlisted assets. */ }
}

/**
 * Batch output name (contract-batch.md §9.12): a listed file name or `report.txt`, never a path.
 * `null` for a separator, `.`/`..`, empty, over 255 characters, or a name the folder listing lacks.
 */
export function batchOutName(name: unknown, listed: readonly string[]): string | null {
  if (typeof name !== 'string' || !name || name.length > 255 || name === '.' || name === '..') return null;
  if (name.includes('/') || name.includes('\\') || name.includes('\0')) return null;
  return name === 'report.txt' || listed.includes(name) ? name : null;
}
