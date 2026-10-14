# Read-only content operations: design and measured scope

This design follows accepted R4c document reading, stream decoding and R5b preservation tests. It does not claim Rust rendering, text extraction, font decoding, image decoding, clipping rasterization or page editing. The first useful renderer-facing milestone is a bounded, source-mapped sequence of operations, followed by a separately gated path-only graphics interpreter. Generic syntax must remain independent of whichever operators that interpreter supports.

## Measured workload

`tools/content-inventory.mjs` classifies the 639 streams in `reference/stream-comparison-r4b.json` using pinned MuPDF 1.28.1. It selects page `/Contents` members and streams with `/Subtype /Form`, including appearance Forms; it does not claim that every Form is reachable or invoked. It verifies every selected decoded SHA-256 against the existing stream comparison. The inventory scanner is test tooling, not the production parser.

`reference/content-inventory.json` records:

| Measure | Result |
| --- | ---: |
| Unique selected content streams | 592 |
| Unique page /Contents streams | 572 |
| Form streams | 20 |
| Selected decoded bytes | 2,544,368 |
| Logical pages, including an empty page | 572 |
| Pages with content | 571 |
| Page content stream references, counting repetition | 573 |
| Pages with multiple /Contents members | 2 |
| Completely scanned streams | 590 |
| Streams stopped before inline-image payload | 2 |
| Operator occurrences, including the two scanned prefixes | 27,680 |
| Occurrences in completely scanned streams | 27,629 |
| Distinct observed operator words | 38 |
| Unknown operator words in scanned data | 0 |
| Cross-checks against the accepted Rust lexer | 592 passed, zero failures |

Frequent operations include `Tj` (3,880), `Td` (2,665), `BT`/`ET`/`Tf` (1,867 each), `Tm` (1,714), `re` (1,217), `S` (1,010), `w` (1,006), `m` (1,004), `cm` (748), and `W` (706). The set also contains `TJ`, both text-show quote operators, `T*`, `TD`, text spacing/scaling/render mode, marked content, `cs`/`sc`, and 41 `Do` operations in 17 streams. Synthetic large-page fixtures dominate these totals; this is not an estimate of general PDF coverage.

Two streams stop exactly at `BI`: `p3/edit-tail.pdf` object 5 and `p3/targets-unsupported.pdf` object 5. Counts after those inline images are deliberately absent. There is no `EI` search. The 2,097,153-byte `p3/targets-limits.pdf` content stream consists mainly of whitespace and is completely scanned; token limits must not accidentally count every whitespace byte as a token.

The scanner does not validate operand types, arity, state balance or resource existence. Zero unknown words therefore establishes useful implementation scope, not content validity. Type 3 CharProcs, tiling patterns and non-Form resource streams are excluded from this page/Form inventory.

Reproduce:

```sh
node tools/content-inventory.mjs \
  /Users/owner/Documents/TPDF/node_modules/mupdf/dist/mupdf.js \
  ../fixtures reference/content-inventory.json \
  /private/tmp/tandem-content-inventory
```

## Stage C0: generic content grammar

Proposed interface:

```rust
ContentParser::new(bytes: &[u8], limits: ContentLimits) -> ContentParser
ContentParser::next_operation(&mut self) -> Result<Option<Operation>, ContentError>

struct Operation {
    operator: Vec<u8>,
    operands: Vec<ContentOperand>,
    span: Range<usize>,
}
```

`ContentOperand` contains null, boolean, integer, real, byte name, byte string, arrays, and byte-key dictionaries. It has **no Reference variant**. Dictionary duplicate handling defaults to rejection. The operation span starts at the first operand, or the operator for a zero-operand operation, and ends at the operator token; it excludes trailing trivia. Errors identify the logical byte offset and remain sticky. Returned operand trees are owned; values are moved out, not cloned per operation.

