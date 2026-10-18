// SPDX-License-Identifier: AGPL-3.0-or-later
//! Copies pages, with their annotations and form fields, from one document
//! into another (engine.ts `graftPages`): objects are renumbered, and an
//! object several copied pages share is copied once.
//!
//! A reference to a page that is not copied (a link destination, an
//! annotation's /P, a field's widget on another page) never drags that page
//! in. Such a reference becomes null; a link whose destination is a page
//! left behind is not copied; named
//! destinations become explicit ones. Each copy of a page listed twice gets
//! its own annotation and field objects; everything they and the page use
//! (contents, resources, appearance streams) is shared. What the repeats
//! copy again is bounded by `MAX_REPEATED_BYTES`.
use std::collections::{BTreeMap, BTreeSet};

use crate::app::edit::{Edit, Place, get, is_name};
use crate::app::info::{self, PageRef};
use crate::app::pagetree::{self, NamedDests, PageIndex};
use crate::document::{Document, DocumentError, DocumentErrorKind};
use crate::object::{Dictionary, Object, ObjectId, decode_text_string};

/// Page keys not copied: the tree link, what is rebuilt, and what only the
/// source's other structures give meaning (article beads, the structure
/// tree's parent key, page actions).
const DROPPED_PAGE_KEYS: [&[u8]; 6] =
    [b"Type", b"Parent", b"Annots", b"B", b"StructParents", b"AA"];

/// A field tree deeper than this is not followed.
const FIELD_DEPTH: usize = 32;

/// What the second and later copies of a repeated page may copy again (its
/// dictionary, annotations and fields), measured by `weight`.
pub const MAX_REPEATED_BYTES: usize = crate::app::MAX_PDF_BYTES;

/// The `LimitExceeded` reason of a graft over `MAX_REPEATED_BYTES`.
pub const REPEATED: &str = "repeated pages";

/// What a graft made in the target.
#[derive(Debug, Default)]
pub struct Grafted {
    /// The new page objects, in order, with /Parent not yet set.
    pub pages: Vec<ObjectId>,
    /// Copied widget annotations, page by page in /Annots order.
    pub widgets: Vec<ObjectId>,
    /// Source /AcroForm /DR entries the target's /DR lacks, by category,
    /// already copied: a whole category, or names within one.
    pub resources: Vec<(Vec<u8>, Resources)>,
    /// The source's /AcroForm /DA, for a target that has none.
    pub appearance: Option<Object>,
}

#[derive(Debug)]
pub enum Resources {
    Category(Object),
    Names(Vec<(Vec<u8>, Object)>),
}

/// The source side of a graft.
struct Source<'s> {
    doc: &'s Document<'s>,
    pages: Vec<PageRef<'s>>,
    index: PageIndex,
    named: NamedDests<'s>,
    /// Annotation objects, each to the first page listing it.
    annots: BTreeMap<ObjectId, usize>,
    /// Every node of the /AcroForm field tree.
    fields: BTreeSet<ObjectId>,
    /// Document-level objects never copied (catalog, outline, name trees...).
    excluded: BTreeSet<ObjectId>,
}

impl<'s> Source<'s> {
    fn new(doc: &'s Document<'s>) -> Self {
        let pages = info::pages(doc);
        let mut annots = BTreeMap::new();
        for (index, page) in pages.iter().enumerate() {
            for annot in annots_of(doc, page.dict) {
                if let Object::Reference(id) = annot {
                    annots.entry(*id).or_insert(index);
                }
            }
        }
        let root = doc.resolve_id(doc.root());
        let mut excluded = BTreeSet::from([doc.root()]);
        if let Some(dict) = root.as_dict() {
            for key in [
                b"AcroForm".as_slice(),
                b"Outlines",
                b"Names",
                b"Dests",
                b"StructTreeRoot",
                b"PageLabels",
                b"Pages",
            ] {
                if let Some(id) = dict
                    .get(key)
                    .and_then(|value| doc.resolve_with_holder(value).0)
                {
                    excluded.insert(id);
                }
            }
        }
        if let Some(id) = doc.trailer_entry(b"Info").and_then(Object::as_reference) {
            excluded.insert(id);
        }
        Self {
            index: PageIndex::of(doc),
            named: NamedDests::load(doc),
            fields: field_nodes(doc),
            annots,
            excluded,
            pages,
            doc,
        }
    }

