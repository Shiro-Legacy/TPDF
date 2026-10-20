# Fonts the app ships

## `DroidSansFallback.ttf`

The CJK fallback font of a Rust build. The Rust engine draws Chinese, Japanese and Korean text whose font a PDF does not embed (and the app's own CJK form values) with it; MuPDF carries the same font inside `mupdf-wasm.wasm`, so a MuPDF build does not ship this copy.

| | |
|---|---|
| Font | Droid Sans Fallback, © 2005-2008 The Android Open Source Project |
| Licence | Apache-2.0: `docs/third-party/droid-sans-fallback-NOTICE.txt` (the licence text with the AOSP copyright, as MuPDF ships it) |
| Source | `https://github.com/aosp-mirror/platform_frameworks_base/raw/master/data/fonts/DroidSansFallback.ttf` (the URL and pin of `docs/third-party/fixture-fonts.md`, which `fixtures/fetch-font.mjs` fetches for the fixtures); the font MuPDF 1.28.1 draws CJK with (`engine/design/app-view.md`) |
| Size | 3,451,900 bytes (1,650,174 gzip -9) |
| sha256 | `21b96a0377f067833a93af3082eb28d4ffab7a8cd46bfd513286f1d64b7b0949` |

Unmodified. `scripts/cjk-font.mjs` pins the hash: the mobile build (`mobile/scripts/build.mjs`) and the test build (`scripts/build-test.mjs`) refuse to build when the file is missing or differs and copy it beside the engine worker (`fonts/`) with its licence; the desktop app ships it in its resources (`fonts/`) with the licence among its licence texts. Why it is committed rather than fetched at build time: `engine/design/app-corpus-cjk.md`.
