// SPDX-License-Identifier: MIT
//! Page organisation and assembly: page-rotate, page-delete, page-move,
//! insert-pages, extract-pages, pdf-from-images, labels, labels-set and
//! labels-clear (engine.ts).
//!
//! The step names (`rotate page`, `delete page`, `move page`, `insert
//! pages`, `set page labels`, `clear page labels`) are part of the wire
//! contract: the renderer refreshes its page list on the first four.
pub mod graft;
pub mod images;

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::app::edit::{Builder, Edit, Place, get};
use crate::app::labels;
use crate::app::pagetree::{self, NamedDests, PageIndex, PageTree, Slot};
use crate::app::{
    AppDocument, AppError, AppSession, Buffers, MAX_PDF_BYTES, MAX_SAVE_BYTES, info, overlay,
    reply, request,
};
use crate::document::{Document, DocumentError, DocumentErrorKind};
use crate::object::{Dictionary, Object, ObjectId};
use crate::serialize::SerializeError;
use crate::writer::{DocumentWriteError, WriteOptions, write_document};

/// engine.ts `MAX_GRAFT_PAGES`.
pub const MAX_GRAFT_PAGES: usize = 2000;
/// engine.ts `MAX_IMAGES`.
pub const MAX_IMAGES: usize = 50;
/// engine.ts `MAX_IMAGE_BYTES`.
pub const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;
/// engine.ts `MAX_IMAGES_TOTAL_BYTES`.
pub const MAX_IMAGES_TOTAL_BYTES: usize = 120 * 1024 * 1024;
/// engine.ts `MAX_LABEL_PREFIX_CHARS`, in UTF-16 code units.
pub const MAX_LABEL_PREFIX_CHARS: usize = 32;

/// The largest label start labels-set writes: /St is an integer readers
/// take as a 32-bit int.
pub const MAX_LABEL_START: i64 = i32::MAX as i64;

fn plain(message: impl Into<String>) -> AppError {
    AppError::plain(message)
}

fn tagged(reason: &str, message: impl Into<String>) -> AppError {
    AppError::reason(reason, message)
}

fn tree(doc: &Document<'_>, op: &str) -> Result<PageTree, AppError> {
    PageTree::read(doc).map_err(|error| plain(format!("{op}: {error}")))
}

/// The page tree of a document being edited, normalized first in the same
/// step (`pagetree::normalize`), with /Count fixed when that changed it.
pub(crate) fn editable_tree<E: Edit>(target: &mut E, op: &str) -> Result<PageTree, AppError> {
    let normalized = pagetree::normalize(target).map_err(|error| {
        plain(format!(
            "{op}: the page tree could not be repaired ({error})"
        ))
    })?;
    let tree = tree(target.document(), op)?;
    if normalized {
        tree.fix_counts(target)?;
    }
    Ok(tree)
}

fn edit_reply(session: &AppSession, request: &Value) -> Value {
    reply(request, Value::Object(session.edit_state()))
}

/// engine.ts `page-rotate`: /Rotate on the page itself (shadowing what it
/// inherits) is its rotation plus `by`, in 0..360.
pub fn rotate(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let doc = session.require_mutable("page-rotate")?;
    let page = session.check_page(request.get("page"), "page-rotate")?;
    let by = request.get("by").and_then(request::as_js_number);
    let by = match by {
        Some(by) if by == 90.0 || by == -90.0 => by as i64,
        _ => return Err(plain("page-rotate: by must be 90 or -90")),
    };
    let current = doc.page(page).map_or(0, |(_, dict)| {
        crate::app::info::page_rotation(doc.document(), dict)
    });
    let next = (current + by + 360) % 360;
    session.mutate("rotate page", |session| {
        let mut tx = session.tx()?;
        let tree = editable_tree(&mut tx, "page-rotate")?;
        let slot = *tree
            .slots
            .get(page)
            .ok_or_else(|| plain("page-rotate: no such page"))?;
        update_page(&mut tx, slot, |dict| {
            dict.insert(b"Rotate".to_vec(), Object::Integer(next));
        })?;
        Ok(())
    })?;
    Ok(edit_reply(session, request))
}

