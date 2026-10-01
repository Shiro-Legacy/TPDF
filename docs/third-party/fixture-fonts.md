# Fixture fonts (`fixtures/*.pdf`)

Owner: ds2. Fonts used to build the text fixtures. The **font files are not committed**:
`fixtures/fetch-font.mjs` downloads them into `fixtures/.fonts/` (gitignored) on the first
generator run and verifies the recorded sha256. Only the *subset* MuPDF cuts out of them is
inside the committed PDFs. Generating a fixture needs the network once; running the tests
(`tests/fixtures.test.mjs`, `npm test`) never does.

Nothing here is a licence decision for the app.

## Fonts

| Font | Licence | Source (verified) | Font sha256 | Licence text |
|---|---|---|---|---|
| Droid Sans Fallback | Apache-2.0, © 2005-2008 The Android Open Source Project | `https://github.com/aosp-mirror/platform_frameworks_base/raw/master/data/fonts/DroidSansFallback.ttf` (3,451,900 bytes) | `21b96a0377f067833a93af3082eb28d4ffab7a8cd46bfd513286f1d64b7b0949` | `droid-sans-fallback-NOTICE.txt`, sha256 `38751245…9561a` (same font as the copy inside `mupdf-wasm.wasm`) |
| Noto Sans Hebrew `[wdth,wght]` | OFL-1.1, © 2022 The Noto Project Authors | `https://github.com/google/fonts/raw/main/ofl/notosanshebrew/NotoSansHebrew%5Bwdth,wght%5D.ttf` (112,640 bytes) | `7ef36a2c3593758cdb622e1bdef4f84523e92fbc3ccc667438dd80ff54c2de88` | `notosanshebrew-OFL-1.1.txt`, sha256 `9b9fe028…b47` |
| Noto Sans Arabic `[wdth,wght]` | OFL-1.1, © 2022 The Noto Project Authors | `https://github.com/google/fonts/raw/main/ofl/notosansarabic/NotoSansArabic%5Bwdth,wght%5D.ttf` (844,676 bytes) | `63111b5b2e074dd48cc67692e0a2726d86ee94c1c37fe8598257b7b4e87e869e` | `notosansarabic-OFL-1.1.txt`, sha256 `07fc70bf…e96` |

Both Noto files are variable fonts; MuPDF's `Font(data)` uses the default instance (wdth 100 /
wght 400), which is the Regular master. Both OFL texts are byte-identical except for the
copyright line, so both are kept.

## Duties

- The subset font programs embedded in `fixtures/cjk.pdf` (Droid Sans Fallback, Apache-2.0) and
  `fixtures/rtl.pdf` (both Noto fonts, OFL-1.1) are redistributable copies. MuPDF writes them as
  `YSMQOZ+DroidSansFallback` and `TEYFQG+NotoSansHebrew` / `IYCFXJ+NotoSansArabic` (deterministic
  six-letter subset prefix, checked in the committed files). Neither OFL text declares a Reserved
  Font Name, so OFL-1.1 clause 3 adds no further rename duty. Apache-2.0 asks for the licence
  text + NOTICE, OFL-1.1 for the licence text and a copyright notice: both are in this directory.
- **Conditional, currently not triggered:** `scripts/build.mjs` copies only `fixtures/hello.pdf`
  and `fixtures/form.pdf` into the app, and `scripts/package.mjs` copies only
  `fixtures/hello.provenance.json` into the licences folder. The three new fixtures are
  development/test inputs and do not ship today. If any of them is ever bundled, the licence
  files above must be shipped with it.
