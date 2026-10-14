# R21 independent stroke design review

Design review only; no production implementation or acceptance is implied.
The owned geometry approach in R21-STROKE-PROPOSAL.md is implementable for solid,
positive-width lines and an explicitly admitted regular-cubic subset. Straight
lines alone cannot unlock the stress fixture: every page also paints a cubic.
Keep dashes, hairlines, cusps, reversals and meaningful zero-length segments as
separate capability decisions. There is no need to implement them to admit the
fixture's two solid strokes.

## Accepted constructive approximation

For a cubic leaf use chord-aligned orthonormal axes. Let e be the forward chord
direction, C its centerline, radius r>0, and n its left unit normal. Require every
derivative control vector Di to have projection dot(Di,e)>=m>0. Convexity of the
quadratic derivative hull proves this for all parameters, so the chord projection
s(t) is strictly increasing from 0 to 1. If

    a = max_i |cross(Di,e)| / dot(Di,e),

then every tangent is in a forward cone of half-angle arctan(a). Its unit normal
is within a of the chord normal: 2*sin(alpha/2)<=tan(alpha) for alpha<pi/2.
Interpolated endpoint normals satisfy the same bound by convexity.

For F(t,u)=C(t)+u*r*n(t), -1<=u<=1, compare

    G(s,u)=chord(s)+u*r*((1-s)*n(0)+s*n(1)).

Use the same chord projection s=s(t). The perpendicular centerline deviation is
bounded by the control hull's maximum distance to the finite chord segment.
Thus, after a linear device map of norm at most L,

    |A F(t,u) - A G(s(t),u)| <= epsilon_center + 2*L*r*a.

The factor 2 is essential. The bound is valid for all filled-strip parameters,
not just the two boundary offsets. Monotonicity/surjectivity of s supplies a
pairing in both directions. If the cross-section vertices form a strictly convex
quad in the intended order, the bilinear image is exactly its filled polygon.
Shared endpoint cross sections avoid synthetic caps or PDF joins. The resulting
filled-set Hausdorff bound is sound even before a no-fold restriction.

## Additional restriction needed for the proposed membership claim

Filled-set Hausdorff distance by itself does not prove equal sample membership
away from a boundary. Require a strict no-fold margin for the first capability:

    r * kappa_bound <= 1 - eta, with a fixed documented eta>0.

