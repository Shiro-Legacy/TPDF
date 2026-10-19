//! Generic event dispatch keyed by the index.html element id, so a UI can drive every
//! control `wireControls` (R:10774-11431) and the rest of renderer.ts R:9533-end wired
//! without knowing method names:
//!
//! - [`Session::on_click`]: a pointer click (or a form's `submit`, keyed by the form's id);
//!   [`Session::on_activate`] with `keyboard: true` is keyboard activation (`event.detail ===
//!   0`), which several tool buttons treat differently.
//! - [`Session::on_input`] / [`Session::on_change`]: the element's new value (and checked
//!   state) is stored first, then the listener runs.
//! - [`Session::on_dialog_cancel`]: a `<dialog>`'s Escape (native cancel, then close).
//!
//! Each returns whether the id has a listener here. Delegated list clicks carry data and
//! have their own methods (`on_note_list_action`, `on_outline_action`, `on_link_list_action`,
//! `activate_content_row`), as do keys and page gestures (`controls.rs`). Listeners that
//! other parts of renderer.ts register (outside R:9533-end) are not dispatched here.

use super::Session;
use super::dom_ids::*;
use super::types::*;

impl Session {
    /// A pointer click on `id` (or the `submit` of the form `id`).
    pub fn on_click(&mut self, id: &str) -> bool {
        self.on_activate(id, false)
    }

