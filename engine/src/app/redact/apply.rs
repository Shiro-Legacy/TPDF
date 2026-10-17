// SPDX-License-Identifier: AGPL-3.0-or-later
//! redact-apply's removal (redact.ts `applyRedactions`: MuPDF's page
//! `applyRedactions` with black boxes, image pixels, line art and text),
//! written into a scratch copy of the document so the session never
//! changes. What goes is decided by `region` and the walk (`content`):
//!
//! - text: each glyph `GlyphRule::removes_glyph` takes leaves its show,
//!   the shows' strings rewritten as `TJ` arrays whose adjustment takes the
//!   removed run's place (one number for a run, so the kept text does not
//!   move);
//!   a run whose glyphs cannot be placed goes whole when it reaches a mark;
//! - paths a mark touches are cut by it (`geometry`): fills and clips keep
//!   the area outside the marks, strokes the lines outside; a path wholly
//!   under a mark goes (a clip becomes empty);
//! - image XObjects (with their soft masks and masks) and inline images
//!   have the pixels under the marks overwritten; a placement wholly under
//!   a mark is no longer drawn, and an image left wholly blank becomes one
//!   blank pixel;
//! - axial and radial shadings are clipped out of the marks;
//! - the alternate texts (/ActualText, /Alt, /E) of marked-content
//!   sequences around removed content go, inline or in /Properties, and so
//!   do those of the structure elements of its MCIDs and their ancestors;
//! - the annotations a mark touches go (Widgets with their fields, Links,
//!   Popups), and every mark;
//! - each marked page's /Contents become one new stream: the old content
//!   wrapped in `q`/`Q` (a `Q` with nothing to restore dropped), comments
//!   dropped, then a black box per mark rect.
//!
//! A Form is rewritten in place, once for all its placements (the preview
//! refused any placement the marks do not cover); its edits from several
//! placements are merged. An image's soft mask and mask are blanked in a
//! copy (another image may share them). Derived copies of what a mark
//! covers go too: a marked page's /Thumb, /PieceInfo and /Metadata, a
//! rewritten image's or Form's /Metadata and /PieceInfo. Content that
//! cannot be redacted exactly refuses (`Unsupported`): an undecodable
//! image, a mesh or function shading under a mark, a show placed
//! differently by two placements of one Form, a paint whose tiling pattern
//! or soft-mask group shows text, images or such a shading, a Type 3 font
//! whose glyphs draw images or text.
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

use crate::app::annots::{self, PageSpace, Point, appearance, invert};
use crate::app::edit::{Builder, Edit, Place};
use crate::app::info;
use crate::app::ops::edit::lex::{Container, Lexer, Operand, Token};
use crate::app::ops::edit::scan::struct_element_in;
use crate::app::ops::fields;
use crate::app::pagetree;
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};
use crate::view::ViewDocument;

use super::content::{
    self, Content, Detail, Draw, DrawKind, Limits, Owner, Pattern, TextDraw, WalkError, Walker,
};
use super::geometry::{self, Budget, Parsed, Region, Use};
use super::images::{self, ImageError};
use super::inventory::STRUCT_KEYS;
use super::marks::Mark;
use super::region::{self, EDGE_REACH, GlyphRule, Regions, reach};
/// Path segments every cut may examine in all.
const MAX_PATH_WORK: usize = 20_000_000;
/// Mark regions the planned edits hold in all, each a draw's copy of a
/// rect (a path or shading cut, an image's or inline image's quads), before
/// any is cut: 64 bytes each, so at most 64 MiB. Without it, 50,000 clips
/// in a Form under 100 search hits held 320 MB of regions before the cut
/// budget refused, and the walk's 500,000 draws under 2,000 marks would
/// pass wasm32's 4 GiB.
const MAX_HELD_REGIONS: usize = 1 << 20;
/// Flattening tolerance, in page points.
const FLATNESS: f64 = 0.02;
/// Coordinates a rewritten path may hold (user space).
const MAX_COORDINATE: f64 = 1e9;
/// Structure ancestors cleaned above an element at most.
const MAX_STRUCT_DEPTH: usize = 64;
/// ParentTree nodes and entries read at most.
const MAX_PARENT_TREE: usize = 1_000_000;
/// Decoded page content one apply joins and rewrites in all (a /Contents
/// element listed again counts again).
const MAX_JOINED_BYTES: usize = 256 << 20;
/// Content tokens the checks of patterns, soft-mask groups and Type 3
/// glyph procedures read in all.
const MAX_HIDDEN_TOKENS: usize = 32_000_000;
/// Patterns and soft-mask groups nested in what those checks read.
const MAX_HIDDEN_DEPTH: usize = 8;
/// Page keys holding derived copies of the page (a thumbnail, application
/// data, XMP), dropped from marked pages.
const PAGE_DERIVED: [&[u8]; 3] = [b"Thumb", b"PieceInfo", b"Metadata"];
/// Stream keys holding derived copies of a rewritten image or Form.
const STREAM_DERIVED: [&[u8]; 3] = [b"Metadata", b"PieceInfo", b"OPI"];

/// Why apply stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyError {
    /// A marked page's content passed the walk's limits.
    TooComplex(usize),
    /// A marked page's content cannot be read.
    Unreadable(usize, String),
    /// An image past the pixel bound, or work past a bound.
    TooLarge(String),
    /// Content under a mark this version cannot redact exactly.
    Unsupported(String),
    /// A write failed.
    Write(String),
}

impl From<crate::document::DocumentError> for ApplyError {
    fn from(error: crate::document::DocumentError) -> Self {
        Self::Write(error.to_string())
    }
}

impl From<crate::app::AppError> for ApplyError {
    fn from(error: crate::app::AppError) -> Self {
        Self::Write(error.message)
    }
}

/// A byte range of decoded content and what replaces it.
type Splice = (Range<usize>, Vec<u8>);

/// The stream an edit is in: a page's /Contents (offsets into their
/// concatenation) or a Form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Target {
    Page(usize),
    Form(ObjectId),
}

/// One operation's edit, merged over every placement that reaches it.
#[derive(Clone, Debug)]
enum OpEdit {
    Show {
        text: TextDraw,
        removed: BTreeSet<usize>,
        whole: bool,
        /// At size 0, the font's name in the stream's resources: the pen
        /// moves past removed glyphs at size 1 (`Tf`), where an adjustment
        /// says how far.
        pen: Option<Vec<u8>>,
    },
    Path {
        fill: bool,
        stroke: bool,
        clips: bool,
        regions: Vec<Region>,
        tolerance: f64,
        remove: bool,
    },
    /// An operation drawn wholly under a mark (an image's `Do`).
    Remove,
    Inline {
        quads: Vec<[Point; 4]>,
        remove: bool,
        resources: Owner,
        page: usize,
    },
    Shading {
        regions: Vec<Region>,
        outer: Option<Region>,
        resources: Owner,
        page: usize,
    },
    /// A property list whose alternate texts go.
    Alt,
}

impl OpEdit {
    fn merge(&mut self, other: OpEdit) -> Result<(), ApplyError> {
        match (self, other) {
            (
                OpEdit::Show {
                    text,
                    removed,
                    whole,
                    ..
                },
                OpEdit::Show {
                    text: other,
                    removed: more,
                    whole: all,
                    ..
                },
            ) => {
                let same = text.size == other.size
                    && text.glyphs.len() == other.glyphs.len()
                    && text.glyphs.iter().zip(&other.glyphs).all(|(a, b)| {
                        a.operand == b.operand && a.bytes == b.bytes && a.advance == b.advance
                    });
                if !same || text.measured != other.measured {
                    return Err(ApplyError::Unsupported(
                        "a Form shows text differently where the marks meet it".into(),
                    ));
                }
                removed.extend(more);
                *whole |= all;
            }
            (
                OpEdit::Path {
                    regions,
                    tolerance,
                    remove,
                    ..
                },
                OpEdit::Path {
                    regions: more,
                    tolerance: t,
                    remove: r,
                    ..
                },
            ) => {
                regions.extend(more);
                *tolerance = tolerance.min(t);
                *remove |= r;
            }
            (
                OpEdit::Inline { quads, remove, .. },
                OpEdit::Inline {
                    quads: q,
                    remove: r,
                    ..
                },
            ) => {
                quads.extend(q);
                *remove |= r;
            }
            (OpEdit::Shading { regions, .. }, OpEdit::Shading { regions: more, .. }) => {
                regions.extend(more);
            }
            (this, OpEdit::Remove) => *this = OpEdit::Remove,
            _ => {}
        }
        Ok(())
    }
}

/// A marked page's /Contents, read for its rewrite.
struct PagePlan {
    page: usize,
    id: ObjectId,
    joined: Vec<u8>,
    opaque: Vec<Range<usize>>,
    excess: Vec<Range<usize>>,
    open_saves: usize,
    open_text: bool,
    open_marked: usize,
    space: PageSpace,
    rects: Vec<[f64; 4]>,
}

