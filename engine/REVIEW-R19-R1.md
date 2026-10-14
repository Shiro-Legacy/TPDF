# R19 R1 independent raster review

Reviewed 2026-10-03. Scope: `scan_raster.rs`, `page_raster.rs`, their typed PP1/G1
connection and [PAGE-RASTER-DESIGN.md](PAGE-RASTER-DESIGN.md). Reviewer-owned Rust
changes are `tests/review_scan_raster.rs` and `tests/review_page_raster.rs` only.
The preceding source audit is [R19-RASTER-AUDIT.md](R19-RASTER-AUDIT.md); its final
owned-scanner recommendation replaced the initial tiny-skia proposal. Existing
R0 and its optional dependency are unchanged. No complete-rewrite claim follows
from this raster subset; see [REWRITE-ROADMAP.md](REWRITE-ROADMAP.md).

## Scanner arithmetic, semantics and resources

The new scanner accepts an explicitly closed edge set on the1/256 output-pixel
lattice. Sample centers are32+64k. Coordinates are admitted within absolute2^29
and sample dimensions within2^23. All endpoint checks include horizontal and
off-canvas edges. Intersection arithmetic uses checked signed i64 numerators and
positive denominators; comparisons use checked i128 cross products. With the
admitted coordinates, intermediate numerator expressions fit below2^61 and cross
products below2^91. This avoids dependency fixed-point slope saturation.

Rows use half-open Y intervals. Rational crossings are heap-sorted anew per row,
and exactly equal X values are grouped before winding/parity changes. Off-canvas
crossings are retained until final visible spans are clamped, so left-of-canvas
edges establish the correct interior state. Rational ceiling determines half-open
sample spans, including exact sample ties and negative intersections. All contours
of one paint share winding evaluation; separate paints remain ordered.

Review requested an explicit failure for an unbalanced final row winding/parity;
the owner added it and the reviewer regression passes. This detects a useful
class of malformed open edge sets, not all possible missing contour closure.
The public closed-contour precondition remains documented. The page adapter
constructs closure itself.

Edges, row visits, individual heap operations/comparisons, spans, covered samples
and scratch allocation requests have cumulative budgets. Allocation charges a
full requested crossing arena when it grows, and retained scratch is reused.
Failures preserve admitted work. The scanner may have emitted earlier spans when
a later error occurs; its callback contract is streaming, with covered sample
work charged before each callback. The page renderer uses private buffers and
never returns those partial pixels on failure.

## Page composition and geometry

The page renderer consumes original PP1 mixed events in order. Paths/clips apply
their saved user-to-page CTM followed by the target map; G1 outlines already in
page space receive only the target map. Each glyph's event/code/context indices
are rechecked. Tr0 fills, Tr3 skips raster geometry, and valid empty glyphs remain
empty; original PP1/G1 records are retained by the input, not reinterpreted as
Unicode glyph selection.

Uniform positive scale and translation targets are required. The initial page
rectangle is classified at the original sample coordinates, without geometry
snapping; a larger canvas never substitutes for the page clip. The owner's final
relative-extent precision guard rejects nonempty but distorted rectangles under
distant translations. Finite fractional page bounds remain valid.

Device-space quadratic/cubic flattening uses a bounded iterative stack and a
finite-chord-segment distance, preserving collinear overshoot and nontrivial loops
with coincident endpoints. Depth/node/edge exhaustion refuses; it never accepts a
coarse final chord. Admitted coordinate bounds keep the flatness arithmetic finite.
The1/64-pixel tolerance and later nearest1/256 vertex snapping are explicit
approximation policies. Half ties round away from zero. Neither policy claims
exact original-curve pixel coverage at every sample.

All clip masks are binary on the same4x grid. AND with the immutable ancestor
mask makes repeated clips idempotent and restores parents without coverage loss.
Opaque Gray/RGB paints compose on the sample surface before one final integer box
resolve; averaging paints separately is not equivalent. Alpha is255. Full-page
semantic refusal catches unsupported strokes/painted CMYK before geometry or pixel
allocation. The public result owns its RGBA buffer independently of session/input.

Raster and scanner budgets are separate. Raster byte reservations may be charged
before semantic refusal; they are not a claim that those buffers were allocated.
Actual allocation requests, geometry work, mask construction/intersection and
final resolve are metered separately. Failed plans or scans retain their admitted
usage and cannot invalidate a previous image. Logical accounting is not a process
RSS limit or universal recoverable-OOM guarantee.

## Independent evidence

The reviewer target passes **25 tests:9 scanner and16 page raster tests** with
`cargo test --test review_page_raster --test review_scan_raster --target-dir
/private/tmp/tandem-r19-review-target` (the final page-only rerun includes the
added half-lattice tie regression). Tiny PDFs and the rectangular embedded font
were built independently of the owner's builders.

The scanner model is per-sample signed ray casting with i128 cross products: it
uses no sorting, rational intersection list or callback-span implementation.
There are240 deterministic polygons, their reversed orientations and both fill
rules, giving960 generated comparisons in addition to targeted cases. Coverage
includes extreme admitted coordinates, nearly horizontal crossings, off-canvas
winding, tied/coincident edges, holes, self-crossings, zero-length/horizontal
edges, malformed coordinates/dimensions, every one-less cumulative limit and
pre-callback sample admission.

