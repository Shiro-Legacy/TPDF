// SPDX-License-Identifier: AGPL-3.0-or-later
//! Objects the app's view redefines in its private bytes (never journaled or
//! saved) so that hayro draws what reference viewers draw for View usage:
//! the layers panel's visibility as an edited default configuration, the
//! annotations View usage leaves out (Invisible ones included) marked
//! Hidden, and Letter for a page whose MediaBox is missing or empty. A
//! redefinition overrides the object wherever the bytes store it, object
//! streams included. Annotations are
//! drawn as reference viewers draw them (`annots::view`): a missing
//! appearance synthesized, NoZoom and NoRotate honoured, /F read as `flags`
//! reads it.
//!
//! Pages: the view reads a page's inheritable attributes through /Parent,
//! hayro along the /Kids path it walked; where the two differ (a direct or
//! foreign /Parent, none at all) the page gets the /Resources, boxes and
//! /Rotate read through /Parent written onto it. hayro reads the page tree only
//! from a /Pages reference: a direct /Pages dictionary becomes a new object,
//! and without a usable page tree the catalog points at a new /Pages node
//! listing the /Type /Page objects (`info::pages`), where hayro would list
//! every dictionary with /Contents. A node the app reads as one without /Type
//! /Pages (untyped, with /Kids: `info::is_node`) is typed /Pages, which hayro
//! needs to walk it rather than draw it as a page. Device colour is drawn
//! through the page's default spaces or the output intent (`color`), a form or
//! appearance finds the names its own resources lack in the enclosing ones
//! (`inherit`), a CID font without a program draws only the glyphs it can place
//! (`cidfont`), transfer functions are left out (`gstate`), and run-length
//! image data gets its missing end byte (`runlength`).
mod cidfont;
mod cmap;
mod color;
mod gstate;
mod inherit;
mod runlength;
mod tiling;

pub(crate) use color::Memo;

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};

use super::annots::{self, PageSpace};
use super::info::{KidsHolder, PageRef, PageSlot};
use super::overlay::NewStreams;
use super::{info, layers, pagetree};

const LETTER: [i64; 4] = [0, 0, 612, 792];

/// The view's redefinitions for `doc` under the panel's `overrides`, and
/// the streams they add. `memo` keeps content rewrites for the next view.
pub(crate) fn redefinitions(
    doc: &Document<'_>,
    overrides: &BTreeMap<ObjectId, bool>,
    memo: &mut Memo,
) -> (BTreeMap<ObjectId, Object>, NewStreams) {
    let mut fixes = Fixes {
        doc,
        objects: BTreeMap::new(),
        streams: NewStreams::new(),
        budget: annots::view::Budget::default(),
        number: doc.next_id().ok().map(|id| id.number()),
        off: layers::hidden_groups(doc, overrides),
        arrays: BTreeSet::new(),
        shared: BTreeMap::new(),
        recolor: color::Recolor::new(doc, std::mem::take(memo)),
    };
    if let Some((id, value)) = layers::edited_config(doc, overrides) {
        fixes.objects.insert(id, value);
    }
    let walked = info::walk(doc, |step| match step {
        info::Step::Page(page, path, slot) => {
            let foreign = !info::same_ancestry(doc, page.dict, path);
            fixes.page(page, slot, foreign);
            fixes.walk_page(slot, path);
        }
        // hayro walks only a /Type /Pages node; the app reads an untyped
        // node with /Kids as one too (`info::is_node`).
        info::Step::Node(node, slot) => {
            if node.dict.get(b"Type".as_slice()) != Some(&Object::name("Pages")) {
                fixes.edit_page(slot, |node| {
                    node.insert(b"Type".to_vec(), Object::name("Pages"));
                });
            }
        }
    });
    let direct_root = (doc.resolve_id(doc.root()).as_dict())
        .and_then(|catalog| catalog.get(b"Pages".as_slice()))
        .is_some_and(|pages| matches!(pages, Object::Dictionary(_)));
    if walked && direct_root {
        // The direct /Pages dictionary, as the pages above edited it.
        if let Some(node) = fixes.next() {
            if let Some(root) = fixes.point_catalog(node) {
                fixes.objects.insert(node, root);
            }
        }
    } else if !walked {
        let listed = info::pages(doc);
        if let Some(node) = fixes.next() {
            let kids = (listed.iter())
                .filter_map(|page| page.id.map(Object::Reference))
                .collect::<Vec<_>>();
            let count = Object::Integer(kids.len() as i64);
            let tree = Object::dictionary([
                ("Type", Object::name("Pages")),
                ("Kids", Object::Array(kids)),
                ("Count", count),
            ]);
            fixes.objects.insert(node, tree);
            fixes.point_catalog(node);
        }
        // hayro inherits nothing from the new node; the view through /Parent.
        for page in listed {
            if let Some(id) = page.id {
                let foreign = !info::same_ancestry(doc, page.dict, &[]);
                fixes.page(page, PageSlot::Object(id), foreign);
                fixes.walk_page(PageSlot::Object(id), &[]);
            }
        }
    }
    *memo = std::mem::take(&mut fixes.recolor.memo);
    (fixes.objects, fixes.streams)
}

