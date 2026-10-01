# P6 U4 review — renderer + human UX lens (real CDP mouse/keys), Opus 5.5, 2026-09-24
Probe: `.quad/shared/p6/checks/review-u4-renderer-probe.mjs` (sessions links/undo2/reopen/nav/tabs/direct3/form2, env ONLY=… selects).
Ran under .quad/run.lock: `npm run build` exit 0; the probe 4 times (full, then links+nav+tabs, links+nav, undo2) after fixing three probe-side bugs
(off-screen dblclick point, a nav session left in Forms mode by the `fields add` step, and a tabs loop that drifted a page).
Verdict: REVISE. Two `should` findings, both in renderer.ts. Everything else in this lens reproduced with real input.

## Findings
R1 (should, renderer, §9.6; the known deviation, confirmed). renderer.ts:2717, `dropDirect` → `void commitLinkEditor(active)` → `linkSet` (5318)
→ `showError`. Repro b20: Annotate, click L1, Enter, insertText `ftp://nope`, click the Read mode radio → #error and STATE.error =
"link-set: ftp: is not a link this app opens", nav.last {link-set, index 0, detail 'scheme'}. §9.6 says a mode or page change commits silently.
Fix: in the dropDirect link branch, call link-set without the error display (pass a `silent` flag to linkSet that skips showError). nav.last.detail can stay.
R2 (should, renderer, §1.5 "Undo/Redo close the editor without a request"). renderer.ts:7114-7115 (Undo/Redo buttons) with the blur commit
at 2702. Repro b26: Annotate, click L1, Enter, insertText `https://example.org/undo-click`, real click on the toolbar Undo → ops ['link-set'],
links[0].uri = the draft, dirty true, revision 1. The mousedown blurs the editor, and the blur starts finishDirect. undoEdit then sees
`directActive` (still set while the commit runs), closes the editor and returns without undoing. The human pressed Undo and got the edit
committed instead. ⌘Z from the keyboard never blurs, so only the mouse path breaks. Field editors (U1) share this path.
Fix: `mousedown` → `preventDefault()` on #undo/#redo while `directActive` is set, so focus stays in the editor and undoEdit closes it with no request.

## Checked fine (real CDP unless noted)
- a0-a4 Read mode on links.pdf: 3 targets, annotIds 10/11/13. Hovering L1 shows the pointer cursor and no handles. A click on L2 goes to page 2
  and sets nav.last {link-follow, page, 2}. A click on L1 sets nav.last uri, main logs `P2_LINK {"uri":"https://example.org/one","allowed":true}`
  with no browser opened, and the page stays 0. A click on L3 shows the visible #nav-status text "This link's scheme (ftp:) is not one this app opens".
- e0/e0b nav.pdf in Read mode with field `edge` over the GoTo link: a click in the overlap opens the field editor (no link-follow), and a click on the
  link part below the field follows it. The "fully under a field" ceiling is stated in the Help ceilings list.
- b1 Annotate hover: grip + 8 handles visible and hit-testable. b2: a body drag of 30/40 pt gives ONE link-set, rect [102,112.01,230,132.01],
  dirty, canUndo, revision +1, nav.last {link-set, index 0}, and direct.active in the §1.8 shape. b3: undo restores. b4: the se drag gives [72,72,220,102].
  b5: Esc mid-drag sends no request and the box snaps back. b6: a drag off the page is clamped to [0,0,128,20] by gestureRect
  (§1.2: the renderer clamps and the engine refuses).
  b7: a forced engine refusal (annotId patched to 9999 in the worker message) snaps the box back (style left/top and rect unchanged).
  #error, STATE.error and nav.last.detail all read `no-such-annot` / "link-set: page 0 has no link 9999", and the document stays clean.
- b8-b13: a click focuses the link and marks its Links row aria-current. Plain → moves 1 pt (one op, no viewport scroll, focus kept), Shift+↓ 10 pt,
  ⌥→ 1 pt, and ⌘→ is not consumed. Three undos restore.
  b14/b15: a real Delete sends one link-delete, focus drops and "Deleted link" is announced; undo restores L1 with its URI and annotId 10.
