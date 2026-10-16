// SPDX-License-Identifier: AGPL-3.0-or-later
//! Structured text from hayro glyph runs, following the MuPDF 1.28 stext
//! device: blocks of lines of characters with page-space quads, plain text,
//! the preserve-spans line summaries the app draws, and search hits.
//! Layout thresholds and font facts were measured against MuPDF 1.28.1.
use super::text_unicode::{self, CMaps, Code, FontText};
use super::walk::{self, FORM_DEPTH, Scope};
use super::{PageFrame, ViewDocument, ViewError};
use hayro::kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2};
use hayro_interpret::font::{Glyph, GlyphRun};
use hayro_interpret::hayro_cmap::BfString;
use hayro_interpret::pattern::Pattern;
use hayro_interpret::{
    BlendMode, CacheKey, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, Paint, SoftMask, interpret, interpret_page,
};
use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::content::{TypedIter, UntypedIter};
use hayro_syntax::object::dict::keys::{
    ACTUAL_TEXT, ASCENT, BASE_FONT, BASE_STATE, BBOX, CHAR_PROCS, D, DESCENDANT_FONTS, DESCENT,
    ENCODING, FIRST_CHAR, FLAGS, FONT_BBOX, FONT_DESC, FONT_FILE, FONT_FILE2, FONT_FILE3,
    FONT_MATRIX, FONT_WEIGHT, KIDS, MCID, NAME, NUMS, OC, OCGS, OCMD, OCPROPERTIES, OFF, ON, P,
    PARENT_TREE, RESOURCES, STRUCT_PARENTS, STRUCT_TREE_ROOT, SUBTYPE, TYPE, TYPE0, TYPE3, WIDTHS,
    XOBJECT,
};
use hayro_syntax::object::{
    Array, Dict, MaybeRef, Name, Object, ObjectIdentifier, Stream, String as PdfString,
};
use hayro_syntax::page::{Page, Resources};
use std::cell::{OnceCell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

const PARAGRAPH_DIST: f64 = 1.5;
const SPACE_DIST: f64 = 0.15;
/// MuPDF compares in single precision: a gap of `SPACE_DIST` made by a `TJ`
/// adjustment (TeX's ellipsis kerns) reads below it there, one made by
/// positioning or letter spacing above it, whichever side of it the gap
/// lies here by its last bits. Emulating the single precision computation
/// from these coordinates matched neither.
const SPACE_SLACK: f64 = 1e-5;
const SPACE_MAX_DIST: f64 = 0.8;
const BASE_MAX_DIST: f64 = 0.8;
const FAKE_BOLD_MAX_DIST: f64 = 0.1;
/// A new line starting this far (points) right of the previous one opens a paragraph.
const INDENT_DIST: f64 = 0.5;
/// Quads one `page.search` call returns at most in the app's MuPDF build.
pub const MAX_SEARCH_QUADS: usize = 500;
/// Show operations of a font a glyph run may skip over to find its own.
const SHOW_LOOKAHEAD: usize = 16;
/// Decoded bytes of Type3 glyph procedures (and the forms they draw) one
/// page may read to find the glyphs MuPDF draws directly; past it, glyphs
/// read as cached.
const TYPE3_CHECK_BYTES: u64 = 4 << 20;
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
    /// First character of a text object: directions start left to right.
    fresh: bool,
    /// A `TJ` adjustment comes just before it.
    kerned: bool,
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
    /// How those fonts' codes read as text.
    texts: HashMap<u128, Rc<FontText>>,
    /// Predefined CMaps those texts use.
    cmaps: CMaps,
    /// Type3 glyph procedures checked so far: drawn directly, and the cost.
    glyph_procs: HashMap<GlyphProc, (bool, u64)>,
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
        {
            let mut cache = self.cache.borrow_mut();
            scan.known = std::mem::take(&mut cache.texts);
            scan.cmaps = std::mem::take(&mut cache.cmaps);
            scan.glyph_procs = std::mem::take(&mut cache.glyph_procs);
        }
        scan.page(page);
        let mut cache = self.cache.borrow_mut();
        cache.texts = std::mem::take(&mut scan.known);
        cache.cmaps = std::mem::take(&mut scan.cmaps);
        cache.glyph_procs = std::mem::take(&mut scan.glyph_procs);
        if scan.overflow {
            return Err(TOO_MUCH_TEXT);
        }
        let clip_mode = if scan.page_clip_text {
            page.page_stream().and_then(clip_mode_content)
        } else {
            None
        };
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
        for (key, text) in &scan.texts {
            cache.texts.entry(*key).or_insert_with(|| text.clone());
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
            clip_text: None,
            text_object: None,
        };
        match &clip_mode {
            Some(data) => interpret(
                TypedIter::new(data),
                page.resources(),
                &mut ctx,
                &mut capture,
            ),
            None => interpret_page(page, &mut ctx, &mut capture),
        }
        capture.flush_clip();
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
    pub fn unclipped_blocks(&self) -> &[TextBlock] {
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
    // The reading direction (right to left) characters of neutral classes take.
    let mut rtl = false;
    for g in glyphs {
        let c = if !whitespace && is_space(g.c) {
            ' '
        } else {
            g.c
        };
        if g.fresh {
            rtl = false;
        }
        rtl = text_unicode::direction(c).unwrap_or(rtl);
        let mut bidi = u8::from(rtl);
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
                    } else if spacing.abs()
                        < SPACE_DIST + if g.kerned { SPACE_SLACK } else { -SPACE_SLACK }
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
        // A change of direction starts a span.
        let turn = (bidi & 1) != (last_bidi & 1);
        if new_line || block.lines.is_empty() || (spans && (g.span_start || turn)) {
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
struct Scan<'a> {
    fonts: Vec<TextFont>,
    by_key: HashMap<u128, usize>,
    /// Decoded program bytes of each font the page sets, by key.
    programs: HashMap<u128, usize>,
    starts: HashMap<[i64; 6], u32>,
    /// BMC/BDC in content order.
    marks: Vec<Mark>,
    /// Marks whose ActualText may come from the structure element of an
    /// MCID: mark index and MCID.
    pending: Vec<(usize, usize)>,
    /// Type3 fonts by dictionary key.
    type3: HashMap<u128, Type3Codes<'a>>,
    /// How each font's codes read as text, by key.
    texts: HashMap<u128, Rc<FontText>>,
    /// Those of the fonts earlier pages of the reader set.
    known: HashMap<u128, Rc<FontText>>,
    /// The reader's predefined CMaps.
    cmaps: CMaps,
    /// The reader's checked Type3 glyph procedures, and this page's: those
    /// it charged to its budget, and the budget left.
    glyph_procs: HashMap<GlyphProc, (bool, u64)>,
    charged: HashSet<GlyphProc>,
    type3_budget: u64,
    /// The codes of the show operations hayro draws, in content order, and
    /// whether a `TJ` adjustment comes before each.
    codes: Vec<Code>,
    kerns: Vec<bool>,
    /// Those show operations by font key; Type3 ones under `None` (hayro
    /// does not say which Type3 font a glyph belongs to).
    shows: HashMap<Option<u128>, VecDeque<Show>>,
    /// Show operations queued so far, and the last one a glyph run took.
    queued: usize,
    taken: Option<usize>,
    /// Optional content as hayro evaluates it, for the shows it draws.
    ocg: OcgState,
    /// Text objects begun so far; the open one.
    objects: u32,
    open_object: Option<u32>,
    /// Text objects that showed text in a clipping mode (MuPDF extracts
    /// that text again at ET, as one clip).
    clip_objects: HashMap<u32, ClipObject>,
    /// The page's own content shows text in clipping mode 7.
    page_clip_text: bool,
    /// Forms being scanned, outermost first.
    active: Vec<ObjectIdentifier>,
    /// Characters of the ActualText strings in `marks`.
    actual_chars: usize,
    /// The ActualText strings pass [`MAX_PAGE_CHARS`].
    overflow: bool,
}

/// A Type3 font's facts index, advances from /Widths, and which glyph
/// procedures MuPDF draws directly (each glyph then a span of its own),
/// found for the codes the page shows.
struct Type3Codes<'a> {
    font: usize,
    /// Advance in ems by code.
    widths: Vec<f64>,
    uncacheable: Vec<Option<bool>>,
    names: Vec<Option<Vec<u8>>>,
    procs: Option<Dict<'a>>,
    resources: Option<Dict<'a>>,
}

/// A Type3 glyph procedure and the resources of its font.
type GlyphProc = (ObjectIdentifier, u128);

/// A show operation hayro draws: its font and its codes in `Scan::codes`.
#[derive(Clone, Copy)]
struct Show {
    /// Its place in content order.
    seq: usize,
    /// Where its first glyph lands: the linear part of its transform, and
    /// where on the page.
    linear: [f64; 4],
    place: Place,
    key: u128,
    /// The font dictionary's object: MuPDF loads a font per object, so two
    /// equal dictionaries are two fonts (hayro's key is their content).
    obj: Option<ObjectIdentifier>,
    start: usize,
    len: usize,
    /// The glyphs hayro draws for it: one per byte for simple fonts.
    glyphs: usize,
    /// Its text rendering mode in the content (7 is drawn as 4, see
    /// [`clip_mode_content`]) and its text object.
    render: i64,
    object: u32,
    /// MuPDF flushed its text since the previous show: a new span starts.
    flushed: bool,
}

/// Where a show's first glyph lands in text space.
#[derive(Clone, Copy, PartialEq)]
enum Place {
    /// At this translation.
    At([f64; 2]),
    /// Just after the glyphs of the show with this `seq` (the pen moved by
    /// their advances, which the scan does not know).
    After(usize),
    /// Somewhere: a vertical font displaces glyphs from the pen by metrics
    /// the scan does not read.
    Anywhere,
}

impl Show {
    /// Whether a glyph run whose first glyph has transform `first`, just
    /// after the run that took show `taken`, is this show's.
    fn drawn_as(&self, glyphs: usize, first: Affine, taken: Option<usize>) -> bool {
        let close = |a: f64, b: f64| (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0);
        let [a, b, c, d, e, f] = first.as_coeffs();
        self.glyphs == glyphs
            && self
                .linear
                .iter()
                .zip([a, b, c, d])
                .all(|(x, y)| close(*x, y))
            && match self.place {
                Place::At([x, y]) => close(x, e) && close(y, f),
                Place::After(seq) => taken == Some(seq),
                Place::Anywhere => true,
            }
    }
}

/// A text object with text in a clipping mode, as MuPDF's run processor
/// treats it: the clipping text is extracted at ET when the object's last
/// show was in a clipping or invisible mode, else dropped.
#[derive(Default)]
struct ClipObject {
    last_mode: i64,
    closed: bool,
}

impl ClipObject {
    fn extracted(&self) -> bool {
        self.closed && (3..=7).contains(&self.last_mode)
    }
}

/// hayro's optional-content state (`OcgState`): the groups off in the
/// catalog's default configuration and a visibility per marked-content level.
#[derive(Default)]
struct OcgState {
    off: HashSet<ObjectIdentifier>,
    stack: Vec<bool>,
}

impl OcgState {
    fn from_catalog(catalog: &Dict<'_>) -> Self {
        let mut off = HashSet::new();
        let Some(props) = catalog.get::<Dict<'_>>(OCPROPERTIES) else {
            return Self::default();
        };
        let Some(config) = props.get::<Dict<'_>>(D) else {
            return Self::default();
        };
        let ids = |array: Option<Array<'_>>| -> Vec<ObjectIdentifier> {
            array
                .iter()
                .flat_map(|a| a.raw_iter())
                .filter_map(|item| item.as_obj_ref().map(ObjectIdentifier::from))
                .collect()
        };
        if config
            .get::<Name<'_>>(BASE_STATE)
            .is_some_and(|b| b.as_ref() == OFF)
        {
            off.extend(ids(props.get::<Array<'_>>(OCGS)));
        }
        for id in ids(config.get::<Array<'_>>(ON)) {
            off.remove(&id);
        }
        off.extend(ids(config.get::<Array<'_>>(OFF)));
        Self {
            off,
            stack: Vec::new(),
        }
    }

    fn visible(&self) -> bool {
        self.stack.last().copied().unwrap_or(true)
    }

    fn begin(&mut self) {
        self.stack.push(self.visible());
    }

    /// An optional-content group (by its `id`) or membership dictionary (an
    /// /OCMD, or any dictionary written directly).
    fn begin_oc(&mut self, dict: &Dict<'_>, id: Option<ObjectIdentifier>) {
        let on = |id: &ObjectIdentifier| !self.off.contains(id);
        let ocmd = dict
            .get::<Name<'_>>(TYPE)
            .is_some_and(|t| t.as_ref() == OCMD);
        let active = if let (Some(id), false) = (id, ocmd) {
            on(&id)
        } else {
            let mut members: Vec<ObjectIdentifier> = match dict.get::<Array<'_>>(OCGS) {
                Some(a) => a
                    .raw_iter()
                    .filter_map(|item| item.as_obj_ref().map(ObjectIdentifier::from))
                    .collect(),
                None => Vec::new(),
            };
            if members.is_empty()
                && let Some(r) = dict.get_ref(OCGS)
            {
                members.push(r.into());
            }
            let policy = dict.get::<Name<'_>>(P);
            members.is_empty()
                || match policy.as_ref().map(|p| p.as_ref()) {
                    Some(b"AllOn") => members.iter().all(on),
                    Some(b"AnyOff") => !members.iter().all(on),
                    Some(b"AllOff") => !members.iter().any(on),
                    _ => members.iter().any(on),
                }
        };
        let visible = self.visible() && active;
        self.stack.push(visible);
    }

    fn end(&mut self) {
        self.stack.pop();
    }
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
    /// Mode 7 was set by the page's own content, which hayro reads
    /// rewritten ([`clip_mode_content`]) and so draws.
    rewritten: bool,
    font: ShowFont,
    /// Whether `tm` is where the pen is.
    pen: Pen,
}

/// The font hayro shows text in.
#[derive(Clone, Copy, PartialEq)]
enum ShowFont {
    /// None yet: `Tj` and `TJ` set the fallback, `'` and `"` show nothing.
    None,
    /// Helvetica (cache key 0), drawing ASCII strings only: no `Tf` before,
    /// or one naming a font the resources lack.
    Fallback,
    /// A font dictionary: its key and object.
    Font(u128, Option<ObjectIdentifier>),
}

/// What the scan knows of hayro's pen.
#[derive(Clone, Copy, PartialEq)]
enum Pen {
    /// At `TextParams::tm`.
    Known,
    /// Past the glyphs of the show with this `seq`.
    After(usize),
    /// Past glyphs hayro did not draw.
    Unknown,
}

impl<'a> Scan<'a> {
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
            pending: Vec::new(),
            type3: HashMap::new(),
            texts: HashMap::new(),
            known: HashMap::new(),
            cmaps: CMaps::new(),
            glyph_procs: HashMap::new(),
            charged: HashSet::new(),
            type3_budget: TYPE3_CHECK_BYTES,
            codes: Vec::new(),
            kerns: Vec::new(),
            shows: HashMap::new(),
            queued: 0,
            taken: None,
            ocg: OcgState::default(),
            objects: 0,
            open_object: None,
            clip_objects: HashMap::new(),
            page_clip_text: false,
            active: Vec::new(),
            actual_chars: 0,
            overflow: false,
        }
    }

    fn font_index(&self, key: u128) -> usize {
        self.by_key.get(&key).copied().unwrap_or(FALLBACK_FONT)
    }

    /// Records a font's facts and text; returns its key.
    fn register(&mut self, dict: &Dict<'a>) -> u128 {
        let key = dict.cache_key();
        if !self.by_key.contains_key(&key) {
            self.by_key.insert(key, self.fonts.len());
            let program = font_program(dict);
            let data = program.as_ref().and_then(|s| s.decoded().ok());
            self.fonts
                .push(font_facts(dict, program.is_some(), data.as_deref()));
            self.programs
                .insert(key, data.as_ref().map_or(0, |d| d.len()));
            let text = match self.known.get(&key) {
                Some(text) => text.clone(),
                None => Rc::new(FontText::new(dict, data.as_deref(), &mut self.cmaps)),
            };
            self.texts.insert(key, text);
        }
        if is_type3(dict) && !self.type3.contains_key(&key) {
            let font = self.font_index(key);
            self.type3.insert(key, type3_codes(dict, font));
        }
        key
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
        // MuPDF keeps ActualText as a C string: it ends at the first NUL.
        let mut text = text_string(bytes);
        if let Some(nul) = text.iter().position(|&c| c == '\0') {
            text.truncate(nul);
        }
        if text.len() > room {
            self.overflow = true;
            return None;
        }
        self.actual_chars += text.len();
        Some(text)
    }

    /// Queues a show operation as hayro draws it, one glyph run whose first
    /// glyph has transform `first`; returns the pen after it. `strings`: its
    /// strings, each with whether a `TJ` adjustment comes before it.
    /// Clipping text (mode 7) is drawn only from the page's own content,
    /// rewritten.
    fn queue_show(
        &mut self,
        p: &TextParams,
        strings: &[(&[u8], bool)],
        first: Option<Affine>,
        flushed: bool,
    ) -> Pen {
        let bytes: usize = strings.iter().map(|s| s.0.len()).sum();
        // hayro's fallback font draws ASCII strings only.
        let ascii = strings.iter().all(|s| s.0.is_ascii());
        let (key, obj) = match p.font {
            ShowFont::None => return p.pen,
            ShowFont::Fallback => (0, None),
            ShowFont::Font(key, obj) => (key, obj),
        };
        let Some(first) = first.filter(|_| bytes > 0) else {
            return p.pen;
        };
        let object = self.open_object.unwrap_or(0);
        if p.render == 7 && p.rewritten {
            self.page_clip_text = true;
            self.clip_objects.entry(object).or_default();
        }
        if let Some(clip) = self.clip_objects.get_mut(&object) {
            clip.last_mode = p.render;
        }
        let drawn = (p.render != 7 || p.rewritten)
            && self.ocg.visible()
            && (ascii || p.font != ShowFont::Fallback);
        if !drawn {
            return Pen::Unknown;
        }
        let text = self.texts.get(&key).cloned();
        let start = self.codes.len();
        let glyphs = match &text {
            Some(text) => {
                // hayro reads each string's codes apart.
                for &(string, kerned) in strings {
                    let at = self.codes.len();
                    text.codes(string, &mut self.codes);
                    let n = self.codes.len() - at;
                    self.kerns.extend((0..n).map(|i| kerned && i == 0));
                }
                if text.one_byte_glyphs() {
                    bytes
                } else {
                    self.codes.len() - start
                }
            }
            None => bytes,
        };
        let len = self.codes.len() - start;
        let type3 = self.type3.contains_key(&key);
        if type3 {
            for (string, _) in strings {
                self.check_type3(key, string);
            }
        }
        let [a, b, c, d, e, f] = first.as_coeffs();
        let place = match p.pen {
            Pen::Known if text.as_ref().is_some_and(|t| t.vertical()) => Place::Anywhere,
            Pen::Known => Place::At([e, f]),
            Pen::After(seq) => Place::After(seq),
            Pen::Unknown => return Pen::Unknown,
        };
        let seq = self.queued;
        self.queued += 1;
        let queue = if type3 { None } else { Some(key) };
        self.shows.entry(queue).or_default().push_back(Show {
            seq,
            linear: [a, b, c, d],
            place,
            key,
            obj,
            start,
            len,
            glyphs,
            render: p.render,
            object,
            flushed,
        });
        Pen::After(seq)
    }

    /// The queued show a glyph run of a font (`None`: a Type3 font) is
    /// drawn from: one with as many glyphs whose first lands where the
    /// run's does. Runs come in content order, so shows queued before the
    /// last one taken were not drawn.
    fn take_show(&mut self, key: Option<u128>, glyphs: usize, first: Affine) -> Option<Show> {
        let taken = self.taken;
        let queue = self.shows.get_mut(&key)?;
        while queue
            .front()
            .is_some_and(|s| taken.is_some_and(|t| s.seq <= t))
        {
            queue.pop_front();
        }
        let at = queue
            .iter()
            .take(SHOW_LOOKAHEAD)
            .position(|s| s.drawn_as(glyphs, first, taken))?;
        queue.drain(..at);
        let show = queue.pop_front()?;
        self.taken = Some(show.seq);
        Some(show)
    }

    /// Finds which of the shown codes of a Type3 font MuPDF draws directly,
    /// each glyph procedure once per reader, within the page's budget.
    fn check_type3(&mut self, key: u128, bytes: &[u8]) {
        let Some(t3) = self.type3.get_mut(&key) else {
            return;
        };
        let resources_key = t3.resources.as_ref().map_or(0, |r| r.cache_key());
        for &byte in bytes {
            let code = usize::from(byte);
            if t3.uncacheable.get(code).is_none_or(Option::is_some) {
                continue;
            }
            let proc = (t3.names[code].as_ref())
                .zip(t3.procs.as_ref())
                .and_then(|(name, procs)| procs.get::<Stream<'_>>(name.as_slice()));
            let Some(proc) = proc else {
                t3.uncacheable[code] = Some(false);
                continue;
            };
            let id = (proc.obj_id(), resources_key);
            // Each procedure costs the page what reading it costs, whether
            // an earlier page of the reader read it or not.
            let first_here = self.charged.insert(id);
            let direct = match (self.glyph_procs.get(&id), first_here) {
                (Some(&(direct, _)), false) => direct,
                (Some(&(direct, cost)), true) if cost <= self.type3_budget => {
                    self.type3_budget -= cost;
                    direct
                }
                // Over the budget on this page.
                (Some(_), true) | (None, true) if self.type3_budget == 0 => false,
                (Some(_), true) => {
                    self.type3_budget = 0;
                    false
                }
                (None, false) => false,
                (None, true) => {
                    let mut meter = Meter {
                        forms: TYPE3_FORM_VISITS,
                        left: self.type3_budget,
                    };
                    let checked = match proc.decoded() {
                        Ok(data) => meter.charge(data.len()).and_then(|()| {
                            GlyphState::default().draws_direct(
                                &data,
                                t3.resources.as_ref(),
                                0,
                                &mut meter,
                            )
                        }),
                        Err(_) => Some(false),
                    };
                    let cost = self.type3_budget - meter.left;
                    self.type3_budget = meter.left;
                    if let Some(direct) = checked {
                        self.glyph_procs.insert(id, (direct, cost));
                    }
                    checked.unwrap_or(false)
                }
            };
            t3.uncacheable[code] = Some(direct);
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
    fn page(&mut self, page: &Page<'a>) {
        let xref = page.xref();
        if let Some(catalog) = xref.get::<Dict<'_>>(xref.root_id()) {
            self.ocg = OcgState::from_catalog(&catalog);
        }
        // Marked content by MCID takes ActualText from its structure element.
        let parents = page.raw().get::<i32>(STRUCT_PARENTS).and_then(|key| {
            let root = xref.get::<Dict<'_>>(xref.root_id())?;
            let tree = root
                .get::<Dict<'_>>(STRUCT_TREE_ROOT)?
                .get::<Dict<'_>>(PARENT_TREE)?;
            number_tree(&tree, key, 0)?.into_array()
        });
        if let Some(data) = page.page_stream() {
            let start = TextParams {
                size: 1.0,
                scale: 100.0,
                rise: 0.0,
                leading: 0.0,
                tm: Affine::IDENTITY,
                tlm: Affine::IDENTITY,
                render: 0,
                rewritten: false,
                font: ShowFont::None,
                pen: Pen::Known,
            };
            self.content(data, page.resources(), parents.is_some(), 0, start);
        }
        // One pass over the elements for the MCIDs the content marks:
        // reaching each through the array's items before it made a page of
        // n MCIDs quadratic, and holding every item costs the array's size.
        let pending = std::mem::take(&mut self.pending);
        let (Some(array), Some(last)) = (parents, pending.iter().map(|p| p.1).max()) else {
            return;
        };
        let mut wanted: HashMap<usize, Vec<usize>> = HashMap::new();
        for (mark, mcid) in pending {
            wanted.entry(mcid).or_default().push(mark);
        }
        for (i, item) in array.raw_iter().enumerate().take(last.saturating_add(1)) {
            let Some(marks) = wanted.remove(&i) else {
                continue;
            };
            let element = match item {
                MaybeRef::Ref(r) => xref.get::<Dict<'_>>(r.into()),
                MaybeRef::NotRef(object) => object.into_dict(),
            };
            if let Some(text) = element.and_then(|e| e.get::<PdfString<'_>>(ACTUAL_TEXT)) {
                for mark in marks {
                    let actual = self.actual_text(text.as_bytes());
                    if let Some(slot) = self.marks.get_mut(mark) {
                        slot.2 = actual;
                    }
                }
            }
            if wanted.is_empty() {
                break;
            }
        }
    }

    /// A form, drawn with the text state of the content that draws it;
    /// hayro draws only forms with a bounding box.
    fn form(&mut self, stream: &Stream<'a>, parent: &Resources<'a>, depth: u32, state: TextParams) {
        let id = stream.obj_id();
        let bbox = stream.dict().get::<[f32; 4]>(BBOX).is_some();
        if self.active.contains(&id) || !self.ocg.visible() || !bbox {
            return;
        }
        let Ok(data) = stream.decoded() else { return };
        let dict = stream.dict();
        let oc = dict.get::<Dict<'_>>(OC);
        if let Some(oc) = &oc {
            self.ocg.begin_oc(oc, dict.get_ref(OC).map(Into::into));
        }
        if self.ocg.visible() {
            let resources = dict
                .get::<Dict<'_>>(RESOURCES)
                .map_or_else(|| parent.clone(), Resources::new);
            self.active.push(id);
            // ponytail: MCIDs inside forms (their own StructParents) get no structure ActualText.
            self.content(&data, &resources, false, depth, state);
            self.active.pop();
        }
        if oc.is_some() {
            self.ocg.end();
        }
    }

    fn content(
        &mut self,
        data: &[u8],
        resources: &Resources<'a>,
        structure: bool,
        depth: u32,
        start: TextParams,
    ) {
        let mut stack = vec![start];
        let next_line = |top: &mut TextParams, tx: f64, ty: f64| {
            top.tlm *= Affine::translate((tx, ty));
            top.tm = top.tlm;
            top.pen = Pen::Known;
        };
        let mut fresh = false;
        let mut flushed = false;
        let mut ops = TypedIter::new(data);
        while let Some(op) = ops.next() {
            let Some(top) = stack.last_mut() else { break };
            // MuPDF flushes its text at a graphics state change, so the next
            // show starts a new text span.
            if self.open_object.is_some() && flushes_text(&op) {
                flushed = true;
            }
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
                    top.pen = Pen::Known;
                    fresh = true;
                    // MuPDF continues a text object that was never ended.
                    if self.open_object.is_none() {
                        self.objects += 1;
                        self.open_object = Some(self.objects);
                    }
                }
                TypedInstruction::EndText(_) => {
                    fresh = false;
                    if let Some(clip) = self
                        .open_object
                        .take()
                        .and_then(|o| self.clip_objects.get_mut(&o))
                    {
                        clip.closed = true;
                    }
                }
                // As hayro: a font the resources lack is its fallback.
                TypedInstruction::TextFont(t) => {
                    top.size = t.1.as_f32();
                    top.font = match resources.get_font(t.0) {
                        Some(dict) => ShowFont::Font(self.register(&dict), dict.obj_id()),
                        None => ShowFont::Fallback,
                    };
                }
                TypedInstruction::TextRenderingMode(r) => {
                    top.render = r.0.as_i64();
                    top.rewritten = depth == 0;
                }
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
                    top.pen = Pen::Known;
                }
                // `Tj` and `TJ` without a font show in hayro's fallback font.
                TypedInstruction::ShowText(s) => {
                    if top.font == ShowFont::None {
                        top.font = ShowFont::Fallback;
                    }
                    let p = *top;
                    let item = [Object::String(s.0.clone())];
                    let first = self.show(&mut fresh, p, item.into_iter());
                    let flush = std::mem::take(&mut flushed);
                    top.pen = self.queue_show(&p, &[(s.0.as_bytes(), false)], first, flush);
                }
                TypedInstruction::ShowTexts(a) => {
                    if top.font == ShowFont::None {
                        top.font = ShowFont::Fallback;
                    }
                    let p = *top;
                    let first = self.show(&mut fresh, p, a.0.iter::<Object<'_>>());
                    let items: Vec<Object<'_>> = a.0.iter::<Object<'_>>().collect();
                    let (mut strings, mut kerned, mut moves) = (Vec::new(), false, false);
                    for item in &items {
                        match item {
                            Object::String(s) if !s.as_bytes().is_empty() => {
                                strings.push((s.as_bytes(), std::mem::take(&mut kerned)));
                            }
                            Object::Number(n) => {
                                moves = true;
                                kerned |= n.as_f64() != 0.0;
                            }
                            _ => {}
                        }
                    }
                    let flush = std::mem::take(&mut flushed);
                    top.pen = self.queue_show(&p, &strings, first, flush);
                    // Adjustments alone move the pen by amounts not tracked.
                    if strings.is_empty() && moves {
                        top.pen = Pen::Unknown;
                    }
                }
                // `'` and `"` show nothing without a font.
                TypedInstruction::NextLineAndShowText(s) => {
                    next_line(top, 0.0, -f64::from(top.leading));
                    if top.font == ShowFont::None {
                        continue;
                    }
                    let p = *top;
                    let item = [Object::String(s.0.clone())];
                    let first = self.show(&mut fresh, p, item.into_iter());
                    let flush = std::mem::take(&mut flushed);
                    top.pen = self.queue_show(&p, &[(s.0.as_bytes(), false)], first, flush);
                }
                TypedInstruction::ShowTextWithParameters(t) => {
                    next_line(top, 0.0, -f64::from(top.leading));
                    if top.font == ShowFont::None {
                        continue;
                    }
                    let p = *top;
                    let item = [Object::String(t.2.clone())];
                    let first = self.show(&mut fresh, p, item.into_iter());
                    let flush = std::mem::take(&mut flushed);
                    top.pen = self.queue_show(&p, &[(t.2.as_bytes(), false)], first, flush);
                }
                TypedInstruction::BeginMarkedContent(bmc) => {
                    self.ocg.begin();
                    self.marks.push((bmc.0.as_ref().to_vec(), None, None));
                }
                TypedInstruction::EndMarkedContent(_) => self.ocg.end(),
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
                    // hayro's optional content: a properties entry or /OC given by reference.
                    let oc = match bdc.1 {
                        Object::Name(n) => resources.properties.get_ref(n.as_ref()).map(|r| {
                            let d = resources.properties.get::<Dict<'_>>(n.as_ref());
                            (d.unwrap_or_default(), r)
                        }),
                        Object::Dict(d) => d
                            .get_ref(OC)
                            .map(|r| (d.get::<Dict<'_>>(OC).unwrap_or_default(), r)),
                        _ => None,
                    };
                    match oc {
                        Some((dict, r)) => self.ocg.begin_oc(&dict, Some(r.into())),
                        None => self.ocg.begin(),
                    }
                    let inline = props.and_then(|p| p.get::<PdfString<'_>>(ACTUAL_TEXT));
                    let actual = inline.as_ref().and_then(|s| self.actual_text(s.as_bytes()));
                    // Else the structure element's, filled in after the content.
                    if let (None, true, Some(mcid)) = (
                        &inline,
                        structure,
                        mcid.and_then(|m| usize::try_from(m).ok()),
                    ) {
                        self.pending.push((self.marks.len(), mcid));
                    }
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
                        self.form(&stream, resources, depth + 1, *top);
                    }
                }
                _ => {}
            }
        }
    }

    /// The transform of a show's first glyph as hayro computes it (for a
    /// font written horizontally), and the start of a text object there.
    fn show<'o>(
        &mut self,
        fresh: &mut bool,
        p: TextParams,
        items: impl Iterator<Item = Object<'o>>,
    ) -> Option<Affine> {
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
                    if std::mem::take(fresh) {
                        *self.starts.entry(start_key(first)).or_default() += 1;
                    }
                    return Some(first);
                }
                _ => {}
            }
        }
        None
    }
}

