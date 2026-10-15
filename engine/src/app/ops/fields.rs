// SPDX-License-Identifier: AGPL-3.0-or-later
//! Form fields: fields, field-set, field-add, field-props and field-delete
//! (engine.ts), with engine.ts's check order, messages and reply shapes.
//!
//! The step names (`fill field`, `add field`, `change field`, `delete
//! field`) are part of the wire contract. A widget is named by its object
//! number; a widget written directly into /Annots lists as id 0 and cannot
//! be changed. Appearances are MuPDF's synthesis (`forms::appearance`),
//! written into the document in the same step.
use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use crate::app::edit::{Edit, Place};
use crate::app::forms::write::{TxSink, base14_font, refresh};
use crate::app::forms::{
    self, COMBO, EDIT, Kind, MULTILINE, PageSpace, RADIO, READ_ONLY, REQUIRED, ScriptContext,
    Widget, button_checked, field_flags, field_head, field_widgets, lineage, on_state, own_dict,
    page_widgets,
};
use crate::app::pagetree;
use crate::app::{AppDocument, AppError, AppSession, Transaction, reply, request};
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};

/// engine.ts `MAX_FIELD_NAME_CHARS`, in UTF-16 code units.
pub const MAX_FIELD_NAME_CHARS: usize = 128;
/// engine.ts `MAX_FIELD_NODES`: field tree nodes a name check visits.
pub const MAX_FIELD_NODES: usize = 4096;
/// engine.ts `MAX_FIELD_KIDS`.
pub const MAX_FIELD_KIDS: usize = 32;
/// engine.ts `MAX_FIELD_OPTIONS`.
pub const MAX_FIELD_OPTIONS: usize = 256;
/// engine.ts `MAX_FIELD_OPTION_CHARS`.
pub const MAX_FIELD_OPTION_CHARS: usize = 256;
/// engine.ts `MAX_FIELD_KID_NAME`.
pub const MAX_FIELD_KID_NAME: usize = 64;
/// engine.ts `MAX_FIELD_TEXT_CHARS`.
pub const MAX_FIELD_TEXT_CHARS: usize = 4096;
/// engine.ts `MAX_FIELD_LABEL`.
pub const MAX_FIELD_LABEL: usize = 256;
/// engine.ts `MIN_FIELD_RECT_PT`.
pub const MIN_FIELD_RECT_PT: f64 = 4.0;
/// engine.ts `MAX_NOTE_CHARS`: a filled value's length.
pub const MAX_NOTE_CHARS: usize = 4096;
/// Widgets of one field whose appearance a fill regenerates at most.
const MAX_FIELD_WIDGETS: usize = 4096;
/// Characters of a listed value or default at most: a longer one is cut
/// (no fill can write it back, `MAX_NOTE_CHARS`).
pub const MAX_LISTED_VALUE_CHARS: usize = 65_536;
/// Options a listed choice field shows at most, each cut at
/// `forms::MAX_NAME_CHARS` characters, as names, labels and tooltips are.
pub const MAX_LISTED_OPTIONS: usize = 4096;
/// String bytes one listing replies at most; a page past it is refused as
/// too large.
pub const MAX_LIST_BYTES: usize = 32 << 20;

const FIELD_DA: &str = "/Helv 12 Tf 0 g";
const FIELD_F_PRINT: i64 = 4;
const FIELD_CHECK_ON: &[u8] = b"Yes";

fn plain(message: impl Into<String>) -> AppError {
    AppError::plain(message)
}

/// engine-core.ts `failField`: reason `invalid` with a detail.
fn fail(detail: &str, message: impl Into<String>) -> AppError {
    AppError::detail("invalid", detail, message)
}

/// JavaScript's `.length`.
fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn edit_reply(session: &AppSession, request: &Value) -> Value {
    reply(request, Value::Object(session.edit_state()))
}

/// The page's dictionary and its object (None when written into /Kids).
fn page_of(doc: &AppDocument, page: usize) -> Result<(Option<ObjectId>, &Dictionary), AppError> {
    doc.page(page).ok_or_else(|| plain("no such page"))
}

/// engine.ts `fields`: one item per widget of the page.
pub fn list(session: &AppSession, request: &Value) -> Result<Value, AppError> {
    let Some(app) = &session.doc else {
        return Err(plain("fields: no document open"));
    };
    if session.locked {
        return Err(plain(
            "fields: document is encrypted, a password is required",
        ));
    }
    let page = session.check_page(request.get("page"), "fields")?;
    let doc = app.document();
    let (_, page_dict) = page_of(app, page)?;
    let space = PageSpace::of(doc, page_dict);
    let scripts = ScriptContext::of(doc);
    let tabs = forms::page_tabs(doc, page_dict);
    let order = (tabs == Some("S"))
        .then(|| forms::struct_order(doc))
        .flatten();
    let mut items = Vec::new();
    let mut bytes = 0usize;
    for widget in page_widgets(doc, page_dict) {
        let Some(mut item) = item(doc, &space, &scripts, widget) else {
            continue;
        };
        if let Some(index) = order.as_ref().and_then(|order| order.get(&widget.number())) {
            item.insert("structIndex".into(), (*index).into());
        }
        bytes = bytes.saturating_add(string_bytes(&item));
        if bytes > MAX_LIST_BYTES {
            return Err(AppError::reason(
                "too-large",
                format!("fields: the page's fields exceed the {MAX_LIST_BYTES} byte ceiling"),
            ));
        }
        items.push(Value::Object(item));
    }
    Ok(reply(
        request,
        json!({"page": page, "tabs": tabs, "items": items}),
    ))
}

/// The bytes of an item's strings.
fn string_bytes(item: &Map<String, Value>) -> usize {
    let size = |value: &Value| match value {
        Value::String(text) => text.len(),
        Value::Array(items) => (items.iter())
            .map(|item| item.as_str().map_or(0, str::len))
            .sum(),
        _ => 0,
    };
    item.values().map(size).sum()
}

/// engine.ts `listFields`' item for one widget; None for a push button or
/// anything else that is not a fillable field. Strings are cut where
/// `MAX_LISTED_VALUE_CHARS`, `forms::MAX_NAME_CHARS` and
/// `MAX_LISTED_OPTIONS` say.
fn item(
    doc: &Document<'_>,
    space: &PageSpace,
    scripts: &ScriptContext,
    widget: Widget<'_>,
) -> Option<Map<String, Value>> {
    let dict = widget.dict;
    let kind = forms::kind(doc, dict)?;
    let name = forms::full_name(doc, widget.id, dict);
    let label = forms::label(doc, dict);
    let flags = field_flags(doc, dict);
    let mut item = Map::new();
    item.insert("id".into(), widget.number().into());
    item.insert("name".into(), name.clone().into());
    let label = if label.is_empty() { name } else { label };
    item.insert("label".into(), label.into());
    item.insert("kind".into(), kind.as_str().into());
    let value = if kind.is_button() {
        Value::Bool(button_checked(doc, widget.id, dict))
    } else {
        forms::value_prefix(doc, dict, MAX_LISTED_VALUE_CHARS).into()
    };
    item.insert("value".into(), value);
    item.insert("readOnly".into(), (flags & READ_ONLY != 0).into());
    item.insert("required".into(), (flags & REQUIRED != 0).into());
    let scripted = scripts.scripted(doc, widget.id, dict);
    item.insert("scripted".into(), scripted.into());
    let rect = space.to_page(forms::pdf_rect(doc, dict));
    item.insert(
        "rect".into(),
        rect.iter().map(|&v| json_number(v)).collect(),
    );
    let (_, own) = own_dict(doc, kind, widget.id, dict);
    let name_cut = |name: &[u8]| {
        let name = name.get(..4 * forms::MAX_NAME_CHARS).unwrap_or(name);
        forms::cut_chars(forms::name_text(name), forms::MAX_NAME_CHARS)
    };
    let tu = doc.dict_get(own, b"TU");
    if tu.as_string().is_some() {
        let tu = forms::text_prefix(tu, forms::MAX_NAME_CHARS);
        item.insert("tu".into(), tu.into());
    }
    match (kind, doc.dict_get(own, b"DV")) {
        (Kind::Checkbox, Object::Name(dv)) => {
            item.insert("default".into(), (dv.as_slice() != b"Off").into());
        }
        (Kind::Radio, Object::Name(dv)) => {
            item.insert("default".into(), name_cut(dv).into());
        }
        (Kind::Checkbox | Kind::Radio, _) => {}
        (_, dv @ Object::String(_)) => {
            let dv = forms::text_prefix(dv, MAX_LISTED_VALUE_CHARS);
            item.insert("default".into(), dv.into());
        }
        _ => {}
    }
    if kind.is_button() {
        item.insert("on".into(), name_cut(&on_state(doc, dict)).into());
    }
    if kind == Kind::Signature {
        let own_value = !doc.dict_get(dict, b"V").is_null();
        let parent = doc.dict_get(dict, b"Parent").as_dict();
        let parent_value = parent.is_some_and(|parent| !doc.dict_get(parent, b"V").is_null());
        item.insert("signed".into(), (own_value || parent_value).into());
    }
    if matches!(kind, Kind::Combo | Kind::List) {
        let max = forms::MAX_NAME_CHARS;
        let options = forms::choices_range(doc, dict, 0, MAX_LISTED_OPTIONS, max);
        let exports: Vec<String> = options.into_iter().map(|choice| choice.export).collect();
        item.insert("options".into(), exports.into());
    }
    if kind == Kind::Combo {
        item.insert("edit".into(), (flags & EDIT != 0).into());
    }
    if kind == Kind::Text {
        let max_len = forms::max_len(doc, dict);
        if max_len > 0 {
            item.insert("maxLen".into(), max_len.into());
        }
        if flags & MULTILINE != 0 {
            item.insert("multiline".into(), true.into());
        }
    }
    Some(item)
}

