// SPDX-License-Identifier: MIT
//! A signing identity: the private key and its certificate chain, and the
//! checks made before it signs.

use crate::cert::Cert;
use crate::keys::{KeyError, SigningKey};
use crate::time;

/// Why a certificate file cannot sign. `reason()` is the token the app's
/// message catalogue maps to words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignError {
    /// The password is wrong (or the key does not decrypt).
    BadPassword,
    /// The key is not RSA of 2048 bits or more, P-256 or P-384.
    UnsupportedKey,
    /// The file is not a certificate file this app can read.
    Unreadable,
    /// The leaf certificate is not valid at the signing time.
    CertificateExpired {
        /// `YYYY-MM-DD` of the end (or start) of the validity period.
        date: String,
        /// Whether it has expired (else it is not valid yet).
        expired: bool,
    },
    /// The CMS does not fit the space the PDF reserved for it.
    CmsTooLarge,
}

impl SignError {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::BadPassword => "bad-password",
            Self::UnsupportedKey => "unsupported-key",
            Self::Unreadable => "unreadable",
            Self::CertificateExpired { .. } => "certificate-expired",
            Self::CmsTooLarge => "cms-too-large",
        }
    }
    /// The sentence the app has shown for an out-of-date certificate.
    pub fn status(&self) -> Option<String> {
        match self {
            Self::CertificateExpired {
                date,
                expired: true,
            } => Some(format!("this certificate expired on {date}")),
            Self::CertificateExpired {
                date,
                expired: false,
            } => Some(format!("this certificate is not valid until {date}")),
            _ => None,
        }
    }
}

/// A private key with its certificates, leaf first.
#[derive(Debug)]
pub struct Identity {
    pub key: SigningKey,
    pub certs: Vec<Vec<u8>>,
}

/// What a container held, before the leaf is matched to the key.
#[derive(Default)]
pub(crate) struct Found {
    pub key: Option<Result<SigningKey, KeyError>>,
    pub certs: Vec<Vec<u8>>,
}

impl Found {
    /// The key must be supported and one certificate must carry its public
    /// key; that certificate moves to the front.
    pub fn finish(self) -> Result<Identity, SignError> {
        let key = match self.key {
            None => return Err(SignError::Unreadable),
            Some(Err(_)) => return Err(SignError::UnsupportedKey),
            Some(Ok(k)) => k,
        };
        let public = key.public_key();
        let leaf = self
            .certs
            .iter()
            .position(|der| {
                Cert::parse(der)
                    .and_then(|c| c.public_key())
                    .is_some_and(|p| p.same_as(&public))
            })
            .ok_or(SignError::Unreadable)?;
        let mut certs = self.certs;
        let first = certs.remove(leaf);
        certs.insert(0, first);
        Ok(Identity { key, certs })
    }
}

/// The facts the sign dialog shows about a usable certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Usable {
    pub cn: String,
    pub not_after_ms: i64,
}

/// Whether the leaf (`certs[0]`) can sign at `when` (ms): it must parse and
/// `notBefore <= when < notAfter`.
pub fn check_usable(certs: &[Vec<u8>], _key: &SigningKey, when: i64) -> Result<Usable, SignError> {
    let leaf = certs
        .first()
        .and_then(|d| Cert::parse(d))
        .ok_or(SignError::Unreadable)?;
    if when < leaf.not_before_ms || when >= leaf.not_after_ms {
        let expired = when >= leaf.not_after_ms;
        let date = time::ymd(if expired {
            leaf.not_after_ms
        } else {
            leaf.not_before_ms
        });
        return Err(SignError::CertificateExpired { date, expired });
    }
    Ok(Usable {
        cn: leaf.subject_cn(),
        not_after_ms: leaf.not_after_ms,
    })
}

impl Identity {
    /// A PKCS#12 file.
    pub fn from_p12(file: &[u8], password: &str) -> Result<Self, SignError> {
        crate::pkcs12::parse_p12(file, password)
    }
    /// PEM text holding a key and certificates.
    pub fn from_pem(text: &[u8], password: &str) -> Result<Self, SignError> {
        crate::pem::parse_pem(text, password)
    }
    /// See [`check_usable`].
    pub fn check_usable(&self, when: i64) -> Result<Usable, SignError> {
        check_usable(&self.certs, &self.key, when)
    }
}
