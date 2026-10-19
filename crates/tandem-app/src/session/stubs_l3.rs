//! Lane 3's calls into other lanes' ranges (deleted by the lead at merge).

use super::Session;
use crate::messages::MessageKind;
use serde_json::{Map, Value};
use tandem_protocol::engine::{EditTarget, FieldItem, FieldValue, Placement};

/// What `target()` (R:2736) returns: `{ page, zoom, scale, key }` (lane 1's `zoom::RenderTarget`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderTarget {
    pub page: i64,
    pub zoom: f64,
    pub scale: f64,
    pub key: String,
}

impl Session {
    // ------------------------------------------------------------ lane 1
    /// R:1075 `request(message)`: the renderer's request (recovery log, paused reads).
    pub(crate) fn request(
        &mut self,
        message: Value,
        inputs: Vec<Vec<u8>>,
    ) -> crate::engine::EngineResult {
        let _ = (message, inputs);
        todo!("lane 1: request")
    }
    /// The RedactHost (R:995): `pages()`.
    pub(crate) fn redact_ui_pages(&self) -> Option<Vec<tandem_protocol::engine::PageSize>> {
        todo!("lane 1: redactUi host pages")
    }
    /// The RedactHost (R:995): `boxesOn(page)`.
    pub(crate) fn redact_ui_boxes_on(
        &mut self,
        page: i64,
    ) -> Result<Vec<super::types::RedactBox>, crate::engine::Refusal> {
        let _ = page;
        todo!("lane 1: redactUi host boxesOn")
    }
    /// The RedactHost (R:995): `deleteBox(page, annotId)`.
    pub(crate) fn redact_ui_delete_box(&mut self, page: i64, annot_id: i64) {
        let _ = (page, annot_id);
        todo!("lane 1: redactUi host deleteBox")
    }
    /// The RedactHost (R:995): `canEdit()`.
    pub(crate) fn redact_ui_can_edit(&mut self) -> bool {
        todo!("lane 1: redactUi host canEdit")
    }
    /// The RedactHost (R:995): `canAnnotate()`.
    pub(crate) fn redact_ui_can_annotate(&mut self) -> bool {
        todo!("lane 1: redactUi host canAnnotate")
    }
    pub(crate) fn permitted(&self, name: &str) -> bool {
        let _ = name;
        todo!("lane 1: permitted")
    }
    pub(crate) fn can_annotate(&self) -> bool {
        todo!("lane 1: canAnnotate")
    }
    pub(crate) fn can_edit(&self) -> bool {
        todo!("lane 1: canEdit")
    }
    pub(crate) fn open_from_url(&mut self, name: &str, url: &str, untitled: bool) {
        let _ = (name, url, untitled);
        todo!("lane 1: openFromUrl")
    }
    pub(crate) fn target(&self) -> Option<RenderTarget> {
        todo!("lane 1: target")
    }
    pub(crate) fn console_line(&mut self, prefix: &str, payload: Value) {
        let _ = (prefix, payload);
        todo!("lane 1: console_line")
    }

    // ------------------------------------------------------------ lane 2
    pub(crate) fn go_to(&mut self, page: i64) {
        let _ = page;
        todo!("lane 2: goTo")
    }
    pub(crate) fn drain(&mut self) {
        todo!("lane 2: drain")
    }
    pub(crate) fn sync_controls(&mut self) {
        todo!("lane 2: syncControls")
    }
    pub(crate) fn repaint(&mut self) {
        todo!("lane 2: repaint")
    }
    pub(crate) fn paint_hits(&mut self, width: f64, height: f64, page: i64) {
        let _ = (width, height, page);
        todo!("lane 2: paintHits")
    }
    /// `canvas.clientWidth` (CSS px of the page canvas).
    pub(crate) fn canvas_client_width(&self) -> f64 {
        todo!("lane 2: canvas.clientWidth")
    }
    /// `canvas.clientHeight`.
    pub(crate) fn canvas_client_height(&self) -> f64 {
        todo!("lane 2: canvas.clientHeight")
    }
    pub(crate) fn note_filled(&mut self) {
        todo!("lane 2: noteFilled")
    }
    pub(crate) fn clear_direct_focus(&mut self) {
        todo!("lane 2: clearDirectFocus")
    }
    /// A free `pub fn view_box(bx: &[f64], m: &[f64])` in lane 2's nav.rs.
    pub(crate) fn view_box(&self, bx: &[f64], m: &[f64]) -> Vec<f64> {
        let _ = (bx, m);
        todo!("lane 2: viewBox")
    }
    pub(crate) fn placement_of(&self, item: &EditTarget) -> Option<Placement> {
        let _ = item;
        todo!("lane 2: placementOf")
    }
    pub(crate) fn sync_inspector_transform(&mut self) {
        todo!("lane 2: syncInspectorTransform")
    }
    pub(crate) fn open_inline_text(&mut self, item: EditTarget, single_run: bool) {
        let _ = (item, single_run);
        todo!("lane 2: openInlineText")
    }
    pub(crate) fn draft_pending(&self) -> bool {
        todo!("lane 2: draftPending")
    }
    pub(crate) fn drop_direct(&mut self) {
        todo!("lane 2: dropDirect")
    }
    pub(crate) fn add_text_at(&mut self, page: usize, point: Vec<f64>, text: &str) -> bool {
        let _ = (page, point, text);
        todo!("lane 2: addTextAt")
    }
    /// A click on the hit layer's `.redact-target[data-annot-id=id]` (R:4584).
    pub(crate) fn on_redact_target_click(&mut self, annot_id: i64) {
        let _ = annot_id;
        todo!("lane 2: redact-target click")
    }

