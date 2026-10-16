// SPDX-License-Identifier: AGPL-3.0-or-later
//! Page decoration (engine.ts `stampPages`) and batch stamping (`batchFile`):
//! a watermark, header or footer of WinAnsi text in Base-14 Helvetica,
//! appended to each page as its own content stream, with `{bates}`,
//! `{page}`, `{pages}` and `{label}` filled in per page.
//!
//! The step names (`add watermark`, `add header`, `add footer`) are part of
//! the wire contract. batch-file stamps a document the session never holds
//! and returns its refusals as replies without journal fields.
use std::collections::BTreeSet;
use std::f64::consts::FRAC_1_SQRT_2;
use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::pages::{check_pages, editable_tree, parse_page_range};
use crate::app::edit::{Place, get};
use crate::app::pagetree;
use crate::app::request::{self, js_number, unescape_key};
use crate::app::{
    AppDocument, AppError, AppSession, Buffers, MAX_PDF_BYTES, MAX_SAVE_BYTES, Transaction, info,
    labels, overlay, reply,
};
use crate::builtin_metrics::{BuiltinFace, BuiltinMetricLookup, BuiltinMetrics};
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};
use crate::serialize::SerializeError;
use crate::simple_encoding::{NamedEncoding, SimpleEncoding};
use crate::writer::{DocumentWriteError, Encryption, WriteOptions, write_document};

/// engine.ts `MAX_STAMP_CHARS`, in UTF-16 code units.
pub const MAX_STAMP_CHARS: usize = 200;
/// engine.ts `MAX_BATES`: the largest Bates number, 10^15 - 1.
pub const MAX_BATES: i64 = 999_999_999_999_999;
/// engine.ts `MAX_BATES_PAD`.
pub const MAX_BATES_PAD: i64 = 15;
/// The most /Contents entries the pages one stamp marks may hold after it
/// (what each keeps plus its two new streams). Every page gets its own
/// copy of a shared /Contents array, so a small file could otherwise ask
/// for billions of entries.
pub const MAX_STAMP_CONTENTS: usize = 2_000_000;
/// The resource name of the stamp font on every page it marks.
const FONT_KEY: &[u8] = b"PDF4AllF";
/// The margin, in user space units, between a header or footer and the edge.
const MARGIN: f64 = 36.0;

fn plain(message: impl Into<String>) -> AppError {
    AppError::plain(message)
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::reason("invalid", message)
}

/// engine.ts `STAMP_KINDS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StampKind {
    Watermark,
    Header,
    Footer,
}

impl StampKind {
    fn parse(kind: &str) -> Option<Self> {
        match kind {
            "watermark" => Some(Self::Watermark),
            "header" => Some(Self::Header),
            "footer" => Some(Self::Footer),
            _ => None,
        }
    }

    /// The journal step, which undo and redo report.
    pub fn step(self) -> &'static str {
        match self {
            Self::Watermark => "add watermark",
            Self::Header => "add header",
            Self::Footer => "add footer",
        }
    }

    /// The marked-content `/Kind` of the appended stream.
    fn tag(self) -> &'static str {
        match self {
            Self::Watermark => "Watermark",
            Self::Header => "Header",
            Self::Footer => "Footer",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

/// A checked stamp action (engine.ts `checkStampAction`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StampAction {
    pub kind: StampKind,
    /// The template, placeholders unexpanded.
    pub text: String,
    pub align: Option<Align>,
    pub bates_start: Option<i64>,
    pub bates_pad: Option<i64>,
}

impl StampAction {
    fn has_bates(&self) -> bool {
        self.text.contains("{bates}")
    }
}

/// engine-core.ts `winAnsiByte`: the WinAnsiEncoding byte of a character.
pub fn win_ansi_byte(c: char) -> Option<u8> {
    let code = u32::from(c);
    if (0x20..=0x7e).contains(&code) || (0xa0..=0xff).contains(&code) {
        return u8::try_from(code).ok();
    }
    Some(match code {
        0x20ac => 0x80,
        0x201a => 0x82,
        0x0192 => 0x83,
        0x201e => 0x84,
        0x2026 => 0x85,
        0x2020 => 0x86,
        0x2021 => 0x87,
        0x02c6 => 0x88,
        0x2030 => 0x89,
        0x0160 => 0x8a,
        0x2039 => 0x8b,
        0x0152 => 0x8c,
        0x017d => 0x8e,
        0x2018 => 0x91,
        0x2019 => 0x92,
        0x201c => 0x93,
        0x201d => 0x94,
        0x2022 => 0x95,
        0x2013 => 0x96,
        0x2014 => 0x97,
        0x02dc => 0x98,
        0x2122 => 0x99,
        0x0161 => 0x9a,
        0x203a => 0x9b,
        0x0153 => 0x9c,
        0x017e => 0x9e,
        0x0178 => 0x9f,
        _ => return None,
    })
}

