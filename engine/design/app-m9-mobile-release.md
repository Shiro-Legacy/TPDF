# M9: mobile on the Rust engine, and the release engine switch (prepared)

October 17, 2026. Branch `app-p9-mobile-release` (base 46f1e95). This is a prepared change for the owner's review
(acceptance report 6.3 and 6.4, completion plan M9). It is not merged into the release path. Nothing is pushed,
no GitHub Action ran, and nothing is published. It does not touch `engine/src` or `src/renderer`.
It was brought up to date with the final `rust-engine-app` (b6bc7b9, then the docs pass ba70615) on October 17;
section 8 lists what the merge changed.

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

Since app-p11-rust-scan (October 17) the mobile app runs fully on the Rust engine: the Rust engine writes the
scanner's PDF. The TypeScript writer (`scan-pdf.ts`, `scan-font.ts`, `scan-metrics.ts`, 590 lines) is deleted.
It duplicated the engine's serializer and xref, TrueType cmap and width parsing, the Helvetica widths and JPEG
header parsing. Section 9 compares its output with the new path on the same inputs.

- **`scan-compose`** (`engine/src/app/ops/scan.rs`) takes the JPEGs once (`images: ArrayBuffer[]`) and the pages:
  each has its size in points, the index of the image filling it (optional), stroked `boxes` and text `runs`.
  - Positions are in points from the top-left, y down. A run's (x, y) is its baseline origin.
  - A run is `invisible` (render mode 3, a searchable scan's OCR line) or filled in `color` (rebuilt pages).
  - `width` scales a run horizontally so that its advance is that width (an OCR line fitted to the recognized
    box), as before.
  - `producer` becomes `/Info << /Producer >>`.
  - It reads no session state and changes none. The reply is `{ bytes, pageCount }`.
- **`scan-measure`** gives the advance of each distinct character of a text in the regular and bold fonts that
  `scan-compose` draws it with, and which font that is. The scanner lays out rebuilt pages with those widths
  (`scan-compose.ts`: Markdown blocks, word wrap with `Intl.Segmenter`, tables, A4 page breaks). So the layout
  and the drawing use the same widths.
- **What stays in TypeScript** (`mobile/src/scan-compose.ts`, 190 lines): the page layout above, NFC and
  control-character cleaning, the user-facing messages, and the reopen check. The reopen check is `open` with
  the expected page count, then `validate` (the strict save check), in a fresh session of the same engine.
- **Reuse.**
  - The image is `pages/images.rs` `read` (the JPEG headers, Adobe CMYK `/Decode`, ICC chunks), stored
    with `store_shared`, so equal images in different buffers are stored once.
  - The file is written by `Builder`, `PageTree` and the app writer (`pages::written`, the 50 MiB ceiling).
  - The Helvetica widths come from `forms::text::Metrics`, which reads the engine's built-in metrics.
  - The Latin glyph names come from `forms::scripts::Eight::Latin`, the same font FreeText and widget
    appearances use.
  - The font programs are built by `view/fonts_scan.rs`. It is a child of the view's `fonts.rs`, so it uses that
    module's sfnt reader and writer, cmap reading, composite glyph flattening and CFF writer, which the view
    already uses to draw the base-14 fonts and the CJK fallback.
- **The CJK font** reaches the op as it reaches every other op that needs it: `set_cjk_font`, module-wide.
  - When a character needs it and none is installed, the op fails with reason `no-font` and turns
    `cjk_font_wanted` on.
  - The adapter (`FONT_OPS` in `engine-rust.ts`) then reads the font, installs it and dispatches the op again,
    as it does for a render. A font that cannot be read is not retried, and the scanner shows "The font for
    non-Latin text could not be loaded."
  - The scan worker now loads the engine through the desktop build's `worker-engine.js`. That gives it the same
    loader and the same synchronous font read as the editor's worker; `scripts/build.mjs` keeps the import
    external, so `dist/engine/` still holds one copy.
- **Fonts, per character, the first that draws it:**
  1. Helvetica or Helvetica-Bold, WinAnsiEncoding, not embedded, with `/Widths` (as before), and since the
     review a ToUnicode CMap of the codes used, so U+00A0 copies as itself.
  2. A Latin letter WinAnsi lacks: Helvetica with the `Eight::Latin` `/Differences` glyph names, not embedded,
     with a ToUnicode CMap.
  3. A subset of the compiled-in URW Nimbus Sans Regular or Bold outlines (OFL, the font MuPDF embedded). It is
     embedded as a Type1 simple font with a bare CFF program (FontFile3 /Type1C), 255 glyphs per font object,
     with `/Differences`, `/Widths` and ToUnicode. The program carries each glyph's advance (the most common
     one as defaultWidthX, the others in their charstrings), equal to `/Widths`, and URW's copyright and the
     OFL notice in its Top DICT. The OFL text names no Reserved Font Name, so the subsets keep the name
     `NimbusSans-Regular` or `NimbusSans-Bold` behind their tag.
  4. A subset of the installed Droid Sans Fallback: Type0, Identity-H, CIDFontType2, ToUnicode. As in the
     TypeScript writer, composite glyphs are kept and their components join the subset, renumbered. A
     character whose glyph is already in the subset (above all the .notdef of characters the font lacks) is a
     16-byte composite of it. The name table keeps only the copyright and licence records (IDs 0, 13 and 14;
     Apache-2.0, so the name may stay).
  5. Nothing draws it: the CJK font's `.notdef` (a box), still mapped to its Unicode in ToUnicode. A combining
     mark or zero-width format character the font lacks is an empty glyph instead. Every combining mark has
     no advance (`/W` 0 and in `scan-measure`): a .notdef advance pushed the next letter along, and MuPDF then
     split the line.
