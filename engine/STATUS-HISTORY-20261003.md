# Rust engine status

## October 3 morning continuation

The user requested that the 30-minute messages stop and authorized continued
engine work. Automation `rust-pdf-engine-overnight` is **PAUSED**, verified in
its saved configuration. Continue interactively in the current chat; do not
resume or replace the scheduler. Prior subagent authorization was verified from
the user's original message. Current ownership: page_adapter = PP1 library and
owner tests; page_oracle = native page oracle/corpus gate; page_review = independent
adversarial review; root = CLI, integration, acceptance and this log.

Pre-continuation R15 source archive (including unlinked drafts):
`/private/tmp/tandem-rust-engine-r15-before-pp1.tar.gz`, SHA-256
`4df04cd5d5786ac1b23bae38c4361a86cf2f8eec978c077108386f8dd955f58a`.
The historical sections below record earlier checkpoints; PP1 is not yet accepted.

Updated October 3, 2026, 07:29 UTC. Authorized work ends at **13:00 UTC / 09:00 Toronto**.

Worktree: `/Users/owner/.codex/worktrees/rust-pdf-engine/TPDF`, crate `engine/`.
The user explicitly authorized autonomous implementation, subagents and safe Rust
redesign under MuPDF licensing through the deadline. CONTRACT.md records scope
before each implementation milestone. Do not ask again for routine authorization.

All changes remain inside `engine/`; no app integration, commit, push or publication.
The original checkout `/Users/owner/Documents/TPDF` has unrelated `.quad/roster`
and `progress.md` changes; leave them alone. Worktree writes use reviewed shell
escalations because it lies outside the ordinary writable roots. No rejections.
Fixtures are immutable and all corpus harnesses verify their hashes. No OCR.

## Latest accepted checkpoint

R15 O0 is accepted. **513 all-feature tests** pass on both normal Rust 1.98.1 and
the declared minimum 1.85.1. Formatting and all-target/all-feature Clippy with
warnings denied pass. The all-feature library compiles for wasm32-unknown-unknown
on 1.85.1. This does not establish browser or Windows/Linux runtime validation.

Logs: `/private/tmp/tandem-r15-tests.txt`, `tandem-r15-clippy.txt`,
`tandem-r15-msrv-tests.txt`, `tandem-r15-wasm.txt`.
Checkpoint: `/private/tmp/tandem-rust-engine-r13-checkpoint.tar.gz`, SHA-256
`9503b573e07bc365745899d5e32321040423f26170b834fd6e80e0fc1faff4d3`.
It excludes builds, downloaded sources and caches. Later design/docs are not in it.
Earlier R3/R6/R8/R9/R12 checkpoints also exist under `/private/tmp/`.

The production library and CLI forbid unsafe code. Default dependencies are only
miniz_oxide 0.8.9 and adler2 2.0.1, which also forbid unsafe. The optional raster
feature adds pinned tiny-skia 0.12.0 and its audited Rust dependencies, with
internal unsafe disclosed and complete notices retained. No production C/FFI.
Builtin metric data retains its source OFL license; surrounding code remains AGPL.

## Implemented and independently reviewed

- Strict byte lexer, owned direct objects and bounded parser, exact spans/errors.
- Classic/stream xrefs, incremental precedence, compressed object streams, checked
  object/stream ranges and global document limits. No encryption/hybrid/repair.
- Flate, ASCIIHex, ASCII85, RunLength, LZW EarlyChange 0/1, TIFF/PNG predictors,
  exact cumulative intermediate decode-work accounting.
- Page inventory, inherited boxes/rotation, non-inherited UserUnit and typed page
  placement. Conservative arithmetic rejects severe cancellation/singularity.
- Bounded object serialization and full rewrite preserving effective live objects
  and raw streams. Signed/linearized/unknown meaningful trailers/dangling refs
  refuse. Rewrite native pixel parity demonstrates preservation, not Rust rendering.
- Reference-free content grammar, standard operand validation, exact logical page
  content joining and member provenance. BI refuses before binary lookahead.
- Strict ToUnicode and four explicit simple encodings, byte-preserving Unicode
  decoding with scalar/source/UTF8 spans and no fallback.
- Document-bound lazy page font resources with independent mapping, explicit-width
  and builtin-width caches. Failed work is charged; no Form expansion or Type0.
- P0 path/state execution into immutable flat path/dash/clip arenas. Mixed CTM
  changes within an active path and unsupported operations refuse.
- R0 optional orthogonal rasterizer: exact integer in-canvas simple nondegenerate
  contours, fills and binary clips only. Curves/strokes/slants/fractional device
  coordinates/CMYK refuse. Gray/RGB uses declared round-half-up quantization;
  native exact pixel parity uses endpoint colors. Backend OOM remains process-level.
- M0 explicit widths, including signed/fractional/zero advances and missing-width
  provenance; M1 exact Helvetica/Times-Roman Type1 substitution advances, 654 names
  per face from the verified AFM/CFF intersection. No aliases or embedded fallback.
