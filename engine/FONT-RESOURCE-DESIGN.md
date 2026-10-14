# Bounded page font resources and declared Unicode mappings

This is a read-only design proposal. No font-resource implementation is authorized by this document. The first useful unit resolves a font resource selected by a page content `Tf` name and returns its declared Unicode mapping. It does not select rendered glyphs, read embedded font programs, compute advances, execute text, infer reading order, or perform extraction.

## Measured first-stage coverage

The pinned evidence is `reference/cmap-inventory.json` and `reference/simple-font-inventory.json`, produced by immutable-fixture dictionary/stream reads through MuPDF 1.28.1. The latter covers inherited page resources and records no native diagnostics or errors; the former separately records 24 diagnostics from missing-object probes. Neither inventory proves that a registered font was actually executed.

| Item | Measured count | Proposed first-stage disposition |
| --- | ---: | --- |
| Unencrypted documents / pages | 49 / 572 | Reuse accepted document and page-tree validation |
| All font dictionaries | 68 | Includes seven descendant CID dictionaries |
| Top-level Type1 / TrueType fonts | 49 / 5 | 54 eligible simple fonts in the all-object inventory; 53 occur in inherited page resources |
| Simple fonts without ToUnicode | 51 | 50 explicit WinAnsi, one explicit MacRoman |
| Simple fonts with ToUnicode | 3 | All three have strict-valid maps; Encoding absent |
| Top-level Type0 fonts | 7 | Explicitly unsupported in this stage |
| Inherited page resource registrations | 582 | 575 simple, seven Type0 |
| Pages containing only simple fonts | 565 | 562 Type1-only, two Type1+TrueType, one TrueType-only |
| Pages with no font registrations | 2 | Empty resource tables are valid |
| Pages with mixed Type1 and Type0 | 5 | Supported simple bindings remain available lazily |

The page-bound subset has 50 explicit-encoding identities and three ToUnicode identities. The remaining simple font, `p0b/certified-p2-filled.pdf` 12:0, appears only in object 13 Resources/Font/Helvetica (an appearance/Form context), outside this page-resource scope. It must be recorded as non-page inventory, not loaded by expanding F0.

The three eligible ToUnicode fonts are `p0b/edit-coded.pdf` 5:0 and `p6/quartz.pdf` 6:0 and 10:0, with 14, 39 and one mappings respectively. The fourth strict-valid map, `rtl.pdf` 5:0, belongs to a Type0 font and is outside this simple-font stage. All six malformed ToUnicode maps also belong to Type0 fonts. The loader's Type0 refusal must not be presented as exercising malformed-map rejection: synthetic simple-font fixtures must test that policy separately.

The 51 encoding-based fonts include two embedded, nonsymbolic TrueType Hebrew subsets. Returning their explicit encoding/name-derived Unicode is a statement about the PDF declarations, not proof of their embedded glyph semantics. Do not relabel those outputs as correct Hebrew extraction. There are no measured Differences arrays, implicit encoding defaults needed by an eligible font, or encoding dictionaries; those need generated fixtures.

## Existing capabilities and the integration gap

`Document<'input>` owns a validated object model while borrowing the original file bytes. `object(ObjectId)` preserves number/generation identity; `stream_data` exposes the correct stream bytes, including document object-stream support. There is no public document mutation API. Cache entries can therefore be scoped to one immutable document borrow without revision invalidation.

`pages::collect_pages` validates catalog membership, Parent/Kids agreement, cycles/shared nodes, inherited geometry and counts. Its public `PageInfo` contains only page identity, geometry and content IDs: it does **not** retain Resources, ancestry, or a trusted membership token. A caller can construct or alter a PageInfo. A font loader must not treat that structure alone as proof that an arbitrary object is a page in this document.

`decode_document_stream_with_usage` supports existing filters/predictors and returns encoded bytes plus every intermediate decoded byte charged. `parse_to_unicode` supplies strict immutable width-preserving maps; `decode_mapped_text` preserves source/UTF8 spans under per-call limits. Neither loader accepts arbitrary font fallback policy.

`simple_encoding::SimpleEncoding` is the separate S0 unit. It supports explicit `NamedEncoding::{WinAnsi,MacRoman,Standard,MacExpert}` and the exact 378-name registry shared by their tables. `lookup_unicode(u8)` yields `UndefinedCode`, `UnknownGlyphName(&[u8])`, or `Unicode(&[char])`. `with_differences(&[Difference], EncodingLimits)` and `with_object_differences(&Object, EncodingLimits)` validate transactional changes; the object adapter accepts a direct array and does **not** resolve references. The font-resource layer owns reference resolution and dictionary semantics. It must preserve unknown versus undefined rather than manufacturing replacements.

