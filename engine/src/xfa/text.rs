// SPDX-License-Identifier: MIT
//! Text for the layout and the emitter: the base 14 face a font request
//! maps to, glyph advances from `builtin_metrics`, WinAnsi codes, and
//! word wrapping of a `Block` (paragraphs of styled runs) into positioned
//! lines. No PDF operators here: `emit` draws a `LaidBlock`.
//!
//!
//! Faces: every family maps to one of the base 14 faces (`face_for`).
//! Metrics: the core 14 AFM advances per WinAnsi code, cached per face on
//! first use, times the family's horizontal scale (`width_scale`: a
//! Calibri line measures and draws narrower than Helvetica); a character
//! WinAnsi lacks measures and shows as `?`.
//! Leading: `lineHeight` when set, else 1.2 × the size (Designer's).
//! Wrapping: greedy, at spaces, after hard hyphens and tabs, between
//! characters for a word wider than the line (`lay_out` has the rules).
//! Every loop is bounded by `limits::MAX_BLOCK_CHARS`.

use std::sync::OnceLock;

use super::limits::MAX_BLOCK_CHARS;
use super::types::{Font, HAlign, Para, Pt, Rgb, Value};

/// A base 14 face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Face {
    Helvetica,
    HelveticaBold,
    HelveticaOblique,
    HelveticaBoldOblique,
    Times,
    TimesBold,
    TimesItalic,
    TimesBoldItalic,
    Courier,
    CourierBold,
    CourierOblique,
    CourierBoldOblique,
    Symbol,
    ZapfDingbats,
}

impl Face {
    /// The /BaseFont name.
    pub fn base_font(self) -> &'static [u8] {
        match self {
            Face::Helvetica => b"Helvetica",
            Face::HelveticaBold => b"Helvetica-Bold",
            Face::HelveticaOblique => b"Helvetica-Oblique",
            Face::HelveticaBoldOblique => b"Helvetica-BoldOblique",
            Face::Times => b"Times-Roman",
            Face::TimesBold => b"Times-Bold",
            Face::TimesItalic => b"Times-Italic",
            Face::TimesBoldItalic => b"Times-BoldItalic",
            Face::Courier => b"Courier",
            Face::CourierBold => b"Courier-Bold",
            Face::CourierOblique => b"Courier-Oblique",
            Face::CourierBoldOblique => b"Courier-BoldOblique",
            Face::Symbol => b"Symbol",
            Face::ZapfDingbats => b"ZapfDingbats",
        }
    }
    /// The resource name the emitter registers the face under (`/XF1`...).
    pub fn resource_name(self) -> &'static [u8] {
        match self {
            Face::Helvetica => b"XF1",
            Face::HelveticaBold => b"XF2",
            Face::HelveticaOblique => b"XF3",
            Face::HelveticaBoldOblique => b"XF4",
            Face::Times => b"XF5",
            Face::TimesBold => b"XF6",
            Face::TimesItalic => b"XF7",
            Face::TimesBoldItalic => b"XF8",
            Face::Courier => b"XF9",
            Face::CourierBold => b"XF10",
            Face::CourierOblique => b"XF11",
            Face::CourierBoldOblique => b"XF12",
            Face::Symbol => b"XF13",
            Face::ZapfDingbats => b"XF14",
        }
    }
    pub const ALL: [Face; 14] = [
        Face::Helvetica,
        Face::HelveticaBold,
        Face::HelveticaOblique,
        Face::HelveticaBoldOblique,
        Face::Times,
        Face::TimesBold,
        Face::TimesItalic,
        Face::TimesBoldItalic,
        Face::Courier,
        Face::CourierBold,
        Face::CourierOblique,
        Face::CourierBoldOblique,
        Face::Symbol,
        Face::ZapfDingbats,
    ];
}

/// The base 14 face for a font request. The family is read as CSS
/// writes it: quotes stripped, the first of a comma list, case ignored.
///
/// | family contains | face |
/// |---|---|
/// | `symbol` (the whole name starts with it) | Symbol |
/// | `dingbats`, `wingdings`, `webdings` | ZapfDingbats |
/// | `courier`, `consolas`, `mono`, `lucida console`, `monaco`, `menlo`, `typewriter`, `letter gothic`, `ocr` | Courier |
/// | `times`, `georgia`, `minion`, `garamond`, `cambria`, `antiqua`, `palatino`, `palladio`, `bookman`, `baskerville`, `caslon`, `didot`, `bodoni`, `century schoolbook`, `constantia`, `serif` (and not `sans`) | Times |
/// | anything else (Arial, Calibri, Myriad Pro, Verdana, Tahoma, Segoe UI, Helvetica, ...) | Helvetica |
///
/// Courier, Times and Helvetica take bold and italic (oblique); Symbol
/// and ZapfDingbats have no styled faces.
pub fn face_for(font: &Font) -> Face {
    let family = normalized_family(&font.family);
    let has = |list: &[&str]| list.iter().any(|s| family.contains(s));
    if family.starts_with("symbol") {
        return Face::Symbol;
    }
    if has(&["dingbats", "wingdings", "webdings"]) {
        return Face::ZapfDingbats;
    }
    let mono = has(&[
        "courier",
        "consolas",
        "mono",
        "lucida console",
        "monaco",
        "menlo",
        "typewriter",
        "letter gothic",
        "ocr",
    ]);
    let serif = !family.contains("sans")
        && has(&[
            "times",
            "georgia",
            "minion",
            "garamond",
            "cambria",
            "antiqua",
            "palatino",
            "palladio",
            "bookman",
            "baskerville",
            "caslon",
            "didot",
            "bodoni",
            "century schoolbook",
            "constantia",
            "serif",
        ]);
    match (mono, serif, font.bold, font.italic) {
        (true, _, false, false) => Face::Courier,
        (true, _, true, false) => Face::CourierBold,
        (true, _, false, true) => Face::CourierOblique,
        (true, _, true, true) => Face::CourierBoldOblique,
        (false, true, false, false) => Face::Times,
        (false, true, true, false) => Face::TimesBold,
        (false, true, false, true) => Face::TimesItalic,
        (false, true, true, true) => Face::TimesBoldItalic,
        (false, false, false, false) => Face::Helvetica,
        (false, false, true, false) => Face::HelveticaBold,
        (false, false, false, true) => Face::HelveticaOblique,
        (false, false, true, true) => Face::HelveticaBoldOblique,
    }
}

