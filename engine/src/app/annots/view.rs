// SPDX-License-Identifier: AGPL-3.0-or-later
//! What the view draws for annotations that hayro, which draws stored
//! appearance streams mapped onto /Rect, would draw otherwise. In the view's
//! private bytes only (`app::view_fixes`), never journaled or saved:
//!
//! - An annotation of a kind the view synthesizes that has no appearance
//!   gets the one `appearance` draws, for display only (a save writes it
//!   only once the annotation is edited).
//! - A Text annotation, or one flagged NoZoom, is drawn at the size of its
//!   appearance's /BBox (whatever /Matrix scales) with the top left corner
//!   of /Rect fixed; a Text annotation, or one flagged NoRotate, stays
//!   upright on a rotated page, turning about that corner (ISO 32000-2
//!   12.5.3; reference viewers do both).
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};

use super::appearance::{self, Form, Mark, Style};
use super::{Color, PageSpace, Point, concat};

/// Bounds on what one view synthesizes: appearances made and content bytes.
const MAX_SYNTHESIZED: usize = 100_000;
const MAX_CONTENT_BYTES: usize = 32 * 1024 * 1024;
/// Bounds on one synthesized appearance: the quads and points it draws.
const MAX_QUADS: usize = 16_384;
const MAX_POINTS: usize = 131_072;
/// The rect MuPDF gives a note when it draws its icon.
const NOTE_RECT: f64 = 20.0;

const NO_ZOOM: i64 = 8;
const NO_ROTATE: i64 = 16;
/// How far (in points) hayro may misplace an appearance before the view
/// places it itself.
const PLACEMENT_TOLERANCE: f64 = 1e-3;

/// An appearance made for an annotation from its dictionary, with the
/// /Rect and the /RD (when it sets one) MuPDF's update writes with it.
#[derive(Clone, Debug, PartialEq)]
pub struct Synthesized {
    pub form: Form,
    pub rect: [f64; 4],
    pub rd: Option<[f64; 4]>,
}

/// The /Rect, and /RD when it sets one, MuPDF's synthesis gives an
/// annotation whose box is its /Rect `outer`: a note's 20 x 20 at the top
/// left, a stamp's 190:50 fitted inside it about the centre, a caret's
/// 20 x 14 about the centre, a Square's or Circle's /RD of half its border
/// when it has none, a Redact mark's quads grown by a point. None for kinds
/// whose box is their geometry or that are not synthesized.
fn rect_box(
    doc: &Document<'_>,
    annot: &Dictionary,
    kind: &[u8],
    outer: [f64; 4],
) -> Option<([f64; 4], Option<[f64; 4]>)> {
    let [x0, y0, x1, y1] = outer;
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    match kind {
        b"Text" => Some(([x0, y1 - NOTE_RECT, x0 + NOTE_RECT, y1], None)),
        b"Stamp" => Some((appearance::icon_fit(outer), None)),
        b"Caret" => Some(([cx - 10.0, cy - 7.0, cx + 10.0, cy + 7.0], None)),
        b"Square" | b"Circle" => {
            let half = super::border_width(doc, annot).max(0.0) / 2.0;
            let rd = super::rect_differences(doc, annot)
                .is_none()
                .then_some([half; 4]);
            Some((outer, rd))
        }
        b"FreeText" => Some((outer, None)),
        b"Redact" => {
            let quads = super::quads(doc, annot);
            let all = appearance::bounds(quads.into_iter().take(MAX_QUADS).flatten());
            Some((all.map_or(outer, |all| appearance::grow(all, 1.0)), None))
        }
        _ => None,
    }
}

/// For an annotation the view synthesizes an appearance for, the /Rect and
/// /RD that synthesis gives it (`rect_box`), which MuPDF lists; None when it
/// has an appearance or its box is its geometry.
pub fn synthesized_box(
    doc: &Document<'_>,
    annot: &Dictionary,
    outer: [f64; 4],
) -> Option<([f64; 4], Option<[f64; 4]>)> {
    if normal_appearance(doc, annot).is_some() {
        return None;
    }
    let kind = doc.dict_get(annot, b"Subtype").as_name()?;
    rect_box(doc, annot, kind, outer)
}

