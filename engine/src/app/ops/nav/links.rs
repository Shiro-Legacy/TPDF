// SPDX-License-Identifier: AGPL-3.0-or-later
//! Link annotations of a page, as MuPDF's `getLinks` lists them, in the
//! app's page space (points, y down, origin at the page's top left), and
//! the writes link-add, link-set and link-delete make.
//!
//! Each listed link is its own /Annots entry, so its identity (`annotId`,
//! the entry's object number) is exact; engine.ts paired MuPDF's links with
//! the entries again by rect.
use std::collections::{BTreeMap, BTreeSet};

use crate::app::annots;
use crate::app::edit::{Edit, Place};
use crate::app::info;
use crate::app::pagetree::{self, PageTree};
use crate::document::{Document, DocumentError, DocumentErrorKind};
use crate::object::{Dictionary, Object, ObjectId};

use super::target::{Resolver, Target};
use super::{MAX_LISTED_BYTES, MAX_LISTED_ITEMS, json_len};

/// One link: its /Annots entry, where it is on the page and where it goes.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    /// The entry's index in the page's /Annots array.
    pub position: usize,
    /// The entry's object number; 0 for a dictionary written into /Annots.
    pub annot_id: u32,
    /// The object holding the link dictionary, when there is one.
    pub holder: Option<ObjectId>,
    /// Page space: [left, top, right, bottom].
    pub rect: [f64; 4],
    pub target: Target,
}

/// The affine map from a page's user space to page space, MuPDF's
/// `pdf_page_transform` as the view's `PageFrame` computes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSpace {
    ctm: [f64; 6],
    /// The page box in page space: [0, 0, width, height].
    pub bounds: [f64; 4],
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
            .and_then(|unit| doc.resolve(unit).as_f64())
            .map(|unit| unit as f32)
            .filter(|unit| unit.is_finite() && *unit > 0.0)
            .map_or(1.0, f64::from);
        let mut rotate = rotation(doc, page);
        if rotate < 0 {
            rotate = 360 - (rotate.unsigned_abs() % 360) as i64;
        }
        rotate = 90 * ((rotate % 360 + 45) / 90);
        let [x0, y0, x1, y1] = area.map(|v| f64::from(v as f32));
        let u = unit;
        let ctm = match rotate {
            90 => [0., u, u, 0., -y0 * u, -x0 * u],
            180 => [-u, 0., 0., u, x1 * u, -y0 * u],
            270 => [0., -u, -u, 0., y1 * u, x1 * u],
            _ => [u, 0., 0., -u, -x0 * u, y1 * u],
        };
        let (width, height) = info::page_size(doc, page);
        Self {
            ctm,
            bounds: [0.0, 0.0, width, height],
        }
    }

    /// A user-space rect in page space (normalized).
    pub fn to_page(&self, rect: [f64; 4]) -> [f64; 4] {
        transform_rect(self.ctm, rect)
    }

    /// A page-space rect in user space, as /Rect stores it (normalized).
    pub fn to_user(&self, rect: [f64; 4]) -> [f64; 4] {
        let [a, b, c, d, e, f] = self.ctm;
        let det = a * d - b * c;
        if det == 0.0 || !det.is_finite() {
            return rect;
        }
        let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
        let inverse = [ia, ib, ic, id, -(e * ia + f * ic), -(e * ib + f * id)];
        transform_rect(inverse, rect)
    }
}

/// The inherited /Rotate as MuPDF's `pdf_to_int` reads it (a real rounds
/// half up); 0 when there is none.
fn rotation(doc: &Document<'_>, page: &Dictionary) -> i64 {
    let mut node = page;
    for _ in 0..64 {
        if let Some(value) = (node.get(b"Rotate".as_slice())).and_then(|v| doc.resolve(v).as_f64())
        {
            let value = value as f32;
            return if value.is_finite() {
                (value + 0.5).floor() as i64
            } else {
                0
            };
        }
        match doc.dict_get(node, b"Parent").as_dict() {
            Some(parent) => node = parent,
            None => break,
        }
    }
    0
}

fn transform_rect(m: [f64; 6], rect: [f64; 4]) -> [f64; 4] {
    let [a, b, c, d, e, f] = m;
    let point = |x: f64, y: f64| (a * x + c * y + e, b * x + d * y + f);
    let corners = [
        point(rect[0], rect[1]),
        point(rect[2], rect[1]),
        point(rect[0], rect[3]),
        point(rect[2], rect[3]),
    ];
    let mut out = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for (x, y) in corners {
        out = [out[0].min(x), out[1].min(y), out[2].max(x), out[3].max(y)];
    }
    out
}

/// MuPDF's `pdf_to_rect` of a /Rect array: missing or non-numeric entries
/// read as 0, corners ordered.
fn raw_rect(doc: &Document<'_>, items: &[Object]) -> [f64; 4] {
    let mut v = [0.0f64; 4];
    for (slot, item) in v.iter_mut().zip(items) {
        *slot = doc.resolve(item).as_f64().unwrap_or(0.0);
    }
    [
        v[0].min(v[2]),
        v[1].min(v[3]),
        v[0].max(v[2]),
        v[1].max(v[3]),
    ]
}

