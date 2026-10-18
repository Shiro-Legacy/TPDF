// SPDX-License-Identifier: AGPL-3.0-or-later
//! The document outline (bookmarks), and the edits outline-add, outline-set and outline-delete make.
//!
//! The walk follows /First and /Next from the /Outlines root in preorder;
//! an item already walked (a cycle, or an item shared by two parents) is
//! not walked again, so every item has one place and one preorder id.
//! Each item object is walked once, and at most `MAX_DIRECT_ITEMS` items
//! written into others, so a walk is linear in the document.
//! Edits keep /First, /Last, /Next, /Prev and /Parent as the walk sees the
//! tree, and rewrite /Count on the edited item's ancestors: the visible
//! descendants of an open item, negated for a closed one (ISO 32000-1 Table
//! 153), the root's omitted when nothing is visible.
use std::collections::{BTreeMap, BTreeSet};

use crate::app::edit::{Edit, Place};
use crate::document::{Document, DocumentError, DocumentErrorKind};
use crate::object::{Dictionary, Object, ObjectId};

/// One outline item where the walk found it.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Where the item dictionary is: its own object, or a dictionary
    /// written into another item.
    pub place: Place,
    /// The parent item; None at the top level.
    pub parent: Option<usize>,
    /// 0 at the top level.
    pub depth: usize,
    pub prev: Option<usize>,
    pub next: Option<usize>,
    pub first: Option<usize>,
    pub last: Option<usize>,
}

/// Items written directly into another item (ISO 32000 wants each its own
/// object) are walked up to this many;
/// their chains stop at the next. Each costs its path of keys.
pub const MAX_DIRECT_ITEMS: usize = 1024;

/// The outline as walked: items in preorder.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outline {
    /// The /Outlines dictionary, when the catalog has one.
    pub root: Option<Place>,
    pub items: Vec<Item>,
    /// The first and last top-level items.
    pub first: Option<usize>,
    pub last: Option<usize>,
    /// Stored links the walk did not follow because they name an item it
    /// had walked already.
    pub repeats: Vec<Repeat>,
    /// The walk stopped at its limit with items left.
    pub truncated: bool,
}

/// An item's /First or /Next naming an item walked before it.
#[derive(Clone, Debug, PartialEq)]
pub struct Repeat {
    pub from: usize,
    pub key: &'static [u8],
    pub to: usize,
}

/// The catalog's own object.
fn catalog(doc: &Document<'_>) -> Place {
    let root = doc.root();
    Place::object(doc.resolve_id_with_holder(root).0.unwrap_or(root))
}

fn invalid(what: &'static str) -> DocumentError {
    DocumentError {
        offset: 0,
        kind: DocumentErrorKind::Invalid(what),
    }
}

/// Whether `dict` has a non-null `key`.
fn has(doc: &Document<'_>, dict: &Dictionary, key: &[u8]) -> bool {
    !doc.dict_get(dict, key).is_null()
}

/// An /Outlines entry that names something other than an outline root:
/// the catalog, the page tree's root, or a dictionary typed as another
/// thing. Its /First, /Last and /Count are not an outline's, and an edit
/// writing them would damage it, so it counts as no outline.
fn foreign(doc: &Document<'_>, catalog: &Place, root: &Place, dict: &Dictionary) -> bool {
    let typed = (doc.dict_get(dict, b"Type").as_name()).is_some_and(|name| name != b"Outlines");
    let pages = (catalog.read(doc).as_dict())
        .and_then(|catalog| catalog.get(b"Pages".as_slice()))
        .and_then(|pages| doc.resolve_with_holder(pages).0);
    let object = root.keys.is_empty().then_some(root.id);
    typed || object.is_some_and(|id| id == catalog.id || Some(id) == pages)
}

impl Outline {
    pub fn read(doc: &Document<'_>) -> Self {
        Self::read_at_most(doc, usize::MAX)
    }

