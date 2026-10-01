# P6 U6 objections — tests / drive / evidence lens (Opus 5.5, 2026-09-24)
Scope: contract-p6-unit6.md §4.1–§4.5, §2.6 drive lines, §3 boxes as test input, §5 tests lines, §6.3 acceptance list, D4 fallout on existing pins. Refute-first; the only scope added is drive coverage of gestures the contract already specifies (rotate-ccw, flip-v, group flip, group ⌥⌘→, crop Enter). Scratch: `/private/tmp/claude-501/-Users-user-Documents-pdf4all/32f48564-8a43-48b6-83ae-ad0dab85ff09/scratchpad/objections-tests/orient.mjs` (plain node; orient of every transform by §1.1's rule, new cm = cm × T in PDF row-vector form). No project file read-modified; no npm/Electron run.

| id | § | sev | evidence | amendment |
|---|---|---|---|---|
| T1 | §4.1 group rotate | blocking | §3 Im3 cm `-100 0 0 50 300 600` (contract:39). Page-space cw T = `[[0,-1],[1,0]]`; `[[-100,0],[0,50]]·T = [[0,100],[50,0]]` → b=100, c=50 → §1.1 (contract:11) `{swap:true, sx:sign(c)=1, sy:sign(b)=1}`. §4.1 (contract:42) pins both members `orient {swap:true,sx:1,sy:-1}`; that holds for Im1 only, so the deep-equal in p6-drive7 (§4.2) goes red. (Pre-multiplying would give `{true,-1,-1}`, also not the contract's value.) | §4.1 group rotate: "Im1 `orient {swap:true,sx:1,sy:-1}`, Im3 `orient {swap:true,sx:1,sy:1}`." |
| T2 | §4.1 crop Im4 | blocking | §3 Im4 is `72 400 60 40 re W n` inside `100 0 0 50 72 400 cm` (contract:39) → visible view `[72,352,132,392]`, full `[72,342,172,392]`. `handle e dx 40` moves only x1: `[72,352,172,392]`, whose y0 352 ≠ full y0 342. The crop does not equal the full box, so the clip stays and `full` stays; "`full` absent (the crop equalled the full box, clip removed)" (contract:42) is false and the step goes red. | §4.1: "`handle {handle:'e', dx:40}` → Im4 `view [72,352,172,392]`, `full [72,342,172,392]` (the clip still hides the top 10 pt), `revision 9`." The clip-removal case stays proven by the engine (§1.4). |
| T3 | §2.6, §4.1 meta drag | blocking | contract:34 says `driveDirectGesture` passes `metaKey` "on the moves". startRectGesture's `end` (renderer.ts:2360-2362) calls `update(next.clientX, next.clientY, next.shiftKey)` with the **pointerup** event, so a meta-less pointerup re-snaps. Result: `[400,142,500,192]`, not the pinned `[398,142,498,192]` (step 39 red). The `fire` helper (renderer.ts:1350) only carries shiftKey/altKey. | §2.6: "`driveDirectGesture` passes `metaKey: value.meta === true` in `fire` (pointermove **and** pointerup) and on the pointerdown; `update` takes the event's `metaKey` on move and on `end`; the CDP script sends `modifiers: 4` on mousePressed, every mouseMoved and mouseReleased." |
| T4 | §4.5 shot fix | blocking | Setting `#viewport.scrollTop` inside `shot()` fires the viewport `scroll` listener, which calls `closeObjectMenu` (renderer.ts:2690). Shot B (`u5-b-menu-fm1.png`) exists to show the open menu, so a scroll inside shot() leaves it empty. The offset comes from `pagePoint` (shot-direct6.mjs:145-151), which re-centres every point: y=72 clamps near scrollTop 0, y=277 scrolls about 20 px further. | §4.5: "`pagePoint(x, y, scroll = true)`. The first call (Im1 row) scrolls and saves `scrollTop`. Every later call passes `scroll=false` (the review-u5-renderer-probe.mjs:130-136 form). `shot()` sets nothing; it only asserts `viewport.scrollTop === saved` before capture." |
| T5 | §4.3(b), §2.4 | blocking | Drag Im1 by (326,0) → raw `[398,142,498,192]`. x: x0 398→400 (Im2 x0) is Δ+2, and cx 448→450 (Im2 x1) is also Δ+2, a tie across moving edges that §2.4's tie rule (contract:29) does not order. y: y0 142 = Im2 y1 142 = Im3 y0 (Δ0), so a **y guide at 142** is present too. "one `{axis:'x', at:400}`" (contract:44) is false and the CDP assert goes red. | §2.4: "ties: moving edges in order x0, x1, cx, y0, y1, cy; for each edge, candidates in `others` order, each rect's edges x0, x1, cx (y0, y1, cy); page last; strictly smaller |delta| replaces." §4.3(b): "`guides` deep-equals `[{axis:'x',at:400},{axis:'y',at:142}]` during the drag; with ⌘ it is `[]`." |
| T6 | §2.6 group handle | should | contract:34: "resolves `handle` against the group box when the hit target is a `multi` member". At (300,167) the hit is the group box's e handle, and `target.closest('.hit-target')` is null (the group box has no `hit-target`, contract:25). The inclusive fallback (renderer.ts:1342) picks **Im3** (x=300 is its right edge), then `querySelector([data-handle=e])` returns Im3's hidden handle, with rect 0,0. The rule works only if the code keys on the fallback `fieldTarget` and the group-box handles carry `data-handle`; neither is stated. | §2.6: "`driveDirectGesture`: `const owner = target?.closest('.group-box') ?? (fieldTarget?.classList.contains('multi') ? hitLayer.querySelector('.group-box') : fieldTarget)`; the handle is `owner.querySelector('[data-handle=…]')`. Group-box handles set `dataset.handle` exactly as addFieldHandles (2311-2318)." |
| T7 | §2.3, §2.6 crop keys | should | driveDirectKey does not dispatch Escape to any handler. After a click `directActive` is set, so `onTarget` (1390) is null and Escape reaches 1404: `closeDirect(); dropDirectFocus()` clears the selection, and step 17's "`direct.active` still Im1" goes red unless a crop branch is added. Enter reaches 1402 `finishDirect()` (or, with no directActive, the focused button's keydown → Replace, a native dialog that hangs the step). | §2.6: "driveDirectKey gains, right after the menu branch (1375-1380): `if (directCrop && (key==='Escape' \|\| key==='Enter')) { leaveCrop(); return; }` (selection and active untouched)." |
| T8 | §4.1 pass-through | should | "the same expect the existing add-text drive `tests/drive/content-edit.jsonl` uses" (contract:42) is wrong: content-edit.jsonl is five panel-edit lines on edit-transformed.pdf with no add-text step. The add-text open expects are direct-edit6.jsonl:64 and direct-edit3.jsonl:35. | §4.1: "→ `{"direct":{"selection":[],"active":{"kind":"add","page":0}}}` (the direct-edit6.jsonl:64 form); this also proves `direct.active` is not Fm1." |
| T9 | §4.1 tail | should | "`revision` ledger continues" (contract:42) is false: opening a document restarts `revision` at 0 (direct-edit6.jsonl opens at `revision 0`). Opening a second file while `dirty` raises the unsaved prompt, and a hung dialog is the known failure. There is no `mode edit` before the direct3 menu line. | §4.1: "`mode read` → `selection []`, `crop null`, `revision 28`, `dirty false`; `open fixtures/p6/direct3.pdf` → `revision 0`; `mode edit`; `menu` Im1 `rotate-cw` → `view [97,117,147,217]`, `revision 1`; `undo` → `revision 2`, `dirty false`." |
| T10 | §4.1 line shape | should | packDrive (scripts/drive.mjs:15-28) folds only click/type/key/drag/handle, so `menu` lines must carry `contents` (direct-edit6.jsonl menu lines). driveDirectGesture returns silently without `page`,`x`,`y` (renderer.ts:1319), yet §4.1's crop `handle {handle:'e', dx:-40}` lines give none. driveObjectMenu rejects anything but replace/duplicate/delete (renderer.ts:1295). | §4.1: "every `menu` line is `{"cmd":"menu","contents":{page,x,y,item?}}`; every `handle` line carries `page,x,y` (the owning object's centre; for a group, the handle point); driveObjectMenu's item check (1295) and its error text grow to the eight items." |
| T11 | §4.1 vs §4.2 | should | `content.objects` holds every non-text item (renderer.ts:4189-4205, 6 on page 0) and the subset match needs exact array lengths (contract:7), so a jsonl `expect` cannot pin one object's `view`/`orient`/`full` without spelling all six. | §4.1: "jsonl expects pin `revision`, `dirty`, `content.edited`, `direct.{active,selection,menu,crop}` only; every `view`/`full` is asserted in p6-drive7 via `all()` ±0.5 and every `orient` via deep-equal on `content.objects.find(name)`." |
| T12 | §2.4 | should | "the returned rect is the input translated by the winning deltas, never resized" (contract:29) contradicts "a side/corner handle = only the edges that handle moves": translating the whole rect on an `e` snap would move x0 too. | §2.4: "grip: the rect is translated by the winning deltas; a handle: only the moving edges shift by their axis delta (the opposite edge stays)." |
| T13 | §2.6 `direct.crop` | should | §2.3 (contract:27) paints `.crop-full` "on the box when uncropped" but `direct.crop.full` is only given as "view space". Step 14 expects Im1 (uncropped) `full [72,142,172,192]`, which needs the rule. | §2.6: "`direct.crop.full` = the member's `full` when cropped, else its visible box." |
| T14 | §4.5 probe split | should | review-u5-renderer-probe.mjs:21 `ONLY` defaults to `sel,save,form2`; `fix` (:401) and `esc` (:485) are not run. d0 (:254-258) presses Enter on the focused Replace item (native dialog; the d0y form at :508 uses ArrowDown first). d11 (:367) uses the menu d10 (:363) opened. b4 (:261) "undo ×2" relies on d-checks having mutated. d0x (:433) repeats the Enter/Replace bug. | §4.5: "each of d0, d6, d8, d9, d10 moves into `if (ONLY.includes('<tag>')) await session('<tag>', 'fixtures/p6/direct3.pdf', {}, …)` with `mode edit` and a wait for `direct.objects === 3`; the default ONLY lists the five new tags; d0 uses ArrowDown before Enter (d0y form); d11 opens its own menu; b4's expected revision is recomputed; the `fix` copies (d0x..d10x) are deleted." |
| T15 | §4.1 coverage | should | rotate-ccw, flip-v, group flip and group ⌥⌘→ (contract:23, 25) and crop-mode Enter (contract:27) have no drive line; §0 needs every new gesture driven, yet each could be absent while the script stays green. | §4.1: insert after the guides block (rev 20) the 10 lines of "Exact line templates" 41-50 (rev 21-28), and make the second crop exit `key Enter`. |
| T16 | §4.1 header | should | "Ledger and step/click counts pinned in the header" gives no numbers. As written: 50 steps / 15 clicks; amended: 60 / 17 (recomputed below). p6-drive6 pins both numerically and against the header (tests/p6-drive6.test.mjs:88-96). | §4.1: "header: `60 steps, 17 click lines`; ledger 0..28 then 0..2." |
| T17 | §2.6 inspector | nit | Inspector buttons are proven only by CDP (§4.3(c)(d)). This is acceptable because the menu runs the same op, and contract:34 already says so. A §4.3 note should add that `#content-crop` toggling off is also CDP-only. | — |
| T18 | §4.1 band | nit | Band click (75,600) is 3 pt from Fm1's x0 = 3.9 px at fit-width (≈1.3 px/pt), inside the 6 px band. The margin is 2 px, and it goes to 0 at s ≤ 0.5 pt/px. It is fine at the drive's fit-width. | Optional: "(74,600)". |
| T19 | §4.1 text | nit | "`click` on 'Over' (its centre from the fixture file)" leaves the point open. Helvetica 14 "Over": O 778 + v 500 + e 556 + r 333 = 2.167 em = 30.3 pt, baseline y-down 542 → centre ≈ (115,537). | "`click` (115,537)". |
| T20 | §2.6 | nit | CONTENT_REASON_TEXT `unsupported-transform` wording (renderer.ts:3602) becomes partly stale once swapped/flipped/clipped placements are admitted. | Renderer seat may reword; no pin reads it. |
| T21 | §4.3 | nit | shot-direct7 PNG names `u6-{a,b,c}-*`, but (d) and (e) have no PNG and `Read` covers only a–c. | "(d)/(e) STATE-only." |

