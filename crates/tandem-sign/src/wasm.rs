// SPDX-License-Identifier: MIT
//! The WASM interface Electron's main process loads (`src/sign/native.ts`).
//!
//! - Private keys stay inside the module as [`Key`] objects; the host holds
//!   only the handle and frees it (which zeroes the key) when done.
//! - The PDF stays in the host's memory: the module reads the windows and
//!   hash chunks it needs through a `Reader` object (`read(offset, len)`).
//! - Results are JSON strings; failures carry the reason token the app's
//!   message catalogue knows (`bad-password`, `unsupported-key`, …).
//! - The host passes the time (ms since 1970) and at least 48 random bytes
//!   for each signature; the module has no clock or entropy of its own.

use crate::cert::Cert;
use crate::cms::{self, Offsets};
use crate::drbg::HmacDrbg;
use crate::identity::{self, SignError};
use crate::keys::SigningKey;
use crate::source::Source;
use crate::{report, time, verify};
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;
use zeroize::Zeroize;

#[wasm_bindgen]
extern "C" {
    /// A host object with `read(offset, length)` returning that many bytes.
    pub type Reader;
    #[wasm_bindgen(method)]
    fn read(this: &Reader, offset: f64, len: u32) -> Vec<u8>;
}

struct JsSource<'a> {
    reader: &'a Reader,
    len: u64,
}

impl Source for JsSource<'_> {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_at(&self, offset: u64, len: usize) -> Option<Vec<u8>> {
        if offset.checked_add(len as u64)? > self.len {
            return None;
        }
        let bytes = self.reader.read(offset as f64, u32::try_from(len).ok()?);
        (bytes.len() == len).then_some(bytes)
    }
}

/// A non-negative integer below 2^53, or `None`.
fn whole(n: f64) -> Option<u64> {
    (n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n <= 9_007_199_254_740_991.0)
        .then_some(n as u64)
}

fn millis(n: f64) -> Option<i64> {
    (n.is_finite() && n.abs() <= 8.64e15).then_some(n as i64)
}

fn fail(reason: &str) -> JsError {
    JsError::new(reason)
}

/// `[u32 big-endian length][bytes]` repeated.
fn unpack(blob: &[u8]) -> Option<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < blob.len() {
        let len = u32::from_be_bytes(blob.get(at..at + 4)?.try_into().ok()?) as usize;
        at += 4;
        out.push(blob.get(at..at.checked_add(len)?)?.to_vec());
        at += len;
    }
    Some(out)
}

fn error_json(e: &SignError) -> String {
    let mut v = json!({ "ok": false, "reason": e.reason() });
    if let SignError::CertificateExpired { date, .. } = e {
        v["date"] = json!(date);
        v["status"] = json!(e.status());
    }
    v.to_string()
}

/// A private key held inside the module. `free()` drops and zeroes it.
#[wasm_bindgen]
pub struct Key {
    inner: SigningKey,
}

#[wasm_bindgen]
impl Key {
    /// `rsa` or `ec`.
    #[wasm_bindgen(getter)]
    pub fn kind(&self) -> String {
        self.inner.kind().to_string()
    }
    /// The RSA modulus size or the curve size, in bits.
    #[wasm_bindgen(getter)]
    pub fn bits(&self) -> u32 {
        self.inner.bits() as u32
    }
    /// `P-256` or `P-384` for an EC key.
    #[wasm_bindgen(getter)]
    pub fn curve(&self) -> Option<String> {
        self.inner.curve().map(str::to_string)
    }
}

/// What a certificate file held: take the key out, read the certificates
/// (leaf first), then free it.
#[wasm_bindgen]
pub struct Identity {
    key: Option<SigningKey>,
    certs: Vec<Vec<u8>>,
}

