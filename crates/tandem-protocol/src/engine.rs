//! Serde types for the engine's requests and replies.
//!
//! Mirrors `src/renderer/protocol.ts`, plus the reply shapes `renderer.ts`
//! types locally (`edit-targets`, `edit-replace-*`, `text-add`, `clip-*`,
//! `stamp`, `batch-file`, `sign-*`, ...). Every struct parses from `{}`
//! (container `#[serde(default)]`), and every string-literal union has an
//! `Unknown` catch-all, so an unexpected or missing key never fails parsing.
//!
//! Binary fields (`ArrayBuffer`) travel as `{"$buffer": n}` markers and are
//! `#[serde(skip)]` here: fill them from `Reply::take_bytes` after parsing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

// ------------------------------------------------------------------ enums

/// Defines a wire-token enum: explicit tokens, a `#[default]` passed in, and a
/// trailing `Unknown` catch-all (`#[serde(other)]`, wire token `"unknown"`).
macro_rules! wire_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident = $token:literal, )+
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name {
            $( $(#[$vmeta])* #[serde(rename = $token)] $variant, )+
            /// A token this build does not know (wire token `"unknown"`).
            #[serde(rename = "unknown")]
            #[serde(other)]
            Unknown,
        }

        impl $name {
            /// The wire token (`Unknown` → `"unknown"`).
            pub fn as_str(&self) -> &'static str {
                match self {
                    $( Self::$variant => $token, )+
                    Self::Unknown => "unknown",
                }
            }

            /// Parses a wire token; anything else is `Unknown`.
            pub fn from_token(token: &str) -> Self {
                match token {
                    $( $token => Self::$variant, )+
                    _ => Self::Unknown,
                }
            }
        }
    };
}

wire_enum! {
    /// Where `stamp` writes: the stamp kinds the app can write.
    pub enum StampKind {
        #[default]
        Watermark = "watermark",
        Header = "header",
        Footer = "footer",
    }
}

wire_enum! {
    /// The document parts `redact-apply` can strip.
    pub enum RedactSanitizeKey {
        #[default]
        Info = "info",
        Xmp = "xmp",
        Attachments = "attachments",
        Outlines = "outlines",
        Annots = "annots",
        Links = "links",
        Fields = "fields",
        Layers = "layers",
        Struct = "struct",
    }
}

wire_enum! {
    /// Why a save or validate was refused; exact tokens the renderer maps to UI text.
    pub enum SaveReason {
        NoDocument = "no-document",
        ReadOnly = "read-only",
        Encrypted = "encrypted",
        TooLarge = "too-large",
        #[default]
        Invalid = "invalid",
        NoMatch = "no-match",
        NoMarks = "no-marks",
        TooMany = "too-many",
        SharedContent = "shared-content",
        ScanBudget = "scan-budget",
        VerifyFailed = "verify-failed",
        UnsupportedTransform = "unsupported-transform",
        UnbalancedContent = "unbalanced-content",
        Certified = "certified",
        AlreadySigned = "already-signed",
        /// `sign-prepare {field}` names no empty signature field on that page.
        NoSuchField = "no-such-field",
        /// `compare-render`/`compare-text` with an empty compare slot.
        NoCompareDocument = "no-compare-document",
        /// Scan text needs the CJK fallback font and none is installed.
        NoFont = "no-font",
        /// An op this engine does not implement.
        Unsupported = "unsupported",
    }
}

wire_enum! {
    /// Which standard security handler a file uses; `none` means no `/Encrypt`.
    /// The TS `"unknown"` token is the `Unknown` variant.
    pub enum DocumentAlgorithm {
        #[default]
        None = "none",
        Rc440 = "rc4-40",
        Rc4128 = "rc4-128",
        Aes128 = "aes-128",
        Aes256 = "aes-256",
    }
}

wire_enum! {
    /// How the user proved access; `owner` implies every permission.
    pub enum AuthLevel {
        #[default]
        None = "none",
        User = "user",
        Owner = "owner",
    }
}

wire_enum! {
    /// What the next save does with security (`SecurityState.mode`).
    pub enum SecurityMode {
        #[default]
        Keep = "keep",
        None = "none",
        Aes256 = "aes-256",
    }
}

wire_enum! {
    /// One key of `DocumentPermissions` (`SecurityState.permissions` items).
    pub enum PermissionName {
        #[default]
        Print = "print",
        Copy = "copy",
        Edit = "edit",
        Annotate = "annotate",
        Form = "form",
    }
}

wire_enum! {
    /// The `kind` a `field-add` request names.
    pub enum FieldAddKind {
        #[default]
        Text = "text",
        Checkbox = "checkbox",
        Combo = "combo",
        Radio = "radio",
        Signature = "signature",
    }
}

wire_enum! {
    /// One fixed refusal token for the field-designer ops.
    pub enum FieldDetail {
        Xfa = "xfa",
        Scripted = "scripted",
        Signature = "signature",
        DuplicateName = "duplicate-name",
        InvalidName = "invalid-name",
        #[default]
        Invalid = "invalid",
        TooMany = "too-many",
        StaleTarget = "stale-target",
        /// `field-set` on a widget whose own field is read-only.
        FieldReadOnly = "field-read-only",
    }
}

wire_enum! {
    /// One fixed refusal token for `edit-text`.
    pub enum EditTextDetail {
        StaleTarget = "stale-target",
        NotCandidate = "not-candidate",
        UnsupportedEncoding = "unsupported-encoding",
        Overflow = "overflow",
        TailWouldMove = "tail-would-move",
        UnmeasurableWidth = "unmeasurable-width",
        AmbiguousTextAssociation = "ambiguous-text-association",
        TooLarge = "too-large",
        OutsidePage = "outside-page",
        #[default]
        Invalid = "invalid",
        WordTooWide = "word-too-wide",
    }
}

wire_enum! {
    /// One Image or Form placement action on one page.
    pub enum EditObjectAction {
        #[default]
        Move = "move",
        Resize = "resize",
        Delete = "delete",
        Replace = "replace",
        Duplicate = "duplicate",
        Transform = "transform",
        Crop = "crop",
    }
}

