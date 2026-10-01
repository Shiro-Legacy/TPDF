# P5 U3 objections — renderer / UX lens (Opus 5.5, 2026-09-24) — 5 blocking, 9 amend, 4 nit
Scope: §2, §2.5–§2.7, renderer parts of §4. All anchors checked by grep/sed on src/ and tests/ (no Electron run; no probe needed: every item is readable from code).

R1 — severity: blocking — `tests/drive/direct-edit.jsonl` breaks: its click on form2's EMPTY `sig` field now opens a modal the drive cannot close
- claim: §2.2 changes the click on form2's `sig` (no `/V`, fixtures/form2.pdf obj 18) from a status line to `showModal()`, and the drive `key Escape` never closes a dialog, so the page stays inert for the 40+ steps after it (count pinned at 75 in p3-drive.test.mjs:427).
- evidence: direct-edit.jsonl:32 `"x": 280, "y": 482, "expect": {"direct": {"active": null, "status": "Use Sign with Certificate to sign this field."}}`; renderer.ts:1345 Escape branch only `closeDirect(); … dropDirectFocus(); return;`.
- amendment: "§2.2: `driveDirectKey` Escape first closes an open `#cert-sign-dialog` or `#verify-dialog` (`dialog.close()`; a real Escape does the same) and returns. The tests role owns `tests/drive/direct-edit.jsonl` for one edit: line 32's expect becomes `{"direct":{"active":null,"status":"Sign with certificate"},"sign":{"dialog":{"field":"sig","page":0}}}`; line 33 (`key Escape`) then closes it. Step count stays 75."

R2 — severity: blocking — existing pins on form2's empty `sig` refusal flip under §1.3, and §4.2 forbids editing one of them
- claim: §1.3 lets an EMPTY signature field be renamed/deleted and rewords `field-set`, so four existing assertions fail; `field-design.jsonl` is declared "never edited".
- evidence: field-design.jsonl:25 `{"action": "props", "name": "sig", "rename": "signed"}, "expect": {"fields": {"last": {"detail": "signature"}}}` + p3-drive.test.mjs:152 `byStep(20).last.detail, 'signature'`; p3-forms.test.mjs:416 `remove(form2, sig.id).detail, 'signature'`; p2-forms.test.mjs:308 `[item('sig'), 'nope', 'later version']`.
- amendment: "§4/§5: the tests role owns field-design.jsonl:25 (expect becomes `{"fields":{"last":{"action":"rename","name":"signed"}}}`) and p3-drive.test.mjs:152 (`byStep(20).last` deep-equals `{action:'rename', name:'signed'}`); the engine role owns p3-forms.test.mjs:416 (`remove(...)` → `ok:true`) and p2-forms.test.mjs:308 (message `click the signature field`). The signed-field refusal stays pinned by §1.5's scratch document."

R3 — severity: blocking — `fields.last.name` for an unnamed signature add reads `''`, not the engine-chosen name
- claim: §2.4 says `driveFields` passes a signature add "unchanged", but `driveFields` and the panel set `fieldDesign.name` from the request before the reply, so §4.1's `fields.last {action:'add', name:'Signature1'}` fails.
- evidence: renderer.ts:4301 `fieldDesign = { action, name };` (name = `""`) then `reply = await request(payload)`; 4806 `fieldDesign = { action: "add", name };` and 4807 status `Added ${kind} field ${name}`.
- amendment: "§2.4: on success every add path (drag, `driveFields`, `addDesignedField`) sets `fieldDesign = {action:'add', name: reply.name ?? name}` and the status `Added ${kind} field ${reply.name}`; a refusal keeps the requested name (`''` when none was given)."

R4 — severity: blocking — `field-set` on a signature field reports `detail:'invalid'`, not `'signature'`
- claim: `refusalDetail` reads `reply.detail`, which the engine only sets for `failField`; the plain `Error` that §1.3 keeps yields `"invalid"`, so §4.1's `fields.last {action:'fill', name:'S1', detail:'signature'}` fails.
- evidence: engine.ts:3302 `throw new Error("field-set: signing arrives in a later version")`; engine.ts:4968 `...(tagged?.detail ? { detail: tagged.detail } : {})`; renderer.ts:4675 `: "invalid"`.
- amendment: "§1.3: `field-set` on a signature field throws `failField("signature", "field-set: click the signature field to sign it")` (was a plain Error); `driveFields` then reports `fields.last {action:'fill', name, detail:'signature'}` and the panel shows `FIELD_DETAIL_TEXT.signature`."

