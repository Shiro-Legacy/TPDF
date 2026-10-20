//! The local types of renderer.ts (`R:n`), redact-ui.ts (`U:n`) and recovery.ts (`V:n`),
//! including the inline object types of renderer.ts's top-level `let`s, named after the
//! variable (`let compareDoc: {…}` → [`CompareDoc`]).
//!
//! Reply shapes come from `tandem_protocol::engine` (re-exported below where the
//! renderer names them). DOM references become plain data: what the logic or STATE reads.
//! Types that only carried callbacks or worker plumbing (`Pending`, `Pdf4AllApi`,
//! `RedactHost`, `RectGesture`'s closures, `ReplaySend`) are not here: they become methods.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, VecDeque};

pub use crate::messages::MessageKind;
pub use crate::messages::redact::PendingAction;
pub use tandem_protocol::engine::{
    AnnotItem, AuthLevel, BatesRange, CompareRenderReply, DocumentInfo, EditTarget, FieldItem,
    FieldValue, LayerItem, LinkEntry, OutlineEntry, PageSize, ParagraphItem, RedactInspectRegion,
    RedactPreview, RedactRegion, RenderReply, ReplaceMatch, ReplaceRejected, SearchHit,
    SecurityState, ThumbReply,
};

/// Declares a string-token enum with `as_str` / `parse` (the TS literal union).
macro_rules! token_enum {
    ($(#[$meta:meta])* pub enum $name:ident { $( $(#[$vmeta:meta])* $variant:ident = $token:literal, )+ }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $( $(#[$vmeta])* #[serde(rename = $token)] $variant, )+
        }
        impl $name {
            /// The TS token.
            pub fn as_str(self) -> &'static str {
                match self { $( Self::$variant => $token, )+ }
            }
            /// The variant for a TS token.
            pub fn parse(token: &str) -> Option<Self> {
                match token { $( $token => Some(Self::$variant), )+ _ => None }
            }
        }
    };
}

// ------------------------------------------------------------------ modes and view (R:196-198)

token_enum! {
    /// R:196.
    pub enum ZoomMode {
        #[default]
        FitWidth = "fit-width",
        FitPage = "fit-page",
        Fixed = "fixed",
    }
}

token_enum! {
    /// R:198: the chosen task: which context row is shown, and which inspector tab it belongs to.
    pub enum UiMode {
        #[default]
        Read = "read",
        Annotate = "annotate",
        Organize = "organize",
        Forms = "forms",
        Edit = "edit",
        Redact = "redact",
    }
}

// ------------------------------------------------------------------ main's contracts (R:108-127)

token_enum! {
    /// R:108 `OpenError.reason`.
    pub enum OpenErrorReason {
        #[default]
        TooLarge = "too-large",
        Unreadable = "unreadable",
        Cancelled = "cancelled",
    }
}

/// R:108: why main could not deliver a document.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OpenError {
    pub reason: OpenErrorReason,
    pub name: Option<String>,
    pub limit: Option<f64>,
}

/// R:111: main delivers the document (contract §1).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IncomingDocument {
    pub name: String,
    pub size: Option<f64>,
    pub url: Option<String>,
    pub untitled: Option<bool>,
}

/// R:114: main's save-result contract; `ok` true carries `name` (and maybe `warning`, `same_file`),
/// `ok` false a `reason` (worded in `messages::save::reason`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SaveResult {
    pub ok: bool,
    pub name: Option<String>,
    /// "source-changed" | "durability-uncertain".
    pub warning: Option<String>,
    pub same_file: Option<bool>,
    pub reason: Option<String>,
}

/// R:124: unit 5 print result; `reason` is "cancelled" | "busy" | "failed" | "no-printer".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PrintResult {
    pub ok: bool,
    pub reason: Option<String>,
}

// ------------------------------------------------------------------ the open document (R:767)

/// R:767. `untitled`: a File → New document, whose Save asks main for a path.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpenDoc {
    pub name: String,
    pub url: Option<String>,
    pub page_count: i64,
    pub pages: Vec<PageSize>,
    pub rotations: Vec<i64>,
    pub info: DocumentInfo,
    pub open_ms: f64,
    pub untitled: Option<bool>,
}

// ------------------------------------------------------------------ object menu, clipboard (R:291-299)

