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

What the view module (`src/view`) leaves to other parts of the work, measured October 14 (`reference/app-view-20261014/summary.json`):

- **Appearance synthesis.** hayro draws stored `/AP` streams only. An annotation or widget without one, or an AcroForm with NeedAppearances, renders blank where MuPDF synthesizes an appearance. The annotations and forms op groups generate appearances into the render overlay.
- **Layer visibility.** hayro reads optional-content state only from the catalog's `/OCProperties /D`. The app's view-only layer toggle (`setLayerVisible`) becomes an edited `/D` (`/ON`, `/OFF`) in the render overlay, never a change to the saved document.
- **Self-invoking content.** hayro has no cycle check. A page whose form, pattern, soft mask or Type3 glyph invokes itself, or whose content expands past about a million content-stream runs, is refused (`ViewError::TooComplex`) instead of drawn with the recursive call skipped as MuPDF does.
- **Clip-mode text (Tr 7)** is neither extracted nor searchable, because hayro emits no glyph run for it.

### 2. Rust owns the SessionRequest contract

Rust implements every `SessionRequest` operation: validation, refusal reasons, reply shapes, PDF mutation and serialization. The TypeScript side is a thin adapter that forwards requests and buffers. This follows the completion plan ("Rust owns PDF interpretation, mutation and serialization"). Porting `engine.ts` is mechanical work with an exact specification: the TypeScript source and the application's own tests.

The alternative of reimplementing the mupdf.js object API under the unchanged TypeScript engine was rejected. It would keep PDF semantics in TypeScript and tie the engine to MuPDF's API shape.

## Acceptance harness

The application's unit tests are the acceptance gate, run against Rust without editing them:

- `tests/engine-select.mjs` registers a Node resolve hook. With `TPDF_ENGINE=rust`, any import of `…/build/renderer/engine.js` resolves to `tests/rust-engine.mjs`.
- `tests/rust-engine.mjs` re-exports everything from the real `engine.js` (constants and pure helpers) and overrides `createEngine` with the Rust adapter.
- In tests only, `session.doc` is a getter that opens the Rust session's current bytes with the real MuPDF module, cached per generation, revision, locked and auth (an unlock replaces the document without a new revision). The copy answers `canUndo`, `canRedo` and `getJournal` from the Rust journal. Tests that inspect the live document therefore inspect Rust's output through an independent reader. MuPDF stays test-only.
- Test-only bridge calls fill in the rest of the session: `session_debug` (password, pending security, held-byte sizes, batch, clipboard, compare slot, journal steps) and `session_set` (the `sourceBytes` and clipboard assignments tests make).
- `npm run test:rust` (`tests/run-rust.mjs`) runs `TPDF_ENGINE=rust CI_NO_ELECTRON=1 node --import ./tests/engine-select.mjs --test tests/*.test.mjs` and reports the pass count. It loads `engine/web/pkg`, which is not tracked; build it first with `engine/tools/build-workflow.sh`. The MuPDF-internal tests (spies and fakes on `saveToBuffer`, prototype patches, an injected raw document, WASM pointer checks) are listed with reasons in `tests/run-rust.mjs` and skipped by name. `--test-skip-pattern` leaves them out of the counts, so the runner prints the list.

Progress is measured as passing app tests under Rust, per op group. A test that encodes a MuPDF quirk may be changed only with a recorded reason, never silently.

## Crate layout

