// SPDX-License-Identifier: AGPL-3.0-or-later
//! Marks mapped to content. A mark's region is page-space rects (one per
//! quad, else its rect); a draw (`content::Draw`), glyph or annotation is
//! touched when its bounds overlap a rect with positive area (redact.ts
//! `intersects`). What apply removes is decided here and nowhere else:
//!
//! - `page_content`: a page's draws (`Detail::Full`: every glyph, path,
//!   image, shading and Form at any depth) for apply to rewrite.
//! - `annotations`: every /Annots entry a region touches, Widgets, Links
//!   and Popups included (Redact marks excluded).
//! - `shared`: XObjects a mark touches that are also drawn untouched
//!   (redact.ts `redactPreview`'s `shared`), at any depth, on any page or
//!   in an annotation appearance.
use std::collections::{BTreeMap, BTreeSet};

use crate::app::annots::{self, PageSpace, appearance};
use crate::app::info;
use crate::app::pagetree;
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};
use crate::view::ViewDocument;

use super::content::{self, Content, Detail, Limits, WalkError, Walker};
use super::marks::Mark;

/// XObject entries a page's resources are searched through before its
/// content is walked anyway.
const MAX_RESOURCE_VISITS: usize = 1 << 16;
/// Entries `shared` reports at most.
pub const MAX_SHARED: usize = 4096;
/// Content tokens `shared` reads on unmarked pages and appearances; past
/// it, whatever they can reach counts as drawn there.
pub const MAX_SHARED_TOKENS: usize = 32_000_000;

/// redact.ts `intersects`: overlap with positive area.
pub fn intersects(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}

/// Overlap below this (points) is single-precision noise, not cover:
/// stored marks are float32 where content positions are computed exactly.
pub const COVER_TOLERANCE: f64 = 0.01;

/// Cells per side of a page's rect index.
const GRID: usize = 32;
/// A rect over more cells than this is tested for every query.
const GRID_SPAN: usize = 64;

/// One page's rects, indexed by a grid once there are many.
#[derive(Clone, Debug, Default)]
struct PageRects {
    rects: Vec<[f64; 4]>,
    grid: Option<Grid>,
}

#[derive(Clone, Debug)]
struct Grid {
    area: [f64; 4],
    cells: Vec<Vec<u32>>,
    /// Rects spanning more than `GRID_SPAN` cells.
    wide: Vec<u32>,
}

impl Grid {
    /// The cells `r` overlaps, as column and row ranges.
    fn span(&self, r: &[f64; 4]) -> Option<(usize, usize, usize, usize)> {
        let [x0, y0, x1, y1] = self.area;
        if !(r[0] <= x1 && r[2] >= x0 && r[1] <= y1 && r[3] >= y0) {
            return None;
        }
        let cell = |v: f64, lo: f64, hi: f64| {
            let t = if hi > lo { (v - lo) / (hi - lo) } else { 0.0 };
            ((t * GRID as f64).floor().max(0.0) as usize).min(GRID - 1)
        };
        Some((
            cell(r[0], x0, x1),
            cell(r[2], x0, x1),
            cell(r[1], y0, y1),
            cell(r[3], y0, y1),
        ))
    }

    fn build(rects: &[[f64; 4]]) -> Option<Self> {
        let area = appearance::bounds(rects.iter().flat_map(|r| [(r[0], r[1]), (r[2], r[3])]))?;
        if !area.iter().all(|v| v.is_finite()) {
            return None;
        }
        let mut grid = Self {
            area,
            cells: vec![Vec::new(); GRID * GRID],
            wide: Vec::new(),
        };
        for (index, r) in rects.iter().enumerate() {
            let index = u32::try_from(index).ok()?;
            let Some((c0, c1, r0, r1)) = grid.span(r) else {
                continue;
            };
            if (c1 - c0 + 1) * (r1 - r0 + 1) > GRID_SPAN {
                grid.wide.push(index);
                continue;
            }
            for row in r0..=r1 {
                for col in c0..=c1 {
                    if let Some(cell) = grid.cells.get_mut(row * GRID + col) {
                        cell.push(index);
                    }
                }
            }
        }
        Some(grid)
    }
}

impl PageRects {
    fn any(&self, bounds: &[f64; 4], test: impl Fn(&[f64; 4]) -> bool) -> bool {
        let Some(grid) = &self.grid else {
            return self.rects.iter().any(test);
        };
        let check = |index: &u32| self.rects.get(*index as usize).is_some_and(&test);
        if grid.wide.iter().any(check) {
            return true;
        }
        let Some((c0, c1, r0, r1)) = grid.span(bounds) else {
            return false;
        };
        (r0..=r1).any(|row| {
            (c0..=c1)
                .any(|col| (grid.cells.get(row * GRID + col)).is_some_and(|c| c.iter().any(check)))
        })
    }
}