R5 — severity: blocking — §2.4 prescribes a nested `runGestureWork`, so the drive's `drag` returns before `field-add` lands
- claim: `startAreaDrag` already wraps `onRect` in `runGestureWork`; if `onRect` calls `runGestureWork` again, the outer promise overwrites `directGestureWork` and resolves at once, so `driveDirectGesture` awaits nothing and `revision 1` / `fields.last` race.
- evidence: renderer.ts:5243 `if (rect) runGestureWork(async () => { await onRect(rect); });`; 2510 `directGestureWork = promise;` (set after `work()` has started); 1286 `if (directGestureWork) await directGestureWork;`.
- amendment: "§2.4: `startAreaDrag(event, page, rect => addSignatureField(page, rect), disarm)`, where `addSignatureField` is async and returns its promise (size check, `showError`, request, `applyEdit` all inside it). No `runGestureWork` inside `onRect`."

R6 — severity: amend — a stale `certSignField` makes a later page/rect `sign` sign the field instead
- claim: §2.3 nulls `certSignField` only on close/reset, so a drive `click` on a field followed by `sign {page, rect}` (no `field`) reuses the open dialog and signs the field; `openCertSignDialog()` without a field never un-hides the page row either.
- evidence: renderer.ts:6746 `if (!certSignDialog.open) certSignDialog.showModal();` (dialog reused); 6835 `certSignDialog.showModal();` unguarded; `#cert-sign-page` sits in a `<p>` with its label (index.html:770).
- amendment: "§2.3: one `setCertSignMode(field: FieldItem | null)` sets `certSignField`, shows/hides `#cert-sign-field`, `certSignPage.closest('p')` and the Appearance fieldset. `openCertSignDialog(field?)` calls it (`null` without a field) and `showModal()` only when `!certSignDialog.open`. `driveSign` calls it in both branches. `runCertSign` captures `certSignField` once; in field mode it validates and sends `certSignField.page` and `rect:null`, and it ignores `certSignPage`/`certSignVisible`. `sign.dialog = certSignDialog.open && certSignField ? {field, page} : null` (the close event is async)."

R7 — severity: amend — the signature target has no handle guard: with a real mouse, every move or resize opens the sign dialog
- claim: the grip is placed outside the box and moves with it, so the click that follows pointerup lands on the handle, bubbles to the button and opens the modal after each gesture. The field loop guards against this; the signature loop does not.
- evidence: renderer.ts:2393 `if ((event.target as HTMLElement).closest(".hit-handle, .hit-grip")) return;` vs 2410 `button.addEventListener("click", event => { event.preventDefault(); button.focus(); directClickWork = activateField(item); })`; styles.css:584 `.hit-grip { … left: -15px; top: -15px; }`.
- amendment: "§2.1: the `.sig-target` click listener returns early when `event.target.closest('.hit-handle, .hit-grip')`, as at renderer.ts:2393."

R8 — severity: amend — the plain-arrow nudge clause does nothing: signature buttons have no keydown handler and no `data-field-id`
- claim: adding `"signature"` to the condition at 2400 changes nothing, because that handler sits only on the non-signature loop. Without `dataset.fieldId`, focus is also lost at the repaint after any nudge, drag or dialog close (`focusedField` refocus at 2413).
- evidence: renderer.ts:2400 `else if (plain && uiMode === "forms" && (item.kind === "checkbox" || item.kind === "radio"))` inside the loop that `continue`s signatures at 2385; the 2404-2411 loop sets no `dataset.fieldId` and no keydown.
- amendment: "§2.1: the `.sig-target` also carries `data-field-id` and a keydown handler: ⌥-arrows and (Forms mode, `!item.signed`) plain arrows → `runGestureWork(() => nudgeField(item, event))`; Enter/Space → the click path. Tab is left to the browser (signatures stay out of `/Tabs` order)."

