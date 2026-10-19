//! The document session (port of src/renderer/renderer.ts, with redact-ui.ts and recovery.ts).
//!
//! [`Session`] holds every top-level `let` of renderer.ts as a field with the same name in
//! snake_case (`/// R:n` is the TS line), plus the module state of redact-ui.ts
//! ([`Session::redact_ui`]) and recovery.ts ([`Session::recovery`]), and the top-level `const`
//! collections that are mutable state (`cache`, `thumbPainted`, `pausedReads`, …).
//! Each part of the port is an `impl Session { … }` block in its own file (see PORTING.md).
//!
//! Not kept, because the synchronous in-process engine has no use for them: the worker
//! plumbing (`worker`, `ready`, `nextId`, `failReady`, `failingWorker`, `driveQueue`), the
//! promise holders that only sequenced async work (`compareWork`, `restoreBarrier`,
//! `redactInspectWork`, `directClickWork`, `directFinishWork`, `directGestureWork`,
//! `directCommit`; each is settled by the time a method returns), the timer handle
//! `searchTimer`, the guards `openGuard`/`workerGate` and the `escapeOwners` listener set.
//! Their STATE-visible effects stay (`worker_restarts`, `worker_stopped`, `restore_running`).

// The fields and tables are filled in by the porting lanes; until each part lands, many are
// read by nothing yet.
#![allow(dead_code)]

pub mod consts;
pub mod dom_ids;
pub mod types;

// lane 1 modules (renderer 1–2768)
mod documents;
mod drive_ops;
pub mod opening;
mod password;
mod recovery;
pub mod recovery_log;
pub mod shell;
mod start_screen;
pub mod worker;
pub mod zoom;
// end lane 1 modules
// lane 2 modules (renderer 2768–5347)
pub mod clipboard;
pub mod direct;
pub mod drive_direct;
pub mod editors;
pub mod fill;
pub mod hit;
pub mod members;
pub mod nav;
pub mod object_menu;
// end lane 2 modules
// lane 3 modules (renderer 5347–7288, redact-ui.ts)
mod content;
mod modes;
pub(crate) mod pages;
pub mod redact_ui;
mod redaction;
mod search;
mod thumbs;
// end lane 3 modules
// lane 4 modules (renderer 7288–9533)
mod assemble;
pub(crate) mod bookmarks_links;
mod compare;
mod exports;
mod field_designer;
mod form_fields;
pub(crate) mod markup;
pub(crate) mod shapes_stamps;
// end lane 4 modules
// lane 5 modules (renderer 9533–end)
mod batch;
mod cert_sign;
pub mod controls;
mod decorate;
mod editable_copy;
mod events;
mod notices;
mod print;
mod save;
mod security;
pub mod state;
pub mod verify;
// end lane 5 modules

use crate::messages::MessageKind;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use tandem_protocol::engine::{
    AnnotItem, AuthLevel, BatesRange, EditTarget, FieldItem, LayerItem, LinkEntry, OutlineEntry,
    RedactPreview, RedactRegion, RenderReply, SearchHit, SecurityState, ThumbReply,
};
use tandem_protocol::limits::{THUMB_CACHE_BYTES, THUMB_CACHE_ENTRIES};
use types::*;

/// Whether `TANDEM_STRICT_REPLIES=1` is set (read once).
fn strict_replies() -> bool {
    static STRICT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *STRICT.get_or_init(|| std::env::var("TANDEM_STRICT_REPLIES").is_ok_and(|v| v == "1"))
}

/// An engine reply (or a part of one) as its typed shape, `None` when it does not match.
/// A mismatch would otherwise drop the whole reply silently, so with `TANDEM_STRICT_REPLIES=1`
/// it panics, naming the type and serde's error, for the test suites to find.
pub(crate) fn parse_reply<T: serde::de::DeserializeOwned>(value: &Value) -> Option<T> {
    match T::deserialize(value) {
        Ok(parsed) => Some(parsed),
        Err(err) if strict_replies() => {
            let mut shown = value.to_string();
            if shown.len() > 400 {
                let mut end = 400;
                while !shown.is_char_boundary(end) {
                    end -= 1;
                }
                shown.truncate(end);
                shown.push('…');
            }
            panic!(
                "reply does not parse as {}: {err}; reply: {shown}",
                std::any::type_name::<T>()
            )
        }
        Err(_) => None,
    }
}

