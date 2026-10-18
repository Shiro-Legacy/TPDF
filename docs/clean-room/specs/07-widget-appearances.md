# Spec 07: form widget appearances, variable text and fallback fonts

Workstream 7 of `docs/mit-clean-room-plan.md`. Status: draft for review,
2026-10-18. Rules: `docs/clean-room/SPEC-RULES.md`.

This spec replaces:

- `engine/src/app/forms/appearance.rs`: widget appearance synthesis;
- `engine/src/app/forms/text.rs`: WinAnsi text, standard-family metrics,
  literal strings;
- `engine/src/app/forms/scripts.rs`: script detection, the Greek,
  Cyrillic and extended-Latin 8-bit fonts, the CJK CID fonts, the shared
  line layout. Its KOI8-U, ISO 8859-7 and extended-Latin tables are
  regenerated from public sources (section 6).

It also provides the variable-text layout and fallback fonts that spec 06
(FreeText) uses.

Not replaced, and they stay: `forms/mod.rs` (field model and readers),
`forms/write.rs` (`Sink`, `appearance_dict`, `refresh`, `base14_font`),
`forms/view.rs` (view-only synthesis driver), `ops/fields.rs`.

Public sources: ISO 32000-2:2020 §12.7.4.3 (variable text), §12.7.5.2
(button fields), §12.7.5.3 (text fields), §12.7.5.4 (choice fields),
§12.5.6.19 and Table 192 (/MK), §12.5.4 (border styles), §12.5.5
(appearance streams), §9.6 (simple fonts, §9.6.5–9.6.6 encodings and
/Differences), §9.7 (CID-keyed fonts, §9.7.5.2 Table 116 predefined CMaps),
§9.8 (font descriptors), §9.10.2 (mapping to Unicode), Annex D; the
Adobe Glyph List and AGLFN; Unicode `Scripts.txt` (UAX #24),
`UnicodeData.txt` (canonical decompositions), `PropList.txt` and
`DerivedCoreProperties.txt` (Default_Ignorable_Code_Point); the Unicode
Consortium mapping files `MAPPINGS/ISO8859/8859-7.TXT` and
`MAPPINGS/VENDORS/MISC/KOI8-U.TXT`; Adobe's public CJK documents
(Adobe Technical Notes #5079 Adobe-GB1, #5080 Adobe-CNS1, #5078 Adobe-Japan1,
#5093 Adobe-Korea1, and the `adobe-type-tools/cmap-resources` CMap files);
BCP 47 (RFC 5646).

The implementer may read this spec, those sources, the acceptance tests of
section 9, the case inputs file (9.4) and the surviving engine code. The
implementer may not read the replaced files, their git history,
`engine/design/app-forms.md`, `app-accept-forms.md`, the recorded MuPDF
streams in `engine/reference/app-forms-20261015/synthesis.json`, or MuPDF
source.

---

## 0. Decisions taken in this spec (owner may override)

| # | Decision | Why |
|---|---|---|
| F1 | Look and proportions (padding, line spacing, baseline placement, check-mark size, selection colour, pressed-button look, CJK descriptor values) are the implementer's. Only outcomes are specified. | The old values are MuPDF's. |
| F2 | Multiline auto size (/DA size 0) shrinks the text until it fits the box, down to a readable minimum. | The value stays readable. MuPDF draws 12 pt and overflows (section 8). |
| F3 | A word wider than the line breaks between characters. | Long words stay visible. |
| F4 | The CJK font for Han follows the BCP 47 /Lang: `zh` with `Hant`, `TW`, `HK` or `MO` → traditional Chinese; other `zh` (`zh-SG` included) → simplified; `ja` → Japanese; `ko` → Korean; anything else or none → simplified Chinese. | It is correct BCP 47. MuPDF maps `zh-SG` to traditional and English to Japanese (section 8). |
| F5 | The CJK /BaseFont names are names Acrobat and pdf.js recognise as their non-embedded CJK fonts for that collection. | Acrobat then substitutes a real font instead of a generic one. |
| F6 | A negative or non-finite /DA size is auto size. | A mirrored or absurd size helps no one. |
| F7 | A list box draws one option per line (clipped at the right), not word-wrapped options. | Rows then line up with what a click selects, as in Acrobat. |
| F8 | /BS /S is honoured: S solid, D dashed, U underline (bottom edge only). B (beveled) and I (inset) are drawn with a 3-D edge, or as solid when not implemented. | ISO §12.5.4. The old code ignored /S. |
| F9 | In a comb field at most MaxLen characters are drawn. | ISO: the field holds MaxLen characters. |

---

## 1. Scope

### 1.1 What the component does

1. For a widget annotation (ISO §12.5.6.19) of a text, choice or button
   field, it builds the appearance streams a viewer shows: /AP /N for text
   and choice fields; Off and on states for check boxes and radio buttons;
   /AP /N and /D for push buttons. Signature fields get none.
2. It lays out variable text (§12.7.4.3): fonts from /DA and /DR, size,
   quadding, line breaking, comb cells.
3. For characters outside WinAnsi it chooses fallback fonts: Greek and
   Cyrillic 8-bit simple fonts, an extended-Latin 8-bit simple font, and
   CJK CID fonts through predefined UTF-16 CMaps. It reports a text as
   **lossy** when some character none of them encodes was replaced by `?`.

### 1.2 Callers (surviving code)

| Caller | Uses | Needs |
|---|---|---|
| `forms::write::refresh` (field-set, field-add, field-props, redaction sanitize, batch) | `Context::of`, `Context::synthesize_with`, `Synth::lossy`, `Synth` variants | When `lossy`, the widget's /AP is removed instead (viewers draw it from /V). Otherwise /AP is replaced. A `States` synth gives /AS /Off when /AS is not a name. |
| `forms::write::appearance_dict`, `form`, `font` | `Form::{fonts, content, dictionary}`, `FontSource` variants, `Eight::{font, suffix}`, `Family::base_font`, `Cjk::{name, descriptor, descendant_font, font}` | One font object per font per operation, keyed by `FontSource`. A CJK font is written as descriptor → CIDFont → Type0, each indirect. |
| `forms::view::appearances` (view-only, also for /NeedAppearances) | `Context::{of, need_appearances, synthesize, work}`, `Synth::States::on_name` | View budget: `work()` counts bytes and characters read |
| `ops::annots` (FreeText, spec 06) | `scripts::{Cjk, han_font}` and the crate-internal layout API (2.3) | FreeText layout and fonts |
| `ops::scan` (`scan-compose`, `scan-measure`) | `text::{Family, Metrics::family, win_ansi_code}`, `scripts::Eight::Latin.{widths, code, font}`, `Family::base_font` | The scanner's second font tier: Helvetica or Helvetica-Bold with the extended-Latin /Differences. It adds its own /ToUnicode to the dictionary `Eight::font` returns. |
| tests (`app_forms.rs`) | `Context`, `Synth`, `forms::appearance::Form`, `write::{Sink, appearance_dict}` | |

### 1.3 Surviving APIs the component builds on

- `forms::{inherited, field_flags, widget_type, WidgetType, to_int,
  text_of, name_text, text_prefix, value_prefix, choices_range, Choice,
  on_state, pdf_rect, rect_object, number, MULTILINE, PASSWORD, COMB,
  max_len}`. These are inherited-attribute reads, bounded value reads and
  the widget rect in user space.
- `simple_encoding::{SimpleEncoding, NamedEncoding, GlyphUnicode}`: Annex
  D encodings, with glyph names and Unicode per code.
- `builtin_metrics::{BuiltinFace, BuiltinMetrics}`: URW (OFL) advances by
  glyph name for Helvetica, Helvetica-Bold, Helvetica-BoldOblique,
  Times-Roman, with `lookup_glyph(name)`.
- `serialize::compact_real` (round before formatting, as in spec 06
  §3.0.1).

### 1.4 Non-goals

- JavaScript formatting (/AA /F). Values are drawn as stored.
- Rich text (/RV, /DS), file-select fields, rollover appearances (/R), the
  push button icon (/MK /I, /IX, /TP).
- Embedding font programs. Fallback fonts are not embedded (open question
  Q1).
- Right-to-left scripts, Indic shaping, Vietnamese beyond Latin
  Extended-A, emoji: these are lossy (`?`, and no stored appearance).

---

## 2. Interface

Every item is `pub` with exactly this signature unless marked **(free)**:
the implementer may change it because only class-A modules use it.

### 2.1 `crate::app::forms::appearance`

```rust
/// Characters of a value drawn at most; a longer value is cut there.
pub const MAX_DRAWN_CHARS: usize = 65_536;

/// A font an appearance's /Resources names.
#[derive(Clone, Debug, PartialEq)]
pub enum FontSource {
    /// The /AcroForm /DR font, as /DR holds it (a reference or a dictionary).
    Existing(Object),
    /// A new simple standard font with this /BaseFont (WinAnsiEncoding
    /// unless Symbol or ZapfDingbats): `write::base14_font` makes it.
    New(&'static [u8]),
    /// An 8-bit fallback font of a standard family (`Eight::font`).
    Eight { family: Family, eight: Eight },
    /// A CJK CID font (`Cjk::font` etc.).
    Cjk(Cjk),
}

/// One appearance stream: a Form XObject of `width` × `height` (the
/// widget's size, swapped for /MK /R 90 or 270) turned by `rotate`.
#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub width: f64,
    pub height: f64,
    /// 0, 90, 180 or 270.
    pub rotate: i64,
    pub content: Vec<u8>,
    /// Resource name and font, in the order the content first uses them;
    /// the DA font (when text is drawn) first.
    pub fonts: Vec<(Vec<u8>, FontSource)>,
    /// A character no font encodes was drawn as `?`.
    pub lossy: bool,
}
impl Form {
    /// /Type /XObject /Subtype /Form /BBox [0 0 width height] /Matrix
    /// (rotation of section 3.1.3, translation 0 0), and when `fonts` is
    /// not empty /Resources << /Font << name → font_refs[i] >> >>.
    pub fn dictionary(&self, font_refs: &[Object]) -> Dictionary;
}

#[derive(Clone, Debug, PartialEq)]
pub enum Synth {
    /// A text or choice field: /AP /N.
    Single(Form),
    /// A check box or radio button: /AP /N << /Off off /<on_name> on >>.
    States { off: Form, on_name: Vec<u8>, on: Form },
    /// A push button: /AP /N and /D.
    Push { normal: Form, down: Form },
}
impl Synth {
    /// The forms in the order above (Single: [n]; States: [off, on];
    /// Push: [normal, down]).
    pub fn forms(&self) -> Vec<&Form>;
    /// Some form is lossy.
    pub fn lossy(&self) -> bool;
}

/// The document's form defaults (/AcroForm /DA, /Q, /DR /Font) and the
/// work done with them so far.
pub struct Context<'d, 'a> { /* private */ }
impl<'d, 'a> Context<'d, 'a> {
    pub fn of(doc: &'d Document<'a>) -> Self;
    /// Bytes and characters every synthesis through this context has read
    /// (/DA strings, values, options), saturating.
    pub fn work(&self) -> usize;
    /// /AcroForm /NeedAppearances is the boolean true.
    pub fn need_appearances(&self) -> bool;
    /// `synthesize_with(widget, None)`.
    pub fn synthesize(&self, widget: &Dictionary) -> Option<Synth>;
    /// The widget's appearance. None when /Subtype is not /Widget, when no
    /// inherited /FT is a name, or for a signature field. `on` names a
    /// button's on state when its /AP has none (3.4.1).
    pub fn synthesize_with(&self, widget: &Dictionary, on: Option<&[u8]>) -> Option<Synth>;
}
```

`Da`, `Color` and the other current public helpers of this file have no
users outside it. They are **(free)**.

### 2.2 `crate::app::forms::scripts`

```rust
/// A CJK character collection (Adobe registry) and its non-embedded font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cjk { Gb1, Cns1, Japan1, Korea1 }  // simplified Chinese, traditional Chinese, Japanese, Korean
impl Cjk {
    /// Resource name and /BaseFont (F5): ASCII, distinct per variant.
    pub fn name(self) -> &'static str;
    /// Type0 font: /Subtype /Type0, /BaseFont, /Encoding = the
    /// collection's predefined UTF-16 horizontal CMap (Table 116:
    /// UniGB-UTF16-H, UniCNS-UTF16-H, UniJIS-UTF16-H, UniKS-UTF16-H),
    /// /DescendantFonts [descendant].
    pub fn font(self, descendant: Object) -> Dictionary;
    /// CIDFontType0 dictionary with /CIDSystemInfo (Registry Adobe, the
    /// collection's Ordering, a Supplement no lower than the CMap's) and
    /// /FontDescriptor `descriptor`; /DW and /W per section 5.4.
    pub fn descendant_font(self, descriptor: Object) -> Dictionary;
    /// The CIDFont's font descriptor (ISO §9.8, section 5.4).
    pub fn descriptor(self) -> Dictionary;
}
/// The collection Han and Bopomofo characters take for a /Lang value
/// (raw text-string bytes; None when absent) (F4).
pub fn han_font(lang: Option<&[u8]>) -> Cjk;

/// The 8-bit fallback encodings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eight { Greek, Cyrillic, Latin }
impl Eight {
    /// A short ASCII tag, distinct per variant, used in cache keys and
    /// resource names. The value is the implementer's.
    pub fn suffix(self) -> &'static str;
    /// The code of `c` in this encoding: ASCII (< 0x80) is itself; an
    /// upper-half code only when its glyph is drawable (section 6.3).
    pub(crate) fn code(self, c: char) -> Option<u8>;
    /// Advances of the 256 codes in `family`, 1/1000 em; 0 for codes
    /// not drawn. ASCII codes 0x20..0x7E take the family's WinAnsi advances.
    pub fn widths(self, family: Family) -> [f64; 256];
    /// The simple font: /Type /Font /Subtype /Type1 /BaseFont `base`,
    /// /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [...] >>
    /// naming every drawable upper-half code, /FirstChar, /LastChar,
    /// /Widths (rounded `widths`). No /ToUnicode (callers may add one).
    pub fn font(self, base: &[u8], family: Family) -> Dictionary;
}
```

Callers to update when `Cjk` variants are renamed: `ops/annots.rs` (type
only) and tests. `Eight::Latin` is the variant name `ops::scan` uses.

### 2.3 Crate-internal layout API (shared with spec 06)

```rust
/// The font a glyph is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot { Latin, Eight(Eight), Cjk(Cjk) }   // Latin = the DA font (WinAnsi)

/// One drawn character.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Glyph {
    pub slot: Slot,
    /// A byte for simple fonts; the Unicode scalar for a CID font.
    pub code: u32,
    /// Advance in 1/1000 of the font size.
    pub advance: f64,
    /// The character drawn (`?` when replaced).
    pub ch: char,
}

/// The fonts a text is encoded for.
pub(crate) struct Fonts<'m> {
    pub latin: &'m Metrics,     // the DA font's WinAnsi advances
    pub family: Family,         // its standard family, for 8-bit fallbacks
    pub han: Cjk,
    /// Latin letters WinAnsi lacks may use `Eight::Latin`.
    pub latin_extended: bool,
}

/// One laid-out line: glyphs (a wrap's trailing spaces included) and the
/// width alignment uses (trailing spaces excluded), at size 1.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Line { pub glyphs: Vec<Glyph>, pub width: f64 }

/// `text` as one line; true when some character became `?`.
pub(crate) fn encode(text: &str, fonts: &Fonts<'_>) -> (Vec<Glyph>, bool);
/// `text` broken into at most `limit` lines of at most `max_width` at
/// `size` (section 4.4); true when lossy.
pub(crate) fn lines(text: &str, fonts: &Fonts<'_>, size: f64, max_width: f64, limit: usize)
    -> (Vec<Line>, bool);
/// A run's string operand: a literal of bytes for simple fonts (bytes
/// outside printable ASCII as octal escapes), big-endian UTF-16 hex for a
/// CID font (surrogate pairs above U+FFFF).
pub(crate) fn string(run: &[Glyph]) -> String;
```

The implementer may reshape this API (it is crate-internal) when spec 06
and 07 land together. It must keep one layout for FreeText and widgets.

### 2.4 `crate::app::forms::text`

```rust
/// A standard family, for advances when a font has no /Widths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family { Helvetica, HelveticaBold, Times, Courier, Symbol, ZapfDingbats }
impl Family {
    /// The family a /BaseFont names: a six-letter subset tag and `+` are
    /// stripped; then, case-insensitively, contains "courier" → Courier,
    /// "times" → Times, "dingbats" or "zapf" → ZapfDingbats, equals
    /// "symbol" → Symbol, contains "bold" → HelveticaBold, else Helvetica.
    pub(crate) fn of(base: &[u8]) -> Self;
    /// Helvetica, Helvetica-Bold, Times-Roman, Courier, Symbol, ZapfDingbats.
    pub(crate) fn base_font(self) -> &'static [u8];
    pub(crate) fn symbolic(self) -> bool;  // Symbol or ZapfDingbats
}

/// Advances of the 256 WinAnsi codes, 1/1000 em.
#[derive(Clone, Debug)]
pub(crate) struct Metrics { /* private */ }
impl Metrics {
    /// The family's WinAnsi advances (URW data through `builtin_metrics`
    /// for Helvetica, Helvetica-Bold and Times; 600 for every Courier
    /// glyph; codes without a glyph 0).
    pub(crate) fn family(family: Family) -> Self;
    /// A simple font dictionary's /FirstChar + /Widths over its family's
    /// advances; codes outside /Widths take /FontDescriptor /MissingWidth
    /// (default 0); non-finite entries 0.
    pub(crate) fn font(doc: &Document<'_>, font: &Dictionary) -> Self;
    pub(crate) fn from_widths(widths: Vec<f64>) -> Self;
    pub(crate) fn advance(&self, byte: u8) -> f64;
}

/// WinAnsiEncoding code of `c` (Annex D); ASCII is itself; U+00A0 → 0xA0
/// and U+00AD → 0xAD.
pub(crate) fn win_ansi_code(c: char) -> Option<u8>;
/// `text` as WinAnsi bytes, `?` for characters without a code.
pub(crate) fn win_ansi(text: &str) -> Vec<u8>;            // (free)
/// Advance of `glyphs` at size 1.
pub(crate) fn advance(glyphs: &[Glyph]) -> f64;           // (free)
/// A PDF literal string: `(`, `)`, `\` escaped, bytes outside 0x20..0x7E
/// as three-digit octal escapes.
pub(crate) fn literal(bytes: &[u8]) -> String;            // (free)
pub(crate) fn font_dict<'d>(doc: &'d Document<'_>, value: &'d Object) -> Option<&'d Dictionary>; // (free)
```

The old single-precision number printer goes. Content numbers use
`compact_real` after rounding.

---

## 3. Widget appearances (required behaviour)

### 3.1 Common frame

#### 3.1.1 Inputs

- Size: from the widget /Rect (`pdf_rect`). /MK /R (§12.5.6.19, Table 192)
  of 90, 180 or 270 (mod 360; anything else is 0) turns the appearance. For
  90 and 270 the form's width and height are the rect's height and width.
- Border width: /BS /W when present (non-null), else /Border's third
  element, else 1. Negative or non-finite → 1.
- Border style: /BS /S (F8), dash /BS /D (default `[3]`).
- Background /MK /BG and border colour /MK /BC: arrays of 1, 3 or 4
  numbers (others: absent). Non-finite components → 0.
- /DA: the widget's inherited /DA, else /AcroForm /DA, else empty. A /DA
  longer than a bound of the implementer's choice (at least 4 KiB) reads as
  empty. Parsing: the operands before the last `Tf` give the font name and
  size; the last `g`, `rg` or `k` with the right operand count gives the
  colour. Defaults: font `Helv`, size 12 (as the surviving annotation /DA
  parser), no colour (text black). Huge sizes are clamped to a bound of the
  implementer's choice (at least 1,000 pt). Size 0, negative or non-finite
  means auto (F6).
- Quadding: inherited /Q, else /AcroForm /Q, else 0. Values other than 1
  or 2 are 0.

#### 3.1.2 Painting order (every form)

1. Background: /MK /BG fills the whole form box (radio buttons: the
   circle, 3.4.3).
2. Border: drawn when /MK /BC is present and the width > 0, in the style of
   F8, the whole stroke inside the box.
3. Content: text or the button mark, clipped so that text never paints over
   the border or outside the box.

Text and choice fields wrap their variable-text content in `/Tx BMC …
EMC` (§12.7.4.3).

#### 3.1.3 Matrix

`Form::dictionary` writes /BBox `[0 0 width height]` and /Matrix:
`[1 0 0 1 0 0]` for 0, `[0 1 -1 0 0 0]` for 90, `[-1 0 0 -1 0 0]` for
180, `[0 -1 1 0 0 0]` for 270. These are the rotations of §12.5.5. The
viewer's algorithm maps the turned /BBox onto /Rect, so no translation is
needed.

#### 3.1.4 Fonts (`Context` font choice)

- The DA font name looks up /AcroForm /DR /Font. When that entry is a simple
  font (/Subtype Type1, TrueType or MMType1) with /Encoding
  /WinAnsiEncoding, and its /BaseFont family (`Family::of`) is not
  symbolic, it is used as is: `FontSource::Existing`, measured with
  `Metrics::font`.
- Otherwise a new standard font of its family is used:
  `FontSource::New(family.base_font())`, Helvetica when /DR lacks the name
  or the family is symbolic. Its resource name stays the DA name.
- The form's `fonts` lists the DA font first (when text is drawn), then each
  fallback font used. Fallback resource names are the implementer's choice
  (e.g. the DA name plus `Eight::suffix`, or `Cjk::name`). They must be
  distinct from each other and from the DA name, and stable for a given
  (DA name, fallback) pair.

### 3.2 Text fields (§12.7.5.3)

#### 3.2.1 Value

- The inherited /V as text (`value_prefix`, cut at `MAX_DRAWN_CHARS`).
- Password flag: draw one `*` per character of the value. The value itself
  never appears in the content.

#### 3.2.2 Single line (no Multiline flag, no comb)

- One line, no wrapping. CR and LF are not line breaks here: they draw
  nothing.
- Horizontal: inside the padded box, aligned by /Q. Padding is the
  implementer's, at least the border width.
- Vertical: the line's glyph box (font ascent to descent at the chosen size)
  is centred in the padded box. **The baseline is always inside the box**,
  and for any size that fits, the capitals and descenders of a Latin line
  are inside the clip.
- Auto size: the largest size at which the line's width fits the padded
  width and its glyph box fits the padded height. An empty value takes the
  height-based size. The minimum is the implementer's (≥ 1 pt), never 0 or
  negative.

#### 3.2.3 Multiline (Multiline flag)

- Lines from the top of the padded box, `lines()` layout (4.4), aligned per
  line by /Q (trailing spaces at a wrap excluded).
- Line spacing: a single factor × size, the implementer's choice, the same
  in every family. The first baseline sits at least one ascent below the top.
- Auto size (F2): the largest size, from a cap of the implementer's
  choosing (common practice is 12 pt) down to a minimum (≥ 4 pt), at which
  the wrapped text's lines all fit the padded height. When nothing fits at
  the minimum, draw at the minimum, clipped.
- Lines whose top is below the box are not written. The number laid out is
  bounded by the box (plus a small margin) and by 65,536.

#### 3.2.4 Comb (Comb flag, MaxLen > 0, Multiline and Password clear)

ISO §12.7.5.3 says Comb is meaningful only with MaxLen, and only when
Multiline, Password and FileSelect are clear.

- The box width (inside the border) is divided into MaxLen equal cells.
  Each character is centred in its cell, on a single baseline placed as in
  3.2.2. At most MaxLen characters are drawn (F9).
- When /MK /BC is present and the border width > 0, MaxLen − 1 vertical
  dividers are drawn between cells, in the border colour, inside the
  border. Draw at most 4,096 dividers.
- Auto size: the largest size at which each character's advance fits its
  cell and the glyph box fits the height.
- Comb with MaxLen absent or ≤ 0: a normal single-line field (3.2.2, normal
  padding). Comb with Multiline or Password set: that kind's layout.

### 3.3 Choice fields (§12.7.5.4)

#### 3.3.1 Combo box (Combo flag)

- Shown text: the display text of the first /Opt entry (among the first
  65,536) whose export value equals /V. Otherwise /V itself.
- Layout: as a single-line text field (3.2.2), auto size included. No
  drop-down arrow is drawn.

#### 3.3.2 List box

- Options from /TI (top index, default 0), in /Opt order, one option per
  line (F7). Options are read only as far as lines can show (bounded).
- Selected options: those whose export value is in /V (a text string, a
  name, or an array of up to 65,536 strings). Each selected option's line
  has a band behind it in a selection colour of the implementer's choice.
  The band spans the inner width, and text in the DA colour stays readable
  on it.
- Size: the DA size; when auto, a fixed readable size of the implementer's
  choice (not shrunk to fit). Line spacing as 3.2.3.

### 3.4 Buttons (§12.7.5.2)

#### 3.4.1 On-state name (check box and radio)

The on state is the first of:

1. the widget's own on state (`forms::on_state`: a key of /AP /N other than
   Off);
2. a key of /AP /D other than Off;
3. `on` (the `synthesize_with` argument);
4. the widget's /AS when it is a name other than Off;
5. the inherited /V when it is a name other than Off;
6. `Yes`.

#### 3.4.2 Check box

- Off: background and border (square) only.
- On: Off's drawing plus a mark in the DA colour (black when none). The mark
  is the /MK /CA character when /CA is exactly one character (encoded
  WinAnsi), shown in ZapfDingbats. Otherwise it is a check-mark glyph of
  ZapfDingbats of the implementer's choice. Acrobat's default "Check" style
  is the reference look.
- The mark is centred in the box and sized to it: its glyph bounds (from
  the ZapfDingbats AFM) lie inside the box less the border, and its height is
  at least 40% of the inner height.
- ZapfDingbats font: the first /DR /Font entry whose /BaseFont is
  ZapfDingbats and that has no /Encoding (`FontSource::Existing` under its
  name), else `FontSource::New(b"ZapfDingbats")` under the name `ZaDb`.

#### 3.4.3 Radio button (Radio flag)

- Background and border are circles inscribed in the box (the largest
  circle that fits, centred).
- On, with no /MK /CA or with /CA `l`: a filled dot centred in the
  circle, in the DA colour, its radius a fraction of the circle's chosen by
  the implementer (visibly inside the border). With another single /CA
  character: that ZapfDingbats glyph as in 3.4.2.

#### 3.4.4 Push button (Pushbutton flag)

- Normal: background fills the whole box, then the border, then the caption
  /MK /CA in the DA font and colour, one line, centred both ways, auto size
  (when the DA size is auto) fitting the box with a margin of the
  implementer's choice, at least 1 pt.
- Down (/AP /D): visibly different from normal (for example a darker
  background or the caption shifted), same caption. When /MK /AC is
  present, the down state may show it instead.
- No caption: no text is drawn and no font resource is needed.

### 3.5 Signature fields

`synthesize_with` returns `None`.

---

## 4. Variable text: encoding and layout (shared with spec 06)

### 4.1 Scripts

Each character's script comes from Unicode `Scripts.txt` (UAX #24),
regenerated into range tables. Do not type the ranges in.

| Script(s) | Class |
|---|---|
| Latin | Latin |
| Greek | Greek |
| Cyrillic | Cyrillic |
| Han, Bopomofo | Han |
| Hiragana, Katakana | Kana |
| Hangul | Hangul |
| everything else (Common, Inherited, other scripts) | Common |

### 4.2 Font for each character

For a character c whose class is not Common:

| Class | Font tried | then |
|---|---|---|
| Latin | the DA font if `win_ansi_code(c)` | `Eight::Latin` if `latin_extended` and it encodes c |
| Greek | `Eight::Greek` | — |
| Cyrillic | `Eight::Cyrillic` | — |
| Han | `Fonts::han` | — |
| Kana | `Cjk::Japan1` | — |
| Hangul | `Cjk::Korea1` | — |

Common characters (spaces, digits, punctuation, symbols) have no font of
their own. The implementer designs the rule; it must give these outcomes:

- A Common character inside or at the edge of a classified phrase stays in
  that phrase's run when the run's font encodes it, so spaces and
  punctuation do not split a Greek, Cyrillic or CJK phrase into several
  runs. Common characters that open a line join the line's first classified
  run on the same terms (`12 Ω` is one Greek run).
- In a CJK run every Common character goes to the CID font (its UTF-16 CMap
  encodes any character).
- Otherwise a Common character goes to some font that encodes it, the DA
  font when it can (`Привет €` draws `€` from the DA font; `a≤b` draws `≤`
  from a fallback font that has it).

A character no tried font encodes is drawn as `?` in the DA font, and the
result is lossy.

### 4.3 Characters before layout

Applied to the text first, in order:

1. **Ignorable:** characters with Unicode Default_Ignorable_Code_Point
   (zero-width spaces and joiners, variation selectors, the BOM, bidi
   controls, Hangul fillers, …), C1 controls (U+0080–U+009F), and U+2028,
   U+2029 are dropped. Exception: U+00AD (soft hyphen) is kept. It is
   WinAnsi 0xAD and is drawn.
2. **Composition:** a combining mark (U+0300–U+036F) after a base character
   is composed with it when Unicode canonical composition (from
   `UnicodeData.txt` decompositions, as NFC does) gives a character that
   some font of 4.2 encodes. Otherwise the mark stays and is `?` if
   nothing encodes it. NFD text from macOS file names (`e` + U+0301,
   `z` + U+0307, `s`/`t` + U+0326, Greek tonos and dialytika, Cyrillic
   breve and diaeresis) thus draws composed.
3. U+00A0 stays a no-break space (WinAnsi 0xA0).

### 4.4 Line breaking (`lines`)

- LF, CR and CRLF each end a line. CRLF is one break. An empty line between
  two breaks is an empty line.
- A line that would pass `max_width` (advances × size / 1000) breaks after
  its last space. The space stays at the end of that line (alignment ignores
  it). Spaces are U+0020.
- A word with no space that is wider than `max_width` alone breaks between
  characters, keeping at least one character per line (F3).
- `limit` bounds the number of lines returned.
- Advances: the DA font's `Metrics` for Latin slot glyphs,
  `Eight::widths(family)` for 8-bit fallbacks, the CID font's advance
  (section 5.4) for CJK.

### 4.5 Strings

Simple fonts: byte strings (`literal`). CID fonts: UTF-16BE hex strings
under the predefined UTF-16 CMap (§9.7.5.2). Characters above U+FFFF are
surrogate pairs.

---

## 5. Fallback fonts

### 5.1 What a viewer must get from them

For every fallback font written:

- **Text extraction** gives back the drawn characters. Use mupdf.js
  `toStructuredText` (and pdf.js where available) on the saved file. This
  works through glyph names in /Differences (AGL, §9.10.2) and through
  the predefined UTF-16 CMaps.
- **Rendering** in the app's view (hayro with the engine's URW and CJK
  fallback data) and in MuPDF draws the glyphs, not blanks or `?`.
