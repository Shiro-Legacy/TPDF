// SPDX-License-Identifier: AGPL-3.0-or-later
//! Marks mapped to content. A mark's region is page-space rects (one per
//! quad, else its rect); a draw (`content::Draw`) or annotation is touched
//! when its bounds overlap a rect with positive area (redact.ts
//! `intersects`); a glyph goes by `GlyphRule`. What apply removes is
//! decided here and nowhere else:
//!
//! - `page_content`: a page's draws (`Detail::Full`: every glyph, path,
//!   image, shading and Form at any depth) for apply to rewrite.
//! - `GlyphRule`: which glyphs apply removes (and verify looks for).
//! - `annotations`: every /Annots entry a region touches, Widgets, Links
//!   and Popups included (Redact marks excluded).
//! - `shared`: XObjects a mark touches that are also drawn where no mark
//!   covers what it touches (redact.ts `redactPreview`'s `shared`), at any
//!   depth, on any page or in an annotation appearance.
use std::collections::{BTreeMap, BTreeSet};

use crate::app::annots::{self, PageSpace, appearance, transform_rect};
use crate::app::info;
use crate::app::pagetree;
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};
use crate::view::ViewDocument;

use super::content::{self, Content, Detail, DrawKind, Limits, WalkError, Walker};
use super::marks::{Mark, SEARCH_MARGIN};