wire_enum! {
    /// A 90° step or a flip about the visible rect's centre.
    pub enum EditObjectTransform {
        #[default]
        RotateCw = "rotate-cw",
        RotateCcw = "rotate-ccw",
        FlipH = "flip-h",
        FlipV = "flip-v",
    }
}

wire_enum! {
    /// One fixed refusal token for `edit-object`.
    pub enum EditObjectDetail {
        StaleTarget = "stale-target",
        #[default]
        NotCandidate = "not-candidate",
        UnsupportedTransform = "unsupported-transform",
        UnsupportedImage = "unsupported-image",
        OutOfBounds = "out-of-bounds",
        TooLarge = "too-large",
        InvalidImage = "invalid-image",
        InvalidSelection = "invalid-selection",
        InvalidTransform = "invalid-transform",
        /// `clip-paste` with no clipboard.
        EmptyClipboard = "empty-clipboard",
    }
}

wire_enum! {
    /// The kind of one copied member (= the STATE selection member kind).
    pub enum ClipKind {
        Image = "image",
        Form = "form",
        #[default]
        Text = "text",
        Field = "field",
        Link = "link",
        Mark = "mark",
    }
}

wire_enum! {
    /// One fixed refusal token for `edit-replace-apply`.
    pub enum EditReplaceDetail {
        StalePreview = "stale-preview",
        StaleTarget = "stale-target",
        #[default]
        Invalid = "invalid",
        TooLarge = "too-large",
        WriteFailed = "write-failed",
    }
}

wire_enum! {
    /// What an edit target is.
    pub enum EditTargetKind {
        #[default]
        Text = "text",
        Image = "image",
        Form = "form",
    }
}

wire_enum! {
    /// Whether an edit target's identity is exact enough to validate an edit against.
    pub enum EditTargetSupport {
        Candidate = "candidate",
        #[default]
        Unsupported = "unsupported",
    }
}

wire_enum! {
    /// The AcroForm field kinds the fill panel shows (push buttons are omitted).
    pub enum FieldKind {
        #[default]
        Text = "text",
        Checkbox = "checkbox",
        Radio = "radio",
        Combo = "combo",
        List = "list",
        Signature = "signature",
    }
}

wire_enum! {
    /// The page's `/Tabs` name.
    pub enum TabsName {
        #[default]
        R = "R",
        C = "C",
        S = "S",
        A = "A",
        W = "W",
    }
}

wire_enum! {
    /// A link's destination kind; `blocked` is never followed.
    pub enum LinkKind {
        Page = "page",
        Uri = "uri",
        #[default]
        Blocked = "blocked",
    }
}

wire_enum! {
    /// What a shared redaction resource is (`RedactShared.kind`).
    pub enum RedactSharedKind {
        #[default]
        Form = "form",
        Image = "image",
    }
}

wire_enum! {
    /// What a painted item under a redaction box is (`RedactInspectShape.kind`).
    pub enum RedactShapeKind {
        #[default]
        Path = "path",
        Shading = "shading",
    }
}

// --------------------------------------------------------- shared values

/// `string | boolean`: a field value or default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FieldValue {
    /// A checkbox or radio state.
    Bool(bool),
    /// A text, choice or export value.
    Text(String),
}

impl Default for FieldValue {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

impl FieldValue {
    /// JS `String(value)`: `"true"`/`"false"` for booleans.
    pub fn to_js_string(&self) -> String {
        match self {
            Self::Bool(value) => value.to_string(),
            Self::Text(value) => value.clone(),
        }
    }

    /// The boolean, when it is one.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            Self::Text(_) => None,
        }
    }

    /// The string, when it is one.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Bool(_) => None,
            Self::Text(value) => Some(value),
        }
    }
}

/// The five `/P` flags this app reads. `annotate` is PDF bit 6, `form` bit 9.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct DocumentPermissions {
    pub print: bool,
    pub copy: bool,
    pub edit: bool,
    pub annotate: bool,
    pub form: bool,
}

/// The renderer's view of security: never a password, never the in-memory key.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct SecurityState {
    pub mode: SecurityMode,
    pub algorithm: DocumentAlgorithm,
    pub permissions: Vec<PermissionName>,
    pub auth: AuthLevel,
    pub pending: bool,
}

/// One text line of a render, in page points.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Line {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub size: f64,
    pub family: String,
    pub weight: String,
    pub style: String,
}

/// A page's size in points.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct PageSize {
    pub width: f64,
    pub height: f64,
}

/// What `open` learns about a document.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct DocumentInfo {
    pub encrypted: bool,
    pub needs_password: bool,
    /// Standard security handler algorithm, read from `/Encrypt` without a password.
    pub algorithm: DocumentAlgorithm,
    /// The five `/P` permissions; all true for an unencrypted document.
    pub permissions: DocumentPermissions,
    pub repaired: bool,
    pub signatures: i64,
    pub certified: bool,
    pub xfa: bool,
    pub attachments: i64,
    pub open_action: bool,
    pub tagged: bool,
    pub versions: i64,
    /// The document's print permission; false only for an encrypted document that clears it.
    pub printable: bool,
}

/// The slice of every mutation/undo/redo/save reply that owns dirty state.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct JournalState {
    pub revision: i64,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// The page geometry a page op's reply carries besides the journal state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct PageState {
    pub page_count: i64,
    pub pages: Vec<PageSize>,
    pub rotations: Vec<i64>,
}

/// One match: a page index and the quads it covers, in PDF points at scale 1.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchHit {
    pub page: i64,
    pub quads: Vec<Vec<f64>>,
}

/// One stext line under preserve-spans: bbox [x0, y0, x1, y1] in y-down page pt.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct CompareLine {
    pub text: String,
    pub bbox: Vec<f64>,
}

/// One optional-content layer as the panel sees it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct LayerItem {
    pub id: i64,
    pub name: String,
    pub visible: bool,
}

/// A Bates number range a stamp wrote.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct BatesRange {
    pub from: i64,
    pub to: i64,
}

// ------------------------------------------------------------- redaction

/// What `redact-apply` would strip (or kept), per document part.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactInventory {
    pub info: i64,
    pub xmp: bool,
    pub attachments: i64,
    pub outlines: i64,
    pub annots: i64,
    pub links: i64,
    pub fields: i64,
    pub layers: i64,
    #[serde(rename = "struct")]
    pub r#struct: i64,
}

