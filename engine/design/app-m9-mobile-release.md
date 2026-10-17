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
| Other text | Droid Sans Fallback embedded whole (3.5 MB font) | a subset of Droid Sans Fallback (Type0, Identity-H, ToUnicode); its tag is six letters hashed from its characters |
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
- **Regression: everything outside WinAnsi that is not CJK or Hangul.** Text in WinAnsi uses Helvetica. Every
  other character goes to Droid Sans Fallback, and that font has almost nothing outside CJK, kana, Hangul and
  fullwidth forms. Measured by a test (`fallback font coverage per script`), glyphs drawn of the assigned
  characters outside WinAnsi:

  | Block | Drawn |
  | --- | --- |
  | Latin Extended-A | 0 of 121 |
  | Latin Extended-B | 0 of 207 |
  | IPA | 0 of 96 |
  | Combining marks | 0 of 112 |
  | Greek | 0 of 135 |
  | Cyrillic | 0 of 256 |
  | Hebrew | 0 of 88 |
  | Arabic | 0 of 256 |
  | Thai | 1 of 87 |
  | Vietnamese (Latin Extended Additional) | 0 of 256 |
  | General Punctuation | 4 of 96 |
  | Currency | 0 of 33 |
  | Letterlike | 0 of 79 |
  | Arrows | 0 of 112 |
  | Math Operators | 0 of 256 |
  | Box Drawing | 0 of 128 |
  | Misc Symbols | 0 of 256 |
  | Emoji | 0 of 1,895 |
  | CJK Symbols | 36 of 64 |
  | Hiragana | 90 of 93 |
  | Katakana | 91 of 96 |
  | CJK Unified (BMP) | 20,902 of 20,992 |
  | Hangul | 11,172 of 11,172 |
  | Fullwidth forms | 164 of 225 |

  The assigned totals follow Node's Unicode version; the drawn counts are pinned by the font's sha256.
  - So Polish ("Zażółć"), Czech ("Příliš"), Vietnamese, Greek, Cyrillic, and "≤ ≥ →" draw as missing-glyph
    boxes in AI-rebuilt pages. Their text still extracts and searches, because each code maps to its Unicode
    (tested).
  - MuPDF drew the Latin, Greek, Cyrillic, arrow, maths and currency characters with its embedded Nimbus Sans
    (checked by rendering).
  - Searchable scans are not affected: their text is invisible.
  - Separate combining accents are now composed first (NFC), so "Cafe" + U+0301 is "Café" in Helvetica.
- **Is there a font to fix it with?** No font file with that coverage ships in the Rust build. The only font file
  the app ships is Droid Sans Fallback.
  - The OFL licences in `engine-licenses/urw-metrics` and `urw-outlines` cover Nimbus Sans metric and outline
    data compiled into the WASM (`builtin_outlines_data.rs`, `substitute_outlines_data.rs`; they include
    Latin Extended-A, Greek and Cyrillic). They do not cover a font file that TypeScript could read.
  - The Foxit Type 1 fonts in `engine/vendor/hayro-interpret-0.8.0/assets` are compiled into hayro and are not
    files either.
  - So no font is added here. Candidates for the owner:
    - **Nimbus Sans Regular and Bold (URW, OFL-1.1), the font MuPDF embedded.** The repo holds them only as
      test reference data: `engine/reference/m1-20261012/Helvetica.cff` is 33,151 bytes and is the pinned
      `NimbusSans-Regular.cff` (sha256 366720e0...). `Helvetica-Bold.cff` is 34,024 bytes. They are CFF, not
      TrueType: the subsetter here would need a CFF path, or each file would embed the whole font as MuPDF did
      (about 33 KB per style), plus a glyph-name-to-Unicode table (Cyrillic glyphs are `afii100xx` names).
    - **Noto Sans (OFL-1.1).** Not in the repo. Its size was not measured, because nothing was downloaded.
    - **A Rust compose-scan op** could embed the Nimbus Sans outlines the WASM already carries, with no new
      asset.
