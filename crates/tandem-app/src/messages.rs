//! The message catalogue: an exact port of `src/renderer/messages.ts`.
//!
//! Tandem PDF's words to the person using it (UX audit X7/X8, docs/rust-and-ux-plan.md §3.2).
//! The rules every entry follows:
//! - Plain language. No internal operation names ("sign-place:"), no error tokens
//!   ("unsupported-encoding"), no byte counts or "MiB", no PDF jargon, and never "refused".
//! - An error says what happened, then what to do next.
//! - Success and information are never shown as an error: each message has a kind,
//!   and only `error` is drawn in red.
//!
//! Layout: one `pub mod` per `MSG` section (`MSG.save.saved(name)` →
//! `messages::save::saved(name)`); a TS string is a `pub const` in SCREAMING case
//! (`MSG.general.desktopOnly` → `general::DESKTOP_ONLY`), a TS function a `pub fn` in
//! snake case. A TS default parameter is an `Option` (or a plain `bool` whose `false`
//! is the default). The output is byte-for-byte the TypeScript's.
//!
//! As in the TS, every string a catalogue *function* returns is remembered (the last 256),
//! and [`user_text`] never rewrites a remembered or fixed catalogue text, even when a value
//! it carries (a file named "open: x.pdf") looks like an internal prefix (re-review fix 5).
//! Helpers the TS calls directly inside the catalogue (not through `MSG.`) do not
//! remember; their `*_raw` twins here keep that difference.

use crate::js::{is_js_space, is_word_char, js_num, js_round, js_trim};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use tandem_protocol::limits::{MAX_PDF_BYTES, MAX_REDACT_SOURCE_BYTES, MAX_SAVE_BYTES};

/// `error` is red; `warning` amber (something went wrong but was handled); `success` green; `info` neutral.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MessageKind {
    #[default]
    Error,
    Warning,
    Success,
    Info,
}

impl MessageKind {
    /// The TS token.
    pub fn as_str(self) -> &'static str {
        match self {
            MessageKind::Error => "error",
            MessageKind::Warning => "warning",
            MessageKind::Success => "success",
            MessageKind::Info => "info",
        }
    }
    pub fn parse(token: &str) -> Option<Self> {
        match token {
            "error" => Some(MessageKind::Error),
            "warning" => Some(MessageKind::Warning),
            "success" => Some(MessageKind::Success),
            "info" => Some(MessageKind::Info),
            _ => None,
        }
    }
}

/// "printing, copying and filling forms".
pub fn listing<S: AsRef<str>>(items: &[S]) -> String {
    let items: Vec<&str> = items.iter().map(AsRef::as_ref).collect();
    if items.len() < 2 {
        return items.join("");
    }
    format!(
        "{} and {}",
        items[..items.len() - 1].join(", "),
        items[items.len() - 1]
    )
}

/// A size limit as people read it: whole megabytes.
pub fn megabytes(bytes: f64) -> String {
    format!("{} MB", js_num(js_round(bytes / (1024.0 * 1024.0))))
}

/// A native message box's contents (Electron `showMessageBox` options).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MessageBox {
    /// `type` ("question", "warning"), when the TS sets one.
    pub kind: Option<&'static str>,
    pub message: String,
    pub detail: Option<String>,
    pub buttons: Vec<String>,
    pub default_id: Option<usize>,
    pub cancel_id: Option<usize>,
}

// ------------------------------------------------------------------ the remembered texts

/// The texts the catalogue's functions have produced (oldest first, at most 256).
static PRODUCED: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

fn remember(text: String) -> String {
    let mut produced = PRODUCED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(at) = produced.iter().position(|t| *t == text) {
        produced.remove(at);
    }
    produced.push_back(text.clone());
    if produced.len() > 256 {
        produced.pop_front();
    }
    text
}

fn remember_opt(text: Option<String>) -> Option<String> {
    text.map(remember)
}

fn produced_has(text: &str) -> bool {
    PRODUCED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .any(|t| t == text)
}

/// The catalogue's fixed texts: every string value in `MSG`, nested objects included
/// (arrays, such as button lists, are not walked, as in the TS `track`).
fn fixed_texts() -> &'static [String] {
    static FIXED: OnceLock<Vec<String>> = OnceLock::new();
    FIXED.get_or_init(|| {
        let mut out: Vec<String> = [
            general::DESKTOP_ONLY,
            general::UNEXPECTED,
            general::NOT_PERMITTED,
            general::READ_ONLY,
            open::NO_DOCUMENT,
            open::SAMPLE,
            save::CANCELLED,
            save::NOTHING_TO_SAVE,
            save::RUNNING,
            save::READ_ONLY,
            filled::DETAIL,
            filled::OVERWRITE,
            filled::NEW_FILE,
            filled::CANCEL,
            banner::LOCKED,
            banner::SIGNED_READ_ONLY,
            banner::READ_ONLY,
            banner::XFA,
            banner::OPEN_ACTION,
            banner::REPAIRED,
            annotate::NOTE_EMPTY,
            annotate::NO_SELECTION,
            annotate::NO_MATCH,
            sign::CHOOSE_CERTIFICATE,
            sign::NEEDS_DOCUMENT,
            sign::NO_DRAWING,
            sign::ROTATED,
            sign::INVALID_INPUT,
            sign::SECURITY_PENDING,
            // MSG.sign.notForSigning.type
            "warning",
            sign::NOT_FOR_SIGNING_MESSAGE,
            sign::PREPARING,
            verify::PARTIAL,
            verify::WHOLE,
            redact::REDACT_CHIP,
            redact::CONFIRM_TEXT,
            redact::SAME_FILE_QUESTION.heading,
            redact::SAME_FILE_QUESTION.text,
            redact::SAME_FILE_QUESTION.yes,
            redact::OTHERS_QUESTION.heading,
            redact::OTHERS_QUESTION.text,
            redact::PENDING_DETAIL,
            redact::SIGNED_QUESTION.heading,
            redact::SIGNED_QUESTION.text,
            redact::SIGNED_QUESTION.yes,
            redact::USAGE_RIGHTS_QUESTION.heading,
            redact::USAGE_RIGHTS_QUESTION.text,
            redact::USAGE_RIGHTS_QUESTION.yes,
            redact::OVER_REDACTED,
            redact::ADDED,
            redact::REMOVED_BOX,
            redact::DRAW_FIRST,
            redact::DRAW_HINT,
            redact::STOPPED,
            redact::ORIGINAL_NOT_SAVED,
            redact::ORIGINAL_UNREADABLE,
            main::DISCARD_MESSAGE,
            main::DISCARD_DETAIL,
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        out.extend(
            redact::SANITIZE_LABELS
                .iter()
                .map(|(_, label)| label.to_string()),
        );
        out.extend(save::reason_table().into_iter().map(|(_, text)| text));
        out.push(redact::same_file());
        out
    })
}

/// Whether [`user_text`] leaves `text` alone because the catalogue produced it.
pub fn is_catalogue_text(text: &str) -> bool {
    fixed_texts().iter().any(|t| t == text) || produced_has(text)
}

// ------------------------------------------------------------------ redaction (docs/rust-and-ux-plan.md §2.3)

/// The redaction tool's words (redact-ui.ts re-exports them under their old names; main's close prompt uses them too).
pub mod redact {
    use super::{MAX_REDACT_SOURCE_BYTES, js_num, remember, remember_opt};

    /// The nine hidden-information switches, in plain words (they sit under "Details").
    /// The same plain names as the mobile app (owner decision); the `links` switch also
    /// removes document scripts (engine c21cec8).
    pub const SANITIZE_LABELS: [(&str, &str); 9] = [
        ("info", "Author, title and other document properties"),
        ("xmp", "Extra document details saved by other apps"),
        ("attachments", "Attached files"),
        ("outlines", "Bookmarks"),
        ("annots", "Other comments and notes, on every page"),
        (
            "links",
            "Web and file addresses in links, and scripts that run when the file opens",
        ),
        ("fields", "Everything typed into form fields"),
        ("layers", "Layer names"),
        ("struct", "Descriptions written for screen readers"),
    ];

