// SPDX-License-Identifier: AGPL-3.0-or-later
//! Redaction marks: the Redact annotations of a document as redact.ts reads
//! them (`redactMarks`), the rect a request may place (`redactRect`), and the
//! annotations redact-add, redact-search and redact-set write, with MuPDF's
//! dictionaries (`createAnnotation`, `setRect`, `setQuadPoints`, `update`).
use std::collections::BTreeMap;

use serde_json::Value;

use crate::app::annots::{self, AnnotError, Color, PageSpace, Quad, appearance};
use crate::app::edit::{Edit, Place};
use crate::app::info;
use crate::app::request;
use crate::app::{AppError, Transaction};
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId, decode_text_string};

/// redact.ts `MAX_REDACT_MARKS`: marks a document may hold.
pub const MAX_REDACT_MARKS: usize = 2000;
/// redact.ts `MAX_REDACT_TEXT`: UTF-16 units of a search.
pub const MAX_REDACT_TEXT: usize = 256;
/// The private key a search mark carries its text under.
pub const SEARCH_KEY: &[u8] = b"PDF4AllSearch";
/// Points a search grows each hit quad by on every side (redact.ts
/// `inflatedQuad`).
pub const SEARCH_MARGIN: f64 = 2.0;
/// Quads a mark keeps for region tests; a mark with more is tested by the
/// union of all of them.
pub const MAX_MARK_QUADS: usize = 1024;
/// Quads the marks of a document keep in all; past it, each further mark
/// with quads is tested by their union.
pub const MAX_DOCUMENT_QUADS: usize = 1 << 18;

/// One Redact annotation of a page.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub page: usize,
    /// None for a dictionary written into /Annots.
    pub id: Option<ObjectId>,
    /// Page space: the union of the quads when there are any (what the
    /// search marked), else MuPDF's `getRect`.
    pub rect: [f64; 4],
    /// Page space, in /QuadPoints order; empty for an area mark, and when
    /// there are more than `MAX_MARK_QUADS` (then `rect` stands for them).
    pub quads: Vec<Quad>,
    pub search: Option<String>,
}

impl Mark {
    /// The page-space rects the mark covers: each quad's bounds, else its rect.
    pub fn rects(&self) -> Vec<[f64; 4]> {
        if self.quads.is_empty() {
            return vec![self.rect];
        }
        (self.quads.iter())
            .filter_map(|q| appearance::bounds(q.iter().copied()))
            .collect()
    }
}

fn is_redact(annot: &annots::Annot<'_>) -> bool {
    annot.subtype == b"Redact"
}

/// The marks of one page, in /Annots order (MuPDF's annotation list).
pub fn page_marks(doc: &Document<'_>, index: usize, page: &Dictionary) -> Vec<Mark> {
    marks_within(doc, index, page, &mut { MAX_DOCUMENT_QUADS })
}

fn marks_within(
    doc: &Document<'_>,
    index: usize,
    page: &Dictionary,
    room: &mut usize,
) -> Vec<Mark> {
    let space = PageSpace::of(doc, page);
    (annots::list(doc, page).iter())
        .filter(|annot| is_redact(annot))
        .map(|annot| read_mark(doc, &space, index, annot, room))
        .collect()
}

/// Every mark, page by page (redact.ts `redactMarks`).
pub fn marks(doc: &Document<'_>) -> Vec<Mark> {
    let mut out = Vec::new();
    let mut room = MAX_DOCUMENT_QUADS;
    for (index, page) in info::pages(doc).iter().enumerate() {
        out.extend(marks_within(doc, index, page.dict, &mut room));
    }
    out
}

/// How many marks the document holds, without reading their geometry.
pub fn count(doc: &Document<'_>) -> usize {
    (info::pages(doc).iter())
        .map(|page| {
            annots::list(doc, page.dict)
                .iter()
                .filter(|a| is_redact(a))
                .count()
        })
        .sum()
}

