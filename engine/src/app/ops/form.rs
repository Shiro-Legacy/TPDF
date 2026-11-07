// SPDX-License-Identifier: MIT
//! The question form (design form, phase 2): a list of questions the engine
//! lays out on the form's own pages, each with its label drawn on the page
//! and its fields placed under it.
//!
//! The list is recorded on the catalog as a page-piece dictionary, beside
//! the stamps (`/PieceInfo /TandemPDF /Private /Form`): the title, the form
//! pages (`/Pages`, references; `/Made` the ones Tandem added), and each
//! question's text, help, type and widgets (`/W [<< /R widget /Laid rect
//! /C choice >>]`). Required flags, choices and names are read from the
//! fields themselves, so the record cannot drift from them; the record's
//! `/Laid` rects tell a field moved by hand (pinned) from one the layout
//! placed. The labels are one Tandem stream pair per form page (their
//! stream dictionaries carry `/TandemForm true`), rewritten whole on every
//! layout.
//!
//! `form-questions` lists the questions, and every other field as `unlisted`.
//! `form-set` declares the whole list: questions missing from it are
//! deleted, new ones added, changed ones changed, and the pages laid out
//! again, all in one "change form" step built from field-add, field-props,
//! field-delete, insert-blank and page-delete. A field change from outside
//! (the popover, the Fields designer, edit-batch, an agent) on a form page
//! lays the pages out again in its own step (`after_field_change`).
//!
//! Pages made elsewhere are never laid out: a question on one of their
//! fields only sets its /TU (the screen-reader description).

mod layout;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use self::layout::{Block, Ink, Shape};
use super::edit::plan::{literal, win_ansi_bytes};
use super::stamp::{add_resource, has_resource, kept_contents, win_ansi_byte};
use super::{fields, pages};
use crate::app::edit::{Place, get};
use crate::app::forms::PageSpace;
use crate::app::request::js_number;
use crate::app::{AppError, AppSession, Batch, Transaction, info, reply};
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId, decode_text_string};
use crate::stream::StreamLimits;

const PIECE: &[u8] = b"TandemPDF";
const KEY: &[u8] = b"Form";
/// The key on a label stream's dictionary.
const MARK: &[u8] = b"TandemForm";
const FONT_REGULAR: &[u8] = b"TandemFormR";
const FONT_BOLD: &[u8] = b"TandemFormB";
pub const MAX_QUESTIONS: usize = 200;
pub const MAX_FORM_PAGES: usize = 50;
/// A question fits a field's /TU.
pub const MAX_QUESTION_CHARS: usize = 256;
pub const MAX_HELP_CHARS: usize = 500;
pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_DESC_CHARS: usize = 1000;
pub const MAX_CHOICES: usize = 32;
pub const MAX_CHOICE_CHARS: usize = 64;
/// A widget farther than this from where the layout put it was moved by hand.
const MOVED: f64 = 0.5;

fn invalid(message: impl Into<String>) -> AppError {
    AppError::reason("invalid", message)
}

fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// A question's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QType {
    Short,
    Paragraph,
    One,
    Several,
    Dropdown,
    YesNo,
    Date,
    Number,
    Email,
    Phone,
    Agree,
    Signature,
}

const TYPES: [(&str, QType); 12] = [
    ("short", QType::Short),
    ("paragraph", QType::Paragraph),
    ("one", QType::One),
    ("several", QType::Several),
    ("dropdown", QType::Dropdown),
    ("yesno", QType::YesNo),
    ("date", QType::Date),
    ("number", QType::Number),
    ("email", QType::Email),
    ("phone", QType::Phone),
    ("agree", QType::Agree),
    ("signature", QType::Signature),
];

impl QType {
    pub fn parse(name: &str) -> Option<Self> {
        TYPES.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
    }

    pub fn name(self) -> &'static str {
        TYPES
            .iter()
            .find(|(_, t)| *t == self)
            .map_or("short", |(n, _)| n)
    }

    fn has_choices(self) -> bool {
        matches!(self, Self::One | Self::Several | Self::Dropdown)
    }

    /// The field kind its widgets are.
    fn field_kind(self) -> &'static str {
        match self {
            Self::One | Self::YesNo => "radio",
            Self::Several | Self::Agree => "checkbox",
            Self::Dropdown => "combo",
            Self::Signature => "signature",
            _ => "text",
        }
    }

    /// The type a field of `kind` is shown as before anyone says otherwise.
    fn of_kind(kind: &str, multiline: bool) -> Self {
        match kind {
            "radio" => Self::One,
            "checkbox" => Self::Agree,
            "combo" | "list" => Self::Dropdown,
            "signature" => Self::Signature,
            _ if multiline => Self::Paragraph,
            _ => Self::Short,
        }
    }

    /// The field box of a one-box type.
    fn field_size(self) -> Option<(f64, f64)> {
        match self {
            Self::Short | Self::Email => Some((300.0, 22.0)),
            Self::Phone => Some((180.0, 22.0)),
            Self::Number | Self::Date => Some((120.0, 22.0)),
            Self::Paragraph => Some((f64::INFINITY, 66.0)),
            Self::Dropdown => Some((220.0, 22.0)),
            Self::Signature => Some((220.0, 44.0)),
            _ => None,
        }
    }
}

/// One widget a recorded question holds.
#[derive(Clone, Debug)]
struct Held {
    id: ObjectId,
    laid: Option<[f64; 4]>,
    /// The choice of a several-choices checkbox.
    choice: String,
}

/// A recorded question.
#[derive(Clone, Debug)]
struct Stored {
    id: i64,
    text: String,
    help: String,
    kind: QType,
    type_auto: bool,
    /// Several choices only (its checkboxes carry no required flag).
    required: bool,
    pinned: bool,
    /// The name the engine gave it (several: the base of its checkboxes'
    /// names); a field still named so follows its question.
    name: String,
    widgets: Vec<Held>,
}

/// The record on the catalog.
#[derive(Clone, Debug, Default)]
struct Record {
    title: String,
    desc: String,
    pages: Vec<ObjectId>,
    made: BTreeSet<ObjectId>,
    questions: Vec<Stored>,
}

fn text_of(doc: &Document<'_>, dict: &Dictionary, key: &[u8]) -> String {
    doc.dict_get(dict, key)
        .as_string()
        .map(decode_text_string)
        .unwrap_or_default()
}

fn rect_of(doc: &Document<'_>, value: &Object) -> Option<[f64; 4]> {
    let items = doc.resolve(value).as_array()?;
    let nums: Vec<f64> = items
        .iter()
        .filter_map(|v| doc.resolve(v).as_f64())
        .collect();
    (nums.len() == 4).then(|| [nums[0], nums[1], nums[2], nums[3]])
}

fn is_widget(doc: &Document<'_>, id: ObjectId) -> bool {
    doc.resolve_id(id)
        .as_dict()
        .is_some_and(|d| doc.dict_get(d, b"Subtype").as_name() == Some(b"Widget"))
}

fn form_dict<'d>(doc: &'d Document<'_>) -> Option<&'d Dictionary> {
    let catalog = doc.resolve_id(doc.root());
    let piece = get(doc, get(doc, catalog, b"PieceInfo"), PIECE);
    get(doc, get(doc, piece, b"Private"), KEY).as_dict()
}

