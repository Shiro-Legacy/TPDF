# Review — coded simple fonts (reviewer, Opus 5.5, 2026-09-25) — ACCEPT with 1 fix applied
Probes live in .quad/scratch/coded/review/ (probe1-9.mjs, ledger.mjs, sample-p2.jsonl; pre-unit engine compiled to review/pre-build/ from the pre copy, build/ untouched by it).

1. should-fix, FIXED — `Tw` ignored for the single-byte code 32. `editMeasureCid` added no word spacing, but PDF (and mupdf) apply Tw to every
   1-byte code 32 of a simple font. probe2.mjs: space moved to code 32 + `50 Tw` → mupdf stext puts `ipsum` at x 209.3 (not 159.3), so +50 pt.
   The pre-fix build accepted `Dolor`→`Do lo` under `200 Tw` (`ok:true`; drawn 200 pt wider than measured, past the limit). The Downloads sample has
   0 `Tw` ops on its 30 pages (probe3.mjs), so no real file hit it. Fix: content-edit.ts `editMeasureCid` adds `wordSpacing × count(code 32)` when
   bytesPerCode is 1 (Identity-H unchanged). New test `coded (g)` in tests/p3-coded.test.mjs: red on the pre-fix build (`ok:true`), green after (overflow, newWidth = glyphs + 200).
2. OK — D1 gate probes (probe1.mjs, synthetic copies): 2-byte codespace `<0000> <FFFF>` with 1-byte or 2-byte sources, and a mixed `<00><FF>`+`<0000><FFFF>`
   → all 3 runs `unsupported-encoding`, text null (no wrong decode). Flags 36 (4|32), `/Encoding` dict, `/Encoding /MacRomanEncoding`, Widths holding a name,
   FirstChar 1.5, `usecmap` → `unsupported-encoding`. Flags 5 (4|1) and Type1 + FontFile → coded candidates (edit ok), as D1 says.
3. OK (info) — Widths shorter than the codes / FirstChar 5: missing codes measure `dw` (MissingWidth or 0), and mupdf also draws them with that advance
   (probe2.mjs: `Dolor` bbox 72–72 with Widths length 1), so the model matches the page; no overflow slip. `xx` then turns `ambiguous-text-association` (safe).
   With MissingWidth 600 the 7×Dolor edit refuses `overflow` (newWidth 688.8 > limit 116).
4. OK — write (probe6.mjs): TJ `[<0102>-50<…>] TJ` → `<0708090a05060102030405> Tj`; StructElem `/ActualText` (MCID) is rewritten to `ipsum Lorem`; saved stext
   `[PDF4All, ipsum Lorem, Dolor, xx]` after `garbage` save reopen; undo byte-identical in both; `Dolx` refusal leaves revision 0 and nothing to undo.
5. OK — regression ledger (ledger.mjs): pre-unit engine vs build/ on all 54 fixture PDFs, 108 pages (≤30/file), items compared as
   [kind, support, reason, text, repertoire, bounds]: 1 page differs, fixtures/p0b/edit-coded.pdf (4/1 → 4/4). cjk, rtl, edit-embedded, edit-subset,
   edit-transformed, p6/paragraph and the rest are identical.
6. OK — sample. The 45th target on page 1 is a DejaVuSans (F2) run holding code 0x22, whose ToUnicode destination is `006600660069` (the `ffi` ligature,
   3 chars) → that code cannot be mapped → `unsupported-encoding` is correct (probe5/5b: 1 such run on page 1, 4 on page 2 = the 41/37 split).
   Page-2 drive (sample-p2.jsonl, 5 steps, 0 red): `ultricies ut purus. Ut facilisis et lacus eu cursus.` → `cursus eu lacus et facilisis. Ut purus ut ultricies`,
   revision 1, save-to OK. Saved file (probe7.mjs): only page-2 line 6 changed in stext, pages 1/3 unchanged, dark px in the line 464→480, operand written
   `<0f0710…>Tj` (2 hex digits per code), render viewed. Saved copy and PNG deleted.
7. OK — fixture: generator run twice from a scratch copy → byte-identical, equal to the committed file (8f6b3bfa…, 3940 B), provenance identical; generator
   sha 6a71cbf4… matches; font URL/sha256/OFL-1.1 match docs/third-party/fixture-fonts.md:16. Program (probe9.mjs): 10 tables with consistent offsets,
   no stray table, cmap = (1,0)+(3,0); dark px in the F2 rows 293/159/49.
8. OK — tests/evidence: red-before log shows (a)(b)(c) failing on `no run "Lorem ipsum"` (runs unsupported-encoding) = right reason; header "8 steps, 0 click
   lines" matches 8 JSONL lines; revisions absolute 0,1,1,1,1,2,3,3; drive test `{ skip: CI_NO_ELECTRON, timeout: 300_000 }`; refusals deep-equalled.
   nit, open (owner orchestrator): `coded (a)` checks bounds by shape only (`ok(length 4)`); the numbers are in item 5's ledger (72,104,234.15,140.4 etc.).
9. nit, open — Help text (index.html:527) and design-agent-drive.md say "LibreOffice and Word exports"; only a LibreOffice export was probed — "Word" is unverified.
   The sentence sits inside the existing cell; p2-shell green. `#content-repertoire` is filled from `item.repertoire` (renderer.ts:4639-4642), which the scan sets.
10. nit, open — scope effect not in the contract: clip-copy/clip-paste (engine.ts:3597 gates on `candidate`) now accept coded runs. probe8.mjs: copy+paste OK,
   pasted run extracts `Dolor`, rescans candidate with its own repertoire `orDl` (grafted font copy). Behaviour is sound; no test pins it.
11. OK — shot-coded.mjs: `node --check` OK; frontmost test inside the same osascript call, process addressed by `unix id`; SIGTERM + `shasum -c fixtures.sha256`
   in `finally`; PNG at checks/review-out/coded-edited.png; STATE keys it reads (content.candidates/edited/refused, revision, dirty) exist (renderer.ts:5077-5082).
   nit: unused copied helpers (near, drag, rightClick, pagePoint). Not run (orchestrator).

Files changed by the reviewer: src/renderer/content-edit.ts (editMeasureCid, 3 lines), tests/p3-coded.test.mjs (appended test `coded (g)`).
Fixture unchanged (8f6b3bfa…); coded-hash-before diff empty (56 lines); fixtures.sha256 9 OK; npx tsc --noEmit OK.
Tests (under lock, after the fix): npm run build OK; node --test p3-coded, p3-coded-drive, p3-targets, p3-edit, p3-replace, p2-shell → tests 97, pass 97, fail 0,
skipped 0. Drive tests/drive/coded-edit.jsonl → exit 0, 8 steps, 0 red. Sample page-2 drive → exit 0, 5 steps, 0 red.
