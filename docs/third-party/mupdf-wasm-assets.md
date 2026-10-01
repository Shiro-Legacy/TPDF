# `mupdf-wasm.wasm` — build recipe and embedded third-party assets

Audit of the exact artefact we ship: `node_modules/mupdf/dist/mupdf-wasm.wasm`,
10,409,826 bytes (pin `mupdf@1.28.1`).

Evidence tags: **[V-bin]** read from the shipped wasm locally; **[V-recipe]** upstream build
script fetched from the matching tag; **[?]** uncertain, listed under "Honest limits".

## 1. Build recipe (AGPL §6 Corresponding Source reference)

Upstream `platform/wasm/tools/build.sh` at tag `1.28.1` — byte-identical to `master` (verified
by diff today) — with the defaults actually used by the documented build path:

| Setting | Value |
|---|---|
| emsdk | `4.0.8` |
| `BUILD` | `small` |
| `DEFINES` | `-DTOFU -DTOFU_CJK_EXT -DFZ_ENABLE_HYPHEN=0` |
| `FEATURES` | `brotli=no mujs=no extract=no xps=no svg=no` |
| link flags | `-Os -g2 --no-entry -fwasm-exceptions -sMODULARIZE=1 -sEXPORT_ES6=1 -sALLOW_MEMORY_GROWTH=1 -sFILESYSTEM=0`, libs `libmupdf.a` + `libmupdf-third.a`, then `terser` |

Upstream also states the library is dual-licensed: "MuPDF.js is available under Open Source
AGPL and commercial license agreements" (`node_modules/mupdf/README.md` [V-tree]); we take the
AGPL branch, consistent with the app being AGPL-compatible.

## 2. Embedded font programs [V-bin]

The wasm is **not** stripped of its name section, so this is read from the binary:

- **15 URW CFF fonts** (all upstream `resources/fonts/urw/*.cff`; family names all present):
  `NimbusSans{,-Bold,-Italic,-BoldItalic}`, `NimbusRoman{…}`, `NimbusMonoPS{…}`, `Dingbats`,
  `NimbusBoxes-Regular`, `StandardSymbolsPS`. Copyright strings `(URW)++ Design & Development`.
  Licence: **SIL OFL 1.1** → `urw-fonts-OFL-1.1.txt` in this directory.
- **`Droid Sans Fallback`** (CJK fallback). Licence: **Apache-2.0**, AOSP →
  `droid-sans-fallback-NOTICE.txt` in this directory.
- Upstream `resources/fonts/{noto,sil,han}` (Noto*, CharisSIL, SourceHanSerif) show **0** string
  hits in this wasm → not shipped by this build. Any engine rebuild that enables them adds new
  OFL obligations.

Consequence: a PDF that uses Base-14 Helvetica (like `fixtures/hello.pdf`) is rendered with the
embedded Nimbus Sans. Expected behaviour, and the reason the OFL notice is ours to ship.

## 3. Embedded ICC profiles [V-bin]

