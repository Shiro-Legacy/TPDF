// SPDX-License-Identifier: AGPL-3.0-or-later
//! Document facts (engine.ts `detect`), permissions, and the page list with
//! the geometry replies carry.
//!
//! Pages are enumerated as hayro enumerates them, so page indices agree with
//! the view: a depth-first walk of /Kids from the catalog's /Pages that skips
//! nodes already visited and treats every dictionary that is not /Type /Pages
//! as a page. Without a usable root the list is every /Type /Page dictionary,
//! in object order, and the view draws from a page tree listing the same
//! (`view_fixes`); hayro alone would take every dictionary with /Contents,
//! annotations included. Sizes are MuPDF's bound page, computed as the view
//! module computes them.
use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId, decode_text_string};

/// The five permissions MuPDF reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Permissions {
    pub print: bool,
    pub copy: bool,
    pub edit: bool,
    pub annotate: bool,
    pub form: bool,
}

/// engine-core.ts `PERMISSION_BITS`: the /P bits the app reads and writes.
pub const PERMISSION_BITS: [(&str, u32); 5] = [
    ("print", 4),
    ("edit", 8),
    ("copy", 16),
    ("annotate", 32),
    ("form", 256),
];

impl Permissions {
    pub const ALL: Self = Self {
        print: true,
        copy: true,
        edit: true,
        annotate: true,
        form: true,
    };

    /// The permissions `/P` grants.
    pub fn from_bits(bits: u32) -> Self {
        let has = |bit: u32| bits & bit == bit;
        Self {
            print: has(4),
            copy: has(16),
            edit: has(8),
            annotate: has(32),
            form: has(256),
        }
    }

    pub fn get(self, name: &str) -> bool {
        match name {
            "print" => self.print,
            "copy" => self.copy,
            "edit" => self.edit,
            "annotate" => self.annotate,
            "form" => self.form,
            _ => false,
        }
    }

    /// engine-core.ts `permissionInt`: the sum of the granted bits.
    pub fn bits(self) -> u32 {
        PERMISSION_BITS
            .iter()
            .filter(|(name, _)| self.get(name))
            .map(|(_, bit)| bit)
            .sum()
    }

    /// engine-core.ts `permissionNames`: granted names, in the fixed order.
    pub fn names(self) -> Vec<&'static str> {
        PERMISSION_BITS
            .iter()
            .filter(|(name, _)| self.get(name))
            .map(|(name, _)| *name)
            .collect()
    }

    pub fn json(self) -> Value {
        json!({"print": self.print, "copy": self.copy, "edit": self.edit,
            "annotate": self.annotate, "form": self.form})
    }
}

/// engine.ts `DocumentInfo`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentInfo {
    pub encrypted: bool,
    pub needs_password: bool,
    /// `none`, `rc4-40`, `rc4-128`, `aes-128` or `aes-256`.
    pub algorithm: &'static str,
    pub repaired: bool,
    pub signatures: usize,
    pub certified: bool,
    pub xfa: bool,
    pub attachments: usize,
    pub open_action: bool,
    pub tagged: bool,
    pub versions: usize,
    pub printable: bool,
    pub permissions: Permissions,
}

impl DocumentInfo {
    pub fn json(&self) -> Value {
        json!({
            "encrypted": self.encrypted,
            "needsPassword": self.needs_password,
            "algorithm": self.algorithm,
            "repaired": self.repaired,
            "signatures": self.signatures,
            "certified": self.certified,
            "xfa": self.xfa,
            "attachments": self.attachments,
            "openAction": self.open_action,
            "tagged": self.tagged,
            "versions": self.versions,
            "printable": self.printable,
            "permissions": self.permissions.json(),
        })
    }

    /// Signed or certified: engine.ts keeps such documents read-only.
    pub fn signed(&self) -> bool {
        self.signatures > 0 || self.certified
    }
}

