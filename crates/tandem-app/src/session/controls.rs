//! The controls (renderer.ts `wireControls`, R:10774-11431): every listener with logic of
//! its own becomes an `on_*` method taking the DOM event's data as plain arguments (page
//! gestures in page points, y down); a listener that only calls another function is
//! reached through the id dispatchers in `events.rs`. Pure presentation (focus, scroll,
//! pointer capture, `preventDefault` bookkeeping) is the UI's: the `bool` an `on_*` key or
//! pointer method returns is "handled" (the TS called `preventDefault`).
//!
//! The MutationObserver that kept the tool chip in step with every tool's `aria-pressed`
//! (R:10798) becomes [`Session::set_tool_pressed`], which syncs the chip after the write.

use super::Session;
use super::consts::{ADD_ROTATED_TEXT, DRAW_LINK_PROMPT, DRAW_SIGFIELD_PROMPT};
use super::dom_ids::*;
use super::types::*;
use crate::messages;
use serde_json::{Value, json};
use tandem_protocol::limits::{MAX_INK_POINTS, NOTE_SIZE};

/// The buttons whose `aria-pressed` the tool chip follows (R:10799).
const CHIP_BUTTONS: [&str; 9] = [
    NOTE_ADD_BUTTON,
    FREE_TEXT_ADD_BUTTON,
    INK_DRAW_BUTTON,
    LINK_DRAW_BUTTON,
    SIGFIELD_DRAW_BUTTON,
    CONTENT_ADD_ARM_BUTTON,
    TEXT_ADD_TOOL_BUTTON,
    TEXTBOX_TOOL_BUTTON,
    FILL_SIGN_BUTTON,
];

/// Every `<dialog>` of index.html (`document.querySelector("dialog[open]")`).
pub(crate) const DIALOGS: [&str; 21] = [
    NOTE_DIALOG,
    HELP_DIALOG,
    redact_ui::CONFIRM_DIALOG,
    redact_ui::FIND_DIALOG,
    redact_ui::PENDING_DIALOG,
    redact_ui::ASK_DIALOG,
    STAMP_ICON_DIALOG,
    SIGPAD_DIALOG,
    STAMP_DIALOG,
    LABELS_DIALOG,
    EXPORT_DIALOG,
    INSERT_DIALOG,
    EXTRACT_DIALOG,
    BOOKMARK_DIALOG,
    LINK_DIALOG,
    PRINT_DIALOG,
    VERIFY_DIALOG,
    BATCH_DIALOG,
    SIGN_CHOICE_DIALOG,
    CERT_SIGN_DIALOG,
    SECURITY_DIALOG,
];

/// Which drag `startAreaDrag` (R:8133) runs from a page press, standing for its `onRect`
/// and `onEnd` callbacks: [`Session::area_drag_rect`] and [`Session::area_drag_end`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AreaDrag {
    /// "Draw box" armed: a redaction box, then the tool disarms.
    RedactArmed,
    /// "Draw link" armed.
    Link,
    /// "Draw signature field" armed.
    SignatureField,
    /// Redact mode, unarmed, on empty page space: a redaction box; the tool stays.
    Redact,
}

/// JS `x.toFixed(1)`: ties go to the larger value (`(0.25).toFixed(1)` is "0.3").
pub(crate) fn to_fixed1(x: f64) -> String {
    let scaled = x * 10.0;
    let floor = scaled.floor();
    let n = if scaled - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    };
    let text = format!("{:.1}", n / 10.0);
    if text == "-0.0" { "0.0".into() } else { text }
}

impl Session {
    /// `button.getAttribute("aria-pressed") === "true"` (`pressed`, R:6944).
    pub(crate) fn is_pressed(&self, id: &str) -> bool {
        self.dom.attr(id, "aria-pressed").as_deref() == Some("true")
    }

    /// `button.setAttribute("aria-pressed", …)`, plus what the chip's MutationObserver did.
    pub(crate) fn set_tool_pressed(&mut self, id: &str, on: bool) {
        self.dom
            .set_attr(id, "aria-pressed", if on { "true" } else { "false" });
        self.dom.set_checked(id, on);
        if CHIP_BUTTONS.contains(&id) {
            self.sync_tool_chip();
        }
    }

