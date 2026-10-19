//! Page organisation (renderer.ts R:7045-7137: the page ops and their mirror), the form
//! smokes and the U6 designer drive (R:7139-7250), and the annotation list's refresh and
//! names (R:7252-7286).
#![allow(dead_code)]

use super::Session;
use super::consts::{ANNOT_LABELS, FIELD_DETAIL_TEXT, lookup};
use super::content::truthy;
use super::modes::journal_of;
use super::thumbs::cache_clear;
use super::types::{AnnotItem, FieldDesign, FieldItem, FieldValue};
use serde_json::{Map, Value, json};
use tandem_protocol::engine::{AnnotsReply, FieldKind, PageReply};
use tandem_protocol::limits::SIGNATURE_SUBJECT;

/// R:7282: a signature appearance is a Stamp by type; its `/Subj` is what tells the two apart.
pub fn annot_name(item: &AnnotItem) -> Option<&'static str> {
    if item.r#type == "Stamp" && item.subject == SIGNATURE_SUBJECT {
        return Some("Signature appearance");
    }
    lookup(&ANNOT_LABELS, &item.r#type)
}

impl Session {
    /// R:7052: a page op, or an undo/redo of one: the reply is the only source of the new page
    /// list, so the thumbnails, the render caches, the now-stale search hits and the
    /// annotation list are all rebuilt from it before the page is chosen.
    pub fn apply_page_edit(&mut self, reply: &PageReply, announcement: &str, page: usize) {
        self.revision = reply.revision;
        self.dirty = reply.dirty;
        self.can_undo = reply.can_undo;
        self.can_redo = reply.can_redo;
        if let Some(doc) = &mut self.doc {
            doc.page_count = reply.page_count;
            doc.pages = reply.pages.clone();
            doc.rotations = reply.rotations.clone();
        }
        self.sync_dirty();
        cache_clear(&mut self.cache); // every cached render is keyed by a page index that just moved
        self.clear_search();
        // Page indices behind `navLinks` just moved; only the shown page is reloaded below.
        self.nav_links.clear();
        let sidebar_was_open = self.thumb_open;
        self.refresh_thumbs(); // rebuilds the rows from the new sizes and drops the thumbnail cache
        // The sidebar keeps whatever state the user left it in; a page op must not pop it open.
        self.set_thumb_sidebar(sidebar_was_open);
        if !announcement.is_empty() {
            self.announce(announcement);
        }
        self.go_to(page as i64); // clamps to the new page count, refreshes the list and repaints
        self.pump_thumbs();
        // A delete repairs the outline, so the panels follow the reply like any other edit.
        self.refresh_outline();
        self.refresh_links();
        if self.compare_doc.is_some() {
            self.refresh_compare(false, false);
        }
    }

    /// One page op's request and mirror; a refusal is shown as the engine worded it.
    fn page_request(
        &mut self,
        request: Value,
        announcement: impl FnOnce(&PageReply) -> (String, usize),
    ) {
        match self.request(request, Vec::new()) {
            Ok(reply) => {
                let reply: PageReply = super::parse_reply_or_default(&reply.value);
                let (text, page) = announcement(&reply);
                self.apply_page_edit(&reply, &text, page);
            }
            Err(err) => self.show_error(&err.message),
        }
    }

    /// R:7081: "Rotate right" / "Rotate left" on one page; the page keeps its index.
    pub fn rotate_page(&mut self, page: usize, by: i64) {
        self.page_request(
            json!({ "op": "page-rotate", "page": page, "by": by }),
            |_| (format!("Rotated page {}", page + 1), page),
        );
    }

    /// R:7091: "Move up" / "Move down": the current page follows to its new index (`to`). The
    /// ends are a dead end, not an error: the button and the menu item are disabled there.
    pub fn move_page(&mut self, page: usize, to: i64) {
        let Some(count) = self.doc.as_ref().map(|d| d.page_count) else {
            return;
        };
        if to < 0 || to >= count {
            return;
        }
        let direction = if to < page as i64 { "up" } else { "down" };
        self.page_request(json!({ "op": "page-move", "page": page, "to": to }), |_| {
            (format!("Moved page {} {direction}", page + 1), to as usize)
        });
    }