## F0: proposed minimal loader and ownership

Use one document-bound session with private fields and immutable result accessors:

```rust
struct FontResourceLoader<'doc, 'input> { /* document borrow, membership, budgets, caches */ }
struct PageFontBindings { /* page identity, resource owner, byte-name bindings */ }
struct LoadedSimpleFont { /* identity, subtype, BaseFont bytes, mapping */ }
enum SimpleFontMapping {
    ToUnicode(/* shared immutable validated map */),
    ExplicitEncoding(/* owned SimpleEncoding plus explicit base provenance */),
}

FontResourceLoader::new(&Document, FontResourceLimits)
    -> Result<FontResourceLoader, FontResourceError>;
loader.page_bindings(page_id: ObjectId)
    -> Result<&PageFontBindings, FontResourceError>;
loader.load(page_id: ObjectId, resource_name: &[u8])
    -> Result<&LoadedSimpleFont, FontResourceError>;
```

The constructor should invoke bounded page inventory once and retain a private membership set. This avoids rerunning a full tree walk for every Tf and prevents accepting orphan Page dictionaries or forged PageInfo values. A future trusted PageIndex shared with geometry/content loading can remove that repeated constructor cost. F0 should not change the public PageInfo shape or assume it contains Resources.

`page_bindings` enumerates the immediate inherited Font dictionary without opening every font. `load` resolves only the selected binding, validates its simple-font subtype and loads its mapping. An unsupported or malformed unused font must not block a page's supported binding. An absent resource name yields `MissingFontResource`, distinct from absent encoding or a missing Unicode entry. Enumerating all bindings and trying each is an explicit inspection operation, not page load behavior.

Cache only immutable successes. Indirect fonts are keyed by complete ObjectId within this loader's document. Resource aliases that resolve to the same object share the font result. A ToUnicode stream cache uses its complete final stream ObjectId, allowing distinct font objects to share a map. No global cache keyed only by object number, resource name, BaseFont, byte hash or raw pointer.

Direct font dictionaries should be supported with stable provenance `(resource-owner page-tree ObjectId, resource-name bytes)`, not fake indirect IDs. Their result can remain cached within that binding. An indirect Resources dictionary shared by several owners may be cached by its own identity; deduplicating every direct dictionary is unnecessary. Cap alias registrations separately from distinct fonts/maps so many aliases cannot evade work or retained-name limits. Use `Rc<ToUnicodeMap>` internally or a private arena index; do not clone large maps for aliases. Do not expose a forgeable cross-loader handle. Borrowed `&LoadedSimpleFont` from a mutable loader operation is a simple safe initial API; a later execution session can request an explicitly shared immutable result if mutation-borrow ergonomics require it.

Mapping getters should expose identity, subtype, original BaseFont/name bytes and mapping provenance. Return the validated map or encoding immutably. F0 need not add a unified string decoder: existing `decode_mapped_text` handles explicit ToUnicode, and the encoding lookup's distinct states are already useful. A later shared decoder must preserve spans and unknown/undefined errors without cloning the 256-slot encoding into a synthetic CMap.

## Resource inheritance and dictionary resolution

Resolve Resources by walking the already validated page's Parent chain, starting at the page, and selecting the nearest non-null Resources value. Resolve that selected value under a separate bounded reference-chain guard and require a non-stream dictionary. The whole dictionary replaces inherited Resources: **do not merge missing Font keys, individual font names or other resource categories with an ancestor**. A present empty dictionary intentionally supplies no fonts. Missing Resources throughout the chain yields empty bindings. Missing or null Font inside the selected Resources likewise yields no font bindings and does not restart inheritance.

Use byte resource names exactly as parsed, including escaped/high-byte names; no Unicode conversion, lowercasing or name normalization. Bound count and cumulative bytes before retaining names. Resolve Font dictionaries and each selected font through bounded reference chains with cycle detection, retaining the final identity for caching. Distinguish an actual stream dictionary from an ordinary dictionary. A stream cannot serve as Resources, Font, font dictionary, Encoding or descriptor. An indirect chain to a stream is acceptable only in the explicitly stream-valued ToUnicode path.

Direct null and indirect-to-null optional values should have an explicit, consistent absent-value policy. Proposed F0 policy is resolved null equals absent, including Resources inheritance; dangling/missing references are errors, never absent. Pinned `pdf_dict_get_inheritable` returns the first raw non-null entry and may distinguish a non-null reference whose target is null. Generated native tests must record any such difference rather than silently importing pointer-level behavior. Empty dictionaries remain different from null. Parent-chain and alias cycles refuse without the native page-tree repair retry.

