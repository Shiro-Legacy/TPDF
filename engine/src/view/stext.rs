// SPDX-License-Identifier: AGPL-3.0-or-later
//! Structured text from hayro glyph runs, following the MuPDF 1.28 stext
//! device: blocks of lines of characters with page-space quads, plain text,
//! the preserve-spans line summaries the app draws, and search hits.
//! Layout thresholds and font facts were measured against MuPDF 1.28.1.
use super::walk::{self, FORM_DEPTH, Scope};
use super::{PageFrame, ViewDocument, ViewError};
use crate::simple_encoding::{
    Difference, EncodingLimits, GlyphUnicode, NamedEncoding, SimpleEncoding,
};
use hayro::kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2};
use hayro_interpret::font::{Glyph, GlyphRun};
use hayro_interpret::hayro_cmap::BfString;
use hayro_interpret::{
    BlendMode, CacheKey, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, Paint, SoftMask, interpret_page,
};
use hayro_syntax::content::TypedIter;
use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::object::dict::keys::{
    ACTUAL_TEXT, ASCENT, BASE_ENCODING, BASE_FONT, DESCENDANT_FONTS, DESCENT, DIFFERENCES,
    ENCODING, FIRST_CHAR, FLAGS, FONT_BBOX, FONT_DESC, FONT_FILE, FONT_FILE2, FONT_FILE3,
    FONT_MATRIX, FONT_WEIGHT, KIDS, MCID, NAME, NUMS, PARENT_TREE, RESOURCES, STRUCT_PARENTS,
    STRUCT_TREE_ROOT, SUBTYPE, TYPE0, TYPE3, WIDTHS,
};
use hayro_syntax::object::{
    Array, Dict, MaybeRef, Name, Object, ObjectIdentifier, Stream, String as PdfString,
};
use hayro_syntax::page::{Page, Resources};
use hayro_syntax::xref::XRef;
use std::cell::{OnceCell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};

const PARAGRAPH_DIST: f64 = 1.5;
const SPACE_DIST: f64 = 0.15;
const SPACE_MAX_DIST: f64 = 0.8;
const BASE_MAX_DIST: f64 = 0.8;
const FAKE_BOLD_MAX_DIST: f64 = 0.1;
/// A new line starting this far (points) right of the previous one opens a paragraph.
const INDENT_DIST: f64 = 0.5;
/// Quads one `page.search` call returns at most in the app's MuPDF build.
pub const MAX_SEARCH_QUADS: usize = 500;
/// Type3 show operations a glyph run may skip over to find its own.
const TYPE3_LOOKAHEAD: usize = 16;
/// Characters one page's structured text may hold, ActualText included;
/// past it the page is refused as too complex.
pub const MAX_PAGE_CHARS: usize = 1 << 20;
/// Decoded font program bytes a [`TextReader`]'s cache may hold before it
/// starts afresh, so a document's fonts do not add up across pages.
const READER_FONT_BYTES: usize = 64 << 20;
/// Fonts and pages one reader cache serves before it starts afresh: hayro
/// also caches colour spaces, functions and shadings, unmeasured.
const READER_FONTS: usize = 64;
const READER_PAGES: usize = 64;

const TOO_MUCH_TEXT: ViewError = ViewError::TooComplex("page has more text than the limit");

/// The font facts MuPDF reports with structured text.
#[derive(Clone, Debug, PartialEq)]
pub struct TextFont {
    pub name: String,
    /// Em-relative extents used for character quads.
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
    pub size: f64,
    /// Index into [`StructuredText::fonts`].
    pub font: usize,
    /// A space inferred from a gap rather than drawn by the page.
    pub synthetic: bool,
    bidi: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextLine {
    /// x0, y0, x1, y1 in page space.
    pub bbox: [f64; 4],
    /// Unit baseline direction.
    pub dir: [f64; 2],
    pub chars: Vec<TextChar>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextBlock {
    pub bbox: [f64; 4],
    pub lines: Vec<TextLine>,
}

/// One line as `engine.ts` `parseLines` reads MuPDF's preserve-spans JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineSummary {
    pub text: String,
    pub x: i32,
    /// Baseline of the first character.
    pub y: i32,
    /// Top of the line's bbox, the JSON `bbox.y` that `stextLines` reads.
    pub top: i32,
    pub w: i32,
    pub h: i32,
    pub size: i32,
    pub family: &'static str,
    pub weight: &'static str,
    pub style: &'static str,
}

/// The text of one page in three MuPDF layouts, each made on first use:
/// preserve-whitespace (`blocks`), preserve-spans (`span_blocks`), and the
/// unclipped layout `page.search` reads.
#[derive(Clone, Debug)]
pub struct StructuredText {
    fonts: Vec<TextFont>,
    /// The characters inside the page and its clips, and every one drawn.
    glyphs: Vec<GlyphEvent>,
    all: Vec<GlyphEvent>,
    blocks: OnceCell<Vec<TextBlock>>,
    spans: OnceCell<Vec<TextBlock>>,
    unclipped: OnceCell<Vec<TextBlock>>,
}

/// Search hits in page order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchResult {
    /// Each hit's quads (ulx, uly, urx, ury, llx, lly, lrx, lry), one per run
    /// of adjacent characters.
    pub hits: Vec<Vec<[f64; 8]>>,
    /// A further match did not fit in the quad limit. It and any later
    /// matches are left out; a hit is never cut short.
    pub truncated: bool,
}

/// One extracted character before layout.
#[derive(Clone, Debug)]
struct GlyphEvent {
    c: char,
    /// Em space to page space.
    trm: Affine,
    /// Advance in ems, without character or word spacing.
    adv: f64,
    font: usize,
    /// A further character of one glyph: it does not move the pen.
    cluster: bool,
    /// First character of a MuPDF text span; preserve-spans starts a line here.
    span_start: bool,
}

impl ViewDocument {
    pub fn structured_text(&self, page: usize) -> Result<StructuredText, ViewError> {
        self.text_reader().structured_text(page)
    }

    /// Structured text of several pages sharing one font and object cache.
    pub fn text_reader(&self) -> TextReader<'_> {
        TextReader {
            view: self,
            cache: RefCell::new(ReaderCache::default()),
        }
    }
}

/// Reads pages of one view with a shared interpreter cache, so fonts used on
/// many pages are loaded once. The cache starts afresh once it holds
/// [`READER_FONT_BYTES`] of font programs, [`READER_FONTS`] fonts or
/// [`READER_PAGES`] pages, so its memory stays that of a few pages.
pub struct TextReader<'a> {
    view: &'a ViewDocument,
    cache: RefCell<ReaderCache<'a>>,
}

#[derive(Default)]
struct ReaderCache<'a> {
    hayro: InterpreterCache<'a>,
    /// Fonts the pages read so far set, by hayro's cache key.
    fonts: HashSet<u128>,
    font_bytes: usize,
    pages: usize,
}

impl TextReader<'_> {
    pub fn structured_text(&self, page: usize) -> Result<StructuredText, ViewError> {
        let page = self.view.page(page)?;
        walk::check_page(
            page,
            Scope {
                paint: false,
                annotations: false,
            },
        )?;
        let frame = PageFrame::new(page);
        let mut scan = Scan::new();
        scan.page(page);
        if scan.overflow {
            return Err(TOO_MUCH_TEXT);
        }
        let mut cache = self.cache.borrow_mut();
        let new: Vec<(u128, usize)> = (scan.programs.iter())
            .filter(|(key, _)| !cache.fonts.contains(*key))
            .map(|(&key, &bytes)| (key, bytes))
            .collect();
        let bytes = (new.iter()).fold(cache.font_bytes, |sum, (_, b)| sum.saturating_add(*b));
        if cache.pages > 0
            && (bytes > READER_FONT_BYTES
                || cache.fonts.len().saturating_add(new.len()) > READER_FONTS
                || cache.pages >= READER_PAGES)
        {
            *cache = ReaderCache::default();
        }
        cache.pages += 1;
        for (key, bytes) in scan.programs.iter() {
            if cache.fonts.insert(*key) {
                cache.font_bytes = cache.font_bytes.saturating_add(*bytes);
            }
        }
        let bounds = Rect::new(0.0, 0.0, f64::from(frame.width), f64::from(frame.height));
        // MuPDF's structured text covers page contents only, not annotations.
        let settings = super::fonts::settings(false, false);
        let mut ctx = Context::new(frame.ctm, bounds, &cache.hayro, page.xref(), settings);
        let mut capture = Capture {
            scan: &mut scan,
            clips: vec![bounds],
            blank: frame.blank,
            glyphs: Vec::new(),
            all: Vec::new(),
            chars: 0,
            overflow: false,
            last: None,
            other: false,
            mark: 0,
            actual: Vec::new(),
        };
        interpret_page(page, &mut ctx, &mut capture);
        while !capture.actual.is_empty() {
            capture.end_marked_content();
        }
        if capture.overflow {
            return Err(TOO_MUCH_TEXT);
        }
        let (glyphs, all) = (capture.glyphs, capture.all);
        Ok(StructuredText::new(glyphs, all, scan.fonts))
    }
}

