# P3 U3 gate probe — opus5, 2026-09-17

Script `scripts/p3/subset-probe.mjs <out>` (mupdf 1.28.1 wasm, save `incremental=no,garbage=yes`). Run: `node scripts/p3/subset-probe.mjs .quad/shared/p3/unit3/probe` from repo root (~3 s). Raw numbers: `.quad/shared/p3/unit3/probe/subset-probe.json`; PDFs + mupdf PNGs in that dir.

## Verdict: **GATE PASS**
Item 1: `/W` advance = stext quad width on every used code, max diff 0 pt (edit-subset 9 codes, cjk 10 codes). Item 2: permutation write extracts correctly in mupdf + pypdf, old hex gone, line renders, pixels outside the line identical, neighbour unchanged.

**§2.4 verdict: `encodeCharacter` returns 0** for a WinAnsi char dropped from a subset simple TrueType (NotoSansHebrew, `Z` → 0 while kept `A/L/P/H` → 1/3/4/2). Keep U2's path; add the test. If forced into the stream anyway, `Z` draws the **.notdef box** (visible tofu), not a blank.

## 1. `/W` advance vs stext quad width (size 28)
| file | codes checked | from `/W` | advance / quad | max diff |
|---|---|---|---|---|
| `p0b/edit-subset.pdf` | 9 (日本語テスト, 隣の行) | 9/9 | 28 / 28 each | 0 |
| `cjk.pdf` | 10 (日本語テスト, 中文测试) | 10/10 | 28 / 28 each | 0 |
| `rtl.pdf` (extra, proportional) | 20 Hebrew-font codes | 20/20 | e.g. ם 18.648/18.648, l 5.796/5.796 | 1e-5 |

All CJK advances are 1000 units, so `rtl.pdf` was added to exercise non-uniform `/W` (both `c [..]` and `c1 c2 w` forms parse; the subset DroidSansFallback `/W` expands to 49 775 codes, so ds2 should store it lazily or only for used codes). The 5 Arabic codes are unmatched because stext reports unshaped base letters, not the presentation forms in ToUnicode; not gated.

## 2. Permutation write on `edit-subset.pdf` (`<1a021b493ebb00f400e700f6>` → `<00f400e700f61a021b49>`, テスト日本)
| check | result |
|---|---|
| mupdf extract new / old | yes / no |
| pypdf extract new / old | yes / no |
| `recover.py <out> 1a021b493ebb00f400e700f6` | ok, raw 0, object hits 0 |
| QL dark px in line rect [72,652.58,240,689.18] | before 16.4 %, after 12.3 % (mupdf render 12.0 %) |
| `verify-edit-region.py before after --rect` | ok; inside diff 28.8 %, outside 0 of 487 050 px differ |
| neighbour 隣の行 rect [72,592.58,156,629.18] | 662 dark px before and after |

## 3. Code in ToUnicode but not drawn (日本語 + 試 `3e83` 験 `4e30`)
Both codes are in the subset's ToUnicode (it still maps the whole font). Text extraction returns `日本語試験`, but the two glyph cells [156–212 pt] have **0 dark px** in QL and mupdf; 日本語 cells 21 %. Blank, not tofu (`subset-tounicode-only.png`). ToUnicode membership is not proof, and extraction would lie about the page.

## 4. Simple subset TrueType (one-off PDF, `addSimpleFont(..., LATIN)` + `subsetFonts()`, `(ALPHA)` → `(ALPHZ)`)
| font | encode full A/Z | encode subset handle A,L,P,H / Z | `/Widths` Z | render of Z cell |
|---|---|---|---|---|
| DroidSansFallback (contract's pick) | 0 / 0 | 0,0,0,0 / 0 | n/a | font has **no Latin glyphs**: ALPHA itself draws one notdef box; moot |
| NotoSansHebrew (has Latin) | 151 / 255 | 1,3,4,2 / **0** | absent (outside FirstChar..LastChar) | notdef box, 10.7 % dark vs A 3.4 % (`simple-subset-noto-ALPHZ.png`) |

So U2's `encodeCharacter` gate already refuses a char absent from a subset simple font. ds2: add the test on a Noto-Hebrew simple subset (DroidSansFallback cannot exercise it). Ceiling: if `encodeCharacter` ever returned non-zero here, the result would be a visible box, not silent blank.

## 5. `rtl.pdf` decode
| run | font | decoded (file order) | code points | in §2.2 ranges |
|---|---|---|---|---|
| `<0017007c00370060>` | F2 NotoSansHebrew | םולש | U+05DD 05D5 05DC 05E9 | all; visual order (final-mem first) |
| `<00090516001b001f004f>` | F3 NotoSansArabic | ﺎﺒﺣﺮﻣ | U+FE8E FE92 FEA3 FEAE FEE3 | all; 5/5 presentation forms (shaped) |
| `<006a00dc…016a>` ×2 | F2 NotoSansHebrew | " PDF4All" | ASCII | none |

`rtl-or-shaping` is honest for both RTL runs. **Note for ds2/fable51:** the two ` PDF4All` runs are Type0 in the *Hebrew* subset font and pass the §2.2 gate, so they become CID candidates (repertoire = the page's codes in that font object: ` PDF4Al` plus ם ו ל ש) — §7 "Latin `PDF4All` runs unchanged" is true only for the F1 Helvetica run; test expectation should say which.
