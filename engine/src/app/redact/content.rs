// SPDX-License-Identifier: MIT
//! The redaction content walk: one bounded pass over a page's /Contents (or
//! an annotation appearance) and every Form they draw, at any depth, that
//! records each draw with the byte span of its operator, the Forms it is
//! drawn through, and its page-space bounds: text with one box per glyph,
//! filled and stroked paths, image XObjects and inline images, shadings and
//! Form XObjects. Redaction maps marks to content with it (`region`): which
//! glyphs, paths, images and Forms each mark touches, and where an object
//! a mark touches is drawn without one.
//!
//! Bounds are conservative: a path's box holds its control points and,
//! stroked, half its line width grown for joins; a glyph's box is its
//! advance across the font's height (never inside the defaults, 0.8 and
//! -0.2 em); a run whose glyphs cannot be placed gets one box that holds
//! every place they could be, and the runs after it on the same line grow
//! by that slack.
//! Bounds ignore clipping (what is clipped away is still in the file); each
//! draw also carries the clip's bounds.
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

use crate::app::annots::{self, appearance, concat, transform_rect};
use crate::app::ops::edit::lex::{Container, Lexer, Operand, Token, is_delimiter, is_white};
use crate::app::pagetree;
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};
use crate::stream::{StreamErrorKind, StreamLimits};
use crate::stream_object::StreamObjectErrorKind;
use crate::view::{Measurer, ViewDocument};

use super::fonts::{self, FontMetrics};

/// What a walk records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detail {
    /// Image and Form XObject draws only (where objects are drawn).
    Objects,
    /// Every draw, text glyph by glyph.
    Full,
    /// `Full`, and paths that only clip or are discarded (`n`), for apply.
    Apply,
}

/// Work one walk may do before it is refused as too complex.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Content tokens, counted again each time a Form is drawn.
    pub tokens: usize,
    /// Decoded bytes of the distinct streams read.
    pub bytes: usize,
    /// Forms drawn inside Forms.
    pub depth: usize,
    /// Draws recorded.
    pub draws: usize,
    /// Glyphs recorded.
    pub glyphs: usize,
}

impl Limits {
    pub const DEFAULT: Self = Self {
        tokens: 8_000_000,
        bytes: 256 << 20,
        depth: 32,
        draws: 500_000,
        glyphs: 1 << 20,
    };
}

/// Content bytes charged as one token each time a stream is run.
const BYTES_PER_TOKEN: usize = 64;
/// Nesting of arrays and dictionaries in one operation.
const MAX_NESTING: usize = 32;
/// Operands of one operation (array items included).
const MAX_OPERANDS: usize = 1 << 20;
/// `q` nesting tracked; deeper saves are counted and restored as no-ops.
const MAX_SAVES: usize = 4096;
/// Open marked-content sequences tracked.
const MAX_MARKED: usize = 4096;
/// Widths (em) below which a glyph's box does not come from its width.
const MIN_GLYPH_WIDTH: f64 = 0.1;
/// How far (em) the box of a glyph without a usable width reaches either
/// way from the pen.
const INK_REACH: f64 = 0.5;
/// Pixels an unfiltered inline image may declare (its size is computed).
const MAX_INLINE_PIXELS: u64 = 1 << 32;

/// Why a walk stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WalkError {
    /// A budget of `Limits` ran out.
    TooComplex(String),
    /// A stream cannot be decoded or tokenized.
    Unreadable(String),
}

impl std::fmt::Display for WalkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooComplex(what) | Self::Unreadable(what) => f.write_str(what),
        }
    }
}

