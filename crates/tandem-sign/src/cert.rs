// SPDX-License-Identifier: MIT
//! X.509 certificates: the fields signing and verification need, read with
//! the lenient DER walker so that a certificate OpenSSL accepts is not
//! refused over an encoding nit, plus raw byte ranges for exact comparisons.

use crate::asn1::{self, Tlv, children, parse};
use crate::digest::{self, DigestAlg};
use crate::keys::{PssParams, PublicKey, pss_params};
use crate::oids;
use crate::time;
use std::ops::Range;

/// A parsed certificate. Ranges index `der`.
pub struct Cert {
    pub der: Vec<u8>,
    pub tbs: Range<usize>,
    /// The serial INTEGER node, header included.
    pub serial: Range<usize>,
    /// The issuer and subject Name nodes, header included.
    pub issuer: Range<usize>,
    pub subject: Range<usize>,
    pub spki: Range<usize>,
    pub not_before_ms: i64,
    pub not_after_ms: i64,
    /// basicConstraints cA.
    pub is_ca: bool,
    /// keyUsage keyCertSign, `None` when the certificate has no keyUsage.
    pub key_cert_sign: Option<bool>,
    pub subject_key_id: Option<Vec<u8>>,
    pub authority_key_id: Option<Vec<u8>>,
    sig_alg: Range<usize>,
    signature: Range<usize>,
}

fn range(t: &Tlv) -> Range<usize> {
    t.start..t.end
}

fn time_of(bytes: &[u8], node: &Tlv) -> Option<i64> {
    match node.tag {
        0x17 => time::parse_utc_time(node.value(bytes)),
        0x18 => time::parse_generalized_time(node.value(bytes)),
        _ => None,
    }
}

/// A DirectoryString as text (UTF-8, Printable, IA5, Teletex as Latin-1,
/// BMP, Universal).
pub fn directory_string(bytes: &[u8], node: &Tlv) -> String {
    let v = node.value(bytes);
    match node.tag {
        0x1e => {
            let units: Vec<u16> = v
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_be_bytes(*c))
                .collect();
            String::from_utf16_lossy(&units)
        }
        0x1c => v
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| char::from_u32(u32::from_be_bytes(*c)).unwrap_or('\u{fffd}'))
            .collect(),
        0x14 => v.iter().map(|&b| char::from(b)).collect(),
        _ => String::from_utf8_lossy(v).into_owned(),
    }
}

/// The first commonName of a Name node, or "".
pub fn common_name(bytes: &[u8], name: &Tlv) -> String {
    let Some(rdns) = children(bytes, name) else {
        return String::new();
    };
    for rdn in rdns {
        for atv in children(bytes, &rdn).unwrap_or_default() {
            let Some(parts) = children(bytes, &atv) else {
                continue;
            };
            if parts.len() >= 2 && asn1::is_oid(bytes, &parts[0], &oids::COMMON_NAME) {
                return directory_string(bytes, &parts[1]);
            }
        }
    }
    String::new()
}

impl Cert {
    /// Parses a DER certificate; `None` when its structure is broken.
    pub fn parse(der: &[u8]) -> Option<Cert> {
        let top = parse(der, 0)?;
        if top.tag != 0x30 || top.end != der.len() {
            return None;
        }
        let parts = children(der, &top)?;
        let (tbs, sig_alg, sig) = (parts.first()?, parts.get(1)?, parts.get(2)?);
        if tbs.tag != 0x30 || sig_alg.tag != 0x30 || sig.tag != 0x03 {
            return None;
        }
        let fields = children(der, tbs)?;
        let skip = usize::from(fields.first()?.tag == 0xa0);
        let serial = fields.get(skip)?;
        let issuer = fields.get(skip + 2)?;
        let validity = fields.get(skip + 3)?;
        let subject = fields.get(skip + 4)?;
        let spki = fields.get(skip + 5)?;
        if serial.tag != 0x02 || issuer.tag != 0x30 || subject.tag != 0x30 || spki.tag != 0x30 {
            return None;
        }
        let times = children(der, validity)?;
        let not_before_ms = time_of(der, times.first()?)?;
        let not_after_ms = time_of(der, times.get(1)?)?;
        let mut cert = Cert {
            der: der.to_vec(),
            tbs: range(tbs),
            serial: range(serial),
            issuer: range(issuer),
            subject: range(subject),
            spki: range(spki),
            not_before_ms,
            not_after_ms,
            is_ca: false,
            key_cert_sign: None,
            subject_key_id: None,
            authority_key_id: None,
            sig_alg: range(sig_alg),
            signature: range(sig),
        };
        if let Some(ext_wrap) = fields.iter().skip(skip + 6).find(|f| f.tag == 0xa3) {
            let list = parse(der, ext_wrap.value_start)?;
            for ext in children(der, &list)? {
                let e = children(der, &ext)?;
                let (id, value) = (e.first()?, e.last()?);
                if value.tag != 0x04 {
                    return None;
                }
                let inner = value.value(der);
                if asn1::is_oid(der, id, &oids::BASIC_CONSTRAINTS) {
                    let bc = parse(inner, 0)?;
                    let kids = children(inner, &bc)?;
                    cert.is_ca = kids.first().is_some_and(|k| {
                        k.tag == 0x01 && k.value(inner).first().is_some_and(|&b| b != 0)
                    });
                } else if asn1::is_oid(der, id, &oids::KEY_USAGE) {
                    let bits = parse(inner, 0)?;
                    let v = bits.value(inner);
                    // Bit 5 (keyCertSign) of the first content byte after the unused-bits count.
                    cert.key_cert_sign = Some(v.get(1).is_some_and(|b| b & 0x04 != 0));
                } else if asn1::is_oid(der, id, &oids::SUBJECT_KEY_ID) {
                    cert.subject_key_id = Some(parse(inner, 0)?.value(inner).to_vec());
                } else if asn1::is_oid(der, id, &oids::AUTHORITY_KEY_ID) {
                    let akid = parse(inner, 0)?;
                    cert.authority_key_id = children(inner, &akid)?
                        .iter()
                        .find(|k| k.tag == 0x80)
                        .map(|k| k.value(inner).to_vec());
                }
            }
        }
        Some(cert)
    }

