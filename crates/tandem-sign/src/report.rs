// SPDX-License-Identifier: MIT
//! A verification verdict in plain English, for people rather than for
//! programs: who signed and when, whether the document changed since, what a
//! certifying signature allows, and whether the signer is trusted. No
//! internal names, byte counts or format jargon. Dates are UTC.

use crate::time;
use crate::verify::{Coverage, Status, Trust, Verdict};

fn has_note(v: &Verdict, note: &str) -> bool {
    v.reason
        .as_deref()
        .is_some_and(|r| r.split(' ').any(|n| n == note))
}

fn signer(v: &Verdict) -> Option<&str> {
    v.signer_cn.as_deref().filter(|s| !s.trim().is_empty())
}

/// One to five short sentences describing `v`.
pub fn plain_sentence(v: &Verdict) -> String {
    let who = signer(v);
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
            let name = who.unwrap_or("an unnamed signer");
            match v.signing_time_ms {
                Some(ms) if !has_note(v, "no-signing-time") => {
                    out.push(format!("Signed by {name} on {}.", time::day_month_year(ms)));
                }
                _ => out.push(format!("Signed by {name}. The signature does not say when it was made.")),
            }
            out.push(match v.coverage {
                Coverage::Partial if has_note(v, "changes-not-permitted") => {
                    "The document was changed after it was signed, although the signer allowed no changes.".to_string()
                }
                Coverage::Partial => "The signed version has not changed. Later changes were added after signing and are not covered by this signature.".to_string(),
                _ => "The document has not changed since it was signed.".to_string(),
            });
            match v.docmdp.as_ref().and_then(serde_json::Value::as_f64) {
                Some(1.0) => out.push("This signature certifies the document and allows no changes.".to_string()),
                Some(2.0) => out.push(
                    "This signature certifies the document and allows only filling in forms and signing.".to_string(),
                ),
                Some(3.0) => out.push(
                    "This signature certifies the document and allows filling in forms, signing and comments.".to_string(),
                ),
                _ => {}
            }
            if has_note(v, "weak-digest") {
                out.push("It uses an old method (SHA-1) that is no longer considered safe.".to_string());
            }
            out.push(match v.trust {
                Trust::Trusted => match v.issuer_cn.as_deref() {
                    Some(issuer) if Some(issuer) != who && !issuer.is_empty() => format!(
                        "The signer's certificate is trusted: it was issued through {issuer}, which is in your trusted list."
                    ),
                    _ => "The signer's certificate is in your trusted list.".to_string(),
                },
                Trust::Expired => "The signer's certificate was not valid on the date it was signed.".to_string(),
                _ => "The signer's certificate is not in your trusted list.".to_string(),
            });
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
            trust: Trust::Untrusted,
            coverage: Coverage::EntireFile,
            covered: Some(10),
            total: Some(10),
            docmdp: None,
        }
    }

    #[test]
    fn the_plan_example() {
        assert_eq!(
            plain_sentence(&verdict()),
            "Signed by Jane Doe on 18 Oct 2026. The document has not changed since it was signed. The signer's certificate is not in your trusted list."
        );
    }

    #[test]
    fn variants() {
        let mut v = verdict();
        v.trust = Trust::Trusted;
        v.issuer_cn = Some("Acme CA".into());
        v.coverage = Coverage::Partial;
        v.docmdp = Some(serde_json::json!(2));
        v.reason = Some("weak-digest".into());
        let text = plain_sentence(&v);
        assert!(text.contains("Later changes were added after signing"));
        assert!(text.contains("allows only filling in forms and signing"));
        assert!(text.contains("SHA-1"));
        assert!(text.ends_with("issued through Acme CA, which is in your trusted list."));
        v.status = Status::Invalid;
        v.reason = Some("digest".into());
        assert_eq!(
            plain_sentence(&v),
            "Signed by Jane Doe, but the document has changed since it was signed, so the signature is no longer valid."
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
            text.starts_with("Signed by Jane Doe. The signature does not say when it was made.")
        );
        assert!(text.ends_with("was not valid on the date it was signed."));
        for text in [plain_sentence(&t), plain_sentence(&v)] {
            for jargon in ["DocMDP", "byte", "CMS", "P=", "digest", "refused"] {
                assert!(!text.contains(jargon), "{text}");
            }
        }
    }
}
