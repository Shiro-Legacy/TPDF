# Spec 04: structured text, page frame and pixel size

Workstream 4 of `docs/mit-clean-room-plan.md`. Status: draft for review,
2026-10-18. Rules: `docs/clean-room/SPEC-RULES.md`.

This spec replaces three pieces of `engine/src/view/`:

- `stext.rs`: structured text (characters, lines, blocks), plain text,
  the line summaries the app draws its text layer from, and search;
- the page-frame logic in `mod.rs` (`PageFrame`, the page-box reader and the
  `/Parent` lookup it uses): page size, rotation and the page-space
  transform;
- the pixel-size rule in `render.rs` (`pixel_size`).

How a character code becomes Unicode is workstream 5
(`05-glyph-unicode.md`). This spec uses the interface that spec defines
(05 §2.3) and nothing else from it.

The implementer may read: this spec, spec 05, ISO 32000-2, Unicode
Standard Annex #9 and the Unicode Character Database, the OpenType
specification, the acceptance tests listed in section 8, the experiment
scripts in section 7, and the surviving engine code (everything except the
class A files of the plan). The implementer may not read the current
`stext.rs`, `text_unicode.rs`, `render.rs::pixel_size`, the `PageFrame`
part of `view/mod.rs`, their git history, the `engine/design/*` notes
(removed from the hand-off tree), or MuPDF source.

Section numbers like §9.4 refer to ISO 32000-2:2020.

---

## 1. Scope

### 1.1 What the component does

For one page of a `ViewDocument` the component produces:

1. **Structured text.** Every character the page shows, with its page-space
   position (origin, quad, size, font facts), grouped into lines and
   blocks, in three layouts (2.4).
2. **Plain text** of those layouts (`as_text`, `spans_text`).
3. **Line summaries**: one record per text run with integer geometry and a
   CSS font description. The app draws its selectable text layer from them.
4. **Search**: case-sensitive matches of a needle, each with the quads to
   highlight.
5. **The page frame**: page size in points, the effective rotation, and
   the transform from user space to page space (origin top left, y down).
6. **The pixel size** of a render at a scale.

### 1.2 Callers (surviving code)

| Caller | Uses | Needs |
|---|---|---|
| `app::ops::view::render_page` (render reply) | `page_size`, `structured_text`, `line_summaries`, `spans_text`, render pixel size | `width`/`height` of the reply are `page_size`. `lines` is the summaries. `text` is shown to the user in the "Extracted text" panel. The app checks `ceil(width × scale)` against its own edge limit before rendering, so the pixel-size rule (5.2) must never give more than that. |
| `app::ops::view::search` | `text_reader`, `search_hits` | Hits in page order, at most a budget of hits in all, `truncated` when a further match exists. Each quad is sent as 8 numbers. |
| `app::ops::view::text_all` | `text_reader`, `as_text` | Every page's text, trimmed, joined by form feeds. |
| `app::ops::view::thumbnail` | `page_size` | |
| `app::ops::compare` (`compare-text`) | `line_summaries` (`text`, `x`, `top`, `w`, `h`) | A line diff of two pages. |
| `app::ops::edit::scan::runs_of` | `span_blocks`, `fonts`, `TextChar` fields `c`, `size`, `quad`, `origin`, `font`; `TextFont::name` | Content editing finds the drawn text of a content-stream run by matching it against the view's lines. A line must therefore not mix fonts (2.4), and the font name must be the one `text_font_name` gives (2.6). |
| `app::ops::edit::font` | `view::text_font_name` | Names fonts the way the view does. |
| `app::redact::verify` | `text_reader`, `search_hits`, `unclipped_blocks` (chars and their quads) | Proves redacted text is gone. It must see every character drawn, clipped or off the page (3.8). |
| `app::redact::marks` (`redact-search`) | `text_reader`, `search_hits` | Marks every hit. |
| `app::ops::edit` (`page_size`) | `page_size` | |
| `view::measure` | `PageFrame::new(page).ctm` | The page-space transform for glyph measurement. |
| `view::render` | `PageFrame` (`ctm`, `width`, `height`) | Draws with `Affine::scale(scale) * ctm` into the pixel size of 5.2. |
| `app::info::page_size`, `app::info::display_rotation`, `app::ops::nav::links` | (their own copies of the page-frame rule) | Must give the same numbers as the view on every page (test `app_core::page_geometry_matches_the_view_on_every_fixture_and_odd_boxes`). See 5.4. |
| `examples/view_probe.rs` | `structured_text`, `line_summaries`, `as_text`, `spans_text`, `search`, `MAX_SEARCH_QUADS` | Diagnostics. |

### 1.3 Surviving code the component builds on

- `hayro` and `hayro-interpret` (vendored, Apache-2.0 OR MIT): the
  interpreter and its `Device` trait (`draw_glyph_run`, `draw_path`,
  `draw_image`, `push_clip_path`, `pop_clip`, transparency groups,
  `begin_marked_content`, `end_marked_content`), glyph runs
  (`Glyph::transform`, `OutlineGlyph::advance_width`, `outline`,
  `font_cache_key`, `font_data`, `as_unicode`), and its optional-content
  evaluation. **Text and render come from the same interpretation**, so
  what the page shows and what it reads agree (3.1).
- `view::walk` (`check_page`, `Scope`, `FORM_DEPTH`, the content
  budgets, `refused`): the refusals a page gets before and during
  interpretation.
- `view::fonts::settings`: the interpreter settings the view uses, with
  the URW substitutes for non-embedded fonts and the CJK fallback font.
- Spec 05: `FontText`, `Code`, `CMaps`, `encoding_names`,
  `normalise_extracted`, `bidi_class`, `bidi_mirror` (05 §2.3).
- `object::decode_text_string` (workstream 1): decoding PDF text strings
  (ActualText).

### 1.4 Non-goals

- Reading-order reconstruction (columns, tables). Blocks and lines come in
  the order the page draws them (3.5).
- Text inside Type3 glyph procedures and tiling-pattern cells.
- Abbreviation expansion (`/E`, §14.9.5) and alternate descriptions
  (`/Alt`).
- Filtering artifacts (§14.8.2.2). Headers and footers marked as
  artifacts are extracted like other text.
- Case-insensitive or diacritic-insensitive search (open question Q3).

---

## 2. Interface

Module paths stay as they are: `crate::view::{StructuredText, ...}`, with
`stext` private to `view`. Items marked `pub` are re-exported from
`crate::view` exactly as today. Private items are the implementer's
choice.

### 2.1 Constants

```rust
/// Characters one page's structured text may hold, ActualText included.
/// Past it the page is refused with
/// ViewError::TooComplex("page has more text than the limit").
pub const MAX_PAGE_CHARS: usize = 1 << 20;

/// A quad cap for direct callers of `StructuredText::search`.
pub const MAX_SEARCH_QUADS: usize;
```

`MAX_PAGE_CHARS` is a limit the project chose itself; it is part of the
interface (tests and `ViewError` docs name it). The value of
`MAX_SEARCH_QUADS` is the implementer's choice, large enough that the
direct searches in the acceptance tests (a few hits each) are not
truncated. The app does not use it.

### 2.2 Records

```rust
#[derive(Clone, Debug, PartialEq)]
pub struct TextFont {
    pub name: String,
    /// Em-relative extent above the baseline (positive) and below it
    /// (negative), used for character quads.
    pub ascender: f64,
    pub descender: f64,
    pub bold: bool,
    pub italic: bool,
    pub serif: bool,
    pub monospace: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextChar {
    pub c: char,
    /// Baseline origin in page space (points, y down).
    pub origin: [f64; 2],
    /// Corners ul, ur, ll, lr as x, y pairs in page space.
    pub quad: [f64; 8],
    /// Font size in page space: sqrt(|det|) of the glyph's em-to-page matrix.
    pub size: f64,
    /// Index into `StructuredText::fonts`.
    pub font: usize,
    /// A space inferred from a gap (3.4), not drawn by the page.
    pub synthetic: bool,
    // private fields allowed
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextLine {
    /// x0, y0, x1, y1 in page space: the union of the chars' quad bounds.
    pub bbox: [f64; 4],
    /// Unit baseline direction in page space.
    pub dir: [f64; 2],
    pub chars: Vec<TextChar>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextBlock {
    /// Union of its lines' bboxes.
    pub bbox: [f64; 4],
    pub lines: Vec<TextLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineSummary {
    pub text: String,
    pub x: i32,
    /// Baseline of the first character.
    pub y: i32,
    /// Top of the line's bbox.
    pub top: i32,
    pub w: i32,
    pub h: i32,
    pub size: i32,
    pub family: &'static str,   // "monospace" | "serif" | "sans-serif"
    pub weight: &'static str,   // "bold" | "normal"
    pub style: &'static str,    // "italic" | "normal"
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchResult {
    /// Each hit's quads (ulx, uly, urx, ury, llx, lly, lrx, lry).
    pub hits: Vec<Vec<[f64; 8]>>,
    /// A further match exists that was left out by the limit.
    pub truncated: bool,
}
```

`TextChar` may keep private fields, so that only the component builds it.

### 2.3 Entry points

```rust
impl ViewDocument {
    pub fn structured_text(&self, page: usize) -> Result<StructuredText, ViewError>;
    pub fn text_reader(&self) -> TextReader<'_>;
    pub fn page_size(&self, page: usize) -> Result<(f64, f64), ViewError>;
}
pub struct TextReader<'a> { /* private */ }
impl TextReader<'_> {
    pub fn structured_text(&self, page: usize) -> Result<StructuredText, ViewError>;
}
```

- `ViewDocument::structured_text(p)` equals
  `text_reader().structured_text(p)`.
- A `TextReader` reads several pages of one view and shares font and CMap
  work between them (6.3). Reading page *p* through a reader gives the same
  `StructuredText` as reading it alone, whatever pages the reader read
  before (test `text_all_and_search_stay_right_across_reader_cache_resets`).
- Errors: `PageOutOfRange` for a bad index; the refusals of
  `walk::check_page`; the interpreter's content refusal
  (`walk::refused`); `TooComplex("page has more text than the limit")`
  past `MAX_PAGE_CHARS`. Nothing else. An empty page is `Ok` with no
  blocks.
- `page_size(p)` is the frame's `(width, height)` in points (5.1).

### 2.4 `StructuredText`

```rust
#[derive(Clone, Debug)]
pub struct StructuredText { /* private */ }
impl StructuredText {
    pub fn blocks(&self) -> &[TextBlock];
    pub fn span_blocks(&self) -> &[TextBlock];
    pub fn unclipped_blocks(&self) -> &[TextBlock];
    pub fn fonts(&self) -> &[TextFont];
    pub fn as_text(&self) -> String;
    pub fn spans_text(&self) -> String;
    pub fn line_summaries(&self) -> Vec<LineSummary>;
    pub fn search(&self, needle: &str, max_quads: usize) -> SearchResult;
    pub fn search_hits(&self, needle: &str, max_hits: usize) -> SearchResult;
}
```

