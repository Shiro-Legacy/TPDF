# ToUnicode CMaps: measured scope and proposed bounded stages

This is a design and workload inventory, not a CMap implementation. The next useful text foundation is a typed mapping from original PDF character-code bytes to validated Unicode sequences. It does not establish glyph selection, font metrics, text placement, reading order, search boxes, extraction, or rendering.

## Actual fixture evidence

`tools/cmap-inventory.mjs` reads immutable fixtures through pinned MuPDF 1.28.1, verifies source-file hashes, and verifies decoded ToUnicode hashes against the accepted stream comparison. Its bounded token scanner measures CMap constructs; it is not production grammar. Evidence is saved in `reference/cmap-inventory.json`. Decoded working copies are under `/private/tmp/tandem-cmap-inventory` and are reproducible.

| Measurement | Result |
| --- | ---: |
| Unencrypted fixture documents examined | 49 |
| Font dictionaries | 68 |
| Type1 / Type0 / CIDFontType2 / TrueType | 49 / 7 / 7 / 5 |
| Descendant CID font dictionaries included above | 7 |
| Top-level/simple font dictionaries after excluding descendants | 61 |
| Fonts with explicit ToUnicode | 10 |
| Unique ToUnicode stream identities / distinct payload hashes | 10 / 6 |
| Decoded ToUnicode bytes | 47,012 |
| bfchar records / bfrange records | 1,322 / 1,175 |
| Source entries represented by those records, counting repeated fixtures | 167,665 |
| Source-code widths present | 1 and 2 bytes |
| bfrange array destinations / multi-scalar destinations / valid surrogate-pair destinations | 0 / 0 / 0 |
| Mapping overlaps / usecmap operators | 0 / 0 |
| Declared section-count mismatches | 0 |
| Invalid UTF-16 destination literals / affected streams | 16 / 6 |

All ten CMaps tokenize successfully. The native object enumeration produces 24 recorded diagnostics while probing missing xref object 8 in 12 signed fixtures. The inventory records these with file/object identity; it does not present that scan as a warning-free document validation. No fixture was changed, repaired or saved. Fonts omitted from `/Type /Font` outside encountered `/Resources /Font` dictionaries may be absent. Font presence is not proof that a glyph is executed on a page.

The fixture font encodings are 50 WinAnsiEncoding, seven Identity-H, one MacRomanEncoding and ten absent entries. Seven absent entries are descendant CID fonts. Explicit ToUnicode therefore covers only ten of 61 top-level/simple font dictionaries; 51 require a separately specified fallback or an explicit unmapped result. Identity-H means a two-byte identity mapping to CIDs, not Unicode identity.

### A real malformed-input issue must remain visible

Six streams contain five-hex-digit Unicode destinations such as `<10400>`, `<10428>`, `<1044d>` and `<10efd>`. The accepted PDF lexer pads the final hex nibble, giving **three bytes**, which is not UTF-16BE. Pinned MuPDF's CMap parser takes `buf->len / 2` and discards the final odd byte: `<10400>` becomes code unit U+1040, rather than the apparent intended U+10400. This is a compatibility recovery, not a correct supplementary-plane encoding.

The affected streams are `cjk.pdf` 5:0, `p0b/edit-embedded.pdf` 5:0, `p0b/edit-subset.pdf` 5:0, `p3/edit-subset2.pdf` 5:0 and 11:0, and `rtl.pdf` 11:0. The report stores exact malformed destinations and source ranges. Existing two-byte sequential destination ranges have no measured overflow or surrogate-block crossings.

The first strict parser should refuse these six complete CMaps with an exact destination offset. The other four are initial real acceptance targets. If preserving good entries from malformed maps is needed later, add an explicit diagnostic-bearing load mode whose invalid entries return `InvalidDestination`; do not guess intended supplementary characters, silently truncate bytes, or silently omit mappings. Such a mode needs its own gate and must not be described as strict parsing.

Reproduce the measurement:

```sh
node tools/cmap-inventory.mjs \
  /Users/owner/Documents/TPDF/node_modules/mupdf/dist/mupdf.js \
  ../fixtures reference/cmap-inventory.json \
  /private/tmp/tandem-cmap-inventory target/debug/pdf-inspect
```

## U0: first bounded implementation milestone

Implement a standalone, read-only `ToUnicodeMap` parser and lookup API for embedded ToUnicode programs. No document resource lookup, system-CMap installation, filesystem access, font fallback, glyph conversion, text extraction, or graphics execution belongs in this first unit.

Proposed types:

```rust
// Fields private; construction validates width and value fit.
struct CharacterCode { value: u32, width: CodeWidth }
enum CodeWidth { One, Two, Three, Four }
struct CodeSpace { low: CharacterCode, high: CharacterCode }
struct UnicodeSequence { scalars: Vec<char> }
struct ToUnicodeMap { /* validated codespaces and owned mapping index */ }

parse_to_unicode(bytes: &[u8], limits: CMapLimits)
    -> Result<ToUnicodeMap, CMapError>;
map.lookup(code: CharacterCode) -> Option<&UnicodeSequence>;
map.decode_code_prefix(bytes: &[u8])
    -> Result<(CharacterCode, usize), CodeDecodeError>;
```

