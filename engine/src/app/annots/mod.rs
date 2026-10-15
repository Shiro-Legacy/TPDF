// SPDX-License-Identifier: AGPL-3.0-or-later
//! Annotations on pages, for every op group that reads or writes them
//! (annotations, redaction): MuPDF's page space, the list `annots` reports
//! (MuPDF's `getAnnotations`), lookup by object number, adding an annotation
//! to a page and removing one with every reference to it cut, the readers
//! of an annotation's geometry and style, and (`appearance`) the appearance
//! streams MuPDF's `pdf_update_appearance` writes. `view` draws, in the
//! view only, the appearances MuPDF synthesizes for annotations without one.
pub mod appearance;
pub mod fonts;
pub(crate) mod view;

use std::collections::BTreeSet;

use crate::document::{Document, DocumentError, DocumentErrorKind};
use crate::object::{Dictionary, Object, ObjectId, decode_text_string};

use super::edit::{Edit, Place};
use super::info;
use super::pagetree::{self, PageTree};

/// A point in PDF user space or page space.
pub type Point = (f64, f64);

/// A quadrilateral in MuPDF's corner order: upper left, upper right, lower
/// left, lower right (the order /QuadPoints stores, contrary to ISO 32000).
pub type Quad = [Point; 4];

/// MuPDF's page space for one page: points, y down, the origin at the top
/// left of the bound page (CropBox within MediaBox), /Rotate and /UserUnit
/// applied. Request and reply geometry is in this space; dictionaries hold
/// PDF user space. Computed as the view computes its page frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSpace {
    /// User space to page space.
    pub ctm: [f64; 6],
    /// User space from page space.
    pub inverse: [f64; 6],
    pub width: f64,
    pub height: f64,
    /// The /Rotate snapped to 0, 90, 180 or 270.
    pub rotate: i32,
}

impl PageSpace {
    pub fn of(doc: &Document<'_>, page: &Dictionary) -> Self {
        let letter = [0.0, 0.0, 612.0, 792.0];
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
            area = letter;
        } else if area[2] - area[0] < 1.0 || area[3] - area[1] < 1.0 {
            area = [0.0, 0.0, 1.0, 1.0];
        }
        let unit = (page.get(b"UserUnit".as_slice()))
            .and_then(|value| doc.resolve(value).as_f64())
            .map(|unit| unit as f32)
            .filter(|unit| unit.is_finite() && *unit > 0.0)
            .unwrap_or(1.0);
        let rotate = snapped_rotation(doc, page);
        let (x0, y0, x1, y1) = (
            area[0] as f32,
            area[1] as f32,
            area[2] as f32,
            area[3] as f32,
        );
        let w = x1 * unit - x0 * unit;
        let h = y1 * unit - y0 * unit;
        let (u, x0, y0, x1, y1) = (
            f64::from(unit),
            f64::from(x0),
            f64::from(y0),
            f64::from(x1),
            f64::from(y1),
        );
        let (ctm, width, height) = match rotate {
            90 => ([0., u, u, 0., -y0 * u, -x0 * u], h, w),
            180 => ([-u, 0., 0., u, x1 * u, -y0 * u], w, h),
            270 => ([0., -u, -u, 0., y1 * u, x1 * u], h, w),
            _ => ([u, 0., 0., -u, -x0 * u, y1 * u], w, h),
        };
        Self {
            ctm,
            inverse: invert(ctm),
            width: f64::from(width),
            height: f64::from(height),
            rotate,
        }
    }

    /// MuPDF's `getBounds`: the page box in page space.
    pub fn bounds(&self) -> [f64; 4] {
        [0.0, 0.0, self.width, self.height]
    }

    pub fn to_page(&self, p: Point) -> Point {
        apply(&self.ctm, p)
    }

    pub fn to_user(&self, p: Point) -> Point {
        apply(&self.inverse, p)
    }

    /// The page-space bounds of a user-space rect.
    pub fn rect_to_page(&self, rect: [f64; 4]) -> [f64; 4] {
        transform_rect(&self.ctm, rect)
    }

    /// The user-space bounds of a page-space rect (MuPDF's `setRect`).
    pub fn rect_to_user(&self, rect: [f64; 4]) -> [f64; 4] {
        transform_rect(&self.inverse, rect)
    }

    /// A page-space quad in user space, corners in the same order.
    pub fn quad_to_user(&self, quad: Quad) -> Quad {
        quad.map(|p| self.to_user(p))
    }

    /// The rotation that turns content upright on this page: user space
    /// rotated by the page's /Rotate, about the origin.
    pub fn upright(&self) -> [f64; 6] {
        match self.rotate {
            90 => [0.0, 1.0, -1.0, 0.0, 0.0, 0.0],
            180 => [-1.0, 0.0, 0.0, -1.0, 0.0, 0.0],
            270 => [0.0, -1.0, 1.0, 0.0, 0.0, 0.0],
            _ => appearance::IDENTITY,
        }
    }
}

