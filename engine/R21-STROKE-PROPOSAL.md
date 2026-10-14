# R21 proposal: bounded stroke geometry

Design only. No production edit or dependency adoption is authorized by this
proposal. R20 remains frozen during its native/reference and full acceptance gates.

## Actual stress fixture and recommended scope

The checked-in fixtures/make-large.py emits two strokes on every one of the 500
pages, with an RGB rectangle fill between them:

```
0.2 0.4 0.9 RG 1.5 w 72 120 m 540 120 l S
0.9 0.4 0.2 rg 200 160 160 36 re f
0.1 0.6 0.3 RG 1 w 72 60 m 200 200 400 200 540 60 c S
```

Both strokes are solid, positive width, with default butt caps, miter joins and
miter limit10. The single cubic has endpoint derivative directions (128,140) and
(140,-140), with middle control edge (200,0); all x derivatives are positive. It
is regular, has no endpoint zero tangent, and is a useful first bounded curved
stroke case. A straight-only stroker removes the first refusal at operation23
but **does not unlock any full stress-fixture page**: the later cubic still
refuses atomically. Do not report 500-page rendering support from line-only tests.

Recommend an owned safe Rust geometry stage, with two explicit acceptance layers:

1. Positive-width solid Move/Line/Close paths; all three caps and joins, checked
   miter clipping to bevel, including nonsimilarity affine CTMs. This establishes
   reusable geometry, event/clip integration and cumulative resource accounting.
2. Positive-width solid regular cubics under a proved device-error/tangent test,
   enough to admit the stress cubic. Distinguish artificial subdivision vertices
   from original path corners. Refuse cusps, degeneracies or exhausted bounds
   until their exact policy is established. This is required for the claimed
   stress-corpus unlock, not an optional future enhancement to that claim.

Strictly positive line-dash arrays with nonnegative phases can follow within the
same stage if independent evidence is ready. Dashed curves need bounded arc-length
error and should be a separate capability gate. Empty dash arrays already mean
solid. Initial explicit refusals may include hairline width0, zero dash entries,
meaningful isolated zero-length segments, unsupported cusp/reversal geometry and
curved dashes. P0 acceptance of an operator does not imply raster support for
every parameter combination. These refusals must be whole-page and documented.

## Why the existing dependency is insufficient as-is

The pinned optional tiny-skia0.12.0 already includes tiny-skia-path0.12.0. Its
stroke-only API is safe Rust and can feed the owned R19 rasterizer, so the prior
backend raster slope-clamping objection does not apply to that proposed use.
Its BSD-style license notices are already included in the R0 license inventory.
Nevertheless safe Rust alone does not establish the required resource contract.

Source inspection of tiny-skia-path0.12.0 found:

- PathStroker::stroke/Path::stroke return Option<Path>, without cumulative work,
  allocation-request, emitted-point or caller cancellation hooks. Internal
  PathBuilder growth uses infallible vectors.
- stroker.rs initially reserves approximately1x input for its inner builder and
  3x for outer output (the source labels these estimates, not worst-case bounds).
  Curve recursion limits are [15,78,33,33], and depth alone permits impractically
  large binary work/output envelopes. Several independently growing builders
  retain temporary geometry. A public small global output/work bound is absent.
- dash.rs uses an estimated per-call MAX_DASH_COUNT of1,000,000 with path-length
  computations and unmetered builders/contour arrays. It explicitly omits special
  handling for single zero-length lines. This is neither a cumulative session
  budget nor proof that every allocation request was admitted before execution.
- Geometry uses f32. Additional admission would be needed for loss of endpoints,
  shallow turns, wide strokes, huge coordinates, cusp handling and miter tests.

A source-audited fork with fallible growth and explicit counters is possible but
would be a substantial new maintained component and retain its native geometry
semantics to verify. A useful preflight restriction without such a fork would
already need much of the owned proof below. Do not make tiny-skia-path mandatory
merely because its API is safe or because R0 has its license. No other dependency
has been audited in this proposal; no new dependency is recommended at this time.

## Geometry representation and affine correctness

Expand offsets in PDF user space using radius=width/2. Apply the saved paint CTM
and raster target only after expansion. Scaling a device-space normal by a single
CTM expansion factor is wrong for anisotropic scale and shear. A negative CTM
determinant reverses all generated contours together; it must not change their
union or width. Singular or precision-losing maps can initially refuse explicitly
unless a separate degenerate-image proof is supplied.

