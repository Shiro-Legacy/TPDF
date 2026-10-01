# Batch — build evidence (Opus 5.5 implementer, lean mode, 2026-09-25)
Contract: .quad/shared/p6/contract-batch.md (§9 folded). No git; every existing file edited in place.

## Red before fix (current build/, no rebuild in that hold) → acceptance/batch-red-before.log: 7/7 not ok
- batch (a)-(f): `unknown op: batch-file` (a: false !== true; b: encrypted.pdf reply `unknown op: batch-file`; c: same; d: `the session document 3 !== 1` (pre-fix counts); e, f: false !== true)
- batchOutName: `policy.ts exports batchOutName` (typeof undefined)

## Changed
- src/renderer/engine.ts: `BatchStampAction`/`BatchFileRequest` types (+ SessionRequest union); `checkStampAction` (:3229), `checkBatesRange`, `stampLoop` (:3265) factored out of `stampPages`; `batchFile` (:3302) transient open via mupdf.Document.openDocument + asPDF + disableJS, refusal order too-large → not-pdf → encrypted → read-only → bad-pages → stamp-error, saveToBuffer(SAVE_OPTIONS) + MAX_SAVE_BYTES, transfer, finally closeDocument; dispatch `case "batch-file"` (:5940), no requireMutable, no OP_PERMISSION row.
- src/policy.ts: `batchOutName(name, listed)`.
- src/main.ts: `let batchDirArg` (:44), `batchListings` Map keyed by sender id (deleted on destroy), `pick-folder` (:922; smoke without dir → cancelled, localeCompare numeric sort, ≤200) and `batch-io` read/exists/write (:953; not-listed, bad-name, too-large, lstat real dir else unwritable, exists, writeNew), File ▸ Batch… Alt+CmdOrCtrl+B (:1095), startDrive `folder` pre-answer (:1802), a `batch` line without contents is sent as "" (renderer refuses).
- src/preload.cts: `pickFolder`, `batchIo` (write size precheck).
- src/renderer/renderer.ts: Pdf4AllApi additions; `batch` branch beside `help` in runEditRequest; #batch-dialog refs/listeners (cancel event preventDefault while running); `openBatchDialog`, `pickBatchFolder`, `startBatch` (Bates §9.11 check), `runBatch` (:8211; exists probe → read → batch-file SAVE_TIMEOUT_MS → write; batesNext moves after a write; worker reason stops the queue; report.txt §9.17; announce + #batch-summary), `driveBatch` (:8281, returns the whole queue; cancelAfter); STATE `batch` (present once touched).
- src/renderer/index.html: #batch-dialog (15 ids of §9.15, #batch-start ships disabled); second <li> in ul#help-bates.
- tests/p6-batch.test.mjs (NEW, 10 tests: engine (a)-(f), batchOutName, main pins, drive batch.jsonl on staged mkdtemp folders, symlink + contents-required; the red-before run covered 7 of the 10 (engine + batchOutName), so main pins and the two drive tests have no red-before evidence); tests/drive/batch.jsonl (NEW, 7 steps, 0 click lines).
- tests/p2-shell.test.mjs: READ_ONLY += 'batch-file'; REQUIRED_IDS += 15 batch-* ids; Help pin 'results go to a pdf4all-out folder'.
- .quad/shared/p6/checks/shot-batch.mjs (NEW; node --check ok; NOT run); .quad/shared/p3/design-agent-drive.md: '## Batch drive additions' appended.

## Re-pins (old → new)
- tests/p2-shell.test.mjs:177 READ_ONLY `'pdf-from-images', 'page-ids'` → `'pdf-from-images', 'batch-file', 'page-ids'`
- tests/p2-shell.test.mjs:52-53 REQUIRED_IDS: 15 batch-* ids added (no id removed)
- tests/p2-shell.test.mjs:143 new Help pin (added, nothing replaced)

## Checks (one lock hold: npm run build && node --test … && 14 drives)
- node --test tests/p6-batch tests/p2-decorate tests/p6-bates tests/p2-shell tests/p3-drive: 58/58 pass, 0 fail.
- drives exit 0: bates, bates-pad, dialogs, refusals, help-fields, direct-edit8-chords, replace-page, direct-edit, direct-edit3, direct-edit8-cut, direct-edit9, object-edit, content-edit, debt1.
- fixtures.sha256 9 OK; batch-hash-before.txt diff empty; `find fixtures -name pdf4all-out` empty; no straggler Electron.