/// The page content with every `7 Tr` followed by `4 Tr`, so hayro draws
/// clipping text (as fill and clip) and structured text sees its glyphs;
/// `None` when there is none. hayro draws nothing for mode 7, and draws a
/// form's content itself, so text set to mode 7 inside forms stays unread.
fn clip_mode_content(data: &[u8]) -> Option<Vec<u8>> {
    let base = data.as_ptr() as usize;
    let mut ops = UntypedIter::new(data);
    let mut out = Vec::new();
    let mut copied = 0;
    while let Some(op) = ops.next() {
        if &**op.operator != b"Tr" {
            continue;
        }
        // hayro reads the last operand, as `TypedIter` does.
        let seven = matches!(
            op.operands().last(),
            Some(Object::Number(n)) if n.as_i64() == 7
        );
        // The operator's bytes lie in `data` (an unescaped name).
        let end = (op.operator.as_ptr() as usize)
            .checked_sub(base)
            .map(|at| at + 2)
            .filter(|&end| end >= copied && data.get(end - 2..end) == Some(b"Tr"));
        if let (true, Some(end)) = (seven, end) {
            out.extend_from_slice(&data[copied..end]);
            out.extend_from_slice(b" 4 Tr ");
            copied = end;
        }
    }
    if copied == 0 {
        return None;
    }
    out.extend_from_slice(&data[copied..]);
    Some(out)
}

