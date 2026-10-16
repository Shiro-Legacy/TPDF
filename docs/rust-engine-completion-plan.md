# Rust PDF engine: what is left

This file is the single list of remaining work to replace MuPDF with the Rust engine. Updated October 14, 2026, late night. It replaces the older roadmap, plan and dated status files, which were deleted on October 14 (see git history, commit 79d295a). Their remaining items are folded in below.

- Per-operation status: [operation ledger](rust-engine-operation-ledger.csv). Check it with `python3 engine/tools/operation-ledger.py`.
- What has been built and its evidence: [engine/WORKFLOW-STATUS.md](../engine/WORKFLOW-STATUS.md).
- Live source: `engine/` in this workspace. The Codex worktree copy is stale.

## When it is done

The rewrite is complete only when all three of these hold:

1. Every ledger row is **accepted**: its complete named scope is done and independently reviewed. The alternative is an explicitly agreed exclusion that matches the app's own behavior.
2. Every app workflow runs on Rust (WASM in the app), with no runtime delegation to MuPDF and no silent fallback.
3. The M10 release evidence is complete.

A passing small corpus or the SBI document alone does not satisfy this. The target is the behavior of the app's `SessionRequest` contract in [engine.ts](../src/renderer/engine.ts), not every optional PDF feature. App restrictions stay legitimate restrictions; a new refusal where the app works is a gap.

Rust owns PDF interpretation, mutation and serialization. The TypeScript interface, Electron shell, dialogs, safe filesystem writes and external signing/verification cryptography stay where they work, behind tested interfaces.

## Where it stands

| | |
| --- | --- |
| Ledger (67 operations) | **0 accepted**. 15 verified subset: open, render, close, page-ids, save, validate, saved, undo, redo, edit-targets, edit-text, links, fields, authenticate, security-set. 1 implementing: layers. 51 planned. The ledger's other rows lag the app engine (`engine/APP-ENGINE-DESIGN.md`); October 16: the independent session review (rounds 0 and 1) accepted close, page-ids, authenticate and security-set and sent open back twice (`engine/design/app-accept-session.md`). |
| Fixtures (54) | 53 open, 51 fully render at 72 DPI, 18 editable. All 18 pass edit, save, fresh reopen and the independent MuPDF check. |
| Tests | 897 pass on Rust 1.98.1 and 1.85.1; fmt and strict Clippy pass; the WASM build and desktop smoke pass. |
| Last independent review | R24 (October 6). Everything since then is tested but not independently reviewed. |
| SBI05131482.pdf | Opens and renders natively and in WASM. Text editing is refused: its fonts are display substitutes. |
| App integration | On the `rust-engine-app` branch the app's worker runs the Rust engine by default (`scripts/build.mjs`; `--engine=mupdf` builds the MuPDF worker): `engine/src/app` behind `engine/bridge` (WASM) and `src/renderer/engine-rust.ts`. `node tests/run-rust.mjs` runs the app unit tests on it. |

## Remaining work by milestone

Order is by dependency. Implement what a blocked document needs, then immediately exercise its full workflow. A new refusal counts as diagnosed work, not a finished feature.

### M0: scope and evidence

- [ ] Map legacy test outcomes to every ledger row (currently "pending M0 legacy outcome mapping").
- [ ] Record the full desktop drive-suite baseline on the current app.
- [ ] Map tests for insert-pages, labels and clip-paste (the three rows the ledger tool reports as lacking literal test evidence; page-ids has `tests/p7-page-ids.test.mjs`).
- [ ] Add independently sourced document families beyond the 54 fixtures, with provenance and privacy handling. Keep the 500-page stress file as a performance case, not 500 compatibility wins.
- [ ] Independent review of all work since R24: workflow, session, writer, M1 rendering, encryption and M2.

### M1: viewing (ledger: open, render, close, page-ids, thumbnail)

