# R23a sampling source audit and numeric verification

2026-10-06. This audit was performed in the implementation session; no separate human or subagent source review is claimed. The Python model is an independent algorithm, not an independent reviewer. Separate arithmetic review remains a gate before page integration under R23-IMAGE-PROPOSAL.md.

Implemented the R23a exact affine image-sampling foundation in `src/image_sampling.rs`. It uses bounded signed 256-bit arithmetic for determinant/coverage and exact nearest-neighbor row/column selection under rotation, reflection and shear. Device edge ownership, top-down source rows, singularity/numeric refusal and cumulative work/attempt budgets are explicit. No heap allocation, unsafe code or new dependency.

Verification: **740 tests pass on Rust 1.98.1 and minimum 1.85.1**, including 16 new arithmetic/sampling tests. Formatting, strict all-target/all-feature Clippy and minimum-version wasm32 library compilation pass. The separate Python Fraction polygon/ray-crossing and Gaussian-elimination model matches **10,084 cases**: 1,358 selected samples, 7,118 outside samples and 1,608 explicit refusals. Five altered/omitted-output mutations are rejected. The final numeric comparison report is `reference/image-sampling-differential-r23a.json`.

This is a tested numeric component, **not complete image support or R23 acceptance**. Image dictionary/filter/predictor validation, retained decoded assets, scoped Do dispatch, ordered image events, sample-buffer composition, native image comparisons and the full image corpus gate remain to implement. Page execution/rendering is unchanged. R22 remains the latest accepted integrated rendering milestone.

## Checked source invariants

- The coordinate bound uses an exact representable literal, not a runtime power computation. Exact f64 bit conversion rejects lost low bits and nonzero values finer than 2^-80; signed zero becomes zero. All corner sums fit i128 before the coordinate admission check.
- 2x2 u64 schoolbook multiplication's accumulator is at most u128::MAX; the second row consumes the first row's high carry before assigning the fourth limb. Addition, subtraction and small-factor scaling reject final carry/borrow. Zero has canonical positive sign. Products and signed sums obey the exponent bounds in R23-SAMPLING-DESIGN.md.
- The sign-normalized U/V tests apply all incident edge conditions. Under a reflection, directed edges reverse before testing top/left ownership. Only already-covered exact source endpoints clamp. Top-down row selection computes height*(D-V) before flooring; reversing a previously floored row is never used.
- Quotient search preserves 0<=lo<=floor(N*size/D)<hi<=size when N<D and takes no more than 17 iterations. Size one and size 65536 are covered. A tiny nonzero determinant whose f64 product difference vanishes remains nonzero in the integer path.
- Preparation/sample methods are the only public numeric entrypoints. Attempts are charged before work; admitted failures retain work. Immutable placements expose dimensions only and contain no borrowed state. Reuse across sessions still pays sample work. Exact/one-less budgets and repeated failure paths are tested.
- The reference computes polygon membership with exact ray crossing after a fixed infinitesimal perturbation and uses Gaussian elimination for source coordinates. Its domain proof keeps perturbations below nonzero boundary distances and gives device top/left edge ownership without replaying production integer predicates. It tests original boundaries and adjacent representable translations, rather than comparing two floating inverses.

## Limits and next gate

The sampler admits only final represented affine coefficients and corners within +/-2^21 on the 2^-80 lattice; image dimensions are <=65536 and nonnegative 4x sample indices are <2^23. This is a structural numeric subset, not all finite f64 transforms. It has no decoded samples or image color/Decode metadata, and is not connected to the page compositor. Fixed conservative work reservations are logical counters, not processor-instruction or wall-clock measurements.

Review before integration should retain these exact policies and tests. It must also assess the loader's smaller caps, numeric preflight before pixel allocation, per-page/session remaining-limit intersection, bounded candidate windows, clip-before-paint accounting and late-error atomicity. No new native rendering/corpus comparison was run for this unused component; accepted R22 page source is byte-identical.
