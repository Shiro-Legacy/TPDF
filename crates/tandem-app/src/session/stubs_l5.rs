//! Lane 5's stubs for renderer functions other lanes port (the lead deletes this file when
//! merging). Lane numbers by the TS line: 1 = 1-2768, 2 = 2768-5347, 3 = 5347-7288 (and
//! redact-ui.ts, recovery.ts), 4 = 7288-9533.
#![allow(unused_variables, dead_code)]

use super::Session;
use super::types::*;
use serde_json::Value;

impl Session {
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
    pub(crate) fn add_text_box_at_centre(&mut self) {
        todo!("lane 2: add_text_box_at_centre")
    }
    pub(crate) fn layer_live(&self) -> bool {
        todo!("lane 2: layer_live")
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
    pub(crate) fn press_sign(&mut self) {
        todo!("lane 2: press_sign")
    }
    pub(crate) fn place_signature(
        &mut self,
        page: usize,
        point: Vec<f64>,
        width: Option<f64>,
    ) -> bool {
        todo!("lane 2: place_signature")
    }
    pub(crate) fn paragraph_text(
        &self,
        paragraph: &ParagraphItem,
        scan: Option<&ContentScan>,
    ) -> String {
        todo!("lane 2: paragraph_text")
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
    pub(crate) fn hide_banner(&mut self) {
        todo!("lane 3: hide_banner")
    }
}
