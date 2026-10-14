# R19 filled-page raster audit and proposal

Design review only, 2026-10-03. No production implementation is authorized by this
file. This extends the pinned dependency review in [RASTER-DESIGN.md](RASTER-DESIGN.md),
not its R0 exact orthogonal-pixel claim. It proposes real page pixels for another
explicit subset; [REWRITE-ROADMAP.md](REWRITE-ROADMAP.md) retains the completion gaps.

## Final recommendation after the numeric audit

Use a **new owned safe-Rust rational scanline supersampler** for R19, preserving
R0's existing tiny-skia adapter unchanged. The backend proposal below is retained
as the audit trail explaining this choice, not the recommended implementation.
The discovered silent slope saturation and clipping/error boundary make the small
owned line scanner easier to bound and permit more off-canvas content safely.
The fixed4x sampling, flattening, binary clipping, event order and pixel acceptance
requirements below still apply. The2047 edge cap was specific to tiny-skia; an
owned implementation should use explicit checked dimension/memory/work limits
rather than inherit that backend limitation.

After bounded f64 flattening, snap to the declared1/256 output-pixel lattice.
Require absolute integer coordinates<=2^29. Sample centers are q=32+64k.
For each active nonhorizontal edge, normalize dy>0, retain its original winding
sign, and form the exact rational intersection
`(x0*dy + (sample_y-y0)*dx) / dy`. Coordinate differences are at most2^30;
intermediate products and their sum fit below2^61 in signed i64. The active-edge
intersection numerator itself is bounded by2^59 because it is a convex weighted
coordinate. Comparing two intersections through signed i128 cross products is
comfortably bounded below2^91. Use checked arithmetic nonetheless and assert
these admission conditions in adversarial tests.

Use half-open Y activation. Sort intersections anew for each row because edges
can cross. Group exactly equal rational X values before updating winding/parity,
then fill sample centers in half-open [left,right) spans with exact comparisons.
Do not discard intersections left of the canvas: they establish winding at the
first visible sample. Clamp only final row/span write ranges to the canvas.
Evaluate all contours of a single path/glyph together; different paint events
remain sequential. Degenerate contours and opposing edges cancel through the
specified fill rule, rather than a signed-area shortcut.

Charge row/edge inspections, bounded sort comparisons and actual span writes
before the work, with cumulative input/edge/row/clip/mask/allocation limits. A
simple explicitly metered insertion or merge sort is easier to justify than
assuming an undocumented precise comparator count for a standard sort. Clip
masks use the same sample predicate and AND. Initial page bounds still require
the exact original-bound classification discussed below. The independent model
uses per-sample rational ray casting, not this scanline/span algorithm. No new
runtime dependency is needed. This is a design recommendation, not completed
production or verification evidence.

## Audited backend alternative (superseded for R19)

Use owned, bounded device-space curve flattening, then **fixed 4 by 4 binary
supersampling and a box resolve**. tiny-skia 0.12.0 receives only canonical line
contours, identity transforms, opaque solid Gray/RGB colors, `anti_alias=false`
and binary masks. Supersampling is the antialiasing policy of this adapter; it is
not tiny-skia's native antialiaser or a claim of MuPDF antialiasing parity.

At output pixel (x,y), the sixteen intended sample centers are
`(x + (i+0.5)/4, y + (j+0.5)/4)`, i,j in0..3. Paint every event into the opaque
white supersampled RGB/RGBA surface in the original PP1 mixed-event order. Resolve
each final channel with the documented integer rule `(sum_of_16_channels + 8)/16`;
alpha remains255. Do not average each paint first and composite the averages:
that changes overlaps and order. Keep the existing byte color quantization rule.

Use Tr0 glyph fills with nonzero winding and Tr3 as an explicit invisible no-op.
Keep glyph records/provenance but avoid allocating raster geometry for Tr3.
Reject other text painting modes, path strokes (including a stroke suffix of a
fill+stroke operation), CMYK, unsupported color/transparency policy and all
existing unsupported PP1/R18 page content before allocating or publishing pixels.
Empty glyph/path geometry is a deliberate no-op; it is not a backend failure.

Read `Program.events()` for global ordering. A P0 path/clip still needs its saved
user-to-page CTM then page-to-device; R18 commands are already in PageSpace and
receive page-to-device only. Clip references resolve to the immutable PP1 arena,
not a second independently reconstructed text clip stack.

## Why not multiply antialiased masks

Two 8-bit coverage masks cannot represent exact geometric clip intersection by
multiplication. Applying an identical partially covered clip twice squares its
coverage and darkens its edge; clipping must be idempotent. Binary masks at the
same fixed sample grid can be combined with AND, retaining that property.

