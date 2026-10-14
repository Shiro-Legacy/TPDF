# Standalone TrueType outline proposal

Status: design only. This does not authorize implementation, add dependencies, or change the engine contract.

## Recommended first milestone

Implement a dependency-free, safe Rust reader of **unhinted simple TrueType glyphs selected by an explicit caller-supplied GID**. It should return exact raw points plus an immutable quadratic outline in font design units. Empty glyphs are successful empty results. A selected composite glyph produces an explicit unsupported error before expansion.

The measured corpus makes this useful: the five embedded FontFile2 programs contain 1,885 slots, of which 1,813 are empty and 72 contain simple outlines; none contains a composite. All use short loca offsets and 1000 units per em. The existing inventory includes original fixture and decoded font hashes in `reference/embedded-outline-inventory.json`; see `EMBEDDED-OUTLINE-INVENTORY.md`. This is a comparison corpus, not evidence that all of those glyphs are used by page content.

Keep code-to-GID selection, Unicode, font metrics, PDF resource loading, hint execution, curve rasterization and composite expansion outside this unit. In particular, accepting a Unicode value or using ToUnicode to select a glyph would conceal a separate unsolved PDF encoding decision. The common builtin Helvetica/Times-Roman outlines are CFF; this TrueType stage does not supply them. R0 cannot rasterize these quadratic outlines.

## Ownership and proposed API

Provisional module name: `truetype_outline`. No public fields that allow callers to forge a validated index or outline. The implementation should remain iterative, contain no unsafe code, and add no runtime dependency.

```rust
pub struct TrueTypeFont<'a> { /* borrowed input; owned table and loca index */ }
pub struct FontLimits { /* input/table/glyph/index limits below */ }
pub struct OutlineLimits { /* session budgets below */ }
pub struct OutlineSession<'font, 'bytes> { /* &TrueTypeFont; monotone usage */ }
pub struct GlyphId(u16); // checked by font.glyph_id(raw)
pub struct GlyphOutline { /* owned immutable points, endpoints, commands */ }
pub struct FontPoint { /* private i32 x/y in integral font units, on_curve */ }
pub struct FontPoint2 { /* private i32 x2/y2: exact half-font-unit positions */ }
pub enum OutlineCommand {
    MoveTo(FontPoint2),
    LineTo(FontPoint2),
    QuadTo { control: FontPoint2, end: FontPoint2 },
    Close,
}
```

Suggested entry points are `TrueTypeFont::open(bytes, FontLimits)`, `font.glyph_id(u32)`, `font.session(OutlineLimits)`, and `session.decode(GlyphId) -> Result<GlyphOutline, OutlineError>`. Expose read-only `units_per_em()`, `glyph_count()`, `table_records()`, and session `usage()`. A GlyphId can be passed across fonts, so `decode` must repeat its cheap range check rather than treating the newtype as font identity.

An outline exposes its GID, relative glyph byte range, raw point slice, contour-endpoint slice, command slice, glyph-header bbox and instruction-byte count. Distinguish `EmptyLocaRange`, `ZeroContours`, and `Simple` provenance. Store computed control-point bounds separately from the advertised bbox; neither is a tight quadratic-curve bound. No advance, glyph name, Unicode or PDF source byte is inferred.

Use doubled integral coordinates for outline commands. Original points have even coordinates; the midpoint between two original off-curve points can have odd coordinates. No floating-point rounding is needed. A future consumer may divide by `2 * units_per_em` when constructing the explicitly tagged font-to-text transform. Do not reuse PageSpace coordinates for font units or convert quadratic curves into cubics in this stage.

Errors carry a category (malformed, unsupported, limit, allocation), absolute font byte offset, optional table tag and optional GID. Decode returns an owned result only after complete validation. Failed attempts consume work already charged; they never return a partial contour or refund the session budget. Returning independent owned results does not let the session prove their live memory, so its allocation allowance is cumulative and is never refunded when an outline is dropped.

## Structural font validation