    // ------------------------------------------------------------ lane 4
    pub(crate) fn refresh_fields(&mut self) {
        todo!("lane 4: refreshFields")
    }
    pub(crate) fn commit_field(
        &mut self,
        item: &FieldItem,
        value: FieldValue,
        page: usize,
    ) -> bool {
        let _ = (item, value, page);
        todo!("lane 4: commitField")
    }
    pub(crate) fn refresh_outline(&mut self) {
        todo!("lane 4: refreshOutline")
    }
    pub(crate) fn refresh_links(&mut self) {
        todo!("lane 4: refreshLinks")
    }
    /// Whether `#hit-layer .area-draft` exists (a box drag of startAreaDrag, R:8133).
    pub(crate) fn area_draft_open(&self) -> bool {
        todo!("lane 4: .area-draft")
    }
    pub(crate) fn render_notes(&mut self) {
        todo!("lane 4: renderNotes")
    }
    pub(crate) fn sync_markup_buttons(&mut self) {
        todo!("lane 4: syncMarkupButtons")
    }
    pub(crate) fn shape_box(&self) -> Option<Vec<f64>> {
        todo!("lane 4: shapeBox")
    }
    pub(crate) fn refresh_compare(&mut self, reset_b: bool, speak: bool) {
        let _ = (reset_b, speak);
        todo!("lane 4: refreshCompare")
    }
    /// A free `pub fn file_stem(name)` in lane 4's exports.rs.
    pub(crate) fn file_stem(&self, name: &str) -> String {
        let _ = name;
        todo!("lane 4: fileStem")
    }
    pub(crate) fn save_new_file(
        &mut self,
        bytes: &[u8],
        suggested_name: &str,
        kind: &str,
        open: bool,
    ) -> Option<String> {
        let _ = (bytes, suggested_name, kind, open);
        todo!("lane 4: saveNewFile")
    }

    // ------------------------------------------------------------ lane 5
    pub(crate) fn label_suffix(&self, page: usize) -> String {
        let _ = page;
        todo!("lane 5: labelSuffix")
    }
    pub(crate) fn log_edit(&mut self) {
        todo!("lane 5: logEdit")
    }
    pub(crate) fn drive_wanted(&mut self, contents: Option<&str>) -> Option<Map<String, Value>> {
        let _ = contents;
        todo!("lane 5: driveWanted")
    }
    pub(crate) fn smoke_password(&mut self, value: Option<&str>) {
        let _ = value;
        todo!("lane 5: smokePassword")
    }
    pub(crate) fn save_in_place(&mut self, mode: &str) {
        let _ = mode;
        todo!("lane 5: saveInPlace")
    }
    pub(crate) fn set_status(&mut self, text: &str) {
        let _ = text;
        todo!("lane 5: setStatus")
    }
    pub(crate) fn show_error(&mut self, text: &str) {
        let _ = text;
        todo!("lane 5: showError")
    }
    pub(crate) fn show_message(&mut self, text: &str, kind: MessageKind) {
        let _ = (text, kind);
        todo!("lane 5: showMessage")
    }
    pub(crate) fn clear_error(&mut self) {
        todo!("lane 5: clearError")
    }
}
