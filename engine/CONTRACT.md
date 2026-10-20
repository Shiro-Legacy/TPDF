# Rust engine overnight contract

> Licence note (October 18, 2026): the Rust engine and the app are now MIT, and no MuPDF code ships. Where this document says engine code is AGPL or MuPDF-derived, or that a build owes AGPL obligations, it records the state before the clean-room relicense (see the Licence section of [`PROVENANCE.md`](reference/PROVENANCE.md)).

Authorized in chat on 2026-10-02 at approximately 23:20 America/Toronto: redesign MuPDF functionality around safe Rust ownership and types; retain MuPDF licensing; autonomous implementation and subagent review until 2026-10-03 09:00 America/Toronto (13:00 UTC). This explicit authorization approves the new engine scope and its necessary dependencies, superseding old per-unit approval requirements for this isolated experiment.

## Scope and boundaries

Work only in engine/ in the attached rust-pdf-engine worktree. Preserve the existing Electron application, source, fixtures, and unrelated uncommitted work. No push, publication, app replacement, or production integration. No OCR. Native code is allowed in the reference oracle, not in the Rust implementation; forbid unsafe Rust in this crate. Use the existing stable toolchain through ~/.cargo/bin. No new runtime dependency unless its need is recorded.

Primary reference: MuPDF 1.28.1 official source, SHA256 dc94c60b2537e2ac9a2d379dd3801545f84a3a302d15c9da358362a1270707c3, as recorded by this repository. Reference-derived code retains AGPL-3.0-or-later and attribution. The source oracle is isolated and not a production backend.

## Milestones

R0: crate, pinned source provenance, architecture/feature map, reproducible reference harness.
R1: bounded byte-oriented PDF lexer, explicit token spans and errors, names, integers/reals, literal and hex strings, arrays/dictionaries delimiters, comments and keywords. Preserve PDF bytes; no implicit UTF-8 conversion. Treat limits and malformed input deliberately. Differential tests against pinned MuPDF plus independent edge cases and deterministic stress tests.
R2: owned PDF object model and bounded recursive parser for scalar values, arrays, dictionaries and indirect references; offsets, checked resource limits and explicit unsupported cases. CLI inspection and object-level tests. Stream/document support is separate and must not be claimed based on object parsing alone.
R3 onward: expand only after preceding tests and independent review pass; record scope and acceptance before implementation. Prioritize an actual read-only document path (xref/object resolution/page inventory), then serialization and editing later. Do not silently replace a rigorous milestone with scaffolding.

## Acceptance

cargo fmt --check; cargo test; cargo clippy --all-targets -- -D warnings; deterministic differential/adversarial inputs; independent review fixes with regression tests. Existing fixtures are immutable. Record exact commands/results and limitations. Add limits for token size, nesting, object count and total decoded allocation before exposing parsing to arbitrary inputs. All success claims must name the supported subset. No full-app regression/package gate is needed while changes are confined to this independent engine crate and its docs/tools.

## Coordination

The primary agent owns lib.rs integration, main.rs, Cargo.toml, reference tooling and status. Delegate lexer.rs and lexer tests to one agent; object.rs/parser.rs and parser tests to another after the interface is fixed. An independent reviewer reviews and stress-tests the combined implementation. Agents do not edit one another's files, commit, or push. Recheck the clock before new milestones; at the deadline stop implementation and report.

## R3: read-only classic PDF document path (03:39 UTC)

R1/R2 gate passed: 50 tests including independent review; differential lexer 2008 valid inputs; strict malformed policy checked separately. Continue under user's overnight authorization.

Implement classic xref tables, indirect-object wrappers and owned object lookup with borrowed stream byte ranges; header/startxref/trailer validation; page-tree inventory with inheritance and cycle/ref/limit checks. Initially refuse xref streams, object streams, encryption and incremental/hybrid xref (explicit Unsupported), never heuristically repair. Accept only stream lengths that can be validated against actual file bytes, including resolvable indirect lengths if implemented. Bound total parsed-object work/decoded payload and reject aliased or overlapping xref offsets to avoid repeated parsing blowups. Existing fixtures must remain unchanged.

Add bounded stream decoding for FlateDecode, ASCIIHexDecode, ASCII85Decode and RunLengthDecode. New dependency miniz_oxide =0.8.9 is justified by existing Flate-compressed fixtures, pinned and locally cached; it and its adler2 dependency forbid unsafe code. Preserve their notices. Strict EOD and trailing-data policies documented. Filter chains bound input, each stage, stage count and cumulative allocation; unsupported predictors/filters fail explicitly. No rendering or editing claims.

Ownership: parser agent owns src/document.rs and tests/document.rs; lexer agent owns src/stream.rs and tests/stream.rs; root owns src/pages.rs, tests/pages.rs, CLI integration, Cargo/lib and fixture comparison tooling. Reference agent independently reviews. Required gates: full fmt/test/clippy; compare page counts/geometry of supported project fixtures to recorded MuPDF golden; explicit unsupported fixtures reported; independent adversarial review. Future xref streams/incremental support is R4 after R3 passes.

## R4a/R4b and predictor follow-up (03:55 UTC)

R3 accepted: initial complete gate109tests+7additional independent mapper review tests pass; fmt and Clippy clean. 37/54 fixture documents (560pages) match MuPDF geometry,17 explicitly unsupported. 621 real streams match byte-for-byte (7,282,487bytes); two predictor streams unsupported. 1036 generated filter comparisons and21strict refusals pass.

Proceed with R4a classic incremental /Prev revisions, then R4b xref streams under R4-DESIGN-PROPOSAL.md. Preserve newer-entry precedence including Free tombstones, exact generation matching, physical object/metadata boundaries, and cumulative resource accounting across all scanned sections. Explicit limits on revisions/decoded xref metadata; no repair or encryption. R4b supports bounded direct bootstrap metadata and Flate xref streams with sparse Index; documented self-entry omission policy must be checked. Type2 compressed entries and hybrid tables remain explicit Unsupported pending their own gate. Require the ten incremental fixtures then two stream-xref fixtures to match existing reference page geometry; test generated adversarial revision/index cases.

In parallel extend stream predictor support as a bounded transform (R3b): PNG predictors and TIFF horizontal differencing, with explicit parameter ranges/supported bit depths and row arithmetic checked before allocation. Integrate /DecodeParms without ignoring meaningful parameters. Every inflate/predictor intermediate contributes to cumulative work/output budget. Two existing predictor streams must match MuPDF; unsupported configurations remain explicit. Lexer agent now owns stream.rs, stream_object.rs and their tests for this stage; parser agent owns document.rs and document tests. Reference agent independently reviews both. Root owns integration, docs, acceptance and any separately specified content-operation layer.

## R5a independent object serialization (parallel with R4)

Root may implement a bounded canonical serializer for the already accepted owned Object model. This does not write or edit whole PDF files. Preserve byte strings/names, integer/reference values and real-number type/finite value; escape names, emit binary strings as hex, use deterministic dictionary ordering. Reject invalid manually constructed objects (null name bytes/nonfinite reals), bound output/depth/object counts before growth, and test exact resource edges and parse-serialize round trips including independent MuPDF token checks. Root owns serialize.rs/tests/serialize.rs and CLI integration. No production app changes or file-save API.

## R4c, LZW, and R5b continuation (04:14 UTC)

Combined R4a/R4b/R3b/R5a fmt/test/clippy gate passed. All49 unencrypted fixtures/572pages match; all639 supported-document streams match exactly (7,403,637bytes), zero stream exclusions or failures. Independent R4 review9xref tests+5predictor tests and R5a6serializer tests pass. Proceed under the same overnight scope.

Parser agent implements R4c compressed object streams according to R4-DESIGN-PROPOSAL.md, owning document.rs and document/object-stream tests. Add explicit cumulative container/member/decode/work limits; only bounded direct bootstrap metadata initially, supported identity/Flate filters, no recursive/compressed containers or encryption/hybrid support. Preserve winning-entry precedence. Require generated valid PDFs to match native MuPDF with no repair, plus adversarial independent review and the unchanged fixture gate.

Lexer agent may add LZWDecode (9–12-bit codes, PDF EarlyChange0/1) with checked fixed-capacity dictionary state, bounded expansion, explicit malformed/truncated/EOD/trailing rules, predictor integration and full DecodeParms handling. Own stream.rs, stream_object.rs and their tests. No new dependencies. Native oracle and independent encoder must compare width changes, dictionary resets, full dictionary, KwKwK and predictors; do not treat shared encoder/decoder roundtrips as sufficient. Coordinate native oracle edits with reference agent.

Root develops R5b a bounded full-rewrite serializer for already readable unencrypted documents. Preserve effective live object IDs/generations and every stream's raw bytes; serialize dictionaries with correct direct Length; write deterministic classic sparse xref tables and a fresh trailer with effective Root, discard obsolete incremental/xref metadata. No incremental-save claims, signing, or app integration. Initially refuse signature-bearing documents, linearized inputs and meaningful unsupported trailer metadata explicitly. No file overwrite API: return a new bounded byte buffer, CLI stdout only if added. Validate native MuPDF can reopen outputs without repair; compare page geometry and decoded streams for eligible fixtures and generated cases. Independent reviewer assesses scope before acceptance.