Reuse `Lexer`, but do not call the file object parser. In particular, do not perform its `n g R` lookahead. File syntax keywords `R`, `obj`, `endobj`, `stream`, `endstream`, `xref`, `trailer`, and `startxref` are forbidden even in compatibility sections. Names such as `/R`, and string bytes spelling `R`, remain ordinary operands. Reserved file words inside containers fail as invalid operand keywords. Boolean/null keywords are operands; all other ordinary top-level words terminate an operation and are preserved verbatim for later classification. Operators cannot appear inside operand arrays or dictionaries.

Examples: `1 0 R` fails as forbidden file syntax; `[(a) -120 (b)] TJ` produces one operation with one array operand; `<< /MCID 0 >>` without a following operator fails at EOF as pending operands. A trailing number is not silently dropped. Integer and real variants retain their original distinction. There is no implicit UTF-8 or Unicode conversion of PDF strings.

`BI` fails immediately on recognition, before reading any dictionary token or binary byte that follows it. `ID` and `EI` outside an implemented inline-image construct also fail explicitly. This rule must be tested with malformed and enormous binary tails and a token budget that would otherwise fail during lookahead. `/BI` and `(BI)` are not inline-image starts.

Limits are cumulative for a parser instance: input bytes, token count/length, operations, operand nodes including dictionary keys, total decoded literal/name bytes, container depth, top-level operands per operation, and operator word length. Sensible initial defaults are 64 MiB input, 100,000 operations, 1,000,000 operand nodes/tokens, 16 MiB decoded payload, depth 64 (hard cap 256), 256 operands per operation, and 64-byte operator names. The accepting milestone must record the actual chosen defaults. Consumer-retained operation lists need their own output budget; a streaming API alone does not bound a caller's retained history.

## Stage C0b: bounded logical page-content loading

`PageInfo.contents` retains order and repeated references but loses the distinction between a direct stream and a one-element array. Inspect the page's actual `/Contents` entry under a bounded reference chain, validate that its resulting IDs equal the supplied `PageInfo.contents`, and retain this distinction.

Pinned MuPDF's `pdf_open_object_array` calls `fz_open_concat(ctx, n, 1)`. `next_concat` inserts **one ASCII space (0x20) after every exhausted array member, including the last and empty members**. Direct single streams receive no padding. Implement that exact compatibility policy. Do not substitute a newline: a comment at the end of one member continues into the next member until a real line ending, and a string spanning members includes the inserted space. Numbers and operator words cannot merge across members.

Proposed loader:

```rust
load_page_content(document, &PageInfo, PageContentLimits) -> Result<PageContent, PageContentError>
// PageContent owns bytes, per-member logical ranges/padding locations, and DecodeUsage.
```

Cap referenced stream count, cumulative encoded input bytes, actual cumulative intermediate decode output, final joined bytes, and each individual decoder's limits. A repeated reference is charged and executed repeatedly. Do not reset the work budget per stream or use only the final decompressed length: two filters plus a predictor can have much larger intermediate output. Use the new `decode_document_stream_with_usage` API; `DecodeUsage.total_decoded_bytes` includes every filter/predictor output and the identity copy when unfiltered.

The joined-byte cap bounds retained logical bytes, while the per-stream and work limits additionally bound temporary decoder output. Charge raw input before decoding, reduce the remaining decoder work limit for each call, and check joined growth before allocation. Padding consumes joined-byte budget but does not count as decoded filter output. Errors name both the page and failing stream where available. The loader does not interpret content or reject `BI`; the content parser performs that refusal before touching its payload.

## Stage C1: classification and typed operation validation

Keep generic parsing complete for ordinary operator words. Add a separate registry distinguishing known PDF operators from unknown extensions. A path-only interpreter must reject a known but unimplemented text/image/resource operation even inside `BX`; it must not confuse it with an ignorable unknown extension.