fn read_record(doc: &Document<'_>) -> Record {
    let Some(form) = form_dict(doc) else {
        return Record::default();
    };
    let refs = |key: &[u8]| -> Vec<ObjectId> {
        doc.dict_get(form, key)
            .as_array()
            .unwrap_or(&[])
            .iter()
            .filter_map(Object::as_reference)
            .filter(|&id| doc.resolve_id(id).as_dict().is_some())
            .collect()
    };
    let mut seen = BTreeSet::new();
    let pages: Vec<ObjectId> = refs(b"Pages")
        .into_iter()
        .filter(|id| seen.insert(*id))
        .collect();
    let made = refs(b"Made").into_iter().collect();
    let mut ids = BTreeSet::new();
    let questions = doc
        .dict_get(form, b"Questions")
        .as_array()
        .unwrap_or(&[])
        .iter()
        .take(MAX_QUESTIONS)
        .filter_map(|q| {
            let q = doc.resolve(q).as_dict()?;
            let id = doc.dict_get(q, b"Id").as_i64().filter(|id| *id > 0)?;
            if !ids.insert(id) {
                return None;
            }
            let kind = doc
                .dict_get(q, b"Type")
                .as_name()
                .and_then(|n| QType::parse(&String::from_utf8_lossy(n)))
                .unwrap_or(QType::Short);
            let flag = |key: &[u8]| doc.dict_get(q, key).as_bool() == Some(true);
            let widgets = doc
                .dict_get(q, b"W")
                .as_array()
                .unwrap_or(&[])
                .iter()
                .filter_map(|w| {
                    let w = doc.resolve(w).as_dict()?;
                    let id = w.get(b"R".as_slice())?.as_reference()?;
                    is_widget(doc, id).then(|| Held {
                        id,
                        laid: w.get(b"Laid".as_slice()).and_then(|r| rect_of(doc, r)),
                        choice: text_of(doc, w, b"C"),
                    })
                })
                .collect();
            Some(Stored {
                id,
                text: text_of(doc, q, b"Text"),
                help: text_of(doc, q, b"Help"),
                kind,
                type_auto: flag(b"TypeAuto"),
                required: flag(b"Required"),
                pinned: flag(b"Pinned"),
                name: text_of(doc, q, b"Name"),
                widgets,
            })
        })
        .collect();
    Record {
        title: text_of(doc, form, b"Title"),
        desc: text_of(doc, form, b"Desc"),
        pages,
        made,
        questions,
    }
}

fn rect_object(rect: [f64; 4]) -> Object {
    Object::array(rect.map(|v| Object::Real((v * 100.0).round() / 100.0)))
}

fn record_object(record: &Record) -> Object {
    let refs = |ids: &mut dyn Iterator<Item = ObjectId>| {
        Object::Array(ids.map(Object::Reference).collect())
    };
    let questions = record.questions.iter().map(|q| {
        let widgets = q.widgets.iter().map(|w| {
            let mut d = Dictionary::new();
            d.insert(b"R".to_vec(), Object::Reference(w.id));
            if let Some(laid) = w.laid {
                d.insert(b"Laid".to_vec(), rect_object(laid));
            }
            if !w.choice.is_empty() {
                d.insert(b"C".to_vec(), Object::text(&w.choice));
            }
            Object::Dictionary(d)
        });
        let mut d = Dictionary::new();
        d.insert(b"Id".to_vec(), Object::Integer(q.id));
        d.insert(b"Text".to_vec(), Object::text(&q.text));
        if !q.help.is_empty() {
            d.insert(b"Help".to_vec(), Object::text(&q.help));
        }
        d.insert(b"Type".to_vec(), Object::name(q.kind.name()));
        d.insert(b"TypeAuto".to_vec(), Object::Boolean(q.type_auto));
        if q.required {
            d.insert(b"Required".to_vec(), Object::Boolean(true));
        }
        if q.pinned {
            d.insert(b"Pinned".to_vec(), Object::Boolean(true));
        }
        if !q.name.is_empty() {
            d.insert(b"Name".to_vec(), Object::text(&q.name));
        }
        d.insert(b"W".to_vec(), Object::array(widgets));
        Object::Dictionary(d)
    });
    let mut form = Dictionary::new();
    form.insert(b"V".to_vec(), Object::Integer(1));
    if !record.title.is_empty() {
        form.insert(b"Title".to_vec(), Object::text(&record.title));
    }
    if !record.desc.is_empty() {
        form.insert(b"Desc".to_vec(), Object::text(&record.desc));
    }
    form.insert(b"Pages".to_vec(), refs(&mut record.pages.iter().copied()));
    form.insert(b"Made".to_vec(), refs(&mut record.made.iter().copied()));
    form.insert(b"Questions".to_vec(), Object::array(questions));
    Object::Dictionary(form)
}

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

/// Writes the record; an empty one (no questions, no title) takes the
/// /Form entry away, and the page piece with it when nothing else is in it.
fn write_record(tx: &mut Transaction<'_>, record: &Record) -> Result<(), AppError> {
    let empty = record.questions.is_empty() && record.title.is_empty() && record.desc.is_empty();
    if empty && form_dict(tx.document()).is_none() {
        return Ok(());
    }
    let catalog = Place::object(tx.document().root());
    ensure_dict(tx, &catalog, b"PieceInfo")?;
    let piece_info = catalog.child(tx.document(), b"PieceInfo");
    ensure_dict(tx, &piece_info, PIECE)?;
    let piece = piece_info.child(tx.document(), PIECE);
    ensure_dict(tx, &piece, b"Private")?;
    let private = piece.child(tx.document(), b"Private");
    let value = (!empty).then(|| record_object(record));
    private.update(tx, |v| {
        if let Some(dict) = v.as_dict_mut() {
            match value {
                Some(value) => {
                    dict.insert(KEY.to_vec(), value);
                }
                None => {
                    dict.remove(KEY);
                }
            }
        }
    })?;
    let private_empty =
        (private.read(tx.document()).as_dict()).is_some_and(|d| d.keys().count() == 0);
    if private_empty {
        piece_info.update(tx, |v| {
            if let Some(dict) = v.as_dict_mut() {
                dict.remove(PIECE);
            }
        })?;
        let none_left =
            (piece_info.read(tx.document()).as_dict()).is_some_and(|d| d.keys().count() == 0);
        if none_left {
            catalog.update(tx, |v| {
                if let Some(dict) = v.as_dict_mut() {
                    dict.remove(b"PieceInfo".as_slice());
                }
            })?;
        }
    }
    Ok(())
}

/// A widget as the field listing shows it.
#[derive(Clone, Debug, Default)]
struct Live {
    page: usize,
    name: String,
    kind: String,
    rect: [f64; 4],
    required: bool,
    on: String,
    options: Vec<String>,
    multiline: bool,
    tu: String,
}

fn live_of(page: usize, item: &Value) -> Option<(u32, Live)> {
    let id = u32::try_from(item.get("id")?.as_i64()?).ok()?;
    let rect: Vec<f64> = item
        .get("rect")?
        .as_array()?
        .iter()
        .filter_map(Value::as_f64)
        .collect();
    let rect = (rect.len() == 4).then(|| [rect[0], rect[1], rect[2], rect[3]])?;
    let s = |key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    Some((
        id,
        Live {
            page,
            name: s("name"),
            kind: s("kind"),
            rect,
            required: item.get("required").and_then(Value::as_bool) == Some(true),
            on: s("on"),
            options: item
                .get("options")
                .and_then(Value::as_array)
                .map(|o| {
                    o.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            multiline: item.get("multiline").and_then(Value::as_bool) == Some(true),
            tu: s("tu"),
        },
    ))
}

/// The widgets of `pages`, by object number, in page and listing order.
fn live(
    session: &AppSession,
    pages: impl IntoIterator<Item = usize>,
) -> Result<(BTreeMap<u32, Live>, Vec<u32>), AppError> {
    let mut map = BTreeMap::new();
    let mut order = Vec::new();
    for page in pages {
        let listed = fields::list(session, &json!({"op": "fields", "page": page}))?;
        for item in listed
            .get("items")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
        {
            if let Some((id, live)) = live_of(page, item) {
                order.push(id);
                map.insert(id, live);
            }
        }
    }
    Ok((map, order))
}

/// Page index of each page object.
fn page_indices(doc: &Document<'_>) -> BTreeMap<ObjectId, usize> {
    info::pages(doc)
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.id.map(|id| (id, i)))
        .collect()
}

/// The form pages still in the document, as indices, in record order.
fn form_pages(doc: &Document<'_>, record: &Record) -> Vec<usize> {
    let index = page_indices(doc);
    record
        .pages
        .iter()
        .filter_map(|id| index.get(id).copied())
        .collect()
}

/// The pages whose widgets the record needs: the form pages and the pages
/// its widgets name in /P (every page when one has none).
fn needed_pages(doc: &Document<'_>, record: &Record) -> BTreeSet<usize> {
    let index = page_indices(doc);
    let mut pages: BTreeSet<usize> = form_pages(doc, record).into_iter().collect();
    for held in record.questions.iter().flat_map(|q| &q.widgets) {
        let page = doc
            .resolve_id(held.id)
            .as_dict()
            .and_then(|d| d.get(b"P".as_slice()))
            .and_then(Object::as_reference)
            .and_then(|p| index.get(&p).copied());
        match page {
            Some(page) => {
                pages.insert(page);
            }
            None => return (0..index.len().max(info::pages(doc).len())).collect(),
        }
    }
    pages
}

/// A question as a request (or the record itself) wants it.
#[derive(Clone, Debug)]
struct Wanted {
    id: Option<i64>,
    /// A field from the unlisted ones taken into the list.
    adopt: Option<u32>,
    text: String,
    help: String,
    kind: QType,
    type_auto: bool,
    choices: Vec<String>,
    required: bool,
    /// `false` puts a pinned question back in line.
    pinned: Option<bool>,
    /// The name its field follows (None keeps the name it has).
    name_hint: Option<String>,
}

fn check_text(value: Option<&Value>, max: usize, op: &str, what: &str) -> Result<String, AppError> {
    match value {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(text))
            if js_len(text) <= max && !text.chars().any(|c| c < ' ' && c != '\n') =>
        {
            Ok(text.trim().to_string())
        }
        Some(_) => Err(invalid(format!(
            "{op}: {what} is text of at most {max} characters, without control characters"
        ))),
    }
}