Prefer an owned StrokeOutline containing typed UserSpace Move/Line/Close commands.
The expansion API receives UserSpace-to-DeviceSpace solely to choose tolerances
and validate output. The existing R19 user_path applies this map exactly once
when building scan edges. This costs a bounded temporary command arena but keeps
space typing and raster integration simple. An internal callback builder can
later eliminate the intermediate buffer without changing public atomicity.

Emit each line body as a closed convex rectangle. Caps and joins are additional
closed convex components; normalize each component to the same user-space winding.
Scan all components of one stroke together with nonzero winding. Overlap then
represents a union without accidental cancellation. A single closed offset ring
is harder to handle correctly at self-intersections and reversals. Interior holes
of a closed rectangular stroke naturally remain uncovered by the component union.
Never orient a possibly self-intersecting contour using its net signed area.

Butt caps add no endpoint extension; square caps extend by radius in the true
endpoint tangent direction. Round caps/joins use user-space circular sectors,
whose transformed ellipse has a device-space approximation bound. A full disk
can replace a round join only where its union with incident body rectangles is
proved equivalent; do not silently use it for bevel/miter joins. Closed subpaths
join at the seam and have no caps. Explicit close-to-first must not emit an extra
cap or duplicate meaningful turn.

For nonparallel ordinary turns, compute the outer offset intersection and compare
its distance/radius to the miter limit; beyond the limit use a bevel. Use checked
finite f64 arithmetic and bounds before division, normalization, addition or
allocation. Near-collinear, opposite-direction and zero-length cases need exact
classifications or an explicit refusal; a guessed epsilon must not choose a
visually different join. Source/control/expanded coordinates all remain subject
to R19's output magnitude and quantization admission.

## Curved strokes: required proof before implementation acceptance

Flattening only the centerline within1/64 device pixel is insufficient by itself:
a wide stroke magnifies tangent error at offsets and caps. Ordinary path joins
also cannot be applied blindly to synthetic tessellation corners. A cubic needs
bounded positional and directional admission before replacing a leaf by stroke
geometry.

For a de Casteljau leaf with user-space control points p0..p3, use derivative
control vectors D0=3(p1-p0), D1=3(p2-p1), D2=3(p3-p2). Their convex hull contains
every derivative on that interval. Require a finite nonzero chord direction e
and a strictly positive lower projection min(dot(Di,e)); otherwise subdivide or
refuse at the depth/node limit. This proves a regular forward tangent cone for
the leaf. Bound tan(alpha) by max(abs(cross(Di,e))/dot(Di,e)); every derivative
lies in that cone. Do not sample only the endpoint tangents.

The linear map's Frobenius norm is a conservative device expansion bound L.
Allocate separate error allowances for transformed centerline hull distance,
normal/tangent error, round-sector approximation and final R19 quantization.
For example radius*L*tan(alpha) conservatively bounds normal-direction deviation
for the forward cone. Subdivide until this and the finite-segment centerline
control-hull bound are below their allocated tolerances. All products/divisions
must be finite and checked before comparison. If the proof cannot progress due
to cancellation or a zero derivative, refuse; never silently accept the chord.

There are two plausible owned constructions that must be chosen in the contract:

- Consecutive true-tangent cross sections linked by convex strips, with shared
  endpoints and no artificial caps. This needs an explicit no-fold/convexity
  proof for the offset strip. A conservative curvature bound can use the derivative
  lower projection and second-derivative control vectors to bound
  radius*abs(cross(v,a))/abs(v)^3; nonadmitted inner-offset folds refuse.
- A stroked polyline with special smooth connectors at synthetic vertices,
  distinguished from actual PDF joins. Its connector and endpoint errors must
  be bounded by the same tangent envelope; treating them as arbitrary PDF
  miter/bevel corners without that proof is not accepted.

The first is the recommended narrow regular-cubic route because cross-section
continuity is explicit. A constructive bound for review is as follows. Positive
derivative projection makes chord projection s(t) monotone and surjective over
[0,1]. The true normal strip is F(t,u)=C(t)+u*radius*n(t), -1<=u<=1. The bilinear
cross-section strip is G(s,u)=chord(s)+u*radius*((1-s)*n0+s*n1). Both true normals
and interpolated endpoint normals lie within tan(alpha) of the chord normal, so
the difference is bounded by centerline_error+2*radius*L*tan(alpha) in device
space. The factor2 is required for this construction. If the four endpoint
cross-section vertices form a strictly convex quadrilateral, its bilinear image
is exactly the filled polygon. Adjacent leaves share the identical cross section.

