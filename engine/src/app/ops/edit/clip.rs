// SPDX-License-Identifier: AGPL-3.0-or-later
//! The worker clipboard (engine.ts `clipCopy`, `clipPaste`).
//!
//! `clip-copy` writes nothing: it copies the members' objects (an image or
//! Form XObject, a run's font, with everything they reference) into a
//! scratch store whose references are renumbered within it, so the
//! clipboard outlives the document and crosses to any session of the
//! engine. `clip-paste` copies them back into the target document in one
//! journal step: new resource names (`PA<n>`, `PF<n>`), one new /Contents
//! element, a rescan proving every object and run landed where it was
//! placed, then the field, link and mark members through their own ops
//! (`field-add`, `link-add`, `redact-add`), dispatched in the same step.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::geometry::{edit_number, invert, js_round, map_rect, multiply, writable};
use super::object::{
    check_after, check_area, check_box, page_clip_box, page_resources, placement_after, rect_affine,
};
use super::scan::{Kind, Placement, runs_of};
use super::write::page_object;
use super::{MAX_EDIT_TARGETS, edit_error, edit_state_reply, fresh_scan, resolve_target, scan_for};
use crate::app::request::{self, js_string};
use crate::app::{
    AppEngine, AppError, AppSession, Batch, Buffers, Need, Transaction, edit::Place, pagetree,
    reply,
};
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};

/// engine.ts `MAX_IMAGE_BYTES`: the stream bytes one copy may hold.
pub const MAX_CLIP_BYTES: usize = 32 * 1024 * 1024;
/// redact.ts `MAX_REDACT_MARKS`.
pub const MAX_REDACT_MARKS: usize = 2000;
/// engine.ts `MAX_FIELD_NODES` and `MAX_FIELD_DEPTH`: the field tree a
/// pasted field's name is picked against.
const MAX_FIELD_NODES: usize = 4096;
const MAX_FIELD_DEPTH: usize = 8;
/// engine.ts `MAX_FIELD_NAME_CHARS`: the longest name field-add takes.
const MAX_FIELD_NAME_CHARS: usize = 128;
/// Nesting a copied value may have; deeper parts are dropped.
const MAX_COPY_DEPTH: usize = 256;

const CLIP_KINDS: [&str; 6] = ["image", "form", "text", "field", "link", "mark"];

/// engine.ts `CLIP_PERMISSION`: the op whose permission a pasted member of
/// each kind takes.
fn clip_permission(kind: &str) -> &'static str {
    match kind {
        "image" | "form" => "edit-object",
        "text" => "text-add",
        "field" => "field-add",
        "link" => "link-add",
        _ => "redact-add",
    }
}

/// Objects copied out of a document: number `n` is `objects[n - 1]`, and
/// references inside them name these numbers.
#[derive(Default)]
pub struct Scratch {
    objects: Vec<(Object, Option<Vec<u8>>)>,
}

/// The counts, not the (up to 32 MiB of) bytes.
impl std::fmt::Debug for Scratch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let bytes: usize = (self.objects.iter())
            .map(|(_, raw)| raw.as_ref().map_or(0, Vec::len))
            .sum();
        f.debug_struct("Scratch")
            .field("objects", &self.objects.len())
            .field("stream_bytes", &bytes)
            .finish()
    }
}

/// A link member's target.
#[derive(Clone, Debug, PartialEq)]
enum LinkTarget {
    Page(i64),
    Uri(String),
}

/// engine.ts `ClipItem`; geometry in the source page's view space unless
/// noted.
#[derive(Clone, Debug)]
enum ClipItem {
    Object {
        kind: Kind,
        /// The source placement (user space); its spans mean nothing here.
        placement: Placement,
        object: u32,
    },
    Text {
        text: String,
        size: f64,
        tc: f64,
        tw: f64,
        tz: f64,
        tl: f64,
        tr: f64,
        origin: [f64; 2],
        linear: [f64; 4],
        bytes: Vec<u8>,
        font: u32,
        bounds: [f64; 4],
    },
    Field {
        /// The `fields` row it was copied from.
        item: Value,
        value: Value,
        kids: Option<Vec<(String, [f64; 4])>>,
        view: [f64; 4],
    },
    Link {
        rect: [f64; 4],
        target: LinkTarget,
    },
    Mark {
        rect: [f64; 4],
    },
}

impl ClipItem {
    fn kind(&self) -> &'static str {
        match self {
            Self::Object { kind, .. } => kind.as_str(),
            Self::Text { .. } => "text",
            Self::Field { .. } => "field",
            Self::Link { .. } => "link",
            Self::Mark { .. } => "mark",
        }
    }

    /// engine.ts `viewOf`.
    fn view(&self) -> [f64; 4] {
        match self {
            Self::Object { placement, .. } => placement.view,
            Self::Text { bounds, .. } => *bounds,
            Self::Field { view, .. } => *view,
            Self::Link { rect, .. } | Self::Mark { rect } => *rect,
        }
    }

    /// engine.ts `boxesOf`: a radio group's kids, else the view box.
    fn boxes(&self) -> Vec<[f64; 4]> {
        match self {
            Self::Field {
                kids: Some(kids), ..
            } => kids.iter().map(|(_, rect)| *rect).collect(),
            _ => vec![self.view()],
        }
    }

    /// The `occupied` list a pasted box is compared with.
    fn slot(&self) -> usize {
        match self {
            Self::Object { .. } => 0,
            Self::Text { .. } => 1,
            Self::Field { .. } => 2,
            Self::Link { .. } => 3,
            Self::Mark { .. } => 4,
        }
    }
}

/// engine.ts `Clipboard`: the members and the objects they need. Cheap to
/// clone; another session of the engine may hold the same one.
#[derive(Clone, Debug, Default)]
pub struct Clipboard {
    held: Option<Arc<Held>>,
}

#[derive(Debug)]
struct Held {
    items: Vec<ClipItem>,
    scratch: Scratch,
}

/// `Clipboard` as a value: an empty clipboard, which `clip-paste` refuses
/// as `empty-clipboard`. It keeps the unit placeholder this type replaced
/// constructible.
#[allow(non_upper_case_globals)]
pub const Clipboard: Clipboard = Clipboard { held: None };

impl Clipboard {
    fn items(&self) -> &[ClipItem] {
        self.held.as_ref().map_or(&[], |held| held.items.as_slice())
    }
    pub fn len(&self) -> usize {
        self.items().len()
    }
    pub fn is_empty(&self) -> bool {
        self.items().is_empty()
    }
}

/// engine.ts `clipKinds`.
fn kinds_of(items: &[ClipItem]) -> Value {
    let mut kinds = Map::new();
    for kind in CLIP_KINDS {
        let count = items.iter().filter(|item| item.kind() == kind).count();
        kinds.insert(kind.into(), count.into());
    }
    Value::Object(kinds)
}

/// Copies objects of one document into a scratch store, each once (one
/// graft map per copy). A reference to a page, a page-tree node or the
/// catalog becomes null, so a member never drags the document along.
#[derive(Default)]
struct Copier {
    scratch: Scratch,
    map: BTreeMap<ObjectId, u32>,
    pending: Vec<(ObjectId, u32)>,
    bytes: usize,
}

fn excluded(doc: &Document<'_>, id: ObjectId) -> bool {
    if id == doc.root() || doc.object(id).is_err() {
        return true;
    }
    let kind = doc
        .resolve_id(id)
        .as_dict()
        .map(|dict| doc.dict_get(dict, b"Type"));
    matches!(kind.and_then(Object::as_name), Some(b"Page" | b"Pages"))
}

impl Copier {
    fn slot(&mut self) -> u32 {
        self.scratch.objects.push((Object::Null, None));
        u32::try_from(self.scratch.objects.len()).unwrap_or(u32::MAX)
    }

    fn reference(&mut self, doc: &Document<'_>, id: ObjectId) -> Object {
        if let Some(&n) = self.map.get(&id) {
            return scratch_ref(n);
        }
        if excluded(doc, id) {
            return Object::Null;
        }
        let n = self.slot();
        self.map.insert(id, n);
        self.pending.push((id, n));
        scratch_ref(n)
    }

