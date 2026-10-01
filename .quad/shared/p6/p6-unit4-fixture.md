# P6 U4 fixture evidence (Opus 5.5 fixture seat, 2026-09-24)
Files: fixtures/p6/make-links.py, links.pdf, links.provenance.json, make-tabs.py, tabs.pdf, tabs.provenance.json (new); tests/fixtures.test.mjs +2 PINNED_SHA256 lines; .quad/shared/p6/checks/fixtures.sha256 +2 lines (now 6).
- links.pdf 2046 B sha256 `0ac413b0fa9b42062542afbc970f0245bceacab0a789017c90a413733962d0bc`
- tabs.pdf 4796 B sha256 `b90c5db2ed9d4b1221b453bc9df02682fd8a6c678724263738a68d24bc047326`
Deterministic: both generators run twice, identical sha256 (pdf and provenance). Single %%EOF, no /Prev asserted.

## links.pdf (Letter; y_down = 792 - y; page 1 is /Rotate 90)
| link | page | obj (annotId) | kind / target | rect y-up | rect y-down / page space | click centre |
|---|---|---|---|---|---|---|
| L1 | 0 | 10 | uri https://example.org/one | [72 700 200 720] | [72,72,200,92] | (136, 82) |
| L2 | 0 | 11 | page 2 (`/Dest [5 0 R /Fit]`) | [72 650 200 670] | [72,122,200,142] | (136, 132) |
| L5 | 0 | 12 | bare, skipped by getLinks | [72 630 200 640] | [72,152,200,162] | (136, 157) — not a link |
| L3 | 0 | 13 | blocked ftp://example.org/x | [72 600 200 620] | [72,172,200,192] | (136, 182) |
| L4 | 1 | 14 | uri https://example.org/rot | [72 700 200 720] (raw) | page space [700,72,720,200] (engine) | (710, 136) page space |
Page 0 /Annots = [10 L1, 11 L2, 12 L5, 13 L3]; getLinks() = 3 → engine ids 0,1,2 = L1,L2,L3. Page 1 /Annots [14]; page 2 none.
Runs (Helvetica 12, y-up baseline): p0 One (72,704) Two (72,654) Three (72,604); p1 Rotated (72,700); p2 Target (72,700). Empty draw band y-up 100..500 = y-down 292..692.

## tabs.pdf (4 pages, all unrotated; /Tx 120x20, /Ff 0, /DA (/Helv 12 Tf 0 g))
| field | page | obj | rect y-up | rect y-down | centre y-down |
|---|---|---|---|---|---|
| a1/a2/a3 | 0 | 13/14/15 | [72 700/650/600 192 +20] | [72,72/122/172,192,+20] | (132, 82/132/182) |
| b1/b2/b3 | 0 | 16/17/18 | [300 700/650/600 420 +20] | [300,72/122/172,420,+20] | (360, 82/132/182) |
| t1/t2/t3 | 1 | 19/20/21 | x 72/220/368, y 700..720 | y 72..92 | (132/280/428, 82) |
| s1/s2/s3 | 2 | 22/23/24 | x 72/220/368, y 700..720 | y 72..92 | (132/280/428, 82) |
| u1/u2 | 3 | 25/26 | x 72/220, y 700..720 | y 72..92 | (132/280, 82) |
| page | /Tabs | /Annots order | expected fields.order | tabsOrder |
|---|---|---|---|---|
| 0 | C | b3 a1 b1 a2 b2 a3 | a1 a2 a3 b1 b2 b3 | C |
| 1 | A | t3 t1 t2 | t3 t1 t2 | A |
| 2 | S | s1 s2 s3 | s2 s3 s1 | S |
| 3 | S | u2 u1 | u1 u2 | fallback |
Struct: catalog /MarkInfo <</Marked true>>, /StructTreeRoot 27 /K [29 E2, 30 E3, 28 E1] (E1..E3 = objs 28..30, OBJR → s1..s3), /ParentTree /Nums [0 E1 1 E2 2 E3], /ParentTreeNextKey 3; s1/s2/s3 /StructParent 0/1/2; no /StructParents, no /TI anywhere.

## Proof (all green)
- `( lock ) node .quad/scratch/p6u4-fixture-prove.mjs` → engine `links` p0 = ids 0..2 uri/page→2/blocked with the y-down rects above, p1 = 1 link rect [700,72,720,200], p2 = 0; raw mupdf p0 /Annots objs [10,11,12,13], getLinks 3; engine `fields` p0..p3 names in /Annots order with the y-down rects and ids = obj numbers above; page dicts /Tabs C,A,S,S; /K-only DFS of StructTreeRoot via /OBJR → s2, s3, s1. "ALL OK".
- `( lock ) node --test tests/fixtures.test.mjs tests/p1-worker.test.mjs` → 31 pass, 0 fail.
- `shasum -a 256 -c .quad/shared/p6/checks/fixtures.sha256` 6/6 OK; `shasum -a 256 -c .quad/shared/p6/checks/u4-hash-before.txt` 50/50 OK.
Not done / notes: engine `fields` has no `tabs`/`structIndex` yet (engine seat), so /Tabs was checked on the page dicts via mupdf. p1-worker's provenance scan is non-recursive and does not reach fixtures/p6 (as for direct3); the pins in fixtures.test.mjs cover the bytes.
