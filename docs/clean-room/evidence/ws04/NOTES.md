# Workstream 04 notes

## Clean-room record

- **One command reached outside the tree, by mistake.** While checking an
  API, a compound shell command included `find / -path /System -prune -o
  -name "*.rs" -path "*hayro-cmap*" -print 2>/dev/null | head -0`, a search
  of the whole filesystem for hayro-cmap sources. Its output was discarded
  (`head -0`, nothing was printed), no file it might have found was opened,
  and the command was stopped (it ran in the background after a timeout).
  Nothing else outside `tpdf-cr/ws04` was read or run, apart from the
  `tpdf-wt/tools/slot` build tool.
- No web search, no MuPDF or other PDF tool, no other repository.

## Changes outside "Your files"

- `engine/vendor/hayro-interpret-0.8.0/` (owner decision T1): the
  text-extraction patch, every change marked `Tandem patch` and recorded in
  its `PATCH.md` (new section, and the diff of this patch at the end; the
  older "Diff against the published crate" section was not regenerated,
  since the published crate is not in this tree).
- `engine/tests/{app_view,cleanroom_text,view,view_text}.rs`: the fifteen
  `#[ignore = "clean-room: 04§…"]` attributes HANDOFF lists were removed;
  nothing else changed in them.
- `engine/src/view/mod.rs`: besides `PageFrame`, the pure page-frame rule
  `frame_of` / `FrameNumbers` (spec 04, 5.4) and the `/Parent` reader it
  uses were added next to it. No other line of the file changed.

## Needed outside this tree's files (not done here)

1. **Spec 04, 5.4: done (integration round, authorized).** `app::info`
   gained `page_frame` (boxes, /Rotate and /UserUnit read as the view reads
   them, then `view::frame_of`); `page_size`, `display_rotation` and
   `nav::links::PageSpace::of` use it (links' own `rotation` helper is
   gone). A temporary test confirmed app and view agree on all the odd
   boxes of `app_core`'s geometry test, the sliver included.
   **Still needed, in `src/app/view_fixes.rs` (not authorized here):** for
   a page whose MediaBox is missing or empty, the Letter fix also writes
   `/CropBox` = Letter (line 178), so the view never sees the page's own
   CropBox: `/MediaBox [0 0 0 0] /CropBox [100 100 400 400]` draws 612 ×
   792 instead of 300 × 300 (`app_view::a_crop_box_outside_the_media_box_…`).
   The view on the raw bytes gives 300 × 300. Writing only `/MediaBox`
   fixes it: hayro then clips to the CropBox within Letter, the same area
   `frame_of` gives.
2. `engine/bridge/Cargo.lock` still needs the `unicode-bidi` entry
   (workstream 05 note).
3. Scrub list of spec 04, 8.5 (comments in `app/info.rs`,
   `app/ops/nav/links.rs`, `app/ops/view.rs`, `app/ops/edit/font.rs`):
   not touched here.

## Acceptance

Passing:

- `tests/cleanroom_text.rs` 26/26 (all nine un-ignored ones).
- `tests/view_text.rs` 12/12 (the four un-ignored ones).
- `tests/view.rs` 23/24, including both un-ignored ones; the failure is
  below.
- `tests/cleanroom_unicode.rs` 22/22 (spec 05 through structured text).
- `tests/view_hostile.rs`, `view_inherited.rs`, `view_cjk_font.rs`,
  `view_cjk_font_metrics.rs`: all pass. `view_robust.rs`: all pass except
  three that panic in the spec 07 stub (`a_page_written_into_kids_…`
  passes its view part; it panics later in `app/forms/appearance.rs`).
- Unit tests in `view::stext` (3) pass.

Failing, not in another workstream's stub:

