# R19 raster reference policy (declared before final comparisons)

The primary specification is the owned Rust fixed 4x4 sample raster policy.
Native text pixels are not the specification: MuPDF hinting and its native
antialiaser remain distinct. R18 source identity/placement gates continue to
apply before raster comparisons.

## Acceptance and diagnostic populations

1. Controlled endpoint-color orthogonal real PDFs must match the independent
   expected pixels and the native 4x AA0 box resolve exactly. This population
   includes clips, compound winding/parity, restores, paint order, transformations,
   and fractional source coordinates producing controlled device grid boundaries.
2. General line contours must match an independent per-sample rational ray-cast
   model, not a scanline sorting/span algorithm. Offcanvas winding, crossings,
   holes, coincident/opposing edges, and quantized fractional vertices are included.
3. Curves need independently certified sample classification or explicit
   uncertainty confined to the declared flattening/quantization band. The model
   may not import production geometry or reproduce the production finite-chord
   flattener as its only source of expected coverage. Exact robust curve cases
   and approximation-band cases must be labeled separately.
4. Every readable fixture page is attempted. Unsupported R18 glyphs, strokes,
   painted CMYK, dimensions and budget errors remain explicit refusals. Passing
   a narrow render population does not retract earlier parser/trace support or
   imply the complete engine replacement is finished.

The native geometry pipeline consumes the independently produced R18 native
trace. It serializes the ordered native path/clip/color state and actual
unhinted glyph outline commands into a font-free geometry stream. Quadratics
are expressed as equivalent cubic commands; no Rust-rendered pixel or flattened
Rust geometry is used as native input. Native AA0 renders at four times each
output dimension, followed by `(sum_of_16_channels+8)/16`. Native AA8 at output
resolution is a second diagnostic. Endpoint channels0/255 avoid the separately
known native interior color quantization policy difference.

Arbitrary native slant/curve/AA differences are diagnostics, not an adjustable
exact-parity threshold. Report different pixels, maximum/mean/RMS channel error,
difference bounds, darkness-area error, and boundary count. Where a conservative
one-device-pixel boundary mask is available, differences outside it must be
identified separately and investigated. Mean error>2 or RMS>12 are predeclared
investigation flags, not thresholds to silently widen after seeing output.
Unexplained interior/winding failures block a claim for the affected population.
No whole-canvas average can excuse a missing shape or reordered overlap.

## Native helper bounds and failure policy

`mupdf-page-raster-oracle` accepts WIDTH HEIGHT AA and standalone geometry stdin.
AA is exactly0 or8. The helper permits only q/Q, cm, m/l/c/h/re, f/F/f*/n,
W/W*, g/rg with checked operand counts and balanced graphics/path state. Arrays,
strings, resources, text, images, transparency, strokes, and other operators are
rejected. Color components are finite0..1 and numeric magnitudes at most1million.

Limits: input4MiB, dimensions1..4096, pixels<=16,777,216, native heap128MiB,
500,000 tokens, graphics stack128, CPU30 seconds and wall45 seconds. Original
native warnings/errors are fatal. Rendering and diagnostics finish before stdout
receives the P6 header or pixels. This executable cannot load external fonts or
resources. Existing MuPDF/FreeType provenance and AGPL reference-only licensing
remain unchanged; no native runtime dependency is introduced into Rust.

Final report paths, exact counts, source hashes, native difference observations,
certification scope and reproduction commands will be appended after the gates.

## R19 final outcome and limits

The frozen production binary is SHA-256
`06fccd5d08917c87d74508af3d8250f44ba7c32c99fc8ee16a928d1c156945cb`.
The independent model is `tools/page_raster_review_model.py`, SHA-256
`8ef0da904c591c97c237de6949862b7f13f17ca62c05981c39ea5e091382d6d4`.
Its pure-line branch casts a separate exact integer ray for every sample; it
never sorts intersections or emits spans. Curves use rational Bernstein interval
and monotonic-crossing certification, independent of production chord flatness.
For mixed curves the uncertain sample band is conservatively1/32 pixel around
curves and1/256 pixel around unsnapped lines. Initial page bounds remain exact.
A pixel is certified only when all sixteen final visible samples are certified.

The full generated gate contains442 real PDFs and362,577 output pixels:

- 322 controlled orthogonal cases have exact model and native4x AA0 equality,
  including fractional sample coverage, repeated clips, and36/72/144 DPI.
