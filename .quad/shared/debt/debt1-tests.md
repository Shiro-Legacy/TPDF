# Debt sweep 1 — TESTS/DRIVE/GUI/DOCS evidence (Opus 5.5)
Rows: 2, 8 (sentence), 10, 16a-d, 21, 22, 23 (tests/p3-edit.test.mjs:5), 24. Scratch: .quad/scratch/debt1-tests/ (pre-fix build from pre-debt1 tgz + logs).
## Files
- new: tests/drive/debt1.jsonl (22 steps), tests/drive/refusals.jsonl (29 steps, salvaged A2, + header comment, no expect changed), tests/drive/nonascii-label.jsonl (4 steps, verbatim), tests/debt1-drive.test.mjs (5 tests)
- new: fixtures/p3/make-field-defs.py + field-defs.pdf (1450 B, 43782ca5…4250, run twice identical) + field-defs.provenance.json
- edited: tests/fixtures.test.mjs (+1 PINNED line), .quad/shared/p6/checks/fixtures.sha256 (+1 line), tests/drive/sign-field.jsonl line 34 expect (+ sign.dialog {Signature1, page 0}; still 45 steps), tests/p5-drive-field.test.mjs (+1 test), tests/p3-edit.test.mjs:5, .quad/shared/p5/checks/shot-sign-u3.mjs (mode() real CDP click on label[for=mode-x]; C/D badges-in-view logged), .quad/shared/p6/checks/shot-debt1.mjs (new), .quad/shared/p3/design-agent-drive.md (appended), README.md (rewritten, 50 lines), fixtures/make-rtl.mjs:11 + rtl.provenance.json generator.sha256 d327c832…12de7
## Results (current build, under lock)
- drive.mjs: debt1 exit 0 (22/0 red), refusals 0 (29/0), nonascii-label 0 (4/0), sign-field 0 (45/0)
- node --test fixtures, p1-worker, debt1-drive, p5-drive-field: 39 tests; 38 pass + 1 fail (16b entry filter), fixed, p5-drive-field rerun 3/3 pass
- shot-debt1.mjs: PASS all 10 checks, PNGs review-out/debt1-{a..f}-*.png (d viewed: add editor grows, 12 pt × 1.307 = 15.69 px)
- shot-sign-u3.mjs: PASS all; C badges in view 1 of 2, D badges in view 1 of 2 (the second signed field is off-screen at fit-width, not missing)
- fixtures.sha256 all OK; debt1-hash-before.txt all OK; no straggler Electron
## Bite evidence
- pre-fix build (pre-debt1 tgz): debt1.jsonl red at step 2 (announcement "" vs "Blocked link (ftp:)"), 6 ("Changed link" vs "Deleted link"), 9 (sig drag rev 0 vs 1), 16 (error kept "Draw a bigger signature field"), 21 (content.edited kept Im2 move)
- current src with only the sigfield onEnd announce("") removed: debt1 red only at step 13 (announcement kept the arm prompt) → row 14
- 16b: one flipped byte in field-signed3.pdf → all three verify "invalid"
## Notes
- Exact app text is "Blocked link (ftp:)" (schemeName keeps the colon); the table's "(ftp)" is shorthand.
- Row 18 button id is #save-copy (menu id save-as); shot checks its text "Save As…".
- make-field-defs.py adds /Parent on the two radio kids (the salvage script omitted it); verify-roundtrip --field-defs ok on all four fields.
- Row 2 (direct.links 0 in Forms, link set by page) is coverage of existing behaviour: green pre-fix by design.