## Exact line templates (amended script; view space, y down; ledger in brackets)
```
1  {"cmd":"open","contents":"fixtures/p6/objects6.pdf","expect":{"revision":0}}
2  {"cmd":"mode","contents":"edit","expect":{"direct":{"objects":6,"selection":[],"menu":null,"crop":null}}}
3  {"cmd":"click","page":0,"x":122,"y":167,"expect":{"direct":{"active":{"kind":"object","name":"Im1"}}}}
4  {"cmd":"menu","contents":{"page":0,"x":122,"y":167},"expect":{"direct":{"menu":{"name":"Im1","items":{"replace":true,"duplicate":true,"rotateCw":true,"rotateCcw":true,"flipH":true,"flipV":true,"crop":true,"delete":true}}}}}
5  {"cmd":"menu","contents":{"page":0,"x":122,"y":167,"item":"rotate-cw"},"expect":{"content":{"edited":{"kind":"image","action":"transform","name":"Im1"}},"revision":1,"dirty":true}}
6  {"cmd":"undo","expect":{"revision":2,"dirty":false}}
7  click Im1 (122,167);  8 menu Im1 item flip-h [3];  9 undo [4]
10 click Im2 (425,92);   11 menu (425,92) item rotate-cw [5];  12 undo [6]
13 click Im1
14 {"cmd":"menu","contents":{"page":0,"x":122,"y":167,"item":"crop"},"expect":{"direct":{"crop":{"name":"Im1","full":[72,142,172,192]},"menu":null}}}
15 {"cmd":"handle","page":0,"x":122,"y":167,"handle":"e","dx":-40,"expect":{"content":{"edited":{"kind":"image","action":"crop","name":"Im1"}},"revision":7}}
16 {"cmd":"handle","page":0,"x":122,"y":167,"handle":"grip","dx":20,"expect":{"revision":8}}
17 {"cmd":"key","key":"Escape","expect":{"direct":{"crop":null,"active":{"kind":"object","name":"Im1"}}}}
18 click Im4 (102,372);  19 menu (102,372) item crop → crop {name:'Im4', full:[72,342,172,392]}
20 {"cmd":"handle","page":0,"x":102,"y":372,"handle":"w","dx":-20,"expect":{"revision":8,"dirty":true,"error":null}}
21 {"cmd":"handle","page":0,"x":102,"y":372,"handle":"e","dx":40,"expect":{"revision":9}}
22 {"cmd":"key","key":"Enter","expect":{"direct":{"crop":null},"revision":9}}
23-25 undo ×3 [10,11,12]
26 click Im1;  27 {"cmd":"click","page":0,"x":250,"y":167,"shift":true,"expect":{"direct":{"selection":[{"name":"Im1"},{"name":"Im3"}]}}}   (member keys per U5 selection shape)
28 {"cmd":"handle","page":0,"x":300,"y":167,"handle":"e","dx":228,"expect":{"content":{"edited":{"kind":"objects","action":"resize","name":"Im1,Im3","count":2}},"revision":13}}
29 undo [14];  30 click Im1;  31 click shift Im3
32 menu (122,167) item rotate-cw → edited {kind:'objects',action:'transform',name:'Im1,Im3',count:2} [15];  33 undo [16]
34 key Escape → selection [];  35 click Im1
36 {"cmd":"drag","page":0,"x":122,"y":167,"dx":326,"dy":0,"expect":{"revision":17}}
37 undo [18];  38 click Im1
39 {"cmd":"drag","page":0,"x":122,"y":167,"dx":326,"dy":0,"meta":true,"expect":{"revision":19}}
40 undo [20]
41 menu (122,167) item rotate-ccw → edited transform Im1 [21];  42 undo [22]
43 menu (250,167) item flip-v → edited transform Im3 [23];  44 undo [24]
45 click Im1;  46 click shift Im3 (250,167)
47 {"cmd":"key","key":"Alt+Meta+ArrowRight","expect":{"content":{"edited":{"kind":"objects","action":"resize","name":"Im1,Im3","count":2}},"revision":25}}
48 undo [26] → selection still 2
49 menu (122,167) item flip-h → edited {kind:'objects',action:'transform',count:2} [27];  50 undo [28]
51 {"cmd":"click","page":0,"x":200,"y":600,"expect":{"direct":{"selection":[],"active":{"kind":"add","page":0}}}}
52 key Escape → active null
53 {"cmd":"click","page":0,"x":75,"y":600,"expect":{"direct":{"active":{"kind":"object","name":"Fm1"}}}}
54 click (325,647) → active Im5;  55 click (115,537) → active.kind 'text'
56 {"cmd":"mode","contents":"read","expect":{"direct":{"selection":[],"crop":null},"revision":28,"dirty":false}}
57 {"cmd":"open","contents":"fixtures/p6/direct3.pdf","expect":{"revision":0}}
58 {"cmd":"mode","contents":"edit"}
59 {"cmd":"menu","contents":{"page":0,"x":122,"y":167,"item":"rotate-cw"},"expect":{"revision":1,"dirty":true}}
60 {"cmd":"undo","expect":{"revision":2,"dirty":false}}
```
Counts: as written 50 steps / 15 clicks (clicks at 3,7,10,13,18,26,27,30,31,35,38 + 4 pass-through). Amended 60 steps / 17 clicks (+41-50: 10 steps, 2 clicks). Both fit `drive(…,150_000)`; direct-edit3 runs 79 steps in the same budget.

