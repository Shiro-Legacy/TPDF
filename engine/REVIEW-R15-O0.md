# Independent O0 review

Accepted for the bounded standalone unhinted simple-outline scope. No unresolved production finding remains. This does not establish PDF character-to-glyph selection, composite expansion, hinting, metrics, font rendering, or page rasterization.

## Evidence

`tools/outline_model.py` independently encodes six-table sfnt programs and expands each cyclic contour by first inserting all implicit on-curve midpoints, then rotating and walking that expanded contour. This differs structurally from the production streaming contour walker. All command coordinates are exact doubled integers; there is no floating tolerance.

`tools/differential-outlines.py` and `reference/outline-differential-o0.json` record the final stable binaries and deterministic seed 1378833:

- 220 generated fonts, 1,415 glyphs, 26,450 raw points and 24,733 commands agree exactly with the independent model and pinned bundled FreeType oracle.
- All five extracted fixture programs, including 1,885 slots and 72 simple glyphs, agree exactly. Both original PDF hashes and extracted program hashes are checked against the inventory.
- Generated cases cover short/long loca, aligned/unaligned table starts, repeated flags beyond 256 points, independent x/y delta planes, signed coordinate extremes, arbitrary opaque instruction bytes, empty slots, zero contours, one-point contours, all-off contours, odd half-unit midpoint sums, multiple contours, and varied unitsPerEm.
- hmtx side bearings vary independently of header xMin; compressed metrics with numberOfHMetrics=1 exercise trailing LSB records. The oracle correction is confirmed against pinned `thirdparty/freetype/src/truetype/ttgload.c`: pp1.x derives from header xMin minus left bearing, then the loaded outline is translated by negative pp1.x. The correction never uses an observed outline bounding box.
- Glyph header/control bounds, retained table checksum values/ranges, and cumulative outline attempts/bytes/contours/points/commands/work counters match independently computed expectations.
- 109 malformed or unsupported input/query cases refuse without stdout or panic. Recovery of malformed fonts is deliberately excluded from native parity.

Final Rust binary SHA256: `31e2c61ee9966b9b73d244fb3d57afacf072f8abffae0057ec296da81380064c`. Native SHA256: `bc02685515eea4e15f5935b861f4efd470b9085ff73b1b590b8080d2753f5668`. Both remained unchanged during the gate.

## Source review and regression tests

Seven independent tests in `tests/review_truetype_outline.rs` pass, together with targeted strict Clippy. They verify unaligned safe reads, immutable input, exact off-curve decomposition, glyph-interval isolation, continued bounded session use after malformed input, cumulative failures, foreign GlyphId range rechecking, all truncation prefixes of a small valid font, font-index limits, zero-contour instruction opacity, and exact outline work/allocation/point/command boundaries.

Review raised one resource-policy issue: the initial command-vector reservation used the full safe upper bound even with a zero remaining command allowance. The owner changed reservation to the smaller of the safe bound and remaining allowance. Emission still charges the exact command count before push. A regression verifies zero command allowance allocates no command arena and exactly three commands succeed for a one-off-point contour.

Byte ranges are validated before slicing; glyph parsing is limited to its loca interval; raw coordinates remain within i16; midpoint arithmetic uses i32; table sorting is in place; glyph payload allocations are explicit and charged before reservation. Session usage retains work already consumed on errors. The approved unaligned-offset adjustment preserves directory/table bounds and nonoverlap checks.

## Remaining scope limits

The reader accepts only the approved strict simple TrueType subset. Checksums are retained but not validated. It rejects composite glyphs and variation tables, does not execute instructions, and does not inspect a font cmap or select glyph IDs. The native oracle requires hhea/hmtx for FreeType's loading convention, while the Rust outline reader does not need those metrics tables. Output agreement on the measured corpus is evidence for this subset, not general font compatibility.
