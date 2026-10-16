# Rust PDF engine: what is left

This file is the single list of remaining work to replace MuPDF with the Rust engine. Updated October 16, 2026, after the independent acceptance review. It replaces the older roadmap, plan and dated status files, which were deleted on October 14 (see git history, commit db942f5 on `rust-engine-app`; 79d295a before a rebase). Their remaining items are folded in below.

- Per-operation status: [operation ledger](rust-engine-operation-ledger.csv). Check it with `python3 engine/tools/operation-ledger.py`.
- What has been built and its evidence: [engine/WORKFLOW-STATUS.md](../engine/WORKFLOW-STATUS.md).
- Live source: `engine/` in this workspace, branch `rust-engine-app`. The Codex worktree copy is stale.

## When it is done

The rewrite is complete only when all three of these hold:

1. Every ledger row is **accepted**: its complete named scope is done and independently reviewed. The alternative is an explicitly agreed exclusion that matches the app's own behavior.
2. Every app workflow runs on Rust (WASM in the app), with no runtime delegation to MuPDF and no silent fallback.
3. The M10 release evidence is complete.

A passing small corpus or the SBI document alone does not satisfy this. The target is the behavior of the app's `SessionRequest` contract in [engine.ts](../src/renderer/engine.ts), not every optional PDF feature. App restrictions stay legitimate restrictions; a new refusal where the app works is a gap.

Rust owns PDF interpretation, mutation and serialization. The TypeScript interface, Electron shell, dialogs, safe filesystem writes and external signing/verification cryptography stay where they work, behind tested interfaces.

## Where it stands

None of the three criteria holds yet. The owner's instruction is to stop after the engine's acceptance; everything below that is still open waits for the owner's decision.

**1. Ledger rows: 64 of 67 accepted.** The independent acceptance review (workflow run wf_47a8f38e-1a3, October 16) checked every row against engine.ts in 12 groups, with up to two fix rounds per group, each re-verified. 41 rows were accepted at the first review and 64 after the fixes. The ledger records each verdict. Findings and decisions per group: `engine/design/app-accept-<group>.md` (view, pages and compare were accepted at the first review and have none). Not accepted:

- `open`: the second fix made the repair lose the object after a dictionary missing its closing `>>`, even when the xref is intact, and the save writes the loss. Three majors remain, all older than the fixes: an intact xref is dropped when an object header is glued to a damaged object; an untyped intermediate page-tree node shows as one blank page hiding its pages; one damaged byte in `trailer` makes RC4 and AES-128 files unopenable where engine.ts opens them.
- `redact-preview` and `redact-apply`: the second fix records every clip path in a touched Form once per mark rect before any work bound, so memory grows as clips times marks (a 3,972-byte file took the preview from 140 to 973 MB) toward the 4 GB wasm32 limit, where the engine aborts and the open document is lost (extrapolated, not run). A major that predates the fixes: with many marks from a file, the shared-content check is quadratic (74-89 s on a 67 KB file, against 0.55 s for engine.ts).

**2. App workflows on Rust: desktop only.** `scripts/build.mjs` picks `--engine`, else `TPDF_ENGINE`, else `rust`, so `npm run build` and `npm start` use the Rust engine (`engine/src/app` behind `engine/bridge` and `src/renderer/engine-rust.ts`); `npm test` builds it and its Electron tests run Rust, but most of its unit tests import engine.ts; `npm run test:rust` runs those on Rust. A build carries one engine, with no runtime fallback. On that build no app test and no drive script failed, measured before the fixes (`engine/reference/app-acceptance-20261016/`). Not on Rust yet:

- Mobile: `mobile/package.json` builds the desktop app with `build:mupdf`, and `mobile/src/scan-worker.ts` scans with `vendor/mupdf.js`.
- Releases and CI: `npm run package` (what `.github/workflows/release.yml` runs) builds MuPDF, and the release ships the MuPDF source archive. `npm run package:rust` builds the Rust package, checked on darwin-arm64 only.

The integration check (not a ledger row) is not accepted. Blocker: a form inherits the fill pattern and Type3 font of the content that draws it, and the content walk does not count them, so decoded forms still chain past the 128 MiB held-content budget. 8 levels reach 1,157 MB of WASM from a 6,923-byte file; about 31 levels would reach the 4 GiB abort (extrapolated, not run). Its majors are the owner decisions in M9: recovery for long operations, and mobile. Minors: releases build MuPDF; refusing an image over 512 MiB grows WASM by about 1 GB that the worker keeps.

