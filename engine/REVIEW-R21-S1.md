# R21 independent stroke implementation review

## Disposition and scope

The solid positive-width line and admitted regular-cubic implementation has no
remaining source-review blocker and is accepted for the R21 contract subset.
Fourteen independent stroke tests and sixteen migrated page-raster tests pass;
full integration and the final native/model/corpus gates below also pass. The
earlier proof and numeric requirements remain in R21-STROKE-REVIEW.md and the
R21 contract.

This subset includes geometrically admitted butt/round/square caps and
miter/round/bevel joins. It explicitly refuses hairlines, nonempty dashes,
zero-length meaningful segments, unresolved tangents/joins, reversals, excessive
curvature/folds and exhausted numeric/subdivision bounds. It is not a general
stroke engine, nor completion of the PDF rewrite.

## Source review and resolved findings

Width expansion occurs in UserSpace. The complete saved affine map is used for
error admission and later applied once in raster planning; shear and reflection
do not replace the width with a scalar device approximation. Generated convex
components have consistent winding and are scanned together as a nonzero union.
Closed seams join without caps. Move-only open subpaths produce empty output;
explicit zero-length operations and move-only closes refuse. Redundant closes
are no-ops, and drawing after Close without a new Move refuses explicitly rather
than losing the suffix or inventing a different cap convention.

Intervals enclose exact dyadic f64 inputs. Arithmetic steps outward, including
subnormals; unresolved signs or denominators cannot authorize geometry. Exact
zero shortcuts require algebraic proof. A reviewed failure initially stepped a
proven nonnegative squared-norm sum below zero, causing sqrt to reject ordinary
strokes. This was fixed by preserving the proven nonnegative lower bound while
keeping sqrt strict. Root also corrected midpoint containment at subnormal scale.
Shared section endpoints and normals survive subdivision; cap/join arcs reuse
section vertices. Carried ideal control uncertainty is not reset to rounded
midpoints.

The cubic gate uses positive derivative-hull projection, the same user-chord
parameter for center error, the factor-2 normal error and a radius-curvature bound
at most 1/2. Strict convexity and the matched-boundary proof support the stated
sample uncertainty band. Center, normal, arc and numerical allowances remain
separate, at most 1/64 device pixel before the existing 1/256 lattice quantization.
Round sectors use bounded normalized-vector bisection and sagitta bounds rather
than platform trigonometric assumptions. Limits and fixed stacks bound all
subdivision. Uncertain miter thresholds and nearly collinear turns refuse.

The exact-cross regression uses integer directions whose true cross product is
one although direct binary64 products subtract to zero. The implementation refuses
the unresolved join instead of declaring it a proved collinear no-op.

## Resource and atomicity review

Each direct expansion intersects call limits with cumulative remaining limits and
retains admitted work on every failure. Checked accumulation is atomic on integer
overflow. The output arena charges full requested capacities before fallible
allocation. Output point/command quotas apply to emitted storage; scratch candidate
normalization/transformation is bounded by separately precharged work and fixed
node/stack bounds. The distinction is explicit: output_points is not a count of
all temporary arithmetic or every transform evaluation.

Raster integration has one temporary StrokeSession per page, bounded by request
stroke caps, the session's per-page policy and cumulative session remaining caps.
Its usage is merged after preparation and execution errors. Whole-page style and
color checks precede geometry/pixels; complete expansion precedes image publication.
Stroke storage, raster plans and scanner buffers remain separately charged.
Failure never replaces a previously returned owned outline or image. No new
runtime dependency or unsafe code was introduced for this capability.

## Independent Rust tests

`tests/review_stroke.rs` contains 14 tests. Output membership is computed by an
independent ray crossing of the returned components, with analytic projection,
distance, miter and closed-ring expectations. It does not call the production
scanner to decide its expected result. Coverage includes all caps, both turn
orientations for each join, miter-to-bevel selection, self-crossing union, closed
seams, a hole, noncommuting affine admission, ownership after source/session drop,
explicit dash binding, empty-dash normalization, repeated failures, move/close
states, near-collinear cancellation and fixed coordinate/depth guards.

The real stress cubic is admitted, and a straight cubic agrees with an analytic
line at independently selected samples. The combined line/cubic/round-cap case
checks exact and one-less admission for all 15 cumulative usage fields, plus a
second call against the exact session quota. `tests/review_page_raster.rs` now
uses explicit hairline and nonempty-dash suffixes for the unsupported-stroke
regression and checks the typed Stroke error. Prior fill tests retain their
meaning.

## Independent reference model

`tools/stroke_review_model.py` uses exact Fraction control points and Bernstein
subdivision, with rational outward square-root bounds formed by integer isqrt.
It encloses the true cubic normal-offset boundaries, never replaying production
cross-section quads. Exact dyadic parameter keys preserve shared endpoint intervals
and their chosen polygon vertices across refinement levels. Both side boundaries
and endpoint sections have bounded enclosure/homotopy checks; points within
2^-100 of a non-axis cap remain uncertain when 2^-80 normalization intervals
cannot decide them.

`StrokeModel` composes line bodies, original joins, endpoint caps, closed seams
and true cubic strips. It returns certified inside/outside or None, with an
optional device-space boundary margin. Line/cap/join certificates use strict
interval face/distance tests, so exact sample ties are not compared as if closed
sets matched the rasterizer's half-open convention. The retained raw line_member
predicate is only an exact closed-set analytic helper, not a raster certificate.

