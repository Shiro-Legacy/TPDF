# Rust PDF engine: what is left

> Licence note (October 18, 2026): the Rust engine and the app are now MIT, and no MuPDF code ships. Where this document says engine code is AGPL or MuPDF-derived, or that a build owes AGPL obligations, it records the state before the clean-room relicense (see the Licence section of [`PROVENANCE.md`](../engine/reference/PROVENANCE.md)).

This file is the single list of remaining work to replace MuPDF with the Rust engine. Updated October 17, 2026, after fix round 3, M9 recovery and the M10 work: the save-failure tests and fuzz campaign, the macOS ARM64 package checklist, the final independent review and its fixes, and the benchmarks (the independent acceptance review was October 16). Later that day the owner merged mobile and the release switch, and the scanner moved to the Rust engine. It replaces the older roadmap, plan and dated status files, which were deleted on October 14 (see git history, commit d9883e0 on `rust-engine-app`; 79d295a before a rebase). Their remaining items are folded in below.

- Per-operation status: [operation ledger](rust-engine-operation-ledger.csv). Check it with `python3 engine/tools/operation-ledger.py`.
- What has been built and its evidence: [engine/WORKFLOW-STATUS.md](../engine/WORKFLOW-STATUS.md).
- Live source: `engine/` in this workspace, branch `rust-engine-app`. The Codex worktree copy is stale.

## When it is done

The rewrite is complete only when all three of these hold:

1. Every ledger row is **accepted**: its complete named scope is done and independently reviewed. The alternative is an explicitly agreed exclusion that matches the app's own behavior. **Holds since October 17: 69 of 69 rows accepted** (the 67 of the October 16 contract, and the scanner's `scan-compose` and `scan-measure`).
2. Every app workflow runs on Rust (WASM in the app), with no runtime delegation to MuPDF and no silent fallback. **Holds in the build, for desktop and mobile, since October 17; device verification is pending.** Mobile has not run on an iOS or Android device, and no GitHub Action has run the release workflow.
3. The M10 release evidence is complete. **Does not hold: the save-failure tests and fuzz campaign (to their stated scope), the benchmarks (on one named machine), the final independent review and the macOS ARM64 package checklist are done; the Windows x64 and Linux x64 packages wait for the owner (they need CI, so a push), and the full corpus differential ran on ba70615, before the mobile and scanner merges (acceptance report 8.10).**

A passing small corpus or the private-sample document alone does not satisfy this. The target is the behavior of the app's `SessionRequest` contract in [engine.ts](../src/renderer/engine.ts), not every optional PDF feature. App restrictions stay legitimate restrictions; a new refusal where the app works is a gap.

Rust owns PDF interpretation, mutation and serialization. The TypeScript interface, Electron shell, dialogs, safe filesystem writes and external signing/verification cryptography stay where they work, behind tested interfaces.

## Where it stands

Criterion 1 holds, criterion 2 holds in the build (device verification is pending), and criterion 3 does not. The owner asked for a third fix round, M9 recovery and the M10 work after the October 16 review, then decided on October 17 that mobile runs fully on Rust; that work is merged locally, at 3d8a6ff. What is left waits for the owner: the acceptance report ([rust-engine-acceptance-report.md](rust-engine-acceptance-report.md)) has the complete list in section 6.7 and the detail in section 8.