/// `Record<RedactSanitizeKey, boolean>`: which parts `redact-apply` strips.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactSanitize {
    pub info: bool,
    pub xmp: bool,
    pub attachments: bool,
    pub outlines: bool,
    pub annots: bool,
    pub links: bool,
    pub fields: bool,
    pub layers: bool,
    #[serde(rename = "struct")]
    pub r#struct: bool,
}

/// A resource under a mark that other pages also use.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactShared {
    pub page: i64,
    pub kind: RedactSharedKind,
    pub other_pages: Vec<i64>,
}

/// One page's mark count in a redaction preview.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactPageCount {
    pub page: i64,
    pub marks: i64,
}

/// `redact-preview`'s reply (the renderer reads it without `id`/`ok`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactPreview {
    pub marks: i64,
    pub pages: Vec<i64>,
    pub page_counts: Vec<RedactPageCount>,
    pub shared: Vec<RedactShared>,
    pub sanitize: RedactInventory,
    pub inline_alt: i64,
}

/// A box held in memory; `rect` is y-down page pt, clamped to the page.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactRegion {
    pub page: i64,
    pub rect: Vec<f64>,
}

/// A box to inspect: an area (`rect`), a Redact mark (`annotId`), or a find hit (`search`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactInspectBox {
    pub page: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rect: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annot_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<bool>,
}

/// A picture a redaction box touches.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactInspectImage {
    pub rect: Vec<f64>,
    /// Apply removes the whole placement.
    pub whole: bool,
}

/// A painted shape or shading a redaction box touches.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactInspectShape {
    pub rect: Vec<f64>,
    pub kind: RedactShapeKind,
    /// Apply removes the whole draw.
    pub whole: bool,
}

/// An annotation or form field a redaction box touches (removed whole).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactInspectAnnot {
    /// The PDF subtype in lower case ("widget", "freetext", "link", "text", ...).
    pub kind: String,
    pub rect: Vec<f64>,
    /// A field's name or a comment's start.
    pub label: String,
}

/// One box of `redact-inspect`'s reply; every rect and quad is y-down page pt.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactInspectRegion {
    pub page: i64,
    pub rect: Vec<f64>,
    /// What the removed glyphs spell (lines joined by "\n").
    pub text: String,
    /// Quads [x0,y0,x1,y1,x2,y2,x3,y3] of the glyphs that go.
    pub glyphs: Vec<Vec<f64>>,
    pub images: Vec<RedactInspectImage>,
    pub shapes: Vec<RedactInspectShape>,
    pub annots: Vec<RedactInspectAnnot>,
    /// Applying needs this page in `flatten` (refusal `needs-flatten`).
    pub needs_flatten: bool,
    /// The reply's bound was reached: `glyphs` is coarse and `text` may be cut short.
    pub truncated: bool,
}

/// `redact-inspect`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactInspectReply {
    pub id: i64,
    pub ok: bool,
    pub regions: Vec<RedactInspectRegion>,
}

/// What `redact-apply` verified in the copy.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactVerified {
    pub text: i64,
    pub pixels: i64,
    pub raw: i64,
}

/// `redact-apply`'s reply: the copy, the marks applied, the pages changed and flattened.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactApplyReply {
    pub id: i64,
    pub ok: bool,
    /// Not parsed: fill it from `Reply::take_bytes("bytes")` after parsing.
    #[serde(skip)]
    pub bytes: Vec<u8>,
    pub marks: i64,
    pub pages: Vec<i64>,
    /// Pages turned into pictures.
    pub flattened: Vec<i64>,
    /// The source was signed and the copy is unsigned.
    pub unsigned: bool,
    /// Something shared under a box was removed elsewhere too.
    pub over_redacted: bool,
    pub sanitized: RedactInventory,
    pub kept: RedactInventory,
    pub verified: RedactVerified,
}

/// `redact-search`'s reply: a mutation plus how many marks it added.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RedactSearchReply {
    pub id: i64,
    pub ok: bool,
    pub revision: i64,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added: Option<i64>,
}

// ---------------------------------------------------------------- content

/// One of the eight axis-aligned orientations of a placement `cm`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(default, rename_all = "camelCase")]
pub struct EditOrient {
    pub swap: bool,
    /// 1 or -1.
    pub sx: i64,
    /// 1 or -1.
    pub sy: i64,
}

impl Default for EditOrient {
    fn default() -> Self {
        Self {
            swap: false,
            sx: 1,
            sy: 1,
        }
    }
}

/// An edit target's placement box in page points (`EditTarget.placement`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Placement {
    /// The visible rect.
    #[serde(rename = "box")]
    pub r#box: Vec<f64>,
    /// The `cm`'s axis orientation (required in TS; the renderer reads it as optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orient: Option<EditOrient>,
    /// The uncropped box (only when clipped).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full: Option<Vec<f64>>,
}

/// An image XObject's own facts.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct EditObjectImage {
    pub width: i64,
    pub height: i64,
    pub smask: bool,
    pub mask: bool,
    pub decode: bool,
    pub interpolate: bool,
    pub oc: bool,
}

/// Font facts as the stream declares them; `null` is "unknown".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct EditTargetFont {
    pub name: Option<String>,
    pub subtype: Option<String>,
    pub embedded: Option<bool>,
    pub subset: Option<bool>,
    pub writing_mode: Option<i64>,
}

/// One existing-content target on a page (`edit-targets` item).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct EditTarget {
    pub target_id: String,
    pub kind: EditTargetKind,
    pub bounds: Option<Vec<f64>>,
    pub support: EditTargetSupport,
    /// Why an item is not a candidate; one of the fixed reason tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Present only when decoding is proven.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<EditTargetFont>,
    /// Text follows this run on the same line, so the width must not change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail_follows: Option<bool>,
    /// A `TJ` run; an edit rewrites it as one `Tj` and drops its kerning.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kerned: Option<bool>,
    /// Shared, but an edit privatises this placement first (copy-on-write).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cowable: Option<bool>,
    /// The characters proven drawable with this run's font on this page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repertoire: Option<String>,
    /// The placement box in page points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placement: Option<Placement>,
    /// What `edit-object` may do to this placement, in UI order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_ops: Option<Vec<EditObjectAction>>,
    /// The `/Resources/XObject` name this placement draws.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<EditObjectImage>,
    /// `Tf` × the text matrix scale, in page pt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    /// `edit-text {move}` is proven for this run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moveable: Option<bool>,
    /// Why a candidate run is not moveable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub move_reason: Option<String>,
    /// The paragraph this line belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paragraph_id: Option<String>,
    /// A print-to-PDF glyph line: how many per-glyph blocks it stands for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glyphs: Option<i64>,
}