`<01>` and `<0001>` are different typed codes; never collapse source bytes to an integer before retaining width. Big-endian accumulation is checked and accepts exactly one through four bytes. Names remain bytes. No `String::from_utf8_lossy`, native pointers, or implicit casts from signed C-style integers.

The lookup returns **missing**, invalid input-code segmentation, and malformed Unicode as distinct conditions. It must not substitute the input integer as Unicode or return an ordinary space. A later text decoder may choose an explicit replacement policy while retaining original bytes and diagnostics. Unicode destinations are UTF-16BE byte strings: require a nonempty even byte length, decode surrogate pairs to scalar values, preserve multi-scalar mappings such as ligatures, and reject unpaired surrogates. Preserve U+0000 and noncharacters that are valid scalar values; do not strip an initial U+FEFF as a guessed BOM.

### Grammar without a PostScript interpreter

Reuse the accepted byte lexer, but add a dedicated CMap grammar. The content parser is unsuitable: CMap programs use declared sections, metadata definitions, and prolog/epilog scaffolding, not PDF page operations. No file-object reference lookahead or arbitrary PostScript evaluation.

Recognize one exact `begincmap` / `endcmap` body and a bounded, documented allowlist of inert common wrappers (the measured `/CIDInit /ProcSet findresource begin`, dictionary setup, and `CMapName currentdict /CMap defineresource pop` plus `end`). Exact metadata definitions may include CMapName, CMapType 2, CIDSystemInfo and WMode. Reject unsupported executable constructs instead of ignoring arbitrary words. Accept only explicitly supported wrapper syntax; broader producer compatibility can be added with fixtures.

Inside the body:

- `n begincodespacerange`: exactly n equal-width low/high byte-string pairs, ordered low <= high, followed by the exact end keyword.
- `n beginbfchar`: exactly n source/destination pairs and exact `endbfchar`.
- `n beginbfrange`: exactly n source-low/source-high/destination entries and exact `endbfrange`. Source endpoints must share a width. Array destination length must equal the inclusive source range size; every element is a valid destination string.
- Section counts are nonnegative integers checked before allocation. Use an explicit U0 cap of 100 records per section, matching the measured producer chunks; multiple sections are allowed. Larger sections remain unsupported until their policy is independently verified. Counts and matched terminators cannot be ignored as MuPDF currently does.
- Literal PDF strings and hex strings are both lexer string tokens; accept the same byte semantics when the actual grammar calls for a string. Empty or >4-byte source strings are invalid.
- Unknown mapping constructs, `begincidchar`, `begincidrange`, `beginnotdef*`, `usecmap` and unsupported CMap types fail explicitly in U0. Those describe distinct mapping semantics and must not be interpreted as ToUnicode by accident.

Validate each mapping source against a declared codespace. A map must have at least one valid codespace; do not synthesize a two-byte range. Reject same-width overlapping codespaces and cross-width prefix ambiguity. For different widths, compare the longer interval's leading-byte prefix interval against the shorter range. A decoder must distinguish an incomplete potentially valid code from a complete code outside every codespace; it must not consume one byte and invent code zero as MuPDF's recovery does.

For sequential bfrange destinations, U0 supports valid BMP sequences and multi-scalar UTF-16 strings only when the final code unit can be incremented across the entire source range without overflow and every resulting string is valid UTF-16. This includes stable high-surrogate + incrementing low-surrogate sequences where the low unit stays within its valid pair range. Crossing a surrogate boundary or requiring carry into an earlier code unit is an explicit unsupported/invalid range in U0. A complete general byte-string increment policy needs its own normative/reference tests; do not silently wrap a u16, increment a Unicode scalar instead of UTF-16 units, or adopt MuPDF's last-unit overflow recovery.

### Mapping overlap and predictable cost

Use strict duplicate/overlap rejection initially, including bfchar-versus-bfrange and range-versus-range collisions. MuPDF trims older intervals when a newer range overlaps them, and local mappings override a base `usecmap` on lookup. A future named compatibility policy may implement last-definition precedence, but it must charge discarded definitions and intermediate splitting work and test all overlap shapes. No such overlap appears in the measured fixtures.

A bounded expanded `BTreeMap<CharacterCode, UnicodeSequence>` is a reasonable first implementation: it makes duplicate detection and lookup simple and predictable. Compute each inclusive range size with checked arithmetic and charge all expanded entries **before** looping or reserving. Compact interval storage is a later optimization, not a reason to accept a four-billion-entry expansion. The measured fixture maps represent at most 33,135 entries in one CMap.

Proposed initial caps, recorded explicitly in the implementation acceptance:

| Resource | Proposed cap |
| --- | ---: |
| CMap input bytes | 4 MiB |
| Lexer token bytes / total tokens | 64 KiB / 1,000,000 |
| Codespace intervals | 64 |
| Total declared records | 100,000 |
| Expanded mapping entries / expansion work | 262,144 |
| Destination UTF-16 units per mapping | 64 |
| Cumulative owned destination/name bytes | 16 MiB |
| Wrapper/container depth | 16 |
| Document-level map count/decoded work | Separate later loader limits |

