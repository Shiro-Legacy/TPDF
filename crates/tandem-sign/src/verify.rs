// SPDX-License-Identifier: MIT
//! Checks one PDF signature and reports what it proves.
//!
//! The input is the engine's `sign-list` entry for a signature field (as
//! JSON, with `contents` as hex) and random access to the file. The output is
//! a [`Verdict`]: whether the signature is intact, how much of the file it
//! covers, who signed and when, whether the signer is trusted, and the
//! certification level. Every rule matches the TypeScript verifier this
//! replaces, so the app's answers do not change; [`crate::report`] turns a
//! verdict into a sentence.

use crate::asn1::{Tlv, children, is_oid, oid_text, parse};
use crate::cert::Cert;
use crate::digest::DigestAlg;
use crate::keys::{PublicKey, pss_params};
use crate::oids;
use crate::source::{Source, hash_ranges};
use crate::time;
use crate::trust;
use serde_json::{Map, Value, json};

/// The largest signature value read (bytes, before hex).
pub const MAX_CONTENTS: usize = 64 * 1024;
/// Longer strings in an entry make it malformed.
pub const MAX_STRING: usize = 1000;
const STRINGS: [&str; 8] = [
    "field",
    "subFilter",
    "filter",
    "name",
    "reason",
    "location",
    "contact",
    "m",
];
const DETACHED: [&str; 2] = ["adbe.pkcs7.detached", "ETSI.CAdES.detached"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Valid,
    Invalid,
    Unsupported,
    Empty,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Invalid => "invalid",
            Self::Unsupported => "unsupported",
            Self::Empty => "empty",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    /// A chain reaches a certificate in the user's trusted list, and every
    /// link was valid at the signing time.
    Trusted,
    /// No chain reaches the trusted list.
    Untrusted,
    /// A chain reaches it, but a link was not valid at the signing time.
    Expired,
    /// Not checked (the signature is not valid).
    None,
}

impl Trust {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Untrusted => "untrusted",
            Self::Expired => "expired",
            Self::None => "none",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    /// The signature covers every byte of the file.
    EntireFile,
    /// It covers a revision; later changes were appended after signing.
    Partial,
    None,
}

impl Coverage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EntireFile => "entire-file",
            Self::Partial => "partial",
            Self::None => "none",
        }
    }
}

/// The verification report for one signature field.
#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub field: String,
    /// The page index from the entry (0 when it is not an integer).
    pub page: i64,
    pub status: Status,
    /// Why it is invalid or unsupported, or space-joined notes on a valid
    /// signature (`weak-digest`, `no-signing-time`, `changes-not-permitted`).
    pub reason: Option<String>,
    pub signer_cn: Option<String>,
    /// The trusted certificate's common name, when trusted.
    pub issuer_cn: Option<String>,
    pub signing_time_ms: Option<i64>,
    pub digest: Option<&'static str>,
    pub trust: Trust,
    pub coverage: Coverage,
    pub covered: Option<u64>,
    pub total: Option<u64>,
    /// The DocMDP permission level P of a certifying signature (1 no changes,
    /// 2 form filling and signing, 3 also comments); JSON null when the entry
    /// says it is not certifying, absent when the entry did not say.
    pub docmdp: Option<Value>,
}

impl Verdict {
    /// The JSON shape the app has always used (keys absent when unset).
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("field".into(), json!(self.field));
        m.insert("page".into(), json!(self.page));
        m.insert("status".into(), json!(self.status.as_str()));
        if let Some(r) = &self.reason {
            m.insert("reason".into(), json!(r));
        }
        m.insert("trust".into(), json!(self.trust.as_str()));
        m.insert("coverage".into(), json!(self.coverage.as_str()));
        if let Some(d) = &self.docmdp {
            m.insert("docmdp".into(), d.clone());
        }
        if let Some(v) = self.covered {
            m.insert("covered".into(), json!(v));
        }
        if let Some(v) = self.total {
            m.insert("total".into(), json!(v));
        }
        if let Some(v) = &self.signer_cn {
            m.insert("signerCn".into(), json!(v));
        }
        if let Some(v) = &self.issuer_cn {
            m.insert("issuerCn".into(), json!(v));
        }
        if let Some(v) = self.signing_time_ms {
            m.insert("signingTime".into(), json!(time::iso(v)));
        }
        if let Some(v) = self.digest {
            m.insert("digest".into(), json!(v));
        }
        Value::Object(m)
    }
}

