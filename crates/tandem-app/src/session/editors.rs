//! Inline editors (R:5061-5347): field Tab order and activation, the run/paragraph editor, the
//! editors' keys and their one commit, Add text (click to add, `text-add`), and the text layer.
#![allow(dead_code)]

use super::Session;
use super::consts::{ADD_ROTATED_TEXT, CONTENT_DETAIL_TEXT, TEXTBOX_ROTATED, lookup};
use super::direct::{grow_paragraph_editor, js_number, paragraph_text, place_editor};
use super::dom_ids as ids;
use super::fill::{add_draft, utf16_len};
use super::hit::{EvTarget, Focus, KeyEv, NodeKey, NodeKind, PtrEvent};
use super::nav::{is_pressed, js_to_fixed, max_of, min_of, reading_order, tab_order};
use super::types::*;
use serde_json::{Value, json};
use tandem_protocol::engine::{EditTargetKind, EditTargetSupport, FieldKind, Line};
use tandem_protocol::limits::MAX_FIELD_TEXT_CHARS;

/// The add editor's CSS font and colour (R:5203 `addFontCss`), for the UI.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditorFont {
    pub family: String,
    pub weight: String,
    pub style: String,
    pub color: String,
}

/// R:5203 `addFontCss()`: base-14 names in CSS terms, so the draft looks like what lands.
pub fn add_font_css(name: &str) -> EditorFont {
    let family = if name.starts_with("Times") {
        "'Times New Roman', Times, serif"
    } else if name.starts_with("Courier") {
        "'Courier New', Courier, monospace"
    } else {
        "Helvetica, Arial, sans-serif"
    };
    EditorFont {
        family: family.into(),
        weight: if name.contains("Bold") {
            "bold"
        } else {
            "normal"
        }
        .into(),
        style: if name.contains("Italic") || name.contains("Oblique") {
            "italic"
        } else {
            "normal"
        }
        .into(),
        color: String::new(),
    }
}

/// The text layer (R:5297 `paintText`) as data for the UI: one `<text>` per line and the
/// search hits' rects, in CSS px.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextLayer {
    pub width: f64,
    pub height: f64,
    pub lines: Vec<TextLine>,
    /// `[x, y, width, height]` and whether it is the current hit.
    pub hits: Vec<([f64; 4], bool)>,
}

/// One `<text>` of the text layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextLine {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub text_length: f64,
    pub font_px: f64,
    pub family: String,
    pub weight: String,
    pub style: String,
}

/// Replaces the UTF-16 range `[start, end)` of `value` with `text` (`setRangeText`).
pub fn splice_utf16(value: &str, start: usize, end: usize, text: &str) -> String {
    let units: Vec<u16> = value.encode_utf16().collect();
    let start = start.min(units.len());
    let end = end.clamp(start, units.len());
    let mut out: Vec<u16> = units[..start].to_vec();
    out.extend(text.encode_utf16());
    out.extend_from_slice(&units[end..]);
    String::from_utf16_lossy(&out)
}

/// `value.slice(0, n)` in UTF-16 units.
pub fn truncate_utf16(value: &str, n: usize) -> String {
    let units: Vec<u16> = value.encode_utf16().take(n).collect();
    String::from_utf16_lossy(&units)
}

impl Session {
    /// R:5061 `removeDirect()`: the editor leaves the page (a focused one leaves focus on the body).
    pub fn remove_direct(&mut self, active: &DirectActive) {
        let _ = active;
        if matches!(self.lane2.focus, Focus::Editor(_)) {
            self.lane2.focus = Focus::Body;
        }
        if self.reported_dirty != (self.dirty || self.restore_dirty) {
            self.sync_dirty();
        }
    }

    /// R:5062 `closeDirect()`.
    pub fn close_direct(&mut self) {
        if let Some(active) = self.direct_active.take() {
            self.remove_direct(&active);
        }
        let (w, h) = self.canvas_client();
        let page = self.shown_page;
        self.paint_hits(w, h, page);
    }

    /// The fillable fields of the shown page in Tab order.
    fn fillable_order(&self) -> Vec<FieldItem> {
        tab_order(&self.fields, self.fields_tabs.as_deref())
            .0
            .into_iter()
            .filter(|f| !f.read_only && f.kind != FieldKind::Signature)
            .collect()
    }