/// Entries `shared` reports at most.
pub const MAX_SHARED: usize = 4096;
/// Work `shared` does beyond the marked pages, document-wide: content
/// tokens read on unmarked pages and appearances, XObject entries visited
/// for reachability, /Annots entries read, mark rects compared between
/// placements. Past it, whatever a page or appearance can reach counts as
/// drawn there and a touched placement as showing what another's marks
/// touch.
pub const MAX_SHARED_WORK: usize = 32_000_000;
/// Placements of objects drawn elsewhere `shared` keeps in all.
const MAX_PLACEMENTS: usize = 1 << 18;
/// Content tokens the marked pages' walks read in all (each walk also
/// within its own limits); past it the preview is too complex.
pub const MAX_MARKED_TOKENS: usize = 64_000_000;
/// Distinct touched XObject draws on the marked pages; past it the preview
/// is too complex.
const MAX_SOURCES: usize = 1 << 16;
/// Mark rects carried between touched draws in all; past it, the classes
/// left count as showing what their marks touch on every other draw.
const MAX_PAIR_WORK: usize = 1 << 22;

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
    /// The indices of the rects that may meet `bounds` (grown by the
    /// tolerance, so a containing or abutting rect is among them).
    fn candidates(&self, bounds: &[f64; 4]) -> Option<Vec<u32>> {
        let grid = self.grid.as_ref()?;
        let mut out = grid.wide.clone();
        let query = appearance::grow(*bounds, COVER_TOLERANCE);
        if let Some((c0, c1, r0, r1)) = grid.span(&query) {
            for row in r0..=r1 {
                for col in c0..=c1 {
                    out.extend(grid.cells.get(row * GRID + col).into_iter().flatten());
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        Some(out)
    }

    fn any(&self, bounds: &[f64; 4], test: impl Fn(&[f64; 4]) -> bool) -> bool {
        match self.candidates(bounds) {
            None => self.rects.iter().any(test),
            Some(found) => (found.iter()).any(|i| self.rects.get(*i as usize).is_some_and(&test)),
        }
    }

    fn select(&self, bounds: &[f64; 4], test: impl Fn(&[f64; 4]) -> bool) -> Vec<[f64; 4]> {
        match self.candidates(bounds) {
            None => self.rects.iter().copied().filter(|r| test(r)).collect(),
            Some(found) => (found.iter())
                .filter_map(|i| self.rects.get(*i as usize).copied())
                .filter(|r| test(r))
                .collect(),
        }
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

    /// `bounds` lies inside a rect (closed, within `COVER_TOLERANCE`), a
    /// box without width or height included.
    pub fn contains(&self, page: usize, bounds: &[f64; 4]) -> bool {
        let t = COVER_TOLERANCE;
        (self.by_page.get(&page)).is_some_and(|p| {
            p.any(bounds, |r| {
                r[0] - t <= bounds[0]
                    && bounds[2] <= r[2] + t
                    && r[1] - t <= bounds[1]
                    && bounds[3] <= r[3] + t
            })
        })
    }

    /// `bounds` meets a rect, edges included (a point or a line counts).
    pub fn meets(&self, page: usize, bounds: &[f64; 4]) -> bool {
        (self.by_page.get(&page)).is_some_and(|p| {
            p.any(bounds, |r| {
                bounds[0] <= r[2] && bounds[2] >= r[0] && bounds[1] <= r[3] && bounds[3] >= r[1]
            })
        })
    }

    /// The rects `bounds` touches.
    pub fn touching(&self, page: usize, bounds: &[f64; 4]) -> Vec<[f64; 4]> {
        (self.by_page.get(&page))
            .map_or_else(Vec::new, |p| p.select(bounds, |r| intersects(r, bounds)))
    }
}

/// Apply's test for a glyph, and verify's for a glyph or character left on
/// a marked page. What lies past the page box is not seen, so a box is first
/// cut to the page box (one wholly outside shrinks onto the nearest edge or
/// corner: a mark reaching the edge takes what runs past it). The glyph goes
/// when that box lies inside a mark's rect (`Regions::contains`: a glyph
/// without width or height, at size 0, under `0 Tz` or a singular matrix
/// included), or when its central half meets the mark (MuPDF's text
/// filter): an area mark's rect, a search mark's hit (its quads less the
/// two points the search grew them by), so the neighbour a search mark's
/// margin partly hides stays. A box with no position (NaN) goes when the
/// page has a mark.
#[derive(Clone, Debug, Default)]
pub struct GlyphRule {
    drawn: Regions,
    hits: Regions,
    boxes: BTreeMap<usize, [f64; 4]>,
}

impl GlyphRule {
    pub fn new(doc: &Document<'_>, marks: &[Mark]) -> Self {
        let pages = info::pages(doc);
        let mut boxes = BTreeMap::new();
        for mark in marks {
            if let Some(page) = pages.get(mark.page) {
                (boxes.entry(mark.page)).or_insert_with(|| PageSpace::of(doc, page.dict).bounds());
            }
        }
        Self::with_boxes(marks, boxes)
    }

    /// The rule with the page boxes given (page space, by page).
    pub fn with_boxes(marks: &[Mark], boxes: BTreeMap<usize, [f64; 4]>) -> Self {
        let mut hits: BTreeMap<usize, Vec<[f64; 4]>> = BTreeMap::new();
        for mark in marks {
            let area = boxes.get(&mark.page).copied();
            (hits.entry(mark.page).or_default()).extend(hit_rects(mark, area));
        }
        Self {
            drawn: Regions::of(marks),
            hits: Regions::from_rects(hits),
            boxes,
        }
    }

    /// The marks' rects (what apply blacks out).
    pub fn drawn(&self) -> &Regions {
        &self.drawn
    }

    /// Apply removes a glyph whose page-space box is `bounds`.
    pub fn removes(&self, page: usize, bounds: &[f64; 4]) -> bool {
        let Some(visible) = self.visible(page, bounds) else {
            return self.boxes.contains_key(&page) && bounds.iter().any(|v| v.is_nan());
        };
        let [x0, y0, x1, y1] = visible;
        let (w, h) = ((x1 - x0) / 4.0, (y1 - y0) / 4.0);
        self.drawn.contains(page, &visible)
            || self.hits.meets(page, &[x0 + w, y0 + h, x1 - w, y1 - h])
    }

    /// `removes` for a glyph whose box is turned: `quad` is the box itself
    /// (page space, corners in order) and `bounds` holds it. Its central
    /// half is the quad's, so a glyph of turned text goes when the middle
    /// of the glyph itself meets the hit, not of the box around it. A box
    /// past the page box falls back to `removes`.
    pub fn removes_glyph(&self, page: usize, bounds: &[f64; 4], quad: &[f32; 8]) -> bool {
        let Some(visible) = self.visible(page, bounds) else {
            return self.boxes.contains_key(&page) && bounds.iter().any(|v| v.is_nan());
        };
        let corners: [(f64, f64); 4] =
            std::array::from_fn(|i| (f64::from(quad[2 * i]), f64::from(quad[2 * i + 1])));
        let finite = corners.iter().all(|(x, y)| x.is_finite() && y.is_finite());
        if !finite || visible != *bounds {
            return self.removes(page, bounds);
        }
        if self.drawn.contains(page, &visible) {
            return true;
        }
        let (cx, cy) = (
            corners.iter().map(|p| p.0).sum::<f64>() / 4.0,
            corners.iter().map(|p| p.1).sum::<f64>() / 4.0,
        );
        let central = corners.map(|(x, y)| (cx + (x - cx) / 2.0, cy + (y - cy) / 2.0));
        let around = appearance::bounds(central.iter().copied()).unwrap_or(visible);
        (self.hits.by_page.get(&page)).is_some_and(|p| {
            p.any(&around, |r| {
                around[0] <= r[2]
                    && around[2] >= r[0]
                    && around[1] <= r[3]
                    && around[3] >= r[1]
                    && quad_meets(&central, r)
            })
        })
    }

    /// A box that holds wherever a run's glyphs could be (an unmeasured
    /// run) meets a mark, edges included: apply removes the run whole.
    pub fn reaches(&self, page: usize, bounds: &[f64; 4]) -> bool {
        match self.visible(page, bounds) {
            Some(visible) => self.drawn.meets(page, &visible),
            None => self.boxes.contains_key(&page) && bounds.iter().any(|v| v.is_nan()),
        }
    }

    /// Verify's test for a character the view still reads: its box (cut to
    /// the page) lies inside a mark's rect, or its centre lies in a mark's
    /// hit. `removes` takes every glyph whose box holds such a character's
    /// (the walk's box holds the view's), so a character this finds was
    /// not removed.
    pub fn under(&self, page: usize, bounds: &[f64; 4]) -> bool {
        let Some(visible) = self.visible(page, bounds) else {
            return self.boxes.contains_key(&page) && bounds.iter().any(|v| v.is_nan());
        };
        let [x0, y0, x1, y1] = visible;
        let (x, y) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        self.drawn.contains(page, &visible) || self.hits.meets(page, &[x, y, x, y])
    }

    /// `bounds` cut to the page box; None off a marked page or for NaN.
    fn visible(&self, page: usize, bounds: &[f64; 4]) -> Option<[f64; 4]> {
        let area = self.boxes.get(&page)?;
        if bounds.iter().any(|v| v.is_nan()) {
            return None;
        }
        let x = |v: f64| v.max(area[0]).min(area[2]);
        let y = |v: f64| v.max(area[1]).min(area[3]);
        let [x0, y0, x1, y1] = [x(bounds[0]), y(bounds[1]), x(bounds[2]), y(bounds[3])];
        Some([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)])
    }
}

/// Whether a convex quad and a rect meet, edges included (separating
/// axes: the rect's and the quad's edge normals).
fn quad_meets(quad: &[(f64, f64); 4], rect: &[f64; 4]) -> bool {
    let corners = [
        (rect[0], rect[1]),
        (rect[2], rect[1]),
        (rect[2], rect[3]),
        (rect[0], rect[3]),
    ];
    let separated = |axis: (f64, f64)| {
        let project = |p: &(f64, f64)| p.0 * axis.0 + p.1 * axis.1;
        let range = |points: &[(f64, f64)]| {
            points
                .iter()
                .map(project)
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
                    (a.min(v), b.max(v))
                })
        };
        let (a0, a1) = range(quad);
        let (b0, b1) = range(&corners);
        a1 < b0 || b1 < a0
    };
    let edges = (0..4).map(|i| {
        let (a, b) = (quad[i], quad[(i + 1) % 4]);
        (a.1 - b.1, b.0 - a.0)
    });
    let mut axes = [(1.0, 0.0), (0.0, 1.0)].into_iter().chain(edges);
    !axes.any(|axis| (axis.0 != 0.0 || axis.1 != 0.0) && separated(axis))
}

