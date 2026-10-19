//! What the Leptos UI draws (`UiState`) and what it sends (`UiAction`).
//!
//! The UI is a view of the core: every change goes through an action, the
//! core answers with a fresh `UiState`, and background changes (menus, a
//! drive script in a window) arrive as the `state` event with the same
//! payload. `UiState::state` is the drive STATE itself (docs/agent.md), so
//! people and agents see one truth; `view` adds what only drawing needs.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Everything the window shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiState {
    /// The drive STATE (`stateSummary()`), unchanged.
    pub state: Value,
    pub view: View,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    /// "start", "password" or "editor".
    pub screen: String,
    /// Bumps on every open and every change that redraws page pixels; part
    /// of the pixel URL so the web view never shows a stale page.
    pub generation: u64,
    pub doc: Option<DocView>,
    /// The page shown (0-based).
    pub page: usize,
    pub zoom: ZoomView,
    pub tool: ToolView,
    pub sidebar: PaneView,
    pub panel: PaneView,
    /// The notice bar: plain words, a kind, and buttons it offers.
    pub notice: Option<Notice>,
    /// A standing banner (the redacted copy, a signed file, a repaired file).
    pub banner: Option<Notice>,
    pub search: SearchView,
    /// Page overlays for the pages the UI asked about (shown page and its neighbours).
    pub overlays: Vec<PageOverlay>,
    pub comments: Vec<Row>,
    pub fields: Vec<Row>,
    pub outline: Vec<OutlineRow>,
    pub layers: Vec<Row>,
    pub redact: RedactView,
    /// The open dialog, if any: its id and the data it needs.
    pub dialog: Option<DialogView>,
    pub recent: Vec<RecentFile>,
    /// Long work in progress (opening a big file, redacting, signing).
    pub busy: Option<Busy>,
    /// A short success line shown as a toast, then cleared by the core.
    pub toast: Option<String>,
    /// Text for screen readers' live region (what just happened).
    pub announce: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocView {
    pub name: String,
    /// "Saved", "Edited", "Saving…", "Not saved yet", "Read-only".
    pub save_state: String,
    pub read_only: bool,
    /// Why it is read-only, in plain words, when it is.
    pub read_only_reason: Option<String>,
    pub page_count: usize,
    pub pages: Vec<PageView>,
    pub can_undo: bool,
    pub can_redo: bool,
    /// The undo/redo step names ("Undo Highlight"), when known.
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    pub can_save: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageView {
    /// Page size in points as shown (rotation applied).
    pub width: f64,
    pub height: f64,
    pub rotation: i32,
    /// The page label ("iv", "A-3"), or the page number.
    pub label: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoomView {
    /// "fit-width", "fit-page" or "fixed".
    pub mode: String,
    /// CSS pixels per PDF point.
    pub scale: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolView {
    /// "select", "highlight", "draw", "note", "text", "fill-sign", "redact", "pages".
    pub id: String,
    /// The armed tool's chip ("Drawing · Esc to stop"), or none for Select.
    pub chip: Option<String>,
    /// Sub-tool for Draw ("pen", "line", "rect", "ellipse") or Highlight
    /// ("highlight", "underline", "strike").
    pub variant: Option<String>,
    /// RGB 0–1 and stroke width for Draw/Highlight.
    pub color: Option<[f64; 3]>,
    pub width: Option<f64>,
    /// Whether the document lets this tool work (else its button is dimmed with a reason).
    pub available: Vec<ToolAvailability>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAvailability {
    pub id: String,
    pub enabled: bool,
    pub reason: Option<String>,
    /// A count shown on the tool (pending redaction boxes).
    pub badge: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneView {
    pub open: bool,
    /// Sidebar: "pages" or "outline". Panel: "comments", "fields",
    /// "properties", "redactions", "signatures", "links", "layers", "compare".
    pub tab: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    /// "error", "warning", "success", "info".
    pub kind: String,
    pub text: String,
    /// Buttons: (action id, label).
    pub actions: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchView {
    pub query: String,
    pub count: usize,
    /// -1 when there is no current hit.
    pub index: i64,
    pub truncated: bool,
    pub busy: bool,
}

/// What is drawn over one page, in PDF points (y down, origin top-left).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageOverlay {
    pub page: usize,
    pub targets: Vec<Target>,
    /// Search hits on this page (quads as rects); `current` marks the shown hit.
    pub hits: Vec<Rect>,
    pub current_hit: Option<usize>,
    /// The text selection's rectangles.
    pub selection: Vec<Rect>,
    /// Pending redaction boxes and the red "what goes" tint.
    pub redactions: Vec<Target>,
    pub tint: Vec<Rect>,
    /// An inline editor open on this page.
    pub editor: Option<EditorView>,
}

pub type Rect = [f64; 4];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    /// "text", "object", "field", "link", "note", "textbox", "signature", "redact", "annot".
    pub kind: String,
    pub id: String,
    pub rect: Rect,
    pub label: String,
    pub selected: bool,
    /// Whether it can be moved/resized (handles are drawn).
    pub movable: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorView {
    /// "field", "text", "paragraph", "add", "note", "textbox", "link".
    pub kind: String,
    pub rect: Rect,
    pub value: String,
    pub font_px: f64,
    pub multiline: bool,
    pub placeholder: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub id: String,
    pub page: Option<usize>,
    pub title: String,
    pub detail: String,
    pub selected: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineRow {
    pub id: String,
    pub title: String,
    pub page: Option<usize>,
    pub uri: Option<String>,
    pub depth: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactView {
    pub boxes: Vec<Row>,
    /// "Redact 3 areas…".
    pub apply_label: String,
    pub can_apply: bool,
    /// "Show what's underneath".
    pub underneath: bool,
    /// The plain note shown when a box will need a page flattened.
    pub flatten_note: Option<String>,
}

/// One element of a dialog or panel as the core holds it (the ported page's state).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeView {
    pub text: String,
    pub value: String,
    pub hidden: bool,
    pub disabled: bool,
    pub checked: bool,
    pub open: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogView {
    /// Which dialog ("password", "watermark", "redact-confirm", "sign", …).
    pub id: String,
    pub title: String,
    /// What the dialog needs (field values, choices, lists), per dialog.
    pub data: Value,
    /// Every element of the dialog by id (texts, values, hidden/disabled/checked).
    pub nodes: std::collections::BTreeMap<String, NodeView>,
    /// Rows for tables and lists inside the dialog (verify results, batch files, redaction pages).
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentFile {
    pub name: String,
    pub path: String,
    /// Plain words ("Today", "Yesterday", "12 Oct").
    pub when: String,
    pub folder: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Busy {
    pub text: String,
    /// 0–1 when known.
    pub progress: Option<f64>,
    pub can_cancel: bool,
}

/// One thing the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum UiAction {
    /// A drive op, as the menus send it (`{op, contents}`); contents is the
    /// op's JSON as in docs/agent.md, or null for "open its dialog".
    Edit {
        op: String,
        contents: Option<Value>,
    },
    /// Open: a path (recent file, drop, Finder) or null for the open panel.
    Open {
        path: Option<String>,
    },
    New {
        size: Option<String>,
    },
    /// "in-place", "as", "copy".
    Save {
        how: String,
    },
    Print,
    Tool {
        id: String,
        variant: Option<String>,
    },
    Goto {
        page: usize,
    },
    /// "in", "out", "actual", "fit-width", "fit-page", or a scale.
    Zoom {
        to: String,
        scale: Option<f64>,
    },
    /// The viewport's size in CSS pixels (fit zoom needs it).
    Viewport {
        width: f64,
        height: f64,
    },
    Search {
        query: String,
    },
    SearchMove {
        by: i64,
    },
    /// A pointer gesture on a page, in PDF points.
    Pointer {
        phase: String,
        page: usize,
        x: f64,
        y: f64,
        shift: bool,
        alt: bool,
        meta: bool,
    },
    /// A key the page area handles (Escape, Delete, arrows, Tab, Enter).
    Key {
        key: String,
        shift: bool,
        alt: bool,
        meta: bool,
    },
    /// Text typed into the open inline editor.
    EditorInput {
        value: String,
    },
    EditorCommit,
    /// A dialog's button with the dialog's form values.
    Dialog {
        id: String,
        button: String,
        values: Value,
    },
    Notice {
        action: String,
    },
    Pane {
        which: String,
        open: Option<bool>,
        tab: Option<String>,
    },
    /// Select a list row (comment, field, bookmark, redaction box).
    Row {
        list: String,
        id: String,
        action: String,
    },
    /// The pages the UI shows now (overlays are sent for these).
    Visible {
        pages: Vec<usize>,
    },
    /// A button (by its element id in the ported dialogs and panels).
    Click { id: String },
    /// A text field or select changed (by element id).
    Input { id: String, value: String },
    /// A checkbox or radio changed (by element id).
    Check { id: String, checked: bool },
    /// Escape or the close button on a dialog (by dialog id).
    DialogCancel { id: String },
    /// Strokes drawn on the signature pad (points in pad pixels).
    SignaturePad { strokes: Vec<Vec<[f64; 2]>>, width: f64, height: f64 },
}
