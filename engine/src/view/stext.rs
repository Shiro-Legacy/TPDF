// SPDX-License-Identifier: MIT
//! Structured text (spec 04): the characters a page shows, read from the
//! same interpretation that draws it, with their page-space geometry,
//! grouped into lines and blocks; plain text, the line summaries the app
//! draws its text layer from, and search.
//!
//! hayro reports each text-showing operator once, whatever its rendering
//! mode, with each glyph's code and the font dictionary (`Device::show_text`,
//! a patch in the vendored interpreter), so every glyph reads the code that
//! drew it. Codes become Unicode through spec 05 (`text_unicode`).
//!
//! The layout keeps drawing order. Its thresholds, in ems of the glyphs'
//! size, are this module's design inside the bands spec 04 sets (the
//! constants below).
use super::fonts::is_cjk_fallback;
use super::text_unicode::{
    BidiClass, CMaps, FontText, bidi_class, bidi_mirror, normalise_extracted,
};
use super::walk::{self, Scope};
use super::{PageFrame, ViewDocument, ViewError};
use crate::object::decode_text_string;
use hayro::kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2};
use hayro_interpret::font::{FontQuery, Glyph, GlyphRun, PositionedGlyph, StandardFont, TextRun};
use hayro_interpret::hayro_cmap::BfString;
use hayro_interpret::{
    BlendMode, CacheKey, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, SoftMask, interpret_page,
};
use hayro_syntax::object::{Array, Dict, MaybeRef, Name, Object, ObjectIdentifier, Stream};
use hayro_syntax::xref::XRef;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// A quad cap for direct callers of `StructuredText::search`.
pub const MAX_SEARCH_QUADS: usize = 1 << 16;

/// Characters one page's structured text may hold, ActualText included.
/// Past it the page is refused with
/// `ViewError::TooComplex("page has more text than the limit")`.
pub const MAX_PAGE_CHARS: usize = 1 << 20;

// ------------------------------------------------------------------ layout design

/// Two glyph directions are parallel when the cosine of their angle is at
/// least this (about 2.6 degrees).
const PARALLEL: f64 = 0.999;
/// A glyph joins a line while its baseline is at most this far (ems of the
/// larger of its size and the line's) from the line's baseline:
/// superscripts and subscripts (up to about 0.4 em) stay, the next line
/// (1 em or more) does not.
const MAX_BASELINE_SHIFT: f64 = 0.55;
/// A glyph that starts more than this many ems before the end of the one
/// before it (an overlap) starts a new line, unless it lies just before the
/// previous glyph (right-to-left text drawn with the pen moving left).
const MAX_OVERLAP: f64 = 1.0;
/// A left-to-right run drawn after a line but starting before it joins
/// the line at its front only if every one of its glyphs ends no more than
/// this far (ems of the smaller of the glyph and the line) into the line's
/// characters: the no-space band of 3.4 (spec 04, 12 C5 rule 1, C5a). A
/// right-to-left glyph (the pen moving left, 3.6) may reach `MAX_OVERLAP`
/// into them, so kerned right-to-left text stays one line (C5 rule 3).
const FRONT_TOUCH: f64 = 0.10;
/// Characters of one run at positions along the line closer than this
/// (ems) are at the same position and keep their drawing order (spec 04,
/// 12 C3.2, the tolerance of 3.5).
const SAME_POSITION: f64 = 0.01;
/// A gap along a line this wide (ems) or wider starts a new line in the
/// same block instead of a space.
const LINE_BREAK_GAP: f64 = 3.0;
/// A gap along a line at least this wide (ems of the second glyph) reads as
/// a space. Spec 04 requires none at 0.10 em and one at 0.25 em; justified
/// word gaps reach down to about 0.17 em while kerning and tracking stay
/// under 0.1 em, and the reference viewer switches between 0.14 and 0.16
/// (spec 04, E3), which the corpus comparison rewards.
const SPACE_GAP: f64 = 0.15;
/// A new line whose baseline is further than this (ems of the smaller of
/// the two lines' sizes) from the previous line's starts a new block:
/// single and 1.15 spacing stay together; from 1.6 em on (the reference
/// viewer's switch, E4) it is a paragraph gap.
const BLOCK_GAP: f64 = 1.55;
/// A new line (at least half an em off the last one's baseline) that starts
/// this many ems right of the block's leftmost line start is a first-line
/// indent and starts a block.
const INDENT: f64 = 0.5;
/// A copy of a glyph drawn again within this distance (ems) reads once
/// (fill then stroke, fake bold, shadows; owner decision T7).
const OVERPRINT: f64 = 0.05;
/// Characters of one overprint cell compared with a new one (a cell holds
/// a handful of distinct characters; this bounds hostile pages).
const MAX_CELL_SCAN: usize = 16;
/// In the run layout, a text-showing run whose first glyph starts more than
/// this many ems left of where the previous run on its line ended, and
/// reaches more than `RUN_OVERLAP` em right of that run's start, starts a
/// new line (an edited run moved back over its neighbour, spec 04, 12 C2).
/// Right-to-left text drawn glyph by glyph with the pen moving left lies
/// wholly left of the run before it (abutting it or with a gap) and still
/// joins.
const RUN_MOVED_BACK: f64 = 1.0;
const RUN_OVERLAP: f64 = 0.05;
/// Search splits a hit's quad at a gap wider than this (ems) between
/// consecutive characters of one line (owner decision T13).
const QUAD_GAP: f64 = 1.0;

/// Bytes of one font program decoded for its facts and Unicode readings.
const MAX_PROGRAM_BYTES: usize = 1 << 26;
/// A reader starts its caches afresh past these: fonts, decoded program
/// bytes, pages read.
const MAX_READER_FONTS: usize = 1024;
const MAX_READER_PROGRAM_BYTES: usize = 1 << 27;
const MAX_READER_PAGES: usize = 64;
/// Number-tree levels followed, and nodes visited, for a structure lookup.
const MAX_TREE_DEPTH: usize = 32;
const MAX_TREE_NODES: usize = 4096;

const REPLACEMENT: char = '\u{FFFD}';

// ------------------------------------------------------------------ public records

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
    /// A space inferred from a gap, not drawn by the page.
    pub synthetic: bool,
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
    pub family: &'static str,
    pub weight: &'static str,
    pub style: &'static str,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchResult {
    /// Each hit's quads (ulx, uly, urx, ury, llx, lly, lrx, lry).
    pub hits: Vec<Vec<[f64; 8]>>,
    /// A further match exists that was left out by the limit.
    pub truncated: bool,
}

/// The structured text of one page.
#[derive(Clone, Debug)]
pub struct StructuredText {
    fonts: Vec<TextFont>,
    blocks: Vec<TextBlock>,
    spans: Vec<TextBlock>,
    unclipped: Vec<TextBlock>,
}

// ------------------------------------------------------------------ entry points

impl ViewDocument {
    pub fn structured_text(&self, page: usize) -> Result<StructuredText, ViewError> {
        self.text_reader().structured_text(page)
    }

    pub fn text_reader(&self) -> TextReader<'_> {
        TextReader {
            view: self,
            state: RefCell::new(ReaderState::default()),
        }
    }
}

/// Reads the structured text of several pages of one view, sharing font
/// and CMap work between them.
pub struct TextReader<'a> {
    view: &'a ViewDocument,
    state: RefCell<ReaderState<'a>>,
}

/// What a reader keeps across pages.
#[derive(Default)]
struct ReaderState<'a> {
    cache: InterpreterCache<'a>,
    fonts: HashMap<u128, Rc<FontEntry>>,
    cmaps: CMaps,
    program_bytes: usize,
    pages: usize,
}

impl ReaderState<'_> {
    /// Starts afresh when the caches grew past their bounds.
    fn bound(&mut self) {
        if self.fonts.len() > MAX_READER_FONTS
            || self.program_bytes > MAX_READER_PROGRAM_BYTES
            || self.pages >= MAX_READER_PAGES
        {
            *self = Self::default();
        }
    }
}

impl<'a> TextReader<'a> {
    pub fn structured_text(&self, page: usize) -> Result<StructuredText, ViewError> {
        let view: &'a ViewDocument = self.view;
        let page = view.page(page)?;
        walk::check_page(
            page,
            &view.font_sizes,
            Scope {
                paint: false,
                annotations: false,
            },
        )?;
        let frame = PageFrame::new(page);
        let area = Rect::new(0.0, 0.0, f64::from(frame.width), f64::from(frame.height));
        let mut state = self.state.borrow_mut();
        state.bound();
        state.pages += 1;
        state.cache.start_content();
        let cache = state.cache.clone();
        let settings = super::fonts::settings(false, false);
        let mut ctx = Context::new(frame.ctm, area, &cache, page.xref(), settings);
        let structure = Structure::new(page.xref(), page.raw());
        let mut collector = Collector::new(&mut state, area, structure);
        interpret_page(page, &mut ctx, &mut collector);
        collector.finish_open_sequences();
        let Collector {
            chars,
            fonts,
            error,
            ..
        } = collector;
        if let Some(why) = cache.content_refusal() {
            return Err(walk::refused(why));
        }
        if let Some(error) = error {
            return Err(error);
        }
        Ok(StructuredText::build(chars, fonts))
    }
}

// ------------------------------------------------------------------ fonts

/// What the reader knows about one font dictionary.
struct FontEntry {
    /// Its codes' Unicode text (spec 05); `None` for the interpreter's own
    /// fallback font.
    text: Option<FontText>,
    facts: TextFont,
}

/// The name `TextFont::name` holds for a font whose /BaseFont has the
/// bytes `base`.
pub(crate) fn text_font_name(base: &[u8]) -> String {
    String::from_utf8_lossy(base).into_owned()
}

fn name_bytes(dict: &Dict<'_>, key: &[u8]) -> Option<Vec<u8>> {
    dict.get::<Name<'_>>(key).map(|n| n.to_vec())
}

fn number_of(object: Option<Object<'_>>) -> Option<f64> {
    match object? {
        Object::Number(n) => Some(n.as_f64()).filter(|v| v.is_finite()),
        _ => None,
    }
}

/// A /BaseFont without its six-letter subset prefix.
fn without_subset(name: &[u8]) -> &[u8] {
    match (name.get(..6), name.get(6), name.get(7..)) {
        (Some(prefix), Some(b'+'), Some(rest)) if prefix.iter().all(u8::is_ascii_uppercase) => rest,
        _ => name,
    }
}

fn contains_word(name: &[u8], word: &str) -> bool {
    String::from_utf8_lossy(name)
        .to_ascii_lowercase()
        .contains(word)
}

