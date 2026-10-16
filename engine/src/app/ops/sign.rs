// SPDX-License-Identifier: AGPL-3.0-or-later
//! Signatures: sign-list and sign-prepare (engine.ts, sign-prepare.ts).
//!
//! sign-list reports every signature field and the signer data a verifier
//! needs (byte ranges, the /Contents bytes, DocMDP), with the retained
//! signed source. sign-prepare writes a copy with a signature field whose
//! /Contents and /ByteRange are placeholders; the external signer (in
//! TypeScript) fills them in place. Neither op changes the session's
//! document or journal.
//!
//! A signed source is kept as it was opened (`AppSession::signed_source`)
//! and a further signature is an incremental update after it, so the
//! earlier signatures stay valid. A source the strict reader refuses only
//! for its layout (hybrid or linearized xref, offsets on the whitespace
//! before an object, text after %%EOF) is edited as rebuilt from its own
//! xref, which keeps every object number; real damage is refused.
pub mod prepare;
pub mod widgets;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::app::{
    AppError, AppSession, Buffers, MAX_SAVE_BYTES, SecuritySetting, info, overlay, reply, request,
};
use crate::document::Document;
use crate::incremental::Original;
use crate::object::{Dictionary, Object, ObjectId, decode_text_string};
use crate::repair::{RepairLimits, RepairedPdf, rebuild};
use crate::serialize::SerializeError;
use crate::writer::{DocumentWriteError, Encryption, WriteOptions, write_document};

use prepare::{MAX_SIG_CONTENTS_BYTES, Options};
use widgets::{Frame, Widget, real, widgets};

/// engine.ts `MAX_FIELD_NODES`: field nodes sign-list visits.
pub const MAX_FIELD_NODES: usize = 4096;
/// engine.ts `MAX_FIELD_DEPTH`.
pub const MAX_FIELD_DEPTH: usize = 8;
/// Nodes the encrypted-form check queues before it assumes a signature
/// field is there.
const MAX_SIGNATURE_PROBE: usize = 65_536;
/// /Annots entries the sign-list page map reads; an array shared by many
/// pages could otherwise cost pages × entries.
const MAX_MAPPED_ANNOTS: usize = 1 << 20;
/// engine.ts sign-prepare's limits on the signer name and reason, in UTF-16
/// code units.
const MAX_SIGNER_CHARS: usize = 200;
/// The largest time value `new Date` accepts, in milliseconds.
const MAX_TIME: f64 = 8.64e15;
/// MuPDF's `PDF_MAX_OBJECT_NUMBER` + 1: a file whose xref accounts for more
/// numbers is one MuPDF repairs.
const MAX_XREF_SIZE: u64 = 8_388_608;

fn tagged(reason: &str, message: impl Into<String>) -> AppError {
    AppError::reason(reason, message)
}

fn js_length(text: &str) -> usize {
    text.encode_utf16().count()
}

/// engine-core.ts `at`: `object[key]` resolved, None when null or when
/// `object` is not a dictionary.
fn at<'d>(doc: &'d Document<'_>, object: Option<&'d Object>, key: &[u8]) -> Option<&'d Object> {
    let dict = doc.resolve(object?).as_dict()?;
    let value = doc.dict_get(dict, key);
    (!value.is_null()).then_some(value)
}

/// `dict[key]` resolved, None when null (`at` on a dictionary).
fn own<'d>(doc: &'d Document<'_>, dict: &'d Dictionary, key: &[u8]) -> Option<&'d Object> {
    let value = doc.dict_get(dict, key);
    (!value.is_null()).then_some(value)
}

/// An output buffer the reply carries as a `Uint8Array` (`{"$bytes": n}`),
/// where engine.ts replies hold a MuPDF byte string.
fn byte_string(buffers: &mut Buffers, bytes: Vec<u8>) -> Value {
    let placeholder = buffers.put(bytes);
    json!({ "$bytes": placeholder.get("$buffer").cloned().unwrap_or(Value::Null) })
}