/// The operators inside a text object at which MuPDF's run processor flushes
/// its pending text (measured with MuPDF 1.28.1; not `Tf`, `Tr`, `Tm`, `i`,
/// `BX` or path construction).
fn flushes_text(op: &TypedInstruction<'_, '_>) -> bool {
    matches!(
        op,
        TypedInstruction::SaveState(_)
            | TypedInstruction::RestoreState(_)
            | TypedInstruction::Transform(_)
            | TypedInstruction::LineWidth(_)
            | TypedInstruction::LineCap(_)
            | TypedInstruction::LineJoin(_)
            | TypedInstruction::MiterLimit(_)
            | TypedInstruction::DashPattern(_)
            | TypedInstruction::RenderingIntent(_)
            | TypedInstruction::SetGraphicsState(_)
            | TypedInstruction::ColorSpaceStroke(_)
            | TypedInstruction::ColorSpaceNonStroke(_)
            | TypedInstruction::StrokeColor(_)
            | TypedInstruction::StrokeColorNamed(_)
            | TypedInstruction::NonStrokeColor(_)
            | TypedInstruction::NonStrokeColorNamed(_)
            | TypedInstruction::StrokeColorDeviceGray(_)
            | TypedInstruction::NonStrokeColorDeviceGray(_)
            | TypedInstruction::StrokeColorDeviceRgb(_)
            | TypedInstruction::NonStrokeColorDeviceRgb(_)
            | TypedInstruction::StrokeColorCmyk(_)
            | TypedInstruction::NonStrokeColorCmyk(_)
            | TypedInstruction::MarkedContentPoint(_)
            | TypedInstruction::MarkedContentPointWithProperties(_)
    )
}

