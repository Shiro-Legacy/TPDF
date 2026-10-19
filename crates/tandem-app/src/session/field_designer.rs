//! The field designer, U6 (renderer.ts 7546–7809): add a field, change or delete the selected
//! one, the refusal words, and `refusalOf`, which every part reads a worker refusal with.
#![allow(dead_code)]

use super::Session;
use super::consts;
use super::dom_ids as ids;
use super::form_fields::{journal_of, nums};
use super::types::*;
use crate::engine::Refusal;
use crate::host::util::js_number;
use crate::js::{js_num, js_round, js_trim};
use serde_json::{Map, Value, json};
use tandem_protocol::engine::FieldKind;

/// R:7571 `refusalOf(err)`: the worker's refusal reply and the error's own `reason` tag.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RefusalOf {
    /// The `{ ok: false }` reply, when the worker refused (None for the renderer's own errors).
    pub reply: Option<Value>,
    pub reason: Option<String>,
}

/// R:7579: the refusal token a worker error carries, `"invalid"` when it has none.
pub fn refusal_detail(err: &Refusal) -> String {
    err.reply
        .get("detail")
        .and_then(Value::as_str)
        .unwrap_or("invalid")
        .to_string()
}

/// R:7600.
pub fn round2(value: f64) -> f64 {
    js_round(value * 100.0) / 100.0
}

/// R:7601.
pub fn round3(value: f64) -> f64 {
    js_round(value * 1000.0) / 1000.0
}

/// JavaScript's `Number(text)` for an input's value: "" is 0, junk is NaN.
pub fn js_to_number(text: &str) -> f64 {
    let t = js_trim(text);
    if t.is_empty() {
        return 0.0;
    }
    for (prefix, radix) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
        if let Some(digits) = t.strip_prefix(prefix) {
            return u64::from_str_radix(digits, radix).map_or(f64::NAN, |v| v as f64);
        }
    }
    let (sign, body) = match t.as_bytes()[0] {
        b'+' => (1.0, &t[1..]),
        b'-' => (-1.0, &t[1..]),
        _ => (1.0, t),
    };
    if body == "Infinity" {
        return sign * f64::INFINITY;
    }
    let plain = !body.is_empty()
        && !body.starts_with(['+', '-'])
        && body
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'));
    if !plain {
        return f64::NAN;
    }
    body.parse::<f64>().map_or(f64::NAN, |v| sign * v)
}

/// `Number(text) || fallback`.
pub(crate) fn number_or(text: &str, fallback: f64) -> f64 {
    let value = js_to_number(text);
    if value == 0.0 || value.is_nan() {
        fallback
    } else {
        value
    }
}

