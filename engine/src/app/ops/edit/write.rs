// SPDX-License-Identifier: MIT
//! Content rewriting (content-edit.ts `editWriteElement`, `editPrivatise`,
//! `editSplice`), only ever inside a journal operation: a page element is
//! made private first when the scan said it is shared (copy-on-write), then
//! spliced once with every edit that targets it, so a batch is one atomic
//! rewrite that undo restores exactly.
use std::collections::BTreeSet;

use super::plan::{TextPlan, name_source};
use super::scan::{Cow, PageScan, Record, parent_entry, struct_element_in};
use super::{MAX_EDIT_SCAN_BYTES, decode, edit_error, scan_error};
use crate::app::edit::Place;
use crate::app::{AppError, Transaction, info, pagetree};
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId, encode_text_string};

/// Bytes one operation may copy for copy-on-write, over all its elements.
pub const MAX_EDIT_COPY_BYTES: usize = 8 * MAX_EDIT_SCAN_BYTES;

/// One byte-span replacement in a decoded stream.
#[derive(Clone, Debug, PartialEq)]
pub struct Splice {
    pub start: usize,
    pub end: usize,
    pub replacement: Vec<u8>,
}

/// A raw span rewrite tied to the record whose element it edits (a
/// paragraph, an object placement).
#[derive(Clone, Debug)]
pub struct SpanEdit {
    pub record: usize,
    pub splice: Splice,
}

/// content-edit.ts `editSplice`: highest offset first; any overlap refuses.
pub fn splice(bytes: &[u8], mut edits: Vec<Splice>) -> Result<Vec<u8>, AppError> {
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.start));
    let mut previous = bytes.len();
    for edit in &edits {
        if edit.end > previous || edit.end < edit.start {
            return Err(AppError::reason(
                "invalid",
                "edit-text: the recorded spans no longer fit the stream",
            ));
        }
        previous = edit.start;
    }
    let mut out = bytes.to_vec();
    for edit in edits {
        out.splice(edit.start..edit.end, edit.replacement);
    }
    Ok(out)
}

fn invalid(message: &str) -> AppError {
    AppError::reason("invalid", message)
}

/// The page's object; a page written directly into /Kids is not editable here.
pub fn page_object(doc: &Document<'_>, page: usize, op: &str) -> Result<ObjectId, AppError> {
    info::pages(doc)
        .get(page)
        .and_then(|p| p.id)
        .ok_or_else(|| invalid(&format!("{op}: the page is not an indirect object")))
}

fn page_dict<'d>(doc: &'d Document<'_>, page: ObjectId) -> Result<&'d Dictionary, AppError> {
    doc.resolve_id(page)
        .as_dict()
        .ok_or_else(|| invalid("edit-text: the page is no longer a dictionary"))
}

/// The stream a page /Contents element names.
fn element_stream(doc: &Document<'_>, page: &Dictionary, element: usize) -> Option<ObjectId> {
    let contents = page.get(b"Contents".as_slice())?;
    let value = match doc.resolve(contents).as_array() {
        Some(items) => items.get(element)?,
        None if element == 0 => contents,
        None => return None,
    };
    let (holder, _) = doc.resolve_with_holder(value);
    let id = holder.filter(|_| value.as_reference().is_some())?;
    doc.object(id)
        .ok()
        .filter(|o| o.stream.is_some())
        .map(|_| id)
}

fn without_length(doc: &Document<'_>, id: ObjectId) -> Result<Dictionary, AppError> {
    let mut dict = doc
        .stream_dict(id)
        .map_err(|e| invalid(&format!("edit-text: {e}")))?
        .clone();
    dict.remove(b"Length".as_slice());
    Ok(dict)
}

fn raw_of(doc: &Document<'_>, id: ObjectId) -> Result<Vec<u8>, AppError> {
    doc.stream_data(id)
        .ok()
        .flatten()
        .map(<[u8]>::to_vec)
        .ok_or_else(|| invalid("edit-text: a content stream cannot be read"))
}

