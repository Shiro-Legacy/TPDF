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
    if choose.outlines {
        rename_destinations(&mut target, op)?;
    }
    page_annotations(&mut target, choose, op)?;
    if choose.links {
        remove_scripts(&mut target, op)?;
    }
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

/// `info` (sixth review): the document's language (/Lang) and each page
/// label's prefix (/PageLabels /P) go when `quotes` says they hold text
/// read inside a box; other labels and languages stay.
pub fn clear_quoted(
    target: &mut Builder,
    quotes: impl Fn(&[u8]) -> bool,
    op: &str,
) -> Result<(), AppError> {
    let root = target.document().root();
    let lang = (target.document().resolve_id(root).as_dict())
        .and_then(|c| c.get(b"Lang".as_slice()))
        .and_then(|l| target.document().resolve(l).as_string())
        .is_some_and(&quotes);
    if lang {
        target
            .update(root, |value| {
                if let Some(dict) = value.as_dict_mut() {
                    dict.remove(b"Lang".as_slice());
                }
            })
            .map_err(|e| plain(op, e))?;
    }
    let labels = Place::object(root).child(target.document(), b"PageLabels");
    let mut nodes: Vec<Place> = Vec::new();
    let mut indirect: Vec<ObjectId> = Vec::new();
    {
        let doc = target.document();
        let quoted = |label: &Object| {
            (doc.resolve(label).as_dict())
                .and_then(|d| d.get(b"P".as_slice()))
                .and_then(|p| doc.resolve(p).as_string())
                .is_some_and(&quotes)
        };
        let mut stack = vec![labels];
        let mut seen: BTreeSet<ObjectId> = BTreeSet::new();
        while let Some(place) = stack.pop() {
            if seen.len() >= MAX_WALK_NODES {
                break;
            }
            let Some(node) = place.read(doc).as_dict() else {
                continue;
            };
            let nums = doc.dict_get(node, b"Nums").as_array().unwrap_or(&[]);
            let mut direct = false;
            for label in nums.iter().skip(1).step_by(2) {
                if quoted(label) {
                    match label.as_reference() {
                        Some(id) => indirect.push(id),
                        None => direct = true,
                    }
                }
            }
            if direct {
                nodes.push(place.clone());
            }
            for kid in doc.dict_get(node, b"Kids").as_array().unwrap_or(&[]) {
                if let Some(id) = kid.as_reference() {
                    if seen.insert(id) {
                        stack.push(Place::object(id));
                    }
                }
            }
        }
    }
    let drop_prefix = |value: &mut Object| {
        if let Some(dict) = value.as_dict_mut() {
            if dict
                .get(b"P".as_slice())
                .and_then(Object::as_string)
                .is_some_and(&quotes)
            {
                dict.remove(b"P".as_slice());
            }
        }
    };
    for place in nodes {
        place
            .update(target, |value| {
                let nums = value
                    .as_dict_mut()
                    .and_then(|n| n.get_mut(b"Nums".as_slice()));
                if let Some(Object::Array(items)) = nums {
                    items.iter_mut().skip(1).step_by(2).for_each(drop_prefix);
                }
            })
            .map_err(|e| plain(op, e))?;
    }
    for id in indirect {
        target.update(id, drop_prefix).map_err(|e| plain(op, e))?;
    }
    Ok(())
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
/// (own /FT /Sig with a /V), indirect or written directly into /Fields, a
/// /Kids array or a page's /Annots, leaves with the fields and widgets
/// under it (indirect ones from every reference, so their signature
/// dictionaries are not written); the catalog loses /Perms (DocMDP, usage
/// rights) and the form /SigFlags. Unsigned signature fields stay. The
/// copy is checked afterwards (`info::detect`).
pub fn remove_signatures(target: &mut Builder, op: &str) -> Result<(), AppError> {
    let root = target.document().root();
    Place::object(root)
        .update(target, |value| {
            if let Some(dict) = value.as_dict_mut() {
                // DocMDP and usage rights; the document security store
                // (validation data for the signatures).
                dict.remove(b"Perms".as_slice());
                dict.remove(b"DSS".as_slice());
            }
        })
        .map_err(|e| plain(op, e))?;
    let acro = Place::object(root).child(target.document(), b"AcroForm");
    if acro.read(target.document()).as_dict().is_some() {
        acro.update(target, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.remove(b"SigFlags".as_slice());
            }
        })
        .map_err(|e| plain(op, e))?;
    }
    // Indirect signed nodes and what is under them; indirect nodes whose
    // /Kids hold direct signed fields.
    let (signed, holders, fields) = {
        let doc = target.document();
        let fields = (acro.read(doc).as_dict())
            .map(|a| doc.dict_get(a, b"Fields").clone())
            .unwrap_or(Object::Null);
        let mut seen = BTreeSet::new();
        let mut signed = BTreeSet::new();
        let mut holders = BTreeSet::new();
        // (value, inside a signed field)
        let mut queue: Vec<(Object, bool)> = vec![(fields.clone(), false)];
        while let Some((value, inside)) = queue.pop() {
            if seen.len() >= MAX_FIELD_NODES * 16 {
                break;
            }
            match value {
                Object::Array(items) => queue.extend(items.into_iter().map(|i| (i, inside))),
                Object::Reference(id) => {
                    if !seen.insert(id) {
                        continue;
                    }
                    let Some(dict) = doc.resolve_id(id).as_dict() else {
                        continue;
                    };
                    let inside = inside || is_signed(doc, dict);
                    if inside {
                        signed.insert(id);
                    }
                    let kids = doc.dict_get(dict, b"Kids").as_array().unwrap_or(&[]);
                    if kids
                        .iter()
                        .any(|k| k.as_dict().is_some_and(|d| is_signed(doc, d)))
                    {
                        holders.insert(id);
                    }
                    queue.extend(kids.iter().map(|k| (k.clone(), inside)));
                }
                Object::Dictionary(dict) => {
                    let inside = inside || is_signed(doc, &dict);
                    let kids = doc
                        .dict_get(&dict, b"Kids")
                        .as_array()
                        .unwrap_or(&[])
                        .to_vec();
                    queue.extend(kids.into_iter().map(|k| (k, inside)));
                }
                _ => {}
            }
        }
        // References inside a direct signed field go too.
        let mut fields = fields;
        prune(doc, &mut fields, &signed, 0);
        (signed, holders, fields)
    };
    if acro.read(target.document()).as_dict().is_some() {
        acro.update(target, |value| {
            if let Some(dict) = value.as_dict_mut() {
                if !fields.is_null() {
                    dict.insert(b"Fields".to_vec(), fields);
                }
            }
        })
        .map_err(|e| plain(op, e))?;
    }
    for id in holders {
        let kids = {
            let doc = target.document();
            let mut kids = (doc.resolve_id(id).as_dict())
                .map(|d| d.get(b"Kids".as_slice()).cloned().unwrap_or(Object::Null))
                .unwrap_or(Object::Null);
            prune(doc, &mut kids, &signed, 0);
            kids
        };
        target
            .update(id, |value| {
                if let Some(dict) = value.as_dict_mut() {
                    dict.insert(b"Kids".to_vec(), kids);
                }
            })
            .map_err(|e| plain(op, e))?;
    }
    // Signed widgets a page's /Annots lists that /Fields does not reach:
    // indirect ones go with the others, direct ones from the array.
    let mut signed = signed;
    {
        let doc = target.document();
        for page in info::pages(doc) {
            for item in doc.dict_get(page.dict, b"Annots").as_array().unwrap_or(&[]) {
                if let Some(id) = item.as_reference() {
                    if doc
                        .resolve_id(id)
                        .as_dict()
                        .is_some_and(|d| signed_widget(doc, d))
                    {
                        signed.insert(id);
                    }
                }
            }
        }
    }
    let pages = info::pages(target.document()).len();
    for page in 0..pages {
        let direct = {
            let doc = target.document();
            let page_dict = info::pages(doc).get(page).map(|p| p.dict);
            let items = page_dict.and_then(|d| doc.dict_get(d, b"Annots").as_array());
            items
                .unwrap_or(&[])
                .iter()
                .any(|a| a.as_dict().is_some_and(|d| signed_widget(doc, d)))
        };
        if direct {
            let doc_signed = |item: &Object, doc: &crate::document::Document<'_>| {
                item.as_dict().is_some_and(|d| signed_widget(doc, d))
            };
            let keep: Vec<bool> = {
                let doc = target.document();
                let page_dict = info::pages(doc).get(page).map(|p| p.dict);
                let items = page_dict.and_then(|d| doc.dict_get(d, b"Annots").as_array());
                items
                    .unwrap_or(&[])
                    .iter()
                    .map(|a| !doc_signed(a, doc))
                    .collect()
            };
            annots::edit_annots(target, page, |items| {
                let mut index = 0;
                items.retain(|_| {
                    let k = keep.get(index).copied().unwrap_or(true);
                    index += 1;
                    k
                });
            })
            .map_err(|e| plain(op, e))?;
        }
    }
    if !signed.is_empty() {
        pagetree::cut_references(target, &BTreeSet::new(), &signed, &BTreeSet::new())
            .map_err(|e| plain(op, e))?;
    }
    Ok(())
}

