// SPDX-License-Identifier: AGPL-3.0-or-later
//! The page tree as an editable structure: where each page sits (its /Pages
//! node and position in that node's /Kids), and edits that keep the tree
//! valid: /Kids, /Count up the tree, /Parent links and the inheritable page
//! attributes. Also destinations: which page a destination names.
//!
//! `PageTree::read` walks /Kids exactly as `info::pages` (and so the view)
//! enumerates pages, so page indices agree everywhere. Every /Pages node must
//! be an indirect object, as ISO 32000-1 7.7.3 requires; a tree that breaks
//! that, or that has no usable root, is not edited.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::document::{Document, DocumentError};
use crate::object::{Dictionary, Object, ObjectId};

use super::edit::{Edit, Place, get, is_name};
use super::info;

/// Attributes a page inherits from its /Pages ancestors (ISO 32000-1 Table 30).
pub const INHERITABLE: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

/// Why a page tree is not edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeError {
    /// No /Pages dictionary object with a /Kids array under the catalog.
    NoRoot,
    /// A /Pages node written directly into a /Kids array.
    DirectNode,
    /// The walk disagrees with the page list (a fallback enumeration).
    Inconsistent,
}

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoRoot => "the document has no usable page tree",
            Self::DirectNode => "the page tree has a node that is not an indirect object",
            Self::Inconsistent => "the page tree does not list the document's pages",
        })
    }
}

impl std::error::Error for TreeError {}

/// Where one page sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    /// The page object; None for a page dictionary written into /Kids.
    pub id: Option<ObjectId>,
    /// The /Pages node whose /Kids holds the page.
    pub parent: ObjectId,
    /// The page's index in that /Kids array.
    pub kid: usize,
}

/// The page tree of a document, as read.
#[derive(Clone, Debug)]
pub struct PageTree {
    pub root: ObjectId,
    /// Pages in display order.
    pub slots: Vec<Slot>,
    /// Each non-root node's parent node and its index in the parent's /Kids.
    nodes: BTreeMap<ObjectId, (ObjectId, usize)>,
    /// Pages under each node.
    counts: BTreeMap<ObjectId, usize>,
}

