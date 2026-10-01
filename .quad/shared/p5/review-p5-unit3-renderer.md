# P5 U3 review: renderer + human UX, real mouse (Opus 5.5 reviewer, 2026-09-24)
Verdict: ACCEPT. Nothing is blocking. There is 1 should and 3 nits, and none of them touches signing correctness, STATE or secrets.
Probe: `.quad/shared/p5/checks/review-u3-renderer-probe.mjs` (sessions A/B/C/D). It sends real CDP mouse and key events and uses Input.insertText for the password. The password is read from tests/drive/sign.jsonl when the probe runs and is never written out. Screenshots are at `checks/review-out/review-u3-*.png`, and the scratch logs are in `.quad/scratch/p5u3/review-renderer/`.

## Runs (all under the lock)
- `npm run build` exit 0. `node --test tests/p2-shell tests/p3-drive tests/p6-direct tests/p5-drive-field` gave 29/29 pass.
- Drive exit codes: sign 0, verify 0, direct-edit 0 (75 steps), direct-edit5 0, field-design 0, help-fields 0, sign-field 0 (32 steps).
- Redaction: sign-field has 6 sign steps and all are `[redacted]`, sign has 3 and verify has 1. No report contains `"password"` or a quoted `"pdf4all"` value.
- pyHanko on the probe's real-click output: `Signature4 INTACT ENTIRE_FILE trusted docmdp-ok`.
- Fixture hashes: the p6 list is 7/7 OK. Against the baseline, the only new file is `fixtures/p5/empty-sigfield.pdf`. No `--drive=` stragglers remain, the user trust store is absent and no udata has a trusted-certs.pem.

## Checked fine (probe ids)
- a0/a1: in Read mode both empty fields have the caption "Sign", a pointer cursor and title = aria-label ("Signature field Signature1, empty"). Neither title says "read-only".
- a2/a3: a real click on Signature1 opens #cert-sign-dialog with the line "Signing field Signature1 on page 1". The page row and the Appearance fieldset are hidden, and sign.dialog is {Signature1, 0}. A real Escape closes it (sign.dialog null).
- a4 (§9.15): after a field dialog, the menu IPC `cert-sign-open` opens the dialog without the field line and with the page/appearance controls visible. sign.dialog is null.
- b1–b6: hovering shows the grip and 8 handles. A grip drag moves the field by (+40,+30) pt with revision +1, dirty and canUndo set, and no dialog opens (§9.16). A se-corner drag resizes by (+20,+10) with revision +1. After the gesture the target keeps focus: plain ↓ nudges 1 pt and ⌥→ nudges 1 pt (§9.17). Enter opens the field dialog and Escape closes it.
- c0–c3 (§9.13/§9.21): the button arms. A real drag of 220x60 pt creates Signature4 at [72,300.09,292,360.1] with fields.last {add, Signature4} and the status "Added signature field Signature4", then disarms. A 2 px drag adds nothing and disarms. A 30x10 pt drag shows "Draw a bigger signature field" and the revision does not change. c5: a good draw after a refused one clears #error.
- c4/e3: the draw button cannot be reached in Read mode, and in Forms mode on a signed (read-only) document it is disabled.
- f1–f5 (§9.19/§9.20): in the designer, empty signature options are enabled and the name field is disabled. rect/label/readOnly/required are editable and default/maxLen/multiline/options are disabled. `#field-new-kind` has signature, and adding one with no name gives "Added signature field Signature5" (Signature4 already existed). The inspector row reads "Signature field Signature1 — empty; click it on the page to sign". fields.count includes signatures while defs and values leave them out. help.rows is 11.
- d0/d1: a refused sign (wrong password) keeps the dialog open in field mode (sign.dialog {Signature4, 0}). Typing the real password and clicking Go signs: sign.last {ok, field:'Signature4', revisions 1}, the dialog closes, the open document stays dirty at revision 1, and the password is not in STATE.
- e0–e4, d2–d4: on the signed output the empty fields have no handles and title = aria-label. Clicking Signature4 opens #verify-dialog with its row aria-current, and the status becomes valid. The caption turns "✓ Valid, untrusted" with the classes `valid untrusted`, and that badge stays after switching to Forms mode. #verify-open opens the dialog. Clicking an empty field on the signed document opens the field sign dialog.

## Findings
1. SHOULD (renderer), src/renderer/index.html:139 and renderer.ts:1981
   - What happens: `#cert-sign-open` "Sign with certificate…" is always disabled. index.html ships it with `disabled`, and no renderer code ever sets `certSignOpenButton.disabled`. It was already like this in pre-p5u3-20260924.tgz.
   - Why it matters: §2.4 puts the new button "after #cert-sign-open", so in Forms mode a person sees a dead Sign button next to the working "Draw signature field". The menu is the only page/rect signing path.
   - Reproduction: probe session A step a4, a real click does nothing. Screenshot review-u3-c1-drawn.png shows the greyed button.
   - Fix: next to line 1981, add `certSignOpenButton.disabled = !doc || doc.info.needsPassword;`.
2. NIT (renderer), renderer.ts:2428-2429
   - What happens: in Forms mode, pressing on the body of an empty sig-target and dragging does not move the field. It opens the modal sign dialog, because the click fires on the unmoved button. The grip drag works.
   - Reproduction: probe step b2c, a 30 pt body drag leaves the rect unchanged and sign.dialog becomes {Signature1}.
   - Fix: in the existing pointerdown listener, record where the press started, and have the click listener return when the pointer moved 3 px or more.
3. NIT (renderer), renderer.ts:7466
   - What happens: after a draw finishes, the announcement (STATE.announcement) still reads "Draw signature field: drag a rectangle on the page". The screenshot's status bar shows it.
   - Fix: call `announce("")` in the onEnd disarm.
4. NIT (renderer), renderer.ts:4852 addDesignedField
   - What happens: a successful designer add leaves an earlier #error in place.
   - Reproduction: probe step f2, STATE.error is still "Draw a bigger signature field" after "Added signature field Signature5".
   - Fix: call `clearError()` on success, as the draw path effectively does.
- Observation (§9.40): an empty field with no /AP shows the app's blue "Sign" caption and mupdf's orange "SIGN" tag at the same spot, so the cue appears twice (review-u3-c1-drawn.png). This is acceptable. Masking the tag is optional.