/// A page-space coordinate as a JSON number: an integer when it is one.
fn json_number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9e15 {
        (value as i64).into()
    } else {
        request::js_number_value(value)
    }
}

/// engine.ts `findWidget`: the widget object `widgetId` names on the page.
fn find_widget(
    doc: &Document<'_>,
    page: &Dictionary,
    widget_id: Option<&Value>,
    op: &str,
) -> Result<ObjectId, AppError> {
    let Some(target) = widget_id.and_then(request::as_int) else {
        return Err(fail("invalid", format!("{op}: id must be an integer")));
    };
    let found = page_widgets(doc, page)
        .into_iter()
        .filter_map(|widget| widget.id)
        .find(|id| i64::from(id.number()) == target);
    found.ok_or_else(|| {
        fail(
            "stale-target",
            format!(
                "{op}: no form field {} on this page",
                request::js_string(widget_id)
            ),
        )
    })
}

fn widget_dict<'d>(doc: &'d Document<'_>, id: ObjectId) -> Result<&'d Dictionary, AppError> {
    doc.resolve_id(id)
        .as_dict()
        .ok_or_else(|| plain("the widget is not a dictionary"))
}

/// What a fill writes.
enum Fill {
    /// A check box or radio button to this state.
    Button(bool),
    Text(String),
    Choice(String),
}

/// engine.ts `field-set`.
pub fn set(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let app = session.require_mutable("field-set")?;
    let page = session.check_page(request.get("page"), "field-set")?;
    let doc = app.document();
    let (_, page_dict) = page_of(app, page)?;
    let id = find_widget(doc, page_dict, request.get("widgetId"), "field-set")?;
    let dict = widget_dict(doc, id)?;
    let Some(kind) = forms::kind(doc, dict) else {
        return Err(plain("field-set: that widget is not a fillable form field"));
    };
    if kind == Kind::Signature {
        return Err(fail(
            "signature",
            "field-set: click the signature field to sign it",
        ));
    }
    if field_flags(doc, dict) & READ_ONLY != 0 {
        return Err(fail(
            "field-read-only",
            "field-set: that field is read-only",
        ));
    }
    let value = request.get("value");
    let fill = if kind.is_button() {
        let Some(on) = value.and_then(Value::as_bool) else {
            return Err(plain(format!(
                "field-set: a {} takes true or false",
                kind.as_str()
            )));
        };
        if kind == Kind::Radio && !on {
            return Err(plain(
                "field-set: a radio cannot be turned off, only switched",
            ));
        }
        if button_checked(doc, Some(id), dict) == on {
            return Ok(edit_reply(session, request));
        }
        Fill::Button(on)
    } else {
        let Some(text) = value.and_then(Value::as_str) else {
            return Err(plain(format!(
                "field-set: a {} takes a string",
                kind.as_str()
            )));
        };
        if js_len(text) > MAX_NOTE_CHARS {
            return Err(plain(format!(
                "field-set: a value holds at most {MAX_NOTE_CHARS} characters"
            )));
        }
        if kind == Kind::Text {
            let max_len = forms::max_len(doc, dict);
            if max_len > 0 && js_len(text) as i64 > max_len {
                return Err(plain(format!(
                    "field-set: this field holds at most {max_len} characters"
                )));
            }
            Fill::Text(text.to_string())
        } else {
            let editable = kind == Kind::Combo && field_flags(doc, dict) & EDIT != 0;
            if !editable && !forms::options(doc, dict).iter().any(|o| o == text) {
                let quoted = serde_json::to_string(text).unwrap_or_default();
                return Err(plain(format!(
                    "field-set: {quoted} is not one of this field's options"
                )));
            }
            Fill::Choice(text.to_string())
        }
    };
    session.mutate("fill field", |session| {
        let mut tx = session.tx()?;
        match fill {
            Fill::Button(on) => set_button(&mut tx, id, on),
            Fill::Text(text) => set_value(&mut tx, id, &text, false),
            Fill::Choice(text) => set_value(&mut tx, id, &text, true),
        }
    })?;
    Ok(edit_reply(session, request))
}

/// MuPDF's `toggle_check_box` to the wanted state: /V on the field (the
/// nearest node with /T) and /AS on each of its widgets, any page: the
/// state where that widget's /AP /N has it, else Off.
fn set_button(tx: &mut Transaction<'_>, id: ObjectId, on: bool) -> Result<(), AppError> {
    let (head, state, writes) = {
        let doc = tx.document();
        let dict = widget_dict(doc, id)?;
        let state = if on {
            on_name(doc, dict)
        } else {
            b"Off".to_vec()
        };
        let (head, _) = field_head(doc, Some(id), dict);
        let head = head.unwrap_or(id);
        let mut writes = Vec::new();
        for widget in group_widgets(doc, head, id) {
            let Some(kid) = doc.resolve_id(widget).as_dict() else {
                continue;
            };
            let normal = doc.dict_get(doc.dict_get(kid, b"AP").as_dict().unwrap_or(&EMPTY), b"N");
            let shown = match normal.as_dict() {
                Some(states) if !states.contains_key(state.as_slice()) => b"Off".to_vec(),
                _ => state.clone(),
            };
            if doc.dict_get(kid, b"AS").as_name() != Some(shown.as_slice()) {
                writes.push((widget, shown));
            }
        }
        (head, state, writes)
    };
    tx.update(head, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.insert(b"V".to_vec(), Object::Name(state));
        }
    })?;
    for (widget, shown) in writes {
        tx.update(widget, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"AS".to_vec(), Object::Name(shown));
            }
        })?;
    }
    Ok(())
}

static EMPTY: Dictionary = Dictionary::new();

/// The widgets of the field `head`, `widget` (the one filled) among them
/// even when its field does not list it.
fn group_widgets(doc: &Document<'_>, head: ObjectId, widget: ObjectId) -> Vec<ObjectId> {
    let mut widgets = field_widgets(doc, Some(head), MAX_FIELD_WIDGETS);
    if !widgets.contains(&widget) {
        widgets.push(widget);
    }
    widgets
}

/// MuPDF's `pdf_button_field_on_state`: the first /AP /N key that is not
/// Off, else /AP /D's, else Yes.
fn on_name(doc: &Document<'_>, dict: &Dictionary) -> Vec<u8> {
    let on = on_state(doc, dict);
    if !on.is_empty() {
        return on;
    }
    let ap = doc.dict_get(dict, b"AP").as_dict().unwrap_or(&EMPTY);
    (doc.dict_get(ap, b"D").as_dict())
        .and_then(|states| states.keys().find(|key| key.as_slice() != b"Off"))
        .cloned()
        .unwrap_or_else(|| FIELD_CHECK_ON.to_vec())
}

/// A text or choice value: /V on the field (the nearest node with /T), any
/// /V below it on the widget's chain removed, a choice's /I dropped from the
/// widget up 32 nodes (engine.ts `dropChoiceIndices`), and every widget of
/// the field redrawn.
fn set_value(
    tx: &mut Transaction<'_>,
    id: ObjectId,
    text: &str,
    choice: bool,
) -> Result<(), AppError> {
    let (head, below, indices, widgets) = {
        let doc = tx.document();
        let dict = widget_dict(doc, id)?;
        let (head, _) = field_head(doc, Some(id), dict);
        let head = head.unwrap_or(id);
        let chain = lineage(doc, Some(id), dict, 32);
        let below: Vec<ObjectId> = (chain.iter())
            .map_while(|(node, dict)| Some((node.filter(|node| *node != head)?, dict)))
            .filter(|(_, dict)| dict.contains_key(b"V".as_slice()))
            .map(|(node, _)| node)
            .collect();
        let indices: Vec<ObjectId> = (chain.iter())
            .filter(|(_, node)| choice && node.contains_key(b"I".as_slice()))
            .filter_map(|(node, _)| *node)
            .collect();
        let widgets = group_widgets(doc, head, id);
        (head, below, indices, widgets)
    };
    for node in below {
        tx.update(node, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.remove(b"V".as_slice());
            }
        })?;
    }
    tx.update(head, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.insert(b"V".to_vec(), Object::text(text));
        }
    })?;
    for node in indices {
        tx.update(node, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.remove(b"I".as_slice());
            }
        })?;
    }
    let mut sink = TxSink::new(tx);
    for widget in widgets {
        refresh(&mut sink, widget, None)?;
    }
    Ok(())
}

// ------------------------------------------------------------ validation

/// engine.ts `checkFieldName`.
fn check_name(name: Option<&Value>, op: &str) -> Result<String, AppError> {
    let name = name.and_then(Value::as_str).filter(|name| {
        let len = js_len(name);
        (1..=MAX_FIELD_NAME_CHARS).contains(&len)
    });
    let Some(name) = name else {
        return Err(fail(
            "invalid-name",
            format!("{op}: a field name is 1..{MAX_FIELD_NAME_CHARS} characters"),
        ));
    };
    if js_trim(name) != name || name.contains('.') || has_control(name) {
        return Err(fail(
            "invalid-name",
            format!("{op}: a field name holds no dots, control characters or outer spaces"),
        ));
    }
    Ok(name.to_string())
}

