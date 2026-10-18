# Workstream 05 notes

## What changed outside "Your files"

- `engine/Cargo.toml`: `unicode-bidi = "=0.3.18"` (owner decision T6).
  `engine/Cargo.lock` gained its entry. **`engine/bridge/Cargo.lock` was not
  changed** and needs the same entry on its next (unlocked) build.
- `engine/tests/cleanroom_unicode.rs`, `engine/tests/view.rs`: the nine
  `#[ignore = "clean-room: 05§…"]` attributes HANDOFF lists were removed;
  nothing else in them changed.
- New: `engine/generate/` (generator `generate_data.py`, typed tables in
  `tables/`, pinned downloads and their licences in `data/`, a `.gitignore`
  for the uncommitted cross-check files). The spec names
  `engine/tools/generate-text-data.py`; the task asked for `engine/generate/`.
- `NOTICE` should gain the AGL BSD-3 notice and the Unicode License v3
  notice (spec 05, 5); there is no NOTICE file in this tree, so the texts
  sit in `engine/generate/data/AGL-LICENSE.md` and `UNICODE-LICENSE.txt`,
  and the generated files' headers name them.

## Acceptance

- Unit tests in `text_unicode.rs` (30) re-create every spec 05 acceptance
  case at this module's level (N1-N11, the G1-G4 rows, the view.rs Type3
  page, the 500,000-group performance case, hostile inputs, and a
  code-point-by-code-point check of `bidi_class` against
  DerivedBidiClass.txt). All pass. `tests/simple_encoding.rs` (12),
  `review_simple_encoding.rs` (4), `builtin_metrics.rs` (4) and
  `review_builtin_metrics.rs` (1) pass.
- Expected failures, all inside other workstreams' `todo!()` stubs:
  - every test of `tests/cleanroom_unicode.rs` (22, including the eight
    un-ignored ones) and `tests/view.rs` (22, including
    `type3_text_comes_from_encoding_names`): `structured_text` is spec 04's
    stub. They pass at the module level (see above) and should pass once
    spec 04 calls `FontText`, `normalise_extracted` and the rest.
  - `cleanroom_text.rs` (18), `view_text.rs` (8): spec 04;
    `app_accept_edit.rs` (9), `app_edit.rs` (21), `app_forms.rs` (19),
    `app_misc.rs` (13), and lib tests
    `app::ops::scan::tests::numbers_tags_and_routes`: spec 07 or spec 04;
    `app::document::tests::a_value_nested_past_the_parse_depth_is_still_repaired_away`:
    spec 03.
  - Not a stub: `security::tests::passwords_convert_to_pdfdoc_as_measured`
    reads `reference/app-encrypt-20261014/docenc-passwords.json`, which is
    not in the hand-off tree.
- Not run: the corpus gate (8.3; no corpus here) and the WASM build.
- `cargo clippy --all-targets -- -D warnings`: nothing in this
  workstream's files; the remaining errors are dead code in spec 07 stubs,
  `view/fonts.rs` and `view/mod.rs`. `cargo fmt` is clean, and the
  generator's output is rustfmt-stable (every table is `#[rustfmt::skip]`),
  so a re-run gives byte-identical files.

## Spec ambiguities and how they were resolved

1. **3.2 vs the UTF-16 acceptance test.** hayro splits a composite font's
   codes by the shortest prefix its CMap *maps to a CID*
   (`lookup_cid_code`), not by codespace ranges. `UniGB-UTF16-H` has no CID
   for `<D840DC00>`, so hayro splits `D8 | 40DC | 0030 | 02`. The test
   needs one four-byte code. Codespace ranges of predefined CMaps are not
   reachable (hayro-cmap stores them in a binary format and has no
   codespace API), so for the predefined `Uni…-UTF16-…` and `Uni…-UCS2-…`
   CMaps the split follows their UTF-16/UCS-2 codespace (two bytes, four
   for a surrogate pair); every other encoding splits exactly as hayro
   does. For codes those CMaps map, the two splits agree. **For spec 04:**
   hayro's `read_code` needs the same rule (a T1-style patch), or drawn
   glyphs and codes differ for unmapped Unicode codes.
