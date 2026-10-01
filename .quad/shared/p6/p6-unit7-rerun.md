# P6 U7 rerun (after review fixes E1-E3, F1-F3, T-1..T-3)

Date: 2026-09-24. Rerun agent: Opus 5.5. No src/ edits. Logs: .quad/scratch/p6u7/review-rerun/.

## Results
- npx tsc --noEmit -p tsconfig.json: exit 0.
- npm run build: exit 0.
- npm test: 530 tests, 529 pass, 0 fail, 1 skipped (the known skip: "legacy p12 parses when rc2-40-cbc exists"), 0 cancelled.
- Drives (node scripts/drive.mjs), all exit 0 with ok true:
  - direct-edit8.jsonl ×3: 115/115 steps each, 0 red.
  - direct-edit, direct-edit2, direct-edit3, direct-edit4, direct-edit5, direct-edit6, direct-edit7,
    direct-edit7-save, debt1, object-edit, content-edit, help-fields: exit 0 each.
- Fixtures: shasum -c fixtures.sha256 gives 9 OK. The full list (55 lines) is byte-identical to
  checks/u7-hash-before.txt, both after the drives and after the shots (D3 holds).
- GUI evidence: shot-direct8.mjs exit 0, "PASS all", OS clipboard restored.
  I viewed all four PNGs in checks/review-out/:
  - u7-a-pasted.png: objects6. The pasted image is selected, offset +10 from Im1, and the badge reads "Edited".
  - u7-b-page-menu.png: after undo, the page menu opens on empty space with only Paste enabled.
  - u7-c-mixed-group.png: direct3. "Move me" and Im1 sit in one group box, with no handles.
  - u7-e-field-paste.png: field-defs, Fill forms. "plain copy" is pasted and selected, the page shows 6 fields and the status reads "Pasted 1 item".
- Electron hygiene: pgrep -fl -- '--drive=' is empty, and nothing under tmp/review-out is running. I killed nothing.

## Red
None.

## Open observations from the fixers (not failures, not pinned; for the owners)
- Tests seat: Cut of [run "Move me", Im1] on direct3 p0 followed by Meta+v drops both "Move me" runs from
  content.bounds (5→4), and the pasted run is not in the selection. Undo restores both.
  Probe: .quad/scratch/p6u7/review-tests/fix/n-cut.json S26. Look at src/renderer/content-edit.ts or renderer.ts.
- Tests seat: after Cut then Meta+v of radio "color", the fields.defs names and rects of the kids reshuffle
  (color.A/color.B), while count stays 5. This may break §10.21 ("restores the group in place").
- Tests seat: in Fill forms, a right-click on "nom" while the "plain" editor is active opens no menu,
  because of the early return on directActive in the hitLayer contextmenu handler, src/renderer/renderer.ts ~8619.
  This may conflict with §10.38.
- Engine seat: a loadPage handle left undestroyed during edit-batch makes the link rewrite come back with annotId 0.
  Only a test can trigger this. The worker always destroys its handles.