    fn value(&mut self, doc: &Document<'_>, value: &Object, depth: usize) -> Object {
        if depth > MAX_COPY_DEPTH {
            return Object::Null;
        }
        match value {
            Object::Reference(id) => self.reference(doc, *id),
            Object::Array(items) => Object::Array(
                items
                    .iter()
                    .map(|item| self.value(doc, item, depth + 1))
                    .collect(),
            ),
            Object::Dictionary(dict) => Object::Dictionary(self.dictionary(doc, dict, depth)),
            other => other.clone(),
        }
    }

    fn dictionary(&mut self, doc: &Document<'_>, dict: &Dictionary, depth: usize) -> Dictionary {
        let mut out = Dictionary::new();
        for (key, value) in dict {
            out.insert(key.clone(), self.value(doc, value, depth + 1));
        }
        out
    }

    /// engine.ts `indirectOf(scratch, graft.graftObject(value))`: the
    /// scratch number holding `value` (copied as an object when direct).
    fn root(&mut self, doc: &Document<'_>, value: &Object) -> u32 {
        if let Some(id) = value.as_reference() {
            if let Some(&n) = self.map.get(&id) {
                return n;
            }
            if doc.object(id).is_ok() {
                let n = self.slot();
                self.map.insert(id, n);
                self.pending.push((id, n));
                self.drain(doc);
                return n;
            }
        }
        let copied = self.value(doc, doc.resolve(value), 0);
        let n = self.slot();
        if let Some(slot) = self.scratch.objects.get_mut((n as usize).wrapping_sub(1)) {
            slot.0 = copied;
        }
        self.drain(doc);
        n
    }

    /// Copies everything mapped so far and what it reaches. Past the byte
    /// ceiling streams are no longer stored; the copy is then refused.
    fn drain(&mut self, doc: &Document<'_>) {
        while let Some((id, n)) = self.pending.pop() {
            let Ok(object) = doc.object(id) else {
                continue;
            };
            let raw = match object.stream {
                Some(_) => doc.stream_data(id).ok().flatten(),
                None => None,
            };
            let mut value = object.value.clone();
            if raw.is_some() {
                if let Some(dict) = value.as_dict_mut() {
                    dict.remove(b"Length".as_slice());
                }
            }
            let copied = self.value(doc, &value, 0);
            let raw = raw.and_then(|raw| {
                self.bytes = self.bytes.saturating_add(raw.len());
                (self.bytes <= MAX_CLIP_BYTES).then(|| raw.to_vec())
            });
            if let Some(slot) = self.scratch.objects.get_mut((n as usize).wrapping_sub(1)) {
                *slot = (copied, raw);
            }
        }
    }
}

fn scratch_ref(n: u32) -> Object {
    ObjectId::new(n, 0).map_or(Object::Null, Object::Reference)
}

/// Copies scratch objects into the document, each once (one graft map per
/// paste, so a soft mask two members share lands once).
struct Paster<'s> {
    scratch: &'s Scratch,
    map: BTreeMap<u32, ObjectId>,
    pending: Vec<(u32, ObjectId)>,
}

impl<'s> Paster<'s> {
    fn new(scratch: &'s Scratch) -> Self {
        Self {
            scratch,
            map: BTreeMap::new(),
            pending: Vec::new(),
        }
    }

    fn reference(&mut self, tx: &mut Transaction<'_>, n: u32) -> Result<Object, AppError> {
        if let Some(&id) = self.map.get(&n) {
            return Ok(Object::Reference(id));
        }
        if self
            .scratch
            .objects
            .get((n as usize).wrapping_sub(1))
            .is_none()
        {
            return Ok(Object::Null);
        }
        let id = tx.allocate(Object::Null, None)?;
        self.map.insert(n, id);
        self.pending.push((n, id));
        Ok(Object::Reference(id))
    }

    fn value(
        &mut self,
        tx: &mut Transaction<'_>,
        value: &Object,
        depth: usize,
    ) -> Result<Object, AppError> {
        if depth > MAX_COPY_DEPTH {
            return Ok(Object::Null);
        }
        Ok(match value {
            Object::Reference(id) => self.reference(tx, id.number())?,
            Object::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.value(tx, item, depth + 1)?);
                }
                Object::Array(out)
            }
            Object::Dictionary(dict) => {
                let mut out = Dictionary::new();
                for (key, value) in dict {
                    out.insert(key.clone(), self.value(tx, value, depth + 1)?);
                }
                Object::Dictionary(out)
            }
            other => other.clone(),
        })
    }

    /// The pasted object for scratch number `n`, with what it reaches.
    fn object(&mut self, tx: &mut Transaction<'_>, n: u32) -> Result<Object, AppError> {
        let reference = self.reference(tx, n)?;
        while let Some((n, id)) = self.pending.pop() {
            let Some((value, raw)) = self.scratch.objects.get((n as usize).wrapping_sub(1)) else {
                continue;
            };
            let value = self.value(tx, value, 0)?;
            match (raw, value) {
                (Some(raw), Object::Dictionary(dict)) => {
                    tx.put_stream_raw(id, dict, raw.clone())?
                }
                (_, value) => tx.put_value(id, value)?,
            }
        }
        Ok(reference)
    }
}

/// A member refusal tagged with its position, engine.ts `tagMember`.
fn tagged(error: AppError, extra: Value) -> AppError {
    error.with_extra(extra)
}

/// One op through the router, as engine.ts's recursive `dispatch`.
fn nested(
    engine: &mut AppEngine,
    session: &mut AppSession,
    request: Value,
) -> Result<Value, AppError> {
    crate::app::ops::route(engine, session, &request, &mut Buffers::default())
}

/// The `items` a listing op answers.
fn listed(
    engine: &mut AppEngine,
    session: &mut AppSession,
    op: &str,
    page: usize,
) -> Result<Vec<Value>, AppError> {
    let reply = nested(engine, session, json!({"op": op, "page": page}))?;
    Ok(reply
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

/// Runs `work` with the session's batch mode set to `mode`.
fn in_batch<T>(
    session: &mut AppSession,
    mode: Batch,
    work: impl FnOnce(&mut AppSession) -> T,
) -> T {
    let previous = std::mem::replace(&mut session.batch, mode);
    let out = work(session);
    session.batch = previous;
    out
}

fn rect4(value: Option<&Value>) -> Option<[f64; 4]> {
    let items = value?.as_array()?;
    let numbers: Option<Vec<f64>> = items.iter().map(request::as_number).collect();
    <[f64; 4]>::try_from(numbers?).ok()
}

fn resource<'d>(
    doc: &'d Document<'_>,
    resources: Option<&'d Dictionary>,
    category: &[u8],
    name: &[u8],
) -> Option<&'d Object> {
    let dict = super::font::dict_at(doc, resources, category)?;
    dict.get(name).filter(|v| !doc.resolve(v).is_null())
}