#[wasm_bindgen]
impl Identity {
    #[wasm_bindgen(js_name = takeKey)]
    pub fn take_key(&mut self) -> Option<Key> {
        self.key.take().map(|inner| Key { inner })
    }
    #[wasm_bindgen(js_name = certCount)]
    pub fn cert_count(&self) -> u32 {
        self.certs.len() as u32
    }
    pub fn cert(&self, index: u32) -> Option<Vec<u8>> {
        self.certs.get(index as usize).cloned()
    }
}

fn identity(result: Result<identity::Identity, SignError>) -> Result<Identity, JsError> {
    let id = result.map_err(|e| fail(e.reason()))?;
    Ok(Identity {
        key: Some(id.key),
        certs: id.certs,
    })
}

/// Reads a certificate file with `read`, then zeroes the file bytes and the
/// password (UTF-8) in place: wasm-bindgen copies a `&mut [u8]` back to the
/// caller's array, so the host's buffers are wiped too.
fn read_identity(
    bytes: &mut [u8],
    password: &mut [u8],
    read: fn(&[u8], &str) -> Result<identity::Identity, SignError>,
) -> Result<Identity, JsError> {
    let result = match std::str::from_utf8(password) {
        Ok(text) => read(bytes, text),
        Err(_) => Err(SignError::Unreadable),
    };
    bytes.zeroize();
    password.zeroize();
    identity(result)
}

/// Reads a PKCS#12 file. Throws with the reason token. Zeroes both inputs.
#[wasm_bindgen(js_name = parseP12)]
pub fn parse_p12(bytes: &mut [u8], password: &mut [u8]) -> Result<Identity, JsError> {
    read_identity(bytes, password, identity::Identity::from_p12)
}

/// Reads PEM text with a key and certificates. Throws with the reason token.
/// Zeroes both inputs.
#[wasm_bindgen(js_name = parsePem)]
pub fn parse_pem(bytes: &mut [u8], password: &mut [u8]) -> Result<Identity, JsError> {
    read_identity(bytes, password, identity::Identity::from_pem)
}

/// `{"ok":true,"cn","notAfter"}` when the leaf can sign at `when`, else
/// `{"ok":false,"reason"[,"date","status"]}`.
#[wasm_bindgen(js_name = checkUsable)]
pub fn check_usable(leaf: &[u8], key: &Key, when: f64) -> String {
    let Some(when) = millis(when) else {
        return error_json(&SignError::Unreadable);
    };
    match identity::check_usable(&[leaf.to_vec()], &key.inner, when) {
        Ok(u) => {
            json!({ "ok": true, "cn": u.cn, "notAfter": time::iso(u.not_after_ms), "forSigning": u.for_signing })
                .to_string()
        }
        Err(e) => error_json(&e),
    }
}

/// Detached CMS SignedData over `content`. `certs` is packed, leaf first.
#[wasm_bindgen(js_name = signDetached)]
pub fn sign_detached(
    key: &Key,
    certs: &[u8],
    content: &[u8],
    entropy: &mut [u8],
) -> Result<Vec<u8>, JsError> {
    let rng = HmacDrbg::new(entropy);
    entropy.zeroize();
    let mut rng = rng.ok_or_else(|| fail("unreadable"))?;
    let certs = unpack(certs).ok_or_else(|| fail("unreadable"))?;
    cms::sign_detached(&key.inner, &certs, content, &mut rng).map_err(|e| fail(e.reason()))
}

fn offsets(contents_at: f64, contents_len: f64, byte_range_at: f64) -> Option<Offsets> {
    Some(Offsets {
        contents_at: whole(contents_at)?,
        contents_len: whole(contents_len)?,
        byte_range_at: whole(byte_range_at)?,
    })
}