/// engine.ts `detect(doc, needsPassword)`. `repaired` is the repair verdict
/// of the open; a locked document (opened without its password) reports the
/// permissions of its /P, as MuPDF does before authentication.
pub fn detect(doc: &Document<'_>, needs_password: bool, repaired: bool) -> DocumentInfo {
    let encrypt = doc
        .trailer_entry(b"Encrypt")
        .map(|value| doc.resolve(value))
        .filter(|value| !value.is_null());
    let permissions = permissions(doc);
    let root = doc.resolve_id(doc.root());
    let acro = present(doc, root, b"AcroForm");
    DocumentInfo {
        encrypted: doc.security().is_some() || encrypt.is_some(),
        needs_password,
        algorithm: encrypt.map_or("none", |encrypt| algorithm(doc, encrypt)),
        repaired,
        signatures: acro.map_or(0, |acro| count_signatures(doc, acro)),
        certified: present(doc, root, b"Perms")
            .and_then(|perms| present(doc, perms, b"DocMDP"))
            .is_some(),
        xfa: acro.and_then(|acro| present(doc, acro, b"XFA")).is_some(),
        attachments: attachments(doc, root),
        open_action: present(doc, root, b"OpenAction").is_some()
            || present(doc, root, b"AA").is_some(),
        tagged: present(doc, root, b"MarkInfo")
            .and_then(|mark| present(doc, mark, b"Marked"))
            .and_then(Object::as_bool)
            .unwrap_or(false),
        versions: doc.trailer_chain().count().saturating_sub(1),
        printable: permissions.print,
        permissions,
    }
}

/// MuPDF `hasPermission`: everything without encryption or with the owner
/// password, otherwise /P. A missing or non-integer /P permits everything.
pub fn permissions(doc: &Document<'_>) -> Permissions {
    if let Some(security) = doc.security() {
        if security.owner {
            return Permissions::ALL;
        }
        return Permissions::from_bits(security.permissions as u32);
    }
    match permission_bits(doc) {
        Some(bits) => Permissions::from_bits(bits),
        None => Permissions::ALL,
    }
}

/// engine-core.ts `permissionBitsOf`: /P as an unsigned 32-bit value, read
/// from /Encrypt whoever authenticated; None without one.
pub fn permission_bits(doc: &Document<'_>) -> Option<u32> {
    let encrypt = doc.resolve(doc.trailer_entry(b"Encrypt")?).as_dict()?;
    let value = doc.dict_get(encrypt, b"P");
    match value {
        Object::Integer(p) => Some(*p as u32),
        Object::Real(p) if p.is_finite() => Some((p.trunc() as i64) as u32),
        _ => None,
    }
}

/// engine-core.ts `algorithmOf`, on the resolved /Encrypt value.
fn algorithm(doc: &Document<'_>, encrypt: &Object) -> &'static str {
    let number = |key: &[u8]| {
        present(doc, encrypt, key)
            .and_then(Object::as_f64)
            .unwrap_or(0.0)
    };
    let (v, r, length) = (number(b"V"), number(b"R"), number(b"Length"));
    if v >= 5.0 {
        return "aes-256";
    }
    if v == 4.0 {
        let cfm = present(doc, encrypt, b"CF")
            .and_then(|cf| present(doc, cf, b"StdCF"))
            .and_then(|std| present(doc, std, b"CFM"));
        let aes256 = cfm
            .and_then(|cfm| cfm.as_name().or_else(|| cfm.as_string()))
            .is_some_and(|text| text.windows(5).any(|w| w == b"AESV3"));
        return if aes256 { "aes-256" } else { "aes-128" };
    }
    if r <= 2.0 || v <= 1.0 {
        return "rc4-40";
    }
    if length == 40.0 { "rc4-40" } else { "rc4-128" }
}

/// engine-core.ts `at`: `object[key]` resolved, or None when it is null or
/// `object` is not a dictionary (a stream's dictionary counts).
fn present<'d>(doc: &'d Document<'_>, object: &'d Object, key: &[u8]) -> Option<&'d Object> {
    let dict = doc.resolve(object).as_dict()?;
    let value = doc.dict_get(dict, key);
    (!value.is_null()).then_some(value)
}