| Family | Typed operands / proposed representation |
| --- | --- |
| `q`, `Q`, `cm` | Save/restore; six finite matrix components |
| `w`, `J`, `j`, `M`, `d`, `ri`, `i`, `gs` | Stroke state; dash array plus phase; intent/resource byte names |
| `m`, `l`, `c`, `v`, `y`, `h`, `re` | Typed point/curve/rectangle/close commands with exact arity |
| `S`, `s`, `f`, `F`, `f*`, `B`, `B*`, `b`, `b*`, `n` | Paint/end-path mode, fill rule and close flag |
| `W`, `W*` | Pending nonzero/even-odd clip, applied at path termination |
| `G/g`, `RG/rg`, `K/k` | Gray, RGB, CMYK component tuples; preserve stroke/fill distinction |
| `CS/cs`, `SC/SCN/sc/scn` | Byte color-space name and component vector, optional pattern name; resource-dependent validation later |
| `BT/ET`, `Tc/Tw/Tz/TL/Tr/Ts`, `Tf` | Text-object boundary/state; font name and size |
| `Td/TD/Tm/T*` | Text/line matrix updates with exact arity |
| `Tj`, `TJ`, `'`, `"` | Byte strings; TJ array accepts only strings/numbers; quote operands kept in defined order |
| `Do`, `sh` | Named XObject/shading invocation; no implicit resource execution |
| `MP/DP/BMC/BDC/EMC` | Byte tag and optional name/property dictionary; marked-content structure |
| `BX/EX` | Compatibility-section boundaries |
| `d0/d1` | Type 3 metrics; parsed/classified but unsupported outside a separately gated glyph context |

Separate arity/type checks from semantic ranges and rendering conversions. Preserve all finite values initially; do not invent integer casts, clamp arbitrary values, or treat a byte text string as characters. Renderer-defined clipping/color/line rules require their own reference evidence.

Compatibility state belongs to the typed consumer: `BX` and `EX` require zero operands, have bounded nesting, and underflow/unclosed sections fail in the strict mode. Unknown operations are errors outside compatibility sections and explicit ignored-extension events inside them. Their already bounded operands are discarded with that operation. Known unsupported operators remain errors. Unknown compatibility events must not execute resource lookups or modify graphics state. Test this synthetically because the current inventory contains no BX/EX cases.

## Stage C2: first useful renderer-facing result

Recommend a **path-only graphics-state/display-list interpreter**, with a strict unsupported result for text, images, shadings, patterns, transparency, optional-content visibility, and resource-dependent operations until each is implemented. The generic syntax layer continues to expose those operations without pretending to draw them.

Support exact arity for path construction/painting, device colors, line width/cap/join/miter/dash, matrix concatenation and balanced save/restore. Store immutable paint events with path geometry, paint CTM, stroke/fill state and clip representation. Keep full curves and fill rules; a bounding rectangle is not a clipping-path substitute. A logical display list can later feed a rasterizer without changing content grammar.

The current path is not part of the saved graphics state. `q/Q` must not restore a path snapshot. `W/W*` sets a pending clip; application and path clearing occur at the path-ending operation. Test closepath/subpath behavior, `s/b/b*`, cubic shorthand `v/y`, negative rectangle dimensions, empty paths, multiple pending clips and save/restore boundaries explicitly. Pin matrix multiplication order with noncommuting translation/scale/rotation cases. MuPDF's `pdf_run_cm` uses `fz_concat(new, current)`; its path callbacks store user-space coordinates and painting supplies the current CTM. Mid-path matrix changes deserve explicit comparison or initial refusal rather than an untested transform assumption. Reject nonfinite arithmetic results rather than allowing overflow into geometry.

Do not claim end-to-end fixture rendering when the fixture contains unsupported operations. Most measured pages contain text; the first useful evidence is exact generated graphics events and native reference traces, followed by narrowly scoped raster output in a later stage. This stage is valuable as a real interpreter boundary, not a rasterizer substitute.

## Resource and execution scopes for later stages

Resource resolution is byte-name lookup by category through an explicit environment, with bounded indirect resolution and cycle errors. A page begins with inherited page-tree Resources. A Form invocation has a separate resource/state scope, its matrix and BBox clip, and an explicit fallback policy. Pinned MuPDF uses caller resources when the Form has no Resources and searches its resource stack on missing names; any equivalent compatibility fallback should be specified and tested rather than implemented by accidentally merging dictionaries.