    /// `!!document.querySelector("dialog[open]")`.
    pub fn any_dialog_open(&self) -> bool {
        DIALOGS.iter().any(|id| self.dom.is_open(id))
    }

    fn shown_page_index(&self) -> usize {
        self.wanted_page.max(0) as usize
    }

    fn wanted_size(&self) -> Option<PageSize> {
        self.doc
            .as_ref()?
            .pages
            .get(self.shown_page_index())
            .cloned()
    }

    /// R:10777 "Add text" (Edit text): a pointer press arms (until Esc or a second press);
    /// keyboard activation opens the editor at once, a third across and half down the page.
    pub fn on_text_add_tool_click(&mut self, keyboard: bool) {
        let Some(size) = self.wanted_size() else {
            return;
        };
        if !self.can_edit() {
            return;
        }
        if keyboard {
            let page = self.shown_page_index();
            let rotation = self
                .doc
                .as_ref()
                .and_then(|d| d.rotations.get(page).copied())
                .unwrap_or(0);
            if rotation % 360 != 0 {
                self.show_error(ADD_ROTATED_TEXT);
                return;
            }
            self.open_add_text(page, [size.width / 3.0, size.height / 2.0]);
            return;
        }
        let was = self.is_pressed(TEXT_ADD_TOOL_BUTTON);
        self.cancel_pending_tools();
        self.set_tool_pressed(TEXT_ADD_TOOL_BUTTON, !was);
        self.paint_hits_now();
    }

    /// R:10790 "Add text box" (Fill forms): keyboard activation adds one at the page centre.
    pub fn on_textbox_tool_click(&mut self, keyboard: bool) {
        if keyboard {
            self.add_text_box_at_centre();
            return;
        }
        let was = self.is_pressed(TEXTBOX_TOOL_BUTTON);
        self.cancel_pending_tools();
        self.set_tool_pressed(TEXTBOX_TOOL_BUTTON, !was);
        self.paint_hits_now();
    }

    /// R:10809: a press on a disabled Undo/Redo still blurs the editor: close it, no request.
    pub fn on_undo_redo_disabled_press(&mut self) {
        if self.direct_active.is_some() {
            self.close_direct();
            self.direct_focused_field = None;
        }
    }

    /// R:10813 "Add note": keyboard activation carries no click point, so the note goes to
    /// the page centre; a pointer click arms press-then-click-on-the-page.
    pub fn on_note_add_click(&mut self, keyboard: bool) {
        if keyboard {
            if let Some(size) = self.wanted_size()
                && self.can_annotate()
            {
                let half = NOTE_SIZE / 2.0;
                self.open_note_editor(
                    self.shown_page_index(),
                    [size.width / 2.0 - half, size.height / 2.0 - half],
                );
            }
            return;
        }
        let pressed = self.is_pressed(NOTE_ADD_BUTTON);
        self.set_tool_pressed(NOTE_ADD_BUTTON, !pressed);
        self.announce(if pressed {
            ""
        } else {
            "Add note: click the page where the note should go"
        });
        self.paint_hits_now();
    }

    /// R:10832 Annotate's "Add text box": keyboard activation opens the dialog at the page
    /// centre; a pointer click arms press-then-click-on-the-page.
    pub fn on_free_text_add_click(&mut self, keyboard: bool) {
        if keyboard {
            if let Some(size) = self.wanted_size() {
                self.open_text_box_dialog([size.width / 2.0, size.height / 2.0]);
            }
            return;
        }
        let pressed = self.is_pressed(FREE_TEXT_ADD_BUTTON);
        self.set_tool_pressed(FREE_TEXT_ADD_BUTTON, !pressed);
        self.announce(if pressed {
            ""
        } else {
            "Add text box: click the page where the box should go"
        });
        self.paint_hits_now();
    }

    /// R:10845 "Square" / "Circle" / "Line": over the current match or at the page centre.
    pub fn on_shape_click(&mut self, kind: &str, announcement: &str) {
        if let Some(rect) = self.shape_box() {
            self.add_shape(kind, rect, announcement);
        }
    }

    /// R:10851 "Draw ink": keyboard activation draws the stand-in zigzag; a pointer click
    /// arms one stroke.
    pub fn on_ink_draw_click(&mut self, keyboard: bool) {
        let pressed = self.is_pressed(INK_DRAW_BUTTON);
        self.set_tool_pressed(INK_DRAW_BUTTON, !pressed);
        if keyboard {
            self.set_tool_pressed(INK_DRAW_BUTTON, false);
            self.add_shape_op("ink");
            return;
        }
        self.announce(if pressed {
            ""
        } else {
            "Draw ink: drag on the page to draw one stroke"
        });
        self.paint_hits_now();
    }

