# R23 image components: independent source review

2026-10-06. Separate subagent review under the user's explicit authorization. This review covers `image_sampling.rs`, `image_resources.rs`, their existing tests, and the R23 proposal/component contracts. The reviewer did not edit production source. It does not accept the whole R23 renderer or complete Rust rewrite.

## Findings and fixes

Two loader accounting issues were reported to the implementing agent and fixed:

1. `Document::object` and `stream_data` search the document's BTreeMap. The loader charged cache/cycle traversal but originally omitted this document-index work. Every lookup now precharges a conservative reservation based on document object count. Independent regression tests verify budget refusal before a missing-object lookup and increasing work for a larger index.
2. Wrapping document lookup errors in `Box<DocumentError>` allocated on a failure path without reserving allocation. These errors are now held inline. The lookup APIs can report a fixed missing-object descriptor without heap allocation; a zero-allocation-cap missing-object test retains zero allocation usage.

The sampler's fixed work bound originally relied on a compact prose estimate. The reviewer requested an explicit primitive-operation bound or a larger allowance to make its conservative interpretation robust. The implementation now reserves 16,384 work units per sample and adjusts the default work allowance accordingly. This comfortably covers both bounded quotient searches and fixed setup/classification; the review has no remaining work-bound objection. Preparation remains 1,024 units. These are logical primitive-operation reservations, not CPU instruction or elapsed-time measurements.

## Arithmetic and sampling disposition

The reviewed exact arithmetic and sampling algorithms are sound within their explicit admitted domain:

- `lattice` converts represented f64 bits exactly, rejecting nonzero subnormals, values finer than 2^-80, and magnitudes above 2^21. Its shift bound follows from the magnitude check; no significant high bit is lost. Signed zero is canonicalized. Coordinate/corner sums fit i128 before admission.
- Four-limb multiplication has four u64-by-u64 products. Each accumulator is bounded by `(2^64-1)^2 + 2*(2^64-1) = 2^128-1`. The second partial-product row consumes the first row's high carry before writing its final limb. Addition, subtraction and small scaling check their final carry/borrow. Signed subtraction has correct sign selection and canonical zero.
- Scaled coefficients are bounded by 2^101, translated samples by 2^102. Determinant and numerator sums stay below 2^204 in magnitude, and multiplying by the maximum dimension stays below 2^220. There is ample space in the 256-bit magnitude. Sign normalization does not narrow these values.
- Edge directions follow the positive-area polygon orientation, including reflections. Coverage checks every incident boundary condition. Source cells are selected only after footprint membership; only exact admitted source endpoints clamp. Stored rows use `floor(height*(D-V)/D)`, preserving downwards source ownership at interior ties.
- Quotient binary search starts with `0 <= quotient < dimension` after separately handling the exact outer endpoint. Each iteration halves the interval; dimensions through 65536 need at most 16 iterations. No unbounded division or allocation occurs.
- Placements own their numeric state and are immutable. Every public sample call charges its attempt and fixed work even if outside the image or numerically refused. Reusing a placement cannot bypass those charges.

The independent sampling tests use integer dot products for the orthogonal inverse, rather than duplicating determinant formulas. They check 1,152 sample/dimension combinations across all eight square symmetries, including dimensions 65535 and 65536, exact outer/interior boundaries, top-down rows and endpoint clamping. Additional tests check ±2^-80 neighbors of a diagonal and unique ownership of shared sheared edges under both determinant signs.

## Loader disposition

- Document identity is bound to the loader; cache identity is the actual indirect stream ObjectId. Samples and filter metadata are owned by the resulting Arc asset and survive loader/document/input destruction.
- Required dimensions/components and optional rendering semantics are resolved through the same bounded resolver. Stream-valued metadata refuses. Cycles, dangling references and the 64-hop limit cannot turn into absence. Null is treated as absence only after successful resolution.
- Width/height/product/row/retained-size bounds are checked before decode. Compact Gray and RGB storage and per-channel inversion agree with checked pixel indexing. Exact decoded length is required before cache publication.
- Filter descriptors are prepared once with metered metadata and bounded allocation. Predictor shape is validated against image layout for nonidentity predictors, and the shared decoder validates identity predictor metadata too.
- Cumulative attempts, metadata, references, lookup work, admitted decode sizes, row/pixel/storage reservations and allocations persist across failures and page changes. Cache reuse charges actual invocation/lookup work without inventing decode work. Per-page zero decode/allocation caps can reuse an already owned asset.
- The decoder's output allocations grow geometrically up to an admitted cap, with at most one cap-truncated last request. Three times each stage cap bounds full requested output capacities, including speculative Flate growth and predictor output; filters/asset/cache storage is reserved separately. This is a logical requested-capacity ledger, not an allocator/RSS measurement. Failed decoder calls retain their full reservations; only successful reported decode output releases unused work allowance.

Independent loader tests cover exact 64 versus 65 reference hops, cycles, null/stream resolution for sixteen optional fields, zero-budget cache reuse and owned lifetime, failed length validation never entering the cache, cross-document isolation, and both reported accounting regressions.

## Exact candidate bounds and compositor disposition

The reviewer additionally inspected `ImagePlacement::sample_bounds` and the image branches in `page_raster.rs`.

The candidate helper computes all four corners on the admitted i128 lattice. Minima use Euclidean floor after division by 2^78; maxima use that floor plus one. These bounds conservatively enclose every 4x sample center, including exact footprint ties, and are clamped to canvas dimensions before u32 conversion. Negative coordinates and cancellation never rely on rounded f64 corner sums. Two additional independent tests cover shears/reflections/crops across the small integer matrix grid and coordinate/lattice extremes.

Raster preflight prepares every image placement before pixel buffers are allocated. The canvas hard limit of 2^21 pixels per side ensures all loop sample indices are below the sampler's 2^23 limit. Checked pixel/sample-byte products established during preflight bound the subsequent row and sample-buffer indexing. Private planning allocations can precede later numeric errors, but no pixels are published on any error.

Images enter the existing ordered paint list and overwrite the same 4x sample buffer as paths/text/strokes. Clip masks and original page bounds are consulted before paint, and one box resolve occurs after composition. The candidate window is charged before traversal; every candidate pays sampling work even under an empty clip. Preparation/sample/work usage merges back into the session after success or failure; each page intersects its caps with remaining session capacity. The final interface uses preparations and candidate samples to bound image event/paint costs conservatively, rather than separate image-event and painted-sample counters proposed earlier.

Four independent compositor tests verify overlapping fractional image/path paints before a single resolve (the expected mixed color differs from blending separately resolved paints), exact/one-below sample caps through an empty clip, failed-page usage followed by success at the exact session remainder, and singular-image refusal before pixel allocation. All pass. No compositor source blocker was found in this review.

## Verification and remaining gates

All **15 new independent tests pass in the actual crate**: six loader, five sampler/bounds, and four compositor tests. The earlier full-crate component run also passed 13 existing loader and 12 existing sampler tests before the sample-work allowance change. Follow-up runs verified the loader tests and identified stale work constants in existing sampler tests for the implementing agent to correct; the final full-suite rerun belongs to the root integration gate. The four private sampler arithmetic unit tests are not included in the 15-test count.

The review is complete for the listed source/components; no unresolved production correctness finding remains. Whole-stage acceptance still requires the final current/MSRV/fmt/Clippy/wasm suites, native decoding/render evidence, independent full-page affine model, full corpus classification and preserved no-image outputs, and final integration/provenance/failure-atomicity checks. The root agent is running those gates. Passing these component tests alone neither accepts the full image stage nor completes the Rust rewrite.