/// One paragraph of `edit-targets` (renderer.ts `ParagraphItem`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ParagraphItem {
    pub paragraph_id: String,
    pub target_ids: Vec<String>,
    pub bounds: Vec<f64>,
    pub left: f64,
    pub width: f64,
    pub leading: f64,
    pub font_size: f64,
    pub lines: i64,
    pub kerned: bool,
    pub tagged: bool,
}

/// `edit-targets`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct EditTargetsReply {
    pub id: i64,
    pub ok: bool,
    pub page: i64,
    pub generation: i64,
    pub revision: i64,
    pub items: Vec<EditTarget>,
    pub paragraphs: Vec<ParagraphItem>,
    /// Maps PDF user space to the y-down page view (6 numbers).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<Vec<f64>>,
}

impl EditTargetsReply {
    /// The transform, only when it is exactly six numbers (as the renderer checks).
    pub fn transform6(&self) -> Option<[f64; 6]> {
        let values = self.transform.as_deref()?;
        <[f64; 6]>::try_from(values).ok()
    }
}

/// One run `edit-replace-preview` can change.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ReplaceMatch {
    pub target_id: String,
    pub text: String,
    pub next: String,
    pub count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail_follows: Option<bool>,
}

/// One run `edit-replace-preview` cannot change, with its U2/U3 token.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct ReplaceRejected {
    pub target_id: String,
    pub text: String,
    pub count: i64,
    pub detail: String,
}

/// `edit-replace-preview`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct EditReplacePreviewReply {
    pub id: i64,
    pub ok: bool,
    pub page: i64,
    pub generation: i64,
    pub revision: i64,
    pub find: String,
    pub replace: String,
    pub matches: Vec<ReplaceMatch>,
    pub rejected: Vec<ReplaceRejected>,
    pub preview_token: String,
}

/// `edit-replace-apply`'s reply: the journal and page state plus the runs replaced.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct EditReplaceApplyReply {
    pub id: i64,
    pub ok: bool,
    pub revision: i64,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub page: i64,
    /// Absent → the renderer counts the requested runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaced: Option<i64>,
}

/// `text-add`'s reply: a mutation plus the new run's target and bounds.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct TextAddReply {
    pub id: i64,
    pub ok: bool,
    pub revision: i64,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    /// Absent or null: the new run has no on-page target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds: Option<Vec<f64>>,
}

/// `Record<ClipKind, number>`, keyed by the wire token (as the renderer indexes it).
pub type ClipKinds = BTreeMap<String, i64>;

/// `clip-copy`'s reply: the clipboard's counts (nothing is written).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct ClipCopyReply {
    pub id: i64,
    pub ok: bool,
    pub count: i64,
    pub kinds: ClipKinds,
    pub revision: i64,
    pub dirty: bool,
}

/// `clip-paste`'s reply: the journal state plus what was pasted.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ClipPasteReply {
    pub id: i64,
    pub ok: bool,
    pub revision: i64,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub page: i64,
    /// Absent → the renderer uses `names.len()`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<i64>,
    /// The pasted image/Form XObject names.
    pub names: Vec<String>,
    pub target_ids: Vec<String>,
    /// The pasted fields' names.
    pub fields: Vec<String>,
    pub field_ids: Vec<i64>,
    pub link_ids: Vec<i64>,
    pub mark_ids: Vec<i64>,
    /// Members not pasted (`{index, kind, reason}`); the renderer reads only the length.
    pub skipped: Vec<Value>,
}

// ------------------------------------------------------------------ items

/// One widget on a page, as the fill panel sees it; `id` is its object number.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct FieldItem {
    pub id: i64,
    pub name: String,
    pub label: String,
    pub kind: FieldKind,
    pub value: FieldValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    /// Combo only: `/Ff` bit 19, any typed value is accepted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_len: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiline: Option<bool>,
    pub read_only: bool,
    /// `/Ff` bit 2 on the field dict (the parent's, for a radio kid).
    pub required: bool,
    /// The field dict's own raw `/TU`, present only when the key exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tu: Option<String>,
    /// `/DV`; absent when the field has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<FieldValue>,
    /// The widget, its field, or a document-level script: never executed here.
    pub scripted: bool,
    /// The on-state export value of a checkbox or radio widget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    /// Page points, y down from the page top.
    pub rect: Vec<f64>,
    /// On a `/Tabs /S` page, the widget's position in the struct-tree walk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub struct_index: Option<i64>,
    /// Kind `signature` only: the widget's own dict has a `/V`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signed: Option<bool>,
}

/// One annotation on a page; `rect` is PDF points, y down from the page top.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct AnnotItem {
    pub id: i64,
    #[serde(rename = "type")]
    pub r#type: String,
    pub rect: Vec<f64>,
    pub contents: String,
    /// `/Subj`, which tells a signature appearance stamp from an icon stamp.
    pub subject: String,
    pub editable: bool,
    /// A Redact mark that still carries search quads or its search tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<bool>,
}

/// One bookmark, in preorder; `page` is null when the destination is not a page.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct OutlineEntry {
    pub id: i64,
    pub title: String,
    pub depth: i64,
    pub page: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// One link on a page; `kind: blocked` is reported but never followed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct LinkEntry {
    pub id: i64,
    /// The `/Annots` entry's object number; 0 when it is direct or unpaired.
    pub annot_id: i64,
    pub rect: Vec<f64>,
    pub kind: LinkKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

// ---------------------------------------------------------------- replies

