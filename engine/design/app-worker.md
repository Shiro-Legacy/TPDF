# App worker on the Rust engine (M9)

October 15, 2026. The Electron app's worker runs the Rust engine (`engine/web/pkg` through `src/renderer/engine-rust.ts`); MuPDF remains a build option and the unit tests' oracle.

## Build

- `scripts/build.mjs` picks the worker engine from `--engine=rust|mupdf`, else `TPDF_ENGINE`, else `rust`; any other value fails the build. `npm run build` builds Rust and `npm run build:mupdf` builds MuPDF. The script empties `build/renderer/vendor` first, so one build never carries the other engine's assets.
- `rust`: copies `tandem_rust_workflow.js` and `tandem_rust_workflow_bg.wasm` from `engine/web/pkg` (built by `engine/tools/build-workflow.sh`). A missing package, or one without `RustApp`, fails the build and names the command to run. It deletes the emitted `worker-engine-mupdf.js`, so no file under `build/` imports MuPDF.
- `mupdf`: copies the three `mupdf/dist` files and installs `worker-engine-mupdf.js` as `worker-engine.js`.
- `build/engine-build.js` exports `ENGINE`. build.mjs writes it over tsc's default (`src/engine-build.ts`, `"rust"`). `src/policy.ts` serves only that engine's assets: for Rust `engine-rust.js` and the two package files, for MuPDF the three vendor files. `worker-engine.js` is served for both.
- No runtime fallback. `worker.ts` statically imports `./worker-engine.js`. A failed load (WASM missing or invalid) is passed to `reportError`, which raises the worker's error event; a rejected top-level await raises none. The renderer then restarts the worker at once, at most 3 times, and then asks the user to reopen the app. Measured on an app copy without the `.wasm`: the smoke fails after 1 s, where it used to take 20 s.
- The WASM is fetched by wasm-bindgen's `init` from `app://pdf4all/vendor/tandem_rust_workflow_bg.wasm` (served as `application/wasm`, so `instantiateStreaming` works). The CSP is unchanged: `'wasm-unsafe-eval'` and `connect-src 'self'` were already there. Rust's WASM is 11.8 MB, MuPDF's 10.4 MB, and both compile in about 20 ms in Node.
- Every `npm run build` (and so `npm test` and `npm start`) now needs `engine/web/pkg`. A machine without the Rust toolchain uses `npm run build:mupdf`.
- Packaging stays on MuPDF: `npm run package` builds with `build:mupdf`, and `scripts/package.mjs` refuses any other build. Its notices describe MuPDF, and the Rust engine's `NOTICE` and dependency licences are not packaged yet. The release workflow is unchanged because it runs `npm run package`. Shipping the Rust engine needs a decision first.
- Mobile stays on MuPDF: its scan worker imports the desktop build's `vendor/mupdf.js`, and its engine worker is the desktop `worker.js` as built. `mobile/package.json` now builds the desktop app with `build:mupdf`, and `mobile/scripts/build.mjs` refuses a Rust desktop build. Before this, a Rust desktop build would have broken the scan worker only at run time. The worker now uses a top-level await, which needs Safari 15 or later in the iOS WebView; this is not verified on a device.
- The renderer still imports constants and helpers from `engine.js` (`createGate`, `createRenderCache`, `parsePageRange`, the `MAX_*` and `THUMB_*` constants...). A Rust build therefore serves `engine.js`, `content-edit.js`, `redact.js` and `sign-prepare.js` as dead code: nothing in them imports MuPDF, and `createEngine` is never called. The follow-up is to move those exports into their own module, re-exported by `engine.ts`, and drop the four files from the Rust allowlist. This was left out because it edits `engine.ts`, which other pieces also change.

## Worker

