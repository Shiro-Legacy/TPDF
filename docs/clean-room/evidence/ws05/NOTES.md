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
15. **ToUnicode**: a stray `begin…` keyword ends the open block and is read
    again, so the next block still maps; a bfrange
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
- Review fix: inverting a format 4 `cmap` (T12) walks the segments once,
  so it is linear in the table (a 256 KB table of 32,766 segments is a unit
  test).

## Section 12 addenda (2026-10-18, after the corpus run)

Implemented U1, U2 and U4; U5 needed no change. U3 is waiting on the
owner: `symbolic_truetype_unnamed_codes_read_as_win_ansi` stays ignored,
and the T11 printable-ASCII option above is unchanged.

- **U1** (Type3): after 4.1 and 4.2, a code with no usable name reads as
  itself at 0x20-0x7E, else U+FFFD. 4.2's control results (`/a16`) are
  still dropped by 3.7 and the code is not printable, so they read U+FFFD
  (U5). `/g65` at 65 and `/a65` at 66 now read `A` and `B`.
- **U2** (embedded TrueType): when the chosen subtable is (3,1), (3,10) or
  platform 0 and the glyph (not 0) has no usable name, the code reads as
  U+00NN through 3.7. This comes before the T11 symbolic option. An
  `/Encoding` that is a stream or anything other than a name or dictionary
  already counted as absent.
- **U4** (T12 inversion): a glyph of several characters reads as the
  lowest outside U+E000-U+F8FF and planes 15-16, else the lowest. This
  replaces item 11's "more than one character → interpreter".
- Unit tests in `text_unicode.rs` mirror every section 12 case
  (`u1_…`, `u2_…`, `u4_…`, `u5_…`); the older ones whose behaviour U1, U2
  and U4 change were updated (Type3 a/g names at ASCII codes, the inverted
  cmap's shared glyph). A test comment no longer names another PDF
  library.
- Results: 36/36 unit tests pass. `cleanroom_unicode_corpus.rs` (U1, U2
  and U4 un-ignored) fails only in spec 04's `structured_text` stub in this
  tree (7 of 7 with `--include-ignored`, all `todo!("clean-room: spec
  04")`). The full suite has the same failures as before, all in the spec
  03, 04 and 07 stubs plus the security test with its missing reference
  file; no new failure. Clippy (`-D warnings`) has nothing in this
  workstream's files, fmt is clean, and the generator's output is
  unchanged.

### Hand-off notes for integration

- U2 and U4 change what `FontText::text` returns for embedded TrueType
  fonts: some codes that gave `None` or U+FFFD now give a character.
  Spec 04 needs no interface change.
- U3 was confirmed later (T11a); see the next section.

## U3 confirmed (T11a, 2026-10-18)

- `TrueTypeReader::reading`: in a symbolic font, a glyph other than 0 with
  no usable name reads as WinAnsiEncoding's character for the code (Annex
  D with its notes, through the registry values the generator emits, then
  3.7). A code WinAnsi leaves undefined reads U+FFFD. This replaces the
  printable-ASCII option (item 10 above). Glyph 0 stays U+FFFD (the
  optional ASCII reading is not taken).
- Order: post name (4.1), then U2 (Unicode subtable: the code as
  U+00NN), then U3. A symbolic font with a (3,1) subtable showing code
  0x85 therefore reads U+2026: U2 gives a C1 control, which 3.7 drops, and
  U3 then gives WinAnsi's ellipsis. A non-symbolic font reads U+FFFD there.
- Tests: `symbolic_truetype_unnamed_codes_read_as_win_ansi` is un-ignored
  (it fails only in the spec 04 stub here). New unit test
  `u3_symbolic_truetype_unnamed_codes_read_as_win_ansi` covers that case
  plus WinAnsi's noted slots, an undefined code, glyph 0 and a non-symbolic
  font. Updated: the post-format test (0x7F now reads U+2022) and the U2
  symbolic case. 37/37 unit tests pass. The full suite has the same
  failures as before, all in the spec 03, 04 and 07 stubs plus the
  security test with its missing reference file; the only addition is the
  newly un-ignored corpus test, inside the spec 04 stub. Clippy and fmt are
  clean for this workstream's files.
- The new 8.3 corpus-gate wording (T14) needs no code.