    /// `SANITIZE_LABELS[key]`.
    pub fn sanitize_label(key: &str) -> Option<&'static str> {
        SANITIZE_LABELS
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, label)| *label)
    }

    fn plural(count: i64, one: &str, many: Option<&str>) -> String {
        let many = many
            .map(str::to_string)
            .unwrap_or_else(|| format!("{one}s"));
        format!("{count} {}", if count == 1 { one } else { &many })
    }

    /// What sends the document out while boxes are pending (redact-ui.ts `PendingAction`).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub enum PendingAction {
        #[default]
        Save,
        Export,
        Print,
        Sign,
    }

    impl PendingAction {
        pub fn as_str(self) -> &'static str {
            match self {
                PendingAction::Save => "save",
                PendingAction::Export => "export",
                PendingAction::Print => "print",
                PendingAction::Sign => "sign",
            }
        }
        pub fn parse(token: &str) -> Option<Self> {
            match token {
                "save" => Some(PendingAction::Save),
                "export" => Some(PendingAction::Export),
                "print" => Some(PendingAction::Print),
                "sign" => Some(PendingAction::Sign),
                _ => None,
            }
        }
    }

    /// The way out main's pending-boxes question is asked for (`"close" | "open"`).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub enum WindowAction {
        #[default]
        Close,
        Open,
    }

    /// A yes/no question's fixed words.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct QuestionText {
        pub heading: &'static str,
        pub text: &'static str,
        pub yes: &'static str,
    }

    /// A yes/no question's words, built from values.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct Question {
        pub heading: String,
        pub text: String,
        pub yes: String,
    }

    impl From<QuestionText> for Question {
        fn from(q: QuestionText) -> Self {
            Question {
                heading: q.heading.to_string(),
                text: q.text.to_string(),
                yes: q.yes.to_string(),
            }
        }
    }

    /// A heading and a sentence (OTHERS_QUESTION).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct HeadingText {
        pub heading: &'static str,
        pub text: &'static str,
    }

    /// The pending-boxes question's words for one way out.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct PendingWords {
        pub detail: &'static str,
        pub go: &'static str,
    }

    /// main's pending-boxes question: `[redact, go on, cancel]`.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct PendingQuestion {
        pub message: String,
        pub detail: String,
        pub buttons: [String; 3],
    }

    /// `bannerText`'s optional facts.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct BannerExtra {
        pub flattened: Option<Vec<i64>>,
        pub unsigned: bool,
        pub usage_rights: bool,
        pub over_redacted: bool,
    }

    pub const REDACT_CHIP: &str = "Drag over anything you want removed. Esc to stop.";

    pub(super) fn areas_text_raw(count: i64) -> String {
        plural(count, "area", None)
    }
    pub(super) fn boxes_text_raw(count: i64) -> String {
        plural(count, "box", Some("boxes"))
    }
    pub fn areas_text(count: i64) -> String {
        remember(areas_text_raw(count))
    }
    pub fn boxes_text(count: i64) -> String {
        remember(boxes_text_raw(count))
    }
    pub fn panel_heading(count: i64) -> String {
        remember(format!("Redactions ({count})"))
    }
    pub fn redact_button_text(count: i64) -> String {
        remember(if count > 0 {
            format!("Redact {}…", areas_text_raw(count))
        } else {
            "Redact areas…".to_string()
        })
    }
    pub fn count_text(boxes: i64, pages: i64) -> String {
        remember(if boxes > 0 {
            format!(
                "{} on {}",
                boxes_text_raw(boxes),
                plural(pages, "page", None)
            )
        } else {
            "No boxes yet".to_string()
        })
    }
    pub fn confirm_heading(count: i64) -> String {
        remember(format!(
            "Permanently remove everything inside {}?",
            boxes_text_raw(count)
        ))
    }
    pub const CONFIRM_TEXT: &str = "Text, pictures, drawings, comments and form fields inside the boxes are deleted from the file, not just covered. The result is saved as a new file; your original is not changed.";
    /// The engine also removed content outside the boxes because it is shared (a reused picture or drawing, a hidden layer, a comment).
    pub const OVER_REDACTED: &str = "Some content outside your boxes was also removed, because it is shared with other parts of the document. Check the copy.";

    fn banner_text_raw(name: &str, count: i64, extra: &BannerExtra) -> String {
        let flat = match &extra.flattened {
            Some(pages) if !pages.is_empty() => {
                let one = pages.len() == 1;
                format!(
                    " {} {} turned into {}.",
                    pages_name_raw(pages, true),
                    if one { "was" } else { "were" },
                    if one { "an image" } else { "images" }
                )
            }
            _ => String::new(),
        };
        let unsigned = if !extra.unsigned {
            ""
        } else if extra.usage_rights {
            " The copy no longer has Adobe Reader's special permissions."
        } else {
            " The copy has no signatures."
        };
        let over = if extra.over_redacted {
            format!(" {OVER_REDACTED}")
        } else {
            String::new()
        };
        format!(
            "You are viewing the redacted copy '{name}'. {} removed and checked.{flat}{unsigned}{over}",
            areas_text_raw(count)
        )
    }
    pub fn banner_text(name: &str, count: i64, extra: &BannerExtra) -> String {
        remember(banner_text_raw(name, count, extra))
    }

    pub(super) fn pages_name_raw(pages: &[i64], capital: bool) -> String {
        let list: Vec<String> = pages
            .iter()
            .map(|page| js_num(*page as f64 + 1.0))
            .collect();
        let name = if list.len() == 1 {
            format!("page {}", list[0])
        } else {
            // `list.at(-1)` of an empty list is undefined.
            let last = list
                .last()
                .cloned()
                .unwrap_or_else(|| "undefined".to_string());
            format!(
                "pages {} and {last}",
                list[..list.len().saturating_sub(1)].join(", ")
            )
        };
        if capital {
            let mut chars = name.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => name,
            }
        } else {
            name
        }
    }
    /// "page 4" / "pages 2, 5 and 7" from 0-based pages; `capital` starts it with "Page".
    pub fn pages_name(pages: &[i64], capital: bool) -> String {
        remember(pages_name_raw(pages, capital))
    }
    /// R4: a box on the shown page covers artwork that can only go by turning the page into an image.
    pub fn flatten_note(page: i64) -> String {
        remember(format!(
            "A box covers artwork that can't be cut precisely: redacting will turn page {} into an image.",
            page + 1
        ))
    }
    fn pending_text_raw(count: i64) -> String {
        format!(
            "{} {} marked but not redacted yet.",
            areas_text_raw(count),
            if count == 1 { "is" } else { "are" }
        )
    }
    pub fn pending_text(count: i64) -> String {
        remember(pending_text_raw(count))
    }
    /// §2.3 step 7, asked by main before the window closes or another file replaces it while boxes are
    /// unsaved: `memory` boxes (a signed file's) live only while the file is open. Buttons: [redact, go on, cancel].
    pub fn pending_question(count: i64, memory: bool, action: WindowAction) -> PendingQuestion {
        let verb = if action == WindowAction::Close {
            "Closing"
        } else {
            "Opening another file"
        };
        PendingQuestion {
            message: pending_text_raw(count),
            detail: if memory {
                format!(
                    "These boxes are kept only while the file is open. {verb} now discards them. Nothing has been removed from the file."
                )
            } else {
                format!(
                    "{verb} now discards the boxes and any other unsaved changes. Nothing has been removed from the file."
                )
            },
            buttons: [
                "Redact now…".to_string(),
                if action == WindowAction::Close {
                    "Close without redacting"
                } else {
                    "Discard the boxes"
                }
                .to_string(),
                "Cancel".to_string(),
            ],
        }
    }
    pub const SAME_FILE_QUESTION: QuestionText = QuestionText {
        heading: "That would replace your original.",
        text: "The redacted copy must be a new file. Choose a different name.",
        yes: "Choose another name…",
    };
    pub const OTHERS_QUESTION: HeadingText = HeadingText {
        heading: "You have other unsaved changes.",
        text: "They will be in the redacted copy, but not in your original.",
    };
    pub const PENDING_DETAIL: &str = "Saving now keeps the boxes as marks. Everything under them stays in the file until you redact it.";
    fn size_limit_text_raw(bytes: Option<f64>) -> String {
        let bytes = bytes.unwrap_or(MAX_REDACT_SOURCE_BYTES as f64);
        let mb = (bytes / 1e5).floor() / 10.0;
        format!(
            "Redaction works on files up to {} MB in this version.",
            js_num(mb)
        )
    }
    /// §2.4: the size ceiling in decimal MB, rounded down so the stated size always works
    /// (`None` = MAX_REDACT_SOURCE_BYTES).
    pub fn size_limit_text(bytes: Option<f64>) -> String {
        remember(size_limit_text_raw(bytes))
    }
    pub fn flatten_question(pages: &[i64]) -> Question {
        let name = pages_name_raw(pages, false);
        let one = pages.len() == 1;
        Question {
            heading: format!(
                "{} {} artwork that can't be cut precisely.",
                pages_name_raw(pages, true),
                if one { "contains" } else { "contain" }
            ),
            text: format!(
                "Redact {name} by turning {} into an image? {} text will no longer be selectable.",
                if one { "it" } else { "them" },
                if one { "Its" } else { "Their" }
            ),
            yes: format!("Turn {name} into an image"),
        }
    }
    pub const SIGNED_QUESTION: QuestionText = QuestionText {
        heading: "Redacting removes the signatures.",
        text: "Make an unsigned redacted copy? The original keeps its signatures.",
        yes: "Make unsigned copy",
    };
    /// A reader-extended file (usage rights, no actual signature): the engine refuses it as signed too.
    pub const USAGE_RIGHTS_QUESTION: QuestionText = QuestionText {
        heading: "This file has special permissions from Adobe Reader.",
        text: "Redacting removes them. Make a redacted copy without them? The original keeps them.",
        yes: "Make redacted copy",
    };
    /// A refused apply in plain words; `None` keeps the engine's own sentence. `pages` are 0-based.
    /// `hidden_kept`: some hidden information was kept, so a failed raw check may be text left in
    /// comments, form fields or properties (R5 decision). TS defaults: `pages = []`, `hidden_kept = false`.
    pub fn refusal_text(
        reason: Option<&str>,
        pages: &[i64],
        detail: Option<&str>,
        hidden_kept: bool,
    ) -> Option<String> {
        remember_opt(refusal_text_raw(reason, pages, detail, hidden_kept))
    }
    fn refusal_text_raw(
        reason: Option<&str>,
        pages: &[i64],
        detail: Option<&str>,
        hidden_kept: bool,
    ) -> Option<String> {
        let text = match reason? {
            "too-large" => format!("{} This file is larger.", size_limit_text_raw(None)),
            "read-only" => "This document can't be changed, so it can't be redacted.".into(),
            "no-match" => "That text was not found in this document.".into(),
            "verify-failed" if detail == Some("too-complex") => "This document is too complex for Tandem to check that everything inside your boxes is gone, so nothing was saved. Try redacting fewer pages at a time, or turn the pages into images first.".into(),
            "verify-failed" if detail == Some("raw") && hidden_kept => "Some of the text inside your boxes is still in the document's comments, form fields or properties, or elsewhere on the page, so nothing was saved. Turn on \"Also remove hidden information, other comments and form entries\" and try again; if it still fails, use a larger box or redact the whole page.".into(),
            "verify-failed" => {
                let place = if pages.is_empty() { String::new() } else { format!(" on {}", pages_name_raw(pages, false)) };
                format!("The redacted copy did not pass its final check{place}, so nothing was saved. Try a larger box, or redact the whole page.")
            }
            "needs-flatten" => "A box covers artwork that can't be cut precisely. Redact again and agree to turn that page into an image.".into(),
            "signed" => "Redacting removes the signatures. Redact again and agree to make an unsigned copy.".into(),
            "scan-budget" => "This page is too complex to check box by box. Redact the whole page instead.".into(),
            "too-many" => "There are too many boxes (2000 at most). Use one larger box or \"Redact whole page\".".into(),
            _ => return None,
        };
        Some(text)
    }
    /// The pending-boxes question's words for each way out ([Redact now…] stays first and default).
    pub fn pending_words(action: PendingAction, memory: bool) -> PendingWords {
        match action {
            PendingAction::Export => PendingWords {
                detail: "An exported page or its text still shows everything under the boxes until you redact.",
                go: "Export anyway",
            },
            PendingAction::Print => PendingWords {
                detail: "A printed page still shows everything under the boxes until you redact.",
                go: "Print anyway",
            },
            // A signed file's boxes live in memory only, and the signed copy opens in place of this file (D6).
            PendingAction::Sign if memory => PendingWords {
                detail: "These boxes are kept only while the file is open. The signed copy will not have them, and it opens in place of this file.",
                go: "Sign anyway",
            },
            PendingAction::Sign => PendingWords {
                detail: "The signed copy keeps the boxes as marks, with everything under them, until you redact.",
                go: "Sign anyway",
            },
            PendingAction::Save if memory => PendingWords {
                detail: "These boxes are kept only while the file is open; a saved copy will not have them.",
                go: "Save without the boxes",
            },
            PendingAction::Save => PendingWords {
                detail: PENDING_DETAIL,
                go: "Save marks only",
            },
        }
    }

    // What the status line and notices say (MSG.redact).
    pub const ADDED: &str = "Added a box. Nothing is removed until you redact.";
    pub fn added_page(page: i64) -> String {
        remember(format!(
            "Added a box over all of page {page}. Nothing is removed until you redact."
        ))
    }
    pub fn added_search(added: i64, text: &str) -> String {
        let boxes = if added == 1 {
            "1 box".to_string()
        } else {
            format!("{added} boxes")
        };
        remember(format!(
            "Added {boxes} over “{text}”. Text inside pictures, form fields and comments is not found: look over each page, or draw boxes."
        ))
    }
    pub const REMOVED_BOX: &str = "Removed a redaction box";
    pub const DRAW_FIRST: &str = "Draw a box first: drag over anything you want removed.";
    pub const DRAW_HINT: &str = "Drag a box over anything you want removed";
    pub const STOPPED: &str = "Stopped redacting";
    /// TS default `over_redacted = false`.
    pub fn saved_kept(name: &str, over_redacted: bool) -> String {
        let over = if over_redacted {
            format!(" {OVER_REDACTED}")
        } else {
            String::new()
        };
        remember(format!(
            "Saved the redacted copy {name}. This window still shows the original with its boxes.{over}"
        ))
    }
    pub fn saved_opened(name: &str) -> String {
        remember(format!("Saved and opened the redacted copy {name}."))
    }
    /// TS default `over_redacted = false`.
    pub fn saved(name: &str, over_redacted: bool) -> String {
        let over = if over_redacted {
            format!(" {OVER_REDACTED}")
        } else {
            String::new()
        };
        remember(format!("Saved the redacted copy {name}.{over}"))
    }
    /// `MSG.redact.sameFile` (a fixed string built from SAME_FILE_QUESTION).
    pub fn same_file() -> String {
        format!(
            "{} {} Nothing was saved.",
            SAME_FILE_QUESTION.heading, SAME_FILE_QUESTION.text
        )
    }
    pub fn not_saved(reason: &str) -> String {
        let next = super::save::new_file(reason)
            .unwrap_or_else(|| "Try again, or choose another folder.".to_string());
        remember(format!("The redacted copy was not saved. {next}"))
    }
    pub const ORIGINAL_NOT_SAVED: &str = "Not redacted: the original was not saved.";
    pub const ORIGINAL_UNREADABLE: &str =
        "The original file could not be opened. It may have been moved or deleted.";
    /// The pending-boxes question was answered Cancel.
    pub fn cancelled(action: PendingAction) -> String {
        let what = match action {
            PendingAction::Export => "Export",
            PendingAction::Print => "Print",
            PendingAction::Sign => "Signing",
            PendingAction::Save => "Save",
        };
        remember(format!("{what} cancelled"))
    }
}

