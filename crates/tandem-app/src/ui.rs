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
    /// Documents an earlier run left unsaved (D8), offered on the start screen.
    pub recovered: Vec<crate::autosave::Recovered>,
}

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

fn mode_of(tool: &str) -> UiMode {
    match tool {
        "highlight" | "draw" | "note" => UiMode::Annotate,
        "text" => UiMode::Edit,
        "fill-sign" => UiMode::Forms,
        "redact" => UiMode::Redact,
        "pages" => UiMode::Organize,
        _ => UiMode::Read,
    }
}

fn tool_of(mode: UiMode) -> &'static str {
    match mode {
        UiMode::Annotate => "highlight",
        UiMode::Edit => "text",
        UiMode::Forms => "fill-sign",
        UiMode::Redact => "redact",
        UiMode::Organize => "pages",
        UiMode::Read => "select",
    }
}

fn node_view(dom: &Dom, id: &str) -> NodeView {
    let n = dom.node(id);
    NodeView { text: n.text, value: n.value, hidden: n.hidden, disabled: n.disabled, checked: n.checked, open: n.open }
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
        self.ui_sync_tool();
        self.autosave_tick();
    }

    /// Keeps the strip's tool in step when a menu, a drive step or the session changed the mode.
    fn ui_sync_tool(&mut self) {
        let mode = self.ui_mode;
        if mode_of(&self.ui.tool) != mode {
            self.ui.tool = tool_of(mode).to_string();
            self.ui.variant = None;
        }
        if self.redact_ui.active && self.ui.tool != "redact" {
            self.ui.tool = "redact".into();
        }
        let revision = self.revision;
        let name = self.doc.as_ref().map(|d| d.name.clone());
        if revision != self.ui.last_revision || name != self.ui.last_doc_name {
            self.ui.last_revision = revision;
            self.ui.last_doc_name = name;
            self.ui.generation += 1;
        }
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
        let id = if id == "fill" || id == "sign" { "fill-sign" } else { id };
        let leaving_redact = self.redact_ui.active && id != "redact";
        if leaving_redact {
            self.redact_ui_set_active(false);
        }
        self.cancel_pending_tools();
        self.ui.drag = None;
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
            "draw" if self.ui.variant.as_deref() == Some("pen") => self.edit("arm", Some(json!({ "tool": "ink" }))),
            "pages" => {}
            _ => {}
        }
        if id == "draw" || id == "highlight" {
            self.ui.color = if id == "highlight" { [0.97, 0.80, 0.11] } else { [0.12, 0.24, 0.55] };
        }
    }

    /// Esc: one layer per press (an open editor, then a selection, then the tool), and
    /// always back to Select at the end (X6).
    pub fn ui_escape(&mut self) {
        if self.any_dialog_open() {
            if let Some((id, _)) = crate::dialog_ids::DIALOGS.iter().find(|(id, _)| self.dom.is_open(id)) {
                self.on_dialog_cancel(id);
            }
            return;
        }
        let before = (self.armed_tool(), self.has_direct_focus(), self.direct_active.is_some(), self.redact_ui.active);
        self.edit("key", Some(json!({ "key": "Escape" })));
        let after = (self.armed_tool(), self.has_direct_focus(), self.direct_active.is_some(), self.redact_ui.active);
        if before == after && self.ui.tool != "select" {
            self.ui_set_tool("select", None);
        }
    }

    /// One action from the window.
    pub fn ui_act(&mut self, action: UiAction) {
        match action {
            UiAction::Edit { op, contents } => self.edit(&op, contents),
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
            UiAction::Goto { page } => self.edit("goto", Some(json!({ "page": page }))),
            UiAction::Zoom { to, scale } => match to.as_str() {
                "in" => self.zoom_step(1),
                "out" => self.zoom_step(-1),
                "actual" => self.ui_zoom_fixed(1.0),
                "fit-width" | "fit-page" => self.edit("zoom", Some(json!({ "mode": to }))),
                _ => self.ui_zoom_fixed(scale.unwrap_or(1.0)),
            },
            UiAction::Viewport { width, height } => self.set_viewport(width, height),
            UiAction::Search { query } => {
                if query.is_empty() {
                    self.on_search_input("");
                } else {
                    self.edit("find", Some(json!({ "query": query })));
                }
            }
            UiAction::SearchMove { by } => self.edit("find", Some(json!({ "dir": if by < 0 { "prev" } else { "next" } }))),
            UiAction::Pointer { phase, page, x, y, shift, alt, meta } => self.ui_pointer(&phase, page, x, y, shift, alt, meta),
            UiAction::Key { key, shift, alt, meta } => {
                if key == "Escape" {
                    self.ui_escape();
                } else {
                    self.key(&key, shift, alt, meta);
                }
            }
            UiAction::EditorInput { value } => self.ui_editor_input(&value),
            UiAction::EditorCommit => self.key("Enter", false, false, true),
            UiAction::Dialog { id: _, button, values } => {
                if let Value::Object(map) = values {
                    for (field, value) in map {
                        match value {
                            Value::Bool(b) => {
                                self.on_change(&field, "", b);
                            }
                            other => {
                                let text = other.as_str().map(str::to_string).unwrap_or_else(|| other.to_string());
                                self.on_input(&field, &text);
                            }
                        }
                    }
                }
                if !button.is_empty() {
                    self.on_click(&button);
                }
            }
            UiAction::Click { id } => {
                self.on_click(&id);
            }
            UiAction::Input { id, value } => {
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
                    for (other, _) in crate::dialog_ids::DIALOGS.iter().flat_map(|(_, ids)| ids.iter()) {
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
                } else {
                    self.on_click(&action);
                }
            }
            UiAction::Pane { which, open, tab } => {
                if which == "sidebar" {
                    let open = open.unwrap_or(!self.sidebar_open);
                    if let Some(tab) = tab {
                        self.ui.sidebar_tab = tab.clone();
                        self.on_tab_click(if tab == "outline" { "tab-bookmarks" } else { "tab-pages" });
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
            UiAction::SignaturePad { strokes, width, height } => {
                // The pad's strokes, scaled to the sign-draw contract (pad pixels).
                let _ = (width, height);
                self.edit("sign-draw", Some(json!({ "strokes": strokes })));
            }
        }
        self.ui_pump();
    }

    fn ui_zoom_fixed(&mut self, scale: f64) {
        let scale = scale.clamp(0.1, 8.0);
        self.set_zoom(crate::session::types::ZoomMode::Fixed, scale);
    }

    /// A pointer gesture on a page (PDF points), interpreted for the current tool.
    #[allow(clippy::too_many_arguments)]
    fn ui_pointer(&mut self, phase: &str, page: usize, x: f64, y: f64, shift: bool, alt: bool, meta: bool) {
        if self.doc.is_none() {
            return;
        }
        if phase == "down" && page as i64 != self.wanted_page {
            self.edit("goto", Some(json!({ "page": page })));
        }
        let tool = self.ui.tool.clone();
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
                    if let Some((_, start, _)) = self.ui.drag.clone() {
                        if (start[0] - x).abs() + (start[1] - y).abs() > 2.0 {
                            self.ui_select_text(start, [x, y]);
                            return;
                        }
                    }
                }
                "up" => {
                    if let Some((_, start, _)) = self.ui.drag.take() {
                        if (start[0] - x).abs() + (start[1] - y).abs() > 2.0 {
                            self.ui_select_text(start, [x, y]);
                            return;
                        }
                        self.on_text_select(None);
                    }
                }
                _ => {}
            }
        }
        self.pointer(phase, page, x, y, shift, alt, meta);
    }

    fn ui_add_shape(&mut self, variant: &str, a: [f64; 2], b: [f64; 2]) {
        let rect = [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])];
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
        let Some(lines) = self.text_layer() else { return };
        let locate = |p: [f64; 2]| -> Option<(usize, usize)> {
            let mut best: Option<(f64, usize, usize)> = None;
            for (li, line) in lines.iter().enumerate() {
                let n = line.text.chars().count();
                for ci in 0..=n {
                    let bx = crate::session::markup::char_box(line, ci.min(n.saturating_sub(1)), n.max(1));
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
            "comments" => self.on_note_list_action(if action == "select" { "edit" } else { action }, num),
            "outline" => self.on_outline_action(if action == "open" || action == "select" { "go" } else { action }, num),
            "links" => self.on_link_list_action(if action == "select" { "go" } else { action }, num),
            "redactions" => {
                if let Some(b) = self.redact_ui.boxes.iter().find(|b| b.id == num).cloned() {
                    if action == "remove" {
                        self.on_redact_panel_remove(b.page, b.id);
                    } else {
                        self.on_redact_panel_box(b.page, b.id);
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
                        self.pointer("down", page, (r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0, false, false, false);
                        self.pointer("up", page, (r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0, false, false, false);
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
        match action {
            "rotate-right" | "rotate-left" => {
                for p in &pages {
                    self.edit("goto", Some(json!({ "page": p })));
                    self.run_page_op(action);
                }
            }
            "delete" => {
                for p in pages.iter().rev() {
                    self.edit("goto", Some(json!({ "page": p })));
                    self.run_page_op("delete-page");
                }
            }
            "extract" => {
                let range = ranges(&pages);
                self.edit("extract", None);
                self.on_input("extract-range", &range);
            }
            "move" => {
                let Some(to) = to else { return };
                // Move each selected page, in order, to sit before `to` (page indices shift as we go).
                let mut target = to;
                for (k, original) in pages.iter().enumerate() {
                    let shift = pages[..k].iter().filter(|p| **p < *original).count();
                    let mut current = original - shift + pages[..k].iter().filter(|p| **p >= target).count().min(0);
                    let goal = if target > current { target - 1 } else { target };
                    while current != goal {
                        self.edit("goto", Some(json!({ "page": current })));
                        if current < goal {
                            self.run_page_op("move-down");
                            current += 1;
                        } else {
                            self.run_page_op("move-up");
                            current -= 1;
                        }
                    }
                    target = goal + 1;
                }
            }
            _ => {}
        }
    }

    /// The window's whole state.
    pub fn ui_state(&mut self) -> UiState {
        let state = self.state_summary();
        let view = self.ui_view();
        UiState { state, view }
    }

    fn ui_view(&mut self) -> View {
        let mut v = View { generation: self.ui.generation, ..View::default() };
        v.recent = self
            .host
            .recent_list()
            .into_iter()
            .map(|r| RecentFile { name: r.name.clone(), path: r.id.clone(), when: String::new(), folder: r.folder.clone() })
            .collect();
        v.dialog = self.ui_dialog();
        v.announce = self.dom.text(crate::session::dom_ids::STATUS_EL);
        let notice = (!self.dom.hidden(crate::session::dom_ids::ERROR_EL)).then(|| self.dom.text(crate::session::dom_ids::ERROR_EL));
        if let Some(text) = notice.filter(|t| !t.is_empty()) {
            let kind = self.notice_kind.as_str().to_string();
            v.notice = Some(Notice { kind, text, actions: vec![] });
        }
        if v.notice.is_none() {
            if let Some(r) = self.ui.recovered.first() {
                v.notice = Some(Notice {
                    kind: "warning".into(),
                    text: format!(
                        "Tandem PDF closed before you saved {}. Restore your unsaved changes? They open as a new copy; the original file is not changed.",
                        r.name
                    ),
                    actions: vec![(format!("restore:{}", r.id), "Restore".into()), (format!("forget:{}", r.id), "Discard".into())],
                });
            }
        }
        let Some(doc) = self.doc.clone() else {
            v.screen = "start".into();
            v.tool.id = "select".into();
            return v;
        };
        v.screen = if doc.info.needs_password { "password" } else { "editor" }.into();
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
                let (w, h) = if rotation % 180 != 0 { (p.height, p.width) } else { (p.width, p.height) };
                let label = self.labels.get(i).filter(|l| !l.is_empty()).cloned().unwrap_or_else(|| (i + 1).to_string());
                PageView { width: w, height: h, rotation, label }
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
            undo_label: None,
            redo_label: None,
            can_save: !self.dom.disabled(crate::session::dom_ids::SAVE_BUTTON),
        });
        let page = self.wanted_page.max(0);
        v.page = page as usize;
        v.zoom = ZoomView { mode: self.zoom_mode.as_str().into(), scale: self.zoom_now(page) };
        v.tool = ToolView {
            id: self.ui.tool.clone(),
            chip: self.ui_chip(),
            variant: self.ui.variant.clone(),
            color: Some(self.ui.color),
            width: Some(self.ui.width),
            available: self.ui_availability(read_only),
        };
        v.sidebar = PaneView { open: self.sidebar_open, tab: self.ui.sidebar_tab.clone() };
        v.panel = PaneView { open: self.ui.panel_open || self.redact_ui.active, tab: if self.redact_ui.active { "redactions".into() } else { self.ui.panel_tab.clone() } };
        if let Some(notice) = &v.notice {
            if notice.kind == "success" {
                v.toast = Some(notice.text.clone());
                v.notice = None;
            }
        }
        let banner_text = self.dom.text(crate::session::dom_ids::redact_ui::BANNER_TEXT_EL);
        if let Some(banner) = self.redact_ui.banner_state.as_ref().map(|_| banner_text).filter(|t| !t.is_empty()) {
            v.banner = Some(Notice { kind: "info".into(), text: banner, actions: vec![("redact-banner-original".into(), "Open original".into())] });
        } else if !self.dom.hidden(crate::session::dom_ids::BANNER_EL)
            && !self.dom.text(crate::session::dom_ids::BANNER_TEXT_EL).is_empty()
        {
            let text = self.dom.text(crate::session::dom_ids::BANNER_TEXT_EL);
            v.banner = Some(Notice { kind: "info".into(), text, actions: vec![("banner-dismiss".into(), "Dismiss".into())] });
        } else if doc.info.repaired {
            v.banner = Some(Notice { kind: "warning".into(), text: "This file was damaged and has been repaired. Check it before saving.".into(), actions: vec![] });
        }
        v.search = SearchView {
            query: self.search_query.clone(),
            count: self.search_hits.len(),
            index: self.search_index,
            truncated: self.search_truncated,
            busy: false,
        };
        v.overlays = vec![self.ui_overlay(page)];
        v.comments = self
            .notes
            .clone()
            .iter()
            .filter(|n| n.r#type != "Redact")
            .map(|n| Row {
                id: n.id.to_string(),
                page: Some(page as usize),
                title: crate::session::pages::annot_name(n).unwrap_or("Comment").to_string(),
                detail: if n.contents.is_empty() { "No text".into() } else { n.contents.chars().take(120).collect() },
                selected: false,
            })
            .collect();
        v.fields = self
            .fields
            .iter()
            .filter(|f| !f.name.is_empty())
            .map(|f| Row {
                id: f.id.to_string(),
                page: Some(page as usize),
                title: if f.label.is_empty() { f.name.clone() } else { f.label.clone() },
                detail: format!("{:?}", f.value).trim_matches('"').chars().take(80).collect(),
                selected: false,
            })
            .collect();
        v.outline = self
            .outline
            .iter()
            .map(|o| OutlineRow { id: o.id.to_string(), title: o.title.clone(), page: o.page.map(|p| p.max(0) as usize), uri: o.uri.clone(), depth: o.depth.max(0) as usize })
            .collect();
        v.layers = self
            .layers
            .iter()
            .map(|l| Row { id: l.id.to_string(), page: None, title: l.name.clone(), detail: if l.visible { "Shown".into() } else { "Hidden".into() }, selected: l.visible })
            .collect();
        let boxes = self.redact_ui.boxes.clone();
        v.redact = RedactView {
            boxes: boxes
                .iter()
                .map(|b| Row {
                    id: b.id.to_string(),
                    page: Some(b.page.max(0) as usize),
                    title: format!("Page {}", b.page + 1),
                    detail: if b.search { "Found text".into() } else { "Box".into() },
                    selected: false,
                })
                .collect(),
            apply_label: {
                let t = self.dom.text(crate::session::dom_ids::redact_ui::APPLY_OPEN);
                if t.is_empty() { "Redact areas…".into() } else { t }
            },
            can_apply: !boxes.is_empty(),
            underneath: self.redact_ui.underneath,
            flatten_note: Some(self.dom.text(crate::session::dom_ids::redact_ui::FLATTEN_NOTE_EL)).filter(|t| !t.is_empty()),
        };
        v
    }

    fn ui_chip(&self) -> Option<String> {
        if self.redact_ui.active {
            return Some(crate::messages::redact::REDACT_CHIP.to_string());
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
            "fill-sign" => "Filling in",
            "pages" => "Arranging pages",
            _ => return self.armed_tool().map(|t| format!("{t} · Esc to stop")),
        };
        Some(format!("{label} · Esc to stop"))
    }

    fn ui_availability(&self, read_only: bool) -> Vec<ToolAvailability> {
        let annotate = !read_only && self.can_annotate();
        let edit = !read_only && self.can_edit();
        let mut out = Vec::new();
        for (id, ok, why) in [
            ("highlight", annotate, "This file does not allow comments."),
            ("draw", annotate, "This file does not allow comments."),
            ("note", annotate, "This file does not allow comments."),
            ("text", edit, "This file does not allow changes."),
            ("redact", self.doc.is_some(), ""),
            ("pages", true, ""),
        ] {
            out.push(ToolAvailability { id: id.into(), enabled: ok, reason: (!ok).then(|| why.to_string()), badge: None });
        }
        if let Some(r) = out.iter_mut().find(|a| a.id == "redact") {
            let n = self.redact_ui.boxes.len();
            r.badge = (n > 0).then_some(n);
        }
        out
    }

    fn ui_overlay(&mut self, page: i64) -> PageOverlay {
        let mut o = PageOverlay { page: page.max(0) as usize, ..PageOverlay::default() };
        let hits: Vec<_> = self.search_hits.iter().enumerate().filter(|(_, h)| h.page == page).collect();
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
        for b in self.redact_ui.boxes.iter().filter(|b| b.page == page) {
            if b.rect.len() == 4 {
                o.redactions.push(Target {
                    kind: "redact".into(),
                    id: b.id.to_string(),
                    rect: [b.rect[0], b.rect[1], b.rect[2], b.rect[3]],
                    label: if self.redact_ui.underneath || self.redact_ui.peek { "peek".into() } else { String::new() },
                    selected: false,
                    movable: true,
                });
            }
        }
        if let Some(tint) = self.redact_ui.tint.as_ref().filter(|t| t.page == page) {
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
        let tool = self.ui.tool.clone();
        if matches!(tool.as_str(), "select" | "fill-sign") {
            for f in &self.fields {
                if f.rect.len() == 4 {
                    o.targets.push(Target {
                        kind: "field".into(),
                        id: f.id.to_string(),
                        rect: [f.rect[0], f.rect[1], f.rect[2], f.rect[3]],
                        label: if f.label.is_empty() { f.name.clone() } else { f.label.clone() },
                        selected: false,
                        movable: false,
                    });
                }
            }
        }
        for l in &self.links {
            if l.rect.len() == 4 {
                o.targets.push(Target {
                    kind: "link".into(),
                    id: l.id.to_string(),
                    rect: [l.rect[0], l.rect[1], l.rect[2], l.rect[3]],
                    label: match &l.uri {
                        Some(uri) => format!("Link to {uri}"),
                        None => format!("Link to page {}", l.page.unwrap_or(0) + 1),
                    },
                    selected: false,
                    movable: false,
                });
            }
        }
        if let Some(active) = &self.direct_active {
            let scale = self.zoom_now(page).max(0.01);
            let e = &active.editor;
            o.editor = Some(EditorView {
                kind: format!("{:?}", active.kind).to_lowercase(),
                rect: [e.left / scale, e.top / scale, (e.left + e.width) / scale, (e.top + e.height) / scale],
                value: e.value.clone(),
                font_px: e.font_px / scale,
                multiline: e.tag != crate::session::types::EditorTag::Input,
                placeholder: e.placeholder.clone(),
            });
        }
        o
    }

    fn ui_dialog(&self) -> Option<DialogView> {
        let (id, ids) = crate::dialog_ids::DIALOGS.iter().find(|(id, _)| self.dom.is_open(id))?;
        let mut nodes = BTreeMap::new();
        for (node, _) in ids.iter() {
            nodes.insert(node.to_string(), node_view(&self.dom, node));
        }
        let heading = ids.iter().find(|(_, kind)| *kind == "h2").map(|(n, _)| self.dom.text(n)).filter(|t| !t.is_empty());
        Some(DialogView {
            id: id.to_string(),
            title: heading.unwrap_or_else(|| dialog_title(id).to_string()),
            data: Value::Null,
            nodes,
            rows: self.ui_dialog_rows(id),
        })
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
            _ => Vec::new(),
        }
    }

    fn ui_editor_input(&mut self, value: &str) {
        if let Some(active) = self.direct_active.as_mut() {
            active.editor.value = value.to_string();
        }
        self.on_direct_editor_input("");
    }

    /// Page pixels for the window: `kind` "page" (the document page at `scale` device
    /// pixels per point) or "thumb".
    pub fn ui_pixels(&mut self, kind: &str, page: usize, scale: f64) -> Result<(u32, u32, Vec<u8>), String> {
        let op = if kind == "thumb" { "render" } else { "render" };
        let mut reply = self.request(json!({ "op": op, "page": page, "scale": scale }), vec![]).map_err(|e| e.message)?;
        let w = reply.get("pixelWidth").and_then(Value::as_f64).unwrap_or(0.0) as u32;
        let h = reply.get("pixelHeight").and_then(Value::as_f64).unwrap_or(0.0) as u32;
        let px = reply.take_bytes("pixels").unwrap_or_default();
        Ok((w, h, px))
    }

    /// A menu item (`tandem-desktop` menu ids).
    pub fn ui_menu(&mut self, id: &str) {
        match id {
            "open" => self.ui_act(UiAction::Open { path: None }),
            "new" => self.ui_act(UiAction::New { size: None }),
            "print" => self.ui_act(UiAction::Print),
            id if id.starts_with("save:") => self.ui_act(UiAction::Save { how: id[5..].into() }),
            id if id.starts_with("edit:") => self.ui_act(UiAction::Edit { op: id[5..].into(), contents: None }),
            id if id.starts_with("tool:") => self.ui_act(UiAction::Tool { id: id[5..].into(), variant: None }),
            "view:zoom-in" => self.ui_act(UiAction::Zoom { to: "in".into(), scale: None }),
            "view:zoom-out" => self.ui_act(UiAction::Zoom { to: "out".into(), scale: None }),
            "view:actual-size" => self.ui_act(UiAction::Zoom { to: "actual".into(), scale: None }),
            "view:fit-width" => self.ui_act(UiAction::Zoom { to: "fit-width".into(), scale: None }),
            "view:fit-page" => self.ui_act(UiAction::Zoom { to: "fit-page".into(), scale: None }),
            "view:sidebar" => self.ui_act(UiAction::Pane { which: "sidebar".into(), open: None, tab: None }),
            "view:panel" => self.ui_act(UiAction::Pane { which: "panel".into(), open: None, tab: None }),
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
        out.push(if start == end { format!("{}", start + 1) } else { format!("{}-{}", start + 1, end + 1) });
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
    fn tools_map_to_modes_and_back() {
        for tool in ["select", "highlight", "text", "fill-sign", "redact", "pages"] {
            assert_eq!(tool_of(mode_of(tool)), tool);
        }
    }
}