/// The rects a mark's hit covers: a search mark's quads less the margin
/// the search grew them by (a side on the page box's edge kept: the margin
/// may have been cut there; a rect thinner than the margins shrinks to its
/// middle, never vanishes), else its rects.
pub fn hit_rects(mark: &Mark, area: Option<[f64; 4]>) -> Vec<[f64; 4]> {
    let rects = mark.rects();
    if mark.search.is_none() || mark.quads.is_empty() {
        return rects;
    }
    let edge = |v: f64, e: Option<f64>| e.is_some_and(|e| (v - e).abs() <= COVER_TOLERANCE);
    let shrink = |lo: f64, hi: f64, lo_edge: Option<f64>, hi_edge: Option<f64>| {
        let a = if edge(lo, lo_edge) {
            lo
        } else {
            lo + SEARCH_MARGIN
        };
        let b = if edge(hi, hi_edge) {
            hi
        } else {
            hi - SEARCH_MARGIN
        };
        if a <= b {
            (a, b)
        } else {
            let middle = (lo + hi) / 2.0;
            (middle, middle)
        }
    };
    (rects.into_iter())
        .map(|r| {
            let (x0, x1) = shrink(r[0], r[2], area.map(|a| a[0]), area.map(|a| a[2]));
            let (y0, y1) = shrink(r[1], r[3], area.map(|a| a[1]), area.map(|a| a[3]));
            [x0, y0, x1, y1]
        })
        .collect()
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
    annotation_entries(doc, page, dict)
        .into_iter()
        .map(|(annot, _)| annot)
        .collect()
}

