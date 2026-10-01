# Acceptance — P6 U4 (links on the page, /Tabs order, plain-arrow nudge, Edit outlines, unarmed redact drag)
Orchestrator: fable51 (Claude Fable 5.1). Date: 2026-09-24. Mode: lean (progress.md §5). Human approved contract §9 (24 amendments) before the build.

## Pipeline
- Baseline before build: `npm test` 420 / 419 / 0 / 1. Backup `.quad/backup/pre-p6u4-20260924.tgz`; hashes `checks/u4-hash-before.txt` (50 files).
- Build workflow `wf_4f424e1f-9ec` (`scripts/p6u4-build.js`, 5 Opus 5.5 agents: fixture → engine ∥ renderer ∥ tests → integrator). Integration green first pass: 436/435/0/1; direct-edit4 ×3 and direct-edit5 ×3 exit 0; six regression drives exit 0; hashes 6/6 + 50/50; 8 GUI shots.
- Backup before review fixes: `.quad/backup/pre-p6u4-review-20260924.tgz`.
- Review workflow `wf_29cc1e85-7aa` (`scripts/p6u4-review.js`, 3 reviewers + 3 fixers + 1 rerun, all Opus 5.5):
  - engine+fixtures ACCEPT — E-1 (should, fixed): `schemeOf` trimmed before the allowlist; padded URI reached mupdf untokenised. E-2 (nit, carried): `/K` arrays count as depth levels.
  - renderer-ux REVISE — R1 (should, fixed): `dropDirect` link commit not silent (§9.6). R2 (should, fixed): real mouse press on Undo/Redo blurred and committed an open link editor. Both proven by CDP probe `checks/review-u4-renderer-probe.mjs` (b20, b26 BAD → OK).
  - tests/drive ACCEPT — S1 (should, fixed): +6 direct-edit5 steps for mark nudge and armed Mark-area threshold/success. N1–N3 nits carried to progress.md §10.
  - Rerun (`p6-unit4-rerun.md`): 436/435/0/1, all drives 0 red, hashes OK, `shot-direct4.mjs` PASS all, no stragglers.
- fable51 follow-up: R1 drive coverage (direct-edit4 steps 26–31, pinned count 65 → 71; bite proof: flipping `commitLinkEditor(active, true)` → `false` in built `renderer.js` makes steps 30–31 red with the refusal text; rebuilt → green ×3).

## Acceptance run (`scripts/accept-u4.sh`, patched per §9.21, one lock hold; log `accept-u4-run.log`)
- tsc clean; build ok; `npm test` 436 / 435 / 0 / 1 (skip = legacy RC2).
- direct-edit4 ×3: exit 0, 71 steps, 0 red. direct-edit5 ×3: exit 0, 35 steps, 0 red.
- direct-edit 75, direct-edit2 50, direct-edit3 79, links-bookmarks 15, field-design 23, help-fields 21: all exit 0, 0 red.
- `shasum -c checks/fixtures.sha256` 6/6 OK after the drives; `u4-hash-before.txt` diff: nothing changed or missing; added only `fixtures/p6/links.pdf` (`0ac413b0…`) and `fixtures/p6/tabs.pdf` (`b90c5db2…`).
- Electron: no `--drive=` processes.

## Orchestrator checks
- All 8 GUI shots viewed (`checks/review-out/u4-{a-link-handles,b-link-moved,c-link-editor,c-link-retargeted,d-tab-b1,e-edit-outlines,f-im2-nudged,g-redact-drag}.png`): handles + blocked L3 dotted; L1 moved; inline URI editor; Tab lands on b1; outlines without hover; Im2 nudged; one red mark. `c-link-retargeted` is byte-identical to `b-link-moved` (nit N2).
- Real CDP gesture rerun by fable51: `node checks/shot-direct4.mjs` → PASS all (`u4-cdp-rerun.log`), hashes 6/6 after, no stragglers.
- `npm run package` exit 0 → `out/PDF4All-P0-darwin-arm64/PDF4All-P0.app` (asar 2026-09-24 11:27; electron v44.4.0 from the checksum-verified local install, `u4-package.log`).

## Process notes
- Both workflow scripts were patched before launch to match §9.20/§9.22 (engine does not touch `p2-shell`; design doc gets an appended section) and the reviewer precedent pointer was repointed to files that exist.
- The Help-rows probe "h1 BAD" noted by the renderer fixer was a probe over-strict assert; the reviewer confirmed `helpRows()` = 11.
- Field bar wording "This document has N form fields" counts the current page (pre-U4); logged in §10.

Verdict: ACCEPT.
