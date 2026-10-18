# Sources (workstream 07)

Everything the workstream 07 implementation used besides this tree's own
code and the specs in `specs/`.

## Downloaded files (engine/generate/data/)

All fetched with curl on 2026-10-18 from the official homes the hand-off
allows. The generator `engine/generate/forms_scripts.py` reads them and writes
`engine/src/app/forms/scripts_data.rs`.

The five vendor code-page files (CP1252, 8859-7, CP1253, CP1251, KOI8-U)
carry no clear redistribution grant. They stay on disk for the generator but
are not tracked in git (`engine/generate/data/.gitignore`). Fetch them again
from the URLs below and check the SHA-256 before re-running the generator. The
generated tables are committed.

| File | URL | SHA-256 | Licence | Used for |
|---|---|---|---|---|
| `CP1252.TXT` | https://unicode.org/Public/MAPPINGS/VENDORS/MICSFT/WINDOWS/CP1252.TXT | f607ae328b4dff5e9bfef725f5fff0ae23f38797f8a5b95998a0d2735c0e8fad | Unicode terms of use (http://www.unicode.org/terms_of_use.html) | WinAnsiEncoding codes (Annex D equals CP1252's defined codes 0x20..0x7E, 0x80..0xFF) |
| `8859-7.TXT` | https://unicode.org/Public/MAPPINGS/ISO8859/8859-7.TXT | 94f4a1c89ba6b2e1c1f8b30676cb5d15f232dbac36ae4ad20018a65e0f3fb9a3 | Unicode terms of use | Greek layout (first source) |
| `CP1253.TXT` | https://unicode.org/Public/MAPPINGS/VENDORS/MICSFT/WINDOWS/CP1253.TXT | 85d917854926ef09b22abefd19c7b61494e229fa391b648dc332f1fac0879ce3 | Unicode terms of use | Greek layout (second source, W5) |
| `CP1251.TXT` | https://unicode.org/Public/MAPPINGS/VENDORS/MICSFT/WINDOWS/CP1251.TXT | d9d491808bd7956c26ef8ed07fa63015bfab32860720e07d5bf3a64a609927ee | Unicode terms of use | Cyrillic layout (first source, W5) |
| `KOI8-U.TXT` | https://unicode.org/Public/MAPPINGS/VENDORS/MISC/KOI8-U.TXT | 854bad6ab67c3644c7b60758cff3d0f30eb54dc45a354c3e5ce470da4015f042 | Unicode terms of use | Cyrillic layout (second source: the KOI8-U symbols, e.g. ≤ ≥ √ ≈) |
| `glyphlist.txt` | https://raw.githubusercontent.com/adobe-type-tools/agl-aglfn/master/glyphlist.txt | a3b2f61ced9f3644cc0d4ecde5c59df34ca286c689d9484a43a710a81c466789 | BSD-3-Clause, Copyright 2002-2019 Adobe | Glyph names per character (spec 07 §6.3) |
| `aglfn.txt` | https://raw.githubusercontent.com/adobe-type-tools/agl-aglfn/master/aglfn.txt | ed735a9ea58549b4cb2b8e804341548adb25aff43e81001ac7f3917247cacca1 | BSD-3-Clause, Copyright 2002-2019 Adobe | Preferred glyph name (§6.3), WinAnsi glyph names |
| `AGL-LICENSE.md` | https://raw.githubusercontent.com/adobe-type-tools/agl-aglfn/master/LICENSE.md | 58147d341e7a34aa2196862395a34d2fd95716c41d5ed26efb59ab0e12f92089 | (the licence text) | Notice for the AGL data |
| `UniGB-UTF16-H` | https://raw.githubusercontent.com/adobe-type-tools/cmap-resources/master/Adobe-GB1-6/CMap/UniGB-UTF16-H | 3750279409a96fb3a8ce1fbc25eb95bab4ff7ade023e4e0df6bd33258a3a8726 | BSD-3-Clause, Copyright 1990-2023 Adobe | /Supplement (6) and the CIDs ASCII maps to (half-width /W) |
| `UniCNS-UTF16-H` | https://raw.githubusercontent.com/adobe-type-tools/cmap-resources/master/Adobe-CNS1-7/CMap/UniCNS-UTF16-H | 074d175c8c6827184d064f2c48cbe0ba4957bd525fd864184717518acf4db970 | BSD-3-Clause | as above (Supplement 7) |
| `UniJIS-UTF16-H` | https://raw.githubusercontent.com/adobe-type-tools/cmap-resources/master/Adobe-Japan1-7/CMap/UniJIS-UTF16-H | 5cf77ed38c25262dba845676738a887afe45e44425c3dd2489334c4d8c510fcb | BSD-3-Clause | as above (Supplement 7) |
| `UniKS-UTF16-H` | https://raw.githubusercontent.com/adobe-type-tools/cmap-resources/master/Adobe-Korea1-2/CMap/UniKS-UTF16-H | 0a1dc38d8dd4e1f55b5a3ab978a2b091109f88f4d08587bbb7864370cda8cfd5 | BSD-3-Clause | as above (Supplement 1) |
| `CMAP-LICENSE.md` | https://raw.githubusercontent.com/adobe-type-tools/cmap-resources/master/LICENSE.md | 742665db9c8e1bc72603c6d319ca3e90b83bd6d95202f0cd4ef11068a07a9c29 | (the licence text) | Notice for the CMap data |

