# Notes for the reviewer (workstream 07)

## Changes outside "Your files"

1. `engine/src/app/forms/write.rs`, one match arm (2 lines): a
   `FontSource::Eight` font is now written by `Eight::embed(sink, family)`
   instead of `sink.object(eight.font(..))`. Owner decision W1 (embedded
   subsets with a /ToUnicode) needs a font program stream and a ToUnicode
   stream, and only the `Sink` can allocate streams; `FontSource` and
   `Eight::font` keep their spec signatures (`Eight::font` stays the
   non-embedded dictionary the scanner and the tests use). If you prefer
   the change elsewhere, the arm is the only call site.
2. New generated file `engine/src/app/forms/scripts_data.rs` (included by
   `scripts.rs` through `#[path]`), and the generator
   `engine/generate/forms_scripts.py` with its inputs in
   `engine/generate/data/` (SOURCES.md). The five vendor code-page files there
   are untracked, so fetch them first. Re-run:
   `python3 -I engine/generate/forms_scripts.py`.

## Data that still has to come from outside

- The Unicode files arrived later (Scripts.txt, DerivedCoreProperties.txt
  and UnicodeData.txt, 16.0.0, in `engine/generate/data/ucd/`). The generator
  now requires them. The provisional excerpts are deleted, and the
  regenerated tables equal the earlier ones byte for byte.
- The vendor code-page files are kept on disk but not tracked in git
  (`engine/generate/data/.gitignore`). Their hashes are in SOURCES.md.
- **ZapfDingbats metrics** (the Core 14 AFM) were not on the whitelist
  either. The six Acrobat mark styles (/MK /CA 4 check, l circle, 8 cross,
  u diamond, n square, H star) and the defaults are therefore drawn as my own
  paths, centred and sized to the box. Any other single /CA character is
  shown as its ZapfDingbats glyph, assuming a 0.8 em advance and a 0.35 em
  middle, and clipped to the box. With the AFM, those two numbers could be
  per glyph.

## Tests

Un-ignored: all 17 tests the hand-off lists (4 in `app_forms.rs`, 13 in
`cleanroom_widgets.rs`).

These pass here: `cleanroom_widgets` (10): a_mark_nothing_composes_with_is_lossy,
a_value_is_drawn_up_to_its_first_65536_characters,
ascii_advances_are_the_familys, cjk_fonts_use_adobe_names,
han_font_defaults_to_simplified_chinese,
han_font_follows_explicit_language_tags,
hostile_inputs_give_finite_content_quickly,
multiline_auto_size_is_between_4_and_12_points,
no_break_space_and_soft_hyphen_stay_in_the_da_font,
win_ansi_maps_latin_and_replaces_the_rest. `app_forms` (18), including
`fallback_fonts_are_well_formed`. Unit tests `app::forms::` (13).

These fail here only at other workstreams' stubs. `cleanroom_widgets`: the
other 35, at `view/stext.rs` (structured text, spec 04) or
`simple_encoding.rs` (spec 05). `app_forms`: 13, at `view/stext.rs`,
`view/mod.rs` `PageFrame::new` (spec 04) or `repair.rs` (spec 03):
a_button_fill_turns_on_only_widgets_with_that_state,
a_character_above_the_bmp_extracts_as_itself,
a_cyrillic_letter_koi8_u_lacks_draws,
a_lossy_text_leaves_every_other_widget_as_it_was,
cjk_runs_take_every_common_character, deep_documents_open_edit_render_and_save,
fills_outside_win_ansi_use_fallback_fonts_or_no_appearance,
fills_write_values_appearances_and_undo_exactly, hostile_fields_never_panic,
need_appearances_redraws_every_widget_but_signatures,
synthesis_cases_meet_the_spec, the_view_bounds_its_synthesis_work,
the_view_draws_widgets_without_appearances_and_saves_never_hold_them.

How I checked the rest locally: I temporarily filled `PageFrame::new` (fixed
612 × 792 page) and `pixel_size` and caught the structured-text panic in
the two test files, so rendering ran. Under that shim every pixel and
structure check passed, including the 102 synthesis cases and their ink,
lossiness, /BBox, /Matrix and resource checks; only the text-extraction
asserts failed. The shims were reverted and are not in any commit. Please
run the full set against real workstream 03, 04 and 05 code.

## Spec ambiguities and how I resolved them

- §6.4 says `win_ansi_code` goes through `simple_encoding`, which is a stub
  here (it would make every field panic). WinAnsi is generated from CP1252
  plus Annex D's names for 0xA0 and 0xAD. The table is the same; switching
  back is a one-line change.