/// The appearance MuPDF synthesizes for `annot` from its entries, for the
/// kinds the app draws: Text, FreeText, the four text markups, Square,
/// Circle, Line, Polygon, PolyLine, Ink, icon Stamps, Carets and Redact
/// marks. At most 16,384 quads and 131,072 points are drawn.
pub fn synthesize(doc: &Document<'_>, annot: &Dictionary) -> Option<Synthesized> {
    let kind = doc.dict_get(annot, b"Subtype").as_name()?;
    let rect = annot
        .get(b"Rect".as_slice())
        .and_then(|r| super::rect(doc, r));
    let boxed = rect.and_then(|outer| rect_box(doc, annot, kind, outer));
    let color = annot.get(b"C".as_slice()).and_then(|c| Color::read(doc, c));
    let alpha = super::opacity(doc, annot);
    let style = || Style {
        stroke: color.clone(),
        fill: annot
            .get(b"IC".as_slice())
            .and_then(|c| Color::read(doc, c)),
        width: super::border_width(doc, annot).max(0.0),
        opacity: alpha,
        dash: super::border_dash(doc, annot),
    };
    let made = |form: Form, rect: [f64; 4]| {
        Some(Synthesized {
            form,
            rect,
            rd: None,
        })
    };
    let quads = || {
        let mut quads = super::quads(doc, annot);
        quads.truncate(MAX_QUADS);
        quads
    };
    match kind {
        b"Text" => {
            let icon = doc.dict_get(annot, b"Name").as_name().unwrap_or(b"Note");
            made(appearance::note(icon, color.as_ref(), alpha), boxed?.0)
        }
        b"FreeText" => {
            let text = super::text(doc, annot, b"Contents");
            let form = crate::app::ops::annots::free_text_form(doc, annot, rect?, &text);
            made(form, rect?)
        }
        b"Highlight" | b"Underline" | b"StrikeOut" | b"Squiggly" => {
            let quads = quads();
            if quads.is_empty() {
                return None;
            }
            let (form, rect) = match kind {
                b"Highlight" => appearance::highlight(&quads, color.as_ref(), alpha),
                b"Underline" => {
                    appearance::text_line(Mark::Underline, &quads, color.as_ref(), alpha)
                }
                b"StrikeOut" => {
                    appearance::text_line(Mark::StrikeOut, &quads, color.as_ref(), alpha)
                }
                _ => appearance::text_line(Mark::Squiggly, &quads, color.as_ref(), alpha),
            };
            made(form, rect)
        }
        b"Square" | b"Circle" => {
            let (outer, rd) = boxed?;
            let style = style();
            let inner = super::inset(outer, rd.or_else(|| super::rect_differences(doc, annot)));
            let form = appearance::rectangle(inner, outer, kind == b"Circle", &style);
            Some(Synthesized {
                form,
                rect: outer,
                rd,
            })
        }
        b"Line" => {
            let [a, b] = super::line_points(doc, annot)?;
            let endings = doc.dict_get(annot, b"LE").as_array().unwrap_or(&[]);
            let ending = |i: usize| {
                endings
                    .get(i)
                    .and_then(|e| doc.resolve(e).as_name())
                    .unwrap_or(b"None")
            };
            let (form, rect) = appearance::line(a, b, [ending(0), ending(1)], &style());
            made(form, rect)
        }
        b"Polygon" | b"PolyLine" => {
            let mut vertices = super::points(doc, annot.get(b"Vertices".as_slice())?);
            vertices.truncate(MAX_POINTS);
            if vertices.is_empty() {
                return None;
            }
            let (form, rect) = appearance::poly(&vertices, kind == b"Polygon", &style());
            made(form, rect)
        }
        b"Ink" => {
            let mut left = MAX_POINTS;
            let strokes: Vec<Vec<Point>> = super::ink_list(doc, annot)
                .into_iter()
                .map(|mut stroke| {
                    stroke.truncate(left);
                    left -= stroke.len();
                    stroke
                })
                .filter(|stroke| !stroke.is_empty())
                .collect();
            if strokes.is_empty() {
                return None;
            }
            let style = style();
            let (form, rect) = appearance::ink(&strokes, &style);
            let rd = appearance::ink_allowance(style.width);
            Some(Synthesized {
                form,
                rect,
                rd: Some([rd; 4]),
            })
        }
        b"Stamp" => {
            let name = doc.dict_get(annot, b"Name").as_name().unwrap_or(b"Draft");
            let form = appearance::stamp(name, color.as_ref(), alpha, appearance::IDENTITY);
            made(form, boxed?.0)
        }
        b"Caret" => made(appearance::caret(color.as_ref(), alpha), boxed?.0),
        b"Redact" => {
            let (form, rect) = appearance::redact(rect?, &quads(), color.as_ref());
            made(form, rect)
        }
        _ => None,
    }
}

