// SPDX-License-Identifier: MIT
//! redact-apply's sanitize options (redact.ts `checkSanitize`,
//! `sanitizeDocument`), applied to the scratch copy after the marks are
//! applied: exactly the app's nine toggles, each clearing the surface
//! `inventory` counts under the same name.
use std::collections::BTreeSet;

use serde_json::Value;

use crate::app::AppError;
use crate::app::annots;
use crate::app::edit::{Builder, Edit, Place};
use crate::app::forms::write::{EditSink, refresh};
use crate::app::info;
use crate::app::pagetree;
use crate::object::{Dictionary, Object, ObjectId};

use super::inventory::{
    Inventory, KEPT_INFO, MAX_WALK_NODES, STRUCT_KEYS, outside_tree, property_lists,
};

/// redact.ts `REDACT_SANITIZE_KEYS`, in its order.
pub const SANITIZE_KEYS: [&str; 9] = [
    "info",
    "xmp",
    "attachments",
    "outlines",
    "annots",
    "links",
    "fields",
    "layers",
    "struct",
];

/// Field-tree levels a widget's value is cleared up (redact.ts).
const FIELD_LEVELS: usize = 8;
/// Field subtree nodes a signature field's removal visits at most.
const MAX_FIELD_NODES: usize = 4096;
/// Action objects the links toggle follows along /Next at most, and the
/// depth of direct actions within one object.
const MAX_ACTIONS: usize = 4096;
const MAX_ACTION_DEPTH: usize = 32;
/// What leaves a Link's action and the actions it chains to (/Next): its
/// URI, file and the Launch action's per-platform file and parameters.
const ACTION_TARGETS: [&[u8]; 5] = [b"URI", b"F", b"Win", b"Mac", b"Unix"];

/// What to clear (redact.ts `RedactSanitize`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sanitize {
    pub info: bool,
    pub xmp: bool,
    pub attachments: bool,
    pub outlines: bool,
    pub annots: bool,
    pub links: bool,
    pub fields: bool,
    pub layers: bool,
    pub structure: bool,
}

impl Sanitize {
    /// redact.ts `checkSanitize`: an object with each key a boolean.
    pub fn read(value: Option<&Value>, op: &str) -> Result<Self, AppError> {
        let Some(object) = value.and_then(Value::as_object) else {
            return Err(AppError::reason(
                "invalid",
                format!("{op}: sanitize must be an object"),
            ));
        };
        let mut flags = [false; 9];
        for (slot, key) in flags.iter_mut().zip(SANITIZE_KEYS) {
            let Some(flag) = object.get(key).and_then(Value::as_bool) else {
                return Err(AppError::reason(
                    "invalid",
                    format!("{op}: sanitize.{key} must be true or false"),
                ));
            };
            *slot = flag;
        }
        let [
            info,
            xmp,
            attachments,
            outlines,
            annots,
            links,
            fields,
            layers,
            structure,
        ] = flags;
        Ok(Self {
            info,
            xmp,
            attachments,
            outlines,
            annots,
            links,
            fields,
            layers,
            structure,
        })
    }

    /// The inventory with the surfaces this clears zeroed (`kept`), or with
    /// the others zeroed (`sanitized`).
    pub fn split(&self, inventory: &Inventory, cleared: bool) -> Inventory {
        let keep = |chosen: bool| chosen == cleared;
        let n = |chosen: bool, value: usize| if keep(chosen) { value } else { 0 };
        Inventory {
            info: n(self.info, inventory.info),
            xmp: keep(self.xmp) && inventory.xmp,
            attachments: n(self.attachments, inventory.attachments),
            outlines: n(self.outlines, inventory.outlines),
            annots: n(self.annots, inventory.annots),
            links: n(self.links, inventory.links),
            fields: n(self.fields, inventory.fields),
            layers: n(self.layers, inventory.layers),
            structure: n(self.structure, inventory.structure),
        }
    }
}

fn plain(op: &str, error: impl std::fmt::Display) -> AppError {
    AppError::plain(format!("{op}: {error}"))
}