/// MuPDF's `asText()`: a newline after every line and another after every block.
fn plain_text(blocks: &[TextBlock]) -> String {
    let mut out = String::new();
    for block in blocks {
        for line in &block.lines {
            out.extend(line.chars.iter().map(|ch| ch.c));
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

impl StructuredText {
    /// `glyphs` are the characters inside the page and its clips, `all` every
    /// character drawn.
    fn new(glyphs: Vec<GlyphEvent>, all: Vec<GlyphEvent>, fonts: Vec<TextFont>) -> Self {
        Self {
            fonts,
            glyphs,
            all,
            blocks: OnceCell::new(),
            spans: OnceCell::new(),
            unclipped: OnceCell::new(),
        }
    }

    /// The preserve-whitespace layout: `toStructuredText('preserve-whitespace')`.
    pub fn blocks(&self) -> &[TextBlock] {
        self.blocks
            .get_or_init(|| layout(&self.glyphs, &self.fonts, false, true))
    }

    /// The preserve-spans layout: a line also starts at every span (text
    /// object, font, matrix or colour change).
    pub fn span_blocks(&self) -> &[TextBlock] {
        self.spans
            .get_or_init(|| layout(&self.glyphs, &self.fonts, true, false))
    }

    /// The layout `page.search` reads: every character drawn, clipped or not.
    fn unclipped_blocks(&self) -> &[TextBlock] {
        self.unclipped
            .get_or_init(|| layout(&self.all, &self.fonts, false, false))
    }

    pub fn fonts(&self) -> &[TextFont] {
        &self.fonts
    }

    /// `asText()` of the preserve-whitespace layout.
    pub fn as_text(&self) -> String {
        plain_text(self.blocks())
    }

    /// `asText()` of the preserve-spans layout, the text of the app's render reply.
    pub fn spans_text(&self) -> String {
        plain_text(self.span_blocks())
    }

    /// The preserve-spans lines with integer geometry, as the app's `parseLines`.
    pub fn line_summaries(&self) -> Vec<LineSummary> {
        let trunc = |v: f32| v as i32;
        let mut out = Vec::new();
        for block in self.span_blocks() {
            for line in &block.lines {
                let Some(first) = line.chars.first() else {
                    continue;
                };
                let font = self.fonts.get(first.font);
                let [x0, y0, x1, y1] = line.bbox.map(|v| v as f32);
                out.push(LineSummary {
                    text: line.chars.iter().map(|ch| ch.c).collect(),
                    x: trunc(x0),
                    y: trunc(first.origin[1] as f32),
                    top: trunc(y0),
                    w: trunc(x1 - x0),
                    h: trunc(y1 - y0),
                    size: trunc(first.size as f32),
                    family: if font.is_some_and(|f| f.monospace) {
                        "monospace"
                    } else if font.is_some_and(|f| f.serif) {
                        "serif"
                    } else {
                        "sans-serif"
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
        }
        out
    }

    /// Case-sensitive search as the app's MuPDF `page.search`, over every
    /// character drawn, including clipped and off-page text: whitespace runs
    /// match any whitespace, a line break reads as a space and a hyphen ending
    /// a line joins it to the next. Hits hold at most `max_quads` quads in all,
    /// as MuPDF counts its limit; unlike MuPDF, a hit that does not fit is left
    /// out whole and reported as `truncated`.
    pub fn search(&self, needle: &str, max_quads: usize) -> SearchResult {
        self.search_within(needle, usize::MAX, max_quads)
    }

    /// `search` limited to `max_hits` hits instead of a number of quads:
    /// `truncated` says a further match exists.
    pub fn search_hits(&self, needle: &str, max_hits: usize) -> SearchResult {
        self.search_within(needle, max_hits, usize::MAX)
    }

    fn search_within(&self, needle: &str, max_hits: usize, max_quads: usize) -> SearchResult {
        let needle: Vec<char> = needle.chars().collect();
        let mut result = SearchResult::default();
        if needle.is_empty() {
            return result;
        }
        let mut hay: Vec<Hay<'_>> = Vec::new();
        for block in self.unclipped_blocks() {
            for line in &block.lines {
                let n = line.chars.len();
                let joined = n > 0 && is_hyphen(line.chars[n - 1].c);
                let end = if joined { n - 1 } else { n };
                hay.extend(
                    line.chars[..end]
                        .iter()
                        .map(|ch| (ch.c, Some((ch, line.dir)))),
                );
                if !joined {
                    hay.push((' ', None));
                }
            }
        }
        let mut total = 0;
        let mut at = 0;
        while at < hay.len() {
            let Some(end) = match_at(&hay, at, &needle) else {
                at += 1;
                continue;
            };
            // A character continues the last quad when its left edge meets that
            // quad's right edge (MuPDF's 0.5 em along, 0.1 em across the line).
            let mut quads: Vec<[f64; 8]> = Vec::new();
            for (_, item) in &hay[at..end] {
                let Some((ch, dir)) = item else { continue };
                let q = ch.quad;
                let near = |ax: f64, ay: f64, bx: f64, by: f64| {
                    let (dx, dy) = (bx - ax, by - ay);
                    (dir[0] * dx + dir[1] * dy).abs() < 0.5 * ch.size
                        && (dir[0] * dy - dir[1] * dx).abs() < 0.1 * ch.size
                };
                match quads.last_mut() {
                    Some(last)
                        if near(last[6], last[7], q[4], q[5])
                            && near(last[2], last[3], q[0], q[1]) =>
                    {
                        last[2..4].copy_from_slice(&q[2..4]);
                        last[6..8].copy_from_slice(&q[6..8]);
                    }
                    _ => quads.push(q),
                }
            }
            if !quads.is_empty() {
                if result.hits.len() >= max_hits || quads.len() > max_quads - total {
                    result.truncated = true;
                    break;
                }
                total += quads.len();
                result.hits.push(quads);
            }
            at = end.max(at + 1);
        }
        result
    }
}

/// A searchable character and, for drawn ones, its char and line direction.
type Hay<'a> = (char, Option<(&'a TextChar, [f64; 2])>);

fn match_at(hay: &[Hay<'_>], start: usize, needle: &[char]) -> Option<usize> {
    let (mut h, mut n) = (start, 0);
    while n < needle.len() {
        let c = hay.get(h)?.0;
        if is_white(needle[n]) {
            if !is_white(c) {
                return None;
            }
            while n < needle.len() && is_white(needle[n]) {
                n += 1;
            }
            while h < hay.len() && is_white(hay[h].0) {
                h += 1;
            }
        } else if c == needle[n] {
            h += 1;
            n += 1;
        } else {
            return None;
        }
    }
    Some(h)
}

fn is_hyphen(c: char) -> bool {
    matches!(c, '-' | '\u{ad}' | '\u{2010}' | '\u{2011}')
}

/// Whitespace for search matching, as measured on MuPDF 1.28.1 `page.search`.
fn is_white(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | ' ' | '\u{a0}' | '\u{1680}' | '\u{180e}' | '\u{2000}'
            ..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

fn is_rtl(c: char) -> bool {
    matches!(c, '\u{590}'..='\u{8ff}' | '\u{fb1d}'..='\u{fdff}' | '\u{fe70}'..='\u{feff}')
}

/// Whether MuPDF may synthesize a space after `c`.
fn may_add_space(c: Option<char>) -> bool {
    c.is_some_and(|c| c != ' ' && (c < '\u{700}' || ('\u{2000}'..='\u{20cf}').contains(&c)))
}

/// MuPDF's `fz_add_stext_char` over the captured glyphs.
fn layout(
    glyphs: &[GlyphEvent],
    fonts: &[TextFont],
    spans: bool,
    whitespace: bool,
) -> Vec<TextBlock> {
    let mut blocks: Vec<TextBlock> = Vec::new();
    let mut pen = Point::ZERO;
    let mut lag_pen = Point::ZERO;
    let mut last_char: Option<char> = None;
    let mut last_bidi = 0u8;
    for g in glyphs {
        let c = if !whitespace && is_space(g.c) {
            ' '
        } else {
            g.c
        };
        let mut bidi = u8::from(is_rtl(c));
        let [a, b, cc, d, e, f] = g.trm.as_coeffs();
        let dir = Vec2::new(a, b);
        let len = dir.hypot();
        let ndir = if len > 0.0 {
            dir / len
        } else {
            Vec2::new(1.0, 0.0)
        };
        let size = (a * d - b * cc).abs().sqrt();
        let p = Point::new(e, f);
        let q = p + dir * g.adv;
        let font = &fonts[g.font];
        if g.cluster
            && let Some(line) = blocks.last_mut().and_then(|b| b.lines.last_mut())
        {
            line.chars
                .push(make_char(c, g, font, size, pen, pen, bidi, false));
            last_char = Some(c);
            last_bidi = bidi;
            continue;
        }
        let mut new_para = false;
        let mut new_line = true;
        let mut add_space = false;
        match blocks.last().and_then(|b| b.lines.last()) {
            Some(line) if ndir.dot(Vec2::new(line.dir[0], line.dir[1])) >= 0.999 => {
                if (q - pen).hypot() / size < FAKE_BOLD_MAX_DIST && last_char == Some(c) {
                    continue;
                }
                let delta = p - pen;
                let spacing = ndir.dot(delta) / size;
                let base_offset = (-ndir.y * delta.x + ndir.x * delta.y) / size;
                if base_offset.abs() < BASE_MAX_DIST {
                    if (bidi & 1) != (last_bidi & 1) {
                        new_line = false;
                    } else if bidi & 1 == 1 {
                        let logical = ndir.dot(p - lag_pen) / size + g.adv * len / size;
                        if logical.abs() < SPACE_DIST {
                            new_line = false;
                        } else if spacing.abs() < SPACE_DIST {
                            bidi = 3;
                            new_line = false;
                        } else if logical < 0.0 && logical > -SPACE_MAX_DIST {
                            add_space = may_add_space(last_char);
                            new_line = false;
                        } else if spacing < 0.0 && spacing > -SPACE_MAX_DIST {
                            new_line = false;
                        } else if spacing > 0.0 && spacing < SPACE_MAX_DIST {
                            bidi = 3;
                            add_space = may_add_space(last_char);
                            new_line = false;
                        }
                    } else if spacing.abs() < SPACE_DIST
                        || (spacing < 0.0 && spacing > -SPACE_MAX_DIST)
                    {
                        new_line = false;
                    } else if spacing > 0.0 && spacing < SPACE_MAX_DIST {
                        add_space = may_add_space(last_char) && c != ' ';
                        new_line = false;
                    }
                } else if base_offset.abs() <= PARAGRAPH_DIST {
                    let start = line.chars.first().map_or(p.x, |ch| ch.origin[0]);
                    new_para = p.x - start > INDENT_DIST;
                } else {
                    new_para = true;
                }
            }
            _ => new_para = true,
        }
        if new_para || blocks.is_empty() {
            blocks.push(TextBlock {
                bbox: [0.0; 4],
                lines: Vec::new(),
            });
        }
        let Some(block) = blocks.last_mut() else {
            continue;
        };
        if new_line || block.lines.is_empty() || (spans && g.span_start) {
            block.lines.push(TextLine {
                bbox: [0.0; 4],
                dir: [ndir.x, ndir.y],
                chars: Vec::new(),
            });
        }
        let Some(line) = block.lines.last_mut() else {
            continue;
        };
        if add_space {
            line.chars
                .push(make_char(' ', g, font, size, pen, p, bidi, true));
        }
        line.chars
            .push(make_char(c, g, font, size, p, q, bidi, false));
        last_char = Some(c);
        last_bidi = bidi;
        lag_pen = p;
        pen = q;
    }
    for block in &mut blocks {
        for line in &mut block.lines {
            if line.chars.iter().any(|ch| ch.bidi == 3) {
                reverse_rtl_runs(&mut line.chars);
            }
            line.bbox = union(line.chars.iter().map(|ch| quad_bounds(&ch.quad)));
        }
        block.bbox = union(block.lines.iter().map(|l| l.bbox));
    }
    blocks
}

/// Visual-order right-to-left runs become logical order.
fn reverse_rtl_runs(chars: &mut [TextChar]) {
    let mut i = 0;
    while i < chars.len() {
        if chars[i].bidi & 1 == 0 {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].bidi & 1 == 1 {
            i += 1;
        }
        chars[start..i].reverse();
    }
}

#[allow(clippy::too_many_arguments)]
fn make_char(
    c: char,
    g: &GlyphEvent,
    font: &TextFont,
    size: f64,
    p: Point,
    q: Point,
    bidi: u8,
    synthetic: bool,
) -> TextChar {
    let [_, _, vx, vy, _, _] = g.trm.as_coeffs();
    let a = Vec2::new(vx, vy) * font.ascender;
    let d = Vec2::new(vx, vy) * font.descender;
    let (ul, ll, ur, lr) = (p + a, p + d, q + a, q + d);
    TextChar {
        c,
        origin: [p.x, p.y],
        quad: [ul.x, ul.y, ur.x, ur.y, ll.x, ll.y, lr.x, lr.y],
        size,
        font: g.font,
        synthetic,
        bidi,
    }
}

fn quad_bounds(q: &[f64; 8]) -> [f64; 4] {
    let xs = [q[0], q[2], q[4], q[6]];
    let ys = [q[1], q[3], q[5], q[7]];
    let min = |v: [f64; 4]| v.into_iter().fold(f64::INFINITY, f64::min);
    let max = |v: [f64; 4]| v.into_iter().fold(f64::NEG_INFINITY, f64::max);
    [min(xs), min(ys), max(xs), max(ys)]
}

fn union(rects: impl Iterator<Item = [f64; 4]>) -> [f64; 4] {
    rects
        .reduce(|a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        })
        .unwrap_or([0.0; 4])
}

/// Font facts, text-object starts, marked-content spans and Type3 text
/// gathered from the content hayro interprets.
struct Scan {
    fonts: Vec<TextFont>,
    by_key: HashMap<u128, usize>,
    /// Decoded program bytes of each font the page sets, by key.
    programs: HashMap<u128, usize>,
    starts: HashMap<[i64; 6], u32>,
    /// BMC/BDC in content order.
    marks: Vec<Mark>,
    /// Type3 fonts by dictionary key.
    type3: HashMap<u128, Type3Codes>,
    /// Type3 show operations that draw, in content order: font key and codes.
    shows: VecDeque<(u128, Vec<u8>)>,
    /// Forms being scanned, outermost first.
    active: Vec<ObjectIdentifier>,
    /// Characters of the ActualText strings in `marks`.
    actual_chars: usize,
    /// The ActualText strings pass [`MAX_PAGE_CHARS`].
    overflow: bool,
}

/// What MuPDF reads for a Type3 font without ToUnicode: characters from
/// the glyph names of its encoding, and advances from /Widths.
struct Type3Codes {
    font: usize,
    chars: Vec<Vec<char>>,
    /// Advance in ems by code.
    widths: Vec<f64>,
}

/// A marked-content start: tag, inline MCID and ActualText.
type Mark = (Vec<u8>, Option<i32>, Option<Vec<char>>);

/// Helvetica, for hayro's fallback font (cache key 0) and unknown keys.
const FALLBACK_FONT: usize = 0;
/// Type3 glyphs do not expose their font dictionary.
const TYPE3_FONT: usize = 1;

#[derive(Clone, Copy)]
struct TextParams {
    size: f32,
    scale: f32,
    rise: f32,
    leading: f32,
    tm: Affine,
    tlm: Affine,
    render: i64,
    /// Key of the current font when it is a Type3 font.
    type3: Option<u128>,
}

impl Scan {
    fn new() -> Self {
        let type3 = TextFont {
            name: "Type3".into(),
            serif: false,
            ..default_font()
        };
        Self {
            fonts: vec![base14_face("Helvetica", "Helvetica"), type3],
            by_key: HashMap::from([(0, FALLBACK_FONT)]),
            programs: HashMap::new(),
            starts: HashMap::new(),
            marks: Vec::new(),
            type3: HashMap::new(),
            shows: VecDeque::new(),
            active: Vec::new(),
            actual_chars: 0,
            overflow: false,
        }
    }

    fn font_index(&self, key: u128) -> usize {
        self.by_key.get(&key).copied().unwrap_or(FALLBACK_FONT)
    }

    /// Records a font's facts; returns its key when it is a Type3 font.
    fn register(&mut self, dict: &Dict<'_>) -> Option<u128> {
        let key = dict.cache_key();
        if !self.by_key.contains_key(&key) {
            self.by_key.insert(key, self.fonts.len());
            let (facts, program) = font_facts(dict);
            self.fonts.push(facts);
            self.programs.insert(key, program);
        }
        if !is_type3(dict) {
            return None;
        }
        if !self.type3.contains_key(&key) {
            let font = self.font_index(key);
            self.type3.insert(key, type3_codes(dict, font));
        }
        Some(key)
    }

    /// An ActualText string's characters while the page's ActualText stays
    /// within [`MAX_PAGE_CHARS`]; past it the page is refused.
    fn actual_text(&mut self, bytes: &[u8]) -> Option<Vec<char>> {
        let room = MAX_PAGE_CHARS.saturating_sub(self.actual_chars);
        // At most four bytes make a character.
        if self.overflow || bytes.len() / 4 > room {
            self.overflow = true;
            return None;
        }
        let text = text_string(bytes);
        if text.len() > room {
            self.overflow = true;
            return None;
        }
        self.actual_chars += text.len();
        Some(text)
    }

    /// Queues a Type3 show operation hayro will draw as one glyph run.
    fn show_type3(&mut self, p: &TextParams, bytes: Vec<u8>) {
        if let Some(key) = p.type3
            && p.render != 7
            && !bytes.is_empty()
        {
            self.shows.push_back((key, bytes));
        }
    }

    fn take_start(&mut self, transform: Affine) -> bool {
        match self.starts.get_mut(&start_key(transform)) {
            Some(count) if *count > 0 => {
                *count -= 1;
                true
            }
            _ => false,
        }
    }

    /// Mirrors hayro's traversal of the page contents and their forms.
    fn page(&mut self, page: &Page<'_>) {
        let xref = page.xref();
        // Marked content by MCID takes ActualText from its structure element.
        let parents = page.raw().get::<i32>(STRUCT_PARENTS).and_then(|key| {
            let root = xref.get::<Dict<'_>>(xref.root_id())?;
            let tree = root
                .get::<Dict<'_>>(STRUCT_TREE_ROOT)?
                .get::<Dict<'_>>(PARENT_TREE)?;
            let rest = number_tree(&tree, key, 0)?.into_array()?.raw_iter();
            Some(Parents {
                xref,
                rest: RefCell::new(Box::new(rest)),
                items: RefCell::default(),
            })
        });
        if let Some(data) = page.page_stream() {
            self.content(data, page.resources(), parents.as_ref(), 0);
        }
    }

    fn form(&mut self, stream: &Stream<'_>, parent: &Resources<'_>, depth: u32) {
        let id = stream.obj_id();
        if self.active.contains(&id) {
            return;
        }
        let Ok(data) = stream.decoded() else { return };
        let resources = stream
            .dict()
            .get::<Dict<'_>>(RESOURCES)
            .map_or_else(|| parent.clone(), Resources::new);
        self.active.push(id);
        // ponytail: MCIDs inside forms (their own StructParents) get no structure ActualText.
        self.content(&data, &resources, None, depth);
        self.active.pop();
    }

    fn content(
        &mut self,
        data: &[u8],
        resources: &Resources<'_>,
        parents: Option<&Parents<'_>>,
        depth: u32,
    ) {
        let mut stack = vec![TextParams {
            size: 1.0,
            scale: 100.0,
            rise: 0.0,
            leading: 0.0,
            tm: Affine::IDENTITY,
            tlm: Affine::IDENTITY,
            render: 0,
            type3: None,
        }];
        let next_line = |top: &mut TextParams, tx: f64, ty: f64| {
            top.tlm *= Affine::translate((tx, ty));
            top.tm = top.tlm;
        };
        let mut fresh = false;
        let mut ops = TypedIter::new(data);
        while let Some(op) = ops.next() {
            let Some(top) = stack.last_mut() else { break };
            match op {
                TypedInstruction::SaveState(_) => {
                    let copy = *top;
                    stack.push(copy);
                }
                TypedInstruction::RestoreState(_) => {
                    if stack.len() > 1 {
                        stack.pop();
                    }
                }
                TypedInstruction::BeginText(_) => {
                    top.tm = Affine::IDENTITY;
                    top.tlm = Affine::IDENTITY;
                    fresh = true;
                }
                TypedInstruction::EndText(_) => fresh = false,
                TypedInstruction::TextFont(t) => {
                    top.size = t.1.as_f32();
                    top.type3 = resources
                        .get_font(t.0)
                        .and_then(|dict| self.register(&dict));
                }
                TypedInstruction::TextRenderingMode(r) => top.render = r.0.as_i64(),
                TypedInstruction::HorizontalScaling(h) => top.scale = h.0.as_f32(),
                TypedInstruction::TextRise(r) => top.rise = r.0.as_f32(),
                TypedInstruction::TextLeading(l) => top.leading = l.0.as_f32(),
                TypedInstruction::NextLine(n) => next_line(top, n.0.as_f64(), n.1.as_f64()),
                TypedInstruction::NextLineAndSetLeading(n) => {
                    top.leading = -n.1.as_f32();
                    next_line(top, n.0.as_f64(), n.1.as_f64());
                }
                TypedInstruction::NextLineUsingLeading(_) => {
                    next_line(top, 0.0, -f64::from(top.leading));
                }
                TypedInstruction::SetTextMatrix(m) => {
                    top.tm = Affine::new([
                        m.0.as_f64(),
                        m.1.as_f64(),
                        m.2.as_f64(),
                        m.3.as_f64(),
                        m.4.as_f64(),
                        m.5.as_f64(),
                    ]);
                    top.tlm = top.tm;
                }
                TypedInstruction::ShowText(s) => {
                    let p = *top;
                    self.show(&mut fresh, p, [Object::String(s.0.clone())].into_iter());
                    self.show_type3(&p, s.0.as_bytes().to_vec());
                }
                TypedInstruction::ShowTexts(a) => {
                    let p = *top;
                    self.show(&mut fresh, p, a.0.iter::<Object<'_>>());
                    let mut bytes = Vec::new();
                    for item in a.0.iter::<Object<'_>>() {
                        if let Object::String(s) = item {
                            bytes.extend_from_slice(s.as_bytes());
                        }
                    }
                    self.show_type3(&p, bytes);
                }
                TypedInstruction::NextLineAndShowText(s) => {
                    next_line(top, 0.0, -f64::from(top.leading));
                    let p = *top;
                    self.show(&mut fresh, p, [Object::String(s.0.clone())].into_iter());
                    self.show_type3(&p, s.0.as_bytes().to_vec());
                }
                TypedInstruction::ShowTextWithParameters(t) => {
                    next_line(top, 0.0, -f64::from(top.leading));
                    let p = *top;
                    self.show(&mut fresh, p, [Object::String(t.2.clone())].into_iter());
                    self.show_type3(&p, t.2.as_bytes().to_vec());
                }
                TypedInstruction::BeginMarkedContent(bmc) => {
                    self.marks.push((bmc.0.as_ref().to_vec(), None, None));
                }
                TypedInstruction::BeginMarkedContentWithProperties(bdc) => {
                    let props = match bdc.1 {
                        Object::Dict(d) => Some(d.clone()),
                        Object::Name(n) => resources.properties.get::<Dict<'_>>(n.as_ref()),
                        _ => None,
                    };
                    let mcid = match bdc.1 {
                        Object::Dict(d) => d.get::<i32>(MCID),
                        _ => None,
                    };
                    let structure = || parents?.element(usize::try_from(mcid?).ok()?);
                    let actual = props
                        .and_then(|p| p.get::<PdfString<'_>>(ACTUAL_TEXT))
                        .or_else(|| structure()?.get::<PdfString<'_>>(ACTUAL_TEXT))
                        .and_then(|s| self.actual_text(s.as_bytes()));
                    self.marks.push((bdc.0.as_ref().to_vec(), mcid, actual));
                }
                // hayro draws a form only below its nesting limit.
                TypedInstruction::XObject(x) if depth < FORM_DEPTH => {
                    let form = resources.get_x_object(x.0).filter(|s| {
                        s.dict()
                            .get::<Name<'_>>(SUBTYPE)
                            .is_some_and(|n| n.as_ref() == b"Form")
                    });
                    if let Some(stream) = form {
                        self.form(&stream, resources, depth + 1);
                    }
                }
                _ => {}
            }
        }
    }

    /// Records where the first glyph of a text object lands, as hayro computes it.
    fn show<'o>(
        &mut self,
        fresh: &mut bool,
        p: TextParams,
        items: impl Iterator<Item = Object<'o>>,
    ) {
        if !*fresh {
            return;
        }
        let mut tm = p.tm;
        let h_scale = p.scale / 100.0;
        for item in items {
            match item {
                Object::Number(n) => {
                    let tx = -n.as_f32() / 1000.0 * p.size * h_scale;
                    tm *= Affine::new([1.0, 0.0, 0.0, 1.0, f64::from(tx), 0.0]);
                }
                Object::String(s) if !s.as_bytes().is_empty() => {
                    let temp = Affine::new([
                        f64::from(p.size) * f64::from(h_scale),
                        0.0,
                        0.0,
                        f64::from(p.size),
                        0.0,
                        f64::from(p.rise),
                    ]);
                    let first = tm * temp * Affine::scale(1.0 / 1000.0);
                    *self.starts.entry(start_key(first)).or_default() += 1;
                    *fresh = false;
                    return;
                }
                _ => {}
            }
        }
    }
}

fn start_key(t: Affine) -> [i64; 6] {
    t.as_coeffs().map(|v| (v * 1e6).round() as i64)
}

/// A page's array of structure parents (indexed by MCID), read once per
/// page and only as far as the largest MCID looked up; an element is
/// resolved only when a marked-content sequence names it.
struct Parents<'a> {
    xref: &'a XRef,
    rest: RefCell<Box<dyn Iterator<Item = MaybeRef<Object<'a>>> + 'a>>,
    items: RefCell<Vec<MaybeRef<Object<'a>>>>,
}

impl<'a> Parents<'a> {
    fn element(&self, mcid: usize) -> Option<Dict<'a>> {
        let mut items = self.items.borrow_mut();
        if let Some(wanted) = (mcid + 1).checked_sub(items.len()) {
            items.extend(self.rest.borrow_mut().by_ref().take(wanted));
        }
        match items.get(mcid)? {
            MaybeRef::Ref(r) => self.xref.get::<Dict<'a>>((*r).into()),
            MaybeRef::NotRef(o) => o.clone().into_dict(),
        }
    }
}

/// The value for `key` in a number tree.
fn number_tree<'a>(node: &Dict<'a>, key: i32, depth: u32) -> Option<Object<'a>> {
    if let Some(nums) = node.get::<Array<'_>>(NUMS) {
        let mut items = nums.iter::<Object<'_>>();
        while let (Some(k), Some(v)) = (items.next(), items.next()) {
            if k.into_i32() == Some(key) {
                return Some(v);
            }
        }
        return None;
    }
    if depth > 16 {
        return None;
    }
    node.get::<Array<'_>>(KIDS)?
        .iter::<Object<'_>>()
        .filter_map(Object::into_dict)
        .find_map(|kid| number_tree(&kid, key, depth + 1))
}

/// A PDF text string, decoded as everywhere else in the engine.
fn text_string(bytes: &[u8]) -> Vec<char> {
    crate::object::decode_text_string(bytes).chars().collect()
}

fn default_font() -> TextFont {
    TextFont {
        name: String::new(),
        ascender: 0.8,
        descender: -0.2,
        bold: false,
        italic: false,
        serif: true,
        monospace: false,
    }
}

/// MuPDF's builtin base-14 faces: FreeType ascender and descender (font bbox).
const BASE14: [(&str, f32, f32); 14] = [
    ("Helvetica", 1.075, -0.299),
    ("Helvetica-Bold", 1.070, -0.307),
    ("Helvetica-Oblique", 1.070, -0.284),
    ("Helvetica-BoldOblique", 1.073, -0.309),
    ("Times-Roman", 1.053, -0.281),
    ("Times-Bold", 1.044, -0.341),
    ("Times-Italic", 0.951, -0.270),
    ("Times-BoldItalic", 0.972, -0.324),
    ("Courier", 0.932, -0.317),
    ("Courier-Bold", 1.007, -0.393),
    ("Courier-Oblique", 0.920, -0.317),
    ("Courier-BoldOblique", 0.997, -0.393),
    ("Symbol", 1.010, -0.293),
    ("ZapfDingbats", 0.819, -0.144),
];

/// The base-14 face MuPDF loads for a non-embedded font: by name, else by
/// descriptor flags (FixedPitch, Serif, Italic, ForceBold).
fn standard_font(name: &str, flags: u32) -> TextFont {
    let plain = strip_subset(name);
    let family = if plain.starts_with("Courier") {
        "Courier"
    } else if plain.starts_with("Helvetica") || plain.starts_with("Arial") {
        "Helvetica"
    } else if plain.starts_with("Times") {
        "Times"
    } else if plain.starts_with("Symbol") {
        "Symbol"
    } else if plain.starts_with("ZapfDingbats") {
        "ZapfDingbats"
    } else if flags & 1 != 0 {
        "Courier"
    } else if flags & 2 != 0 {
        "Times"
    } else {
        "Helvetica"
    };
    let mut face = base14_face(family, plain);
    face.bold |= flags & (1 << 18) != 0;
    face.italic |= flags & (1 << 6) != 0;
    face
}

fn base14_face(family: &str, name: &str) -> TextFont {
    let bold = name.contains("Bold") || name.contains("Black");
    let italic = name.contains("Italic") || name.contains("Oblique");
    let (b, i) = (
        if bold { "Bold" } else { "" },
        if italic { "Italic" } else { "" },
    );
    let face = match family {
        "Times" if !bold && !italic => "Times-Roman".to_string(),
        "Times" => format!("Times-{b}{i}"),
        "Symbol" | "ZapfDingbats" => family.to_string(),
        _ if !bold && !italic => family.to_string(),
        _ => format!("{family}-{b}{}", if italic { "Oblique" } else { "" }),
    };
    let (_, ascender, descender) = BASE14
        .iter()
        .copied()
        .find(|(n, _, _)| *n == face)
        .unwrap_or(BASE14[0]);
    TextFont {
        name: face,
        ascender: f64::from(ascender),
        descender: f64::from(descender),
        bold,
        italic,
        serif: family == "Times",
        monospace: family == "Courier",
    }
}

fn strip_subset(name: &str) -> &str {
    match name.split_once('+') {
        Some((prefix, rest)) if prefix.len() == 6 => rest,
        _ => name,
    }
}

fn is_type3(dict: &Dict<'_>) -> bool {
    dict.get::<Name<'_>>(SUBTYPE)
        .is_some_and(|n| n.as_ref() == TYPE3)
}

/// MuPDF's Type3 font: /Name or "Type3 (n g R)", extents from the FontBBox
/// through the FontMatrix, no style flags.
fn type3_facts(dict: &Dict<'_>) -> TextFont {
    let name = dict.get::<Name<'_>>(NAME).map_or_else(
        || {
            let id = dict.obj_id().unwrap_or(ObjectIdentifier {
                obj_number: 0,
                gen_number: 0,
            });
            format!("Type3 ({} {} R)", id.obj_number, id.gen_number)
        },
        |n| n.as_str().to_string(),
    );
    let matrix = Affine::new(
        dict.get::<[f64; 6]>(FONT_MATRIX)
            .unwrap_or([0.001, 0.0, 0.0, 0.001, 0.0, 0.0]),
    );
    let bbox = dict
        .get::<[f64; 4]>(FONT_BBOX)
        .map(|[x0, y0, x1, y1]| transformed_bounds(matrix, Rect::new(x0, y0, x1, y1)))
        .filter(|r| r.y1 > r.y0);
    TextFont {
        name,
        ascender: bbox.map_or(0.8, |r| r.y1),
        descender: bbox.map_or(-0.2, |r| r.y0),
        bold: false,
        italic: false,
        serif: false,
        monospace: false,
    }
}

/// A Type3 font's characters and advances by code. Without a name a printable
/// ASCII code reads as itself, as in MuPDF.
fn type3_codes(dict: &Dict<'_>, font: usize) -> Type3Codes {
    let mut names: Vec<Option<Vec<u8>>> = vec![None; 256];
    let (base, differences) = match dict.get::<Object<'_>>(ENCODING) {
        Some(Object::Name(n)) => (NamedEncoding::from_pdf_name(n.as_ref()), None),
        Some(Object::Dict(d)) => (
            d.get::<Name<'_>>(BASE_ENCODING)
                .and_then(|n| NamedEncoding::from_pdf_name(n.as_ref())),
            d.get::<Array<'_>>(DIFFERENCES),
        ),
        _ => (Some(NamedEncoding::Standard), None),
    };
    if let Some(base) = base {
        let table = SimpleEncoding::from_named(base);
        for (code, slot) in names.iter_mut().enumerate() {
            *slot = table.glyph_name(code as u8).map(<[u8]>::to_vec);
        }
    }
    let mut code: i64 = 0;
    for item in differences.iter().flat_map(|a| a.iter::<Object<'_>>()) {
        match item {
            Object::Number(n) => code = n.as_i64(),
            Object::Name(n) => {
                if let Some(slot) = usize::try_from(code).ok().and_then(|c| names.get_mut(c)) {
                    *slot = Some(n.as_ref().to_vec());
                }
                code += 1;
            }
            _ => {}
        }
    }
    let chars = names
        .iter()
        .enumerate()
        .map(|(code, name)| match name {
            Some(name) => glyph_unicode(name).unwrap_or_else(|| vec!['\u{fffd}']),
            None if (0x20..0x7f).contains(&code) => vec![char::from(code as u8)],
            None => vec!['\u{fffd}'],
        })
        .collect();
    let scale = dict.get::<[f64; 6]>(FONT_MATRIX).map_or(0.001, |m| m[0]);
    let first = dict.get::<usize>(FIRST_CHAR).unwrap_or(0);
    let last = dict.get::<usize>(hayro_syntax::object::dict::keys::LAST_CHAR);
    let missing = f64::from(
        dict.get::<f32>(hayro_syntax::object::dict::keys::MISSING_WIDTH)
            .unwrap_or(0.0),
    );
    let mut widths = vec![missing * scale; 256];
    if let (Some(last), Some(array)) = (last, dict.get::<Array<'_>>(WIDTHS)) {
        let count = last.saturating_sub(first).saturating_add(1);
        for (slot, w) in widths
            .iter_mut()
            .skip(first)
            .zip(array.iter::<f32>().take(count))
        {
            *slot = f64::from(w) * scale;
        }
    }
    Type3Codes {
        font,
        chars,
        widths,
    }
}

