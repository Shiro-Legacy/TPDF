// SPDX-License-Identifier: AGPL-3.0-or-later
//! Structured text from hayro glyph runs, following the MuPDF 1.28 stext
//! device: blocks of lines of characters with page-space quads, plain text,
//! the preserve-spans line summaries the app draws, and search hits.
//! Layout thresholds and font facts were measured against MuPDF 1.28.1.
use super::{PageFrame, ViewDocument, ViewError};
use hayro::kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2};
use hayro_interpret::font::{Glyph, GlyphRun};
use hayro_interpret::hayro_cmap::BfString;
use hayro_interpret::{
    BlendMode, CacheKey, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, Paint, SoftMask, interpret_page,
};
use hayro_syntax::content::TypedIter;
use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::object::dict::keys::{
    ACTUAL_TEXT, ASCENT, BASE_FONT, DESCENDANT_FONTS, DESCENT, FLAGS, FONT_DESC, FONT_FILE,
    FONT_FILE2, FONT_FILE3, FONT_WEIGHT, KIDS, MCID, NUMS, PARENT_TREE, RESOURCES, STRUCT_PARENTS,
    STRUCT_TREE_ROOT, SUBTYPE, TYPE0,
};
use hayro_syntax::object::{Array, Dict, Name, Object, Stream, String as PdfString};
use hayro_syntax::page::{Page, Resources};
use std::collections::HashMap;

const PARAGRAPH_DIST: f64 = 1.5;
const SPACE_DIST: f64 = 0.15;
const SPACE_MAX_DIST: f64 = 0.8;
const BASE_MAX_DIST: f64 = 0.8;
const FAKE_BOLD_MAX_DIST: f64 = 0.1;
/// A new line starting this far (points) right of the previous one opens a paragraph.
const INDENT_DIST: f64 = 0.5;
/// Quads one `page.search` call returns at most in the app's MuPDF build.
pub const MAX_SEARCH_QUADS: usize = 500;

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
    pub w: i32,
    pub h: i32,
    pub size: i32,
    pub family: &'static str,
    pub weight: &'static str,
    pub style: &'static str,
}

/// The text of one page. `blocks` is MuPDF's preserve-whitespace layout.
#[derive(Clone, Debug)]
pub struct StructuredText {
    glyphs: Vec<GlyphEvent>,
    fonts: Vec<TextFont>,
    blocks: Vec<TextBlock>,
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
        let page = self.page(page)?;
        let frame = PageFrame::new(page);
        let mut scan = Scan::new();
        scan.page(page);
        let bounds = Rect::new(0.0, 0.0, f64::from(frame.width), f64::from(frame.height));
        let cache = InterpreterCache::new();
        // MuPDF's structured text covers page contents only, not annotations.
        let settings = InterpreterSettings {
            render_annotations: false,
            ..InterpreterSettings::default()
        };
        let mut ctx = Context::new(frame.ctm, bounds, &cache, page.xref(), settings);
        let mut capture = Capture {
            scan: &mut scan,
            clips: vec![bounds],
            glyphs: Vec::new(),
            last: None,
            other: false,
            mark: 0,
            actual: Vec::new(),
        };
        interpret_page(page, &mut ctx, &mut capture);
        while !capture.actual.is_empty() {
            capture.end_marked_content();
        }
        let glyphs = capture.glyphs;
        Ok(StructuredText::new(glyphs, scan.fonts))
    }
}

impl StructuredText {
    fn new(glyphs: Vec<GlyphEvent>, fonts: Vec<TextFont>) -> Self {
        let blocks = layout(&glyphs, &fonts, false, true);
        Self {
            glyphs,
            fonts,
            blocks,
        }
    }

    pub fn blocks(&self) -> &[TextBlock] {
        &self.blocks
    }

    pub fn fonts(&self) -> &[TextFont] {
        &self.fonts
    }

