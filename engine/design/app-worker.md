# App worker on the Rust engine (M9)

> Licence note (October 18, 2026): the Rust engine and the app are now MIT, and no MuPDF code ships. Where this document says engine code is AGPL or MuPDF-derived, or that a build owes AGPL obligations, it records the state before the clean-room relicense (see the Licence section of [`PROVENANCE.md`](../reference/PROVENANCE.md)).

October 15, 2026. The Electron app's worker runs the Rust engine (`engine/web/pkg` through `src/renderer/engine-rust.ts`); MuPDF remains a build option and the unit tests' oracle.

## Build

- `scripts/build.mjs` picks the worker engine from `--engine=rust|mupdf`, else `TPDF_ENGINE`, else `rust`; any other value fails the build. `npm run build` builds Rust and `npm run build:mupdf` builds MuPDF. The script empties `build/renderer/vendor` first, so one build never carries the other engine's assets.
- `rust`: copies `tandem_rust_workflow.js` and `tandem_rust_workflow_bg.wasm` from `engine/web/pkg` (built by `engine/tools/build-workflow.sh`). A missing package, or one without `RustApp`, fails the build and names the command to run.
- Stale package: `build-workflow.sh` stores `web/pkg/sources.sha256`, the `<sha256>  <path>` listing of `Cargo.toml`, `bridge/Cargo.toml`, `bridge/Cargo.lock`, `src/` and `bridge/src/` (`engine/tools/workflow-sources.sh`, read before the cargo build, written after wasm-bindgen). build.mjs recomputes it and refuses a package with no listing or with changed, added or removed sources, naming up to five, before it touches `build/`. `TPDF_ALLOW_STALE_ENGINE=1` turns the refusal into a warning. The toolchain and wasm-bindgen versions are not part of the listing.
- So the measurement order is `build-workflow.sh` first, then `npm run build`; the reverse fails after a Rust change instead of copying the old WASM into `build/renderer/vendor`.
- `mupdf`: copies the three `mupdf/dist` files.
- Loader: tsc emits `worker-engine.js` empty (`worker-engine.ts` only declares `WorkerEngine` and `loadEngine`), plus `worker-engine-rust.js` and `worker-engine-mupdf.js`. build.mjs copies the selected one over `worker-engine.js` and deletes both, so a rerun without tsc fails ("run tsc") instead of labelling a build with one engine while its worker loads the other. `tsc -w` after a build re-emits the empty module, and the worker then fails to load at once.
- `build/engine-build.js` exports `ENGINE`. build.mjs writes it over tsc's default (`src/engine-build.ts`, `"rust"`). `src/policy.ts` serves only that engine's assets: for Rust `engine-rust.js` and the two package files; for MuPDF the three vendor files and the TypeScript engine over them (`engine.js`, `content-edit.js`, `redact.js`, `sign-prepare.js`). Both serve `worker-engine.js`, `engine-core.js`, `engine-shared.js` and `limits.js`.
- The renderer imports its values from `engine-shared.js` (bounds, fixed sizes, `createGate`, `createLatestGuard`, `createRenderCache`, `visibleRange`, `parsePageRange`, `normaliseLine`, moved unchanged out of `engine.ts`, which re-exports them), `engine-core.js` (`allowed`, `permissionNames`) and `limits.js`; from `engine.js` only types. A Rust app therefore neither requests nor serves the MuPDF engine modules. They stay in `build/renderer` as the unit tests' oracle (`build/renderer/engine.js`), unserved, and a Rust package leaves them out.
- No runtime fallback. `worker.ts` statically imports `./worker-engine.js`. A failed load (WASM missing or invalid) is passed to `reportError`, which raises the worker's error event; a rejected top-level await raises none. The renderer then restarts the worker at once, at most 3 times, and then asks the user to reopen the app. Measured on an app copy without the `.wasm`: the smoke fails after 1 s, where it used to take 20 s.
- The WASM is fetched by wasm-bindgen's `init` from `app://pdf4all/vendor/tandem_rust_workflow_bg.wasm` (served as `application/wasm`, so `instantiateStreaming` works). The CSP is unchanged: `'wasm-unsafe-eval'` and `connect-src 'self'` were already there. Rust's WASM is 11.8 MB, MuPDF's 10.4 MB, and both compile in about 20 ms in Node.
- Every `npm run build` (and so `npm test` and `npm start`) now needs an up-to-date `engine/web/pkg`. A machine without the Rust toolchain uses `npm run build:mupdf` and `npm run test:mupdf`.

