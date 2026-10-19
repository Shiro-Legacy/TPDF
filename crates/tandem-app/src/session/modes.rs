//! Notes/edits (R:6875-6897: the dirty mirror and the announcement line) and modes and panels
//! (R:6898-7043: tablists, the inspector, the armed-tool chip, the layered Escape's parts,
//! `setMode` and `applyEdit`).
#![allow(dead_code)]

use super::Session;
use super::consts::mode_tab;
use super::dom_ids::*;
use super::thumbs::set_pressed;
use super::types::UiMode;
use crate::messages::user_text;
use serde_json::Value;
use tandem_protocol::engine::JournalState;

/// The window title (`document.title`), which the renderer wrote and nothing read back.
pub const DOCUMENT_TITLE: &str = "title";

/// The journal slice of a mutation, undo, redo or saved reply.
pub(crate) fn journal_of(reply: &Value) -> JournalState {
    super::parse_reply_or_default(reply)
}

/// The tool buttons `cancelPendingTools` unpresses (R:6975-6983), in order.
const TOOL_BUTTONS: [&str; 9] = [
    NOTE_ADD_BUTTON,
    FREE_TEXT_ADD_BUTTON,
    INK_DRAW_BUTTON,
    REDACT_AREA_BUTTON,
    CONTENT_ADD_ARM_BUTTON,
    LINK_DRAW_BUTTON,
    SIGFIELD_DRAW_BUTTON,
    TEXT_ADD_TOOL_BUTTON,
    TEXTBOX_TOOL_BUTTON,
];

impl Session {
    // ------------------------------------------------------------------ notes / edits

    /// R:6881: the worker owns dirty state; the session mirrors the last reply and tells main,
    /// which mirrors it once more for the window (setDocumentEdited).
    pub fn sync_dirty(&mut self) {
        let title = match &self.doc {
            None => "Tandem PDF".to_string(),
            Some(doc) if self.dirty => format!("• {} — Tandem PDF", doc.name),
            Some(doc) => format!("{} — Tandem PDF", doc.name),
        };
        self.dom.set_text(DOCUMENT_TITLE, title);
        self.dom.set_hidden(DIRTY_CHIP_EL, !self.dirty);
        // M9: while a restore runs the window shows no document, but its unsaved edits are not gone.
        // Review fix 8: a typed draft in an open editor counts too, so closing the window asks before it is lost.
        self.reported_dirty = self.dirty || self.restore_dirty || self.draft_pending();
        self.host.set_dirty(self.reported_dirty);
        self.report_redact_pending();
    }

    /// R:6892: the edit and nav live regions show only the latest message (STATE
    /// `announcement` reads either). X7: an engine refusal is announced in plain words too.
    pub fn announce(&mut self, text: &str) {
        self.dom.set_text(EDIT_STATUS_EL, user_text(text));
        self.dom.set_text(NAV_STATUS_EL, "");
    }

    // ------------------------------------------------------------------ modes and panels

    /// R:6901: one tablist: exactly one selected tab, roving tabindex, one visible panel.
    /// (Moving focus out of a panel that hides is the UI's.)
    pub fn select_tab(&mut self, tabs: &[&str], panels: &[&str], index: usize) {
        for (i, (tab, panel)) in tabs.iter().zip(panels).enumerate() {
            let selected = i == index;
            self.dom.set_attr(
                tab,
                "aria-selected",
                if selected { "true" } else { "false" },
            );
            self.dom.set_checked(tab, selected);
            self.dom.set_hidden(panel, !selected);
        }
    }

    /// R:6914.
    pub fn select_tab_by_id(&mut self, tabs: &[&str], panels: &[&str], id: &str) {
        if let Some(index) = tabs.iter().position(|tab| *tab == id) {
            self.select_tab(tabs, panels, index);
        }
    }

    /// R:6919 (moving focus out of a collapsing inspector is the UI's).
    pub fn set_inspector_open(&mut self, open: bool) {
        self.inspector_open = open;
        self.dom.set_hidden(INSPECTOR_EL, !open);
        self.dom.set_attr(
            INSPECTOR_TOGGLE,
            "aria-expanded",
            if open { "true" } else { "false" },
        );
    }