- b16-b25: Enter opens `input.inline-editor.link-editor` with the whole URI selected. insertText then Enter → links[0].uri changed and the Links row
  updated. ftp:// keeps the editor open with its draft, shows the exact error, sets detail 'scheme' and leaves revision unchanged. Escape closes
  with no request. On L2 the draft is '3' and '1' retargets it to page 0. Page 9 → "Enter a page number between 1 and 3" with no request.
  A blur (click on the page) commits once. Undo with an unchanged editor closes it with no link-set. A double-click opens the editor.
- c1-c5 Draw link: aria-pressed true, and a real drag gives the dialog with select#link-kind, nav.linkDraft {page 0, ≈[300,400,420,420]},
  nav.last link-draw and a disarmed button. Web + Save gives one link-add, painted, with annotId 15 and the draft cleared. A 2 px drag disarms with
  no draft, dialog or error. A 3×2 pt drag gives "Draw a bigger link". Escape on the dialog clears the draft.
- d1/d2 rotated page 1: a real drag right+down moves L4 [700,72,720,200] → [740,102.01,760,230.01] (not mirrored). "Save a copy" (--save-to under
  .quad/scratch) → reopen in a fresh Electron: the page 0 links (URIs, retargets, drawn link) and page 1's moved rect equal the pre-save STATE.
- e0a-e5 tabs.pdf in Forms: fields.tabs/tabsOrder/order per page are C/C/a1..b3, A/A/t3 t1 t2, S/S/s2 s3 s1, S/fallback/u1 u2. Real Tab walks each
  page in that order, and Shift+Tab goes back. From a1 the field-bar "Next field" gives a2, a3, b1. Nothing was dirtied.
- f1-f4 direct3 Edit mode: outlines are dashed rgb(101,88,216) with no hover (shot f1). The checkbox toggles them off and on, and the value
  persists in localStorage ('false'/'true') and in direct.outlines. Plain → on Im2 gives one edit-object, [401,700,461,740], with no scroll.
- g1-g5 Redact: a real drag on empty space makes one redact-add at ≈[300,500,400,530] with no selection and no error. A 2 px drag and a click
  make no mark and no error. "Mark area" still draws one mark and disarms. An armed click sends no request and disarms.
- h1 Help: all the U4 phrases are present and helpRows is 11. (The probe's "not yet editable" assert was over-strict: that text is in other rows.)
- i1 form2 field grip drag → one field-props, 9 handles.
- Regression drives (exit, steps, red): direct-edit 0/75/0, direct-edit2 0/50/0, direct-edit3 0/79/0, links-bookmarks 0/15/0, field-design 0/23/0,
  help-fields 0/21/0, direct-edit4 0/65/0, direct-edit5 0/27/0. p6-direct + p2-shell pass. `npx tsc --noEmit` exit 0.
- Hashes: fixtures.sha256 6/6 OK and u4-hash-before 50/50 OK. 52 files (the new ones are links.pdf and tabs.pdf). No --drive= or udata stragglers.

## Not a U4 finding (seen in passing)
- When #error appears it pushes the page down about 30 CSS px (b7 client y 221 → 251), so the page shifts under the pointer. This predates U4.
- A click on another field while a field editor is open is swallowed: the blur's closeDirect repaints the layer between mousedown and click.
  This predates U4 (U1 path).

Screenshots (checks/review-out/): review-u4-a1-read-hover, a4-blocked, b1-link-handles, b2-link-moved, b7-refusal, b16-link-editor,
b18-scheme-refusal, b20-dropdirect-error, b26-undo-button-commits, c1-draw-dialog, c2-drawn, d1-rotated-move, e0-field-over-link, e0-tab,
f1-outlines, f2-outlines-off, g1-redact-drag, i1-field-drag (.png).
