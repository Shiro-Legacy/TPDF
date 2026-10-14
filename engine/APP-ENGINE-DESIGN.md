# App engine design (M9 spine)

October 14, 2026. This is the design for replacing the application's MuPDF worker engine (`src/renderer/engine.ts`, `content-edit.ts`, `redact.ts`, `sign-prepare.ts`) with this crate. It refines the completion plan (`../docs/rust-engine-completion-plan.md`) and records two decisions.

## Decisions

### 1. Rendering, text extraction and tolerant parsing use hayro

The bespoke renderer (`page_program` → `page_glyph` → `page_raster`) refuses features that most real PDFs use: every ExtGState (`gs`), DCT/JPX/CCITT/JBIG2 images, embedded CFF and Type1 fonts, shadings and patterns, CMYK paint, hairline strokes, unknown operators and out-of-range colors. Its fixture pass rate is measured on fixtures shaped for it. Closing those gaps one by one is many months of work.

`hayro` 0.8 (`hayro`, `hayro-syntax`, `hayro-interpret`, with its codec crates) is a pure-Rust renderer. Its own crates `forbid(unsafe_code)`, the license is Apache-2.0 OR MIT, and its regression suite has over 1,000 PDFs from the pdf.js and PDFBox suites. Spike results (scratch crate, release build, October 14):

- It opens and renders all 53 fixture documents, encrypted ones with their password. 77 pages compared with MuPDF at 72 DPI: 76 are at least 97% pixel-identical, and objects6.pdf page 1 is 88% identical with no pixel more than 64 levels apart. empty-sigfield.pdf differs by MuPDF's own "SIGN" marker, a known intentional difference.
- Pages render in 2–14 ms, against tens to hundreds of milliseconds for the bespoke pipeline.
- It builds for `wasm32-unknown-unknown`.

