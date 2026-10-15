// SPDX-License-Identifier: AGPL-3.0-or-later
//! The surfaces redact-apply's sanitize options clear (redact.ts
//! `redactInventory`): string Info entries, XMP metadata, embedded files,
//! outline items, annotations, links, filled fields, named layers and the
//! alternate texts of the structure tree and of the property lists named
//! in resources. Every walk is bounded and refuses (`too-large`) past its
//! bound instead of counting part of a document.
use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::app::edit::Place;
use crate::app::{AppError, info};
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId, decode_text_string};

/// Objects one walk (outline, structure tree, field tree, name tree) visits
/// at most.
pub const MAX_WALK_NODES: usize = 1_000_000;
/// Info entries a sanitize keeps (redact.ts).
pub const KEPT_INFO: [&[u8]; 3] = [b"Producer", b"CreationDate", b"ModDate"];
/// Keys the structure sanitize removes.
pub const STRUCT_KEYS: [&[u8]; 3] = [b"ActualText", b"Alt", b"E"];

/// redact.ts `RedactInventory`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    pub info: usize,
    pub xmp: bool,
    pub attachments: usize,
    pub outlines: usize,
    pub annots: usize,
    pub links: usize,
    pub fields: usize,
    pub layers: usize,
    pub structure: usize,
}

impl Inventory {
    pub fn json(&self) -> Value {
        json!({
            "info": self.info, "xmp": self.xmp, "attachments": self.attachments,
            "outlines": self.outlines, "annots": self.annots, "links": self.links,
            "fields": self.fields, "layers": self.layers, "struct": self.structure,
        })
    }
}

fn too_large(op: &str, what: &str) -> AppError {
    AppError::reason(
        "too-large",
        format!("{op}: the {what} has more than {MAX_WALK_NODES} objects"),
    )
}

/// A bounded visit of objects: each indirect object once.
struct Visits {
    seen: BTreeSet<ObjectId>,
    count: usize,
}

impl Visits {
    fn new() -> Self {
        Self {
            seen: BTreeSet::new(),
            count: 0,
        }
    }

    /// False when `holder` was visited; an error past the bound.
    fn visit(&mut self, holder: Option<ObjectId>, op: &str, what: &str) -> Result<bool, AppError> {
        if let Some(id) = holder {
            if !self.seen.insert(id) {
                return Ok(false);
            }
        }
        self.count += 1;
        if self.count > MAX_WALK_NODES {
            return Err(too_large(op, what));
        }
        Ok(true)
    }
}

fn catalog<'d>(doc: &'d Document<'_>) -> Option<&'d Dictionary> {
    doc.resolve_id(doc.root()).as_dict()
}

/// The document's inventory. `op` names refusals.
pub fn inventory(doc: &Document<'_>, op: &str) -> Result<Inventory, AppError> {
    let root = catalog(doc);
    let at = |key: &[u8]| root.map(|r| doc.dict_get(r, key)).filter(|v| !v.is_null());
    let (annots, links) = annotations(doc);
    Ok(Inventory {
        info: info_entries(doc),
        xmp: at(b"Metadata").is_some(),
        attachments: attachments(doc, root, op)?,
        outlines: outlines(doc, at(b"Outlines"), op)?,
        annots,
        links,
        fields: fields(doc, root, op)?,
        layers: layers(doc, root),
        structure: structure(
            doc,
            root.and_then(|r| r.get(b"StructTreeRoot".as_slice())),
            op,
        )? + (property_lists(doc, op)?.iter())
            .map(|(_, n)| n)
            .sum::<usize>(),
    })
}

/// Where a page's /Resources live: the page or the ancestor it inherits
/// them from (None for a page written into /Kids).
pub fn page_resources(
    doc: &Document<'_>,
    id: Option<ObjectId>,
    page: &Dictionary,
) -> Option<Place> {
    let mut holder = id?;
    let mut dict = page;
    for _ in 0..64 {
        if dict.get(b"Resources".as_slice()).is_some() {
            return Some(Place::object(holder).child(doc, b"Resources"));
        }
        let parent = dict.get(b"Parent".as_slice())?.as_reference()?;
        holder = parent;
        dict = doc.resolve_id(parent).as_dict()?;
    }
    None
}

