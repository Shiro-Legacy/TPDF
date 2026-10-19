//! "Make an editable copy" (walkthrough open#12, sign.md): a signed or certified file opens
//! for reading; its banner offers a copy without the signatures. The engine writes it
//! (`unsigned-copy`: signature values, locks, certification and validation data go; form
//! values, comments and the empty signature fields stay), the save panel asks where (it
//! suggests "<name> (editable).pdf" and never the signed original itself), and the copy opens
//! in place of the original, which is not changed. A drive answers the save panel with the
//! line's `to`, as every save: `{"cmd":"editable-copy","to":"…"}`.
//!
//! Boxes drawn on the signed file (held in memory, since it could not take marks) are carried
//! over to the copy as ordinary boxes; a copy that opens locked (it keeps an encrypted
//! original's password) gets them once it is unlocked.

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
        // Review fix 7: the copy keeps the file's password, so an encrypted original's copy opens
        // locked; its boxes wait for the unlock instead of being dropped.
        let locked = opened && self.doc.as_ref().is_some_and(|d| d.info.needs_password);
        let text = if locked {
            let count = boxes.len();
            if count > 0 {
                self.carry_boxes = Some((saved.name.clone(), boxes));
            }
            words::opened_locked(&saved.name, count)
        } else if opened {
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

    /// After the editable copy was unlocked: the boxes made on the signed file come along.
    pub(crate) fn carry_boxes_over(&mut self) {
        let Some((name, boxes)) = self.carry_boxes.take() else {
            return;
        };
        let unlocked = self
            .doc
            .as_ref()
            .is_some_and(|d| d.name == name && !d.info.needs_password);
        if !unlocked {
            return;
        }
        let carried = boxes
            .into_iter()
            .filter(|b| self.add_redact_area(b.page.max(0) as usize, b.rect.clone(), ""))
            .count();
        if carried > 0 {
            self.announce(&words::carried(carried));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::session::Session;
    use serde_json::json;
    use tandem_pdf_engine::document::{Document, DocumentLimits};
    use tandem_pdf_engine::writer::{
        DocumentWriteLimits, Encryption, Entropy, WriteOptions, write_document,
    };
    use tandem_protocol::ui::UiAction;

    /// signed-intact.pdf rewritten with an AES-256 open password ("u").
    fn encrypted_signed() -> Vec<u8> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/p0b/signed-intact.pdf"
        );
        let signed = std::fs::read(path).unwrap();
        let doc = Document::open(&signed, DocumentLimits::default()).unwrap();
        let options = WriteOptions {
            encryption: Encryption::Aes256 {
                user_password: b"u",
                owner_password: b"o",
                permissions: 4 | 8 | 16 | 32 | 256,
            },
            entropy: Some(Entropy([9; 32])),
            ..WriteOptions::APP
        };
        write_document(&doc, DocumentWriteLimits::default(), options).unwrap()
    }

    #[test]
    fn a_locked_copy_gets_its_boxes_once_unlocked() {
        // Review fix 7: the copy of an encrypted signed file keeps its password and opens
        // locked; the boxes drawn on the signed file wait for the unlock.
        let dir = crate::host::util::mkdtemp(
            &crate::host::util::temp_dir().join("tandem-editable-copy-"),
        )
        .unwrap();
        let out = dir.join("copy.pdf");
        let answers = crate::platform::Answers {
            save_to: Some(out.clone()),
            ..Default::default()
        };
        let mut s = Session::for_drive(crate::host::Host::new(
            crate::host::HostConfig {
                user_data: dir.join("udata"),
                smoke: true,
                echo_stdout: false,
            },
            Box::new(crate::platform::Scripted::new(answers)),
        ));
        s.open_document("signed-enc.pdf", encrypted_signed(), None, false);
        s.ui_act(UiAction::Unlock {
            password: "u".into(),
        });
        assert!(s.signed_read_only(), "unlocked, signed and read-only");
        s.on_edit_request(
            "redact",
            Some(json!({ "page": 0, "rect": [60, 30, 360, 70] }).to_string()),
        );
        assert_eq!(s.redact_memory.len(), 1);
        assert!(s.make_editable_copy());
        let doc = s.doc.as_ref().unwrap();
        assert_eq!(doc.name, "copy.pdf");
        assert!(doc.info.needs_password, "the copy keeps the password");
        let text = s.ui_state().view.notice.map(|m| m.text).unwrap_or_default();
        assert!(text.contains("enter it to edit"), "{text}");
        assert!(text.contains("The area you marked comes back"), "{text}");
        s.ui_act(UiAction::Unlock {
            password: "u".into(),
        });
        assert!(!s.doc.as_ref().unwrap().info.needs_password);
        assert!(s.carry_boxes.is_none());
        assert_eq!(
            s.redact_view().map(|r| r.marks),
            Some(1),
            "the box came along"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
