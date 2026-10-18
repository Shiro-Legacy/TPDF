// SPDX-License-Identifier: MIT
//! Optional-content groups as the layers panel lists them, and the panel's
//! view-only visibility. The visibility is never
//! journaled or saved: the view reads it as an edited default configuration
//! (/OCProperties /D, the only one hayro reads) appended to its private bytes.
use std::collections::{BTreeMap, BTreeSet};

use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId, decode_text_string};

/// One optional-content group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    pub ocg: ObjectId,
    /// The group's /Name; empty when it is missing or not a string.
    pub name: String,
    pub visible: bool,
}

/// The panel's visibility over the default configuration, per group.
#[derive(Clone, Debug, Default)]
pub struct LayerState {
    overrides: BTreeMap<ObjectId, bool>,
    /// Bumped whenever the effective visibility changes; keys the view.
    serial: u64,
}

impl LayerState {
    pub fn serial(&self) -> u64 {
        self.serial
    }

    pub fn overrides(&self) -> &BTreeMap<ObjectId, bool> {
        &self.overrides
    }

    /// The groups with the panel's visibility applied.
    pub fn list(&self, doc: &Document<'_>) -> Vec<Layer> {
        let mut layers = groups(doc);
        for layer in &mut layers {
            if let Some(&visible) = self.overrides.get(&layer.ocg) {
                layer.visible = visible;
            }
        }
        layers
    }

    /// Shows or hides `ocg`. A setting equal to the configuration's own is
    /// dropped, so a toggle back restores the unedited view.
    pub fn set(&mut self, doc: &Document<'_>, ocg: ObjectId, visible: bool) {
        let default = groups(doc)
            .into_iter()
            .find(|layer| layer.ocg == ocg)
            .is_none_or(|layer| layer.visible);
        let current = self.overrides.get(&ocg).copied().unwrap_or(default);
        if current == visible {
            return;
        }
        if visible == default {
            self.overrides.remove(&ocg);
        } else {
            self.overrides.insert(ocg, visible);
        }
        self.serial += 1;
    }
}

/// The groups of /OCProperties /OCGs with the default configuration's
/// visibility, in descending object number (the order reference viewers
/// list; not the reverse of /OCGs that engine.ts says). Each group is listed
/// once and only references to dictionaries count: duplicates and entries
/// that are not groups, whose toggles would change nothing, are left out. A
/// missing /D leaves every group visible.
pub fn groups(doc: &Document<'_>) -> Vec<Layer> {
    let Some(properties) = properties(doc) else {
        return Vec::new();
    };
    let Some(ocgs) = doc.dict_get(properties, b"OCGs").as_array() else {
        return Vec::new();
    };
    let ids: BTreeSet<ObjectId> = (ocgs.iter())
        .filter_map(|item| match item {
            Object::Reference(id) if doc.resolve(item).as_dict().is_some() => Some(*id),
            _ => None,
        })
        .collect();
    let config = doc.dict_get(properties, b"D").as_dict();
    let base_off = config.is_some_and(|d| doc.dict_get(d, b"BaseState").as_name() == Some(b"OFF"));
    let listed = |key: &[u8]| -> BTreeSet<ObjectId> {
        let items = config.and_then(|d| doc.dict_get(d, key).as_array());
        (items.unwrap_or_default().iter())
            .filter_map(|item| match item {
                Object::Reference(id) => Some(*id),
                _ => None,
            })
            .collect()
    };
    let (on, off) = (listed(b"ON"), listed(b"OFF"));
    ids.into_iter()
        .rev()
        .map(|ocg| {
            let name = (doc.resolve(&Object::Reference(ocg)).as_dict())
                .and_then(|group| doc.dict_get(group, b"Name").as_string())
                .map(decode_text_string)
                .unwrap_or_default();
            // ON is applied before OFF, so OFF wins, as in reference viewers
            // and hayro.
            let visible = !off.contains(&ocg) && (on.contains(&ocg) || !base_off);
            Layer { ocg, name, visible }
        })
        .collect()
}

/// The groups the default configuration turns off with the panel's
/// `overrides` applied, read as hayro reads /D: /BaseState /OFF turns off
/// every group of /OCGs, then /ON and /OFF apply in that order.
pub(crate) fn hidden_groups(
    doc: &Document<'_>,
    overrides: &BTreeMap<ObjectId, bool>,
) -> BTreeSet<ObjectId> {
    let mut off = BTreeSet::new();
    let Some(properties) = properties(doc) else {
        return off;
    };
    let refs = |dict: &Dictionary, key: &[u8]| -> Vec<ObjectId> {
        (doc.dict_get(dict, key)
            .as_array()
            .unwrap_or_default()
            .iter())
        .filter_map(|item| match item {
            Object::Reference(id) => Some(*id),
            _ => None,
        })
        .collect()
    };
    if let Some(config) = doc.dict_get(properties, b"D").as_dict() {
        if doc.dict_get(config, b"BaseState").as_name() == Some(b"OFF") {
            off.extend(refs(properties, b"OCGs"));
        }
        for group in refs(config, b"ON") {
            off.remove(&group);
        }
        off.extend(refs(config, b"OFF"));
    }
    for (&group, &visible) in overrides {
        if visible {
            off.remove(&group);
        } else {
            off.insert(group);
        }
    }
    off
}

fn properties<'d>(doc: &'d Document<'_>) -> Option<&'d Dictionary> {
    let catalog = doc.resolve_id(doc.root()).as_dict()?;
    doc.dict_get(catalog, b"OCProperties").as_dict()
}

