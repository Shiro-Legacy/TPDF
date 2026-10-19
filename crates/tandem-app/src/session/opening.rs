//! Lane 1: renderer.ts 2352–2626 (the tail of the "A2 drive ops" section): the help
//! dialog, the open path (`openFromUrl`, `openStartupFixture`, `openDocument`,
//! `emitResult`), the read-only and permission gates, and the document banner.
#![allow(dead_code)]

use super::Session;
use super::consts::SECURITY_MODE_LABEL;
use super::dom_ids as ids;
use super::shell::{DOCUMENT_TITLE, allowed, js_number, permission_names};
use super::types::{CompareAction, CompareLast, MessageKind, OpenDoc};
use crate::messages;
use serde_json::{Value, json};
use std::time::Instant;
use tandem_protocol::engine::{
    AuthLevel, DocumentInfo, OpenReply, PermissionName, RenderReply, SecurityMode, SecurityState,
};
use tandem_protocol::limits::MAX_PDF_BYTES;

/// The body rows of `#help-matrix` (index.html: one static table).
const HELP_ROWS: i64 = 11;

/// R:2516 `isReadOnly`.
pub fn is_read_only(info: &DocumentInfo) -> bool {
    info.signatures > 0 || info.certified || info.needs_password
}

/// R:2521 `stateFromReply`: the engine's security view when no `security-set` was made.
pub fn state_from_reply(info: &DocumentInfo, level: AuthLevel) -> SecurityState {
    SecurityState {
        mode: SecurityMode::Keep,
        algorithm: info.algorithm,
        permissions: if info.needs_password {
            Vec::new()
        } else {
            permission_names(&info.permissions)
        },
        auth: level,
        pending: false,
    }
}