Layouts may be built lazily (on first use) and cached.

- **`blocks()`**, the reading layout: the visible characters (3.8) in lines
  and blocks (3.5).
- **`span_blocks()`**, the run layout: the same characters, with a line
  also broken at every **run boundary**. A run boundary is where any of
  these changes between two consecutive characters: the font, the font
  size or the glyph matrix's linear part, the fill colour, the text
  rendering mode, the text object (`BT`), or the direction (3.6). A line of
  `span_blocks` therefore never mixes fonts. The implementation may also
  break runs at other graphics-state changes; tests pin only the list
  above.
- **`unclipped_blocks()`**, the search layout: like `blocks()`, but with
  every character drawn, including clipped ones and ones outside the
  page (3.8).
- **`fonts()`**: the `TextFont` of every `TextChar::font` index.
- **`as_text()`**: for each block of `blocks()`, each line's characters
  followed by `\n`, then one more `\n` after the block. An empty page
  gives `""`.
- **`spans_text()`**: the text of the summaries (`line_summaries`), one
  line of text per **visual line**: summaries that continue the same
  `blocks()` line are joined with nothing between them (an inferred space
  stays where 3.4 puts it). Lines end in `\n`, blocks in one more `\n`,
  as in `as_text`. This is a deliberate change: a colour change inside a
  word no longer breaks the "Extracted text" panel's line (Q5).
- **`line_summaries()`**: one record per line of `span_blocks()` with at
  least one character:
  - `text`: the line's characters;
  - `x`, `top`, `w`, `h`: the line bbox's x0, y0, x1 − x0, y1 − y0;
  - `y`: the first character's origin y; `size`: its `size`;
  - each number truncated toward zero to `i32` (saturating);
  - `family`, `weight` and `style` from the first character's font:
    `monospace` if `monospace`, else `serif` if `serif`, else
    `sans-serif`; `bold` if `bold`; `italic` if `italic`.

  The app draws `text` at (`x`, `y`) with font size `size` and stretches
  it to `w`. Selection in the app is a browser selection over that layer,
  so the summaries must cover every visible character once, in reading
  order within the line.

### 2.5 Search

`search(needle, max_quads)` and `search_hits(needle, max_hits)` search
`unclipped_blocks()`:

1. The haystack is the layout's characters in order, with a line break
   read as one space, except that a line ending in a hyphen (U+002D,
   U+00AD, U+2010, U+2011) joins the next line without the hyphen and
   without a space.
2. Matching is case-sensitive and exact per character, except whitespace:
   a run of whitespace in the needle matches a run of one or more
   whitespace characters in the haystack. Whitespace is the Unicode
   `White_Space` property (tab, line feed, carriage return, space,
   no-break space and the other `Zs` characters). An empty needle has no
   hits. A needle of only whitespace matches every whitespace run.
3. Matches do not overlap: after a match, the search resumes after its
   end. Hits are in haystack order.
4. A hit's quads: one quad per stretch of the match that lies on one line
   and is contiguous there (no line change, no reversal of direction, no
   gap wider than a word space). A quad covers the union of its
   characters' quads along the line direction, with the height of the
   characters' quads. Characters that come from ActualText (3.7) have
   quads like any other. The joined hyphen of rule 1 gets no quad.
5. `search` stops before the first match whose quads would take the total
   past `max_quads`; `search_hits` stops before the match that would make
   more than `max_hits` hits. A hit is never cut short. `truncated` is
   true exactly when a match was left out. `max_quads == 0` or
   `max_hits == 0` gives no hits, with `truncated` true if any match
   exists.

### 2.6 `text_font_name`

```rust
pub(crate) fn text_font_name(base: &[u8]) -> String;
```

The name that `TextFont::name` holds for a font whose `/BaseFont` (the
descendant's for a Type0 font, else its own) has the bytes `base`. It is
the name decoded as UTF-8, with invalid sequences replaced (U+FFFD), with
no truncation below the 127-byte name limit of Annex C. The view and the
edit scanner both name fonts with it, so the only requirement beyond that
is that the two agree. For a font with no `/BaseFont` the name is empty.
For a Type3 font it is its `/Name` if present, else a stable name of the
implementer's choice (for example one built from its object number).

### 2.7 Page frame

```rust
#[derive(Clone, Copy, Debug)]
pub(crate) struct PageFrame {
    /// User space to page space (points, origin top left, y down).
    pub(crate) ctm: Affine,
    pub(crate) width: f64,
    pub(crate) height: f64,
}
impl PageFrame {
    pub(crate) fn new(page: &Page<'_>) -> Self;
}
```

The old `blank` field is gone (5.1, rule 3). `width` and `height` may stay
`f32` internally if the implementer prefers. The pure rule of 5.1 is also
offered as a function of numbers (5.4), so that `app::info` can use it.

---

## 3. Required behaviour: characters and layout

### 3.1 Principle: text is what the page shows

Every glyph the interpreter draws for the page contributes its characters,
in drawing order. Nothing the interpreter does not draw contributes, with
two exceptions that are text all the same: glyphs in rendering mode 3
(invisible, §9.3.6: OCR layers) and mode 7 (clip only). Consequences,
each tested:

- Text in an optional-content group that is off (§8.11) is not extracted
  and not found by search. The view's layer settings apply.
- A form XObject the interpreter does not draw (for example one without
  `/BBox`) contributes nothing.
- Text shown with `Tj` or `TJ` before any `Tf` is extracted if the
  interpreter draws it (it draws such ASCII text in a fallback font).
  `'` and `"` without a font show nothing.
- An operator with extra operands is read as the interpreter reads it.
- Fill and stroke of one glyph (mode 2, or 6) give its characters once.
- Text in clipping modes 4 to 7 is extracted once, where it is drawn, in
  content order. Mode 7 text is extracted although nothing is painted.
  (The interpreter paints nothing for mode 7. The implementer needs a way
  to see those glyph runs; see Q1.)
- Text inside Type3 glyph procedures and inside pattern cells is not
  extracted (1.4).

The characters of a glyph are the ones spec 05 gives its character code in
its font (05 §2.3, `FontText::text`). Where spec 05 gives none, the
interpreter's own reading (`as_unicode`) is used, and a glyph with no
reading at all reads as U+FFFD. Each character then passes through
`normalise_extracted` (05 §4.3): ligatures and presentation forms become
their component characters. All characters of one glyph share the glyph's
position: the first takes the glyph's advance, the others have none.

Matching glyph runs to their codes is the implementer's design problem:
the interpreter does not report codes. Q1 proposes extending the vendored
interpreter to report, per glyph, the code, the font dictionary and the
text rendering mode. Whatever the design, test N1 must hold: a glyph is
never given the characters of a different show operation.

### 3.2 Character geometry

For a glyph with em-to-page matrix *T* (the glyph's text-space transform,
the CTM and the page frame composed, scaled so that one unit is one em):

- `origin` = *T* applied to (0, 0);
- `size` = sqrt(|det *T*|);
- the advance *a* (in ems) is the glyph's horizontal displacement without
  character and word spacing (§9.4.4): from `/Widths` or `/W` for
  composite fonts, else the program's advance; for Type3 fonts from
  `/Widths` through `/FontMatrix` (§9.6.4);
- `quad`: the corners *T*(0, asc), *T*(a, asc), *T*(0, desc),
  *T*(a, desc), with asc and desc from the font (3.3), in the order ul,
  ur, ll, lr;
- vertical writing (WMode 1, §9.7.4.3): origin and advance follow the
  vertical metrics (`/W2`, `/DW2`); the line direction is down the page.

A glyph whose transform is not finite or not invertible is dropped.

### 3.3 Font facts (`TextFont`)

One `TextFont` per font dictionary the page uses (two dictionaries with
equal contents may share one). Facts come, in order of preference, from:

1. **ascender, descender**: available sources are the font descriptor's
   `/Ascent` and `/Descent` (§9.8.1, glyph space, 1000 units per em), the
   embedded program's own vertical metrics (OpenType `hhea`/`OS/2`, or the
   `/FontBBox` of a Type1 or CFF program), the font the view substitutes
   for a non-embedded font (the URW face of `view::fonts`, its program or
   AFM), and for Type3 fonts the `/FontBBox` through the `/FontMatrix`.
   Which source wins, and how implausible values (a descent with the wrong
   sign, zeros, ascent below descent) are rejected, is the implementer's
   design. Failing all sources, a default pair of the implementer's choice
   whose sum of magnitudes is about one em.
   **Acceptance:** for each of the 14 standard fonts, a quad's top is at
   or above the font's cap height and its bottom at or below its
   descender line (from the URW AFM), and the quad is at most 1.6 em
   high.
2. **bold**: descriptor `/Flags` ForceBold (bit 19, §9.8.2), or
   `/FontWeight` ≥ 600, or the program's `OS/2` fsSelection bold bit or
   `head` macStyle bold, or a base name containing "Bold", "Black",
   "Heavy" or "Semibold".
3. **italic**: `/Flags` Italic (bit 7), a non-zero `/ItalicAngle`, the
   program's italic or oblique bits, or a base name containing "Italic"
   or "Oblique".
4. **monospace**: `/Flags` FixedPitch (bit 1), or the program's `post`
   isFixedPitch, or a Courier face.
5. **serif**: `/Flags` Serif (bit 2), or the program's `OS/2` sFamilyClass
   in a serif class (classes 1 to 5 and 7 of the OpenType spec), or a
   Times face. Otherwise sans-serif. Type3 fonts are neither.
6. **name**: `text_font_name` (2.6).

These drive only the CSS description of line summaries and the quad
height. The tests compare them on the fixtures (8.1).

### 3.4 Spaces between glyphs

Within a line, a space character is inferred between two consecutive
glyphs when the gap between the end of the first glyph's advance and the
start of the second, measured along the line in ems of the second glyph's
size, is large enough, and neither of the two characters is already
whitespace. An inferred space is a `TextChar` with `c == ' '`,
`synthetic == true`, placed at the first glyph's pen position with a
width of the gap.

The threshold is the implementer's choice, inside a band:

| Gap (em) | Required |
|---|---|
| ≤ 0.10 | no space (kerning, tracking, ellipsis kerns) |
| 0.10 to 0.25 | either; choose to maximise the corpus target of 8.3 |
| ≥ 0.25 | a space, up to the line-break gap below |

Rationale: the space glyph of common text faces is 0.25 to 0.33 em wide
(URW AFMs: Times 0.25, Helvetica 0.278, Courier 0.6), and justified text
compresses it by up to about a third. Kerning pairs and tracking stay well
under 0.1 em. Experiment E3 records where MuPDF switches.