pub mod general {
    pub const DESKTOP_ONLY: &str = "This needs the Tandem PDF desktop app.";
    pub const UNEXPECTED: &str = "Something went wrong. Try again; if it keeps happening, save a copy of your work and reopen the file.";
    pub const NOT_PERMITTED: &str = "This document's settings do not allow that change. You can still read, print or save a copy.";
    pub const READ_ONLY: &str = "This document is read-only, so it cannot be changed. Save a copy to keep your own version.";
}

pub mod open {
    use super::{MAX_PDF_BYTES, megabytes, remember};

    /// TS default `limit = MAX_PDF_BYTES` (`None`).
    pub fn too_large(name: &str, limit: Option<f64>) -> String {
        let limit = limit.unwrap_or(MAX_PDF_BYTES as f64);
        remember(format!(
            "{name} is larger than {}, the most Tandem PDF can open. Try a smaller file.",
            megabytes(limit)
        ))
    }
    pub fn unreadable(name: &str) -> String {
        remember(format!(
            "{name} could not be opened. It may be damaged or not a PDF. Check the file and try again."
        ))
    }
    pub fn not_pdf(name: &str) -> String {
        remember(format!(
            "{name} is not a PDF this app can read. Choose a PDF file."
        ))
    }
    /// Walkthrough open#4: the file is gone from where it was.
    pub fn missing(name: &str, was_recent: bool) -> String {
        remember(format!(
            "{name} can't be found. It may have been moved, renamed or deleted.{}",
            if was_recent {
                " It has been removed from Recent Files; open it again from its new place."
            } else {
                " Open it again from its new place."
            }
        ))
    }
    /// The file changed or went away between choosing it and reading it.
    pub fn changed(name: &str) -> String {
        remember(format!(
            "{name} changed or was moved while it was being opened. Try opening it again."
        ))
    }
    /// Walkthrough open#14: what to do next.
    pub const WRONG_PASSWORD: &str = "That password isn't right. Passwords are case-sensitive: check Caps Lock and try again, or open another file.";
    pub const NO_DOCUMENT: &str = "Open a PDF first.";
    pub const SAMPLE: &str = "The sample document is built into the app and cannot be saved. Open or create a PDF to save your work.";
}

