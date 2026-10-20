# Functional Rust workflow

> Licence note (October 18, 2026): the Rust engine and the app are now MIT, and no MuPDF code ships. Where this document says engine code is AGPL or MuPDF-derived, or that a build owes AGPL obligations, it records the state before the clean-room relicense (see the Licence section of [`PROVENANCE.md`](reference/PROVENANCE.md)).

Updated October 17, 2026. **Live source location:** `$REPO/engine/`, tracked in git (branch `rust-engine-app`). The older Codex worktree at `~/.codex/worktrees/rust-pdf-engine/TPDF/engine/` stopped at the October 12 feature-gate follow-up and is superseded; do not build from it.

## October 17, 2026 (later): mobile on Rust, and the scanner on the Rust engine

The owner decided on October 17: "mobile can be fully rust and merged". Two branches were merged into `rust-engine-app`, whose head is now 3d8a6ff (`git log --first-parent 5c21bf3..3d8a6ff`), 250 commits ahead of local `main`. Nothing is pushed or published. The acceptance report (`../docs/rust-engine-acceptance-report.md`, section 8.11) has the detail, and `reference/app-acceptance-20261016/verdicts.json` has both lanes in its `round3` section.

| Branch | Tip | Merged at | Outcome |
| --- | --- | --- | --- |
| `app-p9-mobile-release` | 5a00457 | 92e7612 | mobile on the Rust build; `npm run package` and `release.yml` build only Rust; the Rust NOTICE and About wording |
| `app-p11-rust-scan` | 8ce01d7 | 97fa6dc | `scan-compose` and `scan-measure` accepted (an independent review with three rechecks at 8ce01d7); the TypeScript scan writer deleted |

- **Mobile and releases** (`design/app-m9-mobile-release.md`, sections 1, 2 and 8). The branch was refreshed onto the final head and re-gated on its final tip.
  - Mobile builds the desktop app with Rust and refuses a MuPDF build.
  - `npm run package` builds only Rust (`package:rust` is an alias); `build:mupdf` and `test:mupdf` stay for the MuPDF oracle.
  - `release.yml` builds one reproducible WASM with pinned Rust, checks it against `reference/wasm-release.sha256`, and checks each installer with `scripts/check-package.mjs`.
  - The licence wording is merged and needs the owner's sign-off before a release. No GitHub Action has run.
- **The scanner** (`design/app-m9-mobile-release.md`, section 9; `APP-ENGINE-DESIGN.md`, "Scanner"). The mobile scanner composes its PDF with the Rust engine through `scan-compose` and `scan-measure`. `scan-pdf.ts`, `scan-font.ts` and `scan-metrics.ts` are deleted, which settles the duplicated writer.
  - Rebuilt pages now draw Latin Extended-A 121 of 121, Greek 72 of 135, Cyrillic 137 of 256, maths 28 of 256, arrows 7 of 112 and currency 4 of 33; all were boxes.
  - Vietnamese, Hebrew and Arabic letters, Thai and emoji are still boxes, but extract and search. Covering them needs a new font file, an owner decision.
  - Right-to-left runs are written in visual order, with UAX #9 W4-W6 numbers and marks before their base, so search works in MuPDF, Rust and PDFKit.
  - Two cosmetic notes remain: a trailing U+200F after Arabic that ends in a marked letter, and combining marks no font has are not drawn.
- **Ledger:** 69 rows, all `accepted` (3d8a6ff); `tools/operation-ledger.py` passes.
- **Criterion 2** (every app workflow on Rust) now holds in the build for desktop and mobile. Nothing has run on an iOS or Android device.

Gates on 92e7612 (the mobile merge): all 12 pass.
- Native tests: 1,683 passed, 0 failed on Rust 1.98.1 and 1.92.
- App unit suite: 760 tests, 616 passed, 0 failed.
- App suite on Rust: 749 tests, 623 passed, 0 failed.

**Gates on 3d8a6ff (the scanner port merged): all 12 pass** (October 17, 2026; memcap 3000 MB; logs in `tpdf-wt/gates-out/final-scan/`, not committed). Native tests 1,700 passed, 0 failed on Rust 1.98.1 and 1.92; app unit suite 763 tests, 619 passed, 0 failed, 144 skipped; app suite on Rust 752 tests, 626 passed, 0 failed, 126 skipped; peak 1,920 MB. Mobile `npm test`: 16 passed, 0 failed. The WASM is `2de5d021c63352915f50d72890d9bc7bc824f7065506f398d1129217845f0d40` (13,614,507 bytes), reproduced by a clean build at a second checkout path (`tpdf-wt/p11-docs`); `engine/reference/wasm-release.sha256` records it.

