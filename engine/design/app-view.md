# App view: rendering fidelity (fonts, inherited page attributes, page list, cost)

October 15, 2026. Rendering gaps the op groups recorded, fixed in `src/view/` and the app's view overlay (`src/app/{document,view_fixes,view_depth,info}.rs`). Read with `../APP-ENGINE-DESIGN.md` (View ops, View redefinitions). Evidence: `reference/app-view-20261015/`.

## Fonts (`src/view/fonts.rs`)

- hayro asks a font resolver (`InterpreterSettings::font_resolver`) for every font whose program is not embedded: `FontQuery::Standard` for a base 14 face (simple fonts always end there), `FontQuery::Fallback` for a CID font. The default resolver gives the Foxit fonts hayro embeds, which have Latin glyphs only, and a Foxit Type1 font for a CID fallback, which hayro cannot load as one: the CID font then fails and hayro shows its two-byte codes as one-byte Helvetica text (garbage, measured on MuPDF's own CJK fonts).
- `view::fonts` gives render, structured text and glyph measurement (`view::Measurer`, the edit ops' width and missing-glyph checks, which must see the fonts the page draws with) one resolver:
  - **URW faces.** Helvetica, Helvetica-Bold, Helvetica-BoldOblique and Times-Roman are built from the engine's URW outlines (`builtin_outlines`, the fonts MuPDF draws these faces with) into OpenType fonts with CFF outlines, once per process (`OnceLock`): the CFF program (Type 2 charstrings, no hints), advances (hmtx), names (post 2.0) and a format 4 Unicode cmap. hayro finds a glyph by its name and else through the Adobe Glyph List and the cmap, so MuPDF's `/Acyrillic` finds URW's `afii10017`. The Unicode of each URW glyph name is a generated table (`src/view/fonts_unicode.rs`, `tools/generate-view-font-unicode.py`, from the AGL hayro-interpret 0.8.0 embeds; 653 names).
  - **Helvetica-Oblique** is Helvetica slanted by 12°: URW's Nimbus Sans Italic is that slant (its Latin against MuPDF moves as Helvetica's does; integer baselines: 71.3% of ink pixels within 16 levels, Helvetica 72.8%, Foxit's italic 44.5%).
  - The other faces (Courier ×4, Times-Bold, Times-Italic, Times-BoldItalic, Symbol, ZapfDingbats) keep hayro's Foxit fonts: Latin only.
  - **CJK.** A CID font of the Adobe-GB1, CNS1, Japan1 or Korea1 collection gets the font the host installs with `view::set_cjk_font` (bridge `set_cjk_font(bytes) -> bool`), process-wide, from the next render on. Without one it gets a CFF font with only .notdef: nothing is drawn, the codes are read as the CMap says, and the text is still extracted (ToUnicode, else the collection's UCS2 CMap); a render then sets `cjk_font_wanted()` (bridge `cjk_font_wanted()`), the host's cue to install the font and render again. Text extraction and measurement never set it.
  - An installed TrueType font is augmented once at install: every BMP character it lacks that URW Helvetica has gets that glyph appended (cubic outlines as two quadratics each, scaled to its units per em), every other BMP character but the surrogates and U+FFFF an appended empty glyph, in a new cmap of one format 12 subtable. hayro draws a character the font's cmap lacks as the glyph numbered like its CID, a wrong one ("日 1Ab" drew "日 ㅊ뻥ㄹ꿻と" with Droid Sans Fallback), and MuPDF draws Latin, digits and symbols in a CJK run from its other fallback fonts. Bounded: at most 2^20 cmap entries read, 32 MiB font.
  - Other CID fonts without a program get the base 14 face hayro picks from the descriptor (URW where built). Their characters map through Unicode; one the face lacks still draws as the glyph numbered like its CID (hayro's rule), as before where it drew Latin garbage throughout.
- **CJK decision: an optional asset, installed on demand.** MuPDF 1.28.1's WASM (`node_modules/mupdf/dist/mupdf-wasm.wasm`, 10.4 MB) embeds one CJK font, Droid Sans Fallback (Apache-2.0; 3,451,900 bytes, sha256 `21b96a03…0b0949`, the same file as `fixtures/.fonts/DroidSansFallback.ttf`), and draws every CJK collection with it. Routes measured:
  - embed it in the engine's WASM: +3,451,900 bytes (WASM 11.9 → 15.3 MB) and the same in every worker's memory, used or not;
  - a subset: Droid Sans Fallback averages 70 bytes per glyph (glyf, loca, hmtx, vmtx), so the 7,500 commonest Han, kana and Hangul would add about 0.5 MB to the WASM, and any other character a non-embedded CJK font shows (every collection CID, any ToUnicode) would still be missing;
  - refuse to draw: the app's own CJK form values (forms group, MuPDF's `pdf_add_cjk_font` fonts) would be invisible;
  - **an asset loaded on demand** (chosen): no bytes in the WASM; the packaged file is 3.45 MB (1.65 MB gzip, 1.30 MB brotli) and costs memory only in a worker that met CJK text without a font. After install the process holds the augmented font (about 3.9 MB: the font, 653 glyphs, a cmap of 12 bytes per run of characters). Installing it costs 6 ms natively (read and augment).
  - Integration is the worker's (decision needed there): ship `DroidSansFallback.ttf` beside the WASM (its NOTICE as `docs/third-party/fixture-fonts.md` records), and after a `render`, `thumbnail` or `compare-render` reply, when `cjk_font_wanted()` is true and the font is not installed, fetch it, `set_cjk_font(bytes)` and dispatch the render again (renders are not journaled, so repeating one is safe).
- WASM growth for all of this work: +113,868 bytes (11,824,363 → 11,938,231).
- First use of a face builds it (natively about 3 ms per face, once per process); each render then parses the faces it uses (hayro keeps no font across renders), about 0.1 ms more than Foxit's Type 1 on hello.pdf (0.57 → 0.66 ms).

## Page attributes inherited through /Parent (`view_fixes`, `info::walk_tree`)

- MuPDF (and pdf.js, PDFium) reads a page's /Resources, /MediaBox, /CropBox and /Rotate through its /Parent chain; hayro carries them down the /Kids path it walks. Everything else in the engine (page size, `PageSpace`, stamps, forms) already follows /Parent (`pagetree::inherited`, `info::page_box`).
- `info::walk_tree` is the page walk `info::pages` uses, now with each page's path of /Pages nodes and where its dictionary is written (`PageSlot`: its own object, an entry of a /Kids array in a node or array object, or nested in a direct node). Where a page's /Parent chain is not the path (a direct or foreign /Parent, none at all, a cycle), the view writes MuPDF's values onto the page in its private update: /Resources (`<< >>` when not a dictionary), /MediaBox, /CropBox (the MediaBox when absent), /Rotate snapped to a quarter turn (`info::display_rotation`, what the view's frame uses). Pages whose chain is their path cost one comparison per ancestor and write nothing.
- Pages written directly into /Kids are now edited in the view through their slot (Letter MediaBox, inherited attributes, their direct annotations), where they were skipped.

## The page list without a page tree (`info::pages`, `view_fixes`)

- Without a usable tree (no /Pages dictionary with a /Kids array) `info::pages` lists the /Type /Page dictionaries in object order; it listed every dictionary with /Contents (hayro's brute force), annotations included. The view's update then points the catalog at a new /Pages node listing exactly those pages, so hayro walks a tree and both lists agree by construction (hayro alone orders by file offset, and fails to open a file where it finds none).
- MuPDF 1.28.1 lists no pages in that case (measured: "malformed page tree", or 0 pages; its repair rebuilds no page tree).

## Cost (`view_depth`, `forms::view`, `ViewDocument::open_prepared`)

- **Shared /Annots.** The view build was quadratic when pages share one /Annots array: `ViewDocument::open` hid NoView, Popup and /OC-off annotations again by walking every page's annotations in hayro (pages × items dictionary parses), and the forms view walked every page's widgets (and synthesized a widget listed on several pages once per page). `view_fixes` already marks those annotations Hidden wherever they are stored, so the app's view opens with `ViewDocument::open_prepared` (no second pass; `open` keeps it for other bytes), and the forms view handles an /Annots array object, and a widget, once.
- **Depth.** `view_depth` walked every object on every view rebuild. `AppDocument` now keeps the set of objects nested past hayro's 64 levels (`view_depth::DeepObjects`): one walk on the first view, then only the objects each change (end, abandon, undo, redo) and the open operation touched.
- Measured natively (release, M-series, loaded machine; medians of 5; `examples/view_cost_probe.rs`: open, first view, render, a note added, view, undo, view):

| file | first view | view after an edit | after undo |
| --- | --- | --- | --- |
| large.pdf (500 pages), before | 1.0 ms | 1.1 ms | 1.1 ms |
| large.pdf, after | 1.0–1.4 ms | 1.1 ms | 1.0 ms |
| 1,000 pages sharing a 1,000-item /Annots, before | 1,700 ms | 1,585 ms | 1,603 ms |
| same, after | 3.7 ms | 3.5 ms | 3.9 ms |
| 4,000 sharing 4,000, after (the annots group measured 8.4 s before) | 19.7 ms | 18.8 ms | 12.8 ms |
| 300,000 small objects, before | 96 ms | 77 ms | 92 ms |
| same, after | 75 ms | 11 ms | 12 ms |

  What remains after an edit is proportional to the file: the base copy into the view's bytes and hayro's xref parse (11 ms for the 23 MB, 300,000-object file).

## Parity re-measured (`reference/app-view-20261015/summary.json`)

- All 54 fixtures through the app's view (`tools/compare-app-view.mjs`, `TPDF_VIEW_APP=1`, 72 DPI; large.pdf pages 1, 2, 3, 251, 500), before (parent commit) and after: 82 pages, 77 changed, none worse; ink pixels within 16 levels of MuPDF 57.3% → 71.9% on average (exact 19.7% → 23.9%; hello.pdf 51.5% → 96.7%); whole-page figures as before (76 of 77 pages at least 97% exact, objects6.pdf page 1 88.1%). The five pages unchanged use embedded fonts only. Structured text (plain, spans, line summaries) is byte-identical on every page of every fixture.
- Generated pages (`tools/app-view-cases.mjs`), ink within 16 levels: Helvetica 40.4 → 46.7%, Helvetica-Bold 49.0 → 57.3%, Helvetica-Oblique 36.8 → 46.0%, Helvetica-BoldOblique 47.6 → 56.0%, Times-Roman 43.5 → 44.8% (Latin on fractional baselines; the other faces unchanged); MuPDF's Greek and Cyrillic fallback fonts 10.8 → 58.0% (pixels drawn 4,158 → 31,348 of MuPDF's 43,737: the four URW faces and Helvetica-Oblique draw, Times-Bold, Times-Italic and Courier still do not); MuPDF's CJK fonts 6.5% (garbage) → 5.3% (nothing) without the font, 85.9% with it.
- The forms group's widget scenarios (`tools/app-forms-oracle.mjs`, exact / within 16 over widget rects): text-intl 96.12 / 96.51 → 96.49 / 97.16 (over-64 pixels 4,803 → 2,644), 96.85 / 97.86 (646) with the CJK font; text 93.45 / 96.87 → 93.63 / 97.04; combo 98.27 → 98.48; list 79.61 → 79.80; the others unchanged.

## Tests

- `tests/app_view_fidelity.rs`: resources inherited through a direct /Parent drawn; boxes from /Parent decide the clip (and /Rotate); a page without /Parent inherits nothing; the page list without a tree (annotation not a page, object order against file order, the view's list equal, no page at all); Cyrillic and Greek in each face (drawn in the URW five, not in Courier and Times-Bold) and extracted; a deep object an edit creates is cut in the view after the step, after undo, and inside an open operation (each path checked by disabling it). All fail at the parent commit.
- `tests/view_cjk_font.rs` (its own binary: the font is process-wide): nothing drawn and the text extracted without the font, `cjk_font_wanted` from renders only, refusal of bytes that are not a font, then the square glyph of a generated TrueType font, `1` from URW Helvetica, and nothing for a character neither has.
- Unit tests in `view::fonts`: charstring numbers, every face builds, the cmap reads back (`Acyrillic`'s U+0410 → `afii10017`), the augmented CJK font's glyph count, offsets and cmap.
- `tests/app_misc.rs` "a dictionary the brute-force page list takes for a page is not stamped" (added by the misc group) relied on the old fallback listing an annotation; it now lists the annotation in /Kids, where the walk still takes it for a page, and is renamed "a dictionary the page list takes for a page is not stamped". Its assertions are unchanged.

## Differences from MuPDF and engine.ts

- Courier (all four), Times-Bold, Times-Italic and Times-BoldItalic draw no Cyrillic, Greek or other non-Latin glyphs (hayro's Foxit fonts), and their Latin in Foxit's shapes; MuPDF draws them from URW's. The engine carries URW outlines for four faces only; MuPDF's URW CFF programs for those seven faces are 372 KB (Symbol and ZapfDingbats need no others).
- A non-embedded base 14 font without /Widths draws characters hayro's AFM metrics do not name (Cyrillic, Greek) with no advance, one over another; MuPDF advances by the URW glyph's width. Every font the app writes has /Widths.
- Helvetica-Oblique is Helvetica slanted 12°, URW's own italic being that slant (points rounded to the 1/1000 em grid).
- CJK CID fonts without a program draw nothing until the host installs the CJK font (above); MuPDF has it built in.
- With the CJK font installed, characters it lacks are drawn from URW Helvetica (appended glyphs at the font's 1/256 em grid for Droid Sans Fallback); MuPDF draws them from its other fallbacks (a Droid Sans-like Latin), so shapes differ. Characters outside the BMP that the font lacks still draw as the glyph numbered like the CID (hayro's rule; Droid Sans Fallback maps 5 characters past the BMP).
- A page whose /Parent chain differs from its /Kids path inherits through /Parent as MuPDF, pdf.js and PDFium read it, including a page without /Parent (nothing inherited: Letter, no resources), where hayro alone inherited along /Kids.
- Without a usable page tree the pages are the /Type /Page objects in object order; MuPDF 1.28.1 shows none.
- A page inside a /Pages node written directly into /Kids keeps hayro's inheritance and its direct annotations are not edited by the view (no object holds it to redefine; page edits normalize such trees first).
