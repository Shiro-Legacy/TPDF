//! The status line and the notice bar (renderer.ts R:11771-11815, R:11956): `setStatus`,
//! `showError`, `showMessage`, `clearError`, `failP0`, `messageOf`.
//!
//! X7/X8: the bar shows `userText(text)` (plain words, no internal names), coloured by kind;
//! only `error` is red. STATE `error` keeps the text as given (`notice_raw`; agents and tests
//! read it; success is never in it) and STATE `message` is what the bar shows.

use super::Session;
use super::dom_ids::{ERROR_EL, STATUS_EL, WARNING_EL};
use crate::engine::Refusal;
use crate::messages::{MessageKind, user_text};
use serde_json::json;

impl Session {
    /// R:11771: the status line speaks plainly too (X7).
    pub fn set_status(&mut self, text: &str) {
        self.dom.set_text(STATUS_EL, user_text(text));
    }

    /// R:11782. M9 review: once the restart cap stopped the worker, no later failure
    /// replaces that notice.
    pub fn show_error(&mut self, text: &str) {
        if self
            .stopped_notice
            .as_deref()
            .is_some_and(|stopped| stopped != text)
        {
            return;
        }
        self.show_message(text, MessageKind::Error);
    }

    /// An error whose STATE text (`raw`, which tests and agents read) differs from what people
    /// see (`shown`, plain words with the file's name and a next step).
    pub fn show_error_as(&mut self, raw: &str, shown: &str) {
        self.show_error(raw);
        if self.notice_raw == raw && !self.dom.hidden(ERROR_EL) {
            self.dom.set_text(ERROR_EL, shown);
        }
    }

    /// R:11789 (X8): a notice that is not a failure (success, information) or a handled one
    /// (warning); never red.
    pub fn show_message(&mut self, text: &str, kind: MessageKind) {
        if self
            .stopped_notice
            .as_deref()
            .is_some_and(|stopped| stopped != text)
        {
            return;
        }
        self.notice_raw = text.to_string();
        self.notice_kind = kind;
        self.dom.set_attr(ERROR_EL, "data-kind", kind.as_str());
        self.dom.set_attr(
            ERROR_EL,
            "role",
            if kind == MessageKind::Error {
                "alert"
            } else {
                "status"
            },
        );
        self.dom.set_hidden(ERROR_EL, false);
        self.dom.set_text(ERROR_EL, user_text(text));
    }

    /// R:11799.
    pub fn clear_error(&mut self) {
        self.dom.set_hidden(ERROR_EL, true);
        self.dom.set_text(ERROR_EL, "");
        self.notice_raw.clear();
        self.notice_kind = MessageKind::Error;
        self.dom.remove_attr(ERROR_EL, "data-kind");
    }

    /// R:11807: the startup open failed; the window shows no document and says why.
    pub fn fail_p0(&mut self, text: &str) {
        if self.p0_failed {
            return;
        }
        self.p0_failed = true;
        self.reset_document();
        self.set_status("Failed");
        self.show_error(text);
        self.console_line("P0_ERROR", json!({ "message": text }));
    }

    /// R:11758: the warning bar under the notice (`#warning`, not in STATE).
    pub fn show_warning(&mut self, warning: Option<&str>) {
        let Some(warning) = warning else {
            return;
        };
        self.dom.set_hidden(WARNING_EL, false);
        self.dom.set_text(
            WARNING_EL,
            if warning == "source-changed" {
                "The original file changed on disk after it was opened; this copy is from the version shown here. Your original was not modified."
            } else {
                "The copy was written, but the folder could not be flushed to disk; the copy may not survive a power loss."
            },
        );
    }

    /// R:11766.
    pub fn clear_warning(&mut self) {
        self.dom.set_hidden(WARNING_EL, true);
        self.dom.set_text(WARNING_EL, "");
    }

    /// R:11956 `messageOf(err)`: an engine refusal's (or a thrown error's) message.
    pub fn message_of(&self, err: &Refusal) -> String {
        err.message.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_kinds_and_state_error() {
        let mut s = Session::new();
        s.show_message("Saved x.pdf", MessageKind::Success);
        assert!(!s.dom.hidden(ERROR_EL));
        assert_eq!(s.notice_raw, "Saved x.pdf");
        assert_eq!(s.dom.attr(ERROR_EL, "role").as_deref(), Some("status"));
        s.clear_error();
        assert!(s.dom.hidden(ERROR_EL));
        assert_eq!(s.notice_kind, MessageKind::Error);
        assert!(s.dom.attr(ERROR_EL, "data-kind").is_none());
        s.stopped_notice = Some("stopped".into());
        s.show_error("other");
        assert!(s.dom.hidden(ERROR_EL));
        s.show_error("stopped");
        assert_eq!(s.notice_raw, "stopped");
    }
}