/// One requested member.
enum Target {
    Content(Kind, String),
    Number(&'static str, i64),
}

/// engine.ts `shapeless`: 1..512 distinct `{kind, id}`.
fn targets(value: Option<&Value>) -> Result<Vec<Target>, AppError> {
    let refused = || {
        edit_error(
            "invalid-selection",
            format!("clip-copy: targets must be 1..{MAX_EDIT_TARGETS} distinct {{ kind, id }}"),
        )
    };
    let list = value
        .and_then(Value::as_array)
        .filter(|l| (1..=MAX_EDIT_TARGETS).contains(&l.len()))
        .ok_or_else(refused)?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(list.len());
    for raw in list {
        let kind = raw.get("kind").and_then(Value::as_str).unwrap_or("");
        let Some(&kind) = CLIP_KINDS.iter().find(|k| **k == kind) else {
            return Err(refused());
        };
        let id = raw.get("id");
        let target = match kind {
            "image" | "form" | "text" => {
                let id = id.and_then(Value::as_str).filter(|s| !s.is_empty());
                let kind = match kind {
                    "image" => Kind::Image,
                    "form" => Kind::Form,
                    _ => Kind::Text,
                };
                Target::Content(kind, id.ok_or_else(refused)?.to_string())
            }
            _ => Target::Number(kind, id.and_then(request::as_int).ok_or_else(refused)?),
        };
        let key = match &target {
            Target::Content(_, id) => format!("content:{id}"),
            Target::Number(kind, _) => format!("{kind}:{}", js_string(id)),
        };
        if !seen.insert(key) {
            return Err(refused());
        }
        out.push(target);
    }
    Ok(out)
}

/// engine.ts `isStructuralRadioParent`.
fn structural_radio_parent(doc: &Document<'_>, parent: &Dictionary) -> bool {
    if super::font::at(doc, Some(parent), b"Parent").is_some() {
        return false;
    }
    if super::font::at(doc, Some(parent), b"FT").and_then(Object::as_name) != Some(b"Btn") {
        return false;
    }
    let flags = super::font::number(super::font::at(doc, Some(parent), b"Ff")).unwrap_or(0.0);
    let flags = if flags.is_finite() { flags as i64 } else { 0 };
    if flags & (1 << 15) == 0 || flags & (1 << 16) != 0 {
        return false;
    }
    let Some(kids) = super::font::at(doc, Some(parent), b"Kids").and_then(Object::as_array) else {
        return false;
    };
    !kids.is_empty()
        && kids.iter().all(|kid| {
            let kid = doc.resolve(kid).as_dict();
            super::font::at(doc, kid, b"T").is_none()
                && super::font::at(doc, kid, b"Subtype").and_then(Object::as_name)
                    == Some(b"Widget")
        })
}

/// engine.ts `fieldOn`: the first /AP /N state that is not Off.
fn field_on(doc: &Document<'_>, kid: &Dictionary) -> String {
    let normal = super::font::dict_at(doc, super::font::dict_at(doc, Some(kid), b"AP"), b"N");
    normal
        .and_then(|states| {
            states
                .keys()
                .find(|key| key.as_slice() != b"Off")
                .map(|key| super::font::latin1(key))
        })
        .unwrap_or_default()
}

/// engine.ts `clip-copy`: members into a fresh clipboard; nothing written,
/// and a refusal keeps the old clipboard.
pub fn copy(
    engine: &mut AppEngine,
    session: &mut AppSession,
    request: &Value,
) -> Result<Value, AppError> {
    let op = "clip-copy";
    let Some(doc) = &session.doc else {
        return Err(AppError::reason(
            "no-document",
            "clip-copy: no document open",
        ));
    };
    if session.locked {
        return Err(AppError::reason(
            "read-only",
            "clip-copy: this document is read-only until it is unlocked",
        ));
    }
    let info = doc.info(session.locked);
    if !Need::Copy.allowed(&info) {
        return Err(AppError::reason(
            "read-only",
            "clip-copy: this document does not permit copying",
        ));
    }
    let page = session.check_page(request.get("page"), op)?;
    let list = targets(request.get("targets"))?;
    let mut copier = Copier::default();
    let mut items = Vec::with_capacity(list.len());
    let mut fields: Option<Vec<Value>> = None;
    let mut links: Option<Vec<Value>> = None;
    let mut annots: Option<Vec<Value>> = None;
    let mut radios = BTreeSet::new();
    for target in list {
        match target {
            Target::Content(kind, id) => {
                let item = copy_content(session, request, page, kind, &id, &mut copier)?;
                items.push(item);
            }
            Target::Number("field", number) => {
                if fields.is_none() {
                    fields = Some(listed(engine, session, "fields", page)?);
                }
                let rows = fields.as_deref().unwrap_or_default();
                if let Some(item) = copy_field(session, page, number, rows, info.xfa, &mut radios)?
                {
                    items.push(item);
                }
            }
            Target::Number("link", number) => {
                let extra = json!({"annotId": number});
                if number == 0 {
                    return Err(tagged(
                        edit_error(
                            "not-candidate",
                            "clip-copy: a link without its own object cannot be copied",
                        ),
                        extra,
                    ));
                }
                if links.is_none() {
                    links = Some(listed(engine, session, "links", page)?);
                }
                let rows = links.as_deref().unwrap_or_default();
                let Some(row) = rows
                    .iter()
                    .find(|row| row.get("annotId").and_then(request::as_int) == Some(number))
                else {
                    return Err(tagged(
                        edit_error(
                            "stale-target",
                            format!("clip-copy: no link {number} on page {page}"),
                        ),
                        extra,
                    ));
                };
                let rect = rect4(row.get("rect"));
                let target = match (row.get("kind").and_then(Value::as_str), rect) {
                    (Some("page"), Some(_)) => row
                        .get("page")
                        .and_then(request::as_int)
                        .map(LinkTarget::Page),
                    (Some("uri"), Some(_)) => row
                        .get("uri")
                        .and_then(Value::as_str)
                        .map(|uri| LinkTarget::Uri(uri.to_string())),
                    _ => None,
                };
                let (Some(rect), Some(target)) = (rect, target) else {
                    return Err(tagged(
                        edit_error(
                            "not-candidate",
                            "clip-copy: a blocked link cannot be copied",
                        ),
                        extra,
                    ));
                };
                items.push(ClipItem::Link { rect, target });
            }
            Target::Number(_, number) => {
                let extra = json!({"annotId": number});
                if annots.is_none() {
                    annots = Some(listed(engine, session, "annots", page)?);
                }
                let rows = annots.as_deref().unwrap_or_default();
                let Some(row) = rows
                    .iter()
                    .find(|row| row.get("id").and_then(request::as_int) == Some(number))
                else {
                    return Err(tagged(
                        edit_error(
                            "stale-target",
                            format!("clip-copy: no annotation {number} on page {page}"),
                        ),
                        extra,
                    ));
                };
                let rect = rect4(row.get("rect"));
                let (Some("Redact"), Some(rect)) = (row.get("type").and_then(Value::as_str), rect)
                else {
                    return Err(tagged(
                        edit_error(
                            "not-candidate",
                            "clip-copy: only a redaction mark can be copied",
                        ),
                        extra,
                    ));
                };
                items.push(ClipItem::Mark { rect });
            }
        }
    }
    if copier.bytes > MAX_CLIP_BYTES {
        return Err(edit_error(
            "too-large",
            format!(
                "clip-copy: the copied objects are over the {MAX_CLIP_BYTES}-byte clipboard ceiling"
            ),
        ));
    }
    let count = items.len();
    let kinds = kinds_of(&items);
    session.clipboard = Some(Clipboard {
        held: Some(Arc::new(Held {
            items,
            scratch: copier.scratch,
        })),
    });
    let journal = session.journal_state();
    Ok(reply(
        request,
        json!({"count": count, "kinds": kinds, "revision": journal.revision,
            "dirty": journal.dirty}),
    ))
}

/// One image, Form or text member.
fn copy_content(
    session: &mut AppSession,
    request: &Value,
    page: usize,
    kind: Kind,
    id: &str,
    copier: &mut Copier,
) -> Result<ClipItem, AppError> {
    let extra = json!({"targetId": id});
    let single = json!({
        "generation": request.get("generation"),
        "revision": request.get("revision"),
        "page": page,
        "targetId": id,
    });
    let (scan, index) = resolve_target(session, &single, page).map_err(|error| {
        let detail = match error.reason.as_deref() {
            Some("too-large") => "too-large",
            _ => "stale-target",
        };
        tagged(
            edit_error(detail, format!("clip-copy: {}", error.message)),
            extra.clone(),
        )
    })?;
    let Some(record) = scan.records.get(index) else {
        return Err(tagged(
            edit_error("stale-target", "clip-copy: the target is gone"),
            extra,
        ));
    };
    let refuse = |why: &str| {
        let mut extra = json!({"targetId": id});
        if let Some(reason) = record.reason {
            extra["reason"] = reason.into();
        }
        tagged(
            edit_error(
                "not-candidate",
                format!("clip-copy: {id} cannot be copied ({why})"),
            ),
            extra,
        )
    };
    if record.kind != kind || !record.candidate {
        return Err(refuse(record.reason.unwrap_or("not a candidate")));
    }
    if record.depth != 0 || record.oc_depth != 0 {
        return Err(refuse("inside a Form or a layer"));
    }
    let Some(app) = session.doc.as_ref() else {
        return Err(refuse("not a candidate"));
    };
    let doc = app.document();
    let Some((_, page_dict)) = app.page(page) else {
        return Err(refuse("not a candidate"));
    };
    let resources = page_resources(doc, page_dict);
    if kind == Kind::Text {
        if record.glyph_line.is_some() {
            return Err(refuse("a print-to-PDF line"));
        }
        let (
            Some(text),
            Some(origin),
            Some(linear),
            Some(bytes),
            Some(tf_name),
            Some(size),
            Some(bounds),
        ) = (
            &record.text,
            record.origin,
            record.text_linear,
            &record.payload,
            &record.tf_name,
            record.tf_size,
            record.bounds,
        )
        else {
            return Err(refuse("a run that continues a line"));
        };
        let font = resource(doc, resources, b"Font", tf_name)
            .filter(|font| doc.resolve(font).as_dict().is_some())
            .ok_or_else(|| refuse("its font does not resolve"))?;
        let font_dict = doc.resolve(font).as_dict();
        let type3 = record
            .font
            .as_ref()
            .is_some_and(|f| f.subtype.as_deref() == Some("Type3"))
            || super::font::at(doc, font_dict, b"Subtype").and_then(Object::as_name)
                == Some(b"Type3");
        if type3 {
            return Err(refuse("type3"));
        }
        let font = copier.root(doc, font);
        return Ok(ClipItem::Text {
            text: text.clone(),
            size,
            tc: record.character_spacing,
            tw: record.word_spacing,
            tz: record.horizontal_scale * 100.0,
            tl: record.leading.unwrap_or(0.0),
            tr: record.render_mode,
            origin,
            linear,
            bytes: bytes.clone(),
            font,
            bounds,
        });
    }
    let (Some(placement), Some(_), Some(name)) =
        (&record.placement, record.element, &record.object_name)
    else {
        return Err(refuse("not a top-level simple placement"));
    };
    if record.image.as_ref().is_some_and(|image| image.oc) {
        return Err(refuse("on a layer"));
    }
    let Some(xobject) = resource(doc, resources, b"XObject", name) else {
        return Err(tagged(
            edit_error(
                "stale-target",
                format!("clip-copy: {id} no longer resolves"),
            ),
            extra,
        ));
    };
    let object = copier.root(doc, xobject);
    Ok(ClipItem::Object {
        kind,
        placement: placement.clone(),
        object,
    })
}

/// One field member: a top-level field, or a whole structural radio group
/// (once, whichever kid is named), its kids all on this page.
fn copy_field(
    session: &AppSession,
    page: usize,
    number: i64,
    rows: &[Value],
    xfa: bool,
    radios: &mut BTreeSet<ObjectId>,
) -> Result<Option<ClipItem>, AppError> {
    let extra = json!({"widgetId": number});
    let row_of = |id: i64| {
        rows.iter()
            .find(|row| row.get("id").and_then(request::as_int) == Some(id))
    };
    let Some(row) = row_of(number) else {
        return Err(tagged(
            edit_error(
                "stale-target",
                format!("clip-copy: no form field {number} on page {page}"),
            ),
            extra,
        ));
    };
    let refuse = |why: &str| {
        tagged(
            edit_error(
                "not-candidate",
                format!("clip-copy: field {number} cannot be copied ({why})"),
            ),
            json!({"widgetId": number}),
        )
    };
    if xfa {
        return Err(refuse("an XFA form"));
    }
    if row.get("scripted").and_then(Value::as_bool) == Some(true) {
        return Err(refuse("scripted"));
    }
    let kind = row.get("kind").and_then(Value::as_str).unwrap_or("");
    if matches!(kind, "signature" | "list") {
        return Err(refuse(kind));
    }
    let name = row.get("name").and_then(Value::as_str).unwrap_or("");
    if name.contains('.') {
        return Err(refuse("part of a field hierarchy"));
    }
    let Some(app) = session.doc.as_ref() else {
        return Err(refuse("part of a field hierarchy"));
    };
    let doc = app.document();
    let widget = u32::try_from(number)
        .ok()
        .and_then(|n| ObjectId::new(n, 0).ok())
        .and_then(|id| doc.resolve_id(id).as_dict());
    let parent_value = widget.and_then(|w| w.get(b"Parent".as_slice()));
    let parent = parent_value.and_then(|p| doc.resolve(p).as_dict());
    if parent_value.is_some_and(|p| !doc.resolve(p).is_null())
        && !parent.is_some_and(|p| structural_radio_parent(doc, p))
    {
        return Err(refuse("part of a field hierarchy"));
    }
    let view = rect4(row.get("rect")).unwrap_or_default();
    if kind != "radio" {
        return Ok(Some(ClipItem::Field {
            item: row.clone(),
            value: row.get("value").cloned().unwrap_or(Value::Null),
            kids: None,
            view,
        }));
    }
    let (Some(parent), Some(group)) = (parent, parent_value.and_then(Object::as_reference)) else {
        return Err(refuse("a radio button without a group"));
    };
    if !radios.insert(group) {
        return Ok(None);
    }
    let kids = super::font::at(doc, Some(parent), b"Kids")
        .and_then(Object::as_array)
        .unwrap_or_default();
    if kids.len() > rows.len() {
        return Err(refuse("the group has a kid on another page"));
    }
    let mut found: Vec<(i64, String, [f64; 4], bool)> = Vec::new();
    for kid in kids {
        let id = kid.as_reference().map_or(-1, |id| i64::from(id.number()));
        let Some(field) = row_of(id) else {
            return Err(refuse("the group has a kid on another page"));
        };
        let dict = doc.resolve(kid).as_dict();
        let on = match super::font::at(doc, dict, b"AS").and_then(Object::as_name) {
            Some(state) if state != b"Off" => super::font::latin1(state),
            _ => dict.map(|d| field_on(doc, d)).unwrap_or_default(),
        };
        let rect = rect4(field.get("rect")).unwrap_or_default();
        let checked = field.get("value") == Some(&Value::Bool(true));
        found.push((id, on, rect, checked));
    }
    if found.len() < 2 {
        return Err(refuse("the group has fewer than two kids"));
    }
    let names: BTreeSet<&str> = found.iter().map(|(_, on, ..)| on.as_str()).collect();
    if found.iter().any(|(_, on, ..)| on.is_empty()) || names.len() != found.len() {
        return Err(refuse("its kid names are not unique"));
    }
    found.sort_by_key(|(id, ..)| *id != number);
    let value = found
        .iter()
        .find(|(.., checked)| *checked)
        .map_or(String::new(), |(_, on, ..)| on.clone());
    let kids: Vec<(String, [f64; 4])> = found
        .into_iter()
        .map(|(_, on, rect, _)| (on, rect))
        .collect();
    let view = union(kids.iter().map(|(_, rect)| *rect)).unwrap_or(view);
    Ok(Some(ClipItem::Field {
        item: row.clone(),
        value: Value::String(value),
        kids: Some(kids),
        view,
    }))
}

/// engine.ts `unionOf`.
fn union(boxes: impl IntoIterator<Item = [f64; 4]>) -> Option<[f64; 4]> {
    boxes.into_iter().reduce(|a, b| {
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]
    })
}

