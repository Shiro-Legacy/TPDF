# P6 U7 review — ENGINE lens (Opus 5.5, 2026-09-24) — verdict REVISE (0 blocking, 3 should, 3 nit)

Ran (one lock hold each): `npm run build` 0; `node --test` p6-objects7/6/5, p6-direct3, p3-objects, p3-forms, p2-encrypt,
p2-shell, fixtures → 126/126 pass. Probes under `.quad/scratch/p6u7/review-engine/` (batch, batch2, paste, text, text2,
text3, kinds, marks, shapes, shapes2, extra `.mjs` + `.out`). Afterwards fixtures.sha256 9 OK, the fixture list is
byte-identical to u7-hash-before.txt, and `pgrep --drive=` is empty. The mutate diff against pre-p6u7-20260924.tgz
is the one §9.2 line. content-edit/worker/limits are unchanged, and engine-core has only the three OP_PERMISSION rows.

## Findings
E1 (should, engine.ts:3877-3893 and 3905-3920; §1.3 "per item … refusal, then ONE journal op", §0) — edit-batch checks
only link-delete ids before `mutate`. It sends every other annotation item (field-props, field-delete, link-set,
redact-set, annot-delete) through `dispatch` inside the open operation. When one of them refuses, the op is
abandoned after earlier items wrote, and the redo branch is lost. Repro (batch2.out, batch.out B3/B4):
field-defs `field-props 5`, then `undo` (canRedo true), then batch [field-props 5 valid, field-props 10 rect [0,0,1,1]]
→ refused `invalid`, index 1, canRedo **false**. On direct3, [edit-text move, edit-object move, field-delete 9999] →
`stale-target` index 2 with bytes equal, but canRedo goes true→false. The single `field-delete 9999` refuses with
canRedo still true. Fix: before `mutate`, resolve each widgetId/annotId (listFields / annotations / `not-redact`) and run
the single ops' rect checks (`checkFieldRect`, `checkLinkRect`, `redactRect`) with the item's index. Add a test that
does an undo first and pins `canRedo` after the refusal.

E2 (should, engine.ts:3613 and :3675; §10.4 "reply carries detail + {index, kind}", §10.22, §10.41) — two clip-paste
refusals set only `reason`: `failReason("unbalanced-content")` and the mark cap `failReason("too-many")`. The reply
has no `detail` (shapes.out: `{"reason":"unbalanced-content"}` with no detail). renderer.ts:3017 then records
`detail = "invalid-selection"`, so STATE `content.refused[].detail` is wrong. The texts `CLIPBOARD_DETAIL_TEXT
["unbalanced-content"]` and `["too-many:mark"]` never show. p6-objects7:576 hides this with `capped.reason ?? capped.detail`.
Fix: use `failDetail` or set `detail` for both, and pin `detail` in the test.

E3 (should, tests/p6-objects7.test.mjs, gap; §9.9, §1.4) — no test covers the clipboard byte charge. The behaviour
holds (paste.out P6: a 36 MB raw image → `too-large`, and the old Im1 clipboard still pastes `PA1`, which survives
close+open), but pre-fix code would pass the suite. Nothing tests a mixed content+annotation edit-batch either
(batch.out B1 proves it works). Fix: add both tests, the P6 recipe built in-test.

N1 (nit, engine.ts:3915; §1.3 BatchItem shapes) — `dispatch({ id, page, ...item })` passes unknown item keys through.
A batch `field-props {widgetId, rect, name:'renamed', required:false}` renames the field and clears a flag
(extra.out). An item `page` would override the batch page. Fix: build the synthetic request from the whitelisted keys.
N2 (nit, engine.ts:3591; §10.12/§10.15) — the paste block forces `0 Ts 0 Tr`. A `2 Ts` run pastes 2 pt lower than
source bounds + d, a `1 Tr` (stroked) run becomes filled, and the origin-only proof accepts both (text.out, text2.out).
The Help ceilings mention only black and kerning. Fix: add "rise and render mode are not kept" to the ceilings.
N3 (nit, p6-objects7:401-417, :630-651) — the annotate-only batch refusal and the copy-denied paste refusal do not
pin bytes or journal (no `still`/`nothingWritten`). Probes show nothing is written (kinds.out K4).

## Checked fine (reproduced)
- Cascade: two same-page pastes of objects6 Im1 → PA1 [82,590,…] and PA2 [92,580,…], revision +1 each. Page 1's own
  /Im1 stays width 6 and PA1 is width 4. Reopen keeps all three, and PA2 inks its whole box (4416/4416 px).
- edit-objects p0 (shared indirect /Resources): after the paste /Pages and page 1's dict and scan are unchanged. p0 gets a
  direct /Resources, and exactly 2 objects are added (image + stream). Undo restores both page dicts and /Root exactly.
- A shared SMask lands once (built doc: two images, one SMask → +4 objects incl. the stream; reopen equal).
- All-or-nothing: [Im1, field whose "<name> copy" exceeds 128] → `invalid-name` {index 1, kind field}. Journal, bytes,
  page dict and fields all stay equal. An existing "nom copy" → "nom copy 2", then "nom copy 3".
- Text: Tc 1.5 / Tw 4 / Tz 150 / TL 14 runs paste with equal bounds and a pixel-equal strip. The TJ kerning is dropped
  (width 109.24→104.24). Identity-H 日本語テスト → hello.pdf: candidate, bounds equal, strip equal after the garbage
  save → reopen. Two runs sharing one font graft it once (+2 objects).
- Every field of field-defs → hello.pdf (no AcroForm): names, kinds, values (Zed, checkbox true, radio B) and rects
  survive reopen. Undo restores /AcroForm and the page dict exactly.
- Links: a uri link and a page-2 link → direct3 reopen with the same target. The same pair → 2-page objects6 p1 skips
  the page link with `skipped` [{index 1}]. A search mark pastes with its quads shifted by d.
- edit-batch: run + Im1 + field + link + mark moved by 1 pt → ONE revision, in both item orders (§9.1). Undo restores
  contents, the page dict, fields, links and marks exactly, and redo and reopen hold. Two link-deletes by annotId
  (10, 11) work in one op. A `text` key → invalid-selection. `session.batch` is false after a throwing batch.
- Refusals: image onto direct3 p1 /Rotate 90 → unsupported-transform {index 0, kind image} with dirty false and bytes
  equal. Annotate-only [link, mark, text] → read-only {index 2, kind text}, and a link-set + redact-set batch passes.
  Copy dup → invalid-selection, the wrong kind → not-candidate with its targetId, a stale revision → stale-target.
- §1.5: :350 hoists im3, and :234 adds the box:'junk' invalid-transform case. OP_PERMISSION holds the three rows.
