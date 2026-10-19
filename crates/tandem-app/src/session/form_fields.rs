//! Form fields (renderer.ts 7288–7545): the Fields panel's own copy of the shown page's
//! widgets, one `field-set` per commit, and the `P2_EDIT.fields` slice. The panel's rows are
//! drawn by the UI from [`Session::field_rows`] (the TS built them as DOM nodes).
#![allow(dead_code)]

use super::Session;
use super::dom_ids as ids;
use super::types::*;
use crate::engine::Reply;
use crate::host::util::js_number;
use serde_json::{Map, Value, json};
use tandem_protocol::engine::{FieldKind, JournalState};
use tandem_protocol::limits::{MAX_FIELD_TEXT_CHARS, MAX_NOTE_CHARS};

/// The journal slice of a mutation, undo/redo or page reply (what applyEdit reads of it).
pub(crate) fn journal_of(reply: &Reply) -> JournalState {
    let flag = |key: &str| reply.get(key).and_then(Value::as_bool).unwrap_or(false);
    JournalState {
        revision: reply.get("revision").and_then(Value::as_i64).unwrap_or(0),
        dirty: flag("dirty"),
        can_undo: flag("canUndo"),
        can_redo: flag("canRedo"),
    }
}

/// Numbers as the renderer's JSON carried them: a whole number is an integer (`72`, not `72.0`).
pub(crate) fn nums(values: &[f64]) -> Value {
    Value::Array(values.iter().map(|v| js_number(*v)).collect())
}

/// R:7317.
pub fn field_label(item: &FieldItem) -> String {
    if !item.label.is_empty() {
        item.label.clone()
    } else if !item.name.is_empty() {
        item.name.clone()
    } else {
        format!("field {}", item.id)
    }
}

/// R:7336.
pub fn control_id(item: &FieldItem) -> String {
    format!("field-{}", item.id)
}

/// R:7476.
pub fn field_announcement(item: &FieldItem, value: &FieldValue) -> String {
    match item.kind {
        FieldKind::Checkbox => {
            let on = !matches!(value, FieldValue::Bool(false))
                && *value != FieldValue::Text(String::new());
            format!("{} {}", if on { "Checked" } else { "Unchecked" }, item.name)
        }
        FieldKind::Radio => {
            let on = item
                .on
                .as_deref()
                .filter(|on| !on.is_empty())
                .unwrap_or(&item.name);
            format!("Chose {on}")
        }
        FieldKind::Text => format!("Filled {}", item.name),
        _ => format!("Chose {}", value.to_js_string()),
    }
}

/// The native control one widget gets (`fieldRow`, R:7387).
#[derive(Clone, Debug, PartialEq)]
pub enum FieldControl {
    Checkbox {
        checked: bool,
    },
    TextArea {
        value: String,
    },
    Text {
        value: String,
    },
    /// An editable combo: a text input with a `<datalist>` of the options.
    Combo {
        value: String,
        options: Vec<String>,
    },
    Select {
        value: String,
        options: Vec<String>,
    },
}

/// One radio of a group's fieldset (R:7456).
#[derive(Clone, Debug, PartialEq)]
pub struct RadioOption {
    pub field_id: i64,
    pub control_id: String,
    /// The `name` that groups the radios: `field-radio-<name>-<first id>`.
    pub group: String,
    pub value: String,
    pub label: String,
    pub checked: bool,
    pub disabled: bool,
}

/// One row of the Fields list (R:7365-7380).
#[derive(Clone, Debug, PartialEq)]
pub enum FieldRowView {
    Control {
        field_id: i64,
        control_id: String,
        label: String,
        control: FieldControl,
        /// `maxLength`, when the control has one.
        max_length: Option<i64>,
        /// A read-only field and a document without the form permission are both dead ends.
        disabled: bool,
        /// §8.10: `aria-required` and the `*` mark.
        required: bool,
        /// The scripted warning under the row.
        scripted: bool,
    },
    Radio {
        legend: String,
        required: bool,
        options: Vec<RadioOption>,
        scripted: bool,
    },
    /// A signature field's line: `Signature field <name> — <status>; click it on the page to sign`.
    Note(String),
}