    /// R:5063 `focusNextField()`: Tab past the last field walks the pages (wrapping) to the next
    /// page with fillable fields.
    pub fn focus_next_field(&mut self, reverse: bool, from_page: i64, from_index: i64) {
        let Some(page_count) = self.doc.as_ref().map(|d| d.page_count) else {
            return;
        };
        let ordered = self.fillable_order();
        let next = from_index + if reverse { -1 } else { 1 };
        if from_page == self.wanted_page && next >= 0 && (next as usize) < ordered.len() {
            self.direct_field_index = next;
            let item = ordered[next as usize].clone();
            self.activate_field(&item, false);
            return;
        }
        let direction = if reverse { -1 } else { 1 };
        for step in 1..=page_count {
            let page = (from_page + direction * step + page_count).rem_euclid(page_count);
            let Ok(reply) = self.rq(json!({"op": "fields", "page": page})) else {
                return;
            };
            let items: Vec<FieldItem> = reply
                .get("items")
                .and_then(super::parse_reply)
                .unwrap_or_default();
            if !reading_order(&items)
                .iter()
                .any(|i| !i.read_only && i.kind != FieldKind::Signature)
            {
                continue;
            }
            self.go_to(page);
            self.refresh_fields();
            let current = self.fillable_order();
            if current.is_empty() {
                continue;
            }
            self.direct_field_index = if reverse { current.len() as i64 - 1 } else { 0 };
            let item = current[self.direct_field_index as usize].clone();
            self.activate_field(&item, false);
            return;
        }
    }

    /// R:5084 `moveRadio()`: the previous/next kid of the group (by x then y, wrapping) is set.
    pub fn move_radio(&mut self, item: &FieldItem, delta: i64) {
        let mut group: Vec<FieldItem> = self
            .fields
            .iter()
            .filter(|f| f.kind == FieldKind::Radio && f.name == item.name)
            .cloned()
            .collect();
        if group.is_empty() {
            return;
        }
        group.sort_by(|a, b| {
            a.rect[0]
                .partial_cmp(&b.rect[0])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(
                    a.rect[1]
                        .partial_cmp(&b.rect[1])
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
        });
        let len = group.len() as i64;
        let index = group
            .iter()
            .position(|f| f.id == item.id)
            .map_or(0, |i| i as i64);
        let next = group[((index + delta + len) % len) as usize].clone();
        let page = self.wanted_page;
        self.commit_field(&next, FieldValue::Bool(true), page as usize);
        self.focus_field_node(next.id);
    }

    /// `hitLayer.querySelector([data-field-id=id])?.focus()`.
    fn focus_field_node(&mut self, id: i64) {
        if let Some(i) = self
            .lane2
            .hit
            .nodes
            .iter()
            .position(|n| n.field_id() == Some(id))
        {
            let key = self.lane2.hit.nodes[i].key();
            self.set_focus(Focus::Node(key));
        }
    }

    /// R:5092 `activateField(item, commit)`: a signature field signs or verifies; a checkbox or
    /// radio commits on a click; text and choice fields open their inline editor.
    pub fn activate_field(&mut self, item: &FieldItem, commit: bool) {
        self.direct_field_index = self
            .fillable_order()
            .iter()
            .position(|f| f.id == item.id || (f.kind == FieldKind::Radio && f.name == item.name))
            .map_or(-1, |i| i as i64);
        if item.kind == FieldKind::Signature {
            // P5 U3 §2.2: an empty field opens the sign dialog on it; a signed one opens Verify.
            self.direct_focused_field = None;
            if super::direct::sig_signed(item) {
                self.dom
                    .set_text(ids::FIELDS_STATUS_EL, "Verify signatures");
                self.open_verify_dialog();
                self.mark_verify_row(&item.name);
            } else {
                self.dom
                    .set_text(ids::FIELDS_STATUS_EL, "Sign with certificate");
                self.open_cert_sign_dialog(Some(item));
            }
            return;
        }
        if item.read_only || !self.can_form() {
            self.direct_focused_field = None;
            let text = if item.read_only {
                "This field is read-only."
            } else {
                "Form filling is not permitted."
            };
            self.dom.set_text(ids::FIELDS_STATUS_EL, text);
            return;
        }
        self.direct_focused_field = Some(item.clone());
        self.direct_focused_page = self.wanted_page;
        if !commit {
            self.dom.set_text(ids::FIELDS_STATUS_EL, "");
        }
        self.clear_direct_focus();
        self.close_direct();
        if item.kind == FieldKind::Checkbox || item.kind == FieldKind::Radio {
            if !commit {
                self.close_direct();
                self.focus_field_node(item.id);
                return;
            }
            let value = if item.kind == FieldKind::Checkbox {
                FieldValue::Bool(item.value != FieldValue::Bool(true))
            } else {
                FieldValue::Bool(true)
            };
            let page = self.wanted_page;
            self.commit_field(item, value, page as usize);
            self.direct_focused_field = None;
            return;
        }
        let tag = if item.kind == FieldKind::Text && item.multiline == Some(true) {
            EditorTag::Textarea
        } else if item.kind == FieldKind::Combo && item.edit != Some(true) {
            EditorTag::Select
        } else {
            EditorTag::Input
        };
        let options = item.options.clone().unwrap_or_default();
        let mut value = item.value.to_js_string();
        if tag == EditorTag::Select && !options.contains(&value) {
            // Setting a select's value to no option's value selects none ("").
            value = String::new();
        }
        let option_list =
            (item.kind == FieldKind::Combo && item.edit == Some(true) && tag == EditorTag::Input)
                .then(|| options.clone());
        let label = super::form_fields::field_label(item);
        let mut editor = EditorState {
            tag,
            class_name: "inline-editor".into(),
            max_length: if tag == EditorTag::Select {
                -1
            } else {
                item.max_len.unwrap_or(if item.kind == FieldKind::Combo {
                    MAX_FIELD_TEXT_CHARS as i64
                } else {
                    4096
                })
            },
            title: item
                .tu
                .clone()
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| label.clone()),
            selection_start: (tag != EditorTag::Select).then(|| utf16_len(&value)),
            selection_end: (tag != EditorTag::Select).then(|| utf16_len(&value)),
            options: if tag == EditorTag::Select {
                options.iter().map(|o| (o.clone(), o.clone())).collect()
            } else {
                Vec::new()
            },
            value,
            ..EditorState::default()
        };
        place_editor(&mut editor, &item.rect, self.shown_zoom);
        self.lane2.editor_label = label;
        let page = self.wanted_page;
        self.direct_active = Some(DirectActive {
            kind: DirectKind::Field,
            item: DirectItem::Field(item.clone()),
            editor,
            page,
            option_list,
            ..DirectActive::default()
        });
        self.direct_field_index = self
            .fillable_order()
            .iter()
            .position(|f| f.id == item.id)
            .map_or(-1, |i| i as i64);
        self.setup_inline_events(item.kind == FieldKind::Text && item.multiline == Some(true));
        self.focus_editor();
        if tag != EditorTag::Select {
            self.select_editor_text();
        }
    }