- **Compression.** Streams are compressed with `CompressionStream` where it exists (Safari 16.4+, Chrome 80+).
  On Safari 15.0-16.3 they are written uncompressed: valid, a little larger, and tested.

### Verified (all through `tools/slot`)

- `tsc --noEmit` passes for mobile, and the mobile build passes, with its MuPDF check.
- `node --test tests/*.test.mjs` passes 17 of 17, including the coverage-per-block test and the NFC and
  subset-tag test added after review.
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
    - `engine/tools/build-workflow.sh --wasm-only` on 1.98.1. The new flag skips the native CLI binary; the
      default behaviour is unchanged;
    - uploads `engine/web/pkg`, `sources.sha256` included, and outputs the WASM's sha256.
  - The crates.io registry is cached with `actions/cache@v4`, keyed on `engine/bridge/Cargo.lock`; the build is
    not cached. v4 is the major known to exist, and the owner may align it with the repo's other actions.
  - The three installer builds download it, so all ship the same WASM. Their `scripts/build.mjs` re-checks
    `sources.sha256` against the same commit (`.gitattributes` `* -text` keeps Windows bytes identical).
  - Each platform runs its packaged smoke, then `scripts/check-package.mjs` against the installed resources:
    the DMG's app on macOS, `%LOCALAPPDATA%\Programs\Tandem PDF\resources` on Windows, and
    `/opt/tandem-pdf/resources` on Linux.
  - `check-package.mjs` requires `engine: rust` in both the report and the worker round trip. It also requires:
    - no file named after MuPDF in `app.asar`, and no import, URL or script tag naming one;
    - no MuPDF code under any name: the WASM markers `fz_new_context`, `pdf_new_document` and `mupdf-wasm`,
      and the glue marker `libmupdf`, all absent from the Rust package;
    - the WASM's sha256 equal to the engine job's (`TPDF_WASM_SHA256`), so all three installers ship one WASM;
    - the engine's notices, with `engine-licenses/` present, non-empty and holding wasm-bindgen's licence;
    - a `NOTICE.txt` with no IJG sentence.
  - `linux-check` (Ubuntu 22.04 and 26.04) also requires `engine: rust`.
  - The MuPDF source archive step is removed. The release job downloads only `installers-*`, so engine-pkg is
    neither attached nor listed in SHA256SUMS.
  - Permissions are `contents: read` by default, and only the release job gets `contents: write`. The checkouts
    in the jobs that build crates and run npm use `persist-credentials: false`.
- **Validated locally.**
  - `release.yml` parses with Ruby's Psych. It has 4 jobs, `build` needs `engine`, and the env pins are
    strings. Only `release` has write permission, its download pattern is `installers-*`, and both checkouts
    have `persist-credentials: false`.
  - `npm run package` ran on darwin-arm64 (peak 576 MB). The packaged `--smoke` gives ok, packaged, sandbox, and
    the engine and round trip both `rust`. The protocol requests are the Rust ones only.
  - After the review fixes, the package was rebuilt and the smoke rerun. `check-package.mjs` passes with
    `TPDF_WASM_SHA256` set (244 files in `app.asar`, wasm-bindgen's licence included). A report naming MuPDF
    fails it, and so does a wrong WASM hash. The MuPDF content markers occur in MuPDF 1.28.1's
    `mupdf-wasm.wasm`, `mupdf.js` and `mupdf-wasm.js`, and in no file of the Rust package.
- **Not verified.**
  - Any GitHub run: the toolchain install, the `cargo install` time, and artifact hand-off.
  - The Windows and Linux packaged smokes and `check-package.mjs` there (the pwsh step calls node after the
    installer smoke).
  - Signing and notarization with the Rust WASM.

## 3. Licence wording for owner sign-off (the desktop texts are app-worker.md's; no new legal claims)

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

- the new `engine/NOTICE` paragraph for the WASM bridge. It records that wasm-bindgen 0.2.129 and
  wasm-bindgen-shared are compiled into the WASM, that wasm-bindgen-cli-support generates the JS glue, and the
  build-time crates. Their licence files are copied verbatim from the crate sources into
  `engine/reference/licenses/<crate>-<version>/`, with hashes in `engine/reference/bridge-license-files.json`.

**Mobile has no NOTICE or source statement (not fixed; for the owner).** The app ships the licence files and the
footer links to them. But unlike the desktop `NOTICE.txt`, it has no statement of the AGPL licence of the whole
app and no pointer to the corresponding source (AGPL section 6). That matters for app-store distribution, and
whether App Store or Play terms are compatible with the AGPL is a legal question this note does not answer.

Proposed text, to be shown on a "Licences and source" screen linked from the footer and shipped as
`licenses/NOTICE.txt`, modelled on the desktop wording above:

> Tandem PDF mobile <version>
>
> Tandem PDF's own code is MIT (Tandem-LICENSE.txt). This app bundles the Tandem PDF Rust engine,
> AGPL-3.0-or-later and informed by MuPDF (Engine-NOTICE.txt, Engine-AGPL-3.0.txt; its dependencies' licence
> files are in engine-licenses), so the app as a whole is distributed under the AGPL.
>
> Corresponding source: the app's source at this version's tag, the mobile app in its mobile folder and the
> engine in its engine folder, at https://github.com/Shiro-Legacy/TPDF/releases/tag/v<version> (build steps:
> mobile/README.md and engine/tools/build-workflow.sh).
>
> The CJK fallback font Droid Sans Fallback, Copyright (c) 2005-2008 The Android Open Source Project, is
> Apache-2.0 (third-party/droid-sans-fallback-NOTICE.txt). Capacitor is MIT (Capacitor-*-LICENSE.txt).

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