impl Session {
    /// R:7294: the panel's own copy of the page's widgets, fetched on every page change and
    /// after every edit.
    pub fn refresh_fields(&mut self) {
        self.fields_seq += 1;
        if self.doc.as_ref().is_none_or(|doc| doc.info.needs_password) {
            self.fields = Vec::new();
            self.fields_page = -1;
            self.fields_tabs = None;
            self.render_fields();
            return;
        }
        let page = self.wanted_page;
        // A document that changed under us owns the panel now; a real failure is not worth an alert.
        let Ok(reply) = self
            .engine
            .request(json!({"op": "fields", "page": page}), vec![])
        else {
            return;
        };
        let items = reply.get("items").unwrap_or(&Value::Null);
        let Some(items) = super::parse_reply::<Vec<FieldItem>>(items) else {
            return;
        };
        self.fields = items;
        self.fields_page = page;
        self.fields_tabs = super::nav::tabs_name(reply.get("tabs").and_then(Value::as_str));
        if self.direct_focused_page == page
            && let Some(focused) = &self.direct_focused_field
        {
            let id = focused.id;
            self.direct_focused_field = self.fields.iter().find(|item| item.id == id).cloned();
        }
        self.render_fields();
        self.repaint_hits();
    }

    /// `paintHits(canvas.clientWidth, canvas.clientHeight, shownPage)`: the canvas's CSS size is
    /// the shown page at the shown zoom.
    pub(crate) fn repaint_hits(&mut self) {
        let size = usize::try_from(self.shown_page)
            .ok()
            .and_then(|page| self.doc.as_ref()?.pages.get(page).cloned());
        let (width, height) = size.map_or((0.0, 0.0), |s| {
            (s.width * self.shown_zoom, s.height * self.shown_zoom)
        });
        self.paint_hits(width, height, self.shown_page);
    }

    /// R:7348: the panel's heading, warning, empty line and buttons; the rows are
    /// [`Self::field_rows`]. (The TS also gave focus back to the control the user was in.)
    pub fn render_fields(&mut self) {
        self.dom.set_text(
            ids::FIELDS_HEADING,
            format!("Form fields on page {}", self.wanted_page + 1),
        );
        self.dom.set_hidden(
            ids::FIELDS_WARNING_EL,
            !self.fields.iter().any(|item| item.scripted),
        );
        let locked = self.doc.as_ref().is_none_or(|doc| doc.info.needs_password);
        self.dom
            .set_hidden(ids::FIELDS_EL, locked || self.fields.is_empty());
        let can = self.can_annotate();
        let page_text = if can {
            "No fillable fields on this page. Click the page to type."
        } else {
            "No fillable fields on this page."
        };
        self.set_empty(
            ids::FIELDS_EMPTY_EL,
            self.fields.len(),
            page_text,
            "No document open yet.",
        );
        let off = self.doc.is_none() || !can;
        self.dom.set_disabled(ids::FILL_SIGN_BUTTON, off);
        self.dom.set_disabled(ids::FILL_SIGN_DRAW_BUTTON, off);
        self.render_designer();
    }