- T0 standalone horizontal source-code positioning: injected borrowed font policies,
  typed text/line matrices, spacing/rise/scaling, TJ adjustments, Tr0/3 and q/Q/cm
  outside BT. Cumulative allocation/work/numerical limits and atomic sticky errors.
  One source code receives one advance even with multiscalar Unicode. Provenance
  uses decoded-string offsets plus enclosing operation spans, not raw escaped offsets.

No complete PDF renderer, glyph outlines, page extraction, reading-order inference,
font shaping, image rendering, editing UI or app replacement is claimed.

## Evidence summary

| Gate | Accepted result | Main evidence |
| --- | --- | --- |
| Lexer | 2,008 native valid cases, eight strict refusals | differential-r1.json |
| Filters/predictors/LZW | 1,036 / 700 / 754 valid cases; independent LZW seed 194 | differential-filters-r3.json, differential-predictors-r3b.json, differential-lzw.json |
| Reader/pages | All 49 unencrypted fixtures, 572 pages; five encrypted unsupported | fixture-comparison-r4b.json |
| Real decoded streams | 639 exact streams, 7,403,637 bytes | stream-comparison-r4b.json |
| ObjStm reader | 96 generated PDFs, 286 pages, no native repair | object-stream-differential.json |
| Serializer | 1,000 native cases, 12,282 independent finite floats | serialize-differential-r5a.json, REVIEW-R5A.md |
| Rewrite | 33 eligible fixtures / 554 native pixel-identical pages / 600 streams; 96 generated PDFs / 286 pages / 384 streams | rewrite-comparison-r5b.json, rewrite-object-stream-comparison-r5b.json |
| Content grammar/schemas | 890 inputs / 31,221 ops; 590 full streams / 27,629 ops | content-differential-r6a.json, content-schema-r6a.json |
| Page content joining | 579 cases, 2,543,602 exact bytes | page-content-differential.json |
| ToUnicode | 164 maps, 3,680 entries, 8,402 queries; malformed cases refused | cmap-differential-u0.json |
| Mapped text | 106 cases, 6,332 codes, 39,116 UTF8 bytes | text-decode-differential-r7b.json |
| Encodings | 1,024 slots, 378 names, 210 cases / 53,760 slots | simple-encoding-differential-s0.json |
| Page placement | 305 PDFs / 828 pages, including all fixture pages | page-geometry-differential.json |
| F0 font resources | 582 bindings / 575 simple loads / seven Type0 refusals, 53 page-bound simple identities | font-resource report, REVIEW-R9-F0.md |
| P0 path execution | 310 programs, 5,835 events, 20 refusals | path differential, REVIEW-R9-P0.md |
| D0 encoded text | 110 strings, 50,674 codes, 98,146 UTF8 bytes, 286 refusals | encoding-text-differential-d0.json |
| M0 explicit widths | Five fonts / 160 entries (90 zero) / 1,280 lookups | font-metrics-differential.json |
| R0 raster | 304 programs / 311,296 exact endpoint-color pixels; 2,042 independent color samples; 12 refusals | raster-differential-r0.json, REVIEW-R11-R0.md |
| M1 builtin widths | 1,308 entries, 402 exclusions, 49,152 standalone slots, 48 fixture fonts / 12,288 slots, 37 refusals | builtin-metrics-differential-m1.json |
| T0 positioning | 246 exact rational programs / 2,906 placements, 160 native cases, 48 refusals | text-position-differential-t0.json, REVIEW-R13-T0.md |

JSON evidence is under `reference/`. Native T0 float32 tolerance was declared
before runs; observed maximum absolute component error was 7.6e-6. The 54-font
all-object inventory includes one Form-only font; F0's 53 page-bound identities
are deliberate. M1 loads 570 of 582 bindings; five explicit-width fonts and seven
Type0 bindings refuse distinctly. A preceding 1,600-case mutation smoke passed;
this is not coverage-guided fuzzing, a security audit or process RSS profiling.

## Active planning and ownership

No post-T0 implementation milestone is approved yet.

- Root owns integration, CLI, text_font.rs, docs, acceptance and checkpoints.
- Lexer is drafting PAGE-PROGRAM-DESIGN.md for mixed page path/text orchestration,
  exact state synchronization, clipping/color context and prepare/freeze fonts.
- Reference independently critiques that page architecture and proposes adversarial
  tests/native gates. It completed T0 review with no remaining blocker.
- Parser recorded EMBEDDED-OUTLINE-INVENTORY.md: all five explicit-width fixtures
  use TrueType glyf/loca, 1,885 glyph slots / 72 nonempty simple glyphs / no composites.
  It is drafting a bounded standalone TrueType outline design. No new dependency.

Normal `target/debug/pdf-inspect` and `target/raster-gate/debug/pdf-inspect` are
free. Freeze/hash the relevant binary during differential runs. Dedicated gate
builds avoid cross-agent binary replacement. Do not run cargo fmt concurrently
with source edits or compilation when it might change another agent's files.