/// One glyph of a show operation.
#[derive(Clone, Debug, PartialEq)]
pub struct Glyph {
    /// The string it is in: the operand index of `Tj`, `'` and `"`, the
    /// array element index of `TJ`.
    pub operand: usize,
    /// Its code's bytes within that string's decoded bytes.
    pub bytes: Range<usize>,
    pub code: u32,
    /// The pen's move along the writing direction in unscaled text space
    /// (font size, character and word spacing; horizontal scaling not
    /// applied): what a `TJ` adjustment of `-advance * 1000 / size` replaces.
    pub advance: f64,
    /// Page space.
    pub bounds: [f64; 4],
    /// The box's corners in page space, in order around it (single
    /// precision): `bounds` holds them, and is larger when it is turned.
    pub quad: [f32; 8],
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextDraw {
    /// The font dictionary's object (None for a direct one or none at all).
    pub font: Option<ObjectId>,
    pub size: f64,
    /// Horizontal scaling (Tz / 100).
    pub scale: f64,
    pub render_mode: i64,
    pub vertical: bool,
    /// The operator is `TJ` (glyph operands index its array).
    pub array: bool,
    /// Every glyph was placed from the font's advances. When false the
    /// glyphs list is empty and `bounds` holds wherever they could be.
    pub measured: bool,
    /// How far (text space, along the line) an earlier run that could not
    /// be measured may have moved the pen; glyph boxes are grown by it.
    pub slack: f64,
    pub glyphs: Vec<Glyph>,
    /// The name `Tf` set the font with (None when an ExtGState set it).
    pub font_name: Option<Vec<u8>>,
    /// The font is a Type 3 font.
    pub type3: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DrawKind {
    Text(TextDraw),
    /// A painted path; `start` of the draw is the path's first operator.
    Path {
        fill: bool,
        stroke: bool,
    },
    /// An image XObject (`object`) or an inline image (`BI` to `EI`).
    Image {
        object: Option<ObjectId>,
        /// The image's unit square to page space.
        matrix: [f64; 6],
    },
    Shading,
    /// A Form XObject; its content's draws name this draw as `parent`.
    Form {
        object: ObjectId,
        /// Form space to page space.
        matrix: [f64; 6],
        bbox: Option<[f64; 4]>,
        /// False when the walk did not enter it (it is already being drawn).
        entered: bool,
    },
}

impl DrawKind {
    /// The XObject it draws, with "image" or "form".
    pub fn xobject(&self) -> Option<(ObjectId, &'static str)> {
        match self {
            Self::Image {
                object: Some(id), ..
            } => Some((*id, "image")),
            Self::Form { object, .. } => Some((*object, "form")),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Draw {
    pub kind: DrawKind,
    /// The stream holding the operator.
    pub stream: ObjectId,
    /// The operator's span in that stream's decoded bytes.
    pub op: Range<usize>,
    /// Where the operation starts: its first operand (a path's first
    /// construction operator, an inline image's `BI`).
    pub start: usize,
    /// The Form draw whose content this is.
    pub parent: Option<usize>,
    /// Page space, unclipped.
    pub bounds: [f64; 4],
    /// The clip's page-space bounds; None when nothing clips.
    pub clip: Option<[f64; 4]>,
    /// The innermost marked-content sequence's /MCID.
    pub mcid: Option<i64>,
    /// Inside a marked-content sequence whose property list has
    /// /ActualText, /Alt or /E.
    pub alt: bool,
    /// The CTM the operation draws with (user space to page space).
    pub ctm: [f64; 6],
    /// The page /Contents element holding the operator (0 in a Form).
    pub element: usize,
    /// Where the operation began when its operands began in an earlier
    /// /Contents element: (element, offset).
    pub from: Option<(usize, usize)>,
    /// The innermost open sequence of `Content::sequences` with alternate
    /// text in this stream's frame (`Detail::Apply`).
    pub sequence: Option<usize>,
    /// A path that also sets the clip (`W`, `W*`).
    pub clips: bool,
    /// What it paints with besides its own data.
    pub paint: Paint,
}

/// A pattern a fill or stroke paints with (`scn`/`SCN` with a name).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// A pattern object (tiling or shading).
    Object(ObjectId),
    /// A shading pattern written directly into /Pattern, with its
    /// /ShadingType (0 when unknown).
    Shading(i64),
    /// The name names nothing: nothing is painted.
    Missing,
}

/// What a draw paints with besides its own data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Paint {
    /// The pattern its fill paints with (a stencil mask's too).
    pub fill: Option<Pattern>,
    /// The pattern its stroke paints with.
    pub stroke: Option<Pattern>,
    /// The soft-mask group (ExtGState /SMask /G) in force.
    pub soft_mask: Option<ObjectId>,
    /// An image draw is a stencil mask (`/ImageMask true`).
    pub stencil: bool,
    /// A shading draw's /ShadingType (0 when unknown).
    pub shading: i64,
}

/// Where a marked-content property list's resources live.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Owner {
    /// The page's /Resources (its own or inherited).
    Page,
    /// A Form's /Resources.
    Form(ObjectId),
}

/// A `BDC` whose property list has /ActualText, /Alt or /E.
#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    pub stream: ObjectId,
    pub element: usize,
    /// The whole operation: tag, property list and operator.
    pub span: Range<usize>,
    /// The operation began in an earlier /Contents element (split).
    pub from: Option<(usize, usize)>,
    /// The Form draw whose content holds it; None in the page's /Contents.
    pub form: Option<usize>,
    /// The enclosing such sequence in the same frame.
    pub parent: Option<usize>,
    /// A named property list (in /Properties of `owner`'s resources);
    /// None for one written inline.
    pub name: Option<Vec<u8>>,
    pub owner: Owner,
}

/// What one walk found.
#[derive(Clone, Debug, Default)]
pub struct Content {
    pub draws: Vec<Draw>,
    /// The decoded bytes the spans index, by stream (`Detail::Full` only).
    pub streams: BTreeMap<ObjectId, Arc<Vec<u8>>>,
    /// Every stream read, in first-read order.
    pub read: Vec<ObjectId>,
    /// An operation's operands began in an earlier /Contents stream.
    pub split: bool,
    /// Content tokens the walk read.
    pub tokens: usize,
    /// Occurrences of `/ActualText`, `/Alt` and `/E` in the streams read
    /// (redact.ts `inlineAltCount`'s pattern), each stream counted once.
    pub inline_alt: usize,
    /// The page's /Contents streams in order (a page walk).
    pub elements: Vec<ObjectId>,
    /// Marked-content sequences with alternate text (`Detail::Apply`).
    pub sequences: Vec<Sequence>,
    /// `Q` operators of the page's /Contents with nothing to restore.
    pub excess: Vec<(usize, Range<usize>)>,
    /// What the page's /Contents leave open at their end: `q` saves, a
    /// text object, marked-content sequences.
    pub open_saves: usize,
    pub open_text: bool,
    pub open_marked: usize,
}

/// A parsed operand with its span.
#[derive(Clone, Debug)]
enum Arg {
    Num(f64),
    Name(Vec<u8>),
    Str(Vec<u8>),
    Arr(Vec<Arg>),
    Dict(Vec<(Vec<u8>, Arg)>),
    Bool(bool),
    Null,
}

impl Arg {
    fn num(&self) -> Option<f64> {
        match self {
            Self::Num(n) if n.is_finite() => Some(*n),
            _ => None,
        }
    }
    fn name(&self) -> Option<&[u8]> {
        match self {
            Self::Name(n) => Some(n),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct Gs {
    ctm: [f64; 6],
    clip: Option<[f64; 4]>,
    line_width: f64,
    miter: f64,
    join: i64,
    font: Option<Arc<FontMetrics>>,
    font_name: Option<Vec<u8>>,
    size: f64,
    char_spacing: f64,
    word_spacing: f64,
    scale: f64,
    leading: f64,
    rise: f64,
    mode: i64,
    fill_pattern: Option<Pattern>,
    stroke_pattern: Option<Pattern>,
    soft_mask: Option<ObjectId>,
}

impl Gs {
    fn new(ctm: [f64; 6], clip: Option<[f64; 4]>) -> Self {
        Self {
            ctm,
            clip,
            line_width: 1.0,
            miter: 10.0,
            join: 0,
            font: None,
            font_name: None,
            size: 0.0,
            char_spacing: 0.0,
            word_spacing: 0.0,
            scale: 1.0,
            leading: 0.0,
            rise: 0.0,
            mode: 0,
            fill_pattern: None,
            stroke_pattern: None,
            soft_mask: None,
        }
    }
}

/// The text object: the text and line matrices and the pen's uncertainty.
#[derive(Clone, Debug)]
struct TextPos {
    tm: [f64; 6],
    tlm: [f64; 6],
    slack: f64,
}

impl TextPos {
    fn new() -> Self {
        Self {
            tm: appearance::IDENTITY,
            tlm: appearance::IDENTITY,
            slack: 0.0,
        }
    }
}

/// Operands read but not yet used by an operator.
#[derive(Default)]
struct Pending {
    args: Vec<Arg>,
    stack: Vec<(Container, Vec<Arg>)>,
    count: usize,
    /// Where the inline image being read began: stream, element, offset.
    inline: Option<(ObjectId, usize, usize)>,
    /// Where the pending operands began: (element, offset).
    origin: Option<(usize, usize)>,
}

/// One sequence of streams drawn with the same resources: the page's
/// /Contents, or a Form.
struct Frame<'d> {
    resources: Option<&'d Dictionary>,
    parent: Option<usize>,
    gs: Gs,
    saved: Vec<Gs>,
    extra_saves: usize,
    text: TextPos,
    marked: Vec<(bool, Option<i64>)>,
    /// Per open sequence, its index in `Content::sequences` when it has
    /// alternate text (`Detail::Apply`).
    sequences: Vec<Option<usize>>,
    extra_marked: usize,
    path: Option<[f64; 4]>,
    /// Where the path being built began: (element, offset).
    path_start: Option<(usize, usize)>,
    clip_pending: bool,
    owner: Owner,
    /// The page's /Contents element being run (0 in a Form).
    element: usize,
    /// The page's /Contents: `q` depth counted, `Q` past it recorded.
    page: bool,
    depth: usize,
    in_text: bool,
}

impl<'d> Frame<'d> {
    fn new(resources: Option<&'d Dictionary>, parent: Option<usize>, gs: Gs) -> Self {
        Self {
            resources,
            parent,
            gs,
            saved: Vec::new(),
            extra_saves: 0,
            text: TextPos::new(),
            marked: Vec::new(),
            sequences: Vec::new(),
            extra_marked: 0,
            path: None,
            path_start: None,
            clip_pending: false,
            owner: Owner::Page,
            element: 0,
            page: false,
            depth: 0,
            in_text: false,
        }
    }

    fn sequence(&self) -> Option<usize> {
        self.sequences.iter().rev().find_map(|s| *s)
    }

    fn mcid(&self) -> Option<i64> {
        self.marked.iter().rev().find_map(|(_, mcid)| *mcid)
    }

    fn alt(&self) -> bool {
        self.marked.iter().any(|(alt, _)| *alt)
    }

    fn add_point(&mut self, x: f64, y: f64) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        let (px, py) = annots::apply(&self.gs.ctm, (x, y));
        if !px.is_finite() || !py.is_finite() {
            return;
        }
        self.path = Some(match self.path {
            None => [px, py, px, py],
            Some([x0, y0, x1, y1]) => [x0.min(px), y0.min(py), x1.max(px), y1.max(py)],
        });
    }
}

fn intersect(a: Option<[f64; 4]>, b: [f64; 4]) -> [f64; 4] {
    match a {
        None => b,
        Some(a) => {
            let r = [
                a[0].max(b[0]),
                a[1].max(b[1]),
                a[2].min(b[2]),
                a[3].min(b[3]),
            ];
            // An empty intersection keeps an empty (inverted-free) box.
            [r[0], r[1], r[2].max(r[0]), r[3].max(r[1])]
        }
    }
}

/// How much `m` stretches a length at most (its largest singular value).
fn stretch(m: &[f64; 6]) -> f64 {
    let s = m[0] * m[0] + m[1] * m[1] + m[2] * m[2] + m[3] * m[3];
    let det = m[0] * m[3] - m[1] * m[2];
    ((s + (s * s - 4.0 * det * det).max(0.0).sqrt()) / 2.0).sqrt()
}

/// redact.ts `inlineAltCount`'s `/\/(?:ActualText|Alt|E)\b/g` over bytes.
pub fn count_alt(bytes: &[u8]) -> usize {
    let word = |b: Option<&u8>| b.is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_');
    let mut count = 0;
    let mut at = 0;
    while let Some(offset) = bytes
        .get(at..)
        .and_then(|rest| rest.iter().position(|&b| b == b'/'))
    {
        let slash = at + offset;
        let rest = bytes.get(slash + 1..).unwrap_or(&[]);
        let matched = [b"ActualText".as_slice(), b"Alt", b"E"]
            .into_iter()
            .find(|name| rest.starts_with(name) && !word(rest.get(name.len())));
        match matched {
            Some(name) => {
                count += 1;
                at = slash + 1 + name.len();
            }
            None => at = slash + 1,
        }
    }
    count
}

type TableKey = (Option<ObjectId>, Vec<Vec<u8>>, Vec<u8>);

/// A font dictionary's identity: its object, else (a direct dictionary,
/// which lives as long as the document) its address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum FontKey {
    Object(ObjectId),
    Direct(usize),
}

/// Walks content and collects its draws.
pub struct Walker<'d, 'v> {
    doc: &'d Document<'static>,
    view: Option<&'v ViewDocument>,
    page: usize,
    detail: Detail,
    limits: Limits,
    tokens: usize,
    bytes: usize,
    glyphs: usize,
    /// Fonts read, by object or (a direct dictionary) by its address.
    fonts: BTreeMap<FontKey, Arc<FontMetrics>>,
    /// CMaps and /W tables several fonts share.
    font_parts: fonts::FontCache,
    /// The `EI` search of each stream with inline images of unknown length.
    ei_scans: BTreeMap<ObjectId, EiScan>,
    measurer: Option<Option<Measurer<'v>>>,
    /// Advance tables of fonts the view measured, by font object (or, for
    /// a direct font dictionary, by resource path and name).
    tables: BTreeMap<TableKey, Option<Arc<Vec<Option<f64>>>>>,
    /// The XObject names lead from the page's resources (not inside an
    /// annotation appearance), so the view can measure fonts.
    measurable: bool,
    active: Vec<ObjectId>,
    /// XObject names from the page's resources to the current Form.
    names: Vec<Vec<u8>>,
    decoded: BTreeMap<ObjectId, Arc<Vec<u8>>>,
    /// The bounds a shading fills without a clip.
    area: [f64; 4],
    /// The current operation began in an earlier /Contents element.
    from: Option<(usize, usize)>,
    out: Content,
}

impl<'d, 'v> Walker<'d, 'v> {
    /// A walk of page `page` (for the view's measurer and decoder) that
    /// fills `area` (page space) when nothing clips.
    pub fn new(
        doc: &'d Document<'static>,
        view: Option<&'v ViewDocument>,
        page: usize,
        detail: Detail,
        limits: Limits,
        area: [f64; 4],
    ) -> Self {
        Self {
            doc,
            view,
            page,
            detail,
            limits,
            tokens: 0,
            bytes: 0,
            glyphs: 0,
            fonts: BTreeMap::new(),
            font_parts: fonts::FontCache::default(),
            ei_scans: BTreeMap::new(),
            measurer: None,
            tables: BTreeMap::new(),
            measurable: true,
            active: Vec::new(),
            names: Vec::new(),
            decoded: BTreeMap::new(),
            area,
            from: None,
            out: Content::default(),
        }
    }