2. **Unicode CMap codes as text.** The test also needs U+20000, which no
   GB1 CID gives through `Adobe-GB1-UCS2`. Added to 3.1 step 2, first: for
   those predefined Unicode CMaps the code itself is read as UTF-16BE text
   (the CMap's definition in §9.7.5.2, Table 116); if that gives nothing
   (an unpaired surrogate), the collection rule follows.
3. **Unmatched bytes** (3.2) read as hayro reads them: one byte, code 0,
   CID 0. A missing or unreadable encoding CMap reads every byte that way.
4. **`/UseCMap` in an embedded CMap's dictionary** is not applied: hayro
   ignores it, and the split must equal the interpreter's. `/WMode` in the
   stream dictionary does win over the program's (spec 3.8), but hayro
   uses the program's; this is the difference the spec says to report.
5. **G2, Helvetica without `/Encoding`, code 0xC6**: the spec's table
   records U+02DC; StandardEncoding has `breve` there (Annex D.2, and
   Adobe's stdenc.txt agrees), so this reads U+02D8, as the MacRoman row of
   the same table already does through the Standard fallback.
6. **Two values for one code** in symbol.txt / zdingbat.txt (space,
   Delta, Omega, mu, fraction): the value without a decomposition in
   UnicodeData.txt, else the lower one. That gives U+0020, U+0394, U+03A9,
   U+03BC (the Greek letters the Symbol font shows, not OHM or MICRO SIGN)
   and U+2044. The acceptance data accepts either value.
7. **Bidi defaults.** `unicode-bidi` 0.3.18 (Unicode 16.0.0) agrees with
   DerivedBidiClass.txt except for 3,835 unassigned code points that the
   file defaults to BN and the crate reads as L (noncharacters, U+2065,
   unassigned tags and variation-selector space). The generator emits
   those 23 ranges from DerivedBidiClass.txt and `bidi_class` checks them
   first; a unit test compares all 1,114,112 code points.
8. **`/Differences` with an out-of-range code**: the names after it are
   dropped until the next valid code (the spec says such codes are
   skipped; continuing at the previous cursor seemed worse).
9. **3.5 subtable choice** follows the spec's order literally; a (3,10)
   subtable is used only as "the first of a supported format".
10. **T11 (symbolic TrueType, unnamed glyphs)**: implemented for a glyph
    other than 0 with no usable `post` name, in a symbolic font, at codes
    0x20-0x7E (read as the ASCII character). Not tuned on the corpus, which
    is not here; glyph 0 still reads U+FFFD.
11. **T12 (embedded CIDFontType2 without ToUnicode)**: the program's
    Unicode `cmap` ((3,10), then (3,1), then platform 0) is inverted;
    a glyph reached from more than one character, or from none, is left to
    the interpreter (`None`). The collection rule still comes first, as the
    spec orders.
12. **Collection**: `/CIDSystemInfo` first; if it names no collection with
    a UCS2 CMap (missing, or Identity), the encoding CMap's own collection
    (for example `90ms-RKSJ-H` gives Japan1). Adobe-KR: hayro-cmap has no
    KR family, so it has no UCS2 reading.
13. **"Embedded"** means the descriptor has `/FontFile`, `/FontFile2` or
    `/FontFile3`, or the caller passed a program. An embedded font whose
    program is not a glyf-based sfnt (Type1, CFF, OpenType/CFF) is left to
    the interpreter for codes without a usable name.
14. **Glyph names**: `.notdef`, and names that resolve to nothing, fall
    through to the built-in encoding of non-embedded fonts (3.1 step 3).
    For a ZapfDingbats font the ITC list is consulted before the AGL (AGL
    specification order).
15. **ToUnicode**: a block also ends at a stray `begin…` keyword; a bfrange
    destination whose last unit would pass U+FFFF gives no text for that
    code; destinations longer than 1,024 bytes are skipped as malformed;
    at most 2^20 entries are kept.
16. `text_unicode.rs` carries `#![cfg_attr(not(test), allow(dead_code))]`
    until spec 04's `stext` calls it.

## Points for the reviewer

- Annex D tables and the Macintosh order were typed from the published
  specifications (not fetched here). WinAnsi, MacRoman and Standard match
  the three vendor files; MacExpert has no public file to compare with and
  rests on the counts and AGL membership.
- The spec 04 implementer must know items 1, 2 and 4 above.