fn read_mark(
    doc: &Document<'_>,
    space: &PageSpace,
    page: usize,
    annot: &annots::Annot<'_>,
    room: &mut usize,
) -> Mark {
    let mut union: Option<[f64; 4]> = None;
    let mut quads = Vec::new();
    let mut too_many = false;
    let values = annot
        .dict
        .get(b"QuadPoints".as_slice())
        .map(|v| doc.resolve(v));
    let items = values.and_then(Object::as_array).unwrap_or(&[]);
    for chunk in items.chunks_exact(8) {
        let n = |i: usize| {
            chunk
                .get(i)
                .and_then(|v| annots::number(doc, v))
                .unwrap_or(0.0)
        };
        let quad: Quad = [(n(0), n(1)), (n(2), n(3)), (n(4), n(5)), (n(6), n(7))];
        let quad = quad.map(|p| space.to_page(p));
        if let Some(bounds) = appearance::bounds(quad.iter().copied()) {
            union = Some(match union {
                None => bounds,
                Some(u) => [
                    u[0].min(bounds[0]),
                    u[1].min(bounds[1]),
                    u[2].max(bounds[2]),
                    u[3].max(bounds[3]),
                ],
            });
        }
        if quads.len() < MAX_MARK_QUADS.min(*room) {
            quads.push(quad);
        } else {
            too_many = true;
        }
    }
    if too_many {
        quads.clear();
    }
    *room -= quads.len();
    // MuPDF's getRect is never missing: an annotation without /Rect has the
    // empty rect at the origin.
    let rect = union
        .or_else(|| annots::page_rect(doc, space, annot.dict))
        .unwrap_or_else(|| space.rect_to_page([0.0; 4]));
    let search = (doc.dict_get(annot.dict, SEARCH_KEY).as_string()).map(decode_text_string);
    Mark {
        page,
        id: annot.id,
        rect,
        quads,
        search,
    }
}

/// redact.ts `redactRect`: four finite numbers, normalized and clamped to
/// the page box, at least 1 by 1 point. Refusals are tagged `invalid`.
pub fn redact_rect(
    bounds: [f64; 4],
    value: Option<&Value>,
    op: &str,
) -> Result<[f64; 4], AppError> {
    let numbers = value
        .and_then(Value::as_array)
        .filter(|items| items.len() == 4)
        .and_then(|items| {
            items
                .iter()
                .map(request::as_number)
                .collect::<Option<Vec<f64>>>()
        });
    let Some(raw) = numbers else {
        return Err(AppError::reason(
            "invalid",
            format!("{op}: rect must be four finite numbers"),
        ));
    };
    let n = |i: usize| raw.get(i).copied().unwrap_or(0.0);
    let x0 = bounds[0].max(n(0).min(n(2)));
    let y0 = bounds[1].max(n(1).min(n(3)));
    let x1 = bounds[2].min(n(0).max(n(2)));
    let y1 = bounds[3].min(n(1).max(n(3)));
    if x1 - x0 < 1.0 || y1 - y0 < 1.0 {
        return Err(AppError::reason(
            "invalid",
            format!("{op}: the clamped rect must be at least 1 by 1 point"),
        ));
    }
    Ok([x0, y0, x1, y1])
}

/// A mark to write (redact.ts `RedactPlan`), in page space.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub page: usize,
    pub rect: [f64; 4],
    pub quads: Vec<Quad>,
    pub search: Option<String>,
}

fn annot_error(op: &str, error: AnnotError) -> AppError {
    AppError::plain(format!("{op}: {error}"))
}

/// The indirect /Annots array a page lists, if any.
fn held_annots(doc: &Document<'_>, page: &Dictionary) -> Option<ObjectId> {
    let (holder, value) = doc.resolve_with_holder(page.get(b"Annots".as_slice())?);
    holder.filter(|_| value.as_array().is_some())
}