This is a candidate filled-strip Hausdorff bound, not yet an accepted proof of all
PDF offset-boundary topology or native pixel membership. Independent review must
settle whether the extra curvature/no-fold restriction is required for that
claim, and specify reliable convexity admission and per-sample certification.
The stress cubic has a generous regularity margin; there is no reason to implement
a general cusp stroker merely to cover that fixture.

For round sectors, a transformed radius bound R=L*radius and angular step theta
satisfy sagitta<=R*theta^2/8. Begin with sectors no larger than a quadrant and
bisect under explicit depth/vertex limits until the error bound passes. Charge
nodes, trig evaluations and vertices before work. Use exact shared endpoint
normals at joins/caps, rather than separately recomputing numerically different
endpoints. Offset-generated user coordinates are finally transformed/quantized
under the unchanged R19 contract.

## Dashes, phases and difficult native policies

P0 already validates nonnegative dash elements, at least one positive element for
a nonempty array, and nonnegative phase. An odd-length array conceptually repeats
twice to preserve on/off parity. Do not allocate a doubled array. Reset phase at
each subpath. For straight segments measure dash lengths in user space, continue
across vertices, and preserve joins when one on-run crosses an original corner.
A dash boundary creates caps; it is not an ordinary join.

Normalize phase by a checked finite cycle reduction, not repeated subtraction
proportional to an attacker-controlled huge phase. Refuse sums/normalization that
lose required phase or parity precision. Every emitted on-run, traversed boundary
and numerical advancement consumes cumulative dash work. Demand representable
forward progress; do not spin while a tiny dash cannot advance a large distance.
A closed subpath whose on-run crosses the seam must merge the initial and final
runs and use a seam join. Keeping two separately capped fragments is wrong.
Bound the pending first-run storage and all split/output segments before growth.

Do not infer width0 or degenerate behavior from ordinary positive-width geometry.
Pinned native draw-device.c raises linewidth to a graphics-AA/minimum-line-width
floor (AA0 gives a one-native-pixel floor before transform expansion), even for
some positive widths. Native draw-path.c distinguishes move-only paths from
explicit zero-length operations and has separate line-dot/zero-length-cap paths;
closing a degenerate round subpath can also emit a dot. Zero dash entries may
create dotted strokes rather than being safe to discard. These cases require
pinned native geometry/pixel fixtures and an explicit target-resolution policy.
Initial refusal is preferable to pretending width0 means an invisible stroke or
arbitrarily replacing it with one base pixel. Native minimum-width clamping must
also be separated from the geometric positive-width raster contract in comparisons.

## API, ownership and admission sketch

```
StrokeSession::new(StrokeSessionLimits)
StrokeSession::usage() -> StrokeUsage
StrokeSession::expand(
    commands: &[PathCommand], style: StrokeStyle, dash: Option<&DashPattern>,
    user_to_device: Affine<UserSpace, DeviceSpace>, limits: StrokeLimits
) -> Result<StrokeOutline, StrokeError>
StrokeOutline::commands() -> &[PathCommand] // only Move/Line/Close
```

Keep cumulative counters for attempts, input commands, original segments,
subdivision nodes, accepted curve leaves, dash boundaries/runs, join/cap attempts,
arc nodes/vertices, emitted components/commands/points, full allocation requests
and logical work. Per-call caps intersect session remaining budgets. Maximum
subdivision depth and coordinate magnitude are structural limits. Immutable PP1
input, stroke temporary/output geometry, existing raster plan buffers and scanner
work remain explicitly separate partitions; geometry copied into the raster plan
is charged there too. Failed reservations remain spent. No recursion depth or
per-call segment cap substitutes for a cumulative session work limit.

Raster preparation performs whole-page supported-style/geometry preflight before
allocating sample/mask/output pixels. Each Stroke event becomes one planned
nonzero paint using its saved stroke color, clip and global event index. Fill
then stroke operations already have ordered P0 events; a failing stroke suffix
must invalidate the whole page, including an otherwise valid fill prefix. No
stroke-specific early image or separate clip stack is introduced.

## Evidence required

- Independent analytic distance-to-segment/polygon models for line bodies, caps,
  bevels and miter thresholds; reflections, shear and unequal scale; acute turns,
  self-crossing polylines, closed rings, seam behavior and clipping/paint order.
