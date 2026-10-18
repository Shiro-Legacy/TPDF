// SPDX-License-Identifier: MIT
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
use crate::app::ops::annots::NOTE_SIZE;
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};

use super::appearance::{self, Form, Icon, Mark, Style};
use super::{Color, PageSpace, Quad, concat};

/// Bounds on what one view synthesizes: appearances made and content bytes.
const MAX_SYNTHESIZED: usize = 100_000;
const MAX_CONTENT_BYTES: usize = 32 * 1024 * 1024;
const NO_ZOOM: i64 = 8;
const NO_ROTATE: i64 = 16;
/// How far (in points) hayro may misplace an appearance before the view
/// places it itself.
const PLACEMENT_TOLERANCE: f64 = 1e-3;

/// An appearance made for an annotation from its dictionary, with the
/// /Rect and (when it sets one) /RD that go with it.
#[derive(Clone, Debug, PartialEq)]
pub struct Synthesized {
    pub form: Form,
    pub rect: [f64; 4],
    pub rd: Option<[f64; 4]>,
}

/// For an annotation without a normal appearance whose kind `synthesize`
/// sizes from its /Rect: that box (/Rect `outer` → (rect, rd)). None when
/// it has an appearance, or its box is its geometry, or it is not drawn.
pub fn synthesized_box(
    doc: &Document<'_>,
    annot: &Dictionary,
    outer: [f64; 4],
) -> Option<([f64; 4], Option<[f64; 4]>)> {
    if normal_appearance(doc, annot).is_some() {
        return None;
    }
    let kind = doc.dict_get(annot, b"Subtype").as_name()?;
    let (rect, rd) = sized_box(doc, annot, kind, outer)?;
    // Addendum B1: a Caret lists its whole /Rect, whatever /RD it keeps.
    Some((rect, if kind == b"Caret" { Some([0.0; 4]) } else { rd }))
}

/// The /Rect and /RD the synthesis gives a kind it sizes from /Rect
/// `outer` (spec 06 section 4); `synthesized_box` and `synthesize` share
/// it, so the listing and the drawing agree.
fn sized_box(
    doc: &Document<'_>,
    annot: &Dictionary,
    kind: &[u8],
    outer: [f64; 4],
) -> Option<([f64; 4], Option<[f64; 4]>)> {
    match kind {
        // The note size, from the top left of /Rect.
        b"Text" => Some((
            [
                outer[0],
                outer[3] - NOTE_SIZE,
                outer[0] + NOTE_SIZE,
                outer[3],
            ],
            None,
        )),
        b"Stamp" => Some((appearance::icon_fit(outer), None)),
        // Decision A3: the caret keeps its whole /Rect. Addendum B1: and
        // the file's /RD; /RD is written as zeros only when it has none.
        b"Caret" => {
            let rd = match super::rect_differences(doc, annot) {
                Some(_) => None,
                None => Some([0.0; 4]),
            };
            Some((outer, rd))
        }
        b"Square" | b"Circle" => {
            let rd = match super::rect_differences(doc, annot) {
                Some(_) => None,
                None => Some([shape_width(doc, annot) / 2.0; 4]),
            };
            Some((outer, rd))
        }
        b"FreeText" => Some((outer, None)),
        b"Redact" => {
            let quads = first_quads(doc, annot);
            Some((appearance::redact_rect(&quads).unwrap_or(outer), None))
        }
        _ => None,
    }
}

/// Quads a synthesized appearance draws at most, and points (Polygon,
/// PolyLine, Ink).
const MAX_QUADS: usize = 16_384;
const MAX_POINTS: usize = 131_072;

fn first_quads(doc: &Document<'_>, annot: &Dictionary) -> Vec<Quad> {
    let mut quads = super::quads(doc, annot);
    quads.truncate(MAX_QUADS);
    quads
}

/// The border width a shape draws (decision A4): /BS /W or /Border, 0 for
/// none, a negative width the default 1.
fn shape_width(doc: &Document<'_>, annot: &Dictionary) -> f64 {
    let width = super::border_width(doc, annot);
    if width < 0.0 {
        1.0
    } else {
        width.min(appearance::MAX_WIDTH)
    }
}

fn style(doc: &Document<'_>, annot: &Dictionary) -> Style {
    Style {
        stroke: color(doc, annot, b"C"),
        fill: color(doc, annot, b"IC"),
        width: shape_width(doc, annot),
        opacity: super::opacity(doc, annot),
        dash: super::border_dash(doc, annot),
    }
}

fn color(doc: &Document<'_>, annot: &Dictionary, key: &[u8]) -> Option<Color> {
    annot.get(key).and_then(|c| Color::read(doc, c))
}