/// A page's dictionary: its object, else (a page written into /Kids) the
/// page tree's `index`th page.
fn page_dict<'d>(
    doc: &'d Document<'_>,
    id: Option<ObjectId>,
    index: usize,
) -> Option<&'d Dictionary> {
    match id {
        Some(id) => doc.resolve_id(id).as_dict(),
        None => info::pages(doc).get(index).map(|p| p.dict),
    }
}

/// redact.ts `addRedactPlans`: one Redact annotation per plan, appended to
/// its page's /Annots, numbered before its appearance. A search plan's
/// /Rect is its quads' bounds grown by a point, as MuPDF's `update()`
/// leaves it; an area plan keeps its rect exactly. A page whose /Annots is
/// an array other pages list too gets its own copy first, so the mark is
/// on that page alone (MuPDF appends to the shared array, which marks
/// every page listing it). Returns the new ids.
pub fn write_plans<E: Edit>(
    target: &mut E,
    plans: &[Plan],
    op: &str,
) -> Result<Vec<ObjectId>, AppError> {
    let mut ids = Vec::with_capacity(plans.len());
    let mut listers: BTreeMap<ObjectId, usize> = BTreeMap::new();
    let pages: Vec<Option<ObjectId>> = (info::pages(target.document()).iter())
        .map(|page| {
            if let Some(held) = held_annots(target.document(), page.dict) {
                *listers.entry(held).or_default() += 1;
            }
            page.id
        })
        .collect();
    let no_page = || AppError::plain(format!("{op}: no such page"));
    for plan in plans {
        let page_id = pages.get(plan.page).copied().ok_or_else(no_page)?;
        let doc = target.document();
        let dict = page_dict(doc, page_id, plan.page).ok_or_else(no_page)?;
        let shared = held_annots(doc, dict).filter(|h| listers.get(h).is_some_and(|&n| n > 1));
        // A page written into /Kids keeps the shared array (no place to copy to).
        if let (Some(held), Some(page_id)) = (shared, page_id) {
            let copy = doc.resolve_id(held).clone();
            let place = Place {
                id: page_id,
                keys: vec![b"Annots".to_vec()],
            };
            place.update(target, |value| *value = copy)?;
            if let Some(n) = listers.get_mut(&held) {
                *n -= 1;
            }
        }
        let doc = target.document();
        let dict = page_dict(doc, page_id, plan.page).ok_or_else(no_page)?;
        let space = PageSpace::of(doc, dict);
        let mut annot = Dictionary::new();
        annot.insert(b"Type".to_vec(), Object::name("Annot"));
        annot.insert(b"Subtype".to_vec(), Object::name("Redact"));
        if let Some(page) = page_id {
            annot.insert(b"P".to_vec(), Object::Reference(page));
        }
        annot.insert(b"F".to_vec(), Object::Integer(4));
        let rect = space.rect_to_user(plan.rect);
        annot.insert(b"Rect".to_vec(), appearance::numbers(&rect));
        let quads: Vec<Quad> = plan.quads.iter().map(|q| space.quad_to_user(*q)).collect();
        if !quads.is_empty() {
            let flat: Vec<f64> = quads.iter().flatten().flat_map(|&(x, y)| [x, y]).collect();
            annot.insert(b"QuadPoints".to_vec(), appearance::numbers(&flat));
        }
        if let Some(search) = &plan.search {
            annot.insert(SEARCH_KEY.to_vec(), Object::text(search));
        }
        let id = target.allocate(Object::Null)?;
        let (form, outer) = appearance::redact(rect, &quads, None);
        annot.insert(b"Rect".to_vec(), appearance::numbers(&outer));
        annots::put_appearance(target, &mut annot, &form)?;
        target.put_value(id, Object::Dictionary(annot))?;
        annots::append(target, plan.page, id).map_err(|error| annot_error(op, error))?;
        ids.push(id);
    }
    Ok(ids)
}