fn start_key(t: Affine) -> [i64; 6] {
    t.as_coeffs().map(|v| (v * 1e6).round() as i64)
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

/// A Type3 font's advances by code, from /Widths through the FontMatrix,
/// and its uncacheable glyphs.
fn type3_codes<'a>(dict: &Dict<'a>, font: usize) -> Type3Codes<'a> {
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
        widths,
        uncacheable: vec![None; 256],
        names: text_unicode::encoding_names(dict.get::<Object<'_>>(ENCODING), None, true),
        procs: dict.get::<Dict<'_>>(CHAR_PROCS),
        resources: dict.get::<Dict<'_>>(RESOURCES),
    }
}

/// Forms one Type3 glyph procedure's check may read, nested ones included.
const TYPE3_FORM_VISITS: u32 = 16;

/// What a Type3 glyph procedure's check may still read: forms, and
/// decoded bytes of the page's budget.
struct Meter {
    forms: u32,
    left: u64,
}

impl Meter {
    /// Charges `bytes`; `None` past the budget, which is then spent.
    fn charge(&mut self, bytes: usize) -> Option<()> {
        let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
        if bytes > self.left {
            self.left = 0;
            return None;
        }
        self.left -= bytes;
        Some(())
    }
}

/// What a Type3 glyph procedure has set so far, for
/// [`GlyphState::draws_direct`].
#[derive(Clone, Copy)]
struct GlyphState {
    fill: bool,
    stroke: bool,
    width: bool,
    join: bool,
    caps: bool,
    miter: bool,
    dash: bool,
    dashed: bool,
    mitred: bool,
}