/// R:7596: one non-empty trimmed line per entry (the options and radio-kid textareas).
pub fn textarea_lines(value: &str) -> Vec<String> {
    value
        .split('\n')
        .map(|line| js_trim(line).to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

/// `FIELD_DETAIL_TEXT[detail] ?? messageOf(err)`.
fn detail_text(detail: &str, message: String) -> String {
    consts::lookup(&consts::FIELD_DETAIL_TEXT, detail).map_or(message, str::to_string)
}

/// One option of `#field-select` (R:7634): value, text, disabled, title.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldSelectOption {
    pub value: String,
    pub label: String,
    pub disabled: bool,
    pub title: Option<String>,
}

impl Session {
    /// R:7571.
    pub(crate) fn refusal_of(&self, err: &Refusal) -> RefusalOf {
        RefusalOf {
            reply: err.reply.is_object().then(|| err.reply.clone()),
            reason: err.reason.clone(),
        }
    }

    /// R:7585: the designer's rect in page points, y down, from x, y, width, height inputs.
    pub fn designer_rect(&self, els: [&str; 4]) -> Vec<f64> {
        let number = |id: &str, fallback: f64| {
            let value = js_to_number(&self.dom.value(id));
            if value.is_finite() { value } else { fallback }
        };
        let x = number(els[0], 72.0);
        let y = number(els[1], 72.0);
        vec![x, y, x + number(els[2], 180.0), y + number(els[3], 24.0)]
    }

    /// `#field-select`'s options (R:7634-7643), drawn by the UI.
    pub fn field_select_options(&self) -> Vec<FieldSelectOption> {
        self.fields
            .iter()
            .map(|item| {
                let label = if item.kind == FieldKind::Radio {
                    format!("{} · {}", item.name, item.on.clone().unwrap_or_default())
                } else if !item.name.is_empty() {
                    item.name.clone()
                } else if !item.label.is_empty() {
                    item.label.clone()
                } else {
                    format!("field {}", item.id)
                };
                let signed = item.kind == FieldKind::Signature && self.sig_signed(item);
                let title = if signed {
                    consts::lookup(&consts::FIELD_DETAIL_TEXT, "signature").map(str::to_string)
                } else if item.scripted {
                    Some(consts::SCRIPT_WARNING.to_string())
                } else {
                    None
                };
                FieldSelectOption {
                    value: item.id.to_string(),
                    label,
                    disabled: item.scripted || signed,
                    title,
                }
            })
            .collect()
    }

    /// The field `#field-select` names.
    fn selected_field(&self) -> Option<FieldItem> {
        let value = self.dom.value(ids::FIELD_SELECT_EL);
        self.fields
            .iter()
            .find(|item| item.id.to_string() == value)
            .cloned()
    }

    /// R:7604: the designer shows what the current kind needs and lists the page's fields.
    pub fn render_designer(&mut self) {
        let design = self.can_design();
        let (open, locked, xfa) = match &self.doc {
            Some(doc) => (true, doc.info.needs_password, doc.info.xfa),
            None => (false, false, false),
        };
        let usable = open && !locked && design && !xfa;
        self.dom
            .set_hidden(ids::DESIGNER_EL, !open || locked || !(design || xfa));
        if !open || locked {
            return;
        }
        self.dom.set_hidden(ids::DESIGNER_HINT_EL, false);
        // §7.2 visibility, and the ceilings §6 says the hint must state.
        self.dom.set_text(
            ids::DESIGNER_HINT_EL,
            if xfa {
                "This is an XFA form; fields cannot be added"
            } else {
                "Position is in page points from the page top-left, unrotated. Top-level fields only, Helvetica 12 pt; no list boxes or push buttons. A signature field's name is optional."
            },
        );
        self.dom.set_disabled(ids::FIELD_NEW_NAME_EL, !usable);
        self.dom.set_disabled(ids::FIELD_NEW_ADD_EL, !usable);
        self.dom.set_disabled(ids::FIELD_NEW_KIND_EL, !usable);
        let kind = self.dom.value(ids::FIELD_NEW_KIND_EL);
        self.dom.set_hidden(ids::FIELD_NEW_RECT_EL, kind == "radio");
        self.dom.set_hidden(ids::FIELD_NEW_TEXT_EL, kind != "text");
        self.dom
            .set_hidden(ids::FIELD_NEW_COMBO_EL, kind != "combo");
        self.dom
            .set_hidden(ids::FIELD_NEW_RADIO_EL, kind != "radio");
        // U7: a checkbox default is a checked box; every other kind reads or names a value.
        self.dom.set_hidden(
            ids::FIELD_NEW_DEFAULT_TEXT_EL,
            kind == "checkbox" || kind == "signature",
        );
        self.dom
            .set_hidden(ids::FIELD_NEW_DEFAULT_BOOL_EL, kind != "checkbox");
        // The checkbox default is its own 16x16, the rest keep the text-box 180x24.
        let (w, h) = (
            self.dom.value(ids::FIELD_NEW_WEL),
            self.dom.value(ids::FIELD_NEW_HEL),
        );
        if kind == "checkbox" && w == "180" && h == "24" {
            self.dom.set_value(ids::FIELD_NEW_WEL, "16");
            self.dom.set_value(ids::FIELD_NEW_HEL, "16");
        } else if kind != "checkbox" && w == "16" && h == "16" {
            self.dom.set_value(ids::FIELD_NEW_WEL, "180");
            self.dom.set_value(ids::FIELD_NEW_HEL, "24");
        }
        // Rebuilding the options selects the first enabled one, unless the previous value is kept.
        let previous = self.dom.value(ids::FIELD_SELECT_EL);
        let options = self.field_select_options();
        let value = if options.iter().any(|option| option.value == previous) {
            previous
        } else {
            options
                .iter()
                .find(|option| !option.disabled)
                .map(|option| option.value.clone())
                .unwrap_or_default()
        };
        self.dom.set_value(ids::FIELD_SELECT_EL, value);
        self.render_selected_field();
    }

    /// UI entry: `#field-new-kind` changed.
    pub fn on_field_new_kind(&mut self, kind: &str) {
        self.dom.set_value(ids::FIELD_NEW_KIND_EL, kind);
        self.render_designer();
    }

    /// UI entry: `#field-select` changed.
    pub fn on_field_select(&mut self, value: &str) {
        self.dom.set_value(ids::FIELD_SELECT_EL, value);
        self.render_selected_field();
    }

    /// R:7649: the selected field's values, with the inputs that do not apply to its kind disabled.
    pub fn render_selected_field(&mut self) {
        let item = self.selected_field();
        let signed = item
            .as_ref()
            .is_some_and(|item| item.kind == FieldKind::Signature && self.sig_signed(item));
        let usable =
            item.as_ref().is_some_and(|item| !item.scripted) && !signed && self.can_design();
        self.dom.set_disabled(ids::FIELD_PROP_APPLY_EL, !usable);
        self.dom.set_disabled(ids::FIELD_PROP_DELETE_EL, !usable);
        // §9.1/§9.19: a signature field keeps its name (display only).
        let kind = item.as_ref().map_or(FieldKind::Text, |item| item.kind);
        self.dom.set_disabled(
            ids::FIELD_PROP_NAME_EL,
            !usable
                || item
                    .as_ref()
                    .is_some_and(|item| item.kind == FieldKind::Signature),
        );
        self.dom
            .set_hidden(ids::FIELD_PROP_RECT_EL, item.is_none() || signed);
        self.dom
            .set_hidden(ids::FIELD_PROP_TEXT_EL, kind != FieldKind::Text);
        self.dom
            .set_hidden(ids::FIELD_PROP_COMBO_EL, kind != FieldKind::Combo);
        self.dom.set_disabled(
            ids::FIELD_PROP_MAX_LEN_EL,
            !usable || kind != FieldKind::Text,
        );
        self.dom.set_disabled(
            ids::FIELD_PROP_MULTILINE_EL,
            !usable || kind != FieldKind::Text,
        );
        self.dom.set_disabled(
            ids::FIELD_PROP_OPTIONS_EL,
            !usable || kind != FieldKind::Combo,
        );
        self.dom.set_disabled(ids::FIELD_PROP_LABEL_EL, !usable);
        self.dom.set_disabled(ids::FIELD_PROP_READ_ONLY_EL, !usable);
        self.dom.set_disabled(ids::FIELD_PROP_REQUIRED_EL, !usable);
        self.dom.set_disabled(
            ids::FIELD_PROP_DEFAULT_TEXT_EL,
            !usable || kind == FieldKind::Signature,
        );
        self.dom.set_disabled(
            ids::FIELD_PROP_DEFAULT_BOOL_EL,
            !usable || kind == FieldKind::Signature,
        );
        self.dom
            .set_hidden(ids::FIELD_PROP_DEFAULT_TEXT_EL, kind == FieldKind::Checkbox);
        self.dom
            .set_hidden(ids::FIELD_PROP_DEFAULT_BOOL_EL, kind != FieldKind::Checkbox);
        let Some(item) = item else {
            self.dom.set_value(ids::FIELD_PROP_NAME_EL, "");
            return;
        };
        self.dom
            .set_value(ids::FIELD_PROP_NAME_EL, item.name.clone());
        // §8.4: the label input is the field's own `/TU`, so applying without touching it never writes one.
        self.dom.set_value(
            ids::FIELD_PROP_LABEL_EL,
            item.tu.clone().unwrap_or_default(),
        );
        self.dom
            .set_checked(ids::FIELD_PROP_READ_ONLY_EL, item.read_only);
        self.dom
            .set_checked(ids::FIELD_PROP_REQUIRED_EL, item.required);
        if kind == FieldKind::Checkbox {
            self.dom.set_checked(
                ids::FIELD_PROP_DEFAULT_BOOL_EL,
                item.default == Some(FieldValue::Bool(true)),
            );
        } else {
            let default = item
                .default
                .as_ref()
                .and_then(|d| d.as_text())
                .unwrap_or("")
                .to_string();
            self.dom.set_value(ids::FIELD_PROP_DEFAULT_TEXT_EL, default);
        }
        let [x0, y0, x1, y1] = match item.rect.as_slice() {
            [a, b, c, d] => [*a, *b, *c, *d],
            _ => [0.0; 4],
        };
        self.dom.set_value(ids::FIELD_PROP_XEL, js_num(round2(x0)));
        self.dom.set_value(ids::FIELD_PROP_YEL, js_num(round2(y0)));
        self.dom
            .set_value(ids::FIELD_PROP_WEL, js_num(round2(x1 - x0)));
        self.dom
            .set_value(ids::FIELD_PROP_HEL, js_num(round2(y1 - y0)));
        self.dom.set_value(
            ids::FIELD_PROP_MAX_LEN_EL,
            item.max_len.unwrap_or(0).to_string(),
        );
        self.dom
            .set_checked(ids::FIELD_PROP_MULTILINE_EL, item.multiline == Some(true));
        self.dom.set_value(
            ids::FIELD_PROP_OPTIONS_EL,
            item.options.clone().unwrap_or_default().join("\n"),
        );
    }

    /// R:7694: the one refusal path every designer action shares: announce, keep the panel truthful.
    pub fn refuse_designer(&mut self, action: &str, name: &str, err: &Refusal) {
        let detail = refusal_detail(err);
        self.field_design = Some(FieldDesign {
            action: action.into(),
            name: name.into(),
            detail: Some(detail.clone()),
        });
        let text = detail_text(&detail, self.message_of(err));
        self.dom.set_text(ids::FIELDS_STATUS_EL, text.clone());
        self.show_error(&text);
        self.refresh_fields();
    }

    /// R:7703: `field-add` from the Add group: one op, one undo step.
    /// (The TS then focused the new row.)
    pub fn add_designed_field(&mut self) {
        if self.doc.is_none() || !self.can_design() {
            return;
        }
        let kind = self.dom.value(ids::FIELD_NEW_KIND_EL);
        let name = self.dom.value(ids::FIELD_NEW_NAME_EL);
        let mut payload = Map::new();
        payload.insert("op".into(), json!("field-add"));
        payload.insert("page".into(), json!(self.wanted_page));
        payload.insert("kind".into(), json!(kind));
        if !(kind == "signature" && name.is_empty()) {
            payload.insert("name".into(), json!(name));
        }
        if kind == "radio" {
            let x = number_or(&self.dom.value(ids::FIELD_NEW_XEL), 72.0);
            let y = number_or(&self.dom.value(ids::FIELD_NEW_YEL), 72.0);
            let kids: Vec<Value> = textarea_lines(&self.dom.value(ids::FIELD_NEW_KIDS_EL))
                .into_iter()
                .enumerate()
                .map(|(index, on)| {
                    let dy = index as f64 * 20.0;
                    json!({"on": on, "rect": nums(&[x, y + dy, x + 16.0, y + 16.0 + dy])})
                })
                .collect();
            payload.insert("kids".into(), Value::Array(kids));
        } else {
            let rect = self.designer_rect([
                ids::FIELD_NEW_XEL,
                ids::FIELD_NEW_YEL,
                ids::FIELD_NEW_WEL,
                ids::FIELD_NEW_HEL,
            ]);
            payload.insert("rect".into(), nums(&rect));
            if kind == "combo" {
                payload.insert(
                    "options".into(),
                    json!(textarea_lines(&self.dom.value(ids::FIELD_NEW_OPTIONS_EL))),
                );
            }
            if kind == "text" {
                let max_len = number_or(&self.dom.value(ids::FIELD_NEW_MAX_LEN_EL), 0.0);
                if max_len > 0.0 {
                    payload.insert("maxLen".into(), js_number(max_len));
                }
                if self.dom.checked(ids::FIELD_NEW_MULTILINE_EL) {
                    payload.insert("multiline".into(), json!(true));
                }
            }
        }
        // U7: an empty label or default is "no label", not an empty one.
        let label = self.dom.value(ids::FIELD_NEW_LABEL_EL);
        if !label.is_empty() {
            payload.insert("label".into(), json!(label));
        }
        if self.dom.checked(ids::FIELD_NEW_READ_ONLY_EL) {
            payload.insert("readOnly".into(), json!(true));
        }
        if self.dom.checked(ids::FIELD_NEW_REQUIRED_EL) {
            payload.insert("required".into(), json!(true));
        }
        if kind == "checkbox" {
            if self.dom.checked(ids::FIELD_NEW_DEFAULT_BOOL_EL) {
                payload.insert("default".into(), json!(true));
            }
        } else {
            let default = self.dom.value(ids::FIELD_NEW_DEFAULT_TEXT_EL);
            if kind != "signature" && !default.is_empty() {
                payload.insert("default".into(), json!(default));
            }
        }
        match self.request(Value::Object(payload), vec![]) {
            Ok(reply) => {
                let used = reply
                    .get("name")
                    .and_then(Value::as_str)
                    .map_or(name, str::to_string);
                self.field_design = Some(FieldDesign {
                    action: "add".into(),
                    name: used.clone(),
                    detail: None,
                });
                self.dom
                    .set_text(ids::FIELDS_STATUS_EL, format!("Added {kind} field {used}"));
                self.clear_error(); // debt1 row 15: an add that lands drops an earlier refusal's #error
                self.apply_edit(&journal_of(&reply), "");
            }
            Err(err) => self.refuse_designer("add", &name, &err),
        }
    }

    /// R:7745: `field-props` for the selected field: only the inputs its kind owns travel.
    pub fn apply_designed_field(&mut self) {
        let Some(item) = self.selected_field() else {
            return;
        };
        if self.doc.is_none()
            || !self.can_design()
            || item.scripted
            || (item.kind == FieldKind::Signature && self.sig_signed(&item))
        {
            return;
        }
        let mut payload = Map::new();
        payload.insert("op".into(), json!("field-props"));
        payload.insert("page".into(), json!(self.wanted_page));
        payload.insert("widgetId".into(), json!(item.id));
        // §9.1: a signature field keeps its name, so none travels.
        let name =
            (item.kind != FieldKind::Signature).then(|| self.dom.value(ids::FIELD_PROP_NAME_EL));
        if let Some(name) = &name {
            payload.insert("name".into(), json!(name));
        }
        let rect = self.designer_rect([
            ids::FIELD_PROP_XEL,
            ids::FIELD_PROP_YEL,
            ids::FIELD_PROP_WEL,
            ids::FIELD_PROP_HEL,
        ]);
        payload.insert("rect".into(), nums(&rect));
        let mut text_options = false;
        if item.kind == FieldKind::Text {
            payload.insert(
                "maxLen".into(),
                js_number(number_or(&self.dom.value(ids::FIELD_PROP_MAX_LEN_EL), 0.0)),
            );
            payload.insert(
                "multiline".into(),
                json!(self.dom.checked(ids::FIELD_PROP_MULTILINE_EL)),
            );
            text_options = true;
        }
        let combo_options = item.kind == FieldKind::Combo;
        if combo_options {
            payload.insert(
                "options".into(),
                json!(textarea_lines(&self.dom.value(ids::FIELD_PROP_OPTIONS_EL))),
            );
        }
        // U7: only what the user actually changed travels.
        let label = self.dom.value(ids::FIELD_PROP_LABEL_EL);
        let label_changed = label != item.tu.clone().unwrap_or_default();
        if label_changed {
            payload.insert("label".into(), json!(label));
        }
        let read_only = self.dom.checked(ids::FIELD_PROP_READ_ONLY_EL);
        if read_only != item.read_only {
            payload.insert("readOnly".into(), json!(read_only));
        }
        let required = self.dom.checked(ids::FIELD_PROP_REQUIRED_EL);
        if required != item.required {
            payload.insert("required".into(), json!(required));
        }
        if item.kind == FieldKind::Checkbox {
            let default = self.dom.checked(ids::FIELD_PROP_DEFAULT_BOOL_EL);
            if default != (item.default == Some(FieldValue::Bool(true))) {
                payload.insert("default".into(), json!(default));
            }
        } else if item.kind != FieldKind::Signature {
            let default = self.dom.value(ids::FIELD_PROP_DEFAULT_TEXT_EL);
            if default
                != item
                    .default
                    .as_ref()
                    .and_then(|d| d.as_text())
                    .unwrap_or("")
            {
                payload.insert("default".into(), json!(default));
            }
        }
        let touched_label = payload.contains_key("label");
        let touched_read_only = payload.contains_key("readOnly");
        let touched_required = payload.contains_key("required");
        let touched_default = payload.contains_key("default");
        match self.request(Value::Object(payload), vec![]) {
            Ok(reply) => {
                let renamed = name.as_ref().is_some_and(|name| *name != item.name);
                // The status names what the user actually changed; the rect travels on every
                // apply, so it is only listed when the box really moved (U7 review follow-up 2).
                let was = if item.rect.len() == 4 {
                    item.rect.clone()
                } else {
                    vec![0.0; 4]
                };
                let moved = rect
                    .iter()
                    .zip(&was)
                    .any(|(value, was)| (value - was).abs() > 0.01);
                let mut touched: Vec<&str> = Vec::new();
                if moved {
                    touched.push("position");
                }
                if touched_label {
                    touched.push("label");
                }
                if touched_read_only {
                    touched.push("read-only");
                }
                if touched_required {
                    touched.push("required");
                }
                if touched_default {
                    touched.push("default");
                }
                if text_options {
                    touched.push("text options");
                }
                if combo_options {
                    touched.push("options");
                }
                let used = name.clone().unwrap_or_else(|| item.name.clone());
                self.field_design = Some(FieldDesign {
                    action: if renamed { "rename" } else { "props" }.into(),
                    name: used.clone(),
                    detail: None,
                });
                let moved_only = touched.is_empty() || touched == ["position"];
                let status = if renamed {
                    format!("Renamed {} to {used}", item.name)
                } else if moved_only {
                    format!("Moved {}", item.name)
                } else {
                    format!("Updated {}: {}", item.name, touched.join(", "))
                };
                self.dom.set_text(ids::FIELDS_STATUS_EL, status);
                self.apply_edit(&journal_of(&reply), "");
            }
            Err(err) => self.refuse_designer("props", &item.name, &err),
        }
    }

    /// R:7796: `field-delete` for the selected field (the TS then focused the select).
    pub fn delete_designed_field(&mut self) {
        let Some(item) = self.selected_field() else {
            return;
        };
        if self.doc.is_none()
            || !self.can_design()
            || item.scripted
            || (item.kind == FieldKind::Signature && self.sig_signed(&item))
        {
            return;
        }
        let request = json!({"op": "field-delete", "page": self.wanted_page, "widgetId": item.id});
        match self.request(request, vec![]) {
            Ok(reply) => {
                self.field_design = Some(FieldDesign {
                    action: "delete".into(),
                    name: item.name.clone(),
                    detail: None,
                });
                self.dom
                    .set_text(ids::FIELDS_STATUS_EL, format!("Deleted {}", item.name));
                self.apply_edit(&journal_of(&reply), "");
            }
            Err(err) => self.refuse_designer("delete", &item.name, &err),
        }
    }

    /// R:8165: P5 U3 §2.4/§9.14/§9.21: a drawn rect becomes one empty signature field (Forms
    /// mode, designer rights).
    pub fn add_signature_field(&mut self, page: usize, rect: Vec<f64>) {
        if self.doc.is_none() || !self.can_design() || self.ui_mode != UiMode::Forms {
            self.field_design = Some(FieldDesign {
                action: "add".into(),
                name: String::new(),
                detail: Some("invalid".into()),
            });
            return;
        }
        if rect.len() < 4 || rect[2] - rect[0] < 40.0 || rect[3] - rect[1] < 20.0 {
            self.show_error("Draw a bigger signature field");
            return;
        }
        let request =
            json!({"op": "field-add", "page": page, "kind": "signature", "rect": nums(&rect)});
        match self.request(request, vec![]) {
            Ok(reply) => {
                let name = reply
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.field_design = Some(FieldDesign {
                    action: "add".into(),
                    name: name.clone(),
                    detail: None,
                });
                self.dom.set_text(
                    ids::FIELDS_STATUS_EL,
                    format!("Added signature field {name}"),
                );
                self.clear_error();
                self.apply_edit(&journal_of(&reply), "");
            }
            Err(err) => {
                let detail = refusal_detail(&err);
                self.field_design = Some(FieldDesign {
                    action: "add".into(),
                    name: String::new(),
                    detail: Some(detail.clone()),
                });
                let text = detail_text(&detail, self.message_of(&err));
                self.show_error(&text);
                self.refresh_fields();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_like_js() {
        assert_eq!(js_to_number(""), 0.0);
        assert_eq!(js_to_number(" 12 "), 12.0);
        assert_eq!(js_to_number("1e3"), 1000.0);
        assert_eq!(js_to_number("-2.5"), -2.5);
        assert_eq!(js_to_number("0x10"), 16.0);
        assert!(js_to_number("12px").is_nan());
        assert!(js_to_number("+-1").is_nan());
        assert!(js_to_number("inf").is_nan());
        assert_eq!(js_to_number("-Infinity"), f64::NEG_INFINITY);
        assert_eq!(number_or("", 72.0), 72.0);
        assert_eq!(number_or("abc", 72.0), 72.0);
        assert_eq!(number_or("5", 72.0), 5.0);
    }

    #[test]
    fn designer_rect_falls_back_on_junk_only() {
        let mut session = Session::new();
        session.dom.set_value(ids::FIELD_NEW_XEL, "");
        session.dom.set_value(ids::FIELD_NEW_YEL, "abc");
        session.dom.set_value(ids::FIELD_NEW_WEL, "100");
        session.dom.set_value(ids::FIELD_NEW_HEL, "Infinity");
        let rect = session.designer_rect([
            ids::FIELD_NEW_XEL,
            ids::FIELD_NEW_YEL,
            ids::FIELD_NEW_WEL,
            ids::FIELD_NEW_HEL,
        ]);
        assert_eq!(rect, vec![0.0, 72.0, 100.0, 96.0]);
    }

    #[test]
    fn lines_and_rounding() {
        assert_eq!(textarea_lines(" a \n\n b\n"), vec!["a", "b"]);
        assert_eq!(round2(1.005), 1.0);
        assert_eq!(round2(2.346), 2.35);
        assert_eq!(round3(0.12345), 0.123);
        let err = Refusal {
            reply: json!({"ok": false, "detail": "xfa"}),
            ..Refusal::default()
        };
        assert_eq!(refusal_detail(&err), "xfa");
        assert_eq!(refusal_detail(&Refusal::plain("x")), "invalid");
    }
}
