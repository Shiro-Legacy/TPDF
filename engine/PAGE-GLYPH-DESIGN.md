# Page glyph placement (R18 G1)

This capability joins PP1 document-page interpretation with G0 embedded glyph
selection and O0 unhinted simple outlines. It retains the owned PageProgram and
attaches one glyph record to each positioned source code. It does not rasterize
text, infer reading order or relax existing page/font/operator refusals.

## Coordinates and native distinction

For each doubled O0 point `(x2,y2)`, form em coordinates
`((x2/2 + lsb - header.xMin)/unitsPerEm, (y2/2)/unitsPerEm)` and apply the existing
CodePlacement em-to-page transform exactly once. This includes font size,
horizontal scale, text matrix, rise, CTM and page placement in their existing
order. PDF Widths control advancement between glyphs; hmtx.advanceWidth never
replaces PDF width or scales an outline to match it.

The origin shift matters: the standalone O0 comparison restores FreeType's
loaded coordinates back to the raw glyf coordinates. Actual FreeType drawing
subtracts the first phantom point, `xMin-lsb`, independent of head.flags bit1.
All five original embedded fixture fonts happen to have zero shift. Generated
and independent tests therefore require nonzero shifts, including xMin100 and
lsb-40, differing font/PDF advances, and units16/1000/2048/16384.

`truetype_metrics::HorizontalMetrics` borrows validated font tables, allocates
nothing and validates hhea1.0, reserved/format words, numberOfHMetrics, complete
hmtx coverage and at most three zero padding bytes. It handles trailing LSB-only
records using the last full advance, and rechecks each GID range. Its metadata
and queries have a separate fixed-step accounting partition in the page session.
It does not change earlier raw outline behavior or introduce hinting.

The primary native comparison uses explicitly unhinted, unscaled FreeType
coordinates at their actual drawing origin, transformed to page space. Direct
MuPDF fz_outline_glyph results are recorded separately: that API uses a scaled,
hinted load and may differ in rounding/topology. Differences cannot be hidden by
restoring the raw origin or widening the primary comparison tolerance. Pixel and
hinted rendering equality are not claims of this milestone.

## Identity, ownership and caching

`PageGlyphSession` is bound to one Document and wraps the cumulative PP1 and G0
loaders. Only fonts of actual source-code events need outlines; PP1 continues to
validate every Tf even if it never shows text. Font identity comes from the
page's prepared font registry, not an unrelated caller-provided map. Direct font
dictionaries and unsupported font programs remain explicit refusals.

Each attachment retains code index, global event index, text context ID,
requested font/program object identity, source byte, resolved GID, lookup route,
selected cmap/ranges, unitsPerEm, side bearing/origin shift, metrics table ranges
and a range in the transformed-command arena. The retained PageProgram supplies
all source spans, stream-occurrence projections, extracted Unicode, text state,
colors and clip ancestry. These contexts remain separate from glyph selection.

A session caches loaded programs and validated code maps, and shares immutable
raw GlyphOutline values with Arc by font identity/GID. Aliased resource names of
the same indirect font share cached outlines. A new glyph batch reopens that
font under the opening budget; all-cache-hit pages do not reopen, remap or decode.
Prior successful outputs survive later failures and dropping the entire input
Document/session. Shared outlines are immutable and can cross thread boundaries.

The complete page result is returned only after all attachments/transforms pass.
A later unsupported source code publishes no prefix. Earlier cached work remains
valid and charged, and another page can still be attempted under remaining caps.
Missing GID0, unsupported name fallback and valid empty nonzero outlines retain
their distinct meanings. Invisible Tr3 glyph geometry is inspectable but retains
its invisible intent; only Tr0 and Tr3 are accepted by the existing text engine.

## Budget partitions

PageProgramSession and GlyphFontLoader keep their own live cumulative ledgers.
R18 additionally caps cache entries, occurrences, sfnt index allocation/work,
cmap directory/selection/segments/validation/queries/work, O0 decoding/points/
contours/commands/allocation/work, transformed points/commands, orchestration
searches, metrics inspection and full allocation requests for its arenas/handles.

Before calling an API that cannot return failed work usage, the wrapper reserves
its capped remaining allowance; only success refunds to measured usage. Actual
O0 decoder usage is retained even on failure. Font mapping and later decode-batch
opens are charged separately. Cache and lookup scans are counted so repeated
pages and multiple cached fonts cannot bypass work limits. Metrics validation
admits its bounded directory scans and constant metadata work; each metrics query
reads two words. Fixed256-slot G0 name policy remains a separately documented
constant-sized operation, not part of the low-level cmap work counter.

Arc payload/header requests and vector growth are admitted before allocation;
the underlying outlines keep their own O0 allocation ledger. These are logical
resource partitions and requested capacities, not a universal allocator/RSS
ceiling. Arc construction follows Rust's process-OOM behavior. The prior Document
parse and native test-only allocations are separate prerequisites.

## Inspection and acceptance

`--page-glyph=N FILE.pdf` inspects one-based page N and fails atomically.
`--page-glyphs FILE.pdf` records accepted/refused pages in one cumulative session.
Both keep the existing16MiB CLI input and64MiB atomic output limits. Successful
JSON contains the full PP1 trace, glyph attachment provenance and page-space
Move/Line/Quad/Close commands. This is not a pixel renderer.

Acceptance requires independent Rust adversarial tests; exact GID/order/command
identity and independently calculated geometry; native drawing-origin comparison
on eligible actual pages and generated transforms; explicit ineligible-page
counts; injected comparator faults; full normal/MSRV tests, formatting, strict
Clippy and minimum-toolchain wasm library compilation. Final results and exact
reproduction commands belong in STATUS.md and REVIEW-R18-G1.md.

References:
- https://learn.microsoft.com/en-us/typography/opentype/spec/hmtx
- https://learn.microsoft.com/en-us/typography/opentype/spec/hhea
- Pinned FreeType ttgload.c: tt_loader_set_pp and TT_Load_Glyph origin translation.
- Pinned MuPDF source/fitz/font.c: fz_outline_ft_glyph.

No production application replacement or native runtime dependency is introduced.
The full engine replacement remains governed by REWRITE-ROADMAP.md; accepting
this stage does not complete that broader task. Automation remains paused.