/// A widget that is signed: its field type (its own or inherited, up to
/// eight parents) is /Sig and it or a parent has a /V.
fn signed_widget(doc: &crate::document::Document<'_>, dict: &Dictionary) -> bool {
    let mut node = Some(dict);
    let mut sig = None;
    let mut value = false;
    for _ in 0..9 {
        let Some(current) = node else {
            break;
        };
        if sig.is_none() {
            sig = current
                .get(b"FT".as_slice())
                .and_then(|v| doc.resolve(v).as_name())
                .map(|n| n == b"Sig");
        }
        value |= current
            .get(b"V".as_slice())
            .is_some_and(|v| !doc.resolve(v).is_null());
        node = current
            .get(b"Parent".as_slice())
            .and_then(|p| doc.resolve(p).as_dict());
    }
    sig == Some(true) && value
}

/// A field dictionary that is itself signed: its own /FT is /Sig and it
/// has a /V.
fn is_signed(doc: &crate::document::Document<'_>, dict: &Dictionary) -> bool {
    dict.get(b"FT".as_slice())
        .and_then(|v| doc.resolve(v).as_name())
        == Some(b"Sig".as_slice())
        && dict
            .get(b"V".as_slice())
            .is_some_and(|v| !doc.resolve(v).is_null())
}