/// The horizontal scale that brings the face `face_for` gives a family
/// to roughly the family's own widths: every advance is multiplied by it
/// for wrapping and measuring, and the text is drawn under `Tz` (and a
/// widget's /DA carries the same `Tz`), so drawn glyphs are as wide as
/// measured ones. 1.0 for any family not listed.
///
/// How the numbers were chosen: the average advance of a run of
/// ordinary English text (lowercase-heavy, with capitals, digits and
/// spaces) in each family, over the same in the substitute face; for
/// the sans families that is Arial (whose widths equal Helvetica's), for
/// Georgia and Cambria, Times. Rounded to two places; a line of body text
/// lands within a few percent of the real font, not glyph for glyph.
const WIDTH_SCALES: &[(&str, f64)] = &[
    ("calibri", 0.89),
    ("myriad", 0.94),
    ("segoe", 0.95),
    ("verdana", 1.12),
    ("tahoma", 1.02),
    ("georgia", 1.10),
    ("cambria", 1.03),
];

/// The family's horizontal scale (`WIDTH_SCALES`); 1.0 when not listed.
pub fn width_scale(font: &Font) -> f64 {
    let family = normalized_family(&font.family);
    WIDTH_SCALES
        .iter()
        .find(|(name, _)| family.contains(name))
        .map_or(1.0, |&(_, scale)| scale)
}

/// The family lowercased, quotes stripped, the first of a comma list.
fn normalized_family(family: &str) -> String {
    let first = family.split(',').next().unwrap_or("");
    first
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .trim()
        .to_ascii_lowercase()
}

/// The WinAnsi code of a character, `None` when WinAnsi lacks it (the
/// layout and the emitter draw `?` for such a character). Printable ASCII
/// is itself; 0x80 to 0xFF come from the crate's WinAnsiEncoding table
/// (ISO 32000-2 Annex D) through the glyph names' Unicode values, so
/// U+20AC is 0x80, U+2019 is 0x92, U+2022 is 0x95 (not the duplicate
/// bullet slots 0x7F, 0x81, 0x8D, 0x8F, 0x90, 0x9D). U+00A0 (no-break
/// space) is 0xA0 and U+00AD (soft hyphen) 0xAD, the slots Annex D gives
/// the space and hyphen glyphs again; U+2007 and U+202F (fixed-width
/// spaces) are 0xA0 too, and U+2010, U+2011 (hyphens) and U+2212 (minus)
/// are `-`. Control characters (tab included) have no code.
pub fn win_ansi_code(ch: char) -> Option<u8> {
    let c = u32::from(ch);
    if (0x20..0x7f).contains(&c) {
        return u8::try_from(c).ok();
    }
    if c < 0xa0 {
        return None;
    }
    let table = win_ansi_by_char();
    table
        .binary_search_by_key(&ch, |e| e.0)
        .ok()
        .map(|at| table[at].1)
}

/// The characters of WinAnsi codes 0x80 to 0xFF, sorted by character.
fn win_ansi_by_char() -> &'static [(char, u8)] {
    static TABLE: OnceLock<Vec<(char, u8)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        use crate::simple_encoding::{NamedEncoding, base_glyph_name, registry_unicode};
        const DUPLICATE_BULLETS: [u8; 6] = [0x7f, 0x81, 0x8d, 0x8f, 0x90, 0x9d];
        let mut out: Vec<(char, u8)> = vec![
            ('\u{a0}', 0xa0),
            ('\u{ad}', 0xad),
            ('\u{2007}', 0xa0),
            ('\u{202f}', 0xa0),
            ('\u{2010}', b'-'),
            ('\u{2011}', b'-'),
            ('\u{2212}', b'-'),
        ];
        for code in 0x80..=0xffu8 {
            if DUPLICATE_BULLETS.contains(&code) {
                continue;
            }
            let Some(name) = base_glyph_name(NamedEncoding::WinAnsi, code) else {
                continue;
            };
            if let Some(&[ch]) = registry_unicode(name) {
                // ASCII characters keep their own codes (0xA0 is
                // `space`, 0xAD `hyphen` in Annex D).
                if !ch.is_ascii() {
                    out.push((ch, code));
                }
            }
        }
        // The explicit entries come first, so a stable sort and dedup
        // keep them over a table slot for the same character.
        out.sort_by_key(|e| e.0);
        out.dedup_by_key(|e| e.0);
        out
    })
}

/// Per face, per WinAnsi code, the advance in 1/1000 em. The twelve text
/// faces come from the core 14 AFM advances `app::annots::fonts` carries
/// (the same data `builtin_metrics` has for Helvetica and Times-Roman,
/// and it also has the bold, italic and Courier faces). Symbol and
/// ZapfDingbats have no metrics in the crate: they measure as Helvetica.
fn advances() -> &'static [[f32; 256]; 14] {
    use crate::app::annots::fonts::Base14;
    static TABLE: OnceLock<[[f32; 256]; 14]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut out = [[0f32; 256]; 14];
        for (i, face) in Face::ALL.iter().enumerate() {
            let base = match face {
                Face::Helvetica | Face::Symbol | Face::ZapfDingbats => Base14::Helvetica,
                Face::HelveticaBold => Base14::HelveticaBold,
                Face::HelveticaOblique => Base14::HelveticaOblique,
                Face::HelveticaBoldOblique => Base14::HelveticaBoldOblique,
                Face::Times => Base14::TimesRoman,
                Face::TimesBold => Base14::TimesBold,
                Face::TimesItalic => Base14::TimesItalic,
                Face::TimesBoldItalic => Base14::TimesBoldItalic,
                Face::Courier => Base14::Courier,
                Face::CourierBold => Base14::CourierBold,
                Face::CourierOblique => Base14::CourierOblique,
                Face::CourierBoldOblique => Base14::CourierBoldOblique,
            };
            for (code, slot) in out[i].iter_mut().enumerate() {
                let byte = u8::try_from(code).unwrap_or(0);
                *slot = base.advance(byte) as f32;
            }
            // Annex D gives 0xA0 the space glyph: make sure it measures so.
            out[i][0xa0] = out[i][0x20];
        }
        out
    })
}

fn face_index(face: Face) -> usize {
    face as usize
}