/// R:291 `objectMenuAt`: the page and zoom the open menu belongs to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ObjectMenuAt {
    pub page: i64,
    pub zoom: f64,
}

token_enum! {
    /// R:294 `objectMenuKind`.
    pub enum ObjectMenuKind {
        #[default]
        Selection = "selection",
        Page = "page",
    }
}

/// R:297 `clipboardInfo`: the worker clipboard's counts from the last `clip-copy`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ClipboardInfo {
    pub count: i64,
    pub kinds: BTreeMap<String, i64>,
}

// ------------------------------------------------------------------ compare (R:667-680)

/// R:667.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompareText {
    pub same: i64,
    pub removed: i64,
    pub added: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
    pub removed_lines: Vec<String>,
    pub added_lines: Vec<String>,
}

/// R:668.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ComparePixels {
    pub differing: i64,
    pub total: i64,
    pub ratio: f64,
    pub scale: f64,
}

/// R:670 `compareDoc`: the compare slot's renderer mirror.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompareDoc {
    pub name: String,
    pub page_count: i64,
    pub pages: Vec<PageSize>,
    pub page_a: i64,
    pub page_b: i64,
    pub text: Option<CompareText>,
    pub pixels: Option<ComparePixels>,
}

token_enum! {
    /// R:675 `compareLast.action`.
    pub enum CompareAction {
        #[default]
        Open = "open",
        Close = "close",
    }
}

/// R:675 `compareLast`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompareLast {
    pub action: CompareAction,
    pub ok: bool,
    pub detail: Option<String>,
}

/// R:680 `compareB`: B's last render, keyed `cmp:${pageB}:${S}` (pixels included).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompareB {
    pub key: String,
    pub reply: CompareRenderReply,
}

// ------------------------------------------------------------------ batch (R:694-708)

/// R:694: a batch's stamp action; `kind` is "watermark" | "header" | "footer", `align` "left" | "right".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BatchAction {
    pub kind: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bates_pad: Option<i64>,
}

token_enum! {
    /// R:695 `BatchRow.status`.
    pub enum BatchStatus {
        #[default]
        Queued = "queued",
        Running = "running",
        Ok = "ok",
        Failed = "failed",
    }
}

/// R:695.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BatchRow {
    pub name: String,
    pub status: BatchStatus,
    pub pages: i64,
    pub detail: String,
    pub bates: Option<BatesRange>,
}

/// R:700 `batchState.bates`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BatchBates {
    pub start: i64,
    pub next: i64,
    pub pad: i64,
}

/// R:700 `batchState`: the folder listing and the run, never reset by open/close.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BatchState {
    pub folder: Option<String>,
    pub out: Option<String>,
    pub action: Option<BatchAction>,
    pub files: Vec<BatchRow>,
    pub running: bool,
    pub cancelled: bool,
    pub report: Option<String>,
    pub bates: BatchBates,
}

/// R:708 `certSignField`: the open sign dialog's field mode.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CertSignField {
    pub name: String,
    pub page: i64,
}

// ------------------------------------------------------------------ nav, assemble, decorate (R:805-843)

/// R:805 `navLinkDraft`: the rect "Draw link" drew, while its dialog is open.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NavLinkDraft {
    pub page: i64,
    pub rect: Vec<f64>,
}

/// R:810 one entry of `navLinks[page]`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NavLinkItem {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

token_enum! {
    /// R:814 `linkMode`.
    pub enum LinkMode {
        #[default]
        Page = "page",
        Uri = "uri",
    }
}

/// R:818 `insertSource`: the PDF the insert dialog is about to bring pages in from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InsertSource {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// `{ action, ok, detail? }`: the `last` of the assemble summary and `compareLast`-like records.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LastAction {
    pub action: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// R:820 `assembleSummary`: what the last insert added; a refusal leaves only `last`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AssembleSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inserted: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_names: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<LastAction>,
}

/// R:822 `extractSummary`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExtractSummary {
    pub pages: Vec<i64>,
    pub bytes: i64,
}

/// R:824 `exportSummary`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExportSummary {
    pub kind: String,
    pub bytes: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<i64>,
}

/// R:833 `decorateSummary.stamps`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DecorateStamps {
    pub watermark: i64,
    pub header: i64,
}