    /// R:7103: "Delete page": one Undo brings it back, which is what the status line says.
    pub fn delete_page(&mut self, page: usize) {
        if self.doc.as_ref().is_none_or(|d| d.page_count < 2) {
            return;
        }
        self.page_request(json!({ "op": "page-delete", "page": page }), |reply| {
            let last = (reply.page_count - 1).max(0) as usize;
            (
                format!("Deleted page {} — Undo restores it", page + 1),
                page.min(last),
            )
        });
    }

    /// R:7114: one path for the toolbar and the Page menu; both act on the current page.
    pub fn run_page_op(&mut self, op: &str) {
        let page = self.wanted_page.max(0) as usize;
        match op {
            "rotate-right" => self.rotate_page(page, 90),
            "rotate-left" => self.rotate_page(page, -90),
            "move-up" => self.move_page(page, page as i64 - 1),
            "move-down" => self.move_page(page, page as i64 + 1),
            "delete-page" => self.delete_page(page),
            _ => {}
        }
    }

    /// R:7124: `--edit=pages`: rotate page 1, move page 3 up twice, delete the last page, then log.
    pub fn smoke_pages(&mut self) {
        let Some(count) = self.doc.as_ref().map(|d| d.page_count) else {
            self.log_edit();
            return;
        };
        let _ = count;
        self.go_to(0);
        self.rotate_page(0, 90);
        self.go_to(2);
        let page = self.wanted_page.max(0) as usize;
        self.move_page(page, page as i64 - 1);
        let page = self.wanted_page.max(0) as usize;
        self.move_page(page, page as i64 - 1);
        let last = self.doc.as_ref().map_or(0, |d| d.page_count - 1).max(0) as usize;
        self.go_to(last as i64);
        self.delete_page(self.wanted_page.max(0) as usize);
        self.log_edit();
    }

    /// R:7140: `--edit=form`: fill name, check agree, pick combo two, radio M, then log. No
    /// script runs.
    pub fn smoke_form(&mut self, contents: Option<&str>) {
        if self.doc.is_none() {
            self.log_edit();
            return;
        }
        self.refresh_fields();
        let text = contents.unwrap_or("PDF4All P2 form").to_string();
        let page = self.wanted_page.max(0) as usize;
        let target = |fields: &[FieldItem], name: &str| {
            fields.iter().find(|item| item.name == name).cloned()
        };
        if let Some(field) = target(&self.fields, "name") {
            self.commit_field(&field, FieldValue::Text(text), page);
        }
        if let Some(field) = target(&self.fields, "agree") {
            self.commit_field(&field, FieldValue::Bool(true), page);
        }
        if let Some(field) = target(&self.fields, "choice") {
            self.commit_field(&field, FieldValue::Text("two".into()), page);
        }
        let radio = self
            .fields
            .iter()
            .find(|item| {
                item.kind == FieldKind::Radio
                    && item.name == "size"
                    && item.on.as_deref() == Some("M")
            })
            .cloned();
        if let Some(field) = radio {
            self.commit_field(&field, FieldValue::Bool(true), page);
        }
        self.log_edit();
    }

