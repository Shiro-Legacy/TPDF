# App view: corpus render fidelity (render-pixels)

October 15, 2026. Content differences the external corpus differential (`reference/corpus-20261015/README.md`, class `render-pixels`) found between the app's view and MuPDF 1.28.1, fixed in the view's private bytes (`src/app/view_fixes/*`, `src/app/annots/view.rs`) or recorded here. Read with `app-view.md` and `../APP-ENGINE-DESIGN.md` (View redefinitions). Corpus files are named by source, path and SHA-256 prefix only; every regression test builds its own minimal PDF.

## Owner decisions

### (i) Output intents and default colour spaces (`view_fixes/color.rs`)

- hayro ignores page-level /DefaultGray, /DefaultRGB and /DefaultCMYK and the document's output intent; there is no hook for them in hayro 0.8. The smallest route is the view's private bytes: each resources dictionary on the way names the space (`/TPDFDefaultGray`, `/TPDFDefaultRGB`, `/TPDFDefaultCMYK`), the content's device colour operators and inline images use the name, and images and shadings are redefined with the space. Saves never contain any of it (tested).
- What MuPDF maps (measured with synthetic pages through engine.ts):
  - DeviceGray, DeviceRGB and DeviceCMYK paint (`g`/`rg`/`k`, their stroking forms, `cs` of a device space), images (an Indexed base included), inline images, shadings, forms and annotation appearances.
  - A page's /Default* wins; a form's own /Default* applies inside it; the output intent fills only the families the page leaves at the device space (an explicit `/DefaultCMYK /DeviceCMYK` still gets the intent).
  - Only the first output intent counts; its profile's family must match its /N (else ignored). A CMYK intent maps DeviceCMYK only, an RGB intent DeviceRGB only, a gray intent DeviceGray only.
  - A profile whose device model is `sRGB` is left out: hayro's DeviceRGB is sRGB already.
- Measured: the synthetic CMYK intent case matches MuPDF within 2 levels in every family (`cmyk`/`rg`/`g`/`cs`); on the 462 render-pixels files, 245 left the class with this alone (first implementer's run).
- **Cost.** The rewrite reads every content stream, so it ran on every view: with an RGB intent, 300 pages of 13 MB content took 370 ms per view after any edit (4 ms without an intent) and 143 MB peak. Now:
  - `view_fixes::Memo` (held by `AppDocument`) keeps each stream's rewrite keyed by (stream, families named), forgotten when a change (end, abandon, undo, redo, and the open operation's pending objects before each view) touches the stream; only streams whose /Filter and /DecodeParms are direct are kept; at most 64 MiB. A view is the same with or without the memo: a kept entry charges the content budget with the decode's own length and total output.
  - A colour operator sets the named space only when the content has not already set it (fill and stroke tracked through `q`/`Q`, 4,096 saved states), so `r g b rg` becomes `r g b sc`; growth counts against the 16 MiB content budget.
  - Measured (native, release, loaded machine): first view 220-400 ms for that file (budget-bound at 16 MiB of content), a view after an edit 7-9 ms, 103 MB peak.