**1. Ledger rows: 69 of 69 accepted.** The independent acceptance review (workflow run wf_47a8f38e-1a3, October 16) checked every row against engine.ts in 12 groups, with up to two fix rounds per group, each re-verified: 41 rows were accepted at the first review and 64 after the fixes. A third fix round (October 16-17) closed the rest: `open` was accepted in round 3 (`app-p8-fix-open`, 94be413), and `redact-preview` and `redact-apply` at the third re-review of round 3 (`app-p8-fix-redact`, 608fd9d). The ledger records each verdict. Findings and decisions per group: `engine/design/app-accept-<group>.md` (view, pages and compare were accepted at the first review and have none). Of the 30 minors on accepted ops, 3 are still open (`render`, `annot-delete`, `edit-replace-preview`); the others are resolved or recorded (`engine/reference/app-acceptance-20261016/verdicts.json`). The scanner port added `scan-compose` and `scan-measure`, accepted by an independent review with three rechecks at 8ce01d7 (`app-p11-rust-scan`); two cosmetic notes remain (a trailing U+200F after Arabic that ends in a marked letter, and combining marks no font has are not drawn).

**2. App workflows on Rust: desktop and mobile, in the build.** `scripts/build.mjs` picks `--engine`, else `TPDF_ENGINE`, else `rust`, so `npm run build` and `npm start` use the Rust engine (`engine/src/app` behind `engine/bridge` and `src/renderer/engine-rust.ts`); `npm test` builds it and its Electron tests run Rust, but most of its unit tests import engine.ts; `npm run test:rust` runs those on Rust. A build carries one engine, with no runtime fallback. On that build no app test and no drive script failed, measured before the fixes (`engine/reference/app-acceptance-20261016/`). A worker restart now restores the open document by replay (M9, `engine/design/app-m9-recovery.md`). On October 17 the owner decided that mobile can be fully Rust and merged:

- Mobile and releases (`app-p9-mobile-release`, tip 5a00457, merged at 92e7612): `mobile/package.json` builds the desktop app with Rust; `npm run package` builds only Rust (`package:rust` is an alias), with `build:mupdf` and `test:mupdf` kept for the MuPDF oracle; `release.yml` builds one reproducible WASM with pinned Rust and checks it against `engine/reference/wasm-release.sha256`; the NOTICE and About panel carry the Rust wording. Gates on 92e7612: all 12 pass (native 1,683 passed, 0 failed on 1.98.1 and 1.92; app suite 760 tests, 616 pass, 0 fail; app on Rust 749 tests, 623 pass, 0 fail).
- The scanner (`app-p11-rust-scan`, merged at 97fa6dc): it composes its PDF with the Rust engine through `scan-compose` and `scan-measure`, and the TypeScript PDF writer (`scan-pdf.ts`, `scan-font.ts`, `scan-metrics.ts`) is deleted. Rebuilt pages now draw Latin Extended-A, most of Greek and Slavic Cyrillic, and some maths, arrows and currency; Vietnamese, Hebrew and Arabic letters, Thai and emoji are still boxes, but extract and search. Right-to-left runs are written in visual order, so search works in MuPDF, Rust and PDFKit.
- Pending: device verification on iOS and Android, the owner's sign-off of the licence wording, and any CI run (no GitHub Action has run). Details: `engine/design/app-m9-mobile-release.md` and `engine/APP-ENGINE-DESIGN.md`, "Scanner".

The integration check (not a ledger row) is accepted at its round 3 re-review (`app-p8-fix-integration`, 8ca0c05): hayro now bounds the decoded content it holds along every inheritance path, and images too large to hold are drawn reduced. The three minors it left (wide-image row memory, the scene estimate for strokes, tests bound by timing) were fixed after acceptance on `app-p10-followups` (merged at 11bf970).

**3. M10 evidence: done except the Windows and Linux packages.**
- Save-failure tests for the app's save path and a campaign with the existing review fuzzers are done and accepted (`engine/design/m10-fuzz.md`), to the scope recorded there.
- The macOS ARM64 package passes every item of the M10 checklist, and the WASM builds reproducibly (`app-p10-pkg`; `engine/design/m10-macos-package.md`, `docs/building-rust-release.md`).
- The final independent review of 46f1e95..11bf970 accepted it with no blocker or major. Its five minors were fixed on `app-p10-final`, and the recheck's blocker and minor were fixed and accepted.
- The fixed benchmark set ran on one named machine (MacBookAir10,1), and its budgets are frozen (`engine/reference/bench-20261017/`).
- No packaged Rust build exists for Windows or Linux: that needs CI, so a push, and waits for the owner.