/// `value` (a /Fields or /Kids array, or a field) without its direct signed
/// fields or references to `signed`, at any depth of direct /Kids.
fn prune(
    doc: &crate::document::Document<'_>,
    value: &mut Object,
    signed: &BTreeSet<ObjectId>,
    depth: usize,
) {
    if depth > 64 {
        return;
    }
    match value {
        Object::Array(items) => {
            items.retain(|item| match item {
                Object::Reference(id) => !signed.contains(id),
                Object::Dictionary(d) => !is_signed(doc, d),
                _ => true,
            });
            for item in items.iter_mut() {
                prune(doc, item, signed, depth + 1);
            }
        }
        Object::Dictionary(dict) => {
            if let Some(kids) = dict.get_mut(b"Kids".as_slice()) {
                prune(doc, kids, signed, depth + 1);
            }
        }
        _ => {}
    }
}

/// `links` (fifth and sixth reviews): every script goes. In every
/// object, at any depth of its direct values: additional actions (/AA:
/// the catalog's, pages', annotations', fields') go; an /OpenAction or /A
/// that is a JavaScript action goes; every /JS (a JavaScript action's,
/// one a /Next chain holds, a Rendition's) is emptied, so a chain still
/// reads; the name tree of document scripts (/Names /JavaScript) goes.
fn remove_scripts(target: &mut Builder, op: &str) -> Result<(), AppError> {
    let (edits, actions) = {
        let doc = target.document();
        // JavaScript action objects, for an /A or /OpenAction naming one.
        let actions: BTreeSet<ObjectId> = (doc.objects())
            .filter(|o| o.value.as_dict().is_some_and(is_script_action))
            .map(|o| o.id)
            .collect();
        let edits: Vec<ObjectId> = (doc.objects())
            .filter(|o| holds_scripts(&o.value, &actions))
            .map(|o| o.id)
            .collect();
        (edits, actions)
    };
    for id in edits {
        target
            .update(id, |value| strip_scripts(value, &actions))
            .map_err(|e| plain(op, e))?;
    }
    Ok(())
}