- **Limits** (refused before any image is read):
  - 100 pages (`too-large`);
  - 32 MiB per image, and 50 MiB for all images together: the file stores each image as it is, so more could
    never be saved;
  - 4,000 runs and 4,000 boxes per page, and 40,000 runs and boxes in a request. Each is a JSON object the
    adapter encodes and the engine parses, and 400,000 boxes took 35 s. Since round 3 the adapter
    (`engine-rust.ts`) counts them before encoding and refuses with the engine's own reply (`too-large`, `runs`)
    in under a second; the engine keeps its check;
  - 4,000 characters per run, and 300,000 in all, which is 12 scans of 24,000 (the measured text too);
  - 60,000 distinct characters in the CJK font. The subset holds .notdef, one glyph per character and the
    components of composite glyphs, at most 65,535 (the TrueType limit). The cap leaves 5,535 glyphs for
    components, and a subset past 65,535 is refused too (`too-large`, `characters`). Measured: all 20,992 CJK Unified characters make a 1.69 MB file in
    180 ms (native), and 60,000 characters the font lacks make one under 3 MB;
  - output over the 50 MiB save ceiling.
  - Each `too-large` refusal names its limit in `detail` (`pages`, `images`, `image-size`, `runs`, `text`,
    `characters`, `output`), and the scanner shows a message for each.
  - Page sides and coordinates must be within 14,400 pt. Malformed input is `invalid`, and so is an image that is
    not a readable JPEG (shown as "Invalid scan image."). That includes a frame that declares 0 pixels (its
    height in a DNL marker), more than 16,384 pixels a side, or arithmetic coding (SOF9/SOF10), which the
    scanner's canvas never writes and several viewers do not decode.
- **Text cleaning:** tabs become spaces, and control characters and soft hyphens are dropped. A soft hyphen
  shows only where a line breaks at it, and runs are already lines.
