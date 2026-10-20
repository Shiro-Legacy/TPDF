//! Certificates, signing and verification (main.ts M:1058-1226 with
//! `src/sign/*.ts`), natively through the `tandem-sign` crate.
//!
//! Electron read certificate files and signed in a worker thread that kept
//! the private key and handed main a handle. Natively the key stays in the
//! host: one decrypted identity, held for one signature ("one shot": signing
//! takes it), dropped (its key zeroed by `tandem-sign`'s drop) when cleared,
//! replaced, used, or idle for five minutes. The file bytes and the random
//! seed are zeroed as soon as they are read.

use super::util::{basename, js_number};
use super::{Host, MAX_SAVE_BYTES, Reason, SaveKind, suggested_kind_name};
use crate::platform::{
    BoxKind, CERTIFICATE_FILTER, MessageBox, Pick, Question, SavePurpose, TRUST_FILTER,
};
use crate::safe_write::{self, Inject, SaveFail};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tandem_sign::cert::Cert;
use tandem_sign::cms::{self, Offsets};
use tandem_sign::drbg::HmacDrbg;
use tandem_sign::identity::{Identity, SignError};
use tandem_sign::{report, time, verify};
use zeroize::Zeroize;

/// A held key is dropped after this long unused (`createCertHold(5 * 60 * 1000)`).
pub const CERT_IDLE: Duration = Duration::from_secs(5 * 60);
/// Certificate and trust files: 1 byte … 1 MiB (M:1098, 1206).
const MAX_CERT_FILE: u64 = 1024 * 1024;
/// `MAX_PASSWORD` for a certificate (M:1088).
const MAX_PASSWORD_UNITS: usize = 1024;
/// `SIGNING_SKEW_MS` (cms.ts:15): the signing time must be within ten minutes of now.
pub const SIGNING_SKEW_MS: f64 = 10.0 * 60.0 * 1000.0;
/// Verify: at most this many signatures per call (M:1178).
const MAX_VERIFY_ENTRIES: usize = 200;
const MAX_CONTENTS: usize = 64 * 1024;
const MAX_STRING: usize = 1000;

/// A refused certificate or signing call: the reason token, and for an
/// out-of-date certificate its date and the sentence (`{reason, date?, status?}`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignFail {
    pub reason: &'static str,
    pub date: Option<String>,
    pub status: Option<String>,
}

impl SignFail {
    pub fn of(reason: &'static str) -> Self {
        SignFail {
            reason,
            date: None,
            status: None,
        }
    }

    /// pkcs12.ts `load`: the tokens the message catalogue knows, else `unreadable`.
    fn from_error(e: &SignError) -> Self {
        let reason = match e.reason() {
            "bad-password" => "bad-password",
            "unsupported-key" => "unsupported-key",
            "certificate-expired" => "certificate-expired",
            "too-costly" => "too-costly",
            _ => "unreadable",
        };
        let date = match e {
            SignError::CertificateExpired { date, .. } => Some(date.clone()),
            _ => None,
        };
        SignFail {
            reason,
            status: date.as_ref().and(e.status()),
            date,
        }
    }

    /// The IPC payload (`{ok:false, reason, date?, status?}`).
    pub fn to_json(&self) -> Value {
        let mut v = json!({"ok": false, "reason": self.reason});
        if let (Some(date), Some(status)) = (&self.date, &self.status) {
            v["date"] = json!(date);
            v["status"] = json!(status);
        }
        v
    }
}

/// `certificate-info`'s answer (M:1112).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertInfo {
    pub cn: String,
    /// ISO time.
    pub not_after: String,
    /// How many certificates the file held.
    pub certs: usize,
    /// `not-for-signing` when the certificate's key usage excludes document signing.
    pub warning: Option<&'static str>,
}

/// `sign-file`'s answer (M:1162-1164).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signed {
    pub name: String,
    pub signer: String,
    /// The signed copy now opens in place of the original (not in a smoke run).
    pub opened: bool,
    pub warning: Option<&'static str>,
}

struct Held {
    identity: Identity,
    cn: String,
    warning: Option<&'static str>,
    since: Instant,
}

/// The picked certificate file and the one held identity (`certPaths`, `certHold`).
#[derive(Default)]
pub(crate) struct CertState {
    path: Option<PathBuf>,
    held: Option<Held>,
    /// The name of the last file "Trust certificates…" read (round 2: a refusal names it).
    pub(crate) trust_file: Option<String>,
}

impl CertState {
    /// Drops a key left unused for five minutes.
    pub(crate) fn expire_idle(&mut self) {
        if self
            .held
            .as_ref()
            .is_some_and(|h| h.since.elapsed() >= CERT_IDLE)
        {
            self.held = None;
        }
    }