impl PageTree {
    /// Reads the tree, as `info::pages` walks it.
    pub fn read(doc: &Document<'_>) -> Result<Self, TreeError> {
        let catalog = doc
            .resolve_id(doc.root())
            .as_dict()
            .ok_or(TreeError::NoRoot)?;
        let pages = catalog.get(b"Pages".as_slice()).ok_or(TreeError::NoRoot)?;
        let (Some(root), Object::Dictionary(dict)) = doc.resolve_with_holder(pages) else {
            return Err(TreeError::NoRoot);
        };
        if doc.object(root).is_ok_and(|object| object.stream.is_some()) {
            return Err(TreeError::NoRoot);
        }
        let kids = doc
            .dict_get(dict, b"Kids")
            .as_array()
            .ok_or(TreeError::NoRoot)?;
        let mut visited = BTreeSet::from([root]);
        let mut nodes = BTreeMap::new();
        let mut slots = Vec::new();
        let mut stack = vec![(root, kids.iter().enumerate())];
        while let Some((node, frame)) = stack.last_mut() {
            let node = *node;
            let Some((kid, value)) = frame.next() else {
                stack.pop();
                continue;
            };
            let (id, dict) = match value {
                Object::Reference(id) => {
                    if !visited.insert(*id) {
                        continue;
                    }
                    match doc.object(*id) {
                        Ok(object) if object.stream.is_none() => {
                            (Some(*id), object.value.as_dict())
                        }
                        _ => continue,
                    }
                }
                other => (None, other.as_dict()),
            };
            let Some(dict) = dict else { continue };
            if doc.dict_get(dict, b"Type").as_name() == Some(b"Pages") {
                let Some(id) = id else {
                    return Err(TreeError::DirectNode);
                };
                nodes.insert(id, (node, kid));
                if let Some(kids) = doc.dict_get(dict, b"Kids").as_array() {
                    stack.push((id, kids.iter().enumerate()));
                }
            } else {
                slots.push(Slot {
                    id,
                    parent: node,
                    kid,
                });
            }
        }
        let listed = info::pages(doc);
        if listed.len() != slots.len() || listed.iter().zip(&slots).any(|(a, b)| a.id != b.id) {
            return Err(TreeError::Inconsistent);
        }
        let mut tree = Self {
            root,
            slots,
            nodes,
            counts: BTreeMap::new(),
        };
        let mut counts = BTreeMap::new();
        for slot in &tree.slots {
            for node in tree.ancestors(slot.parent) {
                *counts.entry(node).or_insert(0usize) += 1;
            }
        }
        tree.counts = counts;
        Ok(tree)
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// `node` and the nodes above it, up to the root.
    pub fn ancestors(&self, node: ObjectId) -> Vec<ObjectId> {
        let mut chain = vec![node];
        let mut at = node;
        // Every node has one recorded parent, so the chain ends within `nodes`.
        while let Some(&(parent, _)) = self.nodes.get(&at) {
            if chain.len() > self.nodes.len() {
                break;
            }
            chain.push(parent);
            at = parent;
        }
        chain
    }

    /// Sets each node's /Count to its pages after the edit (`delta` added to
    /// the pages it had), writing only counts that change.
    fn recount<E: Edit>(
        &self,
        target: &mut E,
        deltas: &BTreeMap<ObjectId, i64>,
        skip: &BTreeSet<ObjectId>,
    ) -> Result<(), DocumentError> {
        for (&node, &delta) in deltas {
            if skip.contains(&node) {
                continue;
            }
            let pages = self.counts.get(&node).copied().unwrap_or(0) as i64;
            let count = pages.saturating_add(delta).max(0);
            let stored = target
                .document()
                .resolve_id(node)
                .as_dict()
                .and_then(|dict| dict.get(b"Count".as_slice()))
                .and_then(Object::as_i64);
            if stored != Some(count) {
                target.update(node, |value| {
                    if let Some(dict) = value.as_dict_mut() {
                        dict.insert(b"Count".to_vec(), Object::Integer(count));
                    }
                })?;
            }
        }
        Ok(())
    }

    fn add_delta(&self, deltas: &mut BTreeMap<ObjectId, i64>, node: ObjectId, delta: i64) {
        for node in self.ancestors(node) {
            *deltas.entry(node).or_insert(0) += delta;
        }
    }

    /// Removes page `index` from the tree. A /Pages node left without kids
    /// goes too (the root stays). Returns the /Kids entry that held the page.
    pub fn remove<E: Edit>(&self, target: &mut E, index: usize) -> Result<Object, DocumentError> {
        let slot = *self.slots.get(index).ok_or_else(missing_page)?;
        let mut deltas = BTreeMap::new();
        self.add_delta(&mut deltas, slot.parent, -1);
        // The page's node when the page is its only kid, then each parent
        // whose only kid that node was, bottom-up.
        let mut pruned = Vec::new();
        let mut node = slot.parent;
        while node != self.root && kids_len(target.document(), node) == 1 {
            let Some(&(parent, kid)) = self.nodes.get(&node) else {
                break;
            };
            pruned.push((node, parent, kid));
            node = parent;
        }
        let skip: BTreeSet<ObjectId> = pruned.iter().map(|&(node, _, _)| node).collect();
        self.recount(target, &deltas, &skip)?;
        let removed = remove_kid(target, slot.parent, slot.kid)?;
        for &(_, parent, kid) in &pruned {
            remove_kid(target, parent, kid)?;
        }
        Ok(removed)
    }

    /// Where `insert` puts pages before page `at`: the node and the /Kids
    /// position (before the page at `at`, after the last page at the end).
    fn insertion(&self, at: usize) -> (ObjectId, usize) {
        match (self.slots.get(at), self.slots.last()) {
            (Some(anchor), _) => (anchor.parent, anchor.kid),
            (None, Some(last)) => (last.parent, last.kid + 1),
            (None, None) => (self.root, 0),
        }
    }

    /// The /Pages node `insert` puts pages at `at` into.
    pub fn insertion_parent(&self, at: usize) -> ObjectId {
        self.insertion(at).0
    }

    /// Inserts `pages` (references to page objects, or page dictionaries)
    /// before page `at` (after the last page when `at` is the page count),
    /// setting their /Parent. Returns the node they went into.
    pub fn insert<E: Edit>(
        &self,
        target: &mut E,
        at: usize,
        pages: Vec<Object>,
    ) -> Result<ObjectId, DocumentError> {
        let (parent, position) = self.insertion(at);
        let mut deltas = BTreeMap::new();
        self.add_delta(&mut deltas, parent, pages.len() as i64);
        self.recount(target, &deltas, &BTreeSet::new())?;
        let pages = with_parent(target, pages, parent)?;
        insert_kids(target, parent, position, pages)?;
        Ok(parent)
    }

    /// Moves page `from` so that it becomes page `to`, keeping what it
    /// inherits when it changes /Pages node.
    pub fn move_page<E: Edit>(
        &self,
        target: &mut E,
        from: usize,
        to: usize,
    ) -> Result<(), DocumentError> {
        let slot = *self.slots.get(from).ok_or_else(missing_page)?;
        let rest: Vec<Slot> = (self.slots.iter())
            .enumerate()
            .filter(|&(index, _)| index != from)
            .map(|(_, slot)| *slot)
            .collect();
        let (parent, mut position) = match (rest.get(to), rest.last()) {
            (Some(anchor), _) => (anchor.parent, anchor.kid),
            (None, Some(last)) => (last.parent, last.kid + 1),
            (None, None) => (self.root, 0),
        };
        if parent == slot.parent && position > slot.kid {
            position -= 1;
        }
        let mut deltas = BTreeMap::new();
        self.add_delta(&mut deltas, slot.parent, -1);
        self.add_delta(&mut deltas, parent, 1);
        self.recount(target, &deltas, &BTreeSet::new())?;
        let mut value = remove_kid(target, slot.parent, slot.kid)?;
        if parent != slot.parent {
            value = keep_inherited(target, value, parent)?;
            value = with_parent(target, vec![value], parent)?
                .pop()
                .ok_or_else(missing_page)?;
        }
        insert_kids(target, parent, position, vec![value])
    }
}

fn missing_page() -> DocumentError {
    DocumentError {
        offset: 0,
        kind: crate::document::DocumentErrorKind::Invalid("page index"),
    }
}

/// The raw length of a node's /Kids array.
fn kids_len(doc: &Document<'_>, node: ObjectId) -> usize {
    get(doc, &Object::Reference(node), b"Kids")
        .as_array()
        .map_or(0, <[Object]>::len)
}

fn remove_kid<E: Edit>(
    target: &mut E,
    node: ObjectId,
    kid: usize,
) -> Result<Object, DocumentError> {
    let place = Place::object(node).child(target.document(), b"Kids");
    place
        .update(target, |value| {
            let kids = value.as_array_mut()?;
            (kid < kids.len()).then(|| kids.remove(kid))
        })?
        .ok_or_else(missing_page)
}

fn insert_kids<E: Edit>(
    target: &mut E,
    node: ObjectId,
    position: usize,
    pages: Vec<Object>,
) -> Result<(), DocumentError> {
    let place = Place::object(node).child(target.document(), b"Kids");
    place
        .update(target, |value| {
            if !matches!(value, Object::Array(_)) {
                *value = Object::Array(Vec::new());
            }
            let kids = value.as_array_mut()?;
            let position = position.min(kids.len());
            kids.splice(position..position, pages);
            Some(())
        })?
        .ok_or_else(missing_page)
}

/// Sets /Parent on each page: on the object a reference names, or in place
/// on a page dictionary.
fn with_parent<E: Edit>(
    target: &mut E,
    pages: Vec<Object>,
    parent: ObjectId,
) -> Result<Vec<Object>, DocumentError> {
    let mut out = Vec::with_capacity(pages.len());
    for mut page in pages {
        match &mut page {
            Object::Reference(id) => {
                target.update(*id, |value| {
                    if let Some(dict) = value.as_dict_mut() {
                        dict.insert(b"Parent".to_vec(), Object::Reference(parent));
                    }
                })?;
            }
            Object::Dictionary(dict) => {
                dict.insert(b"Parent".to_vec(), Object::Reference(parent));
            }
            _ => {}
        }
        out.push(page);
    }
    Ok(out)
}

/// What `page` (a /Kids entry) shows for each inheritable attribute is kept
/// when it moves under `parent`: an inherited value that the new ancestors
/// would not give is written onto the page.
fn keep_inherited<E: Edit>(
    target: &mut E,
    page: Object,
    parent: ObjectId,
) -> Result<Object, DocumentError> {
    let doc = target.document();
    let dict = match doc.resolve(&page).as_dict() {
        Some(dict) => dict,
        None => return Ok(page),
    };
    let mut writes = Vec::new();
    for key in INHERITABLE {
        if dict.get(key).is_some_and(|value| !value.is_null()) {
            continue;
        }
        let old = inherited(doc, dict, key).cloned();
        let new = node_inherited(doc, parent, key).cloned();
        if old != new {
            let value = match old {
                Some(value) => value,
                None => neutral(doc, dict, key),
            };
            writes.push((key.to_vec(), value));
        }
    }
    put_entries(target, page, writes)
}

/// Writes `entries` onto a page, through its object or into its dictionary.
fn put_entries<E: Edit>(
    target: &mut E,
    mut page: Object,
    entries: Vec<(Vec<u8>, Object)>,
) -> Result<Object, DocumentError> {
    if entries.is_empty() {
        return Ok(page);
    }
    match &mut page {
        Object::Reference(id) => {
            target.update(*id, |value| {
                if let Some(dict) = value.as_dict_mut() {
                    dict.extend(entries);
                }
            })?;
        }
        Object::Dictionary(dict) => dict.extend(entries),
        _ => {}
    }
    Ok(page)
}

/// Makes a page placed under `parent` not inherit what its own source did
/// not give it: for each inheritable key `page` lacks that `parent`'s chain
/// has, the value a reader assumes when the key is absent.
pub fn shield(doc: &Document<'_>, page: &mut Dictionary, parent: ObjectId) {
    for key in INHERITABLE {
        if page.get(key).is_some_and(|value| !value.is_null()) {
            continue;
        }
        if node_inherited(doc, parent, key).is_some() {
            let value = neutral(doc, page, key);
            page.insert(key.to_vec(), value);
        }
    }
}

/// The value a reader assumes for a missing inheritable attribute: no
/// rotation, no resources, a letter page, a crop box equal to the media box.
fn neutral(doc: &Document<'_>, page: &Dictionary, key: &[u8]) -> Object {
    match key {
        b"Rotate" => Object::Integer(0),
        b"Resources" => Object::Dictionary(Dictionary::new()),
        b"CropBox" => inherited(doc, page, b"MediaBox")
            .cloned()
            .unwrap_or_else(letter),
        _ => letter(),
    }
}

fn letter() -> Object {
    Object::Array(vec![
        Object::Integer(0),
        Object::Integer(0),
        Object::Integer(612),
        Object::Integer(792),
    ])
}

/// MuPDF's `pdf_dict_get_inheritable`: the page's own non-null `key`, else
/// the nearest /Parent's, unresolved (references stay references).
pub fn inherited<'d>(
    doc: &'d Document<'_>,
    page: &'d Dictionary,
    key: &[u8],
) -> Option<&'d Object> {
    let mut node = page;
    for _ in 0..64 {
        if let Some(value) = node.get(key).filter(|value| !doc.resolve(value).is_null()) {
            return Some(value);
        }
        node = doc.dict_get(node, b"Parent").as_dict()?;
    }
    None
}

