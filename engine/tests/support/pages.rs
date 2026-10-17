// SPDX-License-Identifier: AGPL-3.0-or-later
//! Page lookups for the document-layer tests, read through the app's page tree.
#![allow(dead_code)]
use tandem_pdf_engine::{
    app::pagetree::{PageTree, inherited},
    document::Document,
    object::{Dictionary, Object, ObjectId},
};

/// The page objects in page order.
pub fn page_ids(doc: &Document<'_>) -> Vec<ObjectId> {
    PageTree::read(doc)
        .unwrap()
        .slots
        .iter()
        .map(|slot| slot.id.unwrap())
        .collect()
}

/// A page's dictionary.
pub fn page<'d>(doc: &'d Document<'_>, id: ObjectId) -> &'d Dictionary {
    doc.resolve_id(id).as_dict().unwrap()
}

/// A page's content streams in order: one reference, or an array of them,
/// directly or behind a reference.
pub fn contents(doc: &Document<'_>, id: ObjectId) -> Vec<ObjectId> {
    let Some(mut value) = page(doc, id).get(b"Contents".as_slice()) else {
        return Vec::new();
    };
    if let Some(target) = value.as_reference()
        && doc.resolve_id(target).as_array().is_some()
    {
        value = doc.resolve_id(target);
    }
    match value {
        Object::Array(items) => items.iter().filter_map(Object::as_reference).collect(),
        value => value.as_reference().into_iter().collect(),
    }
}

/// A page's /Rotate, inherited, 0 when absent.
pub fn rotation(doc: &Document<'_>, id: ObjectId) -> i64 {
    match inherited(doc, page(doc, id), b"Rotate").map(|value| doc.resolve(value)) {
        None => 0,
        Some(Object::Integer(value)) => *value,
        Some(other) => panic!("/Rotate is {other:?}"),
    }
}

/// The first page's /Rotate.
pub fn first_rotation(doc: &Document<'_>) -> i64 {
    rotation(doc, page_ids(doc)[0])
}

/// A page's /MediaBox, inherited and resolved.
pub fn media_box<'d>(doc: &'d Document<'_>, id: ObjectId) -> Option<&'d Object> {
    inherited(doc, page(doc, id), b"MediaBox").map(|value| doc.resolve(value))
}
