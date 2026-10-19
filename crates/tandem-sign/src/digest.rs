// SPDX-License-Identifier: MIT
//! The hash algorithms signatures name, with one-shot and incremental use.

use crate::oids;
use const_oid::ObjectIdentifier;
use sha2::Digest;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigestAlg {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl DigestAlg {
    /// The algorithm an AlgorithmIdentifier OID names, if supported.
    pub fn from_oid_bytes(bytes: &[u8]) -> Option<Self> {
        [Self::Sha1, Self::Sha256, Self::Sha384, Self::Sha512]
            .into_iter()
            .find(|alg| alg.oid().as_bytes() == bytes)
    }
    pub fn oid(self) -> ObjectIdentifier {
        match self {
            Self::Sha1 => oids::SHA1,
            Self::Sha256 => oids::SHA256,
            Self::Sha384 => oids::SHA384,
            Self::Sha512 => oids::SHA512,
        }
    }
    /// The short name the verification report uses (`sha1`, `sha256`, …).
    pub fn name(self) -> &'static str {
        match self {
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
            Self::Sha384 => "sha384",
            Self::Sha512 => "sha512",
        }
    }
    pub fn hash(self, data: &[u8]) -> Vec<u8> {
        let mut h = Hasher::new(self);
        h.update(data);
        h.finish()
    }
}

/// An incremental hash of any supported algorithm.
pub enum Hasher {
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha384(sha2::Sha384),
    Sha512(sha2::Sha512),
}

impl Hasher {
    pub fn new(alg: DigestAlg) -> Self {
        match alg {
            DigestAlg::Sha1 => Self::Sha1(sha1::Sha1::new()),
            DigestAlg::Sha256 => Self::Sha256(sha2::Sha256::new()),
            DigestAlg::Sha384 => Self::Sha384(sha2::Sha384::new()),
            DigestAlg::Sha512 => Self::Sha512(sha2::Sha512::new()),
        }
    }
    pub fn update(&mut self, data: &[u8]) {
        match self {
            Self::Sha1(h) => h.update(data),
            Self::Sha256(h) => h.update(data),
            Self::Sha384(h) => h.update(data),
            Self::Sha512(h) => h.update(data),
        }
    }
    pub fn finish(self) -> Vec<u8> {
        match self {
            Self::Sha1(h) => h.finalize().to_vec(),
            Self::Sha256(h) => h.finalize().to_vec(),
            Self::Sha384(h) => h.finalize().to_vec(),
            Self::Sha512(h) => h.finalize().to_vec(),
        }
    }
}

/// `AB:CD:…`, the SHA-256 fingerprint form OpenSSL and Node print.
pub fn fingerprint256(der: &[u8]) -> String {
    DigestAlg::Sha256
        .hash(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}