## R6a content syntax and bounded page-content loading (04:33 UTC)

R4c/LZW/R5b implementation and independent review accepted after the /First header token-boundary fix. Combined test gate204tests passed before final13reviewtests/CLItest; targeted finalreview tests/fmt/clippy pass. Realrewrite33PDFs554pages600streams and generatedrewrite96PDFs286pages384streams preserve nativegeometry, raw/decodedstreams, and36DPIpixels withoutnative repair. Reference comparison is preservation evidence, not Rust rendering.

Proceed to CONTENT-DESIGN.md read-only content layer. Lexer agent owns new content.rs/tests/content.rs: bounded incremental operations with byte operands and spans, a ContentOperand type that cannot contain indirect references, dedicated contentgrammar (no fileparser lookahead), immediate explicit refusal of BI before binary data, strict structuralword rejection, arbitrary ordinary operator preservation for later typed validation. Bound input, tokenwork, nesting, per-operation operands, operationcount and cumulative ownedpayload/objectnodes. Sticky failures; EOF with pending operands is malformed. Compare generated and actual supported content token sequences to pinned native lexer and independent grouping, not only shared roundtrips.

For page-content loading the filter decoder must expose actual cumulative intermediate output usage. Lexer agent owns backward-compatible decode-with-usage APIs in stream/stream_object; preserve old API behavior and tests. Parser agent owns new page_content.rs/tests/page_content.rs only after agreement on the usage API, loading PageInfo contents under cumulative referenced-input, decoded-work, combined-byte and stream-count limits. Repeated references must charge repeatedwork; do not rely on unique-streamcount. Define actual stream-array joining semantics from pinned MuPDF/source and content token continuity before claiming compatibility; exclude inlineimages explicitly until their own gated stage.

Root owns CLI/lib integration and a separate typed content-operation validation layer if needed. No rendering, text extraction, resource expansion, page editing or inlineimageboundary guesses are claimed. Reference agent independently reviews grammar/workbudgets and native comparisons. Keep existing document/rewriter corpus gates passing. No new dependencies or app changes.

## R6b geometry foundation (04:48 UTC)

Root may add a small safe geometry module for the later graphics interpreter: finite points and normalized rectangles tagged by coordinate space, affine transforms typed from one space to another, checked composition/inversion/bounds conversion. Constructors reject nonfinite coordinates; operations return explicit overflow/singularity errors. Private fields prevent bypassing invariants; coordinate-space mismatches must fail at compile time. No graphics execution, rasterization, clipping, text, or app integration is implied. Verify transform order with independent known-coordinate examples, inverse and transformed-bound properties, zero-area rectangles, singular/overflow cases, and compile-fail documentation tests. Reference agent independently reviews. This module must not change existing page geometry until a separately tested integration.

## R7 U0: strict standalone ToUnicode maps (04:58 UTC)

R6a accepted:264combined tests/fmt/Clippy;890content grammar cases/31,221operations;590typed-schema streams;579native page-joining cases (572fixture pages +7boundary cases). R6b accepted after independent exact-singularity regression:6root+6review geometry tests,2compile-fail doctests, additional external tag checks; no page integration.

Proceed with U0 in CMAP-DESIGN.md. Lexer agent owns src/cmap.rs/tests/cmap.rs: typed width-preserving character codes, codespace validation, strict bfchar/bfrange and array destinations, validated nonempty UTF16BE scalar sequences, exact count/terminator grammar and bounded inert wrappers. Private fields preserve map/code/Unicode invariants. Reject overlaps, prefix ambiguity, malformed UTF16, unsupported UseCMap/CID/notdef/predefined semantics, unknown executable words and range overflow. Retain exact offsets and bounded errors. Use the design's caps including4MiBinput,64codespaces,100records/section,100000records,262144expandedentries,64destinationunits,16MiBownedpayload and bounded wrapper nesting. Sequential ranges cannot carry to earlier UTF16 units or cross invalid surrogate boundaries. Numeric source bfranges above65535 are explicit U0 Unsupported, matching the documented native subset; wider bfchar/code-boundary cases must keep independent expectations and native representability distinctions visible.

Parser agent owns a new test-only native CMap oracle/buildscript using the pinned source/library and its bounded lookup/decoding protocol; coordinate protocol with lexer/root. Reference agent owns independent generated standard-library Python oracle expectations/differential tools and adversarial review. Root owns lib/CLI integration and acceptance. No engine dependency additions, font fallback, text extraction, document font loading or rendering claims in this stage. The four strict-valid real maps must enumerate exact mappings against native; six malformed maps must fail explicitly with destination offsets and native recovery recorded separately. Root may update notices/provenance for exact source references.

## R7 U0 accepted; R7b bounded mapped-string decoding

Strict CMap review found no blocking issues. Six independent tests and native/independent comparisons pass:164valid maps,3680mapped entries,8402queries; all517entries in the four strict-valid actual maps enumerated.28generated malformed and six actual malformed maps refuse; destination offsets/native recovery independently checked. Keep native widthless lookup and malformed decoding recovery as separate differences.

Lexer agent may implement src/text_decode.rs/tests/text_decode.rs: decode bytes only through an explicitly supplied validated ToUnicodeMap, with no font/resource fallback. Return validated Unicode text plus per-character-code source-byte and UTF8-output spans, preserving ligatures, combining sequences, U+0000, U+FEFF and supplementary scalars. Borrow original input bytes rather than copying them; resulting text need not retain a map borrow. Keep result fields private with immutable accessors. Missing mapping, incomplete code, invalid code and limits are distinct errors at source offsets, with no partial-success output. Bound input bytes, decoded codes, scalar work, UTF8 output and allocations; repeated codes charge repeated work. Use checked geometric growth, not quadratic exact-size append loops. Default suggestedcaps1MiBinput,262144codes,1million scalars,4MiBUTF8 output. Empty input is valid. No replacement/guessing/Identity-H-as-Unicode, reading-order inference, font metrics or text extraction claims.

Root integrates a test/inspection CLI if needed; reference agent independently tests source/UTF8 span correctness and generated byte-string sequences against expected Unicode plus native code/lookup queries. Existing CMap and engine gates must continue passing. Parser agent remains on read-only simple-font fallback design until separately authorized.

## R8 S0: explicit simple-font encodings

Proceed independently from R7b with SIMPLE-FONT-DESIGN.md S0. Parser agent owns src/simple_encoding.rs, generated private table data and their tests. Implement exact WinAnsi/MacRoman/Standard/MacExpert named tables from the pinned-source/native evidence and the declared378-name Unicode registry. Keep byte glyph names and distinct UndefinedCode, UnknownGlyphName and valid Unicode outcomes. No Python codec substitution, implicit base encoding, broad glyph-name guessing, truncation, replacement characters, font loading, metrics or rendering. `.notdef` is explicit undefined; unknown opaque glyph identifiers (including manually provided NUL/high bytes) stay unknown and retain bytes. This does not relax PDF lexer's existing name syntax policy.

Differences must be bounded and transactional: checked0–255 integer resets, names require a preceding code,255mayendthenrequireanotherreset, explicitlast-assignmentprecedence, everyprocessedentry/namebyte charged including overwritten values. Suggestedcaps4096items/127bytespername/256KiBprocessednamebytes; atmost256retainedslots. Preserve exact names and immutable base on failure. A checked Object-array adapter reports the failing Difference index and rejects wrong types/references. No clone of whole input Object trees.

Reference agent independently compares all1024 base slots and378 names to native plus parsed pinnedtables, and generated override/reset/malformed/limit cases. Root integrates library/inspection CLI and verifies all51measuredmissing-ToUnicode font contexts are represented by explicit named bases. This is a standalone byte encoding, not automatic font fallback: malformed explicit ToUnicode remains an error, and Identity-H remains CID semantics. Retain exact source hashes/AGPLattribution for generated data. No new runtime dependency. Accept only after combinedfmt/test/Clippy and independent review.

## R8b: typed page geometry integration

Root may implement a small page_geometry module connecting accepted PageInfo to the tagged geometry types. Validate all public PageInfo geometry again, including finite positive normalized boxes, crop containment, UserUnit range and quarter-turn rotation, before constructing private invariant-bearing PageGeometry. Provide PDF user-space to top-left 72-point page-space transform, typed cropped bounds and checked device scale. Preserve input and existing page inventory behavior. No rendering, clipping, empty-box repair or pixel allocation. Positive sub-unit boxes retain their actual dimensions; MuPDF's minimum-size recovery must be reported separately if tested. Refuse arithmetic overflow and dimensions collapsed by representational precision.