/// The inherited /Rotate as the view reads it (a number, half rounded up),
/// snapped to a quarter turn.
fn snapped_rotation(doc: &Document<'_>, page: &Dictionary) -> i32 {
    let mut node = page;
    let mut rotate = 0;
    for _ in 0..64 {
        let value = node.get(b"Rotate".as_slice()).map(|v| doc.resolve(v));
        if let Some(value) = value.and_then(Object::as_f64) {
            rotate = ((value as f32) + 0.5).floor() as i32;
            break;
        }
        match doc.dict_get(node, b"Parent").as_dict() {
            Some(parent) => node = parent,
            None => break,
        }
    }
    if rotate < 0 {
        rotate = 360 - (rotate.unsigned_abs() % 360) as i32;
    }
    rotate = 90 * ((rotate % 360 + 45) / 90);
    if rotate >= 360 { 0 } else { rotate }
}

pub fn apply(m: &[f64; 6], (x, y): Point) -> Point {
    (x * m[0] + y * m[2] + m[4], x * m[1] + y * m[3] + m[5])
}

/// `a` then `b`.
pub fn concat(a: &[f64; 6], b: &[f64; 6]) -> [f64; 6] {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

pub fn invert(m: [f64; 6]) -> [f64; 6] {
    let det = m[0] * m[3] - m[1] * m[2];
    if det == 0.0 || !det.is_finite() {
        return appearance::IDENTITY;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    [a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)]
}

/// The bounds of a rect's transformed corners.
pub fn transform_rect(m: &[f64; 6], r: [f64; 4]) -> [f64; 4] {
    let corners = [(r[0], r[1]), (r[2], r[1]), (r[0], r[3]), (r[2], r[3])];
    appearance::bounds(corners.map(|p| apply(m, p))).unwrap_or(r)
}

/// An annotation colour: /C or /IC with one, three or four components.
#[derive(Clone, Debug, PartialEq)]
pub enum Color {
    Gray(f64),
    Rgb([f64; 3]),
    Cmyk([f64; 4]),
}

impl Color {
    /// MuPDF's `pdf_annot_color`: an array of 1, 3 or 4 numbers; anything
    /// else is no colour.
    pub fn read(doc: &Document<'_>, value: &Object) -> Option<Self> {
        let items = doc.resolve(value).as_array()?;
        let n = |i: usize| items.get(i).map_or(0.0, |v| number(doc, v).unwrap_or(0.0));
        match items.len() {
            1 => Some(Self::Gray(n(0))),
            3 => Some(Self::Rgb([n(0), n(1), n(2)])),
            4 => Some(Self::Cmyk([n(0), n(1), n(2), n(3)])),
            _ => None,
        }
    }

    pub fn object(&self) -> Object {
        match self {
            Self::Gray(g) => appearance::numbers(&[*g]),
            Self::Rgb(rgb) => appearance::numbers(rgb),
            Self::Cmyk(cmyk) => appearance::numbers(cmyk),
        }
    }

    /// Rec. 601 luma of the colour as RGB, in single precision as MuPDF
    /// computes it (a grey of exactly 0.25 is 0.25).
    pub fn luminance(&self) -> f64 {
        let [r, g, b] = match self {
            Self::Gray(g) => [*g; 3],
            Self::Rgb(rgb) => *rgb,
            Self::Cmyk([c, m, y, k]) => [
                (1.0 - c) * (1.0 - k),
                (1.0 - m) * (1.0 - k),
                (1.0 - y) * (1.0 - k),
            ],
        };
        let (r, g, b) = (r as f32, g as f32, b as f32);
        f64::from(r * 0.3 + g * 0.59 + b * 0.11)
    }
}

/// A resolved finite number.
pub fn number(doc: &Document<'_>, value: &Object) -> Option<f64> {
    doc.resolve(value).as_f64().filter(|v| v.is_finite())
}

/// A resolved array's numbers; a non-number reads as 0, as `pdf_to_real`.
pub fn numbers(doc: &Document<'_>, value: &Object) -> Option<Vec<f64>> {
    let items = doc.resolve(value).as_array()?;
    Some(
        items
            .iter()
            .map(|v| number(doc, v).unwrap_or(0.0))
            .collect(),
    )
}

/// MuPDF's `pdf_to_rect`: four numbers, normalized; None when it is not an
/// array.
pub fn rect(doc: &Document<'_>, value: &Object) -> Option<[f64; 4]> {
    let v = numbers(doc, value)?;
    let n = |i: usize| v.get(i).copied().unwrap_or(0.0);
    let (a, b, c, d) = (n(0), n(1), n(2), n(3));
    Some([a.min(c), b.min(d), a.max(c), b.max(d)])
}

/// /QuadPoints in groups of eight; a trailing partial group is ignored.
pub fn quads(doc: &Document<'_>, annot: &Dictionary) -> Vec<Quad> {
    let Some(values) = annot
        .get(b"QuadPoints".as_slice())
        .and_then(|v| numbers(doc, v))
    else {
        return Vec::new();
    };
    values
        .chunks_exact(8)
        .map(|q| [(q[0], q[1]), (q[2], q[3]), (q[4], q[5]), (q[6], q[7])])
        .collect()
}

/// An array of x y pairs; a trailing odd number is ignored.
pub fn points(doc: &Document<'_>, value: &Object) -> Vec<Point> {
    numbers(doc, value)
        .unwrap_or_default()
        .chunks_exact(2)
        .map(|p| (p[0], p[1]))
        .collect()
}

/// /InkList: one point list per stroke.
pub fn ink_list(doc: &Document<'_>, annot: &Dictionary) -> Vec<Vec<Point>> {
    let Some(strokes) = doc.dict_get(annot, b"InkList").as_array() else {
        return Vec::new();
    };
    strokes.iter().map(|stroke| points(doc, stroke)).collect()
}

/// /L as two points.
pub fn line_points(doc: &Document<'_>, annot: &Dictionary) -> Option<[Point; 2]> {
    let values = annot.get(b"L".as_slice()).and_then(|v| numbers(doc, v))?;
    let n = |i: usize| values.get(i).copied().unwrap_or(0.0);
    (values.len() >= 4).then(|| [(n(0), n(1)), (n(2), n(3))])
}

/// MuPDF's `pdf_annot_border_width`: /BS /W, else the third /Border
/// number, else 1.
pub fn border_width(doc: &Document<'_>, annot: &Dictionary) -> f64 {
    let bs = doc.dict_get(annot, b"BS");
    if let Some(w) = bs.as_dict().and_then(|bs| bs.get(b"W".as_slice())) {
        if let Some(w) = number(doc, w) {
            return w;
        }
    }
    let border = doc.dict_get(annot, b"Border");
    if let Some(w) = border.as_array().and_then(|items| items.get(2)) {
        if let Some(w) = number(doc, w) {
            return w;
        }
    }
    1.0
}

/// The /BS dash pattern when the border style is dashed.
pub fn border_dash(doc: &Document<'_>, annot: &Dictionary) -> Vec<f64> {
    let Some(bs) = doc.dict_get(annot, b"BS").as_dict() else {
        return Vec::new();
    };
    if doc.dict_get(bs, b"S").as_name() != Some(b"D") {
        return Vec::new();
    }
    match bs.get(b"D".as_slice()).and_then(|d| numbers(doc, d)) {
        Some(dash) if !dash.is_empty() => dash,
        _ => vec![3.0],
    }
}

/// /CA, 1 when absent; clamped into 0..=1.
pub fn opacity(doc: &Document<'_>, annot: &Dictionary) -> f64 {
    let ca = annot.get(b"CA".as_slice()).and_then(|v| number(doc, v));
    ca.unwrap_or(1.0).clamp(0.0, 1.0)
}

/// A text string entry (/Contents, /Subj), "" when absent or not a string.
pub fn text(doc: &Document<'_>, annot: &Dictionary, key: &[u8]) -> String {
    doc.dict_get(annot, key)
        .as_string()
        .map(decode_text_string)
        .unwrap_or_default()
}

/// /RD as left, bottom, right and top insets (ISO order [l t r b]); None
/// unless it is four non-negative numbers.
pub fn rect_differences(doc: &Document<'_>, annot: &Dictionary) -> Option<[f64; 4]> {
    let rd = annot.get(b"RD".as_slice()).and_then(|v| numbers(doc, v))?;
    match rd.as_slice() {
        &[l, t, r, b] if [l, t, r, b].iter().all(|v| *v >= 0.0) => Some([l, b, r, t]),
        _ => None,
    }
}

/// One annotation of a page, as MuPDF's `getAnnotations` lists it.
#[derive(Clone, Copy, Debug)]
pub struct Annot<'d> {
    /// The object holding it; None for a dictionary written into /Annots.
    pub id: Option<ObjectId>,
    pub dict: &'d Dictionary,
    pub subtype: &'d [u8],
}

