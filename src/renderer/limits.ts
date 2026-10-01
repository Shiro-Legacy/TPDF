/**
 * The two document-size ceilings, defined once for both processes: the main process
 * (`src/policy.ts` re-exports them; `src/main.ts` enforces them on the picker and the
 * save path) and the worker (`engine.ts`). They live under `renderer/` because every
 * module the worker loads over `app://` has to be on the asset map in `policy.ts`.
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