```
src/app/mod.rs        AppSession, dispatch entry, reply envelope
src/app/request.rs    JSON request reading helpers (typed field access, exact messages)
src/app/journal.rs    operation journal: begin/end/abandon, undo/redo, position
src/view/mod.rs       ViewDocument on PDF bytes: open with either password, page geometry
src/view/render.rs    hayro page render, thumbnail
src/view/stext.rs     structured text from hayro glyph runs: chars, lines, page text, search
src/view/walk.rs      content guard run before hayro: cycles, nesting depth, work budget
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

**Save.** Full rewrite of reachable objects without renumbering (`incremental=no,garbage=yes`), Flate for new streams, optional encryption. Signing uses incremental saves that keep the signed prefix (sign package).

**Encrypted save** (`writer.rs`, `encrypt.rs`, `security.rs`). `WriteOptions { encryption, entropy, ..WriteOptions::APP }`:

- `Encryption::Keep` is MuPDF's `encrypt=keep`: the authenticated handler's `/Encrypt` dictionary unchanged (direct in the trailer) and its key, `/ID[0]` as the R2-R4 key was derived, a fresh `/ID[1]`. A document without a handler is written in plaintext. `Encryption::Plaintext` is `encrypt=none`.
- `Encryption::Aes256 { user_password, owner_password, permissions }` is `encrypt=aes-256`: V5 R6, `/CF/StdCF/CFM/AESV3`, a new file key, `/P = security::standard_permissions(permissions)`. The app passes its `permissionInt` and the UTF-8 passwords.
- `entropy: Some(Entropy([u8; 32]))` is required for encrypted output and must be fresh per save. On wasm32 the adapter should send 32 bytes from `crypto.getRandomValues` with each `save`; natively the caller supplies them. An HMAC_DRBG (SHA-256) expands them, so a seed fixes the output bytes.
- For `DocumentInfo` and `validate`: `Document::encryption()` (`EncryptionInfo { filter, v, r, length, method, p, encrypt_metadata }`), `Document::authentication()` (`User` or `Owner`), and `Document::read_encryption(bytes, limits)` for a locked document. `algorithmOf` maps to `method`: `Rc4` with `length` 40 is `rc4-40`, other `Rc4` is `rc4-128`, `AesV2` is `aes-128`, `AesV3` is `aes-256`.
- MuPDF check: `TPDF_ENCRYPT_ORACLE_DIR=<dir> cargo test --release --test encrypt_write oracle_cases`, then `node tools/encrypt-oracle.mjs <app root> <dir> <summary.json>` (`reference/app-encrypt-20261014/summary.json`: 45 of 45).

## Wire format

- `AppEngine::dispatch(session, request: serde_json::Value, buffers: Vec<Vec<u8>>) -> (serde_json::Value, Vec<Vec<u8>>)`.
- In the JSON, binary fields are `{"$buffer": n}` placeholders indexing `buffers`, in both directions. The TypeScript adapter turns `ArrayBuffer` fields into placeholders and placeholders back into `ArrayBuffer`s, and lists output buffers in `transfer`.
- Errors never throw across the boundary. The reply envelope matches engine.ts:4379-4396: `{id, ok:false, error, reason?, detail?, ...extra, revision, dirty, canUndo, canRedo}`. Plain errors have no `reason`; tagged errors do. The per-op split is part of the contract.

As built (bridge `RustApp`, adapter `src/renderer/engine-rust.ts`):

- A dispatch is `clear_inputs()`, `push_input(bytes)` once per placeholder in placeholder order, `dispatch(session, json) -> json`, then `take_output(n)` for each `$buffer` in the reply. `dispatch` takes every queued input, and `clear_inputs` drops anything an abandoned request queued, so nothing carries over to the next request. An input is taken once (`Buffers::take`) or read in place (`Buffers::input`); outputs of a failed op are dropped.
- Markers are single-key objects whose key begins with one `$`:
  - `{"$buffer": n}`: an input in a request, an output in a reply.
  - `{"$input": n}` (`Buffers::echo`): a reply hands request input n back, as `validate` does. The adapter answers with the caller's own `ArrayBuffer` and transfers it, without a copy.
  - `{"$number": "NaN" | "Infinity" | "-Infinity"}`: JSON has no non-finite numbers. `request::as_js_number`, `is_number`, `truthy`, `nullish` and `js_string` read the marker as JavaScript reads the number, so `NaN` is not nullish; `js_number_value` writes one.
- Data keys that begin with `$` cross with one more `$`, both ways (`request::escape_key`, `unescape_key`), so data never reads as a marker. Rust code that writes arbitrary keys into a reply (an Info dictionary, field values) escapes them. A reply marker that names nothing is an adapter error.
- Numbers parse with serde_json's `float_roundtrip`, so Rust reads the exact double JavaScript sent.
- Only `ArrayBuffer` values become placeholders. A typed array serializes as a plain object, so Rust refuses it as engine.ts refuses a non-`ArrayBuffer`. A detached buffer crosses as empty.
- Lone UTF-16 surrogates in request strings and keys become U+FFFD, because Rust's JSON reader refuses them. This is a deliberate divergence: MuPDF's UTF-8 conversion merged a lone surrogate with the next code unit (`"a\ud800b"` became `a` and U+10062, losing the `b`).
- The adapter drops the request's reference to its bytes where engine.ts does: `pdf` for `validate`, and for `open` and `compare-open` once the name is a non-empty string; `bytes` for `batch-file`.
- The reply `id` is the request's own value, set by the adapter as engine.ts sets `request.id`; Rust echoes it for native callers. A request Rust cannot parse (including one nested deeper than serde_json's 128 levels) or that names an unknown session gets a plain error reply. A request the adapter cannot serialize (a `BigInt`, a cycle) or modify (frozen) gets a plain error reply from the adapter.
- A panic is fatal to the whole `RustApp`. On wasm32 it aborts, the trap poisons the object, and wasm-bindgen refuses every later call on it. The adapter treats any throw from the app that way: the failing request and every later request on that app's sessions answer `reason: "worker"`, the session getters keep the last state read, and new sessions get a fresh `RustApp`. The worker switch should restart the worker on that reply. Op code must therefore never panic on any input: no `unwrap`, `expect` or unchecked indexing on request or document data.
- Request fields are read with `app::request` helpers that reproduce the JavaScript checks: `Number.isInteger`, `Number.isFinite`, `typeof`, `Array.isArray`, strict booleans, truthiness, and `String(x)` for messages, including ECMAScript's even-digit rule for ties.

## Order of work

1. Foundation, in parallel: (a) Document changes and journal; (b) hayro render and structured text on bytes; (c) bridge `dispatch`, TypeScript adapter and test harness.
2. Core ops on AppSession: open, close, render, thumbnail, search, text-all, authenticate, save, validate, saved, undo, redo, page-ids, layers, labels, security-set state. Measure the app suite.
3. Op groups in parallel worktrees, each gated by its app tests: pages and assembly; navigation (outline, links, labels, layers); annotations and stamps; forms; content editing; security, redaction and signing; compare and batch.
4. Worker switch in the app build, drive suites on Rust, performance budgets, independent review, docs and ledger.

## Differences from MuPDF and engine.ts

Encryption (writer, security handler):

- `/P` for AES-256 always has bit 10 (0x200) set, as ISO 32000-2 Table 22 requires of writers; otherwise it is MuPDF's `(permissions & 0xF3C) | 0xFFFFF0C0`. MuPDF leaves bit 10 to the caller, so its `hasPermission("accessibility")` is false for the app's flags. `validate` compares only bits 3-6 and 9.
- The app's flags never set bits 11 (assemble) or 12 (high-quality print), so Acrobat limits printing to low resolution and forbids page assembly even when print and edit are granted. This matches MuPDF; changing it is an app decision (map print to 4 + 2048, edit to 8 + 1024).
- `Keep` writes `/Encrypt` directly in the trailer; MuPDF keeps an indirect `/Encrypt` indirect. The parameters are the same.
- When the source has no `/ID`, an encrypted output gets two equal new elements (ISO 32000-2 14.4: a file written for the first time); MuPDF writes two different ones. Both keep `/ID[0]` when present and write a new `/ID[1]`.
- An empty owner password means the user password (Algorithm 9). MuPDF writes an owner entry no password opens as owner; engine.ts already replaces an empty owner password with the user password.
- Signature `/Contents` is neither encrypted nor decrypted in a dictionary of `/Type /Sig` or `/DocTimeStamp` (or untyped) with a `/ByteRange` array and a `/Contents` string (ISO 32000-2 7.6.2). MuPDF exempts only `/Type /Sig` with `/ByteRange`, so it misreads a time stamp's or an untyped signature's `/Contents` in either direction.
- With `/EncryptMetadata false`, `/Type /Metadata` streams are neither decrypted nor encrypted (ISO 32000, as qpdf and Acrobat do). MuPDF 1.28.1 and pypdf decrypt them anyway and read them as empty. `Keep` preserves the stream bytes either way; MuPDF's `encrypt=keep` damages such a stream.
- Objects with `/Type /XRef` are never decrypted or encrypted. The reader used to decrypt dormant cross-reference streams.
- AES-128 (AESV2) uses a 128-bit key whatever `/Length` says, as MuPDF does; the reader used to refuse the password. AESV3 crypt filters below revision 5 are refused; with a short key they could panic.
- Passwords are not SASLprep-normalized, as in MuPDF; a non-ASCII password that SASLprep would change may not open in Acrobat.
- engine.ts `algorithmOf` reports a V4 document with RC4 crypt filters as `aes-128`; `EncryptionInfo::method` is `Rc4` there, so the port can report `rc4-128`.