/// `[\u0000-\u001f\u007f]`.
fn has_control(text: &str) -> bool {
    text.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
}

/// JavaScript's `String.prototype.trim`: WhiteSpace and LineTerminator.
fn js_trim(text: &str) -> &str {
    let white = |c: char| {
        matches!(
            c,
            '\u{9}'
                | '\u{a}'
                | '\u{b}'
                | '\u{c}'
                | '\u{d}'
                | ' '
                | '\u{a0}'
                | '\u{1680}'
                | '\u{2000}'
                ..='\u{200a}'
                    | '\u{2028}'
                    | '\u{2029}'
                    | '\u{202f}'
                    | '\u{205f}'
                    | '\u{3000}'
                    | '\u{feff}'
        )
    };
    text.trim_matches(white)
}

/// engine.ts `checkFieldLabel`.
fn check_label(label: &Value, op: &str) -> Result<String, AppError> {
    let label = label
        .as_str()
        .filter(|label| js_len(label) <= MAX_FIELD_LABEL);
    let Some(label) = label else {
        return Err(fail(
            "invalid",
            format!("{op}: a label holds at most {MAX_FIELD_LABEL} characters"),
        ));
    };
    if has_control(label) {
        return Err(fail(
            "invalid",
            format!("{op}: a label holds no control characters"),
        ));
    }
    Ok(label.to_string())
}

/// engine.ts `checkFieldFlag`.
fn check_flag(value: &Value, op: &str, what: &str) -> Result<bool, AppError> {
    value
        .as_bool()
        .ok_or_else(|| fail("invalid", format!("{op}: {what} is true or false")))
}

/// engine.ts `checkFieldRect` in the page box.
fn check_rect(bounds: [f64; 4], rect: Option<&Value>, op: &str) -> Result<[f64; 4], AppError> {
    let numbers: Option<Vec<f64>> = rect
        .and_then(Value::as_array)
        .filter(|items| items.len() == 4)
        .and_then(|items| items.iter().map(request::as_number).collect());
    let Some([x0, y0, x1, y1]) = numbers.and_then(|n| <[f64; 4]>::try_from(n).ok()) else {
        return Err(fail(
            "invalid",
            format!("{op}: rect must be 4 finite numbers"),
        ));
    };
    if x1 - x0 < MIN_FIELD_RECT_PT || y1 - y0 < MIN_FIELD_RECT_PT {
        return Err(fail("invalid", format!("{op}: a field is at least 4x4 pt")));
    }
    let on_page =
        |x: f64, y: f64| x >= bounds[0] && x <= bounds[2] && y >= bounds[1] && y <= bounds[3];
    if !on_page(x0, y0) || !on_page(x1, y1) {
        let n = request::js_number;
        return Err(fail(
            "invalid",
            format!(
                "{op}: rect [{}, {}, {}, {}] is off the page",
                n(x0),
                n(y0),
                n(x1),
                n(y1)
            ),
        ));
    }
    Ok([x0, y0, x1, y1])
}

/// engine.ts `checkOptionList`.
fn check_options(options: Option<&Value>, op: &str) -> Result<Vec<String>, AppError> {
    let items = options
        .and_then(Value::as_array)
        .filter(|items| (1..=MAX_FIELD_OPTIONS).contains(&items.len()));
    let Some(items) = items else {
        return Err(fail(
            "invalid",
            format!("{op}: options must be 1..{MAX_FIELD_OPTIONS} entries"),
        ));
    };
    let mut list: Vec<String> = Vec::new();
    for option in items {
        let option = option
            .as_str()
            .filter(|o| (1..=MAX_FIELD_OPTION_CHARS).contains(&js_len(o)));
        let Some(option) = option else {
            return Err(fail(
                "invalid",
                format!("{op}: every option is 1..{MAX_FIELD_OPTION_CHARS} characters"),
            ));
        };
        if list.iter().any(|seen| seen == option) {
            return Err(fail("invalid", format!("{op}: options must be unique")));
        }
        list.push(option.to_string());
    }
    Ok(list)
}

/// One radio kid of a field-add.
struct KidSpec {
    on: String,
    rect: [f64; 4],
}

/// engine.ts `checkKids`.
fn check_kids(bounds: [f64; 4], kids: Option<&Value>, op: &str) -> Result<Vec<KidSpec>, AppError> {
    let items = kids
        .and_then(Value::as_array)
        .filter(|items| (2..=MAX_FIELD_KIDS).contains(&items.len()));
    let Some(items) = items else {
        return Err(fail(
            "invalid",
            format!("{op}: a radio group takes 2..{MAX_FIELD_KIDS} kids"),
        ));
    };
    let mut specs: Vec<KidSpec> = Vec::new();
    for (index, kid) in items.iter().enumerate() {
        let on = kid
            .get("on")
            .and_then(Value::as_str)
            .filter(|on| (1..=MAX_FIELD_KID_NAME).contains(&js_len(on)) && *on != "Off");
        let Some(on) = on else {
            return Err(fail(
                "invalid",
                format!(
                    "{op}: kid {index} needs an export name of 1..{MAX_FIELD_KID_NAME} characters, never Off"
                ),
            ));
        };
        if specs.iter().any(|spec| spec.on == on) {
            return Err(fail(
                "invalid",
                format!("{op}: kid export names must be unique"),
            ));
        }
        let rect = check_rect(bounds, kid.get("rect"), op)?;
        specs.push(KidSpec {
            on: on.to_string(),
            rect,
        });
    }
    Ok(specs)
}

/// A field default as engine.ts `checkFieldDefault` reads it: None removes
/// /DV.
#[derive(Clone, Debug, PartialEq)]
enum Default {
    Check(bool),
    Text(String),
}

/// What a default is checked against: the field as the request leaves it.
struct Resulting<'a> {
    max_len: i64,
    options: &'a [String],
    kids: &'a [String],
    edit: bool,
}

/// engine.ts `checkFieldDefault`.
fn check_default(
    kind: Kind,
    value: &Value,
    resulting: &Resulting<'_>,
    op: &str,
) -> Result<Option<Default>, AppError> {
    if kind == Kind::Checkbox {
        let Some(on) = value.as_bool() else {
            return Err(fail(
                "invalid",
                format!("{op}: a checkbox default is true or false"),
            ));
        };
        return Ok(Some(Default::Check(on)));
    }
    let Some(text) = value.as_str() else {
        return Err(fail(
            "invalid",
            format!("{op}: a {} default is a string", kind.as_str()),
        ));
    };
    if text.is_empty() {
        return Ok(None);
    }
    let too_long = || {
        fail(
            "invalid",
            format!("{op}: a default holds at most {MAX_FIELD_TEXT_CHARS} characters"),
        )
    };
    match kind {
        Kind::Text => {
            if js_len(text) > MAX_FIELD_TEXT_CHARS {
                return Err(too_long());
            }
            if resulting.max_len > 0 && js_len(text) as i64 > resulting.max_len {
                return Err(fail(
                    "invalid",
                    format!(
                        "{op}: the default is longer than maxLen {}",
                        resulting.max_len
                    ),
                ));
            }
        }
        Kind::Combo if resulting.edit => {
            if js_len(text) > MAX_FIELD_TEXT_CHARS {
                return Err(too_long());
            }
        }
        Kind::Combo => {
            if !resulting.options.iter().any(|o| o == text) {
                return Err(fail(
                    "invalid",
                    format!("{op}: the default is not one of this field's options"),
                ));
            }
        }
        Kind::Radio => {
            if !resulting.kids.iter().any(|k| k == text) {
                return Err(fail(
                    "invalid",
                    format!("{op}: the default is not one of this group's kid names"),
                ));
            }
        }
        _ => {
            return Err(fail(
                "invalid",
                format!("{op}: a {} takes no default value", kind.as_str()),
            ));
        }
    }
    Ok(Some(Default::Text(text.to_string())))
}

/// A full name in the field tree walk: its text while it can still equal a
/// requested name (at most `MAX_FIELD_NAME_CHARS` UTF-16 units), else only
/// that it is longer.
#[derive(Clone)]
enum TreeName {
    Short(String),
    Long,
}

/// A /T whose bytes pass this decodes to more than `MAX_FIELD_NAME_CHARS`
/// UTF-16 units in any text string encoding (a unit takes at most 4 bytes).
const MAX_NAME_BYTES: usize = 4 * MAX_FIELD_NAME_CHARS + 4;

