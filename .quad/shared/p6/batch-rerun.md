# Batch rerun (after the engine, renderer and tests fixers) — 2026-09-25
Logs: .quad/scratch/batch/fix-rerun/ (summary.txt, npmtest.log, <drive>.log/.err, run.sh)

1. npx tsc --noEmit -p tsconfig.json: clean (exit 0).
2. One lock hold: npm run build ok; npm test exit 0. 564 tests: 563 pass, 0 fail, 1 skipped (previously 563/562/0/1, +1 = p2-shell 'Batch review R1-R6').
3. Drives in the same hold (batch runs on fresh mktemp /tmp/pdf4all-accept-* staging with batch-in/ + batch-c/, as accept-unit.sh does):
   - batch.jsonl x3: exit 0/0/0, 7/7 steps, report ok true each
   - bates 0 (9 steps), bates-pad 0 (5), dialogs 0 (11), refusals 0 (29), help-fields 0 (21),
     direct-edit8-chords 0 (19), replace-page 0 (7), direct-edit3 0 (79), object-edit 0 (11),
     content-edit 0 (4), debt1 0 (22)
   - No red steps.
4. shasum -c checks/fixtures.sha256: 9 OK. shasum -c checks/batch-hash-before.txt: 55/55 OK, and the
   current fixtures/*.pdf + *.png list is identical to it. find fixtures -name pdf4all-out: empty.
5. pgrep -fl -- '--drive=': empty; no review-out user-data-dir stragglers.

Not run (per brief): .quad/shared/p6/checks/shot-batch.mjs, including the T-R6 second-Start block, which has only had node --check.
Open (contract wording, not red): engine stamp-error detail 'save' (engine.ts batchFile) is not listed in contract §9.4.
The renderer row reads 'stamp-error save'. fable51 decides whether §9.4 lists it.
Nothing in src/ was changed.