/// Edits a page dictionary, in its object or in its parent's /Kids.
fn update_page<E: Edit>(
    target: &mut E,
    slot: Slot,
    edit: impl FnOnce(&mut crate::object::Dictionary),
) -> Result<(), DocumentError> {
    match slot.id {
        Some(id) => target.update(id, |value| {
            if let Some(dict) = value.as_dict_mut() {
                edit(dict);
            }
        }),
        None => {
            let kids = Place::object(slot.parent).child(target.document(), b"Kids");
            kids.update(target, |value| {
                let page = value.as_array_mut().and_then(|kids| kids.get_mut(slot.kid));
                if let Some(dict) = page.and_then(Object::as_dict_mut) {
                    edit(dict);
                }
            })
        }
    }
}

/// engine.ts `page-delete`, with its repairs in the same step: links on
/// other pages to the page go, outline items to it lose their destination,
/// fields lose the page's widgets (and go when that was their last), page
/// labels after it move up, any other reference to the page becomes null,
/// and references to its own annotations (and the fields that went) go, so
/// nothing keeps the removed page in a saved file.
pub fn delete(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let doc = session.require_mutable("page-delete")?;
    let page = session.check_page(request.get("page"), "page-delete")?;
    if doc.page_count() < 2 {
        return Err(plain(
            "page-delete: a one-page document has nothing to delete",
        ));
    }
    session.mutate("delete page", |session| {
        let mut tx = session.tx()?;
        delete_page(&mut tx, page)
    })?;
    Ok(edit_reply(session, request))
}

fn delete_page<E: Edit>(target: &mut E, index: usize) -> Result<(), AppError> {
    let tree = editable_tree(target, "page-delete")?;
    let doc = target.document();
    let slot = *tree
        .slots
        .get(index)
        .ok_or_else(|| plain("page-delete: no such page"))?;
    let (links, outline, own, widgets) = {
        let listed = info::pages(doc);
        let pages = PageIndex::of(doc);
        let named = NamedDests::load(doc);
        let goes_here = |item: &Object| {
            doc.resolve(item)
                .as_dict()
                .and_then(|dict| pagetree::local_dest(doc, dict))
                .and_then(|dest| pagetree::dest_page(doc, &pages, &named, dest))
                == Some(index)
        };
        let mut links: Vec<(Place, Vec<usize>)> = Vec::new();
        let mut elsewhere = BTreeSet::new();
        for (other, page) in listed.iter().enumerate() {
            if other == index {
                continue;
            }
            let annots = annots_of(doc, page.dict);
            elsewhere.extend(annots.iter().filter_map(Object::as_reference));
            let Some(id) = page.id else {
                continue;
            };
            let place = Place::object(id).child(doc, b"Annots");
            // Pages sharing one /Annots array edit it once.
            if links.iter().any(|(done, _)| *done == place) {
                continue;
            }
            let doomed: Vec<usize> = (annots.iter().enumerate())
                .filter(|(_, annot)| {
                    let link = get(doc, annot, b"Subtype").as_name() == Some(b"Link");
                    link && goes_here(annot)
                })
                .map(|(position, _)| position)
                .collect();
            if !doomed.is_empty() {
                links.push((place, doomed));
            }
        }
        let outline: Vec<ObjectId> = outline_items(doc)
            .into_iter()
            .filter(|&item| goes_here(&Object::Reference(item)))
            .collect();
        // The page's annotations no other page lists go with it.
        let own: BTreeSet<ObjectId> = (listed.get(index))
            .map(|page| annots_of(doc, page.dict))
            .unwrap_or_default()
            .iter()
            .filter_map(Object::as_reference)
            .filter(|id| !elsewhere.contains(id))
            .collect();
        let widgets: Vec<ObjectId> = (own.iter().copied())
            .filter(|&id| get(doc, &Object::Reference(id), b"Subtype").as_name() == Some(b"Widget"))
            .collect();
        (links, outline, own, widgets)
    };
    let remaining = i64::try_from(tree.len().saturating_sub(1)).unwrap_or(i64::MAX);
    let at = i64::try_from(index).unwrap_or(i64::MAX);

    for (place, doomed) in links {
        place.update(target, |value| {
            if let Some(annots) = value.as_array_mut() {
                for &position in doomed.iter().rev() {
                    if position < annots.len() {
                        annots.remove(position);
                    }
                }
            }
        })?;
    }
    for item in outline {
        target.update(item, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.remove(b"Dest".as_slice());
                dict.remove(b"A".as_slice());
            }
        })?;
    }
    let fields = remove_widgets(target, &widgets)?;
    tree.remove(target, index)?;
    labels::adjust(target, |entries| labels::deleted(entries, at, remaining))?;
    // The page's own annotations name it (/P) but go with it; anything else
    // naming the page loses it, and anything naming those annotations (a
    // structure OBJR, /AcroForm /CO, another page's /IRT or /Popup) or the
    // fields that went loses them.
    let mut dropped = own;
    dropped.extend(fields);
    let mut skip = dropped.clone();
    let nulled: BTreeSet<ObjectId> = slot.id.into_iter().collect();
    skip.extend(slot.id);
    pagetree::cut_references(target, &nulled, &dropped, &skip)?;
    Ok(())
}

