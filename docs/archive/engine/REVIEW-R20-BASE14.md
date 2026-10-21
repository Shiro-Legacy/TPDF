# R20 independent review: pinned Base14 outlines

## Scope and current disposition

The reviewed production subset adds canonical unhinted outlines for exact Type1
Helvetica and Times-Roman declarations, using the same two pinned URW faces as
M1. Source review found no remaining blocker. R20 is accepted after the targeted
independent tests, reproducibility/native/model reports and root integration gates
recorded below. This milestone does not complete the engine rewrite, provide
runtime CFF parsing, add other Base14 faces, or claim ordinary native raster parity.

## Binding, provenance and transformation

The outline policy is explicit and independent of metric fallback. A font with
valid declared PDF widths can request canonical outlines without replacing those
widths. Encoding and Differences choose the exact glyph name; ToUnicode retains
its separate text-extraction role. The tests deliberately map byte A to the fi
outline while ToUnicode produces XY and PDF width 200 produces a 2-point advance
at 10-point size. Native font widths and glyph bounds cannot alter that advance.

The loader dispatches by resolved subtype and policy, rather than catching an
embedded-font failure. Exact face names are required. Optional descriptors must
have dictionary shape and a valid optional Type. FontFile, FontFile2 and FontFile3
are resolved under the existing reference ledger: meaningful values and malformed
or dangling references refuse; absent/null values permit this capability. Explicit
Encoding remains required. Undefined names, .notdef and names outside the fixed
asset refuse, while a valid empty space outline succeeds.

Common page source/event/font provenance remains available. TrueType metadata is
contained in EmbeddedTrueTypeSource; builtin records expose asset identity and an
asset-local CFF GID, with no fabricated PDF program object or TrueType cmap fields.
Owned encoding/cache records and immutable static outline references remain valid
after input bytes, document and session are dropped. Existing TT review tests were
migrated only to the typed getter, retaining their assertions.

Canonical integer coordinates are divided by 1000 and transformed through the
existing text placement once. CFF does not receive the TrueType bearing correction.
An independent hand-derived model checks a noncommuting graphics CTM, text matrix,
horizontal scale and rise for every move, line and cubic control/end point. The
raster cubic arm uses the existing bounded flattener and target transform. Native
FontMatrix and actual drawing-origin evidence is required separately; the Rust
coordinate tests do not establish those source-font properties by themselves.

## Asset integrity and resource review

The private generated constructor prevents caller-created invalid record ranges.
Independent tests enumerate all 654 records per face and compare exact membership
with M1, sorted unique names, unique native GIDs, contiguous command ranges, units,
exact lookup pointer identity, coordinate bounds and closed contour grammar.
Expected command totals are 12,788 Helvetica and 17,838 Times-Roman (30,626 total).
The explicit .notdef record is inspectable but cannot be selected by PDF text.
Static byte reporting is a logical data partition, not an executable-size or RSS
claim. Runtime does not parse CFF or depend on the offline native generator.

The cache key retains the requested PDF font identity: resource aliases share a
record; different dictionaries with the same face keep different byte encodings.
Each admitted builtin font initializes 256 fixed slots under lookup work and cache
allocation ledgers. Each first successful byte choice charges a cached choice;
failed/undefined choices stay uncached and charge bounded exact-name lookup again
on every retry. The bound includes name comparisons, not just search iterations.
Per-occurrence command count is admitted before both command traversals; cubic
control points charge three transformations. Output allocations and successful
repeated placements remain charged even when static outlines are reused. Failed
attempts retain reservations. The independent exact/one-less tests cover cache
choices, transformed commands, points, lookup work and requested allocations.
Existing G0/PP1 and TT partitions retain their own counters and limits.

## Independent checks completed

`tests/review_builtin_outlines.rs` contains 13 tests covering the properties above,
including all policy combinations, wrong/subset/case face names, malformed program
fields, null versus meaningful programs, source lifetime, failed lookup retries,
transform equations, visible cubic raster output, Tr3 invisibility and depth/work
refusals. All 13 passed together with 13 migrated page-glyph and 16 page-raster
review tests (42 targeted tests total). Production owner tests and root integration
gates are reported independently in the final evidence below.

