//! P6 U7 clipboard (R:3838-3991): the worker clipboard's Cut, Copy and Paste of the page
//! selection, their announcements and refusals.
#![allow(dead_code)]

use super::Session;
use super::consts::{
    CLIP_KIND_ORDER, CLIP_KIND_WORD, CLIPBOARD_DETAIL_TEXT, CONTENT_DETAIL_TEXT, MODE_LABEL, lookup,
};
use super::content::ObjectEntry;
use super::hit::link_annot_id;
use super::members::{
    field_member, is_object_member, link_member, mark_member, member_mode, text_member,
};
use super::nav::{pdf_box, view_box};
use super::types::*;
use crate::engine::Refusal;
use crate::js::js_num;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use tandem_protocol::engine::{EditTargetKind, FieldKind};

/// What `clipboardChordOk` reads of `document.activeElement`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActiveElement {
    pub tag: String,
    /// An `<input>`'s `type`.
    pub kind: Option<String>,
    pub content_editable: bool,
    pub inline_editor: bool,
}

/// R:3875 `clipboardChordOk`'s context.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChordContext {
    pub mode: UiMode,
    pub layer_live: bool,
    pub dialog_open: bool,
    pub menu_open: bool,
    pub editor_open: bool,
    pub active: Option<ActiveElement>,
    pub collapsed: bool,
    pub selection: usize,
    pub clipboard: bool,
    pub editable: bool,
}

/// R:3875 `clipboardChordOk()`: whether a DOM copy/cut/paste event (Edit-menu role, chord or
/// drive) belongs to the page selection. Pure.
pub fn clipboard_chord_ok(kind: &str, ctx: &ChordContext) -> bool {
    if !matches!(
        ctx.mode,
        UiMode::Edit | UiMode::Forms | UiMode::Annotate | UiMode::Redact
    ) || !ctx.layer_live
        || ctx.dialog_open
        || ctx.menu_open
        || ctx.editor_open
    {
        return false;
    }
    if let Some(active) = &ctx.active {
        let tag = active.tag.to_uppercase();
        let input_type = active
            .kind
            .clone()
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "text".into())
            .to_lowercase();
        let textual_input = tag == "INPUT"
            && ![
                "radio", "checkbox", "button", "submit", "range", "color", "file",
            ]
            .contains(&input_type.as_str());
        if tag == "TEXTAREA"
            || tag == "SELECT"
            || active.content_editable
            || active.inline_editor
            || textual_input
        {
            return false;
        }
    }
    if kind == "paste" {
        ctx.clipboard && ctx.editable
    } else {
        ctx.collapsed && ctx.selection > 0
    }
}