/// [`parse_reply`], with the type's default for a reply that does not match.
pub(crate) fn parse_reply_or_default<T: serde::de::DeserializeOwned + Default>(value: &Value) -> T {
    parse_reply(value).unwrap_or_default()
}

/// The editor: every piece of state the Electron renderer kept.
pub struct Session {
    /// The PDF engine, in process.
    pub engine: crate::engine::EngineHost,
    /// The page's observable element state, keyed by index.html id (see `dom_ids`).
    pub dom: crate::dom::Dom,
    /// What Electron's main process did (files, dialogs, saving, printing, stores).
    pub host: crate::host::Host,
    /// What the Mac window adds: the tool strip's tool, panes, drags (crate::ui).
    pub ui: crate::ui::UiExtra,
    /// D8: unsaved edits written to the app's data folder (crate::autosave).
    pub autosave: crate::autosave::Autosave,
    /// R:429 `HEADLESS` (a const from `<meta name="pdf4all-headless">`): a smoke/drive window
    /// shows no in-page prompt unless a step pre-answers it. The drive sets it.
    pub headless: bool,

    // ---------------------------------------------------------------- object menu, clipboard
    /// R:291: the page and zoom the open object menu belongs to.
    pub object_menu_at: Option<ObjectMenuAt>,
    /// R:292: the press that dismissed the menu (an `Event` in TS): true until its click is seen.
    pub object_menu_dismiss: bool,
    /// R:294: what the open menu belongs to.
    pub object_menu_kind: ObjectMenuKind,
    /// R:295: the page menu's view point (y-down page pt; None = keyboard, no `at`).
    pub object_menu_point: Option<Vec<f64>>,
    /// R:297: the worker clipboard's counts from the last `clip-copy`.
    pub clipboard_info: Option<ClipboardInfo>,
    /// R:299: the drive's `menu` page point while its synthetic contextmenu is dispatched.
    pub drive_menu_point: Option<Vec<f64>>,
    /// R:504: a notice to show once the document that is opening has opened (D6).
    pub pending_open_notice: Option<String>,
    /// R:552: the target whose form is open, if any.
    pub content_target: Option<EditTarget>,
    /// R:554: the Image/Form target whose placement form is open, if any.
    pub content_object_target: Option<EditTarget>,
    /// R:601: the chosen task. Read is the safe start: it arms nothing.
    pub ui_mode: UiMode,
    /// R:602.
    pub inspector_open: bool,
    /// R:652.
    pub verify_last: Option<Map<String, Value>>,

    // ---------------------------------------------------------------- compare, batch, cert sign
    /// R:670: the compare slot's renderer mirror.
    pub compare_doc: Option<CompareDoc>,
    /// R:674.
    pub compare_show_diff: bool,
    /// R:675.
    pub compare_last: Option<CompareLast>,
    /// R:677: a newer refresh drops the older one's replies.
    pub compare_gen: i64,
    /// R:680: B's last render, keyed `cmp:${pageB}:${S}` (never in A's cache).
    pub compare_b: Option<CompareB>,
    /// R:700: the batch folder listing and run, never reset by open/close.
    pub batch_state: Option<BatchState>,
    /// R:704.
    pub batch_cancel_after: Option<i64>,
    /// R:705.
    pub cert_sign_path: Option<String>,
    /// R:706.
    pub cert_sign_in_flight: bool,
    /// R:708: the open sign dialog's field mode; None = page/rect mode.
    pub cert_sign_field: Option<CertSignField>,
    /// R:710 (const Map): the last Verify row status per signed field name.
    pub verify_by_field: BTreeMap<String, String>,

    // ---------------------------------------------------------------- worker and document
    /// R:754: the document generation (opens and closes since start).
    pub generation: i64,
    /// R:755.
    pub p0_failed: bool,
    /// R:756.
    pub worker_restarts: i64,
    /// R:757.
    pub worker_stopped: bool,
    /// R:760.
    pub saving: bool,
    /// R:761.
    pub unlocking: bool,
    /// R:763: one print job at a time.
    pub printing: bool,
    /// R:764: cancels the print loop.
    pub print_token: i64,
    /// R:768.
    pub doc: Option<OpenDoc>,
    /// R:769.
    pub wanted_page: i64,
    /// R:770.
    pub shown_page: i64,
    /// R:771.
    pub shown_key: Option<String>,
    /// R:772.
    pub shown_zoom: f64,
    /// R:773.
    pub in_flight: Option<String>,
    /// R:774 (pixels included when the render kept them).
    pub last_result: Option<RenderReply>,
    /// R:878 (const): the page render cache.
    pub cache: RenderCache<RenderReply>,