    /// The first `limit` items in preorder, `truncated` when there are more.
    pub fn read_at_most(doc: &Document<'_>, limit: usize) -> Self {
        let catalog = catalog(doc);
        let root = catalog.child(doc, b"Outlines");
        let Some(root_dict) = root.read(doc).as_dict() else {
            return Self::default();
        };
        if foreign(doc, &catalog, &root, root_dict) {
            return Self::default();
        }
        let mut outline = Self {
            root: Some(root.clone()),
            ..Self::default()
        };
        // Every object walked, with its item (None for the catalog and root).
        let mut seen: BTreeMap<ObjectId, Option<usize>> = BTreeMap::from([(catalog.id, None)]);
        if root.keys.is_empty() {
            seen.insert(root.id, None);
        }
        let mut direct = 0usize;
        // Each frame is the next item of a chain: its parent, depth, the
        // item before it, and where it is.
        let mut stack = Vec::new();
        if has(doc, root_dict, b"First") {
            stack.push((None, 0usize, None, root.child(doc, b"First")));
        }
        while let Some((parent, depth, prev, place)) = stack.pop() {
            let Some(dict) = place.read(doc).as_dict() else {
                continue;
            };
            if place.keys.is_empty() {
                if let Some(&walked) = seen.get(&place.id) {
                    let from = match prev {
                        Some(prev) => Some((prev, b"Next".as_slice())),
                        None => parent.map(|parent| (parent, b"First".as_slice())),
                    };
                    if let (Some(to), Some((from, key))) = (walked, from) {
                        outline.repeats.push(Repeat { from, key, to });
                    }
                    continue;
                }
            } else if direct >= MAX_DIRECT_ITEMS {
                continue;
            } else {
                direct += 1;
            }
            let index = outline.items.len();
            if index >= limit {
                outline.truncated = true;
                break;
            }
            if place.keys.is_empty() {
                seen.insert(place.id, Some(index));
            }
            outline.items.push(Item {
                place: place.clone(),
                parent,
                depth,
                prev,
                next: None,
                first: None,
                last: None,
            });
            match prev {
                Some(prev) => {
                    if let Some(item) = outline.items.get_mut(prev) {
                        item.next = Some(index);
                    }
                }
                None => match parent.and_then(|parent| outline.items.get_mut(parent)) {
                    Some(parent) => parent.first = Some(index),
                    None => outline.first = Some(index),
                },
            }
            match parent.and_then(|parent| outline.items.get_mut(parent)) {
                Some(parent) => parent.last = Some(index),
                None => outline.last = Some(index),
            }
            // The next sibling waits until the children are walked.
            if has(doc, dict, b"Next") {
                stack.push((parent, depth, Some(index), place.child(doc, b"Next")));
            }
            if has(doc, dict, b"First") {
                stack.push((Some(index), depth + 1, None, place.child(doc, b"First")));
            }
        }
        outline
    }

