// SPDX-License-Identifier: MIT
//! redact-inspect: what redact-apply would remove under each box, for the
//! window's "show what's underneath" tint. The boxes are area marks held in
//! memory (`Mark::area`), tested with apply's own rules over one walk of
//! each page (`Detail::Full`, `Limits::DEFAULT`) with one rule for all the
//! page's boxes, so the tint shows exactly what goes and the work is that
//! of one apply's plan, however many boxes there are:
//!
//! - glyphs `GlyphRule::removes_glyph` takes and runs that cannot be placed
//!   that `GlyphRule::reaches` (whole), each attributed to every box its
//!   box meets; what they spell, read from the view's characters whose
//!   centres lie in a removed glyph's quad;
//! - painted paths, shadings and pictures a box touches: the part under
//!   the box, or `whole` where apply removes the whole draw (inside the
//!   box, a singular matrix);
//! - annotations and form fields a box touches (apply removes them whole).
//!
//! The reply is bounded: at most `MAX_ITEMS` glyph quads, shapes and
//! pictures in all. Past it a box reports each show's removed run as one
//! quad, then only its own rect, and says `truncated`. Every number is
//! finite: a quad the walk could not place is reported as the box itself.
use std::collections::{BTreeMap, BTreeSet};

use crate::app::annots::PageSpace;
use crate::app::forms;
use crate::app::info;
use crate::document::Document;
use crate::object::{Dictionary, ObjectId, decode_text_string};
use crate::view::ViewDocument;

use super::apply::ApplyError;
use super::content::{Detail, DrawKind, Limits, WalkError};
use super::images;
use super::marks::Mark;
use super::region::{self, GlyphRule, Regions};

/// Characters of an annotation's label at most.
const MAX_LABEL_CHARS: usize = 80;
/// Glyph quads, shapes and pictures one reply lists at most.
pub const MAX_ITEMS: usize = 10_000;
/// Characters of one box's text at most.
pub const MAX_TEXT_CHARS: usize = 10_000;

/// A picture a box touches: the part under the box, and whether apply
/// removes the whole placement.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageHit {
    pub rect: [f64; 4],
    pub whole: bool,
}

/// A painted path or shading a box touches: the part under the box, and
/// whether apply removes the whole draw.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeHit {
    pub rect: [f64; 4],
    /// "path" or "shading".
    pub kind: &'static str,
    pub whole: bool,
}

/// An annotation or form field a box touches (apply removes it whole).
#[derive(Clone, Debug, PartialEq)]
pub struct AnnotHit {
    /// The annotation's /Subtype, lower case ("widget", "freetext", ...).
    pub kind: String,
    pub rect: [f64; 4],
    /// A widget's field name, else the start of its /Contents; may be empty.
    pub label: String,
}

/// What apply would remove under one box.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    /// Page-space quads (corners in order) of the glyphs that go; runs of
    /// adjacent upright glyphs of one show are joined into one quad.
    pub glyphs: Vec<[f64; 8]>,
    /// What those glyphs spell, as the view reads the page (lines joined
    /// with a newline), at most `MAX_TEXT_CHARS`.
    pub text: String,
    pub images: Vec<ImageHit>,
    pub shapes: Vec<ShapeHit>,
    pub annots: Vec<AnnotHit>,
    /// The box covers artwork apply cannot cut exactly (R4): applying
    /// needs the page flattened.
    pub needs_flatten: bool,
    /// The lists were cut to the reply's bound: `glyphs` holds one quad per
    /// show (or the box itself), `text` may be cut short.
    pub truncated: bool,
}

fn walk_error(page: usize, error: WalkError) -> ApplyError {
    match error {
        WalkError::TooComplex(_) => ApplyError::TooComplex(page),
        WalkError::Unreadable(why) => ApplyError::Unreadable(page, why),
    }
}

/// `a` cut to `b`.
fn meet(a: &[f64; 4], b: &[f64; 4]) -> [f64; 4] {
    [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ]
}

fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

fn finite(values: &[f64]) -> bool {
    values.iter().all(|v| v.is_finite())
}

fn rect_quad(r: [f64; 4]) -> [f64; 8] {
    [r[0], r[1], r[2], r[1], r[2], r[3], r[0], r[3]]
}