fn shift(rect: &[f64; 4], dx: f64, dy: f64) -> [f64; 4] {
    [rect[0] + dx, rect[1] + dy, rect[2] + dx, rect[3] + dy]
}

fn apply(m: &[f64; 6], x: f64, y: f64) -> [f64; 2] {
    [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]
}

/// A pasted image or Form placement.
struct ObjectPaste {
    index: usize,
    kind: &'static str,
    object: u32,
    name: String,
    cm: String,
    clip_text: Option<String>,
    full: [f64; 4],
}

/// A pasted run.
struct TextPaste {
    index: usize,
    font: u32,
    font_name: String,
    block: String,
    text: String,
    origin: [f64; 2],
}

/// The first `<prefix><n>` (from `next`) the resources do not hold.
fn free_name(names: Option<&Dictionary>, prefix: &str, next: &mut u64) -> String {
    loop {
        let name = format!("{prefix}{next}");
        *next = next.saturating_add(1);
        if !names.is_some_and(|d| d.contains_key(name.as_bytes())) || *next == u64::MAX {
            return name;
        }
    }
}

/// A full name in the field tree walk: its text while a pasted name can
/// still equal it (at most `MAX_FIELD_NAME_CHARS` UTF-16 units; field-add
/// refuses longer names), else only that it is longer.
#[derive(Clone)]
enum TreeName {
    Short(String),
    Long,
}