/// R:833 one of `decorateSummary.layers`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DecorateLayer {
    pub name: String,
    pub visible: bool,
}

/// R:833 `decorateSummary.last`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DecorateLast {
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<Vec<i64>>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bates: Option<BatesRange>,
}

/// R:833 `decorateSummary`: what the last decorate smoke did, for `P2_EDIT.decorate`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DecorateSummary {
    pub stamps: DecorateStamps,
    pub labels: Vec<String>,
    pub layers: Vec<DecorateLayer>,
    pub layers_saved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<DecorateLast>,
}

/// R:842 `batesLast`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BatesLast {
    pub from: i64,
    pub to: i64,
    pub pages: Vec<i64>,
}

// ------------------------------------------------------------------ signatures (R:851-871)

/// A point in page points or pad CSS px (`[number, number]`).
pub type Point = [f64; 2];

/// R:853: the saved drawing (pad CSS px, bounds at the origin). `v` is always 1.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DrawnSig {
    pub v: i64,
    pub width: f64,
    pub height: f64,
    pub strokes: Vec<Vec<Point>>,
}

/// R:855: or an image (its PNG/JPEG bytes as base64, decoded pixel size); `kind` is "image".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ImageSig {
    pub v: i64,
    pub kind: String,
    pub width: f64,
    pub height: f64,
    pub data: String,
}

/// R:856 `savedSignature: DrawnSig | ImageSig` (main's `signatureGet`/`signatureSet` value).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SavedSignature {
    Image(ImageSig),
    Drawn(DrawnSig),
}

/// R:860 `fillSignLast` and R:3155 `fillLast`: STATE `fill.lastSignature` / `fill.last`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FillOp {
    pub op: String,
    pub id: i64,
    pub rect: Option<Vec<f64>>,
}

/// R:868 `padImage`: the image chosen with "Use an image…" (the decoded bitmap is the UI's).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PadImage {
    pub width: f64,
    pub height: f64,
    /// Base64 of the file's bytes.
    pub data: String,
}

// ------------------------------------------------------------------ sidebar (R:883)

/// R:883 one of `thumbItems` (a button and a canvas per page): what the logic reads of them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThumbItem {
    pub page: usize,
    /// `aria-current` on its button.
    pub current: bool,
    /// Its button's `tabIndex` is 0 (the roving tab stop).
    pub tab_stop: bool,
}

/// `createRenderCache` (engine-shared.ts): a bounded render cache, oldest first. Entry
/// count alone is not enough, so it also tracks the total pixel bytes. Methods live with
/// the part that ports the cache.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderCache<T> {
    pub entries: VecDeque<(String, T)>,
    pub bytes: usize,
    pub limit_entries: usize,
    pub limit_bytes: usize,
}

impl<T> RenderCache<T> {
    pub fn new(limit_entries: usize, limit_bytes: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
            limit_entries,
            limit_bytes,
        }
    }
}

// ------------------------------------------------------------------ drive records (R:910-935)

/// R:910 `annotLast`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AnnotLast {
    pub action: String,
    pub index: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// R:911 `navLast`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NavLast {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect: Option<Vec<f64>>,
}

/// `{ text, pixels, raw }`: what `redact-apply` checked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactVerifiedCounts {
    pub text: i64,
    pub pixels: i64,
    pub raw: i64,
}

/// R:913 `redactLast`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactLast {
    pub op: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified: Option<RedactVerifiedCounts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opened: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub areas: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flattened: Option<Vec<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsigned: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub over_redacted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
}

token_enum! {
    /// U:95 `PendingChoice`: the pending-boxes question's answer.
    pub enum PendingChoice {
        #[default]
        Redact = "redact",
        Marks = "marks",
        Cancel = "cancel",
    }
}

token_enum! {
    /// R:916 `redactPrompt.choice`: a [`PendingChoice`] or "unasked".
    pub enum PromptChoice {
        #[default]
        Unasked = "unasked",
        Redact = "redact",
        Marks = "marks",
        Cancel = "cancel",
    }
}

/// R:916 `redactPrompt`: the save prompt's last answer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RedactPrompt {
    pub marks: i64,
    pub choice: PromptChoice,
    pub action: PendingAction,
}