- 108 general line cases have exact independent sample-model equality, including
  offcanvas paths, self-crossings, opposing contours, winding/parity, quantization
  ties, and clip restore. Native scan conversion is a separate diagnostic.
- 12 curve cases include cubic arches/overshoot/loops/reversals, repeated curve
  clips, and embedded quadratic glyphs. Ten output images match the exact model's
  resolved bytes. Two arch images differ at two pixels each, all within the
  predeclared approximation band. The model flags76 uncertain pixels across all
  curve cases; these are not counted as fully certified pixels merely because
  some happen to match.
- A one-pixel embedded quadratic is independently analytical: x=t and
  y=2t(1-t), with exactly6of16sample centers covered. Rust returns RGB159 exactly
  and the model reports no uncertain samples. Nontrivial tiny-loop cases are
  also fully certified. These establish more than line-only curve smoke tests.

Every one of572readable fixture pages is attempted independently. Both previously
eligible R18 pages remain rendered: `p3/field-defs.pdf` and
`p3/simple-subset.pdf`. The other570pages produce explicit atomic CLI refusals,
including all500stress pages. Only exit2 with a nonempty `pdf-inspect: ` error
message counts as a refusal; signals, unexpected exits, empty diagnostics and
partial outputs are failures. The previous eligible-page set is asserted rather
than allowed to silently shrink. This does not replace the broader PP1 trace gate.

The blank fixture matches native pixels. For simple-subset at72DPI, native4x
AA0 differs at430shape-boundary pixels (maximum channel difference95,
MAE0.037974, RMS1.430882, darkness-area difference-1.129412pixels). Native AA8
differs at644boundary pixels (maximum39, MAE0.016385, RMS0.587674, darkness-area
error-7.247059pixels). Both retain the same[72,163,190,192] difference bounds;
no difference lies outside the declared conservative boundary band. These are
unhinted-reference diagnostics, not native hinted text pixel parity.

Native arbitrary-geometry differences are substantial in some tiny cases and
remain visible per case. Ninety-five generated native4x comparisons trigger the
predeclared investigation flag;83native AA8 comparisons trigger it. The largest
native4x mean/RMS error is32 on the one-pixel analytical quadratic; MuPDF's own
curve rasterization is not the declared sample-point algorithm. The largest
AA8 mean error is33.694445 and RMS71.529908 for repeated fractional clips.
Native AA coverage clips lose coverage on repeated application, unlike binary
sample-mask AND. Slanted sample-on-edge ties, native edge quantization, and
native curve flattening produce additional differences near boundaries. Their
independent model checks pass; the reports do not relabel those native outputs
as exact parity or loosen the investigation thresholds. No difference outside
the conservative one-pixel native diagnostic band is accepted.

Validation has14 independent model identities,4 positive comparator baselines,
23rejected mutations (including crashes and lost eligible pages),25native wrapper
checks, and2Rust atomic unsupported-suffix checks. Model identities include exact
half-open boundary counts, opposite/coincident winding, parity cancellation,
unsnapped initial bounds, order/overlap, clip idempotence, analytical quadratic,
opaque hiding of prior curve uncertainty, and integer resolve ties.

Reports:

- `reference/page-raster-differential-r1.json`
- `reference/page-raster-reference-validation-r1.json`

Reproduce from the repository root with the pinned native reference build:

```sh
sh engine/tools/build-page-raster-reference.sh /private/tmp/tandem-mupdf-reference
python3 engine/tools/test-page-raster-reference.py --rust /private/tmp/pdf-inspect-r19-oracle-final --report engine/reference/page-raster-reference-validation-r1.json
python3 engine/tools/differential-page-raster.py --rust /private/tmp/pdf-inspect-r19-oracle-final --report engine/reference/page-raster-differential-r1.json --evidence-dir /private/tmp/tandem-r19-page-raster-evidence
```

The build script pins native draw-device/edge/rasterize/path/paint/blend and PDF
interpreter source hashes. The reports bind native/Rust executables, source,
models, input fixtures, rendered bytes and the predeclared policy. Compressed raw
render/geometry/trace evidence is retained in the stated temporary directory.
Direct hinted native PDF pixels are not a hidden input to this gate; R18's
separately recorded hinted outline differences remain the applicable evidence.