    /// R:10864 "Draw box" (Redact).
    pub fn on_redact_area_click(&mut self, keyboard: bool) {
        if keyboard {
            if let Some(rect) = self.shape_box() {
                self.add_redact_area(self.shown_page_index(), rect, messages::redact::ADDED);
            }
            return;
        }
        let pressed = self.is_pressed(REDACT_AREA_BUTTON);
        self.set_tool_pressed(REDACT_AREA_BUTTON, !pressed);
        self.announce(if pressed {
            ""
        } else {
            messages::redact::DRAW_HINT
        });
        self.paint_hits_now();
    }

    /// R:10886: a Comments row's "edit" / "delete" button.
    pub fn on_note_list_action(&mut self, action: &str, annot_id: i64) {
        if action == "edit" {
            self.open_note_dialog(annot_id);
            return;
        }
        let redact = self
            .notes
            .iter()
            .find(|note| note.id == annot_id)
            .is_some_and(|n| n.r#type == "Redact");
        self.delete_note(
            annot_id,
            if redact {
                "Deleted redaction mark"
            } else {
                "Deleted note"
            },
        );
    }

    /// R:10915 / R:10922 "Draw link" and "Draw signature field".
    pub fn on_draw_tool_click(&mut self, id: &str) {
        let prompt = if id == LINK_DRAW_BUTTON {
            DRAW_LINK_PROMPT
        } else {
            DRAW_SIGFIELD_PROMPT
        };
        let pressed = self.is_pressed(id);
        self.cancel_pending_tools();
        self.set_tool_pressed(id, !pressed);
        self.announce(if pressed { "" } else { prompt });
        self.paint_hits_now();
    }

    /// R:11007: a bookmark row's "go" / "rename" / "delete".
    pub fn on_outline_action(&mut self, action: &str, outline_id: i64) {
        match action {
            "go" => {
                let entry = self
                    .outline
                    .iter()
                    .find(|e| e.id == outline_id)
                    .cloned()
                    .unwrap_or(OutlineEntry {
                        id: outline_id,
                        title: String::new(),
                        depth: 0,
                        page: None,
                        uri: None,
                    });
                self.follow_outline(&entry);
            }
            "rename" => self.open_bookmark_dialog(Some(outline_id)),
            "delete" => self.delete_bookmark(outline_id),
            _ => {}
        }
    }

    /// R:11017: a link row's "delete" or follow.
    pub fn on_link_list_action(&mut self, action: &str, link_id: i64) {
        let Some(item) = self.links.iter().find(|entry| entry.id == link_id).cloned() else {
            return;
        };
        if action == "delete" {
            self.delete_link(link_id);
        } else {
            self.follow_link(&item);
        }
    }

    /// `isMemberNode(node)`: the target is a member of the object selection.
    fn is_selection_member(&self, member: &ObjectFocus) -> bool {
        self.direct_selection
            .iter()
            .any(|m| m.target_id == member.target_id && m.kind == member.kind)
    }

    /// R:11032 (capture keydown on the hit layer): `member` is the page target the key landed
    /// on (`memberOfNode`), `object_target` whether it is an image/form object; `corner` its
    /// client top-left (where the object menu opens).
    #[allow(clippy::too_many_arguments)]
    pub fn on_hit_target_keydown(
        &mut self,
        member: Option<&ObjectFocus>,
        object_target: bool,
        key: &str,
        shift: bool,
        meta: bool,
        ctrl: bool,
        corner: (f64, f64),
    ) -> bool {
        if self.direct_active.is_some() || self.direct_gesture.is_some() {
            return false;
        }
        let Some(member) = member else {
            return false;
        };
        if ((key == "F10" && shift) || key == "ContextMenu") && !object_target {
            if !self.is_selection_member(member) {
                self.clear_direct_focus();
                self.toggle_member(member);
            }
            self.open_object_menu(corner.0, corner.1, "selection", None);
            return true;
        }
        if self.direct_selection.len() < 2
            || !self.selection_mixed()
            || !self.is_selection_member(member)
        {
            return false;
        }
        if key == "Escape" {
            self.drop_direct_focus();
            return true;
        }
        if super::consts::ARROW_KEYS.contains(&key) && !meta && !ctrl {
            self.nudge_selection(key, shift);
        } else if key == "Delete" || key == "Backspace" {
            self.delete_selection();
        } else {
            return false;
        }
        true
    }

    /// R:11045 (capture pointerdown on the hit layer): a mixed selection of two or more moves
    /// as one when one of its members is dragged.
    pub fn on_hit_target_pointerdown(
        &mut self,
        member: Option<&ObjectFocus>,
        point: Point,
        button: i64,
        ctrl: bool,
        shift: bool,
        on_handle: bool,
    ) -> bool {
        if button != 0 || ctrl || shift || self.direct_active.is_some() || on_handle {
            return false;
        }
        let Some(member) = member else {
            return false;
        };
        if self.direct_selection.len() < 2
            || !self.selection_mixed()
            || !self.is_selection_member(member)
        {
            return false;
        }
        self.start_selection_gesture(point, member);
        true
    }

    /// R:11051 (§2.1/§9.27): a right-click (or Control-click) on a run, field, link or mark
    /// selects it alone unless it is a member, and opens the object menu at the pointer.
    pub fn on_hit_target_context_menu(
        &mut self,
        member: Option<&ObjectFocus>,
        object_target: bool,
        client: (f64, f64),
    ) -> bool {
        if object_target || self.direct_active.is_some() {
            return false;
        }
        let Some(member) = member else {
            return false;
        };
        if !self.is_selection_member(member) {
            self.clear_direct_focus();
            self.toggle_member(member);
        }
        self.open_object_menu(client.0, client.1, "selection", None);
        true
    }

    /// R:11061 (§2.1/§9.28): right-click on empty page space opens the page menu (only Paste
    /// can be enabled); it pastes there. `in_menu`/`on_target`: the press was inside the
    /// object menu / on a hit-layer target.
    pub fn on_page_context_menu(
        &mut self,
        client: (f64, f64),
        point: Option<Point>,
        in_menu: bool,
        on_target: bool,
    ) -> bool {
        if in_menu || on_target {
            return false;
        }
        let modes = [
            UiMode::Edit,
            UiMode::Forms,
            UiMode::Annotate,
            UiMode::Redact,
        ];
        if self.doc.is_none() || !modes.contains(&self.ui_mode) || !self.layer_live() {
            return false;
        }
        self.drop_direct_focus();
        self.add_blocked = true;
        let at = self
            .drive_menu_point
            .clone()
            .or_else(|| point.and_then(|p| self.page_point(p)).map(|p| p.to_vec()));
        self.open_object_menu(client.0, client.1, "page", at);
        true
    }

    /// R:11070 (§10.30): the Edit menu's Cut/Copy/Paste and the chords reach the page
    /// selection. `active` is the focused element `{tag, type, contentEditable, inlineEditor}`
    /// (None = the body), `collapsed` whether the window's text selection is collapsed.
    pub fn on_clipboard(&mut self, kind: &str, active: Option<Value>, collapsed: bool) -> bool {
        let ctx = json!({
            "mode": self.ui_mode.as_str(), "layerLive": self.layer_live(), "dialogOpen": self.any_dialog_open(),
            "menuOpen": !self.dom.hidden(OBJECT_MENU), "editorOpen": self.direct_active.is_some(),
            "active": active, "collapsed": collapsed, "selection": self.direct_selection.len(),
            "clipboard": self.clipboard_info.is_some(), "editable": self.editable(),
        });
        if !self.clipboard_chord_ok(kind, &ctx) {
            return false;
        }
        self.clipboard_work(kind, None);
        true
    }

    /// R:11081: a click on the page, in page points (None when off the canvas). The order is
    /// the TS's: armed signature, add tools, baseline pick, text box, note.
    pub fn on_page_click(
        &mut self,
        point: Option<Point>,
        button: i64,
        ctrl: bool,
        shift: bool,
        in_menu: bool,
    ) {
        if in_menu {
            return; // §9.15
        }
        let point = point.and_then(|p| self.page_point(p));
        // draw-signature D5: an armed Sign outranks every other click on the page.
        if self.sign_armed && button == 0 && !ctrl {
            let Some(point) = point else {
                return;
            };
            self.add_blocked = false;
            self.place_signature(self.shown_page_index(), point.to_vec(), None);
            return;
        }
        if self.click_to_add(point, button, ctrl, shift) {
            return;
        }
        if self.is_pressed(CONTENT_ADD_ARM_BUTTON) {
            let Some(point) = point else {
                return;
            };
            self.content_add_point = Some(point);
            self.set_tool_pressed(CONTENT_ADD_ARM_BUTTON, false);
            self.announce(&format!(
                "Text baseline picked at {}, {} pt.",
                to_fixed1(point[0]),
                to_fixed1(point[1])
            ));
            return;
        }
        if self.is_pressed(FREE_TEXT_ADD_BUTTON) {
            let Some(point) = point else {
                return;
            };
            self.set_tool_pressed(FREE_TEXT_ADD_BUTTON, false);
            self.open_text_box_dialog(point);
            return;
        }
        if !self.is_pressed(NOTE_ADD_BUTTON) {
            return;
        }
        let Some(point) = point else {
            return;
        };
        self.set_tool_pressed(NOTE_ADD_BUTTON, false);
        // Item 6: the editor opens at once.
        if self.can_annotate() {
            self.open_note_editor(self.shown_page_index(), point);
        }
    }

    /// R:11112: a press on the page. `on_target`: on a hit-layer target or handle. Returns
    /// whether the press started a gesture (the TS captured the pointer / prevented it).
    pub fn on_page_pointerdown(
        &mut self,
        point: Point,
        button: i64,
        ctrl: bool,
        in_menu: bool,
        on_target: bool,
    ) -> bool {
        if in_menu {
            return false; // §9.15
        }
        // §9.9: a press with an editor open only commits/closes it; the click after it adds
        // nothing. P6 U5 §9.22: so does the press that only dismissed the object menu or crop.
        self.add_blocked = self.direct_active.is_some()
            || self.direct_committing
            || self.object_menu_dismiss
            || self.crop_dismiss
            || self.direct_crop.is_some();
        // P6 U7 review F2: a secondary or Control-click never starts an armed tool's gesture.
        if button != 0 || ctrl {
            return false;
        }
        let page = self.shown_page_index();
        let armed = [
            (REDACT_AREA_BUTTON, AreaDrag::RedactArmed),
            (LINK_DRAW_BUTTON, AreaDrag::Link),
            (SIGFIELD_DRAW_BUTTON, AreaDrag::SignatureField),
        ];
        for (id, drag) in armed {
            if self.is_pressed(id) {
                self.start_area_drag(point, page, drag);
                return true;
            }
        }
        if self.ui_mode == UiMode::Redact && self.can_redact_draw() && !on_target {
            self.start_area_drag(point, page, AreaDrag::Redact);
            return true;
        }
        if !self.is_pressed(INK_DRAW_BUTTON) {
            return false;
        }
        let Some(point) = self.ink_point(point) else {
            return false;
        };
        self.ink_stroke = Some(vec![point]);
        true
    }

    /// `startAreaDrag`'s `onRect` for each armed tool (R:11120-11134).
    pub fn area_drag_rect(&mut self, drag: AreaDrag, page: usize, rect: Vec<f64>) {
        match drag {
            AreaDrag::RedactArmed | AreaDrag::Redact => {
                self.add_redact_area(page, rect, messages::redact::ADDED);
            }
            AreaDrag::Link => self.drawn_link(page, rect),
            AreaDrag::SignatureField => self.add_signature_field(page, rect),
        }
    }

    /// `startAreaDrag`'s `onEnd`: an armed tool disarms after one drag.
    pub fn area_drag_end(&mut self, drag: AreaDrag) {
        match drag {
            AreaDrag::RedactArmed => {
                self.set_tool_pressed(REDACT_AREA_BUTTON, false);
                self.paint_hits_now();
            }
            AreaDrag::Link | AreaDrag::SignatureField => {
                let id = if drag == AreaDrag::Link {
                    LINK_DRAW_BUTTON
                } else {
                    SIGFIELD_DRAW_BUTTON
                };
                self.set_tool_pressed(id, false);
                self.announce("");
                self.paint_hits_now();
            }
            AreaDrag::Redact => {}
        }
    }

    /// R:11145: ink by pointer: a drag sample is only a new point when it moved, bounded like
    /// the engine's check.
    pub fn on_page_pointermove(&mut self, point: Point) {
        if self.ink_stroke.is_none() {
            return;
        }
        let Some(point) = self.ink_point(point) else {
            return;
        };
        let Some(stroke) = self.ink_stroke.as_mut() else {
            return;
        };
        let last = stroke[stroke.len() - 1];
        if (point[0] - last[0]).abs() < 0.5 && (point[1] - last[1]).abs() < 0.5 {
            return;
        }
        if stroke.len() >= MAX_INK_POINTS {
            return;
        }
        stroke.push(point);
    }

    /// R:11155 `finishInk` (pointerup / pointercancel).
    pub fn on_page_pointerup(&mut self) {
        let Some(stroke) = self.ink_stroke.take() else {
            return;
        };
        self.set_tool_pressed(INK_DRAW_BUTTON, false);
        if stroke.len() < 2 {
            self.announce("Ink needs a drag, not a click");
            return;
        }
        let announcement = format!("Added ink ({} points)", stroke.len());
        let strokes = vec![stroke.iter().map(|p| p.to_vec()).collect()];
        self.add_ink(strokes, &announcement);
    }

    /// R:11197 a tab's click: select it; a sidebar tab also brings a collapsed sidebar back;
    /// the Text tab rescans content.
    pub fn on_tab_click(&mut self, id: &str) -> bool {
        if let Some(index) = SIDEBAR_TABS.iter().position(|t| *t == id) {
            self.select_tab(&SIDEBAR_TABS, &SIDEBAR_PANELS, index);
            if !self.sidebar_open {
                self.set_thumb_sidebar(true);
            }
            return true;
        }
        if let Some(index) = INSPECTOR_TABS.iter().position(|t| *t == id) {
            self.select_tab(&INSPECTOR_TABS, &INSPECTOR_PANELS, index);
            if id == "tab-text" {
                self.refresh_content_targets(false);
            }
            return true;
        }
        false
    }

    /// R:11204: arrows / Home / End on a tab move and select (WAI-ARIA automatic
    /// activation). Returns the tab to focus.
    pub fn on_tab_keydown(&mut self, id: &str, key: &str) -> Option<&'static str> {
        let (tabs, panels, index): (&[&'static str], &[&'static str], usize) =
            if let Some(i) = SIDEBAR_TABS.iter().position(|t| *t == id) {
                (&SIDEBAR_TABS, &SIDEBAR_PANELS, i)
            } else {
                let i = INSPECTOR_TABS.iter().position(|t| *t == id)?;
                (&INSPECTOR_TABS, &INSPECTOR_PANELS, i)
            };
        let n = tabs.len();
        let next = match key {
            "ArrowRight" | "ArrowDown" => (index + 1) % n,
            "ArrowLeft" | "ArrowUp" => (index + n - 1) % n,
            "Home" => 0,
            "End" => n - 1,
            _ => return None,
        };
        self.select_tab(tabs, panels, next);
        let inspector = tabs.len() == INSPECTOR_TABS.len();
        if inspector && tabs[next] == "tab-text" {
            self.refresh_content_targets(false);
        }
        Some(tabs[next])
    }