/// R:920 `redactAskAnswer`: a drive's pre-answers to the flatten and unsigned questions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactAskAnswer {
    pub flatten: Option<bool>,
    pub unsign: Option<bool>,
}

token_enum! {
    /// R:930 `redactOthersAnswer`: a drive's answer to "You have other unsaved changes."
    pub enum OthersAnswer {
        #[default]
        Save = "save",
        Continue = "continue",
        Cancel = "cancel",
    }
}

/// R:935 `signLast`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SignLast {
    pub op: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revisions: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

// ------------------------------------------------------------------ recovery, renderer side (R:1265-1309)

token_enum! {
    /// R:1265: why the worker was stopped.
    pub enum RestoreCause {
        #[default]
        Slow = "slow",
        Trap = "trap",
        Stopped = "stopped",
    }
}

/// R:1268: the request the worker was running when it hung or failed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Culprit {
    pub op: Value,
    pub page: Option<i64>,
}

token_enum! {
    /// R:3157 `fillSave`: the first Save after a fill writes a "(filled)" copy.
    pub enum FillSave {
        #[default]
        None = "none",
        NeedsCopy = "needs-copy",
        Copied = "copied",
    }
}

/// R:1271: what the window showed when the worker stopped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RestoreSnapshot {
    pub doc: OpenDoc,
    pub wanted_page: i64,
    pub auth: AuthLevel,
    pub security_state: Option<SecurityState>,
    pub fill_save: FillSave,
    pub layers_persisted: bool,
    pub clipboard_info: Option<ClipboardInfo>,
    pub dirty: bool,
    pub culprit: Option<Culprit>,
    pub cause: RestoreCause,
    /// Later changes that were posted but never answered: lost with the culprit.
    pub lost: i64,
}

token_enum! {
    /// R:1402.
    pub enum RestoreOutcome {
        #[default]
        Restored = "restored",
        Reopened = "reopened",
        Lost = "lost",
    }
}

/// R:1292 `restoreProgress`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RestoreProgress {
    pub done: i64,
    pub total: i64,
}

/// R:1305 `diskWrite`: a save in place whose bytes main is writing (or wrote) but whose
/// `saved` is not recorded yet. `done` replaces the TS promise: `None` while writing,
/// then whether it landed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiskWrite {
    pub done: Option<bool>,
    pub bytes: Vec<u8>,
    pub dialog: bool,
}

/// R:1309 `restoreLast`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RestoreLast {
    pub outcome: RestoreOutcome,
    pub dropped: Option<String>,
    pub cause: RestoreCause,
    pub lost: i64,
    pub steps: i64,
    pub over_budget: Option<bool>,
}

// ------------------------------------------------------------------ direct editing (R:3097-3245)

token_enum! {
    /// R:3097.
    pub enum SnapEdge {
        #[default]
        X0 = "x0",
        X1 = "x1",
        Cx = "cx",
        Y0 = "y0",
        Y1 = "y1",
        Cy = "cy",
    }
}

token_enum! {
    /// R:3098 `SnapGuide.axis`.
    pub enum Axis {
        #[default]
        X = "x",
        Y = "y",
    }
}

/// R:3098: an alignment guide (view space, y down).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SnapGuide {
    pub axis: Axis,
    pub at: f64,
}

/// R:3164: on-page focus of an object or a redaction mark; `rect` is the last view rect.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ObjectFocus {
    pub kind: String,
    pub name: String,
    pub target_id: String,
    pub rect: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// `number | string`: an id that is a field/annotation number or a content target id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TargetRef {
    Number(i64),
    Text(String),
}

impl Default for TargetRef {
    fn default() -> Self {
        TargetRef::Text(String::new())
    }
}

token_enum! {
    /// R:3206 `DirectGesture.kind`.
    pub enum GestureKind {
        #[default]
        Move = "move",
        Resize = "resize",
        Duplicate = "duplicate",
        Crop = "crop",
    }
}

/// R:3206: the live move/resize gesture (STATE reads it).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DirectGesture {
    pub kind: GestureKind,
    pub handle: String,
    pub target_id: TargetRef,
    pub rect: Vec<f64>,
    pub guides: Option<Vec<SnapGuide>>,
}