All four upstream profiles are present, with Artifex copyright tags. Full method, residual and an
important label: **this is an audit inference (Artifex copyright + no separate grant ⇒ covered by
MuPDF's AGPL/commercial dual licence), not a verified grant found in writing.** See
`artifex-icc-profiles.md`.

## 4. Libraries compiled in [V-bin] — corrected

Counted from symbol/string runs in the shipped wasm (name section intact). **This replaces the
rev-2 list, which wrongly included MuJS, brotli and cmark-gfm as "compiled in".** Precise checks:
`mujs`/`js_newstate`/`js_parse` → 0 hits; brotli → only the string "brotli compression not
enabled" (a compiled-out path) and `BrotliDecode`; `cmark_parse_document` → 0 hits. Those were
my earlier regex over-matching `js_`/`JS_` noise.

| Library | Evidence in the wasm | Upstream licence | Notice file |
|---|---|---|---|
| harfbuzz | 621 refs, e.g. `hb_ot_shape_plan_t::add_pause` | MIT | `lib/harfbuzz-COPYING.txt` |
| lcms2mt | 203 refs, e.g. `lcms2mt__cmsWriteUInt32NumberM` | MIT | `lib/lcms2-LICENSE.txt` |
| freetype | `FT_New_Memory_Face`, 195 refs | FTL | `lib/freetype-docs_FTL-TXT.txt` |
| libjpeg (IJG) | `jpeg_read_header`, 96 refs | IJG | `lib/libjpeg-README.txt` |
| jbig2dec | `jbig2_page_out`, 95 refs | **AGPL-3.0-or-later** | `lib/jbig2dec-{COPYING,LICENSE}.txt` |
| openjpeg | `opj_decode failed`, 199 refs | BSD-2-Clause | `lib/openjpeg-LICENSE.txt` |
| gumbo | `gumbo_parser_allocate`, 27 refs | Apache-2.0 | `lib/gumbo-COPYING.txt` |
| zlib | `inflateInit2_`, 20 refs | Zlib | `lib/zlib-LICENSE.txt` |
| brotli | only the "not enabled" message | MIT | `lib/brotli-LICENSE.txt` (defensive) |
| cmark-gfm | **61 distinct `cmark_*` symbols** (`cmark_arena_push`, `cmark_iter_new`, `cmark_inline_parser_*`) | MIT | `lib/cmark-gfm-COPYING.txt` |
| MuJS | **not compiled in** — the `js_*` strings are MuPDF's own JS *device bridge*, see below | — | not shipped |

### MuJS: how the false positive happened (and why it is settled)

The rev-2/rev-3 lists read 32-33 `js_*` strings as "MuJS compiled in". They are not MuJS: they are
MuPDF's JS **device bridge** in `mupdf.js` — 24 distinct `js_dev_*` symbols (`js_dev_begin_group`,
`js_dev_fill_path`, `js_dev_begin_mask`, …) plus 3 `js_stm_*` stream callbacks, `js_regcomp` and
`wasm_pdf_js_supported`. None of MuJS's own entry points exist (`js_newstate`, `js_dostring`,
`js_pcall` → 0 hits). Runtime confirmation on the pinned build: `Document.isJSSupported()`
returns **false** (probe run in a throwaway `/tmp` install, Node 22.23.1), and `mupdf.d.ts` exposes
`isJSSupported()`/`enableJS()` but nothing in our code calls `enableJS`. Conclusion: the artefact
matches the recipe default `mujs=no`; **no MuJS notice is required** unless a future build enables
PDF JavaScript.

My earlier `cmark_parse_document` probe was simply the wrong symbol name — cmark-gfm **is** linked
in. Correcting in both directions came from the same pass: count the library's actual prefix
(`cmark_*`, `js_dev_*`, `jbig2_*`), do not pattern-match on a guessed function name.

No copyleft beyond the AGPL family already imposed by MuPDF: **jbig2dec is AGPL-3.0-or-later**
(Artifex, 62 distinct `jbig2_*` symbols), everything else is permissive — so the compiled set is
harfbuzz, lcms2mt, freetype, libjpeg/IJG, **jbig2dec (AGPL)**, openjpeg, gumbo, zlib, cmark-gfm
plus MuPDF itself. Notice texts for all of them are assembled in `docs/third-party/lib/` with
pinned commits in `lib/PROVENANCE.json`; the npm tarball ships none of them [V-tree].

## 5. OCR: qualified absence (not "impossible")

The earlier claim "OCR is absent, 0 strings" was too strong on its own. The evidence is three
independent, but not equally strong, signals:

1. **[V-recipe, strong]** The tag-`1.28.1` wasm build script enables no OCR configuration at
   all: only `-DTOFU -DTOFU_CJK_EXT -DFZ_ENABLE_HYPHEN=0` are defined. MuPDF's OCR path needs
   Tesseract/Leptonica to be built and linked; the documented wasm build does neither.
2. **[V-bin, medium]** The shipped wasm contains no `tesseract`/`leptonica` strings.
3. **[V-bin, medium]** `dist/mupdf-wasm.d.ts` — generated from the actual build by the recipe's
   own `tools/make-wasm-type.js` — exposes no `ocr` API, so even if a hidden code path existed,
   the JS binding cannot call it.

**Qualification:** signals 2 and 3 are absences, and absences prove only what is *not* visible
in this build. A future rebuild with different flags, or a different engine, could restore OCR.
So the correct statement is: *this pinned artefact, as built by its own documented recipe,
ships no OCR capability and no Tesseract/Leptonica code* — re-audit on every engine or flag
change. It is not a permanent property of the stack.

## 6. Honest limits

- The published tarball does not record the environment used for the released build, so the exact
  `FEATURES`/`DEFINES` environment of Artifex's release run is not directly observable. What the
  artefact itself shows is consistent with the script defaults: no MuJS entry points
  (`isJSSupported()` false), brotli's "compression not enabled" message, tesseract/leptonica
  absent, while harfbuzz/lcms2/freetype/libjpeg/jbig2dec/openjpeg/gumbo/zlib/**cmark-gfm** are
  demonstrably linked in. Treat the recipe as the documented build path and the symbol audit as
  the binding evidence for what shipped.
- Symbol-string counting is a proxy for "compiled in", not proof of link-level presence or of
  version for each library. Per-library *versions* in §4 are therefore not asserted.
