// SPDX-License-Identifier: AGPL-3.0-or-later
//! Pages and widgets as MuPDF reports them to the signing ops: the page
//! transform (`pdf_page_transform`, float32), annotation rectangles in page
//! space (`getRect`: y down, origin at the top-left of the page box), page
//! bounds, and a page's widgets with their inherited field type, full name
//! and flags.
use std::collections::BTreeSet;

use crate::app::info;
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId, decode_text_string};

/// /Parent hops followed when a field inherits a value or its name.
const MAX_PARENTS: usize = 64;

/// PDF user space to page space for one page, in float32 as MuPDF computes
/// it, and the page box in user space.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    ctm: [f32; 6],
    area: [f32; 4],
}

impl Frame {
    /// The page box rules of `info::page_size`: the CropBox within the
    /// MediaBox, Letter for an empty result, the unit square for a sliver,
    /// /UserUnit, and /Rotate snapped to a quarter turn.
    pub(crate) fn of(doc: &Document<'_>, page: &Dictionary) -> Self {
        let empty = |r: &[f64; 4]| !(r[0] < r[2] && r[1] < r[3]);
        let mut area = info::page_box(doc, page, b"MediaBox").unwrap_or([0.0; 4]);
        if let Some(crop) = info::page_box(doc, page, b"CropBox") {
            let x0 = area[0].max(crop[0]);
            let y0 = area[1].max(crop[1]);
            area = [
                x0,
                y0,
                area[2].min(crop[2]).max(x0),
                area[3].min(crop[3]).max(y0),
            ];
        }
        if empty(&area) {
            area = [0.0, 0.0, 612.0, 792.0];
        } else if area[2] - area[0] < 1.0 || area[3] - area[1] < 1.0 {
            area = [0.0, 0.0, 1.0, 1.0];
        }
        let unit = page
            .get(b"UserUnit".as_slice())
            .and_then(|value| real(doc, value))
            .filter(|unit| unit.is_finite() && *unit > 0.0)
            .unwrap_or(1.0);
        // MuPDF's pdf_to_int rounds a real half up, then snaps the turn.
        let rotate = inherited(doc, page, b"Rotate").and_then(|value| real(doc, value));
        let mut rotate = rotate.map_or(0, |r| (r + 0.5).floor() as i32);
        if rotate < 0 {
            rotate = 360 - (rotate.unsigned_abs() % 360) as i32;
        }
        rotate = 90 * ((rotate % 360 + 45) / 90);
        if rotate >= 360 {
            rotate = 0;
        }
        let [x0, y0, x1, y1] = area.map(|v| v as f32);
        let u = unit;
        let ctm = match rotate {
            90 => [0.0, u, u, 0.0, -(y0 * u), -(x0 * u)],
            180 => [-u, 0.0, 0.0, u, x1 * u, -(y0 * u)],
            270 => [0.0, -u, -u, 0.0, y1 * u, x1 * u],
            _ => [u, 0.0, 0.0, -u, -(x0 * u), y1 * u],
        };
        Self {
            ctm,
            area: [x0, y0, x1, y1],
        }
    }

    /// `page.getBounds()`.
    pub(crate) fn bounds(&self) -> [f32; 4] {
        transform_rect(self.area, self.ctm)
    }

    /// A user-space rectangle in page space (`pdf_annot_rect`).
    pub(crate) fn page_rect(&self, rect: [f32; 4]) -> [f32; 4] {
        transform_rect(rect, self.ctm)
    }

    /// A page-space rectangle in user space (`pdf_set_annot_rect`).
    pub(crate) fn user_rect(&self, rect: [f32; 4]) -> [f32; 4] {
        transform_rect(rect, invert(self.ctm))
    }

    /// An annotation's `getRect()`: its /Rect (`pdf_to_rect`) in page space.
    /// A missing /Rect reads as [0 0 0 0]; MuPDF transforms its empty
    /// rectangle into nonsense coordinates.
    pub(crate) fn annotation_rect(&self, doc: &Document<'_>, annot: &Dictionary) -> [f32; 4] {
        self.page_rect(rect_of(doc, doc.dict_get(annot, b"Rect")))
    }
}

/// MuPDF's `fz_transform_rect`.
fn transform_rect(rect: [f32; 4], m: [f32; 6]) -> [f32; 4] {
    let point = |x: f32, y: f32| (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5]);
    let [mut x0, mut y0, mut x1, mut y1] = rect;
    if m[1].abs() < f32::EPSILON && m[2].abs() < f32::EPSILON {
        if m[0] < 0.0 {
            std::mem::swap(&mut x0, &mut x1);
        }
        if m[3] < 0.0 {
            std::mem::swap(&mut y0, &mut y1);
        }
        let (a, b) = point(x0, y0);
        let (c, d) = point(x1, y1);
        return [a, b, c, d];
    }
    let corners = [point(x0, y0), point(x0, y1), point(x1, y1), point(x1, y0)];
    let xs = corners.map(|(x, _)| x);
    let ys = corners.map(|(_, y)| y);
    let min = |v: [f32; 4]| v.into_iter().fold(f32::INFINITY, f32::min);
    let max = |v: [f32; 4]| v.into_iter().fold(f32::NEG_INFINITY, f32::max);
    [min(xs), min(ys), max(xs), max(ys)]
}