/// A page's /Annots entries.
fn annots_of<'d>(doc: &'d Document<'_>, page: &'d Dictionary) -> &'d [Object] {
    doc.dict_get(page, b"Annots").as_array().unwrap_or(&[])
}

/// Every outline item, in preorder; shared or cyclic items once.
fn outline_items(doc: &Document<'_>) -> Vec<ObjectId> {
    let root = doc.resolve_id(doc.root());
    let outlines = get(doc, root, b"Outlines");
    let mut items = Vec::new();
    let mut seen = BTreeSet::new();
    let mut stack: Vec<&Object> = (outlines.as_dict())
        .and_then(|dict| dict.get(b"First".as_slice()))
        .into_iter()
        .collect();
    while let Some(item) = stack.pop() {
        let Some(id) = item.as_reference() else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        let Some(dict) = doc.resolve_id(id).as_dict() else {
            continue;
        };
        items.push(id);
        if let Some(next) = dict.get(b"Next".as_slice()) {
            stack.push(next);
        }
        if let Some(first) = dict.get(b"First".as_slice()) {
            stack.push(first);
        }
    }
    items
}

/// Takes widget annotations out of the field tree: each leaves its parent's
/// /Kids, a field left without kids goes too, and a top-level field that is
/// one of them (or emptied) leaves /AcroForm /Fields. Returns the fields
/// that went.
fn remove_widgets<E: Edit>(
    target: &mut E,
    widgets: &[ObjectId],
) -> Result<BTreeSet<ObjectId>, DocumentError> {
    let mut dead: Vec<ObjectId> = widgets.to_vec();
    let mut done = BTreeSet::new();
    let mut gone = BTreeSet::new();
    while let Some(node) = dead.pop() {
        if !done.insert(node) {
            continue;
        }
        let doc = target.document();
        let parent = doc
            .resolve_id(node)
            .as_dict()
            .and_then(|dict| dict.get(b"Parent".as_slice()))
            .and_then(Object::as_reference)
            .filter(|parent| doc.resolve_id(*parent).as_dict().is_some());
        let place = match parent {
            Some(parent) => Place::object(parent).child(doc, b"Kids"),
            None => {
                let acro = Place::object(doc.root()).child(doc, b"AcroForm");
                if acro.read(doc).as_dict().is_none() {
                    continue;
                }
                acro.child(doc, b"Fields")
            }
        };
        if !place
            .read(doc)
            .as_array()
            .is_some_and(|kids| kids.contains(&Object::Reference(node)))
        {
            continue;
        }
        let emptied = place.update(target, |value| {
            let kids = value.as_array_mut()?;
            kids.retain(|kid| *kid != Object::Reference(node));
            Some(kids.is_empty())
        })?;
        if let (Some(parent), Some(true)) = (parent, emptied) {
            gone.insert(parent);
            dead.push(parent);
        }
    }
    Ok(gone)
}