- Character spacing (`Tc`) and word spacing (`Tw`) gaps count like any
  other gap, so letter-spaced text reads "a b c" past the band.
- A negative gap (overlap) of less than about one em continues the line
  without a space.
- A gap along the line much larger than a word space (≥ 3 em is always
  such a gap; the exact value is the implementer's choice) may start a new
  line in the same block instead of a space. The corpus comparison
  treats both alike.
- Inferred spaces are never inserted around characters of scripts written
  without spaces (CJK ideographs, kana, Hangul syllables are exempt from
  this rule: Hangul uses spaces), or after a character that already is
  whitespace.
- Where an inferred space falls on a run boundary of `span_blocks`, it
  belongs to one of the two runs (tests trim).

### 3.5 Lines and blocks

Characters join the current line while all of these hold:

1. the new glyph's line direction is parallel to the line's (within a
   small angle);
2. its baseline is offset from the line's baseline, perpendicular to the
   direction, by no more than a superscript or subscript would be: an
   offset ≤ 0.4 em must join, an offset ≥ 1 em must not; between is the
   implementer's choice;
3. it does not jump backwards along the line by more than about one em
   (an overlap, 3.4) or forwards by more than the line-break gap of 3.4.

Otherwise a new line starts. A new line starts a new **block** when:

- its direction differs from the previous line's; or
- its baseline is far from the previous line's: a distance ≤ 1.3 em must
  stay in the block (single and 1.15 spacing), a distance ≥ 3 em must
  start a new block, between is the implementer's choice (a design that
  learns the block's own line spacing is welcome); or
- it starts noticeably to the right of the block's first line (a
  first-line indent): ≥ 1 em must start a block, ≤ 0.1 em must not. A
  line starting to the left (outdent) does not start a block.

Blocks and lines are kept in drawing order (1.4). Experiment E4 records
MuPDF's choices.

A glyph drawn a second time at the same place (same characters, same
size, origin within 0.01 em) is the same text (fake bold, fill then
stroke, overprinted copies): it is extracted once. Merging copies offset
by up to 0.05 em is recommended (shadow and fake-bold effects), not
required.

### 3.6 Writing direction

Each line's characters are in **logical (reading) order**.

- Text in a left-to-right script drawn left to right is in drawing order.
- Right-to-left scripts (Bidi_Class R or AL: Hebrew, Arabic, Syriac,
  Thaana, N'Ko and the RTL blocks beyond the BMP) are usually drawn in
  visual order: the glyphs left to right on the page. The line's logical
  order is found by applying the Unicode Bidirectional Algorithm (UAX #9)
  in reverse: take the line's characters in visual order, choose the
  paragraph direction (right to left when the line's strong characters
  are mostly R or AL; the implementer may refine this), resolve weak and
  neutral types (W1–W7, N1–N2, I1–I2), and reorder with rule L2 (L2 is
  its own inverse for the cases tested). Numbers and Latin words inside
  right-to-left text keep their left-to-right order; combining marks
  (NSM) stay after their base character. In a reversed run, characters
  with a Bidi_Mirroring_Glyph (rule L4, `BidiMirroring.txt`) are replaced
  by their mirror, so a drawn "(" at the left end of a right-to-left word
  reads as the closing ")" it is.
- Some writers draw right-to-left text in logical order with the pen
  moving left. The line is then built from the glyphs' positions, so it
  reads the same as the visual-order case.
- Bidi classes and mirrors come from spec 05 (`bidi_class`,
  `bidi_mirror`). The `unicode-bidi`
  crate (MIT/Apache-2.0) may be used if the owner approves the dependency.

Expected results, glyphs drawn left to right, Hebrew letters written
here as A–E (U+05D0–U+05D4):

| Drawn | Logical text |
|---|---|
| `ABC DE` | `ED CBA` |
| `ABC 12 DE` | `ED 12 CBA` |
| `ABC. DE` | `ED .CBA` (the period at the visual left end of the first word is its logical end) |
| `XY ABC DE ZW` (Latin XY, ZW) | `XY ED CBA ZW` |
| `(ABC)` | `(CBA)` |

These are the UAX #9 readings. MuPDF differs on the second and fourth and
fifth rows (E5); this spec follows UAX #9 because the result is what a
reader of the page reads.

A line summary should not mix directions: a direction change is a run
boundary (2.4), so that the browser lays each summary out in one
direction.

### 3.7 ActualText

ActualText replaces the text of a marked-content sequence (§14.9.4):

- **Sources**: the `/ActualText` of a `BDC` property list, given inline or
  through the page's `/Properties` resource; or, for a sequence whose
  property list has an `/MCID` and none of its own, the `/ActualText` of
  the structure element the page's `/StructParents` entry in the
  structure tree's `/ParentTree` (a number tree, §7.9.7) gives for that
  MCID (§14.7.5.4). The element is the one at index MCID of that array;
  the lookup must take time linear in the page's content (test: 6,000
  MCIDs in under 3 s).
- The string is a text string (§7.9.2.2), decoded with
  `object::decode_text_string`. U+0000 characters in it are dropped.
- The outermost ActualText wins: nested sequences inside one with
  ActualText contribute nothing of their own.
- **Placement over glyphs**: the replacement characters take the place
  of the glyphs drawn inside the sequence, in that area of the page.
  Characters that equal the glyphs' own at the start and the end keep
  those glyphs' positions. The others are spread over the remaining
  glyphs; any surplus sits at the end of the last replaced glyph with no
  advance. Surplus characters stay on the glyphs' line.
- **Placement over no glyphs**: when the sequence draws no glyph but
  paints images, shadings or paths, the text is placed at the bottom left
  of the union of what it painted, one em the size of that union, without
  advance (one line). When it paints nothing, the ActualText is dropped.
  (Paths are new: the current engine ignores them; vector-drawn lettering
  with ActualText becomes readable, test N9.)
- An empty ActualText removes the sequence's text.
- ActualText characters count toward `MAX_PAGE_CHARS`. A string whose
  decoded length would pass the limit refuses the page.

### 3.8 Visibility (which layout holds a character)

The page area is the frame of 5.1.

- `unclipped_blocks` holds every character drawn (3.1), wherever it lies.
- `blocks` and `span_blocks` hold the characters that are **visible**:
  their glyph box intersects the page area and every clip in force
  (`W`/`W*` paths, clipping text, form `/BBox`, the clip of annotations is
  not involved). The glyph box is the glyph's ink bounds; an inkless glyph
  (a space) is visible when its em box (advance × ascender..descender)
  intersects the clip. Using the em box for every glyph is also
  acceptable. A glyph partly inside is visible.

Search uses `unclipped_blocks` on purpose: text hidden by a clip or placed
off the page is still found, so a user can find and redact it.

### 3.9 Controls and whitespace in output

- Characters that spec 05 maps to controls never appear: spec 05 turns
  whitespace controls into U+0020 and drops or replaces the others.
- `as_text` keeps the page's own space characters (U+00A0 stays U+00A0;
  test `nbsp_extracts_as_itself_and_soft_hyphens_are_not_drawn`).

---

## 4. Tolerant behaviour (observed and chosen)

Each MuPDF fact below has an experiment in section 7; rows marked "not
recorded" state only this spec's choice and need no MuPDF evidence.

| Input | MuPDF 1.28.1 | This spec |
|---|---|---|
| Glyphs right to left, mixed digits (E5) | `" CBA12 ED"` | `"ED 12 CBA"` (UAX #9) |
| Logical-order RTL drawn with the pen moving left (E5) | `ABC`, search for the reversed word fails | logical order, search works |
| Same text drawn twice 0.3 pt apart (E3) | twice | once (recommended), twice allowed |
| Text in mode 4 to 6 (clip and paint) | not recorded in section 7 | once |
| Mode 7 text in a form | not recorded in section 7 | extracted (the current engine does not) |
| ActualText over a path only | not recorded in section 7 | placed on the path's bounds |
| Nested ActualText (E8) | `"outer inner"` | `"outer"` (§14.9.4: the outer replaces all its content) |
| ActualText with U+0000 | not recorded in section 7 | NULs dropped, rest kept |
| CropBox outside the MediaBox (E1) | Letter page, blank | the MediaBox, drawn (5.1) |
| `/UserUnit 0` (E1) | a page of size 0 | ignored (1) |
| `/Rotate 44.6` (E1) | 90 | 0 (nearest quarter turn) |
| Two text columns on one baseline (E4) | rows, in drawing order | drawing order (1.4) |

---

## 5. Required behaviour: page frame and pixel size

### 5.1 Page frame

Inputs, each inherited through `/Parent` when absent from the page
(§7.7.3.4; the walk up `/Parent` is bounded, at a depth of the
implementer's choice no smaller than the strict page walk accepts, and
terminates on cycles; a page written directly inside its parent's `/Kids`
still finds the parent's values): `/MediaBox`, `/CropBox`,
`/Rotate`. `/UserUnit` (§14.11.2.1, not inheritable) is read from the page
only.

1. **Boxes** are rectangles (§7.9.5): four numbers, normalised so that
   x0 ≤ x1 and y0 ≤ y1. A box that is not an array of four numbers is
   absent.
2. **MediaBox**: absent or empty (zero width or height) → US Letter,
   [0 0 612 792] (E1: MuPDF does the same).
3. **Visible area** = CropBox ∩ MediaBox (§14.11.2: the crop box is
   clipped to the media box). Without a CropBox, the MediaBox. When the
   intersection is empty, the MediaBox (the CropBox is ignored). MuPDF
   shows a blank Letter page here (E1); a page with content is better
   shown whole.
4. **Minimum size**: a side shorter than 1 point is made 1 point long
   (from x0, resp. y0).
5. **UserUnit**: a finite number > 0 scales the area (1 unit = UserUnit
   points). Anything else is 1.
6. **Rotate**: a number (integer or real; anything else is 0), taken
   modulo 360 into [0, 360), then snapped to the nearest multiple of 90;
   values exactly halfway (45 + 90k) snap up. 360 is 0. Examples: −90 →
   270, 450 → 90, 44 → 0, 45 → 90, 44.6 → 0, 135 → 180, 179.6 → 180,
   `(90)` → 0.
7. **Size**: w = (x1 − x0) × unit, h = (y1 − y0) × unit; `width`,
   `height` = (w, h) for rotations 0 and 180, (h, w) for 90 and 270.
8. **Transform** `ctm` maps user space to page space (origin at the top
   left of the rotated visible area, y down, in points), with u = unit:
   - 0: (x, y) → (u(x − x0), u(y1 − y))
   - 90: (x, y) → (u(y − y0), u(x − x0))
   - 180: (x, y) → (u(x1 − x), u(y − y0))
   - 270: (x, y) → (u(y1 − y), u(x1 − x))

   These follow from §14.11.2 and §7.7.3.3 (`/Rotate` turns the page
   clockwise when displayed).

### 5.2 Pixel size

`render(page, scale, ..)` makes an image of `px(width) × px(height)`
pixels, where px(v) is the smallest integer ≥ v × scale, except that a
product within a tiny tolerance ε above an integer rounds down to that
integer (so that 612 × 1.5 is 918 although floating point may say
918.0000000001). ε is the implementer's choice in [1e-4, 2e-3] pixels.
MuPDF's switch point lies between 0.0009 and 0.0011 pixel (E2), and
render size equality with MuPDF is part of the corpus gate (8.3).
The image origin is the frame's top left; the size does not depend on
x0 or y0 (E2: [0.4 0 100.4 100] at scale 1 is 100 pixels wide).

`BadScale` when the scale is not finite and positive, when a side would
be 0 or more than `MAX_RENDER_EDGE`, or when the pixel count would pass
`MAX_RENDER_PIXELS`. Both constants are the project's (they stay in
`render.rs`). px never exceeds ceil(v × scale), so the app's own edge
check (1.2) is never looser than the render.

### 5.3 Errors and limits

`PageFrame::new` never fails: it reads what it can and falls back as
above.

### 5.4 One rule for the app and the view

`app::info::page_size`, `app::info::display_rotation` and
`app::ops::nav::links` hold their own copy of the page-frame rule, reading
the boxes through the strict document model. They must agree with the
view on every page. The implementer provides the rule as a pure function
over numbers (for example
`pub(crate) fn frame_of(media: Option<[f64; 4]>, crop: Option<[f64; 4]>, rotate: Option<f64>, user_unit: Option<f64>) -> FrameNumbers`)
and changes those three app functions to call it. That change is in
scope for this workstream; their comments that cite MuPDF are reworded
(8.5).

---

## 6. Limits and safety

1. No panics, no `unsafe`, no unbounded recursion. All arithmetic on
   coordinates tolerates NaN and infinity (drop the glyph).
2. Work per page is bounded by the walk's budgets (forms, content bytes,
   shown bytes, fonts) and by `MAX_PAGE_CHARS`. Structure-tree lookups
   are bounded by the content's MCIDs (linear, 3.7). Number-tree descent
   is depth-bounded.