- Dense independent cubic distance/tangent checks plus interval/error reasoning
  for the admitted regular subset. Test tiny radius, wide radius, cusps, loops,
  equal endpoints, collinear overshoot, zero endpoint tangents, cancellation and
  just-inside/outside subdivision/coordinate limits. No tolerance widening to hide
  systematic offset or cap errors.
- Native cases for hairlines, zero-length operations, repeated points, exact
  reversals, zero dash entries, odd arrays, huge phases and closed seam dashes,
  documenting unsupported cases until a policy is approved.
- Exact cumulative work/allocation/output edges; repeated failed attempts; private
  output atomicity; fill+stroke suffix refusal; source/session lifetime ownership.
- All 500 stress pages render after both line and regular-cubic support, with
  native/reference geometry and separately labelled AA/minimum-width differences.
  Preserve every accepted R19/R20 page and record remaining refusal reasons.
- Existing current/minimum Rust, wasm, strict Clippy, formatting, CLI atomicity and
  comparator mutation/wrapper gates. Dependency changes, if reconsidered, require
  their own numerical/work/allocation/license record before adoption.

## Concrete API and limits for the initial contract

This appendix supersedes the earlier sketch where names differ. Initial production
scope is **solid positive-width lines and admitted regular cubics only**. Nonempty
dash arrays are an explicit refusal, including positive entries; dashed strokes
are a later milestone. Butt/round/square caps and miter/round/bevel joins are
included only for geometrically admitted cases. Hairlines, zero tangents,
nonadmitted reversals/degeneracies and cusp/offset-fold cases remain refusals.

```rust
pub struct StrokeSession { /* limits, cumulative usage; no external state */ }
impl StrokeSession {
    pub fn new(limits: StrokeLimits) -> Self;
    pub fn usage(&self) -> StrokeUsage;
    pub fn expand(
        &mut self,
        commands: &[crate::path::PathCommand],
        style: crate::path::StrokeStyle,
        dash: Option<&crate::path::DashPattern>,
        user_to_device: crate::geometry::Affine<
            crate::geometry::UserSpace, crate::geometry::DeviceSpace>,
        limits: StrokeLimits,
    ) -> Result<StrokeOutline, StrokeError>;
}
pub struct StrokeOutline { /* privately owned Vec<PathCommand> */ }
impl StrokeOutline {
    pub fn commands(&self) -> &[crate::path::PathCommand];
}
#[derive(Clone, Copy, Debug)]
pub struct StrokeLimits {
    pub max_attempts: usize,
    pub max_input_commands: usize,
    pub max_subpaths: usize,
    pub max_line_segments: usize,
    pub max_curve_nodes: usize,
    pub max_curve_leaves: usize,
    pub max_join_attempts: usize,
    pub max_cap_attempts: usize,
    pub max_arc_nodes: usize,
    pub max_arc_vertices: usize,
    pub max_components: usize,
    pub max_output_commands: usize,
    pub max_output_points: usize,
    pub max_allocated_bytes: usize,
    pub max_work: u64,
    pub max_subdivision_depth: usize,
    pub max_arc_depth: usize,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StrokeUsage {
    // Public counter fields use exactly the same names/types without `max_`,
    // excluding the two structural depth caps.
}
impl StrokeLimits {
    pub fn intersect(self, other: Self) -> Self;
    pub fn remaining(self, used: StrokeUsage) -> Result<Self, StrokeError>;
}
impl StrokeUsage {
    pub fn accumulate(&mut self, other: Self) -> Result<(), StrokeError>;
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrokeError {
    pub command_index: Option<usize>,
    pub kind: StrokeErrorKind,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StrokeErrorKind {
    Unsupported(&'static str),
    Limit(&'static str),
    Arithmetic(&'static str),
    Geometry(crate::geometry::GeometryError),
    Allocation,
    Invariant(&'static str),
}
```

Implementation clarification: output_points and arc_vertices count emitted output;
precharged work/node ledgers bound temporary candidate creation and transforms.
Both DFS stacks have25 fixed slots and are a separate stack partition, not heap
allocation requests.

The output contains only MoveTo/LineTo/ClosePath, grouped as independently closed,
consistently oriented components. It owns its commands and can outlive the source,
session and document. A stroke result is published only after complete success.
No borrowed style/dash data is retained. Command indices identify the original
input command even for generated subdivisions/caps; the raster caller supplies
page event/operation/span provenance around this error.