/// MuPDF's `fz_unicode_from_glyph_name` for simple-font glyph names: a suffix
/// after '.' is dropped, TeX's f_f, f_i, f_l, f_f_i and f_f_l are ligatures,
/// other names keep their part before '_', then the glyph list, uniXXXX,
/// uXXXX[XX] and aNN (decimal). The glyph list is the crate's encoding-table
/// names plus the Greek letters.
fn glyph_unicode(name: &[u8]) -> Option<Vec<char>> {
    let name = name.split(|&b| b == b'.').next().unwrap_or_default();
    let ligature = match name {
        b"f_f" => "ff",
        b"f_i" => "fi",
        b"f_l" => "fl",
        b"f_f_i" => "ffi",
        b"f_f_l" => "ffl",
        _ => "",
    };
    if !ligature.is_empty() {
        return Some(ligature.chars().collect());
    }
    let name = name.split(|&b| b == b'_').next().unwrap_or_default();
    if name.is_empty() {
        return None;
    }
    let listed = SimpleEncoding::from_named(NamedEncoding::Standard)
        .with_differences(
            &[Difference::Code(0), Difference::Name(name)],
            EncodingLimits::default(),
        )
        .ok()
        .and_then(|table| match table.lookup_unicode(0) {
            GlyphUnicode::Unicode(chars) => Some(chars.to_vec()),
            _ => None,
        });
    if listed.is_some() {
        return listed;
    }
    if let Some(c) = greek(name) {
        return Some(vec![c]);
    }
    let text = std::str::from_utf8(name).ok()?;
    let number = |digits: &str, radix: u32, lengths: std::ops::RangeInclusive<usize>| {
        let valid = lengths.contains(&digits.len()) && digits.chars().all(|c| c.is_digit(radix));
        valid
            .then(|| u32::from_str_radix(digits, radix).ok())
            .flatten()
            .filter(|&v| v > 0)
            .and_then(char::from_u32)
    };
    let c = if let Some(hex) = text.strip_prefix("uni") {
        number(hex, 16, 4..=4)
    } else if let Some(hex) = text.strip_prefix('u') {
        number(hex, 16, 4..=6)
    } else if let Some(dec) = text.strip_prefix('a') {
        number(dec, 10, 2..=usize::MAX)
    } else {
        None
    };
    c.map(|c| vec![c])
}