    /// R:11228 "Pick baseline point".
    pub fn on_content_add_arm_click(&mut self) {
        let armed = self.is_pressed(CONTENT_ADD_ARM_BUTTON);
        self.set_tool_pressed(CONTENT_ADD_ARM_BUTTON, !armed);
        if armed {
            self.content_add_point = None;
            self.announce("Text placement cancelled.");
        } else {
            self.announce("Click the page to choose the text baseline.");
        }
        self.paint_hits_now();
    }

    /// R:11237: keys in the candidates listbox.
    pub fn on_content_targets_keydown(&mut self, key: &str) -> bool {
        let count = self
            .content_scan
            .as_ref()
            .map_or(0, |scan| scan.items.len()) as i64;
        if count == 0 {
            return false;
        }
        let step = match key {
            "ArrowDown" => 1,
            "ArrowUp" => -1,
            _ => 0,
        };
        if step != 0 {
            let row = (self.content_index + step).max(0).min(count - 1);
            self.select_content_row(row as usize);
            return true;
        }
        if key == "Home" || key == "End" {
            self.select_content_row(if key == "Home" {
                0
            } else {
                (count - 1) as usize
            });
            return true;
        }
        if key == "Enter" || key == " " {
            let row = if self.content_index < 0 {
                0
            } else {
                self.content_index as usize
            };
            self.activate_content_row(row);
            return true;
        }
        false
    }

