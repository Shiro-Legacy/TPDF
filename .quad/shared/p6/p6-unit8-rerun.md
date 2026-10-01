# P6 U8 rerun (after review fixes: engine N2, renderer R8-1/R8-3, tests T8-N2)

Date 2026-09-24. Rerun agent Opus 5.5. No changes to src/ or tests/. Logs are in .quad/scratch/p6u8/fix-rerun/.

1. `npx tsc --noEmit -p tsconfig.json` (no lock): clean, exit 0.
2. Under one lock hold: `npm run build` exit 0, `npm test` exit 0. 536 tests, 535 pass, 0 fail, 1 skip. The skip is the known #387 "legacy p12 parses when rc2-40-cbc exists", which needs OpenSSL legacy support.
3. Drives, all in the same lock hold. Every one exited 0 with no red step:
   - direct-edit9.jsonl, run 3 times: 0/0/0, 43 steps each
   - direct-edit.jsonl: 0, 75 steps
   - direct-edit3.jsonl: 0, 79 steps
   - direct-edit8.jsonl: 0, 115 steps
   - direct-edit8-cut.jsonl: 0, 51 steps
   - content-edit.jsonl: 0, 4 steps
   - debt1.jsonl: 0, 22 steps
   - object-edit.jsonl: 0, 11 steps
   - help-fields.jsonl: 0, 21 steps
4. `shasum -a 256 -c checks/fixtures.sha256`: 9 OK. Re-hashing all 55 files listed in checks/u8-hash-before.txt gives output identical to that file.
5. `pgrep -fl -- '--drive='` finds nothing, so there are no stragglers.

Red remaining: none.
Open, carried to progress.md §10: the engine half of R8-3. After a text-add is undone, the next text-add's /PDF4AllF1 font points at a freed object (src/renderer/engine.ts:5374-5381, the `PDF4AllF${index}` allocation in text-add). The run gets no on-page target and draws in a fallback font. The renderer now announces "Added text is not editable on the page" in this case.
Not run, as the brief says: shot-direct9.mjs.
