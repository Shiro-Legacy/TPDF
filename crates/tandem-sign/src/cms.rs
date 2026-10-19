// SPDX-License-Identifier: MIT
//! Detached CMS SignedData (RFC 5652) for PDF signatures, and patching it
//! into the hole a prepared PDF leaves for it.
//!
//! The signatures are PAdES baseline B-B (ETSI EN 319 142-1): the engine
//! writes SubFilter `ETSI.CAdES.detached` and the claimed signing time as the
//! signature dictionary's `/M`, and the SignerInfo carries three signed
//! attributes: content-type, message-digest and the ESS
//! signing-certificate-v2 (RFC 5035), which binds the signer's certificate
//! into the signature. PAdES forbids the CMS signing-time attribute, so there
//! is none. Certificates are embedded byte for byte as the identity holds them.

use crate::asn1::{self, algorithm, oid, seq, set_of, tlv, uint};
use crate::cert::Cert;
use crate::digest::DigestAlg;
use crate::identity::SignError;
use crate::keys::SigningKey;
use crate::oids;
use crate::source::{Source, hash_ranges};
use rand_core::CryptoRngCore;

/// The CMS size the PDF reserves (`src/renderer/limits.ts` MAX_SIG_CONTENTS_BYTES;
/// change both together). The hole is twice this many hex digits.
pub const MAX_SIG_CONTENTS_BYTES: u64 = 8192;
/// What the engine writes as the `/ByteRange` value before signing; the
/// signer checks these 36 bytes and writes the real ranges over them.
pub const BYTE_RANGE_SENTINEL: &[u8; 36] = b"[0 1000000000 1000000000 1000000000]";

/// The ESS signing-certificate-v2 value: one ESSCertIDv2 with the SHA-256
/// of the leaf (the default hash, so no AlgorithmIdentifier) and its
/// IssuerSerial.
fn signing_certificate_v2(leaf: &Cert) -> Vec<u8> {
    let cert_hash = tlv(0x04, &DigestAlg::Sha256.hash(&leaf.der));
    let general_names = seq(&[&tlv(0xa4, leaf.issuer_bytes())]);
    let issuer_serial = seq(&[&general_names, leaf.serial_bytes()]);
    let ess_cert_id = seq(&[&cert_hash, &issuer_serial]);
    seq(&[&seq(&[&ess_cert_id])])
}

fn attribute(id: &const_oid::ObjectIdentifier, value: Vec<u8>) -> Vec<u8> {
    seq(&[&oid(id), &set_of(&[value])])
}

/// The signed attributes as a DER SET (tag 0x31), the bytes the signature covers.
pub fn signed_attributes(content_digest: &[u8], leaf: &Cert) -> Vec<u8> {
    set_of(&[
        attribute(&oids::CONTENT_TYPE, oid(&oids::DATA)),
        attribute(&oids::MESSAGE_DIGEST, tlv(0x04, content_digest)),
        attribute(&oids::SIGNING_CERTIFICATE_V2, signing_certificate_v2(leaf)),
    ])
}

/// SignedData over content whose `key.digest()` hash is `content_digest`.
/// `certs[0]` is the leaf.
pub fn sign_digest(
    key: &SigningKey,
    certs: &[Vec<u8>],
    content_digest: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<Vec<u8>, SignError> {
    let alg = key.digest();
    let leaf = certs
        .first()
        .and_then(|d| Cert::parse(d))
        .ok_or(SignError::Unreadable)?;
    let attrs = signed_attributes(content_digest, &leaf);
    let signature = key
        .sign_hash(&alg.hash(&attrs), rng)
        .ok_or(SignError::Unreadable)?;
    let mut implicit = attrs.clone();
    implicit[0] = 0xa0;
    let signer_info = seq(&[
        &uint(1),
        &seq(&[leaf.issuer_bytes(), leaf.serial_bytes()]),
        &algorithm(&alg.oid(), false),
        &implicit,
        &key.signature_algorithm(),
        &tlv(0x04, &signature),
    ]);
    let signed_data = seq(&[
        &uint(1),
        &set_of(&[algorithm(&alg.oid(), false)]),
        &seq(&[&oid(&oids::DATA)]),
        &tlv(0xa0, &certs.concat()),
        &set_of(&[signer_info]),
    ]);
    Ok(seq(&[&oid(&oids::SIGNED_DATA), &tlv(0xa0, &signed_data)]))
}

/// SignedData over `content` (hashed here).
pub fn sign_detached(
    key: &SigningKey,
    certs: &[Vec<u8>],
    content: &[u8],
    rng: &mut impl CryptoRngCore,
) -> Result<Vec<u8>, SignError> {
    sign_digest(key, certs, &key.digest().hash(content), rng)
}

/// Where the engine's `sign-prepare` put the hole and the ByteRange.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offsets {
    /// The first hex digit inside `<…>`.
    pub contents_at: u64,
    /// The number of hex digits.
    pub contents_len: u64,
    /// The `[` of the sentinel.
    pub byte_range_at: u64,
}