    /// A click on `id`; `keyboard` is keyboard activation (`event.detail === 0`).
    pub fn on_activate(&mut self, id: &str, keyboard: bool) -> bool {
        if let Some((_, kind, _)) = MARKUP_BUTTONS.iter().find(|(button, _, _)| *button == id) {
            self.mark_selection_or_match(kind);
            return true;
        }
        if let Some((_, kind, announcement)) =
            SHAPE_BUTTONS.iter().find(|(button, _, _)| *button == id)
        {
            self.on_shape_click(kind, announcement);
            return true;
        }
        if let Some((_, op)) = PAGE_TOOL_BUTTONS.iter().find(|(button, _)| *button == id) {
            self.run_page_op(op);
            return true;
        }
        if let Some((name, button)) = CONTENT_TRANSFORM_BUTTONS
            .iter()
            .find(|(_, button)| *button == id)
        {
            self.inspector_transform(name, button);
            return true;
        }
        if SIDEBAR_TABS.contains(&id) || INSPECTOR_TABS.contains(&id) {
            return self.on_tab_click(id);
        }
        match id {
            TEXT_ADD_TOOL_BUTTON => self.on_text_add_tool_click(keyboard),
            TEXTBOX_TOOL_BUTTON => self.on_textbox_tool_click(keyboard),
            TOOL_CHIP_STOP => self.disarm_tools(),
            OPEN_BUTTON => self.request_open(),
            SAVE_BUTTON => self.save_in_place("in-place"),
            // Item 7: Save As rebinds; File ▸ Save a Copy… copies.
            SAVE_COPY_BUTTON => self.save_in_place("save-as"),
            UNDO_BUTTON => self.undo_edit(),
            REDO_BUTTON => self.redo_edit(),
            NOTE_ADD_BUTTON => self.on_note_add_click(keyboard),
            MARKUP_ALL_BUTTON => self.highlight_all_matches(),
            FREE_TEXT_ADD_BUTTON => self.on_free_text_add_click(keyboard),
            INK_DRAW_BUTTON => self.on_ink_draw_click(keyboard),
            REDACT_AREA_BUTTON => self.on_redact_area_click(keyboard),
            redact_ui::APPLY_OPEN => self.open_redact_dialog(),
            redact_ui::CONFIRM_BUTTON => self.confirm_redact(),
            STAMP_ICON_BUTTON => self.open_stamp_icon_dialog(),
            SIGN_BUTTON | CERT_SIGN_OPEN_BUTTON => self.open_sign_choice(),
            SIGN_CHOICE_PICTURE => self.on_sign_choice_picture(),
            SIGN_CHOICE_CERTIFICATE => self.on_sign_choice_certificate(),
            SIGN_CHOICE_CANCEL => {
                self.close_dialog(SIGN_CHOICE_DIALOG);
            }
            STAMP_ICON_SAVE_BUTTON => self.save_stamp_icon_dialog(),
            STAMP_ICON_CANCEL_BUTTON => {
                self.close_dialog(STAMP_ICON_DIALOG);
            }
            NOTE_SAVE_BUTTON => self.save_note_dialog(),
            NOTE_CANCEL_BUTTON => {
                self.close_dialog(NOTE_DIALOG);
            }
            HELP_CLOSE_EL => {
                self.close_dialog(HELP_DIALOG);
            }
            BOOKMARK_ADD_BUTTON => self.open_bookmark_dialog(None),
            FIELD_NEW_ADD_EL => self.add_designed_field(),
            FIELD_PROP_APPLY_EL => self.apply_designed_field(),
            FIELD_PROP_DELETE_EL => self.delete_designed_field(),
            BOOKMARK_SAVE_BUTTON => self.save_bookmark_dialog(),
            BOOKMARK_CANCEL_BUTTON => {
                self.close_dialog(BOOKMARK_DIALOG);
            }
            LINK_PAGE_ADD_BUTTON => self.open_link_dialog("page"),
            LINK_URI_ADD_BUTTON => self.open_link_dialog("uri"),
            LINK_SAVE_BUTTON => self.save_link_dialog(),
            LINK_CANCEL_BUTTON => {
                self.close_dialog(LINK_DIALOG);
            }
            LINK_DRAW_BUTTON | SIGFIELD_DRAW_BUTTON => self.on_draw_tool_click(id),
            INSERT_BUTTON => self.request_insert(),
            INSERT_START_BUTTON => self.save_insert_dialog(),
            INSERT_CANCEL_BUTTON => {
                self.insert_source = None;
                self.close_dialog(INSERT_DIALOG);
            }
            EXTRACT_BUTTON => self.request_extract(),
            EXTRACT_START_BUTTON => self.save_extract_dialog(),
            EXTRACT_CANCEL_BUTTON => {
                self.close_dialog(EXTRACT_DIALOG);
            }
            SECURITY_BUTTON => self.open_security_dialog(),
            CERT_SIGN_PICK_BUTTON => self.pick_cert_sign_file(None),
            CERT_SIGN_GO => self.run_cert_sign(),
            CERT_SIGN_CANCEL => {
                self.close_dialog(CERT_SIGN_DIALOG);
            }
            VERIFY_OPEN_BUTTON => self.open_verify_dialog(),
            VERIFY_CLOSE => {
                self.close_dialog(VERIFY_DIALOG);
            }
            BATCH_PICK_BUTTON => {
                self.pick_batch_folder();
            }
            BATCH_START_BUTTON => {
                self.start_batch();
            }
            BATCH_CANCEL_BUTTON => self.on_batch_cancel(),
            BATCH_CLOSE_BUTTON => self.on_batch_close(),
            VERIFY_REFRESH => self.refresh_verify(),
            VERIFY_TRUST_ADD => self.on_verify_trust_add(),
            VERIFY_TRUST_CLEAR => self.on_verify_trust_clear(),
            SECURITY_APPLY_BUTTON => self.apply_security_dialog(),
            SECURITY_CANCEL_BUTTON => {
                self.close_dialog(SECURITY_DIALOG);
            }
            NEW_IMAGES_BUTTON => self.request_new_from_images(),
            EXPORT_BUTTON => self.request_export(),
            EXPORT_START_BUTTON => self.save_export_dialog(),
            EXPORT_CANCEL_BUTTON => {
                self.close_dialog(EXPORT_DIALOG);
            }
            PRINT_BUTTON => self.open_print_dialog(),
            WATERMARK_BUTTON => self.open_stamp_dialog(Some("watermark")),
            HEADER_BUTTON => self.open_stamp_dialog(Some("header")),
            PAGE_LABELS_BUTTON => self.open_labels_dialog(),
            STAMP_SAVE_BUTTON => {
                self.save_stamp_dialog();
            }
            STAMP_CANCEL_BUTTON => {
                self.close_dialog(STAMP_DIALOG);
            }
            LABELS_SAVE_BUTTON => {
                self.save_labels_dialog();
            }
            LABELS_CLEAR_BUTTON => {
                self.clear_labels();
            }
            LABELS_CANCEL_BUTTON => {
                self.close_dialog(LABELS_DIALOG);
            }
            PRINT_START_BUTTON => self.on_print_start(),
            PRINT_CANCEL_BUTTON => {
                self.close_dialog(PRINT_DIALOG);
            }
            PASSWORD_FORM => self.submit_password(),
            THUMBS_TOGGLE => self.set_thumb_sidebar(!self.thumb_open),
            CONTENT_REFRESH_BUTTON => self.refresh_content_targets(true),
            CONTENT_ADD_ARM_BUTTON => self.on_content_add_arm_click(),
            CONTENT_ADD_GO => self.submit_content_add(),
            CONTENT_FORM => {
                let text = self.dom.value(CONTENT_NEW_TEXT);
                self.submit_content_edit(&text);
            }
            CONTENT_DELETE_BUTTON => self.submit_content_edit(""),
            CONTENT_CANCEL_BUTTON => self.close_content_form(),
            CONTENT_OBJECT_FORM => self.submit_object_edit("box"),
            CONTENT_REPLACE_BUTTON => self.submit_object_edit("replace"),
            CONTENT_DELETE_OBJECT_BUTTON => self.submit_object_edit("delete"),
            CONTENT_CANCEL_OBJECT_BUTTON => self.close_content_object_form(),
            CONTENT_PREVIEW_BUTTON => self.preview_replace(),
            CONTENT_REPLACE_APPLY => self.apply_replace(),
            // The UI then focuses the find field.
            CONTENT_REPLACE_CANCEL => self.clear_replace_preview(),
            INSPECTOR_TOGGLE => self.set_inspector_open(!self.inspector_open),
            PREV_BUTTON => self.go_to(self.wanted_page - 1),
            NEXT_BUTTON => self.go_to(self.wanted_page + 1),
            ZOOM_OUT_BUTTON => self.zoom_step(-1),
            ZOOM_IN_BUTTON => self.zoom_step(1),
            FIT_WIDTH_BUTTON => self.set_zoom(ZoomMode::FitWidth, self.fixed_zoom),
            FIT_PAGE_BUTTON => self.set_zoom(ZoomMode::FitPage, self.fixed_zoom),
            ZOOM100_BUTTON => self.set_zoom(ZoomMode::Fixed, 1.0),
            SEARCH_PREV_BUTTON => self.move_hit(-1),
            SEARCH_NEXT_BUTTON => self.move_hit(1),
            COMPARE_CLOSE_BUTTON => self.close_compare(),
            _ => return false,
        }
        true
    }