- Not mapped: a profile hayro cannot read (veraPDF 6-2-3-t01-fail-d's version 5 RGB profile: MuPDF/lcms2 reads it, the view names it and hayro falls back to device RGB); page transparency groups' blending spaces (below).

### (ii) Glyph weight (no change)

- Fills and glyph outlines cover the same area in both engines: total darkness of synthetic Helvetica and Times text at 7, 10 and 24 pt and of filled rectangles agrees within 1% at scales 1 and 2.
- Glyph positions: MuPDF quantizes a glyph's origin (1/4 px below about 24 px, 1/2 px to about 48 px, whole pixels above; measured by moving one glyph in 0.05 px steps), hayro places it exactly. Edges move by up to 1/8 px, red and blue on either side of each stem in a difference image, not heavier.
- Thin strokes are what looks heavier: hayro (`hayro::path::set_stroke_properties`) widens any non-text stroke below 1 device pixel to 1 pixel (the specification's minimum for width 0; Acrobat's thin-line enhancement does the same); MuPDF draws the true width with a floor of about 0.2 px. Darkness per unit length at scale 1, MuPDF / Rust: width 0 0.20 / 1.00, 0.25 0.27 / 1.00, 0.5 0.54 / 1.00, 0.75 0.80 / 1.00, 1 and above equal; at scale 2 the same from width 0.5 down. Table rules and form boxes on IRS and govdocs1 pages are such strokes.
- Neither can change without patching hayro: its renderer device is private and the threshold is a constant. Recorded as a rasterization difference, not a defect.

## Fixes

- **Form resources fall back on the enclosing ones** (`inherit.rs`): MuPDF looks a name up in the form's resources, then each caller's, out to the page's; hayro only in the form's. The view adds the names a form's content uses that only enclosing resources have (qpdf form-xobjects-some-resources1, 360d6901240c). Bounds: 1 MiB of form content scanned, 4 Mi enclosing names looked up per view.
- **Tiling cells larger than their step** (`tiling.rs`): MuPDF draws the whole /BBox at every step; hayro one step-sized tile. The view redraws such a pattern as a step-sized tile drawing the cell at every offset that reaches it (veraPDF 6-2-9-t06 and 6-2-10-t06 drew blank). Bounds: 4,096 copies and 8 MiB of copied work per pattern, 65,536 copies per view.
- **Embedded CMap writing mode** (`cmap.rs`): MuPDF takes /WMode from the CMap stream's dictionary, hayro from the program; the view writes the dictionary's into the program.
- **Fixed-size annotations** (`annots/view.rs`): a Text note or a NoZoom annotation is drawn at its /BBox's size from the top left of /Rect whatever its /Matrix scales (qpdf test80a1, a163d0be31ce).
- **CID fonts without a program** (`cidfont.rs`): outside the CJK collections hayro finds a glyph through the font's Unicode, else takes the substitute face's glyph numbered like the CID: wrong letters (pdf.js issue5801, ThuluthFeatures, issue19550, issue13916, issue15977, issue20453, issue6127). MuPDF finds the glyph through /ToUnicode or, without one and without a /CIDToGIDMap stream, reads the CID as an index into the standard Macintosh glyph order (measured: CIDs 0-257 give exactly that order's Unicode, 258 and above nothing); it draws nothing for a CID it cannot place. For /Identity-H and /Identity-V fonts the view gives the descendant a /CIDToGIDMap placing no CID and, when the font has neither, a /ToUnicode of the Macintosh order (one stream each per view, shared). Text extraction reads that /ToUnicode too and gives MuPDF's text.
- **Transfer functions** (`gstate.rs`): MuPDF ignores /TR and /TR2 when painting (measured with an inverting and a squaring function on rg, g and k fills); hayro applies them, inverting fills. The view drops them from every graphics state the walk meets; a soft mask's /TR stays. Ten files: Isartor 6-2-8 (4), veraPDF 6-2-5 (4), pdf.js transfer_maps, issue6931_reduced.
- **Invisible annotations** (`view_fixes.rs`): MuPDF (and pdf.js) leave out an annotation whose /F sets Invisible whatever its type; the specification draws a standard type. The view now marks it Hidden like NoView (veraPDF 6-3-2-t02-fail-a, Isartor 6-5-3-t02-fail-d, 8.9.2.2-t01-fail-a). This was app-view.md's open decision; matching the app's MuPDF engine keeps what users saw.
- **Run-length data without its end byte** (`runlength.rs`): MuPDF decodes the runs it finds; hayro refuses the stream. The view appends the end byte to an image whose only filter is RunLengthDecode when every run is whole (govdocs1 000955 lost its logo). Charged to the view's 32 MiB copy budget.
- **Walk cost**: the resources walk went through an /Annots array shared by many pages once per page (4,000 pages sharing 4,000 annotations: 1.9 s per view, now 15 ms).

## Differences from MuPDF (recorded, not changed)

Rust better or equally valid:

- JBIG2 with refinement, halftone and text-region refinement (22 pdf.js `bitmap-*` files, issue20439): hayro-jbig2 draws the intended image; MuPDF draws black or noise.
- Damaged qpdf test files (start, incremental-3, split-content-stream-errors, stream-line-enders, xref-compressed-in-compressed, fuzz 69969/69977e/71624, shared-images-errors-2-out, pyHanko broken-objstream4): Rust draws the page content MuPDF loses.
- CalRGB with its white point and gamma (pdf.js calrgb): hayro converts colorimetrically; MuPDF draws the values as DeviceRGB.
- Link annotation appearance streams are drawn (veraPDF 6-3-2-t01-fail-b, pdf.js zerowidthline has none); MuPDF never draws a Link annotation.
- A form translated by -3.4e38 (TWG A019-pdfa2-pass-b) is off the page; MuPDF draws it in place.
- A broken image MuPDF draws as a band of noise (govdocs1 000208).
- An empty signature field has no "SIGN" marker (pyHanko minimal-*, simple-acroform-states, veraPDF 6-9-t01-fail-b, 7.18.1-t03-pass-f): the known intentional difference.
- Check boxes draw their /MK /CA character (qpdf field-types family); MuPDF 1.28 always draws its own check mark (measured with /CA 8, 4 and l).
- A list box draws its selected option on a highlight (as Acrobat's appearances do); MuPDF 1.28 draws none.
- A widget without a field type and its stored appearance is drawn (pyHanko separate-annots-kids-indir); MuPDF draws nothing.

Not matched (hayro or other pieces):

- Knockout groups (pdf.js knockout_* 5 files) and page transparency groups with a non-RGB blending space (veraPDF 6-2-9/6-2-10 t01-g and t03-g, 6-2-4-3-t04 m/n/p/r, 6-2-4-5-t01-c, 6-2-3-3-t03-pass-k, 6-2-4-3-t02-pass-d): MuPDF composites in the group's space (a gray group turns the page gray); hayro composites in sRGB and has no knockout.
- CJK text of fonts the file does not embed draws nothing in the corpus run, which installs no CJK font; with Droid Sans Fallback installed (`set_cjk_font`) 9 of the 12 pdf.js files of the CJK collections match.
- Times-Bold, Times-Italic, Courier and the other Foxit faces lack glyphs URW has (pdf.js issue5244), as app-view.md records.
- Fills thinner than a pixel and glyph origin quantization (decision ii); unencodable FreeText characters draw as `?` (pdf.js freetext_no_appearance; MuPDF lays the text out with a script font its WASM build lacks); NeedAppearances text synthesis in the qpdf need-appearances family (the value's baseline and its non-WinAnsi characters; a synthetic TrueType or Helvetica field matches MuPDF exactly).
- FileAttachment annotations without an appearance get no icon (veraPDF 6-3-3-t01-fail-a, 8.9.2.4.10-t01-fail-a); MuPDF synthesizes a paper clip.
- A hex string with a non-hex character (veraPDF 6-1-5-t02, 6-1-6-t01/t02) loses its show operation in hayro; MuPDF skips the character.
- A JBIG2 stream that starts with a file header (pdf.js jbig2_file_header) is not decoded by hayro.
- Truncated image data: MuPDF pads the missing rows with zeros (black), hayro leaves them undrawn (qpdf png-filters-*, bad-jpeg).
- A substitute face is not stretched to a CID font's default width (/DW without /W; pdf.js issue15441, issue15443).

## Found, not this piece's

- hayro-interpret expands a CID font's /W and /W2 ranges (`c_first c_last w`) into a map entry per CID with no bound (`font/cid.rs` `read_widths`): `[0 4294967295 w]` asks for 2^32 entries, and many full-range entries cost their product in time. Nothing in the view guards it (left to the robust piece).

## Measured

- Corpus, the 462 files of the baseline's render-pixels list (`--jobs 1`, scale 1): 459 render-pixels on this branch's base (f0945be, its own WASM), 192 after; no page lost agreement (within 16 levels, per page, ±0.05%); no file joined the class. Sub-clusters (first rule that matches, base → after): output intent or default spaces 215 → 25, form widgets 39 → 30, other annotations 36 → 10, CID fonts without a program 25 → 18 (11 of them CJK, matching with the CJK font installed), images 18 → 15, page group blending space 17 → 11, transfer functions 10 → 0, tiling cells 10 → 2, JBIG2 27 → 27, unsigned signature fields 14 → 14, damaged qpdf files 9 → 9, knockout 7 → 7, other 32 → 24.
- Regression sample of 591 other files (all govdocs1 and IRS, 200 pdf.js, 100 veraPDF, 60 qpdf, 20 pyHanko, 5 pdf20examples; seeded sample outside the 462), base and after: no file worse in any class; render-pixels 2 → 1, text-differs 34 → 32, textall-differs 40 → 38 (the Macintosh-order /ToUnicode); per page, 2 better and none worse.
- Fixtures through the app's Rust adapter at scales 1 and 2 (82 pages, large.pdf sampled): byte-identical before and after.
- View cost (native release, medians): 300 pages of 13 MB colour-changing content with an RGB output intent, a view after an edit 370 ms → 9 ms; 4,000 pages sharing a 4,000-annotation /Annots, first view 1.9 s → 15 ms. Both have regression tests (`tests/app_view_corpus.rs`), timing-based with wide margins.
