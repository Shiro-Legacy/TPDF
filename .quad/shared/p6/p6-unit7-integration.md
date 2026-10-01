# P6 U7 integration (2026-09-24, integrator, Opus 5.5)

Requests applied: none (all three reports had `requests: []`). No file edited by the integrator except this one.

## Suite (one lock hold)
- hashes before: fixtures.sha256 9 OK; fixture list diff vs u7-hash-before.txt empty (55).
- `npx tsc --noEmit -p tsconfig.json` exit 0; `npm run build` exit 0.
- `npm test`: tests 523, pass 522, fail 0, skipped 1 (493 + 30 new: p6-objects7 26, p6-drive8 1, p6-drive7 +1,
  p2-shell +1 Help, p6-direct +1 no-chord-keydown). No failing tests.

## mutate diff (engine.ts:1790 vs pre-p6u7-20260924.tgz engine.ts:1732; 22 → 23 lines)
```
1a2
>     if (session.batch) { work(); return journalState(session); }
```
The only change is the §9.2 line; the rest of mutate is byte-identical.

## Drives (same lock hold; exit / steps / red)
- direct-edit8 ×3: exit 0 / 104 / 0 each (wall 3 s, 4 s, 3 s).
- Once each, all exit 0 with 0 red: direct-edit (75), direct-edit2 (50), direct-edit3 (79), direct-edit4 (71),
  direct-edit5 (35), direct-edit6 (59), direct-edit7 (66), direct-edit7-save (19), debt1 (22), object-edit (11),
  content-edit (4), help-fields (21).
- hashes after: 9 OK; list diff empty. `pgrep -fl -- '--drive='` empty.

## GUI (second lock hold): `node .quad/shared/p6/checks/shot-direct8.mjs` (case D included), exit 0, "PASS all"
A ⌘C (CDP commands) → kinds.image 1, defaultPrevented true, revision unchanged; ⌘V → objects 7, PA1 [82,152,182,202].
A' wc.copy()/paste() on Fm1 → form PA1 [82,502,382,702]. B page menu: only paste true (11 items).
D focus content-box-w: copy not prevented, direct.clipboard unchanged, OS clipboard held "100", then restored.
C direct3 Shift-click run + Im1 → kinds [text,image], 1 group box, 0 handles; ArrowRight → selection move count 2, both +1.
E field-defs Fill forms ⌘C/⌘V → fields.count 6, "plain copy" [82,257,202,282].
Screenshots (review-out/, each viewed with Read):
- u7-a-pasted.png: objects6 p1, the PA1 copy with handles offset +10,+10 over the dashed Im1; the badge reads "Edited — not saved".
- u7-b-page-menu.png: the 11-item page menu at the right-click point; only Paste is enabled (Cut/Copy/Replace…Delete dimmed).
- u7-c-mixed-group.png: direct3 p1, one group outline around "Move me" and Im1, no handles; the paragraph is not a member.
- u7-e-field-paste.png: field-defs in Fill forms, "plain copy" selected under plain with handles; the Fields tab lists it;
  the status bar reads "Pasted 1 item".
After the run: 9 OK, list diff empty, no --drive= process.

## Applied fixes
None were needed. There was no red step, so no test, drive or source edit was made.

## Open defects
None found by integration. Carried ceilings from the seats (by design, not defects): a batch edit-text inside a Form
refuses not-candidate; the byte-budget too-large branches of clip-copy/clip-paste are untested; annotation-item
refusals in edit-batch abandon inside the op; the radio kids show on-name "A/A" in the Fields tab
(the pre-existing FieldItem.on display, §10.16); contract-p6-unit6.md §4.1:44 still reads (75,600).
