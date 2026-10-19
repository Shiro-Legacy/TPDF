//! The window's side of the session: what the Leptos UI draws ([`Session::ui_state`]) and
//! what it does ([`Session::ui_act`]).
//!
//! The UI follows the plan's §3.2 model — a tool strip, no modes, Esc always back to
//! Select — on top of the ported Electron logic, which still thinks in modes. Each tool is
//! therefore a mode plus, where needed, an armed sub-tool:
//!
//! | Tool | Ported mode | Armed / gesture |
//! |---|---|---|
//! | Select | read | text selection, links, fields; ⌘-click a link with any tool |
//! | Highlight | annotate | drag across text → highlight / underline / strike |
//! | Draw | annotate | pen (ink), or drag a line / rectangle / ellipse to size |
//! | Note | annotate | `note` (click to place, type at once) |
//! | Text | edit | click text to change it; click empty space to add text |
//! | Fill & Sign | forms | click a field to type; Sign… |
//! | Redact | redact | the redaction tool (drag boxes) |
//! | Pages | organize | the page grid |
//!
//! Everything the menus and dialogs do goes through the same functions the drive protocol
//! uses (`Driven::edit_request`, the element-event dispatchers), so a person, a menu and an
//! agent always reach the same code.

use crate::dom::Dom;
use crate::drive::Driven;
use crate::host::SaveRequest;
use crate::session::Session;
use crate::session::types::UiMode;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tandem_protocol::ui::*;

/// What the window adds to the session's state.
#[derive(Clone, Debug, Default)]
pub struct UiExtra {
    /// The tool strip's tool ("select" …) and its variant.
    pub tool: String,
    pub variant: Option<String>,
    /// Draw/Highlight colour and stroke width.
    pub color: [f64; 3],
    pub width: f64,
    pub panel_open: bool,
    pub panel_tab: String,
    pub sidebar_tab: String,
    /// The pages in view (overlays are built for these).
    pub visible: Vec<usize>,
    /// Bumps whenever page pixels may have changed.
    pub generation: u64,
    pub last_revision: i64,
    pub last_doc_name: Option<String>,
    /// A drag in progress: (page, start point, points so far).
    pub drag: Option<(usize, [f64; 2], Vec<[f64; 2]>)>,
    /// A success line to show once as a toast.
    pub toast: Option<String>,
    /// Text to put on the system clipboard (the shell takes it after each action).
    pub clipboard_out: Option<String>,
    /// Documents an earlier run left unsaved (D8), offered on the start screen.
    pub recovered: Vec<crate::autosave::Recovered>,
    /// What the page pixels depend on besides the revision (document, open generation,
    /// layer visibility): a change bumps `generation`.
    pub pixel_key: u64,
    /// When the toast was set (ms), so it shows for a few seconds.
    pub toast_at: f64,
    /// The last save result seen (a new one makes a toast).
    pub last_save_seen: Option<Value>,
    /// A ⌘-press followed a link: the rest of that gesture does nothing.
    pub link_press: bool,
    /// A dialog changed the mode under the strip's tool; put it back when the dialog closes.
    pub dialog_mode: bool,
    /// The rectangle to bring into view (a comment row was chosen).
    pub reveal: Option<Reveal>,
    pub reveal_seq: u64,
    /// The Pages tool's selection (page-menu items act on it while the tool is Pages).
    pub grid_selection: Vec<usize>,
    /// Where the page's context menu was asked for (page, points).
    pub menu_at: Option<(usize, f64, f64)>,
    /// The window's undo history: how many engine steps each action took (a grid action on
    /// five pages is five page ops), newest last, so one Undo or Redo takes a whole action.
    /// Cleared when another document opens; an empty stack undoes one step.
    /// Each entry also keeps what the action said it did ("Rotated page 2"), for the Undo
    /// and Redo buttons' names.
    /// The third part is the grid selection to bring back when the entry is undone (a
    /// multi-page delete's pages), empty when the action had none.
    pub undo_steps: Vec<Step>,
    pub redo_steps: Vec<Step>,
    /// The pages the action being recorded acted on (set by the grid actions).
    pub step_pages: Option<Vec<usize>>,
    /// The document the stacks belong to (open generation, name).
    pub steps_doc: Option<(u64, String)>,
    /// The certificate dialog is set aside while a box is dragged where the signature goes.
    pub cert_box: bool,
    /// The live region's text and how often it changed, and the three sources it follows
    /// as last seen (page status, edit status, navigation status).
    pub live: String,
    pub live_seq: u64,
    pub live_seen: [String; 3],
    /// The document the strip's tool was chosen on (open generation, name): another one
    /// starts with Select.
    pub tool_doc: Option<(i64, String)>,
    /// The comment, highlight, drawing or stamp the Select tool picked on the shown page.
    pub annot_selected: Option<i64>,
    /// A text box or signature being dragged with the Select tool: (id, press point, its
    /// rect at the press, the pointer now).
    pub annot_drag: Option<AnnotDrag>,
    /// The rest of this press does nothing (a press on a field's handle in Fill & Sign).
    pub press_ignored: bool,
    /// A Fill & Sign press on a signature field: a click, never a drag that moves it.
    pub press_still: Option<(usize, f64, f64)>,
    /// Each redaction box's words as last read on its own page (by box id).
    pub box_words: BTreeMap<i64, String>,
    /// Compare: a page asked for that the other file does not have, said once.
    pub compare_note: Option<String>,
    /// A signed file's redaction box (held in memory, negative id) picked on the page:
    /// Delete removes it.
    pub memory_box: Option<i64>,
}

/// A text box or signature dragged with Select: (id, press point, rect at the press,
/// pointer now), in page points.
pub type AnnotDrag = (i64, [f64; 2], [f64; 4], [f64; 2]);

/// One entry of the window's undo history: engine steps, what the action said, and the
/// grid selection Undo brings back.
pub type Step = (usize, String, Vec<usize>);

impl UiExtra {
    pub fn new() -> Self {
        UiExtra {
            tool: "select".into(),
            color: [0.97, 0.80, 0.11],
            width: 2.0,
            panel_open: false,
            panel_tab: "comments".into(),
            sidebar_tab: "pages".into(),
            ..UiExtra::default()
        }
    }
}

/// The certificate dialog's "Drag on the page…" button (tandem-ui dialogs.rs).
pub const CERT_BOX_BUTTON: &str = "cert-sign-place";

fn mode_of(tool: &str) -> UiMode {
    match tool {
        "highlight" | "draw" | "note" | "annotate" => UiMode::Annotate,
        "text" => UiMode::Edit,
        "fill-sign" | "textbox" => UiMode::Forms,
        "redact" => UiMode::Redact,
        "pages" => UiMode::Organize,
        _ => UiMode::Read,
    }
}

/// The strip's tool for a mode a menu or the session chose. Annotate maps to a neutral
/// "annotate" (no strip button pressed): a menu's Add text box or Stamp is not Highlight.
fn tool_of(mode: UiMode) -> &'static str {
    match mode {
        UiMode::Annotate => "annotate",
        UiMode::Edit => "text",
        UiMode::Forms => "fill-sign",
        UiMode::Redact => "redact",
        UiMode::Organize => "pages",
        UiMode::Read => "select",
    }
}

/// The attribute a status line's kind is kept in ([`Session::set_status_line`]).
pub const STATUS_KIND_ATTR: &str = "data-kind";

fn node_view(dom: &Dom, id: &str) -> NodeView {
    let n = dom.node(id);
    let kind = n.attrs.get(STATUS_KIND_ATTR).cloned().unwrap_or_default();
    NodeView {
        text: n.text,
        value: n.value,
        hidden: n.hidden,
        disabled: n.disabled,
        checked: n.checked,
        open: n.open,
        kind,
    }
}

/// Plain words for the dialogs' titles where the heading element has none.
fn dialog_title(id: &str) -> &'static str {
    match id {
        "note-dialog" => "Note",
        "help-dialog" => "Help",
        "redact-dialog" => "Redact",
        "redact-find-dialog" => "Find text to redact",
        "stamp-icon-dialog" => "Add a stamp",
        "sigpad-dialog" => "Your signature",
        "stamp-dialog" => "Watermark",
        "labels-dialog" => "Page labels",
        "export-dialog" => "Export",
        "insert-dialog" => "Insert pages",
        "extract-dialog" => "Extract pages",
        "bookmark-dialog" => "Bookmark",
        "link-dialog" => "Link",
        "print-dialog" => "Print",
        "verify-dialog" => "Signatures",
        "batch-dialog" => "Batch",
        "sign-choice-dialog" => "Sign",
        "cert-sign-dialog" => "Sign with a certificate",
        "security-dialog" => "Password protection",
        _ => "",
    }
}

impl Session {
    /// Sets a dialog or panel status line and what kind of line it is ("error", "warning",
    /// "info", "success"), so the window draws it by kind instead of guessing from the
    /// words (an error is red and announced). The text is what STATE shows, unchanged.
    pub fn set_status_line(&mut self, id: &str, text: impl Into<String>, kind: &str) {
        self.dom.set_text(id, text);
        self.set_status_kind(id, kind);
    }

    /// Marks the kind of a status line whose text is set elsewhere; "" forgets it.
    pub fn set_status_kind(&mut self, id: &str, kind: &str) {
        if kind.is_empty() {
            self.dom.remove_attr(id, STATUS_KIND_ATTR);
        } else {
            self.dom.set_attr(id, STATUS_KIND_ATTR, kind);
        }
    }

    /// Runs what the host queued for the window (opens, open errors, saves), then the
    /// renderer's deferred tasks, as Electron's IPC and event loop did.
    pub fn ui_pump(&mut self) {
        for _ in 0..8 {
            let events = self.host.take_events();
            if events.is_empty() {
                break;
            }
            for event in events {
                self.dispatch(event);
            }
        }
        self.run_pending_tasks();
        self.host.tick();
        // Console lines are for the drive harness; a window has no use for them.
        if !self.headless {
            self.take_lines();
        }
        self.ui_tool_for_document();
        self.ui_sync_tool();
        self.ui_rearm();
        self.ui_sync_zoom();
        self.ui_sync_generation();
        self.ui_save_toast();
        self.autosave_tick();
        self.ui_sync_live();
    }

    /// Another document (opened or new) starts with Select: a tool armed on the last one does
    /// not carry over. A tool the document does not allow (a signed file opens for reading)
    /// falls back to Select too.
    fn ui_tool_for_document(&mut self) {
        let doc = self.doc.as_ref().map(|d| (self.generation, d.name.clone()));
        if doc != self.ui.tool_doc {
            self.ui.tool_doc = doc.clone();
            self.ui.annot_selected = None;
            self.ui.annot_drag = None;
            self.ui.box_words.clear();
            self.ui.compare_note = None;
            if doc.is_some() && self.ui.tool != "select" && !self.redact_ui.active {
                self.ui_set_tool("select", None);
            }
        }
        let Some(info) = self.doc.as_ref().map(|d| d.info.clone()) else {
            return;
        };
        if info.needs_password || self.ui.tool == "select" {
            return;
        }
        let read_only = crate::session::opening::is_read_only(&info);
        let id = match self.ui.tool.as_str() {
            "annotate" => "highlight",
            other => other,
        };
        let off = self
            .ui_availability(read_only)
            .iter()
            .any(|a| a.id == id && !a.enabled);
        if off {
            self.ui_set_tool("select", None);
        }
    }

    /// The live region follows the session's latest words: what an action said (edit and
    /// navigation status), or where you are (page status) when that changed last.
    fn ui_sync_live(&mut self) {
        use crate::session::dom_ids as ids;
        let now = [
            plain_announcement(&self.dom.text(ids::STATUS_EL)),
            self.dom.text(ids::EDIT_STATUS_EL),
            self.dom.text(ids::NAV_STATUS_EL),
        ];
        let mut said = None;
        for (i, text) in now.iter().enumerate() {
            if *text != self.ui.live_seen[i] && !text.is_empty() {
                said = Some(text.clone());
            }
        }
        self.ui.live_seen = now;
        if let Some(text) = said
            && text != self.ui.live
        {
            self.ui.live = text;
            self.ui.live_seq += 1;
        }
    }

    /// Keeps the strip's tool in step when a menu, a drive step or the session changed the
    /// mode. A dialog that changes the mode (Watermark, Security, Labels) does not change the
    /// tool: the mode goes back when the dialog closes.
    fn ui_sync_tool(&mut self) {
        let mode = self.ui_mode;
        if self.redact_ui.active {
            self.ui.dialog_mode = false;
            if self.ui.tool != "redact" {
                self.ui.tool = "redact".into();
                self.ui.variant = None;
            }
            return;
        }
        if mode_of(&self.ui.tool) == mode {
            self.ui.dialog_mode = false;
            return;
        }
        if self.doc.is_some() && self.any_dialog_open() {
            self.ui.dialog_mode = true;
            return;
        }
        // Review fix 3: a dialog that armed a placement or opened an editor (the signature
        // pad's Save arms the signature) keeps its mode; the strip shows that mode's tool.
        let kept = self.armed_tool().is_some() || self.direct_active.is_some();
        if std::mem::take(&mut self.ui.dialog_mode) && self.doc.is_some() && !kept {
            let want = mode_of(&self.ui.tool);
            self.edit("mode", Some(json!({ "mode": want.as_str() })));
            if self.ui_mode == want {
                return;
            }
        }
        let (tool, variant) = if mode == UiMode::Annotate {
            // A menu armed an annotate tool: show the strip's tool when there is one.
            use crate::session::dom_ids as ids;
            if self.is_pressed(ids::NOTE_ADD_BUTTON) {
                ("note", None)
            } else if self.is_pressed(ids::INK_DRAW_BUTTON) {
                ("draw", Some("pen".to_string()))
            } else {
                ("annotate", None)
            }
        } else {
            (tool_of(mode), None)
        };
        self.ui.tool = tool.to_string();
        self.ui.variant = variant;
    }