- [ ] `thumbnail`.
- [ ] redact.pdf: its stream-valued feature.
- [ ] large.pdf: the 64-page preview limit (see M10 limits).
- [ ] edit-tj.pdf: a deliberately unbalanced `q`. Decide the policy against the app and record it.
- [ ] Stable page identity suitable for page moves and deletion, plus cache invalidation.

### M2: transactions and writer

Done: allocation, replacement, validated deletion, reachable-object saves, delta history, and copy-on-write for shared page streams and nested Form chains with in-place private Forms. Left:

- [ ] Copy-on-write for fonts, images and other resources once an edit changes them (text edits do not yet).
- [ ] Editing through annotation appearances, and text edits on pages that draw annotations (currently not editable).
- [ ] Save-in-place through the app's safe-write, recovery and acknowledgement flow (Save As works).
- [ ] Documented handling of repaired, linearized and incremental inputs.
- [ ] Recheck the cost of undo/redo replay on large documents under the M10 budgets (about 37 ms for 12.6 MB today).

### M3: practical text editing (ledger: edit-paragraph, text-add, edit-replace-preview, edit-replace-apply; finish edit-targets, edit-text)

- [ ] Paragraph edits, new text, and replace preview/apply with the app's semantics.
- [ ] Two-byte (CID/Type0) text editing.
- [ ] Font embedding, or another proven portable representation, for text drawn with substitute fonts. The SBI file needs this.
- [ ] Structure-aware edits: maintain ActualText and the structure tree. Pages with BDC/DP property lists are currently view-only.
- [ ] Line boundaries, spacing, rotation, different replacement lengths and overflow without moving unrelated content.
- [ ] Exit gate: the content, subset, coded, copy-on-write, paragraph and replacement drive cases pass, plus one meaningful edit of the SBI file. Each must go through edit, undo, redo, save, close, fresh reopen, extract/render and edit again.

### M4: rendering and extraction (ledger: search, text-all; compare-render/compare-text with M7)

- [ ] Structured text (lines, spans, quads, stable identities) for selection, search, extraction and compare. Handle ligatures, marked content and ActualText. Keep extraction Unicode separate from glyph mapping.
- [ ] Fonts: vertical writing (Identity-V), CMaps other than Identity-H, CFF-based CIDFontType0, embedded CFF/Type1 simple fonts, and a stated hinting policy for small sizes.
- [ ] Color: Indexed, Separation, DeviceN, Lab and Pattern colors, shadings, and color management (Cal/ICC are drawn as device values today).
- [ ] Transparency: blend modes, group transparency and partial-alpha soft masks (only proved-opaque masks are accepted).
- [ ] Per-feature visual tolerances against MuPDF, recorded before acceptance. MuPDF is an oracle, not the spec: where viewers differ, choose the better behavior and record the difference.

### M5: pages and navigation (16 ledger rows)

- [ ] page-rotate, page-delete, page-move, insert-pages, extract-pages; blank and image document creation.
- [ ] outline, outline-add, outline-set, outline-delete; link-add, link-set, link-delete; finish links.
- [ ] labels, labels-set, labels-clear.
- [ ] layer-set; finish layers. Layered documents are currently view-only.
- [ ] Remap destinations, bookmarks, links, labels, widgets and fields when pages move between documents. Preserve inherited geometry and cross-document references.

### M6: annotations and forms (9 ledger rows; finish fields)

- [ ] annots, annot-add, annot-set, annot-move, annot-delete for the app's types: Text, Highlight, Underline, StrikeOut, FreeText, Square, Circle, Line, Ink, Stamp.
- [ ] field-set, field-add, field-props, field-delete. This covers AcroForm inheritance, values, options, defaults, flags, tab order and shared radio groups across pages.
- [ ] Generate and save appearances. Generated appearances are display-only today; push buttons and other annotation types are refused. Form documents are currently view-only.
- [ ] Empty signature fields are distinct from signing. PDF JavaScript stays disabled. Do not expand XFA beyond the app.

### M7: object editing and utilities (12 ledger rows)