    fn acroform(&self) -> Option<&'s Dictionary> {
        let root = self.doc.resolve_id(self.doc.root());
        get(self.doc, root, b"AcroForm").as_dict()
    }
}

/// The /Annots entries of a page, resolved to its array.
fn annots_of<'d>(doc: &'d Document<'_>, page: &'d Dictionary) -> &'d [Object] {
    doc.dict_get(page, b"Annots").as_array().unwrap_or(&[])
}

/// Every node of the /AcroForm /Fields tree, through /Kids.
fn field_nodes(doc: &Document<'_>) -> BTreeSet<ObjectId> {
    let root = doc.resolve_id(doc.root());
    let fields = get(doc, get(doc, root, b"AcroForm"), b"Fields");
    let mut nodes = BTreeSet::new();
    let mut stack: Vec<(&Object, usize)> = (fields.as_array().unwrap_or(&[]).iter())
        .map(|field| (field, 0))
        .collect();
    while let Some((node, depth)) = stack.pop() {
        if let Object::Reference(id) = node {
            if !nodes.insert(*id) {
                continue;
            }
        }
        if depth < FIELD_DEPTH {
            let kids = get(doc, node, b"Kids").as_array().unwrap_or(&[]);
            stack.extend(kids.iter().map(|kid| (kid, depth + 1)));
        }
    }
    nodes
}

struct Copier<'s, 'a, E: Edit> {
    target: &'a mut E,
    source: &'a Source<'s>,
    /// Source page index of each selected page.
    selected: BTreeSet<usize>,
    /// The new page of each (source page, copy number).
    new_pages: BTreeMap<(usize, usize), ObjectId>,
    /// Field nodes above a copied widget.
    wanted: BTreeSet<ObjectId>,
    /// Source annotations left out (links to pages left behind).
    skipped: BTreeSet<ObjectId>,
    /// Source to target object numbers, one map per copy number; only
    /// annotations and fields use the maps past the first.
    maps: Vec<BTreeMap<ObjectId, ObjectId>>,
    pending: Vec<(ObjectId, ObjectId, usize)>,
    /// What copies past the first have copied, by `weight`.
    repeated: usize,
}

/// Copies the source pages `selection` (indices, in order, repeats allowed)
/// into `target` as new page objects, not yet in its page tree. `parent` is
/// the /Pages node they will go under: a page does not inherit from it what
/// its source did not give it.
pub fn graft<E: Edit>(
    target: &mut E,
    source: &Document<'_>,
    selection: &[usize],
    parent: ObjectId,
) -> Result<Grafted, DocumentError> {
    let source = Source::new(source);
    let mut listed: BTreeMap<usize, usize> = BTreeMap::new();
    for &page in selection {
        *listed.entry(page).or_insert(0) += 1;
    }
    let copies = listed.values().copied().max().unwrap_or(1);
    let mut copier = Copier {
        target,
        source: &source,
        selected: selection.iter().copied().collect(),
        new_pages: BTreeMap::new(),
        wanted: BTreeSet::new(),
        skipped: BTreeSet::new(),
        maps: vec![BTreeMap::new(); copies.max(1)],
        pending: Vec::new(),
        repeated: 0,
    };
    let mut occurrences = Vec::with_capacity(selection.len());
    let mut seen: BTreeMap<usize, usize> = BTreeMap::new();
    for &page in selection {
        let copy = seen.entry(page).or_insert(0);
        occurrences.push((page, *copy));
        let id = copier.target.allocate(Object::Null)?;
        copier.new_pages.insert((page, *copy), id);
        *copy += 1;
    }
    copier.plan();
    let mut grafted = Grafted::default();
    for &(page, copy) in &occurrences {
        let id = copier.page(page, copy, parent, &mut grafted.widgets)?;
        grafted.pages.push(id);
    }
    if !grafted.widgets.is_empty() {
        copier.form(&mut grafted)?;
    }
    copier.drain()?;
    Ok(grafted)
}

