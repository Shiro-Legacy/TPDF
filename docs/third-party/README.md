# Third-party notices — assembly point (`docs/third-party/`)

Owner: ds2. This directory holds the notices **we** must ship that are **not** inside the
dependencies we install. The packaging step copies this folder wholesale into the app bundle.

Assembly ≠ acceptance. Nothing here is the app's own licence or a legal review. The app code is
MIT (`LICENSE` at the repo root, decided 2026-09-15 after this folder was first assembled); it is
the AGPL/LGPL/OFL obligations of what we bundle that this directory tracks.

**Renamed 2026-09-30:** the product is now Tandem PDF and the package is `out/Tandem PDF-darwin-arm64/Tandem PDF.app`. The dated records here still say `PDF4All-P0`; the staged notice names they cite (`licenses/PDF4All-MIT.txt`, `PRIVATE-SPIKE.txt`) are unchanged. The IJG sentence is now also in the app's About panel (`src/main.ts`).

## Contents

| Path | Covers | Provenance |
|---|---|---|
| `urw-fonts-OFL-1.1.txt` | 15 URW CFF fonts compiled into `mupdf-wasm.wasm` (NimbusSans/Roman/Mono, Dingbats, NimbusBoxes, StandardSymbolsPS) | `mupdf@1.28.1:resources/fonts/urw/OFL.txt`, sha256 `3b709798…253ca` |
| `droid-sans-fallback-NOTICE.txt` | `Droid Sans Fallback` CJK fallback font in the wasm (Apache-2.0, AOSP) | `mupdf@1.28.1:resources/fonts/droid/NOTICE`, sha256 `38751245…9561a`, Apache-2.0 text + AOSP copyright |
| `lib/*.txt` + `lib/PROVENANCE.json` | the per-library licences for code linked into the wasm | fetched by `fetch-notices.py` at the submodule commits pinned by MuPDF tag 1.28.1 |
| `artifex-icc-profiles.md` | 4 Artifex ICC profiles embedded in the wasm | tag-table (`desc`/`cprt`) extraction — **audit inference, not a verified grant** |
| `mupdf-wasm-assets.md` | build recipe, embedded fonts/ICC, per-library symbol audit, OCR qualification, honest limits | local binary audit + upstream recipe at the pinned tag |
| `notosanshebrew-OFL-1.1.txt`, `notosansarabic-OFL-1.1.txt` | the two Noto fonts subset into `fixtures/rtl.pdf` (OFL-1.1) | google/fonts `ofl/notosans*`, urls + sha256 in `fixture-fonts.md` |
| `fixture-fonts.md` | which fonts the fixtures embed, their urls/sha256/licences, and when the notices above become a shipping duty | generation-time fetch, recorded per font |
| `corresponding-source.md` | AGPL §6 inventory for the macOS package: MuPDF 1.28.1 + jbig2dec (both AGPL-3.0-or-later), verified source URLs/sha256, tag→commit→`gitHead` chain, the wasm rebuild recipe and its hash, jbig2dec/dev pins cross-checked, and what we still cannot provide | fetched + hashed 2026-09-15; every claim tagged |
| `lgpl-relink.md` | the LGPL part of the package: `libffmpeg.dylib` (LGPL-2.1-or-later) with path/sha256, Electron v44.4.0 + Chromium 152.0.7977.78 source and build-instruction URLs, how a recipient swaps the dylib, and the liblouis/aggregate caveats | read out of `out/PDF4All-P0-darwin-arm64` + verified release URLs |

Both top-level notice files are byte-identical between the `1.28.1` tag and `master` (verified by
diff), so they track the shipped build. Re-run `fetch-notices.py` on any engine bump.

## Per-library licences (`lib/`)

Pinned to MuPDF 1.28.1's `thirdparty` submodule gitlinks (the Artifex forks/builds MuPDF actually
compiles, not upstream HEAD). Full record with repo, commit, path and sha256: `lib/PROVENANCE.json`.

| Library | SPDX | Note |
|---|---|---|
| freetype | FTL | `docs/FTL.TXT`; the repo's `LICENSE.TXT` names FTL or GPLv2 as the two options — we take FTL |
| harfbuzz | MIT | old-style MIT text |
| lcms2 (built as lcms2mt) | MIT | Artifex fork |
| libjpeg | IJG | **the README *is* the licence**; no LICENSE/COPYING exists in that repo |
| openjpeg | BSD-2-Clause | |
| gumbo (gumbo-parser) | Apache-2.0 | Apache-2.0 text; no separate NOTICE file upstream, so none is required |
| zlib | Zlib | Artifex fork |
| jbig2dec | **AGPL-3.0-or-later** | same licence family as MuPDF; `COPYING` (AGPL text) + `LICENSE` (explicit statement) |
| brotli | MIT | no symbol traces in the wasm, but the string "brotli compression not enabled" is present |
| cmark-gfm | MIT | no symbol traces; included because MuPDF's docs list it as a required library |

**IJG wording duty:** the IJG licence requires that, when only executable code is distributed,
the accompanying documentation state *"this software is based in part on the work of the
Independent JPEG Group"*. Add that sentence to the app's about/credits screen together with these
files — the licence text alone does not discharge it.

**Not compiled in, so not shipped:** `mujs` (0 symbol traces in the wasm and the build recipe
defaults to `mujs=no`) — if a future build enables PDF JavaScript, MuJS's own licence must be
added first. Same treatment for Tesseract/Leptonica (OCR, excluded by decision m23), curl,
freeglut, zint, zxing-cpp, extract.

## Still not in this directory, and why it cannot simply be a URL

- **Corresponding Source for the shipped wasm (AGPL §6).** An upstream tag URL alone does **not**
  discharge this: we must be able to provide the source *of the build we ship*, together with the
  build instructions/configuration needed to reproduce it (`mupdf-wasm-assets.md` §1 records the
  recipe: emsdk 4.0.8, `BUILD=small`, the exact `DEFINES`/`FEATURES`, libs linked).
  `corresponding-source.md` now inventories that material — the official tarball URL + sha256, the
  1.28.1 tag commit matching the npm `gitHead`, the recipe script's own sha256, and the gap list
  (no source ships with the package, the build has not been reproduced here, the app's own source
  is not assembled). The source packet itself stays gated until there is a real distribution.
- **The app's own source and build instructions**, plus the `node_modules` notices named in the
  inventory (`mupdf/LICENSE`, `electron/LICENSE`, `electron/dist/LICENSES.chromium.html`,
  LGPL-2.1+ ffmpeg wording). Packaging copies those from the installed tree; they are not
  duplicated here.

Private local spikes need no distribution step. Anything handed to a third party or shipped needs
the full set above **and** the source packet.
