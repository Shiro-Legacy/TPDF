# Debt sweep 2 — rerun after review fixes (2026-09-24, Opus 5.5 rerun seat)

Scope: E-R1/E-R2/E-R3 (engine), R-B1 (renderer, step="any"), R-S1 (tests, shot-debt2.mjs case B). Nothing in src/ touched by this seat.
Artifacts: .quad/scratch/debt2/fix-rerun/ (run.sh, test.out, summary.txt, drive-*.log, rep-*.json).

1. npx tsc --noEmit -p tsconfig.json — clean (no lock).
2. One lock hold: npm test (build + node --test tests/*.test.mjs) — exit 0; tests 544, pass 543, fail 0, skip 1 (U8 baseline 536/535/0/1 + 8 new).
3. Same hold, headless drives, every report ok:true, 0 red steps:
   - direct-edit8-chords.jsonl ×4 (3 required + 1 in the list): 19/19 each, exit 0
   - direct-edit8-cut 51, direct-edit8 115, direct-edit8-save 42, direct-edit9 43, direct-edit3 79,
     direct-edit7 66, direct-edit6 59, object-edit 11, content-edit 4, help-fields 21, dialogs 11, refusals 29 — all exit 0
4. shasum -a 256 -c checks/fixtures.sha256 — 9 OK. checks/debt2-hash-before.txt — 55/55 OK, and the fixtures/ PDF+PNG list is identical (no added/removed files).
5. pgrep -fl -- '--drive=' — empty.

Not run (per brief): .quad/shared/p6/checks/shot-debt2.mjs (real keystrokes). Its case B rewrite (R-S1) is therefore still unproven on the GUI;
the renderer fixer also notes review-debt2-renderer-probe.mjs:296 and :319-321 still compare rounded STATE box against unrounded placement (probe bug, tests owner).

Open defects: none red in src/. Outstanding verification: shot-debt2.mjs GUI run + PNG view, and `npm run package` (contract §5).