/// The standard font a non-embedded font named `name` is drawn with, when
/// its name is one of the 14 (or a common alias of one).
fn standard_of(name: &[u8]) -> Option<StandardFont> {
    use StandardFont as S;
    let name = without_subset(name);
    let base = name.split(|&b| b == b',').next().unwrap_or(name);
    let bold = contains_word(name, "bold");
    let slanted = contains_word(name, "italic") || contains_word(name, "oblique");
    let family = |w: &str| contains_word(base, w);
    let face = |faces: [S; 4]| faces[usize::from(bold) * 2 + usize::from(slanted)];
    Some(if family("courier") {
        face([
            S::Courier,
            S::CourierOblique,
            S::CourierBold,
            S::CourierBoldOblique,
        ])
    } else if family("times") {
        face([
            S::TimesRoman,
            S::TimesItalic,
            S::TimesBold,
            S::TimesBoldItalic,
        ])
    } else if family("helvetica") || family("arial") {
        face([
            S::Helvetica,
            S::HelveticaOblique,
            S::HelveticaBold,
            S::HelveticaBoldOblique,
        ])
    } else if base == b"Symbol" || base == b"SymbolMT" {
        S::Symbol
    } else if base == b"ZapfDingbats" || base == b"Dingbats" {
        S::ZapfDingBats
    } else {
        return None;
    })
}

/// Descender and ascender (ems) of a standard font as the view draws it:
/// the vertical metrics of the program the view substitutes when it is an
/// sfnt, else the FontBBox of Adobe's Core 14 AFM files.
fn standard_extent(font: StandardFont) -> (f64, f64) {
    use StandardFont as S;
    let settings = super::fonts::settings(false, false);
    let substitute = (settings.font_resolver)(&FontQuery::Standard(font))
        .and_then(|(data, _)| program_facts((*data).as_ref()).extent);
    if let Some(extent) = substitute {
        return extent;
    }
    let (low, high) = match font {
        S::Courier | S::CourierOblique => (-250.0, 805.0),
        S::CourierBold | S::CourierBoldOblique => (-250.0, 801.0),
        S::TimesRoman => (-218.0, 898.0),
        S::TimesBold => (-218.0, 935.0),
        S::TimesItalic => (-217.0, 883.0),
        S::TimesBoldItalic => (-218.0, 921.0),
        S::Helvetica | S::HelveticaOblique => (-225.0, 931.0),
        S::HelveticaBold | S::HelveticaBoldOblique => (-228.0, 962.0),
        S::Symbol => (-293.0, 1010.0),
        S::ZapfDingBats => (-143.0, 820.0),
    };
    (low / 1000.0, high / 1000.0)
}

/// The facts an sfnt (TrueType or OpenType) program gives about itself.
#[derive(Default)]
struct ProgramFacts {
    /// Descender and ascender in ems.
    extent: Option<(f64, f64)>,
    bold: bool,
    italic: bool,
    fixed: bool,
    /// The OS/2 family class, when the program gives one other than 0.
    class: Option<u8>,
}

fn be16(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*data.get(at)?, *data.get(at + 1)?]))
}

fn be32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *data.get(at)?,
        *data.get(at + 1)?,
        *data.get(at + 2)?,
        *data.get(at + 3)?,
    ]))
}

/// Reads the `head`, `hhea`, `OS/2` and `post` facts of an sfnt.
fn program_facts(program: &[u8]) -> ProgramFacts {
    let mut facts = ProgramFacts::default();
    if !matches!(
        be32(program, 0),
        Some(0x0001_0000 | 0x7472_7565 | 0x4F54_544F)
    ) {
        return facts;
    }
    let count = usize::from(be16(program, 4).unwrap_or(0));
    let table = |tag: &[u8; 4]| -> Option<&[u8]> {
        (0..count).find_map(|i| {
            let record = 12 + 16 * i;
            if program.get(record..record + 4)? != tag {
                return None;
            }
            let offset = usize::try_from(be32(program, record + 8)?).ok()?;
            let length = usize::try_from(be32(program, record + 12)?).ok()?;
            program.get(offset..offset.checked_add(length)?)
        })
    };
    let head = table(b"head");
    let units = head
        .and_then(|h| be16(h, 18))
        .filter(|&u| u > 0)
        .map_or(1000.0, f64::from);
    if let Some(style) = head.and_then(|h| be16(h, 44)) {
        facts.bold |= style & 1 != 0;
        facts.italic |= style & 2 != 0;
    }
    let os2 = table(b"OS/2");
    if let Some(os2) = os2 {
        if let Some(selection) = be16(os2, 62) {
            facts.bold |= selection & 0x20 != 0;
            facts.italic |= selection & 0x201 != 0;
        }
        facts.class = os2.get(30).copied().filter(|&c| c != 0);
    }
    if let Some(post) = table(b"post") {
        facts.fixed = be32(post, 12).is_some_and(|f| f != 0);
    }
    let signed = |v: u16| f64::from(v as i16) / units;
    let typo = os2.and_then(|o| Some((signed(be16(o, 70)?), signed(be16(o, 68)?))));
    let hhea = table(b"hhea").and_then(|h| Some((signed(be16(h, 6)?), signed(be16(h, 4)?))));
    facts.extent = hhea
        .filter(|e| plausible(*e))
        .or(typo.filter(|e| plausible(*e)));
    facts
}

/// Whether a (descender, ascender) pair in ems is usable for quads: the
/// right signs, not absurdly large, and at least half an em in all.
fn plausible((descender, ascender): (f64, f64)) -> bool {
    ascender > 0.0
        && ascender <= 3.0
        && (-2.0..=0.0).contains(&descender)
        && ascender - descender >= 0.5
}

const PROGRAM_KEYS: [&[u8]; 3] = [b"FontFile2", b"FontFile3", b"FontFile"];

