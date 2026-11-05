// SPDX-License-Identifier: MIT
//! `inspect`: the test suite's raw reader of the open document (feature
//! `inspect`, on in the WASM bridge the suite loads and in the engine's own
//! tests; the desktop app does not enable it). The suite used to reopen the
//! engine's output in a second PDF library to look at trailers, page
//! dictionaries, annotation dictionaries and streams; this op answers the same
//! questions from the live document, so no other PDF reader is needed.
//!
//! Read-only: nothing here journals, mutates or unlocks. Requests:
//!
//! - `{op: "inspect"}`: the document summary, `pageCount`, `pages` (the
//!   object holding each page, `[number, generation]` or null for a page
//!   written directly into `/Kids`), `trailer` (the effective trailer
//!   dictionary), `info` (`DocumentInfo`), `version`, `repaired`, `locked`
//!   and `objects` (every object id, ascending).
//! - `{op: "inspect", page: n}`: page `n`'s own dictionary as `value`, with
//!   the id of the object holding it as `object`. No inheritance: `/Rotate` or `/MediaBox` on an ancestor are
//!   read by following `/Parent`.
//! - `{op: "inspect", object: [number, generation]}`: that object as `value`
//!   (null, with `missing: true`, when there is none), its id echoed as
//!   `object` (a reply's `id` is the request's). With `data:
//!   "decoded"` or `data: "raw"` a stream's bytes come back as the output
//!   buffer `data`.
//! - `{op: "inspect", text: n}`: page `n`'s extracted text (`text`, the
//!   reading text, one line per line and a blank line per block), its line
//!   summaries as `render` reports them (`lines`, plus `top`), and with
//!   `chars: true` every character with its baseline origin (`chars`).
//! - `compare: true` on any of these reads the compare slot's document.
//!
//! Values are JSON: null, booleans and numbers as themselves, a name as
//! `{"name": "..."}`, a string as `{"string": <its bytes, one char per
//! byte>, "text": <decoded as a PDF text string>}`, an array as an array, a
//! dictionary as `{"dict": {...}}` (keys escaped for the wire like every data
//! key) and a reference as `{"ref": [number, generation]}`, never followed:
//! the reader resolves what it asks for. A stream object is `{"stream":
//! {...its dictionary...}, "length": <stored bytes>}`.
use serde_json::{Map, Value, json};

use crate::app::request::{self, js_number_value};
use crate::app::{AppDocument, AppError, AppSession, Buffers, reply};
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId, decode_text_string};
use crate::stream::StreamLimits;

/// Deeper nesting than the open limits allow is cut, never recursed into.
const MAX_DEPTH: usize = 128;

fn plain(message: impl Into<String>) -> AppError {
    AppError::plain(message)
}

pub fn inspect(
    session: &mut AppSession,
    request: &Value,
    buffers: &mut Buffers,
) -> Result<Value, AppError> {
    let compare = matches!(request.get("compare"), Some(Value::Bool(true)));
    let (app, locked) = if compare {
        let Some(slot) = session.compare.as_mut() else {
            return Err(plain("inspect: no compare document is open"));
        };
        (slot.document_mut(), false)
    } else {
        let locked = session.locked;
        let Some(doc) = session.doc.as_mut() else {
            return Err(plain("inspect: no document open"));
        };
        (doc, locked)
    };
    let fields = if let Some(page) = request.get("page") {
        page_fields(app, page)?
    } else if let Some(id) = request.get("object") {
        object_fields(app, id, request.get("data"), buffers)?
    } else if let Some(page) = request.get("text") {
        text_fields(app, page, locked, request.get("chars"))?
    } else {
        summary_fields(app, locked)
    };
    Ok(reply(request, Value::Object(fields)))
}

/// `[number, generation]` of an object id; null for a page held directly.
fn id_value(id: Option<ObjectId>) -> Value {
    match id {
        Some(id) => json!([id.number(), id.generation()]),
        None => Value::Null,
    }
}

fn summary_fields(app: &AppDocument, locked: bool) -> Map<String, Value> {
    let doc = app.document();
    let trailer: Dictionary = doc
        .effective_trailer_entries()
        .map(|(key, value)| (key.to_vec(), value.clone()))
        .collect();
    let pages: Vec<Value> = app.page_ids().into_iter().map(id_value).collect();
    let objects: Vec<Value> = doc
        .objects()
        .map(|object| id_value(Some(object.id)))
        .collect();
    let mut fields = Map::new();
    fields.insert("pageCount".into(), pages.len().into());
    fields.insert("pages".into(), pages.into());
    fields.insert("trailer".into(), dictionary(&trailer, 0));
    fields.insert("info".into(), app.info(locked).json());
    fields.insert("version".into(), doc.version().into());
    fields.insert("repaired".into(), app.repaired().into());
    fields.insert("locked".into(), locked.into());
    fields.insert("objects".into(), objects.into());
    fields
}

/// The page index when it names a page.
fn page_index(app: &AppDocument, page: &Value) -> Result<usize, AppError> {
    let count = app.page_count();
    request::as_int(page)
        .and_then(|page| usize::try_from(page).ok())
        .filter(|&page| page < count)
        .ok_or_else(|| {
            plain(format!(
                "inspect: page {} is out of range (0..{})",
                request::js_string(Some(page)),
                count.saturating_sub(1)
            ))
        })
}

fn page_fields(app: &AppDocument, page: &Value) -> Result<Map<String, Value>, AppError> {
    let index = page_index(app, page)?;
    let Some((id, dict)) = app.page(index) else {
        return Err(plain(format!("inspect: page {index} has no dictionary")));
    };
    let mut fields = Map::new();
    fields.insert("page".into(), index.into());
    fields.insert("object".into(), id_value(id));
    fields.insert("value".into(), json!({"dict": dictionary(dict, 1)}));
    Ok(fields)
}