/// A page's /Annots entries.
fn annots<'d>(doc: &'d Document<'_>, page: &'d Dictionary) -> &'d [Object] {
    doc.dict_get(page, b"Annots").as_array().unwrap_or(&[])
}

/// A page's links as the `links` reply lists them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listing {
    pub links: Vec<Link>,
    /// The list stopped at `MAX_LISTED_ITEMS` links or `MAX_LISTED_BYTES`
    /// of URIs with more to come.
    pub truncated: bool,
}

/// The links of page `index`, in /Annots order: every /Link annotation with
/// a /Rect array and a target MuPDF lists, up to `MAX_LISTED_ITEMS` links
/// and `MAX_LISTED_BYTES` of URI text.
pub fn list(doc: &Document<'_>, resolver: &Resolver<'_>, index: usize) -> Listing {
    let mut listing = Listing::default();
    let pages = info::pages(doc);
    let Some(page) = pages.get(index) else {
        return listing;
    };
    let space = PageSpace::of(doc, page.dict);
    let links = &mut listing.links;
    let mut bytes = 0usize;
    for (position, entry) in annots(doc, page.dict).iter().enumerate() {
        let (holder, value) = doc.resolve_with_holder(entry);
        let Some(annot) = value.as_dict() else {
            continue;
        };
        if doc.dict_get(annot, b"Subtype").as_name() != Some(b"Link") {
            continue;
        }
        let Some(rect) = doc.dict_get(annot, b"Rect").as_array() else {
            continue;
        };
        let Some(target) = resolver.link(annot, index) else {
            continue;
        };
        bytes = bytes.saturating_add(target.uri().map_or(0, json_len));
        if links.len() >= MAX_LISTED_ITEMS || bytes > MAX_LISTED_BYTES {
            listing.truncated = true;
            break;
        }
        links.push(Link {
            position,
            annot_id: entry.as_reference().map_or(0, |id| id.number()),
            holder,
            rect: space.to_page(raw_rect(doc, rect)),
            target,
        });
    }
    listing
}

/// The page object of page `index`, made one first (in the same step) when
/// the page is a dictionary written into its node's /Kids.
pub fn page_object<E: Edit>(target: &mut E, index: usize) -> Result<ObjectId, DocumentError> {
    let missing = || DocumentError {
        offset: 0,
        kind: DocumentErrorKind::Invalid("page index"),
    };
    if let Some(id) = info::pages(target.document())
        .get(index)
        .and_then(|page| page.id)
    {
        return Ok(id);
    }
    if pagetree::normalize(target)? {
        if let Ok(tree) = PageTree::read(target.document()) {
            tree.fix_counts(target)?;
        }
    }
    let tree = PageTree::read(target.document()).map_err(|_| missing())?;
    let slot = *tree.slots.get(index).ok_or_else(missing)?;
    if let Some(id) = slot.id {
        return Ok(id);
    }
    let kids = Place::object(slot.parent).child(target.document(), b"Kids");
    let page = kids
        .read(target.document())
        .as_array()
        .and_then(|kids| kids.get(slot.kid))
        .cloned();
    let Some(page @ Object::Dictionary(_)) = page else {
        return Err(missing());
    };
    let id = target.allocate(page)?;
    kids.update(target, |value| {
        if let Some(kid) = value.as_array_mut().and_then(|kids| kids.get_mut(slot.kid)) {
            *kid = Object::Reference(id);
        }
    })?;
    Ok(id)
}

/// Where the /Annots array of the page object `page` is, when it has one
/// (in the page, or the object its entry names).
fn annots_array(doc: &Document<'_>, page: ObjectId) -> Option<Place> {
    let place = Place::object(page).child(doc, b"Annots");
    place.read(doc).as_array().is_some().then_some(place)
}

/// A /Rect of four reals.
fn rect_object(rect: [f64; 4]) -> Object {
    Object::Array(
        rect.iter()
            .map(|&value| Object::real(value).unwrap_or(Object::Integer(0)))
            .collect(),
    )
}

/// The action MuPDF's `pdf_new_action_from_link` writes for the app's two
/// kinds of target: GoTo a page with /Fit, or a URI.
fn action(page: Option<ObjectId>, uri: Option<&str>) -> Object {
    let mut action = Dictionary::new();
    match (page, uri) {
        (Some(page), _) => {
            action.insert(b"S".to_vec(), Object::name("GoTo"));
            let dest = vec![Object::Reference(page), Object::name("Fit")];
            action.insert(b"D".to_vec(), Object::Array(dest));
        }
        (None, uri) => {
            action.insert(b"S".to_vec(), Object::name("URI"));
            action.insert(b"URI".to_vec(), Object::text(uri.unwrap_or("")));
        }
    }
    Object::Dictionary(action)
}

/// What a link goes to once written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewTarget {
    Page(usize),
    Uri(String),
}

