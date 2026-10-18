# Sources used by workstream 05

Everything read, run or downloaded while implementing
`engine/src/view/text_unicode.rs`, `engine/src/simple_encoding.rs` and their
generated data files. Nothing outside this directory was read, apart from
running the build tool `tpdf-wt/tools/slot` and the crates.io index and
crate downloads cargo makes on its own.

## Specifications in this tree

- `HANDOFF.md`, `specs/05-glyph-unicode.md` (the spec; section 12, the
  corpus addenda U1-U5, read on 2026-10-18 after commit bb4a708), `specs/01-syntax.md`
  (background, headings skimmed), `specs/OWNER-DECISIONS.md` (T6, T10-T13
  applied), `specs/SPEC-RULES.md`.

## Code in this tree read for interfaces

- The two stubs being replaced, `engine/src/lib.rs`, `engine/src/view/mod.rs`
  (module list, head), `engine/src/view/stext.rs` (stub), `engine/src/view/fonts.rs`
  (head and item list), `engine/src/object.rs` (the `Object` enum),
  `engine/Cargo.toml`, `engine/bridge/Cargo.toml`.
- Callers' use of `simple_encoding` (grep only).
- Tests: `engine/tests/cleanroom_unicode_corpus.rs` (section 12),
  `engine/tests/cleanroom_unicode.rs`, `engine/tests/data/adobe-symbol-encodings.rs`
  (included by the unit tests as an independent check), `engine/tests/view.rs`
  (the Type3 fixture), `engine/tests/simple_encoding.rs`,
  `engine/tests/review_simple_encoding.rs`.
- Vendored hayro (Apache-2.0 OR MIT), to match the interpreter:
  `vendor/hayro-interpret-0.8.0/src/font/cid.rs` (how the interpreter splits
  composite-font codes and reads CIDSystemInfo), the head of
  `vendor/hayro-interpret-0.8.0/src/font/mod.rs`, `Cargo.toml` and the head of
  `PATCH.md`; `vendor/hayro-syntax-0.8.0/src/object/{dict,array,name,string,stream,mod}.rs`,
  `pdf.rs`, `xref.rs` and `data.rs` (public signatures). hayro's own
  generated encoding and glyph-name tables (`src/font/generated/`) were not
  opened.
- `hayro-cmap` 0.1.0 (Apache-2.0 OR MIT, a crates.io dependency): its API
  from rustdoc generated locally (`cargo doc -p hayro-cmap --no-deps`, into
  `engine/target/doc`); its source was not read. A throwaway test printed
  the first bytes of four `load_embedded` CMaps to learn they are a binary
  format (`bcmap`); it was removed.

## Downloaded data (2026-10-18, with curl, into `engine/generate/data/`)

| File | URL | SHA-256 | Licence / notice | Used for |
|---|---|---|---|---|
| `glyphlist.txt` | https://raw.githubusercontent.com/adobe-type-tools/agl-aglfn/master/glyphlist.txt (repository commit 4036a9ca80a62f64f9de4f7321a9a045ad0ecfd6, 2019-10-31) | a3b2f61ced9f3644cc0d4ecde5c59df34ca286c689d9484a43a710a81c466789 | BSD-3-Clause, notice in the file header and in `AGL-LICENSE.md` | AGL table; registry values |
| `zapfdingbats.txt` | https://raw.githubusercontent.com/adobe-type-tools/agl-aglfn/master/zapfdingbats.txt | f6394e3cb8a447e84a1dad75d4baaf2aa7f45dc104faf369f4720e1a774ef2dc | BSD-3-Clause, as above | ITC Zapf Dingbats list |
| `AGL-LICENSE.md` | https://raw.githubusercontent.com/adobe-type-tools/agl-aglfn/master/LICENSE.md | 58147d341e7a34aa2196862395a34d2fd95716c41d5ed26efb59ab0e12f92089 | the licence itself | kept beside the AGL files |
| `symbol.txt` | https://unicode.org/Public/MAPPINGS/VENDORS/ADOBE/symbol.txt | deb78ca840a429311939b9d165890873f71fb23ef223ceeb144a6c6d641a7e52 | Unicode, Inc. notice in the file header (free to copy with the notice attached; T10) | Symbol built-in encoding |
| `zdingbat.txt` | https://unicode.org/Public/MAPPINGS/VENDORS/ADOBE/zdingbat.txt | 2d8128a7280cdd47d93272f13c580d7f31fb34586ff65eaf29c7457c224974c0 | as above | ZapfDingbats built-in encoding |
| `UnicodeData.txt` | https://www.unicode.org/Public/16.0.0/ucd/UnicodeData.txt | ff58e5823bd095166564a006e47d111130813dcf8bf234ef79fa51a870edb48f | Unicode License v3 (`UNICODE-LICENSE.txt`) | presentation-form decompositions (NFKD), assigned code points |
| `BidiMirroring.txt` | https://www.unicode.org/Public/16.0.0/ucd/BidiMirroring.txt | d7afdadd1bbd66f5a663ac0e8f7958f18fd9491fc0bc59ec5877cb82db71db7d | Unicode License v3 | `bidi_mirror` |
| `DerivedBidiClass.txt` | https://www.unicode.org/Public/16.0.0/ucd/extracted/DerivedBidiClass.txt | 71ed943a49c58568d8d92e80ecc2ba2f06e62aee9c8ebb0e6e8bd2c3ed8b180e | Unicode License v3 | BN defaults the crate lacks; full check of `bidi_class` |
| `UNICODE-LICENSE.txt` | https://www.unicode.org/license.txt | e7a93b009565cfce55919a381437ac4db883e9da2126fa28b91d12732bc53d96 | the licence itself | kept beside the UCD files |