    /// R:5121 `markActiveTextTarget()`: `.active` (and `.single-run-active`) on the edited text target.
    pub fn mark_active_text_target(&mut self) {
        let Some(active) = self
            .direct_active
            .as_ref()
            .filter(|a| a.kind == DirectKind::Text)
        else {
            return;
        };
        let item = match &active.item {
            DirectItem::Target(t) => t.clone(),
            _ => return,
        };
        let paragraph = active.paragraph.clone();
        let single_run = paragraph.is_none() && item.paragraph_id.is_some();
        for node in &mut self.lane2.hit.nodes {
            let NodeKind::Text { item: painted } = &node.kind else {
                continue;
            };
            let matches = match &paragraph {
                Some(p) => painted.paragraph_id.as_ref() == Some(&p.paragraph_id),
                None => painted.target_id == item.target_id,
            };
            node.active = matches;
            node.single_run_active = matches && single_run;
        }
    }

    /// R:5129 `openInlineText()`: a paragraph textarea or a run input over the text.
    pub fn open_inline_text(&mut self, item: &EditTarget, single_run: bool) {
        let Some(scan) = self.content_scan.clone() else {
            return;
        };
        let Some(bounds) = item.bounds.clone() else {
            return;
        };
        let paragraph = if single_run {
            None
        } else {
            item.paragraph_id.as_ref().and_then(|id| {
                scan.paragraphs
                    .iter()
                    .find(|p| &p.paragraph_id == id)
                    .cloned()
            })
        };
        self.clear_direct_focus();
        self.close_direct();
        let value = match &paragraph {
            Some(p) => paragraph_text(p, self.content_scan.as_ref()),
            None => item.text.clone().unwrap_or_default(),
        };
        let font_size = paragraph
            .as_ref()
            .map(|p| p.font_size)
            .or(item.font_size)
            .unwrap_or(12.0);
        let mut editor = EditorState {
            tag: if paragraph.is_some() {
                EditorTag::Textarea
            } else {
                EditorTag::Input
            },
            class_name: format!(
                "inline-editor text-inline-editor{}",
                if paragraph.is_some() {
                    " paragraph"
                } else {
                    ""
                }
            ),
            max_length: 4096,
            font_size: Some(crate::js::js_num(font_size)),
            selection_start: Some(utf16_len(&value)),
            selection_end: Some(utf16_len(&value)),
            value,
            ..EditorState::default()
        };
        self.lane2.editor_label = match &paragraph {
            Some(p) => format!("Paragraph, {} lines", p.lines),
            None => item.text.clone().unwrap_or_else(|| "Text".into()),
        };
        let zoom = self.shown_zoom;
        place_editor(
            &mut editor,
            paragraph.as_ref().map_or(&bounds, |p| &p.bounds),
            zoom,
        );
        if let Some(p) = &paragraph {
            editor.width = p.width * zoom;
            editor.line_height = Some(p.leading * zoom);
            grow_paragraph_editor(&mut editor, Some((p.bounds[3] - p.bounds[1]) * zoom));
        }
        self.direct_active = Some(DirectActive {
            kind: DirectKind::Text,
            item: DirectItem::Target(item.clone()),
            editor,
            page: scan.page,
            scan: Some(scan),
            paragraph: paragraph.clone(),
            ..DirectActive::default()
        });
        self.mark_active_text_target();
        self.setup_inline_events(paragraph.is_some());
        self.focus_editor();
        self.select_editor_text();
    }

