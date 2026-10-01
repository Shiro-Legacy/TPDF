# P5 U3 fixture — evidence (fixture seat, opus55, 2026-09-24)

- `fixtures/p5/empty-sigfield.pdf`: 1581 bytes, sha256 `07e46851060fc7aa179c0f89a7d08ebdac26f682f4322edf20c57dad0523a206` (= §9.37). Run twice from `/tmp` → identical sha. One `%%EOF`, no `/Prev`, no `/V`, no JS/OpenAction (asserted in the generator).
- Generator `fixtures/p5/make-empty-sigfield.py` (paths via `pathlib.Path(__file__).resolve().parents[1]`, objects byte-identical to `.quad/shared/p5/make-empty-sigfield.py`); also writes `fixtures/p5/empty-sigfield.provenance.json` (links.provenance.json schema, pages 2, generator sha recorded).
- Page 0 is 612x792, unrotated; page 1 has `/Rotate 90` (shown 792x612; drive/click space there is mupdf page space).

| field | obj | page | /Rect y-up | page-space (engine `fields`) | y-down click centre | /AP |
|---|---|---|---|---|---|---|
| Signature1 | 7 | 0 | [72,112,340,172] | [72,620,340,680] | (206, 650) | none |
| Signature2 | 8 | 0 | [360,112,540,172] | [360,620,540,680] | (450, 650) | obj 9 grey box, BBox 180x60 |
| Signature3 | 10 | 1 (/Rotate 90) | [72,500,300,560] | [500,72,560,300] | page-space centre (530, 186) | none |

- /Annots: page 0 `[7 0 R 8 0 R]` (Signature1, Signature2); page 1 `[10 0 R]` (Signature3). /AcroForm /Fields `[7 0 R 8 0 R 10 0 R]`, `/SigFlags 1`, /DR /Helv = obj 3 (Helvetica, Base-14, not embedded). Both pages share content stream obj 5.
- Engine proof (`node .quad/scratch/p5u3/fixture/prove.mjs`, headless against build/renderer/engine.js, no lock): `open` ok, pageCount 2, rotations [0,90], info.signatures 0, permissions.edit true, revision 0 (the engine reply has no `readOnly`; with 0 signatures the renderer's readOnly is false). `fields` page 0 → items id 7 Signature1 kind signature rect [72,620,340,680], id 8 Signature2 [360,620,540,680]; page 1 → id 10 Signature3 [500,72,560,300]. `sign-list` → 3 entries, all `empty:true` (note: sign-list `page` is 1-based: 1,1,2). ALL ASSERTS PASS.
- Pins: `tests/fixtures.test.mjs` PINNED_SHA256 `'p5/empty-sigfield.pdf'`; `.quad/shared/p6/checks/fixtures.sha256` now 7 lines, `shasum -a 256 -c` all OK; `fixtures/p5/README.md` regen line + table row.
- Hashes: all fixtures/**/*.pdf|png vs `.quad/shared/p5/checks/u3-hash-before.txt` (52) → only `fixtures/p5/empty-sigfield.pdf` added (53); nothing else changed.
- Under the lock: `node --test tests/fixtures.test.mjs tests/p1-worker.test.mjs` → 31 tests, 31 pass, 0 fail (no build: no source changed).
- NOT scanned: tests/p1-worker.test.mjs provenance scan does `readdir('fixtures')` (top level only), so `fixtures/p5/empty-sigfield.provenance.json` is not picked up; the sha pin in fixtures.test.mjs covers the bytes.
- Commands: `python3 fixtures/p5/make-empty-sigfield.py`; `shasum -a 256 fixtures/p5/empty-sigfield.pdf`; `node .quad/scratch/p5u3/fixture/prove.mjs`; `shasum -a 256 -c .quad/shared/p6/checks/fixtures.sha256`.