/// `[number, generation]` as an object id.
fn parse_id(value: &Value) -> Result<ObjectId, AppError> {
    let invalid = || plain("inspect: object must be [number, generation]");
    let Value::Array(parts) = value else {
        return Err(invalid());
    };
    let [number, generation] = parts.as_slice() else {
        return Err(invalid());
    };
    let number = request::as_int(number)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(invalid)?;
    let generation = request::as_int(generation)
        .and_then(|g| u16::try_from(g).ok())
        .ok_or_else(invalid)?;
    ObjectId::new(number, generation).map_err(|_| invalid())
}

fn object_fields(
    app: &AppDocument,
    id: &Value,
    data: Option<&Value>,
    buffers: &mut Buffers,
) -> Result<Map<String, Value>, AppError> {
    let id = parse_id(id)?;
    let doc = app.document();
    let mut fields = Map::new();
    fields.insert("object".into(), id_value(Some(id)));
    let Ok(object) = doc.object(id) else {
        fields.insert("value".into(), Value::Null);
        fields.insert("missing".into(), true.into());
        return Ok(fields);
    };
    let is_stream = object.stream.is_some();
    fields.insert(
        "value".into(),
        indirect_value(doc, id, object.stream.is_some(), &object.value),
    );
    match data {
        None | Some(Value::Null) => {}
        Some(Value::String(kind)) if kind == "decoded" || kind == "raw" => {
            if !is_stream {
                return Err(plain(format!(
                    "inspect: object {} {} is not a stream",
                    id.number(),
                    id.generation()
                )));
            }
            let bytes = if kind == "raw" {
                doc.stream_data(id)
                    .map_err(|error| plain(format!("inspect: {error}")))?
                    .map(<[u8]>::to_vec)
                    .unwrap_or_default()
            } else {
                doc.decoded_stream(id, StreamLimits::default())
                    .map_err(|error| plain(format!("inspect: {error}")))?
            };
            fields.insert("data".into(), buffers.put(bytes));
        }
        Some(_) => return Err(plain("inspect: data must be \"decoded\" or \"raw\"")),
    }
    Ok(fields)
}

/// An indirect object's value: a stream shows its dictionary under `stream`.
fn indirect_value(doc: &Document<'_>, id: ObjectId, is_stream: bool, object: &Object) -> Value {
    match (is_stream, object) {
        (true, Object::Dictionary(dict)) => {
            let length = doc.stream_data(id).ok().flatten().map_or(0, <[u8]>::len);
            json!({"stream": dictionary(dict, 1), "length": length})
        }
        _ => value(object, 0),
    }
}

/// A dictionary's entries as a JSON object; keys travel escaped like every
/// data key of the wire.
fn dictionary(dict: &Dictionary, depth: usize) -> Value {
    let mut map = Map::new();
    for (key, entry) in dict.iter() {
        let key = String::from_utf8_lossy(key);
        map.insert(
            request::escape_key(&key).into_owned(),
            value(entry, depth + 1),
        );
    }
    Value::Object(map)
}

fn value(object: &Object, depth: usize) -> Value {
    if depth > MAX_DEPTH {
        return json!({"deep": true});
    }
    match object {
        Object::Null => Value::Null,
        Object::Boolean(flag) => Value::Bool(*flag),
        Object::Integer(number) => Value::from(*number),
        Object::Real(number) => js_number_value(*number),
        Object::Name(name) => json!({"name": String::from_utf8_lossy(name)}),
        Object::String(bytes) => json!({
            "string": bytes.iter().map(|&byte| char::from(byte)).collect::<String>(),
            "text": decode_text_string(bytes),
        }),
        Object::Array(items) => {
            Value::Array(items.iter().map(|item| value(item, depth + 1)).collect())
        }
        Object::Dictionary(dict) => json!({"dict": dictionary(dict, depth)}),
        Object::Reference(id) => json!({"ref": [id.number(), id.generation()]}),
    }
}

fn text_fields(
    app: &mut AppDocument,
    page: &Value,
    locked: bool,
    chars: Option<&Value>,
) -> Result<Map<String, Value>, AppError> {
    if locked {
        return Err(plain(
            "inspect: document is encrypted, a password is required",
        ));
    }
    let with_chars = match chars {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => return Err(plain("inspect: chars must be true or false")),
    };
    let index = page_index(app, page)?;
    let view = app
        .view()
        .map_err(|error| plain(format!("inspect: {error}")))?;
    if index >= view.page_count() {
        return Err(plain(format!("inspect: page {index} is not in the view")));
    }
    let text = view
        .structured_text(index)
        .map_err(|error| plain(format!("inspect: {error}")))?;
    let lines: Vec<Value> = (text.line_summaries().into_iter())
        .map(|line| {
            json!({"text": line.text, "x": line.x, "y": line.y, "top": line.top, "w": line.w,
                "h": line.h, "size": line.size, "family": line.family, "weight": line.weight,
                "style": line.style})
        })
        .collect();
    let mut fields = Map::new();
    fields.insert("page".into(), index.into());
    fields.insert("text".into(), text.as_text().into());
    fields.insert("lines".into(), lines.into());
    if with_chars {
        let chars: Vec<Value> = (text.blocks().iter())
            .flat_map(|block| &block.lines)
            .flat_map(|line| &line.chars)
            .map(|c| {
                json!({"c": c.c.to_string(), "x": js_number_value(c.origin[0]),
                    "y": js_number_value(c.origin[1]), "size": js_number_value(c.size),
                    "quad": c.quad.iter().map(|&v| js_number_value(v)).collect::<Vec<_>>()})
            })
            .collect();
        fields.insert("chars".into(), chars.into());
    }
    Ok(fields)
}
