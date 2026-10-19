# Third-party notices — assembly point (`docs/third-party/`)

> Licence note (October 18, 2026): the Rust engine and the app are now MIT, and no MuPDF code ships. Where this document says engine code is AGPL or MuPDF-derived, or that a build owes AGPL obligations, it records the state before the clean-room relicense (see the Licence section of [`PROVENANCE.md`](../../engine/reference/PROVENANCE.md)).

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
| `droid-sans-fallback-NOTICE.txt` | `Droid Sans Fallback` CJK fallback font in the wasm (Apache-2.0, AOSP); also the licence of the copy a Rust build ships (`assets/fonts/DroidSansFallback.ttf`, served as `fonts/DroidSansFallback.ttf`), which the Tauri app ships under `fonts/` with this notice in its `licenses/` | `mupdf@1.28.1:resources/fonts/droid/NOTICE`, sha256 `38751245…9561a`, Apache-2.0 text + AOSP copyright |
| `notosanshebrew-OFL-1.1.txt`, `notosansarabic-OFL-1.1.txt` | the two Noto fonts subset into `fixtures/rtl.pdf` (OFL-1.1) | google/fonts `ofl/notosans*`, urls + sha256 in `fixture-fonts.md` |
| `fixture-fonts.md` | which fonts the fixtures embed, their urls/sha256/licences, and when the notices above become a shipping duty | generation-time fetch, recorded per font |
| `corresponding-source.md` | AGPL §6 inventory for the macOS package: MuPDF 1.28.1 + jbig2dec (both AGPL-3.0-or-later), verified source URLs/sha256, tag→commit→`gitHead` chain, the wasm rebuild recipe and its hash, jbig2dec/dev pins cross-checked, and what we still cannot provide | fetched + hashed 2026-09-15; every claim tagged |
| `lgpl-relink.md` | *(Electron packages only, retired 2026-10-19; its header says what LGPL code the Rust app's Linux AppImage carries instead)* the LGPL part of the package: `libffmpeg.dylib` (LGPL-2.1-or-later) with path/sha256, Electron v44.4.0 + Chromium 152.0.7977.78 source and build-instruction URLs, how a recipient swaps the dylib, and the liblouis/aggregate caveats | read out of `out/PDF4All-P0-darwin-arm64` + verified release URLs |

**Removed with the MuPDF build of the app (2026-10-17):** the app ships only the Rust engine, and
MuPDF is a test-only oracle (`engine/oracle/`). The records that described only the MuPDF WASM
bundle (`mupdf-wasm-assets.md`, `artifex-icc-profiles.md`, the per-library notices in `lib/` and
`fetch-notices.py`, which fetched them) were deleted; they are in the git history up to commit
9c6b66b. The sections below and `corresponding-source.md` are dated records of that bundle.

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
