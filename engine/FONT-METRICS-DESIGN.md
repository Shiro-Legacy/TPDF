# Bounded horizontal simple-font advances

This is a read-only design and inventory. No runtime metrics implementation is authorized by this document. The proposed unit supplies advances for original one-byte PDF character codes; it does not select glyphs, parse embedded fonts, place Unicode scalars, perform text layout, or establish extraction/rendering correctness.

## Measured page-bound workload

`tools/font-metrics-inventory.mjs` reads only the 53 simple-font identities registered in inherited page Resources, verifies original fixture hashes and pinned MuPDF 1.28.1, and records PDF declarations plus raw AFM data in `reference/font-metrics-inventory.json`. No font was loaded, substituted, saved or interpreted. This pass records zero native diagnostics and zero errors. The appearance/Form-only font excluded by F0 stays excluded here.

| Measurement | Result |
| --- | ---: |
| Page-bound simple-font identities | 53 |
| Fonts with explicit Widths | Five embedded TrueType fonts |
| Width entries | 160, all finite integers, range 0–852 |
| Explicit zero widths | 90 |
| Exact Widths length = LastChar − FirstChar + 1 | All five |
| Declared MissingWidth | None |
| Fonts without Widths | 48 Type1: 47 Helvetica, one Times-Roman |
| Width-bearing fonts with / without ToUnicode | Three / two |

The five explicit arrays are:

| Fixture and font | FirstChar–LastChar | Entries | Unicode declaration |
| --- | --- | ---: | --- |
| p0b/edit-coded.pdf 5:0 | 1–14 | 14 | Strict ToUnicode |
| p3/simple-subset.pdf 3:0 | 65–80 | 16 | Explicit WinAnsi encoding |
| p6/quartz.pdf 6:0 | 33–71 | 39 | Strict ToUnicode |
| p6/quartz.pdf 10:0 | 33–33 | One | Strict ToUnicode |
| p6/quartz.pdf 14:0 | 32–121 | 90 | Explicit MacRoman encoding |

The 90 zero entries are present array values, not holes. Never replace zero with MissingWidth, a space width, an AFM value, or a font-program advance. A code's advance is independent of the number of Unicode scalars to which ToUnicode maps it: a ligature expands the returned Unicode while retaining one character-code advance. Combining marks can have their own explicit zero/nonzero code advance; normalization must not change the source-code sequence.

The two explicit-encoding TrueType fonts are Hebrew subsets. As already recorded by F0, their PDF encoding/name-derived Unicode is not proof of the embedded glyph semantics. Their declared widths remain usable as PDF advance declarations without making that stronger claim.

Reproduce:

```sh
node tools/font-metrics-inventory.mjs \
  /Users/owner/Documents/TPDF/node_modules/mupdf/dist/mupdf.js \
  ../fixtures /private/tmp/tandem-mupdf-reference/mupdf-1.28.1-source \
  reference/font-metrics-inventory.json
```

## Pinned behavior and what must remain distinct

`source/pdf/pdf-font.c` loads simple-font widths from FirstChar, LastChar and Widths. It converts each array entry to an integer, recovers invalid endpoints by setting both endpoints to zero, and reads expected entries even when the array is short. Its default horizontal width comes from FontDescriptor/MissingWidth, whose absence produces zero in the inspected font initialization/descriptor path. A strict implementation should preserve numeric declarations and reject malformed ranges/counts instead of importing those recoveries.

When Widths is absent, MuPDF calls `ft_width` for all 256 codes after selecting an embedded/builtin/substitute font and constructing its code-to-glyph mapping. `ft_width` uses FreeType's unscaled advance and integer `advance * 1000 / units_per_EM`, with a fallback units-per-em value. That general behavior cannot be reproduced merely by recognizing a BaseFont string. It requires glyph selection and a font program, which are outside this engine stage.

`source/pdf/pdf-metrics.c` stores horizontal ranges and a default advance. This supports separating explicit range values from the default. MuPDF's metric structures use integer widths; that is a compatibility limit, not a reason to truncate a PDF real number in the safe API. Generated fractional tests must compare to independent numeric expectations and record native integer conversion separately.

