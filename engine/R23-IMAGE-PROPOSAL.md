# R23 proposal: opaque Gray8/RGB8 image XObjects

Full R23 integration remains a proposal following accepted R22. The user resumed implementation on 2026-10-06; its numeric sampling component is implemented under R23-SAMPLING-DESIGN.md and the corresponding contract entry. See R23-SAMPLING-REVIEW.md for verification and the still-required separate review/integration gates. No new dependency is introduced.

## Measured scope and value

R22's actual native callback inventory found 24 image invocations on 14 of the 16 pages whose first PP1 refusal was Do. A second temporary diagnostic (`/private/tmp/r23-inventory.c`, output `/private/tmp/r23-inventory.txt`) confirms every invoked image is DeviceGray or DeviceRGB, eight bits/component, with no explicit Decode, false/default Interpolate and false/default ImageMask. Two invocations have SMask; none has Mask. Used filters are absent or FlateDecode, including a 200x300 Gray image with PNG Predictor 15. Other sizes range from 1x1 to 200x24 and 180x18, with several small checker/placement images. The inventory observes original images; it is not an accepted decode/raster oracle.

Five pages are image-only: p0b/edit-shared:2, p0b/edit-transformed:1, p6/direct3:3, p6/objects6:2 and p6/paragraph:1. Nine combine images and Forms. After R22, this capability could remove the remaining XObject blocker from those five plus seven unmasked mixed pages, for up to 12 additional candidate pages. Together with R22's two Form-only pages that is up to 14 of the old 16 Do refusals. p3/edit-objects:1 and redact:1 retain soft-mask refusals; redact:1 also has later marked-content semantics. These counts are not a claim that all candidates will pass later operators, numeric bounds or raster budgets. Report actual outcomes after implementation.

## Image dictionary and decoding contract

Accept only an authenticated indirect stream reached through the current R22 Resources/XObject binding, Subtype Image, positive integer Width/Height, BitsPerComponent 8, and ColorSpace resolving to the exact DeviceGray or DeviceRGB name. Do not interpret resource aliases, abbreviated image keys, Indexed/ICC/CalGray/CalRGB/CMYK/DeviceN/Separation, packed depths or 16-bit samples in this first stage. A present Type must be XObject. Named device defaults remain excluded by the effective resource-scope checks.

ImageMask absent/null/false is accepted; true refuses. Any meaningful Mask, SMask, SMaskInData, OC, Alternates, OPI, external-file image source or other rendering-changing extension outside the supported profile refuses before decoding. Resolve references first: invalid/cyclic/dangling mask or optional metadata cannot become absence. A malformed boolean, number, array or stream-valued dictionary field refuses explicitly.

Decode absent/null means `[0,1]` per component. Initially accept explicit pairs exactly `[0,1]` or `[1,0]`, independently for each component; inverted pairs map byte b to 255-b with no rounding. Other Decode ranges refuse, even if finite. Interpolate absent/null/false selects the specified nearest-neighbor sample policy below; true explicitly refuses. Interpolate is not silently ignored.

Reuse `stream::decode_filter_specs_with_usage` and the existing lossless filters: Flate, ASCIIHex, ASCII85, RunLength and LZW. TIFF 2 and PNG 10..15 prediction already exist, including row reconstruction and EarlyChange 0/1. Do not add JPEG/DCT, JPEG2000/JPX, JBIG2, CCITT, Crypt, Brotli or native decoder dependencies. Restrict actual predictors to 8-bit image-compatible parameters: when Predictor !=1, Colors must equal the image components, Columns must equal Width and BitsPerComponent must be 8. Predictor 1 retains existing strict metadata validation. Reject unsupported filter parameters rather than silently dropping them.

After all filters/predictors, require exact byte length `height * (width * components)` using checked multiplication. Extra and truncated rows both refuse; there is no padding, implicit black fill or trailing-data recovery. Retain compact top-down interleaved samples: Gray has one plane, RGB has three adjacent component bytes per pixel. Do not allocate an expanded RGB copy for Gray. Preserve decoded source bytes; apply simple Decode inversion when copying a selected sample.

The current `stream_object` wrapper has per-call 64-hop metadata checks but no cumulative metadata ledger. R23 must either expose a metered prepared FilterSpec helper or prepare specs through the shared document-bound R22 resolver before calling the low-level decoder. Avoid a duplicate unmetered metadata pass. Existing successful DecodeUsage reports encoded input and all stage output lengths, not failed work or full allocation requests. Reserve conservative decode/allocation allowances before calling; only refund successful unused work from trustworthy measured counters. Failed filter/predictor phases retain reservations. An audited conservative full-request envelope, such as three times admitted all-stage output capacity for existing capped geometric growth, must include identity copies and predictor buffers; metadata allocations belong to their separate partition. Do not call final decoded bytes an allocation/work ledger.

## Ordered event and resource integration

