// SPDX-License-Identifier: MIT
//! A verification verdict in plain English, for people rather than for
//! programs: who signed and when, whether the document changed since, what a
//! certifying signature allows, and whether the signer is trusted. No
//! internal names, byte counts or format jargon. Dates are UTC.
//!
//! The signer's name comes from the certificate, which anyone can make, so it
//! is shown in quotes, cleaned of control and text-direction characters and
//! cut to [`MAX_NAME`] characters, and when the certificate is not trusted the
//! sentence says so before it names anyone.

use crate::time;
use crate::verify::{Coverage, Status, Trust, Verdict};

/// The longest signer name shown, in characters.
pub const MAX_NAME: usize = 64;

fn has_note(v: &Verdict, note: &str) -> bool {
    v.reason
        .as_deref()
        .is_some_and(|r| r.split(' ').any(|n| n == note))
}

/// Characters that are dropped from a name: controls, and the marks and
/// overrides that reorder text (so a name cannot make the sentence around it
/// read differently).
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2069}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
        )
}

/// A certificate name for display: cleaned, whitespace collapsed, at most
/// [`MAX_NAME`] characters (an ellipsis marks a cut); `None` when nothing is
/// left. The Verify table shows this; sentences quote it ([`shown_name`]).
pub fn clean_name(raw: &str) -> Option<String> {
    let cleaned: String = raw.chars().filter(|&c| !hidden(c)).collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    let joined = words.join(" ");
    if joined.is_empty() {
        return None;
    }
    let mut name: String = joined.chars().take(MAX_NAME).collect();
    if joined.chars().count() > MAX_NAME {
        name.push('…');
    }
    Some(name)
}

/// [`clean_name`] in quotes, as sentences show a name.
pub fn shown_name(raw: &str) -> Option<String> {
    clean_name(raw).map(|name| format!("“{name}”"))
}

/// A short plain phrase for one reason or note token, for a table cell.
pub fn note_words(token: &str) -> &'static str {
    match token {
        "weak-digest" => "uses an old method (SHA-1)",
        "no-signing-time" => "no date given",
        "changes-not-permitted" => "changed although no changes were allowed",
        "time-changed" => "date edited after signing",
        "outdated-security" => "certificate uses outdated security",
        "not-for-signing" => "certificate not meant for signing",
        "unsupported-ca-rule" => "issuer rules not checked",
        "digest" => "changed after signing",
        "signature" => "does not match the certificate",
        "signer-cert" => "certificate does not belong to the signer",
        "byte-range" => "does not cover the document properly",
        "content-type" => "not a document signature",
        "malformed" => "could not be read",
        "signer-id" | "no-signed-attrs" | "missing-subfilter" => "kind not supported",
        _ => "kind not supported",
    }
}

/// [`note_words`] for each token of a space-joined reason.
pub fn reason_words(reason: &str) -> String {
    reason
        .split(' ')
        .filter(|t| !t.is_empty())
        .map(note_words)
        .collect::<Vec<_>>()
        .join("; ")
}

fn trust_sentence(v: &Verdict, who: Option<&str>) -> String {
    match v.trust {
        Trust::Trusted => match v.issuer_cn.as_deref().and_then(shown_name) {
            Some(issuer) if Some(issuer.as_str()) != who => format!(
                "The signer's certificate is trusted: it was issued through {issuer}, which is in your trusted list."
            ),
            _ => "The signer's certificate is in your trusted list.".to_string(),
        },
        Trust::Expired => {
            "The signer's certificate was not valid on the date it was signed, so the name is not confirmed."
                .to_string()
        }
        _ if has_note(v, "outdated-security") => {
            "The signer's certificate uses outdated security, so the name is not confirmed.".to_string()
        }
        _ if has_note(v, "unsupported-ca-rule") => {
            "The issuer of the signer's certificate uses rules Tandem can't check yet, so the name is not confirmed."
                .to_string()
        }
        _ if has_note(v, "not-for-signing") => {
            "The signer's certificate is not meant for signing documents, so the name is not confirmed."
                .to_string()
        }
        _ => "The signer's certificate is not in your trusted list, so the name is not confirmed.".to_string(),
    }
}

