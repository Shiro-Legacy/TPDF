# Debt sweep 2 review — renderer + human UX, real CDP mouse (Opus 5.5, 2026-09-24)
Verdict: REVISE (1 blocking, 1 should). Probe: `.quad/shared/p6/checks/review-debt2-renderer-probe.mjs` (CDP mouse/keys, Input.insertText, no osascript; udata under checks/review-out/debt2-review-udata-r). Logs: `.quad/scratch/debt2/review-renderer/probe1..5.out` (probe5 = final).

## R-B1 (blocking) §9.8/§9.9 — a real Apply click never submits after a fractional move; the no-op proof is vacuous
index.html:410-415 — `#content-box-x/-y/-w/-h` are `type="number" step="0.1"`. After a real drag of Im1 by (3,7) CSS px at fit width (1.307 px/pt, dpr 2) the placement is [74.295,594.6435,174.295,644.6435]; the Inspector shows `74.3`, `594.64`, `100`, `50` (rounding OK) but `#content-box-y.validity.stepMismatch === true`. A real mouse click on `#content-apply-object` runs Chromium form validation: bubble "Please enter a valid value. The two nearest valid values are 594.6 and 594.7" (review-debt2-renderer-d-apply-unchanged.png), `submit` never fires. So (a) unchanged Apply keeps revision/dirty only because nothing ran — shot-debt2.mjs case B would PASS on the pre-fix build too (pre-fix shows 594.6435, also a step mismatch); (b) a human who edits W to 120 and clicks Apply gets no write (revision 7 → 7, probe1: W field "120", Y stepMismatch). Any 2-decimal value whose hundredths ≠ 0 blocks every Inspector box edit.
The passthrough logic itself is right: dispatching `submit` directly — unchanged → revision/dirty/placement identical (no-op); W=120 → [74.295,594.6435,194.295,644.6435] (other three edges unrounded) (probe5 "D … submit event", "E").
Fix: `step="any"` on the four inputs (index.html:410,411,414,415); shot-debt2 B asserts `document.getElementById('content-object-form').checkValidity()` before its Apply click so the no-op can no longer pass by validation.

## R-S1 (should) §9.9 — shot-debt2.mjs B tests only the unchanged Apply
checks/shot-debt2.mjs:246-258 never edits one field, so the "only that edge changes, the other three keep unrounded values" half of §9.8 has no real-input proof. Fix: after the unchanged Apply, real click `#content-box-w`, select-all, `Input.insertText('120')`, real click Apply → `direct.active.placement` ≈ [p0,p1,p0+120,p3] within 2e-4 (use `direct.active.placement`, not `content.objects[].box`, which STATE rounds to 2 decimals at renderer.ts:4966).

## checkedFine
- Diff vs pre-debt2 tgz: renderer.ts (cutAnnouncement export + :3062 use; round2 display :4670-4673; passthrough :4779-4789), index.html (N2 li replaced with the §9.4 sentence verbatim; clip li with raw U+2026 appended after the crop li; no other change), p2-shell (literal `includes` pins: new N2 + clip; old N2 in `gone`), p6-direct (3 literal assert.equal). Nothing else touched in my files.
- §9.10 real input: CDP Shift-click "Move me" + Im1, CDP ⌘X (commands:cut → DOM cut) → `#edit-status` exactly "Cut 1 item; 1 text run stays (it cannot be deleted)", STATE announcement equal, objects 2, clipboard 2, revision +1, content.edited {selection,delete,Im1,1}; real click #undo → objects 3, dirty false.
- Plural via real UI: right-click empty page → menu Paste (a 2nd run at [299.88,519.6,…]), Shift-add Move me + Im1, ⌘X → "Cut 1 item; 2 text runs stay (they cannot be deleted)", clipboard 3.
- Text-only ⌘X: "Copied 1 text run; text runs cannot be cut", revision and dirty unchanged, clipboard 1, objects 3.
- a11y: all announcements land in `#edit-status` (role=status, aria-live=polite); no new controls/ids in this unit.
- Inspector refreshes after a drag without reselect (fields = round2 of the new placement); regex /^-?\d+(\.\d{1,2})?$/ holds.
- Help sentences truthful vs engine (Tr kept, `0 g 0 G 1 w` → "black with a 1 pt stroke").
- Drives (one lock hold) exit 0, 0 ok:false: direct-edit8-chords, direct-edit8-cut, direct-edit8, direct-edit8-save, direct-edit9, direct-edit3, direct-edit7, direct-edit6, object-edit, content-edit, help-fields. node --test p2-shell + p6-direct: 14/14.
- fixtures.sha256 9 OK; 55-line list identical to debt2-hash-before.txt; no --drive= or review-udata stragglers.

## Screenshots (checks/review-out/)
- review-debt2-renderer-a-cut1.png — after mixed ⌘X: Im1 gone, status singular text.
- review-debt2-renderer-b-textonly.png — text-only ⌘X: nothing removed, "Copied 1 text run…".
- review-debt2-renderer-c-cut2.png — 2 runs + Im1 cut: plural status, both runs remain.
- review-debt2-renderer-d-inspector.png — Inspector after the fractional drag: 74.3 / 594.64 / 100 / 50.
- review-debt2-renderer-d-apply-unchanged.png — real Apply click: validation bubble on Y (R-B1).
- review-debt2-renderer-d-apply-w.png — after the W-edit Apply attempt: still blocked, no write.
