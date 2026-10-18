# Spec 06: annotation appearance streams

Workstream 6 of `docs/mit-clean-room-plan.md`. Status: draft for review,
2026-10-18. Rules: `docs/clean-room/SPEC-RULES.md`.

This spec replaces:

- `engine/src/app/annots/appearance.rs` (all of it);
- in `engine/src/app/annots/view.rs`: `Synthesized`, `synthesized_box`,
  `synthesize`, the private rect-box rule and the synthesis bounds. The rest
  of `view.rs` (`Fix`, `Budget`, `fix`, `normal_appearance`, `is_stream`:
  the placement of existing appearances for NoZoom, NoRotate and hayro's
  offset) is not part of this workstream and stays;
- `engine/src/app/annots/fonts.rs` (all of it, the width tables
  regenerated from public font metrics, section 5).

It depends on spec 07 for variable-text layout and non-WinAnsi fallback
fonts (`app::forms::scripts`, `app::forms::text`). FreeText layout uses that
shared machinery. Workstreams 6 and 7 land together or 7 first.

Public sources: ISO 32000-2:2020 §8.4 (graphics state), §8.5 (path
construction and painting), §9.6.2 (standard Type 1 fonts), §12.5.2
(annotation dictionaries), §12.5.3 (flags), §12.5.4 (border styles), §12.5.5
(appearance streams), §12.5.6.x (each subtype, cited below), §12.7.4.3
(variable text, for FreeText), Annex D (WinAnsiEncoding); the Adobe Glyph
List; the URW++ base-35 AFM files (SIL OFL 1.1) or Adobe's Core 14 AFM
files for the standard-font widths.

The implementer may read this spec, those sources, the acceptance tests of
section 9, and the surviving engine code. The implementer may not read the
replaced files, their git history, `engine/design/app-annots.md`,
`engine/design/app-accept-annots.md`, the recorded MuPDF outputs under
`engine/reference/app-annots-20261015`, or MuPDF source.

---

## 0. Decisions taken in this spec (owner may override)

| # | Decision | Why |
|---|---|---|
| A1 | All icon artwork (note icons, caret, stamp design, line endings, squiggle shape) is new. Only its function is specified. | The old artwork is MuPDF's expressive content. |
| A2 | An icon stamp's label is made from its `/Name`: the name split into words at lower-to-upper case changes, in capitals ("NotForPublicRelease" → "NOT FOR PUBLIC RELEASE"). Non-standard names (the app's `Void`) follow the same rule. | No label table. Every name gets a consistent, readable label. |
| A3 | A Caret without an appearance is drawn to fill its /Rect less /RD. The /Rect is not replaced by a fixed-size box. | Honours the author's rectangle. Acrobat draws carets this way. |
| A4 | A border width of 0 draws no border (ISO 32000-2 §12.5.4, Table 168 /W). A negative or non-numeric width is the default 1. | The ISO meaning. A `0 w` stroke would draw a hairline. |
| A5 | A missing or unusable /C draws stroke-only kinds in black, so the annotation stays visible. A Highlight without /C uses a light highlighter colour. | An invisible annotation helps no one. A black multiply highlight hides text. |
| A6 | Text markups, Ink, Polygon and PolyLine with no usable geometry get no appearance, and their /Rect is not changed. | MuPDF writes an inverted /Rect of ±2^31 for these (section 7). |
| A7 | In FreeText, a word wider than the line is broken between characters (spec 07, §4.4). | It keeps long words such as URLs inside the box. |
| A8 | Each of the 12 non-symbolic standard faces measures with its own AFM widths (bold and oblique included). | It is correct for every face, not only the four regular ones. |

---

## 1. Scope

### 1.1 What the component does

The component builds Form XObject appearance streams (ISO §8.10, §12.5.5)
for the annotation kinds the app creates or edits, and for annotations
without an appearance that the view draws. For each kind it also gives the
/Rect and, where relevant, the /RD that the appearance needs. It supplies
the standard-font metrics and WinAnsi encoding that annotation text uses.

### 1.2 Callers (surviving code)

| Caller | Uses | Needs |
|---|---|---|
| `app::ops::annots::add` (`annot-add`) | `note`, `highlight`, `text_line`, `border_allowance`, `grow`, `rectangle`, `line`, `ink`, `ink_allowance`, `image_stamp`, `icon_fit`, `stamp`, `Style`, `Mark`, `number`, `numbers`, `IDENTITY` | The appearance and the /Rect, /RD it writes. Rules: Square/Circle /Rect = `grow(inner, border_allowance(w))` and /RD = `border_allowance(w)` on each side, so /Rect less /RD is exactly the rectangle the user drew. Ink /RD = `ink_allowance(w)` on each side. Icon stamp /Rect = `icon_fit(requested)` (page space, then turned to user space). |
| `app::ops::annots::free_text_form` | `FreeText`, `free_text`, `line_height` | The FreeText appearance from /DA, /Q, /Rotate, /BS, /C, /CA |
| `app::ops::annots` `free_text_fit` (engine.ts `freeTextFit`, kept) | `line_height(12.0)`, `Base14::Helvetica.advance`, `fonts::win_ansi` | A fit box is `n × line_height(12)` tall and as wide as the widest line's Helvetica 12 advance plus 0.5. The appearance of that box must show all `n` lines (section 3.3.5). |
| `app::ops::annots` `set`, `move_annotation`, `regenerate`, `write_ink` | `view::synthesize`, `ink`, `ink_allowance`, `bounds`, `line_height` | `annot-set` writes the synthesized form with its /Rect and /RD for kinds without /AP. A drawn signature (Ink) is rebuilt on resize. |
| `app::ops::annots::stamp_images` | the image stamp's resource name `I` | Finds the image an existing image stamp shows (section 3.10) |
| `app::annots::page_rect` (listing, `annots` op) | `view::synthesized_box` | The box reported for an annotation without an appearance |
| `app::annots::view::fix` (kept) | `view::synthesize`, `Form::dictionary`, `numbers`, `IDENTITY` | View-only appearances for annotations without one |
| `app::annots::put_appearance` (kept) | `Form`, `Form::dictionary` | Stores a form. It also turns direct Type0 descendant fonts and descriptors in /Resources /Font into objects (section 3.3.4). |
| `app::annots::Color::object`, `PageSpace` code, redaction (`redact::{region, content, apply}`) | `numbers`, `grow`, `bounds`, `IDENTITY` | Plain geometry helpers |
| `app::redact::marks` (`redact-add`, `redact-set`) | `redact`, `numbers` | Redact mark appearance and /Rect |
| `app::ops::annots` `check_free_text_encoding`, `fit_advance` | `fonts::win_ansi`, `Base14::advance` | WinAnsi membership and Helvetica advances |

### 1.3 Surviving APIs the component builds on

- `app::annots::{Color, Point, Quad, PageSpace, concat, transform_rect}`.
  `Quad` is `[ul, ur, ll, lr]`: upper left, upper right, lower left, lower
  right, in user space. That is Acrobat's /QuadPoints order. The edges
  `ul→ur` and `ll→lr` run along the text, and `ll→ul` runs up from the
  baseline side. `Color` is `Gray(f64) | Rgb([f64;3]) | Cmyk([f64;4])`.
  `Color::read` returns `None` for anything but 1, 3 or 4 numbers.
  `Color::luminance()` gives a 0..1 luma.