/// A float32 rectangle as reply numbers.
fn rect_json(rect: [f32; 4]) -> Value {
    Value::Array(
        rect.iter()
            .map(|&v| request::js_number_value(f64::from(v)))
            .collect(),
    )
}

/// A DocMDP reference's /P as engine.ts reads it: `Number(P) || null`.
fn mdp_permission(doc: &Document<'_>, reference: &Object) -> Option<Value> {
    let params = at(doc, Some(reference), b"TransformParams");
    let p = at(doc, params, b"P").and_then(|p| real(doc, p))?;
    (p != 0.0 && !p.is_nan()).then(|| request::js_number_value(f64::from(p)))
}

fn is_docmdp(doc: &Document<'_>, reference: &Object) -> bool {
    at(doc, Some(reference), b"TransformMethod").and_then(Object::as_name) == Some(b"DocMDP")
}

/// engine.ts `sign-list`.
pub fn list(
    session: &mut AppSession,
    request: &Value,
    buffers: &mut Buffers,
) -> Result<Value, AppError> {
    let Some(app) = &session.doc else {
        return Err(tagged("no-document", "sign-list: no document open"));
    };
    if session.locked {
        return Err(tagged(
            "encrypted",
            "sign-list: encrypted document needs a password",
        ));
    }
    let doc = app.document();
    let root = doc.resolve_id(doc.root());
    let acro = at(doc, Some(root), b"AcroForm");
    let fields = at(doc, acro, b"Fields");
    if app.info(false).encrypted && has_signature_field(doc, fields) {
        return Err(tagged(
            "encrypted",
            "sign-list: encrypted signed documents cannot be verified in this version",
        ));
    }
    let placed = widget_pages(doc);
    let mut walk = Walk {
        doc,
        placed: &placed,
        buffers,
        visited: 0,
        contents: 0,
        entries: Vec::new(),
    };
    if let Some(fields) = fields.and_then(Object::as_array) {
        for field in fields {
            walk.node(field, "", None, 1)?;
        }
    }
    let Walk { entries, .. } = walk;
    let signed = entries
        .iter()
        .any(|entry| entry.get("empty") == Some(&Value::Bool(false)));
    if signed && session.signed_source.is_none() {
        return Err(tagged(
            "too-large",
            "sign-list: source is too large to verify",
        ));
    }
    let mut certified = Value::Null;
    let perm = at(doc, at(doc, Some(root), b"Perms"), b"DocMDP");
    if let Some(references) = at(doc, perm, b"Reference").and_then(Object::as_array) {
        for reference in references {
            if is_docmdp(doc, reference) {
                certified = mdp_permission(doc, reference).unwrap_or(Value::Null);
            }
        }
    }
    // MuPDF counts the file's own sections, not a rebuild's.
    let versions = app.versions();
    let bytes = session.signed_source.clone().unwrap_or_default();
    let mut fields = Map::new();
    fields.insert("signatures".into(), Value::Array(entries));
    fields.insert("certified".into(), certified);
    fields.insert("versions".into(), versions.into());
    fields.insert("bytes".into(), buffers.put(bytes));
    Ok(reply(request, Value::Object(fields)))
}

/// The signed file rebuilt from its own xref, when that took no repair.
fn rebuilt(source: &[u8]) -> Option<RepairedPdf> {
    let limits = RepairLimits::for_document(crate::app::document::limits());
    rebuild(source, limits)
        .ok()
        .filter(|rebuilt| !rebuilt.report.damaged)
}