- **Right-to-left text** (Hebrew, Arabic, Syriac, Thaana, NKo and the like) is drawn in visual order
  (`scan_bidi.rs`): UAX #9 for one paragraph per run.
  - Numbers (W4-W7): one `.` `,` `/` `:` (or `+` `-`) between digits joins them, and `%`, currency signs and
    the other ET characters next to a number go with it. Other separators are neutrals.
  - Combining marks: a right-to-left cluster is drawn marks first, so readers, which reverse character by
    character, give each mark after its letter.
    - When that puts a mark at the start of the line (a run whose last letter has one, common in Arabic), a
      zero-width RIGHT-TO-LEFT MARK is drawn before it. Without it, readers leave that mark at the start of
      the line, and the copy then ends with the RLM.
  - Neutrals between equal directions take that direction.
  - Measured with MuPDF, the engine and PDFKit, each finds every typed word and number:
    - plain Hebrew and Arabic;
    - "מחיר 3.50 שקל", "12/05/2026", "1,000", "50%", "₪25", "$30";
    - pointed Hebrew ("שָׁלוֹם עֲלֵיכֶם") and Arabic with harakat ("مَرْحَبًا بِالْعَالَمِ").
    - Before round 2 they found none, and before round 3 they found no numbers with separators and no pointed
      words.
  - **No /ActualText.** Measured, MuPDF and the engine's view reverse a span's ActualText of right-to-left
    characters again, so the typed text extracted backwards and searches failed; PDFKit ignores it.
  - **No mirrored brackets** (UAX #9 L4). All three readers copy a mirrored bracket as drawn, so a visible
    "(חדש)" copied as ")חדש(". Unmirrored, MuPDF and the engine copy each bracket as typed, and PDFKit copies
    the typed line. The cost is the drawing: a visible right-to-left run shows its brackets facing the wrong
    way, around boxes, since no font draws Hebrew or Arabic.
  - Limits:
    - no explicit embeddings or isolates, and no bracket pairs (N0);
    - Arabic-Indic digits are counted as European ones (no W2);
    - MuPDF and the engine order a line's segments their own way. They give "שקל3.50 מחיר" for
      "מחיר 3.50 שקל", and "abc ) (חדש2026 שנת" for "שנת 2026 (חדש) abc", while PDFKit gives the typed
      lines. Each word and number still reads and searches as typed.
    - Hebrew and Arabic still draw as boxes in rebuilt pages: no font has them.
- **engine.ts (MuPDF)** refuses both ops with reason `unsupported`, since it is only a test oracle now.
- **Streams** are always Flate-compressed by the engine, so Safari 15.0-16.3, which has no `CompressionStream`,
  now gets compressed files too.
- **Cost** (Node, M-series; `scratch/p8/scan/compare/timing.mjs`):
  - A 12-page searchable scan, 40 OCR lines per page with CJK: 36,311 bytes, composed in 34 ms the first time
    (the CJK font installed) and 8-9 ms after, checked in 0-2 ms.
  - The 6-page rebuilt document: 8,864 bytes in 29 ms, then 8-9 ms.
  - WASM linear memory goes from 4.2 MB to 8.4 MB (the installed 3.4 MB font).

Why a new op rather than `pdf-from-images`:
- That op sizes pages from the image resolution (72 dpi when none is given), not the scanner's 792 pt long side.
- It applies EXIF orientation, it reads PNGs, and its images and limits are the desktop picker's.
- It has no text.
- Extending it would have mixed two contracts, so `scan-compose` is its own op. It reuses the same image reader,
  and it ignores EXIF orientation as the TypeScript writer did; the scanner's canvas JPEGs carry none.

What the output keeps, compared with MuPDF's scanner and the TypeScript writer:

| | MuPDF scanner | TypeScript writer (deleted) | Now (`scan-compose`) |
| --- | --- | --- | --- |
| Page size | long side 792 pt, scan aspect; A4 for rebuilt pages | same | same (asserted) |
| Image | JPEG stored unchanged (DCTDecode) | same, one stream per distinct image | same, byte-identical (asserted), equal images stored once |
| Colour space | `ICCBased`, MuPDF's sRGB profile | `DeviceRGB`; a profile in the JPEG kept as `ICCBased` | same |
| OCR text | invisible, fitted to the line width | same (Tr 3, horizontal scale) | same; positions identical (section 9) |
| WinAnsi text | Nimbus Sans, embedded (CFF) | Helvetica/Helvetica-Bold, not embedded, `/Widths` | same |
| Other Latin, Greek, Cyrillic, symbols | Nimbus Sans, embedded whole | boxes (Droid subset) | Latin glyph names, or an embedded Nimbus Sans subset (coverage below) |
| CJK, kana, Hangul | Droid Sans Fallback embedded whole (3.5 MB) | a Droid subset (Type0, ToUnicode) | same: composites kept, components renumbered |
| Metadata | `/Producer (MuPDF 1.28.1)` | `/Info << /Producer (Tandem PDF) >>` | same |
| Streams | compressed | compressed where `CompressionStream` exists | always compressed |
| Limits | 1-12 scans, 100 output pages, 50 MB | same messages | same messages; the engine enforces its own bounds too |

These intentional differences from MuPDF remain, with the reasons given at p9:
- Base-14 Helvetica is not embedded. Files are 25-60 times smaller, but they are not PDF/A.
- `DeviceRGB` instead of shipping MuPDF's sRGB profile (an Artifex asset).
- Tabs become spaces, and other control characters are dropped (MuPDF drew .notdef). The scanner composes
  separate accents first (NFC); the engine drops control characters too.

### Font coverage outside WinAnsi, rebuilt pages

Measured by `scan-measure` (`mobile/tests/scanning.test.mjs`, "font coverage per script"). The table counts the
assigned characters per block that are not in WinAnsi (Node's Unicode version) and the font that draws each
one. Searchable scans are not affected, since their text is invisible.

| Block | Assigned | Latin names | Nimbus Sans | CJK font | Box (before: box) |
| --- | --- | --- | --- | --- | --- |
| Latin Extended-A | 121 | 121 | 0 | 0 | 0 (121) |
| Latin Extended-B | 207 | 4 | 6 | 0 | 197 (207) |
| IPA | 96 | 0 | 0 | 0 | 96 (96) |
| Combining marks | 112 | 0 | 0 | 0 | 112 (112) |
| Greek | 135 | 0 | 72 | 0 | 63 (135) |
| Cyrillic | 256 | 0 | 137 | 0 | 119 (256) |
| Hebrew | 88 | 0 | 0 | 0 | 88 (88) |
| Arabic | 256 | 0 | 0 | 0 | 256 (256) |
| Thai | 87 | 0 | 0 | 1 | 86 (86) |
| Vietnamese (Latin Extended Additional) | 256 | 0 | 8 | 0 | 248 (256) |
| General Punctuation | 96 | 0 | 8 | 4 | 84 (92) |
| Currency | 33 | 0 | 4 | 0 | 29 (33) |
| Letterlike | 79 | 0 | 5 | 0 | 74 (79) |
| Arrows | 112 | 0 | 7 | 0 | 105 (112) |
| Math Operators | 256 | 0 | 28 | 0 | 228 (256) |
| Box Drawing | 128 | 0 | 0 | 0 | 128 (128) |
| Misc Symbols | 256 | 0 | 11 | 0 | 245 (256) |
| CJK Symbols | 64 | 0 | 0 | 36 | 28 (28) |
| Hiragana | 93 | 0 | 0 | 90 | 3 (3) |
| Katakana | 96 | 0 | 0 | 91 | 5 (5) |
| CJK Unified (BMP) | 20,992 | 0 | 0 | 20,902 | 90 (90) |
| Hangul Syllables | 11,172 | 0 | 0 | 11,172 | 0 (0) |
| Fullwidth forms | 225 | 0 | 0 | 164 | 61 (61) |
| Emoji | 1,895 | 0 | 0 | 0 | 1,895 (1,895) |

- **Now drawn:** Polish, Czech, Hungarian, Romanian, Turkish, the Baltic languages and the rest of Latin
  Extended-A. Also the modern Greek alphabet with tonos, and Russian, Ukrainian, Belarusian, Serbian, Macedonian
  and Bulgarian Cyrillic. Also ← ↑ → ↓ ↔ ↕, ≤ ≥ ≠ ≈ ∑ √ ∞ ∂ ∆ ∏ ∫, € ₤ ₣ ₧, and ‰ ‹ › † ‡ … in Nimbus where
  WinAnsi lacks them.
  - These were boxes with the TypeScript writer. MuPDF drew them with the same Nimbus Sans, embedded whole.
- **Still boxes:** most of Vietnamese, because Nimbus Sans has no precomposed Latin Extended Additional letters
  and no combining marks. Only ơ, ư and a few others (Latin Extended-B) draw, so "Tiếng Việt" shows ế and ệ as
  boxes. Also polytonic and archaic Greek, historic and non-Slavic Cyrillic, IPA, Hebrew, Arabic, Thai, box
  drawing, emoji, and most symbols.
  - Their text still extracts, searches and copies (ToUnicode; tested).
  - A fix needs a font with that coverage. No font file was added (the brief: no new font or dependency).

Recorded differences from the TypeScript writer and from MuPDF:
- **The Latin glyph-name font is not embedded.** It is drawn by the viewer's own Helvetica, as FreeText and
  widgets already are; Acrobat, pdf.js, MuPDF and the engine all have these glyphs.
- **The Nimbus subsets are bare CFF** (no hints, no FontBBox in the program; the descriptor has it). The engine
  view and MuPDF draw them (tested by rendering both), and so do PDFKit and CoreGraphics (checked). Acrobat was
  not tried.
- **The CJK subset** has the same six-letter tag (the same hash of the same characters). It keeps the name
  table's copyright and licence records.
- **U+00A0** copies as itself from the engine. MuPDF and PDFKit turn it into a space; all three find it when the
  search uses a space.
- **Rebuilt pages with Greek, Cyrillic or Latin Extended text wrap differently** from the TypeScript writer,
  because the characters now have their real widths instead of the CJK font's box width. Text, page count and
  the positions of the lines before the first such character are unchanged (section 9).

### Verified at p9, with the TypeScript writer (all through `tools/slot`; section 9 has the Rust op)

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
  scanner's layout uses `Intl.Segmenter` (14.1); since app-p11-rust-scan the engine compresses the scan, so
  `CompressionStream` is no longer used. Capacitor serves `.wasm` as
  `application/wasm` on both platforms, so streaming compile works.
- **Workers.** The engine worker uses a top-level await in a module worker (Safari 15). The scan worker loads
  the same `worker-engine.js` from its message handler.
- **iOS floor (pre-existing, not changed).** The mobile UI uses `<dialog>` and `Array.prototype.at`, which need
  Safari 15.4. The Xcode target says iOS 15.0.
- **Pending on a device** (not run: no Xcode, simulator or gradle, per the brief):
  - the WASM compiles and instantiates in WKWebView on iOS 15.x and current iOS;
  - `'wasm-unsafe-eval'` CSP handling there;
  - the engine worker's synchronous XHR for the CJK font over `capacitor://`;
  - the scan worker's synchronous XHR for the CJK font (since app-p11-rust-scan, the same read as the engine
    worker's);
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
    - Since the merge, the script is the head's reproducible build (`docs/building-rust-release.md`): physical
      paths, `--remap-path-prefix` for the checkout, target, `CARGO_HOME` and `RUSTUP_HOME`, and
      `tools/rustc-stable-metadata.sh` as `RUSTC_WRAPPER`, for the wasm32 build only. The job fails if the module
      names `$HOME`, the workspace, `/home/` or `/Users/`.
    - It then compares the WASM and glue with `engine/reference/wasm-release.sha256` (sha256sum format: the
      hashes of `sources.sha256`, the WASM and the glue from a local build). Same sources: the bytes must match,
      or the job fails. Other sources: the record is stale and the job warns, so an engine commit does not need a
      record refresh to pass CI.
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
    - a WASM that names no build-machine path, and the pinned CJK font;
    - every shipped notice the same bytes as the checkout's (`Engine-NOTICE.txt`, `Engine-AGPL-3.0.txt`,
      `PDF4All-MIT.txt`, the LGPL and Droid notices), `engine-licenses/` equal to `engine/reference/licenses`
      file for file, the asar's `licenses/` equal to the readable folder, and Electron's two notices (inside the
      bundle on macOS, beside the binary elsewhere);
    - a `NOTICE.txt` that names the Rust engine and the corresponding source, and has no IJG sentence and no
      MuPDF archive.
  - Since the merge it is the only package checker. The head's `scripts/check-macos-package.mjs` (the M10 macOS
    checklist) imports its `checkPackage()` for a new `package` item, its `attribution` item (the licence checks)
    and `wasm-paths`, instead of keeping its own copies; `tests/m10-package.test.mjs` uses its `wasmPathLeaks`.
  - The release job also needs `engine`, and the draft's notes name the WASM's sha256, ahead of GitHub's
    generated notes.
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
  - That the Linux engine job reproduces the macOS record. Three macOS checkouts gave the same WASM for these
    sources (the head's gates and benchmarks, and this branch's gates: `3c18b0e8…`), but a Linux host never built
    it. If the first CI run differs, the record check fails that run.
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