    /// R:5144 `setupInlineEvents(editor, multiline)`: the open editor's listeners. Every path
    /// that opens an editor calls it right after setting `directActive` (lane 4's link editor
    /// too); it gives the editor its identity and its place in the layer's order.
    pub fn setup_inline_events(&mut self, multiline: bool) {
        self.lane2.editor_id += 1;
        self.lane2.editor_multiline = multiline;
        self.lane2.hit.next_seq += 1;
        self.lane2.hit.editor_seq = self.lane2.hit.next_seq;
    }

    /// `editor.select()`.
    pub fn select_editor_text(&mut self) {
        if let Some(active) = self.direct_active.as_mut()
            && active.editor.tag != EditorTag::Select
        {
            active.editor.selection_start = Some(0);
            active.editor.selection_end = Some(utf16_len(&active.editor.value));
        }
    }

    /// R:5145 the editor's `input`: the dirty indicator follows a pending draft; a paragraph
    /// grows, the add and text-box editors grow to their draft.
    pub(crate) fn on_editor_input(&mut self) {
        if self.draft_pending() != self.reported_dirty && !self.dirty && !self.restore_dirty {
            self.sync_dirty();
        }
        let Some(active) = self.direct_active.as_mut() else {
            return;
        };
        if active.paragraph.is_some() {
            grow_paragraph_editor(&mut active.editor, None);
        }
        if matches!(active.kind, DirectKind::Add | DirectKind::Textbox) {
            self.grow_add_editor();
        }
    }

    /// R:5146 the editor's `keydown`: Escape discards (a note with text commits), ⌥-arrow nudges
    /// a field, Tab commits (and moves on from a field or run), Enter commits a single-line
    /// editor and ⌘Enter / Ctrl+Enter any.
    pub(crate) fn on_editor_keydown(&mut self, ev: &mut KeyEv) {
        let Some(active) = self.direct_active.clone() else {
            return;
        };
        if ev.key == "Escape" && !ev.composing {
            ev.prevent();
            if active.kind == DirectKind::Note && !self.direct_unchanged(&active) {
                self.finish_direct();
                return;
            }
            if active.kind == DirectKind::Add {
                self.clear_error();
            }
            self.direct_focused_field = None;
            self.close_direct();
        } else if ev.alt && ev.is_arrow() && active.kind == DirectKind::Field {
            ev.prevent();
            if let DirectItem::Field(item) = &active.item {
                self.nudge_field(item, &ev.key, ev.shift, ev.cmd());
            }
        } else if ev.key == "Tab" {
            ev.prevent();
            if matches!(
                active.kind,
                DirectKind::Add | DirectKind::Link | DirectKind::Textbox | DirectKind::Note
            ) {
                self.finish_direct();
                return;
            }
            let (reverse, page, index) = (ev.shift, active.page, self.direct_field_index);
            self.finish_direct();
            self.focus_next_field(reverse, page, index);
        } else if ev.key == "Enter" && (!self.lane2.editor_multiline || ev.meta || ev.ctrl) {
            ev.prevent();
            self.finish_direct();
        }
    }

    /// R:5150 `draftPending()`: an open editor holds typing that is not in the document yet.
    pub fn draft_pending(&self) -> bool {
        self.direct_active
            .as_ref()
            .is_some_and(|a| !self.direct_unchanged(a))
    }