## 6. Owner decisions needed

1. **Licence wording.** Sign off on the texts in section 3: the desktop `NOTICE.txt`, the About panel, the
   README and mobile footer wording, and the `engine/NOTICE` paragraph for the bridge.
2. **Mobile notice.** Decide on the proposed NOTICE screen and file, and whether to distribute an AGPL app
   through the app stores (legal).
3. **Corresponding source for releases.** Is the tag enough, or should a `cargo vendor` archive of the engine's
   crates be attached? (Legal.)
4. **No MuPDF fallback.** Confirm that `package.mjs` refuses MuPDF builds (commit 31b7cad).
5. **Scanner fonts.** Characters outside WinAnsi, CJK and Hangul draw as boxes in AI-rebuilt pages (section 1).
   Choose one: Nimbus Sans CFF (OFL, 33 KB per style, needs a CFF path and a glyph-name map), Noto Sans (OFL,
   size not measured), a Rust compose-scan op that reuses the WASM's Nimbus Sans outlines, or accepting the gap.
6. **Duplicated PDF writer.** The TypeScript scanner duplicates code the engine already has: a PDF serializer,
   TrueType cmap and width parsing, the Helvetica widths and JPEG header parsing. The two will drift. A Rust
   compose-scan op would remove the duplicate, and would also settle item 5.
7. **Other output differences.** Accept non-embedded Helvetica and DeviceRGB.
8. **Device verification.** Verify on iOS and Android (section 1). The iOS target says 15.0, but the existing UI
   already needs 15.4 for `<dialog>`.
9. **Follow-ups.** Move `mupdf` to devDependencies, which needs a lockfile refresh. Align the `actions/cache`
   major.

## 7. Gates

The full gates (`tools/gates.sh`, under slot, logs in `tpdf-wt/gates-out/p9-mobile/`) ran on 2c607d9 and again on
1326d17, after the review fixes. Both runs pass every gate with the same numbers:
- fmt and clippy, engine and bridge, on stable and 1.92;
- native tests: 1,618 passed, 0 failed on stable and on 1.92;
- bridge tests;
- the WASM package and `npm run build`;
- the app suite without Electron: 681 tests, 568 pass, 0 fail, 113 skip;
- `tests/run-rust.mjs`: 670 tests, 567 pass, 0 fail, 103 skip.

`engine/web/pkg` is not tracked. Mobile, after the last change:
- `tsc` and the build pass;
- `node --test`: 17 of 17;
- the three Electron UI suites pass.