    // ---------------------------------------------------------------- edit state
    /// R:778: the dirty/undo slice every mutation, undo, redo and saved reply carries.
    pub revision: i64,
    /// R:779.
    pub dirty: bool,
    /// R:780.
    pub can_undo: bool,
    /// R:781.
    pub can_redo: bool,
    /// R:783: how the session proved access (mirrors the worker).
    pub auth: AuthLevel,
    /// R:785: what the next save will write, as the worker reports it.
    pub security_state: Option<SecurityState>,
    /// R:789: notes and other annotations on the shown page.
    pub notes: Vec<AnnotItem>,
    /// R:790.
    pub notes_seq: i64,
    /// R:792: the fillable fields on the shown page.
    pub fields: Vec<FieldItem>,
    /// R:793.
    pub fields_seq: i64,
    /// R:794.
    pub fields_page: i64,
    /// R:796: the document's bookmarks.
    pub outline: Vec<OutlineEntry>,
    /// R:797.
    pub outline_seq: i64,
    /// R:798: the shown page's links.
    pub links: Vec<LinkEntry>,
    /// R:799.
    pub links_seq: i64,
    /// R:801: the page `links` holds.
    pub links_page: i64,
    /// R:803: the shown page's `/Tabs` name from the `fields` reply.
    pub fields_tabs: Option<String>,
    /// R:805: the rect "Draw link" drew, while its dialog is open.
    pub nav_link_draft: Option<NavLinkDraft>,
    /// R:810: the smoke's `P2_EDIT.nav.links` slice, keyed by page index.
    pub nav_links: BTreeMap<i64, Vec<NavLinkItem>>,
    /// R:812: the rect the next link is anchored to.
    pub link_anchor: Option<Vec<f64>>,
    /// R:814: which dialog the one link form is in.
    pub link_mode: LinkMode,
    /// R:816: the bookmark being renamed, or None when the dialog is adding one.
    pub editing_bookmark: Option<i64>,
    /// R:818: the PDF the insert dialog is about to bring pages in from.
    pub insert_source: Option<InsertSource>,
    /// R:820.
    pub assemble_summary: Option<AssembleSummary>,
    /// R:822.
    pub extract_summary: Option<ExtractSummary>,
    /// R:824.
    pub export_summary: Option<ExportSummary>,
    /// R:826: `labels[i]` is page i's label.
    pub labels: Vec<String>,
    /// R:827.
    pub labels_seq: i64,
    /// R:829.
    pub layers: Vec<LayerItem>,
    /// R:830.
    pub layers_seq: i64,
    /// R:831.
    pub layers_persisted: bool,
    /// R:833.
    pub decorate_summary: Option<DecorateSummary>,
    /// R:840: Bates counter, spans every document opened in this window.
    pub bates_next: i64,
    /// R:841.
    pub bates_pad: i64,
    /// R:842.
    pub bates_last: Option<BatesLast>,
    /// R:843: the range the latest stamp request produced.
    pub stamp_bates: Option<BatesRange>,

    // ---------------------------------------------------------------- ink and signatures
    /// R:851: the ink stroke a drag is recording, in page points.
    pub ink_stroke: Option<Vec<Point>>,
    /// R:856: the saved drawn or image signature.
    pub saved_signature: Option<SavedSignature>,
    /// R:858.
    pub sign_armed: bool,
    /// R:859.
    pub direct_focused_sig: Option<i64>,
    /// R:860: STATE `fill.lastSignature`.
    pub fill_sign_last: Option<FillOp>,
    /// R:862: the pad's finished strokes, in pad CSS px.
    pub pad_strokes: Vec<Vec<Point>>,
    /// R:863: the pad stroke being drawn.
    pub pad_stroke: Option<Vec<Point>>,
    /// R:865.
    pub pad_then_arm: bool,
    /// R:866.
    pub pad_full: bool,
    /// "Save" in the close question saved the document: the window may close (open#6).
    pub close_ready: bool,
    /// R:868: the image chosen with "Use an image…" (its bitmap is the UI's).
    pub pad_image: Option<PadImage>,
    /// R:869.
    pub editing_note: Option<i64>,
    /// R:871: a pending text-box add.
    pub box_point: Option<Point>,