/// The advance of `ch` in `face` at `size`, in points. A character
/// WinAnsi lacks measures as `?`.
pub fn advance(face: Face, ch: char, size: Pt) -> Pt {
    let code = win_ansi_code(ch).unwrap_or(b'?');
    f64::from(advances()[face_index(face)][usize::from(code)]) * size / 1000.0
}

/// The ascent and descent (positive) of `face` at `size`: the AFM
/// `Ascender` and `Descender` (Helvetica 718/207, Times-Roman 683/217,
/// Times-Bold 676/205, Courier 629/157, per 1000); Symbol and
/// ZapfDingbats, whose AFMs have none, as Helvetica.
pub fn ascent_descent(face: Face, size: Pt) -> (Pt, Pt) {
    let (a, d) = match face {
        Face::Helvetica
        | Face::HelveticaBold
        | Face::HelveticaOblique
        | Face::HelveticaBoldOblique
        | Face::Symbol
        | Face::ZapfDingbats => (718.0, 207.0),
        Face::Times | Face::TimesItalic | Face::TimesBoldItalic => (683.0, 217.0),
        Face::TimesBold => (676.0, 205.0),
        Face::Courier | Face::CourierBold | Face::CourierOblique | Face::CourierBoldOblique => {
            (629.0, 157.0)
        }
    };
    (a * size / 1000.0, d * size / 1000.0)
}

/// The line height of `font`: `para.line_height` when set, else 1.2 of
/// the size (Designer's default leading).
pub fn line_height(font: &Font, para: &Para) -> Pt {
    para.line_height.unwrap_or(font.size * 1.2)
}

/// A run of text in one style.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub font: Font,
}

/// A paragraph: runs and their paragraph settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Paragraph {
    pub runs: Vec<Run>,
    pub para: Para,
}

/// Paragraphs to lay out.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Block {
    pub paragraphs: Vec<Paragraph>,
}

impl Block {
    /// The block's text, paragraphs joined with newlines.
    pub fn plain(&self) -> String {
        let mut out = String::new();
        for (i, p) in self.paragraphs.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            for r in &p.runs {
                out.push_str(&r.text);
            }
        }
        out
    }
    pub fn is_empty(&self) -> bool {
        self.paragraphs
            .iter()
            .all(|p| p.runs.iter().all(|r| r.text.is_empty()))
    }
}

/// The block for a value: plain text as one paragraph per line (`\n`,
/// `\r\n`), rich text through `richtext::parse` (floating fields resolved
/// by `embed`).
pub fn block(
    value: &Value,
    font: &Font,
    para: &Para,
    embed: &dyn Fn(&str) -> Option<String>,
) -> Block {
    match value {
        // Designer writes U+2029 (and U+2028) for line breaks inside a
        // plain text value, as well as newlines.
        Value::Text(t) => Block {
            paragraphs: t
                .split(['\n', '\u{2028}', '\u{2029}'])
                .map(|line| Paragraph {
                    runs: vec![Run {
                        text: line.trim_end_matches('\r').to_string(),
                        font: font.clone(),
                    }],
                    para: para.clone(),
                })
                .collect(),
        },
        Value::Html(h) => super::richtext::parse(h, font, para, embed),
        Value::Empty | Value::Image(_) => Block::default(),
    }
}

/// A piece of a laid-out line: one run's characters at one position.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// From the block's left edge, after alignment.
    pub x: Pt,
    pub width: Pt,
    pub face: Face,
    pub size: Pt,
    pub color: Rgb,
    pub underline: bool,
    pub line_through: bool,
    pub letter_spacing: Pt,
    /// The family's horizontal scale (`width_scale`), drawn as `Tz`.
    pub scale: f64,
    /// The characters shown (`?` for any WinAnsi lacks).
    pub text: String,
}

/// A laid-out line.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// The baseline's distance from the block's top.
    pub baseline: Pt,
    /// The line's advance width (sum of its pieces).
    pub width: Pt,
    pub pieces: Vec<Piece>,
}

/// A block laid out into lines.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LaidBlock {
    pub lines: Vec<Line>,
    /// The widest line's extent from the block's left edge: its
    /// paragraph's left margin, the first line's indent, the line's width
    /// and the right margin (no alignment shift). With zero margins, the
    /// widest line's width.
    pub width: Pt,
    /// The height from the block's top to the last line's descent plus
    /// paragraph spacing.
    pub height: Pt,
}

/// Lays out `block` within `width` (`None`: unbounded, one line per
/// paragraph). Wraps at spaces, breaks a word longer than the width
/// between characters, honours `HAlign`, paragraph margins, indent and
/// spacing; a character WinAnsi lacks is shown as `?`. Stops after
/// `limits::MAX_BLOCK_CHARS` characters.
///
/// The rules, per paragraph:
///
/// - The available width is `width - margin_left - margin_right`, and
///   for the first line also minus `text_indent`. A negative indent
///   (hanging) moves the first line left of the margin but never left of
///   the block: it is at least `-margin_left`. A width that is NaN or
///   negative is zero (every character then sits on its own line).
/// - Advances are the face's times the run's `width_scale`.
/// - A line breaks at a space (spaces at the break are dropped: neither
///   drawn nor counted), after a hard hyphen between two characters, and
///   after a tab. A word wider than the line breaks between characters
///   (at least one character per line). No-break spaces that end a word
///   (U+00A0 before a space or the paragraph's end) hang: they are drawn
///   but do not count when deciding whether the word fits.
/// - A tab advances to the next multiple of four spaces' width of its
///   run, measured from the paragraph's left margin; it ends a piece.
/// - `letter_spacing` is added after every character.
/// - `Center` and `Right` shift a line within the available width when
///   `width` is `Some`; `Justify` is laid out as `Left`.
/// - `space_above` comes before the paragraph, `space_below` after it. A
///   paragraph with no text still takes one line.
/// - A line's height is `para.line_height` when set, else the largest
///   1.2 × size among the runs on it; its baseline is its top plus the
///   largest ascent among those runs (also when the line height is less
///   than ascent plus descent).
/// - Control characters other than tab are dropped.
pub fn lay_out(block: &Block, width: Option<Pt>) -> LaidBlock {
    let bound = match width {
        None => f64::INFINITY,
        Some(w) if w.is_nan() || w <= 0.0 => 0.0,
        Some(w) => w,
    };
    let mut out = LaidBlock::default();
    let mut budget = MAX_BLOCK_CHARS;
    let mut y: Pt = 0.0;
    for paragraph in &block.paragraphs {
        if budget == 0 {
            break;
        }
        budget -= 1;
        lay_out_paragraph(
            paragraph,
            bound,
            width.is_some(),
            &mut budget,
            &mut y,
            &mut out,
        );
    }
    out.height = y;
    out
}