impl<'s, E: Edit> Copier<'s, '_, E> {
    /// Marks the links that are left out and the field nodes that are kept.
    fn plan(&mut self) {
        let source = self.source;
        let doc = source.doc;
        let mut skipped = BTreeSet::new();
        let mut wanted = BTreeSet::new();
        for &page in &self.selected {
            let Some(page) = source.pages.get(page) else {
                continue;
            };
            for annot in annots_of(doc, page.dict) {
                let Some(dict) = doc.resolve(annot).as_dict() else {
                    continue;
                };
                let subtype = doc.dict_get(dict, b"Subtype");
                if subtype.as_name() == Some(b"Link")
                    && pagetree::local_dest(doc, dict).is_some()
                    && self.rewrite_dest(dict, b"Dest").is_none()
                    && self.rewrite_action(dict).is_none()
                {
                    skipped.extend(annot.as_reference());
                    continue;
                }
                if subtype.as_name() != Some(b"Widget") {
                    continue;
                }
                let mut node = dict;
                for _ in 0..FIELD_DEPTH {
                    let (Some(parent), Some(next)) = (
                        node.get(b"Parent".as_slice())
                            .and_then(Object::as_reference),
                        doc.dict_get(node, b"Parent").as_dict(),
                    ) else {
                        break;
                    };
                    if !wanted.insert(parent) {
                        break;
                    }
                    node = next;
                }
            }
        }
        self.skipped = skipped;
        self.wanted = wanted;
    }

    /// The new page for a source page in copy `copy` (its first copy when
    /// it has fewer).
    fn new_page(&self, page: usize, copy: usize) -> Option<ObjectId> {
        (self.new_pages.get(&(page, copy)))
            .or_else(|| self.new_pages.get(&(page, 0)))
            .copied()
    }

    /// Whether each copy of a page gets its own copy of the object: an
    /// annotation names its page, and a field lists its widgets.
    fn per_copy(&self, id: ObjectId) -> bool {
        self.source.annots.contains_key(&id)
            || self.source.fields.contains(&id)
            || self.wanted.contains(&id)
    }

    /// Counts what a copy past the first copies again.
    fn charge(&mut self, copy: usize, weight: impl FnOnce() -> usize) -> Result<(), DocumentError> {
        if copy == 0 {
            return Ok(());
        }
        self.repeated = self.repeated.saturating_add(weight());
        if self.repeated > MAX_REPEATED_BYTES {
            return Err(DocumentError {
                offset: 0,
                kind: DocumentErrorKind::LimitExceeded(REPEATED),
            });
        }
        Ok(())
    }

    fn is_page_node(&self, id: ObjectId) -> bool {
        let value = self.source.doc.resolve_id(id);
        let kind = get(self.source.doc, value, b"Type");
        is_name(self.source.doc, kind, b"Page") || is_name(self.source.doc, kind, b"Pages")
    }

    /// The target value for a source reference in copy `copy`.
    fn reference(&mut self, id: ObjectId, copy: usize) -> Result<Object, DocumentError> {
        if let Some(page) = self.source.index.get(id) {
            return Ok(match self.selected.contains(&page) {
                true => self
                    .new_page(page, copy)
                    .map_or(Object::Null, Object::Reference),
                false => Object::Null,
            });
        }
        let annot_elsewhere =
            (self.source.annots.get(&id)).is_some_and(|page| !self.selected.contains(page));
        let unwanted_field = self.source.fields.contains(&id)
            && !self.wanted.contains(&id)
            && !self.source.annots.contains_key(&id);
        if self.source.excluded.contains(&id)
            || self.skipped.contains(&id)
            || annot_elsewhere
            || unwanted_field
            || self.is_page_node(id)
            || self.source.doc.object(id).is_err()
        {
            return Ok(Object::Null);
        }
        let copy = match self.per_copy(id) {
            true => copy.min(self.maps.len().saturating_sub(1)),
            false => 0,
        };
        if let Some(&mapped) = self.maps.get(copy).and_then(|map| map.get(&id)) {
            return Ok(Object::Reference(mapped));
        }
        let mapped = self.target.allocate(Object::Null)?;
        if let Some(map) = self.maps.get_mut(copy) {
            map.insert(id, mapped);
        }
        self.pending.push((id, mapped, copy));
        Ok(Object::Reference(mapped))
    }

    /// A source value with its references mapped.
    fn value(
        &mut self,
        value: &Object,
        copy: usize,
        depth: usize,
    ) -> Result<Object, DocumentError> {
        if depth > 256 {
            return Ok(Object::Null);
        }
        Ok(match value {
            Object::Reference(id) => self.reference(*id, copy)?,
            Object::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.value(item, copy, depth + 1)?);
                }
                Object::Array(out)
            }
            Object::Dictionary(dict) => Object::Dictionary(self.dictionary(dict, copy, depth)?),
            other => other.clone(),
        })
    }

    fn dictionary(
        &mut self,
        dict: &Dictionary,
        copy: usize,
        depth: usize,
    ) -> Result<Dictionary, DocumentError> {
        let mut out = Dictionary::new();
        for (key, value) in dict {
            let value = self.value(value, copy, depth + 1)?;
            if !value.is_null() {
                out.insert(key.clone(), value);
            }
        }
        Ok(out)
    }

    /// An explicit destination in source terms for a destination whose
    /// page is copied; None when it names a page left behind or none.
    fn explicit(&self, dest: &Object) -> Option<Object> {
        let doc = self.source.doc;
        let mut dest = doc.resolve(dest);
        for _ in 0..4 {
            match dest {
                Object::Array(items) => {
                    let page = match items.first()? {
                        Object::Reference(id) => self.source.index.get(*id)?,
                        Object::Integer(page) => usize::try_from(*page).ok()?,
                        _ => return None,
                    };
                    if !self.selected.contains(&page) {
                        return None;
                    }
                    let id = self.source.pages.get(page)?.id?;
                    let mut items = items.to_vec();
                    if let Some(first) = items.first_mut() {
                        *first = Object::Reference(id);
                    }
                    return Some(Object::Array(items));
                }
                Object::Name(name) | Object::String(name) => {
                    dest = doc.resolve(self.source.named.get(doc, name)?);
                }
                Object::Dictionary(dict) => dest = doc.dict_get(dict, b"D"),
                _ => return None,
            }
        }
        None
    }

    /// `dict[key]` as an explicit destination to a copied page.
    fn rewrite_dest(&self, dict: &Dictionary, key: &[u8]) -> Option<Object> {
        let dest = self.source.doc.dict_get(dict, key);
        (!dest.is_null()).then(|| self.explicit(dest)).flatten()
    }

    /// A /GoTo action whose destination is a copied page, made explicit.
    fn rewrite_action(&self, dict: &Dictionary) -> Option<Object> {
        let doc = self.source.doc;
        let action = doc.dict_get(dict, b"A").as_dict()?;
        if !is_name(doc, doc.dict_get(action, b"S"), b"GoTo") {
            return None;
        }
        let dest = self.rewrite_dest(action, b"D")?;
        let mut action = action.clone();
        action.insert(b"D".to_vec(), dest);
        Some(Object::Dictionary(action))
    }

    /// The source object's value as it is copied: an annotation points at
    /// its new page and keeps only destinations to copied pages; a field
    /// keeps only kids that are copied; a signature loses its value (it
    /// signed the source, not this document).
    fn adapted(&self, id: ObjectId, mut dict: Dictionary) -> (Dictionary, Option<usize>) {
        let doc = self.source.doc;
        let mut page = None;
        if let Some(&on) = self.source.annots.get(&id) {
            page = Some(on);
            dict.remove(b"StructParent".as_slice());
            if dict.contains_key(b"Dest".as_slice()) {
                match self.rewrite_dest(&dict, b"Dest") {
                    Some(dest) => {
                        dict.insert(b"Dest".to_vec(), dest);
                    }
                    None => {
                        dict.remove(b"Dest".as_slice());
                    }
                }
            }
            let goto = is_name(doc, get(doc, doc.dict_get(&dict, b"A"), b"S"), b"GoTo");
            if goto {
                match self.rewrite_action(&dict) {
                    Some(action) => {
                        dict.insert(b"A".to_vec(), action);
                    }
                    None => {
                        dict.remove(b"A".as_slice());
                    }
                }
            }
        }
        if self.wanted.contains(&id) || self.source.fields.contains(&id) {
            if let Some(kids) = doc.dict_get(&dict, b"Kids").as_array() {
                let kept: Vec<Object> = (kids.iter())
                    .filter(|kid| match kid {
                        Object::Reference(kid) => {
                            self.wanted.contains(kid)
                                || (self.source.annots.get(kid))
                                    .is_some_and(|page| self.selected.contains(page))
                        }
                        _ => true,
                    })
                    .cloned()
                    .collect();
                dict.insert(b"Kids".to_vec(), Object::Array(kept));
            }
        }
        if is_name(doc, doc.dict_get(&dict, b"FT"), b"Sig") {
            dict.remove(b"V".as_slice());
        }
        (dict, page)
    }

    /// Copies everything mapped so far, and what that reaches.
    fn drain(&mut self) -> Result<(), DocumentError> {
        while let Some((id, mapped, copy)) = self.pending.pop() {
            let doc = self.source.doc;
            let object = doc.object(id)?;
            let raw = match object.stream {
                Some(_) => doc.stream_data(id)?,
                None => None,
            };
            let size = raw.map_or(0, <[u8]>::len);
            self.charge(copy, || weight(&object.value).saturating_add(size))?;
            let raw = raw.map(<[u8]>::to_vec);
            let value = match &object.value {
                Object::Dictionary(dict) => {
                    let (dict, page) = self.adapted(id, dict.clone());
                    let mut dict = self.dictionary(&dict, copy, 0)?;
                    if let Some(page) = page.and_then(|page| self.new_page(page, copy)) {
                        dict.insert(b"P".to_vec(), Object::Reference(page));
                    }
                    Object::Dictionary(dict)
                }
                other => self.value(other, copy, 0)?,
            };
            match (raw, value) {
                (Some(raw), Object::Dictionary(dict)) => {
                    self.target.put_stream_raw(mapped, dict, raw)?
                }
                (_, value) => self.target.put_value(mapped, value)?,
            }
        }
        Ok(())
    }

    /// Builds the new page for source page `page`, copy `copy`.
    fn page(
        &mut self,
        page: usize,
        copy: usize,
        parent: ObjectId,
        widgets: &mut Vec<ObjectId>,
    ) -> Result<ObjectId, DocumentError> {
        let all = self.source;
        let doc = all.doc;
        let (Some(id), Some(source)) = (self.new_page(page, copy), all.pages.get(page)) else {
            return Err(DocumentError {
                offset: 0,
                kind: DocumentErrorKind::Invalid("graft page"),
            });
        };
        self.charge(copy, || dict_weight(source.dict))?;
        let mut dict = Dictionary::new();
        dict.insert(b"Type".to_vec(), Object::Name(b"Page".to_vec()));
        for (key, value) in source.dict {
            if DROPPED_PAGE_KEYS.contains(&key.as_slice()) {
                continue;
            }
            let value = self.value(value, 0, 0)?;
            if !value.is_null() {
                dict.insert(key.clone(), value);
            }
        }
        for key in pagetree::INHERITABLE {
            if dict.contains_key(key) {
                continue;
            }
            if let Some(value) = pagetree::inherited(doc, source.dict, key) {
                let value = self.value(value, 0, 0)?;
                if !value.is_null() {
                    dict.insert(key.to_vec(), value);
                }
            }
        }
        let mut annots = Vec::new();
        for annot in annots_of(doc, source.dict) {
            let widget = is_name(doc, get(doc, annot, b"Subtype"), b"Widget");
            let value = match annot {
                Object::Reference(source) if self.skipped.contains(source) => continue,
                Object::Reference(_) => self.value(annot, copy, 0)?,
                // A direct annotation (not allowed, but seen) is copied as it is.
                other => self.value(other, copy, 0)?,
            };
            if let Object::Reference(new) = value {
                if widget {
                    widgets.push(new);
                }
            }
            if !value.is_null() {
                annots.push(value);
            }
        }
        if !annots.is_empty() {
            dict.insert(b"Annots".to_vec(), Object::Array(annots));
        }
        pagetree::shield(self.target.document(), &mut dict, parent);
        self.target.put_value(id, Object::Dictionary(dict))?;
        Ok(id)
    }

    /// The source form's /DR entries the target lacks, and its /DA when the
    /// target has none, so copied widgets find the fonts they name.
    fn form(&mut self, grafted: &mut Grafted) -> Result<(), DocumentError> {
        let Some(acro) = self.source.acroform() else {
            return Ok(());
        };
        let doc = self.source.doc;
        let target_acro = {
            let target = self.target.document();
            let root = target.resolve_id(target.root());
            get(target, root, b"AcroForm").clone()
        };
        let target_doc = self.target.document();
        let has = |category: &[u8], name: Option<&[u8]>| -> bool {
            let dr = get(target_doc, &target_acro, b"DR");
            let entry = get(target_doc, dr, category);
            match name {
                None => !entry.is_null(),
                Some(name) => !get(target_doc, entry, name).is_null(),
            }
        };
        let mut wanted = Vec::new();
        if let Some(dr) = doc.dict_get(acro, b"DR").as_dict() {
            for (category, entries) in dr {
                match doc.resolve(entries).as_dict() {
                    Some(entries) if has(category, None) => {
                        let names: Vec<(Vec<u8>, Object)> = (entries.iter())
                            .filter(|(name, _)| !has(category, Some(name)))
                            .map(|(name, value)| (name.clone(), value.clone()))
                            .collect();
                        if !names.is_empty() {
                            wanted.push((category.clone(), None, names));
                        }
                    }
                    _ if !has(category, None) => {
                        wanted.push((category.clone(), Some(entries.clone()), Vec::new()));
                    }
                    _ => {}
                }
            }
        }
        let appearance = doc.dict_get(acro, b"DA");
        let target_has_da = !get(target_doc, &target_acro, b"DA").is_null();
        if !appearance.is_null() && !target_has_da {
            grafted.appearance = Some(appearance.clone());
        }
        for (category, whole, names) in wanted {
            let resources = match whole {
                Some(value) => Resources::Category(self.value(&value, 0, 0)?),
                None => {
                    let mut copied = Vec::with_capacity(names.len());
                    for (name, value) in names {
                        let value = self.value(&value, 0, 0)?;
                        if !value.is_null() {
                            copied.push((name, value));
                        }
                    }
                    Resources::Names(copied)
                }
            };
            grafted.resources.push((category, resources));
        }
        Ok(())
    }
}