Accept only a standalone sfnt with version `0x00010000`. Explicitly refuse TTC collections, WOFF/WOFF2, CFF/OTTO and the older `true` signature initially. Refuse active variation tables (`fvar`, `gvar`, `avar`, `HVAR`, `VVAR`, `MVAR`) instead of silently interpreting a default instance. Unused ordinary tables may remain borrowed and uninterpreted.

The proposed directory reader checks the input cap first, then checked `12 + 16 * numTables`, unique tags, offsets and lengths, and nonoverlapping nonempty table ranges outside the directory. Zero-length records retain their tag and require an in-input offset but occupy no interval. Lookup uses our owned sorted index, not the file's search accelerators; their values are informational and must never control traversal. These structural decisions follow the sfnt layout while deliberately defining a narrower first-stage subset. [OpenType font organization](https://learn.microsoft.com/en-us/typography/opentype/spec/otff)

Require `head`, `maxp`, `loca` and `glyf`. For head, validate the fixed structure, version 1.0, magic value, unitsPerEm in 16–16384, loca format 0 or 1, and glyphDataFormat 0. Head flags do not authorize hint execution or side-bearing normalization. [OpenType head](https://learn.microsoft.com/en-us/typography/opentype/spec/head)

Require maxp 1.0 with its complete 32-byte structure and a nonzero glyph count within the chosen cap. Read maxPoints/maxContours as consistency metadata; never allocate directly from them. A decoded simple glyph exceeding these advertised maxima is malformed under this strict subset. Other interpreter/composite maxima do not cause allocations. [OpenType maxp](https://learn.microsoft.com/en-us/typography/opentype/spec/maxp)

Build a checked N+1 loca index. Short entries are doubled; long entries are direct offsets. Entries must be monotone and at most the glyf table length. Equal adjacent entries mean an empty glyph with no header read. Every selected nonempty interval is a hard reader boundary. Require the expected loca payload, permitting only at most three zero padding bytes. Do not require the final loca offset to equal glyf length; remaining bytes are uninterpreted trailing table storage, never part of a selected glyph. [OpenType loca](https://learn.microsoft.com/en-us/typography/opentype/spec/loca)

O0 compatibility adjustment: the measured simple-subset.pdf font packs valid bounded table payloads at unaligned offsets, including post at 2579. CONTRACT explicitly authorizes unaligned offsets, preserving directory exclusion and nonoverlap. Safe byte reads require no alignment. This is a documented compatibility policy, not a claim of sfnt standards conformance.

The first stage does **not** verify table checksums or whole-font checkSumAdjustment. Retain declared checksums as provenance and state this limitation explicitly. It validates the structures needed to decode outlines, not every OpenType table or the font's authenticity. Checksums offer no substitute for byte bounds and may be added later with an explicit whole-input work charge. Native acceptance of a damaged checksum is not an outline correctness gate.

## Simple glyph decoding and exact contour semantics

Each nonempty range must contain a ten-byte header. A negative contour count returns `UnsupportedComposite`; do not recurse or even follow a component index. Nonnegative counts use a bounded cursor within that glyph alone.

Read strictly increasing contour endpoints, derive point count from the last endpoint with checked addition, and charge the point/contour allowances before reserving arrays. Read and bounds-check instruction length, then skip and charge those bytes without interpreting them. Expand repeated flags only up to the known point count; an overlong run is malformed. Read the complete x-delta plane followed by the y-delta plane, keeping accumulation continuous across contours. Signed/short/unchanged forms must consume precisely their indicated bytes. Bit 7 is reserved; bit 6 is the current OVERLAP_SIMPLE flag, not a second reserved bit. [OpenType glyf](https://learn.microsoft.com/en-us/typography/opentype/spec/glyf)

The implementation policy is to retain whether the first point advertises overlap and reject a later overlap flag only when the first point did not advertise it. A first repeated flag can therefore legitimately repeat bit 6. Overlap does not alter outline geometry, and its absence does not prove disjoint contours. Preserve contour direction, coincident points, zero-length edges, and overlaps; R0's polygon restrictions do not apply to this decoder.

Accumulate coordinate deltas in checked i32 arithmetic and require final raw coordinates to fit i16. For nonempty point sets, require the header bbox to equal computed control-point extrema under the initial strict policy. This prevents a valid-looking header from hiding an inconsistent decode; if the measured corpus exposes a benign mismatch, record it and request an explicit contract adjustment rather than silently repairing. Do not compare the header with curve extrema, which can differ from control-point extrema.

For zero contours, accept a header-only body or a bounded optional instruction-length/body. A single trailing zero byte is treated as alignment padding, not a truncated instruction word. After decoded data, permit at most three zero padding bytes; arbitrary leftover payload is malformed. This intentionally narrower padding policy must have native compatibility cases and clear errors.

Contour conversion is iterative. Choose the initial on-curve point as follows: use the first point if it is on-curve; otherwise use the last point if that is on-curve; otherwise start at the midpoint of the first and last off-curve points. A run of adjacent off-curve points inserts their midpoint as an implied on-curve endpoint. Emit explicit quadratic commands and close every contour. The exact command convention should match doubled-coordinate FreeType decomposition: retain its final return-to-start segment, then add one `Close`; do not simplify degenerates or rotate the contour start. This convention is independently implementable from the raw point sequence. [Apple TrueType fundamentals](https://developer.apple.com/fonts/TrueType-Reference-Manual/RM01/Chap1.html), pinned `thirdparty/freetype/src/base/ftoutln.c`

Before implementation, lock examples for single on-curve and single off-curve contours, two off-curve points, all-off-curve rings, both possible on-curve endpoints, and consecutive off-curve points with odd coordinate sums. These are semantic cases, not merely parser round trips.

## Proposed default budgets and accounting

These defaults are deliberately much larger than the measured five fonts while remaining finite. Callers may lower them. Hard representational checks still apply if callers raise limits.

| Resource | Proposed default | Charge point |
| --- | ---: | --- |
| Font input bytes | 32 MiB | Before directory access |
| Table records | 256 | Before index allocation |
| Glyph slots | 65,535 | Before N+1 loca allocation |
| Owned font-index allocation | 2 MiB | Before each index reserve |
| Glyph decode attempts per session | 65,536 | Before GID lookup, including failures/empties |
| Contours per glyph | 4,096 | From header, before endpoint reserve |
| Points per glyph | 32,768 | From endpoints, before point/flag reserve |
| Instruction bytes per glyph | 65,535 | Before skip |
| Total consumed glyph bytes | 64 MiB | Every repeated decode and every byte traversed |
| Total contours / points | 262,144 / 1,048,576 | Before each expansion |
| Total emitted commands | 2,097,152 | Before command reserve/push |
| Total owned output and scratch allocation | 64 MiB | Checked size/capacity accounting |
| Total work units | 16,777,216 | Byte reads/skips, flag expansion, point decode, command emission |

Index construction also needs its own bounded work count: table sorting and overlap checks must be O(T log T), loca validation O(N), with no pairwise glyph-range work. No table checksum scan or whole-font glyph decoding occurs implicitly during open.

The session should charge a selected interval once before scanning, then count semantic operations separately; document this as conservative traversal accounting rather than an exact CPU-time measurement. Repeated GIDs consume the same work again. Limits cannot reset internally for each glyph. Capacity arithmetic uses checked multiplication/addition and fallible reserves. Precompute a safe linear command bound from points and contours; do not grow a vector by exact one-element reserves. Errors after allocation still retain the charged allocation budget.

No cache in the first stage. A future immutable glyph cache must have an explicit font identity and memory limit; a hit must not imply that downstream transformation/raster work is free. No recursion means no stack-depth exposure in outline decoding or result destruction.

## Why composites should wait

Bounded composite support is feasible, but it is not a small extension to simple delta decoding. It needs cycle detection and a global component-expansion budget, exact component order, point-attachment arguments as well as XY offsets, F2DOT14 transforms, offset-scaling flags, grid rounding, nested transforms, and policies for instructions and USE_MY_METRICS. These affect geometry and native normalization, not only performance. A simplistic translation-only implementation would accept fonts while giving wrong outlines.

The first corpus offers no real composite case to validate such behavior. Explicit first-stage refusal therefore gives a useful measurable result sooner. A next stage can begin with generated acyclic XY-positioned components under exact fixed-point affine arithmetic, then add point attachment and the remaining flags only with separate native comparisons. It must reject unsupported flags before returning any expanded outline. Do not promise that this narrow first stage will accept ordinary arbitrary TrueType fonts.

## Acceptance and native comparison

Use three independent layers:

1. Hand-written binary fixtures with independently specified raw points/contours and exact doubled-coordinate commands. Cover short/long loca, both coordinate encodings, flag repeats, every truncation boundary, off-curve midpoint cases, instructions, zero contours, empty slots and unusual but valid degenerates.
2. A Python integer model that generates point sequences and independently encodes them. Compare expected sequences and commands against Rust; mutation tests independently target lengths, flags, directory overlap, duplicate tags, coordinate overflow, malformed endpoints, bbox inconsistency, variation refusal, composites, allocation/work limits, and repeated failing requests. Avoid using the Rust output as the generator's expectation.
3. A bounded test-only oracle linked to the already pinned bundled FreeType. Compare all 1,885 observed slots (including the 72 nonempty outlines), then hundreds of generated valid fonts. Recheck fixture/program hashes, hash the oracle and Rust binaries, stage output atomically, reject diagnostics, and cap font/query/output bytes, native heap, CPU and wall time. Native permissive malformed-font recovery is separate evidence, never the definition of Rust acceptance.

Do **not** use `fz_outline_ft_glyph` as the exact reference. Pinned MuPDF `source/fitz/font.c:1517` initially loads with hinting and retries NO_HINTING only after failure; it can also apply synthetic style/width adjustments. Direct bundled `FT_Load_Glyph` with NO_SCALE, NO_HINTING, NO_AUTOHINT, NO_BITMAP and IGNORE_TRANSFORM is the intended reference, with no variation instance or system-font loading. [FreeType glyph loading](https://freetype.org/freetype2/docs/reference/ft2-glyph_retrieval.html)

There are two important native adapter corrections, both derived from the pinned source rather than assumed pixel equivalence:

- `ttgload.c:1339,2582` translates the loaded outline by minus `(glyphHeader.xMin - hmtx.leftSideBearing)`. The oracle must independently read the header and relevant hhea/hmtx record, report that adjustment, and undo it to recover raw glyf coordinates. Do not normalize by measured outline bounds, since that can conceal an incorrect decode. hmtx parsing belongs to this test adapter, not the proposed production outline API.
- `ftoutln.c` uses integer arithmetic for implied midpoints. Double the original integral FT points before calling `FT_Outline_Decompose`, so half-unit midpoints survive exactly. Report raw points/tags/endpoints before decomposition as a separate gate. A failure in command canonicalization cannot then be hidden by matching only rendered pixels. [FreeType outline processing](https://freetype.org/freetype2/docs/reference/ft2-outline_processing.html)

The oracle emits the requested GID, empty-kind evidence, raw points/on-curve tags, contour endpoints, origin adjustment and canonical Move/Line/Quad/Close commands in x2 integers. All geometry comparisons are exact; there is no floating tolerance or cubic approximation. Check native non-cubic tags and unexpected hint/variation behavior explicitly. Generated fonts must contain sufficient independent sfnt metadata for the native loader, including valid hhea/hmtx; production support remains the four outline tables described above.

Acceptance does not include glyph selection, Unicode correctness, text advances, pixel parity, antialiasing or hinting. Those remain separately testable engine stages.

## Source and license boundary

Production implementation should be original safe Rust from the format specification and the documented exact geometry contract. Retain the engine's AGPL license. Pinned MuPDF font.c carries Copyright 2004–2025 Artifex. Pinned FreeType files have their own FreeType/dual-license notices; inspecting or linking them in the test-only oracle does not make them AGPL-origin code. Preserve the applicable FreeType license and file notices if any source is copied or a binary is redistributed. Record exact reference-source hashes when the oracle is implemented. This design does not vendor source, font data, or a new production library.
