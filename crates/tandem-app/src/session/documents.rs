//! Lane 1: renderer.ts 1551–1733, "documents": the document reset, main's messages
//! (`registerOpenPath`), the `driven` queue and `refreshPageIds`, and the drive harness seam
//! ([`crate::drive::Driven`]).
#![allow(dead_code)]

use super::Session;
use super::consts::op_mode;
use super::dom_ids as ids;
use super::types::{FillSave, UiMode};
use crate::host::{Document, Event, Host, OpenError, SaveRequest};
use crate::messages;
use serde_json::{Value, json};
use tandem_protocol::engine::PageIdsReply;
use tandem_protocol::limits::MAX_PDF_BYTES;

/// The dialogs `resetEditState` closes (R:1638-1647); print, help, batch, verify, cert-sign,
/// sign-choice, sigpad, stamp-icon and the compare view survive by design.
const RESET_DIALOGS: [&str; 10] = [
    ids::NOTE_DIALOG,
    ids::BOOKMARK_DIALOG,
    ids::LINK_DIALOG,
    ids::INSERT_DIALOG,
    ids::EXTRACT_DIALOG,
    ids::EXPORT_DIALOG,
    ids::STAMP_DIALOG,
    ids::LABELS_DIALOG,
    ids::SECURITY_DIALOG,
    ids::REDACT_DIALOG,
];

impl Session {
    /// R:1554 `resetDocument`: drops every trace of the open document.
    pub(crate) fn reset_document(&mut self) {
        self.doc = None;
        self.wanted_page = 0;
        self.shown_page = -1;
        self.shown_key = None;
        self.shown_zoom = 0.0;
        self.in_flight = None;
        self.last_result = None;
        self.cache.clear();
        self.reset_thumbs();
        self.set_thumb_sidebar(true); // a closed document shows the sidebar and its empty line
        self.reset_edit_state();
        self.dom.set_text(ids::EXTRACTED, "");
        self.dom.set_hidden(ids::TEXT_EMPTY_EL, false);
        self.dom.set_text(ids::DOCUMENT_TITLE_EL, "Tandem PDF");
        self.dom.set_value(ids::SEARCH_INPUT, "");
        self.clear_search();
        self.clear_warning();
        self.hide_password_form();
    }

    /// R:1576 `resetEditState`: no document, nothing dirty, no history.
    pub(crate) fn reset_edit_state(&mut self) {
        self.revision = 0;
        self.content_edited = None; // debt1 row 19: STATE content never carries the last document's edit
        self.content_refusals.clear();
        self.dirty = false;
        self.can_undo = false;
        self.can_redo = false;
        self.notes.clear();
        self.notes_seq += 1;
        self.fields.clear();
        self.fields_page = -1;
        self.fields_tabs = None;
        self.links_page = -1;
        self.nav_link_draft = None;
        self.direct_focused_link = None;
        // A new document never inherits the last one's editor.
        if let Some(active) = self.direct_active.take() {
            self.remove_direct(&active);
        }
        self.direct_active = None;
        self.direct_focused_field = None;
        self.direct_field_index = -1;
        self.direct_focused_box = None; // fill-anywhere D7: a new or closed document is not filled
        self.fill_last = None;
        // draw-signature D5/D7
        self.sign_armed = false;
        self.set_pressed(ids::FILL_SIGN_BUTTON, false);
        self.fill_sign_last = None;
        self.direct_focused_sig = None;
        self.fill_save = FillSave::None;
        self.fields_seq += 1;
        self.field_design = None;
        self.outline.clear();
        self.outline_seq += 1;
        self.links.clear();
        self.links_seq += 1;
        self.nav_links.clear();
        self.link_anchor = None;
        self.editing_bookmark = None;
        self.insert_source = None;
        self.assemble_summary = None;
        self.extract_summary = None;
        self.page_ids = None;
        self.annot_last = None;
        self.nav_last = None;
        self.redact_state = None;
        self.redact_last = None;
        self.redact_prompt = None;
        self.redact_marks_acknowledged = -1;
        self.redact_memory.clear();
        self.redact_inspect_key.clear();
        self.redact_deletes.clear();
        self.redact_ui_close_find();
        self.cert_sign_field = None;
        self.verify_by_field.clear();
        self.export_summary = None;
        self.labels.clear();
        self.labels_seq += 1;
        self.layers.clear();
        self.layers_seq += 1;
        self.layers_persisted = false;
        self.decorate_summary = None;
        self.editing_note = None;
        self.box_point = None;
        // A new document starts with no proven access and no pending security change.
        self.auth = Default::default();
        self.security_state = None;
        for id in RESET_DIALOGS {
            self.close_dialog(id);
        }
        self.render_redact_state();
        self.render_notes();
        self.render_fields();
        self.render_outline();
        self.render_links();
        self.render_layers();
        self.announce_nav("");
        self.dom.set_text(ids::PAGE_STATUS_EL, "");
        self.sync_dirty();
    }

    // ------------------------------------------------------------ registerOpenPath (R:1658)

    /// `document {name, url, untitled?}` (R:1660): main hands the document over as a one-shot
    /// `app://pdf4all/open/<token>` URL.
    pub fn on_document(&mut self, name: &str, url: &str, untitled: bool) {
        let (name, url) = (name.to_string(), url.to_string());
        self.driven(move |s| s.open_from_url(&name, &url, untitled));
    }