/// R:3995 one of `ObjectTargets` (the DOM node is dropped).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectTarget {
    pub member: ObjectFocus,
    pub item: EditTarget,
    pub rect: Vec<f64>,
}

token_enum! {
    /// R:3150 `directActive.kind`.
    pub enum DirectKind {
        #[default]
        Field = "field",
        Text = "text",
        Add = "add",
        Link = "link",
        Textbox = "textbox",
        Note = "note",
    }
}

/// R:3150 `directActive.item: FieldItem | EditTarget | LinkEntry`.
#[derive(Clone, Debug, PartialEq)]
pub enum DirectItem {
    Field(FieldItem),
    Target(EditTarget),
    Link(LinkEntry),
}

impl Default for DirectItem {
    fn default() -> Self {
        DirectItem::Target(EditTarget::default())
    }
}

token_enum! {
    /// The element an inline editor is.
    pub enum EditorTag {
        #[default]
        Input = "input",
        Textarea = "textarea",
        Select = "select",
    }
}

/// R:3150 `directActive.editor` (an `<input>`, `<textarea>` or `<select>` on the page): the
/// element replaced by what the logic and STATE read of it. Box geometry is CSS px (page
/// points × zoom, see `editorBox`); `font_size` is `dataset.fontSize` (page pt).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditorState {
    pub tag: EditorTag,
    /// `className` (e.g. "inline-editor text-inline-editor", "note-inline-editor").
    pub class_name: String,
    pub value: String,
    /// `maxLength`; -1 when unset (the DOM default).
    pub max_length: i64,
    pub placeholder: String,
    pub title: String,
    /// `dataset.fontSize`, `dataset.initial`, `dataset.minHeight`.
    pub font_size: Option<String>,
    pub initial: Option<String>,
    pub min_height: Option<String>,
    /// `style.left/top/width/height/fontSize/lineHeight` in CSS px.
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
    pub font_px: f64,
    pub line_height: Option<f64>,
    pub selection_start: Option<usize>,
    pub selection_end: Option<usize>,
    /// A `<select>`'s options (value, label).
    pub options: Vec<(String, String)>,
}

/// R:3150 `directActive`: the open inline editor. `option_list` (a combo's `<datalist>`)
/// becomes the list's option values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DirectActive {
    pub kind: DirectKind,
    pub item: DirectItem,
    pub editor: EditorState,
    pub page: i64,
    pub scan: Option<ContentScan>,
    pub paragraph: Option<ParagraphItem>,
    pub option_list: Option<Vec<String>>,
    pub point: Option<Point>,
    /// fill-anywhere D3/D4: the FreeText a `textbox` editor edits (None = a new box; the TS
    /// `undefined` and `null` are read alike).
    pub annot_id: Option<i64>,
}

/// R:3168 `directCrop`: crop mode on the single selected member.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DirectCrop {
    pub target_id: String,
    pub full: Vec<f64>,
}

/// R:3171/3173 `directFocusedMark` / `directFocusedLink`: `{ annotId }`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnnotRef {
    pub annot_id: i64,
}

// ------------------------------------------------------------------ existing content (R:5700-5721)

/// R:5702: one page's `edit-targets` scan. `transform` maps PDF user space to the y-down
/// page view; `doc_generation` is the renderer's open counter at scan time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContentScan {
    pub page: i64,
    pub generation: i64,
    pub revision: i64,
    pub items: Vec<EditTarget>,
    pub paragraphs: Vec<ParagraphItem>,
    pub transform: Option<Vec<f64>>,
    pub doc_generation: Option<i64>,
}

/// Round 2: the shown page whose `edit-targets` was refused (no targets on it). `reason` is
/// the engine's (`too-large`, `invalid`, `read-only`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContentPageRefusal {
    pub page: i64,
    pub generation: i64,
    pub reason: String,
}

/// R:5706 one of `contentRefusals` (and of `contentReplaced.rejected`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ContentRefusal {
    pub text: String,
    pub detail: String,
}

/// R:5707 `contentEdited`: a text edit, or an object action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContentEdited {
    Text {
        from: String,
        to: String,
    },
    Object {
        kind: String,
        action: String,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        count: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        skipped: Option<i64>,
    },
}

