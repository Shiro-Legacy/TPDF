# P6 U7 review: renderer + human UX lens (real CDP mouse/keys)

Verdict: **REVISE** (2 blocking, 1 should, 3 nits)

## Findings
**F1 blocking, renderer (+tests)** `src/renderer/renderer.ts:2879/2890` resolveMembers. Clauses §2.3, §9.31, §9.32, §10.39.
- Repro (real mouse, probe3-direct3.out): mixed selection [run "Move me", Im1], then drag the run body by 30pt. Both objects move (rev 2, edited {selection, move, count 2}), but the selection becomes [Im1] only. A drag started from Im1 does the same.
- The next Delete deletes Im1 ("Deleted image", objects 2) instead of refusing per §9.40.
- The drive probe `checks/review-u7-renderer-mixed.jsonl` reproduces it: ArrowRight (1pt) keeps 2 members, Shift+ArrowRight (10pt) drops the run.
- Cause: applyEdit runs refreshFields/refreshLinks before refreshContentTargets. Their restoreDirectFocus sees the stale scan, so `textMember(found)` resets the member's rect to the pre-move bounds. The final ±1.01 match then fails.
- Fix: treat content as not ready while `scan.revision !== revision` and keep the member untouched. Add a Shift+Arrow or drag step to direct-edit8 that pins selection length 2 (tests).

**F2 blocking, renderer** `renderer.ts:8677/8681` (armed branches of pageRegion pointerdown) and `:8625` contextmenu. Clauses §10.38, §9.26.
- Redact with Mark area armed: one real right-click at (300,450) creates mark id 6 [306,396,466,496] (rev 1, dirty) and opens the page menu on top of it (m1 shot).
- Annotate with Draw link armed: a right-click disarms the tool and opens no menu (m2 shot).
- Fix: in both armed branches, `if (event.button !== 0 || event.ctrlKey) return;` before startAreaDrag. Control-click takes the same path (inferred).

**F3 should, renderer** `renderer.ts:3916-3921` clickToAdd. Clauses §2.2, §9.26.
- A real Shift-click on empty space in Edit text with [Im1] selected drops the selection and opens the add-text editor (direct.active kind 'add'). A near-miss while building a multi-selection destroys it.
- Fix: add `event.shiftKey` to the early return.

**Nits** (renderer)
- N1 `index.html:559`: Help says a multi-selection shows "one frame with handles that resize it as a block". A mixed selection has no handles (§9.31).
- N2: the ceiling cell says a multi-selection "moves, resizes, rotates, flips and deletes as a block", which is false for mixed selections.
- N3: the Help object-menu list "(Replace, Duplicate, rotate, flip, Crop, Delete)" omits Cut/Copy/Paste.

## Checked fine (real CDP mouse/keys)
- objects6, 16/16 in probe2-objects6.out:
  - ⌘C gives clipboard {1, image 1} with no revision and "Copied 1 object".
  - ⌘V gives PA1 at +10 (selected, rev 1, edited paste); a second ⌘V cascades PA2. Two undos restore.
  - ⌘X on Im2 cuts it, announces, and empties the selection.
- Page menu:
  - A right-click on empty space opens it with only Paste enabled and focus on Paste. ↑/↓ keep focus, Escape closes and refocuses #page.
  - Enter pastes at the click point.
  - Control-click on empty space opens the page menu (no add editor). Control-click on Im1 opens the object menu.
  - Shift+F10 on #page opens it at the viewport top-left.
- Chord gate: ⌘C in the Inspector W field puts "100" on the OS clipboard with direct.clipboard unchanged. With Help open, ⌘C/⌘V do nothing.
- direct3:
  - Shift-click seeds [text] with no editor, and Shift-click on Im1 builds one group box with 0 handles. ArrowRight is one revision.
  - A paragraph is refused. Escape clears.
  - ⌘C on the mixed selection gives {image 1, text 1}. ⌘X gives "Cut 1 item; 1 text run stay…".
  - Forms-mode page paste announces the skipped kinds. ⌘V on p2 pastes the image and the run, both selected.
- Inline editors: in the run editor and field input, ⌘C/⌘V go to the OS clipboard with no engine call.
- Fields, links and marks copy/paste through a real Shift-click or click plus chords: plain copy at +10, link 15, a new mark.
- Shift-click on a checkbox does not toggle it.
- Form2 grip drag is one revision. shot-direct7 (single-object drag/resize/crop) passes.
- An unarmed Redact right-click opens the page menu with no mark.
- Drives exit 0: direct-edit, direct-edit2 through direct-edit8, direct-edit7-save, debt1, object-edit, content-edit, help-fields.
- Fixtures: 9 OK, and the list is byte-identical to u7-hash-before.txt. No `--drive=` stragglers.
- Help §10.11/§10.40 sentences are present and pinned (p2-shell:48, :205).
- workerFailed clears clipboardInfo (`renderer.ts:852`). This was checked statically only; restart was not run.

## Probes and outputs
- `checks/review-u7-renderer-probe.mjs` (sessions objects6, direct3, fields, links, hello, misc, form2)
- `checks/review-u7-renderer-mixed.jsonl`
- Outputs in `.quad/scratch/p6u7/review-renderer/`: probe*.out, mixed.json, drive-*.json, shot-direct7.out
- The OS clipboard was saved and restored on every run.

## Screenshots (checks/review-out/)

- a1-paste1 PA1 at +10; a2-paste2 PA2 cascade; a3-page-menu only Paste enabled; a4-page-paste paste at the click point
- c1-mixed group box, no handles; c2-after-drag F1: only Im1 has handles after the drag, the run is separate
- d1-p2-paste mixed paste on p2; d1-text-paste run pasted on p2; e1-field-paste "plain copy"
- f1-link-click clicked link; f2-form2-drag field drag; g1-mark-click clicked mark
- m1-redact-armed-rightclick F2: stray mark with the page menu over it; m2-linkdraw-rightclick F2: no menu
(all named review-u7-*.png)