    fn take(&mut self) -> Option<Held> {
        self.expire_idle();
        self.held.take()
    }

    /// Whether a key is held (for tests and the UI).
    pub(crate) fn holding(&self) -> bool {
        self.held.is_some()
    }
}

/// `signingWhenOk` (cms.ts:20).
pub fn signing_when_ok(when: f64, now: f64) -> bool {
    when.is_finite() && (when - now).abs() <= SIGNING_SKEW_MS
}

fn read_small(path: &Path) -> Option<Vec<u8>> {
    let data = safe_write::read_capped(path, MAX_CERT_FILE + 1).ok()?;
    (!data.is_empty() && data.len() as u64 <= MAX_CERT_FILE).then_some(data)
}

/// `NOT_FOR_SIGNING_BOX` (MSG.sign.notForSigning).
pub fn not_for_signing_box() -> MessageBox {
    MessageBox {
        question: Question::NotForSigning,
        kind: BoxKind::Warning,
        message: "This certificate isn't meant for signing documents, so other apps may not trust the signature. Sign anyway?".into(),
        detail: None,
        buttons: vec!["Sign anyway".into(), "Cancel".into()],
        default: 1,
        cancel: 1,
    }
}

impl Host {
    /// Whether a certificate key is held right now.
    pub fn holding_certificate(&self) -> bool {
        self.cert.holding()
    }

    /// `pick-certificate` (M:1066): `path` is a drive's pre-answer (used only in a
    /// smoke/drive run, verbatim); otherwise the native sheet.
    pub fn pick_certificate(&mut self, path: Option<&str>) -> Result<String, Reason> {
        let given = path
            .filter(|p| self.smoke && !p.is_empty())
            .map(PathBuf::from);
        let path = match given {
            Some(p) => p,
            None => self
                .platform
                .pick_file(Pick::Certificate, &[CERTIFICATE_FILTER])
                .ok_or("cancelled")?,
        };
        self.cert.held = None;
        let name = basename(&path);
        self.cert.path = Some(path);
        Ok(name)
    }

    /// `certificate-info` (M:1086): reads the picked file with `password`, checks
    /// it can sign now and holds its key for one signature.
    pub fn certificate_info(&mut self, password: &str) -> Result<CertInfo, SignFail> {
        if password.encode_utf16().count() > MAX_PASSWORD_UNITS {
            return Err(SignFail::of("unreadable"));
        }
        let path = self
            .cert
            .path
            .clone()
            .ok_or(SignFail::of("no-certificate"))?;
        // A new attempt drops any key a previous attempt left, so a failed retry never signs with it.
        self.cert.held = None;
        let mut data = read_small(&path).ok_or(SignFail::of("unreadable"))?;
        let pem = path.to_string_lossy().to_lowercase().ends_with(".pem");
        let parsed = if pem {
            Identity::from_pem(&data, password)
        } else {
            Identity::from_p12(&data, password)
        };
        data.zeroize();
        let identity = parsed.map_err(|e| SignFail::from_error(&e))?;
        let facts = identity
            .check_usable(crate::engine::now_ms() as i64)
            .map_err(|e| SignFail::from_error(&e))?;
        // A certificate whose key usage excludes document signing: other readers may not trust the
        // signature, so ask first (a drive or smoke proceeds and reports the warning).
        let warning = (!facts.for_signing).then_some("not-for-signing");
        if warning.is_some() && !self.smoke && self.platform.ask(&not_for_signing_box()) != 0 {
            return Err(SignFail::of("cancelled"));
        }
        let info = CertInfo {
            cn: facts.cn.clone(),
            not_after: time::iso(facts.not_after_ms),
            certs: identity.certs.len(),
            warning,
        };
        self.cert.held = Some(Held {
            identity,
            cn: facts.cn,
            warning,
            since: Instant::now(),
        });
        Ok(info)
    }

    /// `certificate-clear` (M:1117).
    pub fn certificate_clear(&mut self) {
        self.cert.held = None;
    }

    /// `sign-file` (M:1123): signs the prepared PDF with the held key (taking
    /// it), asks where the signed copy goes and writes it. Outside a smoke run
    /// the copy then opens in place of the original with no discard question.
    pub fn sign_file(
        &mut self,
        bytes: Vec<u8>,
        offsets: Offsets,
        when: f64,
        suggested_name: Option<&str>,
    ) -> Result<Signed, SignFail> {
        if self.save_busy {
            return Err(SignFail::of("busy"));
        }
        let held = self.cert.take().ok_or(SignFail::of("no-certificate"))?;
        self.save_busy = true;
        let result = self.sign_file_inner(bytes, offsets, when, suggested_name, held);
        self.save_busy = false;
        result
    }