/// The signed file to extend and the object numbers its xref accounts for:
/// opened as it is, else rebuilt from its own xref when the strict reader
/// refuses only its layout. A file that needs repair is refused, as MuPDF
/// refuses incremental writes on a repaired file.
fn extendable(source: &[u8]) -> Result<(Document<'static>, u64), AppError> {
    let refused = |why: String| {
        tagged(
            "invalid",
            format!(
                "sign-prepare: the signed file needs repair, so another signature cannot keep the earlier ones valid ({why})"
            ),
        )
    };
    let limits = crate::app::document::limits();
    let options = crate::app::document::open_options();
    let strict = match Document::open_shared_with(Arc::new(source.to_vec()), limits, b"", options) {
        // A reference to another generation is read from the rebuilt file,
        // as the session reads it (`AppDocument::load_any`).
        Ok(doc) if doc.has_stale_generations() => crate::document::DocumentError {
            offset: 0,
            kind: crate::document::DocumentErrorKind::Invalid("reference to another generation"),
        },
        // MuPDF repairs a file holding a number past its limit.
        Ok(doc)
            if doc
                .next_id()
                .is_ok_and(|next| u64::from(next.number()) <= MAX_XREF_SIZE) =>
        {
            let size = Original::of(&doc).size;
            return Ok((doc, size));
        }
        Ok(_) => return Err(refused("object numbers past 8388607".into())),
        Err(error) => error,
    };
    let Some(RepairedPdf { bytes, report }) = rebuilt(source) else {
        return Err(refused(strict.to_string()));
    };
    if report.xref_size > MAX_XREF_SIZE {
        return Err(refused("object numbers past 8388607".into()));
    }
    let doc = Document::open_shared_with(Arc::new(bytes), limits, b"", options)
        .map_err(|error| refused(error.to_string()))?;
    Ok((doc, report.xref_size))
}

/// Whether a node of the field tree, within `MAX_FIELD_DEPTH` levels, has
/// its own /FT /Sig. A shared node is visited once per depth; past
/// `MAX_SIGNATURE_PROBE` queued nodes the answer is yes (engine.ts recurses
/// without a bound).
fn has_signature_field(doc: &Document<'_>, fields: Option<&Object>) -> bool {
    let Some(fields) = fields.and_then(Object::as_array) else {
        return false;
    };
    let mut seen: BTreeSet<(ObjectId, usize)> = BTreeSet::new();
    let Some(mut budget) = MAX_SIGNATURE_PROBE.checked_sub(fields.len()) else {
        return true;
    };
    let mut stack: Vec<(&Object, usize)> = fields.iter().map(|node| (node, 0)).collect();
    while let Some((node, depth)) = stack.pop() {
        if let Object::Reference(id) = node {
            if !seen.insert((*id, depth)) {
                continue;
            }
        }
        if at(doc, Some(node), b"FT").and_then(Object::as_name) == Some(b"Sig") {
            return true;
        }
        if depth < MAX_FIELD_DEPTH {
            if let Some(kids) = at(doc, Some(node), b"Kids").and_then(Object::as_array) {
                let Some(left) = budget.checked_sub(kids.len()) else {
                    return true;
                };
                budget = left;
                stack.extend(kids.iter().map(|kid| (kid, depth + 1)));
            }
        }
    }
    false
}

/// engine.ts's widget map: each widget's object and its /Parent ancestors
/// (`MAX_FIELD_DEPTH` nodes in all) to the 1-based page and `getRect()` of
/// the last widget listed under them. Direct dictionaries have no object
/// number and are left out (MuPDF files them all under 0). Pages past
/// `MAX_MAPPED_ANNOTS` annotation entries are not mapped.
fn widget_pages(doc: &Document<'_>) -> BTreeMap<ObjectId, (usize, [f32; 4])> {
    let mut placed = BTreeMap::new();
    let mut budget = MAX_MAPPED_ANNOTS;
    for (index, page) in info::pages(doc).iter().enumerate() {
        let annots = doc.dict_get(page.dict, b"Annots").as_array();
        let Some(left) = budget.checked_sub(annots.map_or(0, <[Object]>::len)) else {
            return placed;
        };
        budget = left;
        let frame = Frame::of(doc, page.dict);
        for widget in widgets(doc, page.dict) {
            let found = (index + 1, frame.annotation_rect(doc, widget.dict));
            let Widget { mut id, dict } = widget;
            let mut node = Some(dict);
            for _ in 0..MAX_FIELD_DEPTH {
                if let Some(id) = id {
                    placed.insert(id, found);
                }
                let Some(parent) = node.and_then(|dict| dict.get(b"Parent".as_slice())) else {
                    break;
                };
                let (holder, value) = doc.resolve_with_holder(parent);
                if value.is_null() {
                    break;
                }
                id = holder;
                node = value.as_dict();
            }
        }
    }
    placed
}