So: page display (`render`, `thumbnail`, `compare-render`) and structured text (`render`'s text and lines, `search`, `text-all`, `compare-text`, redaction search, paragraph measurement) come from hayro. The bespoke pipeline stays in the crate. The existing preview Session still uses it for edit validation, and it remains the reference for the strict edit path until that path is ported. It no longer decides whether a page can be viewed.

Costs:

- hayro needs Rust 1.92, so the minimum toolchain rises from 1.85.1 to 1.92. The two-toolchain rule becomes 1.98.1 (current stable here) and 1.92.
- New dependencies (hayro's tree, which includes `vello_cpu`, `fearless_simd`, `moxcms`, `zune-jpeg`, `image`/`png`, `kurbo`, `pic-scale` and others) contain internal unsafe code, as tiny-skia did. They are recorded in DEPENDENCIES.md, NOTICE and reference/licenses, versions pinned with `=`.
- MuPDF remains the measurement oracle. Where hayro and MuPDF differ, choose the better behavior and record it, as before.

### 2. Rust owns the SessionRequest contract

Rust implements every `SessionRequest` operation: validation, refusal reasons, reply shapes, PDF mutation and serialization. The TypeScript side is a thin adapter that forwards requests and buffers. This follows the completion plan ("Rust owns PDF interpretation, mutation and serialization"). Porting `engine.ts` is mechanical work with an exact specification: the TypeScript source and the application's own tests.

The alternative of reimplementing the mupdf.js object API under the unchanged TypeScript engine was rejected. It would keep PDF semantics in TypeScript and tie the engine to MuPDF's API shape.

## Acceptance harness

The application's unit tests are the acceptance gate, run against Rust without editing them:

- `tests/engine-select.mjs` registers a Node resolve hook. With `TPDF_ENGINE=rust`, any import of `…/build/renderer/engine.js` resolves to `tests/rust-engine.mjs`.
- `tests/rust-engine.mjs` re-exports everything from the real `engine.js` (constants and pure helpers) and overrides `createEngine` with the Rust adapter.
- In tests only, `session.doc` is a getter that opens the Rust session's current bytes with the real MuPDF module, cached per generation and revision. Tests that inspect the live document therefore inspect Rust's output through an independent reader. MuPDF stays test-only.
- `TPDF_ENGINE=rust CI_NO_ELECTRON=1 node --import ./tests/engine-select.mjs --test tests/*.test.mjs` reports the pass count. A short list of MuPDF-internal tests (spies, prototype patches, WASM pointer checks) is recorded with reasons and skipped by name.

Progress is measured as passing app tests under Rust, per op group. A test that encodes a MuPDF quirk may be changed only with a recorded reason, never silently.

## Crate layout

```
src/app/mod.rs        AppSession, dispatch entry, reply envelope
src/app/request.rs    JSON request reading helpers (typed field access, exact messages)
src/app/journal.rs    operation journal: begin/end/abandon, undo/redo, position
src/app/render.rs     hayro page render, thumbnail
src/app/stext.rs      structured text from hayro glyph runs: chars, lines, page text, search
src/app/info.rs       DocumentInfo (detect), permissions, page sizes and rotations
src/app/ops/*.rs      one module per op group (core, pages, nav, annots, fields, edit, redact, sign, security, compare, labels, layers, stamp)
```

`serde_json` (already reviewed for the bridge, `=1.0.151`) becomes a core dependency so dispatch runs and is tested natively.

## Document changes

- **Owned input.** `Document` keeps its `'a` parameter but can own its bytes: `Document::open_owned(Vec<u8>, …) -> Document<'static>` (input stored as `Cow<'a, [u8]>`). `AppSession` holds one live `Document<'static>` for the whole session instead of reparsing for every operation.
- **Journal primitives.** `snapshot(id) -> Option<ObjectState>` captures an object's value and stream bytes exactly (raw or replaced, with its filter state); `restore(id, Option<ObjectState>)` puts it back or removes it. The trailer is journaled the same way (`trailer_state`/`restore_trailer`), and the trailer gets setters for Info, ID and Encrypt.
- **Raw streams.** A public way to store a stream with its existing filters (grafted pages, DCT images) and a way to replace only a stream's dictionary.
- **Compression on write.** New and replaced streams are Flate-compressed with `miniz_oxide` when written; original streams keep their bytes.
- **Tolerant open.** When the strict open fails, the document is rebuilt from a repair scan (or from hayro-syntax's repaired view) and marked `repaired`, as MuPDF does. The strict refusals stay available to code that needs proofs.
- **Limits.** App limits replace preview limits for `AppSession`: 120 MiB input, 50 MiB save, no 64-page cap (the stress fixture has 500 pages).

## AppSession

Mirrors `engine.ts`'s `Session` (engine.ts:800-845): document, name, source bytes, locked, auth, password, locked bytes, signed source, pending security, security dirty, saved position, revision, generation, clipboard, compare slot, batch mode. Generation comes from an engine-wide counter (`AppEngine` owns sessions so the counter is shared).

**Journal.** `begin(name)` starts a change set. Every mutation goes through the session's document wrapper, which snapshots an object the first time the operation touches it. `end()` records before/after states and drops the redo branch. `abandon()` restores the befores. Undo and redo apply states in O(changes). `position`, `can_undo`, `can_redo` and step names match MuPDF's journal semantics that `engine.ts` relies on (dirty = position ≠ saved position, the redo-branch rule in `mutate`).

**Render bytes.** hayro reads bytes. The session keeps a render base and appends the current overlay (changed and new objects) as one incremental update whose xref section points back at the base. This is O(changes) per revision and cached per revision.

- The render base is the original input when it opened strictly and is not encrypted.
- Otherwise it is a full plaintext rewrite made once at open or authentication.

**Save.** Full rewrite of reachable objects without renumbering (`incremental=no,garbage=yes`), Flate for new streams, optional encryption (AES-256 writing in the security package). Signing uses incremental saves that keep the signed prefix (sign package).

## Wire format

- `AppEngine::dispatch(session, request: serde_json::Value, buffers: Vec<Vec<u8>>) -> (serde_json::Value, Vec<Vec<u8>>)`.
- In the JSON, binary fields are `{"$buffer": n}` placeholders indexing `buffers`, in both directions. The TypeScript adapter turns `ArrayBuffer` fields into placeholders and placeholders back into `ArrayBuffer`s, and lists output buffers in `transfer`.
- Errors never throw across the boundary. The reply envelope matches engine.ts:4379-4396: `{id, ok:false, error, reason?, detail?, ...extra, revision, dirty, canUndo, canRedo}`. Plain errors have no `reason`; tagged errors do. The per-op split is part of the contract.

## Order of work

1. Foundation, in parallel: (a) Document changes and journal; (b) hayro render and structured text on bytes; (c) bridge `dispatch`, TypeScript adapter and test harness.
2. Core ops on AppSession: open, close, render, thumbnail, search, text-all, authenticate, save, validate, saved, undo, redo, page-ids, layers, labels, security-set state. Measure the app suite.
3. Op groups in parallel worktrees, each gated by its app tests: pages and assembly; navigation (outline, links, labels, layers); annotations and stamps; forms; content editing; security, redaction and signing; compare and batch.
4. Worker switch in the app build, drive suites on Rust, performance budgets, independent review, docs and ledger.