fn check_bool(value: Option<&Value>, op: &str, what: &str) -> Result<Option<bool>, AppError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(invalid(format!("{op}: {what} is true or false"))),
    }
}

fn check_choices(kind: QType, value: Option<&Value>, op: &str) -> Result<Vec<String>, AppError> {
    if kind == QType::YesNo {
        return Ok(vec!["Yes".into(), "No".into()]);
    }
    if !kind.has_choices() {
        return Ok(Vec::new());
    }
    let items = match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.clone(),
        Some(_) => return Err(invalid(format!("{op}: choices is a list of text"))),
    };
    let mut choices = Vec::new();
    for item in items {
        let text = item
            .as_str()
            .map(str::trim)
            .filter(|t| (1..=MAX_CHOICE_CHARS).contains(&js_len(t)) && !t.chars().any(|c| c < ' '))
            .ok_or_else(|| {
                invalid(format!(
                    "{op}: each choice is 1 to {MAX_CHOICE_CHARS} characters of text"
                ))
            })?;
        if choices.iter().any(|c: &String| c == text) {
            return Err(invalid(format!(
                "{op}: duplicate-choice: two choices are both “{text}”"
            )));
        }
        if kind == QType::One && text == "Off" {
            return Err(invalid(format!(
                "{op}: off-choice: a choice cannot be called “Off”"
            )));
        }
        choices.push(text.to_string());
    }
    let least = if kind == QType::One { 2 } else { 1 };
    if choices.len() < least || choices.len() > MAX_CHOICES {
        return Err(invalid(format!(
            "{op}: choices: this question needs {least} to {MAX_CHOICES} choices"
        )));
    }
    Ok(choices)
}

fn parse_wanted(
    request: &Value,
    record: &Record,
    unlisted: &BTreeSet<u32>,
) -> Result<Vec<Wanted>, AppError> {
    let op = "form-set";
    let items = request
        .get("questions")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(format!("{op}: questions is a list")))?;
    if items.len() > MAX_QUESTIONS {
        return Err(AppError::reason(
            "too-large",
            format!("{op}: a form holds at most {MAX_QUESTIONS} questions"),
        ));
    }
    let mut seen = BTreeSet::new();
    let mut adopted = BTreeSet::new();
    let mut wanted = Vec::new();
    for item in items {
        let id = match item.get("id") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_i64()
                    .filter(|id| record.questions.iter().any(|q| q.id == *id))
                    .ok_or_else(|| {
                        AppError::reason(
                            "stale-target",
                            format!("{op}: stale-target: the form has no question {v}"),
                        )
                    })?,
            ),
        };
        if let Some(id) = id
            && !seen.insert(id)
        {
            return Err(invalid(format!("{op}: question {id} is listed twice")));
        }
        let adopt = match item.get("widgetId") {
            None | Some(Value::Null) => None,
            Some(v) if id.is_none() => Some(
                v.as_i64()
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| unlisted.contains(n) && adopted.insert(*n))
                    .ok_or_else(|| {
                        AppError::reason(
                            "stale-target",
                            format!("{op}: stale-target: {v} is not a field outside the list"),
                        )
                    })?,
            ),
            Some(_) => {
                return Err(invalid(format!(
                    "{op}: a question with an id keeps its own fields (no widgetId)"
                )));
            }
        };
        let kind = match item.get("type").and_then(Value::as_str) {
            Some(name) => QType::parse(name).ok_or_else(|| {
                invalid(format!(
                    "{op}: type must be one of {}",
                    TYPES.map(|(n, _)| n).join(", ")
                ))
            })?,
            None => QType::Short,
        };
        let name_hint = match item.get("nameHint") {
            None | Some(Value::Null) => None,
            Some(v) => Some(check_text(Some(v), 4096, op, "nameHint")?),
        };
        wanted.push(Wanted {
            id,
            adopt,
            text: check_text(item.get("text"), MAX_QUESTION_CHARS, op, "text")?,
            help: check_text(item.get("help"), MAX_HELP_CHARS, op, "help")?,
            kind,
            type_auto: check_bool(item.get("typeAuto"), op, "typeAuto")?.unwrap_or(false),
            choices: check_choices(kind, item.get("choices"), op)?,
            required: check_bool(item.get("required"), op, "required")?.unwrap_or(false),
            pinned: check_bool(item.get("pinned"), op, "pinned")?,
            name_hint,
        });
    }
    Ok(wanted)
}

/// A choice-buttons question holds every button of its group, the ones
/// added on the page (field-kid-add) too.
fn refresh_groups(record: &mut Record, live: &BTreeMap<u32, Live>) {
    for q in &mut record.questions {
        if !matches!(q.kind, QType::One | QType::YesNo) {
            continue;
        }
        let Some(name) = (q.widgets.iter())
            .find_map(|h| live.get(&h.id.number()))
            .map(|l| l.name.clone())
        else {
            continue;
        };
        let held: BTreeSet<u32> = q.widgets.iter().map(|h| h.id.number()).collect();
        for (id, l) in live {
            if l.kind == "radio" && l.name == name && !held.contains(id) {
                if let Ok(id) = ObjectId::new(*id, 0) {
                    q.widgets.push(Held {
                        id,
                        laid: None,
                        choice: String::new(),
                    });
                }
            }
        }
    }
}

/// The record's questions as they are now (for a layout after a field
/// changed from outside): a description changed on the field becomes the
/// question, "several lines" its type. A question all of whose widgets are
/// gone goes.
fn current(record: &Record, live: &BTreeMap<u32, Live>) -> Vec<Wanted> {
    record
        .questions
        .iter()
        .filter_map(|q| {
            let alive: Vec<(&Held, &Live)> = q
                .widgets
                .iter()
                .filter_map(|w| live.get(&w.id.number()).map(|l| (w, l)))
                .collect();
            let (_, first) = alive.first()?;
            let choices = match q.kind {
                QType::One | QType::YesNo => alive.iter().map(|(_, l)| l.on.clone()).collect(),
                QType::Several => alive.iter().map(|(w, _)| w.choice.clone()).collect(),
                QType::Dropdown => first.options.clone(),
                _ => Vec::new(),
            };
            let text = if q.kind != QType::Several
                && !first.tu.is_empty()
                && Some(first.tu.as_str()) != fields_label(&q.text).as_deref()
            {
                first.tu.clone()
            } else {
                q.text.clone()
            };
            let kind = match q.kind {
                QType::Paragraph if !first.multiline && first.kind == "text" => QType::Short,
                k if first.multiline && first.kind == "text" && k != QType::Paragraph => {
                    QType::Paragraph
                }
                k => k,
            };
            Some(Wanted {
                id: Some(q.id),
                adopt: None,
                text,
                help: q.help.clone(),
                kind,
                type_auto: q.type_auto,
                choices,
                required: if q.kind == QType::Several {
                    q.required
                } else {
                    first.required
                },
                pinned: None,
                name_hint: None,
            })
        })
        .collect()
}