/// `JSON.stringify` of a request value as JavaScript sees it: markers read
/// as the values they stand for (a buffer is `{}`, a non-finite number
/// `null`) and escaped keys unescaped.
fn js_json(value: &Value) -> String {
    let string = |text: &str| Value::String(text.to_string()).to_string();
    if let Some(number) = request::as_js_number(value) {
        return if number.is_finite() {
            js_number(number)
        } else {
            "null".into()
        };
    }
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => value.to_string(),
        Value::String(text) => string(text),
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(js_json).collect();
            format!("[{}]", items.join(","))
        }
        Value::Object(_) if request::buffer_index(value).is_some() => "{}".into(),
        Value::Object(fields) => {
            let fields: Vec<String> = (fields.iter())
                .map(|(key, value)| format!("{}:{}", string(unescape_key(key)), js_json(value)))
                .collect();
            format!("{{{}}}", fields.join(","))
        }
    }
}

/// engine.ts `checkStampAction` (with engine-core.ts `checkStampText`): the
/// kind, text, alignment and Bates fields, before any document is touched.
/// The upper Bates bound needs the page count (`check_bates_range`).
pub fn check_stamp_action(action: &Value) -> Result<StampAction, AppError> {
    let kind = (action.get("kind").and_then(Value::as_str))
        .and_then(StampKind::parse)
        .ok_or_else(|| plain("stamp: kind must be one of watermark, header, footer"))?;
    let text = (action.get("text").and_then(Value::as_str))
        .filter(|text| (1..=MAX_STAMP_CHARS).contains(&text.encode_utf16().count()))
        .ok_or_else(|| {
            plain(format!(
                "stamp: text must be 1..{MAX_STAMP_CHARS} characters"
            ))
        })?;
    if let Some(c) = text.chars().find(|&c| win_ansi_byte(c).is_none()) {
        return Err(plain(format!(
            "stamp: {} (U+{:X}) is not a WinAnsi character",
            Value::String(c.to_string()),
            u32::from(c)
        )));
    }
    let align = match action.get("align") {
        None => None,
        Some(Value::String(align)) if align == "left" => Some(Align::Left),
        Some(Value::String(align)) if align == "right" => Some(Align::Right),
        Some(other) => {
            return Err(invalid(format!(
                "stamp: invalid-align: align must be left or right, not {}",
                js_json(other)
            )));
        }
    };
    let bates_pad = match action.get("batesPad") {
        None => None,
        Some(pad) => Some(
            request::as_int(pad)
                .filter(|pad| (1..=MAX_BATES_PAD).contains(pad))
                .ok_or_else(|| {
                    invalid(format!(
                        "stamp: invalid-bates: batesPad must be an integer in 1..{MAX_BATES_PAD}"
                    ))
                })?,
        ),
    };
    let bates_start = match action.get("batesStart") {
        None => None,
        Some(start) => Some(
            request::as_int(start)
                .filter(|start| (0..=MAX_BATES).contains(start))
                .ok_or_else(|| {
                    invalid(format!(
                        "stamp: invalid-bates: batesStart must be an integer in 0..{MAX_BATES}"
                    ))
                })?,
        ),
    };
    if text.contains("{bates}") && bates_start.is_none() {
        return Err(invalid(
            "stamp: bates-start-required: the text has {bates} but no batesStart",
        ));
    }
    Ok(StampAction {
        kind,
        text: text.to_string(),
        align,
        bates_start,
        bates_pad,
    })
}

/// engine.ts `checkBatesRange`: the last Bates number of `count` pages
/// stays within `MAX_BATES`.
pub fn check_bates_range(start: Option<i64>, count: usize) -> Result<(), AppError> {
    let Some(start) = start else {
        return Ok(());
    };
    let last = i64::try_from(count).map_or(i64::MAX, |count| start.saturating_add(count - 1));
    if last > MAX_BATES {
        return Err(invalid(format!(
            "stamp: invalid-bates: batesStart must be an integer with batesStart..batesStart+{} in 0..{MAX_BATES}",
            i64::try_from(count).map_or(i64::MAX, |count| count - 1)
        )));
    }
    Ok(())
}

