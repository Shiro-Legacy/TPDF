//! Verify signatures and the trust list (renderer.ts R:10360-10447). The engine lists the
//! signatures (`sign-list`); `self.host` checks them against the trusted certificates.

use super::Session;
use super::dom_ids::*;
use crate::engine::Reply;
use crate::messages::verify as words;
use serde_json::{Map, Value, json};

/// One row of the Verify table (`#verify-list`), as the UI draws it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VerifyRow {
    /// `tr.dataset.field`.
    pub field: String,
    /// Field / page, Signer, Date, Status, Trust, Coverage, Reason.
    pub cells: Vec<String>,
    /// The verifier's plain-English sentence (hover title and `aria-description`).
    pub plain: Option<String>,
    /// `aria-current` (the signed field that was clicked, `markVerifyRow`).
    pub current: bool,
}

fn text_of(row: &Value, key: &str) -> String {
    match row.get(key) {
        None | Some(Value::Null) => String::new(),
        Some(value) => super::cert_sign::js_string(Some(value)),
    }
}

/// R:10421: the status cell; `reason` is space-joined notes, so weak-digest may sit beside
/// no-signing-time.
pub(crate) fn verify_status_text(row: &Value) -> String {
    let status = match row.get("status") {
        None | Some(Value::Null) => "unsupported".to_string(),
        Some(value) => super::cert_sign::js_string(Some(value)),
    };
    if status == "valid"
        && text_of(row, "reason")
            .split(' ')
            .any(|t| t == "weak-digest")
    {
        "valid (weak SHA-1 digest)".to_string()
    } else {
        status
    }
}

/// R:10426.
pub(crate) fn verify_refusal(reason: &str) -> String {
    match reason {
        "too-large" => "This document is too large to verify in this version",
        "encrypted" => "Encrypted signed documents cannot be verified in this version",
        "no-document" => "Open a document before verifying signatures",
        "unreadable" => "The signatures could not be read",
        "too-many" => "There are too many signatures to verify in this version",
        other => return other.to_string(),
    }
    .to_string()
}

/// `sign-list`'s entries for main: each `contents` marker becomes the bytes' hex (what the
/// preload handed main as a Uint8Array, which main's `wire` turned into hex).
fn entries_for_host(listed: &Reply) -> Vec<Value> {
    let Some(Value::Array(items)) = listed.get("signatures") else {
        return Vec::new();
    };
    items
        .iter()
        .map(|entry| {
            let mut entry = entry.clone();
            let marker = entry
                .get("contents")
                .filter(|c| c.get("$bytes").is_some() || c.get("$buffer").is_some());
            if let Some(bytes) = listed.bytes_at(marker) {
                entry["contents"] =
                    Value::String(bytes.iter().map(|b| format!("{b:02x}")).collect());
            }
            entry
        })
        .collect()
}

impl Session {
    /// R:10360: the Verify dialog (status bar "Verify", File ▸ Verify Signatures…).
    pub fn open_verify_dialog(&mut self) {
        self.dom.set_open(VERIFY_DIALOG, true);
        self.refresh_verify();
    }

    /// Worker and main refusals still reach the dialog and STATE.
    fn verify_refuse(&mut self, reason: &str) {
        let message = verify_refusal(reason);
        self.dom.set_text(VERIFY_SUMMARY, message.clone());
        let mut last = Map::new();
        last.insert("op".into(), json!("verify"));
        last.insert("ok".into(), json!(false));
        last.insert("reason".into(), json!(reason));
        self.verify_last = Some(last);
        self.show_error(&message);
        self.verify_by_field.clear();
        self.paint_hits_now();
    }