/// engine.ts `page-move`: page `page` becomes page `to`. Labels stay with
/// the positions.
pub fn move_page(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    session.require_mutable("page-move")?;
    let page = session.check_page(request.get("page"), "page-move")?;
    let to = session.check_page(request.get("to"), "page-move")?;
    if to == page {
        return Err(plain(format!("page-move: page {page} is already there")));
    }
    session.mutate("move page", |session| {
        let mut tx = session.tx()?;
        let tree = editable_tree(&mut tx, "page-move")?;
        tree.move_page(&mut tx, page, to)?;
        Ok(())
    })?;
    Ok(edit_reply(session, request))
}

/// engine.ts `checkPages`: 1..2000 page indices below `count`, in order,
/// repeats kept; tagged `invalid`.
pub(crate) fn check_pages(
    pages: Option<&Value>,
    count: usize,
    op: &str,
) -> Result<Vec<usize>, AppError> {
    let Some(items) = pages
        .and_then(Value::as_array)
        .filter(|items| (1..=MAX_GRAFT_PAGES).contains(&items.len()))
    else {
        return Err(tagged(
            "invalid",
            format!("{op}: pages must be 1..{MAX_GRAFT_PAGES} page numbers"),
        ));
    };
    (items.iter().enumerate())
        .map(|(index, page)| {
            request::as_int(page)
                .and_then(|page| usize::try_from(page).ok())
                .filter(|&page| page < count)
                .ok_or_else(|| {
                    tagged(
                        "invalid",
                        format!(
                            "{op}: page {index} must be an integer in 0..{}",
                            count.saturating_sub(1)
                        ),
                    )
                })
        })
        .collect()
}

/// JavaScript's `\s` and `String.prototype.trim` whitespace.
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{B}' | '\u{C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// engine.ts `parsePageRange`: 1-based `a`, `a-b`, `a-`, `-b` items joined
/// by commas, reversed ranges swapped; "" is every page. The pages come back
/// 0-based, ascending and distinct; None when the text is not a usable range.
pub fn parse_page_range(text: &str, count: usize) -> Option<Vec<usize>> {
    let trimmed = text.trim_matches(js_space);
    if trimmed.is_empty() {
        return Some((0..count).collect());
    }
    let digits = |text: &str| text.chars().all(|c| c.is_ascii_digit());
    // `Number(digits)`: every digit string is a finite number.
    let value = |text: &str| text.parse::<f64>().unwrap_or(f64::INFINITY);
    let mut wanted = BTreeSet::new();
    for item in trimmed.split(',') {
        let token = item.trim_matches(js_space);
        let (mut first, mut last) = match token.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (a.trim_end_matches(js_space), b.trim_start_matches(js_space));
                if !digits(a) || !digits(b) || (a.is_empty() && b.is_empty()) {
                    return None;
                }
                let first = if a.is_empty() { 1.0 } else { value(a) };
                let last = if b.is_empty() { count as f64 } else { value(b) };
                (first, last)
            }
            None if !token.is_empty() && digits(token) => (value(token), value(token)),
            None => return None,
        };
        if first > last {
            std::mem::swap(&mut first, &mut last);
        }
        if first < 1.0 || last > count as f64 {
            return None;
        }
        // Both are whole numbers in 1..=count here.
        wanted.extend((first as usize - 1)..(last as usize));
    }
    Some(wanted.into_iter().collect())
}

