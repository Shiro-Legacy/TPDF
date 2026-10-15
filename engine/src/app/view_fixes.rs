// SPDX-License-Identifier: AGPL-3.0-or-later
//! Objects the app's view redefines in its private bytes (never journaled or
//! saved) so that hayro draws what MuPDF's `toPixmap` with View usage draws:
//! the layers panel's visibility as an edited default configuration, the
//! annotations View usage leaves out marked Hidden, and Letter for a page
//! whose MediaBox is missing or empty. A redefinition overrides the object
//! wherever the bytes store it, object streams included. Annotations are
//! drawn as MuPDF draws them (`annots::view`): a missing appearance
//! synthesized, NoZoom and NoRotate honoured.
//!
//! Pages: MuPDF reads a page's inheritable attributes through /Parent,
//! hayro along the /Kids path it walked; where the two differ (a direct or
//! foreign /Parent, none at all) the page gets MuPDF's /Resources, boxes and
//! /Rotate written onto it. Without a usable page tree the catalog points at
//! a new /Pages node listing the /Type /Page objects (`info::pages`), where
//! hayro would list every dictionary with /Contents.
use std::collections::{BTreeMap, BTreeSet};

use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};

use super::annots::{self, PageSpace};
use super::info::{PageRef, PageSlot, TreeNode};
use super::overlay::NewStreams;
use super::{info, layers, pagetree};

const LETTER: [i64; 4] = [0, 0, 612, 792];

