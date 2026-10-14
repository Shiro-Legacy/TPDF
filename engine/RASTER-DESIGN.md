# First bounded raster milestone: audited backend boundary

This is a source audit and proposal. The source audit preceded implementation; CONTRACT R11 subsequently authorized the optional dependency and R0 adapter. The recommendation is to use a tightly scoped, pinned tiny-skia backend through its safe public API, while retaining `#![forbid(unsafe_code)]` in our engine. Internal unsafe in that dependency is an explicit audited boundary, not an assertion that the full dependency tree is unsafe-free. A small bespoke scanline model is more useful as an independent test oracle than as a rushed general production renderer.

## Candidate and reproducible source inspection

The audited candidate is the published **tiny-skia 0.12.0**, with `default-features = false, features = ["std"]`. The published archive's recorded Git revision is `e48b9027762d33731e60e40eb17eaa4f583b0185`. Published source differs in small dependency details from the repository's moving main branch, so all conclusions here use the downloaded crate archives, not main.

| Archive | SHA256 |
| --- | --- |
| tiny-skia-0.12.0.crate | 47ffee5eaaf5527f630fb0e356b90ebdec84d5d18d937c5e440350f88c5a91ea |
| tiny-skia-path-0.12.0.crate | edca365c3faccca67d06593c5980fa6c57687de727a03131735bb85f01fdeeb9 |

Archives were fetched from the official static.crates.io distribution and checked against crates.io metadata. Sources, a temporary probe manifest/lockfile and an isolated Cargo home reside under `/private/tmp/tandem-raster-audit`. `reference/tiny-skia-audit.json` retains exact package checksums, features, source hashes, compiler version and audit limits. The original engine manifest/lockfile were untouched.

The initial existing Cargo cache could not resolve tiny-skia offline. Authorized network source download succeeded. A separate probe then resolved and compiled the following exact tree offline, using the installed Rust 1.98.1 compiler. The root subsequently verified the same audit probe using isolated Rust 1.85.1, and the engine now declares that exact supported minimum. tiny-skia itself does not declare an MSRV.

| Package | Audited resolved version | Declared license |
| --- | --- | --- |
| tiny-skia / tiny-skia-path | 0.12.0 / 0.12.0 | BSD-3-Clause |
| arrayref | 0.3.9 | BSD-2-Clause |
| arrayvec | 0.7.8 | MIT OR Apache-2.0 |
| bytemuck | 1.25.2 | Zlib OR Apache-2.0 OR MIT |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 |
| log | 0.4.34 | MIT OR Apache-2.0 |
| strict-num | 0.1.1 | MIT |

The exact resolution activates std on tiny-skia/path and `aarch64_simd` on bytemuck because tiny-skia requests that feature unconditionally. tiny-skia's own `simd` feature is disabled. PNG, libm/no-std-float, C/C++ libraries, FFI targets and build scripts are absent from this resolved tree. Feature unification must be checked if another future dependency enables simd or PNG.

