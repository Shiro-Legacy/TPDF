// SPDX-License-Identifier: MIT
//! redact-inspect: what redact-apply would remove under each box, for the
//! window's "show what's underneath" tint. Each box is an area mark held in
//! memory (`Mark::area`), tested with apply's own rules (`GlyphRule`,
//! `Regions::touches`) over one walk of its page, so the tint shows exactly
//! what goes: the glyphs (`GlyphRule::removes_glyph`, R1), runs that cannot
//! be placed (`GlyphRule::reaches`, whole), the pictures, shapes and
//! shadings a box touches (cut to the box; a picture wholly inside goes),
//! and the annotations and form fields it touches (whole). Nothing is
//! written; the walk is bounded by `Limits::DEFAULT` as apply's is.
use std::collections::{BTreeMap, BTreeSet};

use crate::app::forms;
use crate::app::info;
use crate::document::Document;
use crate::object::{Dictionary, ObjectId, decode_text_string};
use crate::view::ViewDocument;

use super::apply::ApplyError;
use super::content::{Detail, DrawKind, Limits, WalkError};
use super::marks::Mark;
use super::region::{self, GlyphRule, Regions};

/// Characters of an annotation's label at most.
const MAX_LABEL_CHARS: usize = 80;

/// A picture a box touches: the part under the box, and whether the whole
/// placement lies inside it (apply then stops drawing it).
#[derive(Clone, Debug, PartialEq)]
pub struct ImageHit {
    pub rect: [f64; 4],
    pub whole: bool,
}

/// A painted path or shading a box touches: the part under the box.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeHit {
    pub rect: [f64; 4],
    /// "path" or "shading".
    pub kind: &'static str,
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
    /// with a newline).
    pub text: String,
    pub images: Vec<ImageHit>,
    pub shapes: Vec<ShapeHit>,
    pub annots: Vec<AnnotHit>,
    /// The box covers artwork apply cannot cut exactly (R4): applying
    /// needs the page flattened.
    pub needs_flatten: bool,
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

/// A quad's corners as numbers, each kept to a hundredth of a point.
fn quad_numbers(quad: &[f32; 8]) -> [f64; 8] {
    quad.map(|v| (f64::from(v) * 100.0).round() / 100.0)
}

/// An upright quad (sides along the axes) as its rect.
fn upright(q: &[f64; 8]) -> Option<[f64; 4]> {
    let xs = [q[0], q[2], q[4], q[6]];
    let ys = [q[1], q[3], q[5], q[7]];
    let distinct = |v: [f64; 4]| {
        let mut v = v.to_vec();
        v.sort_by(f64::total_cmp);
        v.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        v
    };
    let (dx, dy) = (distinct(xs), distinct(ys));
    match (dx.as_slice(), dy.as_slice()) {
        ([x0, x1], [y0, y1]) => Some([*x0, *y0, *x1, *y1]),
        _ => None,
    }
}