/// redact.ts `sanitizeDocument`.
pub fn sanitize(target: Builder, choose: &Sanitize, op: &str) -> Result<Builder, AppError> {
    let mut target = target;
    if choose.info {
        target = clear_info(target, op)?;
    }
    let root = target.document().root();
    let catalog_keys: Vec<&[u8]> = [
        (choose.xmp, b"Metadata".as_slice()),
        (choose.outlines, b"Outlines"),
    ]
    .into_iter()
    .filter_map(|(on, key)| on.then_some(key))
    .collect();
    if !catalog_keys.is_empty() {
        target
            .update(root, |value| {
                if let Some(dict) = value.as_dict_mut() {
                    for key in &catalog_keys {
                        dict.remove(*key);
                    }
                }
            })
            .map_err(|e| plain(op, e))?;
    }
    if choose.attachments {
        let names = Place::object(root).child(target.document(), b"Names");
        if names.read(target.document()).as_dict().is_some() {
            names
                .update(&mut target, |value| {
                    if let Some(dict) = value.as_dict_mut() {
                        dict.remove(b"EmbeddedFiles".as_slice());
                    }
                })
                .map_err(|e| plain(op, e))?;
        }
    }
    page_annotations(&mut target, choose, op)?;
    if choose.fields {
        clear_fields(&mut target, op)?;
    }
    if choose.layers {
        rename_layers(&mut target, op)?;
    }
    if choose.structure {
        clear_structure(&mut target, op)?;
    }
    Ok(target)
}

/// Every /Info entry but Producer, CreationDate and ModDate goes.
fn clear_info(target: Builder, op: &str) -> Result<Builder, AppError> {
    let mut target = target;
    let entry = target.document().trailer_entry(b"Info").cloned();
    let keep = |dict: &mut Dictionary| dict.retain(|key, _| KEPT_INFO.contains(&key.as_slice()));
    match entry {
        Some(Object::Reference(id)) => {
            if target.document().resolve_id(id).as_dict().is_some() {
                target
                    .update(id, |value| {
                        if let Some(dict) = value.as_dict_mut() {
                            keep(dict);
                        }
                    })
                    .map_err(|e| plain(op, e))?;
            }
            Ok(target)
        }
        Some(Object::Dictionary(mut dict)) => {
            keep(&mut dict);
            let mut doc = target.into_document();
            doc.set_trailer_entry(b"Info", Some(Object::Dictionary(dict)))
                .map_err(|e| plain(op, e))?;
            Ok(Builder::from_document(doc))
        }
        _ => Ok(target),
    }
}

/// redact.ts `removePageAnnotations`: FileAttachment annotations
/// (`attachments`) and every annotation but Widgets, Links and marks
/// (`annots`) leave every page; Links lose their action's targets
/// (`links`: `ACTION_TARGETS`, along /Next chains; redact.ts cleared /URI
/// and /F of the first action only).
fn page_annotations(target: &mut Builder, choose: &Sanitize, op: &str) -> Result<(), AppError> {
    if !(choose.attachments || choose.annots || choose.links) {
        return Ok(());
    }
    let mut removed: Vec<(usize, ObjectId)> = Vec::new();
    let mut direct: Vec<(usize, BTreeSet<usize>)> = Vec::new();
    let mut actions: Vec<Place> = Vec::new();
    {
        let doc = target.document();
        for (index, page) in info::pages(doc).iter().enumerate() {
            let items = doc.dict_get(page.dict, b"Annots").as_array().unwrap_or(&[]);
            let mut gone = BTreeSet::new();
            let mut seen = BTreeSet::new();
            for (position, item) in items.iter().enumerate() {
                let (holder, value) = doc.resolve_with_holder(item);
                let id = holder.filter(|_| item.as_reference().is_some());
                let Some(dict) = value.as_dict() else {
                    continue;
                };
                let kind = doc.dict_get(dict, b"Subtype").as_name().unwrap_or(b"");
                let remove = (kind == b"FileAttachment" && choose.attachments)
                    || (choose.annots && !matches!(kind, b"Widget" | b"Link" | b"Redact"));
                if let (b"Link", true, Some(id)) = (kind, choose.links, id) {
                    let place = Place::object(id).child(doc, b"A");
                    if place.read(doc).as_dict().is_some() {
                        actions.push(place);
                    }
                }
                if !remove {
                    continue;
                }
                match id {
                    Some(id) => {
                        if seen.insert(id) {
                            removed.push((index, id));
                        }
                    }
                    None => {
                        gone.insert(position);
                    }
                }
            }
            if !gone.is_empty() {
                direct.push((index, gone));
            }
        }
    }
    let mut chained = Vec::new();
    for place in actions {
        place
            .update(target, |value| scrub_action(value, &mut chained, 0))
            .map_err(|e| plain(op, e))?;
    }
    if choose.links {
        direct_links(target, &mut chained, op)?;
        scrub_chained(target, chained, op)?;
    }
    for (page, positions) in direct {
        annots::edit_annots(target, page, |items| {
            let mut at = 0;
            items.retain(|_| {
                let keep = !positions.contains(&at);
                at += 1;
                keep
            });
        })
        .map_err(|e| plain(op, e))?;
    }
    if !removed.is_empty() {
        annots::remove_many(target, &removed).map_err(|e| plain(op, e))?;
    }
    Ok(())
}

