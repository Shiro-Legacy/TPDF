/**
 * The document-size ceilings and request bounds of the worker protocol, defined once:
 * the MuPDF test oracle (engine/oracle/app-engine) enforces them, the tests check the
 * Rust engine's own against MAX_PDF_BYTES and MAX_SAVE_BYTES (tests/p1-worker.test.mjs),
 * and the Rust app's host keeps the same values (crates/tandem-app).
 */

export const MAX_PDF_BYTES = 120 * 1024 * 1024;
/**
 * Separate Save As ceiling, lower than the open ceiling: the rewrite is held in
 * the WASM heap and copied across IPC, so saving is the expensive direction.
 */
export const MAX_SAVE_BYTES = 50 * 1024 * 1024;
/**
 * Redaction temporarily holds the intermediate rewrite, scratch document, touched
 * image pixmaps and validation reopen. Review measurement crossed 578 MB at 24.7
 * MiB, so apply fails closed at 12 MiB until those stages stop retaining copies.
 */
export const MAX_REDACT_SOURCE_BYTES = 12 * 1024 * 1024;
export const MAX_SIG_CONTENTS_BYTES = 8192;
export const MAX_TEXT_ADD_CHARS = 4096;
export const MAX_TEXT_ADD_LINES = 200;