    /// R:11262 / R:11275: Escape in the content text form closes it; in the object form it
    /// leaves crop mode first (P6 U6 review R3).
    pub fn on_content_form_escape(&mut self, object_form: bool) {
        if !object_form {
            self.close_content_form();
        } else if self.direct_crop.is_some() {
            self.leave_crop();
        } else {
            self.close_content_object_form();
        }
    }

    /// R:11299: the page number field's `change`: go to the page (the UI then leaves the
    /// field, so the Page menu's chords reach the menu).
    pub fn on_page_input_change(&mut self) {
        let page = super::decorate::js_number_of(&self.dom.value(PAGE_INPUT)) - 1.0;
        if page.is_finite() {
            self.go_to(page.trunc() as i64);
        }
    }

    /// R:11311: Enter / Shift+Enter in the search field.
    pub fn on_search_enter(&mut self, shift: bool) {
        if !self.search_hits.is_empty() {
            self.move_hit(if shift { -1 } else { 1 });
        } else {
            self.start_search();
        }
    }

    /// R:11320 (§9.14): PageUp/PageDown turn A (B follows), Escape leaves compare.
    /// `in_page_b`: the key came from the "Page of the other file" field.
    pub fn on_compare_keydown(&mut self, key: &str, in_page_b: bool) -> bool {
        if in_page_b {
            return false;
        }
        match key {
            "PageUp" => self.go_to(self.wanted_page - 1),
            "PageDown" => self.go_to(self.wanted_page + 1),
            "Escape" => self.close_compare(),
            _ => return false,
        }
        true
    }