    fn sign_file_inner(
        &mut self,
        mut bytes: Vec<u8>,
        offsets: Offsets,
        when: f64,
        suggested_name: Option<&str>,
        held: Held,
    ) -> Result<Signed, SignFail> {
        if bytes.len() > MAX_SAVE_BYTES {
            return Err(SignFail::of("too-large"));
        }
        if !signing_when_ok(when, crate::engine::now_ms()) {
            return Err(SignFail::of("invalid"));
        }
        let Held {
            identity,
            cn,
            warning,
            ..
        } = held;
        // One shot: check the certificate at `when`, sign, drop the key.
        identity
            .check_usable(when as i64)
            .map_err(|e| SignFail::from_error(&e))?;
        let mut seed = crate::engine::entropy(tandem_sign::drbg::MIN_SEED);
        let rng = HmacDrbg::new(&seed);
        seed.zeroize();
        let mut rng = rng.ok_or(SignFail::of("unreadable"))?;
        let signed = cms::sign_pdf(
            &mut bytes,
            offsets,
            &identity.key,
            &identity.certs,
            &mut rng,
        );
        drop(identity);
        signed.map_err(|e| {
            SignFail::of(if e == SignError::CmsTooLarge {
                "cms-too-large"
            } else {
                "unreadable"
            })
        })?;
        let default_path = suggested_kind_name(suggested_name, SaveKind::Pdf)
            .unwrap_or_else(|| "signed.pdf".into());
        let dest = self
            .platform
            .pick_save(
                SavePurpose::Signed,
                Path::new(&default_path),
                &[crate::platform::PDF_FILTER],
            )
            .ok_or(SignFail::of("cancelled"))?;
        let written =
            safe_write::write_new(Some(&dest), &bytes, Some(&self.journal), &Inject::default())
                .map_err(|e| {
                    SignFail::of(match e {
                        SaveFail::Cancelled => "cancelled",
                        SaveFail::TooLarge => "too-large",
                        SaveFail::Permission => "permission",
                        SaveFail::DiskFull => "disk-full",
                        SaveFail::BrokenLink => "broken-link",
                        _ => "unreadable",
                    })
                })?;
        // D6 (UX item 9): the signed copy opens in place of the original, whose unsaved edits it carries,
        // so no discard question. A smoke or drive stays on the original, as its scripts expect.
        let opened = !self.smoke;
        if opened {
            let path = written
                .file
                .as_ref()
                .map_or(dest.clone(), |f| f.path.clone());
            self.deliver_open(Some(&path), true);
        }
        Ok(Signed {
            name: written.name,
            signer: cn,
            opened,
            warning,
        })
    }

    /// `verify-signatures` (M:1174): a verdict per engine `sign-list` entry
    /// (each entry's `contents` as a hex string or an array of byte values),
    /// against the trusted list; also the trusted list's `{cn, notAfter}`.
    pub fn verify_signatures(&mut self, pdf: &[u8], entries: &[Value]) -> Result<Value, Reason> {
        if pdf.len() > MAX_SAVE_BYTES {
            return Err("too-large");
        }
        if entries.len() > MAX_VERIFY_ENTRIES {
            return Err("too-many");
        }
        let trusted = self.trust.certs();
        let anchors = verify::parse_certs(trusted.iter().map(|c| c.raw.as_slice()));
        let now = crate::engine::now_ms() as i64;
        let verdicts: Vec<Value> = entries
            .iter()
            .map(|entry| verify_one(pdf, entry, &anchors, now))
            .collect();
        let list: Vec<Value> = trusted
            .iter()
            .map(|c| json!({"cn": c.cn, "notAfter": c.not_after}))
            .collect();
        Ok(json!({"verdicts": verdicts, "trusted": list}))
    }

    /// `trust-add` (M:1188): adds the certificates in a PEM/CRT file (`path` is
    /// a drive's pre-answer, used verbatim in a smoke/drive run): `(added, total)`.
    pub fn trust_add(&mut self, path: Option<&str>) -> Result<(usize, usize), Reason> {
        let given = path
            .filter(|p| self.smoke && !p.is_empty())
            .map(PathBuf::from);
        let path = match given {
            Some(p) => p,
            None => self
                .platform
                .pick_file(Pick::TrustCertificates, &[TRUST_FILTER])
                .ok_or("cancelled")?,
        };
        self.cert.trust_file = Some(basename(&path));
        let data = read_small(&path).ok_or("unreadable")?;
        // Round 2: a file without one certificate (a PDF, say) is refused, not "already trusted".
        if super::trust::parse_blocks(&String::from_utf8_lossy(&data)).is_empty() {
            return Err("no-certificate");
        }
        self.trust.add(&data).map_err(|_| "unreadable")
    }