fn rect_quad(r: [f64; 4]) -> [f64; 8] {
    [r[0], r[1], r[2], r[1], r[2], r[3], r[0], r[3]]
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

/// What the characters the view reads inside `removed` (the boxes of the
/// glyphs that go, by page) spell: a character goes when its centre lies in
/// such a box (the walk's box holds the view's character).
fn spelled(view: &ViewDocument, page: usize, removed: &Regions) -> String {
    let Ok(text) = view.text_reader().structured_text(page) else {
        return String::new();
    };
    let inside = |quad: &[f64; 8]| {
        let x = (quad[0] + quad[2] + quad[4] + quad[6]) / 4.0;
        let y = (quad[1] + quad[3] + quad[5] + quad[7]) / 4.0;
        x.is_finite() && y.is_finite() && removed.meets(page, &[x, y, x, y])
    };
    let mut lines: Vec<String> = Vec::new();
    for block in text.unclipped_blocks() {
        for line in &block.lines {
            let taken: Vec<bool> = line.chars.iter().map(|c| inside(&c.quad)).collect();
            let mut out = String::new();
            for (i, c) in line.chars.iter().enumerate() {
                let keep = taken.get(i) == Some(&true)
                    || (c.synthetic
                        && i > 0
                        && taken.get(i - 1) == Some(&true)
                        && taken.get(i + 1) == Some(&true));
                if keep {
                    out.push(c.c);
                }
            }
            let out = out.trim().to_string();
            if !out.is_empty() {
                lines.push(out);
            }
        }
    }
    lines.join("\n")
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

/// What apply would remove under each of `boxes` (area marks), in their
/// order. One walk per page; each box is tested on its own.
pub fn inspect(
    doc: &Document<'static>,
    view: Option<&ViewDocument>,
    boxes: &[Mark],
) -> Result<Vec<Found>, ApplyError> {
    let mut out: Vec<Found> = vec![Found::default(); boxes.len()];
    let pages: BTreeSet<usize> = boxes.iter().map(|b| b.page).collect();
    let refs = info::pages(doc);
    for page in pages {
        let content = region::page_content(doc, view, page, Detail::Full, Limits::DEFAULT)
            .map_err(|e| walk_error(page, e))?;
        let entries = match refs.get(page) {
            Some(p) => region::annotation_entries(doc, page, p.dict),
            None => Vec::new(),
        };
        for (index, mark) in boxes.iter().enumerate().filter(|(_, b)| b.page == page) {
            let rule = GlyphRule::new(doc, std::slice::from_ref(mark));
            let regions = rule.drawn();
            let area = rule.area(page).unwrap_or(mark.rect);
            let under = meet(&mark.rect, &area);
            let found = &mut out[index];
            let mut removed: Vec<[f64; 4]> = Vec::new();
            for draw in &content.draws {
                if rule.spent() {
                    return Err(ApplyError::TooComplex(page));
                }
                let touched = regions.touches(page, &draw.bounds);
                match &draw.kind {
                    DrawKind::Text(text) if text.measured => {
                        let mut previous: Option<usize> = None;
                        for (i, g) in text.glyphs.iter().enumerate() {
                            if rule.spent() {
                                return Err(ApplyError::TooComplex(page));
                            }
                            if rule.removes_glyph(page, &g.bounds, &g.quad) {
                                let join = previous == Some(i.wrapping_sub(1));
                                push_glyph(&mut found.glyphs, quad_numbers(&g.quad), join);
                                removed.push(g.bounds);
                                previous = Some(i);
                            }
                        }
                    }
                    DrawKind::Text(_) => {
                        if rule.reaches(page, &draw.bounds) {
                            let b = rule.onto_page(page, &draw.bounds);
                            found.glyphs.push(rect_quad(b));
                            removed.push(draw.bounds);
                        }
                    }
                    DrawKind::Path { fill, stroke } if touched && (*fill || *stroke) => {
                        found.shapes.push(ShapeHit {
                            rect: meet(&draw.bounds, &under),
                            kind: "path",
                        });
                    }
                    DrawKind::Shading if touched => {
                        found.shapes.push(ShapeHit {
                            rect: meet(&draw.bounds, &under),
                            kind: "shading",
                        });
                    }
                    DrawKind::Image { .. } if touched => {
                        found.images.push(ImageHit {
                            rect: meet(&draw.bounds, &under),
                            whole: regions.contains(page, &draw.bounds),
                        });
                    }
                    _ => {}
                }
            }
            if let (Some(view), false) = (view, removed.is_empty()) {
                let index = Regions::from_rects(BTreeMap::from([(page, removed)]));
                found.text = spelled(view, page, &index);
            }
            for (annot, dict) in &entries {
                if regions.touches(page, &annot.rect) {
                    found.annots.push(AnnotHit {
                        kind: String::from_utf8_lossy(&annot.subtype).to_lowercase(),
                        rect: annot.rect,
                        label: label(doc, annot.id, dict),
                    });
                }
            }
        }
    }
    Ok(out)
}