Verify all four rotations, offset/negative origins, UserUnit, inverse round trips, invalid manually constructed PageInfo, and overflow boundaries. Compare transform coefficients and bounds on all supported fixture pages and independently generated normal boxes against the pinned native page-transform API. Root owns page_geometry.rs/tests/page_geometry.rs, a separate native oracle and CLI adapter. Reference agent independently reviews after S0; no existing oracle output changes.

## R9 F0 and P0 continuation

S0 independent review accepted: 210 cases, 53,760 compared slots, every fixed table entry/registry name, 31 strict CLI refusals; 12 owner and four independent library tests pass. R7b combined gate passed 318 tests. R8b typed page geometry remains under review; its precision-consistency issue is being fixed independently.

Lexer agent may implement F0 under FONT-RESOURCE-DESIGN.md in font_resources.rs and dedicated tests. Use a document-bound lazy loader, validated page membership, nearest whole Resources inheritance without key merging, exact byte names and complete object identities. Support selected Type1/TrueType declared mappings only, direct and indirect font dictionaries, authoritative strict one-byte horizontal ToUnicode, and explicit S0 encodings/Differences. Cache successful immutable fonts/maps without cloning alias payloads; cap aliases, names, fonts, maps, references, decoded input/work, mappings/scalars and Differences work across the session. Reserve bounded attempted decode/parse work before calls and retain reservation on failure, since nested failures do not expose exact usage. Meaningful stream UseCMap and unsupported types fail explicitly; no fallback after rejected ToUnicode. Reference resolves optional null consistently and documents native differences. Required real gate: 572 resource tables, 582 bindings, 54 eligible simple identities/575 registrations, 51 exact 256-slot encoding maps plus 54 entries across three simple ToUnicode maps; seven Type0 refusals remain separate from malformed CMap tests. No metrics, embedded glyph selection, Form expansion or text execution.

Parser agent may implement P0 under PATH-DESIGN.md in path.rs and dedicated tests. Return private immutable flat path/clip/dash arenas with ordered fill, stroke, clip and restore events. Strict bounded q/Q, stable CTM per active path, checked cm composition, path construction/shorthand expansion, line state and explicit device colors only. Current path and pending clip live outside saved state; fill then stroke use old clip, then pending clip applies. Preserve signed rectangle winding, hairlines and degenerate geometry; no bounds-as-clipping shortcut. Refuse unsupported operators, unresolved state, mixed-path CTM changes, integer-to-f64 values outside exact conversion and invalid/overflowing geometry. Charge cumulative discarded work and actual retained capacities, not only live lengths; no recursively owned clip chains. No partial display list on failure. Required independent state/geometry/event model plus bounded pinned native device trace comparisons with f32 differences/normalization explicit. No rasterization or full-page support claims.

Root integrates APIs/CLI, owns R8b geometry and gates. Reference agent completes R8b review then independently reviews F0/P0; agents may create scoped native tools after coordinating file ownership. Avoid editing shared lib/main or rebuilding a binary during a hash-stable reference run. No new runtime dependencies.


## R9 acceptance and measured scope correction

F0/P0 accepted after independent review and 385 combined tests, formatting and
strict Clippy. F0 matches all 572 page resource tables/582 bindings:575 loaded
simple bindings and seven Type0 refusals. Page-bound font identities are53,
comprising50 explicit encodings/12,800 slots and three ToUnicode maps/54entries.
The earlier54-font proposal included p0b/certified-p2-filled.pdf font12:0, which
native inspection confirms belongs to XObject/Form13:0 only. No scope expansion
was made to conceal this distinction; that font stays outside page-only F0.
Differences error indices were fixed after independent review.

P0 compares310 programs/5,835 events against native callbacks and independent
state expectations, plus20strict no-stdout refusals. Flat ownership, cumulative
capacity/discarded work and finite control points pass independent tests.
Native empty-paint callbacks are a separately recorded difference.

Next designs are read-only: lexer measures font metrics; parser audits a possible
pure-Rust raster dependency and compares a bespoke subset. Neither new runtime
dependency nor raster/font-metric implementation is authorized yet. Reference
runs a deterministic bounded mutation smoke test on existing stages. Root owns
integration and will record the next concrete scope after these findings.

## R10 M0 and D0: explicit advances and declared text decoding

R9 mutation smoke passed1,600 bounded deterministic cases with no panics, signals,
timeouts or partial output on failure. This is smoke evidence, not semantic
validation of malformed PDFs or coverage-guided fuzzing.

Lexer agent may implement M0 in FONT-METRICS-DESIGN.md: immutable finite signed
horizontal FontAdvance values and exact FirstChar/LastChar/Widths ranges, with
observable WidthsEntry/DeclaredMissingWidth/DefaultMissingWidth provenance.
Preserve explicit zero/fractional/negative widths and reject inexact integer
conversion. Add a separately lazy metrics load/cache within the document-bound
resource session without changing Unicode load behavior. Resolve individual
values and descriptor MissingWidth under bounded existing references plus new
attempt/processed-width/retained-metric counters. Absence of Widths remains
NeedsBuiltinOrFontProgramMetrics; incomplete declarations are invalid. No font
program decoding, glyph selection, builtin guesses or text execution. Require
all five real explicit-width fonts/160 declarations and all1,280 byte lookups,
plus independent malformed/limit/cache tests. Builtin AFM/CFF parity investigation
may continue read-only; M1 runtime tables are not authorized yet.

Root may implement D0 by refactoring text_decode.rs around one private bounded
output/span accumulator shared by strict ToUnicode and explicit SimpleEncoding
lookup. New simple encoding decoding preserves one source code per input byte,
valid Unicode sequences and existing exact spans/work limits. Undefined code and
unknown glyph name must be distinct errors, separate from ToUnicode missing/
invalid/incomplete codes. A small font_text.rs dispatcher accepts a validated
LoadedSimpleFont and selects its declared mapping; it never falls back after a
ToUnicode error and does not clone whole maps/encodings. Preserve all existing
ToUnicode behavior, lifetimes and tests. No positioning, reading order, metrics
application or replacement characters. Independent checks cover every predefined
slot, Differences names/unknowns, exact limits and mixed valid/error prefixes.

Reference agent builds bounded raster test tooling while the raster design is
pending, then independently reviews M0/D0. Parser remains on raster design; no
new Rust runtime dependency or raster production code authorized yet.

## R11 R0: optional bounded raster backend

The pinned tiny-skia0.12.0 std-only audit probe compiles successfully with isolated
Rust1.85.1. Root authorizes exact optional dependency tiny-skia=0.12.0 under a new
`raster` feature, default features disabled, std only. Default/core builds retain
their current dependency set. Root owns Cargo.toml/Cargo.lock and feature-gated
lib/CLI integration. This changes the prior no-new-runtime-dependency constraint
for this recorded purpose: producing pixels from accepted path events through a
mature pure-Rust rasterizer. No native production backend is introduced.

The engine continues to forbid unsafe code. tiny-skia and transitive dependencies
contain internal unsafe, including scalar representation/array casts and numeric
invariants; this audited safe-public-API boundary must be disclosed in README/
NOTICE, not described as an unsafe-free dependency tree. No backend unsafe API,
SIMD feature, PNG, FFI or C build is authorized. Retain exact package checksums,
complete license notices and source-audit evidence. Genuine backend process OOM
is not a recoverable guarantee; our own image/mask buffers use fallible allocation
and source-based hidden allocation limits must be measured before acceptance.

Parser agent may implement raster.rs and tests under the final RASTER-DESIGN.md
R0 domain: preflight the entire immutable PathProgram before any rendering;
opaque white RGBA8 output, solid Gray/RGB fills, both fill rules, binary clips
and restore events. Only simple nondegenerate orthogonal contours whose fully
transformed device coordinates are exact integers within the canvas. Refuse
fractional/slanted/cubic/self-intersecting/zero-area contours, Stroke, CMYK and
unsupported suffixes explicitly. Canonicalize away empty move/close runs to
avoid backend iterator recursion, while retaining meaningful contour winding.
No silent snapping, partial render, bounds-as-clipping, or fuzzy pixel-parity
claim. The native oracle proves fractional, slanted and degenerate edge coverage
differences; those remain documented outside R0.

Bound dimensions4096/pixels4194304, path and event counts, transformed command
work, pairwise contour validation, scanline quadratic-edge estimate, all pixel
and mask passes, clip ancestry and retained mask bytes. Plan all storage/work
before execution, use flat mask caches, pass identity to backend and disableAA.
Explicitly audit every void-backend early return as benign or precluded within
this domain. Use an immutable private plan/image API and errors with event span.
Reference agent owns native raster oracle and independent mask expectations,
including200 initial native cases; root integrates CLI/raw PPM and reviews.
Require new feature and core gates, Rust1.85.1 build, feature tree/notice checks,
independent resource-limit review and exact pixels on accepted cases before
acceptance. This is a bounded orthogonal vector renderer, not full PDF rendering.

## R10 M0/D0 acceptance and R12 M1 builtin advances

