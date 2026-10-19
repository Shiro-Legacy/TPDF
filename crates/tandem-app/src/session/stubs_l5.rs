//! Lane 5's stubs for renderer functions other lanes port (the lead deletes this file when
//! merging). Lane numbers by the TS line: 1 = 1-2768, 2 = 2768-5347, 3 = 5347-7288 (and
//! redact-ui.ts, recovery.ts), 4 = 7288-9533.
#![allow(unused_variables, dead_code)]

use super::Session;
use super::controls::AreaDrag;
use super::types::*;
use crate::engine::{Refusal, Reply};
use serde_json::Value;
use tandem_protocol::engine::JournalState;

impl Session {
    pub(crate) fn go_to(&mut self, page: i64) {
        todo!("lane 1: go_to")
    }
    pub(crate) fn drain(&mut self) {
        todo!("lane 1: drain")
    }
    pub(crate) fn sync_controls(&mut self) {
        todo!("lane 1: sync_controls")
    }
    pub(crate) fn can_change_security(&self) -> bool {
        todo!("lane 1: can_change_security")
    }
    pub(crate) fn security_refusal(&self) -> String {
        todo!("lane 1: security_refusal")
    }
    pub(crate) fn recovery_saved_copy(&mut self, bytes: Option<Vec<u8>>) {
        todo!("lane 1: recovery_saved_copy")
    }
    pub(crate) fn recovery_fits_saved(&self, len: usize) -> bool {
        todo!("lane 1: recovery_fits_saved")
    }
    pub(crate) fn clear_direct_focus(&mut self) {
        todo!("lane 2: clear_direct_focus")
    }
    pub(crate) fn add_text_box_at_centre(&mut self) {
        todo!("lane 2: add_text_box_at_centre")
    }
    pub(crate) fn layer_live(&self) -> bool {
        todo!("lane 2: layer_live")
    }
    pub(crate) fn paint_hits(&mut self, width: f64, height: f64, page: i64) {
        todo!("lane 2: paint_hits")
    }
    pub(crate) fn signatures_summary(&self) -> Value {
        todo!("lane 2: signatures_summary")
    }
    pub(crate) fn selection_mixed(&self) -> bool {
        todo!("lane 2: selection_mixed")
    }
    pub(crate) fn toggle_member(&mut self, member: &ObjectFocus) {
        todo!("lane 2: toggle_member")
    }
    pub(crate) fn start_selection_gesture(&mut self, point: Point, member: &ObjectFocus) {
        todo!("lane 2: start_selection_gesture")
    }
    pub(crate) fn nudge_selection(&mut self, key: &str, shift: bool) {
        todo!("lane 2: nudge_selection")
    }
    pub(crate) fn delete_selection(&mut self) {
        todo!("lane 2: delete_selection")
    }
    pub(crate) fn clipboard_chord_ok(&self, kind: &str, ctx: &Value) -> bool {
        todo!("lane 2: clipboard_chord_ok")
    }
    pub(crate) fn clipboard_work(&mut self, which: &str, point: Option<Vec<f64>>) -> bool {
        todo!("lane 2: clipboard_work")
    }
    pub(crate) fn object_menu_state(&self) -> Option<Value> {
        todo!("lane 2: object_menu_state")
    }
    pub(crate) fn open_object_menu(
        &mut self,
        client_x: f64,
        client_y: f64,
        kind: &str,
        point: Option<Vec<f64>>,
    ) {
        todo!("lane 2: open_object_menu")
    }
    pub(crate) fn drop_direct_focus(&mut self) {
        todo!("lane 2: drop_direct_focus")
    }
    pub(crate) fn leave_crop(&mut self) {
        todo!("lane 2: leave_crop")
    }
    pub(crate) fn inspector_transform(&mut self, which: &str, button: &str) {
        todo!("lane 2: inspector_transform")
    }
    pub(crate) fn open_note_editor(&mut self, page: usize, point: Point) {
        todo!("lane 2: open_note_editor")
    }
    pub(crate) fn is_drawn_signature(&self, item: &AnnotItem) -> bool {
        todo!("lane 2: is_drawn_signature")
    }
    pub(crate) fn is_signature(&self, item: &AnnotItem) -> bool {
        todo!("lane 2: is_signature")
    }
    pub(crate) fn disarm_sign(&mut self) {
        todo!("lane 2: disarm_sign")
    }
    pub(crate) fn press_sign(&mut self) {
        todo!("lane 2: press_sign")
    }
    pub(crate) fn place_signature_at_centre(&mut self) {
        todo!("lane 2: place_signature_at_centre")
    }
    pub(crate) fn place_signature(
        &mut self,
        page: usize,
        point: Vec<f64>,
        width: Option<f64>,
    ) -> bool {
        todo!("lane 2: place_signature")
    }
    pub(crate) fn close_direct(&mut self) {
        todo!("lane 2: close_direct")
    }
    pub(crate) fn paragraph_text(
        &self,
        paragraph: &ParagraphItem,
        scan: Option<&ContentScan>,
    ) -> String {
        todo!("lane 2: paragraph_text")
    }
    pub(crate) fn finish_direct(&mut self) {
        todo!("lane 2: finish_direct")
    }
    pub(crate) fn open_add_text(&mut self, page: usize, point: Point) {
        todo!("lane 2: open_add_text")
    }
    pub(crate) fn click_to_add(
        &mut self,
        point: Option<Point>,
        button: i64,
        ctrl: bool,
        shift: bool,
    ) -> bool {
        todo!("lane 2: click_to_add")
    }
    pub(crate) fn hit_layer_count(&self, selector: &str) -> usize {
        todo!("lane 2: hit_layer_count")
    }
    pub(crate) fn hit_layer_box(&self, class: &str, key: &str) -> Option<Value> {
        todo!("lane 2: hit_layer_box")
    }
    pub(crate) fn on_search_input(&mut self) {
        todo!("lane 3: on_search_input")
    }
    pub(crate) fn start_search(&mut self) {
        todo!("lane 3: start_search")
    }
    pub(crate) fn move_hit(&mut self, delta: i64) {
        todo!("lane 3: move_hit")
    }
    pub(crate) fn set_thumb_sidebar(&mut self, open: bool) {
        todo!("lane 3: set_thumb_sidebar")
    }
    pub(crate) fn invalidate_thumb(&mut self, page: usize) {
        todo!("lane 3: invalidate_thumb")
    }
    pub(crate) fn refresh_content_targets(&mut self, force: bool) {
        todo!("lane 3: refresh_content_targets")
    }
    pub(crate) fn select_content_row(&mut self, index: usize) {
        todo!("lane 3: select_content_row")
    }
    pub(crate) fn close_content_form(&mut self) {
        todo!("lane 3: close_content_form")
    }
    pub(crate) fn activate_content_row(&mut self, index: usize) {
        todo!("lane 3: activate_content_row")
    }
    pub(crate) fn submit_content_add(&mut self) {
        todo!("lane 3: submit_content_add")
    }
    pub(crate) fn submit_content_edit(&mut self, text: &str) {
        todo!("lane 3: submit_content_edit")
    }
    pub(crate) fn close_content_object_form(&mut self) {
        todo!("lane 3: close_content_object_form")
    }
    pub(crate) fn submit_object_edit(&mut self, action: &str) {
        todo!("lane 3: submit_object_edit")
    }
    pub(crate) fn preview_replace(&mut self) {
        todo!("lane 3: preview_replace")
    }
    pub(crate) fn clear_replace_preview(&mut self) {
        todo!("lane 3: clear_replace_preview")
    }
    pub(crate) fn apply_replace(&mut self) {
        todo!("lane 3: apply_replace")
    }
    pub(crate) fn content_summary(&self) -> Value {
        todo!("lane 3: content_summary")
    }
    pub(crate) fn can_redact_draw(&self) -> bool {
        todo!("lane 3: can_redact_draw")
    }
    pub(crate) fn redact_escape_layer(&self) -> bool {
        todo!("lane 3: redact_escape_layer")
    }
    pub(crate) fn exit_redact_tool(&mut self) {
        todo!("lane 3: exit_redact_tool")
    }
    pub(crate) fn add_redact_area(
        &mut self,
        page: usize,
        rect: Vec<f64>,
        announcement: &str,
    ) -> bool {
        todo!("lane 3: add_redact_area")
    }
    pub(crate) fn open_redact_dialog(&mut self) {
        todo!("lane 3: open_redact_dialog")
    }
    pub(crate) fn confirm_redact(&mut self) {
        todo!("lane 3: confirm_redact")
    }
    pub(crate) fn confirm_pending_save(&mut self, action: PendingAction) -> bool {
        todo!("lane 3: confirm_pending_save")
    }
    pub(crate) fn hide_banner(&mut self) {
        todo!("lane 3: hide_banner")
    }
    pub(crate) fn sync_dirty(&mut self) {
        todo!("lane 3: sync_dirty")
    }
    pub(crate) fn announce(&mut self, text: &str) {
        todo!("lane 3: announce")
    }
    pub(crate) fn select_tab(&mut self, tabs: &[&str], panels: &[&str], index: usize) {
        todo!("lane 3: select_tab")
    }
    pub(crate) fn set_inspector_open(&mut self, open: bool) {
        todo!("lane 3: set_inspector_open")
    }
    pub(crate) fn has_direct_focus(&self) -> bool {
        todo!("lane 3: has_direct_focus")
    }
    pub(crate) fn armed_tool(&self) -> Option<String> {
        todo!("lane 3: armed_tool")
    }
    pub(crate) fn sync_tool_chip(&mut self) {
        todo!("lane 3: sync_tool_chip")
    }
    pub(crate) fn disarm_tools(&mut self) {
        todo!("lane 3: disarm_tools")
    }
    pub(crate) fn cancel_pending_tools(&mut self) {
        todo!("lane 3: cancel_pending_tools")
    }
    pub(crate) fn set_mode(&mut self, mode: UiMode) {
        todo!("lane 3: set_mode")
    }
    pub(crate) fn set_empty(
        &mut self,
        el: &str,
        count: usize,
        page_text: &str,
        document_text: &str,
    ) {
        todo!("lane 3: set_empty")
    }
    pub(crate) fn apply_edit(&mut self, reply: &JournalState, announcement: &str) {
        todo!("lane 3: apply_edit")
    }
    pub(crate) fn run_page_op(&mut self, op: &str) {
        todo!("lane 3: run_page_op")
    }
    pub(crate) fn annot_name(&self, item: &AnnotItem) -> Option<String> {
        todo!("lane 3: annot_name")
    }
    pub(crate) fn pump_thumbs(&mut self) {
        todo!("lane 3: pump_thumbs")
    }
}
