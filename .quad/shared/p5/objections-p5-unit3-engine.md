# P5 U3 objections — ENGINE + FIXTURES lens (Opus 5.5, 2026-09-24) — 3 blocking, 4 amend, 3 nit
Probes (no Electron, no build): `.quad/scratch/p5u3/engine/probe-engine.mjs` (log `probe-log.txt`), `probe-update-ap.mjs`, `make-hier.py` (h1..h5 scratch PDFs). Run from repo root with `node`. They use the P6 U4 `build/`. The field-add path is emulated the way the engine creates a field: `pdf.addObject` + `/Annots` + `/Fields`, then `widget.setRect`. mupdf's `createAnnotation` is not used. See `mergedFieldDict`, engine.ts:2353.

**E1** severity: blocking
- claim: §1.3 opens `field-props` rename and `field-delete` to empty signature fields. Three existing assertions expect `signature` on form2's `sig` field, and that field has no `/V`.
- evidence: fixtures/form2.pdf:70 `/FT /Sig /T (sig) /Rect [160 280 400 340]` (no `/V`). tests/drive/field-design.jsonl:25 `props name sig rename signed` → expect `detail "signature"`. §4.2 says that file is "never edited", and it is part of acceptance. tests/p3-drive.test.mjs:152 `byStep(20).last.detail === 'signature'`. tests/p3-forms.test.mjs:416 `remove(form2, sig.id).detail === 'signature'`. None of the three files has an owner in §5.
- amendment: §1.3: "An empty signature field takes `field-props {rect | label | readOnly | required}` and `field-delete`. A rename stays refused: `failField("signature", "field-props: a signature field keeps its name")`." (This keeps field-design.jsonl:25 and p3-drive:152 green.) §5 engine owns tests/p3-forms.test.mjs:414-416, which become: `field-delete` on form2 `sig` → ok, then `undo` puts it back (`fields` read back).

**E2** severity: blocking
- claim: §1.3's new `field-set` message breaks an existing test that the engine may only change additively.
- evidence: tests/p2-forms.test.mjs:308 `[item('sig'), 'nope', 'later version']` matches today's "field-set: signing arrives in a later version" (engine.ts:3302). §5 says p2-forms is "additive only".
- amendment: §5 engine: "`tests/p2-forms.test.mjs:308` changes its pattern from `'later version'` to `'click the signature field'`. This is the only non-additive edit."

**E3** severity: blocking
- claim: §1.4's field branch writes a broken file, or a file the app cannot read back, when the widget sits under `/Parent`. The name `listFields` reports is the full name, not `/T`.
- evidence: h1 (kid `/T (A)` under `/FT /Sig /T (Sigs)`): `listFields` name `"Sigs.A"`, own `/T` `"A"` (`widget.getName()`, engine.ts:1870). A `/T` match, as probe-sign-u3.mjs:31 does it, never finds this widget. Signing it with `/V` on the widget: pyHanko `Sigs.A INTACT ENTIRE_FILE trusted`, but the engine reopen reports `sigs 0` (countSignatureField needs `/FT` and `/V` on the same node, engine.ts:5129). The document stays writable, and `sign-list` → `too-large`. h5 (pure-widget kid of `/FT /Sig /T (P)`): pyHanko `expected 1 signatures, got 0`.
- amendment: §1.4, after `no-such-field`: "`invalid` (`sign-prepare: ${field} is part of a field hierarchy`) when the widget dict has `/Parent` or lacks its own `/FT /Sig` and `/T`. Only merged top-level fields are signed in place (a ceiling). Both the engine check and the reopened-source lookup match `widget.getName()`, never the raw `/T`."

