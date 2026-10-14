# Rust engine status

Updated 2026-10-06. **R22 ordinary Form XObjects are accepted.** The full rewrite is incomplete and remains isolated from the installed Electron/MuPDF app.

## Active R23 continuation — 2026-10-06

Opaque Gray8/RGB8 image loading, scoped Form/page Do traversal, owned cached assets, ordered image paint events and exact affine sample composition are implemented. This remains **pending integrated native/model/corpus acceptance**; R22 is the last accepted full rendering milestone.

Current production gate: **796 tests pass on both Rust 1.98.1 and minimum 1.85.1**; format, strict all-target/all-feature Clippy and minimum-version wasm32 library check pass. Fifteen independent review tests cover sampling, loader accounting, exact candidate windows and compositor failures. Additional adapter review fixed scoped document-index work and pre-reserved image diagnostic allocation. Sampling reserves 16384 work units per candidate after review; all 10084 independent Fraction cases retain their exact expected outcomes.

The source-verified final Rust binaries are frozen in `/private/tmp/tandem-r23-final/`; `reference/image-build-r23.json` pins all Rust sources and binaries. CLI SHA256: `bbb4b81bc9c860b14ad800fb2d75f6c9644a75a637a20745fbb034f65434078a`. The final report suite is still being assembled: decoder comparisons, native source-policy witnesses, original-page trace/model certification of newly supported corpus pages, complete previous-output preservation, and a fail-closed acceptance verifier. Intermediate successful reports alone are not R23 acceptance.

Implementation/review contracts: R23-IMAGE-LOADER-DESIGN.md, R23-IMAGE-INTEGRATION.md, R23-IMAGE-INDEPENDENT-REVIEW.md and R23-IMAGE-ADAPTER-REVIEW.md. No new runtime dependency, unsafe Rust, fixture modification or production-app replacement. The recurring automation remains paused; the explicit continuous rebuild goal remains active.

## Latest continuation — R23a, 2026-10-06

Implemented the R23a exact affine image-sampling foundation in `src/image_sampling.rs`. It uses bounded signed 256-bit arithmetic for determinant/coverage and exact nearest-neighbor row/column selection under rotation, reflection and shear. Device edge ownership, top-down source rows, singularity/numeric refusal and cumulative work/attempt budgets are explicit. No heap allocation, unsafe code or new dependency.

Verification: **740 tests pass on Rust 1.98.1 and minimum 1.85.1**, including 16 new arithmetic/sampling tests. Formatting, strict all-target/all-feature Clippy and minimum-version wasm32 library compilation pass. The separate Python Fraction polygon/ray-crossing and Gaussian-elimination model matches **10,084 cases**: 1,358 selected samples, 7,118 outside samples and 1,608 explicit refusals. Five altered/omitted-output mutations are rejected. The final numeric comparison report is `reference/image-sampling-differential-r23a.json`.

This is a tested numeric component, **not complete image support or R23 acceptance**. Image dictionary/filter/predictor validation, retained decoded assets, scoped Do dispatch, ordered image events, sample-buffer composition, native image comparisons and the full image corpus gate remain to implement. Page execution/rendering is unchanged. R22 remains the latest accepted integrated rendering milestone.

Design/audit: `R23-SAMPLING-DESIGN.md`, `R23-SAMPLING-REVIEW.md`. Numeric oracle: `tools/differential-image-sampling.py`; test probe: `examples/image_sampling_probe.rs`. Source/report/log/probe checkpoint: `~/Documents/TPDF/.quad/backup/rust-r23a-sampling-20261006/`. The R22 checkpoint remains untouched.

## Verified capability and gates

R22 ordinary Form XObject execution is accepted after the complete native/model/corpus gate. Scoped resources and fonts, nested Forms, affine Matrix/BBox clipping, state restoration, owned provenance and cumulative failure budgets are verified. These are reusable drawing streams, not interactive AcroForm fields.

- Rust 1.98.1 and minimum 1.85.1: **724 tests pass each**. Formatting, strict all-target/all-feature Clippy and minimum-toolchain wasm32 library compilation pass.
- **58 positive generated Form graphs**, four explicit native-fallback/strict-scope differences and ten malformed/cycle/state refusals pass. **66 generated renders and 10 independent pixel models** pass with zero certified channel-interval violations.
- **545/572 corpus pages render at 18 DPI**, with 27 explicit refusals; all 500 stress pages pass. All **48 supplementary 72-DPI renders** pass. The two new Form-only pages render at both resolutions. All **589 previously accepted render hashes remain identical**; all 29 old refusal pages receive a Form-aware census.
- Reference validation: 23 native comparator mutations, 25 Rust comparator mutations, 13 native process faults, six Rust process faults, three actual native negative-wrapper cases and eight pixel/baseline mutations are rejected as expected.
- The new `tools/verify-form-gate.py` requires exact corpus identities/counts, all generated/model/census coverage, preserved hashes and current evidence hashes. Ten deliberately shortened or corrupted reports are rejected. It prevents a successful `--skip-corpus` or `--native-only` run from being treated as final acceptance.

Native AA/edge differences and the existing source-certified 229-versus-230 color policy remain diagnostic. No tolerance or policy was widened. This is a bounded Forms milestone, not complete PDF rendering or application parity.

## Evidence and reproduction

Final reports: `reference/form-differential-f0.json`, `reference/form-reference-validation-f0.json` and `reference/form-acceptance-f0.json`. Design and independent source review: `R22-FORM-PROPOSAL.md`, `R22-FORM-REVIEW.md`, `REVIEW-R22-FORMS.md`. Native policy/runbook: `FORM-NATIVE-NOTES.md`.

Current source rebuilt CLI SHA256: `16a526548c38db80a1e415812c26bac6e288c38d322a451bb938b0eb5fd5fe1b`. This rebuilt binary differs from the saved October 3 CLI; all 589 accepted raster outputs are unchanged. Full raw evidence is `/private/tmp/tandem-r22-resume-form-evidence`. Logs are `/private/tmp/tandem-r22-resume-{tests,msrv-tests,fmt,clippy,wasm,reference,differential}.txt`.

The complete source/evidence recovery checkpoint is `~/Documents/TPDF/.quad/backup/rust-r22-accepted-20261006/`. Its manifest and SHA256SUMS cover the source archive, CLI, reports, logs and raw evidence. Absolute temporary evidence paths in reports refer to this run; restore those prerequisites from the checkpoint before replaying hash verification on another session.

After the full reference commands in FORM-NATIVE-NOTES.md, run:

```sh
python3 engine/tools/verify-form-gate.py --report engine/reference/form-differential-f0.json --validation engine/reference/form-reference-validation-f0.json --output /private/tmp/form-acceptance-check.json
```

## Next work and boundaries

R23 opaque Gray8/RGB8 image XObject loading and integration are next; its numeric sampling component is implemented and verified above. `R23-IMAGE-PROPOSAL.md` remains a proposal requiring its concrete design/review gate before production implementation. Images/masks, additional fonts, color/transparency, dashes/hairlines, extraction/search, mutations/journal, fields/annotations, security-sensitive writes and worker/application parity remain open in `REWRITE-ROADMAP.md`.

Work stays in this worktree's `engine/`, with necessary handoff updates. No production integration, installed-app replacement, commit, push or publication has occurred. The recurring automation stays paused. Preserve immutable fixtures, unrelated app work, no-OCR, safe Rust and licensing/provenance requirements. WASM compilation does not prove runtime parity.