/// Whether a phase-1 field still has the name the engine made up for it.
fn generated_name(name: &str) -> bool {
    let numbered = |word: &str, sep: &str| {
        name.strip_prefix(word)
            .and_then(|rest| rest.strip_prefix(sep))
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    };
    ["Text", "Checkbox", "Dropdown", "Choice"]
        .iter()
        .any(|w| numbered(w, " "))
        || numbered("Signature", "")
}

/// Whether the page holds nothing but empty content and widgets (a page a
/// form may take over), and whether it has no annotations at all.
fn blank(doc: &Document<'_>, page: &Dictionary, widgets_ok: bool) -> bool {
    if page_rotation(doc, page) != 0 {
        return false;
    }
    let annots_ok = match doc.dict_get(page, b"Annots").as_array() {
        None => true,
        Some(annots) => {
            annots.is_empty()
                || widgets_ok
                    && annots.iter().all(|a| {
                        doc.resolve(a).as_dict().is_some_and(|d| {
                            doc.dict_get(d, b"Subtype").as_name() == Some(b"Widget")
                        })
                    })
        }
    };
    let Some(kept) = kept_contents(doc, page) else {
        return false;
    };
    let limits = StreamLimits {
        max_encoded_bytes: 4096,
        max_decoded_bytes: 4096,
        max_filters: 4,
        max_total_decoded_bytes: 4096,
    };
    annots_ok
        && kept.iter().all(|entry| {
            ours(doc, entry)
                || entry
                    .as_reference()
                    .and_then(|id| doc.decoded_stream(id, limits).ok())
                    .is_some_and(|data| {
                        data.iter()
                            .all(|b| b.is_ascii_whitespace() || matches!(b, b'q' | b'Q'))
                    })
        })
}

fn page_rotation(doc: &Document<'_>, page: &Dictionary) -> i64 {
    crate::app::info::page_rotation(doc, page)
}

/// Whether a /Contents entry is one of the form's label streams.
fn ours(doc: &Document<'_>, entry: &Object) -> bool {
    entry
        .as_reference()
        .and_then(|id| doc.object(id).ok())
        .and_then(|o| o.value.as_dict())
        .is_some_and(|d| d.get(MARK).is_some())
}

fn num(v: f64) -> String {
    js_number((v * 100.0).round() / 100.0)
}

/// A form page's label stream body.
fn label_stream(space: &PageSpace, ink: &[Ink]) -> Vec<u8> {
    let mut out = b"/TandemForm BMC\n".to_vec();
    for item in ink {
        match item {
            Ink::Text {
                x,
                y,
                size,
                bold,
                grey,
                text,
            } => {
                let p = space.to_pdf([*x, *y, *x, *y]);
                let font = if *bold { FONT_BOLD } else { FONT_REGULAR };
                let fill = if *grey { "0.4 g" } else { "0 g" };
                out.extend_from_slice(
                    format!(
                        "BT /{} {} Tf {fill} 1 0 0 1 {} {} Tm ",
                        String::from_utf8_lossy(font),
                        num(*size),
                        num(p[0]),
                        num(p[1])
                    )
                    .as_bytes(),
                );
                out.extend_from_slice(&literal(&win_ansi_bytes(text)));
                out.extend_from_slice(b" Tj ET\n");
            }
            Ink::Rule { x0, x1, y } => {
                let a = space.to_pdf([*x0, *y, *x0, *y]);
                let b = space.to_pdf([*x1, *y, *x1, *y]);
                out.extend_from_slice(
                    format!(
                        "0.7 G 0.5 w {} {} m {} {} l S\n",
                        num(a[0]),
                        num(a[1]),
                        num(b[0]),
                        num(b[1])
                    )
                    .as_bytes(),
                );
            }
        }
    }
    out.extend_from_slice(b"EMC\n");
    out
}

fn font(base: &str) -> Object {
    Object::dictionary([
        ("Type", Object::name("Font")),
        ("Subtype", Object::name("Type1")),
        ("BaseFont", Object::name(base)),
        ("Encoding", Object::name("WinAnsiEncoding")),
    ])
}

/// Rewrites a form page's label streams: the old ones go, `ink` (when there
/// is any) is drawn on top of the page's own content.
fn draw(
    tx: &mut Transaction<'_>,
    page: ObjectId,
    ink: &[Ink],
    fonts: &mut [Option<ObjectId>; 2],
) -> Result<(), AppError> {
    let doc = tx.document();
    let dict = doc
        .resolve_id(page)
        .as_dict()
        .ok_or_else(|| AppError::plain("form-set: a form page is not a page"))?;
    let space = PageSpace::of(doc, dict);
    let kept = kept_contents(doc, dict)
        .ok_or_else(|| AppError::plain("form-set: a form page's /Contents is not content"))?;
    let others: Vec<Object> = kept.iter().filter(|e| !ours(doc, e)).cloned().collect();
    let mut contents = Vec::with_capacity(others.len() + 2);
    if ink.is_empty() {
        contents = others;
    } else {
        let body = label_stream(&space, ink);
        for (slot, (key, base)) in [(FONT_REGULAR, "Helvetica"), (FONT_BOLD, "Helvetica-Bold")]
            .into_iter()
            .enumerate()
        {
            if has_resource(tx.document(), page, b"Font", key) {
                continue;
            }
            let id = match fonts[slot] {
                Some(id) => id,
                None => {
                    let id = tx.allocate(font(base), None)?;
                    fonts[slot] = Some(id);
                    id
                }
            };
            add_resource(tx, page, b"Font", key, id)?;
        }
        let marked = || Object::dictionary([(MARK, Object::Boolean(true))]);
        let head = tx.allocate(marked(), Some(b"q\n".to_vec()))?;
        let mut tail_bytes = b"Q\n".to_vec();
        tail_bytes.extend_from_slice(&body);
        let tail = tx.allocate(marked(), Some(tail_bytes))?;
        contents.push(Object::Reference(head));
        contents.extend(others);
        contents.push(Object::Reference(tail));
    }
    tx.update(page, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.insert(b"Contents".to_vec(), Object::Array(contents));
        }
    })?;
    Ok(())
}

/// Refuses text the labels cannot draw (Base-14 Helvetica, WinAnsi).
fn check_drawable(text: &str) -> Result<(), AppError> {
    match text
        .chars()
        .find(|&c| c != '\n' && win_ansi_byte(c).is_none())
    {
        None => Ok(()),
        Some(c) => Err(AppError::reason(
            "not-drawable",
            format!(
                "form-set: not-drawable: labels can use only Western European letters for now, and “{text}” has “{c}”"
            ),
        )),
    }
}

/// The label a question draws: its text, ` *` when required.
fn label(w: &Wanted) -> String {
    let text = if w.text.is_empty() {
        "Untitled question"
    } else {
        w.text.as_str()
    };
    if w.required {
        format!("{text} *")
    } else {
        text.to_string()
    }
}

fn shape(w: &Wanted) -> Shape {
    match w.kind {
        QType::Agree => Shape::Agree,
        QType::One | QType::YesNo | QType::Several => Shape::Boxes(w.choices.clone()),
        kind => {
            let (w, h) = kind.field_size().unwrap_or((300.0, 22.0));
            Shape::Field { w, h }
        }
    }
}

fn differs(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).any(|(x, y)| (x - y).abs() > MOVED)
}

/// Runs a field op inside the form's step.
fn run(
    session: &mut AppSession,
    op: fn(&mut AppSession, &Value) -> Result<Value, AppError>,
    request: Value,
) -> Result<Value, AppError> {
    op(session, &request)
}

fn number(reply: &Value, key: &str) -> Option<ObjectId> {
    reply
        .get(key)
        .and_then(Value::as_i64)
        .and_then(|n| u32::try_from(n).ok())
        .and_then(|n| ObjectId::new(n, 0).ok())
}

