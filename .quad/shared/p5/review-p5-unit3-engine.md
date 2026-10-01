# P5 U3 review — ENGINE + FIXTURES lens (Opus 5.5, 2026-09-24)
Verdict: REVISE (1 blocking, 1 should, 2 nits). Probes: `.quad/scratch/p5u3/review-engine/{probe,probe2,u1same}.mjs`, log `probe-log.txt` (no secrets; the p12 password came from an env var).

## Ran
- Lock: `npm run build` exit 0; `node --test` p5-sign-field, p5-sign, p2-forms, p3-forms, p2-shell, fixtures, p1-worker, p3-drive → 116/116 pass, 0 fail. `npx tsc --noEmit` clean.
- Headless probes against build/renderer/engine.js + cms/patchSignature + pyHanko (`--ca fixtures/p0b/.ca/ca.pem`).
- Fixture: generator run twice from /tmp into scratch (argv path) → both 07e46851…a206, 1581 B; provenance identical to the committed one; generator sha 3e716f… matches provenance; `shasum -c` p6 list 7/7 OK; fixtures/**/*.pdf|png vs u3-hash-before.txt → only `fixtures/p5/empty-sigfield.pdf` added.
- Backup diffs: p3-forms only 416→416-420 (inside the allowed 414-416 edit), p2-forms only 308, p5-sign additive (+1 test), fixtures.test.mjs +1 PINNED line; field-design.jsonl and p3-drive.test.mjs identical.
- U1 path byte-for-byte: old sign-prepare.ts (from the backup, type-stripped) vs new on hello/edit-tj/form2/empty-sigfield × {visible, invisible, certify} → identical; on p0b/signed-intact (incremental) only the random second `/ID` differs (old vs old differs the same way).

## Findings
**E-R1 (blocking) engine.ts:1929 — a `/Sig` widget that is not in `/AcroForm /Fields` is "signed" but no verifier sees it.** §9.3 "only merged top-level fields are signed in place"; §0 proven-or-refused.
Repro (probe2 B): fixture with `/Fields [8 0 R 10 0 R]` (Signature1 still in page `/Annots`, same byte length). `fields` lists Signature1 `kind:signature signed:false` (so the renderer paints a `.sig-target`); `sign-prepare {field:'Signature1'}` → ok; after patch+CMS: reopen `info.signatures 0`, `save` ok (writable), `sign-list` omits it; pyHanko `expected 1 signatures, got 0` (exit 1). The app reports `sign.last ok:true` for a file with no signature.
Fix: in `checkSignatureField` also refuse `invalid` (`… is part of a field hierarchy` or `… is not a form field`) when `dict` is not an entry of `existingFields(pdf)` (compare `asIndirect()`); add the case to tests/p5-sign-field.test.mjs (fails today).

**E-S1 (should) tests/p5-sign-field.test.mjs:136-140 — §9.3 hierarchy refusal and §1.4 "too small" have no test.**
Probe: h1 (`Sigs.A` kid), h2 (/V on parent), h3 (/V on kid), h5 (pure-widget kid) → all `invalid` "…is part of a field hierarchy", journal unchanged; fixture with Signature1 /Rect 30×10 → `invalid` "…too small for a signature appearance". Behaviour holds, but deleting the /Parent test or the size test at engine.ts:1929/1934 leaves the suite green.
Fix: one test building an h1-style doc with `buildPdf` and one with a 30×10 `/Sig` widget, each asserting reason + message + `journalState` unchanged.

**E-N1 (nit) engine.ts:1901 — `signed` is false for a field whose `/V` sits on its `/Parent` (h2).** `info.signatures 1`, `fields` → `P signed:false`, so STATE `signatures[].status` reads `empty` and the page shows "Sign" on a signed field; a click then gets the hierarchy refusal. This is what §9.10 specifies (own /V only). Fix if wanted: `signed = !!at(dict,"V") || !!at(at(dict,"Parent"),"V")`.

**E-N2 (nit, pre-existing U1) sign-prepare.ts:85 — `/Rotate` inherited from `/Pages` is ignored.** Fixture variant with `/Rotate 90` on the Pages node: `fields` rect `[112,72,172,340]`, `sign-prepare {field}` writes `/Matrix[1 0 0 1 0 0]` and the rendered appearance is squashed sideways (out/inherited-rotate.png). pyHanko is not affected. Engine `pageRotation` (engine.ts:1331) has the same gap. Fix: `doc.findPage(n).getInheritable("Rotate")` in both places.

## Checked fine (reproduced)
- (1) Signature1/2/3 each: ok, revisions 1, rects [72,620,340,680]/[360,620,540,680]/[500,72,560,300], object numbers 7/8/10 unchanged, only the signed one has /V; Signature2's /AP is `{N}` with the U1 stream (the grey stream is gone, not stacked); pyHanko `<field> INTACT ENTIRE_FILE trusted docmdp-ok`; reopen signatures 1, `signed:true`, save `read-only`, sign-list `empty:false`. Rotated: /Matrix [0 1 -1 0 0 0], text upright inside the field (out/sig3-page1.png; the 60 pt width clips the text, a fixture-shape consequence).
- (2) chain Signature1 → Signature2: prefix identical, revisions 2, increment rewrites `8 0 obj`; pyHanko Signature1 ENTIRE_REVISION + Signature2 ENTIRE_FILE, docmdp-ok. A U1 invisible sig on top → revisions 3, pyHanko 3 intact.
- (3) order: rect non-null/undefined, `''`, number → invalid (arg group); wrong page → `no-such-field` (as §1.5 pins); page 9 → invalid page; unknown → `no-such-field` with the §1.4 message; read-only → invalid; certified P=2 + field → ok, pyHanko `DocMDP 2` both docmdp-ok; P=1 → `certified`; certify+field on a certified doc → `already-signed`; certify via field on an unsigned doc → `/Perms /DocMDP`, pyHanko `DocMDP 2`. Every refusal leaves `journalState` equal.
- (4) field-add signature: widget keys `Type Subtype FT T Rect F P DA MK` (no /AP, no /V) even after render + undo/redo + save; no `/SigFlags` written (hello none, fixture stays 1); names Signature4 on the fixture, Signature1 when only `Sigs.Signature1` exists (full names differ); 4×4 before the 40×20 floor (39×20 and 40×19 refused, 40×20 ok); default/options/kids/maxLen/multiline/edit refusals; `name` in the reply for text, checkbox, combo, radio; /Rotate 90 page rect round-trips through save.
- (5) props label keeps the grey /AP; rect → /AP deleted (after render + save); undo restores the grey /AP; rename → `signature` "keeps its name"; kids/maxLen refusals; delete + undo restores obj 8 in /Fields [7,8,10] and /Annots [7,8]; field-set → `signature` "click the signature field to sign it"; a signed doc refuses props/delete as `read-only`, the scratch doc path is `signature` (test).
- (6) info.signatures 0 on the fixture, writable. (7) same session field-add → render → sign-prepare {field} → pyHanko INTACT.
- (8) README lines 9 and 26; PINNED_SHA256 line 11; p6 list line 7.