/// JavaScript truthiness of a JSON value.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn slice_utf16(s: &str, max: usize) -> String {
    let units: Vec<u16> = s.encode_utf16().take(max).collect();
    String::from_utf16_lossy(&units)
}

/// A non-negative integer JSON number.
fn index(v: &Value) -> Option<u64> {
    let f = v.as_f64()?;
    (f >= 0.0 && f.fract() == 0.0 && f <= 9_007_199_254_740_991.0).then_some(f as u64)
}

/// The verdict's fixed fields, from whatever the entry holds.
fn shell(entry: &Value, status: Status, reason: Option<&str>) -> Verdict {
    let field = entry
        .get("field")
        .and_then(Value::as_str)
        .map(|s| slice_utf16(s, MAX_STRING))
        .unwrap_or_default();
    let page = entry
        .get("page")
        .and_then(Value::as_f64)
        .filter(|f| f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_991.0)
        .map_or(0, |f| f as i64);
    let docmdp = entry
        .get("docmdp")
        .filter(|d| d.is_null() || d.is_number())
        .cloned();
    Verdict {
        field,
        page,
        status,
        reason: reason.map(str::to_string),
        signer_cn: None,
        issuer_cn: None,
        signing_time_ms: None,
        digest: None,
        trust: Trust::None,
        coverage: Coverage::None,
        covered: None,
        total: None,
        docmdp,
    }
}

fn strings_ok(entry: &Value) -> bool {
    if !entry.get("field").is_some_and(Value::is_string) {
        return false;
    }
    STRINGS.iter().all(|k| match entry.get(*k) {
        None => true,
        Some(Value::String(s)) => js_len(s) <= MAX_STRING,
        Some(_) => false,
    })
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(2) {
        return None;
    }
    let nib = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    b.as_chunks::<2>()
        .0
        .iter()
        .map(|p| Some(nib(p[0])? << 4 | nib(p[1])?))
        .collect()
}

/// The covered prefix must end at a revision: `%%EOF` plus at most two CR/LF bytes.
fn at_revision(tail: &[u8]) -> bool {
    let mut end = tail.len();
    let mut eol = 0;
    while eol < 2 && end > 0 && (tail[end - 1] == b'\n' || tail[end - 1] == b'\r') {
        eol += 1;
        end -= 1;
    }
    end >= 5 && &tail[end - 5..end] == b"%%EOF"
}

/// A signature value padded with zero bytes after one DER node.
fn cms_end(cms: &[u8]) -> Option<usize> {
    let top = parse(cms, 0)?;
    if top.end == 0 || cms[top.end..].iter().any(|&b| b != 0) {
        return None;
    }
    Some(top.end)
}

/// The first value of the attribute with `id` among `attrs`.
fn attr_value(cms: &[u8], attrs: &[Tlv], id: &const_oid::ObjectIdentifier) -> Option<Tlv> {
    attrs.iter().find_map(|attr| {
        let body = children(cms, attr)?;
        if !is_oid(cms, body.first()?, id) {
            return None;
        }
        children(cms, body.get(1)?)?.first().copied()
    })
}

/// When the CMS carries an ESS signing-certificate(-v2) attribute, the first
/// certificate hash in it must be the leaf's (RFC 5035): otherwise the
/// signature could be presented with another certificate for the same key.
fn ess_mismatch(cms: &[u8], attrs: &[Tlv], leaf: &Cert) -> bool {
    let check = |id: &const_oid::ObjectIdentifier, v2: bool| -> Option<bool> {
        let value = attr_value(cms, attrs, id)?;
        let certs = children(cms, &value)?;
        let first = children(cms, certs.first()?)?;
        let cert_id = children(cms, first.first()?)?;
        let (alg, hash) = match (v2, cert_id.first()?) {
            (true, n) if n.tag == 0x30 => {
                let alg_kids = children(cms, n)?;
                (
                    DigestAlg::from_oid_bytes(alg_kids.first()?.value(cms)),
                    cert_id.get(1)?,
                )
            }
            (true, n) => (Some(DigestAlg::Sha256), n),
            (false, n) => (Some(DigestAlg::Sha1), n),
        };
        // An unknown hash cannot be checked; it is not counted as a mismatch.
        let alg = alg?;
        Some(hash.value(cms) != alg.hash(&leaf.der).as_slice())
    };
    check(&oids::SIGNING_CERTIFICATE_V2, true).unwrap_or(false)
        || check(&oids::SIGNING_CERTIFICATE, false).unwrap_or(false)
}

