# App edit ops (content editing), stage A

October 15, 2026. `src/app/ops/edit.rs` and `src/app/ops/edit/` port engine.ts's `edit-targets`, `edit-text`, `edit-paragraph`, `text-add`, `edit-replace-preview` and `edit-replace-apply` (with content-edit.ts) onto `AppSession`. Stage B (`edit-object`, `edit-objects`, `clip-copy`, `clip-paste`, `edit-batch`) and the redaction group build on the same scanner and writer.

## Layout

- `edit.rs`: the six ops, limits, `Budget`, `decode`, the scan cache (`scan_for`), `resolve_target` (engine.ts `resolveEditTarget`), the replace preview.
- `edit/lex.rs`: the content tokenizer with byte spans (content-edit.ts `nextToken`), `next_q_end`. The crate's `content::ContentParser` keeps one span per operation, not per operand, and reads inline images and malformed input by its own stricter rules, so the scanner has its own tokenizer with engine.ts's rules.
- `edit/scan.rs`: `PageScan`, `Record` and its parts, ids, the structured-text association (`runs_of`, `match_runs`), growth limits, move rules, struct-tree lookup.
- `edit/walk.rs`: `scan_page` (`scanEditTargets`, `walkEditContent`): the bounded walk that builds the records.
- `edit/font.rs`: font facts (`editResolveFont`), the `Codec` of a simple font, proven code maps (`Cid`: Identity-H and coded), /ToUnicode and /W parsers, repertoires, WinAnsi tables.
- `edit/glyph.rs`: Quartz glyph blocks and their collapse into glyph lines.
- `edit/paragraph.rs`: paragraph discovery and the paragraph plan.
- `edit/plan.rs`: text plans (`editPlanText`, geometry, width rules, glyph-line plan), number and operand writers, `Measure`.
- `edit/write.rs`: `splice`, `privatise` (copy-on-write), `write_element`, `write_plans`, text-add's write.
- `edit/geometry.rs`: matrices, `edit_number` (`editNumber`), `round2`, JavaScript `Math.round`.
- `view/measure.rs`: `ViewDocument::page_transform`, `measurer` (codes measured with a page's fonts by hayro) and `decoded_stream` (hayro's tolerant decode of one stream).
- Shared files: `ops/mod.rs` routes the six ops; `AppDocument` gains the private scan cache (`edit_scan()`) and `view_with_document()`; `view/mod.rs` declares `measure`.

## Scanner