- the `engine/NOTICE` paragraph for the WASM bridge (one paragraph since the merge: the head's M10 entry for the
  same crates is folded into it). It records that wasm-bindgen 0.2.129 and
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
5. **Scanner fonts.** Done in app-p11-rust-scan, with no new font: Latin Extended-A, modern Greek, Slavic
   Cyrillic, arrows, maths and currency now draw in rebuilt pages (Helvetica's Latin glyph names, then embedded
   Nimbus Sans subsets; coverage table in section 1). Most of Vietnamese, IPA, Hebrew, Arabic, Thai, emoji and
   most symbols still draw as boxes. Covering them needs a font file with that coverage, such as Noto Sans
   (OFL, not in the repo, size not measured): still an owner decision.
6. **Duplicated PDF writer.** Done in app-p11-rust-scan: the Rust op `scan-compose` (with `scan-measure`) writes
   the scan, and the TypeScript writer is deleted (section 9).
7. **Other output differences.** Accept non-embedded Helvetica and DeviceRGB.
8. **Device verification.** Verify on iOS and Android (section 1). The iOS target says 15.0, but the existing UI
   already needs 15.4 for `<dialog>`.
9. **Follow-ups.** Move `mupdf` to devDependencies, which needs a lockfile refresh. Align the `actions/cache`
   major.
