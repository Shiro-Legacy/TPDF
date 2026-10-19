//! The value constants of renderer.ts (`R:n`) and recovery.ts (`V:n`): numbers, strings and
//! tables. DOM lookups are in `dom_ids`; function-valued consts (`cmdKey`, `sigRect`,
//! `padCount`, `round2`, `round3`) are ported as functions by their parts; the latest-guard
//! and gate (`openGuard`, `workerGate`) collapse in the synchronous port. `HEADLESS` is a
//! `Session` field, and `SAVE_REASON_TEXT` (R:11505) is `crate::messages::save::reason`.
//! Tables keep the TS key order.

use super::types::{OpKind, UiMode};

/// R:201: the command modifier is ⌘ on macOS and Ctrl elsewhere.
pub const IS_MAC: bool = cfg!(target_os = "macos");
/// R:225.
pub const CACHE_LIMIT: usize = 8;
/// R:226.
pub const CACHE_BYTES: usize = 96 * 1024 * 1024;
/// R:227.
pub const REQUEST_TIMEOUT_MS: f64 = 20_000.0;
/// R:229: a full rewrite of a near-ceiling document can take well over the render watchdog.
pub const SAVE_TIMEOUT_MS: f64 = 120_000.0;
/// R:230.
pub const MAX_WORKER_RESTARTS: i64 = 3;
/// R:231.
pub const MAX_ZOOM: f64 = 8.0;
/// R:232.
pub const ZOOM_STEPS: [f64; 8] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0];
/// R:233.
pub const SEARCH_DEBOUNCE_MS: f64 = 200.0;
/// R:235: sidebar geometry (THUMB_ROW_PX is also the inline row height in buildThumbs).
pub const THUMB_ROW_PX: f64 = 88.0;
/// R:236.
pub const THUMB_BOX_PX: f64 = 64.0;
/// R:237.
pub const THUMB_BUFFER: usize = 4;
/// R:239: thumbnails are decoration: a slow one must not be mistaken for a hung worker.
pub const THUMB_TIMEOUT_MS: f64 = 10_000.0;
/// R:242: print raster resolution.
pub const PRINT_DPI: f64 = 150.0;
/// R:251: total JPEG data-URL bytes for one print job (measured, see the TS comment).
pub const PRINT_MAX_BYTES: usize = 40 * 1024 * 1024;

/// R:592: which inspector tab each mode belongs to; Organize has no tab of its own (X3).
pub const MODE_TAB: [(UiMode, Option<&str>); 6] = [
    (UiMode::Read, Some("tab-links")),
    (UiMode::Annotate, Some("tab-notes")),
    (UiMode::Organize, None),
    (UiMode::Forms, Some("tab-fields")),
    (UiMode::Edit, Some("tab-text")),
    (UiMode::Redact, Some("tab-notes")),
];

/// `MODE_TAB[mode]`.
pub fn mode_tab(mode: UiMode) -> Option<&'static str> {
    MODE_TAB
        .iter()
        .find(|(m, _)| *m == mode)
        .and_then(|(_, tab)| *tab)
}

/// R:729: the Page menu's edit-request names.
pub const PAGE_OPS: [&str; 5] = [
    "rotate-right",
    "rotate-left",
    "move-up",
    "move-down",
    "delete-page",
];
/// R:731: a journal step that moved the page tree, so an undo/redo of it rebuilds the page list.
pub const PAGE_STEPS: [&str; 4] = ["rotate page", "delete page", "move page", "insert pages"];
/// R:733: a journal step that changed the labels; an undo/redo of one reloads them.
pub const LABEL_STEPS: [&str; 2] = ["set page labels", "clear page labels"];