/// MuPDF's `fz_invert_matrix`: a singular matrix stays as it is.
fn invert(m: [f32; 6]) -> [f32; 6] {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.is_nan() || (-f32::EPSILON..=f32::EPSILON).contains(&det) {
        return m;
    }
    let r = 1.0 / det;
    let (a, b, c, d) = (m[3] * r, -m[1] * r, -m[2] * r, m[0] * r);
    [a, b, c, d, -m[4] * a - m[5] * c, -m[4] * b - m[5] * d]
}

/// A number as MuPDF's float `pdf_to_real` reads it.
pub(crate) fn real(doc: &Document<'_>, value: &Object) -> Option<f32> {
    match doc.resolve(value) {
        Object::Integer(v) => Some(*v as f32),
        Object::Real(v) => Some(*v as f32),
        _ => None,
    }
}

/// MuPDF's `pdf_to_rect`: four numbers (missing or non-numeric ones read as
/// zero), ordered; anything but an array reads as [0 0 0 0].
pub(crate) fn rect_of(doc: &Document<'_>, value: &Object) -> [f32; 4] {
    let Some(items) = doc.resolve(value).as_array() else {
        return [0.0; 4];
    };
    let mut v = [0.0f32; 4];
    for (slot, item) in v.iter_mut().zip(items) {
        *slot = real(doc, item).unwrap_or(0.0);
    }
    [
        v[0].min(v[2]),
        v[1].min(v[3]),
        v[0].max(v[2]),
        v[1].max(v[3]),
    ]
}

/// `key` on `node` or the nearest /Parent that has it (non-null), with a
/// cycle stopping the walk (`pdf_dict_get_inheritable`).
pub(crate) fn inherited<'d>(
    doc: &'d Document<'_>,
    node: &'d Dictionary,
    key: &[u8],
) -> Option<&'d Object> {
    let mut seen = BTreeSet::new();
    let mut node = node;
    for _ in 0..MAX_PARENTS {
        if let Some(value) = node.get(key).filter(|value| !doc.resolve(value).is_null()) {
            return Some(value);
        }
        let parent = node.get(b"Parent".as_slice())?;
        if let Object::Reference(id) = parent {
            if !seen.insert(*id) {
                return None;
            }
        }
        node = doc.resolve(parent).as_dict()?;
    }
    None
}

/// One widget annotation on a page.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Widget<'d> {
    /// The object holding it; None for a dictionary written into /Annots.
    pub(crate) id: Option<ObjectId>,
    pub(crate) dict: &'d Dictionary,
}

impl<'d> Widget<'d> {
    /// `getFieldType() === "signature"`: the inherited /FT is /Sig.
    pub(crate) fn is_signature(&self, doc: &'d Document<'_>) -> bool {
        inherited(doc, self.dict, b"FT").is_some_and(|ft| doc.resolve(ft).as_name() == Some(b"Sig"))
    }

    /// `getName()`: the non-empty /T strings from the top field down, joined
    /// by dots; None when the /Parent chain cycles (MuPDF throws).
    pub(crate) fn name(&self, doc: &'d Document<'_>) -> Option<String> {
        let mut parts = Vec::new();
        let mut seen: BTreeSet<ObjectId> = self.id.into_iter().collect();
        let mut node = self.dict;
        for _ in 0..MAX_PARENTS {
            if let Some(Object::String(text)) = node.get(b"T".as_slice()).map(|t| doc.resolve(t)) {
                let text = decode_text_string(text);
                if !text.is_empty() {
                    parts.push(text);
                }
            }
            let Some(parent) = node.get(b"Parent".as_slice()) else {
                parts.reverse();
                return Some(parts.join("."));
            };
            if let Object::Reference(id) = parent {
                if !seen.insert(*id) {
                    return None;
                }
            }
            let Some(dict) = doc.resolve(parent).as_dict() else {
                parts.reverse();
                return Some(parts.join("."));
            };
            node = dict;
        }
        None
    }

    /// `isReadOnly()`: bit 1 of the inherited /Ff.
    pub(crate) fn read_only(&self, doc: &'d Document<'_>) -> bool {
        let flags = inherited(doc, self.dict, b"Ff").and_then(|ff| real(doc, ff));
        // pdf_to_int rounds a real half up.
        flags.is_some_and(|ff| ((ff + 0.5).floor() as i64) & 1 == 1)
    }
}

/// `page.getWidgets()`: the /Annots entries that are /Widget dictionaries,
/// in order.
pub(crate) fn widgets<'d>(doc: &'d Document<'_>, page: &'d Dictionary) -> Vec<Widget<'d>> {
    let Some(annots) = doc.dict_get(page, b"Annots").as_array() else {
        return Vec::new();
    };
    annots
        .iter()
        .filter_map(|annot| {
            let (id, value) = doc.resolve_with_holder(annot);
            let dict = value.as_dict()?;
            let widget = doc.dict_get(dict, b"Subtype").as_name() == Some(b"Widget");
            widget.then_some(Widget { id, dict })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_follow_mupdf() {
        let upright = [1.0, 0.0, 0.0, -1.0, 0.0, 792.0];
        assert_eq!(
            transform_rect([72.0, 112.0, 340.0, 172.0], upright),
            [72.0, 620.0, 340.0, 680.0]
        );
        assert_eq!(
            transform_rect([72.0, 620.0, 340.0, 680.0], invert(upright)),
            [72.0, 112.0, 340.0, 172.0]
        );
        let turned = [0.0, 1.0, 1.0, 0.0, 0.0, 0.0];
        assert_eq!(
            transform_rect([10.0, 20.0, 30.0, 60.0], turned),
            [20.0, 10.0, 60.0, 30.0]
        );
        assert_eq!(invert([0.0; 6]), [0.0; 6]);
    }
}