    /// R:5154 `directUnchanged()`: an editor whose value is still the document's commits nothing.
    pub fn direct_unchanged(&self, active: &DirectActive) -> bool {
        let value = &active.editor.value;
        match active.kind {
            DirectKind::Add | DirectKind::Note => add_draft(value).is_empty(),
            DirectKind::Textbox => match active.annot_id {
                None => add_draft(value).is_empty(),
                Some(_) => {
                    add_draft(value) == add_draft(active.editor.initial.as_deref().unwrap_or(""))
                }
            },
            DirectKind::Link => {
                crate::js::js_trim(value) == active.editor.initial.as_deref().unwrap_or("")
            }
            _ => {
                let document = match (&active.item, &active.paragraph) {
                    (DirectItem::Field(f), _) => f.value.to_js_string(),
                    (_, Some(p)) => paragraph_text(p, active.scan.as_ref()),
                    (DirectItem::Target(t), None) => t.text.clone().unwrap_or_default(),
                    (DirectItem::Link(_), None) => String::new(),
                };
                *value == document
            }
        }
    }

    /// R:5162 `dropDirect()`: closes the editor now and commits its draft (a mode or page change).
    pub fn drop_direct(&mut self) {
        let Some(active) = self.direct_active.take() else {
            return;
        };
        self.direct_focused_field = None;
        self.remove_direct(&active);
        if self.direct_committing || self.direct_unchanged(&active) {
            return;
        }
        self.commit_editor(&active, true);
    }

    /// The commit an editor's kind makes (`dropDirect` passes `silent` to the link editor).
    fn commit_editor(&mut self, active: &DirectActive, silent: bool) -> bool {
        let value = active.editor.value.clone();
        let point = active.point.unwrap_or([0.0, 0.0]);
        match (&active.kind, &active.item) {
            (DirectKind::Field, DirectItem::Field(item)) => {
                self.commit_field(item, FieldValue::Text(value), active.page as usize)
            }
            (DirectKind::Link, _) => self.commit_link_editor(active, silent),
            (DirectKind::Note, _) => {
                self.add_note_at(active.page, point, &add_draft(&value), "Added note")
            }
            (DirectKind::Add, _) => {
                let at = self.add_point(active);
                self.add_text_at(active.page, &at, &add_draft(&value))
            }
            (DirectKind::Textbox, _) => {
                self.commit_text_box(active.page, active.annot_id, &point, &add_draft(&value))
            }
            (_, DirectItem::Target(item)) => {
                let scan = active.scan.clone().unwrap_or_default();
                match &active.paragraph {
                    Some(p) => self.apply_paragraph_edit(
                        &scan,
                        p,
                        &value,
                        super::direct::TextTransform::default(),
                    ),
                    None => self.apply_content_edit(&scan, item, &value),
                }
            }
            _ => false,
        }
    }

    /// R:5176 `finishDirect()`: the open editor's one commit (a second call while it runs is a no-op).
    pub fn finish_direct(&mut self) {
        let Some(active) = self.direct_active.clone() else {
            return;
        };
        if self.direct_committing {
            return;
        }
        let id = self.lane2.editor_id;
        let field_kind = match &active.item {
            DirectItem::Field(f) => Some(f.kind),
            _ => None,
        };
        if active.kind == DirectKind::Field
            && matches!(field_kind, Some(FieldKind::Checkbox | FieldKind::Radio))
        {
            self.close_direct();
            return;
        }
        if self.direct_unchanged(&active) {
            self.direct_focused_field = None;
            self.close_direct();
            if active.kind == DirectKind::Note {
                self.announce(crate::messages::annotate::NOTE_EMPTY);
            }
            return;
        }
        // One commit per editor: Enter then blur (or a page change) must not send a second field-set.
        self.direct_committing = true;
        let ok = self.commit_editor(&active, false);
        self.direct_committing = false;
        let still_open = self.direct_active.is_some() && self.lane2.editor_id == id;
        if ok {
            self.direct_focused_field = None;
            if still_open {
                self.close_direct();
            } else {
                self.remove_direct(&active);
            }
            // §9.7: focus the new run only when nothing else took the page meanwhile.
            if active.kind == DirectKind::Add
                && self.direct_active.is_none()
                && let Some(target_id) = self
                    .content_last_add
                    .as_ref()
                    .map(|a| a.target_id.clone())
                    .filter(|t| !t.is_empty())
            {
                self.focus_node(&NodeKey::Text(target_id));
            }
        } else if !matches!(
            active.kind,
            DirectKind::Add | DirectKind::Link | DirectKind::Textbox | DirectKind::Note
        ) && still_open
        {
            let revert = match (&active.item, &active.paragraph) {
                (DirectItem::Field(f), _) => f.value.to_js_string(),
                (_, Some(p)) => paragraph_text(p, self.content_scan.as_ref()),
                (DirectItem::Target(t), None) => t.text.clone().unwrap_or_default(),
                _ => String::new(),
            };
            if let Some(open) = self.direct_active.as_mut() {
                open.editor.selection_start = Some(utf16_len(&revert));
                open.editor.selection_end = Some(utf16_len(&revert));
                open.editor.value = revert;
            }
        }
    }

