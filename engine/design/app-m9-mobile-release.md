# M9: mobile on the Rust engine, and the release engine switch (prepared)

October 17, 2026. Branch `app-p9-mobile-release` (base 46f1e95). This is a prepared change for the owner's review
(acceptance report 6.3 and 6.4, completion plan M9). It is not merged into the release path. Nothing is pushed,
no GitHub Action ran, and nothing is published. It does not touch `engine/src` or `src/renderer`.

## 1. Mobile

### What changed

- `mobile/package.json` builds the desktop app with `npm run build`, which is the Rust build.
  `mobile/scripts/build.mjs` refuses a MuPDF build.
- `build.mjs` copies from `build/renderer` only what `src/policy.ts` serves on a Rust build, excluding the
  desktop page, its script, its styles and its fixtures. That is `worker.js`, `worker-engine.js`,
  `engine-rust.js`, `engine-core.js`, `engine-shared.js`, `limits.js`, `vendor/tandem_rust_workflow{.js,_bg.wasm}`
  and `fonts/DroidSansFallback.ttf`.
- The build ships the Rust notices: `Engine-NOTICE.txt`, `Engine-AGPL-3.0.txt`, `engine-licenses/` and the CJK
  font's Apache-2.0 notice. It no longer ships `MuPDF-LICENSE.txt` or `docs/third-party`.
- The build fails if any file in `dist/` is named after MuPDF. It also fails if any script or page imports,
  links or `new URL`s one. The footer link "MuPDF license" became "PDF engine license" and "Engine notices".
- `scripts/sample.mjs` makes `welcome.pdf` (a hand-written page, with its field added by the Rust `field-add`)
  and the sample scan with the Rust engine. The sample scan is a Rust `render` at 2x. It is now `sample-scan.png`:
  the scanner re-encodes every input through a canvas, so a PNG works the same.
- `tests/workflow.test.mjs` runs on the Rust engine (`tests/rust.mjs`).
- The UI harnesses write their screenshots under `os.tmpdir()` instead of `/private/tmp`.

### The scanner

`scan-worker.ts` no longer imports `vendor/mupdf.js`.

- `scan-pdf.ts` composes the PDF itself. `scan-font.ts` subsets TrueType fonts. `scan-metrics.ts` holds the
  Helvetica widths.
- The worker then checks the result with the Rust engine the app ships: `open` (the expected page count), then
  `validate`, which is the strict save check and refuses a file that needs repair.
- This replaces MuPDF's reopen check: `!wasRepaired()`, the page count, and loading every page.
- The glue, the adapter and `engine-core.js` load from `engine/`, so the app ships one copy of each.

Why not `pdf-from-images` directly:

- Its page size comes from the image resolution (72 dpi when none is given), not the scanner's 792 pt long side.
  A canvas JPEG would make a 1224 x 1584 pt page instead of 612 x 792.
- The Rust engine cannot write the scanner's text. `text-add` is WinAnsi only, has no invisible mode and has no
  fallback font. The searchable mode and the rebuilt mode need all three.
- A Rust op that does this (`fit`, OCR lines, CJK embedding) is a possible later step: an owner decision, outside
  `engine/src` tonight.

### What the output keeps and what changed

| | MuPDF scanner | Now |
| --- | --- | --- |
| Page size | long side 792 pt, scan aspect; A4 for rebuilt pages | same (asserted: 612 x 792 for 1224 x 1584 px) |
| Image | JPEG stored unchanged (DCTDecode) | same, byte-identical (asserted); one stream per distinct image, as before |
| Colour space | `ICCBased`, MuPDF's own sRGB profile | `DeviceRGB`; a profile embedded in the JPEG is kept as `ICCBased` (asserted) |
| OCR text | invisible, fitted to the line width | same (Tr 3, horizontal scale; asserted x and width) |
| Latin text fonts | Nimbus Sans, embedded (CFF) | Helvetica/Helvetica-Bold, not embedded, with `/Widths` |
| Other text | Droid Sans Fallback embedded whole (3.5 MB font) | a subset of Droid Sans Fallback (Type0, Identity-H, ToUnicode) |
| Metadata | `/Producer (MuPDF 1.28.1)` (written inside the catalog) | trailer `/Info << /Producer (Tandem PDF) >>` |
| Multi-page, limits | 1-12 scans, at most 100 output pages, 50 MB | same messages and limits |