- One bounded pass per page, two walks as engine.ts does (a count-only walk refuses a page past 512 targets before structured text is read). Budgets: 2 MiB of decoded streams (CMaps and copy-on-write copies included), 128 streams, Form depth 16, nesting 32, 200k tokens, 4096 raw and 512 final targets, 4096 UTF-16 units per text and 65536 per reply, 256 pending operands.
- A `Record` holds the operator and operand spans of its stream, the stream object, the path (`c<i>` or `c<i>/<Name>@<Do offset>`), the text and font facts, the text-space state at the show (Tf, Tc, Tw, Tz, Tr, TL, the linear part of Tm x CTM x page transform, the origin), tail and paragraph links, copy-on-write needs, and for an image or Form the placement spans (`q`, the clip's `re` operands, `cm`), boxes and orientation. Items in the reply are built from records.
- Ids are engine.ts's: `p<page>.<path>.<operator offset>.<fnv1a32(op|payload)>`, `Do|<name>`, `GL|<key>`; paragraphs `para.<first id>`.
- `scan_for` caches one scan per document, keyed on generation, revision, page and the document's content state (every write, undo, redo and abandon changes it), so a batch's nested writes never see a stale scan. `edit-targets` always scans afresh.
- Association and bounds come from the view's preserve-spans structured text (hayro), with quads and origins in single precision as MuPDF reports them.
- `PageScan::streams` keeps the decoded bytes of every walked stream; writers splice those when the strict decoder cannot read a stream the view reads.

## Measurement

- engine.ts measured a simple-font run with MuPDF's font handles from the structured text (`encodeCharacter`, `advanceGlyph`). Here the codes are shown with the run's own font (page or Form resources) through hayro's interpreter (`Measurer`): the advance is what the view draws (/Widths, else the program or the standard metrics) and glyph 0 means the font cannot draw the character. One interpreter cache serves a whole preview.
- Widths are page points: `x-scale x Th x (|Tf| x (sum(w)/1000 - TJ/1000) + Tc x n + Tw x spaces)`, x-scale being the length of the text x axis in page space. Identity-H and coded runs use their parsed /W or /Widths the same way.

## Writing

- Plans are complete before an operation opens; the write is one `mutate` step (`edit text`, `move text`, `edit paragraph`, `add text`, `replace text`).
- `write_element` privatises an element at most once per operation (`privatise`: inherited /Resources copied onto the page, a shared element copied with its raw bytes and filters, a depth-1 Form copied and registered as `<Name>c<n>` with this `Do` renamed), then splices it once with every plan and span (`splice` refuses overlaps), then writes StructElem /ActualText. Content is stored decoded; the app writer compresses it on save.
- For stage B and redaction: `write_element(tx, scan, page, element, plans, spans, privatised, budget)` takes raw `SpanEdit`s (a placement's `cm`, clip or `q..Q` span, a run's operand) next to text plans; `Record::placement` and `Record::object_ops` carry what edit-object needs; `plan::plan_text` with `Geometry` is edit-batch's text move; `Measure::glyphs` gives per-code advances for splitting a run at glyph boundaries; `PageScan::streams` has the bytes the spans index; `scan::runs_of` and `scan::match_runs` give a run's structured-text characters (quads) for glyph-level regions.

## Measured

- `tools/compare-app-edit.mjs` runs 15 edits from the app tests (plain, scaled, moved and resized runs, TJ, Identity-H, coded, shared contents, a Form, a tagged run, paragraphs with and without geometry, Quartz glyph lines edited and moved, text-add, replace) on engine.ts and on Rust, saves both and renders the edited pages with MuPDF: every page has the same content stream bytes and 0 differing pixels (`reference/app-edit-20261015/summary.json`).
- Cost through the app adapter (WASM in node, warm, median of 5; engine.ts on MuPDF in brackets): edit-targets on fixtures/p6/quartz.pdf (glyph lines) 239 [417] ms, p6/paragraph.pdf 2.3 [3.4] ms, large.pdf page 0 3.4 [3.7] ms; a replace preview of `e` on quartz.pdf (20 matches) 4.4 [16] ms.
- App tests on Rust, files using these ops: p3-edit 31/32, p3-targets 21/22, p3-replace 20/20, p3b-content 12/12, p3-coded 9/9, p6-text8 5/5, print-edit 16/17, p6-paragraph 10/12, p6-direct3 6/7, bugfix1 3/4 (2 skipped, Electron); the rest of their failures belong to other op groups (see the stage report).

## Differences from MuPDF and engine.ts

- Simple-font widths come from hayro's view of the font. MuPDF measured `Ä` in an unembedded WinAnsi Helvetica as 333 units (its Unicode lookup lands on another glyph); here it is 667, as drawn. The app test "every refusal token is reachable and none of them writes" expects `ALPHA` -> `BRÄVO` in fixtures/p0b/edit-transformed.pdf to fit; it is 84.02 pt against a 150.7 pt limit from x 72, so it is refused `overflow` like `BRAVO`.
- Character and word spacing count in text space through the text matrix (engine.ts added them as page points), and Identity-H and coded widths go through the text matrix (engine.ts used the Tf operand alone, 12 times too small under `12 0 0 12 Tm /F 1 Tf`).
- A run's old width counts a glyph the font lacks at its drawn advance; engine.ts counted it as 0.
- A simple-font run is measured without finding it in the structured text first; engine.ts refused `unmeasurable-width` when its text match failed (a run it could not associate is still not a candidate, so this only matters for a candidate whose line the structured text splits differently).
- `/MacRomanEncoding` and `/StandardEncoding` fonts decode and write through their own tables; engine.ts read them as WinAnsi and wrote printable ASCII only. Fonts without /Encoding keep engine.ts's rule.
- `true`, `false` and `null` are operands: a `BDC` dictionary with a boolean no longer refuses the page, and an inline image's `/IM true` is a 1-bit mask, so an unfiltered mask without /L is skipped instead of refused.
- Vertical writing is the CMap's: an `-V` encoding or a CMap stream with /WMode 1 refuses `vertical-writing`; the descendant's non-standard /WMode still counts, as in engine.ts.
- An inline /ActualText must decode (PDFDocEncoding or UTF-16) to the run's text for every font; engine.ts compared a simple font's raw bytes. The tag is written as printable ASCII or UTF-16BE for every font; engine.ts wrote a simple font's WinAnsi bytes (wrong for 0x80-0x9F as PDFDocEncoding).
- The ParentTree is read as a number tree (/Kids within /Limits); engine.ts read only the top /Nums, leaving a StructElem /ActualText stale.
- Names written into content (`/<Name>c<n>`, a resized run's `Tf`) are `#xx`-escaped from their bytes; engine.ts wrote copy-on-write renames raw and escaped the UTF-8 of a Latin-1 name.
- A Form edited through copy-on-write is copied alone, with its stream's bytes and filters, and a copy of its /Resources dictionary when that is indirect; fonts and images stay shared. MuPDF's graft copied every object the Form reaches (fonts and their programs included).
- A content stream the strict decoder refuses (a bad zlib checksum) is read as the view reads it, and an edit writes it back decoded; the strict path alone would refuse the page.
- A page written directly into /Kids (not an indirect object) refuses edits (`invalid`); a StructElem written directly into the ParentTree array is not updated (`invalid`).
- `text-add` allocates a new font dictionary per add; MuPDF's font cache reused one object per base-14 font.
- Scan cache: one per document, keyed on the content state too; engine.ts kept one per engine and nulled it in clip-paste and edit-batch.

## App tests not passed by design or by the harness

- p3-edit "every refusal token is reachable and none of them writes": the `BRÄVO` width above (MuPDF measurement defect).
- p6-paragraph "§3.4 word-too-wide; ... undo restores": holds a page object of `session.doc` across `undo`; the harness's MuPDF copy is per revision, so the page shows the pre-undo state.
