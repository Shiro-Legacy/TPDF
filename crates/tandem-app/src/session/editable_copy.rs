//! "Make an editable copy" (walkthrough open#12, sign.md): a signed or certified file opens
//! for reading; its banner offers a copy without the signatures. The engine writes it
//! (`unsigned-copy`: signature values, locks, certification and validation data go; form
//! values, comments and the empty signature fields stay), the save panel asks where (it
//! suggests "<name> (editable).pdf" and never the signed original itself), and the copy opens
//! in place of the original, which is not changed. A drive answers the save panel with the
//! line's `to`, as every save: `{"cmd":"editable-copy","to":"…"}`.
//!
//! Boxes drawn on the signed file (held in memory, since it could not take marks) are carried
//! over to the copy as ordinary boxes.

use super::Session;
use crate::host::SaveKind;
use crate::messages::MessageKind;
use crate::messages::editable_copy as words;
use crate::safe_write::SaveFail;
use serde_json::json;

impl Session {
    /// Whether the open document is read-only because it is signed or certified.
    pub fn signed_read_only(&self) -> bool {
        self.doc
            .as_ref()
            .is_some_and(|d| !d.info.needs_password && (d.info.signatures > 0 || d.info.certified))
    }

    /// The banner's "Make an editable copy…"; `true` when the copy was written.
    pub fn make_editable_copy(&mut self) -> bool {
        let Some(source) = self.doc.as_ref().map(|d| d.name.clone()) else {
            self.show_error(words::NEEDS_DOCUMENT);
            return false;
        };
        if !self.signed_read_only() {
            self.show_error_as(
                "editable-copy: the document is not signed",
                words::NOT_SIGNED,
            );
            return false;
        }
        let mut reply = match self.request(json!({ "op": "unsigned-copy" }), Vec::new()) {
            Ok(reply) => reply,
            Err(err) => {
                let shown = words::failed(err.reason_str());
                self.show_error_as(&err.message, &shown);
                return false;
            }
        };
        let bytes = reply.take_bytes("bytes").unwrap_or_default();
        let suggested = words::suggested(&super::exports::file_stem(&source));
        let boxes = self.redact_memory.clone();
        // `redacted`: the copy never replaces the file it was made from, and main issues its open.
        let mut saved = self
            .host
            .save_new(&bytes, Some(&suggested), SaveKind::Pdf, false, true);
        while saved == Err(SaveFail::SameFile)
            && !self.headless
            && self.redact_ui_ask_yes_no(&words::SAME_FILE_QUESTION.into())
        {
            saved = self
                .host
                .save_new(&bytes, Some(&suggested), SaveKind::Pdf, false, true);
        }
        let saved = match saved {
            Ok(saved) => saved,
            Err(SaveFail::Cancelled) => return false,
            Err(SaveFail::SameFile) => {
                let shown = words::same_file();
                self.show_error_as("editable-copy: same-file", &shown);
                return false;
            }
            Err(fail) => {
                let shown = words::failed(fail.as_str());
                self.show_error_as(&format!("editable-copy: {}", fail.as_str()), &shown);
                return false;
            }
        };
        if let Some(url) = saved.url.clone() {
            self.open_bound(&saved.name, &url);
        }
        let opened =
            self.doc.as_ref().is_some_and(|d| d.name == saved.name) && !self.signed_read_only();
        let text = if opened {
            for b in boxes {
                self.add_redact_area(b.page.max(0) as usize, b.rect, "");
            }
            words::opened(&saved.name)
        } else {
            words::saved(&saved.name)
        };
        self.set_status(&text);
        // Information that stays until dismissed: it says what changed in the file.
        self.show_message(&text, MessageKind::Info);
        true
    }
}