impl Annot<'_> {
    /// The id the app names it by: its object number (generation ignored),
    /// 0 for a direct dictionary.
    pub fn number(&self) -> u32 {
        self.id.map_or(0, ObjectId::number)
    }
}

/// The page's annotations in /Annots order, as MuPDF lists them: Links,
/// Popups and Widgets are not among them, nor entries without a /Subtype
/// name; an object listed twice is listed once.
pub fn list<'d>(doc: &'d Document<'_>, page: &'d Dictionary) -> Vec<Annot<'d>> {
    let Some(items) = doc.dict_get(page, b"Annots").as_array() else {
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    for item in items {
        let (id, value) = doc.resolve_with_holder(item);
        if let Some(id) = id
            && !seen.insert(id)
        {
            continue;
        }
        let Some(dict) = value.as_dict() else {
            continue;
        };
        let Some(subtype) = doc.dict_get(dict, b"Subtype").as_name() else {
            continue;
        };
        if matches!(subtype, b"Link" | b"Popup" | b"Widget") {
            continue;
        }
        found.push(Annot { id, dict, subtype });
    }
    found
}

/// The listed annotation whose object number is `number`.
pub fn find<'d>(doc: &'d Document<'_>, page: &'d Dictionary, number: i64) -> Option<Annot<'d>> {
    let number = u32::try_from(number).ok().filter(|&n| n > 0)?;
    list(doc, page)
        .into_iter()
        .find(|annot| annot.number() == number)
}

