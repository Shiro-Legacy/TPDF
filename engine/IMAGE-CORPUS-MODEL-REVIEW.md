# R23 image corpus model review

2026-10-06. The adapter author built the new native trace and source model; the separate image reviewer inspected the image ray/inverse math, uncertainty propagation and source-affine reconstruction. This document records their scope and evidence. It does not accept the complete R23 milestone or Rust rewrite.

## Independent inputs and source proofs

`mupdf-image-trace-oracle.c` is a new copy of the frozen R22 Form trace. Frozen R22 sources/reports were not edited. The new device preserves ordered path, unhinted glyph, stroke and clip callbacks, adding opaque `fill_image` callbacks with original unscaled decoded samples, dimensions/components, native top-down placement, clip ancestry, actual stream identity and original Do operation index. Preflight validates the admitted image family before tracing; unsupported later operators cannot expose a successful prefix. All native image samples are obtained with `fz_get_unscaled_pixmap_from_image`, rather than native renderer downsampling. The build verifies the existing Form source pins plus the R23 image/native-library pins.

`image_corpus_model.py` reconstructs image placement independently from original PDF metadata and native-decoded original content spans. It parses the original Page/parent geometry and Form Matrix arrays, then replays original `q`, `Q`, `cm` and implicit Form boundaries with binary64 homogeneous matrix multiplication. The resulting represented coefficients enter an exact Fraction/Gaussian model. Native f32 callback matrices are separately checked for correspondence; they never replace the original source matrix in exact pixel selection.

Original source `g`, `G`, `rg` and `RG` values are tracked through the same save/restore and Form boundaries. Their binary64 byte quantization is independently checked against native paint-color correspondence. This matters on `p6/objects6.pdf`: original `.9` Gray rounds to 230, while the native f32 value would round to 229. That discovery led to source-color proof, not a wider tolerance or a pixel-derived color exception.

## Explicit fixture-only parser limits

The source reader is a deliberately strict fixture certificate, not a general PDF parser. It requires unique original line-start indirect object headers, directly readable dictionary bodies, direct finite numeric Page box/rotation/UserUnit and Form Matrix values, and ordinary parent references. Unsupported indirect numeric arrays/scalars, duplicate objects, missing metadata, malformed numeric operands, ancestry cycles and unsupported source color operations refuse. Native content decoding supplies the actual original operation spans and Form source bytes. The complete driver must pin the original input and native trace evidence together.

Text certification currently requires at most one distinct source text-show color per page; a page needing per-glyph association across several source text colors refuses. The certificate accepts only the already admitted solid stroke profile and Gray/RGB paints. The model bounds canvases to two million output pixels and requires the exact source-derived canvas extent. These restrictions are appropriate only when the selected pinned fixtures satisfy them. They must not be generalized by silent fallback.

## Sample and interval provenance

Image placement uses independent Gaussian elimination to solve source coordinates, integer-scaled inverse rows for fast exact sample evaluation, and exact half-open ray crossings for transformed footprint membership. Stored row selection applies `height*(1-v)` before floor. Exact admitted outer endpoints alone clamp. The image branch does not use production determinant helpers, production sampling code, native fixed-point image placement, path snapping or an image boundary tolerance.

Path/glyph geometry reuses the frozen R22 exact line/rational Bernstein sample classifier; strokes reuse its independent analytic stroke model. Their existing 1/32-pixel curve/stroke uncertainty allowance remains separate from exact image coverage. Per-event conservative hull windows reduce work without excluding possible boundary samples. The model composes every paint in native callback order on one 4× sample buffer with original page bounds and the full clip stack, then applies the existing integer 16-sample resolve.

Only geometrically uncertain samples receive unconstrained channel contributions. Known opaque later paint clears earlier uncertainty at those samples. Every final channel receives its own independently derived inclusive interval; the verifier compares all channels against these intervals. An uncertain pixel is never skipped wholesale. Images themselves introduce no uncertainty. Source-matrix/color checking counts, exact/bounded channel counts, intervals, expected pixels and native trace evidence are retained for verification.

## Verification and mutation sensitivity

`reference/image-corpus-model-validation-r23c.json` records 44 checks:

- 28 exact RGB matches against the separately authored generated image/page model, including reflections, shear, exact ties, clipping, nested Forms, Decode inversion and page geometry. All these image-only channel bounds are exact.
- Two generated mixed text/image/stroke pages compared to frozen Rust pixels with zero channel-interval violations.
- Ten source/trace corruptions rejected: changed callback placement, truncated samples, wrong dimensions, wrong source index, missing image callback, changed clip ancestry, truncated Do span, changed source cm, changed source color and changed original page geometry.
- Altered decoded sample colors change exact output constraints and cause baseline interval violations, demonstrating sensitivity beyond image boundary handling.
- Three native refusal cases—soft mask, interpolation, and an unsupported suffix after an image—return no partial successful trace.

The validation runner pins its full direct/transitive helper closure and executable inputs before execution and checks that the same hashes remain afterward. The final corpus driver separately checks every newly accepted image page at 18 and 72 DPI. The acceptance verifier must rederive model RGB, intervals, counts and certificates from the pinned original/native evidence; trusting saved intervals alone is insufficient. The independent reviewer has added that rederivation gate.

## Complexity and remaining gates

Memory is proportional to the bounded 4× page sample buffer, clip depth and final per-channel interval table. Image work is proportional to conservative transformed bounding-window samples times four ray edges plus fixed-size integer affine operations. Straight geometry work is bounded by cropped samples times edge count. Curves/strokes retain the frozen subdivision/query/work ceilings, and cropped hull windows bound their candidate queries. Initial page masks use exact integer sample ranges and row fills. Python's exact arithmetic is test-only; it is not a production runtime or an unbounded native backend.

The separate corpus driver reports that its final 632 corpus rows plus two mixed cases passed: all 24 newly accepted page/DPI pairs and two mixed cases have zero channel violations across 18,526,122 exact channels and 15,342 bounded channels, with 37 image calls and 37 source-CTM checks. It verified 132 saved compressed evidence blobs against their decompressed hashes. This author review does not independently replace that driver or the acceptance verifier. Whole-corpus coverage, unchanged prior render hashes, manifest/report rederivation, and the full production toolchain checks remain the root's acceptance responsibility.

Final integration note: the complete corpus and acceptance verifier subsequently passed, including rederivation of all24new-page and2mixed certificates plus29structural/4raw mutations. See reference/image-acceptance-r23.json. The user requested pausing after the R23 checkpoint.
