//! Certificate signing (renderer.ts R:10090-10121, R:10449-10588): the drive's `sign`, the
//! Sign chooser, the "Sign with certificate" dialog and "Sign and save as…". The key never
//! reaches the session: `self.host` reads the certificate file, holds the key for one
//! signature, signs the prepared bytes and writes the signed copy.

use super::Session;
use super::decorate::js_number_of;
use super::dom_ids::*;
use super::types::*;
use crate::host::sign::SignOffsets;
use crate::host::util::js_number;
use crate::js::{js_num, js_round};
use crate::messages::{self, user_text};
use serde_json::{Value, json};

/// JS `Number(value)` of a parsed JSON value.
pub(crate) fn js_to_number(value: Option<&Value>) -> f64 {
    match value {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => js_number_of(s),
        Some(Value::Array(items)) => match items.as_slice() {
            [] => 0.0,
            [one] => js_number_of(&js_string(Some(one))),
            _ => f64::NAN,
        },
        Some(Value::Object(_)) => f64::NAN,
    }
}

/// JS `String(value)` of a parsed JSON value.
pub(crate) fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => js_num(n.as_f64().unwrap_or(f64::NAN)),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                if item.is_null() {
                    String::new()
                } else {
                    js_string(Some(item))
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

/// `name.replace(/\.pdf$/i, "")`.
fn strip_pdf(name: &str) -> &str {
    let n = name.len();
    if n >= 4 && name.is_char_boundary(n - 4) && name[n - 4..].eq_ignore_ascii_case(".pdf") {
        &name[..n - 4]
    } else {
        name
    }
}

fn sign_fail(reason: &str) -> SignLast {
    SignLast {
        op: "sign".into(),
        ok: false,
        reason: Some(reason.into()),
        ..SignLast::default()
    }
}

impl Session {
    /// R:10092 drive `sign {cert, password, field?, page?, reason?, certify?, rect?}` (spec
    /// §3.B.24). No `clearError()` at the start. The TS waited for the shown page's fields
    /// to be listed before looking up `field`; here they are listed by the time this runs.
    pub fn drive_sign(&mut self, contents: Option<&str>) {
        let Some(values) = self.drive_wanted(contents) else {
            self.sign_last = Some(sign_fail("invalid"));
            return;
        };
        let (Some(Value::String(cert)), Some(Value::String(password))) =
            (values.get("cert"), values.get("password"))
        else {
            self.sign_last = Some(sign_fail("invalid"));
            return;
        };
        let (cert, password) = (cert.clone(), password.clone());
        // P5 U3 §2.5/§9.23: `field` signs that existing field on the shown page.
        let mut field = None;
        if let Some(Value::String(name)) = values.get("field") {
            field = self
                .fields
                .iter()
                .find(|item| item.kind.as_str() == "signature" && item.name == *name)
                .cloned();
            if field.is_none() {
                self.sign_last = Some(sign_fail("no-such-field"));
                return;
            }
        }
        self.pick_cert_sign_file(Some(&cert));
        if self.cert_sign_path.is_none() {
            self.sign_last = Some(sign_fail("unreadable"));
            return;
        }
        let page = js_to_number(values.get("page"));
        let page = if page.is_nan() || page == 0.0 {
            1.0
        } else {
            page
        };
        self.dom.set_value(CERT_SIGN_PAGE, js_num(page));
        let reason = match values.get("reason") {
            Some(Value::String(reason)) => reason.clone(),
            _ => String::new(),
        };
        self.dom.set_value(CERT_SIGN_REASON, reason);
        let signed = self.doc.as_ref().is_some_and(|doc| doc.info.signatures > 0);
        self.dom.set_disabled(CERT_SIGN_CERTIFY, signed);
        self.dom.set_checked(
            CERT_SIGN_CERTIFY,
            values.get("certify") == Some(&Value::Bool(true)),
        );
        let rect = values.get("rect").and_then(Value::as_array);
        self.dom.set_checked(CERT_SIGN_VISIBLE, rect.is_some());
        self.dom.set_hidden(CERT_SIGN_BOX, rect.is_none());
        if let Some(rect) = rect.filter(|rect| rect.len() == 4) {
            let n = |i: usize| js_to_number(rect.get(i));
            self.dom.set_value(CERT_SIGN_X, js_string(rect.first()));
            self.dom.set_value(CERT_SIGN_Y, js_string(rect.get(1)));
            self.dom.set_value(CERT_SIGN_W, js_num(n(2) - n(0)));
            self.dom.set_value(CERT_SIGN_H, js_num(n(3) - n(1)));
        }
        self.dom.set_value(CERT_SIGN_PASSWORD, password);
        self.set_cert_sign_mode(field.as_ref());
        self.dom.set_open(CERT_SIGN_DIALOG, true);
        self.run_cert_sign();
    }

    /// R:10449 (P5 U3 §9.15): field mode hides the page and appearance inputs (the field is
    /// both); the UI hides them while `cert_sign_field` is set.
    pub fn set_cert_sign_mode(&mut self, field: Option<&FieldItem>) {
        self.cert_sign_field = field.map(|f| CertSignField {
            name: f.name.clone(),
            page: self.wanted_page,
        });
        self.dom.set_hidden(CERT_SIGN_FIELD_EL, field.is_none());
        let text = field.map_or(String::new(), |f| {
            format!("Signing field {} on page {}", f.name, self.wanted_page + 1)
        });
        self.dom.set_text(CERT_SIGN_FIELD_EL, text);
    }

    /// R:10461 (item 9): the chooser; the picture choice needs a document that allows
    /// comments, the certificate one any open PDF.
    pub fn open_sign_choice(&mut self) {
        if self.doc.as_ref().is_none_or(|doc| doc.info.needs_password) {
            self.show_error(messages::sign::NEEDS_DOCUMENT);
            return;
        }
        let picture = self.can_annotate();
        self.dom.set_disabled(SIGN_CHOICE_PICTURE, !picture);
        self.dom.set_open(SIGN_CHOICE_DIALOG, true);
    }

    /// R:10882: the chooser's "Add my signature".
    pub fn on_sign_choice_picture(&mut self) {
        self.dom.set_open(SIGN_CHOICE_DIALOG, false);
        if self.ui_mode != UiMode::Forms {
            self.set_mode(UiMode::Forms);
        }
        self.press_sign();
    }

    /// R:10883: the chooser's "Sign digitally with a certificate".
    pub fn on_sign_choice_certificate(&mut self) {
        self.dom.set_open(SIGN_CHOICE_DIALOG, false);
        self.open_cert_sign_dialog(None);
    }

    /// R:10468: "Sign with certificate", for the page or for an empty signature field.
    pub fn open_cert_sign_dialog(&mut self, field: Option<&FieldItem>) {
        let Some(doc) = &self.doc else {
            self.show_error(messages::sign::NEEDS_DOCUMENT);
            return;
        };
        let size = doc
            .pages
            .get(self.wanted_page.max(0) as usize)
            .cloned()
            .unwrap_or_default();
        let certified = doc.info.certified;
        let can_certify = doc.info.signatures == 0;
        self.set_cert_sign_mode(field);
        self.dom
            .set_value(CERT_SIGN_PAGE, (self.wanted_page + 1).to_string());
        self.dom
            .set_value(CERT_SIGN_X, js_num((size.width - 236.0).max(0.0)));
        self.dom
            .set_value(CERT_SIGN_Y, js_num((size.height - 96.0).max(0.0)));
        // A certified document accepts only an invisible signature (the engine refuses a visible one).
        self.dom.set_checked(CERT_SIGN_VISIBLE, !certified);
        self.dom.set_disabled(CERT_SIGN_VISIBLE, certified);
        self.dom.set_hidden(CERT_SIGN_BOX, certified);
        self.dom.set_disabled(CERT_SIGN_CERTIFY, !can_certify);
        self.dom.set_checked(CERT_SIGN_CERTIFY, false);
        self.dom.set_text(
            CERT_SIGN_CERTIFY_REASON,
            if can_certify {
                ""
            } else {
                "Certification must be the first signature."
            },
        );
        self.dom.set_text(CERT_SIGN_STATUS, "");
        // Item 9: "Sign and save as…" waits for a certificate; it never does nothing silently.
        self.dom
            .set_disabled(CERT_SIGN_GO, self.cert_sign_path.is_none());
        self.dom.set_open(CERT_SIGN_DIALOG, true);
    }

    /// R:10490: "Choose certificate…"; `path` is a drive's pre-answer.
    pub fn pick_cert_sign_file(&mut self, path: Option<&str>) {
        match self.host.pick_certificate(path) {
            Err(reason) => {
                if reason != "cancelled" {
                    self.dom
                        .set_text(CERT_SIGN_STATUS, "Could not read that certificate file.");
                }
            }
            Ok(name) => {
                self.cert_sign_path = Some(path.map_or_else(|| name.clone(), str::to_string));
                self.dom.set_text(CERT_SIGN_FILE_EL, name);
                self.dom.set_text(CERT_SIGN_STATUS, "");
                self.dom.set_disabled(CERT_SIGN_GO, false);
            }
        }
    }

    /// R:10503: "Sign and save as…".
    pub fn run_cert_sign(&mut self) {
        if self.cert_sign_path.is_none() {
            self.dom
                .set_text(CERT_SIGN_STATUS, messages::sign::CHOOSE_CERTIFICATE);
            return;
        }
        // Review fix 6: a signed copy would not carry a protection change still waiting for Save.
        if self.security_state.as_ref().is_some_and(|s| s.pending) {
            self.sign_last = Some(sign_fail("security-pending"));
            self.dom
                .set_text(CERT_SIGN_STATUS, messages::sign::SECURITY_PENDING);
            return;
        }
        let Some(page_count) = self.doc.as_ref().map(|doc| doc.page_count) else {
            return;
        };
        // Redaction §2.3 step 7: a signed copy carries the boxes as marks ("Sign anyway").
        if !self.confirm_pending_save(PendingAction::Sign) {
            return;
        }
        let field = self.cert_sign_field.clone();
        let page = match &field {
            Some(field) => field.page as f64,
            None => js_number_of(&self.dom.value(CERT_SIGN_PAGE)) - 1.0,
        };
        let reason = self.dom.value(CERT_SIGN_REASON);
        let when = crate::engine::now_ms();
        let control = reason.chars().any(|c| (c as u32) < 0x20 || c == '\u{7f}');
        if page.fract() != 0.0
            || !page.is_finite()
            || page < 0.0
            || page >= page_count as f64
            || reason.encode_utf16().count() > 200
            || control
        {
            self.sign_last = Some(sign_fail("invalid"));
            self.dom
                .set_text(CERT_SIGN_STATUS, messages::sign::INVALID_INPUT);
            return;
        }
        let password = self.dom.value(CERT_SIGN_PASSWORD);
        self.dom.set_disabled(CERT_SIGN_GO, true);
        let cert = self.host.certificate_info(&password);
        self.dom.set_value(CERT_SIGN_PASSWORD, "");
        drop(password);
        if !self.dom.is_open(CERT_SIGN_DIALOG) {
            self.host.certificate_clear();
            self.dom.set_disabled(CERT_SIGN_GO, false);
            return;
        }
        let cert = match cert {
            Ok(cert) => cert,
            Err(fail) => {
                self.sign_last = Some(sign_fail(fail.reason));
                self.dom.set_text(
                    CERT_SIGN_STATUS,
                    messages::sign::certificate(fail.reason, fail.status.as_deref()),
                );
                self.host.certificate_clear();
                self.dom.set_disabled(CERT_SIGN_GO, false);
                return;
            }
        };
        let num = |s: &Self, id: &str| js_number_of(&s.dom.value(id));
        let rect = if field.is_none() && self.dom.checked(CERT_SIGN_VISIBLE) {
            let (x, y) = (num(self, CERT_SIGN_X), num(self, CERT_SIGN_Y));
            let (w, h) = (num(self, CERT_SIGN_W), num(self, CERT_SIGN_H));
            json!([
                js_number(x),
                js_number(y),
                js_number(x + w),
                js_number(y + h)
            ])
        } else {
            Value::Null
        };
        let certify = self.dom.checked(CERT_SIGN_CERTIFY);
        let mut message = json!({
            "op": "sign-prepare", "name": cert.cn, "reason": reason, "page": js_number(page), "rect": rect,
            "certify": certify, "when": js_number(when),
        });
        if let Some(field) = &field {
            message["field"] = json!(field.name);
        }
        let prepared = match self.request(message, vec![]) {
            Ok(reply) => reply,
            Err(err) => {
                self.host.certificate_clear();
                self.sign_last = Some(sign_fail(err.reason.as_deref().unwrap_or("invalid")));
                // `request()` rejects every `ok: false` reply, so the TS's resolved-refusal branch
                // (R:10548, "This document cannot be signed as it is.") never ran.
                self.dom.set_text(CERT_SIGN_STATUS, user_text(&err.message));
                self.dom.set_disabled(CERT_SIGN_GO, false);
                return;
            }
        };
        if !self.dom.is_open(CERT_SIGN_DIALOG) {
            self.host.certificate_clear();
            self.dom.set_disabled(CERT_SIGN_GO, false);
            return;
        }
        self.cert_sign_in_flight = true;
        let source_name = self
            .doc
            .as_ref()
            .map_or(String::new(), |doc| doc.name.clone());
        let suggested = format!(
            "{}{}",
            strip_pdf(&source_name),
            if certify {
                " certified.pdf"
            } else {
                " signed.pdf"
            }
        );
        let at = |key: &str| js_to_number(prepared.get(key)).max(0.0) as u64;
        let offsets = SignOffsets {
            contents_at: at("contentsAt"),
            contents_len: at("contentsLen"),
            byte_range_at: at("byteRangeAt"),
        };
        let bytes = prepared
            .bytes("bytes")
            .map(<[u8]>::to_vec)
            .unwrap_or_default();
        let revisions = js_round(js_to_number(prepared.get("revisions")));
        let signed = self.host.sign_file(bytes, offsets, when, Some(&suggested));
        self.cert_sign_in_flight = false;
        self.dom.set_disabled(CERT_SIGN_GO, false);
        let signed = match signed {
            Ok(signed) => signed,
            Err(fail) => {
                self.sign_last = Some(sign_fail(fail.reason));
                self.dom.set_text(
                    CERT_SIGN_STATUS,
                    messages::sign::failed(fail.reason, fail.status.as_deref()),
                );
                return;
            }
        };
        // The signing certificate was not meant for documents (main asked first; a drive proceeds): STATE says so.
        let last = SignLast {
            op: "sign".into(),
            ok: true,
            reason: None,
            name: Some(signed.name.clone()),
            signer: Some(signed.signer.clone()),
            revisions: Some(revisions as i64),
            field: field.as_ref().map(|f| f.name.clone()),
        };
        self.lane5.sign_warning = signed.warning.map(|w| (last.clone(), w.to_string()));
        self.sign_last = Some(last);
        self.cert_sign_path = None;
        self.close_cert_sign_dialog();
        // D6: main opens the signed copy in place of the original (a drive or smoke stays on it).
        let status = if signed.opened {
            messages::sign::signed_opened(&signed.name)
        } else {
            format!(
                "Signed as {} \u{2192} {}. The open document is unchanged.",
                signed.signer, signed.name
            )
        };
        if signed.opened {
            self.pending_open_notice = Some(status.clone());
        }
        self.dom.set_text(CERT_SIGN_STATUS, status.clone());
        self.set_status(&status);
        self.announce(&status);
    }

    /// `certSignDialog.close()` with its `close` listener (R:10969): clears field mode, the
    /// password, the chosen certificate (and the key main holds, unless signing is in
    /// flight), and disables "Sign and save as…".
    pub fn close_cert_sign_dialog(&mut self) {
        self.dom.set_open(CERT_SIGN_DIALOG, false);
        self.cert_sign_field = None;
        self.dom.set_value(CERT_SIGN_PASSWORD, "");
        if !self.cert_sign_in_flight && self.cert_sign_path.is_some() {
            self.host.certificate_clear();
        }
        self.cert_sign_path = None;
        self.dom
            .set_text(CERT_SIGN_FILE_EL, "No certificate chosen yet");
        self.dom.set_disabled(CERT_SIGN_GO, true);
    }

    /// Round 2: the box dragged on the page goes into the dialog's fields, and a line says
    /// where it landed ("Page 1, 180 × 80 pt, 72 pt from the left and 600 pt from the top").
    /// `rect` is page points, y down from the top.
    pub fn cert_sign_box_placed(&mut self, page: usize, rect: [f64; 4]) {
        let n = |v: f64| js_num(v.round());
        let [x, y, w, h] = rect;
        self.dom.set_value(CERT_SIGN_PAGE, (page + 1).to_string());
        self.dom.set_value(CERT_SIGN_X, n(x));
        self.dom.set_value(CERT_SIGN_Y, n(y));
        self.dom.set_value(CERT_SIGN_W, n(w));
        self.dom.set_value(CERT_SIGN_H, n(h));
        self.dom.set_checked(CERT_SIGN_VISIBLE, true);
        self.dom.set_hidden(CERT_SIGN_BOX, false);
        self.dom.set_text(
            CERT_SIGN_STATUS,
            format!(
                "Page {}, {} × {} pt, {} pt from the left and {} pt from the top.",
                page + 1,
                n(w),
                n(h),
                n(x),
                n(y)
            ),
        );
    }

    /// R:10967: "Visible signature" toggles the appearance box.
    pub fn on_cert_sign_visible_change(&mut self) {
        let visible = self.dom.checked(CERT_SIGN_VISIBLE);
        self.dom.set_hidden(CERT_SIGN_BOX, !visible);
    }
}

#[cfg(test)]
impl Session {
    fn state_summary_sign_for_test(&self) -> Value {
        json!({ "ready": !self.dom.disabled(CERT_SIGN_GO), "label": self.dom.text(CERT_SIGN_GO) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_conversions() {
        assert!(js_to_number(None).is_nan());
        assert_eq!(js_to_number(Some(&json!(null))), 0.0);
        assert_eq!(js_to_number(Some(&json!("2"))), 2.0);
        assert_eq!(js_to_number(Some(&json!([3]))), 3.0);
        assert_eq!(js_string(Some(&json!(72))), "72");
        assert_eq!(js_string(Some(&json!(1.5))), "1.5");
        assert_eq!(js_string(Some(&json!([1, null, 2]))), "1,,2");
        assert_eq!(strip_pdf("a.PDF"), "a");
        assert_eq!(strip_pdf("a.txt"), "a.txt");
    }

    #[test]
    fn cert_dialog_opens_without_a_certificate() {
        let mut s = Session::new();
        s.open_cert_sign_dialog(None);
        assert_eq!(s.notice_raw, messages::sign::NEEDS_DOCUMENT);
        s.doc = Some(OpenDoc {
            name: "a.pdf".into(),
            page_count: 1,
            pages: vec![PageSize {
                width: 612.0,
                height: 792.0,
            }],
            ..OpenDoc::default()
        });
        s.wanted_page = 0;
        s.open_cert_sign_dialog(None);
        assert!(s.dom.is_open(CERT_SIGN_DIALOG));
        assert!(s.dom.disabled(CERT_SIGN_GO));
        assert_eq!(s.dom.value(CERT_SIGN_X), "376");
        assert_eq!(s.dom.value(CERT_SIGN_Y), "696");
        let state = s.state_summary_sign_for_test();
        assert_eq!(state, json!({"ready": false, "label": "Sign and save as…"}));
    }
}