    /// R:6931 (X6): whether anything on the page is selected or focused (Escape clears that
    /// before it stops a tool).
    pub fn has_direct_focus(&self) -> bool {
        !self.direct_selection.is_empty()
            || self.direct_focused_object.is_some()
            || self.direct_focused_mark.is_some()
            || self.direct_focused_link.is_some()
            || self.direct_focused_box.is_some()
            || self.direct_focused_sig.is_some()
            || self.direct_focused_field.is_some()
    }

    /// R:6939: whether Escape at a target belongs to a part that handles it itself: the
    /// inspector's text and object edit forms, a dialog and the compare view, except the
    /// compare view's page-number input. `path` is the target's id and its ancestors' ids,
    /// innermost first (`closest`); an empty path is a target that is not an element.
    pub fn escape_handled_at(&self, path: &[&str]) -> bool {
        let Some(first) = path.first() else {
            return false;
        };
        if *first == COMPARE_PAGE_BINPUT {
            return false;
        }
        path.iter().any(|id| {
            *id == CONTENT_FORM
                || *id == CONTENT_OBJECT_FORM
                || *id == COMPARE_VIEW
                || id.ends_with("-dialog")
        })
    }

    /// R:6944 (X6): whether a tool button is armed.
    pub fn pressed(&self, button: &str) -> bool {
        self.dom.attr(button, "aria-pressed").as_deref() == Some("true")
    }

    /// R:6946 (X6): the armed tool in plain words, or None; the chip shows it with "Esc to
    /// stop". Redaction keeps its own row state.
    pub fn armed_tool(&self) -> Option<String> {
        if self.sign_armed {
            return Some("Placing your signature: click the page".into());
        }
        let tools: [(&str, &str); 8] = [
            (NOTE_ADD_BUTTON, "Placing a note: click the page"),
            (FREE_TEXT_ADD_BUTTON, "Placing a text box: click the page"),
            (INK_DRAW_BUTTON, "Drawing: drag on the page"),
            (LINK_DRAW_BUTTON, "Drawing a link: drag a rectangle"),
            (
                SIGFIELD_DRAW_BUTTON,
                "Drawing a signature field: drag a rectangle",
            ),
            (
                CONTENT_ADD_ARM_BUTTON,
                "Choosing where new text goes: click the page",
            ),
            (
                TEXT_ADD_TOOL_BUTTON,
                "Adding text: click empty space on the page",
            ),
            (TEXTBOX_TOOL_BUTTON, "Adding text boxes: click the page"),
        ];
        tools
            .iter()
            .find(|(button, _)| self.pressed(button))
            .map(|(_, label)| label.to_string())
    }

    /// R:6958.
    pub fn sync_tool_chip(&mut self) {
        let label = if self.doc.is_some() {
            self.armed_tool()
        } else {
            None
        };
        self.dom.set_hidden(TOOL_CHIP, label.is_none());
        self.dom.set_text(
            TOOL_CHIP_TEXT,
            label
                .map(|l| format!("{l} · Esc to stop"))
                .unwrap_or_default(),
        );
    }

    /// R:6964 (X6): Escape, the chip's button: every armed tool stops and the page is live again.
    pub fn disarm_tools(&mut self) {
        let was_sign = self.sign_armed;
        self.cancel_pending_tools();
        if was_sign {
            set_pressed(&mut self.dom, FILL_SIGN_BUTTON, false);
        }
        let (w, h) = self.canvas_client();
        self.paint_hits(w, h, self.shown_page);
        self.announce("");
        self.sync_tool_chip();
    }

    /// R:6973.
    pub fn cancel_pending_tools(&mut self) {
        self.direct_focused_field = None;
        self.clear_direct_focus();
        for button in TOOL_BUTTONS {
            set_pressed(&mut self.dom, button, false);
        }
        // draw-signature D5: a mode change or another tool disarms.
        self.sign_armed = false;
        set_pressed(&mut self.dom, FILL_SIGN_BUTTON, false);
        self.ink_stroke = None;
        // §9.14: a dialog a draw opened goes with its draft (its close listener, R:10914,
        // clears the anchor and the draft).
        if self.nav_link_draft.is_some() && self.dom.is_open(LINK_DIALOG) {
            self.dom.set_open(LINK_DIALOG, false);
            self.link_anchor = None;
        }
        self.nav_link_draft = None;
    }