/// One to five short sentences describing `v`.
pub fn plain_sentence(v: &Verdict) -> String {
    let who = v.signer_cn.as_deref().and_then(shown_name);
    let who = who.as_deref();
    let reason = v.reason.as_deref().unwrap_or("");
    match v.status {
        Status::Empty => "This signature field has not been signed yet.".to_string(),
        Status::Unsupported if reason == "malformed" => {
            "This signature could not be read, so it was not checked.".to_string()
        }
        Status::Unsupported => match who {
            Some(name) => format!(
                "Signed by {name}, but this kind of signature cannot be checked by this app yet."
            ),
            None => "This kind of signature cannot be checked by this app yet.".to_string(),
        },
        Status::Invalid => match (reason, who) {
            ("digest", Some(name)) => format!(
                "Signed by {name}, but the document has changed since it was signed, so the signature is no longer valid."
            ),
            ("digest", None) => {
                "The document has changed since it was signed, so the signature is no longer valid.".to_string()
            }
            ("signature", Some(name)) => format!(
                "The signature by {name} does not match the signer's certificate, so it is not valid."
            ),
            ("signer-cert", _) => {
                "The certificate in this signature does not belong to the signer, so the signature is not valid."
                    .to_string()
            }
            ("byte-range", _) => {
                "This signature does not cover the document the way it should, so it cannot be relied on.".to_string()
            }
            _ => "This signature is not valid.".to_string(),
        },
        Status::Valid => {
            let mut out = Vec::new();
            let trust = trust_sentence(v, who);
            if v.trust != Trust::Trusted {
                out.push(trust.clone());
            }
            let name = who.unwrap_or("an unnamed signer");
            match v.signing_time_ms {
                Some(ms) if !has_note(v, "no-signing-time") => {
                    // The date is the signer's own claim (no timestamp), so it is "dated", not proven.
                    out.push(format!("Signed by {name}, dated {}.", time::day_month_year(ms)));
                }
                _ => out.push(format!(
                    "Signed by {name}. The signature does not say when it was made."
                )),
            }
            out.push(match v.coverage {
                Coverage::Partial if has_note(v, "changes-not-permitted") => {
                    "The document was changed after it was signed, although the signer allowed no changes.".to_string()
                }
                Coverage::Partial => "The signed version has not changed. Later changes were added after signing and are not covered by this signature.".to_string(),
                _ => "The document has not changed since it was signed.".to_string(),
            });
            match v.docmdp.as_ref().and_then(serde_json::Value::as_f64) {
                Some(1.0) => out.push(
                    "This signature certifies the document and allows no changes.".to_string(),
                ),
                Some(2.0) => out.push(
                    "This signature certifies the document and allows only filling in forms and signing.".to_string(),
                ),
                Some(3.0) => out.push(
                    "This signature certifies the document and allows filling in forms, signing and comments.".to_string(),
                ),
                _ => {}
            }
            if has_note(v, "time-changed") {
                out.push(
                    "A later change edited the signature's date; the date shown is the one that was signed."
                        .to_string(),
                );
            }
            if has_note(v, "weak-digest") {
                out.push(
                    "It uses an old method (SHA-1) that is no longer considered safe.".to_string(),
                );
            }
            if v.trust == Trust::Trusted {
                out.push(trust);
            }
            out.join(" ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verify::{Coverage, Status, Trust};

    fn verdict() -> Verdict {
        Verdict {
            field: "Sig1".into(),
            page: 0,
            status: Status::Valid,
            reason: None,
            signer_cn: Some("Jane Doe".into()),
            issuer_cn: None,
            signing_time_ms: Some(time::utc_ms(2026, 10, 18, 9, 30, 0)),
            digest: Some("sha256"),
            trust: Trust::Trusted,
            coverage: Coverage::EntireFile,
            covered: Some(10),
            total: Some(10),
            docmdp: None,
        }
    }

    #[test]
    fn the_plan_example_with_the_name_quoted_and_trust_first_when_untrusted() {
        let mut v = verdict();
        assert_eq!(
            plain_sentence(&v),
            "Signed by “Jane Doe”, dated 18 Oct 2026. The document has not changed since it was signed. The signer's certificate is in your trusted list."
        );
        v.trust = Trust::Untrusted;
        assert_eq!(
            plain_sentence(&v),
            "The signer's certificate is not in your trusted list, so the name is not confirmed. Signed by “Jane Doe”, dated 18 Oct 2026. The document has not changed since it was signed."
        );
    }

    #[test]
    fn names_are_cleaned_capped_and_quoted() {
        assert_eq!(
            shown_name("Jane\u{202E}eoD\u{202C}\n Doe").as_deref(),
            Some("“JaneeoD Doe”")
        );
        assert_eq!(shown_name("\u{200F}\u{0007} "), None);
        let long = "x".repeat(100);
        let shown = shown_name(&long).unwrap();
        assert_eq!(shown.chars().count(), MAX_NAME + 3);
        assert!(shown.ends_with("…”"));
        let mut v = verdict();
        v.signer_cn = Some("\u{202E}".into());
        assert!(plain_sentence(&v).starts_with("Signed by an unnamed signer, dated"));
    }

    #[test]
    fn variants() {
        let mut v = verdict();
        v.issuer_cn = Some("Acme CA".into());
        v.coverage = Coverage::Partial;
        v.docmdp = Some(serde_json::json!(2));
        v.reason = Some("weak-digest".into());
        let text = plain_sentence(&v);
        assert!(text.contains("Later changes were added after signing"));
        assert!(text.contains("allows only filling in forms and signing"));
        assert!(text.contains("SHA-1"));
        assert!(text.ends_with("issued through “Acme CA”, which is in your trusted list."));
        v.status = Status::Invalid;
        v.reason = Some("digest".into());
        assert_eq!(
            plain_sentence(&v),
            "Signed by “Jane Doe”, but the document has changed since it was signed, so the signature is no longer valid."
        );
        v.status = Status::Empty;
        assert_eq!(
            plain_sentence(&v),
            "This signature field has not been signed yet."
        );
        let mut t = verdict();
        t.reason = Some("no-signing-time".into());
        t.trust = Trust::Expired;
        let text = plain_sentence(&t);
        assert!(
            text.starts_with("The signer's certificate was not valid on the date it was signed")
        );
        assert!(
            text.contains("Signed by “Jane Doe”. The signature does not say when it was made.")
        );
        let mut o = verdict();
        o.trust = Trust::Untrusted;
        o.reason = Some("outdated-security".into());
        assert!(plain_sentence(&o).starts_with("The signer's certificate uses outdated security"));
        o.reason = Some("unsupported-ca-rule".into());
        assert!(
            plain_sentence(&o).starts_with("The issuer of the signer's certificate uses rules")
        );
        assert_eq!(
            reason_words("weak-digest no-signing-time"),
            "uses an old method (SHA-1); no date given"
        );
        o.reason = Some("not-for-signing".into());
        assert!(
            plain_sentence(&o)
                .starts_with("The signer's certificate is not meant for signing documents")
        );
        for text in [plain_sentence(&t), plain_sentence(&v), plain_sentence(&o)] {
            for jargon in [
                "DocMDP", "byte", "CMS", "P=", "digest", "refused", "SHA-256",
            ] {
                assert!(!text.contains(jargon), "{text}");
            }
        }
    }
}