/// The decoded embedded program of a font (its descendant's for Type0).
fn embedded_program(descriptor: Option<&Dict<'_>>) -> Option<Vec<u8>> {
    let descriptor = descriptor?;
    PROGRAM_KEYS
        .iter()
        .find_map(|key| descriptor.get::<Stream<'_>>(*key))
        .and_then(|stream| stream.decoded_within(MAX_PROGRAM_BYTES).ok())
        .map(|data| data.into_owned())
}

/// The descriptor's /Flags bit `bit` (numbered from 1, §9.8.2).
fn flag(flags: Option<u32>, bit: u32) -> bool {
    flags.is_some_and(|f| f & (1 << (bit - 1)) != 0)
}

fn flags_of(descriptor: Option<&Dict<'_>>) -> Option<u32> {
    number_of(descriptor.and_then(|d| d.get::<Object<'_>>(b"Flags"))).map(|f| f as i64 as u32)
}

impl FontEntry {
    /// The entry of a font dictionary, and the program bytes it decoded.
    fn new(font: &Dict<'_>, cmaps: &mut CMaps) -> (Self, usize) {
        let subtype = name_bytes(font, b"Subtype").unwrap_or_default();
        let descendant = if subtype == b"Type0" {
            font.get::<Array<'_>>(b"DescendantFonts")
                .and_then(|fonts| fonts.iter::<Dict<'_>>().next())
        } else {
            None
        };
        let described = descendant.as_ref().unwrap_or(font);
        let descriptor = described.get::<Dict<'_>>(b"FontDescriptor");
        let program = embedded_program(descriptor.as_ref());
        let text = FontText::new(font, program.as_deref(), cmaps);
        let base = name_bytes(described, b"BaseFont")
            .or_else(|| name_bytes(font, b"BaseFont"))
            .unwrap_or_default();
        let facts = if subtype == b"Type3" {
            type3_facts(font, descriptor.as_ref())
        } else {
            font_facts(&base, descriptor.as_ref(), program.as_deref())
        };
        let bytes = program.as_ref().map_or(0, Vec::len);
        (
            Self {
                text: Some(text),
                facts,
            },
            bytes,
        )
    }

    /// The font the interpreter draws with when none (or none it can load)
    /// is set: Helvetica.
    fn fallback() -> Self {
        Self {
            text: None,
            facts: font_facts(b"Helvetica", None, None),
        }
    }
}

/// `TextFont` of a Type1, TrueType or composite font (spec 04, 3.3).
fn font_facts(base: &[u8], descriptor: Option<&Dict<'_>>, program: Option<&[u8]>) -> TextFont {
    let flags = flags_of(descriptor);
    let number = |key: &[u8]| number_of(descriptor.and_then(|d| d.get::<Object<'_>>(key)));
    let program = program.map(program_facts).unwrap_or_default();
    let embedded = descriptor.is_some_and(|d| PROGRAM_KEYS.iter().any(|key| d.contains_key(key)));
    let name = without_subset(base);
    let word = |w: &str| contains_word(name, w);
    let described = number(b"Ascent")
        .zip(number(b"Descent"))
        .map(|(a, d)| (d / 1000.0, a / 1000.0))
        .filter(|e| plausible(*e));
    let (descender, ascender) = described
        .or(program.extent)
        .or_else(|| (!embedded).then(|| standard_of(base).map(standard_extent))?)
        .unwrap_or((-0.2, 0.8));
    let bold = flag(flags, 19)
        || number(b"FontWeight").is_some_and(|w| w >= 600.0)
        || program.bold
        || ["bold", "black", "heavy", "semibold"]
            .iter()
            .any(|w| word(w));
    let italic = flag(flags, 7)
        || number(b"ItalicAngle").is_some_and(|a| a != 0.0)
        || program.italic
        || word("italic")
        || word("oblique");
    let monospace = flag(flags, 1) || program.fixed || word("courier");
    let serif_class = program.class.is_some_and(|c| matches!(c, 1..=5 | 7));
    let sans = program.class == Some(8) || word("helvetica") || word("arial");
    // An embedded program that classifies itself (OS/2 family class)
    // decides; one that does not is shown in the reader's default face,
    // serif in CSS. A font the view substitutes is what its descriptor or
    // name says, else sans-serif.
    let serif = flag(flags, 2)
        || serif_class
        || word("times")
        || (embedded && !sans && program.class.is_none());
    TextFont {
        name: text_font_name(base),
        ascender,
        descender,
        bold,
        italic,
        serif: serif && !monospace,
        monospace,
    }
}

/// `TextFont` of a Type3 font: its extent is the /FontBBox through the
/// /FontMatrix; it is neither serif nor monospace.
fn type3_facts(font: &Dict<'_>, descriptor: Option<&Dict<'_>>) -> TextFont {
    let numbers = |key: &[u8]| -> Option<Vec<f64>> {
        let array = font.get::<Array<'_>>(key)?;
        array
            .iter::<Object<'_>>()
            .map(|o| number_of(Some(o)))
            .collect()
    };
    let matrix = numbers(b"FontMatrix")
        .filter(|m| m.len() == 6)
        .map_or(Affine::scale(0.001), |m| {
            Affine::new([m[0], m[1], m[2], m[3], m[4], m[5]])
        });
    let extent = numbers(b"FontBBox").filter(|b| b.len() == 4).and_then(|b| {
        let ys = [(b[0], b[1]), (b[2], b[1]), (b[0], b[3]), (b[2], b[3])]
            .map(|(x, y)| (matrix * Point::new(x, y)).y);
        let low = ys.iter().copied().fold(f64::INFINITY, f64::min);
        let high = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Some((low, high)).filter(|e| plausible(*e))
    });
    let (descender, ascender) = extent.unwrap_or((-0.2, 0.8));
    let name = name_bytes(font, b"Name").map_or_else(
        || match font.obj_id() {
            Some(id) => format!("Type3-{}-{}", id.obj_number, id.gen_number),
            None => format!("Type3-{:08x}", font.cache_key() as u32),
        },
        |n| text_font_name(&n),
    );
    let flags = flags_of(descriptor);
    TextFont {
        name,
        ascender,
        descender,
        bold: flag(flags, 19),
        italic: flag(flags, 7),
        serif: false,
        monospace: false,
    }
}

// ------------------------------------------------------------------ characters

/// What changes a run (spec 04, 2.4): font, size and glyph matrix, fill
/// colour, rendering mode, text object, direction.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RunKey {
    font: usize,
    matrix: [i64; 4],
    fill: Option<[u8; 4]>,
    mode: u8,
    object: u64,
    rtl: bool,
}

/// A character as the page draws it.
#[derive(Clone, Copy, Debug)]
struct RawChar {
    c: char,
    origin: Point,
    /// The pen position after the character's advance.
    end: Point,
    /// Unit baseline direction.
    dir: Vec2,
    /// ul, ur, ll, lr.
    quad: [Point; 4],
    size: f64,
    font: usize,
    run: RunKey,
    visible: bool,
    /// Drawn by a glyph (a copy drawn again reads once).
    glyph: bool,
    /// The first character of its glyph.
    first: bool,
    /// Counts the glyphs drawn on the page: the characters of one glyph
    /// share it.
    glyph_serial: u64,
    /// Counts the text-showing operators (and ActualText placements): the
    /// characters of one share it.
    show: u64,
    synthetic: bool,
}

impl RawChar {
    fn public(&self) -> TextChar {
        let q = &self.quad;
        TextChar {
            c: self.c,
            origin: [self.origin.x, self.origin.y],
            quad: [
                q[0].x, q[0].y, q[1].x, q[1].y, q[2].x, q[2].y, q[3].x, q[3].y,
            ],
            size: self.size,
            font: self.font,
            synthetic: self.synthetic,
        }
    }
}

fn bounds_of(points: &[Point]) -> Rect {
    let mut r = Rect::new(
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for p in points {
        r.x0 = r.x0.min(p.x);
        r.y0 = r.y0.min(p.y);
        r.x1 = r.x1.max(p.x);
        r.y1 = r.y1.max(p.y);
    }
    r
}

fn finite(r: Rect) -> bool {
    r.x0.is_finite() && r.y0.is_finite() && r.x1.is_finite() && r.y1.is_finite()
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0.max(b.x0) < a.x1.min(b.x1) && a.y0.max(b.y0) < a.y1.min(b.y1)
}

/// Whether `inner` lies wholly inside `outer`.
fn inside(inner: Rect, outer: Rect) -> bool {
    inner.x0 >= outer.x0 && inner.y0 >= outer.y0 && inner.x1 <= outer.x1 && inner.y1 <= outer.y1
}

fn quantise(v: f64) -> i64 {
    (v * 1000.0).round() as i64
}

/// An open marked-content sequence whose ActualText replaces its content.
struct ActualSeq {
    text: Vec<char>,
    /// Index of the first character drawn inside it.
    start: usize,
    /// The union of what it painted other than glyphs (page space), and
    /// whether any of that was visible.
    painted: Option<Rect>,
    painted_visible: bool,
}

/// Structure-tree lookups for one page: the page's array of the parent
/// tree, found once (§14.7.5.4), then indexed by MCID.
struct Structure<'a> {
    xref: &'a XRef,
    key: Option<i64>,
    parents: Option<Vec<MaybeRef<Object<'a>>>>,
    looked: bool,
}

impl<'a> Structure<'a> {
    fn new(xref: &'a XRef, page: &Dict<'a>) -> Self {
        let key = number_of(page.get::<Object<'_>>(b"StructParents")).map(|k| k as i64);
        Self {
            xref,
            key,
            parents: None,
            looked: false,
        }
    }

    fn resolve(&self, item: &MaybeRef<Object<'a>>) -> Option<Object<'a>> {
        match item {
            MaybeRef::Ref(r) => self.xref.get::<Object<'a>>(ObjectIdentifier::from(*r)),
            MaybeRef::NotRef(o) => Some(o.clone()),
        }
    }

    /// The ActualText of the structure element of `mcid`, if any.
    fn actual_text(&mut self, mcid: i64) -> Option<Vec<char>> {
        if !self.looked {
            self.looked = true;
            self.parents = self.find_parents();
        }
        let index = usize::try_from(mcid).ok()?;
        let item = self.parents.as_ref()?.get(index)?;
        let element = self.resolve(item)?.into_dict()?;
        let actual = element.get::<hayro_syntax::object::String<'_>>(b"ActualText")?;
        Some(text_chars(actual.as_bytes()))
    }

    fn find_parents(&self) -> Option<Vec<MaybeRef<Object<'a>>>> {
        let key = self.key?;
        let catalog = self.xref.get::<Dict<'a>>(self.xref.root_id())?;
        let tree = catalog
            .get::<Dict<'a>>(b"StructTreeRoot")?
            .get::<Dict<'a>>(b"ParentTree")?;
        let mut nodes = 0;
        let value = self.number_tree(&tree, key, 0, &mut nodes)?;
        let array = self.resolve(&value)?.into_array()?;
        Some(array.raw_iter().collect())
    }

    /// The value of `key` in the number tree at `node` (§7.9.7).
    fn number_tree(
        &self,
        node: &Dict<'a>,
        key: i64,
        depth: usize,
        nodes: &mut usize,
    ) -> Option<MaybeRef<Object<'a>>> {
        *nodes += 1;
        if depth > MAX_TREE_DEPTH || *nodes > MAX_TREE_NODES {
            return None;
        }
        if let Some(nums) = node.get::<Array<'a>>(b"Nums") {
            let mut items = nums.raw_iter();
            while let (Some(k), Some(v)) = (items.next(), items.next()) {
                if number_of(self.resolve(&k)).map(|k| k as i64) == Some(key) {
                    return Some(v);
                }
            }
        }
        let kids = node.get::<Array<'a>>(b"Kids")?;
        for kid in kids.iter::<Dict<'a>>() {
            let limits: Vec<f64> = kid
                .get::<Array<'a>>(b"Limits")
                .map(|a| {
                    a.iter::<Object<'_>>()
                        .filter_map(|o| number_of(Some(o)))
                        .collect()
                })
                .unwrap_or_default();
            let within = match limits.as_slice() {
                [low, high] => (*low as i64) <= key && key <= (*high as i64),
                _ => true,
            };
            if within {
                if let Some(found) = self.number_tree(&kid, key, depth + 1, nodes) {
                    return Some(found);
                }
            }
        }
        None
    }
}

/// A text string's characters (§7.9.2.2) without U+0000.
fn text_chars(bytes: &[u8]) -> Vec<char> {
    decode_text_string(bytes)
        .chars()
        .filter(|&c| c != '\0')
        .collect()
}

/// The device that collects a page's characters.
struct Collector<'s, 'a> {
    reader: &'s mut ReaderState<'a>,
    /// Font key to index in `fonts`.
    page_fonts: HashMap<u128, usize>,
    entries: Vec<Rc<FontEntry>>,
    fonts: Vec<TextFont>,
    chars: Vec<RawChar>,
    /// The page area in page space.
    area: Rect,
    /// The bounds of the clips in force, each within those before it.
    clips: Vec<Rect>,
    /// One entry per open marked-content sequence: whether it began the
    /// ActualText in force.
    marked: Vec<bool>,
    /// The ActualText in force (the outermost one), if any.
    actual: Option<ActualSeq>,
    structure: Structure<'a>,
    /// Counts ActualText placed over no glyphs, each its own run.
    serial: u64,
    /// Counts the glyphs drawn.
    glyphs: u64,
    /// Counts the text-showing operators.
    shows: u64,
    error: Option<ViewError>,
}

const TOO_MUCH_TEXT: ViewError = ViewError::TooComplex("page has more text than the limit");

impl<'s, 'a> Collector<'s, 'a> {
    fn new(reader: &'s mut ReaderState<'a>, area: Rect, structure: Structure<'a>) -> Self {
        Self {
            reader,
            page_fonts: HashMap::new(),
            entries: Vec::new(),
            fonts: Vec::new(),
            chars: Vec::new(),
            area,
            clips: Vec::new(),
            marked: Vec::new(),
            actual: None,
            structure,
            serial: 0,
            glyphs: 0,
            shows: 0,
            error: None,
        }
    }

    /// The visible region: the page area within every clip in force.
    fn region(&self) -> Rect {
        match self.clips.last() {
            Some(clip) => self.area.intersect(*clip),
            None => self.area,
        }
    }

    fn in_actual_text(&self) -> bool {
        self.actual.is_some()
    }

    /// The index of the font of `dict` (`None`: the interpreter's fallback).
    fn font(&mut self, dict: Option<&Dict<'_>>) -> usize {
        let key = dict.map_or(0, |d| d.cache_key().max(1));
        if let Some(&index) = self.page_fonts.get(&key) {
            return index;
        }
        let entry = match (dict, self.reader.fonts.get(&key)) {
            (_, Some(entry)) => entry.clone(),
            (None, None) => Rc::new(FontEntry::fallback()),
            (Some(dict), None) => {
                let (entry, bytes) = FontEntry::new(dict, &mut self.reader.cmaps);
                self.reader.program_bytes = self.reader.program_bytes.saturating_add(bytes);
                Rc::new(entry)
            }
        };
        self.reader.fonts.insert(key, entry.clone());
        let index = self.fonts.len();
        self.fonts.push(entry.facts.clone());
        self.entries.push(entry);
        self.page_fonts.insert(key, index);
        index
    }

    fn push(&mut self, c: RawChar) -> bool {
        if self.chars.len() >= MAX_PAGE_CHARS {
            self.error = Some(TOO_MUCH_TEXT);
            return false;
        }
        self.chars.push(c);
        true
    }

    /// Records what an ActualText sequence painted besides glyphs.
    fn paint(&mut self, bounds: Rect) {
        if !finite(bounds) {
            return;
        }
        let visible = overlaps(bounds, self.region());
        if let Some(seq) = self.actual.as_mut() {
            seq.painted = Some(seq.painted.map_or(bounds, |p| p.union(bounds)));
            seq.painted_visible |= visible;
        }
    }

    /// The characters of one glyph's code: spec 05's reading, else the
    /// interpreter's, else U+FFFD; normalised.
    fn glyph_text(entry: &FontEntry, glyph: &PositionedGlyph<'_>) -> Vec<char> {
        let code = glyph.code();
        let from_font = entry.text.as_ref().and_then(|text| {
            let mut codes = Vec::new();
            text.codes(code.as_bytes(), &mut codes);
            text.text(*codes.first()?)
        });
        let chars = from_font.or_else(|| {
            let chars: Vec<char> = match glyph.as_unicode()? {
                BfString::Char(c) => vec![c],
                BfString::String(s) => s.chars().collect(),
            };
            let chars: Vec<char> = chars
                .into_iter()
                .filter_map(|c| match c {
                    '\u{9}'..='\u{D}' => Some(' '),
                    '\u{0}'..='\u{1F}' | '\u{7F}'..='\u{9F}' | '\u{FFFE}' | '\u{FFFF}' => None,
                    c => Some(c),
                })
                .collect();
            (!chars.is_empty()).then_some(chars)
        });
        let mut out = Vec::new();
        for c in chars.unwrap_or_else(|| vec![REPLACEMENT]) {
            normalise_extracted(c, &mut out);
        }
        out
    }

    /// The characters of one text-showing operator (spec 04, 3.1, 3.2).
    fn show(&mut self, run: &TextRun<'_, '_>) {
        if self.error.is_some() || run.glyphs.is_empty() {
            return;
        }
        self.shows += 1;
        let show = self.shows;
        let font = self.font(run.font);
        let Some(entry) = self.entries.get(font).cloned() else {
            return;
        };
        let (ascender, descender) = (entry.facts.ascender, entry.facts.descender);
        let region = self.region();
        let mut cjk: Option<bool> = None;
        for glyph in run.glyphs {
            let code = glyph.code();
            // Em space, origin at the pen.
            let em = run.transform
                * glyph.transform()
                * Affine::translate(-code.displacement)
                * Affine::scale(1000.0);
            let c = em.as_coeffs();
            let det = em.determinant();
            if !c.iter().all(|v| v.is_finite()) || det == 0.0 || !det.is_finite() {
                continue;
            }
            let size = det.abs().sqrt();
            // Along the baseline and across it, in em space.
            let (along, up, advance, top, bottom) = if run.horizontal {
                (
                    Vec2::new(1.0, 0.0),
                    Vec2::new(0.0, 1.0),
                    code.advance.x / 1000.0,
                    ascender,
                    descender,
                )
            } else {
                let half = (code.displacement.x / 1000.0).abs().max(0.25);
                (
                    Vec2::new(0.0, -1.0),
                    Vec2::new(1.0, 0.0),
                    -code.advance.y / 1000.0,
                    half,
                    -half,
                )
            };
            if !advance.is_finite() {
                continue;
            }
            let at = |a: f64, h: f64| em * (along * a + up * h).to_point();
            let origin = at(0.0, 0.0);
            let end = at(advance, 0.0);
            let dir = Vec2::new(
                c[0] * along.x + c[2] * along.y,
                c[1] * along.x + c[3] * along.y,
            );
            let length = dir.hypot();
            if length == 0.0 || !length.is_finite() {
                continue;
            }
            let dir = dir / length;
            let quad = [
                at(0.0, top),
                at(advance, top),
                at(0.0, bottom),
                at(advance, bottom),
            ];
            let em_box = bounds_of(&quad);
            let visible = if inside(em_box, region) {
                true
            } else {
                let ink = match &**glyph {
                    Glyph::Outline(outline) => {
                        let fallback = *cjk.get_or_insert_with(|| {
                            outline
                                .font_data()
                                .is_some_and(|data| is_cjk_fallback(&data.data))
                        });
                        if fallback {
                            None
                        } else {
                            let path: BezPath =
                                run.transform * glyph.transform() * outline.outline();
                            (path.elements().len() > 1).then(|| path.bounding_box())
                        }
                    }
                    Glyph::Type3(_) => None,
                };
                overlaps(ink.filter(|r| finite(*r)).unwrap_or(em_box), region)
            };
            let key = RunKey {
                font,
                matrix: [
                    quantise(c[0]),
                    quantise(c[1]),
                    quantise(c[2]),
                    quantise(c[3]),
                ],
                fill: run.fill,
                mode: run.render_mode,
                object: run.text_object,
                rtl: false,
            };
            self.glyphs += 1;
            for (i, ch) in Self::glyph_text(&entry, glyph).into_iter().enumerate() {
                let first = i == 0;
                let raw = RawChar {
                    c: ch,
                    origin,
                    end: if first { end } else { origin },
                    dir,
                    quad: if first {
                        quad
                    } else {
                        [quad[0], quad[0], quad[2], quad[2]]
                    },
                    size,
                    font,
                    run: key,
                    visible,
                    glyph: true,
                    first,
                    glyph_serial: self.glyphs,
                    show,
                    synthetic: false,
                };
                if !self.push(raw) {
                    return;
                }
            }
        }
    }

    fn begin(&mut self, actual: Option<Vec<char>>) {
        let begins = match actual {
            Some(text) if !self.in_actual_text() => {
                if self.chars.len().saturating_add(text.len()) > MAX_PAGE_CHARS {
                    self.error = Some(TOO_MUCH_TEXT);
                }
                self.actual = Some(ActualSeq {
                    text,
                    start: self.chars.len(),
                    painted: None,
                    painted_visible: false,
                });
                true
            }
            _ => false,
        };
        self.marked.push(begins);
    }

    fn end(&mut self) {
        if self.marked.pop() == Some(true) {
            if let Some(seq) = self.actual.take() {
                self.replace(seq);
            }
        }
    }

    /// Ends the sequences the page left open.
    fn finish_open_sequences(&mut self) {
        self.marked.clear();
        if let Some(seq) = self.actual.take() {
            self.replace(seq);
        }
    }

    /// Puts the ActualText of `seq` in place of the glyphs drawn inside it
    /// (spec 04, 3.7): glyphs whose own text begins or ends it keep their
    /// places, the other characters take the remaining glyphs' places one
    /// each, and any surplus sits at the end of the last replaced glyph
    /// with no advance.
    fn replace(&mut self, seq: ActualSeq) {
        if self.error.is_some() {
            return;
        }
        let start = seq.start.min(self.chars.len());
        let drawn: Vec<RawChar> = self.chars.split_off(start);
        if drawn.is_empty() {
            self.place_over_painting(seq);
            return;
        }
        let mut glyphs: Vec<Vec<RawChar>> = Vec::new();
        for c in drawn {
            match glyphs.last_mut() {
                Some(g) if !c.first => g.push(c),
                _ => glyphs.push(vec![c]),
            }
        }
        let text = seq.text;
        let same = |g: &[RawChar], t: &[char]| {
            g.len() == t.len() && g.iter().zip(t).all(|(c, t)| c.c == *t)
        };
        let (mut head, mut used) = (0, 0);
        while let Some(g) = glyphs.get(head) {
            match text.get(used..used + g.len()) {
                Some(t) if same(g, t) => {
                    used += g.len();
                    head += 1;
                }
                _ => break,
            }
        }
        let (mut tail, mut left) = (glyphs.len(), text.len());
        while tail > head {
            let g = &glyphs[tail - 1];
            match left
                .checked_sub(g.len())
                .filter(|&from| from >= used)
                .and_then(|from| text.get(from..left))
            {
                Some(t) if same(g, t) => {
                    left -= g.len();
                    tail -= 1;
                }
                _ => break,
            }
        }
        let middle = text.get(used..left).unwrap_or_default();
        let slots = glyphs.get(head..tail).unwrap_or_default();
        let anchor = slots
            .last()
            .or_else(|| glyphs.get(..head).and_then(<[_]>::last))
            .and_then(|g| g.first().copied());
        let before_all = glyphs
            .get(tail..)
            .and_then(<[_]>::first)
            .and_then(|g| g.first().copied());
        let mut out: Vec<RawChar> = Vec::with_capacity(text.len());
        for g in glyphs.get(..head).unwrap_or_default() {
            out.extend(g);
        }
        for (i, &c) in middle.iter().enumerate() {
            let mut ch = match slots.get(i).and_then(|g| g.first()) {
                Some(slot) => *slot,
                None => {
                    let (base, at) = match (anchor, before_all) {
                        (Some(base), _) => (base, base.end),
                        (None, Some(base)) => (base, base.origin),
                        (None, None) => break,
                    };
                    let shift = at - base.origin;
                    let mut ch = base;
                    ch.origin = at;
                    ch.end = at;
                    ch.quad = [
                        base.quad[0] + shift,
                        base.quad[0] + shift,
                        base.quad[2] + shift,
                        base.quad[2] + shift,
                    ];
                    ch
                }
            };
            ch.c = c;
            ch.glyph = false;
            ch.first = true;
            out.push(ch);
        }
        for g in glyphs.get(tail..).unwrap_or_default() {
            out.extend(g);
        }
        for c in out {
            if !self.push(c) {
                return;
            }
        }
    }

    /// ActualText over a sequence that drew no glyphs: at the bottom left
    /// of what it painted, one em the size of that area, without advance.
    fn place_over_painting(&mut self, seq: ActualSeq) {
        let Some(painted) = seq.painted else {
            return;
        };
        let size = (painted.width() * painted.height()).sqrt();
        let size = if size > 0.0 && size.is_finite() {
            size
        } else {
            painted.width().max(painted.height()).max(1.0)
        };
        let font = self.font(None);
        let (ascender, descender) = self
            .fonts
            .get(font)
            .map_or((0.8, -0.2), |f| (f.ascender, f.descender));
        let origin = Point::new(painted.x0, painted.y1);
        let top = Point::new(origin.x, origin.y - ascender * size);
        let bottom = Point::new(origin.x, origin.y - descender * size);
        self.serial += 1;
        let key = RunKey {
            font,
            matrix: [quantise(size), 0, 0, quantise(size)],
            fill: None,
            mode: 0,
            object: u64::MAX - self.serial,
            rtl: false,
        };
        for c in seq.text {
            let raw = RawChar {
                c,
                origin,
                end: origin,
                dir: Vec2::new(1.0, 0.0),
                quad: [top, top, bottom, bottom],
                size,
                font,
                run: key,
                visible: seq.painted_visible,
                glyph: false,
                first: true,
                glyph_serial: 0,
                show: u64::MAX - self.serial,
                synthetic: false,
            };
            if !self.push(raw) {
                return;
            }
        }
    }
}

impl<'a> Device<'a> for Collector<'_, 'a> {
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, _: &DrawMode) {
        if self.in_actual_text() {
            let bounds = (props.transform * path.clone()).bounding_box();
            self.paint(bounds);
        }
    }

    fn push_clip_path(&mut self, clip: &ClipPath) {
        let bounds = clip.path.bounding_box();
        let bounds = match self.clips.last() {
            Some(top) => top.intersect(bounds),
            None => bounds,
        };
        self.clips.push(bounds);
    }

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}

    fn draw_glyph_run(&mut self, _: &GlyphRun<'_, 'a>, _: DrawProps<'a>, _: &DrawMode) {}

    fn draw_image(&mut self, image: Image<'a, '_>, props: ImageDrawProps<'a>) {
        if self.in_actual_text() {
            // The transform places the image's samples.
            let (w, h) = (f64::from(image.width()), f64::from(image.height()));
            let samples = Rect::new(0.0, 0.0, w, h).to_path(0.1);
            self.paint((props.transform * samples).bounding_box());
        }
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    fn pop_transparency_group(&mut self) {}

    fn begin_marked_content(&mut self, _tag: &[u8], _mcid: Option<i32>) {
        self.begin(None);
    }

    fn begin_marked_content_with(
        &mut self,
        _tag: &[u8],
        mcid: Option<i32>,
        properties: Option<&Dict<'_>>,
    ) {
        let has_own = properties.is_some_and(|p| p.contains_key(b"ActualText"));
        let actual = if has_own {
            properties
                .and_then(|p| p.get::<hayro_syntax::object::String<'_>>(b"ActualText"))
                .map(|s| text_chars(s.as_bytes()))
        } else if self.in_actual_text() {
            None
        } else {
            number_of(properties.and_then(|p| p.get::<Object<'_>>(b"MCID")))
                .map(|m| m as i64)
                .or(mcid.map(i64::from))
                .and_then(|m| self.structure.actual_text(m))
        };
        self.begin(actual);
    }

    fn end_marked_content(&mut self) {
        self.end();
    }

    fn show_text(&mut self, run: &TextRun<'_, 'a>) {
        self.show(run);
    }
}

// ------------------------------------------------------------------ layout

fn dot(a: Vec2, b: Vec2) -> f64 {
    a.x * b.x + a.y * b.y
}

fn cross(a: Vec2, b: Vec2) -> f64 {
    a.x * b.y - a.y * b.x
}

/// Character, size, direction and cell of the overprint grid.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct CellKey {
    c: char,
    size: i64,
    dir: [i64; 2],
    cell: [i64; 2],
}

/// A multiply-rotate hasher for the overprint grid's small keys, seeded
/// per grid from the standard library's random state (the default SipHash
/// cost most of the check).
struct Mix(u64);

impl std::hash::Hasher for Mix {
    fn finish(&self) -> u64 {
        // Fold the high bits down: the table indexes by the low ones.
        self.0 ^ (self.0 >> 29)
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(u64::from(b));
        }
    }
    fn write_u32(&mut self, v: u32) {
        self.write_u64(u64::from(v));
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    fn write_i64(&mut self, v: i64) {
        self.write_u64(v as u64);
    }
    fn write_usize(&mut self, v: usize) {
        self.write_u64(v as u64);
    }
}

#[derive(Clone, Copy)]
struct MixState(u64);

impl std::hash::BuildHasher for MixState {
    type Hasher = Mix;
    fn build_hasher(&self) -> Mix {
        Mix(self.0)
    }
}

/// Drops copies of a glyph drawn again at about the same place (spec 04,
/// 3.5): the same characters at the same size and direction, the origin
/// within `OVERPRINT` em of the first drawing's. A glyph of several
/// characters is compared and merged as a whole, each character with its
/// own counterpart, so what the copy carries (visibility) reaches all of
/// them (C6). Linear in the characters: cells two reaches wide, so a copy
/// lies in the cell of the first drawing or one of the three cells on its
/// side (four probes); each cell's glyphs are chained through `next`, and
/// only `MAX_CELL_SCAN` of them are compared.
fn without_overprints(chars: Vec<RawChar>) -> Vec<RawChar> {
    use std::hash::BuildHasher;
    let seed = std::collections::hash_map::RandomState::new().hash_one(0u8);
    let mut heads: HashMap<CellKey, usize, MixState> =
        HashMap::with_capacity_and_hasher(chars.len().min(1 << 16), MixState(seed));
    let mut next: Vec<usize> = Vec::with_capacity(chars.len());
    let mut out: Vec<RawChar> = Vec::with_capacity(chars.len());
    let mut chars = chars.into_iter().peekable();
    let mut glyph: Vec<RawChar> = Vec::new();
    while let Some(c) = chars.next() {
        if !c.glyph {
            next.push(usize::MAX);
            out.push(c);
            continue;
        }
        // The characters of one glyph.
        glyph.clear();
        glyph.push(c);
        while let Some(more) =
            chars.next_if(|m| m.glyph && !m.first && m.glyph_serial == c.glyph_serial)
        {
            glyph.push(more);
        }
        let reach = OVERPRINT * c.size;
        let width = (2.0 * reach).max(1e-9);
        // `as` saturates; so do the neighbours below.
        let place = |v: f64| {
            let at = v / width;
            let cell = at.floor() as i64;
            let side = if at - at.floor() < 0.5 { -1 } else { 1 };
            (cell, side)
        };
        let ((cx, sx), (cy, sy)) = (place(c.origin.x), place(c.origin.y));
        let mut key = CellKey {
            c: c.c,
            size: quantise(c.size),
            dir: [quantise(c.dir.x), quantise(c.dir.y)],
            cell: [cx, cy],
        };
        // The same characters, one glyph: `out[at..at + n]` and no more.
        let same_glyph = |out: &[RawChar], at: usize| {
            let n = glyph.len();
            let Some(kept) = out.get(at..at + n) else {
                return false;
            };
            let serial = kept[0].glyph_serial;
            kept.iter()
                .zip(&glyph)
                .all(|(k, g)| k.c == g.c && k.glyph_serial == serial)
                && out
                    .get(at + n)
                    .is_none_or(|after| after.glyph_serial != serial || after.first)
        };
        let mut copy_of = None;
        'search: for (dx, dy) in [(0, 0), (sx, 0), (0, sy), (sx, sy)] {
            key.cell = [cx.saturating_add(dx), cy.saturating_add(dy)];
            let mut at = heads.get(&key).copied().unwrap_or(usize::MAX);
            for _ in 0..MAX_CELL_SCAN {
                let Some(other) = out.get(at) else { break };
                if other.glyph_serial != c.glyph_serial
                    && (other.origin - c.origin).hypot() <= reach
                    && dot(other.dir, c.dir) >= PARALLEL
                    && same_glyph(&out, at)
                {
                    copy_of = Some(at);
                    break 'search;
                }
                at = next.get(at).copied().unwrap_or(usize::MAX);
            }
        }
        match copy_of {
            Some(at) => {
                for (kept, copy) in out.iter_mut().skip(at).zip(&glyph) {
                    kept.visible |= copy.visible;
                }
            }
            None => {
                key.cell = [cx, cy];
                next.push(heads.insert(key, out.len()).unwrap_or(usize::MAX));
                out.push(c);
                for &more in &glyph[1..] {
                    next.push(usize::MAX);
                    out.push(more);
                }
            }
        }
    }
    out
}

/// A line being built.
struct LineBuild {
    dir: Vec2,
    base: Point,
    size: f64,
    /// Along-line start, end and size of the last character added.
    prev_s: f64,
    prev_e: f64,
    prev_size: f64,
    /// The leftmost along-line position of its characters.
    min: f64,
    /// Along-line start of its first character.
    start: f64,
    /// The run layout (`span_blocks`): a run that moves back over the one
    /// before starts a new line (spec 04, 12 C2).
    spans: bool,
    /// The text-showing operator of the last character, and the along-line
    /// extent of its characters on this line.
    run_show: u64,
    run_s: f64,
    run_e: f64,
    /// A left-to-right run joined at the front of the line, on probation
    /// until it ends: if it reaches into the line's characters it leaves
    /// (spec 04, 12 C5.2).
    front: Option<Front>,
    chars: Vec<RawChar>,
}

/// A run joined at a line's front, and the line as it was before.
#[derive(Clone, Copy)]
struct Front {
    show: u64,
    /// Index of its first character in `chars`.
    at: usize,
    /// The line's leftmost position before it.
    limit: f64,
    saved: (f64, f64, f64, f64, u64, f64, f64),
}

/// What happened to a character offered to a line.
enum Join {
    Joined,
    Refused,
    /// The run on probation at the line's front reached into it: its
    /// characters (the offered one last) leave the line.
    Ejected(Vec<RawChar>),
}

impl LineBuild {
    fn new(c: RawChar, spans: bool) -> Self {
        let s = dot(c.dir, c.origin.to_vec2());
        let e = dot(c.dir, c.end.to_vec2());
        Self {
            dir: c.dir,
            base: c.origin,
            size: c.size,
            prev_s: s,
            prev_e: e,
            prev_size: c.size,
            min: s.min(e),
            start: s,
            spans,
            run_show: c.show,
            run_s: s.min(e),
            run_e: s.max(e),
            front: None,
            chars: vec![c],
        }
    }

    /// Adds `c` if it continues the line (spec 04, 3.5).
    fn join(&mut self, c: RawChar) -> Join {
        if self.front.is_some_and(|f| f.show != c.show) {
            self.front = None;
        }
        if dot(self.dir, c.dir) < PARALLEL {
            return Join::Refused;
        }
        let shift = cross(self.dir, c.origin - self.base).abs();
        if shift > MAX_BASELINE_SHIFT * self.size.max(c.size) {
            return Join::Refused;
        }
        let s = dot(self.dir, c.origin.to_vec2());
        let e = dot(self.dir, c.end.to_vec2());
        let rtl = right_to_left(c.c);
        // Front joins (spec 04, 12 C5, C5a), in ems of the smaller of the
        // glyph and the line.
        let front_em = c.size.min(self.size);
        // A run on probation at the line's front leaves it as soon as one
        // of its glyphs, joining or not, ends past the allowance into the
        // line's characters.
        if let Some(front) = self.front {
            if (s.max(e) - front.limit) / front_em > FRONT_TOUCH {
                return self.eject(front, c);
            }
        }
        // In ems of the smaller of the two glyphs (spec 04, 12 C3.1).
        let em = c.size.min(self.prev_size);
        let forward = (s - self.prev_e) / em;
        // Drawn back to the left of everything on the line (right-to-left
        // text with the pen moving left): how far left of it.
        let left = (self.min - e) / em;
        let near = |gap: f64| (-MAX_OVERLAP..LINE_BREAK_GAP).contains(&gap);
        // A right-to-left glyph may overlap like any other (about an em,
        // 3.5 rule 3); a left-to-right one drawn back is a front join.
        let touch = if rtl { MAX_OVERLAP } else { FRONT_TOUCH };
        let leftward = (-touch..LINE_BREAK_GAP).contains(&left);
        if !(near(forward) || (forward < -MAX_OVERLAP && leftward)) {
            return Join::Refused;
        }
        let new_run = c.show != self.run_show;
        if self.spans
            && new_run
            && (self.run_e - s) / c.size > RUN_MOVED_BACK
            && (e - self.run_s) / c.size > RUN_OVERLAP
        {
            return Join::Refused;
        }
        // A left-to-right run whose first glyph starts before the line's
        // leftmost character, however short the line, is a front-join
        // candidate: it joins only within the allowance, and stays on
        // probation until it ends.
        if self.front.is_none() && new_run && !rtl && s < self.min {
            if (e.max(s) - self.min) / front_em > FRONT_TOUCH {
                return Join::Refused;
            }
            self.front = Some(Front {
                show: c.show,
                at: self.chars.len(),
                limit: self.min,
                saved: (
                    self.prev_s,
                    self.prev_e,
                    self.prev_size,
                    self.min,
                    self.run_show,
                    self.run_s,
                    self.run_e,
                ),
            });
        }
        if new_run {
            (self.run_show, self.run_s, self.run_e) = (c.show, s.min(e), s.max(e));
        } else {
            (self.run_s, self.run_e) = (self.run_s.min(s.min(e)), self.run_e.max(s.max(e)));
        }
        self.prev_s = s;
        self.prev_e = e;
        self.prev_size = c.size;
        self.min = self.min.min(s.min(e));
        self.chars.push(c);
        Join::Joined
    }

    /// Takes the run on probation at the front back out of the line, the
    /// line as it was before it, and gives its characters and `c` back.
    fn eject(&mut self, front: Front, c: RawChar) -> Join {
        let (prev_s, prev_e, prev_size, min, run_show, run_s, run_e) = front.saved;
        (self.prev_s, self.prev_e, self.prev_size, self.min) = (prev_s, prev_e, prev_size, min);
        (self.run_show, self.run_s, self.run_e) = (run_show, run_s, run_e);
        self.front = None;
        let mut out = self.chars.split_off(front.at.min(self.chars.len()));
        out.push(c);
        Join::Ejected(out)
    }
}

/// A laid-out line: its characters in reading order.
struct Laid {
    dir: Vec2,
    chars: Vec<RawChar>,
}

/// Whether a new line `new` after `prev` (the last line of the block,
/// whose leftmost start is `block_start`) starts a block (spec 04, 3.5).
fn starts_block(prev: &LineBuild, new: &LineBuild, block_start: f64) -> bool {
    let distance = cross(prev.dir, new.base - prev.base).abs() / prev.size.min(new.size);
    dot(prev.dir, new.dir) < PARALLEL
        || distance > BLOCK_GAP
        || (distance >= 0.5 && (new.start - block_start) / new.size >= INDENT)
}

/// Groups characters, in drawing order, into blocks of lines; `spans`: the
/// run layout, where a run moved back over the one before starts a line.
fn layout<'c>(chars: impl Iterator<Item = &'c RawChar>, spans: bool) -> Vec<Vec<Laid>> {
    let mut blocks: Vec<Vec<LineBuild>> = Vec::new();
    let mut block_start = 0.0;
    let mut current: Option<LineBuild> = None;
    let mut chars = chars.copied();
    // Characters a line gave back (a run that left its front), first.
    let mut pending: std::collections::VecDeque<RawChar> = std::collections::VecDeque::new();
    while let Some(c) = pending.pop_front().or_else(|| chars.next()) {
        let c = match current.as_mut().map(|line| line.join(c)) {
            Some(Join::Joined) => continue,
            Some(Join::Refused) | None => c,
            Some(Join::Ejected(run)) => {
                let mut run = run.into_iter();
                let Some(first) = run.next() else { continue };
                for (i, rest) in run.enumerate() {
                    pending.insert(i, rest);
                }
                first
            }
        };
        let new = LineBuild::new(c, spans);
        if let Some(done) = current.take() {
            match blocks.last_mut() {
                Some(lines) => lines.push(done),
                None => blocks.push(vec![done]),
            }
        }
        let fresh = match blocks.last().and_then(|lines| lines.last()) {
            Some(prev) => starts_block(prev, &new, block_start),
            None => true,
        };
        if fresh {
            blocks.push(Vec::new());
            block_start = new.start;
        } else {
            block_start = f64::min(block_start, new.start);
        }
        current = Some(new);
    }
    if let Some(done) = current {
        match blocks.last_mut() {
            Some(lines) => lines.push(done),
            None => blocks.push(vec![done]),
        }
    }
    blocks
        .into_iter()
        .filter(|lines| !lines.is_empty())
        .map(|lines| lines.into_iter().map(finish_line).collect())
        .collect()
}

/// Whether `c` is a strong right-to-left character (Bidi_Class R or AL).
/// Nothing below U+0590 is, which spares the table lookup for Latin,
/// Greek and Cyrillic text.
fn right_to_left(c: char) -> bool {
    c >= '\u{590}' && matches!(bidi_class(c), BidiClass::R | BidiClass::AL)
}

/// Characters of scripts written without spaces between words, around
/// which no space is inferred (CJK ideographs and punctuation, kana,
/// Bopomofo; not Hangul).
fn spaceless(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x2FFF
        | 0x3000..=0x303F
        | 0x3040..=0x30FF
        | 0x3100..=0x312F
        | 0x31A0..=0x31FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF01..=0xFF0F
        | 0xFF1A..=0xFF20
        | 0xFF3B..=0xFF40
        | 0xFF5B..=0xFF9F
        | 0x20000..=0x3FFFF
    )
}

/// A line's characters in drawing order, except that position along the
/// line orders whole runs placed after one another, and the characters of
/// a run whose pen moves left (right-to-left text drawn in logical order);
/// characters of one run are never moved between those of another, and
/// characters at the same position keep their drawing order (spec 04, 12
/// C3.2).
fn in_reading_order(chars: Vec<RawChar>, d: Vec2) -> Vec<RawChar> {
    let along = |c: &RawChar| dot(d, c.origin.to_vec2());
    let mut runs: Vec<Vec<RawChar>> = Vec::new();
    for c in chars {
        match runs.last_mut() {
            Some(run) if run.last().is_some_and(|last| last.show == c.show) => run.push(c),
            _ => runs.push(vec![c]),
        }
    }
    for run in &mut runs {
        let mut reach = f64::NEG_INFINITY;
        let moves_left = run.iter().any(|c| {
            let s = along(c);
            let back = s < reach - SAME_POSITION * c.size;
            reach = reach.max(s);
            back
        });
        // Only right-to-left text drawn in logical order (the pen moving
        // left, 3.6) is put in visual order here; a left-to-right run keeps
        // its drawing order (spec 04, 12 C3.2).
        if moves_left && run.iter().any(|c| right_to_left(c.c)) {
            run.sort_by(|a, b| along(a).total_cmp(&along(b)));
        }
    }
    let start = |run: &Vec<RawChar>| run.iter().map(along).fold(f64::INFINITY, f64::min);
    runs.sort_by(|a, b| start(a).total_cmp(&start(b)));
    runs.into_iter().flatten().collect()
}

/// Orders a line's characters along it, infers spaces from gaps (spec 04,
/// 3.4), and puts right-to-left text in logical order (3.6).
fn finish_line(line: LineBuild) -> Laid {
    let d = line.dir;
    let along = |p: Point| dot(d, p.to_vec2());
    let chars = in_reading_order(line.chars, d);
    let mut out: Vec<RawChar> = Vec::with_capacity(chars.len() + chars.len() / 4);
    let mut reach = f64::NEG_INFINITY;
    for c in chars {
        let s = along(c.origin);
        if let Some(prev) = out.last() {
            let gap = (s - reach) / c.size;
            if gap >= SPACE_GAP
                && !prev.c.is_whitespace()
                && !c.c.is_whitespace()
                && !spaceless(prev.c)
                && !spaceless(c.c)
            {
                // From the end of the advance before to this glyph's start,
                // as high as this glyph's quad; part of this glyph's run.
                let at = c.origin + d * (reach - s);
                let up = c.quad[0] - c.origin;
                let down = c.quad[2] - c.origin;
                out.push(RawChar {
                    c: ' ',
                    origin: at,
                    end: c.origin,
                    quad: [at + up, c.origin + up, at + down, c.origin + down],
                    glyph: false,
                    first: true,
                    synthetic: true,
                    ..c
                });
            }
        }
        reach = reach.max(along(c.end)).max(s);
        out.push(c);
    }
    reorder(&mut out);
    Laid { dir: d, chars: out }
}

// ------------------------------------------------------------------ bidirectional text

/// Puts a line drawn in visual order into logical order with the Unicode
/// Bidirectional Algorithm (UAX #9, owner decision T2), for one paragraph
/// without explicit embeddings: W1–W7, N1–N2, I1–I2 and L1 on the visual
/// sequence, then the reversals of L2 (their own inverse here), with L4
/// mirroring of characters at odd levels.
fn reorder(chars: &mut Vec<RawChar>) {
    if !chars.iter().any(|c| right_to_left(c.c)) {
        return;
    }
    let original: Vec<BidiClass> = chars.iter().map(|c| bidi_class(c.c)).collect();
    let levels = resolve_levels(&original, paragraph_rtl(&original));
    let mut order: Vec<usize> = (0..chars.len()).collect();
    let highest = levels.iter().copied().max().unwrap_or(0);
    let lowest_odd = levels
        .iter()
        .copied()
        .filter(|l| l % 2 == 1)
        .min()
        .unwrap_or(highest.saturating_add(1));
    let mut level = highest;
    while level >= lowest_odd && level > 0 {
        let mut i = 0;
        while i < order.len() {
            if levels[order[i]] >= level {
                let start = i;
                while i < order.len() && levels[order[i]] >= level {
                    i += 1;
                }
                order[start..i].reverse();
            } else {
                i += 1;
            }
        }
        level -= 1;
    }
    // The characters of one glyph (a decomposed presentation form, a
    // multi-character name) are already in logical order: put back any
    // the reversals turned round.
    let serial = |i: usize| chars[i].glyph_serial;
    let mut i = 0;
    while i < order.len() {
        let start = i;
        while i + 1 < order.len()
            && serial(order[i]) != 0
            && serial(order[i + 1]) == serial(order[i])
        {
            i += 1;
        }
        order[start..=i].sort_unstable();
        i += 1;
    }
    let reordered: Vec<RawChar> = order
        .into_iter()
        .map(|i| {
            let mut c = chars[i];
            if levels[i] % 2 == 1 {
                c.run.rtl = true;
                if let Some(mirror) = bidi_mirror(c.c) {
                    c.c = mirror;
                }
            }
            c
        })
        .collect();
    *chars = reordered;
}

/// The paragraph direction of a line in visual order: right to left when
/// its strong characters begin and end right to left (an RTL paragraph's
/// logical start is at the right end), left to right when they begin and
/// end left to right, else by the majority of strong characters.
fn paragraph_rtl(classes: &[BidiClass]) -> bool {
    use BidiClass::*;
    let rtl = |c: &BidiClass| matches!(c, R | AL);
    let strong = |c: &&BidiClass| matches!(c, L | R | AL);
    match (
        classes.iter().find(strong),
        classes.iter().rev().find(strong),
    ) {
        (Some(f), Some(l)) if rtl(f) == rtl(l) => rtl(f),
        _ => {
            let right = classes.iter().filter(|c| rtl(c)).count();
            let left = classes.iter().filter(|c| **c == L).count();
            right > left
        }
    }
}

/// The embedding level of each character (UAX #9 W1–W7, N1–N2, I1–I2, L1).
fn resolve_levels(original: &[BidiClass], rtl: bool) -> Vec<u8> {
    use BidiClass::*;
    let n = original.len();
    let edge = if rtl { R } else { L };
    // Explicit formatting characters and BN read as neutrals.
    let mut t: Vec<BidiClass> = original
        .iter()
        .map(|&c| match c {
            LRE | RLE | LRO | RLO | PDF | LRI | RLI | FSI | PDI | BN => ON,
            c => c,
        })
        .collect();
    // W1
    let mut prev = edge;
    for x in t.iter_mut() {
        if *x == NSM {
            *x = prev;
        }
        prev = *x;
    }
    // W2, W3
    let mut strong = edge;
    for x in t.iter_mut() {
        match *x {
            L | R | AL => strong = *x,
            EN if strong == AL => *x = AN,
            _ => {}
        }
        if *x == AL {
            *x = R;
        }
    }
    // W4
    for i in 1..n.saturating_sub(1) {
        let (a, b) = (t[i - 1], t[i + 1]);
        if t[i] == ES && a == EN && b == EN {
            t[i] = EN;
        } else if t[i] == CS && a == b && matches!(a, EN | AN) {
            t[i] = a;
        }
    }
    // W5
    let mut i = 0;
    while i < n {
        if t[i] == ET {
            let start = i;
            while i < n && t[i] == ET {
                i += 1;
            }
            if (start > 0 && t[start - 1] == EN) || (i < n && t[i] == EN) {
                t[start..i].fill(EN);
            }
        } else {
            i += 1;
        }
    }
    // W6, W7
    let mut strong = edge;
    for x in t.iter_mut() {
        if matches!(*x, ES | ET | CS) {
            *x = ON;
        }
        match *x {
            L | R => strong = *x,
            EN if strong == L => *x = L,
            _ => {}
        }
    }
    // N1, N2
    let neutral = |c: BidiClass| matches!(c, B | S | WS | ON);
    let direction = |c: BidiClass| if c == L { L } else { R };
    let mut i = 0;
    while i < n {
        if neutral(t[i]) {
            let start = i;
            while i < n && neutral(t[i]) {
                i += 1;
            }
            let before = if start == 0 {
                edge
            } else {
                direction(t[start - 1])
            };
            let after = if i == n { edge } else { direction(t[i]) };
            t[start..i].fill(if before == after { before } else { edge });
        } else {
            i += 1;
        }
    }
    // I1, I2
    let base = u8::from(rtl);
    let mut levels: Vec<u8> = t
        .iter()
        .map(|&c| match (rtl, c) {
            (false, R) => 1,
            (false, AN | EN) => 2,
            (true, L | EN | AN) => 2,
            _ => base,
        })
        .collect();
    // L1: separators, and whitespace before them or at the end of the line.
    let blank = |c: BidiClass| {
        matches!(
            c,
            WS | BN | LRE | RLE | LRO | RLO | PDF | LRI | RLI | FSI | PDI
        )
    };
    let mut trailing = true;
    for i in (0..n).rev() {
        if matches!(original[i], S | B) {
            levels[i] = base;
            trailing = true;
        } else if trailing && blank(original[i]) {
            levels[i] = base;
        } else {
            trailing = false;
        }
    }
    levels
}

// ------------------------------------------------------------------ the result

fn public_line(dir: Vec2, chars: &[RawChar]) -> TextLine {
    let mut points = Vec::with_capacity(chars.len() * 4);
    for c in chars {
        points.extend_from_slice(&c.quad);
    }
    let b = bounds_of(&points);
    TextLine {
        bbox: [b.x0, b.y0, b.x1, b.y1],
        dir: [dir.x, dir.y],
        chars: chars.iter().map(RawChar::public).collect(),
    }
}

fn public_block(lines: Vec<TextLine>) -> TextBlock {
    let mut bbox = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for line in &lines {
        bbox[0] = bbox[0].min(line.bbox[0]);
        bbox[1] = bbox[1].min(line.bbox[1]);
        bbox[2] = bbox[2].max(line.bbox[2]);
        bbox[3] = bbox[3].max(line.bbox[3]);
    }
    TextBlock { bbox, lines }
}

fn hyphen(c: char) -> bool {
    matches!(c, '\u{2D}' | '\u{AD}' | '\u{2010}' | '\u{2011}')
}

/// One unit of a search needle.
#[derive(Clone, Copy, PartialEq)]
enum Needle {
    Char(char),
    /// A run of whitespace, matching one or more whitespace characters.
    Space,
}

/// One character of the search haystack: where it lies (block, line,
/// character), or `None` for the space a line break reads as.
struct Hay {
    c: char,
    at: Option<(usize, usize, usize)>,
}

impl StructuredText {
    fn build(chars: Vec<RawChar>, fonts: Vec<TextFont>) -> Self {
        // The run layout keeps each run's own characters, overprinted
        // copies included (spec 04, 12 C1); the others read a copy once.
        let spans_laid = layout(chars.iter().filter(|c| c.visible), true);
        let chars = without_overprints(chars);
        let public = |lines: &[Laid]| {
            public_block(lines.iter().map(|l| public_line(l.dir, &l.chars)).collect())
        };
        let unclipped = layout(chars.iter(), false)
            .iter()
            .map(|b| public(b))
            .collect();
        let blocks = layout(chars.iter().filter(|c| c.visible), false)
            .iter()
            .map(|b| public(b))
            .collect();
        let mut spans = Vec::with_capacity(spans_laid.len());
        for lines in &spans_laid {
            let mut span_lines = Vec::new();
            for line in lines {
                let mut start = 0;
                for i in 1..=line.chars.len() {
                    if i == line.chars.len() || line.chars[i].run != line.chars[i - 1].run {
                        span_lines.push(public_line(line.dir, &line.chars[start..i]));
                        start = i;
                    }
                }
            }
            spans.push(public_block(span_lines));
        }
        Self {
            fonts,
            blocks,
            spans,
            unclipped,
        }
    }

    pub fn blocks(&self) -> &[TextBlock] {
        &self.blocks
    }

    pub fn span_blocks(&self) -> &[TextBlock] {
        &self.spans
    }

    pub fn unclipped_blocks(&self) -> &[TextBlock] {
        &self.unclipped
    }

    pub fn fonts(&self) -> &[TextFont] {
        &self.fonts
    }

    pub fn as_text(&self) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            for line in &block.lines {
                out.extend(line.chars.iter().map(|c| c.c));
                out.push('\n');
            }
            out.push('\n');
        }
        out
    }

    /// The summaries' text, one line per reading line (owner decision T5):
    /// the runs of one line of `blocks()` joined, overprinted copies read
    /// once (spec 04, 12 C1), which is the reading text.
    pub fn spans_text(&self) -> String {
        self.as_text()
    }

    pub fn line_summaries(&self) -> Vec<LineSummary> {
        // Truncated toward zero, saturating (`as`), after rounding off
        // floating-point noise (a size of 29.9999997 from a matrix written
        // with eight decimals is 30).
        let int = |v: f64| ((v * 1e4).round() / 1e4) as i32;
        let mut out = Vec::new();
        for line in self.spans.iter().flat_map(|b| &b.lines) {
            let Some(first) = line.chars.first() else {
                continue;
            };
            let font = self.fonts.get(first.font);
            let [x0, y0, x1, y1] = line.bbox;
            out.push(LineSummary {
                text: line.chars.iter().map(|c| c.c).collect(),
                x: int(x0),
                y: int(first.origin[1]),
                top: int(y0),
                w: int(x1 - x0),
                h: int(y1 - y0),
                size: int(first.size),
                family: match font {
                    Some(f) if f.monospace => "monospace",
                    Some(f) if f.serif => "serif",
                    _ => "sans-serif",
                },
                weight: if font.is_some_and(|f| f.bold) {
                    "bold"
                } else {
                    "normal"
                },
                style: if font.is_some_and(|f| f.italic) {
                    "italic"
                } else {
                    "normal"
                },
            });
        }
        out
    }

    pub fn search(&self, needle: &str, max_quads: usize) -> SearchResult {
        self.find(needle, |_, quads, next| {
            max_quads == 0 || quads.saturating_add(next) > max_quads
        })
    }

    pub fn search_hits(&self, needle: &str, max_hits: usize) -> SearchResult {
        self.find(needle, |hits, _, _| hits >= max_hits)
    }

    /// Every match of `needle` (spec 04, 2.5), in order, until `full(hits
    /// so far, quads so far, the next hit's quads)` says the next does not
    /// fit.
    fn find(&self, needle: &str, full: impl Fn(usize, usize, usize) -> bool) -> SearchResult {
        let mut pattern: Vec<Needle> = Vec::new();
        for c in needle.chars() {
            if !search_space(c) {
                pattern.push(Needle::Char(c));
            } else if pattern.last() != Some(&Needle::Space) {
                pattern.push(Needle::Space);
            }
        }
        let mut result = SearchResult::default();
        if pattern.is_empty() {
            return result;
        }
        let hay = self.haystack();
        let mut quads = 0usize;
        let mut i = 0;
        while i < hay.len() {
            let Some(end) = match_at(&hay, i, &pattern) else {
                // A needle that starts with whitespace takes a whole run:
                // failing at a run's start, it fails inside it too.
                let run = pattern.first() == Some(&Needle::Space)
                    && hay.get(i).is_some_and(|h| search_space(h.c));
                i += 1;
                while run && hay.get(i).is_some_and(|h| search_space(h.c)) {
                    i += 1;
                }
                continue;
            };
            let hit = self.quads(hay.get(i..end).unwrap_or_default());
            // A match of nothing drawn (the space a line break reads as)
            // has nothing to show.
            if hit.is_empty() {
                i = end.max(i + 1);
                continue;
            }
            if full(result.hits.len(), quads, hit.len()) {
                result.truncated = true;
                break;
            }
            quads += hit.len();
            result.hits.push(hit);
            i = end.max(i + 1);
        }
        result
    }

    /// The characters of `unclipped_blocks` in order, a line break read as
    /// a space, a hyphen ending a line joining it to the next.
    fn haystack(&self) -> Vec<Hay> {
        let mut hay: Vec<Hay> = Vec::new();
        let mut joined = false;
        for (b, block) in self.unclipped.iter().enumerate() {
            for (l, line) in block.lines.iter().enumerate() {
                if !hay.is_empty() && !joined {
                    hay.push(Hay { c: ' ', at: None });
                }
                hay.extend(line.chars.iter().enumerate().map(|(i, c)| Hay {
                    c: c.c,
                    at: Some((b, l, i)),
                }));
                joined = line.chars.last().is_some_and(|c| hyphen(c.c));
                if joined {
                    hay.pop();
                }
            }
        }
        hay
    }

    /// The quads of a match: one per stretch of it on one line that runs
    /// one way without a wide gap, covering its characters' quads.
    fn quads(&self, matched: &[Hay]) -> Vec<[f64; 8]> {
        struct Stretch {
            at: (usize, usize),
            dir: Vec2,
            up: Vec2,
            along: (f64, f64),
            height: (f64, f64),
            last: f64,
            sign: f64,
        }
        let finish = |s: &Stretch| -> [f64; 8] {
            let p = |a: f64, h: f64| s.dir * a + s.up * h;
            let (ul, ur) = (p(s.along.0, s.height.1), p(s.along.1, s.height.1));
            let (ll, lr) = (p(s.along.0, s.height.0), p(s.along.1, s.height.0));
            [ul.x, ul.y, ur.x, ur.y, ll.x, ll.y, lr.x, lr.y]
        };
        let mut out = Vec::new();
        let mut current: Option<Stretch> = None;
        for h in matched {
            let Some((b, l, i)) = h.at else {
                continue;
            };
            let Some(line) = self.unclipped.get(b).and_then(|blk| blk.lines.get(l)) else {
                continue;
            };
            let Some(c) = line.chars.get(i) else {
                continue;
            };
            let dir = Vec2::new(line.dir[0], line.dir[1]);
            // Up on the page for a line read left to right.
            let up = Vec2::new(dir.y, -dir.x);
            let corners = [
                Vec2::new(c.quad[0], c.quad[1]),
                Vec2::new(c.quad[2], c.quad[3]),
                Vec2::new(c.quad[4], c.quad[5]),
                Vec2::new(c.quad[6], c.quad[7]),
            ];
            let span = |axis: Vec2| {
                corners
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                        let v = dot(axis, *p);
                        (lo.min(v), hi.max(v))
                    })
            };
            let (along, height) = (span(dir), span(up));
            let s = dot(dir, Vec2::new(c.origin[0], c.origin[1]));
            let size = c.size.max(f64::MIN_POSITIVE);
            let continues = current.as_ref().is_some_and(|st| {
                let gap = (along.0 - st.along.1).max(st.along.0 - along.1) / size;
                st.at == (b, l) && gap <= QUAD_GAP && st.sign * (s - st.last) >= -0.01 * size
            });
            match current.as_mut() {
                Some(st) if continues => {
                    let step = s - st.last;
                    if st.sign == 0.0 && step.abs() > 0.01 * size {
                        st.sign = step.signum();
                    }
                    st.along = (st.along.0.min(along.0), st.along.1.max(along.1));
                    st.height = (st.height.0.min(height.0), st.height.1.max(height.1));
                    st.last = s;
                }
                _ => {
                    if let Some(st) = current.take() {
                        out.push(finish(&st));
                    }
                    current = Some(Stretch {
                        at: (b, l),
                        dir,
                        up,
                        along,
                        height,
                        last: s,
                        sign: 0.0,
                    });
                }
            }
        }
        if let Some(st) = current {
            out.push(finish(&st));
        }
        out
    }
}

