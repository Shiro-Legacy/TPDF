# P6 U6 — integration (Opus 5.5 integrator, 2026-09-24)
All runs held .quad/run.lock (no stale lock found; none left). Outputs: .quad/scratch/p6u6/integrate/*.out.

## Requests applied
- None needed. The renderer seat's two requests (drive `open {path}` / `mode {mode}` house forms; STATE `error` is "" not null) were already applied by the tests seat in direct-edit7.jsonl (step 20 expects "").

## Build and suite
- `npx tsc --noEmit` 0; `npm run build` 0.
- `npm test` run 1: 492 tests / 490 pass / 1 fail / 1 skip. Failure: tests/p2-pages.test.mjs:420 "large.pdf: 500-page page ops stay inside the reply budget", "a 500-page page list must cost under 50 ms, not 82.0 ms" (wall-clock budget, not U6 code). p2-pages alone ×3: 13/13 pass each time.
- `npm test` run 2: **492 / 491 pass / 0 fail / 1 skip** (480 + 11 p6-objects6 + 1 p6-drive7).

## Drives (exit code; steps; red)
- direct-edit7.jsonl ×3: 0, 0, 0 (60 steps, 0 red each).
- direct-edit 0 (75), direct-edit2 0 (50), direct-edit3 0 (79), direct-edit4 0 (71), direct-edit5 0 (35), direct-edit6 0 (59), debt1 0 (22), object-edit 0 (11), content-edit 0 (4), help-fields 0 (21). No red step anywhere.

## Hashes
- `shasum -a 256 -c checks/fixtures.sha256`: 9 OK before, after the drives, after the GUI scripts, after suite run 2.
- baseline diff vs u6-hash-before.txt: exactly one added line `ca0a5988…2ab8  fixtures/p6/objects6.pdf`; nothing changed or removed.

## §1.4 placement diff (.quad/scratch/p6u6/integrate/placements.mjs on build/, objects6.pdf p0)
- Fixture seat's pre-U6 reply: Im1/Fm1/Im5 candidate with bare `{box}`; Im2/Im3/Im4 unsupported-transform, bounds null, ops [replace].
- Built engine now: ALL MATCH §1.4 sentence 1 (deep-equal placement and objectOps): Im1 identity [72,600,172,650]; Im2 {swap:true,sx:1,sy:-1} [400,650,450,750]; Im3 {swap:false,sx:-1,sy:1} [200,600,300,650]; Im4 box [72,400,132,440] full [72,400,172,450]; Fm1 [72,100,372,300] ops without replace; Im5 [300,120,350,170]; every image ops = move,resize,delete,replace,duplicate,transform,crop; "Over" a text candidate.

## GUI (§4.3, §4.5) — each PNG viewed with Read
- shot-direct7.mjs: PASS all. shot-direct6.mjs: PASS all. review-u5-renderer-probe.mjs: ALL CHECKS OK.
- u6-a-group-box.png: Im1 + Im3 inside one purple group frame with 8 handles; members carry no handles of their own.
- u6-b-guides.png: mid-drag, Im1's outline at Im2's left edge; vertical guide x 400 and horizontal y 142 span the page; the picture itself stays at its origin until commit (as in U5).
- u6-c-crop-mode.png: Im1 with filled crop handles; Inspector row shows rotate glyphs, Flip H, Flip V and a pressed Crop.
- u6-c-cropped.png: Im1 visible 60×50 with the cropped-away part as a pale dashed box; Inspector "Image 60×50 pt"; "Edited — not saved".
- u5-a-multi-select.png: direct3 group frame around the paragraph, Im1 and the blue image; u5-b-menu-fm1.png: Fm1 menu with 8 items, Replace dimmed; u5-c-alt-duplicate.png: ⌥-duplicate copy offset from the original. a and b share the Im1 row height (scroll offset fix holds).
- `pgrep -fl -- '--drive='` empty after every run.

## Applied fixes
- None (nothing red came from tests/drive/checks files; no engine/renderer defect found).

## Open defects
- None blocking. Observations for review: tests/p2-pages.test.mjs:420 50 ms wall-clock budget flakes under full-suite load (82 ms once); Inspector rotate glyphs ⟲/⟳ render small (u6-c-crop-mode.png); u5-c Inspector shows Y 659.988 (float not rounded in the field); drag preview moves only the outline.