/// The appearance for `annot` from its entries (spec 06 section 4); None
/// when the kind is not drawn or its geometry is unusable.
pub fn synthesize(doc: &Document<'_>, annot: &Dictionary) -> Option<Synthesized> {
    let kind = doc.dict_get(annot, b"Subtype").as_name()?;
    let outer = || {
        annot
            .get(b"Rect".as_slice())
            .and_then(|r| super::rect(doc, r))
    };
    let c = color(doc, annot, b"C");
    let alpha = super::opacity(doc, annot);
    let sized = |outer: [f64; 4]| sized_box(doc, annot, kind, outer);
    let made =
        |form: Form, (rect, rd): ([f64; 4], Option<[f64; 4]>)| Synthesized { form, rect, rd };
    match kind {
        b"Text" => {
            let name = doc.dict_get(annot, b"Name").as_name().unwrap_or(b"Note");
            let form = appearance::note(name, c.as_ref(), alpha);
            Some(made(form, sized(outer()?)?))
        }
        b"FreeText" => {
            let outer = outer()?;
            let text = super::text(doc, annot, b"Contents");
            let form = crate::app::ops::annots::free_text_form(doc, annot, outer, &text);
            Some(made(form, sized(outer)?))
        }
        b"Highlight" | b"Underline" | b"StrikeOut" | b"Squiggly" => {
            let quads = first_quads(doc, annot);
            if quads.is_empty() {
                return None;
            }
            let (form, rect) = match kind {
                b"Highlight" => appearance::highlight(&quads, c.as_ref(), alpha),
                b"Underline" => appearance::text_line(Mark::Underline, &quads, c.as_ref(), alpha),
                b"StrikeOut" => appearance::text_line(Mark::StrikeOut, &quads, c.as_ref(), alpha),
                _ => appearance::text_line(Mark::Squiggly, &quads, c.as_ref(), alpha),
            };
            Some(made(form, (rect, None)))
        }
        b"Square" | b"Circle" => {
            let outer = outer()?;
            let style = style(doc, annot);
            let inner = match super::rect_differences(doc, annot) {
                Some(rd) => super::inset(outer, Some(rd)),
                None => appearance::grow(outer, -style.width / 2.0),
            };
            let form = appearance::rectangle(inner, outer, kind == b"Circle", &style);
            Some(made(form, sized(outer)?))
        }
        b"Line" => {
            let [a, b] = super::line_points(doc, annot)?;
            let endings = doc.dict_get(annot, b"LE").as_array();
            let ending = |i: usize| {
                endings
                    .and_then(|e| e.get(i))
                    .and_then(|e| doc.resolve(e).as_name())
                    .unwrap_or(b"None")
            };
            let (form, rect) = appearance::line(a, b, [ending(0), ending(1)], &style(doc, annot));
            Some(made(form, (rect, None)))
        }
        b"Polygon" | b"PolyLine" => {
            let mut vertices = annot
                .get(b"Vertices".as_slice())
                .map(|v| super::points(doc, v))
                .unwrap_or_default();
            vertices.truncate(MAX_POINTS);
            if vertices.len() < 2 {
                return None;
            }
            let (form, rect) = appearance::poly(&vertices, kind == b"Polygon", &style(doc, annot));
            Some(made(form, (rect, None)))
        }
        b"Ink" => {
            let mut left = MAX_POINTS;
            let mut strokes = Vec::new();
            for mut stroke in super::ink_list(doc, annot) {
                if left == 0 {
                    break;
                }
                stroke.truncate(left);
                left -= stroke.len();
                if !stroke.is_empty() {
                    strokes.push(stroke);
                }
            }
            if strokes.is_empty() {
                return None;
            }
            let style = style(doc, annot);
            let (form, rect) = appearance::ink(&strokes, &style);
            let rd = appearance::ink_allowance(style.width);
            Some(made(form, (rect, Some([rd; 4]))))
        }
        b"Stamp" => {
            let name = doc.dict_get(annot, b"Name").as_name().unwrap_or(b"Draft");
            let form = appearance::stamp(name, c.as_ref(), alpha, appearance::IDENTITY);
            Some(made(form, sized(outer()?)?))
        }
        b"Caret" => {
            // Drawn over /Rect less /RD (decision A3).
            let outer = outer()?;
            let inner = super::design_rect(doc, annot, outer);
            let form =
                appearance::fitted(appearance::caret(c.as_ref(), alpha), inner, outer, false);
            Some(made(form, sized(outer)?))
        }
        b"Redact" => {
            let outer = outer()?;
            let quads = first_quads(doc, annot);
            let (form, rect) = appearance::redact(outer, &quads, c.as_ref());
            Some(made(form, (rect, None)))
        }
        // Owner decision A5: an icon centred in /Rect.
        b"FileAttachment" | b"Sound" => {
            let outer = outer()?;
            let icon = if kind == b"Sound" {
                Icon::Speaker
            } else {
                Icon::Paperclip
            };
            let form = appearance::fitted(
                appearance::tile(icon, c.as_ref(), alpha),
                outer,
                outer,
                true,
            );
            Some(made(form, (outer, None)))
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
    use std::sync::Arc;

    use super::*;
    use crate::document::DocumentLimits;

    /// A one-page file whose page lists `annot` (object 4).
    fn document(annot: &str) -> Document<'static> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>".to_string(),
            annot.to_string(),
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        Document::open_shared(Arc::new(bytes), DocumentLimits::default(), b"").unwrap()
    }

    fn annot<'d>(doc: &'d Document<'_>) -> &'d Dictionary {
        doc.resolve_id(ObjectId::new(4, 0).unwrap())
            .as_dict()
            .unwrap()
    }

    #[test]
    fn a_caret_keeps_its_rect_differences() {
        let outer = [100.0, 600.0, 200.0, 700.0];
        let doc =
            document("<< /Type /Annot /Subtype /Caret /Rect [100 600 200 700] /RD [10 20 30 5] >>");
        let made = synthesize(&doc, annot(&doc)).unwrap();
        assert_eq!((made.rect, made.rd), (outer, None));
        // Drawn into /Rect less /RD (ISO order: left, top, right, bottom).
        assert!(made.form.content.starts_with(b"q\n2 0 0 3.75 110 605 cm\n"));
        assert_eq!(made.form.bbox, outer);
        // Listed whole.
        assert_eq!(
            synthesized_box(&doc, annot(&doc), outer),
            Some((outer, Some([0.0; 4])))
        );
        // Without /RD, zeros are written.
        let doc = document("<< /Type /Annot /Subtype /Caret /Rect [100 600 200 700] >>");
        let made = synthesize(&doc, annot(&doc)).unwrap();
        assert_eq!(made.rd, Some([0.0; 4]));
        assert!(
            made.form
                .content
                .starts_with(b"q\n3.3333 0 0 5 100 600 cm\n")
        );
    }
}