impl Default for GlyphState {
    fn default() -> Self {
        Self {
            fill: false,
            stroke: false,
            width: false,
            join: false,
            caps: false,
            miter: false,
            dash: false,
            dashed: false,
            mitred: true,
        }
    }
}

impl GlyphState {
    /// Whether MuPDF draws a Type3 glyph procedure directly instead of
    /// caching it (pdf-op-run.c device flags): a stroke whose colour, join,
    /// width or dash it never set, a dashed stroke without caps, a mitred
    /// one without a limit, or a fill of a coloured (d0) glyph that sets no
    /// fill colour; also inside the forms it draws, which see the state at
    /// their `Do`. `None` past the meter's budget.
    fn draws_direct(
        mut self,
        data: &[u8],
        resources: Option<&Dict<'_>>,
        depth: u32,
        meter: &mut Meter,
    ) -> Option<bool> {
        let mut ops = TypedIter::new(data);
        while let Some(op) = ops.next() {
            let (strokes, fills) = match op {
                TypedInstruction::ShapeGlyph(_) => {
                    (self.fill, self.stroke) = (true, true);
                    (false, false)
                }
                // MuPDF reads `d1` whatever its operands.
                TypedInstruction::Fallback(op) if &**op == b"d1" => {
                    (self.fill, self.stroke) = (true, true);
                    (false, false)
                }
                TypedInstruction::LineWidth(_) => {
                    self.width = true;
                    (false, false)
                }
                TypedInstruction::LineJoin(j) => {
                    self.join = true;
                    self.mitred = j.0.as_i64() == 0;
                    (false, false)
                }
                TypedInstruction::LineCap(_) => {
                    self.caps = true;
                    (false, false)
                }
                TypedInstruction::MiterLimit(_) => {
                    self.miter = true;
                    (false, false)
                }
                TypedInstruction::DashPattern(d) => {
                    self.dash = true;
                    self.dashed = d.0.iter::<Object<'_>>().next().is_some();
                    (false, false)
                }
                TypedInstruction::ColorSpaceStroke(_)
                | TypedInstruction::StrokeColor(_)
                | TypedInstruction::StrokeColorNamed(_)
                | TypedInstruction::StrokeColorDeviceGray(_)
                | TypedInstruction::StrokeColorDeviceRgb(_)
                | TypedInstruction::StrokeColorCmyk(_) => {
                    self.stroke = true;
                    (false, false)
                }
                TypedInstruction::ColorSpaceNonStroke(_)
                | TypedInstruction::NonStrokeColor(_)
                | TypedInstruction::NonStrokeColorNamed(_)
                | TypedInstruction::NonStrokeColorDeviceGray(_)
                | TypedInstruction::NonStrokeColorDeviceRgb(_)
                | TypedInstruction::NonStrokeColorCmyk(_) => {
                    self.fill = true;
                    (false, false)
                }
                TypedInstruction::StrokePath(_) | TypedInstruction::CloseAndStrokePath(_) => {
                    (true, false)
                }
                TypedInstruction::FillPathNonZero(_)
                | TypedInstruction::FillPathNonZeroCompatibility(_)
                | TypedInstruction::FillPathEvenOdd(_) => (false, true),
                TypedInstruction::FillAndStrokeNonZero(_)
                | TypedInstruction::FillAndStrokeEvenOdd(_)
                | TypedInstruction::CloseFillAndStrokeNonZero(_)
                | TypedInstruction::CloseFillAndStrokeEvenOdd(_) => (true, true),
                TypedInstruction::XObject(x) if depth < FORM_DEPTH && meter.forms > 0 => {
                    let form = resources
                        .and_then(|r| r.get::<Dict<'_>>(XOBJECT))
                        .and_then(|x_objects| x_objects.get::<Stream<'_>>(x.0.as_ref()))
                        .filter(|s| {
                            s.dict()
                                .get::<Name<'_>>(SUBTYPE)
                                .is_some_and(|n| n.as_ref() == b"Form")
                        });
                    if let Some(form) = form {
                        meter.forms -= 1;
                        let inner = form.dict().get::<Dict<'_>>(RESOURCES);
                        let inner = inner.as_ref().or(resources);
                        if let Ok(data) = form.decoded() {
                            meter.charge(data.len())?;
                            if self.draws_direct(&data, inner, depth + 1, meter)? {
                                return Some(true);
                            }
                        }
                    }
                    (false, false)
                }
                _ => (false, false),
            };
            let s = &self;
            if strokes
                && (!s.stroke
                    || !s.join
                    || !s.width
                    || !s.dash
                    || (s.dashed && !s.caps)
                    || (s.mitred && !s.miter))
            {
                return Some(true);
            }
            if fills && !s.fill {
                return Some(true);
            }
        }
        Some(false)
    }
}

/// The font (a composite font's descendant) whose descriptor holds the program.
fn program_font<'a>(dict: &Dict<'a>) -> Dict<'a> {
    let is_type0 = dict
        .get::<Name<'_>>(SUBTYPE)
        .is_some_and(|n| n.as_ref() == TYPE0);
    if is_type0 {
        dict.get::<Array<'_>>(DESCENDANT_FONTS)
            .and_then(|a| a.iter::<Dict<'_>>().next())
            .unwrap_or_else(|| dict.clone())
    } else {
        dict.clone()
    }
}

/// The embedded program of a font dictionary (not a Type3 font).
fn font_program<'a>(dict: &Dict<'a>) -> Option<Stream<'a>> {
    if is_type3(dict) {
        return None;
    }
    let descriptor = program_font(dict).get::<Dict<'_>>(FONT_DESC)?;
    [FONT_FILE, FONT_FILE2, FONT_FILE3]
        .iter()
        .find_map(|k| descriptor.get::<Stream<'_>>(*k))
}