/// Checks a prepared PDF's hole: `{"ok":true,"gapStart","second","byteRange"}`
/// or `{"ok":false,"reason":"unreadable"}`.
#[wasm_bindgen(js_name = planSignature)]
pub fn plan_signature(
    reader: &Reader,
    len: f64,
    contents_at: f64,
    contents_len: f64,
    byte_range_at: f64,
) -> String {
    let (Some(len), Some(o)) = (
        whole(len),
        offsets(contents_at, contents_len, byte_range_at),
    ) else {
        return error_json(&SignError::Unreadable);
    };
    match cms::plan(&JsSource { reader, len }, o) {
        Ok(p) => json!({
            "ok": true,
            "gapStart": p.gap_start,
            "second": p.second,
            "byteRange": String::from_utf8_lossy(&p.byte_range),
        })
        .to_string(),
        Err(e) => error_json(&e),
    }
}

/// Signs a prepared PDF: `{"ok":true,"byteRangeAt","byteRange","contentsAt","hex"}`
/// (the two writes to make) or `{"ok":false,"reason"}`.
#[wasm_bindgen(js_name = signPrepared)]
#[allow(clippy::too_many_arguments)]
pub fn sign_prepared(
    reader: &Reader,
    len: f64,
    contents_at: f64,
    contents_len: f64,
    byte_range_at: f64,
    key: &Key,
    certs: &[u8],
    entropy: &mut [u8],
) -> String {
    let rng = HmacDrbg::new(entropy);
    entropy.zeroize();
    let (Some(len), Some(o), Some(certs), Some(mut rng)) = (
        whole(len),
        offsets(contents_at, contents_len, byte_range_at),
        unpack(certs),
        rng,
    ) else {
        return error_json(&SignError::Unreadable);
    };
    match cms::sign_prepared(&JsSource { reader, len }, o, &key.inner, &certs, &mut rng) {
        Ok(p) => json!({
            "ok": true,
            "byteRangeAt": p.plan.offsets.byte_range_at,
            "byteRange": String::from_utf8_lossy(&p.plan.byte_range),
            "contentsAt": p.plan.offsets.contents_at,
            "hex": p.hex,
        })
        .to_string(),
        Err(e) => error_json(&e),
    }
}

/// Verifies the engine's `sign-list` entries (JSON array, `contents` as hex)
/// against the trusted certificates (packed DER). Returns a JSON array of
/// verdicts, each with `plain`, the sentence for people.
#[wasm_bindgen(js_name = verifySignatures)]
pub fn verify_signatures(
    reader: &Reader,
    len: f64,
    entries: &str,
    trusted: &[u8],
    now: f64,
) -> Result<String, JsError> {
    let len = whole(len).ok_or_else(|| fail("unreadable"))?;
    let now = millis(now).ok_or_else(|| fail("unreadable"))?;
    let entries: Value = serde_json::from_str(entries).map_err(|_| fail("unreadable"))?;
    let ders = unpack(trusted).ok_or_else(|| fail("unreadable"))?;
    let anchors = verify::parse_certs(ders.iter().map(Vec::as_slice));
    let verdicts = verify::verify_all(&JsSource { reader, len }, &entries, &anchors, now);
    let out: Vec<Value> = verdicts
        .iter()
        .map(|v| {
            let mut j = v.to_json();
            j["plain"] = json!(report::plain_sentence(v));
            // Names and reasons for the Verify table: cleaned, and in words.
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
        })
        .collect();
    Ok(Value::Array(out).to_string())
}

/// `{"cn","fingerprint256","notBefore","notAfter","ca"}` for a DER
/// certificate, or `undefined` when it does not parse.
#[wasm_bindgen(js_name = certInfo)]
pub fn cert_info(der: &[u8]) -> Option<String> {
    let c = Cert::parse(der)?;
    Some(
        json!({
            "cn": c.subject_cn(),
            "fingerprint256": c.fingerprint256(),
            "notBefore": time::iso(c.not_before_ms),
            "notAfter": time::iso(c.not_after_ms),
            "ca": c.is_ca,
        })
        .to_string(),
    )
}

/// The CMS size the PDF reserves, for the host's consistency check.
#[wasm_bindgen(js_name = maxSigContentsBytes)]
pub fn max_sig_contents_bytes() -> u32 {
    cms::MAX_SIG_CONTENTS_BYTES as u32
}