/// engine.ts `stamp`: one operation stamps every named page (all by
/// default), in ascending order.
pub fn stamp(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let doc = session.require_mutable("stamp")?;
    let action = check_stamp_action(request)?;
    let count = doc.page_count();
    let pages = match request.get("pages") {
        None => (0..count).collect(),
        pages => {
            let mut pages = check_pages(pages, count, "stamp")?;
            pages.sort_unstable();
            pages.dedup();
            pages
        }
    };
    check_bates_range(action.bates_start, pages.len())?;
    let range = session.mutate(action.kind.step(), |session| {
        let mut tx = session.tx()?;
        stamp_pages(&mut tx, &pages, &action)
    })?;
    let mut fields = Map::new();
    if let Some(Some((from, to))) = range {
        fields.insert("bates".into(), json!({"from": from, "to": to}));
    }
    fields.extend(session.edit_state());
    Ok(reply(request, Value::Object(fields)))
}

/// A batch-file refusal: a reply, never an error, so it carries no journal
/// fields.
fn refusal(request: &Value, reason: &str, error: String, detail: Option<&str>) -> Value {
    let mut fields = Map::new();
    if let Some(id) = request.get("id") {
        fields.insert("id".into(), id.clone());
    }
    fields.insert("ok".into(), false.into());
    fields.insert("reason".into(), reason.into());
    if let Some(detail) = detail.filter(|detail| !detail.is_empty()) {
        fields.insert("detail".into(), detail.into());
    }
    fields.insert("error".into(), error.into());
    Value::Object(fields)
}

/// The refusal token of a stamp message (`/^stamp: ([a-z-]+):/`), else "text".
fn stamp_token(message: &str) -> &str {
    message
        .strip_prefix("stamp: ")
        .and_then(|rest| rest.split_once(':'))
        .map(|(token, _)| token)
        .filter(|token| {
            !token.is_empty() && token.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
        })
        .unwrap_or("text")
}

/// engine.ts `batch-file`: the action stamped onto a separate document's
/// bytes and saved as the app saves, with the session untouched. Every
/// refusal is a reply (`{ok: false, reason, detail?, error}`).
pub fn batch_file(request: &Value, buffers: &mut Buffers) -> Result<Value, AppError> {
    let refuse = |reason: &str, error: String, detail: Option<&str>| {
        Ok(refusal(request, reason, error, detail))
    };
    let Some(bytes) = buffers.take(request.get("bytes")) else {
        return refuse("not-pdf", "batch-file: no bytes".into(), None);
    };
    if bytes.len() > MAX_PDF_BYTES {
        return refuse(
            "too-large",
            format!("batch-file: over the {MAX_PDF_BYTES} byte ceiling"),
            None,
        );
    }
    let empty = Value::Object(Map::new());
    let action = match request.get("action") {
        Some(Value::Null) | None => &empty,
        Some(action) => action,
    };
    let checked = match check_stamp_action(action) {
        Ok(checked) => checked,
        Err(error) => {
            let token = stamp_token(&error.message).to_string();
            return refuse("stamp-error", error.message, Some(&token));
        }
    };
    if bytes.len() < 5 {
        return refuse(
            "not-pdf",
            "batch-file: too few bytes for a PDF".into(),
            None,
        );
    }
    let mut doc = match AppDocument::load(Arc::new(bytes), b"") {
        Ok(doc) => doc,
        Err(error) => {
            return refuse(
                "not-pdf",
                format!("batch-file: the file could not be read ({error})"),
                None,
            );
        }
    };
    let info = doc.info(doc.is_locked());
    if info.needs_password || info.encrypted {
        return refuse(
            "encrypted",
            "batch-file: encrypted files are skipped".into(),
            None,
        );
    }
    if info.signed() {
        return refuse(
            "read-only",
            "batch-file: signed or certified files are read-only".into(),
            None,
        );
    }
    let count = doc.page_count();
    if count < 1 {
        return refuse("not-pdf", "batch-file: the file has no pages".into(), None);
    }
    let pages = match action.get("pages") {
        None => Some((0..count).collect()),
        Some(Value::String(range)) => parse_page_range(range, count),
        Some(_) => None,
    };
    let Some(pages) = pages.filter(|pages: &Vec<usize>| !pages.is_empty()) else {
        let pages = action
            .get("pages")
            .map_or_else(|| "undefined".into(), js_json);
        return refuse(
            "bad-pages",
            format!("batch-file: pages {pages} do not fit {count} pages"),
            None,
        );
    };
    let stamped = check_bates_range(checked.bates_start, pages.len())
        .and_then(|()| stamp_detached(&mut doc, &pages, &checked));
    let range = match stamped {
        Ok(range) => range,
        Err(error) if error.reason.as_deref() == Some("too-large") => {
            return refuse("too-large", error.message, None);
        }
        Err(error) => {
            let token = stamp_token(&error.message).to_string();
            return refuse("stamp-error", error.message, Some(&token));
        }
    };
    let mut limits = overlay::limits();
    limits.syntax.max_output_bytes = MAX_SAVE_BYTES;
    let options = WriteOptions {
        encryption: Encryption::Plaintext,
        ..WriteOptions::APP
    };
    let written = match crate::app::save_tree::for_save(doc.document()) {
        Ok(written) => written,
        Err(error) => return refuse("invalid", format!("batch-file: {error}"), None),
    };
    let out = match saved(write_document(&written, limits, options)) {
        Ok(out) => out,
        Err((reason, error, detail)) => return refuse(reason, error, detail),
    };
    drop(doc);
    let mut fields = Map::new();
    fields.insert("pages".into(), pages.len().into());
    fields.insert("pageCount".into(), count.into());
    fields.insert("bytes".into(), buffers.put(out));
    if let Some((from, to)) = range {
        fields.insert("bates".into(), json!({"from": from, "to": to}));
    }
    Ok(reply(request, Value::Object(fields)))
}