/// Adds a question's fields on `page` at `rects`; returns the widgets held
/// and the name it took.
fn add_fields(
    session: &mut AppSession,
    w: &Wanted,
    page: usize,
    rects: &[[f64; 4]],
) -> Result<(Vec<Held>, String), AppError> {
    let hint = w.name_hint.clone().unwrap_or_else(|| w.text.clone());
    let label = fields_label(&w.text);
    let mut held = Vec::new();
    let nums = |r: &[f64; 4]| json!(r.map(|v| (v * 100.0).round() / 100.0));
    let name;
    match w.kind {
        QType::One | QType::YesNo => {
            let kids: Vec<Value> = w
                .choices
                .iter()
                .zip(rects)
                .map(|(on, r)| json!({"on": on, "rect": nums(r)}))
                .collect();
            let mut request = json!({"op": "field-add", "page": page, "kind": "radio",
                "nameHint": hint, "kids": kids, "required": w.required});
            if let Some(label) = &label {
                request["label"] = json!(label);
            }
            let reply = run(session, fields::add, request)?;
            name = reply["name"].as_str().unwrap_or("").to_string();
            let ids = reply["widgetIds"].as_array().cloned().unwrap_or_default();
            for (i, r) in rects.iter().enumerate() {
                let id = ids
                    .get(i)
                    .and_then(Value::as_i64)
                    .and_then(|n| u32::try_from(n).ok())
                    .and_then(|n| ObjectId::new(n, 0).ok())
                    .ok_or_else(|| AppError::plain("form-set: a choice button was not made"))?;
                held.push(Held {
                    id,
                    laid: Some(*r),
                    choice: String::new(),
                });
            }
        }
        QType::Several => {
            let base = fields::clean_hint(&hint);
            for (choice, r) in w.choices.iter().zip(rects) {
                let mut request = json!({"op": "field-add", "page": page, "kind": "checkbox",
                    "rect": nums(r), "nameHint": format!("{base} - {choice}")});
                if let Some(label) = fields_label(&format!("{}: {choice}", w.text)) {
                    request["label"] = json!(label);
                }
                let reply = run(session, fields::add, request)?;
                let id = number(&reply, "widgetId")
                    .ok_or_else(|| AppError::plain("form-set: a checkbox was not made"))?;
                held.push(Held {
                    id,
                    laid: Some(*r),
                    choice: choice.clone(),
                });
            }
            name = base;
        }
        kind => {
            let r = rects
                .first()
                .ok_or_else(|| AppError::plain("form-set: a field has no place"))?;
            let mut request = json!({"op": "field-add", "page": page,
                "kind": kind.field_kind(), "rect": nums(r), "nameHint": hint});
            if let Some(label) = &label {
                request["label"] = json!(label);
            }
            match kind {
                QType::Signature => {}
                QType::Dropdown => {
                    request["options"] = json!(w.choices);
                    request["required"] = json!(w.required);
                }
                QType::Paragraph => {
                    request["multiline"] = json!(true);
                    request["required"] = json!(w.required);
                }
                _ => request["required"] = json!(w.required),
            }
            let reply = run(session, fields::add, request)?;
            name = reply["name"].as_str().unwrap_or("").to_string();
            let id = number(&reply, "widgetId")
                .ok_or_else(|| AppError::plain("form-set: a field was not made"))?;
            held.push(Held {
                id,
                laid: Some(*r),
                choice: String::new(),
            });
        }
    }
    Ok((held, name))
}

/// A question as a field's /TU, cut to fit.
fn fields_label(text: &str) -> Option<String> {
    let text: String = text
        .chars()
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect();
    let mut out = String::new();
    for c in text.trim().chars() {
        if js_len(&out) + c.len_utf16() > fields::MAX_FIELD_LABEL {
            break;
        }
        out.push(c);
    }
    (!out.is_empty()).then_some(out)
}

/// Whether the question's widgets are what it wants (kind, count, choices):
/// else they are made again.
fn same_shape(w: &Wanted, held: &[Held], live: &BTreeMap<u32, Live>) -> bool {
    let lives: Vec<&Live> = held
        .iter()
        .filter_map(|h| live.get(&h.id.number()))
        .collect();
    if lives.is_empty() || lives.len() != held.len() {
        return false;
    }
    let kind = w.kind.field_kind();
    if lives.iter().any(|l| l.kind != kind) {
        return false;
    }
    match w.kind {
        QType::One | QType::YesNo => {
            lives.len() == w.choices.len()
                && lives.iter().zip(&w.choices).all(|(l, c)| &l.on == c)
                && lives.iter().all(|l| l.name == lives[0].name)
        }
        QType::Several => {
            held.len() == w.choices.len()
                && held.iter().zip(&w.choices).all(|(h, c)| &h.choice == c)
        }
        _ => lives.len() == 1,
    }
}

/// Deletes widgets (each a field, or a choice button of its group).
fn delete_widgets(
    session: &mut AppSession,
    held: &[Held],
    live: &BTreeMap<u32, Live>,
) -> Result<(), AppError> {
    for h in held {
        if let Some(l) = live.get(&h.id.number()) {
            run(
                session,
                fields::delete,
                json!({"op": "field-delete", "page": l.page, "widgetId": h.id.number()}),
            )?;
        }
    }
    Ok(())
}

/// Props on one widget when anything is asked.
fn props(
    session: &mut AppSession,
    l: &Live,
    id: u32,
    changes: Map<String, Value>,
) -> Result<(), AppError> {
    if changes.is_empty() {
        return Ok(());
    }
    let mut request = json!({"op": "field-props", "page": l.page, "widgetId": id});
    if let Some(map) = request.as_object_mut() {
        map.extend(changes);
    }
    run(session, fields::props, request).map(|_| ())
}

/// Brings a kept question's fields up to date: description, required flag,
/// drop-down choices, several lines, and its name while it is automatic.
fn update_fields(
    session: &mut AppSession,
    w: &Wanted,
    stored: &mut Stored,
    live: &BTreeMap<u32, Live>,
) -> Result<(), AppError> {
    let held = stored.widgets.clone();
    let first = held
        .first()
        .and_then(|h| live.get(&h.id.number()).map(|l| (h.id.number(), l)));
    let Some((first_id, first)) = first else {
        return Ok(());
    };
    if w.kind == QType::Several {
        stored.required = w.required;
        let base_auto = held.iter().all(|h| {
            live.get(&h.id.number())
                .is_some_and(|l| l.name.starts_with(&format!("{} - ", stored.name)))
        });
        let new_base = w.name_hint.as_deref().map(fields::clean_hint);
        for h in &held {
            let Some(l) = live.get(&h.id.number()) else {
                continue;
            };
            let mut changes = Map::new();
            let tu = fields_label(&format!("{}: {}", w.text, h.choice)).unwrap_or_default();
            if tu != l.tu {
                changes.insert("label".into(), json!(tu));
            }
            if let Some(base) = new_base
                .as_ref()
                .filter(|b| base_auto && **b != stored.name)
            {
                let doc = session.require_mutable("form-set")?.document();
                let name = fields::free_name_for(doc, &format!("{base} - {}", h.choice), &l.name)?;
                if name != l.name {
                    changes.insert("name".into(), json!(name));
                }
            }
            props(session, l, h.id.number(), changes)?;
        }
        if let Some(base) = new_base.filter(|_| base_auto) {
            stored.name = base;
        }
        return Ok(());
    }
    let mut changes = Map::new();
    let tu = fields_label(&w.text).unwrap_or_default();
    if tu != first.tu {
        changes.insert("label".into(), json!(tu));
    }
    if w.kind != QType::Signature && w.required != first.required {
        changes.insert("required".into(), json!(w.required));
    }
    if w.kind == QType::Dropdown && w.choices != first.options && !w.choices.is_empty() {
        changes.insert("options".into(), json!(w.choices));
    }
    if first.kind == "text" {
        let multiline = w.kind == QType::Paragraph;
        if multiline != first.multiline {
            changes.insert("multiline".into(), json!(multiline));
        }
    }
    let auto = !stored.name.is_empty() && stored.name == first.name;
    if let Some(hint) = w
        .name_hint
        .as_deref()
        .filter(|_| auto && w.kind != QType::Signature)
    {
        let doc = session.require_mutable("form-set")?.document();
        let name = fields::free_name_for(doc, hint, &first.name)?;
        if name != first.name {
            changes.insert("name".into(), json!(name.clone()));
            stored.name = name;
        }
    }
    props(session, first, first_id, changes)
}