/// `page_annotations` with each annotation's dictionary (a direct one too).
fn annotation_entries<'d>(
    doc: &'d Document<'_>,
    page: usize,
    dict: &'d Dictionary,
) -> Vec<(PageAnnot, &'d Dictionary)> {
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
        let entry = PageAnnot {
            page,
            id,
            subtype: subtype.to_vec(),
            rect: space.rect_to_page(rect),
        };
        out.push((entry, annot));
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

/// One `shared` entry: an object a mark touches on `page` and drawn on
/// `other` where no mark covers what it touches.
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
    /// Object space (an image's unit square, a Form's space) to page space;
    /// None where the draw was not walked (an appearance, a page that
    /// could not be read).
    matrix: Option<[f64; 6]>,
}

/// Draws of touched objects elsewhere, each object's distinct (page, box)
/// up to `MAX_SHARED` (what the reply can list), `MAX_PLACEMENTS` in all.
#[derive(Default)]
struct Elsewhere {
    by_object: BTreeMap<ObjectId, Drawn>,
    kept: usize,
}

/// One object's draws, each distinct (page, box bits) once.
type Drawn = (BTreeSet<(usize, [u64; 4])>, Vec<Placement>);

impl Elsewhere {
    fn add(&mut self, placement: Placement) {
        let (keys, list) = self.by_object.entry(placement.object).or_default();
        if keys.len() >= MAX_SHARED || self.kept >= MAX_PLACEMENTS {
            return;
        }
        if keys.insert((placement.page, placement.bounds.map(f64::to_bits))) {
            list.push(placement);
            self.kept += 1;
        }
    }
}

