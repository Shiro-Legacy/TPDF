# P6 U5 — integration (Opus 5.5 integrator, 2026-09-24)

## Requests applied
- engine → renderer: tests/p2-shell.test.mjs four #object-menu ids. The renderer seat had already landed them (:48), and 'edit-objects' is in DEFAULT_EDIT (:173). Nothing left for the integrator to apply.
- No other requests. No fixes were needed in drive, test or check files, and none in engine or renderer.

## Build and suite (one lock hold)
- `npx tsc --noEmit`: clean. `npm run build`: ok.
- `npm test`: 478 tests, 477 pass, 0 fail, 1 skip ("legacy p12 parses when rc2-40-cbc exists"). That is 466 + 12 new (p6-objects5 11, p6-drive6 1).

## Drives (same hold, exit codes)
- direct-edit6.jsonl ×3: 0 / 0 / 0 (54 steps, 0 red each).
- direct-edit 0, direct-edit2 0, direct-edit3 0, direct-edit4 0, direct-edit5 0, debt1 0, object-edit 0, content-edit 0, help-fields 0.

## Hashes
- `shasum -c checks/fixtures.sha256`: 8 OK before the suite, 8 OK after the drives, 8 OK after the GUI script.
- fixtures *.pdf/*.png listing vs checks/u5-hash-before.txt: 54 lines and an empty diff (after the drives and after the GUI script).

## GUI (second lock hold): `node .quad/shared/p6/checks/shot-direct6.mjs` gave PASS all, Electron exit 0
Shots written to .quad/shared/p6/checks/review-out/, all viewed with Read:
- u5-a-multi-select.png: Im1 (grey) and Im2 (blue) both have the solid selected outline and no handles (18 .multi handles, 0 shown). Fm1 and the text runs keep dashed outlines. The Inspector shows "Image 100×50 pt, X 72 Y 600", which is Im1, not the anchor Im2 (nit, see below).
- u5-b-menu-fm1.png: a real right-click on Fm1 opens a dark menu at the pointer. Replace… is dimmed and Duplicate and Delete are enabled. Fm1 is selected with handles, and the Inspector shows "Form Fm1 100×30 pt". DOM disabled + aria-disabled="true", and STATE items.replace false.
- u5-c-alt-duplicate.png: a real ⌥-drag of Im2 leaves a copy at [460,92.01,520,132.01], selected with handles, and the original stays at [400,52,460,92]. The badge reads "Edited — not saved". gesture.kind 'duplicate' was read mid-drag.
- `pgrep -fl -- '--drive='`: empty after every hold.

## Open defects (not fixed here: too large for the integrator, or not settled by the contract)
- src/renderer/renderer.ts:6284 (undoEdit, and the redo twin at :6304): `clearDirectFocus()` empties `directSelection` on undo/redo. Contract §9.28/§2.1 say "after undo, members re-resolve to the nearest same kind + name", and design-agent-drive.md's trap line says the same. No expect fails today, because direct-edit6 sends Escape or re-clicks after every undo, so the re-resolve rule is unproven. Fix it in the review round: keep the selection across undo/redo so restoreDirectFocus re-resolves it, then add a drive step after an undo that pins `selection` length 2. Alternatively, amend §9.28 to "undo clears the selection".
- Nit (not in the contract): src/renderer/renderer.ts:2938-2951 `toggleObject` moves the anchor but never calls selectContentRow/openContentObjectForm the way focusObject (:2848-2849) does, so the Inspector keeps showing the first member (shot A). No test pins this. It belongs to the UX lens.