    /// MuPDF's `asText()`: a newline after every line and another after every block.
    pub fn as_text(&self) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            for line in &block.lines {
                out.extend(line.chars.iter().map(|ch| ch.c));
                out.push('\n');
            }
            out.push('\n');
        }
        out
    }

    /// The preserve-spans lines with integer geometry, as the app's `parseLines`.
    pub fn line_summaries(&self) -> Vec<LineSummary> {
        let trunc = |v: f32| v as i32;
        let mut out = Vec::new();
        for block in layout(&self.glyphs, &self.fonts, true, false) {
            for line in block.lines {
                let Some(first) = line.chars.first() else {
                    continue;
                };
                let font = &self.fonts[first.font];
                let [x0, y0, x1, y1] = line.bbox.map(|v| v as f32);
                out.push(LineSummary {
                    text: line.chars.iter().map(|ch| ch.c).collect(),
                    x: trunc(x0),
                    y: trunc(first.origin[1] as f32),
                    w: trunc(x1 - x0),
                    h: trunc(y1 - y0),
                    size: trunc(first.size as f32),
                    family: if font.monospace {
                        "monospace"
                    } else if font.serif {
                        "serif"
                    } else {
                        "sans-serif"
                    },
                    weight: if font.bold { "bold" } else { "normal" },
                    style: if font.italic { "italic" } else { "normal" },
                });
            }
        }
        out
    }

    /// Case-sensitive search as the app's MuPDF `page.search`: whitespace runs
    /// match any whitespace, a line break reads as a space and a hyphen ending a
    /// line joins it to the next. Each hit is a list of quads (ulx, uly, urx,
    /// ury, llx, lly, lrx, lry), one per run of adjacent characters; at most
    /// `max_quads` quads in all, as MuPDF counts its hit limit.
    pub fn search(&self, needle: &str, max_quads: usize) -> Vec<Vec<[f64; 8]>> {
        let needle: Vec<char> = needle.chars().collect();
        if needle.is_empty() || max_quads == 0 {
            return Vec::new();
        }
        let mut hay: Vec<Hay<'_>> = Vec::new();
        for block in &self.blocks {
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
        let mut hits = Vec::new();
        let mut total = 0;
        let mut at = 0;
        while at < hay.len() && total < max_quads {
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
            quads.truncate(max_quads - total);
            total += quads.len();
            if !quads.is_empty() {
                hits.push(quads);
            }
            at = end.max(at + 1);
        }
        hits
    }
}

/// A searchable character and, for drawn ones, its char and line direction.
type Hay<'a> = (char, Option<(&'a TextChar, [f64; 2])>);