/// The page-space box `annots` reports (engine.ts `annotRect`): /Rect less
/// /RD for the kinds that have a rect, the quads' union for text markup,
/// the bounds of the points of a Line, Ink, Polygon or PolyLine, else
/// /Rect. None when the annotation has none of these.
pub fn page_rect(doc: &Document<'_>, space: &PageSpace, annot: &Dictionary) -> Option<[f64; 4]> {
    let subtype = doc.dict_get(annot, b"Subtype").as_name().unwrap_or(b"");
    let to_page =
        |points: Vec<Point>| appearance::bounds(points.into_iter().map(|p| space.to_page(p)));
    let geometry = match subtype {
        b"Highlight" | b"Underline" | b"StrikeOut" | b"Squiggly" => {
            to_page(quads(doc, annot).into_iter().flatten().collect())
        }
        b"Line" => line_points(doc, annot).and_then(|l| to_page(l.to_vec())),
        b"Ink" => to_page(ink_list(doc, annot).into_iter().flatten().collect()),
        b"Polygon" | b"PolyLine" => {
            let vertices = annot.get(b"Vertices".as_slice()).map(|v| points(doc, v));
            to_page(vertices.unwrap_or_default())
        }
        _ => None,
    };
    if geometry.is_some() {
        return geometry;
    }
    let outer = annot.get(b"Rect".as_slice()).and_then(|r| rect(doc, r))?;
    Some(space.rect_to_page(design_rect(doc, annot, outer)))
}

/// /Rect less /RD for the kinds /RD applies to (FreeText, Square, Circle,
/// Caret), when that leaves a non-empty rect.
pub fn design_rect(doc: &Document<'_>, annot: &Dictionary, outer: [f64; 4]) -> [f64; 4] {
    let subtype = doc.dict_get(annot, b"Subtype").as_name().unwrap_or(b"");
    if !matches!(subtype, b"FreeText" | b"Square" | b"Circle" | b"Caret") {
        return outer;
    }
    match rect_differences(doc, annot) {
        Some([l, b, r, t]) => {
            let inner = [outer[0] + l, outer[1] + b, outer[2] - r, outer[3] - t];
            if inner[0] <= inner[2] && inner[1] <= inner[3] {
                inner
            } else {
                outer
            }
        }
        None => outer,
    }
}

/// Why an annotation was not added or removed.
#[derive(Debug)]
pub enum AnnotError {
    /// No page at that index.
    NoPage,
    /// A page written into /Kids whose page tree cannot be edited.
    Tree(pagetree::TreeError),
    Document(DocumentError),
}