/// The view's changes to one annotation: its new value (with /AP and
/// /Rect replaced) and the streams it needs, numbered from `next`.
pub(crate) struct Fix {
    pub value: Dictionary,
    pub streams: Vec<(ObjectId, Dictionary, Vec<u8>)>,
}

/// Running totals that bound one view's work.
#[derive(Default)]
pub(crate) struct Budget {
    made: usize,
    bytes: usize,
}

/// What the view draws for `annot` on a page laid out by `space`, when it
/// differs from what hayro would draw from the stored dictionary; None
/// otherwise. `next` gives fresh object numbers for new streams.
pub(crate) fn fix(
    doc: &Document<'_>,
    annot: &Dictionary,
    space: &PageSpace,
    budget: &mut Budget,
    next: &mut dyn FnMut() -> Option<ObjectId>,
) -> Option<Fix> {
    let kind = doc.dict_get(annot, b"Subtype").as_name()?;
    if kind == b"Popup" || kind == b"Link" || kind == b"Widget" {
        return None;
    }
    let flags = match doc.dict_get(annot, b"F") {
        Object::Integer(flags) if u32::try_from(*flags).is_ok() => *flags,
        _ => 0,
    };
    let mut value = annot.clone();
    let mut streams = Vec::new();
    let mut changed = false;
    // The normal appearance hayro would draw: a stream, or the /AS state.
    let mut appearance = normal_appearance(doc, annot);
    if appearance.is_none() {
        if budget.made >= MAX_SYNTHESIZED || budget.bytes >= MAX_CONTENT_BYTES {
            return None;
        }
        let made = synthesize(doc, annot)?;
        let id = next()?;
        budget.made += 1;
        budget.bytes = budget.bytes.saturating_add(made.form.content.len());
        let mut ap = Dictionary::new();
        ap.insert(b"N".to_vec(), Object::Reference(id));
        value.insert(b"AP".to_vec(), Object::Dictionary(ap));
        value.insert(b"Rect".to_vec(), appearance::numbers(&made.rect));
        if let Some(rd) = made.rd {
            value.insert(b"RD".to_vec(), appearance::numbers(&rd));
        }
        let bbox = made.form.bbox;
        let matrix = made.form.matrix;
        streams.push((id, made.form.dictionary(), made.form.content));
        appearance = Some((id, bbox, matrix));
        changed = true;
    }
    let text = kind == b"Text";
    let fixed_size = text || flags & NO_ZOOM != 0;
    let upright = (text || flags & NO_ROTATE != 0) && space.rotate != 0;
    let Some((id, bbox, matrix)) = appearance else {
        return changed.then_some(Fix { value, streams });
    };
    let Some(rect) = value
        .get(b"Rect".as_slice())
        .and_then(|r| super::rect(doc, r))
    else {
        return changed.then_some(Fix { value, streams });
    };
    let shown = super::transform_rect(&matrix, bbox);
    let (bw, bh) = (shown[2] - shown[0], shown[3] - shown[1]);
    if !(bw > 0.0 && bh > 0.0) {
        return changed.then_some(Fix { value, streams });
    }
    let (sx, sy) = if fixed_size {
        // Drawn at its /BBox's own size, whatever /Matrix scales.
        (
            (bbox[2] - bbox[0]).abs() / bw,
            (bbox[3] - bbox[1]).abs() / bh,
        )
    } else {
        ((rect[2] - rect[0]) / bw, (rect[3] - rect[1]) / bh)
    };
    // hayro maps the transformed box onto /Rect with the translation left
    // unscaled (rect.x0 - box.x0, ISO 32000-2 12.5.5 has box.x0 scaled), so
    // a box off the origin drawn at another scale (a turned stamp, a
    // foreign /Matrix) lands displaced: the view then places it itself.
    let misplaced = (shown[0] * (sx - 1.0)).abs() > PLACEMENT_TOLERANCE
        || (shown[1] * (sy - 1.0)).abs() > PLACEMENT_TOLERANCE;
    if !(fixed_size || upright || misplaced) {
        return changed.then_some(Fix { value, streams });
    }
    // The appearance box's top left onto /Rect's, scaled, then turned
    // about that corner against the page rotation.
    let place = [
        sx,
        0.0,
        0.0,
        sy,
        rect[0] - shown[0] * sx,
        rect[3] - shown[3] * sy,
    ];
    let corner = (rect[0], rect[3]);
    let turn = if upright {
        space.upright()
    } else {
        appearance::IDENTITY
    };
    let about = concat(
        &concat(&[1.0, 0.0, 0.0, 1.0, -corner.0, -corner.1], &turn),
        &[1.0, 0.0, 0.0, 1.0, corner.0, corner.1],
    );
    let to_user = concat(&place, &about);
    let drawn = super::transform_rect(&to_user, shown);
    if upright || misplaced {
        // A wrapper form, drawn at scale 1 where hayro places it right,
        // invokes the appearance under the whole placement.
        let wrapper = next()?;
        let mut xobjects = Dictionary::new();
        xobjects.insert(b"A".to_vec(), Object::Reference(id));
        let mut resources = Dictionary::new();
        resources.insert(b"XObject".to_vec(), Object::Dictionary(xobjects));
        let mut content = b"q\n".to_vec();
        for v in to_user {
            content.extend_from_slice(crate::serialize::compact_real(v).as_bytes());
            content.push(b' ');
        }
        content.extend_from_slice(b"cm\n/A Do\nQ\n");
        let form = Form {
            bbox: drawn,
            matrix: appearance::IDENTITY,
            resources,
            content,
        };
        let mut ap = Dictionary::new();
        ap.insert(b"N".to_vec(), Object::Reference(wrapper));
        value.insert(b"AP".to_vec(), Object::Dictionary(ap));
        value.remove(b"AS".as_slice());
        streams.push((wrapper, form.dictionary(), form.content));
    }
    value.insert(b"Rect".to_vec(), appearance::numbers(&drawn));
    Some(Fix { value, streams })
}

