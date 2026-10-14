# Independent geometry review

Reviewed finite point/rectangle/affine invariants, composition order, inversion, raster conversion, and coordinate-space typing. One blocking numerical issue was found and fixed; no remaining blocker was identified in the documented strict f64 subset.

## Closed finding

The exactly rank-one matrix `[1,3,15,45,0,0]` initially produced a nonzero determinant after normalization because of floating-point rounding. `inverse()` then returned huge finite coefficients for an actually singular map. The implementation now rejects determinants within a conservative rounding-error bound relative to the two determinant products. This also deliberately refuses some invertible ill-conditioned inputs; it is not an algebraic proof of invertibility over all f64 inputs.

The rectangle transformation API was clarified as `transform_bounds`: it returns the axis-aligned bounds of transformed corners. A zero-width line under shear may therefore gain nonzero bounding-box area. This policy is documented and independently tested; a future clipping interpreter must preserve empty-region semantics separately.

## Evidence

Six independent tests pass:

- Noncommuting, cross-space composition against manually calculated integer coefficients and coordinates, plus an independently calculated inverse.
- The singular regression and all 4,235 rank-one integer matrix combinations in a small signed grid.
- Finite inverse behavior across sampled powers of two spanning exponents -1023 through 1023.
- Four-corner rectangle extrema with reflection and shear.
- Exact signed32-bit raster boundaries, refusal beyond them, and zero-area raster preservation.
- Explicit line-to-bounding-box behavior under shear.

Both existing compile-fail documentation tests pass. Additional direct rustc checks accepted a custom-tag A→B→C pipeline and rejected wrong-space points, rectangles, composition, and cross-space identity construction. These checks used marker types with no required runtime data or extra traits. Targeted Clippy with warnings denied passes.

Private coordinate/coefficient fields and by-value getters preserve the finite-constructor invariants. Intersection bounds remain subsets of finite input rectangles. Raster extents use widened signed subtraction before unsigned conversion, avoiding i32 overflow and saturation. This module does not change page geometry, execute content, clip, or rasterize.