The inspected tiny-skia/path LICENSE files identify Google (2011) and Yevhenii Reizner (2020) and BSD redistribution conditions. Permissive dependency terms can coexist with the engine's AGPL distribution; retain the complete chosen dependency notices and upstream per-file notices when distributing. This does not change our crate's license. The [published crate](https://crates.io/crates/tiny-skia/0.12.0) and [versioned API documentation](https://docs.rs/tiny-skia/0.12.0/tiny_skia/) are primary external references; exact audited implementation is pinned by the hashes above.

## What unsafe remains reachable

Disabling SIMD removes tiny-skia's architecture intrinsic branches; it does not make the scalar implementation unsafe-free. Source inspection identifies these relevant categories:

- Unconditional `unsafe impl bytemuck::{Pod,Zeroable}` for packed/premultiplied colors and scalar wide-vector wrappers. Pixel access and raster pipelines call bytemuck's checked public cast/cast_slice APIs, whose implementations use pointer representation casts. The safety obligations concern representation, alignment and valid bit patterns.
- Pipeline slice-to-fixed-array views use arrayref macros, which contain unsafe pointer casts behind length checks. These are present on ordinary scalar pixel paths, not just SIMD branches.
- ArrayVec is used for fixed-capacity pipeline stages and clipped-edge lists. Its initialized/uninitialized element storage and destruction use internal unsafe. Capacity remains a separate potential panic condition even through a safe interface.
- strict-num and tiny-skia-path use unchecked numeric constructors after explicit invariants/bounds, including constant values. These are unsafe contracts rather than direct external/native code. The backend also exposes an unsafe color constructor; our adapter must never call it.
- log contains global logger/concurrency internals; bytemuck's enabled target-type implementations remain in the dependency feature set. Our render path should not depend on a global logging side channel for success/failure.

These categories explain why a safe API boundary can be reasonable without claiming a full memory-safety proof. Standard library internals also contain unsafe, and our crate's forbid lint is local to our crate. A normal Cargo dependency build with a command-line unsafe lint can be capped by Cargo and is not evidence that dependencies contain no unsafe. The evidence report's lexical unsafe-line counts are discovery aids only: they include comments and disabled target branches and are not reachable-site counts.

Acceptance should review the exact retained feature set, verify no unsafe in our adapter, test all input checks and use Miri/sanitizer/fuzz runs where practical. No vulnerability-database audit or guarantee of absence of backend bugs is claimed here.

## Actual backend behavior that shapes the scope

The source is materially more capable than a small new rasterizer: fills and both fill rules, cubic paths, strokes, dash construction, masks, antialiasing and blending already exist. Its APIs also require a careful wrapper:

1. `Pixmap::new` and `Mask::new` use infallible Vec allocation after size validation; Option indicates shape/overflow failure, not recoverable out-of-memory. Prefer buffers allocated by our `try_reserve_exact` plus `Pixmap::from_vec`/`PixmapMut::from_bytes` and `Mask::from_vec`.
2. `fill_path` and `stroke_path` return `()`. Several numerical/path failures warn and skip work. This does not fit our explicit-error contract for arbitrary inputs. A restricted, preflighted domain must establish which skip conditions are benign (empty geometry) and which are prevented. Catching panics or listening to global log output does not turn this into reliable structured error reporting.
3. Nonidentity transforms clone and transform paths internally. Large surfaces trigger tiling above 8,191 pixels per dimension and repeated transformed path work. Pretransform checked points ourselves and pass identity; keep surfaces smaller than the tiler threshold initially.
4. `Mask::intersect_path` allocates a full-sized temporary mask and performs a full pixel pass. Avoid that convenience method. Own mask planning, scratch storage and intersection work explicitly; binary masks can be combined with a bounded bytewise AND pass.
5. PathBuilder uses infallible Vec growth. EdgeBuilder begins with a 64-edge allocation and grows a Vec; scan conversion sorts edges and maintains active edges through backward insertion. Internal allocations are not supplied by a caller allocator. Dimension/edge/input caps bound requests, but do not provide a recoverable process-wide OOM guarantee.
6. Active-edge insertion can take quadratic work per scanline in adversarial arrangements. A budget of just pixels plus segments is insufficient. Conservative work must include approximately height times squared edge count, as well as pixel writes and mask passes.
7. Curves add extrema chopping, clipping and up to 64 subdivision steps in the inspected edge representation. Stroke code has recursive approximation limits up to 78; dash processing has a one-million-dash cap that is far larger than a reasonable initial engine budget. Stroke/dash paths also allocate expanded geometry. These are reasons to gate curves and strokes separately, not to assume finite input alone makes them cheap.
8. The line-edge iterator recursively skips Move/Close when no edge is emitted. Canonicalize nonempty contours before constructing the backend path to bound this call depth; line-only input does not itself eliminate all recursion.
9. Backend pipelines use a fixed-capacity stage array. The first renderer should use only a fixed solid-color/source-over configuration, never arbitrary gradients/pattern shaders that alter stage complexity.

Source references: tiny-skia `src/painter.rs`, `mask.rs`, `pixmap.rs`, `edge_builder.rs`, `edge.rs`, `scan/path.rs`, `scan/path_aa.rs`, `pipeline/mod.rs`, `wide/f32x4_t.rs`; tiny-skia-path `src/path_builder.rs`, `stroker.rs`, `dash.rs`, `floating_point.rs`. Exact hashes for the principal files are in the audit report.

## Proposed R0: opaque integer orthogonal fills and binary clipping

The smallest useful production milestone with defensible exact native coverage is a **fully preflighted path-program renderer with opaque solid Gray/RGB integer orthogonal fills and binary clipping, without antialiasing**. General slanted polygons are deferred because measured tiny-skia and native scanline conventions differ even with integer vertices. This yields real pixels for bounded vector diagrams and explicit failures for the rest. It consumes our immutable P0 program; it does not reparse PDF syntax or invent missing resources.

Initial contract:

- Accept MoveTo, LineTo and ClosePath geometry whose transformed contours are orthogonal, both nonzero and even-odd fill rules, ordered Fill/ClipPush/ClipRestore events and the initial Unbounded/Rectangle/Empty clip. Every nonzero edge, including implicit closure, must be horizontal or vertical. Empty paths remain explicit adapter no-ops; nonempty contours must have at least four distinct corner vertices and nonzero signed area. The most conservative first gate should reject self-intersecting/touching individual contours with a checked pairwise segment test; different individually simple contours may overlap/cancel according to the requested fill rule.
- Reject every Stroke event, cubic command, CMYK paint, transparency, unsupported color policy or numerical precision failure before publishing any pixels. A combined B event program includes a Stroke and must fail as a whole; rendering its fill alone is incorrect.
- Keep opaque white background initially and produce owned row-major RGBA8 with alpha 255. Solid DeviceGray maps to equal RGB values; DeviceRGB components use one specified checked rounding rule. Do not claim ICC-managed DeviceRGB. Quantization expectations must be explicit; use exact black/white and byte-exact colors in initial pixel parity cases.
- Accept an explicit `Affine<PageSpace,DeviceSpace>` from the existing page geometry layer. Compose each event's user-to-page CTM with that transform in f64, transform all points once and check finite values. Require every transformed coordinate to be an exact integer in f64, within the canvas inclusive, before exact conversion to f32. Never silently round a fractional coordinate. Validate the initial rectangle with the same device-grid rules. A later fractional/slanted policy needs separate pixel evidence before broadening this.
- Require coordinates in `[0,width] × [0,height]`; out-of-canvas vertices are Unsupported even when the visible portion would be simple. This deliberate restriction excludes the backend line clipper and its edge multiplication/numerical branches. Extending the viewport domain is a separate source-audit gate.
- Handle an actually empty path and explicitly Empty initial clip in our adapter; never interpret an empty clip as absence of clipping. Reject nonempty zero-area or collapsed contours before backend entry: MuPDF AA0 paints some zero-height rectangles as a one-pixel row while tiny-skia skips them. Preserve implicit fill closure and winding; no bounding-box replacement. Consecutive identical points may be removed only during checked canonicalization; reject a resulting degenerate contour.
- Pass identity transforms and `anti_alias = false` to tiny-skia. Avoid internal tiling, stroke/dash conversion, gradient/image pipelines and `Mask::intersect_path`.

A private checked `RasterPlan` should be built first. It records transformed paths, all referenced clips, resolved opaque colors, checked dimensions/strides and conservative work/memory estimates. Only after **the whole program** passes preflight should allocation/rendering begin. No malformed or unsupported suffix may leave a returned partially drawn image.

Suggested interface:

```rust
prepare_orthogonal_raster(&PathProgram, RasterTarget, RasterLimits)
    -> Result<RasterPlan, RasterError>;
plan.render() -> Result<RasterImage, RasterError>;
// RasterImage has private width/height/stride/pixels and immutable accessors.
```

A one-shot wrapper may combine those steps. Errors retain the P0 event span and distinguish unsupported feature, precision, size, work, memory and allocation failures. Raw RGBA/PPM is adequate for first inspection; do not enable the backend's PNG feature just to save test images. Existing external image tooling can create visual comparison artifacts.

## Proposed dependency feature and source-derived skip boundary

Keep the default/core engine dependency set unchanged. Subject to root authorization and the minimum-compiler build, add only an optional exact backend:

```toml
[features]
raster = ["dep:tiny-skia"]

[dependencies]
tiny-skia = { version = "=0.12.0", optional = true, default-features = false, features = ["std"] }
```

Continue forbidding unsafe in this crate. State explicitly in user/developer documentation that the optional backend and transitive crates use internal unsafe through safe APIs. Lock and audit the exact resolved tree, retain distribution notices, and check feature unification in CI. This is an optional implementation boundary, not a license or safety-policy change for the core crate.

The public void drawing API cannot report arbitrary backend failures. The following source-derived preconditions close the identified skip branches for **this restricted domain**, without relying on logging or panic catching:

| Backend branch | Required preflight or intentional result |
| --- | --- |
| Painter skips nearly-zero width/height (threshold 1/4096) | Nonempty canonical contour has nonzero integer width/height, hence each is at least 1; reject collapsed/zero-area contours. Empty program/path handled by adapter. |
| Painter rejects huge coordinates | Every coordinate lies between 0 and at most 4096; exact finite f32 values. |
| Transform/tiling/subpixmap failure | Pass identity only, dimension at most 4096 below 8191 tiling threshold; use exact validated full surface. |
| Pipeline blitter rejects different mask size | Every cache/scratch mask has exactly the target dimensions and length. |
| Pipeline fast-reject or shader setup failure | Fixed opaque SolidColor + SourceOver only, no Destination/DestinationIn; SolidColor.push_stages unconditionally returns true. Explicit linear/no-transfer color space, avoiding extra transfer stages. |
| Fixed pipeline stage ArrayVec overflow | This fixed solid/mask/source-over configuration has fewer than 10 stages including terminal function, below the private capacity 32; do not expose arbitrary shaders/blends. |
| Conservative bounds or shifted clip construction failure | Positive integer width/height <=4096 and origin 0, shift 0. Conservative rounding of integer coordinates equals those integers. |
| Clipped edge nonfinite failure | Exact bounds contained in canvas means no EdgeClipper path; only finite direct line edges enter. |
| EdgeBuilder returns fewer than two edges | Horizontal edges and perfectly cancelled vertical spans represent empty coverage; for closed orthogonal contours a lone uncancelled vertical edge cannot enclose pixels. Multiple valid contours may cancel completely, which is a valid no-op, not an error. |
| Negative scanline conversion | Integer bounds and the origin-0 canvas make start/stop nonnegative and <=4096. |
| Fixed-point overflow/drift | Horizontal edges are skipped. Vertical edges have slope zero and x in [0,4096], exactly represented as 16.16; y*64 <=262144, row indices <=4096. No cumulative slope drift. |
| Recursive skipping of Move/Close | PathEdgeIter.next recursively skips these commands. Emit only canonical nonempty contours, one Move followed by actual Line edges and one Close, so the skipped run is bounded; never pass arbitrary empty contour runs. |

Pin these findings to `painter.rs`, `scan/path.rs`, `edge_builder.rs`, `edge.rs`, `pipeline/blitter.rs`, `pipeline/mod.rs` and `shaders/mod.rs`. Source changes or relaxed geometry/color policies invalidate this audit and require another review. Empty fill from cancellation, empty binary clip, painting white on white and zero covered pixels are legitimate no-ops; output equality cannot be used as a success test.

Within the narrow domain, no identified numerical or shader skip needs to be inferred from a void return. There is still **no general public success/error signal**, formal backend-correctness proof, or recoverable OOM guarantee. If the project requires positive backend acknowledgement or all-allocation fallibility, unmodified tiny-skia remains a blocker; choose a small audited fork or the bounded bespoke implementation. Do not claim the wrapper makes arbitrary tiny-skia input safe from silent skips.

## Observed coverage differences and acceptance evidence

The isolated probe uses the pinned std-only crate and has `#![forbid(unsafe_code)]` in its adapter. It accepts one arbitrary polygon and emits P6 RGB; it is test-only under `/private/tmp/tandem-raster-audit/probe`. The independent native oracle uses MuPDF 1.28.1 with AA=0 and identity CTM. These examples are exact measurements:

| Geometry | tiny-skia | native MuPDF AA0 |
| --- | --- | --- |
| Rectangle (1,2), width 3, height 2 | x=1..3, y=2..3 | same |
| Rectangle (1.6,2.6), width 3, height 2 | x=2..4, y=3..4 | x=1..3, y=2..3 |
| Triangle (0,0),(5,0),(0,5) | row lengths 5,4,3,2,1 | same |
| Triangle (0,0),(8,0),(0,4) | row lengths 7,5,3,1 | 8,6,4,2 |
| Zero-height rectangle (17,9), width -14 | no coverage | one row x=3..16, y=9 |

The independent reviewer has 200/200 native-versus-model nondegenerate integer compound rectangle matches (204,800 pixels) including both fill rules, clip/restore and RGB. This is evidence for the integer orthogonal gate, not proof for arbitrary polygons. A rational integer-origin sampler also disagreed on many random integer triangles: do not describe MuPDF AA0 as a general floor or pixel-center rule based on a few examples. The two rasterizers have different edge conventions.

## Defensible budgets and cache behavior

Proposed starting ceilings, subject to measured adversarial validation:

| Resource | Initial cap |
| --- | ---: |
| Canvas dimension | 4,096 each |
| Total pixels | 4,194,304 |
| Output bytes | 16 MiB RGBA |
| Commands per transformed polygon path | 1,024 |
| Total transformed commands | 65,536 |
| Paint/clip events | 4,096 |
| Total live cached mask bytes | 32 MiB |
| Clip ancestry depth | 128 |
| Conservative scan/pixel work estimate | 100 million units |
| Planned peak storage | 64 MiB plus separately documented backend overhead |

All products/sums must be checked before allocation. Charge work for every draw and every recomputed/cached clip-mask build, including repeated references. Pixel cost includes initialization, scratch clearing, intersections and output writes. Repeated painting of the same path shares transformed geometry only; it still consumes scan/pixel work.

For the proposed integer in-canvas orthogonal domain, conservative rounded bounds equal the exact integer path bounds, so the backend skips line clipping. Each explicit/implicit source edge creates at most one edge; horizontal edges disappear, and vertical combination only removes or shrinks edges. Use the number of canonical line edges including implicit closes as `edge_bound`. Precharge `(height + 2) * (edge_bound + 2)^2`, plus an explicit bounded pairwise-contour validation term and full pixel/mask passes. Reject before calling the backend when this bound exceeds the work budget. Do not present the units as wall-clock time or exact internal instruction count; benchmarks/fuzz timeout evidence calibrate the chosen cap.

Clip ancestry uses our flat immutable IDs. Memoize masks by clip node and exact device transform, with a byte cap and explicit accounting. Build a node by rasterizing its polygon into scratch, then intersecting with its parent mask. For first implementation, keeping all required masks and refusing a plan that exceeds the cache budget is simpler and deterministic. A later liveness-aware eviction/recompute policy must charge recomputation and must not recurse on unbounded ancestry. Never retain a full mask for every clip node without a checked total.

Our buffers should use fallible allocation, but unmodified backend edge/path/pipeline allocations can still abort on genuine process OOM. The adapter reserves a conservative 1 MiB planning allowance for one backend call's path, edge storage and sorting scratch. Safe instrumentation in copied sources measures requested path/edge capacities (reference/raster-allocation-audit-r0.json); the observed maximum is well below that allowance. Pipeline ArrayVec storage is fixed and gradient Vecs remain empty in the solid shader configuration. This is a source/capacity bound, not measurement of allocator metadata or a portable promise of exact process RSS. If a hard fallible-allocation guarantee is mandatory, stock tiny-skia's current APIs cannot supply it: use a narrowly maintained fork with fallible hooks or choose the bespoke limited backend. Do not disguise this remaining limitation with an Option return type.

## Comparison with a bespoke limited fill rasterizer

A small safe scanline renderer can implement line edges, half-open Y intervals, even-odd/nonzero winding and per-scanline sorting with checked bounded storage. It can directly meter every edge/scanline operation and return allocation/work errors. This gives stronger ownership/resource transparency and an excellent independent model.

The cost is taking responsibility for pixel-center conventions, ties, shared vertices, coincident edges, self-intersection, implicit closure, fixed-point overflow, clipping, degenerate contours, antialiasing and later curve/stroke expansion. An initial implementation that only fills rectangles would be narrower than the proposed polygon backend and should not be described as a foundation already equivalent to MuPDF. Adding a general antialiased stroker is substantial work and duplicates mature backend functionality.

Recommendation: accept an explicit pure-Rust dependency boundary if root approves it, begin with the integer orthogonal/binary-clip domain above, and use a bespoke **test-only** winding/coverage model for independent exact expectations. If the audit's remaining memory/error limitations cannot be bounded to the project's standard, implement that same limited polygon model as production without claiming broader support. Either choice retains safe ownership/types in our code; only the latter can avoid new dependency-internal unsafe beyond Rust/std and the engine's existing dependencies.

## Native pixel acceptance and future gates

Generate minimal path-only PDFs with a known page transform and no fonts/resources/annotations. Compare:

1. Exact independent integer orthogonal winding/span expectations, without claiming they define MuPDF's general edge conventions.
2. tiny-skia output under the declared non-AA policy.
3. Pinned native MuPDF rendered with graphics antialiasing disabled, identical canvas, opaque white background and matching RGB output/channel order.

First exact-parity fixtures should use integer-aligned rectangles, simple orthogonal contours, holes, reflected winding, both fill rules, overlapping paints and clip/restore order. Keep diagonals, self-intersecting individual contours, fractional vertices and zero-area shapes as explicit-refusal tests in R0; shared boundaries and coincident edges across valid contours still need acceptance tests. Native versus independent disagreements at edge conventions must be understood, documented or explicitly excluded; do not quietly add a fuzzy pixel tolerance to hide a systematic fill-rule/clip bug.

For future antialiasing, exact pixel parity across rasterizers is generally not a reasonable universal claim. Record coverage-edge differences separately from fully covered interiors, with predetermined numerical/image metrics and visual inspection. Keep semantic event parity, interior colors, clip membership and no-repair status as separate checks. Include adversarial input tests for every budget, huge transforms, precision collapse, many mutually crossing edges, repeated mask reuse and unsupported trailing events.

Acceptance also needs a build with the declared Rust minimum, checksum/notice retention for every chosen dependency, a no-default-features feature-tree check, source-audit reproducibility and existing engine gates. The first render milestone makes pixels for its declared subset; it does not establish complete PDF pages with text, images, transparency, color management, curves or strokes. Those require independently bounded follow-up stages.