Sizes on the same inputs (a 30,073-byte JPEG; text "Hello scan résumé 中文"; the test's 6-page rebuilt document):

| Output | MuPDF | Now, compressed | Now, uncompressed |
| --- | --- | --- | --- |
| image, 2 pages of one JPEG | 33,702 | 33,240 | 33,194 |
| searchable, CJK line | 1,769,007 | 34,637 | 34,845 |
| rebuilt, 6 pages, CJK | 1,772,903 | 7,661 | 31,166 |
| rebuilt, Latin only | 66,767 | 2,765 | 2,788 |
| rebuilt, 4 pages of dense CJK, kana and Hangul (60 paragraphs) | not measured | 11,552 | 38,851 |

Composing takes 0-41 ms and the Rust check 0-9 ms (Node, M-series). WASM linear memory starts at 4.2 MB. It is
4.4 MB after checking a 12-page searchable scan (one image, deduplicated), and 14.4 MB after `fixtures/large.pdf`
(500 pages) with five 2x renders.

Intentional differences, with the reasons:

- **Base-14 Helvetica is not embedded.** Every viewer has it or a metric-compatible substitute, and the widths
  are in the file. Files are 25-60 times smaller. It is not PDF/A.
- **DeviceRGB.** Shipping MuPDF's sRGB profile would bring back an Artifex asset (gotchas: "Artifex ICC profile
  coverage"). Canvas JPEGs are sRGB, and viewers treat DeviceRGB as sRGB.
- **Control characters.** Tabs become spaces, and other control characters are dropped. MuPDF drew .notdef.
- **Regression: scripts the fallback font lacks.** Droid Sans Fallback has no glyphs for Greek, Cyrillic, Thai,
  emoji and others. In a rebuilt (AI) document, those characters now draw as the font's missing-glyph box. Their
  text still extracts and searches: each code maps to its Unicode (asserted for `𝔘 ✓ ภาษา`).
  - MuPDF drew Greek and Cyrillic with its embedded Nimbus Sans (checked by rendering).
  - Searchable scans are not affected, because their text is invisible.
  - The fix needs a TrueType font with Latin, Greek and Cyrillic glyphs (for example an OFL font, subset the same
    way). That is an owner decision (new asset, licence).
- **Compression.** Streams are compressed with `CompressionStream` where it exists (Safari 16.4+, Chrome 80+).
  On Safari 15.0-16.3 they are written uncompressed: valid, a little larger, and tested.

### Verified (all through `tools/slot`)

- `tsc --noEmit` passes for mobile, and the mobile build passes, with its MuPDF check.
- `node --test tests/*.test.mjs` passes 15 of 15.
- The scanner tests read every output with MuPDF as an independent oracle and with the Rust engine (`text-all`,
  `search`, `render`). They keep the old assertions and add:
  - page size, the untouched JPEG, ICC passthrough, image dedupe and Producer;
  - the Rust check failing on a wrong page count and on a truncated file;
  - the font read only when needed, the subset size and characters missing from the font;
  - the uncompressed path, and a 100-page overflow.
  - A regression test for a bug found here: subsetting from a Node `Buffer` edited the caller's font bytes,
    because `Buffer.slice` is a view. Fixed by copying.
- `electron tests/browser.cjs`, `scanning-browser.cjs` and `chatgpt-browser.cjs` all pass against the preview
  server (Chromium 152): the real engine worker, the scan worker with the WASM in a module worker, the image
  PDF and the typeset PDF.
- A CJK rebuilt page renders the same in MuPDF and Rust: CJK, Hangul and accented Latin draw correctly.

### iOS and Android: what holds and what needs a device

- **The WASM's features.** It uses no SIMD, exceptions, tail calls or memory64. Its `target_features` section
  lists only bulk-memory, multivalue, mutable-globals, nontrapping-fptoint, reference-types, sign-ext and
  call-indirect-overlong, all in Safari 15.0.
- **Size.** 13.4 MB (MuPDF's was 10.4 MB), of which 1.7 MB is the `name` debug section, which could be stripped.
  Initial memory is 4.2 MB, with no maximum.
- **Glue and APIs.** The glue needs `FinalizationRegistry` (Safari 14.1) and `TextDecoder` with `fatal`. The
  composer uses `Intl.Segmenter` (14.1) and `CompressionStream` (16.4, optional). Capacitor serves `.wasm` as
  `application/wasm` on both platforms, so streaming compile works.
- **Workers.** The engine worker uses a top-level await in a module worker (Safari 15). The scan worker's is
  inside its message handler.
- **iOS floor (pre-existing, not changed).** The mobile UI uses `<dialog>` and `Array.prototype.at`, which need
  Safari 15.4. The Xcode target says iOS 15.0.
- **Pending on a device** (not run: no Xcode, simulator or gradle, per the brief):
  - the WASM compiles and instantiates in WKWebView on iOS 15.x and current iOS;
  - `'wasm-unsafe-eval'` CSP handling there;
  - the engine worker's synchronous XHR for the CJK font over `capacitor://`;
  - the scan worker's `fetch` of the font;
  - memory with two engine instances (editor and scan worker) under the WebView limit (about 1-2 GB);
  - Android System WebView on API 24 devices needs Chrome 89+ for top-level await in module workers;
  - the four iPhone simulator workflows and the Android build named in `mobile/README.md`.

## 2. Release and CI

- `npm run package` builds Rust (`package:rust` is now an alias). `scripts/package.mjs` refuses a MuPDF build and
  writes only the Rust notices. No MuPDF fallback is kept: the plan makes MuPDF a test-only oracle, and
  `build:mupdf` and `test:mupdf` remain for tests. Reverting commit 31b7cad restores MuPDF packaging.
- `release.yml`:
  - A new `engine` job (ubuntu-24.04) does the following:
    - `rustup toolchain install 1.98.1 1.92 --target wasm32-unknown-unknown`;
    - `cargo install wasm-bindgen-cli --version 0.2.129 --locked`;
    - `cargo +1.92 check` of the bridge library for wasm32 (the MSRV);
    - `engine/tools/build-workflow.sh` on 1.98.1;
    - uploads `engine/web/pkg`, `sources.sha256` included.
  - The three installer builds download it, so all ship the same WASM. Their `scripts/build.mjs` re-checks
    `sources.sha256` against the same commit (`.gitattributes` `* -text` keeps Windows bytes identical).
  - Each platform runs its packaged smoke, then `scripts/check-package.mjs` against the installed resources:
    the DMG's app on macOS, `%LOCALAPPDATA%\Programs\Tandem PDF\resources` on Windows, and
    `/opt/tandem-pdf/resources` on Linux.
  - `check-package.mjs` requires `engine: rust` in both the report and the worker round trip. It also requires
    no MuPDF file or loader in `app.asar`, the engine's notices, and a `NOTICE.txt` with no IJG sentence.
  - `linux-check` (Ubuntu 22.04 and 26.04) also requires `engine: rust`.
  - The MuPDF source archive step is removed.
- **Validated locally.**
  - `release.yml` parses with Ruby's Psych: 4 jobs, `build` needs `engine`, and the env pins are strings.
  - `npm run package` ran on darwin-arm64 (peak 576 MB). The packaged `--smoke` gives ok, packaged, sandbox, and
    the engine and round trip both `rust`. The protocol requests are the Rust ones only.
  - `check-package.mjs` passes (230 files in `app.asar`). A report naming MuPDF fails it.
- **Not verified.**
  - Any GitHub run: the toolchain install, the `cargo install` time, and artifact hand-off.
  - The Windows and Linux packaged smokes and `check-package.mjs` there (the pwsh step calls node after the
    installer smoke).
  - Signing and notarization with the Rust WASM.

## 3. Licence wording for owner sign-off (unchanged from app-worker.md; not invented here)

`NOTICE.txt` of a Rust build (scripts/package.mjs), lines 2-4:

> Tandem PDF's own code is MIT (PDF4All-MIT.txt). This build bundles the Tandem PDF Rust engine,
> AGPL-3.0-or-later and informed by MuPDF (Engine-NOTICE.txt, Engine-AGPL-3.0.txt; its dependencies' licence
> files are in engine-licenses), so the build as a whole is distributed under the AGPL.
>
> Corresponding source: the app's source at this version's tag, the engine in its engine folder, at
> https://github.com/Shiro-Legacy/TPDF/releases/tag/v<version> (build steps: README.md and
> engine/tools/build-workflow.sh).
>
> The CJK fallback font Droid Sans Fallback (fonts/DroidSansFallback.ttf in the app), Copyright (c) 2005-2008
> The Android Open Source Project, is Apache-2.0 (third-party/droid-sans-fallback-NOTICE.txt).

About panel (src/main.ts, Rust build):

- copyright: "Copyright © 2026 Shiro-Legacy. MIT licence; the bundled Rust PDF engine is AGPL-3.0-or-later."
- credits: "The PDF engine is informed by MuPDF, Copyright © Artifex Software, Inc.; see its NOTICE."

The Rust build drops "This software is based in part on the work of the Independent JPEG Group." There is no
libjpeg; JPEG is decoded by zune-jpeg (engine/NOTICE).

New wording in this branch, also for sign-off:

- the root README licence paragraph;
- the mobile footer links ("PDF engine license", "Engine notices");
- the mobile README licence line.

The mobile app ships no `NOTICE.txt` or source statement, as before with MuPDF.

## 4. docs/gotchas.md "Still undecided": what changes when MuPDF stops shipping

- **Gone with MuPDF:**
  - how to fulfil the MuPDF source obligation (a 66 MB tarball or a written offer);
  - never put that tarball under `docs/third-party/`;
  - "a mirror we control" (for that archive);
  - the Artifex ICC profile coverage inference;
  - the WASM's embedded URW CFF fonts, 4 Artifex ICC profiles and AGPL jbig2dec;
  - the cmark-gfm note in `docs/third-party/README.md`;
  - the IJG sentence release blocker, which is resolved for Rust builds (NOTICE and About carry none).
- **Replaced:**
  - The OFL notice for URW data now ships as `engine-licenses/urw-metrics` and `urw-outlines`.
  - Droid Sans Fallback ships with its Apache-2.0 notice.
  - The engine's corresponding source is the tag. Whether to attach a `cargo vendor` archive of its crates is an
    open owner and legal question.
- **Unchanged** (Electron and process): the release cut, the signing and notarization budget, the distribution
  channel, online providers, LGPL and Chromium notices inside the .app, the 42 LGPL-mentioning entries, the
  ffmpeg variant, Windows `d3dcompiler_47.dll` and dxcompiler/dxil, and the `package-lock.json`
  `"UNLICENSED"` root entry.

## 5. Corrections to other notes (not edited here)

- These statements in `engine/design/app-worker.md` are superseded on this branch:
  - "`npm run package` (what `release.yml` runs) builds MuPDF";
  - "`npm run test:mupdf` is the pre-tag suite";
  - "Mobile stays on MuPDF".
- `app-worker.md` is left alone because another lane edits it. The coordinator updates the acceptance report and
  the plan (6.3, 6.4, M9 items).
- `mupdf` can now move to `devDependencies` (only tests use it). That needs a `package-lock.json` refresh, so it
  is left for a follow-up.

## 6. Gates

The full gates (`tools/gates.sh`, under slot, logs in `tpdf-wt/gates-out/p9-mobile/`) ran on 2c607d9 and all
pass:
- fmt and clippy, engine and bridge, on stable and 1.92;
- native tests: 1,618 passed, 0 failed on stable and on 1.92;
- bridge tests;
- the WASM package and `npm run build`;
- the app suite without Electron: 681 tests, 568 pass, 0 fail, 113 skip;
- `tests/run-rust.mjs`: 670 tests, 567 pass, 0 fail, 103 skip.

`engine/web/pkg` is not tracked. Mobile, after the last change:
- `tsc` and the build pass;
- `node --test`: 15 of 15;
- the three Electron UI suites pass.