/// Lays the form out again, as it is (after a field changed from outside).
fn relayout(session: &mut AppSession) -> Result<(), AppError> {
    let doc = session.require_mutable("form-set")?.document();
    let mut record = read_record(doc);
    let pages = needed_pages(doc, &record);
    let (live, _) = live(session, pages)?;
    refresh_groups(&mut record, &live);
    let wanted = current(&record, &live);
    apply(session, None, None, wanted, Vec::new())
}

/// After field-props, field-delete or field-kid-add changed the widget
/// numbered `widget` on page `page`, inside that op's step: lays the form
/// out again when the page is a form page or the widget is in the list.
pub(crate) fn after_field_change(
    session: &mut AppSession,
    page: Option<ObjectId>,
    widget: u32,
) -> Result<(), AppError> {
    if session.form_busy || session.batch == Batch::Plan {
        return Ok(());
    }
    let Some(doc) = session.doc.as_ref() else {
        return Ok(());
    };
    let doc = doc.document();
    if form_dict(doc).is_none() {
        return Ok(());
    }
    let record = read_record(doc);
    let on_form_page = page.is_some_and(|p| record.pages.contains(&p));
    let linked = record
        .questions
        .iter()
        .flat_map(|q| &q.widgets)
        .any(|w| w.id.number() == widget);
    if on_form_page || linked {
        relayout(session)?;
    }
    Ok(())
}

/// The form's work, inside its step.
fn apply(
    session: &mut AppSession,
    title: Option<String>,
    desc: Option<String>,
    wanted: Vec<Wanted>,
    moves: Vec<(u32, [f64; 4])>,
) -> Result<(), AppError> {
    let busy = session.form_busy;
    session.form_busy = true;
    let result = apply_busy(session, title, desc, wanted, moves);
    session.form_busy = busy;
    result
}