/// R:735: which mode owns each menu `edit-request` op, so the row that shows it is up first.
pub const OP_MODE: [(&str, UiMode); 41] = [
    ("note", UiMode::Annotate),
    ("highlight", UiMode::Annotate),
    ("underline", UiMode::Annotate),
    ("strikeout", UiMode::Annotate),
    ("freetext", UiMode::Annotate),
    ("square", UiMode::Annotate),
    ("circle", UiMode::Annotate),
    ("line", UiMode::Annotate),
    ("ink", UiMode::Annotate),
    ("stamp", UiMode::Annotate),
    ("sign", UiMode::Annotate),
    ("shapes", UiMode::Annotate),
    ("pages", UiMode::Organize),
    ("insert", UiMode::Organize),
    ("assemble", UiMode::Organize),
    ("extract", UiMode::Organize),
    ("watermark", UiMode::Organize),
    ("header-footer", UiMode::Organize),
    ("labels", UiMode::Organize),
    ("decorate", UiMode::Organize),
    ("security", UiMode::Organize),
    ("form", UiMode::Forms),
    ("fill-text", UiMode::Forms),
    ("sign-place", UiMode::Forms),
    ("content-edit", UiMode::Annotate),
    ("text-add-open", UiMode::Annotate),
    ("content", UiMode::Annotate),
    ("content-object", UiMode::Annotate),
    ("field-add", UiMode::Forms),
    ("field-props", UiMode::Forms),
    ("field-delete", UiMode::Forms),
    ("fields", UiMode::Forms),
    ("redact-area", UiMode::Redact),
    ("redact-search", UiMode::Redact),
    ("redact-apply-open", UiMode::Redact),
    ("cert-sign-open", UiMode::Forms),
    ("redact", UiMode::Redact),
    ("redact-preview", UiMode::Redact),
    ("redact-apply", UiMode::Redact),
    ("redact-confirm", UiMode::Redact),
    ("redact-page", UiMode::Redact),
];

/// `OP_MODE[op]`.
pub fn op_mode(op: &str) -> Option<UiMode> {
    OP_MODE
        .iter()
        .find(|(name, _)| *name == op)
        .map(|(_, mode)| *mode)
}

/// R:787: the scripted-field warning, shown on the row and once at the top of the panel.
pub const SCRIPT_WARNING: &str =
    "Runs a script this app does not execute; the value is saved as typed.";

/// R:1311: why the restart notice says changes were not recoverable, per `Unavailable`.
pub const UNTRACKED_TEXT: [(&str, &str); 3] = [
    ("budget", "they were too large to keep a recovery copy."),
    ("clipboard", "they include a paste from another document."),
    ("generation", "they could not be recorded for recovery."),
];

/// R:1322: reads the window issues by itself; one that hung or crashed the worker is paused.
pub const AUTO_READS: [&str; 12] = [
    "render",
    "thumbnail",
    "annots",
    "fields",
    "links",
    "outline",
    "labels",
    "layers",
    "page-ids",
    "edit-targets",
    "redact-preview",
    "sign-list",
];

/// R:2222: the modes `mode {mode}` accepts.
pub const DIRECT_MODES: [UiMode; 6] = [
    UiMode::Read,
    UiMode::Annotate,
    UiMode::Organize,
    UiMode::Forms,
    UiMode::Edit,
    UiMode::Redact,
];
/// R:2255.
pub const DRAW_LINK_PROMPT: &str = "Draw link: drag a rectangle on the page";
/// R:2256.
pub const DRAW_SIGFIELD_PROMPT: &str = "Draw signature field: drag a rectangle on the page";

/// R:2553.
pub const ALGORITHM_LABEL: [(&str, &str); 6] = [
    ("none", "no encryption"),
    ("rc4-40", "40-bit RC4, weak encryption"),
    ("rc4-128", "128-bit RC4, weak encryption"),
    ("aes-128", "128-bit AES"),
    ("aes-256", "256-bit AES"),
    ("unknown", "an unrecognised encryption"),
];
/// R:2562.
pub const SECURITY_MODE_LABEL: [(&str, &str); 3] = [
    ("keep", "keeping the current encryption"),
    ("none", "no encryption"),
    ("aes-256", "AES-256"),
];
/// R:2568.
pub const PERMISSION_TITLE: &str = "This document does not permit changes";