/// engine.ts `countSignatures`: fields under /AcroForm /Fields and their
/// /Kids whose own /FT is /Sig and which have their own /V. Shared or cyclic
/// nodes count once.
fn count_signatures(doc: &Document<'_>, acro: &Object) -> usize {
    let Some(fields) = present(doc, acro, b"Fields").and_then(Object::as_array) else {
        return 0;
    };
    let mut seen = BTreeSet::new();
    let mut stack: Vec<(&Object, usize)> = fields.iter().rev().map(|f| (f, 0)).collect();
    let mut total = 0;
    while let Some((field, depth)) = stack.pop() {
        if let Object::Reference(id) = field {
            if !seen.insert(*id) {
                continue;
            }
        }
        let signed = present(doc, field, b"FT").and_then(Object::as_name) == Some(b"Sig")
            && present(doc, field, b"V").is_some();
        total += usize::from(signed);
        if depth >= 64 {
            continue;
        }
        if let Some(kids) = present(doc, field, b"Kids").and_then(Object::as_array) {
            stack.extend(kids.iter().rev().map(|kid| (kid, depth + 1)));
        }
    }
    total
}

/// mupdf.js `getEmbeddedFiles` key count: distinct decoded names in the
/// /Names arrays of the /Root /Names /EmbeddedFiles tree and its /Kids.
fn attachments(doc: &Document<'_>, root: &Object) -> usize {
    let Some(tree) = present(doc, root, b"Names").and_then(|n| present(doc, n, b"EmbeddedFiles"))
    else {
        return 0;
    };
    let mut names = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut stack = vec![(tree, 0usize)];
    while let Some((node, depth)) = stack.pop() {
        if let Object::Reference(id) = node {
            if !seen.insert(*id) {
                continue;
            }
        }
        let Some(dict) = doc.resolve(node).as_dict() else {
            continue;
        };
        if let Some(pairs) = doc.dict_get(dict, b"Names").as_array() {
            for name in pairs.iter().step_by(2) {
                let text = match doc.resolve(name) {
                    Object::String(bytes) => decode_text_string(bytes),
                    // mupdf.js asString() of anything else is the empty string.
                    _ => String::new(),
                };
                names.insert(text);
            }
        }
        if depth >= 64 {
            continue;
        }
        if let Some(kids) = doc.dict_get(dict, b"Kids").as_array() {
            stack.extend(kids.iter().rev().map(|kid| (kid, depth + 1)));
        }
    }
    names.len()
}

/// One page of the page list.
#[derive(Clone, Copy, Debug)]
pub struct PageRef<'d> {
    /// The indirect object holding the page dictionary; None for a page
    /// written directly into a /Kids array.
    pub id: Option<ObjectId>,
    pub dict: &'d Dictionary,
}

/// The pages in display order, as the view enumerates them.
pub fn pages<'d>(doc: &'d Document<'_>) -> Vec<PageRef<'d>> {
    let mut pages = Vec::new();
    if !walk_tree(doc, |page, _, _| pages.push(page)) {
        pages = brute_force(doc);
    }
    pages
}

static NULL: Object = Object::Null;

fn kids<'d>(doc: &'d Document<'_>, dict: &'d Dictionary) -> Option<&'d [Object]> {
    doc.dict_get(dict, b"Kids").as_array()
}

/// A /Pages node on a page's path from the root.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TreeNode<'d> {
    pub(crate) id: Option<ObjectId>,
    pub(crate) dict: &'d Dictionary,
}

/// Where a page's dictionary is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PageSlot {
    /// Its own object.
    Object(ObjectId),
    /// Entry `index` of a /Kids array that is the value of `holder` (an
    /// array object) or, `in_node`, the /Kids entry of the node `holder`.
    Kid {
        holder: ObjectId,
        in_node: bool,
        index: usize,
    },
    /// Inside a node that is itself written into /Kids.
    Nested,
}