    /// R:11331: "Page of the other file"; §9.19(g): an emptied field reverts.
    pub fn on_compare_page_b_change(&mut self) {
        let text = self.dom.value(COMPARE_PAGE_BINPUT);
        let page = super::decorate::js_number_of(&text);
        if !crate::js::js_trim(&text).is_empty() && page.is_finite() && page.fract() == 0.0 {
            self.set_compare_page_b(page as i64 - 1);
        } else if let Some(compare) = &self.compare_doc {
            let value = (compare.page_b + 1).to_string();
            self.dom.set_value(COMPARE_PAGE_BINPUT, value);
        }
    }

    /// R:11336: keys on the page region. `in_hit_layer`: the key came from a page target or
    /// an inline editor (their arrows are theirs); `corner`: the view's client top-left.
    pub fn on_page_keydown(
        &mut self,
        key: &str,
        shift: bool,
        in_hit_layer: bool,
        in_menu: bool,
        corner: (f64, f64),
    ) -> bool {
        // draw-signature D5: while armed, Enter places at the page centre and Escape disarms.
        if self.sign_armed && (key == "Enter" || key == "Escape") {
            if key == "Escape" {
                self.disarm_sign();
            } else {
                self.place_signature_at_centre();
            }
            return true;
        }
        if in_hit_layer {
            return false;
        }
        // P6 U7 §9.28: Shift+F10 / the Menu key opens the page menu at the view's corner (no paste point).
        if ((key == "F10" && shift) || key == "ContextMenu") && !in_menu {
            let modes = [
                UiMode::Edit,
                UiMode::Forms,
                UiMode::Annotate,
                UiMode::Redact,
            ];
            if self.doc.is_none() || !modes.contains(&self.ui_mode) || !self.layer_live() {
                return false;
            }
            self.drop_direct_focus();
            self.open_object_menu(corner.0, corner.1, "page", None);
            return true;
        }
        let last = self.doc.as_ref().map_or(1, |d| d.page_count) - 1;
        match key {
            "PageUp" | "ArrowLeft" => self.go_to(self.wanted_page - 1),
            "PageDown" | "ArrowRight" => self.go_to(self.wanted_page + 1),
            "Home" => self.go_to(0),
            "End" => self.go_to(last),
            _ => return false,
        }
        true
    }