/// A finite, non-negative number, else zero.
fn sane(v: Pt) -> Pt {
    if v.is_finite() && v > 0.0 { v } else { 0.0 }
}

/// A finite number, else zero (indents may be negative).
fn finite(v: Pt) -> Pt {
    if v.is_finite() { v } else { 0.0 }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Normal,
    Space,
    Tab,
    Hyphen,
}

/// One character to place.
struct Glyph {
    shown: char,
    run: usize,
    /// Advance plus letter spacing (tabs: zero, measured on placement).
    adv: Pt,
    kind: Kind,
}

/// A run's measuring settings.
struct RunStyle {
    face: Face,
    size: Pt,
    letter_spacing: Pt,
    /// The family's horizontal scale.
    scale: f64,
    /// A tab stop's interval: four spaces.
    tab: Pt,
    color: Rgb,
    underline: bool,
    line_through: bool,
}

/// A placed glyph: its index and its x from the line's origin.
type Placed = (usize, Pt);

struct LineState<'a> {
    styles: &'a [RunStyle],
    glyphs: &'a [Glyph],
    para: &'a Para,
    bound: Pt,
    aligned: bool,
    first: bool,
    placed: Vec<Placed>,
    x: Pt,
}

impl LineState<'_> {
    /// The offset of the current line's origin from the margin: a
    /// negative first-line indent stops at the block's edge.
    fn indent(&self) -> Pt {
        if self.first {
            finite(self.para.text_indent).max(-sane(self.para.margin_left))
        } else {
            0.0
        }
    }
    fn avail(&self) -> Pt {
        let v =
            self.bound - sane(self.para.margin_left) - sane(self.para.margin_right) - self.indent();
        if v.is_nan() || v < 0.0 { 0.0 } else { v }
    }
    /// The width of glyph `i` placed at `x`.
    fn width_at(&self, i: usize, x: Pt) -> Pt {
        let g = &self.glyphs[i];
        if g.kind != Kind::Tab {
            return g.adv;
        }
        let stop = self.styles[g.run].tab;
        if stop <= 0.0 {
            return 0.0;
        }
        let at = self.indent() + x;
        let next = ((at / stop) + 1e-9).floor() + 1.0;
        (next * stop - at).max(0.0)
    }
    fn range_width(&self, range: std::ops::Range<usize>, x: Pt) -> Pt {
        let mut at = x;
        for i in range {
            at += self.width_at(i, at);
        }
        at - x
    }
    fn place(&mut self, i: usize) {
        let w = self.width_at(i, self.x);
        self.placed.push((i, self.x));
        self.x += w;
    }
}

const EPS: Pt = 1e-6;

fn lay_out_paragraph(
    paragraph: &Paragraph,
    bound: Pt,
    aligned: bool,
    budget: &mut usize,
    y: &mut Pt,
    out: &mut LaidBlock,
) {
    let para = &paragraph.para;
    let styles: Vec<RunStyle> = paragraph
        .runs
        .iter()
        .map(|r| {
            let face = face_for(&r.font);
            let size = sane(r.font.size);
            let letter_spacing = finite(r.font.letter_spacing);
            let scale = width_scale(&r.font);
            RunStyle {
                face,
                size,
                letter_spacing,
                scale,
                tab: 4.0 * (advance(face, ' ', size) * scale + letter_spacing),
                color: r.font.color,
                underline: r.font.underline,
                line_through: r.font.line_through,
            }
        })
        .collect();
    let mut glyphs = Vec::new();
    'runs: for (run, r) in paragraph.runs.iter().enumerate() {
        let st = &styles[run];
        for ch in r.text.chars() {
            if *budget == 0 {
                break 'runs;
            }
            let kind = match ch {
                ' ' | '\u{2028}' | '\u{2029}' => Kind::Space,
                '\t' => Kind::Tab,
                '-' => Kind::Hyphen,
                c if c.is_control() => continue,
                _ => Kind::Normal,
            };
            *budget -= 1;
            let shown = if kind == Kind::Space {
                ' '
            } else if kind == Kind::Tab || win_ansi_code(ch).is_some() {
                ch
            } else {
                '?'
            };
            let adv = if kind == Kind::Tab {
                0.0
            } else {
                advance(st.face, shown, st.size) * st.scale + st.letter_spacing
            };
            glyphs.push(Glyph {
                shown,
                run,
                adv,
                kind,
            });
        }
    }

    *y += sane(para.space_above);
    let mut state = LineState {
        styles: &styles,
        glyphs: &glyphs,
        para,
        bound,
        aligned,
        first: true,
        placed: Vec::new(),
        x: 0.0,
    };
    // Trailing spaces of the last unit placed: drawn only when more
    // content follows on the same line.
    let mut pending = 0..0;
    let mut i = 0;
    let n = glyphs.len();
    while i < n {
        // A unit: content (no spaces; ends after a tab or a hard hyphen
        // between characters) then its trailing spaces.
        let cs = i;
        while i < n && glyphs[i].kind != Kind::Space {
            let kind = glyphs[i].kind;
            if kind == Kind::Tab {
                if i == cs {
                    i += 1;
                }
                break;
            }
            i += 1;
            if kind == Kind::Hyphen
                && i - 1 > cs
                && i < n
                && glyphs[i].kind == Kind::Normal
                && glyphs[i - 2].kind == Kind::Normal
            {
                break;
            }
        }
        let ce = i;
        while i < n && glyphs[i].kind == Kind::Space {
            i += 1;
        }
        let se = i;
        // Leading spaces (no content before them on the line) are drawn.
        let (content, trailing) = if ce > cs {
            (cs..ce, ce..se)
        } else {
            (cs..se, se..se)
        };
        if content.is_empty() {
            continue;
        }
        // No-break spaces ending the content hang when a break may follow.
        let mut fit_end = content.end;
        if se > ce || se == n {
            while fit_end > content.start && glyphs[fit_end - 1].shown == '\u{a0}' {
                fit_end -= 1;
            }
        }
        // At most two passes: once on the current line, once on a new one.
        for _ in 0..2 {
            let has_content = !state.placed.is_empty();
            let pw = state.range_width(pending.clone(), state.x);
            let w = state.range_width(content.start..fit_end, state.x + pw);
            let avail = state.avail();
            if has_content && state.x + pw + w > avail + EPS {
                flush_line(&mut state, y, out);
                pending = 0..0;
                continue;
            }
            for p in pending.clone() {
                state.place(p);
            }
            if !has_content && w > avail + EPS {
                // Too long for an empty line: break between characters.
                for g in content.clone() {
                    let gw = state.width_at(g, state.x);
                    if !state.placed.is_empty() && state.x + gw > state.avail() + EPS {
                        flush_line(&mut state, y, out);
                    }
                    state.place(g);
                }
            } else {
                for g in content.clone() {
                    state.place(g);
                }
            }
            break;
        }
        pending = trailing;
    }
    // The last line; an empty paragraph still takes one.
    if !state.placed.is_empty() || state.first {
        flush_line(&mut state, y, out);
    }
    *y += sane(para.space_below);
}

