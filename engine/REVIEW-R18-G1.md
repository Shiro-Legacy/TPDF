# R18 G1 independent page-glyph review

Reviewed 2026-10-03. Scope: `page_glyph.rs`, the new `truetype_metrics.rs`, and
their connection to accepted PP1, G0, O0 and typed page/text geometry. Review
changes are confined to `tests/review_page_glyph.rs` and this document; the owner
implemented production changes. The broader completion inventory is
[REWRITE-ROADMAP.md](REWRITE-ROADMAP.md). This subset does not complete the rewrite.

## Geometry and identity

The output owns the entire PP1 page result and attaches a glyph record to each
positioned source-code event. Each record identifies the code/event/context,
requested font dictionary, resolved FontFile2 object, selected cmap record,
source-code lookup choice, unitsPerEm, metrics table ranges and shared immutable
raw O0 outline. The command arena contains page-space moves, lines, quadratics
and closes. Text rendering mode, fill color and clip ancestry remain in the
retained PP1 context; a command is not a claim that an invisible glyph was painted.

The crucial drawing-origin normalization is `(raw_x + lsb - header_xMin) /
unitsPerEm`, with `raw_y / unitsPerEm`, before applying the T0 em-to-page transform
exactly once. O0 doubled coordinates are divided by `2 * unitsPerEm`, with the
bearing shift doubled too. This differs from comparing O0's deliberately restored
raw glyf coordinates to an equally restored native oracle. The page oracle must
compare actual native drawing-origin coordinates. PDF Widths control text
advancement independently of hmtx advance; neither width source rescales outlines.

The bounded horizontal metrics reader handles both long metrics and trailing
signed LSB entries, with the last long advance reused for later glyphs. It checks
hhea version, count, reserved fields, bounded table lengths/padding and font-local
GID bounds. It retains absolute table ranges. Its exposed advance is deliberately
not the page advance. Empty nonzero glyphs retain their code identity and empty
command range; missing GID0 and unsupported mapping are whole-page refusals.

Unicode is independent of drawing identity. A source A can extract BC and still
produce exactly one GID1 outline. Source/string spans are decoded-string arena
offsets, not encoded content byte spans. `operation_span` is the logical content
range that PP1 projects across source streams and synthetic joining padding.
No exact per-hex-digit or escaped-byte source location is claimed.

## Ownership, caches and limits

Caches are scoped to one borrowed document and keyed by the requested font
object, rather than only the shared FontFile2 object. This preserves different
encoding policies. Aliased resource names referencing the same font share the map
and each decoded GID. The retained source bytes are immutable; font reopening
uses those bytes and rechecks GID bounds. Returned results own PP1 and Arc outline
references and remain valid after the document, input bytes and session are dropped.

A cache hit skips new font/cmap/outline decoding but still charges occurrence,
lookup, transformed command/point and output allocation work. The review identified
linear font/glyph cache searches as work outside the initial phase counters; the
owner added a cumulative `lookup_work` admission counter for these comparisons,
code/font selection scans and output event iteration. Metrics validation has its
own conservative bounded scan allowance and constant query charges. Opening,
cmap, O0 decoding, G0 loading and PP1 remain distinct budget partitions.

Failed work retains conservative phase reservations when the callee cannot return
actual usage. Successful calls refund only the unused portion. Cache-slot counts
may conservatively include failed reservations. A failed output can leave useful
immutable cache entries, but it cannot publish a partial PageGlyphProgram or
invalidate an earlier result. Per-phase and per-page failures remain bounded on
retry. The fixed 256-slot needed array and its scans additionally receive a conservative
512-unit lookup reservation per cached-font batch. The separate G0 map policy
retains its documented fixed 256-slot name-lookup bound; these partitioned counters
are not a single CPU instruction count.

Allocation accounting covers full requested buffer capacities and Arc payload/
header requests in addition to the independently charged underlying O0 arenas.
It does not promise a universal process RSS bound or recoverable allocator OOM;
Arc allocation follows the documented ordinary Rust allocation behavior.

## Independent regression evidence