    /// `trust-clear` (M:1213).
    pub fn trust_clear(&mut self) -> bool {
        self.trust.clear().is_ok()
    }

    /// Asks before "Forget trusted certificates" in a window (a smoke/drive run never asks).
    pub fn confirm_forget_trust(&mut self) -> bool {
        if self.smoke {
            return true;
        }
        self.platform.ask(&forget_trust_box()) == 0
    }

    /// `signature-get` (M:1219): the saved drawing or image, or `None`.
    pub fn signature_get(&self) -> Option<Value> {
        self.signature.get()
    }

    /// `signature-set` (M:1223): replaces (or with `None` removes) it; `false` when it is not valid.
    pub fn signature_set(&mut self, value: Option<&Value>) -> bool {
        self.signature.set(value)
    }
}

/// `wire(entry)` (verify.ts): `contents` as hex (or `false` past 64 KiB or of
/// another type), every string bounded.
fn wire(entry: &Value) -> Value {
    let Some(rec) = entry.as_object() else {
        return entry.clone();
    };
    let mut out = Map::new();
    for (key, value) in rec {
        let wired = if key == "contents" {
            match value {
                Value::String(hex)
                    if hex.len() / 2 <= MAX_CONTENTS
                        && hex.len() % 2 == 0
                        && hex.bytes().all(|b| b.is_ascii_hexdigit()) =>
                {
                    value.clone()
                }
                Value::Array(items)
                    if items.len() <= MAX_CONTENTS
                        && items.iter().all(|i| i.as_u64().is_some_and(|n| n < 256)) =>
                {
                    Value::String(
                        items
                            .iter()
                            .filter_map(Value::as_u64)
                            .map(|n| format!("{n:02x}"))
                            .collect(),
                    )
                }
                Value::Null => continue,
                _ => Value::Bool(false),
            }
        } else if let Value::String(s) = value {
            // One cut just past the limit (still over it, so still malformed).
            if s.chars().count() > MAX_STRING {
                Value::String(s.chars().take(MAX_STRING + 2).collect())
            } else {
                value.clone()
            }
        } else {
            value.clone()
        };
        out.insert(key.clone(), wired);
    }
    Value::Object(out)
}

/// `shell(entry)` (verify.ts): the verdict for an entry that could not be checked.
fn shell(entry: &Value) -> Value {
    let field = entry
        .get("field")
        .and_then(Value::as_str)
        .map_or(String::new(), |s| s.chars().take(MAX_STRING).collect());
    let page = entry
        .get("page")
        .and_then(Value::as_f64)
        .filter(|n| n.fract() == 0.0)
        .map_or(json!(0), js_number);
    json!({
        "field": field, "page": page, "status": "unsupported", "reason": "malformed", "trust": "none",
        "coverage": "none", "plain": "This signature could not be read, so it was not checked.",
    })
}

/// One verdict, as `tandem-sign`'s WASM `verifySignatures` built it.
fn verify_one(pdf: &[u8], entry: &Value, anchors: &[Cert], now: i64) -> Value {
    let wired = wire(entry);
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let v = verify::verify_signature(&pdf, &wired, anchors, now);
        let mut j = v.to_json();
        j["plain"] = json!(report::plain_sentence(&v));
        if let Some(name) = v.signer_cn.as_deref().and_then(report::clean_name) {
            j["signerName"] = json!(name);
        }
        if let Some(name) = v.issuer_cn.as_deref().and_then(report::clean_name) {
            j["issuerName"] = json!(name);
        }
        if let Some(reason) = &v.reason {
            j["reasonWords"] = json!(report::reason_words(reason));
        }
        j
    }));
    run.unwrap_or_else(|_| shell(entry))
}

/// Re-exported for the session: the hole `sign-prepare` reported.
pub type SignOffsets = Offsets;

/// The question before forgetting every trusted certificate; Return keeps them.
pub fn forget_trust_box() -> MessageBox {
    MessageBox {
        question: Question::ForgetTrust,
        kind: BoxKind::Warning,
        message: "Forget all the certificates you trust?".into(),
        detail: Some(
            "Signatures made with them will show as not trusted until you trust the certificates again.".into(),
        ),
        buttons: vec!["Forget".into(), "Cancel".into()],
        default: 1,
        cancel: 1,
    }
}