    pub fn finish(mut self) -> Content {
        self.out.tokens = self.tokens;
        self.out
    }

    /// The page's /Contents, drawn with `ctm` (user space to page space).
    pub fn page(&mut self, page: &'d Dictionary, ctm: [f64; 6]) -> Result<(), WalkError> {
        let doc = self.doc;
        let resources =
            pagetree::inherited(doc, page, b"Resources").and_then(|r| doc.resolve(r).as_dict());
        let streams = content_streams(doc, page);
        let mut frame = Frame::new(resources, None, Gs::new(ctm, None));
        frame.page = true;
        let mut pending = Pending::default();
        self.out.elements = streams.clone();
        for (element, id) in streams.into_iter().enumerate() {
            let bytes = self.read(id)?;
            frame.element = element;
            self.run(&mut frame, id, &bytes, &mut pending)?;
        }
        self.out.open_saves = frame.depth;
        self.out.open_text = frame.in_text;
        self.out.open_marked = frame.marked.len() + frame.extra_marked;
        Ok(())
    }

    /// A Form drawn on its own (an annotation appearance): `matrix` maps
    /// form space to page space.
    pub fn form(
        &mut self,
        id: ObjectId,
        matrix: [f64; 6],
        resources: Option<&'d Dictionary>,
    ) -> Result<(), WalkError> {
        let doc = self.doc;
        let dict = doc.stream_dict(id).ok();
        let own = dict
            .and_then(|d| d.get(b"Resources".as_slice()))
            .and_then(|r| doc.resolve(r).as_dict());
        let bytes = self.read(id)?;
        let mut frame = Frame::new(own.or(resources), None, Gs::new(matrix, None));
        frame.owner = Owner::Form(id);
        self.active.push(id);
        let measurable = std::mem::replace(&mut self.measurable, false);
        let result = self.run(&mut frame, id, &bytes, &mut Pending::default());
        self.measurable = measurable;
        self.active.pop();
        result
    }

    fn too_complex(what: &str) -> WalkError {
        WalkError::TooComplex(format!("content is too complex ({what})"))
    }

    /// A stream's decoded bytes, strictly or as the view reads them.
    fn read(&mut self, id: ObjectId) -> Result<Arc<Vec<u8>>, WalkError> {
        if let Some(bytes) = self.decoded.get(&id) {
            return Ok(bytes.clone());
        }
        let room = self.limits.bytes.saturating_sub(self.bytes);
        let limits = StreamLimits {
            max_decoded_bytes: room,
            max_total_decoded_bytes: room,
            ..StreamLimits::default()
        };
        let bytes = match self.doc.decoded_stream(id, limits) {
            Ok(bytes) => bytes,
            Err(error) if over_limit(&error.kind) => {
                return Err(Self::too_complex("decoded bytes"));
            }
            // Damage the strict decoder refuses is read as the view reads it.
            Err(_) => (self.view)
                .and_then(|view| view.decoded_stream(id.number(), id.generation()))
                .ok_or_else(|| {
                    WalkError::Unreadable(format!("stream {} cannot be decoded", id.number()))
                })?,
        };
        self.bytes = self.bytes.saturating_add(bytes.len());
        if self.bytes > self.limits.bytes {
            return Err(Self::too_complex("decoded bytes"));
        }
        self.out.inline_alt += count_alt(&bytes);
        self.out.read.push(id);
        let bytes = Arc::new(bytes);
        if self.detail != Detail::Objects {
            self.out.streams.insert(id, bytes.clone());
        }
        self.decoded.insert(id, bytes.clone());
        Ok(bytes)
    }

    fn push(&mut self, draw: Draw) -> Result<usize, WalkError> {
        if self.out.draws.len() >= self.limits.draws {
            return Err(Self::too_complex("draws"));
        }
        self.out.draws.push(draw);
        Ok(self.out.draws.len() - 1)
    }

    fn next(&mut self, lexer: &mut Lexer<'_>) -> Result<Option<Token>, WalkError> {
        self.tokens += 1;
        if self.tokens > self.limits.tokens {
            return Err(Self::too_complex("tokens"));
        }
        // The edit lexer's own page budget is not ours: count afresh.
        let mut scratch = 0;
        lexer
            .next(&mut scratch)
            .map_err(|error| WalkError::Unreadable(error.message))
    }