    /// `open-error {reason, name?, limit?}` (R:1666).
    pub fn on_open_error(&mut self, reason: &str, name: Option<&str>, limit: Option<f64>) {
        self.pending_open_notice = None; // review fix 6: the signed copy did not open, so its notice is not shown later
        if reason == "cancelled" {
            self.clear_error();
        } else {
            // Fix 3: a recent file that could not be opened has left main's list.
            if !self.dom.hidden(ids::START_SCREEN) {
                self.show_start_screen();
            }
            let name = name.unwrap_or("That file");
            let text = if reason == "too-large" {
                let limit = limit
                    .filter(|l| *l != 0.0 && !l.is_nan())
                    .unwrap_or(MAX_PDF_BYTES as f64);
                messages::open::too_large(name, Some(limit))
            } else {
                messages::open::unreadable(name)
            };
            self.show_error(&text);
        }
        // `driven(() => undefined)`: the error is handled outside the queue, and one STATE follows.
        self.driven(|_| {});
    }

    /// `save-request` (R:1681): `{inPlace:true}` is Save, `{as:true}` Save As, anything else
    /// Save a Copy.
    pub fn on_save_request(&mut self, request: SaveRequest) {
        self.driven(move |s| match request {
            SaveRequest::As => s.save_in_place("save-as"),
            SaveRequest::InPlace => s.save_in_place("in-place"),
            SaveRequest::Copy => s.save_as(),
        });
    }

    /// `edit-request {op, contents?}` (R:1687): a missing `contents` is the menu path. A menu
    /// op lands in the mode whose row shows it before it arms anything; a bare `sign` (the
    /// old menu op) is a Place Signature press, so Fill forms.
    pub fn on_edit_request(&mut self, op: &str, contents: Option<String>) {
        let wanted_mode = if op == "sign" && contents.is_none() {
            Some(UiMode::Forms)
        } else {
            op_mode(op)
        };
        if let Some(mode) = wanted_mode
            && mode != self.ui_mode
        {
            self.set_mode(mode);
        }
        let op = op.to_string();
        self.driven(move |s| s.run_edit_request(&op, contents.as_deref()));
    }

    /// `print-request {range?}` (R:1699): the menu opens the dialog; a range prints at once.
    pub fn on_print_request(&mut self, range: Option<String>) {
        self.driven(move |s| match range {
            None => s.open_print_dialog(),
            Some(range) => s.start_print(&range),
        });
    }

    /// R:1713 `driven`: requests are serialised; each runs to the end (the engine is
    /// synchronous), then the work scheduled for later runs and the messages main queued
    /// meanwhile are delivered. The drive harness then asks `Driven::state` (R:1716:
    /// `refreshPageIds` and the STATE line).
    pub(crate) fn driven(&mut self, work: impl FnOnce(&mut Session)) {
        work(self);
        self.run_pending_tasks();
        self.pump_host_events();
    }

    /// Delivers the main → renderer messages the host queued (an open's `document`, an
    /// `open-error`, an `edit-request` the pending-boxes question sent), each through its
    /// handler. Bounded: a message may queue more.
    pub fn pump_host_events(&mut self) {
        for _ in 0..16 {
            let events = self.host.take_events();
            if events.is_empty() {
                return;
            }
            for event in events {
                self.deliver_event(event);
            }
        }
    }

    /// One main → renderer message.
    pub fn deliver_event(&mut self, event: Event) {
        match event {
            Event::Document(Document {
                name,
                url,
                untitled,
                ..
            }) => self.on_document(&name, &url, untitled),
            Event::OpenError(OpenError {
                reason,
                name,
                limit,
            }) => self.on_open_error(reason, name.as_deref(), limit.map(|l| l as f64)),
            Event::SaveRequest(request) => self.on_save_request(request),
            Event::EditRequest { op, contents } => {
                if let Some(contents) = crate::drive::preload_edit(&op, contents) {
                    self.on_edit_request(&op, contents);
                }
            }
            Event::PrintRequest { range } => self.on_print_request(range),
        }
    }

    /// R:1720 `refreshPageIds`: the current page object numbers, or None when there is
    /// nothing to ask (a failed request reports none: stale ids would lie).
    pub fn refresh_page_ids(&mut self) {
        let locked = self.doc.as_ref().is_none_or(|doc| doc.info.needs_password);
        if locked {
            self.page_ids = None;
            return;
        }
        self.page_ids = self
            .request(json!({"op": "page-ids"}), Vec::new())
            .ok()
            .and_then(|reply| super::parse_reply::<PageIdsReply>(&reply.value))
            .map(|reply| reply.ids);
    }
}

/// The drive harness drives the session through main's messages (spec-drive-state §2.6).
impl crate::drive::Driven for Session {
    fn start(&mut self, fixture: Option<&str>) {
        self.open_startup_fixture(fixture);
        self.run_pending_tasks();
        self.pump_host_events();
    }

    fn deliver_document(&mut self, doc: Document) {
        self.on_document(&doc.name, &doc.url, doc.untitled);
    }

    fn open_error(&mut self, error: OpenError) {
        self.on_open_error(
            error.reason,
            error.name.as_deref(),
            error.limit.map(|l| l as f64),
        );
    }

    fn save_request(&mut self, request: SaveRequest) {
        self.on_save_request(request);
    }

    fn edit_request(&mut self, op: &str, contents: Option<String>) {
        self.on_edit_request(op, contents);
    }

    fn print_request(&mut self, range: Option<String>) {
        self.on_print_request(range);
    }

    /// R:1716: `refreshPageIds` then `logState` (`stateSummary`, R:9926).
    fn state(&mut self) -> Value {
        self.refresh_page_ids();
        self.run_pending_tasks();
        self.state_summary()
    }

    fn take_lines(&mut self) -> Vec<(String, Value)> {
        std::mem::take(&mut self.lane1.lines)
    }

    fn host(&mut self) -> &mut Host {
        &mut self.host
    }
}