fn node_inherited<'d>(doc: &'d Document<'_>, node: ObjectId, key: &[u8]) -> Option<&'d Object> {
    inherited(doc, doc.resolve_id(node).as_dict()?, key)
}

/// Page objects in page order, each to its first index.
#[derive(Clone, Debug, Default)]
pub struct PageIndex {
    ids: BTreeMap<ObjectId, usize>,
    count: usize,
}

impl PageIndex {
    pub fn of(doc: &Document<'_>) -> Self {
        let pages = info::pages(doc);
        let mut ids = BTreeMap::new();
        for (index, page) in pages.iter().enumerate() {
            if let Some(id) = page.id {
                ids.entry(id).or_insert(index);
            }
        }
        Self {
            ids,
            count: pages.len(),
        }
    }

    pub fn get(&self, id: ObjectId) -> Option<usize> {
        self.ids.get(&id).copied()
    }

    pub fn count(&self) -> usize {
        self.count
    }
}

/// The named destinations: /Root /Dests (PDF 1.1) and the /Root /Names
/// /Dests name tree, first definition of a name winning.
#[derive(Debug, Default)]
pub struct NamedDests<'d> {
    dests: Option<&'d Dictionary>,
    tree: BTreeMap<&'d [u8], &'d Object>,
}

impl<'d> NamedDests<'d> {
    pub fn load(doc: &'d Document<'_>) -> Self {
        let root = doc.resolve_id(doc.root());
        let dests = get(doc, root, b"Dests").as_dict();
        let tree = get(doc, get(doc, root, b"Names"), b"Dests");
        Self {
            dests,
            tree: name_tree(doc, tree),
        }
    }

