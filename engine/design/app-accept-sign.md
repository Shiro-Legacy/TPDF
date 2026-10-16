# Acceptance review: sign-list, sign-prepare (round 1)

October 16, 2026. Fixes for the acceptance review of these ops at ed62a16. Verdicts: sign-list accepted, sign-prepare not accepted (one blocker, two minors). Read with `../APP-ENGINE-DESIGN.md` and `app-sign.md`, which is corrected where these fixes change it.

## Shared /Annots (blocker, sign-prepare)

- **Verified.** `prepare.rs` `annotate` pushed the new widget onto the array object behind the page's /Annots. Pages sharing that array all listed the widget and drew it. When the array was also /AcroForm /Fields, `form` pushed the widget a second time. The new tests reproduced both cases on ed62a16.
- **Fix: `annotate`.** The page always gets a direct /Annots array: its current entries plus the widget. This is what MuPDF's `createAnnotation` writes: measured, its update rewrites the page and leaves the array object alone even when no other page shares it. The array object stays as it was for every other holder (pages, /Fields, anything else), so no sharing analysis is needed. A missing or non-array /Annots becomes `[widget]`, as before.
- **Fix: `form`.** /Fields is still updated in place, as engine.ts's `fields.push` does. The exception is a /Fields array object that is also some page's /Annots. That form gets a direct /Fields copy holding the widget (`annots_of_a_page`, one `info::pages` walk).
- **Difference (better), recorded in app-sign.md.** MuPDF pushes onto that shared array, so its widget also shows on the other pages, and pyHanko fails on MuPDF's chains ("Error in diff operation between revision 0 and 1"). Rust's chains verify INTACT and docmdp-ok.
- **Why pages always get a copy.** The annots fixer (p7-fix-annots, `annots::own_annots`) copies only an array that another page shares. Signing follows MuPDF exactly instead: the update objects stay MuPDF's (the review's p3 object-level probe: 540/540 identical), and every other holder of the array is covered. pyHanko accepts the page rewrite under DocMDP 2 (chains D and D2 below).
- **Tests** (`tests/app_sign.rs`), each failing on ed62a16:
  - `a_new_widget_on_pages_sharing_an_annots_array_lands_on_its_own_page_alone`:
    - full save: three pages share one array holding a note; the visible signature on the middle page is on that page alone, the shared object is unchanged, and sign-list reports page 2;
    - incremental: a signed source whose two pages share an array holding the signed widget; the update writes the page and not the array, and sign-list puts the new signature on page 1.
  - `a_new_widget_goes_once_into_an_annots_array_that_is_also_the_form_s_fields`:
    - /Annots is /Fields: certify, then a second signature as an update; each widget is listed once in /Fields and once on the page, and sign-list lists each signature once;
    - two pages share the /Fields array: the widget is on the signed page only, and the shared object is unchanged.
- **annot-add** has the same root behaviour. It is fixed on the annots branch (p7-fix-annots: `annots::append`, plus link-add). The fixes do not overlap: this one touches only `ops/sign/prepare.rs`.

## Minors

- **'original startxref' (sign-prepare): fixed.**
  - Cause: `incremental::startxref` searched the final 1 KiB, while the strict reader skips trailing whitespace (NUL included) before %%EOF. A signed file with more than 1 KiB of NULs after %%EOF opened without repair, but could not be signed.
  - Fix: the writer now searches the final kilobyte before the trailing whitespace, so the reader and the writer agree on every file the strict reader opens.
  - Text after %%EOF is not whitespace. More than 1 KiB of it also hides startxref from the rebuild, and that file gets the documented "the signed file needs repair" refusal (tested).
  - Recorded as a better difference in app-sign.md. MuPDF repairs such a file and refuses ("Can't do incremental writes on a repaired file"). On a real signed hello.pdf plus 3,000 NULs, Rust's output passes three checks:
    - MuPDF opens it without repair (countVersions 2);
    - pyHanko, which cannot read the source ("EOF marker not found"), reports both signatures INTACT, trusted and docmdp-ok, the first ENTIRE_REVISION;
    - the review's p6-synthetic S7_nul3000 now signs (verdicts valid; MuPDF reopens it without repair).
  - Tests: `a_signed_source_with_a_long_blank_tail_takes_an_update_after_it`, which also checks the reopened output (not repaired, versions 2, two signatures) and the over-1-KiB text refusal. The `startxref_reads_the_last_keyword` unit test gained NUL-tail and all-blank cases.
- **Stale 'App tests on Rust' in app-sign.md: fixed.** The section now gives the current outcomes: p5-sign 9/9, p5-verify 3/3, p5-sign-field 14/14 (not 4/14), p2-shell 14/14 and rust-engine-sign-wire 3/3; p5-drive-field's 3 tests are skipped without Electron.

## Measured

The review's probes and new chain probes ran through the adapter on this build, with engine.ts on mupdf.js 1.28.1 beside it (scratch `tpdf-wt/scratch/fix-sign`):
- p6c and p6d: a widget per page and blue appearance pixels match MuPDF on both the unsigned and the signed shared layouts: `Signature1 / - / -` and 16080/0/0, where Rust showed `Signature1` on every page and 16080/16080/16080 before. sign-list puts the new signature on page 1.
- p6b: the update rewrites page 3 with a direct `/Annots[5 0 R 13 0 R]` and leaves object 12 alone, as MuPDF does.
- p15: /Annots-is-/Fields outputs are byte-identical to MuPDF's objects. The widget is listed once, and sign-list lists it once.
- Chains, each saved and checked with pyHanko (`scripts/verify-sign.py`):
  - A1 and A2 (shared /Annots: certify then sign; three visible signatures), B and B2 (/Annots is /Fields), D and D2 (unshared indirect arrays): every signature is INTACT, trusted and docmdp-ok on both engines, and the widgets per page are identical.
  - C and C2 (/Fields shared with two pages' /Annots): Rust's signatures verify INTACT and docmdp-ok, and each widget sits on its own page. MuPDF's widget shows on both pages, and pyHanko fails to diff MuPDF's revisions.
- Corpus (counts only): qpdf `qpdf/qtest/qpdf/form-fields-and-annotations-shared.pdf` (10ff29590d32), `rotated-shared-annotations-1.pdf` (600c0ac18588) and `rotated-shared-annotations-2.pdf` (5fb093766389). A visible signature on each page lands on that page alone, as MuPDF's does; the per-page results match MuPDF's, refusals included.
- p3-prepare-objects: 540/540 identical at object level, as before.
- p6-synthetic: 252 identical. The 21 differences are the recorded ones: U7/U8, U22, S6, S16, and now S7, which signs where MuPDF refuses.

## Gates

All under memcap 3000, with CARGO_BUILD_JOBS=2 and RUST_TEST_THREADS=2. `engine/bridge` is untouched.
- `cargo fmt --check`: passes.
- clippy `-D warnings`: passes on stable and 1.92.
- `cargo test --release --all-features`: 1,534 passed, 3 ignored, on stable and on 1.92. `app_sign` runs 25 tests, 3 of them new.
- The WASM build and `npm run build` succeed.
- App on Rust (`node tests/run-rust.mjs`): 663 tests, 559 passed, 103 skipped, and 1 failure: the known load-sensitive p3-edit "/W expansion" timing test. Run alone, p3-edit passes 32/32.
- Default suite (`CI_NO_ELECTRON=1 node --test tests/*.test.mjs`): 674 tests, 564 passed, 110 skipped, 0 failed.
- No app test was edited.
