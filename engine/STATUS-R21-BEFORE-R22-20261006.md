# Rust engine status

Updated 2026-10-03. **R21 solid-stroke rendering is accepted.** The full Rust
engine rewrite remains active and incomplete; it is not integrated into the app.

## Authorization and isolation

The user authorized implementation until the engine rewrite is done, superseding
the original overnight deadline. The recurring 30-minute automation remains
**PAUSED**. REWRITE-ROADMAP.md maps actual application requirements and final gates.
Worktree: `/Users/owner/.codex/worktrees/rust-pdf-engine/TPDF`; edits only in
engine/. No commit, push, publication or production replacement. Preserve unrelated
main-checkout `.quad/roster` and `progress.md` changes and immutable fixtures.
No OCR. Retain MuPDF-derived AGPL and dependency/font/source attribution.

## Latest capability

Owned, bounded solid positive-width strokes now render alongside R19 filled paths
and R20 embedded/pinned-font glyph outlines. User-space expansion preserves the
saved full affine transform, butt/round/square caps, miter/round/bevel joins and
closed seams. Cubics must satisfy explicit regularity, curvature and numeric
admission checks. Their independent true-boundary model does not replay production
polygons. Four fixed error allowances total at most1/64 device pixel before the
existing1/256 quantization. Hairlines, meaningful dashes, unresolved degeneracies,
folds/reversals and exhausted numeric bounds explicitly refuse.

One stroke session serves each page under per-page and cumulative session limits.
All admitted work survives failures, including a later scanner failure. Complete
page preparation precedes image publication; an unsupported suffix cannot emit a
partial image. Runtime code remains safe Rust with no new runtime dependency.

`--page-render=N@DPI` produces atomic P6 PPM using opaque Gray/RGB, fixed4x4 samples,
unhinted glyphs and intersected clips. It explicitly enables the independent pinned
Helvetica/Times-Roman metric and outline policies. Declared PDF widths control
advances, Encoding/Differences controls outline selection and ToUnicode controls
extraction independently. Input/output caps remain16MiB/64MiB.

## Verification

- **684 tests pass** on Rust1.98.1 and minimum1.85.1. Formatting, strict
  all-target/all-feature Clippy and minimum-toolchain wasm library compilation pass.
- **110 generated stroke cases pass**, including24 exact model/native cases,
  analytic caps/joins/seams, true cubics, affine reflection/shear, clipping/order
  and native minimum-width diagnostics. Zero certified sample-color channel
  interval violations; uncertain samples retain explicit bounded contributions.
- **543/572 fixture pages render at18DPI**, including all500 stress pages;
  29 explicit refusals remain. All43 previously accepted raster pages also pass
  at72DPI, plus stress pages1,250,500. Canvas dimensions are independently checked.
- Native4xAA0 and targetAA8 remain diagnostic comparisons with fixed independently
  derived geometric boundary bands. Each variant has92,300 raw outside-band
  differences across the corpus checks, all independently classified as the exact
  known solid-color policy pair: Rust rounds source .9*255 to230, while pinned
  native code truncates to229. The other channels must match exactly. A structural
  source/paint-order/interior proof excludes later strokes and checks every proved
  pixel, including equal-but-wrong outputs. No blanket channel tolerance or band
  enlargement was introduced. Zero unexplained differences or proof violations.
- Reference validation passes7 positive checks,53 rejected mutations,33 exact-exit
  native-wrapper checks and3 real Rust atomic-suffix refusals. Invalid native
  envelopes/crashes cannot hide behind an independent Rust refusal.

Design/proof: R21-STROKE-PROPOSAL.md and R21-STROKE-REVIEW.md.
Integration/review: STROKE-RASTER-INTEGRATION.md and REVIEW-R21-S1.md.
Native policy/results: STROKE-NATIVE-NOTES.md; original policy-s0 remains frozen,
with the explicit source-derived color amendment in stroke-color-policy-s1.json.
Reports: reference/stroke-raster-{differential,reference-validation}-s1.json.
Logs: `/private/tmp/tandem-r21-{tests,msrv-tests,clippy,fmt,wasm}.txt`.
Frozen CLI: `/private/tmp/pdf-inspect-r21-oracle-final`, SHA256
21e1a4ac418c8610c67e923c7579f55accd661c65675c3a843ba067cdf4f0198.
Raw evidence: `/private/tmp/tandem-r21-raster-evidence`.

## Continuing work

Next is reviewed R22 ordinary Form XObject execution: scoped resources/fonts,
shared graphics/text state, full Matrix/BBox clipping, bounded nested traversal,
owned invocation provenance and cumulative failure accounting. See
R22-FORM-PROPOSAL.md and R22-FORM-REVIEW.md. Forms alone should unlock two current
fixture pages; opaque images follow. The other fourteen Do-first-refusal pages
contain images, including two with soft masks, and later features can still refuse.

Dashes/hairlines, broader fonts, images/Forms, color/transparency and other rendering
capabilities remain open. The full roadmap also requires structured text/search,
mutation/journal, fields/annotations/navigation, security/signature-aware writes
and actual worker/WASM/application validation. A subset milestone is not completion.

## Reproduction and checkpoints

Use `cargo fmt --all --check`, `cargo test --offline --locked --all-features`,
`cargo clippy --offline --locked --all-targets --all-features -- -D warnings`.
Minimum toolchain uses `RUSTUP_HOME=/private/tmp/tandem-rust-msrv` and cargo+1.85.1;
keep builds in /private/tmp. Wasm compilation does not establish runtime parity.
Historical statuses include STATUS-R20-20261003.md and earlier milestone records.
R21 source/docs/reports checkpoint is saved before R22 production implementation
at `/private/tmp/tandem-rust-engine-r21-checkpoint.tar.gz`, with adjacent SHA256 and
manifest sidecars. Builds/caches/downloaded native sources are excluded. Worktree
changes remain uncommitted. The full-rewrite goal stays active; automation paused.

Verified R21 archive:442files,86evidencehashentries,2659704bytes; SHA256
fb64cec50a94bdcb23fb10e684d15c07f888be193c41203353dda40d28963011.
R22 implementation is now authorized under the reviewed contract.

R22 implementation progress: the shared interpreter now has typed Form entry/exit,
true four-corner BBox clipping, a fixed64-frame save floor, indexed scoped font
selection and synthetic boundary origins. Four new core unit tests and35 existing
program/text/font regressions pass. Document traversal/provenance integration,
independent Form tests and final native/model gates remain in progress. No R22
acceptance or full rewrite completion is claimed.