/// `open`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct OpenReply {
    pub id: i64,
    pub ok: bool,
    pub page_count: i64,
    pub pages: Vec<PageSize>,
    pub rotations: Vec<i64>,
    pub info: DocumentInfo,
    pub open_ms: f64,
    pub revision: i64,
    pub auth: AuthLevel,
    /// The pair was over MAX_PDF_BYTES, so the compare slot was closed first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compare_closed: Option<bool>,
}

/// `render`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RenderReply {
    pub id: i64,
    pub ok: bool,
    pub page: i64,
    pub revision: i64,
    pub width: f64,
    pub height: f64,
    pub pixel_width: i64,
    pub pixel_height: i64,
    /// Not parsed: fill it from `Reply::take_bytes("pixels")` after parsing.
    #[serde(skip)]
    pub pixels: Vec<u8>,
    pub text: String,
    pub lines: Vec<Line>,
    pub non_white: i64,
    pub paint_ms: f64,
    pub render_ms: f64,
}

/// `compare-open`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct CompareOpenReply {
    pub id: i64,
    pub ok: bool,
    pub name: String,
    pub page_count: i64,
    pub pages: Vec<PageSize>,
}

/// `compare-render`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct CompareRenderReply {
    pub id: i64,
    pub ok: bool,
    pub page: i64,
    pub width: f64,
    pub height: f64,
    pub pixel_width: i64,
    pub pixel_height: i64,
    /// Not parsed: fill it from `Reply::take_bytes("pixels")` after parsing.
    #[serde(skip)]
    pub pixels: Vec<u8>,
    pub text: String,
    pub lines: Vec<Line>,
    pub non_white: i64,
    pub paint_ms: f64,
    pub render_ms: f64,
}

/// `compare-text`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct CompareTextReply {
    pub id: i64,
    pub ok: bool,
    pub page_a: i64,
    pub page_b: i64,
    pub revision: i64,
    pub same: i64,
    pub removed: Vec<CompareLine>,
    pub added: Vec<CompareLine>,
    /// Always 0.
    pub moved: i64,
}

/// `search`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchReply {
    pub id: i64,
    pub ok: bool,
    pub hits: Vec<SearchHit>,
    pub total: i64,
    pub truncated: bool,
    pub search_ms: f64,
}

/// `save`'s reply; `ticket` names the state the bytes were written from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct SaveReply {
    pub id: i64,
    pub ok: bool,
    /// Not parsed: fill it from `Reply::take_bytes("bytes")` after parsing.
    #[serde(skip)]
    pub bytes: Vec<u8>,
    pub save_ms: f64,
    pub dirty: bool,
    pub revision: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security: Option<SecurityState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
}

/// `thumbnail`'s reply: pixels only, no structured text.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ThumbReply {
    pub id: i64,
    pub ok: bool,
    pub page: i64,
    pub revision: i64,
    pub width: f64,
    pub height: f64,
    pub pixel_width: i64,
    pub pixel_height: i64,
    /// Not parsed: fill it from `Reply::take_bytes("pixels")` after parsing.
    #[serde(skip)]
    pub pixels: Vec<u8>,
}

/// `validate`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct ValidateReply {
    pub id: i64,
    pub ok: bool,
    pub page_count: i64,
    /// Not parsed: fill it from `Reply::take_bytes("bytes")` after parsing.
    #[serde(skip)]
    pub bytes: Vec<u8>,
    pub dirty: bool,
    pub revision: i64,
}

/// `annots`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct AnnotsReply {
    pub id: i64,
    pub ok: bool,
    pub page: i64,
    pub items: Vec<AnnotItem>,
}

/// A mutation's reply: the journal state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct MutationReply {
    pub id: i64,
    pub ok: bool,
    pub revision: i64,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    /// The same box again: no journal op (renderer's `MutationReply & {noop?}` on
    /// `redact-set`, `annot-move`, `link-set`, `edit-object`, `edit-batch`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub noop: Option<bool>,
}

/// A page op or an undo/redo: the reply owns the new page count, sizes and rotations.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct PageReply {
    pub id: i64,
    pub ok: bool,
    pub revision: i64,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub page_count: i64,
    pub pages: Vec<PageSize>,
    pub rotations: Vec<i64>,
    /// Undo/redo: the step undone or redone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
}

impl PageReply {
    /// The page geometry part.
    pub fn page_state(&self) -> PageState {
        PageState {
            page_count: self.page_count,
            pages: self.pages.clone(),
            rotations: self.rotations.clone(),
        }
    }
}

/// `fields`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct FieldsReply {
    pub id: i64,
    pub ok: bool,
    pub page: i64,
    /// The page's `/Tabs`, null when absent or not one of the five.
    pub tabs: Option<TabsName>,
    pub items: Vec<FieldItem>,
}

/// `field-add`'s reply: a mutation plus the new widget and the name used.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct FieldAddReply {
    pub id: i64,
    pub ok: bool,
    pub revision: i64,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub widget_id: Option<i64>,
    /// `unknown` in TS; use `name_str`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Value>,
}

impl FieldAddReply {
    /// `typeof reply.name === "string" ? reply.name : ""`.
    pub fn name_str(&self) -> &str {
        self.name.as_ref().and_then(Value::as_str).unwrap_or("")
    }
}

/// `outline`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct OutlineReply {
    pub id: i64,
    pub ok: bool,
    pub items: Vec<OutlineEntry>,
}

/// `links`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct LinksReply {
    pub id: i64,
    pub ok: bool,
    pub page: i64,
    pub items: Vec<LinkEntry>,
}

/// `insert-pages`'s reply: a page edit plus what it added.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct InsertReply {
    pub id: i64,
    pub ok: bool,
    pub revision: i64,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub page_count: i64,
    pub pages: Vec<PageSize>,
    pub rotations: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    pub inserted: i64,
    pub field_names: Vec<String>,
}

/// `extract-pages`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct ExtractReply {
    pub id: i64,
    pub ok: bool,
    /// Not parsed: fill it from `Reply::take_bytes("bytes")` after parsing.
    #[serde(skip)]
    pub bytes: Vec<u8>,
    pub page_count: i64,
}

