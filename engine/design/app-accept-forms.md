# Acceptance review: fields, field-set, field-add, field-props, field-delete (round 1)

October 16, 2026. Fixes for the acceptance review of the form ops at ed62a16. The verdicts were: `fields` and `field-delete` accepted; `field-set`, `field-add` and `field-props` not accepted. Read this with `app-forms.md`, which is corrected where these fixes change it. Probes are the reviewer's (`tpdf-wt/scratch/accept-forms-r0`), re-run against this branch's build. Corpus files are named by source, path and sha256 prefix.

## Lossy text set a document-wide flag (major: field-set, field-add, field-props)

- **Verified.** `forms::write::refresh` reached the lossy branch for any value, default or option that no fallback font encodes. That branch removed the widget's /AP and set /AcroForm /NeedAppearances. The flag makes the app's view, and every viewer that honours it, redraw every other widget too.
  - Reproduced with p3-needapp (form2.pdf), p12-props-lossy (combo add, props redraw) and p7-scripts (text default).
  - Corpus (corpus-needapp, 195 forms): 65 files and 291 of 1,109 other widgets changed in-app.
  - This broke engine.ts's contract §2.1 ("/NeedAppearances is never written").
- **Fix.** `refresh` now removes only that widget's /AP and writes nothing document-wide. `write::need_appearances` is gone. field-set, field-add, field-props and redaction all write through `refresh`, so all of them are covered.
- **Alternatives rejected.**
  - An embedded font subset (the reviewer's preferred fix) would need a font-program writer and subsetter, and the engine has neither. Adding one is outside a fix round, and new dependencies are not allowed.
  - Keeping the old /AP on a field-props redraw would leave the appearance wrong after the change. After an options change it would show a value that is no longer current. After a rect, maxLen or multiline change it would show the old geometry.
  - A `?` appearance would store the wrong glyphs for every viewer.
- **Difference that remains** (recorded in app-forms.md). The filled widget has no /AP.
  - A viewer that synthesizes widgets without /AP draws the value. MuPDF draws Rust's save exactly as it draws engine.ts's (p2-lossy, 7 scripts).
  - A viewer that draws stored appearances only leaves that one widget empty.
  - The app's view draws `?` for the characters no font has.
- **Follow-up after merge.** The annots fix branch (`p7-fix-annots`, not in this base) adds a re-encoded Latin font, `scripts::Eight::Latin`, for FreeText, and leaves widgets out (`Fonts::latin_extended: false`). If widgets used it too, Polish, Czech, Turkish and Romanian fills would get a stored appearance. That is a one-line change once both branches are merged.
- **Measured after the fix.**
  - corpus-needapp on the same 195 forms: 0 files and 0 of 1,109 other widgets change, in-app or in MuPDF's drawing of the save. Before the fix: 65 files and 291 widgets.
  - p2-lossy, p12 and p7: no /NeedAppearances on either engine.
- **Tests.**
  - Native `a_lossy_text_leaves_every_other_widget_as_it_was`. It covers field-set with Polish, Vietnamese, Hebrew, an arrow and an emoji; field-add of a text default and of combo options; and a field-props rect change on a stored lossy value. Each time it checks that the other widgets' dictionaries and their in-view ink are unchanged, and that no /NeedAppearances reaches the save. Eight undos restore the original state exactly.
  - App test (p3-forms): "text outside WinAnsi gains no /NeedAppearances and leaves every other widget its /AP". The contract was tested with Latin text only before; the new test passes on both engines.
  - **Changed test.** `fills_outside_win_ansi_use_fallback_fonts_or_need_appearances` was added during this rewrite and encoded the old behaviour. It is renamed `..._or_no_appearance`, and its /NeedAppearances assertions now expect the flag to be absent.

## Radio kids without /AP switched on (major: field-set)

- **Verified.** In `ops::fields::set_button`, every widget without an /AP /N states dictionary took the chosen state. A radio group could then end with two kids on.
  - Reproduced with p8 (a mixed group) and p6 (a kid on another page with /AS /A).
  - Corpus: qpdf `qpdf/qtest/qpdf/button-set-broken.pdf` (488dccf17eb7) and `button-set-broken-out.pdf` (ca11263c97ac).
- **The suggested rule was not enough.** The review proposed "Off unless the kid's own /AS already names the state". That still leaves two kids on in a group where no kid has /AP:
  - the first fill writes /AS /Yes on its kid;
  - every kid's on state defaults to `Yes`;
  - so on the next fill, the first kid's /AS names the new state and it stays on.
- **Fix** (`shown_state`).
  - A widget with /AP /N states takes the state when it has it, else Off. This is unchanged and is MuPDF's `set_check`.
  - The widget being filled takes the state, so a check box without appearances can still be checked.
  - Any other radio kid without states is Off.
  - Another widget of a check box follows the box, unless its own on state (its /AP /D key, else its /AS when not Off) names a different one. Same-named check boxes with different export values are exclusive.
- **Difference** (recorded in app-forms.md). In a group whose kids all lack /AP, MuPDF turns on every kid, because it synthesized each one with a `Yes` state. Rust turns on one.
- **Measured after the fix.**
  - p8: the mixed group matches engine.ts. Groups with no /AP end with one kid on (MuPDF: all three).
  - p6: the cross-page group matches engine.ts.
  - Corpus forms differential (corpus-forms, the 288 files with widgets):
    - 3,367 items listed on both engines;
    - 403 fills with identical replies;
    - 245 saves read back with equal values by MuPDF;
    - the two qpdf files now list as engine.ts does after their fills (their combo listing difference went too, since it was the radio kid on the same page);
    - `form-errors.pdf` (1215418133f2) is unchanged: Rust leaves one kid on, MuPDF two.
- **Test.** `a_button_fill_turns_on_only_widgets_with_that_state` covers:
  - a mixed group, including filling the kid without /AP;
  - a group with no /AP, switched twice;
  - a kid whose /AS names another state;
  - a cross-page group;
  - a check box with three widgets without /AP;
  - an exact undo.

## Minors

- **Label on a /Parent cycle (fields): recorded, unchanged.** The listing shows the label the bounded chain inherits (/TU, else /T). engine.ts shows '' because MuPDF's `getLabel` throws. The label is the only thing that identifies such a widget, since its name is '' on both engines. Recorded in app-forms.md: pdfjs `issue12963.pdf`, bd61f8496b77, 16 widgets.
- **Exponent numbers (cross-cutting): routed, unchanged.** Reproduced at this base with p5c-exp: `/Ff 1e20` makes Rust repair, and the save drops the widget and a Square annotation that MuPDF keeps. The cause is the shared lexer and parser, which belong to open/save. The session round's lenient repair parser (`p7-fix-session`) is the place to fix it. No corpus file is affected.
- **Stale view bullet (field-set): fixed in app-forms.md.**
  - The view draws Greek and Cyrillic runs.
  - It draws CJK runs only once the adapter installs the CJK font. Measured with p7-scripts: CJK 1012 px with the font and 0 without (`NO_CJK`), against 1001 on MuPDF.
- **field-delete on a page written into /Kids: fixed.** Before, it was refused with a plain error.
  - Now `unlink_widget` skips the page's /Annots write when the page has no object of its own.
  - The cut of references to the widget then removes its entry. The cut acts on the page dictionary inside /Kids, or on the page's /Annots array object.
  - engine.ts deletes there too.
  - field-add there stays refused, as recorded: the new widget's /P could name no page object.
  - Test: `a_field_on_a_page_written_into_kids_can_be_deleted`. It checks the listing, /Fields, the reopened save, and an exact undo.

## Round 3 minors

October 16, 2026 (branch `app-p8-fix-minors` on 46f1e95). The three minor findings the final review left open on field-set.

- **Lossy Latin text stored no appearance (field-set): fixed.** The follow-up above, now that the annots branch is merged: widget synthesis passes `Fonts::latin_extended: true` (`forms/appearance.rs` `writer`), so Latin letters WinAnsi lacks take `<DA>LAT`, the re-encoded base 14 font FreeText uses. Polish, Czech, Slovak, Turkish, Romanian, Hungarian, Latvian and Lithuanian fills, defaults and options now store an appearance that draws the letters; the app's view no longer shows `?` for them, and viewers that draw stored appearances only no longer leave the widget empty. Still lossy (no /AP, as before): Vietnamese letters outside Latin Extended-A, Hebrew, Arabic, arrows, emoji.
  - Difference recorded in `app-forms.md`: MuPDF embeds a Nimbus Sans subset and (1.28.1, recorded `tx_polish`) places that text's baseline at -3.2 in a 24 pt box, where its own clip cuts the glyphs (the review's 93 px in-app against Rust's full letters). Here the WinAnsi layout's baseline (0.8 size) holds.
  - Tests: native `latin_letters_winansi_lacks_store_a_widget_appearance` (`tests/app_accept_minors.rs`: Polish, Czech, Turkish and Romanian fills store /AP with `HelvLAT`, no `?`, no /NeedAppearances; the reopened save draws ż ł ř ı ș ț, each unlike a `?`; four undos exact). App test "a Polish, Czech or Romanian fill stores an appearance and no /NeedAppearances" (`tests/p8-minors.test.mjs`, both engines).
  - **Changed tests** (they encoded Polish as lossy): `synthesis_matches_mupdfs_recorded_streams` (`tx_polish` is a listed difference with the new reason, `LOSSY` is `tx_arrow` alone), `fallback_fonts_match_mupdfs` (skips both cases MuPDF draws with an embedded program, `EMBEDDED`), `fills_outside_win_ansi_use_fallback_fonts_or_no_appearance` (`Łódź` now expects `HelvLAT`; the lossy case is `a → b`) and `a_lossy_text_leaves_every_other_widget_as_it_was` (Vietnamese, Hebrew, an arrow and an emoji; the field-add default and combo options and widget 9's stored value use text that is still lossy; seven undos).
- **Radio-kid fix had no app test (field-set): fixed.** `tests/p8-minors.test.mjs`: a mixed group (one kid without /AP among kids with /B and /C states; each fill leaves the AP-less kid off, listing and saved /AS agree, undo) and the cross-page variant (page 0's kid without /AP and /AS /A goes Off when page 1's kid is chosen). Both pass on engine.ts and Rust.
- **Check-box widgets without /AP (field-set): recorded.** The rule was described but not among the differences; `app-forms.md` now lists it with the review's two cases (a widget without /AP beside one with /AP /N `On` turns on here and stays off in MuPDF; one with /AS /No turns off here and stays on in MuPDF) and the reason (check boxes with different export values are exclusive).

## Gates

- `cargo fmt --check` passes. clippy `-D warnings` passes on stable and on 1.92.
- `cargo test --release --all-features`: 1,534 passed, 0 failed, 3 ignored, on stable and on 1.92. `app_forms` now has 29 tests.
- The bridge's fmt, clippy and test pass, and the wasm build, `npm run build` and `node tests/run-rust.mjs` run.
- `node tests/run-rust.mjs`: 664 tests, 560 passed, 103 skipped, 1 failed. The failure is the known load-sensitive p3-edit test "/W expansion is bounded".
- `node --test tests/*.test.mjs` on engine.ts: 675 tests, 565 passed, 0 failed, 110 skipped.
