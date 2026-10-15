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

use std::collections::{BTreeMap, BTreeSet};

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

/// The page-space box `annots` reports (engine.ts `annotRect`): the quads'
/// union for text markup, the bounds of the points of a Line, Ink, Polygon
/// or PolyLine, else MuPDF's `getRect`: /Rect less /RD, where /Rect and /RD
/// are those MuPDF's synthesis gives an annotation drawn without an
/// appearance (`view::synthesized_box`). None when the annotation has none
/// of these.
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
    let inner = match view::synthesized_box(doc, annot, outer) {
        Some((outer, rd)) => inset(outer, rd.or_else(|| rect_differences(doc, annot))),
        None => design_rect(doc, annot, outer),
    };
    Some(space.rect_to_page(inner))
}

/// /Rect less /RD (MuPDF's `pdf_annot_rect`, for every kind), when that
/// leaves a non-empty rect.
pub fn design_rect(doc: &Document<'_>, annot: &Dictionary, outer: [f64; 4]) -> [f64; 4] {
    inset(outer, rect_differences(doc, annot))
}

/// `outer` less insets in `rect_differences` order (left, bottom, right,
/// top); `outer` itself when that would leave nothing.
pub fn inset(outer: [f64; 4], rd: Option<[f64; 4]>) -> [f64; 4] {
    match rd {
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

/// `inner` grown by the annotation's /RD: the /Rect whose design box
/// (`design_rect`) it is.
pub fn grow_by_rd(doc: &Document<'_>, annot: &Dictionary, inner: [f64; 4]) -> [f64; 4] {
    match rect_differences(doc, annot) {
        Some([l, b, r, t]) => [inner[0] - l, inner[1] - b, inner[2] + r, inner[3] + t],
        None => inner,
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

/// Stores `form` as a new stream and points `annot`'s /AP /N at it.
pub fn put_appearance<E: Edit>(
    target: &mut E,
    annot: &mut Dictionary,
    form: &appearance::Form,
) -> Result<ObjectId, DocumentError> {
    let id = target.allocate(Object::Null)?;
    target.put_stream_raw(id, form.dictionary(), form.content.clone())?;
    let mut ap = Dictionary::new();
    ap.insert(b"N".to_vec(), Object::Reference(id));
    annot.insert(b"AP".to_vec(), Object::Dictionary(ap));
    Ok(id)
}

/// Removes annotation `id` shown on page `index`; see `remove_many`.
pub fn remove<E: Edit>(target: &mut E, index: usize, id: ObjectId) -> Result<(), AnnotError> {
    remove_many(target, &[(index, id)])
}

/// Removes annotations, each `(page index, id)` shown on that page (MuPDF's
/// `deleteAnnotation`), with one pass over the document for all of them.
/// Each annotation and its popups (the /Popup it names when that is a
/// Popup annotation whose /Parent is it or absent, and any Popup on the
/// page naming it as /Parent) leave the page's /Annots; then `cut_unlisted`
/// cuts every other reference to those no page lists any more (a reply's
/// /IRT, a structure OBJR), so no save writes them. Their objects stay,
/// unreferenced. One that another page still lists stays whole there.
pub fn remove_many<E: Edit>(target: &mut E, marks: &[(usize, ObjectId)]) -> Result<(), AnnotError> {
    let doc = target.document();
    let pages = info::pages(doc);
    let mut by_page: BTreeMap<usize, BTreeSet<ObjectId>> = BTreeMap::new();
    for &(index, id) in marks {
        if index >= pages.len() {
            return Err(AnnotError::NoPage);
        }
        by_page.entry(index).or_default().insert(id);
    }
    let mut groups: BTreeMap<ObjectId, BTreeSet<ObjectId>> = BTreeMap::new();
    let mut unlisted = Vec::new();
    for (&index, ids) in &by_page {
        let Some(page) = pages.get(index) else {
            return Err(AnnotError::NoPage);
        };
        let mut gone = ids.clone();
        gone.extend(ids.iter().filter_map(|&id| own_popup(doc, id)));
        let items = doc.dict_get(page.dict, b"Annots").as_array().unwrap_or(&[]);
        for item in items {
            let (Some(other), dict) = doc.resolve_with_holder(item) else {
                continue;
            };
            let Some(dict) = dict.as_dict() else {
                continue;
            };
            let parent = dict
                .get(b"Parent".as_slice())
                .and_then(Object::as_reference);
            if doc.dict_get(dict, b"Subtype").as_name() == Some(b"Popup")
                && parent.is_some_and(|parent| ids.contains(&parent))
            {
                gone.insert(other);
            }
        }
        // The entries naming one, directly or through a reference chain.
        let mut entries = BTreeSet::new();
        for item in items {
            let Some(entry) = item.as_reference() else {
                continue;
            };
            let holder = doc.resolve_with_holder(item).0;
            if let Some(id) = [holder, Some(entry)]
                .into_iter()
                .flatten()
                .find(|id| gone.contains(id))
            {
                entries.insert(entry);
                groups.entry(id).or_default().insert(entry);
            }
        }
        for &id in &gone {
            groups.entry(id).or_default().insert(id);
        }
        if !entries.is_empty() {
            unlisted.push((index, entries));
        }
    }
    for (index, entries) in unlisted {
        edit_annots(target, index, |items| {
            items.retain(|item| !item.as_reference().is_some_and(|r| entries.contains(&r)));
        })?;
    }
    let groups: Vec<BTreeSet<ObjectId>> = groups.into_values().collect();
    cut_unlisted(target, &groups)?;
    Ok(())
}

/// After annotations left pages' /Annots (an annotation or link deleted,
/// a redaction mark applied): cuts every other reference to each one that
/// no page lists any more (a structure OBJR, another annotation's /IRT or
/// /Popup, a name tree), so a save does not keep it. Each group holds the
/// ids naming one annotation (its /Annots entry and the object that entry
/// resolves to); a group any page still lists, by either, stays whole, and
/// so does one that a listed annotation names as its /Popup (MuPDF takes a
/// note's popup out of the page's /Annots and leaves the note's /Popup). The
/// catalog, page-tree nodes (in the tree, or typed /Page, /Pages or
/// /Catalog) and streams are never cut, whatever named them. One pass over
/// the document.
pub fn cut_unlisted<E: Edit>(
    target: &mut E,
    removed: &[BTreeSet<ObjectId>],
) -> Result<(), DocumentError> {
    let wanted: BTreeSet<ObjectId> = removed.iter().flatten().copied().collect();
    if wanted.is_empty() {
        return Ok(());
    }
    let doc = target.document();
    let pages = info::pages(doc);
    // An /Annots array several pages share is read once.
    let mut arrays = BTreeSet::new();
    let mut listed = BTreeSet::new();
    for page in &pages {
        let Some(value) = page.dict.get(b"Annots".as_slice()) else {
            continue;
        };
        let (holder, value) = doc.resolve_with_holder(value);
        if holder.is_some_and(|holder| !arrays.insert(holder)) {
            continue;
        }
        for entry in value.as_array().unwrap_or(&[]) {
            let (holder, annot) = doc.resolve_with_holder(entry);
            // A popup an annotation still listed names stays with it.
            let popup = (annot.as_dict())
                .and_then(|dict| dict.get(b"Popup".as_slice()))
                .and_then(Object::as_reference);
            listed.extend(
                [entry.as_reference(), holder, popup]
                    .into_iter()
                    .flatten()
                    .filter(|id| wanted.contains(id)),
            );
        }
    }
    let nodes = tree_nodes(doc, &pages);
    let dropped: BTreeSet<ObjectId> = (removed.iter())
        .filter(|group| group.is_disjoint(&listed))
        .flatten()
        .copied()
        .filter(|&id| !structural(doc, &nodes, id))
        .collect();
    if dropped.is_empty() {
        return Ok(());
    }
    pagetree::cut_references(target, &BTreeSet::new(), &dropped, &dropped)
}

/// The Popup annotation `id` names as its /Popup, when that is one whose
/// /Parent is `id` or absent. Anything else /Popup names is left alone
/// (MuPDF takes it out of the page's /Annots whatever it is).
fn own_popup(doc: &Document<'_>, id: ObjectId) -> Option<ObjectId> {
    let annot = doc.resolve_id(id).as_dict()?;
    let popup = annot.get(b"Popup".as_slice())?.as_reference()?;
    let dict = doc.resolve_id(popup).as_dict()?;
    if doc.dict_get(dict, b"Subtype").as_name() != Some(b"Popup") {
        return None;
    }
    match dict.get(b"Parent".as_slice()) {
        None | Some(Object::Null) => Some(popup),
        Some(parent) => (parent.as_reference() == Some(id)).then_some(popup),
    }
}

/// The page objects and page-tree nodes, as far as the tree reads.
fn tree_nodes(doc: &Document<'_>, pages: &[info::PageRef<'_>]) -> BTreeSet<ObjectId> {
    let mut nodes: BTreeSet<ObjectId> = pages.iter().filter_map(|p| p.id).collect();
    let catalog = doc.resolve_id(doc.root()).as_dict();
    nodes.extend(
        catalog
            .and_then(|c| c.get(b"Pages".as_slice()))
            .and_then(Object::as_reference),
    );
    if let Ok(tree) = PageTree::read(doc) {
        nodes.insert(tree.root);
        let parents: BTreeSet<ObjectId> = tree.slots.iter().map(|slot| slot.parent).collect();
        for parent in parents {
            nodes.extend(tree.ancestors(parent));
        }
    }
    nodes
}

/// An object a deletion never cuts: the catalog, a page or page-tree node
/// (in the tree or typed so), or a stream.
fn structural(doc: &Document<'_>, nodes: &BTreeSet<ObjectId>, id: ObjectId) -> bool {
    if id == doc.root() || nodes.contains(&id) {
        return true;
    }
    let Ok(object) = doc.object(id) else {
        return false;
    };
    object.stream.is_some()
        || object.value.as_dict().is_some_and(|dict| {
            matches!(
                doc.dict_get(dict, b"Type").as_name(),
                Some(b"Page" | b"Pages" | b"Catalog")
            )
        })
}