/// A refusal's reason, message and detail.
type SaveRefusal = (&'static str, String, Option<&'static str>);

/// A batch save as engine.ts answers it: the bytes, or the refusal for a
/// failed save or one past the ceiling.
fn saved(result: Result<Vec<u8>, DocumentWriteError>) -> Result<Vec<u8>, SaveRefusal> {
    match result {
        Ok(out) if out.len() <= MAX_SAVE_BYTES => Ok(out),
        Ok(_) | Err(DocumentWriteError::Serialize(SerializeError::OutputLimit)) => Err((
            "too-large",
            format!("batch-file: the result would exceed the {MAX_SAVE_BYTES} byte save ceiling"),
            None,
        )),
        Err(error) => Err(("stamp-error", error.to_string(), Some("save"))),
    }
}

/// The stamp loop on a document the session does not hold; its journal
/// only gives the writes a `Transaction`.
fn stamp_detached(
    doc: &mut AppDocument,
    pages: &[usize],
    action: &StampAction,
) -> Result<Option<(i64, i64)>, AppError> {
    let refused = || plain("stamp: the file cannot be changed");
    if !doc.begin("stamp") {
        return Err(refused());
    }
    let result = match doc.transaction() {
        Some(mut tx) => stamp_pages(&mut tx, pages, action),
        None => Err(refused()),
    };
    doc.end();
    result
}

/// engine.ts `stampLoop`: stamps `pages` (ascending, distinct) in order,
/// `{bates}` counting from the start in that order. Returns the Bates range
/// when the text has `{bates}`.
fn stamp_pages(
    tx: &mut Transaction<'_>,
    pages: &[usize],
    action: &StampAction,
) -> Result<Option<(i64, i64)>, AppError> {
    check_contents(tx.document(), pages)?;
    let ids = indirect_pages(tx, pages)?;
    let doc = tx.document();
    let count = ids.len();
    let labels = labels::items(doc, count);
    let metrics = BuiltinMetrics::from_encoding(
        BuiltinFace::Helvetica,
        SimpleEncoding::from_named(NamedEncoding::WinAnsi),
    );
    let mut font = stamp_font(doc);
    let start = action.bates_start.unwrap_or(0);
    let pad = usize::try_from(action.bates_pad.unwrap_or(6)).unwrap_or(6);
    let align = match action.kind {
        StampKind::Watermark => None,
        _ => action.align,
    };
    for (position, &index) in pages.iter().enumerate() {
        let Some(id) = ids.get(index).copied().flatten() else {
            return Err(plain(format!(
                "stamp: page {index} is not in the page tree"
            )));
        };
        let bates = action.has_bates().then(|| {
            let number = i64::try_from(position).map_or(i64::MAX, |p| start.saturating_add(p));
            format!("{number:0>pad$}")
        });
        let label = labels.get(index).map_or("", String::as_str);
        let text = expand(&action.text, bates.as_deref(), index, count, label);
        if !has_stamp_font(tx.document(), id) {
            let font_id = match font {
                Some(font) => font,
                None => {
                    let new = tx.allocate(helvetica(), None)?;
                    font = Some(new);
                    new
                }
            };
            add_font(tx, id, font_id)?;
        }
        let body = content(tx.document(), id, action.kind, align, &text, &metrics);
        let head = tx.allocate(Object::Dictionary(Dictionary::new()), Some(b"q\n".to_vec()))?;
        let tail = tx.allocate(Object::Dictionary(Dictionary::new()), Some(body))?;
        set_contents(tx, id, head, tail)?;
    }
    let to = i64::try_from(pages.len()).map_or(i64::MAX, |n| start.saturating_add(n - 1));
    Ok(action.has_bates().then_some((start, to)))
}