/// Greek letter glyph names missing from the crate's encoding tables.
fn greek(name: &[u8]) -> Option<char> {
    const LOWER: [&str; 25] = [
        "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
        "lambda", "mu", "nu", "xi", "omicron", "pi", "rho", "sigma1", "sigma", "tau", "upsilon",
        "phi", "chi", "psi", "omega",
    ];
    let name = std::str::from_utf8(name).ok()?;
    if let Some(i) = LOWER.iter().position(|n| *n == name) {
        return char::from_u32(0x3b1 + i as u32);
    }
    if let Some(i) = LOWER
        .iter()
        .position(|n| *n != "sigma1" && name == capitalize(n))
    {
        // No capital final sigma: U+03A2 is unassigned.
        return char::from_u32(0x391 + i as u32);
    }
    match name {
        "theta1" => Some('\u{3d1}'),
        "Upsilon1" => Some('\u{3d2}'),
        "phi1" => Some('\u{3d5}'),
        "omega1" => Some('\u{3d6}'),
        _ => None,
    }
}

fn capitalize(name: &str) -> String {
    let mut chars = name.chars();
    chars
        .next()
        .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
}

/// MuPDF's facts for a font dictionary: FontDescriptor Ascent/Descent win over
/// the font program; style flags come from the program (OS/2, post) when
/// embedded. Also the decoded size of the embedded program.
fn font_facts(dict: &Dict<'_>) -> (TextFont, usize) {
    if is_type3(dict) {
        return (type3_facts(dict), 0);
    }
    let is_type0 = dict
        .get::<Name<'_>>(SUBTYPE)
        .is_some_and(|n| n.as_ref() == TYPE0);
    let font = if is_type0 {
        dict.get::<Array<'_>>(DESCENDANT_FONTS)
            .and_then(|a| a.iter::<Dict<'_>>().next())
            .unwrap_or_else(|| dict.clone())
    } else {
        dict.clone()
    };
    let base = font
        .get::<Name<'_>>(BASE_FONT)
        .or_else(|| dict.get::<Name<'_>>(BASE_FONT))
        .map(|n| n.as_str().to_string())
        .unwrap_or_default();
    let descriptor = font.get::<Dict<'_>>(FONT_DESC);
    let flags = descriptor
        .as_ref()
        .and_then(|d| d.get::<u32>(FLAGS))
        .unwrap_or(0);
    let program = descriptor.as_ref().and_then(|d| {
        [FONT_FILE, FONT_FILE2, FONT_FILE3]
            .iter()
            .find_map(|k| d.get::<Stream<'_>>(*k))
    });
    let data = program.as_ref().and_then(|stream| stream.decoded().ok());
    let mut facts = match (&program, &data) {
        (Some(_), data) => data
            .as_ref()
            .and_then(|data| sfnt_facts(data))
            // ponytail: bare CFF and Type1 programs are not parsed; FreeType
            // would take extents from the FontBBox and style from FontInfo.
            .unwrap_or_else(|| TextFont {
                italic: flags & (1 << 6) != 0,
                bold: flags & (1 << 18) != 0,
                monospace: flags & 1 != 0,
                ..default_font()
            }),
        (None, _) => standard_font(&base, flags),
    };
    let program_bytes = data.as_ref().map_or(0, |data| data.len());
    if let Some(d) = &descriptor {
        if program.is_none() && d.get::<f32>(FONT_WEIGHT).is_some_and(|w| w > 500.0) {
            facts.bold = true;
        }
        if let (Some(ascent), Some(descent)) = (d.get::<f32>(ASCENT), d.get::<f32>(DESCENT)) {
            let descent = -descent.abs();
            if ascent > descent {
                facts.ascender = f64::from(ascent / 1000.0);
                facts.descender = f64::from(descent / 1000.0);
            }
        }
    }
    // MuPDF keeps font names in a 32-byte buffer.
    let mut end = base.len().min(31);
    while !base.is_char_boundary(end) {
        end -= 1;
    }
    facts.name = base[..end].to_string();
    (facts, program_bytes)
}

fn read_u16(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn read_i16(d: &[u8], at: usize) -> Option<i16> {
    read_u16(d, at).map(|v| v as i16)
}

fn read_u32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// FreeType's view of a TrueType/OpenType program: hhea extents (OS/2 when
/// both are zero), style bits, and MuPDF's serif rule (OS/2 class 8 is sans).
fn sfnt_facts(data: &[u8]) -> Option<TextFont> {
    let version = read_u32(data, 0)?;
    if !matches!(version, 0x0001_0000 | 0x7472_7565 | 0x4f54_544f) {
        return None;
    }
    let tables = read_u16(data, 4)?;
    let table = |tag: &[u8; 4]| {
        (0..usize::from(tables)).find_map(|i| {
            let rec = 12 + 16 * i;
            if data.get(rec..rec + 4)? == tag {
                read_u32(data, rec + 8).map(|v| v as usize)
            } else {
                None
            }
        })
    };
    let head = table(b"head")?;
    let upem = f32::from(read_u16(data, head + 18).filter(|u| *u > 0)?);
    let mac_style = read_u16(data, head + 44).unwrap_or(0);
    let hhea = table(b"hhea");
    let mut ascender = hhea.and_then(|t| read_i16(data, t + 4)).unwrap_or(0);
    let mut descender = hhea.and_then(|t| read_i16(data, t + 6)).unwrap_or(0);
    let os2 = table(b"OS/2").filter(|t| read_u16(data, *t).is_some_and(|v| v != 0xffff));
    if ascender == 0
        && descender == 0
        && let Some(t) = os2
    {
        (ascender, descender) = match (read_i16(data, t + 68), read_i16(data, t + 70)) {
            (Some(a), Some(d)) if a != 0 || d != 0 => (a, d),
            _ => (
                read_i16(data, t + 74).unwrap_or(0),
                read_i16(data, t + 76).map_or(0, |v| -v),
            ),
        };
    }
    let (bold, italic) = match os2.and_then(|t| read_u16(data, t + 62)) {
        Some(s) => (s & 32 != 0, s & (1 | 512) != 0),
        None => (mac_style & 1 != 0, mac_style & 2 != 0),
    };
    let extent = |v: i16, fallback: f64| {
        if v == 0 {
            fallback
        } else {
            f64::from(f32::from(v) / upem)
        }
    };
    Some(TextFont {
        name: String::new(),
        ascender: extent(ascender, 0.8),
        descender: extent(descender, -0.2),
        bold,
        italic,
        serif: os2
            .and_then(|t| read_i16(data, t + 30))
            .is_none_or(|class| class >> 8 != 8),
        monospace: table(b"post")
            .and_then(|t| read_u32(data, t + 12))
            .is_some_and(|v| v != 0),
    })
}

/// The previous glyph run, for fill-then-stroke deduplication and span starts.
#[derive(Clone, Copy)]
struct RunMark {
    count: usize,
    first: Affine,
    last: Affine,
    mode: DrawKind,
    paint: Option<[u32; 4]>,
    font: Option<u128>,
    linear: [f64; 4],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DrawKind {
    Fill,
    Stroke,
    Invisible,
}

struct Capture<'s> {
    scan: &'s mut Scan,
    /// Device-space clip bounds; the first entry is the page.
    clips: Vec<Rect>,
    /// The page shows nothing ([`PageFrame::blank`]): no character is inside.
    blank: bool,
    /// Characters inside the page and its clips.
    glyphs: Vec<GlyphEvent>,
    /// Every character drawn, for search.
    all: Vec<GlyphEvent>,
    /// Characters in `all` and in open ActualText spans' `all`.
    chars: usize,
    /// More than [`MAX_PAGE_CHARS`] characters were drawn; the rest are dropped.
    overflow: bool,
    last: Option<RunMark>,
    /// A non-text call happened since the last run; MuPDF flushes its text object there.
    other: bool,
    /// Next unmatched entry of `Scan::marks`.
    mark: usize,
    /// Open marked-content spans; the outermost ActualText span collects its glyphs.
    actual: Vec<Option<ActualSpan>>,
}

struct ActualSpan {
    text: Vec<char>,
    glyphs: Vec<GlyphEvent>,
    all: Vec<GlyphEvent>,
}

impl ActualSpan {
    /// Replaces the span's glyph text: a shared prefix and suffix keep their own
    /// glyphs, the middle characters take the middle glyphs in order, surplus
    /// glyphs are dropped and surplus characters sit at the pen with no width.
    fn resolve(text: &[char], units: &[GlyphEvent]) -> Vec<GlyphEvent> {
        let (n, m) = (units.len(), text.len());
        let mut pre = 0;
        while pre < n.min(m) && units[pre].c == text[pre] {
            pre += 1;
        }
        let mut suf = 0;
        while suf < (n - pre).min(m - pre) && units[n - 1 - suf].c == text[m - 1 - suf] {
            suf += 1;
        }
        let mut out: Vec<GlyphEvent> = units[..pre].to_vec();
        let middle = &units[pre..n - suf];
        for (i, &c) in text[pre..m - suf].iter().enumerate() {
            match middle.get(i) {
                Some(unit) => out.push(GlyphEvent { c, ..unit.clone() }),
                None => {
                    let Some(at) = out.last().or(units.get(n - suf)) else {
                        continue;
                    };
                    out.push(GlyphEvent {
                        c,
                        adv: 0.0,
                        cluster: true,
                        span_start: false,
                        ..at.clone()
                    });
                }
            }
        }
        out.extend_from_slice(&units[n - suf..]);
        out
    }
}

impl Capture<'_> {
    fn clip(&self) -> Rect {
        self.clips.last().copied().unwrap_or(Rect::ZERO)
    }

    /// The codes of a Type3 glyph run: the next queued show operation with as
    /// many codes. Operations hayro did not draw (hidden optional content) are
    /// skipped.
    fn type3_codes(&mut self, count: usize) -> Option<(u128, Vec<u8>)> {
        let shows = &mut self.scan.shows;
        let at = shows
            .iter()
            .take(TYPE3_LOOKAHEAD)
            .position(|(_, codes)| codes.len() == count)?;
        shows.drain(..at);
        shows.pop_front()
    }
}

fn transformed_bounds(t: Affine, r: Rect) -> Rect {
    (t * r.to_path(0.1)).bounding_box()
}

fn paint_key(paint: &Paint<'_>) -> Option<[u32; 4]> {
    match paint {
        Paint::Color(c) => Some(c.to_rgba().components().map(f32::to_bits)),
        Paint::Pattern(_) => None,
    }
}

impl<'a> Device<'a> for Capture<'_> {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {
        self.other = true;
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        let next = self.clip().intersect(clip_path.path.bounding_box());
        self.clips.push(next);
        self.other = true;
    }

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {
        self.other = true;
    }

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, mode: &DrawMode) {
        let glyphs = run.glyphs();
        let (Some(first), Some(last)) = (glyphs.first(), glyphs.last()) else {
            return;
        };
        let kind = match mode {
            DrawMode::Invisible => DrawKind::Invisible,
            DrawMode::Stroke(_) => DrawKind::Stroke,
            _ => DrawKind::Fill,
        };
        let (first_t, last_t) = (first.transform(), last.transform());
        // Fill-and-stroke text arrives twice; MuPDF extracts the text object once.
        if kind == DrawKind::Stroke
            && !self.other
            && self.last.is_some_and(|m| {
                m.mode == DrawKind::Fill
                    && m.count == glyphs.len()
                    && m.first == first_t
                    && m.last == last_t
            })
        {
            return;
        }
        let font_key = match &**first {
            Glyph::Outline(o) => Some(o.font_cache_key()),
            Glyph::Type3(_) => None,
        };
        let em = Affine::scale(1000.0);
        let [a, b, c, d, _, _] = (props.transform * first_t * em).as_coeffs();
        let linear = [a, b, c, d];
        let paint = paint_key(&props.paint);
        let starts_object = self.scan.take_start(first_t);
        let new_object = starts_object
            || self.other
            || self.last.is_none_or(|m| m.paint != paint || m.mode != kind);
        let same_matrix = |m: &RunMark| {
            m.linear
                .iter()
                .zip(linear)
                .all(|(x, y)| (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0))
        };
        let mut span_start = new_object
            || self
                .last
                .is_none_or(|m| m.font != font_key || !same_matrix(&m));
        self.last = Some(RunMark {
            count: glyphs.len(),
            first: first_t,
            last: last_t,
            mode: kind,
            paint,
            font: font_key,
            linear,
        });
        self.other = false;

        let type3 = match &**first {
            Glyph::Type3(_) => self.type3_codes(glyphs.len()),
            Glyph::Outline(_) => None,
        };
        let type3 = type3.and_then(|(key, codes)| Some((self.scan.type3.get(&key)?, codes)));
        let mut items: Vec<(Affine, f64, usize)> = glyphs
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let trm = props.transform * g.transform() * em;
                match (&**g, &type3) {
                    (Glyph::Outline(o), _) => (
                        trm,
                        f64::from(o.advance_width().unwrap_or(0.0)) / 1000.0,
                        self.scan.font_index(o.font_cache_key()),
                    ),
                    (Glyph::Type3(_), Some((t3, codes))) => {
                        (trm, t3.widths[usize::from(codes[i])], t3.font)
                    }
                    (Glyph::Type3(_), None) => (trm, f64::NAN, TYPE3_FONT),
                }
            })
            .collect();
        let chars: Vec<Vec<char>> = glyphs
            .iter()
            .enumerate()
            .map(|(i, g)| match (g.as_unicode(), &type3) {
                (None, Some((t3, codes))) => expand(t3.chars[usize::from(codes[i])].clone()),
                (mapped, _) => unicode(mapped),
            })
            .collect();
        // ponytail: hayro 0.8 hides Type3 advances; use the distance to the next
        // glyph, and the run's mean (or half an em) for the last one.
        for i in 0..items.len() {
            if items[i].1.is_nan()
                && let Some(next) = items.get(i + 1)
            {
                let size = Vec2::new(items[i].0.as_coeffs()[0], items[i].0.as_coeffs()[1]).hypot();
                items[i].1 = (next.0.translation() - items[i].0.translation()).hypot() / size;
            }
        }
        let known: Vec<f64> = items
            .iter()
            .map(|i| i.1)
            .filter(|v| v.is_finite())
            .collect();
        let mean = if known.is_empty() {
            0.5
        } else {
            known.iter().sum::<f64>() / known.len() as f64
        };
        for item in &mut items {
            if !item.1.is_finite() {
                item.1 = mean;
            }
        }
        // MuPDF keeps a run wholly inside the clip; otherwise only glyphs whose
        // ink meets it, so inkless glyphs such as spaces drop out. Glyph
        // outlines are only measured when the em boxes, grown by half an em,
        // are not all inside.
        let clip = self.clip();
        let inside =
            |r: &Rect| r.x0 >= clip.x0 && r.y0 >= clip.y0 && r.x1 <= clip.x1 && r.y1 <= clip.y1;
        let em_box = |(trm, adv, font): &(Affine, f64, usize), grow: f64| {
            let f = &self.scan.fonts[*font];
            let r = Rect::new(-grow, f.descender - grow, adv + grow, f.ascender + grow);
            transformed_bounds(*trm, r)
        };
        let count = chars.iter().map(Vec::len).fold(0, usize::saturating_add);
        if self.overflow || self.chars.saturating_add(count) > MAX_PAGE_CHARS {
            self.overflow = true;
            return;
        }
        self.chars += count;
        let keep: Vec<bool> = if self.blank {
            vec![false; items.len()]
        } else if items.iter().all(|i| inside(&em_box(i, 0.5))) {
            vec![true; items.len()]
        } else {
            let ink: Vec<Option<Rect>> = glyphs
                .iter()
                .zip(&items)
                .map(|(g, item)| {
                    let r = match &**g {
                        Glyph::Outline(o) => {
                            (props.transform * g.transform() * o.outline()).bounding_box()
                        }
                        Glyph::Type3(_) => em_box(item, 0.0),
                    };
                    (r.width() > 0.0 && r.height() > 0.0).then_some(r)
                })
                .collect();
            let all_inside = ink.iter().flatten().all(inside);
            ink.iter()
                .map(|r| {
                    all_inside
                        || r.is_some_and(|r| {
                            r.x0 < clip.x1 && r.x1 > clip.x0 && r.y0 < clip.y1 && r.y1 > clip.y0
                        })
                })
                .collect()
        };
        let mut all_start = span_start;
        for ((chars, (trm, adv, font)), keep) in chars.into_iter().zip(items).zip(keep) {
            let (kept, all) = match self.actual.iter_mut().flatten().next() {
                Some(span) => (&mut span.glyphs, &mut span.all),
                None => (&mut self.glyphs, &mut self.all),
            };
            for (i, c) in chars.into_iter().enumerate() {
                let event = GlyphEvent {
                    c,
                    trm,
                    adv: if i == 0 { adv } else { 0.0 },
                    font,
                    cluster: i > 0,
                    span_start: false,
                };
                if keep {
                    kept.push(GlyphEvent {
                        span_start: span_start && i == 0,
                        ..event.clone()
                    });
                }
                all.push(GlyphEvent {
                    span_start: all_start && i == 0,
                    ..event
                });
            }
            all_start = false;
            if keep {
                span_start = false;
            }
        }
    }

    fn begin_marked_content(&mut self, tag: &[u8], mcid: Option<i32>) {
        self.other = true;
        let marks = &self.scan.marks;
        let found = (self.mark..marks.len().min(self.mark + 64))
            .find(|&i| marks[i].0 == tag && marks[i].1 == mcid);
        let actual = found.and_then(|i| {
            self.mark = i + 1;
            marks[i].2.clone()
        });
        // An enclosing ActualText wins over nested ones.
        let outer = self.actual.iter().flatten().next().is_some();
        self.actual
            .push(actual.filter(|_| !outer).map(|text| ActualSpan {
                text,
                glyphs: Vec::new(),
                all: Vec::new(),
            }));
    }

    fn end_marked_content(&mut self) {
        self.other = true;
        if let Some(Some(span)) = self.actual.pop() {
            // The span's characters become its ActualText, one per character.
            let chars = (self.chars.saturating_sub(span.all.len())).saturating_add(span.text.len());
            if self.overflow || chars > MAX_PAGE_CHARS {
                self.overflow = true;
                return;
            }
            self.chars = chars;
            self.glyphs
                .extend(ActualSpan::resolve(&span.text, &span.glyphs));
            self.all.extend(ActualSpan::resolve(&span.text, &span.all));
        }
    }

    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {
        self.other = true;
    }

    fn pop_clip(&mut self) {
        if self.clips.len() > 1 {
            self.clips.pop();
        }
        self.other = true;
    }

    fn pop_transparency_group(&mut self) {
        self.other = true;
    }
}

