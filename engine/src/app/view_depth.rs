// SPDX-License-Identifier: AGPL-3.0-or-later
//! The app parses and writes 256 nested arrays and dictionaries (MuPDF
//! reads structure trees deeper than 64 levels), but the view's parser
//! (hayro) refuses an object nested past 64 and, for one it needs such as
//! the catalog, the whole document. In the view's private update such an
//! object is redefined with its containers past that depth cut to null;
//! the document and its saves keep them.
use std::collections::BTreeMap;

use crate::document::Document;
use crate::object::{Object, ObjectId};

/// hayro-syntax's `MAX_OBJECT_NESTING_DEPTH`: containers at depth 64 (the
/// object's own value at 0) are refused.
const VIEW_DEPTH: usize = 64;

/// Adds to `fixes` (the view's redefined objects, used where they already
/// redefine one) every non-stream object nested past `VIEW_DEPTH`, cut.
pub(crate) fn shallow(doc: &Document<'_>, fixes: &mut BTreeMap<ObjectId, Object>) {
    let mut cut = Vec::new();
    for object in doc.objects() {
        if object.stream.is_some() {
            continue;
        }
        let value = fixes.get(&object.id).unwrap_or(&object.value);
        if too_deep(value) {
            cut.push((object.id, clamp(value, 0)));
        }
    }
    fixes.extend(cut);
}

fn too_deep(value: &Object) -> bool {
    let mut stack = vec![(value, 0usize)];
    while let Some((value, depth)) = stack.pop() {
        match value {
            Object::Array(items) => {
                if depth >= VIEW_DEPTH {
                    return true;
                }
                stack.extend(items.iter().map(|item| (item, depth + 1)));
            }
            Object::Dictionary(dict) => {
                if depth >= VIEW_DEPTH {
                    return true;
                }
                stack.extend(dict.values().map(|item| (item, depth + 1)));
            }
            _ => {}
        }
    }
    false
}

/// `value` with every container at `VIEW_DEPTH` or deeper null (recursion
/// stops there).
fn clamp(value: &Object, depth: usize) -> Object {
    match value {
        Object::Array(_) | Object::Dictionary(_) if depth >= VIEW_DEPTH => Object::Null,
        Object::Array(items) => {
            Object::Array(items.iter().map(|item| clamp(item, depth + 1)).collect())
        }
        Object::Dictionary(dict) => Object::Dictionary(
            dict.iter()
                .map(|(key, item)| (key.clone(), clamp(item, depth + 1)))
                .collect(),
        ),
        other => other.clone(),
    }
}