- `loadEngine()` (`worker-engine.ts` or `worker-engine-mupdf.ts`) returns `{name, createSession, dispatch}`. The worker initialises the WASM and creates its session before it posts `{ready: true}`. The request/reply contract and the handshake are unchanged.
- The P0 `roundtrip` now runs through engine ops in two sessions of its own: open `form.pdf`, `fields` (to find `name`), `annot-add` Square `[100,100,300,200]` with the note text, `field-set`, `save`, then `open` the saved copy and read back with `annots` and `fields`. The reply adds `engine`. main's smoke reports `engine` and requires `roundtrip.engine === ENGINE`.
- Differences from the old round trip, which called MuPDF directly: there is no red interior colour (`annot-add` uses the app's Square style), and the saved bytes are the app save rather than `saveToBuffer("incremental=no")`. The smoke reads back the same values on both engines.
- `probe` (CSP, no Node globals) is unchanged.

## Crash recovery

- A reply with `reason: "worker"` comes only from the Rust adapter: a `RustApp` that threw, as wasm-bindgen does after a trap. engine.ts never sends it. The renderer now treats such a reply like a dead worker and calls `workerFailed`:
  - pending requests are rejected;
  - the document, clipboard and compare slot are cleared;
  - the worker is terminated and restarted;
  - the user is told to open the document again, and the 3-restart cap applies.
- The failing request is rejected with its own reply first. The restart runs on the next task, so the caller's handlers (save, batch cancel) see the tagged refusal. Later `worker` replies from the same stopped worker change nothing.
- Before this change, a stopped engine left the document on screen while every later request failed until the app was restarted, a new `open` included ("the PDF engine stopped after an internal error"). The new Electron test fails without the change for exactly that reason.
- Startup (a renderer defect on both engines): a worker that failed before it was ready left its 20 s startup timer armed, so the timer later killed the healthy restarted worker, and requests waiting on `ready` hung for those 20 s. `workerFailed` now ends that wait (`failReady`).
- What a trap loses matches what a dead or hung MuPDF worker loses: the open document and its unsaved edits. Difference: MuPDF turns a failed allocation into a catchable error and keeps the document, while Rust on wasm32 aborts on allocation failure. An op that exhausted WASM memory would therefore cost the session where engine.ts refuses the op. Op code bounds work and allocation so that real input does not reach this.
- Timeouts are unchanged and the same for both engines. The 20 s request watchdog terminates and restarts the worker. Save, validate and batch (120 s) and thumbnails (10 s) reject with `reason: "worker"` without terminating it. Both engines run synchronously in the worker, so a hung op behaves the same on either. The M9 item "recovery for long operations" (unsaved edits are lost on termination) is not addressed here.

## Tests

- `tests/p0.test.mjs`: the asserted vendor WASM now follows the build (`tandem_rust_workflow_bg.wasm` or `mupdf-wasm.wasm`). It pinned a MuPDF asset name, and the allowlist now lists only the shipped engine's assets.
- `tests/worker-build.test.mjs` (new):
  - Every module the renderer and worker load (static and dynamic imports and `new URL(..., import.meta.url)`, followed transitively) is built and on the allowlist.
  - `vendor/` holds exactly the engine's files, and only those are served.
  - The Rust worker never loads `engine.js`, `content-edit.js`, `redact.js` or `sign-prepare.js`.
  - A Rust build has no file named after MuPDF and no import or URL that names it.
  - Electron: on an app copy whose `RustApp` throws on `search`, a find leads to the restart message and `doc: null`, and the next open works.
- Proof that MuPDF is absent at runtime: the Rust build's smoke passes from a copy of `build/` and `package.json` with no `node_modules`. Its protocol requests are `worker.js`, `worker-engine.js`, `engine-rust.js` and the two package files, and no MuPDF asset.

## Measured (October 15, M-series, `node --test tests/*.test.mjs` with Electron)

- Rust build: 649 pass, 6 fail, 3 skipped. Of the 100 Electron tests that ran, 95 pass. The 5 that fail are the redaction drives (p3-drive P4, p6-drive3, p6-drive4 U4, p6-drive8 ×2). Every failing step is `unknown op: redact-add|redact-search|redact-preview|redact-apply`, or a later step whose state differs because an earlier redaction step did nothing: in direct-edit3, for example, the two undos meant for the marks undo the image resize and the added text, so a later find of "Hello there" counts 0. The sixth failure is a MuPDF timing budget in a unit test under load (p2-pages page-move 819 ms); earlier runs failed the 2048-point Ink budget in the same way.
- MuPDF build: 653 pass, 0 fail, 5 skipped (the Rust-only build checks are skipped).
- No drive failure points to the adapter, the worker or an op other than redaction.