    /// The button a tool keeps armed while it is chosen (placing tools stay armed until Esc,
    /// Phase 1 decision): the Text tool adds text in empty space, the pen draws stroke after
    /// stroke, Note places note after note.
    fn ui_standing_button(&self) -> Option<&'static str> {
        use crate::session::dom_ids as ids;
        match self.ui.tool.as_str() {
            "text" => Some(ids::TEXT_ADD_TOOL_BUTTON),
            "note" => Some(ids::NOTE_ADD_BUTTON),
            // Fill & Sign: click a field to fill it, or anywhere else to type there (round 2
            // decision); the Text box tool (Insert ▸ Text Box) places box after box.
            "fill-sign" | "textbox" => Some(ids::TEXTBOX_TOOL_BUTTON),
            "draw" if self.ui.variant.as_deref().is_none_or(|v| v == "pen") => {
                Some(ids::INK_DRAW_BUTTON)
            }
            _ => None,
        }
    }

    /// Re-arms the tool's standing button after a placement or a stroke released it.
    fn ui_rearm(&mut self) {
        let Some(button) = self.ui_standing_button() else {
            return;
        };
        let allowed = if self.ui.tool == "text" {
            self.can_edit()
        } else {
            self.can_annotate()
        };
        if !allowed
            || self.doc.is_none()
            || self.ui_mode != mode_of(&self.ui.tool)
            || self.any_dialog_open()
            || self.direct_active.is_some()
            || self.ink_stroke.is_some()
            || self.dom.disabled(button)
            || self.armed_tool().is_some()
        {
            return;
        }
        self.set_tool_pressed(button, true);
        self.paint_hits_now();
    }

    /// The session places editors and hit targets at the zoom of its last paint
    /// (`shown_zoom`); the window draws at `zoom_now`. A viewport resize (fit zoom) changes
    /// `zoom_now` without a paint, so repaint until the two agree: one zoom for the page
    /// pixels, the overlay and the inline editors.
    fn ui_sync_zoom(&mut self) {
        let Some(doc) = &self.doc else { return };
        if doc.info.needs_password || self.in_flight.is_some() {
            return;
        }
        let page = self.wanted_page;
        if self.shown_page == page && self.shown_zoom == self.zoom_now(page) {
            return;
        }
        // The repaint places an open editor again at the new zoom.
        self.sync_zoom();
        self.drain();
    }

    /// Bumps `generation` when the pixels may have changed: an edit (the revision), another
    /// document, or a change that repaints without a revision (a layer shown or hidden).
    fn ui_sync_generation(&mut self) {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.revision.hash(&mut h);
        self.generation.hash(&mut h);
        self.doc.as_ref().map(|d| &d.name).hash(&mut h);
        for layer in &self.layers {
            (layer.id, layer.visible).hash(&mut h);
        }
        let key = h.finish();
        if key != self.ui.pixel_key {
            self.ui.pixel_key = key;
            self.ui.generation += 1;
        }
    }

    /// Save, Save As and Save a Copy say so in a toast, not only to screen readers.
    fn ui_save_toast(&mut self) {
        let last = self.last_save.clone().map(Value::Object);
        if last == self.ui.last_save_seen {
            return;
        }
        self.ui.last_save_seen = last.clone();
        let ok = last
            .as_ref()
            .and_then(|l| l.get("ok"))
            .and_then(Value::as_bool)
            == Some(true);
        let status = self.dom.text(crate::session::dom_ids::STATUS_EL);
        if ok && !status.is_empty() {
            let text = if status.ends_with('.') {
                status
            } else {
                format!("{status}.")
            };
            self.ui_toast(text);
        }
    }

    fn ui_toast(&mut self, text: String) {
        self.ui.toast = Some(text);
        self.ui.toast_at = crate::engine::now_ms();
    }

    fn edit(&mut self, op: &str, contents: Option<Value>) {
        let text = contents.map(|c| match c {
            Value::String(s) => s,
            other => other.to_string(),
        });
        self.edit_request(op, text);
    }

    /// Chooses a tool on the strip (§3.2).
    pub fn ui_set_tool(&mut self, id: &str, variant: Option<String>) {
        let id = if id == "fill" || id == "sign" {
            "fill-sign"
        } else {
            id
        };
        // A tool the document cannot use stays off and says why (open#12).
        if let Some(doc) = &self.doc {
            let read_only = crate::session::opening::is_read_only(&doc.info);
            if let Some(why) = self
                .ui_availability(read_only)
                .into_iter()
                .find(|a| a.id == id && !a.enabled)
                .and_then(|a| a.reason)
            {
                self.announce(&why);
                return;
            }
        }
        let leaving_redact = self.redact_ui.active && id != "redact";
        if leaving_redact {
            self.redact_ui_set_active(false);
            // The red "what goes" tint belongs to the tool (review redact#8).
            self.redact_ui.tint = None;
        }
        self.ui.menu_at = None;
        self.cancel_pending_tools();
        self.ui.drag = None;
        self.ui.annot_selected = None;
        self.ui.annot_drag = None;
        self.ui.memory_box = None;
        let mode = mode_of(id);
        if self.ui_mode != mode {
            self.edit("mode", Some(json!({ "mode": mode.as_str() })));
        }
        self.ui.tool = id.to_string();
        self.ui.variant = variant.or_else(|| match id {
            "highlight" => Some("highlight".into()),
            "draw" => Some("pen".into()),
            _ => None,
        });
        match id {
            "note" => self.edit("arm", Some(json!({ "tool": "note" }))),
            "redact" if !self.redact_ui.active => self.on_redact_area_click(false),
            "draw" if self.ui.variant.as_deref() == Some("pen") => {
                self.edit("arm", Some(json!({ "tool": "ink" })))
            }
            "pages" => {}
            _ => {}
        }
        if id == "draw" || id == "highlight" {
            self.ui.color = if id == "highlight" {
                [0.97, 0.80, 0.11]
            } else {
                [0.12, 0.24, 0.55]
            };
        }
    }

    /// Esc: one layer per press (an open editor, then a selection, then the tool), and
    /// always back to Select at the end (X6).
    pub fn ui_escape(&mut self) {
        // Dragging the certificate signature's box: Esc goes back to the dialog as it was.
        if self.ui.cert_box {
            self.ui.cert_box = false;
            self.ui.drag = None;
            return;
        }
        if self.any_dialog_open() {
            if let Some((id, _)) = crate::dialog_ids::DIALOGS
                .iter()
                .find(|(id, _)| self.dom.is_open(id))
            {
                self.on_dialog_cancel(id);
            }
            return;
        }
        // Compare closes on Esc like a sheet.
        if self.compare_doc.is_some() {
            self.close_compare();
            return;
        }
        if self.ui.menu_at.is_some() && !self.dom.hidden(crate::session::dom_ids::OBJECT_MENU) {
            self.ui.menu_at = None;
            self.close_object_menu(true);
            return;
        }
        if self.ui.memory_box.take().is_some() && self.redact_ui.active {
            return;
        }
        // A picked annotation is let go first.
        if self.ui.annot_selected.take().is_some() {
            self.ui.annot_drag = None;
            return;
        }
        // The tool's own standing arm is not a layer of its own: with nothing else to undo,
        // Esc goes back to Select.
        if let Some(button) = self.ui_standing_button()
            && self.is_pressed(button)
        {
            self.set_tool_pressed(button, false);
        }
        let before = (
            self.armed_tool(),
            self.has_direct_focus(),
            self.direct_active.is_some(),
            self.redact_ui.active,
        );
        self.edit("key", Some(json!({ "key": "Escape" })));
        let after = (
            self.armed_tool(),
            self.has_direct_focus(),
            self.direct_active.is_some(),
            self.redact_ui.active,
        );
        if before == after && self.ui.tool != "select" {
            self.ui_set_tool("select", None);
        }
    }

    /// One action from the window.
    pub fn ui_act(&mut self, action: UiAction) {
        let before = self.engine.journal_position();
        let said_before = self.dom.text(crate::session::dom_ids::EDIT_STATUS_EL);
        let undo = undo_kind(&action);
        match undo {
            Some(redo) => self.ui_undo(redo),
            None => self.ui_act_one(action),
        }
        self.ui_pump();
        self.ui_record_steps(undo.is_none(), before, &said_before);
    }

    /// Undo (or Redo) one whole action of the window: as many engine steps as it took.
    fn ui_undo(&mut self, redo: bool) {
        self.ui_steps_doc();
        let (steps, label, pages) = if redo {
            self.ui.redo_steps.last().cloned()
        } else {
            self.ui.undo_steps.last().cloned()
        }
        .unwrap_or((1, String::new(), Vec::new()));
        let steps = steps.max(1);
        let mut done = 0;
        for _ in 0..steps {
            let can = if redo { self.can_redo } else { self.can_undo };
            if done > 0 && !can {
                break;
            }
            let r0 = self.revision;
            if redo {
                self.redo_edit();
            } else {
                self.undo_edit();
            }
            // The first press only closed an editor, or nothing was there to undo.
            if self.revision == r0 {
                break;
            }
            done += 1;
        }
        if done == 0 {
            return;
        }
        let (from, to) = if redo {
            (&mut self.ui.redo_steps, &mut self.ui.undo_steps)
        } else {
            (&mut self.ui.undo_steps, &mut self.ui.redo_steps)
        };
        from.pop();
        to.push((done, label, pages.clone()));
        // Undoing a grid action brings its pages back as the selection (a multi-page delete
        // selects the pages it brings back, not one).
        if !redo && !pages.is_empty() {
            let count = self
                .doc
                .as_ref()
                .map_or(0, |d| d.page_count.max(0) as usize);
            let back: Vec<usize> = pages.into_iter().filter(|p| *p < count).collect();
            if !back.is_empty() {
                self.ui.grid_selection = back;
            }
        }
        if done > 1 {
            let verb = if redo { "Redid" } else { "Undid" };
            self.announce(&format!("{verb} {done} page changes"));
        }
    }

    /// The stacks belong to the open document: another one starts them afresh.
    fn ui_steps_doc(&mut self) {
        let doc = self
            .doc
            .as_ref()
            .map(|d| (self.generation.max(0) as u64, d.name.clone()));
        if doc != self.ui.steps_doc {
            self.ui.steps_doc = doc;
            self.ui.undo_steps.clear();
            self.ui.redo_steps.clear();
        }
    }

    /// After an action that was not Undo or Redo: the journal steps it took are one entry.
    /// Counted on the journal's position, not the revision: a `security-set` or a mutation
    /// that changed nothing moves the revision without a step (review fix 2).
    fn ui_record_steps(&mut self, record: bool, before: Option<i64>, said_before: &str) {
        let pages = self.ui.step_pages.take().unwrap_or_default();
        let opened = self
            .doc
            .as_ref()
            .map(|d| (self.generation.max(0) as u64, d.name.clone()));
        if opened != self.ui.steps_doc {
            self.ui_steps_doc();
            return;
        }
        let steps = match (before, self.engine.journal_position()) {
            (Some(before), Some(after)) if after > before => (after - before) as usize,
            _ => 0,
        };
        if record && steps > 0 {
            // What this action said it did; an action that said nothing is named after the
            // engine's step ("Add stamp"), in plain words (the same words again are the same
            // kind of action repeated).
            let said = self.dom.text(crate::session::dom_ids::EDIT_STATUS_EL);
            let label = plain_step(&said);
            let repeated = self
                .ui
                .undo_steps
                .last()
                .is_some_and(|(_, l, _)| *l == label);
            let label = if (said != said_before || repeated) && !label.is_empty() {
                label
            } else {
                self.ui_engine_step_name().unwrap_or_default()
            };
            self.ui.undo_steps.push((steps, label, pages));
            self.ui.redo_steps.clear();
            if self.ui.undo_steps.len() > 200 {
                self.ui.undo_steps.remove(0);
            }
        }
    }

    /// The name of the step the journal applied last, as an Undo name (marked as such).
    fn ui_engine_step_name(&self) -> Option<String> {
        let debug = self.engine.debug_json();
        let journal = debug.get("journal")?;
        let position = journal.get("position")?.as_i64()?;
        let steps = journal.get("steps")?.as_array()?;
        let name = steps
            .get(usize::try_from(position - 1).ok()?)?
            .as_str()?
            .trim();
        (!name.is_empty()).then(|| format!("{ENGINE_STEP_MARK}{}", engine_step_words(name)))
    }

    /// One action from the window (without the pump).
    fn ui_act_one(&mut self, action: UiAction) {
        match action {
            // Insert ▸ Text Box arms the text box tool: a click places the box with its editor
            // there (annotate#7), not a dialog at a fixed spot.
            UiAction::Edit { op, contents: None } if op == "freetext" => {
                self.ui_set_tool("textbox", None)
            }
            UiAction::Click { id } if id == crate::session::dom_ids::TEXTBOX_TOOL_BUTTON => {
                let again = self.ui.tool == "textbox";
                self.ui_set_tool(if again { "select" } else { "textbox" }, None);
            }
            UiAction::Edit { op, contents } if op == "redact-selection" => {
                let _ = contents;
                // A box over each selected line, then the Redact tool's panel shows them.
                let page = self.wanted_page;
                let rects: Vec<[f64; 4]> = self.ui_overlay(page).selection;
                for r in rects {
                    self.edit("redact", Some(json!({ "page": page, "rect": r })));
                }
                self.on_text_select(None);
                self.ui_set_tool("redact", None);
            }
            UiAction::Edit { op, contents }
                if matches!(op.as_str(), "highlight" | "underline" | "strikeout")
                    && contents == Some(json!("selection")) =>
            {
                // The selection bar: mark the selected text, and stay with the tool the
                // person had. Without a selection there is nothing to mark (the drive's
                // smoke path would search for its sample word instead).
                if self.selection_markup().is_some() {
                    let tool = self.ui.tool.clone();
                    let variant = self.ui.variant.clone();
                    self.edit(&op, Some(json!("selection")));
                    self.on_text_select(None);
                    self.ui_keep_tool(&tool, variant);
                }
            }
            // In the Pages tool the Page menu (and the toolbar's page buttons) act on the
            // grid's selection, not only on the current page (review pages#12).
            UiAction::Edit { op, .. }
                if self.ui.tool == "pages"
                    && !self.ui.grid_selection.is_empty()
                    && matches!(
                        op.as_str(),
                        "rotate-right" | "rotate-left" | "delete-page" | "move-up" | "move-down"
                    ) =>
            {
                let pages = self.ui.grid_selection.clone();
                let (action, to) = match op.as_str() {
                    "rotate-right" => ("rotate-right", None),
                    "rotate-left" => ("rotate-left", None),
                    "delete-page" => ("delete", None),
                    "move-up" => ("move", Some(pages[0].saturating_sub(1))),
                    _ => ("move", Some(pages[pages.len() - 1] + 2)),
                };
                self.ui_pages(action, pages, to);
            }
            UiAction::Edit { op, contents } => {
                let tool = self.ui.tool.clone();
                let variant = self.ui.variant.clone();
                let mode_op = op == "mode";
                self.edit(&op, contents);
                if !mode_op {
                    self.ui_after_menu_edit(&tool, variant);
                }
            }
            UiAction::Open { path: None } => self.host.open_file(),
            UiAction::Open { path: Some(path) } => {
                if self.host.recent_list().iter().any(|r| r.id == path) {
                    self.host.recent_open(&path);
                } else {
                    self.host.open_path(&path);
                }
            }
            UiAction::New { size } => self.host.new_blank(size.map(Value::String).as_ref()),
            UiAction::Save { how } => {
                let request = match how.as_str() {
                    "as" => SaveRequest::As,
                    "copy" => SaveRequest::Copy,
                    _ => SaveRequest::InPlace,
                };
                self.save_request(request);
            }
            // The macOS print panel has its own page ranges and preview: print everything.
            UiAction::Print => self.print_request(Some(String::new())),
            UiAction::Tool { id, variant } => self.ui_set_tool(&id, variant),
            UiAction::Goto { page } => {
                // Next page on the last page stays there (no range error).
                let last = self
                    .doc
                    .as_ref()
                    .map_or(0, |d| (d.page_count - 1).max(0) as usize);
                self.edit("goto", Some(json!({ "page": page.min(last) })));
            }
            UiAction::Zoom { to, scale } => match to.as_str() {
                // From a fit zoom, step from the scale shown (the widest page's fit).
                "in" | "out" => {
                    if self.zoom_mode != crate::session::types::ZoomMode::Fixed {
                        let shown = self.ui_scale();
                        self.ui_zoom_fixed(shown);
                    }
                    self.zoom_step(if to == "in" { 1 } else { -1 });
                }
                "actual" => self.ui_zoom_fixed(1.0),
                "fit-width" | "fit-page" => self.edit("zoom", Some(json!({ "mode": to }))),
                _ => self.ui_zoom_fixed(scale.unwrap_or(1.0)),
            },
            // ui_pump repaints at the new fit zoom (ui_sync_zoom).
            UiAction::Viewport { width, height } => self.set_viewport(width, height),
            UiAction::Search { query } => {
                if query.is_empty() {
                    self.on_search_input("");
                } else {
                    self.edit("find", Some(json!({ "query": query })));
                }
            }
            UiAction::SearchMove { by } => self.edit(
                "find",
                Some(json!({ "dir": if by < 0 { "prev" } else { "next" } })),
            ),
            UiAction::Pointer {
                phase,
                page,
                x,
                y,
                shift,
                alt,
                meta,
            } => self.ui_pointer(&phase, page, x, y, shift, alt, meta),
            UiAction::Key {
                key,
                shift,
                alt,
                meta,
            } => {
                if key == "Escape" {
                    self.ui_escape();
                } else if let Some(id) = self.ui.memory_box.filter(|_| {
                    matches!(key.as_str(), "Delete" | "Backspace") && self.redact_ui.active
                }) {
                    self.ui.memory_box = None;
                    if let Some(b) = self.redact_ui.boxes.iter().find(|b| b.id == id).cloned() {
                        self.on_redact_panel_remove(b.page, b.id);
                    }
                } else if meta || alt || !self.ui_annot_key(&key, shift) {
                    self.key(&key, shift, alt, meta);
                }
            }
            UiAction::EditorInput { value } => self.ui_editor_input(&value),
            UiAction::EditorCommit => self.key("Enter", false, false, true),
            UiAction::Dialog {
                id: _,
                button,
                values,
            } => {
                if let Value::Object(map) = values {
                    for (field, value) in map {
                        match value {
                            Value::Bool(b) => {
                                self.on_change(&field, "", b);
                            }
                            other => {
                                let text = other
                                    .as_str()
                                    .map(str::to_string)
                                    .unwrap_or_else(|| other.to_string());
                                self.on_input(&field, &text);
                            }
                        }
                    }
                }
                if button == CERT_BOX_BUTTON {
                    self.ui_cert_box_start();
                } else if !button.is_empty() {
                    self.on_click(&button);
                }
            }
            UiAction::Click { id } if id == CERT_BOX_BUTTON => self.ui_cert_box_start(),
            UiAction::Click { id } => {
                self.on_click(&id);
            }
            UiAction::Input { id, mut value } => {
                // A page the other file does not have shows its last page and says so.
                if id == "compare-page-b"
                    && let Some(c) = &self.compare_doc
                    && let Ok(n) = value.trim().parse::<i64>()
                {
                    let last = c.page_count.max(1);
                    if !(1..=last).contains(&n) {
                        let shown = n.clamp(1, last);
                        let plural = if last == 1 { "" } else { "s" };
                        // Said where Compare shows its summary, not only in a toast the
                        // compare view covers (pages#11).
                        let note = format!(
                            "Page {n} doesn't exist: {} has {last} page{plural}. Showing page {shown}.",
                            c.name
                        );
                        value = shown.to_string();
                        self.ui.compare_note = Some(note.clone());
                        self.ui_toast(note);
                    } else {
                        self.ui.compare_note = None;
                    }
                }
                if !self.on_input(&id, &value) {
                    self.on_change(&id, &value, false);
                }
            }
            UiAction::Check { id, checked } => {
                if let Some(rest) = id.strip_prefix("radio:") {
                    // A radio group without element ids: `radio:<name>:<value>`.
                    let mut parts = rest.splitn(2, ':');
                    let (name, value) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                    self.on_change(&format!("radio:{name}"), value, true);
                    for (other, _) in crate::dialog_ids::DIALOGS
                        .iter()
                        .flat_map(|(_, ids)| ids.iter())
                    {
                        if other.starts_with(&format!("radio:{name}:")) {
                            self.dom.set_checked(other, *other == id);
                        }
                    }
                } else {
                    self.on_change(&id, "", checked);
                }
            }
            UiAction::DialogCancel { id } => {
                self.on_dialog_cancel(&id);
            }
            UiAction::Notice { action } => {
                if let Some(id) = action.strip_prefix("restore:") {
                    self.ui.recovered.clear();
                    self.autosave_restore(id);
                } else if let Some(id) = action.strip_prefix("forget:") {
                    crate::autosave::discard(self.host.user_data(), id);
                    self.ui.recovered.retain(|r| r.id != id);
                } else if action == "dismiss" {
                    self.clear_error();
                } else if action == "verify-open" {
                    self.edit("verify-open", None);
                } else if action == "editable-copy" {
                    self.edit("editable-copy", None);
                } else if action == "redact-banner-original" {
                    self.on_redact_banner_original();
                } else if action == "redact-banner-dismiss" {
                    self.on_redact_banner_dismiss();
                } else {
                    self.on_click(&action);
                }
            }
            UiAction::Pane { which, open, tab } => {
                if which == "sidebar" {
                    let open = open.unwrap_or(!self.sidebar_open);
                    if let Some(tab) = tab {
                        self.ui.sidebar_tab = tab.clone();
                        self.on_tab_click(if tab == "outline" {
                            "tab-bookmarks"
                        } else {
                            "tab-pages"
                        });
                    }
                    if open != self.sidebar_open {
                        self.set_thumb_sidebar(open);
                    }
                } else {
                    self.ui.panel_open = open.unwrap_or(!self.ui.panel_open);
                    if let Some(tab) = tab {
                        self.ui.panel_tab = tab;
                        self.ui.panel_open = true;
                    }
                    let tab = match self.ui.panel_tab.as_str() {
                        "fields" => Some("tab-fields"),
                        "links" => Some("tab-links"),
                        "properties" => Some("tab-text"),
                        "layers" => Some("tab-layers"),
                        "comments" => Some("tab-notes"),
                        _ => None,
                    };
                    if let Some(tab) = tab {
                        self.on_tab_click(tab);
                    }
                }
            }
            UiAction::Row { list, id, action } => self.ui_row(&list, &id, &action),
            UiAction::Visible { pages } => self.ui.visible = pages,
            UiAction::Pages { action, pages, to } => self.ui_pages(&action, pages, to),
            UiAction::Focus { target } if target.starts_with("annot-") => {
                let id = target["annot-".len()..].parse::<i64>().ok();
                if id.is_some() && self.ui.annot_selected != id {
                    self.ui_pick_annot(id);
                }
            }
            UiAction::Focus { target } => {
                let key = self
                    .lane2
                    .hit
                    .nodes
                    .iter()
                    .find(|n| n.seq.to_string() == target)
                    .map(|n| n.key());
                if let Some(key) = key
                    && self.lane2.focus != crate::session::hit::Focus::Node(key.clone())
                {
                    self.focus_node(&key);
                }
            }
            UiAction::ContextMenu { page, x, y } => {
                if self.doc.is_some() {
                    if page as i64 != self.wanted_page {
                        self.edit("goto", Some(json!({ "page": page })));
                    }
                    self.context_menu(page, x, y);
                    self.ui.menu_at = (!self.dom.hidden(crate::session::dom_ids::OBJECT_MENU))
                        .then_some((page, x, y));
                }
            }
            UiAction::MenuItem { item } => {
                self.ui.menu_at = None;
                if item == "close" {
                    self.close_object_menu(true);
                } else if let Some((key, _)) = crate::session::dom_ids::OBJECT_MENU_BUTTONS
                    .iter()
                    .find(|(key, id)| *key == item || *id == item)
                {
                    self.activate_object_menu(key);
                }
            }
            UiAction::Unlock { password } => self.on_password_submit(&password),
            UiAction::Clipboard { what } => {
                let text = self
                    .selection_markup()
                    .map(|m| m.text.clone())
                    .filter(|t| !t.is_empty());
                match (what.as_str(), text) {
                    ("copy", Some(text)) => {
                        self.ui.clipboard_out = Some(text);
                        self.announce("Copied");
                        // Said again on every copy, also the same words twice.
                        self.ui.live_seq += 1;
                        self.ui.live = "Copied".into();
                    }
                    (kind, _) => {
                        self.on_clipboard_event(kind);
                    }
                }
            }
            UiAction::SignaturePad {
                strokes,
                width,
                height,
            } => {
                // The pad's strokes, scaled to the sign-draw contract (pad pixels).
                let _ = (width, height);
                self.edit("sign-draw", Some(json!({ "strokes": strokes })));
            }
        }
    }

    /// Puts the strip's tool (and its mode) back after an op that moved the mode.
    fn ui_keep_tool(&mut self, tool: &str, variant: Option<String>) {
        if self.redact_ui.active && tool != "redact" {
            return;
        }
        let want = mode_of(tool);
        if self.ui_mode != want {
            self.edit("mode", Some(json!({ "mode": want.as_str() })));
        }
        if self.ui_mode == want {
            self.ui.tool = tool.to_string();
            self.ui.variant = variant;
        }
    }

    /// A menu's op changed the mode (OP_MODE): a dialog keeps the tool until it closes
    /// (`ui_sync_tool`); an armed placement or an open editor shows its own tool; anything
    /// else (Rotate, Delete page from the menu) leaves the strip's tool as it was.
    fn ui_after_menu_edit(&mut self, tool: &str, variant: Option<String>) {
        if self.doc.is_none() || self.redact_ui.active || self.ui_mode == mode_of(tool) {
            return;
        }
        if self.any_dialog_open() {
            self.ui.dialog_mode = true;
            return;
        }
        if self.armed_tool().is_some()
            || self.direct_active.is_some()
            || self.compare_doc.is_some()
            || self.ui_mode == UiMode::Redact
        {
            return;
        }
        self.ui_keep_tool(tool, variant);
    }

    /// The one scale the window draws every page at. A fit zoom fits the widest (fit page:
    /// the largest) page, so a document mixing portrait and landscape pages keeps one scale
    /// as it scrolls and no page overflows (the session's own fit is per shown page; it
    /// places hit targets and editors in page points, so the two need not agree).
    pub fn ui_scale(&self) -> f64 {
        let page = self.wanted_page.max(0);
        let Some(doc) = &self.doc else {
            return self.zoom_now(page);
        };
        if self.zoom_mode == crate::session::types::ZoomMode::Fixed || doc.pages.len() < 2 {
            return self.zoom_now(page);
        }
        (0..doc.pages.len() as i64)
            .map(|p| self.zoom_now(p))
            .fold(f64::INFINITY, f64::min)
            .min(self.zoom_now(page))
    }

    fn ui_zoom_fixed(&mut self, scale: f64) {
        let scale = scale.clamp(0.1, 8.0);
        self.set_zoom(crate::session::types::ZoomMode::Fixed, scale);
    }

    /// A pointer gesture on a page (PDF points), interpreted for the current tool.
    #[allow(clippy::too_many_arguments)]
    fn ui_pointer(
        &mut self,
        phase: &str,
        page: usize,
        x: f64,
        y: f64,
        shift: bool,
        alt: bool,
        meta: bool,
    ) {
        if self.doc.is_none() {
            return;
        }
        if self.ui.cert_box {
            self.ui_cert_box_pointer(phase, page, x, y);
            return;
        }
        if phase == "down" && page as i64 != self.wanted_page {
            self.edit("goto", Some(json!({ "page": page })));
        }
        if phase == "down" {
            self.ui.menu_at = None;
            if !self.dom.hidden(crate::session::dom_ids::OBJECT_MENU) {
                self.close_object_menu(false);
            }
        }
        // ⌘-click follows a link whatever the tool (review view#9).
        if phase == "down" {
            self.ui.link_press = false;
            if meta && let Some(link) = self.ui_link_at(page, x, y) {
                self.ui.link_press = true;
                self.ui.drag = None;
                self.follow_link(&link);
                return;
            }
        } else if self.ui.link_press {
            if phase == "up" || phase == "cancel" {
                self.ui.link_press = false;
            }
            return;
        }
        let tool = self.ui.tool.clone();
        if phase == "down" {
            self.ui.press_ignored = false;
            self.ui.press_still = None;
        } else if self.ui.press_ignored {
            if phase == "up" || phase == "cancel" {
                self.ui.press_ignored = false;
            }
            return;
        }
        // Fill & Sign fills fields, it does not move them: a press on a field's handle does
        // nothing, and a press on a signature field is a click (sign it), never a drag.
        if tool == "fill-sign"
            && phase == "down"
            && let Some((index, on_handle)) = self.ui_hit_node_at(page, x, y)
        {
            use crate::session::hit::NodeKind;
            let kind = self.lane2.hit.nodes.get(index).map(|n| n.kind.clone());
            match kind {
                Some(NodeKind::Field { .. } | NodeKind::SigField { .. }) if on_handle => {
                    self.ui.press_ignored = true;
                    return;
                }
                Some(NodeKind::SigField { .. }) => self.ui.press_still = Some((page, x, y)),
                _ => {}
            }
        }
        if let Some((p, sx, sy)) = self.ui.press_still
            && phase != "down"
        {
            if phase == "up" || phase == "cancel" {
                self.ui.press_still = None;
                self.pointer(phase, p, sx, sy, shift, alt, meta);
            }
            return;
        }
        if tool == "select" && self.ui_annot_pointer(phase, page, x, y) {
            return;
        }
        // Redact on a signed file: its boxes (held in memory) cannot move, so a press inside
        // one picks it instead of drawing a box inside the box (redact#11).
        if tool == "redact"
            && phase == "down"
            && let Some(b) = self
                .redact_ui
                .boxes
                .iter()
                .filter(|b| b.id < 0 && b.page == page as i64 && b.rect.len() == 4)
                .find(|b| {
                    let r = &b.rect;
                    x >= r[0].min(r[2])
                        && x <= r[0].max(r[2])
                        && y >= r[1].min(r[3])
                        && y <= r[1].max(r[3])
                })
                .cloned()
        {
            self.ui.press_ignored = true;
            self.ui.memory_box = Some(b.id);
            return;
        }
        if phase == "down" {
            self.ui.memory_box = None;
        }
        let variant = self.ui.variant.clone().unwrap_or_default();
        let shaped = tool == "draw" && variant != "pen";
        if tool == "highlight" || shaped {
            match phase {
                "down" => self.ui.drag = Some((page, [x, y], vec![[x, y]])),
                "move" => {
                    if let Some((_, start, points)) = self.ui.drag.as_mut() {
                        points.push([x, y]);
                        let start = *start;
                        if tool == "highlight" {
                            self.ui_select_text(start, [x, y]);
                        }
                    }
                }
                "up" => {
                    if let Some((_, start, _)) = self.ui.drag.take() {
                        if tool == "highlight" {
                            self.ui_select_text(start, [x, y]);
                            let op = match variant.as_str() {
                                "underline" => "underline",
                                "strike" => "strikeout",
                                _ => "highlight",
                            };
                            if self.selection_markup().is_some() {
                                self.edit(op, Some(json!("selection")));
                                self.on_text_select(None);
                            }
                        } else {
                            self.ui_add_shape(&variant, start, [x, y]);
                        }
                    }
                }
                _ => self.ui.drag = None,
            }
            return;
        }
        if tool == "select" && phase != "cancel" {
            // Text selection by dragging on the page (§3.2 Select).
            match phase {
                "down" => self.ui.drag = Some((page, [x, y], vec![])),
                "move" => {
                    if let Some((_, start, _)) = self.ui.drag.clone()
                        && (start[0] - x).abs() + (start[1] - y).abs() > 2.0
                    {
                        self.ui_select_text(start, [x, y]);
                        return;
                    }
                }
                "up" => {
                    if let Some((_, start, _)) = self.ui.drag.take() {
                        if (start[0] - x).abs() + (start[1] - y).abs() > 2.0 {
                            self.ui_select_text(start, [x, y]);
                            return;
                        }
                        self.on_text_select(None);
                        // A click on a comment, highlight or drawing picks it (annotate#10).
                        let picked = if self.ui_hit_node_at(page, x, y).is_some() {
                            None
                        } else {
                            self.ui_annot_at(x, y).map(|(n, _)| n.id)
                        };
                        self.ui_pick_annot(picked);
                    }
                }
                _ => {}
            }
        }
        self.pointer(phase, page, x, y, shift, alt, meta);
    }

    /// Select picks an annotation: its comment row is the current one, and keys act on it.
    fn ui_pick_annot(&mut self, id: Option<i64>) {
        self.ui.annot_selected = id;
        if id.is_some() {
            self.lane2.note_row_current = id;
        }
    }

    /// Select on a text box or placed signature: the press picks it and a drag moves it.
    /// Returns whether the gesture was the annotation's.
    fn ui_annot_pointer(&mut self, phase: &str, page: usize, x: f64, y: f64) -> bool {
        match phase {
            "down" => {
                let hit = if self.ui_hit_node_at(page, x, y).is_some() {
                    None
                } else {
                    self.ui_annot_at(x, y)
                };
                match hit {
                    Some((item, true)) => {
                        let r = [item.rect[0], item.rect[1], item.rect[2], item.rect[3]];
                        self.ui_pick_annot(Some(item.id));
                        self.ui.annot_drag = Some((item.id, [x, y], r, [x, y]));
                        self.ui.drag = None;
                        self.on_text_select(None);
                        true
                    }
                    Some((item, false)) if self.ui.annot_selected == Some(item.id) => false,
                    _ => {
                        self.ui.annot_selected = None;
                        false
                    }
                }
            }
            "move" => match self.ui.annot_drag.as_mut() {
                Some(drag) => {
                    drag.3 = [x, y];
                    true
                }
                None => false,
            },
            "up" | "cancel" => {
                let Some((id, start, r, _)) = self.ui.annot_drag.take() else {
                    return false;
                };
                let (dx, dy) = (x - start[0], y - start[1]);
                if phase == "up" && dx.abs() + dy.abs() > 1.0 {
                    self.ui_move_annot(id, page, r, dx, dy);
                }
                true
            }
            _ => false,
        }
    }

    /// Moves a text box or placed signature by (dx, dy) points, kept on the page.
    fn ui_move_annot(&mut self, id: i64, page: usize, r: [f64; 4], dx: f64, dy: f64) {
        let Some(item) = self.notes.iter().find(|n| n.id == id).cloned() else {
            return;
        };
        let (pw, ph) = self
            .doc
            .as_ref()
            .and_then(|d| d.pages.get(page))
            .map_or((f64::MAX, f64::MAX), |p| (p.width, p.height));
        let (w, h) = (r[2] - r[0], r[3] - r[1]);
        let x0 = (r[0] + dx).clamp(0.0, (pw - w).max(0.0));
        let y0 = (r[1] + dy).clamp(0.0, (ph - h).max(0.0));
        let rect = [x0, y0, x0 + w, y0 + h];
        if crate::session::direct::is_signature(&item) {
            self.commit_signature_rect(id, page as i64, &r, &rect, false);
        } else {
            self.commit_box_move(id, page as i64, &rect);
        }
        self.ui.annot_selected = Some(id);
    }

    /// Keys for the annotation Select picked: Delete removes it, Return opens its text,
    /// arrows move a text box or signature (⇧ by 10 points). Returns whether it took the key.
    fn ui_annot_key(&mut self, key: &str, shift: bool) -> bool {
        let Some(id) = self.ui.annot_selected else {
            return false;
        };
        if self.ui.tool != "select" || self.any_dialog_open() || self.direct_active.is_some() {
            return false;
        }
        let Some((item, movable)) = self.ui_annots().into_iter().find(|(n, _)| n.id == id) else {
            self.ui.annot_selected = None;
            return false;
        };
        match key {
            "Delete" | "Backspace" => {
                if !self.can_annotate() {
                    let read_only = self
                        .doc
                        .as_ref()
                        .is_some_and(|d| crate::session::opening::is_read_only(&d.info));
                    let why = self
                        .ui_availability(read_only)
                        .into_iter()
                        .find(|a| a.id == "note")
                        .and_then(|a| a.reason)
                        .unwrap_or_else(|| "This file does not allow comments.".into());
                    self.announce(&why);
                    return true;
                }
                self.ui.annot_selected = None;
                self.on_note_list_action("delete", id);
                true
            }
            "Enter" => {
                self.on_note_list_action("edit", id);
                true
            }
            "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" if movable => {
                let step = if shift { 10.0 } else { 1.0 };
                let (dx, dy) = match key {
                    "ArrowUp" => (0.0, -step),
                    "ArrowDown" => (0.0, step),
                    "ArrowLeft" => (-step, 0.0),
                    _ => (step, 0.0),
                };
                let r = [item.rect[0], item.rect[1], item.rect[2], item.rect[3]];
                let page = self.wanted_page.max(0) as usize;
                self.ui_move_annot(id, page, r, dx, dy);
                true
            }
            _ => false,
        }
    }

    /// "Drag on the page…" in the certificate dialog: the dialog steps aside (it stays open
    /// in the core, so nothing typed is lost) until a box is dragged or Esc is pressed.
    fn ui_cert_box_start(&mut self) {
        use crate::session::dom_ids as ids;
        if !self.dom.is_open(ids::CERT_SIGN_DIALOG) {
            return;
        }
        if self.cert_sign_field.is_some() {
            return;
        }
        if self.dom.disabled(ids::CERT_SIGN_VISIBLE) {
            // A certified document takes only an invisible signature.
            self.announce("This document takes only an invisible signature.");
            return;
        }
        self.ui.cert_box = true;
        self.ui.drag = None;
    }

    /// The drag that places the certificate signature: its page and box (page points, y down)
    /// go into the dialog's position fields, then the dialog comes back.
    fn ui_cert_box_pointer(&mut self, phase: &str, page: usize, x: f64, y: f64) {
        use crate::session::dom_ids as ids;
        match phase {
            "down" => {
                if page as i64 != self.wanted_page {
                    self.edit("goto", Some(json!({ "page": page })));
                }
                self.ui.drag = Some((page, [x, y], vec![[x, y]]));
            }
            "move" => {
                if let Some((_, _, points)) = self.ui.drag.as_mut() {
                    points.push([x, y]);
                }
            }
            "up" => {
                let Some((page, start, _)) = self.ui.drag.take() else {
                    return;
                };
                let size = self.doc.as_ref().and_then(|d| d.pages.get(page)).cloned();
                let (pw, ph) = size.map_or((612.0, 792.0), |s| (s.width, s.height));
                let (mut x0, mut y0) = (start[0].min(x), start[1].min(y));
                let (mut w, mut h) = ((start[0] - x).abs(), (start[1] - y).abs());
                // A click (or a tiny box) places the dialog's usual size there.
                if w < 40.0 || h < 20.0 {
                    w = 200.0;
                    h = 60.0;
                    x0 = start[0];
                    y0 = start[1];
                }
                x0 = x0.clamp(0.0, (pw - w).max(0.0));
                y0 = y0.clamp(0.0, (ph - h).max(0.0));
                let n = |v: f64| crate::js::js_num(v.round());
                self.dom
                    .set_value(ids::CERT_SIGN_PAGE, (page + 1).to_string());
                self.dom.set_value(ids::CERT_SIGN_X, n(x0));
                self.dom.set_value(ids::CERT_SIGN_Y, n(y0));
                self.dom.set_value(ids::CERT_SIGN_W, n(w.min(pw)));
                self.dom.set_value(ids::CERT_SIGN_H, n(h.min(ph)));
                self.dom.set_checked(ids::CERT_SIGN_VISIBLE, true);
                self.dom.set_hidden(ids::CERT_SIGN_BOX, false);
                self.ui.cert_box = false;
            }
            _ => self.ui.drag = None,
        }
    }

    /// The link under a point of the shown page, if any.
    fn ui_link_at(
        &self,
        page: usize,
        x: f64,
        y: f64,
    ) -> Option<tandem_protocol::engine::LinkEntry> {
        if self.links_page != page as i64 {
            return None;
        }
        self.links
            .iter()
            .find(|l| {
                l.rect.len() == 4
                    && x >= l.rect[0].min(l.rect[2])
                    && x <= l.rect[0].max(l.rect[2])
                    && y >= l.rect[1].min(l.rect[3])
                    && y <= l.rect[1].max(l.rect[3])
            })
            .cloned()
    }

    fn ui_add_shape(&mut self, variant: &str, a: [f64; 2], b: [f64; 2]) {
        let rect = [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[0].max(b[0]),
            a[1].max(b[1]),
        ];
        if rect[2] - rect[0] < 3.0 && rect[3] - rect[1] < 3.0 {
            return;
        }
        match variant {
            "line" => {
                self.ui_annot_line(a, b);
            }
            "ellipse" => {
                self.add_shape("Circle", &rect, "Ellipse added");
            }
            _ => {
                self.add_shape("Square", &rect, "Rectangle added");
            }
        }
    }

    /// A straight line from where the drag started to where it ended.
    fn ui_annot_line(&mut self, a: [f64; 2], b: [f64; 2]) -> bool {
        if self.refuses_add() {
            return false;
        }
        let page = self.wanted_page;
        let request = json!({"op": "annot-add", "page": page, "type": "Line", "points": [[a[0], a[1]], [b[0], b[1]]], "contents": ""});
        self.annot_add(request, "Line added")
    }

    /// Selects the text between two points on the shown page (line, char offset).
    fn ui_select_text(&mut self, a: [f64; 2], b: [f64; 2]) {
        let Some(lines) = self.text_layer() else {
            return;
        };
        let locate = |p: [f64; 2]| -> Option<(usize, usize)> {
            let mut best: Option<(f64, usize, usize)> = None;
            for (li, line) in lines.iter().enumerate() {
                let n = line.text.chars().count();
                for ci in 0..=n {
                    let bx = crate::session::markup::char_box(
                        line,
                        ci.min(n.saturating_sub(1)),
                        n.max(1),
                    );
                    let cx = if ci == n { bx[0] + bx[2] } else { bx[0] };
                    let cy = bx[1] + bx[3] / 2.0;
                    let d = (cx - p[0]).abs() + (cy - p[1]).abs() * 3.0;
                    if best.is_none_or(|(bd, _, _)| d < bd) {
                        best = Some((d, li, ci));
                    }
                }
            }
            best.map(|(_, l, c)| (l, c))
        };
        let range = locate(a).zip(locate(b));
        self.on_text_select(range);
    }

    fn ui_row(&mut self, list: &str, id: &str, action: &str) {
        let num = id.parse::<i64>().unwrap_or(-1);
        match list {
            "comments" => {
                if action == "select" {
                    // A row goes to its comment on the page and flashes it (review annotate#9);
                    // Edit opens the text.
                    if let Some(note) = self.notes.iter().find(|n| n.id == num).cloned()
                        && note.rect.len() == 4
                    {
                        let page = self.wanted_page.max(0) as usize;
                        self.ui.reveal_seq += 1;
                        self.ui.reveal = Some(Reveal {
                            page,
                            rect: [note.rect[0], note.rect[1], note.rect[2], note.rect[3]],
                            seq: self.ui.reveal_seq,
                        });
                        self.lane2.note_row_current = Some(num);
                    }
                } else {
                    self.on_note_list_action(action, num)
                }
            }
            "outline" => self.on_outline_action(
                if action == "open" || action == "select" {
                    "go"
                } else {
                    action
                },
                num,
            ),
            "links" => {
                self.on_link_list_action(if action == "select" { "go" } else { action }, num)
            }
            "content" => {
                if let Some(i) = id
                    .strip_prefix("content-target-")
                    .and_then(|i| i.parse::<i64>().ok())
                {
                    self.select_content_row(i, true);
                }
            }
            "redactions" => {
                if let Some(b) = self.redact_ui.boxes.iter().find(|b| b.id == num).cloned() {
                    if action == "remove" {
                        self.ui.memory_box = None;
                        self.on_redact_panel_remove(b.page, b.id);
                    } else {
                        self.ui.memory_box = (b.id < 0).then_some(b.id);
                        self.on_redact_panel_box(b.page, b.id);
                        // Keys then act on it (arrows move it, Delete removes it).
                        let key = crate::session::hit::NodeKey::Mark(b.id);
                        if b.id > 0 && self.lane2.hit.find(&key).is_some() {
                            self.focus_node(&key);
                        }
                    }
                }
            }
            "layers" => {
                if let Some(layer) = self.layers.iter().find(|l| l.id == num).cloned() {
                    self.on_change(&format!("layer-{}", layer.id), "", !layer.visible);
                }
            }
            "fields" => {
                if let Some(field) = self.fields.iter().find(|f| f.id == num).cloned() {
                    let r = &field.rect;
                    if r.len() == 4 {
                        let page = self.wanted_page.max(0) as usize;
                        self.pointer(
                            "down",
                            page,
                            (r[0] + r[2]) / 2.0,
                            (r[1] + r[3]) / 2.0,
                            false,
                            false,
                            false,
                        );
                        self.pointer(
                            "up",
                            page,
                            (r[0] + r[2]) / 2.0,
                            (r[1] + r[3]) / 2.0,
                            false,
                            false,
                            false,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    /// The page grid's actions, each a loop over the ported page ops.
    fn ui_pages(&mut self, action: &str, mut pages: Vec<usize>, to: Option<usize>) {
        pages.sort_unstable();
        pages.dedup();
        let count = self
            .doc
            .as_ref()
            .map_or(0, |d| d.page_count.max(0) as usize);
        pages.retain(|p| *p < count);
        if matches!(action, "rotate-right" | "rotate-left" | "delete" | "move") {
            // Undo brings these pages back as the selection.
            self.ui.step_pages = Some(pages.clone());
        }
        match action {
            "select" => {
                self.ui.grid_selection = pages;
                return;
            }
            // One engine step per page (the engine has no multi-page op or step group); the
            // window's undo stack makes the whole action one Undo (`ui_undo`).
            "rotate-right" | "rotate-left" => {
                let by = if action == "rotate-right" { 90 } else { -90 };
                for p in &pages {
                    self.rotate_page(*p, by);
                }
            }
            "delete" => {
                for p in pages.iter().rev() {
                    self.delete_page(*p);
                }
            }
            "extract" => {
                let range = ranges(&pages);
                self.edit("extract", None);
                self.on_input("extract-range", &range);
            }
            "move" => {
                let Some(to) = to else { return };
                let plan = move_plan(count, &pages, to);
                for (from, dest) in &plan.steps {
                    self.move_page(*from, *dest as i64);
                }
                self.ui.grid_selection = plan.landed;
            }
            _ => {}
        }
        if matches!(action, "rotate-right" | "rotate-left") {
            if pages.len() > 1 {
                self.announce(&format!("Rotated {} pages", pages.len()));
            }
            self.ui.grid_selection = pages;
        } else if action == "delete" {
            let n = self
                .doc
                .as_ref()
                .map_or(0, |d| d.page_count.max(0) as usize);
            let first = pages.first().copied().unwrap_or(0).min(n.saturating_sub(1));
            if pages.len() > 1 {
                self.announce(&format!(
                    "Deleted {} pages — Undo brings them back",
                    pages.len()
                ));
            }
            self.ui.grid_selection = vec![first];
        } else if action == "move" && pages.len() > 1 {
            self.announce(&format!("Moved {} pages", pages.len()));
        }
    }

    /// The window's whole state.
    pub fn ui_state(&mut self) -> UiState {
        let state = self.state_summary();
        let view = self.ui_view();
        UiState { state, view }
    }

    fn ui_view(&mut self) -> View {
        let mut v = View {
            generation: self.ui.generation,
            ..View::default()
        };
        v.recent = self
            .host
            .recent_list()
            .into_iter()
            .map(|r| {
                // The real path (a tooltip people can read, and what Open takes), never the
                // list's internal id; `when` from the file itself.
                let path = self.host.recent.path(&r.id);
                RecentFile {
                    name: r.name.clone(),
                    when: path.as_deref().map(recent_when).unwrap_or_default(),
                    path: path
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|| r.id.clone()),
                    folder: r.folder.clone(),
                }
            })
            .collect();
        v.dialog = self.ui_dialog();
        v.announce = self.ui.live.clone();
        v.announce_seq = self.ui.live_seq;
        let notice = (!self.dom.hidden(crate::session::dom_ids::ERROR_EL))
            .then(|| self.dom.text(crate::session::dom_ids::ERROR_EL));
        if let Some(text) = notice.filter(|t| !t.is_empty()) {
            let kind = self.notice_kind.as_str().to_string();
            v.notice = Some(Notice {
                kind,
                text,
                actions: vec![],
            });
        }
        if v.notice.is_none()
            && let Some(r) = self.ui.recovered.first()
        {
            v.notice = Some(Notice {
                kind: "warning".into(),
                text: format!(
                    "Tandem PDF closed before you saved {}. Restore your unsaved changes? They open as a new copy; the original file is not changed.",
                    r.name
                ),
                actions: vec![
                    (format!("restore:{}", r.id), "Restore".into()),
                    (format!("forget:{}", r.id), "Discard".into()),
                ],
            });
        }
        let Some(doc) = self.doc.clone() else {
            v.screen = "start".into();
            v.tool.id = "select".into();
            return v;
        };
        v.screen = if doc.info.needs_password {
            "password"
        } else {
            "editor"
        }
        .into();
        let read_only = crate::session::opening::is_read_only(&doc.info);
        let untitled = doc.untitled == Some(true);
        let save_state = if read_only {
            "Read-only"
        } else if self.dirty {
            "Edited"
        } else if untitled {
            "Not saved yet"
        } else {
            "Saved"
        };
        let reason = if !read_only {
            None
        } else if doc.info.signatures > 0 {
            Some("This file is signed. Editing it would break the signature, so it opens for reading.".to_string())
        } else {
            Some("This file does not allow changes.".to_string())
        };
        let pages = doc
            .pages
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let rotation = doc.rotations.get(i).copied().unwrap_or(0) as i32;
                // The engine's page sizes are already as shown (its page frame applies
                // /Rotate): swapping again drew rotated pages squashed (review pages#3).
                let (w, h) = (p.width, p.height);
                let label = self
                    .labels
                    .get(i)
                    .filter(|l| !l.is_empty())
                    .cloned()
                    .unwrap_or_else(|| (i + 1).to_string());
                PageView {
                    width: w,
                    height: h,
                    rotation,
                    label,
                }
            })
            .collect::<Vec<_>>();
        v.doc = Some(DocView {
            name: doc.name.clone(),
            save_state: save_state.into(),
            read_only,
            read_only_reason: reason,
            page_count: pages.len(),
            pages,
            can_undo: self.can_undo,
            can_redo: self.can_redo,
            undo_label: step_label("Undo", self.can_undo, &self.ui.undo_steps),
            redo_label: step_label("Redo", self.can_redo, &self.ui.redo_steps),
            // An Untitled document is saved with Save… (the panel asks where), like the
            // menu's Save (open#8).
            can_save: (untitled && !read_only)
                || !self.dom.disabled(crate::session::dom_ids::SAVE_BUTTON),
        });
        let page = self.wanted_page.max(0);
        v.page = page as usize;
        v.zoom = ZoomView {
            mode: self.zoom_mode.as_str().into(),
            scale: self.ui_scale(),
        };
        v.tool = ToolView {
            id: self.ui.tool.clone(),
            chip: self.ui_chip(),
            variant: self.ui.variant.clone(),
            color: Some(self.ui.color),
            width: Some(self.ui.width),
            available: self.ui_availability(read_only),
        };
        v.sidebar = PaneView {
            open: self.sidebar_open,
            tab: self.ui.sidebar_tab.clone(),
        };
        v.panel = PaneView {
            open: self.ui.panel_open || self.redact_ui.active,
            tab: if self.redact_ui.active {
                "redactions".into()
            } else {
                self.ui.panel_tab.clone()
            },
        };
        if let Some(notice) = &v.notice
            && notice.kind == "success"
        {
            v.toast = Some(notice.text.clone());
            v.notice = None;
        }
        if v.toast.is_none()
            && let Some(text) = &self.ui.toast
        {
            if crate::engine::now_ms() - self.ui.toast_at < 5000.0 {
                v.toast = Some(text.clone());
            } else {
                self.ui.toast = None;
            }
        }
        let banner_text = self
            .dom
            .text(crate::session::dom_ids::redact_ui::BANNER_TEXT_EL);
        if let Some(banner) = self
            .redact_ui
            .banner_state
            .as_ref()
            .map(|_| banner_text)
            .filter(|t| !t.is_empty())
        {
            v.banner = Some(Notice {
                kind: "info".into(),
                text: banner,
                actions: {
                    let mut a = Vec::new();
                    if !self
                        .dom
                        .hidden(crate::session::dom_ids::redact_ui::BANNER_ORIGINAL)
                    {
                        a.push(("redact-banner-original".into(), "Open original".into()));
                    }
                    a.push(("redact-banner-dismiss".into(), "Dismiss".into()));
                    a
                },
            });
        } else if !self.dom.hidden(crate::session::dom_ids::BANNER_EL)
            && !self
                .dom
                .text(crate::session::dom_ids::BANNER_TEXT_EL)
                .is_empty()
        {
            let text = self.dom.text(crate::session::dom_ids::BANNER_TEXT_EL);
            let signed = doc.info.signatures > 0 || doc.info.certified;
            let mut actions = Vec::new();
            if signed {
                // Signed files (review open#12): an editable copy without the signatures, and
                // Verify. The copy comes first: it is what someone who wants to edit is after.
                if self.signed_read_only() {
                    actions.push((
                        "editable-copy".to_string(),
                        crate::messages::editable_copy::BUTTON.to_string(),
                    ));
                }
                actions.push(("verify-open".to_string(), "Verify signatures…".to_string()));
            }
            actions.push(("banner-dismiss".into(), "Dismiss".into()));
            // A repaired file is a warning, not information (review open#13).
            let kind = if doc.info.repaired || signed {
                "warning"
            } else {
                "info"
            };
            v.banner = Some(Notice {
                kind: kind.into(),
                text,
                actions,
            });
        } else if doc.info.repaired {
            v.banner = Some(Notice {
                kind: "warning".into(),
                text: "This file was damaged and has been repaired. Check it before saving.".into(),
                actions: vec![],
            });
        }
        v.search = SearchView {
            query: self.search_query.clone(),
            count: self.search_hits.len(),
            index: self.search_index,
            truncated: self.search_truncated,
            busy: false,
            note: self.search_truncated.then(|| {
                format!(
                    "Only the first {} matches are shown. Type more words to narrow the search.",
                    self.search_hits.len()
                )
            }),
        };
        v.overlays = vec![self.ui_overlay(page)];
        if doc.info.needs_password {
            v.password = Some(PasswordView {
                name: doc.name.clone(),
                error: self.dom.text(crate::session::dom_ids::PASSWORD_ERROR_EL),
            });
        }
        if let Some(c) = &self.compare_doc {
            let paint_key = self.lane4.compare_paint.as_ref().map_or(0, |p| {
                p.a.2.len() as u64 ^ (p.b.2.len() as u64).rotate_left(17) ^ c.page_b as u64
            });
            v.compare = Some(CompareView {
                name_a: doc.name.clone(),
                name_b: c.name.clone(),
                page_a: c.page_a.max(0) as usize,
                page_b: c.page_b.max(0) as usize,
                pages_b: c.page_count.max(0) as usize,
                show_diff: self.compare_show_diff,
                // Plain words (review pages#11); the drive's summary text stays as it is.
                summary: {
                    let found = compare_summary(c, &doc.name)
                        .unwrap_or_else(|| self.dom.text("compare-summary"));
                    match &self.ui.compare_note {
                        Some(note) => format!("{note} {found}"),
                        None => found,
                    }
                },
                rows: self
                    .lane4
                    .compare_rows
                    .iter()
                    .enumerate()
                    .map(|(i, r)| Row {
                        id: i.to_string(),
                        page: None,
                        title: r.text.clone(),
                        // "Only in a.pdf", not "Only left".
                        detail: match r.side.as_str() {
                            "Only left" => format!("Only in {}", doc.name),
                            "Only right" => format!("Only in {}", c.name),
                            other => other.to_string(),
                        },
                        selected: false,
                    })
                    .collect(),
                generation: self.ui.generation ^ paint_key,
            });
        }
        v.selected_text = self
            .selection_markup()
            .map(|m| m.text.clone())
            .filter(|t| !t.is_empty());
        let panel_id = match v.panel.tab.as_str() {
            "fields" => "panel-fields",
            "links" => "panel-links",
            "properties" => "text-panel",
            "layers" => "panel-layers",
            _ => "panel-notes",
        };
        if let Some(ids) = crate::dialog_ids::group(panel_id) {
            for (node, _) in ids.iter() {
                v.panel_nodes
                    .insert(node.to_string(), node_view(&self.dom, node));
            }
        }
        v.lists.insert(
            "links".into(),
            self.links
                .iter()
                .map(|l| Row {
                    id: l.id.to_string(),
                    page: Some(page as usize),
                    title: crate::session::bookmarks_links::link_label(l),
                    detail: String::new(),
                    selected: false,
                })
                .collect(),
        );
        v.lists.insert(
            "content".into(),
            self.content_rows()
                .into_iter()
                .map(|r| Row {
                    id: r.id,
                    page: None,
                    title: r.text,
                    detail: r.meta,
                    selected: r.selected,
                })
                .collect(),
        );
        v.lists.insert(
            "matches".into(),
            self.replace_rows()
                .into_iter()
                .enumerate()
                .map(|(i, r)| Row {
                    selected: r.checkbox.as_deref().is_some_and(|id| self.dom.checked(id)),
                    id: r.checkbox.unwrap_or_else(|| format!("rejected-{i}")),
                    page: None,
                    title: r.text,
                    detail: String::new(),
                })
                .collect(),
        );
        let chosen = self.dom.value("field-select");
        v.lists.insert(
            "field-select".into(),
            self.field_select_options()
                .into_iter()
                .map(|o| Row {
                    selected: o.value == chosen,
                    id: o.value,
                    page: None,
                    title: o.label,
                    detail: String::new(),
                })
                .collect(),
        );
        // "Strikethrough on page 2" and the words it marks (review annotate#8).
        let current_note = self.lane2.note_row_current;
        v.comments = self
            .notes
            .clone()
            .iter()
            .filter(|n| n.r#type != "Redact")
            .map(|n| {
                let name = crate::session::pages::annot_name(n).unwrap_or("Comment");
                Row {
                    id: n.id.to_string(),
                    page: Some(page as usize),
                    title: format!("{name} on page {}", page + 1),
                    detail: self.annot_summary(n),
                    selected: current_note == Some(n.id),
                }
            })
            .collect();
        let current_field = self.lane2.field_row_current.clone();
        v.fields = self
            .fields
            .iter()
            .filter(|f| !f.name.is_empty())
            .map(|f| Row {
                id: f.id.to_string(),
                page: Some(page as usize),
                title: field_title(f),
                detail: field_detail(f),
                selected: current_field.as_deref() == Some(f.id.to_string().as_str()),
            })
            .collect();
        // The current entry: the last one (in reading order) that goes to the shown page or
        // a page before it, preferring the deepest on the same page.
        let current = self
            .outline
            .iter()
            .enumerate()
            .filter(|(_, o)| o.page.is_some_and(|p| p <= page))
            .max_by_key(|(i, o)| (o.page.unwrap_or(0), *i))
            .map(|(i, _)| i);
        v.outline = self
            .outline
            .iter()
            .enumerate()
            .map(|(i, o)| OutlineRow {
                id: o.id.to_string(),
                title: o.title.clone(),
                page: o.page.map(|p| p.max(0) as usize),
                uri: o.uri.clone(),
                depth: o.depth.max(0) as usize,
                current: current == Some(i),
                has_children: self
                    .outline
                    .get(i + 1)
                    .is_some_and(|next| next.depth > o.depth),
            })
            .collect();
        v.layers = self
            .layers
            .iter()
            .map(|l| Row {
                id: l.id.to_string(),
                page: None,
                title: l.name.clone(),
                detail: if l.visible {
                    "Shown".into()
                } else {
                    "Hidden".into()
                },
                selected: l.visible,
            })
            .collect();
        let boxes = self.redact_ui.boxes.clone();
        let selected_box = self.ui_selected_box();
        let mut per_page: BTreeMap<i64, usize> = BTreeMap::new();
        let mut rows = Vec::new();
        for b in &boxes {
            let n = per_page.entry(b.page).or_default();
            *n += 1;
            // "Page 1 · box 2" and what it covers (review redact#9): whole words, read on the
            // box's own page and kept while another page is shown.
            if b.page == self.shown_page && b.rect.len() == 4 {
                let words = self.ui_box_words(&b.rect);
                if !words.is_empty() {
                    self.ui.box_words.insert(b.id, words);
                }
            }
            let words = self.ui.box_words.get(&b.id).cloned().unwrap_or_default();
            let detail = if !words.is_empty() {
                let short: String = words.chars().take(60).collect();
                let short = if words.chars().count() > 60 {
                    // Cut at a word, not inside one.
                    let cut = short.rfind(' ').filter(|i| *i > 20).unwrap_or(short.len());
                    format!("{}…", &short[..cut])
                } else {
                    short
                };
                if b.search {
                    format!("Found ‘{short}’")
                } else {
                    format!("Covers ‘{short}’")
                }
            } else if b.search {
                "Found text".into()
            } else if b.rect.len() == 4 {
                format!(
                    "{} × {} pt, {} pt from the top",
                    (b.rect[2] - b.rect[0]).abs().round(),
                    (b.rect[3] - b.rect[1]).abs().round(),
                    b.rect[1].min(b.rect[3]).round()
                )
            } else {
                String::new()
            };
            rows.push(Row {
                id: b.id.to_string(),
                page: Some(b.page.max(0) as usize),
                title: format!("Page {} · box {n}", b.page + 1),
                detail,
                selected: selected_box == Some(b.id),
            });
        }
        v.redact = RedactView {
            boxes: rows,
            apply_label: {
                let t = self
                    .dom
                    .text(crate::session::dom_ids::redact_ui::APPLY_OPEN);
                if t.is_empty() {
                    "Redact areas…".into()
                } else {
                    t
                }
            },
            can_apply: !boxes.is_empty(),
            underneath: self.redact_ui.underneath,
            flatten_note: Some(
                self.dom
                    .text(crate::session::dom_ids::redact_ui::FLATTEN_NOTE_EL),
            )
            .filter(|t| !t.is_empty()),
        };
        if self.compare_doc.is_none() {
            self.ui.compare_note = None;
        }
        v.menu = self.ui_context_menu();
        v.reveal = self.ui.reveal.clone();
        v.page_selection = self.ui.grid_selection.clone();
        v
    }

    /// The words a redaction box covers on the shown page, each word whole (a box that
    /// clips "SECRET" still reads "P0B-SECRET-7731"), lines joined by a space.
    fn ui_box_words(&mut self, rect: &[f64]) -> String {
        let Some(lines) = self.text_layer().map(<[_]>::to_vec) else {
            // Asks for the page's text, as the panel's own reading does.
            return self.marked_text(rect);
        };
        let [x0, y0, x1, y1] = match rect {
            [a, b, c, d] => [a.min(*c), b.min(*d), a.max(*c), b.max(*d)],
            _ => return String::new(),
        };
        let mut out: Vec<String> = Vec::new();
        for line in &lines {
            let chars: Vec<char> = line.text.chars().collect();
            let n = chars.len();
            let inside: Vec<usize> = (0..n)
                .filter(|i| {
                    let [x, y, w, h] = crate::session::markup::char_box(line, *i, n);
                    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
                    cx >= x0 && cx <= x1 && cy >= y0 && cy <= y1
                })
                .collect();
            let (Some(&first), Some(&last)) = (inside.first(), inside.last()) else {
                continue;
            };
            let mut a = first;
            while a > 0 && !chars[a - 1].is_whitespace() {
                a -= 1;
            }
            let mut b = last;
            while b + 1 < n && !chars[b + 1].is_whitespace() {
                b += 1;
            }
            let words: String = chars[a..=b].iter().collect();
            let words = words.trim();
            if !words.is_empty() {
                out.push(words.to_string());
            }
        }
        out.join(" ")
    }

    /// The page's context menu as the window draws it: the object menu's visible items.
    fn ui_context_menu(&mut self) -> Option<ContextMenuView> {
        let (page, x, y) = self.ui.menu_at?;
        if self.dom.hidden(crate::session::dom_ids::OBJECT_MENU) {
            self.ui.menu_at = None;
            return None;
        }
        // With nothing under the click, the object items that cannot apply are left out
        // rather than listed greyed (redact: eleven greyed items above "Redact whole page").
        let nothing_under = self.ui_hit_node_at(page, x, y).is_none();
        let items: Vec<(String, String, bool)> = crate::session::dom_ids::OBJECT_MENU_BUTTONS
            .iter()
            .filter(|(_, id)| !self.dom.hidden(id))
            .map(|(key, id)| (key.to_string(), self.dom.text(id), !self.dom.disabled(id)))
            .filter(|(_, label, on)| !label.is_empty() && (*on || !nothing_under))
            .collect();
        if !items.iter().any(|(_, _, on)| *on) {
            return None;
        }
        Some(ContextMenuView { page, x, y, items })
    }

    /// The redaction box selected on the page (its mark is the focused or selected target,
    /// or a signed file's box in memory is focused).
    fn ui_selected_box(&self) -> Option<i64> {
        use crate::session::hit::{Focus, NodeKey, NodeKind};
        if let Some(id) = self
            .ui
            .memory_box
            .filter(|id| self.redact_ui.boxes.iter().any(|b| b.id == *id))
        {
            return Some(id);
        }
        if let Some(id) = self.direct_focused_box {
            return Some(id);
        }
        self.lane2.hit.nodes.iter().find_map(|n| match &n.kind {
            NodeKind::Mark { item, .. }
                if n.selected
                    || n.active
                    || self.lane2.focus == Focus::Node(NodeKey::Mark(item.id)) =>
            {
                Some(item.id)
            }
            _ => None,
        })
    }

    fn ui_chip(&self) -> Option<String> {
        if self.redact_ui.active {
            return Some(crate::messages::redact::REDACT_CHIP.to_string());
        }
        if self.sign_armed {
            return Some("Click where your signature goes · Esc to cancel".into());
        }
        if self.ui.cert_box {
            return Some("Drag a box where the signature goes · Esc to go back".into());
        }
        let label = match self.ui.tool.as_str() {
            "highlight" => "Drag across text to mark it",
            "draw" => match self.ui.variant.as_deref() {
                Some("line") => "Drag to draw a line",
                Some("rect") => "Drag to draw a rectangle",
                Some("ellipse") => "Drag to draw an ellipse",
                _ => "Drawing",
            },
            "note" => "Click where the note goes",
            "text" => "Click text to change it, or empty space to add text",
            "fill-sign" => "Click a field, or anywhere to type",
            "textbox" => "Click where the text box goes",
            "pages" => "Arranging pages",
            _ => return self.armed_tool().map(|t| format!("{t} · Esc to stop")),
        };
        Some(format!("{label} · Esc to stop"))
    }

    fn ui_availability(&self, read_only: bool) -> Vec<ToolAvailability> {
        let annotate = !read_only && self.can_annotate();
        let edit = !read_only && self.can_edit();
        let signed = self
            .doc
            .as_ref()
            .is_some_and(|d| d.info.signatures > 0 || d.info.certified);
        // Why, in the words of the banner: a signed file opens for reading (its redaction
        // still works: it makes an unsigned copy).
        let (no_comments, no_changes, no_pages) = if signed {
            (
                "This file is signed, so comments would break the signature.",
                "This file is signed, so changing it would break the signature.",
                "This file is signed, so rearranging pages would break the signature.",
            )
        } else {
            (
                "This file does not allow comments.",
                "This file does not allow changes.",
                "This file does not allow changing its pages.",
            )
        };
        let (no_fill, no_sign) = if signed {
            (
                "This file is signed, so filling it in would break the signature.",
                "This file is signed and opens for reading, so it can't be signed again here.",
            )
        } else {
            (
                "This file does not allow filling in.",
                "This file does not allow changes, so it can't be signed here.",
            )
        };
        let pages = !read_only && self.editable() && self.permitted("edit");
        let mut out = Vec::new();
        for (id, ok, why) in [
            ("highlight", annotate, no_comments),
            ("draw", annotate, no_comments),
            ("note", annotate, no_comments),
            ("text", edit, no_changes),
            ("redact", self.doc.is_some(), ""),
            ("pages", pages, no_pages),
            // A signed file opens for reading: filling in or placing a drawn signature would
            // break its signature; the toolbar's Sign… follows "sign" (review sign: signed
            // read-only copy).
            ("fill-sign", !read_only, no_fill),
            ("textbox", annotate, no_comments),
            ("sign", !read_only, no_sign),
        ] {
            out.push(ToolAvailability {
                id: id.into(),
                enabled: ok,
                reason: (!ok).then(|| why.to_string()),
                badge: None,
            });
        }
        if let Some(r) = out.iter_mut().find(|a| a.id == "redact") {
            // Pending boxes stay counted after the tool is left (review redact#8).
            let marks = if self.dirty {
                self.redact_state
                    .as_ref()
                    .map_or(0, |s| s.marks.max(0) as usize)
            } else {
                0
            };
            let n = self
                .redact_ui
                .boxes
                .len()
                .max(self.redact_memory.len() + marks);
            r.badge = (n > 0).then_some(n);
        }
        out
    }

    fn ui_overlay(&mut self, page: i64) -> PageOverlay {
        let mut o = PageOverlay {
            page: page.max(0) as usize,
            ..PageOverlay::default()
        };
        let hits: Vec<_> = self
            .search_hits
            .iter()
            .enumerate()
            .filter(|(_, h)| h.page == page)
            .collect();
        for (k, (i, h)) in hits.iter().enumerate() {
            if *i as i64 == self.search_index {
                o.current_hit = Some(k);
            }
            if let Some(r) = crate::session::bookmarks_links::hit_rect(h) {
                o.hits.push([r[0], r[1], r[2], r[3]]);
            }
        }
        if let Some(sel) = self.selection_markup() {
            for q in &sel.quads {
                if q.len() >= 8 {
                    let xs = [q[0], q[2], q[4], q[6]];
                    let ys = [q[1], q[3], q[5], q[7]];
                    o.selection.push([
                        xs.iter().cloned().fold(f64::MAX, f64::min),
                        ys.iter().cloned().fold(f64::MAX, f64::min),
                        xs.iter().cloned().fold(f64::MIN, f64::max),
                        ys.iter().cloned().fold(f64::MIN, f64::max),
                    ]);
                }
            }
        }
        // Pending boxes are black whether or not the tool is on (review redact#8): with the
        // tool off they come from the page's marks and a signed file's boxes in memory.
        let active = self.redact_ui.active;
        let mut boxes: Vec<(i64, Vec<f64>)> = if active {
            self.redact_ui
                .boxes
                .iter()
                .filter(|b| b.page == page)
                .map(|b| (b.id, b.rect.clone()))
                .collect()
        } else {
            let mut list: Vec<(i64, Vec<f64>)> = if self.shown_page == page {
                self.notes
                    .iter()
                    .filter(|n| n.r#type == "Redact")
                    .map(|n| (n.id, n.rect.clone()))
                    .collect()
            } else {
                Vec::new()
            };
            list.extend(
                self.redact_memory
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.page == page)
                    .map(|(i, r)| (-(i as i64 + 1), r.rect.clone())),
            );
            list
        };
        boxes.retain(|(_, r)| r.len() == 4);
        let selected_box = self.ui_selected_box();
        let peek = active && (self.redact_ui.underneath || self.redact_ui.peek);
        for (id, r) in boxes {
            o.redactions.push(Target {
                kind: "redact".into(),
                id: id.to_string(),
                rect: [r[0], r[1], r[2], r[3]],
                label: if peek { "peek".into() } else { String::new() },
                selected: active && selected_box == Some(id),
                movable: active,
                ..Target::default()
            });
        }
        // The box being dragged out (review redact#6), or a shape being drawn.
        if let Some(drag) = self.lane4.area_drag.as_ref().filter(|d| d.page == page)
            && let Some(d) = drag.draft.as_ref().filter(|d| d.len() == 4)
        {
            o.draft = Some([d[0], d[1], d[2], d[3]]);
        } else if let Some((p, start, points)) = &self.ui.drag
            && *p as i64 == page
            && (self.ui.tool == "draw" || self.ui.cert_box)
            && let Some(end) = points.last()
        {
            o.draft = Some([
                start[0].min(end[0]),
                start[1].min(end[1]),
                start[0].max(end[0]),
                start[1].max(end[1]),
            ]);
        }
        if let Some(tint) = self
            .redact_ui
            .tint
            .as_ref()
            .filter(|t| active && t.page == page)
        {
            for shape in &tint.shapes {
                let p = &shape.points;
                if p.len() >= 4 {
                    let xs: Vec<f64> = p.iter().step_by(2).cloned().collect();
                    let ys: Vec<f64> = p.iter().skip(1).step_by(2).cloned().collect();
                    o.tint.push([
                        xs.iter().cloned().fold(f64::MAX, f64::min),
                        ys.iter().cloned().fold(f64::MAX, f64::min),
                        xs.iter().cloned().fold(f64::MIN, f64::max),
                        ys.iter().cloned().fold(f64::MIN, f64::max),
                    ]);
                }
            }
        }
        // The hit layer the core paints and hit-tests (lane 2): exactly what a click reaches.
        if self.shown_page == page {
            use crate::session::hit::NodeKind;
            for node in self
                .lane2
                .hit
                .nodes
                .iter()
                .filter(|n| !n.hidden && n.rect.len() == 4)
            {
                let kind = match &node.kind {
                    NodeKind::Link { .. } => "link",
                    NodeKind::TextBox { .. } => "textbox",
                    NodeKind::Signature { .. } => "signature",
                    NodeKind::Field { .. } => "field",
                    NodeKind::SigField { .. } => "field",
                    NodeKind::ParaOutline { .. } => "paragraph",
                    NodeKind::Object { .. } => "object",
                    NodeKind::Text { .. } => "text",
                    NodeKind::Mark { .. } => "mark",
                    NodeKind::GroupBox => "group",
                    NodeKind::CropFull => "crop",
                };
                // Redaction boxes are drawn as black boxes, not as targets.
                if kind == "mark" {
                    continue;
                }
                let r = &node.rect;
                let focused = self.lane2.focus == crate::session::hit::Focus::Node(node.key());
                let mut label = if node.label.is_empty() {
                    node.title.clone()
                } else {
                    node.label.clone()
                };
                // Fields are filled in Fill & Sign, not moved: no handles there (the field
                // designer's handles are for Select). A read-only field says so.
                let field = matches!(
                    node.kind,
                    NodeKind::Field { .. } | NodeKind::SigField { .. }
                );
                let still = field && self.ui.tool == "fill-sign";
                if let NodeKind::Field { item } = &node.kind
                    && item.read_only
                {
                    label = format!("{label}, read-only (its value can't be changed)");
                }
                o.targets.push(Target {
                    kind: kind.into(),
                    id: format!("{}", node.seq),
                    rect: [r[0], r[1], r[2], r[3]],
                    label,
                    // The focused target is the selected one (keys act on it): handles show.
                    selected: node.selected || node.active || focused,
                    movable: !node.handles.is_empty() && !still,
                    handles: if still {
                        Vec::new()
                    } else {
                        node.handles.iter().map(|h| h.to_string()).collect()
                    },
                    focused,
                });
            }
            o.targets.extend(self.ui_annot_targets());
        }
        if let Some(active) = &self.direct_active {
            // The editor was placed at the zoom of the session's last paint; `ui_sync_zoom`
            // keeps that equal to the zoom the window draws at.
            let scale = if self.shown_zoom > 0.0 {
                self.shown_zoom
            } else {
                self.zoom_now(page).max(0.01)
            };
            let e = &active.editor;
            o.editor = Some(EditorView {
                kind: format!("{:?}", active.kind).to_lowercase(),
                rect: [
                    e.left / scale,
                    e.top / scale,
                    (e.left + e.width) / scale,
                    (e.top + e.height) / scale,
                ],
                value: e.value.clone(),
                font_px: e.font_px / scale,
                multiline: e.tag != crate::session::types::EditorTag::Input,
                placeholder: e.placeholder.clone(),
                options: e.options.clone(),
                label: self.lane2.editor_label.clone(),
            });
        }
        o
    }

    /// The annotations the Select tool can pick on the shown page (annotate#10): comments,
    /// highlights, drawings, stamps, text boxes and placed signatures, each as a target. A
    /// text box or signature moves by dragging or the arrow keys; Delete removes any of them.
    fn ui_annot_targets(&self) -> Vec<Target> {
        if self.ui.tool != "select" || self.redact_ui.active {
            return Vec::new();
        }
        self.ui_annots()
            .into_iter()
            .map(|(item, movable)| {
                let name = crate::session::pages::annot_name(&item).unwrap_or("Comment");
                let words = item.contents.trim();
                let label = if words.is_empty() {
                    name.to_string()
                } else {
                    let short: String = words.chars().take(60).collect();
                    format!("{name}: {short}")
                };
                let selected = self.ui.annot_selected == Some(item.id);
                let rect = match self.ui.annot_drag {
                    Some((id, start, r, now)) if id == item.id => {
                        let (dx, dy) = (now[0] - start[0], now[1] - start[1]);
                        [r[0] + dx, r[1] + dy, r[2] + dx, r[3] + dy]
                    }
                    _ => [item.rect[0], item.rect[1], item.rect[2], item.rect[3]],
                };
                Target {
                    kind: "annot".into(),
                    id: format!("annot-{}", item.id),
                    rect,
                    label,
                    selected,
                    movable,
                    handles: Vec::new(),
                    focused: selected,
                }
            })
            .collect()
    }

    /// The shown page's annotations Select can pick, and whether each can be moved (the
    /// engine moves text boxes and placed signatures only).
    fn ui_annots(&self) -> Vec<(tandem_protocol::engine::AnnotItem, bool)> {
        if self.shown_page != self.wanted_page {
            return Vec::new();
        }
        let can = self.can_annotate();
        self.notes
            .iter()
            .filter(|n| {
                n.rect.len() == 4
                    && !matches!(n.r#type.as_str(), "Redact" | "Link" | "Widget" | "Popup")
                    && n.rect[2] > n.rect[0]
                    && n.rect[3] > n.rect[1]
            })
            .map(|n| {
                let movable = can
                    && n.editable
                    && (n.r#type == "FreeText" || crate::session::direct::is_signature(n));
                (n.clone(), movable)
            })
            .collect()
    }

    /// The topmost annotation Select can pick at a point (the smallest one, so a note on a
    /// highlight is reachable).
    fn ui_annot_at(&self, x: f64, y: f64) -> Option<(tandem_protocol::engine::AnnotItem, bool)> {
        self.ui_annots()
            .into_iter()
            .filter(|(n, _)| {
                let r = &n.rect;
                x >= r[0] - 2.0 && x <= r[2] + 2.0 && y >= r[1] - 2.0 && y <= r[3] + 2.0
            })
            .min_by(|(a, _), (b, _)| {
                let area = |r: &[f64]| (r[2] - r[0]) * (r[3] - r[1]);
                area(&a.rect)
                    .partial_cmp(&area(&b.rect))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// The session's own hit-layer target (a link, a field) at a point, and whether the
    /// point is on one of its handles.
    fn ui_hit_node_at(&self, page: usize, x: f64, y: f64) -> Option<(usize, bool)> {
        use crate::session::hit::{EvTarget, Part};
        let (cx, cy) = self.client_of(page as i64, x, y)?;
        match self.hit_test(cx, cy) {
            EvTarget::Node { index, part } => Some((index, matches!(part, Part::Handle(_)))),
            _ => None,
        }
    }

    fn ui_dialog(&self) -> Option<DialogView> {
        if self.ui.cert_box {
            return None;
        }
        let (id, ids) = crate::dialog_ids::DIALOGS
            .iter()
            .find(|(id, _)| self.dom.is_open(id))?;
        let mut nodes = BTreeMap::new();
        for (node, _) in ids.iter() {
            nodes.insert(node.to_string(), node_view(&self.dom, node));
        }
        let heading = ids
            .iter()
            .find(|(_, kind)| *kind == "h2")
            .map(|(n, _)| self.dom.text(n))
            .filter(|t| !t.is_empty());
        Some(DialogView {
            id: id.to_string(),
            title: heading.unwrap_or_else(|| dialog_title(id).to_string()),
            data: self.ui_dialog_data(id),
            nodes,
            rows: self.ui_dialog_rows(id),
        })
    }

    /// What a dialog needs besides its elements: the verify dialog's trusted certificates
    /// and each row's verdict kind ("valid", "untrusted", "broken", "empty" (not signed yet),
    /// "unknown") for its icon.
    fn ui_dialog_data(&self, id: &str) -> Value {
        match id {
            "verify-dialog" => {
                let kinds: Vec<&str> = self
                    .lane5
                    .verify_rows
                    .iter()
                    .map(|r| {
                        let status = r.cells.get(3).map(String::as_str).unwrap_or("");
                        let trust = r.cells.get(4).map(String::as_str).unwrap_or("");
                        if status.starts_with("valid") {
                            if trust.starts_with("trusted") {
                                "valid"
                            } else {
                                "untrusted"
                            }
                        } else if matches!(status, "invalid" | "modified" | "tampered" | "broken") {
                            "broken"
                        } else if status == "empty" {
                            "empty"
                        } else {
                            "unknown"
                        }
                    })
                    .collect();
                json!({
                    "trusted": self.lane5.verify_trusted,
                    "trustedEmpty": "You haven't trusted any certificates yet.",
                    "rowKinds": kinds,
                })
            }
            // The pad shows the picture chosen with "Use a picture…" and the signature saved
            // earlier (walkthrough sign, shot 22).
            "sigpad-dialog" => json!({
                "image": self.pad_image.as_ref().map(|i| json!({
                    "data": i.data,
                    "width": i.width,
                    "height": i.height,
                })),
                "saved": self.saved_signature,
            }),
            _ => Value::Null,
        }
    }

    fn ui_dialog_rows(&self, id: &str) -> Vec<Vec<String>> {
        match id {
            "verify-dialog" => self
                .lane5
                .verify_rows
                .iter()
                .map(|r| {
                    // Signature, signed by, and the plain verdict (the sentence, else the status cell).
                    let cell = |i: usize| r.cells.get(i).cloned().unwrap_or_default();
                    vec![cell(0), cell(1), r.plain.clone().unwrap_or_else(|| cell(3))]
                })
                .collect(),
            // Each file of the last run and what happened to it (walkthrough open#11).
            "batch-dialog" => self.batch_result_rows(),
            _ => Vec::new(),
        }
    }

    fn ui_editor_input(&mut self, value: &str) {
        self.editor_input(value, None);
    }

    /// Page pixels for the window: `kind` "page" (the document page at `scale` device
    /// pixels per point) or "thumb".
    pub fn ui_pixels(
        &mut self,
        kind: &str,
        page: usize,
        scale: f64,
    ) -> Result<(u32, u32, Vec<u8>), String> {
        // Compare's three canvases are what the compare view painted (review pages#1).
        if let Some(which) = kind.strip_prefix("compare-") {
            let paint = self
                .lane4
                .compare_paint
                .as_ref()
                .ok_or_else(|| "nothing to compare yet".to_string())?;
            let (w, h, px) = match which {
                "a" => (paint.a.0, paint.a.1, paint.a.2.clone()),
                "b" => (paint.b.0, paint.b.1, paint.b.2.clone()),
                // The overlay is on B's grid.
                _ => (paint.b.0, paint.b.1, paint.overlay.clone()),
            };
            if w <= 0 || h <= 0 || px.len() != (w * h * 4) as usize {
                return Err("the compared page could not be drawn".into());
            }
            return Ok((w as u32, h as u32, px));
        }
        // A thumbnail is the same render at its own scale. Past the engine's 4096 px edge
        // the window stretches a smaller render (review view#1): never fail at high zoom.
        let scale = self.ui_pixel_scale(page, scale);
        let mut reply = self
            .request(
                json!({ "op": "render", "page": page, "scale": scale }),
                vec![],
            )
            .map_err(|e| e.message)?;
        let w = reply
            .get("pixelWidth")
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as u32;
        let h = reply
            .get("pixelHeight")
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as u32;
        let px = reply.take_bytes("pixels").unwrap_or_default();
        Ok((w, h, px))
    }

    /// `scale` inside the engine's bounds: the scale range and the 4096 px edge.
    fn ui_pixel_scale(&self, page: usize, scale: f64) -> f64 {
        use tandem_protocol::limits::{MAX_PIXEL_EDGE, MAX_SCALE, MIN_SCALE};
        let scale = if scale.is_finite() { scale } else { 1.0 };
        let mut scale = scale.clamp(MIN_SCALE, MAX_SCALE);
        if let Some(size) = self.doc.as_ref().and_then(|d| d.pages.get(page)) {
            let longest = size.width.max(size.height).max(1.0);
            let edge = MAX_PIXEL_EDGE as f64 / longest;
            // Floor to 1/1000 so rounding never lands a pixel over the edge.
            scale = scale.min((edge * 1000.0).floor() / 1000.0);
        }
        scale.max(MIN_SCALE)
    }

    /// A menu item (`tandem-desktop` menu ids).
    pub fn ui_menu(&mut self, id: &str) {
        match id {
            "open" => self.ui_act(UiAction::Open { path: None }),
            "new" => self.ui_act(UiAction::New { size: None }),
            "print" => self.ui_act(UiAction::Print),
            id if id.starts_with("save:") => self.ui_act(UiAction::Save {
                how: id[5..].into(),
            }),
            id if id.starts_with("edit:") => self.ui_act(UiAction::Edit {
                op: id[5..].into(),
                contents: None,
            }),
            id if id.starts_with("tool:") => self.ui_act(UiAction::Tool {
                id: id[5..].into(),
                variant: None,
            }),
            "view:zoom-in" => self.ui_act(UiAction::Zoom {
                to: "in".into(),
                scale: None,
            }),
            "view:zoom-out" => self.ui_act(UiAction::Zoom {
                to: "out".into(),
                scale: None,
            }),
            "view:actual-size" => self.ui_act(UiAction::Zoom {
                to: "actual".into(),
                scale: None,
            }),
            "view:fit-width" => self.ui_act(UiAction::Zoom {
                to: "fit-width".into(),
                scale: None,
            }),
            "view:fit-page" => self.ui_act(UiAction::Zoom {
                to: "fit-page".into(),
                scale: None,
            }),
            "view:sidebar" => self.ui_act(UiAction::Pane {
                which: "sidebar".into(),
                open: None,
                tab: None,
            }),
            "view:panel" => self.ui_act(UiAction::Pane {
                which: "panel".into(),
                open: None,
                tab: None,
            }),
            "view:find-next" => self.ui_act(UiAction::SearchMove { by: 1 }),
            "view:find-previous" => self.ui_act(UiAction::SearchMove { by: -1 }),
            "view:next-page" => {
                let p = (self.wanted_page + 1).max(0) as usize;
                self.ui_act(UiAction::Goto { page: p });
            }
            "view:prev-page" => {
                let p = (self.wanted_page - 1).max(0) as usize;
                self.ui_act(UiAction::Goto { page: p });
            }
            _ => {}
        }
    }
}

/// A field's row title: its description or name; a choice button adds its own value
/// ("size · S"), so the three buttons of one group read apart.
fn field_title(f: &tandem_protocol::engine::FieldItem) -> String {
    use tandem_protocol::engine::FieldKind;
    let name = if f.label.is_empty() {
        f.name.clone()
    } else {
        f.label.clone()
    };
    match (&f.kind, f.on.as_deref()) {
        (FieldKind::Radio, Some(on)) if !on.is_empty() => format!("{name} · {on}"),
        _ => name,
    }
}

/// A field's row detail: its value, and "Read-only" when it cannot be changed.
fn field_detail(f: &tandem_protocol::engine::FieldItem) -> String {
    use tandem_protocol::engine::{FieldKind, FieldValue};
    let value = match (&f.kind, &f.value) {
        (FieldKind::Radio, FieldValue::Bool(true)) => "Chosen".to_string(),
        (FieldKind::Radio, FieldValue::Bool(false)) => "Not chosen".to_string(),
        (FieldKind::Radio, FieldValue::Text(v)) => {
            if f.on.as_deref().is_some_and(|on| on == v) {
                "Chosen".to_string()
            } else {
                "Not chosen".to_string()
            }
        }
        (FieldKind::Signature, _) => {
            if f.signed == Some(true) {
                "Signed".to_string()
            } else {
                "Not signed".to_string()
            }
        }
        (_, value) => field_value_text(value),
    };
    if f.read_only {
        format!("Read-only · {value}")
    } else {
        value
    }
}

/// When a recent file last changed, in plain words ("Today", "Yesterday", "3 days ago");
/// "Not found" when it is gone.
fn recent_when(path: &std::path::Path) -> String {
    let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return "Not found".into();
    };
    let age = std::time::SystemTime::now()
        .duration_since(modified)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = age / 86_400;
    match days {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=13 => format!("{days} days ago"),
        14..=59 => format!("{} weeks ago", days / 7),
        60..=729 => format!("{} months ago", days / 30),
        _ => format!("{} years ago", days / 365),
    }
}

/// The live region says where you are, not how the page was drawn (review view#13):
/// "Page 1 of 3 — rendered in 7 ms at 131%" is read as "Page 1 of 3".
fn plain_announcement(status: &str) -> String {
    match status.find(" — rendered in ") {
        Some(at) => status[..at].to_string(),
        None => status.to_string(),
    }
}

/// A field's value as a person reads it in the list.
fn field_value_text(value: &tandem_protocol::engine::FieldValue) -> String {
    let raw = serde_json::to_value(value).unwrap_or(Value::Null);
    match raw {
        Value::String(s) if s.is_empty() => "Empty".into(),
        Value::String(s) => s.chars().take(80).collect(),
        Value::Bool(true) => "Checked".into(),
        Value::Bool(false) => "Not checked".into(),
        Value::Null => "Empty".into(),
        other => other.to_string().chars().take(80).collect(),
    }
}

/// The page-moves that put `pages` (sorted) together before index `to`, each one page-move
/// straight to its final place (pages moving down go last-first, pages moving up first-last).
struct MovePlan {
    /// (current index, final index) per page-move, in order.
    steps: Vec<(usize, usize)>,
    /// Where the moved pages end up.
    landed: Vec<usize>,
}

fn move_plan(count: usize, pages: &[usize], to: usize) -> MovePlan {
    let to = to.min(count);
    let base = (0..to).filter(|p| !pages.contains(p)).count();
    let mut order: Vec<usize> = (0..count).collect();
    let mut steps = Vec::new();
    let place = |order: &mut Vec<usize>, steps: &mut Vec<(usize, usize)>, k: usize| {
        let page = pages[k];
        let dest = base + k;
        if let Some(cur) = order.iter().position(|p| *p == page)
            && cur != dest
        {
            let item = order.remove(cur);
            order.insert(dest, item);
            steps.push((cur, dest));
        }
    };
    let down: Vec<usize> = (0..pages.len()).filter(|k| pages[*k] < to).collect();
    for k in down.iter().rev() {
        place(&mut order, &mut steps, *k);
    }
    for k in (0..pages.len()).filter(|k| pages[*k] >= to) {
        place(&mut order, &mut steps, k);
    }
    MovePlan {
        steps,
        landed: (base..base + pages.len()).collect(),
    }
}

/// What Compare found, in plain words: "The text is the same. 0.4% of the page looks
/// different." (the drive keeps its own "Page 1 vs 2: 3 lines same, …" summary).
fn compare_summary(c: &crate::session::types::CompareDoc, name_a: &str) -> Option<String> {
    let pixels = c.pixels.as_ref()?;
    let text = c.text.as_ref()?;
    let lines = |n: i64| {
        if n == 1 {
            "1 line".to_string()
        } else {
            format!("{n} lines")
        }
    };
    let words = match text.refused.as_deref() {
        Some("too-many") => {
            "The pages have too many lines to compare their text, so only how they look was compared.".to_string()
        }
        Some("read-only") => {
            "The file does not allow copying its text, so only how the pages look was compared.".to_string()
        }
        Some(_) => "The text could not be compared, only how the pages look.".to_string(),
        None => match (text.removed, text.added) {
            (0, 0) => "The text is the same.".to_string(),
            (r, 0) => format!("{} only in {name_a}.", lines(r)),
            (0, a) => format!("{} only in {}.", lines(a), c.name),
            (r, a) => format!("{} only in {name_a}, {} only in {}.", lines(r), lines(a), c.name),
        },
    };
    let percent = pixels.ratio * 100.0;
    let look = if pixels.differing == 0 {
        "The pages look the same.".to_string()
    } else if percent < 0.1 {
        "Less than 0.1% of the page looks different.".to_string()
    } else {
        format!("{:.1}% of the page looks different.", percent)
    };
    Some(format!("{words} {look}"))
}

/// "Undo “Rotated page 2”" for the newest entry the window knows, when it said something.
/// Words that carry their own quotes ("Highlighted “quarterly”") are not quoted again; a
/// name from the engine's step ("Add stamp") reads as a menu item ("Undo Add stamp").
fn step_label(verb: &str, can: bool, steps: &[Step]) -> Option<String> {
    let (_, said, _) = steps.last().filter(|_| can)?;
    if said.is_empty() {
        return None;
    }
    let quoted = said.contains(['“', '”', '"', '‘', '’']);
    let named = said.starts_with(ENGINE_STEP_MARK);
    Some(if named {
        format!("{verb} {}", &said[ENGINE_STEP_MARK.len()..])
    } else if quoted {
        format!("{verb} {said}")
    } else {
        format!("{verb} “{said}”")
    })
}

/// Marks an undo name taken from the engine's step (not what the action said).
const ENGINE_STEP_MARK: &str = "\u{1}";

/// What an action said, as an Undo name: no trailing full stop, no internal words ("in 3
/// runs", resource names like PA1), no coordinates ("Moved image to 290, 554" is "Moved
/// image").
fn plain_step(said: &str) -> String {
    let mut text = said.trim().trim_end_matches('.').to_string();
    for cut in [" in ", " to "] {
        if let Some(at) = text.rfind(cut) {
            let rest = &text[at + cut.len()..];
            let internal = rest.ends_with(" runs")
                || rest.ends_with(" run")
                || rest
                    .chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, ',' | ' ' | '.' | '-'));
            // Only outside quoted words.
            let quotes = text[..at].matches(['“', '"']).count();
            let closes = text[..at].matches(['”', '"']).count();
            let inside = text[..at].contains('“') && quotes > closes;
            if internal && !rest.is_empty() && !inside {
                text.truncate(at);
            }
        }
    }
    // Resource names (PA1, Fm1, Im0) are not words people know: say "drawing".
    let mut out = Vec::new();
    let mut quoted = false;
    for word in text.split(' ') {
        let opens = word.matches(['“', '‘']).count();
        let closes = word.matches(['”', '’']).count();
        let resource = !quoted
            && word.len() >= 3
            && word.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && word.chars().last().is_some_and(|c| c.is_ascii_digit())
            && word.chars().all(|c| c.is_ascii_alphanumeric())
            && word.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 2;
        out.push(if resource { "drawing" } else { word });
        if opens > closes {
            quoted = true;
        } else if closes > opens {
            quoted = false;
        }
    }
    out.join(" ")
}

/// The engine's step names ("add highlight", "move text box") as Undo names.
fn engine_step_words(step: &str) -> String {
    let words = match step {
        "add strikeout" => "add strikethrough".to_string(),
        "add square" => "add rectangle".to_string(),
        "add circle" => "add ellipse".to_string(),
        "add ink" => "add drawing".to_string(),
        "add freetext" => "add text box".to_string(),
        "add text" => "add note".to_string(),
        "edit annotation" => "edit comment".to_string(),
        "delete annotation" => "delete comment".to_string(),
        other => other.replace(['-', '_'], " "),
    };
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Whether an action is the window's Undo (`Some(false)`) or Redo (`Some(true)`).
fn undo_kind(action: &UiAction) -> Option<bool> {
    match action {
        UiAction::Edit { op, .. } | UiAction::Click { id: op } => match op.as_str() {
            "undo" => Some(false),
            "redo" => Some(true),
            _ => None,
        },
        _ => None,
    }
}

/// "1-3, 5" for [0, 1, 2, 4].
fn ranges(pages: &[usize]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < pages.len() {
        let start = pages[i];
        let mut end = start;
        while i + 1 < pages.len() && pages[i + 1] == end + 1 {
            i += 1;
            end = pages[i];
        }
        out.push(if start == end {
            format!("{}", start + 1)
        } else {
            format!("{}-{}", start + 1, end + 1)
        });
        i += 1;
    }
    out.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_ranges_read_like_people_write_them() {
        assert_eq!(ranges(&[0, 1, 2, 4]), "1-3, 5");
        assert_eq!(ranges(&[3]), "4");
    }

    #[test]
    fn pixels_for_the_open_document() {
        let mut s = Session::for_drive(crate::host::Host::new(
            crate::host::HostConfig {
                user_data: crate::host::util::session_scratch("tandem-ui-test"),
                smoke: true,
                echo_stdout: false,
            },
            Box::new(crate::platform::Scripted::new(
                crate::platform::Answers::default(),
            )),
        ));
        s.start(Some("hello.pdf"));
        s.ui_pump();
        let (w, h, px) = s.ui_pixels("page", 0, 1.0).expect("renders");
        assert_eq!((w, h), (612, 792));
        assert_eq!(px.len(), 612 * 792 * 4);
        let ink = px.as_chunks::<4>().0.iter().filter(|p| p[0] < 128).count();
        assert!(ink > 100, "the page has text: {ink} dark pixels");
        let state = s.ui_state();
        assert_eq!(state.view.screen, "editor");
    }

    fn session(fixture: &str) -> Session {
        let mut s = Session::for_drive(crate::host::Host::new(
            crate::host::HostConfig {
                user_data: crate::host::util::session_scratch("tandem-ui-test"),
                smoke: true,
                echo_stdout: false,
            },
            Box::new(crate::platform::Scripted::new(
                crate::platform::Answers::default(),
            )),
        ));
        s.start(Some(fixture));
        s.ui_pump();
        s
    }

    #[test]
    fn the_pad_shows_the_chosen_picture_and_the_saved_signature() {
        use crate::session::types::{DrawnSig, PadImage, SavedSignature};
        let mut s = session("hello.pdf");
        assert_eq!(s.ui_dialog_data("sigpad-dialog")["image"], Value::Null);
        s.pad_image = Some(PadImage {
            width: 40.0,
            height: 20.0,
            data: "iVBORw0K".into(),
        });
        s.saved_signature = Some(SavedSignature::Drawn(DrawnSig {
            v: 1,
            width: 100.0,
            height: 30.0,
            strokes: vec![vec![[0.0, 0.0], [10.0, 5.0]]],
        }));
        let data = s.ui_dialog_data("sigpad-dialog");
        assert_eq!(data["image"]["data"], "iVBORw0K");
        assert_eq!(data["image"]["width"], 40.0);
        assert_eq!(data["saved"]["width"], 100.0);
        assert_eq!(data["saved"]["strokes"][0][1][0], 10.0);
    }

    #[test]
    fn one_zoom_for_pixels_overlay_and_editors() {
        let mut s = session("form.pdf");
        s.ui_act(UiAction::Viewport {
            width: 700.0,
            height: 600.0,
        });
        s.ui_act(UiAction::Zoom {
            to: "fit-width".into(),
            scale: None,
        });
        // The window grows (fit width): the session must repaint at the new zoom.
        s.ui_act(UiAction::Viewport {
            width: 1180.0,
            height: 900.0,
        });
        let view = s.ui_state().view;
        assert!((view.zoom.scale - 1178.0 / 612.0).abs() < 1e-9);
        assert_eq!(view.zoom.scale, s.shown_zoom);
        s.ui_act(UiAction::Tool {
            id: "fill-sign".into(),
            variant: None,
        });
        let field = s
            .fields
            .iter()
            .find(|f| f.kind == tandem_protocol::engine::FieldKind::Text && f.rect.len() == 4)
            .cloned()
            .expect("a text field");
        let r = field.rect.clone();
        let (x, y) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
        for phase in ["down", "up"] {
            s.ui_act(UiAction::Pointer {
                phase: phase.into(),
                page: 0,
                x,
                y,
                shift: false,
                alt: false,
                meta: false,
            });
        }
        let view = s.ui_state().view;
        let editor = view.overlays[0].editor.clone().expect("the field editor");
        // The editor's CSS box is page points × the scale the window draws at.
        let scale = view.zoom.scale;
        for i in 0..4 {
            let css = editor.rect[i] * scale;
            assert!(
                (css / scale - r[i]).abs() < 0.5,
                "editor {:?} vs field {:?}",
                editor.rect,
                r
            );
        }
        // Resize again with the editor open: it follows.
        s.ui_act(UiAction::Viewport {
            width: 800.0,
            height: 700.0,
        });
        let view = s.ui_state().view;
        assert_eq!(view.zoom.scale, s.shown_zoom);
        if let Some(editor) = view.overlays[0].editor.clone() {
            for i in 0..4 {
                assert!(
                    (editor.rect[i] - r[i]).abs() < 0.5,
                    "{:?} vs {:?}",
                    editor.rect,
                    r
                );
            }
        }
    }

    #[test]
    fn pixels_never_pass_the_engine_edge() {
        let mut s = session("hello.pdf");
        let (w, h, px) = s.ui_pixels("page", 0, 12.0).expect("clamped, not refused");
        assert!(w <= 4096 && h <= 4096, "{w}×{h}");
        assert_eq!(px.len(), (w * h * 4) as usize);
        assert!(s.ui_pixels("compare-a", 0, 1.0).is_err());
    }

    #[test]
    fn goto_stays_inside_the_document() {
        let mut s = session("hello.pdf");
        s.ui_act(UiAction::Goto { page: 99 });
        let state = s.ui_state();
        assert_eq!(state.view.notice, None);
        assert_eq!(state.view.page + 1, state.view.doc.unwrap().page_count);
    }

    #[test]
    fn placing_tools_stay_armed_and_esc_goes_back_to_select() {
        let mut s = session("hello.pdf");
        s.ui_act(UiAction::Tool {
            id: "text".into(),
            variant: None,
        });
        assert!(s.is_pressed(crate::session::dom_ids::TEXT_ADD_TOOL_BUTTON));
        s.ui_act(UiAction::Tool {
            id: "draw".into(),
            variant: None,
        });
        assert!(s.is_pressed(crate::session::dom_ids::INK_DRAW_BUTTON));
        let stroke = [[100.0, 100.0], [150.0, 130.0], [200.0, 120.0]];
        for (i, p) in stroke.iter().enumerate() {
            let phase = match i {
                0 => "down",
                2 => "up",
                _ => "move",
            };
            s.ui_act(UiAction::Pointer {
                phase: phase.into(),
                page: 0,
                x: p[0],
                y: p[1],
                shift: false,
                alt: false,
                meta: false,
            });
        }
        assert!(s.can_undo, "the stroke was added");
        assert!(
            s.is_pressed(crate::session::dom_ids::INK_DRAW_BUTTON),
            "the pen is ready for the next stroke"
        );
        assert_eq!(s.ui_state().view.tool.id, "draw");
        s.ui_act(UiAction::Key {
            key: "Escape".into(),
            shift: false,
            alt: false,
            meta: false,
        });
        assert_eq!(s.ui_state().view.tool.id, "select");
        assert!(!s.is_pressed(crate::session::dom_ids::INK_DRAW_BUTTON));
    }

    #[test]
    fn the_selection_bar_needs_a_selection_and_keeps_the_tool() {
        let mut s = session("hello.pdf");
        s.ui_act(UiAction::Edit {
            op: "highlight".into(),
            contents: Some(json!("selection")),
        });
        let view = s.ui_state().view;
        assert_eq!(view.tool.id, "select");
        assert!(view.search.query.is_empty(), "no search for a sample word");
        assert!(!s.can_undo);
    }

    #[test]
    fn grid_moves_go_straight_to_their_place() {
        let apply = |count: usize, pages: &[usize], to: usize| {
            let plan = move_plan(count, pages, to);
            let mut order: Vec<usize> = (0..count).collect();
            for (from, dest) in &plan.steps {
                let p = order.remove(*from);
                order.insert(*dest, p);
            }
            (order, plan.landed, plan.steps.len())
        };
        assert_eq!(apply(4, &[0, 2], 4), (vec![1, 3, 0, 2], vec![2, 3], 2));
        assert_eq!(apply(5, &[0, 4], 2), (vec![1, 0, 4, 2, 3], vec![1, 2], 2));
        assert_eq!(apply(5, &[3], 0), (vec![3, 0, 1, 2, 4], vec![0], 1));
        assert_eq!(apply(3, &[1], 2), (vec![0, 1, 2], vec![1], 0));
        assert_eq!(
            apply(6, &[1, 2, 4], 6),
            (vec![0, 3, 5, 1, 2, 4], vec![3, 4, 5], 3)
        );
    }

    fn act_pages(s: &mut Session, action: &str, pages: Vec<usize>, to: Option<usize>) {
        s.ui_act(UiAction::Pages {
            action: action.into(),
            pages,
            to,
        });
    }

    #[test]
    fn a_grid_action_is_one_undo() {
        let mut s = session("large.pdf");
        let count = s.doc.as_ref().unwrap().page_count;
        assert!(count >= 3, "{count} pages");
        s.ui_act(UiAction::Tool {
            id: "pages".into(),
            variant: None,
        });
        act_pages(&mut s, "rotate-right", vec![0, 1, 2], None);
        let rotations = |s: &Session| s.doc.as_ref().unwrap().rotations[..3].to_vec();
        assert_eq!(rotations(&s), vec![90, 90, 90]);
        act_pages(&mut s, "rotate-right", vec![1], None);
        assert_eq!(rotations(&s), vec![90, 180, 90]);
        let undo = |s: &mut Session| {
            s.ui_act(UiAction::Edit {
                op: "undo".into(),
                contents: None,
            })
        };
        let label = s.ui_state().view.doc.unwrap().undo_label;
        assert_eq!(label.as_deref(), Some("Undo “Rotated page 2”"));
        undo(&mut s);
        assert_eq!(rotations(&s), vec![90, 90, 90], "the single rotation");
        let doc = s.ui_state().view.doc.unwrap();
        assert_eq!(doc.undo_label.as_deref(), Some("Undo “Rotated 3 pages”"));
        assert_eq!(doc.redo_label.as_deref(), Some("Redo “Rotated page 2”"));
        undo(&mut s);
        assert_eq!(rotations(&s), vec![0, 0, 0], "all three pages at once");
        assert!(!s.can_undo);
        s.ui_act(UiAction::Edit {
            op: "redo".into(),
            contents: None,
        });
        assert_eq!(
            rotations(&s),
            vec![90, 90, 90],
            "redo takes the three again"
        );
        // The Page menu acts on the grid's selection in the Pages tool.
        act_pages(&mut s, "select", vec![0, 2], None);
        s.ui_menu("edit:rotate-left");
        assert_eq!(rotations(&s), vec![0, 90, 0]);
    }

    #[test]
    fn a_step_without_undo_does_not_shift_the_grid_groups() {
        // Review fix 2: security-set moves the revision but adds no journal step.
        let mut s = session("large.pdf");
        s.ui_act(UiAction::Tool {
            id: "pages".into(),
            variant: None,
        });
        act_pages(&mut s, "rotate-right", vec![0, 1, 2], None);
        act_pages(&mut s, "rotate-right", vec![1], None);
        let rotations = |s: &Session| s.doc.as_ref().unwrap().rotations[..3].to_vec();
        assert_eq!(rotations(&s), vec![90, 180, 90]);
        let revision = s.revision;
        s.ui_act(UiAction::Edit {
            op: "security".into(),
            contents: None,
        });
        s.ui_act(UiAction::Dialog {
            id: crate::session::dom_ids::SECURITY_DIALOG.into(),
            button: crate::session::dom_ids::SECURITY_APPLY_BUTTON.into(),
            values: json!({ crate::session::dom_ids::SECURITY_MODE_NONE_INPUT: true }),
        });
        assert!(s.revision > revision, "security-set moved the revision");
        let label = s.ui_state().view.doc.unwrap().undo_label;
        assert_eq!(label.as_deref(), Some("Undo “Rotated page 2”"));
        let undo = |s: &mut Session| {
            s.ui_act(UiAction::Edit {
                op: "undo".into(),
                contents: None,
            })
        };
        undo(&mut s);
        assert_eq!(rotations(&s), vec![90, 90, 90], "the single rotation");
        undo(&mut s);
        assert_eq!(rotations(&s), vec![0, 0, 0], "all three pages at once");
        assert!(!s.can_undo);
    }

    #[test]
    fn a_refused_open_keeps_the_document_generation() {
        // Review fix 6: the open document stays, and so do its undo groups.
        let mut s = session("large.pdf");
        s.ui_act(UiAction::Tool {
            id: "pages".into(),
            variant: None,
        });
        act_pages(&mut s, "rotate-right", vec![0, 1], None);
        let generation = s.generation;
        assert!(
            s.open_document("bad.pdf", b"not a pdf at all".to_vec(), None, false)
                .is_none()
        );
        s.ui_pump();
        assert_eq!(s.generation, generation);
        assert_eq!(s.doc.as_ref().unwrap().name, "large.pdf");
        let label = s.ui_state().view.doc.unwrap().undo_label;
        assert_eq!(label.as_deref(), Some("Undo “Rotated 2 pages”"));
        s.ui_act(UiAction::Edit {
            op: "undo".into(),
            contents: None,
        });
        assert_eq!(s.doc.as_ref().unwrap().rotations[..2], [0, 0]);
        // A document that opens is a new generation.
        s.open_document(
            "hello.pdf",
            std::fs::read(crate::host::fixture_path("hello.pdf").unwrap()).unwrap(),
            None,
            false,
        );
        assert_eq!(s.generation, generation + 1);
    }

    #[test]
    fn the_menu_signature_is_ready_to_place_after_the_pad() {
        // Review fix 3: Add my signature with nothing saved opens the pad; its Save arms the
        // placement, and closing the dialogs does not put the old tool back over it.
        let mut s = session("hello.pdf");
        s.headless = false;
        assert!(s.saved_signature.is_none());
        s.ui_menu("edit:sign-place");
        assert!(s.dom.is_open(crate::session::dom_ids::SIGPAD_DIALOG));
        s.ui_act(UiAction::SignaturePad {
            strokes: vec![vec![[10.0, 10.0], [60.0, 40.0], [120.0, 20.0]]],
            width: 300.0,
            height: 120.0,
        });
        s.ui_act(UiAction::Click {
            id: crate::session::dom_ids::SIGPAD_SAVE_BUTTON.into(),
        });
        assert!(!s.dom.is_open(crate::session::dom_ids::SIGPAD_DIALOG));
        assert!(s.sign_armed, "the first drawn signature is ready to place");
        assert_eq!(s.ui_mode, UiMode::Forms);
        assert_eq!(s.ui_state().view.tool.id, "fill-sign");
    }

    #[test]
    fn compare_draws_both_files_in_plain_words() {
        let mut s = session("hello.pdf");
        let other = std::fs::read(crate::host::fixture_path("form.pdf").unwrap()).unwrap();
        s.open_compare(other, "form.pdf");
        s.ui_pump();
        let (wa, ha, a) = s.ui_pixels("compare-a", 0, 2.0).expect("left page");
        let (wb, hb, b) = s.ui_pixels("compare-b", 0, 2.0).expect("right page");
        let (_, _, o) = s.ui_pixels("compare-overlay", 0, 2.0).expect("differences");
        assert_eq!(a.len(), (wa * ha * 4) as usize);
        assert_eq!(o.len(), (wb * hb * 4) as usize);
        assert_ne!(a, b, "the two panes show the two files");
        assert!(
            o.as_chunks::<4>().0.iter().any(|p| p[3] > 0),
            "some pixels differ"
        );
        let view = s.ui_state().view;
        let compare = view.compare.expect("compare is shown");
        assert!(
            compare.summary.ends_with("looks different.") || compare.summary.ends_with("same."),
            "{}",
            compare.summary
        );
        assert!(
            !compare.summary.contains("lines same"),
            "{}",
            compare.summary
        );
        for row in &compare.rows {
            assert!(row.detail.starts_with("Only in "), "{}", row.detail);
        }
        s.ui_act(UiAction::Key {
            key: "Escape".into(),
            shift: false,
            alt: false,
            meta: false,
        });
        assert!(s.ui_state().view.compare.is_none(), "Esc closes Compare");
    }

    #[test]
    fn a_dialog_does_not_switch_the_tool() {
        let mut s = session("hello.pdf");
        s.ui_act(UiAction::Edit {
            op: "security".into(),
            contents: None,
        });
        let view = s.ui_state().view;
        assert!(view.dialog.is_some(), "the security dialog is open");
        assert_eq!(view.tool.id, "select");
        assert_eq!(view.tool.chip, None);
        s.ui_act(UiAction::Key {
            key: "Escape".into(),
            shift: false,
            alt: false,
            meta: false,
        });
        let view = s.ui_state().view;
        assert!(view.dialog.is_none());
        assert_eq!(view.tool.id, "select");
        assert_eq!(s.ui_mode, UiMode::Read);
    }

    #[test]
    fn a_target_takes_the_keyboard_focus() {
        let mut s = session("form.pdf");
        s.ui_act(UiAction::Tool {
            id: "fill-sign".into(),
            variant: None,
        });
        let target = s.ui_state().view.overlays[0]
            .targets
            .iter()
            .find(|t| t.kind == "field")
            .cloned()
            .expect("a field target");
        assert!(!target.focused);
        s.ui_act(UiAction::Focus {
            target: target.id.clone(),
        });
        let view = s.ui_state().view;
        let now = view.overlays[0]
            .targets
            .iter()
            .find(|t| t.id == target.id)
            .expect("still there");
        assert!(now.focused && now.selected);
    }

    #[test]
    fn the_certificate_signature_box_is_dragged_on_the_page() {
        use crate::session::dom_ids as ids;
        let mut s = session("hello.pdf");
        s.open_cert_sign_dialog(None);
        s.ui_pump();
        assert!(s.ui_state().view.dialog.is_some());
        s.ui_act(UiAction::Click {
            id: CERT_BOX_BUTTON.into(),
        });
        let view = s.ui_state().view;
        assert!(view.dialog.is_none(), "the dialog steps aside");
        assert!(view.tool.chip.unwrap().starts_with("Drag a box"));
        for (phase, x, y) in [
            ("down", 300.0, 600.0),
            ("move", 400.0, 640.0),
            ("up", 460.0, 670.0),
        ] {
            s.ui_act(UiAction::Pointer {
                phase: phase.into(),
                page: 0,
                x,
                y,
                shift: false,
                alt: false,
                meta: false,
            });
            if phase == "move" {
                let draft = s.ui_state().view.overlays[0].draft;
                assert_eq!(draft, Some([300.0, 600.0, 400.0, 640.0]));
            }
        }
        let view = s.ui_state().view;
        assert_eq!(
            view.dialog.map(|d| d.id).as_deref(),
            Some(ids::CERT_SIGN_DIALOG)
        );
        let v = |id: &str| s.dom.value(id);
        assert_eq!(
            [
                v(ids::CERT_SIGN_PAGE),
                v(ids::CERT_SIGN_X),
                v(ids::CERT_SIGN_Y),
                v(ids::CERT_SIGN_W),
                v(ids::CERT_SIGN_H)
            ],
            ["1", "300", "600", "160", "70"].map(String::from)
        );
        assert!(s.dom.checked(ids::CERT_SIGN_VISIBLE));
        // Esc while placing goes back to the dialog, not out of it.
        s.ui_act(UiAction::Click {
            id: CERT_BOX_BUTTON.into(),
        });
        s.ui_act(UiAction::Key {
            key: "Escape".into(),
            shift: false,
            alt: false,
            meta: false,
        });
        assert!(s.ui_state().view.dialog.is_some());
    }

    fn press(s: &mut Session, path: &[(f64, f64)]) {
        for (i, (x, y)) in path.iter().enumerate() {
            let phase = if i == 0 {
                "down"
            } else if i + 1 == path.len() {
                "up"
            } else {
                "move"
            };
            s.ui_act(UiAction::Pointer {
                phase: phase.into(),
                page: 0,
                x: *x,
                y: *y,
                shift: false,
                alt: false,
                meta: false,
            });
        }
    }

    fn key(s: &mut Session, key: &str) {
        s.ui_act(UiAction::Key {
            key: key.into(),
            shift: false,
            alt: false,
            meta: false,
        });
    }

    #[test]
    fn a_drag_inside_a_redaction_box_moves_it() {
        let mut s = session("hello.pdf");
        s.ui_act(UiAction::Tool {
            id: "redact".into(),
            variant: None,
        });
        press(&mut s, &[(100.0, 100.0), (200.0, 150.0), (300.0, 200.0)]);
        assert_eq!(s.ui_state().view.redact.boxes.len(), 1);
        press(&mut s, &[(150.0, 120.0), (180.0, 150.0), (200.0, 160.0)]);
        let boxes = s.redact_ui.boxes.clone();
        assert_eq!(boxes.len(), 1, "no box inside the box: {boxes:?}");
        let r = &boxes[0].rect;
        assert!(
            (r[0] - 150.0).abs() < 1.0 && (r[1] - 140.0).abs() < 1.0,
            "{r:?}"
        );
        // Another drag outside makes a second box.
        press(&mut s, &[(320.0, 400.0), (360.0, 420.0), (400.0, 450.0)]);
        assert_eq!(s.redact_ui.boxes.len(), 2);
        // With the tool just chosen (its first drag armed) and a box already there.
        let mut s = session("hello.pdf");
        s.ui_act(UiAction::Edit {
            op: "redact".into(),
            contents: Some(json!({ "page": 0, "rect": [100.0, 100.0, 300.0, 200.0] })),
        });
        s.ui_act(UiAction::Tool {
            id: "select".into(),
            variant: None,
        });
        s.ui_act(UiAction::Tool {
            id: "redact".into(),
            variant: None,
        });
        assert_eq!(s.redact_ui.boxes.len(), 1);
        press(&mut s, &[(150.0, 120.0), (180.0, 150.0), (200.0, 160.0)]);
        assert_eq!(s.redact_ui.boxes.len(), 1, "{:?}", s.redact_ui.boxes);
    }

    fn signed() -> Session {
        let mut s = session("hello.pdf");
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/p0b/signed-intact.pdf"
        );
        s.open_document(
            "signed-intact.pdf",
            std::fs::read(path).unwrap(),
            None,
            false,
        );
        s.ui_pump();
        s
    }

    #[test]
    fn a_drag_inside_a_box_on_a_signed_file_picks_it() {
        let mut s = signed();
        s.ui_menu("tool:redact");
        press(&mut s, &[(50.0, 40.0), (200.0, 100.0), (400.0, 140.0)]);
        assert_eq!(s.redact_ui.boxes.len(), 1, "{:?}", s.redact_ui.boxes);
        press(&mut s, &[(150.0, 90.0), (160.0, 100.0), (180.0, 120.0)]);
        assert_eq!(
            s.redact_ui.boxes.len(),
            1,
            "no box in the box: {:?}",
            s.redact_ui.boxes
        );
        let view = s.ui_state().view;
        assert!(view.redact.boxes[0].selected, "the press picked the box");
        key(&mut s, "Delete");
        assert!(
            s.redact_ui.boxes.is_empty(),
            "Delete removes the picked box"
        );
    }

    fn tool(s: &mut Session, id: &str, variant: Option<&str>) {
        s.ui_act(UiAction::Tool {
            id: id.into(),
            variant: variant.map(String::from),
        });
    }

    #[test]
    fn another_document_starts_with_select() {
        let mut s = session("form.pdf");
        tool(&mut s, "fill-sign", None);
        assert_eq!(s.ui_state().view.tool.id, "fill-sign");
        s.ui_act(UiAction::New { size: None });
        let notice = s.ui_state().view.notice;
        assert_eq!(
            s.doc.as_ref().map(|d| d.name.as_str()),
            Some("Untitled.pdf"),
            "{notice:?}"
        );
        let view = s.ui_state().view;
        assert_eq!(view.tool.id, "select", "a new document starts with Select");
        assert_eq!(s.ui_mode, UiMode::Read);
        // An Untitled document can be saved from the toolbar (Save… asks where).
        assert!(view.doc.unwrap().can_save);
        tool(&mut s, "draw", None);
        let mut s2 = signed();
        tool(&mut s2, "fill-sign", None);
        assert_eq!(
            s2.ui_state().view.tool.id,
            "select",
            "a signed file is read"
        );
        // A tool the document stops allowing falls back to Select.
        let mut s = session("form.pdf");
        tool(&mut s, "fill-sign", None);
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/p0b/signed-intact.pdf"
        );
        s.open_document(
            "signed-intact.pdf",
            std::fs::read(path).unwrap(),
            None,
            false,
        );
        s.ui_pump();
        assert_eq!(s.ui_state().view.tool.id, "select");
        assert!(s.ui_state().view.tool.chip.is_none());
    }

    #[test]
    fn fill_and_sign_types_where_you_click() {
        let mut s = session("hello.pdf");
        tool(&mut s, "fill-sign", None);
        press(&mut s, &[(300.0, 500.0), (300.0, 500.0)]);
        let view = s.ui_state().view;
        let editor = view.overlays[0]
            .editor
            .clone()
            .expect("a text box editor at the click");
        assert!((editor.rect[0] - 300.0).abs() < 30.0, "{:?}", editor.rect);
        s.ui_act(UiAction::EditorInput {
            value: "Jane Doe".into(),
        });
        s.ui_act(UiAction::EditorCommit);
        assert!(
            s.notes
                .iter()
                .any(|n| n.r#type == "FreeText" && n.contents == "Jane Doe"),
            "{:?}",
            s.notes
        );
        assert_eq!(s.ui_state().view.tool.id, "fill-sign", "the tool stays");
        // Insert ▸ Text Box arms the text box tool: the next click places one.
        s.ui_menu("edit:freetext");
        let view = s.ui_state().view;
        assert_eq!(view.tool.id, "textbox");
        assert!(view.dialog.is_none(), "no dialog at a fixed spot");
        assert!(
            view.tool
                .chip
                .unwrap()
                .starts_with("Click where the text box goes")
        );
        press(&mut s, &[(100.0, 600.0), (100.0, 600.0)]);
        assert!(s.ui_state().view.overlays[0].editor.is_some());
    }

    #[test]
    fn fields_are_filled_not_moved_in_fill_and_sign() {
        let mut s = session("form.pdf");
        tool(&mut s, "fill-sign", None);
        let field = s
            .fields
            .iter()
            .find(|f| f.kind == tandem_protocol::engine::FieldKind::Text && f.rect.len() == 4)
            .cloned()
            .expect("a text field");
        let target = s.ui_state().view.overlays[0]
            .targets
            .iter()
            .find(|t| t.kind == "field")
            .cloned()
            .unwrap();
        s.ui_act(UiAction::Focus { target: target.id });
        for t in &s.ui_state().view.overlays[0].targets {
            if t.kind == "field" {
                assert!(t.handles.is_empty() && !t.movable, "{t:?}");
            }
        }
        // A drag from the field's corner does not resize it.
        let r = field.rect.clone();
        press(
            &mut s,
            &[
                (r[2], r[3]),
                (r[2] + 40.0, r[3] + 40.0),
                (r[2] + 60.0, r[3] + 60.0),
            ],
        );
        let now = s
            .fields
            .iter()
            .find(|f| f.id == field.id)
            .unwrap()
            .rect
            .clone();
        assert_eq!(now, r);
    }

    #[test]
    fn select_picks_and_deletes_annotations() {
        let mut s = session("hello.pdf");
        tool(&mut s, "draw", Some("rect"));
        press(&mut s, &[(300.0, 300.0), (400.0, 350.0), (450.0, 380.0)]);
        key(&mut s, "Escape");
        assert_eq!(s.ui_state().view.tool.id, "select");
        let before = s.notes.len();
        assert!(before >= 1);
        press(&mut s, &[(375.0, 340.0), (375.0, 340.0)]);
        let view = s.ui_state().view;
        let t = view.overlays[0]
            .targets
            .iter()
            .find(|t| t.kind == "annot")
            .cloned()
            .expect("the rectangle is a target");
        assert!(t.selected, "{t:?}");
        assert!(t.label.starts_with("Rectangle"), "{}", t.label);
        key(&mut s, "Delete");
        assert_eq!(s.notes.len(), before - 1, "Delete removed it");
        let doc = s.ui_state().view.doc.unwrap();
        assert!(doc.undo_label.is_some_and(|l| !l.contains("““")));
        // Esc lets a picked annotation go before leaving the tool.
        s.ui_act(UiAction::Edit {
            op: "undo".into(),
            contents: None,
        });
        press(&mut s, &[(375.0, 340.0), (375.0, 340.0)]);
        assert!(s.ui.annot_selected.is_some());
        key(&mut s, "Escape");
        assert!(s.ui.annot_selected.is_none());
    }

    #[test]
    fn select_moves_a_text_box() {
        let mut s = session("hello.pdf");
        tool(&mut s, "textbox", None);
        press(&mut s, &[(100.0, 600.0), (100.0, 600.0)]);
        s.ui_act(UiAction::EditorInput {
            value: "Moved".into(),
        });
        s.ui_act(UiAction::EditorCommit);
        key(&mut s, "Escape");
        key(&mut s, "Escape");
        assert_eq!(s.ui_state().view.tool.id, "select");
        let item = s
            .notes
            .iter()
            .find(|n| n.r#type == "FreeText")
            .cloned()
            .expect("the text box");
        let r = item.rect.clone();
        let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
        press(
            &mut s,
            &[(cx, cy), (cx + 20.0, cy + 10.0), (cx + 50.0, cy + 30.0)],
        );
        let now = s
            .notes
            .iter()
            .find(|n| n.id == item.id)
            .unwrap()
            .rect
            .clone();
        assert!((now[0] - (r[0] + 50.0)).abs() < 1.0, "{r:?} → {now:?}");
        assert!((now[1] - (r[1] + 30.0)).abs() < 1.0, "{r:?} → {now:?}");
        key(&mut s, "ArrowRight");
        let after = s
            .notes
            .iter()
            .find(|n| n.id == item.id)
            .unwrap()
            .rect
            .clone();
        assert!((after[0] - now[0] - 1.0).abs() < 0.1, "{now:?} → {after:?}");
    }

    #[test]
    fn undo_names_read_plainly() {
        assert_eq!(plain_step("Replaced “a” in 3 runs."), "Replaced “a”");
        assert_eq!(plain_step("Moved image to 290, 554"), "Moved image");
        assert_eq!(plain_step("Moved PA1"), "Moved drawing");
        assert_eq!(plain_step("Added “Q4 PA1 notes”"), "Added “Q4 PA1 notes”");
        assert_eq!(plain_step("Added A4 page"), "Added A4 page");
        let steps = vec![(1, "Highlighted “quarterly”".to_string(), vec![])];
        assert_eq!(
            step_label("Undo", true, &steps).as_deref(),
            Some("Undo Highlighted “quarterly”")
        );
        let steps = vec![(1, format!("{ENGINE_STEP_MARK}Add stamp"), vec![])];
        assert_eq!(
            step_label("Undo", true, &steps).as_deref(),
            Some("Undo Add stamp")
        );
        assert_eq!(engine_step_words("add strikeout"), "Add strikethrough");
    }

    #[test]
    fn undoing_a_multi_page_delete_selects_the_pages_again() {
        let mut s = session("large.pdf");
        tool(&mut s, "pages", None);
        act_pages(&mut s, "delete", vec![1, 2, 3], None);
        assert_eq!(s.ui_state().view.page_selection, vec![1]);
        s.ui_act(UiAction::Edit {
            op: "undo".into(),
            contents: None,
        });
        assert_eq!(s.ui_state().view.page_selection, vec![1, 2, 3]);
    }

    #[test]
    fn fit_width_fits_the_widest_page() {
        let mut s = session("large.pdf");
        s.ui_act(UiAction::Viewport {
            width: 900.0,
            height: 700.0,
        });
        s.ui_act(UiAction::Zoom {
            to: "fit-width".into(),
            scale: None,
        });
        let portrait = s.ui_state().view.zoom.scale;
        act_pages(&mut s, "rotate-right", vec![2], None);
        let doc = s.ui_state().view.doc.unwrap();
        let widest = doc.pages.iter().map(|p| p.width).fold(0.0, f64::max);
        assert!(widest > doc.pages[0].width);
        let view = s.ui_state().view;
        assert!(
            view.zoom.scale < portrait,
            "{} vs {portrait}",
            view.zoom.scale
        );
        assert!((view.zoom.scale * widest - 898.0).abs() < 1.0);
        // The same scale on every page as it scrolls.
        s.ui_act(UiAction::Goto { page: 2 });
        assert_eq!(s.ui_state().view.zoom.scale, view.zoom.scale);
    }

    #[test]
    fn the_live_region_says_what_just_happened() {
        let mut s = session("large.pdf");
        let seq = s.ui_state().view.announce_seq;
        tool(&mut s, "pages", None);
        act_pages(&mut s, "rotate-right", vec![0, 1], None);
        let view = s.ui_state().view;
        assert_eq!(view.announce, "Rotated 2 pages");
        assert!(view.announce_seq > seq);
        s.ui_act(UiAction::Goto { page: 3 });
        let view = s.ui_state().view;
        assert!(view.announce.starts_with("Page 4"), "{}", view.announce);
    }

    #[test]
    fn the_page_menu_leaves_out_what_cannot_apply() {
        let mut s = session("hello.pdf");
        tool(&mut s, "redact", None);
        s.ui_act(UiAction::ContextMenu {
            page: 0,
            x: 300.0,
            y: 700.0,
        });
        let menu = s.ui_state().view.menu.expect("the page menu");
        assert!(menu.items.iter().all(|(_, _, on)| *on), "{:?}", menu.items);
    }

    #[test]
    fn rotated_pages_keep_the_engine_size() {
        let mut s = session("hello.pdf");
        s.ui_act(UiAction::Pages {
            action: "rotate-right".into(),
            pages: vec![0],
            to: None,
        });
        let page = s.ui_state().view.doc.unwrap().pages[0].clone();
        assert_eq!((page.width, page.height, page.rotation), (792.0, 612.0, 90));
    }

    #[test]
    fn announcements_say_where_you_are() {
        assert_eq!(
            plain_announcement("Page 1 of 3 — rendered in 7 ms at 131%"),
            "Page 1 of 3"
        );
        assert_eq!(plain_announcement("Saved a.pdf"), "Saved a.pdf");
    }

    #[test]
    fn tools_map_to_modes_and_back() {
        for tool in ["select", "annotate", "text", "fill-sign", "redact", "pages"] {
            assert_eq!(tool_of(mode_of(tool)), tool);
        }
    }

    #[test]
    fn a_signed_file_offers_an_editable_copy_and_verify() {
        let mut s = session("hello.pdf");
        let banner = s.ui_state().view.banner;
        assert!(banner.is_none_or(|b| !b.actions.iter().any(|(id, _)| id == "editable-copy")));
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/p0b/signed-intact.pdf"
        );
        let bytes = std::fs::read(path).unwrap();
        s.open_document("signed-intact.pdf", bytes, None, false);
        s.ui_pump();
        let banner = s.ui_state().view.banner.expect("the signed banner");
        assert_eq!(banner.kind, "warning");
        let ids: Vec<&str> = banner.actions.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["editable-copy", "verify-open", "banner-dismiss"]);
        assert_eq!(banner.actions[0].1, "Make an editable copy…");
    }
}
