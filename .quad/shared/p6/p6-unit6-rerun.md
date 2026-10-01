# P6 U6 rerun (after review fixes R1-R4 and TS1), 2026-09-24

Rerun agent: Opus 5.5. No src/ edits. Scratch: .quad/scratch/p6u6/review-rerun/

## Results
- npx tsc --noEmit -p tsconfig.json: clean (exit 0)
- npm run build: exit 0
- npm test: 493 tests, 492 pass, 0 fail, 1 skipped (exit 0)
- Drives (all exit 0, ok:true, 0 red):
  - direct-edit7 x3: 66/66 steps each
  - direct-edit 75, direct-edit2 50, direct-edit3 79, direct-edit4 71,
    direct-edit5 35, direct-edit6 59, debt1 22, object-edit 11,
    content-edit 4, help-fields 21
- shasum -c checks/fixtures.sha256: 9 OK (after drives and again after shots)
- Fixture hash list against checks/u6-hash-before.txt: exactly one added line,
  `ca0a5988...fb2ab8  fixtures/p6/objects6.pdf` (D3). Nothing else added, nothing changed.
- Shots (run once each, all exit 0): shot-direct7 PASS all, shot-direct6 PASS all,
  review-u5-renderer-probe ALL CHECKS OK
- Electron hygiene: pgrep -fl -- '--drive=' is empty after both lock holds

## PNGs (checks/review-out/), all viewed
- u6-a-group-box.png: group box with handles around Im1+Im3, both members outlined
- u6-b-guides.png: horizontal and vertical smart guides while dragging Im1 to Im2's edges
- u6-c-crop-mode.png: crop mode on Im1, with filled handles; inspector shows Crop active
- u6-c-cropped.png: Im1 cropped to 60x50, the cropped-away part shown as a ghost, "Edited - not saved"
- u5-a-multi-select.png, u5-b-menu-fm1.png (menu shows Replace disabled on the form, other items enabled),
  u5-c-alt-duplicate.png: U5 behaviour holds, no regressions seen

## Open defects
None. R1-R4 (renderer) and TS1 (tests) are verified as green.
