# debt1 — ENGINE evidence (2026-09-24)

Rows 4, 6, 7, 11, 12, 23 (src part), 25: all shipped.

Source edits (targeted, in place):
- src/renderer/engine.ts: structObjectOrder array recursion passes `depth` (row 4, ~2630); checkPage/checkPages throw via failReason("invalid", same text) (row 6, 1530/2765); pageRotation uses getInheritable("Rotate") (row 12, 1331); text-add rotate check uses getInheritable (row 12, ~4568); FieldItem.signed = own /V ?? /Parent /V (row 11, ~1902); checkSignatureField refuses already-signed when /Parent has /V, before the hierarchy refusal (row 11, ~1930); citations 168/180 (row 23).
- src/renderer/sign-prepare.ts:85 getInheritable("Rotate") (row 12).
- src/renderer/content-edit.ts:236 citation comment only (row 23).
- src/safe-write.ts: writeJournal writes `${path}.tmp` (0600, fsync), unlinks it on a failed write, renames over path; new optional WriteInject.journalWrite; journalAdd threads inject (row 25). Copy path untouched; every export/signature kept.

Tests appended (each named "debt1 row N"):
- tests/p6-tabs.test.mjs row 4 (40 dict levels -> 80 with arrays, s2 structIndex 0)
- tests/p2-forms.test.mjs row 6 (fields page -1/99/0.5/'0' -> reason invalid, message regex)
- tests/p4-redact.test.mjs row 7 (OP_PERMISSION['redact-set'] === 'annotate')
- tests/p5-sign-field.test.mjs row 11 (in-memory /Parent with /V -> signed true, sign-prepare already-signed), row 12 (hello.pdf with /Rotate 90 on /Pages -> rotations [90], text-add unsupported-transform, sign /AP /Matrix[0 1 -1 0 0 0])
- tests/p1-save.test.mjs row 25 (no pending-saves.json.tmp after a save; journalWrite inject that writes 10 bytes then throws ENOSPC -> disk-full, previous journal bytes intact)

Bite: reverted every fix in the built JS (engine.js, engine-core.js, sign-prepare.js, safe-write.js) under one lock hold -> the 6 debt1 tests: 0 pass / 6 fail; restored -> 6/6 pass.

Run (one lock hold): npm run build && node --test p6-tabs p5-sign-field p1-save p3-journal p2-forms p4-redact p3-targets p3-drive p6-links p5-sign p5-verify -> 167 tests, 167 pass, 0 fail, 0 skipped.
Fixture hashes: shasum -c .quad/shared/p6/checks/fixtures.sha256 all OK. tsc --noEmit clean.

Not done: nothing. Notes: render/thumbnail "page must be an integer" errors (engine.ts ~329/386 built) are not checkPage and still carry no reason (out of row 6 scope); checkAt (insert at) unchanged.