/// The checked hole and the ByteRange text to write over the sentinel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub offsets: Offsets,
    /// The `<` of the hole: the first range is `[0, gap_start)`.
    pub gap_start: u64,
    /// One past the `>`: the second range is `[second, len)`.
    pub second: u64,
    pub len: u64,
    pub byte_range: [u8; 36],
}

/// Checks the sentinel and the all-zero hole (anything else means the bytes
/// changed after `sign-prepare`) and works out the ByteRange.
pub fn plan(src: &dyn Source, o: Offsets) -> Result<Plan, SignError> {
    let len = src.len();
    let bad = || SignError::Unreadable;
    if o.contents_len != MAX_SIG_CONTENTS_BYTES * 2 || o.contents_at < 1 {
        return Err(bad());
    }
    if o.byte_range_at.checked_add(36).is_none_or(|e| e > len) {
        return Err(bad());
    }
    if o.contents_at + o.contents_len >= len {
        return Err(bad());
    }
    if src.read_at(o.byte_range_at, 36).as_deref() != Some(BYTE_RANGE_SENTINEL.as_slice()) {
        return Err(bad());
    }
    let hole = src
        .read_at(
            o.contents_at - 1,
            usize::try_from(o.contents_len + 2).map_err(|_| bad())?,
        )
        .ok_or_else(bad)?;
    let n = hole.len();
    if hole[0] != b'<' || hole[n - 1] != b'>' || hole[1..n - 1].iter().any(|&b| b != b'0') {
        return Err(bad());
    }
    let gap_start = o.contents_at - 1;
    let second = o.contents_at + o.contents_len + 1;
    let tail = len - second;
    const MAX10: u64 = 9_999_999_999;
    if gap_start > MAX10 || second > MAX10 || tail > MAX10 {
        return Err(bad());
    }
    let text = format!("[0 {gap_start:010} {second:010} {tail:010}]");
    let mut byte_range = [0u8; 36];
    byte_range.copy_from_slice(text.as_bytes());
    Ok(Plan {
        offsets: o,
        gap_start,
        second,
        len,
        byte_range,
    })
}

/// The two writes that sign the PDF: the ByteRange text at
/// `offsets.byte_range_at`, then the CMS hex at `offsets.contents_at`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    pub plan: Plan,
    /// Uppercase hex of the CMS, at most `contents_len` digits (the rest of
    /// the hole stays `0`).
    pub hex: String,
}

/// The hash of the signed ranges, as they read once the ByteRange is written.
pub fn content_digest(src: &dyn Source, plan: &Plan, alg: DigestAlg) -> Option<Vec<u8>> {
    hash_ranges(
        src,
        alg,
        &[(0, plan.gap_start), (plan.second, plan.len)],
        Some((plan.offsets.byte_range_at, plan.byte_range.as_slice())),
    )
}

/// Signs a prepared PDF: checks the hole, hashes the ranges and returns the
/// writes ([`Patch`]); the caller applies them to its own copy.
pub fn sign_prepared(
    src: &dyn Source,
    offsets: Offsets,
    key: &SigningKey,
    certs: &[Vec<u8>],
    rng: &mut impl CryptoRngCore,
) -> Result<Patch, SignError> {
    let plan = plan(src, offsets)?;
    let digest = content_digest(src, &plan, key.digest()).ok_or(SignError::Unreadable)?;
    let cms = sign_digest(key, certs, &digest, rng)?;
    let hex: String = cms.iter().map(|b| format!("{b:02X}")).collect();
    if hex.len() as u64 > offsets.contents_len {
        return Err(SignError::CmsTooLarge);
    }
    Ok(Patch { plan, hex })
}

impl Patch {
    /// Applies the writes to `bytes` (the same bytes the plan was made from).
    pub fn apply(&self, bytes: &mut [u8]) {
        let br = self.plan.offsets.byte_range_at as usize;
        let at = self.plan.offsets.contents_at as usize;
        bytes[br..br + 36].copy_from_slice(&self.plan.byte_range);
        bytes[at..at + self.hex.len()].copy_from_slice(self.hex.as_bytes());
    }
}

/// Signs a prepared PDF held in memory, in place.
pub fn sign_pdf(
    bytes: &mut [u8],
    offsets: Offsets,
    key: &SigningKey,
    certs: &[Vec<u8>],
    rng: &mut impl CryptoRngCore,
) -> Result<(), SignError> {
    let view: &[u8] = bytes;
    let patch = sign_prepared(&view, offsets, key, certs, rng)?;
    patch.apply(bytes);
    Ok(())
}

/// Parses a node of a CMS this crate wrote, for tests.
#[doc(hidden)]
pub fn top_children(cms: &[u8]) -> Option<Vec<asn1::Tlv>> {
    asn1::children(cms, &asn1::parse(cms, 0)?)
}