A conservative bound is kappa_bound=Amax/m^2, where Amax bounds the norms of both
second-derivative control vectors. This follows from
|cross(C',C'')|/|C'|^3 <= |C''|/|C'|^2. A tighter certified cross-product bound can
be introduced later; subdivision or explicit refusal is sufficient initially.
Use interval/outward guards for the actual floating-point admission, not an
unguarded comparison to a rounded value of 1.

This condition gives an elementary leaf injectivity proof. In chord axes,

    x_t = C'_x*(1-u*r*kappa) > 0,
    det(F_t,F_u) = r*|C'|*(1-u*r*kappa) > 0.

For any fixed x, values of u for which the strip intersects that vertical line
form an interval: its two inequalities are determined by endpoint cross sections,
whose x coordinates are affine in u. The corresponding t is unique because x_t
is positive. Along that slice, implicit differentiation gives

    d y / d u at fixed x = det(F_t,F_u) / x_t > 0.

Therefore each slice and the whole strip are injective; the boundary is a simple
closed curve with the expected orientation. This argument uses the forward cone
and the no-fold margin together, rather than assuming a general offset is simple.

The true and quad boundaries have matching parametrizations within the uniform
error bound, with identical endpoint sections. Their straight-line boundary
homotopy cannot cross any sample farther than that bound from the true boundary.
Winding/inside membership is then equal for that sample. This supplies the needed
boundary-band certificate. Curvature admission is a sufficient conservative
restriction for this claim; it is not claimed necessary for every valid stroke.

Multiple leaves and original segment bodies may overlap or self-intersect globally.
Treat them as a union of individually admitted positively oriented components.
At an uncertain component, union uncertainty can be cleared only when another
component is definitely inside; shared section edges can remain conservatively
uncertain. Do not derive global topology from net contour area. Original joins
and caps require their own exact or bounded component geometry.

## Numerical and geometry requirements before implementation acceptance

- Measure offsets, widths, joins and cap extensions in PDF user space. Apply the
  full affine map afterward. A single scalar device width is incorrect under
  shear/unequal scale. Reflections must reverse all components consistently.
- Use an upper bound for L (Frobenius norm is sufficient). Keep separate center,
  normal, round-sector and final quantization error allowances, plus numerical
  evaluation error. Existing 1/256 snapping adds coordinate error in both axes;
  do not silently spend the same tolerance twice.
- Compute and retain identical split points and endpoint normals between sibling
  leaves. Check strict quad convexity with a numerical guard; uncertain signs,
  collapsed sides, overflow/underflow or lost representable progress subdivide
  or refuse. f64 finiteness alone is not a convexity proof.
- A curve may be regular yet fail the sufficient derivative-control-hull test.
  Subdivide under explicit node/depth limits; never interpret a failed bound as
  permission to use its chord. Wide radii can violate no-fold even on a smooth
  centerline and should refuse in this first subset.
- Line rectangles plus cap/join components must have consistent positive winding.
  Closed paths join their seam without caps. Open stroke paths are not implicitly
  closed as filled paths are. Original corners and artificial leaf boundaries
  must remain distinct in the representation.
- Test miter distance against the specified miter limit using the actual outer
  intersection. Ambiguous near-parallel or reversed geometry needs a precise
  classification or refusal, not an epsilon that switches cap/join semantics.
- For round arcs, use admitted angular pieces and the transformed sagitta bound,
  retaining exact shared endpoints. Full disks are safe only where their union
  with the incident bodies has been proved equivalent to the intended round join.
- P0 already refuses mixed-CTM path construction, so a saved paint CTM applies to
  the whole admitted path. Preserve that prerequisite; do not accidentally expand
  user-space geometry using only the final matrix for a newly broadened path scope.

## API and resource disposition

The proposed owned StrokeSession and StrokeOutline are appropriate. The map is
used for tolerance/admission during expansion, while the returned commands remain
UserSpace and get the map exactly once in raster planning. Document this clearly.
A public API that accepts both StrokeStyle and Option<&DashPattern> must validate
consistency with style.dash(); it must not allow omission of a meaningful pattern
to silently turn a dashed stroke into solid. A narrower explicit solid-style
input is also sufficient for the first implementation.

Admit attempts, source commands and every semantic/validation traversal; count
curve nodes before splitting, accepted leaves, normalization/arc work, joins/caps,
components, points/commands and all requested capacities. Intersect per-call caps
with session remaining budgets. Fixed stack depth is a structural bound, not a
replacement for cumulative node/work limits. Temporary stroke and raster/scanner
allocations are separate charged partitions. Refusals retain spent reservations.
A complete stroke plan must precede pixel allocation; a failed stroke suffix must
publish no partial image from preceding fills. Session/output ownership and all
exact/one-less cumulative limits require independent tests.

## Evidence and remaining gates

Use an independent per-sample model of line rectangles, analytical caps/joins and
true cubic normal strips. A production flattening replay is not independent.
For cubics, isolate inverse chord parameters and boundary positions with interval
reasoning; dense samples alone are diagnostic. Mark unresolved boundary samples
uncertain and compare every certified sample exactly. The uncertainty band must
include all approximation/quantization/numerical allowances, and its size must be
reported so an excessively loose certificate cannot disguise a failure.

Include the fixture cubic, noncommuting/reflected/sheared maps, wide and tiny
radii, near-threshold curvature, zero endpoint derivative, loops, cusp/reversal,
collinear overshoot, narrow pieces, joins at curve endpoints, closed seams and
exact budget boundaries. Native stroke comparison must separate AA and native
minimum-width clamping from the geometric positive-width contract. Thin strokes
whose native width was clamped are not evidence for exact geometric parity.

Remaining blockers to implementation acceptance are concrete: choose numeric
admission margins/error allocations; define exact degenerate/style refusal policy;
implement and independently test interval membership certification; prove all
allocation/work reservations and atomic raster integration; complete native/model
and full-corpus gates. These are bounded engineering tasks. No general-purpose
cusp stroker or new unmetered dependency is required for the proposed narrow scope.