/// The /Contents entries a stamp keeps between its two streams: an array's
/// items, a lone stream, none for a missing or null value, and any other
/// value as engine.ts keeps it, but only on a /Type /Page dictionary. The
/// brute-force page list of a tree without /Kids also lists annotations,
/// whose /Contents is their text: those are None.
fn kept_contents<'d>(doc: &'d Document<'_>, page: &'d Dictionary) -> Option<&'d [Object]> {
    let Some(own) = page.get(b"Contents".as_slice()) else {
        return Some(&[]);
    };
    let stream = (own.as_reference())
        .is_some_and(|id| doc.object(id).is_ok_and(|object| object.stream.is_some()));
    match doc.resolve(own) {
        Object::Array(items) => Some(items),
        Object::Null => Some(&[]),
        _ if stream || doc.dict_get(page, b"Type").as_name() == Some(b"Page") => {
            Some(std::slice::from_ref(own))
        }
        _ => None,
    }
}

/// Refuses, before any write, a page whose /Contents is not content
/// (`kept_contents`) and pages that would hold more than
/// `MAX_STAMP_CONTENTS` entries.
fn check_contents(doc: &Document<'_>, pages: &[usize]) -> Result<(), AppError> {
    let list = info::pages(doc);
    let mut total = 0usize;
    for &index in pages {
        let Some(page) = list.get(index) else {
            continue;
        };
        let kept = kept_contents(doc, page.dict).ok_or_else(|| {
            invalid(format!(
                "stamp: not-a-page: page {index} has a /Contents that is not content"
            ))
        })?;
        total = total.saturating_add(kept.len()).saturating_add(2);
        if total > MAX_STAMP_CONTENTS {
            return Err(AppError::reason(
                "too-large",
                format!(
                    "stamp: the pages would list more than {MAX_STAMP_CONTENTS} content streams"
                ),
            ));
        }
    }
    Ok(())
}

/// The page objects in page order, after giving each page in `pages` that
/// is written directly into /Kids its own object (ISO 32000 /Kids holds
/// references), in the same step.
fn indirect_pages(
    tx: &mut Transaction<'_>,
    pages: &[usize],
) -> Result<Vec<Option<ObjectId>>, AppError> {
    let ids: Vec<Option<ObjectId>> = info::pages(tx.document()).iter().map(|p| p.id).collect();
    if !pages
        .iter()
        .any(|&index| matches!(ids.get(index), Some(None)))
    {
        return Ok(ids);
    }
    let tree = editable_tree(tx, "stamp")?;
    for &index in pages {
        let Some(slot) = tree.slots.get(index).filter(|slot| slot.id.is_none()) else {
            continue;
        };
        let kids = Place::object(slot.parent).child(tx.document(), b"Kids");
        let page = kids
            .read(tx.document())
            .as_array()
            .and_then(|kids| kids.get(slot.kid));
        let Some(page) = page.filter(|page| page.as_dict().is_some()).cloned() else {
            continue;
        };
        let id = tx.allocate(page, None)?;
        kids.update(tx, |value| {
            if let Some(kid) = value.as_array_mut().and_then(|kids| kids.get_mut(slot.kid)) {
                *kid = Object::Reference(id);
            }
        })?;
    }
    let ids: Vec<Option<ObjectId>> = info::pages(tx.document()).iter().map(|p| p.id).collect();
    if pages
        .iter()
        .any(|&index| !matches!(ids.get(index), Some(Some(_))))
    {
        return Err(plain("stamp: a page could not be given its own object"));
    }
    Ok(ids)
}