/// Visits the pages of the catalog's page tree in display order (hayro's
/// `resolve_pages`), each with the /Pages nodes from the root to its
/// container and where it is written. False, visiting nothing, when there
/// is no usable tree: no /Pages dictionary (or a stream) with a /Kids array.
pub(crate) fn walk_tree<'d>(
    doc: &'d Document<'_>,
    mut visit: impl FnMut(PageRef<'d>, &[TreeNode<'d>], PageSlot),
) -> bool {
    let catalog = doc.resolve_id(doc.root()).as_dict();
    let root =
        catalog.map(|c| doc.resolve_with_holder(c.get(b"Pages".as_slice()).unwrap_or(&NULL)));
    let Some((root_id, Object::Dictionary(root))) = root else {
        return false;
    };
    if root_id.is_some_and(|id| doc.object(id).is_ok_and(|o| o.stream.is_some())) {
        return false;
    }
    let Some(root_kids) = doc.dict_get(root, b"Kids").as_array() else {
        return false;
    };
    let mut visited: BTreeSet<ObjectId> = root_id.into_iter().collect();
    let mut path = vec![TreeNode {
        id: root_id,
        dict: root,
    }];
    // Each frame: its node's /Kids, the holder of that array, the next index.
    let holder = |node: TreeNode<'d>| match node.dict.get(b"Kids".as_slice()) {
        Some(Object::Reference(id)) => Some((*id, false)),
        _ => node.id.map(|id| (id, true)),
    };
    let mut stack = vec![(root_kids.iter().enumerate(), holder(path[0]))];
    while let Some((frame, kids_holder)) = stack.last_mut() {
        let Some((index, kid)) = frame.next() else {
            stack.pop();
            path.pop();
            continue;
        };
        let kids_holder = *kids_holder;
        let (id, dict) = match kid {
            Object::Reference(id) => {
                if !visited.insert(*id) {
                    continue;
                }
                match doc.object(*id) {
                    Ok(object) if object.stream.is_none() => (Some(*id), object.value.as_dict()),
                    _ => continue,
                }
            }
            other => (None, other.as_dict()),
        };
        let Some(dict) = dict else { continue };
        if doc.dict_get(dict, b"Type").as_name() == Some(b"Pages") {
            if let Some(kids) = kids(doc, dict) {
                let node = TreeNode { id, dict };
                path.push(node);
                stack.push((kids.iter().enumerate(), holder(node)));
            }
        } else {
            let slot = match (id, kids_holder) {
                (Some(id), _) => PageSlot::Object(id),
                (None, Some((holder, in_node))) => PageSlot::Kid {
                    holder,
                    in_node,
                    index,
                },
                (None, None) => PageSlot::Nested,
            };
            visit(PageRef { id, dict }, &path, slot);
        }
    }
    true
}

/// Every /Type /Page dictionary, in object order: the page list when the
/// catalog has no usable page tree.
fn brute_force<'d>(doc: &'d Document<'_>) -> Vec<PageRef<'d>> {
    doc.objects()
        .filter(|object| object.stream.is_none())
        .filter_map(|object| {
            let dict = object.value.as_dict()?;
            (doc.dict_get(dict, b"Type").as_name() == Some(b"Page")).then_some(PageRef {
                id: Some(object.id),
                dict,
            })
        })
        .collect()
}

/// `key` on the page or the nearest /Parent that has a value `convert`
/// accepts, as the view's `inherited` reads it.
fn inherited<'d, T>(
    doc: &'d Document<'_>,
    page: &'d Dictionary,
    key: &[u8],
    convert: impl Fn(&'d Object) -> Option<T>,
) -> Option<T> {
    let mut node = page;
    for _ in 0..64 {
        if let Some(value) = node.get(key).and_then(|v| convert(doc.resolve(v))) {
            return Some(value);
        }
        node = doc.dict_get(node, b"Parent").as_dict()?;
    }
    None
}

fn f32_of(doc: &Document<'_>, value: &Object) -> Option<f32> {
    match doc.resolve(value) {
        Object::Integer(v) => Some(*v as f32),
        Object::Real(v) => Some(*v as f32),
        _ => None,
    }
}

/// The view's `box_of`: missing or non-numeric entries read as zero.
fn box_of(doc: &Document<'_>, items: &[Object]) -> [f64; 4] {
    let mut v = [0.0f64; 4];
    for (slot, item) in v.iter_mut().zip(items) {
        *slot = f32_of(doc, item).map_or(0.0, f64::from);
    }
    [
        v[0].min(v[2]),
        v[1].min(v[3]),
        v[0].max(v[2]),
        v[1].max(v[3]),
    ]
}

/// The page's inherited /MediaBox or /CropBox (`key`) as the view reads it.
pub(crate) fn page_box(doc: &Document<'_>, page: &Dictionary, key: &[u8]) -> Option<[f64; 4]> {
    inherited(doc, page, key, |v| v.as_array().map(|a| box_of(doc, a)))
}