    /// R:10365: list, verify, fill the table, the summary, the badges and STATE `verify.last`.
    pub fn refresh_verify(&mut self) {
        self.dom.set_text(VERIFY_SUMMARY, "Verifying…");
        let listed = match self.request(json!({ "op": "sign-list" }), vec![]) {
            Ok(listed) => listed,
            Err(err) => {
                let reason = err.reason.clone().unwrap_or_else(|| "unreadable".into());
                self.verify_refuse(&reason);
                return;
            }
        };
        let pdf = listed
            .bytes("bytes")
            .map(<[u8]>::to_vec)
            .unwrap_or_default();
        let entries = entries_for_host(&listed);
        let result = match self.host.verify_signatures(&pdf, &entries) {
            Ok(result) => result,
            Err(reason) => {
                self.verify_refuse(reason);
                return;
            }
        };
        let rows: Vec<Value> = result
            .get("verdicts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let trusted: Vec<Value> = result
            .get("trusted")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let status = |row: &Value| {
            row.get("status")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let nonempty: Vec<&Value> = rows.iter().filter(|row| status(row) != "empty").collect();
        let counted: Vec<&&Value> = nonempty
            .iter()
            .filter(|row| {
                status(row) == "valid"
                    && row.get("trust").and_then(Value::as_str) == Some("trusted")
            })
            .collect();
        let partial = counted
            .iter()
            .any(|row| row.get("coverage").and_then(Value::as_str) == Some("partial"));
        let summary = if !nonempty.is_empty() {
            format!(
                "{} of {} signatures valid and trusted{}",
                counted.len(),
                nonempty.len(),
                if partial {
                    "; later changes were not verified"
                } else {
                    ""
                }
            )
        } else if rows.is_empty() {
            "No signature fields".to_string()
        } else {
            "No signed signatures".to_string()
        };
        self.dom.set_text(VERIFY_SUMMARY, summary.clone());
        let certified = listed.get("certified").filter(|c| !c.is_null()).cloned();
        let certified_text = certified.as_ref().map_or(String::new(), |level| {
            words::certified(level.as_f64().map_or(0, |l| l as i64))
        });
        self.dom.set_text(VERIFY_CERTIFIED, certified_text);
        let mut table = Vec::new();
        for row in &rows {
            let reason = text_of(row, "reason");
            let tokens: Vec<&str> = reason.split(' ').collect();
            let page = super::cert_sign::js_to_number(row.get("page"));
            let page = if page.is_nan() { 0.0 } else { page };
            let signer = match row.get("signerName") {
                None | Some(Value::Null) => "\u{2014}".to_string(),
                Some(value) => super::cert_sign::js_string(Some(value)),
            };
            let issuer = row
                .get("issuerName")
                .and_then(Value::as_str)
                .or_else(|| row.get("issuerCn").and_then(Value::as_str));
            let trust = match row.get("trust") {
                None | Some(Value::Null) => "none".to_string(),
                Some(value) => super::cert_sign::js_string(Some(value)),
            };
            let coverage = match row.get("coverage").and_then(Value::as_str) {
                Some("partial") => words::PARTIAL.to_string(),
                Some("entire-file") => words::WHOLE.to_string(),
                _ => "\u{2014}".to_string(),
            };
            let cells = vec![
                format!("{} / {}", text_of(row, "field"), crate::js::js_num(page)),
                // Names cleaned by the verifier and reasons in words; a plain local date (STATE
                // keeps the verifier's ISO time). "no-signing-time" means the date given is not one.
                signer,
                words::signed_on(
                    row.get("signingTime").and_then(Value::as_str),
                    row.get("timestamped") != Some(&Value::Bool(true)),
                    tokens.contains(&"no-signing-time"),
                    None,
                ),
                verify_status_text(row),
                words::trust(&trust, issuer),
                coverage,
                words::notes(&reason, row.get("reasonWords").and_then(Value::as_str)),
            ];
            let plain = row
                .get("plain")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
                .map(str::to_string);
            table.push(VerifyRow {
                field: text_of(row, "field"),
                cells,
                plain,
                current: false,
            });
        }
        self.lane5.verify_rows = table;
        self.lane5.verify_trusted = trusted
            .iter()
            .map(|cert| {
                cert.get("cn")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            })
            .collect();
        let signatures: Vec<Value> = rows
            .iter()
            .map(|row| {
                let mut out = Map::new();
                // `JSON.stringify` drops the keys a verdict lacks.
                for key in ["field", "status", "trust", "coverage", "signerCn", "reason"] {
                    if let Some(value) = row.get(key) {
                        out.insert(key.into(), value.clone());
                    }
                }
                out.insert("cell".into(), json!(verify_status_text(row)));
                if let Some(plain) = row.get("plain") {
                    out.insert("plain".into(), plain.clone());
                }
                out.insert(
                    "signingTime".into(),
                    row.get("signingTime").cloned().unwrap_or(Value::Null),
                );
                Value::Object(out)
            })
            .collect();
        let mut last = Map::new();
        last.insert("op".into(), json!("verify"));
        last.insert("ok".into(), json!(true));
        last.insert("certified".into(), certified.unwrap_or(Value::Null));
        last.insert("trusted".into(), json!(trusted.len()));
        last.insert("summary".into(), json!(summary));
        last.insert("signatures".into(), Value::Array(signatures));
        self.verify_last = Some(last);
        // P5 U3 §9.18: the badges follow the rows just shown.
        self.verify_by_field.clear();
        for row in nonempty {
            let status = match row.get("status") {
                None | Some(Value::Null) => "unsupported".to_string(),
                Some(value) => super::cert_sign::js_string(Some(value)),
            };
            self.verify_by_field.insert(text_of(row, "field"), status);
        }
        self.paint_hits_now();
    }

    /// R:10414 (§2.2): the Verify row of the signed field that was clicked is the current one.
    pub fn mark_verify_row(&mut self, name: &str) {
        for row in &mut self.lane5.verify_rows {
            row.current = row.field == name;
        }
    }

    /// R:10436 drive `verify {trust?, forget?}` (spec §3.B.18): the dialog is not opened.
    pub fn drive_verify(&mut self, contents: Option<&str>) {
        self.clear_error();
        let Some(wanted) = self.drive_wanted(contents) else {
            let mut last = Map::new();
            last.insert("op".into(), json!("verify"));
            last.insert("ok".into(), json!(false));
            last.insert("reason".into(), json!("invalid"));
            self.verify_last = Some(last);
            return;
        };
        if wanted.get("forget") == Some(&Value::Bool(true)) {
            self.host.trust_clear();
        }
        if let Some(Value::String(path)) = wanted.get("trust")
            && let Err(reason) = self.host.trust_add(Some(path))
        {
            let mut last = Map::new();
            last.insert("op".into(), json!("verify"));
            last.insert("ok".into(), json!(false));
            last.insert("reason".into(), json!(reason));
            self.verify_last = Some(last);
            return;
        }
        self.refresh_verify();
    }

    /// R:10957: "Trust certificates…".
    pub fn on_verify_trust_add(&mut self) {
        if let Err(reason) = self.host.trust_add(None)
            && reason != "cancelled"
        {
            self.set_status(&format!("Trust certificates: {reason}"));
        }
        self.refresh_verify();
    }

    /// R:10962: "Forget all trusted certificates" (no confirmation).
    pub fn on_verify_trust_clear(&mut self) {
        self.host.trust_clear();
        self.refresh_verify();
    }

    /// `paintHits(canvas.clientWidth, canvas.clientHeight, shownPage)`: the canvas CSS box is
    /// the shown page's size × the zoom.
    pub(crate) fn paint_hits_now(&mut self) {
        let page = self.shown_page;
        let size = usize::try_from(page)
            .ok()
            .and_then(|p| self.doc.as_ref()?.pages.get(p).cloned());
        let (width, height) = match size {
            Some(size) => {
                let zoom = self.zoom_now(page);
                (size.width * zoom, size.height * zoom)
            }
            None => (0.0, 0.0),
        };
        self.paint_hits(width, height, page);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_cells_and_refusals() {
        assert_eq!(
            verify_status_text(
                &json!({"status": "valid", "reason": "no-signing-time weak-digest"})
            ),
            "valid (weak SHA-1 digest)"
        );
        assert_eq!(verify_status_text(&json!({})), "unsupported");
        assert_eq!(
            verify_refusal("too-many"),
            "There are too many signatures to verify in this version"
        );
        assert_eq!(verify_refusal("odd"), "odd");
    }

    #[test]
    fn contents_markers_become_hex() {
        let listed = Reply {
            value: json!({"signatures": [{"field": "S", "contents": {"$bytes": 0}}]}),
            outputs: vec![vec![0x30, 0x82]],
        };
        let entries = entries_for_host(&listed);
        assert_eq!(entries[0]["contents"], json!("3082"));
    }
}
