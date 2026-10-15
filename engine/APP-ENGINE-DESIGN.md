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
- **Layer visibility.** hayro reads optional-content state only from the catalog's `/OCProperties /D`. The app's view-only layer toggle (`setLayerVisible`) becomes an edited `/D` (`/ON`, `/OFF`) in the render overlay, never a change to the saved document. Done in the view ops (below); hayro also ignores an annotation's `/OC`, which the view now applies.
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
- The layers panel's visibility (`layer-set`) is view-only state that MuPDF keeps on its live document and never writes. The getter reads it with a `layers` request, keys the copy on it as well, and applies it to the copy with `setLayerVisible`.
- Test-only bridge calls fill in the rest of the session: `session_debug` (password, pending security, held-byte sizes, batch, clipboard, compare slot, journal steps) and `session_set` (the `sourceBytes` and clipboard assignments tests make).
- `npm run test:rust` (`tests/run-rust.mjs`) runs `TPDF_ENGINE=rust CI_NO_ELECTRON=1 node --import ./tests/engine-select.mjs --test tests/*.test.mjs` and reports the pass count. It loads `engine/web/pkg`, which is not tracked; build it first with `engine/tools/build-workflow.sh`. The MuPDF-internal tests (spies and fakes on `saveToBuffer`, prototype patches, an injected raw document, WASM pointer checks) are listed with reasons in `tests/run-rust.mjs` and skipped by name. `--test-skip-pattern` leaves them out of the counts, so the runner prints the list.

Progress is measured as passing app tests under Rust, per op group. A test that encodes a MuPDF quirk may be changed only with a recorded reason, never silently.

## Crate layout

```
src/app/mod.rs        AppSession, dispatch entry, reply envelope
src/app/request.rs    JSON request reading helpers (typed field access, exact messages)
src/app/journal.rs    operation journal: begin/end/abandon, undo/redo, position
src/app/document.rs   AppDocument (live Document, journal, view cache) and Transaction
src/app/overlay.rs    render bytes: base plus one incremental update
src/view/mod.rs       ViewDocument on PDF bytes: open with either password, page geometry
src/view/render.rs    hayro page render, thumbnail
src/view/stext.rs     structured text from hayro glyph runs: chars, lines, page text, search
src/view/walk.rs      content guard run before hayro: cycles, nesting depth, work budget
src/app/info.rs       DocumentInfo (detect), permissions, page list, sizes and rotations
src/app/layers.rs     optional-content groups, the layers panel's view-only visibility, the edited /D
src/app/ops/*.rs      one module per op group (core, view, pages, nav, annots, fields, edit, redact, sign, security, compare, labels, stamp)
```

`serde_json` (already reviewed for the bridge, `=1.0.151`) becomes a core dependency so dispatch runs and is tested natively.

## Document changes

