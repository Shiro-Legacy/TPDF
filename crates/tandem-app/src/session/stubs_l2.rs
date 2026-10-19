//! Lane 2's stubs for functions other lanes port (deleted by the lead when merging).
//! Page parameters are `usize` as the brief maps them; the lane's own code keeps `i64` pages
//! (the `wanted_page`/`shown_page` field type) and converts at these calls.
#![allow(dead_code, unused_variables)]

use super::Session;
use super::hit::{KeyEv, NodeKey, PtrEvent};
use super::nav::RenderGoal;
use super::types::*;
use serde_json::{Map, Value};

impl Session {
    // ------------------------------------------------------------------ lane 1 (renderer 1–2768)
    pub(crate) fn permitted(&mut self, name: &str) -> bool {
        todo!("lane 1: permitted")
    }
    pub(crate) fn editable(&mut self) -> bool {
        todo!("lane 1: editable")
    }
    pub(crate) fn can_annotate(&mut self) -> bool {
        todo!("lane 1: canAnnotate")
    }
    pub(crate) fn can_edit(&mut self) -> bool {
        todo!("lane 1: canEdit")
    }
    pub(crate) fn can_form(&mut self) -> bool {
        todo!("lane 1: canForm")
    }
    pub(crate) fn can_design(&mut self) -> bool {
        todo!("lane 1: canDesign")
    }
    pub(crate) fn target(&mut self) -> Option<RenderGoal> {
        todo!("lane 1: target")
    }
    pub(crate) fn sync_zoom(&mut self) {
        todo!("lane 1: syncZoom")
    }

    // ------------------------------------------------------------------ lane 3 (renderer 5347–7288, redact-ui.ts)
    pub(crate) fn set_thumb_current(&mut self, page: usize) {
        todo!("lane 3: setThumbCurrent")
    }
    pub(crate) fn pump_thumbs(&mut self) {
        todo!("lane 3: pumpThumbs")
    }
    pub(crate) fn refresh_content_targets(&mut self, force: bool) {
        todo!("lane 3: refreshContentTargets")
    }
    pub(crate) fn object_row_label(&mut self, item: &EditTarget) -> String {
        todo!("lane 3: objectRowLabel")
    }
    pub(crate) fn object_row_state(&mut self, item: &EditTarget) -> String {
        todo!("lane 3: objectRowState")
    }
    pub(crate) fn select_content_row(&mut self, index: usize, focus: bool) {
        todo!("lane 3: selectContentRow")
    }
    pub(crate) fn apply_content_edit(
        &mut self,
        scan: &ContentScan,
        item: &EditTarget,
        text: &str,
    ) -> bool {
        todo!("lane 3: applyContentEdit")
    }
    /// `{name, bytes}` of the one image picked, or None.
    pub(crate) fn pick_one_image(&mut self) -> Option<(String, Vec<u8>)> {
        todo!("lane 3: pickOneImage")
    }
    pub(crate) fn open_content_object_form(&mut self, item: &EditTarget, focus: bool) {
        todo!("lane 3: openContentObjectForm")
    }
    /// `extra` may carry `{"$buffer": i}` placeholders into `inputs` (the replace image).
    pub(crate) fn apply_object_edit(
        &mut self,
        scan: &ContentScan,
        item: &EditTarget,
        action: &str,
        extra: Value,
        inputs: Vec<Vec<u8>>,
    ) -> bool {
        todo!("lane 3: applyObjectEdit")
    }
    /// `entries: {item, box?}[]`.
    pub(crate) fn apply_objects_edit(
        &mut self,
        scan: &ContentScan,
        entries: Vec<(EditTarget, Option<Vec<f64>>)>,
        action: &str,
        transform: Option<&str>,
    ) -> bool {
        todo!("lane 3: applyObjectsEdit")
    }
    pub(crate) fn can_redact_draw(&mut self) -> bool {
        todo!("lane 3: canRedactDraw")
    }
    pub(crate) fn redact_page_rects(&mut self, page: usize) -> Vec<Vec<f64>> {
        todo!("lane 3: redactPageRects")
    }
    pub(crate) fn schedule_redact_inspect(&mut self, page: usize, rects: Vec<Vec<f64>>) {
        todo!("lane 3: scheduleRedactInspect")
    }
    pub(crate) fn add_redact_page(&mut self, page: usize) -> bool {
        todo!("lane 3: addRedactPage")
    }
    pub(crate) fn sync_dirty(&mut self) {
        todo!("lane 3: syncDirty")
    }
    pub(crate) fn announce(&mut self, text: &str) {
        todo!("lane 3: announce")
    }
    pub(crate) fn has_direct_focus(&mut self) -> bool {
        todo!("lane 3: hasDirectFocus")
    }
    pub(crate) fn armed_tool(&mut self) -> Option<String> {
        todo!("lane 3: armedTool")
    }
    pub(crate) fn sync_tool_chip(&mut self) {
        todo!("lane 3: syncToolChip")
    }
    pub(crate) fn disarm_tools(&mut self) {
        todo!("lane 3: disarmTools")
    }
    pub(crate) fn set_mode(&mut self, mode: UiMode) {
        todo!("lane 3: setMode")
    }
    pub(crate) fn apply_edit(&mut self, reply: Value, announcement: &str) {
        todo!("lane 3: applyEdit")
    }
    pub(crate) fn refresh_notes(&mut self) {
        todo!("lane 3: refreshNotes")
    }
    /// redact-ui.ts `render()`.
    pub(crate) fn redact_ui_render(&mut self) {
        todo!("lane 3: redactUi.render")
    }
    /// redact-ui.ts `paintBoxes(fragment, ns, page, zoom, rects)` (the fragment is the UI's).
    pub(crate) fn redact_ui_paint_boxes(&mut self, page: usize, zoom: f64, rects: Vec<Vec<f64>>) {
        todo!("lane 3: redactUi.paintBoxes")
    }