/// The marks' rects by page.
#[derive(Clone, Debug, Default)]
pub struct Regions {
    by_page: BTreeMap<usize, PageRects>,
}

impl Regions {
    pub fn of(marks: &[Mark]) -> Self {
        let mut by_page: BTreeMap<usize, Vec<[f64; 4]>> = BTreeMap::new();
        for mark in marks {
            by_page.entry(mark.page).or_default().extend(mark.rects());
        }
        Self::from_rects(by_page)
    }

    fn from_rects(by_page: BTreeMap<usize, Vec<[f64; 4]>>) -> Self {
        let by_page = (by_page.into_iter())
            .map(|(page, rects)| {
                let grid = (rects.len() > GRID_SPAN)
                    .then(|| Grid::build(&rects))
                    .flatten();
                (page, PageRects { rects, grid })
            })
            .collect();
        Self { by_page }
    }

    /// The marked pages, ascending.
    pub fn pages(&self) -> impl Iterator<Item = usize> + '_ {
        self.by_page.keys().copied()
    }

    pub fn on(&self, page: usize) -> &[[f64; 4]] {
        self.by_page.get(&page).map_or(&[], |p| p.rects.as_slice())
    }

    pub fn touches(&self, page: usize, bounds: &[f64; 4]) -> bool {
        (self.by_page.get(&page)).is_some_and(|p| p.any(bounds, |r| intersects(r, bounds)))
    }

    /// Overlap with a rect by more than `COVER_TOLERANCE` both ways.
    pub fn covers(&self, page: usize, bounds: &[f64; 4]) -> bool {
        let t = COVER_TOLERANCE;
        (self.by_page.get(&page)).is_some_and(|p| {
            p.any(bounds, |r| {
                r[0].max(bounds[0]) + t < r[2].min(bounds[2])
                    && r[1].max(bounds[1]) + t < r[3].min(bounds[3])
            })
        })
    }

    /// Every rect shrunk by `by` on each side (redact.ts checks text in
    /// the mark less two points); a rect that would vanish is dropped.
    pub fn inset(&self, by: f64) -> Self {
        let by_page = (self.by_page.iter())
            .map(|(&page, rects)| {
                let inner = (rects.rects.iter())
                    .map(|r| [r[0] + by, r[1] + by, r[2] - by, r[3] - by])
                    .filter(|r| r[0] < r[2] && r[1] < r[3])
                    .collect();
                (page, inner)
            })
            .collect();
        Self::from_rects(by_page)
    }
}

/// A page's draws: its /Contents and every Form they draw.
pub fn page_content(
    doc: &Document<'static>,
    view: Option<&ViewDocument>,
    page: usize,
    detail: Detail,
    limits: Limits,
) -> Result<Content, WalkError> {
    let pages = info::pages(doc);
    let Some(dict) = pages.get(page).map(|p| p.dict) else {
        return Err(WalkError::Unreadable("no such page".into()));
    };
    let space = PageSpace::of(doc, dict);
    let mut walker = Walker::new(doc, view, page, detail, limits, space.bounds());
    walker.page(dict, space.ctm)?;
    Ok(walker.finish())
}

/// An annotation of a page, with its page-space rect.
#[derive(Clone, Debug, PartialEq)]
pub struct PageAnnot {
    pub page: usize,
    /// None for a dictionary written into /Annots.
    pub id: Option<ObjectId>,
    pub subtype: Vec<u8>,
    pub rect: [f64; 4],
}

/// Each /Annots entry of `page` that is an annotation dictionary with a
/// /Rect, an object listed twice once; Redact marks are left out.
pub fn page_annotations(doc: &Document<'_>, page: usize, dict: &Dictionary) -> Vec<PageAnnot> {
    let space = PageSpace::of(doc, dict);
    let Some(items) = doc.dict_get(dict, b"Annots").as_array() else {
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for item in items {
        let (holder, value) = doc.resolve_with_holder(item);
        let id = holder.filter(|_| item.as_reference().is_some());
        if id.is_some_and(|id| !seen.insert(id)) {
            continue;
        }
        let Some(annot) = value.as_dict() else {
            continue;
        };
        let subtype = doc.dict_get(annot, b"Subtype").as_name().unwrap_or(b"");
        if subtype == b"Redact" {
            continue;
        }
        let Some(rect) = annot
            .get(b"Rect".as_slice())
            .and_then(|r| annots::rect(doc, r))
        else {
            continue;
        };
        out.push(PageAnnot {
            page,
            id,
            subtype: subtype.to_vec(),
            rect: space.rect_to_page(rect),
        });
    }
    out
}

/// The annotations a region touches, page by page.
pub fn annotations(doc: &Document<'_>, regions: &Regions) -> Vec<PageAnnot> {
    let pages = info::pages(doc);
    let mut out = Vec::new();
    for page in regions.pages() {
        let Some(dict) = pages.get(page).map(|p| p.dict) else {
            continue;
        };
        out.extend(
            page_annotations(doc, page, dict)
                .into_iter()
                .filter(|annot| regions.touches(page, &annot.rect)),
        );
    }
    out
}

/// One `shared` entry: an object a mark touches on `page` and drawn
/// untouched on `other`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shared {
    pub page: usize,
    pub kind: &'static str,
    pub other: usize,
}