Optional `/Type` is allowed absent/null; when present it must equal `/Font` for the font and `/Encoding` for an encoding dictionary. `/Subtype` must be a name equal to Type1 or TrueType. Type0, Type3, MMType1, CIDFontType0/2 and unknown/missing subtypes produce distinct unsupported/invalid errors. Never register a descendant CIDFont dictionary as a simple-font mapping. F0 does not validate complete rendering font dictionaries: FirstChar, LastChar, Widths, FontDescriptor and embedded programs belong to later metrics/glyph stages and should not acquire invented defaults here.

## Mapping precedence and meaningful unsupported fields

1. Resolve ToUnicode first. Missing or resolved null means no explicit map. A non-null named predefined map is unsupported. A non-stream value is invalid. A valid indirect stream proceeds through bounded stream decoding and strict CMap parsing.
2. Before decoding, inspect the stream dictionary's `/UseCMap`; any non-null value is unsupported, even if the stream program itself is standalone. The program parser already refuses `usecmap`. Do not ignore the stream dictionary dependency, which native `pdf-cmap-load.c` considers independently. Reject unsupported external-file streams through the existing decoder.
3. For a simple font, every codespace/mapping source must have width one. A valid multi-byte standalone CMap is not a valid simple-font segmentation policy. Preserve the map's metadata, but refuse WMode 1 for this initial horizontal-only simple-font mapping stage; vertical text execution has no contract yet.
4. Once a strict ToUnicode map is selected, it is authoritative for this loader. Do not parse an unused Encoding merely to invent a fallback, and do not replace a missing map entry, invalid code or malformed map with encoding-derived Unicode. A non-null rejected ToUnicode is an error even if Encoding is valid. Rendering validation of an unused malformed Encoding is outside this mapping-only stage and must not be claimed.
5. If ToUnicode is absent, require an explicit supported named Encoding or an encoding dictionary with an explicit supported BaseEncoding. Missing Encoding/BaseEncoding yields `NeedsFontBuiltinEncoding`, not an automatic StandardEncoding. Unknown names, including Identity-H/Identity-V, are unsupported. BaseFont spelling, subset prefixes and descriptor flags do not authorize guessing.
6. An encoding dictionary may carry absent/null Differences, or a resolved bounded array. Resolve array items individually under the loader's budgets; validate integer 0..255 and byte-name types before creating borrowed `Difference` entries. Avoid cloning an Object tree. Preserve source array index in errors. Feed the bounded resolved sequence to S0's transactional adapter; repeated overrides remain last-assignment wins and every processed item/name byte is charged. Unsupported executable or nested values are errors.

Explicit encoding lookup describes the declared glyph-name mapping even for embedded or symbolic fonts. Do not claim that it equals the selected glyph without a font-program/selection stage. If product use later demands that stronger promise, introduce a separate capability check rather than retrospectively changing what the mapping loader means.

## Cumulative limits, accounting and failure behavior

Per-call limits are insufficient for many pages, aliases or repeatedly failing maps. A loader session owns monotonic work counters, separate from retained-cache counters. Suggested starting caps:

| Resource | Session default |
| --- | ---: |
| Page-tree traversal | Existing PageLimits, default 10,000 pages / 20,000 nodes / depth64 |
| Reference-chain / inherited-parent depth | 64 each; caller cannot remove hard stack safety |
| Font selection calls / total reference hops | 100,000 / 1,000,000 |
| Resource bindings / distinct font results / distinct ToUnicode maps | 100,000 / 4,096 / 1,024 |
| Retained resource and font-name bytes | 4 MiB |
| Referenced encoded ToUnicode bytes | 64 MiB |
| Intermediate decoded ToUnicode work | 128 MiB |
| Total decoded CMap input parsed | 64 MiB |
| Retained CMap entries / scalar payload | 1,048,576 / 64 MiB |
| Total Differences items / processed name bytes | 100,000 / 16 MiB |
| One map / one encoding | Existing CMapLimits, StreamLimits, EncodingLimits |

Every lookup/selection call and every actual reference hop is charged even on cache hits. Success caches can avoid repeated stream decoding and CMap parsing, so report this clearly as actual loader work. Repeated text decoding remains separately charged by TextDecodeLimits and, later, an execution-session cumulative text budget. Caching a font does not make arbitrary repeated Tj output free.

Shrink each per-map decoder/parser allowance to the remaining session allowance before allocating. Charge physical encoded input before calling the decoder; on success charge its exact `total_decoded_bytes`, not just final length. Cache payload only after complete validation. Retain neither decoded CMap source buffers nor failed partial maps unless an explicitly capped diagnostic artifact requires them.

