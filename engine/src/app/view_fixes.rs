// SPDX-License-Identifier: AGPL-3.0-or-later
//! Objects the app's view redefines in its private bytes (never journaled or
//! saved) so that hayro draws what MuPDF's `toPixmap` with View usage draws:
//! the layers panel's visibility as an edited default configuration, the
//! annotations View usage leaves out marked Hidden, and Letter for a page
//! whose MediaBox is missing or empty. A redefinition overrides the object
//! wherever the bytes store it, object streams included.
use std::collections::{BTreeMap, BTreeSet};

use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};

use super::{info, layers};

const LETTER: [i64; 4] = [0, 0, 612, 792];

/// The view's redefinitions for `doc` under the panel's `overrides`.
pub(crate) fn redefinitions(
    doc: &Document<'_>,
    overrides: &BTreeMap<ObjectId, bool>,
) -> BTreeMap<ObjectId, Object> {
    let mut fixes = Fixes {
        doc,
        objects: BTreeMap::new(),
    };
    if let Some((id, value)) = layers::edited_config(doc, overrides) {
        fixes.objects.insert(id, value);
    }
    let off = layers::hidden_groups(doc, overrides);
    for page in info::pages(doc) {
        let Some(page_id) = page.id else {
            continue;
        };
        if media_missing(doc, page.dict) {
            // MuPDF's page is then Letter whatever the CropBox says; hayro
            // would clip to the CropBox within its own empty or A4 box.
            fixes.edit(page_id, |page| {
                let letter = Object::Array(LETTER.map(Object::Integer).to_vec());
                if let Some(page) = page.as_dict_mut() {
                    page.insert(b"CropBox".to_vec(), letter.clone());
                    page.insert(b"MediaBox".to_vec(), letter);
                }
            });
        }
        let Some(annots) = page.dict.get(b"Annots".as_slice()) else {
            continue;
        };
        let (holder, items) = doc.resolve_with_holder(annots);
        let Some(items) = items.as_array() else {
            continue;
        };
        let mut direct = BTreeSet::new();
        for (index, item) in items.iter().enumerate() {
            match item {
                Object::Reference(_) => {
                    let (annot_id, annot) = doc.resolve_with_holder(item);
                    if let (Some(annot_id), Some(annot)) = (annot_id, annot.as_dict())
                        && hidden(doc, annot, &off)
                    {
                        fixes.edit(annot_id, mark_hidden);
                    }
                }
                Object::Dictionary(annot) if hidden(doc, annot, &off) => {
                    direct.insert(index);
                }
                _ => {}
            }
        }
        if direct.is_empty() {
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
                }
            }
        };
        match holder {
            Some(array_id) => fixes.edit(array_id, mark),
            None => fixes.edit(page_id, |page| {
                if let Some(array) = page
                    .as_dict_mut()
                    .and_then(|p| p.get_mut(b"Annots".as_slice()))
                {
                    mark(array);
                }
            }),
        }
    }
    fixes.objects
}

struct Fixes<'d, 'a> {
    doc: &'d Document<'a>,
    objects: BTreeMap<ObjectId, Object>,
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