M0/D0 independent review found no remaining blocker. M0 matches five real fonts,
160 explicit widths (90zero) and all1,280 code/provenance lookups;48fonts require
builtin metrics separately. D0 compares110 strings/50,674codes/98,146UTF8bytes,
every746defined and278undefined predefined slots plus378registry names;
286strict refusals pass. Shared ToUnicode regression106cases/6,332codes remains
unchanged. Six independent tests pass. Root is completing the combined default
check while R11 optional raster work proceeds.

Lexer agent may implement M1 under the revised FONT-METRICS-DESIGN.md, owning
builtin_metrics.rs, reproducible private tables, loader adapter and tests.
Use only the654 exact AFM/CFF shared names for each verified Helvetica/NimbusSans
and Times-Roman/NimbusRoman face:1,308 shared widths matched exactly. Exclude the
402 AFM-only entries from runtime claims. Preserve exact byte-name lookup,
UndefinedGlyph and MissingBuiltinMetric outcomes; do not use Unicode to select
advances or silently supply .notdef width. Retain exact source/CFF hashes and
full applicable URW notices. No font program runtime dependency or font files.

Add a separately lazy load_builtin_metrics API/cache so existing M0 explicit
width behavior stays unchanged. Require exact Type1 BaseFont Helvetica or
Times-Roman, absent/null Widths and no incomplete FirstChar/LastChar declaration,
no non-null embedded FontFile/FontFile2/FontFile3 in a valid descriptor, and an
explicit S0 encoding independently of ToUnicode. No subset stripping, aliases,
implicit encoding or TrueType fallback. A malformed explicit Widths must never
be hidden by builtin fallback. Reuse bounded reference/metadata/encoding budgets
and explicit cumulative load/retained-data limits; successful aliases share cache.
Unicode loading stays independent. Require all48eligible fixturefonts/all12,288
byte outcomes, every1,308exactnames, excluded/missing names and adversarial
selection/embedding/encoding/cache cases. Root owns CLI/differential integration;
reference reviews independently after raster gates.

## Raster color contract clarification

R0 preserves the audited explicit conversion: finite device Gray/RGB components
in[0,1] quantize round-half-up to8bit, with Gray copied to all three channels.
Independent expectations test this exact policy including ties. Exact native
whole-pixel comparisons use the endpoint palette (components0 or1); native
color conversion can differ for other colors even at interior pixels, including
0.5 mapping to127 versus our128. Record these differences separately and never
claim universal native RGB/Gray or ICC-managed parity. Geometry/clip coverage
for the accepted integer orthogonal domain still requires exact comparisons;
no fuzzy pixel tolerance is introduced.

## R13 T0: bounded standalone horizontal text positioning (06:32 UTC)

R11/R12 accepted: 444 all-feature tests on 1.98.1 and minimum 1.85.1, formatting,
strict Clippy and wasm32 library compilation pass. Raster and builtin metric
independent/native gates are recorded in their reviews. Proceed with T0 under
TEXT-POSITION-DESIGN.md and the user's overnight authorization; no new runtime
dependency. This is positioned source-code tracing, not glyph rendering or
complete page text extraction.

Root owns src/text_font.rs borrowed validated font bindings/registry and tests,
lib integration, CLI and acceptance. Lexer owns src/text_position.rs, execution
state, output arenas and owner tests. Parser owns the bounded native text oracle
and source attribution. Reference owns the independent rational model, generated
differential and adversarial review. Agree public APIs before dependent edits.

Adopt the proposed one-byte horizontal mapping/advance separation, private typed
matrices and IDs, flat owned result arenas, cumulative limits and sticky atomic
failure. Support the listed text operators, Tr0/3 and q/Q/cm outside BT only.
Refuse unsupported mixed page operators and q/Q/cm inside text objects; current
PDF 2.0 errata and pinned native matrix-save behavior differ. Preserve decoded
string ranges plus enclosing operation spans; do not label them raw-file spans.
No fallback Unicode, invented glyph IDs, silent builtin selection or reading
order claims. Each source code receives one advance regardless of Unicode count.

Require at least 200 independent arithmetic programs, at least 100 native
supported integer-width cases, limit/atomicity/provenance tests, full compiler
and regression gates. Fractional/negative declared-width native quantization and
float32 arithmetic differences must be separated from Rust f64 expectations.
Implementation must not progress to page adapters or outlines before T0 review.

## R14 C0: standalone mixed path/text coordinator (07:00 UTC)

R13 T0 accepted: 470 tests on both compiler versions, fmt/Clippy/wasm checks and
246 exact model / 160 native positioning cases pass. PAGE-PROGRAM-DESIGN.md and
PAGE-PROGRAM-REVIEW.md are approved as direction. Split implementation into two
reviewed units: first a standalone mixed coordinator with injected TextFonts;
only after its gate, a document-bound page/resource/session adapter. The current
543-page operator inventory is a ceiling, not accepted page execution.

Lexer owns src/program.rs and owner tests plus minimal path.rs/text_position.rs
introspection, global source-index injection and internal budgeted pushes. Root
owns CLI (--program=FONTSFILE), integration and acceptance. Parser owns a native
mixed device oracle; reference owns independent mixed-state/event expectations
and adversarial review. Preserve existing standalone public APIs.

Keep one persistent P0 and T0 executor, one coordinator admission decision for
q/Q/cm, CTM/depth equality assertions (signed zero equal), exact global operation
indices independent of child work counters, and sticky atomic failure. Capture
current clip/fill-color context for ordered text events. Refuse active/pending
path at BT, path construction/paint inside BT and q/Q/cm inside BT; permit only
specified colors/styles inside BT. Unsupported resource/marked/image operators
must never be skipped. No complete page, glyph rendering or reading-order claim.

Use pre-admission aggregate execution allocation/work budgets. For a consistent
cumulative allocation ledger, strengthen P0 allocation accounting from capacity
growth to full requested targets at each reserve; retain its old API but document
this stricter budget interpretation. T0 already uses full targets. Add crate-only
budgeted pushes that temporarily cap child allowance to current usage + parent
remaining before any reserve, then charge actual deltas, including failure work.
Coordinator buffers likewise charge full requests before growth. Never run both
children under independent full allowances and check the aggregate afterward.
Parsing remains a separately declared bounded input phase in standalone C0;
input font registry/map storage remains separately caller-bounded. The future
page adapter must account repeated parsing/content/font work across calls.

Require independent/native mixed event ordering/state/context comparisons,
original-index and suffix atomicity tests, every shared budget boundary,
second-child failure, pending empty clip, color changes inside text, save/restore
around text, initial clipping and noncommuting transforms. Rerun accepted P0/T0
comparisons after shared internal changes and complete normal/MSRV/wasm gates.
No new runtime dependency. Document session implementation remains the next unit.

## R15 O0: independent simple TrueType outline unit (07:04 UTC)

This unit depends on the accepted parsing/geometry foundations, not the active
mixed-program coordinator, and may be developed in parallel. Approve the scoped
TRUETYPE-OUTLINE-DESIGN.md: safe dependency-free standalone sfnt 1.0 head/maxp/
loca/glyf reading with checked table index, explicit caller GID, empty/simple
unhinted glyphs, exact raw points and doubled-integer quadratic commands.
Composite glyphs, variations, other containers/outlines, hint execution, cmap/
PDF glyph selection, advances and rendering remain explicitly outside scope.

Parser owns src/truetype_outline.rs, owner tests, bounded native FreeType oracle
and provenance. Root integrates library/CLI and reviews structure. Reference
independently reviews/model-compares after its C0 gate; O0 cannot be accepted
without that separate review. No new runtime dependencies or runtime font files.
Preserve borrowed font bytes and immutable owned indices/results; cumulative
session budgets charge repeated/failed calls and allocations before work.

Use the design's strict table/padding/maxp/bbox rules initially. If measured valid
corpus compatibility requires adjustment, record evidence and a concrete policy
change here before changing acceptance; root may decide under existing overnight
authority. Do not automatically ask the user. Source checksum verification is
not claimed. Test-only FreeType uses unhinted/unscaled outlines, reports and undoes
its independently derived bearing normalization, and doubles points before
canonical decomposition. No pixel/glyph-selection claim follows from outline
parity. Gate all 1,885 fixture slots (72 nonempty) plus independently generated
valid/adversarial fonts, cumulative limits, normal/MSRV/wasm and exact geometry.

O0 policy adjustment — 07:14 UTC: measured p3/simple-subset.pdf font 3:0 has
SHA-256 17ca6c7141568ebe100f390d894d7f240d2e6502a429fac16c10b80789dbabc5
and 2,630 decoded bytes. Its nonoverlapping in-bounds sfnt tables include hhea at
682, hmtx at718, loca at738 and post at2579, so the proposed four-byte-alignment
restriction rejects an existing native-readable fixture. Approve unaligned table
offsets in the borrowed byte reader. Preserve checked bounds, directory exclusion,
nonoverlap and exact original offsets. This is a documented compatibility policy,
not an assertion that the font follows OpenType alignment recommendations. Safe
byte reads require no aligned pointer access. Add a regression and update design.