- [ ] edit-object, edit-objects, edit-batch.
- [ ] clip-copy, clip-paste.
- [ ] stamp, pdf-from-images, batch-file.
- [ ] compare-open, compare-close, compare-render, compare-text.
- [ ] Each one is atomic and uses the same transaction, permission and save rules as direct edits, including refusal partway through a batch.

### M8: security and permanent removal (8 rows; finish authenticate)

Each package is a separate acceptance with adversarial and interoperability gates.

- [ ] **Encryption:** security-set, encrypted saves, password change/removal, permissions on every affected operation, and named crypt filters and public-key handlers if the app needs them. Reading RC4/AES with a password works; encrypted documents are view-only.
- [ ] **Redaction:** redact-add, redact-set, redact-search, redact-preview, redact-apply. This means true text, image and vector removal, shared-resource isolation and configured sanitization. Inspection must prove the removal, including old revisions and unreachable objects; painting a rectangle is not redaction.
- [ ] **Signatures:** sign-list, sign-prepare. This needs exact ByteRange/Contents reservation, incremental saves that preserve signed prefixes, multi-signature and certification restrictions, and independent verification. Signed documents stay non-writable until then.

### M9: replace the app's engine

- [ ] Production adapter on the real `SessionRequest` union: reply types, errors, buffers, generation/revision identity, compare and scratch sessions, cleanup and save tokens.
- [ ] Run every app unit and drive test with the Rust backend, including failures and UI controls. Also test CSP, sandboxing, no Node globals, close/reopen lifetime and worker crashes.
- [ ] Recovery for long operations. The current 60-second termination loses unsaved edits.
- [ ] File dialogs, Save As filename adoption, save-in-place, printing, export and packaging paths.
- [ ] Prove the production build works with MuPDF runtime assets absent. MuPDF stays as a test-only oracle.

### M10: release

- [ ] Fixed benchmark set on named machines: open and first page, zoom, render, thumbnail, search, edit, save, reopen, peak memory, retained history and session growth. Record warm, cold and tail results, then freeze the budgets.
- [ ] Replace the preview limits (16 MiB, 64 pages, 64 steps, 64 MiB history) with the app's limits (120 MiB input, 50 MiB output) without an unbounded full-document pipeline.
- [ ] Fuzz, mutation and malformed-input campaigns; differential render and semantic checks; save failure and recovery tests; the full drive suite.
- [ ] Independent review of everything since the last accepted checkpoint, with no unexplained crash, corruption, wrong render or data loss left.
- [ ] Real packaged builds on macOS ARM64, Windows x64 and Linux x64: WASM startup, fonts and assets, dialogs, filesystem recovery, printing, high-DPI, attribution and reproducible build instructions.

## Rules that stay in force

- No unsafe Rust. Every new dependency is reviewed and recorded in `engine/DEPENDENCIES.md`, `engine/NOTICE` and `engine/reference/licenses`. Keep the AGPL source-informed attribution and pinned reference provenance.
- Fixtures are immutable. Durable evidence goes in versioned `engine/reference/` directories, never in `.stage` (deleted by packaging) or `.quad/backup`.
- Every package must:
  - pass the full test suite on both toolchains, plus fmt and strict Clippy for core and bridge;
  - show no render-hash regressions (`tools/workflow-view-corpus.py`);
  - pass edit, save and reopen with the independent oracle (`tools/workflow-edit-corpus.py` and `tools/workflow-corpus-oracle.mjs`);
  - pass the WASM build (`tools/build-workflow.sh`) and the desktop smoke.

  Afterwards, update `engine/WORKFLOW-STATUS.md`, `engine/STATUS.md`, this file and the ledger.
- The WASM build needs wasm-bindgen-cli 0.2.129: `cargo install wasm-bindgen-cli --version 0.2.129 --locked --root <dir>`, then set `WASM_BINDGEN_BIN`. It is no longer in `.stage`.
- Ledger statuses are planned, implementing, verified subset and accepted. Never infer a percentage complete from test counts, milestones or the stress document's page count. Calendar dates are not committed; re-estimate from measured packages.
- No commit, push, publication or installed-app replacement is implied by this plan.