pub mod save {
    use super::{MAX_PDF_BYTES, MAX_SAVE_BYTES, megabytes, remember, remember_opt};

    pub fn saved(name: &str) -> String {
        remember(format!("Saved {name}."))
    }
    pub fn saved_copy(name: &str) -> String {
        remember(format!(
            "Saved a copy as {name}. The window still shows the original."
        ))
    }
    pub fn saved_as(name: &str) -> String {
        remember(format!("Saved as {name}. The window now edits {name}."))
    }
    pub const CANCELLED: &str = "Save cancelled.";
    pub const NOTHING_TO_SAVE: &str = "No changes to save.";
    pub const RUNNING: &str = "A save is already running. Wait for it to finish.";
    pub const READ_ONLY: &str =
        "This document cannot be changed, so it cannot be saved over. Use Save As… to keep a copy.";
    /// After a restart, the save that was being written landed.
    pub fn late_shown(name: &str) -> String {
        remember(format!(
            "Saved {name}. The window now shows the saved file."
        ))
    }
    pub fn late_kept(name: &str) -> String {
        remember(format!(
            "Saved {name} from before the restart. The window holds newer changes: Save writes them over {name}."
        ))
    }

    /// `MSG.save.reason`: why a save did not happen; every reason main can answer with
    /// (tests/m10-save-failure.test.mjs). In TS order.
    pub fn reason_table() -> Vec<(&'static str, String)> {
        let fixed = |key: &'static str, text: &str| (key, text.to_string());
        vec![
            fixed(
                "no-document",
                "There is no document open to save. Open or create a PDF first.",
            ),
            fixed(
                "read-only",
                "This document is signed or certified, so it cannot be saved in this version. Your file on disk is unchanged.",
            ),
            fixed(
                "encrypted",
                "This document is password-protected, and saving a protected copy is not supported yet. Your file on disk is unchanged.",
            ),
            fixed(
                "invalid",
                "The saved file did not read back correctly, so nothing was written. Your file on disk is unchanged; try again.",
            ),
            fixed(
                "worker",
                "The PDF engine stopped responding, so nothing was written. Try again.",
            ),
            fixed("cancelled", "Save cancelled."),
            (
                "too-large",
                format!(
                    "Tandem PDF can open documents up to {}, but can save at most {}. Remove some pages or images and try again.",
                    megabytes(MAX_PDF_BYTES as f64),
                    megabytes(MAX_SAVE_BYTES as f64)
                ),
            ),
            fixed(
                "same-file",
                "That would overwrite the open document. Choose a different name.",
            ),
            fixed(
                "document-changed",
                "The open document changed while saving, so nothing was written. Save again.",
            ),
            fixed(
                "source-changed",
                "The file changed on disk since you opened it, so it was not overwritten. Use Save As… to save your version under another name.",
            ),
            fixed(
                "disk-full",
                "There is not enough disk space to save. The file on disk was not changed; free some space and try again.",
            ),
            fixed(
                "permission",
                "The file or its folder is read-only, or Tandem PDF is not allowed to write there. The file on disk was not changed; use Save As… to save somewhere else.",
            ),
            fixed(
                "unreadable",
                "The file could not be written. The file on disk was not changed; try again or save somewhere else.",
            ),
            fixed(
                "write-failed",
                "The file could not be written. The file on disk was not changed; try again or save somewhere else.",
            ),
            fixed(
                "busy",
                "Another save is already running. Wait for it to finish.",
            ),
            fixed(
                "broken-link",
                "That name is a link that does not lead to a file, so nothing was written. Choose another name.",
            ),
        ]
    }
    /// `MSG.save.reason[key]` (`SAVE_REASON_TEXT[key]` in renderer.ts).
    pub fn reason(key: &str) -> Option<String> {
        reason_table()
            .into_iter()
            .find(|(k, _)| *k == key)
            .map(|(_, text)| text)
    }
    /// A new file (export, a PDF from images, a signed copy) could not be written.
    pub fn new_file(reason: &str) -> Option<String> {
        remember_opt(match reason {
            "permission" => Some(
                "The folder or file is read-only, or Tandem PDF is not allowed to write there. Choose another folder."
                    .to_string(),
            ),
            "disk-full" | "broken-link" => self::reason(reason),
            _ => None,
        })
    }
}

/// UX item 8: the first save of a filled form asks once where it goes.
pub mod filled {
    use super::{MessageBox, remember};

    pub fn question(name: &str) -> String {
        remember(format!("Save over “{name}” or as a new file?"))
    }
    pub const DETAIL: &str = "You filled in this form. Saving over it replaces the original; a new file keeps the original empty.";
    pub const OVERWRITE: &str = "Save Over";
    pub const NEW_FILE: &str = "Save as New File…";
    pub const CANCEL: &str = "Cancel";
    /// `MSG.filled.box`: the native question main shows: Return (the default) never overwrites
    /// the original; Esc cancels.
    pub fn message_box(name: &str) -> MessageBox {
        MessageBox {
            kind: Some("question"),
            buttons: vec![
                OVERWRITE.to_string(),
                NEW_FILE.to_string(),
                CANCEL.to_string(),
            ],
            default_id: Some(1),
            cancel_id: Some(2),
            message: question(name),
            detail: Some(DETAIL.to_string()),
        }
    }
}

/// The notice under the toolbar that describes the open file (§3.3 Open).
pub mod banner {
    use super::{listing, remember};

    pub const LOCKED: &str =
        "This file is password-protected. Enter its password below to open it.";
    pub fn unlocked<S: AsRef<str>>(all: bool, allowed: &[S], weak: bool) -> String {
        let head = if all {
            "This file is password-protected; it is unlocked and you can change it.".to_string()
        } else {
            format!(
                "This file is password-protected; its settings allow only {}.",
                if allowed.is_empty() {
                    "reading".to_string()
                } else {
                    listing(allowed)
                }
            )
        };
        let tail = if weak {
            " It uses old, weak encryption: File ▸ Security… can protect it better."
        } else {
            ""
        };
        remember(format!("{head}{tail}"))
    }
    /// Round 2: says what to do to change it (the copy), not only how to check it.
    pub const SIGNED_READ_ONLY: &str = "This file is signed, so it opens for reading: editing it would break the signatures. To change it, make an editable copy without the signatures; the original stays as it is.";
    pub const READ_ONLY: &str = "This file is read-only, so it opens for reading.";
    pub const XFA: &str =
        "This form is an Adobe dynamic form; its fields may not all show or fill here.";
    pub fn attachments(count: i64) -> String {
        remember(format!(
            "It has {count} attached file{}.",
            if count == 1 { "" } else { "s" }
        ))
    }
    pub const OPEN_ACTION: &str = "It asks to run an action when opened; Tandem PDF never runs it.";
    pub const REPAIRED: &str =
        "This file was damaged and has been repaired. Check it before saving.";
}

pub mod annotate {
    use super::remember;

    pub const NOTE_EMPTY: &str = "Note discarded: it had no text.";
    pub const NO_SELECTION: &str =
        "Select some text on the page first, or search for it, then choose Highlight.";
    pub const NO_MATCH: &str = "Search for the text first, then choose Highlight match.";
    pub fn highlighted_all(count: i64) -> String {
        remember(format!(
            "Highlighted {count} match{}.",
            if count == 1 { "" } else { "es" }
        ))
    }
}

pub mod sign {
    use super::{MessageBox, remember};