/// The object to redefine in the view's bytes so that the default
/// configuration shows and hides the groups `overrides` names: the
/// innermost indirect object holding /D (the configuration itself, the
/// /OCProperties dictionary or the catalog) with /ON and /OFF edited. None
/// without overrides or optional content.
pub fn edited_config(
    doc: &Document<'_>,
    overrides: &BTreeMap<ObjectId, bool>,
) -> Option<(ObjectId, Object)> {
    if overrides.is_empty() {
        return None;
    }
    let (catalog_holder, catalog) = doc.resolve_id_with_holder(doc.root());
    let (catalog_holder, catalog) = (catalog_holder?, catalog.as_dict()?);
    let (properties_holder, properties) =
        doc.resolve_with_holder(catalog.get(b"OCProperties".as_slice())?);
    let properties = properties.as_dict()?;
    let (config_holder, config) = match properties.get(b"D".as_slice()) {
        Some(value) => doc.resolve_with_holder(value),
        None => (None, &Object::Null),
    };
    let mut config = config.as_dict().cloned().unwrap_or_default();
    let array = |config: &Dictionary, key: &[u8]| -> Vec<Object> {
        doc.dict_get(config, key)
            .as_array()
            .map(<[Object]>::to_vec)
            .unwrap_or_default()
    };
    let (mut on, mut off) = (array(&config, b"ON"), array(&config, b"OFF"));
    for (&ocg, &visible) in overrides {
        let group = Object::Reference(ocg);
        if visible {
            off.retain(|item| *item != group);
            if !on.contains(&group) {
                on.push(group);
            }
        } else if !off.contains(&group) {
            off.push(group);
        }
    }
    config.insert(b"ON".to_vec(), Object::Array(on));
    config.insert(b"OFF".to_vec(), Object::Array(off));
    if let Some(holder) = config_holder {
        return Some((holder, Object::Dictionary(config)));
    }
    let mut properties = properties.clone();
    properties.insert(b"D".to_vec(), Object::Dictionary(config));
    if let Some(holder) = properties_holder {
        return Some((holder, Object::Dictionary(properties)));
    }
    let mut catalog = catalog.clone();
    catalog.insert(b"OCProperties".to_vec(), Object::Dictionary(properties));
    Some((catalog_holder, Object::Dictionary(catalog)))
}

/// What a reader's copy redefines so that it shows every optional content,
/// whatever the configurations, the panel, membership policies (/P) or
/// visibility expressions (/VE) say: the catalog without /OCProperties
/// (every group on), each membership dictionary object (/Type /OCMD) as
/// `<< /Type /OCMD >>` (no groups: shown by hayro's rule and the view's
/// text reader, as by reference viewers), and each Form whose /OC is a dictionary
/// written into it without that /OC. Redaction searches such a copy, so
/// text a membership shows only while a group is off (/AllOff, /AnyOff,
/// /VE /Not) is found as text in a hidden layer is. A membership or group
/// written directly into /Properties is no optional content to hayro or
/// the view (each needs a reference), so it needs nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShowAll {
    /// The catalog holds /OCProperties.
    pub catalog: bool,
    pub memberships: Vec<ObjectId>,
    pub forms: Vec<ObjectId>,
}

impl ShowAll {
    /// Whether the document has optional content that can hide anything.
    pub fn is_empty(&self) -> bool {
        !self.catalog && self.memberships.is_empty() && self.forms.is_empty()
    }

    /// The value each membership object takes.
    pub fn membership() -> Object {
        Object::dictionary([("Type", Object::name("OCMD"))])
    }

    /// Applies the redefinitions to `target` (a scratch copy).
    pub fn apply<E: crate::app::edit::Edit>(
        &self,
        target: &mut E,
    ) -> Result<(), crate::document::DocumentError> {
        if self.catalog {
            let root = target.document().root();
            target.update(root, |value| {
                if let Some(catalog) = value.as_dict_mut() {
                    catalog.remove(b"OCProperties".as_slice());
                }
            })?;
        }
        for &id in &self.memberships {
            target.put_value(id, Self::membership())?;
        }
        for &id in &self.forms {
            target.update(id, |value| {
                if let Some(dict) = value.as_dict_mut() {
                    dict.remove(b"OC".as_slice());
                }
            })?;
        }
        Ok(())
    }
}

/// [`ShowAll`] for `doc`: one pass over its objects.
pub fn show_all(doc: &Document<'_>) -> ShowAll {
    let catalog = properties_entry(doc);
    let mut memberships = Vec::new();
    let mut forms = Vec::new();
    for object in doc.objects() {
        let Some(dict) = object.value.as_dict() else {
            continue;
        };
        match object.stream {
            None => {
                if doc.dict_get(dict, b"Type").as_name() == Some(b"OCMD") {
                    memberships.push(object.id);
                }
            }
            Some(_) => {
                let direct = matches!(dict.get(b"OC".as_slice()), Some(Object::Dictionary(_)));
                if direct && doc.dict_get(dict, b"Subtype").as_name() == Some(b"Form") {
                    forms.push(object.id);
                }
            }
        }
    }
    ShowAll {
        catalog,
        memberships,
        forms,
    }
}

fn properties_entry(doc: &Document<'_>) -> bool {
    (doc.resolve_id(doc.root()).as_dict())
        .is_some_and(|catalog| catalog.contains_key(b"OCProperties".as_slice()))
}