/// Why `shared` could not decide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SharedError {
    /// A marked page's content passed the walk's limits.
    TooComplex(usize),
    /// A marked page's content cannot be read.
    Unreadable(usize, String),
}

/// One draw of an XObject.
#[derive(Clone, Debug)]
struct Placement {
    page: usize,
    object: ObjectId,
    kind: &'static str,
    bounds: [f64; 4],
    touched: bool,
}

/// redact.ts `redactPreview`'s `shared`: for each draw of an XObject a mark
/// touches, each draw of the same object no mark touches, once per other
/// page and box. Draws at any depth count, and an annotation appearance
/// that draws the object (its annotation untouched, so apply keeps it) is
/// a draw on that annotation's page. A page that could draw the object but
/// cannot be read counts as drawing it.
pub fn shared(
    doc: &Document<'static>,
    view: Option<&ViewDocument>,
    marks: &[Mark],
    limits: Limits,
) -> Result<(Vec<Shared>, usize), SharedError> {
    let regions = Regions::of(marks);
    let pages = info::pages(doc);
    let mut placements: Vec<Placement> = Vec::new();
    let mut inline_alt = 0;
    for page in regions.pages() {
        let content = match page_content(doc, view, page, Detail::Objects, limits) {
            Ok(content) => content,
            Err(WalkError::TooComplex(_)) => return Err(SharedError::TooComplex(page)),
            Err(WalkError::Unreadable(why)) => return Err(SharedError::Unreadable(page, why)),
        };
        inline_alt += content.inline_alt;
        placements.extend(content.draws.iter().filter_map(|draw| {
            let (object, kind) = draw.kind.xobject()?;
            Some(Placement {
                page,
                object,
                kind,
                bounds: draw.bounds,
                touched: regions.touches(page, &draw.bounds),
            })
        }));
    }
    let touched: BTreeSet<ObjectId> = (placements.iter())
        .filter(|p| p.touched)
        .map(|p| p.object)
        .collect();
    if touched.is_empty() {
        return Ok((Vec::new(), inline_alt));
    }
    // Untouched draws elsewhere: unmarked pages that can reach a touched
    // object, and annotation appearances.
    let mut spent = 0usize;
    for (index, page) in pages.iter().enumerate() {
        if regions.on(index).is_empty() {
            let found = other_page(doc, view, index, page.dict, &touched, limits, &mut spent);
            placements.extend(found);
        }
        let found = appearance_draws(
            doc, index, page.dict, &regions, &touched, limits, &mut spent,
        );
        placements.extend(found);
    }
    placements.sort_by_key(|p| p.page);
    let mut untouched: BTreeMap<ObjectId, Vec<&Placement>> = BTreeMap::new();
    for placement in placements.iter().filter(|p| !p.touched) {
        untouched
            .entry(placement.object)
            .or_default()
            .push(placement);
    }
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut sources = BTreeSet::new();
    for placement in placements.iter().filter(|p| p.touched) {
        // Another touched draw of the object on the same page adds nothing.
        if !sources.insert((placement.page, placement.object)) {
            continue;
        }
        for other in untouched
            .get(&placement.object)
            .map_or(&[][..], Vec::as_slice)
        {
            let key = (
                placement.page,
                placement.object,
                other.page,
                other.bounds.map(f64::to_bits),
            );
            if out.len() < MAX_SHARED && seen.insert(key) {
                out.push(Shared {
                    page: placement.page,
                    kind: placement.kind,
                    other: other.page,
                });
            }
        }
    }
    Ok((out, inline_alt))
}

fn resources<'d>(doc: &'d Document<'_>, page: &'d Dictionary) -> Option<&'d Dictionary> {
    pagetree::inherited(doc, page, b"Resources").and_then(|r| doc.resolve(r).as_dict())
}