/// content-edit.ts `editPrivatise`, in the caller's operation: inherited
/// /Resources copied onto the page, a shared element replaced by a private
/// copy, a Form invocation re-pointed at a private copy of the Form.
/// Returns the stream the edit writes into. Each call is charged from the
/// scan's bytes, as engine.ts charges it, so a preview's verdict holds for
/// every element; `copied` is the operation's running total, capped at
/// `MAX_EDIT_COPY_BYTES`.
pub fn privatise(
    tx: &mut Transaction<'_>,
    scan: &PageScan,
    page: ObjectId,
    element: usize,
    cow: &Cow,
    copied: &mut usize,
) -> Result<(ObjectId, ObjectId), AppError> {
    let mut budget = scan.budget_bytes;
    let mut charge = |bytes: usize| {
        budget = budget.saturating_add(bytes);
        *copied = copied.saturating_add(bytes);
        if budget > MAX_EDIT_SCAN_BYTES || *copied > MAX_EDIT_COPY_BYTES {
            return Err(edit_error(
                "too-large",
                "edit-text: copying this placement would exceed the scan ceiling",
            ));
        }
        Ok(())
    };
    let doc = tx.document();
    let dict = page_dict(doc, page)?;
    let own = dict
        .get(b"Resources".as_slice())
        .is_some_and(|v| !doc.resolve(v).is_null());
    if cow.inherited_resources && !own {
        let inherited = pagetree::inherited(doc, dict, b"Resources")
            .and_then(|v| doc.resolve(v).as_dict())
            .cloned()
            .ok_or_else(|| invalid("edit-text: the inherited resources are no longer reachable"))?;
        tx.update(page, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"Resources".to_vec(), Object::Dictionary(inherited));
            }
        })?;
    }
    let doc = tx.document();
    let dict = page_dict(doc, page)?;
    let original = element_stream(doc, dict, element)
        .ok_or_else(|| invalid("edit-text: the content stream is no longer where it was"))?;
    let mut owned = original;
    if cow.shared_contents.is_some() {
        let raw = raw_of(doc, original)?;
        charge(raw.len())?;
        let stream_dict = without_length(doc, original)?;
        let contents = dict
            .get(b"Contents".as_slice())
            .cloned()
            .unwrap_or(Object::Null);
        let resolved = doc.resolve(&contents).clone();
        owned = tx.allocate_stream_raw(stream_dict, raw)?;
        let next = match (&contents, resolved) {
            (Object::Array(items), _) => {
                let mut items = items.clone();
                if let Some(slot) = items.get_mut(element) {
                    *slot = Object::Reference(owned);
                }
                Object::Array(items)
            }
            (_, Object::Array(mut items)) => {
                if let Some(slot) = items.get_mut(element) {
                    *slot = Object::Reference(owned);
                }
                Object::Array(items)
            }
            _ => Object::Reference(owned),
        };
        tx.update(page, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"Contents".to_vec(), next);
            }
        })?;
    }
    let Some(form) = &cow.form else {
        return Ok((owned, original));
    };
    let doc = tx.document();
    let dict = page_dict(doc, page)?;
    let resources = super::font::dict_at(doc, Some(dict), b"Resources");
    let xobjects = super::font::dict_at(doc, resources, b"XObject");
    let same = xobjects
        .and_then(|x| x.get(form.name.as_slice()))
        .and_then(Object::as_reference)
        .is_some_and(|id| doc.resolve_id_with_holder(id).0 == Some(form.reference));
    let xobjects = xobjects
        .filter(|_| same)
        .ok_or_else(|| invalid("edit-text: the Form XObject is no longer where it was"))?;
    let raw = raw_of(doc, form.reference)?;
    charge(raw.len())?;
    let mut n = 1u32;
    let name = loop {
        let mut candidate = form.name.clone();
        candidate.extend_from_slice(format!("c{n}").as_bytes());
        if !xobjects.contains_key(candidate.as_slice()) {
            break candidate;
        }
        n = n.saturating_add(1);
        if n == u32::MAX {
            return Err(invalid("edit-text: no free XObject name"));
        }
    };
    let mut form_dict = without_length(doc, form.reference)?;
    // The copy gets its own /Resources dictionary (the fonts and images in
    // it stay shared), so nothing written to it reaches the original's.
    let shared_resources = form_dict
        .get(b"Resources".as_slice())
        .and_then(Object::as_reference)
        .and_then(|id| doc.resolve_id(id).as_dict().cloned());
    let place = Place::object(page)
        .child(doc, b"Resources")
        .child(doc, b"XObject");
    let bytes = bytes_of(scan, doc, owned, original)?;
    if let Some(resources) = shared_resources {
        let own = tx.allocate(Object::Dictionary(resources), None)?;
        form_dict.insert(b"Resources".to_vec(), Object::Reference(own));
    }
    let copy = tx.allocate_stream_raw(form_dict, raw)?;
    place.update(tx, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.insert(name.clone(), Object::Reference(copy));
        }
    })?;
    let renamed = splice(
        &bytes,
        vec![Splice {
            start: form.operand.start,
            end: form.operand.end,
            replacement: name_source(&name).into_bytes(),
        }],
    )?;
    tx.replace_decoded_stream(owned, renamed)?;
    Ok((copy, form.reference))
}