Proposed defaults, shared by the configurable per-call and session limit type:

| Field | Default | Admission meaning |
| --- | ---: | --- |
| max_attempts | 10,000 | Every expand attempt, including unsupported style |
| max_input_commands | 100,000 | Commands admitted before input traversal |
| max_subpaths | 10,000 | Original subpaths |
| max_line_segments | 100,000 | Original line/close segments inspected |
| max_curve_nodes | 1,000,000 | Every visited subdivision node, including failed leaves |
| max_curve_leaves | 500,000 | Admitted regular-cubic strip leaves |
| max_join_attempts | 200,000 | Original joins and seam joins considered |
| max_cap_attempts | 200,000 | Original open-subpath cap attempts |
| max_arc_nodes | 1,000,000 | Every tested arc subdivision node |
| max_arc_vertices | 500,000 | Arc vertices emitted |
| max_components | 1,000,000 | Closed body/join/cap components |
| max_output_commands | 5,000,000 | Every emitted Move/Line/Close command |
| max_output_points | 4,000,000 | Emitted points admitted before output storage; temporary candidates use work |
| max_allocated_bytes | 268,435,456 | Full vector allocation requests, including failed reservations |
| max_work | 100,000,000 | Precharged logical geometry/inspection work |
| max_subdivision_depth | 24 | Per original cubic, structural hard maximum24 |
| max_arc_depth | 24 | Per round sector, structural hard maximum24 |

These are logical defaults, not an RSS guarantee. The initial implementation
must measure the stress corpus before freezing defaults; changing them cannot
replace a missing work proof. Work accounting documents fixed costs for ordinary
operations and meters every input/node/component loop. Comparisons, normals,
convexity tests, curvature tests and bounded transcendental evaluations are
charged before execution. There are no implicit loops proportional to coordinate
magnitude. The immutable input path remains a separately bounded P0 prerequisite.

Structural numeric admission is fixed independently of user-raised counters:
absolute input and expanded user coordinates/radius at most2^40; absolute mapped
device coordinates at most2^21, matching the R19 quantized scanner envelope.
All intermediates still require finite checks; these magnitude guards alone do
not prove divisions, normalization or convexity. Values outside the envelope
refuse explicitly. Depth caps are clamped to24 even if the caller requests more.
Zero structural/cumulative limits are valid restrictive limits; no silent default
replacement is allowed. A zero radius fails as unsupported hairline, before
normalization or output allocation.

`expand` constructs local effective caps as the intersection of its call limits
and session remaining limits. On success or error it accumulates the local
admitted usage into the session. Attempts are charged before capability checks;
input counts and work are charged before traversal; failed reservations remain
spent. On quota refusal the count remains at its prior admitted value. Returning
an error cannot undo previous successful output or reset counters.

`remaining` uses checked subtraction for every cumulative field and returns an
Arithmetic error for an externally inconsistent usage value; it never wraps or
panics. Depth caps remain unchanged. `intersect` takes the component-wise minimum,
including depths. `accumulate` checks every addition into a temporary value before
assigning, so overflow cannot partially mutate a public ledger. The session's
internal admitted usage is always within its original limits by construction.

### Raster integration without duplicated arithmetic

Root-owned page integration adds `PageRasterLimits.stroke: StrokeLimits` as a
whole-page cap and `PageRasterSessionLimits.stroke: StrokeLimits` as a whole-session
cap. For each render, compute:

```
local_cap = page_cap.intersect(session_cap.remaining(total_stroke_usage)?);
local_session = StrokeSession::new(local_cap);
result = prepare_all_events_with(&mut local_session);
total_stroke_usage.accumulate(local_session.usage())?; // on success AND failure
```

Every stroke event uses that same temporary session, even when it also supplies
per-expand caps. Root retrieves `stroke_usage()` alongside existing raster/scan
usage. Geometry is copied into the raster plan under the existing raster allocation
ledger; temporary stroke storage belongs to the new stroke partition. Failures
before any stroke invocation leave its ledger unchanged; failures during expansion
retain the attempted geometry's counters. The session helper eliminates duplicated
15-counter arithmetic from raster integration. No changes to R19's scanning,
initial page clip, immutable clip graph, sample composition or resolve are required.

### Reviewer-agreed additional cubic restriction