/// The property lists named in resources (/Properties) that hold
/// /ActualText, /Alt or /E, each once with how many of them: those of
/// every page's resources and of every Form they reach.
pub fn property_lists(doc: &Document<'_>, op: &str) -> Result<Vec<(Place, usize)>, AppError> {
    let mut out = Vec::new();
    let mut visits = Visits::new();
    let mut seen: BTreeSet<(ObjectId, Vec<Vec<u8>>)> = BTreeSet::new();
    let mut queue: Vec<Place> = (info::pages(doc).iter())
        .filter_map(|page| page_resources(doc, page.id, page.dict))
        .collect();
    let mut forms: BTreeSet<ObjectId> = BTreeSet::new();
    while let Some(resources) = queue.pop() {
        if !seen.insert((resources.id, resources.keys.clone())) {
            continue;
        }
        visits.visit(None, op, "resource tree")?;
        let Some(dict) = resources.read(doc).as_dict() else {
            continue;
        };
        for value in (doc.dict_get(dict, b"XObject").as_dict())
            .map(|x| x.values().collect::<Vec<_>>())
            .unwrap_or_default()
        {
            let (Some(id), _) = doc.resolve_with_holder(value) else {
                continue;
            };
            let form = doc.stream_dict(id).is_ok_and(|d| {
                doc.dict_get(d, b"Subtype").as_name() == Some(b"Form")
                    && d.get(b"Resources".as_slice()).is_some()
            });
            if form && forms.insert(id) {
                queue.push(Place::object(id).child(doc, b"Resources"));
            }
        }
        let props = resources.child(doc, b"Properties");
        let names: Vec<Vec<u8>> = (props.read(doc).as_dict())
            .map(|p| p.keys().cloned().collect())
            .unwrap_or_default();
        for name in names {
            visits.visit(None, op, "resource tree")?;
            let place = props.child(doc, &name);
            let Some(list) = place.read(doc).as_dict() else {
                continue;
            };
            let n = (STRUCT_KEYS.iter())
                .filter(|key| !doc.dict_get(list, key).is_null())
                .count();
            if n > 0 && seen.insert((place.id, place.keys.clone())) {
                out.push((place, n));
            }
        }
    }
    Ok(out)
}

/// String-valued /Info entries other than Producer, CreationDate, ModDate.
pub fn info_entries(doc: &Document<'_>) -> usize {
    let Some(info) = doc
        .trailer_entry(b"Info")
        .and_then(|i| doc.resolve(i).as_dict())
    else {
        return 0;
    };
    info.iter()
        .filter(|(key, value)| {
            !KEPT_INFO.contains(&key.as_slice()) && doc.resolve(value).as_string().is_some()
        })
        .count()
}

/// Names in the /EmbeddedFiles name tree (MuPDF's `getEmbeddedFiles`, each
/// name once).
fn attachments(doc: &Document<'_>, root: Option<&Dictionary>, op: &str) -> Result<usize, AppError> {
    let tree = root
        .map(|r| doc.dict_get(r, b"Names"))
        .and_then(Object::as_dict)
        .and_then(|names| names.get(b"EmbeddedFiles".as_slice()));
    let mut names = BTreeSet::new();
    let mut visits = Visits::new();
    let mut queue: Vec<&Object> = tree.into_iter().collect();
    while let Some(node) = queue.pop() {
        let (holder, value) = doc.resolve_with_holder(node);
        if !visits.visit(holder, op, "embedded file tree")? {
            continue;
        }
        let Some(dict) = value.as_dict() else {
            continue;
        };
        if let Some(pairs) = doc.dict_get(dict, b"Names").as_array() {
            for key in pairs.chunks(2).filter_map(|pair| pair.first()) {
                if let Some(key) = doc.resolve(key).as_string() {
                    names.insert(decode_text_string(key));
                }
            }
        }
        if let Some(kids) = doc.dict_get(dict, b"Kids").as_array() {
            queue.extend(kids.iter().rev());
        }
    }
    Ok(names.len())
}

/// Outline items with a string /Title: the tree reached through /First,
/// /Last, /Next and /Prev.
fn outlines(doc: &Document<'_>, root: Option<&Object>, op: &str) -> Result<usize, AppError> {
    let mut count = 0;
    let mut visits = Visits::new();
    let mut queue: Vec<&Object> = root.into_iter().collect();
    while let Some(node) = queue.pop() {
        let (holder, value) = doc.resolve_with_holder(node);
        let Some(dict) = value.as_dict() else {
            continue;
        };
        if !visits.visit(holder, op, "outline")? {
            continue;
        }
        if doc.dict_get(dict, b"Title").as_string().is_some() {
            count += 1;
        }
        for key in [b"Prev".as_slice(), b"Last", b"Next", b"First"] {
            if let Some(next) = dict.get(key) {
                queue.push(next);
            }
        }
    }
    Ok(count)
}

