// SPDX-License-Identifier: AGPL-3.0-or-later
//! The page tree a save writes. The app shows the pages `info::pages` lists
//! (the view's walk of /Kids, or the /Type /Page objects when there is no
//! usable tree); reference viewers read the tree from the catalog by /Count
//! and /Kids.
//! When the two readings could differ (a repaired or fallback page list: a
//! cycle, a page listed twice, a wrong /Count, a page as the root, no tree,
//! a direct node or page, a /Parent that is not the node), the save writes
//! a copy whose tree is one /Pages node listing exactly the app's pages,
//! each with what it inherited written onto it (a direct /Resources
//! dictionary once, as a new object the pages share). An intact tree is
//! written as it is. The app and reference viewers read a kid's kind alike
//! but for an untyped node with /Kids and a /MediaBox (to a viewer, a page
//! hiding its kids) and an untyped page with empty /Kids and /Contents (to a
//! viewer, a node); such a tree is rebuilt, its nodes typed, so viewers read
//! the app's pages.
//!
//! This runs on the document being written, never on the session's: the
//! live document, its journal (undo and redo) and incremental saves, which
//! keep a signed prefix, are unaffected.
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use crate::document::{Document, DocumentError, DocumentErrorKind};
use crate::object::{Dictionary, Object, ObjectId};

use super::document::{LowNumbers, MAX_OBJECT_NUMBER};
use super::info::{self, PageRef};
use super::pagetree::{INHERITABLE, inherited};

/// Deeper trees are rebuilt rather than checked.
const MAX_DEPTH: usize = 256;

/// `doc` when a reference viewer reads its page tree as the app's page
/// list, otherwise a copy with a tree that gives exactly that list.
pub fn for_save<'d>(
    doc: &'d Document<'static>,
) -> Result<Cow<'d, Document<'static>>, DocumentError> {
    let pages = info::pages(doc);
    if intact(doc, &pages) {
        return Ok(Cow::Borrowed(doc));
    }
    rebuilt(doc, &pages).map(Cow::Owned)
}

fn kids<'d>(doc: &'d Document<'_>, dict: &'d Dictionary) -> Option<&'d [Object]> {
    doc.dict_get(dict, b"Kids").as_array()
}

/// A reference viewer's reading of a page-tree kid: a node when typed
/// /Pages, or untyped with /Kids and no /MediaBox.
fn viewer_node(doc: &Document<'_>, dict: &Dictionary) -> bool {
    match doc.dict_get(dict, b"Type") {
        Object::Null => {
            !doc.dict_get(dict, b"Kids").is_null() && doc.dict_get(dict, b"MediaBox").is_null()
        }
        kind => kind.as_name() == Some(b"Pages"),
    }
}

/// The non-stream dictionary object `id`.
fn node_dict<'d>(doc: &'d Document<'_>, id: ObjectId) -> Option<&'d Dictionary> {
    let object = doc.object(id).ok()?;
    object.stream.is_none().then_some(())?;
    object.value.as_dict()
}

/// Whether reading the tree as reference viewers do (the root's /Count,
/// then down /Kids by /Count) lists the pages `pages` holds, in order, with
/// every /Parent naming its node.
fn intact(doc: &Document<'_>, pages: &[PageRef<'_>]) -> bool {
    let Some(catalog) = doc.resolve_id(doc.root()).as_dict() else {
        return false;
    };
    let Some(Object::Reference(root)) = catalog.get(b"Pages".as_slice()) else {
        return false;
    };
    let Some(root_dict) = node_dict(doc, *root) else {
        return false;
    };
    let Some(root_kids) = kids(doc, root_dict) else {
        return false;
    };
    if doc.dict_get(root_dict, b"Type").as_name() != Some(b"Pages")
        || doc.dict_get(root_dict, b"Parent").as_dict().is_some()
    {
        return false;
    }
    let mut listed: Vec<Option<ObjectId>> = Vec::with_capacity(pages.len());
    let mut visited = BTreeSet::from([*root]);
    // Each frame: the node, its /Kids, the next kid, the pages listed before it.
    let mut stack = vec![(*root, root_dict, root_kids, 0usize, 0usize)];
    while let Some(frame) = stack.last_mut() {
        let (node, node_dict_value, node_kids, next, start) = *frame;
        if next == node_kids.len() {
            stack.pop();
            let count = match doc.dict_get(node_dict_value, b"Count") {
                Object::Integer(count) => usize::try_from(*count).ok(),
                _ => None,
            };
            if count != Some(listed.len() - start) {
                return false;
            }
            continue;
        }
        frame.3 += 1;
        // A page written directly into /Kids reads alike (viewers draw it);
        // a direct node gives its pages no /Parent to inherit through.
        let (id, dict) = match &node_kids[next] {
            Object::Reference(id) => match node_dict(doc, *id) {
                Some(dict) if visited.insert(*id) => (Some(*id), dict),
                _ => return false,
            },
            Object::Dictionary(dict) => (None, dict),
            _ => return false,
        };
        if stack.len() > MAX_DEPTH {
            return false;
        }
        if dict.get(b"Parent".as_slice()) != Some(&Object::Reference(node)) {
            return false;
        }
        // A viewer: a /Type other than /Pages is a page; untyped, /Kids
        // without /MediaBox is a node. The app: `info::is_node` (/Type /Pages, or
        // untyped with /Kids, as the repair reads it).
        let app_node = info::is_node(doc, dict);
        match (app_node, viewer_node(doc, dict), kids(doc, dict), id) {
            (true, true, Some(kids), Some(id)) => stack.push((id, dict, kids, 0, listed.len())),
            (false, false, _, id) => listed.push(id),
            _ => return false,
        }
    }
    listed.len() == pages.len() && listed.iter().zip(pages).all(|(id, page)| page.id == *id)
}