    /// R:7171: the U6 drive path (`{"cmd":"fields","contents":{…}}`, contract §3): one
    /// designer action per line, every target resolved by name against the current `fields`
    /// list. A refusal lands in `P2_EDIT.fields.last.detail` and in the error line.
    pub fn smoke_field_design(&mut self, contents: Option<&str>) {
        self.refresh_fields();
        let mut wanted = Map::new();
        if let Some(text) = contents.filter(|c| crate::js::js_trim(c).starts_with('{')) {
            match serde_json::from_str::<Value>(text) {
                Ok(Value::Object(map)) => wanted = map,
                Ok(_) => {}
                Err(err) => {
                    self.show_error(&format!("fields: {err}"));
                    return;
                }
            }
        }
        let action = wanted
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let name = wanted
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let on = wanted.get("on").cloned();
        let page = self.wanted_page;
        // `if (wanted.x)` copies a truthy value; `!== undefined` copies anything given.
        let copy_truthy = |payload: &mut Map<String, Value>, key: &str| {
            if let Some(v) = wanted.get(key).filter(|v| truthy(v)) {
                payload.insert(key.into(), v.clone());
            }
        };
        let copy_given = |payload: &mut Map<String, Value>, key: &str| {
            if let Some(v) = wanted.get(key) {
                payload.insert(key.into(), v.clone());
            }
        };
        let find = |fields: &[FieldItem], on: &Option<Value>| {
            fields
                .iter()
                .find(|item| {
                    item.name == name
                        && on.as_ref().is_none_or(|o| {
                            o.as_str().is_some_and(|o| item.on.as_deref() == Some(o))
                        })
                })
                .cloned()
        };
        let result: Result<Value, crate::engine::Refusal> = (|| {
            match action.as_str() {
                "add" => {
                    let mut payload = Map::new();
                    payload.insert("op".into(), json!("field-add"));
                    payload.insert("page".into(), json!(page));
                    copy_given(&mut payload, "kind");
                    payload.insert("name".into(), json!(name));
                    // P5 U3 §1.2: a signature field's name is optional (the engine picks Signature<n>).
                    if wanted.get("kind").and_then(Value::as_str) == Some("signature")
                        && name.is_empty()
                    {
                        payload.remove("name");
                    }
                    for key in ["rect", "kids", "options"] {
                        copy_truthy(&mut payload, key);
                    }
                    for key in [
                        "edit",
                        "maxLen",
                        "multiline",
                        "label",
                        "readOnly",
                        "required",
                        "default",
                    ] {
                        copy_given(&mut payload, key);
                    }
                    self.field_design = Some(FieldDesign {
                        action: action.clone(),
                        name: name.clone(),
                        detail: None,
                    });
                    let reply = self.request(Value::Object(payload), Vec::new())?;
                    // §9.13: the name the engine used (a signature field may have been given none).
                    if let Some(used) = reply.get("name").and_then(Value::as_str) {
                        self.field_design = Some(FieldDesign {
                            action: action.clone(),
                            name: used.to_string(),
                            detail: None,
                        });
                    }
                    self.clear_error(); // debt1 row 15: an add that lands drops an earlier refusal's #error
                    Ok(reply.value)
                }
                "props" => {
                    let item = find(&self.fields, &on).ok_or_else(|| {
                        crate::engine::Refusal::plain(format!(
                            "fields: no field {name} on this page"
                        ))
                    })?;
                    let mut payload = Map::new();
                    payload.insert("op".into(), json!("field-props"));
                    payload.insert("page".into(), json!(page));
                    payload.insert("widgetId".into(), json!(item.id));
                    let rename = wanted
                        .get("rename")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    if let Some(rename) = &rename {
                        payload.insert("name".into(), json!(rename));
                    }
                    for key in ["rect", "options"] {
                        copy_truthy(&mut payload, key);
                    }
                    for key in [
                        "edit",
                        "maxLen",
                        "multiline",
                        "label",
                        "readOnly",
                        "required",
                        "default",
                    ] {
                        copy_given(&mut payload, key);
                    }
                    self.field_design = Some(match rename {
                        Some(rename) => FieldDesign {
                            action: "rename".into(),
                            name: rename,
                            detail: None,
                        },
                        None => FieldDesign {
                            action: "props".into(),
                            name: name.clone(),
                            detail: None,
                        },
                    });
                    Ok(self.request(Value::Object(payload), Vec::new())?.value)
                }
                "delete" => {
                    let item = find(&self.fields, &on).ok_or_else(|| {
                        crate::engine::Refusal::plain(format!(
                            "fields: no field {name} on this page"
                        ))
                    })?;
                    self.field_design = Some(FieldDesign {
                        action: action.clone(),
                        name: name.clone(),
                        detail: None,
                    });
                    let request =
                        json!({ "op": "field-delete", "page": page, "widgetId": item.id });
                    Ok(self.request(request, Vec::new())?.value)
                }
                "fill" => {
                    // A radio's `value` names the kid whose export value is chosen; every other
                    // kind takes the value itself (§3).
                    let value = wanted.get("value").cloned();
                    let matches_on = |item: &FieldItem, o: &Option<Value>| match o {
                        None => item.on.is_none(),
                        Some(o) => o.as_str().is_some_and(|o| item.on.as_deref() == Some(o)),
                    };
                    let item = self
                        .fields
                        .iter()
                        .find(|entry| {
                            entry.name == name
                                && if entry.kind == FieldKind::Radio {
                                    match value.as_ref().and_then(Value::as_str) {
                                        Some(v) => entry.on.as_deref() == Some(v),
                                        None => matches_on(entry, &on),
                                    }
                                } else {
                                    on.is_none() || matches_on(entry, &on)
                                }
                        })
                        .cloned()
                        .ok_or_else(|| {
                            crate::engine::Refusal::plain(format!(
                                "fields: no field {name} on this page"
                            ))
                        })?;
                    self.field_design = Some(FieldDesign {
                        action: action.clone(),
                        name: name.clone(),
                        detail: None,
                    });
                    let mut payload = Map::new();
                    payload.insert("op".into(), json!("field-set"));
                    payload.insert("page".into(), json!(page));
                    payload.insert("widgetId".into(), json!(item.id));
                    if item.kind == FieldKind::Radio {
                        payload.insert("value".into(), json!(true));
                    } else if let Some(value) = value {
                        payload.insert("value".into(), value);
                    }
                    let reply = self.request(Value::Object(payload), Vec::new())?;
                    self.note_filled();
                    Ok(reply.value)
                }
                _ => Err(crate::engine::Refusal::plain(format!(
                    "fields: unknown action {}",
                    super::content::json_string(&action)
                ))),
            }
        })();
        match result {
            Ok(reply) => self.apply_edit(&journal_of(&reply), ""),
            Err(err) => {
                let detail = err.detail.clone().unwrap_or_else(|| "invalid".into());
                self.field_design = Some(FieldDesign {
                    action,
                    name,
                    detail: Some(detail.clone()),
                });
                let text = lookup(&FIELD_DETAIL_TEXT, &detail)
                    .map(str::to_string)
                    .unwrap_or(err.message);
                self.show_error(&text);
                self.refresh_fields();
            }
        }
    }