- **Owned input.** `Document` keeps its `'a` parameter but can own its bytes: `Document::open_owned(Vec<u8>, …) -> Document<'static>` (input stored as `Cow<'a, [u8]>`). `AppSession` holds one live `Document<'static>` for the whole session instead of reparsing for every operation.
- **Journal primitives.** `snapshot(id) -> Option<ObjectState>` captures an object's value and stream bytes exactly (raw or replaced, with its filter state); `restore(id, Option<ObjectState>)` puts it back or removes it. The trailer is journaled the same way (`trailer_state`/`restore_trailer`), and the trailer gets setters for Info, ID and Encrypt.
- **Raw streams.** A public way to store a stream with its existing filters (grafted pages, DCT images) and a way to replace only a stream's dictionary.
- **Compression on write.** New and replaced streams are Flate-compressed with `miniz_oxide` when written; original streams keep their bytes.
- **Tolerant open.** When the strict open fails, the document is rebuilt from a repair scan (or from hayro-syntax's repaired view) and marked `repaired`, as MuPDF does. The strict refusals stay available to code that needs proofs.
- **Ordered dictionaries.** `object::Dictionary` orders keys as MuPDF's dictionaries do on insertion, so the app save keeps each dictionary's source key order and a new dictionary the order the op code builds it in. Replacing a value keeps its position; a parser `KeepLast` duplicate keeps the first position with the last value (`pdf_dict_put`). As in MuPDF (`pdf_dict_get_put`), a put (new key or replaced value) into a dictionary that already holds more than 100 entries sorts it by key bytes, so an insertion-ordered dictionary has at most 101 entries and is searched linearly, and a sorted one is a `BTreeMap`: a 100k-key dictionary parses in O(n log n). Removal keeps the others in order and a sorted dictionary sorted, which MuPDF does not (see Differences). Equality ignores order (PDF semantics), so journal states and tests compare content; two dictionaries with the same key order compare in one pass. The canonical style still writes keys in byte order, so strict rewrites and digests are unchanged; validation loops whose first error or identity index depends on order iterate `sorted_entries()`. Repair output keeps source order and /Length's position. `tools/compare-app-save.mjs` compares the app save with MuPDF's object by object: all 1,586 objects of the 54 fixtures match (702 with sorted keys). It reports trailers separately: 37 of 54 match, and the other 17 differ only as the last Differences bullet says.
- **Shared and locked opens.** `Document::open_shared(Arc<Vec<u8>>, …)` leaves the caller its bytes when the open fails (the repair fallback needs them), and `open_shared_locked` reads the structure of an encrypted document without its password.
- **Limits.** App limits replace preview limits for `AppSession` (`app::document::limits`): 120 MiB input, 50 MiB save, no 64-page cap (the stress fixture has 500 pages). One object may use the whole value and decoded-byte budget (a flat /Kids of 120,000 pages opens); 65,536 xref sections and 2 million xref entries, past which a repair rebuild collapses the file into one section.

## AppSession

Mirrors `engine.ts`'s `Session` (engine.ts:800-845): document, name, source bytes, locked, auth, password, locked bytes, signed source, pending security, security dirty, saved position, revision, generation, clipboard, compare slot, batch mode. Generation comes from an engine-wide counter (`AppEngine` owns sessions so the counter is shared).

**Document.** `AppDocument` (`src/app/document.rs`) owns the live `Document<'static>`, its journal and the views derived from it. `AppDocument::load` (`load_any` takes several encodings of one password) opens strictly, then with junk before the header skipped (MuPDF's bias, not damage), falls back to a repair rebuild (`repaired` = the report's `damaged`; size limits stop the open, the structural ones fall back), and opens a document whose password is unknown with `Document::open_shared_locked`: structure only, as MuPDF reads it before authentication (strings and streams encrypted, object-stream members absent), so open can report page geometry and /P while locked.

**Passwords** (`src/app/password.rs`). As MuPDF: PDFDocEncoding for R2-R4 (characters it lacks are dropped), UTF-8 unchanged for R5/R6 (MuPDF's SASLprep is a stub), ending at the first NUL. `password::candidates` tries MuPDF's bytes first and the other encoding second; `authenticate` and `validate` use it. The core handler opens with the empty password as owner only when it is also the user password (Acrobat's rule).

**Journal and transactions** (`src/app/journal.rs`). The document is private; the only write access is a `Transaction`, which exists only while the journal records an operation (`AppSession::tx`, inside `AppSession::mutate`). Every write snapshots the object (or the trailer) on its first touch. Methods: `put_value`, `replace_value`, `put_stream_raw`, `put_stream_decoded`, `replace_decoded_stream`, `set_stream_dict`, `update(id, |value| ...)`, `remove_object`, `delete_objects`, `allocate`, `allocate_stream_raw`, `set_trailer_entry`; reads go through `tx.document()`. A failed write that changed nothing is not a touch. `allocate` never reuses a number: it starts past every /Size, every object of the file and every number allocated before, and undo or abandon do not lower it (MuPDF's xref never shrinks), so a stale annotation id or a dangling reference never names a new object. It costs O(log n).

- `end` records a step only when the operation touched something (a write of an equal value counts, as in MuPDF); it drops the redo branch.
- `abandon` restores the befores and drops the redo branch only when something was touched.
- Undo and redo restore one step's before or after states, O(objects in the step), and return the step name.
- Each step has a serial number that is never reused; a save ticket names the state by it.

**`AppSession::mutate(step, work)`** is engine.ts `mutate` (:1886-1908): `Batch::Plan` skips `work` (`Ok(None)`), `Batch::Run` runs it inside the batch's operation with no step or revision, a call inside an open operation joins it, otherwise begin, `work(session)`, end or abandon, the redo-branch rule (`saved_position = -1` when a cut branch held the saved state, on both paths), `revision += 1` on success. `work` gets the session, so nested dispatch (edit-batch) works. Helpers for op groups: `require_mutable(op)` with `OP_PERMISSION` and the exact messages, `check_page`, `info()` (detect), `page_state()` and `edit_state()`, `security_state()`, `view()`.

**Page list and geometry** (`src/app/info.rs`). Pages are enumerated as hayro enumerates them, so indices agree with the view; sizes are MuPDF's bound page computed as the view computes them (a test compares both on every fixture), rotations are the raw inherited /Rotate as engine.ts reports it.

**Render bytes** (`src/app/overlay.rs`). hayro reads bytes. The view is the render base plus one incremental update holding the objects changed since the base, with a classic xref section whose /Prev points at the base's (tested on classic, xref-stream with object-stream and incremental bases). It is cached per content state, which every write, undo, redo and abandon bumps, so a view taken inside an operation shows its pending writes.

- The base is the document's input when it is unencrypted (a repaired document's input is the rebuilt file). Otherwise it is a plaintext rewrite of every object (no renumbering or garbage collection), made on first use.
- When the update outgrows half the base (and 1 MiB), or an object that a rewritten base printed as null (a dangling reference) comes into existence, the base is rebuilt from the current state.
- An object the live document no longer has is an in-use `null` in the view's update (hayro ignores free entries and would still read the base's object) and a free entry in `snapshot_bytes`, as MuPDF's xref marks it.
- Measured (release, M-series): large.pdf (500 pages) opens in 3 ms, first view 1 ms, view after an edit 1.6 ms. A generated 105 MB, 400-page file: open 3 ms, first view 23 ms, view after an edit 11 ms, undo plus view 5 ms; page state 0.4 ms.

`snapshot_bytes` (the test harness's MuPDF copy) is the source while locked, the input of an encrypted document while its journal is at position 0 (it keeps its encryption), and the render bytes otherwise.

**Save.** Full rewrite of reachable objects without renumbering (`WriteOptions::APP`), 50 MiB ceilings in engine.ts order. `save_encryption(session)` is the one place a save picks encryption, from `AppSession::effective_security`: a pending `none` or `aes-256`, else what the last acknowledged save wrote (`AppDocument::written_security`, set by `saved`), else the source's own. Plaintext for plain documents and `none`; `keep` of an encrypted file is `Encryption::Keep` and `aes-256` is `Encryption::Aes256` (below), with entropy the adapter sends. `security_state` and `validate` resolve `keep` the same way. The reply adds a `ticket` (`generation:state:security`) that `saved` accepts. Signing uses incremental saves that keep the signed prefix (sign package).

**Timing.** `std::time::Instant` panics on wasm32-unknown-unknown, so Rust reports `openMs` and `saveMs` as 0 and the adapter measures the whole dispatch and fills them in.

## View ops

`src/app/ops/view.rs`: `render`, `thumbnail`, `search`, `text-all`, `layers` and `layer-set` with engine.ts's check order, messages (plain, except `text-all`'s tagged `read-only` and `too-large`) and reply fields. All of them read `AppDocument::view`.

- **render** draws with annotations, as MuPDF's `toPixmap` with View usage (NoView, Popup and hidden-layer annotations are left out), returns the RGBA pixels as an output buffer, and takes `text` and `lines` from the preserve-spans layout (`spans_text`, `line_summaries`). `nonWhite` counts pixels whose RGB is not white. The body is `render_page(view, request, buffers)` (engine.ts `renderPage`), which compare-render can share: page and scale checks, then the view, the `render:` messages, and the fields from `width` on. **thumbnail** is the same drawing at `min(maxEdge/w, maxEdge/h, 4)`, without text.
- **Limits.** A page is refused as too complex (`ViewError::TooComplex`) when its text operators show more than `MAX_SHOWN_BYTES` (1 MiB) of string bytes, counted each time a form draws them (checked by the content walk before hayro runs, as hayro holds a glyph per byte of a show operation), or when its structured text would hold more than `MAX_PAGE_CHARS` (1,048,576) characters, ToUnicode expansions and ActualText included. About 330 bytes of WASM memory go to each character, so one page stays near 350 MiB; a one-Tj page of 14 million glyphs (14 KB compressed) used to abort the WASM engine. `render` fails on such a page; `search` and `text-all` read it as empty.
- **Text reader.** `search` and `text-all` read pages with one `TextReader`, whose hayro cache keeps fonts loaded across pages. It starts afresh before a page once its fonts' decoded programs (measured while the page is scanned) would pass 64 MiB, or it holds 64 fonts, or it has served 64 pages, so the reader's memory is that of a few pages rather than the sum over the document.
- **search** runs `StructuredText::search_hits` page by page with one `TextReader` until the hit budget is full; `truncated` is set once a further match is found, on the same page or a later one, as engine.ts does.
- **text-all** joins every page's preserve-whitespace text, JavaScript-trimmed, with form feeds; the copy permission is `info::permissions` (MuPDF `hasPermission`), and the 50 MiB budget is checked after each page.
- **Timing.** `paintMs`, `renderMs` and `searchMs` are 0 from Rust; the adapter fills them with the dispatch time, so `paintMs` includes text extraction.
- **Layers.** `app::layers` lists the groups of `/OCProperties /OCGs` in MuPDF's order with the default configuration's visibility. `layer-set` stores a per-group override in `AppDocument` (`LayerState`; not journaled, no revision, reset when the document is replaced, as MuPDF keeps it on its `pdf_document`). The view is cached on (content state, layer serial).
- **View redefinitions** (`app::view_fixes`). The view's bytes are the render bytes plus one more incremental update, written in the same allocation, that redefines: the innermost indirect object holding `/D` (the configuration, `/OCProperties` or the catalog) with `/ON` and `/OFF` edited by the panel's overrides; every annotation hayro would draw that View usage leaves out (NoView, Popup, or an `/OC` that is off under the edited configuration) with the Hidden flag set, or the array or page holding it when it is direct; and every page whose MediaBox is missing or empty with a Letter MediaBox and CropBox. A redefinition wins over the object wherever the bytes stored it, object streams included (an in-place patch cannot reach those). hayro reads `/D` for page content, so render, thumbnail, search and text-all all honour the panel, as MuPDF's `setLayerVisible` does. Saves and `snapshot_bytes` never contain any of it. `ViewDocument::open` on other bytes still hides only annotations stored as top-level objects.
- **Page boxes.** The page frame is the CropBox within the MediaBox, a missing MediaBox counting as empty; an empty result is Letter (so an empty MediaBox ignores the CropBox, as MuPDF does) and a sliver the unit square. When the CropBox misses a non-empty MediaBox the page is blank: nothing drawn, no `text` or `lines`, while `search` still finds its text in the Letter frame, as MuPDF does. `info::page_size` follows the same rule.
- Measured against MuPDF 1.28.1 through the app's adapter on 40 pages of 20 fixtures at scale 1: the same pixel sizes, render `text` and `lines` on every page of that sample, at least 98.9% of pixels within 16 levels; the same search totals and `text-all` text; on `layers.pdf` the same items, text and search after each toggle. The differences found elsewhere are listed below.
- Cost (release, M-series, medians). Native render op (pixels and text) at scale 1 / 2: large.pdf pages 1.3 / 2.7 ms, quartz.pdf 8.7 / 10.1 ms, a dense generated text page (11,000 glyphs, 200 strokes) 11.9 / 16.8 ms; searching all 500 pages of large.pdf 90 ms. WASM in node (warm), with MuPDF's WASM in brackets: large.pdf 3.1 [3.1] / 6.5 [10] ms, quartz.pdf 19 [12] / 22 [20] ms, dense page 40 [10] / 62 [19] ms, search of large.pdf 126 [96] ms, text-all 133 [145] ms. A small edit on large.pdf then the render of the edited page costs 1.1 ms natively (overlay plus reparse of the view), undo plus render 1.2 ms. Re-measured after the review fixes on a loaded machine, WASM: large.pdf 3.0 [2.9] / 5.8 [10] ms, search 130 [85] ms, text-all 145 [88] ms; the reader cache's resets cost nothing measurable on large.pdf (132 ms either way, native).

**Encrypted save** (`writer.rs`, `encrypt.rs`, `security.rs`). `WriteOptions { encryption, entropy, ..WriteOptions::APP }`:

- `Encryption::Keep` is MuPDF's `encrypt=keep`: the authenticated handler's `/Encrypt` dictionary unchanged (direct in the trailer) and its key, `/ID[0]` as the R2-R4 key was derived, a fresh `/ID[1]`. A document without a handler is written in plaintext. `Encryption::Plaintext` is `encrypt=none`.
- `Encryption::Aes256 { user_password, owner_password, permissions }` is `encrypt=aes-256`: V5 R6, `/CF/StdCF/CFM/AESV3`, a new file key, `/P = security::standard_permissions(permissions)`. The app passes its `permissionInt` and the UTF-8 passwords.
- `entropy: Some(Entropy([u8; 32]))` is required for encrypted output and must be fresh per save. On wasm32 the adapter should send 32 bytes from `crypto.getRandomValues` with each `save`; natively the caller supplies them. Two HMAC_DRBG (SHA-256) instances with different personalization strings expand them, one for the AES-256 file key and one for salts, IVs and `/ID`, so a seed fixes the output bytes, and reused entropy never publishes a key.
- Passwords are the UTF-8 the app receives, for `Document::open_with_password` and `Aes256` alike. R5/R6 hash them as given. R2-R4 hash their PDFDocEncoding (characters without a code dropped), as MuPDF does, then the bytes as given.
- For `DocumentInfo` and `validate`: `Document::encryption()` (`EncryptionInfo { filter, v, r, length, method, p, encrypt_metadata }`), `Document::authentication()` (`User` or `Owner`), and `Document::read_encryption(bytes, limits)` for a locked document. `algorithmOf` becomes, in order: no `/Encrypt` is `none`; `v >= 5` or `method` `AesV3` is `aes-256`; `AesV2` is `aes-128`; `Rc4` is `rc4-40` when `r <= 2` or `length == 40`, otherwise `rc4-128`; `Identity` or `None` (V4 with Identity filters, or an unknown `/CFM`) is `unknown`.
- `validate`: `write_document` never changes the session document, while MuPDF's `encrypt=aes-256` save writes `/Encrypt` and `/ID` into the live trailer, which is what makes engine.ts's `before.encrypted` match the copy. The port takes `expectedEncrypted` from the pending setting: `aes-256` is true, `none` is false, `keep` or no setting is `before.encrypted`. It must not emulate the side effect with `set_trailer_entry`: `Keep` refuses a trailer `/Encrypt` without an authenticated handler.
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

- `Dictionary::remove` and `retain` keep the other keys in order, and a sorted dictionary stays sorted. MuPDF's `pdf_dict_del` moves the last entry into the hole and clears the sorted flag, so after a delete of any key but the last MuPDF saves a different key order: for `delete AP` then `put AP` (sign-prepare.ts) on `<</Type/Annot/Subtype/Widget/FT/Sig/T(Signature1)/Rect[..]/AP<<>>/F 4/P 7>>` MuPDF writes `.../Rect[..]/P 7/F 4/AP<<>>`, the app save `.../Rect[..]/F 4/P 7/AP<<>>`. The same applies to `Document::put_stream_decoded` and `allocate` removing /Filter and /DecodeParms. No app-test regex depends on it.
- Writes through `get_mut`, `iter_mut` and `values_mut` never sort, while MuPDF's `pdf_dict_put` of an existing key into a dictionary of more than 100 entries does. Op code that needs MuPDF's order on such a dictionary uses `insert`.
- The compact printer moves /Type first when its value is not null, then /Subtype when that is not null, as MuPDF does; a reference counts as null when it is printed as null.
- The app save (`live` in writer.rs) drops every object whose value resolves to null through `Document::resolve_id`, as MuPDF's `garbage=yes` does: null objects, chains of references ending at null or at a missing object, and reference cycles. References to them print as null and an Info that resolves to null leaves the trailer. `Document::resolve` follows 64 hops where MuPDF follows 10, so a chain of 11 to 64 references to a value is kept and written here; MuPDF drops it and prints references to it as null. The save keeps what the engine itself reads.
- A stream's /Length is written inline and is not a reachability edge in the app save, so an object referenced only as some stream's /Length is dropped, as MuPDF drops it. The strict `rewrite_reachable` still keeps it.
- A stream dictionary that holds more than 100 entries with /Length is printed sorted, as MuPDF's writer `pdf_dict_put` of /Length sorts it. When /Length is the last key both before and after that sort, MuPDF 1.28.1 writes `/Length n/Length n`; the app save writes it once.
- Trailers: /Size is the largest written object number plus one; MuPDF writes its xref length, which counts dropped objects (7 of 54 fixtures). The second /ID element is copied from the source; MuPDF writes a new one for encrypted (saved with `encrypt=none`) and signed or certified fixtures (17 of 54).

Encryption (writer, security handler):

- `/P` for AES-256 always has bit 10 (0x200) set, as ISO 32000-2 Table 22 requires of writers; otherwise it is MuPDF's `(permissions & 0xF3C) | 0xFFFFF0C0`. MuPDF leaves bit 10 to the caller, so its `hasPermission("accessibility")` is false for the app's flags. `validate` compares only bits 3-6 and 9.
- The app's flags never set bits 11 (assemble) or 12 (high-quality print), so Acrobat limits printing to low resolution and forbids page assembly even when print and edit are granted. This matches MuPDF; changing it is an app decision (map print to 4 + 2048, edit to 8 + 1024).
- `Keep` writes `/Encrypt` directly in the trailer; MuPDF keeps an indirect `/Encrypt` indirect. The parameters are the same. A source object that held `/Encrypt` and is still written (no garbage collection, or its number reused) is encrypted like any other object.
- When the source has no `/ID`, an encrypted output gets two equal new elements (ISO 32000-2 14.4: a file written for the first time); MuPDF writes two different ones. Both keep `/ID[0]` when present and write a new `/ID[1]`.
- An empty owner password means the user password (Algorithm 9). MuPDF writes an owner entry no password opens as owner; engine.ts already replaces an empty owner password with the user password.
- Signature `/Contents` is neither encrypted nor decrypted in a dictionary of `/Type /Sig` or `/DocTimeStamp` (or untyped) with a `/ByteRange` array and a `/Contents` string (ISO 32000-2 7.6.2). MuPDF exempts only `/Type /Sig` with `/ByteRange`, so it misreads a time stamp's or an untyped signature's `/Contents` in either direction.
- With `/EncryptMetadata false`, `/Type /Metadata` streams are neither decrypted nor encrypted (ISO 32000, as qpdf and Acrobat do). MuPDF 1.28.1 and pypdf decrypt them anyway and read them as empty. `Keep` preserves the stream bytes either way; MuPDF's `encrypt=keep` damages such a stream.
- Objects with `/Type /XRef` are never decrypted or encrypted. The reader used to decrypt dormant cross-reference streams.
- AES-128 (AESV2) uses a 128-bit key whatever `/Length` says, as MuPDF does; the reader used to refuse the password. AESV3 crypt filters below revision 5 are refused; with a short key they could panic.
- R2-R4 passwords are converted from UTF-8 to PDFDocEncoding as MuPDF does (measured on 298 characters: `reference/app-encrypt-20261014/docenc-passwords.json`). Unlike MuPDF, the bytes as given are tried next, so files from writers that hashed UTF-8 also open, and a password that converts to nothing is not tried as the empty password.
- R5/R6 passwords are not SASLprep-normalized, as in MuPDF; a non-ASCII password that SASLprep would change may not open in Acrobat.
- `algorithmOf` (rules above) differs from engine.ts, which reads only V, R, `/Length` and `/CF/StdCF/CFM`: V4 with RC4 crypt filters is `rc4-128` (engine.ts: `aes-128`); V4 with Identity filters or an unknown `/CFM` is `unknown` (engine.ts: `aes-128`); V2 R3 without `/Length` is `rc4-40`, the 40-bit default MuPDF also uses (engine.ts reads the missing `/Length` as 0: `rc4-128`); an AESV3 filter not named `StdCF` is `aes-256`.
- After an AES-256 save of a plain document, the session document stays plain until it is reopened: `DocumentInfo.encrypted` stays false and a later `security-set` is allowed. engine.ts refuses it with the owner-password error, because MuPDF wrote `/Encrypt` into the live trailer; the person who set the security can change it again.
- Page count comes from walking /Kids as hayro does (a page listed twice counts once; without a usable tree, every dictionary with /Contents), not from MuPDF's trust of /Pages /Count, so it always matches what the view can draw.
- `countSignatures` and the attachment count visit shared or cyclic nodes once; MuPDF's recursion overflows on a cycle and reports 0.
- `validate` checks `encrypted` against the pending setting (`aes-256` means encrypted) instead of the /Encrypt that MuPDF's `saveToBuffer` writes into the live trailer. Without that side effect a later `security-set` after an AES-256 save of a plain document is allowed; MuPDF refuses it as if the session were not the owner.
- `validate` reopens strictly with the Rust parser: any structural problem is "the saved copy needed repair", where MuPDF either repairs (same message) or fails with its own text.
- `saved` accepts the optional `ticket` from the `save` reply and marks the state that save serialized (unreachable once its branch is cut), so a late acknowledgement cannot mark newer edits clean; the security change counts as saved only if no `security-set` came after. A malformed or foreign ticket is refused (`invalid`); a request without one keeps engine.ts semantics.
- `open` of an unreadable file says "open: the file could not be read (detail)" instead of MuPDF's message.
- `authenticate` checks the password by reopening the source with it (once per encoding); engine.ts authenticates the locked copy and then reopens.
- An encrypted document whose object streams hold the catalog opens locked with 0 pages, as MuPDF does, and gets its pages from `authenticate`.
- `keep` after an acknowledged `none` or `aes-256` save keeps what that save wrote, decided at `saved`; MuPDF decides at `saveToBuffer`, whose /Encrypt side effect on the live trailer happens even when the write never lands. `security-set`'s owner check and `detect` still see the source's encryption.
- A password that fails in MuPDF's encoding is tried in the other (UTF-8 bytes for R2-R4, PDFDocEncoding for R5/R6), so files from producers that encode differently open too.
- More than 65,536 xref sections, or 2 million xref entries, open through the repair rebuild (`repaired`, no versions); MuPDF opens them as they are. A file with over 2 million live objects is refused; MuPDF takes 8,388,607.
- An encrypted file whose trailer and /ID are lost (truncated or stripped) is refused ("invalid encryption without the file identifier"); MuPDF's repair does not find /Encrypt and opens the encrypted bytes as plaintext.
- A non-batch `mutate` inside an open operation joins it, as MuPDF nests operations; engine.ts never nests.
- The test harness's `session.doc` is opened from the render bytes, so after an edit it shows one more version (`countVersions`) and no unsaved changes, unlike MuPDF's live document.
- The adapter sends a request buffer whose contents cannot be read at its `byteLength` (a Proxy in tests) as that many zero bytes, so size checks see the length engine.ts sees.
- `layers` lists groups in MuPDF 1.28's order, descending object number (engine.ts's comment says the reverse of /OCGs; measured otherwise), but each referenced group once: MuPDF also lists duplicate entries and entries that are not group references (names, strings, missing objects) as rows whose toggle does nothing.
- An /OCProperties without /D lists its groups as visible and togglable; MuPDF reports "No default Layer config" and lists none.
- Optional-content visibility comes from /D and the layers panel only. MuPDF also hides a group whose /Usage /View /ViewState is /OFF whatever the panel says (it reports the layer visible), and every group whose /Intent the configuration's /Intent does not include.
- An optional-content membership dictionary (OCMD) follows its /P policy (AllOn, AnyOn, AnyOff, AllOff), in page content (hayro) and on annotations alike; MuPDF 1.28 treats every OCMD as AnyOn, indirect or direct.
- `search` has no per-page limit of 500 quads, which made MuPDF silently under-count a dense page; `truncated` still means a further match exists.
- `search` counts one hit per match, its quads one per run of adjacent characters; MuPDF splits a later match spanning several quads into one hit per quad (p3/edit-subset2.pdf, "日本語": MuPDF 3 hits, Rust 2), which changes `total` and how fast `maxHits` fills.
- A marked-content ActualText replaces the text of its span as one run, so render `text` and `lines` keep the replaced word whole: p3/edit-replace.pdf reads "Alpha" (6 lines) where MuPDF splits it as "Alph\na" (7 lines), and p3/edit-tail.pdf "Gamma" where MuPDF has "Gamm\na".
- A page whose text operators show more than 1 MiB of string bytes, or whose structured text would hold more than 1,048,576 characters, is refused as too complex (`render` fails; `search` and `text-all` read it as empty); MuPDF reads it however long it takes (a 14-million-glyph page was still running after 10 minutes).
- A page with an empty or missing MediaBox draws all of Letter. MuPDF clips such a page to its CropBox when the CropBox does not reach the origin (MediaBox [0 0 0 0] with CropBox [100 100 400 400] draws a 300×300 square), and draws all of Letter otherwise.
- `search` and `text-all` treat a page refused as too complex (self-invoking or exploding content) as having no text instead of failing; `render` of that page fails as before.
- `paintMs` is the whole render dispatch, text extraction included (the adapter's measure); engine.ts stopped it after painting.