## R16 PP1: document-bound page-program adapter (07:15 UTC)

R14 C0 accepted: 492 all-feature tests on 1.98.1/1.85.1 plus fmt/Clippy/wasm;
220 mixed programs, 26 atomic refusals, refreshed P0 and T0 comparisons pass.
Proceed with the document session half of PAGE-PROGRAM-DESIGN.md and its review.
This may run beside independent O0, which is not a dependency of page tracing.

Lexer owns src/page_program.rs, session/resource/parser usage plumbing and owner
tests. Root owns integration, compact/full CLI and fixture/native corpus tooling.
Reference reviews PP1 after O0; no acceptance without separate review. Existing
original document/fixtures remain immutable. No new runtime dependency.

Library policy defaults to DeclaredOnly; AllowPinnedBase14 is an explicit named
session policy recorded in output. CLI inspection deliberately enables the latter
and says so. Use one document-bound FontResourceLoader across pages, only used Tf
names (including selections with no shows), and immutable prepare/freeze font
bindings. Only exact missing-width-source status permits M1; malformed widths
never fall back. Retain exact joined PageContent bytes and occurrence segments.
Use typed page placement once and initial page-bounds clipping.

Use two bounded parse passes and one persistent C0 program per page. Preserve
original operation indices and decoded-string/member provenance. Expose owned
PageProgram results that outlive session. Page failure publishes no partial trace;
prior successes remain usable. Repeated calls and failures consume session work.

Choose explicit nonoverlapping session budget partitions for content/font decode,
parser input/nodes/operations, retained joined bytes, preparation metadata and C0
execution requests. Prior already-open Document work is separately bounded and
must not be claimed inside the session envelope. Every phase admits remaining
allowance before execution/allocation. Where exact failure usage is unavailable,
reserve a documented worst case, refund verified unused work only on success,
and retain failed reservations. Never discover an aggregate overshoot only after
running a child at its independent full cap. Report partitions honestly instead
of calling an after-the-fact sum a universal memory/RSS guarantee.

Gate all572 readable fixture pages, separating500 stress pages from43 other
inventory candidates; explain candidate reductions and all29 expected refusal
pages. A compact corpus trace may retain only the full semantic data needed for
native comparison, but must compare every accepted source-code transform/Unicode,
path event, color/clip relation and order—not only counts or first/last samples.
Full single-page CLI retains the complete C0 trace and source metadata. Corpus
output remains bounded and sessions reuse the opened document/cache. Add all
reviewed cross-stream/resource/state/failure/budget scenarios and compiler gates.


## Morning continuation (October 3)

User requested no more 30-minute messages and expressly authorized continued work.
The recurring heartbeat is paused and must remain paused. Resume the already
approved R16 PP1 scope interactively, with the previously authorized subagents.
No production application integration or publication is included. Current writes
remain confined to engine/. Root verifies the compiler and corpus gates before
claiming this milestone complete; partial native coverage is reported explicitly.


## R16 PP1 acceptance — October 3 morning continuation

Accepted after 538 all-feature tests on both supported toolchains, formatting,
strict all-target Clippy and minimum-toolchain wasm compilation. Native comparison
matches543/572 fixture pages (500stress+43other), with29 classified refusals;
220generated PDF pages also match the exact independent page-transformed model.
Independent review17tests and oracle/comparator26checks pass. See STATUS.md,
REVIEW-R16-PP1.md and reference/page-program-differential-pp1.json.

Review clarified the allocation ledger into explicit independent partitions:
prior opened Document, session inventory under FontResourceLimits.pages,
font/cache work, parser nodes/payload/tokens, and measured full output allocation
requests. Failed conservative reservations remain charged. Page groups/default
device-color overrides refuse. Geometry is validated once with errors retained
per page, preserving successful neighboring-page traces. No scope expansion to
missing Encoding, glyph selection/rendering or production app integration.


## R17 G0: embedded simple TrueType glyph selection (October 3 continuation)

User requested the next step. Implement a bounded format4 cmap reader and a
separate actual-PDF font loader/glyph policy that bridges source bytes to O0
unhinted simple outlines. No pixel rendering, page-trace integration, CFF, Type0,
composites, shaping, system font lookup or new runtime dependency is authorized
by this milestone. Automation remains paused; work is interactive.

page_adapter owns truetype_cmap.rs, owner tests, and an immutable crate-private
O0 table accessor. Root owns embedded_glyph.rs, PDF load policy/CLI/integration.
page_oracle owns bounded native cmap/PDF glyph tooling and differential reports.
page_review independently audits code and writes adversarial tests. Preserve all
previous subsets, source/font licenses and immutable fixture hashes.

Format4 selection is explicit by directory record; the parser never selects a
Unicode policy. Validate cmap directory, selected subtable range/language0,
ordered disjoint segments, canonical FFFF-to-zero sentinel, glyph-array-relative
offsets, wrapping delta semantics and every mapped GID before accepting selection.
Search accelerator fields are not trusted or used. Duplicate selectors are not
silently chosen by PDF policy. Query and validation budgets remain cumulative,
including failures, and results distinguish missing GID0 from malformed data.

PDF policy follows the pinned MuPDF3/0 symbolic or3/1-then1/0 non-symbolic branches
only. Flags32 overrides Flags4 as in the reference. Symbolic fonts require absent
Encoding for the initial subset. Non-symbolic fonts require one of the existing
explicit S0 encodings; unsupported glyph-name/name-table/fuzzy fallback is an
explicit per-code refusal, not guessed by ToUnicode. ToUnicode is never read by
this capability. Preserve native raw-byte seed and successful override provenance;
font GID0 is a distinct missing result, not an automatically rendered outline.

Loaded font bytes and requested font/resolved program identities remain owned.
Descriptor/encoding reference identities are not retained. Actual PDF font
loads use a document-bound cumulative decoder/metadata budget; prior Document work
is a separate prerequisite. cmap/O0 sessions use their own explicit partitions.
A single successful code can be decoded to O0 geometry without claiming all256
slots or all page content are supported. CLI exposes all slot statuses and exact
outline geometry for resolved nonzero slots with atomic overall output.

Gate all9 real format4 subtables exhaustively and native/model generated fonts,
all5 actual PDF font dictionaries and256 code slots with honest percode unsupported
classification, matching resolved GIDs/outlines. Add tests for symbol F000 lookup,
non-symbolic encoding names/MacRoman space alias, fallback precedence, forged GID,
truncation/range/arithmetic, repeated failed work, decode budget and ToUnicode
independence. Finish normal/MSRV/wasm/fmt/Clippy plus independent review. Do not
broaden any tolerance or claim glyph raster/pixel parity.


## R17 G0 acceptance — October3 interactive continuation

Accepted with571 all-feature tests on both Rust1.98.1 and minimum1.85.1,
formatting, strict all-target Clippy and all-feature minimum-toolchain wasm build.
The19 independent review tests found no remaining blocker; loader/atomic CLI
checks add6 and the cmap owner8. Native/model/Rust comparison matches15,007,744
16-bit character-code queries over9 real and220 generated records exactly.
All1,280 PDF slots are classified:71 mapped and989 missing match native, while
220 remain explicit unsupported name fallback. Every native nonzero selection
and all71 corresponding O0 outlines/1,544commands match exactly. Comparator
fault injection rejects13semantic mutations;15native routing checks pass.

Fixed review findings: cumulative selected-segment accounting, failed source-code
attempt admission, one MacRoman table construction per map, and optional present
Font/FontDescriptor Type validation. The native notes now distinguish actual
Missing/null policy from native GID0/.notdef rendering. Requested-font/resolved-
program IDs are retained; a full metadata reference history is not claimed.
Budget partitions and fixed256-slot name lookup are documented explicitly.

Scope stays resource-level simple TrueType glyph identity/unhinted outlines:
no page glyph placement, text pixels, composites, CFF/Type0, implicit encoding,
post-name/fuzzy fallback, app integration or native runtime dependency. Automation
remains paused. Reproduction/report/checkpoint paths are in STATUS.md.


## Continuing rewrite authorization — October3, after R17

The user explicitly requested: "continue and you can keep going until the engine
rewrite in rust is done". This supersedes the original09:00 deadline for engine
implementation. Continue through the remaining engine replacement scope and its
reference/regression gates; do not declare the whole rewrite complete at a subset
milestone. Existing isolated-worktree, app-preservation, licensing, no-push and
no-publication boundaries remain. The thirty-minute automation remains paused;
this authorization does not resume it or schedule replacement messages.

An active task goal tracks the full rewrite. REWRITE-ROADMAP.md will map current
application engine needs to concrete missing capabilities and completion gates.
Unknown or unsupported functionality must remain visible in that map.