## Packaging and the release gate

- `scripts/package.mjs` packages the engine `build/engine-build.js` names. `npm run package` (what `release.yml` runs) builds MuPDF; `npm run package:rust` builds Rust. Which engine releases ship is the owner's decision; nothing here changes it.
- Of `build/renderer` the package stages only what `policy.ts` serves, for either engine. A Rust package therefore holds no MuPDF engine module, no `vendor/mupdf*` and no file named after MuPDF. A MuPDF package loses only the unserved `engine-rust.js`.
- Rust notices: `licenses/Engine-NOTICE.txt` (`engine/NOTICE`), `Engine-AGPL-3.0.txt` (`engine/COPYING`), `engine-licenses/` (`engine/reference/licenses`, which also holds the test-only `freetype-native` and the optional `tiny-skia` sets; engine/NOTICE says which apply) and `third-party/lgpl-relink.md` (Electron's ffmpeg; the rest of `docs/third-party` describes the MuPDF WASM). `NOTICE.txt` names the Rust engine as AGPL-3.0-or-later and MuPDF-informed and points to the app's source at the tag (engine/ included) as corresponding source.
- Difference: a Rust build drops the IJG sentence from `NOTICE.txt` and the About panel. It is owed for MuPDF's libjpeg; the Rust engine decodes JPEG with zune-jpeg (engine/NOTICE). The About panel names the bundled engine and its AGPL licence from `ENGINE`, with "informed by MuPDF, Copyright Artifex Software, Inc." as the Rust credit. The wording of both is for the owner to confirm before a Rust release.
- Measured on darwin-arm64: `npm run package:rust`, then the packaged `--smoke`: ok, `packaged: true`, `sandbox: true`, `engine: "rust"`, round trip read back. app.asar lists no MuPDF file and no file imports one; the protocol requests are the Rust ones only. `npm run package` (MuPDF) and its packaged smoke pass as before.
- Release gate: `npm test` builds the Rust engine, but releases ship MuPDF, so `npm run test:mupdf` is the pre-tag suite (README and the `release.yml` header say so).
- `mupdf` stays in `dependencies`: `build:mupdf`, mobile and the unit tests' oracle use it. No package needs it at run time (packages carry `build/`, not `node_modules`); moving it to `devDependencies` can follow once mobile no longer needs it.
- Mobile stays on MuPDF: its scan worker imports the desktop build's `vendor/mupdf.js`, and its engine worker is the desktop `worker.js` as built. `mobile/package.json` now builds the desktop app with `build:mupdf`, and `mobile/scripts/build.mjs` refuses a Rust desktop build. Before this, a Rust desktop build would have broken the scan worker only at run time. The worker now uses a top-level await, which needs Safari 15 or later in the iOS WebView; this is not verified on a device.

## Worker

- `loadEngine()` (`worker-engine-rust.ts` or `worker-engine-mupdf.ts`, installed as `worker-engine.js`) returns `{name, createSession, dispatch}`. The worker initialises the WASM and creates its session before it posts `{ready: true}`. The request/reply contract and the handshake are unchanged.
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
- Startup (a renderer defect on both engines): a worker that failed before it was ready left its 20 s startup timer armed, so the timer later killed the healthy restarted worker, and requests waiting on `ready` hung for those 20 s. `workerFailed` now ends that wait (`failReady`). The Electron test for it fails without the fix: 20 s after the failed load, "worker startup timed out" kills the third worker and its document.
- What a trap loses matches what a dead or hung MuPDF worker loses: the open document and its unsaved edits. Difference: MuPDF turns a failed allocation into a catchable error and keeps the document, while Rust on wasm32 aborts on allocation failure. An op that exhausted WASM memory would therefore cost the session where engine.ts refuses the op. Op code bounds work and allocation so that real input does not reach this. The integration acceptance review found small documents that did (recursion in colour spaces and functions, memory from nested `q`, nested Flate, long paths and dash patterns); `app-accept-integration.md` bounds them, and `tests/rust-engine-trap.test.mjs` drives a real trap through the adapter.
- Timeouts are the same for both engines. The 20 s request watchdog terminates and restarts the worker. Both engines run synchronously in the worker, so a hung op behaves the same on either. Superseded in part by M9 (`app-m9-recovery.md`, October 17):
  - after any restart, the open document is now restored from the renderer's recovery log, without the request that hung or trapped;
  - the long jobs (120 s) now also restart the worker when the document can be restored, and otherwise reject with `reason: "worker"` as before;
  - thumbnails (10 s) still only reject;
  - the "open the document again" message remains for a document that cannot be restored.

## Tests

- `tests/p0.test.mjs`: the asserted vendor WASM, and whether `engine.js` and `content-edit.js` are served, now follow the build (`engine-shared.js` is asserted served). It pinned MuPDF assets, and the allowlist now lists only the shipped engine's.
- `tests/worker-build.test.mjs` (new):
  - Every module the renderer and worker load (static and dynamic imports and `new URL(..., import.meta.url)`, followed transitively) is built and on the allowlist.
  - `vendor/` holds exactly the engine's files, and only those are served.
  - `engine.js`, `content-edit.js`, `redact.js` and `sign-prepare.js` are loaded and served exactly when the build is MuPDF; no `worker-engine-rust.js` or `worker-engine-mupdf.js` is left in `build/renderer`.
  - A Rust build has no file named after MuPDF and no import or URL that names it.
  - build.mjs in a scratch tree with a stand-in engine: installs each loader and leaves neither variant, fails a rerun without tsc, refuses a changed source (naming it) before `build/` changes, warns under `TPDF_ALLOW_STALE_ENGINE=1`, and refuses a package without `sources.sha256`. The listing comes from `workflow-sources.sh` itself, so the shell and Node listings must agree. Skipped on Windows (sh).
  - Electron, Rust: on an app copy whose `RustApp` throws on `search`, a find leads to the restart message and `doc: null`, and the next open works.
  - Electron, both engines: a copy whose loader crashes the first worker on `search` and fails the second worker's load (an IndexedDB load count); the second restart message names the load failure, the next open works, and 21 s later the document is still open with no error.
- Proof that MuPDF is absent at runtime: the Rust source smoke and the packaged Rust smoke request `worker.js`, `worker-engine.js`, `engine-rust.js`, the two package files and the shared modules, and no MuPDF engine module or asset.

## Decision for the orchestrator: merge order

- The default build is Rust, as the task asked, while the redaction ops (`redact-add`, `redact-set`, `redact-search`, `redact-preview`, `redact-apply`) are not ported. Merged on its own, this branch makes `npm test` fail the 5 redaction drives and every Redact-mode action in a dev build answer "unknown op". So it should merge after, or together with, the redaction piece. If it must merge first, the stopgap is `'mupdf'` as the last fallback in `scripts/build.mjs` (one line; `src/engine-build.ts` only types the emitted default, which build.mjs always overwrites) until redaction lands. Releases are unaffected either way: `npm run package` builds MuPDF.

## Measured (October 15, after the review fixes)

- Full suite with Electron (`node --test tests/*.test.mjs`), Rust build: 651 pass, 6 fail, 3 skipped. The failures are the 5 redaction drives (p3-drive P4, p6-drive3, p6-drive4 U4, p6-drive8 ×2), whose failing steps answer `unknown op: redact-add` (31), `redact-preview` (14), `redact-search` (6) or `redact-apply` (3) or follow from them, and the p3-edit "/W expansion" test, a MuPDF engine.ts timing check under load that passes alone. worker-build 5/0/0, p0 2/0/0.
- The same, MuPDF build: 655 pass, 0 fail, 5 skipped (the two Rust-only worker-build tests, two Rust smoke tests and one p5-sign-crypto test skip). worker-build 3/0/2, p0 2/0/0. Before the fixes, the reviewers measured Rust 648-650 pass with the same 5 redaction drives failing, and MuPDF 651/2/5 with two timing budgets.
- `CI_NO_ELECTRON=1` default suite, Rust build: 556 pass, 0 fail, 104 skipped (before: 555/0/103; the build-script test is new, and the startup test is skipped there).
- `node tests/run-rust.mjs`: 525 pass, 24 fail, 102 skipped. The same 23 failures as before (11 p4-redact, 8 p6-objects7, one each in p2-assemble, p2-edit, p3-edit and p6-paragraph) plus the timing-sensitive p3-edit "/W expansion" test, which passes when p3-edit runs alone (31/1, the one failure being the known p3-edit one).
- No drive failure points to the adapter, the worker or an op other than redaction.