- Advances used for layout equal the advances the font dictionary declares
  (/Widths, /DW, /W). Centred and right-aligned text then lands where the
  layout put it.

### 5.2 8-bit fonts (`Eight`)

- Greek: ISO 8859-7. Cyrillic: KOI8-U. Latin: the app's own extended-Latin
  layout (6.2).
- Codes below 0x80 are ASCII. Upper-half codes per section 6.
- `/Encoding << /BaseEncoding /WinAnsiEncoding /Differences [...] >>` names
  every drawable upper-half code with its glyph name (6.3). Codes that are
  not drawable are not named and are not encodable (`code` returns `None`).
- /FirstChar and /LastChar span the drawn codes. /Widths are `widths`
  rounded to integers.

### 5.3 CJK fonts (`Cjk`)

- Type0 font with the predefined UTF-16 horizontal CMap of its collection
  (ISO Table 116), with one CIDFontType0 descendant, not embedded.
- /BaseFont (F5): use names Acrobat's Asian font support and pdf.js map to
  a font of that collection. Candidates from Adobe's documentation of its
  non-embedded CJK fonts: `STSong-Light` (Adobe-GB1), `MSung-Light`
  (Adobe-CNS1), `HeiseiMin-W3` or `KozMinPro-Regular` (Adobe-Japan1),
  `HYSMyeongJo-Medium` (Adobe-Korea1). The implementer confirms each with
  mupdf.js text extraction and the app's view.
