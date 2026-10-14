# Independent review: typed page placement

Reviewed `src/page_geometry.rs` against the R8b contract, tagged geometry foundation, page inventory and native oracle. The independent review found a precision inconsistency that root fixed before acceptance: a page with crop/media `[1e16, 0, 1e16+2, 10]` and UserUnit 0.1 produced nominal width 0.2 while the published affine transform mapped the right edge to 0.25. Finite, nonempty checks alone did not reject the mismatch.

The corrected constructor compares transformed crop corners to nominal bounds using a tolerance scaled by the expected page dimensions, not the distant source origin. This deliberately refuses severe cancellation, preserving the stated strict geometry subset. Four-rotation regression coverage confirms the example now refuses. Overflow, underflow and finite checks precede publication of immutable typed values; PageInfo remains unchanged.

Four tests in `tests/review_page_geometry.rs` pass. In addition to the cancellation regression, 140 dyadic box/scale/rotation combinations check independent width/height expectations, transformed crop bounds, center placement, 144-DPI composition and inverse round trips. One-representable-step containment violations on all four media edges refuse. Positive sub-unit boxes retain their dimensions under every quarter turn, without the native minimum-unit-rectangle recovery. Targeted Clippy and rustfmt pass.

No further blocking issue found. General f64 geometry remains approximate; exact arithmetic, rendering, clipping, pixel allocation and a guarantee that all invertible extreme transforms can be inverted are outside this API. Native fixture/generated comparison is owned and recorded separately by root.
