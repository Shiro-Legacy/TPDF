# P6 U6 — tests seat evidence (Opus 5.5, 2026-09-24)
Files (new): tests/drive/direct-edit7.jsonl (60 steps, 17 click lines, ledger 0..28 on objects6.pdf then 0..2 on direct3.pdf), tests/p6-drive7.test.mjs,
.quad/shared/p6/checks/shot-direct7.mjs. In place: scripts/drive.mjs:25 ('meta' appended to the packDrive key list; the rest is byte-identical),
.quad/shared/p3/design-agent-drive.md (appended '## P6 U6 drive additions', 8 lines), checks/shot-direct6.mjs (§9.30), checks/review-u5-renderer-probe.mjs (§9.32).
Script = objections-p6-unit6-tests.md template with §9.14 (step 21 ne dx 40 dy -10), §9.24 band (74,600), T19 (115,537), pass-through expect {selection:[],active:{kind:'add',page:0}},
second crop exit key Enter. Deviations from the template text: open/mode lines use the house form ({"cmd":"open","path":…}, mode contents {"mode":…}), and
step 20 expects "error":"" rather than null (STATE error is "" when there is no error).
p6-drive7: pins 60/17 from the header, exit 0, zero red; sha of objects6.pdf (ca0a5988…2ab8) and direct3.pdf (e3ae5a43…7466) before and after; every view/full of
§4.1 and 'Recomputed geometry' checked ±0.5 with all(), each object found by view and then checked for its name; orient deep-equal (group rotate Im1 {swap:true,sx:1,sy:-1}, Im3 {swap:true,sx:1,sy:1});
pristine (6 objects, all orients, Im4 full) after every undo; Im4 full absent after ne; direct3 rotate + undo.
Runs (each under .quad/run.lock, build green with the engine + renderer seats' code):
- npm run build → 0; node scripts/drive.mjs tests/drive/direct-edit7.jsonl → exit 0, 60 steps, 0 red; node --test tests/p6-drive7.test.mjs → 1 pass 0 fail.
- shot-direct7.mjs → PASS all (A group box 1, 8 handles shown, member handles 0, 6 object targets; east drag → objects resize count 2, Im1 [72,142,272,192] Im3 [328,142,528,192];
  B guides during drag [{x,400},{y,142}], .guide painted, none after; ⌘ → [] and [398,142,498,192]; C #content-crop → .cropping 1, .crop-full 1, aria-pressed true, east −40 → crop
  [72,142,132,192] full [72,142,172,192], mode stays on, #content-crop again → crop null; D #content-rotate-cw → orient {true,1,-1}; E interior hit null (not Fm1), band → Fm1).
  First run failed only in the script: #content-crop sat below the window (y 979 > 900), so centre() now scrolls Inspector controls into view inside the Inspector first.
- shot-direct6.mjs → PASS all; review-u5-renderer-probe.mjs (default ONLY = d0,d6,d8,d9,d10) → ALL CHECKS OK (8 checks, 5 sessions).
- shasum -a 256 -c checks/fixtures.sha256 → 9 OK after every run; pgrep -fl -- '--drive=' empty.
What the PNGs show (viewed with Read):
- u6-a-group-box.png: Im1 and Im3 inside one purple frame with 8 handles; the members have no handles of their own.
- u6-b-guides.png (taken mid-drag): the Im1 outline sits at Im2's left edge. A vertical guide runs at x 400 and a horizontal one at y 142 across the full page.
- u6-c-crop-mode.png: Im1 with filled crop handles and a dashed full frame; the Inspector Crop button shows as pressed.
- u6-c-cropped.png: Im1 visible at 60×50 pt with the cropped-away part shown as a pale dashed box; the Inspector reads "Image 60×50 pt".
- u5-a/b/c re-shot: the three share one scroll offset (Im1 row at the same y in a and b); b shows the menu open with its eight items and Replace dimmed.
Observation, not a failure: during a drag only the hit outline moves (the picture repaints at commit), as in U5.
Probe split (§9.32): d0/d6/d8/d9/d10 each have their own menuSession (mode edit, wait for objects === 3, one centring scroll). d0 uses ArrowDown before Enter.
d11 opens its own menu in 'sel'. b4 does one undo and checks revision rEdge+2. The fix copies d0x/d6x/d8x/d9x/d10x were deleted (b3x, b4x and d14 stay).
D4 re-pins: none (§9.5: p6-drive3:121 and direct-edit3.jsonl:44 are the /Rotate add-text refusal, unchanged).
Not done: nothing. Sessions sel/fix/esc/save/form2 of the probe were not re-run (not in the default ONLY).

## Fixes after review
- TS1 (§2.1, §9.22): reproduced — no drive/test step read `direct.menu.items` on a group or on a rotate that cannot fit.
  `tests/drive/direct-edit7.jsonl`: after the group shift-click (Im1+Im3) a `menu {page:0,x:122,y:167}` (no item) expects
  `{count:2, items {replace:false, duplicate:true, rotateCw:true, rotateCcw:true, flipH:true, flipV:true, crop:false, delete:true}}` (rev 14).
  After the group flip-h undo (rev 28): `click` Im1, `handle s dy +588` (Im1 view [72,142,172,780], 638 pt tall > 612 pt page width) rev 29,
  `menu` → `items {rotateCw:false, rotateCcw:false, flipH:true, flipV:true, crop:true}`, `key Escape` → menu null, `undo` rev 30.
  Later objects6 revisions 28 → 30; header now "66 steps, 18 click lines", ledger 0..30.
  `tests/p6-drive7.test.mjs`: pins 66/18; asserts group menu crop false, Im1 tall view ±0.5, rotateCw/Ccw false, pristine after undo.
  A build that leaves Crop enabled on a group or rotate enabled on a non-fitting box now fails the drive. Run: tsc clean; build + drive 66/66 green; node --test 1/1 pass; fixtures unchanged.