Retain a strict no-fold margin `radius * curvature_bound <= 1/2`, computed from
conservative derivative/second-derivative bounds for every leaf. In chord axes,
for F(t,u)=C(t)+u*radius*n(t), positive derivative projection and this restriction
ensure x_t=C'_x*(1-u*radius*kappa)>0 and
`det(F_t,F_u)=radius*abs(C')*(1-u*radius*kappa)>0`. Endpoint cross-section x-values
are affine in u; fixed-x valid u form an interval, and implicit dy/du is positive.
This supplies leaf injectivity/simple boundary. Pair this with strictly convex
quad admission, shared cross sections and the matched boundary error envelope.
Overlap of different leaves/subpaths is unioned with the same winding. The
filled-set Hausdorff bound alone is not a claim of sample membership; certification
requires samples outside the matched boundary error band and the no-fold argument.
Conservative floating-point admission for these signs/ratios remains an explicit
review item, not something to hide under raster pixel tolerance.

For the initial numerical implementation, prefer a small local outward-rounded
interval helper (safe f64 bit stepping is available without unsafe code or a new
dependency) over scattered heuristic epsilons. Reviewer R21-STROKE-REVIEW.md permits
explicit operation-by-operation conservative error bounds as an alternative, but
requires lower bounds for derivative projections and quad orientation, upper
bounds for tangent ratios/curvature/device error, and normalization error. Finite
checks plus a global epsilon are insufficient. Unresolved intervals trigger
subdivision or a typed refusal; they never authorize the geometry. Empty dash
arrays are solid even if the saved style still carries a dash identifier.

## Final numeric implementation plan and style preflight

The standalone module additionally exposes:

```rust
pub fn validate_style(
    style: crate::path::StrokeStyle,
    dash: Option<&crate::path::DashPattern>,
) -> Result<(), StrokeError>;
```

This performs constant-size checks only: positive finite width and admitted radius,
finite miter limit>=1, finite nonnegative dash phase, enum cap/join validity by type,
and agreement between the style's optional DashId and the supplied pattern's
presence. Missing/mismatched binding is an Invariant error. A present nonempty
array is Unsupported without traversing its elements. A present empty array is
solid. Width0 is Unsupported hairline. Invalid/nonfinite numeric style values are
Arithmetic/Geometry errors, not silently clamped substitutes. The root calls this
under the raster preflight work charge before allocating pixels. Direct expand
charges its own attempt and constant style work before calling the same function;
validate_style has no independent attempt ledger or allocation.

### Outward intervals and error transport

Use a private Copy interval type I{lo:f64,hi:f64} and IPoint{x:I,y:I}. Input PDF
coordinates, width and affine coefficients become exact singleton intervals: the
contract interprets the parsed f64 inputs as their exact dyadic values. Each
addition, subtraction, multiplication, division and square root encloses the real
result by stepping the rounded endpoint outward. Safe to_bits/from_bits stepping
works on the minimum toolchain without unsafe code. Handle signed zero and
subnormals explicitly; retain exact zero where algebra proves it (for example
multiplication by an exact zero). Reject NaN/nonfinite endpoints and division by
an interval containing zero. Square root requires a proved nonnegative interval;
never repair a numerically unresolved negative lower bound by an arbitrary epsilon.

Arithmetic implementation and its operation count are independently unit-tested
against exact dyadic/rational expectations. Square-root enclosure tests use exact
integer squaring to bracket returned endpoints. Underflow must enclose its true
result or refuse; it must not create a fictitious zero tangent. A bounded fixed
number of interval endpoint operations belongs to each charged geometry node;
no dynamic precision or retry loop is hidden in the helper.

Carry interval control points through every de Casteljau midpoint operation from
the original input. Do not reset a child interval to its rounded central estimate.
Store the ideal shared split endpoint and one ideal normalized tangent/cross-section
record once, then give both children the same record and emitted endpoint values.
This avoids cracks and inconsistent artificial joins caused by separately
normalizing the same mathematical seam. Original PDF corners keep distinct
incoming/outgoing records because their selected join style still applies.

Subdivision uses a fixed explicit stack at depth<=24. Interval records, normal
vectors and arithmetic scratch have fixed sizes and no heap allocation. An arc
stack has the same fixed structural depth bound. State their concrete maximum
stack bytes once the layout is implemented. Heap storage is limited to explicitly
metered source bookkeeping, component staging if necessary, and the final owned
command arena. Full requested vector capacities are charged before fallible
reserve, including geometric growth. Refusal returns no partial outline and
retains every admitted request/work counter.