/// Whitespace for search (spec 04, 2.5): tab, line feed, vertical tab,
/// form feed, carriage return and the space separators, and the line and
/// paragraph separators; not U+0085 (NEL), which JavaScript, and so the
/// app's query trimming, does not take as whitespace.
fn search_space(c: char) -> bool {
    c.is_whitespace() && c != '\u{85}'
}

/// Where a match of `pattern` starting at `i` ends, if one does.
fn match_at(hay: &[Hay], i: usize, pattern: &[Needle]) -> Option<usize> {
    let mut j = i;
    for unit in pattern {
        match unit {
            Needle::Char(c) => {
                if hay.get(j)?.c != *c {
                    return None;
                }
                j += 1;
            }
            Needle::Space => {
                if !search_space(hay.get(j)?.c) {
                    return None;
                }
                while hay.get(j).is_some_and(|h| search_space(h.c)) {
                    j += 1;
                }
            }
        }
    }
    Some(j)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_constants_lie_in_the_spec_bands() {
        const { assert!(SPACE_GAP > 0.10 && SPACE_GAP < 0.25) };
        const { assert!(BLOCK_GAP > 1.3 && BLOCK_GAP < 3.0) };
        const { assert!(INDENT > 0.1 && INDENT < 1.0) };
        const { assert!(MAX_BASELINE_SHIFT > 0.4 && MAX_BASELINE_SHIFT < 1.0) };
        const { assert!(OVERPRINT >= 0.01 && OVERPRINT <= 0.05) };
    }

    #[test]
    fn font_names_decode_lossily() {
        assert_eq!(text_font_name(b"ABCDEF+Times"), "ABCDEF+Times");
        assert_eq!(text_font_name(b"\xff"), "\u{fffd}");
        assert_eq!(without_subset(b"ABCDEF+Times"), b"Times");
        assert_eq!(without_subset(b"Abcdef+Times"), b"Abcdef+Times");
        assert_eq!(without_subset(b"AB+"), b"AB+");
    }

    /// A drawn character for layout tests: `size` pt, advancing half an em.
    fn raw(c: char, x: f64, y: f64, size: f64, serial: u64, dir: Vec2) -> RawChar {
        let origin = Point::new(x, y);
        RawChar {
            c,
            origin,
            end: origin + dir * (0.5 * size),
            dir,
            quad: [origin; 4],
            size,
            font: 0,
            run: RunKey {
                font: 0,
                matrix: [0; 4],
                fill: None,
                mode: 0,
                object: 0,
                rtl: false,
            },
            visible: true,
            glyph: true,
            first: true,
            glyph_serial: serial,
            show: serial,
            synthetic: false,
        }
    }

    #[test]
    fn overprints_at_hostile_coordinates_do_not_overflow() {
        let right = Vec2::new(1.0, 0.0);
        let chars = vec![
            raw('a', 1e300, -1e300, 1e-300, 1, right),
            raw('a', 1e300, -1e300, 1e-300, 2, right),
            raw('a', f64::MAX, f64::MIN, f64::MIN_POSITIVE, 3, right),
            raw('a', f64::MIN, f64::MAX, f64::MIN_POSITIVE, 4, right),
        ];
        assert_eq!(without_overprints(chars).len(), 3);
    }

    #[test]
    fn a_visible_copy_of_a_glyph_of_two_characters_shows_both() {
        // Spec 04, 12 C6: "ff" drawn clipped away, then drawn visibly at
        // the same place, then "x": every character of the glyph is kept
        // once, and visible.
        let right = Vec2::new(1.0, 0.0);
        let mut chars = Vec::new();
        for (serial, visible) in [(1, false), (2, true)] {
            for (i, c) in ['f', 'f'].into_iter().enumerate() {
                let mut ch = raw(c, 50.0, 100.0, 12.0, serial, right);
                (ch.first, ch.visible, ch.show) = (i == 0, visible, serial);
                chars.push(ch);
            }
        }
        chars.push(raw('x', 56.0, 100.0, 12.0, 3, right));
        let kept = without_overprints(chars);
        let read: String = kept.iter().map(|c| c.c).collect();
        assert_eq!(read, "ffx");
        assert!(kept.iter().all(|c| c.visible), "{kept:?}");
    }

    #[test]
    fn overprints_in_many_directions_at_one_spot_stay_linear() {
        let n: u32 = 200_000;
        let chars: Vec<RawChar> = (0..n)
            .map(|i| {
                let angle = f64::from(i) * std::f64::consts::TAU / f64::from(n);
                let dir = Vec2::new(angle.cos(), angle.sin());
                raw('a', 100.0, 100.0, 10.0, u64::from(i) + 1, dir)
            })
            .collect();
        let started = std::time::Instant::now();
        let kept = without_overprints(chars);
        assert!(!kept.is_empty() && kept.len() < 200_000);
        assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
    }

    #[test]
    fn a_long_whitespace_run_is_searched_in_linear_time() {
        let right = Vec2::new(1.0, 0.0);
        let mut chars: Vec<RawChar> = (0..200_000u32)
            .map(|i| {
                raw(
                    ' ',
                    f64::from(i) * 5.0,
                    100.0,
                    10.0,
                    u64::from(i) + 1,
                    right,
                )
            })
            .collect();
        chars.push(raw('x', 1e6, 100.0, 10.0, 300_000, right));
        let text = StructuredText::build(chars, Vec::new());
        let started = std::time::Instant::now();
        assert!(text.search(" y", MAX_SEARCH_QUADS).hits.is_empty());
        assert_eq!(text.search(" x", MAX_SEARCH_QUADS).hits.len(), 1);
        assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
    }

    #[test]
    fn deeply_nested_marked_content_under_actual_text_stays_linear() {
        let depth = 100_000;
        let mut content = String::from("/Span << /ActualText (Deep) >> BDC ");
        for _ in 0..depth {
            content.push_str("/P << >> BDC 0 0 1 1 re f ");
        }
        for _ in 0..=depth {
            content.push_str("EMC ");
        }
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R >>".to_string(),
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ),
        ];
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
        }
        let xref = pdf.len();
        pdf.extend(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend(format!("{offset:010} 00000 n \n").bytes());
        }
        pdf.extend(
            format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").bytes(),
        );
        let view = ViewDocument::open(pdf, "").unwrap();
        let started = std::time::Instant::now();
        let text = view.structured_text(0).unwrap();
        assert_eq!(text.as_text(), "Deep\n\n");
        assert!(started.elapsed().as_secs() < 10, "{:?}", started.elapsed());
    }

    /// One Letter page showing `content` with Helvetica as /F1.
    fn helvetica_page(content: &str) -> ViewDocument {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
                .to_string(),
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
                .to_string(),
        ];
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
        }
        let xref = pdf.len();
        pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for offset in offsets {
            pdf.extend(format!("{offset:010} 00000 n \n").bytes());
        }
        pdf.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .bytes(),
        );
        ViewDocument::open(pdf, "").unwrap()
    }

    fn span_texts(text: &StructuredText) -> Vec<String> {
        (text.span_blocks().iter())
            .flat_map(|b| &b.lines)
            .map(|l| {
                l.chars
                    .iter()
                    .map(|c| c.c)
                    .collect::<String>()
                    .trim()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn overprinted_runs_keep_their_own_lines_in_the_run_layout() {
        // Spec 04, 12 C1.
        let run = "BT /F1 12 Tf 300 392 Td (Same) Tj ET";
        let text = helvetica_page(&format!("{run} {run}"))
            .structured_text(0)
            .unwrap();
        assert_eq!(span_texts(&text), ["Same", "Same"]);
        assert_eq!(text.line_summaries().len(), 2);
        assert_eq!(text.as_text(), "Same\n\n");
        assert_eq!(text.spans_text(), "Same\n\n");
        assert_eq!(text.search("Same", MAX_SEARCH_QUADS).hits.len(), 1);
    }

    #[test]
    fn a_run_moved_back_over_the_one_before_starts_a_line_in_the_run_layout() {
        // Spec 04, 12 C2: "Hello" moved right and down onto "World"'s line.
        for world_y in ["685.6", "682.72"] {
            let text = helvetica_page(&format!(
                "BT /F1 1 Tf 12 0 0 12 82 680 Tm (Hello) Tj 12 0 0 12 72 {world_y} Tm (World) Tj ET"
            ))
            .structured_text(0)
            .unwrap();
            assert_eq!(span_texts(&text), ["Hello", "World"], "{world_y}");
            let hello = &text.span_blocks()[0].lines[0];
            // Hello is 27.336 pt wide at 12 pt from x 82.
            assert!(
                hello.bbox[0] >= 82.0 - 0.01 && hello.bbox[2] <= 109.336 + 0.01,
                "{world_y}: {:?}",
                hello.bbox
            );
            for c in &hello.chars {
                for x in [c.quad[0], c.quad[2], c.quad[4], c.quad[6]] {
                    assert!((82.0 - 0.01..=109.346).contains(&x), "{c:?}");
                }
            }
        }
        // Right-to-left text drawn glyph by glyph, the pen moving left,
        // stays one run line per show only where runs change.
        let text =
            helvetica_page("BT /F1 10 Tf 105.56 100 Td (a) Tj -6 0 Td (b) Tj -6 0 Td (c) Tj ET")
                .structured_text(0)
                .unwrap();
        let lines: usize = text.span_blocks().iter().map(|b| b.lines.len()).sum();
        assert_eq!(lines, 1);
    }

    #[test]
    fn right_to_left_levels_follow_uax9() {
        use BidiClass::*;
        // "abc 12 de" in Hebrew: the digits keep their order.
        let classes = [R, R, R, WS, EN, EN, WS, R, R];
        assert!(paragraph_rtl(&classes));
        assert_eq!(resolve_levels(&classes, true), [1, 1, 1, 1, 2, 2, 1, 1, 1]);
        // Latin words around Hebrew ones: a left-to-right paragraph.
        let classes = [L, L, WS, R, R, WS, L];
        assert!(!paragraph_rtl(&classes));
        assert_eq!(resolve_levels(&classes, false), [0, 0, 0, 1, 1, 0, 0]);
    }
}