10. **WASM record policy.** `engine/reference/wasm-release.sha256` must be refreshed after each engine change
    (command in `docs/building-rust-release.md`). A stale record only warns, also on a tag. Decide whether a tag
    build should instead fail without a current record.

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

## 8. Merge with the final head (October 17)

`git merge --no-ff rust-engine-app` at b6bc7b9 (450e1ce), then the docs pass at ba70615 (24ff369, no conflicts).

- **Conflicts.**
  - `engine/NOTICE`: both sides recorded the bridge's crates. One paragraph is kept, this branch's (it also names
    wasm-bindgen-cli-support and rustversion and points at `bridge-license-files.json`), noting both the M9 and
    M10 uses. The licence folders both sides added are byte-identical; the manifest's 14 hashes still match, and
    every `wasm-bindgen*`, `bumpalo` and `rustversion` folder is listed in it.
  - `engine/tools/build-workflow.sh`: the head's reproducible wasm32 build with this branch's `--wasm-only`, which
    skips only the native CLI at the end.
- **Adjusted for the head.** `release.yml` (section 2), one package checker (section 2), and the mobile bundle no
  longer copies the head's `recovery.js` (the desktop renderer's edit log; `policy.ts` now serves it, and only
  `renderer.js` imports it).
- **Scanner.** The head's lexer, open, redaction and save changes do not reach `scan-pdf.ts`: it composes PDFs in
  TypeScript and uses the engine only for `open` and `validate`. Its tests (17 of 17, with the Rust engine at
  b6bc7b9) and the three Electron UI suites pass unchanged.
