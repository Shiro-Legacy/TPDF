# Batch review — lens TESTS, DRIVE, EVIDENCE (Opus 5.5, 2026-09-25)
Verdict: ACCEPT (no blocking; 2 should, 3 nit). Scratch: .quad/scratch/batch/review-tests/ (run.sh, summary.txt, norm-*.json, edges.mjs).

## Ran (one lock hold, released)
- batch.jsonl ×3 staged per §9.18 (mktemp; batch-in + batch-c copies; --out=): exit 0 ×3; pdf4all-out = hello nav report.txt rotated each time.
- Normalised STATE (paths → $OUT) of all 7 steps: identical across 3 runs; listing order encrypted, hello, nav, rotated every time. Output PDFs + report.txt: byte-identical across runs (6 shas ×3).
- node --test tests/p6-batch.test.mjs ×3: 10/10/0/0 each. `find fixtures -name pdf4all-out` empty after every run.
- Fixtures after: fixtures.sha256 9 OK; 55-line list identical to batch-hash-before.txt. No --drive= stragglers.
- Backup diff (.quad/backup/pre-batch-20260924.tgz) over every tests/*.mjs and tests/drive/*.jsonl: only p2-shell (REQUIRED_IDS +15, Help pin, READ_ONLY 'batch-file'), all per §9.6/§9.15/§9.20. New: p6-batch.test.mjs, drive/batch.jsonl. No existing drive line changed.
- Right-edge probe (mupdf structured text, run 1 outputs): B-0001..B-0005 all x1 = 575.0 on 612-wide pages (w−36 = 576), incl. rotated.pdf.
- acceptance/batch-red-before.log: 7 not ok, reasons match batch-build.md. int-batch-npmtest.log 563/562/0/1; int-batch-p6batch-1..3 10/10.
- shot-batch.mjs: node --check ok; read fully (not run).

## batch.jsonl vs §9.16 (header "7 steps, 0 click lines" = 7 lines, 0 clicks)
1 open nav — baseline. 2 header "Open doc" rev 1 — baseline. 3 batch batch-in: full slice (total 4 done 4 ok 3 failed 1, report ok, bates {1,6,4}, 4 rows) + announcement + doc/rev 1/dirty/canUndo — FAILS if absent. 4 undo rev 2 dirty false — proves open doc journal intact. 5 footer {bates} 6-8, next 9 — FAILS if batch did not advance batesNext. 6 rerun: all 4 failed (encrypted + 3 exists), report "exists", bates {9,9}, decorate next 9, announcement — FAILS if absent. 7 batch-c cancelAfter 2: done 2, cancelled true, rows 3–4 queued, next 10 — FAILS if absent. No vacuous batch expect.

## Findings
- T-R1 (should) tests/p6-batch.test.mjs:303-314 — outputs are checked by text regex only; `align:"right"` (pinned only as STATE echo in batch.action) is never checked in the bytes. If batch-file dropped align, every test passes. Fix: after pdfWords, assert each B-000n line's bbox x+w is within 10 pt of pageWidth−36 (incl. rotated.pdf; today 575 vs 576).
- T-R2 (should) tests/p6-batch.test.mjs:317-318 — the "byte-identical to the first run" check hashes report.txt against report1 read from the same file after all runs: tautological. Covered only indirectly by the exact-string assert at :316. Fix: read report.txt after step 3 is impossible in one drive, so drop the sha line or compare against the literal expected string's sha and say so.
- T-R3 (nit) tests/p6-batch.test.mjs:329 — `readdir(fixtures).includes('pdf4all-out')` is top-level only; §9.18 guard is `find fixtures -name pdf4all-out`. Fix: recursive readdir({recursive:true}) and assert no entry ends with pdf4all-out.
- T-R4 (nit) batch-build.md:17 says p6-batch has "11 tests"; the file has 10 (integration says 10). Red-before covers only 7 (engine + batchOutName); the main-pin and drive tests have no red-before evidence.
- T-R5 (nit) §9.14 dialog path (⌥⌘B/menu, #batch-start disabled until pick, disabled while running) has no automated evidence yet; only shot-batch.mjs covers it and it is unrun. Fix: run shot-batch in accept-batch.sh (as §6 requires) before accepting.

## checkedFine
- main.ts pins: ipcMain.handle('pick-folder' :922 and ('batch-io' :953 each followed by isTrustedOpen on the next line; accelerator 'Alt+CmdOrCtrl+B' once (:1095); `let batchDirArg` :44; folder pre-answer :1802.
- Symlink test: every non-encrypted row failed/unwritable, report unwritable, counter unmoved, elsewhere dir empty; contents-less batch → "batch: contents required", running false.
- shot-batch.mjs: osascript addresses `first process whose unix id is pid` with frontmost check + keystroke in ONE call; exit 2 on OSA-UNAVAILABLE; exit 1 naming the chord if #batch-dialog stays closed; --batch-dir= a mkdtemp copy; CDP pick/start; waits running false + done 4; gates ok 3/failed 1/order/statuses/bates {1,6,4}/report ok/rev 0/dirty false/#batch-summary; shot batch-a-done.png; SIGTERM in finally then 9 OK + four shas by value + no pdf4all-out.
- design-agent-drive.md: "## Batch drive additions" at EOF after "## Bates drive additions"; names the step, line-level folder, --batch-dir=, STATE slice, localeCompare order, cancelAfter; row-detail format matches batchFailure.