/// Content the walk does not follow, checked when a redacted draw paints
/// with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Hidden {
    /// A tiling pattern's cell.
    Pattern,
    /// A soft-mask group (ExtGState /SMask /G).
    Group,
    /// A Type 3 font's glyph procedures.
    Glyphs,
}

/// Whose /StructParents an MCID is under.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum StructOwner {
    Page(ObjectId),
    Form(ObjectId),
}

/// What apply did, for the reply and tests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// The marked pages, ascending.
    pub pages: Vec<usize>,
    pub glyphs: usize,
    pub paths: usize,
    pub images: usize,
    pub annotations: usize,
}

#[derive(Default)]
struct Planner {
    ops: BTreeMap<(Target, usize, usize), OpEdit>,
    forms: BTreeMap<ObjectId, Arc<Vec<u8>>>,
    form_opaque: BTreeMap<ObjectId, BTreeSet<(usize, usize)>>,
    pages: Vec<PagePlan>,
    /// Unit-square quads under marks, by image object.
    images: BTreeMap<ObjectId, Vec<[Point; 4]>>,
    /// The page each image object is first touched on.
    image_pages: BTreeMap<ObjectId, usize>,
    /// Patterns, groups and Type 3 fonts checked: whether they show
    /// content apply cannot rewrite.
    hidden: BTreeMap<(ObjectId, Hidden), bool>,
    hidden_tokens: usize,
    /// Joined page content so far.
    joined: usize,
    named: BTreeSet<(usize, Owner, Vec<u8>)>,
    /// Sequences already noted, by page.
    noted: BTreeSet<(usize, usize)>,
    mcids: BTreeSet<(StructOwner, i64)>,
    applied: Applied,
    /// Regions the planned edits hold (`MAX_HELD_REGIONS`).
    held: usize,
}

fn walk_error(page: usize, error: WalkError) -> ApplyError {
    match error {
        WalkError::TooComplex(_) => ApplyError::TooComplex(page),
        WalkError::Unreadable(why) => ApplyError::Unreadable(page, why),
    }
}

/// How much `m` stretches a length at most.
fn stretch(m: &[f64; 6]) -> f64 {
    let s = m[0] * m[0] + m[1] * m[1] + m[2] * m[2] + m[3] * m[3];
    let det = m[0] * m[3] - m[1] * m[2];
    ((s + (s * s - 4.0 * det * det).max(0.0).sqrt()) / 2.0).sqrt()
}

fn singular(m: &[f64; 6]) -> bool {
    let det = m[0] * m[3] - m[1] * m[2];
    det == 0.0 || !det.is_finite()
}

/// The resources a draw's operator names its resources in.
fn owner_of(content: &Content, draw: &Draw, doc: &Document<'_>) -> Owner {
    let mut at = draw.parent;
    for _ in 0..=Limits::DEFAULT.depth + 1 {
        let Some(index) = at else {
            return Owner::Page;
        };
        let Some(parent) = content.draws.get(index) else {
            return Owner::Page;
        };
        if let DrawKind::Form { object, .. } = parent.kind {
            let own = doc
                .stream_dict(object)
                .ok()
                .is_some_and(|d| d.get(b"Resources".as_slice()).is_some());
            if own {
                return Owner::Form(object);
            }
        }
        at = parent.parent;
    }
    Owner::Page
}

/// The resources dictionary `owner` names.
fn resources<'d>(
    doc: &'d Document<'_>,
    page: &'d Dictionary,
    owner: Owner,
) -> Option<&'d Dictionary> {
    match owner {
        Owner::Page => {
            pagetree::inherited(doc, page, b"Resources").and_then(|r| doc.resolve(r).as_dict())
        }
        Owner::Form(id) => doc
            .dict_get(doc.stream_dict(id).ok()?, b"Resources")
            .as_dict(),
    }
}

impl Planner {
    /// Charges `n` held regions (`MAX_HELD_REGIONS`).
    fn hold(&mut self, n: usize, what: &str) -> Result<(), ApplyError> {
        self.held = self.held.saturating_add(n);
        if self.held > MAX_HELD_REGIONS {
            return Err(ApplyError::TooLarge(format!(
                "the {what} under the marks are too complex"
            )));
        }
        Ok(())
    }

