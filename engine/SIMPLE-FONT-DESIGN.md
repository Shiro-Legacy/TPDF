# Simple-font byte encodings: measured scope and proposed standalone unit

This is a design and read-only inventory, not a font engine implementation. The useful next unit is an explicit byte-to-glyph-name encoding with a conservative Unicode lookup. It requires no font files, system fonts, document resource traversal, text layout or rendering.

## Fixture evidence

`reference/simple-font-inventory.json`, produced by `tools/simple-font-inventory.mjs`, extends the accepted CMap inventory with descriptor, embedded-program and inherited page-resource metadata. It verifies immutable fixture hashes and prior encoding values, uses pinned MuPDF 1.28.1 dictionary reads, and performs no font loading. This pass records zero native diagnostics and zero errors. The earlier CMap inventory's malformed-xref probing diagnostics remain recorded there; this pass uses already identified font references and does not repeat that enumeration.

| Measurement | Count |
| --- | ---: |
| Unencrypted documents / pages | 49 / 572 |
| All font dictionaries / top-level font dictionaries | 68 / 61 |
| Top-level Type1 / TrueType / Type0 fonts | 49 / 5 / 7 |
| Top-level fonts with / without ToUnicode | 10 / 51 |
| Without ToUnicode: explicit WinAnsi / MacRoman | 50 / 1 |
| Top-level encoding names / absent encodings / encoding dictionaries | 58 / 3 / 0 |
| Differences arrays | 0 |
| Inherited page-font resource registrations | 582 |
| Pages with no registered font resources | 2 |
| Top-level embedded simple fonts / those without ToUnicode | 5 / 2 |

The 51 fonts without ToUnicode comprise 48 Helvetica and one Times-Roman Type1 fonts, one embedded TrueType Hebrew subset with WinAnsi, and one embedded TrueType Hebrew subset with MacRoman. Both embedded examples have descriptor Flags 32 (nonsymbolic). That classification does not establish the embedded glyph shapes, selection or intended Hebrew text; the proposed unit reports the PDF encoding/name-derived mapping only.

The three absent Encoding entries are `p0b/edit-coded.pdf` font 5:0 and `p6/quartz.pdf` fonts 6:0 and 10:0; all already have ToUnicode. None of the 51 requires an implicit default to exercise the proposed explicit encoding unit. Seven top-level fonts use Identity-H and are composite Type0 fonts; this is a CID encoding and supplies no Unicode identity policy.

All BaseFont values and dictionary/resource contexts are retained in the evidence. Resource registration is not proof of an executed Tf/Tj operation. The inventory does not expand Form invocations or decide which glyphs were used; earlier contexts include encountered Form/resources, and this report additionally records inherited page Resources/Font entries.

Reproduce:

```sh
node tools/simple-font-inventory.mjs \
  $REPO/node_modules/mupdf/dist/mupdf.js \
  ../fixtures reference/simple-font-inventory.json
```

## PDF encodings are glyph-name tables, not Python codecs

The exported pinned APIs are `fz_glyph_name_from_win_ansi`, `fz_glyph_name_from_mac_roman`, `fz_glyph_name_from_adobe_standard`, `fz_glyph_name_from_mac_expert`, and `fz_unicode_from_glyph_name[_strict]`. There is no need to load a PDF font to query these. The test-only `tools/mupdf-encoding-oracle.c` emits exactly 1,024 fixed rows and reads no input or font files. Its builder checks the relevant encoding/AGL source hashes. `tools/encoding-table-inventory.py` independently parses the four fixed C tables, checks native names byte for byte, records strict/loose native values and compares Python codecs as diagnostic contrast only.

`reference/encoding-table-inventory.json` records every row, binary/source hashes, undefined slots and exact codec differences:

| Table | Undefined byte slots | Distinct from corresponding Python codec |
| --- | ---: | ---: |
| WinAnsiEncoding | 32 | 40 |
| MacRomanEncoding | 48 | 50 |
| StandardEncoding | 107 | No corresponding codec assumption |
| MacExpertEncoding | 91 | No corresponding codec assumption |

The union contains 378 named glyphs; every one has a valid, nonzero native strict single-scalar mapping. Notable PDF-name-derived values include:

| Encoding / byte | PDF glyph name / Unicode | Python codec result |
| --- | --- | --- |
| WinAnsi 7F | bullet / U+2022 | U+007F |
| WinAnsi 81, 8D, 8F, 90, 9D | bullet / U+2022 | Undefined in CP1252 |
| WinAnsi A0 | space / U+0020 | U+00A0 |
| WinAnsi AD | hyphen / U+002D | U+00AD |
| MacRoman CA | space / U+0020 | U+00A0 |
| MacRoman DB | currency / U+00A4 | U+20AC |
| MacRoman AD, B0, B2 and other recorded slots | Undefined glyph slot | Defined mathematical characters |

Bytes 00–1F are undefined in both PDF tables, even though codecs return control characters. Undefined must remain distinct from an explicitly mapped U+0000. PDFDocEncoding describes another domain and must not be reused for font character bytes.

Reproduce fixed-table evidence:

```sh
sh tools/build-encoding-reference.sh /private/tmp/tandem-mupdf-reference
python3 tools/encoding-table-inventory.py \
  /private/tmp/tandem-mupdf-reference/mupdf-encoding-oracle \
  /private/tmp/tandem-mupdf-reference/mupdf-1.28.1-source \
  reference/encoding-table-inventory.json
```

## S0: proposed useful first unit

Implement the four exact named PDF tables and a standalone, transactional Differences application. Support explicit `NamedEncoding::{WinAnsi,MacRoman,Standard,MacExpert}`; no host codec calls, implicit BaseFont mapping or fallback table. The standalone caller supplies the base explicitly. A later dictionary adapter can resolve `/BaseEncoding`; an absent base is an explicit `NeedsFontBuiltinEncoding` condition until its font-context policy is implemented.

Suggested API shape:

```rust
enum NamedEncoding { WinAnsi, MacRoman, Standard, MacExpert }
struct GlyphName { /* validated bounded nonempty owned PDF name bytes */ }
enum Difference { Code(u8), Name(GlyphName) }
struct SimpleEncoding { /* exactly 256 optional glyph-name slots */ }
enum GlyphUnicode<'a> {
    UndefinedCode,
    UnknownGlyphName(&'a [u8]),
    Unicode(&'a [char]),
}
SimpleEncoding::from_named(base: NamedEncoding) -> Self;
SimpleEncoding::with_differences(base, entries, limits) -> Result<Self, EncodingError>;
encoding.glyph_name(code: u8) -> Option<&[u8]>;
encoding.lookup_unicode(code: u8) -> GlyphUnicode<'_>;
```

A checked object adapter should report integer overflow/type errors by source array index before constructing `Difference::Code`; do not cast arbitrary i64 to u8. Byte input is intrinsically exactly one code. Retain the raw source byte and glyph name if a later string decoder adds provenance. Store static table names/scalars without per-lookup allocation; Differences names may use owned bytes. Avoid relying on a giant cloned Object array for validation.

Differences grammar and policy:

- The first item must be an integer code 0–255. Each subsequent name maps the current code and increments it. Another integer resets it. Names before a code, non-integer/non-name objects, negative/out-of-range codes and names after code 255 without resetting fail explicitly.
- A name at 255 may validly end the array; the internal cursor may become 256 but cannot index until reset. Use checked arithmetic even though the range is small.
- Repeated assignments have explicit last-assignment precedence, reflecting the encoded sequence. Charge every processed entry and name byte, including overwritten data. Record this policy in tests instead of inheriting it accidentally from container mutation.
- `.notdef` is an explicit undefined glyph slot; retain enough provenance if distinguishing it from an initially unassigned slot becomes necessary. Other unknown names remain named slots with `UnknownGlyphName`, not guessed characters, replacement characters, empty strings or parse failure.
- Parse PDF name escapes through the accepted lexer. Names are bytes, including possible NUL or non-ASCII bytes; do not force them through C strings or lossy UTF-8. Unknown byte names remain unknown. Empty name and over-limit names fail this strict first-unit policy explicitly.
- Suggested caps: 4,096 Difference items, 127 bytes per name, 256 KiB cumulative processed name bytes, 256 retained slots. Check the complete update before publishing a result. Allocation/limit errors leave the original encoding intact.

For S0, use the exact **378-name union of the four tables** as the initial named-Unicode registry. This supplies every predefined entry and Differences using those names. Every other valid glyph name yields UnknownGlyphName. This small declared subset provides useful independently testable behavior for all 51 explicit fallback encodings in the measured corpus without pretending to implement the complete Adobe Glyph List algorithm. It also prevents accidentally importing invalid Unicode from MuPDF's extended TeX name registry. Keep `Unicode(&[char])` shaped for later multi-scalar results even though the first registry contains only single scalars.

## Broader AGL policy and attribution