/// What `shared` spends beyond the marked pages (`MAX_SHARED_WORK`), with
/// what it learned: reachability by resources dictionary, the touched
/// objects each appearance stream draws.
struct Work<'t> {
    touched: &'t BTreeSet<ObjectId>,
    limits: Limits,
    spent: usize,
    /// The touched objects a resources dictionary (by address: it lives as
    /// long as the document) can draw; None when the budget ran out.
    reach: BTreeMap<usize, Option<BTreeSet<ObjectId>>>,
    appearances: BTreeMap<ObjectId, Vec<ObjectId>>,
}

impl Work<'_> {
    fn spent(&self) -> bool {
        self.spent > MAX_SHARED_WORK
    }

    fn charge(&mut self, work: usize) {
        self.spent = self.spent.saturating_add(work);
    }

    fn reach(
        &mut self,
        doc: &Document<'_>,
        resources: Option<&Dictionary>,
    ) -> Option<BTreeSet<ObjectId>> {
        let key = resources.map_or(0, |r| std::ptr::from_ref(r).addr());
        if let Some(found) = self.reach.get(&key) {
            return found.clone();
        }
        let room = MAX_SHARED_WORK.saturating_sub(self.spent);
        let mut left = room;
        let found = content::reachable_xobjects(doc, resources, &mut left).map(|all| {
            all.intersection(self.touched)
                .copied()
                .collect::<BTreeSet<_>>()
        });
        self.charge(room - left);
        if found.is_none() {
            // Out of budget: nothing more is searched.
            self.spent = MAX_SHARED_WORK.saturating_add(1);
        }
        self.reach.insert(key, found.clone());
        found
    }

    /// The touched objects an appearance stream draws, read once.
    fn appearance(
        &mut self,
        doc: &Document<'static>,
        page: usize,
        stream: ObjectId,
        rect: [f64; 4],
    ) -> Vec<ObjectId> {
        if let Some(found) = self.appearances.get(&stream) {
            return found.clone();
        }
        let mut used: Vec<ObjectId> = Vec::new();
        if self.touched.contains(&stream) {
            used.push(stream);
        }
        let form = doc.stream_dict(stream).ok();
        let own = form.and_then(|f| doc.dict_get(f, b"Resources").as_dict());
        let reach = self.reach(doc, own);
        if !reach.as_ref().is_some_and(BTreeSet::is_empty) {
            let mut walker = Walker::new(doc, None, page, Detail::Objects, self.limits, rect);
            let walked = match self.spent() {
                true => Err(WalkError::TooComplex("the walk budget is spent".into())),
                false => walker.form(stream, appearance::IDENTITY, own),
            };
            let content = walker.finish();
            self.charge(content.tokens);
            match walked {
                Ok(()) => used.extend(
                    (content.draws.iter())
                        .filter_map(|d| d.kind.xobject().map(|(id, _)| id))
                        .filter(|id| self.touched.contains(id)),
                ),
                Err(_) => used.extend(
                    (self.touched.iter())
                        .filter(|id| reach.as_ref().is_none_or(|reach| reach.contains(id))),
                ),
            }
        }
        used.sort_unstable();
        used.dedup();
        self.appearances.insert(stream, used.clone());
        used
    }
}