/// engine.ts `insert-pages`: pages of another PDF go in before page `at`,
/// all in one step, and their form fields join the document's.
pub fn insert(
    session: &mut AppSession,
    request: &Value,
    buffers: &mut Buffers,
) -> Result<Value, AppError> {
    let doc = session.require_mutable("insert-pages")?;
    let count = doc.page_count();
    let at = request::int(request, "at")
        .and_then(|at| usize::try_from(at).ok())
        .filter(|&at| at <= count)
        .ok_or_else(|| plain(format!("insert-pages: at must be an integer in 0..{count}")))?;
    if request::string(request, "name").is_none_or(str::is_empty) {
        return Err(plain("insert-pages: missing source name"));
    }
    let Some(bytes) = buffers.take(request.get("pdf")) else {
        return Err(plain("insert-pages: no source bytes"));
    };
    if bytes.len() < 5 {
        return Err(plain(
            "insert-pages: the source is not a PDF (too few bytes)",
        ));
    }
    if bytes.len() > MAX_PDF_BYTES {
        return Err(plain(format!(
            "insert-pages: the source exceeds the {MAX_PDF_BYTES} byte ceiling"
        )));
    }
    let source = AppDocument::load(Arc::new(bytes), b"").map_err(|error| {
        plain(format!(
            "insert-pages: the source could not be read ({error})"
        ))
    })?;
    let info = source.info(source.is_locked());
    if info.needs_password {
        return Err(plain("insert-pages: the source needs a password"));
    }
    if info.encrypted {
        return Err(plain(
            "insert-pages: an encrypted source cannot be inserted",
        ));
    }
    let source_count = source.page_count();
    if source_count < 1 {
        return Err(plain("insert-pages: the source has no pages"));
    }
    let pages = if request.get("pages").is_some_and(Value::is_array) {
        check_pages(request.get("pages"), source_count, "insert-pages")?
    } else if let Some(range) = request::string(request, "range") {
        let parsed = parse_page_range(range, source_count)
            .ok_or_else(|| plain("insert-pages: pages must look like \"1-3, 5\""))?;
        let parsed = parsed.into_iter().map(Value::from).collect();
        check_pages(Some(&Value::Array(parsed)), source_count, "insert-pages")?
    } else {
        (0..source_count).collect()
    };
    let inserted = pages.len();
    session.mutate("insert pages", |session| {
        let mut tx = session.tx()?;
        let tree = editable_tree(&mut tx, "insert-pages")?;
        let parent = tree.insertion_parent(at);
        let grafted = graft::graft(&mut tx, source.document(), &pages, parent)
            .map_err(|error| graft_failed("insert-pages", error))?;
        let new_pages = grafted
            .pages
            .iter()
            .map(|&id| Object::Reference(id))
            .collect();
        tree.insert(&mut tx, at, new_pages)?;
        let (at, added) = (
            i64::try_from(at).unwrap_or(i64::MAX),
            i64::try_from(inserted).unwrap_or(i64::MAX),
        );
        labels::adjust(&mut tx, |entries| labels::inserted(entries, at, added))?;
        graft::register_fields(&mut tx, grafted)?;
        Ok(())
    })?;
    let names = session
        .doc
        .as_ref()
        .map(|doc| graft::field_names(doc.document()))
        .unwrap_or_default();
    let mut fields = Map::new();
    fields.insert("inserted".into(), inserted.into());
    fields.insert("fieldNames".into(), names.into());
    fields.extend(session.edit_state());
    Ok(reply(request, Value::Object(fields)))
}

/// A graft's error; one over the repeated-pages bound is `too-large`.
fn graft_failed(op: &str, error: DocumentError) -> AppError {
    match error.kind {
        DocumentErrorKind::LimitExceeded(graft::REPEATED) => tagged(
            "too-large",
            format!(
                "{op}: the repeated pages would copy more than {} bytes again",
                graft::MAX_REPEATED_BYTES
            ),
        ),
        _ => error.into(),
    }
}

/// The app writer's bytes for a new document, under the save ceiling.
pub(crate) fn written(doc: &Document<'_>, op: &str, what: &str) -> Result<Vec<u8>, AppError> {
    let too_large = || {
        tagged(
            "too-large",
            format!("{op}: the {what} would exceed the {MAX_SAVE_BYTES} byte save ceiling"),
        )
    };
    let mut limits = overlay::limits();
    limits.syntax.max_output_bytes = MAX_SAVE_BYTES;
    match write_document(doc, limits, WriteOptions::APP) {
        Ok(bytes) if bytes.len() <= MAX_SAVE_BYTES => Ok(bytes),
        Ok(_) | Err(DocumentWriteError::Serialize(SerializeError::OutputLimit)) => Err(too_large()),
        Err(error) => Err(plain(format!("{op}: {error}"))),
    }
}