/// MuPDF's facts for a font dictionary: FontDescriptor Ascent/Descent win over
/// the font program; style flags come from the program (OS/2, post) when
/// embedded. `embedded`: the descriptor names a program, `data` its bytes.
fn font_facts(dict: &Dict<'_>, embedded: bool, data: Option<&[u8]>) -> TextFont {
    if is_type3(dict) {
        return type3_facts(dict);
    }
    let font = program_font(dict);
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
    let mut facts = match (embedded, data) {
        (true, data) => data
            .and_then(sfnt_facts)
            // ponytail: bare CFF and Type1 programs are not parsed; FreeType
            // would take extents from the FontBBox and style from FontInfo.
            .unwrap_or_else(|| TextFont {
                italic: flags & (1 << 6) != 0,
                bold: flags & (1 << 18) != 0,
                monospace: flags & 1 != 0,
                ..default_font()
            }),
        (false, _) => standard_font(&base, flags),
    };
    if let Some(d) = &descriptor {
        if !embedded && d.get::<f32>(FONT_WEIGHT).is_some_and(|w| w > 500.0) {
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
    facts
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
    font: FontId,
    linear: [f64; 4],
    /// The run's last glyph was drawn directly (a Type3 glyph MuPDF does not cache).
    direct: bool,
    /// The text rendering mode of its show, when known.
    render: Option<i64>,
}

/// Which font drew a run: hayro's key, and the font object when known.
#[derive(Clone, Copy, PartialEq)]
struct FontId {
    key: Option<u128>,
    obj: Option<ObjectIdentifier>,
}

impl FontId {
    fn differs(self, other: Self) -> bool {
        self.key != other.key
            || (self.obj.is_some() && other.obj.is_some() && self.obj != other.obj)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DrawKind {
    Fill,
    Stroke,
    Invisible,
}

struct Capture<'s, 'd> {
    scan: &'s mut Scan<'d>,
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
    /// Clipping text of the current text object.
    clip_text: Option<PendingClip>,
    /// The text object of the last run drawn.
    text_object: Option<u32>,
}

struct ActualSpan {
    text: Vec<char>,
    glyphs: Vec<GlyphEvent>,
    all: Vec<GlyphEvent>,
    /// Device-space bounds of the images and shadings drawn inside.
    bounds: Option<Rect>,
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
                        fresh: false,
                        ..at.clone()
                    });
                }
            }
        }
        out.extend_from_slice(&units[n - suf..]);
        out
    }
}

impl Capture<'_, '_> {
    fn clip(&self) -> Rect {
        self.clips.last().copied().unwrap_or(Rect::ZERO)
    }

    fn grow_actual(&mut self, rect: Rect) {
        if let Some(span) = self.actual.iter_mut().flatten().next()
            && rect.is_finite()
        {
            span.bounds = Some(span.bounds.map_or(rect, |b| b.union(rect)));
        }
    }

    /// Extracts the pending clipping text, as MuPDF does at the end of its
    /// text object: a new object after everything the object drew.
    fn flush_clip(&mut self) {
        let Some(clip) = self.clip_text.take() else {
            return;
        };
        let extracted = self.scan.clip_objects.get(&clip.object);
        if !extracted.is_some_and(ClipObject::extracted) {
            return;
        }
        if self.overflow || self.chars.saturating_add(clip.events.len()) > MAX_PAGE_CHARS {
            self.overflow = true;
            return;
        }
        self.chars += clip.events.len();
        let (kept, all) = match self.actual.iter_mut().flatten().next() {
            Some(span) => (&mut span.glyphs, &mut span.all),
            None => (&mut self.glyphs, &mut self.all),
        };
        let mut kept_start = true;
        for (i, (event, keep)) in clip.events.into_iter().enumerate() {
            if keep {
                kept.push(GlyphEvent {
                    span_start: kept_start && !event.cluster,
                    fresh: kept_start && !event.cluster,
                    ..event.clone()
                });
                kept_start &= event.cluster;
            }
            all.push(GlyphEvent {
                span_start: i == 0,
                fresh: i == 0,
                ..event
            });
        }
        self.last = None;
        self.other = true;
    }

    /// ActualText over no text, as MuPDF places it: every character at the
    /// bottom left of the images and shadings the span drew (nothing without
    /// one), an em as wide and high as those, without advance, in the last
    /// font used (Helvetica before any).
    fn place_actual(&mut self, span: &ActualSpan) {
        let Some(b) = span.bounds.filter(|b| b.width() > 0.0 || b.height() > 0.0) else {
            return;
        };
        let trm = Affine::new([b.width(), 0.0, 0.0, b.y0 - b.y1, b.x0, b.y1]);
        let font = self.all.last().map_or(FALLBACK_FONT, |g| g.font);
        // One line at that origin: the characters after the first follow
        // it as one glyph's would, so none reads as a fake-bold repeat.
        for (i, &c) in span.text.iter().enumerate() {
            let event = GlyphEvent {
                c,
                trm,
                adv: 0.0,
                font,
                cluster: i > 0,
                span_start: i == 0,
                fresh: i == 0,
                kerned: false,
            };
            self.glyphs.push(event.clone());
            self.all.push(event);
        }
    }

