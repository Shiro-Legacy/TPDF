# Rust engine status

Updated 2026-10-06. **R23 opaque Gray8/RGB8 image XObjects are accepted.** The full engine rebuild is incomplete and remains isolated from the installed Electron/MuPDF application.

## Current accepted capability

R23 adds bounded image dictionary/filter/predictor validation, normal/inverted Decode, document-owned cached samples, scoped page/Form image dispatch, ordered image paint events and exact affine nearest-neighbor composition at the existing 4× sample grid. Images compose with paths, glyphs, solid strokes and clips before one sixteen-sample resolve. Output and asset lifetime survive dropping the document/session.

Independent review corrected document-index lookup accounting, reserved image-error allocation, event-limit diagnostics and the sampler's conservative work allowance. Every candidate sample consumes16384 arithmetic work units, including candidates excluded by clips. Exact lattice bounds reduce visits without omitting a possible covered center. No new runtime dependency or unsafe code.

## Accepted verification

- **796 tests pass on Rust1.98.1 and minimum1.85.1.** Formatting, strict all-target/all-feature Clippy and minimum-version wasm32 library compilation pass.
- Exact independent numeric model:10084 cases, with1358 selected,7118 outside and1608 explicit refusals. Five altered-output mutations fail.
- Image decoder:265 exact native/independently authored matches;13 both-refused cases and11 explicit native-recovery/strict-Rust differences. All49 readable corpus documents yield20 image objects:18 exact byte matches and2 mask refusals.
- Generated page pixels:28 exact Rust/model cases. Nineteen match native samples exactly; nine differ under native policy, with every native channel reproduced by a pinned f32/gridfit/fixed14 source witness. No blanket native tolerance.
- Complete corpus:632 attempts; **557/572 pages render at18DPI**, with15 explicit refusals, and **60/60 supplementary72DPI renders pass**. All593 R22 successful PPM hashes remain unchanged. Twelve new pages pass at both resolutions.
- Every one of the24 new corpus renders and2 mixed text/image/stroke cases has an independent original-source/native-trace certificate with zero channel violations. Image sampling introduces no uncertainty; existing source-certified path/glyph/stroke bounds remain explicit.
- Reference fault checks:35 decoder faults,96 page faults and44 corpus-model checks. The final verifier rederives every new/mixed certificate, checks all raw evidence and source/binary/tool identities, and rejects29structural plus4raw-evidence mutations, including fabricated full-range bounds.

## Evidence and recovery

Final gate: `reference/image-acceptance-r23.json`. Exact coverage/tool policy: `reference/image-acceptance-policy-r23.json`. Source-verified production build: `reference/image-build-r23.json`. Test logs are pinned by `reference/image-rust-checks-r23.json`. Detailed references/runbooks are in IMAGE-NATIVE-NOTES.md and IMAGE-CORPUS-MODEL-REVIEW.md; contracts/reviews are R23-IMAGE-INTEGRATION.md, R23-IMAGE-LOADER-DESIGN.md, R23-SAMPLING-DESIGN.md and the independent/adapter review files.

Frozen release binaries: `/private/tmp/tandem-r23-final/`. CLI SHA256: `bbb4b81bc9c860b14ad800fb2d75f6c9644a75a637a20745fbb034f65434078a`.

Recovery checkpoint destination: `~/Documents/TPDF/.quad/backup/rust-r23-accepted-20261006/`. Its source archive, external-evidence restore map and SHA256SUMS preserve this accepted stage. Absolute temporary paths in reports require restoring that map before replaying verification elsewhere. The older R22 and R23a checkpoints remain untouched.

## Pause at this checkpoint

The user requested stopping after the next checkpoint. Preserve this accepted R23 checkpoint and pause the rebuild; resume only on a new user instruction. R24's reviewed next capability is validated fully opaque soft masks, described in R24-OPAQUE-SOFT-MASK-DESIGN.md; it remains design-only. This does not replace later general alpha, image masks, lossy codecs or color management. Additional fonts, marked content/layers, extraction/search, mutations/journal, fields/annotations, security-sensitive writes and worker/application parity remain open in REWRITE-ROADMAP.md.

No production application integration, installed-app replacement, fixture edits, commit, push or publication has occurred. Recurring automation stays paused. Preserve safe Rust, no OCR, source/dependency attribution and independent reference evidence. WASM compilation is not runtime/application parity.