/// R:3509.
pub const ARROW_KEYS: [&str; 4] = ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"];
/// R:3789.
pub const RUN_DELETE_TEXT: &str = "Text runs cannot be deleted; deselect them first or use Cut";

/// R:3841: clipboard refusals in words, per kind first; `content.refused[].detail` stays the bare token.
pub const CLIPBOARD_DETAIL_TEXT: [(&str, &str); 14] = [
    ("too-large", "That selection is too large to copy or paste."),
    ("empty-clipboard", "Nothing to paste"),
    ("not-candidate", "That selection cannot be copied."),
    ("not-candidate:image", "That image cannot be copied."),
    ("not-candidate:form", "That form cannot be copied."),
    (
        "not-candidate:text",
        "That text run cannot be copied (a run inside a Form, a run that continues a line, or a Type 3 font).",
    ),
    (
        "not-candidate:field",
        "That field cannot be copied (a signature, scripted, list or nested field).",
    ),
    ("not-candidate:link", "That link cannot be copied."),
    ("not-candidate:mark", "That redaction box cannot be copied."),
    ("out-of-bounds", "The pasted items do not fit on this page."),
    (
        "unbalanced-content",
        "This page's drawing instructions are unbalanced; nothing was pasted.",
    ),
    (
        "too-many:mark",
        "No more redaction boxes can be added to this file",
    ),
    (
        "too-many:field",
        "This form is too large to design fields in.",
    ),
    (
        "no-such-page",
        "Every link's page is missing from this document; nothing was pasted.",
    ),
];
/// R:3927.
pub const CLIP_KIND_ORDER: [&str; 6] = ["image", "form", "text", "field", "link", "mark"];
/// R:3928.
pub const CLIP_KIND_WORD: [(&str, &str); 6] = [
    ("image", "image"),
    ("form", "form"),
    ("text", "text run"),
    ("field", "field"),
    ("link", "link"),
    ("mark", "redaction box"),
];
/// R:3929.
pub const MODE_LABEL: [(&str, &str); 4] = [
    ("edit", "Edit text"),
    ("forms", "Fill forms"),
    ("annotate", "Annotate"),
    ("redact", "Redact"),
];

/// R:4645: half the FreeText line height (Helv 12 × 1.2 / 2).
pub const TEXTBOX_HALF_LINE: f64 = 7.2;
/// R:4646.
pub const TEXTBOX_ROTATED: &str = "A text box cannot be placed on a rotated page";
/// R:4751.
pub const SIGN_ROTATED: &str = "A signature cannot be placed on a rotated page";
/// R:4752.
pub const SIGN_MIN_WIDTH: f64 = 24.0;
/// R:4779: a 65th pad stroke is refused; the whole drawing stops at 4096 points.
pub const MAX_PAD_STROKES: usize = 64;
/// R:4779.
pub const MAX_PAD_POINTS: usize = 4096;
/// R:4780.
pub const PAD_FULL: &str = "The drawing is too long; press Clear and draw it again";
/// R:4806.
pub const SIGN_IMAGE_ONE: &str = "A signature uses one image; pick a single file.";
/// R:4806.
pub const SIGN_IMAGE_LARGE: &str = "That image is larger than this app will read.";
/// R:4807.
pub const SIGN_IMAGE_UNREADABLE: &str = "That image could not be read.";
/// R:4807.
pub const SIGN_IMAGE_NOT: &str = "That image is not a PNG or JPEG this app can read.";
/// R:5234.
pub const ADD_ROTATED_TEXT: &str = "Text cannot be added on a rotated page (unsupported-transform)";