fn new_action<E: Edit>(target: &mut E, to: &NewTarget) -> Result<Object, DocumentError> {
    Ok(match to {
        NewTarget::Page(page) => action(Some(page_object(target, *page)?), None),
        NewTarget::Uri(uri) => action(None, Some(uri)),
    })
}

/// link-add: a new link annotation at the end of the page's /Annots, its
/// rect given in page space. Returns its object.
pub fn add<E: Edit>(
    target: &mut E,
    page: usize,
    rect: [f64; 4],
    to: &NewTarget,
) -> Result<ObjectId, DocumentError> {
    let page_id = page_object(target, page)?;
    let doc = target.document();
    let dict = doc
        .resolve_id(page_id)
        .as_dict()
        .cloned()
        .unwrap_or_default();
    let space = PageSpace::of(doc, &dict);
    let action = new_action(target, to)?;
    let mut border = Dictionary::new();
    border.insert(b"Type".to_vec(), Object::name("Border"));
    border.insert(b"S".to_vec(), Object::name("S"));
    border.insert(b"W".to_vec(), Object::Integer(0));
    let mut annot = Dictionary::new();
    annot.insert(b"Type".to_vec(), Object::name("Annot"));
    annot.insert(b"Subtype".to_vec(), Object::name("Link"));
    annot.insert(b"Rect".to_vec(), rect_object(space.to_user(rect)));
    annot.insert(b"BS".to_vec(), Object::Dictionary(border));
    annot.insert(b"A".to_vec(), action);
    annot.insert(b"P".to_vec(), Object::Reference(page_id));
    let id = target.allocate(Object::Dictionary(annot))?;
    // A /Annots that is not an array is replaced in the page itself: the
    // object it names (a font, the page itself) is not rewritten.
    let place = annots_array(target.document(), page_id).unwrap_or(Place {
        id: page_id,
        keys: vec![b"Annots".to_vec()],
    });
    place.update(target, |value| {
        if !matches!(value, Object::Array(_)) {
            *value = Object::Array(Vec::new());
        }
        if let Some(annots) = value.as_array_mut() {
            annots.push(Object::Reference(id));
        }
    })?;
    Ok(id)
}

/// link-set: a new rect (page space) and/or a new target for the link held
/// by `holder` on page `page`. A new target replaces /A and drops /Dest.
pub fn set<E: Edit>(
    target: &mut E,
    page: usize,
    holder: ObjectId,
    rect: Option<[f64; 4]>,
    to: Option<&NewTarget>,
) -> Result<(), DocumentError> {
    let action = match to {
        Some(to) => Some(new_action(target, to)?),
        None => None,
    };
    let rect = match rect {
        Some(rect) => {
            let doc = target.document();
            let pages = info::pages(doc);
            let space = pages.get(page).map(|page| PageSpace::of(doc, page.dict));
            space.map(|space| rect_object(space.to_user(rect)))
        }
        None => None,
    };
    target.update(holder, |value| {
        if let Some(dict) = value.as_dict_mut() {
            if let Some(rect) = rect {
                dict.insert(b"Rect".to_vec(), rect);
            }
            if let Some(action) = action {
                dict.remove(b"Dest".as_slice());
                dict.insert(b"A".to_vec(), action);
            }
        }
    })
}

/// link-delete: the /Annots entry at `position` on page `page` goes, as
/// `remove_many` removes it.
pub fn remove<E: Edit>(target: &mut E, page: usize, position: usize) -> Result<(), DocumentError> {
    remove_many(target, &[(page, position)])
}

/// Removes the /Annots entry at each `(page, position)`, positions as they
/// are before any removal. When no page lists a removed annotation any
/// more, references to it elsewhere (a structure OBJR, another annotation's
/// /Popup or /IRT) go too (`annots::cut_unlisted`), so a save does not keep
/// it: one pass over the document for the whole set, so a batch should
/// remove its links together.
pub fn remove_many<E: Edit>(
    target: &mut E,
    entries: &[(usize, usize)],
) -> Result<(), DocumentError> {
    let mut by_page: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for &(page, position) in entries {
        by_page.entry(page).or_default().insert(position);
    }
    let mut removed = Vec::new();
    for (page, positions) in by_page {
        let page = page_object(target, page)?;
        let Some(place) = annots_array(target.document(), page) else {
            continue;
        };
        let taken = place.update(target, |value| {
            let Some(annots) = value.as_array_mut() else {
                return Vec::new();
            };
            let mut taken = Vec::new();
            let mut at = 0usize;
            annots.retain(|entry| {
                let keep = !positions.contains(&at);
                if !keep {
                    taken.push(entry.clone());
                }
                at += 1;
                keep
            });
            taken
        })?;
        removed.extend(taken);
    }
    let doc = target.document();
    // Each removed annotation: its reference and the object it resolves to.
    let groups: Vec<BTreeSet<ObjectId>> = (removed.iter())
        .filter_map(|entry| {
            let id = entry.as_reference()?;
            let holder = doc.resolve_with_holder(entry).0;
            Some([Some(id), holder].into_iter().flatten().collect())
        })
        .collect();
    annots::cut_unlisted(target, &groups)
}