| | |
| --- | --- |
| Branch | `rust-engine-app` at 3d8a6ff: the October 16 evidence and fix branches, then `app-p8-fix-open`, `app-p8-fix-minors`, `app-p8-fix-redact`, `app-p9-m9-replay`, `app-p8-fix-integration`, `app-p10-fuzz`, `app-p10-followups`, `app-p10-pkg`, `app-p10-final`, `app-p10-bench`, `app-p10-verify`, `app-p9-mobile-release` (at 92e7612) and `app-p11-rust-scan` (at 97fa6dc) merged; 250 commits ahead of local `main`. Nothing is pushed or published. |
| App suites | Acceptance evidence at d352fcd: Rust unit run 560 pass, 0 fail, 114 skip; full suite on the Rust build 666 pass, 0 fail, 8 skip; all 50 drive scripts pass on both builds. On 39b0699 (gates): app unit suite 760 tests, 616 pass, 0 fail; on Rust 749 tests, 623 pass, 0 fail. The full suite with Electron, drive-script tests included, at 11bf970 (final review): 749 tests, 0 fail. `close`, `security-set`, `extract-pages`, `pdf-from-images`, `text-all` and `layer-set` had no drive script at d352fcd. |
| External corpus | 5,049 public PDFs (`engine/reference/corpus-20261015/`). At 4280502 (the integration follow-up): 239 files on the failing list (723 at the October 15 baseline; 238 at 62a9b21, where MuPDF's open of one file trapped); no Rust render, text or save error; Rust opens 4,952 files and MuPDF 4,923, and Rust refuses none that MuPDF opens. The final review ran a 603-file sample at 11bf970 with no new Rust error. The full run on ba70615 (acceptance report 8.10): 237 files on the failing list, none newly; no Rust trap, timeout or error. No full run after the mobile and scanner merges is recorded. |
| Last independent review | The final M10 review of 46f1e95..11bf970 (October 17): accepted, no blocker or major; its five minors fixed on `app-p10-final`, and the recheck of those fixes accepted (acceptance report section 8.8). `app-p10-pkg` and `app-p10-bench` came after it; no review of them is recorded. |
| Preview Session | The separate preview (`engine/desktop`, the bespoke render pipeline), last measured October 14: 53 of 54 fixtures open, 51 render, 18 editable. Not re-measured since. `engine/src/app` does not use it. |
| private-sample.pdf | Opens and renders in the preview, natively and in WASM; text editing there is refused because its fonts are display substitutes. No app-engine result is recorded. |

Gates on the merged head:

All 12 gates of `tools/gates.sh` pass at `9ef8f49`, the merged head before `app-p10-fuzz` (2026-10-17; under memcap 3000 MB; logs in `tpdf-wt/gates-out/merged-9ef8f49/`, not committed):
- `cargo fmt --check` and `cargo clippy -D warnings`, engine and bridge, on Rust 1.98.1 and 1.92.
- Native tests: 1,664 passed, 0 failed on 1.98.1 and on 1.92.
- Bridge tests: 4 passed.
- The WASM package and `npm run build` both build.
- App unit suite without Electron: 728 tests, 593 passed, 0 failed, 135 skipped.
- App unit suite on Rust (`tests/run-rust.mjs`): 717 tests, 598 passed, 0 failed, 119 skipped.
- Peak 1,957 MB.
- The gate script now fails with a failing app suite and names every failing test; before `app-p10-followups` it piped each app suite into `tail`, whose exit status was the line's. The counts above were read from the logs' `# fail 0` lines, so they stand.
- **Gates on 11bf970, the head the final review covered: all 12 pass** (October 17, 2026; every step under memcap 3000 MB, 2 build jobs, 2 test threads, with the fixed gate script; logs in `tpdf-wt/gates-out/final-11bf970/`, not committed). fmt and strict clippy, engine and bridge, on Rust 1.98.1 and 1.92.0; native tests 1,670 passed, 0 failed on both toolchains; bridge tests; the WASM package and `npm run build`; app unit suite 749 tests, 609 passed, 0 failed, 140 skipped; app suite on Rust 738 tests, 614 passed, 0 failed, 124 skipped; peak 1,987 MB. The docs merge 525ad90 changes `vendor/hayro-interpret-0.8.0/PATCH.md` prose, which the WASM source hash covers, so the package was rebuilt there (sha256 `6e900969…`), `npm run build` passed and the app suite on Rust passed again (738 tests, 614 passed, 0 failed; logs in `gates-out/post-docs-525ad90/`). The final review ran the full suite with Electron on 11bf970: 749 tests, 0 failed.
- **Gates on 39b0699, all 12 pass** (`app-p10-pkg` and `app-p10-final` merged; October 17, 2026; under memcap 3000 MB; logs in `tpdf-wt/gates-out/final-39b0699/`, not committed). fmt and strict clippy, engine and bridge, on Rust 1.98.1 and 1.92; native tests 1,683 passed, 0 failed on both toolchains; bridge tests 4 passed; the WASM package (sha256 `3c18b0e8cb8dcf868fa13c4aacb4f5a75e29efe3744b353245c298e94783de93`) and `npm run build`; app unit suite 760 tests, 616 passed, 0 failed, 144 skipped; app suite on Rust 749 tests, 623 passed, 0 failed, 126 skipped; peak 1,977 MB. The head b6bc7b9 adds only the benchmark tools and reference, outside the WASM's sources.
- **Gates on 92e7612** (the mobile merge): all 12 pass. Native tests 1,683 passed, 0 failed on 1.98.1 and 1.92; app unit suite 760 tests, 616 passed, 0 failed; app suite on Rust 749 tests, 623 passed, 0 failed.
- **Gates on 3d8a6ff (the scanner port merged): all 12 pass** (October 17, 2026; memcap 3000 MB; logs in `tpdf-wt/gates-out/final-scan/`, not committed). Native tests 1,700 passed, 0 failed on Rust 1.98.1 and 1.92; app unit suite 763 tests, 619 passed, 0 failed, 144 skipped; app suite on Rust 752 tests, 626 passed, 0 failed, 126 skipped; peak 1,920 MB. Mobile `npm test`: 16 passed, 0 failed. The WASM is `2de5d021c63352915f50d72890d9bc7bc824f7065506f398d1129217845f0d40` (13,614,507 bytes), reproduced by a clean build at a second checkout path (`tpdf-wt/p11-docs`); `engine/reference/wasm-release.sha256` records it.

Open for the owner (the acceptance report's section 6.7 is the complete list): the licence wording sign-off; the mobile notice (AGPL section 6); corresponding source; scanner fonts for the remaining scripts (a new font file); the scanner's output differences; the package follow-ups; the NOTICE source tag; Electron fuses; the performance follow-ups the benchmarks show; device verification; the Windows and Linux packages, which need CI and so a push; pushing and merging `rust-engine-app` to `main`.

## Remaining work by milestone

Order is by dependency. Implement what a blocked document needs, then immediately exercise its full workflow. A new refusal counts as diagnosed work, not a finished feature.

The app engine renders and extracts text with hayro (`engine/APP-ENGINE-DESIGN.md`, decision 1). Items below that were written for the bespoke pipeline or the preview Session say so.

### M0: scope and evidence

- [x] Map legacy test outcomes to every ledger row: `engine/reference/app-acceptance-20261016/` (`op-evidence.json`; the ledger's `candidate_legacy_tests` and `remaining_verification` columns were rewritten from it).
- [x] Record the full desktop drive-suite baseline on the current app: all 50 drive scripts, on the Rust and MuPDF builds (same folder).
- [x] Map tests for insert-pages, labels and clip-paste: in `op-evidence.json` (insert-pages 5 unit tests and 2 drive scripts, labels 2 and 50, clip-paste 23 and 4).
- [x] Add independently sourced document families beyond the 54 fixtures, with provenance and privacy handling: 5,049 public PDFs from 7 sources (`engine/reference/corpus-20261015/`). Keep the 500-page stress file as a performance case, not 500 compatibility wins.
- [x] Independent review: the October 16 acceptance review covered all 67 ledger rows and the app integration. It did not cover the preview Session or the bespoke render pipeline, which the app engine does not use.
- [x] Record the review's verdicts in the ledger: its status, scope, evidence and remaining columns were rewritten from the results, 64 `accepted` and 3 `not accepted` (`engine/reference/app-acceptance-20261016/verdicts.json` has the rounds, branches and open findings per group). Updated October 17 after round 3: 67 `accepted`; `verdicts.json` gained a `round3` section.

### M1: viewing (ledger: open, render, close, page-ids, thumbnail)

Status: all five accepted. `open` was accepted in fix round 3 (`engine/design/app-accept-session.md`, "Round 3").

- [x] `thumbnail`.
- [x] redact.pdf, large.pdf and edit-tj.pdf, the bespoke pipeline's last fixture failures, render in the app engine: every page of redact.pdf and edit-tj.pdf (the unbalanced `q` included) is at least 99.4% within 16 levels of MuPDF (`engine/reference/app-view-20261014/summary.json`), and large.pdf (500 pages) opens and its pages render (APP-ENGINE-DESIGN.md, "Limits" and the view timings). The app limits have no page cap.
- [x] Stable page identity: `page-ids` accepted.
- [x] `open`: the repair and page-walk findings fixed in round 3 (`app-p8-fix-open`, 94be413) and re-reviewed; the M10 campaign fixed three more repair cases (`engine/design/m10-fuzz.md`). Two rare damage shapes the campaign found are open there.

### M2: transactions and writer

Status: the app engine's journal, transactions and app save (`engine/APP-ENGINE-DESIGN.md`, "AppSession") carry `save`, `validate`, `saved`, `undo` and `redo`, all accepted. The preview Session's own work is done: allocation, replacement, validated deletion, reachable-object saves, delta history, and copy-on-write for shared page streams and nested Form chains with in-place private Forms. Left:

- [ ] Preview Session: copy-on-write for fonts, images and other resources once an edit changes them, and editing through annotation appearances. The app engine edits as engine.ts does (edit rows accepted); these items have not been re-checked against it.
- [x] Save-in-place through the app's safe-write, recovery and acknowledgement flow: the app's own save path runs on the Rust build, and its save-in-place and save-to drive scripts pass. Since October 17 the renderer sends the save ticket with `saved`, so an edit made during the write stays unsaved (`engine/design/app-m9-recovery.md`), and the save path refuses to replace a read-only file and writes through symlinks (`engine/design/m10-fuzz.md`).
- [x] Documented handling of repaired, linearized and incremental inputs: documented (APP-ENGINE-DESIGN.md, "Tolerant open" and the Differences), and `open` is accepted since round 3.
- [ ] Undo/redo cost on large documents under the M10 budgets. The app journal restores one step's object states and does not replay; the 37 ms replay for 12.6 MB is the preview Session's. The M10 benchmarks budget 200 annot-add and undo cycles per document (cycle median 2.2 ms on the 500-page `large`, 1.0-1.6 ms on the 226- and 352-page files; `engine/reference/bench-20261017/`); redo is not measured on its own.

### M3: practical text editing (ledger: edit-paragraph, text-add, edit-replace-preview, edit-replace-apply; finish edit-targets, edit-text)

Status: all six rows accepted, after two fix rounds (`engine/design/app-accept-edit.md`).

- [x] Paragraph edits, new text, and replace preview/apply with the app's semantics.
- [x] Two-byte (Type0) runs with Identity-H or coded CMaps (`engine/design/app-edit.md`).
- [ ] Font embedding, or another proven portable representation, for text drawn with substitute fonts. The private-sample file needs this.
- [ ] Structure-aware edits that maintain ActualText and the structure tree, beyond engine.ts. The app engine's handling of runs under ActualText is in `engine/design/app-edit.md`.
- [ ] Line boundaries, spacing, rotation, different replacement lengths and overflow without moving unrelated content, beyond what engine.ts does.
- [ ] Exit gate: the drive scripts, the edit cases among them, pass on the Rust build. One meaningful edit of the private-sample file is not shown.

### M4: rendering and extraction (ledger: search, text-all; compare-render/compare-text with M7)

Status: `search`, `text-all`, `compare-render` and `compare-text` accepted at the first review.

- [x] Structured text (lines, spans, quads, identities) for selection, search, extraction and compare, with ligatures, marked content and ActualText (`engine/src/view/stext.rs`, `engine/design/app-corpus-text.md`).
- [ ] Fonts, color and transparency gaps: written for the bespoke pipeline. The app engine draws with hayro; the corpus differential measures it against MuPDF (`render-pixels` on 185 of 5,049 files at 62a9b21) and its README lists the remaining classes.
- [x] Visual tolerances against MuPDF, recorded: the corpus differential's content classes and thresholds (`engine/reference/corpus-20261015/README.md`). MuPDF is an oracle, not the spec: where viewers differ, choose the better behavior and record the difference.

### M5: pages and navigation (16 ledger rows)

Status: all accepted. The pages group was accepted at the first review; navigation after one fix round (`engine/design/app-accept-nav.md`); `layers` and `layer-set` with the view group.

- [x] page-rotate, page-delete, page-move, insert-pages, extract-pages; blank and image document creation.
- [x] outline, outline-add, outline-set, outline-delete; links, link-add, link-set, link-delete.
- [x] labels, labels-set, labels-clear.
- [x] layers, layer-set.
- [x] Destinations, bookmarks, links, labels, widgets and fields when pages move between documents (APP-ENGINE-DESIGN.md, "Pages and assembly" and the Differences).

### M6: annotations and forms (9 ledger rows; finish fields)

Status: all accepted after one fix round each (`engine/design/app-accept-annots.md`, `app-accept-forms.md`). The round 3 minors lane fixed or recorded their minors, except a cut inside a /Nums value array (annot-delete, skipped).

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

Status: all eight accepted. `redact-preview` and `redact-apply` were accepted at the third re-review of round 3 (`engine/design/app-accept-redact.md`, "Round 3").

- [x] **Encryption:** security-set and encrypted saves (keep, remove, or AES-256 R6), as engine.ts. Public-key handlers and named non-Identity crypt filters are refused (`engine/src/security.rs`); add them only if the app needs them.
- [x] **Redaction:** redact-preview and redact-apply. Round 3 bounded the clip entries in the shared-content check by its work budget, made the region queries independent of duplicate and wide mark rects, and checks pixels row by row (`app-p8-fix-redact`, 608fd9d; `engine/design/app-accept-redact.md`). On the 174-file corpus sample every round left no character inside a mark and no search hit, and changed no unmarked page.
- [x] **Signatures:** sign-list and sign-prepare, as engine.ts. Signed and certified documents stay read-only, as in engine.ts.

### M9: replace the app's engine

- [x] Production adapter on the real `SessionRequest` union: `src/renderer/engine-rust.ts` (APP-ENGINE-DESIGN.md, "Wire format").
- [x] Run every app unit and drive test with the Rust backend: no failure (`engine/reference/app-acceptance-20261016/`). Re-run at 82c8693 by the integration review: `node tests/run-rust.mjs` 667 tests, 564 pass, 0 fail, 103 skip; the full suite with Electron 678 tests, 667 pass, 0 fail, 11 skip.
- [x] Integration blocker: inherited pattern and Type3 chains past the held-content budget. Fixed in round 3 with a content budget inside the vendored hayro-interpret, and the integration check accepted at re-review (`app-p8-fix-integration`, 8ca0c05; `engine/design/app-accept-integration.md`). Its three minors (wide-image row memory, the scene estimate for strokes, tests bound by timing) were fixed after acceptance on `app-p10-followups`.
- [x] Recovery for long operations: after a watchdog restart, a trap, a worker error or a failed start, the renderer reopens the opened bytes and replays the logged changes, without the request that hung, on both engines; long jobs that time out restore too. Accepted after 3 review rounds (`app-p9-m9-replay`, merged at fb9e163; `engine/design/app-m9-recovery.md`). A pre-existing save-ticket bug was fixed with it.
- [x] Mobile (`mobile/`) on the Rust build, done in the build: merged at 92e7612 on the owner's decision of October 17. The scanner composes its PDF with the Rust engine (`scan-compose` and `scan-measure`, accepted; `app-p11-rust-scan`, merged at 97fa6dc), and the TypeScript PDF writer is deleted. Still open: device verification on iOS and Android, the mobile notice (AGPL section 6), and scanner fonts for Vietnamese, Hebrew, Arabic, Thai and emoji, which need a new font file (`engine/design/app-m9-mobile-release.md`, sections 6 and 9).
- [x] Release engine switch, done in the build (merged at 92e7612): `npm run package` builds only Rust (`package:rust` is an alias; `build:mupdf` and `test:mupdf` stay for the MuPDF oracle), and `release.yml` builds one reproducible WASM with pinned Rust, checks it against `engine/reference/wasm-release.sha256`, and checks every installer with `scripts/check-package.mjs`. Validated locally on darwin-arm64 only; no GitHub Action has run it.
- [ ] Licence wording: the `NOTICE.txt` and About panel text of a Rust build (`engine/design/app-worker.md`), with the README, mobile footer and `engine/NOTICE` wording. **Merged at 92e7612; the owner's sign-off is pending before a Rust release,** as is the question of corresponding source.
- [ ] File dialogs, Save As filename adoption, save-in-place, printing, export and packaging paths. The drive scripts that save to a new file, save in place and print to PDF pass on the Rust build, and on the macOS ARM64 package; the package checklist also exports PNG and text (`engine/design/m10-macos-package.md`). Every picker was pre-answered: the native open and save panels and the print dialog were never shown. Packaging is checked on darwin-arm64 only. The M10 save-failure tests cover read-only files and folders, symlinks, files replaced or deleted during a save, and a killed save (`engine/design/m10-fuzz.md`), and Save As now binds the window to the identity of the file it wrote (final review, `tests/save-as-rebind.test.mjs`).
- [x] The production build works with MuPDF runtime assets absent, on darwin-arm64: the `npm run package:rust` app holds no MuPDF file, and its packaged smoke passes. MuPDF stays a test-only oracle there. Windows and Linux are M10.

### M10: release

- [x] Fixed benchmark set on named machines: open and first page, zoom, render, thumbnail, search, edit, save, reopen, peak memory, retained history and session growth. Record warm, cold and tail results, then freeze the budgets. Done on one named machine, MacBookAir10,1 (Apple M1, 8 GB, macOS 26.6.2), at 39b0699: 18 documents, 3 cold processes and 10 warm passes each, against MuPDF through the app's adapters (`app-p10-bench`; `engine/reference/bench-20261017/`, tools in `engine/oracle/bench/`). Rust is much faster at edits and lower in peak RSS. It is slower at renders at 0.5x and 1x and at thumbnails (2-20x; a fixed per-call cost), at warm open of large files (up to 26x; it parses more at open, where MuPDF opens lazily) and at search on some files (`comparison.md`). Budgets are frozen at 1.5x the Rust p90, with floors; `check-budgets.mjs` is timing-dependent and not one of the default gates.
- [x] Replace the preview limits (16 MiB, 64 pages, 64 steps, 64 MiB history) with the app's limits (120 MiB input, 50 MiB output) without an unbounded full-document pipeline. Written for the preview pipeline (`engine/desktop`), which the app does not use. The app engine already enforces the app's limits: 120 MiB input and 50 MiB save (`engine/src/app/mod.rs`, `MAX_PDF_BYTES` and `MAX_SAVE_BYTES`, as engine.ts), with no page cap (`engine/APP-ENGINE-DESIGN.md`, "Limits"; the `open` review checked them). Its journal keeps one step's object states, with no step cap, as engine.ts has none; the benchmarks measured retained history (0 WASM growth on every document) and growth over 200 edit cycles.
- [x] Fuzz, mutation and malformed-input campaigns, and save failure and recovery tests, done to the scope recorded in `engine/design/m10-fuzz.md` and accepted after 3 review rounds (`app-p10-fuzz`, merged at de49f8a). The campaign reuses the existing review fuzzers (`engine/oracle/fuzz`); it does not cover editing operations on damaged inputs, encrypted inputs beyond the p7 fixtures, large inputs or real-world damage. Recovery is M9's replay, above. Found and not fixed: two rare repair shapes and a damaged form /BBox in the view. The final review's fresh seeds gave 0 traps, and its repair and lexer fixes were checked with paired fuzz reruns against the previous build (`m10-fuzz.md`, "Final review (minors)" and "Final re-review: integers past i64").
- [ ] Differential render and semantic checks, and the full drive suite, on the final head. The final review ran the full suite with Electron, drive-script tests included, on 11bf970 (749 tests, 0 failed) and a 603-file corpus sample with no new Rust error. On ba70615 (acceptance report 8.10) the full corpus differential found no Rust trap, timeout or error and no file newly failing, and the full suite with Electron passed (760 tests, 740 passed, 0 failed). Neither has run after the mobile and scanner merges (92e7612, 97fa6dc).
- [x] Independent review of everything since the last accepted checkpoint, with no unexplained crash, corruption, wrong render or data loss left. The final review of 46f1e95..11bf970 accepted it with no blocker or major (the full suite with Electron, a 603-file corpus sample, fresh fuzz seeds with 0 traps). Its five minors were fixed on `app-p10-final` (merged at 39b0699): the redaction pixel check on pages the view refuses at full scale, plain verify-failed messages, the trailer reread rule, Save As rebinding to the written identity, and integers past i64 read as reals. The recheck found a blocker in the first pixel-check fix (an endless loop on /UserUnit 1e36) and an edge-band minor; both were fixed and accepted (`engine/design/app-accept-redact.md`, `m10-fuzz.md`). `app-p10-pkg` and `app-p10-bench` came after the review.
- [ ] Real packaged builds on macOS ARM64, Windows x64 and Linux x64: WASM startup, fonts and assets, dialogs, filesystem recovery, printing, high-DPI, attribution and reproducible build instructions.
  - [x] macOS ARM64: every checklist item passes on the package (`app-p10-pkg`; `engine/design/m10-macos-package.md`). Native pickers were pre-answered, not shown, and the package is unsigned. The WASM builds reproducibly across checkout paths (`docs/building-rust-release.md`). Missing wasm-bindgen and bumpalo licence files were added. Owner decisions: Electron fuses (a new dev dependency, `@electron/fuses`) and the source tag `NOTICE.txt` names.
  - [ ] Windows x64 and Linux x64: pending the owner. They have no Rust package: the `release.yml` switch is merged but has never run, because CI needs a push.

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