/// engine.ts `extract-pages`: a new PDF of the listed pages, in order. Not
/// a mutation: no permission check, no journal step.
pub fn extract(
    session: &mut AppSession,
    request: &Value,
    buffers: &mut Buffers,
) -> Result<Value, AppError> {
    let Some(doc) = &session.doc else {
        return Err(plain("extract-pages: no document open"));
    };
    if session.locked {
        return Err(plain(
            "extract-pages: document is encrypted, a password is required",
        ));
    }
    let pages = check_pages(request.get("pages"), doc.page_count(), "extract-pages")?;
    let source = doc.document();
    let version = source.version().max("1.7");
    let mut out = Builder::new(version)?;
    let tree = tree(out.document(), "extract-pages")?;
    let grafted = graft::graft(&mut out, source, &pages, tree.root)
        .map_err(|error| graft_failed("extract-pages", error))?;
    let new_pages = grafted
        .pages
        .iter()
        .map(|&id| Object::Reference(id))
        .collect();
    tree.insert(&mut out, 0, new_pages)?;
    graft::register_fields(&mut out, grafted)?;
    let bytes = written(&out.into_document(), "extract-pages", "extract")?;
    let fields = json!({"bytes": buffers.put(bytes), "pageCount": pages.len()});
    Ok(reply(request, fields))
}

/// engine.ts `pdf-from-images`: a new PDF with one page per image, in
/// order. No session state is read or changed.
pub fn from_images(request: &Value, buffers: &mut Buffers) -> Result<Value, AppError> {
    let images = request::array(request, "images")
        .filter(|images| (1..=MAX_IMAGES).contains(&images.len()))
        .ok_or_else(|| {
            plain(format!(
                "pdf-from-images: images must be 1..{MAX_IMAGES} files"
            ))
        })?;
    let mut total = 0usize;
    let mut named = Vec::with_capacity(images.len());
    for (index, image) in images.iter().enumerate() {
        let name = image
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty() && name.encode_utf16().count() <= 255)
            .ok_or_else(|| {
                plain(format!(
                    "pdf-from-images: image {index} needs a name of at most 255 characters"
                ))
            })?;
        let size = buffers
            .input(image.get("bytes"))
            .map(<[u8]>::len)
            .filter(|&size| size >= 1)
            .ok_or_else(|| {
                plain(format!(
                    "pdf-from-images: image {index} ({name}) has no bytes"
                ))
            })?;
        if size > MAX_IMAGE_BYTES {
            return Err(plain(format!(
                "pdf-from-images: {name} is over the {MAX_IMAGE_BYTES} byte image ceiling"
            )));
        }
        total = total.saturating_add(size);
        named.push((name, image.get("bytes")));
    }
    if total > MAX_IMAGES_TOTAL_BYTES {
        return Err(plain(format!(
            "pdf-from-images: the images total more than {MAX_IMAGES_TOTAL_BYTES} bytes"
        )));
    }
    let mut out = Builder::new("1.7")?;
    let mut pages = Vec::with_capacity(named.len());
    // The file is at least as large as its image streams, so an image that
    // takes them past the save ceiling ends the op before the rest are read.
    let mut stored = 0usize;
    for (name, field) in named {
        let bytes = buffers.take(field).unwrap_or_default();
        let budget = MAX_SAVE_BYTES.saturating_sub(stored);
        let placed = images::read(&bytes, budget).map_err(|error| match error {
            images::ImageError::TooLarge => tagged(
                "too-large",
                format!(
                    "pdf-from-images: the document would exceed the {MAX_SAVE_BYTES} byte save ceiling"
                ),
            ),
            images::ImageError::Unreadable(error) => tagged(
                "invalid",
                format!("pdf-from-images: {name} is not a PNG or JPEG the app can read ({error})"),
            ),
        })?;
        drop(bytes);
        stored = stored.saturating_add(placed.stored());
        if placed.width == 0 || placed.height == 0 {
            return Err(tagged(
                "invalid",
                format!("pdf-from-images: {name} has no pixels"),
            ));
        }
        pages.push(Object::Reference(images::add_page(&mut out, placed)?));
    }
    let count = pages.len();
    let tree = tree(out.document(), "pdf-from-images")?;
    tree.insert(&mut out, 0, pages)?;
    let bytes = written(&out.into_document(), "pdf-from-images", "document")?;
    let fields = json!({"bytes": buffers.put(bytes), "pageCount": count});
    Ok(reply(request, fields))
}