/// engine.ts `registerGraftedFields`: each copied widget's top-level field
/// goes into the target's /AcroForm /Fields (created when missing), renamed
/// `<name> (2)`, `(3)`... when its name is taken; the copied /DR entries and
/// /DA join the target's form.
pub fn register_fields<E: Edit>(target: &mut E, grafted: Grafted) -> Result<(), DocumentError> {
    let mut tops = Vec::new();
    let mut seen = BTreeSet::new();
    for &widget in &grafted.widgets {
        let doc = target.document();
        let mut top = Some(widget);
        let mut visited = BTreeSet::from([widget]);
        while let Some(node) = top {
            if visited.len() > FIELD_DEPTH {
                break;
            }
            let parent =
                (doc.resolve_id(node).as_dict()).and_then(|dict| dict.get(b"Parent".as_slice()));
            match parent.map(|parent| (parent, doc.resolve(parent))) {
                Some((Object::Reference(parent), Object::Dictionary(_))) => {
                    if !visited.insert(*parent) {
                        break;
                    }
                    top = Some(*parent);
                }
                // A field written directly into its kid is not registered (engine.ts).
                Some((_, Object::Dictionary(_))) => top = None,
                _ => break,
            }
        }
        if let Some(top) = top {
            if seen.insert(top) {
                tops.push(top);
            }
        }
    }
    if tops.is_empty() {
        return Ok(());
    }
    let doc = target.document();
    let acro = Place::object(doc.root()).child(doc, b"AcroForm");
    acro.update(target, |value| {
        if !matches!(value, Object::Dictionary(_)) {
            *value = Object::Dictionary(Dictionary::new());
        }
    })?;
    let doc = target.document();
    let fields = acro.child(doc, b"Fields");
    let mut taken: BTreeSet<String> = (fields.read(doc).as_array().unwrap_or(&[]).iter())
        .map(|field| field_name(doc, field))
        .filter(|name| !name.is_empty())
        .collect();
    let mut renames = Vec::new();
    for &top in &tops {
        let name = field_name(target.document(), &Object::Reference(top));
        if name.is_empty() {
            continue;
        }
        let mut unique = name.clone();
        if taken.contains(&unique) {
            let bound = taken.len() + 2;
            if let Some(free) = (2..=bound)
                .map(|suffix| format!("{name} ({suffix})"))
                .find(|candidate| !taken.contains(candidate))
            {
                unique = free;
            }
        }
        if unique != name {
            renames.push((top, unique.clone()));
        }
        taken.insert(unique);
    }
    for (top, name) in renames {
        target.update(top, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"T".to_vec(), Object::text(&name));
            }
        })?;
    }
    fields.update(target, |value| {
        if !matches!(value, Object::Array(_)) {
            *value = Object::Array(Vec::new());
        }
        if let Some(items) = value.as_array_mut() {
            items.extend(tops.iter().map(|&top| Object::Reference(top)));
        }
    })?;
    if let Some(appearance) = grafted.appearance {
        acro.update(target, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"DA".to_vec(), appearance);
            }
        })?;
    }
    if grafted.resources.is_empty() {
        return Ok(());
    }
    let doc = target.document();
    let dr = acro.child(doc, b"DR");
    dr.update(target, |value| {
        if !matches!(value, Object::Dictionary(_)) {
            *value = Object::Dictionary(Dictionary::new());
        }
    })?;
    for (category, resources) in grafted.resources {
        match resources {
            Resources::Category(value) => {
                dr.update(target, |dict| {
                    if let Some(dict) = dict.as_dict_mut() {
                        dict.insert(category, value);
                    }
                })?;
            }
            Resources::Names(names) => {
                let doc = target.document();
                let place = dr.child(doc, &category);
                place.update(target, |dict| {
                    if let Some(dict) = dict.as_dict_mut() {
                        dict.extend(names);
                    }
                })?;
            }
        }
    }
    Ok(())
}

