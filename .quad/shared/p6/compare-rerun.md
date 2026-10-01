# Compare rerun (2026-09-25, Opus 5.5 rerun agent, after the engine/renderer/tests fixers)

Nothing in src/ was edited. Logs: .quad/scratch/compare/fix-rerun/ (run.sh, build.log, test.log, drive-*.log, summary.txt).

1. `npx tsc --noEmit -p tsconfig.json` (no lock): clean, exit 0.
2. Under one lock hold, `npm run build` exited 0 and `npm test` exited 1: **575 tests / 573 pass / 1 fail / 1 skip**.
   - The failure is `tests/p3-edit.test.mjs:875`, 'U3 review: /W expansion is bounded to 2-byte codes and to the CMap code ceiling'. The assert at :888, `performance.now() - started < 2000` ('the scan does not expand codes that cannot exist'), failed: the test took 2501 ms under full-suite load.
   - I reran it alone under the lock 3 times: 3/3 pass at about 420 ms each. This is a wall-clock budget flake under load, like the p2-pages:420 50 ms flake. It is not a Compare regression, and no Compare file touches that path.
   - Every Compare test passed in the full suite: p6-compare (engine a..c2 plus drive), and the p2-shell 'Compare review fixes are wired (§9.19)'.
3. In the same hold, every drive listed below exited 0 with top-level ok:true and no red steps:
   - compare.jsonl ×3
   - dialogs, debt1, direct-edit, direct-edit8-save, verify, sign-field, help-fields, nav-zoom-find
   - redact, bates, direct-edit3, refusals, replace-page, direct-edit8-chords, object-edit, content-edit
4. `shasum -a 256 -c checks/fixtures.sha256`: 9 OK. `shasum -c checks/compare-hash-before.txt`: 55/55 OK. A fresh list of every fixtures/**/*.pdf (plus the 2 listed PNGs) matches compare-hash-before.txt exactly.
5. `pgrep -fl -- '--drive='` is empty, and so is the check for review-out user-data-dirs. No Electron stragglers.

shot-compare.mjs was not run, as the brief says.

Open (not Compare): tests/p3-edit.test.mjs:888 has a 2000 ms wall-clock budget that can flake under full-suite load. The fix would be to raise the budget or measure the scan in isolation (debt item).
