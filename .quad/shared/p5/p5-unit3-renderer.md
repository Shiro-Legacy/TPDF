# P5 U3 renderer — evidence (renderer seat, opus55, 2026-09-24)

Files: `src/renderer/renderer.ts`, `src/renderer/index.html`, `src/renderer/styles.css`, `src/main.ts` (driveSecrets only), `tests/p2-shell.test.mjs` (two ids added to REQUIRED_IDS). No change to `src/preload.cts` or `src/sign/*`.

Shipped (contract §2 as amended by §9):
- §2.1/§9.16/§9.17/§9.24/§9.26 `button.hit-target.sig-target[data-field-name][data-signed][data-field-id]`, aria-label = title = `Signature field <name>, <status>`; classes `valid|invalid|unsupported` (+`untrusted` when the last Verify row is not trusted); handles only when writable and `!signed`; click ignores `.hit-handle, .hit-grip`; keydown: ⌥-arrows and Forms plain arrows nudge an empty field, Enter/Space → the click path. CSS: dashed `#2f6fd6`, `::after` captions Sign / Signed / ✓ Valid / ✓ Valid, untrusted / ✗ Invalid / ? Unsupported.
- §2.2/§9.18 `activateField`: empty → `openCertSignDialog(item)`, status "Sign with certificate"; signed → `await openVerifyDialog()` + the row `aria-current`, status "Verify signatures".
- §2.3/§9.15 `setCertSignMode(field|null)`, `#cert-sign-field` line, page row + Appearance fieldset hidden in field mode; `runCertSign` captures `certSignField`, sends `{page, rect:null, field}`; `signLast.field`; dialog close + `resetEditState` null it.
- §2.4/§9.13/§9.14/§9.21 `button#sigfield-draw` (Forms row, disabled unless `canDesign()`), `driveArm` `sigfield`, `layerLive`/`cancelPendingTools`; pageRegion order redact → link → sigfield → unarmed redact; `addSignatureField` async inside `startAreaDrag` (no nested runGestureWork), 40×20 floor → "Draw a bigger signature field"; designer kind `signature` (empty name omitted, no default sent); every add path takes the name from `reply.name`.
- §9.19 designer: only SIGNED signature items are disabled; rename input disabled for any signature item and no `name` travels in its `field-props`.
- §2.5/§9.23/§9.25 STATE `signatures[]` and `sign.dialog {field, page}`; `driveSign {field}` waits for `fieldsPage === wantedPage`, missing → `no-such-field` with the dialog untouched.
- §9.12 `driveDirectKey` Escape closes an open sign/verify dialog first and returns.
- §2.6/§9.27 `driveSecrets` returns a `sign` step's `password` → contents `[redacted]`; all six reports below hold no `"pdf4all"` / `"password"` string.
- §2.7/§9.20 Help: Certificate signing row (both cells), Form designer ceilings cell + `<ul>` item, direct-editing nudge `<li>`; inspector row text; `FIELD_DETAIL_TEXT.signature`. helpRows() still 11 (help-fields.jsonl exit 0).

Root-cause fix found on the way: `paintHits` re-focused the previously focused target with `focus()` (scrolls); after the sign dialog restored focus to the off-screen sig target, the next drive click at the page top was scrolled away (direct-edit step 29 red). Now `focus({ preventScroll: true })`.

Runs (under the lock): `npm run build` ok; `node --test tests/p2-shell.test.mjs tests/p3-drive.test.mjs tests/p6-direct.test.mjs` → 27 pass, 0 fail (incl. direct-edit.jsonl 75 steps with the tests seat's line-32 expect). `node scripts/drive.mjs` exit 0: sign, verify, direct-edit5, field-design, help-fields. `tests/drive/sign-field.jsonl` (tests seat, 32 steps): 31/32 green; step 19 expects `revisions:3` for a third signature, engine replies 2 (`src/renderer/sign-prepare.ts:128` returns `sourceLength ? 2 : 1`); the file itself has 3 `%%EOF`s and pyHanko says `Signature4/Signature1 ENTIRE_REVISION, Signature3 ENTIRE_FILE`, all INTACT trusted docmdp-ok. Engine or tests seat to settle.

Hashes: `shasum -c .quad/shared/p6/checks/fixtures.sha256` all OK; fixtures/**/*.pdf|png vs u3-hash-before.txt → only `fixtures/p5/empty-sigfield.pdf` added. `pgrep -fl -- --drive=` empty.

Not done: no real-mouse (CDP) check of the draw / handle guard / keyboard nudge on a sig target (the GUI script is the tests seat's §4.3).

## Fixes after review

- R-U3-1 (reproduced: probe session A a4 BAD, dialog stayed closed): renderer.ts next to the sigfield-draw enable block now sets `certSignOpenButton.disabled = !doc || doc.info.needsPassword;`. After: a4 ok (dialog open, page/rect mode, no field line). New p2-shell test: every button shipped `disabled` in index.html with its own `const` must have a `.disabled =` assignment in renderer.ts (fails pre-fix on #cert-sign-open only).
- S2 (reproduced: tests-probe steps 1-10, step 9 Enter → sign.dialog null, red): `#hit-layer .sig-target` added to driveDirectKey's onTarget selector, so the drive key reaches the target's own keydown (Enter/Space → activateField) and awaits directGestureWork. After: same 10 steps all green. New p2-shell static test pins the selector.
- Skipped (not my file): tests/drive/sign-field.jsonl after the ArrowRight nudge step (revision 9) should add `{"cmd":"key","key":"Enter","expect":{"sign":{"dialog":{"field":"S1","page":0}},"revision":9}}` and `{"cmd":"key","key":"Escape","expect":{"sign":{"dialog":null},"revision":9}}`, with tests/p5-drive-field.test.mjs:80 step count 43 → 45. Verified locally that this drive passes post-fix, then reverted.
- Ran under the lock: tsc, build, p2-shell + p3-drive + p5-drive-field 30/30, drive sign-field / direct-edit / sign / verify all exit 0. Fixture hashes unchanged (only empty-sigfield.pdf new). Pre-existing probe f4 BAD not in scope.