/// About how many bytes a value takes written out: strings, names and keys
/// by length, anything else a few bytes.
fn weight(value: &Object) -> usize {
    let mut total = 0usize;
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        let size = match value {
            Object::Name(bytes) | Object::String(bytes) => bytes.len().saturating_add(2),
            Object::Array(items) => {
                stack.extend(items);
                2
            }
            Object::Dictionary(dict) => {
                for (key, item) in dict {
                    total = total.saturating_add(key.len().saturating_add(2));
                    stack.push(item);
                }
                4
            }
            _ => 8,
        };
        total = total.saturating_add(size);
    }
    total
}

fn dict_weight(dict: &Dictionary) -> usize {
    (dict.iter()).fold(4, |total, (key, value)| {
        total
            .saturating_add(key.len().saturating_add(2))
            .saturating_add(weight(value))
    })
}

/// A field's /T as text, "" when it has none.
fn field_name(doc: &Document<'_>, field: &Object) -> String {
    match get(doc, field, b"T") {
        Object::String(bytes) => decode_text_string(bytes),
        _ => String::new(),
    }
}

/// engine.ts `fieldNames`: the top-level /AcroForm /Fields names, in order.
pub fn field_names(doc: &Document<'_>) -> Vec<String> {
    let root = doc.resolve_id(doc.root());
    let fields = get(doc, get(doc, root, b"AcroForm"), b"Fields");
    (fields.as_array().unwrap_or(&[]).iter())
        .map(|field| field_name(doc, field))
        .collect()
}