Cross-check files the spec names (5.1), downloaded into
`engine/generate/data/crosscheck/` and **not committed** (CP1252.TXT and
ROMAN.TXT carry no redistribution grant); the generator runs the
cross-checks when they are present:

| File | URL | SHA-256 | Used for |
|---|---|---|---|
| `CP1252.TXT` | https://unicode.org/Public/MAPPINGS/VENDORS/MICSFT/WINDOWS/CP1252.TXT | f607ae328b4dff5e9bfef725f5fff0ae23f38797f8a5b95998a0d2735c0e8fad | WinAnsi check (equal through the AGL except Annex D's 8 noted codes) |
| `ROMAN.TXT` | https://unicode.org/Public/MAPPINGS/VENDORS/APPLE/ROMAN.TXT | 18e571645be895e9553ed5c842ea8f65f9c5d3c9ccb43e66e0c33a132ed0d721 | MacRoman check (equal except the 15 math/Apple codes PDF leaves out, 0x7F, 0xCA, 0xDB) |
| `stdenc.txt` | https://unicode.org/Public/MAPPINGS/VENDORS/ADOBE/stdenc.txt | 4bcda13f60f43b79fa403240f3557dc3f8018e80495d61e2b089b2607a372a9d | Standard check (all 149 names equal) |

## Typed from published specifications (`engine/generate/tables/`)

- `annex-d-latin.txt`: ISO 32000-2:2020 Annex D.2 "Latin character set and
  encodings" (the same table as ISO 32000-1:2008 D.2 and PDF Reference 1.7
  Appendix D.2), STD/MAC/WIN columns, typed from the published table; the
  table's notes are applied by the generator. Verified by the three
  cross-checks above and by the spec's counts (undefined 107/48/32).
- `annex-d-expert.txt`: ISO 32000-2:2020 Annex D.4 "Expert set and
  MacExpertEncoding", the MacExpertEncoding column, typed from the published
  table. No public mapping file exists to cross-check it; verified by the
  spec's counts (91 undefined, 378 names over the four tables) and by every
  name being an AGL name with one value.
- `mac-glyph-order.txt`: the 258 standard Macintosh glyph names, Apple
  TrueType Reference Manual, chapter 6, `post` table
  (https://developer.apple.com/fonts/TrueType-Reference-Manual/RM06/Chap6post.html),
  typed from the published list (the page was not fetched in this session).
  Verified by count and by the spec's facts (CIDs 1-3 `.null`,
  `nonmarkingreturn`, `space`; 0x24 `A`, 0x44 `a`).

## Crates

- `unicode-bidi` 0.3.18 (MIT OR Apache-2.0, crates.io; `cargo info` and
  `cargo search` for the version; `unicode_bidi::UNICODE_VERSION` is
  16.0.0). Added to `engine/Cargo.toml` per owner decision T6.

## Standards relied on (no files fetched)

- ISO 32000-2:2020 §9.6.4-9.6.6, §9.7.4-9.7.6, §9.8.2, §9.10, Annex D.
- Adobe Glyph List Specification (as summarised in spec 05, 4.1).
- OpenType specification, `cmap` formats 0, 4, 6, 12, 13 and `post` formats
  1, 2, 2.5, 3.
- Unicode Standard §3.11 (canonical ordering, for NFKD); UAX #9 class names.