R18 next: document-bound page glyph placement for the supported embedded simple
TrueType subset. Retain the complete PP1 trace and attach source-code/GID/outline
identity with page-space geometry, operation/event/clip/color provenance and
normalization by unitsPerEm. Preserve Unicode and PDF width independence and
existing PP1 refusals. Budget repeated font loads/opens/cmap/decode/transform work,
cache/output allocation, and failures cumulatively. Immutable cache/output binding
must prevent a glyph map or outline being paired with another font. No pixel
rendering claim until a separately validated raster milestone.

page_adapter owns new page_glyph.rs and owner tests; root owns CLI/lib integration,
status and acceptance; page_oracle owns actual native/model placement gates;
page_review owns engine completion inventory and independent review/tests.
Require actual PDF and generated transform/spacing/page geometry comparison,
explicit unsupported classifications, no widened prior tolerances, full normal/
minimum-toolchain tests, wasm compilation, formatting and strict Clippy.


## R18 G1 acceptance

Accepted after594 all-feature tests on Rust1.98.1 and minimum1.85.1, formatting,
strict all-target Clippy and minimum-toolchain wasm compilation. Independent
review13 tests, owner6 (including atomicCLI suffixfailure) and horizontalmetrics4
pass. Actual drawing-origin normalization by lsb-header.xMin is implemented;
rawO0 outline semantics remain unchanged. Shared immutable cache data and phase
budgets include failedwork, repeatedopens, lookupscans and metric queries.

All572 readable fixture pages attempted:2accepted/matched,570explicit refusals,
with identical native supported/refused status and no skipped/documenterror cases.
This is the narrower glyph subset; R16's543 accepted traces are not withdrawn.
All237 generated pages (220dyadic+1explicitbearing+16actualfont programs) match,
covering2362codes/32434commands and2202 nonzerooriginshifts. Native maximum
coordinate difference9e-6 stays within inherited T0/P0 policy; dyadic rational
comparisons are exact, explicit1000-unit bearing allows1ulp as documented.
Direct hinted fz_outline differences are observed separately, not declared parity.
Comparator29semantic mutations reject,3baselines and10nativewrapper checks pass.
Reports and hashes: reference/page-glyph-differential-g1.json and
reference/page-glyph-reference-validation-g1.json; review REVIEW-R18-G1.md.

The next implementation is R19: a bounded safeRust scanline supersampler and page
raster integration. Existing R0 tiny-skia behavior remains unchanged. No new native
runtime or dependency; reference rendering stays native/test-only. Record the
exact R19 contract/API before starting that source work. Automation stays paused;
the full rewrite goal remains active and incomplete.

## R19 implementation authorization and interfaces

R18 checkpoint: `/private/tmp/tandem-rust-engine-r18-checkpoint.tar.gz`, 366 files,
SHA256 `8b568ed9432fe9bc3a71191313f21724f3db4f107b0332729c5de2c97e99db88`.
The user's continuing authorization now permits R19 source work under the final
owned-scanner recommendation in R19-RASTER-AUDIT.md. This supersedes that file's
proposal-only status, not its numeric and independent-verification requirements.

Root owns `src/scan_raster.rs`: safe checked rational line intersections on the
1/256-pixel lattice, sample centers32+64k, half-open scan conversion, metered heap
sort and callback spans. Public ScanPoint{x:i32,y:i32}, ScanEdge{start,end},
ScanSpan{y:u32,x0:u32,x1:u32}; coordinates checked against absolute2^29 on every
scan. ScanSession::new(ScanLimits), usage(), scan(edges:&[ScanEdge], FillRule,
sample_width:u32, sample_height:u32, callback:impl FnMut(ScanSpan))->Result.
ScanLimits/ScanUsage cumulative fields: edges, rows, work, allocated_bytes,
spans, samples (limits use max_ prefixes; work and samples u64, others usize).
Dimensions refer to4x sample space; nonzero dimensions at most2^23. All work and
span samples charged before callback; callback must perform only bounded writes
in caller-reserved storage. Scanner retains scratch and cumulative reservations
including failed attempts. No new runtime dependency, existing R0 unchanged.

page_adapter owns page_raster.rs and owner tests: bounded curve flattening,
quantization (nearest with half ties away fromzero), implicit closure, immutable
clip masks AND, mixed event order, opaque Gray/RGB, Tr0/Tr3, white opaque result,
4x4 sample resolve. Initially accept only positive uniform scale+translation
page targets, validating original initial page bounds without snapping. Stroke
or CMYK paint refuses before pixel allocation. Public PageRasterSession owns
cumulative budgets; render takes &PageGlyphProgram, RasterTarget and per-call
limits, returns owned image; no reusable uncharged public render plan.

page_oracle owns native/reference pixel comparisons and comparator validation;
page_review owns independent per-sample rational model and adversarial tests.
Root owns CLI and integration after scanner. Tests must cover offcanvas winding,
coincident crossings, selfintersection, holes, nested/restored/idempotent clips,
curve overshoot and limits, page boundary fractions, glyph/path transforms,
paint order, invisible and empty glyphs, atomic refusals and cumulative budgets.
R19 is a filled-page subset milestone, never completion of the whole rewrite.

## R19 R1 acceptance

R19 owned filled-page rasterization is accepted after independent source review,
630normal andMSRV1.85.1 tests, strictalltarget/allfeatureClippy, formatting and
MSRV wasm32 library compilation. Final CLI SHA256
06fccd5d08917c87d74508af3d8250f44ba7c32c99fc8ee16a928d1c156945cb.
442generated PDFs:322controlled exact native4x/model,108exact independent line
models,12curve cases (10matching outputs;2arch cases with4differences entirely
inside76explicitly uncertain pixels). All572readable fixture pages attempted:
2rendered,570atomicrefusals; both prior R18 eligible pages preserved. Arbitrary
nativeAA/hinting differences remain diagnostic and boundary-confined, not parity.
Validation14model checks,4positive baselines,23rejected mutations,25nativewrapper
checks and2Rust atomic suffix checks pass. Review REVIEW-R19-R1.md; reports
reference/page-raster-{differential-r1,reference-validation-r1}.json. Rootverified
all23evidence hashes. Raw2218gzip outputs reside in /private/tmp. No production
application changes or new runtime dependency. Fullrewrite remains unfinished.

The next selected capability is the reviewed R20-BASE14-OUTLINE-PROPOSAL.md:
licensed canonical font assets for exact Helvetica/Times-Roman, typed G1 sources,
explicit independent outline policy and bounded binding/placement. Record its
final production contract after saving the R19 checkpoint, before source edits.

## R20 implementation authorization

R19 checkpoint is saved and verified:388files,23evidencehashes;
`/private/tmp/tandem-rust-engine-r19-checkpoint.tar.gz`, SHA256
2f18dccf78f2a12e8c41d01627733325f811f28f4f5ede4483ac538ab354485d.
Proceed with R20-BASE14-OUTLINE-PROPOSAL.md under the user's continuing full-rewrite
authorization. The two licensed pinned URW canonical outline assets are necessary
runtime font data, not a new runtime library/native dependency. Preserve their
OFL notices separately from AGPL implementation code. All other faces/CFF parsing,
malformed embedded fallback, implicit Encoding and app integration remain outside
this milestone. Do not imply that R20 completes the full rewrite.

Root owns builtin_outlines.rs, low-level tests, CLI/lib integration and the new
page_raster CubicTo arm. page_adapter owns embedded_glyph loader source dispatch,
PageOutlinePolicy (lower-level/reexport), page_glyph typed source/cache/placement
and owner tests/migration. page_oracle owns generator/native oracle, generated
builtin_outlines_data.rs, manifest and independent native/model comparisons.
page_review owns independent asset integrity/binding/raster tests and its TT
review-test migrations. No overlapping file edits without coordination.

Exact data interface: private child builtin_outlines_data.rs imports
`super::{BuiltinOutline,BuiltinOutlineCommand,BuiltinFace}`. It exports to parent
static HELVETICA and TIMES_ROMAN [BuiltinOutline;654], with corresponding
HELVETICA_COMMANDS and TIMES_ROMAN_COMMANDS arrays. Record fields: face:BuiltinFace,
name:&'static[u8], native_gid:u16, command_start:u32, command_len:u32. Command enum
uses named i32 fields MoveTo{x,y},LineTo{x,y},CubicTo{x1,y1,x2,y2,x,y},Close.
Sorted exact names, bounded signed coordinates, closed contours, source checksums,
units1000 and actual font matrix/origin are generator/independent gate invariants.
Root exposes lookup(face,name), glyphs(face), record getters and a conservative
lookup_work_bound(face) including comparisons/name bytes (asset names<=128bytes).
Static bytes are a fixed documented data partition; runtime maps and every lookup,
occurrence, transformation and failed attempt remain under G0/G1 ledgers.