3. **Reader memory**: a `TextReader` caches fonts' `FontText` and the
   predefined CMaps (spec 05 `CMaps`) across pages, and starts afresh
   when the cache grows past a bound (decoded font program bytes, number
   of fonts, number of pages) of the implementer's choice. Acceptance:
   `text-all` on `fixtures/large.pdf` within 2× the baseline time and
   +10% peak RSS (8.3); a page of 200 non-embedded Japan1 fonts stays
   under 300 MB peak.
4. Performance tests that stay: eight TrueType fonts sharing a program
   with a 500,000-group format 12 cmap read in under 1.5 s (spec 05);
   6,000 MCIDs in under 3 s.
5. Layout cost must be O(n log n) or better in the page's characters
   (a page of `MAX_PAGE_CHARS` characters must lay out in seconds, not
   minutes).

---

## 7. Recorded black-box experiments

### 7.1 Method

Scripts (self-authored, no MuPDF code) in
`reviewer archive spec-04-05/`:

- `lib.mjs`: builds classic-xref PDFs; one 300 × 300 page; Helvetica
  WinAnsi as `/F1` unless stated.
- `exp04.mjs [geom|pix|space|lines|bidi|search|actual]`: the experiments
  below. Run: `slot node exp04.mjs > exp04.txt`.
