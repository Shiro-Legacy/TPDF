// SPDX-License-Identifier: MIT
//! Document facts (engine.ts `detect`), permissions, and the page list with
//! the geometry replies carry.
//!
//! Pages are enumerated as hayro enumerates them, so page indices agree with
//! the view: a depth-first walk of /Kids from the catalog's /Pages that skips
//! nodes already visited and treats every dictionary that is not a node
//! (`is_node`: /Type /Pages, or untyped with /Kids, as the repair reads it)
//! as a page; the view types an untyped node /Pages so that hayro walks it
//! too (`view_fixes`). Without a usable root the list is every /Type /Page dictionary,
//! in object order, and the view draws from a page tree listing the same
//! (`view_fixes`); hayro alone would take every dictionary with /Contents,
//! annotations included. A direct /Pages dictionary, which hayro does not
//! read, is walked too and becomes an object in the view. Sizes are the
//! page bounds, computed as the view module computes them.
use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId, decode_text_string};
use crate::security::{EncryptionInfo, Method};

use super::pagetree;
use crate::view::{FrameNumbers, frame_of};

/// The five permissions reported for a document.
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
/// permissions of its /P.
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

/// The granted permissions: everything without encryption or with the owner
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

/// engine-core.ts `algorithmOf`, read as the security handler reads the
/// resolved /Encrypt value (`EncryptionInfo`): V5 or AESV3 is `aes-256`,
/// AESV2 `aes-128`, RC4 `rc4-40` for R2 or a 40-bit key and `rc4-128`
/// otherwise, and Identity or an unknown crypt filter `unknown`.
fn algorithm(doc: &Document<'_>, encrypt: &Object) -> &'static str {
    let Some(dict) = doc.resolve(encrypt).as_dict() else {
        return "unknown";
    };
    let info = EncryptionInfo::from_dictionary(&encryption_entries(doc, dict));
    match info.method {
        _ if info.v >= 5 => "aes-256",
        Some(Method::AesV3) => "aes-256",
        Some(Method::AesV2) => "aes-128",
        Some(Method::Rc4) if info.r <= 2 || info.length == 40 => "rc4-40",
        Some(Method::Rc4) => "rc4-128",
        Some(Method::Identity) | None => "unknown",
    }
}

/// The /Encrypt entries `EncryptionInfo` reads, resolved: its numbers,
/// names and booleans, and of /CF only the filters /StmF and /StrF name,
/// each with its /CFM. A bounded copy, whatever else /Encrypt holds.
fn encryption_entries(doc: &Document<'_>, encrypt: &Dictionary) -> Dictionary {
    let scalar = |value: &Object| {
        matches!(
            value,
            Object::Integer(_) | Object::Real(_) | Object::Name(_) | Object::Boolean(_)
        )
        .then(|| value.clone())
    };
    let mut entries = Dictionary::new();
    for key in [
        b"Filter".as_slice(),
        b"V",
        b"R",
        b"Length",
        b"P",
        b"EncryptMetadata",
        b"StmF",
        b"StrF",
    ] {
        if let Some(value) = scalar(doc.dict_get(encrypt, key)) {
            entries.insert(key.to_vec(), value);
        }
    }
    if let Some(filters) = doc.dict_get(encrypt, b"CF").as_dict() {
        let mut named = Dictionary::new();
        for key in [b"StmF".as_slice(), b"StrF"] {
            let Some(name) = doc.dict_get(encrypt, key).as_name() else {
                continue;
            };
            let Some(filter) = doc.dict_get(filters, name).as_dict() else {
                continue;
            };
            let mut method = Dictionary::new();
            if let Some(cfm) = scalar(doc.dict_get(filter, b"CFM")) {
                method.insert(b"CFM".to_vec(), cfm);
            }
            named.insert(name.to_vec(), Object::Dictionary(method));
        }
        entries.insert(b"CF".to_vec(), Object::Dictionary(named));
    }
    entries
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
    let fields = pagetree::stored(doc, acro, b"Fields");
    // Each object once, direct fields in a shared /Kids array included
    // (`pagetree::forest_nodes`): a 500-byte file took 2^64 steps.
    (pagetree::forest_nodes(doc, fields).into_iter())
        .filter(|field| {
            doc.dict_get(field, b"FT").as_name() == Some(b"Sig")
                && !doc.dict_get(field, b"V").is_null()
        })
        .count()
}

/// The embedded file count: distinct decoded names in the
/// /Names arrays of the /Root /Names /EmbeddedFiles tree and its /Kids.
fn attachments(doc: &Document<'_>, root: &Object) -> usize {
    let tree = pagetree::stored(doc, pagetree::stored(doc, root, b"Names"), b"EmbeddedFiles");
    let mut names = BTreeSet::new();
    // Each object once (`pagetree::tree_nodes`), as the named destinations.
    for node in pagetree::tree_nodes(doc, tree) {
        if let Some(pairs) = doc.dict_get(node, b"Names").as_array() {
            for name in pairs.iter().step_by(2) {
                let text = match doc.resolve(name) {
                    Object::String(bytes) => decode_text_string(bytes),
                    // A name that is not a string is the empty string.
                    _ => String::new(),
                };
                names.insert(text);
            }
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

/// The object holding a node's /Kids array, when /Kids names one.
fn kids_object(doc: &Document<'_>, dict: &Dictionary) -> Option<ObjectId> {
    let (holder, kids) = doc.resolve_with_holder(dict.get(b"Kids".as_slice())?);
    holder.filter(|_| kids.as_array().is_some())
}

/// A /Pages node on a page's path from the root.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TreeNode<'d> {
    pub(crate) id: Option<ObjectId>,
    pub(crate) dict: &'d Dictionary,
    /// Whether the node's /Parent chain names the path down to it (for the
    /// root: it has no /Parent dictionary), decided once as it is entered.
    pub(crate) ancestry: bool,
}

/// Whether the /Parent of `dict` names `node`: the same object, or for a
/// direct node the same dictionary.
fn parent_is(dict: &Dictionary, node: &TreeNode<'_>) -> bool {
    match (dict.get(b"Parent".as_slice()), node.id) {
        (Some(Object::Reference(id)), Some(wanted)) => *id == wanted,
        (Some(Object::Dictionary(parent)), None) => std::ptr::eq(parent, node.dict),
        _ => false,
    }
}

/// Whether the /Parent chain of `page` names the nodes of `path` (root
/// first), nearest first: then reference viewers and hayro inherit from the
/// same nodes.
/// One comparison: each node's own chain was checked as it was entered.
pub(crate) fn same_ancestry(doc: &Document<'_>, page: &Dictionary, path: &[TreeNode<'_>]) -> bool {
    match path.last() {
        Some(node) => node.ancestry && parent_is(page, node),
        None => no_parent(doc, page),
    }
}

fn no_parent(doc: &Document<'_>, dict: &Dictionary) -> bool {
    doc.resolve(dict.get(b"Parent".as_slice()).unwrap_or(&NULL))
        .as_dict()
        .is_none()
}

/// What holds the /Kids array a page written into /Kids is reached from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KidsHolder {
    /// An array object.
    Array(ObjectId),
    /// The /Kids entry of a node object.
    Node(ObjectId),
    /// The /Kids entry of the catalog's direct /Pages dictionary.
    Root,
}

/// Where a page's dictionary is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PageSlot<'p> {
    /// Its own object.
    Object(ObjectId),
    /// Entry `index` of a /Kids array: the one `holder` holds, or, through
    /// `nodes`, the /Kids of the direct node at each index in turn.
    Kid {
        holder: KidsHolder,
        nodes: &'p [usize],
        index: usize,
    },
}