- `view::type3_text_comes_from_encoding_names`: the characters are right;
  the spacing is not. Its last two lines are drawn with `Tc 2` still set
  from an earlier text object (text-state parameters outlive `ET`), so
  every glyph of `<a1…a9>` is followed by the same 0.2 em gap. The test
  wants a space after six of those gaps and none after two
  (`Th \u{fffd}\u{fffd}é`), which no gap threshold can give: 0.2 em is in
  the 0.10–0.25 band where spec 04, 3.4 allows either, and this design
  (0.15 em) reads a space at every one. Got
  `… Th \u{fffd} \u{fffd} é α A \u{fffd}`. The expectation should be
  re-derived (with a space at every gap, or none, depending on the band
  value the reviewer prefers).
- `security::tests::passwords_convert_to_pdfdoc_as_measured`: reads a
  reference file not in the hand-off tree (as workstream 05 noted).

Failing inside other workstreams' `todo!()` stubs (expected): every test of
`app_accept_edit`, `app_edit`, `app_edit_objects` that runs an edit, most
of `app_annots`, `app_core`, `app_misc`, `app_nav`, `app_redact*`,
`app_scan_text` and all but one of `app_view` (spec 06
`app/annots/appearance.rs`, spec 07 `app/forms/appearance.rs` and
`app/forms/text.rs`), `view_robust`'s three above, lib tests
`app::ops::scan::tests::numbers_tags_and_routes` (spec 07) and
`app::document::tests::a_value_nested_past_the_parse_depth_is_still_repaired_away`
(spec 03). `app_redact_pixel_scale`'s `a_user_unit_past_f32_…` and
`any_page_size_…` report "no reply in 20 s" because the worker thread
panicked in the spec 06 stub; their view checks pass. These cover spec 04
behaviour too (`app_view::a_crop_box_outside_the_media_box_shows_the_media_box`,
`degenerate_page_boxes_draw_and_read`, the RTL tests of `app_scan_text`,
`app_redact` quads) and must be re-run once those stubs are filled.

Not run: the corpus gate (8.3), the WASM build, `npm test`.

## Design (thresholds in ems of the glyph size)

| Rule | Value | Band in the spec |
|---|---|---|
| inferred space | gap ≥ 0.15 | none ≤ 0.10, space ≥ 0.25 |
| new line along the baseline | gap ≥ 3, or overlap > 1 (unless the glyph lies just left of the previous one: RTL drawn with the pen moving left) | ≥ 3 may, ~1 overlap continues |
| same line across the baseline | offset ≤ 0.55 × larger size | ≤ 0.4 join, ≥ 1 not |
| parallel | cos ≥ 0.999 (≈2.6°) | "small angle" |
| new block: baseline distance | > 1.55 × smaller size | ≤ 1.3 stay, ≥ 3 new |
| new block: indent | ≥ 0.5 right of the block's leftmost start, when the line is ≥ 0.5 below | ≤ 0.1 not, ≥ 1 new |
| overprint merge | same char and size, origin within 0.05 | 0.01 required, 0.05 recommended (T7) |
| search quad split | gap > 1 | T13 |
| summary numbers | rounded to 1e-4 before truncation | (float noise: 29.9999997 pt reads 30) |

## Spec ambiguities and how they were resolved

1. **Generic family of embedded fonts.** `line_summaries_of_fixtures`
   requires `serif` for `rtl.pdf`'s embedded Noto Sans Hebrew/Arabic
   (Flags 32, OS/2 sFamilyClass 0). Rule: an embedded program that
   classifies itself decides (classes 1–5, 7 serif; 8 sans); one that does
   not, and is not named Helvetica/Arial, is serif (the CSS default face);
   a substituted font is serif only by its Serif flag, its OS/2 class or a
   Times name.
2. **Ascender source.** Descriptor `/Ascent`/`/Descent` when plausible
   (ascent in (0, 3], descent in [−2, 0], at least 0.5 em in all); else the
   embedded sfnt's `hhea` (then `OS/2` typo); else, for a non-embedded
   standard font, the vertical metrics of the program the view substitutes
   when it is an sfnt (the URW Helvetica faces and Times-Roman), else the
   Core 14 AFM FontBBox; else 0.8 / −0.2. Type3: FontBBox through
   FontMatrix.