/// `pdf-from-images`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct PdfFromImagesReply {
    pub id: i64,
    pub ok: bool,
    /// Not parsed: fill it from `Reply::take_bytes("bytes")` after parsing.
    #[serde(skip)]
    pub bytes: Vec<u8>,
    pub page_count: i64,
}

/// `scan-compose`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct ScanComposeReply {
    pub id: i64,
    pub ok: bool,
    /// Not parsed: fill it from `Reply::take_bytes("bytes")` after parsing.
    #[serde(skip)]
    pub bytes: Vec<u8>,
    pub page_count: i64,
}

/// `scan-measure`'s reply: per distinct character, its advances (1/1000 em) and font.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ScanMeasureReply {
    pub id: i64,
    pub ok: bool,
    pub chars: Vec<String>,
    pub regular: Vec<f64>,
    pub bold: Vec<f64>,
    /// helvetica, latin, nimbus, cjk, or missing (drawn as a box).
    pub fonts: Vec<String>,
}

/// `text-all`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct TextAllReply {
    pub id: i64,
    pub ok: bool,
    pub text: String,
}

/// `labels`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct LabelsReply {
    pub id: i64,
    pub ok: bool,
    pub items: Vec<String>,
}

/// `labels-set`/`labels-clear`'s reply: a mutation plus the new labels.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct LabelsSetReply {
    pub id: i64,
    pub ok: bool,
    pub revision: i64,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub items: Vec<String>,
}

/// `page-ids`'s reply: the page objects' numbers, in page order.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct PageIdsReply {
    pub id: i64,
    pub ok: bool,
    pub ids: Vec<i64>,
}

/// `layers`'s reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct LayersReply {
    pub id: i64,
    pub ok: bool,
    pub items: Vec<LayerItem>,
}

/// `layer-set`'s reply; `persisted: false` is a view-only toggle.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct LayerSetReply {
    pub id: i64,
    pub ok: bool,
    pub persisted: bool,
    pub items: Vec<LayerItem>,
}

/// `authenticate`'s reply; the document fields are present only when `authenticated`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct AuthenticateReply {
    pub id: i64,
    pub ok: bool,
    pub authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<Vec<PageSize>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotations: Option<Vec<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<DocumentInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthLevel>,
}

/// `security-set`'s reply: the new security view plus the journal state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct SecurityReply {
    pub id: i64,
    pub ok: bool,
    pub security: SecurityState,
    pub revision: i64,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

/// `stamp`'s reply: a mutation plus the Bates range written, if any.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct StampReply {
    pub id: i64,
    pub ok: bool,
    pub revision: i64,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bates: Option<BatesRange>,
}

/// `batch-file`'s reply: one stamped file of a folder (never the open document).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct BatchFileReply {
    pub id: i64,
    pub ok: bool,
    pub pages: i64,
    /// Not parsed: fill it from `Reply::take_bytes("bytes")` after parsing.
    #[serde(skip)]
    pub bytes: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bates: Option<BatesRange>,
}

/// `sign-list`'s reply: the signature fields for the verifier and the file's bytes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct SignListReply {
    pub id: i64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signatures: Option<Vec<Value>>,
    /// The certification's `/P`, or null when not certified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certified: Option<i64>,
    /// Not parsed: fill it from `Reply::take_bytes("bytes")` after parsing.
    #[serde(skip)]
    pub bytes: Vec<u8>,
}

/// `sign-prepare`'s reply: the prepared file and where its signature goes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct SignPrepareReply {
    pub id: i64,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Not parsed: fill it from `Reply::take_bytes("bytes")` after parsing.
    #[serde(skip)]
    pub bytes: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contents_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contents_len: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byte_range_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revisions: Option<i64>,
}

/// An `{ ok: false }` reply with an error message.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct ErrorReply {
    pub id: i64,
    pub ok: bool,
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// An engine refusal: `reason` is the refusal/save reason, `detail` the U1-U6 token;
/// any other key (e.g. `missing`, `pages`, journal state) lands in `extra`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct RefusalReply {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub ok: bool,
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The journal state of a reply that carries one.
macro_rules! journal_of {
    ($($name:ident),+ $(,)?) => {
        $(
            impl $name {
                /// The journal-state part (what `applyEdit` reads).
                pub fn journal(&self) -> JournalState {
                    JournalState {
                        revision: self.revision,
                        dirty: self.dirty,
                        can_undo: self.can_undo,
                        can_redo: self.can_redo,
                    }
                }
            }
        )+
    };
}

journal_of!(
    MutationReply,
    PageReply,
    InsertReply,
    LabelsSetReply,
    SecurityReply,
    StampReply,
    FieldAddReply,
    TextAddReply,
    RedactSearchReply,
    EditReplaceApplyReply,
    ClipPasteReply,
);

// --------------------------------------------------------------- requests

/// What `sign-prepare` signs: the signer's name and reason, where, and whether it certifies.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct SignPrepareOptions {
    pub name: String,
    pub reason: String,
    pub page: i64,
    /// Null for an invisible signature or an existing field.
    pub rect: Option<[f64; 4]>,
    pub certify: bool,
    pub when: f64,
    /// Sign this existing empty field on `page` in place; absent or null = a new widget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

/// A link's target in `link-add`/`link-set`: a page or a URI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum LinkTarget {
    /// A 0-based page.
    Page {
        /// The page index.
        page: i64,
    },
    /// An external URI.
    Uri {
        /// The URI.
        uri: String,
    },
}

/// A move in page pt, y down (the `bounds` space).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct EditMove {
    pub dx: f64,
    pub dy: f64,
}

/// A batch stamp's action (`batch-file`'s `action`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct BatchStampAction {
    pub kind: String,
    pub text: String,
    /// A range string; "" or absent = all pages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bates_start: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bates_pad: Option<i64>,
}

