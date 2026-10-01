# P6 U6 review: renderer + human UX lens (real CDP mouse and keys). Opus 5.5, 2026-09-24
Probe `.quad/shared/p6/checks/review-u6-renderer-probe.mjs` (sessions menu, group, crop, guides, pass, form2, direct3, plus group2/crop2
re-runs chosen with `ONLY=`). Both runs held .quad/run.lock. `npm run build` exited 0. Run 1: 9 FAIL. Two sessions aborted after the first
real defect left the page in an unexpected state. Run 2 (group2, crop2): 3 FAIL. Outputs are in .quad/scratch/p6u6/review-renderer/probe{1,2}.out.
fixtures.sha256 is 9/9 OK. The u6-hash-before diff is exactly one added line (objects6.pdf). `pgrep --drive=` is empty.
Verdict: REVISE. One blocking finding and three should findings, all in renderer.ts.

## Findings
R1 (blocking, §2.3/§9.18 "directCrop stays on", "the click that ends cropping adds no text"; §9.19 min rule). renderer.ts:2443-2454
(`startRectGesture` end). After a moved gesture, the real trailing `click` goes to the common ancestor (pageRegion) whenever the release
lands off the moved element: a clamped crop handle, a clamped grip, or a group box held at its min. `pageRegion`'s pointerdown never ran
(stopPropagation at :2404), so `addBlocked` is stale. Also, `clickToAdd` (:3466) runs dropDirectFocus before it reads `blocked`.
Repro c4: menu Crop on Im1, drag e −40 then −10 (ok), then drag e by (+150, +80)·s. The write is correct (rev 3, view [72,142,172,192]).
But then `direct.crop` is null, the selection is [], and `direct.active {kind:'add', point:[270.8,246.4]}` with 1 editor (shot c4).
Repro b5: group Im1+Im3, west handle +400·s. The members stay at 1 pt (ok) and nothing is refused, but `direct.active.kind 'add'` and the selection is ''.
In run 1 the same thing happened after a clamped grip drag (+80·s). Crop mode was silently gone, so the next body drag MOVED the object
(full [52,142,152,192], c6) and a Delete key DELETED Im1 (objects 5, c7). The user thinks they are cropping.
The drive can never see this because it does not release on the page.
Fix: in `end`, when `moved`, swallow the next click once. For example
`window.addEventListener('click', e => { e.stopPropagation(); e.preventDefault(); }, { capture: true, once: true })`, which also covers cancel.
Tests owner: a CDP check in shot-direct7 (outward crop drag, release off the object → crop still on, no add editor).
R2 (should, §2.2 "Escape mid-drag puts every member and the group box back"; §9.18 "while directGesture is set the button keydown ignores Escape").
renderer.ts:2978. The guard only exists inside `if (cropping())` (:2974). For any other gesture the focused button's Escape runs first and calls
dropDirectFocus. Repro b8 (group2): group box east drag, Escape before release. There is no request (rev 6 = 6) and the members snap back, but the
`.group-box` is removed, the selection is dropped, and the members keep a stale `.multi` class. Fix: hoist `if (event.key === "Escape" && directGesture) return;`
to the top of the button keydown.
R3 (should, §2.3 "Leave: Escape"; §0 human/AI parity). renderer.ts:3233 (inspectorTransform focuses #content-crop) and :8305-8308 (the object-form
Escape only closes the form). Repro c12: select Im1, click #content-crop (crop on, focus on #content-crop), press real Escape. The form closes,
focus goes to content-target-0, and `direct.crop` stays {Im1}. c9: the same with focus in #content-box-w. The drive's `key Escape` always leaves
(:1401), so drive and human disagree. Fix: in the contentObjectForm keydown (or a window capture keydown), `if (directCrop && Escape) { leaveCrop(); return; }` before closing the form.
R4 (should, §9.26 "…and after restoreDirectFocus, call syncObjectInspector(anchor.targetId)"). renderer.ts:2845-2900 (restoreDirectFocus has
no sync). Repro a3: menu Rotate clockwise on Im1 (the Inspector shows W 50 H 100, ok), then a real toolbar Undo gives rev 2 with Im1 back at
[72,142,172,192]. But the Inspector still reads X 97 Y 575 W 50 H 100, and `contentObjectTarget` is the old id, so Apply edits from a stale form.
Fix: call `syncObjectInspector(directFocusedObject.targetId)` at the end of restoreDirectFocus when an object is focused and the form is open.

