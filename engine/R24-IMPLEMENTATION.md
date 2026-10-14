# R24 validated opaque image soft masks

Implemented 2026-10-06 after the user resumed with “work on r24.” **R24 is accepted after independent source review and final evidence replay.** Both review findings in the acceptance verifier are resolved; no production loader defect was found. See R24-INDEPENDENT-REVIEW.md. Work remains isolated to the Rust engine; the application is unchanged.

## Implemented contract

An otherwise compatible Gray8/RGB8 image may retain one indirect Gray8 soft mask with matching dimensions only after every effective alpha value is proved 255. Normal Decode with all 255 samples and inverted Decode with all 0 samples are supported. Partial alpha, nested masks, Matte, transfer functions, differing mask dimensions, interpolation and unsupported mask semantics refuse. No general transparency or compositor change is included.

`ImageAsset::opaque_mask()` exposes an immutable owned Arc, retaining final mask identity through aliases and preserving dependency lifetime after the document/session/input is dropped. The loader uses an explicit one-edge mask role, a separate document-bound opacity certificate table and atomic dependent-base publication. Ordinary image cache entries cannot bypass mask-role admission. Successful masked-base reuse reauthenticates the edge and mask role/shape/filter caps; ordinary unmasked R23 cache reuse keeps its zero-metadata-work behavior.

New cumulative session/page counters are `mask_dependency_attempts` (100000 default), `mask_certificate_attempts` (4096) and `opacity_scan_samples` (64000000). A non-null source edge is charged before resolution, including malformed edges and subsequent base failures. Certificate attempts are charged before attempting certification; the entire expected alpha plane is reserved before scanning and remains charged after failure. Completed certificates avoid new scans, while lookups/dependency checks still consume work. Mask decoding, retained bytes, Arc allocations and table growth use the existing ledgers. Arc clones do not allocate new headers.

Errors retain the actual failing object and field plus optional originating base identity inline. The existing page diagnostic-box reservation automatically accounts for the larger error type; its exact allocation-boundary tests continue to pass. New tests cover cache order, shared masks, cross-document identity, failed scan reuse, first/middle/last nonopacity, aliases/null/cycles, per-page/session limits, current structural caps, lifetime, clipped/offscreen validation and late nested-Form failure.

## Verified automated evidence

- 814 tests pass on current Rust 1.98.1 and minimum Rust 1.85.1. Formatting, strict all-target/all-feature Clippy, release binaries/examples and minimum-toolchain wasm32 library compilation pass. The alternate MSRV home is `/private/tmp/tandem-rust-msrv`; the incomplete default-home 1.85.1 entry is not used.
- 44 generated positive masks cover identity, ASCIIHex, ASCII85, RunLength, Flate, LZW EarlyChange0/1, chains and TIFF/PNG predictors, each with normal/inverted Decode. Exact authored alpha planes, native attached-mask bytes, owned Rust mask provenance and decoded base pixels agree. Original native clip/fill/pop callbacks and masked/unmasked native renders are checked. Every positive Rust page matches the separate authored 4× sample model.
- 12 generated negatives refuse in Rust, including early/middle/late nonopacity, Matte, transfer functions, interpolation, nested masks, SMaskInData and a fully transparent control. The strong control demonstrably changes native page pixels relative to the unmasked base.
- Full corpus:634 attempts, including all 632 R23 attempts plus supplementary72 DPI attempts for both actual masked files. **All617 successful R23 PPM hashes remain identical.** At18 DPI,558/572 pages render and14 refuse. At72 DPI,61/62 attempts render; `redact.pdf` refuses at both resolutions because of later marked content. `p3/edit-objects.pdf` is newly certified at18 and72 DPI.
- Original-source independent alpha decoding, the native base’s actual attached mask, and Rust’s retained mask agree on all 60,000 samples in `p3/edit-objects.pdf` and 3,240 in `redact.pdf`. No fixture was altered.
- `tools/verify-soft-mask-gate.py` requires complete identities/coverage, live source/binary/tool hashes, every saved raw artifact, exact generated expectations, regenerated corpus channel certificates and bound Rust check logs. Its adversarial checks reject missing/duplicate coverage, unsupported success, missing provenance, changed/skipped alpha, wrong dependencies/transforms, removed clip/pop callbacks and altered glyph witnesses. The verifier’s exact mutation count is recorded in `reference/soft-mask-acceptance-r24.json`.

Reports: `reference/soft-mask-differential-r24.json`, `soft-mask-rust-checks-r24.json`, `soft-mask-acceptance-r24.json`, and `soft-mask-frozen-baseline-r24.json`. R23 reference tools, policies and reports remain byte-for-byte frozen.

## Reference model correction discovered during verification

The new72 DPI page exposed nine differing channels in three pixels of a rotated Helvetica M, outside the image region. Native f32 outlines placed one coordinate at 459.083984, while the exact source value 459.084 lies across the half-step boundary for 1/256-pixel quantization. Treating the f32 callback as exact had selected the wrong lattice point.

The R24-only model now reconstructs every glyph on this newly supported page from original BT/Tf/Td/Tj operands, source Form/page transforms, pinned AFM integer widths and native raw doubled CFF outlines. It checks all 21 native glyph placements and widths against that source witness before raster certification. The copied AFM is SHA256 `ed4ead49b4d090c80c1d4a8d771879153a41af262d331168dd2635508634cfa1`, under the existing URW OFL notice. This is a strict ASCII/Helvetica source-witness subset; unsupported text syntax refuses. All new page channels now satisfy rederived bounds without widening the inherited path/curve policy or changing production glyph rendering.

## Independent review and remaining scope

The separately delegated reviewer found two blocking acceptance-verifier defects: missing raw native masks/preserved renders could pass, and required executable/tool/source bindings could be omitted. The R24-only reference tools now enforce exact artifact schemas, explicit executable roles bound to checked binary digests, complete source/dependency manifests, coherent success/refusal outcomes, saved refusal diagnostics and Rust masked/unmasked outputs. Production Rust did not need a correction.

Five independent Rust regression tests pass. The complete unmodified report and checks pass before all 407 independently authored omission, role, source-binding and check mutations are rejected. The final verifier also rejects 23 built-in mutations. Native source/attached/Rust opacity proofs and the glyph-coordinate correction were independently replayed. The reviewer records no remaining R24 blocker in R24-INDEPENDENT-REVIEW.md; raw results are reference/soft-mask-independent-{gate,replay,native,glyph}-r24.json.

R24 is accepted for the documented fully opaque, matching-size Gray8 SMask subset. No claim of general transparency, broader masks or complete application replacement follows. No commit, push, publication, installed-app change, runtime dependency, unsafe Rust or resumed automation. General transparency, image masks, broader image codecs/color spaces and the remaining roadmap are still open. Do not advance beyond R24 under the current request.
