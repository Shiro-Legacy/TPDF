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

use crate::cert::Cert;

/// The longest chain followed.
pub const MAX_CHAIN: usize = 5;

/// Whether `issuer` issued `cert`.
pub fn issued_by(cert: &Cert, issuer: &Cert) -> bool {
    if !issuer.is_ca || cert.issuer_bytes() != issuer.subject_bytes() {
        return false;
    }
    if let (Some(akid), Some(skid)) = (&cert.authority_key_id, &issuer.subject_key_id) {
        if akid != skid {
            return false;
        }
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
        if let Some(issuer) = store.iter().find(|c| issued_by(current, c)) {
            chain.push(issuer);
            return Some(Anchor {
                anchor: issuer,
                chain,
            });
        }
        let next = embedded
            .iter()
            .find(|c| !seen.contains(&c.fingerprint256()) && issued_by(current, c))?;
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