    /// R:7357-7380, `fieldRow` (R:7387) and `radioFieldset` (R:7449): the Fields list rows.
    pub fn field_rows(&self) -> Vec<FieldRowView> {
        let can_form = self.can_form();
        let mut rows = Vec::new();
        let mut radio_done: Vec<&str> = Vec::new();
        for item in &self.fields {
            match item.kind {
                FieldKind::Radio => {
                    if radio_done.contains(&item.name.as_str()) {
                        continue;
                    }
                    radio_done.push(&item.name);
                    let group: Vec<&FieldItem> = self
                        .fields
                        .iter()
                        .filter(|kid| kid.kind == FieldKind::Radio && kid.name == item.name)
                        .collect();
                    let options = group
                        .iter()
                        .map(|kid| {
                            let on = kid.on.clone().filter(|on| !on.is_empty());
                            RadioOption {
                                field_id: kid.id,
                                control_id: control_id(kid),
                                group: format!("field-radio-{}-{}", kid.name, item.id),
                                value: on.clone().unwrap_or_else(|| kid.id.to_string()),
                                label: on.unwrap_or_else(|| format!("option {}", kid.id)),
                                checked: kid.value == FieldValue::Bool(true),
                                disabled: kid.read_only || !can_form,
                            }
                        })
                        .collect();
                    rows.push(FieldRowView::Radio {
                        legend: field_label(item),
                        required: item.required,
                        options,
                        scripted: group.iter().any(|kid| kid.scripted),
                    });
                }
                FieldKind::Signature => {
                    rows.push(FieldRowView::Note(format!(
                        "Signature field {} — {}; click it on the page to sign",
                        item.name,
                        self.sig_status(item)
                    )));
                }
                _ => {
                    let value = item.value.to_js_string();
                    let (control, max_length) = match item.kind {
                        FieldKind::Checkbox => (
                            FieldControl::Checkbox {
                                checked: item.value == FieldValue::Bool(true),
                            },
                            None,
                        ),
                        FieldKind::Text if item.multiline == Some(true) => (
                            FieldControl::TextArea { value },
                            Some(item.max_len.unwrap_or(MAX_NOTE_CHARS as i64)),
                        ),
                        FieldKind::Text => (
                            FieldControl::Text { value },
                            Some(item.max_len.unwrap_or(MAX_NOTE_CHARS as i64)),
                        ),
                        FieldKind::Combo if item.edit == Some(true) => (
                            FieldControl::Combo {
                                value,
                                options: item.options.clone().unwrap_or_default(),
                            },
                            Some(MAX_FIELD_TEXT_CHARS as i64),
                        ),
                        _ => (
                            FieldControl::Select {
                                value,
                                options: item.options.clone().unwrap_or_default(),
                            },
                            None,
                        ),
                    };
                    rows.push(FieldRowView::Control {
                        field_id: item.id,
                        control_id: control_id(item),
                        label: field_label(item),
                        control,
                        max_length,
                        disabled: item.read_only || !can_form,
                        required: item.required,
                        scripted: item.scripted,
                    });
                }
            }
        }
        rows
    }

    /// UI entry: a row's `change` (blur or Enter for text, a click for a box or radio).
    /// A radio commits `true`.
    pub fn on_field_change(&mut self, field_id: i64, value: FieldValue) {
        if let Some(item) = self.fields.iter().find(|item| item.id == field_id).cloned() {
            let page = self.wanted_page.max(0) as usize;
            self.commit_field(&item, value, page);
        }
    }

    /// R:7488: one `field-set` per commit, then the panel and the page follow the reply. A
    /// refusal reaches the user and the control snaps back without dirtying anything.
    /// (TS default: `page = wantedPage`.)
    pub fn commit_field(&mut self, item: &FieldItem, value: FieldValue, page: usize) -> bool {
        if self.doc.is_none() {
            return false;
        }
        let request = json!({"op": "field-set", "page": page, "widgetId": item.id, "value": value});
        match self.request(request, vec![]) {
            Ok(reply) => {
                self.note_filled();
                self.dom
                    .set_text(ids::FIELDS_STATUS_EL, field_announcement(item, &value));
                self.apply_edit(&journal_of(&reply), "");
                true
            }
            Err(err) => {
                let message = self.message_of(&err);
                self.dom.set_text(ids::FIELDS_STATUS_EL, message.clone());
                self.show_error(&message);
                self.refresh_fields();
                false
            }
        }
    }

    /// R:7505: the `P2_EDIT.fields` slice: how many, how many scripted, and every value by name.
    pub fn field_summary(&self) -> Value {
        let mut values = Map::new();
        for item in &self.fields {
            match item.kind {
                FieldKind::Signature => continue,
                FieldKind::Radio => {
                    // The group keeps its key even with nothing selected, so a name never disappears.
                    if !values.contains_key(&item.name) {
                        values.insert(item.name.clone(), json!(""));
                    }
                    if item.value == FieldValue::Bool(true) {
                        values.insert(
                            item.name.clone(),
                            json!(item.on.clone().unwrap_or_default()),
                        );
                    }
                }
                _ => {
                    values.insert(item.name.clone(), json!(item.value));
                }
            }
        }
        let (order, used) = super::nav::tab_order(&self.fields, self.fields_tabs.as_deref());
        let order: Vec<String> = order
            .iter()
            .filter(|item| !item.read_only && item.kind != FieldKind::Signature)
            .map(|item| item.name.clone())
            .collect();
        let mut out = Map::new();
        out.insert("count".into(), json!(self.fields.len()));
        out.insert(
            "scripted".into(),
            json!(self.fields.iter().filter(|item| item.scripted).count()),
        );
        out.insert("values".into(), Value::Object(values));
        out.insert("defs".into(), self.field_defs());
        if let Some(design) = &self.field_design {
            out.insert("last".into(), json!(design));
        }
        out.insert("tabs".into(), json!(self.fields_tabs));
        out.insert("tabsOrder".into(), json!(used));
        out.insert("order".into(), json!(order));
        Value::Object(out)
    }