/// R:5709 `contentLastAdd`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ContentLastAdd {
    pub target_id: String,
    pub bounds: Option<Vec<f64>>,
}

/// R:5715 `replacePreview`: the last preview, its token and which matches are ticked.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReplacePreview {
    pub page: i64,
    pub generation: i64,
    pub revision: i64,
    pub find: String,
    pub replace: String,
    pub token: String,
    pub target_id: Option<String>,
    pub matches: Vec<ReplaceMatch>,
    pub rejected: Vec<ReplaceRejected>,
}

/// R:5720 `contentReplaced`: STATE `content.replaced`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ContentReplaced {
    pub find: String,
    pub replace: String,
    pub applied: i64,
    pub matches: i64,
    pub rejected: Vec<ContentRefusal>,
}

// ------------------------------------------------------------------ fields (R:7159-7563)

/// R:7159: the U6 drive path's parsed `contents`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FieldDesignWanted {
    pub action: Option<String>,
    pub kind: Option<String>,
    pub name: Option<String>,
    pub on: Option<String>,
    pub rename: Option<String>,
    pub rect: Option<Vec<f64>>,
    pub options: Option<Vec<String>>,
    pub kids: Option<Vec<FieldKid>>,
    pub max_len: Option<i64>,
    pub multiline: Option<bool>,
    pub edit: Option<bool>,
    pub value: Option<FieldValue>,
    pub label: Option<String>,
    pub read_only: Option<bool>,
    pub required: Option<bool>,
    pub default: Option<FieldValue>,
}

/// A radio kid: `{ on, rect }`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FieldKid {
    pub on: String,
    pub rect: Vec<f64>,
}

/// R:7563 `fieldDesign`: the last designer action (`P2_EDIT.fields.last`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FieldDesign {
    pub action: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

token_enum! {
    /// R:8540 `markupLast.source`.
    pub enum MarkupSource {
        #[default]
        Selection = "selection",
        Search = "search",
    }
}

/// R:8540 `markupLast`: STATE `markup`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MarkupLast {
    #[serde(rename = "type")]
    pub kind: String,
    pub page: i64,
    pub source: MarkupSource,
    pub quads: Vec<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

// ------------------------------------------------------------------ redact-ui.ts

/// U:56: one box in the "Redactions (N)" panel (`id` < 0 = a box held in memory).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactBox {
    pub page: i64,
    pub id: i64,
    pub rect: Vec<f64>,
    pub search: bool,
}

/// U:57.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RedactPageGroup {
    pub page: i64,
    pub boxes: Vec<RedactBox>,
}

token_enum! {
    /// U:102 `TintShape.kind`.
    pub enum TintKind {
        #[default]
        Glyph = "glyph",
        Image = "image",
        Shape = "shape",
        Annot = "annot",
    }
}

/// U:102: one tinted shape of the underneath view: a glyph quad (8 numbers) or a rect (4), y-down page pt.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TintShape {
    pub kind: TintKind,
    pub points: Vec<f64>,
}

/// U:226 `bannerState`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RedactBannerState {
    pub name: String,
    pub areas: i64,
    pub original: bool,
    pub over_redacted: bool,
}

/// U:228 `tint`: the last `redact-inspect` answer for the shown page's boxes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RedactTint {
    pub page: i64,
    pub key: String,
    pub shapes: Vec<TintShape>,
}

/// The closure state of `createRedactUi` (U:223-228), held by `Session::redact_ui`, plus the
/// two body classes its logic reads back (`underneath()` at U:475).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RedactUiState {
    /// U:223.
    pub active: bool,
    /// U:224.
    pub boxes: Vec<RedactBox>,
    /// U:225.
    pub list_seq: i64,
    /// U:226.
    pub banner_state: Option<RedactBannerState>,
    /// U:228.
    pub tint: Option<RedactTint>,
    /// `body.redact-underneath` (the "Show what's underneath" toggle; also `#redact-underneath` checked).
    pub underneath: bool,
    /// `body.redact-peek`: the pointer is down on the page while the tool is active.
    pub peek: bool,
}

// ------------------------------------------------------------------ recovery.ts