    /// R:6995: switching mode shows that task's tool row and that task's inspector panel. It
    /// never touches the document: nothing is marked by choosing a mode.
    pub fn set_mode(&mut self, mode: UiMode) {
        if self.direct_active.is_some() && mode != self.ui_mode {
            self.drop_direct();
        }
        if mode == UiMode::Redact && self.ui_mode != UiMode::Redact {
            self.redact_return_mode = self.ui_mode;
        }
        self.ui_mode = mode;
        self.redact_ui_set_active(mode == UiMode::Redact);
        for (input, name, row) in MODE_INPUTS {
            self.dom.set_checked(input, name == mode);
            self.dom.set_hidden(row, name != mode);
        }
        self.cancel_pending_tools();
        if mode == UiMode::Redact {
            self.refresh_redact_preview();
        }
        if let Some(tab) = mode_tab(mode) {
            self.select_tab_by_id(&INSPECTOR_TABS, &INSPECTOR_PANELS, tab);
        }
        // Annotate and Forms work in the inspector, so a collapsed one reopens for them.
        if !self.inspector_open && matches!(mode, UiMode::Annotate | UiMode::Forms | UiMode::Edit) {
            self.set_inspector_open(true);
        }
        if mode == UiMode::Edit {
            self.refresh_content_targets(true);
        }
        let (w, h) = self.canvas_client();
        self.paint_hits(w, h, self.shown_page);
        self.announce("");
    }

    /// R:7016: the panel's empty line: a closed document is not the same as an empty page.
    pub fn set_empty(&mut self, el: &str, count: usize, page_text: &str, document_text: &str) {
        self.dom.set_hidden(el, count > 0);
        let text = if self.doc.is_some() {
            page_text
        } else {
            document_text
        };
        self.dom.set_text(el, text);
    }

    /// R:7025: mirrors one mutation/undo/redo/saved reply, then repaints the current page at
    /// the new revision, refreshes that page's thumbnail only, and reloads the lists.
    pub fn apply_edit(&mut self, reply: &JournalState, announcement: &str) {
        self.revision = reply.revision;
        self.dirty = reply.dirty;
        self.can_undo = reply.can_undo;
        self.can_redo = reply.can_redo;
        self.sync_dirty();
        self.sync_controls();
        if !announcement.is_empty() {
            self.announce(announcement);
        }
        self.refresh_notes();
        if self.ui_mode == UiMode::Redact
            || self.redact_state.as_ref().is_some_and(|s| s.marks != 0)
        {
            self.refresh_redact_preview();
        }
        self.refresh_fields();
        self.refresh_outline();
        self.refresh_links();
        self.refresh_content_targets(self.ui_mode == UiMode::Edit);
        self.invalidate_thumb(self.wanted_page);
        self.drain();
        self.pump_thumbs();
        if self.compare_doc.is_some() {
            self.refresh_compare(false, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_select_one() {
        let mut session = Session::new();
        session.select_tab_by_id(&INSPECTOR_TABS, &INSPECTOR_PANELS, "tab-text");
        assert_eq!(
            session.dom.attr("tab-text", "aria-selected").as_deref(),
            Some("true")
        );
        assert_eq!(
            session.dom.attr("tab-links", "aria-selected").as_deref(),
            Some("false")
        );
        assert!(!session.dom.hidden(TEXT_PANEL_EL));
        assert!(session.dom.hidden("panel-links"));
        session.select_tab_by_id(&INSPECTOR_TABS, &INSPECTOR_PANELS, "tab-nope");
        assert!(!session.dom.hidden(TEXT_PANEL_EL));
    }

    #[test]
    fn armed_tool_words() {
        let mut session = Session::new();
        assert_eq!(session.armed_tool(), None);
        set_pressed(&mut session.dom, INK_DRAW_BUTTON, true);
        assert_eq!(
            session.armed_tool().as_deref(),
            Some("Drawing: drag on the page")
        );
        session.sign_armed = true;
        assert_eq!(
            session.armed_tool().as_deref(),
            Some("Placing your signature: click the page")
        );
        assert!(session.escape_handled_at(&["redact-search-text", "redact-find-dialog"]));
        assert!(!session.escape_handled_at(&[COMPARE_PAGE_BINPUT, COMPARE_VIEW]));
        assert!(!session.escape_handled_at(&["page"]));
    }

    #[test]
    fn announce_is_plain_words() {
        let mut session = Session::new();
        session.dom.set_text(NAV_STATUS_EL, "Page 2");
        session.announce("Stopped redacting");
        assert_eq!(session.dom.text(EDIT_STATUS_EL), "Stopped redacting");
        assert_eq!(session.dom.text(NAV_STATUS_EL), "");
    }
}