/// `/^[A-Za-z0-9._-]+\.pdf$/`.
fn fixture_name_ok(name: &str) -> bool {
    name.len() > 4
        && name.ends_with(".pdf")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The render reply without its pixels (what the open path hands back and reports).
fn without_pixels(reply: &mut RenderReply) -> RenderReply {
    let pixels = std::mem::take(&mut reply.pixels);
    let copy = reply.clone();
    reply.pixels = pixels;
    copy
}

impl Session {
    /// R:2352 `helpOp`: opening an open dialog and closing a closed one are no-ops.
    pub(crate) fn help_op(&mut self, contents: Option<&str>) {
        if contents == Some("close") {
            self.close_dialog(ids::HELP_DIALOG);
            return;
        }
        if !self.dom.is_open(ids::HELP_DIALOG) {
            self.dom.set_open(ids::HELP_DIALOG, true);
        }
    }

    /// R:2361 `helpRows`: the matrix body rows, which is what the drive state reports.
    pub fn help_rows(&self) -> i64 {
        HELP_ROWS
    }

    /// R:2365 `openFromUrl`: the bytes behind main's one-shot URL, then `openDocument`.
    pub fn open_from_url(&mut self, name: &str, url: &str, untitled: bool) {
        let Some(bytes) = self.host.read_open(url) else {
            // main's protocol handler answers 403 for a token it does not hold (M:360).
            self.show_error(&format!("{name} could not be delivered (HTTP 403)."));
            return;
        };
        self.open_document(name, bytes, Some(url.to_string()), false);
        if untitled
            && let Some(doc) = self
                .doc
                .as_mut()
                .filter(|doc| doc.url.as_deref() == Some(url))
        {
            doc.untitled = Some(true);
        }
    }

    /// R:2380 `openStartupFixture`: `Some(name)` opens the packaged sample (smoke, drive,
    /// `--sample`) and logs `P0_RESULT`; `None` (main named no sample) shows the start screen.
    pub fn open_startup_fixture(&mut self, fixture: Option<&str>) {
        let started = Instant::now();
        self.set_status("Opening the sample…");
        let Some(fixture) = fixture else {
            self.show_start_screen();
            return;
        };
        if let Err(message) = self.open_fixture(fixture, started) {
            self.fail_p0(&message);
        }
    }

    fn open_fixture(&mut self, fixture: &str, started: Instant) -> Result<(), String> {
        if fixture.is_empty() {
            self.show_start_screen();
            return Ok(());
        }
        if !fixture_name_ok(fixture) {
            return Err("bad fixture".into());
        }
        let bytes = self.host.fixture_bytes(fixture).ok_or("fixture HTTP 404")?;
        let first = self.open_document(fixture, bytes, None, false);
        let first = first.ok_or("startup render produced no page")?;
        if !first.text.contains("PDF4All") {
            return Err("extracted text missing PDF4All".into());
        }
        let open_ms = self.doc.as_ref().map_or(0.0, |doc| doc.open_ms);
        self.console_line(
            "P0_RESULT",
            json!({
                "width": js_number(first.width),
                "height": js_number(first.height),
                "pixelWidth": first.pixel_width,
                "pixelHeight": first.pixel_height,
                "text": first.text,
                "renderMs": js_number(crate::js::js_round(started.elapsed().as_secs_f64() * 1000.0)),
                "openMs": js_number(open_ms),
                "firstPageRenderMs": js_number(first.paint_ms),
                "nonWhite": first.non_white,
            }),
        );
        Ok(())
    }

    /// R:2420 `openDocument`. `url` is None for the built-in startup sample, which has no
    /// file to save over. `restoring`: M9's fallback reopen, which does not count as proof that
    /// the engine is healthy (the restart cap stays as it is). Returns the first page's render
    /// (without pixels) when page 0 was drawn.
    pub fn open_document(
        &mut self,
        name: &str,
        bytes: Vec<u8>,
        url: Option<String>,
        restoring: bool,
    ) -> Option<RenderReply> {
        self.generation += 1;
        let started_generation = self.generation;
        // D6: the notice belongs to this open only; a failed open drops it.
        let open_notice = if restoring {
            None
        } else {
            self.pending_open_notice.take()
        };
        self.reset_document();
        self.redact_ui_hide_banner(); // §2.3 step 6: the redacted-copy banner belongs to the copy it was shown for
        self.recovery.reset(); // the open below starts the next log
        self.paused_reads.clear(); // a read paused for the last document may work for this one
        self.render_paused();
        self.disk_write = None; // what the last document's save wrote no longer matters
        self.clear_error();
        if bytes.len() < 5 {
            self.show_error(&messages::open::not_pdf(name));
            return None;
        }
        if bytes.len() > MAX_PDF_BYTES {
            self.show_error(&messages::open::too_large(name, None));
            return None;
        }
        self.set_status(&format!("Opening {name}…"));
        let opened = match self.request(
            json!({"op": "open", "pdf": {"$buffer": 0}, "name": name}),
            vec![bytes],
        ) {
            Ok(reply) => reply,
            Err(err) => {
                // §9.19(d): the pair-cap close happened before the open failed, so the engine slot is gone.
                let compare_closed = err.reply.get("compareClosed") == Some(&Value::Bool(true));
                if compare_closed && self.compare_doc.is_some() {
                    self.clear_compare();
                    self.compare_last = Some(CompareLast {
                        action: CompareAction::Close,
                        ok: true,
                        detail: Some("too-large".into()),
                    });
                }
                if started_generation == self.generation {
                    self.show_error(&err.message);
                }
                if compare_closed {
                    self.announce("Compare closed: the two files together exceed the size limit");
                }
                return None;
            }
        };
        if started_generation != self.generation {
            return None;
        }
        let opened: OpenReply = super::parse_reply_or_default(&opened.value);
        if !restoring {
            self.worker_restarts = 0; // a session that opened a document is healthy
        }
        let compare_closed = opened.compare_closed == Some(true);
        if compare_closed {
            // §9.3: the engine dropped the compare slot because the pair is over the size limit.
            self.clear_compare();
            self.compare_last = Some(CompareLast {
                action: CompareAction::Close,
                ok: true,
                detail: Some("too-large".into()),
            });
        }
        let info = opened.info.clone();
        self.doc = Some(OpenDoc {
            name: name.to_string(),
            url,
            page_count: opened.page_count,
            pages: opened.pages.clone(),
            rotations: opened.rotations.clone(),
            info: info.clone(),
            open_ms: opened.open_ms,
            untitled: None,
        });
        self.hide_start_screen();
        self.revision = opened.revision;
        self.auth = opened.auth;
        self.security_state = Some(state_from_reply(&info, self.auth));
        self.dom
            .set_text(DOCUMENT_TITLE, format!("{name} — Tandem PDF"));
        self.dom.set_text(ids::DOCUMENT_TITLE_EL, name);
        self.sync_dirty();
        self.banner_dismissed = None;
        self.render_banner(&info);
        self.wanted_page = 0;
        self.shown_page = -1;
        self.shown_key = None;
        self.refresh_thumbs();
        self.refresh_notes();
        self.refresh_fields();
        self.refresh_outline();
        self.refresh_links();
        self.refresh_labels();
        self.refresh_layers();
        self.refresh_redact_preview();
        self.sync_controls();
        let count = opened.page_count;
        self.set_status(&format!(
            "Opened {name} — {count} page{} in {} ms",
            if count == 1 { "" } else { "s" },
            crate::js::js_num(opened.open_ms)
        ));
        if info.needs_password {
            self.set_status(&format!(
                "{name} is encrypted; a password is required, so nothing can be rendered."
            ));
            self.show_password_form();
        } else {
            self.drain();
        }
        // A newer document took over during the render: report nothing for this one.
        if started_generation != self.generation {
            return None;
        }
        if self.compare_doc.is_some() {
            self.refresh_compare(true, true); // D1: the slot survives; B follows A's new page 0
        }
        if compare_closed {
            self.announce("Compare closed: the two files together exceed the size limit");
        }
        let first = if self.shown_page == 0 {
            self.last_result.as_mut().map(without_pixels)
        } else {
            None
        };
        if let Some(notice) = open_notice {
            self.show_message(&notice, MessageKind::Success);
            self.announce(&notice);
        }
        self.emit_result(first.as_ref());
        first
    }

    /// R:2504 `emitResult`: one `P1_RESULT` line per open (and per unlock).
    pub(crate) fn emit_result(&mut self, first: Option<&RenderReply>) {
        let Some(doc) = &self.doc else { return };
        let payload = json!({
            "name": doc.name,
            "pageCount": doc.page_count,
            "info": serde_json::to_value(&doc.info).unwrap_or(Value::Null),
            "openMs": js_number(doc.open_ms),
            "firstPageRenderMs": first.map_or(Value::Null, |first| js_number(first.render_ms)),
            "readOnly": is_read_only(&doc.info),
        });
        self.console_line("P1_RESULT", payload);
    }

    /// R:2536 `permitted`: one `/P` permission for this document; false while locked.
    pub fn permitted(&self, name: &str) -> bool {
        match &self.doc {
            Some(doc) if !doc.info.needs_password => allowed(&doc.info, name),
            _ => false,
        }
    }

    /// R:2546 `editable`: unlocked, not signed or certified, and granting at least one of the
    /// three editing permissions.
    pub fn editable(&self) -> bool {
        let Some(doc) = &self.doc else { return false };
        let info = &doc.info;
        if info.needs_password || info.signatures > 0 || info.certified {
            return false;
        }
        allowed(info, "any")
    }

    /// R:2571.
    pub fn can_annotate(&self) -> bool {
        self.editable() && self.permitted("annotate")
    }

    /// R:2575.
    pub fn can_edit(&self) -> bool {
        self.editable() && self.permitted("edit")
    }

    /// R:2579.
    pub fn can_form(&self) -> bool {
        self.editable() && self.permitted("form")
    }

    /// R:2584 `canDesign`: authoring writes the field tree (edit) and the page `/Annots` (annotate).
    pub fn can_design(&self) -> bool {
        self.doc
            .as_ref()
            .is_some_and(|doc| self.editable() && allowed(&doc.info, "edit+annotate"))
    }

    /// `#banner-dismiss` click (R:2592): the bar stays away until its text changes or another
    /// document opens.
    pub fn on_banner_dismiss(&mut self) {
        self.banner_dismissed = Some(self.dom.text(ids::BANNER_TEXT_EL));
        self.dom.set_hidden(ids::BANNER_EL, true);
    }

    /// R:2598 `renderBanner`: whole sentences that say what the file is and what that means.
    pub fn render_banner(&mut self, info: &DocumentInfo) {
        let mut flags: Vec<String> = Vec::new();
        if info.encrypted {
            if info.needs_password {
                flags.push(messages::banner::LOCKED.to_string());
            } else {
                let granted = permission_names(&info.permissions);
                let words: Vec<&str> = granted
                    .iter()
                    .map(|name| match name {
                        PermissionName::Print => "printing",
                        PermissionName::Edit => "changing pages",
                        PermissionName::Copy => "copying text",
                        PermissionName::Annotate => "adding comments",
                        PermissionName::Form => "filling forms",
                        PermissionName::Unknown => "unknown",
                    })
                    .collect();
                flags.push(messages::banner::unlocked(
                    granted.len() == 5,
                    &words,
                    info.algorithm.as_str().starts_with("rc4"),
                ));
            }
        }
        // A signed or certified file is always read-only, so it always gets the signed sentence.
        let signed = info.signatures > 0 || info.certified;
        if signed {
            flags.push(messages::banner::SIGNED_READ_ONLY.to_string());
        } else if is_read_only(info) && !info.needs_password {
            flags.push(messages::banner::READ_ONLY.to_string());
        }
        if info.xfa {
            flags.push(messages::banner::XFA.to_string());
        }
        if info.attachments > 0 {
            flags.push(messages::banner::attachments(info.attachments));
        }
        if info.open_action {
            flags.push(messages::banner::OPEN_ACTION.to_string());
        }
        if info.repaired {
            flags.push(messages::banner::REPAIRED.to_string());
        }
        let pending = match &self.security_state {
            Some(state) if state.pending => format!(
                " Security will change to {} when you save.",
                super::consts::lookup(&SECURITY_MODE_LABEL, state.mode.as_str())
                    .unwrap_or("undefined")
            ),
            _ => String::new(),
        };
        if flags.is_empty() && pending.is_empty() {
            self.dom.set_hidden(ids::BANNER_EL, true);
            self.dom.set_text(ids::BANNER_TEXT_EL, "");
            return;
        }
        let text = crate::js::js_trim(&format!("{}{pending}", flags.join(" "))).to_string();
        self.dom.set_text(ids::BANNER_TEXT_EL, text.clone());
        self.dom.set_hidden(
            ids::BANNER_EL,
            self.banner_dismissed.as_deref() == Some(text.as_str()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_names() {
        assert!(fixture_name_ok("hello.pdf"));
        assert!(!fixture_name_ok("../x.pdf"));
        assert!(!fixture_name_ok(".pdf"));
        assert!(!fixture_name_ok("a.PDF"));
    }

    #[test]
    fn banner_sentences() {
        let mut s = Session::new();
        let mut info = DocumentInfo {
            encrypted: true,
            ..DocumentInfo::default()
        };
        info.permissions.print = true;
        info.permissions.copy = true;
        info.algorithm = tandem_protocol::engine::DocumentAlgorithm::Rc4128;
        info.repaired = true;
        s.render_banner(&info);
        assert_eq!(
            s.dom.text(ids::BANNER_TEXT_EL),
            "This file is password-protected; its settings allow only printing and copying text. It uses old, weak encryption: File ▸ Security… can protect it better. This file was damaged and has been repaired. Check it before saving."
        );
        assert!(!s.dom.hidden(ids::BANNER_EL));
        s.on_banner_dismiss();
        s.render_banner(&info);
        assert!(s.dom.hidden(ids::BANNER_EL));
        s.render_banner(&DocumentInfo::default());
        assert!(s.dom.hidden(ids::BANNER_EL));
        assert_eq!(s.dom.text(ids::BANNER_TEXT_EL), "");
    }

    #[test]
    fn gates() {
        let info = DocumentInfo {
            signatures: 1,
            ..DocumentInfo::default()
        };
        assert!(is_read_only(&info));
        let state = state_from_reply(&DocumentInfo::default(), AuthLevel::User);
        assert!(state.permissions.is_empty() && state.auth == AuthLevel::User);
        let s = Session::new();
        assert!(!s.editable() && !s.can_edit() && !s.permitted("print"));
    }
}