    // ---------------------------------------------------------------- view and sidebar
    /// R:876.
    pub zoom_mode: ZoomMode,
    /// R:877.
    pub fixed_zoom: f64,
    /// R:883: one per page in the sidebar (DOM nodes replaced by what logic reads).
    pub thumb_items: Vec<ThumbItem>,
    /// R:884.
    pub thumb_open: bool,
    /// R:886.
    pub sidebar_open: bool,
    /// R:888.
    pub thumb_busy: bool,
    /// R:889.
    pub thumb_current: i64,
    /// R:890.
    pub thumb_tab: i64,
    /// R:891.
    pub thumb_rendered: i64,
    /// R:892.
    pub thumb_ms: f64,
    /// R:893.
    pub thumb_smoke_done: bool,
    /// R:894 (const Set).
    pub thumb_painted: BTreeSet<i64>,
    /// R:896 (const): thumbnails have their own cache.
    pub thumb_cache: RenderCache<ThumbReply>,
    /// R:898 (const Set): pages whose thumbnail already failed.
    pub thumb_tried: BTreeSet<i64>,

    // ---------------------------------------------------------------- search and drive records
    /// R:900.
    pub search_hits: Vec<SearchHit>,
    /// R:901.
    pub search_index: i64,
    /// R:902.
    pub search_seq: i64,
    /// R:904.
    pub search_query: String,
    /// R:905.
    pub search_truncated: bool,
    /// R:909: fetched only on the drive path.
    pub page_ids: Option<Vec<i64>>,
    /// R:910.
    pub annot_last: Option<AnnotLast>,
    /// R:911.
    pub nav_last: Option<NavLast>,
    /// R:912.
    pub redact_state: Option<RedactPreview>,
    /// R:913.
    pub redact_last: Option<RedactLast>,
    /// R:915: the mode Escape returns to.
    pub redact_return_mode: UiMode,
    /// R:916.
    pub redact_prompt: Option<RedactPrompt>,
    /// R:917.
    pub redact_prompt_answer: Option<PendingChoice>,
    /// R:918.
    pub redact_marks_acknowledged: i64,
    /// R:920: a drive's pre-answers to the flatten and unsigned questions.
    pub redact_ask_answer: Option<RedactAskAnswer>,
    /// R:925: a signed file's boxes, held in memory (R6/D5).
    pub redact_memory: Vec<RedactRegion>,
    /// R:926.
    pub redact_inspect_key: String,
    /// R:928 (const Set): Redact marks deleted since the last save (`page:annotId`).
    pub redact_deletes: BTreeSet<String>,
    /// R:930.
    pub redact_others_answer: Option<OthersAnswer>,
    /// R:932: the last pending-boxes count main was told.
    pub redact_pending_sent: String,
    /// R:935.
    pub sign_last: Option<SignLast>,
    /// R:995 (const `redactUi`): the redaction tool's own state (redact-ui.ts).
    pub redact_ui: RedactUiState,

    // ---------------------------------------------------------------- recovery (M9)
    /// R:1287 (const): the recovery log for the open document.
    pub recovery: RecoveryLog,
    /// R:1290.
    pub restore_running: bool,
    /// R:1292.
    pub restore_progress: Option<RestoreProgress>,
    /// R:1294.
    pub restore_dirty: bool,
    /// R:1296: what syncDirty last told main.
    pub reported_dirty: bool,
    /// R:1300: once the restart cap stopped the worker, this notice is the last word.
    pub stopped_notice: Option<String>,
    /// R:1305.
    pub disk_write: Option<DiskWrite>,
    /// R:1307.
    pub fallback_generation: i64,
    /// R:1309.
    pub restore_last: Option<RestoreLast>,
    /// R:1327 (const Set): paused read keys (`op` or `op:page`), in insertion order.
    pub paused_reads: Vec<String>,
    /// R:2590: the notice text the user closed.
    pub banner_dismissed: Option<String>,

