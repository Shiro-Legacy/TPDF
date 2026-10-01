# Bates rerun (after E-R1/E-R2, B-R1..3, T-R1/T-R2/T-R4 fixes) — 2026-09-24

Logs: .quad/scratch/bates/fix-rerun/ (build.log, test.log, *.log, codes.txt, hash-after.txt)

1. npx tsc --noEmit -p tsconfig.json: clean (exit 0).
2. npm run build: exit 0. npm test: 553 tests, 552 pass, 0 fail, 1 skip
   (skip = #395 "legacy p12 parses when rc2-40-cbc exists", environment-gated, pre-existing).
3. Drives (all in one lock hold):
   - tests/drive/bates.jsonl x3: exit 0, 0, 0
   - tests/drive/bates-pad.jsonl (new, T-R2) x3: exit 0, 0, 0
   - x1: bates, dialogs, refusals, help-fields, direct-edit8-chords, replace-page,
     direct-edit3, direct-edit8-cut, object-edit, content-edit, debt1: all exit 0
   - Red steps: none.
4. shasum -a 256 -c .quad/shared/p6/checks/fixtures.sha256: 9 OK.
   Full list (55 pdf/png under fixtures/) byte-identical to checks/bates-hash-before.txt;
   file set identical too.
5. pgrep -fl -- '--drive=': empty. No Electron stragglers; nothing killed.

Not run (per brief): .quad/shared/p6/checks/shot-bates.mjs.
src/ untouched by this rerun. Open defects: none.