/// Removes an action's targets (`ACTION_TARGETS`) and those of the direct
/// actions its /Next chains to; actions /Next names by reference are added
/// to `chained`.
fn scrub_action(value: &mut Object, chained: &mut Vec<ObjectId>, depth: usize) {
    let Some(dict) = value.as_dict_mut() else {
        return;
    };
    for key in ACTION_TARGETS {
        dict.remove(key);
    }
    if depth >= MAX_ACTION_DEPTH {
        return;
    }
    match dict.get_mut(b"Next".as_slice()) {
        Some(Object::Reference(id)) => chained.push(*id),
        Some(Object::Array(items)) => {
            for item in items.iter_mut() {
                match item {
                    Object::Reference(id) => chained.push(*id),
                    other => scrub_action(other, chained, depth + 1),
                }
            }
        }
        Some(next) => scrub_action(next, chained, depth + 1),
        None => {}
    }
}

/// The action objects /Next chains name, each once, at most `MAX_ACTIONS`.
fn scrub_chained(target: &mut Builder, chained: Vec<ObjectId>, op: &str) -> Result<(), AppError> {
    let mut queue = chained;
    let mut seen: BTreeSet<ObjectId> = BTreeSet::new();
    while let Some(id) = queue.pop() {
        if seen.len() >= MAX_ACTIONS || !seen.insert(id) {
            continue;
        }
        let action = target.document().object(id);
        if !action.is_ok_and(|o| o.stream.is_none() && o.value.as_dict().is_some()) {
            continue;
        }
        let mut more = Vec::new();
        target
            .update(id, |value| scrub_action(value, &mut more, 0))
            .map_err(|e| plain(op, e))?;
        queue.extend(more);
    }
    Ok(())
}

/// Link dictionaries written directly into /Annots lose their targets too.
fn direct_links(
    target: &mut Builder,
    chained: &mut Vec<ObjectId>,
    op: &str,
) -> Result<(), AppError> {
    let pages: Vec<usize> = {
        let doc = target.document();
        (info::pages(doc).iter().enumerate())
            .filter(|(_, page)| {
                (doc.dict_get(page.dict, b"Annots")
                    .as_array()
                    .unwrap_or(&[])
                    .iter())
                .any(|i| {
                    i.as_dict()
                        .is_some_and(|d| doc.dict_get(d, b"Subtype").as_name() == Some(b"Link"))
                })
            })
            .map(|(i, _)| i)
            .collect()
    };
    for page in pages {
        // Indirect actions of direct links.
        let held: Vec<ObjectId> = {
            let doc = target.document();
            let dict = info::pages(doc).get(page).map(|p| p.dict);
            (dict.and_then(|d| doc.dict_get(d, b"Annots").as_array()))
                .unwrap_or(&[])
                .iter()
                .filter_map(Object::as_dict)
                .filter(|d| doc.dict_get(d, b"Subtype").as_name() == Some(b"Link"))
                .filter_map(|d| d.get(b"A".as_slice()).and_then(Object::as_reference))
                .collect()
        };
        chained.extend(held);
        annots::edit_annots(target, page, |items| {
            for item in items.iter_mut() {
                let Some(dict) = item.as_dict_mut() else {
                    continue;
                };
                if dict.get(b"Subtype".as_slice()).and_then(Object::as_name) != Some(b"Link") {
                    continue;
                }
                if let Some(action @ Object::Dictionary(_)) = dict.get_mut(b"A".as_slice()) {
                    scrub_action(action, chained, 0);
                }
            }
        })
        .map_err(|e| plain(op, e))?;
    }
    Ok(())
}

