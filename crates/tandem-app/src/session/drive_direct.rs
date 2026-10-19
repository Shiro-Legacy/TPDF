//! The drive's gesture ops (R:1988-2179, in lane 1's line range but they drive this lane's hit
//! layer): `click`, `menu`, `drag`/`handle`, `type` and `key`, as synthetic events at page
//! points routed through the same listeners the UI's `pointer` and `key` reach.
//!
//! The renderer waited (10 ms polls) until the page had rendered and its fields and links
//! were read; here those reads are done when `go_to` + `drain` return. If they are still not
//! in place (a failing read), the TS polled until the drive's 60 s step timeout; here the op
//! returns at once (the step then reports the STATE as it is).
#![allow(dead_code)]

use super::Session;
use super::dom_ids as ids;
use super::hit::{EvTarget, Focus, HitNode, KeyEv, NodeKind, Part, PtrEvent};
use super::nav::is_pressed;
use super::object_menu::menu_button;
use super::types::*;
use crate::js::js_round;
use serde_json::{Map, Value};
use tandem_protocol::engine::FieldKind;

/// R:2034 `MENU_ITEMS`: drive item names → menu keys.
pub const MENU_ITEMS: [(&str, &str); 12] = [
    ("cut", "cut"),
    ("copy", "copy"),
    ("paste", "paste"),
    ("replace", "replace"),
    ("duplicate", "duplicate"),
    ("rotate-cw", "rotateCw"),
    ("rotate-ccw", "rotateCcw"),
    ("flip-h", "flipH"),
    ("flip-v", "flipV"),
    ("crop", "crop"),
    ("delete", "delete"),
    ("redact-page", "redactPage"),
];

