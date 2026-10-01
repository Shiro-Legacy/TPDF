# P6 U5 renderer seat evidence (Opus 5.5, 2026-09-24)
Files changed: src/renderer/renderer.ts, src/renderer/index.html, src/renderer/styles.css, tests/p2-shell.test.mjs (two additive edits), tests/p6-direct.test.mjs (additive asserts). preload.cts / main.ts: no change.

## What landed (contract §2 as amended by §9.13–§9.27)
- Selection: `directSelection: ObjectFocus[]` ({kind,name,targetId,rect}, click order, anchor = last = directFocusedObject). Shift-click toggles (`toggleObject`); removal refocuses the new anchor or #page (§9.23); a plain press on a non-member of a group makes it single first. `restoreDirectFocus` resolves the anchor (with the rename pass) and then every member, each painted target used once; `markSelection()` paints `.selected` + `.multi` (§9.19). Cleared by clearDirectFocus (page/mode/open/undo/redo), Escape, empty-space click, compound delete.
- `RectGesture` gains optional `place`, `duplicate`, `done`; unset = today's `placeBox(button, …)` behaviour (U2–U4 drives green). Group drag on the union rect (`startGroupGesture`), group arrows (⌥⌘ ignored), group Delete → one `edit-objects`; Enter/dblclick single only. `content.edited {kind:'objects', action, name:'Im1,Im2', count}`; single edits unchanged (no count). Announcements "Moved/Deleted/Duplicated N objects", "Duplicated image".
- ⌥-drag duplicate: single `edit-object {action:'duplicate', box}` / group `edit-objects` duplicate items; unclassed `div.ghost` outlines removed in `done`; `direct.gesture.kind:'duplicate'` mid-drag; copies selected via `selectCopies` (nearest same kind+name among targets absent from the pre-reply scan, §9.18).
- Context menu `#object-menu` (role=menu, tabindex -1, three menuitems); contextmenu / Shift+F10 / ContextMenu key; items per §2.4; keyboard ↑↓ Home End Enter Space Escape Tab with preventDefault+stopPropagation (§9.21); capture-phase document pointerdown outside-close, dismissing press blocks the add-text click (§9.22); stopPropagation + pageRegion early returns (§9.15); closes on scroll, other page/zoom paint, lost anchor; same-page repaint re-derives items (§9.16). Menu duplicate = `menuDuplicateRect` (+10, −10 where no room, §9.25). Ctrl-press ignored by the body gesture (§9.27).
- STATE: `direct.selection`, `direct.menu`, `content.edited.count`. Drive: `click shift` on all three events, `drag alt` on pointerdown + fire moves (empty-space branch unchanged), new `menu {page,x,y,item?}` op, driveDirectKey open-menu branch right after the sign/verify Escape line (onTarget line untouched).
- Help: gesture cell, ceilings cell, `<li>` edits and one new `<li>` per §2.6/§9.27; helpRows stays 11. `CONTENT_DETAIL_TEXT["invalid-selection"]` only.
- Pure helpers exported and pinned in tests/p6-direct.test.mjs: `unionRect`, `menuDuplicateRect` (union dy −80 → −52 clamp; +10/−10 offsets).

## Runs (all under .quad/run.lock)
- `npx tsc --noEmit` clean; `npm run build` ok.
- `node --test tests/p6-direct.test.mjs tests/p2-shell.test.mjs tests/p3-drive.test.mjs tests/p6-drive3.test.mjs tests/p6-drive6.test.mjs` → 32 pass, 0 fail.
- `node scripts/drive.mjs` exit 0: direct-edit6 (54 steps; geometry matches §4.2/§9.29/§9.30 incl. copies selected), direct-edit, direct-edit2, direct-edit3, debt1, object-edit, direct-edit4, direct-edit5, content-edit, help-fields.
- `shasum -a 256 -c .quad/shared/p6/checks/fixtures.sha256` 8 OK; `pgrep -fl -- --drive=` empty.

## Not done / notes
- No real-CDP GUI run by this seat (shot-direct6.mjs belongs to the tests seat).
- Undo/redo clear the selection via clearDirectFocus (existing behaviour); direct-edit6 re-clicks after every undo, so §9.28's "re-resolve after undo" is moot.
- Full `npm test` not run by this seat.

## Fixes after review
- R1/S1 (§9.28, K1): undoEdit/redoEdit now clear only the mark/link focus and close the menu; directFocusedObject and
  directSelection survive, so the reply's repaint (restoreDirectFocus) re-resolves every member by nearest kind + name and
  drops any that no longer resolve. Pre-fix repro: review probe K1 gave []; direct-edit6 step at line 59 (undo after the
  group menu duplicate, expects [Im1,Im2]) failed. Now K1 ok and direct-edit6 exits 0. No drive lines added.
- R2 (§0, K2): new syncObjectInspector(targetId) (row + form), used by focusObject, both toggleObject branches and the end
  of selectCopies. Probe K2 now reads Im2 (x 400) after Shift-click; K2b still Im1 after removal.
- R3 (§2.2/§9.22): startRectGesture's Escape cancel sets addBlocked = true, so the trailing click after an Escaped drag
  adds no text. Probe b3x (group) and e1 (single) now ok (active null, 0 editors).
- Regression test: tests/p6-direct.test.mjs 'U5 review…' pins the three wirings in source (fails on the pre-fix file,
  checked against a reverted copy). The drive cannot release after Escape or read the Inspector form, so the real-mouse
  checks remain review-u5-renderer-probe.mjs K1, K2, b3x, e1.
- Ran under the lock: tsc, build, node --test p6-direct/p2-shell/p3-drive (31/0), p6-drive6 (1/0), drives direct-edit6,
  direct-edit3, debt1, object-edit and review-u5-renderer-menu.jsonl all exit 0. Fixtures 8 OK, u5 hash list identical.
- Still FAIL in the probe, as before this fix and outside R1-R3: d0/d0x, d6, d8/d8x, d9/d9x, d10 (sequencing in the
  'sel'/'fix' sessions; the reviewer's d0y, d6x, d8y, d10x reruns pass).
