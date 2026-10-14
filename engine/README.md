Current functional preview (updated October 14, 2026, M1 in progress): see [WORKFLOW-STATUS.md](WORKFLOW-STATUS.md) for the implemented text-edit, undo/redo, save and reopen workflow, its limits, tests and launcher. The historical milestone descriptions below remain reference material.

# Tandem PDF Rust engine experiment

Current R24 status (2026-10-06): validated fully opaque soft masks are accepted after independent review and complete automated gates. See [R24 implementation and evidence](R24-IMPLEMENTATION.md) and [status](STATUS.md). R24 is the latest accepted milestone.

A safe Rust redesign of the MuPDF functionality used by Tandem PDF. This is an independent crate alongside the existing Electron application. It is not integrated into the app. It renders a bounded subset of PDF pages, including nested ordinary Form XObjects, but does not yet edit page content or sign PDFs. Current scope and evidence are in STATUS.md.

The user authorized this experiment on October 2, 2026. Engine source is AGPL-3.0-or-later; see COPYING, NOTICE and reference/PROVENANCE.md. The parent application's existing license is unchanged. Our crate, miniz_oxide and adler2 forbid unsafe code. The optional raster feature uses an audited tiny-skia safe API; that dependency and its transitive dependencies contain internal unsafe. Native MuPDF is used only by test oracles.

## Implemented

- Byte-preserving PDF lexer with exact source spans, strict malformed-input errors and token/count limits.
- Owned direct objects, dictionaries, arrays and checked indirect references; bounded depth, object count and decoded payload. Duplicate keys reject by default; KeepLast is explicit.
- Read-only classic and stream xrefs, incremental revisions, and compressed object streams. Exact wrappers, offsets, generation numbers, newest-entry/free precedence, stream lengths and borrowed stream ranges. Global syntax/object/payload/metadata budgets, container and member limits. Bootstrap and object-stream metadata have a deliberately narrow direct identity/Flate subset.
- Page-tree inventory with inherited page boxes and rotation, non-inherited UserUnit, count checks, cycle/shared-node detection and aggregate content-reference limits.
- Bounded Flate, ASCIIHex, ASCII85, RunLength and LZW decoding and filter chains. LZW supports EarlyChange 0/1; Flate/LZW support TIFF2 and PNG10–15 predictors with 1/2/4/8/16-bit components. Termination and cumulative expansion are checked. Strict malformed input is deliberately less tolerant than MuPDF recovery.
- Reference-free content operands and incremental operation parsing, with source spans, sticky errors and cumulative limits. Inline images stop explicitly at BI before binary payloads are read.
- Logical page-content loading with exact array-member space padding and cumulative encoded-input, intermediate decode-work, joined-byte and stream-count limits. Repeated references count repeatedly; source segments remain available.
- Lazy, document-bound page font resources with validated membership, nearest whole-dictionary inheritance, immutable success caches, authoritative one-byte horizontal ToUnicode and explicit simple encodings. Failed attempts retain conservative work charges. Type0 and built-in encoding guesses remain unsupported. R22 adds scoped ordinary Form resources.
- Standalone path/graphics-state execution into immutable flat path, dash and clip arenas. Preserves fill/stroke/clip order, q/Q state, signed rectangles, device colors and hairline requests under cumulative work/capacity limits. Mixed transforms within a path and unsupported operations fail explicitly. The optional raster stage handles the restricted contour subset described below.
- Allocation-free classification of standard PDF operators and local operand-schema checks. This does not execute operators or resolve fonts, color spaces, images or graphics state.
- Strict standalone ToUnicode CMaps with width-preserving character codes, codespaces, bfchar/bfrange and array destinations, validated Unicode sequences and bounded expansion. The standalone parser itself does not load document fonts or provide fallback/text extraction. Malformed destinations, overlaps, ambiguous prefixes, unsupported wrappers and UseCMap dependencies fail explicitly.
- Strict mapped-string decoding with validated UTF-8, borrowed input, and source/output spans per character code. Missing mappings, incomplete codes and invalid codes are distinct failures; repeated expansion is charged against cumulative limits.
- Explicit WinAnsi, MacRoman, Standard and MacExpert byte encodings with an exact 378-name Unicode registry, transactional Differences, and separate undefined/unknown outcomes. This is declared encoding data, not font-program glyph selection.
- Checked placement from public PageInfo into typed user-to-page transforms and positive DPI scaling. It rejects overflow, underflow and severe cancellation; positive sub-unit boxes retain their actual dimensions.
- Finite points, rectangles and affine maps tagged by coordinate space, with checked composition, inverse and outward raster bounds. Page placement is integrated separately; complete page rendering is not implemented.
- Deterministic bounded object serialization, preserving binary values and finite f64 reals in decimal syntax.
- Experimental full-document rewrites with retained effective live objects, original header version, inherited Info/ID metadata, exact raw stream bytes, and fresh sparse classic xrefs. Stream Length becomes a direct integer; dormant original xref/object streams remain retained objects. Input bytes remain unchanged; output is returned as a new buffer.