    /// R:5207 `addFontSize()`.
    pub fn add_font_size(&self) -> f64 {
        let n = js_number(&self.dom.value(ids::CONTENT_ADD_SIZE));
        let n = if n.is_nan() || n == 0.0 { 12.0 } else { n };
        400.0f64.min(4.0f64.max(n))
    }

    /// R:5216 `addPoint()`: the baseline ≈ top + 0.8·size for base-14, kept on the page.
    pub fn add_point(&self, active: &DirectActive) -> Vec<f64> {
        let point = active.point.unwrap_or([0.0, 0.0]);
        let size = active
            .editor
            .font_size
            .as_deref()
            .map_or(f64::NAN, js_number);
        let size = if size.is_nan() || size == 0.0 {
            12.0
        } else {
            size
        };
        let height = self
            .page_size(active.page)
            .map_or(f64::INFINITY, |s| s.height);
        vec![point[0], (point[1] + 0.8 * size).min(height)]
    }

    /// R:5220 `openAddText()`: the add editor at the click point.
    pub fn open_add_text(&mut self, page: i64, point: [f64; 2]) {
        self.clear_direct_focus();
        self.close_direct();
        self.clear_error();
        let size = self.add_font_size();
        let mut font = add_font_css(&self.dom.value(ids::CONTENT_ADD_FONT));
        font.color = self.dom.value(ids::CONTENT_ADD_COLOR);
        let rect = vec![
            point[0],
            point[1],
            point[0] + (8.0 * size).max(48.0),
            point[1] + 1.2 * size,
        ];
        let mut editor = EditorState {
            tag: EditorTag::Textarea,
            class_name: "inline-editor text-inline-editor add-text".into(),
            max_length: 4096,
            font_size: Some(crate::js::js_num(size)),
            selection_start: Some(0),
            selection_end: Some(0),
            ..EditorState::default()
        };
        self.lane2.editor_label = format!(
            "New text at {}, {} pt",
            js_to_fixed(point[0], 1),
            js_to_fixed(point[1], 1)
        );
        self.lane2.editor_font = Some(font);
        place_editor(&mut editor, &rect, self.shown_zoom);
        let item = EditTarget {
            kind: EditTargetKind::Text,
            bounds: Some(rect),
            support: EditTargetSupport::Candidate,
            text: Some(String::new()),
            ..EditTarget::default()
        };
        self.direct_active = Some(DirectActive {
            kind: DirectKind::Add,
            item: DirectItem::Target(item),
            editor,
            page,
            point: Some(point),
            ..DirectActive::default()
        });
        self.grow_add_editor();
        self.setup_inline_events(true);
        self.focus_editor();
    }