Use a document-bound ImageLoader inside PageProgramSession with an independent image/cache budget partition, analogous to the existing font partition. Pass one resolves every used image Do and validates/decodes its asset before the frozen execution pass. Even fully clipped images are validated initially; skipping semantic checks based on visibility is a later optimization requiring its own proof.

Extend the global Program event stream with a typed image event, not a path approximation or a glyph attachment:

```rust
pub enum ProgramEventKind { /* existing Path/Text */, Image(usize) }
pub struct ImagePaint { /* private fields */ }
// image_id(), ctm()->Affine<UserSpace,PageSpace>, clip(), operation_index()
pub struct ImageAsset { /* immutable owned samples + validated metadata */ }
// object_id(), width(), height(), components(), row_bytes(), samples(), decode()
```

A crate-private C0 `push_image` hook accepts an already authenticated asset ID, snapshots the current full CTM and clip, charges its event/work/storage and leaves graphics/text/path state unchanged. Preserve the initial Do-outside-BT/no-active-or-pending-path restriction. PageProgram owns a result-local asset table of Arc<ImageAsset>, while the session reuses immutable decoded assets by actual stream object ID. Scope/name and original Do provenance remain per invocation. Same bytes under a different scope, CTM or clip do not reuse a painted result. An asset cache hit still pays lookup/reference work performed and each invocation/event/paint cost.

PageGlyphProgram retains its PageProgram, so its existing text/path outline preparation can pass image events/assets through without pretending images are glyphs. PageRasterSession adds an image branch at the exact global event position. It consults the current binary clip mask and original page bounds, copies opaque image RGB into each covered 4x sample, and uses the existing 16-sample box resolve. Fills, text, images and strokes must compose before resolve; rendering each image separately and blending resolved pixels would change edges and overlap semantics.

All successful samples and source descriptors survive document/session destruction. The original Do operation origin identifies root content or the enclosing R22 Form invocation. Asset provenance is the image object and validated filter metadata; a selected texel can be described by row/column and decoded byte range. Compressed pixels generally have no one-to-one original encoded-byte subrange, so do not fabricate one. Image errors use R22's pre-reserved bounded diagnostic chain and identify the image object/metadata field or decoder stage+offset; no pixel-sized diagnostics.

## Full affine sampling and ties

The complete image placement is its current user-to-page CTM followed once by RasterTarget's page-to-device transform. The PDF image occupies unit coordinates(u,v), with u increasing right and v increasing upward. Stored row 0 is the top row; source coordinates are `s = width*u`, `t = height*(1-v)`. Compute t before floor. Flipping a previously floored v index is wrong at exact row boundaries.

For final represented affine coefficients `[a,b,c,d,e,f]` and device sample(x,y), use determinant numerators:

```
D = a*d - b*c
U = d*(x-e) - c*(y-f)
V = a*(y-f) - b*(x-e)
```

Normalize D's sign so D>0 and negate U,V when needed. Then u=U/D and v=V/D. Do not assume axis alignment, positive determinant or separable x/y scale. A singular or unsupported numeric transform refuses during whole-page preflight, before pixel allocation. The current general f64 `Affine::inverse` explicitly provides no exact-arithmetic guarantee; a finite inverse plus floor is insufficient for exact texel selection near boundaries.

Keep footprint-edge ownership separate from texel ownership. Proposed footprint policy: exact unquantized transformed unit quadrilateral with the same device-space top/left inclusion convention as the scanline compositor. Orient its vertices to positive signed area; an exact boundary edge is included when its directed dy<0, or dy==0 and dx>0. All incident boundary constraints apply at a vertex. Test this against an independent half-open ray crossing model. Do not use the 1/256 path quantization on image edges or use source half-open bounds as a substitute for this device-edge rule under reflections.

For an admitted covered sample, select `column=floor(width*U/D)` and `row=floor(height*(D-V)/D)`. At interior texel boundaries, floor selects the cell to the right/down in stored top-down source coordinates. If device footprint ownership admits an exact outer source endpoint producing column==width or row==height, clamp that endpoint only to the final column/row. Reject other out-of-range values; never clamp an outside footprint sample into the image. This makes transformed image-edge ties explicit without out-of-bounds reads. Opaque Gray maps to [r,r,r], RGB preserves its three bytes, with the accepted Decode inversions applied exactly.

Recommended numeric implementation: fixed-size exact dyadic determinant predicates and quotient comparisons, with no new dependency. Optional outward-f64 fast classification may avoid exact work only when it proves the same result; it cannot decide uncertain signs/floors. A reviewed 256-bit signed helper can suffice under a structural admission bound: every final affine coefficient and relevant device sample has magnitude <= 2^21 and is exactly representable on one common 2^-K lattice, with 3 <= K <= 80. Include translations, sample centers, subtraction and corner sums in admission; never truncate a tiny coefficient to fit. Scaled values fit signed128 bits (<=2^101 before differences), determinant/numerator products and sums fit below 2^206, and multiplying by dimensions<=2^16 remains below 2^222. The final implementation must prove each intermediate bound, sign normalization and checked shift rather than relying on this coarse envelope. A finite nonzero determinant is evaluated exactly inside that admitted domain; unsupported exponent spread/range refuses explicitly.

