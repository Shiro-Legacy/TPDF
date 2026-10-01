# P6 U6 renderer evidence (2026-09-24)

Files: src/renderer/renderer.ts, src/renderer/index.html, src/renderer/styles.css,
tests/p2-shell.test.mjs (10 ids added, L49-50), tests/p6-direct.test.mjs (snapRect/within/turnRect/mapRect cases).
src/preload.cts and src/main.ts were not changed.

Shipped (§2.1-§2.7 as amended by §9.13-§9.28):
- Object menu gets rotate-cw/ccw, flip-h/v and crop (menuitemcheckbox). Inspector row: content-rotate-*, content-flip-*, content-crop (aria-pressed).
- Rotate/flip for one object or a group, about the union centre (turnRect), clamped to the page; flips swap on axis-swapping pages.
- Crop mode: directCrop, crop-full frame, within = full∩page. Esc/Enter or a press outside leaves it; arrows and Delete are swallowed. STATE direct.crop {name, full}.
- Group resize: group-box with 8 handles, members mapRect'd, min from groupMin; ⌥⌘←/→ from keys and drive.
- Guides: exported snapRect (4 pt, tie order x0,x1,cx / y0,y1,cy, then page). Snap is on for drag, resize, ⌥-duplicate, group and crop; ⌘ (drive meta:true) turns it off.
- Form pass-through: .wrapping plus span.ring, 6 px evenodd band.
- Text: invalid-transform detail; unsupported-transform wording "Skewed, nested or fully clipped placement"; announcements Rotated/Flipped/Cropped (single and group); Help matrix, gesture cell, ceilings.
- STATE contentSummary objects gain orient and full (view, 0.01).

Numbers (scratch drive .quad/scratch/p6u6/renderer/r7.jsonl on fixtures/p6/objects6.pdf, report r7.json):
- Im1 rotate cw -> view [97,117,147,217], orient {swap:true,sx:1,sy:-1}. Im2 -> [375,67,475,117] {sx:-1,sy:-1}.
- Crop Im1 -> view [72,142,132,192], full [72,142,172,192]; crop grip +20 -> [92,142,152,192].
- Im4 crop 'w' dx -20 at the full edge is a no-op (revision stays 8).
- Group resize -> Im1 [72,142,272,192], Im3 [328,142,528,192]. Group rotate -> [161,53,211,153] / [161,181,211,281].
- Snapped drag -> [400,142,500,192]; meta drag -> [398,142,498,192]; ⌥⌘→ -> [72,142,172.44,192] / [200.56,142,301,192].
- 59/60 steps ok. Step 20 failed only on expect error:null; STATE reports "" (see tests note).

Final hold (under run.lock): npm run build OK; drive.mjs direct-edit, direct-edit2, direct-edit3, direct-edit6, debt1, object-edit all exit 0.
node --test p6-direct, p2-shell, p3-drive, p6-drive3, p6-drive6: 33 pass / 0 fail. fixtures.sha256: 9 OK. No stray drive Electron processes.

D4 re-pins: none by the renderer.

Notes for the tests seat:
- The drive template in objections-p6-unit6-tests.md uses open {contents}. The drive expects {"cmd":"open","path":...}, and mode takes contents {"mode":"edit"}.
- STATE error is "" when there is no error, not null. Expect "" or omit it.

Not done: no new tests/drive/*.jsonl (owned by the tests seat); preload/main untouched (nothing needed).
Risks: group lockAspect only when every member is an image; the guide filter matches any final edge on the axis;
snapping also applies to single-object handle resize; Inspector crop on a non-member focuses it first.

## Fixes after review
Reproduced first under the lock: `ONLY=menu,group,crop,group2,crop2` review-u6-renderer-probe.mjs failed a3, b5, c4, c6, c7, b8, c9, c12 (plus a6, b4) → scratch review-renderer/fix-pre.out.
- R1 (blocking): new `swallowNextClick()` (a capture `click` listener, `once`, dropped on the next tick when no click arrives). `startRectGesture` `end` calls it when `moved`. `cancel` arms it on the next `pointerup`. The release that lands off the object no longer reaches pageRegion/clickToAdd. Drive clicks await rAF before they dispatch, so the tick removal can't eat one.
- R2: `if (event.key === "Escape" && directGesture) return;` is now hoisted above `if (cropping())` in the object button keydown.
- R3: the contentObjectForm Escape runs `if (directCrop) { leaveCrop(); return; }` before it closes the form, the same as the drive's key Escape.
- R4: the end of restoreDirectFocus calls `syncObjectInspector(directFocusedObject.targetId)` when `contentObjectTarget` isn't the anchor. The rescan's renderContentTargets clears it, so after undo/redo the form reopens on the anchor. A repaint that keeps the anchor leaves typed values alone.
- Test: tests/p6-direct.test.mjs "U6 review: …" pins the four wirings. Each one is absent in the pre-fix source. The real-input check is the review probe itself: after the fix the same sessions print ALL CHECKS OK (fix-post.out, 69 ok; a6 and b4 also pass).
- Ran under the lock: tsc, build, and node --test p6-direct/p2-shell/p3-drive (32/32). Drives direct-edit7/6/3, debt1 and object-edit are all ok:true. fixtures.sha256 is 9 OK, and the hash diff is just the objects6.pdf line. No `--drive=` stragglers.
- Tests owner (optional): a shot-direct7 CDP check that an outward crop drag released off the object keeps crop mode and opens no add editor. The probe's c4 already covers this.