Retain default EmbeddedOnly outline policy, independent from metric fallback.
Only explicit AllowPinnedBase14 permits the two exact Type1 face declarations.
Declared PDF Widths retain control of advances; Encoding names select outlines
independently of ToUnicode. Unsupported or malformed embedded programs never fall
back. Common page provenance remains typed; TT-only metadata moves to an explicit
source branch without fake object IDs/TrueType GIDs/cmap identities for CFF assets.
CLI page rendering explicitly opts into the pinned policy after this milestone;
legacy glyph inspection can retain EmbeddedOnly for prior differential reproduction.
Add a clearly named combined-outline inspection mode for R20 reference use.

Require reproducible asset bytes/membership/hashes, all1308native name/outline
comparisons, fontmatrix/drawingorigin evidence, independent table integrity,
encoding/width/ToUnicode/policy/cache tests, failures and exact cumulative budgets,
new page/raster comparisons and unchanged TT behavior, complete corpus accounting,
mutation/failclosedwrapper checks and normal/MSRV/fmt/Clippy/wasm gates.


## R20 B0 acceptance

R20 pinned Helvetica/Times-Roman outline binding and placement is accepted after
652normal/MSRV1.85.1 tests, strictClippy, fmt andMSRVwasm gates. All1308canonical
records/30626commands match fresh native queries with pinned asset identity; two
generations reproduce the exact source. Full corpus543/572outline pages match,
includingall500stress pages;29explicitrefusals. All220generatedmixed-statepages
match independent rational state/native geometry. Corpus223657glyphs/3006160commands
pass10603730rationalcoordinatechecks; maximum error5.64e-14 within derived bounds.
43fixturepages render (upfrom2),529atomicrefusals (all500stress stillneedstrokes).
16smallCFFcases:11exact,5with7pixel differences inside118uncertainpixels;0certified
mismatches. Native4x/AA8 diagnostic differences22852/33948corpus pixels,0outside
fixed boundary band. Final6positive,47mutationrejects,17nativewrapper and43independent
canvasdimensionchecks pass. Root verified51final evidencehashentries. Source/native
notes/reports in STATUS.md; CLI SHA256
358f47bc3b6c56d036a8c925ed1d9814f4ea9632a5e29a87f17a73d886ea51c1.
No runtime native/CFF dependency, no app integration, no fullrewrite completion.
Save the R20 checkpoint before beginning reviewed R21stroke implementation.

## R21 implementation authorization

The R20 checkpoint is saved and verified before these production edits:
417files,51evidencehashentries,2370513bytes; archive
/private/tmp/tandem-rust-engine-r20-checkpoint.tar.gz, SHA256
7d0716a2347819f938b08d0cda489562972e6cb5d036630104f12e2d054ed1ed. Continue
under the user's full-rewrite authorization with the reviewed solid-stroke stage
in R21-STROKE-PROPOSAL.md and R21-STROKE-REVIEW.md. This is a bounded rendering
capability, not the full engine replacement. No new runtime dependency is adopted.

page_adapter owns src/stroke.rs and tests/stroke.rs: owned UserSpace component
expansion, solid positive-width line bodies and admitted regular cubics, original
caps/joins/seams, outward numeric predicates and cumulative resource accounting.
Root owns lib export, page_raster integration, CLI/regression migrations and
integration tests. page_review owns independent stroke tests/model/source review
and its review_page_raster migration. page_oracle owns native/model raster gates,
corpus/refusal accounting and harness fault checks. Coordinate changes to shared
files. All implementation remains inside engine/; the automation stays paused.

The exact public interface and fields/defaults are in the proposal's concrete API
appendix. StrokeSession::new(StrokeLimits), usage(), expand(commands,style,dash,
user_to_device,StrokeLimits) returns owned StrokeOutline of Move/Line/Close user
commands. The validate_style free function performs constant-size style/dash
consistency checks. A saved empty dash array is solid; meaningful dash arrays,
hairlines, ambiguous degeneracies/cusps/reversals and unsupported folds refuse.
No omitted dash argument can silently remove a declared pattern.

Use consistent positive-winding closed convex components scanned together as a
nonzero union. Expand width/caps/joins in user space; apply the full saved affine
map exactly once during raster planning. Curved leaves require positive derivative
projection, outward no-fold radius*curvature<=1/2, strict convex endpoint sections,
shared subdivision endpoints/normals and bounded paired-boundary displacement.
Four fixed 1/256-device-pixel allowances cover center, tangent, arc and numeric
error, using only relevant allowances per construction and at most1/64 before
existing1/256 quantization. Carry subdivision uncertainty with safe outward
interval arithmetic; unresolved predicates subdivide or refuse. Round sectors use
bounded normalized-vector bisection and conservative sagitta, without unproved
platform trigonometric assumptions. Structural depth<=24 and coordinate guards
remain effective even when callers raise logical limits.

PageRasterLimits.stroke bounds all expansion work on one page, additionally
intersected with the session's per-page raster policy; PageRasterSessionLimits.stroke
bounds all pages cumulatively. One temporary StrokeSession serves every stroke in
a page under the intersection of page cap and cumulative remaining cap. Checked
remaining/intersect/accumulate helpers avoid duplicated ledger arithmetic. Merge
admitted work on success and every failure. stroke_usage() exposes its separate
partition; source input, stroke geometry, raster plan and scan buffers retain
explicit accounting. Full-page style/color validation and complete geometry
preparation precede pixel allocation. A failed suffix never publishes an image.

Acceptance requires independent analytic lines/caps/joins, true-cubic interval
membership certificates with reported uncertain samples, shear/reflection/scale,
closed seams, clipping and event order, numeric and exact budget boundaries,
repeat failures and owned output lifetimes. Preserve all R20 accepted geometry and
raster pages; all500 stress pages must render before claiming that corpus unlock.
Keep native minimum-width/AA differences separate from geometric stroke correctness.
Complete native/comparator fault gates and current/MSRV tests, fmt, strict Clippy
and wasm builds. Every remaining stroke/rendering/application gap stays explicit.

## R21 acceptance

R21 is accepted after684 current/MSRV tests, formatting, strict Clippy and MSRV
wasm checks;110 independent generated stroke cases;543/572 corpus renders at18DPI
including all500stress pages; preservation of43prior72DPI pages and3stress72DPI
representatives. There are29 explicit corpus refusals. Final CLI SHA256 is
21e1a4ac418c8610c67e923c7579f55accd661c65675c3a843ba067cdf4f0198.

The independently reviewed source-color amendment is explicit: native truncation
and Rust nearest rounding produce exactly(229,102,51)/(230,102,51) on certified
opaque stress-rectangle interiors. All92,300 raw outside-band differences per
native variant remain reported; none are unexplained. Geometric bands were not
widened, and exact pair checks also reject equal-but-wrong pixels. All589 rendered
corpus rows retain the initial failed-run Rust/native/trace/input hashes; only
source-derived classification changed. Validation passes7 positives,53 mutations,
33 exact-exit native wrapper checks and3 actual Rust atomic suffix refusals.
Root verifies both final report hash maps (35 entries), alongside source review
and unchanged R20 baselines. Reports and STROKE-NATIVE-NOTES.md are frozen.
Save the R21 checkpoint before R22 production changes. The full rewrite remains
active and incomplete, with the recurring automation paused.

R21 checkpoint saved:442files,86evidencehashentries,2659704bytes;
/private/tmp/tandem-rust-engine-r21-checkpoint.tar.gz, SHA256
fb64cec50a94bdcb23fb10e684d15c07f888be193c41203353dda40d28963011.

## R22 implementation authorization

Begin after the accepted R21 checkpoint recorded above. Continue under the user's
full-engine rewrite authorization with R22-FORM-PROPOSAL.md and
R22-FORM-REVIEW.md. Implement bounded ordinary Form XObjects; images, transparency
groups, annotation appearances and broader unsupported operators remain explicit
refusals. The measured immediate gain is two Form-only fixture pages, not all
sixteen pages whose first refusal was Do. No new runtime dependency is required.

page_adapter owns form_resources.rs, scoped font_resources.rs loader hooks,
page_program.rs traversal/provenance and ContentParser/Lexer consuming checkpoint
hooks, with dedicated tests. Root owns program.rs, text_position.rs, text_font.rs,
any required path.rs typed boundary hooks, exports and CLI origin output. page_review
owns independent state/scope/cycle/budget/ownership tests and model. page_oracle owns
new native Form-aware reference and corpus/model/fault evidence. Coordinate shared
files; all edits remain inside engine/, automation paused.

Use one persistent C0 interpreter. Entry/exit each consume one global execution
index with synthetic span0..0; typed internal q/cm/re/W/n and Q operations retain
all child work/allocation charges and attach events to the boundary origin.
Form Matrix composes before caller CTM; BBox retains its actual transformed sides.
Opaque single-use LIFO frames and an interpreter-owned save floor prevent child Q
from popping implicit or caller saves. Entry/exit require outside BT, no active or
pending path and locally balanced state. Failure is sticky and never publishes a
prefix. Preserve the existing q/Q/cm-inside-BT restriction. Inherit and restore
selected resolved font IDs, colors, clips, line and text parameters.