/// Numbers for the objects a rebuilt tree adds: past every number an object
/// holds or a reference names (a dangling reference must not come to name a
/// new node), or while that would pass `MAX_OBJECT_NUMBER`, the lowest
/// numbers none of them uses.
enum Numbers {
    Past(u32),
    Low(LowNumbers),
}

impl Numbers {
    fn new(doc: &Document<'_>) -> Self {
        let mut last = doc.root().number();
        let mut values: Vec<&Object> = Vec::new();
        for object in doc.objects() {
            last = last.max(object.id.number());
            values.push(&object.value);
        }
        values.extend(doc.effective_trailer_entries().map(|(_, value)| value));
        // References anywhere inside those values, without recursion.
        while let Some(value) = values.pop() {
            match value {
                Object::Reference(id) => last = last.max(id.number()),
                Object::Array(items) => values.extend(items),
                Object::Dictionary(entries) => values.extend(entries.values()),
                _ => {}
            }
        }
        match last.checked_add(1) {
            Some(past) if past <= MAX_OBJECT_NUMBER => Self::Past(past),
            _ => Self::Low(LowNumbers::new(doc, None)),
        }
    }

    fn fresh(&mut self) -> Result<ObjectId, DocumentError> {
        let number = match self {
            Self::Past(next) => {
                let number = *next;
                *next = next.saturating_add(1);
                Some(number).filter(|&number| number <= MAX_OBJECT_NUMBER)
            }
            Self::Low(low) => low.allocate(),
        };
        number
            .and_then(|number| ObjectId::new(number, 0).ok())
            .ok_or(DocumentError {
                offset: 0,
                kind: DocumentErrorKind::LimitExceeded("object numbers"),
            })
    }
}

/// A copy of `doc` whose page tree is one node listing `pages`.
fn rebuilt(
    doc: &Document<'static>,
    pages: &[PageRef<'_>],
) -> Result<Document<'static>, DocumentError> {
    let mut copy = doc.clone();
    let mut numbers = Numbers::new(doc);
    let listed: BTreeSet<ObjectId> = pages.iter().filter_map(|page| page.id).collect();
    // The catalog's /Pages object is reused only when it is a page-tree node
    // (typed /Pages, or untyped with /Kids), not the catalog, a trailer
    // entry such as /Info, or a page: anything else keeps its contents.
    let catalog = doc.resolve_id(doc.root()).as_dict();
    let reusable = |id: ObjectId| {
        let Some(dict) = node_dict(doc, id) else {
            return false;
        };
        info::is_node(doc, dict)
            && id != doc.root()
            && !listed.contains(&id)
            && !(doc.effective_trailer_entries()).any(|(_, value)| *value == Object::Reference(id))
    };
    let root = match catalog.and_then(|catalog| catalog.get(b"Pages".as_slice())) {
        Some(Object::Reference(id)) if reusable(*id) => *id,
        _ => numbers.fresh()?,
    };
    // A direct dictionary inherited from a node becomes one shared object,
    // keyed by its address in `doc`.
    let mut shared: BTreeMap<usize, ObjectId> = BTreeMap::new();
    let mut kids = Vec::with_capacity(pages.len());
    for page in pages {
        let mut dict = page.dict.clone();
        for key in INHERITABLE {
            if page
                .dict
                .get(key)
                .is_some_and(|value| !doc.resolve(value).is_null())
            {
                continue;
            }
            let Some(value) = inherited(doc, page.dict, key) else {
                continue;
            };
            let value = match value {
                Object::Dictionary(_) => {
                    let address = std::ptr::from_ref(value) as usize;
                    let id = match shared.get(&address) {
                        Some(&id) => id,
                        None => {
                            let id = numbers.fresh()?;
                            copy.put_value(id, value.clone())?;
                            shared.insert(address, id);
                            id
                        }
                    };
                    Object::Reference(id)
                }
                value => value.clone(),
            };
            dict.insert(key.to_vec(), value);
        }
        // A catalog listed as a page stays typed a catalog: viewers and the
        // app read any /Type but /Pages as a page.
        if page.id != Some(doc.root()) {
            dict.insert(b"Type".to_vec(), Object::Name(b"Page".to_vec()));
        }
        dict.insert(b"Parent".to_vec(), Object::Reference(root));
        let id = match page.id {
            Some(id) => id,
            None => numbers.fresh()?,
        };
        copy.put_value(id, Object::Dictionary(dict))?;
        kids.push(Object::Reference(id));
    }
    let count = i64::try_from(kids.len()).unwrap_or(i64::MAX);
    let mut node = Dictionary::new();
    node.insert(b"Type".to_vec(), Object::Name(b"Pages".to_vec()));
    node.insert(b"Kids".to_vec(), Object::Array(kids));
    node.insert(b"Count".to_vec(), Object::Integer(count));
    copy.put_value(root, Object::Dictionary(node))?;
    copy.update_value(doc.root(), |catalog| {
        if let Object::Dictionary(catalog) = catalog {
            catalog.insert(b"Pages".to_vec(), Object::Reference(root));
        }
    })?;
    Ok(copy)
}