/// One member write of an `edit-batch`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum EditBatchItem {
    /// Move one text run.
    #[serde(rename_all = "camelCase")]
    EditText {
        /// The run.
        target_id: String,
        /// The move.
        #[serde(rename = "move")]
        r#move: EditMove,
    },
    /// Move or resize one field widget.
    #[serde(rename_all = "camelCase")]
    FieldProps {
        /// The widget.
        widget_id: i64,
        /// The new rect.
        rect: Vec<f64>,
    },
    /// Delete one field widget.
    #[serde(rename_all = "camelCase")]
    FieldDelete {
        /// The widget.
        widget_id: i64,
    },
    /// Move or resize one link.
    #[serde(rename_all = "camelCase")]
    LinkSet {
        /// The link's `/Annots` object number.
        annot_id: i64,
        /// The new rect.
        rect: Vec<f64>,
    },
    /// Delete one link.
    #[serde(rename_all = "camelCase")]
    LinkDelete {
        /// The link's `/Annots` object number.
        annot_id: i64,
    },
    /// Move or resize one Redact mark.
    #[serde(rename_all = "camelCase")]
    RedactSet {
        /// The mark.
        annot_id: i64,
        /// The new rect.
        rect: Vec<f64>,
    },
    /// Delete one annotation.
    #[serde(rename_all = "camelCase")]
    AnnotDelete {
        /// The annotation.
        annot_id: i64,
    },
    /// Move or delete one object placement.
    #[serde(rename_all = "camelCase")]
    EditObject {
        /// The placement.
        target_id: String,
        /// `move` or `delete`.
        action: EditObjectAction,
        /// The new box (page points, bottom-left origin) for `move`.
        #[serde(default, rename = "box", skip_serializing_if = "Option::is_none")]
        r#box: Option<Vec<f64>>,
    },
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;
    use serde_json::json;

    fn parse<T: DeserializeOwned>(value: Value) -> T {
        serde_json::from_value(value).expect("parses")
    }

    #[test]
    fn every_reply_parses_from_an_empty_object() {
        macro_rules! empty {
            ($($name:ty),+ $(,)?) => {
                $( let _: $name = parse(json!({})); )+
            };
        }
        empty!(
            OpenReply,
            RenderReply,
            ThumbReply,
            SearchReply,
            SaveReply,
            ValidateReply,
            AnnotsReply,
            FieldsReply,
            OutlineReply,
            LinksReply,
            MutationReply,
            PageReply,
            InsertReply,
            AuthenticateReply,
            SecurityReply,
            ExtractReply,
            PdfFromImagesReply,
            ScanComposeReply,
            ScanMeasureReply,
            TextAllReply,
            LabelsReply,
            LabelsSetReply,
            PageIdsReply,
            LayersReply,
            LayerSetReply,
            CompareOpenReply,
            CompareRenderReply,
            CompareTextReply,
            RedactPreview,
            RedactInspectReply,
            RedactApplyReply,
            RedactSearchReply,
            EditTargetsReply,
            EditReplacePreviewReply,
            EditReplaceApplyReply,
            TextAddReply,
            ClipCopyReply,
            ClipPasteReply,
            StampReply,
            BatchFileReply,
            SignListReply,
            SignPrepareReply,
            FieldAddReply,
            ErrorReply,
            RefusalReply,
            DocumentInfo,
            SecurityState,
            EditTarget,
            FieldItem,
        );
    }

    #[test]
    fn open_reply_with_info_and_permissions() {
        let reply: OpenReply = parse(json!({
            "id": 3, "ok": true, "pageCount": 2,
            "pages": [{"width": 612, "height": 792}, {"width": 595.5, "height": 842}],
            "rotations": [0, 90],
            "info": {
                "encrypted": true, "needsPassword": false, "algorithm": "aes-256",
                "permissions": {"print": true, "copy": false, "edit": false, "annotate": true, "form": true},
                "repaired": false, "signatures": 1, "certified": false, "xfa": false,
                "attachments": 0, "openAction": false, "tagged": true, "versions": 2, "printable": true
            },
            "openMs": 12.5, "revision": 0, "auth": "user",
            "pdf": {"$buffer": 0}
        }));
        assert_eq!(reply.id, 3);
        assert!(reply.ok);
        assert_eq!(reply.page_count, 2);
        assert_eq!(
            reply.pages[1],
            PageSize {
                width: 595.5,
                height: 842.0
            }
        );
        assert_eq!(reply.rotations, vec![0, 90]);
        assert_eq!(reply.info.algorithm, DocumentAlgorithm::Aes256);
        assert_eq!(reply.info.algorithm.as_str(), "aes-256");
        assert!(reply.info.permissions.print && !reply.info.permissions.copy);
        assert!(!reply.info.needs_password && reply.info.tagged);
        assert_eq!(reply.info.versions, 2);
        assert_eq!(reply.auth, AuthLevel::User);
        assert_eq!(reply.compare_closed, None);
        assert_eq!(reply.open_ms, 12.5);
        // Round trip keeps the optional key out.
        let back = serde_json::to_value(&reply).expect("serializes");
        assert!(back.get("compareClosed").is_none());
        assert_eq!(back["info"]["algorithm"], "aes-256");
    }

    #[test]
    fn field_values_are_bool_or_text() {
        let checked: FieldItem = parse(
            json!({"id": 7, "name": "agree", "kind": "checkbox", "value": true, "on": "Yes"}),
        );
        assert_eq!(checked.value, FieldValue::Bool(true));
        assert_eq!(checked.value.to_js_string(), "true");
        assert_eq!(checked.kind, FieldKind::Checkbox);
        assert_eq!(checked.on.as_deref(), Some("Yes"));
        let text: FieldItem =
            parse(json!({"id": 8, "kind": "text", "value": "x", "default": false}));
        assert_eq!(text.value, FieldValue::Text("x".into()));
        assert_eq!(text.value.to_js_string(), "x");
        assert_eq!(text.default, Some(FieldValue::Bool(false)));
        let empty: FieldItem = parse(json!({}));
        assert_eq!(empty.value.to_js_string(), "");
    }

    #[test]
    fn edit_target_with_placement_and_orient() {
        let target: EditTarget = parse(json!({
            "targetId": "p0:i3", "kind": "image", "bounds": null, "support": "candidate",
            "placement": {"box": [10, 20, 110, 220.5], "orient": {"swap": true, "sx": -1, "sy": 1}},
            "objectOps": ["move", "crop", "warp"], "objectName": "Im1",
            "image": {"width": 640, "height": 480, "smask": false, "mask": false, "decode": false, "interpolate": true, "oc": false}
        }));
        assert_eq!(target.kind, EditTargetKind::Image);
        assert_eq!(target.support, EditTargetSupport::Candidate);
        assert_eq!(target.bounds, None);
        let placement = target.placement.as_ref().expect("placement");
        assert_eq!(placement.r#box, vec![10.0, 20.0, 110.0, 220.5]);
        assert_eq!(
            placement.orient,
            Some(EditOrient {
                swap: true,
                sx: -1,
                sy: 1
            })
        );
        assert_eq!(placement.full, None);
        assert_eq!(
            target.object_ops,
            Some(vec![
                EditObjectAction::Move,
                EditObjectAction::Crop,
                EditObjectAction::Unknown
            ])
        );
        assert_eq!(target.image.as_ref().map(|image| image.width), Some(640));
        // A bare orient defaults to the identity, not to zero scales.
        let bare: EditOrient = parse(json!({}));
        assert_eq!(
            bare,
            EditOrient {
                swap: false,
                sx: 1,
                sy: 1
            }
        );
        // `bounds: null` round-trips as null (nullable, not optional).
        let back = serde_json::to_value(&target).expect("serializes");
        assert_eq!(back["bounds"], Value::Null);
        assert!(back.get("text").is_none());
    }

    #[test]
    fn unknown_tokens_parse_to_unknown() {
        let link: LinkEntry = parse(json!({"kind": "javascript", "rect": [0, 0, 1, 1]}));
        assert_eq!(link.kind, LinkKind::Unknown);
        assert_eq!(link.kind.as_str(), "unknown");
        let algorithm: DocumentAlgorithm = parse(json!("unknown"));
        assert_eq!(algorithm, DocumentAlgorithm::Unknown);
        let fields: FieldsReply = parse(json!({"tabs": "S", "items": [{"kind": "push"}]}));
        assert_eq!(fields.tabs, Some(TabsName::S));
        assert_eq!(fields.items[0].kind, FieldKind::Unknown);
        let none: FieldsReply = parse(json!({"tabs": null}));
        assert_eq!(none.tabs, None);
        assert_eq!(
            SaveReason::from_token("no-such-field"),
            SaveReason::NoSuchField
        );
        assert_eq!(SaveReason::from_token("whatever"), SaveReason::Unknown);
        assert_eq!(DocumentAlgorithm::Rc440.as_str(), "rc4-40");
        assert_eq!(
            serde_json::to_value(TabsName::R).expect("serializes"),
            json!("R")
        );
    }

    #[test]
    fn refusal_keeps_extra_keys() {
        let refusal: RefusalReply = parse(json!({
            "id": 9, "ok": false, "error": "text-add: not in font", "detail": "unsupported-encoding",
            "missing": ["\u{4e00}"], "revision": 4
        }));
        assert_eq!(refusal.id, Some(9));
        assert!(!refusal.ok);
        assert_eq!(refusal.detail.as_deref(), Some("unsupported-encoding"));
        assert_eq!(refusal.reason, None);
        assert_eq!(refusal.extra.get("missing"), Some(&json!(["\u{4e00}"])));
        assert_eq!(refusal.extra.get("revision"), Some(&json!(4)));
        assert!(!refusal.extra.contains_key("error"));
    }

    #[test]
    fn local_reply_shapes() {
        let targets: EditTargetsReply = parse(json!({
            "id": 1, "ok": true, "page": 0, "generation": 2, "revision": 3, "items": [],
            "paragraphs": [{"paragraphId": "p1", "targetIds": ["a", "b"], "bounds": [1, 2, 3, 4],
                "left": 1.5, "width": 200, "leading": 14.4, "fontSize": 12, "lines": 2, "kerned": false, "tagged": true}],
            "transform": [1, 0, 0, -1, 0, 792]
        }));
        assert_eq!(targets.paragraphs[0].lines, 2);
        assert_eq!(
            targets.transform6(),
            Some([1.0, 0.0, 0.0, -1.0, 0.0, 792.0])
        );
        let paste: ClipPasteReply = parse(
            json!({"revision": 5, "dirty": true, "canUndo": true, "names": ["Im2"], "skipped": [{"index": 0}]}),
        );
        assert_eq!(paste.count, None);
        assert_eq!(paste.skipped.len(), 1);
        assert_eq!(
            paste.journal(),
            JournalState {
                revision: 5,
                dirty: true,
                can_undo: true,
                can_redo: false
            }
        );
        let copy: ClipCopyReply =
            parse(json!({"count": 2, "kinds": {"image": 1, "text": 1, "form": 0}}));
        assert_eq!(copy.kinds.get("image"), Some(&1));
        let added: FieldAddReply = parse(json!({"name": "Signature1", "widgetId": 12}));
        assert_eq!(added.name_str(), "Signature1");
        let numbered: FieldAddReply = parse(json!({"name": 3}));
        assert_eq!(numbered.name_str(), "");
        let stamp: StampReply = parse(json!({"bates": {"from": 1, "to": 4}}));
        assert_eq!(stamp.bates, Some(BatesRange { from: 1, to: 4 }));
        let undo: PageReply = parse(
            json!({"pageCount": 1, "pages": [{"width": 10, "height": 20}], "rotations": [270], "step": "rotate"}),
        );
        assert_eq!(undo.step.as_deref(), Some("rotate"));
        assert_eq!(undo.page_state().rotations, vec![270]);
        let applied: RedactApplyReply = parse(
            json!({"marks": 2, "verified": {"text": 2, "pixels": 2, "raw": 1}, "bytes": {"$buffer": 0}}),
        );
        assert!(!applied.over_redacted && applied.flattened.is_empty());
        assert_eq!(applied.verified.raw, 1);
        let item = EditBatchItem::EditObject {
            target_id: "t".into(),
            action: EditObjectAction::Move,
            r#box: Some(vec![1.0, 2.0, 3.0, 4.0]),
        };
        assert_eq!(
            serde_json::to_value(&item).expect("serializes"),
            json!({"op": "edit-object", "targetId": "t", "action": "move", "box": [1.0, 2.0, 3.0, 4.0]})
        );
    }
}
