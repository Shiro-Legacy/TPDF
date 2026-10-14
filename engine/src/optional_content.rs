// SPDX-License-Identifier: AGPL-3.0-or-later
//! Optional-content (layer) visibility under the document's default
//! configuration (/OCProperties /D): BaseState, ON and OFF. Membership
//! dictionaries use /VE when present, else /OCGs with /P.
//! ponytail: /AS usage auto-states and /Intent are ignored; add when a
//! document needs print- or zoom-dependent layers.

use std::collections::BTreeSet;

use crate::document::Document;
use crate::object::{Object, ObjectId};

const MAX_DEPTH: usize = 32;

#[derive(Debug, Default)]
pub struct OptionalContent {
    off: BTreeSet<ObjectId>,
}

fn resolve<'a>(doc: &'a Document<'_>, value: &'a Object) -> Option<&'a Object> {
    match value {
        Object::Reference(id) => doc.object(*id).ok().map(|o| &o.value),
        other => Some(other),
    }
}
fn get<'a>(doc: &'a Document<'_>, value: &'a Object, key: &[u8]) -> Option<&'a Object> {
    match resolve(doc, value)? {
        Object::Dictionary(d) => d.get(key).and_then(|v| resolve(doc, v)),
        _ => None,
    }
}
fn refs(doc: &Document<'_>, value: Option<&Object>) -> Vec<ObjectId> {
    match value.and_then(|v| resolve(doc, v)) {
        Some(Object::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                Object::Reference(id) => Some(*id),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

impl OptionalContent {
    /// The default configuration; documents without /OCProperties show everything.
    pub fn load(doc: &Document<'_>) -> Self {
        let root = Object::Reference(doc.root());
        let Some(properties) = get(doc, &root, b"OCProperties") else {
            return Self::default();
        };
        let config = get(doc, properties, b"D");
        let entry = |key: &[u8]| config.and_then(|d| get(doc, d, key));
        let off = if matches!(entry(b"BaseState"), Some(Object::Name(n)) if n == b"OFF") {
            let on: BTreeSet<_> = refs(doc, entry(b"ON")).into_iter().collect();
            refs(doc, get(doc, properties, b"OCGs"))
                .into_iter()
                .filter(|id| !on.contains(id))
                .collect()
        } else {
            refs(doc, entry(b"OFF")).into_iter().collect()
        };
        Self { off }
    }

    /// Visibility of an /OC value: an optional content group or membership
    /// dictionary. Unknown or malformed values are visible, as in other viewers.
    pub fn visible(&self, doc: &Document<'_>, value: &Object) -> bool {
        self.visible_at(doc, value, 0)
    }

    fn visible_at(&self, doc: &Document<'_>, value: &Object, depth: usize) -> bool {
        if depth > MAX_DEPTH {
            return true;
        }
        let group = |id: &ObjectId| !self.off.contains(id);
        let Some(Object::Dictionary(d)) = resolve(doc, value) else {
            return true;
        };
        match d.get(b"Type".as_slice()) {
            Some(Object::Name(n)) if n == b"OCG" => match value {
                Object::Reference(id) => group(id),
                _ => true,
            },
            Some(Object::Name(n)) if n == b"OCMD" => {
                if let Some(expression) = d.get(b"VE".as_slice()) {
                    return self.expression(doc, expression, depth + 1);
                }
                let groups: Vec<ObjectId> = match d.get(b"OCGs".as_slice()) {
                    Some(Object::Reference(id))
                        if matches!(
                            resolve(doc, &Object::Reference(*id)),
                            Some(Object::Dictionary(_))
                        ) =>
                    {
                        vec![*id]
                    }
                    other => refs(doc, other),
                };
                if groups.is_empty() {
                    return true;
                }
                let mut states = groups.iter().map(group);
                match d.get(b"P".as_slice()).and_then(|p| resolve(doc, p)) {
                    Some(Object::Name(p)) if p == b"AllOn" => states.all(|on| on),
                    Some(Object::Name(p)) if p == b"AnyOff" => states.any(|on| !on),
                    Some(Object::Name(p)) if p == b"AllOff" => states.all(|on| !on),
                    _ => states.any(|on| on),
                }
            }
            _ => true,
        }
    }

    fn expression(&self, doc: &Document<'_>, value: &Object, depth: usize) -> bool {
        if depth > MAX_DEPTH {
            return true;
        }
        if let Object::Reference(id) = value {
            if !matches!(resolve(doc, value), Some(Object::Array(_))) {
                return !self.off.contains(id);
            }
        }
        let Some(Object::Array(items)) = resolve(doc, value) else {
            return true;
        };
        let Some(Object::Name(op)) = items.first() else {
            return true;
        };
        let mut operands = items[1..]
            .iter()
            .map(|v| self.expression(doc, v, depth + 1));
        match op.as_slice() {
            b"And" => operands.all(|v| v),
            b"Or" => operands.any(|v| v),
            b"Not" => !operands.next().unwrap_or(false),
            _ => true,
        }
    }
}