## Recomputed geometry (y-down `view`; §3 views Im1 [72,142,172,192], Im2 [400,42,450,142], Im3 [200,142,300,192], Im4 vis [72,352,132,392] / full [72,342,172,392], Fm1 [72,492,372,692], Im5 [300,622,350,672])
- Step 5 Im1 cw `[97,117,147,217]` orient `{true,1,-1}` ✓. Step 8 Im1 flip-h view unchanged, `{false,-1,1}` ✓. Step 11 Im2 cw `[375,67,475,117]` `{false,-1,-1}` ✓ (Im2 pristine `{true,1,-1}` ✓).
- Step 15 `[72,142,132,192]` full `[72,142,172,192]` ✓ (x1 132 = Im4 x1, Δ0 snap). Step 16 `[92,142,152,192]` ✓ (only Δ0 y candidates).
- Step 20 Im4 w: silent no-op ✓ (within clamp returns the same rect → startRectGesture `place(initial)`, no commit). Step 21: `[72,352,172,392]`, full stays (T2).
- Step 28: Im1 `[72,142,272,192]`, Im3 `[328,142,528,192]` ✓, no candidate within 4 of x1 528.
- Step 32: union centre (186,167) → Im1 `[161,53,211,153]`, Im3 `[161,181,211,281]` ✓. Orient Im1 `{true,1,-1}`, **Im3 `{true,1,1}`** (T1).
- Step 36 `[400,142,500,192]` ✓, guides during move x (400) + y (142) (T5). Step 39 `[398,142,498,192]` ✓ once T3 is applied.
- Step 41 Im1 ccw `[97,117,147,217]` `{true,-1,1}`. Step 43 Im3 flip-v view `[200,142,300,192]` `{false,-1,-1}`.
- Step 47 union x1 300→301 (scale 229/228): Im1 `[72,142,172.4386,192]`, Im3 `[200.5614,142,301,192]`; keyboard path, no snap.
- Step 49 group flip-h about x=186: Im1 `[200,142,300,192]` `{false,-1,1}`, Im3 `[72,142,172,192]` `{false,1,1}`.
- Step 59 direct3 Im1 `[97,117,147,217]`. Ledger 1..20 as written is correct apart from T2's content; undo ×3 → 10, 11, 12 ✓.

