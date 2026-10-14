October 14, 2026 (late night, later): a Form drawn and named only once (including earlier edit copies) is edited in place, so repeated Form-text edits no longer grow the file (edit-shared.pdf stays 1.8 KB over 20 edits instead of 7.2 KB). Render hashes unchanged, all edits independently verified, 897 tests on both toolchains. See WORKFLOW-STATUS.md.

October 14, 2026 (late night): M2 remaining items. Object deletion with reference validation, reachable-object collection on save (orphans from copy-on-write and deletion are not retained; existing signature/encryption refusals unchanged), delta history instead of whole-file snapshots (a 12.6 MB document retains 25 MB after 20 edits instead of 264 MB), and copy-on-write for Forms nested in Forms. Still 53/51/18 fixtures open/render/edit, all render hashes unchanged, all edits independently verified; 895 tests pass on both toolchains. See WORKFLOW-STATUS.md.

October 14, 2026 (night): M2 first increment, copy-on-write editing of shared page streams and Form placements through new object allocation in the writer. 18 fixtures are editable, and all pass the independent edit check; 887 tests pass on both toolchains.

October 14, 2026 (evening): encryption reading (RC4/AES-128/AES-256 with a password, view-only), default-state layers, inline images, device-equivalent cs/CS color spaces and display-only generated appearances (text/choice/button/signature widgets, Text notes, NeedAppearances). 53 of 54 fixtures open, 51 fully render, 14 are editable, and 886 tests pass on both toolchains. See WORKFLOW-STATUS.md.

# Current functional workflow

October 14, 2026 (later): annotation and widget appearance streams render after page content, and marked-content property lists (BDC/DP) render. Form and XFA documents open view-only. Identity-H CID fonts and composite TrueType glyphs render too. 36 of 54 fixtures now fully render, 13 are editable, and 872 tests pass on both toolchains. Earlier the same day, M1 rendering work makes the user's SBI05131482.pdf open and render in native Rust and in WASM. This adds CalRGB images, standard image intents, tag-only marked content, dashed strokes and bounded clip masks. 863 tests pass on both toolchains. The live source is this workspace's `engine/`; the Codex worktree copy is stale. See WORKFLOW-STATUS.md.

October 12, 2026: the user resumed the rewrite. A native and Rust WASM text-edit, undo/redo, disk-save and fresh-reopen workflow is implemented and tested in a separate desktop preview. See WORKFLOW-STATUS.md for current evidence and ../docs/rust-engine-completion-plan.md for remaining work. Full application replacement remains incomplete. The R24-only stop instruction below is historical and superseded by the new user instruction; R24 remains the last independently reviewed acceptance checkpoint.

# Rust engine status

Updated 2026-10-06. **R24 validated fully opaque image soft masks are accepted after independent review.** The full Rust rebuild remains incomplete and isolated from the installed Electron/MuPDF application.

R24 admits one authenticated, matching-size Gray8 image SMask only when every effective alpha sample is 255. Normal and inverted Decode are supported within that proof. Retained assets own the mask identity and lifetime; document-local role checks, complete scan reservations, cumulative page/session budgets and atomic dependent publication are enforced. Partial alpha and broader mask semantics still refuse. See R24-IMPLEMENTATION.md and R24-INDEPENDENT-REVIEW.md.

Verification: 814 tests pass on Rust 1.98.1 and minimum 1.85.1; formatting, strict all-target/all-feature Clippy, release binaries/examples and minimum-toolchain wasm32 compilation pass. All 44 generated positive masks and 12 negative controls pass. The 634-attempt corpus replay preserves all 617 R23 successful hashes and certifies two new renders of p3/edit-objects.pdf at 18 and 72 DPI. At 18 DPI, 558/572 pages render and 14 refuse; at 72 DPI, 61/62 supplementary attempts render. redact.pdf passes mask loading but still refuses later marked content. All 63,240 original alpha samples have source/native/Rust opacity proofs.

Independent review found two acceptance-verifier defects, both resolved: required raw artifacts were optional and identity/source manifests could be incomplete. Exact schemas and checked executable/source bindings now prevent those omissions. No production Rust fix was needed. Five independent Rust regressions pass; 407 independent mutations and 23 built-in mutations are rejected after clean positive controls. The reviewer reports no remaining R24 blocker. All 278 frozen R23 reference/tool files remain unchanged.

Evidence: reference/soft-mask-acceptance-r24.json, soft-mask-differential-r24.json, soft-mask-rust-checks-r24.json, soft-mask-independent-*-r24.json and soft-mask-milestone-r24.json. Prior R23 acceptance was recorded in STATUS-R23-ACCEPTED-20261006.md (removed October 14 with the other dated status snapshots; see git history).

Recovery checkpoint: none. The `.quad/backup/rust-r2x-*` checkpoints (R22, R23, R24 pre-review and accepted) were deleted in the 2026-10-14 disk housekeeping. `engine/`, including `reference/`, has been tracked in git since commit 79d295a.

The current request covers R24 and its independent review only. Do not start another milestone without a new instruction. Recurring automation remains paused. No application/fixture change, commit, push, publication, installed-app replacement, runtime dependency or unsafe code.