/// A /T longer than this decodes to more than `MAX_FIELD_NAME_CHARS` UTF-16
/// units in any text string encoding.
const MAX_NAME_BYTES: usize = 4 * MAX_FIELD_NAME_CHARS + 4;

/// engine.ts `fieldTreeNames`: every fully qualified name in /AcroForm
/// /Fields, `too-many` past `MAX_FIELD_NODES` visited nodes; nodes deeper
/// than `MAX_FIELD_DEPTH` are not visited. Each node's /Kids are walked one
/// at a time and only names a pasted name can equal are kept, so work and
/// memory stay bounded by the visited nodes whatever the /T lengths and
/// /Kids sizes (the forms group's walk; after its merge, call that one).
fn field_tree_names(doc: &Document<'_>) -> Result<BTreeSet<String>, AppError> {
    let mut names = BTreeSet::new();
    let catalog = doc.resolve_id(doc.root()).as_dict();
    let form = super::font::dict_at(doc, catalog, b"AcroForm");
    let Some(fields) = super::font::at(doc, form, b"Fields").and_then(Object::as_array) else {
        return Ok(names);
    };
    let mut visited = 0usize;
    // The kids being walked, the next one, their prefix and their depth.
    let mut stack: Vec<(&[Object], usize, TreeName, usize)> =
        vec![(fields, 0, TreeName::Short(String::new()), 1)];
    while let Some(frame) = stack.last_mut() {
        let Some(node) = frame.0.get(frame.1) else {
            stack.pop();
            continue;
        };
        frame.1 += 1;
        let (prefix, depth) = (frame.2.clone(), frame.3);
        visited += 1;
        if visited > MAX_FIELD_NODES {
            return Err(edit_error(
                "too-many",
                "field-add: this form is too large to design fields in",
            ));
        }
        let dict = doc.resolve(node).as_dict();
        let own = super::font::at(doc, dict, b"T").and_then(|t| match t {
            Object::String(bytes) => Some(bytes.as_slice()),
            _ => None,
        });
        let own_name = own.map(|own| match own.len() > MAX_NAME_BYTES {
            true => TreeName::Long,
            false => TreeName::Short(crate::object::decode_text_string(own)),
        });
        let full = match (own_name, prefix) {
            (Some(TreeName::Short(own)), prefix) if own.is_empty() => prefix,
            (Some(TreeName::Short(own)), TreeName::Short(prefix)) => {
                let joined = match prefix.is_empty() {
                    true => own,
                    false => format!("{prefix}.{own}"),
                };
                match super::js_len(&joined) > MAX_FIELD_NAME_CHARS {
                    true => TreeName::Long,
                    false => TreeName::Short(joined),
                }
            }
            (Some(_), _) => TreeName::Long,
            (None, prefix) => prefix,
        };
        if let (Some(_), TreeName::Short(full)) = (own, &full)
            && !full.is_empty()
        {
            names.insert(full.clone());
        }
        // Kids past the depth bound are never visited, so never walked.
        if depth < MAX_FIELD_DEPTH
            && let Some(kids) = super::font::at(doc, dict, b"Kids").and_then(Object::as_array)
        {
            stack.push((kids, 0, full, depth + 1));
        }
    }
    Ok(names)
}

/// Redact annotations with a rect, over the whole document (redact.ts
/// `redactMarks`).
fn redact_marks(doc: &Document<'_>) -> usize {
    let mut count = 0usize;
    for page in crate::app::info::pages(doc) {
        let annots = super::font::at(doc, Some(page.dict), b"Annots").and_then(Object::as_array);
        for annot in annots.unwrap_or_default() {
            let dict = doc.resolve(annot).as_dict();
            let redact =
                super::font::at(doc, dict, b"Subtype").and_then(Object::as_name) == Some(b"Redact");
            let rect = super::font::at(doc, dict, b"Rect")
                .and_then(Object::as_array)
                .is_some_and(|r| r.len() == 4);
            if redact && rect {
                count += 1;
            }
        }
    }
    count
}

/// The annotation ids a listing op answers for one kind of member.
fn listed_ids(
    engine: &mut AppEngine,
    session: &mut AppSession,
    kind: &str,
    page: usize,
) -> Result<BTreeSet<i64>, AppError> {
    let (op, key) = match kind {
        "link" => ("links", "annotId"),
        _ => ("annots", "id"),
    };
    let rows = listed(engine, session, op, page)?;
    Ok(rows
        .iter()
        .filter(|row| kind == "link" || row.get("type").and_then(Value::as_str) == Some("Redact"))
        .filter_map(|row| row.get(key).and_then(request::as_int))
        .filter(|&id| id > 0)
        .collect())
}

/// One pasted field: engine.ts `addField` with the copied value. Until the
/// forms op group is in this tree this is its `field-add` op, which takes
/// no value, so the copy starts at its default. With it, this is
/// `fields::add_field(session, spec, Some(value))`.
fn add_field(
    engine: &mut AppEngine,
    session: &mut AppSession,
    spec: &Value,
    _value: &Value,
) -> Result<Value, AppError> {
    nested(engine, session, spec.clone())
}