Every rejected limit must have a source offset and stable typed error. Count overwritten data if a future policy allows it. Limit lookup decoding/output work when converting an entire input string; a map lookup API alone does not bound caller-retained text. Native stack depth must remain independently bounded.

## U0 acceptance and native oracle strategy

The first deliverable is working parse/typed lookup/code-boundary behavior plus reproducible evidence; no production font integration yet.

Build a test-only C oracle against the already pinned native library using `pdf_load_cmap`, `pdf_lookup_cmap_full` and `pdf_decode_cmap`. It should accept a bounded CMap byte buffer plus bounded explicit query bytes, emit code widths/code values and Unicode scalar sequences, and cap source bytes, query count and output. No system CMap loading in the standalone oracle. Test subprocesses require timeouts. Source/ABI dependencies are isolated from production Rust.

Compare generated standard-library Python mappings against three independent expectations: the generated intended Unicode sequences, native lookup, and Rust lookup. Boundary decoding compares exact consumed byte counts. Enumerate all mapped source entries for the four strict-valid fixture maps; separately record the six strict refusals and native recovery for their offending destinations. Shared parser/serializer round trips alone are insufficient.

Required generated coverage:

- Source widths 1/2/3/4, leading zeros, boundary values, byte-prefix ambiguity, missing/out-of-range/incomplete input codes.
- bfchar, sequential bfrange and bfrange arrays; exact counts 0/1/100; multiple sections and interleaved mapping forms.
- BMP, U+0000, U+FEFF, non-BMP surrogate pairs, several Unicode scalars, ligature expansions, combining marks and sequences with supplementary characters before/after BMP characters.
- Odd destination byte counts, empty destinations, every unpaired-surrogate form, final-unit increment boundaries, range reversal, unequal source widths, short/long arrays and count mismatches.
- All duplicate/overlap shapes, codespace coverage, exact input/token/record/entry/payload limits and range-expansion bombs.
- Unknown and prefix-matched keywords: unlike MuPDF's `strncmp`, `beginbfcharGarbage` must not be accepted as `beginbfchar`.
- Every parser error path remains bounded, and generated byte mutations cannot panic or loop.

MuPDF's bf-range loader currently rejects source numeric values above 65535 even though other CMap APIs support four-byte codes. Keep generated independent expectations separate for wider numeric bfranges: either make them an explicit U0 unsupported subset or establish semantics with another primary reference before claiming native parity. Four-byte bfchar and code-boundary tests can still be useful, but high-bit values need unsigned query plumbing in the oracle.

## U1 and later document/font integration

A later loader resolves the font's ToUnicode stream through `decode_document_stream_with_usage`, with cumulative map/decode/cache limits. Preserve font object identity and original character bytes. Cache a map by source identity only; repeated text decoding still charges work. Do not report rendering glyph IDs as original PDF character codes.

`/UseCMap` dictionary references and in-program `/Name usecmap` require a separately gated resolver: active-stack cycle detection, reference/call depth, distinct cached maps, cumulative bytes and inherited mapping work. Pinned MuPDF gives the stream dictionary `/UseCMap` priority over the program's named base, uses a base only for missing local mappings, and inherits base codespaces only when local codespaces are absent. An explicit local/base ownership model such as immutable shared maps is preferable to eagerly duplicating every inherited entry. U0 rejects all such dependencies.

Predefined encoding CMaps (including Identity-H/Identity-V) are not predefined Unicode maps. Supporting Adobe CJK collections requires separately pinned table data, licensing/attribution, versioning and query oracles. Do not treat an unavailable predefined CMap as identity or derive a Unicode value from a CID. A missing ToUnicode on a simple font may instead require WinAnsi/MacRoman/StandardEncoding plus Differences and glyph-name mappings; embedded font cmap fallback is yet another capability. These are explicit future stages, not silent fallbacks in U0.

Text positioning will additionally need font widths, character/word spacing, horizontal scaling, writing mode and text matrices. Search/extraction needs string decoding policy, original-byte spans, ligature mapping and an ordering strategy. None follows merely from successful CMap loading.

## Pinned source and attribution

All source inspection used MuPDF 1.28.1 from the verified archive recorded in `reference/PROVENANCE.md`. Any implementation derived from these files retains the repository's AGPL-3.0-or-later and Artifex attribution:

- `source/pdf/pdf-cmap-parse.c`, Artifex 2004–2021: exact supported sections, integer source conversion, tolerant terminators/counts, odd Unicode-byte truncation and sequential mapping recovery.
- `source/pdf/pdf-cmap.c`: codespace decoding, mapping interval insertion/overlap precedence, Unicode surrogate decoding and base-map lookup.
- `source/pdf/pdf-cmap-load.c`, Artifex 2004–2024: embedded/system map loading, UseCMap priority, recursion checks and Identity encodings.
- `source/pdf/pdf-unicode.c`, Artifex 2004–2021: ToUnicode remapping through font encoding, collection and glyph-name fallbacks. This demonstrates why a standalone CMap is only one part of text decoding.