- /CIDSystemInfo: Registry `Adobe`, Ordering `GB1`, `CNS1`, `Japan1` or
  `Korea1`, and a Supplement at least the one the UTF-16 CMap declares in
  Adobe's `cmap-resources`.

### 5.4 CJK descriptor and widths

- Descriptor (ISO §9.8, Table 120): all required keys (/Type, /FontName =
  /BaseFont, /Flags with the Symbolic bit, /FontBBox, /ItalicAngle,
  /Ascent, /Descent, /CapHeight, /StemV). Values describe a generic CJK
  face on a 1000-unit em: Ascent > 0 > Descent, a /FontBBox that contains
  `[0 Descent 1000 Ascent]`. The values are the implementer's.
- /DW 1000 (full width) is the default. The implementer may give
  proportional or half-width widths (/W) for the collection's half-width
  ranges. The layout advances must match (5.1).

---

## 6. Tables (generated, never typed)

### 6.1 ISO 8859-7 and KOI8-U

Generate from the Unicode Consortium mapping files
`MAPPINGS/ISO8859/8859-7.TXT` and `MAPPINGS/VENDORS/MISC/KOI8-U.TXT`: code →
Unicode, upper half (0x80–0xFF). C1 control positions and unmapped codes
are unused.

### 6.2 Extended Latin