/// The sign-list field walk.
struct Walk<'d, 'b> {
    doc: &'d Document<'static>,
    placed: &'d BTreeMap<ObjectId, (usize, [f32; 4])>,
    buffers: &'b mut Buffers,
    visited: usize,
    /// /Contents bytes copied out so far.
    contents: usize,
    entries: Vec<Value>,
}

impl Walk<'_, '_> {
    /// engine.ts `walk`: depth-first, entries in visiting order. A node
    /// listed twice is visited (and reported) twice, within the bounds.
    fn node(
        &mut self,
        node: &Object,
        prefix: &str,
        inherited: Option<&[u8]>,
        depth: usize,
    ) -> Result<(), AppError> {
        if depth > MAX_FIELD_DEPTH {
            return Ok(());
        }
        self.visited += 1;
        if self.visited > MAX_FIELD_NODES {
            return Err(tagged(
                "too-many",
                "sign-list: this form is too large to verify",
            ));
        }
        let doc = self.doc;
        let own = match at(doc, Some(node), b"T") {
            Some(Object::String(text)) => decode_text_string(text),
            _ => String::new(),
        };
        let field = match (prefix.is_empty(), own.is_empty()) {
            (_, true) => prefix.to_string(),
            (true, false) => own.clone(),
            (false, false) => format!("{prefix}.{own}"),
        };
        let ft = at(doc, Some(node), b"FT").and_then(Object::as_name);
        let kind = ft.or(inherited);
        let value = at(doc, Some(node), b"V");
        if kind == Some(b"Sig") && (ft.is_some() || value.is_some() || !own.is_empty()) {
            let entry = self.entry(node, &field, value)?;
            self.entries.push(entry);
        }
        // Kids past the depth limit are skipped unvisited.
        if depth < MAX_FIELD_DEPTH {
            if let Some(kids) = at(doc, Some(node), b"Kids").and_then(Object::as_array) {
                for kid in kids {
                    self.node(kid, &field, kind, depth + 1)?;
                }
            }
        }
        Ok(())
    }

    fn entry(
        &mut self,
        node: &Object,
        field: &str,
        value: Option<&Object>,
    ) -> Result<Value, AppError> {
        let doc = self.doc;
        let placed = node.as_reference().and_then(|id| self.placed.get(&id));
        let mut entry = Map::new();
        entry.insert("field".into(), field.into());
        entry.insert("page".into(), placed.map_or(0, |(page, _)| *page).into());
        let rect = placed.map_or(Value::Null, |(_, rect)| rect_json(*rect));
        entry.insert("rect".into(), rect);
        entry.insert("empty".into(), value.is_none().into());
        let Some(value) = value else {
            return Ok(Value::Object(entry));
        };
        let range = at(doc, Some(value), b"ByteRange").and_then(Object::as_array);
        let range: Option<Vec<u64>> = range
            .filter(|range| range.len() == 4)
            .and_then(|range| range.iter().map(|n| offset(doc.resolve(n))).collect());
        let contents = match at(doc, Some(value), b"Contents") {
            Some(Object::String(bytes)) => Some(bytes),
            _ => None,
        };
        let (Some(range), Some(contents), Some(_)) =
            (range, contents, doc.resolve(value).as_dict())
        else {
            entry.insert("status".into(), "unsupported".into());
            entry.insert("reason".into(), "malformed".into());
            return Ok(Value::Object(entry));
        };
        let name = |key: &[u8]| {
            at(doc, Some(value), key)
                .and_then(Object::as_name)
                .map(|name| String::from_utf8_lossy(name).into_owned())
                .unwrap_or_default()
        };
        entry.insert("subFilter".into(), name(b"SubFilter").into());
        entry.insert("filter".into(), name(b"Filter").into());
        for (key, label) in [
            (b"Name".as_slice(), "name"),
            (b"Reason", "reason"),
            (b"Location", "location"),
            (b"ContactInfo", "contact"),
            (b"M", "m"),
        ] {
            if let Some(Object::String(text)) = at(doc, Some(value), key) {
                entry.insert(label.into(), decode_text_string(text).into());
            }
        }
        entry.insert("byteRange".into(), json!(range));
        self.contents = self.contents.saturating_add(contents.len());
        if self.contents > MAX_SAVE_BYTES {
            return Err(tagged(
                "too-large",
                "sign-list: the signatures hold more than the save ceiling to verify",
            ));
        }
        let contents = byte_string(self.buffers, contents.clone());
        entry.insert("contents".into(), contents);
        if let Some(references) = at(doc, Some(value), b"Reference").and_then(Object::as_array) {
            if let Some(reference) = references.iter().find(|r| is_docmdp(doc, r)) {
                let p = mdp_permission(doc, reference).unwrap_or(Value::Null);
                entry.insert("docmdp".into(), p);
            }
        }
        Ok(Value::Object(entry))
    }
}