fn match_at(hay: &[Hay<'_>], start: usize, needle: &[char]) -> Option<usize> {
    let (mut h, mut n) = (start, 0);
    while n < needle.len() {
        let c = hay.get(h)?.0;
        if is_space(needle[n]) {
            if !is_space(c) {
                return None;
            }
            while n < needle.len() && is_space(needle[n]) {
                n += 1;
            }
            while h < hay.len() && is_space(hay[h].0) {
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

/// Font facts, text-object starts and marked-content spans gathered from the
/// content hayro interprets.
struct Scan {
    fonts: Vec<TextFont>,
    by_key: HashMap<u128, usize>,
    starts: HashMap<[i64; 6], u32>,
    /// BMC/BDC in content order.
    marks: Vec<Mark>,
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
            starts: HashMap::new(),
            marks: Vec::new(),
        }
    }

    fn font_index(&self, key: u128) -> usize {
        self.by_key.get(&key).copied().unwrap_or(FALLBACK_FONT)
    }

    fn register(&mut self, dict: &Dict<'_>) {
        let key = dict.cache_key();
        if !self.by_key.contains_key(&key) {
            self.by_key.insert(key, self.fonts.len());
            self.fonts.push(font_facts(dict));
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
            number_tree(&tree, key, 0)?.into_array()
        });
        if let Some(data) = page.page_stream() {
            self.content(data, page.resources(), parents.as_ref(), 0);
        }
    }

    fn form(&mut self, stream: &Stream<'_>, parent: &Resources<'_>, depth: u32) {
        let Ok(data) = stream.decoded() else { return };
        let resources = stream
            .dict()
            .get::<Dict<'_>>(RESOURCES)
            .map_or_else(|| parent.clone(), Resources::new);
        // ponytail: MCIDs inside forms (their own StructParents) get no structure ActualText.
        self.content(&data, &resources, None, depth);
    }

    fn content(
        &mut self,
        data: &[u8],
        resources: &Resources<'_>,
        parents: Option<&Array<'_>>,
        depth: u32,
    ) {
        let mut stack = vec![TextParams {
            size: 1.0,
            scale: 100.0,
            rise: 0.0,
            leading: 0.0,
            tm: Affine::IDENTITY,
            tlm: Affine::IDENTITY,
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
                    if let Some(dict) = resources.get_font(t.0) {
                        self.register(&dict);
                    }
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
                }
                TypedInstruction::ShowText(s) => {
                    let p = *top;
                    self.show(&mut fresh, p, [Object::String(s.0.clone())].into_iter());
                }
                TypedInstruction::ShowTexts(a) => {
                    let p = *top;
                    self.show(&mut fresh, p, a.0.iter::<Object<'_>>());
                }
                TypedInstruction::NextLineAndShowText(s) => {
                    next_line(top, 0.0, -f64::from(top.leading));
                    let p = *top;
                    self.show(&mut fresh, p, [Object::String(s.0.clone())].into_iter());
                }
                TypedInstruction::ShowTextWithParameters(t) => {
                    next_line(top, 0.0, -f64::from(top.leading));
                    let p = *top;
                    self.show(&mut fresh, p, [Object::String(t.2.clone())].into_iter());
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
                    let structure = || {
                        let elem = parents?
                            .iter::<Object<'_>>()
                            .nth(usize::try_from(mcid?).ok()?);
                        elem?.into_dict()
                    };
                    let actual = props
                        .and_then(|p| p.get::<PdfString<'_>>(ACTUAL_TEXT))
                        .or_else(|| structure()?.get::<PdfString<'_>>(ACTUAL_TEXT))
                        .map(|s| text_string(s.as_bytes()));
                    self.marks.push((bdc.0.as_ref().to_vec(), mcid, actual));
                }
                TypedInstruction::XObject(x) if depth < 16 => {
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

/// A PDF text string: UTF-16BE or UTF-8 with a byte order mark, else Latin-1.
fn text_string(bytes: &[u8]) -> Vec<char> {
    if let Some(rest) = bytes.strip_prefix(&[0xfe, 0xff]) {
        let units = rest
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]));
        return char::decode_utf16(units)
            .map(|c| c.unwrap_or('\u{fffd}'))
            .collect();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8_lossy(rest).chars().collect();
    }
    // ponytail: PDFDocEncoding's 0x18-0x1f and 0x80-0x9f differences from Latin-1 are ignored.
    bytes.iter().map(|&b| char::from(b)).collect()
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

/// MuPDF's facts for a font dictionary: FontDescriptor Ascent/Descent win over
/// the font program; style flags come from the program (OS/2, post) when embedded.
fn font_facts(dict: &Dict<'_>) -> TextFont {
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
    let mut facts = match &program {
        Some(stream) => stream
            .decoded()
            .ok()
            .and_then(|data| sfnt_facts(&data))
            // ponytail: bare CFF and Type1 programs are not parsed; FreeType
            // would take extents from the FontBBox and style from FontInfo.
            .unwrap_or_else(|| TextFont {
                italic: flags & (1 << 6) != 0,
                bold: flags & (1 << 18) != 0,
                monospace: flags & 1 != 0,
                ..default_font()
            }),
        None => standard_font(&base, flags),
    };
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
    glyphs: Vec<GlyphEvent>,
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
}

impl ActualSpan {
    /// Replaces the span's glyph text: a shared prefix and suffix keep their own
    /// glyphs, the middle characters take the middle glyphs in order, surplus
    /// glyphs are dropped and surplus characters sit at the pen with no width.
    fn resolve(self) -> Vec<GlyphEvent> {
        let (text, units) = (self.text, self.glyphs);
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

        let mut items: Vec<(Affine, f64, usize)> = glyphs
            .iter()
            .map(|g| {
                let trm = props.transform * g.transform() * em;
                match &**g {
                    Glyph::Outline(o) => (
                        trm,
                        f64::from(o.advance_width().unwrap_or(0.0)) / 1000.0,
                        self.scan.font_index(o.font_cache_key()),
                    ),
                    Glyph::Type3(_) => (trm, f64::NAN, TYPE3_FONT),
                }
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
        let keep: Vec<bool> = if items.iter().all(|i| inside(&em_box(i, 0.5))) {
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
        for ((g, (trm, adv, font)), keep) in glyphs.iter().zip(items).zip(keep) {
            if !keep {
                continue;
            }
            let target = match self.actual.iter_mut().flatten().next() {
                Some(span) => &mut span.glyphs,
                None => &mut self.glyphs,
            };
            for (i, c) in unicode(g.as_unicode()).into_iter().enumerate() {
                target.push(GlyphEvent {
                    c,
                    trm,
                    adv: if i == 0 { adv } else { 0.0 },
                    font,
                    cluster: i > 0,
                    span_start: span_start && i == 0,
                });
            }
            span_start = false;
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
            }));
    }

    fn end_marked_content(&mut self) {
        self.other = true;
        if let Some(Some(span)) = self.actual.pop() {
            self.glyphs.extend(span.resolve());
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
    let text = match mapped {
        Some(BfString::Char(c)) => c.to_string(),
        Some(BfString::String(s)) if !s.is_empty() => s,
        _ => '\u{fffd}'.to_string(),
    };
    let mut out = Vec::new();
    for c in text.chars() {
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