The upper half holds, in Unicode order, every letter of Latin Extended-A
(U+0100–U+017F) that WinAnsi lacks, then U+0218–U+021B (S and T with comma
below). The table is generated from that rule. It holds at most 128 codes.
Unused codes stay empty.

### 6.3 Glyph names and drawability

For each upper-half code with a Unicode value u:

1. Candidate names: all Adobe Glyph List names for u, plus `uniXXXX`.
2. The code is **drawable** when the URW Helvetica face
   (`BuiltinFace::Helvetica`) has a glyph under one of those names. That
   name is used in /Differences. When several match, use the AGLFN name
   if it is among them, else the first in AGL order.
3. Advances (`widths`): the family's face (`Helvetica`, `HelveticaBold`,
   `TimesRoman` via `builtin_metrics`) under that name. For Courier, 600.
   A face without the glyph gives the Helvetica advance.

Box-drawing and block characters of KOI8-U, which the URW faces lack, are
therefore not drawable. They are lossy unless another font encodes them.

### 6.4 WinAnsi

`win_ansi_code` is Annex D's WinAnsiEncoding through
`simple_encoding::SimpleEncoding::from_named(WinAnsi)`, plus U+00A0 → 0xA0
and U+00AD → 0xAD (Annex D names those codes `space` and `hyphen`, which
would otherwise map from U+0020 and U+002D).