/// The stream hayro draws for an annotation (its /AP /N, or the /AS state
/// of it), with that form's /BBox and /Matrix; None without one.
fn normal_appearance(
    doc: &Document<'_>,
    annot: &Dictionary,
) -> Option<(ObjectId, [f64; 4], [f64; 6])> {
    let ap = doc.dict_get(annot, b"AP").as_dict()?;
    let normal = ap.get(b"N".as_slice())?;
    let (holder, value) = doc.resolve_with_holder(normal);
    let id = match (holder, value) {
        (Some(id), _) if is_stream(doc, id) => id,
        (_, Object::Dictionary(states)) => {
            let state = doc.dict_get(annot, b"AS").as_name().unwrap_or(b"Off");
            let chosen = states
                .get(state)
                .or_else(|| states.get(b"Off".as_slice()))?;
            let (holder, _) = doc.resolve_with_holder(chosen);
            holder.filter(|&id| is_stream(doc, id))?
        }
        _ => return None,
    };
    let form = doc.resolve_id(id).as_dict()?;
    let bbox = form
        .get(b"BBox".as_slice())
        .and_then(|b| super::rect(doc, b))?;
    let matrix = match form
        .get(b"Matrix".as_slice())
        .and_then(|m| super::numbers(doc, m))
    {
        Some(m) if m.len() == 6 => [m[0], m[1], m[2], m[3], m[4], m[5]],
        _ => appearance::IDENTITY,
    };
    Some((id, bbox, matrix))
}

fn is_stream(doc: &Document<'_>, id: ObjectId) -> bool {
    doc.object(id).is_ok_and(|object| object.stream.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbers(values: impl IntoIterator<Item = f64>) -> Object {
        Object::Array(values.into_iter().map(Object::Real).collect())
    }

    #[test]
    fn one_synthesized_appearance_draws_bounded_quads_and_points() {
        let doc = crate::app::edit::Builder::new("1.7")
            .unwrap()
            .into_document();
        let mut annot = Dictionary::new();
        annot.insert(b"Subtype".to_vec(), Object::name("Highlight"));
        let quad = [0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0];
        annot.insert(
            b"QuadPoints".to_vec(),
            numbers((0..MAX_QUADS + 500).flat_map(|_| quad)),
        );
        let made = synthesize(&doc, &annot).unwrap();
        let text = String::from_utf8(made.form.content).unwrap();
        assert_eq!(text.matches("f\n").count(), MAX_QUADS);

        let mut ink = Dictionary::new();
        ink.insert(b"Subtype".to_vec(), Object::name("Ink"));
        let stroke = || numbers((0..MAX_POINTS).flat_map(|i| [i as f64, 1.0]));
        ink.insert(b"InkList".to_vec(), Object::Array(vec![stroke(), stroke()]));
        let made = synthesize(&doc, &ink).unwrap();
        let text = String::from_utf8(made.form.content).unwrap();
        assert_eq!(
            text.matches(" m\n").count() + text.matches(" l\n").count(),
            MAX_POINTS
        );
    }
}
