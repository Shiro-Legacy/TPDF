# P6 U5 tests seat evidence (Opus 5.5, 2026-09-24)
Files (all new except the append): tests/drive/direct-edit6.jsonl, tests/p6-drive6.test.mjs, .quad/shared/p6/checks/shot-direct6.mjs,
.quad/shared/p3/design-agent-drive.md (appended "## P6 U5 drive additions", 6 lines, §9.36 traps included). scripts/drive.mjs: no change.

## direct-edit6.jsonl (§4.1 + §9.28-§9.33)
54 steps, 21 click lines (header states both; the test pins them from the header). Revision ledger 0…18, every undo pinned
(2,4,6,8,10,12,14,16,18); Shift toggles pin active.name; group Enter pins dirty:false; `menu` lines carry `contents`
(objections "Exact line templates" used verbatim); Escape before the fresh Shift-click (§9.28); group ⌥-drag rev 17/18 (§9.29).
No gesture-produced box in any expect.

## p6-drive6.test.mjs (§4.2 + §9.30)
Own drive()/pick()/near(); all(step,name) filter+sort, never find. ±0.5 pt view boxes: group drag +30,+20; dy −80 clamp
(Im2 [400,0,460,40], Im1 [72,90,172,140]); group Delete; group ⌥→ (+1); group Enter unchanged; ⌥-drag copy [460,92,520,132];
menu duplicate [410,62,470,102]; menu replace refused writes nothing; menu delete Fm1; group menu duplicate (+10,+10 each);
group ⌥-drag copies Im1 [92,162,192,212], Im2 [420,72,480,112]; pristine boxes after every undo; direct3 sha before == after == e3ae5a43….

## Runs (under .quad/run.lock, after both engine and renderer evidence existed)
- `npm run build && node scripts/drive.mjs tests/drive/direct-edit6.jsonl` → exit 0, 54 steps, 0 red
  (report kept at .quad/scratch/p6u5/tests/drive6-report.json).
- `node --test tests/p6-drive6.test.mjs` → 1 pass, 0 fail, 0 skip.
- `node .quad/shared/p6/checks/shot-direct6.mjs` → PASS all, Electron exit 0; fixtures.sha256 8/8 OK after; `pgrep -fl -- --drive=` empty.

## GUI shots (viewed with Read)
- review-out/u5-a-multi-select.png: Im1 (grey) and Im2 (blue, top right) both carry the solid selected outline, no handles
  (computed display:none on all 18 .multi .hit-handle while hovering Im2); Fm1 and the text runs keep dashed outlines.
- review-out/u5-b-menu-fm1.png: real right-click on Fm1 → dark menu at the pointer with Replace… dimmed, Duplicate and Delete
  enabled; Fm1 selected with handles; DOM replace disabled + aria-disabled="true", STATE items.replace false.
- review-out/u5-c-alt-duplicate.png: real ⌥-drag of Im2 by 60,40 pt → copy at [460,92.01,520,132.01] selected with handles,
  original unchanged at [400,52,460,92]; 4 .object-target; direct.gesture.kind 'duplicate' read mid-drag; badge "Edited — not saved".

## Not done / notes
- Full `npm test` not run by this seat (integrator's job). Other acceptance drives not rerun here (the renderer seat reports them green).
- Nit seen in shot A: the Inspector panel shows Im1 (X 72, Y 600) while the anchor (direct.active) is Im2.

## Fixes after review
Tests seat (Opus 5.5), 2026-09-24. Each job ran under .quad/run.lock. tsc ran clean, the build exited 0, fixtures.sha256 is 8 OK, the u5-hash-before diff is empty, and no --drive= processes are left.
- S2 reproduced: in the pre-fix run the undo after the group menu duplicate leaves selection []. direct-edit6 now pins `direct.selection [{name:'Im1'},{name:'Im2'}]` on that undo, which is now step 41. The jsonl:60 comment cites §9.28. p6-drive6 asserts that the undo's selection targetIds are the originals. The design-agent-drive.md trap is unchanged because it already states the contract rule. This step is RED until renderer S1/R1 lands (renderer.ts:6284/:6304 undoEdit/redoEdit must keep directSelection so restoreDirectFocus re-resolves it). Drive exit 1 and p6-drive6 fail 1 are expected until then; only step 41 is red.
- S3 reproduced: no assertion told a copy from its original. p6-drive6 now has `selectsCopies`. For the ⌥-drag, menu duplicate, group menu duplicate, group ⌥-drag and keyboard menu duplicate steps, it asserts that each selection targetId is the object whose view is the copy box. An offline mutation that swaps the ⌥-drag selection to the original's targetId fails with "⌥-drag: selection is the copies".
- S4 reproduced: every key sent to an open menu was Escape. Added before mode read: click Im1, key Shift+F10, key ArrowDown, key Enter (edited image/duplicate Im1, objects 4, menu null, revision 19), then undo (revision 20). mode read now expects revision 20. The header now reads 59 steps and 22 click lines, and p6-drive6 pins the same counts and the copy box [82,152,182,202]. All 5 new steps are green. The design-agent-drive.md trap now lists the menu keys.