/// MuPDF's bound page size in points (CropBox within MediaBox, UserUnit,
/// /Rotate snapped to a quarter turn), computed as the view's `PageFrame`.
pub fn page_size(doc: &Document<'_>, page: &Dictionary) -> (f64, f64) {
    let letter = [0.0, 0.0, 612.0, 792.0];
    let empty = |r: &[f64; 4]| !(r[0] < r[2] && r[1] < r[3]);
    let boxed = |key: &[u8]| page_box(doc, page, key);
    // A missing MediaBox is empty; Letter replaces an empty intersection.
    let mut area = boxed(b"MediaBox").unwrap_or([0.0; 4]);
    if let Some(crop) = boxed(b"CropBox") {
        let x0 = area[0].max(crop[0]);
        let y0 = area[1].max(crop[1]);
        area = [
            x0,
            y0,
            area[2].min(crop[2]).max(x0),
            area[3].min(crop[3]).max(y0),
        ];
    }
    if empty(&area) {
        area = letter;
    } else if area[2] - area[0] < 1.0 || area[3] - area[1] < 1.0 {
        area = [0.0, 0.0, 1.0, 1.0];
    }
    let unit = page
        .get(b"UserUnit".as_slice())
        .and_then(|v| f32_of(doc, v))
        .filter(|u| u.is_finite() && *u > 0.0)
        .unwrap_or(1.0);
    let rotate = display_rotation(doc, page);
    let (x0, y0, x1, y1) = (
        area[0] as f32,
        area[1] as f32,
        area[2] as f32,
        area[3] as f32,
    );
    let w = x1 * unit - x0 * unit;
    let h = y1 * unit - y0 * unit;
    let (w, h) = if rotate == 90 || rotate == 270 {
        (h, w)
    } else {
        (w, h)
    };
    (f64::from(w), f64::from(h))
}

/// The inherited /Rotate as MuPDF displays the page: rounded, then snapped
/// to a quarter turn (0, 90, 180 or 270), as the view's `PageFrame`.
pub(crate) fn display_rotation(doc: &Document<'_>, page: &Dictionary) -> i32 {
    let rotate = inherited(doc, page, b"Rotate", |v| f32_of(doc, v));
    // MuPDF's pdf_to_int rounds a real half up.
    let mut rotate = rotate.map_or(0, |r| (r + 0.5).floor() as i32);
    if rotate < 0 {
        rotate = 360 - (rotate.unsigned_abs() % 360) as i32;
    }
    rotate = 90 * ((rotate % 360 + 45) / 90);
    if rotate >= 360 { 0 } else { rotate }
}

/// engine.ts `pageRotation`: the inherited /Rotate truncated into 0..360,
/// not snapped (45 stays 45); missing or non-numeric is 0.
pub fn page_rotation(doc: &Document<'_>, page: &Dictionary) -> i64 {
    let mut node = page;
    for _ in 0..64 {
        if let Some(value) = node.get(b"Rotate".as_slice()) {
            let value = doc.resolve(value).as_f64().unwrap_or(0.0);
            let turned = if value.is_finite() {
                value.trunc()
            } else {
                0.0
            };
            return ((turned % 360.0 + 360.0) % 360.0) as i64;
        }
        let Some(parent) = doc.dict_get(node, b"Parent").as_dict() else {
            break;
        };
        node = parent;
    }
    0
}

/// engine.ts `PageState`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageState {
    pub sizes: Vec<(f64, f64)>,
    pub rotations: Vec<i64>,
}

impl PageState {
    pub fn of(doc: &Document<'_>) -> Self {
        let pages = pages(doc);
        Self {
            sizes: pages.iter().map(|p| page_size(doc, p.dict)).collect(),
            rotations: pages.iter().map(|p| page_rotation(doc, p.dict)).collect(),
        }
    }

    pub fn count(&self) -> usize {
        self.sizes.len()
    }

    /// `{pageCount, pages, rotations}`.
    pub fn json(&self) -> Map<String, Value> {
        let mut fields = Map::new();
        fields.insert("pageCount".into(), self.count().into());
        let pages = self
            .sizes
            .iter()
            .map(|&(width, height)| json!({"width": width, "height": height}));
        fields.insert("pages".into(), pages.collect());
        fields.insert("rotations".into(), self.rotations.clone().into());
        fields
    }
}
