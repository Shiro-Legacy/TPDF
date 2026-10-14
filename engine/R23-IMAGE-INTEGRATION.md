# R23 opaque image integration

This implements the narrow opaque Gray8/RGB8 contract in R23-IMAGE-LOADER-DESIGN.md and R23-SAMPLING-DESIGN.md. Full R23 acceptance is pending the native/model/corpus gate. No application engine replacement is claimed.

## Traversal and ownership

The scoped XObject resolver distinguishes ordinary Forms and image object identities. PageProgram pass one validates each Do's scope, state, metadata and decoded payload, retains owned Arc<ImageAsset> values, and freezes the selected asset index together with operation origin. Pass two only replays that authenticated table. An image Do consumes exactly one original operation index; Form entry/exit retain their existing explicit boundary behavior. Shared aliases and repeat placements retain one decoded asset per document loader. Assets and paint programs survive dropping the document/input.

ProgramInterpreter::push_image validates a Do outside text and active/pending paths, snapshots the same CTM/clip state as the path interpreter, and appends one typed ordered image event. It uses the ordinary operation, event, work and full-request allocation ceilings. Errors are sticky; no partially completed Program is returned. It does not invent font state or serialize synthetic image operators.

## Preparation and composition

The rasterizer validates asset/event provenance and prepares every final user-to-device image transform before allocating any sample/output pixels. An exact ImagePlacement holds the final represented affine coefficients. Singular or out-of-domain placements refuse even through an empty clip. Exact integer corner bounds give an outward candidate rectangle within the canvas; the independent rational sampler determines final coverage and texel identity.

Images retain their place in the same paint sequence as paths, glyphs and strokes. Each candidate center is charged and classified even if its clip excludes it. Covered centers passing the existing initial/page/Form/path clip mask overwrite the private opaque sample buffer with the selected RGB value. Gray expansion and normal/inverted Decode pairs are applied without a second retained pixel buffer. Only after all paints succeed does the existing sixteen-sample box resolve return owned RGBA pixels. Any later failure drops the private partial canvas.

## Budget partitions

ImageLoader metadata/decode/cache counters remain separate from PageProgram traversal and output buffers. PageRasterLimits.images intersects the remaining PageRasterSessionLimits.images and the session's raster image cap for each render. The exposed ImageSamplingUsage reports preparation attempts, candidate samples and conservative arithmetic work. Success and failure both accumulate these counters; no failed page resets its budget. Preparations bound image paint events, and candidate samples bound both covered and painted samples, so separate duplicated event/painted-sample counters from the preliminary proposal are unnecessary.

Candidate-loop work also consumes the ordinary raster work ledger. Geometry plans, image placement descriptors, paint records, clip masks, sample buffers and final pixels consume the raster full-request allocation ledger. The sampler itself uses fixed-size stack arithmetic and no heap allocation. Caller-provided assets are already owned and bounded by image loading. Logical limits do not promise a universal resident-memory ceiling or allocator-failure recovery.

## Scope and verification

Inline images, masks/soft masks, interpolation, color-key masks, other bit depths/color spaces and lossy codecs are explicitly unsupported. Existing unsupported suffixes remain atomic failures. No new runtime dependency or unsafe code is introduced.

Direct tests cover top-down rows, reflection, ordered path/image overlap, clip save/restore, subpixel white-background resolve, off-canvas and fully clipped images, pre-pixel singular suffix refusal, exact/one-below budgets and retained failed-render accounting. Independent tests and native byte/page comparisons are recorded separately; the full corpus and prior-pixel preservation checks are required before calling R23 accepted.