/// A /ByteRange number: a non-negative integer, or a real holding one.
fn offset(value: &Object) -> Option<u64> {
    match value {
        Object::Integer(n) => u64::try_from(*n).ok(),
        Object::Real(r) if r.fract() == 0.0 && (0.0..=9_007_199_254_740_991.0).contains(r) => {
            Some(*r as u64)
        }
        _ => None,
    }
}

/// engine.ts `sign-prepare`: refusals in engine.ts order, then the copy.
pub fn prepare(
    session: &mut AppSession,
    request: &Value,
    buffers: &mut Buffers,
) -> Result<Value, AppError> {
    let encrypted = || {
        tagged(
            "encrypted",
            "sign-prepare: encrypted documents cannot be signed in this version",
        )
    };
    let Some(app) = &session.doc else {
        return Err(tagged("no-document", "sign-prepare: no document open"));
    };
    if session.locked {
        return Err(encrypted());
    }
    if session.source_bytes > MAX_SAVE_BYTES {
        return Err(tagged(
            "too-large",
            "sign-prepare: source exceeds save limit",
        ));
    }
    let signer = request::string(request, "name")
        .filter(|name| (1..=MAX_SIGNER_CHARS).contains(&js_length(name)));
    let reason = request::string(request, "reason").filter(|reason| {
        js_length(reason) <= MAX_SIGNER_CHARS
            && !reason.chars().any(|c| c < '\u{20}' || c == '\u{7f}')
    });
    let certify = request::boolean(request, "certify");
    let when = request
        .get("when")
        .and_then(request::as_js_number)
        .filter(|when| when.is_finite());
    let (Some(signer), Some(reason), Some(certify), Some(when)) = (signer, reason, certify, when)
    else {
        return Err(tagged(
            "invalid",
            "sign-prepare: invalid signer, reason, or timestamp",
        ));
    };
    // `new Date(when)` is invalid past ±8.64e15 ms; engine.ts would write
    // "NaN" dates.
    let when = when.trunc();
    if when.abs() > MAX_TIME {
        return Err(tagged(
            "invalid",
            "sign-prepare: invalid signer, reason, or timestamp",
        ));
    }
    let when = when as i64;
    // P5 U3 §1.4/§9.8: a field signature names the field, its page, and no rect.
    let rect_value = request.get("rect");
    let field = match request.get("field") {
        None | Some(Value::Null) => None,
        Some(Value::String(field)) if !field.is_empty() && rect_value == Some(&Value::Null) => {
            Some(field.as_str())
        }
        Some(_) => {
            return Err(tagged(
                "invalid",
                "sign-prepare: a field signature takes rect null and the field's page",
            ));
        }
    };
    let info = app.info(false);
    if info.encrypted {
        return Err(encrypted());
    }
    // The copy would be written with the security the next save writes.
    if matches!(
        session.effective_security(),
        Some(SecuritySetting::Aes256 { .. })
    ) {
        return Err(encrypted());
    }
    let doc = app.document();
    let root = doc.resolve_id(doc.root());
    let mdp = at(doc, at(doc, Some(root), b"Perms"), b"DocMDP");
    let first = at(doc, mdp, b"Reference")
        .and_then(Object::as_array)
        .and_then(<[Object]>::first);
    let params = first.and_then(|first| at(doc, Some(first), b"TransformParams"));
    if at(doc, params, b"P").and_then(|p| real(doc, p)) == Some(1.0) {
        return Err(tagged(
            "certified",
            "sign-prepare: this certified document does not allow changes",
        ));
    }
    if certify && info.signatures > 0 {
        return Err(tagged(
            "already-signed",
            "sign-prepare: certification must be the first signature",
        ));
    }
    // A new visible field after certification is an illegal DocMDP change
    // (pyHanko, Acrobat); invisible is form filling.
    if field.is_none() && info.certified && rect_value != Some(&Value::Null) {
        return Err(tagged(
            "certified",
            "sign-prepare: a certified document accepts only an invisible signature",
        ));
    }
    let page = request
        .get("page")
        .and_then(request::as_int)
        .and_then(|page| usize::try_from(page).ok())
        .filter(|&page| page < app.page_count())
        .ok_or_else(|| tagged("invalid", "sign-prepare: invalid page"))?;
    let rect = match rect_value {
        Some(Value::Null) => None,
        _ => Some(
            request::numbers(request, "rect")
                .and_then(|rect| <[f64; 4]>::try_from(rect).ok())
                .ok_or_else(|| tagged("invalid", "sign-prepare: invalid rectangle"))?,
        ),
    };
    if let Some(field) = field {
        check_field(doc, page, field)?;
    }
    // The adapter sends `new Date(when).getTimezoneOffset()` (UTC minus
    // local, minutes); without it the appearance shows UTC.
    let local_offset = request
        .get("timezoneOffset")
        .and_then(request::as_number)
        .filter(|minutes| minutes.abs() <= 1440.0)
        .map_or(0, |minutes| (-minutes * 60_000.0).round() as i64);
    let options = Options {
        name: signer,
        reason,
        page,
        rect,
        certify,
        when,
        local_offset,
        field,
    };
    let prepared = match &session.signed_source {
        Some(source) => {
            let (doc, size) = extendable(source)?;
            let original = Original {
                bytes: source,
                size,
            };
            prepare::prepare(doc, &options, Some(original))?
        }
        None => prepare::prepare(serialize(session)?, &options, None)?,
    };
    if let Some(source) = &session.signed_source {
        if !prepared.bytes.starts_with(source) {
            return Err(tagged(
                "invalid",
                "sign-prepare: incremental save changed source bytes",
            ));
        }
    }
    let mut fields = Map::new();
    fields.insert("bytes".into(), buffers.put(prepared.bytes));
    fields.insert("contentsAt".into(), prepared.contents_at.into());
    fields.insert("contentsLen".into(), (MAX_SIG_CONTENTS_BYTES * 2).into());
    fields.insert("byteRangeAt".into(), prepared.byte_range_at.into());
    fields.insert("page".into(), page.into());
    let rect = prepared.rect.map(request::js_number_value);
    fields.insert("rect".into(), Value::Array(rect.to_vec()));
    fields.insert("revisions".into(), prepared.revisions.into());
    if let Some(field) = field {
        fields.insert("field".into(), field.into());
    }
    Ok(reply(request, Value::Object(fields)))
}