/// The stamp font: `<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>`,
/// as MuPDF's `addSimpleFont` writes Base-14 Helvetica.
fn helvetica() -> Object {
    Object::dictionary([
        ("Type", Object::name("Font")),
        ("Subtype", Object::name("Type1")),
        ("BaseFont", Object::name("Helvetica")),
        ("Encoding", Object::name("WinAnsiEncoding")),
    ])
}

/// A page's resources as MuPDF's `pdf_dict_get_inheritable` finds them:
/// its own non-null /Resources, else the nearest ancestor's.
fn inherited_resources<'d>(doc: &'d Document<'_>, page: &'d Dictionary) -> &'d Object {
    pagetree::inherited(doc, page, b"Resources").map_or(&Object::Null, |value| doc.resolve(value))
}

/// The stamp font an earlier stamp registered on any page: a /PDF4AllF
/// font resource that is exactly `helvetica()`, so stamps share one font
/// object as MuPDF's font cache makes them share.
fn stamp_font(doc: &Document<'_>) -> Option<ObjectId> {
    let wanted = helvetica();
    let mut seen = BTreeSet::new();
    info::pages(doc).iter().find_map(|page| {
        let fonts = get(doc, inherited_resources(doc, page.dict), b"Font");
        let id = fonts.as_dict()?.get(FONT_KEY)?.as_reference()?;
        (seen.insert(id)
            && doc
                .object(id)
                .is_ok_and(|object| object.stream.is_none() && object.value == wanted))
        .then_some(id)
    })
}

/// Whether the page's resources already name a /PDF4AllF font.
fn has_stamp_font(doc: &Document<'_>, page: ObjectId) -> bool {
    let Some(dict) = doc.resolve_id(page).as_dict() else {
        return false;
    };
    let fonts = get(doc, inherited_resources(doc, dict), b"Font");
    !get(doc, fonts, FONT_KEY).is_null()
}

/// Makes `key` of the dictionary at `place` a dictionary when it is not one
/// (missing, null, dangling or another type).
fn ensure_dict(tx: &mut Transaction<'_>, place: &Place, key: &[u8]) -> Result<(), AppError> {
    if get(tx.document(), place.read(tx.document()), key)
        .as_dict()
        .is_some()
    {
        return Ok(());
    }
    place.update(tx, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.insert(key.to_vec(), Object::Dictionary(Dictionary::new()));
        }
    })?;
    Ok(())
}

/// Registers the stamp font as /PDF4AllF in the font resources the page
/// draws with. Inherited resources are edited where they live (an
/// ancestor, direct or not, as `pagetree::inherited` walks), so the page
/// keeps everything it inherits (MuPDF gives such a page its own
/// /Resources holding only the stamp font).
fn add_font(tx: &mut Transaction<'_>, page: ObjectId, font: ObjectId) -> Result<(), AppError> {
    let doc = tx.document();
    let mut holder = Place::object(page);
    let mut node = Place::object(page);
    for _ in 0..64 {
        let Some(dict) = node.read(doc).as_dict() else {
            break;
        };
        if let Some(value) = dict.get(b"Resources".as_slice()) {
            let value = doc.resolve(value);
            if value.as_dict().is_some() {
                holder = node.clone();
            }
            if !value.is_null() {
                break;
            }
        }
        if doc.dict_get(dict, b"Parent").as_dict().is_none() {
            break;
        }
        node = node.child(doc, b"Parent");
    }
    ensure_dict(tx, &holder, b"Resources")?;
    let resources = holder.child(tx.document(), b"Resources");
    ensure_dict(tx, &resources, b"Font")?;
    let fonts = resources.child(tx.document(), b"Font");
    fonts.update(tx, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.insert(FONT_KEY.to_vec(), Object::Reference(font));
        }
    })?;
    Ok(())
}

/// /Contents becomes `[head, ...the page's streams, tail]`, so nothing the
/// page leaves on the graphics state reaches the stamp.
fn set_contents(
    tx: &mut Transaction<'_>,
    page: ObjectId,
    head: ObjectId,
    tail: ObjectId,
) -> Result<(), AppError> {
    let doc = tx.document();
    let kept = (doc.resolve_id(page).as_dict())
        .and_then(|dict| kept_contents(doc, dict))
        .ok_or_else(|| plain("stamp: a page's /Contents is not content"))?;
    let mut contents = Vec::with_capacity(kept.len().saturating_add(2));
    contents.push(Object::Reference(head));
    contents.extend(kept.iter().cloned());
    contents.push(Object::Reference(tail));
    tx.update(page, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.insert(b"Contents".to_vec(), Object::Array(contents));
        }
    })?;
    Ok(())
}