    /// R:11360 (review fix 10, window capture): Escape stops an armed tool before a focused
    /// page target sees the key. `handled_at_target`: `escapeHandledAt(event.target)` (an
    /// inspector edit form, a dialog or the compare view, not its page field).
    pub fn on_window_escape(&mut self, composing: bool, handled_at_target: bool) -> bool {
        if composing
            || self.dom.is_open(HELP_DIALOG)
            || self.printing
            || self.direct_active.is_some()
            || self.direct_crop.is_some()
            || self.direct_gesture.is_some()
            || !self.dom.hidden(OBJECT_MENU)
        {
            return false;
        }
        if self.any_dialog_open()
            || self.has_direct_focus()
            || self.armed_tool().is_none()
            || handled_at_target
        {
            return false;
        }
        self.disarm_tools();
        true
    }

    /// R:11370: the document's keydown. Cmd/Ctrl+Z, Shift+Cmd/Ctrl+Z and Cmd/Ctrl+S are
    /// main's menu accelerators; only keys with no menu item are handled here, so one
    /// keystroke has exactly one owner. Cmd/Ctrl+F returns true: the UI focuses and selects
    /// the search field.
    #[allow(clippy::too_many_arguments)]
    pub fn on_document_keydown(
        &mut self,
        key: &str,
        meta: bool,
        ctrl: bool,
        shift: bool,
        composing: bool,
        default_prevented: bool,
        handled_at_target: bool,
    ) -> bool {
        // §8.9: an open help dialog owns the keyboard; its native cancel closes it on Escape.
        if self.dom.is_open(HELP_DIALOG) {
            return false;
        }
        if key == "Escape" {
            if self.printing {
                self.cancel_print();
                return true;
            }
            // X6: Escape peels one layer per press: the selection, then the tool, then the
            // redaction tool itself (§2.3 step 1).
            let armed = self.armed_tool().is_some();
            if !default_prevented
                && !composing
                && self.direct_active.is_none()
                && !self.any_dialog_open()
                && (armed || self.redact_escape_layer())
                && !handled_at_target
            {
                if self.has_direct_focus() {
                    self.drop_direct_focus();
                } else if armed {
                    self.disarm_tools();
                } else {
                    self.exit_redact_tool();
                }
                return true;
            }
            return false;
        }
        if !(meta || ctrl) {
            return false;
        }
        match key.to_lowercase().as_str() {
            "t" if shift => {
                if !self.dom.disabled(THUMBS_TOGGLE) {
                    self.set_thumb_sidebar(!self.thumb_open);
                }
            }
            "o" => self.request_open(),
            "=" | "+" => self.zoom_step(1),
            "-" | "_" => self.zoom_step(-1),
            "0" => self.set_zoom(ZoomMode::FitWidth, self.fixed_zoom),
            "f" => {}
            _ => return false,
        }
        true
    }