    /// MuPDF's `pdf_lookup_dest`: the old dictionary when there is one,
    /// otherwise the name tree.
    pub fn get(&self, doc: &'d Document<'_>, name: &[u8]) -> Option<&'d Object> {
        match self.dests {
            Some(dests) => Some(doc.dict_get(dests, name)).filter(|value| !value.is_null()),
            None => self.tree.get(name).copied(),
        }
    }
}

/// Every key and value of a name tree's leaves; the first of equal keys wins.
pub fn name_tree<'d>(doc: &'d Document<'_>, root: &'d Object) -> BTreeMap<&'d [u8], &'d Object> {
    let mut entries = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut stack = vec![(root, 0usize)];
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
            for pair in pairs.chunks_exact(2) {
                if let [key, value] = pair {
                    if let Some(key) = doc.resolve(key).as_string() {
                        entries.entry(key).or_insert(value);
                    }
                }
            }
        }
        if depth < 64 {
            if let Some(kids) = doc.dict_get(dict, b"Kids").as_array() {
                stack.extend(kids.iter().rev().map(|kid| (kid, depth + 1)));
            }
        }
    }
    entries
}

/// The destination an annotation or outline item goes to: its /Dest, else
/// the /D of a /GoTo action.
pub fn local_dest<'d>(doc: &'d Document<'_>, item: &'d Dictionary) -> Option<&'d Object> {
    let dest = doc.dict_get(item, b"Dest");
    if !dest.is_null() {
        return Some(dest);
    }
    let action = doc.dict_get(item, b"A");
    if is_name(doc, get(doc, action, b"S"), b"GoTo") {
        let dest = get(doc, action, b"D");
        return (!dest.is_null()).then_some(dest);
    }
    None
}