/// engine.ts `fieldTreeNames`: every full name in /AcroForm /Fields with a
/// string /T, `too-many` past `MAX_FIELD_NODES` visited nodes; nodes deeper
/// than eight are not visited. Only names a requested name can equal (at
/// most `MAX_FIELD_NAME_CHARS` UTF-16 units) are kept, and each node's /Kids
/// are walked one at a time, so work and memory stay bounded by the visited
/// nodes whatever the /T lengths and /Kids sizes.
pub(crate) fn field_tree_names(
    doc: &Document<'_>,
    op: &str,
) -> Result<Option<BTreeSet<String>>, AppError> {
    let Some(fields) = existing_fields(doc) else {
        return Ok(None);
    };
    let mut names = BTreeSet::new();
    let mut visited = 0usize;
    // The kids being walked, the next one, their prefix and their depth.
    let mut stack: Vec<(&[Object], usize, TreeName, usize)> =
        vec![(fields, 0, TreeName::Short(String::new()), 1)];
    while let Some(frame) = stack.last_mut() {
        let kids: &[Object] = frame.0;
        let Some(node) = kids.get(frame.1) else {
            stack.pop();
            continue;
        };
        frame.1 += 1;
        let (prefix, depth) = (frame.2.clone(), frame.3);
        visited += 1;
        if visited > MAX_FIELD_NODES {
            return Err(fail(
                "too-many",
                format!("{op}: this form is too large to design fields in"),
            ));
        }
        let Some(dict) = doc.resolve(node).as_dict() else {
            continue;
        };
        let own = doc.dict_get(dict, b"T").as_string();
        let own_name = own.map(|own| match own.len() > MAX_NAME_BYTES {
            true => TreeName::Long,
            false => TreeName::Short(crate::object::decode_text_string(own)),
        });
        let full = match (own_name, prefix) {
            (Some(TreeName::Short(own)), prefix) if own.is_empty() => prefix,
            (Some(TreeName::Short(own)), TreeName::Short(prefix)) => {
                let joined = if prefix.is_empty() {
                    own
                } else {
                    format!("{prefix}.{own}")
                };
                match js_len(&joined) > MAX_FIELD_NAME_CHARS {
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
        if depth < forms::MAX_FIELD_DEPTH
            && let Some(kids) = doc.dict_get(dict, b"Kids").as_array()
        {
            stack.push((kids, 0, full, depth + 1));
        }
    }
    Ok(Some(names))
}

/// engine.ts `existingFields`: /AcroForm /Fields when it is an array.
fn existing_fields<'d>(doc: &'d Document<'_>) -> Option<&'d [Object]> {
    let catalog = doc.resolve_id(doc.root()).as_dict()?;
    let acro = doc.dict_get(catalog, b"AcroForm").as_dict()?;
    doc.dict_get(acro, b"Fields").as_array()
}

/// engine.ts `checkNameFree`.
fn check_name_free(
    doc: &Document<'_>,
    name: &str,
    except: Option<&str>,
    op: &str,
) -> Result<(), AppError> {
    let Some(mut taken) = field_tree_names(doc, op)? else {
        return Ok(());
    };
    if let Some(except) = except {
        taken.remove(except);
    }
    if taken.contains(name) {
        return Err(fail(
            "duplicate-name",
            format!("{op}: a field named {name} already exists"),
        ));
    }
    Ok(())
}

/// engine.ts `isStructuralRadioParent`, on the node's own keys.
fn structural_radio_parent(doc: &Document<'_>, parent: &Dictionary) -> bool {
    if !doc.dict_get(parent, b"Parent").is_null()
        || doc.dict_get(parent, b"FT").as_name() != Some(b"Btn")
    {
        return false;
    }
    let bits = js_int32(doc.dict_get(parent, b"Ff"));
    if bits & RADIO == 0 || bits & forms::PUSHBUTTON != 0 {
        return false;
    }
    let Some(kids) = doc
        .dict_get(parent, b"Kids")
        .as_array()
        .filter(|k| !k.is_empty())
    else {
        return false;
    };
    kids.iter().all(|kid| {
        let kid = doc.resolve(kid).as_dict().unwrap_or(&EMPTY);
        doc.dict_get(kid, b"T").is_null()
            && doc.dict_get(kid, b"Subtype").as_name() == Some(b"Widget")
    })
}

/// A number as JavaScript's bitwise operators read it (ToInt32); 0 for
/// anything else.
fn js_int32(value: &Object) -> i64 {
    let Some(number) = value.as_f64().filter(|n| n.is_finite()) else {
        return 0;
    };
    let wrapped = number.trunc().rem_euclid(4_294_967_296.0);
    let wrapped = if wrapped >= 2_147_483_648.0 {
        wrapped - 4_294_967_296.0
    } else {
        wrapped
    };
    wrapped as i64
}

/// engine.ts `isTopLevelField`, by object number.
fn top_level(doc: &Document<'_>, node: Option<ObjectId>) -> bool {
    let (Some(node), Some(fields)) = (node, existing_fields(doc)) else {
        return false;
    };
    fields.iter().any(|entry| {
        entry
            .as_reference()
            .is_some_and(|id| id.number() == node.number())
    })
}

/// The widget's /Parent: its object and dictionary.
fn parent_of<'d>(
    doc: &'d Document<'_>,
    dict: &'d Dictionary,
) -> Option<(Option<ObjectId>, &'d Dictionary)> {
    let (holder, value) = doc.resolve_with_holder(dict.get(b"Parent".as_slice())?);
    value.as_dict().map(|parent| (holder, parent))
}

/// engine.ts `checkFieldEditable`.
fn check_editable(
    app: &AppDocument,
    id: ObjectId,
    dict: &Dictionary,
    op: &str,
) -> Result<Kind, AppError> {
    let doc = app.document();
    if app.info(false).xfa {
        return Err(fail(
            "xfa",
            format!("{op}: this is an XFA form; fields cannot be authored"),
        ));
    }
    let Some(kind) = forms::kind(doc, dict) else {
        return Err(fail(
            "invalid",
            format!("{op}: that widget is not a form field"),
        ));
    };
    if kind == Kind::Signature && !doc.dict_get(dict, b"V").is_null() {
        return Err(fail(
            "signature",
            format!("{op}: a signed field cannot be changed"),
        ));
    }
    if ScriptContext::of(doc).scripted(doc, Some(id), dict) {
        return Err(fail(
            "scripted",
            format!("{op}: this field runs a script this app does not execute"),
        ));
    }
    Ok(kind)
}

/// The structural radio parent a widget may be changed through; refused
/// for any other /Parent (engine.ts' "part of a field hierarchy").
fn structural_parent(
    doc: &Document<'_>,
    dict: &Dictionary,
    message: &str,
) -> Result<Option<ObjectId>, AppError> {
    if doc.dict_get(dict, b"Parent").is_null() {
        return Ok(None);
    }
    if let Some((holder, parent)) = parent_of(doc, dict)
        && structural_radio_parent(doc, parent)
        && top_level(doc, holder)
    {
        return Ok(holder);
    }
    Err(fail("invalid", message.to_string()))
}

// ------------------------------------------------------------ writing

/// engine.ts `ensureAcroForm`: the /AcroForm, with whatever of /Fields,
/// /DR /Font /Helv and /DA it lacks; returns where /Fields is.
fn ensure_acroform(tx: &mut Transaction<'_>) -> Result<Place, AppError> {
    let doc = tx.document();
    let (catalog_id, catalog) = doc.resolve_id_with_holder(doc.root());
    let (Some(catalog_id), Some(catalog)) = (catalog_id, catalog.as_dict()) else {
        return Err(fail("invalid", "field-add: the document has no catalog"));
    };
    if doc.dict_get(catalog, b"AcroForm").as_dict().is_none() {
        let helv = tx.allocate(Object::Dictionary(base14_font(b"Helvetica")), None)?;
        let fonts = Dictionary::from([(b"Helv".to_vec(), Object::Reference(helv))]);
        let resources = Dictionary::from([(b"Font".to_vec(), Object::Dictionary(fonts))]);
        let acro = Object::dictionary([
            ("Fields", Object::Array(Vec::new())),
            ("DA", Object::text(FIELD_DA)),
            ("DR", Object::Dictionary(resources)),
        ]);
        let acro = tx.allocate(acro, None)?;
        tx.update(catalog_id, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"AcroForm".to_vec(), Object::Reference(acro));
            }
        })?;
        return Ok(Place::object(acro).child(tx.document(), b"Fields"));
    }
    let acro = Place::object(catalog_id).child(doc, b"AcroForm");
    let missing = |place: &Place, key: &[u8], doc: &Document<'_>| {
        place
            .read(doc)
            .as_dict()
            .is_none_or(|dict| doc.dict_get(dict, key).is_null())
    };
    if missing(&acro, b"Fields", tx.document()) || place_array(tx.document(), &acro, b"Fields") {
        acro.update(tx, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"Fields".to_vec(), Object::Array(Vec::new()));
            }
        })?;
    }
    if missing(&acro, b"DR", tx.document()) {
        acro.update(tx, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"DR".to_vec(), Object::Dictionary(Dictionary::new()));
            }
        })?;
    }
    let dr = acro.child(tx.document(), b"DR");
    if missing(&dr, b"Font", tx.document()) {
        dr.update(tx, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"Font".to_vec(), Object::Dictionary(Dictionary::new()));
            }
        })?;
    }
    let fonts = dr.child(tx.document(), b"Font");
    if missing(&fonts, b"Helv", tx.document()) {
        let helv = tx.allocate(Object::Dictionary(base14_font(b"Helvetica")), None)?;
        fonts.update(tx, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"Helv".to_vec(), Object::Reference(helv));
            }
        })?;
    }
    if missing(&acro, b"DA", tx.document()) {
        acro.update(tx, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"DA".to_vec(), Object::text(FIELD_DA));
            }
        })?;
    }
    Ok(acro.child(tx.document(), b"Fields"))
}

