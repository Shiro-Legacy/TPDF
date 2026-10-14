# R19 filled-page rasterization

R19 connects an owned G1 page glyph program to opaque RGBA pixels through the new
safe Rust scanner. It adds no runtime dependency and works without the optional
`raster` feature. The older R0 orthogonal tiny-skia adapter retains its separate
scope and checks. The complete application rewrite remains governed by
REWRITE-ROADMAP.md; this stage accepts only the filled-page subset described here.

## Rendering contract

PageRasterSession renders &PageGlyphProgram with an independent RasterTarget and
PageRasterLimits into an owned PageRasterImage. Targets accept positive uniform
scale and translation, with nonzero bounded canvas dimensions. Normal CLI targets
use DPI/72 and the outward-rounded page dimensions. Initial page clipping still
uses the original transformed rectangle and half-open sample classification;
rounding canvas bounds cannot expand content outside the page.

Every output pixel has sixteen sample centers, at offsets (i+0.5)/4 on each axis.
The sample surface starts opaque white. Nonzero/even-odd filled paths and Tr0
nonzero glyph fills paint opaque Gray/RGB byte colors in the original mixed PP1
event order. Color components use nearest integer quantization after scaling by
255. Tr3 text is an explicit invisible no-op and does not allocate flattened
raster geometry. Empty glyphs and degenerate paths produce no sample coverage.
Strokes, including the stroke suffix of a fill+stroke operation, and painted CMYK
are explicit whole-page refusals. Broader font, image, Form, graphics-state and
text-mode restrictions still apply in the upstream G1/PP1 program.

Path/clip vertices receive the saved UserSpace-to-PageSpace CTM, then the target
PageSpace-to-DeviceSpace map. G1 glyph vertices are already in PageSpace and
receive only the target map. Clip nodes refer to the existing immutable PP1 clip
arena; save/restore selects the recorded ancestor. A separate binary mask for
each clip is ANDed with its parent's mask. All masks use the same sample grid,
so an identical repeated clip is idempotent. Fractional coverage is never squared.

Each paint changes the sample surface before the next paint executes. A single
final box resolve computes each RGB channel as (sum_of_16_channels+8)/16, with
alpha255. Averaging each paint separately would change overlap semantics and is
not the implemented policy. Page images remain valid after source/session drop.

## Curve geometry and numeric boundaries

An iterative device-space de Casteljau flattener uses an explicit depth-first
stack, visited-node cap and edge cap. It accepts a chord only when all interior
control points are within1/64 pixel of its finite segment. Finite-segment distance
retains collinear overshoots and loops that an infinite-line test would erase.
Subdivision exhaustion refuses rather than silently accepting a coarse chord.
All generated points are bounded and finite. Curve tolerance is in output pixels,
independent of font units, input CTM or DPI.

Vertices are then snapped to the1/256 output-pixel lattice, nearest with half ties
away fromzero. Maximum per-coordinate quantization error is1/512 pixel. Identical
successive points collapse without edges; contours close implicitly on a new
Move and at fill completion. Opposing contours and selfintersection are handled
by the fill rule, with no signed-area shortcut. The scanner's exact rational
intersection and tie behavior are specified in SCAN-RASTER-DESIGN.md. Initial
page rectangle classification does not use snapped vertices.

## Ownership, atomicity and resources

The page session owns a cumulative raster ledger and a separate cumulative scan
ledger. Per-call raster caps are intersected with session remaining allowances.
The raster ledger counts attempts, pixels, events, source commands, flat edges,
curve nodes, mask/sample/output byte reservations, full allocation requests and
work. Dimension and depth limits also apply. Failed attempts retain admitted
work and reservations; previous successful images remain independent.

A private plan holds flattened clip/paint paths and source provenance. Complete
unsupported-semantic preflight occurs before geometry construction and pixel
allocation. Byte reservations may be charged before semantic refusal; these
counters represent admitted requests, not only successful returned buffers.
Plan arenas, masks, surface and output use fallible vector reservations with
full requested capacities charged before allocation. The scanner's scratch,
edge inspections, sort operations and callback samples have their own ledger.
These logical caps are not a universal RSS bound or a process-OOM recovery promise.

All masks are explicitly allocated, cleared and combined under the budget;
there are no backend hidden full-surface temporaries. Sample writes happen in
private storage. Any geometry, budget or scanner failure returns an error with
no image, even if private work had begun. Error provenance preserves the current
mixed event, operation and source span when available. The low-level scanner is
streaming, but the public page renderer is atomic.

## Inspection and evidence

`pdf-inspect --page-render=N@DPI FILE.pdf` outputs P6 PPM only after successful
full-page rendering. N is one-based; DPI must be positive finite. Existing16MiB
input and64MiB atomic CLI output caps remain. No new desktop application or
worker integration is implied by this inspection command.

Acceptance requires independent per-sample rational polygon expectations and
curve checks, clipping/idempotence/order/transform cases, empty/invisible text,
exact limit boundaries and failed/repeated calls. Controlled native comparisons
use separately serialized unhinted geometry and MuPDF at4x with AA0 followed by
the declared integer resolve. Native hinting and arbitrary slant/curve AA may
differ; those observations must be recorded without claiming general pixel parity.
The oracle must record explicit refusals and comparator mutation/wrapper checks.
Normal and minimum-toolchain regressions, strict Clippy, formatting and wasm
compilation remain required. Implementation and acceptance results are tracked
separately in STATUS.md and the R19 review/reference reports.
