# Compare build (implementer, Opus 5.5, 2026-09-25, lean mode)
Files: src/renderer/engine.ts (SaveReason "no-compare-document"; Compare*Request/Reply, CompareLine; Session.compare {doc,name,pageCount,sourceBytes}, createSession compare:null; open :4362-4368 closes the slot + compareClosed when the pair > MAX_PDF_BYTES; compare-open :4413, compare-close, compare-render, compare-text :4465; pageLines, stextLines, normaliseLine, diffLines :6387, MAX_COMPARE_LINES 2000), src/main.ts :1126 (File ▸ Compare with… Alt+CmdOrCtrl+K after Verify Signatures…), src/renderer/renderer.ts (pickInsertSource(action), compare branch in runEditRequest, requestCompare/openCompare :7305/clearCompare/closeCompare/refreshCompare :7371/runCompare/paintCompare :7454/driveCompare :7492/compareSummary, STATE compare :8231, goTo/applyEdit/applyPageEdit/openDocument/driveGoto hooks, workerFailed clearCompare, #compare-view keys), src/renderer/index.html :224 (#compare-view after #reader), :594 (ul#help-compare after #help-bates), src/renderer/styles.css (§9.14 rules + .compare-b relative, overlay transparent), tests/p6-compare.test.mjs (NEW, 10 tests), tests/drive/compare.jsonl (NEW, 12 steps, 0 clicks), tests/p2-shell.test.mjs, .quad/shared/p6/checks/shot-compare.mjs (NEW, node --check only), .quad/shared/p3/design-agent-drive.md (appended "## Compare drive notes"). engine-core.ts and worker.ts untouched (no OP_PERMISSION row, §9.1).

## Done per §
- §9.1 no write gate: compare ops never call requireMutable/mutate/beginOperation; READ_ONLY += 4 ops (p2-shell :184); signed-intact main compares (test "§9.1/§9.7").
- §9.2 copy gate on MAIN doc → read-only; in-memory rc4 permissions=-20 main (test c); encrypted.pdf → encrypted, slot kept.
- §9.3 size cap before opening (test c2: MAX_PDF_BYTES+1 → too-large; padded compare + hello main → compareClosed, slot null). Memory measurement left to the review.
- §9.4 bbox [x0,y0,x1,y1] y-down from stext (test b: PDF4All [72,53,204,102], one [72,66,112,98] ±0.5).
- §9.5 test a (compare-render p2 sha = separate session's nav p2; p0 "one" vs main "PDF4All"; identical pair same 1, empty, equal pixels) and g (alpha/beta/gamma vs alpha/gamma/delta → same 2, beta, delta; "a  b" = "a b").
- §9.6/9.7 createSession compare:null; workerFailed clears; store-then-destroy; reply {ok,name,pageCount,pages}; too-many > 2000 lines (test).
- §9.8 S = min(1, floor(4096/maxEdge*100)/100); A from cache key `${pageA}:${S}:${revision}`, B `cmp:${pageB}:${S}` held apart; raw-buffer diff on maxW×maxH, white outside; fresh ImageData overlay 255,0,0,153.
- §9.9 menu; §9.10-9.13 drive `from` line key, pageB/page/showDiff/close, compareGen + compareWork awaited (driveGoto, applyEdit/applyPageEdit, openDocument), summary = announcement, rows titled "Page N, x0 y0 x1 y1 pt".
- §9.14-9.15 markup/CSS/keys (PageUp/PageDown/Escape), Help sentence pinned tag-stripped (p2-shell :146) + order pin; eleven matrix rows unchanged.
- §9.16 drive ledger revisions [0,1,1,1,1,1,1,2,0,0,0,0]; hello-vs-nav differing 1590/484704 ratio 0.0033, nav p2 1670, identical 0, total = 612*792 at S=1.

## redBeforeFix (acceptance/compare-red-before.log, pre-unit build/)
10/10 red (regenerated after review N-4 with the current test file + §9.19(a) drive, pre-compare-20260925.tgz built in scratch, Electron on): 9 engine tests (a)-(g), (c2), §9.1/§9.7 — "unknown op: compare-open" / compare-open reply ok false !== true; drive half — pre-unit STATE has no compare slice (step 1 compare undefined).

## Commands (one lock hold, .quad/scratch/compare/implementer/run.sh)
npm run build && node --test tests/p6-compare.test.mjs tests/p2-shell.test.mjs tests/p3-drive.test.mjs tests/p6-bates.test.mjs → 45 tests, 45 pass, 0 fail.
Drives (each once, exit 0, 0 red): compare 12, dialogs 11, debt1 22, direct-edit 75, direct-edit8-save 42, verify 17, sign-field 45, help-fields 21, nav-zoom-find 20, redact 8, bates 9, direct-edit3 79, refusals 29, replace-page 7.
Hashes: fixtures.sha256 9 OK; fixtures diff vs compare-hash-before.txt empty. No straggler Electron.

## Re-pins: none (p2-shell additions only: REQUIRED_IDS :54-55, Help pins :146-148, READ_ONLY :184).
## Not done: shot-compare.mjs not run (fable51 runs it); §5 memory measurement (review); full npm test not run.
## Risks: compare-text reads the LIVE main doc, pixel diff O(W·H) on the renderer thread (~40 ms at 4096²); after an edit the compare summary refreshes silently (the edit's announcement wins); an error from a refused compare-open persists through the next undo (undo does not clearError).

## Fixes after review
- Engine fixer (wf_2acf1101-1b0 findings E-R1..E-R3). All three reproduced before the fix.
- E-R1 / §9.19(d): engine.ts `open` wraps openDocument + detect + the no-pages check; when the pair cap already closed the slot, the rethrown error carries `extra.compareClosed = true` (the dispatch catch spreads it). Renderer half (openDocument's catch calls clearCompare on `compareClosed`) is R-E1, the renderer owner's.
- E-R2 / §9.7: test (c) re-opens a compare file and asserts the previous doc's `pointer === 0` (destroyed) and the new one is live. Kills M1 (fails 1 of 9).
- E-R3 / §9.3: test (c2) opens paddedPdf(MAX - hello) beside hello (ok), then a 4 KiB `%PDF-1.4 garbage` main open → ok:false, compareClosed:true, slot null, main still hello.pdf. Kills M7; pre-fix build fails the compareClosed assert.
- Ran: tsc clean; npm run build; node --test tests/p6-compare.test.mjs tests/p2-shell.test.mjs → 22/22. Fixture hashes: 9 OK, compare-hash-before list 55/55 OK.

## Fixes after review
- Renderer fixer (wf_2acf1101-1b0 R1-R4, R6, R7, R-E1; contract §9.19). R1, R2, R6 reproduced live with copies of review-compare-renderer-{wide,mouse}.mjs (scratch/compare/fix-renderer): overlay 818 vs B 612; y-scale 0.768 vs 0.593; after unlock text/pixels null, stale summary, guard error kept. R3, R4, R7, R-E1 reproduced by code read (35 px field; `Number("")` commits page 1).
- R1 §9.19(e): paintCompare overlay = `ImageData(b.pixelWidth, b.pixelHeight)`, in-B pixels only (differing/total still maxW×maxH); `.compare-b{width:fit-content;max-width:100%}`. Post-fix: overlay 612 == B 612 wide; overlay scale == B scale portrait-vs-landscape.
- R2 §9.19(h): runCompare's locked branch empties the 3 canvases, list and summary; applyUnlocked ends with `refreshCompare(true)` (post-fix: text/pixels filled after unlock, A re-rendered).
- R6 §9.19(c): clearError in applyUnlocked and on a successful compare-open; undo untouched.
- R3 §9.19(f): the summary words `read-only` / `too-many` separately.
- R4/R7 §9.19(g): emptied #compare-page-b reverts; width 4rem; `.compare-bar` flex gap; `#compare-name-a/b` captions (doc.name / compareDoc.name), added to REQUIRED_IDS.
- R-E1 §9.19(d): openDocument's catch reads `err.reply.compareClosed` (engine spreads `extra` into the reply), clears compare (last close/too-large) and announces the size-limit sentence.
- Pins: p2-shell test "Compare review fixes are wired (§9.19)" — every assert keys on text absent before the fix.
- Ran: tsc clean; build; p2-shell + p6-compare 23/23; compare.jsonl ok; dialogs.jsonl ok; fixtures 9 OK, compare-hash-before identical; no stragglers.

## Fixes after review
- Tests fixer (wf_2acf1101-1b0 S-1, S-2, N-1..N-5). All seven reproduced by code read; S-1 also live (old compare.jsonl exits 0 on a clamp→refusal mutant).
- S-1 §9.19(a): compare.jsonl step 4 {pageB:1} → addedLines ["two"]; step 5 {pageB:9} → pageB 2, ["three"], error "". Ledger comment updated; p6-compare drive half pins steps[3]/[4] pageB [1,2] and step 5 differing>0. New script fails on the mutant (pageB 1, error set).
- S-2: shot-compare.mjs types "2"+Tab into #compare-page-b → pageB 1 ["two"], then Escape in #compare-view → open false, #reader shown, then ⌥⌘K reopen + #compare-close click. Reviewer's "3" rejected: nav has 3 pages, so a 0-based slip clamps 3→2 and passes (shown live). Mutants: setComparePageB(page) → FAIL pageB 2; dead Escape → FAIL "Escape in #compare-view did not close compare".
- N-1: chordOpens() names "⌥⌘K (key code 40 using {option down, command down}) did not open compare". N-2: centre() hit-tests with elementFromPoint, logs {hit, over}, press() logs FALLBACK + element.click(); sleeps replaced by #status/STATE waits. N-3: hash check in finally, exitCode 1 unless 9 OK.
- N-4: acceptance/compare-red-before.log regenerated (10/10 red, see redBeforeFix). N-5: design-agent-drive.md Traps drops "(or locked)", S = min(1, floor(4096/maxEdge·100)/100).
- Ran: tsc clean; npm run build; p6-compare 10/10; node --check shot-compare ok; compare.jsonl exit 0; shot-compare.mjs exit 0 (all clicks real hits); fixtures 9 OK, compare-hash-before 55/55 identical; no stragglers.