/// The value at `key` of the dictionary at `place` is present but not an
/// array (replaced: nothing can be appended to it).
fn place_array(doc: &Document<'_>, place: &Place, key: &[u8]) -> bool {
    place
        .read(doc)
        .as_dict()
        .map(|dict| doc.dict_get(dict, key))
        .is_some_and(|value| !value.is_null() && value.as_array().is_none())
}

/// Appends `items` to the array at `place`.
fn push<E: Edit>(target: &mut E, place: &Place, items: &[ObjectId]) -> Result<(), AppError> {
    place.update(target, |value| {
        if let Some(array) = value.as_array_mut() {
            array.extend(items.iter().map(|id| Object::Reference(*id)));
        }
    })?;
    Ok(())
}

/// engine.ts `directAnnots`: the page's /Annots as a new direct array (so a
/// shared array is never changed), with `extra` appended.
fn direct_annots<E: Edit>(
    tx: &mut E,
    page: ObjectId,
    keep: impl Fn(&Object) -> bool,
    extra: &[ObjectId],
) -> Result<(), AppError> {
    let current: Vec<Object> = {
        let doc = tx.document();
        let page_dict = widget_dict(doc, page)?;
        (doc.dict_get(page_dict, b"Annots").as_array())
            .unwrap_or(&[])
            .iter()
            .filter(|entry| keep(entry))
            .cloned()
            .collect()
    };
    let mut annots = current;
    annots.extend(extra.iter().map(|id| Object::Reference(*id)));
    tx.update(page, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.insert(b"Annots".to_vec(), Object::Array(annots));
        }
    })?;
    Ok(())
}

/// The kinds field-add creates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AddKind {
    Text,
    Checkbox,
    Combo,
    Radio,
    Signature,
}

impl AddKind {
    fn kind(self) -> Kind {
        match self {
            Self::Text => Kind::Text,
            Self::Checkbox => Kind::Checkbox,
            Self::Combo => Kind::Combo,
            Self::Radio => Kind::Radio,
            Self::Signature => Kind::Signature,
        }
    }
}

/// A validated field-add.
struct AddPlan {
    kind: AddKind,
    name: String,
    page: usize,
    page_id: ObjectId,
    rect: Option<[f64; 4]>,
    kids: Vec<KidSpec>,
    options: Vec<String>,
    flags: i64,
    max_len: i64,
    label: Option<String>,
    default: Option<Option<Default>>,
    value: Option<Object>,
    state: Option<Vec<u8>>,
    radio_value: String,
    radio_on: String,
    rotate: i64,
}

/// What field-add made: its name and widget objects.
pub struct Added {
    pub name: String,
    pub ids: Vec<ObjectId>,
    pub radio: bool,
}

/// engine.ts `field-add`.
pub fn add(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let added = add_field(session, request, None)?;
    let mut fields = Map::new();
    let first = added.ids.first().map_or(0, |id| id.number());
    fields.insert("widgetId".into(), first.into());
    fields.insert("name".into(), added.name.into());
    if added.radio {
        let ids: Vec<Value> = added.ids.iter().map(|id| id.number().into()).collect();
        fields.insert("widgetIds".into(), ids.into());
    }
    fields.extend(session.edit_state());
    Ok(reply(request, Value::Object(fields)))
}

/// engine.ts `addField`: validates everything, then writes one "add field"
/// step. `value` is clip-paste's own value (a string or a boolean).
pub fn add_field(
    session: &mut AppSession,
    request: &Value,
    value: Option<&Value>,
) -> Result<Added, AppError> {
    let plan = plan_add(session, request, value)?;
    let Some(added) = session.mutate("add field", |session| {
        let mut tx = session.tx()?;
        write_add(&mut tx, &plan)
    })?
    else {
        // A batch plan: the name it would take, no ids.
        return Ok(Added {
            name: plan.name,
            ids: Vec::new(),
            radio: plan.kind == AddKind::Radio,
        });
    };
    Ok(added)
}

fn plan_add(
    session: &AppSession,
    request: &Value,
    value_override: Option<&Value>,
) -> Result<AddPlan, AppError> {
    let op = "field-add";
    let app = session.require_mutable(op)?;
    if app.info(session.locked).xfa {
        return Err(fail(
            "xfa",
            "field-add: this is an XFA form; fields cannot be added",
        ));
    }
    let page = session.check_page(request.get("page"), op)?;
    let doc = app.document();
    let kind = match request.get("kind").and_then(Value::as_str) {
        Some("text") => AddKind::Text,
        Some("checkbox") => AddKind::Checkbox,
        Some("combo") => AddKind::Combo,
        Some("radio") => AddKind::Radio,
        Some("signature") => AddKind::Signature,
        _ => {
            return Err(fail(
                "invalid",
                "field-add: kind must be one of text, checkbox, combo, radio, signature",
            ));
        }
    };
    let has = |key: &str| request.get(key).is_some();
    let auto_name = kind == AddKind::Signature
        && match request.get("name") {
            None => true,
            Some(Value::String(name)) => name.is_empty(),
            Some(_) => false,
        };
    let mut name = if auto_name {
        String::new()
    } else {
        check_name(request.get("name"), op)?
    };
    if kind == AddKind::Signature {
        if has("default") {
            return Err(fail(
                "invalid",
                "field-add: default is not a signature property",
            ));
        }
        if has("options") {
            return Err(fail(
                "invalid",
                "field-add: options are a dropdown property",
            ));
        }
        if has("kids") {
            return Err(fail("invalid", "field-add: kids are a radio property"));
        }
    }
    let (page_id, page_dict) = page_of(app, page)?;
    let space = PageSpace::of(doc, page_dict);
    let bounds = space.bounds();
    let mut rect = None;
    let mut kids = Vec::new();
    let mut options = Vec::new();
    let mut ff = 0;
    let mut value: Option<Object> = None;
    if kind == AddKind::Radio {
        kids = check_kids(bounds, request.get("kids"), op)?;
        ff = RADIO;
    } else {
        let given = request
            .get("rect")
            .filter(|v| !v.is_null() || kind == AddKind::Signature);
        let fallback = match kind {
            AddKind::Checkbox => json!([72, 72, 88, 88]),
            _ => json!([72, 72, 252, 96]),
        };
        let checked = match (kind, given) {
            (AddKind::Signature, given) => check_rect(bounds, given, op)?,
            (_, Some(given)) => check_rect(bounds, Some(given), op)?,
            (_, None) => check_rect(bounds, Some(&fallback), op)?,
        };
        rect = Some(checked);
        match kind {
            AddKind::Signature => {
                if checked[2] - checked[0] < 40.0 || checked[3] - checked[1] < 20.0 {
                    return Err(fail(
                        "invalid",
                        "field-add: a signature field is at least 40x20 pt",
                    ));
                }
            }
            AddKind::Checkbox => value = Some(Object::name("Off")),
            AddKind::Text => value = Some(Object::text("")),
            _ => {
                options = check_options(request.get("options"), op)?;
                value = options.first().map(|first| Object::text(first));
            }
        }
    }
    if kind == AddKind::Combo {
        ff = COMBO;
    }
    if let Some(edit) = request.get("edit") {
        if kind != AddKind::Combo {
            return Err(fail("invalid", "field-add: edit is a dropdown property"));
        }
        if check_flag(edit, op, "edit")? {
            ff |= EDIT;
        }
    }
    let mut max_len = 0;
    if kind == AddKind::Text {
        if let Some(given) = request.get("maxLen") {
            match request::as_int(given) {
                Some(n) if (0..=MAX_FIELD_TEXT_CHARS as i64).contains(&n) => max_len = n,
                _ => {
                    return Err(fail(
                        "invalid",
                        format!("field-add: maxLen is an integer 0..{MAX_FIELD_TEXT_CHARS}"),
                    ));
                }
            }
        }
        if let Some(multiline) = request.get("multiline")
            && check_flag(multiline, op, "multiline")?
        {
            ff |= MULTILINE;
        }
    } else if has("maxLen") || has("multiline") {
        return Err(fail(
            "invalid",
            "field-add: maxLen and multiline are text properties",
        ));
    }
    let label = match request.get("label") {
        None => None,
        Some(Value::String(text)) if text.is_empty() => None,
        Some(label) => Some(check_label(label, op)?),
    };
    let read_only = match request.get("readOnly") {
        None => false,
        Some(flag) => check_flag(flag, op, "readOnly")?,
    };
    let required = match request.get("required") {
        None => false,
        Some(flag) => check_flag(flag, op, "required")?,
    };
    let kid_names: Vec<String> = kids.iter().map(|kid| kid.on.clone()).collect();
    let default = match request.get("default") {
        None => None,
        Some(given) => Some(check_default(
            kind.kind(),
            given,
            &Resulting {
                max_len,
                options: &options,
                kids: &kid_names,
                edit: request.get("edit").and_then(Value::as_bool) == Some(true),
            },
            op,
        )?),
    };
    let flags = ff | if read_only { READ_ONLY } else { 0 } | if required { REQUIRED } else { 0 };
    let radio_on = match (kind, &default) {
        (AddKind::Radio, Some(Some(Default::Text(on)))) => on.clone(),
        _ => String::new(),
    };
    let mut state = (kind == AddKind::Checkbox).then(|| b"Off".to_vec());
    if let Some(default) = &default {
        match (kind, default) {
            (AddKind::Text | AddKind::Combo, Some(Default::Text(text))) => {
                value = Some(Object::text(text));
            }
            (AddKind::Checkbox, default) => {
                let on = matches!(default, Some(Default::Check(true)));
                let name = if on { FIELD_CHECK_ON } else { b"Off" };
                state = Some(name.to_vec());
                value = Some(Object::Name(name.to_vec()));
            }
            _ => {}
        }
    }
    let mut radio_value = radio_on.clone();
    if let Some(given) = value_override {
        match kind {
            AddKind::Text | AddKind::Combo => {
                value = Some(Object::text(&request::js_string(Some(given))));
            }
            AddKind::Checkbox => {
                let on = given.as_bool() == Some(true);
                let name = if on { FIELD_CHECK_ON } else { b"Off" };
                state = Some(name.to_vec());
                value = Some(Object::Name(name.to_vec()));
            }
            AddKind::Radio => radio_value = given.as_str().unwrap_or("").to_string(),
            AddKind::Signature => {}
        }
    }
    if auto_name {
        let taken = field_tree_names(doc, op)?.unwrap_or_default();
        let mut n = 1u64;
        while taken.contains(&format!("Signature{n}")) {
            n += 1;
        }
        name = format!("Signature{n}");
    }
    if existing_fields(doc).is_some() {
        check_name_free(doc, &name, None, op)?;
    }
    let Some(page_id) = page_id else {
        return Err(plain("field-add: the page is not an indirect object"));
    };
    let rotate = crate::app::info::page_rotation(doc, page_dict);
    Ok(AddPlan {
        kind,
        name,
        page,
        page_id,
        rect,
        kids,
        options,
        flags,
        max_len,
        label,
        default,
        value,
        state,
        radio_value,
        radio_on,
        rotate,
    })
}