/// engine.ts `labels`: every page's label, the 1-based index where the
/// document gives none. Plain errors only.
pub fn list_labels(session: &AppSession, request: &Value) -> Result<Value, AppError> {
    let Some(doc) = &session.doc else {
        return Err(plain("labels: no document open"));
    };
    if session.locked {
        return Err(plain(
            "labels: document is encrypted, a password is required",
        ));
    }
    let items = labels::items(doc.document(), doc.page_count());
    Ok(reply(request, json!({ "items": items })))
}

fn labels_reply(session: &AppSession, request: &Value) -> Value {
    let mut fields = session.edit_state();
    let items = (session.doc.as_ref())
        .map(|doc| labels::items(doc.document(), doc.page_count()))
        .unwrap_or_default();
    fields.insert("items".into(), items.into());
    reply(request, Value::Object(fields))
}

/// engine.ts `labels-set`: a label range from page `from`.
pub fn labels_set(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    session.require_mutable("labels-set")?;
    let from = session.check_page(request.get("from"), "labels-set")?;
    let style = request::string(request, "style")
        .filter(|style| labels::STYLES.contains(style))
        .ok_or_else(|| plain("labels-set: style must be one of D, r, R, a, A, \"\""))?
        .to_string();
    let prefix = match request.get("prefix") {
        value if request::nullish(value) => String::new(),
        Some(Value::String(prefix)) if prefix.encode_utf16().count() <= MAX_LABEL_PREFIX_CHARS => {
            prefix.clone()
        }
        _ => {
            return Err(plain(format!(
                "labels-set: prefix must be a string of at most {MAX_LABEL_PREFIX_CHARS} characters"
            )));
        }
    };
    let start = match request.get("start") {
        value if request::nullish(value) => 1,
        Some(value) => request::as_int(value)
            .filter(|&start| start >= 1)
            .ok_or_else(|| plain("labels-set: start must be an integer of at least 1"))?,
        None => 1,
    };
    // engine.ts takes any integer; readers take /St as a 32-bit int.
    if start > MAX_LABEL_START {
        return Err(plain(format!(
            "labels-set: start must be at most {MAX_LABEL_START}"
        )));
    }
    let from = i64::try_from(from).unwrap_or(i64::MAX);
    session.mutate("set page labels", |session| {
        let mut tx = session.tx()?;
        let entries = labels::entries(tx.document()).unwrap_or_default();
        let entries = labels::set(entries, from, labels::label(&style, &prefix, start));
        labels::write(&mut tx, entries)?;
        Ok(())
    })?;
    Ok(labels_reply(session, request))
}

/// engine.ts `labels-clear`: the range starting at page `from` goes (page
/// 0's becomes decimal). Clearing where no range starts changes nothing and
/// records no step and writes no tree where there was none.
pub fn labels_clear(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let doc = session.require_mutable("labels-clear")?;
    let from = session.check_page(request.get("from"), "labels-clear")?;
    let from = i64::try_from(from).unwrap_or(i64::MAX);
    let changes = labels::entries(doc.document())
        .is_some_and(|entries| labels::clear(entries.clone(), from) != labels::normalized(entries));
    if changes {
        session.mutate("clear page labels", |session| {
            let mut tx = session.tx()?;
            labels::adjust(&mut tx, |entries| labels::clear(entries, from))?;
            Ok(())
        })?;
    }
    Ok(labels_reply(session, request))
}