struct Fixes<'d, 'a> {
    doc: &'d Document<'a>,
    objects: BTreeMap<ObjectId, Object>,
    streams: NewStreams,
    budget: annots::view::Budget,
    /// The number the next new object takes.
    number: Option<u32>,
    /// Optional-content groups the panel shows off.
    off: BTreeSet<ObjectId>,
    /// /Annots array objects already fixed: several pages may share one.
    arrays: BTreeSet<ObjectId>,
    /// The new object written for each inherited direct /Resources value,
    /// by its address in the document: pages sharing it share the object.
    shared: BTreeMap<usize, ObjectId>,
    recolor: color::Recolor,
}

/// How the view treats an annotation.
enum Verdict {
    /// The view leaves it out and hayro would draw it: marked Hidden, with
    /// the flags `flags` reads.
    Hide(u32),
    /// The view draws it, from the dictionary as stored or, when hayro
    /// reads its /F otherwise, with the flags `flags` reads as an integer.
    Draw(Option<Dictionary>),
    /// Neither draws it.
    Skip,
}

impl<'d> Fixes<'d, '_> {
    fn next(&mut self) -> Option<ObjectId> {
        let id = ObjectId::new(self.number?, 0).ok()?;
        self.number = self.number?.checked_add(1);
        Some(id)
    }

    /// The fixes of one page, written where `slot` says.
    fn page(&mut self, page: PageRef<'d>, slot: PageSlot<'_>, foreign: bool) {
        let doc = self.doc;
        if foreign {
            let attributes = self.inherited_attributes(page.dict);
            self.edit_page(slot, |page| {
                for (key, value) in attributes {
                    page.insert(key.to_vec(), value);
                }
            });
        }
        if media_missing(doc, page.dict) {
            // An empty or absent MediaBox is US Letter; a CropBox inside it
            // still applies (spec 04 section 5.1, decision T4).
            self.edit_page(slot, |page| {
                let letter = Object::Array(LETTER.map(Object::Integer).to_vec());
                page.insert(b"MediaBox".to_vec(), letter);
            });
        }
        if crop_misses(doc, page.dict) {
            // A CropBox that misses the MediaBox is ignored: the page shows
            // its MediaBox (spec 04 section 5.1, decision T4).
            self.edit_page(slot, |page| {
                page.remove(b"CropBox".as_slice());
            });
        }
        let Some(annots) = page.dict.get(b"Annots".as_slice()) else {
            return;
        };
        let (holder, items) = doc.resolve_with_holder(annots);
        let Some(items) = items.as_array() else {
            return;
        };
        // An /Annots array object several pages share is fixed once, for
        // the first page that lists it.
        if holder.is_some_and(|array| !self.arrays.insert(array)) {
            return;
        }
        let space = PageSpace::of(doc, page.dict);
        let mut direct = BTreeMap::new();
        let mut drawn = BTreeMap::new();
        for (index, item) in items.iter().enumerate() {
            match item {
                Object::Reference(_) => {
                    let (annot_id, annot) = doc.resolve_with_holder(item);
                    let (Some(annot_id), Some(annot)) = (annot_id, annot.as_dict()) else {
                        continue;
                    };
                    match self.verdict(annot) {
                        Verdict::Hide(flags) => {
                            self.edit(annot_id, |annot| mark_hidden(annot, flags))
                        }
                        Verdict::Draw(normalized) if !self.objects.contains_key(&annot_id) => {
                            if let Some(value) = self.drawn(annot, normalized, &space) {
                                self.objects.insert(annot_id, Object::Dictionary(value));
                            }
                        }
                        _ => {}
                    }
                }
                Object::Dictionary(annot) => match self.verdict(annot) {
                    Verdict::Hide(flags) => {
                        direct.insert(index, flags);
                    }
                    Verdict::Draw(normalized) => {
                        if let Some(value) = self.drawn(annot, normalized, &space) {
                            drawn.insert(index, value);
                        }
                    }
                    Verdict::Skip => {}
                },
                _ => {}
            }
        }
        if direct.is_empty() && drawn.is_empty() {
            return;
        }
        // A direct annotation is edited inside its array, wherever that lives.
        let mark = |array: &mut Object| {
            let Some(items) = array.as_array_mut() else {
                return;
            };
            for (index, item) in items.iter_mut().enumerate() {
                if let Some(&flags) = direct.get(&index) {
                    mark_hidden(item, flags);
                } else if let Some(value) = drawn.get(&index) {
                    *item = Object::Dictionary(value.clone());
                }
            }
        };
        match holder {
            Some(array_id) => self.edit(array_id, mark),
            None => self.edit_page(slot, |page| {
                if let Some(array) = page.get_mut(b"Annots".as_slice()) {
                    mark(array);
                }
            }),
        }
    }

    /// Whether View usage draws `annot` and hayro would (`flags`).
    fn verdict(&self, annot: &Dictionary) -> Verdict {
        let doc = self.doc;
        let value = doc.dict_get(annot, b"F");
        let (wanted, hayro) = (flags(value), hayro_flags(value));
        // Invisible (1), Hidden (2) or NoView (32), whatever the type:
        // reference viewers leave out an invisible annotation of a standard
        // type too, which the specification would draw.
        let left_out = wanted & (1 | 2 | 32) != 0
            || doc.dict_get(annot, b"Subtype").as_name() == Some(b"Popup")
            || content_off(doc, annot, &self.off);
        // hayro draws only an appearance; the forms view gives a widget
        // without one an appearance after these fixes.
        let drawable = annot.contains_key(b"AP".as_slice())
            || doc.dict_get(annot, b"Subtype").as_name() == Some(b"Widget");
        if left_out {
            return match hayro & 2 == 0 && drawable {
                true => Verdict::Hide(wanted),
                false => Verdict::Skip,
            };
        }
        if wanted == hayro {
            return Verdict::Draw(None);
        }
        let mut normalized = annot.clone();
        normalized.insert(b"F".to_vec(), Object::Integer(i64::from(wanted)));
        Verdict::Draw(Some(normalized))
    }

    /// What the view writes for an annotation it draws: the appearance
    /// `annots::view` gives it, else the dictionary with `flags`' value when
    /// hayro reads them otherwise; None to leave it as stored.
    fn drawn(
        &mut self,
        annot: &Dictionary,
        normalized: Option<Dictionary>,
        space: &PageSpace,
    ) -> Option<Dictionary> {
        let source = normalized.as_ref().unwrap_or(annot);
        let mut number = self.number;
        let mut next = || {
            let id = ObjectId::new(number?, 0).ok()?;
            number = number?.checked_add(1);
            Some(id)
        };
        let fix = annots::view::fix(self.doc, source, space, &mut self.budget, &mut next);
        self.number = number;
        match fix {
            Some(fix) => {
                (self.streams).extend(fix.streams.into_iter().map(|(id, d, b)| (id, (d, b))));
                Some(fix.value)
            }
            None => normalized,
        }
    }

    /// The page's /Resources, /MediaBox, /CropBox and /Rotate as read
    /// through /Parent (`pagetree::inherited`, `info::page_box`, the
    /// displayed rotation), to write onto the page. Resources that are not a
    /// dictionary are empty ones, and the page's own dictionary is left as
    /// it is; a missing MediaBox is left to the Letter fix and a missing
    /// CropBox is the MediaBox.
    fn inherited_attributes(&mut self, page: &Dictionary) -> Vec<(&'static [u8], Object)> {
        let doc = self.doc;
        let mut out = Vec::new();
        match pagetree::inherited(doc, page, b"Resources") {
            Some(value) if doc.resolve(value).as_dict().is_some() => {
                let own = page.get(b"Resources".as_slice());
                if !own.is_some_and(|own| std::ptr::eq(own, value)) {
                    let value = match value {
                        Object::Reference(_) => Some(value.clone()),
                        _ => self.shared(value),
                    };
                    out.extend(value.map(|value| (b"Resources".as_slice(), value)));
                }
            }
            _ => out.push((
                b"Resources".as_slice(),
                Object::Dictionary(Dictionary::new()),
            )),
        }
        let rect = |r: [f64; 4]| Object::Array(r.iter().map(|&v| number(v)).collect());
        let media = info::page_box(doc, page, b"MediaBox");
        let crop = info::page_box(doc, page, b"CropBox").or(media);
        let rotate = Object::Integer(i64::from(info::display_rotation(doc, page)));
        out.push((b"Rotate", rotate));
        out.extend(media.map(|r| (b"MediaBox".as_slice(), rect(r))));
        out.extend(crop.map(|r| (b"CropBox".as_slice(), rect(r))));
        out
    }

    /// A reference to a new object holding the direct `value` of the
    /// document, written once however many pages inherit it.
    fn shared(&mut self, value: &Object) -> Option<Object> {
        let at = std::ptr::from_ref(value).addr();
        let id = match self.shared.get(&at) {
            Some(&id) => id,
            None => {
                let id = self.next()?;
                self.objects.insert(id, value.clone());
                self.shared.insert(at, id);
                id
            }
        };
        Some(Object::Reference(id))
    }

    /// Points the view's catalog at the /Pages node `node`, returning the
    /// /Pages value it replaces.
    fn point_catalog(&mut self, node: ObjectId) -> Option<Object> {
        let root = self.doc.root();
        let catalog = match self.objects.entry(root) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let value = self.doc.object(root).map(|object| object.value.clone());
                entry.insert(value.unwrap_or(Object::Null))
            }
        };
        if catalog.as_dict().is_none() {
            *catalog = Object::dictionary([("Type", Object::name("Catalog"))]);
        }
        let catalog = catalog.as_dict_mut()?;
        catalog.insert(b"Pages".to_vec(), Object::Reference(node))
    }

    /// Applies `change` to the view's value of `id`. Streams are left alone:
    /// a redefinition carries no stream data.
    fn edit(&mut self, id: ObjectId, change: impl FnOnce(&mut Object)) {
        let value = match self.objects.get_mut(&id) {
            Some(value) => value,
            None => {
                let Ok(object) = self.doc.object(id) else {
                    return;
                };
                if object.stream.is_some() {
                    return;
                }
                self.objects.entry(id).or_insert(object.value.clone())
            }
        };
        change(value);
    }

    /// Applies `change` to the view's copy of a page dictionary wherever it
    /// is written: its own object, or the /Kids arrays that lead to it.
    fn edit_page(&mut self, slot: PageSlot<'_>, change: impl FnOnce(&mut Dictionary)) {
        let (holder, nodes, index) = match slot {
            PageSlot::Object(id) => {
                return self.edit(id, |page| {
                    if let Some(page) = page.as_dict_mut() {
                        change(page);
                    }
                });
            }
            PageSlot::Kid {
                holder,
                nodes,
                index,
            } => (holder, nodes, index),
        };
        let reach = move |value: &mut Object, kids_of: &[&[u8]]| {
            let mut kids = Some(value);
            for key in kids_of {
                kids = kids
                    .and_then(Object::as_dict_mut)
                    .and_then(|dict| dict.get_mut(*key));
            }
            for &node in nodes {
                kids = (kids.and_then(Object::as_array_mut))
                    .and_then(|kids| kids.get_mut(node))
                    .and_then(Object::as_dict_mut)
                    .and_then(|node| node.get_mut(b"Kids".as_slice()));
            }
            let page = (kids.and_then(Object::as_array_mut))
                .and_then(|kids| kids.get_mut(index))
                .and_then(Object::as_dict_mut);
            if let Some(page) = page {
                change(page);
            }
        };
        match holder {
            KidsHolder::Array(id) => self.edit(id, |value| reach(value, &[])),
            KidsHolder::Node(id) => self.edit(id, |value| reach(value, &[b"Kids"])),
            KidsHolder::Root => {
                let root = self.doc.root();
                self.edit(root, |value| reach(value, &[b"Pages", b"Kids"]));
            }
        }
    }
}