Allocate clip masks explicitly, render local clip coverage with AA disabled, and
combine with the parent's binary mask in a metered pass. Validate every resulting
byte is0 or255 in tests. Do not call `Mask::intersect_path`, which allocates its own
full-size temporary and performs a hidden whole-surface pass. Cache immutable
clip-node masks under an explicit byte/count budget or rebuild with explicit
repeated-work charges; record the chosen policy. A default "no clip" is not an
empty clip. Restoring a parent must recover its original binary mask exactly.

The initial page rectangle is mandatory even if the caller provides a larger
canvas or translated target. For the normal positive uniform-scale page target,
classify subpixel centers against the original transformed page rectangle, with
an explicit half-open boundary rule. Do not replace it with rounded canvas bounds.
If arbitrary affine targets are supported, use a checked inverse or equivalent
oriented-edge classification against the original page rectangle. Singular or
numerically ambiguous target transforms must refuse. This initial mask must not
be expanded by curve flattening or vertex snapping.

## Bounded geometry and numerical preflight

Transform in f64 and reject nonfinite values before any builder call. Flatten
quadratic and cubic curves with iterative de Casteljau subdivision, explicit
stack depth, visited-node and produced-line limits. Test maximum control-point
distance to the **finite chord segment**, not its infinite line: collinear
overshoot, reversal and loops must not disappear. Suggested flatness tolerance is
1/64 output pixel. At depth/cost exhaustion return a limit error; never accept the
last coarse chord simply because recursion reached its cap. Each retained line
and implicit closing edge must count toward the output and backend edge budgets.

Flattening uses output device space before the4x sample scale. This makes the
error independent of PDF font units, DPI and CTM. Round output vertices explicitly
to a1/256 output-pixel lattice (1/64 sample pixel), with specified tie behavior,
only after retaining the f64 geometry/error bound. This gives at most1/512 pixel
coordinate error per component and exact f32 storage within the proposed domain.
An independent curve oracle must use a different calculation, not the same
recursive flattener. Drop identical consecutive vertices and canonicalize contours
so Move/Close runs cannot trigger unbounded recursive skipping in the backend.
Do not discard a nonempty loop just because its signed area cancels: self-crossing
and opposite-winding subpaths have fill-rule semantics.

**Coordinate bounds alone are insufficient.** `LineEdge::new` quantizes endpoints
to26.6, and `fdot16::div` clamps a slope outside signed16.16. Near-horizontal edges
can encounter that clamp even on a modest canvas. Preflight the actual quantized
edge values: for a nonzero dy6 that produces scan rows, require the checked
`abs(dx6) * 65536 / abs(dy6)` to fit the permitted signed range, along with safe
initial x accumulation. Do not rely on a finite f32 or the painter's enormous
`is_too_big_for_math` threshold. Quantized horizontal/zero-height edges must have
an explicit documented sample-grid interpretation.

The narrowest approved-input proposal keeps flattened/snapped vertices inside
`[0,4*width] x [0,4*height]` in sample coordinates. This avoids backend line
clipping and permits a direct audit of every edge calculation. Out-of-canvas
geometry is an explicit refusal at this first gate, including shapes that could
otherwise be clipped. This limitation must remain visible as a completion gap.
If off-canvas support is required in R19, add a separate checked polygon/edge
clipping preflight with winding-preserving closure and verify its post-clip
quantized edges; simply clamping vertices is incorrect. A finite guard (suggested
absolute8192 output units before clipping) is useful but does not replace that
numeric preflight. The pinned native line clipper can produce at most three edges
per input line; its topology and numeric tests are still required before allowing
that backend branch.

Use output edges at most2047 initially: after4x expansion the largest edge is8188,
strictly below tiny-skia's8191 non-tiling threshold. Width/height multiplication,
strides, pixel counts, mask sizes and all budget products use checked arithmetic.
This is an explicit initial resolution limit, not the application's4096 limit
being silently reduced. Positive finite scale and exact page dimensions remain
checked independently of the memory limits.

## Backend audit and resource reservations

This audit read the cached published0.12.0 files identified in the original audit:
`painter.rs`, `mask.rs`, `edge_builder.rs`, `edge.rs`, `line_clipper.rs`,
`edge_clipper.rs`, `fixed_point.rs`, `scan/path.rs`, `scan/path_aa.rs`, and
`tiny-skia-path/path_builder.rs`. The pinned archive/checksum/license records and
safe-API/internal-unsafe boundary remain unchanged.

- `fill_path` returns unit and can silently skip near-zero bounds, huge paths,
  invalid integer bounds or fewer than two built edges. Our domain/preflight
  must classify the first and last as intentional empty sample coverage and
  prevent the numerical failures. Any unexplained missing paint is a blocker.
- Passing identity avoids path cloning/transforming. The surface cap avoids
  tiling. Passing only lines avoids extrema chopping, recursive cubic clipping
  and the backend's quadratic/cubic coefficient-subdivision machinery.