token_enum! {
    /// V:29: how a request touches the worker's session.
    pub enum OpKind {
        #[default]
        Base = "base",
        Mutate = "mutate",
        Unlock = "unlock",
        Clipboard = "clipboard",
        Read = "read",
        Compare = "compare",
    }
}

token_enum! {
    /// V:156: why recovery is off for the open document.
    pub enum Unavailable {
        #[default]
        Budget = "budget",
        Clipboard = "clipboard",
        Generation = "generation",
    }
}

/// V:159 `Entry`: one logged request. `capture`: for a `saved` with a ticket, how many
/// entries the log held when that save captured its state. The message's buffers ride in `inputs`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecoveryEntry {
    pub op: String,
    pub message: Map<String, Value>,
    pub inputs: Vec<Vec<u8>>,
    pub bytes: usize,
    pub reply: String,
    pub ms: f64,
    pub capture: Option<usize>,
}

/// V:175 `Kept`: what `keep` took from a request before it was posted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Kept {
    pub op: String,
    pub bytes: usize,
    pub message: Option<Map<String, Value>>,
    pub inputs: Vec<Vec<u8>>,
}

/// V:457.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReplayResult {
    /// The open answer (page geometry and info as opened).
    pub open: Map<String, Value>,
    /// The last answer that carried journal state, or the open answer.
    pub journal: Map<String, Value>,
    /// The last answer that carried page geometry, or the open answer.
    pub pages: Map<String, Value>,
    pub steps: usize,
}

/// A save ticket this log saw issued: the entry count at capture, and its restore epoch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capture {
    pub index: usize,
    pub epoch: u64,
}

/// V:241 `RecoveryLog`'s data: the open document's recovery state (methods live with the
/// part that ports recovery.ts).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecoveryLog {
    /// The bytes the document was opened from; None when none is tracked.
    pub base: Option<Vec<u8>>,
    pub name: String,
    /// The open answer's fingerprint, and how long the open took (ms).
    pub opened: String,
    pub opened_ms: f64,
    /// The bytes of the last acknowledged save in place, or None.
    pub saved: Option<Vec<u8>>,
    /// True once a save in place was acknowledged whose bytes are not kept.
    pub saved_lost: bool,
    pub entries: Vec<RecoveryEntry>,
    pub unavailable: Option<Unavailable>,
    /// The worker clipboard was last filled by a `clip-copy` in this log.
    pub clipboard_in_log: bool,
    pub entry_bytes: usize,
    pub captures: HashMap<String, Capture>,
    pub epoch: u64,
    pub budget: usize,
}

impl RecoveryLog {
    /// `new RecoveryLog(budget)`.
    pub fn new(budget: usize) -> Self {
        Self {
            budget,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip() {
        assert_eq!(UiMode::parse("forms"), Some(UiMode::Forms));
        assert_eq!(ZoomMode::default().as_str(), "fit-width");
        assert_eq!(
            serde_json::to_value(FillSave::NeedsCopy).unwrap(),
            "needs-copy"
        );
        assert_eq!(PromptChoice::parse("unasked"), Some(PromptChoice::Unasked));
    }

    #[test]
    fn saved_signature_parses_both_kinds() {
        let drawn: SavedSignature =
            serde_json::from_str(r#"{"v":1,"width":10,"height":5,"strokes":[[[0,0],[1,2]]]}"#)
                .unwrap();
        assert!(matches!(drawn, SavedSignature::Drawn(ref d) if d.strokes[0][1] == [1.0, 2.0]));
        let image: SavedSignature =
            serde_json::from_str(r#"{"v":1,"kind":"image","width":10,"height":5,"data":"AA=="}"#)
                .unwrap();
        assert!(matches!(image, SavedSignature::Image(ref i) if i.data == "AA=="));
    }

    #[test]
    fn content_edited_serialises_untagged() {
        let text = ContentEdited::Text {
            from: "a".into(),
            to: "b".into(),
        };
        assert_eq!(
            serde_json::to_value(&text).unwrap(),
            serde_json::json!({"from": "a", "to": "b"})
        );
        let wanted: FieldDesignWanted =
            serde_json::from_str(r#"{"action":"add","value":true,"maxLen":3}"#).unwrap();
        assert_eq!(wanted.value, Some(FieldValue::Bool(true)));
        assert_eq!(wanted.max_len, Some(3));
    }
}