Independent page expectations cover repeated fractional clips, nested restore,
empty clips, global opaque paint order before resolve, off-canvas coverage,
original fractional page bounds and translated canvases, holes/implicit closure,
collinear cubic overshoot, genuine loops and depth exhaustion, unsupported suffix
atomicity, exact allocation/byte/work edges and repeated failures. A generated
rectangular GID verifies glyph/path event order and target scaling exactly once.
Tr3 under a CMYK text state and empty nonzero glyphs produce white with no flat
edges. The quantization tie test distinguishes32.5/256 from32/256 at an exact
sample boundary.

The independent Python reference was authored separately in
`/private/tmp/page_raster_review_model.py` and handed to the native-oracle owner
for a versioned tool/report copy. Line scenes use exact common-denominator integer
per-sample ray casting after the declared snapping policy. Curves use rational
Bernstein hull subdivision and monotonic crossing certification, not the
production finite-chord flattener. A conservative1/32-pixel curve-boundary band,
plus straight-segment snapping uncertainty in mixed paths, marks uncertified
pixels explicitly. Certified pixels must match; uncertain pixels are not silently
counted as exact comparisons. Initial model self-checks cover opaque rectangles,
repeated fractional clipping and an analytic quadratic crossing.

## Acceptance boundary

The source and targeted independent test review has no remaining identified
blocker. The complete build and native/model gates are recorded below. Independent
acceptance is for the declared fixed-sample filled-page subset, with the
approximation and compatibility limitations retained.

Native comparison must distinguish controlled exact sample cases from general
AA, boundary approximation and hinting observations. Do not relabel boundary
uncertainty as exact pixel parity, and do not reuse a production flattener as the
independent curve model. Remaining unsupported fonts/content, strokes, CMYK,
transparency, images/Forms and application integration stay explicit.

## Integration gates reported by the owner

The integration owner reports **630 all-feature tests pass** on Rust1.98.1 and
minimum Rust1.85.1. Formatting, strict all-target/all-feature Clippy and the
all-feature minimum-toolchain wasm library check pass. Final CLI SHA256 is
`06fccd5d08917c87d74508af3d8250f44ba7c32c99fc8ee16a928d1c156945cb`.
The final copied independent model is `tools/page_raster_review_model.py`, with
uncertainty propagation constrained by the parent mask. Final native/reference evidence was inspected directly as summarized below.


## Final native/model acceptance

Inspected `reference/page-raster-differential-r1.json`,
`reference/page-raster-reference-validation-r1.json` and
`PAGE-RASTER-NATIVE-NOTES.md`. The integration owner independently rechecked all23
report-bound evidence hashes. The model's frozen SHA256 is
`8ef0da904c591c97c237de6949862b7f13f17ca62c05981c39ea5e091382d6d4`.

The generated population is **442 real PDFs /362,577 pixels**:

- **322 controlled cases** match both independent pixels and native4x AA0
  exactly, including36/72/144 DPI, fractional coverage and repeated clips.
- **108 general line cases** match the independent exact sample model. Native
  slanted-edge scan conversion remains a separately reported diagnostic.
- **12 curve cases** include ten byte-matching images and two arch cases with
  four differing pixels total, all inside the declared approximation band.
  Across this population76 pixels are marked uncertain; matching bytes do not
  turn an uncertain pixel into a certified one. There are zero differences
  outside the model band.
- The analytical one-pixel quadratic covers exactly6of16samples and returns
  RGB159 with no uncertainty. Tiny-loop cases are also fully certified. These
  are substantive independent curve checks rather than only flat-path tests.

Every one of572 readable corpus pages was attempted: the two prior eligible
pages still render, and570 explicitly refuse, including all500stress pages. The
eligible-page baseline is enforced. This preserves the narrower glyph-rendering
subset and does not retract PP1's broader trace support. One eligible page is
blank; the other actual embedded-text page is a native boundary diagnostic,
not an exact native-text pixel match.

Native differences are not concealed by changing thresholds. On the real text
page, native4x differs at430 boundary pixels and AA8 at644, with zero differences
outside the declared native diagnostic band. Ninety-five generated native4x and
83native AA8 cases retain their predeclared investigation flags. The notes explain
sample-on-edge conventions, native curve rasterization and repeated-AA-clip
coverage behavior. For example, the exact analytical quadratic differs from
native4x by32 channel units, and repeated fractional native AA clips reduce
coverage while the declared binary-sample intersection is idempotent. The
independent sample specification passes; broad native AA/text-pixel parity is
not an accepted claim.

Reference validation passes **14 model identities,4 positive comparator
baselines,23 rejected mutations,25 native wrapper checks and2 Rust atomic
unsupported-suffix refusals**. Mutations cover missing/extra/reordered pixels,
paint/clip/winding errors, certified curve pixels, broken PPM output, unexpected
exits/crashes and silently lost eligible pages.

With these explicit populations and limits, the independent review has no
remaining blocker for accepting R19 R1. The complete engine rewrite remains
unfinished, with unsupported capabilities tracked in the roadmap.