/// `typeof value === "number"`.
fn num(value: &Map<String, Value>, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

impl Session {
    /// Clears the per-event state a new drive step or UI event starts without.
    pub(crate) fn begin_event(&mut self) {
        self.lane2.blurred = None;
        self.lane2.blur_committed = false;
    }

    /// The common start of `click`/`menu`/`drag`: the page, rendered, with its fields and
    /// links read. Returns the page size and the canvas CSS size.
    fn drive_page_ready(&mut self, page: i64) -> Option<(PageSize, [f64; 2])> {
        self.go_to(page);
        self.drain();
        if self.doc.is_none()
            || self.shown_page != page
            || self.in_flight.is_some()
            || self.fields_page != page
            || self.links_page != page
        {
            return None;
        }
        let size = self.page_size(self.shown_page)?;
        if size.height < 1.0 {
            return None;
        }
        let canvas = self.lane2.canvas_css;
        if canvas[0] < 1.0 {
            return None;
        }
        Some((size, canvas))
    }

    /// The `.hit-target` a target is in (`closest(".hit-target")`).
    fn closest_target(&self, target: &EvTarget) -> Option<usize> {
        match target {
            EvTarget::Node { index, .. }
                if self
                    .lane2
                    .hit
                    .nodes
                    .get(*index)
                    .is_some_and(HitNode::is_target) =>
            {
                Some(*index)
            }
            _ => None,
        }
    }

    /// R:1988 `driveDirectClick()`: `click {page, x, y, alt?, shift?}` at page points.
    pub fn drive_direct_click(&mut self, contents: Option<&str>) {
        self.begin_event();
        self.lane2.swallow_click = false;
        let Some(value) = self.drive_wanted(contents) else {
            return;
        };
        let Some(page_count) = self.doc.as_ref().map(|d| d.page_count) else {
            return;
        };
        let (Some(page), Some(x), Some(y)) =
            (num(&value, "page"), num(&value, "x"), num(&value, "y"))
        else {
            self.show_error("click: page, x and y are required");
            return;
        };
        if page.fract() != 0.0 || page < 0.0 || page >= page_count as f64 {
            self.show_error(&format!(
                "click: page must be an integer between 0 and {}",
                page_count - 1
            ));
            return;
        }
        let (alt, shift) = (
            value.get("alt") == Some(&Value::Bool(true)),
            value.get("shift") == Some(&Value::Bool(true)),
        );
        let Some((size, canvas)) = self.drive_page_ready(page as i64) else {
            return;
        };
        // Review fix 14: a mouse event's coordinates are whole CSS pixels: round to the nearest.
        let cx = js_round(x * canvas[0] / size.width);
        let cy = js_round(y * canvas[1] / size.height);
        let target = self.hit_test(cx, cy);
        let placing_note = is_pressed(&self.dom, ids::NOTE_ADD_BUTTON);
        let click_target = if placing_note && target.in_hit_layer() {
            EvTarget::Page
        } else {
            target
        };
        let saved = self.target_key(&click_target);
        match &click_target {
            EvTarget::Node {
                index,
                part: Part::Body,
            } if self.lane2.hit.nodes[*index].is_target() => {
                let key = self.lane2.hit.nodes[*index].key();
                self.set_focus(Focus::Node(key));
            }
            EvTarget::Editor => self.focus_editor(),
            _ => {}
        }
        let event = PtrEvent {
            client_x: cx,
            client_y: cy,
            button: 0,
            shift,
            alt,
            target: click_target.clone(),
            detail: 1,
            ..PtrEvent::default()
        };
        // The focus change may have committed an editor and repainted: the same node, by key.
        let mut down = event.clone();
        self.refresh_target(&mut down.target, &saved);
        self.dispatch_pointerdown(&mut down);
        // §9.9: a press on the page outside the layer moves focus to #page, so an open editor commits first.
        if !click_target.in_hit_layer() && click_target.in_page() {
            let open = self.direct_active.is_some().then_some(self.lane2.editor_id);
            let blurred = self.set_focus(Focus::Page);
            if !blurred
                && open.is_some_and(|id| self.direct_active.is_some() && self.lane2.editor_id == id)
            {
                self.finish_direct(); // no blur event in an unfocused window
            }
        }
        let mut up = event.clone();
        self.refresh_target(&mut up.target, &saved);
        self.dispatch_pointerup(&mut up, false, false);
        let mut click = event;
        self.refresh_target(&mut click.target, &saved);
        self.dispatch_click(&mut click);
    }

    /// R:2030 `driveObjectMenu()`: `menu {page, x, y, item?}` — a right-click on the target under
    /// the point, then optionally one item.
    pub fn drive_object_menu(&mut self, contents: Option<&str>) {
        self.begin_event();
        self.lane2.swallow_click = false;
        let Some(value) = self.drive_wanted(contents) else {
            return;
        };
        let Some(page_count) = self.doc.as_ref().map(|d| d.page_count) else {
            return;
        };
        let (Some(page), Some(x), Some(y)) =
            (num(&value, "page"), num(&value, "x"), num(&value, "y"))
        else {
            self.show_error("menu: page, x and y are required");
            return;
        };
        if page.fract() != 0.0 || page < 0.0 || page >= page_count as f64 {
            self.show_error(&format!(
                "menu: page must be an integer between 0 and {}",
                page_count - 1
            ));
            return;
        }
        let wanted = match value.get("item") {
            None => None,
            Some(Value::String(item)) if MENU_ITEMS.iter().any(|(k, _)| k == item) => {
                Some(item.clone())
            }
            Some(_) => {
                self.show_error("menu: item must be cut, copy, paste, replace, duplicate, rotate-cw, rotate-ccw, flip-h, flip-v, crop, delete or redact-page");
                return;
            }
        };
        self.close_object_menu(false); // as the outside press of a real right-click would
        let Some((size, canvas)) = self.drive_page_ready(page as i64) else {
            return;
        };
        let cx = x * canvas[0] / size.width;
        let cy = y * canvas[1] / size.height;
        let target = self.hit_test_all(cx, cy).into_iter().next();
        // P6 U7 §9.28: empty page space opens the page menu; the drive hands it the exact page point.
        self.drive_menu_point = Some(vec![x, y]);
        let mut ev = PtrEvent {
            client_x: cx,
            client_y: cy,
            button: 2,
            target: target.unwrap_or(EvTarget::Page),
            ..PtrEvent::default()
        };
        self.dispatch_contextmenu(&mut ev);
        self.drive_menu_point = None;
        let Some(wanted) = wanted else { return };
        let key = MENU_ITEMS
            .iter()
            .find(|(k, _)| *k == wanted)
            .map_or("", |(_, v)| v);
        if self.dom.hidden(ids::OBJECT_MENU) || self.dom.disabled(menu_button(key)) {
            self.show_error(&format!("menu: {wanted} is not available"));
            return;
        }
        let mut click = PtrEvent {
            target: EvTarget::MenuItem(key),
            detail: 0,
            ..PtrEvent::default()
        };
        self.dispatch_click(&mut click);
    }

    /// R:2061 `driveDirectGesture()`: `drag`/`handle {page, x, y, dx?, dy?, handle?, shift?,
    /// alt?, meta?, escape?}`; no validation and no error messages.
    pub fn drive_direct_gesture(&mut self, op: &str, contents: Option<&str>) {
        self.begin_event();
        self.lane2.swallow_click = false;
        let Some(value) = self.drive_wanted(contents) else {
            return;
        };
        if self.doc.is_none() {
            return;
        }
        let (Some(page), Some(x), Some(y)) =
            (num(&value, "page"), num(&value, "x"), num(&value, "y"))
        else {
            return;
        };
        // goTo truncates; a page that is not an integer never matches the shown page (the TS waited forever).
        if page.fract() != 0.0 {
            self.go_to(page.trunc() as i64);
            self.drain();
            return;
        }
        let Some((size, canvas)) = self.drive_page_ready(page as i64) else {
            return;
        };
        let (sx, sy) = (canvas[0] / size.width, canvas[1] / size.height);
        let (cx, cy) = (x * sx, y * sy);
        let dx = num(&value, "dx").map_or(0.0, |d| d * sx);
        let dy = num(&value, "dy").map_or(0.0, |d| d * sy);
        let shift = value.get("shift") == Some(&Value::Bool(true));
        let alt = value.get("alt") == Some(&Value::Bool(true));
        let meta = value.get("meta") == Some(&Value::Bool(true));
        let target = self.hit_test_all(cx, cy).into_iter().next();
        let on_target = target.as_ref().is_some_and(|t| {
            matches!(
                t,
                EvTarget::Node {
                    part: Part::Handle(_) | Part::Ring,
                    ..
                }
            ) || self.closest_target(t).is_some()
        });
        if op == "drag" && !(self.layer_live() && on_target) {
            // Empty space (or an armed tool): the page takes the press, the window the move and release.
            let base = PtrEvent {
                button: 0,
                shift,
                ..PtrEvent::default()
            };
            let mut down = PtrEvent {
                client_x: cx,
                client_y: cy,
                target: EvTarget::Page,
                ..base.clone()
            };
            self.dispatch_pointerdown(&mut down);
            let mut mv = PtrEvent {
                client_x: cx + dx,
                client_y: cy + dy,
                target: EvTarget::Window,
                ..base.clone()
            };
            self.dispatch_pointermove(&mut mv, true);
            let mut up = PtrEvent {
                client_x: cx + dx,
                client_y: cy + dy,
                target: EvTarget::Window,
                ..base
            };
            self.dispatch_pointerup(&mut up, true, false);
            return;
        }
        let field_target = target
            .as_ref()
            .and_then(|t| self.closest_target(t))
            .or_else(|| {
                self.lane2.hit.nodes.iter().position(|n| {
                    n.is_target()
                        && n.css.left <= cx
                        && cx <= n.css.left + n.css.width
                        && n.css.top <= cy
                        && cy <= n.css.top + n.css.height
                })
            });
        // P6 U6 §9.13: a handle on a group member resolves against the union's `.group-box`.
        let group_box = self.lane2.hit.group_box();
        let owner = match &target {
            Some(EvTarget::Node { index, .. })
                if self.lane2.hit.nodes[*index].kind == NodeKind::GroupBox =>
            {
                Some(*index)
            }
            _ => match field_target {
                Some(i) if self.lane2.hit.nodes[i].multi => group_box,
                other => other,
            },
        };
        let (handle_target, start) = if op == "handle" {
            let name = value.get("handle").map_or("grip".to_string(), |h| match h {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            });
            let Some(owner) = owner else { return };
            let node = &self.lane2.hit.nodes[owner];
            let Some(handle) = node.handles.iter().copied().find(|h| *h == name) else {
                return;
            };
            let b = node.handle_box(handle);
            (
                EvTarget::Node {
                    index: owner,
                    part: Part::Handle(handle),
                },
                (b.left + b.width / 2.0, b.top + b.height / 2.0),
            )
        } else {
            let Some(target) = target else { return };
            (target, (cx, cy))
        };
        let down = PtrEvent {
            client_x: start.0,
            client_y: start.1,
            button: 0,
            shift,
            alt,
            meta,
            target: handle_target,
            ..PtrEvent::default()
        };
        self.dispatch_pointerdown(&mut down.clone());
        let mut mv = PtrEvent {
            client_x: start.0 + dx,
            client_y: start.1 + dy,
            target: EvTarget::Window,
            ..down.clone()
        };
        self.dispatch_pointermove(&mut mv, true);
        if value.get("escape") == Some(&Value::Bool(true)) {
            let mut esc = KeyEv {
                target: EvTarget::Window,
                cancelable: false,
                ..KeyEv::new("Escape", false, false, false)
            };
            self.dispatch_keydown(&mut esc);
        }
        let mut up = PtrEvent {
            client_x: start.0 + dx,
            client_y: start.1 + dy,
            target: EvTarget::Window,
            ..down
        };
        self.dispatch_pointerup(&mut up, true, false);
    }

    /// R:2103 `driveDirectType()`: `type {text}` into the open editor (no commit).
    pub fn drive_direct_type(&mut self, contents: Option<&str>) {
        let Some(value) = self.drive_wanted(contents) else {
            return;
        };
        let Some(text) = value
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return;
        };
        let Some(active) = self.direct_active.as_mut() else {
            return;
        };
        let editor = &mut active.editor;
        if editor.tag == EditorTag::Select {
            let Some((option, _)) = editor
                .options
                .iter()
                .find(|(v, label)| *label == text || *v == text)
                .cloned()
            else {
                self.dom.set_text(ids::FIELDS_STATUS_EL, "not an option");
                return;
            };
            editor.value = option;
        } else {
            let len = super::fill::utf16_len(&editor.value);
            let start = editor.selection_start.unwrap_or(len);
            let end = editor.selection_end.unwrap_or(start);
            editor.value = super::editors::splice_utf16(&editor.value, start, end, &text);
            let caret = start.min(len) + super::fill::utf16_len(&text);
            editor.selection_start = Some(caret);
            editor.selection_end = Some(caret);
            // maxlength does not stop setRangeText; §8.14 truncates and commits that one value.
            if editor.max_length > 0
                && super::fill::utf16_len(&editor.value) as i64 > editor.max_length
            {
                editor.value =
                    super::editors::truncate_utf16(&editor.value, editor.max_length as usize);
                let end = super::fill::utf16_len(&editor.value);
                editor.selection_start = Some(end);
                editor.selection_end = Some(end);
            }
        }
        self.dispatch_editor_input();
    }

    /// R:2114 `driveDirectKey()`: `key {key}` with `Alt+`, `Meta+`, `Shift+` prefixes.
    pub fn drive_direct_key(&mut self, contents: Option<&str>) {
        self.begin_event();
        let Some(value) = self.drive_wanted(contents) else {
            return;
        };
        let Some(mut key) = value.get("key").and_then(Value::as_str).map(str::to_string) else {
            return;
        };
        let had_editor = self.direct_active.is_some();
        let (mut alt, mut meta, mut shift) = (false, false, false);
        for (prefix, flag) in [
            ("Alt+", &mut alt),
            ("Meta+", &mut meta),
            ("Shift+", &mut shift),
        ] {
            if let Some(rest) = key.strip_prefix(prefix) {
                *flag = true;
                key = rest.to_string();
            }
        }
        let translated = if key == "Space" {
            " ".to_string()
        } else {
            key.clone()
        };
        let keydown = |target: EvTarget, cancelable: bool| KeyEv {
            target,
            cancelable,
            ..KeyEv::new(&translated, shift, alt, meta)
        };
        // P5 U3 §9.12: Escape first closes an open sign or verify dialog.
        let cert_open = self.dom.is_open(ids::CERT_SIGN_DIALOG);
        let verify_open = self.dom.is_open(ids::VERIFY_DIALOG);
        if key == "Escape" && (cert_open || verify_open) {
            // `dialog.close()` runs each dialog's close listener.
            self.close_dialog(ids::CERT_SIGN_DIALOG);
            self.close_dialog(ids::VERIFY_DIALOG);
            return;
        }
        // P6 U7 §10.35: ⌘C/⌘X/⌘V are the Edit-menu roles: a DOM clipboard event (never a keydown).
        let chord = match key.as_str() {
            "c" => Some("copy"),
            "x" => Some("cut"),
            "v" => Some("paste"),
            _ => None,
        };
        if meta
            && !alt
            && !shift
            && let Some(kind) = chord
        {
            self.on_clipboard_event(kind);
            return;
        }
        // P6 U5 §9.17: an open object menu owns every key; Escape closes it.
        if !self.dom.hidden(ids::OBJECT_MENU) {
            if key == "Escape" {
                self.close_object_menu(true);
                return;
            }
            let target = match self.lane2.focus {
                Focus::MenuItem(name) => EvTarget::MenuItem(name),
                _ => EvTarget::Menu,
            };
            let mut ev = keydown(target, true);
            self.dispatch_keydown(&mut ev);
            return;
        }
        // draw-signature D5: while armed, Enter places at the page centre and Escape disarms.
        if self.sign_armed && self.direct_active.is_none() && (key == "Enter" || key == "Escape") {
            if key == "Escape" {
                self.disarm_sign();
            } else {
                self.place_signature_at_centre();
            }
            return;
        }
        // P6 U6 §9.18: Escape or Enter leaves crop mode first (the selection stays).
        if self.direct_crop.is_some() && (key == "Escape" || key == "Enter") {
            self.leave_crop();
            return;
        }
        // §9.21: ⌥⌘←/→ on a group reaches the anchor's keydown wherever focus is.
        if alt
            && meta
            && (key == "ArrowLeft" || key == "ArrowRight")
            && self.direct_active.is_none()
            && self.direct_selection.len() >= 2
            && let Some(anchor) = self.direct_focused_object.clone()
            && let Some(index) = self.object_node(&anchor.target_id)
        {
            let mut ev = keydown(
                EvTarget::Node {
                    index,
                    part: Part::Body,
                },
                true,
            );
            self.dispatch_keydown(&mut ev);
            return;
        }
        if key == "Space"
            && let Some(item) = self
                .direct_focused_field
                .clone()
                .filter(|f| f.kind == FieldKind::Checkbox || f.kind == FieldKind::Radio)
        {
            let value = if item.kind == FieldKind::Checkbox {
                FieldValue::Bool(item.value != FieldValue::Bool(true))
            } else {
                FieldValue::Bool(true)
            };
            let page = self.wanted_page;
            self.commit_field(&item, value, page as usize);
            return;
        }
        if (key == "ArrowLeft" || key == "ArrowRight")
            && !alt
            && let Some(item) = self
                .direct_focused_field
                .clone()
                .filter(|f| f.kind == FieldKind::Radio)
        {
            self.move_radio(&item, if key == "ArrowRight" { 1 } else { -1 });
            return;
        }
        if key == "Tab" {
            let page = self
                .direct_active
                .as_ref()
                .map_or(self.wanted_page, |a| a.page);
            let index = self.direct_field_index;
            if self.direct_active.as_ref().is_some_and(|a| {
                matches!(
                    a.kind,
                    DirectKind::Add | DirectKind::Link | DirectKind::Textbox | DirectKind::Note
                )
            }) {
                self.finish_direct();
                return;
            }
            if self.direct_active.is_some() {
                self.finish_direct();
            }
            self.focus_next_field(shift, page, index);
            return;
        }
        // §9.7: a focused object or redaction mark owns the key (Delete, Enter, ⌥-arrows).
        if self.direct_active.is_none()
            && key != "Escape"
            && let Some(index) = self.focused_node()
        {
            let many = self.direct_selection.len() >= 2;
            let owns = match &self.lane2.hit.nodes[index].kind {
                NodeKind::Object { .. }
                | NodeKind::Mark { .. }
                | NodeKind::TextBox { .. }
                | NodeKind::Signature { .. }
                | NodeKind::Link { .. }
                | NodeKind::SigField { .. } => true,
                NodeKind::Text { .. } | NodeKind::Field { .. } => many,
                _ => false,
            };
            if owns {
                let mut ev = keydown(
                    EvTarget::Node {
                        index,
                        part: Part::Body,
                    },
                    true,
                );
                self.dispatch_keydown(&mut ev);
                return;
            }
        }
        if alt && super::consts::ARROW_KEYS.contains(&key.as_str()) {
            let item = match self.direct_active.as_ref() {
                Some(a) if a.kind == DirectKind::Field => match &a.item {
                    DirectItem::Field(f) => Some(f.clone()),
                    _ => None,
                },
                _ => self.direct_focused_field.clone(),
            };
            if let Some(item) = item {
                self.nudge_field(&item, &key, shift, meta);
                return;
            }
        }
        let multiline_kind = self.direct_active.as_ref().is_some_and(|a| {
            a.paragraph.is_some() || matches!(a.kind, DirectKind::Add | DirectKind::Note)
        });
        // §9.7 plain Enter is a newline in the add and paragraph editors.
        if key == "Enter" && !meta && had_editor && multiline_kind {
            self.drive_direct_type(Some("{\"text\":\"\\n\"}"));
            return;
        }
        if key == "Enter" && (meta || !multiline_kind) {
            if self.direct_active.is_some() {
                self.finish_direct();
            }
            return;
        }
        if key == "Enter" && self.direct_active.is_none() {
            return;
        }
        // X6: Escape with no editor open stops an armed tool; it peels one layer at a time.
        if key == "Escape"
            && self.direct_active.is_none()
            && !self.has_direct_focus()
            && self.armed_tool().is_some()
        {
            self.disarm_tools();
            return;
        }
        // Review fix 8: Escape keeps a note that has text (an empty one is discarded).
        if key == "Escape"
            && let Some(active) = self
                .direct_active
                .clone()
                .filter(|a| a.kind == DirectKind::Note)
            && !self.direct_unchanged(&active)
        {
            self.finish_direct();
            return;
        }
        if key == "Escape" {
            if self
                .direct_active
                .as_ref()
                .is_some_and(|a| a.kind == DirectKind::Add)
            {
                self.clear_error();
            }
            if self.direct_active.is_some() {
                self.close_direct();
            }
            self.direct_focused_field = None;
            self.drop_direct_focus();
            return;
        }
        let target = if had_editor && self.direct_active.is_some() {
            EvTarget::Editor
        } else {
            self.key_focus_target()
        };
        let mut ev = keydown(target, false);
        self.dispatch_keydown(&mut ev);
        if key == "Space"
            && let Some(active) = self.direct_active.clone()
            && active.kind == DirectKind::Field
            && let DirectItem::Field(item) = &active.item
            && (item.kind == FieldKind::Checkbox || item.kind == FieldKind::Radio)
        {
            let value = if item.kind == FieldKind::Checkbox {
                FieldValue::Bool(item.value != FieldValue::Bool(true))
            } else {
                FieldValue::Bool(true)
            };
            let page = self.wanted_page;
            self.commit_field(item, value, page as usize);
        }
    }

    /// R:2340 `redact-escape`: a real Escape keydown on the focused element (else the body),
    /// through the window's capture listener, the target's own, `#page` and the document's.
    pub(crate) fn drive_escape_keydown(&mut self) {
        self.begin_event();
        let target = self.key_focus_target();
        let mut ev = KeyEv {
            target,
            cancelable: true,
            ..KeyEv::new("Escape", false, false, false)
        };
        self.dispatch_keydown(&mut ev);
    }

    /// `document.activeElement` as an event target.
    fn key_focus_target(&self) -> EvTarget {
        match &self.lane2.focus {
            Focus::Node(key) => {
                self.lane2
                    .hit
                    .find(key)
                    .map_or(EvTarget::Outside, |index| EvTarget::Node {
                        index,
                        part: Part::Body,
                    })
            }
            Focus::Editor(_) if self.direct_active.is_some() => EvTarget::Editor,
            Focus::Page => EvTarget::Page,
            Focus::Menu => EvTarget::Menu,
            Focus::MenuItem(name) => EvTarget::MenuItem(name),
            _ => EvTarget::Outside,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_items_map_drive_names() {
        assert_eq!(
            MENU_ITEMS
                .iter()
                .find(|(k, _)| *k == "rotate-ccw")
                .map(|(_, v)| *v),
            Some("rotateCcw")
        );
        assert_eq!(MENU_ITEMS.len(), super::super::object_menu::MENU_KEYS.len());
    }
}