    pub const CHOOSE_CERTIFICATE: &str = "Choose a certificate first.";
    pub const NEEDS_DOCUMENT: &str = "Open a PDF first, then sign it.";
    pub fn signed_opened(name: &str) -> String {
        remember(format!(
            "Signed and saved as {name}. The signed copy is now open."
        ))
    }
    pub const NO_DRAWING: &str = "Draw or choose your signature first.";
    pub const ROTATED: &str =
        "A signature cannot be placed on a rotated page. Rotate the page back first.";
    /// Why a certificate could not be used. An empty `status` counts as none (JS falsy).
    pub fn certificate(reason: &str, status: Option<&str>) -> String {
        if let Some(status) = status.filter(|s| !s.is_empty()) {
            return remember(format!(
                "This certificate cannot be used: {status}. Choose another certificate."
            ));
        }
        // Phase 2 (the signing worker, crates/tandem-sign): one plain sentence per reason a certificate is refused.
        let text = match reason {
            "bad-password" => "The certificate password is incorrect. Type it again.",
            "unsupported-key" => {
                "This certificate's key cannot sign here. Use an RSA key of 2048 bits or more, or a P-256 or P-384 key."
            }
            "certificate-expired" => {
                "This certificate is not valid today. Choose another certificate."
            }
            "no-certificate" => "Choose your certificate file again, then sign.",
            "unreadable" => {
                "This file could not be read as a certificate. Choose a .p12, .pfx or .pem file that holds your key and certificate."
            }
            "too-costly" => {
                "This certificate file is protected in a way that takes too long to open. Export it again with standard settings."
            }
            "cancelled" => "Not signed.",
            "busy" => "Another save is running. Wait for it to finish, then sign again.",
            _ => "This certificate cannot be used. Choose another certificate.",
        };
        remember(text.to_string())
    }
    /// Why writing the signed copy failed.
    pub fn failed(reason: &str, status: Option<&str>) -> String {
        if matches!(
            reason,
            "certificate-expired" | "unsupported-key" | "bad-password" | "too-costly"
        ) {
            return remember(certificate(reason, status));
        }
        let text = match reason {
            "cancelled" => Some("Signing cancelled: no signed copy was written."),
            "invalid" => Some(
                "The signature could not be made: the signing time or details look wrong. Check this computer's date and time, then sign again.",
            ),
            "unreadable" => Some(
                "The signature could not be made with this certificate. Choose the certificate again, enter its password, and sign again.",
            ),
            "too-large" => Some("This document is too large to sign in this version."),
            "too-many" => {
                Some("This document has too many signatures to sign again in this version.")
            }
            "busy" => Some("Another save is running. Wait for it to finish, then sign again."),
            "no-certificate" => {
                Some("Choose a certificate and enter its password again, then sign.")
            }
            _ => None,
        };
        let text = match text {
            Some(text) => text.to_string(),
            None => super::save::new_file(reason).unwrap_or_else(|| {
                "The signed copy could not be written. Try again or choose another folder."
                    .to_string()
            }),
        };
        remember(text)
    }
    pub const INVALID_INPUT: &str =
        "Enter a page that exists in this document, and a reason of at most 200 characters.";
    pub const SECURITY_PENDING: &str = "You changed this document's protection (Security…) and have not saved it yet. Save the document first, then sign it.";
    pub const NOT_FOR_SIGNING_MESSAGE: &str = "This certificate isn't meant for signing documents, so other apps may not trust the signature. Sign anyway?";
    /// `MSG.sign.notForSigning`: main asks before signing with a certificate whose key usage
    /// excludes document signing (Phase 2).
    pub fn not_for_signing() -> MessageBox {
        MessageBox {
            kind: Some("warning"),
            buttons: vec!["Sign anyway".to_string(), "Cancel".to_string()],
            default_id: Some(1),
            cancel_id: Some(1),
            message: NOT_FOR_SIGNING_MESSAGE.to_string(),
            detail: None,
        }
    }
    pub const PREPARING: &str = "Signing…";
}

/// "Make an editable copy" of a signed file (walkthrough open#12, sign.md).
pub mod editable_copy {
    use super::redact::QuestionText;
    use super::remember;

    /// The banner button.
    pub const BUTTON: &str = "Make an editable copy…";
    pub fn suggested(stem: &str) -> String {
        format!("{stem} (editable).pdf")
    }
    pub fn opened(name: &str) -> String {
        remember(format!(
            "Opened an editable copy, {name}. The signatures were removed; the original is unchanged."
        ))
    }
    /// The copy opened but asks for the original's password first (review fix 7).
    pub fn opened_locked(name: &str, boxes: usize) -> String {
        let marks = match boxes {
            0 => String::new(),
            1 => " The area you marked comes back once it is open.".to_string(),
            n => format!(" The {n} areas you marked come back once it is open."),
        };
        remember(format!(
            "Opened an editable copy, {name}. The signatures were removed; the original is unchanged. The copy has the original's password: enter it to edit.{marks}"
        ))
    }
    /// The boxes from the signed file, back on the unlocked copy.
    pub fn carried(count: usize) -> String {
        if count == 1 {
            "The area you marked on the signed file is marked on the copy".to_string()
        } else {
            format!("The {count} areas you marked on the signed file are marked on the copy")
        }
    }
    /// The copy was written but the window could not switch to it.
    pub fn saved(name: &str) -> String {
        remember(format!(
            "Saved an editable copy, {name}. The signatures were removed; the original is unchanged. Open it to edit it."
        ))
    }
    pub const NOT_SIGNED: &str = "This file has no signatures, so you can edit it as it is.";
    pub const NEEDS_DOCUMENT: &str = "Open a signed PDF first, then make an editable copy.";
    pub const SAME_FILE_QUESTION: QuestionText = QuestionText {
        heading: "That would replace the signed original.",
        text: "The editable copy must be a new file. Choose a different name.",
        yes: "Choose another name…",
    };
    pub fn same_file() -> String {
        remember(format!(
            "{} {} Nothing was saved.",
            SAME_FILE_QUESTION.heading, SAME_FILE_QUESTION.text
        ))
    }
    /// Why the copy was not made or not saved.
    pub fn failed(reason: &str) -> String {
        let next = match reason {
            "too-large" => "This file is too large to copy in this version.".to_string(),
            "encrypted" => "Enter the file's password first, then try again.".to_string(),
            _ => super::save::new_file(reason)
                .unwrap_or_else(|| "Try again, or choose another folder.".to_string()),
        };
        remember(format!("The editable copy was not saved. {next}"))
    }
}

/// The Verify signatures dialog (§3.3 Sign: "Verify speaks plainly").
pub mod verify {
    use super::remember;
    use chrono::{
        DateTime, Datelike, FixedOffset, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc,
    };

    pub fn certified(level: i64) -> String {
        remember(
            match level {
                1 => "Certified: the author allows no changes.",
                2 => "Certified: the author allows filling in the form and signing.",
                3 => "Certified: the author allows filling in the form, signing and comments.",
                _ => "Certified by its author.",
            }
            .to_string(),
        )
    }