/// A stream's current decoded bytes; a copy the strict decoder cannot read
/// (damage the view tolerates) is its `source`'s bytes as the scan read them.
fn bytes_of(
    scan: &PageScan,
    doc: &Document<'_>,
    id: ObjectId,
    source: ObjectId,
) -> Result<Vec<u8>, AppError> {
    match decode(doc, id, usize::MAX) {
        Ok(bytes) => Ok(bytes),
        Err(error) => scan
            .streams
            .get(&source)
            .map(|b| b.to_vec())
            .ok_or_else(|| {
                let message = match error {
                    super::DecodeError::TooLarge => "too large".to_string(),
                    super::DecodeError::Invalid(message) => message,
                };
                invalid(&format!(
                    "edit-text: a content stream cannot be read ({message})"
                ))
            }),
    }
}

/// content-edit.ts `editWriteElement`: one /Contents element's plans and
/// spans, privatised once, spliced once, then the StructElem ActualText puts.
/// `privatised` and `copied` (bytes copied so far, see `privatise`) belong
/// to the whole operation: pass the same ones for every element.
#[allow(clippy::too_many_arguments)]
pub fn write_element(
    tx: &mut Transaction<'_>,
    scan: &PageScan,
    page: ObjectId,
    element: usize,
    plans: &[&TextPlan],
    spans: &[SpanEdit],
    privatised: &mut BTreeSet<usize>,
    copied: &mut usize,
) -> Result<(), AppError> {
    let record = |index: usize| -> Result<&Record, AppError> {
        scan.records
            .get(index)
            .ok_or_else(|| invalid("edit-text: the target is gone"))
    };
    let doc = tx.document();
    let dict = page_dict(doc, page)?;
    let mut stream = element_stream(doc, dict, element)
        .ok_or_else(|| invalid("edit-text: the content stream is no longer where it was"))?;
    let mut cow: Option<&Cow> = None;
    for plan in plans {
        if let Some(found) = &record(plan.record)?.cow {
            cow = Some(found);
            break;
        }
    }
    if cow.is_none() {
        for span in spans {
            if let Some(found) = &record(span.record)?.cow {
                cow = Some(found);
                break;
            }
        }
    }
    let mut source = stream;
    if let Some(cow) = cow {
        if privatised.insert(element) {
            (stream, source) = privatise(tx, scan, page, element, cow, copied)?;
        }
    }
    let bytes = bytes_of(scan, tx.document(), stream, source)?;
    let mut edits: Vec<Splice> = spans.iter().map(|s| s.splice.clone()).collect();
    for plan in plans {
        let target = record(plan.record)?;
        let mut operand = plan.prefix.clone().into_bytes();
        operand.extend_from_slice(&plan.operand);
        edits.push(Splice {
            start: target.operand.start,
            end: target.operand.end,
            replacement: operand,
        });
        if target.kerned || !plan.suffix.is_empty() {
            let mut op = if target.kerned {
                b"Tj".to_vec()
            } else {
                bytes.get(target.op.clone()).unwrap_or_default().to_vec()
            };
            op.extend_from_slice(plan.suffix.as_bytes());
            edits.push(Splice {
                start: target.op.start,
                end: target.op.end,
                replacement: op,
            });
        }
        if let Some(actual) = &target.actual_text {
            edits.push(Splice {
                start: actual.span.start,
                end: actual.span.end,
                replacement: plan.tag.clone(),
            });
        }
    }
    let spliced = splice(&bytes, edits)?;
    tx.replace_decoded_stream(stream, spliced)?;
    let mut tagged = Vec::new();
    for plan in plans {
        let target = record(plan.record)?;
        if let (Some(mcid), Some(_)) = (target.mcid, &target.struct_actual_text) {
            tagged.push((mcid, plan));
        }
    }
    if tagged.is_empty() {
        return Ok(());
    }
    let doc = tx.document();
    let entry = parent_entry(doc, page_dict(doc, page)?);
    let mut holders = Vec::with_capacity(tagged.len());
    for (mcid, plan) in tagged {
        let holder = entry
            .and_then(|entry| struct_element_in(doc, entry, mcid))
            .and_then(|(holder, _)| holder)
            .ok_or_else(|| invalid("edit-text: the tagged element is no longer reachable"))?;
        holders.push((holder, plan));
    }
    for (holder, plan) in holders {
        let value = Object::String(encode_text_string(&plan.next));
        tx.update(holder, |object| {
            if let Some(dict) = object.as_dict_mut() {
                dict.insert(b"ActualText".to_vec(), value);
            }
        })?;
    }
    Ok(())
}