/// redact.ts `sanitizeFields`: no /XFA or /SigFlags; signature fields leave
/// /Fields and the pages; every other widget and up to seven of its
/// parents lose /V, /DV, /RV (the rich-text value) and /I (a choice's
/// selected indices: with /V gone they still name the choice), which
/// redact.ts kept, and /AP (a button's /AS becomes Off), then the widget
/// gets a new, empty appearance.
fn clear_fields(target: &mut Builder, op: &str) -> Result<(), AppError> {
    let root = target.document().root();
    let acro = Place::object(root).child(target.document(), b"AcroForm");
    if acro.read(target.document()).as_dict().is_none() {
        return Ok(());
    }
    acro.update(target, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.remove(b"XFA".as_slice());
            dict.remove(b"SigFlags".as_slice());
        }
    })
    .map_err(|e| plain(op, e))?;
    // Signature fields and their kids.
    let signatures: BTreeSet<ObjectId> = {
        let doc = target.document();
        let mut found = BTreeSet::new();
        let fields = acro.read(doc).as_dict().map(|a| doc.dict_get(a, b"Fields"));
        for top in fields.and_then(Object::as_array).unwrap_or(&[]) {
            let Some(id) = top.as_reference() else {
                continue;
            };
            let is_sig = (doc.resolve_id(id).as_dict())
                .is_some_and(|d| doc.dict_get(d, b"FT").as_name() == Some(b"Sig"));
            if !is_sig {
                continue;
            }
            let mut queue = vec![id];
            while let Some(node) = queue.pop() {
                if found.len() >= MAX_FIELD_NODES || !found.insert(node) {
                    continue;
                }
                let kids = (doc.resolve_id(node).as_dict())
                    .and_then(|d| doc.dict_get(d, b"Kids").as_array())
                    .unwrap_or(&[]);
                queue.extend(kids.iter().filter_map(Object::as_reference));
            }
        }
        found
    };
    if !signatures.is_empty() {
        let fields = acro.child(target.document(), b"Fields");
        fields
            .update(target, |value| {
                if let Some(items) = value.as_array_mut() {
                    items.retain(|i| i.as_reference().is_none_or(|r| !signatures.contains(&r)));
                }
            })
            .map_err(|e| plain(op, e))?;
        pagetree::cut_references(target, &BTreeSet::new(), &signatures, &BTreeSet::new())
            .map_err(|e| plain(op, e))?;
    }
    // Every other widget, and its parents.
    let widgets: Vec<ObjectId> = {
        let doc = target.document();
        let mut seen = BTreeSet::new();
        (info::pages(doc).iter())
            .flat_map(|page| crate::app::forms::page_widgets(doc, page.dict))
            .filter_map(|w| w.id)
            .filter(|id| !signatures.contains(id) && seen.insert(*id))
            .collect()
    };
    for widget in &widgets {
        let mut node = Some(*widget);
        let mut seen = BTreeSet::new();
        for _ in 0..FIELD_LEVELS {
            let Some(id) = node.filter(|id| seen.insert(*id)) else {
                break;
            };
            let doc = target.document();
            let Some(dict) = doc.resolve_id(id).as_dict() else {
                break;
            };
            let button = doc.dict_get(dict, b"FT").as_name() == Some(b"Btn");
            node = dict
                .get(b"Parent".as_slice())
                .and_then(Object::as_reference);
            target
                .update(id, |value| {
                    if let Some(dict) = value.as_dict_mut() {
                        for key in [b"V".as_slice(), b"DV", b"RV", b"I", b"AP"] {
                            dict.remove(key);
                        }
                        if button {
                            dict.insert(b"AS".to_vec(), Object::name("Off"));
                        }
                    }
                })
                .map_err(|e| plain(op, e))?;
        }
    }
    let mut sink = EditSink::new(target);
    for widget in widgets {
        refresh(&mut sink, widget, None)?;
    }
    Ok(())
}