- §5.2 + W5: all three layouts are generated alike: characters WinAnsi
  lacks that the URW Helvetica face draws, packed from 0x80 in source order.
  Greek is ISO 8859-7 then CP1253 (73 codes). Cyrillic is CP1251 then KOI8-U
  (102 codes: every CP1251 letter, including Serbian, Macedonian and
  Belarusian ones, plus KOI8-U's ≤ ≥ √ ≈ ⌠ ⌡ ∙). Extended Latin has 125
  codes. Leaving out what WinAnsi has keeps `€` after Cyrillic in the DA
  font (§4.2).
- W1: the engine can only write CFF programs for the Nimbus Sans regular and
  bold outlines (`scan_fonts::outlines`). So embedded fallbacks are Nimbus
  Sans for every family, and bold only for Helvetica-Bold. Times and Courier
  Greek, Cyrillic and Latin letters are therefore sans. The layout measures
  those runs with the embedded face's advances. Each fallback embeds its
  whole layout (ASCII + upper half, about 17–26 KB uncompressed), once per
  operation. Per-text subsets would need `FontSource::Eight` to carry the
  codes used.
- §4.2 Common characters: each gap of Common characters takes the fonts
  that draw it with the fewest font changes, counting the change into the
  classified neighbours on either side (Viterbi). A CJK neighbour's font
  draws any Common character. Ties go, in order, to:
  1. the font before the gap;
  2. the font after it;
  3. the DA font (§4.2: `Привет €` draws `€` in it);
  4. the 8-bit fallbacks, ranked by (review fix 8):
     - how many of the line's characters each already draws by script, so a
       symbol joins a font the text already uses: fewer font resources
       (each embedded fallback is about 20 KB) and one face for the
       phrase;
     - then how many of the gap's characters it draws;
     - then the larger repertoire.

  With today's layouts no non-ASCII character is in two fallbacks, so the
  ranking only matters once layouts overlap.
- §3.2.2 auto size has no cap for single lines. The minimum is 1 pt.
  Explicit sizes are clamped to 0.01..2000 pt. A non-number before `Tf`
  (e.g. `1e-30`, which is not PDF syntax) reads as the default 12.
- §3.2.3: the line pitch is 1.2 em. Auto size steps from 12 pt down by
  0.25 pt to 4 pt. Lines whose top is below the inner box are not written.
- §3.3.2: the selection band is teal, RGB 0.5 0.78 0.8. It spans the
  inner width and one line pitch. It must stay that dark: the kept test
  `need_appearances_redraws_every_widget_but_signatures` counts only pixels
  whose RGB sum is under 600, so a light blue such as 0.70 0.82 1.0 (sum
  643) would fail it. An auto-size list draws at 12 pt. Selected export
  values are a set.
- Option budget (review fix 4): one widget's /Opt reads may take at most
  4,194,304 characters, and its /V selection reads as many again.
  - When the budget runs out, a combo box shows /V itself (§3.3.1).
  - A list box shows the rows read so far.
  - An export read that the budget would cut short is not compared, so
    a cut export can never match by accident.
- §3.4.4: down = background × 0.7, or grey 0.75 without /MK /BG, with /MK
  /AC as its caption when present and not empty (W4). Push captions are not wrapped in
  /Tx BMC: they are not variable text.
- F8: B and I draw the solid border plus a 3-D edge one border width
  inside it. B is the background moved 65% toward white (top-left) and the
  background × 0.55 (bottom-right), with white standing in for a missing
  background. I is grey 0.42 (top-left) and 0.82 (bottom-right). The text margin then starts two
  border widths in.
- Password with Multiline draws one line of `*`.
- `han_font`: an explicit script subtag (`Hans`/`Hant`) wins over the
  region; extlangs such as `zh-yue-HK` read the region.
- CJK descriptors use my own values: Ascent 880, Descent −120, FontBBox
  [−90 −210 1090 960], CapHeight 700, StemV 72, Flags 6 (symbolic, serif).
  /W gives 500 to exactly the CIDs each UTF-16 CMap maps ASCII to, and the
  layout gives ASCII 500 in CJK runs.

## Points for the reviewer

- The write.rs arm (above).
- `Eight::font` remains the spec's non-embedded dictionary. Widgets never
  write it now; FreeText (spec 06) can call `Eight::embed` the same way.
- The shared layout API is `scripts::{encode, lines, shape, wrap, string}`.
  `shape` + `wrap` let a caller try many sizes on one encoding. `lines` is
  the spec's one-call form, unused inside the crate until spec 06 lands.
- Unused interface items kept for the spec (`lines`, `win_ansi`,
  `Metrics::from_widths`) carry `#[allow(dead_code)]`.
- `cargo clippy --all-targets -- -D warnings` stops on findings in
  `view/fonts.rs`, `view/mod.rs` and `view/text_unicode.rs` (other
  workstreams' stubs). Without `-D`, it reports nothing in my files or the
  two test files.