/// A JavaScript action (or any action carrying a script: a Rendition's
/// /JS).
fn is_script_action(dict: &Dictionary) -> bool {
    dict.get(b"S".as_slice()).and_then(Object::as_name) == Some(b"JavaScript")
}

/// What `strip_scripts` would change in `value`.
fn holds_scripts(value: &Object, actions: &BTreeSet<ObjectId>) -> bool {
    let mut stack = vec![value];
    let mut visits = 0usize;
    while let Some(value) = stack.pop() {
        visits += 1;
        if visits > MAX_WALK_NODES {
            // Too large to look through: rewritten (the strip is bounded too).
            return true;
        }
        match value {
            Object::Array(items) => stack.extend(items),
            Object::Dictionary(dict) => {
                if dict.get(b"AA".as_slice()).is_some()
                    || dict
                        .get(b"JS".as_slice())
                        .is_some_and(|js| !empty_script(js))
                    || dict.get(b"JavaScript".as_slice()).is_some()
                    || [b"OpenAction".as_slice(), b"A"]
                        .iter()
                        .any(|k| dict.get(*k).is_some_and(|v| runs_script(v, actions)))
                {
                    return true;
                }
                stack.extend(dict.values());
            }
            _ => {}
        }
    }
    false
}

fn empty_script(js: &Object) -> bool {
    js.as_string().is_some_and(<[u8]>::is_empty)
}

/// An /A or /OpenAction value that is a JavaScript action.
fn runs_script(value: &Object, actions: &BTreeSet<ObjectId>) -> bool {
    match value {
        Object::Reference(id) => actions.contains(id),
        Object::Dictionary(dict) => is_script_action(dict),
        _ => false,
    }
}

/// Removes the scripts of `value` (see `remove_scripts`), direct values at
/// any depth.
fn strip_scripts(value: &mut Object, actions: &BTreeSet<ObjectId>) {
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
                dict.remove(b"AA".as_slice());
                dict.remove(b"JavaScript".as_slice());
                for key in [b"OpenAction".as_slice(), b"A"] {
                    if dict.get(key).is_some_and(|v| runs_script(v, actions)) {
                        dict.remove(key);
                    }
                }
                if dict.get(b"JS".as_slice()).is_some() {
                    dict.insert(b"JS".to_vec(), Object::String(Vec::new()));
                }
                stack.extend(dict.values_mut());
            }
            _ => {}
        }
    }
}