    /// R:7525: every field's kind and rect, radio kids as `<name>.<on>`, and the properties
    /// "only when set": a raw `/TU`, a true flag bit, a stored `/DV`, an editable combo.
    pub fn field_defs(&self) -> Value {
        let properties = |item: &FieldItem, def: &mut Map<String, Value>| {
            if let Some(tu) = &item.tu {
                def.insert("label".into(), json!(tu));
            }
            if item.read_only {
                def.insert("readOnly".into(), json!(true));
            }
            if item.required {
                def.insert("required".into(), json!(true));
            }
            if let Some(default) = &item.default {
                def.insert("default".into(), json!(default));
            }
            if item.edit == Some(true) {
                def.insert("edit".into(), json!(true));
            }
        };
        let mut defs = Map::new();
        for item in &self.fields {
            if item.kind == FieldKind::Signature {
                continue;
            }
            if item.kind == FieldKind::Radio {
                let on = item.on.clone().unwrap_or_else(|| item.id.to_string());
                defs.insert(
                    format!("{}.{}", item.name, on),
                    json!({"kind": "radio", "rect": nums(&item.rect)}),
                );
                if !defs.contains_key(&item.name) {
                    let mut def = Map::new();
                    def.insert("kind".into(), json!("radio"));
                    def.insert("rect".into(), nums(&item.rect));
                    properties(item, &mut def);
                    defs.insert(item.name.clone(), Value::Object(def));
                }
                continue;
            }
            let mut def = Map::new();
            def.insert("kind".into(), json!(item.kind));
            def.insert("rect".into(), nums(&item.rect));
            properties(item, &mut def);
            defs.insert(item.name.clone(), Value::Object(def));
        }
        Value::Object(defs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: FieldKind, name: &str) -> FieldItem {
        FieldItem {
            id: 7,
            name: name.into(),
            kind,
            rect: vec![1.0, 2.0, 3.5, 4.0],
            ..FieldItem::default()
        }
    }

    #[test]
    fn labels_and_announcements() {
        let mut text = item(FieldKind::Text, "name");
        assert_eq!(field_label(&text), "name");
        text.label = "Your name".into();
        assert_eq!(field_label(&text), "Your name");
        assert_eq!(field_label(&item(FieldKind::Text, "")), "field 7");
        assert_eq!(
            field_announcement(&text, &FieldValue::Text("Ada".into())),
            "Filled name"
        );
        let check = item(FieldKind::Checkbox, "agree");
        assert_eq!(
            field_announcement(&check, &FieldValue::Bool(true)),
            "Checked agree"
        );
        assert_eq!(
            field_announcement(&check, &FieldValue::Bool(false)),
            "Unchecked agree"
        );
        let mut radio = item(FieldKind::Radio, "colour");
        assert_eq!(
            field_announcement(&radio, &FieldValue::Bool(true)),
            "Chose colour"
        );
        radio.on = Some("red".into());
        assert_eq!(
            field_announcement(&radio, &FieldValue::Bool(true)),
            "Chose red"
        );
        assert_eq!(
            field_announcement(&item(FieldKind::Combo, "c"), &FieldValue::Text("b".into())),
            "Chose b"
        );
    }

    #[test]
    fn defs_name_radio_kids_and_set_properties_only() {
        let mut session = Session::new();
        let mut red = item(FieldKind::Radio, "colour");
        red.on = Some("red".into());
        red.required = true;
        let mut text = item(FieldKind::Text, "name");
        text.tu = Some("".into());
        text.default = Some(FieldValue::Text("x".into()));
        session.fields = vec![red, text, item(FieldKind::Signature, "sig")];
        let defs = session.field_defs();
        assert_eq!(
            defs["colour.red"],
            json!({"kind": "radio", "rect": [1, 2, 3.5, 4]})
        );
        assert_eq!(
            defs["colour"],
            json!({"kind": "radio", "rect": [1, 2, 3.5, 4], "required": true})
        );
        assert_eq!(
            defs["name"],
            json!({"kind": "text", "rect": [1, 2, 3.5, 4], "label": "", "default": "x"})
        );
        assert!(defs.get("sig").is_none());
    }
}
