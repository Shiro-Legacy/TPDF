//! The Form tool's question list (design form, phase 2): the form's questions as cards in
//! the side panel, kept by the engine (`form-questions`, `form-set`), which lays them out on
//! the form's own pages. Every change the cards make sends the whole list once: one
//! "change form" step. The type follows the question's words until one is picked, and the
//! field's name follows the question until it is renamed (`form_guess`).

use super::form_guess;
use crate::session::Session;
use serde_json::{Value, json};
use tandem_protocol::ui::{FormView, QuestionCard};

/// The engine's listing, kept until the fields change.
#[derive(Clone, Debug, Default)]
pub struct FormModel {
    /// The `fields_seq` and revision it was read at.
    pub at: (i64, i64),
    pub listing: Value,
}

/// A question's words (or the form title, key "title") being dragged on the page:
/// let go without moving, it is a click (the question's card opens).
#[derive(Clone, Debug, PartialEq)]
pub struct LabelDrag {
    pub key: String,
    pub page: usize,
    pub rect: [f64; 4],
    pub start: [f64; 2],
    pub now: [f64; 2],
}

impl LabelDrag {
    fn delta(&self) -> [f64; 2] {
        [self.now[0] - self.start[0], self.now[1] - self.start[1]]
    }

    /// Where the words would go now.
    pub fn moved(&self) -> [f64; 4] {
        let [dx, dy] = self.delta();
        let r = self.rect;
        [r[0] + dx, r[1] + dy, r[2] + dx, r[3] + dy]
    }
}

fn rect4(value: &Value) -> Option<[f64; 4]> {
    let n: Vec<f64> = value.as_array()?.iter().filter_map(Value::as_f64).collect();
    (n.len() == 4).then(|| [n[0], n[1], n[2], n[3]])
}

const CHOICE_TYPES: [&str; 3] = ["one", "several", "dropdown"];

fn default_choices() -> Vec<String> {
    vec!["Choice 1".into(), "Choice 2".into()]
}

fn default_rows() -> Vec<String> {
    vec!["Statement 1".into(), "Statement 2".into()]
}