- **Re-gated** (logs in `tpdf-wt/gates-out/p9-mobile-refresh/`, on 450e1ce plus the checker changes): every gate
  passes. Native tests 1,683 passed, 0 failed on stable and 1.92; the app suite without Electron 760 tests, 616
  pass, 0 fail, 144 skip; `tests/run-rust.mjs` 749 tests, 623 pass, 0 fail, 126 skip. The WASM is `3c18b0e8…`,
  13,497,323 bytes, with no `/Users/` or `/home/` path.
- **Package.** `npm run package` (peak 563 MB): the packaged smoke is ok, packaged, sandboxed, engine and round
  trip `rust`. `check-package.mjs` passes with `TPDF_WASM_SHA256` (245 files in `app.asar`); a wrong hash, a
  report naming MuPDF, and an edited `Engine-NOTICE.txt` plus an edited licence file each fail it. The full
  macOS checklist (`check-macos-package.mjs`, now on the shared checker) passes every item, `package` included
  (median first page 551 ms cold, 565 ms warm; peak 965 MB). `release.yml` parses with Ruby's YAML (4 jobs;
  `release` needs `engine`, `build`, `linux-check`).


## 9. The scanner on the Rust engine (app-p11-rust-scan, October 17)

Branch `app-p11-rust-scan` (base 92e7612). The owner decided that mobile runs fully on Rust. The TypeScript
scan writer is replaced by the Rust ops `scan-compose` and `scan-measure` (section 1, "The scanner";
`APP-ENGINE-DESIGN.md`, "Scanner"). Decisions 5 and 6 (section 6) are done.