/// The draws of touched objects on an unmarked page.
fn other_page(
    doc: &Document<'static>,
    view: Option<&ViewDocument>,
    index: usize,
    dict: &Dictionary,
    touched: &BTreeSet<ObjectId>,
    limits: Limits,
    spent: &mut usize,
) -> Vec<Placement> {
    let reach = content::reachable_xobjects(doc, resources(doc, dict), MAX_RESOURCE_VISITS);
    if reach
        .as_ref()
        .is_some_and(|reach| reach.is_disjoint(touched))
    {
        return Vec::new();
    }
    let space = PageSpace::of(doc, dict);
    let walked = match *spent > MAX_SHARED_TOKENS {
        true => Err(WalkError::TooComplex("the walk budget is spent".into())),
        false => page_content(doc, view, index, Detail::Objects, limits),
    };
    // A walk that stopped is charged its whole budget.
    *spent = spent.saturating_add(walked.as_ref().map_or(limits.tokens, |c| c.tokens));
    match walked {
        Ok(content) => (content.draws.iter())
            .filter_map(|draw| {
                let (object, kind) = draw.kind.xobject()?;
                touched.contains(&object).then_some(Placement {
                    page: index,
                    object,
                    kind,
                    bounds: draw.bounds,
                    touched: false,
                })
            })
            .collect(),
        // Unreadable or too complex: whatever it can reach, it may draw.
        Err(_) => (touched.iter())
            .filter(|id| reach.as_ref().is_none_or(|reach| reach.contains(id)))
            .map(|&object| Placement {
                page: index,
                object,
                kind: xobject_kind(doc, object),
                bounds: space.bounds(),
                touched: false,
            })
            .collect(),
    }
}

fn xobject_kind(doc: &Document<'_>, id: ObjectId) -> &'static str {
    match doc
        .stream_dict(id)
        .ok()
        .map(|d| doc.dict_get(d, b"Subtype"))
    {
        Some(Object::Name(name)) if name == b"Form" => "form",
        _ => "image",
    }
}

/// An annotation's appearance streams: /N, /R and /D, each a stream or a
/// dictionary of states.
pub fn appearance_streams(doc: &Document<'_>, annot: &Dictionary) -> Vec<ObjectId> {
    let Some(ap) = doc.dict_get(annot, b"AP").as_dict() else {
        return Vec::new();
    };
    let is_stream = |id: ObjectId| doc.object(id).is_ok_and(|o| o.stream.is_some());
    let mut out = Vec::new();
    for key in [b"N".as_slice(), b"R", b"D"] {
        let Some(value) = ap.get(key) else {
            continue;
        };
        let (holder, resolved) = doc.resolve_with_holder(value);
        match holder.filter(|&id| is_stream(id)) {
            Some(id) => out.push(id),
            None => {
                for state in resolved
                    .as_dict()
                    .map(|d| d.values().collect::<Vec<_>>())
                    .unwrap_or_default()
                {
                    if let (Some(id), _) = doc.resolve_with_holder(state) {
                        if is_stream(id) {
                            out.push(id);
                        }
                    }
                }
            }
        }
    }
    out
}

/// Touched objects drawn by the appearances of `page`'s annotations that
/// no region touches.
fn appearance_draws(
    doc: &Document<'static>,
    index: usize,
    dict: &Dictionary,
    regions: &Regions,
    touched: &BTreeSet<ObjectId>,
    limits: Limits,
    spent: &mut usize,
) -> Vec<Placement> {
    let mut out = Vec::new();
    for annot in page_annotations(doc, index, dict) {
        if regions.touches(index, &annot.rect) {
            continue;
        }
        let Some(annot_dict) = annot.id.and_then(|id| doc.resolve_id(id).as_dict()) else {
            continue;
        };
        for stream in appearance_streams(doc, annot_dict) {
            let mut used: Vec<ObjectId> = Vec::new();
            if touched.contains(&stream) {
                used.push(stream);
            }
            let form = doc.stream_dict(stream).ok();
            let own = form.and_then(|f| doc.dict_get(f, b"Resources").as_dict());
            let reach = content::reachable_xobjects(doc, own, MAX_RESOURCE_VISITS);
            if !reach
                .as_ref()
                .is_some_and(|reach| reach.is_disjoint(touched))
            {
                let mut walker = Walker::new(doc, None, index, Detail::Objects, limits, annot.rect);
                let walked = match *spent > MAX_SHARED_TOKENS {
                    true => Err(WalkError::TooComplex("the walk budget is spent".into())),
                    false => walker.form(stream, appearance::IDENTITY, own),
                };
                let content = walker.finish();
                *spent = spent.saturating_add(content.tokens);
                match walked {
                    Ok(()) => used.extend(
                        (content.draws.iter())
                            .filter_map(|d| d.kind.xobject().map(|(id, _)| id))
                            .filter(|id| touched.contains(id)),
                    ),
                    Err(_) => used.extend(
                        (touched.iter())
                            .filter(|id| reach.as_ref().is_none_or(|reach| reach.contains(id))),
                    ),
                }
            }
            out.extend(used.into_iter().map(|object| Placement {
                page: index,
                object,
                kind: xobject_kind(doc, object),
                bounds: annot.rect,
                touched: false,
            }));
        }
    }
    out
}