/// A rect's number as an integer when it is one.
fn number(v: f64) -> Object {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        Object::Integer(v as i64)
    } else {
        Object::Real(v)
    }
}

fn empty(r: &[f64; 4]) -> bool {
    !(r[0] < r[2] && r[1] < r[3])
}

/// A CropBox present but empty, or with no overlap with the page's MediaBox
/// (Letter when that is missing or empty).
fn crop_misses(doc: &Document<'_>, page: &Dictionary) -> bool {
    let Some(crop) = info::page_box(doc, page, b"CropBox") else {
        return false;
    };
    let media = info::page_box(doc, page, b"MediaBox")
        .filter(|r| !empty(r))
        .unwrap_or(LETTER.map(|v| v as f64));
    let normal = |r: [f64; 4]| {
        [
            r[0].min(r[2]),
            r[1].min(r[3]),
            r[0].max(r[2]),
            r[1].max(r[3]),
        ]
    };
    let (c, m) = (normal(crop), normal(media));
    empty(&[
        c[0].max(m[0]),
        c[1].max(m[1]),
        c[2].min(m[2]),
        c[3].min(m[3]),
    ])
}

/// A MediaBox the view replaces with Letter: missing, not an array, or empty.
fn media_missing(doc: &Document<'_>, page: &Dictionary) -> bool {
    info::page_box(doc, page, b"MediaBox").is_none_or(|r| empty(&r))
}

