# R23a: exact affine image sampling foundation

2026-10-06. The user's continuation authorizes this bounded implementation under the R23 proposal. R22's accepted source/evidence checkpoint is preserved. This stage supplies numeric preparation and texel selection only; it does not accept image dictionaries, decode images, change page execution or claim completed R23 rendering. No dependency changes.

## Concrete interface

`image_sampling::ImageSamplingSession` owns cumulative preparation-attempt, sample-attempt and fixed arithmetic-work reservations. `prepare(Affine<UserSpace, DeviceSpace>, width, height)` returns an immutable owned `ImagePlacement`. `sample(&ImagePlacement, sample_x, sample_y)` returns `Option<ImageTexel>` for the exact device sample center `((2*x+1)/8, (2*y+1)/8)`. Texels expose stored top-down `row` and `column`. A placement may be reused across sessions: each sample attempt pays the same complete bounded work. No heap allocation, borrowed document state, unmetered sampling entrypoint, or unsafe code.

The numeric hard image dimensions are 1..=65536; the eventual loader's smaller caps remain separate. Affine coefficients and exact transformed unit-square corners must lie in [-2^21, 2^21], and each coefficient must be exactly representable on the 2^-80 lattice. Values outside the domain, including nonzero subnormals, explicitly refuse; no rounding to the lattice. Nonnegative sample indices must be below 2^23. `Affine` already guarantees finite coefficients. Singular matrices are detected exactly.

## Arithmetic proof and edge policy

Scale all coefficients and sample coordinates by 2^80. Individual values have absolute magnitude at most 2^101; translated sample differences at most 2^102. Corner sums use i128 and are checked against the coordinate cap. Products for D fit at most 2^202 each and their difference at most 2^203. U/V products fit at most 2^203 each and their differences at most 2^204. Signed 256-bit magnitude operations therefore have ample headroom; every potentially overflowing limb add/multiply still checks. Multiplication by dimensions <=2^16 gives at most 2^220 before coverage and 2^219 after it. No floating-point arithmetic is used for determinant, footprint classification or quotient decisions.

A private four-u64 magnitude type implements checked addition/subtraction, exact u128-by-u128 multiplication and checked u32 scaling; signed values use canonical sign/magnitude. Division is a bounded binary search for the largest q with D*q <= N, at most 17 iterations per dimension. No general big-integer library or unbounded division loop.

Normalize D's sign and use exact 0<=U,V<=D footprint predicates. Equality at each of the four edges is admitted only by that positively oriented edge's device top/left rule: dy<0, or dy==0 and dx>0. All incident constraints apply at vertices. This uses transformed device edge directions, not source half-open intervals. Interior source selection is floor(width*U/D), floor(height*(D-V)/D). Only an admitted exact endpoint yielding the dimension clamps to dimension-1. Outside samples never clamp into the image. Paths' 1/256 quantization is not applied.

## Work and failures

Precharge one attempt, then 1024 arithmetic units for preparation or 16384 for a sample (raised during independent R23b review), before validation. Each unit upper-bounds one fixed primitive limb/bit/sign operation, with loop dispatch included; the allowance deliberately covers both quotient searches even on an early outside/refusal. No unused-work refunds. A counter refusal preserves previously admitted counters; failed work admission consumes its already admitted attempt. The preparation path has six bounded float-bit conversions, twelve bounded corner sums, two 2x2 limb products and a signed subtraction, and fixed edge predicates. A sample has four 2x2 limb products, two signed subtractions, at most four signed comparisons, and two <=17-step searches of four-limb scale/compare operations. These stay below the declared fixed allowances. Counters are u64 and checked; defaults are 100000 preparations, 64000000 samples and 1100000000000 work units. Logical budgets are not elapsed-time or RSS promises.

## Verification before integration

Rust tests cover identity/top-down selection, exact row/column boundaries, transformed outer edges and vertices, reflection/rotation/shear, tiny nonzero determinants, singularity, admission thresholds, dimensions/sample indices, owned reuse, exact/one-less counters and failure accumulation. Private arithmetic tests cover all-limb carries/borrows, overflow, sign cancellation and extreme products.

A separate Python Fraction reference uses polygon ray crossing at a rigorously tiny positive perturbation (x+2^-512,y+2^-1024) and Gaussian elimination, independently of the production U/V predicates. In the admitted lattice domain nonzero boundary distances dominate the perturbation, and the leading perturbation selects the same top/left ownership at exact edges. Original and one-step-neighbor cases, non-square images, very small determinants and randomized dyadic transforms are compared through a test-only example binary. Mutated outputs must fail its comparator. This is an independent numeric oracle, not native PDF image-decoder/render evidence.

Full current/MSRV tests, formatting, strict all-target/all-feature Clippy and minimum-version wasm compilation remain required. R23 loader, Program events, ordered compositor and native/corpus acceptance remain subsequent work. The arithmetic source must be reviewed against this proof before page integration.

## R23b review amendment: work allowance and candidate windows

Independent review raised the sample allowance to 16384, without changing selection predicates. Conservative source-level sub-bounds are: four signed 2x2 products at 256 units each (1024); signed subtraction/normalization, coverage and row inversion at 1024; two quotient searches with at most 17 iterations, each allowed 256 units for a four-limb scale/compare and loop bookkeeping (8704); initial quotient scaling at 512; sample conversion, guards and charge/bookkeeping at 1024. Total 12288 leaves 4096 headroom. These are accounting units for the fixed source arithmetic, not machine instruction, time or allocator guarantees. Preparation retains 1024.

`ImagePlacement::sample_bounds` computes minima/maxima of the four exact i128 lattice corners. Dividing by 2^78 with Euclidean floor gives a conservative lower sample index; floor+1 gives the exclusive upper index. Clamp in i128 before converting to u32. This deliberately admits at most one extra edge sample; no potentially covered center can be omitted. The raster target limits each dimension to 2^21 pixels, hence candidate indices to less than 2^23. A transformed placement is validated even if its candidate window or clip is empty.

The R23a frozen report and checkpoint retain their original work allowance; R23b repeats the numeric differential for the current source.
