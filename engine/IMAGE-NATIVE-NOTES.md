# R23 image reference evidence

The image references are test-only, bounded and separate from frozen R19–R22 tools and reports. They do not install a native production backend or claim full PDF/application parity. Policy is frozen in `reference/image-reference-policy-r23.json`; source/archive-build libraries are checked by `tools/build-image-reference.sh` against `reference/image-native-pins.sha256` before linking. Static-library hashes identify this already verified local reference build; recompiling its archives requires explicitly re-verifying those build artifacts.

## Decode oracle

`mupdf-image-decode-oracle` reads one original indirect image object from a real PDF and calls MuPDF's image decoder and unscaled pixmap API. It checks source dimensions/components, rejects masks, requires opaque output and emits top-down RGB P6 bytes only after successful completion. Gray is expanded solely at the diagnostic output boundary. MuPDF applies the PDF Decode array itself. Input is capped at16MiB, native heap128MiB, dimensions16384, pixels4194304, output64MiB, CPU10s and wall15s. Repairs/warnings/errors fail closed. Oracle argument/input errors exit1; semantic/decode errors exit2 with zero stdout. Rust's independently implemented loader probe emits the same atomic P6 envelope and exits2 for errors.

265 positive generated original PDFs cover Gray/RGB, normal and per-component inverted Decode, identity/ASCIIHex/ASCII85/RunLength/Flate/LZW (EarlyChange0/1 and code-width transitions), ASCII85+Flate chains, TIFF2/PNG10–15 prediction with Flate/LZW, one-pixel and nonsquare images, and indirect dimensions/color/depth. Every decoder must reproduce independently authored bytes exactly.24 strict negative cases distinguish both-refused from native-recovery/strict-Rust refusal. Native acceptance of extra samples, noninteger metadata or ignored semantics is recorded, never called strict parity.

`mupdf-image-inventory-oracle` scans bounded actual xref entries in all49 readable fixture documents. This is an image-object inventory, including unused images, not a count of executed Do callbacks.20 images are present:18 decode byte-for-byte identically and2 masked images explicitly refuse. Fixture inputs are authenticated by the frozen inventory hashes. Encrypted fixtures retain the prior exclusion.

## Full original-page native raster

`mupdf-image-page-oracle PAGE DPI` executes original page contents with the real native PDF interpreter and draw device, AA0, white opaque RGB, minimum line width0 and `FZ_DONT_INTERPOLATE_IMAGES`. It sets `FZ_IMAGE_RENDERING_QUALITY`. Input/heap remain16/128MiB; dimensions4096 and pixels16777216; CPU30s/wall45s. Native repair/warning/error fails before stdout.72DPI Rust comparisons use288DPI native samples followed by the frozen integer16-sample box resolve. This renderer provides original-page evidence, not a reconstruction from Rust display lists.

Pinned source audit explains why Interpolate=false alone is insufficient. `draw-device.c:1938` disables smooth image prescaling only with the device hint, and `draw-affine.c:3931` otherwise enables interpolation for nonrectilinear/upscaled placements. `image.c:1078` bypasses power-of-two decode subsampling in QUALITY mode. `draw-device.c:1897` still calls `fz_gridfit_matrix`; `draw-affine.c:3727` uses ordinary outward grid fitting with `.001f` comparison epsilon for axis-aligned and quarter-turned matrices. The source first flips the PDF image vertically (`pdf-op-run.c:918`). It then pre-scales by decoded dimensions, calculates an inverse, casts coefficients to float, truncates them to14 fractional bits and advances integer texture coordinates across rows/spans (`draw-affine.c:3976–3990`, `PREC` at33). These choices differ from R23's exact unquantized footprint and exact top-down texel policy.

`image_native_policy.py` is a source-derived diagnostic witness for the generated fixtures. It reproduces these f32/gridfit/fixed14 steps, and uses the frozen independently tested native AA0 path oracle only for original authored clip polygons. Every native sample channel must equal the witness. It never uses Rust output or full-page native pixels to derive its prediction. Native corruption, including a one-byte interior or white-background change, fails this comparison. Exact independent Rust sample equality is a separate mandatory check: rational Gaussian inversion, exact half-open ray crossing, Decode and ordered opaque per-sample composition. No difference is hidden under a generic diagnostic label.

The generated page gate has28 cases:19 exact native/exact-model agreements and9 differences with1050 raw differing sample channels, all exactly reproduced by the native source-policy witness.96 byte-corruption, envelope and real process/refusal mutations are rejected. The decode envelope/process gate rejects35 faults.

Generated geometry includes Gray/RGB, reflections, quarter rotations, nonsquare texels, shear/full affine transforms, exact footprint/texel ties and dyadic neighbors, ordered image/path/image painting, caller clipping, nested Form matrices/BBoxes, Decode inversion, and page CropBox/Rotate/UserUnit. Native differences retain raw channel counts, maximum differences, source-step records and compressed sample evidence; exact native agreement remains separately identified.

## Reproduction and acceptance boundaries