/// The view's redefinitions for `doc` under the panel's `overrides`, and
/// the streams they add.
pub(crate) fn redefinitions(
    doc: &Document<'_>,
    overrides: &BTreeMap<ObjectId, bool>,
) -> (BTreeMap<ObjectId, Object>, NewStreams) {
    let mut fixes = Fixes {
        doc,
        objects: BTreeMap::new(),
    };
    let mut streams = NewStreams::new();
    let mut budget = annots::view::Budget::default();
    let mut number = doc.next_id().ok().map(|id| id.number());
    let mut next = || {
        let id = ObjectId::new(number?, 0).ok()?;
        number = number?.checked_add(1);
        Some(id)
    };
    if let Some((id, value)) = layers::edited_config(doc, overrides) {
        fixes.objects.insert(id, value);
    }
    let off = layers::hidden_groups(doc, overrides);
    // Each page, where it is written, and whether hayro inherits for it from
    // other nodes than MuPDF does.
    let mut pages: Vec<(PageRef<'_>, PageSlot, bool)> = Vec::new();
    let walked = info::walk_tree(doc, |page, path, slot| {
        pages.push((page, slot, !same_ancestry(doc, page.dict, path)));
    });
    if !walked {
        let listed = info::pages(doc);
        if let Some(node) = next() {
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
            let root = doc.root();
            let mut catalog = match fixes
                .objects
                .get(&root)
                .or_else(|| (doc.object(root).ok()).map(|object| &object.value))
            {
                Some(Object::Dictionary(catalog)) => catalog.clone(),
                _ => Dictionary::from([(b"Type".to_vec(), Object::name("Catalog"))]),
            };
            catalog.insert(b"Pages".to_vec(), Object::Reference(node));
            fixes.objects.insert(root, Object::Dictionary(catalog));
        }
        // hayro inherits nothing from the new node; MuPDF through /Parent.
        pages = (listed.into_iter())
            .filter_map(|page| {
                let foreign = !same_ancestry(doc, page.dict, &[]);
                Some((page, PageSlot::Object(page.id?), foreign))
            })
            .collect();
    }
    // An /Annots array object several pages share is fixed once, for the
    // first page that lists it.
    let mut arrays = BTreeSet::new();
    for (page, slot, foreign) in pages {
        if foreign {
            let attributes = inherited_attributes(doc, page.dict);
            fixes.edit_page(slot, |page| {
                for (key, value) in attributes {
                    page.insert(key.to_vec(), value);
                }
            });
        }
        if media_missing(doc, page.dict) {
            // MuPDF's page is then Letter whatever the CropBox says; hayro
            // would clip to the CropBox within its own empty or A4 box.
            fixes.edit_page(slot, |page| {
                let letter = Object::Array(LETTER.map(Object::Integer).to_vec());
                page.insert(b"CropBox".to_vec(), letter.clone());
                page.insert(b"MediaBox".to_vec(), letter);
            });
        }
        let Some(annots) = page.dict.get(b"Annots".as_slice()) else {
            continue;
        };
        let (holder, items) = doc.resolve_with_holder(annots);
        let Some(items) = items.as_array() else {
            continue;
        };
        if holder.is_some_and(|array| !arrays.insert(array)) {
            continue;
        }
        let space = PageSpace::of(doc, page.dict);
        let mut direct = BTreeSet::new();
        let mut drawn = BTreeMap::new();
        for (index, item) in items.iter().enumerate() {
            match item {
                Object::Reference(_) => {
                    let (annot_id, annot) = doc.resolve_with_holder(item);
                    let (Some(annot_id), Some(annot)) = (annot_id, annot.as_dict()) else {
                        continue;
                    };
                    if hidden(doc, annot, &off) {
                        fixes.edit(annot_id, mark_hidden);
                    } else if !left_out(doc, annot, &off) && !fixes.objects.contains_key(&annot_id)
                    {
                        let fix = annots::view::fix(doc, annot, &space, &mut budget, &mut next);
                        if let Some(fix) = fix {
                            fixes
                                .objects
                                .insert(annot_id, Object::Dictionary(fix.value));
                            streams.extend(fix.streams.into_iter().map(|(id, d, b)| (id, (d, b))));
                        }
                    }
                }
                Object::Dictionary(annot) if hidden(doc, annot, &off) => {
                    direct.insert(index);
                }
                Object::Dictionary(annot) if !left_out(doc, annot, &off) => {
                    if let Some(fix) = annots::view::fix(doc, annot, &space, &mut budget, &mut next)
                    {
                        drawn.insert(index, fix.value);
                        streams.extend(fix.streams.into_iter().map(|(id, d, b)| (id, (d, b))));
                    }
                }
                _ => {}
            }
        }
        if direct.is_empty() && drawn.is_empty() {
            continue;
        }
        // A direct annotation is edited inside its array, wherever that lives.
        let mark = |array: &mut Object| {
            let Some(items) = array.as_array_mut() else {
                return;
            };
            for (index, item) in items.iter_mut().enumerate() {
                if direct.contains(&index) {
                    mark_hidden(item);
                } else if let Some(value) = drawn.get(&index) {
                    *item = Object::Dictionary(value.clone());
                }
            }
        };
        match holder {
            Some(array_id) => fixes.edit(array_id, mark),
            None => fixes.edit_page(slot, |page| {
                if let Some(array) = page.get_mut(b"Annots".as_slice()) {
                    mark(array);
                }
            }),
        }
    }
    (fixes.objects, streams)
}

struct Fixes<'d, 'a> {
    doc: &'d Document<'a>,
    objects: BTreeMap<ObjectId, Object>,
}

/// Whether the /Parent chain of `page` names the nodes of `path` (root
/// first), nearest first: then MuPDF and hayro inherit from the same nodes.
fn same_ancestry(doc: &Document<'_>, page: &Dictionary, path: &[TreeNode<'_>]) -> bool {
    let mut node = page;
    for expected in path.iter().rev() {
        let parent = node.get(b"Parent".as_slice());
        let (id, dict) = match parent {
            Some(Object::Reference(id)) => (Some(*id), doc.resolve_id(*id).as_dict()),
            Some(Object::Dictionary(dict)) => (None, Some(dict)),
            _ => return false,
        };
        let Some(dict) = dict else {
            return false;
        };
        let same = match (id, expected.id) {
            (Some(id), Some(wanted)) => id == wanted,
            (None, None) => std::ptr::eq(dict, expected.dict),
            _ => false,
        };
        if !same {
            return false;
        }
        node = dict;
    }
    doc.resolve(node.get(b"Parent".as_slice()).unwrap_or(&Object::Null))
        .as_dict()
        .is_none()
}