**E4** severity: amend
- claim: "no `/AP`" (§1.2) and §1.3's rect change do not hold if the signature kind goes through today's create and props code. `refreshAppearance` calls `widget.update()` (engine.ts:2327), and on an `/FT /Sig` widget mupdf then draws its own orange "Sign" tag.
- evidence: probe C2 `setRect + update()` → `"AP":true`. probe-update-ap.mjs `/N` = `".95 .33 .18 rg 72 72 m 112 72 l 117 77 l … /F0 8.33 Tf … TJ"` (MuPDF's flag plus a new font). C4: Signature2 after `delete AP + setRect + update` → `"AP":true`. Also, mergedFieldDict maps every non-button, non-combo kind to `FT: "Tx"` (engine.ts:2359). `FieldAddKind` (:317) has no `signature`, and `FieldAddRequest.name` is a required `string`.
- amendment: §1.2/§1.3: "Signature kind never calls `refreshAppearance` or `widget.update()`. `field-add` writes no `/AP`. `field-props {rect}` does `setRect` and then deletes `/AP` (the field's old look is dropped, not stretched). `mergedFieldDict` maps `signature` → `FT: "Sig"`. `FieldAddKind` gains `"signature"`, and `FieldAddRequest.name` becomes `name?: string`."

**E5** severity: amend
- claim: "`default`, `maxLen`, `multiline`, `options`, `edit`, `kids` → the existing 'is a … property' refusals" is only partly true.
- evidence: `field-add` reads `options` only for combo and `kids` only for radio. Any other kind drops them silently (engine.ts:3362-3369). `checkFieldDefault` takes any string for a kind that is not checkbox, text or combo (engine.ts:2116). A signature `default: "x"` would therefore write `/DV (x)` on a `/Sig` field, in field-add and in field-props (`writeFieldDefault`).
- amendment: §1.2/§1.3: "For kind `signature`, before any write: `default` → `failField("invalid", "${op}: default is not a signature property")`, `options` → `"${op}: options are a dropdown property"`, `kids` → `"${op}: kids are a radio property"`. The existing maxLen/multiline/edit messages apply unchanged."

**E6** severity: amend
- claim: §1.4 lets a read-only signature field be signed. `field-add`/`field-props` can set `readOnly` on one, and Acrobat refuses to sign such a field.
- evidence: §1.2 allows `readOnly`, and `FieldItem.readOnly` comes from `widget.isReadOnly()` (engine.ts:1878). §1.4's check list has no read-only step.
- amendment: §1.4, after `already-signed`: "`invalid` (`sign-prepare: ${field} is read-only`) when the widget `isReadOnly()`." §1.5 adds one refusal case: `field-props {readOnly:true}` on hello's field, then save/reopen, then sign → `invalid`.

**E7** severity: amend
- claim: §3's fixture rules leave out the conventions `fixtures/p5/` already has. The provenance fields listed there do not match the schema the tests read.
- evidence: fixtures/p5/README.md has a regeneration block and a `| file | what |` table covering every p5 file. tests/p1-worker.test.mjs:40-54 reads `bytes`, `sha256`, `generator.reproduce` and `pages`, and that scan is non-recursive. The p6 precedent (fixtures/p6/links.provenance.json) has `id`, `file`, `pages`, `generator{path,helper,sha256,deterministic,timestamps_in_output,dependencies,reproduce}`, `provenance{authorship,third_party_content,fonts,network_access}` and `generated_on`. The scratch generator hard-codes `~/.../fixtures` (make-empty-sigfield.py:5).
- amendment: §3: "The provenance JSON uses the links.provenance.json schema (`pages: 2`, `generator.reproduce: "python3 fixtures/p5/make-empty-sigfield.py"`, `generator.helper: "fixtures/minipdf.py"`). The per-field rects are copied from probe G1/G5, since the generator runs without a build. The fixture agent adds `python3 fixtures/p5/make-empty-sigfield.py` to the README regeneration block and one table row, and owns README.md for that change."

**E8** severity: nit
- claim: §1.4's "the certified-visible refusal (4020) is skipped" does nothing, because a field request already requires `rect: null`. What matters is where the rect-null check goes.
- evidence: engine.ts:4020 `info.certified && request.rect !== null`. When `field` is set with a non-null rect, a certified document answers `certified` before the new `invalid` check can run.
- amendment: §1.4: "The `field`/`rect`/`page` argument check joins the invalid-args group (engine.ts:4002-4006). Line 4020 is unchanged."

**E9** severity: nit
- claim: §1.2's free-name rule and floor are ambiguous.
- evidence: U1 reads only the top-level `/T` of each field (sign-prepare.ts:29). `checkNameFree` uses full tree names (`fieldTreeNames`, engine.ts:2005). A 2×2 rect hits `checkFieldRect`'s "at least 4x4 pt" first (engine.ts:2166). Existing messages write `x`, not `×`.
- amendment: §1.2: "n is the first for which `Signature<n>` is not in `fieldTreeNames(existingFields(pdf))` (an empty set when there is no form). The 40x20 floor runs after `checkFieldRect`. The message is `field-add: a signature field is at least 40x20 pt`."

**E10** severity: nit
- claim: With §1.1's `/Parent`-chain `signed`, a field can read as signed while `info.signatures` is 0.
- evidence: h3 (`/V` on a kid with no `/FT`, `/FT` on the parent): `sigs 0`, the document is writable, `sign-list` → `too-large`. The badge would show "Signed", and Verify would then refuse.
- amendment: §1.1: "`signed` = the widget's own dict has `/V` (merged fields). With E3 in place, a hierarchy field is never signed in place, so the parent chain is not read."

## Confirmed (tried to break it, could not)
- (b) `checkFieldRect` + `setRect` handle `/Rotate 90`. Page-space `[100,100,300,160]` becomes raw `/Rect [100 100 160 300]`, and `fields` reports it back unchanged (C3).
- (c) A field-add-style `/FT /Sig` widget with no `/AP` lists as `kind signature`, renders (612x792 pixmap) and survives `garbage=yes`. Object numbers stay 7/8/10/11 even when orphan objects exist (C1, C3).
- (d) After an unsaved field-add in the same session, `save` (`serializeDocument`) bytes hold widget obj 7. Adopting it by name → `revisions 1`, pyHanko `Signature1 INTACT ENTIRE_FILE trusted docmdp-ok`. The U1 path then picks `Signature2` (D1).
- (e) On a signed source, `signedSource` is set and `fields` lists Signature1/2. Incremental adopt of Signature2: obj 8, `revisions 2`, prefix byte-identical (E1).
- (f) §1.5's signed-branch scratch document (`/V` widget not in `/Fields`, h4): `sigs 0`, `field-props` → `signature`, `dirty:false`.
- (g) `SaveReason` gains `no-such-field` additively. There is no `Record<SaveReason,…>`. `refusalOf` is untyped and `verifyRefusal` falls back to the raw token (renderer.ts:4665, 6800).
- (i) `certify` on a signed document → `already-signed` at 4018, before any field logic (I1). On a certified P=2 source, adopting Signature2, which has its own `/AP`: pyHanko `Signature2 INTACT ENTIRE_FILE trusted docmdp-ok`, `DocMDP 2` (I2). P=3 is not stricter than P=2 for form fill, and P=1 is refused at 4015.
- (h) The fixture is deterministic. Two regenerations → 1581 B, `07e46851…3206` (engine/regen*.pdf). `.quad/shared/p6/checks/fixtures.sha256` has 6 lines today, so 7 after the fixture is added.