/// redact.ts `setRedactRect`: the mark's new rect (its design box, so a
/// foreign /RD is kept around it), no quads and no search tag (a moved
/// search mark is an area mark), and a new appearance.
pub fn set_rect(
    tx: &mut Transaction<'_>,
    page: usize,
    id: ObjectId,
    rect: [f64; 4],
) -> Result<(), AppError> {
    let doc = tx.document();
    let dict = (info::pages(doc).get(page).copied())
        .ok_or_else(|| AppError::plain("redact-set: no such page"))?
        .dict;
    let space = PageSpace::of(doc, dict);
    let Some(mut annot) = doc.resolve_id(id).as_dict().cloned() else {
        return Err(AppError::plain("redact-set: the mark is not a dictionary"));
    };
    let inner = space.rect_to_user(rect);
    let outer = annots::grow_by_rd(doc, &annot, inner);
    let color = annot.get(b"C".as_slice()).and_then(|c| Color::read(doc, c));
    annot.insert(b"Rect".to_vec(), appearance::numbers(&outer));
    annot.remove(b"QuadPoints".as_slice());
    annot.remove(SEARCH_KEY);
    let (form, _) = appearance::redact(outer, &[], color.as_ref());
    annots::put_appearance(tx, &mut annot, &form)?;
    tx.put_value(id, Object::Dictionary(annot))?;
    Ok(())
}

/// Why a search placed no marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchError {
    /// More hits than the marks the document has room for.
    TooMany,
    /// A page's text could not be read (too complex for the view).
    Unsearchable(usize),
}

/// redact.ts `inflatedQuad`: a hit quad's bounds grown by two points,
/// clamped to the page box, as an axis-aligned quad.
fn inflated(quad: &[f64; 8], bounds: [f64; 4]) -> Quad {
    let xs = [quad[0], quad[2], quad[4], quad[6]];
    let ys = [quad[1], quad[3], quad[5], quad[7]];
    let min = |v: [f64; 4]| v.into_iter().fold(f64::INFINITY, f64::min);
    let max = |v: [f64; 4]| v.into_iter().fold(f64::NEG_INFINITY, f64::max);
    let x0 = bounds[0].max(min(xs) - SEARCH_MARGIN);
    let y0 = bounds[1].max(min(ys) - SEARCH_MARGIN);
    let x1 = bounds[2].min(max(xs) + SEARCH_MARGIN);
    let y1 = bounds[3].min(max(ys) + SEARCH_MARGIN);
    [(x0, y0), (x1, y0), (x0, y1), (x1, y1)]
}

/// redact.ts `redactSearchPlan`: one plan per hit on every page of `view`
/// (the document as drawn), its quads inflated, at most `room` of them.
pub fn search_plans(
    doc: &Document<'_>,
    view: &crate::view::ViewDocument,
    text: &str,
    room: usize,
) -> Result<Vec<Plan>, SearchError> {
    let pages = info::pages(doc);
    let reader = view.text_reader();
    let mut plans: Vec<Plan> = Vec::new();
    for index in 0..view.page_count() {
        let Some(page) = pages.get(index) else {
            break;
        };
        let bounds = PageSpace::of(doc, page.dict).bounds();
        let structured = reader
            .structured_text(index)
            .map_err(|_| SearchError::Unsearchable(index))?;
        let found = structured.search_hits(text, (room + 1).saturating_sub(plans.len()));
        for hit in found.hits {
            // MuPDF's quads are single precision.
            let quads: Vec<Quad> = (hit.iter())
                .map(|q| inflated(&q.map(|v| f64::from(v as f32)), bounds))
                .collect();
            let Some(rect) = appearance::bounds(quads.iter().flatten().copied()) else {
                continue;
            };
            plans.push(Plan {
                page: index,
                rect,
                quads,
                search: Some(text.to_string()),
            });
        }
        if found.truncated || plans.len() > room {
            return Err(SearchError::TooMany);
        }
    }
    Ok(plans)
}