impl std::fmt::Display for AnnotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPage => f.write_str("no such page"),
            Self::Tree(error) => error.fmt(f),
            Self::Document(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for AnnotError {}

impl From<DocumentError> for AnnotError {
    fn from(error: DocumentError) -> Self {
        Self::Document(error)
    }
}

/// The page object at `index`, None for a page written into /Kids.
pub fn page_object(doc: &Document<'_>, index: usize) -> Result<Option<ObjectId>, AnnotError> {
    let page = info::pages(doc)
        .get(index)
        .copied()
        .ok_or(AnnotError::NoPage)?;
    Ok(page.id)
}

/// Runs `edit` on page `index`'s /Annots array: in its own object when it
/// has one, else in the page dictionary (created when missing, replacing
/// anything that is not an array).
pub fn edit_annots<E: Edit, R>(
    target: &mut E,
    index: usize,
    edit: impl FnOnce(&mut Vec<Object>) -> R,
) -> Result<R, AnnotError> {
    let doc = target.document();
    let page = info::pages(doc)
        .get(index)
        .copied()
        .ok_or(AnnotError::NoPage)?;
    let page_id = page.id;
    // The array's own object, when /Annots refers to one.
    let held = (page.dict.get(b"Annots".as_slice()))
        .map(|value| doc.resolve_with_holder(value))
        .and_then(|(holder, value)| holder.filter(|_| value.as_array().is_some()));
    let run = |value: &mut Object| {
        if value.as_array().is_none() {
            *value = Object::Array(Vec::new());
        }
        value.as_array_mut().map(edit)
    };
    let result = match (held, page_id) {
        (Some(array), _) => target.update(array, run)?,
        (None, Some(id)) => {
            let place = Place {
                id,
                keys: vec![b"Annots".to_vec()],
            };
            place.update(target, run)?
        }
        (None, None) => {
            let tree = PageTree::read(target.document()).map_err(AnnotError::Tree)?;
            let slot = *tree.slots.get(index).ok_or(AnnotError::NoPage)?;
            let kids = Place::object(slot.parent).child(target.document(), b"Kids");
            kids.update(target, |value| {
                let page = value.as_array_mut()?.get_mut(slot.kid)?.as_dict_mut()?;
                if !page.contains_key(b"Annots".as_slice()) {
                    page.insert(b"Annots".to_vec(), Object::Array(Vec::new()));
                }
                page.get_mut(b"Annots".as_slice()).and_then(run)
            })?
        }
    };
    result.ok_or(AnnotError::Document(DocumentError {
        offset: 0,
        kind: DocumentErrorKind::Invalid("page /Annots is not an array"),
    }))
}

/// Adds `annot` to page `index` as a new object at the end of its /Annots.
pub fn add<E: Edit>(
    target: &mut E,
    index: usize,
    annot: Dictionary,
) -> Result<ObjectId, AnnotError> {
    let id = target.allocate(Object::Dictionary(annot))?;
    append(target, index, id)?;
    Ok(id)
}

/// Appends a reference to `id` to page `index`'s /Annots.
pub fn append<E: Edit>(target: &mut E, index: usize, id: ObjectId) -> Result<(), AnnotError> {
    edit_annots(target, index, |items| items.push(Object::Reference(id)))
}

/// Removes annotation `id` shown on page `index` (MuPDF's
/// `deleteAnnotation`): it, its /Popup and any Popup on the page naming it
/// as /Parent leave /Annots, and every other reference to them is cut (a
/// reply's /IRT, a structure OBJR, another page's /Annots), so no save
/// writes them. Their objects stay, unreferenced.
pub fn remove<E: Edit>(target: &mut E, index: usize, id: ObjectId) -> Result<(), AnnotError> {
    let doc = target.document();
    let page = info::pages(doc)
        .get(index)
        .copied()
        .ok_or(AnnotError::NoPage)?;
    let mut dropped = BTreeSet::from([id]);
    let popup = doc
        .resolve_id(id)
        .as_dict()
        .and_then(|annot| annot.get(b"Popup".as_slice()));
    dropped.extend(popup.and_then(Object::as_reference));
    for item in doc.dict_get(page.dict, b"Annots").as_array().unwrap_or(&[]) {
        let Some(other) = item.as_reference() else {
            continue;
        };
        let Some(dict) = doc.resolve_id(other).as_dict() else {
            continue;
        };
        let popup = doc.dict_get(dict, b"Subtype").as_name() == Some(b"Popup");
        if popup
            && dict
                .get(b"Parent".as_slice())
                .and_then(Object::as_reference)
                == Some(id)
        {
            dropped.insert(other);
        }
    }
    edit_annots(target, index, |items| {
        items.retain(|item| !item.as_reference().is_some_and(|r| dropped.contains(&r)));
    })?;
    pagetree::cut_references(target, &BTreeSet::new(), &dropped, &dropped)?;
    Ok(())
}
