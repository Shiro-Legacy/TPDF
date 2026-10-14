# R17 native glyph-selection evidence

This document records the native reference evidence, the originally proposed scope, and the final bounded G0/G1 reference gate outcome below. Production acceptance additionally depends on the separate Rust review/compiler gates. `reference/glyph-selection-inventory.json` independently expands all nine actual format4 subtables, computes all1,280 PDF byte mappings for five embedded TrueType font dictionaries, and matches the pinned native `pdf_font_desc->cid_to_gid` tables exactly. There are71 nonzero PDF code mappings. ToUnicode is not an input to the independent calculation.

| Fixture/font object | Flags/Encoding | Selected record | Nonzero raw cmap / PDF codes | post |
| --- | --- | --- | --- | --- |
| p0b/edit-coded.pdf,5:0 |4 /absent |1: Microsoft3/0 symbol |14 /14 |absent |
| p3/simple-subset.pdf,3:0 |32 /WinAnsi |0: Microsoft3/1 Unicode |4 /4 |2.0: .notdef,A,H,L,P |
| p6/quartz.pdf,6:0 |4 /absent |1: Microsoft3/0 symbol |39 /39 |absent |
| p6/quartz.pdf,10:0 |4 /absent |1: Microsoft3/0 symbol |1 /1 |absent |
| p6/quartz.pdf,14:0 |32 /MacRoman |0: Macintosh1/0 |12 /13 |absent |

All selected and unselected records are format4. The symbolic records map U+F000+byte, while their Macintosh alternatives map the unshifted byte. The native font selection must therefore be separated from raw cmap lookup. Font3:0 maps A/H/L/P to GIDs1/2/3/4. The last font additionally maps PDF byte202 to GID106: its PDF encoding glyph name is `space`, whose native reverse MacRoman lookup selects byte32. Treating PDF byte202 as a charmap index would lose this glyph.

## Pinned native priority and fallback rules

The relevant source is MuPDF1.28.1 `source/pdf/pdf-font.c`, especially `select_truetype_cmap`, `pdf_load_simple_font`, `lookup_mre_code`, and `ft_find_glyph_by_unicode_name`; helper behavior is in `source/fitz/font.c`, `ft_char_index` and `ft_name_index`.

1. Effective symbolic status is Flags bit4 unless bit32 is set; bit32 takes precedence when both appear. For TrueType, select the first Microsoft3/0 record when symbolic; otherwise/the next choices are first Microsoft3/1, then Macintosh1/0, then the first native charmap. Native synthetic records and other formats need explicit treatment; a format4-only implementation must refuse a selected unsupported higher-priority record rather than silently choose a lower-priority format4 record.
2. Initialize every byte's GID with the selected cmap lookup at that byte, then U+F000+byte if the first returns0. MuPDF's helper also falls back U+22EF to U+2026; this does not occur in the five actual mappings but can matter for glyph-name Unicode queries.
3. PDF Encoding supplies glyph names independently of ToUnicode. For nonsymbolic Microsoft cmaps, a glyph name first gets a strict Adobe-name Unicode lookup, then a nonzero cmap GID; a failure tries font glyph names and finally fuzzy Unicode lookup. Only a **nonzero** result overrides the initial byte mapping. Returning0 immediately when a name override fails changes native behavior.
4. Nonsymbolic Macintosh cmap selection maps the PDF glyph name to the native MacRoman index. Native uses16 explicit correction names before scanning its MacRoman name table from index0; first matching names matter (`space` selects32). A positive index is looked up through the same cmap helper; font-name fallback follows. Again, failed overrides retain the original byte GID.
5. Symbolic Microsoft3/0 cmaps do not use PDF glyph-name overrides. Other symbolic charmap kinds have font-name behavior and should remain outside the first bounded policy unless separately gated.
6. `ft_name_index` is broader than exact post-name lookup: it tries the requested name, same-Unicode duplicate names, then a generated `uniXXXX` name. Strict Unicode, exact post name, alias name, and fuzzy Unicode can conflict; preserve that priority. A minimal post2 parser that only tries the requested name does not implement the full fallback rule.

The existing S0 named encoding tables contain378 names with strict single-codepoint mappings. Restricting initial PDF support to those names avoids arbitrary fuzzy-name parsing, but does not automatically eliminate post-name alias fallbacks when the Unicode cmap misses. The actual simple-subset font's four live glyph names resolve directly through its Unicode cmap. A defensible first implementation can preserve per-code `UnsupportedNameFallback` when that route fails and post could change the answer, while still accepting the four known used codes. Whole-font preparation should not convert a per-code unsupported route into an invented `.notdef` result.

## Missing glyphs, provenance and outline bridge

Native GID0 denotes `.notdef`. The accepted narrow Rust G1 policy represents that result as explicit `Missing` with null GID and no outline decode; it does not draw or claim parity for native glyph0. Decoding/rendering `.notdef` through O0 would require a separate policy expansion. The source-code result remains present rather than being silently discarded. A GID must be checked against maxp even after delta wrapping, and a mapping record should preserve font identity, selected raw cmap record, PDF source code, lookup codepoint, route, and any name override. Keep PDF advance widths and extracted Unicode separate from GID selection and font-program hmtx; neither is a substitute for the others.