/// `outlines` (sixth review): named destinations (bookmark and link
/// targets) are renamed `dest<n>`, in the order of the name tree so it
/// stays sorted, and the names of the catalog's /Dests dictionary
/// likewise; every link and go-to action naming one is given its new name.
/// A name can quote a heading under a box.
fn rename_destinations(target: &mut Builder, op: &str) -> Result<(), AppError> {
    let root = target.document().root();
    let tree = Place::object(root)
        .child(target.document(), b"Names")
        .child(target.document(), b"Dests");
    // The tree's nodes in order: the root (a place), then its kids.
    let mut nodes: Vec<Place> = Vec::new();
    let mut strings: std::collections::BTreeMap<Vec<u8>, Vec<u8>> = Default::default();
    {
        let doc = target.document();
        if tree.read(doc).as_dict().is_some() {
            let mut stack = vec![tree.clone()];
            let mut seen: BTreeSet<ObjectId> = BTreeSet::new();
            while let Some(place) = stack.pop() {
                if nodes.len() >= MAX_WALK_NODES {
                    return Err(AppError::reason(
                        "too-large",
                        format!("{op}: the destinations have more than {MAX_WALK_NODES} names"),
                    ));
                }
                let Some(node) = place.read(doc).as_dict() else {
                    continue;
                };
                for (at, entry) in (doc.dict_get(node, b"Names").as_array().unwrap_or(&[]))
                    .iter()
                    .enumerate()
                {
                    if let (0, Some(key)) = (at % 2, doc.resolve(entry).as_string()) {
                        let next = strings.len() + 1;
                        strings
                            .entry(key.to_vec())
                            .or_insert_with(|| format!("dest{next:08}").into_bytes());
                    }
                }
                let kids = doc.dict_get(node, b"Kids").as_array().unwrap_or(&[]);
                for kid in kids.iter().rev() {
                    if let Some(id) = kid.as_reference() {
                        if seen.insert(id) {
                            stack.push(Place::object(id));
                        }
                    }
                }
                nodes.push(place);
            }
        }
    }
    for place in &nodes {
        place
            .update(target, |value| {
                let Some(node) = value.as_dict_mut() else {
                    return;
                };
                for key in [b"Names".as_slice(), b"Limits"] {
                    let step = if key == b"Names" { 2 } else { 1 };
                    if let Some(Object::Array(items)) = node.get_mut(key) {
                        for item in items.iter_mut().step_by(step) {
                            if let Object::String(text) = item {
                                match strings.get(text.as_slice()) {
                                    Some(new) => *text = new.clone(),
                                    // A limit naming no destination.
                                    None => text.clear(),
                                }
                            }
                        }
                    }
                }
            })
            .map_err(|e| plain(op, e))?;
    }
    // PDF 1.1: the catalog's /Dests dictionary, keyed by names.
    let old = Place::object(root).child(target.document(), b"Dests");
    let mut names: std::collections::BTreeMap<Vec<u8>, Vec<u8>> = Default::default();
    if let Some(dict) = old.read(target.document()).as_dict() {
        for (index, key) in dict.keys().enumerate() {
            names.insert(key.clone(), format!("dest{:08}", index + 1).into_bytes());
        }
        old.update(target, |value| {
            if let Some(dict) = value.as_dict_mut() {
                let entries: Vec<(Vec<u8>, Object)> =
                    dict.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                *dict = entries
                    .into_iter()
                    .filter_map(|(k, v)| names.get(&k).map(|n| (n.clone(), v)))
                    .collect();
            }
        })
        .map_err(|e| plain(op, e))?;
    }
    if strings.is_empty() && names.is_empty() {
        return Ok(());
    }
    // What names them: a /Dest, a go-to action's /D.
    let users: Vec<ObjectId> = (target.document().objects())
        .filter(|o| names_destination(&o.value, &strings, &names))
        .map(|o| o.id)
        .collect();
    for id in users {
        target
            .update(id, |value| rename_uses(value, &strings, &names))
            .map_err(|e| plain(op, e))?;
    }
    Ok(())
}

type Renames = std::collections::BTreeMap<Vec<u8>, Vec<u8>>;

/// The destination name a dictionary's entry `key` holds, when it is one
/// that `strings` (texts) or `names` rename.
fn destination_entry<'v>(dict: &'v mut Dictionary, key: &[u8]) -> Option<&'v mut Object> {
    let goto = matches!(
        dict.get(b"S".as_slice()).and_then(Object::as_name),
        Some(b"GoTo")
    );
    match key {
        b"Dest" => dict.get_mut(b"Dest".as_slice()),
        b"D" if goto => dict.get_mut(b"D".as_slice()),
        _ => None,
    }
}

fn renamed<'r>(value: &Object, strings: &'r Renames, names: &'r Renames) -> Option<&'r Vec<u8>> {
    match value {
        Object::String(text) => strings.get(text.as_slice()),
        Object::Name(name) => names.get(name.as_slice()),
        _ => None,
    }
}

/// Whether `value` (direct values at any depth) names a renamed
/// destination.
fn names_destination(value: &Object, strings: &Renames, names: &Renames) -> bool {
    let mut stack = vec![value];
    let mut visits = 0usize;
    while let Some(value) = stack.pop() {
        visits += 1;
        if visits > MAX_WALK_NODES {
            return true;
        }
        match value {
            Object::Array(items) => stack.extend(items),
            Object::Dictionary(dict) => {
                let goto = dict.get(b"S".as_slice()).and_then(Object::as_name) == Some(b"GoTo");
                let named = |key: &[u8]| {
                    dict.get(key)
                        .is_some_and(|v| renamed(v, strings, names).is_some())
                };
                if named(b"Dest") || (goto && named(b"D")) {
                    return true;
                }
                stack.extend(dict.values());
            }
            _ => {}
        }
    }
    false
}