    /// The items under `index`: the preorder range after it.
    fn subtree(&self, index: usize) -> std::ops::Range<usize> {
        let depth = self.items.get(index).map_or(0, |item| item.depth);
        let end = (index.saturating_add(1)..self.items.len())
            .find(|&at| self.items.get(at).is_some_and(|item| item.depth <= depth))
            .unwrap_or(self.items.len());
        index..end
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The item's dictionary.
    pub fn dict<'d>(&self, doc: &'d Document<'_>, index: usize) -> Option<&'d Dictionary> {
        self.items.get(index)?.place.read(doc).as_dict()
    }

    fn children(&self, parent: Option<usize>) -> Vec<usize> {
        let mut children = Vec::new();
        let mut at = match parent {
            Some(parent) => self.items.get(parent).and_then(|item| item.first),
            None => self.first,
        };
        while let Some(index) = at {
            children.push(index);
            at = self.items.get(index).and_then(|item| item.next);
        }
        children
    }

    /// An item with children is open when its /Count is positive.
    fn open(&self, doc: &Document<'_>, index: usize) -> bool {
        let has_children = self
            .items
            .get(index)
            .is_some_and(|item| item.first.is_some());
        has_children
            && self
                .dict(doc, index)
                .is_some_and(|dict| count(doc, dict) > 0)
    }

    /// The descendants an open item shows (all of the root's that its open
    /// items show), for every item; the root's last.
    fn visible(&self, doc: &Document<'_>) -> (Vec<i64>, i64) {
        let mut shown = vec![0i64; self.items.len()];
        let total = |shown: &[i64], children: Vec<usize>| -> i64 {
            children
                .into_iter()
                .map(|child| {
                    let below = if self.open(doc, child) {
                        shown.get(child).copied().unwrap_or(0)
                    } else {
                        0
                    };
                    below.saturating_add(1)
                })
                .fold(0i64, i64::saturating_add)
        };
        // Children come after their parent in preorder.
        for index in (0..self.items.len()).rev() {
            let value = total(&shown, self.children(Some(index)));
            if let Some(slot) = shown.get_mut(index) {
                *slot = value;
            }
        }
        let root = total(&shown, self.children(None));
        (shown, root)
    }
}

/// /Count as an integer: a real rounds half up.
fn count(doc: &Document<'_>, dict: &Dictionary) -> i64 {
    doc.dict_get(dict, b"Count")
        .as_f64()
        .filter(|count| count.is_finite())
        .map_or(0, |count| (count + 0.5).floor() as i64)
}

/// Sets `key` of the dictionary at `place`, or removes it with None; a
/// value already there is not written again.
fn put<E: Edit>(
    target: &mut E,
    place: &Place,
    key: &[u8],
    value: Option<Object>,
) -> Result<(), DocumentError> {
    let current = place
        .read(target.document())
        .as_dict()
        .and_then(|dict| dict.get(key));
    if current == value.as_ref() {
        return Ok(());
    }
    place.update(target, |dict| {
        if let Some(dict) = dict.as_dict_mut() {
            match value {
                Some(value) => {
                    dict.insert(key.to_vec(), value);
                }
                None => {
                    dict.remove(key);
                }
            }
        }
    })
}

/// Makes the dictionary written at `place` its own object, leaving a
/// reference where it was.
fn own_object<E: Edit>(target: &mut E, place: &Place) -> Result<ObjectId, DocumentError> {
    let Some((key, keys)) = place.keys.split_last() else {
        return Ok(place.id);
    };
    let value = place.read(target.document()).clone();
    if value.as_dict().is_none() {
        return Err(invalid("outline item"));
    }
    let id = target.allocate(value)?;
    let holder = Place {
        id: place.id,
        keys: keys.to_vec(),
    };
    holder.update(target, |dict| {
        if let Some(dict) = dict.as_dict_mut() {
            dict.insert(key.clone(), Object::Reference(id));
        }
    })?;
    Ok(id)
}

/// Gives the items `wanted` picks (preorder indices) and the root their own
/// objects, outermost first, so references can name them. ISO 32000 wants
/// every item indirect, but items written into others are listed too.
fn objects_for<E: Edit>(
    target: &mut E,
    wanted: impl Fn(&Outline) -> Vec<usize>,
    root: bool,
) -> Result<Outline, DocumentError> {
    // Each pass gives one more of a bounded set its object.
    for _ in 0..8 {
        let outline = Outline::read(target.document());
        if root {
            if let Some(place) = outline.root.as_ref().filter(|place| !place.keys.is_empty()) {
                own_object(target, place)?;
                continue;
            }
        }
        let mut picked = wanted(&outline);
        picked.sort_unstable();
        let direct = picked
            .into_iter()
            .filter_map(|index| outline.items.get(index))
            .find(|item| !item.place.keys.is_empty());
        match direct {
            Some(item) => {
                own_object(target, &item.place.clone())?;
            }
            None => return Ok(outline),
        }
    }
    Ok(Outline::read(target.document()))
}

/// The reference naming an item's own object.
fn reference(outline: &Outline, index: usize) -> Option<Object> {
    let item = outline.items.get(index)?;
    item.place
        .keys
        .is_empty()
        .then_some(Object::Reference(item.place.id))
}

/// Rewrites /Count on the items at `starts` and every item above them, and
/// on the root, from the tree as it now is.
fn recount<E: Edit>(target: &mut E, starts: &[Place]) -> Result<(), DocumentError> {
    let doc = target.document();
    let outline = Outline::read(doc);
    let (shown, total) = outline.visible(doc);
    let index: BTreeMap<(ObjectId, &[Vec<u8>]), usize> = (outline.items.iter().enumerate())
        .map(|(at, item)| ((item.place.id, item.place.keys.as_slice()), at))
        .collect();
    let mut done = BTreeSet::new();
    let mut writes = Vec::new();
    for start in starts {
        let mut at = index.get(&(start.id, start.keys.as_slice())).copied();
        while let Some(at_index) = at {
            let Some(item) = outline
                .items
                .get(at_index)
                .filter(|_| done.insert(at_index))
            else {
                break;
            };
            let value = match (item.first, shown.get(at_index)) {
                (Some(_), Some(&shown)) if outline.open(doc, at_index) => {
                    Some(Object::Integer(shown))
                }
                (Some(_), Some(&shown)) => Some(Object::Integer(shown.saturating_neg())),
                _ => None,
            };
            writes.push((item.place.clone(), value));
            at = item.parent;
        }
    }
    if let Some(root) = &outline.root {
        writes.push((root.clone(), (total > 0).then_some(Object::Integer(total))));
    }
    for (place, value) in writes {
        put(target, &place, b"Count", value)?;
    }
    Ok(())
}

/// Cuts the stored /First and /Next entries the walk skipped
/// (`Outline::repeats`: a cycle, an item shared by two parents) held by
/// items outside `gone`, so an edited outline is the tree the walk lists
/// and a deleted item cannot come back through one: a skipped /First leaves
/// its item without children, a skipped /Next ends its chain (the parent's
/// /Last names it). Returns the items whose /Count may change.
fn cut_repeats<E: Edit>(
    target: &mut E,
    outline: &Outline,
    gone: std::ops::Range<usize>,
) -> Result<Vec<Place>, DocumentError> {
    let mut starts = Vec::new();
    for repeat in (outline.repeats.iter()).filter(|repeat| !gone.contains(&repeat.from)) {
        let Some(from) = outline.items.get(repeat.from) else {
            continue;
        };
        put(target, &from.place, repeat.key, None)?;
        if repeat.key == b"First" {
            put(target, &from.place, b"Last", None)?;
            starts.push(from.place.clone());
            continue;
        }
        let holder = match from.parent {
            Some(parent) => outline.items.get(parent).map(|parent| parent.place.clone()),
            None => outline.root.clone(),
        };
        if let (Some(holder), Some(reference)) = (holder, reference(outline, repeat.from)) {
            put(target, &holder, b"Last", Some(reference))?;
        }
        if let Some(parent) = from.parent.and_then(|parent| outline.items.get(parent)) {
            starts.push(parent.place.clone());
        }
    }
    Ok(starts)
}

/// outline-add: a top-level item after the last one, going to `page`
/// (`dest`, an explicit destination). An outline is made when there is none.
pub fn append<E: Edit>(
    target: &mut E,
    title: &str,
    dest: Object,
) -> Result<ObjectId, DocumentError> {
    let outline = objects_for(target, |outline| outline.last.into_iter().collect(), true)?;
    let starts = cut_repeats(target, &outline, 0..0)?;
    let root = match &outline.root {
        Some(root) if root.keys.is_empty() => root.id,
        _ => {
            let mut dict = Dictionary::new();
            dict.insert(b"Type".to_vec(), Object::name("Outlines"));
            let root = target.allocate(Object::Dictionary(dict))?;
            let catalog = catalog(target.document());
            put(target, &catalog, b"Outlines", Some(Object::Reference(root)))?;
            root
        }
    };
    let last = outline.last.and_then(|last| reference(&outline, last));
    let mut item = Dictionary::new();
    item.insert(b"Parent".to_vec(), Object::Reference(root));
    item.insert(b"Title".to_vec(), Object::text(title));
    item.insert(b"Dest".to_vec(), dest);
    if let Some(last) = &last {
        item.insert(b"Prev".to_vec(), last.clone());
    }
    let id = target.allocate(Object::Dictionary(item))?;
    let new = Some(Object::Reference(id));
    match outline.last.and_then(|last| outline.items.get(last)) {
        Some(last) => put(target, &last.place.clone(), b"Next", new.clone())?,
        None => put(target, &Place::object(root), b"First", new.clone())?,
    }
    put(target, &Place::object(root), b"Last", new)?;
    recount(target, &starts)?;
    Ok(id)
}

/// outline-set: only the title changes.
pub fn rename<E: Edit>(target: &mut E, index: usize, title: &str) -> Result<(), DocumentError> {
    let outline = Outline::read(target.document());
    let item = outline
        .items
        .get(index)
        .ok_or_else(|| invalid("outline item"))?;
    item.place.update(target, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.insert(b"Title".to_vec(), Object::text(title));
        }
    })
}