## Integration gate update

Root reports 652 tests passing on Rust 1.98.1 and MSRV 1.85.1, plus strict
all-target/all-feature Clippy, formatting and the MSRV wasm32 library build.
The final CLI used for native comparisons has SHA256
358f47bc3b6c56d036a8c925ed1d9814f4ea9632a5e29a87f17a73d886ea51c1.
Two root asset/membership tests and two CLI policy/provenance tests pass.
BASE14-OUTLINE-DESIGN.md matches the inspected implementation. Its owned binary
search fixes comparison-count behavior independently of the standard library and
fits the existing conservative admitted lookup-work bound. No source changes are
requested by this review. Final native/model evidence follows.


## Final native/model acceptance

Inspected the four saved B0 reports and BUILTIN-OUTLINES-NATIVE-NOTES.md. Every
file digest listed in the reports' evidence maps was independently recomputed
and matched. The oracle confirms its final files are frozen.

`reference/builtin-outlines-differential-b0.json` exactly matches all 1,308 asset
records and 30,626 commands against the native faces, including names, native
GIDs, units, advances, hashes and six valid empty integrity records. Two fresh
generator runs reproduce the checked-in source and manifest. The generated source
is 2,190,965 bytes with SHA256
d811729c399645d18c013015871b98f9ed22a3f7e2902e84d2c55fa9602ef4dd.
The compiled target's logical static data partition is 908,842 bytes. Independent
CFF metadata inspection confirms the omitted FontMatrix/default 1/1000 identity
and zero translation; actual PDF page drawing confirms the unhinted origin and
normalization. Original OFL notices are retained under
`reference/licenses/urw-outlines/`.

`reference/page-outlines-differential-b0.json` attempts all 572 readable corpus
pages: 543 match native outlines and 29 refuse explicitly, with matching native
status. All 500 stress pages match the geometry gate. Accepted corpus pages have
223,657 placements (223,634 pinned and 23 embedded) and 3,006,160 commands. The
220 generated mixed-state pages also match, covering 2,200 placements and 40,480
commands. Native maximum coordinate differences are 0.000193 for the corpus and
0.000009 for generated pages, within unchanged T0/P0 policies. Independently
rational coordinate checks use per-operation enclosures: corpus maximum error
5.64e-14 and maximum enclosure radius 1.25e-13; generated maximum error 1.55e-14.
Identity, Unicode, source byte, font/name/GID/hash, topology/ranges/order, clips
and state remain exact assertions. Hinted native geometry remains diagnostic.

`reference/page-outlines-raster-b0.json` attempts all 572 pages: 43 render and 529
refuse atomically. The 500 stress pages still refuse rasterization because they
contain strokes; geometry acceptance is not a claim they render. The 43 rendered
pages contain 20,746,368 pixels. Native unhinted 4x AA0 differs on 22,852 pixels
and native AA8 on 33,948, all inside the separately fixed diagnostic boundary band.
No exact general native pixel parity is claimed. The 16 small generated CFF cases
contain 1,024 pixels: 11 match the independent sample model byte for byte; five
have seven differing pixels, all inside 118 conservatively uncertain pixels.
There are zero model differences outside that band and zero native differences
outside the separate native band. Corpus raster rows do not claim independent
per-sample certification merely because their unused model counters are zero.

`reference/page-outlines-reference-validation-b0.json` passes six positive checks,
47 rejected mutations and 17 native wrapper checks. Its final supplement verifies
all 43 accepted corpus canvas dimensions independently from native page bounds;
width/height mutations are rejected, so a wrong Rust canvas cannot shrink the
reference target. The positive checks include both fresh asset reproductions.
Unexpected native signals, partial output or malformed envelopes remain failures
when Rust also refuses. Existing accepted embedded-outline and raster pages are
preserved. Combined with the 652-test current/MSRV and formatting/Clippy/wasm gates,
this evidence accepts the stated R20 subset. The whole engine rewrite and app
replacement remain unfinished.