/// engine.ts `stampTextFor`: `{bates}`, then `{page}`, `{pages}` and
/// `{label}`, each inserted literally.
fn expand(template: &str, bates: Option<&str>, index: usize, count: usize, label: &str) -> String {
    let page = index.saturating_add(1).to_string();
    template
        .replace("{bates}", bates.unwrap_or(""))
        .replace("{page}", &page)
        .replace("{pages}", &count.to_string())
        .replace("{label}", label)
}

/// engine.ts `round2`: `Math.round(value * 100) / 100`.
fn round2(value: f64) -> f64 {
    (value * 100.0 + 0.5).floor() / 100.0
}

/// The page's visible area in unrotated user space (CropBox within
/// MediaBox, both inherited, as the bound page has it) and its display
/// rotation (/Rotate snapped to a quarter turn), as `info::page_size`.
fn frame(doc: &Document<'_>, page: &Dictionary) -> ([f64; 4], i32) {
    let empty = |r: &[f64; 4]| !(r[0] < r[2] && r[1] < r[3]);
    let mut area = info::page_box(doc, page, b"MediaBox").unwrap_or([0.0; 4]);
    if let Some(crop) = info::page_box(doc, page, b"CropBox") {
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
        area = [0.0, 0.0, 612.0, 792.0];
    } else if area[2] - area[0] < 1.0 || area[3] - area[1] < 1.0 {
        area = [0.0, 0.0, 1.0, 1.0];
    }
    let mut rotate = 0.0f32;
    let mut node = page;
    for _ in 0..64 {
        let value = node
            .get(b"Rotate".as_slice())
            .map(|value| doc.resolve(value));
        if let Some(Object::Integer(r)) = value {
            rotate = *r as f32;
            break;
        }
        if let Some(Object::Real(r)) = value {
            rotate = *r as f32;
            break;
        }
        match doc.dict_get(node, b"Parent").as_dict() {
            Some(parent) => node = parent,
            None => break,
        }
    }
    let mut rotate = (rotate + 0.5).floor() as i32;
    if rotate < 0 {
        rotate = 360 - (rotate.unsigned_abs() % 360) as i32;
    }
    rotate = 90 * ((rotate % 360 + 45) / 90);
    if rotate >= 360 {
        rotate = 0;
    }
    (area, rotate)
}

/// The width of WinAnsi `bytes` in Helvetica, in em.
fn advance(bytes: &[u8], metrics: &BuiltinMetrics) -> f64 {
    let units: f64 = (bytes.iter())
        .map(|&byte| match metrics.lookup(byte) {
            BuiltinMetricLookup::Advance(advance) => advance.value(),
            _ => 500.0,
        })
        .sum();
    units / 1000.0
}