    // ---------------------------------------------------------------- direct editing
    /// R:3150: the open inline editor.
    pub direct_active: Option<DirectActive>,
    /// R:3154.
    pub direct_focused_box: Option<i64>,
    /// R:3155: STATE `fill.last`.
    pub fill_last: Option<FillOp>,
    /// R:3157.
    pub fill_save: FillSave,
    /// R:3163.
    pub direct_focused_object: Option<ObjectFocus>,
    /// R:3166: the object selection in click order; the last member is the anchor.
    pub direct_selection: Vec<ObjectFocus>,
    /// R:3168.
    pub direct_crop: Option<DirectCrop>,
    /// R:3170: the press that ended crop mode (an `Event` in TS): true until its click is seen.
    pub crop_dismiss: bool,
    /// R:3171.
    pub direct_focused_mark: Option<AnnotRef>,
    /// R:3173.
    pub direct_focused_link: Option<AnnotRef>,
    /// R:3175.
    pub add_blocked: bool,
    /// R:3179.
    pub direct_field_index: i64,
    /// R:3180.
    pub direct_focused_field: Option<FieldItem>,
    /// R:3181.
    pub direct_focused_page: i64,
    /// R:3182: localStorage "field-highlight" !== "false" (the host persists it).
    pub highlight_fields: bool,
    /// R:3195: localStorage "edit-outlines" !== "false".
    pub show_outlines: bool,
    /// The renderer's `localStorage` (memory only in a smoke/drive run, else
    /// `<user data>/ui-prefs.json`).
    pub local_storage: BTreeMap<String, String>,
    /// R:3207.
    pub direct_gesture: Option<DirectGesture>,
    /// R:5160.
    pub direct_committing: bool,
    /// R:5296: the lines and zoom the text layer was last built for.
    pub painted_text_key: String,

    // ---------------------------------------------------------------- existing content
    /// R:5703.
    pub content_scan: Option<ContentScan>,
    /// R:5704.
    pub content_seq: i64,
    /// R:5705.
    pub content_index: i64,
    /// R:5706.
    pub content_refusals: Vec<ContentRefusal>,
    /// R:5707.
    pub content_edited: Option<ContentEdited>,
    /// R:5708.
    pub content_last_shift: f64,
    /// R:5709.
    pub content_last_add: Option<ContentLastAdd>,
    /// R:5710.
    pub content_add_point: Option<Point>,
    /// R:5715.
    pub replace_preview: Option<ReplacePreview>,
    /// R:5720.
    pub content_replaced: Option<ContentReplaced>,
    /// R:5721.
    pub content_smoke_active: bool,
    /// R:6742: one apply at a time.
    pub redact_applying: bool,
    /// R:7563.
    pub field_design: Option<FieldDesign>,
    /// R:8413.
    pub notes_need_text: bool,
    /// R:8540.
    pub markup_last: Option<MarkupLast>,
    /// R:9851.
    pub last_save: Option<Map<String, Value>>,
    /// R:9852.
    pub last_print: Option<Map<String, Value>>,
    /// R:11444.
    pub drop_depth: i64,
    /// R:11445.
    pub drop_busy: bool,
    /// R:11780: the notice text as given (STATE `error`).
    pub notice_raw: String,
    /// R:11781.
    pub notice_kind: MessageKind,
    // lane 1 state
    /// Lane 1's state: the console-line outbox, the viewport size, deferred engine restarts.
    pub lane1: shell::Lane1State,
    // end lane 1 state
    // lane 2 state
    /// The hit layer, focus, live gestures and the inline editor's identity (renderer 2768–5347).
    pub lane2: hit::Lane2State,
    // end lane 2 state
    // lane 3 state
    /// Lane 3's state the renderer kept in the DOM (thumbnail viewport and bitmaps, …).
    pub lane3: thumbs::Lane3State,
    // end lane 3 state
    // lane 4 state
    /// Lane 4's own state (selection, area drag, stamp icon, compare paint).
    pub lane4: markup::Lane4State,
    // end lane 4 state
    // lane 5 state
    /// Lane 5's state the struct lacks (see [`state::Lane5State`]).
    pub lane5: state::Lane5State,
    // end lane 5 state
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// A session over a memory-only smoke host with no pre-answers (unit tests).
    pub fn new() -> Self {
        let user_data =
            std::env::temp_dir().join(format!("tandem-session-test-{}", std::process::id()));
        let config = crate::host::HostConfig {
            user_data,
            smoke: true,
            echo_stdout: false,
        };
        let platform = crate::platform::Scripted::new(crate::platform::Answers::default());
        Self::with_host(crate::host::Host::new(config, Box::new(platform)))
    }