All five original fonts have usable raw O0 programs, but their mere mapping availability does not make the Quartz page executable: PP1 still refuses its named-color-space operators. A glyph-selection gate should test resource-level code/GID and outlines without relabeling the whole page supported.

## Bounded native gates

`tools/mupdf-truetype-cmap-oracle.c` links only pinned bundled FreeType for standalone sfnt queries. It emits every one of65,536 direct 16-bit character-code lookups per original format4 record and validates native record identity/count against the raw directory. It applies no PDF byte-offset or encoding heuristic. Limits:32MiB input,128MiB native heap,64MiB output,128records,10CPU/15wall seconds. All nine actual subtables must be compared exhaustively to the independent expanded-segment model and Rust; generated rational integer mappings cover delta/range-offset/zero/wrap/boundary cases. Malformed native recovery must never justify Rust acceptance.

`tools/mupdf-glyph-inventory-oracle.c` opens a bounded original PDF, selects one explicitly identified embedded simple TrueType dictionary, and captures the native256-entry code/GID table plus selected charmap and cmap inventory. It rejects fallback/substitution, encryption, warnings and repair. Limits:16MiB input,128MiB native heap,2MiB output,16native charmaps,10CPU/15wall seconds. The five complete tables match the independent inventory, including unused bytes. This is test-only; production retains no MuPDF/FreeType runtime dependency.

Required PDF adversarial/reference cases include both symbolic flag bits, reordered/conflicting cmap records, unsupported selected formats, source-byte versus F000 conflicts, PDF Differences versus base names, Unicode versus post conflicts, duplicate post-name aliases, direct-glyph fallback after a failed override, `.notdef`, out-of-range GIDs, authoritative ToUnicode deliberately unrelated to glyph identity, and repeated/aliased resources under cumulative budgets.

## Source and licensing evidence

MuPDF policy code and its existing encoding/name data are AGPL-3.0-or-later. Reference-only FreeType carries its own bundled license. No embedded font program is added to production sources. `tools/build-glyph-selection-reference.sh` verifies source hashes before linking, and the inventory records all source/report/binary hashes.

| Source | SHA-256 |
| --- | --- |
| source/pdf/pdf-font.c |c0c29c2826883e47191133f66b7d9fb769116f9923419606ac4f7136f54feca5 |
| source/fitz/font.c |7fe9aa150fd9ac515e769f2213570b9e696a5650bdb7cd2a62a2bef35a48b6e6 |
| source/fitz/encodings.c |db046231a23e4470c89e31850cff8670a1ca95def64c71d0d4f4c416622fe4a6 |
| source/fitz/encodings.h |224122c5581a91254a22d3a009c50322d39fbbac5f1b2c1c5464cba8528839de |
| source/fitz/glyphlist.h |16fe0427189e16ff852c02984a49dba184c0db51756b8ecef01524ad6efb6002 |
| thirdparty/freetype/src/sfnt/ttcmap.c |f7288c09bf7b64381ebc794df9236d99f2d378043d89ab477b6e5be697f8a00a |
| thirdparty/freetype/src/sfnt/ttpost.c |9d8b3d635e819e2278d0cab60641b95f6795e0b19877e42b3eb130c8f706180c |
| COPYING (MuPDF) |57c8ff33c9c0cfc3ef00e650a1cc910d7ee479a8bc509f6c9209a7c2a11399d6 |
| thirdparty/freetype/docs/FTL.TXT |5a5ee54c5001bbad1cdc1a57cc3dd4c42199b2da09d39c7ee41fab002d02967f |

## Final bounded reference-gate outcome

The final Rust binary and native/model comparisons pass. `reference/truetype-cmap-differential-g0.json` records all15,007,744 direct 16-bit character-code queries:229format4 records comprising nine records from the five actual embedded fonts and220generated fonts. Every integer mapping and selected-record byte range matches; no numerical tolerance or native malformed-recovery equivalence is used.

`reference/glyph-selection-differential-g1.json` records all1,280 PDF source-code slots:71mapped,989explicit Missing,220unsupported name-fallback results. All71native nonzero mappings are covered, including MacRoman byte202; each accepted route/lookup codepoint matches the independent inventory, and all71corresponding O0 outlines match native points, contour endpoints and1,544doubled-coordinate path commands. The220unsupported slots are all conservatively unresolved name fallback in simple-subset and are not reported as Rust support. Native GID0 is compared to the explicit Missing classification only, without a glyph0-rendering claim.

Fifteen native wrapper/routing checks pass. `reference/glyph-comparator-validation-g1.json` records two valid baselines and13rejected semantic mutations covering missing/reordered records, record identity, changed code/GID, missing queries/slots, changed outline points/commands, and route/codepoint provenance. Fifteen compressed raw PDF/native/outline outputs are retained at `/private/tmp/tandem-r17-glyph-evidence` for review; the reports pin their uncompressed hashes. The frozen binary SHA-256 is `c0f68f4b603b172a7fef92167376fdb2c0c7165477e0cc3e25cb5bcce7f5cdaa`.

These gates establish the bounded mapping and unhinted-outline bridge. They do not expand PP1 Quartz operator support, implement generalized post-name fallback, draw glyph0, execute hinting, render text pixels, or integrate the engine into the application.