/// engine.ts' `/MK`: black border, white background, the page's /Rotate.
fn mk(rotate: i64) -> Object {
    let mut mk = Dictionary::new();
    mk.insert(b"BC".to_vec(), Object::array([Object::Integer(0)]));
    mk.insert(b"BG".to_vec(), Object::array([Object::Integer(1)]));
    if rotate != 0 {
        mk.insert(b"R".to_vec(), Object::Integer(rotate));
    }
    Object::Dictionary(mk)
}

fn write_add(tx: &mut Transaction<'_>, plan: &AddPlan) -> Result<Added, AppError> {
    let fields = ensure_acroform(tx)?;
    let space = {
        let doc = tx.document();
        PageSpace::of(doc, widget_dict(doc, plan.page_id)?)
    };
    let page_ref = Object::Reference(plan.page_id);
    let da = Object::text(FIELD_DA);
    let mut ids = Vec::new();
    let mut hints: Vec<Option<Vec<u8>>> = Vec::new();
    if plan.kind == AddKind::Radio {
        let mut parent = Dictionary::new();
        parent.insert(b"FT".to_vec(), Object::name("Btn"));
        let parent_id = tx.allocate(Object::Dictionary(parent), None)?;
        for kid in &plan.kids {
            let state = if kid.on == plan.radio_value {
                kid.on.as_bytes().to_vec()
            } else {
                b"Off".to_vec()
            };
            let widget = Object::dictionary([
                ("Type", Object::name("Annot")),
                ("Subtype", Object::name("Widget")),
                ("Parent", Object::Reference(parent_id)),
                ("Rect", forms::rect_object(space.to_pdf(kid.rect))),
                ("F", Object::Integer(FIELD_F_PRINT)),
                ("P", page_ref.clone()),
                ("AS", Object::Name(state)),
                ("DA", da.clone()),
                ("MK", mk(plan.rotate)),
            ]);
            ids.push(tx.allocate(widget, None)?);
            hints.push(Some(kid.on.as_bytes().to_vec()));
        }
        let mut parent = Dictionary::new();
        parent.insert(b"FT".to_vec(), Object::name("Btn"));
        parent.insert(b"T".to_vec(), Object::text(&plan.name));
        parent.insert(b"Ff".to_vec(), Object::Integer(plan.flags));
        let kids = ids.iter().map(|id| Object::Reference(*id));
        parent.insert(b"Kids".to_vec(), Object::array(kids));
        let value = if plan.radio_value.is_empty() {
            "Off"
        } else {
            plan.radio_value.as_str()
        };
        parent.insert(b"V".to_vec(), Object::name(value));
        if let Some(label) = &plan.label {
            parent.insert(b"TU".to_vec(), Object::text(label));
        }
        if !plan.radio_on.is_empty() {
            parent.insert(b"DV".to_vec(), Object::name(plan.radio_on.as_str()));
        }
        tx.put_value(parent_id, Object::Dictionary(parent))?;
        push(tx, &fields, &[parent_id])?;
    } else {
        let ft = match plan.kind {
            AddKind::Checkbox => "Btn",
            AddKind::Combo => "Ch",
            AddKind::Signature => "Sig",
            _ => "Tx",
        };
        let rect = plan.rect.unwrap_or([72.0, 72.0, 252.0, 96.0]);
        let mut widget = Dictionary::new();
        widget.insert(b"Type".to_vec(), Object::name("Annot"));
        widget.insert(b"Subtype".to_vec(), Object::name("Widget"));
        widget.insert(b"FT".to_vec(), Object::name(ft));
        widget.insert(b"T".to_vec(), Object::text(&plan.name));
        widget.insert(b"Rect".to_vec(), forms::rect_object(space.to_pdf(rect)));
        widget.insert(b"F".to_vec(), Object::Integer(FIELD_F_PRINT));
        widget.insert(b"P".to_vec(), page_ref.clone());
        widget.insert(b"DA".to_vec(), da.clone());
        widget.insert(b"MK".to_vec(), mk(plan.rotate));
        if plan.flags != 0 {
            widget.insert(b"Ff".to_vec(), Object::Integer(plan.flags));
        }
        if plan.kind == AddKind::Combo {
            let options = plan.options.iter().map(|o| Object::text(o));
            widget.insert(b"Opt".to_vec(), Object::array(options));
        }
        if let Some(label) = &plan.label {
            widget.insert(b"TU".to_vec(), Object::text(label));
        }
        match &plan.default {
            Some(Some(Default::Check(on))) => {
                let name = if *on { FIELD_CHECK_ON } else { b"Off" };
                widget.insert(b"DV".to_vec(), Object::Name(name.to_vec()));
            }
            Some(Some(Default::Text(text))) => {
                widget.insert(b"DV".to_vec(), Object::text(text));
            }
            _ => {}
        }
        if let Some(value) = &plan.value {
            widget.insert(b"V".to_vec(), value.clone());
        }
        if let Some(state) = &plan.state {
            widget.insert(b"AS".to_vec(), Object::Name(state.clone()));
        }
        if plan.kind == AddKind::Text && plan.max_len > 0 {
            widget.insert(b"MaxLen".to_vec(), Object::Integer(plan.max_len));
        }
        let id = tx.allocate(Object::Dictionary(widget), None)?;
        push(tx, &fields, &[id])?;
        ids.push(id);
        hints.push((plan.kind == AddKind::Checkbox).then(|| FIELD_CHECK_ON.to_vec()));
    }
    direct_annots(tx, plan.page_id, |_| true, &ids)?;
    // The new widgets list on their page, as engine.ts checks.
    let landed = {
        let doc = tx.document();
        let listed: BTreeSet<ObjectId> = (page_widgets(doc, widget_dict(doc, plan.page_id)?))
            .into_iter()
            .filter_map(|widget| widget.id)
            .collect();
        ids.iter().all(|id| listed.contains(id))
    };
    if !landed {
        return Err(fail(
            "invalid",
            "field-add: the new field did not land on the page",
        ));
    }
    if plan.kind != AddKind::Signature {
        let mut sink = TxSink::new(tx);
        for (id, hint) in ids.iter().zip(&hints) {
            refresh(&mut sink, *id, hint.as_deref())?;
        }
    }
    let _ = plan.page;
    Ok(Added {
        name: plan.name.clone(),
        ids,
        radio: plan.kind == AddKind::Radio,
    })
}

/// engine.ts `setFieldFlag`: one /Ff bit on the node's own /Ff (a number,
/// directly or through a reference); an absent key stays absent when the
/// result is 0.
fn set_field_flag(
    tx: &mut Transaction<'_>,
    node: ObjectId,
    bit: i64,
    on: bool,
) -> Result<(), AppError> {
    let (present, bits) = own_flags(tx.document(), node);
    let next = if on { bits | bit } else { bits & !bit };
    if !present && next == 0 {
        return Ok(());
    }
    update(tx, node, &|dict| {
        dict.insert(b"Ff".to_vec(), Object::Integer(next));
    })
}

/// The node's own /Ff as engine.ts reads it (`isNumber`, `asNumber`, both
/// through a reference): whether it is a number, and its bits.
fn own_flags(doc: &Document<'_>, node: ObjectId) -> (bool, i64) {
    let dict = doc.resolve_id(node).as_dict().unwrap_or(&EMPTY);
    let flags = doc.dict_get(dict, b"Ff");
    (flags.as_f64().is_some(), js_int32(flags))
}