/// Whether a page-tree kid is a node: /Type /Pages, or, without a /Type (or
/// with a null one), /Kids as the repair reads it
/// (`repair::untyped_kid_is_node`: /Kids, unless it lists nothing and the
/// kid has /Contents or a /MediaBox). Any other kid is a page, whatever its
/// /Type, as reference viewers and hayro read it.
pub(crate) fn is_node(doc: &Document<'_>, dict: &Dictionary) -> bool {
    match doc.dict_get(dict, b"Type") {
        Object::Null => {}
        kind => return kind.as_name() == Some(b"Pages"),
    }
    let kids = match doc.dict_get(dict, b"Kids") {
        Object::Null => None,
        Object::Array(kids) => Some(kids.len()),
        _ => Some(0),
    };
    let contents = matches!(
        dict.get(b"Contents".as_slice()),
        Some(Object::Reference(_) | Object::Array(_))
    );
    let media = !doc.dict_get(dict, b"MediaBox").is_null();
    crate::repair::untyped_kid_is_node(kids, contents, media)
}

/// One step of `walk`.
pub(crate) enum Step<'p, 'd> {
    /// A page, with the nodes from the root to its container and where it is
    /// written.
    Page(PageRef<'d>, &'p [TreeNode<'d>], PageSlot<'p>),
    /// A node below the root, entered, and where it is written (a direct
    /// node: entry `index` of the /Kids its slot reaches).
    Node(&'p TreeNode<'d>, PageSlot<'p>),
}

/// Visits the pages of the catalog's page tree in display order (hayro's
/// `resolve_pages`), each with the /Pages nodes from the root to its
/// container and where it is written. False, visiting nothing, when there
/// is no usable tree: no /Pages dictionary (or a stream) with a /Kids array.
pub(crate) fn walk_tree<'d>(
    doc: &'d Document<'_>,
    mut visit: impl FnMut(PageRef<'d>, &[TreeNode<'d>], PageSlot<'_>),
) -> bool {
    walk(doc, |step| {
        if let Step::Page(page, path, slot) = step {
            visit(page, path, slot);
        }
    })
}

/// `walk_tree`, visiting the nodes below the root as they are entered too.
pub(crate) fn walk<'d>(doc: &'d Document<'_>, mut visit: impl FnMut(Step<'_, 'd>)) -> bool {
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
    // Objects entered: nodes and pages, and /Kids array objects, so the
    // direct nodes of an array that names itself are not walked again.
    let mut visited: BTreeSet<ObjectId> = root_id.into_iter().collect();
    visited.extend(kids_object(doc, root));
    let mut path = vec![TreeNode {
        id: root_id,
        dict: root,
        ancestry: no_parent(doc, root),
    }];
    // The holder of a node's /Kids array, when it is not reached through the
    // direct nodes above it.
    let holder = |node: &TreeNode<'d>| match node.dict.get(b"Kids".as_slice()) {
        Some(Object::Reference(id)) => Some(KidsHolder::Array(*id)),
        _ => node.id.map(KidsHolder::Node),
    };
    // The index of each node below the root in its parent's /Kids; a frame's
    // array is reached from its holder through `indices[from..]`.
    let mut indices: Vec<usize> = Vec::new();
    // Each frame: its node's /Kids, their holder, where its indices start.
    let root_holder = holder(&path[0]).unwrap_or(KidsHolder::Root);
    let mut stack = vec![(root_kids.iter().enumerate(), root_holder, 0)];
    while let Some((frame, kids_holder, from)) = stack.last_mut() {
        let Some((index, kid)) = frame.next() else {
            stack.pop();
            path.pop();
            indices.pop();
            continue;
        };
        let (kids_holder, from) = (*kids_holder, *from);
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
        if is_node(doc, dict) {
            let ancestry = path
                .last()
                .is_some_and(|parent| parent.ancestry && parent_is(dict, parent));
            let node = TreeNode { id, dict, ancestry };
            let slot = match id {
                Some(id) => PageSlot::Object(id),
                None => PageSlot::Kid {
                    holder: kids_holder,
                    nodes: &indices[from..],
                    index,
                },
            };
            visit(Step::Node(&node, slot));
            if kids_object(doc, dict).is_some_and(|array| !visited.insert(array)) {
                continue;
            }
            if let Some(kids) = kids(doc, dict) {
                indices.push(index);
                let frame = match holder(&node) {
                    Some(holder) => (kids.iter().enumerate(), holder, indices.len()),
                    None => (kids.iter().enumerate(), kids_holder, from),
                };
                path.push(node);
                stack.push(frame);
            }
        } else {
            let slot = match id {
                Some(id) => PageSlot::Object(id),
                None => PageSlot::Kid {
                    holder: kids_holder,
                    nodes: &indices[from..],
                    index,
                },
            };
            visit(Step::Page(PageRef { id, dict }, &path, slot));
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

/// The page frame's numbers (spec 04, 5.1), from the page's /MediaBox,
/// /CropBox and /Rotate (inherited through /Parent) and /UserUnit, read as
/// the view reads them: `view::frame_of` decides.
pub(crate) fn page_frame(doc: &Document<'_>, page: &Dictionary) -> FrameNumbers {
    let number = |value: &Object| match doc.resolve(value) {
        Object::Integer(v) => Some(*v as f64),
        Object::Real(v) => Some(*v).filter(|v| v.is_finite()),
        _ => None,
    };
    // Rectangles are arrays of exactly four numbers; anything else is absent,
    // and the next /Parent is asked.
    let (media, crop) = frame_boxes(doc, page);
    // A /Rotate that is not a number is 0.
    let rotate = frame_entry(doc, page, b"Rotate", |value| Some(number(value))).flatten();
    let unit = page.get(b"UserUnit".as_slice()).and_then(number);
    frame_of(media, crop, rotate, unit)
}

/// The page's /MediaBox and /CropBox as the page frame reads them: arrays of
/// exactly four numbers, inherited through /Parent; anything else is absent.
pub(crate) fn frame_boxes(
    doc: &Document<'_>,
    page: &Dictionary,
) -> (Option<[f64; 4]>, Option<[f64; 4]>) {
    let number = |value: &Object| match doc.resolve(value) {
        Object::Integer(v) => Some(*v as f64),
        Object::Real(v) => Some(*v).filter(|v| v.is_finite()),
        _ => None,
    };
    let rect = |value: &Object| -> Option<[f64; 4]> {
        let items = doc.resolve(value).as_array()?;
        let [a, b, c, d] = items else {
            return None;
        };
        Some([number(a)?, number(b)?, number(c)?, number(d)?])
    };
    (
        frame_entry(doc, page, b"MediaBox", rect),
        frame_entry(doc, page, b"CropBox", rect),
    )
}

/// `key` of the page or of the nearest /Parent whose value `read` accepts,
/// as the view's page frame reads it.
fn frame_entry<'d, T>(
    doc: &'d Document<'_>,
    page: &'d Dictionary,
    key: &[u8],
    read: impl Fn(&'d Object) -> Option<T>,
) -> Option<T> {
    let mut node = page;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..256 {
        if let Some(value) = node.get(key).and_then(&read) {
            return Some(value);
        }
        node = doc.dict_get(node, b"Parent").as_dict()?;
        if !seen.insert(std::ptr::from_ref(node).addr()) {
            return None;
        }
    }
    None
}

/// The bound page size in points (CropBox within MediaBox, UserUnit,
/// /Rotate snapped to a quarter turn), as the view's `PageFrame`.
pub fn page_size(doc: &Document<'_>, page: &Dictionary) -> (f64, f64) {
    let frame = page_frame(doc, page);
    // The view keeps the sides as f32.
    (
        f64::from(frame.width as f32),
        f64::from(frame.height as f32),
    )
}

/// The inherited /Rotate the page displays with, snapped to the nearest
/// quarter turn (0, 90, 180 or 270), as the view's `PageFrame`.
pub(crate) fn display_rotation(doc: &Document<'_>, page: &Dictionary) -> i32 {
    i32::from(page_frame(doc, page).rotate)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentLimits;
    use crate::parser::{ParseLimits, parse_one};

    #[test]
    fn the_algorithm_is_what_the_security_handler_reads() {
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for body in [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [] /Count 0 >>",
            "128",
        ] {
            offsets.push(bytes.len());
            let number = offsets.len();
            bytes.extend_from_slice(format!("{number} 0 obj\n{body}\nendobj\n").as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let cf = |method: &str| {
            format!("/CF << /StdCF << /CFM /{method} >> >> /StmF /StdCF /StrF /StdCF")
        };
        for (encrypt, expected) in [
            ("<< /V 1 /R 2 >>".to_string(), "rc4-40"),
            ("<< /V 2 /R 3 /Length 128 >>".into(), "rc4-128"),
            // An indirect /Length is read through its reference.
            ("<< /V 2 /R 3 /Length 3 0 R >>".into(), "rc4-128"),
            // The handler's default key length is 40 bits (engine.ts: 128).
            ("<< /V 2 /R 3 >>".into(), "rc4-40"),
            (format!("<< /V 4 /R 4 {} >>", cf("V2")), "rc4-128"),
            (format!("<< /V 4 /R 4 /Length 40 {} >>", cf("V2")), "rc4-40"),
            (format!("<< /V 4 /R 4 {} >>", cf("AESV2")), "aes-128"),
            (format!("<< /V 4 /R 4 {} >>", cf("AESV3")), "aes-256"),
            (
                "<< /V 4 /R 4 /StmF /Identity /StrF /Identity >>".into(),
                "unknown",
            ),
            (format!("<< /V 4 /R 4 {} >>", cf("Weird")), "unknown"),
            (
                "<< /V 4 /R 4 /CF << /A << /CFM /AESV3 >> >> /StmF /A /StrF /A >>".into(),
                "aes-256",
            ),
            ("<< /V 5 /R 6 >>".into(), "aes-256"),
            ("42".into(), "unknown"),
        ] {
            let parsed = parse_one(encrypt.as_bytes(), ParseLimits::default()).unwrap();
            assert_eq!(algorithm(&doc, &parsed.object), expected, "{encrypt}");
        }
    }
}