    /// `signedOn`'s `format` (for tests): a locale and an IANA time zone.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct DateFormat {
        pub locale: Option<String>,
        pub time_zone: Option<String>,
    }

    /// The Date cell: the signing time in this computer's language and time zone
    /// ("Oct 19, 2026, 2:36 AM"), or "No date". `claimed` (TS default true): the time is only
    /// what the signer wrote (no trusted timestamp), so it is "Dated …". `no_date` (TS default false).
    ///
    /// Port limits: `Intl.DateTimeFormat` is reproduced for `en-US` (the default) and `en-GB`
    /// only, and the time zone is the local one or `UTC` (no time-zone database here).
    pub fn signed_on(
        iso: Option<&str>,
        claimed: bool,
        no_date: bool,
        format: Option<&DateFormat>,
    ) -> String {
        let when = if no_date {
            None
        } else {
            iso.and_then(parse_js_date)
        };
        let Some(when) = when else {
            return remember("No date".to_string());
        };
        let utc = format
            .and_then(|f| f.time_zone.as_deref())
            .is_some_and(|tz| tz.eq_ignore_ascii_case("UTC"));
        let local: NaiveDateTime = if utc {
            when.naive_utc()
        } else {
            Local.from_utc_datetime(&when.naive_utc()).naive_local()
        };
        let british = format.and_then(|f| f.locale.as_deref()) == Some("en-GB");
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let month = MONTHS[local.month0() as usize];
        let text = if british {
            format!(
                "{} {month} {}, {:02}:{:02}",
                local.day(),
                local.year(),
                local.hour(),
                local.minute()
            )
        } else {
            let (pm, hour) = local.hour12();
            format!(
                "{month} {}, {}, {hour}:{:02} {}",
                local.day(),
                local.year(),
                local.minute(),
                if pm { "PM" } else { "AM" }
            )
        };
        remember(if claimed {
            format!("Dated {text}")
        } else {
            text
        })
    }

    /// `new Date(iso)` for the forms a signing time takes: RFC 3339 / ISO with an offset,
    /// an ISO date-time without one (local time), or a bare date (UTC). None = Invalid Date.
    fn parse_js_date(text: &str) -> Option<DateTime<Utc>> {
        if text.is_empty() {
            return None;
        }
        if let Ok(t) = DateTime::<FixedOffset>::parse_from_rfc3339(text) {
            return Some(t.with_timezone(&Utc));
        }
        for pattern in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"] {
            if let Ok(naive) = NaiveDateTime::parse_from_str(text, pattern) {
                return Local
                    .from_local_datetime(&naive)
                    .earliest()
                    .map(|t| t.with_timezone(&Utc));
            }
        }
        if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
            return date
                .and_hms_opt(0, 0, 0)
                .map(|naive| Utc.from_utc_datetime(&naive));
        }
        None
    }

    pub const PARTIAL: &str =
        "Part of the file: changes made after this signature were not checked";
    pub const WHOLE: &str = "The whole file";
    /// The Trust cell; `issuer` is the verifier's cleaned issuer name (empty = none).
    pub fn trust(trust: &str, issuer: Option<&str>) -> String {
        remember(match trust {
            "expired" => "certificate not valid at the time it claims to have signed".to_string(),
            "trusted" => {
                format!(
                    "trusted by {}",
                    issuer
                        .filter(|s| !s.is_empty())
                        .unwrap_or("a certificate you trust")
                )
            }
            _ => "not in your trusted list".to_string(),
        })
    }
    /// The reason notes a verdict carries, space-separated tokens, as words. `words` is the
    /// verifier's own plain phrase for them (crates/tandem-sign report.rs `reason_words`); used when non-empty.
    pub fn notes(tokens: &str, words: Option<&str>) -> String {
        if let Some(words) = words.filter(|w| !w.is_empty()) {
            return remember(words.to_string());
        }
        let text = tokens
            .split(' ')
            .filter(|t| !t.is_empty())
            .map(|token| match token {
                "no-signing-time" => "no signing time given".to_string(),
                "weak-digest" => "uses old SHA-1 checking".to_string(),
                "unknown-digest" => "uses an unknown checking method".to_string(),
                "changes-not-permitted" => {
                    "the file changed in ways its certification does not allow".to_string()
                }
                "missing-subfilter" => "the signature format is not stated".to_string(),
                "byte-range" => "the signed part of the file is not what it claims".to_string(),
                "digest" => "the document changed after it was signed".to_string(),
                "signature" => "the signature does not match the document".to_string(),
                "signer-cert" => "the signer's certificate is not valid".to_string(),
                "malformed" => "the signature cannot be read".to_string(),
                "no-signed-attrs" => "the signature format is not supported".to_string(),
                "signer-id" => "the signer cannot be identified".to_string(),
                "time-changed" => "the date was edited after signing".to_string(),
                "outdated-security" => "the certificate uses outdated security".to_string(),
                "not-for-signing" => "the certificate is not meant for signing".to_string(),
                "unsupported-ca-rule" => "the issuer's rules were not checked".to_string(),
                "content-type" => "not a document signature".to_string(),
                other => other.replace('-', " "),
            })
            .collect::<Vec<_>>()
            .join("; ");
        remember(text)
    }
}

pub mod main {
    use super::{MessageBox, remember};

    pub const DISCARD_MESSAGE: &str = "Do you want to save the changes you made?";
    pub const DISCARD_DETAIL: &str = "Your changes will be lost if you don't save them.";
    /// Walkthrough open#6: the Mac's Save, Don't Save, Cancel (the host's
    /// `discard_box` names the document).
    pub const DISCARD_BUTTONS: [&str; 3] = ["Save", "Don't Save", "Cancel"];
    /// `MSG.main.discard`.
    pub fn discard() -> MessageBox {
        MessageBox {
            kind: None,
            message: DISCARD_MESSAGE.to_string(),
            detail: Some(DISCARD_DETAIL.to_string()),
            buttons: DISCARD_BUTTONS.iter().map(|b| b.to_string()).collect(),
            default_id: None,
            cancel_id: None,
        }
    }
    pub fn open_link(uri: &str) -> String {
        remember(format!("Open {uri} in your browser?"))
    }
}

// ------------------------------------------------------------------ userText

/// Exact internal texts with a plain replacement (engine and worker refusals that reach the error bar).
const EXACT: [(&str, &str); 3] = [
    (
        "That edit was refused.",
        "That change could not be made. Undo or try a different change.",
    ),
    (
        "Draw a bigger link",
        "Drag a bigger rectangle to make a link.",
    ),
    (
        "Draw a bigger signature field",
        "Drag a bigger rectangle to make a signature field.",
    ),
];

/// The operation names engine and drive messages start with ("sign-place: …", "fill-text: …").
const INTERNAL_OPS: &str = "annot annot-add annot-delete annot-move annot-set annots arm assemble authenticate batch batch-file bookmark \
cert-sign-open circle click clip-copy clip-paste compare compare-close compare-open compare-render compare-text content content-edit \
crop decorate delete-page drag edit-batch edit-object edit-objects edit-paragraph edit-replace edit-replace-apply edit-replace-preview \
edit-targets edit-text encrypt encrypt-keep export export-jpeg export-png export-txt extract extract-pages field-add field-delete \
field-props field-set fields fill-text find freetext goto handle header-footer highlight highlight-all ink insert insert-pages key \
labels labels-clear labels-set layer-set layers line link link-add link-delete link-set links menu mode move-down move-up nav \
new-from-images open outline outline-add outline-delete outline-set outlines page-delete page-ids page-move page-retry page-rotate \
pages pdf-from-images redact redact-add redact-apply redact-preview redact-set redo render rotate-left rotate-right save saved \
scan-compose scan-measure search security security-set select shapes sign sign-draw sign-list sign-place sign-prepare square stamp \
strikeout tab text-add text-all thumbnail underline undo validate verify watermark zoom";

fn is_internal_op(name: &str) -> bool {
    INTERNAL_OPS.split(' ').any(|op| op == name)
}

fn char_before(text: &str, at: usize) -> Option<char> {
    text[..at].chars().next_back()
}

fn char_at(text: &str, at: usize) -> Option<char> {
    text[at..].chars().next()
}

/// `\b` before position `at` whose next char is a word char.
fn starts_word(text: &str, at: usize) -> bool {
    !char_before(text, at).is_some_and(is_word_char)
}

/// `\b` after a word char ending at `at`.
fn ends_word(text: &str, at: usize) -> bool {
    !char_at(text, at).is_some_and(is_word_char)
}

fn digits_end(text: &str, from: usize) -> usize {
    from + text[from..].bytes().take_while(u8::is_ascii_digit).count()
}

/// A global regex replace driven by a matcher: `matcher(text, at)` returns the end of a
/// match starting at `at` and its replacement.
fn replace_all(text: &str, matcher: impl Fn(&str, usize) -> Option<(usize, String)>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(c) = char_at(text, at) {
        if let Some((end, replacement)) = matcher(text, at) {
            out.push_str(&replacement);
            at = end;
        } else {
            out.push(c);
            at += c.len_utf8();
        }
    }
    out
}

/// `/\bWORD\b/g` → `replacement` (the word starts and ends with a word char).
fn replace_word(text: &str, word: &str, replacement: &str) -> String {
    replace_all(text, |t, at| {
        (t[at..].starts_with(word) && starts_word(t, at) && ends_word(t, at + word.len()))
            .then(|| (at + word.len(), replacement.to_string()))
    })
}

/// `/\b(\d+)([- ]?)(bytes?)\b/g`: a byte count of a megabyte or more is a size limit worth
/// saying in MB; smaller counts stay as they are.
fn replace_bytes(text: &str) -> String {
    replace_all(text, |t, at| {
        if !char_at(t, at).is_some_and(|c| c.is_ascii_digit()) || !starts_word(t, at) {
            return None;
        }
        let digits = digits_end(t, at);
        let mut k = digits;
        if matches!(char_at(t, k), Some('-' | ' ')) {
            k += 1;
        }
        let end = if t[k..].starts_with("bytes") && ends_word(t, k + 5) {
            k + 5
        } else if t[k..].starts_with("byte") && ends_word(t, k + 4) {
            k + 4
        } else {
            return None;
        };
        let n: f64 = t[at..digits].parse().unwrap_or(f64::NAN);
        let replacement = if n >= 1024.0 * 1024.0 {
            megabytes(n)
        } else {
            t[at..end].to_string()
        };
        Some((end, replacement))
    })
}

