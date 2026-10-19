// SPDX-License-Identifier: MIT
//! Chain building from a signer's certificate to the user's trusted list.
//!
//! Our own code rather than a TLS verifier: document signing has no host
//! names or TLS key usages. The rules (unchanged from the TypeScript code):
//! a trusted certificate with the leaf's own fingerprint trusts it directly;
//! otherwise each issuer must be a CA (basicConstraints cA), carry the child's
//! issuer name as its subject, match the child's authority key id when both
//! are present, allow certificate signing when it has a keyUsage, and its key
//! must verify the child's signature. Chains end at five links. There is no
//! revocation checking (no network).
//!
//! Added with the port (review of October 19, 2026): an issuer's
//! pathLenConstraint is honoured, a certificate with a critical extension
//! this code does not process cannot be a link, and [`assess`] refuses trust
//! when a link is signed with SHA-1 or by an RSA key under 2048 bits
//! ("outdated security") or when the signer's certificate is not meant for
//! signing (keyUsage without digitalSignature or nonRepudiation, or an
//! extKeyUsage without a document-signing purpose).

use crate::cert::Cert;

/// The longest chain followed.
pub const MAX_CHAIN: usize = 5;

/// Whether `issuer` issued `cert`, with `below` CA certificates already
/// between `cert` and the leaf (for the issuer's pathLenConstraint).
/// A certificate from the user's trusted list (`anchor`) is trusted as it
/// is, so its own unknown critical extensions do not stop the chain.
pub fn issued_by(cert: &Cert, issuer: &Cert, below: usize, anchor: bool) -> bool {
    if !issuer.is_ca || cert.issuer_bytes() != issuer.subject_bytes() {
        return false;
    }
    if issuer.unknown_critical && !anchor {
        return false;
    }
    if issuer.path_len.is_some_and(|max| below as u64 > max) {
        return false;
    }
    if let (Some(akid), Some(skid)) = (&cert.authority_key_id, &issuer.subject_key_id)
        && akid != skid
    {
        return false;
    }
    if issuer.key_cert_sign == Some(false) {
        return false;
    }
    issuer
        .public_key()
        .is_some_and(|key| cert.signature_verifies_under(&key))
}

/// The trusted certificate a chain from `leaf` reaches, as an index into
/// `store`, with the chain (leaf first; the anchor last when it was found as
/// an issuer). `embedded` are the certificates the signature carries.
pub struct Anchor<'a> {
    pub anchor: &'a Cert,
    pub chain: Vec<&'a Cert>,
}

pub fn anchor_of<'a>(
    leaf: &'a Cert,
    embedded: &'a [Cert],
    store: &'a [Cert],
) -> Option<Anchor<'a>> {
    let by_fp = |fp: &str| store.iter().find(|c| c.fingerprint256() == fp);
    let leaf_fp = leaf.fingerprint256();
    if let Some(itself) = by_fp(&leaf_fp) {
        return Some(Anchor {
            anchor: itself,
            chain: vec![leaf],
        });
    }
    let mut seen = vec![leaf_fp];
    let mut current = leaf;
    let mut chain = vec![leaf];
    for _ in 0..MAX_CHAIN {
        let below = chain.len() - 1;
        if let Some(issuer) = store.iter().find(|c| issued_by(current, c, below, true)) {
            chain.push(issuer);
            return Some(Anchor {
                anchor: issuer,
                chain,
            });
        }
        let next = embedded
            .iter()
            .find(|c| !seen.contains(&c.fingerprint256()) && issued_by(current, c, below, false))?;
        chain.push(next);
        let fp = next.fingerprint256();
        if let Some(stored) = by_fp(&fp) {
            return Some(Anchor {
                anchor: stored,
                chain,
            });
        }
        seen.push(fp);
        current = next;
    }
    None
}

/// The least RSA modulus a link or the signature itself may use.
pub const MIN_RSA_BITS: usize = 2048;

/// Why a chain that reaches the trusted list is still not trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A link is signed with SHA-1 or by an RSA key under 2048 bits, or the
    /// signature itself was made with such a key.
    OutdatedSecurity,
    /// The signer's certificate is not meant for signing documents.
    NotForSigning,
    /// A CA in the chain (not the trusted certificate itself) sets rules for
    /// the certificates under it (name or policy constraints) that this code
    /// does not check.
    UnsupportedCaRule,
}

impl Refusal {
    /// The note the verdict carries.
    pub fn note(self) -> &'static str {
        match self {
            Self::OutdatedSecurity => "outdated-security",
            Self::NotForSigning => "not-for-signing",
            Self::UnsupportedCaRule => "unsupported-ca-rule",
        }
    }
}

/// Checks a chain [`anchor_of`] found: every link's strength and the leaf's
/// purpose. `None` when nothing stands in the way of trust.
pub fn assess(anchor: &Anchor<'_>) -> Option<Refusal> {
    let chain = &anchor.chain;
    let weak_key = |c: &Cert| {
        c.public_key()
            .and_then(|k| k.rsa_bits())
            .is_some_and(|bits| bits < MIN_RSA_BITS)
    };
    let weak_link = chain
        .windows(2)
        .any(|pair| pair[0].signed_with_sha1() || weak_key(pair[1]));
    if weak_link || chain.first().is_some_and(|leaf| weak_key(leaf)) {
        return Some(Refusal::OutdatedSecurity);
    }
    // The intermediates: neither the leaf nor the trusted certificate at the end
    // (when the chain reached the list through an issuer).
    let intermediates = chain.len().saturating_sub(1);
    if chain.iter().take(intermediates).skip(1).any(|c| c.ca_rules) {
        return Some(Refusal::UnsupportedCaRule);
    }
    if !chain.first().is_some_and(|leaf| leaf.may_sign_documents()) {
        return Some(Refusal::NotForSigning);
    }
    None
}
