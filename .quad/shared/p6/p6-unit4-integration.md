# P6 U4 integration (Opus 5.5 integrator, 2026-09-24)
Inputs: contract-p6-unit4.md §0, §5–§9; evidence p6-unit4-{fixture,engine,renderer,tests}.md. All runs held .quad/run.lock (no stale lock was found).

## Requests applied
- None. No report had a `requests` item, and no drive, test or GUI file needed a fix.

## Suite
- `npx tsc --noEmit -p tsconfig.json` exit 0.
- `npm test` (build + node --test tests/*.test.mjs) exit 0: 436 tests, 435 pass, 0 fail, 1 skip. The skip is "legacy p12 parses when rc2-40-cbc exists" (pre-existing).
- tests/p6-drive4.test.mjs ran inside the suite and passed (both tests, not skipped).

## Drives (node scripts/drive.mjs; exit code, steps, red steps)
- direct-edit4 x3: 0/0/0, 65 steps, 0 red each run
- direct-edit5 x3: 0/0/0, 27 steps, 0 red each run
- direct-edit 0 (75, 0 red); direct-edit2 0 (50, 0); direct-edit3 0 (79, 0)
- links-bookmarks 0 (15, 0); field-design 0 (23, 0); help-fields 0 (21, 0)
- Reports: scratchpad only (not kept in the repo).

## Fixture hashes
- `shasum -a 256 -c .quad/shared/p6/checks/fixtures.sha256`: 6/6 OK before the suite, after the drives and after the GUI run.
- u4-hash-before.txt: 50/50 OK each time. The only fixtures/**/*.pdf|png files not in the baseline are fixtures/p6/links.pdf and fixtures/p6/tabs.pdf (allowed).

## GUI (node .quad/shared/p6/checks/shot-direct4.mjs): PASS all
- Shots in .quad/shared/p6/checks/review-out/: u4-a-link-handles, u4-b-link-moved, u4-c-link-editor, u4-c-link-retargeted, u4-d-tab-b1, u4-e-edit-outlines, u4-f-im2-nudged, u4-g-redact-drag (.png).
- Viewed u4-a (L1 grip and 8 handles in Annotate, L3 dotted as blocked) and u4-g (one red mark in Redact, one Comments row).
- Real input results: Im2 plain → [401,700,461,740]; Redact drag [300,500.14,400,530.15].
- Afterwards `pgrep -fl -- '--drive='` and `pgrep -fl u4-udata` were empty. No other Electron process was touched.

## Open defects
- None found by this run.
- Carried from the renderer seat (§9.6): dropDirect commits a changed link editor through the Enter path, so a refusal on a mode or page change shows in #error instead of being silent. renderer.ts (dropDirect link branch). This needs a fable51 ruling: it is not a drive red, so it was left unfixed.

## Notes
- memory says U4 needs human approval before the build. This integrator worked from the orchestrator's brief and did not see that approval.
- The relayed user request (a 1M context window instead of 265k) is a session setting that no seat can change. The seat reports model claude-opus-5-5[1m].