fn plural(n: i64, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// R:3931 `cutAnnouncement()`: the Cut announcement of a selection whose text runs stay.
pub fn cut_announcement(items: i64, runs: i64) -> String {
    let stays = if runs == 0 {
        String::new()
    } else {
        format!(
            "; {} {} be deleted)",
            plural(runs, "text run"),
            if runs == 1 {
                "stays (it cannot"
            } else {
                "stay (they cannot"
            }
        )
    };
    format!("Cut {}{stays}", plural(items, "item"))
}

/// R:3936 `pasteAnnouncement()` (exact; pinned at the hello paste).
pub fn paste_announcement(
    kinds: &BTreeMap<String, i64>,
    count: i64,
    skipped: i64,
    mode: UiMode,
    names: &[String],
) -> String {
    let n_of = |kind: &str| kinds.get(kind).copied().unwrap_or(0);
    let objects_only = CLIP_KIND_ORDER
        .iter()
        .all(|k| *k == "image" || *k == "form" || n_of(k) == 0)
        && skipped == 0;
    if objects_only && mode == UiMode::Edit {
        return if count == 1 && names.len() == 1 {
            format!(
                "Pasted {} {}",
                if n_of("form") > 0 { "form" } else { "image" },
                names[0]
            )
        } else {
            format!("Pasted {count} objects")
        };
    }
    let mut text = format!("Pasted {}", plural(count, "item"));
    for kind in CLIP_KIND_ORDER {
        let n = n_of(kind) - if kind == "link" { skipped } else { 0 };
        let home = member_mode(kind);
        if n > 0 && home != mode {
            let word = lookup(&CLIP_KIND_WORD, kind).unwrap_or(kind);
            let label = lookup(&MODE_LABEL, home.as_str()).unwrap_or("");
            text += &format!("; {} shown in {label}", plural(n, word));
        }
    }
    if skipped > 0 {
        text += &format!(
            "; {} skipped (its page is not in this document)",
            plural(skipped, "link")
        );
    }
    text
}

/// `String(id)` of a reply's number or string id.
fn id_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => n.as_f64().map(js_num),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

impl Session {
    /// R:3858 `refusedKind()`: the kind a refusal names.
    fn refused_kind(&self, reply: &Value) -> Option<String> {
        let raw = reply.as_object()?;
        if let Some(Value::String(kind)) = raw.get("kind") {
            return Some(kind.clone());
        }
        if raw.contains_key("widgetId") {
            return Some("field".into());
        }
        let id = raw.get("targetId").or_else(|| raw.get("annotId"))?;
        let id = id_text(id)?;
        self.direct_selection
            .iter()
            .find(|m| m.target_id == id)
            .map(|m| m.kind.clone())
    }

    /// R:3865 `showClipRefusal()`: clipboard refusals in words, per kind first;
    /// `content.refused[].detail` stays the bare token.
    pub fn show_clip_refusal(&mut self, err: &Refusal, name: &str) {
        let raw = &err.reply;
        let detail = match raw.get("detail") {
            Some(Value::String(d)) => d.clone(),
            _ if err.reason_str() == "read-only" => "read-only".into(),
            _ => "invalid-selection".into(),
        };
        let kind = self.refused_kind(raw).unwrap_or_else(|| "undefined".into());
        let text = lookup(&CLIPBOARD_DETAIL_TEXT, &format!("{detail}:{kind}"))
            .or_else(|| lookup(&CLIPBOARD_DETAIL_TEXT, &detail))
            .or_else(|| lookup(&CONTENT_DETAIL_TEXT, &detail))
            .map(str::to_string)
            .unwrap_or_else(|| match raw.get("error") {
                Some(Value::String(e)) => e.clone(),
                _ => err.message.clone(),
            });
        self.content_refusals.push(ContentRefusal {
            text: name.to_string(),
            detail,
        });
        self.announce(&text);
        self.show_error(&text);
    }

    /// R:3887 `clipboardWork()`: Cut, Copy and Paste (menu, chords, page menu) as gesture work.
    pub fn clipboard_work(
        &mut self,
        which: &str,
        point: Option<Vec<f64>>,
    ) -> Result<bool, Refusal> {
        if which == "paste" {
            return self.paste_clipboard(point);
        }
        let members = self.direct_selection.clone();
        let count = members.len() as i64;
        if count == 0 || !self.copy_selection(&members) {
            return Ok(false);
        }
        if which == "copy" {
            let text = if members.iter().all(is_object_member) {
                format!("Copied {}", plural(count, "object"))
            } else {
                format!("Copied {}", plural(count, "item"))
            };
            self.announce(&text);
            return Ok(true);
        }
        Ok(self.cut_members(members))
    }

    /// R:3895 `copySelection()`: one `clip-copy` over the whole selection; nothing is written.
    pub fn copy_selection(&mut self, members: &[ObjectFocus]) -> bool {
        let scan = self.content_scan.clone();
        let content = members
            .iter()
            .any(|m| is_object_member(m) || m.kind == "text");
        if self.doc.is_none()
            || (content && scan.as_ref().is_none_or(|s| s.page != self.wanted_page))
        {
            return false;
        }
        let mut message = Map::new();
        message.insert("op".into(), json!("clip-copy"));
        message.insert("page".into(), json!(self.wanted_page));
        if let Some(s) = scan.as_ref().filter(|_| content) {
            message.insert("generation".into(), json!(s.generation));
            message.insert("revision".into(), json!(s.revision));
        }
        let targets: Vec<Value> = members
            .iter()
            .map(|m| {
                let id = if is_object_member(m) || m.kind == "text" {
                    json!(m.target_id)
                } else {
                    number_of(&m.target_id)
                };
                json!({"kind": m.kind, "id": id})
            })
            .collect();
        message.insert("targets".into(), Value::Array(targets));
        match self.rq(Value::Object(message)) {
            Ok(reply) => {
                let count = reply.get("count").and_then(Value::as_i64).unwrap_or(0);
                let kinds: BTreeMap<String, i64> = reply
                    .get("kinds")
                    .and_then(|k| serde_json::from_value(k.clone()).ok())
                    .unwrap_or_default();
                self.clipboard_info = Some(ClipboardInfo { count, kinds });
                true
            }
            Err(err) => {
                let name = members.last().map(|m| m.name.clone()).unwrap_or_default();
                self.show_clip_refusal(&err, &name);
                false
            }
        }
    }

    /// R:3908 `cutMembers()`: after the copy, one delete of every member but the runs; the
    /// selection is empty afterwards.
    pub fn cut_members(&mut self, members: Vec<ObjectFocus>) -> bool {
        let runs = members.iter().filter(|m| m.kind == "text").count() as i64;
        let rest: Vec<ObjectFocus> = members
            .iter()
            .filter(|m| m.kind != "text")
            .cloned()
            .collect();
        if rest.is_empty() {
            self.announce(&format!(
                "Copied {}; text runs cannot be cut",
                plural(runs, "text run")
            ));
            self.drop_direct_focus();
            return true;
        }
        let said = cut_announcement(rest.len() as i64, runs);
        if !rest.iter().all(is_object_member) {
            let ok = self.delete_members(rest, &said);
            if ok {
                self.drop_direct_focus();
            }
            return ok;
        }
        // §9.38: objects keep the U5 delete paths and `content.edited` shapes; only the announcement says Cut.
        let scan = self.content_scan.clone();
        let mut entries: Vec<EditTarget> = Vec::new();
        for member in &rest {
            let item = scan
                .as_ref()
                .filter(|s| s.page == self.wanted_page)
                .and_then(|s| s.items.iter().find(|v| v.target_id == member.target_id));
            if let Some(item) = item {
                entries.push(item.clone());
            }
        }
        let Some(scan) = scan else { return false };
        if entries.len() != rest.len() || !rest.iter().all(|m| self.member_deletable(m)) {
            return false;
        }
        let previous = self.direct_selection.clone();
        self.set_selection(Vec::new());
        let ok = if entries.len() == 1 {
            self.apply_object_edit(&scan, &entries[0], "delete", Map::new(), Vec::new())
        } else {
            self.apply_objects_edit(
                &scan,
                &entries
                    .iter()
                    .map(|i| ObjectEntry {
                        item: i.clone(),
                        r#box: None,
                    })
                    .collect::<Vec<_>>(),
                "delete",
                None,
            )
        };
        if !ok {
            self.set_selection(previous);
            self.restore_direct_focus(false);
            return false;
        }
        if runs > 0 {
            let name = rest.last().map(|m| m.name.clone()).unwrap_or_default();
            self.content_edited = Some(ContentEdited::Object {
                kind: "selection".into(),
                action: "delete".into(),
                name,
                count: Some(rest.len() as i64),
                skipped: None,
            });
            self.announce(&said);
        } else if entries.len() == 1 {
            let text = format!(
                "Cut {} {}",
                entries[0].kind.as_str(),
                entries[0].object_name.clone().unwrap_or_default()
            );
            self.announce(crate::js::js_trim(&text));
        } else {
            self.announce(&format!("Cut {} objects", entries.len()));
        }
        self.drop_direct_focus();
        true
    }

    /// R:3949 `pasteClipboard()`: one `clip-paste`; the pasted members of this mode's layer
    /// become the selection (anchor = the last).
    pub fn paste_clipboard(&mut self, point: Option<Vec<f64>>) -> Result<bool, Refusal> {
        let Some(info) = self.clipboard_info.clone() else {
            return Ok(false);
        };
        if self.doc.is_none() || !self.editable() {
            return Ok(false);
        }
        let page = self.wanted_page;
        let mut at: Option<Vec<f64>> = None;
        if let Some(point) = point {
            // §9.28: the view point through the inverse page transform (MediaBox origin and /Rotate) → PDF points.
            let current = self.content_scan.as_ref().is_some_and(|s| {
                s.page == page && s.transform.is_some() && s.doc_generation == Some(self.generation)
            });
            if !current {
                self.refresh_content_targets(true);
            }
            let m = self
                .content_scan
                .as_ref()
                .filter(|s| s.page == page)
                .and_then(|s| s.transform.clone());
            let Some(m) = m else { return Ok(false) };
            let bx = pdf_box(&[point[0], point[1], point[0], point[1]], &m)?;
            at = Some(vec![bx[0], bx[1]]);
        }
        let mut message = Map::new();
        message.insert("op".into(), json!("clip-paste"));
        message.insert("page".into(), json!(page));
        if let Some(s) = self.content_scan.as_ref().filter(|s| {
            s.page == page
                && s.doc_generation == Some(self.generation)
                && s.revision == self.revision
        }) {
            message.insert("generation".into(), json!(s.generation));
            message.insert("revision".into(), json!(s.revision));
        }
        if let Some(at) = at {
            message.insert("at".into(), json!(at));
        }
        let reply = match self.rq(Value::Object(message)) {
            Ok(reply) => reply,
            Err(err) => {
                self.show_clip_refusal(&err, "");
                return Ok(false);
            }
        };
        let list = |key: &str| {
            reply
                .get(key)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        let names: Vec<String> = list("names")
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        let target_ids: Vec<String> = list("targetIds")
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        let field_ids: Vec<i64> = list("fieldIds")
            .into_iter()
            .filter_map(|v| v.as_i64())
            .collect();
        let link_ids: Vec<i64> = list("linkIds")
            .into_iter()
            .filter_map(|v| v.as_i64())
            .collect();
        let mark_ids: Vec<i64> = list("markIds")
            .into_iter()
            .filter_map(|v| v.as_i64())
            .collect();
        let skipped = list("skipped").len() as i64;
        let count = reply
            .get("count")
            .and_then(Value::as_i64)
            .unwrap_or(names.len() as i64);
        let objects_only = CLIP_KIND_ORDER.iter().all(|k| {
            *k == "image" || *k == "form" || info.kinds.get(*k).copied().unwrap_or(0) == 0
        }) && skipped == 0;
        self.content_edited = Some(if objects_only {
            ContentEdited::Object {
                kind: "objects".into(),
                action: "paste".into(),
                name: names.join(","),
                count: Some(count),
                skipped: None,
            }
        } else {
            ContentEdited::Object {
                kind: "selection".into(),
                action: "paste".into(),
                name: String::new(),
                count: Some(count),
                skipped: Some(skipped),
            }
        });
        let announcement = paste_announcement(&info.kinds, count, skipped, self.ui_mode, &names);
        self.apply_edit(&super::modes::journal_of(&reply.value), &announcement);
        if self.ui_mode != UiMode::Edit {
            self.refresh_content_targets(true);
        }
        // Set only after both refreshes, so a repaint never sees the old selection (§10.34).
        let mut picks: Vec<ObjectFocus> = Vec::new();
        let now = self.content_scan.clone();
        match self.ui_mode {
            UiMode::Edit => {
                if let Some(now) = now.as_ref().filter(|s| s.page == page)
                    && let Some(m) = now.transform.as_ref()
                {
                    for name in &names {
                        let item = now.items.iter().find(|v| {
                            v.kind != EditTargetKind::Text
                                && v.object_name.as_ref() == Some(name)
                                && v.placement.is_some()
                        });
                        if let Some(item) = item {
                            picks.push(ObjectFocus {
                                kind: item.kind.as_str().into(),
                                name: name.clone(),
                                target_id: item.target_id.clone(),
                                rect: view_box(&item.placement.as_ref().unwrap().r#box, m),
                                text: None,
                            });
                        }
                    }
                    for id in &target_ids {
                        if let Some(item) = now.items.iter().find(|v| &v.target_id == id)
                            && item.bounds.as_ref().is_some_and(|b| b.len() == 4)
                        {
                            picks.push(text_member(item));
                        }
                    }
                }
            }
            UiMode::Forms => {
                let mut groups: Vec<String> = Vec::new();
                for id in &field_ids {
                    let Some(item) = self.fields.iter().find(|v| v.id == *id) else {
                        continue;
                    };
                    if item.kind == FieldKind::Radio && groups.contains(&item.name) {
                        continue;
                    }
                    groups.push(item.name.clone());
                    picks.push(field_member(item));
                }
            }
            UiMode::Annotate => {
                for id in &link_ids {
                    if let Some(item) = self.links.iter().find(|v| link_annot_id(v) == *id) {
                        picks.push(link_member(item));
                    }
                }
            }
            UiMode::Redact => {
                for id in &mark_ids {
                    if let Some(item) = self.notes.iter().find(|v| v.id == *id) {
                        picks.push(mark_member(item));
                    }
                }
            }
            _ => {}
        }
        if !objects_only
            && let Some(ContentEdited::Object { name, action, .. }) = self.content_edited.as_mut()
            && action == "paste"
        {
            *name = picks.last().map(|p| p.name.clone()).unwrap_or_default();
        }
        self.clear_direct_focus();
        let anchor = picks.last().cloned();
        self.set_selection(picks);
        self.restore_direct_focus(true);
        if let Some(anchor) = anchor {
            self.sync_anchor(&anchor);
        }
        Ok(true)
    }
}

/// `Number(text)` as JSON (NaN → null, as `JSON.stringify` writes it).
fn number_of(text: &str) -> Value {
    let n = super::direct::js_number(text);
    if n.is_finite() {
        if n.fract() == 0.0 && n.abs() < 9e15 {
            json!(n as i64)
        } else {
            json!(n)
        }
    } else {
        Value::Null
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announcements_are_exact() {
        assert_eq!(cut_announcement(2, 0), "Cut 2 items");
        assert_eq!(
            cut_announcement(1, 1),
            "Cut 1 item; 1 text run stays (it cannot be deleted)"
        );
        assert_eq!(
            cut_announcement(3, 2),
            "Cut 3 items; 2 text runs stay (they cannot be deleted)"
        );
        let mut kinds = BTreeMap::new();
        kinds.insert("image".to_string(), 1);
        assert_eq!(
            paste_announcement(&kinds, 1, 0, UiMode::Edit, &["Im1".into()]),
            "Pasted image Im1"
        );
        kinds.insert("field".to_string(), 2);
        kinds.insert("link".to_string(), 2);
        assert_eq!(
            paste_announcement(&kinds, 5, 1, UiMode::Edit, &[]),
            "Pasted 5 items; 2 fields shown in Fill forms; 1 link shown in Annotate; 1 link skipped (its page is not in this document)"
        );
    }

    #[test]
    fn chords_stay_out_of_text_entry() {
        let mut ctx = ChordContext {
            mode: UiMode::Edit,
            layer_live: true,
            collapsed: true,
            selection: 1,
            ..ChordContext::default()
        };
        assert!(clipboard_chord_ok("copy", &ctx));
        ctx.active = Some(ActiveElement {
            tag: "input".into(),
            kind: Some("checkbox".into()),
            ..ActiveElement::default()
        });
        assert!(clipboard_chord_ok("copy", &ctx));
        ctx.active = Some(ActiveElement {
            tag: "INPUT".into(),
            kind: None,
            ..ActiveElement::default()
        });
        assert!(!clipboard_chord_ok("copy", &ctx));
        ctx.active = None;
        assert!(!clipboard_chord_ok("paste", &ctx));
        ctx.clipboard = true;
        ctx.editable = true;
        assert!(clipboard_chord_ok("paste", &ctx));
        ctx.mode = UiMode::Read;
        assert!(!clipboard_chord_ok("paste", &ctx));
    }
}