## Not done / notes
- shot-batch.mjs not run (fable51 runs it; the ⌥⌘B System Events chord is unchecked). Full npm test not run.
- stampPages now runs kind/text/align/pad/start/bates-required before asPDF/checkPages, so a request with BOTH bad pages and a bad Bates/align field reports the Bates/align token first (single-error behaviour unchanged; Bates (a)-(f) green).
- A saveToBuffer throw inside batch-file reaches dispatch's catch (ok:false, no reason); the row fails with the message and the queue continues.

## Fixes after review
- E-R1 (engine.ts batchFile): detect()+countPages() now in a try → `not-pdf`; `saveToBuffer(SAVE_OPTIONS)` in a try → `stamp-error` detail `save`. No mupdf throw after the open escapes as a reasonless reply; the previous note above about a save throw is superseded.
- E-R3 (engine.ts batchFile): the ArrayBuffer and MAX_PDF_BYTES checks now run before checkStampAction (§9.1 too-large first); both still precede the open (§9.5).
- E-R2 (tests/p6-batch.test.mjs): (a) runs security-set aes-256 on the open doc, then batch-file → output needsPassword() false; (b) adds a broken page tree (/Kids → missing obj) → not-pdf; (c) asserts zero opens for every bad action and for maxPdfBytes+1 (with FOOTER and with align 'centre' → too-large); (d) proxy `failSave` = 'throw' → stamp-error/save, 'huge' → too-large with the buffer destroyed, opens−destroys balanced.
- Pre-fix engine (scratch/batch/fix-engine/engine.prefix.ts) fails (b), (c), (d); fixed: p6-batch + p2-decorate + p6-bates 26/26. Fixture hashes unchanged; no pdf4all-out under fixtures/.

## Fixes after review
Renderer fixer (review wf_9fbe1195-4ab). Each finding was reproduced with review-batch-renderer-mouse.mjs (ONLY=ab) before the fix: A7 FAIL with bates {1,1,4}, B0 Start at y=1378 hit null.
- R1: at the end of runBatch, `finally` now refreshes #batch-bates-start and #batch-bates-pad from batesNext/batesPad, so a second Start continues the counter. Probe A7 now reads bates {start:6,next:6,pad:4}.
- R2/R6: #batch-list sits in div#batch-list-box (max-height 45vh, overflow auto) and has aria-label="Batch files". #batch-dialog styles match #verify-dialog. Probe B0: Start at y=746, hit batch-start.
- R3: added a Bates column (the padded from–to range) and th/td padding. STATE shapes are unchanged.
- R4: renderBatch sets batchCloseButton.disabled = state.running.
- R5: the pad falls back to `action.batesPad ?? batesPad` (it was 6).
- Pins: new p2-shell test "Batch review R1-R6". It fails on the pre-fix source.
- Ran under the lock: tsc, then build. p2-shell + p6-batch 22/22. The staged batch.jsonl and bates.jsonl drives exited 0. Probe ab: 27 ok, 0 fail. Fixture hashes are 9 OK, the batch-hash-before list is 55/55 OK, and there is no pdf4all-out under fixtures/.

## Fixes after review (tests fixer, 2026-09-25)
- T-R1: tests/p6-batch.test.mjs `rightGaps()` + drive assert: each B-000n line's bbox.x+w within 10 pt of pageWidth−36 (hello, nav p0-2, rotated). Red: .quad/scratch/batch/fix-tests/red-align.mjs — align dropped gives gap 508 → FAIL; right gives 1 → PASS.
- T-R2: removed the tautological sha(report.txt)===sha(report1); the exact first-run literal (read after the rerun) carries §9.13, comment says so.
- T-R3: fixtures guard uses readdir({recursive:true}) basename check (test and shot-batch finally).
- T-R4: count corrected to 10 tests; red-before covered 7 of 10 (noted in Changed).
- T-R6: shot-batch.mjs second Start (field untouched): rows encrypted/exists×3, ok 0 failed 4 report "exists", batch.bates {6,6,4}, #batch-bates-start 6, #batch-summary, report.txt Buffer-equal. node --check only (not run). #batch-close.disabled not pinned: §9.14 only disables pick/start and renderer.ts:8122-8123 never disables close (click is ignored while running, :8863).
- Checks (one lock hold): tsc ok; build; node --test tests/p6-batch 10/10; node --check shot-batch ok; staged batch.jsonl exit 0 (pdf4all-out = hello nav report.txt rotated); find fixtures -name pdf4all-out empty; fixtures.sha256 9 OK; batch-hash-before diff empty.