    pub fn tbs_bytes(&self) -> &[u8] {
        &self.der[self.tbs.clone()]
    }
    pub fn serial_bytes(&self) -> &[u8] {
        &self.der[self.serial.clone()]
    }
    pub fn issuer_bytes(&self) -> &[u8] {
        &self.der[self.issuer.clone()]
    }
    pub fn subject_bytes(&self) -> &[u8] {
        &self.der[self.subject.clone()]
    }
    pub fn spki_bytes(&self) -> &[u8] {
        &self.der[self.spki.clone()]
    }

    /// The subject's first commonName, or "".
    pub fn subject_cn(&self) -> String {
        parse(&self.der, self.subject.start)
            .map_or_else(String::new, |n| common_name(&self.der, &n))
    }

    pub fn public_key(&self) -> Option<PublicKey> {
        PublicKey::from_spki_der(self.spki_bytes())
    }

    pub fn fingerprint256(&self) -> String {
        digest::fingerprint256(&self.der)
    }

    /// Whether `when` (ms) lies within the validity period, ends included.
    pub fn valid_at(&self, when: i64) -> bool {
        when >= self.not_before_ms && when <= self.not_after_ms
    }

    /// Whether `issuer`'s key signed this certificate (the TBS bytes under
    /// the certificate's signature algorithm). Names and constraints are
    /// checked by the caller ([`crate::trust`]).
    pub fn signature_verifies_under(&self, issuer: &PublicKey) -> bool {
        let der = &self.der;
        let Some(alg) = parse(der, self.sig_alg.start) else {
            return false;
        };
        let Some(alg_kids) = children(der, &alg) else {
            return false;
        };
        let Some(id) = alg_kids.first() else {
            return false;
        };
        let id = id.value(der);
        let (hash, pss): (DigestAlg, Option<PssParams>) = if id == oids::RSA_PSS.as_bytes() {
            let Some(p) = pss_params(der, alg_kids.get(1)) else {
                return false;
            };
            (p.hash, Some(p))
        } else if let Some(h) = [
            (oids::SHA1_RSA, DigestAlg::Sha1),
            (oids::SHA256_RSA, DigestAlg::Sha256),
            (oids::SHA384_RSA, DigestAlg::Sha384),
            (oids::SHA512_RSA, DigestAlg::Sha512),
            (oids::ECDSA_SHA1, DigestAlg::Sha1),
            (oids::ECDSA_SHA256, DigestAlg::Sha256),
            (oids::ECDSA_SHA384, DigestAlg::Sha384),
            (oids::ECDSA_SHA512, DigestAlg::Sha512),
        ]
        .iter()
        .find(|(oid, _)| oid.as_bytes() == id)
        .map(|(_, h)| *h)
        {
            (h, None)
        } else {
            return false;
        };
        let is_ec_alg = id.starts_with(&oids::ECDSA_SHA256.as_bytes()[..5]);
        if is_ec_alg != (issuer.key_type() == "ec") {
            return false;
        }
        let Some(sig) = parse(der, self.signature.start) else {
            return false;
        };
        let bits = sig.value(der);
        if bits.first() != Some(&0) {
            return false;
        }
        issuer.verify_hash(hash, &hash.hash(self.tbs_bytes()), &bits[1..], pss)
    }
}

/// Certificates from PEM text (`BEGIN CERTIFICATE` blocks); blocks that do
/// not decode or parse are skipped.
pub fn certs_from_pem(text: &[u8]) -> Vec<Vec<u8>> {
    crate::pem::blocks(text)
        .into_iter()
        .filter(|b| b.label == "CERTIFICATE")
        .filter_map(|b| b.der.map(|d| d.to_vec()))
        .filter(|der| Cert::parse(der).is_some())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings() {
        let bmp = asn1::tlv(0x1e, &[0, b'J', 0, b'o']);
        assert_eq!(directory_string(&bmp, &parse(&bmp, 0).unwrap()), "Jo");
        let t61 = asn1::tlv(0x14, &[0xe9]);
        assert_eq!(directory_string(&t61, &parse(&t61, 0).unwrap()), "é");
        assert!(Cert::parse(&[0x30, 0x00]).is_none());
    }
}
