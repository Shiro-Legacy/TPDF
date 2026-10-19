//! Lane 1: renderer.ts 2628–2706, "password": unlocking an encrypted file. The password goes
//! to the engine and nowhere else: no IPC, no log, no console line.
#![allow(dead_code)]

use super::Session;
use super::dom_ids as ids;
use super::opening::state_from_reply;
use crate::messages;
use serde_json::json;
use tandem_protocol::engine::{AuthLevel, AuthenticateReply};

impl Session {
    /// R:2630 `showPasswordForm`.
    pub(crate) fn show_password_form(&mut self) {
        let Some(doc) = &self.doc else { return };
        let name = doc.name.clone();
        self.dom.set_text(ids::PASSWORD_NAME, name);
        self.dom.set_text(ids::PASSWORD_ERROR_EL, "");
        self.dom.set_value(ids::PASSWORD_INPUT, "");
        self.dom.set_hidden(ids::PASSWORD_FORM, false);
    }

    /// R:2639 `hidePasswordForm`.
    pub(crate) fn hide_password_form(&mut self) {
        self.dom.set_hidden(ids::PASSWORD_FORM, true);
        self.dom.set_value(ids::PASSWORD_INPUT, "");
        self.dom.set_text(ids::PASSWORD_ERROR_EL, "");
        self.unlocking = false;
    }

    /// The form's submit (Enter or "Unlock"), with what the user typed in `#password-input`.
    pub fn on_password_submit(&mut self, password: &str) {
        self.dom.set_value(ids::PASSWORD_INPUT, password);
        self.submit_password();
        self.run_pending_tasks();
    }

    /// R:2650 `submitPassword`: reads `#password-input` and clears it at once; a correct
    /// password re-runs the open path (banner, thumbs, controls, first page).
    pub fn submit_password(&mut self) {
        let locked = self.doc.as_ref().is_some_and(|doc| doc.info.needs_password);
        if !locked || self.unlocking {
            return;
        }
        let password = self.dom.value(ids::PASSWORD_INPUT);
        self.dom.set_value(ids::PASSWORD_INPUT, ""); // the request is the only place it lives from here on
        let started_generation = self.generation;
        self.unlocking = true;
        self.dom.set_text(ids::PASSWORD_ERROR_EL, "");
        let name = self
            .doc
            .as_ref()
            .map(|doc| doc.name.clone())
            .unwrap_or_default();
        self.set_status(&format!("Unlocking {name}…"));
        let reply = self.request(
            json!({"op": "authenticate", "password": password}),
            Vec::new(),
        );
        self.unlocking = false;
        let reply = match reply {
            Ok(reply) => reply,
            Err(err) => {
                if started_generation == self.generation {
                    self.show_error(&err.message);
                }
                return;
            }
        };
        if started_generation != self.generation || self.doc.is_none() {
            return; // a newer document owns the UI now
        }
        let reply: AuthenticateReply = super::parse_reply_or_default(&reply.value);
        if !reply.authenticated {
            self.dom
                .set_text(ids::PASSWORD_ERROR_EL, messages::open::WRONG_PASSWORD);
            self.set_status(&format!("{name} is still locked."));
            return;
        }
        self.apply_unlocked(reply);
    }

    /// R:2680 `applyUnlocked`: the same reopen path the first page took.
    pub(crate) fn apply_unlocked(&mut self, reply: AuthenticateReply) {
        let Some(doc) = self.doc.as_mut() else { return };
        if let Some(count) = reply.page_count {
            doc.page_count = count;
        }
        if let Some(pages) = reply.pages {
            doc.pages = pages;
        }
        if let Some(rotations) = reply.rotations {
            doc.rotations = rotations;
        }
        if let Some(info) = reply.info {
            doc.info = info;
        }
        let info = doc.info.clone();
        self.auth = reply.auth.unwrap_or(AuthLevel::User);
        self.security_state = Some(state_from_reply(&info, self.auth));
        self.hide_password_form();
        self.render_banner(&info);
        self.refresh_thumbs();
        self.refresh_fields();
        self.refresh_outline();
        self.refresh_links();
        self.refresh_labels();
        self.refresh_layers();
        self.refresh_redact_preview();
        self.wanted_page = 0;
        self.shown_page = -1;
        self.shown_key = None;
        self.shown_zoom = 0.0;
        self.sync_controls();
        self.clear_error(); // §9.19(c): a stale refusal does not outlive the unlock
        self.drain();
        if self.compare_doc.is_some() {
            self.refresh_compare(true, true); // §9.19(h)
        }
        let first = if self.shown_page == 0 {
            self.last_result
                .as_ref()
                .map(|r| tandem_protocol::engine::RenderReply {
                    pixels: Vec::new(),
                    ..r.clone()
                })
        } else {
            None
        };
        self.emit_result(first.as_ref());
    }
}
