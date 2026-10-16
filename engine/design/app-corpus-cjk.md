# App CJK fallback font (M9)

October 15, 2026. The Rust build ships Droid Sans Fallback and draws CJK text in fonts a PDF does not embed, and the app's own CJK form values, with it. Read with `app-view.md` (the view's CJK route: `view::set_cjk_font`, `cjk_font_wanted`, `has_cjk_font`, the cue and its latch) and `app-worker.md` (build, policy, packaging). This note supersedes `app-view.md`'s "Open item (M9/M10)" and its two bullets on characters past the BMP and on the appended glyphs (below).

## How the font ships: a committed copy (decision)

- `assets/fonts/DroidSansFallback.ttf`: 3,451,900 bytes (1,650,174 gzip -9; the repository's packs were 15.3 MB), sha256 `21b96a03…0b0949`, the pin of `docs/third-party/fixture-fonts.md`. `assets/fonts/README.md` records source, licence and pin. Unmodified.
- `scripts/cjk-font.mjs` holds the pin and `readCjkFont()` (sync read, sha256 check, a loud error naming the file when it is missing or differs). `scripts/build.mjs` checks it before `build/` changes, so a Rust build without the font fails; a MuPDF build neither checks nor carries it.
- Chosen over a fetch-and-cache build step because:
  - a fresh clone builds offline, CI included; with a fetch, every new checkout and CI runner needs the network once, and `npm test` (which runs `npm run build`) would too, where the tests never download today;
  - the fixtures' source URL is a moving branch (`aosp-mirror/.../raw/master/...`): the pin would keep a changed file out, but a moved or removed one breaks every new checkout;
  - the source at a release tag is then the whole build input (M10's reproducible build instructions);
  - the cost is one 1.65 MB compressed blob in history, once.

## Wiring

- **Build** (`scripts/build.mjs`): Rust copies the font to `build/renderer/fonts/DroidSansFallback.ttf` and its licence (`docs/third-party/droid-sans-fallback-NOTICE.txt`, Apache-2.0 with the AOSP copyright) beside it; `build/renderer/fonts` is emptied on every build, so a MuPDF build after a Rust one carries neither.
- **Policy** (`src/policy.ts`): Rust only, `/fonts/DroidSansFallback.ttf` as `font/ttf`. The CSP is unchanged (`connect-src 'self'` covers the read). The licence file is not served; the package drops it from `build/` and ships it under `licenses/`.
- **Package** (`scripts/package.mjs`, Rust): `licenses/third-party/droid-sans-fallback-NOTICE.txt` and a `NOTICE.txt` line naming the font, its copyright and licence. The font itself is in `app.asar` (`build/renderer/fonts/`), staged because it is served.
- **Adapter** (`src/renderer/engine-rust.ts`): `engine.useCjkFont(route, read)` (`route`: the package's module-wide `set_cjk_font`, `cjk_font_wanted`, `has_cjk_font`; `read`: the font's bytes, synchronously, or null). After every dispatch, when the engine raised the cue, the adapter reads the font once, installs it and, when `set_cjk_font` returned true and the op is `render`, `thumbnail` or `compare-render` with an ok reply, dispatches the same request again and answers with that reply. Renders are not journaled, so repeating one is safe; no other op is repeated (one that raised the cue just installs the font for the next render). Before `redact-apply` the font is installed when none is (its pixel check draws pages whose pixels are not returned, so it cannot be repeated). A read that fails, throws or is refused settles the route: never read again, nothing drawn twice (the engine's latch covers a refusal; the adapter's covers a failed read, which the engine cannot see). A trap in `set_cjk_font` stops the app like a trap in an op (`reason: "worker"`). `renderMs`/`paintMs` are the whole dispatch, read and first draw included (what the user waited; the status bar shows it).
- No renderer-visible change: the render reply that raised the cue is replaced by the second draw before it leaves the worker.
- **Worker** (`src/renderer/worker-engine-rust.ts`): `read` is a synchronous `XMLHttpRequest` (`responseType` `arraybuffer`, allowed in a worker) for `./fonts/DroidSansFallback.ttf`, synchronous like the dispatch that asks; a failure warns once on the console. `createRustEngine({ RustApp })` is unchanged (the Electron trap test patches that call), the route is set after it.
- **Node** (`tests/rust-engine.mjs`, so `tests/run-rust.mjs` and every tool on it, `engine/tools/app-forms-oracle.mjs` included; `engine/tools/corpus-run.mjs`): `useCjkFont(wasm, () => readCjkFont())` from the same committed file, so tests and the corpus run the app's behaviour. The corpus runner records whether its sources were modified as before; a run without the font needs that line commented out (done for the "no font" column below, not committed).

## Lazy, not at startup (measured)

- Node, WASM release build: `readCjkFont` 2.8–4.2 ms (0.6–0.9 ms of it the read), `set_cjk_font` 41–43 ms, WASM linear memory 4.1 → 20.3 MB (wasm32 memory never shrinks; the input, the augmented font and the copies between them).
- Electron 44, macOS arm64, Rust build, first page through `--smoke --open`: a 200 × 120 pt page of two CJK characters 77–78 ms (read, install, two draws) against 2–4 ms for the same page in Helvetica; the tab's working set +20 MB (231 → 251 MB).
- At startup every worker would pay about 45 ms and 16–20 MB; on demand only a worker that meets such text does, once. The smokes on hello.pdf and form.pdf (source and packaged) never request the font.

## Engine font hook (`src/view/fonts.rs`, `with_total_cmap`): two defects the corpus showed

- **Advances of the appended glyphs.** Glyphs past `numberOfHMetrics` take the last long advance; Droid Sans Fallback's is 256 units, a full em, and the URW Latin glyphs appended at install sat past it. hayro stretches a CID font's fallback glyph to its /W width by the glyph's advance (`stretch_glyph`) and keeps one outline per glyph for the whole render, so Latin in a CJK CID font with /W drew at W/1000 of its width, every use of a glyph as squeezed as its first. Now every glyph has a long metric: the font's own keep theirs (short ones the last advance), the appended ones URW's advance at the font's units, the empty ones the last advance; `hhea.numberOfHMetrics` and `advanceWidthMax` follow. Cost: 2 bytes more per glyph that was short (Droid: 16,637 of 49,775, +33 KB).
- **Characters past the BMP.** The total cmap covered the BMP only, so a character past it that the font lacks drew as the glyph numbered like its CID (hayro's rule): a wrong character (corpus: a field appearance in pdf.js `issue19182.pdf`, sha256 `6e49be1a…`, drew a wrong Han glyph where MuPDF draws none). Now format 12 groups cover U+10000 to U+10FFFF: the font's own runs (its UCS-4 subtable; Droid maps 5 characters there), every other character in runs of at most 4,096 onto 4,096 appended empty glyphs (256 groups; loca, hmtx and vmtx +40 KB). Not added when fewer than 256 glyph ids remain under 65,535, nor for a font with more than 65,536 UCS-4 groups; such a font keeps the BMP-only map, as before.
- Bounds unchanged otherwise (32 MiB font, 2^20 cmap entries read). WASM 12,713,381 → 12,739,960 bytes.
- Tests: `view::fonts` unit tests `appended_glyphs_keep_urw_advances_in_long_metrics` and `characters_past_the_bmp_keep_the_fonts_glyphs_and_else_map_to_empty_ones`; `tests/view_cjk_font_metrics.rs` (own binary: the font is process-wide): 1 from the appended URW glyph as wide as in Helvetica with and without a /W entry, U+20000 draws nothing, the font's own glyph drawn; it fails at the parent on both counts. `a_truetype_cjk_font_gets_an_empty_glyph_for_what_it_lacks` (added by the view group in this rewrite) now counts the 4,096 empty glyphs and finds the BMP's empty glyph by id instead of as the last glyph; its other assertions are unchanged.

## App tests

- `tests/rust-cjk-font.test.mjs`: fake package (always run): one read, install and second draw on the cue, the reply being the second draw; thumbnail and compare-render draw again; another op installs without repeating; a failed render is not repeated; an unreadable (null or throwing) or refused font is not asked for again and nothing draws twice; `redact-apply` installs first, once; the reported time includes the read; a trap in the install stops the engine. WASM (Rust build or `test:rust`): a GB1 page without a program draws nothing and raises the cue without a route, then more than 1,000 ink pixels after one read, text unchanged; a CJK value set with `field-set` draws nothing, then more than 300 ink pixels in the widget; ink within 16 levels of MuPDF above 85% (97.55% measured). Electron (Rust build): the hello smoke requests no font; a CJK page opened and exported as PNG requests `fonts/DroidSansFallback.ttf` and the PNG has the glyphs.
- `tests/worker-build.test.mjs` (build-script test, added during this rewrite by the worker group): the scratch tree now also copies `scripts/cjk-font.mjs`, the font, its licence, `build/policy.js` and `build/renderer/limits.js` (a Rust build needs the font now); added: a Rust build has the font and licence with their hashes and serves the font as `font/ttf`, a MuPDF build has and serves neither, a missing or altered font fails the Rust build before `build/` changes. The worker graph test follows `new URL("./fonts/DroidSansFallback.ttf", ...)` in the Rust loader, so it also checks that the font is built and served.

## Measured

- Fixtures through the adapter (54 files, 82 pages, scale 1; `render` pixels, `nonWhite` and text hashed): parent commit without a route against this branch with it: byte-identical; no fixture raises the cue.
- Pixels against MuPDF (engine.ts) through the app adapter, ink within 16 levels (pixels either render darkens below 192), no font → font:
  - a GB1 page without a program (the test's): 0 → 97.55% (scale 1), 98.49% (scale 2); non-white 3,655 against MuPDF's 3,651.
  - MuPDF's own CJK fonts (`tools/app-view-cases.mjs` `mupdf-cjk.pdf`: zh-Hans, ja, ko, zh-Hant, a mixed run): 0 → 85.42% (scale 1), 89.13% (scale 2); whole page within 16 98.57 → 99.83% (scale 2).
  - `fixtures/cjk.pdf` (embedded subsets): 92.88% either way, renders identical.
  - Form values (`tools/app-forms-oracle.mjs`, which now runs with the font, scale 2, exact / within 16 over widget rects): text-intl (6 widgets) 96.49 / 97.16 (over-64 pixels 2,644; `app-view.md`, no font) → 96.85 / 97.86 (646); text 93.63 / 97.04, checkbox 93.85 / 96.97, combo 98.48 / 98.95, radio 83.60 / 89.33, list 79.80 / 80.42, signature 97.14 / 97.19 as before.
- Corpus (`corpus-run.mjs`, `--jobs 1 --rss-mb 1200`). Files drawing CJK text without its font: 203 candidates (pdf.js and veraPDF files naming a CJK collection or CMap in uncompressed bytes, 120; pdf.js `render-pixels` failures) opened on Rust without the font, first, middle and last page rendered; 16 raised the cue (pdf.js 13, veraPDF 3; none found in qpdf, pdf20examples, pyHanko, IRS, govdocs1; object streams were not scanned, so a lower bound).
  - Those 16, `render-pixels`: 13 in the baseline summary (426a2c6, an older engine that drew such codes as Helvetica), 16 on this branch without the font, 3 with it. `text-differs` and `textall-differs` 1 → 1. The 3 left: pdf.js `90ms_rksj_h_sample.pdf` (`d50ad3b9…`, below) and veraPDF `6-2-11-4-1-t01-fail-d` / `6-2-10-4-1-t01-fail-d` (`8c106bb2…`, `726f5b81…`, the same page: Latin in a CJK CID font, unmatched pixels 2,957 / 0 → 76 / 100, flat 55; the Latin face, below).
  - Regression sample, 441 other files (govdocs1 200, IRS 11, pdf.js 130, qpdf 40, veraPDF 40, pyHanko 20): 67 failing before and after; `render-pixels` 31 → 30 (govdocs1 `000262.pdf`), `info-differs` 0 → 1 (govdocs1 `000187.pdf`, `3443c4a4…`: Rust names the encryption algorithm `rc4-128` where MuPDF says `aes-128`; `open` is untouched here, so the change is in this branch's base since the baseline summary, not in this work), the other classes equal; 1 timeout as before.

## Differences from MuPDF and engine.ts

- The font is read on the first render that needs it (about 75 ms once per worker in Electron); MuPDF's WASM has it from startup. The reply that needed it is answered after the second draw.
- Latin, digits and symbols in a CJK font are URW Helvetica's (now at URW's advance) where MuPDF draws them from a Droid Sans-like Latin (`app-view.md`); two veraPDF test pages stay `render-pixels` for it.
- A CJK character past the BMP that the font lacks draws nothing, as in MuPDF; one the font has (5 in Droid Sans Fallback) draws.
- A Type0 font with `90ms-RKSJ-H` whose CID font has no /FontDescriptor: MuPDF ("missing font descriptor") reads the codes as one-byte Latin and draws Latin letters; Rust decodes the CMap and draws, and extracts, the CJK characters. Rust is right; the file stays `render-pixels` and `text-differs`.
- `redact-apply` installs the font before it runs (one read, even for a document without CJK text), so its pixel check sees CJK text as MuPDF would draw it.

## Open

- `set_cjk_font` grows WASM memory by 16 MB for a 3.5 MB font (copies while augmenting); building the augmented font in one buffer would cut that.
- hayro keeps one outline per glyph per render, so a glyph used with two /W widths in one render is stretched by its first (upstream).
- Packaging and the synchronous read are verified on macOS arm64 only (Electron 44); Windows and Linux packages are M10.
- `app-view.md` still lists the open item and the two superseded bullets; left unedited for a clean merge.