/// redact.ts `redactPreview`'s `shared`: for each draw of an XObject a mark
/// touches, each other draw of the same object that shows what the marks
/// touch unmarked: one no mark touches, or one whose marks do not cover
/// that part of the object (carried through both draws' matrices); once
/// per other page and box. Draws at any depth count, and an annotation
/// appearance that draws the object is a draw on that annotation's page
/// unless apply removes the annotation (one a region touches).
/// A page that could draw the object but cannot be read counts as drawing
/// it.
pub fn shared(
    doc: &Document<'static>,
    view: Option<&ViewDocument>,
    marks: &[Mark],
    limits: Limits,
) -> Result<(Vec<Shared>, usize), SharedError> {
    let regions = Regions::of(marks);
    let pages = info::pages(doc);
    let mut elsewhere = Elsewhere::default();
    // Touched draws, each distinct (page, object, box, matrix) once.
    let mut sources: Vec<Placement> = Vec::new();
    let mut distinct = BTreeSet::new();
    let mut inline_alt = 0;
    let mut walked = 0usize;
    for page in regions.pages() {
        let room = MAX_MARKED_TOKENS.saturating_sub(walked);
        let page_limits = Limits {
            tokens: limits.tokens.min(room),
            ..limits
        };
        let content = match page_content(doc, view, page, Detail::Objects, page_limits) {
            Ok(content) => content,
            Err(WalkError::TooComplex(_)) => return Err(SharedError::TooComplex(page)),
            Err(WalkError::Unreadable(why)) => return Err(SharedError::Unreadable(page, why)),
        };
        walked = walked.saturating_add(content.tokens);
        inline_alt += content.inline_alt;
        for draw in &content.draws {
            let Some((object, kind)) = draw.kind.xobject() else {
                continue;
            };
            let matrix = match &draw.kind {
                DrawKind::Image { matrix, .. } | DrawKind::Form { matrix, .. } => Some(*matrix),
                _ => None,
            };
            let placement = Placement {
                page,
                object,
                kind,
                bounds: draw.bounds,
                matrix,
            };
            if !regions.touches(page, &draw.bounds) {
                elsewhere.add(placement);
                continue;
            }
            let key = (
                page,
                object,
                draw.bounds.map(f64::to_bits),
                matrix.map(|m| m.map(f64::to_bits)),
            );
            if distinct.insert(key) {
                if sources.len() >= MAX_SOURCES {
                    return Err(SharedError::TooComplex(page));
                }
                sources.push(placement);
            }
        }
    }
    let touched: BTreeSet<ObjectId> = sources.iter().map(|p| p.object).collect();
    if touched.is_empty() {
        return Ok((Vec::new(), inline_alt));
    }
    let mut work = Work {
        touched: &touched,
        limits,
        spent: 0,
        reach: BTreeMap::new(),
        appearances: BTreeMap::new(),
    };
    for (index, page) in pages.iter().enumerate() {
        if regions.on(index).is_empty() {
            for found in other_page(doc, view, index, page.dict, &mut work) {
                elsewhere.add(found);
            }
        }
        for found in appearance_draws(doc, index, page.dict, &regions, &mut work) {
            elsewhere.add(found);
        }
    }
    let pairs = Pairs::of(&regions, &sources);
    let mut order: Vec<usize> = (0..sources.len()).collect();
    order.sort_by_key(|&i| sources.get(i).map_or(0, |p| p.page));
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut listed = BTreeSet::new();
    let mut compared = BTreeSet::new();
    for index in order {
        let Some(placement) = sources.get(index) else {
            continue;
        };
        if out.len() >= MAX_SHARED {
            break;
        }
        let mut others: Vec<&Placement> = Vec::new();
        // Each touched draw of an object on a page lists the same
        // untouched draws, and each of a class on a page the same touched.
        if listed.insert((placement.page, placement.object)) {
            others.extend(
                (elsewhere.by_object.get(&placement.object))
                    .map_or(&[][..], |(_, list)| list.as_slice()),
            );
        }
        let class = pairs.class.get(index).copied().unwrap_or(0);
        if compared.insert((placement.page, class)) {
            others.extend(
                (pairs.showing(placement.object, class).iter())
                    .filter(|&&j| j != index)
                    .filter_map(|&j| sources.get(j)),
            );
        }
        others.sort_by_key(|p| p.page);
        for other in others {
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

/// What one class of touched draws (the same object and the same parts of
/// it under marks, so the same answers) shows unmarked.
enum Shows {
    /// These touched draws (indices into the sources).
    Draws(Vec<usize>),
    /// Every touched draw of the object (not compared).
    All,
}

/// The touched draws compared pairwise: whether the marks on one touched
/// draw's page cover the parts of the object another's marks touch.
struct Pairs {
    /// Each source's class.
    class: Vec<usize>,
    /// By class.
    shows: Vec<Shows>,
    /// Each object's touched draws, one per distinct page and box.
    distinct: BTreeMap<ObjectId, Vec<usize>>,
}

/// Object-space parts, rounded to a millionth, as a class key.
type ClassKey = Vec<[i64; 4]>;

impl Pairs {
    fn of(regions: &Regions, sources: &[Placement]) -> Self {
        let mut by_object: BTreeMap<ObjectId, Vec<usize>> = BTreeMap::new();
        let mut distinct: BTreeMap<ObjectId, Vec<usize>> = BTreeMap::new();
        let mut boxes = BTreeSet::new();
        for (index, source) in sources.iter().enumerate() {
            by_object.entry(source.object).or_default().push(index);
            if boxes.insert((source.object, source.page, source.bounds.map(f64::to_bits))) {
                distinct.entry(source.object).or_default().push(index);
            }
        }
        let mut class = vec![0; sources.len()];
        let mut shows: Vec<Shows> = Vec::new();
        let mut work = 0usize;
        for list in by_object.values() {
            let mut classes: BTreeMap<ClassKey, usize> = BTreeMap::new();
            for &i in list {
                let Some(source) = sources.get(i) else {
                    continue;
                };
                let id = match object_parts(regions, source) {
                    // A singular matrix: not compared.
                    None => {
                        shows.push(Shows::All);
                        shows.len() - 1
                    }
                    Some(parts) => {
                        let key: ClassKey = (parts.iter())
                            .map(|r| r.map(|v| (v * 1e6).round() as i64))
                            .collect();
                        match classes.get(&key) {
                            Some(&known) => known,
                            None => {
                                // A new class is compared with each touched
                                // draw (itself included) while the budget lasts.
                                let compared = if work > MAX_PAIR_WORK {
                                    Shows::All
                                } else {
                                    let mut found = Vec::new();
                                    for &j in list {
                                        let Some(other) = sources.get(j) else {
                                            continue;
                                        };
                                        work = work.saturating_add(1 + parts.len());
                                        if !covered(regions, &parts, other) {
                                            found.push(j);
                                        }
                                    }
                                    Shows::Draws(found)
                                };
                                classes.insert(key, shows.len());
                                shows.push(compared);
                                shows.len() - 1
                            }
                        }
                    }
                };
                if let Some(slot) = class.get_mut(i) {
                    *slot = id;
                }
            }
        }
        Self {
            class,
            shows,
            distinct,
        }
    }

    /// The touched draws of `object` class `class` shows unmarked.
    fn showing(&self, object: ObjectId, class: usize) -> &[usize] {
        match self.shows.get(class) {
            Some(Shows::Draws(list)) => list,
            Some(Shows::All) => self.distinct.get(&object).map_or(&[], Vec::as_slice),
            None => &[],
        }
    }
}

/// The parts of the object a draw's marks touch, in object space (an
/// image's unit square, a Form's space); None for a singular matrix.
fn object_parts(regions: &Regions, source: &Placement) -> Option<Vec<[f64; 4]>> {
    let back = inverse(source.matrix?)?;
    let b = source.bounds;
    let mut parts: Vec<[f64; 4]> = (regions.touching(source.page, &b).iter())
        .map(|r| {
            [
                r[0].max(b[0]),
                r[1].max(b[1]),
                r[2].min(b[2]),
                r[3].min(b[3]),
            ]
        })
        .map(|part| transform_rect(&back, part))
        .collect();
    parts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(parts)
}

/// A matrix's inverse; None when it has none.
fn inverse(m: [f64; 6]) -> Option<[f64; 6]> {
    let det = m[0] * m[3] - m[1] * m[2];
    (det != 0.0 && det.is_finite()).then(|| annots::invert(m))
}

/// Whether the marks on `other`'s page cover `parts` (object space) as
/// `other` draws them: each part inside one rect there. An unknown matrix
/// answers no.
fn covered(regions: &Regions, parts: &[[f64; 4]], other: &Placement) -> bool {
    let Some(to) = other.matrix else {
        return false;
    };
    let o = other.bounds;
    parts.iter().all(|part| {
        let there = transform_rect(&to, *part);
        let there = [
            there[0].max(o[0]),
            there[1].max(o[1]),
            there[2].min(o[2]),
            there[3].min(o[3]),
        ];
        // Nothing of the part is drawn there.
        let shown = there[0] < there[2] && there[1] < there[3];
        !shown || regions.contains(other.page, &there)
    })
}

fn resources<'d>(doc: &'d Document<'_>, page: &'d Dictionary) -> Option<&'d Dictionary> {
    pagetree::inherited(doc, page, b"Resources").and_then(|r| doc.resolve(r).as_dict())
}

/// Touched objects drawn on an unmarked page.
fn other_page(
    doc: &Document<'static>,
    view: Option<&ViewDocument>,
    index: usize,
    dict: &Dictionary,
    work: &mut Work<'_>,
) -> Vec<Placement> {
    let reach = work.reach(doc, resources(doc, dict));
    if reach.as_ref().is_some_and(BTreeSet::is_empty) {
        return Vec::new();
    }
    let space = PageSpace::of(doc, dict);
    let walked = match work.spent() {
        true => Err(WalkError::TooComplex("the walk budget is spent".into())),
        false => page_content(doc, view, index, Detail::Objects, work.limits),
    };
    // A walk that stopped is charged its whole budget.
    work.charge(walked.as_ref().map_or(work.limits.tokens, |c| c.tokens));
    match walked {
        Ok(content) => (content.draws.iter())
            .filter_map(|draw| {
                let (object, kind) = draw.kind.xobject()?;
                work.touched.contains(&object).then_some(Placement {
                    page: index,
                    object,
                    kind,
                    bounds: draw.bounds,
                    matrix: None,
                })
            })
            .collect(),
        // Unreadable or too complex: whatever it can reach, it may draw.
        Err(_) => (work.touched.iter())
            .filter(|id| reach.as_ref().is_none_or(|reach| reach.contains(id)))
            .map(|&object| Placement {
                page: index,
                object,
                kind: xobject_kind(doc, object),
                bounds: space.bounds(),
                matrix: None,
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
/// apply keeps: those no region touches (apply removes the others, Widgets
/// included). Past the budget, the page counts as drawing every touched
/// object.
fn appearance_draws(
    doc: &Document<'static>,
    index: usize,
    dict: &Dictionary,
    regions: &Regions,
    work: &mut Work<'_>,
) -> Vec<Placement> {
    let Some(items) = doc.dict_get(dict, b"Annots").as_array() else {
        return Vec::new();
    };
    work.charge(items.len());
    let placement = |object: ObjectId, bounds: [f64; 4]| Placement {
        page: index,
        object,
        kind: xobject_kind(doc, object),
        bounds,
        matrix: None,
    };
    if work.spent() {
        let bounds = PageSpace::of(doc, dict).bounds();
        return (work.touched.iter())
            .map(|&object| placement(object, bounds))
            .collect();
    }
    let mut out = Vec::new();
    for (annot, annot_dict) in annotation_entries(doc, index, dict) {
        if regions.touches(index, &annot.rect) {
            continue;
        }
        for stream in appearance_streams(doc, annot_dict) {
            let drawn = work.appearance(doc, index, stream, annot.rect);
            out.extend(
                drawn
                    .into_iter()
                    .map(|object| placement(object, annot.rect)),
            );
        }
    }
    out
}