/// R:5664: one `edit-text` refusal token in words.
pub const CONTENT_DETAIL_TEXT: [(&str, &str); 15] = [
    (
        "stale-target",
        "This page changed since the list was scanned; refresh and try again.",
    ),
    ("not-candidate", "That text cannot be edited yet."),
    (
        "unsupported-encoding",
        "The replacement has characters this font cannot draw.",
    ),
    (
        "overflow",
        "The replacement exceeds the available free space.",
    ),
    (
        "tail-would-move",
        "Following text would move past the available free-space limit.",
    ),
    (
        "unmeasurable-width",
        "This run's width cannot be measured in this version.",
    ),
    (
        "ambiguous-text-association",
        "That text cannot be told apart from another run on the page.",
    ),
    ("too-large", "That text is too long."),
    (
        "read-only",
        "This document does not permit changes to its content.",
    ),
    ("out-of-bounds", "That box must stay inside the page."),
    (
        "invalid-image",
        "That file is not a PNG or JPEG this app can read.",
    ),
    ("stale-preview", "The page changed; preview again."),
    (
        "write-failed",
        "The whole set was not written; nothing changed.",
    ),
    (
        "invalid-selection",
        "That selection cannot be edited together.",
    ),
    (
        "invalid-transform",
        "That rotation or flip is not one the editor offers.",
    ),
];

/// R:5683: why a discovered target is not editable, in the words the panel shows.
pub const CONTENT_REASON_TEXT: [(&str, &str); 14] = [
    (
        "subset-font",
        "The font is incomplete: only letters it already has on this page can be typed",
    ),
    (
        "shared-or-unproven-sharing",
        "This content is shared with other places and cannot be edited here yet",
    ),
    (
        "unsupported-graphics-state",
        "Hidden, clipped or unusually drawn text cannot be edited",
    ),
    (
        "ambiguous-text-association",
        "The same text is drawn twice here, so it cannot be edited",
    ),
    (
        "unsupported-encoding",
        "This font's letters cannot be read, so the text cannot be edited",
    ),
    (
        "unsupported-text-operator",
        "This text is drawn in a way the editor does not support yet",
    ),
    ("type3", "Decorative or vertical text cannot be edited"),
    (
        "vertical-writing",
        "Decorative or vertical text cannot be edited",
    ),
    (
        "rtl-or-shaping",
        "Right-to-left or joined-script text cannot be edited yet",
    ),
    (
        "unsupported-transform",
        "Skewed, nested or fully clipped content cannot be edited yet",
    ),
    (
        "unsupported-image",
        "A picture with a mask or layer cannot be replaced yet",
    ),
    (
        "shared-placement",
        "This object is used in several places: edit one placement at a time",
    ),
    (
        "unresolved-resource",
        "This kind of object cannot be edited",
    ),
    (
        "unsupported-xobject",
        "This kind of object cannot be edited",
    ),
];

/// R:6490: the ops a redaction log names; without it any unsaved change counts.
pub const REDACT_LOG_OPS: [&str; 6] = [
    "redact-add",
    "redact-set",
    "redact-search",
    "undo",
    "redo",
    "clip-copy",
];

/// R:7276: row label for each editable annotation type.
pub const ANNOT_LABELS: [(&str, &str); 11] = [
    ("Text", "Note"),
    ("Highlight", "Highlight"),
    ("Underline", "Underline"),
    ("StrikeOut", "Strikethrough"),
    ("FreeText", "Text box"),
    ("Square", "Rectangle"),
    ("Circle", "Ellipse"),
    ("Line", "Line"),
    ("Ink", "Drawing"),
    ("Stamp", "Stamp"),
    ("Redact", "Redaction box"),
];

/// R:7549: the U6 refusal tokens in the words the panel shows.
pub const FIELD_DETAIL_TEXT: [(&str, &str); 9] = [
    ("xfa", "This is an XFA form; fields cannot be added."),
    ("scripted", "Runs a script this app does not execute."),
    ("signature", "A signed field cannot be changed."),
    ("duplicate-name", "A field with that name already exists."),
    ("invalid-name", "Names are 1–128 characters without dots."),
    ("too-many", "This form is too large to design fields in."),
    ("stale-target", "That field is no longer on this page."),
    // §8.1: the field's own read-only flag, told apart from the document-level refusal.
    ("field-read-only", "This field is read-only"),
    ("invalid", "That change is not possible on this field."),
];