    /// An `input` event: the field's value is `value`, then its listener runs.
    pub fn on_input(&mut self, id: &str, value: &str) -> bool {
        self.dom.set_value(id, value);
        match id {
            PRINT_RANGE_INPUT => self.update_print_range(),
            SEARCH_INPUT => self.on_search_input(),
            _ => return false,
        }
        true
    }

    /// A `change` event: the control's value and checked state are stored (a checked radio
    /// unchecks the others of its group), then its listener runs.
    pub fn on_change(&mut self, id: &str, value: &str, checked: bool) -> bool {
        self.dom.set_value(id, value);
        self.dom.set_checked(id, checked);
        let groups: [&[&str]; 2] = [
            &[SECURITY_MODE_NONE_INPUT, SECURITY_MODE_AES_INPUT],
            &[
                "mode-read",
                "mode-annotate",
                "mode-organize",
                "mode-forms",
                "mode-edit",
                "mode-redact",
            ],
        ];
        if checked && let Some(group) = groups.iter().find(|g| g.contains(&id)) {
            for other in group.iter().filter(|other| **other != id) {
                self.dom.set_checked(other, false);
            }
        }
        if let Some((_, mode, _)) = MODE_INPUTS.iter().find(|(input, _, _)| *input == id) {
            if checked {
                self.set_mode(*mode);
            }
            return true;
        }
        if let Some(layer) = id
            .strip_prefix("layer-")
            .and_then(|n| n.parse::<i64>().ok())
        {
            // R:9610: each layer checkbox is `layer-${id}`.
            self.set_layer(layer, checked);
            return true;
        }
        match id {
            STAMP_KIND_SELECT => self.sync_stamp_heading(),
            BATCH_KIND_SELECT => self.on_batch_kind_change(),
            FIELD_NEW_KIND_EL => self.render_designer(),
            FIELD_SELECT_EL => self.render_selected_field(),
            LINK_KIND_SELECT => {
                self.set_link_mode(if value == "uri" { "uri" } else { "page" }, true)
            }
            CERT_SIGN_VISIBLE => self.on_cert_sign_visible_change(),
            SECURITY_MODE_NONE_INPUT | SECURITY_MODE_AES_INPUT => self.sync_security_mode(),
            EXPORT_KIND_SELECT => self.sync_export_dpi(),
            PAGE_INPUT => self.on_page_input_change(),
            COMPARE_SHOW_DIFF_INPUT => self.set_compare_show_diff(checked),
            COMPARE_PAGE_BINPUT => self.on_compare_page_b_change(),
            _ => return false,
        }
        true
    }