    /// R:7253: the page list is the semantic path to a sticky note; nothing depends on the canvas.
    pub fn refresh_notes(&mut self) {
        self.notes_seq += 1;
        let seq = self.notes_seq;
        if self.doc.as_ref().is_none_or(|d| d.info.needs_password) {
            self.notes.clear();
            self.render_notes();
            return;
        }
        let page = self.wanted_page;
        let started_generation = self.generation;
        let Ok(reply) = self.request(json!({ "op": "annots", "page": page }), Vec::new()) else {
            // A document that changed under us owns the list now; a real failure is not worth an alert.
            return;
        };
        if seq != self.notes_seq
            || started_generation != self.generation
            || self.wanted_page != page
        {
            return;
        }
        let reply: AnnotsReply = super::parse_reply_or_default(&reply.value);
        self.notes = reply.items;
        self.render_notes();
        self.repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotation_names() {
        let stamp = AnnotItem {
            r#type: "Stamp".into(),
            subject: SIGNATURE_SUBJECT.into(),
            ..AnnotItem::default()
        };
        assert_eq!(annot_name(&stamp), Some("Signature appearance"));
        let plain = AnnotItem {
            r#type: "Stamp".into(),
            ..AnnotItem::default()
        };
        assert_eq!(annot_name(&plain), Some("Stamp"));
        let redact = AnnotItem {
            r#type: "Redact".into(),
            ..AnnotItem::default()
        };
        assert_eq!(annot_name(&redact), Some("Redaction mark"));
        let other = AnnotItem {
            r#type: "Popup".into(),
            ..AnnotItem::default()
        };
        assert_eq!(annot_name(&other), None);
    }
}