Separate stream identity, explicit Resources-provider identity and effective
invocation scope. Missing/null Resources inherits the caller; an explicit dictionary
replaces lookup scope without native outer-frame name fallback. Reject meaningful
Group/OC/Ref/OPI/alternate dispatch/default-device-color overrides. Prepare every
used Tf by (scope, original name) before execution, including unused selections.
Indexed font selection must validate the original name and prepared scoped ID;
public name-based registry behavior remains unchanged. Never cache painted results.

Pass one admits each logical attempted Do once, including failed resolution, and
records an invocation plan. Pass two replays that plan without charging a second
logical invocation; it pays all actual parser/interpreter/lookup work. Repeated
calls after return execute again; only active-ancestry resolved stream identity
constitutes a cycle. Unique stream/scope admission counters are separate from
per-invocation output references, and failed admitted work is retained.

Explicit bounded DFS uses consuming non-Clone parser checkpoints preserving all
cursor/limits/counters/sticky errors. An immutable retained Arc keeps source bytes
alive throughout suspension; pointer/length checks are identity comparisons only.
No suffix parser reset or suspension refund is allowed. Failed nested phases retain
all active conservative reservations. Every result owns source/provenance data.
Reserve bounded diagnostic storage lazily before first Do, in its separate checked
partition; failure fills/moves no-grow <=64-frame records without copying source
payload or requiring exhausted output capacity. No-Do pages need no diagnostic
allocation. Errors retain resolvable object/span/callsite chains after private
execution output is discarded.

Intersect per-page and cumulative session limits; precharge metadata/reference/name
comparisons, decode/parser work, repeated execution, output/origin/frame storage and
full allocation requests. Explicitly assign one owner partition to every buffer or
Arc handle. Fixed structural depth<=64 applies regardless of logical caps. Merge
usage on success and failure. Preserve no-Form numbering and all accepted prior
geometry/raster behavior, including independently documented native color rounding.

Acceptance requires both actual Form-only pages, complete remaining-refusal census,
independent native entry/exit and exact affine/state/provenance checks, resource
shadowing/inheritance/aliases, repeated use under different caller state, true BBox
clip versus AABB, nested malformed suffixes/cycles/DAG quotas, parser checkpoint
source/sticky/quota preservation, owned lifetime and exact/one-less budget tests.
Complete current/MSRV tests, formatting, strict Clippy and wasm build plus reference
fault checks before accepting R22. This capability does not complete the rewrite.

## R22 acceptance — 2026-10-06

R22 ordinary Form XObject execution is accepted after the complete native/model/corpus gate. Scoped resources and fonts, nested Forms, affine Matrix/BBox clipping, state restoration, owned provenance and cumulative failure budgets are verified. These are reusable drawing streams, not interactive AcroForm fields.

- Rust 1.98.1 and minimum 1.85.1: **724 tests pass each**. Formatting, strict all-target/all-feature Clippy and minimum-toolchain wasm32 library compilation pass.
- **58 positive generated Form graphs**, four explicit native-fallback/strict-scope differences and ten malformed/cycle/state refusals pass. **66 generated renders and 10 independent pixel models** pass with zero certified channel-interval violations.
- **545/572 corpus pages render at 18 DPI**, with 27 explicit refusals; all 500 stress pages pass. All **48 supplementary 72-DPI renders** pass. The two new Form-only pages render at both resolutions. All **589 previously accepted render hashes remain identical**; all 29 old refusal pages receive a Form-aware census.
- Reference validation: 23 native comparator mutations, 25 Rust comparator mutations, 13 native process faults, six Rust process faults, three actual native negative-wrapper cases and eight pixel/baseline mutations are rejected as expected.
- The new `tools/verify-form-gate.py` requires exact corpus identities/counts, all generated/model/census coverage, preserved hashes and current evidence hashes. Ten deliberately shortened or corrupted reports are rejected. It prevents a successful `--skip-corpus` or `--native-only` run from being treated as final acceptance.

Native AA/edge differences and the existing source-certified 229-versus-230 color policy remain diagnostic. No tolerance or policy was widened. This is a bounded Forms milestone, not complete PDF rendering or application parity.

## R23a sampling implementation — 2026-10-06

The user resumed work after R22 acceptance. Implement the exact, allocation-free affine image sampling foundation in R23-SAMPLING-DESIGN.md under the continuing rewrite authorization. This is a component stage, not full R23 acceptance. Image loading, ordered page rendering and native/corpus integration remain separate gates. Preserve the accepted R22 checkpoint and prior reports; no runtime dependency or app changes.

### R23a verification — 2026-10-06

Implemented the R23a exact affine image-sampling foundation in `src/image_sampling.rs`. It uses bounded signed 256-bit arithmetic for determinant/coverage and exact nearest-neighbor row/column selection under rotation, reflection and shear. Device edge ownership, top-down source rows, singularity/numeric refusal and cumulative work/attempt budgets are explicit. No heap allocation, unsafe code or new dependency.

Verification: **740 tests pass on Rust 1.98.1 and minimum 1.85.1**, including 16 new arithmetic/sampling tests. Formatting, strict all-target/all-feature Clippy and minimum-version wasm32 library compilation pass. The separate Python Fraction polygon/ray-crossing and Gaussian-elimination model matches **10,084 cases**: 1,358 selected samples, 7,118 outside samples and 1,608 explicit refusals. Five altered/omitted-output mutations are rejected. The final numeric comparison report is `reference/image-sampling-differential-r23a.json`.

This is a tested numeric component, **not complete image support or R23 acceptance**. Image dictionary/filter/predictor validation, retained decoded assets, scoped Do dispatch, ordered image events, sample-buffer composition, native image comparisons and the full image corpus gate remain to implement. Page execution/rendering is unchanged. R22 remains the latest accepted integrated rendering milestone.

The implementation-session source audit does not replace the separate arithmetic review required before page integration.

## R23b image loader — 2026-10-06

Continue under the user’s explicit instruction to finish the rebuild unless stopped. Implement R23-IMAGE-LOADER-DESIGN.md, preserving R22 and R23a checkpoints. Full R23 acceptance still requires scoped page integration and native/model/corpus gates.

## R23 integrated implementation and final evidence gate — 2026-10-06

Proceed under the user's continuing rebuild authorization and explicit authorization for three subagents. The implementation contract is R23-IMAGE-INTEGRATION.md, with ImageLoader and exact sampling contracts in their separate design files. Independent source review and accounting fixes are recorded. Preserve the R23a numeric and R22 integrated checkpoints. All 796 current/MSRV tests and fmt/Clippy/wasm checks pass; final native/model/corpus verification is still required. Final acceptance must certify every newly supported corpus output, preserve all593 previous successful render hashes, record every old refusal, pin source/binary/tool evidence, and reject omitted/duplicate/corrupted reports. Raw native differences must have source-derived witnesses, never an unconstrained diagnostic bypass. No complete engine/application rebuild claim follows from this image subset.

## R23 accepted; pause after checkpoint — 2026-10-06

The complete image gate passed:796current/MSRV tests, fmt/strictClippy/wasm;265positive decoded-image matches;28generated exact models/source-native witnesses;632corpus attempts,617successful outputs,593preserved prior hashes and24new corpus certificates plus2mixed cases. The final verifier rederived original-source/native-trace evidence and rejected29structural/4raw mutations. Preserve the source/evidence checkpoint before R24 changes.

The latest user instruction is to stop after this checkpoint. After verifying the R23 backup, pause the rebuild goal and perform no R24 implementation until the user resumes. Keep R23 oracles/policies/reports frozen and preserve617successful hashes. R24-OPAQUE-SOFT-MASK-DESIGN.md and its reviews are prepared, but the full rebuild in REWRITE-ROADMAP.md remains incomplete.

## R24 resumed by user (2026-10-06)

The new user instruction “work on r24” resumes this milestone after the verified R23checkpoint. Implement the reviewed R24-OPAQUE-SOFT-MASK-DESIGN.md scope only. R24-IMPLEMENTATION.md records809current/MSRVtests, full corpus/pixel/provenance evidence and19verifier mutations. Independent source review is still pending, so R24 is not marked accepted. Retain the independent review requirement before advancing. Current delegation instructions require explicit user permission; one review subagent was requested in chat. Automation remains paused. No application/fixture changes, commits, pushes or publication.

## R24 independent review and acceptance (2026-10-06)

The user explicitly requested “do a independent review,” authorizing a separate reviewer. R24-INDEPENDENT-REVIEW.md records two acceptance-verifier findings and their fixes. No production Rust defect was found. All 814 current/MSRV tests and required build/style/WASM checks pass; the complete 634-attempt corpus preserves 617 accepted hashes and certifies two new renders. The reviewer independently replays opacity/source-glyph evidence and rejects all 407 omission/identity/check mutations; the final verifier rejects 23 built-in mutations. Both findings are resolved, with no remaining blocker. R24 is accepted for the design's bounded opaque-mask subset. The preceding pending-review text is historical. Do not start another milestone under this R24-only request; automation remains paused.