/// An upright quad (sides along the axes) as its rect.
fn upright(q: &[f64; 8]) -> Option<[f64; 4]> {
    let distinct = |v: [f64; 4]| {
        let mut v = v.to_vec();
        v.sort_by(f64::total_cmp);
        v.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        v
    };
    let dx = distinct([q[0], q[2], q[4], q[6]]);
    let dy = distinct([q[1], q[3], q[5], q[7]]);
    match (dx.as_slice(), dy.as_slice()) {
        ([x0, x1], [y0, y1]) => Some([*x0, *y0, *x1, *y1]),
        _ => None,
    }
}

/// Whether point `(x, y)` lies in convex quad `q` (corners in order, either
/// way round), edges included.
fn in_quad(q: &[f64; 8], x: f64, y: f64) -> bool {
    let mut sign = 0.0f64;
    for i in 0..4 {
        let (ax, ay) = (q[2 * i], q[2 * i + 1]);
        let (bx, by) = (q[(2 * i + 2) % 8], q[(2 * i + 3) % 8]);
        let cross = (bx - ax) * (y - ay) - (by - ay) * (x - ax);
        if cross.abs() < 1e-9 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

/// A removed glyph: its quad (finite), and the boxes it is attributed to.
struct Removed {
    quad: [f64; 8],
    bounds: [f64; 4],
    boxes: Vec<usize>,
}

/// One box's findings while a page is read.
#[derive(Default)]
struct Pending {
    found: Found,
    /// Removed glyph quads, with the show they are in, before joining.
    quads: Vec<([f64; 8], usize)>,
    /// One rect per show with removed glyphs (its removed glyphs' bounds).
    runs: Vec<[f64; 4]>,
}

/// The label of an annotation the inspector reports.
fn label(doc: &Document<'_>, id: Option<ObjectId>, dict: &Dictionary) -> String {
    let subtype = doc.dict_get(dict, b"Subtype").as_name().unwrap_or(b"");
    let raw = if subtype == b"Widget" {
        forms::full_name(doc, id, dict)
    } else {
        (doc.dict_get(dict, b"Contents").as_string())
            .map(decode_text_string)
            .unwrap_or_default()
    };
    let words: Vec<&str> = raw.split_whitespace().collect();
    words.join(" ").chars().take(MAX_LABEL_CHARS).collect()
}

/// The boxes (indices) whose rect meets `bounds`, edges included.
fn boxes_at(
    page: usize,
    index: &Regions,
    by_rect: &BTreeMap<[u64; 4], Vec<usize>>,
    bounds: &[f64; 4],
) -> Vec<usize> {
    let mut out: Vec<usize> = (index.meeting(page, bounds).iter())
        .filter_map(|r| by_rect.get(&r.map(f64::to_bits)))
        .flatten()
        .copied()
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// What the view's characters inside removed glyphs spell, per box: a
/// character goes to the boxes of each removed glyph whose quad holds its
/// centre (the walk's box holds the view's character).
fn spelled(
    view: &ViewDocument,
    page: usize,
    removed: &[Removed],
    boxes: usize,
) -> Vec<(String, bool)> {
    let mut out = vec![(String::new(), false); boxes];
    let Ok(text) = view.text_reader().structured_text(page) else {
        return out;
    };
    let index = Regions::from_rects(BTreeMap::from([(
        page,
        removed.iter().map(|r| r.bounds).collect(),
    )]));
    let mut by_bounds: BTreeMap<[u64; 4], Vec<usize>> = BTreeMap::new();
    for (i, r) in removed.iter().enumerate() {
        by_bounds
            .entry(r.bounds.map(f64::to_bits))
            .or_default()
            .push(i);
    }
    let owners = |q: &[f64; 8]| -> Vec<usize> {
        let x = (q[0] + q[2] + q[4] + q[6]) / 4.0;
        let y = (q[1] + q[3] + q[5] + q[7]) / 4.0;
        if !(x.is_finite() && y.is_finite()) {
            return Vec::new();
        }
        let mut owners: Vec<usize> = (index.meeting(page, &[x, y, x, y]).iter())
            .filter_map(|b| by_bounds.get(&b.map(f64::to_bits)))
            .flatten()
            .filter_map(|&i| removed.get(i))
            .filter(|r| in_quad(&r.quad, x, y))
            .flat_map(|r| r.boxes.iter().copied())
            .collect();
        owners.sort_unstable();
        owners.dedup();
        owners
    };
    for block in text.unclipped_blocks() {
        for line in &block.lines {
            let taken: Vec<Vec<usize>> = line.chars.iter().map(|c| owners(&c.quad)).collect();
            let mut wanted: BTreeSet<usize> = taken.iter().flatten().copied().collect();
            wanted.retain(|&b| b < boxes);
            for b in wanted {
                let Some((text, cut)) = out.get_mut(b) else {
                    continue;
                };
                let has = |i: usize| taken.get(i).is_some_and(|o| o.contains(&b));
                let mut piece = String::new();
                for (i, c) in line.chars.iter().enumerate() {
                    let between = c.synthetic && i > 0 && has(i - 1) && has(i + 1);
                    if has(i) || between {
                        piece.push(c.c);
                    }
                }
                let piece = piece.trim();
                if piece.is_empty() || *cut {
                    continue;
                }
                if !text.is_empty() {
                    text.push('\n');
                }
                let room = MAX_TEXT_CHARS.saturating_sub(text.chars().count());
                if piece.chars().count() > room {
                    text.extend(piece.chars().take(room));
                    *cut = true;
                } else {
                    text.push_str(piece);
                }
            }
        }
    }
    out
}

/// Adds a shape or picture hit to each box `bounds` touches, within the
/// reply's bound.
fn add_hit(
    pending: &mut [Pending],
    boxes: &[Mark],
    owners: Vec<usize>,
    bounds: &[f64; 4],
    items: &mut usize,
    mut add: impl FnMut(&mut Found, [f64; 4]),
) {
    for o in owners {
        let (Some(p), Some(mark)) = (pending.get_mut(o), boxes.get(o)) else {
            continue;
        };
        if !region::intersects(&mark.rect, bounds) {
            continue;
        }
        if *items >= MAX_ITEMS {
            p.found.truncated = true;
            continue;
        }
        *items += 1;
        let part = meet(bounds, &mark.rect);
        add(&mut p.found, if finite(&part) { part } else { mark.rect });
    }
}

/// What apply would remove under each of `boxes` (area marks), in their
/// order. One walk per page and one rule for its boxes.
#[allow(clippy::too_many_lines)]
pub fn inspect(
    doc: &Document<'static>,
    view: Option<&ViewDocument>,
    boxes: &[Mark],
) -> Result<Vec<Found>, ApplyError> {
    let mut pending: Vec<Pending> = (0..boxes.len()).map(|_| Pending::default()).collect();
    let refs = info::pages(doc);
    let pages: BTreeSet<usize> = boxes.iter().map(|b| b.page).collect();
    // Glyph quads, shapes and pictures kept so far, in all.
    let mut items = 0usize;
    for page in pages {
        let Some(page_ref) = refs.get(page) else {
            continue;
        };
        let area = PageSpace::of(doc, page_ref.dict).bounds();
        let marks: Vec<Mark> = boxes.iter().filter(|b| b.page == page).cloned().collect();
        let rule = GlyphRule::with_boxes(&marks, BTreeMap::from([(page, area)]));
        let regions = rule.drawn();
        let mut by_rect: BTreeMap<[u64; 4], Vec<usize>> = BTreeMap::new();
        for (i, b) in boxes.iter().enumerate().filter(|(_, b)| b.page == page) {
            by_rect.entry(b.rect.map(f64::to_bits)).or_default().push(i);
        }
        let everyone: Vec<usize> = by_rect.values().flatten().copied().collect();
        // Each box's own rule, to say which boxes take a glyph the page's
        // rule removes (a glyph one box takes may only graze another).
        let own: BTreeMap<usize, GlyphRule> = (everyone.iter())
            .filter_map(|&b| {
                let mark = boxes.get(b)?;
                let rule = GlyphRule::with_boxes(
                    std::slice::from_ref(mark),
                    BTreeMap::from([(page, area)]),
                );
                Some((b, rule))
            })
            .collect();
        let mut attributed = 0usize;
        let content = region::page_content(doc, view, page, Detail::Full, Limits::DEFAULT)
            .map_err(|e| walk_error(page, e))?;
        let mut removed: Vec<Removed> = Vec::new();
        for (index, draw) in content.draws.iter().enumerate() {
            if rule.spent() {
                return Err(ApplyError::TooComplex(page));
            }
            let touched = regions.touches(page, &draw.bounds);
            match &draw.kind {
                DrawKind::Text(text) if text.measured => {
                    // Per box: the removed glyphs' bounds in this show.
                    let mut runs: BTreeMap<usize, [f64; 4]> = BTreeMap::new();
                    for g in &text.glyphs {
                        if rule.spent() {
                            return Err(ApplyError::TooComplex(page));
                        }
                        if !rule.removes_glyph(page, &g.bounds, &g.quad) {
                            continue;
                        }
                        let quad = g.quad.map(f64::from);
                        let (quad, bounds) = if finite(&quad) && finite(&g.bounds) {
                            (quad, g.bounds)
                        } else {
                            let b = rule.onto_page(page, &g.bounds);
                            let b = if finite(&b) { b } else { area };
                            (rect_quad(b), b)
                        };
                        let mut owners = boxes_at(page, regions, &by_rect, &g.bounds);
                        attributed = attributed.saturating_add(owners.len());
                        if attributed > region::MAX_QUERY_VISITS {
                            return Err(ApplyError::TooComplex(page));
                        }
                        owners.retain(|b| {
                            own.get(b)
                                .is_some_and(|r| r.removes_glyph(page, &g.bounds, &g.quad))
                        });
                        if owners.is_empty() {
                            // A glyph with no position goes with every box
                            // of its page.
                            owners.clone_from(&everyone);
                        }
                        for &b in &owners {
                            let Some(p) = pending.get_mut(b) else {
                                continue;
                            };
                            (runs.entry(b))
                                .and_modify(|r| *r = union(*r, bounds))
                                .or_insert(bounds);
                            if items < MAX_ITEMS {
                                p.quads.push((quad, index));
                                items += 1;
                            } else {
                                p.found.truncated = true;
                            }
                        }
                        removed.push(Removed {
                            quad,
                            bounds,
                            boxes: owners,
                        });
                    }
                    for (b, run) in runs {
                        if let Some(p) = pending.get_mut(b) {
                            p.runs.push(run);
                        }
                    }
                }
                DrawKind::Text(_) => {
                    if rule.reaches(page, &draw.bounds) {
                        let b = rule.onto_page(page, &draw.bounds);
                        let b = if finite(&b) { b } else { area };
                        let mut owners = boxes_at(page, regions, &by_rect, &b);
                        attributed = attributed.saturating_add(owners.len());
                        if attributed > region::MAX_QUERY_VISITS {
                            return Err(ApplyError::TooComplex(page));
                        }
                        owners
                            .retain(|o| own.get(o).is_some_and(|r| r.reaches(page, &draw.bounds)));
                        if owners.is_empty() {
                            owners.clone_from(&everyone);
                        }
                        for &o in &owners {
                            if let Some(p) = pending.get_mut(o) {
                                if items < MAX_ITEMS {
                                    p.quads.push((rect_quad(b), index));
                                    items += 1;
                                } else {
                                    p.found.truncated = true;
                                }
                                p.runs.push(b);
                            }
                        }
                        removed.push(Removed {
                            quad: rect_quad(b),
                            bounds: b,
                            boxes: owners,
                        });
                    }
                }
                DrawKind::Path { fill, stroke } if touched && (*fill || *stroke) => {
                    let det = draw.ctm[0] * draw.ctm[3] - draw.ctm[1] * draw.ctm[2];
                    let whole =
                        regions.contains(page, &draw.bounds) || det == 0.0 || !det.is_finite();
                    let owners = boxes_at(page, regions, &by_rect, &draw.bounds);
                    add_hit(
                        &mut pending,
                        boxes,
                        owners,
                        &draw.bounds,
                        &mut items,
                        |f, rect| {
                            f.shapes.push(ShapeHit {
                                rect,
                                kind: "path",
                                whole,
                            });
                        },
                    );
                }
                DrawKind::Shading if touched => {
                    let det = draw.ctm[0] * draw.ctm[3] - draw.ctm[1] * draw.ctm[2];
                    let whole = det == 0.0 || !det.is_finite();
                    let owners = boxes_at(page, regions, &by_rect, &draw.bounds);
                    add_hit(
                        &mut pending,
                        boxes,
                        owners,
                        &draw.bounds,
                        &mut items,
                        |f, rect| {
                            f.shapes.push(ShapeHit {
                                rect,
                                kind: "shading",
                                whole,
                            });
                        },
                    );
                }
                DrawKind::Image { matrix, .. } if touched => {
                    let rects = regions.touching(page, &draw.bounds);
                    let whole = regions.contains(page, &draw.bounds)
                        || images::unit_regions(&rects, matrix).is_none();
                    let owners = boxes_at(page, regions, &by_rect, &draw.bounds);
                    add_hit(
                        &mut pending,
                        boxes,
                        owners,
                        &draw.bounds,
                        &mut items,
                        |f, rect| {
                            f.images.push(ImageHit { rect, whole });
                        },
                    );
                }
                _ => {}
            }
        }
        // The text, read once per page for all its boxes.
        if let (Some(view), false) = (view, removed.is_empty()) {
            let texts = spelled(view, page, &removed, boxes.len());
            for (b, (text, cut)) in texts.into_iter().enumerate() {
                if boxes.get(b).is_some_and(|m| m.page == page) {
                    if let Some(p) = pending.get_mut(b) {
                        p.found.text = text;
                        p.found.truncated |= cut;
                    }
                }
            }
        }
        for (annot, dict) in region::annotation_entries(doc, page, page_ref.dict) {
            for o in boxes_at(page, regions, &by_rect, &annot.rect) {
                let (Some(p), Some(mark)) = (pending.get_mut(o), boxes.get(o)) else {
                    continue;
                };
                if !region::intersects(&mark.rect, &annot.rect) {
                    continue;
                }
                let rect = if finite(&annot.rect) {
                    annot.rect
                } else {
                    mark.rect
                };
                p.found.annots.push(AnnotHit {
                    kind: String::from_utf8_lossy(&annot.subtype).to_lowercase(),
                    rect,
                    label: label(doc, annot.id, dict),
                });
            }
        }
    }
    // Glyph quads: joined where adjacent; a box past the bound lists one
    // quad per show, or only its own rect.
    let mut room = MAX_ITEMS;
    let out = (pending.into_iter().zip(boxes))
        .map(|(mut p, mark)| {
            if p.found.truncated {
                if p.runs.len() <= room {
                    room -= p.runs.len();
                    p.found.glyphs = p.runs.iter().map(|r| rect_quad(*r)).collect();
                } else {
                    p.found.glyphs = vec![rect_quad(mark.rect)];
                }
            } else {
                let mut last = None;
                for (quad, draw) in p.quads {
                    push_glyph(&mut p.found.glyphs, quad, last == Some(draw));
                    last = Some(draw);
                }
            }
            p.found
        })
        .collect();
    Ok(out)
}

/// Appends a glyph's quad, joined to the last one when both are upright,
/// share their top and bottom and touch along the line.
fn push_glyph(out: &mut Vec<[f64; 8]>, quad: [f64; 8], join: bool) {
    if join {
        if let (Some(last), Some(next)) = (out.last().and_then(upright), upright(&quad)) {
            let same_line = (last[1] - next[1]).abs() < 0.01 && (last[3] - next[3]).abs() < 0.01;
            if same_line && (next[0] - last[2]).abs() < 0.5 {
                if let Some(slot) = out.last_mut() {
                    *slot = rect_quad([last[0], last[1], next[2], last[3]]);
                }
                return;
            }
        }
    }
    out.push(quad);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_in_turned_quads() {
        // A unit square turned 45 degrees about the origin.
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let q = [0.0, 0.0, h, h, 0.0, 2.0 * h, -h, h];
        assert!(in_quad(&q, 0.0, h));
        assert!(!in_quad(&q, 0.6, 0.1));
        assert!(!in_quad(&q, -0.6, 0.1));
        let r = rect_quad([0.0, 0.0, 2.0, 1.0]);
        assert!(in_quad(&r, 1.0, 0.5) && in_quad(&r, 2.0, 1.0) && !in_quad(&r, 2.1, 0.5));
    }
}
