// SPDX-License-Identifier: MIT
// Request bounds the tests assert against that only the Rust engine defines
// (web/engine-shared.ts and web/limits.ts hold the rest; rust-engine.mjs
// re-exports both). Each value restates the named Rust constant; the tests that
// use them check the engine's refusals at or just past these bounds, so a
// drift fails there.

/** engine/src/app/ops/nav.rs `LINK_SCHEMES`: the only schemes the app creates or follows. */
export const LINK_SCHEMES = Object.freeze(['http:', 'https:', 'mailto:']);
/** engine/src/app/ops/edit.rs `MAX_EDIT_TARGETS`: edit targets one page lists. */
export const MAX_EDIT_TARGETS = 512;
/** engine/src/app/ops/pages.rs `MAX_GRAFT_PAGES`: pages one insert carries. */
export const MAX_GRAFT_PAGES = 2000;
/** engine/src/app/ops/pages.rs `MAX_IMAGES`: files one pdf-from-images takes. */
export const MAX_IMAGES = 50;
/** engine/src/app/ops/pages.rs `MAX_IMAGE_BYTES`: one pdf-from-images file's ceiling. */
export const MAX_IMAGE_BYTES = 32 * 1024 * 1024;
/** engine/src/app/redact/marks.rs `MAX_REDACT_MARKS`: redaction marks one document holds. */
export const MAX_REDACT_MARKS = 2000;
/** engine/src/app/redact/marks.rs `MAX_REDACT_TEXT`: UTF-16 units of a redaction search. */
export const MAX_REDACT_TEXT = 256;
/** engine/src/app/ops/annots.rs `MAX_SIGN_NAME_CHARS`: a signature name, in UTF-16 units. */
export const MAX_SIGN_NAME_CHARS = 200;
/** engine/src/app/ops/stamp.rs `MAX_STAMP_CHARS`: a stamp's text, in UTF-16 units. */
export const MAX_STAMP_CHARS = 200;
/** engine/src/app/redact/sanitize.rs `SANITIZE_KEYS`, in its order. */
export const REDACT_SANITIZE_KEYS = Object.freeze([
  'info', 'xmp', 'attachments', 'outlines', 'annots', 'links', 'fields', 'layers', 'struct',
]);