Unsupported features fail explicitly. The reader excludes hybrid xrefs, encryption, external-file streams and repair; it requires the PDF header at byte zero. Compressed containers require direct bootstrap metadata and may not themselves be compressed. The writer additionally refuses signature-bearing inputs (including unsigned signature fields), linearization, unknown meaningful trailer entries and dangling references. It does not preserve signatures or incremental revision history. Optional null trailer entries are treated as absent, including inherited Root/Info/ID; this is an explicit reader policy, not a claim that all such incomplete trailers are standards-conforming.

Parsing, rewriting or geometry success is not proof of PDF conformance or Rust rendering capability. The reference renderer's identical output is used to test rewrite preservation.

## Build and use

Rust 1.85.1 or newer, with rustfmt and clippy, is required. The core suite has also been run on 1.85.1; normal development uses 1.98.1. Cargo.lock pins dependencies. No Node/Electron is needed to run the Rust engine.

```sh
cd engine
cargo build --locked
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
printf '<< /Type /Page /Parent 2 0 R >>' | cargo run --quiet -- --canonical
cargo run --quiet -- --document ../fixtures/large.pdf
printf '00ff>' | cargo run --quiet -- --decode=asciihex
printf 'q 1 0 0 1 10 20 cm Q' | cargo run --quiet -- --content
printf '/F1 12 Tf (Hello) Tj' | cargo run --quiet -- --check-content
cargo run --quiet -- --page-content=1 /path/to/input.pdf
cargo run --quiet -- --stream=4:0 /path/to/input.pdf
cargo run --quiet -- --rewrite /path/to/input.pdf > /path/to/new-output.pdf
```

Use a **different output path** with shell redirection: shells truncate the redirected path before the program opens its input. The library has no file-overwrite API.

`pdf-inspect --tokens` emits token TSV, `--object` prints an owned object, `--canonical` emits canonical direct-object syntax, `--document` emits page geometry and stream IDs as JSON, `--decode=FILTERS` emits decoded bytes, and `--stream=N:G` decodes a document stream. Raw LZW CLI names are `lzw` (EarlyChange1) and `lzw0`; PDF DecodeParms resolve parameters through the document API. `--content` emits hex-encoded operator names, top-level operand counts and canonical operands as TSV; `--check-content` validates local operator schemas, and `--page-content=N` emits logical page bytes for a one-based page. `--cmap=QUERIES` reads a standalone ToUnicode map and runs a query file containing `lookup HEX` or `decode HEX` lines (`decode -` represents empty input); it reports scalar values, missing mappings and distinct incomplete/invalid code errors. Query files are capped at8MiB/262,144queries. `--rewrite` emits a complete new PDF after successful serialization. Input defaults to stdin, or pass a file path; CLI input is capped at 16 MiB, library limits are configurable. Documents borrow original stream bytes and own parsed objects, including decoded compressed members.

## Reference tests

The official MuPDF 1.28.1 archive must match SHA256 `dc94c60b2537e2ac9a2d379dd3801545f84a3a302d15c9da358362a1270707c3`. Download location is recorded in reference/PROVENANCE.md. Large sources and binaries are ignored, not vendored.