    fn add(&mut self, key: (Target, usize, usize), edit: OpEdit) -> Result<(), ApplyError> {
        match self.ops.get_mut(&key) {
            Some(existing) => existing.merge(edit),
            None => {
                self.ops.insert(key, edit);
                Ok(())
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn plan_page(
        &mut self,
        doc: &Document<'static>,
        view: Option<&ViewDocument>,
        rule: &GlyphRule,
        marks: &[Mark],
        page: usize,
    ) -> Result<(), ApplyError> {
        let regions: &Regions = rule.drawn();
        let content = region::page_content(doc, view, page, Detail::Apply, Limits::DEFAULT)
            .map_err(|e| walk_error(page, e))?;
        let pages = info::pages(doc);
        let page_ref = pages
            .get(page)
            .ok_or_else(|| ApplyError::Unreadable(page, "no such page".into()))?;
        let id = page_ref.id.ok_or_else(|| {
            ApplyError::Unsupported(format!(
                "page {} is written into its page tree and cannot be rewritten",
                page + 1
            ))
        })?;
        let dict = page_ref.dict;
        let space = PageSpace::of(doc, dict);
        let area = space.bounds();
        let size = (content.elements.iter())
            .map(|s| {
                content
                    .streams
                    .get(s)
                    .map_or(1, |b| b.len().saturating_add(1))
            })
            .fold(0usize, usize::saturating_add);
        self.joined = self.joined.saturating_add(size);
        if self.joined > MAX_JOINED_BYTES {
            return Err(ApplyError::TooLarge(
                "the marked pages' content is too large to rewrite".into(),
            ));
        }
        let mut base = Vec::with_capacity(content.elements.len());
        let mut joined = Vec::with_capacity(size);
        for (i, stream) in content.elements.iter().enumerate() {
            if i > 0 {
                joined.push(b'\n');
            }
            base.push(joined.len());
            if let Some(bytes) = content.streams.get(stream) {
                joined.extend_from_slice(bytes);
            }
        }
        let at = |element: usize, offset: usize| base.get(element).map(|b| b + offset);
        let span = |draw: &Draw| -> Option<(Target, usize, usize)> {
            if draw.parent.is_none() {
                let start = match draw.from {
                    Some((e, o)) => at(e, o)?,
                    None => at(draw.element, draw.start)?,
                };
                Some((Target::Page(page), start, at(draw.element, draw.op.end)?))
            } else {
                Some((Target::Form(draw.stream), draw.start, draw.op.end))
            }
        };
        let mut opaque = Vec::new();
        for draw in &content.draws {
            if let DrawKind::Image { object: None, .. } = draw.kind {
                match span(draw) {
                    Some((Target::Page(_), s, e)) => opaque.push(s..e),
                    Some((Target::Form(f), s, e)) => {
                        self.form_opaque.entry(f).or_default().insert((s, e));
                    }
                    None => {}
                }
            }
        }
        for stream in content.streams.keys() {
            if !content.elements.contains(stream) {
                if let Some(bytes) = content.streams.get(stream) {
                    self.forms.entry(*stream).or_insert_with(|| bytes.clone());
                }
            }
        }
        for (index, draw) in content.draws.iter().enumerate() {
            let Some(key) = span(draw) else {
                continue;
            };
            let touched = regions.touches(page, &draw.bounds);
            let edit = match &draw.kind {
                DrawKind::Text(text) if text.measured => {
                    let removed: BTreeSet<usize> = (text.glyphs.iter().enumerate())
                        .filter(|(_, g)| rule.removes_glyph(page, &g.bounds, &g.quad))
                        .map(|(i, _)| i)
                        .collect();
                    self.applied.glyphs += removed.len();
                    let pen = match (text.size == 0.0, &text.font_name, text.font) {
                        (true, Some(name), Some(font)) => {
                            let own = resources(doc, dict, owner_of(&content, draw, doc));
                            let entry = (own.and_then(|r| doc.dict_get(r, b"Font").as_dict()))
                                .and_then(|f| f.get(name.as_slice()));
                            let named = entry.and_then(|e| doc.resolve_with_holder(e).0);
                            (named == Some(font)).then(|| name.clone())
                        }
                        _ => None,
                    };
                    (!removed.is_empty()).then(|| OpEdit::Show {
                        text: text.clone(),
                        removed,
                        whole: false,
                        pen,
                    })
                }
                DrawKind::Text(text) => rule.reaches(page, &draw.bounds).then(|| OpEdit::Show {
                    text: text.clone(),
                    removed: BTreeSet::new(),
                    whole: true,
                    pen: None,
                }),
                DrawKind::Path { fill, stroke } if touched => {
                    self.applied.paths += 1;
                    let remove = regions.contains(page, &draw.bounds)
                        || singular(&draw.ctm)
                        || (!fill && !stroke && !draw.clips);
                    let back = invert(draw.ctm);
                    // A path that goes is not cut: it holds no regions.
                    let local: Vec<Region> = match remove {
                        true => Vec::new(),
                        false => (regions.touching(page, &draw.bounds).iter())
                            .filter_map(|r| Region::of(reach(*r, area), &back))
                            .collect(),
                    };
                    self.hold(local.len(), "paths")?;
                    Some(OpEdit::Path {
                        fill: *fill,
                        stroke: *stroke,
                        clips: draw.clips,
                        regions: local,
                        tolerance: FLATNESS / stretch(&draw.ctm).max(f64::MIN_POSITIVE),
                        remove,
                    })
                }
                DrawKind::Image { object, matrix } if touched => {
                    let rects: Vec<[f64; 4]> = (regions.touching(page, &draw.bounds).iter())
                        .map(|r| reach(*r, area))
                        .collect();
                    let whole = regions.contains(page, &draw.bounds);
                    let unit = images::unit_regions(&rects, matrix);
                    self.hold(unit.as_ref().map_or(0, Vec::len), "images")?;
                    match (object, unit) {
                        (_, None) => Some(OpEdit::Remove),
                        (Some(image), Some(unit)) => {
                            self.images.entry(*image).or_default().extend(unit);
                            self.image_pages.entry(*image).or_insert(page);
                            whole.then_some(OpEdit::Remove)
                        }
                        (None, Some(unit)) => Some(OpEdit::Inline {
                            quads: unit,
                            remove: whole,
                            resources: owner_of(&content, draw, doc),
                            page,
                        }),
                    }
                }
                DrawKind::Shading if touched => {
                    let back = invert(draw.ctm);
                    let local: Vec<Region> = (regions.touching(page, &draw.bounds).iter())
                        .filter_map(|r| Region::of(reach(*r, area), &back))
                        .collect();
                    self.hold(local.len(), "shadings")?;
                    let outer = Region::of(
                        [
                            area[0] - EDGE_REACH,
                            area[1] - EDGE_REACH,
                            area[2] + EDGE_REACH,
                            area[3] + EDGE_REACH,
                        ],
                        &back,
                    );
                    match singular(&draw.ctm) {
                        true => Some(OpEdit::Remove),
                        false => Some(OpEdit::Shading {
                            regions: local,
                            outer,
                            resources: owner_of(&content, draw, doc),
                            page,
                        }),
                    }
                }
                _ => None,
            };
            // An image object a mark touches has its pixels blanked whether
            // or not its operation changes.
            let blanked = touched
                && matches!(
                    draw.kind,
                    DrawKind::Image {
                        object: Some(_),
                        ..
                    }
                );
            if edit.is_some() || blanked {
                self.check_paint(doc, dict, &content, draw, page, area)?;
                self.note(&content, page, id, index, &base)?;
            }
            if let Some(edit) = edit {
                self.add(key, edit)?;
            }
        }
        let at = |element: usize, offset: usize| base.get(element).map(|b| b + offset);
        let excess = (content.excess.iter())
            .filter_map(|(e, r)| Some(at(*e, r.start)?..at(*e, r.end)?))
            .collect();
        // Each distinct box once: a file can bring 262,144 copies of one
        // quad, and one path of that many boxes is past what the view draws.
        let mut seen = BTreeSet::new();
        let rects = (marks.iter())
            .filter(|m| m.page == page)
            .flat_map(Mark::rects)
            .filter(|r| seen.insert(r.map(f64::to_bits)))
            .collect();
        self.pages.push(PagePlan {
            page,
            id,
            joined,
            opaque,
            excess,
            open_saves: content.open_saves,
            open_text: content.open_text,
            open_marked: content.open_marked,
            space,
            rects,
        });
        Ok(())
    }

    /// Refuses a redacted draw that paints with content apply cannot
    /// rewrite: a tiling pattern or soft-mask group showing text, images or
    /// a mesh or function shading, such a shading pattern, a Type 3 font
    /// whose glyphs draw images or text.
    fn check_paint(
        &mut self,
        doc: &Document<'static>,
        page_dict: &Dictionary,
        content: &Content,
        draw: &Draw,
        page: usize,
        area: [f64; 4],
    ) -> Result<(), ApplyError> {
        let paint = draw.paint;
        for pattern in [paint.fill, paint.stroke].into_iter().flatten() {
            if self.pattern_keeps(doc, page, area, pattern, 0)? {
                return Err(ApplyError::Unsupported(
                    "a mark covers content painted with a pattern that shows text, images or a mesh or function shading".into(),
                ));
            }
        }
        if let Some(group) = paint.soft_mask {
            if self.keeps(doc, page, area, group, Hidden::Group, None, 0)? {
                return Err(ApplyError::Unsupported(
                    "a mark covers content under a soft mask that shows text, images or a mesh or function shading".into(),
                ));
            }
        }
        let DrawKind::Text(text) = &draw.kind else {
            return Ok(());
        };
        if !text.type3 {
            return Ok(());
        }
        let own = resources(doc, page_dict, owner_of(content, draw, doc));
        let font = match text.font {
            Some(id) => doc.resolve_id(id).as_dict().map(|d| (Some(id), d)),
            None => text.font_name.as_deref().and_then(|name| {
                let entry = doc.dict_get(own?, b"Font").as_dict()?.get(name)?;
                let (holder, value) = doc.resolve_with_holder(entry);
                Some((holder, value.as_dict()?))
            }),
        };
        let keeps = match font {
            Some((id, dict)) => self.glyphs_keep(doc, page, area, id, dict, own)?,
            None => true,
        };
        match keeps {
            true => Err(ApplyError::Unsupported(
                "a mark covers text in a Type 3 font whose glyphs draw images or text".into(),
            )),
            false => Ok(()),
        }
    }

    /// Whether a pattern shows content apply cannot rewrite.
    fn pattern_keeps(
        &mut self,
        doc: &Document<'static>,
        page: usize,
        area: [f64; 4],
        pattern: Pattern,
        depth: usize,
    ) -> Result<bool, ApplyError> {
        let positional = |kind: i64| !matches!(kind, 2 | 3);
        match pattern {
            Pattern::Missing => Ok(false),
            Pattern::Shading(kind) => Ok(positional(kind)),
            Pattern::Object(id) => {
                let Some(dict) = doc.resolve_id(id).as_dict() else {
                    return Ok(false);
                };
                match doc.dict_get(dict, b"PatternType").as_i64() {
                    Some(1) => self.keeps(doc, page, area, id, Hidden::Pattern, None, depth),
                    Some(2) => {
                        let shading = dict.get(b"Shading".as_slice()).unwrap_or(&Object::Null);
                        Ok(positional(content::shading_kind(doc, shading)))
                    }
                    _ => Ok(false),
                }
            }
        }
    }

    /// Whether a Type 3 font's glyph procedures draw images (other than
    /// stencil masks), text or shadings.
    fn glyphs_keep(
        &mut self,
        doc: &Document<'static>,
        page: usize,
        area: [f64; 4],
        id: Option<ObjectId>,
        font: &Dictionary,
        fallback: Option<&Dictionary>,
    ) -> Result<bool, ApplyError> {
        if let Some(known) = id.and_then(|id| self.hidden.get(&(id, Hidden::Glyphs))) {
            return Ok(*known);
        }
        let own = doc.dict_get(font, b"Resources").as_dict().or(fallback);
        let procs: Vec<ObjectId> = (doc.dict_get(font, b"CharProcs").as_dict())
            .map(|procs| procs.values().filter_map(Object::as_reference).collect())
            .unwrap_or_default();
        let mut keeps = false;
        for proc in procs {
            if self.keeps(doc, page, area, proc, Hidden::Glyphs, own, 0)? {
                keeps = true;
                break;
            }
        }
        if let Some(id) = id {
            self.hidden.insert((id, Hidden::Glyphs), keeps);
        }
        Ok(keeps)
    }

    /// Whether the content of stream `id` (a pattern cell, a group, a glyph
    /// procedure) shows what apply cannot rewrite; a stream that cannot be
    /// read, or nesting past `MAX_HIDDEN_DEPTH`, does.
    #[allow(clippy::too_many_arguments)]
    fn keeps(
        &mut self,
        doc: &Document<'static>,
        page: usize,
        area: [f64; 4],
        id: ObjectId,
        what: Hidden,
        resources: Option<&Dictionary>,
        depth: usize,
    ) -> Result<bool, ApplyError> {
        if let Some(&known) = self.hidden.get(&(id, what)) {
            return Ok(known);
        }
        if depth > MAX_HIDDEN_DEPTH {
            return Ok(true);
        }
        // While it is read, a cycle back to it keeps.
        self.hidden.insert((id, what), true);
        let mut walker = Walker::new(doc, None, page, Detail::Full, Limits::DEFAULT, area);
        let walked = walker.form(id, appearance::IDENTITY, resources);
        let found = walker.finish();
        self.hidden_tokens = self.hidden_tokens.saturating_add(found.tokens);
        if self.hidden_tokens > MAX_HIDDEN_TOKENS {
            return Err(ApplyError::TooLarge(
                "the patterns, soft masks and Type 3 fonts under the marks are too complex to check"
                    .into(),
            ));
        }
        let mut keeps = walked.is_err();
        for draw in &found.draws {
            if keeps {
                break;
            }
            keeps = match &draw.kind {
                DrawKind::Text(_) => true,
                // Type 3 glyphs are often bitmaps: stencil masks.
                DrawKind::Image { .. } => !(what == Hidden::Glyphs && draw.paint.stencil),
                DrawKind::Shading => !matches!(draw.paint.shading, 2 | 3),
                DrawKind::Path { .. } | DrawKind::Form { .. } => false,
            };
            for pattern in [draw.paint.fill, draw.paint.stroke].into_iter().flatten() {
                keeps = keeps || self.pattern_keeps(doc, page, area, pattern, depth + 1)?;
            }
            if let Some(group) = draw.paint.soft_mask.filter(|_| !keeps) {
                keeps = self.keeps(doc, page, area, group, Hidden::Group, None, depth + 1)?;
            }
        }
        self.hidden.insert((id, what), keeps);
        Ok(keeps)
    }

    /// Records the alternate texts around a redacted draw: its sequences
    /// and MCIDs, and those of the Form draws it is inside.
    fn note(
        &mut self,
        content: &Content,
        page: usize,
        page_id: ObjectId,
        index: usize,
        base: &[usize],
    ) -> Result<(), ApplyError> {
        let mut at = Some(index);
        for _ in 0..=Limits::DEFAULT.depth + 1 {
            let Some(draw) = at.and_then(|i| content.draws.get(i)) else {
                break;
            };
            let mut sequence = draw.sequence;
            while let Some(s) = sequence {
                // Its enclosing sequences were noted with it.
                if !self.noted.insert((page, s)) {
                    break;
                }
                let Some(seq) = content.sequences.get(s) else {
                    break;
                };
                match &seq.name {
                    Some(name) => {
                        self.named.insert((page, seq.owner, name.clone()));
                    }
                    None => {
                        let key = match seq.form {
                            None => {
                                let start = match seq.from {
                                    Some((e, o)) => base.get(e).map(|b| b + o),
                                    None => base.get(seq.element).map(|b| b + seq.span.start),
                                };
                                let end = base.get(seq.element).map(|b| b + seq.span.end);
                                start.zip(end).map(|(s, e)| (Target::Page(page), s, e))
                            }
                            Some(_) => {
                                Some((Target::Form(seq.stream), seq.span.start, seq.span.end))
                            }
                        };
                        if let Some(key) = key {
                            self.add(key, OpEdit::Alt)?;
                        }
                    }
                }
                sequence = seq.parent;
            }
            if let Some(mcid) = draw.mcid {
                let owner = match draw.parent {
                    None => StructOwner::Page(page_id),
                    Some(_) => StructOwner::Form(draw.stream),
                };
                self.mcids.insert((owner, mcid));
            }
            at = draw.parent;
        }
        Ok(())
    }
}

/// Blanks the comments of content `bytes` (outside strings and the
/// `opaque` inline image spans) with spaces, so offsets keep.
pub fn blank_comments(bytes: &mut [u8], opaque: &[Range<usize>]) {
    let mut opaque: Vec<Range<usize>> = opaque.to_vec();
    opaque.sort_by_key(|r| r.start);
    let mut k = 0;
    let mut i = 0;
    let len = bytes.len();
    while i < len {
        while opaque.get(k).is_some_and(|r| r.end <= i) {
            k += 1;
        }
        if let Some(r) = opaque.get(k).filter(|r| r.start <= i) {
            i = r.end;
            continue;
        }
        match bytes.get(i).copied() {
            Some(b'(') => {
                let mut depth = 0usize;
                while let Some(&b) = bytes.get(i) {
                    i += 1;
                    match b {
                        b'\\' => i += 1,
                        b'(' => depth += 1,
                        b')' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some(b'<') if bytes.get(i + 1) == Some(&b'<') => i += 2,
            Some(b'<') => {
                while bytes.get(i).is_some_and(|b| *b != b'>') {
                    i += 1;
                }
                i += 1;
            }
            Some(b'%') => {
                while let Some(b) = bytes.get_mut(i) {
                    if matches!(*b, b'\n' | b'\r') {
                        break;
                    }
                    *b = b' ';
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
}

/// The next token, without the edit scanner's page budget (what is read
/// back is one operation the walk already read).
fn lex(lexer: &mut Lexer<'_>) -> Result<Option<Token>, crate::app::AppError> {
    let mut scratch = 0;
    lexer.next(&mut scratch)
}

/// One operation read back from content: operator, operand tokens.
struct Operation {
    op: Vec<u8>,
    span: Range<usize>,
    numbers: Vec<f64>,
}

/// The operations of `bytes[range]`, each with its whole span.
fn operations(bytes: &[u8], range: Range<usize>) -> Option<Vec<Operation>> {
    let slice = bytes.get(range.clone())?;
    let mut lexer = Lexer::new(slice);
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut numbers = Vec::new();
    let mut depth = 0usize;
    while let Some(token) = lex(&mut lexer).ok()? {
        match token {
            Token::Operand(operand) => {
                start.get_or_insert(operand.span().start);
                if depth == 0 {
                    numbers.push(operand.number().unwrap_or(f64::NAN));
                }
            }
            Token::Open(_, span) => {
                start.get_or_insert(span.start);
                depth += 1;
            }
            Token::Close(..) => depth = depth.saturating_sub(1),
            Token::Op(span) => {
                let begin = start.take().unwrap_or(span.start);
                out.push(Operation {
                    op: slice.get(span.clone())?.to_vec(),
                    span: range.start + begin..range.start + span.end,
                    numbers: std::mem::take(&mut numbers),
                });
            }
        }
    }
    Some(out)
}

/// Hex string operand.
fn hex(bytes: &[u8], out: &mut Vec<u8>) {
    out.push(b'<');
    for b in bytes {
        out.extend_from_slice(format!("{b:02x}").as_bytes());
    }
    out.push(b'>');
}

/// A show without its removed glyphs: `[...] TJ` (after `T*`, or after
/// `aw Tw ac Tc T*`, for `'` and `"`), each run of removed glyphs and the
/// adjustments among them one adjustment. At size 0 an adjustment moves
/// nothing: the pen moves past a run that spacing advances in a `TJ` of
/// its own at size 1 (`pen`, the font's name), the array split around it.
fn show_replacement(
    bytes: &[u8],
    range: Range<usize>,
    text: &TextDraw,
    removed: &BTreeSet<usize>,
    whole: bool,
    pen: Option<&[u8]>,
) -> Result<Vec<u8>, ApplyError> {
    let bad = || ApplyError::Unsupported("a text operation could not be read back".into());
    let slice = bytes.get(range).ok_or_else(bad)?;
    let mut lexer = Lexer::new(slice);
    let mut top: Vec<Operand> = Vec::new();
    let mut elements: Vec<Option<Operand>> = Vec::new();
    let mut depth = 0usize;
    let mut op = Vec::new();
    while let Some(token) = lex(&mut lexer).map_err(|_| bad())? {
        match token {
            Token::Open(Container::Array, _) if depth == 0 => depth = 1,
            Token::Open(..) => {
                if depth == 1 {
                    elements.push(None);
                }
                depth += 1;
            }
            Token::Close(..) => depth = depth.saturating_sub(1),
            Token::Operand(operand) => match depth {
                0 => top.push(operand),
                1 => elements.push(Some(operand)),
                _ => {}
            },
            Token::Op(span) => op = slice.get(span).unwrap_or(&[]).to_vec(),
        }
    }
    // The strings and adjustments shown, by operand or element index.
    let items: Vec<(usize, Option<&Operand>)> = match op.as_slice() {
        b"TJ" => elements
            .iter()
            .enumerate()
            .map(|(i, e)| (i, e.as_ref()))
            .collect(),
        b"\"" => vec![(2, top.get(2))],
        _ => vec![(0, top.first())],
    };
    let mut out: Vec<u8> = Vec::new();
    match op.as_slice() {
        b"'" => out.extend_from_slice(b"T* "),
        b"\"" => {
            for (index, operator) in [(0, "Tw"), (1, "Tc")] {
                let value = top.get(index).and_then(Operand::number).unwrap_or(0.0);
                out.extend_from_slice(
                    format!("{} {operator} ", geometry::number(value)).as_bytes(),
                );
            }
            out.extend_from_slice(b"T* ");
        }
        _ => {}
    }
    let size = text.size;
    let mut elems: Vec<u8> = Vec::new();
    let mut buffer: Vec<u8> = Vec::new();
    let mut pending: Option<f64> = None;
    let flush = |buffer: &mut Vec<u8>, elems: &mut Vec<u8>| {
        if !buffer.is_empty() {
            hex(buffer, elems);
            buffer.clear();
        }
    };
    let number = |value: f64, elems: &mut Vec<u8>| {
        if !elems.is_empty() {
            elems.push(b' ');
        }
        elems.extend_from_slice(geometry::number(value).as_bytes());
        elems.push(b' ');
    };
    // At size 0: how far removed glyphs moved the pen, not yet written.
    let mut moved = 0.0f64;
    let place = |elems: &mut Vec<u8>, out: &mut Vec<u8>, moved: &mut f64| {
        let Some(name) = pen.filter(|_| *moved != 0.0) else {
            return;
        };
        let name = crate::app::ops::edit::plan::name_source(name);
        out.push(b'[');
        out.extend_from_slice(elems);
        out.extend_from_slice(b"] TJ ");
        let step = geometry::number(-*moved * 1000.0);
        out.extend_from_slice(format!("{name} 1 Tf [{step}] TJ {name} 0 Tf ").as_bytes());
        elems.clear();
        *moved = 0.0;
    };
    // Glyphs come in operand order: one cursor walks them.
    let mut cursor = 0usize;
    for (index, item) in items {
        match item {
            Some(Operand::Num(value, _)) => {
                flush(&mut buffer, &mut elems);
                pending = Some(pending.unwrap_or(0.0) + value);
            }
            Some(Operand::Str(string, _)) if !whole => {
                while text.glyphs.get(cursor).is_some_and(|g| g.operand < index) {
                    cursor += 1;
                }
                let first = cursor;
                while text.glyphs.get(cursor).is_some_and(|g| g.operand == index) {
                    cursor += 1;
                }
                let glyphs = text.glyphs.get(first..cursor).unwrap_or(&[]);
                for (g, glyph) in (first..).zip(glyphs) {
                    if removed.contains(&g) {
                        flush(&mut buffer, &mut elems);
                        if glyph.advance != 0.0 {
                            if !size.is_finite() || (size == 0.0 && pen.is_none()) {
                                return Err(ApplyError::Unsupported(
                                    "text at size 0 moves the pen past a removed glyph".into(),
                                ));
                            }
                            match size == 0.0 {
                                true => moved += glyph.advance,
                                false => {
                                    pending = Some(
                                        pending.unwrap_or(0.0) - glyph.advance * 1000.0 / size,
                                    );
                                }
                            }
                        } else {
                            pending.get_or_insert(0.0);
                        }
                    } else {
                        place(&mut elems, &mut out, &mut moved);
                        if let Some(value) = pending.take() {
                            number(value, &mut elems);
                        }
                        buffer.extend_from_slice(string.get(glyph.bytes.clone()).unwrap_or(&[]));
                    }
                }
            }
            _ => {}
        }
    }
    flush(&mut buffer, &mut elems);
    if whole {
        // Where the pen ends is not known either way: nothing is shown.
        out.push(b' ');
        return Ok(out);
    }
    if let Some(value) = pending.filter(|v| *v != 0.0) {
        number(value, &mut elems);
    }
    place(&mut elems, &mut out, &mut moved);
    out.push(b'[');
    out.extend_from_slice(&elems);
    out.extend_from_slice(b"] TJ");
    Ok(out)
}

/// Splices for a path: its construction operators go and its paint becomes
/// the cut path's.
#[allow(clippy::too_many_arguments)]
fn path_splices(
    bytes: &[u8],
    range: Range<usize>,
    fill: bool,
    stroke: bool,
    clips: bool,
    regions: &[Region],
    tolerance: f64,
    remove: bool,
    budget: &mut Budget,
) -> Result<Vec<Splice>, ApplyError> {
    let ops = operations(bytes, range)
        .ok_or_else(|| ApplyError::Unsupported("a path could not be read back".into()))?;
    let Some((paint, construction)) = ops.split_last() else {
        return Ok(Vec::new());
    };
    let is_construction = |op: &[u8]| {
        matches!(
            op,
            b"m" | b"l" | b"c" | b"v" | b"y" | b"re" | b"h" | b"W" | b"W*"
        )
    };
    let even_odd_clip = construction.iter().any(|o| o.op == b"W*");
    let clip_op: &[u8] = if even_odd_clip { b"W* n\n" } else { b"W n\n" };
    let parsed = geometry::parse(
        &(construction.iter())
            .map(|o| (o.op.clone(), o.numbers.clone()))
            .collect::<Vec<_>>(),
        matches!(paint.op.as_slice(), b"s" | b"b" | b"b*"),
    );
    let empty_clip = |out: &mut Vec<u8>| {
        if clips {
            out.extend_from_slice(b"0 0 m ");
            out.extend_from_slice(clip_op);
        }
    };
    let mut replacement = Vec::new();
    match parsed {
        Parsed::Path(subpaths) if !remove => {
            let mut pieces: Vec<(Vec<geometry::Subpath>, &[u8])> = Vec::new();
            let fill_op: &[u8] = match paint.op.as_slice() {
                b"f*" | b"B*" | b"b*" => b"f*\n",
                _ => b"f\n",
            };
            let mut cut = |usage: Use| {
                geometry::cut(&subpaths, regions, usage, tolerance, budget).ok_or_else(|| {
                    ApplyError::TooLarge("the paths under the marks are too complex".into())
                })
            };
            if fill {
                pieces.push((cut(Use::Area)?, fill_op));
            }
            if stroke {
                pieces.push((cut(Use::Lines)?, b"S\n"));
            }
            let clip = match clips {
                true => Some(cut(Use::Area)?),
                false => None,
            };
            let sane = pieces
                .iter()
                .all(|(p, _)| geometry::within(p, MAX_COORDINATE))
                && clip
                    .as_ref()
                    .is_none_or(|c| geometry::within(c, MAX_COORDINATE));
            if !sane {
                empty_clip(&mut replacement);
            } else {
                for (subpaths, op) in pieces {
                    if !subpaths.is_empty() {
                        geometry::write(&subpaths, &mut replacement);
                        replacement.extend_from_slice(op);
                    }
                }
                match clip {
                    Some(clip) if !clip.is_empty() => {
                        geometry::write(&clip, &mut replacement);
                        replacement.extend_from_slice(clip_op);
                    }
                    Some(_) => empty_clip(&mut replacement),
                    None => {}
                }
            }
        }
        _ => empty_clip(&mut replacement),
    }
    let mut out: Vec<Splice> = construction
        .iter()
        .filter(|o| is_construction(&o.op))
        .map(|o| (o.span.clone(), b" ".to_vec()))
        .collect();
    let mut paint_text = b" ".to_vec();
    paint_text.extend_from_slice(&replacement);
    out.push((paint.span.clone(), paint_text));
    Ok(out)
}

/// A path for a region's outline (for clips).
fn region_path(region: &Region, out: &mut Vec<u8>) {
    let [a, b, c, d] = region.corners;
    let subpath = geometry::Subpath {
        start: a,
        segments: vec![
            geometry::Segment::Line(b),
            geometry::Segment::Line(c),
            geometry::Segment::Line(d),
        ],
        closed: true,
    };
    geometry::write(&[subpath], out);
}

/// `sh` drawn with each region clipped out (an even-odd clip of the page
/// around it per region); a mesh or function shading refuses.
fn shading_replacement(
    doc: &Document<'_>,
    page: &Dictionary,
    bytes: &[u8],
    range: Range<usize>,
    regions: &[Region],
    outer: Option<&Region>,
    owner: Owner,
) -> Result<Vec<u8>, ApplyError> {
    let ops = operations(bytes, range.clone())
        .ok_or_else(|| ApplyError::Unsupported("a shading could not be read back".into()))?;
    let name = (ops.last())
        .and_then(|o| bytes.get(o.span.clone()))
        .and_then(|text| {
            let mut lexer = Lexer::new(text);
            match lex(&mut lexer) {
                Ok(Some(Token::Operand(Operand::Name(n, _)))) => Some(n),
                _ => None,
            }
        });
    let kind = name
        .as_deref()
        .and_then(|n| {
            let shadings = doc
                .dict_get(resources(doc, page, owner)?, b"Shading")
                .as_dict()?;
            let value = doc.resolve(shadings.get(n)?);
            let dict = match value.as_dict() {
                Some(d) => d,
                None => doc.stream_dict(shadings.get(n)?.as_reference()?).ok()?,
            };
            doc.dict_get(dict, b"ShadingType").as_i64()
        })
        .unwrap_or(0);
    if !matches!(kind, 2 | 3) {
        return Err(ApplyError::Unsupported(
            "a mark covers part of a mesh or function shading".into(),
        ));
    }
    let Some(outer) = outer else {
        return Ok(Vec::new());
    };
    let mut out = b" q\n".to_vec();
    for region in regions {
        region_path(outer, &mut out);
        region_path(region, &mut out);
        out.extend_from_slice(b"W* n\n");
    }
    out.extend_from_slice(bytes.get(range).unwrap_or(&[]));
    out.extend_from_slice(b"\nQ ");
    Ok(out)
}

/// The inline image entries of `BI ... ID` and its data range.
fn inline_parts(bytes: &[u8], range: Range<usize>) -> Option<(images::Inline<'_>, Range<usize>)> {
    let slice = bytes.get(range.clone())?;
    let mut lexer = Lexer::new(slice);
    match lex(&mut lexer).ok()?? {
        Token::Op(span) if slice.get(span.clone()) == Some(b"BI") => {}
        _ => return None,
    }
    let mut entries = Vec::new();
    let mut key: Option<Vec<u8>> = None;
    loop {
        let token = lex(&mut lexer).ok()??;
        match token {
            Token::Op(span) => {
                if slice.get(span.clone()) != Some(b"ID") {
                    return None;
                }
                let data = span.end + 1;
                let end = slice.len().checked_sub(2)?.max(data);
                return Some((
                    images::Inline { entries },
                    range.start + data..range.start + end,
                ));
            }
            Token::Operand(Operand::Name(name, _)) if key.is_none() => key = Some(name),
            Token::Operand(operand) => {
                let key = key.take()?;
                let span = operand.span();
                entries.push((
                    key,
                    images::InlineValue {
                        source: slice.get(span)?,
                        value: operand_object(&operand, slice),
                    },
                ));
            }
            Token::Open(_, open) => {
                let key = key.take()?;
                let mut depth = 1usize;
                let mut items = Vec::new();
                let mut end = open.end;
                while depth > 0 {
                    match lex(&mut lexer).ok()?? {
                        Token::Open(..) => depth += 1,
                        Token::Close(_, span) => {
                            depth -= 1;
                            end = span.end;
                        }
                        Token::Operand(operand) if depth == 1 => {
                            items.push(operand_object(&operand, slice));
                        }
                        Token::Op(_) => return None,
                        Token::Operand(_) => {}
                    }
                }
                entries.push((
                    key,
                    images::InlineValue {
                        source: slice.get(open.start..end)?,
                        value: Object::Array(items),
                    },
                ));
            }
            Token::Close(..) => return None,
        }
    }
}

fn operand_object(operand: &Operand, slice: &[u8]) -> Object {
    match operand {
        Operand::Num(n, _) if n.fract() == 0.0 && n.abs() < 9e15 => Object::Integer(*n as i64),
        Operand::Num(n, _) => Object::Real(*n),
        Operand::Name(n, _) => Object::Name(n.clone()),
        Operand::Str(s, _) => Object::String(s.clone()),
        Operand::Word(span) => match slice.get(span.clone()) {
            Some(b"true") => Object::Boolean(true),
            Some(b"false") => Object::Boolean(false),
            _ => Object::Null,
        },
        _ => Object::Null,
    }
}

/// An inline image with the pixels under `quads` blanked.
#[allow(clippy::too_many_arguments)]
fn inline_replacement(
    doc: &Document<'_>,
    page: &Dictionary,
    index: usize,
    bytes: &[u8],
    range: Range<usize>,
    quads: &[[Point; 4]],
    owner: Owner,
    room: &mut u64,
) -> Result<Vec<u8>, ApplyError> {
    let (image, data) = inline_parts(bytes, range)
        .ok_or_else(|| ApplyError::Unsupported("an inline image could not be read back".into()))?;
    let named = |name: &[u8]| {
        let spaces = doc
            .dict_get(resources(doc, page, owner)?, b"ColorSpace")
            .as_dict()?;
        images::components(doc, spaces.get(name)?)
    };
    let error = |e| image_error(e, index);
    let (mut raster, mask) =
        images::read_inline(&image, bytes.get(data).unwrap_or(&[]), named, room).map_err(error)?;
    let value = match mask {
        true => {
            let decode = image
                .entries
                .iter()
                .find(|(k, _)| k == b"Decode" || k == b"D")
                .and_then(|(_, v)| v.value.as_array().and_then(|a| a.first()?.as_f64()));
            if decode == Some(1.0) { 0 } else { 1 }
        }
        false => 0,
    };
    let regions = images::pixel_regions(quads, raster.width, raster.height);
    images::blank(&mut raster, &regions, value).map_err(error)?;
    let mut out = b" ".to_vec();
    out.extend(images::write_inline(&image, &raster));
    out.push(b' ');
    Ok(out)
}

/// An image error on page `page` (0-based).
fn image_error(error: ImageError, page: usize) -> ApplyError {
    match error {
        ImageError::Pixels => ApplyError::TooLarge(format!(
            "an image on page {} exceeds {} pixels",
            page + 1,
            images::MAX_IMAGE_PIXELS
        )),
        ImageError::TooLarge(why) => ApplyError::TooLarge(why),
        ImageError::Unsupported(why) => ApplyError::Unsupported(why),
    }
}

/// Splices deleting /ActualText, /Alt and /E from a `BDC`'s inline
/// property list.
fn alt_splices(bytes: &[u8], range: Range<usize>) -> Vec<Splice> {
    let Some(slice) = bytes.get(range.clone()) else {
        return Vec::new();
    };
    let mut lexer = Lexer::new(slice);
    let mut out = Vec::new();
    let mut depth = 0usize;
    // The entry being read: its key's start and whether it goes.
    let mut key: Option<(usize, bool)> = None;
    // A container value opened at this depth.
    let mut container: Option<usize> = None;
    let mut finish = |key: &mut Option<(usize, bool)>, end: usize| {
        if let Some((start, true)) = key.take() {
            out.push((range.start + start..range.start + end, b" ".to_vec()));
        }
    };
    while let Ok(Some(token)) = lex(&mut lexer) {
        match token {
            Token::Open(kind, _) => {
                if depth == 0 && kind != Container::Dict {
                    break;
                }
                if depth == 1 && key.is_some() && container.is_none() {
                    container = Some(depth);
                }
                depth += 1;
            }
            Token::Close(_, span) => {
                depth = depth.saturating_sub(1);
                if container == Some(depth) {
                    container = None;
                    finish(&mut key, span.end);
                }
                if depth == 0 {
                    break;
                }
            }
            Token::Operand(operand) if depth == 1 && container.is_none() => match key {
                None => {
                    if let Operand::Name(name, span) = &operand {
                        key = Some((span.start, STRUCT_KEYS.contains(&name.as_slice())));
                    }
                }
                Some(_) => finish(&mut key, operand.span().end),
            },
            _ => {}
        }
    }
    out
}

/// Applies `splices` (range, replacement) to `bytes`; overlapping ones
/// refuse.
fn splice(bytes: &[u8], mut splices: Vec<Splice>) -> Result<Vec<u8>, ApplyError> {
    splices.sort_by_key(|(r, _)| (r.start, r.end));
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0usize;
    for (range, replacement) in splices {
        if range.start < at || range.end < range.start || range.end > bytes.len() {
            return Err(ApplyError::Unsupported(
                "content operations overlap where the marks meet them".into(),
            ));
        }
        out.extend_from_slice(bytes.get(at..range.start).unwrap_or(&[]));
        out.extend_from_slice(&replacement);
        at = range.end;
    }
    out.extend_from_slice(bytes.get(at..).unwrap_or(&[]));
    Ok(out)
}

/// The splices of every edit of one stream.
#[allow(clippy::too_many_arguments)]
fn stream_splices(
    doc: &Document<'_>,
    page_dicts: &BTreeMap<usize, &Dictionary>,
    bytes: &[u8],
    edits: &[(usize, usize, &OpEdit)],
    budget: &mut Budget,
    room: &mut u64,
) -> Result<Vec<Splice>, ApplyError> {
    let mut out = Vec::new();
    for &(start, end, edit) in edits {
        let range = start..end;
        match edit {
            OpEdit::Show {
                text,
                removed,
                whole,
                pen,
            } => out.push((
                range.clone(),
                show_replacement(bytes, range, text, removed, *whole, pen.as_deref())?,
            )),
            OpEdit::Path {
                fill,
                stroke,
                clips,
                regions,
                tolerance,
                remove,
            } => out.extend(path_splices(
                bytes, range, *fill, *stroke, *clips, regions, *tolerance, *remove, budget,
            )?),
            OpEdit::Remove => out.push((range, b" ".to_vec())),
            OpEdit::Inline {
                quads,
                remove,
                resources,
                page,
            } => {
                let replacement = match remove {
                    true => b" ".to_vec(),
                    false => {
                        let dict = page_dicts
                            .get(page)
                            .copied()
                            .ok_or_else(|| ApplyError::Write("an edited page is gone".into()))?;
                        inline_replacement(
                            doc,
                            dict,
                            *page,
                            bytes,
                            range.clone(),
                            quads,
                            *resources,
                            room,
                        )?
                    }
                };
                out.push((range, replacement));
            }
            OpEdit::Shading {
                regions,
                outer,
                resources,
                page,
            } => {
                let page = page_dicts
                    .get(page)
                    .copied()
                    .ok_or_else(|| ApplyError::Write("an edited page is gone".into()))?;
                let replacement = shading_replacement(
                    doc,
                    page,
                    bytes,
                    range.clone(),
                    regions,
                    outer.as_ref(),
                    *resources,
                )?;
                out.push((range, replacement));
            }
            OpEdit::Alt => out.extend(alt_splices(bytes, range)),
        }
    }
    Ok(out)
}

/// A rewritten Form's dictionary for Flate-compressed new bytes, without
/// derived copies of its content.
fn flate_dict(mut dict: Dictionary) -> Dictionary {
    for key in [b"Filter".as_slice(), b"DecodeParms", b"DL", b"Length"]
        .into_iter()
        .chain(STREAM_DERIVED)
    {
        dict.remove(key);
    }
    dict.insert(b"Filter".to_vec(), Object::name("FlateDecode"));
    dict
}

/// Redacts every marked page of `target` (a scratch copy) in place.
pub fn apply(
    target: &mut Builder,
    view: Option<&ViewDocument>,
    marks: &[Mark],
) -> Result<Applied, ApplyError> {
    let mut planner = Planner::default();
    {
        let doc = target.document();
        let rule = GlyphRule::new(doc, marks);
        let pages: BTreeSet<usize> = marks.iter().map(|m| m.page).collect();
        for &page in &pages {
            planner.plan_page(doc, view, &rule, marks, page)?;
        }
        planner.applied.pages = pages.into_iter().collect();
    }
    let mut budget = Budget(MAX_PATH_WORK);
    let mut room = images::MAX_TOTAL_PIXELS;
    // Forms, rewritten in place.
    let mut by_target: BTreeMap<Target, Vec<(usize, usize, OpEdit)>> = BTreeMap::new();
    for ((target_key, start, end), edit) in std::mem::take(&mut planner.ops) {
        by_target
            .entry(target_key)
            .or_default()
            .push((start, end, edit));
    }
    for (target_key, edits) in &by_target {
        let Target::Form(id) = target_key else {
            continue;
        };
        let Some(bytes) = planner.forms.get(id) else {
            continue;
        };
        let mut bytes = (**bytes).clone();
        let opaque: Vec<Range<usize>> = (planner.form_opaque.get(id).into_iter().flatten())
            .map(|&(s, e)| s..e)
            .collect();
        blank_comments(&mut bytes, &opaque);
        let doc = target.document();
        let page_dicts: BTreeMap<usize, &Dictionary> = (info::pages(doc).into_iter())
            .enumerate()
            .filter(|(i, _)| planner.applied.pages.contains(i))
            .map(|(i, p)| (i, p.dict))
            .collect();
        let refs: Vec<(usize, usize, &OpEdit)> =
            edits.iter().map(|(s, e, x)| (*s, *e, x)).collect();
        let splices = stream_splices(doc, &page_dicts, &bytes, &refs, &mut budget, &mut room)?;
        let out = splice(&bytes, splices)?;
        let dict = flate_dict(doc.stream_dict(*id)?.clone());
        let data = miniz_oxide::deflate::compress_to_vec_zlib(&out, 6);
        target.put_stream_raw(*id, dict, data)?;
    }
    // Pages: one new content stream each.
    for plan in std::mem::take(&mut planner.pages) {
        let mut joined = plan.joined;
        blank_comments(&mut joined, &plan.opaque);
        let doc = target.document();
        let page_dicts: BTreeMap<usize, &Dictionary> = (info::pages(doc).into_iter())
            .enumerate()
            .filter(|(i, _)| *i == plan.page)
            .map(|(i, p)| (i, p.dict))
            .collect();
        let refs: Vec<(usize, usize, &OpEdit)> = (by_target.get(&Target::Page(plan.page)))
            .into_iter()
            .flatten()
            .map(|(s, e, x)| (*s, *e, x))
            .collect();
        let mut splices = stream_splices(doc, &page_dicts, &joined, &refs, &mut budget, &mut room)?;
        splices.extend(plan.excess.iter().map(|r| (r.clone(), b" ".to_vec())));
        let body = splice(&joined, splices)?;
        let mut out = b"q\n".to_vec();
        out.extend_from_slice(&body);
        out.push(b'\n');
        if plan.open_text {
            out.extend_from_slice(b"ET\n");
        }
        for _ in 0..plan.open_marked {
            out.extend_from_slice(b"EMC\n");
        }
        for _ in 0..=plan.open_saves {
            out.extend_from_slice(b"Q\n");
        }
        // The black boxes, in user space.
        out.extend_from_slice(b"q 0 g\n");
        for rect in &plan.rects {
            let corners = [
                (rect[0], rect[1]),
                (rect[2], rect[1]),
                (rect[2], rect[3]),
                (rect[0], rect[3]),
            ]
            .map(|p| plan.space.to_user(p));
            let box_ = geometry::Subpath {
                start: corners[0],
                segments: corners[1..]
                    .iter()
                    .map(|p| geometry::Segment::Line(*p))
                    .collect(),
                closed: true,
            };
            geometry::write(&[box_], &mut out);
        }
        out.extend_from_slice(b"f\nQ\n");
        let mut dict = Dictionary::new();
        dict.insert(b"Filter".to_vec(), Object::name("FlateDecode"));
        let data = miniz_oxide::deflate::compress_to_vec_zlib(&out, 6);
        let stream = target.allocate(Object::Null)?;
        target.put_stream_raw(stream, dict, data)?;
        target.update(plan.id, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"Contents".to_vec(), Object::Reference(stream));
                for key in PAGE_DERIVED {
                    dict.remove(key);
                }
            }
        })?;
    }
    redact_images(
        target,
        view,
        &planner.images,
        &planner.image_pages,
        &mut room,
        &mut planner.applied,
    )?;
    clean_named(target, &planner.named)?;
    clean_structure(target, &planner.mcids)?;
    planner.applied.annotations = remove_annotations(target, marks)?;
    Ok(planner.applied)
}

/// Blanks the pixels under the marks of each image object, and of its soft
/// mask and mask (which share its unit square) in copies the image points
/// to: another image may draw them too.
fn redact_images(
    target: &mut Builder,
    view: Option<&ViewDocument>,
    found: &BTreeMap<ObjectId, Vec<[Point; 4]>>,
    pages: &BTreeMap<ObjectId, usize>,
    room: &mut u64,
    applied: &mut Applied,
) -> Result<(), ApplyError> {
    for (id, quads) in found {
        let page = pages.get(id).copied().unwrap_or(0);
        let error = |e| image_error(e, page);
        let doc = target.document();
        let decoded = images::read(doc, view, *id, room).map_err(error)?;
        let images::Decoded {
            mut raster,
            mut dict,
            alpha,
        } = decoded;
        let mask = doc.dict_get(&dict, b"ImageMask").as_bool() == Some(true);
        let value = images::blank_value(doc, &dict, mask);
        let regions = images::pixel_regions(quads, raster.width, raster.height);
        let blanked = images::blank(&mut raster, &regions, value).map_err(error)?;
        applied.images += 1;
        let whole = blanked >= (raster.width as u64).saturating_mul(raster.height as u64);
        if whole {
            raster = images::single_pixel(&raster, value);
            dict.insert(b"Width".to_vec(), Object::Integer(1));
            dict.insert(b"Height".to_vec(), Object::Integer(1));
            dict.remove(b"SMask".as_slice());
            dict.remove(b"Mask".as_slice());
        } else if let Some(mut alpha) = alpha {
            let regions = images::pixel_regions(quads, alpha.width, alpha.height);
            images::blank(&mut alpha, &regions, 0).map_err(error)?;
            let mut alpha_dict = Dictionary::new();
            alpha_dict.insert(b"Type".to_vec(), Object::name("XObject"));
            alpha_dict.insert(b"Subtype".to_vec(), Object::name("Image"));
            alpha_dict.insert(b"Width".to_vec(), Object::Integer(alpha.width as i64));
            alpha_dict.insert(b"Height".to_vec(), Object::Integer(alpha.height as i64));
            alpha_dict.insert(b"ColorSpace".to_vec(), Object::name("DeviceGray"));
            alpha_dict.insert(
                b"BitsPerComponent".to_vec(),
                Object::Integer(i64::from(alpha.bits)),
            );
            let data = images::encode(&alpha, &mut alpha_dict);
            let smask = target.allocate(Object::Null)?;
            target.put_stream_raw(smask, alpha_dict, data)?;
            dict.insert(b"SMask".to_vec(), Object::Reference(smask));
        } else {
            for key in [b"SMask".as_slice(), b"Mask"] {
                let Some(mask) = dict.get(key).and_then(Object::as_reference) else {
                    continue;
                };
                if target.document().stream_dict(mask).is_err() {
                    continue;
                }
                let copy = blanked_copy(target, view, mask, quads, room, page)?;
                dict.insert(key.to_vec(), Object::Reference(copy));
            }
        }
        let data = images::encode(&raster, &mut dict);
        target.put_stream_raw(*id, dict, data)?;
    }
    Ok(())
}

/// A new image object: mask `id` with the pixels under `quads` blanked.
fn blanked_copy(
    target: &mut Builder,
    view: Option<&ViewDocument>,
    id: ObjectId,
    quads: &[[Point; 4]],
    room: &mut u64,
    page: usize,
) -> Result<ObjectId, ApplyError> {
    let error = |e| image_error(e, page);
    let doc = target.document();
    let images::Decoded {
        mut raster,
        mut dict,
        ..
    } = images::read(doc, view, id, room).map_err(error)?;
    let stencil = doc.dict_get(&dict, b"ImageMask").as_bool() == Some(true);
    let value = images::blank_value(doc, &dict, stencil);
    let regions = images::pixel_regions(quads, raster.width, raster.height);
    images::blank(&mut raster, &regions, value).map_err(error)?;
    let data = images::encode(&raster, &mut dict);
    let copy = target.allocate(Object::Null)?;
    target.put_stream_raw(copy, dict, data)?;
    Ok(copy)
}

/// Where page `page`'s /Resources live.
fn page_resources_place(doc: &Document<'_>, page: usize) -> Option<Place> {
    let pages = info::pages(doc);
    let page_ref = pages.get(page)?;
    super::inventory::page_resources(doc, page_ref.id, page_ref.dict)
}

/// Drops the alternate texts of named property lists around redacted
/// content.
fn clean_named(
    target: &mut Builder,
    named: &BTreeSet<(usize, Owner, Vec<u8>)>,
) -> Result<(), ApplyError> {
    for (page, owner, name) in named {
        let doc = target.document();
        let resources = match owner {
            Owner::Page => page_resources_place(doc, *page),
            Owner::Form(id) => Some(Place::object(*id).child(doc, b"Resources")),
        };
        let Some(resources) = resources else {
            return Err(ApplyError::Unsupported(
                "a marked-content property list's resources cannot be found".into(),
            ));
        };
        let place = resources.child(doc, b"Properties").child(doc, name);
        if place.read(doc).as_dict().is_none() {
            continue;
        }
        place.update(target, |value| {
            if let Some(dict) = value.as_dict_mut() {
                for key in STRUCT_KEYS {
                    dict.remove(key);
                }
            }
        })?;
    }
    Ok(())
}

/// The ParentTree's entries by key (/Nums, then /Kids), at most
/// `MAX_PARENT_TREE` nodes and entries read.
fn parent_tree<'d>(doc: &'d Document<'_>) -> BTreeMap<i64, &'d [Object]> {
    let mut out = BTreeMap::new();
    let tree = (doc.resolve_id(doc.root()).as_dict())
        .and_then(|c| doc.dict_get(c, b"StructTreeRoot").as_dict())
        .and_then(|t| doc.dict_get(t, b"ParentTree").as_dict());
    let mut stack: Vec<&Dictionary> = tree.into_iter().collect();
    let mut seen: BTreeSet<usize> = BTreeSet::new();
    let mut work = 0usize;
    while let Some(node) = stack.pop() {
        work += 1;
        if work > MAX_PARENT_TREE || !seen.insert(std::ptr::from_ref(node).addr()) {
            continue;
        }
        if let Some(nums) = doc.dict_get(node, b"Nums").as_array() {
            for pair in nums.chunks_exact(2) {
                work += 1;
                let (Some(key), Some(value)) = (pair.first(), pair.get(1)) else {
                    continue;
                };
                let key = doc.resolve(key).as_f64().filter(|k| k.is_finite());
                if let (Some(key), Some(entry)) = (key, doc.resolve(value).as_array()) {
                    out.entry(key.trunc() as i64).or_insert(entry);
                }
            }
        }
        if let Some(kids) = doc.dict_get(node, b"Kids").as_array() {
            stack.extend(kids.iter().rev().filter_map(|k| doc.resolve(k).as_dict()));
        }
    }
    out
}

/// Drops the alternate texts of the structure elements of redacted MCIDs
/// and of their ancestors, and the title (/T) of the elements themselves
/// (MuPDF's redaction drops /T, /E, /ActualText and /Alt of the element; a
/// title naming the redacted figure or table would stay).
fn clean_structure(
    target: &mut Builder,
    mcids: &BTreeSet<(StructOwner, i64)>,
) -> Result<(), ApplyError> {
    let mut elements: BTreeSet<ObjectId> = BTreeSet::new();
    let mut own: BTreeSet<ObjectId> = BTreeSet::new();
    if !mcids.is_empty() {
        let doc = target.document();
        let tree = parent_tree(doc);
        for (owner, mcid) in mcids {
            let holder = match owner {
                StructOwner::Page(id) => doc.resolve_id(*id).as_dict(),
                StructOwner::Form(id) => doc.stream_dict(*id).ok(),
            };
            let key = holder
                .and_then(|h| doc.dict_get(h, b"StructParents").as_f64())
                .filter(|k| k.is_finite());
            let Some(entry) = key.and_then(|k| tree.get(&(k.trunc() as i64))) else {
                continue;
            };
            let Some((Some(mut id), _)) = struct_element_in(doc, entry, *mcid) else {
                continue;
            };
            let root = (doc.resolve_id(id).as_dict())
                .is_some_and(|d| doc.dict_get(d, b"Type").as_name() == Some(b"StructTreeRoot"));
            if !root {
                own.insert(id);
            }
            for _ in 0..MAX_STRUCT_DEPTH {
                let Some(dict) = doc.resolve_id(id).as_dict() else {
                    break;
                };
                if doc.dict_get(dict, b"Type").as_name() == Some(b"StructTreeRoot")
                    || !elements.insert(id)
                {
                    break;
                }
                match dict.get(b"P".as_slice()).and_then(Object::as_reference) {
                    Some(parent) => id = parent,
                    None => break,
                }
            }
        }
    }
    for id in elements {
        let title: &[u8] = b"T";
        let keys: Vec<&[u8]> = match own.contains(&id) {
            true => STRUCT_KEYS.iter().copied().chain([title]).collect(),
            false => STRUCT_KEYS.to_vec(),
        };
        let has = (target.document().resolve_id(id).as_dict())
            .is_some_and(|d| keys.iter().any(|k| d.get(*k).is_some()));
        if has {
            target.update(id, |value| {
                if let Some(dict) = value.as_dict_mut() {
                    for key in &keys {
                        dict.remove(*key);
                    }
                }
            })?;
        }
    }
    Ok(())
}

/// Removes every mark and every annotation a mark touches (Widgets with
/// their fields). Returns how many annotations other than marks went.
fn remove_annotations(target: &mut Builder, marks: &[Mark]) -> Result<usize, ApplyError> {
    let regions = Regions::of(marks);
    let (touched, direct, page_ids) = {
        let doc = target.document();
        let touched = region::annotations(doc, &regions);
        let pages = info::pages(doc);
        // Direct dictionaries in /Annots: marks and touched ones, by index.
        let mut direct: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        for page in regions.pages() {
            let Some(page_ref) = pages.get(page) else {
                continue;
            };
            let space = PageSpace::of(doc, page_ref.dict);
            let items = doc
                .dict_get(page_ref.dict, b"Annots")
                .as_array()
                .unwrap_or(&[]);
            for (index, item) in items.iter().enumerate() {
                let Object::Dictionary(dict) = item else {
                    continue;
                };
                let mark = doc.dict_get(dict, b"Subtype").as_name() == Some(b"Redact");
                let hit = (dict.get(b"Rect".as_slice()))
                    .and_then(|r| annots::rect(doc, r))
                    .is_some_and(|r| regions.touches(page, &space.rect_to_page(r)));
                if mark || hit {
                    direct.entry(page).or_default().insert(index);
                }
            }
        }
        let page_ids: BTreeMap<usize, Option<ObjectId>> =
            pages.iter().enumerate().map(|(i, p)| (i, p.id)).collect();
        (touched, direct, page_ids)
    };
    for (page, indices) in &direct {
        annots::edit_annots(target, *page, |items| {
            let mut index = 0;
            items.retain(|_| {
                let keep = !indices.contains(&index);
                index += 1;
                keep
            });
        })
        .map_err(|e| ApplyError::Write(e.to_string()))?;
    }
    let mut widgets = Vec::new();
    let mut others = Vec::new();
    for annot in &touched {
        let Some(id) = annot.id else {
            continue;
        };
        match (
            annot.subtype.as_slice(),
            page_ids.get(&annot.page).copied().flatten(),
        ) {
            (b"Widget", Some(page_id)) => widgets.push((page_id, id)),
            _ => others.push((annot.page, id)),
        }
    }
    let count = touched.len();
    for mark in marks {
        if let Some(id) = mark.id {
            others.push((mark.page, id));
        }
    }
    if !widgets.is_empty() {
        fields::remove_widgets(target, &widgets)?;
    }
    if !others.is_empty() {
        annots::remove_many(target, &others).map_err(|e| ApplyError::Write(e.to_string()))?;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_are_blanked_outside_strings_and_images() {
        let mut bytes = b"(a%b) Tj % secret\nBI ID %x EI <25>".to_vec();
        let opaque = vec![Range { start: 19, end: 30 }];
        blank_comments(&mut bytes, &opaque);
        assert_eq!(&bytes, b"(a%b) Tj         \nBI ID %x EI <25>");
    }

    #[test]
    fn alt_texts_leave_the_property_list() {
        let bytes = b"/Span <</MCID 3 /ActualText (secret) /Lang (en) /Alt <<>> >> BDC";
        let out = splice(bytes, alt_splices(bytes, 0..bytes.len())).unwrap();
        assert_eq!(&out, b"/Span <</MCID 3   /Lang (en)   >> BDC");
    }
}
