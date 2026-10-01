# P6 U6 — fixture seat evidence (Opus 5.5, 2026-09-24)
Files (new): fixtures/p6/make-objects6.py (sha256 38afb712…79bb5), fixtures/p6/objects6.pdf, fixtures/p6/objects6.provenance.json. In place: tests/fixtures.test.mjs:16 `'p6/objects6.pdf'` pin (after p6/links); .quad/shared/p6/checks/fixtures.sha256 line 9 appended.
objects6.pdf: **2372 bytes, sha256 ca0a5988eb81cd6b35916918695cd8c9881b8ebba82ae590e38cdf8a97fb2ab8**, 2 pages 612×792, one %%EOF, no /Prev. Deterministic: `python3 fixtures/p6/make-objects6.py` run twice → identical sha256. Images DeviceGray 8-bit, all samples 0x80, no filter/mask: Im1 4×2, Im2 2×4, Im3 3×2, Im4 5×2, Im5 2×2, page-1 Im1 6×3 (obj 14). Fm1 /BBox [0 0 300 200], no /Matrix, `0.9 g 0 0 300 200 re f`.
Page 0 stream (exact): `q 100 0 0 50 72 600 cm /Im1 Do Q` / `q 0 -100 50 0 400 750 cm /Im2 Do Q` / `q -100 0 0 50 300 600 cm /Im3 Do Q` / `q 72 400 60 40 re W n 100 0 0 50 72 400 cm /Im4 Do Q` / `q 1 0 0 1 72 100 cm /Fm1 Do Q` / `q 50 0 0 50 300 120 cm /Im5 Do Q` / `BT /F1 14 Tf 100 250 Td (Over) Tj ET` (one per line, `\n` after each). Page 1: `q 100 0 0 50 72 600 cm /Im1 Do Q`.

Boxes, y-up → y-down (view = [x0, 792−y1, x1, 792−y0]):
- Im1 [72,600,172,650] → [72,142,172,192]; Im2 (rot cw) [400,650,450,750] → [400,42,450,142]; Im3 (flip) [200,600,300,650] → [200,142,300,192]
- Im4 visible [72,400,132,440] → [72,352,132,392]; full [72,400,172,450] → [72,342,172,392]
- Fm1 [72,100,372,300] → [72,492,372,692]; Im5 [300,120,350,170] → [300,622,350,672]; page-1 Im1 [72,600,172,650] → [72,142,172,192]
Click centres (y-down): Im1 (122,167), Im2 (425,92), Im3 (250,167), Im4 (102,372), Im5 (325,647), Fm1 band (74,600), Fm1 interior empty (200,600), "Over" (115,537), page empty (500,450).
"Over" bounds: engine edit-targets `bounds` (y-down) [100,526.9500122070312,130.33799743652344,546.1859741210938] → y-up [100,245.814,130.338,265.05]; mupdf stext line bbox (JSON, integer) y-down [100,526,130,545] → y-up [100,247,130,266]. Origin y-up (100,250), y-down (100,542).

Today's engine reply (build/ as of U5, before the engine seat's rule), `edit-targets` page 0, verbatim minus `image` flags (all false):
- Im1 p0.c0.28.178329c1 support candidate, bounds [72,600,172,650], placement {box:[72,600,172,650]}, objectOps [move,resize,delete,replace,duplicate], image 4×2
- Im2 p0.c0.63.14832508 support candidate, reason unsupported-transform, bounds null, no placement, objectOps [replace], image 2×4
- Im3 p0.c0.98.1583269b same as Im2 (unsupported-transform, [replace]), image 3×2
- Im4 p0.c0.151.1a832e7a same (unsupported-transform, [replace]), image 5×2
- Fm1 p0.c0.181.93b5f78c support candidate, bounds [72,100,372,300], placement {box:[72,100,372,300]}, objectOps [move,resize,delete,duplicate]
- Im5 p0.c0.214.1b83300d support candidate, bounds [300,120,350,170], placement {box:[300,120,350,170]}, objectOps [move,resize,delete,replace,duplicate], image 2×2
- text p0.c0.250.02b4b43f "Over" Helvetica 14, candidate, moveable true, tailFollows false
Page 1: Im1 p1.c0.28.178329c1 candidate, bounds [72,600,172,650], placement {box:[72,600,172,650]}, objectOps as page-0 Im1, **image.width 6** (height 3) vs page-0 Im1 width 4.
Render probe (page 0, mupdf toPixmap identity, DeviceGray, 72 dpi, y-down): Im4 visible [72,352,132,392] dark(<200) 2400/2400; cropped-away [132,342,172,392] 0/2000; top strip [72,342,132,352] 0/600; 2 px ring outside visible 0 dark. Fm1 [72,492,372,692] non-white 60000/60000 (57350 px = 229 grey, rest Im5 + "Over"); 12 px strips left/right/above/below Fm1 0 non-white. Samples: (200,600)=229, (102,372)=128, (500,450)=255.

Commands: `python3 fixtures/p6/make-objects6.py` (×2); under the lock `node .quad/scratch/p6u6/fixture/probe.mjs` (output .quad/scratch/p6u6/fixture/probe.out) and a raw `edit-targets` dump via `node -e` against build/renderer/engine.js; `node --test tests/fixtures.test.mjs tests/p1-worker.test.mjs` → 31 pass, 0 fail. `shasum -a 256 -c .quad/shared/p6/checks/fixtures.sha256` → 9 OK. Baseline diff (54 lines) → exactly one added line `ca0a5988…2ab8  fixtures/p6/objects6.pdf`, nothing else.
D4 re-pins: none (fixture seat). Not done: nothing.