/// A validated field-props.
#[derive(Default)]
struct PropsPlan {
    name: Option<String>,
    rename: Option<ObjectId>,
    rect: Option<[f64; 4]>,
    max_len: Option<i64>,
    multiline: Option<bool>,
    options: Option<Vec<String>>,
    label: Option<String>,
    read_only: Option<bool>,
    required: Option<bool>,
    default: Option<Option<Default>>,
    edit: Option<bool>,
}

/// engine.ts `field-props`.
pub fn props(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let op = "field-props";
    let app = session.require_mutable(op)?;
    let page = session.check_page(request.get("page"), op)?;
    let doc = app.document();
    let (_, page_dict) = page_of(app, page)?;
    let id = find_widget(doc, page_dict, request.get("widgetId"), op)?;
    let dict = widget_dict(doc, id)?;
    let kind = check_editable(app, id, dict, op)?;
    let hierarchy = "field-props: part of a field hierarchy; cannot change it here";
    let parent = structural_parent(doc, dict, hierarchy)?;
    let has = |key: &str| request.get(key).is_some();
    let wants_properties = has("label") || has("readOnly") || has("required") || has("default");
    if wants_properties
        && let Some(parent) = parent
        && radio_kids_carry_own(doc, parent)
    {
        return Err(fail("invalid", hierarchy));
    }
    let any = ["name", "rect", "maxLen", "multiline", "options", "edit"]
        .iter()
        .any(|key| has(key));
    if !any && !wants_properties {
        return Err(fail(
            "invalid",
            "field-props: at least one property is needed",
        ));
    }
    let signature = kind == Kind::Signature;
    if signature && has("name") {
        return Err(fail(
            "signature",
            "field-props: a signature field keeps its name",
        ));
    }
    if signature && has("default") {
        return Err(fail(
            "invalid",
            "field-props: default is not a signature property",
        ));
    }
    let mut plan = PropsPlan::default();
    if has("name") {
        let name = check_name(request.get("name"), op)?;
        let target = parent.unwrap_or(id);
        if existing_fields(doc).is_some() {
            let target_dict = widget_dict(doc, target)?;
            let own = forms::short_full_name(doc, Some(target), target_dict);
            check_name_free(doc, &name, Some(&own), op)?;
        }
        plan.name = Some(name);
        plan.rename = Some(target);
    }
    if has("rect") {
        let bounds = PageSpace::of(doc, page_dict).bounds();
        let rect = check_rect(bounds, request.get("rect"), op)?;
        if signature && (rect[2] - rect[0] < 40.0 || rect[3] - rect[1] < 20.0) {
            return Err(fail(
                "invalid",
                "field-props: a signature field is at least 40x20 pt",
            ));
        }
        plan.rect = Some(rect);
    }
    if let Some(given) = request.get("maxLen") {
        if kind != Kind::Text {
            return Err(fail("invalid", "field-props: maxLen is a text property"));
        }
        let max_len = request::as_int(given)
            .filter(|n| (0..=MAX_FIELD_TEXT_CHARS as i64).contains(n))
            .ok_or_else(|| {
                fail(
                    "invalid",
                    format!("field-props: maxLen is an integer 0..{MAX_FIELD_TEXT_CHARS}"),
                )
            })?;
        if max_len > 0 && js_len(&forms::value_text(doc, dict)) as i64 > max_len {
            return Err(fail(
                "invalid",
                "field-props: maxLen is below the current value's length",
            ));
        }
        plan.max_len = Some(max_len);
    }
    if let Some(given) = request.get("multiline") {
        if kind != Kind::Text {
            return Err(fail("invalid", "field-props: multiline is a text property"));
        }
        let Some(multiline) = given.as_bool() else {
            return Err(fail("invalid", "field-props: multiline is true or false"));
        };
        plan.multiline = Some(multiline);
    }
    if has("options") {
        if kind != Kind::Combo {
            return Err(fail(
                "invalid",
                "field-props: options are a dropdown property",
            ));
        }
        plan.options = Some(check_options(request.get("options"), op)?);
    }
    if let Some(given) = request.get("edit") {
        if kind != Kind::Combo {
            return Err(fail("invalid", "field-props: edit is a dropdown property"));
        }
        plan.edit = Some(check_flag(given, op, "edit")?);
    }
    if let Some(given) = request.get("label") {
        plan.label = Some(check_label(given, op)?);
    }
    if let Some(given) = request.get("readOnly") {
        plan.read_only = Some(check_flag(given, op, "readOnly")?);
    }
    if let Some(given) = request.get("required") {
        plan.required = Some(check_flag(given, op, "required")?);
    }
    let flags = field_flags(doc, dict);
    if let Some(given) = request.get("default") {
        let options = match &plan.options {
            Some(options) => options.clone(),
            None if kind == Kind::Combo => forms::options(doc, dict),
            None => Vec::new(),
        };
        let kids = parent.map_or_else(Vec::new, |parent| kid_on_names(doc, parent));
        let max_len = plan.max_len.unwrap_or(if kind == Kind::Text {
            forms::max_len(doc, dict)
        } else {
            0
        });
        let edit = plan
            .edit
            .unwrap_or(kind == Kind::Combo && flags & EDIT != 0);
        let resulting = Resulting {
            max_len,
            options: &options,
            kids: &kids,
            edit,
        };
        plan.default = Some(check_default(kind, given, &resulting, op)?);
    }
    if kind == Kind::Text
        && let Some(max_len) = plan.max_len.filter(|n| *n > 0)
    {
        let (_, own) = own_dict(doc, kind, Some(id), dict);
        if let Object::String(stale) = doc.dict_get(own, b"DV") {
            let stale = crate::object::decode_text_string(stale);
            if js_len(&stale) as i64 > max_len {
                return Err(fail(
                    "invalid",
                    "field-props: maxLen is below the default value's length",
                ));
            }
        }
    }
    session.mutate("change field", |session| {
        let mut tx = session.tx()?;
        write_props(&mut tx, page, id, kind, &plan)
    })?;
    Ok(edit_reply(session, request))
}

/// engine.ts `radioKidsCarryOwn`.
fn radio_kids_carry_own(doc: &Document<'_>, parent: ObjectId) -> bool {
    let Some(parent) = doc.resolve_id(parent).as_dict() else {
        return false;
    };
    let kids = doc.dict_get(parent, b"Kids").as_array().unwrap_or(&[]);
    kids.iter().any(|kid| {
        let kid = doc.resolve(kid).as_dict().unwrap_or(&EMPTY);
        [b"Ff".as_slice(), b"TU", b"DV", b"V"]
            .iter()
            .any(|key| !doc.dict_get(kid, key).is_null())
    })
}

/// engine.ts `kidOnNames`.
fn kid_on_names(doc: &Document<'_>, parent: ObjectId) -> Vec<String> {
    let Some(parent) = doc.resolve_id(parent).as_dict() else {
        return Vec::new();
    };
    let kids = doc.dict_get(parent, b"Kids").as_array().unwrap_or(&[]);
    kids.iter()
        .map(|kid| {
            let kid = doc.resolve(kid).as_dict().unwrap_or(&EMPTY);
            forms::name_text(&on_state(doc, kid))
        })
        .collect()
}