/// engine.ts `clip-paste`: every member checked first, then one journal
/// step.
pub fn paste(
    engine: &mut AppEngine,
    session: &mut AppSession,
    request: &Value,
) -> Result<Value, AppError> {
    let op = "clip-paste";
    session.require_mutable(op)?;
    let page = session.check_page(request.get("page"), op)?;
    let Some(clipboard) = session.clipboard.clone().filter(|c| !c.is_empty()) else {
        return Err(edit_error(
            "empty-clipboard",
            "clip-paste: the clipboard is empty",
        ));
    };
    let point = match request.get("at") {
        None => None,
        Some(value) => {
            let numbers = value
                .as_array()
                .filter(|v| v.len() == 2)
                .and_then(|v| v.iter().map(request::as_number).collect::<Option<Vec<_>>>());
            match numbers.as_deref() {
                Some(&[x, y]) => Some([x, y]),
                _ => {
                    return Err(edit_error(
                        "out-of-bounds",
                        "clip-paste: at must be two finite numbers",
                    ));
                }
            }
        }
    };
    let Some(held) = clipboard.held.clone() else {
        return Err(edit_error(
            "empty-clipboard",
            "clip-paste: the clipboard is empty",
        ));
    };
    let items = held.items.as_slice();
    super::require_each(
        session,
        items.iter().map(|item| clip_permission(item.kind())),
    )
    .map_err(|(index, error)| {
        let kind = items.get(index).map_or("", ClipItem::kind);
        tagged(error, json!({"index": index, "kind": kind}))
    })?;
    let transform = session
        .view()?
        .page_transform(page)
        .map_err(|error| AppError::plain(format!("clip-paste: {error}")))?;
    let (clip_pdf, geometry_blocked, page_size) = {
        let doc = session
            .doc
            .as_ref()
            .ok_or_else(|| AppError::plain("clip-paste: no document open"))?;
        let (_, dict) = doc
            .page(page)
            .ok_or_else(|| AppError::plain("clip-paste: no such page"))?;
        let document = doc.document();
        let rotate = pagetree::inherited(document, dict, b"Rotate")
            .and_then(|v| super::font::number(Some(document.resolve(v))));
        let unit = super::font::number(super::font::at(document, Some(dict), b"UserUnit"));
        let blocked =
            rotate.is_some_and(|r| r.trunc() % 360.0 != 0.0) || unit.is_some_and(|u| u != 1.0);
        (
            page_clip_box(document, dict),
            blocked,
            crate::app::info::page_size(document, dict),
        )
    };
    let inverse = invert(&[transform[0], transform[1], transform[2], transform[3]])
        .map(|lin| {
            [
                lin[0],
                lin[1],
                lin[2],
                lin[3],
                -(transform[4] * lin[0] + transform[5] * lin[2]),
                -(transform[4] * lin[1] + transform[5] * lin[3]),
            ]
        })
        .ok_or_else(|| {
            edit_error(
                "unsupported-transform",
                "clip-paste: the page transform is not invertible",
            )
        })?;
    let content = items
        .iter()
        .any(|item| matches!(item, ClipItem::Object { .. } | ClipItem::Text { .. }));
    let scan = if content {
        Some(scan_for(session, page)?)
    } else {
        None
    };
    let has = |slot: usize| items.iter().any(|item| item.slot() == slot);
    let mut occupied: [Vec<[f64; 4]>; 5] = Default::default();
    if let Some(scan) = &scan {
        for record in &scan.records {
            match (&record.placement, record.kind) {
                (Some(p), Kind::Image | Kind::Form) if record.candidate => occupied[0].push(p.view),
                _ => {}
            }
            if let (Kind::Text, Some(bounds)) = (record.kind, record.bounds) {
                occupied[1].push(bounds);
            }
        }
    }
    for (slot, op, key) in [
        (2, "fields", None),
        (3, "links", None),
        (4, "annots", Some("Redact")),
    ] {
        if !has(slot) {
            continue;
        }
        for row in listed(engine, session, op, page)? {
            if key.is_some() && row.get("type").and_then(Value::as_str) != key {
                continue;
            }
            if let Some(rect) = rect4(row.get("rect")) {
                occupied[slot].push(rect);
            }
        }
    }
    let views = items
        .iter()
        .map(ClipItem::view)
        .chain(items.iter().flat_map(ClipItem::boxes));
    let Some(u) = union(views) else {
        return Err(edit_error(
            "empty-clipboard",
            "clip-paste: the clipboard is empty",
        ));
    };
    let c = map_rect(&transform, &clip_pdf);
    let (mut dx, mut dy) = (0.0, 0.0);
    if let Some([x, y]) = point {
        let [vx, vy] = apply(&transform, x, y);
        dx = vx - u[0];
        dy = vy - u[1];
    } else {
        let same = |a: &[f64; 4], b: &[f64; 4]| a.iter().zip(b).all(|(l, r)| (l - r).abs() <= 0.5);
        let taken = |dx: f64, dy: f64| {
            items.iter().any(|item| {
                item.boxes().iter().any(|b| {
                    let moved = shift(b, dx, dy);
                    occupied[item.slot()]
                        .iter()
                        .any(|other| same(&moved, other))
                })
            })
        };
        for _ in 0..20 {
            if !taken(dx, dy) {
                break;
            }
            dx += if u[2] + dx + 10.0 > c[2] { -10.0 } else { 10.0 };
            dy += if u[3] + dy + 10.0 > c[3] { -10.0 } else { 10.0 };
        }
    }
    if u[2] - u[0] > c[2] - c[0] + 1e-6 || u[3] - u[1] > c[3] - c[1] + 1e-6 {
        return Err(edit_error(
            "out-of-bounds",
            "clip-paste: the clipboard is larger than the page",
        ));
    }
    dx = dx.max(c[0] - u[0]).min(c[2] - u[2]);
    dy = dy.max(c[1] - u[1]).min(c[3] - u[3]);
    let (xobjects, fonts) = {
        let doc = session
            .doc
            .as_ref()
            .ok_or_else(|| AppError::plain("clip-paste: no document open"))?;
        let (_, dict) = doc
            .page(page)
            .ok_or_else(|| AppError::plain("clip-paste: no such page"))?;
        let document = doc.document();
        let resources = page_resources(document, dict);
        (
            super::font::dict_at(document, resources, b"XObject").cloned(),
            super::font::dict_at(document, resources, b"Font").cloned(),
        )
    };
    let snap = |v: f64| js_round(v * 1e6) / 1e6;
    let mut objects: Vec<ObjectPaste> = Vec::new();
    let mut texts: Vec<TextPaste> = Vec::new();
    let (mut next_object, mut next_font) = (1u64, 1u64);
    for (index, item) in items.iter().enumerate() {
        let tag = |error: AppError| tagged(error, json!({"index": index, "kind": item.kind()}));
        match item {
            ClipItem::Text {
                text,
                size,
                tc,
                tw,
                tz,
                tl,
                tr,
                origin,
                linear,
                bytes,
                font,
                ..
            } => {
                let linear = multiply(linear, &[inverse[0], inverse[1], inverse[2], inverse[3]]);
                let origin = [origin[0] + dx, origin[1] + dy];
                let [ox, oy] = apply(&inverse, origin[0], origin[1]);
                let numbers = [*size, *tc, *tw, *tz, *tl, *tr, ox, oy];
                if !numbers.iter().chain(&linear).all(|&v| writable(v)) {
                    return Err(tag(edit_error(
                        "unsupported-transform",
                        "clip-paste: a pasted run cannot be placed on this page",
                    )));
                }
                let font_name = free_name(fonts.as_ref(), "PF", &mut next_font);
                let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                let n = edit_number;
                let block = format!(
                    "\nq BT /{font_name} {} Tf {} Tc {} Tw {} Tz {} TL 0 Ts {} Tr 0 g 0 G 1 w 0 J 0 j [] 0 d {} {} {} Tm <{hex}> Tj ET Q",
                    n(*size),
                    n(*tc),
                    n(*tw),
                    n(*tz),
                    n(*tl),
                    n(*tr),
                    linear.iter().map(|&v| n(v)).collect::<Vec<_>>().join(" "),
                    n(ox),
                    n(oy),
                );
                texts.push(TextPaste {
                    index,
                    font: *font,
                    font_name,
                    block,
                    text: text.clone(),
                    origin,
                });
            }
            ClipItem::Object {
                kind,
                placement,
                object,
            } => {
                if geometry_blocked {
                    return Err(tag(edit_error(
                        "unsupported-transform",
                        "clip-paste: rotated or scaled pages take no pasted objects",
                    )));
                }
                check_area(placement).map_err(tag)?;
                let mapped = map_rect(&inverse, &shift(&placement.view, dx, dy)).map(snap);
                let visible = [
                    mapped[0].max(clip_pdf[0]),
                    mapped[1].max(clip_pdf[1]),
                    mapped[2].min(clip_pdf[2]),
                    mapped[3].min(clip_pdf[3]),
                ];
                check_box(&visible, &clip_pdf).map_err(tag)?;
                let after = placement_after(
                    placement,
                    &rect_affine(&placement.visible, &visible),
                    Some(visible),
                );
                check_after(&after).map_err(tag)?;
                objects.push(ObjectPaste {
                    index,
                    kind: kind.as_str(),
                    object: *object,
                    name: free_name(xobjects.as_ref(), "PA", &mut next_object),
                    cm: after.cm,
                    clip_text: placement.clip.as_ref().and(after.clip_text),
                    full: after.full,
                });
            }
            _ => {}
        }
    }
    if let Some(scan) = &scan {
        if !scan.balanced {
            return Err(edit_error(
                "unbalanced-content",
                "clip-paste: existing page content has unbalanced graphics state",
            ));
        }
        if scan.records.len() + objects.len() + texts.len() > MAX_EDIT_TARGETS {
            return Err(edit_error(
                "too-large",
                format!(
                    "clip-paste: the paste would put more than {MAX_EDIT_TARGETS} targets on this page"
                ),
            ));
        }
    }
    let moved = |rect: &[f64; 4]| shift(rect, dx, dy);
    // Fields: names picked against the tree and this paste, each add
    // checked now (a plan) and written in the step.
    let mut field_adds: Vec<(usize, String, Value, Value)> = Vec::new();
    if items
        .iter()
        .any(|item| matches!(item, ClipItem::Field { .. }))
    {
        let mut taken = {
            let doc = session
                .doc
                .as_ref()
                .ok_or_else(|| AppError::plain("clip-paste: no document open"))?;
            field_tree_names(doc.document())?
        };
        for (index, item) in items.iter().enumerate() {
            let ClipItem::Field {
                item: row,
                value,
                kids,
                ..
            } = item
            else {
                continue;
            };
            let tag = |error: AppError| tagged(error, json!({"index": index, "kind": "field"}));
            let source = row.get("name").and_then(Value::as_str).unwrap_or("");
            let mut name = source.to_string();
            let mut n = 1u64;
            while taken.contains(&name) && n < u64::MAX {
                name = if n == 1 {
                    format!("{source} copy")
                } else {
                    format!("{source} copy {n}")
                };
                n += 1;
            }
            taken.insert(name.clone());
            let spec = field_spec(page, row, &name, kids.as_deref(), &moved);
            in_batch(session, Batch::Plan, |session| {
                add_field(engine, session, &spec, value)
            })
            .map_err(tag)?;
            field_adds.push((index, name, spec, value.clone()));
        }
    }
    let page_count = session.doc.as_ref().map_or(0, |doc| doc.page_count());
    let mut skipped = Vec::new();
    let mut link_adds: Vec<(usize, Value)> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let ClipItem::Link { rect, target } = item else {
            continue;
        };
        let target = match target {
            LinkTarget::Page(p) if usize::try_from(*p).map_or(true, |p| p >= page_count) => {
                skipped.push(json!({"index": index, "kind": "link", "reason": "no-such-page"}));
                continue;
            }
            LinkTarget::Page(p) => json!({"page": p}),
            LinkTarget::Uri(uri) => json!({"uri": uri}),
        };
        let spec = json!({"id": request.get("id"), "op": "link-add", "page": page,
            "rect": moved(rect), "target": target});
        in_batch(session, Batch::Plan, |session| {
            nested(engine, session, spec.clone())
        })
        .map_err(|error| tagged(error, json!({"index": index, "kind": "link"})))?;
        link_adds.push((index, spec));
    }
    if !skipped.is_empty() && skipped.len() == items.len() {
        return Err(edit_error(
            "no-such-page",
            "clip-paste: every link names a page this document does not have",
        )
        .with_extra(json!({"index": 0, "kind": "link"})));
    }
    let mut mark_adds: Vec<(usize, Value)> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let ClipItem::Mark { rect } = item else {
            continue;
        };
        let rect = redact_rect(&moved(rect), page_size)
            .map_err(|error| tagged(error, json!({"index": index, "kind": "mark"})))?;
        mark_adds.push((
            index,
            json!({"id": request.get("id"), "op": "redact-add", "page": page, "rect": rect}),
        ));
    }
    if !mark_adds.is_empty() {
        let existing = session
            .doc
            .as_ref()
            .map_or(0, |doc| redact_marks(doc.document()));
        let room = MAX_REDACT_MARKS.saturating_sub(existing);
        if mark_adds.len() > room {
            let index = mark_adds.get(room).map_or(0, |(index, _)| *index);
            return Err(edit_error(
                "too-many",
                format!("clip-paste: at most {MAX_REDACT_MARKS} marks"),
            )
            .with_extra(json!({"index": index, "kind": "mark"})));
        }
    }
    // Each mark checked as a plan too, so a refusal of redact-add's own
    // comes before the step.
    for (index, spec) in &mark_adds {
        in_batch(session, Batch::Plan, |session| {
            nested(engine, session, spec.clone())
        })
        .map_err(|error| tagged(error, json!({"index": index, "kind": "mark"})))?;
    }
    let links_before = match link_adds.is_empty() {
        true => BTreeSet::new(),
        false => listed_ids(engine, session, "link", page)?,
    };
    let marks_before = match mark_adds.is_empty() {
        true => BTreeSet::new(),
        false => listed_ids(engine, session, "mark", page)?,
    };
    let mut target_ids: Vec<String> = Vec::new();
    let mut field_ids: Vec<Value> = Vec::new();
    session.mutate("paste", |session| {
        if !objects.is_empty() || !texts.is_empty() {
            let path = write_content(session, page, &held.scratch, &objects, &texts)?;
            target_ids = prove(session, page, &path, &objects, &texts)?;
        }
        in_batch(session, Batch::Run, |session| {
            for (index, _, spec, value) in &field_adds {
                let added = add_field(engine, session, spec, value)
                    .map_err(|error| tagged(error, json!({"index": index, "kind": "field"})))?;
                match added.get("widgetIds").and_then(Value::as_array) {
                    Some(ids) => field_ids.extend(ids.iter().cloned()),
                    None => field_ids.extend(added.get("widgetId").cloned()),
                }
            }
            for (index, spec) in &link_adds {
                nested(engine, session, spec.clone())
                    .map_err(|error| tagged(error, json!({"index": index, "kind": "link"})))?;
            }
            for (index, spec) in &mark_adds {
                nested(engine, session, spec.clone())
                    .map_err(|error| tagged(error, json!({"index": index, "kind": "mark"})))?;
            }
            Ok(())
        })
    })?;
    let link_ids: Vec<i64> = match link_adds.is_empty() {
        true => Vec::new(),
        false => (listed_ids(engine, session, "link", page)?)
            .difference(&links_before)
            .copied()
            .collect(),
    };
    let mark_ids: Vec<i64> = match mark_adds.is_empty() {
        true => Vec::new(),
        false => (listed_ids(engine, session, "mark", page)?)
            .difference(&marks_before)
            .copied()
            .collect(),
    };
    let count = objects.len() + texts.len() + field_adds.len() + link_adds.len() + mark_adds.len();
    let names: Vec<&str> = objects.iter().map(|o| o.name.as_str()).collect();
    let fields: Vec<&str> = field_adds
        .iter()
        .map(|(_, name, ..)| name.as_str())
        .collect();
    Ok(edit_state_reply(
        session,
        request,
        json!({"page": page, "count": count, "names": names, "targetIds": target_ids,
            "fields": fields, "fieldIds": field_ids, "linkIds": link_ids, "markIds": mark_ids,
            "skipped": skipped}),
    ))
}

