# Debt sweep 1 — integration (2026-09-24, Opus 5.5)

## Requests applied
None: engine, renderer and tests reports all had empty `requests`.

## Suite
- `npx tsc --noEmit -p tsconfig.json`: clean.
- `npm test` (build + node --test tests/*.test.mjs, under lock): 466 tests, 465 pass, 0 fail, 1 skipped (baseline was 453/452/0/1, so 13 new tests, all green).

## Drives (node scripts/drive.mjs, under one lock hold)
| script | runs | exit codes |
|---|---|---|
| debt1.jsonl | 3 | 0 0 0 |
| refusals.jsonl | 3 | 0 0 0 |
| nonascii-label.jsonl | 3 | 0 0 0 |
| sign-field.jsonl | 1 | 0 |
| direct-edit3 / direct-edit4 / direct-edit5 | 1 each | 0 / 0 / 0 |
| links-bookmarks / field-design / help-fields | 1 each | 0 / 0 / 0 |
No red steps.

## Fixture hashes
- `shasum -a 256 -c .quad/shared/p6/checks/fixtures.sha256`: all OK (this includes the new p3/field-defs.pdf line).
- `debt1-hash-before.txt` (53 files): all OK. Comparing the fixtures/**/*.pdf|png list with that file finds one new file, fixtures/p3/field-defs.pdf, which the table allows.

## GUI shots (debt1 and sign-field were green, so both ran)
- shot-debt1.mjs: exit 0, "PASS all 10 checks". A: handles show on .selected and stay hidden on the others. B: a 2 px drag leaves revision 0; a 6 px drag moves Im1 from x 72 to 76.59. C: a double-click replaces Im2. D: the add editor grows 125.49 → 241 px and its font is 15.69 px (12 pt × 1.307). Row 18: the Save As button reads "Save As…". Row 3: "This page has 6 form fields." on page 1 and "3" on page 2.
  PNGs in .quad/shared/p6/checks/review-out/: debt1-a-selected-handles, debt1-b-drag-threshold, debt1-c-dblclick-replace, debt1-d-add-editor, debt1-e-field-count-p1, debt1-f-field-count-p2.
- shot-sign-u3.mjs: exit 0, "PASS draw", "PASS signed", "PASS all". Badges in view: 1 of 2 in C and 1 of 2 in D (the second signed field is below the fold at fit-width).
  PNGs in .quad/shared/p5/checks/review-out/: u3-a-drawn-field, u3-b-sign-dialog, u3-c-signed-badges, u3-d-valid-badges, u3-d-verify-dialog.
- Afterwards `pgrep -fl -- '--drive='` was empty and no Electron with a review-out or tmp user-data-dir was left.

## Applied fixes
None were needed: nothing went red.

## Open defects
None.

## Rows not proven by a failing-then-passing check
- 5: orchestrator housekeeping (the scratch purge), done before the build.
- 9: deferred (79 steps pinned; §5 forbids inserting lines).
- 17: skipped (not wanted).
- 20: code-only (`pageIds = null` in the catch). A dead worker cannot be reached from a drive, so the only evidence is tsc plus green drives.
- 2 and 16a: these pin behaviour that already worked, so they were green on the pre-fix build by design.
- 8, 16d, 21, 22, 23: doc and comment changes, proved by grep. The grep for a bare `contract-p3-unit` citation in src/ and tests/ finds none.
- 10: the check lines pass. The orchestrator still has to view every PNG at acceptance.
- Note: contentReplaced, contentLastAdd and contentLastShift still carry over from the previous document after an open. This is outside row 19's scope and is a candidate for §10.