/// `/\b(\d+(?:\.\d+)?) MiB\b/g` → `"$1 MB"`.
fn replace_mib(text: &str) -> String {
    replace_all(text, |t, at| {
        if !char_at(t, at).is_some_and(|c| c.is_ascii_digit()) || !starts_word(t, at) {
            return None;
        }
        let mut k = digits_end(t, at);
        if t[k..].starts_with('.') && char_at(t, k + 1).is_some_and(|c| c.is_ascii_digit()) {
            k = digits_end(t, k + 1);
        }
        (t[k..].starts_with(" MiB") && ends_word(t, k + 4))
            .then(|| (k + 4, format!("{} MB", &t[at..k])))
    })
}

/// The internal words, and what people call them, in the TS `WORDS` order.
fn plain_words(text: &str) -> String {
    let mut text = replace_bytes(text);
    text = replace_mib(&text);
    for (word, replacement) in [
        ("ceiling", "limit"),
        ("was refused", "could not be done"),
        ("is refused", "is not possible"),
        ("are refused", "are not possible"),
        ("refused", "not possible"),
        ("Identity-H", "embedded"),
        ("copy-on-write", "a private copy"),
        // `\bincremental (?:update|save)\b` and `\bshow-text runs?\b`, one alternative at a time.
        ("incremental update", "signed save"),
        ("incremental save", "signed save"),
        ("show-text runs", "lines of text"),
        ("show-text run", "lines of text"),
    ] {
        text = replace_word(&text, word, replacement);
    }
    text
}

/// The error tokens a refusal can end with in parentheses ("… (unsupported-transform)").
fn is_suffix_token(token: &str) -> bool {
    let tail = |prefix: &str| {
        token.strip_prefix(prefix).is_some_and(|rest| {
            !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
        })
    };
    tail("unsupported-")
        || tail("invalid-")
        || matches!(
            token,
            "subset-font"
                | "stale-target"
                | "not-candidate"
                | "ambiguous-text-association"
                | "tail-would-move"
                | "shared-or-unproven-sharing"
                | "rtl-or-shaping"
                | "vertical-writing"
                | "write-failed"
                | "read-only"
                | "out-of-bounds"
                | "unresolved-resource"
                | "too-large"
        )
}

/// `text.replace(TOKEN_SUFFIX, "")`: a known token in parentheses at the very end (before an
/// optional final "."), with the space before it, is dropped.
fn drop_token_suffix(text: &str) -> String {
    let (body, dot) = match text.strip_suffix(").") {
        Some(body) => (body, "."),
        None => match text.strip_suffix(')') {
            Some(body) => (body, ""),
            None => return text.to_string(),
        },
    };
    let Some(open) = body.rfind(" (") else {
        return text.to_string();
    };
    if is_suffix_token(&body[open + 2..]) {
        format!("{}{dot}", &body[..open])
    } else {
        text.to_string()
    }
}

/// `/\s{2,}/g` → " ".
fn collapse_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    for c in text.chars() {
        if is_js_space(c) {
            run.push(c);
            continue;
        }
        flush_run(&mut out, &mut run);
        out.push(c);
    }
    flush_run(&mut out, &mut run);
    out
}

fn flush_run(out: &mut String, run: &mut String) {
    if run.chars().count() >= 2 {
        out.push(' ');
    } else {
        out.push_str(run);
    }
    run.clear();
}

/// `/^([a-z][a-z0-9]*(?:-[a-z0-9]+)*): (?=\S)/`: the prefix's name and its length.
fn op_prefix(text: &str) -> Option<(&str, usize)> {
    let colon = text.find(':')?;
    let name = &text[..colon];
    let well_formed = name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        });
    if !well_formed || !text[colon + 1..].starts_with(' ') {
        return None;
    }
    let next = char_at(text, colon + 2)?;
    (!is_js_space(next)).then_some((name, colon + 2))
}