/// Verifies one entry. `trusted` is the user's trusted list and `now_ms` the
/// time used when the signature names none.
pub fn verify_signature(src: &dyn Source, entry: &Value, trusted: &[Cert], now_ms: i64) -> Verdict {
    if !entry.is_object() || !strings_ok(entry) {
        return shell(entry, Status::Unsupported, Some("malformed"));
    }
    if entry.get("empty").is_some_and(truthy) {
        return shell(entry, Status::Empty, None);
    }
    let range: Option<Vec<u64>> = entry
        .get("byteRange")
        .and_then(Value::as_array)
        .filter(|a| a.len() == 4)
        .and_then(|a| a.iter().map(index).collect());
    let contents = entry
        .get("contents")
        .and_then(Value::as_str)
        .and_then(hex_decode)
        .filter(|c| c.len() <= MAX_CONTENTS);
    let (Some(range), Some(contents)) = (range, contents) else {
        return shell(entry, Status::Unsupported, Some("malformed"));
    };
    let len = src.len();
    let (a, b, c, d) = (range[0], range[1], range[2], range[3]);
    if a != 0 || b < 1 || c < b || c > len || d > len - c || c < 1 {
        return shell(entry, Status::Invalid, Some("byte-range"));
    }
    // The gap must be exactly `<hex of contents>`.
    let gap_ok = c - b == 2 * contents.len() as u64 + 2
        && src.read_at(b, (c - b) as usize).is_some_and(|gap| {
            gap[0] == b'<'
                && gap[gap.len() - 1] == b'>'
                && std::str::from_utf8(&gap[1..gap.len() - 1])
                    .ok()
                    .and_then(hex_decode)
                    .is_some_and(|decoded| decoded == contents)
        });
    if !gap_ok || cms_end(&contents).is_none() {
        return shell(entry, Status::Invalid, Some("byte-range"));
    }
    let cms = contents.as_slice();
    let covered = c + d;
    let partial = covered != len;
    if partial {
        let from = covered.saturating_sub(7);
        let tail = src
            .read_at(from, (covered - from) as usize)
            .unwrap_or_default();
        if !at_revision(&tail) {
            return shell(entry, Status::Invalid, Some("byte-range"));
        }
    }
    let docmdp_one = entry.get("docmdp").and_then(Value::as_f64) == Some(1.0);
    let span = |mut v: Verdict| -> Verdict {
        v.coverage = if partial {
            Coverage::Partial
        } else {
            Coverage::EntireFile
        };
        v.covered = Some(covered);
        v.total = Some(len);
        v
    };
    let sub = entry.get("subFilter").and_then(Value::as_str).unwrap_or("");
    if !DETACHED.contains(&sub) {
        let why = if sub.is_empty() {
            "missing-subfilter"
        } else {
            sub
        };
        return span(shell(entry, Status::Unsupported, Some(why)));
    }
    let malformed = || shell(entry, Status::Unsupported, Some("malformed"));
    let Some(top) = parse(cms, 0).and_then(|t| children(cms, &t)) else {
        return span(malformed());
    };
    let Some(first) = top.first() else {
        return malformed();
    };
    if !is_oid(cms, first, &oids::SIGNED_DATA) {
        return span(malformed());
    }
    let Some(wrapper) = top.get(1) else {
        return malformed();
    };
    let Some(signed) = children(cms, wrapper).and_then(|k| k.first().copied()) else {
        return span(malformed());
    };
    let Some(parts) = children(cms, &signed) else {
        return span(malformed());
    };
    let cert_set = parts.iter().find(|p| p.tag == 0xa0);
    let infos = parts
        .iter()
        .rfind(|p| p.tag == 0x31)
        .and_then(|s| children(cms, s));
    let (Some(cert_set), Some(infos)) = (cert_set, infos) else {
        return span(shell(entry, Status::Unsupported, Some("signer-id")));
    };
    if infos.len() != 1 {
        return span(shell(entry, Status::Unsupported, Some("signer-id")));
    }
    let Some(info) = children(cms, &infos[0]).filter(|i| i.len() >= 5) else {
        return span(malformed());
    };
    if info[1].tag != 0x30 {
        return span(shell(entry, Status::Unsupported, Some("signer-id")));
    }
    let Some(cert_nodes) = children(cms, cert_set) else {
        return span(malformed());
    };
    let certs: Vec<&[u8]> = cert_nodes.iter().map(|n| n.raw(cms)).collect();
    let Some(sid) = children(cms, &info[1]).filter(|s| s.len() >= 2) else {
        return span(shell(entry, Status::Unsupported, Some("signer-id")));
    };
    let (want_issuer, want_serial) = (sid[0].raw(cms), sid[1].raw(cms));
    let leaf_der = certs.iter().find(|der| {
        let Some(top) = parse(der, 0) else {
            return false;
        };
        let Some(tbs) = children(der, &top).and_then(|k| k.first().copied()) else {
            return false;
        };
        let Some(fields) = children(der, &tbs) else {
            return false;
        };
        let skip = usize::from(fields.first().is_some_and(|f| f.tag == 0xa0));
        fields.get(skip).is_some_and(|s| s.raw(der) == want_serial)
            && fields
                .get(skip + 2)
                .is_some_and(|i| i.raw(der) == want_issuer)
    });
    let Some(leaf_der) = leaf_der else {
        return span(shell(entry, Status::Invalid, Some("signer-cert")));
    };
    let Some(leaf) = Cert::parse(leaf_der) else {
        return malformed();
    };
    let Some(leaf_key) = leaf.public_key() else {
        return malformed();
    };
    let signer = Some(leaf.subject_cn());
    let with_signer = |v: Verdict, digest: Option<&'static str>| -> Verdict {
        let mut v = span(v);
        v.signer_cn = signer.clone();
        v.digest = digest;
        v
    };
    let digest_node = info[2];
    let digest_kids = if digest_node.tag == 0x30 {
        children(cms, &digest_node)
    } else {
        None
    };
    let hash = digest_kids
        .as_ref()
        .and_then(|k| k.first())
        .and_then(|n| DigestAlg::from_oid_bytes(n.value(cms)));
    let Some(hash) = hash else {
        let why = digest_kids
            .as_ref()
            .and_then(|k| k.first())
            .map_or_else(|| "unknown-digest".to_string(), |n| oid_text(cms, n));
        return with_signer(shell(entry, Status::Unsupported, Some(&why)), None);
    };
    let name = Some(hash.name());
    let signed_attrs = info.iter().find(|p| p.tag == 0xa0);
    let signature = info.iter().rfind(|p| p.tag == 0x04);
    let (Some(signed_attrs), Some(signature)) = (signed_attrs, signature) else {
        return with_signer(
            shell(entry, Status::Unsupported, Some("no-signed-attrs")),
            name,
        );
    };
    let Some(attrs) = children(cms, signed_attrs) else {
        return span(malformed());
    };
    let digests: Vec<&Tlv> = attrs
        .iter()
        .filter(|attr| {
            children(cms, attr)
                .and_then(|b| b.first().copied())
                .is_some_and(|id| is_oid(cms, &id, &oids::MESSAGE_DIGEST))
        })
        .collect();
    let invalid = |why: &str| with_signer(shell(entry, Status::Invalid, Some(why)), name);
    if digests.len() != 1 {
        return invalid("digest");
    }
    // RFC 5652 5.3 and 11.1: exactly one content-type attribute, and for a PDF
    // signature it and the encapsulated content type are id-data.
    let content_types = attrs
        .iter()
        .filter(|attr| {
            children(cms, attr)
                .and_then(|b| b.first().copied())
                .is_some_and(|id| is_oid(cms, &id, &oids::CONTENT_TYPE))
        })
        .count();
    let attr_is_data = attr_value(cms, &attrs, &oids::CONTENT_TYPE)
        .is_some_and(|v| v.tag == 0x06 && is_oid(cms, &v, &oids::DATA));
    let econtent_is_data = parts
        .iter()
        .find(|p| p.tag == 0x30)
        .and_then(|e| children(cms, e))
        .and_then(|k| k.first().copied())
        .is_some_and(|id| is_oid(cms, &id, &oids::DATA));
    if content_types != 1 || !attr_is_data || !econtent_is_data {
        return invalid("content-type");
    }
    let digest_value = attr_value(cms, &attrs, &oids::MESSAGE_DIGEST);
    let Some(digest_value) = digest_value.filter(|v| v.tag == 0x04) else {
        return invalid("digest");
    };
    let Some(actual) = hash_ranges(src, hash, &[(0, b), (c, covered)], None) else {
        return invalid("digest");
    };
    if digest_value.value(cms) != actual.as_slice() {
        return invalid("digest");
    }
    let mut set_tagged = signed_attrs.raw(cms).to_vec();
    set_tagged[0] = 0x31;
    let sig_alg = info
        .iter()
        .rfind(|p| p.tag == 0x30)
        .and_then(|a| children(cms, a));
    let sig_oid = sig_alg
        .as_ref()
        .and_then(|k| k.first())
        .map(|n| oid_text(cms, n))
        .unwrap_or_default();
    let pss = sig_alg
        .as_ref()
        .and_then(|k| k.first())
        .is_some_and(|n| is_oid(cms, n, &oids::RSA_PSS));
    let key_type = leaf_key.key_type().to_string();
    let supported = key_type == "rsa" || (key_type == "ec" && !pss);
    if !supported {
        let why = if sig_oid.is_empty() {
            hash.name().to_string()
        } else {
            sig_oid
        };
        return with_signer(shell(entry, Status::Unsupported, Some(&why)), name);
    }
    if let PublicKey::OtherCurve(curve) = &leaf_key {
        // A curve this app cannot check is not evidence of tampering.
        return with_signer(shell(entry, Status::Unsupported, Some(curve)), name);
    }
    let pss_params = if pss {
        match sig_alg.as_ref().and_then(|k| pss_params(cms, k.get(1))) {
            Some(p) => Some(p),
            None => return invalid("signature"),
        }
    } else {
        None
    };
    let sig_hash = pss_params.map_or(hash, |p| p.hash);
    let ok = leaf_key.verify_hash(
        sig_hash,
        &sig_hash.hash(&set_tagged),
        signature.value(cms),
        pss_params,
    );
    if !ok {
        return invalid("signature");
    }
    if ess_mismatch(cms, &attrs, &leaf) {
        return invalid("signer-cert");
    }
    let time_node = attr_value(cms, &attrs, &oids::SIGNING_TIME);
    let mut when = time_node.and_then(|n| match n.tag {
        0x17 => time::parse_utc_time(n.value(cms)),
        0x18 => time::parse_generalized_time(n.value(cms)),
        _ => None,
    });
    if when.is_none() {
        when = entry
            .get("m")
            .and_then(Value::as_str)
            .and_then(time::parse_pdf_date);
    }
    let no_time = when.is_none();
    let when = when.unwrap_or(now_ms);
    let embedded: Vec<Cert> = certs.iter().filter_map(|d| Cert::parse(d)).collect();
    let anchor = trust::anchor_of(&leaf, &embedded, trusted);
    let refusal = anchor.as_ref().and_then(trust::assess);
    let trust = match &anchor {
        None => Trust::Untrusted,
        Some(_) if refusal.is_some() => Trust::Untrusted,
        Some(a) if a.chain.iter().any(|c| !c.valid_at(when)) => Trust::Expired,
        Some(_) => Trust::Trusted,
    };
    let notes: Vec<&str> = [
        (hash == DigestAlg::Sha1).then_some("weak-digest"),
        no_time.then_some("no-signing-time"),
        (docmdp_one && partial).then_some("changes-not-permitted"),
        refusal.map(trust::Refusal::note),
    ]
    .into_iter()
    .flatten()
    .collect();
    let reason = (!notes.is_empty()).then(|| notes.join(" "));
    let mut v = with_signer(shell(entry, Status::Valid, reason.as_deref()), name);
    if trust == Trust::Trusted {
        v.issuer_cn = anchor.map(|a| a.anchor.subject_cn());
    }
    v.signing_time_ms = Some(when);
    v.trust = trust;
    v
}

/// Verifies each entry of a JSON array; a non-array is treated as empty.
pub fn verify_all(
    src: &dyn Source,
    entries: &Value,
    trusted: &[Cert],
    now_ms: i64,
) -> Vec<Verdict> {
    entries
        .as_array()
        .map(|list| {
            list.iter()
                .map(|e| verify_signature(src, e, trusted, now_ms))
                .collect()
        })
        .unwrap_or_default()
}

/// Parses DER certificates, skipping any that do not parse.
pub fn parse_certs<'a>(ders: impl IntoIterator<Item = &'a [u8]>) -> Vec<Cert> {
    ders.into_iter().filter_map(Cert::parse).collect()
}