/// The page a destination names: an explicit `[page ...]` with a page
/// reference or a 0-based page number, or a named destination (a name or a
/// string) that leads to one.
pub fn dest_page(
    doc: &Document<'_>,
    pages: &PageIndex,
    named: &NamedDests<'_>,
    dest: &Object,
) -> Option<usize> {
    let mut dest = doc.resolve(dest);
    for _ in 0..4 {
        match dest {
            Object::Array(items) => {
                return match items.first()? {
                    Object::Reference(id) => pages.get(*id),
                    Object::Integer(page) => usize::try_from(*page)
                        .ok()
                        .filter(|&page| page < pages.count()),
                    _ => None,
                };
            }
            Object::Name(name) | Object::String(name) => dest = doc.resolve(named.get(doc, name)?),
            Object::Dictionary(dict) => dest = doc.dict_get(dict, b"D"),
            _ => return None,
        }
    }
    None
}

/// Replaces every reference to `target` with null in every object outside
/// `skip`, so a removed page's objects stop being reachable (a save writes
/// only what is reachable).
pub fn null_references<E: Edit>(
    edit: &mut E,
    target: ObjectId,
    skip: &BTreeSet<ObjectId>,
) -> Result<(), DocumentError> {
    let holders: Vec<ObjectId> = (edit.document().objects())
        .filter(|object| !skip.contains(&object.id) && refers_to(&object.value, target))
        .map(|object| object.id)
        .collect();
    for id in holders {
        edit.update(id, |value| replace_reference(value, target))?;
    }
    Ok(())
}

fn refers_to(value: &Object, target: ObjectId) -> bool {
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            Object::Reference(id) if *id == target => return true,
            Object::Array(items) => stack.extend(items),
            Object::Dictionary(dict) => stack.extend(dict.values()),
            _ => {}
        }
    }
    false
}

fn replace_reference(value: &mut Object, target: ObjectId) {
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            Object::Reference(id) if *id == target => *value = Object::Null,
            Object::Array(items) => stack.extend(items.iter_mut()),
            Object::Dictionary(dict) => stack.extend(dict.values_mut()),
            _ => {}
        }
    }
}