/// R:11908.
pub const PRINT_REASON_TEXT: [(&str, &str); 3] = [
    ("busy", "Another print job is already running."),
    ("failed", "The document could not be printed."),
    ("no-printer", "No printer is available."),
];

/// A `Record<string, string>` table lookup (`TABLE[key]`).
pub fn lookup(table: &[(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

// ------------------------------------------------------------------ recovery.ts

/// V:126: what the renderer may hold for recovery (base, last saved copy and log together).
pub const RECOVERY_BUDGET_BYTES: usize = 256 * 1024 * 1024;
/// V:128: the budget for a phone or tablet WebView.
pub const MOBILE_RECOVERY_BUDGET_BYTES: usize = 96 * 1024 * 1024;
/// V:130: desktops that report 4 GiB of memory or less keep half the desktop budget.
pub const SMALL_RECOVERY_BUDGET_BYTES: usize = 128 * 1024 * 1024;
/// V:147: a replay may take this long plus twice what its changes took the first time.
pub const REPLAY_BASE_MS: f64 = 30_000.0;
/// V:153: a replay closes this many times at most to reach the document generation.
pub const MAX_REPLAY_CLOSES: i64 = 10_000;

/// V:37: every engine op, classified: (op, kind, label for the restart notice, runs with the 120 s limit).
pub const OPS: [(&str, OpKind, &str, bool); 70] = [
    ("open", OpKind::Base, "Opening the document", false),
    ("close", OpKind::Base, "Closing the document", false),
    (
        "authenticate",
        OpKind::Unlock,
        "Unlocking the document",
        false,
    ),
    ("render", OpKind::Read, "Showing the page", false),
    ("thumbnail", OpKind::Read, "Drawing a page thumbnail", false),
    ("search", OpKind::Read, "Searching", false),
    ("save", OpKind::Read, "Saving", true),
    ("validate", OpKind::Read, "Checking the saved file", true),
    ("saved", OpKind::Mutate, "Recording the save", false),
    (
        "security-set",
        OpKind::Mutate,
        "Changing the security settings",
        false,
    ),
    ("annots", OpKind::Read, "Reading the annotations", false),
    ("annot-add", OpKind::Mutate, "Adding the annotation", false),
    ("annot-set", OpKind::Mutate, "Editing the annotation", false),
    ("annot-move", OpKind::Mutate, "Moving the annotation", false),
    (
        "annot-delete",
        OpKind::Mutate,
        "Deleting the annotation",
        false,
    ),
    (
        "redact-add",
        OpKind::Mutate,
        "Marking an area for redaction",
        false,
    ),
    (
        "redact-set",
        OpKind::Mutate,
        "Changing a redaction box",
        false,
    ),
    (
        "redact-search",
        OpKind::Mutate,
        "Marking search results for redaction",
        false,
    ),
    (
        "redact-preview",
        OpKind::Read,
        "Previewing the redactions",
        false,
    ),
    (
        "redact-apply",
        OpKind::Mutate,
        "Applying the redactions",
        true,
    ),
    (
        "redact-inspect",
        OpKind::Read,
        "Showing what a redaction box covers",
        false,
    ),
    (
        "sign-prepare",
        OpKind::Read,
        "Preparing the signature",
        true,
    ),
    ("sign-list", OpKind::Read, "Reading the signatures", false),
    ("page-rotate", OpKind::Mutate, "Rotating the page", false),
    ("page-delete", OpKind::Mutate, "Deleting the page", false),
    ("page-move", OpKind::Mutate, "Moving the page", false),
    ("fields", OpKind::Read, "Reading the form fields", false),
    (
        "field-set",
        OpKind::Mutate,
        "Filling in the form field",
        false,
    ),
    ("field-add", OpKind::Mutate, "Adding the form field", false),
    (
        "field-props",
        OpKind::Mutate,
        "Changing the form field",
        false,
    ),
    (
        "field-delete",
        OpKind::Mutate,
        "Deleting the form field",
        false,
    ),
    ("outline", OpKind::Read, "Reading the bookmarks", false),
    ("outline-add", OpKind::Mutate, "Adding the bookmark", false),
    (
        "outline-set",
        OpKind::Mutate,
        "Renaming the bookmark",
        false,
    ),
    (
        "outline-delete",
        OpKind::Mutate,
        "Deleting the bookmark",
        false,
    ),
    ("links", OpKind::Read, "Reading the links", false),
    ("link-add", OpKind::Mutate, "Adding the link", false),
    ("link-delete", OpKind::Mutate, "Deleting the link", false),
    ("link-set", OpKind::Mutate, "Changing the link", false),
    ("insert-pages", OpKind::Mutate, "Inserting the pages", true),
    ("extract-pages", OpKind::Read, "Extracting the pages", true),
    (
        "pdf-from-images",
        OpKind::Read,
        "Making a PDF from the images",
        false,
    ),
    (
        "scan-compose",
        OpKind::Read,
        "Making the scanned PDF",
        false,
    ),
    (
        "scan-measure",
        OpKind::Read,
        "Measuring the scanned text",
        false,
    ),
    ("text-all", OpKind::Read, "Exporting the text", false),
    (
        "compare-open",
        OpKind::Compare,
        "Opening the document to compare",
        false,
    ),
    (
        "compare-close",
        OpKind::Compare,
        "Closing the comparison",
        false,
    ),
    (
        "compare-render",
        OpKind::Compare,
        "Showing the compared page",
        false,
    ),
    ("compare-text", OpKind::Compare, "Comparing the text", false),
    (
        "stamp",
        OpKind::Mutate,
        "Adding the watermark or header",
        false,
    ),
    ("batch-file", OpKind::Read, "Processing a batch file", true),
    ("labels", OpKind::Read, "Reading the page labels", false),
    (
        "labels-set",
        OpKind::Mutate,
        "Setting the page labels",
        false,
    ),
    (
        "labels-clear",
        OpKind::Mutate,
        "Clearing the page labels",
        false,
    ),
    ("layers", OpKind::Read, "Reading the layers", false),
    (
        "layer-set",
        OpKind::Mutate,
        "Showing or hiding the layer",
        false,
    ),
    ("page-ids", OpKind::Read, "Reading the pages", false),
    (
        "edit-targets",
        OpKind::Read,
        "Finding the editable content",
        false,
    ),
    ("edit-text", OpKind::Mutate, "Editing the text", false),
    (
        "edit-paragraph",
        OpKind::Mutate,
        "Editing the paragraph",
        false,
    ),
    ("text-add", OpKind::Mutate, "Adding the text", false),
    ("edit-object", OpKind::Mutate, "Editing the object", false),
    ("edit-objects", OpKind::Mutate, "Editing the objects", false),
    (
        "edit-replace-preview",
        OpKind::Read,
        "Previewing the replacement",
        false,
    ),
    (
        "edit-replace-apply",
        OpKind::Mutate,
        "Replacing the text",
        false,
    ),
    ("clip-copy", OpKind::Clipboard, "Copying", false),
    ("clip-paste", OpKind::Mutate, "Pasting", false),
    ("edit-batch", OpKind::Mutate, "Editing the selection", false),
    ("undo", OpKind::Mutate, "Undo", false),
    ("redo", OpKind::Mutate, "Redo", false),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables() {
        assert_eq!(op_mode("redact-page"), Some(UiMode::Redact));
        assert_eq!(op_mode("zzz"), None);
        assert_eq!(mode_tab(UiMode::Organize), None);
        assert_eq!(
            lookup(&PRINT_REASON_TEXT, "busy"),
            Some("Another print job is already running.")
        );
        assert!(
            OPS.iter().any(|(op, kind, _, long)| *op == "redact-apply"
                && *kind == OpKind::Mutate
                && *long)
        );
    }
}