/// The `field-add` request a pasted field makes (engine.ts's spec).
fn field_spec(
    page: usize,
    row: &Value,
    name: &str,
    kids: Option<&[(String, [f64; 4])]>,
    moved: &impl Fn(&[f64; 4]) -> [f64; 4],
) -> Value {
    let kind = row.get("kind").and_then(Value::as_str).unwrap_or("");
    let mut spec = json!({"op": "field-add", "page": page, "kind": kind, "name": name});
    match kids {
        Some(kids) => {
            let kids: Vec<Value> = kids
                .iter()
                .map(|(on, rect)| json!({"on": on, "rect": moved(rect)}))
                .collect();
            spec["kids"] = kids.into();
        }
        None => spec["rect"] = json!(moved(&rect4(row.get("rect")).unwrap_or_default())),
    }
    if kind == "combo" {
        spec["options"] = row.get("options").cloned().unwrap_or_else(|| json!([]));
        spec["edit"] = (row.get("edit") == Some(&Value::Bool(true))).into();
    }
    if kind == "text" {
        if let Some(max) = row.get("maxLen") {
            spec["maxLen"] = max.clone();
        }
        if request::truthy(row.get("multiline")) {
            spec["multiline"] = true.into();
        }
    }
    if let Some(tu) = row.get("tu") {
        spec["label"] = tu.clone();
    }
    spec["readOnly"] = row.get("readOnly").cloned().unwrap_or(Value::Null);
    spec["required"] = row.get("required").cloned().unwrap_or(Value::Null);
    if let Some(default) = row.get("default") {
        spec["default"] = default.clone();
    }
    spec
}