- `app::annots::{border_width, border_dash, opacity, rect_differences,
  quads, points, ink_list, line_points, text, rect, inset, design_rect}`:
  readers of the annotation dictionary. `border_dash` gives the /BS /D
  array for style D (default `[3]`, ISO Table 168) and empty for a solid
  border.
- `app::ops::annots::{default_appearance, DefaultAppearance, NOTE_SIZE}`:
  the /DA parser (font name, size, colour; Helv 12 black by default) and
  the note size of the app (20).
- `serialize::compact_real` (spec 01 §3.7.5). It prints the shortest
  round-trip decimal, up to 17 significant digits. **Round every computed
  coordinate before formatting** (section 3.0.1).
- `app::forms::scripts` and `app::forms::text` (spec 07): `Cjk`,
  `han_font`, the variable-text line layout and the fallback fonts.
- `builtin_metrics::BuiltinFace` (URW advances by glyph name for Helvetica,
  Helvetica-Bold, Helvetica-BoldOblique and Times-Roman). It is OFL data
  already in the engine.

### 1.4 Non-goals

- Icons for FileAttachment and Sound, Popup windows, Link, Widget (spec 07),
  3D, Watermark, Polygon or PolyLine clouds (/BE), Line leader lines and
  captions (/LL, /LLE, /Cap), FreeText callouts (/CL, /LE) and rich text
  (/RC, /DS). Rich text and /DS are not applied. The /DA governs.