    /// R:5236 `clickToAdd()`: a click on empty page space in Edit text opens the add editor
    /// there; true when the click was used. Stray clicks without the tool change nothing (X6).
    pub fn click_to_add(&mut self, ev: &PtrEvent) -> bool {
        let blocked = std::mem::take(&mut self.add_blocked);
        // P6 U7 §9.27/F3: a Control-click is the macOS right-click; a Shift-click only extends the selection.
        if ev.ctrl || ev.shift || ev.button != 0 {
            return false;
        }
        if ev.target.in_hit_layer() {
            return false;
        }
        let focused = self.direct_focused_object.is_some()
            || self.direct_focused_mark.is_some()
            || self.direct_focused_box.is_some()
            || !self.direct_selection.is_empty();
        if matches!(self.ui_mode, UiMode::Edit | UiMode::Redact | UiMode::Forms) && focused {
            self.drop_direct_focus();
        }
        if self.ui_mode == UiMode::Forms {
            return is_pressed(&self.dom, ids::TEXTBOX_TOOL_BUTTON)
                && self.click_to_type(ev, blocked);
        }
        if self.ui_mode != UiMode::Edit
            || !is_pressed(&self.dom, ids::TEXT_ADD_TOOL_BUTTON)
            || blocked
            || self.direct_active.is_some()
            || self.direct_committing
            || !self.layer_live()
            || self.doc.is_none()
            || !self.can_edit()
        {
            return false;
        }
        if !self.lane2.text_collapsed {
            return false;
        }
        let page = self.wanted_page;
        let (Some(size), Some(point)) = (self.page_size(page), self.page_point_of(ev)) else {
            return false;
        };
        if self.shown_page != page
            || point[0] < 0.0
            || point[1] < 0.0
            || point[0] > size.width
            || point[1] > size.height
        {
            return false;
        }
        if self.rotation(page) % 360 != 0 {
            return self.refuse_add_text();
        }
        let scan = self.content_scan.clone();
        let Some(m) = scan
            .as_ref()
            .filter(|s| s.page == page && s.doc_generation == Some(self.generation))
            .and_then(|s| s.transform.clone())
        else {
            return false;
        };
        if (m[0] - 1.0).abs() > 1e-6
            || m[1].abs() > 1e-6
            || m[2].abs() > 1e-6
            || (m[3] + 1.0).abs() > 1e-6
        {
            return self.refuse_add_text();
        }
        self.open_add_text(page, point);
        true
    }

    /// `clickToAdd`'s refusal on a rotated or transformed page.
    fn refuse_add_text(&mut self) -> bool {
        self.show_error(ADD_ROTATED_TEXT);
        self.content_refusals.push(ContentRefusal {
            text: String::new(),
            detail: "unsupported-transform".into(),
        });
        true
    }

    /// R:5258 `clickToType()`: in Fill forms a click on empty page space opens the text-box editor.
    pub fn click_to_type(&mut self, ev: &PtrEvent, blocked: bool) -> bool {
        if blocked
            || self.direct_active.is_some()
            || self.direct_committing
            || !self.layer_live()
            || self.doc.is_none()
            || !self.can_annotate()
        {
            return false;
        }
        if !self.lane2.text_collapsed {
            return false;
        }
        let page = self.wanted_page;
        let (Some(size), Some(point)) = (self.page_size(page), self.page_point_of(ev)) else {
            return false;
        };
        if self.shown_page != page
            || point[0] < 0.0
            || point[1] < 0.0
            || point[0] > size.width
            || point[1] > size.height
        {
            return false;
        }
        if self.rotation(page) % 360 != 0 {
            self.show_error(TEXTBOX_ROTATED);
            return true;
        }
        self.open_text_box(page, point, None);
        true
    }

    /// R:5268 `addTextAt()`: the one `text-add` the page editor and the panel send.
    pub fn add_text_at(&mut self, page: i64, point: &[f64], text: &str) -> bool {
        if self.doc.is_none() || !self.can_edit() || text.is_empty() {
            return false;
        }
        self.clear_error();
        let color_value = self.dom.value(ids::CONTENT_ADD_COLOR);
        let hex = color_value.get(1..).unwrap_or("");
        let color: Vec<Value> = [0usize, 2, 4]
            .iter()
            .map(|&i| {
                match hex
                    .get(i..i + 2)
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                {
                    Some(n) => json!(n as f64 / 255.0),
                    None => Value::Null,
                }
            })
            .collect();
        let font = self.dom.value(ids::CONTENT_ADD_FONT);
        let size = self.add_font_size();
        let reply = match self.rq(json!({"op": "text-add", "page": page, "point": point, "text": text, "font": font, "size": size, "color": color})) {
            Ok(reply) => reply,
            Err(err) => {
                let refused = &err.reply;
                let detail = refused.get("detail").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| err.reason.clone().unwrap_or_else(|| "invalid".into()));
                let missing: Vec<String> = refused.get("missing").and_then(Value::as_array).map(|m| m.iter().filter_map(|c| c.as_str().map(str::to_string)).collect()).unwrap_or_default();
                let message = if detail == "unsupported-encoding" && !missing.is_empty() {
                    format!("Cannot draw: {} (not in this font)", missing.join(" "))
                } else {
                    lookup(&CONTENT_DETAIL_TEXT, &detail).map(str::to_string).unwrap_or_else(|| err.message.clone())
                };
                self.content_refusals.push(ContentRefusal { text: String::new(), detail });
                self.announce(&message);
                self.show_error(&message);
                return false;
            }
        };
        let target_id = reply
            .get("targetId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let bounds = reply
            .get("bounds")
            .and_then(super::parse_reply::<Vec<f64>>);
        self.content_last_add = Some(ContentLastAdd {
            target_id: target_id.clone(),
            bounds: bounds.clone(),
        });
        self.apply_edit(&super::modes::journal_of(&reply.value), "Added text");
        self.refresh_content_targets(true);
        // P6 U8: text added exactly on top of the same text stays ambiguous (overprint) and gets no target.
        if !target_id.is_empty() && bounds.is_none() {
            self.announce("Added text is not selectable on the page (it lies exactly on top of the same text)");
        } else if target_id.is_empty() {
            // R8-3: the engine found no target for the new run.
            self.announce("Added text is not editable on the page");
        }
        true
    }