Examples: `examples/orthogonal-demo.png` is actual Rust raster output, byte-identical
PPM to native within the subset. `examples/text-position.*` shows 36 source-code
placements from two builtin faces; no newline/reading order is invented.

## Environment and overnight continuation

Normal tools: `/Users/owner/.cargo/bin`, Rust 1.98.1. Commands from engine/:
`cargo fmt --check`, `cargo test --offline --locked --all-features`,
`cargo clippy --offline --locked --all-targets --all-features -- -D warnings`.
Minimum toolchain is isolated at `RUSTUP_HOME=/private/tmp/tandem-rust-msrv`,
`cargo +1.85.1`; targets `/private/tmp/tandem-engine-msrv-target` and
`/private/tmp/tandem-engine-wasm-target`. User's default toolchain is unchanged.

Pinned MuPDF 1.28.1 archive: `/private/tmp/tandem-mupdf-1.28.1-source.tar.gz`,
SHA-256 `dc94c60b2537e2ac9a2d379dd3801545f84a3a302d15c9da358362a1270707c3`.
Native source/build/oracles: `/private/tmp/tandem-mupdf-reference/`.
WASM oracle: `/Users/owner/Documents/TPDF/node_modules/mupdf/dist/mupdf.js`.
No worktree node_modules. Native code is test-only. Source hashes/notices are
in reference/PROVENANCE.md, NOTICE and reference/licenses/.

Heartbeat `rust-pdf-engine-overnight` already runs every 30 minutes in this chat;
do not create a duplicate. Its config is under the user's Codex automations
folder. Time-limited `caffeinate -i -t 32700` session50790 expires near the deadline.
At **13:00 UTC**, stop implementation, checkpoint/report and pause the heartbeat
while preserving its fields. Do not claim a complete MuPDF replacement.

## Active implementation update — 07:04 UTC

CONTRACT now authorizes two separately gated units:

- R14 C0 standalone mixed path/text coordinator first. Lexer owns program.rs and
  minimal path/text bridge changes; root CLI --program=FONTSFILE is written but
  not linked until module lands. Reference model/native gate already passes220
  programs /5,280 ordered model events /3,960 native semantic events /2,200 codes.
  Native C0 oracle has42 passing adapter tests. Root will rerun P0/T0 regressions
  after the P0 full-request allocation and global-index bridge changes. Actual
  document session/page execution remains a later unit;543-page ceiling is not
  an acceptance claim.
- R15 O0 independent simple TrueType outline unit. Parser owns truetype_outline.rs,
  owner tests and bounded native FreeType oracle. Root integration/structural
  review; reference independent model/review follows C0. Approved design is
  TRUETYPE-OUTLINE-DESIGN.md. No dependency, composites, hinting, glyph selection,
  curve raster or runtime font assets. Strict policy adjustments require recorded
  evidence and root decision, already authorized by the user.

R13 checkpoint remains the last accepted snapshot; later active source is not
covered by its470-test acceptance. Do not claim active modules passed yet.

C0 preliminary update — 07:06 UTC: module and --program=FONTSFILE are linked,
cargo check and smoke pass. Independent comparison passes all220 programs with
exact raw arenas, contexts and global provenance, plus native semantic comparison.
Reference found late TJ preflight failure undercharging traversed work; lexer is
fixing it and adding tests. Normal CLI is free, but final rebuild/gate is pending.
No C0 acceptance claim yet. O0 implementation is active in parallel.

## 07:23 UTC continuation

R14 C0 accepted: 220 mixed programs, 5,280 modeled events, 3,960 native
semantic events, 2,200 codes and 26 atomic refusals. Refreshed P0/T0 gates pass
on the same frozen program-gate binary; see REVIEW-R14-C0.md. The 492-test
compiler gate predates linking O0. R13 remains the latest archived checkpoint.

R15 O0 is active: all 1,885 fixture glyph slots compare with unhinted FreeType;
independent generated-font and budget review is underway. Root has linked the
outline CLI, so current source is ahead of the accepted compiler gate.

R16 PP1 is authorized and assigned to lexer: bounded document session adapter,
used-font preparation, page geometry, persistent mixed interpreter and exact
source metadata. Root owns CLI/native corpus; reference reviews after O0.

## 07:29 UTC O0 accepted

R15 full normal/MSRV gates now pass 513 tests, formatting, all-target/all-feature
Clippy, and minimum-toolchain wasm check. Independent review accepted 220 fonts,
1,415 glyphs, 26,450 points, 24,733 exact commands; all five fixture fonts/1,885
slots and 109 atomic refusals. See REVIEW-R15-O0.md. No open O0 findings.
PP1 remains active/unaccepted. Parser owns a separate native page oracle;
reference owns independent PP1 review; root owns full/compact CLI/corpus driver.
The archive checkpoint may contain unlinked PP1 drafts; the acceptance claim
applies to the linked O0 source and recorded gates, not unfinished PP1 files.
