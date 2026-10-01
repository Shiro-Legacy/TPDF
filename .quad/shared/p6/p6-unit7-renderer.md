# P6 U7 renderer evidence (2026-09-24)

Files: src/renderer/renderer.ts, src/renderer/index.html, tests/p6-direct.test.mjs, tests/p2-shell.test.mjs.
Not touched: styles.css, preload.cts, main.ts (Edit-menu roles :1067-1069 unchanged), engine files, tests/drive/*.

- Members `{kind,name,targetId,rect,text?}` for image|form|text|field|link|mark (renderer.ts:2765-3020): selectionMembers,
  memberOfNode (own-mode layer per §10.29), resolveMembers (§10.39), shiftSelect (editor commits first and seeds),
  startSelectionGesture / nudgeSelection / commitSelectionMove / deleteSelection -> one edit-batch (>= 2 items) or the single op.
- Clipboard: clip-copy / clip-paste as plain objects; CLIPBOARD_DETAIL_TEXT + showClipRefusal; exported pure
  clipboardChordOk (:3024) and pasteAnnouncement; document copy/cut/paste listeners (no ⌘C/⌘X/⌘V keydown handler).
- Menu: 11 items (cut/copy/paste first); page menu (right-click / Shift+F10 on #page) with the view point -> `at`.
  Keyboard focus opens on the first enabled U5/U6 item (Replace...Delete), so direct-edit6 line 78 stays unchanged.
- Capture listeners on #hit-layer: mixed selections (>= 2) drag, arrow-nudge, Delete, Escape as one; contextmenu and
  Shift+F10 on a run/field/link/mark. Ctrl-click never drags, adds text or starts a redact area.
- Drive: menu items cut/copy/paste; `menu` on empty space opens the page menu at the exact page point (driveMenuPoint:
  MouseEvent client px truncate, -0.54 pt seen); Meta+c/x/v -> ClipboardEvent on activeElement; onTarget widened
  for >= 2 members (still contains `#hit-layer .sig-target`).
- STATE: direct.selection in every mode, direct.clipboard {count,kinds}|null, content.edited.skipped.
- Help (index.html): Direct-editing row gains the Cut/Copy/Paste + Shift-click sentence; ceilings list updated
  ("no multi-select of text runs" removed; 9 new items).

Re-pins (additive only):
- tests/p2-shell.test.mjs:48 ids list gains 'object-menu-cut','object-menu-copy','object-menu-paste'; new test :205 (Help).
- tests/p6-direct.test.mjs:123-150 chord predicate + paste announcement asserts in P6_HELPERS; new test :186 (no chord keydown).

Tests (under the lock; build 0; tsc --noEmit clean):
- node --test p6-direct p2-shell p3-drive p6-drive3 p6-drive6 p6-drive7 -> 38/38 pass.
- node --test tests/p6-drive8.test.mjs -> 1/1 pass.
- node scripts/drive.mjs exit 0: direct-edit, -2, -3, -4, -5, -6, -7, debt1, object-edit, content-edit, help-fields, direct-edit8.
- shasum -c fixtures.sha256 -> 9 OK; nothing added under fixtures/; no --drive= stragglers left.

Not done: shot-direct8.mjs GUI evidence (integrator); full `npm test` not run by this seat.
Risks: a plain click on a link in Annotate now sets direct.selection [link]; the page menu's `at` for a real mouse
is whole-pixel (±0.5 px); Help sentences are pinned verbatim by tests/p2-shell.test.mjs:205.

## Fixes after review
- F1 (reproduced: review-u7-renderer-mixed.jsonl red at Shift+ArrowRight, selection [Im1], Delete deleted Im1). `resolveMembers` treats the content scan as not ready while `scan.revision !== revision`, so a member keeps its moved rect until the post-write scan lands. Pinned by direct-edit8.jsonl steps 110-120 (direct3: Shift+ArrowRight and a 30 pt drag keep [run, Im1], then Delete gives the §9.40 refusal).
- F3 (reproduced: a Shift-click on empty space gave selection [] and active add). `clickToAdd` returns early on `shiftKey`. Pinned by direct-edit8 step 117 (selection kept, active stays object Im1).
- F2 (reproduced for Draw link: a right-click disarmed the tool. The Mark area half of the probe was an artifact: `redact-area.click()` has detail 0, which is the keyboard "mark the shape box" path, and that is what made mark 6. With a detail-1 arm, pre-fix code still disarmed on right-click). The pageRegion pointerdown returns on `button !== 0 || ctrlKey` before every armed branch (Mark area, Draw link, signature field, ink). Pinned by p6-direct.test.mjs "U7 review: a secondary or Control-click…" (source guard; the drive's `menu` sends no secondary pointerdown). A corrected probe copy (.quad/scratch/p6u7/review-renderer-ux/probe.mjs misc) gives 3/3: no mark or link, and the tool stays armed.
- Not changed: with a tool armed the page menu does not open, because `layerLive()` is false. §10.38 only forbids arming or starting an area. Opening the menu over an armed tool would be a design change.
- Tests owner: tests/p6-drive8.test.mjs:90-91,96-97 pins must become 115 / 26 (the script header now says 115 steps, 26 click lines). Steps were appended after step 109, so no S(n) index moved.
- Ran under lock: tsc, build, node --test p6-direct/p2-shell/p3-drive (35 pass), and drives direct-edit8 (115/115), 7, 6, 3, debt1 and object-edit, all 0. Fixtures: 9 OK, 55-line list identical.