The pinned `scripts/glyphlist.txt` identifies Adobe Glyph List 2.0, dated September 20, 2002. MuPDF's `glyphdump.py` also reads `texglyphlist.txt` (lcdf-typetools 2.95), keeps first-name precedence, rejects values above U+FFFF while generating its tables, and produces separate single/multiple mapping arrays. Therefore its generated `glyphlist.h` is not simply the unmodified Adobe list. The TeX extension includes explicitly invalid surrogate values such as `altselector;D802`; blindly converting every entry into Rust char is incorrect.

MuPDF's strict exported resolver searches only its generated single-name table and uses integer zero as a missing sentinel. Its loose resolver additionally truncates names to 63 bytes, strips period suffixes and underscore components, special-cases selected f ligatures, parses `uniXXXX`, `u...`, `a...` and decimal names, and otherwise returns U+FFFD. It can return surrogate numbers and has no general one-to-many result. Those are reference compatibility behaviors, not the S0 policy.

A separately gated S1 can use a pinned Adobe AGL dataset and documented Adobe glyph-name algorithm: exact names, suffix handling, underscore component sequences and validated `uni`/`u` forms, preserving every Unicode scalar and never treating UTF-16 surrogate units as scalar values. Decide standard ligature codepoint versus decomposed sequence deliberately; no NFC/NFKC normalization belongs in the basic map. Unknown components cannot be silently removed. Validate U+0000, noncharacters, supplementary codepoints, multiple scalars and ambiguous malformed names. Bound component count, output scalars and parsing work.

The official [Adobe AGL repository](https://github.com/adobe-type-tools/agl-aglfn) supplies AGL/AGLFN resources and links its specification. Its [license](https://github.com/adobe-type-tools/agl-aglfn/blob/master/LICENSE.md) permits redistribution with copyright/conditions/disclaimer retention and prohibits implied endorsement. Before adding a new vendor snapshot, record an exact commit, hashes and the complete upstream notices. AGLFN is a font-production naming subset and is not a replacement for the legacy extraction list. The source subset derived now from pinned MuPDF retains the engine's recorded AGPL-3.0-or-later/Artifex attribution; do not relabel derived files as permissively licensed just because a related upstream dataset is permissive.

S1 expected Unicode sequences should come from independently parsed, pinned upstream data and algorithm examples. Native strict parity is appropriate only for shared exact single-name entries. Native loose output and names affected by TeX extensions, truncation, ligature heuristics or zero-as-missing belong in separate compatibility records, not an expected-value generator.

## Acceptance and later integration boundaries

S0 tests should exhaust all 256 slots of all four tables against the native fixed-table oracle and an independent literal-table generator. Cross-check all 378 named results and undefined slots. Explicitly assert the CP1252/MacRoman distinctions above; Python codec success is never the correctness oracle.

Generate Differences cases with code resets, final slot 255, repeated overrides, nonmonotonic resets, `.notdef`, unknown ASCII/NUL/high-byte names, every malformed item type, negative/256/large integers, leading names, overflow past slot 255 and exact item/name/processed-byte limits. Thousands of overrides to one slot must still consume the work budget. Check transactional failures and bounded lookup/allocation. No Differences are present in measured fixtures, so synthetic independent expectations are essential.

Document/font integration is a later milestone. It must separately resolve resource aliases and indirect encoding dictionaries, cap cycles/cache/work, validate simple versus composite font types, and account for built-in encodings, symbolic fonts, embedded font cmaps, Base-14 substitutions and glyph selection. `pdf-font.c` defaults StandardEncoding only in context-dependent nonembedded/nonsymbolic cases; copying that default into the standalone parser is wrong.

Explicit ToUnicode has precedence. If its parser rejects malformed Unicode, this must stay a diagnostic/error according to an explicit load policy; do not silently switch to WinAnsi or glyph-name fallback. Missing ToUnicode, unsupported mapping dependencies and invalid ToUnicode are distinct states. Type0 Identity-H maps bytes to CIDs, never automatically to Unicode. Successful fallback table lookup establishes neither rendered glyph equivalence nor text advance, positioning, ordering, extraction quality or search boxes.

Pinned implementation references and SHA256 values are preserved in `reference/encoding-table-inventory.json`: `source/fitz/encodings.c`, `encodings.h`, `glyphlist.h`, `scripts/glyphlist.txt`, `texglyphlist.txt`, `glyphdump.py`, `source/pdf/pdf-font.c` and `pdf-unicode.c`. All inspected implementation is from the verified MuPDF 1.28.1 archive.
