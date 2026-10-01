# P6 U5 review: renderer + human UX lens (real CDP mouse and keys). Opus 5.5, 2026-09-24
Probe: `.quad/shared/p6/checks/review-u5-renderer-probe.mjs` (sessions sel/fix/esc/save/form2, pick them with `ONLY=`) and the drive probe
`checks/review-u5-renderer-menu.jsonl`. All runs used .quad/run.lock. `npm run build` exited 0. The probe ran 3 times; two early fails were
probe bugs: off-screen points after a scroll, and an Enter that hit the focused Replace item. The drives direct-edit, direct-edit2,
direct-edit3, debt1, object-edit and help-fields all exited 0. `node --test p6-direct p2-shell`: 9/9 pass. fixtures.sha256 is 8/8 OK.
The u5-hash-before diff is empty. `pgrep --drive=` is empty.
Verdict: REVISE. Three `should` findings and one `nit`, all owned by renderer.ts. The group, duplicate and menu behaviour itself holds under real input.

## Findings
R1 (should, K1, §2.1/§9.28). renderer.ts:6284 (undoEdit) and :6304 (redoEdit) call `clearDirectFocus()`, which empties `directSelection`.
Real-mouse repro b2: click Im1, Shift-click Im2, drag Im2 30,20 pt, then click the toolbar Undo. `direct.selection` is `[]` (expected
length 2). Drive repro (review-u5-renderer-menu.jsonl step 10: click Im1, click shift Fm1, drag, undo) gives `selection []`, exit 1.
The ⌥-drag undo (c1) also leaves `[]` instead of the originals. Fix: undo/redo drop only `directFocusedMark` and `directFocusedLink`,
and keep `directFocusedObject` and `directSelection` so that `restoreDirectFocus` re-resolves them. Tests owner: add one direct-edit6
step after an undo that pins selection length 2, and move the p6-drive6 step-count pin with it.
R2 (should, K2, §0 parity "direct.active and the panel agree"). renderer.ts:2938-2951 `toggleObject` and :2873 `selectCopies` never call
`selectContentRow` or `openContentObjectForm` (focusObject does this at :2848-2849). Repro a1: click Im1, then Shift-click Im2. The anchor is Im2,
but the Inspector reads "Image 100×50 pt, X 72, Y 600" (Im1), see shot a1. After a real ⌥-drag the copy is the anchor, yet the Inspector
still shows the pre-drag form with no copy row current (shot c1). Fix: add one helper `syncObjectInspector(anchor)`
(`findIndex` by targetId → `selectContentRow(i,false)` + `openContentObjectForm(item,false)`). Call it from toggleObject's add and
remove branches and from the end of selectCopies.
R3 (should, §2.2 "Escape mid-drag puts every member back", §9.22 spirit). renderer.ts:2372 (startRectGesture `cancel`). After Escape the
buttons snap back, so the real pointerup lands on empty page. The trailing click then goes to the common ancestor, `pageRegion`, and
`clickToAdd` opens an add-text editor at the release point. Repro b3x (group) and e1 (single Im2): press, move 40,40 pt,
Escape, release gives `direct.active {kind:'add', point:[469.7,111.7]}` and 1 inline editor (shot e1). This was already true for single
objects since U3, but it is now reachable from the new group gesture, and the next click then only closes the editor (it broke my first b4 run).
The drive never releases after `escape`, so it cannot see this. Fix: in `cancel`, set `addBlocked = true` so the trailing click adds
nothing (clickToAdd already reads and clears it).
R4 (nit, §2.4). renderer.ts:2663-2666 `activateObjectMenu` closes the menu and then does nothing, with no message, while `directGestureWork` is
pending. Repro: menu Replace (a native picker stays open), then menu Delete on a group gives no request and no error. Fix: show "Wait for the current
edit to finish" or keep the menu open.

## Checked fine (real CDP unless noted)
- a: Shift-click gives 2 `.selected.multi`, and 18/18 handles are display:none (2 grips) while hovering Im2. Selection targetIds match content.objects
  and the anchor is Im2 (shot a1). A second Shift-click on Im2 removes it, and focus moves to Im1's button. A real Delete then deletes only Im1 (objects 2,
  "Deleted image"). Undo gives 3.
- b: a 2 px press on a group member sends no request, and all three button CSS boxes are identical before and after (no union resize). A body drag gives 1 revision,
  edited {objects, move, 'Im1,Im2', count 2}, Im1 [102,162,202,212] and Im2 [430,72,490,112], selection 2 with fresh ids, "Moved 2 objects".
  Save As to the scratch file and reopen through the engine gives Im1 [102,580,202,630] and Im2 [430,680,490,720] with Fm1 unchanged (s1/s2).
  Escape mid-drag sends no request and every member returns. A drag of +300,-200 clamps to Im2 [552,0,612,40] and Im1 [224,90,324,140], refused stays [] (shot b4).
  Plain ↓ on a focused member moves the group 1 pt, and Shift+→ moves it 10 pt. Group Enter does nothing.
- c: during a single ⌥-drag, gesture.kind is 'duplicate' with 1 ghost. On drop: 4 objects, copy [460,92,520,132], ghost gone, selection and
  active = the NEW id p0.c0.283 (the original is p0.c0.245), edited without count, "Duplicated image" (shot c1). A group ⌥-drag gives 5 objects,
  count 2, copies [92,162,..] and [420,72,..] selected, "Duplicated 2 objects". Both undos give 3.
- d: a right-click on Fm1 opens the menu at the pointer (±1 px) inside #viewport. Replace is disabled in the DOM, in aria and in STATE, and focus is on Duplicate (shot d1).
  ↑/↓ skip Replace. Home and End stay in the menu with page 0. Escape refocuses Fm1. Enter adds exactly +1 revision and a copy at [410,272,510,302],
  "Duplicated Form Fm1". Space adds +1 (Delete). A click on empty space closes the menu with no add editor and active null. A right-click on a non-member
  of a group makes it the single selection. A right-click on a member keeps count 2 with Replace off (shot d8), and menu Delete there gives 1 request, count 2,
  "Deleted 2 objects", selection []. Control-click opens the menu with no gesture, no revision and Im1 unchanged. Shift+F10 opens it with focus on Replace.
  Tab closes it and refocuses the anchor. A wheel scroll, ⌘= zoom and PageDown each close it. The corner Im2 menu Duplicate gives [542,10,602,50]
  (−10 x, +10 y), selected (shot d0).
- §9.16 drive: `menu` Fm1, the same point again, then Im2 while open re-opens for each target, and `item duplicate` gives objects 4 (steps 2-5 ok).
- f: index.html:551/552/562 read as §2.6/§9.27 ("right-click, Control-click or Shift+F10", the new `<li>` without rotation). CONTENT_DETAIL_TEXT
  has "invalid-selection" and no "invalid" (renderer.ts:3584). CSS uses the tokens and `:not([hidden])`. p2-shell (helpRows 11) passes.
- g: a single Im1 drag gives edited {image, move} with no count. The se handle resizes. A real form2 `name` grip drag gives [200,88,500,112] with revision 1.
Shots in checks/review-out/: review-u5-a1-multi (two multi outlines, Inspector still on Im1 = R2), b2-group-drag (both moved),
b4-clamp (block at the top-right edge), c1-alt-duplicate (copy with handles, original outlined, Inspector stale = R2), d0-corner-dup (copy left of
the corner), d1-menu-fm1 (menu with Replace dimmed), d8-group-menu (count-2 menu), d13-after-pagedown (page 2, no menu), e1-escape-release
(stray add-text editor = R3), g-single (resized Im1).
