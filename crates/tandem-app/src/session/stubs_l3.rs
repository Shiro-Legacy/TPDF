//! Lane 3's calls into other lanes' ranges (deleted by the lead at merge).

use super::Session;
use tandem_protocol::engine::{EditTarget, Placement};

/// What `target()` (R:2736) returns: `{ page, zoom, scale, key }` (lane 1's `zoom::RenderTarget`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderTarget {
    pub page: i64,
    pub zoom: f64,
    pub scale: f64,
    pub key: String,
}

impl Session {
    pub(crate) fn repaint(&mut self) {
        todo!("lane 2: repaint")
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

    /// Whether `#hit-layer .area-draft` exists (a box drag of startAreaDrag, R:8133).
    pub(crate) fn area_draft_open(&self) -> bool {
        todo!("lane 4: .area-draft")
    }
}