## checkedFine
- Check 7 / D4: tests-seat `unsupported-transform` pins are tests/p6-drive3.test.mjs:121 and tests/drive/direct-edit3.jsonl:44. Both are the page-1 `/Rotate` add-text refusal (renderer.ts:3167-3178, engine.ts:4581), not a placement rule, so neither flips. Engine-seat pins (p6-direct3:215, p5-sign-field:418, p6-objects5:167/290, p3b-content:243, p3-targets:493-522, p3-objects:131-338) belong to the engine lens. direct-edit.jsonl:55-69 uses edit-transformed.pdf, whose image is `0.707 0.707 -0.707 0.707 … cm 64 0 0 64 0 0 cm` (fixtures/p0b/make-edit.py:87): 45° and double cm, still refused, clicks unaffected. make-direct3.py places everything plain, so direct3 drives are unchanged.
- Snapping does not move existing drive gestures: direct-edit3 lines 11/14/15/17/18/103 and direct-edit6 lines 18/23/41/73, plus U5 probe drags b2/b4/c1/c2/g1, have only Δ0 candidates or none within 4 pt. p6-drive3's ±0.05 pins (Im2 grip [450,82,510,122], Im1 se [72,580,212,650], Fm1 [380,500,480,530]/[380,480,490,530]) hold.
- Engine §1.4 Im4 resize/rotate numbers recomputed and correct.
- Check 9 evidence: fixtures.sha256 goes 8 → 9 lines (contract:7, 39); u6-hash-before 54 → 55; `tests/drive/debt1.jsonl` exists (33 lines) for §6.3. p6-drive7 must pin before/after sha of objects6.pdf **and** direct3.pdf (§4.2 says both ✓).
- `key Alt+Meta+ArrowRight` parses: prefix loop Alt, Meta, Shift (renderer.ts:1371).
- Pass-through (200,600) misses Over (≈[100,532,130,545]) and Im5, so it lands on empty space inside a wrapping Fm1 → add-text (3169).