    /// The show operation a glyph run whose first glyph has transform
    /// `first` was drawn from: its codes, one slot per glyph, and the text of
    /// its font (none for hayro's fallback font, whose reading stands).
    fn run_codes(&mut self, key: Option<u128>, glyphs: usize, first: Affine) -> Option<RunCodes> {
        let show = self.scan.take_show(key, glyphs, first)?;
        let mut text = self.scan.texts.get(&show.key).cloned();
        let (mut slots, mut kerns) = (Vec::new(), Vec::new());
        let range = show.start..show.start + show.len;
        if let (Some(font), Some(codes), Some(kerned)) = (
            &text,
            self.scan.codes.get(range.clone()),
            self.scan.kerns.get(range),
        ) {
            slots.reserve(glyphs);
            for (code, &kerned) in codes.iter().zip(kerned) {
                slots.push(Slot::Code(*code));
                kerns.push(kerned);
                if font.one_byte_glyphs() {
                    slots.extend((1..code.bytes).map(|_| Slot::Merged));
                    kerns.extend((1..code.bytes).map(|_| false));
                }
            }
        }
        if slots.len() != glyphs {
            (text, slots, kerns) = (None, Vec::new(), Vec::new());
        }
        Some(RunCodes {
            key: show.key,
            obj: show.obj,
            text,
            slots,
            kerns,
            render: show.render,
            object: show.object,
            flushed: show.flushed,
        })
    }
}

/// The codes of a glyph run and the text of its font.
struct RunCodes {
    key: u128,
    obj: Option<ObjectIdentifier>,
    text: Option<Rc<FontText>>,
    slots: Vec<Slot>,
    /// Whether a `TJ` adjustment comes before each slot's glyph.
    kerns: Vec<bool>,
    render: i64,
    object: u32,
    flushed: bool,
}

/// The clipping text of one text object, extracted at its end: each
/// glyph's first character (MuPDF's clip text has no one-to-many fillers),
/// and whether it is inside the page and its clips.
struct PendingClip {
    object: u32,
    events: Vec<(GlyphEvent, bool)>,
}

#[derive(Clone, Copy)]
enum Slot {
    Code(Code),
    /// A further byte of the previous glyph's code (S22PDF's Chinese fonts).
    Merged,
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

impl<'a> Device<'a> for Capture<'_, '_> {
    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, _: &DrawMode) {
        self.flush_clip();
        self.other = true;
        // MuPDF draws a shading (`sh`, or a shading pattern's paint) as an
        // image-like shade, which ActualText over no text is placed on.
        if let Paint::Pattern(pattern) = &props.paint
            && let Pattern::Shading(_) = &**pattern
            && self.actual.iter().flatten().next().is_some()
        {
            self.grow_actual((props.transform * path.clone()).bounding_box());
        }
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        self.flush_clip();
        let next = self.clip().intersect(clip_path.path.bounding_box());
        self.clips.push(next);
        self.other = true;
    }

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {
        self.flush_clip();
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
        let run = self.run_codes(font_key, glyphs.len(), first_t);
        let (render, object) = run
            .as_ref()
            .map_or((0, None), |r| (r.render, Some(r.object)));
        if self
            .clip_text
            .as_ref()
            .is_some_and(|c| Some(c.object) != object)
        {
            self.flush_clip();
        }
        let font_id = FontId {
            key: run.as_ref().map(|r| r.key).or(font_key),
            obj: run.as_ref().and_then(|r| r.obj),
        };
        let starts_object = self.scan.take_start(first_t);
        // Text in another rendering mode is another span, as in MuPDF.
        let new_object = starts_object
            || run.as_ref().is_some_and(|r| r.flushed)
            || self
                .last
                .is_some_and(|m| m.render.is_some() && m.render != run.as_ref().map(|r| r.render))
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
                .is_none_or(|m| m.font.differs(font_id) || !same_matrix(&m));
        let slot = |i: usize| run.as_ref().and_then(|r| r.slots.get(i).copied());
        let kerned = |i: usize| run.as_ref().is_some_and(|r| r.kerns.get(i) == Some(&true));
        let type3 = run.as_ref().and_then(|r| self.scan.type3.get(&r.key));
        // MuPDF draws an uncacheable Type3 glyph directly, in a span of its
        // own, and the glyph after it starts another.
        let direct: Vec<bool> = (0..glyphs.len())
            .map(|i| match (type3, slot(i)) {
                (Some(t3), Some(Slot::Code(c))) => t3
                    .uncacheable
                    .get(c.code as usize)
                    .copied()
                    .flatten()
                    .unwrap_or(false),
                _ => false,
            })
            .collect();
        let after_direct = !new_object && self.last.is_some_and(|m| m.direct);
        let mut items: Vec<(Affine, f64, usize)> = glyphs
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let trm = props.transform * g.transform() * em;
                match (&**g, type3, slot(i)) {
                    (Glyph::Outline(o), _, _) => (
                        trm,
                        f64::from(o.advance_width().unwrap_or(0.0)) / 1000.0,
                        self.scan.font_index(o.font_cache_key()),
                    ),
                    (Glyph::Type3(_), Some(t3), Some(Slot::Code(c))) => (
                        trm,
                        t3.widths.get(c.code as usize).copied().unwrap_or(f64::NAN),
                        t3.font,
                    ),
                    (Glyph::Type3(_), ..) => (trm, f64::NAN, TYPE3_FONT),
                }
            })
            .collect();
        // A merged glyph's advance belongs to its code's first glyph.
        for i in 1..items.len() {
            if matches!(slot(i), Some(Slot::Merged)) {
                let adv = std::mem::take(&mut items[i].1);
                items[i - 1].1 += adv;
            }
        }
        if let Some(adv) = run.as_ref().and_then(|r| r.text.as_ref()?.advance()) {
            for (i, item) in items.iter_mut().enumerate() {
                item.1 = if matches!(slot(i), Some(Slot::Merged)) {
                    0.0
                } else {
                    adv
                };
            }
        }
        let mapped: Vec<Vec<char>> = glyphs
            .iter()
            .enumerate()
            .map(
                |(i, g)| match (run.as_ref().and_then(|r| r.text.as_ref()), slot(i)) {
                    (_, Some(Slot::Merged)) => Vec::new(),
                    (Some(text), Some(Slot::Code(code))) => {
                        text.text(code).unwrap_or_else(|| unicode(g.as_unicode()))
                    }
                    _ => unicode(g.as_unicode()),
                },
            )
            .collect();
        let expanded = |mapped: &[char]| {
            let mut out = Vec::new();
            for &c in mapped {
                text_unicode::expand(c, &mut out);
            }
            out
        };
        let chars: Vec<Vec<char>> = mapped.iter().map(|m| expanded(m)).collect();
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
        // MuPDF drops a glyph whose bounds lie wholly outside the clip; an
        // inkless glyph's bounds are its origin. Glyph outlines are only
        // measured when the em boxes, grown by half an em, are not all inside.
        let clip = self.clip();
        let inside =
            |r: &Rect| r.x0 >= clip.x0 && r.y0 >= clip.y0 && r.x1 <= clip.x1 && r.y1 <= clip.y1;
        let em_box = |(trm, adv, font): &(Affine, f64, usize), grow: f64| {
            let f = &self.scan.fonts[*font];
            let r = Rect::new(-grow, f.descender - grow, adv + grow, f.ascender + grow);
            transformed_bounds(*trm, r)
        };
        let keep: Vec<bool> = if self.blank {
            vec![false; items.len()]
        } else if items.iter().all(|i| inside(&em_box(i, 0.5))) {
            vec![true; items.len()]
        } else {
            glyphs
                .iter()
                .zip(&items)
                .map(|(g, item)| {
                    let r = match &**g {
                        Glyph::Outline(o) => {
                            let outline = props.transform * g.transform() * o.outline();
                            if outline.segments().next().is_some() {
                                outline.bounding_box()
                            } else {
                                let origin = props.transform * g.transform() * Point::ZERO;
                                Rect::from_points(origin, origin)
                            }
                        }
                        Glyph::Type3(_) => em_box(item, 0.0),
                    };
                    r.x1 >= clip.x0 && r.y1 >= clip.y0 && r.x0 <= clip.x1 && r.y0 <= clip.y1
                })
                .collect()
        };
        // Clipping text drawn in no other way is extracted again at the end
        // of its text object; MuPDF also repeats the text of modes 4 to 6.
        if render == 7 {
            let clip = self.clip_text.get_or_insert_with(|| PendingClip {
                object: object.unwrap_or(0),
                events: Vec::new(),
            });
            for (g, ((mapped, (trm, adv, font)), keep)) in
                mapped.iter().zip(&items).zip(&keep).enumerate()
            {
                let first = expanded(&mapped[..mapped.len().min(1)]);
                for (i, c) in first.into_iter().enumerate() {
                    let event = GlyphEvent {
                        c,
                        trm: *trm,
                        adv: if i == 0 { *adv } else { 0.0 },
                        font: *font,
                        cluster: i > 0,
                        span_start: false,
                        fresh: false,
                        kerned: i == 0 && kerned(g),
                    };
                    clip.events.push((event, *keep));
                }
            }
            if clip.events.len() > MAX_PAGE_CHARS {
                self.overflow = true;
            }
            // Mode 7 text is only the clip (`clip_mode_content` draws it as 4).
            return;
        }
        let count = chars.iter().map(Vec::len).fold(0, usize::saturating_add);
        if self.overflow || self.chars.saturating_add(count) > MAX_PAGE_CHARS {
            self.overflow = true;
            return;
        }
        self.chars += count;
        self.last = Some(RunMark {
            count: glyphs.len(),
            first: first_t,
            last: last_t,
            mode: kind,
            paint,
            font: font_id,
            linear,
            direct: direct.last().copied().unwrap_or(false),
            render: run.as_ref().map(|r| r.render),
        });
        self.other = false;
        let fresh = match object {
            Some(o) => self.text_object.replace(o) != Some(o),
            None => starts_object,
        };
        let (mut kept_fresh, mut all_fresh) = (fresh, fresh);
        let mut all_start = span_start;
        for (g, ((chars, (trm, adv, font)), keep)) in
            chars.into_iter().zip(items).zip(keep).enumerate()
        {
            let before = if g == 0 { after_direct } else { direct[g - 1] };
            if direct[g] || before {
                (span_start, all_start) = (true, true);
            }
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
                    fresh: false,
                    kerned: i == 0 && kerned(g),
                };
                if keep {
                    kept.push(GlyphEvent {
                        span_start: span_start && i == 0,
                        fresh: std::mem::take(&mut kept_fresh),
                        ..event.clone()
                    });
                }
                all.push(GlyphEvent {
                    span_start: all_start && i == 0,
                    fresh: std::mem::take(&mut all_fresh),
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
                bounds: None,
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
            if span.all.is_empty() {
                self.place_actual(&span);
                return;
            }
            self.glyphs
                .extend(ActualSpan::resolve(&span.text, &span.glyphs));
            self.all.extend(ActualSpan::resolve(&span.text, &span.all));
        }
    }

    fn draw_image(&mut self, image: Image<'a, '_>, props: ImageDrawProps<'a>) {
        self.flush_clip();
        self.other = true;
        // The transform maps the image's pixel grid.
        let pixels = Rect::new(
            0.0,
            0.0,
            f64::from(image.width()),
            f64::from(image.height()),
        );
        self.grow_actual(transformed_bounds(props.transform, pixels));
    }

    fn pop_clip(&mut self) {
        self.flush_clip();
        if self.clips.len() > 1 {
            self.clips.pop();
        }
        self.other = true;
    }

    fn pop_transparency_group(&mut self) {
        self.flush_clip();
        self.other = true;
    }
}