/// engine.ts `checkSignatureField`: the empty, merged, top-level signature
/// field `name` on page `index`; each refusal comes before any write.
fn check_field(doc: &Document<'_>, index: usize, name: &str) -> Result<(), AppError> {
    let invalid = |what: &str| tagged("invalid", format!("sign-prepare: {name} {what}"));
    let signed = || {
        tagged(
            "already-signed",
            format!("sign-prepare: {name} is already signed"),
        )
    };
    let pages = info::pages(doc);
    let page = pages
        .get(index)
        .ok_or_else(|| tagged("invalid", "sign-prepare: invalid page"))?;
    let found = widgets(doc, page.dict)
        .into_iter()
        .find(|w| w.is_signature(doc) && w.name(doc).as_deref() == Some(name));
    let Some(widget) = found else {
        return Err(tagged(
            "no-such-field",
            format!("sign-prepare: page {index} has no signature field {name}"),
        ));
    };
    let dict = widget.dict;
    let parent = own(doc, dict, b"Parent");
    if at(doc, parent, b"V").is_some() {
        return Err(signed());
    }
    let own_type = own(doc, dict, b"FT").and_then(Object::as_name);
    let titled = matches!(own(doc, dict, b"T"), Some(Object::String(_)));
    if parent.is_some() || own_type != Some(b"Sig") || !titled {
        return Err(invalid("is part of a field hierarchy"));
    }
    // §9.3: a widget on /Annots but not in /AcroForm /Fields is no field a
    // verifier sees.
    let root = doc.resolve_id(doc.root());
    let fields = at(doc, at(doc, Some(root), b"AcroForm"), b"Fields").and_then(Object::as_array);
    let listed = widget.id.is_some_and(|id| {
        fields.is_some_and(|fields| fields.iter().any(|f| f.as_reference() == Some(id)))
    });
    if !listed {
        return Err(invalid("is not in the document's /AcroForm /Fields"));
    }
    if own(doc, dict, b"V").is_some() {
        return Err(signed());
    }
    if widget.read_only(doc) {
        return Err(invalid("is read-only"));
    }
    let rect = Frame::of(doc, page.dict).annotation_rect(doc, widget.dict);
    let (w, h) = (
        f64::from(rect[2]) - f64::from(rect[0]),
        f64::from(rect[3]) - f64::from(rect[1]),
    );
    if !(w >= 40.0 && h >= 20.0) {
        return Err(invalid("is too small for a signature appearance"));
    }
    Ok(())
}

