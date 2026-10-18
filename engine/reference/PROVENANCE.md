# Pinned reference and lexer oracle

The development oracle uses **MuPDF 1.28.1**, matching the application's pinned
engine version. It is a comparison tool, not part of the Rust runtime.

The oracles and drivers that remain live in `engine/oracle/` (AGPL-3.0-or-later,
test-only, outside the repository's MIT grant; see its README). The tools for
the engine modules removed in Phase 0 of `docs/mit-clean-room-plan.md` (the
content, CMap, path, raster, text-position, program, outline, page-geometry,
page-glyph, page-raster, stroke, form, image and soft-mask oracles, their
`build-*-reference.sh` scripts, Python models and differential drivers) were
deleted. Sections below that name them under `tools/` describe evidence they
produced; the files are in the git history up to commit 9c6b66b.

- Official archive: https://mupdf.com/downloads/archive/mupdf-1.28.1-source.tar.gz
- Archive redirect recorded by the existing repository audit:
  https://casper.mupdf.com/downloads/archive/mupdf-1.28.1-source.tar.gz
- SHA-256: `dc94c60b2537e2ac9a2d379dd3801545f84a3a302d15c9da358362a1270707c3`
- Archive root: `mupdf-1.28.1-source/`
- Version declaration: `include/mupdf/fitz/version.h`
- Primary lexical reference: `source/pdf/pdf-lex.c`; SHA-256
  `401002fee1c6de41e6c40c783f023351a2e34c6e94c0af9de230e283a8f7ecf1`
- API and buffer declarations: `include/mupdf/pdf/parse.h` and
  `include/mupdf/pdf/document.h`.
- `../oracle/COPYING` (until October 18, 2026 also `../COPYING`) is copied
  verbatim from the archive, SHA-256
  `57c8ff33c9c0cfc3ef00e650a1cc910d7ee479a8bc509f6c9209a7c2a11399d6`.
- Prior source audit: `../../docs/third-party/corresponding-source.md`.

## Licence (October 18, 2026)

The engine and its WASM bridge are MIT (the repository's `LICENSE`); `../NOTICE`
lists the third-party notices they still carry. Earlier versions were
AGPL-3.0-or-later because parts were written with knowledge of MuPDF's source.
In October 2026 those parts were deleted (Phase 0) or rewritten in a clean room
from the PDF specification and black-box measurement (eight workstreams), and
the encoding and glyph-name tables taken from MuPDF source files were
regenerated from public sources (the URW font tables are URW's OFL data);
see `../../docs/mit-clean-room-plan.md` and `../../docs/clean-room/`. MuPDF
remains only a black-box test oracle in `engine/oracle/` (AGPL, not shipped).
The sections below are the dated evidence record. Where they call code
MuPDF-informed, or say it keeps Artifex notices or AGPL terms, they describe
modules that have since been deleted or rewritten.

The URW font data tables keep their OFL-1.1 terms. On October 18, 2026 the
comment in `src/builtin_outlines_data.rs` that called the surrounding code AGPL
was changed by hand to say MIT; that one line differs from the output of
`oracle/generate-builtin-outlines.py` recorded in
`builtin-outlines-manifest.json` (whose `license` field is likewise historical).

## Rebuild offline

A C11 compiler, `make`, `tar`, `shasum`, and the verified source archive are
needed. No system installation or network download is performed. From the
repository root:

```sh
engine/oracle/build-reference.sh /absolute/path/mupdf-1.28.1-source.tar.gz
```

The default output is `engine/reference/build/mupdf-lex-oracle` (ignored).
An optional second argument selects another build directory; `JOBS` defaults to
4. `CC` can select the compiler for the adapter. The script verifies the full
archive before extraction and rebuilds MuPDF static `libs` only. GUI viewers,
command-line application targets, curl, libcrypto, OCR, and barcode integration
are disabled. Bundled core libraries and assets remain in the reference build;
this is deliberately not a claim that the compiled archive contains only a PDF
lexer. No document handler registration or document JavaScript execution is
used by the adapter. Build tested on macOS arm64 with Apple clang; other host
platforms have not been validated.

Initial build and script validation used `/private/tmp/tandem-mupdf-reference`.
Temporary directories are expendable and are not required for future builds.

## Protocol

Read raw PDF lexical bytes from stdin (maximum 16 MiB). Emit ASCII TSV to stdout,
one token per line, no header, no EOF record and no byte spans:

| Token | Output |
| --- | --- |
| Integer | `integer<TAB>` followed by signed decimal int64 |
| Real | `real<TAB>` followed by C `%.9g` formatting of MuPDF's float32 value |
| Name | `name<TAB>` followed by lowercase hex bytes, excluding `/` |
| String | `string<TAB>` followed by lowercase hex decoded bytes |
| Keyword | `keyword<TAB>` followed by lowercase hex bytes |
| Delimiters | `array-start`, `array-end`, `dict-start`, `dict-end`, `brace-start`, `brace-end` |

Reserved MuPDF token types (`true`, `false`, `null`, `R`, `obj`, `endobj`, `stream`,
`endstream`, `xref`, `trailer`, `startxref`, `newobj`) are normalized to keywords.
String/name lengths come from `pdf_lexbuf.len`, preserving embedded binary NULs.
Ordinary keyword lengths also use that field. The malformed numeric-to-keyword
fallback does not set length in MuPDF, so that path uses its NUL-terminated
scratch buffer; this exposes native recovery semantics, not raw source bytes.
The adapter uses the large 65,536-byte lexical scratch buffer.

Exit 0 means lexing completed, 2 means a MuPDF lexical error, exception, or
progress invariant failure, and 1 means an input/output or adapter allocation
error. Warnings and diagnostics go to stderr. An error can follow valid token
lines; callers must inspect exit status before accepting the complete output.
The decimal formatter starts in the C locale; the adapter never calls setlocale.

## Deliberate limits and comparison policy

The oracle records MuPDF behavior, including permissive recovery and overflow;
it does not establish specification correctness. Compare ordinary valid inputs
exactly for bytes and kinds. Compare reals after a documented float32 conversion
or tolerance, since the safe engine may retain float64 precision. Inputs beyond
MuPDF's internal name/number limits, malformed names/numbers/hex strings, integer
overflow, and malformed input recovery require an explicit policy comparison.

Verified examples: `(a\\000b)` emits `string<TAB>610062`, `<004142ff>` preserves
NUL and high bytes, `/A#20B` decodes to `412042`, and `0.1` prints `0.100000001`.
`)` and an unclosed literal string exit 2. MuPDF accepts `<zz>` as byte `00` with
a warning, preserves `/A#00B` as bytes `A#00B`, and classifies `1e2` and `1.2.3`
as keywords. The Rust implementation need not reproduce unsafe or ambiguous
repair behavior; record such divergences and test its intended policy instead.

This lexer oracle does not validate object parsing, streams, document rendering,
saving, redaction, or compatibility with arbitrary complete PDF files.

## Stream-filter oracle (R3)

After building the pinned native reference, run:

```sh
engine/oracle/build-filter-reference.sh engine/reference/build
```

The optional argument is the same reference build directory used by
`build-reference.sh`; it defaults to `engine/reference/build`. This adapter-only
build reuses the existing static archives, checks the pinned lexical source hash
as a sanity check, and does not independently prove the archives' build history.
Rebuild them with the verified full source archive if their provenance is unknown.

`mupdf-filter-oracle flate|asciihex|ascii85|runlength` accepts raw encoded stdin
and emits raw decoded stdout. Both input and decoded output are bounded to
16 MiB. No stdout is emitted for a decoding failure or size-limit failure.
Exit 0 means native decoding completed, 1 means usage/input/output/allocation
failure, and 2 means a decoder exception, stream error, or output-limit refusal.
Diagnostics/warnings go to stderr. The adapter checks MuPDF's stream error flag
because `fz_read` can catch exceptions and return partial data internally.

The Flate adapter uses `fz_open_flated(..., 15)` for zlib-wrapped deflate.
Other entry points are `fz_open_ahxd`, `fz_open_a85d`, and `fz_open_rld`.
It does not compose filter chains, apply predictors, or validate PDF dictionaries.

`filter-vectors.json` records 12 reproducible smoke vectors and observed native
results. MuPDF accepts the tested missing end markers and truncated zlib trailer;
the safe Rust decoder deliberately refuses these. Native recovery output is not
a reason to relax the Rust strict EOD policy. Binary data, each malformed-filter
failure, and rejection of a 16 MiB + 1 inflated payload were tested successfully.

### Predictor adapter extension (R4)

`mupdf-filter-oracle flate PREDICTOR COLORS BPC COLUMNS` additionally decodes
TIFF/PNG predictor bytes after the Flate stage. Arguments are positive decimal
integers; Colors is capped at 64, BPC accepts 1, 2, 4, 8 or 16, and row size is
checked before invoking native code. Encoded input, inflated intermediate output,
predictor output and each predictor row are capped at 16 MiB. The existing exit
codes and no-partial-stdout guarantee remain unchanged. The adapter invokes
`fz_open_predict(ctx, chain, predictor, columns, colors, bpc)` and checks both
stage stream-error flags. No unsupported predictor number is silently normalized
by the adapter; any such native normalization is reference behavior, not Rust
policy.

Verified TIFF Predictor 2 and PNG Predictor 15 smoke cases, identity Predictor 1,
invalid argument refusals and oversized-row refusal. Native MuPDF accepts any
PNG row filter byte for Predictor 10–15 (for example, Predictor 10 with a Sub row
prefix applies Sub). Differential tests should document their intended policy.

### LZW adapter extension

`mupdf-filter-oracle lzw EARLY_CHANGE [PREDICTOR COLORS BPC COLUMNS]` uses
`fz_open_lzwd(ctx, chain, early_change, 9, 0, 0)`. EarlyChange must be exactly 0
or 1. Optional prediction and all byte/row limits behave as for Flate. Native
smoke tests cover both EarlyChange values, empty clear/EOD, a KwKwK sequence
producing ABABABA, and PNG prediction after LZW. Invalid EarlyChange is rejected.

## Document oracle and rewrite comparison

`oracle/build-document-reference.sh` links `oracle/mupdf-document-oracle.c`
against the same verified static build. It reads at most 16 MiB of PDF bytes
from stdin, disables document JavaScript, requires no password, and loads and
bounds every page (at most 10,000) before reporting JSON `{repaired,pages}`.
Repair state is queried after page loading because MuPDF can repair lazily.
This is a page-geometry oracle, not a general PDF conformance validator.

`oracle/compare-rewrites.mjs` checks immutable fixture hashes, rewrites through
the Rust CLI, requires native reopening without repair, and compares both raw
and decoded streams with pinned MuPDF WASM. It also compares page geometry and
all eligible pages rasterized by MuPDF at 36 DPI, including annotations/widgets.
Identical reference-renderer output validates rewrite preservation; it is not
Rust rendering support. The writer has deliberate signature/linearization and
unknown-trailer exclusions, individually recorded in its report.

The LZW and predictor behavioral references include
`source/fitz/filter-lzw.c` (Copyright 2004–2022 Artifex) and
`source/fitz/filter-predict.c` (Copyright 2004–2021 Artifex).

## Content syntax and logical page streams

`oracle/differential-content.py` groups native lexer TSV with an independent
Python grammar and compares owned Rust operands after native re-lexing of their
canonical serialization. It compares byte strings, names, arrays, dictionaries,
operator order and operand counts; native real values use the documented f32
comparison tolerance. Inline images are explicitly refused before binary input.
`oracle/content-inventory.mjs` records page/Form stream hashes and operator counts;
`tools/check-content-corpus.py` separately checks local operator operand schemas.
Neither tool executes graphics or establishes text/font semantics.

`tools/build-content-reference.sh` builds `mupdf-content-oracle`, which accepts a
one-based page argument and a PDF on stdin, then emits logical decoded content
through MuPDF's public `pdf_open_contents_stream`. This independently checks
MuPDF's array-member padding, including final and empty members. Joining uses
one ASCII space; replacing it with a newline changes comment/string semantics.

## Native logical page-content oracle

`tools/mupdf-content-oracle.c` links the same verified MuPDF 1.28.1 static libraries. Build with `sh tools/build-content-reference.sh /private/tmp/tandem-mupdf-reference`. Its ABI is `mupdf-content-oracle PAGE_NUMBER`, with one-based page index and PDF stdin; stdout contains only exact decoded logical content bytes. Input/output each have a 16 MiB ceiling and document/page index a 10,000-page ceiling. Exit 0 is success, 1 usage/I/O, 2 decode/limit/repair refusal. It calls public `pdf_open_contents_stream`, checks stream errors, checks repaired state after reading, and buffers output before emitting it. Inline-image bytes remain opaque; this tool does not parse or execute content.

The pinned implementation `source/pdf/pdf-stream.c` uses `fz_open_concat(ctx,n,1)` for a Contents array. `source/fitz/filter-basic.c:next_concat` inserts ASCII space after every member, including final and empty members. A direct single Contents stream receives no padding. Independent generated tests confirm empty-part padding, comments continuing through joins, literal strings spanning joins, and separate numeric tokens. `tools/differential-page-content.py` compares this API directly to Rust across immutable fixture hashes; `reference/page-content-differential.json` records exact byte hashes, binary identities, and scope.

`oracle/differential-content.py` compares content grammar using the native lexer plus a separate Python recursive grouping implementation. Rust's canonical operands are re-lexed with the native lexer, preserving operation order, operand counts, byte payloads, container shape, integer/real distinction, and dictionary contents. Real comparisons permit relative 2e-6 / absolute 1e-7 differences from native float precision. Generated ordinary operator names are ASCII, matching the accepted Rust lexer subset. Inline-image cases are strict refusals and their binary bytes are never sent through the native lexer. This is syntax/value evidence, not operator execution, resource resolution, text extraction, or rendering evidence.

## Standalone ToUnicode maps

The U0 Rust parser uses dedicated CMap grammar and typed byte widths, not a
PostScript interpreter. Behavioral sources are `source/pdf/pdf-cmap-parse.c`
and `source/pdf/pdf-cmap.c` (Artifex2004–2021, retained in NOTICE). The measured
font/ToUnicode workload and future boundaries are in CMAP-DESIGN.md and
reference/cmap-inventory.json. Six actual fixture maps contain odd-byte Unicode
destinations; U0 rejects them rather than copying MuPDF's byte truncation.

`tools/build-cmap-reference.sh` checks the pinned CMap source hashes and builds
a test-only adapter. ABI: `mupdf-cmap-oracle QUERY_FILE`, CMap bytes on stdin;
query lines are `lookup HEX` (one through four bytes) or `decode HEX` (zero
through four, `-` means empty). The Rust CLI mirrors this as
`pdf-inspect --cmap=QUERY_FILE [CMAP_FILE|-]`. Native decode recovery is recorded
separately from Rust Incomplete/Invalid errors. The native lookup API discards
source width; explicitly ambiguous numeric lookup cases therefore compare the
Rust result to independent intended mappings and retain native behavior as a
limitation, not as a false parity success. No system CMaps are loaded.

## Explicit simple-font encoding subset (R8 S0)

`oracle/generate-simple-encoding.py` reads only these exact files from the verified
MuPDF 1.28.1 archive, refuses mismatched SHA256 values, parses the four 256-slot
PDF encoding tables plus native exact single-name glyph mappings, and writes
private `src/simple_encoding_data.rs`. Its output included AGPL/Artifex notices
and these source hashes, and was formatted reproducibly with the Rust formatter.
(Clean-room workstream 05 replaced that data: `src/simple_encoding_data.rs` is
now generated by `engine/generate/generate_data.py` from public sources only;
see `../NOTICE`.)

| Source | SHA256 |
| --- | --- |
| source/fitz/encodings.h | 224122c5581a91254a22d3a009c50322d39fbbac5f1b2c1c5464cba8528839de |
| source/fitz/encodings.c | db046231a23e4470c89e31850cff8670a1ca95def64c71d0d4f4c416622fe4a6 |
| source/fitz/glyphlist.h | 16fe0427189e16ff852c02984a49dba184c0db51756b8ecef01524ad6efb6002 |

The registry contains exactly the 378 names appearing in WinAnsiEncoding,
MacRomanEncoding, StandardEncoding and MacExpertEncoding; all have validated
single-scalar mappings. It does not import the complete AGL/TeX registry,
heuristic glyph-name decoding, or host codecs. The pinned upstream generator
reads Adobe Glyph List 2.0 (September 20, 2002) and lcdf-typetools texglyphlist
2.95, with additional filtering and precedence. No new external glyph dataset
has been vendored or relicensed. Exact references and future broader AGL
boundaries are recorded in SIMPLE-FONT-DESIGN.md.

Reproduce the private data with:

```sh
python3 oracle/generate-simple-encoding.py \
  /private/tmp/tandem-mupdf-reference/mupdf-1.28.1-source \
  src/simple_encoding_data.rs
```

The test-only native `mupdf-encoding-oracle` reads no fonts/documents/system
CMaps and emits exactly 1,024 fixed-table TSV rows. Its builder verifies the same
three source files. `reference/encoding-table-inventory.json` records binary
hashes, all rows, separately parsed source-table agreement, and the exact Python
codec differences (40 WinAnsi and 50 MacRoman slots); Python codec values are
comparison data rather than expected PDF mappings. The standalone Rust
Differences adapter has separate typed errors and bounded work, and never
chooses a font's base encoding or silently overrides malformed ToUnicode.

### Typed page placement reference

R8b's separate `mupdf-page-geometry-oracle.c` calls `pdf_page_transform` and
`pdf_bound_page` after loading every page, then reports the final repair state.
It has the existing 16 MiB input / 10,000 page caps and no output on exceptions.
The builder verifies `source/pdf/pdf-page.c` SHA-256
`3d8b0b0852d35449f0d146ad1dd62727d39225c1d3a28681b5ef5d6939ae4258`.
The Rust transform follows the pinned quarter-turn/UserUnit/CropBox placement,
using checked f64 and explicit strict precision/overflow rules. Native's
positive sub-unit rectangle enlargement is not adopted. Comparison report records
binary identities, immutable fixture hashes and f32 tolerance events.

## Standalone path/state interpreter (R9 P0)

`src/path.rs` references pinned MuPDF 1.28.1 path/state behavior while using safe,
private flat-arena ownership. Inspected files carry Copyright (C) 2004-2026
Artifex Software, Inc., retained in NOTICE and the module header:

| Source | SHA256 |
| --- | --- |
| source/pdf/pdf-op-run.c | a269ed581d47fce07ed62f6b0a8f687669d63d7475a62b06516ddadc0bbfff14 |
| source/fitz/path.c | 598278070de5bf44237e75298fdfab7b3a94371107e6acbfc5bebf7013294347 |

P0 preserves path/current-point state outside q/Q, stores paint-time transforms,
and applies pending W/W* only after fill and stroke under the previous clip.
Pending clipping is also outside the saved graphics frame. Source path geometry,
fill rules, device-color values, stroke styles and clip ancestry remain explicit;
there is no rasterization, flattening, color-management or font/resource lookup.

The accepted initial subset refuses cm with an active path/pending clip and Q
when it would change such a path's CTM. It refuses unsupported operators rather
than returning a partially interpreted page. Strict state/EOF errors and empty
S/f/B suppression deliberately differ from native recovery/empty callbacks; the
native differential evidence must distinguish those cases. Integer values beyond
exact f64 conversion are refused; checked arithmetic and transformed control-point
validation prevent publishing nonfinite output geometry. Flat clip/path/dash
arenas prevent recursive ownership and repeated state copying. The capacity
budget conservatively charges cumulative allocation growth without refunds,
including discarded storage, while command/dash/operation limits count all work.

PATH-DESIGN.md records the subset and test strategy. Owner tests exercise state,
geometry, event order, empty clips, strict failures and budgets. Independent
native device traces and state-machine comparisons are separate acceptance work;
matching those traces will validate path execution, not a Rust renderer.

### Standalone path device trace (R9 P0)

`tools/mupdf-path-oracle.c` and `tools/build-path-reference.sh` reuse the pinned
1.28.1 static libraries. Raw P0 content on stdin (4 MiB maximum) executes through
`pdf_process_glyph`/`pdf_new_run_processor` with identity CTM, an empty resource
dictionary and no page flip. JSON has an `events` array; fill/stroke/clip records
include expanded path commands, matrix and pre-event clip depth; paint records
include device color, strokes include cap/join/width/miter/dash/phase, and explicit
Q pops report the resulting depth. Only final implicit processor cleanup pops are
excluded. Output is staged and capped at 64 MiB, with 128 MiB native heap,
one-million token/command and 300,000-event caps, plus CPU10s/wall15s deadlines.
Unexpected native warnings/errors refuse. Operators outside the P0 lexical subset
are refused before native execution. No font programs, rasterizer or page loading
are involved.

`tools/differential-path.py` compares a separately implemented Python state model,
Rust commands and native callbacks. Native move/cubic packing and empty-paint
callback differences are recorded explicitly in `reference/path-differential-p0.json`;
semantic degenerate geometry is not discarded to manufacture parity. See
`REVIEW-R9-P0.md` for the exact comparison boundary and tolerances.

### Native aliased line-fill raster evidence

`tools/mupdf-raster-oracle.c` is a separate test-only pinned native tool, built by
`tools/build-raster-reference.sh`. It emits bounded opaque RGB P6 data from raw
standalone line-fill content, using identity CTM and antialiasing off; unsupported
operators, native diagnostics and limits refuse without rendered output. See
`RASTER-ORACLE-NOTES.md` for ABI/caps, independent integer rectangle checks and
explicit fractional/degenerate/slanted-edge native behavior. This test tooling
makes no production Rust rasterization or general native-pixel-parity claim.


## R11 optional raster dependency boundary

The optional `raster` feature pins published tiny-skia and tiny-skia-path 0.12.0,
std only, with default features disabled. The audited published revision is
`e48b9027762d33731e60e40eb17eaa4f583b0185`; exact crate/source checksums and resolved
features are retained in `tiny-skia-audit.json` and Cargo.lock. Eight additional
packages are resolved only for the optional feature. Full packaged licenses
are retained under `licenses/`, with byte hashes in `raster-license-files.json`.

Our safe adapter owns bounded integer orthogonal geometry, flat masks and RGBA
storage. Backend source inspection informs numerical preflight rather than
copying a general rasterizer into this crate. Internal unsafe in dependencies
is explicitly accepted at the safe public API boundary; it is not an unsafe-free
tree or a proof of absence of backend bugs. The deterministic quantization is
round-half-up to byte Gray/RGB, without ICC conversion; native color parity is
limited to the separately tested endpoint palette.

`tools/audit-raster-allocations.py` copies the resolved source to a temporary
folder and inserts safe capacity logging in PathBuilder and the edge scan
converter. It leaves original registry sources, engine source and Cargo.lock
unchanged. `raster-allocation-audit-r0.json` measures requested capacities and
adds a conservative complete-edge-slice sorting allowance, not allocator
metadata or recoverable OOM. Test-only instrumentation has no custom unsafe
allocator and is not compiled into the production crate.


## M1 URW metric data license

The pinned source contains `resources/fonts/urw/OFL.txt`, not merely the root
AGPL license. Full SIL OFL1.1 and the directory Copyright2016 notice are retained
verbatim in `licenses/urw-metrics/OFL.txt`. Its SHA256 is `3b709798a0943cbf824ed21b6b2c735e482cf6cb8fa11441e57381c2650253ca`.
Both selected AFMs retain their Copyright2014 (URW)++ Design & Development
notices. Generated subsets retain this font-data license; the Rust runtime
code around them is MIT. No source font outline is vendored.


## R13 T0 native positioning oracle

`tools/mupdf-text-position-oracle.c` processes a generated single-page PDF's raw
content under identity CTM, avoiding an implicit page y-flip. It records actual
fill_text/ignore_text device callbacks, reconstructed item/span matrices,
callback CTM, forward page matrix, source cid, Unicode continuation items and
native advance. Output is staged until parsing/execution completes without
repair or native diagnostics. No selected glyph-ID equivalence is claimed.

The adapter caps PDF/decoded content at4MiB, its native allocator at128MiB,
output at64MiB, tokens/items at1million, and CPU/wall time at10/15seconds.
Only raw content/ToUnicode streams and declared Type1 Helvetica/Times-Roman
resources are accepted. Standard system font lookup always returns NULL without
accessing the OS; bundled Base14 resources are used. CJK/fallback hooks throw.
No embedded font program, external stream, font resource expansion or native
production dependency is introduced.

The build script verifies these additional exact source hashes:

- `source/pdf/pdf-op-run.c`: `a269ed581d47fce07ed62f6b0a8f687669d63d7475a62b06516ddadc0bbfff14`
- `source/pdf/pdf-interpret.c`: `dbff6c518bc9c1c9f17bac161538548a95d598d29ab7a4d0e8a130b274b18883`
- `source/pdf/pdf-font.c`: `c0c29c2826883e47191133f66b7d9fb769116f9923419606ac4f7136f54feca5`
- `include/mupdf/fitz/text.h`: `174b56b21d13fe18246427dd68d712b0a81c254fbc17cd48154d023a4c1195c9`

`tools/test-text-position-reference.py` tests native callback reconstruction,
Unicode continuations, raw-code word spacing, invisible intent, CTM composition,
both bundled faces and atomic refusal paths. The independent Fraction model and
160-case preliminary comparison are separately owned by the reviewer and saved
as `text-position-native-preliminary.json`. f64-versus-native-float32 tolerances
are stated by that harness, not inferred from successful output after the fact.


## R14 C0 native mixed program oracle

`tools/mupdf-program-oracle.c` combines the existing P0 path and T0 text device
traces into one ordered event stream. Its identity CTM and unbounded initial
clip are explicit; page transforms are not inferred. Clip event identities
and depth accompany every path/text callback. Actual fill_text paint colors
are recorded; ignore_text has no native color parameter and is explicitly
unobservable, leaving invisible intent color to the independent model.
Explicit Q pops remain ordered; implicit processor-close pops are counted
separately. Native empty paint callbacks and path simplifications are retained
for explicit policy comparison. See `PROGRAM-ORACLE.md` for the bounded ABI.

The build script rechecks all T0 source hashes and additionally verifies:

- `source/fitz/path.c`: `598278070de5bf44237e75298fdfab7b3a94371107e6acbfc5bebf7013294347`
- `include/mupdf/fitz/path.h`: `1644bfa7df3549cf8028b3eba7f93d3473f18fa3116bd8190855c56c64a61bc5`
- `include/mupdf/fitz/device.h`: `6b12f488c9b69eee072d64aed9c5fb956ff39e5924117c077a423ada4d3f7bae`

`tools/test-program-reference.py` passed 42 bounded adapter checks. This is
separate from the independent mixed-state/native program differential gate.
The same native heap/input/output/time/font-loading restrictions as T0 apply,
with explicit event/path-command/clip limits added. No native runtime dependency
or new production font/data source is introduced.


## R15 O0 unhinted simple TrueType outlines

`src/truetype_outline.rs` implements borrowed sfnt bytes with owned checked
table/loca indices and cumulative outline decode budgets. It emits original
points and exact doubled-integer quadratic geometry. Strict checks cover
required tables, byte ranges, overlap, maxp limits, bounds and trailing payload.
Table checksums are retained but not verified. CONTRACT explicitly allows
unaligned table offsets after the measured simple-subset font exposed packed
tables; all other structural range checks remain. No native runtime dependency
or font bytes are added.

Contour start/closure behavior is informed by the pinned FreeType decomposition
source, with original safe Rust ownership and checked integer arithmetic. The
FreeType License and source copyright notices are preserved under
`licenses/freetype-native/`. (`src/truetype_outline.rs` was deleted in Phase 0
of the clean-room plan; with no FreeType-derived code left in the engine, the
notices moved to `../oracle/licenses/freetype-native/` on October 18, 2026.) The full
unaltered FTL.TXT SHA256 is
`5a5ee54c5001bbad1cdc1a57cc3dd4c42199b2da09d39c7ee41fab002d02967f`.

`tools/mupdf-outline-oracle.c` links only the existing native third-party static
reference. It loads explicit GIDs unscaled/unhinted, reverses independently read
header/hmtx bearing normalization, and doubles points before native quadratic
decomposition. This avoids hinted MuPDF outline extraction and midpoint
truncation. See `OUTLINE-ORACLE.md` for input/output/caps and limitations.
The build verifies these source hashes:

- `thirdparty/freetype/src/truetype/ttgload.c`: `627e20092871f554c7eaf719e56dac72497d2d9a1a91b5919ac78bcf74d84deb`
- `thirdparty/freetype/src/base/ftoutln.c`: `e9a31e1a88398615df585a009632c5eebbc456da0e09b6f6ad6ac2965c1bc3b3`
- `thirdparty/freetype/src/base/ftobjs.c`: `e5558a906bfef754843740c55a1110759f23c737303d092ba6fefdce2ba4bb9f`
- `thirdparty/freetype/include/freetype/freetype.h`: `fd45e7ee01c8f0f2419186d3fc9dd58e25cd866ef0c49f0b347c2d141665a364`
- `thirdparty/freetype/include/freetype/ftoutln.h`: `db0585ea338debdb9b721adc5f6d138ec7f88df30ff44cf6fb3cac79a70982f8`

Owner gates: 14 Rust tests and strict library Clippy; native adapter 26 cases;
initial exact owner comparison covers all five measured programs and 1,885
glyph slots (72 nonempty). Independent model/native/adversarial review remains
a distinct gate; passing geometry does not establish code-to-GID selection,
Unicode, advances, hinting, rendering or structured text behavior.

## R23 opaque image loading and exact sample composition

The safe Rust loader validates bounded Gray8/RGB8 image dictionaries, lossless
filters/predictors and normal/inverted Decode pairs. Native image decoding and
original-page execution are comparison tools only. The MuPDF-derived AGPL,
lossless-filter and pinned font notices then in force were in ../NOTICE and
../COPYING (rewritten and removed for the MIT relicense of October 18, 2026).
No new runtime library, native engine fallback or image codec dependency is added.

`image-native-pins.sha256` records the source and local static-library hashes
verified by `../tools/build-image-reference.sh`. The native image trace extends
the copied bounded Form reference without changing the accepted R22 tools.
The image references retain original PDF source operations, asset identity,
unscaled sample bytes and image matrices; their outputs are independently checked.

`../IMAGE-NATIVE-NOTES.md` and `image-reference-policy-r23.json` specify decode,
full-page draw settings and the distinction between exact Rust sampling and
native f32/gridfit/fixed14 behavior. The Python native-policy witness is derived
from the pinned draw-affine/draw-device/image source under AGPL-3.0-or-later.
Every claimed native policy difference needs exact witness evidence; successful
native execution or arbitrary tolerated pixels is not sufficient acceptance.

The final source-verified Rust binary manifest is `image-build-r23.json`.
Stage acceptance and its exact evidence closure are separate reports; absence
or incompleteness of that report means this provenance note makes no acceptance
claim. All earlier accepted evidence/checkpoints remain unchanged.