```sh
sh tools/build-reference.sh /path/to/mupdf-1.28.1-source.tar.gz
sh tools/build-filter-reference.sh reference/build
sh tools/build-document-reference.sh reference/build
python3 tools/differential.py --reference reference/build/mupdf-lex-oracle --cases 2000
python3 tools/differential-filters.py --reference reference/build/mupdf-filter-oracle --cases 1000
python3 tools/compare-fixtures.py --minimum-supported 49
node tools/compare-rewrites.mjs /path/to/mupdf/dist/mupdf.js ../fixtures target/debug/pdf-inspect reference/build/mupdf-document-oracle reference/rewrite-comparison.json
```

Additional predictor/LZW/object-stream/serializer differential tools are under tools/. Decoder comparisons check exact output against independent Python encoders and MuPDF. Fixture hashes are checked before geometry and rewrite comparisons. Golden inventory uses pinned MuPDF 1.28.1 WASM; passwords and private keys are not needed or recorded. Native document comparisons query repair state after page loading.

Current evidence and stage acceptance are in STATUS.md, REVIEW-*.md and reference/*-r*.json. Deterministic mutation tests are not coverage-guided fuzzing or a complete security audit. Logical limits do not guarantee process allocation failures become recoverable errors. Gates on this machine do not establish Windows/Linux runtime validation. Rust 1.85.1 core tests and an all-feature wasm32 library compilation have passed.

## Unicode-map scope

Four strict-valid fixture ToUnicode maps pass complete native lookup comparisons;
six malformed fixture maps are refused at the destination byte offset. MuPDF's
odd-byte UTF-16 truncation is recorded as recovery, not silently adopted. Typed
Rust lookups retain source width, while the native lookup API merges equal
numeric values across widths; those cases have separate independent expected
results. Identity-H maps bytes to CIDs and is not a Unicode fallback. The 51 fixture fonts without ToUnicode have explicit simple encodings. The standalone
encoding implementation is accepted; lazy page font loading is implemented for Type1/TrueType declarations.
See CMAP-DESIGN.md and reference/cmap-differential-u0.json for precise limits.


## Mapped strings

`--cmap-text=RAWFILE [CMAPFILE|-]` decodes the raw bytes through a strict map and
emits UTF-8 bytes in hex plus source and output spans. Each span covers one whole
character-code mapping, which may contain several Unicode scalars. These are
neither grapheme boundaries nor positioned text. There is no normalization,
replacement character, implicit encoding, or font fallback. See
`REVIEW-R7B.md` and `reference/text-decode-differential-r7b.json` for the independent
106-case comparison, including all entries from the four valid fixture maps.


## Encodings and page placement

`--encoding` reads a PDF encoding name or dictionary with an explicit BaseEncoding
and optional Differences. It emits the 256 slot names and Unicode/undefined/unknown
outcomes. Missing bases require font context rather than an invented default.
See `REVIEW-R8-S0.md`: all 1,024 base slots and 378 known names pass independent
native/source comparisons, plus generated overrides and 31 strict CLI refusals.

`--page-geometry` reads a PDF and emits each checked user-to-page matrix and
continuous page bounds. All 572 readable fixture pages and 256 generated pages
match the pinned native API within recorded f32/f64 tolerances. Positive sub-unit
boxes are deliberately preserved while native recovery enlarges them. Review
found and fixed cancellation for tiny pages at distant origins. This does not
allocate pixels or apply clipping; see `REVIEW-R8B.md` and
`reference/page-geometry-differential.json`.


## Page font declarations and standalone path execution

`--fonts` lists each page's exact font resource names, loaded identity or explicit
per-binding error, deduplicated simple-font mappings and cache/work usage. It
matches all 572 fixture resource tables and 582 bindings: 575 simple bindings load,
seven Type0 bindings refuse. There are 53 page-bound simple fonts: 50 explicit
encodings (12,800 exact slots) and three ToUnicode maps (54 exact entries). An
additional Form-only font in the all-object inventory remains outside this scope.
These declarations do not prove rendered glyph identity or extraction quality.

`--path` reads a standalone content program with identity initial transform and
unbounded initial clipping. It emits a complete drawing/clip event trace only on
success. The library also accepts explicit initial page geometry and clipping.
Independent native/model comparisons cover 310 programs and 5,835 events, plus
20 strict refusals. The path restriction and native empty-paint difference are
recorded in `REVIEW-R9-P0.md`; font cache/failed-work review is in
`REVIEW-R9-F0.md`. The combined R9 suite passes 385 tests, formatting and Clippy
with warnings denied. None of this is a Rust page renderer yet.


## Declared text and explicit advances

`--encoding-text=RAWFILE` uses the same bounded UTF-8/span accumulator as
`--cmap-text`, with distinct undefined-code and unknown-glyph errors. The library
`font_text::decode_declared_text` dispatches a loaded font's authoritative mapping;
a ToUnicode failure never switches to an encoding. It retains no font borrow.
Independent comparisons exercise every predefined encoding slot, known registry
name and exact UTF-8/source span; the existing ToUnicode differential still passes.

`--font-metrics` exposes explicit FirstChar/LastChar/Widths declarations with
per-code advance provenance. Zero, negative and fractional widths stay explicit;
MissingWidth is read from the descriptor or reported as default zero. Five real
fonts match 160 entries and 1,280 complete byte lookups. The other 48 page-bound
simple fonts require a separately implemented builtin path. Metrics do not apply
font size, place text, select glyph outlines or imply extraction correctness.

The R10 default-feature gate passes 407 tests, formatting and strict Clippy.
A 1,600-case deterministic mutation smoke also passed for the preceding reader,
font, content and path APIs; it is not a coverage-guided fuzzing/security claim.

## Optional orthogonal rasterizer

`cargo build --locked --features raster` enables the exact pinned tiny-skia
0.12.0 backend with std and without its default SIMD/PNG features. Default builds
retain the two original runtime dependencies. The adapter forbids unsafe code;
the backend has internal representation/array casts and numeric invariants.
The audited subset, early-return constraints, remaining process-OOM limitation,
license files and exact checksums are recorded in `RASTER-DESIGN.md`,
`reference/tiny-skia-audit.json` and `reference/licenses/`.

The first raster API has passed independent review. It supports only preflighted
in-canvas integer orthogonal contours and binary clipping; slanted/fractional/
zero-area contours, strokes, curves and CMYK explicitly refuse. RGB/Gray uses
specified round-half-up bytes, with exact independent color tests; native full
pixel parity is limited to endpoint colors because native color conversion differs.
No complete PDF page renderer or app integration is claimed.


The raster comparison passes 304 programs and 311,296 exact pixels against both
MuPDF and an independent geometry/clip model using endpoint colors. Another
2,042 samples verify the specified Gray/RGB quantization, and twelve unsupported
CLI inputs refuse without output. See `REVIEW-R11-R0.md`. The reproducible
[orthogonal demo](examples/README.md) was rendered by Rust and matches the native
reference byte-for-byte. The lettering is built from paths; this is not font
rendering.

## Builtin advances

Two exact Type1 faces, Helvetica and Times-Roman, have 654 verified glyph-name
advances each. Tables use the intersection of the pinned AFM and bundled CFF,
with the font data's OFL notices retained. Width selection is independent of
Unicode decoding. Undefined bytes, `.notdef` slots and missing metrics remain
explicit outcomes. `--builtin=FACE` inspects an explicit encoding; the separate
`--builtin-metrics` document adapter requires absent Widths and no embedded font
program. Aliases, subset-name stripping and all other faces are unsupported.
Independent implementation and fixture review passed: all 1,308 table entries,
402 excluded AFM-only names, 49,152 standalone slot outcomes, and all 48 fixture
fonts / 12,288 slots match the recorded expectations. Thirty-seven strict CLI
refusals emit no output. See `FONT-METRICS-DESIGN.md` and `REVIEW-R12-M1.md`.


The combined R12 all-feature gate passes 444 tests on Rust 1.98.1 and the declared
minimum 1.85.1. Formatting and strict Clippy pass. The all-feature library also
compiles for wasm32-unknown-unknown on 1.85.1; browser runtime behavior is not yet
tested. Default builds still use only miniz_oxide and adler2.


## Standalone horizontal text positioning

`--text=FONTSFILE [CONTENTFILE|-]` interprets the supported text operators using
explicit borrowed font bindings. The font configuration is a direct PDF
dictionary; see [the text example](examples/README.md). It supports declared
one-byte widths or an explicitly selected builtin face, independent authoritative
ToUnicode, text and line matrices, spacing, rise, scaling, TJ adjustments and
rendering modes 0/3. Graphics saves and CTM changes are supported outside text
objects; q/Q/cm inside BT and all mixed page operators refuse explicitly.

Results own flat Unicode/source arenas, per-code placements, ordered events and
final matrices. One source code receives one advance even when it maps to several
Unicode scalars. Source offsets refer to decoded string operands plus enclosing
operation spans, not escaped file offsets. Allocation, work and numerical bounds
are cumulative; a failed suffix cannot publish a successful prefix. This is not
reading-order extraction, glyph selection or text rendering.

T0 passes 246 exact rational programs / 2,906 placements, 160 bounded native
comparisons and 48 atomic CLI refusals. The full all-feature suite passes 470 tests
on Rust 1.98.1 and 1.85.1, with formatting, strict Clippy and the 1.85.1 wasm32
library check passing. See `REVIEW-R13-T0.md` and `TEXT-POSITION-DESIGN.md`.

## Mixed path and text programs

`--program=FONTSFILE [CONTENTFILE|-]` runs one persistent path/text interpreter
with shared graphics state and cumulative budgets. Its ordered trace retains
path paints, clip changes, every text placement and the color/clip context of
text events. Unsupported resource operators and invalid cross-engine state
transitions fail atomically. The standalone content interpreter is also used by the bounded document
page adapter described below.

C0 passes 220 mixed programs, 5,280 modeled events, 3,960 native semantic events,
2,200 codes and 26 atomic refusals. Refreshed standalone path/text comparisons
pass. The R14 gate passes 492 tests on Rust 1.98.1 and 1.85.1, formatting, strict
Clippy and the minimum-toolchain wasm library check. See REVIEW-R14-C0.md.

## Simple TrueType outlines

`--outline=all [FONTFILE|-]` reads a standalone TrueType font and emits exact
unhinted simple-glyph geometry. A query file can replace `all`, with one decimal
glyph ID per line. Coordinates for quadratic commands are doubled integers so
implicit half-unit points remain exact. Results include original point flags,
contour endpoints, bounds, table metadata and cumulative resource usage.

The reader uses borrowed checked byte slices and owned bounded result arenas.
It accepts in-bounds unaligned tables for compatibility with an existing fixture,
and preserves declared checksums without claiming to validate them. Composite
and variable fonts, font cmap lookup, PDF glyph selection, hinting and font
rasterization are unsupported. Instructions are retained only as a byte count.

O0 passes exact comparisons for 220 generated fonts / 1,415 glyphs and all five
fixture fonts / 1,885 slots, plus 109 atomic refusals. The combined suite passes
513 tests on both supported Rust versions with formatting, strict Clippy and
minimum-toolchain wasm compilation. See REVIEW-R15-O0.md and
TRUETYPE-OUTLINE-DESIGN.md.

## Document page traces

`--page-program=N [PDF|-]` produces a complete mixed trace for a one-based page.
`--page-programs [PDF|-]` opens the document once and uses one cumulative session
for all pages, returning compact semantic traces or precise per-page refusals.
Both modes enable the explicitly named `AllowPinnedBase14` metric policy and
report that choice. The library defaults to `DeclaredOnly`.

`PageProgramSession` joins the exact logical content stream, discovers every
selected Tf name (even when no text is shown), prepares only those fonts and
freezes their caches before running one persistent mixed interpreter. Results own
their bytes, stream-occurrence segments, font identities, paths and positioned
source codes. They remain valid after the input/session is dropped. Page geometry
and the initial page clip apply once. Logical spans can be projected back to
individual decoded-stream occurrences and synthetic joining spaces; string
payload offsets are not raw PDF file offsets.

Failures publish no successful prefix. Successful earlier outputs remain usable,
and repeated or failed calls keep consuming cumulative budgets. Content decode,
parser passes, font/cache work, and output allocations have explicit independent
partitions. Session inventory and the previously opened document are separately
bounded prerequisites; these counters do not claim to measure process RSS.
Unsupported page groups and default device-color overrides fail explicitly.

This milestone produces page-content traces, not a complete page renderer.
Glyph selection/rasterization, images, Form XObjects, Type0/CJK fonts, marked
content, transparency and annotations remain outside this page execution subset.
See `REVIEW-R16-PP1.md` and `reference/page-program-differential-pp1.json` for
accepted evidence, exact refusals and native floating-point comparison policies.

PP1 passes 543 native-matched fixture pages (500 stress and 43 other), with 29
explicit refusals, plus 220 generated real-PDF cases matching both native behavior
and the exact independent model. The combined all-feature suite passes 538 tests
on Rust 1.98.1 and 1.85.1, with formatting, strict Clippy and minimum-toolchain
WebAssembly compilation passing. No page pixel parity is claimed by this gate.


## Embedded glyph selection and outlines

`--tt-cmap=all FONT.ttf` inspects each original format4 cmap record over all65,536
16-bit character codes; `--tt-cmap=N` selects one record. `--font-glyphs=N:G FILE.pdf` loads a simple
embedded TrueType font and reports all256 source-byte slots, mapping route,
missing/unsupported status and exact O0 outlines for mapped nonzero glyphs.
Selection is independent of ToUnicode and preserves native symbol/private-use
and explicit Unicode/MacRoman encoding policy for the supported subset. Name
fallbacks refuse per code. The font/map binding prevents decoding a map against
another font. Failed code attempts and failed stream work remain budgeted.
This does not place glyphs on pages or render text pixels. See
GLYPH-SELECTION-DESIGN.md and REVIEW-R17-G0.md for scope, budgets and evidence.


## Embedded glyphs placed on actual pages

`--page-glyph=N FILE.pdf` emits a complete PP1 trace with embedded simple-TrueType
source-code/GID attachments and page-space outline commands. `--page-glyphs`
classifies every page under one cumulative session. Glyph drawing origins use
hmtx side bearings; PDF Widths remain independent advancement values. Cached
immutable outlines are shared while transformed geometry remains owned.
The current placement subset accepts2 of572 corpus pages, with570 explicit
refusals; the broader PP1 trace capability still accepts543 pages. All237 new
supported generated pages match native unhinted geometry and independent state
checks. This is not text rasterization or completion of the whole engine rewrite.
See PAGE-GLYPH-DESIGN.md, REVIEW-R18-G1.md and REWRITE-ROADMAP.md.

Filled-page raster development (R19):

```sh
cargo run --offline --locked --bin pdf-inspect -- --page-render=1@72 ../fixtures/p3/simple-subset.pdf > /tmp/page.ppm
```

This produces an opaque P6 PPM through the owned Rust renderer without enabling
an optional feature. It supports G1 embedded simple TrueType glyphs and PP1 filled
Gray/RGB paths with binary sample clipping and explicit4x4 supersampling. Strokes,
CMYK paints and upstream unsupported fonts/content fail atomically. R19 is accepted for this subset; see PAGE-RASTER-DESIGN.md and STATUS.md. This is not yet the
complete application engine replacement.

Pinned standard-font development (R20):

```sh
cargo run --offline --locked --bin pdf-inspect -- --builtin-outlines=Helvetica
cargo run --offline --locked --bin pdf-inspect -- --page-outlines=1 ../fixtures/hello.pdf
cargo run --offline --locked --bin pdf-inspect -- --page-render=1@72 ../fixtures/hello.pdf > /tmp/hello.ppm
```

Combined outline inspection and rendering explicitly select pinned Helvetica and
Times-Roman outlines. The legacy page-glyph modes keep their embedded-font policy.
These are licensed canonical font assets with safe Rust lookup and placement;
general CFF/Type1 decoding is still a separate gap. See BASE14-OUTLINE-DESIGN.md;
R20 is accepted:543/572fixture outline pages and43rendered pages, with every
remaining refusal explicit. Native/reference reports and remaining scope are in
BUILTIN-OUTLINES-NATIVE-NOTES.md and STATUS.md.


R21 solid-stroke rendering is accepted:543/572fixture pages render, including all500
stress pages. Positive solid widths support admitted lines/regular cubics with
butt/round/square caps and miter/round/bevel joins under the full saved affine map.
Hairlines, meaningful dashes and unresolved/folded geometry still refuse. All684
current/MSRV tests, strict Clippy, fmt and wasm checks pass;110 generated cases and
the complete corpus gate pass. Native color truncation versus Rust rounding is
separately source-certified, not hidden by a general tolerance. See STATUS.md,
REVIEW-R21-S1.md and STROKE-NATIVE-NOTES.md. This is the historical R21 baseline.


## R22 ordinary Forms — accepted 2026-10-06

R22 ordinary Form XObject execution is accepted after the complete native/model/corpus gate. Scoped resources and fonts, nested Forms, affine Matrix/BBox clipping, state restoration, owned provenance and cumulative failure budgets are verified. These are reusable drawing streams, not interactive AcroForm fields.

- Rust 1.98.1 and minimum 1.85.1: **724 tests pass each**. Formatting, strict all-target/all-feature Clippy and minimum-toolchain wasm32 library compilation pass.
- **58 positive generated Form graphs**, four explicit native-fallback/strict-scope differences and ten malformed/cycle/state refusals pass. **66 generated renders and 10 independent pixel models** pass with zero certified channel-interval violations.
- **545/572 corpus pages render at 18 DPI**, with 27 explicit refusals; all 500 stress pages pass. All **48 supplementary 72-DPI renders** pass. The two new Form-only pages render at both resolutions. All **589 previously accepted render hashes remain identical**; all 29 old refusal pages receive a Form-aware census.
- Reference validation: 23 native comparator mutations, 25 Rust comparator mutations, 13 native process faults, six Rust process faults, three actual native negative-wrapper cases and eight pixel/baseline mutations are rejected as expected.
- The new `tools/verify-form-gate.py` requires exact corpus identities/counts, all generated/model/census coverage, preserved hashes and current evidence hashes. Ten deliberately shortened or corrupted reports are rejected. It prevents a successful `--skip-corpus` or `--native-only` run from being treated as final acceptance.

Native AA/edge differences and the existing source-certified 229-versus-230 color policy remain diagnostic. No tolerance or policy was widened. This is a bounded Forms milestone, not complete PDF rendering or application parity.

## R23a image-sampling foundation — 2026-10-06

Implemented the R23a exact affine image-sampling foundation in `src/image_sampling.rs`. It uses bounded signed 256-bit arithmetic for determinant/coverage and exact nearest-neighbor row/column selection under rotation, reflection and shear. Device edge ownership, top-down source rows, singularity/numeric refusal and cumulative work/attempt budgets are explicit. No heap allocation, unsafe code or new dependency.

Verification: **740 tests pass on Rust 1.98.1 and minimum 1.85.1**, including 16 new arithmetic/sampling tests. Formatting, strict all-target/all-feature Clippy and minimum-version wasm32 library compilation pass. The separate Python Fraction polygon/ray-crossing and Gaussian-elimination model matches **10,084 cases**: 1,358 selected samples, 7,118 outside samples and 1,608 explicit refusals. Five altered/omitted-output mutations are rejected. The final numeric comparison report is `reference/image-sampling-differential-r23a.json`.

This is a tested numeric component, **not complete image support or R23 acceptance**. Image dictionary/filter/predictor validation, retained decoded assets, scoped Do dispatch, ordered image events, sample-buffer composition, native image comparisons and the full image corpus gate remain to implement. Page execution/rendering is unchanged. R22 remains the latest accepted integrated rendering milestone.

## R23 opaque image XObjects accepted — 2026-10-06

Image loading, scoped traversal and ordered exact sample composition are accepted. All796current/MSRV tests and fmt/Clippy/wasm gates pass. Native/model evidence covers265decoded positives,28generated pages,632corpus attempts,24new-page certificates and2mixed-content certificates.557/572pages render at18DPI and60/60at72DPI; all593prior hashes are unchanged. The final verifier rejects29structural/4raw corruptions. See STATUS.md, IMAGE-NATIVE-NOTES.md and reference/image-acceptance-r23.json. General images/transparency and the full engine/application rewrite remain incomplete.