**Open, for the owner** (the acceptance report's section 6.7 is the complete list):
- the licence wording sign-off, before a release;
- the mobile notice (AGPL section 6 and app-store distribution) and corresponding source;
- scanner fonts for the remaining scripts (a new font file) and the scanner's other output differences;
- the NOTICE source tag and Electron fuses;
- the performance follow-ups the benchmarks show, and the package follow-ups;
- device verification;
- Windows x64 and Linux x64 packages, which need CI and so a push;
- pushing and merging `rust-engine-app` to `main`.

## October 17, 2026 (final): M10 package checklist, final review and benchmarks

After the gates on 11bf970, three more branches were merged into `rust-engine-app`, whose head is now b6bc7b9 (`git log --first-parent de398ab..b6bc7b9`). Nothing is pushed or published. The acceptance report (`../docs/rust-engine-acceptance-report.md`, sections 8.6 to 8.9) has the detail, and `reference/app-acceptance-20261016/verdicts.json` has the lanes, `finalReview` and `bench` in its `round3` section.

| Branch | Tip | Merged at | Outcome |
| --- | --- | --- | --- |
| `app-p10-pkg` | 94f3ca9 | 2efe665 | M10 macOS ARM64 package checklist: every item passes; reproducible WASM; missing licence files added |
| `app-p10-final` | 4d65808 | 39b0699 | the final review's five minors fixed; the recheck's blocker and minor fixed and accepted |
| `app-p10-bench` | 03341e0 | b6bc7b9 | M10 fixed benchmark set on one named machine; budgets frozen |

- **macOS ARM64 package** (`design/m10-macos-package.md`). `scripts/check-macos-package.mjs` runs the M10 checklist against the package `npm run package:rust` makes. Every item passes:
  - WASM startup: median 764 ms cold and 731 ms warm to the first page drawn;
  - fonts and assets;
  - dialogs, with pickers pre-answered: 48 of 51 drive scripts on their own, and the other three through their tests;
  - orphan temp cleanup;
  - the M9 replay, on a patched clone;
  - print to PDF;
  - high-DPI;
  - attribution;
  - reproducible build instructions.

  The native panels, the print dialog and the About window were never shown, and the package is unsigned. The WASM had held checkout and home paths, and cargo hashed path crates by absolute path. `tools/build-workflow.sh` now remaps the paths, and `tools/rustc-stable-metadata.sh` (a `RUSTC_WRAPPER`) hashes those crates without the path. Three builds were byte-identical, two of them clean from different paths (`../docs/building-rust-release.md`). The wasm-bindgen and bumpalo licence files were missing and are added. Owner decisions: Electron fuses, which need `@electron/fuses`, a new dev dependency; and the source tag `v0.1.0` that `NOTICE.txt` names, which does not exist yet. Windows and Linux are not covered.
- **Final independent review** of 46f1e95..11bf970: accepted with no blocker or major. The full suite with Electron gave 749 tests, 0 failed; a 603-file corpus sample gave no new Rust error; fresh fuzz seeds gave 0 traps. Its five minors are fixed:
  - The redaction pixel check draws a page the view refuses at full scale again at half scale, refuses `scan-budget` for page sizes it cannot draw, and keeps the unread edge band under 2.02 pt.
  - Trailers get the repair's reread rule.
  - Save As rebinds the window to the identity of the file written.
  - Integers past i64 are read as reals, never wrapped (a recorded difference from MuPDF).
  - verify-failed messages no longer name search texts.

  The recheck found a blocker in the first pixel-check fix: /UserUnit 1e36 made its retry loop endless. It also found the edge-band minor. Both were fixed (297c53a, 4d65808) and accepted. Notes: `design/app-accept-redact.md`, `design/m10-fuzz.md`, `design/app-accept-session.md`.
- **Benchmarks** (`reference/bench-20261017/`, tools in `tools/bench/`). MacBookAir10,1 (Apple M1, 8 GB, macOS 26.6.2), 18 documents, Rust against MuPDF through the app's adapters, 3 cold processes and 10 warm passes each.
  - Rust is much faster at the edit sequence (geometric mean 0.14-0.32x) and the annot-add and undo cycle (0.11-0.13x), and lower in peak RSS (0.77-0.81x).
  - It is slower at renders at 0.5x and 1x and at thumbnails (2-20x; a fixed per-call cost), at warm open of large files (up to 26x; it parses more at open, where MuPDF opens lazily) and at search on some files (`comparison.md`).
  - Budgets are frozen at 1.5x the Rust p90, with floors. `tools/bench/check-budgets.mjs` passed right after the measurement; it is timing-dependent, so it is not one of the default gates.

Gates on 39b0699 (`app-p10-pkg` and `app-p10-final` merged; 2026-10-17; under memcap 3000 MB; logs in `tpdf-wt/gates-out/final-39b0699/`, not committed): all 12 pass.
- fmt and strict clippy, engine and bridge, on Rust 1.98.1 and 1.92.
- Native tests: 1,683 passed, 0 failed on both toolchains.
- Bridge tests: 4 passed.
- The WASM package (sha256 `3c18b0e8cb8dcf868fa13c4aacb4f5a75e29efe3744b353245c298e94783de93`) and `npm run build`.
- App unit suite: 760 tests, 616 passed, 0 failed, 144 skipped.
- App suite on Rust: 749 tests, 623 passed, 0 failed, 126 skipped.
- Peak 1,977 MB.
- b6bc7b9 adds only the benchmark tools and reference, outside the WASM's sources.

**Open, for the owner** at b6bc7b9 (superseded by the section above):
- the decisions on the mobile and release branch: licence wording, the mobile notice and app-store distribution, corresponding source, no MuPDF fallback, scanner fonts, the duplicated PDF writer, output differences, follow-ups. The branch was merged later at 92e7612, and the scanner port at 97fa6dc (above);
- the NOTICE source tag and Electron fuses;
- the performance follow-ups the benchmarks show;
- device verification;
- Windows x64 and Linux x64 packages, which need CI and so a push;
- pushing and merging `rust-engine-app` to `main`.

## October 17, 2026: all 67 operations accepted; M9 recovery; M10 save failures and fuzzing

This section is the state at 11bf970. Its open items and next steps are superseded by the section above.

After the October 16 review the owner asked for a third fix round, M9 recovery and part of M10. Six branches, each reviewed independently, and the integration follow-up are merged into `rust-engine-app` at 11bf970 (`git log --first-parent 46f1e95..11bf970`). Nothing is pushed or published. The acceptance report (`../docs/rust-engine-acceptance-report.md`, section 8) has the detail, and `reference/app-acceptance-20261016/verdicts.json` a `round3` section.

| Branch | Tip | Merged at | Outcome |
| --- | --- | --- | --- |
| `app-p8-fix-open` | 94be413 | d0980a4 | `open` accepted; the review's two minors fixed in 94be413 |
| `app-p8-fix-minors` | 9356748 | 034eead | minors on accepted ops fixed or recorded; accepted, no regressions |
| `app-p8-fix-redact` | 608fd9d | 714396c | `redact-search`, `redact-preview` and `redact-apply` accepted at the third re-review |
| `app-p9-m9-replay` | 0fd6015 | fb9e163 | M9 recovery accepted after 3 review rounds |
| `app-p8-fix-integration` | 8ca0c05 | 9ef8f49 | integration check accepted at re-review; its three minors fixed on `app-p10-followups` (0dacd84, merged at 11bf970) |
| `app-p10-fuzz` | b251a79 | de49f8a | M10 save-failure tests and fuzz campaign accepted after 3 review rounds |

At 11bf970 the ledger recorded 67 rows `accepted` (69 since the scanner ops, above).

- **open** (`design/app-accept-session.md`, "Round 3"). The repair reads an xref entry the scan did not define at its own offset instead of dropping the xref, and no longer reads a dictionary past a header that starts a line and parses whole; an untyped page-tree node with /Kids is a node in the repair, the page walk, page edits, the save and the view; trailer entries are taken from dictionaries between objects when the file is scanned, so RC4 and AES-128 files with a damaged `trailer` keyword open. The review found two minors in that last change (loose dictionaries overriding an intact xref; junk between objects using up a budget), both fixed. Corpus: no file newly refused, none with fewer pages or less text.
- **Redaction** (`design/app-accept-redact.md`, "Round 3" and its re-reviews). The clip entries in the shared-content check are bounded by its work budget (a 3,972-byte file: 973 MB to 186 MB); region queries no longer depend on repeated file-borne rects (a 67 KB file: 74-89 s to 0.25 s); the pixel check reads each row once. The re-reviews found a Form without a usable /BBox decided by a box cut at the page edge (blocker, pre-existing), uncharged region-list keys (blocker, from the first re-review's fix) and three minors, all fixed. The 174-file sample is unchanged in every round: 0 characters left inside marks, 0 search hits left.
- **Integration** (`design/app-accept-integration.md`, "Round 3" and its re-review). A content budget inside the vendored hayro-interpret bounds the decoded content hayro holds along every inheritance path to 128 MiB (8-level inherited chains: 1,157 MB of WASM, drawn, to a refusal at 261 MB). Flate-only images too large to hold are drawn reduced a row at a time (`gray-24000.pdf`: 1,030 MB and refused, to drawn in 1.0 s at 6-8 MB). The re-review's two blockers (large images drawn tiny; type 4 functions without a bound) and three minors are fixed, the render's scene estimate is bounded at 384 MiB, and corpus run 62a9b21 refuses no page newly (238 files on the failing list). After acceptance, `app-p10-followups` bounded reduced image rows at 2^20 samples, estimates strokes by the tiles they cross (`vert-4000` and `hatch-4000` now draw, `plot-200k` is now refused), and scales eleven app tests' wall-clock bounds to the machine's speed; corpus run 4280502 matches 62a9b21 file for file on the Rust side.
- **Minors on accepted ops** (the "Round 3 minors" sections of the forms, annots, journal, edit, objects and sign notes). Widgets take the re-encoded Latin font; FreeText draws format characters as nothing, composes combining marks and keeps dashed borders; a large valid PNG is `too-large`; an empty /BaseFont is listed as null; validate names the values limit; the sign window is guarded. The rest are recorded as differences. Still open: three minors (`render`, `annot-delete`, `edit-replace-preview`).
- **M9 recovery** (`design/app-m9-recovery.md`). The renderer keeps the opened bytes and a log of the requests that changed the session. After a watchdog restart, a trap, a worker error or a failed start, it replays the log into a fresh worker, checks every answer and restores the window; the request that hung is not replayed and the notice names it. If the replay fails, the last saved state is reopened. On large.pdf, 240 changes replay in 422 ms on Rust. A pre-existing bug was fixed with it: `saved` was sent without the save's ticket, so an edit made during a save in place was reported saved on both engines.
- **M10** (`design/m10-fuzz.md`). Save-failure tests for the app's save path found and fixed: Save silently replacing a read-only file, an in-place Save through a symlink replacing the link, a link retargeted during the write, and Save As resetting an existing file's mode. The campaign reuses the existing review fuzzers (`tools/fuzz`): 20,200 generated inputs per run, with 0 traps, 0 hangs and 0 memory-cap hits. It fixed three repair cases and a /Contents entry that blanked the page in the view. Scope: no new fuzz or mutation code was written (a first attempt that wrote a new mutator was stopped by a safety filter); editing operations on damaged inputs, encrypted inputs beyond the p7 fixtures, large inputs and real-world damage are not covered.

**Prepared at 11bf970, merged since at 92e7612 (above):** `app-p9-mobile-release` (then d470956, base 46f1e95): mobile on the Rust build with a scanner that writes its own PDF, `npm run package` and `release.yml` building only Rust, and the licence and NOTICE text. Its owner decisions are in `design/app-m9-mobile-release.md` on that branch, section 6. It was gated on its base 46f1e95 only, so it needs the current head merged in and the gates run again before the owner merges it.

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
- **Gates on 11bf970, the head the final review covered: all 12 pass** (October 17, 2026; every step under memcap 3000 MB, 2 build jobs, 2 test threads, with the fixed gate script; logs in `tpdf-wt/gates-out/final-11bf970/`, not committed). fmt and strict clippy, engine and bridge, on Rust 1.98.1 and 1.92.0; native tests 1,670 passed, 0 failed on both toolchains; bridge tests; the WASM package and `npm run build`; app unit suite 749 tests, 609 passed, 0 failed, 140 skipped; app suite on Rust 738 tests, 614 passed, 0 failed, 124 skipped; peak 1,987 MB. The docs merge 525ad90 changes `vendor/hayro-interpret-0.8.0/PATCH.md` prose, which the WASM source hash covers, so the package was rebuilt there (sha256 `6e900969…`), `npm run build` passed and the app suite on Rust passed again (738 tests, 614 passed, 0 failed; logs in `gates-out/post-docs-525ad90/`). The full suite with Electron is run by the final review.

**Open, for the owner** (`../docs/rust-engine-completion-plan.md`):

- the decisions on the mobile and release branch (merged since at 92e7612): licence wording, the mobile notice and app-store distribution, corresponding source, no MuPDF fallback, scanner fonts, the duplicated PDF writer, output differences, device verification;
- Windows and Linux packaging: no Rust package exists, because the `release.yml` switch was then unmerged and nothing was pushed;
- whether and when to push.

Next, by the coordinator: the M10 benchmarks, the macOS ARM64 package checklist and a final independent review of everything since October 16. All three are done (above).

## October 16, 2026: app engine acceptance, 64 of 67 operations (M0-M9)

This section is the October 16 state. The three ops and the integration check it lists as not accepted were accepted on October 17 (above), and its open items are updated there.

**What the app engine is.** Since October 14 the app's engine has been rebuilt on the `rust-engine-app` branch. `engine/src/app` implements all 67 operations of the app's `SessionRequest` contract (`src/renderer/engine.ts`): validation, refusal reasons, reply shapes, PDF mutation, the journal and the app save. Rendering and structured text come from hayro 0.8, vendored with patches under `engine/vendor/`. The WASM bridge (`engine/bridge`, `RustApp`) and the TypeScript adapter (`src/renderer/engine-rust.ts`) connect it to the app's worker. `scripts/build.mjs` picks `--engine`, else `TPDF_ENGINE`, else `rust`, so `npm run build` and `npm start` ship and run the Rust engine. `npm test` builds it too, and its Electron tests run Rust, but most of its unit tests import engine.ts; `npm run test:rust` runs those on Rust. A build carries one engine with no runtime fallback, and `npm run build:mupdf` builds MuPDF. Design and decisions: `APP-ENGINE-DESIGN.md` and `design/app-*.md`. The preview Session described in the sections below is separate; `engine/src/app` does not use it or the bespoke render pipeline.

**Independent acceptance review** (workflow run wf_47a8f38e-1a3, October 16). Every ledger row was reviewed against engine.ts in 12 groups, plus an app integration check. Each group got up to two fix rounds, each re-verified independently. The evidence branch (`app-p7-evidence`, 06e717d) and the 10 fix branches are merged into `rust-engine-app` at 3d1a300. Nothing is pushed or published.

| Group | Ops | Accepted at first review | Fix rounds | Accepted at the end |
| --- | --- | --- | --- | --- |
| session | open, close, page-ids, authenticate, security-set | 4 | 2 | 4 (`open` not) |
| journal | save, validate, saved, undo, redo | 3 | 1 | 5 |
| view | render, thumbnail, search, text-all, layers, layer-set | 6 | 0 | 6 |
| pages | page-rotate, page-delete, page-move, insert-pages, extract-pages, pdf-from-images, batch-file, stamp | 8 | 0 | 8 |
| nav | outline, outline-add, outline-set, outline-delete, links, link-add, link-set, link-delete, labels, labels-set, labels-clear | 3 | 1 | 11 |
| annots | annots, annot-add, annot-set, annot-move, annot-delete | 3 | 1 | 5 |
| forms | fields, field-set, field-add, field-props, field-delete | 2 | 1 | 5 |
| edit | edit-targets, edit-text, edit-paragraph, text-add, edit-replace-preview, edit-replace-apply | 2 | 2 | 6 |
| objects | edit-object, edit-objects, edit-batch, clip-copy, clip-paste | 3 | 2 | 5 |
| compare | compare-open, compare-close, compare-render, compare-text | 4 | 0 | 4 |
| redact | redact-add, redact-set, redact-search, redact-preview, redact-apply | 2 | 2 | 3 (`redact-preview`, `redact-apply` not) |
| sign | sign-list, sign-prepare | 1 | 1 | 2 |
| **Total** | 67 | 41 | | **64** |
| integration | not a ledger row | not accepted | 2 | not accepted |

Not accepted:

- **`open`.** The second session fix (c4d2181) made the repair skip the object header after a dictionary missing its closing `>>`, and then drop the whole xref, even when it is intact; the save writes the loss. On a synthetic 3-page file, 8 of 9 damage shapes lose a page; blanking those two bytes in three fixtures loses a link, a field or a page. Three majors remain, all older than the fixes: an intact xref is still dropped when an object header is glued to a damaged object; an untyped intermediate page-tree node shows as one blank page hiding its pages (deleting it deletes them); one damaged byte in the `trailer` keyword makes RC4 and AES-128 files unopenable where engine.ts opens them.
- **`redact-preview` and `redact-apply`.** The second redaction fix (67ed038) records every clip path inside a touched Form once per mark rect before any work bound is checked, so memory grows as clips times marks. A 3,972-byte file with 100 search hits and a Form of 50,000 clips took the preview from 140 to 973 MB. Nothing bounds it short of the 4 GB wasm32 heap, where the engine aborts and the open document and its unsaved edits are lost (extrapolated, not run). A major that predates the fixes: with many mark rects from the file (262,144 through shared /QuadPoints), the shared-content check is quadratic, 74-89 s on a 67 KB file against 0.55 s for engine.ts.
- **Integration** (not a ledger row). A form inherits the fill pattern and Type3 font of the content that draws it, and the content walk does not count them, so decoded forms still chain past the 128 MiB held-content budget: 8 levels reach 1,157 MB of WASM from a 6,923-byte file, about 128 MB per level, and about 31 levels would reach the 4 GiB abort (extrapolated, not run). The two majors are owner decisions: recovery for long operations, and mobile on MuPDF. Minors: releases build MuPDF; refusing an image over 512 MiB grows WASM by about 1 GB that the worker keeps.

Findings, fixes and decisions per group: `design/app-accept-<group>.md` (view, pages and compare were accepted at the first review and have none). `reference/app-acceptance-20261016/verdicts.json` lists each group's rounds, branch and tip, and each op's verdict with the findings still open. The ledger (`../docs/rust-engine-operation-ledger.csv`) records 64 ops `accepted` and 3 `not accepted`.

**Evidence.**

- `reference/app-acceptance-20261016/`: the op-to-test map for all 67 ops and both full suites, measured on the app and engine code of d352fcd. No test fails on either engine or either build. Rust unit run: 560 pass, 0 fail, 114 skip. Full suite on the Rust build: 666 pass, 0 fail, 8 skip. All 50 drive scripts pass on both builds. `page-ids` had no unit test then (`tests/p7-page-ids.test.mjs` came with the session fixes); `close`, `security-set`, `extract-pages`, `pdf-from-images`, `text-all` and `layer-set` have no drive script.
- `reference/corpus-20261015/`: the differential against MuPDF on 5,049 public PDFs (its README has every run). At 767bb5e, the last integration round: 239 files on the failing list (723 at the October 15 baseline); no Rust trap, timeout, OOM or crash; Rust opens 4,949 files and MuPDF 4,923, and Rust refuses none that MuPDF opens.
- The integration review's re-run at 82c8693: `node tests/run-rust.mjs` 667 tests, 564 pass, 0 fail, 103 skip; the full suite with Electron 678 tests, 667 pass, 0 fail, 11 skip.

Gates on the merged head:

All pass at `3d1a300` (2026-10-16; every step under memcap 3000 MB, 2 build jobs, 2 test threads; logs in `tpdf-wt/gates-out/final-3d1a300/`, not committed):
- `cargo fmt --check` and `cargo clippy -D warnings`, engine and bridge, on Rust 1.98.1 and 1.92.0.
- Native tests: 1,618 passed, 0 failed in 131 test binaries on 1.98.1, and the same on 1.92.0 (peak 1,521 MB).
- Bridge tests: 4 passed.
- The WASM package (`engine/tools/build-workflow.sh`) and `npm run build` (Rust build) both build.
- App unit suite without Electron: 681 tests, 568 passed, 0 failed, 113 skipped.
- App unit suite on Rust (`tests/run-rust.mjs`): 670 tests, 567 passed, 0 failed, 103 skipped.
- Full suite with Electron on the Rust build, including the 20 test files that run drive scripts (`node --test --test-concurrency=2 tests/*.test.mjs`): 681 tests, 670 passed, 0 failed, 11 skipped (peak 1,721 MB).

**Open, for the owner** (`../docs/rust-engine-completion-plan.md`):

- a third fix round for `open`, `redact-preview`, `redact-apply` and the integration blocker;
- M9: recovery for long operations (the 20 s watchdog restarts the worker and drops unsaved edits); mobile still builds and scans with MuPDF; releases and CI still build MuPDF (`npm run package`, `release.yml`); the `NOTICE.txt` and About wording for a Rust release;
- M10: benchmarks, fuzzing, and packaged Windows and Linux builds. The Rust package is checked on darwin-arm64 only.

## October 14, 2026 (late night, later): Form copies are edited in place (M2)

Editing Form text no longer leaves a new copy behind on every edit. A Form on the placement chain is edited in place when it is private:

- the page traces exactly one invocation of that Form object, annotation appearances included;
- the document holds exactly one reference to it, inside the object that places it.

That placing object must be this page (its own direct /Resources) or a parent Form that is private in the same sense. Another page cannot resolve a page's own resources, so a private Form is drawn exactly once in the document. When the edited Form is private, nothing above it changes. When a parent is private, its copy-of-child name is added to its own resources in place instead of copying it. Otherwise the chain is copied as before.

This applies to the copies made by earlier edits, and also to original Forms that were already private: targets-streams.pdf's Fm2 (NEST) is now edited in place, while Fm1, which two pages and Fm2 name, is still copied.

Measured over 20 edits (`history-costs.jsonl`): edit-shared.pdf stays at 1.8 KB instead of growing to 7.2 KB, edit-objects.pdf at 3.3 KB instead of 8.6 KB, and redact-shared-only.pdf at 1.5 KB instead of 7.5 KB. Their retained history drops 3.6–4×. Other documents are unchanged.

Fixtures: 53/51/18 open/render/edit, and all render hashes and capabilities equal the previous evidence. All 18 editable documents pass the independent MuPDF edit check, and so do the two in-place cases on targets-streams.pdf (nested FORM and NEST). Verification: 897 tests pass on Rust 1.98.1 and 1.85.1, with fmt and strict Clippy for core and bridge. The WASM build and desktop smoke pass. The WASM SHA-256 is `68282b036199ec2292c4b13e5defdff7024246869eb0cc268540788fc03eb8f3`. Evidence: `reference/m2-inplace-20261014/`.

## October 14, 2026 (late night): deletion, reachable saves, delta history and nested Forms (M2)

The four remaining M2 items from the night increment are implemented.

- **Object deletion.** `Document::delete_objects` removes a set of objects in the transaction-local document. It refuses, removing nothing, if the set holds the catalog or a missing object, or if any object outside the set or the effective trailer (Info) still references one. Cycles inside the set may be deleted together. `allocate`, `replace_value` and `delete_objects` are now public, so tests and later page/annotation operations (M5/M6) can use them; nothing is published except through a writer.
- **Reachable-object collection.** `writer::rewrite_reachable` writes only objects reachable from the trailer's Root and Info, without renumbering, like the application's MuPDF save (`garbage=yes`). It is separate from ordinary preservation (`rewrite_document`, still used by the save-eligibility preflight). Every refusal still considers all objects, reachable or not, so signature, permissions, linearization and encrypted documents are refused exactly as before. Edits and saves now use collection: objects orphaned by copy-on-write (for example a page's indirect /Resources replaced by the page's private copy) or by deletion are not retained, and neither are objects that were unreachable in the input.
- **Delta history.** History no longer keeps one whole file per node. It keeps the original bytes, the current node's bytes and, per step, the objects that changed (value, and raw stream bytes only when they changed) or were removed. Undo/redo replays deltas onto the original and rewrites; the result must match the SHA-256 of the bytes the node published, or the move fails with `invalid-history` and nothing changes. `max_history_bytes` now charges the original, the current node and all deltas. Undo/redo/save-ticket semantics are unchanged, and the existing rollback tests pass unmodified.
- **Nested Forms.** Text in a Form placed by another Form is targetable when every Form on the chain except the innermost has its own /Resources. Editing copies each Form on the chain, from the edited one up to the one the page places. Each parent copy names its child copy under a fresh XObject name in a private copy of its resources, and the page gets one too. Shared or indirect resource dictionaries are never modified. A Form that inherits page resources, and annotation appearances, stay outside the edit path.

**Intentional difference.** The application's MuPDF path offers Form text only at depth 1 (`tests/p3-targets.test.mjs`: "the Fm1 nested in Fm2 is not" cowable). This engine also edits nested placements, which leaves every other placement unchanged. targets-streams.pdf page 1 therefore has 5 text targets instead of 4.

**Measured history costs** (`examples/history_cost_probe.rs`, release build, 20 edits per document; `history-costs.jsonl`). Whole-snapshot bytes are what the previous design retained for the same published nodes.

| Documents | Whole snapshots | Delta history | Max commit | Max undo/redo |
| --- | --- | --- | --- | --- |
| 18 editable fixtures | 13–119 KB | 3–45 KB (2.0–8.1× less) | 22.7 ms | 0.1 ms |
| 12.6 MB synthetic (12 MB metadata stream) | 264 MB | 25.2 MB (79 bytes per step) | 38.1 ms | 38.3 ms |

Under the default 64 MiB budget, the previous design refused the large document's fifth edit; it now reaches the 64-step limit. Undo/redo of a large document now costs a replay and rewrite (tens of milliseconds) instead of a pointer move.

Known growth: every edit of Form text allocates a new copy, and the previous copy stays named in the page's resources. edit-shared.pdf grows from 1.6 KB to 7.2 KB over 20 edits. Editing a page-owned copy in place is a possible follow-up.

Fixtures: 53 of 54 open, 51 fully render and 18 are editable, as before. All 74 rendered pages are byte-identical to the previous evidence. All 18 editable documents pass edit, save and fresh-process reopen, and independent MuPDF finds no changed pixels outside the edits and no repair. The nested edit (targets-streams.pdf, fourth target) passes the same independent check. Verification: 895 tests pass on Rust 1.98.1 and 1.85.1, with fmt and strict Clippy for core and bridge. The WASM build and desktop smoke pass. The WASM SHA-256 is `cc06b280be92b7a0fc0c23ebcb86c915f12011ee38706732e8ef370ff1a12a8a`. wasm-bindgen-cli 0.2.129 was reinstalled outside the repository because `.stage` had been removed. Evidence: `reference/m2-history-20261014/`.

Not yet in M2: copy-on-write for other resource kinds when later edits change them (fonts, images), annotation-appearance editing, and save-in-place integration.

## October 14, 2026 (night): copy-on-write editing (M2 first increment)

The document now supports transaction-local object allocation and value replacement (`Document::allocate`, `Document::replace_value`), and the writer serializes them. Text edits use this for copy-on-write:

- **Shared page streams.** A content stream that other pages also list in /Contents is copied for the edited page, and only that page's /Contents is repointed. Streams referenced any other way, or listed twice on one page, are still refused.
- **Form placements.** Text inside a Form placed directly by page content is targetable. Editing one placement allocates an edited copy of the Form. The page gets its own copy of its effective resources (inherited or shared dictionaries are left alone) with a fresh XObject name, and only that placement's `Do` is renamed. Other placements, other pages and the original Form are unchanged. This matches the application's `tests/drive/cow-edit.jsonl` expectation. Nested Forms and annotation appearances stay outside the edit path.
- **Edit validation.** Unaffected text is compared by font identity rather than registry index, because a copied Form has its own resource scope.

Editable documents: 14 → 18 (edit-objects, targets-streams, objects6, redact-shared-only). edit-shared.pdf's Form text is now editable too. All 18 pass edit, save and fresh-process reopen, and independent MuPDF finds no changed pixels outside the edits and no repair. All render hashes are unchanged. Verification: 887 tests pass on Rust 1.98.1 and 1.85.1, with fmt and strict Clippy for core and bridge. The WASM build and desktop smoke pass. The WASM SHA-256 is `4ba4279a1e1832fecbb8e034e253981943d4a2c59c430cad9b662594c184bdbe`. Evidence: `reference/m2-cow-20261014/`.

Not yet in M2: object deletion, reachable-object collection on save, efficient history (still whole-file snapshots), and copy-on-write for nested Forms, fonts and other resource kinds.

## October 14, 2026 (evening): encryption, layers, inline images, color spaces and generated appearances (M1)

Five rendering gaps from the remaining-failure list are closed.

- **Encryption reading.** The standard security handler is implemented in `src/security.rs`: R2–R4 (RC4-40, RC4-128, AES-128) and R5/R6 (AES-256). The user or owner password is accepted; a wrong or missing one reports `needs-password`. Strings, streams and object-stream containers are decrypted at open. Xref streams, the encryption dictionary and Metadata under `/EncryptMetadata false` stay as stored. Encrypted documents are view-only, as in the application, and the writer refuses them. `Session::open_view_with_password`, WASM `RustEngine.with_password` and the CLI's `TPDF_PDF_PASSWORD` take the password. Hashes and AES come from the RustCrypto `md-5`, `sha2` and `aes` crates (see DEPENDENCIES.md). Public-key handlers and named non-Identity crypt filters are refused.
- **Xref entry 0.** A free entry 0 is accepted whatever its generation. MuPDF's object-stream output writes 1-byte xref-stream fields that cannot hold 65535, and pdf.js and Acrobat accept them.
- **Layers.** `src/optional_content.rs` evaluates the default configuration: BaseState, ON, OFF, and OCMD /P and /VE. Hidden `BDC /OC` content, Forms, images and annotations paint nothing but still update graphics state. Layered documents are view-only; usage auto-states (/AS) are ignored.
- **Inline images.** `BI … ID … EI` parses as one operation. Data length comes from /L, else the unfiltered sample size, else the first delimited `EI`. Abbreviations and named color spaces are expanded, and the image goes through the same validated loader as XObjects. A missing ASCIIHex/ASCII85 end marker is tolerated, as in MuPDF and pdf.js.
- **cs/CS color spaces.** Device spaces, resource names for them, CalGray, CalRGB and ICCBased (by /N) set colors through sc/scn/SC/SCN. Cal and ICC values are drawn as device values without profile conversion. Indexed, Separation, DeviceN, Lab and Pattern colors stay refused.
- **Generated appearances.** `src/generated_appearance.rs` builds display-only appearances for annotations without one: text fields (single-line, multiline, password, quadding), combo and list boxes, check boxes, radio buttons, signature fields (MK decoration only) and Text notes. Layout constants are measured against MuPDF. With `/NeedAppearances true`, text and choice fields are regenerated from their values. The generated streams live in a cloned Document that saving never sees. Text uses built-in Helvetica or Times with WinAnsi characters. Push buttons and other annotation types without an appearance stay refused.

Fixtures: 48 → 53 of 54 open and 36 → 51 of 54 fully render. All 58 previously rendered pages are byte-identical. Editable documents go from 13 to 14 (targets-unsupported.pdf, now that its inline image renders). All 14 pass edit, save and fresh-process reopen, and independent MuPDF finds no changed pixels outside the edits and no repair. Compared with MuPDF at 72 DPI, the new pages are 97.4–99.98% identical. The two lowest are explained:
- **quartz.pdf:** antialiasing of a thin embedded font. Total ink agrees within 0.4%.
- **empty-sigfield.pdf:** an intentional difference. MuPDF draws its own orange "SIGN" marker on unsigned signature fields and hides their stored appearance. The Rust engine draws the stored appearance and no marker, like Acrobat and pdf.js.

The remaining first failures are edit-tj.pdf's deliberately unbalanced `q`, the large.pdf page-count limit and redact.pdf's stream-valued feature.

Verification: 886 tests pass on Rust 1.98.1 and on 1.85.1. fmt and strict Clippy pass for core and bridge. The bridge builds for wasm32, and a Node WASM smoke opens the AES-256 fixture with its password and renders. The desktop workflow smoke passes on hello.pdf. The WASM SHA-256 is `85672c0ec58e054acc42acfcc05a95623697d2209e86d4c021b017dcc788097f`. Evidence: `reference/m1-encrypt-20261014/` (view-admission.json, new-pages-vs-mupdf.json, edits.json, independent-oracle.json, msrv-tests.log, wasm-build.log, wasm-smoke.log, desktop/). New tools: tools/compare-mupdf-pages.mjs and tools/generate-encrypted-cases.mjs (writes the extra cipher cases in tests/data/).

## October 14, 2026 (later): CID fonts and composite glyphs render (M1)

Type0 fonts with the Identity-H CMap and an embedded TrueType CIDFontType2 descendant now render.

- **Font loading.** `/W` and `/DW` widths are parsed. Both group forms are supported, the default width is 1000, and overlapping ranges are refused. `CIDToGIDMap` may be Identity or a stream. A ToUnicode map with a two-byte codespace is required.
- **Text positioning.** Text runs in two-byte codes, `TextCode.code` is now u32, and `Tw` applies only to the single-byte code 32.
- **Composite TrueType glyphs.** These are supported for all TrueType fonts, not only CID fonts. Components may have offsets and uniform, x/y or 2×2 F2Dot14 transforms, with FreeType-style rounding. Nesting depth is capped at 16. Anchor-point matching is refused.
- **ToUnicode tolerance.** A ToUnicode entry with a malformed UTF-16 destination (a common producer bug, such as cjk.pdf's odd-length `<1044d>`) is now dropped instead of failing the whole map. Showing a dropped code is still refused, so wrong text is never produced.
- **Still refused.** Vertical writing (Identity-V), other CMaps, CFF-based CIDFontType0, and text editing in CID fonts. Editing needs two-byte replacement writing (M3).

Fixtures fully rendered: 31 → 36 of 54 (cjk, rtl, edit-embedded, edit-subset, edit-subset2). All 53 previously rendered pages are byte-identical, and the private-sample render hash is unchanged. Compared with MuPDF, the new pages are 99.41–99.62% pixel-identical, and visual inspection of the Hebrew/Arabic page shows matching glyphs. Its 48 pixels more than 64 levels apart are hairline antialiasing. The 13 editable documents still pass the independent edit check. Verification: 872 tests on 1.98.1 and 1.85.1, fmt, strict Clippy for core and bridge, and the desktop smoke. The WASM SHA-256 is `72d53ab59bb62497a0cf1a030d3c8e577d72b46f7c81fa1aad5a300325cf281e`. Evidence: `reference/m1-cid-20261014/`.

Remaining first failures (18 documents): 5 encrypted, 5 annotations needing generated appearances, 2 layers, and 1 each for NeedAppearances, redact.pdf's stream-valued feature, the page-count limit, an inline image, a `cs` color space and the deliberate unbalanced `q`.

## October 14, 2026 (later): marked-content property lists render (M1)

`BDC` and `DP` with property lists (MCID, ActualText, Artifact attributes) are now accepted as rendering no-ops, and BMC/BDC share one balance check. `BDC /OC` (optional content) stays refused until layer visibility is evaluated. Text edits on a page are withheld if its content has a property list ("Marked-content properties (ActualText, MCID) need structure-aware editing"). The page can still be viewed and saved unchanged. Other pages of the document stay editable.

Fixtures fully rendered: 28 → 31 of 54. All 50 pages that rendered before are byte-identical. The new pages (edit-tagged, edit-replace and paragraph page 2) are 99.15–99.89% pixel-identical to MuPDF, with no pixel more than 64 levels apart. Documents with editable targets: 11 → 13, because edit-replace.pdf and paragraph.pdf now render completely, which allows saving. All 13 pass the actual edit, save and fresh-process reopen. Independent MuPDF finds no changed pixels outside the edits and no repair. Verification: 869 tests on 1.98.1 and 1.85.1, fmt, strict Clippy for core and bridge, and the desktop smoke. The WASM SHA-256 is `21a39cf3c7b7fed9bf9e44b6fd0ee1a8c74d7744139b871cd3f14765916d5bae`. Evidence: `reference/m1-bdc-20261014/`.

Remaining first failures (23 documents): 5 CID/Type0 fonts, 5 encrypted, 5 annotations needing generated appearances, 2 layers, and 1 each for NeedAppearances, redact.pdf's stream-valued feature, the page-count limit, an inline image, a `cs` color space and the deliberate unbalanced `q`.

Known limit: a page that draws annotations is not text-editable yet. Its operation indices include the annotation and page-scope boundaries, and the edit path still rejects any Form invocation. Mapping edits through origins is M3 work.

## October 14, 2026 (later): annotation and form appearances render (M1)

Pages now draw their annotations' normal appearance streams after the page content. Documents with form fields or XFA open in view mode instead of being refused. Editing and saving them stays unavailable with reason `unsupported-forms` until form values and appearances can be preserved transactionally (M6). Fully rendered fixture documents went from 16 to 28 of 54, and all 35 pages that rendered before are byte-identical.

- **Placement.** ISO 32000 Algorithm 8.1: the appearance BBox, transformed by its Matrix, is fitted to the annotation Rect, and drawing is clipped to the BBox. `/AS` selects the state stream from an `/N` dictionary. A state with no stream draws nothing. Appearance streams without `/Subtype /Form` are accepted, because the context already makes them Forms.
- **Isolation.** When a page has annotations, its content runs inside a synthetic q/Q (`OperationOrigin::PageScope`). Each appearance therefore starts from the initial graphics state, whatever CTM, colors or clip the content leaves at depth zero.
- **Skipped.** These draw nothing: Hidden or NoView flags, zero-area Rects, popups, borderless links, and text-field widgets with an empty or absent value and no `/MK` border or background. MuPDF draws nothing for any of these.
- **Refused, per page.** Annotations with `/OC` are refused. So are other annotations without an appearance stream (checked checkboxes, list boxes, unsigned visible signature fields, note icons): viewers synthesize these, and generating them is M6. `NeedAppearances true` is still refused, because the stored streams are not authoritative.
- **Deliberately unlike MuPDF.** A widget outside the AcroForm field tree is drawn. Acrobat and pdf.js draw these, and MuPDF hides them. MuPDF is a comparison oracle, not the target behavior.

Oracle evidence is in `reference/m1-forms-20261014/oracle/`. It holds a synthetic page with a rotated Matrix and offset BBox, a leaked page `cm`, a unit BBox scaled 60×, an `/AS` state dictionary and a stream without Subtype. Against MuPDF, no pixel differs by more than 64 levels. Most of the rest is the existing ±1 gray rounding. The 15 newly rendered fixture pages are 99.84–99.96% pixel-identical to MuPDF, and the remaining differences are text edge antialiasing. Per-annotation MuPDF pixel contributions are recorded as well.

Verification: 868 tests pass on Rust 1.98.1 and 1.85.1. fmt and strict Clippy pass for the core and the bridge. The WASM rebuild SHA-256 is `835ca5d08582740b3a821fbcdc621bd622434f235aacfb7799170a552adefad6`. The desktop smoke passes with the same before and after hashes as M1. The fixture sweep is 48/54 opened, 28/54 fully rendered and 11/54 editable.

First failures for the 26 documents that are still not fully rendered, one per document:

| Count | First failure |
| --- | --- |
| 5 | CID/Type0 fonts |
| 5 | Encrypted |
| 5 | Annotations needing a generated appearance |
| 3 | `BDC` |
| 2 | Layers |
| 1 each | NeedAppearances; stream-valued feature (redact.pdf); page-count limit; inline image; `cs` color space; deliberate unbalanced `q` |

## October 14, 2026: private-sample.pdf opens and renders (M1)

The user's real target document now opens in view mode and renders its whole page in both native Rust and Rust WASM. The two renders are pixel-identical, and WASM open plus 72-DPI render takes about 0.2 s. Against MuPDF 1.28.1 at 72 DPI, 85% of pixels are identical and 98.5% are within 64 levels. The remaining differences are glyph and barcode edge antialiasing plus the substituted Arial outlines. Side-by-side output and dash-phase crops are visually identical. This is a visual and statistical comparison, not a certified pixel gate. Text editing on this page stays deliberately unavailable ("display substitute font; portable editing requires embedding"), so M3's private-sample edit remains open. The document's contents are not copied into fixtures, evidence or archives.

Four blockers were fixed in order. Each has regression tests:

- **Image `/Intent`.** The four standard intent names are accepted on image XObjects. Nonstandard names are refused. Every admitted color space is device or matrix/gamma, so intent cannot change the output. MuPDF renders Perceptual and RelativeColorimetric identically for these spaces. The content-stream `ri` operator still admits only RelativeColorimetric. Widening it would follow the same reasoning.
- **CalRGB images.** These are converted to sRGB with exact double-precision math: per-channel gamma, the PDF Matrix, Bradford adaptation from the declared WhitePoint to D65, then the sRGB transfer curve. On a synthetic CalRGB gradient the result is within one level per channel of MuPDF/LittleCMS. Raw samples are kept unchanged and conversion happens per sampled texel. A nonzero BlackPoint, a WhitePoint whose Y is not 1, nonpositive X/Z, Gamma outside (0, 16], and malformed arrays are refused. CalRGB in content-stream `cs`/`CS` is still unsupported.
- **Tag-only marked content (`BMC`/`EMC`/`MP`).** This is admitted as a rendering and text-position no-op. Page preflight enforces BMC/EMC balance per content stream, within the save-depth limit. `BDC`/`DP` property lists (ActualText, MCID, optional content) remain refused.
- **Dashed strokes.** The user-space centerline is cut into "on" pieces, and each piece goes through the existing certified stroke expander with the configured caps and joins. Odd arrays repeat, and the phase restarts on every subpath. ClosePath dashes its closing line. Curves are split exactly with de Casteljau at chord-length positions (32 samples per cubic). Dashes at a closed subpath's seam are capped separately rather than joined. Negative or all-zero arrays are refused, as are zero-length dashes with round or square caps (cap-only dots).

A fifth blocker appeared only at 72 DPI. Capability probing renders at 18 DPI, where it is not visible. The page rasterizer kept one full-page 4× supersampled mask per clip, and private-sample's roughly 70 small clips exceeded the 256 MiB mask budget. Clip masks are now stored only within each clip's sample-grid bounds, intersected with the parent's, and only that area is charged. All 35 fixture pages that rendered before still produce byte-identical 72-DPI output.

Verification: 863 Rust tests pass on Rust 1.98.1 and on the minimum 1.85.1, with no failures. Strict all-target/all-feature Clippy and formatting pass for the core and the bridge. The rebuilt WASM SHA-256 is `41bce21e5d6df772808e90830d195a2a3f0bc06f66c24ccaeaddf531e1514cee`. The sandboxed Electron workflow smoke on hello.pdf passes edit, save, reopen, second edit, all worker-failure cases, and the Node-global checks. The fixture view sweep is unchanged at 48/54 page trees, 16/54 fully rendered and 11/54 editable. None of the fixtures' first failures was one of the newly fixed features. Evidence is in `reference/m1-20261014/`: test logs, Clippy, view-admission.json, the desktop report, and private-sample-render.json with the input hash, render hashes, MuPDF diff histograms and CalRGB oracle values.

Here are the first failures for the 38 fixture documents that are not fully rendered, counting one per document: 18 nonempty forms or XFA, 5 encrypted, 5 CID/Type0 fonts, 3 `BDC` marked content, 2 layers, and 1 each for a stream-valued feature (redact.pdf), the page-count limit, an inline image, a `cs` color space, and the deliberately unbalanced `q` page in edit-tj.pdf.

## October 12, 2026 late session (recorded retroactively)

A session after the feature-gate follow-up made the following changes without updating this file. Its evidence is in `reference/m1-20261012/` and `reference/m3-20261012/`, and its final run was 857 tests on both toolchains.

- **M1 view/edit split.** `Session::open_view` and per-page capabilities report render and edit errors without refusing the whole document. Signed documents are view-only.
- **Rendering features.** Explicit RelativeColorimetric `ri` and default flatness are accepted. Indexed DeviceRGB images with 8-bit lookup and binary color-key masks are supported.
- **Display-only Arial substitution.** Exact-name Arial and ArialMT regular, bold and bold-italic are drawn with pinned Helvetica outlines that keep the PDF's advance widths. The outlines are generated by tools/generate-substitute-outlines.py, and their asset hashes are in m1-20261012/font-assets.json. Pages that use a substitute are never editable.
- **M3 edit coverage.** TJ-array runs and content-array member streams can be replaced and saved. All 11 editable fixtures pass independent MuPDF edit checks (m1-20261012/independent-edits.json).

## Original October 12 workflow (historical)

A working native and Rust WASM editing workflow now exists on top of R24. The experimental desktop editor can open an eligible PDF, select existing text, replace it, undo and redo, Save As a new copy, adopt its filename and file identity, reopen that copy, and make and save another edit. The installed Tandem PDF application is unchanged. This is a functional preview for a restricted PDF subset, not completion of the full replacement roadmap.

## Implemented

The session owns bounded immutable PDF snapshots. Each edit changes a transaction-local decoded stream, serializes with the existing strict writer, reparses, checks text and unaffected text positions, and renders before publishing one history node. The writer resets stream Length and removes stale Filter and DecodeParms. Failed edits preserve content, dirty state and redo. Generation and revision reject stale targets; unique history nodes distinguish branches at the same undo position. Exact save receipts prevent a delayed disk write from marking later edits clean. Saving and undo remain available after the editing work budget is exhausted.

Existing horizontal simple-font Tj strings and TJ-array runs can be replaced or deleted, including in content-array member streams. Pinned Helvetica and Times-Roman use proven canonical glyphs and declared mappings for new characters. Embedded fonts currently use the proven on-page repertoire. Unchanged text positions, source resource identity and stream ownership are checked. Existing encoded, escaped and compressed strings are handled. Shared streams, Form text edits and paragraph edits remain outside this edit path.

The WASM bridge executes in a sandboxed Electron worker with no MuPDF runtime fallback. The separate editor uses the existing application's safe-write module, filesystem dialogs, validation before write, and save tickets. Cancelled and failed saves preserve dirty state. A 60-second worker timeout terminates the worker rather than allowing an unobserved late edit; unsaved changes in that worker are then unavailable. This is a deterministic failure policy, not a recovery or responsiveness guarantee.

## Verification

- The original checkpoint passed 829 Rust tests on Rust 1.98.1 and minimum Rust 1.85.1. The feature-gate follow-up passes 836 tests on Rust 1.98.1 and all 22 session tests on Rust 1.85.1; the full minimum-toolchain suite was not rerun for this follow-up.
- Core and bridge formatting and strict Clippy pass. The release WASM library and JavaScript bindings build and execute in Electron. The bridge's declared minimum toolchain is checked separately in its evidence log.
- The desktop smoke test uses the actual controls for selection, text entry, undo, redo, save and reopen. It verifies identical pixels after undo and after fresh reopen, performs a second saved edit, and checks stale targets, invalid open, cancelled save, injected disk-full, and a late save receipt. Both renderer and worker report no Node globals. Native save dialogs are bypassed only in smoke mode; interaction is automated, not a human usability review.
- All 8 fixture documents with supported targets passed an actual edit, save and fresh-process reopen. Independent MuPDF 1.28.1 reopened every output without repair, found the replacement text, and found zero changed pixels outside the edited region across all pages.
- The original workflow admission sweep opened 13 of 54 fixture documents; the current feature-gate sweep opens 16 of 54, with 11 documents exposing editable targets. All 11 pass an actual edit/save/fresh-process reopen and independent MuPDF text/pixel checks. This is separate from R24's much broader rendering-only page count; the workflow refuses unsupported writes, visible annotation appearances, nonempty forms, optional content and other gaps up front. Borderless links are preserved without active link navigation.
- Existing filesystem save tests: 22 pass, 29 Electron-spawning tests skipped for this focused run; zero failures. The new Electron workflow test runs separately.
- All 141 reference files in the accepted R24 archive manifest remain byte-identical. Existing production changes are confined to engine/src/document.rs and engine/src/lib.rs; new modules, bridge, preview and tests are additive. The full historical R24 differential/mutation replay was not rerun and is not claimed as new evidence.

This checkpoint has automated regression and independent-consumer evidence. It has not had a separate independent implementation review and does not supersede R24's formally reviewed acceptance status.

## October 12 follow-up: document feature gates

The blanket scan for Annots, AcroForm and OCProperties was replaced with bounded inspection of the live catalog and page dictionaries. Empty form field arrays, including indirect arrays with default appearance/resources, now survive open/edit/save/reopen. Borderless Link annotations without appearance streams are preserved; border-style precedence and omitted-width defaults are checked. Visible annotations, nonempty forms, XFA and layer settings remain explicitly refused with distinct messages. Tagged structure trees do not expose editable targets until associated replacement/accessibility text can be updated safely.

private-sample.pdf in the user's Downloads folder reproduced the reported failure: an AcroForm with an empty Fields array and no page annotations. That gate is fixed, but the original document still fails rendering at the RelativeColorimetric rendering-intent operator. It also uses unembedded Arial regular, bold and bold italic fonts, which the current outline policy does not substitute. (Superseded: the October 12 late session fixed the rendering intent and Arial substitution, and the October 14 work above makes the page render.) Its original hash is unchanged; its contents have not been added to fixtures or recovery archives.

Validation: 836 Rust tests and strict Clippy pass; 22 session tests pass on the minimum toolchain. Rebuilt native and WASM engines. The sandboxed Electron workflow passes twice through edit/save/reopen with synthetic empty-form metadata. Independent MuPDF validation confirms saved form metadata and link actions/destinations are preserved, outputs reopen without repair, and zero pixels outside the edited line change. Evidence is under the main workspace's .stage/workflow-gates. Prior R24 and workflow recovery checkpoints remain unchanged.

## Run the preview

The app itself now runs the Rust engine: `npm start` builds it (`engine/web/pkg` must be current; build it with `tools/build-workflow.sh`) and launches it. The preview below is the separate October 12 editor on the preview Session.

From the main Tandem PDF workspace, with its Node dependencies and build/safe-write.js available:

```sh
engine/desktop/run.sh
```

From another directory, set TPDF_APP_ROOT to the application workspace. The preview uses the generated WASM files under engine/web/pkg and launches its own Electron process. It neither replaces nor launches the installed Tandem PDF bundle. The tested target is macOS Apple Silicon; Windows and Linux preview execution and packaging remain unverified.

The default editing limits are 16 MiB per input/output PDF, 64 pages, 64 undo steps and 64 MiB retained snapshot bytes. Target discovery admits at most 8,192 placed codes, 10,000 operations, 256 targets and 1 MiB decoded page content. These limits do not describe total process RSS. Save As uses the existing Save Copy path to protect the original, then adopts the new file as the current document before acknowledging the saved snapshot. Text replacement is not redaction and may leave old data in otherwise preserved objects.

## Rebuild

The bridge pins wasm-bindgen 0.2.129 and serde_json 1.0.151 in a separate crate and lockfile. The core now also depends on hayro 0.8 (vendored with patches under `vendor/`), serde_json and png, all pinned with `=` and recorded in DEPENDENCIES.md. hayro raises the minimum toolchain to 1.92, so the two toolchains are 1.98.1 and 1.92. Install the wasm32-unknown-unknown Rust target and the matching wasm-bindgen-cli, then run tools/build-workflow.sh. Set CARGO_BIN, WASM_BINDGEN_BIN and CARGO_TARGET_DIR when using workspace-local tools. The acceptance runs used the generator at `$HOME/.local/tpdf-wasm-tools/bin/wasm-bindgen`; `.stage/wasm-tools` no longer exists. `tools/build-workflow.sh` writes `web/pkg/sources.sha256`, and `scripts/build.mjs` refuses a package built from other sources. Source forbids unsafe Rust; generated glue and dependencies may contain internal unsafe code. AGPL and asset attribution are retained.

The native CLI is tandem-rust-workflow in the selected target's release directory. It supports inspect FILE and edit FILE PAGE TARGET TEXT OUTPUT, with zero-based page and target indices. It creates a new output path and refuses an existing one.

## Remaining work

The app engine now covers the whole worker contract, and 64 of its 67 operations are accepted (October 16 above). What is left, all waiting on the owner's decision (`../docs/rust-engine-completion-plan.md`):

- a third fix round for `open`, `redact-preview`, `redact-apply` and the integration blocker;
- M9: recovery for long operations, mobile on MuPDF, the release engine switch (releases and CI build MuPDF) and the licence wording of a Rust build;
- M10: a fixed benchmark set, fuzzing and mutation campaigns, and packaged Windows and Linux builds.

The preview Session's own open items (editing with substitute fonts for the private-sample file, copy-on-write of fonts and images) are listed in the plan; the app engine does not depend on them.

Evidence is retained under reference/app-acceptance-20261016, reference/corpus-20261015, the app work's reference/app-*-20261014 and app-*-20261015 folders and reference/text-unicode-20261015, and for the preview under reference/m2-inplace-20261014, reference/m2-history-20261014, reference/m2-cow-20261014, reference/m1-encrypt-20261014, reference/m1-cid-20261014, reference/m1-bdc-20261014, reference/m1-forms-20261014, reference/m1-20261014, reference/m1-20261012, reference/m3-20261012 and reference/workflow-20261012-final. Frozen R24 evidence remains intact. The recovery checkpoints were deleted in the October 14 disk housekeeping (STATUS.md); git history holds `engine/` since commit db942f5 on `rust-engine-app` (79d295a, the same commit before a rebase, is on no branch).