/// redact.ts `redactRect`: normalized, clamped to the page box, at least
/// 1 by 1.
fn redact_rect(rect: &[f64; 4], (width, height): (f64, f64)) -> Result<[f64; 4], AppError> {
    let x0 = rect[0].min(rect[2]).max(0.0);
    let y0 = rect[1].min(rect[3]).max(0.0);
    let x1 = rect[0].max(rect[2]).min(width);
    let y1 = rect[1].max(rect[3]).min(height);
    let wide = |side: f64| side >= 1.0;
    if !wide(x1 - x0) || !wide(y1 - y0) {
        return Err(AppError::reason(
            "invalid",
            "redact-add: the clamped rect must be at least 1 by 1 point",
        ));
    }
    Ok([x0, y0, x1, y1])
}

/// The pasted objects and runs: fresh direct /Resources with the new names,
/// one new /Contents element. Returns the element's path.
fn write_content(
    session: &mut AppSession,
    page: usize,
    scratch: &Scratch,
    objects: &[ObjectPaste],
    texts: &[TextPaste],
) -> Result<String, AppError> {
    let mut tx = session.tx()?;
    let page_id = page_object(tx.document(), page, "clip-paste")?;
    let mut paster = Paster::new(scratch);
    let (mut resources, mut xobjects, mut fonts, contents) = {
        let doc = tx.document();
        let dict = doc.resolve_id(page_id).as_dict().ok_or_else(|| {
            AppError::reason("invalid", "clip-paste: the page is not a dictionary")
        })?;
        let resources = page_resources(doc, dict).cloned().unwrap_or_default();
        let xobjects = super::font::dict_at(doc, Some(&resources), b"XObject").cloned();
        let fonts = super::font::dict_at(doc, Some(&resources), b"Font").cloned();
        let contents: Vec<Object> = match dict.get(b"Contents".as_slice()) {
            None => Vec::new(),
            Some(value) => match doc.resolve(value) {
                Object::Array(items) => items.clone(),
                Object::Null => Vec::new(),
                _ => vec![value.clone()],
            },
        };
        (
            resources,
            xobjects.unwrap_or_default(),
            fonts.unwrap_or_default(),
            contents,
        )
    };
    for write in objects {
        let object = paster.object(&mut tx, write.object)?;
        xobjects.insert(write.name.as_bytes().to_vec(), object);
    }
    for write in texts {
        let font = paster.object(&mut tx, write.font)?;
        fonts.insert(write.font_name.as_bytes().to_vec(), font);
    }
    if !objects.is_empty() {
        resources.insert(b"XObject".to_vec(), Object::Dictionary(xobjects));
    }
    if !texts.is_empty() {
        resources.insert(b"Font".to_vec(), Object::Dictionary(fonts));
    }
    let mut blocks = String::new();
    for write in objects {
        blocks.push_str("\nq\n");
        if let Some(clip) = &write.clip_text {
            blocks.push_str(&format!("{clip} re W n\n"));
        }
        blocks.push_str(&format!("{} cm\n/{} Do\nQ", write.cm, write.name));
    }
    for write in texts {
        blocks.push_str(&write.block);
    }
    blocks.push('\n');
    let stream = tx.allocate(
        Object::Dictionary(Dictionary::new()),
        Some(blocks.into_bytes()),
    )?;
    let mut contents = contents;
    contents.push(Object::Reference(stream));
    let path = format!("c{}", contents.len().saturating_sub(1));
    Place::object(page_id).update(&mut tx, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.insert(b"Resources".to_vec(), Object::Dictionary(resources));
            dict.insert(b"Contents".to_vec(), Object::Array(contents));
        }
    })?;
    Ok(path)
}

/// engine.ts's in-step proof: a fresh scan finds every pasted object as a
/// candidate at its box, and every run once at its origin. Returns the
/// pasted runs that scan as candidates.
fn prove(
    session: &mut AppSession,
    page: usize,
    path: &str,
    objects: &[ObjectPaste],
    texts: &[TextPaste],
) -> Result<Vec<String>, AppError> {
    let scan = fresh_scan(session, page)?;
    let fresh: Vec<_> = scan.records.iter().filter(|r| r.path == path).collect();
    for write in objects {
        let landed = fresh
            .iter()
            .find(|r| r.object_name.as_deref() == Some(write.name.as_bytes()))
            .filter(|r| r.candidate)
            .and_then(|r| r.placement.as_ref())
            .is_some_and(|p| {
                p.full
                    .iter()
                    .zip(write.full)
                    .all(|(a, b)| (a - b).abs() <= 0.01)
            });
        if !landed {
            return Err(edit_error(
                "unsupported-transform",
                format!(
                    "clip-paste: {} did not land where it was placed",
                    write.name
                ),
            )
            .with_extra(json!({"index": write.index, "kind": write.kind})));
        }
    }
    if !texts.is_empty() {
        let runs = {
            let view = session.view()?;
            runs_of(view, page)
        };
        for write in texts {
            let want = super::js_len(&write.text);
            let mut hits = 0usize;
            for run in &runs {
                for (j, ch) in run.chars.iter().enumerate() {
                    if (ch.origin[0] - write.origin[0]).abs() > 0.5
                        || (ch.origin[1] - write.origin[1]).abs() > 0.5
                    {
                        continue;
                    }
                    let mut text = String::new();
                    for next in run.chars.iter().skip(j) {
                        if super::js_len(&text) >= want {
                            break;
                        }
                        text.push(next.c);
                    }
                    if text == write.text {
                        hits += 1;
                    }
                }
            }
            if hits != 1 {
                return Err(edit_error(
                    "unsupported-transform",
                    "clip-paste: a pasted run did not land where it was placed",
                )
                .with_extra(json!({"index": write.index, "kind": "text"})));
            }
        }
    }
    let mut runs: Vec<_> = fresh
        .iter()
        .filter(|r| r.kind == Kind::Text && r.candidate)
        .map(|r| (r.op.start, r.target_id.clone()))
        .collect();
    runs.sort_by_key(|(start, _)| *start);
    Ok(runs.into_iter().map(|(_, id)| id).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentLimits;

    /// A classic-xref PDF from object bodies numbered from 1.
    fn classic(objects: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            bytes.extend_from_slice(body);
            bytes.extend_from_slice(b"\nendobj\n");
        }
        let xref = bytes.len();
        let size = objects.len() + 1;
        bytes.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n")
                .as_bytes(),
        );
        bytes
    }

    /// /AcroForm /Fields [2 0 R]: field 2 is `name` with `kids` references
    /// to field 3, `leaf`.
    fn tree(name: &[u8], kids: usize, leaf: &[u8]) -> Vec<u8> {
        let mut field = b"<< /T (".to_vec();
        field.extend_from_slice(name);
        field.extend_from_slice(b") /Kids [");
        field.extend(std::iter::repeat_n(b"3 0 R ".as_slice(), kids).flatten());
        field.extend_from_slice(b"] >>");
        let mut leaf_field = b"<< /T (".to_vec();
        leaf_field.extend_from_slice(leaf);
        leaf_field.extend_from_slice(b") >>");
        classic(&[
            b"<< /Type /Catalog /AcroForm << /Fields [2 0 R] >> >>".to_vec(),
            field,
            leaf_field,
        ])
    }

    #[test]
    fn field_tree_names_are_full_names() {
        let bytes = tree(b"a", 2, b"b");
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let names = field_tree_names(&doc).unwrap();
        assert_eq!(names, BTreeSet::from(["a".to_string(), "a.b".to_string()]));
    }

    /// Kids are walked one at a time and long names are never joined, so a
    /// long /T with many kids costs the visited nodes alone (each kid used
    /// to copy its parent's name: about 1 GB for this file).
    #[test]
    fn a_wide_tree_with_a_long_name_is_bounded() {
        let long = vec![b'a'; 128 * 1024];
        let bytes = tree(&long, 8000, b"b");
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let refused = field_tree_names(&doc).unwrap_err();
        assert_eq!(refused.detail.as_deref(), Some("too-many"));
        // Within the node cap, names past the field-add limit are left out.
        let bytes = tree(&long, 100, b"b");
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        assert!(field_tree_names(&doc).unwrap().is_empty());
        let bytes = tree(&[b'a'; 127], 1, b"b");
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        assert_eq!(field_tree_names(&doc).unwrap().len(), 1);
    }
}