## buildRisks
- Snap order vs clamp: snap before `gestureRect`'s clamp (contract:29); in crop mode the `within` clamp can undo a snap. Harmless at the pinned numbers.
- `.ring` clip-path hit-testing: `elementsFromPoint` honours clip-path in Chromium; a point inside the band but on a 0.5 px AA edge can miss. Keep drive points ≥ 2 px inside the band.
- Group-box handles need `data-handle` (T6), and `.group-box`/`.crop-full`/`.guide` must not carry `object-target`, or `direct.objects` becomes 7+ and step 2 goes red on later repaint checks.
- Step 48 → 49 relies on the selection re-resolving after undo (U5 §2.1). Menu is point-hit, so the flip still works if focus moved.
- Step 47 needs the group ⌥⌘→ handler reachable from driveDirectKey: `onTarget` requires `!directActive` and focus on a member button (1390). If a group sets `directActive`, the key falls to 1396-1399 (fields only) and does nothing. The renderer seat must route it.
- Inspector `#content-crop` click in §4.3(c) is a "click anywhere else" per §2.3 unless the leave rule excludes the Inspector. State the exclusion.
- driveObjectMenu's item check and error string (renderer.ts:1295) change; no test or check matches the old text (grep "item must be" over tests/ and checks/ is empty), so nothing needs re-pinning.
- A crop-mode `handle` resolves `fieldTarget` via `target.closest('.hit-target')`; `.crop-full` sits under the button with `pointer-events:none`, so this is fine as long as it stays inside `#hit-layer` without the `hit-target` class.