    // ------------------------------------------------------------------ lane 4 (renderer 7288–9533)
    pub(crate) fn refresh_fields(&mut self) {
        todo!("lane 4: refreshFields")
    }
    pub(crate) fn field_label(&mut self, item: &FieldItem) -> String {
        todo!("lane 4: fieldLabel")
    }
    pub(crate) fn commit_field(
        &mut self,
        item: &FieldItem,
        value: FieldValue,
        page: usize,
    ) -> bool {
        todo!("lane 4: commitField")
    }
    pub(crate) fn refresh_links(&mut self) {
        todo!("lane 4: refreshLinks")
    }
    pub(crate) fn link_label(&mut self, item: &LinkEntry) -> String {
        todo!("lane 4: linkLabel")
    }
    pub(crate) fn scheme_name(&mut self, uri: &str) -> String {
        todo!("lane 4: schemeName")
    }
    pub(crate) fn follow_link(&mut self, item: &LinkEntry) {
        todo!("lane 4: followLink")
    }
    pub(crate) fn delete_link(&mut self, link_id: i64) {
        todo!("lane 4: deleteLink")
    }
    pub(crate) fn commit_link_rect(
        &mut self,
        item: &LinkEntry,
        page: usize,
        rect: Vec<f64>,
    ) -> bool {
        todo!("lane 4: commitLinkRect")
    }
    pub(crate) fn sync_link_current(&mut self) {
        todo!("lane 4: syncLinkCurrent")
    }
    /// `focusLink(item, button)`: the button is the link's hit-layer node.
    pub(crate) fn focus_link(&mut self, item: &LinkEntry, button: Option<NodeKey>) {
        todo!("lane 4: focusLink")
    }
    pub(crate) fn open_link_editor(&mut self, item: &LinkEntry, page: usize) {
        todo!("lane 4: openLinkEditor")
    }
    pub(crate) fn commit_link_editor(&mut self, active: &DirectActive, silent: bool) -> bool {
        todo!("lane 4: commitLinkEditor")
    }
    pub(crate) fn render_notes(&mut self) {
        todo!("lane 4: renderNotes")
    }
    pub(crate) fn add_note_at(
        &mut self,
        page: usize,
        point: [f64; 2],
        contents: &str,
        announcement: &str,
    ) -> bool {
        todo!("lane 4: addNoteAt")
    }
    pub(crate) fn current_hit(&mut self) -> Option<SearchHit> {
        todo!("lane 4: currentHit")
    }
    pub(crate) fn sync_markup_buttons(&mut self) {
        todo!("lane 4: syncMarkupButtons")
    }
    pub(crate) fn delete_note(&mut self, annot_id: i64, announcement: &str) {
        todo!("lane 4: deleteNote")
    }
    pub(crate) fn refresh_compare(&mut self, reset_b: bool, speak: bool) {
        todo!("lane 4: refreshCompare")
    }
    /// `startAreaDrag`'s window `pointermove` listener (a no-op when no area drag is live).
    pub(crate) fn area_drag_move(&mut self, event: &PtrEvent) {
        todo!("lane 4: startAreaDrag move")
    }
    /// `startAreaDrag`'s window `pointerup`/`pointercancel` listener (a no-op when none is live).
    pub(crate) fn area_drag_end(&mut self, event: &PtrEvent, cancel: bool) {
        todo!("lane 4: startAreaDrag end")
    }