- `PathBuilder::with_capacity` permits an exact pre-counted point/verb request
  but remains infallible allocation inside the dependency. `BasicEdgeBuilder`
  starts with64 slots, grows a Vec, inserts two sentinels and stable-sorts edges.
  Reserve for growth capacity and stable-sort temporary storage, not just final
  logical edges. The enum stores the largest curve variant even for line edges.
- For L planned edges use E=L+2 in the contained domain, or E=3L+2 only after the
  clipped-line domain has been separately admitted. A conservative published
  dependency allowance is256 bytes per edge slot, at least64 slots, with2x
  growth plus1x sort scratch. This is deliberately larger than the audited enum
  layout; re-audit on any dependency/compiler layout change and record it as a
  logical request envelope, not RSS. Also count the backend path points/verbs,
  plan arenas and full Vec requests in our own layer.
- Preflight scan work per paint/mask is at least
  `(sample_height+2) * E * E + sample_width * sample_height`, plus edge build/sort
  allowance, geometry subdivision and every mask clear/copy/AND pass. A bounded
  number of edges alone does not bound active-edge reordering across scan rows.
  Empty/fully clipped events still charge inspected input and lookup work.
- Output RGBA requires4WH bytes; the live supersampled RGBA surface requires64WH;
  each live binary mask requires16WH. Charge all simultaneous masks and any
  scratch surface, plus cumulative allocations/rebuilds, before construction.
  The final resolve visits16WH samples and writes4WH bytes. Caller-owned scratch
  buffers should use fallible reservation and safe `from_vec`/`from_bytes` APIs.
- Dependency-internal infallible allocations and Arc/Vec allocator behavior do
  not become recoverable merely because the public interface is safe. Retain
  `forbid(unsafe_code)` in the engine and the existing explicit OOM limitation.

Build a private complete RasterPlan first: validated event references, geometry,
clip dependency order, dimensions, byte colors and conservative budget requests.
Only then allocate/render. Unsupported or over-budget suffixes must return no
image; a previous returned image remains immutable and independently owned.

## Independent pixel model and acceptance proposal

Implement a small test-only rational model for tiny canvases. For rational line
contours, each sample center is exact; use half-open Y intervals and signed ray
crossings for nonzero winding or parity, with explicit on-edge tie cases. Intersect
all clip predicates logically, then apply each opaque byte color to matching
samples in global event order and average once at the end. This model must not
call tiny-skia, use its fixed-point edge walker or import production flattening.

For curves, use analytical quadratic cases and high-precision rational Bernstein
subdivision/enclosure with a substantially tighter bound. Classify a sample only
when its winding/side is certified; refine uncertain intervals, and label exact
boundary ties explicitly. Separately compare the production flattened geometry
against that curve enclosure. A shared production/model flattener is inadequate.
Include holes, self-crossings, collinear reversals, tiny loops, coincident edges,
repeated clips, empty initial clips, translated/larger canvases, alternating
text/path paints and glyph holes. Native hinted text is a separate diagnostic:
the primary reference must paint the same unhinted outlines as R18.

Before accepting broad polygons, establish the backend's non-AA sample convention
with slanted/fractional cases. R0 observed scanline differences from MuPDF; do not
assume changing the surface scale makes them vanish. Exact model parity is required
for integer orthogonal and robust non-boundary sample fixtures. For arbitrary
curves/polygons, any backend/model difference must be confined to a computed
quantization/flattening boundary band with an explained cause. If unexplained
interior or winding differences persist, narrow the domain or implement the owned
sample model as the raster path; do not call the approximation exact.

For a native comparison, render unhinted reference geometry with the same page
transform, white background, fill/clip rules and paint ordering. Record exact
semantic parity independently of pixels. Compare native AA and our fixed4x result
with predeclared metrics: differences must be confined to a1-device-pixel band of
shape/clip boundaries; all certified interior/exterior pixels and alpha must
match exactly; no missing connected region or reordered overlap is allowed.
Report max/RMS/mean absolute channel error, boundary-pixel count, coverage-area
error and bounds per case, not only a whole-canvas average. Suggested initial
investigation gates are mean absolute error<=2 and RMS<=12 on normalized8-bit
channels for the small generated corpus, with explicit per-case failures. These
are proposals to test before adoption, not results or adjustable pass thresholds.
If a case fails, explain the geometry/coverage cause and resolve or explicitly
refuse it before acceptance; never loosen all thresholds to make a corpus pass.

Adversarial acceptance must include every one-less budget, large subdivision
requests, exhausted depth, slope saturation, f64/f32 collapse, many crossing
edges, repeated alias clips, clip restore, unsupported trailing stroke/CMYK,
canvas/page-bound mismatch and full output lifetime. Retain all R18 and earlier
gates, normal/minimum Rust tests, fmt/Clippy, feature-tree checks, wasm compile
and comparator mutation self-tests. A passed R19 is a bounded filled-page raster
subset, not full native rendering, text hinting, images, strokes or app integration.