`source/fitz/noto.c` recognizes exact Base14 names; `font-table.h` associates Helvetica with NimbusSans-Regular and Times-Roman with NimbusRoman-Regular in this pinned build. Those are concrete bundled substitutes, not universal operating-system font choices or all possible PDF font behavior.

## M0: explicit numeric declarations first

The smallest independently useful unit is immutable, bounded, horizontal simple-font advances with an explicit MissingWidth/default policy:

```rust
struct FontAdvance { /* finite private f64 in 1/1000 text-space units */ }
struct HorizontalMetrics { /* first/last, at most 256 advances, missing advance */ }
enum AdvanceOrigin { WidthsEntry, DeclaredMissingWidth, DefaultMissingWidth }
struct AdvanceLookup { /* FontAdvance plus AdvanceOrigin */ }

FontAdvance::new(value: f64) -> Result<FontAdvance, MetricsError>;
FontAdvance::value(self) -> f64;
HorizontalMetrics::from_widths(
    first: u8, last: u8, widths: &[FontAdvance], missing: Option<FontAdvance>
) -> Result<HorizontalMetrics, MetricsError>;
metrics.lookup(code: u8) -> AdvanceLookup;
```

Fields stay private and returned values immutable. Preserve signed finite advances, including zero, negative advances and fractional values. Do not force advances to positive geometry dimensions. Object integers outside the exactly representable f64 range should refuse rather than silently round; preserve finite real values already validated by the object parser. Arithmetic that applies font size, scaling or a text matrix belongs to a later checked execution layer. A raw finite width does not promise every subsequent multiplication is finite.

Require FirstChar and LastChar to be integers in 0..255, first <= last, and Widths to contain exactly the inclusive count. Cap before allocation and reject nonnumbers, streams, nested arrays and unresolved references in the standalone constructor. The document adapter resolves permitted indirect references under its own budgets before constructing values; it does not clone the Object tree. Preserve Widths array index, object identity and dictionary field in errors.

For codes inside the range, return the declared entry even if zero. For codes outside the range, return declared MissingWidth when present, otherwise explicit `DefaultMissingWidth` zero. MissingWidth is a descriptor entry; do not look for a same-named font-dictionary key or infer it from FontBBox, an embedded `.notdef`, or a Base14 table. A non-null FontDescriptor must resolve to an ordinary dictionary, and a non-null MissingWidth must be a finite number. FontDescriptor absence is permitted for this mapping/advance stage; this is not validation that a complete rendering font is conformant.

A document adapter should reject incomplete non-null width declarations: a Widths array missing valid endpoints, wrong endpoint/count types, or endpoints supplied without Widths. A missing/null Widths value is a distinct `NeedsBuiltinOrFontProgramMetrics` condition until M1. Do not synthesize widths from Unicode, multiply widths for a multi-scalar ToUnicode destination, or copy another font's widths by BaseFont similarity.

M0 acceptance can cover all five explicit-width fonts and all 160 declared values immediately, plus all 256 code lookups per font including out-of-range default zero with provenance. The other 48 fonts remain explicitly pending builtin support. Keeping this first gate small avoids claiming a general font implementation from five valid arrays.

## M1: narrowly pinned builtin metrics

A separate stage can cover the 48 measured Widths-absent fonts with exact `/Subtype /Type1`, exact `/BaseFont /Helvetica` or `/Times-Roman`, no embedded font program, and an explicit supported glyph-name encoding. Do not silently add Arial aliases, subset-prefix stripping, style guessing or all 14 standard fonts. More Base14 families/styles can be added after equally pinned table/native coverage; the measured workload requires only these two regular faces.

The inspected archive includes `resources/fonts/urw/input/NimbusSans-Regular.afm` and `NimbusRoman-Regular.afm`, each with 855 unique named character metrics. Their full hashes and copyright strings, plus bundled CFF hashes and implementation source hashes, are recorded in the report. Both AFMs cover every named WinAnsi, MacRoman and StandardEncoding slot. Each lacks 135 of the MacExpert names, so a generic promise of every S0 encoding would be false.

Useful independent examples, in units per 1000:

| Glyph name | Helvetica/NimbusSans | Times-Roman/NimbusRoman |
| --- | ---: | ---: |
| space | 278 | 250 |
| A | 667 | 722 |
| B | 667 | 667 |
| i | 222 | 278 |
| fi | 500 | 556 |
| bullet | 350 | 350 |
| Euro | 556 | 500 |
| .notdef in the AFM | 278 | 250 |

Map a PDF code through its **glyph-name encoding**, then perform an exact byte-name metric lookup. The AFM's legacy `C` integer is not a WinAnsi byte table. Never go code -> ToUnicode string -> Unicode -> guessed glyph width: ligatures, aliases and alternate glyphs make that incorrect. Apply explicit Differences using S0 semantics before selecting the named metric. Known glyph names with no AFM metric return `MissingBuiltinMetric`; an undefined or `.notdef` encoding slot remains an explicit `UndefinedGlyph` outcome. Do not silently use the AFM .notdef/space width for those slots. Native may select glyph zero and return a width; record that recovery separately.

A font can have both ToUnicode and Encoding. ToUnicode remains authoritative for Unicode, while Encoding independently determines builtin glyph-name metric selection. F0 currently exposes only its chosen Unicode mapping branch, so a metrics adapter must independently resolve the explicit Encoding when needed; it must not treat the presence of a ToUnicode branch as proof that glyph-name encoding is available. Reuse the bounded encoding-resolution helper under loader budgets, or introduce a separate validated encoding declaration field. Missing explicit glyph encoding remains `NeedsFontBuiltinEncoding` even when ToUnicode is valid. No such ambiguity occurs in the 48 measured Base14 fonts, all of which use WinAnsi.

Embedded font data, nonstandard BaseFont names, TrueType Widths absence, symbolic selection, Type0/CID and Type3 fonts remain outside M1. Simply ignoring an embedded FontFile/FontFile2/FontFile3 while using a same-named builtin would change the intended advance source. Inspect descriptor/program presence without decoding programs and refuse that unsupported path.

Before adding runtime tables, require two independent sources of expected values: parse the pinned AFM glyph-name/WX pairs and query the pinned native CFF face by the same glyph names. A test-only native oracle should call the explicit bundled-face loader, use glyph-name lookup and unscaled advance; it must report missing glyphs and units-per-em rather than making them zero silently. The initial inventory reads raw AFM only. A subsequent read-only native comparison is now recorded below; it establishes parity only for the exact shared-name subset. General native PDF font loading may additionally validate normal fixture behavior, but substitution/Unicode recovery must not define the strict table semantics.

Any generated data must retain its exact source hash and the AFM URW 2014 notice, along with applicable bundled-source licensing/provenance and existing engine attribution. This proposal copies no font programs into the engine and does not relicense bundled data as newly authored Rust. Keep full notices with any later vendored metric subset; do not derive legal permissions from the existence of similar public AFMs.

## Document integration, ownership and budgets

Add metrics as a separate immutable result/cache keyed by F0 FontIdentity inside the document-bound session, or use a separate document-bound metrics loader sharing validated identity provenance. Do not attach advances to Unicode scalar indexes. Preserve original code bytes through `DecodedText.codes()`; each one-byte code's advance is requested once regardless of the scalar count in its UTF8 span.

M0 stores at most 256 finite advances plus default/provenance per font. M1 can use static named tables and a validated 256-code outcome array. Sharing a font through resource aliases must not duplicate those arrays. Existing `load` mapping behavior should remain independent; adding metric support must not silently make F0 validate unrelated rendering fields or reject previously accepted declared Unicode mappings.

Suggested new session counters are metric load attempts (100,000), distinct retained metric objects (4,096), processed width entries (1,048,576), and retained numeric entries (1,048,576). Reuse F0's cumulative reference and metadata-work budgets for endpoints, Widths, descriptor/MissingWidth, builtin embedding checks and encoding/Differences references. Count failed arrays and repeated overwritten Differences as work; cache hits can avoid reparsing but still charge selection calls. Cap a standalone Widths array at 256 before cloning or allocation. Bound source-name comparisons and dictionary-field paths; no retained full font programs or arbitrary stream decoding belongs here.