/// A glyph's characters as hayro reads them; unknown glyphs read U+FFFD,
/// as do control characters (MuPDF has no name for them), but white-space
/// controls read as a space.
fn unicode(mapped: Option<BfString>) -> Vec<char> {
    let mut chars: Vec<char> = match mapped {
        Some(BfString::Char(c)) => vec![c],
        Some(BfString::String(s)) if !s.is_empty() => s.chars().collect(),
        _ => vec!['\u{fffd}'],
    };
    for c in &mut chars {
        *c = text_unicode::control_free(*c);
    }
    chars
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page showing codes 0..=255 of a Type3 font whose 256 glyph names
    /// map to `procs` glyph procedures (name i to procedure i mod procs),
    /// each a fill with no colour set (drawn directly in MuPDF).
    fn type3_document(procs: usize, shown: &str) -> ViewDocument {
        let names: Vec<String> = (0..256).map(|i| format!("/g{i}")).collect();
        let entries: Vec<String> = (0..256)
            .map(|i| format!("/g{i} {} 0 R", 6 + i % procs))
            .collect();
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] \
             /Resources << /Font << /T3 5 0 R >> >> /Contents 4 0 R >>"
                .to_string(),
        ];
        let content = format!("BT /T3 1 Tf 10 100 Td <{shown}> Tj ET");
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ));
        objects.push(format!(
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] \
             /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << {} >> \
             /Encoding << /Differences [0 {}] >> /FirstChar 0 /LastChar 255 \
             /Widths [{}] >>",
            entries.join(" "),
            names.join(" "),
            ["500"; 256].join(" ")
        ));
        for i in 0..procs {
            let data = format!("500 0 d0 0 0 m {i} 750 l 375 0 l f");
            objects.push(format!(
                "<< /Length {} >>\nstream\n{data}\nendstream",
                data.len()
            ));
        }
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for offset in offsets {
            out.extend(format!("{offset:010} 00000 n \n").bytes());
        }
        out.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .bytes(),
        );
        ViewDocument::open(out, "").unwrap()
    }

    fn scanned<'a>(doc: &'a ViewDocument, budget: u64) -> Scan<'a> {
        let mut scan = Scan::new();
        scan.type3_budget = budget;
        scan.page(doc.page(0).unwrap());
        scan
    }

    fn direct(scan: &Scan<'_>) -> Vec<Option<bool>> {
        scan.type3.values().next().unwrap().uncacheable.clone()
    }

    #[test]
    fn type3_glyph_procedures_are_checked_once_each_for_the_codes_shown() {
        let all: String = (0..=255u8).map(|b| format!("{b:02x}")).collect();
        // 256 names, one procedure: read and charged once.
        let doc = type3_document(1, &all);
        let scan = scanned(&doc, TYPE3_CHECK_BYTES);
        assert_eq!(scan.glyph_procs.len(), 1);
        let cost = "500 0 d0 0 0 m 0 750 l 375 0 l f".len() as u64;
        assert_eq!(TYPE3_CHECK_BYTES - scan.type3_budget, cost);
        assert!(direct(&scan).iter().all(|d| *d == Some(true)));
        // Only the codes shown are checked.
        let doc = type3_document(16, "0102");
        let scan = scanned(&doc, TYPE3_CHECK_BYTES);
        assert_eq!(scan.glyph_procs.len(), 2);
        let checked = direct(&scan).iter().filter(|d| d.is_some()).count();
        assert_eq!(checked, 2);
        // Past the page's budget glyphs read as cached, and nothing is kept.
        let doc = type3_document(16, &all);
        let scan = scanned(&doc, 3 * cost + cost / 2);
        assert_eq!(scan.glyph_procs.len(), 3);
        assert_eq!(scan.type3_budget, 0);
        let shown = direct(&scan);
        assert_eq!(shown.iter().filter(|d| **d == Some(true)).count(), 3 * 16);
        assert!(shown.iter().all(Option::is_some));
        // A reader's later page charges what it costs without reading it.
        let mut again = Scan::new();
        again.glyph_procs = scan.glyph_procs.clone();
        again.type3_budget = 3 * cost + cost / 2;
        again.page(doc.page(0).unwrap());
        assert_eq!(direct(&again), shown);
        assert_eq!(again.type3_budget, 0);
    }
}