/// A glyph's characters as MuPDF reports them: unknown glyphs read U+FFFD,
/// Latin ligatures split, and Arabic presentation forms become base letters.
fn unicode(mapped: Option<BfString>) -> Vec<char> {
    match mapped {
        Some(BfString::Char(c)) => expand([c]),
        Some(BfString::String(s)) if !s.is_empty() => expand(s.chars()),
        _ => vec!['\u{fffd}'],
    }
}

fn expand(text: impl IntoIterator<Item = char>) -> Vec<char> {
    let mut out = Vec::new();
    for c in text {
        match c {
            '\u{fb00}' => out.extend(['f', 'f']),
            '\u{fb01}' => out.extend(['f', 'i']),
            '\u{fb02}' => out.extend(['f', 'l']),
            '\u{fb03}' => out.extend(['f', 'f', 'i']),
            '\u{fb04}' => out.extend(['f', 'f', 'l']),
            '\u{fb05}' | '\u{fb06}' => out.extend(['s', 't']),
            '\u{fef5}'..='\u{fefc}' => {
                let alef = ['\u{622}', '\u{623}', '\u{625}', '\u{627}'];
                out.extend(['\u{644}', alef[(c as usize - 0xfef5) / 2]]);
            }
            '\u{fe80}'..='\u{fef4}' => out.push(arabic_base(c)),
            _ => out.push(c),
        }
    }
    out
}