/// engine.ts `serializeDocument` for an unsigned document: a full save of
/// the current state (unsaved edits included), reopened as the copy to
/// sign. The checks and messages are the save's.
fn serialize(session: &AppSession) -> Result<Document<'static>, AppError> {
    let Some(app) = &session.doc else {
        return Err(tagged("no-document", "save: no document open"));
    };
    if app.info(session.locked).signed() {
        return Err(tagged(
            "read-only",
            "save: signed or certified documents are read-only",
        ));
    }
    let mut limits = overlay::limits();
    limits.syntax.max_output_bytes = MAX_SAVE_BYTES;
    let options = WriteOptions {
        encryption: Encryption::Plaintext,
        ..WriteOptions::APP
    };
    let written = crate::app::save_tree::for_save(app.document())
        .map_err(|error| tagged("invalid", format!("save: {error}")))?;
    let bytes = match write_document(&written, limits, options) {
        Ok(bytes) if bytes.len() <= MAX_SAVE_BYTES => bytes,
        Ok(_) | Err(DocumentWriteError::Serialize(SerializeError::OutputLimit)) => {
            return Err(tagged(
                "too-large",
                format!("save: the copy would exceed the {MAX_SAVE_BYTES} byte save ceiling"),
            ));
        }
        Err(error) => return Err(tagged("invalid", format!("save: {error}"))),
    };
    let (limits, options) = (
        crate::app::document::limits(),
        crate::app::document::open_options(),
    );
    Document::open_shared_with(Arc::new(bytes), limits, b"", options).map_err(|error| {
        tagged(
            "invalid",
            format!("sign-prepare: the saved copy could not be read ({error})"),
        )
    })
}