/// D5: an unsigned copy of a signed document. Every signed signature field
/// (own /FT /Sig with a /V) and the fields and widgets under it leave
/// /Fields, the pages and every other reference (their signature
/// dictionaries are then not written); the catalog loses /Perms (DocMDP,
/// usage rights) and the form /SigFlags. Unsigned signature fields stay.
pub fn remove_signatures(target: &mut Builder, op: &str) -> Result<(), AppError> {
    let root = target.document().root();
    Place::object(root)
        .update(target, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.remove(b"Perms".as_slice());
            }
        })
        .map_err(|e| plain(op, e))?;
    let acro = Place::object(root).child(target.document(), b"AcroForm");
    if acro.read(target.document()).as_dict().is_none() {
        return Ok(());
    }
    acro.update(target, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.remove(b"SigFlags".as_slice());
        }
    })
    .map_err(|e| plain(op, e))?;
    let signed: BTreeSet<ObjectId> = {
        let doc = target.document();
        let fields = acro.read(doc).as_dict().map(|a| doc.dict_get(a, b"Fields"));
        let mut seen = BTreeSet::new();
        let mut found = BTreeSet::new();
        // (node, inside a signed field)
        let mut queue: Vec<(ObjectId, bool)> = (fields.and_then(Object::as_array).unwrap_or(&[]))
            .iter()
            .filter_map(Object::as_reference)
            .map(|id| (id, false))
            .collect();
        while let Some((node, inside)) = queue.pop() {
            if seen.len() >= MAX_FIELD_NODES * 16 || !seen.insert(node) {
                continue;
            }
            let Some(dict) = doc.resolve_id(node).as_dict() else {
                continue;
            };
            let own_sig = dict
                .get(b"FT".as_slice())
                .and_then(|v| doc.resolve(v).as_name())
                == Some(b"Sig".as_slice())
                && dict
                    .get(b"V".as_slice())
                    .is_some_and(|v| !doc.resolve(v).is_null());
            let inside = inside || own_sig;
            if inside {
                found.insert(node);
            }
            let kids = doc.dict_get(dict, b"Kids").as_array().unwrap_or(&[]);
            queue.extend(
                kids.iter()
                    .filter_map(Object::as_reference)
                    .map(|k| (k, inside)),
            );
        }
        found
    };
    if signed.is_empty() {
        return Ok(());
    }
    let fields = acro.child(target.document(), b"Fields");
    fields
        .update(target, |value| {
            if let Some(items) = value.as_array_mut() {
                items.retain(|i| i.as_reference().is_none_or(|r| !signed.contains(&r)));
            }
        })
        .map_err(|e| plain(op, e))?;
    pagetree::cut_references(target, &BTreeSet::new(), &signed, &BTreeSet::new())
        .map_err(|e| plain(op, e))?;
    Ok(())
}

/// redact.ts: each optional-content group's /Name becomes `Layer <n>`; the
/// configurations (/D, /Configs) lose their /Name too.
fn rename_layers(target: &mut Builder, op: &str) -> Result<(), AppError> {
    let root = target.document().root();
    let groups = Place::object(root)
        .child(target.document(), b"OCProperties")
        .child(target.document(), b"OCGs");
    let entries: Vec<Option<ObjectId>> = (groups.read(target.document()).as_array())
        .unwrap_or(&[])
        .iter()
        .map(Object::as_reference)
        .collect();
    for (index, entry) in entries.iter().enumerate() {
        let name = Object::text(&format!("Layer {}", index + 1));
        match entry {
            Some(id) => {
                if target.document().resolve_id(*id).as_dict().is_some() {
                    target
                        .update(*id, |value| {
                            if let Some(dict) = value.as_dict_mut() {
                                dict.insert(b"Name".to_vec(), name);
                            }
                        })
                        .map_err(|e| plain(op, e))?;
                }
            }
            None => {
                groups
                    .update(target, |value| {
                        let group = value.as_array_mut().and_then(|items| items.get_mut(index));
                        if let Some(dict) = group.and_then(Object::as_dict_mut) {
                            dict.insert(b"Name".to_vec(), name);
                        }
                    })
                    .map_err(|e| plain(op, e))?;
            }
        }
    }
    // The configurations' names go too (redact.ts kept them).
    let properties = Place::object(root).child(target.document(), b"OCProperties");
    let config = properties.child(target.document(), b"D");
    let named = |place: &Place, target: &Builder| {
        (place.read(target.document()).as_dict())
            .is_some_and(|d| d.get(b"Name".as_slice()).is_some())
    };
    if named(&config, target) {
        config
            .update(target, |value| {
                if let Some(dict) = value.as_dict_mut() {
                    dict.remove(b"Name".as_slice());
                }
            })
            .map_err(|e| plain(op, e))?;
    }
    let configs = properties.child(target.document(), b"Configs");
    let listed: Vec<(usize, Option<ObjectId>)> = (configs.read(target.document()).as_array())
        .unwrap_or(&[])
        .iter()
        .map(Object::as_reference)
        .enumerate()
        .take(MAX_ACTIONS)
        .collect();
    for (index, entry) in listed {
        match entry {
            Some(id) => {
                let place = Place::object(id);
                if named(&place, target) {
                    place
                        .update(target, |value| {
                            if let Some(dict) = value.as_dict_mut() {
                                dict.remove(b"Name".as_slice());
                            }
                        })
                        .map_err(|e| plain(op, e))?;
                }
            }
            None => {
                configs
                    .update(target, |value| {
                        let config = value.as_array_mut().and_then(|items| items.get_mut(index));
                        if let Some(dict) = config.and_then(Object::as_dict_mut) {
                            dict.remove(b"Name".as_slice());
                        }
                    })
                    .map_err(|e| plain(op, e))?;
            }
        }
    }
    Ok(())
}