---

## 7. Limits and safety

- No panics, only safe Rust. All reads go through the bounded readers of
  `forms/mod.rs`.
- Bounds: a value is drawn up to 65,536 characters; at most 65,536 lines;
  a combo box looks through 65,536 /Opt entries; a list box reads /V
  arrays up to 65,536 entries and options only as far as visible; comb
  dividers 4,096; the /DA bound of 3.1.1; a bounded number of /DA operands
  retained (the implementer's choice).
- `Context::work` charges each /DA, value and option read, so
  `forms::view` stops past its 64 MiB budget.
- Hostile geometry (zero, negative, 1e30 rects, /MaxLen 10^8, /DA size
  1e-30, operator soup) gives finite output with no NaN or inf in the
  content, quickly.

---

## 8. Experiment log (black-box, MuPDF 1.28.1 via npm `mupdf`)

Harness: `/Users/owner/Documents/tpdf-wt/scratch/spec-06-08/widget-synth.mjs`
(`tpdf-wt/tools/slot node widget-synth.mjs`). It builds a one-widget form,
sets the value with `setTextValue`/`setChoiceValue`, calls `update()`, and
prints the strings shown, the `Tf` sizes, non-black fills and the number of
fonts. Stream bytes and font names are not recorded.

| Input | MuPDF | This spec |
|---|---|---|
| Password field, value `secret` | shows `secret` | one `*` per character (3.2.1) |
| Combo /Opt [[e1 (Display One)] [e2 (Display Two)]], /V e2 | shows `e2` | shows `Display Two` (3.3.1) |
| List (one two three), /V two | all three, no selection fill | all three, band behind `two` (3.3.2) |
| Comb flag without MaxLen, `1234` | one plain string | plain single-line field (3.2.4) |
| Comb, MaxLen 6, value of 8 characters | positioned show of the value | 6 characters in 6 cells (F9) |
| /DA `/Helv -5 Tf` | `Tf` size −5 (mirrored text) | auto size (F6) |
| Multiline, /DA size 0, long text in a 100×40 box | drawn at 12 pt, 5+ lines overflowing the box | shrinks until it fits, down to the minimum (F2) |
| /Lang (en), value 中文 | a Japanese-collection CID font | simplified Chinese (F4) |
| /Lang (zh-SG), value 中文 | a traditional-Chinese-collection CID font | simplified Chinese (F4) |
| Multiline 60 pt wide, `Supercalifragilisticexpialidocious` | one line wider than the box | broken between characters (F3) |

---

## 9. Acceptance

### 9.1 Keep (black-box)

`engine/tests/app_forms.rs` (29 tests; 24 kept unchanged, names and
comments scrubbed per 9.5):

`the_listing_reads_every_kind_as_mupdf_does` (rename),
`inherited_attributes_hierarchies_and_radio_groups_across_pages`,
`values_read_names_strings_rich_text_streams_and_nothing_else`,
`fill_refusals_change_nothing`,
`add_writes_mupdfs_dictionaries_and_one_undo_step_each` (rename; asserts
dictionaries and states, not streams),
`a_shared_annots_array_is_copied_and_the_rect_follows_rotation_and_crop`,
`add_refusals_change_nothing`, `props_change_what_engine_ts_changes`,
`delete_removes_widgets_and_fields_and_undoes_exactly`,
`delete_drops_structure_references_to_the_widget`,
`hierarchies_signed_fields_and_scripts_are_refused`,
`the_view_draws_widgets_without_appearances_and_saves_never_hold_them`
(ink in each widget's region; no `SIGN` marker),
`need_appearances_redraws_every_widget_but_signatures`,
`hostile_fields_never_panic`, `deep_documents_open_edit_render_and_save`,
`full_page_rects_on_fractional_pages_are_on_the_page`,
`indirect_flags_keep_their_other_bits`, `field_name_walks_stay_bounded`,
`the_view_bounds_its_synthesis_work`,
`the_listing_cuts_long_strings_and_refuses_huge_pages`,
`helpers_work_on_a_builder`,
`a_lossy_text_leaves_every_other_widget_as_it_was` (Vietnamese, Hebrew,
arrow and emoji stay lossy with no /AP),
`a_button_fill_turns_on_only_widgets_with_that_state`,
`a_field_on_a_page_written_into_kids_can_be_deleted`.

Also kept: `tests/app_sign.rs` (signature widgets are not synthesized),
`tests/app_redact*.rs` uses of `refresh`, `tests/app_scan.rs` and
`tests/app_scan_text.rs` (the `Eight::Latin` tier; extraction round-trip),
and node `tests/p2-forms.test.mjs`, `tests/p3-forms.test.mjs`,
`tests/p5-sign-field.test.mjs`, `tests/p5-drive-field.test.mjs`,
`tests/p8-minors.test.mjs`, and `tests/rust-cjk-font.test.mjs` (field-set
`日本語` renders through the CJK fallback). Its hand-built CID-font fixtures
(lines 179–218) copy a MuPDF font descriptor. Replace those numbers with
neutral ones (scrub, 9.5). The test is about the view, not this
workstream.

### 9.2 Replace

The replacements below are written on the reviewer side **before hand-off**.
The old versions hold MuPDF streams, font dictionaries and resource names,
so they must not be in the implementer's tree, not even commented out.

| Test | MuPDF-specific part | Replacement |
|---|---|---|
| `app_forms.rs` `synthesis_matches_mupdfs_recorded_streams` | byte-equal content against MuPDF's recorded streams, with a list of deliberate differences | `synthesis_cases_meet_the_spec`: for each of the 102 cases in the new inputs file (9.4), build the PDF as `synthesis_pdf` does, synthesize, store it through `appearance_dict` into a copy, and check: (a) /BBox = `[0 0 w h]` with w and h swapped for /MK /R 90 and 270, and the /Matrix of 3.1.3; (b) every content stream parses with the engine's content lexer, with balanced `q`/`Q` and `BT`/`ET`, and `/Tx BMC … EMC` for text and choice; (c) lossy exactly for `tx_arrow`; (d) render the page through the view at scale 2: no ink outside /Rect (1 px), and ink inside it whenever a value, caption or on state is drawn; (e) mupdf.js structured text of the saved page inside the widget equals the expected shown text (the value; `*` × length for `tx_password`; display text for `ch_combo_pairs`; the first MaxLen characters for combs; whitespace normalised; lines joined); (f) for single-line cases, the extracted text's baseline is inside /Rect; (g) every font named in content is in /Resources and every resource is used. |
| `app_forms.rs` `fallback_fonts_match_mupdfs` | equality with MuPDF's font dictionaries (Differences, Widths, CID descriptor numbers) | `fallback_fonts_are_well_formed`: every 8-bit fallback font has /Type /Font /Subtype /Type1, /BaseFont of the family, /Encoding with /BaseEncoding /WinAnsiEncoding and /Differences whose names are all AGL names or `uniXXXX`, and /Widths of length LastChar − FirstChar + 1 that equal `Eight::widths(family)` rounded. Every CID font has Type0 → CIDFontType0 → descriptor with the required keys (5.4), the collection's UTF-16 CMap as /Encoding, matching Ordering, and Ascent > 0 > Descent. |
| `app_forms.rs` `fills_write_values_appearances_and_undo_exactly` | one assertion of an exact content stream | keep the rest; replace with: /AP /N content parses, wraps in `/Tx BMC … EMC`, names the DA font `Helv` at 12, and the saved page extracts `Typed text` inside the widget |
| `app_forms.rs` `fills_outside_win_ansi_use_fallback_fonts_or_no_appearance` | MuPDF's fallback resource names | for `Привет`, `Ελληνικά`, `日本語`, `Łódź`: a stored /AP whose /Resources /Font has the DA font plus one more font; extracted text equals the value; no `?`; the rest (lossy cases, no /NeedAppearances, undo) unchanged |
| `app_forms.rs` `cjk_runs_take_every_common_character` | exact runs under MuPDF's CID font names | not lossy for `日→𠀀。` and `カタカナー`, and each is drawn in a single CID-font run (one font, one show), extracted text equal; `Ўў` is lossy while the Cyrillic layout is KOI8-U (see Q5) |
| `app_accept_minors.rs` `latin_letters_winansi_lacks_store_a_widget_appearance` | an exact fallback font name | keep: no `?`, stored /AP, no /NeedAppearances, render draws each letter (not `?`), undo. Replace the name checks with: some font in /Resources has /Differences naming the letters' AGL glyph names (e.g. `zdotaccent`, `lslash`) |
| `src/app/forms/text.rs` unit tests (3): `numbers_print_as_mupdf_does`, `lines_break_after_spaces_and_at_line_ends`, `win_ansi_maps_latin_and_replaces_the_rest` | deleted with the file; the first asserts MuPDF's `%g` format, the second its long-word behaviour | new unit tests 9.3 items 1–3; the third is reproduced unchanged (`aé€日` → `[a, 0xE9, 0x80, ?]`) |

Tools and records (reviewer-side, not handed to the implementer):
`engine/tools/app-forms-oracle.mjs`, `engine/tools/app-forms-diff.mjs`,
`engine/reference/app-forms-20261015/{synthesis,summary}.json`. They compare
with MuPDF and are records, not gates. `differential.json` and
`sequences.json` (field listings and op replies, not appearances) stay
useful. Re-run them after landing for information.

### 9.3 New cases

1. Layout (`lines`):
   - `a\r\nb\rc` → `a`, `b`, `c`;
   - `a\n\nb` keeps the empty middle line;
   - wrap after the last space, keeping the space;
   - a 40-character word in a 56 pt line breaks into lines each ≤ 56 pt;
   - a limit of 7 returns 7 lines;
   - 65,536 characters cap.
2. `encode`:
   - `nbsp\u{a0}x shy\u{ad}x` stays in the DA font;
   - U+200B, U+FEFF, U+2028, U+0085, U+200D and U+FE0F are dropped;
   - `q\u{301}` → `q?` (lossy);
   - `12 Ω` → one Greek run including `12 `;
   - `a≤b` encodes `≤` in a fallback font, not lossy;
   - `Привет €` draws `€` in the DA font.
3. Tables:
   - `Eight::Cyrillic.code('Ж')` is some code ≥ 0x80 and `Eight::Greek.code('Ω')` likewise;
   - `Eight::Cyrillic.code('▀')` is None (not drawable);
   - every name in every /Differences maps back through the AGL to its
     Unicode value;
   - `Eight::Latin` covers all of U+0100–U+017F that WinAnsi lacks, plus
     U+0218–U+021B.
4. `han_font`:
   - `None` → Gb1, `zh` → Gb1, `zh-CN` → Gb1, `zh-SG` → Gb1;
   - `zh-TW`, `zh-HK`, `zh-Hant`, `zh-Hant-CN` → Cns1;
   - `ja`, `ja-JP` → Japan1; `ko`, `ko-KR` → Korea1;
   - `en`, `fr`, empty → Gb1;
   - case-insensitive.
5. Single line: for /Rect heights 8, 12, 24, 100 and sizes 0 (auto), 8,
   12, the rendered ink of `Hxg` lies within the clip. Auto size makes the
   ink fill at least 50% of the inner height for a short value in a tall
   box, and fit the width for a long value.
6. Multiline auto: a 300-character text in a 100 × 40 box → all text
   extracted, no ink outside /Rect, size ≥ the minimum.
7. Comb: MaxLen 6, `12345678` → extracted `123456`; each character's ink
   centred in its cell within 1 pt; 5 dividers with /MK /BC.
8. Quadding: left, centre and right ink positions differ, and the centre
   is centred within 1 pt.
9. /MK /R 90: extraction gives the value. The ink lies inside /Rect.
10. Check box: on-state ink inside the box, not in the Off form; /MK /CA
    `8` (cross) differs from the default; DA colour `1 0 0 rg` gives red
    mark pixels. Radio: circle and dot inside the box; /MK /CA `u` draws
    that glyph.
11. Push button: caption extracts; Down differs from Normal in pixels;
    /MK /BG fills the whole box.
12. List: /V array of two options → two bands; /TI 2 shows from the third
    option.
13. /BS /S /D draws a dashed border (gaps in the edge pixels); /S /U draws
    only the bottom edge.
14. Hostile: /DA `/Helv 1e-30 Tf`, `/Helv -5 Tf`, `Tf Tf rg /`; /MaxLen
    100000000; /Rect [1 1 2 2]: no panic, no NaN.

### 9.4 Case inputs file

Before hand-off, create `engine/tests/data/widget-cases.json` from
`engine/reference/app-forms-20261015/synthesis.json`. Keep only each case's
`name`, `widget` (the dictionary text, which the project wrote) and
`V.text` (the value). Leave out the recorded `N`, `D`, `states` and `fonts`
(MuPDF output). `app_forms.rs` reads the new file. The old file stays
reviewer-side.

### 9.5 Comments and names to scrub (surviving files, before hand-off)

- `engine/tests/app_forms.rs`: module doc (lines 2–4); comments at 1107,
  1118, 1122, 1162–1213 (the DIFFERENCES table and LOSSY/EMBEDDED
  constants go with the replaced test), 1325, 1419, 1427, 1463, 1649,
  1728, 1764, 1996, 2104, 2321, 2334; test names containing `mupdf`.
- `engine/tests/app_accept_minors.rs`: line 344 ("as MuPDF converts it",
  workstream 8's PNG case).
- `engine/tests/rust-cjk-font.test.mjs`: the fixture descriptor numbers.
- `engine/src/app/forms/write.rs`: doc of `refresh` ("with MuPDF's
  synthesis", "where MuPDF embeds a font program"), `base14_font` callers'
  comments.
- `engine/src/app/forms/view.rs`: module doc ("as MuPDF does").
- `engine/src/app/forms/mod.rs`, `engine/src/app/ops/fields.rs`: comments
  naming MuPDF functions or calling the synthesis "MuPDF's" (e.g. fields.rs
  line 8).
- `engine/src/app/ops/scan.rs` module doc ("as FreeText and widget
  appearances draw it" is fine; check for MuPDF references).

---

## 10. Open questions for the owner

- **Q1. Embed instead of naming glyphs?** Greek, Cyrillic and extended-Latin
  text relies on the viewer's substitute for a non-embedded standard font
  having those glyph names. That holds for MuPDF and the app's view (URW).
  It does not hold everywhere: pdf.js's standard-font data and some Acrobat
  substitutes may cover only Latin. The scanner already embeds subsets of the
  engine's URW outlines (OFL) with a /ToUnicode (`ops::scan`, `scan_fonts`).
  Reusing that for widget and FreeText fallbacks would make them render
  everywhere, at the cost of a `Sink` change (font streams) and larger
  files. Not in this spec.
- **Q2. CJK /BaseFont names.** F5 lists Adobe's names. If the owner prefers
  neutral names, Acrobat still renders through /CIDSystemInfo, but may
  pick a less fitting face.
- **Q3. Auto-size bounds.** Cap 12 pt and minimum 4 pt for multiline auto
  are suggestions. Acrobat's behaviour is similar but not documented.
- **Q4. /MK /AC for the down caption.** It is optional here.
- **Q5. Greek and Cyrillic code layouts.** ISO 8859-7 and KOI8-U are the
  layouts the old code shared with MuPDF. KOI8-U lacks Belarusian `Ў`,
  Serbian and Macedonian letters. A wider layout (for example Windows-1251,
  or a generated "letters WinAnsi lacks" layout like `Eight::Latin`) would
  make fewer texts lossy. The layout is invisible to users except through
  what is lossy. Keep KOI8-U, or let the implementer choose?

---

## Leak-check change log (2026-10-18, independent checker)

- 3.1.1 and 7: the /DA byte bound, size clamp and operand bound copied from
  the old code became implementer bounds with minimums.
- 3.1.2: the border-inset and clip-inset geometry restated as outcomes.
- 3.3.2: the list box's fixed auto size (the old code's value) became the
  implementer's choice.
- 4.2: the ordered font-trial algorithm for Common characters (context font
  from the previous character or, at a line start, the next one; then DA,
  Cyrillic, Greek) restated as required outcomes. That order mirrored the
  old code and MuPDF's run splitting step by step.
- 9.2: the replaced tests are rewritten reviewer-side before hand-off; the
  `Ўў` lossy expectation tied to KOI8-U; new question Q5.
- 9.5: scrub marked as a before-hand-off step.