```sh
sh engine/tools/build-image-reference.sh /private/tmp/tandem-mupdf-reference
python3 engine/tools/differential-images.py --rust /private/tmp/tandem-r23-final/image_decode_probe --report engine/reference/image-decode-differential-r23b.json --evidence-dir /private/tmp/tandem-r23-image-decode-evidence
python3 engine/tools/test-image-reference.py --rust /private/tmp/tandem-r23-final/image_decode_probe --report engine/reference/image-decode-validation-r23b.json
python3 engine/tools/image-corpus-inventory.py --rust /private/tmp/tandem-r23-final/image_decode_probe --report engine/reference/image-corpus-inventory-r23b.json --evidence-dir /private/tmp/tandem-r23-image-corpus-evidence
python3 engine/tools/differential-image-pages.py --rust /private/tmp/tandem-r23-final/pdf-inspect --report engine/reference/image-page-differential-r23c.json --evidence-dir /private/tmp/tandem-r23-image-page-evidence
python3 engine/tools/test-image-page-reference.py --report engine/reference/image-page-validation-r23c.json
```

Full corpus evidence requires all572 pages at18DPI, all48 prior supplementary72DPI attempts, plus all new image pages at72DPI. All593 R22 successful PPM hashes must remain identical. The27 old refusal identities are retained explicitly. The final632-attempt corpus run found12 new successful image pages, giving557/572 renders and15 refusals at18DPI; all60 supplementary72DPI renders pass. All593 previous successful hashes remain identical. All24 new image-page renders and2 mixed generated renders pass the mandatory actual-page source-aware independent certificate gate:18,526,122 exact channels,15,342 conservatively bounded nonimage-geometry channels,37 checked image invocations, and zero violations. All27 old refusal identities have native trace census evidence. A diagnostic-only corpus classification would not satisfy this gate. Reports are accepted only with complete dependency/source/binary hashes and retained evidence, after final production build verification and all normal/MSRV/fmt/Clippy/wasm checks.

## Actual-page image correctness certificate

`mupdf-image-trace-oracle` is a new image-aware copy of the frozen Form trace. Its real native device callbacks preserve image/path/text/stroke order and clip ancestry, including nested Forms. Image callbacks emit bounded unscaled decoded samples; original decoded content byte spans and Form invocation descriptors provide source provenance. `image_corpus_model.py` independently reads original PDF geometry/Form dictionaries and native source spans to reconstruct binary64 image placement. It checks native callback CTMs but never substitutes rounded native floats for exact Rust image placement.

The actual-page model uses rational Gaussian inversion and exact integer half-open ray crossing for image samples. Source g/rg/G/RG operations and q/Q/Form state determine color bytes independently; this specifically avoids the known native-f32 .9 Gray229 versus source-binary64 Gray230 conversion difference without widening any channel interval. Nonimage geometry retains the frozen R22 curve/stroke boundary certificates. Every RGB channel is checked against its own resolved interval. There is no entire-pixel omission and no fallback image uncertainty band. Output bounds are a row-major RGB list of integer `[lower,upper]` pairs. Every new image invocation must have a reconstructed original source CTM.

Two additional actual text/image/stroke mixed fixtures use this same gate. `image-corpus-model-validation-r23c.json` separately records28 independent generated image model checks,2 mixed pixel checks,10 trace/source corruption rejections, a decoded-sample mutation sensitivity check, and3 native atomic refusal checks.

```sh
sh engine/tools/build-image-trace-reference.sh /private/tmp/tandem-mupdf-reference
python3 engine/tools/differential-image-corpus.py --rust /private/tmp/tandem-r23-final/pdf-inspect --report engine/reference/image-corpus-differential-r23c.json --evidence-dir /private/tmp/tandem-r23-image-corpus-pages
python3 engine/tools/test-image-corpus-model.py --trace /private/tmp/tandem-mupdf-reference/mupdf-image-trace-oracle --rust /private/tmp/tandem-r23-final/pdf-inspect --output engine/reference/image-corpus-model-validation-r23c.json
python3 engine/tools/verify-image-gate.py --policy engine/reference/image-acceptance-policy-r23.json --output engine/reference/image-acceptance-r23.json
```

Final measured oracle evidence (2026-10-06): decode265 exact +24 strict classifications; object corpus49 documents/20 images; generated page28 exact Rust models with28 exact native source-policy witnesses; decode/process35 and page/process96 mutation checks; complete corpus632 attempts plus2 mixed generated certificates. These references are complete for the declared R23 profile. Acceptance is still pending while the root verifier runs. Only its passing result, the normal/MSRV/fmt/Clippy/wasm checks and the final source-build manifest authorize accepting the integrated R23 milestone. Even an accepted R23 milestone is bounded opaque-image support; the complete Rust engine rebuild remains unfinished.

## Final acceptance

The final gate passed in `reference/image-acceptance-r23.json`: all632corpus identities,593prior render hashes,24new-page certificates and2mixed certificates were verified from original/raw evidence. The verifier rejected29structural and4raw-evidence mutations, including forged full-range bounds. R23 is accepted as the bounded opaque-image milestone; the full Rust engine rebuild remains incomplete. The user requested stopping after this checkpoint, so R24 remains design-only.