/// redact.ts: /ActualText, /Alt and /E leave every dictionary of the
/// structure tree (the tree `inventory` counts in), and the property lists
/// named in resources that `inventory` counts.
fn clear_structure(target: &mut Builder, op: &str) -> Result<(), AppError> {
    for (place, _) in property_lists(target.document(), op)? {
        place
            .update(target, |value| {
                if let Some(dict) = value.as_dict_mut() {
                    for key in STRUCT_KEYS {
                        dict.remove(key);
                    }
                }
            })
            .map_err(|e| plain(op, e))?;
    }
    const OUT: [&[u8]; 4] = [b"Pg", b"Obj", b"Stm", b"StmOwn"];
    let root = target.document().root();
    let start = Place::object(root).child(target.document(), b"StructTreeRoot");
    let mut queue = vec![start];
    let mut seen: BTreeSet<ObjectId> = BTreeSet::new();
    let mut visits = 0usize;
    while let Some(place) = queue.pop() {
        let doc = target.document();
        if place.keys.is_empty() {
            if !seen.insert(place.id) {
                continue;
            }
            let object = doc.object(place.id);
            if object.is_ok_and(|o| o.stream.is_some()) {
                continue;
            }
        }
        // Walk the value, direct parts only: what to clear and what to visit.
        let mut refs = Vec::new();
        let mut dirty = false;
        let mut stack = vec![place.read(doc)];
        while let Some(value) = stack.pop() {
            visits += 1;
            if visits > MAX_WALK_NODES {
                return Err(AppError::reason(
                    "too-large",
                    format!("{op}: the structure tree has more than {MAX_WALK_NODES} objects"),
                ));
            }
            match value {
                Object::Reference(id) => refs.push(*id),
                Object::Array(items) => stack.extend(items),
                Object::Dictionary(dict) => {
                    if outside_tree(doc, None, dict) {
                        continue;
                    }
                    dirty |= STRUCT_KEYS.iter().any(|k| dict.get(*k).is_some());
                    for (key, value) in dict.iter() {
                        if !OUT.contains(&key.as_slice()) {
                            stack.push(value);
                        }
                    }
                }
                _ => {}
            }
        }
        if place.keys.is_empty()
            && (doc.resolve_id(place.id).as_dict())
                .is_some_and(|d| outside_tree(doc, Some(place.id), d))
        {
            continue;
        }
        queue.extend(refs.into_iter().map(Place::object));
        if dirty {
            place.update(target, clear_keys).map_err(|e| plain(op, e))?;
        }
    }
    Ok(())
}

/// Removes the structure keys from every dictionary of `value` (direct
/// parts, not crossing /Pg, /Obj, /Stm, /StmOwn or outside the tree).
fn clear_keys(value: &mut Object) {
    const OUT: [&[u8]; 4] = [b"Pg", b"Obj", b"Stm", b"StmOwn"];
    let mut stack = vec![value];
    let mut visits = 0usize;
    while let Some(value) = stack.pop() {
        visits += 1;
        if visits > MAX_WALK_NODES {
            return;
        }
        match value {
            Object::Array(items) => stack.extend(items.iter_mut()),
            Object::Dictionary(dict) => {
                let typed = dict.get(b"Type".as_slice()).and_then(Object::as_name);
                if matches!(
                    typed,
                    Some(b"Page" | b"Pages" | b"Catalog" | b"Annot" | b"XObject" | b"Font")
                ) {
                    continue;
                }
                for key in STRUCT_KEYS {
                    dict.remove(key);
                }
                for (key, value) in dict.iter_mut() {
                    if !OUT.contains(&key.as_slice()) {
                        stack.push(value);
                    }
                }
            }
            _ => {}
        }
    }
}
