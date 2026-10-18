# Workstream 06 notes for the reviewer

## Acceptance status (run in this tree)

`cleanroom_annots`: 41 pass. `app_annots`: 23 pass. Every remaining
failure is in another workstream's stub or in a test itself:

Fail only in workstream 04 (`view/stext.rs`, text extraction):
- cleanroom_annots: `a_cjk_font_in_a_stored_form_is_three_objects`,
  `a_long_stamp_name_stays_inside_its_box`, `a_long_word_wraps_inside_the_box`,
  `a_turned_free_text_reads_the_same_text`, `any_stamp_name_reads_in_capitals_with_spaces`,
  `fitted_boxes_draw_every_line_unwrapped`,
  `free_text_paints_background_then_border_then_text_inside_its_rect`,
  `standard_stamp_names_read_as_their_labels`
- app_annots: `every_kind_is_written_with_its_appearance`,
  `free_text_fits_its_lines_and_refuses_what_it_cannot_write`,
  `free_text_draws_greek_cyrillic_cjk_and_latin_letters_in_fallback_fonts`,
  `free_text_lays_out_inside_its_rect_differences`, `rotated_pages_keep_what_the_user_sees`,
  `set_rewrites_contents_and_refits_a_text_box_from_its_top_left`
- app_accept_minors: `latin_letters_winansi_lacks_store_a_widget_appearance`,
  `an_empty_base_font_name_is_listed_as_null`

Fail only in workstream 03 (`repair.rs`; the files hold `1e300`, which a
strict open refuses, so the open goes to repair):
- cleanroom_annots `hostile_annotations_without_an_appearance_stay_finite`
  (the same annotations written with 300-digit numbers pass locally; see below)
- app_annots `a_stamp_signature_with_rect_differences_moves_at_a_stable_size`
- app_accept_minors `an_invalid_free_text_dash_array_falls_back_to_the_default`

All the pixel parts before the extraction calls in these tests passed.

## Two tests that look wrong (please check)

1. `cleanroom_annots::a_caret_fills_its_rect`, second case: /RD [10 20 30 5]
   is expected to give the inner box [110 620 170 695], i.e. it reads /RD as
   left, bottom, right, top. ISO 32000-2 (Square/Circle/Caret/FreeText /RD)
   and the surviving reader `annots::rect_differences` read left, top,
   right, bottom, giving [110 605 170 680]. I draw the caret into
   `design_rect` (ISO order), so the caret lands in [110 605 170 680] and the
   test fails at its ink check. The listing part (A3: whole /Rect) passes.
   Either the test's box or the reader should change; I changed neither.
2. `app_annots::random_requests_never_panic_and_undo_to_the_start` panics in
   the test itself: `Rng(seed * 0x9E37_79B9_7F4A_7C15)` overflows u64 for
   seed >= 2 in a debug build (`wrapping_mul` would fix it).

## Other things outside my files

- `src/view/text_unicode.rs` tests `include!` `tests/data/adobe-symbol-encodings.rs`,
  which is missing, so `cargo test --lib` and `cargo clippy --all-targets`
  cannot build the lib test target in this tree.
- `cargo clippy -D warnings` on the lib fails outside my files:
  `view/fonts.rs:131` `is_cjk_fallback` unused; `view/mod.rs:147` two
  useless `f64::from` conversions. My three files have no warnings.
- In a local render the CJK fallback (non-embedded `STSong-Light` Type0)
  drew no glyphs, while Greek, Cyrillic and extended Latin did. That is the
  view's fallback (workstream 04/07), not the FreeText content.
- FreeText's Greek/Cyrillic/extended-Latin fallback fonts are written
  non-embedded (`Eight::font` with /Differences): a `Form`'s resources are a
  direct dictionary and cannot hold a font program stream. Embedding them as
  widgets do (decision W1) would need `annots::put_appearance` to write
  streams.

## How I verified locally (nothing of this is committed)

`view::PageFrame::new` and `render::pixel_size` are workstream 04 stubs,
so no render ran. To run the pixel tests I put a throwaway stand-in in
those two functions (page box and transform from hayro's
`render_dimensions` / `initial_transform`), and, for the lib unit tests, an
empty stand-in `tests/data/adobe-symbol-encodings.rs`. Both were removed
before committing; `git diff` shows neither. A scratch test (kept outside
the tree) rendered every icon, stamp, ending, markup and FreeText to PNGs
for a visual check, and re-ran the hostile listing test with 300-digit
numbers instead of `1e300` (it passes).

## Choices where the spec leaves room

- /RD of a Caret: `synthesize` and `synthesized_box` give rd = [0 0 0 0]
  with rect = /Rect, so the listing is the whole /Rect (A3, and the
  `a_caret_...` listing checks) and `annot-set` writes an /RD matching the
  stored appearance, which draws into the old /Rect less /RD.
- Squiggly budget: past about 10,900 quads (3 vertices each) the 32,768
  vertex budget cannot give every quad a wave; the quads past the budget get
  no squiggle (the view caps quads at 16,384).
- `fonts::win_ansi` gives no code for C0 controls and U+007F (spec 5.2 lists
  only U+0020–U+007E and U+00A0–U+00FF besides 0x80–0x9F); `encode` keeps
  C0 bytes as the spec says.
- The FileAttachment and Sound icons (A5) keep their aspect, centred in
  /Rect; any /Name draws the paperclip (FileAttachment) or the speaker (Sound).