`Do` resolves to a Form or Image object; Form bytes may be cached by ObjectId, but execution is charged at every invocation. Use an active invocation stack for cycles, not a global visited set that suppresses legitimate repeated Forms. Bound call depth, invocation count, total executed operations, decoded/cache bytes, emitted events/path segments, clip nodes, resource resolutions, and save/restore depth. A cached operation program does not cache caller-dependent graphics or fallback-resource state.

Form entry/exit must fence its graphics, marked-content and compatibility state. Preserve the caller's state even on errors; do not let extra Q/ET/EMC escape a Form. Transparency Groups, soft masks, blend modes, pattern content, Type 3 glyphs, annotations and optional-content policies each require explicit additional scopes. Text matrices and graphics text-state parameters have different lifetimes; a font/CMap/width engine is required before TJ advances or Unicode extraction can be claimed.

## Inline images: separate exact-boundary milestone

An inline image is not a normal operand dictionary followed by arbitrary bytes searched for `EI`. Binary data can contain those letters. Start with only cases whose encoded boundary is provable:

1. Unfiltered images with checked dimensions, bit depth and a supported color-space component count: calculate row bytes and exact payload extent.
2. Supported filters with decoder-reported consumed encoded bytes and validated end markers/checksums. The current whole-input decoder API rejects trailing bytes and is insufficient for this; add a separately tested prefix-decoding API before using it.

After the exact payload extent, validate the required EI token boundary. Handle ID's whitespace/CRLF rules explicitly without trimming image bytes. Inline-image abbreviations belong to this grammar, not general stream dictionary aliases. Reject unsupported filters, predictors, unresolved color spaces, impossible sizes and ambiguous boundaries. Never use regex EI search, a binary scan, or an unverified Length extension as the default boundary rule. Include payloads containing fake `EI`, `endstream`, whitespace and comment markers in the acceptance corpus.

## Acceptance and sources

C0/C0b should compare all 590 complete selected streams and the two inline-image prefixes; the two BI inputs must report the explicit unsupported location. Native lexical comparison plus independently generated operand/operator cases must exercise nested arrays/dictionaries, all byte escapes, malformed numbers/containers, forbidden references, pending EOF operands and every budget edge. Assert that BI refusal performs no post-BI lookahead.

Test direct Contents versus arrays of zero/one/many/repeated/empty members, comments and strings across joins, indirect array references/cycles, encoded-input limits, multi-filter intermediate growth, predictor work, exact padding limits and forged PageInfo mismatches. Keep source identity/range information so later errors can be translated from joined offsets to an original stream or inserted padding.

Use generated operator sequences to validate C1/C2 independently: arithmetic/type failures, unknown/BX semantics, q/Q path ownership, clipping timing, noncommuting transforms, state isolation, recursion and repeated resource calls. Round trips through our own parser alone are insufficient. Keep existing file-reader, decoder, writer, native no-repair, byte and pixel-preservation gates unchanged.

Primary implementation references are the pinned MuPDF 1.28.1 archive in `reference/PROVENANCE.md`, specifically:

- `source/pdf/pdf-interpret.c`: `pdf_process_stream`, `pdf_process_keyword`, `parse_inline_image`, `pdf_lookup_resource`.
- `source/pdf/pdf-stream.c`: `pdf_open_object_array`, `pdf_open_contents_stream` (array concat at line 728 in the pinned archive).
- `source/fitz/filter-basic.c`: `next_concat` (space padding at lines 350–361).
- `source/pdf/pdf-op-run.c`: `pdf_run_xobject`, graphics/path callbacks, `pdf_run_cm` (2777 onward).

These explain the reference behavior, including recovery and compatibility extensions. A new safe Rust layer should retain strict explicit errors where promised rather than silently importing MuPDF's malformed-input recovery. All reference-derived implementation remains under the recorded MuPDF-compatible AGPL licensing.