- Signature appearances drawn by `app::ops::sign` (its own code).
- Saving: view-synthesized appearances are never saved (`view::fix` puts
  them in the view's private update only).

---

## 2. Interface

### 2.1 `crate::app::annots::appearance`

Every item below is `pub` with exactly this signature unless marked
**(free)**, which means the implementer may change it because only this
module uses it. Private items are the implementer's choice.

```rust
pub const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// A Form XObject appearance.
#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub bbox: [f64; 4],
    pub matrix: [f64; 6],
    /// Empty when the content needs none.
    pub resources: Dictionary,
    pub content: Vec<u8>,
}
impl Form {
    /// The stream dictionary: /Type /XObject, /Subtype /Form, /BBox,
    /// /Matrix, and /Resources when not empty. No /Length or /Filter
    /// (the writer adds those).
    pub fn dictionary(&self) -> Dictionary;
}

/// How a shape is painted: /C (stroke), /IC (fill), border width, /CA,
/// dash array (empty = solid).
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub stroke: Option<Color>,
    pub fill: Option<Color>,
    pub width: f64,
    pub opacity: f64,
    pub dash: Vec<f64>,
}
impl Default for Style; // stroke None, fill None, width 1 (ISO default), opacity 1, dash empty

/// A PDF number: Integer when `value` is integral and |value| < 1e15,
/// else Real; non-finite values become Integer 0.
pub fn number(value: f64) -> Object;
/// An array of `number`s.
pub fn numbers(values: &[f64]) -> Object;
/// Axis-aligned bounds [x0 y0 x1 y1] of the points; None for none.
pub fn bounds(points: impl IntoIterator<Item = Point>) -> Option<[f64; 4]>;
/// `rect` grown by `by` on every side (shrunk for negative `by`).
pub fn grow(rect: [f64; 4], by: f64) -> [f64; 4];

/// A Text annotation's icon (section 3.1). `icon` is the /Name bytes.
pub fn note(icon: &[u8], color: Option<&Color>, alpha: f64) -> Form;

/// Highlight over `quads`; returns the form and the /Rect (section 3.2).
pub fn highlight(quads: &[Quad], color: Option<&Color>, alpha: f64) -> (Form, [f64; 4]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark { Underline, StrikeOut, Squiggly }
/// Underline, StrikeOut or Squiggly; returns the form and the /Rect.
pub fn text_line(mark: Mark, quads: &[Quad], color: Option<&Color>, alpha: f64)
    -> (Form, [f64; 4]);

/// The /RD each side of a Square, Circle (or Polygon) the app creates
/// from a rectangle with border width `width` (section 3.4).
pub fn border_allowance(width: f64) -> f64;
/// Square (`ellipse` false) or Circle whose border runs along `inner`,
/// in a form whose /BBox is `bbox`.
pub fn rectangle(inner: [f64; 4], bbox: [f64; 4], ellipse: bool, style: &Style) -> Form;

/// Line from `a` to `b`; `endings` are the two /LE names (b"" or b"None"
/// for none). Returns the form and the /Rect.
pub fn line(a: Point, b: Point, endings: [&[u8]; 2], style: &Style) -> (Form, [f64; 4]);

/// Polygon (`closed`) or PolyLine; returns the form and the /Rect.
pub fn poly(vertices: &[Point], closed: bool, style: &Style) -> (Form, [f64; 4]);

/// The /RD each side of an Ink with border width `width` (section 3.6).
pub fn ink_allowance(width: f64) -> f64;
/// Ink strokes; returns the form and the /Rect.
pub fn ink(strokes: &[Vec<Point>], style: &Style) -> (Form, [f64; 4]);

/// A Caret mark (section 3.8), in a form whose /BBox the implementer
/// chooses (the view maps it onto /Rect less /RD).
pub fn caret(color: Option<&Color>, alpha: f64) -> Form;

/// An icon stamp's box fitted inside `rect` about its centre (section 3.9).
pub fn icon_fit(rect: [f64; 4]) -> [f64; 4];
/// An icon stamp for /Name `name`; `matrix` is the form's /Matrix (the
/// caller passes a rotation that keeps it upright on a rotated page).
pub fn stamp(name: &[u8], color: Option<&Color>, alpha: f64, matrix: [f64; 6]) -> Form;

/// An image stamp or signature (section 3.10).
pub fn image_stamp(image: ObjectId, placement: [f64; 6], matrix: [f64; 6]) -> Form;

/// A Redact mark (section 3.11): returns the form and the /Rect.
pub fn redact(rect: [f64; 4], quads: &[Quad], color: Option<&Color>) -> (Form, [f64; 4]);

/// What a FreeText draws.
#[derive(Clone, Debug, PartialEq)]
pub struct FreeText<'a> {
    pub text: &'a str,
    /// The /DA font name: the resource name of the main font.
    pub font_name: &'a [u8],
    pub font: Base14,
    /// The CID font for Han characters (spec 07 `scripts::han_font`).
    pub han: Cjk,
    pub size: f64,
    /// Text and border colour (/DA colour). None: black.
    pub color: Option<Color>,
    /// Border width (/BS /W or /Border); 0 or less: no border.
    pub border: f64,
    /// Dash array (/BS /S /D); empty: solid.
    pub dash: Vec<f64>,
    /// Background (/C); None: no fill.
    pub background: Option<Color>,
    /// /Q: 0 left, 1 centred, 2 right; anything else is 0.
    pub quadding: i64,
    /// /Rotate, a multiple of 90 in 0..360.
    pub rotate: i32,
    pub opacity: f64,
}
/// Characters of a FreeText laid out at most (app limit).
pub const MAX_FREE_TEXT_CHARS: usize = 65_536;
/// Distance between baselines of FreeText lines at font size `size`.
pub fn line_height(size: f64) -> f64;
/// The FreeText appearance: text laid out in `rect` (/Rect less /RD),
/// form /BBox `bbox` (the /Rect).
pub fn free_text(bbox: [f64; 4], rect: [f64; 4], box_: &FreeText<'_>) -> Form;
```

`STAMP_BOX` and `free_text_lines` are no longer public. Nothing outside
the module used them.

### 2.2 `crate::app::annots::view` (the replaced part)

```rust
/// An appearance made for an annotation from its dictionary, with the
/// /Rect and (when it sets one) /RD that go with it.
#[derive(Clone, Debug, PartialEq)]
pub struct Synthesized { pub form: Form, pub rect: [f64; 4], pub rd: Option<[f64; 4]> }

/// For an annotation without a normal appearance whose kind `synthesize`
/// sizes from its /Rect: that box (/Rect `outer` → (rect, rd)). None when
/// it has an appearance, or its box is its geometry, or it is not drawn.
pub fn synthesized_box(doc: &Document<'_>, annot: &Dictionary, outer: [f64; 4])
    -> Option<([f64; 4], Option<[f64; 4]>)>;

/// The appearance for `annot` from its entries (section 4); None when the
/// kind is not drawn or its geometry is unusable.
pub fn synthesize(doc: &Document<'_>, annot: &Dictionary) -> Option<Synthesized>;
```

`synthesized_box` must call the same private rule that `synthesize` uses
for its /Rect, so the listing and the drawing agree.

### 2.3 `crate::app::annots::fonts`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base14 {
    Helvetica, HelveticaBold, HelveticaOblique, HelveticaBoldOblique,
    TimesRoman, TimesBold, TimesItalic, TimesBoldItalic,
    Courier, CourierBold, CourierOblique, CourierBoldOblique,
}
impl Base14 {
    /// The face a /DA font resource name stands for (section 5.1);
    /// anything else is Helvetica.
    pub fn from_resource_name(name: &[u8]) -> Self;
    /// The standard /BaseFont name (ISO §9.6.2.2).
    pub fn base_font(self) -> &'static [u8];
    /// Advance of WinAnsi code `byte` in 1/1000 em; 0 for codes the face
    /// does not draw (control codes, undefined WinAnsi codes).
    pub fn advance(self, byte: u8) -> f64;
    /// Width of WinAnsi `bytes` at `size`: sum of advances × size / 1000.
    pub fn width(self, bytes: &[u8], size: f64) -> f64;
    /// /Type /Font /Subtype /Type1 /BaseFont <base_font> /Encoding /WinAnsiEncoding.
    pub fn dictionary(self) -> Dictionary;
}
/// The WinAnsiEncoding code of `c` (Annex D), if any.
pub fn win_ansi(c: char) -> Option<u8>;
/// `text` as WinAnsi bytes: a C0 control character (< U+0020) is kept as
/// its byte; any other character without a code becomes `?`.
pub fn encode(text: &str) -> Vec<u8>;
```

---

## 3. Required behaviour

### 3.0 Common to every appearance

#### 3.0.1 Numbers and operators

- Content streams use the operators of ISO §8.4–8.5 and §9.4 only. Numbers
  are written with `compact_real` after rounding to at most 4 decimal places
  (1/10,000 pt). A non-finite computed value is written as 0, and never as
  `NaN` or `inf`.
- /BBox and /Matrix entries are written with `number`.
- Content is ASCII except bytes inside string operands.

#### 3.0.2 Colours

- `Color::Gray` → `g`/`G`, `Rgb` → `rg`/`RG`, `Cmyk` → `k`/`K` (ISO §8.6.8).
- Defaults when the colour argument is `None` (decision A5):
  - stroke-only kinds (Underline, StrikeOut, Squiggly, Line, PolyLine,
    Ink, Caret): black;
  - Square, Circle, Polygon: stroke with `style.stroke` and fill with
    `style.fill`. When both are `None`, stroke black, so the shape stays
    visible;
  - Highlight: a light colour of the implementer's choice that keeps black
    text readable under the multiply blend (yellow is the usual choice);
  - Text (note) background: a light neutral colour;
  - Stamp: a strong colour that reads on white (implementer's choice);
  - Redact outline: a conspicuous colour (implementer's choice; red is
    usual);
  - FreeText text and border: black; background: none.

#### 3.0.3 Opacity and blending

- `alpha`/`opacity` < 1 sets /CA and /ca in an ExtGState resource that the
  content selects with `gs` before painting (ISO §8.4.5, §11.6.4.4).
  Opacity ≥ 1, or not finite, needs no ExtGState.
- A Highlight always paints with blend mode Multiply (ISO §11.3.5), so the
  text beneath stays readable. This needs an ExtGState with /BM /Multiply.
- ExtGState resource names are the implementer's choice.

#### 3.0.4 Borders

- `style.width` ≤ 0 or not finite: no border is stroked (decision A4).
- `style.dash` non-empty: `[d…] 0 d` before stroking (ISO §8.4.3.6).
  Callers already replace invalid dash arrays with `[3]`.
- Line joins and caps are the implementer's choice, with one rule: every
  painted pixel must lie inside the /Rect the function returns (the
  acceptance tests render and check).

#### 3.0.5 Form dictionary

`Form::dictionary` writes exactly /Type /XObject, /Subtype /Form, /BBox,
/Matrix, and /Resources when not empty. Key order is the implementer's
choice. /Matrix is always written, IDENTITY included.

### 3.1 Text annotation icon (`note`), ISO §12.5.6.4

- **Need:** a recognisable icon for each standard /Name of §12.5.6.4
  (Comment, Key, Note, Help, NewParagraph, Paragraph, Insert). Each icon
  must be visually distinct from the others. Any other name, and the empty
  name, draws the Note icon.
- The icon sits on a filled background in `color` (/C), framed by a thin
  dark outline (or another clear edge) so that it reads on a white page. The
  glyph colour is dark or light, whichever has more contrast with the
  background: compare `Color::luminance()` with a threshold the implementer
  picks so that pure yellow gets a dark glyph and pure blue and dark red get a
  light one.
- /BBox: `[0 0 N N]` where N = `ops::annots::NOTE_SIZE`, the app's note
  size, so a note the app creates fills its /Rect exactly. /Matrix: identity.
- The whole icon, outline included, lies inside the /BBox.
- With `alpha` < 1, an ExtGState applies it (3.0.3).

ISO says a Text annotation behaves as if NoZoom and NoRotate were set.
The surviving `view::fix` already draws it at /BBox size from the top-left
corner of /Rect, upright. Nothing here repeats that.

### 3.2 Highlight (`highlight`), ISO §12.5.6.10

- For each quad, fill a region covering the quad. Ends may be rounded or
  square (implementer). Painting uses Multiply (3.0.3) and `color` (A5).
- Non-axis-aligned quads follow the quad's own edges (rotated text).
- Returned /Rect: the union, over all quads, of the painted region's bounds
  (rounded ends included), grown by a small margin of the implementer's
  choice no larger than 1/8 of the quad height. /BBox = that /Rect,
  /Matrix identity.
- A degenerate quad (zero height or width, or all four corners equal) paints
  nothing harmful: no NaN in the content, and a finite /Rect.

### 3.3 FreeText (`free_text`, `line_height`), ISO §12.5.6.6 and §12.7.4.3

#### 3.3.1 Frame

- `bbox` is the annotation's /Rect. It is the form /BBox, with identity
  /Matrix. `rect` is /Rect less /RD (the caller passes `design_rect`). All
  drawing happens inside `rect`.
- Painting order: background (`background`, fills `rect`), then border
  (`border` width, `color` or black, `dash`; the whole stroke lies inside
  `rect`), then text. The text is clipped so that it never paints over the
  border or outside `rect`.
- `rotate` 90, 180 or 270: the text is laid out in a frame turned by that
  angle about `rect`, so that it reads upright to a user viewing the page
  with the same /Rotate (the app sets /Rotate to the page's rotation). The
  available width and height swap for 90 and 270.

#### 3.3.2 Text

- Font: the /DA font name is the resource name of the main font, a
  `Base14` simple font in WinAnsiEncoding (`Base14::dictionary`) at `size`.
  Text colour: `color`, else black.
- Characters go through spec 07's encoder (`scripts::lines` with
  `latin_extended` true). WinAnsi characters use the main font. Greek,
  Cyrillic, Latin letters outside WinAnsi, and CJK (CID fonts chosen by
  `han`) use spec 07's fallback fonts. Each fallback font used becomes a
  /Resources /Font entry under a resource name of the implementer's choice.
  The names must be distinct from `font_name` and from each other. A
  character no font encodes is drawn as `?`.
- Ignorable characters, combining-mark composition, and the line-break rules
  are those of spec 07 §4.3–4.4: CR, LF and CRLF each end a line; wrap at
  spaces; a word wider than the line breaks between characters (A7).
- Alignment by `quadding`. Trailing spaces at a wrap point do not count
  toward a line's width when centring or right-aligning.
- Lines whose baseline falls below the clipped box are not written. They
  would be invisible.

#### 3.3.3 Vertical metrics

- `line_height(size)` is the distance between consecutive baselines. It is
  a single factor × `size`, chosen by the implementer for comfortable
  reading. It must be the same factor for every face and positive for any
  positive finite size.
- The first baseline sits at least one ascent below the top of the padded
  text box, so the first line's capitals are fully inside the box.
- Padding between the border and the text is the implementer's choice, at
  least the border width.

#### 3.3.4 CJK font objects

A CJK fallback font in /Resources /Font is written as a Type0 font
dictionary, with its CIDFont in /DescendantFonts and the CIDFont's
/FontDescriptor as **direct** dictionaries. `annots::put_appearance` turns
them into indirect objects when it stores the form (ISO §9.6.2 needs the
descriptor to be indirect). The view keeps them direct. The dictionaries
come from spec 07 (`Cjk::font`, `Cjk::descendant_font`, `Cjk::descriptor`).

#### 3.3.5 Fit contract (with `free_text_fit`, kept)

For any text of WinAnsi lines that `free_text_fit` accepts, the box it
returns is `n × line_height(12)` tall and `max line advance + 0.5` wide.
`free_text` on that box with Helv 12, no border and no /RD must:

- draw every line, unwrapped, with each line's baseline inside the box;
- put the first line's vertical centre (half of `line_height(12)` below the
  top of the box) within 1 pt of where `free_text_fit` placed it.

With no padding the boxes are tight. The implementer either uses zero
horizontal padding when the border is 0, or keeps the wrap width from
cutting a line that `free_text_fit` measured as fitting. A 0.5 pt slack
already exists for rounding.

### 3.4 Square and Circle (`rectangle`, `border_allowance`), ISO §12.5.6.8

- The border path runs along `inner`: a rectangle, or the ellipse inscribed
  in it (ISO allows Bézier approximation; the error must be under 0.1% of
  the radius). It is filled with `style.fill` (/IC) and stroked with
  `style.stroke` (/C), dashed per `style.dash`, at `style.width`.
- `bbox` is the form /BBox (the caller passes the /Rect). /Matrix identity.
- `border_allowance(w)`: the /RD on each side when the app makes a Square,
  Circle or Polygon from a rectangle drawn by the user. The user's rectangle
  is the border's centre line, so it must be at least `w/2` and is the
  implementer's choice beyond that (to cover anti-aliasing). It is
  monotonic in `w`, and finite for finite `w`.

### 3.5 Line (`line`), ISO §12.5.6.7

- Stroke from `a` to `b` with `style.stroke` (A5: black when None), width,
  dash.
- Line endings at `a` (`endings[0]`) and `b` (`endings[1]`), ISO Table 179:
  Square, Circle, Diamond, OpenArrow, ClosedArrow, None, Butt, ROpenArrow,
  RClosedArrow, Slash. **Need:** each name drawn as a recognisable shape of
  that name, oriented along the line. Arrows point away from the line at
  their end. Reversed arrows point back along the line. Closed shapes
  (Square, Circle, Diamond, ClosedArrow, RClosedArrow) are filled with
  `style.fill` (/IC) when present and stroked with /C. Butt is a short
  perpendicular bar. Slash is a short bar at an angle. The size of an ending
  grows with the width and has a minimum, so endings are visible at widths
  of 1 or less. Exact proportions are the implementer's choice.
- An unknown name, b"", or b"None" draws no ending.
- A zero-length line (`a == b`) draws its endings in a fixed direction (+x)
  and produces no NaN.
- Returned /Rect: bounds of the line and both endings, grown by at least half
  the stroke width. /BBox = /Rect.

### 3.6 Ink (`ink`, `ink_allowance`), ISO §12.5.6.13

- Each stroke is a path through its points in order. Straight segments, or a
  smooth curve that passes through every point, are both allowed. Round caps
  and joins, so it looks like a pen. Stroked with `style.stroke` (A5),
  width, opacity. A stroke of one point draws a dot of the line width.
- `ink_allowance(w)` ≥ `w/2`. It is the implementer's choice beyond that,
  monotonic and finite.
- Returned /Rect: bounds of all points grown by `ink_allowance(width)`.
  /BBox = /Rect. Callers write /RD = `ink_allowance(width)` on every side,
  so /Rect less /RD is the points' bounds. `annot-move` relies on that for
  drawn signatures.
- Callers bound the point count (section 6).

### 3.7 Underline, StrikeOut, Squiggly (`text_line`), ISO §12.5.6.10

For each quad, with h = the length of the quad's `ll→ul` side:

- **Underline:** a straight stroke parallel to `ll→lr`, in the lower part of
  the quad (below the x-height and above the quad's bottom edge).
- **StrikeOut:** a straight stroke parallel to `ll→lr`, through the middle
  band of lower-case letters (between 30% and 50% of h from the bottom).
- **Squiggly:** a wave or zigzag along the bottom of the quad, its
  amplitude and period proportional to h.
- Stroke width proportional to h, so the marks scale with the text. The
  factor is the implementer's choice, visible at h = 10 pt.
- Colour `color` (A5), opacity `alpha`.
- Returned /Rect: union of the quads' bounds and the strokes' bounds, grown by
  at least half the stroke width. /BBox = /Rect.
- **Bound:** a Squiggly's vertices over all its quads are at most 32,768
  (the app's limit). When the natural period would exceed that, the period
  widens uniformly so that every quad still gets a wave. 400 quads 590 pt
  long and 0.001 pt tall must give under 2 MiB of content.

### 3.8 Caret (`caret`), ISO §12.5.6.11

- **Need:** a recognisable caret (an insertion mark shaped like `^`), filled
  with `color` (A5: black), opacity `alpha`.
- /BBox: the implementer's choice of size and aspect (wider than tall is
  usual). The view maps it onto /Rect less /RD (decision A3), stretching it.
- /Sy /P (a paragraph symbol) may be ignored.

### 3.9 Icon stamp (`stamp`, `icon_fit`), ISO §12.5.6.12

- **Label** (decision A2): from `name` (the /Name bytes; empty means
  `Draft`, the ISO default):
  1. Read the bytes as Latin-1 characters.
  2. Insert a space before each upper-case ASCII letter that follows a
     lower-case ASCII letter, and before each digit run that follows a
     letter.
  3. Upper-case the ASCII letters.
  4. Encode as WinAnsi (`fonts::encode`). A character with no code becomes
     `?`.

  Examples: `Approved` → `APPROVED`, `AsIs` → `AS IS`, `ForComment` → `FOR
  COMMENT`, `NotForPublicRelease` → `NOT FOR PUBLIC RELEASE`, `TopSecret`
  → `TOP SECRET`, `Void` → `VOID`, `approved` → `APPROVED`, `Draft2` →
  `DRAFT 2`.
- **Design:** the label in a bold standard face (`Base14::HelveticaBold` or
  `TimesBold`, as a `/Resources /Font` entry from `Base14::dictionary`),
  inside a border, all in `color` (A5 default). Any further decoration is
  the implementer's own design. Nothing may cross the /BBox.
- **Fit:** the label is centred and scaled to fill the box, with a margin
  inside the border. A label too long for one line at a readable size
  (judge by its `Base14::width`) breaks at a space into two lines. Very long
  labels (any name, up to 127 bytes) shrink to fit and never cross the
  border.
- **Box:** a fixed aspect ratio of the implementer's choice, **between 2:1
  and 5:1** (wider than tall). /BBox = `[0 0 W H]` in that aspect.
  /Matrix = `matrix`.
- `icon_fit(rect)`: the largest rectangle of the box's aspect that fits
  inside `rect`, centred on `rect`'s centre. A degenerate or huge `rect`
  gives finite values (zero width gives a zero-size rect at the centre).
  Since the aspect is at least 2:1, a 160 × 100 request keeps its width and
  shrinks its height.
- Opacity `alpha` per 3.0.3.

### 3.10 Image stamp (`image_stamp`)

This form is functional, not artwork, and is specified exactly:

- /BBox `[0 0 1 1]`, /Matrix `matrix`.
- /Resources `<< /XObject << /I <image reference> >> >>`. The name **must**
  be `I`, because `stamp_images` looks for it.
- Content: `/I Do\n` when `placement == IDENTITY`. Otherwise `q\n`, the six
  numbers of `placement` and `cm\n`, then `/I Do\nQ\n`.

### 3.11 Redact mark (`redact`), ISO §12.5.6.23

- With quads: each quad outlined as a closed polygon through its four
  corners (ll, lr, ur, ul), stroked with `color` (A5 default), width the
  implementer's choice (≥ 0.5 pt). Returned /Rect: union of the quads'
  bounds grown by an allowance ≥ half the outline width. /BBox = that /Rect.
- Without quads: the outline is `rect` inset by at least half the outline
  width, so the stroke lies inside `rect`. The returned /Rect is `rect`
  unchanged. Area marks keep the user's rectangle.
- No fill. The mark only shows where redaction will apply. /OC, /OverlayText
  and /RO are not drawn.

---

## 4. Synthesis for annotations without an appearance (`view`)

`synthesize` reads the dictionary through the surviving readers (1.3) and
calls section 3. It returns `None` when /Subtype is missing or not a name,
or the kind is not in the table, or the row says so.

| Kind | Inputs | Result (`rect`, `rd`) | None when |
|---|---|---|---|
| Text | /Name (default `Note`), /C, opacity | `note(...)`; rect = `[x0, y1 − N, x0 + N, y1]` from /Rect, N = `NOTE_SIZE` | no usable /Rect |
| FreeText | as `ops::annots::free_text_form` | that form; rect = /Rect | no usable /Rect |
| Highlight, Underline, StrikeOut, Squiggly | /QuadPoints (first 16,384 quads), /C, opacity | section 3.2 / 3.7 form and rect | no quads (A6) |
| Square, Circle | /Rect, /RD, /C, /IC, border, dash, opacity | `rectangle(inner, outer, …)` where inner = /Rect less (/RD, or half the border width on each side when /RD is absent); rect = /Rect; rd = `Some([w/2; 4])` when /RD was absent, else None | no usable /Rect |
| Line | /L, /LE (default None), /C, /IC, border, dash | `line(...)` | no usable /L |
| Polygon, PolyLine | /Vertices (first 131,072 points), style | `poly(...)` | fewer than 2 points (A6) |
| Ink | /InkList (131,072 points in total at most, strokes in order, empty strokes dropped), style | `ink(...)`; rd = `ink_allowance(w)` on each side | no points (A6) |
| Stamp | /Name (default `Draft`), /C, opacity | `stamp(name, …, IDENTITY)`; rect = `icon_fit(/Rect)` | no usable /Rect |
| Caret | /C, opacity | `caret(...)`; rect = /Rect (A3) | no usable /Rect |
| Redact | /Rect, /QuadPoints, /C | `redact(...)` | no usable /Rect |

"Usable /Rect" means the `annots::rect` reader returns one. The style for
shapes is `Style { stroke: /C, fill: /IC, width: the border width under decision A4 (0 draws no border; negative or non-numeric is 1), opacity, dash: border_dash }`.

`synthesized_box(doc, annot, outer)`:

- `None` when the annotation has a normal appearance (a stream at /AP /N, or
  /AP /N a dictionary with an /AS or /Off stream), as `view::fix` judges it.
- Otherwise the (rect, rd) of the table for Text, Stamp, Caret, Square,
  Circle, FreeText and Redact (Redact: union of the first 16,384 quads grown
  as in 3.11, else `outer`), and `None` for every other kind (their box is
  their geometry).

This is the box `annots` lists (`page_rect` insets it by rd, or by the
file's /RD when rd is None). It is also the /Rect and /RD that `annot-set`
writes for such an annotation.

---

## 5. Standard-font data (`fonts`)

### 5.1 Resource names

`from_resource_name` maps the /DA font names that Acrobat's AcroForm
default resources use, and the full /BaseFont names:

| Names | Face |
|---|---|
| `Helv`, `Helvetica` | Helvetica |
| `HeBo`, `Helvetica-Bold` | HelveticaBold |
| `HeOb`, `Helvetica-Oblique` | HelveticaOblique |
| `HeBO`, `Helvetica-BoldOblique` | HelveticaBoldOblique |
| `TiRo`, `Times-Roman` | TimesRoman |
| `TiBo`, `Times-Bold` | TimesBold |
| `TiIt`, `Times-Italic` | TimesItalic |
| `TiBI`, `Times-BoldItalic` | TimesBoldItalic |
| `Cour`, `Courier` | Courier |
| `CoBo`, `Courier-Bold` | CourierBold |
| `CoOb`, `Courier-Oblique` | CourierOblique |
| `CoBO`, `Courier-BoldOblique` | CourierBoldOblique |
| anything else (`Symb` and `ZaDb` included) | Helvetica |

`Symb` and `ZaDb` draw Helvetica on purpose. Typed text stays readable
instead of turning into symbols.

### 5.2 WinAnsi

`win_ansi(c)`: U+0020–U+007E and U+00A0–U+00FF map to the same byte. Codes
0x80–0x9F map per ISO 32000-2 Annex D (Table D.2, WinAnsiEncoding column):
take each code's glyph name and the Unicode value the Adobe Glyph List gives
it. The codes Annex D leaves undefined in WinAnsi (0x81, 0x8D, 0x8F, 0x90,
0x9D) map from no character. Generate the table from Annex D and the AGL.
Do not type it in.

### 5.3 Widths

- Source: the AFM files of the standard faces. Use either the URW++ base-35
  AFMs released under SIL OFL 1.1 (the release whose licence is kept in
  `engine/reference/licenses/urw-metrics/OFL.txt`; the files Nimbus Sans,
  Nimbus Roman and Nimbus Mono PS in Regular, Bold, Italic or Oblique, and
  Bold Italic or Bold Oblique) or Adobe's Core 14 AFM files. The two are
  metric-compatible for WinAnsi glyphs. See open question Q3 on fetching.
- For each face and each WinAnsi code with a glyph, the advance is the
  AFM `WX` of the glyph Annex D names at that code. Courier faces have 600
  for every glyph.
- Each of the 12 faces uses its own AFM (decision A8).
- Generate a Rust data file with a header comment naming the source file
  names, their SHA-256 and the OFL notice (or the Adobe notice). Do not
  type the tables in.
- Sanity values from the public AFMs (acceptance 9.3): Helvetica `space`
  278, `A` 667, `W` 944; Helvetica-Bold `A` 722; Times-Roman `A` 722, `a`
  444; Times-Bold `A` 722, `a` 500; Courier anything 600.

---

## 6. Limits and safety

- No panics, only safe Rust, no recursion on input-controlled depth.
- Inputs may be hostile: coordinates of ±1e300, NaN through `f64` math,
  thousands of quads. Every returned rect is finite and non-inverted.
  Content never contains `NaN` or `inf`.
- Bounds per synthesized appearance: 16,384 quads, 131,072 points
  (Polygon, PolyLine, Ink combined), 32,768 Squiggly vertices, 65,536
  FreeText characters (`MAX_FREE_TEXT_CHARS`) and 65,536 lines. Per view:
  100,000 appearances and 32 MiB of content (these stay in `view::fix`'s
  `Budget`).
- Content size is linear in the drawn elements. No quadratic string
  building.

---

## 7. Tolerant behaviour and experiment log

Harness: `reviewer archive spec-06-08/annot-synth.mjs`
(`slot node annot-synth.mjs`). It builds a one-page PDF with
one annotation without /AP. It opens the file with npm `mupdf` 1.28.1,
changes /Contents so the annotation is dirty, calls `update()`, and prints
whether an /AP was written, the resulting /Rect, and only the colour
operators and string operands used. Stream bytes were not recorded.

| Input | MuPDF result | This spec |
|---|---|---|
| Text, /Name /Foo | AP made, /Rect kept 20×20 | Note icon (3.1) |
| Stamp /Name /Void | AP, /Rect fitted about the centre, label `Void` as is | label `VOID` (A2), box per 3.9 |
| Stamp, no /Name | label `DRAFT` | label `DRAFT` (ISO default name) |
| Stamp /Name /approved (lower case) | label `approved` as is | `APPROVED` |
| Square /C [1 0] (two numbers) | stroked, gray operator (default black) | /C unusable → black stroke (A5) |
| Square /BS /W -3 | stroked | width read as default 1 (A4) |
| Square /BS /W 0 | stroked (hairline) | no border (A4) |
| Line /LE [/Foo /ClosedArrow] | AP; /Rect covers line and arrow | unknown ending = none; arrow drawn |
| Highlight without /QuadPoints | AP written with /Rect [2147483520 2147483520 -2147483648 -2147483648] | no AP, /Rect kept (A6) |
| Ink /InkList [] | same inverted /Rect | no AP, /Rect kept (A6) |
| Caret /Rect 100×100 | /Rect replaced by a small fixed box at the centre | caret fills /Rect (A3) |
| Polygon with one vertex | AP, tiny /Rect around the point | no AP (A6) |
| FreeText without /DA | drawn in black, default font and size | Helv 12 black (`default_appearance`) |
| FreeText /DA size 0 | drawn at a default size | size 12 (`free_text_form` replaces size ≤ 0 with 12) |

Other tolerant rules, by design:

- Colours: an array of a length other than 1, 3 or 4 is unusable
  (`Color::read`). Component values outside 0..1 are written as given; PDF
  clamps them (ISO §8.6.4).
- /QuadPoints with a length that is not a multiple of 8: the trailing
  partial quad is ignored (surviving `annots::quads`).
- Quads in another corner order (counter-clockwise ISO order instead of
  Acrobat's): drawn as given. Highlights still cover the same area.
  Underlines may land on another edge. Accepted: Acrobat order is what
  writers produce.

---

## 8. User-visible changes

- New artwork: note icons, caret, line endings, squiggle, stamp design and
  stamp box aspect. Stamp labels are the /Name in capitals with spaces
  (`VOID`, not `Void`).
- An icon stamp's /Rect aspect changes (the implementer's aspect, 2:1–5:1).
  Existing app-made stamps keep their stored appearance.
- Carets fill their rectangle. Annotations with empty geometry get no
  synthesized appearance.
- FreeText line spacing and first-baseline position may change slightly
  (`line_height`). Fit boxes are sized from the same function, so they stay
  tight. Long words wrap.
- A Square, Circle or other shape with border width 0 shows no border.
- Bold and oblique faces measure with their own widths, so centred and
  right-aligned bold text sits correctly.

---

## 9. Acceptance

### 9.1 Keep (black-box; scrub names and comments as listed)

`engine/tests/app_annots.rs` (30 tests; 21 kept unchanged):

| Test | Note |
|---|---|
| `a_note_brings_a_popup_and_the_dictionaries_keep_mupdf_order` | dictionary keys and popup, no stream bytes; rename (drop "mupdf") |
| `refusals_come_in_engine_ts_order_and_journal_nothing` | |
| `signed_and_locked_documents_list_but_refuse_edits` | |
| `move_keeps_size_clamps_and_turns_a_landing_in_place_into_a_no_op` | relative to the fit box; drawn-signature resize |
| `delete_takes_the_popup_and_cuts_references_once_no_page_lists_it` | scrub comment at line 763 |
| `delete_removes_an_entry_that_reaches_the_annotation_through_a_chain` | |
| `listing_follows_mupdf_and_reports_geometry_in_page_space` | rename |
| `image_stamps_and_signatures_draw_the_image_over_their_rect` | asserts `/I Do\n` (3.10 keeps it exact) |
| `the_view_draws_annotations_without_an_appearance_and_saves_none` | pixels: fill and border colours of a synthesized Square (border W 3 inside /Rect) |
| `the_view_draws_a_note_at_its_icon_size_from_the_top_left` | foreign 16×16 form, placement by kept `view::fix`; scrub comment at line 1108 |
| `random_requests_never_panic_and_undo_to_the_start` | |
| `page_space_matches_the_view_on_every_rotation` | |
| `delete_never_cuts_the_page_tree_or_what_a_popup_entry_wrongly_names` | |
| `remove_many_takes_marks_on_several_pages_in_one_pass` | |
| `a_stamp_signature_with_rect_differences_moves_at_a_stable_size` | scrub comment at line 1492 |
| `moving_a_drawn_signature_without_strokes_is_refused` | |
| `stamps_on_turned_pages_draw_inside_their_rect` | ink inside the listed rect and ≥ 80% of its width: the stamp design must fill its box across |
| `pages_sharing_one_annots_array_draw_its_annotations_on_each` | |
| `an_add_on_a_page_sharing_its_annots_array_lands_on_that_page_alone` | |
| `identical_stamp_images_share_one_image_object` | relies on resource name `I` |
| `a_delete_cuts_a_name_or_number_tree_pair_whole` | |

Also kept:

- `engine/tests/app_accept_minors.rs`
  `free_text_draws_format_characters_as_nothing_and_composes_accents`:
  pixel equality between equivalent texts, black-box.
- `engine/tests/app_view.rs` `a_view_after_an_edit_rewrites_only_what_the_edit_touched`
  and `a_kept_rewrite_is_charged_once_to_the_content_budget`: they add a
  note; budgets only.
- `engine/tests/app_view_corpus.rs`
  `a_fixed_size_appearance_is_drawn_at_its_bbox_size_whatever_its_matrix`:
  kept `view::fix`.
- `src/app/ops/annots.rs` unit tests `civil_dates`,
  `default_appearances_parse`.
- Node: `tests/p2-annots2.test.mjs` (all): `/AP` present, journal steps,
  stamp keeps x0/x1 and centre with a shorter height for a 160×100 request
  (holds for any aspect ≥ 2:1), image stamp keeps its rect.
  `tests/draw-signature.test.mjs`, `tests/sign-merge.test.mjs`,
  `tests/p2-edit.test.mjs`, `tests/p3-journal.test.mjs`: these do not depend
  on stream bytes.

### 9.2 Replace

The replacements below are written on the reviewer side **before hand-off**.
The old versions hold MuPDF content streams, constants and labels, so they
must not be in the implementer's tree, not even commented out.

| Test | MuPDF-specific part | Replacement check |
|---|---|---|
| `app_annots.rs` `every_kind_is_written_as_mupdf_writes_it` | exact content streams for 10 kinds (MuPDF artwork, constants, label positions) and /Rects derived from MuPDF's constants (highlight reach, stamp box) | Keep the request list, step names, /P, /F, junk-key checks, undo/redo and the listed rects (the request geometry: note 20×20, quads, square inner, line, ink; the stamp's listed rect is `icon_fit` of the request, centred, aspect in [2, 5]). Rename `every_kind_is_written_with_its_appearance`. For each kind, instead of bytes: (a) the content parses with the engine's content lexer, and has only §8.4–8.5 and §9.4 operators; (b) the form's /BBox contains /Rect less /RD; (c) rendering the page through the view (scale 2) puts non-white pixels only inside the annotation's /Rect (1 px tolerance) and some inside it; (d) colour: the dominant non-white pixel colour is the requested /C (Highlight: yellowish over white); (e) Stamp: the saved file's structured text (`view` text extraction or mupdf.js) inside the rect reads `DRAFT`; (f) FreeText: extracted text reads `free text`; (g) Square: /Rect = inner grown by `border_allowance(1)` and /RD = `[border_allowance(1); 4]`; Ink /RD = `ink_allowance(1.5)`. |
| `app_annots.rs` `free_text_fits_its_lines_and_refuses_what_it_cannot_write` | exact /Rect from MuPDF's line height and an exact text-position operator | Rect = `[300, 200 − lh/2, 300 + w, 200 − lh/2 + 2·lh]` in page space, with lh = `appearance::line_height(12.0)` and w = the widest Helvetica-12 advance + 0.5; extracted text has two lines `fit me`, `line2`; refusal and wrap parts unchanged except the wrap list is compared by extracted line text (`a long line of text that will`, …), not by `Tj` operands |
| `app_annots.rs` `set_rewrites_contents_and_refits_a_text_box_from_its_top_left` | exact /Rect height from MuPDF's line height | top-left kept; height = 2 × `line_height(12.0)`; width from Helvetica advances; rest unchanged |
| `app_annots.rs` `rotated_pages_keep_what_the_user_sees` | an exact frame-rotation operator in the FreeText content | keep the rest; replace that assertion: render the page through the view (scale 2), and the FreeText's ink lies inside its listed rect; text extraction of the saved page gives `turned`, with its characters advancing along the page's turned x direction as the user sees it (top to bottom in the PDF's unrotated y, for /Rotate 90) |
| `app_annots.rs` `a_squiggly_draws_a_bounded_zigzag_however_many_thin_quads_it_has` | MuPDF's zigzag step: an exact vertex count for a 14×14 quad | keep the 2 MiB and finite-rect checks and the render; replace the count with: a 14 pt-high, 140 pt-long quad's Squiggly has at least 10 and at most 200 vertices, and its ink spans ≥ 90% of the quad length |
| `app_annots.rs` `free_text_lays_out_inside_its_rect_differences` | exact stream prefix with MuPDF border and frame numbers | keep listed rect, /BBox = /Rect = [100 600 300 700], and the move check; replace the stream prefix with: rendering shows the border inside /Rect less /RD ([120 610 270 690] in user space) and no ink outside it; the text `changed` extracts from inside that box |
| `app_annots.rs` `annotations_without_an_appearance_list_and_save_the_rect_mupdf_derives` | stamp, caret and redact rects from MuPDF constants | rename `…list_and_save_the_synthesized_rect`. Keep Text (`[72 342 92 362]`, set writes `[72 430 92 450]`), Square (`/RD 0.5`), Circle (`/RD 2`), FreeText. Stamp: listed rect = `icon_fit([100 400 300 600])` as page space, centred at (200, 292), width 200, height = 200 / aspect with aspect in [2, 5]; set writes the same in user space. Caret: listed = /Rect = `[50 722 70 742]`. Redact: contains the quad `[110 102 200 112]` (page space) and is within 2 pt of it on each side. "Nothing above the stamp" render check kept. |
| `app_annots.rs` `free_text_draws_greek_cyrillic_cjk_and_latin_letters_in_fallback_fonts` | MuPDF's fallback resource names and exact byte runs | keep the render checks (each letter draws and is not a `?`), the CJK font object structure (Type0 → indirect CIDFont → indirect descriptor), the Differences check (`zdotaccent`, `lslash` named in some font's /Differences). Replace names and runs with: /Resources /Font has the DA font `Helv` plus at least one font for each script, all distinct; text extraction of the saved page (mupdf.js `toStructuredText`, and pdf.js `getTextContent` if available in the test environment) returns `Ωμέγα Жук 日本語 Zażółć` (whitespace normalised); the Cyrillic edit and Greek move cases: extracted text equals the contents and no `?` |
| `app_annots.rs` `an_icon_stamp_fits_inside_a_rect_wider_than_its_icon` | MuPDF's exact fitted rects | listed and saved rects = `icon_fit(request)`: height kept (50, 90) for these wide requests, aspect in [2, 5], centred; foreign stamp: same rule on [300 600 500 650] |
| `src/app/annots/appearance.rs` unit tests (8): `stamp_labels_are_mupdfs`, `a_note_glyph_reads_on_its_background`, `free_text_lines_break_as_mupdf_breaks_them`, `free_text_runs_take_mupdfs_fallback_fonts`, `free_text_keeps_winansi_spaces_skips_ignorables_and_composes_marks`, `a_dashed_free_text_border_stays_dashed`, `an_icon_stamp_fits_inside_its_rect`, `hostile_geometry_stays_finite_and_bounded` | deleted with the file; they assert MuPDF labels, artwork bytes, line-break and font-run bytes | new unit tests, section 9.3 items 1–8 |
| `src/app/annots/view.rs` unit test `one_synthesized_appearance_draws_bounded_quads_and_points` | counts `f\n` operators (one per quad) | new test: a Highlight with 16,884 quads draws exactly 16,384 quads' worth (render or count painted subpaths with the content lexer); an Ink with two 131,072-point strokes uses 131,072 points (count path-construction points with the content lexer) |
| `app_accept_minors.rs` `latin_letters_winansi_lacks_store_a_widget_appearance` | (spec 07) | see spec 07 |
| `app_accept_minors.rs` `a_dashed_free_text_border_is_regenerated_dashed` | exact operator bytes | parse the content: a `d` operator with array `[3 2]` comes before the border stroke, and the stroke width is 2; keep the pixel gap check |
| `app_accept_minors.rs` `an_invalid_free_text_dash_array_falls_back_to_the_default` | exact operator bytes | parse: `d` with `[3]` (ISO default), width 2 |
| `app_redact.rs` `an_area_mark_is_one_step_with_mupdfs_dictionary` | exact outline stream | rename; keep dictionary keys, /Rect [0 712 80 772] (area mark keeps its rect, 3.11), numbering; replace the stream with: content parses; one closed stroked path; rendering shows the outline inside /Rect and touching within 2 pt of each edge; stroke colour conspicuous (not white or black) |
| `app_redact.rs` `search_marks_every_hit_with_quads_and_the_tag` | exact /Rect from MuPDF's growth | /Rect contains the quads' union and exceeds it by at most 2 pt per side; rest unchanged |
| node `tests/fill-anywhere.test.mjs` tests (a) at lines 47 and 100, and the drive at 187 | exact line-height numbers, one `Tj` per line | take H from a one-line fit box's height; check first-line centre at `rect[1] + H/2`, two-line box `2H` tall; count drawn lines by extracting the saved page's structured text with mupdf.js (lines inside the box), not by `Tj` |

Tools and records:

- `engine/tools/compare-annots.mjs` and
  `engine/reference/app-annots-20261015/summary.json` compare pixels with
  MuPDF's renders. They are records, not gates. They stay reviewer-side and
  are not handed to the implementer. After landing, a new record is made for
  information only. Icon, stamp, caret and FreeText rows will drop, by
  design.

### 9.3 New cases

1. `note`: for each standard name and `Foo`, the form renders inside its
   /BBox, and the seven standard icons render pairwise different. A yellow
   /C gives a darker glyph than background, and a dark blue /C a lighter
   one (sample the rendered icon).
2. Stamp labels: `Approved` → extracted text `APPROVED`; `NotForPublicRelease`
   → `NOT FOR PUBLIC RELEASE` (one or two lines); `Void` → `VOID`; empty name
   → `DRAFT`; a 127-byte name renders with no ink outside the /BBox.
3. `icon_fit`: aspect of the result equals the stamp /BBox aspect; centred;
   inside the input; `[5 5 5 9]` → zero-width rect at (5, 7); ±1e30 finite.
4. Every kind through `synthesize` with hostile geometry (1e300, NaN
   through arithmetic, zero-size quads): finite rect, no `NaN`/`inf` in
   content, content < 4 MiB.
5. Line endings: for each of the 10 names at width 1 and 5, rendering
   shows ink beyond the line's end point (except None), all inside the
   returned /Rect. ClosedArrow with /IC fills, OpenArrow does not (pixel
   inside the triangle).
6. Border width 0: Square with /BS /W 0 and /IC renders the fill and no
   darker outline. /W −3 renders as width 1.
7. Highlight multiply: black text under a yellow highlight stays darker than
   50% grey in the render.
8. FreeText:
   - `line_height` is linear in size;
   - for sizes 6, 12 and 36 the first line's glyph ink starts inside the
     box top;
   - a 300-character word in a 160 pt box wraps to several lines, none wider
     than the box;
   - /Rotate 90, 180 and 270 extract the same text;
   - `nbsp\u{a0}x shy\u{ad}x` keeps a single WinAnsi run (no fallback font
     added);
   - zero-width and format characters add nothing;
   - NFD `e\u{301}` renders as `é` (pixel-equal to NFC);
   - the CJK font in a stored form is three indirect objects.
9. Fit contract (3.3.5): for 1–5 lines of random WinAnsi text, `free_text`
   on `free_text_fit`'s box draws every line (text extraction) with no wrap.
10. Fonts: the AFM sanity values of 5.3. `Base14::width(b"Hello", 10)` for
    Helvetica = (722 + 556 + 222 + 222 + 556) / 100 = 22.78. `win_ansi` of
    U+20AC = 0x80 and U+0081 = None. `encode("a\tb日")` = `[0x61, 0x09,
    0x62, 0x3F]`. `from_resource_name` per the 5.1 table.
11. Opacity: `alpha` 0.5 on every kind writes an ExtGState with /CA and /ca
    0.5 that the content selects.

### 9.4 Comments and names to scrub (surviving files)

- `engine/tests/app_annots.rs`: module doc (lines 2–5); comments at 352,
  590, 763, 1108, 1389, 1400, 1429, 1492, 1545, 1831, 1941; test names
  containing `mupdf`.
- `engine/tests/app_redact.rs`: module doc line 5; test name at 190;
  comment at 393.
- `engine/src/app/annots/mod.rs`, `ops/annots.rs`, `redact/marks.rs`: the
  module doc of `mod.rs` (lines 3–8) and every comment that names a MuPDF
  function or describes MuPDF's internals (about 38 lines; find them with
  `grep -n -i 'mupdf\|pdf_\|fz_'`). Reword as plain behaviour. This scrub is
  done before hand-off by the reviewer side, not by the implementer.
- `engine/src/app/annots/view.rs` kept part: the module doc and `fix`
  comments ("as MuPDF draws it").
- `engine/src/app/ops/annots.rs`: the module doc and the comments on
  object numbering and `write_ink` key order that cite MuPDF.

---

## 10. Open questions for the owner

- **Q1.** Stamp label face: Helvetica-Bold (modern) or Times-Bold
  (traditional rubber stamp)? This spec lets the implementer choose.
- **Q2.** Should /Sy /P on a Caret draw a paragraph mark? Not required here.
- **Q3.** Fetching the AFMs. The URW OFL AFMs the engine already credits came
  from a MuPDF source distribution. Those files are URW's data, not MuPDF
  code, but the implementer may not open the MuPDF tree. Either a neutral person copies the AFMs into
  `engine/reference/licenses/urw-metrics/` (with their OFL notice) before
  hand-off, or the implementer uses Adobe's Core 14 AFMs. Their permissive
  notice must then be added to NOTICE.
- **Q4.** Should the view also synthesize FileAttachment and Sound icons
  (new artwork)? Today and in this spec they list their /Rect and draw
  nothing.

---

## Leak-check change log (2026-10-18, independent checker)

- 3.3.1: the border-inset and clip-inset geometry restated as outcomes (the
  stroke lies inside `rect`; text never paints over the border) instead of
  the old code's exact inset steps.
- 3.9: removed the list of candidate stamp decorations (tilt, rounded
  corners, double rule), which described the old artwork.
- 9.2: the replaced tests are rewritten reviewer-side before hand-off; the
  old ones (MuPDF streams and labels) never reach the implementer.
- Q3: removed the path into the MuPDF source tree.
