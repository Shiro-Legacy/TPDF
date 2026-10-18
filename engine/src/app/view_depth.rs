// SPDX-License-Identifier: AGPL-3.0-or-later
//! The app parses and writes 256 nested arrays and dictionaries (reference
//! viewers read structure trees deeper than 64 levels), but the view's parser
//! (hayro) refuses an object nested past 64 and, for one it needs such as
//! the catalog, the whole document. In the view's private update such an
//! object is redefined with its containers past that depth cut to null;
//! the document and its saves keep them.
//!
//! The objects nested that deep are found by one walk over the document on
//! the first view, then kept up to date from the objects each change
//! touches, so a view after an edit costs what the edit changed.
use std::collections::{BTreeMap, BTreeSet};

use crate::document::Document;
use crate::object::{Object, ObjectId};

/// hayro-syntax's `MAX_OBJECT_NESTING_DEPTH`: containers at depth 64 (the
/// object's own value at 0) are refused.
const VIEW_DEPTH: usize = 64;

/// The non-stream objects nested past `VIEW_DEPTH`, kept per document.
#[derive(Debug, Default)]
pub(crate) struct DeepObjects {
    scanned: bool,
    ids: BTreeSet<ObjectId>,
    /// Objects changed since the set was last brought up to date.
    dirty: BTreeSet<ObjectId>,
}

impl DeepObjects {
    /// Records objects a change touched.
    pub(crate) fn note(&mut self, changed: impl IntoIterator<Item = ObjectId>) {
        if self.scanned {
            self.dirty.extend(changed);
        }
    }

    /// Adds to `fixes` (the view's redefined objects, used where they
    /// already redefine one) every non-stream object nested past
    /// `VIEW_DEPTH`, cut. `pending` are objects an open operation has
    /// touched, checked on every call until the operation ends.
    pub(crate) fn shallow(
        &mut self,
        doc: &Document<'_>,
        pending: impl IntoIterator<Item = ObjectId>,
        fixes: &mut BTreeMap<ObjectId, Object>,
    ) {
        if !self.scanned {
            self.ids = (doc.objects())
                .filter(|object| object.stream.is_none() && too_deep(&object.value))
                .map(|object| object.id)
                .collect();
            self.scanned = true;
            self.dirty.clear();
        }
        let mut check = std::mem::take(&mut self.dirty);
        check.extend(pending);
        for id in check {
            match doc.object(id) {
                Ok(object) if object.stream.is_none() && too_deep(&object.value) => {
                    self.ids.insert(id);
                }
                _ => {
                    self.ids.remove(&id);
                }
            }
        }
        // The redefinitions themselves, then the deep objects they leave.
        let mut cut = Vec::new();
        for (&id, value) in fixes.iter() {
            if too_deep(value) {
                cut.push((id, clamp(value, 0)));
            }
        }
        for &id in &self.ids {
            if fixes.contains_key(&id) {
                continue;
            }
            if let Ok(object) = doc.object(id) {
                cut.push((id, clamp(&object.value, 0)));
            }
        }
        fixes.extend(cut);
    }
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