fn rename_uses(value: &mut Object, strings: &Renames, names: &Renames) {
    visit_destinations(value, |item| {
        if let Some(new) = renamed(item, strings, names) {
            match item {
                Object::String(text) => *text = new.clone(),
                Object::Name(name) => *name = new.clone(),
                _ => {}
            }
        }
    });
}

/// Calls `each` on every /Dest and go-to /D value in `value`'s direct
/// values.
fn visit_destinations(value: &mut Object, mut each: impl FnMut(&mut Object)) {
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
                for key in [b"Dest".as_slice(), b"D"] {
                    if let Some(item) = destination_entry(dict, key) {
                        each(item);
                    }
                }
                stack.extend(dict.values_mut());
            }
            _ => {}
        }
    }
}

/// redact.ts: each optional-content group's /Name becomes `Layer <n>`; the
/// configurations (/D, /Configs) lose their /Name too. Sixth review: so
/// does every group the list does not name (one a page's resources or an
/// annotation names), and the configurations lose their /Creator and the
/// labels of their layer lists (/Order).
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
        (place.read(target.document()).as_dict()).is_some_and(|d| {
            [b"Name".as_slice(), b"Creator", b"Order"]
                .iter()
                .any(|k| d.get(*k).is_some())
        })
    };
    if named(&config, target) {
        config
            .update(target, scrub_config)
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
                        .update(target, scrub_config)
                        .map_err(|e| plain(op, e))?;
                }
            }
            None => {
                configs
                    .update(target, |value| {
                        if let Some(config) =
                            value.as_array_mut().and_then(|items| items.get_mut(index))
                        {
                            scrub_config(config);
                        }
                    })
                    .map_err(|e| plain(op, e))?;
            }
        }
    }
    // Every other group: in any object, at any depth of its direct values.
    let listed: BTreeSet<ObjectId> = entries.iter().flatten().copied().collect();
    let others: Vec<ObjectId> = (target.document().objects())
        .filter(|o| !listed.contains(&o.id) && holds_group(&o.value))
        .map(|o| o.id)
        .collect();
    let mut next = entries.len();
    for id in others {
        target
            .update(id, |value| rename_groups(value, &mut next))
            .map_err(|e| plain(op, e))?;
    }
    Ok(())
}

fn is_group(dict: &Dictionary) -> bool {
    dict.get(b"Type".as_slice()).and_then(Object::as_name) == Some(b"OCG")
}

/// Whether `value` is or holds (directly) a group with a name.
fn holds_group(value: &Object) -> bool {
    let mut stack = vec![value];
    let mut visits = 0usize;
    while let Some(value) = stack.pop() {
        visits += 1;
        if visits > MAX_WALK_NODES {
            return true;
        }
        match value {
            Object::Array(items) => stack.extend(items),
            Object::Dictionary(dict) => {
                if is_group(dict) && dict.get(b"Name".as_slice()).is_some() {
                    return true;
                }
                stack.extend(dict.values());
            }
            _ => {}
        }
    }
    false
}

/// Each group `value` is or holds directly becomes `Layer <n>`, counting
/// on from `next`.
fn rename_groups(value: &mut Object, next: &mut usize) {
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
                if is_group(dict) && dict.get(b"Name".as_slice()).is_some() {
                    *next += 1;
                    dict.insert(b"Name".to_vec(), Object::text(&format!("Layer {next}")));
                }
                stack.extend(dict.values_mut());
            }
            _ => {}
        }
    }
}

/// A configuration loses its /Name, its /Creator and the labels (texts)
/// of its layer list.
fn scrub_config(value: &mut Object) {
    let Some(dict) = value.as_dict_mut() else {
        return;
    };
    dict.remove(b"Name".as_slice());
    dict.remove(b"Creator".as_slice());
    let Some(order) = dict.get_mut(b"Order".as_slice()) else {
        return;
    };
    let mut stack = vec![order];
    let mut visits = 0usize;
    while let Some(value) = stack.pop() {
        visits += 1;
        if visits > MAX_WALK_NODES {
            return;
        }
        if let Object::Array(items) = value {
            items.retain(|item| !matches!(item, Object::String(_)));
            stack.extend(items.iter_mut());
        }
    }
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
