# P6 U5 rerun (after review fixes) — 2026-09-24, Opus 5.5

Scope: rerun only, nothing in src/ changed. Logs: .quad/scratch/p6u5/review-rerun/.

## Results (two lock holds)
- `npx tsc --noEmit -p tsconfig.json` (no lock): exit 0.
- `npm run build`: exit 0.
- `npm test`: 480 tests, 479 pass, 0 fail, 1 skipped (`legacy p12 parses when rc2-40-cbc exists`, the known SKIP). That is 466 + 14 new.
- `node scripts/drive.mjs tests/drive/direct-edit6.jsonl` ×3: exit 0 all three times, 59 steps, all ok. Step 41, the undo that keeps selection [Im1,Im2] per §9.28, is green now that renderer R1 has landed.
- One run each, all exit 0 with no red step: direct-edit, direct-edit2, direct-edit3, direct-edit4, direct-edit5, debt1, object-edit, content-edit, help-fields.
- `shasum -a 256 -c checks/fixtures.sha256`: 8 OK, both after the drives and after the GUI script.
- The fixtures/**/*.pdf|png hash list (54 lines) matches checks/u5-hash-before.txt byte for byte, with an empty diff, checked after both holds (D2).
- `node .quad/shared/p6/checks/shot-direct6.mjs`: exit 0, "PASS all". Shots are in checks/review-out/ and I viewed each one with Read:
  - u5-a-multi-select.png: Im1 and Im2 both have the selected outline. The Inspector shows "Image 60×40 pt X 400 Y 700", which is the anchor Im2, so the R2 fix is visible.
  - u5-b-menu-fm1.png: the object menu is open on Fm1 with Replace… disabled and Duplicate and Delete enabled. The Inspector shows Form Fm1.
  - u5-c-alt-duplicate.png: after the ⌥-drag, the copy sits beside Im2 and is the selected item. The Inspector shows X 460 Y 659.988, which is the copy and not the original. The header shows "Edited — not saved" and Undo is enabled.
- Electron hygiene: `pgrep -fl -- '--drive='` returned nothing. I killed no processes.

## Open defects (not fixed; out of scope for the rerun)
- src/renderer/content-edit.ts:1036 (editPrivatise, reached from content-edit.ts:920 on the edit-text path): when /Contents is an indirect array shared between pages, `contents.put(elementIndex, owned)` writes into the shared array. That is the same leak as E-1 but on the text path. The engine seat skipped it because it does not own those lines, and it gave a suggested patch in its fix report. No test covers it, so the suite cannot catch it.
- Cosmetic: in shot B the page sits about 20 px higher than in shots A and C, a scroll offset after the right-click. It is not asserted anywhere and I did not look into it.