    // ------------------------------------------------------------------ lane 5 (renderer 9533–end)
    pub(crate) fn label_suffix(&mut self, page: usize) -> String {
        todo!("lane 5: labelSuffix")
    }
    pub(crate) fn apply_label_text(&mut self) {
        todo!("lane 5: applyLabelText")
    }
    pub(crate) fn drive_wanted(&mut self, contents: Option<&str>) -> Option<Map<String, Value>> {
        todo!("lane 5: driveWanted")
    }
    pub(crate) fn open_verify_dialog(&mut self) {
        todo!("lane 5: openVerifyDialog")
    }
    pub(crate) fn mark_verify_row(&mut self, name: &str) {
        todo!("lane 5: markVerifyRow")
    }
    pub(crate) fn open_sign_choice(&mut self) {
        todo!("lane 5: openSignChoice")
    }
    pub(crate) fn open_cert_sign_dialog(&mut self, field: Option<&FieldItem>) {
        todo!("lane 5: openCertSignDialog")
    }
    pub(crate) fn set_status(&mut self, text: &str) {
        todo!("lane 5: setStatus")
    }
    pub(crate) fn show_error(&mut self, text: &str) {
        todo!("lane 5: showError")
    }
    pub(crate) fn clear_error(&mut self) {
        todo!("lane 5: clearError")
    }
    pub(crate) fn print_available(&mut self) -> bool {
        todo!("lane 5: printAvailable")
    }
    /// R:11112 `pageRegion` pointerdown (addBlocked, area drags, ink).
    pub(crate) fn on_page_pointerdown(&mut self, event: &mut PtrEvent) {
        todo!("lane 5: pageRegion pointerdown")
    }
    /// R:11146 `pageRegion` pointermove (ink).
    pub(crate) fn on_page_pointermove(&mut self, event: &mut PtrEvent) {
        todo!("lane 5: pageRegion pointermove")
    }
    /// R:11165 `pageRegion` pointerup/pointercancel (`finishInk`).
    pub(crate) fn on_page_pointerup(&mut self, event: &mut PtrEvent, cancel: bool) {
        todo!("lane 5: pageRegion pointerup")
    }
    /// R:11081 `pageRegion` click (sign placement, clickToAdd, baseline pick, FreeText, note).
    pub(crate) fn on_page_click(&mut self, event: &mut PtrEvent) {
        todo!("lane 5: pageRegion click")
    }
    /// R:11325 `pageRegion` keydown.
    pub(crate) fn on_page_keydown(&mut self, event: &mut KeyEv) {
        todo!("lane 5: pageRegion keydown")
    }
    /// R:11369 `document` keydown.
    pub(crate) fn on_document_keydown(&mut self, event: &mut KeyEv) {
        todo!("lane 5: document keydown")
    }
    /// R:11349 `window` keydown (capture): Escape stops an armed tool.
    pub(crate) fn on_window_keydown_capture(&mut self, event: &mut KeyEv) {
        todo!("lane 5: window keydown capture")
    }
    /// R:10969 `certSignDialog` close.
    pub(crate) fn on_cert_sign_dialog_close(&mut self) {
        todo!("lane 5: certSignDialog close")
    }

    // ------------------------------------------------------------------ the lead (host / platform)
    /// `localStorage.setItem(key, value)` (the persisted "field-highlight" / "edit-outlines").
    pub(crate) fn local_storage_set(&mut self, key: &str, value: &str) {
        todo!("lead: localStorage")
    }
    /// R:4832-4843: the picked PNG/JPEG decoded upright (EXIF applied), scaled to a long side of
    /// at most 1000 px and re-encoded as PNG with no metadata: `(png, width, height)`; Err is the
    /// refusal text (`SIGN_IMAGE_NOT` when it does not decode or a side is over 16384 px,
    /// `SIGN_IMAGE_UNREADABLE` when encoding fails).
    pub(crate) fn decode_signature_image(
        &mut self,
        bytes: &[u8],
    ) -> Result<(Vec<u8>, f64, f64), String> {
        todo!("lead: decode a signature image")
    }
}

/// engine-shared.ts `createRenderCache`'s `get`/`set` (the part that ports the cache).
impl<T: Clone> RenderCache<T> {
    pub(crate) fn get(&mut self, key: &str) -> Option<T> {
        todo!("lane 1: cache.get")
    }
    pub(crate) fn set(&mut self, key: &str, value: T) {
        todo!("lane 1: cache.set")
    }
}