/// content-edit.ts `editWritePlans`: plans grouped by element (ascending),
/// each group by operator offset (descending), one write per element.
/// `fail` maps a writer error and the group's first target id.
pub fn write_plans(
    tx: &mut Transaction<'_>,
    scan: &PageScan,
    page: ObjectId,
    plans: &[TextPlan],
    fail: impl Fn(AppError, &str) -> AppError,
) -> Result<(), AppError> {
    let mut groups: Vec<(usize, Vec<&TextPlan>)> = Vec::new();
    for plan in plans {
        let glyph_line = scan
            .records
            .get(plan.record)
            .is_some_and(|r| r.glyph_line.is_some());
        if glyph_line && !plan.rewrite {
            continue;
        }
        match groups
            .iter_mut()
            .find(|(element, _)| *element == plan.element)
        {
            Some((_, group)) => group.push(plan),
            None => groups.push((plan.element, vec![plan])),
        }
    }
    groups.sort_by_key(|(element, _)| *element);
    let mut privatised = BTreeSet::new();
    let mut copied = 0;
    for (element, mut group) in groups {
        group.sort_by(|a, b| {
            let start = |p: &TextPlan| scan.records.get(p.record).map_or(0, |r| r.op.start);
            start(b).cmp(&start(a))
        });
        let first = group
            .first()
            .and_then(|p| scan.records.get(p.record))
            .map(|r| r.target_id.clone())
            .unwrap_or_default();
        write_element(
            tx,
            scan,
            page,
            element,
            &group,
            &[],
            &mut privatised,
            &mut copied,
        )
        .map_err(|error| fail(error, &first))?;
    }
    Ok(())
}

/// The standard font dictionary text-add writes (a simple font for a
/// base-14 font with Latin encoding).
pub fn base14_font(name: &str) -> Object {
    Object::dictionary([
        ("Type", Object::name("Font")),
        ("Subtype", Object::name("Type1")),
        ("BaseFont", Object::name(name)),
        ("Encoding", Object::name("WinAnsiEncoding")),
    ])
}

/// text-add's write: the page gets direct /Resources (a shallow copy) whose
/// direct /Font holds the new font, and a direct /Contents array with the
/// new stream appended.
pub fn add_text(
    tx: &mut Transaction<'_>,
    page: ObjectId,
    font_name: &[u8],
    font: &str,
    stream: Vec<u8>,
) -> Result<(), AppError> {
    let doc = tx.document();
    let dict = page_dict(doc, page)?;
    let resources = match dict.get(b"Resources".as_slice()).map(|v| doc.resolve(v)) {
        Some(value) if !value.is_null() => value.as_dict().cloned(),
        _ => pagetree::inherited(doc, dict, b"Resources")
            .and_then(|v| doc.resolve(v).as_dict().cloned()),
    };
    let mut resources = resources.unwrap_or_default();
    let mut fonts = resources
        .get(b"Font".as_slice())
        .and_then(|v| doc.resolve(v).as_dict())
        .cloned()
        .unwrap_or_default();
    let mut contents: Vec<Object> = match dict.get(b"Contents".as_slice()) {
        None => Vec::new(),
        Some(value) => match doc.resolve(value) {
            Object::Array(items) => items.clone(),
            Object::Null => Vec::new(),
            _ => vec![value.clone()],
        },
    };
    let font_id = tx.allocate(base14_font(font), None)?;
    fonts.insert(font_name.to_vec(), Object::Reference(font_id));
    resources.insert(b"Font".to_vec(), Object::Dictionary(fonts));
    let content = tx.allocate(Object::Dictionary(Dictionary::new()), Some(stream))?;
    contents.push(Object::Reference(content));
    tx.update(page, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.insert(b"Resources".to_vec(), Object::Dictionary(resources));
            dict.insert(b"Contents".to_vec(), Object::Array(contents));
        }
    })?;
    Ok(())
}

/// A scan error raised while writing keeps its own token.
pub fn writer_error(error: AppError) -> AppError {
    if error.reason.is_some() || error.detail.is_some() {
        error
    } else {
        scan_error("invalid", error.message)
    }
}
