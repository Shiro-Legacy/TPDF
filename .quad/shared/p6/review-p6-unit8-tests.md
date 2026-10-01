# P6 U8 review — TESTS / DRIVE / EVIDENCE lens (Opus 5.5, 2026-09-24)
Verdict: ACCEPT (0 blocking, 0 should, 3 nits). Scratch: .quad/scratch/p6u8/review-tests/.

## Ran (build/node --test/drive under .quad/run.lock)
- npm run build (0); node --test p6-drive9 p6-drive8 p6-drive3 p3-drive: 26/26 pass.
- drive direct-edit9 x3: exit 0/0/0, 30 steps, 0 red each; STATE of the 3 runs identical except the "rendered in N ms" status text.
- npm test: 536 tests / 535 pass / 0 fail / 1 skip (the build report's 534+1 fail predates the §9.14 cut flip; accept-u8.sh:9 gate 535 >= 529+6 holds exactly).
- fixtures.sha256 9 OK; full list cmp u8-hash-before.txt identical (55) after drives and after the suite; 0 --drive= stragglers.
- Red-before-fix re-derived: scratch copy of build/ with `depth === 0 ||` removed from content-edit.js:1165 -> p6-text8 (a)(b)(e) not ok, (c)(d) ok — matches acceptance/u8-red-before.log.
- Mutation: :2360 call passing 0 -> (d) fails (guard real). :1269 and :2941 passing 0 -> p6-text8 + p3-targets/edit/replace/objects, p3b-content, p6-paragraph/objects7/direct3: 158/158 still pass (see N1).
- Engine probe (probe-reopen.mjs): Twin x2 + "Move me" copy, edit B -> save -> reopen: all four candidates with own bounds; editing the reopened copy leaves the original unchanged.

## Backup diff (pre-p6u8-20260924.tgz, every tests/*.mjs, tests/drive/*.jsonl, scripts/drive.mjs)
Non-empty only: p2-shell.test.mjs (:217 new sentence, :222 gone list + renderer-src announce pins; §9.8/9.9), p6-objects7.test.mjs:450-465 (§9.5, stricter: scan bounds +10 ±0.5, targetIds = the pasted id), p3-targets.test.mjs (the four §9.1 pins, each keeps distinct-id asserts and adds own-origin bounds; the :655 candidate list only excludes ALPHA which is now pinned separately — not a weakening), drive/direct-edit8-cut.jsonl:32 (§9.14, stricter: name "Move me" + selection pinned). scripts/drive.mjs unchanged. No other edit.

## Drive checks
- Every on-page step is non-vacuous pre-fix: clicks at 15/17/22/27/29 expect kind "text" (pre-fix twins and the original "Move me" are ambiguous -> click opens kind "add"); targets 7 (line 14) and 8 (line 26) differ pre-fix.
- p6-drive9.test.mjs pins by index: S9 != S11 ids, S16 == S9, S14 != S11, rects A/B/copy ±0.5, S20 lastAdd != original + bounds, S21 == original id + rect, S23/S24 != original, S24 rect = copy + 20. Indices verified against the run ledger.
- Header "30 steps, 9 click lines" equals the file (9 click cmds; the handle step is not a click) and is pinned three ways in the test.
- Undo x5 back to targets 5, dirty false (line 36). Click before each key; no reselect after undo; revisions absolute.

## Shot script (read fully, not run)
- osascript addressed by `first process whose unix id is <child.pid>` (spawned Electron binary); frontmost check and keystroke in ONE call; OSA -1712/1002 -> exit 2; SIGTERM + fixtures.sha256 in finally; no clipboard use.
- review-out/u8-a-twins.png: direct3 p0 in Edit text, "Edited — not saved"; two "Twin" runs outlined, the upper (B) with its editor and handles open, original "Move me" outlined. Matches the build claim.

## Findings (nits)
- N1 tests/p6-text8.test.mjs:109 (§1.2/9.3): the depth passed at content-edit.ts:1269 and :2941 is unguarded (mutating either to 0 keeps 158/158 green; my Form probe shows no observable difference). Fix: note it in §10, or add an edit-paragraph/edit-text case on edit-shared ALPHA if a difference is ever found.
- N2 tests/drive/direct-edit9.jsonl:36 (§3.1/§1.4): no save -> reopen step and no check that undoing B's edit restores "Twin" (only target counts). Works (probe). Fix: the objections' optional +4 (save-to OUT, open, mode edit, click copy -> text "Move me").
- N3 .quad/shared/p6/checks/shot-direct9.mjs:219 (§9.11): "Twin" is typed by CDP Input.insertText, not an osascript keystroke (only ⌘Return is real; declared in the header); the builder's "exit 0" has no log until accept-u8.sh writes u8-shot.log; accept-u8.sh:10 counts not-ok >= 3 without checking (a)(b)(e) by name. Fix: grep the three names in the red-before gate.

## design-agent-drive.md
"## P6 U8" matches shipped behaviour: duplicates per run; overprints ambiguous; repeated-Form runs have no on-page bounds; no suffix-only identification.