    /// A window with no document: the TS initial values, and the DOM as index.html and the
    /// scripts' top-level code leave it.
    pub fn with_host(host: crate::host::Host) -> Self {
        // The worker's CJK font (engine-rust.ts `useCjkFont`): read on demand, never up front.
        let mut engine = crate::engine::EngineHost::new();
        if let Some(path) = crate::host::cjk_font_path() {
            engine.use_cjk_font(path);
        }
        let mut session = Session {
            engine,
            dom: crate::dom::Dom::new(),
            host,
            ui: crate::ui::UiExtra::new(),
            autosave: crate::autosave::Autosave::default(),
            headless: false,

            object_menu_at: None,
            object_menu_dismiss: false,
            object_menu_kind: ObjectMenuKind::Selection,
            object_menu_point: None,
            clipboard_info: None,
            drive_menu_point: None,
            pending_open_notice: None,
            content_target: None,
            content_object_target: None,
            ui_mode: UiMode::Read,
            inspector_open: true,
            verify_last: None,

            compare_doc: None,
            compare_show_diff: false,
            compare_last: None,
            compare_gen: 0,
            compare_b: None,
            batch_state: None,
            batch_cancel_after: None,
            cert_sign_path: None,
            cert_sign_in_flight: false,
            cert_sign_field: None,
            verify_by_field: BTreeMap::new(),

            generation: 0,
            p0_failed: false,
            worker_restarts: 0,
            worker_stopped: false,
            saving: false,
            unlocking: false,
            printing: false,
            print_token: 0,
            doc: None,
            wanted_page: 0,
            shown_page: -1,
            shown_key: None,
            shown_zoom: 0.0,
            in_flight: None,
            last_result: None,
            cache: RenderCache::new(consts::CACHE_LIMIT, consts::CACHE_BYTES),

            revision: 0,
            dirty: false,
            can_undo: false,
            can_redo: false,
            auth: AuthLevel::None,
            security_state: None,
            notes: Vec::new(),
            notes_seq: 0,
            fields: Vec::new(),
            fields_seq: 0,
            fields_page: -1,
            outline: Vec::new(),
            outline_seq: 0,
            links: Vec::new(),
            links_seq: 0,
            links_page: -1,
            fields_tabs: None,
            nav_link_draft: None,
            nav_links: BTreeMap::new(),
            link_anchor: None,
            link_mode: LinkMode::Page,
            editing_bookmark: None,
            insert_source: None,
            assemble_summary: None,
            extract_summary: None,
            export_summary: None,
            labels: Vec::new(),
            labels_seq: 0,
            layers: Vec::new(),
            layers_seq: 0,
            layers_persisted: false,
            decorate_summary: None,
            bates_next: 1,
            bates_pad: 6,
            bates_last: None,
            stamp_bates: None,

            ink_stroke: None,
            saved_signature: None,
            sign_armed: false,
            direct_focused_sig: None,
            fill_sign_last: None,
            pad_strokes: Vec::new(),
            pad_stroke: None,
            pad_then_arm: false,
            pad_full: false,
            close_ready: false,
            pad_image: None,
            editing_note: None,
            box_point: None,

            zoom_mode: ZoomMode::FitWidth,
            fixed_zoom: 1.0,
            thumb_items: Vec::new(),
            thumb_open: false,
            sidebar_open: true,
            thumb_busy: false,
            thumb_current: -1,
            thumb_tab: -1,
            thumb_rendered: 0,
            thumb_ms: 0.0,
            thumb_smoke_done: false,
            thumb_painted: BTreeSet::new(),
            thumb_cache: RenderCache::new(THUMB_CACHE_ENTRIES, THUMB_CACHE_BYTES),
            thumb_tried: BTreeSet::new(),

            search_hits: Vec::new(),
            search_index: -1,
            search_seq: 0,
            search_query: String::new(),
            search_truncated: false,
            page_ids: None,
            annot_last: None,
            nav_last: None,
            redact_state: None,
            redact_last: None,
            redact_return_mode: UiMode::Read,
            redact_prompt: None,
            redact_prompt_answer: None,
            redact_marks_acknowledged: -1,
            redact_ask_answer: None,
            redact_memory: Vec::new(),
            redact_inspect_key: String::new(),
            redact_deletes: BTreeSet::new(),
            redact_others_answer: None,
            redact_pending_sent: String::new(),
            sign_last: None,
            redact_ui: RedactUiState::default(),

            // `recoveryBudgetFor({deviceMemoryGiB})`: Chromium caps deviceMemory at 8, so a
            // desktop with more than 4 GiB takes the full budget.
            recovery: RecoveryLog::new(consts::RECOVERY_BUDGET_BYTES),
            restore_running: false,
            restore_progress: None,
            restore_dirty: false,
            reported_dirty: false,
            stopped_notice: None,
            disk_write: None,
            fallback_generation: -1,
            restore_last: None,
            paused_reads: Vec::new(),
            banner_dismissed: None,

            direct_active: None,
            direct_focused_box: None,
            fill_last: None,
            fill_save: FillSave::None,
            direct_focused_object: None,
            direct_selection: Vec::new(),
            direct_crop: None,
            crop_dismiss: false,
            direct_focused_mark: None,
            direct_focused_link: None,
            add_blocked: false,
            direct_field_index: -1,
            direct_focused_field: None,
            direct_focused_page: -1,
            highlight_fields: true,
            show_outlines: true,
            local_storage: BTreeMap::new(),
            direct_gesture: None,
            direct_committing: false,
            painted_text_key: String::new(),

            content_scan: None,
            content_seq: 0,
            content_index: -1,
            content_refusals: Vec::new(),
            content_edited: None,
            content_last_shift: 0.0,
            content_last_add: None,
            content_add_point: None,
            replace_preview: None,
            content_replaced: None,
            content_smoke_active: false,
            redact_applying: false,
            field_design: None,
            notes_need_text: false,
            markup_last: None,
            last_save: None,
            last_print: None,
            drop_depth: 0,
            drop_busy: false,
            notice_raw: String::new(),
            notice_kind: MessageKind::Error,
            // lane 1 state
            lane1: shell::Lane1State::default(),
            // end lane 1 state
            // lane 2 state
            lane2: hit::Lane2State::default(),
            // end lane 2 state
            // lane 3 state
            lane3: thumbs::Lane3State::default(),
            // end lane 3 state
            // lane 4 state
            lane4: markup::Lane4State::default(),
            // end lane 4 state
            // lane 5 state
            lane5: state::Lane5State::default(),
            // end lane 5 state
        };
        dom_ids::init_dom(&mut session.dom);
        // R:3182, R:3195: the persisted toggles (`getItem(key) !== "false"`).
        session.local_storage = session.local_storage_load();
        session.highlight_fields =
            session.local_storage_get("field-highlight").as_deref() != Some("false");
        session.show_outlines =
            session.local_storage_get("edit-outlines").as_deref() != Some("false");
        session.load_effects();
        // R:1030 `wireControls()`: its load-time effects (lane 5, controls.rs).
        session.wire_controls();
        session
    }

