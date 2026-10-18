# Sources used for workstream 04

No web page, no other repository and no other PDF tool was consulted.
Everything below is a public standard or data known from it, or code in
this tree.

| What | Where from | Used for |
|---|---|---|
| ISO 32000-2:2020 | the standard (as cited in spec 04) | §7.7.3.3–7.7.3.4 page attributes and inheritance, §7.9.2.2 text strings, §7.9.5 rectangles, §7.9.7 number trees, §9.3.6 text rendering modes, §9.4.4 glyph displacement, §9.7.4.3 vertical writing, §9.7.5.2 Table 116 (predefined Unicode CMaps and their codespaces), §9.8.1–9.8.2 font descriptor entries and /Flags bits, §14.7.5.4 parent tree and MCIDs, §14.9.4 ActualText, §14.11.2 page boundaries and UserUnit |
| Unicode Standard Annex #9 (Unicode Bidirectional Algorithm) | unicode.org/reports/tr9, the published rules W1–W7, N1–N2, I1–I2, L1, L2, L4 (from knowledge of the standard, not fetched) | `stext::reorder`, `resolve_levels` |
| Unicode `White_Space` property and block ranges (Blocks.txt: CJK, kana, Bopomofo, halfwidth and fullwidth forms) | Unicode Character Database, through Rust's `char::is_whitespace` and the block boundaries | search whitespace; scripts without inferred spaces |
| OpenType specification: `head` (unitsPerEm, macStyle), `hhea` (ascender, descender), `OS/2` (fsSelection, sFamilyClass, sTypoAscender, sTypoDescender), `post` (isFixedPitch) table layouts | Microsoft/Adobe OpenType specification (from knowledge of the published table layouts, not fetched) | font facts of embedded programs (spec 04, 3.3) |
| Adobe Core 14 AFM files, `FontBBox` lower and upper values of the 14 standard fonts | Adobe's published Core 14 AFM metrics (from knowledge of the published files, not fetched; the reviewer should check the 28 numbers in `stext::standard_extent`) | ascender and descender of non-embedded standard fonts |
| Spec 04 experiments E2–E4 (recorded in the spec) | `specs/04-text-extraction.md` | choosing the space gap (0.15 em) and block gap (1.55 em) inside the spec's bands |
| hayro, hayro-interpret, hayro-syntax 0.8.0 (vendored, Apache-2.0 OR MIT) | `engine/vendor/` in this tree | the interpreter API, and the text-extraction patch (`vendor/hayro-interpret-0.8.0/PATCH.md`) |
| `unicode-bidi` crate | already in `Cargo.toml` (workstream 05) | not used directly: spec 05's `bidi_class` wraps it |
| Spec 04 section 12 (C3, C4) and its experiment E9 | `specs/04-text-extraction.md` | line order and joining of overlapping runs; the pixel tolerance ε = 1.02e-3 inside the pinned [0.00101, 0.00104) |
| Spec 04 section 12, addenda C5, C5a and C6 (hand-off commits 0bb1334 and 82a7582) | `specs/04-text-extraction.md` | front joins decided by the whole run within 0.10 em; a copy of a glyph of several characters merged character by character |
| A rotate-xor-multiply hash, as in the rustc-hash/FxHash design (MIT/Apache-2.0) | general knowledge of the design, not copied from its code | `stext::Mix`, the overprint grid's hasher, seeded per grid from `std::collections::hash_map::RandomState` |
