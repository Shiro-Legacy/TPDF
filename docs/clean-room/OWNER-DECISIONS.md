# Clean-room rewrite: decisions for the owner

**Status: the owner accepted every recommendation below on 2026-10-18.**
The recommendation column is now the decision, and it overrides any
"owner may override" default in a spec.

All eight specs are written and leak-checked (2026-10-18). Each spec's
section 10 lists its open questions; this page collects them, with a
recommendation for each.

**Bold** marks the four that change what users see the most.

## Syntax and filters (specs 01, 02)

| # | Question | Recommendation |
|---|---|---|
| S1 | **Dictionary key order: keep insertion order at every size** (saved files stop being byte-identical to MuPDF's) | Yes |
| S2 | **Real numbers: write the shortest decimal that reads back exactly** (no precision loss on load/save) | Yes |
| S3 | Write /Type then /Subtype first in stream dictionaries too | Yes, same rule everywhere |
| S4 | Lenient decoding accepts a missing end marker in ASCIIHex/ASCII85/RunLength/LZW | Yes, in the lenient (viewing) path only |
| S5 | Specs may quote the engine's own limits (64, 100,000, 16 MiB) | Yes, they are ours, not MuPDF's |
| S6 | Filter details: what "non-default predictor" means; zlib preset dictionary; exact Flate error offsets | Predictor ≠ 1; refuse preset dictionaries (PDF never defines one); offsets need not be exact |

## Repair (spec 03)

| # | Question | Recommendation |
|---|---|---|
| R1 | A wrong `/Length` on an intact xref no longer counts as damage (so such files can be signed again) | Yes |
| R2 | A page-tree entry that points at a missing object: drop it, or show a blank page | Drop it (today's behaviour) |
| R3 | A page entry of the wrong type: repair drops it, the normal walk shows it blank | Keep as is for now; unify later |
| R4 | Build a catalog when none exists, so surviving pages open | Yes |
| R5 | Download `pdfjs-dist` (npm) into scratch as a second reference viewer | Yes |
| R6 | Default repair limits become the app's limits | Yes |
| R7 | Re-derive the `startxref` search distance and junk skip (inherited from MuPDF-informed code) before hand-off | Yes, re-derive |
| R8 | Also drop dangling entries from `/Fields`, `/OCGs`, `/Order`, `/ON`, `/OFF` | Yes |
| R9 | What counts as a "damaged keyword"; how cut-short objects are counted | Implementer's choice |

## Text extraction and Unicode (specs 04, 05)

| # | Question | Recommendation |
|---|---|---|
| T1 | Patch the vendored hayro to report each glyph's code, font and render mode (a second kind of patch besides panic fixes) | Yes; it removes the most fragile part of the design |
| T2 | **Right-to-left text reads in logical order (Unicode bidi)**, even though the corpus will count those pages as "different from MuPDF" | Yes |
| T3 | Make search case-insensitive | Not in this rewrite; a separate product change |
| T4 | A CropBox outside the MediaBox shows the MediaBox instead of a blank page | Yes |
| T5 | Panel text joins runs on one visual line (some corpus pages become "equal ignoring spaces") | Yes |
| T6 | Add the `unicode-bidi` crate (MIT/Apache) | Yes, instead of generating bidi tables |
| T7 | Merge overprinted copies offset by up to 0.05 em (fake bold) | Yes |
| T8 | Re-design a clean rule for old GBK Chinese fonts (two pdf.js corpus files) | Yes, as a small follow-up with its own experiment |
| T9 | Add the TeX glyph list | No for now (its licence needs checking) |
| T10 | Use Adobe's `symbol.txt` / `zdingbat.txt` (free to copy with notice) | Yes, keep the notice |
| T11 | Symbolic TrueType unnamed codes: implementer option tuned on corpus, or require U+FFFD | Implementer option |
| T12 | Invert the embedded TrueType cmap for CID fonts without ToUnicode | Yes |
| T13 | Search quad gap and "parallel" tolerance; Symbol/Dingbats aliases; reliance on hayro's code split | Implementer's choice; hayro reliance is acceptable |

## Annotations, form fields, images (specs 06, 07, 08)

| # | Question | Recommendation |
|---|---|---|
| A1 | **New icons, caret, line endings and stamp artwork** (all MuPDF artwork is dropped) | Yes, required for the clean room |
| A2 | Stamp label face: Helvetica-Bold or Times-Bold | Helvetica-Bold |
| A3 | Caret with /Sy /P draws a paragraph mark | No |
| A4 | Standard-font widths: neutral copy of the URW AFMs, or Adobe's Core 14 AFMs | Adobe Core 14 AFMs (no MuPDF tarball involved; add their notice) |
| A5 | Draw new FileAttachment and Sound icons (today they draw nothing) | Yes |
| A6 | Border width 0 means Line, Ink and PolyLine draw nothing; an Ink curve must stay inside /Rect | Yes to both |
| W1 | Greek, Cyrillic and extended-Latin field text embeds font subsets (as the scanner does) so it renders in every viewer | Yes |
| W2 | CJK /BaseFont uses Adobe's names (e.g. `STSong-Light`) | Yes |
| W3 | Multiline auto size: cap 12 pt, minimum 4 pt | Yes |
| W4 | Push-button down caption uses /MK /AC | Yes |
| W5 | Cyrillic/Greek code layouts: keep KOI8-U and ISO 8859-7, or implementer picks wider ones (e.g. Windows-1251) | Implementer picks wider ones |
| J1 | Treat image resolutions below 10 dpi as unknown | Yes |
| J2 | Read resolution from XMP | No |

## Before hand-off (no decision needed; for the record)

1. Scrub MuPDF-citing comments from surviving code the implementers read
   (each spec's scrub list, e.g. `incremental.rs`, `app/document.rs`, test
   names and comments).
2. Rewrite the tests that assert MuPDF output (byte-equal streams, MuPDF
   artwork and labels, filter vectors) on the reviewer side.
3. Build the hand-off tree: no git history, with `engine/oracle/`,
   `engine/design/`, the replaced files and their MuPDF-produced evidence
   in `engine/reference/` removed.
4. Trim `font_metrics.rs` to the `FontAdvance` type (workstream 5).