    /// A `<dialog>`'s Escape: the native `cancel` (Batch keeps its dialog while running and
    /// cancels after the running file), then `close` with its listener.
    pub fn on_dialog_cancel(&mut self, id: &str) -> bool {
        if id == BATCH_DIALOG && self.on_batch_dialog_cancel() {
            return true;
        }
        if !super::controls::DIALOGS.contains(&id) {
            return false;
        }
        self.close_dialog(id)
    }

    /// `dialog.close()` and its `close` listener, if R:9533-end registers one; true when it did.
    pub fn close_dialog(&mut self, id: &str) -> bool {
        match id {
            NOTE_DIALOG => self.close_note_dialog(),
            CERT_SIGN_DIALOG => self.close_cert_sign_dialog(),
            SECURITY_DIALOG => self.close_security_dialog(),
            BOOKMARK_DIALOG => {
                self.dom.set_open(id, false);
                self.editing_bookmark = None;
            }
            LINK_DIALOG => {
                self.dom.set_open(id, false);
                self.link_anchor = None;
                self.nav_link_draft = None;
            }
            INSERT_DIALOG => {
                self.dom.set_open(id, false);
                self.insert_source = None;
            }
            _ => {
                self.dom.set_open(id, false);
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialogs_close_with_their_listeners() {
        let mut s = Session::new();
        s.dom.set_open(NOTE_DIALOG, true);
        s.editing_note = Some(3);
        assert!(s.on_dialog_cancel(NOTE_DIALOG));
        assert!(!s.dom.is_open(NOTE_DIALOG));
        assert_eq!(s.editing_note, None);
        s.dom.set_open(PRINT_DIALOG, true);
        assert!(!s.on_dialog_cancel(PRINT_DIALOG));
        assert!(!s.dom.is_open(PRINT_DIALOG));
        assert!(!s.on_dialog_cancel("not-a-dialog"));
    }

    #[test]
    fn change_and_input_store_values() {
        let mut s = Session::new();
        assert!(s.on_change(STAMP_KIND_SELECT, "header", false));
        assert_eq!(s.dom.text(STAMP_DIALOG_HEADING), "Add header");
        assert!(s.on_input(PRINT_RANGE_INPUT, "1"));
        assert_eq!(s.dom.text(PRINT_STATUS_EL), "Invalid range");
        assert!(!s.on_click("no-such-button"));
    }
}
