//! Lane 4's calls into renderer functions other lanes port (deleted by the lead at merge).
#![allow(dead_code, unused_variables)]

use super::Session;
use super::types::*;
use serde_json::Value;

impl Session {
    pub(crate) fn tabs_name(&self, value: Value) -> Option<String> {
        todo!("lane 2: tabs_name")
    }
    pub(crate) fn tab_order(
        &self,
        items: &[FieldItem],
        tabs: Option<&str>,
    ) -> (Vec<FieldItem>, String) {
        todo!("lane 2: tab_order")
    }
    pub(crate) fn sig_status(&self, item: &FieldItem) -> String {
        todo!("lane 2: sig_status")
    }
    pub(crate) fn sig_signed(&self, item: &FieldItem) -> bool {
        todo!("lane 2: sig_signed")
    }
    pub(crate) fn sync_note_current(&mut self) {
        todo!("lane 2: sync_note_current")
    }
    pub(crate) fn place_box(&self, element: &mut EditorState, rect: &[f64], zoom: f64) {
        todo!("lane 2: place_box")
    }
    pub(crate) fn shift_select(&mut self, member: Option<ObjectFocus>) {
        todo!("lane 2: shift_select")
    }
    pub(crate) fn link_member(&self, item: &LinkEntry) -> ObjectFocus {
        todo!("lane 2: link_member")
    }
    pub(crate) fn set_selection(&mut self, members: Vec<ObjectFocus>) {
        todo!("lane 2: set_selection")
    }
    pub(crate) fn mark_selection(&mut self) {
        todo!("lane 2: mark_selection")
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn nudge_rect(
        &self,
        rect: &[f64],
        key: &str,
        shift_key: bool,
        meta_key: bool,
        ctrl_key: bool,
        min: f64,
        size: &PageSize,
    ) -> Vec<f64> {
        todo!("lane 2: nudge_rect")
    }
    /// `console.log(line)`: the renderer's stdout lines (P2_NEW …), forwarded by main.
    pub(crate) fn console_log(&mut self, line: &str) {
        todo!("lead: console_log")
    }
    /// GAP: the TS encoded the page's RGBA on an OffscreenCanvas (`convertToBlob`: PNG, or JPEG
    /// at quality 0.9). tandem-app has no image encoder; the `png` crate is already in the
    /// lockfile through the engine. `kind` is "png" | "jpeg"; Err is the message to show.
    pub(crate) fn encode_page_image(
        &mut self,
        pixels: &[u8],
        width: i64,
        height: i64,
        kind: &str,
    ) -> Result<Vec<u8>, String> {
        todo!("lead: encode_page_image (PNG/JPEG encoder)")
    }
}