`cargo test --test review_page_glyph --target-dir
/private/tmp/tandem-r18-review-target` passed all **13 tests**. The reviewer built
tiny sfnt and PDF fixtures independently rather than importing the owner's test
builders. These cover:

- Nonzero raw xMin=100 versus lsb=-40, hmtx advance900 versus PDF width250, and
  unitsPerEm16,1000,2048,16384. Raw outline coordinates remain unchanged.
- A hand-calculated noncommuting CTM/text matrix/horizontal scale/rise with crop,
  rotation90 and UserUnit2. Move and quadratic coordinates match the independent
  expected point (204.3,63.15), without reapplying page geometry.
- Resource aliases, repeated page cache reuse and pointer identity while output
  work/allocation continues to accumulate; outputs outlive every borrowed input.
- Valid empty glyphs, invisible text intent and missing-code atomic refusal.
- A successful page, failed second page and successful retry retain the first
  owned output and reusable cache without partial output publication.
- Exact cumulative occurrence, transformed-command, point, metrics and lookup
  limits; one-less boundaries; repeated invalid hhea charges until exhaustion.
- A ToUnicode multi-character result does not change glyph selection; a hex
  string spanning content streams preserves operation-level provenance.
- Two font dictionaries sharing FontFile2 do not share encoding policy: an
  unsupported glyph-name override in the second dictionary correctly refuses.

## Acceptance boundary

Source review and independent targeted tests have no remaining identified blocker
for the narrow embedded simple TrueType placement subset. The final evidence below satisfies the targeted source/native review gate. The
integration owner retains responsibility for recording the complete build gate
set and declaring milestone acceptance.

Retained exclusions include direct font dictionaries for outline loading, Base14
outline supply, composite glyf, unsupported G0 mappings, CFF/Type0/CID and broader
PP1 unsupported page operations. No text rasterization, hinting, pixel equivalence,
complete page support, editing or application integration is established here.

## Final gate evidence

The integration owner reports **594 all-feature tests pass** on the normal and
minimum Rust toolchains, with the wasm library check passing and all-target
Clippy clean. The independent thirteen-test target was rerun successfully after
the final fixed-array lookup reservation change. Final native/model reports were then inspected as summarized below.


`reference/page-glyph-differential-g1.json` attempts all572 readable corpus pages:
**2 matched and570 explicitly refused**, with identical native accepted/refused
classification. The two accepted pages contain5 positioned codes and87 outline
commands; one is an empty content page. This is a substantially narrower glyph
subset than PP1's retained543 accepted traces, not a regression hidden by changing
the trace gate or a general-page rendering result.

The generated gate passes **237 PDFs,2362 codes and32434 outline commands**:
220 exact dyadic mixed-state/nonzero-bearing cases, the reviewer's explicit
xMin100/lsb-40/hmtx900/PDF250 case, and16 pages using the actual embedded subset
font with independent extraction mappings and transforms. There are2202 nonzero
origin-shift occurrences. The dyadic rational model difference is zero; the
1000-units reviewer case explicitly permits1ulp, and the16 real-font cases use
native outline geometry plus independently modeled rational state. Observed
primary-native maximum coordinate difference is0.0000090000000057 under the
unchanged T0/P0 tolerances. Identity, GID, source, Unicode, event/clip order, units,
bearing, table ranges and command topology remain exact checks.

`reference/page-glyph-reference-validation-g1.json` passes3 positive comparator
checks, rejects29 independently injected mutations, and passes10 native wrapper
checks. It detects altered origin normalization, identity/provenance, command
coordinates/types/order, clip ancestry and global path/text order. Both reports
identify Rust binary SHA256
`fcd48042be619fa1200e48db98ea5844ff0109b98f045b2e33c57f871a78174f`.

Native hinted outlines are recorded separately and differ in topology and some
coordinates; they are explicitly excluded from the primary unhinted gate. The
corpus has3 hinted topology differences; generated cases have1524. No hinting or
pixel parity is inferred. Initial page bounds and clip topology are metadata in
this milestone, not applied raster coverage. These limitations and the570 corpus
refusals remain visible. With these bounds, the independent review has no remaining
blocker for accepting R18 page glyph placement.