The4x centers `(2*k+1)/8` are exact dyadics. Exact integer comparisons can determine coverage and the bounded row/column quotient by a fixed-iteration binary search, avoiding an unbounded big-integer library or division loop. All limb operations/search steps are charged; optionally use proven exact incremental numerator changes across a scan row. A conservative transformed bounding window can reduce visits, but its rounding must be outward and cannot clip away an edge sample. Samples outside clips still count as visited work unless a separately bounded row/mask skip proves they need no lookup.

## Budgets and failure ownership

Suggested initial structural per-image caps: dimensions<=16,384, pixels<=4,194,304, row bytes<=49,152, raw retained samples <= 12 MiB, filter count <= 8, each filter stage <= 16 MiB and all-stage output <= 64 MiB. These are explicit refusal limits, not format limits. The numeric hard dimension bound <= 2^16 remains separate from default 16,384. An image's inflated predictor row bytes can exceed final samples by Height; never cap inflation to final samples before stripping PNG filter bytes.

Proposed cumulative ImageSessionLimits/Usage fields (separate from content/font partitions): load_attempts 100,000; unique_images 4,096; metadata_items 1,000,000; reference_hops 1,000,000; lookup_work 100,000,000; encoded_bytes 256 MiB; decoded_work_bytes 512 MiB; decoded_rows 1,000,000; decoded_pixels 64,000,000; retained_sample_bytes 192 MiB; allocated_bytes 2 GiB. Use checked remaining/intersect/accumulate helpers and per-page limits. Structural row/image caps are not resettable substitutes for totals. Reserve known row/pixel/raw-size counts before decode; invalid size or failed decode still consumes its intended attempt and admitted failure reservations. Successful cache reuse consumes no fictitious decode work but does consume invocation/lookup/paint work.

Raster adds cumulative image_events, candidate_samples, painted_samples and exact_arithmetic_work to the existing allocation/work partitions; suggested initial page candidate cap 64,000,000, whole-session 512,000,000, with an independently checked arithmetic-work allowance. Every bounding-window sample is admitted before clip/coverage/texel work. Repeatedly painting one tiny cached image over the full page cannot evade this quota. All multiplication/indexing is checked, including row*stride+column*components and sample buffer offsets. Whole-page supported-metadata/numeric/event preflight precedes pixel allocation; any later budget or invariant error still discards the entire private raster and retains consumed usage.

## Required independent gates

- Strict decode tests: every accepted lossless filter and predictor, exact rows/bytes, mismatched predictor metadata, truncated/extra data, invalid dimensions/Decode/Interpolate/mask references, stage output limits and repeated failed loads. Verify normal and inverted Gray/RGB against native decoded samples independently of placement.
- Exact Python Fraction model: independently derive affine sample coordinates from generated PDF matrices and the typed final matrix; implement half-open geometric ray crossing and top-down texel floor separately from production determinant code. Cover 2x2 four-color images, non-square rows, scales, reflection, rotation, shear, Form/page transform composition, noncommuting transforms, exact texel/outer-edge/vertex ties and one-dyadic-step neighbors, plus singular and exponent-admission failures.
- Native reference: use actual PDF image decode and full-affine draw at 4x AA0 with `FZ_DONT_INTERPOLATE_IMAGES`, then existing box resolve. Pinned draw code can otherwise interpolate/downsample despite Interpolate=false. Audit other grid fitting/subsampling behavior; controlled boundary-free cases should agree, while exact native edge/texel fixed-point differences are reported separately from the declared exact policy. Do not widen tolerance or label native policy differences matches.
- Composition/provenance: alternating text/path/image/stroke, nested Form BBox and caller clips, repeated object across scopes/pages, root/Form call origins, output after session/document drop, and late soft-mask/image/suffix refusal with zero CLI stdout. Preserve all prior accepted no-image traces and raster evidence.
- Budget tests: exact/one-below every nonzero counter; shared cache across success/failure; per-page/session merge; huge tiny-image replay; exhausted raster budget still retains R22 diagnostics. Full normal/MSRV/fmt/Clippy/wasm and entire corpus classification are required before acceptance.

Suggested ownership after approval: adapter owns image dictionary/asset loader and PP1 resource-plan branch; root owns Program image event, raster sample compositor and CLI; reviewer owns exact affine/texel/clip model plus budget tests; oracle owns native decode/draw configuration and corpus evidence. The fixed-width arithmetic helper deserves a separate source review before integration. This stage does not provide interpolation, transparency, masks or general color management.