- Oracle: MuPDF 1.28.1 (npm `mupdf`, the repository's `node_modules`):
  `page.getBounds()`, `page.toPixmap(Matrix.scale(s, s), DeviceRGB)`,
  `page.toStructuredText("preserve-whitespace").asText()`,
  `page.search(needle)`.
- pdf.js and Acrobat were not run (not installed). Raw output:
  `exp04.txt`.

### 7.2 E1 page bounds (MediaBox [0 0 400.5 500.25] unless stated)

| Page entries | MuPDF bounds | This spec |
|---|---|---|
| none | 400.5 × 500.25 | = |
| `/CropBox [10.3 20.7 300.55 400.25]` | 290.25 × 379.55 | = |
| `/CropBox [500 600 700 800]` (outside) | 612 × 792 | 400.5 × 500.25 (rule 3) |
| `/CropBox [0 0 0.5 0.5]` | 1 × 1 | 1 × 1 |
| `/CropBox [0 0 100 0.4]` | 1 × 1 | 100 × 1 (rule 4) |
| `/Rotate 90`, `-90`, `450` | 500.25 × 400.5 | = |
| `/Rotate 44` / `45` / `46` | unrotated / turned / turned | = |
| `/Rotate 135` | unrotated (180) | = |
| `/Rotate 44.6` / `44.4` | turned / unrotated | unrotated / unrotated |
| `/Rotate 89.5`, `91` | turned | = |
| `/Rotate (90)` | unrotated | = |
| `/UserUnit 2` | 801 × 1000.5 | = |
| `/UserUnit 0.5 /Rotate 270` | 250.125 × 200.25 | = |
| `/UserUnit -1` | 400.5 × 500.25 | = |
| `/UserUnit 0` | 0 × 0 | 400.5 × 500.25 (rule 5) |
| MediaBox `[400.5 500.25 0 0]` | 400.5 × 500.25 | = |
| MediaBox `[0 0 400]`, `[0 0 0 0]`, absent | 612 × 792 | = |
| MediaBox `[10 20 10.5 400]` | 1 × 1 | 1 × 380 (rule 4) |
| MediaBox `[-100 -200 200 100]` | 300 × 300 | = |
| MediaBox [0 0 200 100] and `/Rotate 90` on `/Pages` | 100 × 200 | = |

### 7.3 E2 pixel size (MediaBox [0 0 W 100])

| W | scale 1 | 1.5 | 1.173 | 0.25 | 3 |
|---|---|---|---|---|---|
| 100 | 100 | 150 | 118 | 25 | 300 |
| 100.0004 | 100 | 150 | 118 | 25 | 301 |
| 100.0009 | 100 | 151 | 118 | 25 | 301 |
| 100.0011 | 101 | 151 | 118 | 25 | 301 |
| 100.01 | 101 | 151 | 118 | 26 | 301 |
| 100.5 | 101 | 151 | 118 | 26 | 302 |
| 99.9995 | 100 | 150 | 118 | 25 | 300 |
| 612 | 612 | 918 | 718 | 153 | 1836 |

Height 100 gave 100, 150, 118, 25, 300 throughout. MediaBox
[0.4 0 100.4 100] and [0.7 0.7 100.2 100.2] at scale 1: 100 × 100.

### 7.4 E3 gaps on one line (Helvetica, `[(a) -N (b)] TJ`)

| N (thousandths of an em) | 50–140 | 160–700 | 800–2000 |
|---|---|---|---|
| 10 pt and 12 pt | `ab` | `a b` | `a` newline `b` |

- `Tc` at 10 pt on `(abc)`: 0.5–1.4 → `abc`; 1.6–3 → `a b c`.
- Positive adjustments (overlap) 100–500 → `ab`; 900, 1500 → new line.
- `(ab)` and `(cd)` in separate text objects 1 em apart → two lines.
- `[(a ) -300 (b)]` → `a b` (no second space after a drawn one).
- `(Bold)` drawn twice, 0.3 pt or 3 pt apart, separate objects → twice.
- `2 Tr` → once.

### 7.5 E4 lines and blocks (Helvetica 10 pt)

- Second line `0 -L Td`: L = 0 to 15 → same block; 16, 20, 30 → new
  block.
- Third line indented by 0.3 pt → same block; 0.6, 1, 5, 20 pt → new
  block.
- Rise (`Ts`) of 2, 3, 5 pt → same line (`x2 y`); 8 pt → separate lines
  and blocks.
- Two columns on shared baselines, drawn row by row → rows in drawing
  order.
- Lines drawn bottom first → drawing order.
- A colour change inside a text object, preserve-spans mode → two lines.

### 7.6 E5 right to left (ToUnicode a–e → U+05D0–U+05D4)

| Drawn left to right | MuPDF text |
|---|---|
| `abc de` | `ED CBA` |
| `abc 12 de` | ` CBA12 ED` |
| `XY abc de ZW` | `XY  ED CBAZW` |
| `abc. de` | `ED .CBA` |
| `(abc)` | `()CBA` |
| a, b, c each placed 6 pt left of the previous | `ABC`; search for the logical word finds nothing |

### 7.7 E6 search

On "The Cat sat on the cat-" / "alog with  double  spaces" /
"CAT cat Cat café": `cat` 2, `Cat` 2, `CAT` 1, `catalog` 1, `cat-alog` 0,
`with double` 1, `spaces CAT` 1, `cafe` 0, `café` 1. A U+FB01 ligature
reads `fi` and is found by `find`. Text clipped away or off the page:
not in the text, found by search.

### 7.8 E8 ActualText

- `/Span << /ActualText (hyphen-less) >> BDC (hy-) Tj EMC (phen) Tj` →
  `hyphen-lessphen`.
- ActualText `ABCDEFG` over `(AB)` → `ABCDEFG`.
- UTF-16 ActualText `fi` over one glyph → `fi`.
- Empty ActualText over `B` between `A` and `C` → `A C`.
- Nested `outer` / `inner` → MuPDF `outer inner`; this spec `outer`.

---

## 8. Acceptance

Classes: **keep** (unchanged); **modify** (assertion changed by a
decision here); **replace** (MuPDF-equality assertion rewritten as a
tolerance or text-equivalence check); **delete**; **scrub** (comment or
name cites MuPDF internals or copied constants; 8.5).

"Text-equivalent" below means: equal after replacing every run of
whitespace by one space and trimming, unless a test is about spacing.
"Quad tolerance" means: x along the baseline within 0.05 pt of the
advance-derived value; top and bottom within 0.35 em of the old value
(the ascender source changes, 3.3).

### 8.1 Existing tests

**`engine/tests/view_text.rs` (13): 2 keep, 2 modify, 5 replace, 2
delete, 2 move to spec 05.**

| Test | Class | New check |
|---|---|---|
| `clipping_text_is_extracted_at_the_end_of_its_text_object` | replace | Modes 4, 5, 6, 7: the shown text once, in content order (`7 Tr (AB) Tj 4 Tr (CD) Tj` → `ABCD` on one line); without `ET` the mode 7 text is still extracted. Search finds clip-only text once. |
| `glyphs_are_clipped_by_their_bounds_and_inkless_ones_by_their_origin` | replace | Visibility per 3.8: a glyph wholly outside the clip is absent; partly inside present; the trailing space case may read `A` or `A `. |
| `right_to_left_runs_take_neutral_characters_as_mupdf_does` | replace | The table of 3.6. Rename `right_to_left_lines_read_in_logical_order`. |
| `text_spans_start_where_mupdf_flushes_its_text` | replace | Run boundaries of 2.4: font, size, colour (`0 g`, `1 0 0 rg`), mode, text object split `span_blocks`; `Tc`, `Tw`, `Tz`, `Td` inside one object do not. Other operators unpinned. `blocks()` never split by any of them. |
| `actual_text_over_no_text_is_placed_on_the_images_it_covers` | modify | Image case keeps `Bookkeeper`, origin at the image's bottom left (±0.01). Path case now `Hello` on the path's bounds. |
| `type3_glyphs_whose_forms_fill_without_a_colour_are_drawn_directly` | delete | Tested a detail of the old implementation's Type3 handling. N2 covers Type3 text through forms. |
| `to_unicode_cmaps_read_as_leniently_as_mupdf` | move to spec 05 (05 §8) | |
| `s22pdf_chinese_fonts_read_as_gbk_with_full_em_advances` | delete | Rule dropped (05 Q2). |
| `structure_elements_are_found_by_mcid_in_linear_time` | keep | |
| `text_shown_before_any_font_is_set_is_extracted_as_drawn` | keep | 3.1. |
| `gaps_of_the_space_threshold_read_by_how_they_were_made` | replace | Band test N3. |
| `each_glyph_run_reads_the_show_operation_that_drew_it` | keep, modify last case | The form that sets `7 Tr` itself: `XY` is now extracted (`XY\n\nCD`). |
| `truetype_cmaps_with_many_groups_are_read_by_bisection` | move to spec 05 | |

**`engine/tests/view.rs` (21 tests, 17 about this spec):**

| Test | Class | New check |
|---|---|---|
| `render_size_matches_mupdf_on_fixtures` | keep, rename `render_sizes_on_fixtures` | Values are fixture facts. |
| `render_size_follows_mupdf_page_geometry` | modify, rename `render_size_follows_page_geometry` | `/Rotate 44.6` now unrotated; `/CropBox [0 0 0 0]` now the MediaBox (101 × 126 at 0.25); `/CropBox [0 0 0.5 0.5]` stays 1 × 1. |
| `encrypted_documents_need_their_password`, `owner_password_opens_encrypted_documents` | keep | Plain fixture strings. |
| `page_text_matches_mupdf` | replace, rename `page_text_of_fixtures` | Text-equivalent to the listed strings, except `p6/paragraph.pdf` "Head runtail run", whose gap is checked against the band (N3). Block counts checked only where 3.5 decides (gaps ≥ 3 em). |
| `structure_actual_text_keeps_the_matching_glyphs` | modify | The ActualText's `P` lies within the replaced glyphs' bounds. |
| `line_summaries_match_mupdf_parse_lines` | replace | `text` per 2.4 (RTL fixture in logical order), `x`, `y`, `size` exact, `w` ±1, `top` and `h` within 0.35 em, `family`/`weight`/`style` exact. |
| `spans_split_lines_at_text_objects_and_fonts` | keep | Leading space of `" World"` may be on either run (compare trimmed). |
| `layout_and_clipping_follow_mupdf` | replace | Indent 20 pt → new block; outdent → none; `PartClip` → `P` (and possibly `Pa`); mode 3 and mode 2 once; `2 Tc` at 12 pt → `S p a c e d` or `Spaced` (0.17 em lies in the band where 3.4 allows either); `[(A) -200 (B) -500 (C) -1000 (D)]` → `A B C` then `D` either on a new line or after a space; off-page text absent. |
| `search_matches_mupdf_hits_and_quads` | modify | Counts keep; quads with quad tolerance. |
| `rotated_page_quads_match_mupdf` | modify | Quad tolerance; `dir` exact. Rename `rotated_page_quads`. |
| `self_invoking_and_exploding_content_is_refused` | keep | |
| `search_reads_clipped_and_off_page_text` | modify | Counts keep, quad tolerance. |
| `search_whitespace_needles_and_truncation` | keep, quad tolerance | |
| `spans_layout_matches_mupdf_preserve_spans` | replace | `rtl.pdf` lines in logical order. `p6/quartz.pdf`: drop `reference/app-view-20261014/quartz-spans.txt` (MuPDF output); check that the words of `as_text` equal those of `spans_text` and that summaries equal `span_blocks` lines in count. |
| `type3_text_comes_from_encoding_names_and_widths` | modify | Per spec 05: `T_h` → `Th`, `a67` at code 162 → U+FFFD, `u00E9` → é, widths from `/Widths`; Type3 font name per 2.6. |
| `rotated_page_text_quads_match_mupdf` | modify | Quad tolerance, summaries `x`/`y`/`size` exact. |
| `render_is_opaque_rgba_on_white`, `render_options_and_no_view_annotations`, `oversized_images_are_refused_before_allocation`, `popup_and_no_view_annotations_are_not_drawn` | keep | Not about this spec beyond pixel size. |

**`engine/tests/app_view.rs` (20 tests, 13 about this spec):** keep
`render_replies_with_pixels_text_lines_and_counts`,
`render_and_thumbnail_refuse_as_engine_ts_does`,
`thumbnail_pins_the_long_edge_and_carries_no_text`,
`search_trims_like_javascript_and_checks_the_query_length`,
`search_hits_follow_page_order_and_the_budget` (scrub),
`search_has_no_per_page_quad_cap` (scrub),
`text_all_joins_trimmed_pages_and_honours_the_copy_bit` (scrub),
`text_all_on_large_pdf_has_every_page`,
`layer_set_is_view_only_and_every_view_honours_it`,
`render_after_an_edit_shows_it_and_reports_the_revision`,
`a_page_showing_more_text_than_the_limit_is_refused`,
`a_page_whose_text_expands_past_the_character_limit_is_refused`,
`text_all_and_search_stay_right_across_reader_cache_resets`.
**Modify** `degenerate_page_boxes_draw_and_read_as_mupdf_does` (rename
`degenerate_page_boxes_draw_and_read`): the three CropBox-outside cases now
show the MediaBox, draw and read `Hi`; search total stays 6; `text-all`
becomes `Hi` on all six pages.

**Other Rust tests that depend on this spec (all keep unless stated):**

- `app_scan_text.rs`: `right_to_left_runs_draw_in_visual_order_and_extract_as_typed`,
  `numbers_and_pointed_letters_in_right_to_left_runs_extract_as_typed`
  (scrub the MuPDF/PDFKit comment), `nbsp_extracts_as_itself_and_soft_hyphens_are_not_drawn`.
  The first two get stronger with 3.6: add whole-line checks (N6).
- `view_cjk_font.rs`, `view_cjk_font_metrics.rs`: 2 tests.
- `view_robust.rs`: `marked_content_finds_its_structure_element_by_mcid`.
- `view_hostile.rs`: `pages_past_the_kept_content_draw_from_content_decoded_again`
  and the tests that call `structured_text` for refusals (4).
- `view_inherited.rs`: 2 tests that read text within budgets.
- `app_core.rs`: `page_geometry_matches_the_view_on_every_fixture_and_odd_boxes`
  (modify: the odd boxes follow 5.1; the shared function makes it hold),
  `hayro_reads_an_overlay_on_classic_xref_stream_and_incremental_bases`.
- `app_annots.rs`: `page_space_matches_the_view_on_every_rotation`.
- `app_nav.rs`: `rects_follow_rotation_crop_box_and_user_unit` (scrub).
- `app_redact.rs`: `search_marks_every_hit_with_quads_and_the_tag`
  (modify: rect with quad tolerance in y, x within 0.05),
  `search_finds_text_in_a_hidden_layer`,
  `a_moved_search_mark_becomes_an_area_mark_and_undo_restores_it`.
- `app_redact_apply.rs`: `search_with_optional_content_reads_what_the_view_reads`,
  `text_a_membership_shows_only_while_a_group_is_off_is_found_and_removed`.
- `app_redact_bounds.rs`: `clips_in_a_form_under_many_search_hits_stay_bounded`,
  `page_size_clips_around_every_run_share_their_regions`.
- `app_misc.rs`: `compare_text_diffs_lines_and_refuses_as_engine_ts`,
  `compare_text_refuses_a_page_of_more_than_2000_lines`,
  `batch_file_stamps_a_separate_document_and_leaves_the_session_alone`.
- `app_edit*.rs`, `app_accept_edit.rs`: every content-edit test depends on
  `span_blocks` matching runs (2.4). All keep; they are the acceptance for
  run boundaries.
- Unit tests in the replaced files: `stext.rs`
  `type3_glyph_procedures_are_checked_once_each_for_the_codes_shown`
  (delete with the file).

**App tests (`tests/*.mjs`).** After phase 0 removes the TypeScript
engine, these run on the Rust engine only. Files that assert text,
search, quads or lines: `fixtures`, `p1-worker`, `p2-decorate`,
`p2-edit`, `p2-export`, `p2-nav`, `p2-shell`, `p3-drive`, `p4-redact`,
`p6-batch`, `p6-bates`, `p6-compare`, `p6-direct3`, `p6-paragraph`,
`m9-recovery`, `rust-cjk-font`, `rust-engine-bounds`,
`rust-engine-inherited`, `rust-engine-robust`, `rust-redact-bounds`,
`rust-scan-compose`, `worker-build` (22 files). They assert app
behaviour (hits found, text present, edits land); keep them all. Where one
compares quads or line geometry numerically, apply the quad tolerance.
Where one compares Rust output with MuPDF output, that comparison is
replaced by the expected value of this spec, or removed if it only
restated parity.

### 8.2 New cases

Each is built in the test from a constructed page (Helvetica unless
stated).

| Case | Expect |
|---|---|
| N1 | Shows in a form the interpreter skips (no `/BBox`), then shows on the page with the same font and glyph count: page text only, each at its own position. |
| N2 | Type3 font whose glyph procedures draw a form: text from the encoding names, one line. |
| N3 | Band: `[(a) -N (b)] TJ` at 10 pt and 12 pt: N = 50, 100 → `ab`; N = 250, 300, 500 → `a b`. `Tc` letter spacing of 0.3 em → spaces; 0.05 em → none. A real space followed by a gap of 0.3 em → one space. |
| N4 | Lines at leading 1.0, 1.2 em → one block; 3.5 em → two blocks. Indent of 2 em → new block; 0.05 em → none. Rise of 0.33 em → same line. |
| N5 | The same string drawn twice at the same origin in two text objects → once; in mode 2 → once. |
| N6 | 3.6 table, glyphs drawn in visual order; and the same Hebrew words drawn in logical order with the pen moving left: identical text and search hits for each word. Arabic with combining marks: each mark follows its letter. |
| N7 | Mode 7 text with no `ET`, mode 7 text inside a form, mode 7 followed by mode 0 in one object: each extracted once. |
| N8 | Nested ActualText → outer only; ActualText containing U+0000 → NUL dropped; ActualText with surplus characters → all characters on the glyphs' line; search for the ActualText finds one hit whose quads lie within the replaced glyphs' bounds. |
| N9 | ActualText over a filled path only → placed on the path's bounds; over nothing at all → dropped. |
| N10 | Frame: every row of 7.2 with this spec's value, at scales 0.25, 1, 1.5, 4. |
| N11 | Pixel size: every row of 7.3. The rows pin ε to [0.0009, 0.0011); this narrower range overrides the [1e-4, 2e-3] of 5.2. |
| N12 | Ligatures U+FB01 and U+FB03 read `fi`, `ffi`; search for `office` finds the word drawn with the ligature. |
| N13 | `TextFont` facts per 3.3 for: each standard 14 name non-embedded; descriptor flags Serif+Italic; ForceBold; FontWeight 700; an embedded TrueType with OS/2 bold and post fixed pitch. |
| N14 | Vertical writing (Identity-V with the CJK fallback): one line, `dir` = (0, 1), characters top to bottom. |
| N15 | Rotated text (90° `Tm`) on an unrotated page: its own line, `dir` = (0, −1). |
| N16 | `spans_text` of `(Red) Tj 1 0 0 rg (Blue) Tj` is `RedBlue\n\n`; `line_summaries` has two records. |

### 8.3 Corpus acceptance

Reviewer-side gate: the runner is `engine/oracle/corpus-run.mjs` (AGPL,
moved there in Phase 0; it drives MuPDF), so the reviewer runs it, not the
implementer. Run it (with the CJK font, `--jobs 1`) and compare with the
baseline `summary-ba70615.json`
(run the reviewer's corpus run). Its text figures: render
text equal on 5,850 of 5,903 pages, 12 equal ignoring spaces, 41
different; `text-all` equal on 4,854 of 4,901 files, 5 equal ignoring
spaces, 42 different; search totals equal in 7,021 of 7,039 queries.

The comparison is with MuPDF, which this spec deliberately departs from
in places (section 4, 3.6). Required:

1. **Characters** (owner decision T14, 2026-10-18; replaces the earlier
   page counts of equality with the reference viewer). Equality with the
   reference viewer is reported but is not a target. Required instead:
   - **No lost text.** On every page, each non-space character the
     reference viewer extracts is in this spec's text (character
     multiset), except characters 3.5 or decision T7 merges as copies,
     characters the reference reads as U+FFFD, controls or
     noncharacters (spec 05 3.7, U5), and differences an owner decision
     names. The runner lists every other page with a character
     shortfall; the target is none unexplained.
   - **No new U+FFFD.** No file reads more U+FFFD than the baseline
     engine run (spec 05 8.3), except files an owner decision names
     (T9).
   - **Reading order.** For each page, the reference viewer's
     non-space words in order and this spec's words in order are
     compared by their longest common subsequence. Pages where it
     covers less than 90% of the reference's words, and that 3.6
     (right-to-left order) does not explain, are listed and reviewed
     by eye; the target is none unexplained. The C3 acceptance tests
     must pass.
2. **Spacing**: `text-all` differing only in spaces on at most 1.5% of
   compared files (at most 73). This is the agreement target the 3.4 band
   is tuned on.
3. **Search**: totals equal in at least 7,000 queries.
4. **No new failures**: no `textall-error`, `search-error`,
   `render-error`, `timeout`, `oom`, `crash` or `rust-trap` that the
   baseline did not have; Rust-empty text on no more pages than the
   baseline (5).
5. **Geometry**: `pagesize-differs` and `render-dims` on no more files
   than the baseline, except files whose CropBox misses the MediaBox
   (5.1 rule 3), listed by the runner.
6. **Cost**: `text-all` and `search` wall time per file at the median and
   p99 no worse than 1.5× the baseline; peak RSS p99 no worse than
   +10%.

Corpus rules apply: record no document text, never read
`SBI05131482.pdf`.

### 8.4 Gates

- `cargo test` for the engine and the WASM build, through
  `slot`.
- `npm test` (the app tests) on the Rust engine.
- The corpus run (8.3).
- Independent review that the implementation was written without the old
  files.

### 8.5 Comments, names and evidence to scrub

- `tests/view_text.rs`: module comment ("against MuPDF 1.28.1", "the Chinese-font
  case") and per-case comments that state MuPDF's output.
- `tests/view.rs`: module comment; test names containing `mupdf`
  (renames above); comments that name MuPDF functions or its buffer sizes.
- `tests/app_view.rs`: comments "as MuPDF's", "MuPDF's search still
  finds…", the 500-quad comment.
- `tests/app_scan_text.rs`, `tests/app_redact.rs`, `tests/app_nav.rs`:
  comments giving MuPDF's values as the reason.
- `src/app/info.rs` (`page_size`, `display_rotation`: a comment citing a
  MuPDF function's rounding), `src/app/ops/nav/links.rs` (cites a MuPDF
  function), `src/app/ops/view.rs` ("MuPDF's silently
  under-counts"), `src/app/ops/edit/font.rs` ("names a font as MuPDF
  does"): reword as behaviour.
- Evidence to retire or move to `engine/oracle/` (AGPL, outside the MIT
  grant): `engine/reference/app-view-20261014/quartz-spans.txt` (MuPDF
  output); the text sections of `app-view-20261014/summary.json` and
  `app-view-20261015/summary.json` (MuPDF agreement figures; keep as
  history, not as a gate).
- `engine/design/app-corpus-text.md` and the text parts of
  `engine/design/app-view.md` describe the old implementation: move out
  of the implementer's tree (plan, phase 2).

---

## 9. Behaviour changes users would notice

1. **Right-to-left text copies in reading order.** Hebrew or Arabic mixed
   with numbers or Latin words now copies and searches as a reader reads
   it (3.6), where it came out scrambled.
2. **Slightly different spaces and paragraph breaks** in copied text and
   `text-all`: thresholds are new (3.4, 3.5). Within the corpus target.
3. **"Extracted text" panel** keeps a visual line on one line when the
   colour or font changes inside it (2.4).
4. **Pages whose CropBox misses the MediaBox** show their content instead
   of a blank Letter page (5.1).
5. **Search highlight heights** follow the font's real ascent and descent;
   boxes are tighter for the standard fonts (3.3).
6. **Clipping-mode text** (outlined or image-filled words) reads once and
   in place; mode 7 text inside forms is now found.
7. **Overprinted duplicates** (fake bold) read once where they coincide.
8. **ActualText on vector artwork** is now extracted.
9. Odd `/Rotate` reals (44.6) snap to the nearest quarter turn; an
   invalid `/UserUnit` no longer collapses a page.

---

## 10. Open questions for the owner

- **Q1.** Patch the vendored `hayro-interpret` to report, per glyph, its
  character code, its font dictionary and the text rendering mode, and to
  report mode 7 glyph runs (as an invisible draw)? This removes the need to
  rebuild codes from a parallel parse of the content, which is the most
  fragile part of any design. The current patch policy is "panics only";
  this would be a second kind of patch.
- **Q2.** Accept the UAX #9 departure from MuPDF for right-to-left text
  (3.6), knowing that the corpus will count those pages as different from
  MuPDF?
- **Q3.** Search stays case-sensitive and diacritic-sensitive, as the app
  works today. Most viewers default to case-insensitive. Change the
  product behaviour (an app protocol change, not this workstream)?
- **Q4.** The CropBox-outside-MediaBox choice (5.1 rule 3): show the
  MediaBox (this spec), or a blank page as MuPDF does?
- **Q5.** `spans_text` joins runs of one visual line (2.4). The corpus
  runner compares render text with MuPDF's preserve-spans text after
  whitespace normalisation, so this moves some pages from "equal" to
  "equal ignoring spaces". Accept, or keep one line per run?
- **Q6.** Optional dependency `unicode-bidi` for 3.6?
- **Q7.** Should overprinted copies offset by up to 0.05 em be merged
  (recommended in 3.5) or only exact coincidences (required)?

---

## 11. Leak-check change log (2026-10-18)

Independent leak check against SPEC-RULES.md. The layout thresholds are
acceptance bands that do not reproduce the replaced code's constants.
Edits made:

1. Header: removed the file names of the old design notes (a pointer);
   8.5 still lists them for the owner's clean-up.
2. 2.1: `MAX_SEARCH_QUADS` no longer carries a floor equal to the old
   value (and the reference program's search default).
3. 3.3 rule 1: the ascent/descent rule walked through the replaced code's
   decision sequence (sign flip, zero fallback from `hhea` to `OS/2`).
   Rewritten as the list of public sources with the choice left to the
   implementer; the acceptance criterion is unchanged.
4. 5.1: the `/Parent` depth bound copied from the old code is now the
   implementer's choice, with a floor tied to the strict page walk.
5. Section 4: four MuPDF cells had no recorded experiment (modes 4 to 6,
   mode 7 in a form, ActualText over a path only, ActualText with U+0000;
   checked against `exp04.txt`); they now say "not recorded".
6. 8.3: the corpus runner moved to `engine/oracle/` in Phase 0; marked as
   a reviewer-side gate.
7. 8.1 and 8.5: two phrases asserting MuPDF internals (Type3 caching, a
   Chinese-font special case) reworded.

Hand-off prerequisites found (not spec text): `engine/design/app-corpus-text.md`
and `app-view.md`, `engine/reference/app-view-*`, and `engine/oracle/` stay
out of the hand-off tree; the class C comments in 8.5 (`app/info.rs`,
`app/ops/nav/links.rs`, `app/ops/view.rs`, `app/ops/edit/font.rs`) are
scrubbed first, since the implementer edits two of those files (5.4).

Questions for the owner (gaps found, not filled):

- Q1 is blocking: without the interpreter reporting codes, font and
  rendering mode per glyph, 3.1 leaves the central design open.
- 2.5 rule 4: "a gap wider than a word space" splits a hit's quads; is
  the band of 3.4 the intended measure?
- 3.5: "within a small angle" for parallel lines: any band to pin?
- 8.1/8.2: which existing expected values in `view.rs` and `view_text.rs`
  are MuPDF outputs (they become "old value" under the quad tolerance)?
  Those tests' comments must be scrubbed before the implementer sees them.

## 12. Integration addenda (2026-10-18)

- **C1. Overprints in the run layout.** The rule that a glyph drawn twice at
  the same place is extracted once (3.5) applies to `blocks()`, `as_text`,
  `spans_text` and search. It does not apply to `span_blocks()`. There, each
  text-showing run keeps its own characters, because content editing
  associates content-stream runs with those lines and must see that two runs
  share one origin (app test p6-text8 (c): two runs at one origin stay
  ambiguous).
- **C2. Runs that move back.** In `span_blocks()`, a text-showing run whose
  first glyph starts well to the left of where the previous run on that
  line ended starts a new line, even within one text object and within the
  baseline band. Editing moves runs, so a moved run must stay its own line.
  Test: app test p6-paragraph "§9.1–2 move and size through any invertible
  matrix". In the "Tm-scaled 1 Tf" case,
  `BT /F1 1 Tf 12 0 0 12 72 700 Tm (Hello) Tj 0 -1.2 Td (World) Tj ET`,
  "Hello" is moved by (10, 20) through `edit-run`. Afterwards a scan must
  still find "Hello" alone, with its bounds shifted by (10, 20). The two
  runs' baselines are then 0.47 em apart.

## 12 (continued). Corpus-run addenda (2026-10-18)

Found by the corpus run of the clean-room tree against the baseline.

- **C3. A line never interleaves its runs.** 3.6 says left-to-right text is
  in drawing order; 3.5 rule 3 says a glyph that jumps backwards along the
  line by more than about one em starts a new line. Made precise:
  1. **Whose em.** In 3.5 rule 3, when the two glyphs (the new one and the
     one the line ended with) have different sizes, the em is that of the
     smaller size. A small label drawn back across a large symbol
     therefore starts its own line.
  2. **Drawing order inside a line.** Characters are kept in the order
     the page draws them. Position along the line decides order only for
     right-to-left text (3.6) and for whole runs placed after one another;
     it never moves single characters of one run between the characters of
     another. Two characters whose positions along the line differ by less
     than 0.01 em (the same tolerance 3.5 uses for coinciding glyphs) are
     at the same position, and keep their drawing order. Zero-advance
     combining marks (Thai, Devanagari, Hebrew points) are the common case:
     rounding in the text matrix can put the next letter a hair to the left
     of the mark before it.
  3. **Overlapping runs.** A run whose extent along the line overlaps
     characters already on that line by more than the 3.5 rule 3 allowance
     is not merged into it character by character: it starts a new line in
     the same block (or, at the implementer's choice, is appended whole
     after the line's characters).

  User-visible effect: form labels such as a warning sign's "!" drawn over
  the word "CAUTION", an overlay form XObject's word drawn over a page's
  word, OCR words whose boxes overlap, and Thai vowel marks read as drawn,
  where they read as interleaved letters. Tests
  (`cleanroom_text_corpus.rs`):
  `c3_a_run_drawn_back_over_a_line_is_not_interleaved_with_it`,
  `c3_a_small_label_drawn_across_a_large_mark_reads_whole`,
  `c3_characters_at_one_position_keep_their_drawing_order`, and the control
  `c3_runs_that_follow_each_other_read_in_order`.

- **C4. Pixel size from the box numbers as written (5.2, N11).** The
  product v × scale of 5.2 is computed in double precision from the box
  numbers as parsed. Rounding the frame size to single precision first is
  not allowed: it moves a written excess of 0.001 point to about 0.00101
  and across the tolerance. ε is pinned to [0.00101, 0.00104): a side
  written as N + 0.001 (any N up to at least 1000) is N pixels at scale 1,
  N + 0.00105 and N + 0.0011 are N + 1. This narrows N11's range
  [0.0009, 0.0011), whose rows all still hold. Experiment E9 below.
  Test (`cleanroom_text_corpus.rs`):
  `c4_pixel_size_reads_the_box_numbers_as_written`.

  **E9 (black box, MuPDF 1.28.1 through the npm `mupdf` package).** Input:
  one page with no content and the page entries listed; `page.toPixmap`
  at scales 1 and 2. Script and raw output in `reviewer archive
  corpus-addenda-2026-10-18/` (`exp_c4.mjs`, self-authored, builds the
  PDFs; `exp_c4.txt`), run as `slot node exp_c4.mjs`. Output (width ×
  height):

  | Page entries | scale 1 | scale 2 |
  |---|---|---|
  | `/MediaBox [0 0 100 500.001]` | 100 × 500 | 200 × 1001 |
  | `/MediaBox [0 0 100 315.001]` | 100 × 315 | 200 × 631 |
  | `/MediaBox [0 0 100 1000.001]` | 100 × 1000 | 200 × 2001 |
  | `/MediaBox [0 0 100 100.001]` | 100 × 100 | 200 × 201 |
  | `/MediaBox [0 0 100 600] /CropBox [0 100.999 100 600]` | 100 × 499 | 200 × 999 |
  | `/MediaBox [0 0 100 600] /CropBox [0 200.999 100 516]` | 100 × 315 | 200 × 631 |
  | `/MediaBox [0 0 100 500.0011]` | 100 × 501 | 200 × 1001 |
  | `/MediaBox [0 0 100 500.00105]` | 100 × 501 | 200 × 1001 |

  In double precision the excess of `100.001` is 0.0010000000000048, which
  is why ε must be strictly above 0.001.

- **Corpus findings that are expected (no rule change).** For the owner's
  review of the 8.3 gate:
  - Same text drawn twice at the same place (3.5, must) and copies offset
    by up to 0.05 em (owner decision T7) now read once. Pages that drew
    every label twice, or a dot leader three to five times as a shadow,
    lose hundreds of characters against the reference viewer, which keeps
    every copy. These pages count against gate items 1 and 2 although they
    are the intended behaviour. Recommendation: the runner lists pages
    whose only difference is copies that 3.5 merges, and counts them as
    equal, as it already does for right-to-left order.
  - Dot leaders and form values that join their label's line (owner
    decision T5) make many form pages "equal ignoring spaces".

## 12 (continued). Second corpus-run addenda (2026-10-18)

Found by the corpus run of the tree with C1 to C4 implemented, against the
previous run: eleven files whose page text changed line order.

- **C5. Runs drawn back to the front of a line (3.5 rule 3, C3.3).** 3.5
  rule 3 measures a backward jump from the glyph the line ended with. Read
  alone, it makes every run that a page draws after a line, but places
  before it, a line of its own that reads after the line. Made precise:
  1. **Front join.** A left-to-right run drawn after a line, lying wholly
     before the line's first character along the line, on a baseline that
     joins by rule 2, joins the line at its front: it ends before the
     line's first character with a gap below the line-break gap of 3.4, or
     overlaps it by no more than a small allowance of the implementer's
     choice, at most the no-space band of 3.4 (0.10 em; ems of the smaller
     size, C3.1). Its characters read before the line's, as one
     piece, with a space inferred from the gap by 3.4. Basis: writers draw
     list bullets, line numbers, footnote numbers and field labels after
     the text they stand in front of; the reader reads them first. The
     reference keeps drawing order (E10), so these pages differ from it in
     line order; this spec chooses the order of the page. Owner to confirm
     (the alternative, no front join, reads every bullet after its item).
  2. **The whole run decides.** Whether a run joins a line at its front is
     decided for the run as a whole, not by its first glyph. A run that
     starts before the line, but whose extent along the line then reaches
     into the line's characters by more than the C3.3 allowance, does not
     join at the front: it is an overlapping run (C3.3), starts a new line
     in the same block, and so reads after the line in drawing order. A
     first glyph that is whitespace (a caption that starts with a space)
     changes nothing. The line's own text stays together in every case.
  3. Right-to-left text drawn with the pen moving left (3.6) is not
     affected: each of its glyphs keeps the allowance of 3.5 rule 3 (about
     one em) against the glyph drawn before it.

  User-visible effect: bullets and line numbers read before their line
  (rule 1); formula pieces placed by a later operator back across a line,
  and a small caption row starting just left of a large title on a nearby
  baseline, no longer read in front of that line with its first word glued
  to the caption's last (rule 2). Tests (`cleanroom_text_corpus.rs`):
  `c5_a_run_drawn_back_before_a_line_and_across_it_reads_after_it`
  (rule 2), and the control `c5_a_mark_drawn_after_its_line_just_before_it_reads_first`
  (rule 1).

  Corpus: rule 2, pdf.js `issue3205r.pdf`, govdocs1 `000380.pdf`,
  `000956.pdf` (equal to the reference before C3, different after). Rule
  1, line order only, against a reference that keeps drawing order:
  govdocs1 `000018.pdf`, `000168.pdf`, `000345.pdf`, `000578.pdf`,
  `000834.pdf`, pdf.js `freeculture.pdf`, `issue15910.pdf`. For the 8.3
  gate these count as differences until the owner confirms rule 1; the
  runner can list pages whose only difference is a run moved to the front
  of the line it stands before.

  **E10 (black box, MuPDF 1.28.1 through the npm `mupdf` package).** Input:
  the PDFs the two C5 tests build (Helvetica, WinAnsi, one 300 × 300
  page), saved by the reviewer from the tests. Command:
  `slot node mu-text.mjs test-pdfs/c5_*.pdf` (self-authored; prints each
  structured-text line with its origin, `toStructuredText("preserve-whitespace")`).
  Script, PDFs and raw output in `reviewer archive
  corpus-addenda-2026-10-18-r2/` (`mu-text.mjs`, `test-pdfs/`,
  `exp-c5.txt`). Observed lines, in output order:

  | Content | Reference lines |
  |---|---|
  | `100 200 Td (Item one) Tj -14 0 Td (\225) Tj` (12 pt) | `Item one` at x 100; `•` at x 86 |
  | `(Be it enacted)` at x 100, 10 pt; `(12)` at x 84, 8 pt, same baseline | `Be it enacted`; `12` at x 84 |
  | `(alpha)` at 100, `(beta)` at 160, then `-78 0 Td [(X) -1500 (Y) -1500 (Z)] TJ` (12 pt) | `alpha`; `beta`; `X` at 82; `Y` at 108; `Z` at 134 |
  | `(Title)` 40 pt at (100, 200); `( Month)` 10 pt at (96, 190) | `Title`; ` Month` |
  | `(alpha beta)` at 100; `(XY gamma)` at 90, same baseline, 12 pt | `alpha beta`; `XY gamma` |

- **C5a. C5 rules 1 and 2 made one test (review of the C5 implementation).**
  Rule 1's allowance (at most 0.10 em) is the only allowance for a front
  join; the C3.3 allowance of rule 2 does not widen it. Precisely:
  1. A run is a candidate for a front join when its first glyph starts
     before the leftmost character of the line, along the line, however
     short the line is (a line shorter than an em is no exception).
  2. The candidate joins at the front only if every glyph of the run,
     including any glyph that would not itself join the line, ends no more
     than the rule 1 allowance into the line's characters. Otherwise the
     whole run leaves the line: it starts a new line in the same block and
     reads after the line in drawing order (rule 2). The line's own text
     stays together.
  3. Rule 3 (right-to-left text) is unchanged.
  Test: `c5a_a_front_run_that_reaches_into_the_line_reads_after_it`
  (cases: a later glyph jumping far into the line; a line shorter than an
  em; a second glyph overlapping by about 0.6 em).

- **C6. Copies of a glyph of several characters (3.5, T7).** When a glyph
  whose text is several characters is drawn again as a copy, each of its
  characters matches its own counterpart in the first drawing, so a
  property the copy carries (visibility, 3.8) reaches every character of
  the glyph, and the text reads the glyph's characters once, in order.
  Test (holds for its case today; keep it green):
  `c6_a_visible_copy_of_a_ligature_glyph_keeps_all_its_characters`.

## 12 (continued). Third corpus-run addenda (2026-10-18)

Found by the T14 corpus run of the tree with C5a and C6 implemented. Every
page the runner listed as lost text was checked by position: for each
character the reference viewer extracts and this spec's text lacks, the
reviewer looked for another drawing of the same character in the reference
within 0.05 em of its origin (3.5, T7). Probe scripts and counts are in
`reviewer archive corpus-addenda-2026-10-18-r3/` (`probe/`,
`probe-lost-positions.txt`).

- **C7. Characters that joined a line at its front belong to it (3.5,
  C3.3, C5, C5a).** Made precise:
  1. Once a run has joined a line at its front (C5 rule 1), the line's
     leftmost character for C5a rule 1 is the leftmost character of that
     run. A later run is a front candidate only if it starts before it.
  2. A later run is placed in a line only at its front (C5, C5a) or after
     its last character (3.5). It is never placed between characters
     already on the line, and in particular never between a run that
     joined at the front and the characters it stands before.
  3. A later run that starts among or just after the front-joined
     characters, and whose extent along the line overlaps the line's other
     characters by more than the C3.3 allowance, is an overlapping run
     (C3.3): it starts a new line in the same block (or, at the
     implementer's choice, is appended whole after the line's characters),
     and reads after the line in drawing order.

  User-visible effect: an equation set one run per font (the numbers, then
  a variable drawn just before them, then the operators) reads the numbers
  together instead of `k = ±0.23` with the operators pushed between the
  variable and the number they overlap. Tests (`cleanroom_text_corpus.rs`):
  `c7_a_run_after_a_front_joined_mark_does_not_enter_the_line` (two
  cases), and the control
  `c7_a_second_run_wholly_before_the_line_joins_its_front` (holds today).
  Corpus: pdf.js `issue3205r.pdf` page 1 (the reference keeps drawing
  order, 12 lines; this tree reads 4 lines with the operator run inside
  the second line).

  **E12 (black box, MuPDF 1.28.1 through the npm `mupdf` package).** Input:
  the PDFs the C7 tests build (Helvetica, WinAnsi, one 300 × 300 page).
  Command: `slot node mu-text.mjs test-pdfs/c7_*.pdf`. Script, PDFs and
  raw output in `reviewer archive corpus-addenda-2026-10-18-r3/`
  (`exp-c7.txt`). Observed (reference lines in output order; this tree
  before the fix):

  | Content | Reference | This tree (c02149f) |
  |---|---|---|
  | `[(0.23) -844 (0.03;)]` at x 30; `(k)` at 22; `[(=) -2092 (+)]` at 27 (12 pt) | `0.23`; `0.03;`; `k=`; `+` | `k= +0.230.03;` |
  | `(alpha beta)` at 100; `(1)` at 90; `(2 WXYZ)` at 97 | `alpha beta`; `12 WXYZ` | `12 WXYZalpha beta` |
  | `(alpha)` at 100; `(1)` at 80; `(.)` at 87 | `alpha`; `1.` | `1. alpha` |

### Third corpus run: findings that are expected (no rule change)

1. **Copies the runner cannot see.** Every listed page of govdocs1
   `000762.pdf` (7 pages, 1,028 characters), `000618.pdf`, `000595.pdf`,
   `000583.pdf`, `000751.pdf`, `000887.pdf`, `000352.pdf`, `000149.pdf`,
   `000904.pdf`, `000632.pdf`, `000744.pdf`, pdf.js `issue1350.pdf`,
   `visibility_expressions.pdf`, qpdf `job-json-underlay-overlay.pdf`
   and `overlay-copy-annotations-p1.pdf`: every missing character has a
   second drawing of the same character within 0.05 em of its origin in
   the reference (3.5, T7). The runner's copy filter compares adjacent
   reference lines only, and these pages draw the copies far apart in
   drawing order (a figure's labels drawn twice, an overlay drawn over the
   page). govdocs1 `000208.pdf` (16 characters on each of 38 pages) and
   `000269.pdf`: a running header drawn twice at one origin, the first
   time with an `fi` ligature glyph and the second time with a plain `f`;
   each character of the second drawing coincides with one of the first
   (C6), so the header reads once, with its `fi`. Recommendation for the
   runner: classify a shortfall character as a copy when the reference
   has another drawing of it within 0.05 em, whatever the line order.
2. **Text in a pattern cell** (3.1, 1.4): pdf.js `issue15716.pdf` draws
   its four ZapfDingbats glyphs only inside a tiling pattern used as a
   fill colour. The page text is empty by 3.1.
3. **Visibility by ink** (3.8): pdf.js `bug946506.pdf` loses three glyphs
   at the bottom edge. They are drawn through a sheared text matrix; their
   ink lies beyond the page's bottom edge (reviewer's estimate from the
   glyph heights), while their
   em boxes, taken from the descriptors' ascent, cross it. 3.8 lets the glyph box be the ink bounds; they stay in
   `unclipped_blocks` and are found by search.
4. **Outside the rewritten code.**
   - Page tree: qpdf `page_api_2.pdf` lists one page object twice in
     `/Kids`; qpdf `xref-compressed-in-compressed.pdf` likewise lists page
     7 again near the end. The app's page walk and the vendored
     interpreter's page list show such a page once; the reference shows it
     at each listing. Per-page comparison then pairs shifted pages (the
     reference's extra page reads "lost"). No text is lost.
   - Content-stream lexing in the vendored interpreter
     (`hayro-syntax`, the content operator iterator): a malformed token
     ends the rest of the content stream, where the reference skips the
     token and goes on (E13). veraPDF `6-1-5-t02-fail-a`,
     `6-1-6-t01-fail-b`, `6-1-6-t02-fail-a` (a hex string `<484!>` with a
     non-hex byte: the reference reads it with the byte as 0, as spec 01
     4.1 rule 2 does for objects) and pdf.js `poppler-90-0-fuzzed.pdf`
     page 14 (fuzzed bytes such as `0#` and controls early in the stream;
     the page draws nothing at all, 888 characters). Its page 10 (2
     characters) is the vendored interpreter's ASCIIHex decoder
     (`hayro-syntax` filters): a bad digit partway through a content
     stream drops the whole stream, where the reference interprets the
     part decoded before it. The reference opens the file without repair;
     neither is a
     spec 03 or 04 rule. Fixing them is a patch to the vendored
     interpreter (like T1).
   - pdf.js `issue12823.pdf`: a Type0 font whose `/DescendantFonts` is
     `[null]` is not drawn by the vendored interpreter, so its two glyphs
     give no text. The reference draws them with a fallback and reads
     U+FFFD and `‡`; the font's ToUnicode maps those codes to U+26F3 and
     U+2B50, which neither engine shows.
5. **More U+FFFD than the baseline** (spec 05 8.3 gate): pdf.js
   `complex_ttf_font.pdf`, `ThuluthFeatures.pdf`, `issue5874.pdf` and
   probably `TAMReview.pdf` (+2, not checked glyph by glyph). The extra
   characters are emitted by the layout, and they are right by 3.5 and T7:
   each is a different glyph with no reading, 0.05 em to 0.1 em from a
   neighbouring glyph with no reading. The reference treats two U+FFFD
   that close as one overprinted copy (E11); this spec merges copies only
   up to 0.05 em, and these are not copies (different glyphs; in
   `complex_ttf_font.pdf` the second is a blank glyph 0.051 em wide used as
   a word space). Owner decision needed: name these files as exceptions
   to the gate, or adopt a rule for blank glyphs without a reading (read
   them as U+0020 when they advance, nothing when they do not), which
   would remove most of `complex_ttf_font.pdf`'s extra characters and
   give its words their spaces.

   **E11 (black box, MuPDF 1.28.1 npm).** Input: a Type0 font, Identity-H,
   CIDFontType2 with an embedded TrueType program that has no `cmap` and
   no `post` table, 20 pt; glyphs 1 and 3 outlined. `<00010003>` shown
   with `/W [1 [N]]`, so glyph 3 is drawn N/1000 em after glyph 1; and
   `<000100020001>` (glyph 2 blank), `<000100030001>` with glyph 3 blank
   or outlined, at the default width. Command: `slot node mu-text.mjs
   test-pdfs/w*.pdf test-pdfs/blank*.pdf test-pdfs/outlined3.pdf`; raw
   output `exp-fffd.txt`. Observed:

   | Input | Reference | This tree (c02149f) |
   |---|---|---|
   | N = 30, 50 | one U+FFFD | one U+FFFD |
   | N = 52, 80 | one U+FFFD | two U+FFFD |
   | N = 120 | two U+FFFD | two U+FFFD |
   | blank glyph between two outlined glyphs, 0.5 em apart | three U+FFFD | three U+FFFD |

   **E13 (black box, MuPDF 1.28.1 npm).** Input: Helvetica pages with
   `BT … (A) Tj ET`, then a malformed token, then `BT … (B) Tj ET`.
   Command: `slot node mu-text.mjs test-pdfs/garb_*.pdf …`; raw output
   `exp-content-lex.txt`. The reference reads `A` and `B` for each of
   `q 10 0 0# 1 0 0 m`, `q 10 0 0` followed by bytes 13 02 03 and `m`,
   a high-byte keyword, and a stray `(…)` string; this tree reads only `A`
   for the first two.
6. **Pages with empty text** (8.3 item 4): six against the baseline's
   five. The new one is pdf.js `issue11922_reduced.pdf`, ten spaces by
   spec 05 U4 where the reference shows ten U+FFFD. Recommendation: the
   gate counts a page whose reference text is only U+FFFD, controls and
   noncharacters (spec 05 U5) as explained, in both runs. That gives four
   and four: TWG `A025-pdfa2-pass-a.pdf` (one reference U+FFFD) drops out
   of both; the three veraPDF hex-string files (item 4) and
   `issue15716.pdf` (item 2) remain in both.

- **C7 note (implementation review, 2026-10-18).** In C5a and C7, "the
  line's characters" for a front join means the line's own characters,
  not characters already joined at its front: a second mark before a line
  that already has a front-joined mark is measured against the line's own
  leftmost character (so "1", then "." drawn between "1" and the line,
  reads "1.alpha", as the C7 control expects).
  The C5 rule 1 gap (below the line-break gap of 3.4) is still measured to
  the line's leftmost character, front-joined pieces included, so a front
  group built from several runs keeps joining ("7", then "•", then "text"
  reads "7 • text"; a line drawn word by word from right to left stays one
  line). Test: `c7_a_front_group_of_several_runs_keeps_joining`.