**Comparison with the TypeScript writer.** Both were built from the same tree before the writer was deleted,
and run on the same inputs (`tpdf-wt/scratch/p8/scan/compare/compare.mjs`):
- the old `scan-pdf.ts` from 92e7612, with `CompressionStream`;
- the new `scan-compose.ts` with the WASM package.

MuPDF reads both outputs: page bounds, the raw `Im0` stream, structured-text lines and their boxes.

| Input | Pages old / new | Bytes old / new | Page sizes | Image bytes | Text | Lines | Max line x / y / width delta | CJK subset tag |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| image, 3 pages of one JPEG, mixed orientation | 3 / 3 | 26,698 / 24,515 | same | same, and equal to the JPEG | same | 0 | - | - |
| searchable, 4 OCR lines (Latin, CJK, kana, Hangul), 2 pages | 2 / 2 | 29,444 / 28,929 | same | same | same | 6 | 0 / 0 / 0 | same |
| rebuilt, Latin only, with a list and a table | 1 / 1 | 3,018 / 3,804 | same | - | same | 15 | 0 / 0 / 0 | - |
| rebuilt, 6 pages with CJK and a table | 6 / 6 | 7,500 / 8,449 | same | - | same | 157 | 0 / 0 / 0 | same |
| rebuilt, dense CJK, kana and Hangul | 1 / 1 | 7,012 / 6,460 | same | - | same | 30 | 0 / 0 / 0 | same |
| rebuilt, Polish, Czech, Greek, Cyrillic, symbols | 1 / 1 | 4,939 / 12,030 | same | - | same text, different line breaks | 8 | 0 / 0 / 106 | old: Droid; new: none (Latin names and Nimbus) |

- Where the fonts are the same (WinAnsi and CJK text), the outputs read back identically. The positions are
  equal to MuPDF's precision, so well within the review's 0.2 pt for positions and 1 pt for fitted widths. The
  native tests check those two tolerances on the engine's own extraction.
- The last row is the intended change: those characters were boxes at the CJK font's width, and they now have
  their real Nimbus or Helvetica widths. The same words therefore wrap at other points.
- The file grows by the two embedded Nimbus subsets (regular and bold), about 3 KB each.
- The sizes are after the review fixes (section 9, "Review minors"). The Helvetica ToUnicode adds about 0.4 KB
  per style used, and the CJK name records about 0.2 KB.
- The CJK subset tags are identical: the same hash of the same characters.

**Tests.**
- Native, `engine/tests/app_scan.rs` (6 tests):
  - structure, the JPEG byte for byte with one image object for equal inputs, page sizes, and Producer;
  - extracted text equal to the runs, at their x and baseline within 0.2 pt, and fitted runs ending within 1 pt;
  - the font chain (F1/F2, E1, N1_0/N2_0 with Type1C programs and ToUnicode; boxes and colours in the content);
  - ICC passthrough;
  - `scan-measure` widths and fonts, and the compose widths matching them;
  - every limit and malformed input;
  - the CJK path: `no-font` with the cue set before the font is installed, then a Type0 subset under 20 KB with
    its ToUnicode, the same tag for the same characters and a box for ✓;
  - the scanner's open and validate checks on every output.
- Unit tests in `scan.rs` and `fonts_scan.rs`.
- App, `tests/rust-scan-compose.test.mjs`:
  - through the production adapter and WASM: the font is read once, on the cue, and the op dispatched again;
  - MuPDF reads the text, positions, image and Producer;
  - Greek drawn (dark pixels) by both MuPDF and the engine's view;
  - `text-all`, `search` and `validate` on the engine;
  - refusals, an unreadable font not retried, and engine.ts refusing both ops `unsupported`.
  - `tests/m9-recovery.test.mjs` classifies both ops as reads.
- Mobile, `mobile/tests/scanning.test.mjs` (16 tests in all): the scanner's old assertions on the new path, plus:
  - the font read only when CJK text needs it (not for Greek, Cyrillic or Latin Extended);
  - the coverage-per-block table (section 1), pinned;
  - a lone surrogate written as U+FFFD;
  - streams always compressed.
  - The `CompressionStream`-less test is gone with the code path; the Node `Buffer` font regression no longer
    applies, since the font is read by the engine.
- `electron tests/scanning-browser.cjs` passes against the preview server (image PDF, OCR, typeset PDF through
  the module worker loading `engine/worker-engine.js`).