Union certification is deliberately conservative: one definitely interior
component proves inside, and every component definitely exterior proves outside.
None can include internal component seams as well as the true union boundary.
Report this uncertainty rather than claiming an exact union-boundary distance.
The model's global forward/no-fold admission is a sufficient subset of production's
local leaf gate; RefLimit is a model failure, never a silent outside/refusal result.
Depth, node, query, rational bit-size and work caps bound the harness.

The saved selfcheck report records 20 analytic identities, 275 straight-cubic
sample equivalences, four refusal/budget checks, five endpoint/margin checks,
1,683 certified line/cap equivalences, four closed-seam identities and three mixed
line/cubic/cap identities. The stress cubic with scale, shear and reflection
certifies 7,679 of 7,680 samples; the one exact endpoint boundary remains uncertain.
A separate positive-margin check terminates without unbounded refinement. Final
native/model raster gate figures and full toolchain results are recorded below.

## Full integration gate update

Root reports all 684 tests passing on the current compiler and MSRV 1.85.1,
plus strict all-target/all-feature Clippy, formatting and the MSRV wasm32 library
build. Final test-only lint edits use three review-test struct initializers and
a lexical lifetime scope in the owner test; the six owner and fourteen review
tests pass on both compilers after those edits. Production code is unchanged.
The frozen CLI used for native/corpus acceptance has SHA256
21e1a4ac418c8610c67e923c7579f55accd661c65675c3a843ba067cdf4f0198.
Source and Rust tests are frozen; the final reference acceptance is recorded below.

## Native interior color-policy review

The stress corpus exposed a separately understood color conversion distinction:
Rust's fixed nearest-byte rule produces (230,102,51) for the opaque .9/.4/.2 fill,
while pinned native resolve_color converts by truncation to (229,102,51). This is
not a stroke approximation error and does not justify changing production color
conversion or widening geometric/native bands.

The proposed narrow reference classification is accepted only with independently
verified source structure, exact fixture/source hashes, native event/color/order
checks, absence of hidden clip/state changes, the full pixel strictly inside the
known rectangle, and certified exclusion of the later true cubic under the
separately derived native-policy margin. Every channel must match that exact pair.
Wrong source color, rectangle, order, clip, region or channel must fail mutation
checks. Original outside-band totals must remain visible, with classified color
policy differences reported separately from unclassified failures. This is not a
blanket one-level tolerance or a claim of general native pixel parity. Final report
counts and implementation of these gates are included in the acceptance evidence.

The implemented `tools/stroke_source_color.py` and amendment
`reference/stroke-color-policy-s1.json` were subsequently inspected. The pinned
`draw-device.c` source digest matches, and resolve_color's float-to-byte assignment
confirms truncation. The implementation enforces the exact pair on every certified
mask pixel, including pixels inside the ordinary band and cases where both outputs
agree on an incorrect value. Its structural source/native checks and independent
later-stroke exclusion match the approved conditions. The saved validation suite
contains seven positive checks, 53 rejected mutations, 33 native wrapper checks and
three Rust atomic suffix checks, including the required color/source/state/region
and report-hash corruption cases. No blocker was found; the final full rerun
passes with the acceptance figures below.


## Final R21 acceptance evidence

The frozen differential and validation reports both have status `passed`. This
review independently recomputed all 18 differential and 17 validation evidence
hash entries against their current files and executables; all 35 match. Report
SHA256 values are:

- `reference/stroke-raster-differential-s1.json`:
  `6366c80701db511b5faa94a301f5b3ef0276f429f24fcb9f84830c01412400d5`.
- `reference/stroke-raster-reference-validation-s1.json`:
  `62e82b3a7fa188fa2b4ee8a138fe74d1d06b77f49f05c58261e68771fe963083`.

All 110 generated pages pass: 36 match the exact baseline and 74 satisfy the
independent certified channel intervals. Across 7,040 pixels the model reports
700 uncertain pixels / 1,456 uncertain samples, 510 baseline pixel differences
within the certificates, and zero channel-interval violations. Its 4,694
certified nonwhite channel constraints make the small-stroke checks nonvacuous.
The 24 controlled cases also match the native raster exactly. General native
4x and AA8 comparisons have zero differences outside the previously specified
geometric bands; this does not turn the 74 interval cases into byte-equality
claims.

At 18 DPI, all 572 corpus pages are attempted: 543 render and pass the native
diagnostics, including all 500 stress pages; 29 are explicitly refused. All 43
previously accepted pages remain accepted at 72 DPI, and stress pages 1, 250 and
500 also pass at 72 DPI. Corpus comparison is native diagnostic evidence, not an
independent exact per-sample proof; zero/unused corpus model counters must not be
read as such a proof.

For each of the native 4x and AA8 modes, the original outside-band counts remain
visible: 78,500 pixels at 18 DPI plus 13,800 in the three 72-DPI stress pages,
92,300 in total. Every one is classified by the independently certified exact
source-color pair described above. The stricter check verifies that exact pair
on 120,733 pixels per native mode, including certified pixels that were not
outside the ordinary band. There are zero source-pair violations and zero
unexplained outside-band differences. The 43 preserved 72-DPI pages require no
color exception and have zero outside-band differences. No geometric tolerance
or boundary band was widened to accommodate the color conversion distinction.

The reference validation passes seven positive checks, 53 rejected mutations,
33 native wrapper checks and three Rust atomic unsupported-suffix checks. These
support the measured subset, error handling and comparison rules; they do not
prove general native antialiasing, color, hinting or pixel parity. Hairlines,
dashes, nonadmitted degeneracies/curves and the other explicit R21 refusals remain
outside this acceptance. The review, independent model and tests are frozen for
the R21 checkpoint. This acceptance does not complete the engine rewrite.