**3. M10 evidence: incomplete.** There is no fixed benchmark set, no fuzzing or mutation campaign beyond the reviews' own probes, and no packaged Rust build for Windows or Linux.

| | |
| --- | --- |
| Branch | `rust-engine-app` at 3d1a300: the evidence branch and all 10 fix branches merged. Nothing is pushed or published. |
| App suites | Acceptance evidence at d352fcd: Rust unit run 560 pass, 0 fail, 114 skip; full suite on the Rust build 666 pass, 0 fail, 8 skip; all 50 drive scripts pass on both builds. `page-ids` had no unit test then (`tests/p7-page-ids.test.mjs` came with the session fixes); `close`, `security-set`, `extract-pages`, `pdf-from-images`, `text-all` and `layer-set` have no drive script. |
| External corpus | 5,049 public PDFs (`engine/reference/corpus-20261015/`). At 767bb5e: 239 files on the failing list (723 at the October 15 baseline); no Rust trap, timeout, OOM or crash; Rust opens 4,949 files and MuPDF 4,923, and Rust refuses none that MuPDF opens. |
| Last independent review | The October 16 acceptance review above. |
| Preview Session | The separate preview (`engine/desktop`, the bespoke render pipeline), last measured October 14: 53 of 54 fixtures open, 51 render, 18 editable. Not re-measured since. `engine/src/app` does not use it. |
| SBI05131482.pdf | Opens and renders in the preview, natively and in WASM; text editing there is refused because its fonts are display substitutes. No app-engine result is recorded. |

Gates on the merged head:

All pass at `3d1a300` (2026-10-16; every step under memcap 3000 MB, 2 build jobs, 2 test threads; logs in `tpdf-wt/gates-out/final-3d1a300/`, not committed):
- `cargo fmt --check` and `cargo clippy -D warnings`, engine and bridge, on Rust 1.98.1 and 1.92.0.
- Native tests: 1,618 passed, 0 failed in 131 test binaries on 1.98.1, and the same on 1.92.0 (peak 1,521 MB).
- Bridge tests: 4 passed.
- The WASM package (`engine/tools/build-workflow.sh`) and `npm run build` (Rust build) both build.
- App unit suite without Electron: 681 tests, 568 passed, 0 failed, 113 skipped.
- App unit suite on Rust (`tests/run-rust.mjs`): 670 tests, 567 passed, 0 failed, 103 skipped.
- Full suite with Electron on the Rust build, including the 20 test files that run drive scripts (`node --test --test-concurrency=2 tests/*.test.mjs`): 681 tests, 670 passed, 0 failed, 11 skipped (peak 1,721 MB).

Open decisions for the owner: a third fix round for `open`, `redact-preview`, `redact-apply` and the integration blocker; the remaining M9 items; M10.

## Remaining work by milestone

Order is by dependency. Implement what a blocked document needs, then immediately exercise its full workflow. A new refusal counts as diagnosed work, not a finished feature.

The app engine renders and extracts text with hayro (`engine/APP-ENGINE-DESIGN.md`, decision 1). Items below that were written for the bespoke pipeline or the preview Session say so.

### M0: scope and evidence