## Checked fine (real CDP)
- a: The menu has 8 items in the contract order. Crop is a menuitemcheckbox with aria-checked. ArrowDown cycles all 8 and wraps. STATE items are all true.
  Rotate cw gives rev 1, edited {image, transform, Im1} with no count, view [97,117,147,217], orient {true,1,−1}. The button is re-placed.
  The Inspector shows W 50 H 100. "Rotated image" is announced. Focus is on the Im1 button. Undo restores the view and orient.
  Control-click opens the menu without a gesture. Flip-h gives orient {false,−1,1} and "Flipped image". After an Inspector resize to 100×650,
  Rotate cw/ccw are disabled in the DOM, in aria and in STATE, while Flip stays enabled.
- b: The group has one .group-box, 8 shown handles, no grip and no member handles (shot b1). The east +228·s drag gives edited {objects, resize, count 2},
  Im1 [72,142,272,192] and Im3 [328,142,528,192], and "Resized 2 objects". Save As → reopen gives [72,600,272,650] and [328,600,528,650], and Im3 keeps orient {false,−1,1}.
  A corner drag keeps the aspect. Shift frees it (h 50). The min rule holds (members 1 pt, no refusal). ⌥⌘→ gives count 2 with the union 228→229.
  Group rotate gives [161,53,211,153] / [161,181,211,281] with orients {t,1,−1} / {t,1,1}, "Rotated 2 objects", and the selection and box kept (shot b7).
- c (crop2): .cropping, one dashed .crop-full, handles #6558d8, aria-checked/aria-pressed true, and direct.crop {Im1,[72,142,172,192]}. A second drag still crops.
  Outward drags stop at full and the clip is removed. A grip or body press inside moves the window and keeps full. Arrows and Delete do nothing.
  An Inspector click keeps crop mode. Escape and Enter on the object leave and keep the selection. An empty click leaves with no editor.
  Undo in crop mode leaves the frame, STATE and object in agreement.
- d: Mid-drag STATE guides are [{x,400},{y,142}] and two lines are painted (shot d1). The drop lands at [400,142,500,192] and the lines are gone.
  ⌘ gives [] and 398. Escape removes the guides. The e handle snaps only x1 ([72,142,400,192]).
- e: The Fm1 interior hits the text layer, not Fm1, and a click opens the add editor. The band selects Fm1 with handles, and a right-click opens its menu
  (replace off, crop on). The se handle resizes. Im5 and "Over" are clickable. With Add text armed the layer is not live and the band takes no click.
- f/g: The Help sentences, the unsupported-transform text and the 10 REQUIRED_IDS are present. All buttons are type=button with aria-labels. A form2 field drag
  gives rev 1 with no snap. A direct3 Im1 drag gives {image, move} with no count, and a resize still works.

## Screenshots (checks/review-out/)
- review-u6-a2-rotated.png: Im1 turned upright.
- review-u6-a6-rotate-disabled.png: the menu with Rotate dimmed on a 100×650 Im1.
- review-u6-b1-group.png: one frame with 8 handles.
- review-u6-b5-min-release.png: the stray add editor (R1).
- review-u6-b7-group-rotated.png: Im1 and Im3 stacked vertically, with the group box.
- review-u6-c1-crop-mode.png: crop handles and the dashed full frame.
- review-u6-c2-cropped.png: 60×50 visible inside the dashed full frame.
- review-u6-c4-outward-release.png: Im1 is back to full, crop mode is gone, and an empty add editor sits at the release point (R1).
- review-u6-c14-undo-in-crop.png: the frame on full after undo.
- review-u6-d1-guides-mid.png: the x 400 and y 142 lines across the page.
- review-u6-e3-fm1-band.png: Fm1 selected through its band.
Not run by this lens: the regression drives (direct-edit*, debt1, object-edit, help-fields). The integrator reports them exit 0, and direct3/form2 were covered with real input above.
