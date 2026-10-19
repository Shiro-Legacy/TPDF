//! Lane 1: renderer.ts 937–1035, "start screen (UX audit X4)", and the `createRedactUi`
//! host callbacks declared there (R:995-1022) that carry logic of their own. The others
//! forward to a renderer function the redaction tool calls directly: `preview` →
//! `redact_view`, `goTo` → `go_to`, `addSearch` → `add_redact_search`, `openOriginal` →
//! `open_redact_original`, `repaint`, `reportPending` → `report_redact_pending`, `showBox` →
//! `show_redact_box`.
#![allow(dead_code)]

use super::Session;
use super::dom_ids as ids;
use super::types::{PageSize, RedactBox};
use crate::messages;
use serde_json::json;
use tandem_protocol::engine::AnnotsReply;

impl Session {
    /// `#start-open` (R:948).
    pub fn on_start_open(&mut self) {
        self.request_open();
        self.run_pending_tasks();
        self.pump_host_events();
    }

    /// `#start-new` (R:949): New blank PDF; main writes Untitled.pdf and delivers it.
    pub fn on_start_new(&mut self) {
        self.clear_error();
        self.host.new_blank(None);
        self.pump_host_events();
    }

    /// `#start-images` (R:950).
    pub fn on_start_images(&mut self) {
        self.request_new_from_images();
        self.run_pending_tasks();
        self.pump_host_events();
    }

    /// `#start-recent-clear` (R:952): main forgets the list, then the screen is redrawn.
    pub fn on_start_recent_clear(&mut self) {
        self.host.recent_clear();
        self.show_start_screen();
    }

    /// A `#start-recent-list` row (`data-recent-id`, R:953).
    pub fn on_start_recent(&mut self, id: &str) {
        if id.is_empty() {
            return;
        }
        self.clear_error();
        self.host.recent_open(id);
        self.pump_host_events();
    }

    /// R:963 `showStartScreen`: a normal launch: Open, New, New from images, recent files and
    /// the drop hint, until a document opens. The renderer sees names and folders, never paths.
    pub fn show_start_screen(&mut self) {
        self.lane1.starting = true;
        self.dom.set_hidden(ids::START_SCREEN, false);
        self.sync_controls();
        self.set_status("No document open");
        let recent = self.host.recent_list();
        self.lane1.start_recent = recent
            .into_iter()
            .map(|entry| (entry.id, entry.name, entry.folder))
            .collect();
        self.dom
            .set_hidden(ids::START_RECENT, self.lane1.start_recent.is_empty());
    }

    /// R:989 `hideStartScreen`.
    pub(crate) fn hide_start_screen(&mut self) {
        if self.dom.hidden(ids::START_SCREEN) {
            return;
        }
        self.dom.set_hidden(ids::START_SCREEN, true);
        self.lane1.starting = false;
    }

    // ------------------------------------------------------------ createRedactUi host (R:995)

    /// `pages()`: the page sizes, or None while locked (or with no document).
    pub fn redact_ui_pages(&self) -> Option<Vec<PageSize>> {
        let doc = self.doc.as_ref()?;
        (!doc.info.needs_password).then(|| doc.pages.clone())
    }

    /// `boxesOn(page)`: the page's Redact marks plus the boxes held in memory (negative ids
    /// `-(index + 1)`); a signed file's boxes live only in memory.
    pub fn redact_ui_boxes_on(
        &mut self,
        page: i64,
    ) -> Result<Vec<RedactBox>, crate::engine::Refusal> {
        let memory: Vec<RedactBox> = self
            .redact_memory
            .iter()
            .enumerate()
            .map(|(index, region)| RedactBox {
                page: region.page,
                id: -(index as i64 + 1),
                rect: region.rect.clone(),
                search: false,
            })
            .filter(|b| b.page == page)
            .collect();
        if self.redact_memory_mode() {
            return Ok(memory);
        }
        let mut boxes = if page == self.wanted_page {
            let notes = self.notes.clone();
            self.redact_boxes(notes, page.max(0) as usize)
        } else {
            let reply = self.request(json!({"op": "annots", "page": page}), Vec::new())?;
            let reply: AnnotsReply = serde_json::from_value(reply.value).unwrap_or_default();
            self.redact_boxes(reply.items, page.max(0) as usize)
        };
        boxes.extend(memory);
        Ok(boxes)
    }

    /// `deleteBox(page, annotId)`: a memory box is dropped here; a mark is deleted in the
    /// engine and recorded in `redactDeletes`, so it does not count as another unsaved change.
    pub fn redact_ui_delete_box(&mut self, page: i64, annot_id: i64) {
        if annot_id < 0 {
            let index = (-annot_id - 1) as usize;
            if index < self.redact_memory.len() {
                self.redact_memory.remove(index);
            }
            self.after_redact_memory(messages::redact::REMOVED_BOX);
            return;
        }
        if !self.can_annotate() {
            return;
        }
        self.redact_deletes.insert(format!("{page}:{annot_id}"));
        match self.request(
            json!({"op": "annot-delete", "page": page, "annotId": annot_id}),
            Vec::new(),
        ) {
            Ok(reply) => self.apply_edit(reply.value, messages::redact::REMOVED_BOX),
            Err(err) => self.show_error(&err.message),
        }
    }

    /// `canEdit()`: the tool may change something (marks, or boxes held in memory).
    pub fn redact_ui_can_edit(&mut self) -> bool {
        self.can_edit() || self.redact_memory_mode()
    }

    /// `canAnnotate()`: the tool may draw.
    pub fn redact_ui_can_annotate(&mut self) -> bool {
        self.can_redact_draw()
    }
}