fn apply_busy(
    session: &mut AppSession,
    title: Option<String>,
    desc: Option<String>,
    wanted: Vec<Wanted>,
    moves: Vec<(u32, [f64; 4])>,
) -> Result<(), AppError> {
    let op = "form-set";
    let doc = session.require_mutable(op)?.document();
    let mut record = read_record(doc);
    if let Some(title) = title {
        record.title = title;
    }
    if let Some(desc) = desc {
        record.desc = desc;
    }
    // Moves first: a moved field is a pinned one.
    if !moves.is_empty() {
        let all = 0..info::pages(doc).len();
        let (live_now, _) = live(session, all)?;
        for (id, rect) in moves {
            let l = live_now.get(&id).ok_or_else(|| {
                AppError::reason("stale-target", format!("{op}: stale-target: no field {id}"))
            })?;
            let mut changes = Map::new();
            changes.insert("rect".into(), json!(rect));
            props(session, l, id, changes)?;
        }
    }
    let doc = session.require_mutable(op)?.document();
    let mut pages_now = form_pages(doc, &record);
    // Which widgets the form's questions hold, and the fields to adopt.
    let all = 0..info::pages(doc).len();
    let (mut live_map, _) = live(session, all)?;
    refresh_groups(&mut record, &live_map);
    let is_form_page = |pages: &[usize], page: usize| pages.contains(&page);
    // Foreign: a question whose fields are on a page the form does not own.
    let foreign = |w: &Wanted, record: &Record, live: &BTreeMap<u32, Live>, pages: &[usize]| {
        let page = match (w.id, w.adopt) {
            (_, Some(adopt)) => live.get(&adopt).map(|l| l.page),
            (Some(id), _) => record
                .questions
                .iter()
                .find(|q| q.id == id)
                .and_then(|q| q.widgets.first())
                .and_then(|h| live.get(&h.id.number()))
                .map(|l| l.page),
            _ => None,
        };
        page.is_some_and(|p| !is_form_page(pages, p))
    };
    let mut flags: Vec<bool> = wanted
        .iter()
        .map(|w| foreign(w, &record, &live_map, &pages_now))
        .collect();
    // Labels are drawn for every question on the form's pages.
    for (w, f) in wanted.iter().zip(&flags) {
        if !f {
            check_drawable(&w.text)?;
            check_drawable(&w.help)?;
            for c in &w.choices {
                check_drawable(c)?;
            }
        }
    }
    check_drawable(&record.title)?;
    check_drawable(&record.desc)?;
    // A first form page: the one blank page of a new document (fields drawn on
    // it first included), else a new page at the end.
    let lone_blank = {
        let doc = session.require_mutable(op)?.document();
        let listed = info::pages(doc);
        (listed.len() == 1)
            .then(|| listed[0].id.filter(|_| blank(doc, listed[0].dict, true)))
            .flatten()
    };
    let needs_page = flags.iter().any(|f| !f) || (lone_blank.is_some() && !wanted.is_empty());
    if pages_now.is_empty() && needs_page {
        let doc = session.require_mutable(op)?.document();
        let listed = info::pages(doc);
        let take = lone_blank;
        match take {
            Some(id) => record.pages = vec![id],
            None => {
                let count = listed.len();
                run(
                    session,
                    pages::insert_blank,
                    json!({"op": "insert-blank", "after": count as i64 - 1, "rotate": 0}),
                )?;
                let doc = session.require_mutable(op)?.document();
                let id = info::pages(doc)
                    .get(count)
                    .and_then(|p| p.id)
                    .ok_or_else(|| AppError::plain("form-set: the new page has no object"))?;
                record.pages = vec![id];
                record.made.insert(id);
            }
        }
        let doc = session.require_mutable(op)?.document();
        pages_now = form_pages(doc, &record);
        let (refreshed, _) = live(session, 0..info::pages(doc).len())?;
        live_map = refreshed;
        flags = wanted
            .iter()
            .map(|w| foreign(w, &record, &live_map, &pages_now))
            .collect();
    }
    // Questions no longer listed go, and their fields on form pages with them.
    let kept_ids: BTreeSet<i64> = wanted.iter().filter_map(|w| w.id).collect();
    for q in record
        .questions
        .iter()
        .filter(|q| !kept_ids.contains(&q.id))
    {
        let on_form: Vec<Held> = q
            .widgets
            .iter()
            .filter(|h| {
                live_map
                    .get(&h.id.number())
                    .is_some_and(|l| is_form_page(&pages_now, l.page))
            })
            .cloned()
            .collect();
        delete_widgets(session, &on_form, &live_map)?;
    }
    // Each wanted question as it will be recorded.
    let next_id = record.questions.iter().map(|q| q.id).max().unwrap_or(0);
    let mut next_id = next_id;
    let mut stored: Vec<Stored> = Vec::with_capacity(wanted.len());
    for (w, &is_foreign) in wanted.iter().zip(&flags) {
        let old =
            w.id.and_then(|id| record.questions.iter().find(|q| q.id == id))
                .cloned();
        let mut s = match old {
            Some(old) => old,
            None => {
                next_id += 1;
                let widgets = match w.adopt {
                    Some(n) => {
                        let group = live_map.get(&n).map(|l| l.name.clone());
                        // A radio group is adopted whole.
                        let ids: Vec<u32> = match (&group, live_map.get(&n)) {
                            (Some(name), Some(l)) if l.kind == "radio" => live_map
                                .iter()
                                .filter(|(_, o)| o.kind == "radio" && &o.name == name)
                                .map(|(id, _)| *id)
                                .collect(),
                            _ => vec![n],
                        };
                        ids.into_iter()
                            .filter_map(|n| ObjectId::new(n, 0).ok())
                            .map(|id| Held {
                                id,
                                laid: live_map.get(&id.number()).map(|l| l.rect),
                                choice: String::new(),
                            })
                            .collect()
                    }
                    None => Vec::new(),
                };
                let name = w
                    .adopt
                    .and_then(|n| live_map.get(&n))
                    .map(|l| l.name.clone())
                    .filter(|n| generated_name(n))
                    .unwrap_or_default();
                Stored {
                    id: next_id,
                    text: String::new(),
                    help: String::new(),
                    kind: w.kind,
                    type_auto: w.type_auto,
                    required: false,
                    pinned: w.adopt.is_some() && !is_foreign,
                    name,
                    widgets,
                }
            }
        };
        // Widgets deleted from outside are dropped.
        s.widgets.retain(|h| live_map.contains_key(&h.id.number()));
        if is_foreign {
            // Its type is the field's own; only its description follows.
            if let Some(l) = s.widgets.first().and_then(|h| live_map.get(&h.id.number())) {
                s.kind = if w.adopt.is_some() || w.kind.field_kind() != l.kind {
                    QType::of_kind(&l.kind, l.multiline)
                } else {
                    w.kind
                };
            }
        } else {
            let moved = s
                .widgets
                .iter()
                .any(|h| match (h.laid, live_map.get(&h.id.number())) {
                    (Some(laid), Some(l)) => differs(laid, l.rect),
                    _ => true,
                });
            s.pinned = (s.pinned || (moved && !s.widgets.is_empty())) && w.pinned != Some(false);
            if w.adopt.is_some() {
                s.pinned = w.pinned != Some(false);
            }
            if !s.widgets.is_empty() && !same_shape(w, &s.widgets, &live_map) {
                if w.adopt.is_some() {
                    // An adopted field keeps its kind.
                    s.kind = QType::of_kind(
                        &live_map
                            .get(&s.widgets[0].id.number())
                            .map(|l| l.kind.clone())
                            .unwrap_or_default(),
                        false,
                    );
                } else {
                    delete_widgets(session, &s.widgets, &live_map)?;
                    s.widgets.clear();
                    s.pinned = false;
                }
            }
            s.kind = if w.adopt.is_some() { s.kind } else { w.kind };
        }
        s.text = w.text.clone();
        s.help = w.help.clone();
        s.type_auto = w.type_auto;
        stored.push(s);
    }
    let doc = session.require_mutable(op)?.document();
    let (refreshed, _) = live(session, 0..info::pages(doc).len())?;
    live_map = refreshed;
    // The layout of everything on the form's pages.
    let sizes: Vec<(f64, f64)> = pages_now
        .iter()
        .map(|&p| {
            let dict = info::pages(doc)[p].dict;
            let b = PageSpace::of(doc, dict).bounds();
            (b[2], b[3])
        })
        .collect();
    let slot_of = |page: usize| pages_now.iter().position(|&p| p == page);
    let mut blocks = Vec::new();
    let mut block_of = Vec::new();
    for (index, (s, (w, &is_foreign))) in stored.iter().zip(wanted.iter().zip(&flags)).enumerate() {
        if is_foreign {
            continue;
        }
        let mut wanted_now = w.clone();
        wanted_now.kind = s.kind;
        let pinned = if s.pinned && !s.widgets.is_empty() {
            let rects: Vec<[f64; 4]> = s
                .widgets
                .iter()
                .filter_map(|h| live_map.get(&h.id.number()).map(|l| l.rect))
                .collect();
            let page = live_map.get(&s.widgets[0].id.number()).map(|l| l.page);
            page.and_then(slot_of).map(|slot| (slot, rects))
        } else {
            None
        };
        if s.pinned && matches!(s.kind, QType::One | QType::YesNo) && !s.widgets.is_empty() {
            wanted_now.choices = s
                .widgets
                .iter()
                .filter_map(|h| live_map.get(&h.id.number()).map(|l| l.on.clone()))
                .collect();
        }
        if s.kind == QType::Several && s.pinned {
            wanted_now.choices = s.widgets.iter().map(|h| h.choice.clone()).collect();
        }
        blocks.push(Block {
            label: label(&wanted_now),
            help: w.help.clone(),
            shape: shape(&wanted_now),
            pinned,
        });
        block_of.push(index);
    }
    let held: BTreeSet<u32> = stored
        .iter()
        .flat_map(|s| s.widgets.iter().map(|h| h.id.number()))
        .collect();
    let fixed: Vec<(usize, [f64; 4])> = live_map
        .iter()
        .filter(|(id, _)| !held.contains(id))
        .filter_map(|(_, l)| slot_of(l.page).map(|slot| (slot, l.rect)))
        .collect();
    let laid = layout::lay_out(&sizes, &record.title, &record.desc, &blocks, &fixed);
    if laid.slots > MAX_FORM_PAGES {
        return Err(AppError::reason(
            "too-large",
            format!("{op}: a form takes at most {MAX_FORM_PAGES} pages"),
        ));
    }
    // More pages when the layout needs them, after the last form page.
    if needs_page && laid.slots > pages_now.len() {
        let after = pages_now.iter().copied().max().unwrap_or(0);
        let extra = laid.slots - pages_now.len();
        run(
            session,
            pages::insert_blank,
            json!({"op": "insert-blank", "after": after, "count": extra, "rotate": 0}),
        )?;
        let doc = session.require_mutable(op)?.document();
        let listed = info::pages(doc);
        for k in 1..=extra {
            let id = listed
                .get(after + k)
                .and_then(|p| p.id)
                .ok_or_else(|| AppError::plain("form-set: a new page has no object"))?;
            record.pages.push(id);
            record.made.insert(id);
        }
        pages_now = form_pages(doc, &record);
        let (refreshed, _) = live(session, 0..listed.len())?;
        live_map = refreshed;
    }
    // Fields placed or moved where the layout put them.
    for (block, &index) in block_of.iter().enumerate() {
        let (slot, rects) = &laid.placed[block];
        let Some(&page) = pages_now.get(*slot) else {
            continue;
        };
        let s = &mut stored[index];
        let w = &wanted[index];
        if s.pinned && !s.widgets.is_empty() {
            for h in &mut s.widgets {
                h.laid = live_map.get(&h.id.number()).map(|l| l.rect);
            }
            update_fields(session, w, s, &live_map)?;
            continue;
        }
        let on_page = s
            .widgets
            .iter()
            .all(|h| live_map.get(&h.id.number()).is_some_and(|l| l.page == page));
        if !s.widgets.is_empty() && !on_page {
            delete_widgets(session, &s.widgets, &live_map)?;
            s.widgets.clear();
        }
        if s.widgets.is_empty() {
            let mut w = w.clone();
            w.kind = s.kind;
            let (held, name) = add_fields(session, &w, page, rects)?;
            s.widgets = held;
            s.name = name;
            continue;
        }
        for (h, r) in s.widgets.iter_mut().zip(rects) {
            if let Some(l) = live_map.get(&h.id.number())
                && differs(l.rect, *r)
            {
                let mut changes = Map::new();
                changes.insert("rect".into(), json!(r.map(|v| (v * 100.0).round() / 100.0)));
                props(session, l, h.id.number(), changes)?;
            }
            h.laid = Some(*r);
        }
        let doc = session.require_mutable(op)?.document();
        let (refreshed, _) = live(session, 0..info::pages(doc).len())?;
        live_map = refreshed;
        update_fields(session, w, s, &live_map)?;
    }
    // Questions on other pages: their fields' descriptions only.
    for (index, &is_foreign) in flags.iter().enumerate() {
        if !is_foreign {
            continue;
        }
        let doc = session.require_mutable(op)?.document();
        let (refreshed, _) = live(session, 0..info::pages(doc).len())?;
        live_map = refreshed;
        let s = &mut stored[index];
        let mut w = wanted[index].clone();
        w.kind = s.kind;
        w.required = s
            .widgets
            .first()
            .and_then(|h| live_map.get(&h.id.number()))
            .is_some_and(|l| l.required)
            || wanted[index].required;
        if let Some(l) = s.widgets.first().and_then(|h| live_map.get(&h.id.number())) {
            w.choices = if w.kind == QType::Dropdown {
                l.options.clone()
            } else {
                Vec::new()
            };
        }
        w.name_hint = None;
        update_fields(session, &w, s, &live_map)?;
    }
    // Pages Tandem added that the form no longer needs go (the last first).
    let doc = session.require_mutable(op)?.document();
    let used: BTreeSet<usize> = (0..laid.slots.min(pages_now.len())).collect();
    let listed = info::pages(doc);
    let mut spare: Vec<(usize, ObjectId)> = pages_now
        .iter()
        .enumerate()
        .filter(|(slot, _)| !used.contains(slot))
        .filter_map(|(_, &p)| listed.get(p).and_then(|l| l.id.map(|id| (p, id, l.dict))))
        .filter(|(_, id, dict)| record.made.contains(id) && blank(doc, dict, false))
        .map(|(p, id, _)| (p, id))
        .collect();
    spare.sort_by_key(|s| std::cmp::Reverse(s.0));
    let mut gone = BTreeSet::new();
    for (p, id) in spare {
        let count = info::pages(session.require_mutable(op)?.document()).len();
        if count < 2 {
            break;
        }
        run(
            session,
            pages::delete,
            json!({"op": "page-delete", "page": p}),
        )?;
        gone.insert(id);
    }
    record.pages.retain(|id| !gone.contains(id));
    record.made.retain(|id| !gone.contains(id));
    // The labels.
    let doc = session.require_mutable(op)?.document();
    let index = page_indices(doc);
    let slots: Vec<(usize, ObjectId)> = record
        .pages
        .iter()
        .filter(|id| index.contains_key(id))
        .copied()
        .enumerate()
        .collect();
    let mut tx = session.tx()?;
    let mut fonts = [None, None];
    let no_ink = Vec::new();
    for (slot, page) in slots {
        let ink = laid.ink.get(slot).unwrap_or(&no_ink);
        draw(&mut tx, page, ink, &mut fonts)?;
    }
    record.questions = stored;
    write_record(&mut tx, &record)
}