/// The page's /Resources, /MediaBox, /CropBox and /Rotate as MuPDF reads
/// them through /Parent (`pagetree::inherited`, `info::page_box`, the
/// displayed rotation), to write onto the page. Resources that are not a
/// dictionary are empty ones; a missing MediaBox is left to the Letter fix
/// and a missing CropBox is the MediaBox.
fn inherited_attributes(doc: &Document<'_>, page: &Dictionary) -> Vec<(&'static [u8], Object)> {
    let resources = match pagetree::inherited(doc, page, b"Resources") {
        Some(value) if doc.resolve(value).as_dict().is_some() => value.clone(),
        _ => Object::Dictionary(Dictionary::new()),
    };
    let rect = |r: [f64; 4]| Object::Array(r.iter().map(|&v| number(v)).collect());
    let media = info::page_box(doc, page, b"MediaBox");
    let crop = info::page_box(doc, page, b"CropBox").or(media);
    let rotate = Object::Integer(i64::from(info::display_rotation(doc, page)));
    let mut out = vec![(b"Resources".as_slice(), resources), (b"Rotate", rotate)];
    out.extend(media.map(|r| (b"MediaBox".as_slice(), rect(r))));
    out.extend(crop.map(|r| (b"CropBox".as_slice(), rect(r))));
    out
}

/// A number as an integer when it is one.
fn number(v: f64) -> Object {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        Object::Integer(v as i64)
    } else {
        Object::Real(v)
    }
}

impl Fixes<'_, '_> {
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
    /// is written; a page inside a node written into /Kids is left alone.
    fn edit_page(&mut self, slot: PageSlot, change: impl FnOnce(&mut Dictionary)) {
        match slot {
            PageSlot::Object(id) => self.edit(id, |page| {
                if let Some(page) = page.as_dict_mut() {
                    change(page);
                }
            }),
            PageSlot::Kid {
                holder,
                in_node,
                index,
            } => self.edit(holder, |value| {
                let kids = match in_node {
                    true => value
                        .as_dict_mut()
                        .and_then(|node| node.get_mut(b"Kids".as_slice())),
                    false => Some(value),
                };
                let page = (kids.and_then(Object::as_array_mut))
                    .and_then(|kids| kids.get_mut(index))
                    .and_then(Object::as_dict_mut);
                if let Some(page) = page {
                    change(page);
                }
            }),
            PageSlot::Nested => {}
        }
    }
}

fn empty(r: &[f64; 4]) -> bool {
    !(r[0] < r[2] && r[1] < r[3])
}

/// A MediaBox MuPDF replaces with Letter: missing, not an array, or empty.
fn media_missing(doc: &Document<'_>, page: &Dictionary) -> bool {
    info::page_box(doc, page, b"MediaBox").is_none_or(|r| empty(&r))
}

/// An annotation's /F as hayro reads it: 0 unless an integer within u32.
fn flags(value: &Object) -> i64 {
    match value {
        Object::Integer(flags) if u32::try_from(*flags).is_ok() => *flags,
        _ => 0,
    }
}

/// Sets the Hidden flag (2), which hayro honours, on an annotation dictionary.
fn mark_hidden(annot: &mut Object) {
    if let Some(annot) = annot.as_dict_mut() {
        let flags = annot.get(b"F".as_slice()).map_or(0, flags);
        annot.insert(b"F".to_vec(), Object::Integer(flags | 2));
    }
}

/// Whether MuPDF's View usage leaves out an annotation hayro would draw: a
/// NoView or Popup annotation, or one whose /OC is off.
fn hidden(doc: &Document<'_>, annot: &Dictionary, off: &BTreeSet<ObjectId>) -> bool {
    let flags = flags(doc.dict_get(annot, b"F"));
    if flags & 2 != 0 || !annot.contains_key(b"AP".as_slice()) {
        return false;
    }
    flags & 32 != 0
        || doc.dict_get(annot, b"Subtype").as_name() == Some(b"Popup")
        || content_off(doc, annot, off)
}

/// Whether View usage leaves out an annotation whatever its appearance:
/// Hidden, NoView, a Popup, or an /OC that is off.
fn left_out(doc: &Document<'_>, annot: &Dictionary, off: &BTreeSet<ObjectId>) -> bool {
    let flags = flags(doc.dict_get(annot, b"F"));
    flags & (2 | 32) != 0
        || doc.dict_get(annot, b"Subtype").as_name() == Some(b"Popup")
        || content_off(doc, annot, off)
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