3. **Annotations** are not part of structured text (as before:
   `redact.pdf`'s text excludes its annotations' appearances).
4. **Search whitespace excludes U+0085 (NEL)**, which JavaScript does
   not trim, so a NEL query matches only a NEL in the text. **A search match of nothing drawn** (the space a line break reads as,
   matched by a whitespace-only needle) is not a hit: it has no quad to
   show. This gives the expected three hits for `"\n"` in
   `search_whitespace_needles_and_truncation`.
5. **Paragraph direction (UAX #9 on visual order).** Right to left when
   the line's first and last strong characters (visual order) are both
   R/AL, left to right when both are L, else by majority. Plain "mostly R"
   would make `XY abc de ZW` right to left and read `ZW … XY`. Lines without
   R/AL are never reordered (a run of Arabic-Indic digits alone keeps its
   order). The characters of one glyph (a decomposed presentation form, a
   multi-character glyph name) keep their order through the reversal.
6. **ActualText surplus with no glyph before it** (every glyph matched the
   ActualText's end) sits at the first suffix glyph's origin.
7. **ActualText from the structure tree** is looked up only when the
   property list has no `/ActualText` of its own and no outer ActualText is
   in force; the MCID comes from the property list (inline or named).
8. **Visibility** uses clip bounding boxes (a conservative test: a glyph
   inside a non-rectangular clip's box counts as visible). Ink bounds are
   computed only for glyphs whose em box is not wholly inside the visible
   area; CJK fallback glyphs and Type3 glyphs use their em box.
9. **Vertical writing:** the quad is the advance down the line, half the
   horizontal origin displacement to each side.
10. **Reader caches** start afresh past 1,024 fonts, 128 MiB of decoded
    programs or 64 pages; one hayro cache serves a reader's pages.
11. **`/Parent` given by reference is read from the xref table afresh**: a
    page written directly into its parent's /Kids was parsed while that
    parent was being read, and hayro-syntax's own resolution of the
    reference from inside it reports nothing.

## Integration addenda (spec 04, section 12)

- C1: `span_blocks()` is laid out from the visible characters before
  overprinted copies are merged, so each run keeps its characters;
  `blocks()`, `as_text`, `spans_text` (now the reading text, `as_text`)
  and search merge copies as before.
- C2: in the run layout a new text-showing run whose first glyph starts
  more than 1 em left of the previous run's end on the line and reaches
  more than 0.05 em right of that run's start begins a new line.
  Right-to-left text drawn glyph by glyph lies wholly left of the run
  before it and still joins. Unit tests cover both cases and both C2
  variants (685.6 and 682.72).

## Corpus addenda C3 and C4 (spec 04, section 12)

- **C3.1** Line joining measures the along-line overlap in ems of the
  smaller of the new glyph and the line's last one. A glyph drawn back to
  the left joins only when it lies left of everything on the line (it may
  reach 0.05 em into it): right-to-left text drawn with the pen moving left
  still joins, a run drawn back over the line (a large word over a small
  one, a small label across a large mark) starts its own line.
- **C3.2** A line keeps drawing order. Its characters are grouped into
  text-showing runs; the runs are ordered by their leftmost position
  (stable: ties keep drawing order); a run's own characters stay in drawing
  order unless its pen moves left by more than 0.01 em (right-to-left text
  drawn in logical order in one operator), which sorts that run by
  position. Characters within 0.01 em keep their drawing order (the Thai
  mark case). Bidi reordering (3.6) follows, as before.
- **C3.3** follows from C3.1: an overlapping run starts a new line in the
  same block.
- **C4** `PageFrame` carries its sides in double precision (`exact`), and
  the pixel size uses them; `width`/`height` stay f32 for `page_size`
  (and so for `app::info`, which matches it). ε is now 1.02e-3, inside the
  pinned [0.00101, 0.00104).
- Tests: `cleanroom_text_corpus.rs` 5/5 (the four ignored ones
  un-ignored). Full suite: unchanged apart from these, i.e. only the
  known `view::type3_text_comes_from_encoding_names` expectation, the
  missing `security` reference file, and panics inside the spec 03/06/07
  stubs of this tree.

## C3 review fix

- **C3.2** Position now orders the characters inside a run only when the
  run's pen moves left and the run holds right-to-left characters (Bidi R
  or AL, the 3.6 case); any other run keeps drawing order, and only whole
  runs are ordered by position. Test
  `c3_a_left_to_right_run_moving_back_keeps_drawing_order`
  (`[(ab) 800 (c)] TJ` reads `abc`); the review's input is placed at
  y = 200 so that it lies on the test file's 300 × 300 page.
- **Left join tolerance (review note 2).** Agreed: 0.05 em was too tight
  for right-to-left text drawn glyph by glyph with negative kerning. A
  right-to-left glyph (R or AL) drawn back to the left of a line now joins
  when it reaches up to about one em (`MAX_OVERLAP`, 3.5 rule 3) into it.
  A left-to-right glyph keeps the 0.05 em limit, since a left-to-right run
  drawn back over a line is exactly what C3.1 and C3.3 split off; widening
  it for them would let a small label drawn across a large mark join again.
- Blank lines left by the removed ignore attributes in
  `cleanroom_text_corpus.rs` removed.
- Full suite: unchanged apart from these, i.e. only the known type3
  expectation, the missing `security` reference file, and stub panics.

## Addendum C5 and the cost of text extraction

- **C5.2, the whole run decides.** A left-to-right run that joins a line
  at its front (its first glyph lies before the line's first character)
  is on probation until it ends: if any of its glyphs reaches into the
  line's characters by more than about an em (the C3.3 allowance, ems of
  the smaller of the glyph and the line's size), its characters leave the
  line, the line's state is restored as it was before the run, and the run
  starts a new line in the same block. A whitespace first glyph changes
  nothing; right-to-left glyphs (C5.3) never go on probation. C5.1 front
  joins (bullets, line numbers) are kept. Test
  `c5_a_run_drawn_back_before_a_line_and_across_it_reads_after_it`
  un-ignored and passing; the C5.1 control stays green.
- **Cost (8.3 item 6).** Results unchanged; changes:
  1. Overprint check: a small keyed struct and a seeded multiply-rotate
     hasher instead of SipHash over a tuple; cells two reaches wide, so
     four probes instead of nine; one map entry per cell with the cell's
     characters chained through an index vector instead of a `Vec` per
     cell.
  2. Bidi: `right_to_left(c)` skips the class lookup below U+0590 (no
     strong right-to-left character there); used in the line join, in run
     ordering and before the per-line reordering, which looks classes up
     only for lines that hold right-to-left text.
  Measured (release, best of five, this machine, a temporary benchmark
  since removed), before → after: `fixtures/large.pdf` text of all 500
  pages 349 → 139 ms, search of all pages 347 → 139 ms; a page of 85,800
  glyphs 140 → 52 ms; the overprint check on 1,000,000 glyphs 962 →
  205 ms; the right-to-left test on 1,000,000 Latin letters 22.6 →
  0.3 ms. Font interpretation was not changed.
- Full suite: unchanged apart from these (the known type3 expectation,
  the missing `security` reference file, stub panics).

## Addenda C5a and C6, evidence

- **C5a.** One allowance for front joins, 0.10 em (`FRONT_TOUCH`, ems of
  the smaller of the glyph and the line), replacing both the 0.05 em
  left-join limit and the one-em probation limit. A left-to-right run is a
  front candidate when its first glyph starts before the line's leftmost
  character, whichever path would admit it (so a line shorter than an em
  is no exception); a first glyph that ends past the allowance is refused,
  and a later glyph of the run that does, whether it would join or not,
  takes the whole run back out of the line (it starts a new line in the
  same block and reads after the line). Right-to-left glyphs keep the
  one-em allowance (C5 rule 3). `c5a_a_front_run_that_reaches_into_the_line_reads_after_it`
  un-ignored and passing; C5 tests, C3 and the C5.1 control stay green.
- **C6.** The overprint check now works glyph by glyph: only a glyph's
  first character is placed in the grid, a later glyph is a copy only if
  its whole character sequence equals the kept glyph's (same characters,
  same count), and then each kept character takes its own counterpart's
  visibility. Unit test
  `a_visible_copy_of_a_glyph_of_two_characters_shows_both` (an "ff" glyph
  clipped away, drawn again visibly, then "x": "ffx", all visible); the
  corpus test `c6_…` stays green.
- **Cost recheck** (same temporary benchmark, removed again): large.pdf
  text 150 ms, search 151 ms, dense page 57 ms, overprint check on 1M
  glyphs 260 ms (the C6 sequence comparison); still about 2.3× faster
  than before a43a580.
- **SOURCES.md** cites spec 04 §12 C5, C5a, C6 (hand-off commits 0bb1334,
  82a7582) and the rotate-xor-multiply hash construction (FxHash design,
  used from general knowledge).
- Full suite: unchanged apart from these (type3 expectation, missing
  `security` reference file, stub panics).

## Addendum C7

- A line now keeps the leftmost position of its own characters (those not
  joined at its front, `core_min`) and its rightmost position (`max`).
- **Rules 2 and 3.** A new run's first glyph is measured from the line's
  rightmost character, not from the last glyph drawn (which, after a front
  join, is the front piece): it joins after the line's end within the
  usual allowance (overlap up to about an em, gap below 3 em), or starts a
  new line in the same block. A glyph of the same run still continues from
  the glyph before it.
- **Rule 1 and the control.** A front candidate is a left-to-right run
  whose first glyph starts before the line's own characters (`core_min`).
  It joins if it ends within 0.10 em into them and less than the
  line-break gap before them, and then stays on probation until it ends,
  measured against `core_min` (C5a). A second run lying wholly before the
  line's own characters therefore joins the front group, as the control
  `c7_a_second_run_wholly_before_the_line_joins_its_front` requires
  ("1." before "alpha"). Read literally, rule 1 ("starts before the
  front-joined run's leftmost character") would refuse that run. A run
  starting among the front pieces that reaches into the line's own
  characters ("2 WXYZ" after "1") is refused and starts its own line.
- `c7_a_run_after_a_front_joined_mark_does_not_enter_the_line` un-ignored
  and passing. Full suite: unchanged apart from this (type3 expectation,
  missing `security` reference file, stub panics).

## C7 review fix

- The front-join gap (C5 rule 1: less than the 3.4 line-break gap before
  the line) is now measured to the line's leftmost character, front-joined
  pieces included; the 0.10 em "into" allowance, front candidacy and
  probation still use the line's own characters. A front group built from
  several runs ("7" before "•" before "text"; "Say" before "Hello" before
  "World") stays one line. `c7_a_front_group_of_several_runs_keeps_joining`
  un-ignored and passing; the other C7 tests and controls stay green.
- Optional point, not done: the gap is still measured from the front
  run's first glyph, so a front run longer than 3 em whose last glyph ends
  just before the line is refused. Deciding at the run's end would mean
  admitting such a run provisionally and taking it back out when the run
  ends far from the line, which in the streaming line builder happens when
  the next run's first glyph arrives (or the line ends) and would then
  have to place that glyph against a different line. That is more than
  falls out of this change, so it is left as it was.
- Full suite: unchanged apart from this (type3 expectation, missing
  `security` reference file, stub panics).