/// The text a message shows. Catalogue text, and anything a person wrote or named, passes
/// through unchanged. Internal text (it starts with an operation name) loses that prefix, and
/// its byte counts and jargon are put into plain words; a known error token at the end is
/// dropped. Never empty (unless `raw` is).
pub fn user_text(raw: &str) -> String {
    let mut text = js_trim(raw).to_string();
    if text.is_empty() {
        return String::new();
    }
    if let Some((_, plain)) = EXACT.iter().find(|(from, _)| *from == text) {
        return plain.to_string();
    }
    if is_catalogue_text(&text) {
        return text; // catalogue text is already plain
    }
    // An engine message can arrive wrapped by a drive op ("content: edit-text: …"): every leading prefix goes.
    let mut internal = false;
    while let Some((name, len)) = op_prefix(&text) {
        if !is_internal_op(name) {
            break;
        }
        internal = true;
        text = text[len..].to_string();
    }
    if internal {
        text = plain_words(&text);
    }
    // Walkthrough view#5: a page number past the end, in people's numbering.
    if let Some(last) = text
        .to_lowercase()
        .strip_prefix("page must be an integer between 0 and ")
        .and_then(|n| n.trim_end_matches('.').trim().parse::<u64>().ok())
    {
        let pages = last + 1;
        return format!(
            "That page doesn't exist: this document has {pages} page{}.",
            if pages == 1 { "" } else { "s" }
        );
    }
    text = drop_token_suffix(&text);
    text = text
        .replace(" (window.pdf4all is unavailable)", "")
        .replace("(window.pdf4all is unavailable)", "");
    text = js_trim(&collapse_spaces(&text)).to_string();
    if text.is_empty() {
        return general::UNEXPECTED.to_string();
    }
    if internal {
        let mut chars = text.chars();
        if let Some(first) = chars.next() {
            // `text[0].toUpperCase()`: a character outside the BMP is a lone surrogate in JS and stays.
            let head: String = if (first as u32) > 0xFFFF {
                first.to_string()
            } else {
                first.to_uppercase().collect()
            };
            return format!("{head}{}", chars.as_str());
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_numbers_past_the_end() {
        assert_eq!(
            user_text("goto: page must be an integer between 0 and 2"),
            "That page doesn't exist: this document has 3 pages."
        );
    }

    #[test]
    fn helpers() {
        assert_eq!(
            listing(&["printing", "copying", "filling forms"]),
            "printing, copying and filling forms"
        );
        assert_eq!(listing(&["printing"]), "printing");
        assert_eq!(listing::<&str>(&[]), "");
        assert_eq!(megabytes(120.0 * 1024.0 * 1024.0), "120 MB");
        assert_eq!(megabytes(130_000_000.0), "124 MB");
    }

    #[test]
    fn user_text_cleans_internal_text_only() {
        assert_eq!(
            user_text("sign-place: no signature has been drawn"),
            "No signature has been drawn"
        );
        assert_eq!(user_text("fill-text: text is empty"), "Text is empty");
        assert_eq!(
            user_text("Text cannot be added on a rotated page (unsupported-transform)"),
            "Text cannot be added on a rotated page"
        );
        assert_eq!(
            user_text("save: 130000000 bytes is over the 125829120 byte save ceiling"),
            "124 MB is over the 120 MB save limit"
        );
        assert_eq!(
            user_text("open: PDF exceeds the 125829120 byte ceiling"),
            "PDF exceeds the 120 MB limit"
        );
        let sign_prepare = user_text("sign-prepare: incremental save changed source bytes");
        assert!(
            !sign_prepare.contains("sign-prepare") && !sign_prepare.contains("incremental"),
            "{sign_prepare}"
        );
        assert_eq!(
            user_text("That edit was refused."),
            "That change could not be made. Undo or try a different change."
        );
        assert_eq!(
            user_text("edit-text: longer text is refused per run"),
            "Longer text is not possible per run"
        );
        assert_eq!(
            user_text("Saving needs the desktop app (window.pdf4all is unavailable)."),
            "Saving needs the desktop app."
        );
        assert_eq!(user_text(""), "");
        assert_eq!(
            user_text("content: edit-text: the text is refused"),
            "The text is not possible"
        );
        assert_eq!(user_text("open: claims.pdf is gone"), "Claims.pdf is gone");
        assert_eq!(
            user_text("batch-file: too few bytes for a PDF (1500 bytes)"),
            "Too few bytes for a PDF (1500 bytes)"
        );
        assert_eq!(
            user_text("render: 2.5 MiB of copy-on-write and Identity-H"),
            "2.5 MB of a private copy and embedded"
        );
        assert_eq!(
            user_text("edit-text: two  spaces \tand show-text runs."),
            "Two spaces and lines of text."
        );
        assert_eq!(user_text("a\tb"), "a\tb");
        assert_eq!(user_text("stamp: (too-large)"), "(too-large)");
        assert_eq!(user_text("x (write-failed)."), "x.");
        assert_eq!(user_text("open: "), "open:");
        assert_eq!(user_text("Open: x"), "Open: x");
        assert_eq!(
            user_text("open: (window.pdf4all is unavailable)"),
            general::UNEXPECTED
        );
    }

    #[test]
    fn user_text_never_rewrites_what_a_person_wrote() {
        for text in [
            "Saved refused-claims.pdf.".to_string(),
            "invoice.pdf could not be opened. It may be damaged or not a PDF. Check the file and try again.".to_string(),
            "re: contract.pdf is larger than 120 MB, the most Tandem PDF can open. Try a smaller file.".to_string(),
            "Highlighted “the claim was refused (see p. 2)”".to_string(),
            "Note: 130000000 bytes per MiB, Identity-H, copy-on-write".to_string(),
            open::too_large("refused-claims.pdf", None),
            save::saved_as("re: contract.pdf"),
            sign::signed_opened("incremental update.pdf"),
        ] {
            assert_eq!(user_text(&text), text);
        }
        let named = open::too_large("open: claims.pdf", None);
        assert_eq!(user_text(&named), named);
        assert_eq!(
            user_text(&save::saved_as("save: 2026.pdf")),
            "Saved as save: 2026.pdf. The window now edits save: 2026.pdf."
        );
        let text = redact::saved_opened("redact: secret.pdf");
        assert_eq!(
            text,
            "Saved and opened the redacted copy redact: secret.pdf."
        );
        assert!(is_catalogue_text(&text));
        assert_eq!(user_text(&text), text);
        assert_eq!(
            user_text(&redact::added_page(3)),
            "Added a box over all of page 3. Nothing is removed until you redact."
        );
        assert!(is_catalogue_text(general::UNEXPECTED));
        assert!(is_catalogue_text(&save::reason("too-large").unwrap()));
    }

    #[test]
    fn redaction_words() {
        assert_eq!(redact::areas_text(1), "1 area");
        assert_eq!(redact::boxes_text(2), "2 boxes");
        assert_eq!(redact::redact_button_text(0), "Redact areas…");
        assert_eq!(redact::redact_button_text(3), "Redact 3 areas…");
        assert_eq!(redact::count_text(3, 1), "3 boxes on 1 page");
        assert_eq!(redact::count_text(0, 0), "No boxes yet");
        assert_eq!(redact::pages_name(&[1, 4, 6], false), "pages 2, 5 and 7");
        assert_eq!(redact::pages_name(&[3], true), "Page 4");
        assert_eq!(
            redact::size_limit_text(None),
            "Redaction works on files up to 12.5 MB in this version."
        );
        assert_eq!(
            redact::banner_text(
                "x.pdf",
                2,
                &redact::BannerExtra {
                    flattened: Some(vec![0]),
                    unsigned: true,
                    ..Default::default()
                }
            ),
            "You are viewing the redacted copy 'x.pdf'. 2 areas removed and checked. Page 1 was turned into an image. The copy has no signatures."
        );
        assert_eq!(
            redact::refusal_text(Some("verify-failed"), &[0, 1], Some("too-complex"), true)
                .unwrap(),
            "This document is too complex for Tandem to check that everything inside your boxes is gone, so nothing was saved. Try redacting fewer pages at a time, or turn the pages into images first."
        );
        assert!(
            redact::refusal_text(Some("verify-failed"), &[0], None, false)
                .unwrap()
                .contains("did not pass its final check on page 1")
        );
        assert_eq!(redact::refusal_text(Some("other"), &[], None, false), None);
        assert_eq!(redact::refusal_text(None, &[], None, false), None);
        assert_eq!(
            redact::sanitize_label("links"),
            Some("Web and file addresses in links, and scripts that run when the file opens")
        );
        let not_saved = redact::not_saved("write-failed");
        assert!(!not_saved.contains('(') && !not_saved.contains("write-failed"));
        assert!(redact::not_saved("disk-full").contains("not enough disk space"));
        let q = redact::flatten_question(&[2]);
        assert_eq!(
            q.heading,
            "Page 3 contains artwork that can't be cut precisely."
        );
        assert_eq!(q.yes, "Turn page 3 into an image");
        let p = redact::pending_question(2, true, redact::WindowAction::Open);
        assert_eq!(p.message, "2 areas are marked but not redacted yet.");
        assert_eq!(p.buttons[1], "Discard the boxes");
        assert_eq!(
            redact::cancelled(redact::PendingAction::Sign),
            "Signing cancelled"
        );
        assert_eq!(
            redact::same_file(),
            "That would replace your original. The redacted copy must be a new file. Choose a different name. Nothing was saved."
        );
    }

    #[test]
    fn other_sections() {
        assert_eq!(
            save::reason("too-large").unwrap(),
            "Tandem PDF can open documents up to 120 MB, but can save at most 50 MB. Remove some pages or images and try again."
        );
        assert_eq!(save::new_file("cancelled"), None);
        assert_eq!(
            banner::unlocked(false, &["printing", "copying"], true),
            "This file is password-protected; its settings allow only printing and copying. It uses old, weak encryption: File ▸ Security… can protect it better."
        );
        assert_eq!(
            banner::unlocked::<&str>(false, &[], false),
            "This file is password-protected; its settings allow only reading."
        );
        assert_eq!(banner::attachments(1), "It has 1 attached file.");
        assert_eq!(annotate::highlighted_all(2), "Highlighted 2 matches.");
        assert_eq!(
            sign::certificate("x", Some("revoked")),
            "This certificate cannot be used: revoked. Choose another certificate."
        );
        assert_eq!(
            sign::certificate("bad-password", Some("")),
            "The certificate password is incorrect. Type it again."
        );
        assert_eq!(
            sign::failed("too-costly", None),
            sign::certificate("too-costly", None)
        );
        assert_eq!(
            sign::failed("permission", None),
            "The folder or file is read-only, or Tandem PDF is not allowed to write there. Choose another folder."
        );
        assert_eq!(
            sign::failed("zzz", None),
            "The signed copy could not be written. Try again or choose another folder."
        );
        assert_eq!(
            verify::notes("weak-digest odd-thing", None),
            "uses old SHA-1 checking; odd thing"
        );
        assert_eq!(verify::notes("weak-digest", Some("plain")), "plain");
        assert_eq!(
            verify::trust("trusted", Some("")),
            "trusted by a certificate you trust"
        );
        let b = filled::message_box("a.pdf");
        assert_eq!(b.message, "Save over “a.pdf” or as a new file?");
        assert_eq!(b.default_id, Some(1));
    }

    #[test]
    fn signed_on_formats_like_intl() {
        let iso = "2026-10-19T06:36:20.000Z";
        let utc = verify::DateFormat {
            locale: None,
            time_zone: Some("UTC".into()),
        };
        assert_eq!(
            verify::signed_on(Some(iso), false, false, Some(&utc)),
            "Oct 19, 2026, 6:36 AM"
        );
        assert_eq!(
            verify::signed_on(Some(iso), true, false, Some(&utc)),
            "Dated Oct 19, 2026, 6:36 AM"
        );
        let gb = verify::DateFormat {
            locale: Some("en-GB".into()),
            time_zone: Some("UTC".into()),
        };
        assert_eq!(
            verify::signed_on(Some(iso), true, false, Some(&gb)),
            "Dated 19 Oct 2026, 06:36"
        );
        for missing in [None, Some(""), Some("not a date")] {
            assert_eq!(verify::signed_on(missing, true, false, None), "No date");
        }
        assert_eq!(verify::signed_on(Some(iso), true, true, None), "No date");
        let local = verify::signed_on(Some(iso), true, false, None);
        assert!(local.starts_with("Dated ") && local.contains("2026"));
        assert_eq!(user_text(&local), local);
    }
}