/// The listing: the record's questions with their fields as they are, and
/// every other field.
fn listing(session: &AppSession) -> Result<Value, AppError> {
    let Some(app) = &session.doc else {
        return Err(AppError::plain("form-questions: no document open"));
    };
    let doc = app.document();
    let mut record = read_record(doc);
    let pages = form_pages(doc, &record);
    let index = page_indices(doc);
    let (live, order) = live(session, 0..info::pages(doc).len())?;
    refresh_groups(&mut record, &live);
    let mut held = BTreeSet::new();
    let questions: Vec<Value> = record
        .questions
        .iter()
        .filter_map(|q| {
            let alive: Vec<(&Held, &Live)> = q
                .widgets
                .iter()
                .filter_map(|w| live.get(&w.id.number()).map(|l| (w, l)))
                .collect();
            let (_, first) = *alive.first()?;
            held.extend(alive.iter().map(|(w, _)| w.id.number()));
            let choices: Vec<String> = match q.kind {
                QType::One | QType::YesNo => alive.iter().map(|(_, l)| l.on.clone()).collect(),
                QType::Several => alive.iter().map(|(w, _)| w.choice.clone()).collect(),
                QType::Dropdown => first.options.clone(),
                _ => Vec::new(),
            };
            let on_page: Vec<[f64; 4]> = alive
                .iter()
                .filter(|(_, l)| l.page == first.page)
                .map(|(_, l)| l.rect)
                .collect();
            let rect = layout_union(&on_page);
            let name = if q.kind == QType::Several {
                q.name.clone()
            } else {
                first.name.clone()
            };
            let name_auto = if q.kind == QType::Several {
                alive
                    .iter()
                    .all(|(_, l)| l.name.starts_with(&format!("{} - ", q.name)))
            } else {
                !q.name.is_empty() && q.name == first.name
            };
            Some(json!({
                "id": q.id,
                "text": q.text,
                "help": q.help,
                "type": q.kind.name(),
                "typeAuto": q.type_auto,
                "required": if q.kind == QType::Several { q.required } else { first.required },
                "choices": choices,
                "name": name,
                "nameAuto": name_auto,
                "widgetIds": alive.iter().map(|(w, _)| w.id.number()).collect::<Vec<_>>(),
                "page": first.page,
                "rect": rect,
                "pinned": q.pinned,
                "foreign": !pages.contains(&first.page),
            }))
        })
        .collect();
    // Every other field, a radio group once.
    let mut groups: Vec<(String, Vec<u32>)> = Vec::new();
    for id in order.iter().filter(|id| !held.contains(id)) {
        let l = &live[id];
        let group = (l.kind == "radio")
            .then(|| groups.iter_mut().find(|(n, _)| n == &l.name))
            .flatten();
        match group {
            Some((_, ids)) => ids.push(*id),
            None => groups.push((l.name.clone(), vec![*id])),
        }
    }
    let unlisted: Vec<Value> = groups
        .into_iter()
        .map(|(name, ids)| {
            let l = &live[&ids[0]];
            let rects: Vec<[f64; 4]> = ids
                .iter()
                .map(|id| &live[id])
                .filter(|o| o.page == l.page)
                .map(|o| o.rect)
                .collect();
            json!({
                "widgetIds": ids,
                "name": name,
                "kind": l.kind,
                "type": QType::of_kind(&l.kind, l.multiline).name(),
                "page": l.page,
                "rect": layout_union(&rects),
                "tu": l.tu,
                "required": l.required,
                "choices": if l.kind == "radio" {
                    ids.iter().map(|id| live[id].on.clone()).collect::<Vec<_>>()
                } else {
                    l.options.clone()
                },
                "onFormPage": pages.contains(&l.page),
            })
        })
        .collect();
    let made: Vec<usize> = record
        .made
        .iter()
        .filter_map(|id| index.get(id).copied())
        .collect();
    Ok(json!({
        "title": record.title,
        "desc": record.desc,
        "formPages": pages,
        "madePages": made,
        "questions": questions,
        "unlisted": unlisted,
    }))
}

fn layout_union(rects: &[[f64; 4]]) -> Vec<f64> {
    rects
        .iter()
        .copied()
        .reduce(|a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        })
        .map(|r| r.map(|v| (v * 100.0).round() / 100.0).to_vec())
        .unwrap_or_default()
}

/// The `form-questions` op.
pub fn list(session: &AppSession, request: &Value) -> Result<Value, AppError> {
    if session.locked {
        return Err(AppError::plain(
            "form-questions: document is encrypted, a password is required",
        ));
    }
    Ok(reply(request, listing(session)?))
}

/// The `form-set` op.
pub fn set(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let op = "form-set";
    let app = session.require_mutable(op)?;
    if app.info(session.locked).xfa {
        return Err(AppError::reason(
            "xfa",
            "form-set: xfa: this is an XFA form; its fields cannot be laid out",
        ));
    }
    let doc = app.document();
    let record = read_record(doc);
    let title = match request.get("title") {
        None => None,
        v => Some(check_text(v, MAX_TITLE_CHARS, op, "title")?),
    };
    let desc = match request.get("desc") {
        None => None,
        v => Some(check_text(v, MAX_DESC_CHARS, op, "desc")?),
    };
    let listed = listing(session)?;
    let unlisted: BTreeSet<u32> = listed["unlisted"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter_map(|u| u["widgetIds"][0].as_u64())
        .filter_map(|n| u32::try_from(n).ok())
        .collect();
    let wanted = parse_wanted(request, &record, &unlisted)?;
    let moves = match request.get("moves") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) if items.len() <= MAX_QUESTIONS => items
            .iter()
            .map(|m| {
                let id = m["widgetId"].as_u64().and_then(|n| u32::try_from(n).ok());
                let rect: Vec<f64> = m["rect"]
                    .as_array()
                    .map(|r| r.iter().filter_map(Value::as_f64).collect())
                    .unwrap_or_default();
                match (id, rect.as_slice()) {
                    (Some(id), [a, b, c, d]) => Ok((id, [*a, *b, *c, *d])),
                    _ => Err(invalid(format!(
                        "{op}: each move is {{widgetId, rect: [x0, y0, x1, y1]}}"
                    ))),
                }
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(invalid(format!("{op}: moves is a list"))),
    };
    session.mutate("change form", |session| {
        apply(session, title, desc, wanted, moves)
    })?;
    let mut fields = match listing(session)? {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    fields.extend(session.edit_state());
    Ok(reply(request, Value::Object(fields)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_names_are_the_engines_own() {
        assert!(generated_name("Text 1"));
        assert!(generated_name("Signature3"));
        assert!(!generated_name("Email address"));
        assert!(!generated_name("Text"));
    }

    #[test]
    fn types_round_trip() {
        for (name, kind) in TYPES {
            assert_eq!(QType::parse(name), Some(kind));
            assert_eq!(kind.name(), name);
        }
        assert_eq!(QType::of_kind("text", true), QType::Paragraph);
    }
}