/// Ends the current line: pieces, alignment, height and baseline.
fn flush_line(state: &mut LineState<'_>, y: &mut Pt, out: &mut LaidBlock) {
    let placed = std::mem::take(&mut state.placed);
    let styles = state.styles;
    let glyphs = state.glyphs;
    let para = state.para;
    let line_width = state.x;
    let origin = sane(para.margin_left) + state.indent();
    let shift = if state.aligned && state.bound.is_finite() {
        let room = (state.avail() - line_width).max(0.0);
        match para.h_align {
            HAlign::Left | HAlign::Justify => 0.0,
            HAlign::Center => room / 2.0,
            HAlign::Right => room,
        }
    } else {
        0.0
    };

    // Height and ascent: the runs on the line, or (an empty line) every
    // run of the paragraph, or the default font.
    let mut runs: Vec<usize> = placed.iter().map(|&(g, _)| glyphs[g].run).collect();
    runs.dedup();
    if runs.is_empty() {
        runs = (0..styles.len()).collect();
    }
    let (mut tallest, mut ascent): (Pt, Pt) = (0.0, 0.0);
    if runs.is_empty() {
        let font = Font::default();
        tallest = font.size * 1.2;
        ascent = ascent_descent(face_for(&font), font.size).0;
    }
    for &r in &runs {
        let st = &styles[r];
        tallest = tallest.max(st.size * 1.2);
        ascent = ascent.max(ascent_descent(st.face, st.size).0);
    }
    let height = match para.line_height {
        Some(h) if h.is_finite() => h.max(0.0),
        _ => tallest,
    };

    let mut pieces: Vec<Piece> = Vec::new();
    let mut joinable = false;
    for &(g, x) in &placed {
        let glyph = &glyphs[g];
        if glyph.kind == Kind::Tab {
            joinable = false;
            continue;
        }
        let st = &styles[glyph.run];
        let px = origin + shift + x;
        if let Some(last) = pieces.last_mut().filter(|p| {
            joinable
                && p.face == st.face
                && p.size == st.size
                && p.color == st.color
                && p.underline == st.underline
                && p.line_through == st.line_through
                && p.letter_spacing == st.letter_spacing
                && p.scale == st.scale
        }) {
            last.text.push(glyph.shown);
            last.width = px + glyph.adv - last.x;
        } else {
            pieces.push(Piece {
                x: px,
                width: glyph.adv,
                face: st.face,
                size: st.size,
                color: st.color,
                underline: st.underline,
                line_through: st.line_through,
                letter_spacing: st.letter_spacing,
                scale: st.scale,
                text: glyph.shown.to_string(),
            });
        }
        joinable = true;
    }
    out.width = out.width.max(origin + line_width + sane(para.margin_right));
    out.lines.push(Line {
        baseline: *y + ascent,
        width: line_width,
        pieces,
    });
    *y += height;
    state.first = false;
    state.x = 0.0;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font(size: Pt) -> Font {
        Font {
            family: "Helvetica".into(),
            size,
            ..Font::default()
        }
    }

    fn para() -> Para {
        Para::default()
    }

    fn one(text: &str, f: Font, p: Para) -> Block {
        Block {
            paragraphs: vec![Paragraph {
                runs: vec![Run {
                    text: text.into(),
                    font: f,
                }],
                para: p,
            }],
        }
    }

    fn width(text: &str, size: Pt) -> Pt {
        text.chars()
            .map(|c| advance(Face::Helvetica, c, size))
            .sum()
    }

    fn line_text(line: &Line) -> String {
        line.pieces.iter().map(|p| p.text.as_str()).collect()
    }

    fn close(a: Pt, b: Pt) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn helvetica_advances_match_the_afm() {
        assert!(close(advance(Face::Helvetica, 'H', 10.0), 7.22));
        assert!(close(advance(Face::Helvetica, ' ', 10.0), 2.78));
        assert!(close(advance(Face::Helvetica, '?', 10.0), 5.56));
        assert!(close(advance(Face::Helvetica, '\u{20ac}', 10.0), 5.56));
        assert!(close(advance(Face::Helvetica, '\u{a0}', 10.0), 2.78));
        assert!(close(advance(Face::Helvetica, 'i', 1000.0), 222.0));
        assert!(close(advance(Face::HelveticaBold, 'i', 1000.0), 278.0));
        assert!(close(advance(Face::Times, 'H', 1000.0), 722.0));
        assert!(close(advance(Face::TimesBold, 'a', 1000.0), 500.0));
        assert!(close(advance(Face::Courier, 'i', 1000.0), 600.0));
        assert!(close(advance(Face::CourierBoldOblique, 'W', 1000.0), 600.0));
        // A character WinAnsi lacks measures as `?`.
        assert!(close(advance(Face::Helvetica, '\u{4e2d}', 10.0), 5.56));
        assert!(close(
            advance(Face::Times, '\u{141}', 10.0),
            advance(Face::Times, '?', 10.0)
        ));
    }

    #[test]
    fn win_ansi_codes_follow_the_table() {
        assert_eq!(win_ansi_code('A'), Some(0x41));
        assert_eq!(win_ansi_code('~'), Some(0x7e));
        assert_eq!(win_ansi_code('\u{20ac}'), Some(0x80));
        assert_eq!(win_ansi_code('\u{201a}'), Some(0x82));
        assert_eq!(win_ansi_code('\u{2026}'), Some(0x85));
        assert_eq!(win_ansi_code('\u{2018}'), Some(0x91));
        assert_eq!(win_ansi_code('\u{2019}'), Some(0x92));
        assert_eq!(win_ansi_code('\u{201c}'), Some(0x93));
        assert_eq!(win_ansi_code('\u{2022}'), Some(0x95));
        assert_eq!(win_ansi_code('\u{2013}'), Some(0x96));
        assert_eq!(win_ansi_code('\u{2014}'), Some(0x97));
        assert_eq!(win_ansi_code('\u{2122}'), Some(0x99));
        assert_eq!(win_ansi_code('\u{17d}'), Some(0x8e));
        assert_eq!(win_ansi_code('\u{a0}'), Some(0xa0));
        assert_eq!(win_ansi_code('\u{ad}'), Some(0xad));
        assert_eq!(win_ansi_code('\u{e9}'), Some(0xe9));
        assert_eq!(win_ansi_code('\u{ff}'), Some(0xff));
        assert_eq!(win_ansi_code('\u{2011}'), Some(b'-'));
        assert_eq!(win_ansi_code('\t'), None);
        assert_eq!(win_ansi_code('\u{7f}'), None);
        assert_eq!(win_ansi_code('\u{141}'), None);
        assert_eq!(win_ansi_code('\u{4e2d}'), None);
    }

    #[test]
    fn ascent_and_descent_are_the_afm_values() {
        assert_eq!(ascent_descent(Face::Helvetica, 10.0), (7.18, 2.07));
        assert_eq!(ascent_descent(Face::HelveticaBold, 1000.0), (718.0, 207.0));
        assert_eq!(ascent_descent(Face::Times, 1000.0), (683.0, 217.0));
        assert_eq!(ascent_descent(Face::TimesBold, 1000.0), (676.0, 205.0));
        assert_eq!(ascent_descent(Face::Courier, 1000.0), (629.0, 157.0));
        assert_eq!(line_height(&font(10.0), &para()), 12.0);
        assert_eq!(
            line_height(
                &font(10.0),
                &Para {
                    line_height: Some(15.0),
                    ..para()
                }
            ),
            15.0
        );
    }

    #[test]
    fn families_map_to_base14_faces() {
        let f = |family: &str, bold: bool, italic: bool| {
            face_for(&Font {
                family: family.into(),
                bold,
                italic,
                ..Font::default()
            })
        };
        for sans in [
            "Arial",
            "Calibri",
            "Myriad Pro",
            "Verdana",
            "Tahoma",
            "Segoe UI",
            "Helvetica",
            "Microsoft Sans Serif",
            "'Arial', sans-serif",
            "",
        ] {
            assert_eq!(f(sans, false, false), Face::Helvetica, "{sans}");
        }
        for serif in [
            "Times New Roman",
            "Georgia",
            "Cambria",
            "Minion Pro",
            "Garamond",
            "Book Antiqua",
            "\"Times\"",
        ] {
            assert_eq!(f(serif, false, false), Face::Times, "{serif}");
        }
        for mono in ["Courier New", "Consolas", "Courier", "Lucida Console"] {
            assert_eq!(f(mono, false, false), Face::Courier, "{mono}");
        }
        assert_eq!(f("Symbol", true, false), Face::Symbol);
        assert_eq!(f("ZapfDingbats", false, false), Face::ZapfDingbats);
        assert_eq!(f("Wingdings", false, false), Face::ZapfDingbats);
        assert_eq!(f("Arial", true, false), Face::HelveticaBold);
        assert_eq!(f("Arial", false, true), Face::HelveticaOblique);
        assert_eq!(f("Arial", true, true), Face::HelveticaBoldOblique);
        assert_eq!(f("Times New Roman", true, false), Face::TimesBold);
        assert_eq!(f("Georgia", false, true), Face::TimesItalic);
        assert_eq!(f("Georgia", true, true), Face::TimesBoldItalic);
        assert_eq!(f("Courier New", true, false), Face::CourierBold);
        assert_eq!(f("Consolas", false, true), Face::CourierOblique);
        assert_eq!(f("Consolas", true, true), Face::CourierBoldOblique);
    }

    #[test]
    fn wraps_at_spaces_and_drops_the_break_spaces() {
        let word = width("aaa", 10.0);
        let space = width(" ", 10.0);
        let laid = lay_out(
            &one("aaa bbb   ccc", font(10.0), para()),
            Some(2.0 * word + space + 0.01),
        );
        assert_eq!(laid.lines.len(), 2);
        assert_eq!(line_text(&laid.lines[0]), "aaa bbb");
        assert_eq!(line_text(&laid.lines[1]), "ccc");
        assert!(close(laid.lines[0].width, 2.0 * word + space));
        assert!(close(laid.lines[1].width, width("ccc", 10.0)));
        assert_eq!(laid.lines[0].pieces.len(), 1);
        assert!(close(laid.lines[1].pieces[0].x, 0.0));
        assert!(close(laid.width, 2.0 * word + space));
        assert!(close(laid.height, 24.0));
        assert!(close(laid.lines[0].baseline, 7.18));
        assert!(close(laid.lines[1].baseline, 12.0 + 7.18));
    }

    #[test]
    fn a_word_longer_than_the_line_breaks_between_characters() {
        let n = width("n", 10.0);
        let laid = lay_out(&one("nnnnnnnnnn", font(10.0), para()), Some(3.5 * n));
        let texts: Vec<String> = laid.lines.iter().map(line_text).collect();
        assert_eq!(texts, ["nnn", "nnn", "nnn", "n"]);
        // A long word after a short one starts on its own line.
        let laid = lay_out(&one("n nnnnnnnn", font(10.0), para()), Some(3.5 * n));
        let texts: Vec<String> = laid.lines.iter().map(line_text).collect();
        assert_eq!(texts, ["n", "nnn", "nnn", "nn"]);
    }

    #[test]
    fn breaks_after_a_hard_hyphen() {
        let fits = width("known", 10.0).max(width("well-", 10.0)) + 0.01;
        assert!(fits < width("well-known", 10.0));
        let laid = lay_out(&one("well-known", font(10.0), para()), Some(fits));
        let texts: Vec<String> = laid.lines.iter().map(line_text).collect();
        assert_eq!(texts, ["well-", "known"]);
        // Not after a leading hyphen.
        let laid = lay_out(
            &one("-5 x", font(10.0), para()),
            Some(width("-5", 10.0) + 0.01),
        );
        assert_eq!(line_text(&laid.lines[0]), "-5");
    }

    #[test]
    fn tabs_advance_to_four_space_stops() {
        let stop = 4.0 * width(" ", 10.0);
        let laid = lay_out(&one("a\tb", font(10.0), para()), None);
        let line = &laid.lines[0];
        assert_eq!(line.pieces.len(), 2);
        assert_eq!(line.pieces[0].text, "a");
        assert_eq!(line.pieces[1].text, "b");
        assert!(close(line.pieces[1].x, stop));
        assert!(close(line.width, stop + width("b", 10.0)));
        // A tab right on a stop goes to the next one.
        let laid = lay_out(&one("\t\tb", font(10.0), para()), None);
        assert!(close(laid.lines[0].pieces[0].x, 2.0 * stop));
    }

    #[test]
    fn letter_spacing_follows_every_character() {
        let f = Font {
            letter_spacing: 1.0,
            ..font(10.0)
        };
        let laid = lay_out(&one("ab", f, para()), None);
        let piece = &laid.lines[0].pieces[0];
        assert!(close(piece.width, width("ab", 10.0) + 2.0));
        assert_eq!(piece.letter_spacing, 1.0);
    }

    #[test]
    fn center_and_right_shift_within_the_margins() {
        let w = width("ab", 10.0);
        let p = Para {
            h_align: HAlign::Center,
            ..para()
        };
        let laid = lay_out(&one("ab", font(10.0), p), Some(100.0));
        assert!(close(laid.lines[0].pieces[0].x, (100.0 - w) / 2.0));
        assert!(close(laid.width, w), "alignment is not in the width");
        let p = Para {
            h_align: HAlign::Right,
            margin_left: 10.0,
            margin_right: 20.0,
            ..para()
        };
        let laid = lay_out(&one("ab", font(10.0), p), Some(100.0));
        assert!(close(laid.lines[0].pieces[0].x, 10.0 + 70.0 - w));
        let p = Para {
            h_align: HAlign::Justify,
            ..para()
        };
        let laid = lay_out(&one("ab", font(10.0), p), Some(100.0));
        assert!(close(laid.lines[0].pieces[0].x, 0.0));
        // No width: no shift.
        let p = Para {
            h_align: HAlign::Right,
            ..para()
        };
        let laid = lay_out(&one("ab", font(10.0), p), None);
        assert!(close(laid.lines[0].pieces[0].x, 0.0));
    }

    #[test]
    fn paragraphs_take_spacing_margins_and_indent() {
        let p = Para {
            space_above: 5.0,
            space_below: 3.0,
            margin_left: 2.0,
            text_indent: 7.0,
            ..para()
        };
        let para_of = |t: &str| Paragraph {
            runs: vec![Run {
                text: t.into(),
                font: font(10.0),
            }],
            para: p.clone(),
        };
        let block = Block {
            paragraphs: vec![para_of("aaa bbb"), para_of("c")],
        };
        let word = width("aaa", 10.0);
        // The first line has the indent less room: "bbb" wraps.
        let laid = lay_out(&block, Some(2.0 + 7.0 + word + 1.0));
        let texts: Vec<String> = laid.lines.iter().map(line_text).collect();
        assert_eq!(texts, ["aaa", "bbb", "c"]);
        assert!(close(laid.lines[0].pieces[0].x, 9.0));
        assert!(close(laid.lines[1].pieces[0].x, 2.0));
        assert!(close(laid.lines[2].pieces[0].x, 9.0));
        assert!(close(laid.lines[0].baseline, 5.0 + 7.18));
        assert!(close(laid.lines[1].baseline, 17.0 + 7.18));
        assert!(close(laid.lines[2].baseline, 5.0 + 24.0 + 3.0 + 5.0 + 7.18));
        assert!(close(laid.height, 5.0 + 24.0 + 3.0 + 5.0 + 12.0 + 3.0));
        assert!(close(laid.width, 9.0 + word));
    }

    #[test]
    fn a_negative_indent_stops_at_the_block_edge() {
        // Designer's textIndent -14.4pt with no left margin: the first
        // line starts at the box edge, with the whole width.
        let p = Para {
            text_indent: -14.4,
            ..para()
        };
        let text = "(date of the most recent notice)";
        let w = width(text, 12.0);
        let laid = lay_out(&one(text, font(12.0), p.clone()), Some(w + 1.0));
        assert_eq!(laid.lines.len(), 1);
        assert!(close(laid.lines[0].pieces[0].x, 0.0));
        // With a margin it hangs, but only as far as the margin.
        let hanging = Para {
            margin_left: 18.0,
            text_indent: -18.0,
            ..para()
        };
        let laid = lay_out(&one("aaa bbb", font(10.0), hanging), Some(18.0 + 1.0));
        assert!(close(laid.lines[0].pieces[0].x, 0.0));
        assert!(close(laid.lines[1].pieces[0].x, 18.0));
        let deeper = Para {
            margin_left: 5.0,
            text_indent: -18.0,
            ..para()
        };
        let laid = lay_out(&one("ab", font(10.0), deeper), Some(100.0));
        assert!(close(laid.lines[0].pieces[0].x, 0.0));
    }

    #[test]
    fn a_no_break_space_ending_a_word_hangs() {
        // Designer writes the space after "before" as U+00A0: the words
        // fit without it, so they stay on one line, the space drawn.
        let text = "conducted before\u{a0}";
        let words = width("conducted before", 12.0);
        let laid = lay_out(&one(text, font(12.0), para()), Some(words + 0.01));
        assert_eq!(laid.lines.len(), 1);
        assert_eq!(line_text(&laid.lines[0]), text);
        // Inside a word it still counts, and the word wraps whole.
        let laid = lay_out(
            &one("conducted be\u{a0}fore", font(12.0), para()),
            Some(words + 0.01),
        );
        assert_eq!(laid.lines.len(), 2);
        assert_eq!(line_text(&laid.lines[1]), "be\u{a0}fore");
    }

    #[test]
    fn narrow_families_measure_and_draw_narrower() {
        let calibri = Font {
            family: "Calibri".into(),
            size: 10.0,
            ..Font::default()
        };
        assert_eq!(face_for(&calibri), Face::Helvetica);
        assert!(close(width_scale(&calibri), 0.89));
        assert!(close(width_scale(&font(10.0)), 1.0));
        assert!(close(
            width_scale(&Font {
                family: "'Myriad Pro', sans-serif".into(),
                ..Font::default()
            }),
            0.94
        ));
        let text = "The quick brown fox";
        let helvetica = lay_out(&one(text, font(10.0), para()), None);
        let narrow = lay_out(&one(text, calibri.clone(), para()), None);
        assert!(close(narrow.width, helvetica.width * 0.89));
        assert!(close(narrow.lines[0].pieces[0].scale, 0.89));
        assert!(close(helvetica.lines[0].pieces[0].scale, 1.0));
        // A width that wraps Helvetica holds the Calibri line.
        let bound = helvetica.width * 0.95;
        assert_eq!(
            lay_out(&one(text, font(10.0), para()), Some(bound))
                .lines
                .len(),
            2
        );
        assert_eq!(
            lay_out(&one(text, calibri, para()), Some(bound))
                .lines
                .len(),
            1
        );
    }

    #[test]
    fn unbounded_width_keeps_one_line_per_paragraph() {
        let text = "a long line of text that would wrap anywhere narrow";
        let laid = lay_out(&one(text, font(10.0), para()), None);
        assert_eq!(laid.lines.len(), 1);
        assert_eq!(line_text(&laid.lines[0]), text);
        assert!(close(laid.width, width(text, 10.0)));
    }

    #[test]
    fn an_empty_paragraph_takes_one_line() {
        let laid = lay_out(&one("", font(20.0), para()), Some(50.0));
        assert_eq!(laid.lines.len(), 1);
        assert!(laid.lines[0].pieces.is_empty());
        assert!(close(laid.height, 24.0));
        assert!(close(laid.lines[0].baseline, 14.36));
        let laid = lay_out(
            &Block {
                paragraphs: vec![Paragraph {
                    runs: vec![],
                    para: para(),
                }],
            },
            None,
        );
        assert!(close(laid.height, 12.0));
        assert!(close(lay_out(&Block::default(), None).height, 0.0));
    }

    #[test]
    fn line_height_and_mixed_sizes() {
        let block = Block {
            paragraphs: vec![Paragraph {
                runs: vec![
                    Run {
                        text: "a".into(),
                        font: font(10.0),
                    },
                    Run {
                        text: "b".into(),
                        font: font(20.0),
                    },
                ],
                para: para(),
            }],
        };
        let laid = lay_out(&block, None);
        assert!(close(laid.height, 24.0));
        assert!(close(laid.lines[0].baseline, 14.36));
        assert_eq!(laid.lines[0].pieces.len(), 2);
        assert!(close(laid.lines[0].pieces[1].x, width("a", 10.0)));
        let p = Para {
            line_height: Some(5.0),
            ..para()
        };
        let laid = lay_out(&one("a", font(10.0), p), None);
        assert!(close(laid.height, 5.0));
        assert!(close(laid.lines[0].baseline, 7.18));
    }

    #[test]
    fn runs_of_one_style_merge_and_others_do_not() {
        let bold = Font {
            bold: true,
            ..font(10.0)
        };
        let block = Block {
            paragraphs: vec![Paragraph {
                runs: vec![
                    Run {
                        text: "ab ".into(),
                        font: font(10.0),
                    },
                    Run {
                        text: "cd".into(),
                        font: font(10.0),
                    },
                    Run {
                        text: "ef".into(),
                        font: bold,
                    },
                ],
                para: para(),
            }],
        };
        let laid = lay_out(&block, None);
        let pieces = &laid.lines[0].pieces;
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0].text, "ab cd");
        assert_eq!(pieces[1].text, "ef");
        assert_eq!(pieces[1].face, Face::HelveticaBold);
        assert!(close(pieces[1].x, width("ab cd", 10.0)));
    }

    #[test]
    fn characters_winansi_lacks_show_as_question_marks() {
        let laid = lay_out(&one("a\u{4e2d}\u{2019}", font(10.0), para()), None);
        assert_eq!(laid.lines[0].pieces[0].text, "a?\u{2019}");
        assert!(close(laid.lines[0].width, width("a?\u{2019}", 10.0)));
    }

    #[test]
    fn stops_at_the_character_limit() {
        let text = "a".repeat(MAX_BLOCK_CHARS + 10);
        let laid = lay_out(&one(&text, font(10.0), para()), None);
        let shown: usize = laid
            .lines
            .iter()
            .flat_map(|l| &l.pieces)
            .map(|p| p.text.len())
            .sum();
        assert!(
            shown <= MAX_BLOCK_CHARS && shown + 1 >= MAX_BLOCK_CHARS,
            "{shown}"
        );
        // Paragraphs count too.
        let many = Block {
            paragraphs: vec![
                Paragraph {
                    runs: vec![],
                    para: para()
                };
                MAX_BLOCK_CHARS + 5
            ],
        };
        assert_eq!(lay_out(&many, None).lines.len(), MAX_BLOCK_CHARS);
    }

    #[test]
    fn nan_and_negative_widths_put_each_character_on_a_line() {
        for w in [f64::NAN, -5.0, 0.0] {
            let laid = lay_out(&one("ab c", font(10.0), para()), Some(w));
            let texts: Vec<String> = laid.lines.iter().map(line_text).collect();
            assert_eq!(texts, ["a", "b", "c"], "{w}");
        }
        let laid = lay_out(
            &one(
                "ab",
                font(10.0),
                Para {
                    h_align: HAlign::Right,
                    ..para()
                },
            ),
            Some(f64::INFINITY),
        );
        assert!(close(laid.lines[0].pieces[0].x, 0.0));
        let nan_font = Font {
            size: f64::NAN,
            letter_spacing: f64::NAN,
            ..font(10.0)
        };
        let laid = lay_out(&one("ab", nan_font, para()), Some(10.0));
        assert!(laid.height.is_finite() && laid.width.is_finite());
    }

    #[test]
    fn plain_values_become_one_paragraph_per_line() {
        let b = block(
            &Value::Text("one\r\ntwo".into()),
            &font(10.0),
            &para(),
            &|_| None,
        );
        assert_eq!(b.paragraphs.len(), 2);
        assert_eq!(b.plain(), "one\ntwo");
    }
}