- [x] Map legacy test outcomes to every ledger row: `engine/reference/app-acceptance-20261016/` (`op-evidence.json`; the ledger's `candidate_legacy_tests` and `remaining_verification` columns were rewritten from it).
- [x] Record the full desktop drive-suite baseline on the current app: all 50 drive scripts, on the Rust and MuPDF builds (same folder).
- [x] Map tests for insert-pages, labels and clip-paste: in `op-evidence.json` (insert-pages 5 unit tests and 2 drive scripts, labels 2 and 50, clip-paste 23 and 4).
- [x] Add independently sourced document families beyond the 54 fixtures, with provenance and privacy handling: 5,049 public PDFs from 7 sources (`engine/reference/corpus-20261015/`). Keep the 500-page stress file as a performance case, not 500 compatibility wins.
- [x] Independent review: the October 16 acceptance review covered all 67 ledger rows and the app integration. It did not cover the preview Session or the bespoke render pipeline, which the app engine does not use.
- [x] Record the review's verdicts in the ledger: its status, scope, evidence and remaining columns were rewritten from the results, 64 `accepted` and 3 `not accepted` (`engine/reference/app-acceptance-20261016/verdicts.json` has the rounds, branches and open findings per group).

### M1: viewing (ledger: open, render, close, page-ids, thumbnail)

Status: `render`, `close`, `page-ids` and `thumbnail` accepted. `open` not accepted (repair and page-walk findings above).

- [x] `thumbnail`.
- [x] redact.pdf, large.pdf and edit-tj.pdf, the bespoke pipeline's last fixture failures, render in the app engine: every page of redact.pdf and edit-tj.pdf (the unbalanced `q` included) is at least 99.4% within 16 levels of MuPDF (`engine/reference/app-view-20261014/summary.json`), and large.pdf (500 pages) opens and its pages render (APP-ENGINE-DESIGN.md, "Limits" and the view timings). The app limits have no page cap.
- [x] Stable page identity: `page-ids` accepted.
- [ ] `open`: fix the repair and page-walk findings, then re-review.

### M2: transactions and writer

Status: the app engine's journal, transactions and app save (`engine/APP-ENGINE-DESIGN.md`, "AppSession") carry `save`, `validate`, `saved`, `undo` and `redo`, all accepted. The preview Session's own work is done: allocation, replacement, validated deletion, reachable-object saves, delta history, and copy-on-write for shared page streams and nested Form chains with in-place private Forms. Left:

- [ ] Preview Session: copy-on-write for fonts, images and other resources once an edit changes them, and editing through annotation appearances. The app engine edits as engine.ts does (edit rows accepted); these items have not been re-checked against it.
- [x] Save-in-place through the app's safe-write, recovery and acknowledgement flow: the app's own save path runs on the Rust build, and its save-in-place and save-to drive scripts pass.
- [ ] Documented handling of repaired, linearized and incremental inputs: documented (APP-ENGINE-DESIGN.md, "Tolerant open" and the Differences), but the repair still has the findings that keep `open` from acceptance.
- [ ] Undo/redo cost on large documents under the M10 budgets. The app journal restores one step's object states and does not replay; the 37 ms replay for 12.6 MB is the preview Session's.

### M3: practical text editing (ledger: edit-paragraph, text-add, edit-replace-preview, edit-replace-apply; finish edit-targets, edit-text)

Status: all six rows accepted, after two fix rounds (`engine/design/app-accept-edit.md`).

- [x] Paragraph edits, new text, and replace preview/apply with the app's semantics.
- [x] Two-byte (Type0) runs with Identity-H or coded CMaps (`engine/design/app-edit.md`).
- [ ] Font embedding, or another proven portable representation, for text drawn with substitute fonts. The SBI file needs this.
- [ ] Structure-aware edits that maintain ActualText and the structure tree, beyond engine.ts. The app engine's handling of runs under ActualText is in `engine/design/app-edit.md`.
- [ ] Line boundaries, spacing, rotation, different replacement lengths and overflow without moving unrelated content, beyond what engine.ts does.
- [ ] Exit gate: the drive scripts, the edit cases among them, pass on the Rust build. One meaningful edit of the SBI file is not shown.

### M4: rendering and extraction (ledger: search, text-all; compare-render/compare-text with M7)

Status: `search`, `text-all`, `compare-render` and `compare-text` accepted at the first review.

- [x] Structured text (lines, spans, quads, identities) for selection, search, extraction and compare, with ligatures, marked content and ActualText (`engine/src/view/stext.rs`, `engine/design/app-corpus-text.md`).
- [ ] Fonts, color and transparency gaps: written for the bespoke pipeline. The app engine draws with hayro; the corpus differential measures it against MuPDF (`render-pixels` on 187 of 5,049 files at 767bb5e) and its README lists the remaining classes.
- [x] Visual tolerances against MuPDF, recorded: the corpus differential's content classes and thresholds (`engine/reference/corpus-20261015/README.md`). MuPDF is an oracle, not the spec: where viewers differ, choose the better behavior and record the difference.

### M5: pages and navigation (16 ledger rows)

Status: all accepted. The pages group was accepted at the first review; navigation after one fix round (`engine/design/app-accept-nav.md`); `layers` and `layer-set` with the view group.

- [x] page-rotate, page-delete, page-move, insert-pages, extract-pages; blank and image document creation.
- [x] outline, outline-add, outline-set, outline-delete; links, link-add, link-set, link-delete.
- [x] labels, labels-set, labels-clear.
- [x] layers, layer-set.
- [x] Destinations, bookmarks, links, labels, widgets and fields when pages move between documents (APP-ENGINE-DESIGN.md, "Pages and assembly" and the Differences).

### M6: annotations and forms (9 ledger rows; finish fields)

Status: all accepted after one fix round each (`engine/design/app-accept-annots.md`, `app-accept-forms.md`).

- [x] annots, annot-add, annot-set, annot-move, annot-delete for the app's types.
- [x] fields, field-set, field-add, field-props, field-delete.
- [x] Appearances generated and saved by the app engine.
- Rules that stay: empty signature fields are distinct from signing; PDF JavaScript stays disabled; XFA is not expanded beyond the app.

### M7: object editing and utilities (12 ledger rows)

Status: all accepted. Compare, stamp, pdf-from-images and batch-file at the first review; the object group after two fix rounds (`engine/design/app-accept-objects.md`).

- [x] edit-object, edit-objects, edit-batch.
- [x] clip-copy, clip-paste.
- [x] stamp, pdf-from-images, batch-file.
- [x] compare-open, compare-close, compare-render, compare-text.

### M8: security and permanent removal (8 rows; finish authenticate)

Status: `security-set`, `authenticate`, `redact-add`, `redact-set`, `redact-search`, `sign-list` and `sign-prepare` accepted. `redact-preview` and `redact-apply` not accepted.

- [x] **Encryption:** security-set and encrypted saves (keep, remove, or AES-256 R6), as engine.ts. Public-key handlers and named non-Identity crypt filters are refused (`engine/src/security.rs`); add them only if the app needs them.
- [ ] **Redaction:** redact-preview and redact-apply. Bound the clip entries in the shared-content check by its work budget, and make the region queries independent of duplicate and wide mark rects (`engine/design/app-accept-redact.md`). The removal itself held in the second review: no secret was left in any object or revision of its probe outputs, each one revision, and on a 174-file corpus sample no character was left inside a mark and no search hit remained.
- [x] **Signatures:** sign-list and sign-prepare, as engine.ts. Signed and certified documents stay read-only, as in engine.ts.

### M9: replace the app's engine

- [x] Production adapter on the real `SessionRequest` union: `src/renderer/engine-rust.ts` (APP-ENGINE-DESIGN.md, "Wire format").
- [x] Run every app unit and drive test with the Rust backend: no failure (`engine/reference/app-acceptance-20261016/`). Re-run at 82c8693 by the integration review: `node tests/run-rust.mjs` 667 tests, 564 pass, 0 fail, 103 skip; the full suite with Electron 678 tests, 667 pass, 0 fail, 11 skip.
- [ ] Integration blocker: inherited pattern and Type3 chains past the held-content budget (above; `engine/design/app-accept-integration.md`). Owner's decision on a third fix round.
- [ ] Recovery for long operations. The 20-second watchdog on ordinary requests restarts the worker and loses unsaved edits (saves and other long jobs are only rejected after 120 s, thumbnails after 10 s) (proposal: `engine/design/app-accept-integration.md`).
- [ ] Mobile (`mobile/`) still builds the desktop app with MuPDF and scans with `vendor/mupdf.js`: move it to the Rust engine (scanning needs a page-from-images path, verified on a device) or record an agreed exclusion. Owner's decision.
- [ ] Release engine switch: `npm run package` and `release.yml` build and ship MuPDF. Switching needs the Rust toolchain and `engine/tools/build-workflow.sh` in `release.yml`. Owner's decision.
- [ ] Licence wording: the `NOTICE.txt` and About panel text of a Rust build (`engine/design/app-worker.md`), for the owner to confirm before a Rust release.
- [ ] File dialogs, Save As filename adoption, save-in-place, printing, export and packaging paths. The drive scripts that save to a new file, save in place and print to PDF pass on the Rust build. Exporting text (`text-all`) has no drive script. Packaging is checked on darwin-arm64 only.
- [x] The production build works with MuPDF runtime assets absent, on darwin-arm64: the `npm run package:rust` app holds no MuPDF file, and its packaged smoke passes. MuPDF stays a test-only oracle there. Windows and Linux are M10.

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
- Ledger statuses are planned, implementing, verified subset, accepted and not accepted (reviewed, with findings still open). Never infer a percentage complete from test counts, milestones or the stress document's page count. Calendar dates are not committed; re-estimate from measured packages.
- No commit, push, publication or installed-app replacement is implied by this plan.
