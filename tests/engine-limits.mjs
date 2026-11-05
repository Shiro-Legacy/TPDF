// SPDX-License-Identifier: MIT
// The limits and helpers the suite checks the engine against. Most are the
// app's own, in web/ (compiled into build-test/ by `npm run build:test`); the
// rest were constants of the former TypeScript engine and are pinned here to
// the Rust engine's values, each with the Rust constant it mirrors. A drift
// shows up as a refusal the tests do not expect.
export * from '../build-test/engine-shared.js';
export { MAX_PDF_BYTES, MAX_SAVE_BYTES, MAX_REDACT_SOURCE_BYTES, MAX_SIG_CONTENTS_BYTES } from '../build-test/limits.js';
export { OP_PERMISSION, PERMISSION_BITS, allowed } from '../build-test/engine-core.js';

/** engine/src/app/ops/edit.rs `MAX_EDIT_TARGETS`: targets one `edit-targets` reply lists. */
export const MAX_EDIT_TARGETS = 512;
/** engine/src/app/redact/marks.rs `MAX_REDACT_MARKS` and `MAX_REDACT_TEXT` (UTF-16 units of a search). */
export const MAX_REDACT_MARKS = 2000;
export const MAX_REDACT_TEXT = 256;
/** engine/src/app/redact/sanitize.rs `SANITIZE_KEYS`, in its order. */
export const REDACT_SANITIZE_KEYS = ['info', 'xmp', 'attachments', 'outlines', 'annots', 'links', 'fields', 'layers', 'struct'];
/** engine/src/app/ops/stamp.rs `MAX_STAMP_CHARS`. */
export const MAX_STAMP_CHARS = 200;
/** engine/src/app/ops/annots.rs `MAX_SIGN_NAME_CHARS`. */
export const MAX_SIGN_NAME_CHARS = 200;
/** engine/src/app/ops/pages.rs `MAX_GRAFT_PAGES`, `MAX_IMAGES` and `MAX_IMAGE_BYTES`. */
export const MAX_GRAFT_PAGES = 2000;
export const MAX_IMAGES = 50;
export const MAX_IMAGE_BYTES = 32 * 1024 * 1024;
/** engine/src/app/ops/nav.rs `LINK_SCHEMES`: the schemes `link-add` and `link-set` accept. */
export const LINK_SCHEMES = ['http:', 'https:', 'mailto:'];
