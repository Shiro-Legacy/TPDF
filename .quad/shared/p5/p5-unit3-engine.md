# P5 U3 engine — evidence (engine seat, Opus 5.5, 2026-09-24)

Files: `src/renderer/engine.ts`, `src/renderer/sign-prepare.ts`, `tests/p5-sign-field.test.mjs` (new, 10 tests), `tests/p5-sign.test.mjs` (+1 test), `tests/p2-forms.test.mjs:308` (9.2 pattern), `tests/p3-forms.test.mjs:414-419` (9.1 delete + undo). No change: `engine-core.ts` (SaveReason lives in engine.ts:188, `no-such-field` added there), `limits.ts`, `worker.ts`.

Shipped:
- §1.1/9.10 `FieldItem.signed` (signature kind only) = own `/V` present.
- §1.2/9.4/9.5/9.9 `field-add kind:'signature'`: allowlist message "text, checkbox, combo, radio, signature"; name optional -> first `Signature<n>` free in `fieldTreeNames`; rect required; floor `field-add: a signature field is at least 40x20 pt`; default/options/kids refusals; `FT /Sig`, no `/AP`, no `/V`, no `refreshAppearance`; reply gains `name` (all kinds).
- §1.3/9.1/9.11 `checkFieldEditable` refuses only a signed field (`a signed field cannot be changed`); rename refused `field-props: a signature field keeps its name` (detail `signature`); rect floor on props; props rect -> `setRect` + delete `/AP`; `field-set` -> failField("signature", "field-set: click the signature field to sign it").
- §1.4/9.3/9.6/9.8 `sign-prepare {field}`: arg check (`rect` must be null) in the invalid-args group; certified-visible refusal skipped for fields; order page -> `no-such-field` -> hierarchy (`/Parent` or no own `/FT /Sig`+`/T`) -> `already-signed` -> read-only -> too small; lookup by `widget.getName()` in both places. `prepareSignature` field branch reuses the U1 appearance + `/Sig` dict (extracted helpers, same object creation order), replaces `/AP`, puts `/V`, SigFlags 3, `/Perms` when certifying. Reply gains `field`; `rect` is the field's page-space rect.

Proofs (tests/p5-sign-field.test.mjs): fixture sha pin + provenance; fields ids 7/8/10 + rects; Signature1 adopt (obj 7/8/10 kept, only 7 has /V + fresh AP, 8 keeps grey AP) -> sign -> reopen signatures 1, signed:true, save read-only, sign-list empty:false; signed source refusals (already-signed, no-such-field x2, invalid), Signature2 incremental revisions 2, prefix identical, `8 0 obj` in increment; rotated Signature3 `/Matrix[0 1 -1 0 0 0]`, all blue pixels inside [500,72,560,300]x2; certified P=2 fill ok while new visible stays `certified`; field-add on hello (Signature1, no AP, obj kept across save, signs); floor/duplicate/Signature4; props/delete + undo with save->reopen->re-list; §9.6 read-only; §1.3 signed scratch doc (`signature`, dirty false); pyHanko test (skippable) over 5 outputs: all INTACT trusted docmdp-ok, certified-filled `DocMDP 2`.

Test run (lock held, after `npm run build`): `node --test tests/p5-sign-field.test.mjs tests/p5-sign.test.mjs tests/p2-forms.test.mjs tests/p3-forms.test.mjs tests/p2-shell.test.mjs tests/fixtures.test.mjs tests/p3-drive.test.mjs` -> 90 tests, 89 pass, 1 fail.
The failure: `tests/p3-drive.test.mjs` "drive P6: click-to-fill …" — `tests/drive/direct-edit.jsonl:32` now expects the §9.12 sign dialog (`sign.dialog`, status "Sign with certificate"), which is renderer work in flight (renderer.ts was being edited during the run). Not an engine reply; expected green once the renderer lands.

Hashes: `shasum -c .quad/shared/p6/checks/fixtures.sha256` 7/7 OK; fixtures/**/*.pdf|png vs u3-hash-before.txt: only `fixtures/p5/empty-sigfield.pdf` added.
Not done: nothing in the engine scope.

## Fixes after review

- E-R1 (blocking), reproduced: review-engine/probe2.mjs B (Signature1 on /Annots, dropped from /Fields) gave sign-prepare ok, a reopen with 0 signatures that still saved, and pyHanko `expected 1 signatures, got 0`. Fix, `src/renderer/engine.ts` checkSignatureField: after the hierarchy check, refuse `invalid` "sign-prepare: <name> is not in the document's /AcroForm /Fields" unless the widget's object number (asIndirect) is an entry of existingFields(pdf). The same probe now returns that refusal.
- E-S1 (should), reproduced (neither message was in tests/): new test "P5 U3 sign-prepare {field} refuses what it cannot sign in place (§9.3, §1.4)" in tests/p5-sign-field.test.mjs builds three scratch PDFs (orphan widget not in /Fields, a /Sig parent with a /T kid, a 30x10 widget). For each one it asserts reason `invalid`, the exact message and an unchanged, clean journalState. It failed on the pre-fix build at the orphan case.
- Run (lock held): tsc clean; npm run build; node --test p5-sign-field, p5-sign, p2-forms, p3-forms, p2-shell, p3-drive -> 86/86 pass.
- Hashes: p6 fixtures.sha256 all OK. Against u3-hash-before.txt the only new file is fixtures/p5/empty-sigfield.pdf.