/// An annotation's /F as the view reads it: an integer cast to
/// 32 bits, a real (a float) rounded half up, 0 past the 32-bit range; 0
/// for anything else.
fn flags(value: &Object) -> u32 {
    match value {
        Object::Integer(flags) => *flags as i32 as u32,
        Object::Real(flags) => {
            let rounded = (*flags as f32 + 0.5).floor();
            match (-2_147_483_648.0..2_147_483_648.0).contains(&rounded) {
                true => rounded as i32 as u32,
                false => 0,
            }
        }
        _ => 0,
    }
}

/// An annotation's /F as hayro reads it: truncated, 0 outside u32.
fn hayro_flags(value: &Object) -> u32 {
    let wide = match value {
        Object::Integer(flags) => *flags,
        Object::Real(flags) => *flags as i64,
        _ => 0,
    };
    u32::try_from(wide).unwrap_or(0)
}

/// Sets the Hidden flag (2), which hayro honours, on an annotation
/// dictionary, over `flags`' value.
fn mark_hidden(annot: &mut Object, flags: u32) {
    if let Some(annot) = annot.as_dict_mut() {
        annot.insert(b"F".to_vec(), Object::Integer(i64::from(flags | 2)));
    }
}

/// The view's `content_off` on the core document: a group that is off, or
/// a membership dictionary (/OCGs and /P, default AnyOn) that evaluates off.
fn content_off(doc: &Document<'_>, annot: &Dictionary, off: &BTreeSet<ObjectId>) -> bool {
    let Some(value) = annot.get(b"OC".as_slice()) else {
        return false;
    };
    let Some(oc) = doc.resolve(value).as_dict() else {
        return false;
    };
    let membership = doc.dict_get(oc, b"Type").as_name() == Some(b"OCMD");
    if let (false, Object::Reference(group)) = (membership, value) {
        return off.contains(group);
    }
    let groups: Vec<bool> = match oc.get(b"OCGs".as_slice()) {
        Some(value) => match doc.resolve(value) {
            Object::Array(items) => (items.iter())
                .filter_map(|item| match item {
                    Object::Reference(group) => Some(off.contains(group)),
                    _ => None,
                })
                .collect(),
            _ => match value {
                Object::Reference(group) => vec![off.contains(group)],
                _ => Vec::new(),
            },
        },
        None => Vec::new(),
    };
    if groups.is_empty() {
        return false;
    }
    match doc.dict_get(oc, b"P").as_name() {
        Some(b"AllOn") => groups.iter().any(|&hidden| hidden),
        Some(b"AnyOff") => !groups.iter().any(|&hidden| hidden),
        Some(b"AllOff") => !groups.iter().all(|&hidden| hidden),
        _ => groups.iter().all(|&hidden| hidden),
    }
}