    /// Runs one stream. `pending` holds the operands an earlier /Contents
    /// stream left for an operator in this one (the streams read as one);
    /// they keep their values, and the operation's start is its
    /// operator.
    fn run(
        &mut self,
        frame: &mut Frame<'d>,
        stream: ObjectId,
        bytes: &[u8],
        pending: &mut Pending,
    ) -> Result<(), WalkError> {
        // Lexing reads every byte whatever the tokens (a Form drawn again is
        // read again): a token per `BYTES_PER_TOKEN`.
        self.tokens = self.tokens.saturating_add(bytes.len() / BYTES_PER_TOKEN);
        if self.tokens > self.limits.tokens {
            return Err(Self::too_complex("tokens"));
        }
        let mut lexer = Lexer::new(bytes);
        // Operands already pending began in another stream.
        let mut first: Option<usize> = None;
        let mut here = pending.args.is_empty() && pending.stack.is_empty();
        while let Some(token) = self.next(&mut lexer)? {
            match token {
                Token::Operand(operand) => {
                    pending.count += 1;
                    if pending.count > MAX_OPERANDS {
                        return Err(Self::too_complex("operands"));
                    }
                    let start = operand.span().start;
                    let arg = match operand {
                        Operand::Num(n, _) => Arg::Num(n),
                        Operand::Name(n, _) => Arg::Name(n),
                        Operand::Str(s, _) => Arg::Str(s),
                        Operand::Word(span) => match bytes.get(span) {
                            Some(b"true") => Arg::Bool(true),
                            Some(b"false") => Arg::Bool(false),
                            _ => Arg::Null,
                        },
                        Operand::Arr(..) | Operand::Dict(_) => Arg::Null,
                    };
                    match pending.stack.last_mut() {
                        Some((_, items)) => items.push(arg),
                        None => {
                            first.get_or_insert(start);
                            pending.origin.get_or_insert((frame.element, start));
                            pending.args.push(arg);
                        }
                    }
                }
                Token::Open(kind, span) => {
                    if pending.stack.len() >= MAX_NESTING {
                        return Err(Self::too_complex("nesting"));
                    }
                    if pending.stack.is_empty() {
                        first.get_or_insert(span.start);
                        pending.origin.get_or_insert((frame.element, span.start));
                    }
                    pending.stack.push((kind, Vec::new()));
                }
                Token::Close(kind, _) => {
                    let Some((open, items)) = pending.stack.pop() else {
                        continue;
                    };
                    if open != kind {
                        // A mismatched close drops the operand being read;
                        // parsing resumes after the token.
                        pending.stack.clear();
                        continue;
                    }
                    let arg = match kind {
                        Container::Array => Arg::Arr(items),
                        Container::Dict => Arg::Dict(pairs(items)),
                    };
                    match pending.stack.last_mut() {
                        Some((_, outer)) => outer.push(arg),
                        None => pending.args.push(arg),
                    }
                }
                Token::Op(span) => {
                    if !pending.stack.is_empty() {
                        return Err(WalkError::Unreadable(
                            "an operator inside an array or dictionary".into(),
                        ));
                    }
                    let op = bytes.get(span.clone()).unwrap_or(&[]);
                    if !here {
                        self.out.split = true;
                    }
                    let start = match (here, first) {
                        (true, Some(first)) => first,
                        _ => span.start,
                    };
                    self.from = if here { None } else { pending.origin };
                    if op == b"BI" {
                        pending.inline = Some((stream, frame.element, span.start));
                    } else if op == b"ID" {
                        let (start, from) = match pending.inline.take() {
                            Some((s, e, at)) if s == stream && e == frame.element => (at, None),
                            Some((_, e, at)) => (span.start, Some((e, at))),
                            None => (span.start, None),
                        };
                        let data = span.end + usize::from(is_white(bytes.get(span.end).copied()));
                        let end = inline_end(bytes, data, &pending.args)
                            .or_else(|| self.ei_scans.entry(stream).or_default().find(bytes, data))
                            .ok_or_else(|| {
                                WalkError::Unreadable("an inline image has no end".into())
                            })?;
                        lexer.pos = end;
                        self.from = from;
                        let stencil = pending.args.chunks(2).any(|pair| {
                            matches!(pair, [Arg::Name(k), Arg::Bool(true)] if k == b"IM" || k == b"ImageMask")
                        });
                        self.inline_image(frame, stream, start..end, stencil)?;
                    } else {
                        let args = std::mem::take(&mut pending.args);
                        self.operation(frame, stream, op, &args, start..span.end, span.clone())?;
                    }
                    self.from = None;
                    pending.args.clear();
                    pending.count = 0;
                    pending.origin = None;
                    first = None;
                    here = true;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn operation(
        &mut self,
        frame: &mut Frame<'d>,
        stream: ObjectId,
        op: &[u8],
        args: &[Arg],
        whole: Range<usize>,
        span: Range<usize>,
    ) -> Result<(), WalkError> {
        let n = |i: usize| args.get(i).and_then(Arg::num);
        let full = self.detail != Detail::Objects;
        match op {
            b"q" => {
                if frame.saved.len() < MAX_SAVES {
                    frame.saved.push(frame.gs.clone());
                } else {
                    frame.extra_saves += 1;
                }
                frame.depth = frame.depth.saturating_add(1);
            }
            b"Q" => {
                if frame.extra_saves > 0 {
                    frame.extra_saves -= 1;
                } else if let Some(gs) = frame.saved.pop() {
                    frame.gs = gs;
                }
                match frame.depth.checked_sub(1) {
                    Some(depth) => frame.depth = depth,
                    None if frame.page => self.out.excess.push((frame.element, whole.clone())),
                    None => {}
                }
            }
            b"cm" => {
                if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) =
                    (n(0), n(1), n(2), n(3), n(4), n(5))
                {
                    frame.gs.ctm = concat(&[a, b, c, d, e, f], &frame.gs.ctm);
                }
            }
            b"w" => {
                if let Some(w) = n(0) {
                    frame.gs.line_width = w;
                }
            }
            b"j" => {
                if let Some(j) = n(0) {
                    frame.gs.join = j as i64;
                }
            }
            b"M" => {
                if let Some(m) = n(0) {
                    frame.gs.miter = m;
                }
            }
            b"gs" => self.ext_gstate(frame, args.first().and_then(Arg::name)),
            b"cs" | b"g" | b"rg" | b"k" | b"sc" => frame.gs.fill_pattern = None,
            b"CS" | b"G" | b"RG" | b"K" | b"SC" => frame.gs.stroke_pattern = None,
            b"scn" | b"SCN" => {
                let pattern = match args.last() {
                    Some(Arg::Name(name)) => Some(self.pattern(frame.resources, name)),
                    _ => None,
                };
                match op {
                    b"scn" => frame.gs.fill_pattern = pattern,
                    _ => frame.gs.stroke_pattern = pattern,
                }
            }
            b"m" | b"l" => {
                if full {
                    self.path_start(frame, whole.start);
                    if let (Some(x), Some(y)) = (n(0), n(1)) {
                        frame.add_point(x, y);
                    }
                }
            }
            b"c" | b"v" | b"y" => {
                if full {
                    self.path_start(frame, whole.start);
                    for pair in args.chunks(2) {
                        if let (Some(x), Some(y)) = (
                            pair.first().and_then(Arg::num),
                            pair.get(1).and_then(Arg::num),
                        ) {
                            frame.add_point(x, y);
                        }
                    }
                }
            }
            b"re" => {
                if full {
                    self.path_start(frame, whole.start);
                    if let (Some(x), Some(y), Some(w), Some(h)) = (n(0), n(1), n(2), n(3)) {
                        frame.add_point(x, y);
                        frame.add_point(x + w, y);
                        frame.add_point(x + w, y + h);
                        frame.add_point(x, y + h);
                    }
                }
            }
            b"h" => {}
            b"W" | b"W*" => frame.clip_pending = true,
            b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"n" => {
                self.paint(frame, stream, op, span)?;
            }
            b"BT" => {
                frame.text = TextPos::new();
                frame.in_text = true;
            }
            b"ET" => frame.in_text = false,
            b"Tf" => {
                if let Some(size) = n(1) {
                    frame.gs.size = size;
                }
                let name = args.first().and_then(Arg::name);
                frame.gs.font_name = name.map(<[u8]>::to_vec);
                frame.gs.font = if full {
                    self.font(frame.resources, name)
                } else {
                    None
                };
            }
            b"Tc" => frame.gs.char_spacing = n(0).unwrap_or(frame.gs.char_spacing),
            b"Tw" => frame.gs.word_spacing = n(0).unwrap_or(frame.gs.word_spacing),
            b"Tz" => frame.gs.scale = n(0).map_or(frame.gs.scale, |z| z / 100.0),
            b"TL" => frame.gs.leading = n(0).unwrap_or(frame.gs.leading),
            b"Ts" => frame.gs.rise = n(0).unwrap_or(frame.gs.rise),
            b"Tr" => frame.gs.mode = n(0).map_or(frame.gs.mode, |m| m as i64),
            b"Td" | b"TD" => {
                if let (Some(x), Some(y)) = (n(0), n(1)) {
                    if op == b"TD" {
                        frame.gs.leading = -y;
                    }
                    move_line(&mut frame.text, x, y);
                }
            }
            b"Tm" => {
                if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) =
                    (n(0), n(1), n(2), n(3), n(4), n(5))
                {
                    frame.text = TextPos {
                        tm: [a, b, c, d, e, f],
                        tlm: [a, b, c, d, e, f],
                        slack: 0.0,
                    };
                }
            }
            b"T*" => move_line(&mut frame.text, 0.0, -frame.gs.leading),
            b"Tj" | b"TJ" | b"'" | b"\"" => {
                if op == b"\"" {
                    if let (Some(aw), Some(ac)) = (n(0), n(1)) {
                        frame.gs.word_spacing = aw;
                        frame.gs.char_spacing = ac;
                    }
                }
                if op == b"'" || op == b"\"" {
                    move_line(&mut frame.text, 0.0, -frame.gs.leading);
                }
                if full {
                    self.show(frame, stream, op, args, whole, span)?;
                }
            }
            b"Do" => self.xobject(frame, stream, args.first().and_then(Arg::name), whole, span)?,
            b"sh" => {
                if full {
                    let bounds = frame.gs.clip.unwrap_or(self.area);
                    let kind = self.shading_type(frame.resources, args.first().and_then(Arg::name));
                    let index = self.draw(frame, DrawKind::Shading, stream, whole, span, bounds)?;
                    if let Some(draw) = self.out.draws.get_mut(index) {
                        draw.paint.shading = kind;
                    }
                }
            }
            b"BMC" => self.marked(frame, false, None, None),
            b"BDC" => {
                let (alt, mcid) = self.properties(frame.resources, args.get(1));
                if alt && self.out.sequences.len() >= self.limits.draws {
                    return Err(Self::too_complex("marked content"));
                }
                let sequence = (alt && self.detail == Detail::Apply).then(|| Sequence {
                    stream,
                    element: frame.element,
                    span: whole.clone(),
                    from: self.from,
                    form: frame.parent,
                    parent: frame.sequence(),
                    name: args.get(1).and_then(Arg::name).map(<[u8]>::to_vec),
                    owner: frame.owner,
                });
                self.marked(frame, alt, mcid, sequence);
            }
            b"EMC" => {
                if frame.extra_marked > 0 {
                    frame.extra_marked -= 1;
                } else {
                    frame.marked.pop();
                    frame.sequences.pop();
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn marked(
        &mut self,
        frame: &mut Frame<'d>,
        alt: bool,
        mcid: Option<i64>,
        sequence: Option<Sequence>,
    ) {
        if frame.marked.len() < MAX_MARKED {
            frame.marked.push((alt, mcid));
            let index = sequence.map(|sequence| {
                self.out.sequences.push(sequence);
                self.out.sequences.len() - 1
            });
            frame.sequences.push(index);
        } else {
            frame.extra_marked += 1;
        }
    }

    /// A BDC property list: inline, or named in /Properties.
    fn properties(
        &self,
        resources: Option<&Dictionary>,
        props: Option<&Arg>,
    ) -> (bool, Option<i64>) {
        const ALT: [&[u8]; 3] = [b"ActualText", b"Alt", b"E"];
        match props {
            Some(Arg::Dict(entries)) => {
                let alt = entries
                    .iter()
                    .any(|(k, v)| ALT.contains(&k.as_slice()) && !matches!(v, Arg::Null));
                let mcid = entries
                    .iter()
                    .find(|(k, _)| k == b"MCID")
                    .and_then(|(_, v)| v.num())
                    .filter(|v| v.fract() == 0.0)
                    .map(|v| v as i64);
                (alt, mcid)
            }
            Some(Arg::Name(name)) => {
                let doc = self.doc;
                let dict = resources
                    .and_then(|r| doc.dict_get(r, b"Properties").as_dict())
                    .and_then(|p| doc.dict_get(p, name).as_dict());
                let Some(dict) = dict else {
                    return (false, None);
                };
                let alt = ALT.iter().any(|k| !doc.dict_get(dict, k).is_null());
                let mcid = doc.dict_get(dict, b"MCID").as_i64();
                (alt, mcid)
            }
            _ => (false, None),
        }
    }

    fn ext_gstate(&mut self, frame: &mut Frame<'d>, name: Option<&[u8]>) {
        let doc = self.doc;
        let Some(gs) = frame
            .resources
            .and_then(|r| doc.dict_get(r, b"ExtGState").as_dict())
            .and_then(|g| doc.dict_get(g, name?).as_dict())
        else {
            return;
        };
        if let Some(w) = doc.dict_get(gs, b"LW").as_f64() {
            frame.gs.line_width = w;
        }
        if let Some(m) = doc.dict_get(gs, b"ML").as_f64() {
            frame.gs.miter = m;
        }
        if let Some(j) = doc.dict_get(gs, b"LJ").as_i64() {
            frame.gs.join = j;
        }
        match doc.dict_get(gs, b"SMask") {
            Object::Null => {}
            Object::Dictionary(mask) => {
                frame.gs.soft_mask = mask.get(b"G".as_slice()).and_then(Object::as_reference);
            }
            _ => frame.gs.soft_mask = None,
        }
        if let Some(font) = doc.dict_get(gs, b"Font").as_array() {
            let (holder, value) = match font.first() {
                Some(f) => doc.resolve_with_holder(f),
                None => return,
            };
            if let Some(size) = font.get(1).and_then(|s| doc.resolve(s).as_f64()) {
                frame.gs.size = size;
            }
            frame.gs.font_name = None;
            frame.gs.font = match value.as_dict() {
                Some(dict) if self.detail != Detail::Objects => Some(self.metrics(dict, holder)),
                _ => None,
            };
        }
    }

    fn path_start(&mut self, frame: &mut Frame<'d>, at: usize) {
        if frame.path_start.is_none() {
            frame.path_start = Some(self.from.unwrap_or((frame.element, at)));
        }
    }

    fn paint(
        &mut self,
        frame: &mut Frame<'d>,
        stream: ObjectId,
        op: &[u8],
        span: Range<usize>,
    ) -> Result<(), WalkError> {
        let path = frame.path.take();
        let start = frame.path_start.take();
        let clips = frame.clip_pending;
        if frame.clip_pending {
            frame.clip_pending = false;
            if let Some(path) = path {
                frame.gs.clip = Some(intersect(frame.gs.clip, path));
            }
        }
        let (fill, stroke) = match op {
            b"S" | b"s" => (false, true),
            b"f" | b"F" | b"f*" => (true, false),
            // A path that only clips or is discarded draws nothing, but its
            // geometry is in the stream: apply records it.
            b"n" if self.detail == Detail::Apply => (false, false),
            b"n" => return Ok(()),
            _ => (true, true),
        };
        let Some(mut bounds) = path else {
            return Ok(());
        };
        if stroke {
            let half = frame.gs.line_width.abs() / 2.0;
            let join = if frame.gs.join == 0 {
                frame.gs.miter.clamp(1.0, 10.0)
            } else {
                std::f64::consts::SQRT_2
            };
            // A hairline (width 0) still paints a device pixel.
            let grow = (half * join * stretch(&frame.gs.ctm)).max(0.5);
            bounds = appearance::grow(bounds, grow);
        }
        let (begin, from) = match start {
            Some((element, at)) if element == frame.element => (at, None),
            Some(earlier) => (span.start, Some(earlier)),
            None => (span.start, self.from),
        };
        self.from = from;
        self.draw(
            frame,
            DrawKind::Path { fill, stroke },
            stream,
            begin..span.end,
            span,
            bounds,
        )?;
        if let Some(draw) = self.out.draws.last_mut() {
            draw.clips = clips;
        }
        Ok(())
    }

    fn draw(
        &mut self,
        frame: &Frame<'d>,
        kind: DrawKind,
        stream: ObjectId,
        whole: Range<usize>,
        span: Range<usize>,
        bounds: [f64; 4],
    ) -> Result<usize, WalkError> {
        let gs = &frame.gs;
        let (fill, stroke) = match &kind {
            DrawKind::Path { fill, stroke } => (*fill, *stroke),
            DrawKind::Text(text) => (
                matches!(text.render_mode, 0 | 2 | 4 | 6),
                matches!(text.render_mode, 1 | 2 | 5 | 6),
            ),
            _ => (false, false),
        };
        let paint = Paint {
            fill: gs.fill_pattern.filter(|_| fill),
            stroke: gs.stroke_pattern.filter(|_| stroke),
            soft_mask: gs.soft_mask,
            stencil: false,
            shading: 0,
        };
        self.push(Draw {
            kind,
            stream,
            op: span,
            start: whole.start,
            parent: frame.parent,
            bounds,
            clip: frame.gs.clip,
            mcid: frame.mcid(),
            alt: frame.alt(),
            ctm: frame.gs.ctm,
            element: frame.element,
            from: self.from,
            sequence: frame.sequence(),
            clips: false,
            paint,
        })
    }

    /// A stencil mask paints with the fill.
    fn stencil(&mut self, frame: &Frame<'d>, index: usize) {
        if let Some(draw) = self.out.draws.get_mut(index) {
            draw.paint.stencil = true;
            draw.paint.fill = frame.gs.fill_pattern;
        }
    }

    /// The pattern `name` names in `resources`.
    fn pattern(&self, resources: Option<&'d Dictionary>, name: &[u8]) -> Pattern {
        let doc = self.doc;
        let entry = resources
            .and_then(|r| doc.dict_get(r, b"Pattern").as_dict())
            .and_then(|p| p.get(name));
        let Some(entry) = entry else {
            return Pattern::Missing;
        };
        match doc.resolve_with_holder(entry) {
            (Some(id), value) if !value.is_null() => Pattern::Object(id),
            (None, Object::Dictionary(dict)) => {
                Pattern::Shading(shading_kind(doc, doc.dict_get(dict, b"Shading")))
            }
            _ => Pattern::Missing,
        }
    }

    /// The /ShadingType of the shading `name` names (0 when unknown).
    fn shading_type(&self, resources: Option<&'d Dictionary>, name: Option<&[u8]>) -> i64 {
        let doc = self.doc;
        let value = resources
            .and_then(|r| doc.dict_get(r, b"Shading").as_dict())
            .and_then(|s| s.get(name?));
        value.map_or(0, |v| shading_kind(doc, v))
    }

    fn inline_image(
        &mut self,
        frame: &Frame<'d>,
        stream: ObjectId,
        op: Range<usize>,
        stencil: bool,
    ) -> Result<(), WalkError> {
        if self.detail == Detail::Objects {
            return Ok(());
        }
        let matrix = frame.gs.ctm;
        let bounds = transform_rect(&matrix, [0.0, 0.0, 1.0, 1.0]);
        let kind = DrawKind::Image {
            object: None,
            matrix,
        };
        let index = self.draw(frame, kind, stream, op.clone(), op, bounds)?;
        if stencil {
            self.stencil(frame, index);
        }
        Ok(())
    }

    fn font(
        &mut self,
        resources: Option<&'d Dictionary>,
        name: Option<&[u8]>,
    ) -> Option<Arc<FontMetrics>> {
        let doc = self.doc;
        let fonts = resources.and_then(|r| doc.dict_get(r, b"Font").as_dict());
        let Some(entry) = fonts.and_then(|f| f.get(name?)) else {
            return Some(Arc::new(FontMetrics::unknown()));
        };
        let (holder, value) = doc.resolve_with_holder(entry);
        match value.as_dict() {
            Some(dict) => {
                Some(self.metrics(dict, holder.filter(|_| entry.as_reference().is_some())))
            }
            None => Some(Arc::new(FontMetrics::unknown())),
        }
    }

    /// A font's metrics, read once per walk; reading is charged as tokens.
    fn metrics(&mut self, dict: &'d Dictionary, object: Option<ObjectId>) -> Arc<FontMetrics> {
        let key = match object {
            Some(id) => FontKey::Object(id),
            None => FontKey::Direct(std::ptr::from_ref(dict).addr()),
        };
        if let Some(cached) = self.fonts.get(&key) {
            return cached.clone();
        }
        let before = self.font_parts.work;
        let metrics = Arc::new(self.font_parts.read(self.doc, dict, object));
        // Past the budget, the next token refuses the walk.
        let work = self.font_parts.work.saturating_sub(before);
        self.tokens = self.tokens.saturating_add(work);
        self.fonts.insert(key, metrics.clone());
        metrics
    }

    /// The view's advances (1000 units) for each code of `strings` shown
    /// with the current font; None per string it cannot measure.
    /// The view's advance per unit size of each of the 256 codes of a
    /// simple font without /Widths, measured once per font.
    fn table(&mut self, font: &FontMetrics, name: &[u8]) -> Option<Arc<Vec<Option<f64>>>> {
        if !self.measurable {
            return None;
        }
        let key = match font.object {
            Some(object) => (Some(object), Vec::new(), Vec::new()),
            None => (None, self.names.clone(), name.to_vec()),
        };
        if let Some(table) = self.tables.get(&key) {
            return table.clone();
        }
        let view = self.view;
        let page = self.page;
        let measurer = self
            .measurer
            .get_or_insert_with(|| view.and_then(|v| v.measurer(page).ok()));
        let codes: Vec<u8> = (0..=255).collect();
        let table = measurer.as_ref().and_then(|measurer| {
            let run = measurer
                .measure(&self.names, name, &[codes])
                .into_iter()
                .next()
                .flatten()?;
            Some(Arc::new(
                run.into_iter()
                    .map(|g| g.advance.map(|a| a / 1000.0))
                    .collect::<Vec<_>>(),
            ))
        });
        self.tables.insert(key, table.clone());
        table
    }

    #[allow(clippy::too_many_arguments)]
    fn show(
        &mut self,
        frame: &mut Frame<'d>,
        stream: ObjectId,
        op: &[u8],
        args: &[Arg],
        whole: Range<usize>,
        span: Range<usize>,
    ) -> Result<(), WalkError> {
        let font = frame
            .gs
            .font
            .clone()
            .unwrap_or_else(|| Arc::new(FontMetrics::unknown()));
        // The strings shown, with their operand index, and the adjustments.
        let mut items: Vec<Item> = Vec::new();
        let array = op == b"TJ";
        match op {
            b"TJ" => {
                if let Some(Arg::Arr(elements)) = args.first() {
                    for (i, element) in elements.iter().enumerate() {
                        match element {
                            Arg::Str(s) => items.push(Item::Str(i, s.clone())),
                            Arg::Num(n) if n.is_finite() => items.push(Item::Adjust(*n)),
                            _ => {}
                        }
                    }
                }
            }
            _ => {
                let index = if op == b"\"" { 2 } else { 0 };
                if let Some(Arg::Str(s)) = args.get(index) {
                    items.push(Item::Str(index, s.clone()));
                }
            }
        }
        // A simple font without /Widths: the view's advances.
        let table = match (font.needs_measure, frame.gs.font_name.clone()) {
            (true, Some(name)) => self.table(&font, &name),
            _ => None,
        };
        let gs = &frame.gs;
        let size = gs.size;
        let th = gs.scale;
        let vertical = font.vertical;
        // Text space per glyph unit along the line, and across it.
        let along = if vertical { 1.0 } else { th.abs() };
        let mut text = frame.text.clone();
        let mut glyphs: Vec<Glyph> = Vec::new();
        let mut ok = font.measured();
        let mut total = 0.0f64;
        for item in &items {
            match item {
                Item::Adjust(n) => {
                    let shift = -n / 1000.0 * size;
                    total += shift.abs() * along;
                    if ok {
                        let (tx, ty) = if vertical {
                            (0.0, shift)
                        } else {
                            (shift * th, 0.0)
                        };
                        text.tm = concat(&[1.0, 0.0, 0.0, 1.0, tx, ty], &text.tm);
                    }
                }
                Item::Str(operand, bytes) => {
                    let codes = font.split(bytes);
                    let n_codes = codes.as_ref().map_or(bytes.len(), Vec::len);
                    let per_code = font.max_advance * size.abs()
                        + gs.char_spacing.abs()
                        + gs.word_spacing.abs();
                    total += n_codes as f64 * per_code * along;
                    if !ok {
                        continue;
                    }
                    let Some(codes) = codes else {
                        ok = false;
                        continue;
                    };
                    for (code, range) in codes {
                        let advance = match (font.needs_measure, &table) {
                            (true, Some(table)) => table.get(code as usize).copied().flatten(),
                            (true, None) => None,
                            (false, _) => font.advance(code, range.len()),
                        };
                        let Some(w) = advance else {
                            ok = false;
                            break;
                        };
                        self.glyphs += 1;
                        if self.glyphs > self.limits.glyphs {
                            return Err(Self::too_complex("glyphs"));
                        }
                        let space = range.len() == 1 && code == 32;
                        let spacing = gs.char_spacing + if space { gs.word_spacing } else { 0.0 };
                        let slack = text.slack;
                        let (box_, advance) = if vertical {
                            let v = font.vertical_advance().unwrap_or(-1.0);
                            let half = w.abs().max(0.5) / 2.0 * size * th;
                            let low = v.min(font.descent - 0.88) * size + gs.rise;
                            let high = (font.ascent - 0.88).max(0.0) * size + gs.rise;
                            (
                                ordered([-half, low - slack, half, high + slack]),
                                v * size + spacing,
                            )
                        } else {
                            // A glyph without a usable width (a hostile
                            // /Widths 0, a combining mark) still has ink at
                            // the pen: its box reaches `INK_REACH` em
                            // either way, so a box over that ink takes it.
                            let (lo, hi) = match w.abs() < MIN_GLYPH_WIDTH {
                                true => (-INK_REACH, INK_REACH),
                                false => (w.min(0.0), w.max(0.0)),
                            };
                            let (x0, x1) = (lo * size * th, hi * size * th);
                            let (y0, y1) =
                                (font.descent * size + gs.rise, font.ascent * size + gs.rise);
                            (widen(ordered([x0, y0, x1, y1]), slack), w * size + spacing)
                        };
                        let m = concat(&text.tm, &gs.ctm);
                        let bounds = transform_rect(&m, box_);
                        let mut quad = [0f32; 8];
                        for (i, (x, y)) in [
                            (box_[0], box_[1]),
                            (box_[2], box_[1]),
                            (box_[2], box_[3]),
                            (box_[0], box_[3]),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            let (px, py) = annots::apply(&m, (x, y));
                            quad[2 * i] = px as f32;
                            quad[2 * i + 1] = py as f32;
                        }
                        glyphs.push(Glyph {
                            operand: *operand,
                            bytes: range,
                            code,
                            advance,
                            bounds,
                            quad,
                        });
                        let (tx, ty) = if vertical {
                            (0.0, advance)
                        } else {
                            (advance * th, 0.0)
                        };
                        text.tm = concat(&[1.0, 0.0, 0.0, 1.0, tx, ty], &text.tm);
                    }
                }
            }
        }
        let start_slack = frame.text.slack;
        let at = concat(&frame.text.tm, &gs.ctm);
        let (bounds, glyphs) = if ok {
            let bounds = glyphs.iter().map(|g| g.bounds).reduce(|a, b| {
                [
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].max(b[2]),
                    a[3].max(b[3]),
                ]
            });
            // An empty show still marks where the pen is.
            (
                bounds.unwrap_or_else(|| transform_rect(&at, [0.0; 4])),
                glyphs,
            )
        } else {
            // Wherever the glyphs could be: `total` either way along the line.
            let reach = total + start_slack;
            let height =
                |a: f64, d: f64| ordered([0.0, d * size + gs.rise, 0.0, a * size + gs.rise]);
            let box_ = if vertical {
                let half = font.max_advance.max(1.0) * (size * th).abs();
                [-half, -reach, half, reach]
            } else {
                let h = height(font.ascent.max(1.0), font.descent.min(-0.5));
                [-reach, h[1], reach, h[3]]
            };
            text = frame.text.clone();
            text.slack = reach;
            (transform_rect(&at, box_), Vec::new())
        };
        let draw = TextDraw {
            font: font.object,
            size,
            scale: th,
            render_mode: gs.mode,
            vertical,
            array,
            measured: ok,
            slack: start_slack,
            glyphs: if ok { glyphs } else { Vec::new() },
            font_name: gs.font_name.clone(),
            type3: font.type3,
        };
        frame.text = text;
        self.draw(frame, DrawKind::Text(draw), stream, whole, span, bounds)?;
        Ok(())
    }

    fn xobject(
        &mut self,
        frame: &mut Frame<'d>,
        stream: ObjectId,
        name: Option<&[u8]>,
        whole: Range<usize>,
        span: Range<usize>,
    ) -> Result<(), WalkError> {
        let doc = self.doc;
        let Some(name) = name else {
            return Ok(());
        };
        let entry = frame
            .resources
            .and_then(|r| doc.dict_get(r, b"XObject").as_dict())
            .and_then(|x| x.get(name));
        let Some(entry) = entry else {
            return Ok(());
        };
        let (holder, _) = doc.resolve_with_holder(entry);
        let Some(id) = holder else {
            return Ok(());
        };
        let Ok(dict) = doc.stream_dict(id) else {
            return Ok(());
        };
        match doc.dict_get(dict, b"Subtype").as_name() {
            Some(b"Image") => {
                let matrix = frame.gs.ctm;
                let bounds = transform_rect(&matrix, [0.0, 0.0, 1.0, 1.0]);
                let kind = DrawKind::Image {
                    object: Some(id),
                    matrix,
                };
                let index = self.draw(frame, kind, stream, whole, span, bounds)?;
                if doc.dict_get(dict, b"ImageMask").as_bool() == Some(true) {
                    self.stencil(frame, index);
                }
            }
            Some(b"Form") => {
                let form_matrix = (doc.dict_get(dict, b"Matrix").as_array())
                    .filter(|m| m.len() == 6)
                    .map(|m| {
                        let n =
                            |i: usize| m.get(i).and_then(|v| annots::number(doc, v)).unwrap_or(0.0);
                        [n(0), n(1), n(2), n(3), n(4), n(5)]
                    })
                    .unwrap_or(appearance::IDENTITY);
                let matrix = concat(&form_matrix, &frame.gs.ctm);
                let bbox = dict
                    .get(b"BBox".as_slice())
                    .and_then(|b| annots::rect(doc, b));
                let bounds = match bbox {
                    Some(bbox) => transform_rect(&matrix, bbox),
                    None => frame.gs.clip.unwrap_or(self.area),
                };
                let enter = !self.active.contains(&id);
                let kind = DrawKind::Form {
                    object: id,
                    matrix,
                    bbox,
                    entered: enter,
                };
                let index = self.draw(frame, kind, stream, whole, span, bounds)?;
                if !enter {
                    return Ok(());
                }
                if self.active.len() >= self.limits.depth {
                    return Err(Self::too_complex("Form nesting"));
                }
                let own = dict
                    .get(b"Resources".as_slice())
                    .and_then(|r| doc.resolve(r).as_dict());
                let mut gs = frame.gs.clone();
                gs.ctm = matrix;
                if bbox.is_some() {
                    gs.clip = Some(intersect(frame.gs.clip, bounds));
                }
                let bytes = self.read(id)?;
                let mut child = Frame::new(own.or(frame.resources), Some(index), gs);
                child.owner = match own {
                    Some(_) => Owner::Form(id),
                    None => frame.owner,
                };
                self.active.push(id);
                self.names.push(name.to_vec());
                let result = self.run(&mut child, id, &bytes, &mut Pending::default());
                self.names.pop();
                self.active.pop();
                result?;
            }
            _ => {}
        }
        Ok(())
    }
}

enum Item {
    Str(usize, Vec<u8>),
    Adjust(f64),
}

/// A box with its corners in order.
fn ordered(r: [f64; 4]) -> [f64; 4] {
    [
        r[0].min(r[2]),
        r[1].min(r[3]),
        r[0].max(r[2]),
        r[1].max(r[3]),
    ]
}

/// A box grown along x by `by` on both sides.
fn widen(r: [f64; 4], by: f64) -> [f64; 4] {
    [r[0] - by, r[1], r[2] + by, r[3]]
}

fn move_line(text: &mut TextPos, x: f64, y: f64) {
    text.tlm = concat(&[1.0, 0.0, 0.0, 1.0, x, y], &text.tlm);
    text.tm = text.tlm;
    text.slack = 0.0;
}

fn pairs(items: Vec<Arg>) -> Vec<(Vec<u8>, Arg)> {
    let mut out = Vec::new();
    let mut iter = items.into_iter();
    while let (Some(key), Some(value)) = (iter.next(), iter.next()) {
        if let Arg::Name(key) = key {
            out.push((key, value));
        }
    }
    out
}

/// The /ShadingType of a shading dictionary or stream (0 when unknown).
pub fn shading_kind(doc: &Document<'_>, value: &Object) -> i64 {
    let dict = match doc.resolve_with_holder(value) {
        (Some(id), _) => doc
            .stream_dict(id)
            .ok()
            .or_else(|| doc.resolve(value).as_dict()),
        (None, value) => value.as_dict(),
    };
    dict.and_then(|d| doc.dict_get(d, b"ShadingType").as_i64())
        .unwrap_or(0)
}

/// The page's /Contents streams in order (non-streams skipped).
pub fn content_streams(doc: &Document<'_>, page: &Dictionary) -> Vec<ObjectId> {
    let Some(contents) = page.get(b"Contents".as_slice()) else {
        return Vec::new();
    };
    let is_stream = |id: ObjectId| doc.object(id).is_ok_and(|o| o.stream.is_some());
    let one = |value: &Object| {
        let (holder, _) = doc.resolve_with_holder(value);
        holder.filter(|&id| is_stream(id))
    };
    if let Some(id) = one(contents) {
        return vec![id];
    }
    match doc.resolve(contents).as_array() {
        Some(items) => items.iter().filter_map(one).collect(),
        None => Vec::new(),
    }
}

fn inline_entry<'a>(args: &'a [Arg], long: &[u8], short: &[u8]) -> Option<&'a Arg> {
    let mut iter = args.iter();
    while let (Some(key), Some(value)) = (iter.next(), iter.next()) {
        if let Some(key) = key.name() {
            if key == long || key == short {
                return Some(value);
            }
        }
    }
    None
}

/// White space read between an inline image's data and its `EI`.
const MAX_EI_GAP: usize = 64;
/// Bytes after an `EI` that must be text for it to end binary data.
const EI_CONTEXT: usize = 64;

/// Where an inline image's operation ends (just past `EI`) by its declared
/// /Length, else by the size an unfiltered image's /Width, /Height,
/// /BitsPerComponent and colour space give; None when neither is followed
/// by `EI` (`EiScan` then searches). `data` is where its data starts.
fn inline_end(bytes: &[u8], data: usize, args: &[Arg]) -> Option<usize> {
    let int = |long: &[u8], short: &[u8]| {
        inline_entry(args, long, short)
            .and_then(Arg::num)
            .filter(|v| v.fract() == 0.0 && *v >= 0.0)
            .map(|v| v as u64)
    };
    let declared = int(b"Length", b"L");
    let filtered = inline_entry(args, b"Filter", b"F").is_some_and(|f| match f {
        Arg::Arr(items) => !items.is_empty(),
        _ => true,
    });
    let computed = if filtered {
        None
    } else {
        let mask = matches!(
            inline_entry(args, b"ImageMask", b"IM"),
            Some(Arg::Bool(true))
        );
        let comps = match inline_entry(args, b"ColorSpace", b"CS") {
            _ if mask => Some(1),
            Some(Arg::Name(n)) => match n.as_slice() {
                b"G" | b"DeviceGray" | b"CalGray" | b"I" | b"Indexed" => Some(1),
                b"RGB" | b"DeviceRGB" | b"CalRGB" => Some(3),
                b"CMYK" | b"DeviceCMYK" => Some(4),
                _ => None,
            },
            Some(Arg::Arr(items)) => match items.first().and_then(Arg::name) {
                Some(b"I" | b"Indexed") => Some(1),
                _ => None,
            },
            None if mask => Some(1),
            _ => None,
        };
        let bpc = if mask {
            Some(1)
        } else {
            int(b"BitsPerComponent", b"BPC")
        };
        match (int(b"Width", b"W"), int(b"Height", b"H"), bpc, comps) {
            (Some(w), Some(h), Some(bpc), Some(comps))
                if w.saturating_mul(h) <= MAX_INLINE_PIXELS =>
            {
                let row = w.saturating_mul(comps).saturating_mul(bpc).div_ceil(8);
                Some(row.saturating_mul(h))
            }
            _ => None,
        }
    };
    // A bounded gap: each image reads at most `MAX_EI_GAP` bytes here.
    let ei_at = |at: usize| {
        let gap = (bytes.get(at..).unwrap_or(&[]).iter())
            .take(MAX_EI_GAP)
            .take_while(|&&b| is_white(Some(b)))
            .count();
        let p = at + gap;
        (bytes.get(p..p + 2) == Some(b"EI") && ends_token(bytes, p + 2)).then_some(p + 2)
    };
    [declared, computed]
        .into_iter()
        .flatten()
        .filter_map(|length| data.checked_add(usize::try_from(length).ok()?))
        .filter(|&end| end <= bytes.len())
        .find_map(ei_at)
}

/// The search for the end of inline images of unknown length in one
/// stream: the first `EI` between white space that text follows (64 bytes,
/// or to the stream's end), else the first `EI` between white space. Data
/// starts only move forward in a walk and answers are kept for the next
/// walk of the stream, so a stream is read about once however many images
/// it holds.
#[derive(Debug, Default)]
struct EiScan {
    /// Answers by data start.
    answers: BTreeMap<usize, Option<usize>>,
    /// `(from, at)`: the first `EI` text follows at or after `from`.
    content: Option<(usize, Option<usize>)>,
    /// `(from, at)`: the first byte that is not text at or after `from`.
    binary: Option<(usize, usize)>,
}

fn is_text(b: u8) -> bool {
    matches!(b, 9..=13 | 32..=126)
}

impl EiScan {
    /// Just past the `EI` ending an image whose data starts at `data`.
    fn find(&mut self, bytes: &[u8], data: usize) -> Option<usize> {
        if let Some(&answer) = self.answers.get(&data) {
            return answer;
        }
        let content = match self.content {
            Some((from, at)) if from <= data && at.is_none_or(|at| at >= data) => at,
            _ => {
                let at = self.first(bytes, data, true);
                self.content = Some((data, at));
                at
            }
        };
        let answer = content
            .or_else(|| self.first(bytes, data, false))
            .map(|at| at + 2);
        self.answers.insert(data, answer);
        answer
    }

    /// The first `EI` between white space at or after `from`, text after
    /// it when `content`.
    fn first(&mut self, bytes: &[u8], from: usize, content: bool) -> Option<usize> {
        let mut p = from;
        while let Some(offset) = bytes
            .get(p..)
            .and_then(|rest| rest.iter().position(|&b| b == b'E'))
        {
            let at = p + offset;
            p = at + 1;
            let candidate = bytes.get(at + 1) == Some(&b'I')
                && at > 0
                && is_white(bytes.get(at - 1).copied())
                && ends_token(bytes, at + 2);
            if candidate && (!content || self.text_follows(bytes, at + 2)) {
                return Some(at);
            }
        }
        None
    }

    /// The bytes from `at` for `EI_CONTEXT` (or to the end) are text.
    fn text_follows(&mut self, bytes: &[u8], at: usize) -> bool {
        let binary = match self.binary {
            Some((from, found)) if from <= at && at <= found => found,
            _ => {
                let found = (bytes.get(at..))
                    .and_then(|rest| rest.iter().position(|&b| !is_text(b)))
                    .map_or(bytes.len(), |offset| at + offset);
                self.binary = Some((at, found));
                found
            }
        };
        binary >= bytes.len().min(at + EI_CONTEXT)
    }
}

fn over_limit(kind: &StreamObjectErrorKind) -> bool {
    matches!(kind, StreamObjectErrorKind::Decode(error)
        if matches!(error.kind, StreamErrorKind::DecodedOutputLimit { .. }
            | StreamErrorKind::TotalOutputLimit { .. }))
}

fn ends_token(bytes: &[u8], at: usize) -> bool {
    match bytes.get(at) {
        None => true,
        Some(&b) => is_white(Some(b)) || is_delimiter(b),
    }
}

/// The XObjects `resources` can draw, at any depth through Forms; each
/// XObject entry visited takes one from `budget`, and None is returned
/// when it runs out.
pub fn reachable_xobjects(
    doc: &Document<'_>,
    resources: Option<&Dictionary>,
    budget: &mut usize,
) -> Option<BTreeSet<ObjectId>> {
    let mut found = BTreeSet::new();
    let mut queue: Vec<&Dictionary> = resources.into_iter().collect();
    while let Some(resources) = queue.pop() {
        let Some(xobjects) = doc.dict_get(resources, b"XObject").as_dict() else {
            continue;
        };
        for value in xobjects.values() {
            if *budget == 0 {
                return None;
            }
            *budget -= 1;
            let (Some(id), _) = doc.resolve_with_holder(value) else {
                continue;
            };
            if !found.insert(id) {
                continue;
            }
            let Ok(dict) = doc.stream_dict(id) else {
                continue;
            };
            if doc.dict_get(dict, b"Subtype").as_name() == Some(b"Form") {
                if let Some(inner) = doc.dict_get(dict, b"Resources").as_dict() {
                    queue.push(inner);
                }
            }
        }
    }
    Some(found)
}