/// The appended stream: the graphics state restored, then the text in a
/// `/PDF4All` marked-content sequence. Watermarks are 48 pt 50% grey on the
/// diagonal through the centre, reading upward on the display; headers and
/// footers 10 pt black, 36 units in from the visual top or bottom, upright
/// on the display, starting 36 units from the left or (`right`) ending 36
/// from the right.
fn content(
    doc: &Document<'_>,
    page: ObjectId,
    kind: StampKind,
    align: Option<Align>,
    text: &str,
    metrics: &BuiltinMetrics,
) -> Vec<u8> {
    let empty = Dictionary::new();
    let dict = doc.resolve_id(page).as_dict().unwrap_or(&empty);
    let ([llx, lly, urx, ury], rotate) = frame(doc, dict);
    let (width, height) = (urx - llx, ury - lly);
    let drawn: Vec<u8> = text
        .chars()
        .map(|c| win_ansi_byte(c).unwrap_or(b'?'))
        .collect();
    let em = advance(&drawn, metrics);
    let n = js_number;
    let body = match kind {
        StampKind::Watermark => {
            let size = 48.0;
            let c = FRAC_1_SQRT_2;
            // The text matrix turns the text 45 degrees past the display's upright.
            let [a, b, c2, d] = match rotate {
                90 => [-c, c, -c, -c],
                180 => [-c, -c, c, -c],
                270 => [c, -c, c, c],
                _ => [c, c, -c, c],
            };
            // The text's centre (half its width along, half the size up) on the area's.
            let (u, v) = (em * size / 2.0, size / 2.0);
            let tx = (llx + width / 2.0) - (a * u + c2 * v);
            let ty = (lly + height / 2.0) - (b * u + d * v);
            format!(
                "0.5 g BT /PDF4AllF 48 Tf {} {} {} {} {} {} Tm ",
                n(a),
                n(b),
                n(c2),
                n(d),
                n(round2(tx)),
                n(round2(ty))
            )
        }
        StampKind::Header | StampKind::Footer => {
            let size = 10.0;
            let (visual_width, visual_height) = match rotate {
                90 | 270 => (height, width),
                _ => (width, height),
            };
            let vx = match align {
                Some(Align::Right) => MARGIN.max(visual_width - MARGIN - em * size),
                _ => MARGIN,
            };
            let vy = match kind {
                StampKind::Header => MARGIN,
                _ => visual_height - MARGIN,
            };
            // Where a visual point (from the top left of the display) lies in
            // unrotated space, and the matrix that turns the text upright.
            let (x, y, m) = match rotate {
                90 => (llx + vy, lly + vx, [0, 1, -1, 0]),
                180 => (llx + width - vx, lly + vy, [-1, 0, 0, -1]),
                270 => (llx + width - vy, lly + height - vx, [0, -1, 1, 0]),
                _ => (llx + vx, lly + height - vy, [1, 0, 0, 1]),
            };
            format!(
                "0 g BT /PDF4AllF 10 Tf {} {} {} {} {} {} Tm ",
                m[0],
                m[1],
                m[2],
                m[3],
                n(round2(x)),
                n(round2(y))
            )
        }
    };
    let mut out = format!("Q\n/PDF4All <</Kind /{}>> BDC {body}(", kind.tag()).into_bytes();
    for byte in drawn {
        if matches!(byte, b'(' | b')' | b'\\') {
            out.push(b'\\');
        }
        out.push(byte);
    }
    out.extend_from_slice(b") Tj ET EMC\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stringify_matches_javascript() {
        let value = serde_json::json!({"$$a": [1, 1.5, "x\n\"", null, {"$number": "NaN"}],
            "b": {"$buffer": 0}, "c": true});
        assert_eq!(
            js_json(&value),
            r#"{"$a":[1,1.5,"x\n\"",null,null],"b":{},"c":true}"#
        );
        assert_eq!(js_json(&serde_json::json!(1e21)), "1e+21");
    }

    #[test]
    fn tokens_follow_the_engine_ts_pattern() {
        assert_eq!(stamp_token("stamp: invalid-bates: x"), "invalid-bates");
        assert_eq!(stamp_token("stamp: text must be 1..200 characters"), "text");
        assert_eq!(stamp_token("stamp: \"\u{1}\" (U+1) is not"), "text");
        assert_eq!(stamp_token("other: invalid: x"), "text");
        assert_eq!(stamp_token("stamp: : x"), "text");
    }

    #[test]
    fn batch_saves_map_to_engine_ts_refusals() {
        assert_eq!(saved(Ok(vec![1, 2])), Ok(vec![1, 2]));
        let ceiling =
            format!("batch-file: the result would exceed the {MAX_SAVE_BYTES} byte save ceiling");
        assert_eq!(
            saved(Ok(vec![0; MAX_SAVE_BYTES + 1])),
            Err(("too-large", ceiling.clone(), None))
        );
        let output = DocumentWriteError::Serialize(SerializeError::OutputLimit);
        assert_eq!(saved(Err(output)), Err(("too-large", ceiling, None)));
        assert_eq!(
            saved(Err(DocumentWriteError::ObjectLimit)),
            Err((
                "stamp-error",
                "document rewrite indirect-object limit exceeded".into(),
                Some("save")
            ))
        );
        let depth = DocumentWriteError::Serialize(SerializeError::DepthLimit);
        assert_eq!(
            saved(Err(depth)),
            Err((
                "stamp-error",
                "PDF serialization depth limit exceeded".into(),
                Some("save")
            ))
        );
    }

    #[test]
    fn round2_rounds_half_up_like_math_round() {
        assert_eq!(round2(246.597), 246.6);
        assert_eq!(round2(-0.005), 0.0);
        assert_eq!(round2(1.005), 1.0);
        assert_eq!(round2(302.664), 302.66);
    }
}