The current stream decoder reports usage only on success, and the CMap parser does not expose parse-work usage. A loader must not retry arbitrary failed loads with uncharged work. A practical F0 policy is to reserve the effective per-call worst-case decode/parse allowance before calling, refund unused reservation when measurable success returns, and keep the reservation spent on failure. Shrink these reservations to remaining budgets so capacity is not exceeded. Alternatively add backward-compatible usage-bearing failure/results; that is a separately reviewed API change, not something this design assumes exists. Retained payload can be computed from validated map entries/scalars/name and encoding slots, but does not replace attempted-work accounting. Limit exhaustion is sticky for that exhausted budget; callers may still inspect already loaded immutable results through non-mutating getters.

Errors carry page identity, raw resource name (borrowed where possible or length-bounded), final font/stream identity when known, and a bounded field path or Differences index. Preserve nested document, stream, CMap and encoding errors; a CMap byte offset is within the decoded ToUnicode stream, not falsely a file offset. Avoid retaining unbounded input excerpts or an entire reference path in an error. Distinguish missing binding, reference cycle/missing object, invalid type, unsupported font/encoding/dependency, builtin-encoding requirement, malformed ToUnicode and resource limit.

## Acceptance before integration

First require the complete existing Rust fmt/test/Clippy gates and independent review. Add a dictionary-level native inspector; do not call `pdf_load_font` as the sole oracle, because it loads/substitutes font programs and may recover invalid mappings. Pinned `pdf_dict_get_inheritable` plus dictionary identity reads establish resource selection; the accepted native CMap and encoding-table oracles establish mapping contents independently.

For the real corpus, compare all 572 inherited page resource tables and all 582 name/object registrations. Load all 53 page-bound eligible simple-font identities, with aliases sharing results; compare all 256 byte slots for each of the 50 explicit encodings (12,800 slots), including undefined slots, and all 54 mapped entries across the three simple ToUnicode maps. Record the one non-page simple font separately; no Form expansion is implied. Report the seven Type0 identities/registrations explicitly unsupported. Assert the 567 simple-or-empty pages enumerate without composite failures and that five mixed pages retain their supported simple bindings. Do not interpret these counts as successful text execution or extraction.

Generated tests must cover:

- Catalog membership, forged PageInfo/orphan pages, Parent/Kids disagreement, parent/resource/encoding reference cycles, missing generations, compressed font dictionaries, inherited versus direct Resources, empty override, null policy, Font absence and byte-name aliases. Verify whole-dictionary replacement without key merging.
- Direct font dictionaries, indirect alias chains, two documents reusing an ObjectId, same font under multiple names, shared ToUnicode streams, and unused unsupported fonts beside valid selected fonts. Confirm caches cannot cross documents or elide selection work.
- Simple ToUnicode without Encoding, authoritative explicit mappings despite differing Encoding, malformed/unsupported ToUnicode with valid Encoding and no fallback, missing map entries, multi-byte source maps, WMode 1, stream `/UseCMap`, named predefined maps and external streams.
- Explicit named bases and dictionary/Differences bases; absent base, unknown byte names, scalar/array/type errors, indirect array/item values, .notdef and unknown-glyph results, repeated assignments, byte255 edges and exact cumulative item/name limits.
- Zero/exact/one-over limits for every cumulative counter, compressed expansion, a large map shared by aliases, many distinct small maps, repeated failed loads, and prefix/error cases that prove attempted work is charged. Check no result publication on validation failure and borrowed/owned lifetimes at compile time.

No fixture modification, font-file decoding, metrics, Form/XObject resource-stack expansion, text execution, page editing, search output or rendering belongs to F0. A later Form loader must add nested resource scopes; page Resources cannot be used blindly for text inside a Form. Text execution will also need Tf state, Tj/TJ sequencing, font widths, writing mode, spacing/scaling, matrices and provenance through content-stream joins.

## Primary source and attribution

Inspection used only the verified pinned MuPDF 1.28.1 archive already recorded in `reference/PROVENANCE.md`: `source/pdf/pdf-object.c` (`pdf_dict_get_inheritable`, raw nearest-value return and cycle repair), `pdf-page.c` (`pdf_page_resources`), `pdf-font.c` (explicit encoding tables, context-dependent builtin defaults and recovered ToUnicode loading), `pdf-unicode.c` (ToUnicode remapping through font encoding and name-based secondary data), and the previously reviewed `pdf-cmap-load.c`. These explain reference behavior, not a license to copy its recovery policy into the strict loader. Any derived implementation retains the engine's AGPL-3.0-or-later and Artifex attribution through NOTICE. The S0 table provenance remains in SIMPLE-FONT-DESIGN.md and reference/encoding-table-inventory.json.