/// outline-delete: the item and everything under it leave the outline; its
/// neighbours are linked to each other and its parent's /First, /Last and
/// the counts above it follow.
pub fn delete<E: Edit>(target: &mut E, index: usize) -> Result<(), DocumentError> {
    let outline = objects_for(
        target,
        |outline| {
            let item = outline.items.get(index);
            let last = match item.and_then(|item| item.parent) {
                Some(parent) => outline.items.get(parent).and_then(|parent| parent.last),
                None => outline.last,
            };
            let near = [
                item.and_then(|item| item.prev),
                item.and_then(|item| item.next),
            ];
            near.into_iter()
                .chain([last])
                .flatten()
                .filter(|&at| at != index)
                .collect()
        },
        false,
    )?;
    let item = outline
        .items
        .get(index)
        .cloned()
        .ok_or_else(|| invalid("outline item"))?;
    let parent = match item.parent {
        Some(parent) => outline.items.get(parent).map(|parent| parent.place.clone()),
        None => outline.root.clone(),
    }
    .ok_or_else(|| invalid("outline item"))?;
    let prev = item
        .prev
        .and_then(|prev| outline.items.get(prev))
        .map(|prev| prev.place.clone());
    let next = item
        .next
        .and_then(|next| outline.items.get(next))
        .map(|next| next.place.clone());
    let prev_ref = item.prev.and_then(|prev| reference(&outline, prev));
    let next_ref = item.next.and_then(|next| reference(&outline, next));
    // The last of the siblings left, which /Last must name.
    let last = match item.next {
        None => item.prev,
        Some(_) => match item.parent {
            Some(parent) => outline.items.get(parent).and_then(|parent| parent.last),
            None => outline.last,
        },
    };
    let last_ref = last.and_then(|last| reference(&outline, last));
    match &prev {
        Some(prev) => put(target, prev, b"Next", next_ref.clone())?,
        None => put(target, &parent, b"First", next_ref.clone())?,
    }
    if let Some(next) = &next {
        put(target, next, b"Prev", prev_ref)?;
    }
    put(target, &parent, b"Last", last_ref)?;
    let mut starts = cut_repeats(target, &outline, outline.subtree(index))?;
    starts.extend(item.parent.and(Some(parent)));
    recount(target, &starts)
}