    /// R:5297 `paintText()`: the selectable text layer, the search hits and the redaction boxes.
    /// Unchanged lines and zoom keep their nodes (and a pending text selection).
    pub fn paint_text(
        &mut self,
        lines: &[Line],
        zoom: f64,
        css_width: f64,
        css_height: f64,
        page: i64,
    ) {
        let key = json!([
            page,
            zoom,
            lines
                .iter()
                .map(|l| json!([l.text, l.x, l.y, l.w, l.size]))
                .collect::<Vec<Value>>()
        ])
        .to_string();
        let keep_text = key == self.painted_text_key && !self.lane2.text_layer.lines.is_empty();
        self.painted_text_key = key;
        let mut layer = TextLayer {
            width: css_width,
            height: css_height,
            ..TextLayer::default()
        };
        layer.lines = if keep_text {
            std::mem::take(&mut self.lane2.text_layer.lines)
        } else {
            lines
                .iter()
                .map(|l| TextLine {
                    text: l.text.clone(),
                    x: l.x * zoom,
                    y: l.y * zoom,
                    text_length: l.w * zoom,
                    font_px: l.size * zoom,
                    family: l.family.clone(),
                    weight: l.weight.clone(),
                    style: l.style.clone(),
                })
                .collect()
        };
        for (index, hit) in self.search_hits.iter().enumerate() {
            if hit.page != page {
                continue;
            }
            for quad in &hit.quads {
                if quad.len() < 8 {
                    continue;
                }
                let xs = [quad[0], quad[2], quad[4], quad[6]];
                let ys = [quad[1], quad[3], quad[5], quad[7]];
                let (x, y) = (min_of(&xs) * zoom, min_of(&ys) * zoom);
                layer.hits.push((
                    [x, y, max_of(&xs) * zoom - x, max_of(&ys) * zoom - y],
                    index as i64 == self.search_index,
                ));
            }
        }
        self.lane2.text_layer = layer;
        // Redaction redesign §2.3 step 2: boxes, and the tint of what goes under them (redact-ui.ts).
        let boxes = self.redact_page_rects(page);
        self.lane2.redact_overlay = self.redact_ui_paint_boxes(page, zoom, &boxes);
        self.schedule_redact_inspect(page as usize, boxes);
    }

    /// Whether the event target is the open editor.
    pub fn on_editor(ev: &EvTarget) -> bool {
        *ev == EvTarget::Editor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_map_to_css() {
        let f = add_font_css("Times-BoldItalic");
        assert_eq!(
            (f.family.as_str(), f.weight.as_str(), f.style.as_str()),
            ("'Times New Roman', Times, serif", "bold", "italic")
        );
        let f = add_font_css("Courier-Oblique");
        assert_eq!((f.weight.as_str(), f.style.as_str()), ("normal", "italic"));
    }

    #[test]
    fn utf16_splices() {
        assert_eq!(splice_utf16("abc", 1, 2, "XY"), "aXYc");
        assert_eq!(splice_utf16("a😀b", 3, 3, "!"), "a😀!b");
        assert_eq!(truncate_utf16("abcdef", 4), "abcd");
    }

    #[test]
    fn unchanged_editors_commit_nothing() {
        let s = Session::new();
        let mut active = DirectActive {
            kind: DirectKind::Textbox,
            annot_id: Some(3),
            ..DirectActive::default()
        };
        active.editor.initial = Some("Hi\n".into());
        active.editor.value = "\nHi".into();
        assert!(s.direct_unchanged(&active));
        active.annot_id = None;
        assert!(!s.direct_unchanged(&active));
        let link = DirectActive {
            kind: DirectKind::Link,
            editor: EditorState {
                value: " 3 ".into(),
                initial: Some("3".into()),
                ..EditorState::default()
            },
            ..DirectActive::default()
        };
        assert!(s.direct_unchanged(&link));
    }
}