/// What a question of `kind` starts with: choices, a grid's scale and statements, a
/// rating's steps. Kept when it already has them.
fn fill_defaults(item: &mut Value, kind: &str) {
    if CHOICE_TYPES.contains(&kind) && strings(&item["choices"]).len() < 2 {
        item["choices"] = json!(default_choices());
    }
    if kind == "likert" {
        let scale = strings(&item["choices"]);
        if !(2..=7).contains(&scale.len()) || scale == default_choices() {
            item["choices"] = json!(form_guess::preset("agree").unwrap_or_default());
        }
        if strings(&item["rows"]).is_empty() {
            item["rows"] = json!(default_rows());
        }
    }
    if kind == "rating" && !(3..=10).contains(&item["steps"].as_u64().unwrap_or(0)) {
        item["steps"] = json!(5);
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn s(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn b(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool) == Some(true)
}

fn first_widget(value: &Value) -> i64 {
    value["widgetIds"][0].as_i64().unwrap_or(0)
}

/// A question of the listing as `form-set` takes it back.
fn item_of(question: &Value) -> Value {
    json!({
        "id": question["id"],
        "text": s(question, "text"),
        "help": s(question, "help"),
        "type": s(question, "type"),
        "typeAuto": b(question, "typeAuto"),
        "choices": question["choices"],
        "required": b(question, "required"),
        "other": b(question, "other"),
        "rows": question["rows"],
        "steps": question["steps"],
        "style": question["style"],
        "newPage": b(question, "newPage"),
    })
}

/// Choices as typed: blank ones dropped, pasted lines split, repeats dropped.
fn clean_choices(choices: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for choice in choices {
        for line in form_guess::split_pasted_choices(choice) {
            if !out.contains(&line) {
                out.push(line);
            }
        }
    }
    out
}

/// "form-set: not-drawable: labels can …" → "Labels can …".
fn plain(message: &str) -> String {
    let rest = message.strip_prefix("form-set: ").unwrap_or(message);
    let rest = match rest.split_once(": ") {
        Some((token, tail))
            if !token.is_empty() && token.bytes().all(|c| c.is_ascii_lowercase() || c == b'-') =>
        {
            tail
        }
        _ => rest,
    };
    let mut chars = rest.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

impl Session {
    /// The listing, read again when the fields changed since.
    fn form_model(&mut self) -> Option<&Value> {
        let at = (self.fields_seq, self.revision);
        let fresh = self.ui.form.as_ref().is_some_and(|m| m.at == at);
        if !fresh {
            let listing = match self.request(json!({"op": "form-questions"}), vec![]) {
                Ok(reply) => reply.value,
                Err(_) => {
                    self.ui.form = None;
                    return None;
                }
            };
            self.ui.form = Some(FormModel { at, listing });
        }
        self.ui.form.as_ref().map(|m| &m.listing)
    }

    /// Why questions cannot be laid out in this document, or "".
    fn form_blocked(&self) -> String {
        match &self.doc {
            None => "Open a document to make a form.".into(),
            Some(doc) if doc.info.xfa => {
                "This form is built in a way (XFA) that doesn't let questions be added here.".into()
            }
            Some(_) if !self.can_design() => "This file does not allow adding form fields.".into(),
            Some(_) => String::new(),
        }
    }

    /// The question list's view while the Form tool is on.
    pub(crate) fn ui_form_view(&mut self) -> Option<FormView> {
        if self.ui.tool != "form" || self.doc.is_none() {
            return None;
        }
        let blocked = self.form_blocked();
        let listing = self.form_model()?.clone();
        let mut questions = Vec::new();
        for (index, q) in listing["questions"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .enumerate()
        {
            questions.push(QuestionCard {
                key: format!("q{}", q["id"].as_i64().unwrap_or(0)),
                number: index + 1,
                text: s(q, "text"),
                help: s(q, "help"),
                kind: s(q, "type"),
                guessed: b(q, "typeAuto"),
                choices: strings(&q["choices"]),
                required: b(q, "required"),
                pinned: b(q, "pinned"),
                foreign: b(q, "foreign"),
                unlisted: false,
                name: s(q, "name"),
                page: q["page"].as_u64().unwrap_or(0) as usize,
                widget: first_widget(q),
                other: b(q, "other"),
                rows: strings(&q["rows"]),
                steps: q["steps"].as_u64().unwrap_or(0) as usize,
                stars: q["style"] != "numbers",
                new_page: b(q, "newPage"),
            });
        }
        for u in listing["unlisted"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
        {
            questions.push(QuestionCard {
                key: format!("u{}", first_widget(u)),
                number: 0,
                text: s(u, "tu"),
                help: String::new(),
                kind: s(u, "type"),
                guessed: false,
                choices: strings(&u["choices"]),
                required: b(u, "required"),
                pinned: true,
                foreign: !b(u, "onFormPage"),
                unlisted: true,
                name: s(u, "name"),
                page: u["page"].as_u64().unwrap_or(0) as usize,
                widget: first_widget(u),
                ..QuestionCard::default()
            });
        }
        let pages = strings_of_numbers(&listing["formPages"]);
        let where_note = match pages.as_slice() {
            [] => String::new(),
            [one] => format!("Questions go on page {}.", one + 1),
            [first, .., last] => format!("Questions go on pages {} to {}.", first + 1, last + 1),
        };
        let open = self
            .ui
            .form_open
            .clone()
            .filter(|key| questions.iter().any(|q| &q.key == key));
        Some(FormView {
            title: s(&listing, "title"),
            title_moved: b(&listing, "titleMoved"),
            desc: s(&listing, "desc"),
            questions,
            open,
            where_note,
            blocked,
        })
    }

    /// The form's words on `page` a press picks: each question's (its card key, a name for
    /// it) and the title's ("title").
    pub(crate) fn form_label_boxes(&mut self, page: usize) -> Vec<(String, String, [f64; 4])> {
        if self.ui.tool != "form" {
            return Vec::new();
        }
        let Some(listing) = self.form_model() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if listing["titlePage"].as_u64() == Some(page as u64)
            && let Some(r) = rect4(&listing["titleBox"])
        {
            out.push((
                "title".to_string(),
                format!("Form title “{}”: drag to move", s(listing, "title")),
                r,
            ));
        }
        for (i, q) in listing["questions"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .enumerate()
        {
            if b(q, "foreign") || q["boxPage"].as_u64() != Some(page as u64) {
                continue;
            }
            if let Some(r) = rect4(&q["box"]) {
                let what = if s(q, "type") == "section" {
                    "Section"
                } else {
                    "Question"
                };
                out.push((
                    format!("q{}", q["id"].as_i64().unwrap_or(0)),
                    format!("{what} {}: click to change it, drag to move it", i + 1),
                    r,
                ));
            }
        }
        out
    }

    /// A press, move or release of the Form tool on the page for a question's words.
    /// Returns whether it was theirs.
    pub(crate) fn form_label_pointer(&mut self, phase: &str, page: usize, x: f64, y: f64) -> bool {
        match phase {
            "down" => {
                let hit = self.form_label_boxes(page).into_iter().find(|(_, _, r)| {
                    x >= r[0] - 2.0 && x <= r[2] + 2.0 && y >= r[1] - 2.0 && y <= r[3] + 2.0
                });
                let Some((key, _, rect)) = hit else {
                    return false;
                };
                self.ui.form_label_drag = Some(LabelDrag {
                    key,
                    page,
                    rect,
                    start: [x, y],
                    now: [x, y],
                });
                true
            }
            "move" => match self.ui.form_label_drag.as_mut() {
                Some(drag) => {
                    drag.now = [x, y];
                    true
                }
                None => false,
            },
            "up" => {
                let Some(mut drag) = self.ui.form_label_drag.take() else {
                    return false;
                };
                drag.now = [x, y];
                let [dx, dy] = drag.delta();
                self.ui.panel_open = true;
                self.ui.panel_tab = "fields".into();
                if dx.hypot(dy) < 3.0 {
                    // A click: the question's card opens.
                    if drag.key != "title" {
                        self.form_open_card(Some(drag.key));
                    }
                    return true;
                }
                let mut items = self.form_items();
                if drag.key == "title" {
                    let mut request = json!({"op": "form-set", "titleMoveBy": [dx, dy]});
                    request["questions"] =
                        json!(items.into_iter().map(|(_, i)| i).collect::<Vec<_>>());
                    self.form_send(request, "Title moved");
                } else {
                    if let Some((_, item)) = items.iter_mut().find(|(k, _)| *k == drag.key) {
                        item["moveBy"] = json!([dx, dy]);
                    }
                    self.ui.form_open = Some(drag.key);
                    self.form_set(items.into_iter().map(|(_, i)| i).collect(), "Moved");
                }
                true
            }
            _ => self.ui.form_label_drag.take().is_some(),
        }
    }

    /// The card a widget belongs to, if any.
    pub(crate) fn form_card_of_widget(&mut self, widget: i64) -> Option<String> {
        let listing = self.form_model()?;
        let holds = |v: &Value| {
            v["widgetIds"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|i| i.as_i64() == Some(widget)))
        };
        if let Some(q) = listing["questions"].as_array()?.iter().find(|q| holds(q)) {
            return Some(format!("q{}", q["id"].as_i64().unwrap_or(0)));
        }
        listing["unlisted"]
            .as_array()?
            .iter()
            .find(|u| holds(u))
            .map(|u| format!("u{}", first_widget(u)))
    }

    /// The list as `form-set` takes it, each item with its card key.
    fn form_items(&mut self) -> Vec<(String, Value)> {
        let Some(listing) = self.form_model() else {
            return Vec::new();
        };
        listing["questions"]
            .as_array()
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .map(|q| (format!("q{}", q["id"].as_i64().unwrap_or(0)), item_of(q)))
            .collect()
    }

    /// An unlisted field as a new item that adopts it.
    fn form_unlisted_item(&mut self, key: &str) -> Option<Value> {
        let listing = self.form_model()?;
        let u = listing["unlisted"]
            .as_array()?
            .iter()
            .find(|u| format!("u{}", first_widget(u)) == key)?;
        Some(json!({
            "widgetId": first_widget(u),
            "text": s(u, "tu"),
            "type": s(u, "type"),
            "typeAuto": false,
            "choices": u["choices"],
            "required": b(u, "required"),
        }))
    }

    /// Sends the list; on success the page, the fields and the cards follow. Returns the
    /// listing the engine replied with.
    fn form_send(&mut self, request: Value, said: &str) -> Option<Value> {
        match self.request(request, vec![]) {
            Ok(reply) => {
                let listing = reply.value.clone();
                self.apply_edit(&crate::session::reply_journal_of(&reply), "");
                self.refresh_fields();
                self.ui.form = Some(FormModel {
                    at: (self.fields_seq, self.revision),
                    listing: listing.clone(),
                });
                self.clear_error();
                if !said.is_empty() {
                    self.announce(said);
                }
                Some(listing)
            }
            Err(err) => {
                let text = plain(&self.message_of(&err));
                self.show_error(&text);
                self.ui.form = None;
                None
            }
        }
    }

    fn form_set(&mut self, items: Vec<Value>, said: &str) -> Option<Value> {
        self.form_send(json!({"op": "form-set", "questions": items}), said)
    }

    /// The Form tool's question-list actions (`UiAction::Edit { op: "form-…" }`).
    /// Returns whether it took the action.
    pub(crate) fn ui_form_edit(&mut self, op: &str, contents: Option<&Value>) -> bool {
        let empty = json!({});
        let c = contents.unwrap_or(&empty);
        if let Some(locale) = c.get("locale").and_then(Value::as_str) {
            self.ui.form_locale = locale.to_string();
        }
        let key = c.get("key").and_then(Value::as_str).map(str::to_string);
        match op {
            "form-open" => {
                self.form_open_card(key);
                true
            }
            "form-add-question" => {
                self.form_add_question(key.as_deref(), c.get("type").and_then(Value::as_str));
                true
            }
            "form-question" => {
                if let Some(key) = key {
                    self.form_change(&key, c);
                }
                true
            }
            "form-move" => {
                if let (Some(key), Some(to)) = (key, c.get("to").and_then(Value::as_u64)) {
                    self.form_move(&key, to as usize);
                }
                true
            }
            "form-delete" => {
                if let Some(key) = key {
                    self.form_delete(&key);
                }
                true
            }
            "form-duplicate" => {
                if let Some(key) = key {
                    self.form_duplicate(&key);
                }
                true
            }
            "form-unpin" => {
                if let Some(key) = key {
                    let mut items = self.form_items();
                    if let Some((_, item)) = items.iter_mut().find(|(k, _)| *k == key) {
                        item["pinned"] = json!(false);
                    }
                    self.form_set(
                        items.into_iter().map(|(_, i)| i).collect(),
                        "Put back in line",
                    );
                }
                true
            }
            "form-title-back" => {
                let mut request = json!({"op": "form-set", "titleBack": true});
                request["questions"] = json!(
                    self.form_items()
                        .into_iter()
                        .map(|(_, i)| i)
                        .collect::<Vec<_>>()
                );
                self.form_send(request, "Title back at the top");
                true
            }
            "form-title" => {
                let mut request = json!({"op": "form-set"});
                request["questions"] = json!(
                    self.form_items()
                        .into_iter()
                        .map(|(_, i)| i)
                        .collect::<Vec<_>>()
                );
                if let Some(title) = c.get("title").and_then(Value::as_str) {
                    request["title"] = json!(title);
                }
                if let Some(desc) = c.get("desc").and_then(Value::as_str) {
                    request["desc"] = json!(desc);
                }
                self.form_send(request, "");
                true
            }
            "form-required-all" => {
                let on = c.get("on").and_then(Value::as_bool).unwrap_or(true);
                let items: Vec<Value> = self
                    .form_items()
                    .into_iter()
                    .map(|(_, mut i)| {
                        if i["type"] != "signature" && i["type"] != "section" {
                            i["required"] = json!(on);
                        }
                        i
                    })
                    .collect();
                let said = if on {
                    "Every question must be answered"
                } else {
                    "No question must be answered"
                };
                self.form_set(items, said);
                true
            }
            "form-preset" => {
                if let (Some(key), Some(choices)) = (
                    key,
                    c.get("preset")
                        .and_then(Value::as_str)
                        .and_then(form_guess::preset),
                ) {
                    let mut change = json!({"choices": choices});
                    let kind = self
                        .form_items()
                        .into_iter()
                        .find(|(k, _)| *k == key)
                        .map(|(_, i)| s(&i, "type"));
                    if !kind.is_some_and(|k| CHOICE_TYPES.contains(&k.as_str()) || k == "likert") {
                        change["type"] = json!("one");
                    }
                    self.form_change(&key, &change);
                }
                true
            }
            _ => false,
        }
    }

    /// Opens a card: its field picked and its page shown.
    fn form_open_card(&mut self, key: Option<String>) {
        self.ui.form_open = key.clone();
        let Some(key) = key else {
            return;
        };
        let Some(view) = self.ui_form_view() else {
            return;
        };
        let Some(card) = view.questions.into_iter().find(|q| q.key == key) else {
            return;
        };
        if card.widget <= 0 {
            return;
        }
        if self.wanted_page != card.page as i64 {
            self.edit("goto", Some(json!({ "page": card.page })));
        }
        self.on_field_select(&card.widget.to_string());
    }

    /// A new question after the card `after` (else at the end), of `kind` when one was
    /// picked (else its type follows its words). Its card opens.
    fn form_add_question(&mut self, after: Option<&str>, kind: Option<&str>) {
        let blocked = self.form_blocked();
        if !blocked.is_empty() {
            self.show_error(&blocked);
            return;
        }
        let mut items = self.form_items();
        let before: Vec<i64> = items.iter().filter_map(|(_, i)| i["id"].as_i64()).collect();
        // No type picked: it follows the words typed.
        let auto = kind.is_none();
        let kind = kind.unwrap_or("short");
        let mut item = json!({"text": "", "type": kind, "typeAuto": auto, "required": false});
        fill_defaults(&mut item, kind);
        if kind == "date" {
            item["help"] = json!(form_guess::date_help(&self.ui.form_locale));
        }
        let at = after
            .and_then(|key| items.iter().position(|(k, _)| k == key))
            .map_or(items.len(), |i| i + 1);
        items.insert(at, (String::new(), item));
        let Some(listing) = self.form_set(
            items.into_iter().map(|(_, i)| i).collect(),
            "Question added",
        ) else {
            return;
        };
        let new = listing["questions"]
            .as_array()
            .and_then(|qs| {
                qs.iter()
                    .filter_map(|q| q["id"].as_i64())
                    .find(|id| !before.contains(id))
            })
            .map(|id| format!("q{id}"));
        self.ui.form_open = new;
    }

    /// One card's change: `text`, `type`, `choices`, `help` or `required`.
    fn form_change(&mut self, key: &str, change: &Value) {
        let mut items = self.form_items();
        let index = match items.iter().position(|(k, _)| k == key) {
            Some(i) => i,
            None => match self.form_unlisted_item(key) {
                // A field drawn on the page joins the list at its end.
                Some(item) => {
                    items.push((key.to_string(), item));
                    items.len() - 1
                }
                None => return,
            },
        };
        let adopting = items[index].1.get("widgetId").is_some();
        let locale = self.ui.form_locale.clone();
        let item = &mut items[index].1;
        if let Some(text) = change.get("text").and_then(Value::as_str) {
            item["text"] = json!(text);
            let name = form_guess::name_from_question(text);
            if !name.is_empty() {
                item["nameHint"] = json!(name);
            }
            if b(item, "typeAuto")
                && !adopting
                && let Some(guess) = form_guess::guess_type(text)
            {
                let kind_was = s(item, "type");
                item["type"] = json!(guess.kind);
                let choices = strings(&item["choices"]);
                if !guess.choices.is_empty()
                    && (choices.is_empty()
                        || choices == default_choices()
                        || kind_was != guess.kind)
                {
                    item["choices"] = json!(guess.choices);
                }
                fill_defaults(item, guess.kind);
                if guess.kind == "date" && s(item, "help").is_empty() {
                    item["help"] = json!(form_guess::date_help(&locale));
                }
            }
        }
        if let Some(kind) = change.get("type").and_then(Value::as_str) {
            item["type"] = json!(kind);
            item["typeAuto"] = json!(false);
            fill_defaults(item, kind);
            if kind == "date" && s(item, "help").is_empty() {
                item["help"] = json!(form_guess::date_help(&locale));
            }
        }
        if let Some(choices) = change.get("choices") {
            item["choices"] = json!(clean_choices(&strings(choices)));
        }
        if let Some(help) = change.get("help").and_then(Value::as_str) {
            item["help"] = json!(help);
        }
        if let Some(required) = change.get("required").and_then(Value::as_bool) {
            item["required"] = json!(required);
        }
        if let Some(rows) = change.get("rows") {
            item["rows"] = json!(clean_choices(&strings(rows)));
        }
        for key in ["other", "newPage"] {
            if let Some(on) = change.get(key).and_then(Value::as_bool) {
                item[key] = json!(on);
            }
        }
        if let Some(steps) = change.get("steps").and_then(Value::as_u64) {
            item["steps"] = json!(steps.clamp(3, 10));
        }
        if let Some(style) = change.get("style").and_then(Value::as_str) {
            item["style"] = json!(if style == "numbers" {
                "numbers"
            } else {
                "stars"
            });
        }
        let listing = self.form_set(items.into_iter().map(|(_, i)| i).collect(), "");
        if adopting && let Some(listing) = listing {
            // The drawn field's card is a question now.
            let widget: i64 = key.trim_start_matches('u').parse().unwrap_or(0);
            let id = listing["questions"].as_array().and_then(|qs| {
                qs.iter()
                    .find(|q| {
                        q["widgetIds"]
                            .as_array()
                            .is_some_and(|ids| ids.iter().any(|i| i.as_i64() == Some(widget)))
                    })
                    .and_then(|q| q["id"].as_i64())
            });
            if let Some(id) = id {
                self.ui.form_open = Some(format!("q{id}"));
            }
        }
    }

    /// Moves a card to place `to` (0-based) in the list.
    fn form_move(&mut self, key: &str, to: usize) {
        let mut items = self.form_items();
        let Some(from) = items.iter().position(|(k, _)| k == key) else {
            return;
        };
        let item = items.remove(from);
        let to = to.min(items.len());
        if to == from {
            return;
        }
        items.insert(to, item);
        self.form_set(
            items.into_iter().map(|(_, i)| i).collect(),
            &format!("Moved to place {}", to + 1),
        );
    }

    fn form_delete(&mut self, key: &str) {
        let mut items = self.form_items();
        let Some(at) = items.iter().position(|(k, _)| k == key) else {
            return;
        };
        items.remove(at);
        if self.ui.form_open.as_deref() == Some(key) {
            self.ui.form_open = None;
        }
        self.ui.field_pop = None;
        self.form_set(
            items.into_iter().map(|(_, i)| i).collect(),
            "Question deleted",
        );
    }

    fn form_duplicate(&mut self, key: &str) {
        let mut items = self.form_items();
        let Some(at) = items.iter().position(|(k, _)| k == key) else {
            return;
        };
        let mut copy = items[at].1.clone();
        if let Some(map) = copy.as_object_mut() {
            map.remove("id");
        }
        let before: Vec<i64> = items.iter().filter_map(|(_, i)| i["id"].as_i64()).collect();
        items.insert(at + 1, (String::new(), copy));
        if let Some(listing) = self.form_set(
            items.into_iter().map(|(_, i)| i).collect(),
            "Question copied",
        ) {
            self.ui.form_open = listing["questions"]
                .as_array()
                .and_then(|qs| {
                    qs.iter()
                        .filter_map(|q| q["id"].as_i64())
                        .find(|id| !before.contains(id))
                })
                .map(|id| format!("q{id}"));
        }
    }

    /// Delete on a field the list holds (on a form page): its question goes, label and
    /// all, in one step. Returns whether it did.
    pub(crate) fn form_delete_widget(&mut self, widget: i64) -> bool {
        let Some(key) = self.form_card_of_widget(widget) else {
            return false;
        };
        if !key.starts_with('q') {
            return false;
        }
        let foreign = self
            .form_model()
            .and_then(|l| {
                l["questions"]
                    .as_array()?
                    .iter()
                    .find(|q| format!("q{}", q["id"].as_i64().unwrap_or(0)) == key)
                    .map(|q| b(q, "foreign"))
            })
            .unwrap_or(true);
        if foreign {
            return false;
        }
        self.form_delete(&key);
        true
    }
}

fn strings_of_numbers(value: &Value) -> Vec<usize> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_u64)
                .map(|n| n as usize)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_read_plainly() {
        assert_eq!(
            plain("form-set: not-drawable: labels can use only Western European letters"),
            "Labels can use only Western European letters"
        );
        assert_eq!(
            plain("form-set: choices: this question needs 2 to 32 choices"),
            "This question needs 2 to 32 choices"
        );
    }

    #[test]
    fn pasted_and_repeated_choices_are_cleaned() {
        let typed = vec![
            "Red\nGreen".to_string(),
            "".into(),
            "Red".into(),
            "Blue".into(),
        ];
        assert_eq!(clean_choices(&typed), ["Red", "Green", "Blue"]);
    }
}