One request failed (404, nothing saved):
https://raw.githubusercontent.com/adobe-type-tools/agl-aglfn/master/LICENSE
(the file is `LICENSE.md`, fetched above).

## Unicode Character Database (engine/generate/data/ucd/)

Supplied by the coordinator from https://www.unicode.org/Public/16.0.0/ucd/,
downloaded 2026-10-18. Licence: Unicode License v3 (`license.txt`). The
generator requires these files.

| File | SHA-256 | Used for |
|---|---|---|
| `Scripts.txt` | 9e88f0a677df47311106340be8ede2ecdacd9c1c931831218d2be6d5508e0039 | Script classes (spec 07 §4.1) |
| `DerivedCoreProperties.txt` | 39d35161f2954497f69e08bdb9e701493f476a3d30222de20028feda36c1dabd | Default_Ignorable_Code_Point (§4.3) |
| `UnicodeData.txt` | ff58e5823bd095166564a006e47d111130813dcf8bf234ef79fa51a870edb48f | Canonical decompositions (§4.3); letter categories for the extended-Latin layout (§6.2) |
| `license.txt` | e7a93b009565cfce55919a381437ac4db883e9da2126fa28b91d12732bc53d96 | Unicode License v3 notice |

Before these arrived, the first commit used provisional excerpts I typed and
Python's `unicodedata`. Those excerpts are deleted. Regenerating from the real
files gave byte-identical tables.

## Standards and documents (read from knowledge, not fetched)

- ISO 32000-2:2020 §8.4 (graphics state, dash patterns), §9.6 (simple
  fonts, encodings, /Differences), §9.7 (Type0 and CIDFontType0 fonts,
  Table 116 predefined CMaps), §9.8 (font descriptors), §9.10.3 (ToUnicode
  CMaps), §12.5.4 (border styles), §12.5.5 (appearance streams, /Matrix),
  §12.5.6.19 and Table 192 (/MK), §12.7.4.3 (variable text), §12.7.5
  (button, text and choice fields), Annex D (WinAnsiEncoding).
- RFC 5646 (BCP 47 language tags): subtag structure for `han_font`.
- Adobe Technical Notes on the CJK collections, for the non-embedded font
  names the spec lists (STSong-Light, MSung-Light, HeiseiMin-W3,
  HYSMyeongJo-Medium).
- Unicode Standard: canonical composition (as NFC composes), UAX #24.

## In-tree data used

- `engine/src/builtin_metrics_data.rs`: the URW glyph names of Helvetica
  (drawability, §6.3) and, through `builtin_metrics`, advances.
- `engine/src/builtin_outlines*.rs`: glyph extents (the glyph box text is
  placed by, cap height and stem of the embedded descriptor).
- `crate::view::scan_fonts::outlines` (`view/fonts_scan.rs`): the URW Nimbus
  Sans outlines and CFF subset writer the scanner uses, for the embedded
  fallback fonts (owner decision W1). OFL; the notice travels in each
  program.