    /// A `--smoke` / `--drive` window (main.ts M:713 sets the `pdf4all-headless` meta).
    pub fn for_drive(host: crate::host::Host) -> Self {
        let mut session = Self::with_host(host);
        session.headless = true;
        session
    }

    /// What the scripts' top-level code writes into the page at load, before any event:
    /// the two persisted toggles (R:3183, R:3196) and createRedactUi's fixed words (U:215-221).
    fn load_effects(&mut self) {
        use crate::messages::redact;
        self.dom
            .set_checked(dom_ids::FIELD_HIGHLIGHT, self.highlight_fields);
        self.dom
            .set_checked(dom_ids::EDIT_OUTLINES_INPUT, self.show_outlines);
        self.dom
            .set_text(dom_ids::redact_ui::CHIP, redact::REDACT_CHIP);
        self.dom
            .set_text(dom_ids::redact_ui::LIMIT_EL, redact::size_limit_text(None));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_like_the_renderer() {
        let session = Session::new();
        assert_eq!(session.shown_page, -1);
        assert_eq!(session.bates_pad, 6);
        assert_eq!(session.ui_mode, UiMode::Read);
        assert!(session.inspector_open && session.sidebar_open);
        assert!(session.dom.disabled(dom_ids::SAVE_BUTTON));
        assert!(session.dom.checked(dom_ids::FIELD_HIGHLIGHT));
        assert_eq!(
            session.dom.text(dom_ids::redact_ui::CHIP),
            crate::messages::redact::REDACT_CHIP
        );
        assert_eq!(session.cache.limit_entries, consts::CACHE_LIMIT);
    }
}