For M1, a static table's process-wide bytes are fixed, while each allocated result/encoding cache counts against session limits. Keep builtin selection provenance (exact face/table hash or version) observable alongside advance origins. Query and later text execution work must have separate cumulative budgets; a cached 256-entry table does not bound unlimited text output.

## Required tests and later execution boundary

M0 independent tests should cover all endpoint boundaries, code 0/255, exact/short/long arrays, missing endpoints, indirect arrays and entries, wrong types, reference cycles/generations, zero/negative/fractional widths, integer conversion precision, nonfinite manually constructed values, signed zero policy, MissingWidth absent/present/invalid and lookup on both sides of the declared range. Verify zero entries are never treated as absent. Compare generated finite real numbers to independent arithmetic expectations; preserve native truncation as an explicit difference.

M1 should exhaust every AFM glyph against native glyph-name advances, then every named slot of the four S0 tables with missing-name cases preserved. The real gate is 48 fonts x 256 outcomes (12,288), including undefined slots, plus the M0 five-font gate (1,280 outcomes/160 explicit entries). Test Differences aliases, unknown names, .notdef, empty/null encodings, a conflicting ToUnicode ligature, embedded program presence, unknown/subset BaseFont names and exact face selection. Do not count a native .notdef fallback as strict success.

Later horizontal text execution will need font size, character spacing, word spacing on the original relevant single-byte code, horizontal scale, TJ numeric adjustments, text/line matrices and CTM. For a Unicode ligature the source code advances once; for a TJ adjustment there is no source character/Unicode span. These are future state-machine rules, not work performed by `metrics.lookup`. No kerning, shaping, vertical writing, ascent/descent, outline bounds, selection quads, search boxes or rasterization is implied by an advance table.

## Read-only M1 parity result (after M0 implementation)

`tools/mupdf-builtin-metrics-oracle.c`, built by `tools/build-builtin-metrics-reference.sh`, loads only the two exact bundled faces and queries printable ASCII glyph names under fixed query/length limits. It calls `FT_Get_Name_Index` and checks the returned glyph name exactly; MuPDF's public `fz_encode_character_by_glyph_name` is unsuitable here because it falls back through Unicode on a missing name. FreeType calls run inside MuPDF's required font lock/allocator context. The oracle reports units-per-em, raw unscaled advances and the native integer width. No runtime Rust font tables are added.

`tools/compare-builtin-metrics.py` independently parses the pinned AFM semicolon fields using exact rational widths, queries every AFM name plus the S0 glyph-name union and an unknown sentinel, and writes `reference/builtin-metrics-parity.json`. Subprocesses have bounded timeouts. All five malformed query cases (empty, overlong, whitespace, embedded NUL and excess query count) refuse.

The result is **1,982 queries, 1,308 exact shared-name width comparisons, zero value mismatches**. Each bundled CFF retains 654 of the AFM's 855 names, all at 1000 units-per-em. The 402 combined AFM-only names are explicitly recorded as subset exclusions, not treated as successful native matches. Every named WinAnsi, MacRoman and StandardEncoding slot remains available; each face lacks 142 MacExpert byte slots in the native CFF subset, compared with the initial AFM's 135 missing names.

This narrows the proposed M1 runtime dataset to the exact **AFM/CFF name intersection**, rather than importing all 855 AFM entries. A known S0 glyph such as `zerosuperior` can have an AFM width while being absent from the bundled CFF; its strict builtin outcome must remain missing. The required 48 real Base14 fonts still have complete coverage for their defined WinAnsi slots. Undefined slots remain explicit, regardless of native glyph-zero advance.

Reproduce the investigation:

```sh
sh tools/build-builtin-metrics-reference.sh /private/tmp/tandem-mupdf-reference
python3 tools/compare-builtin-metrics.py \
  /private/tmp/tandem-mupdf-reference/mupdf-builtin-metrics-oracle \
  /private/tmp/tandem-mupdf-reference/mupdf-1.28.1-source \
  reference/builtin-metrics-parity.json
```

M1 runtime tables remain a separately authorized stage despite this successful shared-subset comparison.