    /// Load-time effects of `wireControls` itself (the listeners aside): the password field's
    /// bound (the same as the engine's own check) and the export dialog's DPI field.
    pub fn wire_controls(&mut self) {
        let max = tandem_protocol::limits::MAX_PASSWORD_CHARS.to_string();
        self.dom.set_attr(PASSWORD_INPUT, "maxlength", max);
        self.sync_export_dpi();
    }

    /// R:10830: the document's `selectionchange`: the markup buttons follow the text selection.
    pub fn on_selection_change(&mut self) {
        self.sync_markup_buttons();
    }

    /// R:11295: the thumbnail list scrolled.
    pub fn on_thumbs_scroll(&mut self) {
        self.pump_thumbs();
    }

    /// R:11418: the viewport resized: fit modes follow the page region (one rAF per burst
    /// in the TS; the UI coalesces).
    pub fn on_viewport_resize(&mut self) {
        self.sync_zoom();
        self.drain();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_fixed_matches_js() {
        assert_eq!(to_fixed1(0.25), "0.3");
        assert_eq!(to_fixed1(12.34), "12.3");
        assert_eq!(to_fixed1(100.0), "100.0");
        assert_eq!(to_fixed1(-0.01), "0.0");
    }

    #[test]
    fn dialogs_and_pressed() {
        let mut s = Session::new();
        assert!(!s.any_dialog_open());
        s.dom.set_open(PRINT_DIALOG, true);
        assert!(s.any_dialog_open());
        s.dom.set_attr(REDACT_AREA_BUTTON, "aria-pressed", "true");
        assert!(s.is_pressed(REDACT_AREA_BUTTON));
    }
}
