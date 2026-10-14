# Embedded outline inventory and rendering gap

This is read-only evidence for follow-on planning. No font parser, glyph selector, outline interpreter or rendering dependency was added. `tools/embedded-outline-inventory.mjs` rechecks existing fixture SHA256 values, reads font dictionaries and decoded streams through pinned MuPDF 1.28.1 WASM, then inspects bounded sfnt table/loca/glyph headers. Exact fixture/program hashes, native dictionary strings and table offsets are in `reference/embedded-outline-inventory.json`. The run produced no native diagnostics.

All five page-bound simple fonts with explicit Widths embed **TrueType glyf/loca** through FontFile2. None contains CFF/CFF2 or variable-font tables. The variable-looking name in simple-subset.pdf is not evidence of an active variable font.

| Fixture and font object | Program bytes | Glyph slots | Nonempty simple glyphs | Composite glyphs | Font cmap subtables |
| --- | ---: | ---: | ---: | ---: | --- |
| p0b/edit-coded.pdf,5:0 | 6,312 | 470 | 13 | 0 | platform 1/encoding 0 format 4; platform 3/encoding 0 format 4 |
| p3/simple-subset.pdf,3:0 | 2,630 | 5 | 5 | 0 | platform 3/encoding 1 format 4 |
| p6/quartz.pdf,6:0 | 9,028 | 470 | 40 | 0 | platform 1/encoding 0 format 4; platform 3/encoding 0 format 4 |
| p6/quartz.pdf,10:0 | 5,228 | 470 | 2 | 0 | platform 1/encoding 0 format 4; platform 3/encoding 0 format 4 |
| p6/quartz.pdf,14:0 | 6,260 | 470 | 12 | 0 | platform 1/encoding 0 format 4; platform 3/encoding 0 format 4 |

Together these programs contain 1,885 glyph slots: 72 nonempty simple outlines and 1,813 empty slots. All use 1000 units per em and short loca offsets. The largest simple-glyph contour count seen in headers is 3. All contain a prep table; this inventory does not execute instructions or establish which glyphs contain hint instructions. Only simple-subset.pdf has a post table (version 2); the others lack it.

Three fonts have PDF Flags 4 (symbolic) and no explicit Encoding: edit-coded5:0 and quartz6:0/10:0. The remaining fonts have Flags 32 and explicit WinAnsi or MacRoman encoding. Thus “all are TrueType” does not resolve how a PDF source byte selects a glyph. In particular the platform 3/encoding 0 subtables are symbolic charmaps, and four programs have no post glyph-name table. A future code-to-GID resolver must independently validate the relevant PDF encoding/flags/charmap rules. ToUnicode's byte→Unicode mapping must never be inverted or substituted as proof of glyph identity.

## What the app actually consumes

`src/renderer/engine.ts:1148` renders page pixels through `page.toPixmap`, returning RGBA plus page and pixel dimensions. It also returns `toStructuredText("preserve-spans")` text and parsed lines from the same page. Its thumbnail path at 1198 needs pixels at a separately chosen scale. The existing app accepts page zoom 0.25–4 and a 4096-pixel edge ceiling. Redaction verification also renders page pixels; inaccurate text coverage is therefore more than a cosmetic preview issue. Editing code separately calls MuPDF font character encoding and glyph advances when inserting text.

Replacing that surface requires several independent capabilities beyond M0/M1 widths and T0 positions:

1. A source-code→glyph-ID decision with explicit provenance, independently of Unicode extraction.
2. Bounded font program decoding into real contours, including quadratic curves, implied on-curve points, contour closure and eventually composite glyph transforms/cycles. Font units must remain tagged and convert through unitsPerEm and the text rendering matrix.
3. A curve-capable raster path with appropriate fill rules, clipping, paint order, antialiasing and eventually the other text rendering modes. Current R0 only accepts integer orthogonal geometry and cannot render these TrueType contours.
4. A stated hinting policy and separate pixel acceptance at small sizes; unhinted outline correctness does not imply MuPDF's hinted pixel parity.
5. Resource/Form execution and font selection in the actual page, plus a distinct structured-text layer for lines, selection and reading order. Per-code placement is not that structure.

The 48 page-bound simple fonts without explicit Widths use Helvetica (47) and Times-Roman (1). Our M1 substitute metrics came from pinned bundled URW **CFF** programs. A TrueType-only outline stage would address these five embedded programs, but would not supply those commonly used builtin outlines. CFF Type2 interpretation or another explicitly approved matching outline source remains a separate future decision.

## Smallest defensible outline experiment

A useful next standalone unit could accept validated sfnt bytes plus an explicit glyph ID and return bounded, immutable **unhinted simple TrueType contours** in font units. The observed 72 nonempty glyphs offer a concrete native-outline comparison corpus without needing composite support first. Empty glyphs must remain explicit empty results; composite glyphs and unsupported formats must fail explicitly. Synthetic tests must cover malformed tables, short/long loca, repeated flags, delta coordinate decoding, off-curve endpoints, instruction-length skipping and every point/contour/work limit. A caller-provided glyph ID is a test input, not a PDF glyph-resolution claim.

Keep source-code→GID mapping, composite expansion, hint execution and raster curve support outside that first unit until each has its own bounded design and oracle. No inference about complete PDF text rendering follows merely from recognizing glyf in these five files.