R9 — severity: amend — nothing repaints the badges after Verify, and the signed-field click must await it
- claim: `refreshVerify` rebuilds only the dialog table, so the `valid`/`invalid` classes never appear on the page after `verify` (§4.3's "✓ Valid" screenshot) until an unrelated repaint.
- evidence: renderer.ts:6789-6791 `verifyLast = {…}` ends `refreshVerify`, with no `paintHits`; `openVerifyDialog` returns `refreshVerify()` (6750-6752).
- amendment: "§2.5: `refreshVerify` sets `verifyByField` from the non-empty rows (and clears it on a refusal), then calls `paintHits(canvas.clientWidth, canvas.clientHeight, shownPage)`. `activateField` on a signed field does `await openVerifyDialog()`, so `directClickWork` covers it."

R10 — severity: amend — the designer panel blocks every signature field, so a person cannot delete or rename an empty one (the drive can)
- claim: §1.3 opens props/delete for empty fields and §2.4 keeps "`option.disabled`". There is no on-page delete for fields, so deleting or renaming is drive-only, which breaks the human/AI parity rule.
- evidence: renderer.ts:4732 `option.disabled = item.scripted || item.kind === "signature";`, 4744 `usable = … item.kind !== "signature"`, 4749, 4835 and 4885 `|| item.kind === "signature") return;`; 4796 `as "text" | "checkbox" | "combo" | "radio"`.
- amendment: "§2.4: at 4732/4744/4749/4835/4885 the signature test becomes `item.kind === "signature" && item.signed`. For a signature item the panel enables name, rect, label, read-only and required and disables default/maxLen/multiline/options. `addDesignedField` adds `"signature"` to the kind union and never sends `default` for it."

R11 — severity: amend — the Help, inspector and panel texts that §2.7 leaves untouched become false
- claim: four existing sentences now contradict the unit.
- evidence: index.html Direct editing ceilings `plain arrows nudge a checkbox or radio in Forms mode only`; the Certificate signing ceilings `a certified document accepts only an invisible signature`; the Form designer ceilings cell `… list boxes, push buttons, drag placement`; renderer.ts:4470 `Signature field — sign in a later version.` and 4650 `"Signature fields cannot be changed here."`.
- amendment: "§2.7 adds: the nudge `<li>` reads 'a checkbox, radio or empty signature field'; 'a certified document accepts only an invisible signature or a signature in an existing empty field'; the designer cell's 'drag placement' becomes 'drag placement (except signature fields)'; the inspector row reads `Signature field ${name} — ${status word}; click it on the page to sign`; `FIELD_DETAIL_TEXT.signature` = 'A signed field cannot be changed.'"

R12 — severity: amend — the armed signature-field press has no permission or mode check
- claim: `driveArm` presses any mapped button, disabled or not (the `disabled` attribute only stops real clicks). The pageRegion branch must therefore refuse by itself, as `drawnLink` does with `canAnnotate()`.
- evidence: renderer.ts:1416 `button?.setAttribute("aria-pressed", "true");`; 5249 `if (!doc || !canAnnotate()) return;`.
- amendment: "§2.4: `addSignatureField` returns without a request unless `canDesign()` and `uiMode === 'forms'`, and in that case sets `fieldDesign = {action:'add', name:'', detail:'invalid'}`."

R13 — severity: amend — a refused drive `sign` leaves the modal open, so a later `click` in the same script hits an inert page
- claim: `runCertSign` keeps the dialog open on every refusal (right for a person). In §4.1, `already-signed` leaves it open through the `Nope`/`goto` steps, which is harmless only because no `click` follows.
- evidence: renderer.ts:6893-6899 (the `!prepared.ok` branch does not close); §4.1 order `already-signed` → `Nope` → `goto` → `sign Signature3`.
- amendment: "§4.1 header comment: 'a refused sign leaves #cert-sign-dialog open (sign.dialog non-null); send key Escape (R1) before any click'. §4.1 adds `sign.dialog {field:'Signature1', page:0}` to the already-signed step's expect."

R14 — severity: amend — `certSignField.page` should come from `fieldsPage`
- claim: the lookup runs over `fields`, which belongs to `fieldsPage`; during a page change `wantedPage` can already be the next page.
- evidence: renderer.ts:4397 `const page = wantedPage;` then an await before `fieldsPage` is set; the while-loop wait at 1239 exists only in click/drag.
- amendment: "§2.5: `driveSign` with `field` waits as `driveDirectClick` does (`fieldsPage === wantedPage`) and then sets `certSignField = {name, page: fieldsPage}`."

R15 — severity: nit — caption and badge CSS on a real page
- claim: a top-left caption inside the box sits under the nw handle (z-index 3, styles.css:592). '✓ Valid, untrusted' at 10 px is about 95 px wide, which overflows a 40×20 pt field. `.hit-target:hover` (567) and `.sig-target` have the same specificity, so either the hover outline or the badge colour is lost depending on rule order. The inherited `cursor: text` is wrong for a sign target.
- evidence: styles.css:562 `cursor: text;`, 567, 584-592; the app has a single dark theme (`color-scheme: dark`, styles.css:5), but the targets sit on the white paper.
- amendment: "§2.1 CSS: `.sig-target{cursor:pointer}` `.sig-target::after{position:absolute;left:0;bottom:100%;white-space:nowrap;font:10px/1.2 system-ui;background:#fffd;padding:0 2px;pointer-events:none}`, with the badge colours on `::after` only and the hover outline kept."

R16 — severity: nit — the `pdf4all` secret is safe as a substring today; say so and state the limit
- claim: after the first `sign` step, any STATE line containing `pdf4all` fails the run. STATE carries only basenames (`doc.name`, `sign.last.name`, `save.name`) and the signer CN `P0B Signer`. The udata/OUT paths never reach STATE, so the fixed password does not false-fail sign.jsonl, verify.jsonl or sign-field.jsonl.
- evidence: safe-write.ts:232 `name: basename(destPath)`; `openssl pkcs12 -in fixtures/p5/leaf.p12` → `subject=CN=P0B Signer`; lowercase `pdf4all` appears in renderer strings only in `window.pdf4all is unavailable` errors, which are unreachable in Electron.
- amendment: "§2.6 adds: 'the leak check is a substring test; no drive fixture or output name may contain `pdf4all`'. The tests role cannot `pick()` a `sign` step by `contents` (it is `[redacted]`), so it picks by `to` or by index."

R17 — severity: nit — `editSummary` can overwrite `sign.dialog`
- claim: `stateSummary` spreads `...editSummary()` after `sign: {last}`, and `editSummary` emits `sign: {name, subj}` when the page has an ink 'Signed by' stamp, which removes `sign.last` and `sign.dialog` from STATE.
- evidence: renderer.ts:6599 `sign: { last: signLast },` and 6674 `...(sign ? { sign } : {}),`.
- amendment: "§2.5: `sign.dialog` sits beside `sign.last`; the ink-stamp collision is pre-existing (no U3 fixture has one) and is recorded as a ceiling, not fixed."

R18 — severity: nit — the signed-document read-only title
- claim: today an empty field in a signed document gets `title = "This document is read-only"`, but it is signable. §2.1's `title` mirroring `aria-label` fixes this only if the `else` at 2409 is removed.
- evidence: renderer.ts:2409 `if (editable() && permitted("edit")) addFieldHandles(...); else button.title = "This document is read-only";`.
- amendment: "§2.1: the `.sig-target` `title` is always the aria-label text; handles only when `editable() && permitted('edit') && !item.signed`."

## Confirmed (tried to refute, could not)
- (a) Signature target: `pointerdown` preventDefault and click → `directClickWork = activateField(item)` (renderer.ts:2409-2411); `driveDirectClick` awaits it (1258). `activateField` is `Promise<void>`, so a sync `openCertSignDialog` fits.
- (d) `driveSign` guards `showModal` (6746); `pickCertSignFile` is plain IPC and works with the dialog open (6842-6851).
- (e) `fieldGesture`/`nudgeField` → `field-props` has no kind filter (2248-2261). The renderer's kind filters are readingOrder/tabOrder/fieldSummary (keep) plus the designer sites in R10.
- (f) Read/Annotate: plain-arrow nudge requires `uiMode === "forms"` (2400), so Escape followed by arrows scrolls in Read mode. No conflict.
- (g) `resetDocument` → `resetEditState` runs on every open (1552, 836), so `verifyByField` clears per document. `sign-list` names are AcroForm full names, and a multi-widget field shares one status, which is correct.
- (h) An armed tool makes `layerLive()` false → `driveDirectGesture` sends the press to `pageRegion` (1275-1283); `.hit-target` has `pointer-events:none` when the layer is not live (styles.css:560). `startAreaDrag` 3 px threshold (5241).
- `fields.count` includes signatures; `values`/`defs` skip them (4602, 4613, 4629).
- (j) `helpRows()` = tBody rows (1498), pinned at 11 (p2-shell:126). No test pins 'signature fields' or 'drag placement' text.
- (k) p2-shell names a button by its text (named(), p2-shell:59-64); `<p id="cert-sign-field">` is not scanned. `REQUIRED_IDS` needs no change.
- (l) A `sign` step is not redacted today: `driveSecrets` returns `[]` for any cmd other than password/encrypt/encrypt-keep (main.ts:1666-1668).