/// Page annotations: (annotations a sanitize removes, links). Each page's
/// /Annots entries that are dictionaries, an object listed twice once;
/// Widgets, Redact marks and Popups are not counted.
fn annotations(doc: &Document<'_>) -> (usize, usize) {
    let (mut annots, mut links) = (0, 0);
    for page in info::pages(doc) {
        let Some(items) = doc.dict_get(page.dict, b"Annots").as_array() else {
            continue;
        };
        let mut seen = BTreeSet::new();
        for item in items {
            let (holder, value) = doc.resolve_with_holder(item);
            if holder.is_some_and(|id| !seen.insert(id)) {
                continue;
            }
            let Some(dict) = value.as_dict() else {
                continue;
            };
            match doc.dict_get(dict, b"Subtype").as_name() {
                Some(b"Link") => links += 1,
                Some(b"Widget" | b"Redact" | b"Popup") => {}
                _ => annots += 1,
            }
        }
    }
    (annots, links)
}

/// Field-tree nodes holding /V, /DV or /AP (redact.ts `fieldNodes`).
fn fields(doc: &Document<'_>, root: Option<&Dictionary>, op: &str) -> Result<usize, AppError> {
    let top = root
        .map(|r| doc.dict_get(r, b"AcroForm"))
        .and_then(Object::as_dict)
        .and_then(|form| doc.dict_get(form, b"Fields").as_array());
    let mut count = 0;
    let mut visits = Visits::new();
    let mut queue: Vec<&Object> = top.unwrap_or(&[]).iter().rev().collect();
    while let Some(node) = queue.pop() {
        let (holder, value) = doc.resolve_with_holder(node);
        if !visits.visit(holder, op, "field tree")? {
            continue;
        }
        let Some(dict) = value.as_dict() else {
            continue;
        };
        if [b"V".as_slice(), b"DV", b"AP"]
            .iter()
            .any(|key| !doc.dict_get(dict, key).is_null())
        {
            count += 1;
        }
        if let Some(kids) = doc.dict_get(dict, b"Kids").as_array() {
            queue.extend(kids.iter().rev());
        }
    }
    Ok(count)
}

/// Optional-content groups with a /Name.
fn layers(doc: &Document<'_>, root: Option<&Dictionary>) -> usize {
    let groups = root
        .map(|r| doc.dict_get(r, b"OCProperties"))
        .and_then(Object::as_dict)
        .and_then(|oc| doc.dict_get(oc, b"OCGs").as_array());
    groups
        .unwrap_or(&[])
        .iter()
        .filter_map(|group| doc.resolve(group).as_dict())
        .filter(|group| !doc.dict_get(group, b"Name").is_null())
        .count()
}

/// Objects the structure walk does not enter: what structure elements
/// point at (pages, annotations, XObjects, content streams), not part of
/// the tree.
pub fn outside_tree(doc: &Document<'_>, holder: Option<ObjectId>, dict: &Dictionary) -> bool {
    if holder.is_some_and(|id| doc.object(id).is_ok_and(|o| o.stream.is_some())) {
        return true;
    }
    matches!(
        doc.dict_get(dict, b"Type").as_name(),
        Some(b"Page" | b"Pages" | b"Catalog" | b"Annot" | b"XObject" | b"Font")
    )
}

/// /ActualText, /Alt and /E entries in the structure tree: every
/// dictionary reached from /StructTreeRoot (its /K, /ParentTree, /IDTree,
/// attributes) without following /Pg, /Obj, /Stm or /StmOwn out of it.
fn structure(doc: &Document<'_>, root: Option<&Object>, op: &str) -> Result<usize, AppError> {
    const OUT: [&[u8]; 4] = [b"Pg", b"Obj", b"Stm", b"StmOwn"];
    let mut count = 0;
    let mut visits = Visits::new();
    let mut queue: Vec<&Object> = root.into_iter().collect();
    while let Some(node) = queue.pop() {
        let (holder, value) = doc.resolve_with_holder(node);
        match value {
            Object::Array(items) => {
                if !visits.visit(holder, op, "structure tree")? {
                    continue;
                }
                queue.extend(items.iter().rev());
            }
            Object::Dictionary(dict) => {
                if outside_tree(doc, holder, dict) || !visits.visit(holder, op, "structure tree")? {
                    continue;
                }
                count += STRUCT_KEYS
                    .iter()
                    .filter(|key| !doc.dict_get(dict, key).is_null())
                    .count();
                for (key, value) in dict.iter() {
                    if !OUT.contains(&key.as_slice()) {
                        queue.push(value);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(count)
}