/// Arabic Presentation Forms-B (U+FE80..U+FEF4) to the base letter.
fn arabic_base(c: char) -> char {
    // (first form, number of forms, base letter)
    const GROUPS: [(u32, u32, u32); 36] = [
        (0xfe80, 1, 0x621),
        (0xfe81, 2, 0x622),
        (0xfe83, 2, 0x623),
        (0xfe85, 2, 0x624),
        (0xfe87, 2, 0x625),
        (0xfe89, 4, 0x626),
        (0xfe8d, 2, 0x627),
        (0xfe8f, 4, 0x628),
        (0xfe93, 2, 0x629),
        (0xfe95, 4, 0x62a),
        (0xfe99, 4, 0x62b),
        (0xfe9d, 4, 0x62c),
        (0xfea1, 4, 0x62d),
        (0xfea5, 4, 0x62e),
        (0xfea9, 2, 0x62f),
        (0xfeab, 2, 0x630),
        (0xfead, 2, 0x631),
        (0xfeaf, 2, 0x632),
        (0xfeb1, 4, 0x633),
        (0xfeb5, 4, 0x634),
        (0xfeb9, 4, 0x635),
        (0xfebd, 4, 0x636),
        (0xfec1, 4, 0x637),
        (0xfec5, 4, 0x638),
        (0xfec9, 4, 0x639),
        (0xfecd, 4, 0x63a),
        (0xfed1, 4, 0x641),
        (0xfed5, 4, 0x642),
        (0xfed9, 4, 0x643),
        (0xfedd, 4, 0x644),
        (0xfee1, 4, 0x645),
        (0xfee5, 4, 0x646),
        (0xfee9, 4, 0x647),
        (0xfeed, 2, 0x648),
        (0xfeef, 2, 0x649),
        (0xfef1, 4, 0x64a),
    ];
    let code = c as u32;
    GROUPS
        .iter()
        .find(|(start, count, _)| (*start..start + count).contains(&code))
        .and_then(|(_, _, base)| char::from_u32(*base))
        .unwrap_or(c)
}