**Gates** (`tools/gates.sh` under slot, logs in `tpdf-wt/gates-out/p11-scan/`, on 2e98508): every gate passes.
- fmt and clippy, engine and bridge, on stable and 1.92.
- Native tests: 1,691 passed, 0 failed, on stable and on 1.92.
- The bridge tests, the WASM package and `npm run build`.
- The app suite without Electron: 619 pass, 0 fail, 144 skip.
- `tests/run-rust.mjs`: 626 pass, 0 fail, 126 skip.
- Mobile: `npm test` (the root build, `tsc`, the mobile build with its MuPDF check, and 16 of 16 tests) and
  `electron tests/scanning-browser.cjs` pass.
- `engine/web/pkg` is not tracked.

**Review minors (round 2).** The review accepted both ops and found nine minors, all fixed:
1. The embedded CFF carries the real advances, equal to `/Widths`. `cff_program` takes them, and the view's own
   `cff` is unchanged. A test parses the CFF back and compares.
2. Right-to-left runs are drawn in visual order, without /ActualText; the measurements are in section 1. Search
   for the typed word works in MuPDF, the engine and PDFKit (tested in the first two; PDFKit checked by hand).
3. Soft hyphens are not drawn. The WinAnsi fonts have a ToUnicode, so NBSP copies as U+00A0 from the engine.
4. The CJK subset keeps composites instead of flattening them, which failed silently at about 20,000
   characters. The cap is now 60,000 distinct characters, refused `too-large` (`characters`) past it, and past
   65,535 glyphs with components. Tested at 20,992 real CJK characters and at 60,000 and 60,001 characters the
   font lacks.
5. 40,000 runs and boxes per request.
6. JPEG frames of 0 or more than 16,384 pixels a side, and arithmetic coding, are refused `invalid`.
   `pages/images.rs` now records arithmetic coding on `Placed`; pdf-from-images does not use it.
7. Each `too-large` has a `detail`, and the scanner a message for each.
8. The op's doc comment no longer claims Vietnamese.
9. The Nimbus subsets carry URW's copyright and the OFL notice. The Droid subset keeps the name records 0, 13
   and 14. Neither licence reserves a font name.

Tests: `engine/tests/app_scan_text.rs` (6 tests) and the bidi unit test; the app test adds Hebrew and Arabic
search in MuPDF and the engine and a drawn CJK line; the mobile tests add soft hyphen and NBSP, the item cap and
the per-limit messages.

Gates after the minors (logs in `tpdf-wt/gates-out/p11-scan-r3/`, on ee1d970): every gate passes. Native tests
1,698 passed, 0 failed, on stable and on 1.92. The app suite has 619 pass, 0 fail; `run-rust` has 626 pass,
0 fail. The first run (`p11-scan-r2`) failed only clippy 1.92 (`nonminimal_bool`), which is fixed in ee1d970.
Mobile `npm test` passes 16 of 16 with the build, and `electron tests/scanning-browser.cjs` passes.

**Review minors (round 3).** The recheck accepted both ops again (composites pixel-identical, CFF widths exact)
and found five minors, all fixed:
1. Numbers in right-to-left runs follow W4-W6: prices, dates, thousands, percentages and currency read and
   search as typed in MuPDF, the engine and PDFKit.
2. Combining marks in right-to-left clusters are drawn before their letter, with an RLM before a line-initial
   mark. Every combining mark has no advance, and a mark the CJK font lacks is an empty glyph, not a .notdef.
   Pointed Hebrew and Arabic with harakat read and search as typed in all three.
3. Brackets are no longer mirrored (section 1 has the reasons and the cost).
4. The adapter refuses a scan-compose of more than 40,000 runs and boxes before encoding it, with the engine's
   reply. In the app test, 400,000 boxes are refused in under a second, where it took 33 s.
5. The notes no longer say Droid's glyphs have no components, or that composites are flattened.

Tests:
- `engine/tests/app_scan_text.rs` adds numbers and pointed letters (text, search, zero `/W` for marks).
- The bidi unit tests add W4-W6, mark order and the RLM.
- `tests/rust-scan-compose.test.mjs` adds the numbers, pointed words and Arabic in MuPDF and the engine, and
  the adapter's cap.
- PDFKit was checked by hand with the reviewer's `pdfkit` probe on the same lines.