### Four error allowances before quantization

The overall pre-quantization device-space geometry bound is at most1/64 pixel.
Use four conservative fixed allowances of1/256 pixel each:

1. Centerline approximation of an admitted cubic leaf.
2. True-normal versus interpolated cross-section tangent error.
3. Round cap/join sector chord error.
4. Numeric enclosure, normalization and emitted-vertex displacement.

A construction uses only the allowances relevant to it; do not spend an unused
allowance to disguise an unresolved sign or fold. Existing1/256-lattice quantization
adds its separately documented maximum1/512 error per component afterward. Native
AA/hinting/minimum-width differences are still separate from this geometry bound.

Compute an outward upper bound L=sqrt(a*a+b*b+c*c+d*d) for the affine linear part.
This Frobenius bound dominates its operator norm and treats anisotropy/shear
conservatively. Translation is excluded from L but included in every mapped
vertex interval. Refuse overflow, an unresolved singular/precision-lost mapping,
or bounds outside the admitted device envelope. No scalar "average scale" is
used to expand the PDF stroke.

The cubic pairing uses **user-chord projection** s(t). Therefore its center allowance
must bound the same-parameter residual, not independently choose the closest point
on a transformed chord. For interval unit user-chord e, bound
`L * max_i(abs(cross(pi-p0,e)))`. Strictly positive derivative projections make
control projections ordered within the chord, so this perpendicular residual has
the required monotone pairing. The reviewer explicitly confirmed that closest-
device-chord distance alone can understate this bound under shear.

Use lower interval bounds for each dot(Di,e), and upper interval bounds for
abs(cross(Di,e))/dot(Di,e), then test `2*radius*L*tan_alpha <= 1/256`.
Normalize tangents by interval square root/division, retaining the resulting
normal enclosures. Bound curvature using the derivative control hull and linear
second-derivative hull: an upper bound on max(abs(cross(Di,Aj))) divided by the
cube of the minimum positive derivative projection is conservative. Admit only
an upper bound `radius*curvature <= 1/2`. Unresolved signs/denominators/folds cause
subdivision and eventual typed refusal, never a guessed turn direction.

Construct ideal cross-section endpoint boxes from these enclosures. Every convex
quad orientation determinant must have the same proved nonzero sign over the
boxes; this also controls intermediate boundary homotopies, not merely one rounded
quad. Choose finite emitted user coordinates within their boxes. Transform those
chosen coordinates using the same Affine::transform_point operation order that
R19 later uses, and compare them to the outward affine image of the ideal boxes.
The resulting Euclidean upper displacement must fit the numeric1/256 allowance.
This explicitly accounts for normalization, carried subdivision error and distant
translation cancellation, rather than adding a global machine epsilon.

For original line/cap/join constructions use the same interval normalization,
orientation and vertex-error checks. Exact algebraic zero cases may take explicitly
proved special branches. A determinant containing zero without proof of exact
zero is numerically unresolved, not automatically a collinear/no-op join. Precise
refusals are acceptable at this first gate; silently dropping geometry is not.

### Round sectors without platform trigonometric assumptions

Avoid sin/cos/atan2 accuracy assumptions in production. Represent an arc by unit
endpoint-vector enclosures about the user-space center. Seed known cap half-circles
with exact quarter-turn/sign changes of their endpoint unit vectors. For a join,
use the already-proved turn direction and subdivide until a sector is proved no
larger than a quadrant. Exact reversal remains unsupported unless separately
specified; an unresolved zero vector sum cannot be normalized.

The angular midpoint direction is normalize(u+v), using outward intervals. Bisect
until the sector chord bound passes. For unit endpoints and a sector<=pi/2,
`sagitta <= radius * abs(u-v)^2 / 4` is a conservative trig-free upper bound.
Multiply by outward L and require <=1/256 device pixel. This is intentionally
looser than the exact rationalized sagitta but easier to audit. Shared arc endpoints
reuse the existing cap/join endpoint records; every midpoint carries its interval
normalization uncertainty into the numeric allowance. Arc nodes and their work are charged before testing; emitted arc vertices and
commands/points are charged before output storage. Scratch candidate creation
and transforms consume the precharged work rather than the output-point cap. Numerical uncertainty
that stops bisection progress is a refusal at the structural or error limit.