fn write_props(
    tx: &mut Transaction<'_>,
    page: usize,
    id: ObjectId,
    kind: Kind,
    plan: &PropsPlan,
) -> Result<(), AppError> {
    ensure_acroform(tx)?;
    let (own, space) = {
        let doc = tx.document();
        let dict = widget_dict(doc, id)?;
        let (own, _) = own_dict(doc, kind, Some(id), dict);
        let page_dict = crate::app::info::pages(doc)
            .get(page)
            .map(|page| page.dict)
            .unwrap_or(&EMPTY);
        (own.unwrap_or(id), PageSpace::of(doc, page_dict))
    };
    if let (Some(name), Some(target)) = (&plan.name, plan.rename) {
        update(tx, target, &|dict| {
            dict.insert(b"T".to_vec(), Object::text(name));
        })?;
    }
    let mut appearance = false;
    if let Some(rect) = plan.rect {
        let rect = forms::rect_object(space.to_pdf(rect));
        update(tx, id, &|dict| {
            dict.insert(b"Rect".to_vec(), rect.clone());
        })?;
        appearance = true;
    }
    if let Some(max_len) = plan.max_len {
        update(tx, id, &|dict| {
            if max_len == 0 {
                dict.remove(b"MaxLen".as_slice());
            } else {
                dict.insert(b"MaxLen".to_vec(), Object::Integer(max_len));
            }
        })?;
        appearance = true;
    }
    if let Some(multiline) = plan.multiline {
        let (_, bits) = own_flags(tx.document(), id);
        update(tx, id, &|dict| {
            let next = if multiline {
                bits | MULTILINE
            } else {
                bits & !MULTILINE
            };
            dict.insert(b"Ff".to_vec(), Object::Integer(next));
        })?;
        appearance = true;
    }
    if let Some(edit) = plan.edit {
        set_field_flag(tx, own, EDIT, edit)?;
        appearance = true;
    }
    let options_now = match (&plan.options, plan.edit) {
        (Some(options), _) => Some(options.clone()),
        (None, Some(false)) => Some(forms::options(
            tx.document(),
            widget_dict(tx.document(), id)?,
        )),
        _ => None,
    };
    if let Some(options_now) = options_now {
        let (editable, current, stale) = {
            let doc = tx.document();
            let dict = widget_dict(doc, id)?;
            let own_dict = widget_dict(doc, own)?;
            let stale = match doc.dict_get(own_dict, b"DV") {
                Object::String(dv) => Some(crate::object::decode_text_string(dv)),
                _ => None,
            };
            (
                field_flags(doc, dict) & EDIT != 0,
                forms::value_text(doc, dict),
                stale,
            )
        };
        if !editable {
            if !options_now.contains(&current)
                && let Some(first) = options_now.first()
            {
                update(tx, id, &|dict| {
                    dict.insert(b"V".to_vec(), Object::text(first));
                })?;
                drop_choice_indices(tx, id)?;
            }
            if stale.is_some_and(|stale| !options_now.contains(&stale)) {
                update(tx, own, &|dict| {
                    dict.remove(b"DV".as_slice());
                })?;
            }
        }
    }
    if let Some(options) = &plan.options {
        let list = Object::array(options.iter().map(|o| Object::text(o)));
        update(tx, id, &|dict| {
            dict.insert(b"Opt".to_vec(), list.clone());
        })?;
        appearance = true;
    }
    if let Some(label) = &plan.label {
        update(tx, own, &|dict| {
            if label.is_empty() {
                dict.remove(b"TU".as_slice());
            } else {
                dict.insert(b"TU".to_vec(), Object::text(label));
            }
        })?;
    }
    if let Some(read_only) = plan.read_only {
        set_field_flag(tx, own, READ_ONLY, read_only)?;
    }
    if let Some(required) = plan.required {
        set_field_flag(tx, own, REQUIRED, required)?;
    }
    if let Some(default) = &plan.default {
        let on = {
            let doc = tx.document();
            on_state(doc, widget_dict(doc, own)?)
        };
        update(tx, own, &|dict| match default {
            None => {
                dict.remove(b"DV".as_slice());
            }
            Some(Default::Check(true)) => {
                let on = if on.is_empty() {
                    FIELD_CHECK_ON.to_vec()
                } else {
                    on.clone()
                };
                dict.insert(b"DV".to_vec(), Object::Name(on));
            }
            Some(Default::Check(false)) => {
                dict.insert(b"DV".to_vec(), Object::name("Off"));
            }
            Some(Default::Text(text)) if kind == Kind::Radio => {
                dict.insert(b"DV".to_vec(), Object::name(text.as_str()));
            }
            Some(Default::Text(text)) => {
                dict.insert(b"DV".to_vec(), Object::text(text));
            }
        })?;
    }
    if appearance && kind == Kind::Signature {
        update(tx, id, &|dict| {
            dict.remove(b"AP".as_slice());
        })?;
    } else if appearance {
        let mut sink = TxSink::new(tx);
        refresh(&mut sink, id, None)?;
    }
    Ok(())
}

/// Edits the dictionary of `target`.
fn update(
    tx: &mut Transaction<'_>,
    target: ObjectId,
    edit: &dyn Fn(&mut Dictionary),
) -> Result<(), AppError> {
    tx.update(target, |value| {
        if let Some(dict) = value.as_dict_mut() {
            edit(dict);
        }
    })?;
    Ok(())
}

/// engine.ts `dropChoiceIndices`: /I off the widget and 32 nodes up.
fn drop_choice_indices(tx: &mut Transaction<'_>, id: ObjectId) -> Result<(), AppError> {
    let nodes: Vec<ObjectId> = {
        let doc = tx.document();
        let dict = widget_dict(doc, id)?;
        lineage(doc, Some(id), dict, 32)
            .into_iter()
            .filter(|(_, node)| node.contains_key(b"I".as_slice()))
            .filter_map(|(node, _)| node)
            .collect()
    };
    for node in nodes {
        tx.update(node, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.remove(b"I".as_slice());
            }
        })?;
    }
    Ok(())
}

/// engine.ts `field-delete`.
pub fn delete(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let op = "field-delete";
    let app = session.require_mutable(op)?;
    let page = session.check_page(request.get("page"), op)?;
    let doc = app.document();
    let (page_id, page_dict) = page_of(app, page)?;
    let id = find_widget(doc, page_dict, request.get("widgetId"), op)?;
    let dict = widget_dict(doc, id)?;
    check_editable(app, id, dict, op)?;
    let hierarchy = "field-delete: part of a field hierarchy; cannot delete here";
    // Only a structural radio parent's kid is deleted (from its /Kids).
    structural_parent(doc, dict, hierarchy)?;
    let Some(page_id) = page_id else {
        return Err(plain("field-delete: the page is not an indirect object"));
    };
    session.mutate("delete field", |session| {
        let mut tx = session.tx()?;
        remove_widget(&mut tx, page_id, id)
    })?;
    Ok(edit_reply(session, request))
}

/// Removes the widget `id` (engine.ts field-delete's writes) on any `Edit`
/// target: from the page `page`'s /Annots (a new direct array), and from
/// /AcroForm /Fields or, for a kid, its /Parent's /Kids; a parent its last
/// kid leaves goes from /Fields too, its /DV goes when it named the
/// widget's state, and its /V becomes Off when only this kid had that
/// state. Every other reference to the widget, or to a parent it emptied,
/// is cut (structure OBJRs included), so nothing keeps them reachable.
pub fn remove_widget<E: Edit>(edit: &mut E, page: ObjectId, id: ObjectId) -> Result<(), AppError> {
    let (parent, drop_default, drop_value) = {
        let doc = edit.document();
        let dict = widget_dict(doc, id)?;
        let parent = parent_of(doc, dict).and_then(|(holder, parent)| Some((holder?, parent)));
        let on = on_state(doc, dict);
        let drop_default = parent.is_some_and(|(_, parent)| {
            !on.is_empty() && doc.dict_get(parent, b"DV").as_name() == Some(on.as_slice())
        });
        // The group's value named only the deleted kid: the group is now off.
        let drop_value = parent.is_some_and(|(_, parent)| {
            let kids = doc.dict_get(parent, b"Kids").as_array().unwrap_or(&[]);
            let shared = kids.iter().any(|kid| {
                kid.as_reference() != Some(id)
                    && doc
                        .resolve(kid)
                        .as_dict()
                        .is_some_and(|kid| on_state(doc, kid) == on)
            });
            !on.is_empty() && !shared && doc.dict_get(parent, b"V").as_name() == Some(on.as_slice())
        });
        (parent.map(|(holder, _)| holder), drop_default, drop_value)
    };
    let number = id.number();
    let other = move |entry: &Object| entry.as_reference().is_none_or(|e| e.number() != number);
    direct_annots(edit, page, other, &[])?;
    let fields = acroform_fields(edit.document());
    let emptied = match parent {
        None => {
            if let Some(fields) = &fields {
                fields.update(edit, |value| {
                    if let Some(items) = value.as_array_mut() {
                        items.retain(other);
                    }
                })?;
            }
            None
        }
        Some(parent_id) => {
            let empty = edit.update(parent_id, |value| {
                let dict = value.as_dict_mut()?;
                if drop_default {
                    dict.remove(b"DV".as_slice());
                }
                if drop_value {
                    dict.insert(b"V".to_vec(), Object::name("Off"));
                }
                let kids = match dict.get(b"Kids".as_slice()) {
                    Some(Object::Array(kids)) => {
                        kids.iter().filter(|k| other(k)).cloned().collect()
                    }
                    _ => Vec::new(),
                };
                let empty = kids.is_empty();
                dict.insert(b"Kids".to_vec(), Object::Array(kids));
                Some(empty)
            })?;
            if empty == Some(true)
                && let Some(fields) = &fields
            {
                let parent_number = parent_id.number();
                fields.update(edit, |value| {
                    if let Some(items) = value.as_array_mut() {
                        items.retain(|e| {
                            e.as_reference().is_none_or(|e| e.number() != parent_number)
                        });
                    }
                })?;
            }
            (empty == Some(true)).then_some(parent_id)
        }
    };
    let dropped: BTreeSet<ObjectId> = [Some(id), emptied].into_iter().flatten().collect();
    pagetree::cut_references(edit, &BTreeSet::new(), &dropped, &BTreeSet::new())?;
    Ok(())
}

/// Where /AcroForm /Fields is, written as a direct array into /AcroForm
/// (engine.ts `acro.put("Fields", ...)`); None without an array there.
fn acroform_fields(doc: &Document<'_>) -> Option<FieldsPlace> {
    existing_fields(doc)?;
    let (catalog_id, _) = doc.resolve_id_with_holder(doc.root());
    let acro = Place::object(catalog_id?).child(doc, b"AcroForm");
    Some(FieldsPlace { acro })
}

/// /AcroForm /Fields edited as a direct array in /AcroForm.
struct FieldsPlace {
    acro: Place,
}

impl FieldsPlace {
    fn update<E: Edit>(&self, tx: &mut E, edit: impl FnOnce(&mut Object)) -> Result<(), AppError> {
        let current = {
            let doc = tx.document();
            let acro = self.acro.read(doc).as_dict().unwrap_or(&EMPTY);
            doc.dict_get(acro, b"Fields").clone()
        };
        let mut fields = current;
        edit(&mut fields);
        self.acro.update(tx, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"Fields".to_vec(), fields);
            }
        })?;
        Ok(())
    }
}
