// The CJK fallback font a Rust build ships (assets/fonts/README.md): Droid Sans Fallback, the font MuPDF
// carries inside its WASM, Apache-2.0. The engine draws CJK text whose font the PDF does not embed with it.
// scripts/build.mjs copies it into the app, scripts/package.mjs ships its licence, and the Node users of the
// Rust adapter (tests/rust-engine.mjs, engine/oracle/corpus-run.mjs) hand the engine these same bytes.
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = fileURLToPath(new URL('../', import.meta.url));

export const CJK_FONT = {
  /** The committed copy, relative to the repository root. */
  source: 'assets/fonts/DroidSansFallback.ttf',
  sha256: '21b96a0377f067833a93af3082eb28d4ffab7a8cd46bfd513286f1d64b7b0949',
  size: 3_451_900,
  /** Where a Rust build puts it, relative to build/renderer (src/policy.ts serves it there). */
  served: 'fonts/DroidSansFallback.ttf',
  /** Apache-2.0 text with the AOSP copyright, shipped beside it. */
  notice: 'docs/third-party/droid-sans-fallback-NOTICE.txt',
};

/** The font's bytes from `dir` (default: this repository); throws when the file is missing or not the pinned font. */
export function readCjkFont(dir = ROOT) {
  const file = join(dir, CJK_FONT.source);
  let bytes;
  try {
    bytes = readFileSync(file);
  } catch (err) {
    throw new Error(`the CJK font ${CJK_FONT.source} is missing (${err.code ?? err.message}); restore it from git`);
  }
  const got = createHash('sha256').update(bytes).digest('hex');
  if (got !== CJK_FONT.sha256) throw new Error(`${CJK_FONT.source}: sha256 ${got} is not the pinned ${CJK_FONT.sha256}`);
  return bytes;
}
