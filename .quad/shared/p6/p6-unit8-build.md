# P6 U8 build evidence (implementer, Opus 5.5, lean mode, 2026-09-24)
Engine: src/renderer/content-edit.ts editMatchRuns gains REQUIRED 7th param `depth` (§9.3); :1470 accept `near.length === 1 && (depth === 0 || wholeLines === (nearWhole ? 1 : 0))`; call sites :1269 record.depth, :2360 walk depth, :2941 first.depth; comment :1425 rewritten. tsc clean.
Renderer: renderer.ts:3957-3958 comment + announce per §9.9. index.html:569 Help: U7 pasted-run sentence removed; "Added text that exactly repeats…" → "Text placed exactly on top of the same text, and text inside a form drawn more than once, cannot be edited."
## Red before fix (pre-unit build/, copy at .quad/scratch/p6u8/prebuild; log acceptance/u8-red-before.log: 3 not ok, 2 ok)
- (a) not ok: 'the second text-add reply carries bounds' (bounds null)
- (b) not ok: supports ['unsupported','unsupported'] vs ['candidate','candidate'] (both ambiguous-text-association)
- (e) not ok: preview.matches [] vs ['p0.c2.89.f22530a3'] (rejected ambiguous-text-association)
- (c) overprint and (d) edit-shared Form guard pass on both builds (by design)
## Re-pins (file:line old → new)
- tests/p3-targets.test.mjs:229-245 rtl " PDF4All" ×2 ambiguous → candidate, repertoire 'םלש וADFP4l', bounds [160,102,260,140]/[170,172,270,210] ±1, distinct ids; Type1 marker find now filters subtype Type1
- tests/p3-targets.test.mjs:388-394 literal+hex PDF4All ambiguous → candidate, distinct ids, x0 72, Δy 150
- tests/p3-targets.test.mjs:555-558 PDF4All ×2 around inline image ambiguous → candidate, distinct ids, Δy 50
- tests/p3-targets.test.mjs:647-654 targets-syntax ALPHA ×4 ambiguous → candidate, ambiguous [] , 4 ids, bounds 72,59/99/179/219
- tests/p6-objects7.test.mjs:450-467 "…scan ambiguous" → "…scan as candidates": both candidate, pasted bounds = source+10 ±0.5, targetIds.length 1 = pasted id
- tests/p2-shell.test.mjs:217 U7 sentence → new §9.8 sentence; :222 gone list + both old sentences; renderer src pins new announce, old gone
## New files
tests/p6-text8.test.mjs (5 tests a–e), tests/drive/direct-edit9.jsonl (30 steps, 9 click lines), tests/p6-drive9.test.mjs, .quad/shared/p6/checks/shot-direct9.mjs, design-agent-drive.md "## P6 U8".
## Runs (under .quad/run.lock)
- npm run build && node --test p6-text8 p6-objects7 p2-shell p3-targets p3-edit p3-replace p6-direct3 p6-paragraph p3b-content: 149/149 pass
- drive direct-edit9 30 steps 0 red; direct-edit3 79/0; direct-edit8 115/0; p6-drive9 1/1 pass
- npm test (once): 536 tests, 534 pass, 1 fail, 1 skip — fail = p6-drive8.test.mjs:236 (direct-edit8-cut.jsonl:32), see below
- shot-direct9.mjs: exit 0 (first attempt died on my AppleScript one-line if/else syntax error before any keystroke; fixed, rerun once). Real osascript ⌘Return ×2 → rev 1, 2; twins p0.c1.95.f22530a3 / p0.c2.94.f22530a3. review-out/u8-a-twins.png shows both "Twin" runs outlined as targets, the upper one (B) with its editor and handles open.
- fixtures.sha256 9 OK; u8-hash-before diff empty; 0 --drive= stragglers
## Not done / deviations
- UNOWNED FLIP (left red): tests/drive/direct-edit8-cut.jsonl:32 expects paste `edited.name "PA1"`; after D1 the pasted duplicate "Move me" is a candidate, so the paste selection is [PA1, Move me] and name is "Move me". Integrator decides.
- §9.10 ledger changed: GUI adds land box-top at the click (A [299.88,396.16,325.21,412.65], B [299.88,296.3,325.21,312.79]); B edited to "Pair" not "Twin 2" (a GUI-added run's growth limit is its own right edge → overflow, pre-U8 too); "Move me" added BEFORE the undos because text-add after an undone text-add lands with a dangling font (/PDF4AllF1 → freed 14 0 R, unsupported-encoding) — reproduced on the pre-unit build (engine.ts, not in scope).
- §9.13 refind covered: grip move of the "Move me" copy keeps the editor on the copy (step 24). Shared-/Contents duplicates: no dedicated pin.

## Fixes after review
- N2 (engine, should): reproduced as a test gap (behaviour already correct). tests/p6-text8.test.mjs case (a) now saves after editing B to 'Twin 2', reopens the bytes and asserts one 'Twin 2' plus A with its targetId and bounds unchanged; after undo it asserts every page-0 /Contents element is byte-identical to before the edit. No source change. Build + p6-text8/p6-objects7/p3-targets/p3-edit/p3-replace/p6-direct3: 116/116 pass; fixtures 9 OK.
- R8-1 (renderer, should): reproduced (cow-edit.jsonl exit 0: edit-shared.pdf form run ALPHA→BRAV edits via the Text panel). index.html:569 now reads "Text placed exactly on top of the same text cannot be edited; text inside a form drawn more than once has no on-page target (edit it from the Text panel, this placement only)."; p2-shell P6 U7 Help test pins it and adds the old "…, and text inside a form drawn more than once, cannot be edited." clause to `gone`.
- R8-3 (renderer, should): reproduced by scratch drive (direct3 p0: add Twin, undo, on-page add "Solo" → rev 3, lastAdd {targetId "", bounds null}, targets 5, no error, no announcement). renderer.ts addTextAt now announces "Added text is not editable on the page" when targetId is ""; p2-shell pins the source line (red on pre-fix). Scratch drive .quad/scratch/p6u8/fix-renderer/r83b.jsonl expects that `announcement` at rev 3, exit 0. Engine root cause (dangling /PDF4AllF1 after undo, engine.ts) stays in §10.
- Ran under lock: tsc, build, p2-shell + p3-drive 30/30, direct-edit9/3/8 + cow-edit exit 0; fixtures.sha256 9 OK; u8-hash-before list all OK.
- T8-N2 (tests, should): reproduced as a drive gap (no save/reopen; B's post-undo text unchecked). The literal fix (save/open/click (324,204) after the final undo) is wrong: at rev 10 every add is undone, so the copy is gone and (324,204) would open an add box; the moved copy sits at (324,224). Appended 13 steps at the END of direct-edit9.jsonl (none inserted): redo x2 -> click B (312,304) reads "Twin" at rev 12 with its pre-edit id/rect; redo x3 -> rev 15; save-to ${OUT}/direct9.pdf; open it (dirty false, rev 0); mode edit -> targets 8; click (324,224) -> "Move me" copy (own id, moved rect); Escape; click B -> "Pair". Header now 43 steps / 12 click lines; p6-drive9 pins counts plus S33 id = S11 id, S41 copy ≠ original with moved rect, S43 id/rect = S14. Under the lock: tsc clean, build, drive exit 0 (43 steps, 0 red), node --test p6-drive9 1/1 pass; fixtures 9 OK, full hash list identical to u8-hash-before.txt.
