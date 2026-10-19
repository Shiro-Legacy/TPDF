// SPDX-License-Identifier: MIT
//! Signing keys (RSA, ECDSA P-256 and P-384) and the public keys signatures
//! are checked against (RSA, ECDSA P-256, P-384 and P-521).
//!
//! Private key material lives only inside these types. `rsa::RsaPrivateKey`
//! and the curve crates' `SecretKey` zero their memory when dropped, and every
//! decrypted buffer on the way here is a `Zeroizing` vector.

use crate::digest::DigestAlg;
use crate::oids;
use der::Decode;
use rand_core::CryptoRngCore;
use rsa::traits::PublicKeyParts;
use rsa::{BigUint, Pkcs1v15Sign, Pss, RsaPrivateKey, RsaPublicKey};
use signature::hazmat::{PrehashSigner, PrehashVerifier};

/// ECDSA over a prehash with a DER signature, for one curve crate.
macro_rules! ecdsa_verify {
    ($curve:ident, $key:expr, $field:expr, $hash:expr, $sig:expr) => {{
        match $curve::ecdsa::Signature::from_der($sig) {
            Ok(signature) => $key
                .verify_prehash(&pad_prehash($hash, $field), &signature)
                .is_ok(),
            Err(_) => false,
        }
    }};
}

/// RSA keys below this size are refused for signing (as before the port).
pub const MIN_RSA_BITS: usize = 2048;
/// Larger RSA keys are refused: they are slow and exotic, and the bound keeps
/// a hostile file from asking for huge big-number work.
pub const MAX_RSA_BITS: usize = 16384;

/// Why a key could not be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyError {
    /// The bytes are not a private key of a known form.
    Malformed,
    /// A well-formed key of a type or size this app does not sign with.
    Unsupported,
}

/// A private key this app can sign with.
pub enum SigningKey {
    Rsa(Box<RsaPrivateKey>),
    P256(p256::SecretKey),
    P384(p384::SecretKey),
}

impl std::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print key material.
        write!(f, "SigningKey({} {})", self.kind(), self.bits())
    }
}

fn rsa_from_pkcs1(key: &pkcs1::RsaPrivateKey<'_>) -> Result<SigningKey, KeyError> {
    if key.version() != pkcs1::Version::TwoPrime {
        return Err(KeyError::Unsupported);
    }
    let n = BigUint::from_bytes_be(key.modulus.as_bytes());
    let bits = n.bits();
    if bits > MAX_RSA_BITS {
        return Err(KeyError::Unsupported);
    }
    let e = BigUint::from_bytes_be(key.public_exponent.as_bytes());
    let d = BigUint::from_bytes_be(key.private_exponent.as_bytes());
    let p = BigUint::from_bytes_be(key.prime1.as_bytes());
    let q = BigUint::from_bytes_be(key.prime2.as_bytes());
    let private =
        RsaPrivateKey::from_components(n, e, d, vec![p, q]).map_err(|_| KeyError::Malformed)?;
    if bits < MIN_RSA_BITS {
        return Err(KeyError::Unsupported);
    }
    Ok(SigningKey::Rsa(Box::new(private)))
}

fn ec_from_sec1(
    key: &sec1::EcPrivateKey<'_>,
    curve: Option<der::asn1::ObjectIdentifier>,
) -> Result<SigningKey, KeyError> {
    let curve = curve
        .or_else(|| key.parameters.and_then(|p| p.named_curve()))
        .ok_or(KeyError::Malformed)?;
    if curve == oids::P256 {
        p256::SecretKey::from_slice(key.private_key)
            .map(SigningKey::P256)
            .map_err(|_| KeyError::Malformed)
    } else if curve == oids::P384 {
        p384::SecretKey::from_slice(key.private_key)
            .map(SigningKey::P384)
            .map_err(|_| KeyError::Malformed)
    } else {
        Err(KeyError::Unsupported)
    }
}

impl SigningKey {
    /// A PKCS#8 PrivateKeyInfo (`BEGIN PRIVATE KEY`, or the plaintext of an
    /// encrypted one or of a PKCS#12 key bag).
    pub fn from_pkcs8_der(der: &[u8]) -> Result<Self, KeyError> {
        let info = pkcs8::PrivateKeyInfo::from_der(der).map_err(|_| KeyError::Malformed)?;
        let alg = info.algorithm.oid;
        if alg == oids::RSA {
            let key = pkcs1::RsaPrivateKey::from_der(info.private_key)
                .map_err(|_| KeyError::Malformed)?;
            rsa_from_pkcs1(&key)
        } else if alg == oids::EC_PUBLIC_KEY {
            let curve = info
                .algorithm
                .parameters
                .and_then(|p| p.decode_as::<der::asn1::ObjectIdentifier>().ok());
            let key =
                sec1::EcPrivateKey::from_der(info.private_key).map_err(|_| KeyError::Malformed)?;
            ec_from_sec1(&key, curve)
        } else {
            Err(KeyError::Unsupported)
        }
    }

    /// A PKCS#1 RSAPrivateKey (`BEGIN RSA PRIVATE KEY`).
    pub fn from_pkcs1_der(der: &[u8]) -> Result<Self, KeyError> {
        let key = pkcs1::RsaPrivateKey::from_der(der).map_err(|_| KeyError::Malformed)?;
        rsa_from_pkcs1(&key)
    }

    /// A SEC1 ECPrivateKey (`BEGIN EC PRIVATE KEY`).
    pub fn from_sec1_der(der: &[u8]) -> Result<Self, KeyError> {
        let key = sec1::EcPrivateKey::from_der(der).map_err(|_| KeyError::Malformed)?;
        ec_from_sec1(&key, None)
    }

    /// `rsa` or `ec`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Rsa(_) => "rsa",
            Self::P256(_) | Self::P384(_) => "ec",
        }
    }

    /// The RSA modulus size or the curve size, in bits.
    pub fn bits(&self) -> usize {
        match self {
            Self::Rsa(k) => k.n().bits(),
            Self::P256(_) => 256,
            Self::P384(_) => 384,
        }
    }

    /// The curve name for an EC key.
    pub fn curve(&self) -> Option<&'static str> {
        match self {
            Self::Rsa(_) => None,
            Self::P256(_) => Some("P-256"),
            Self::P384(_) => Some("P-384"),
        }
    }

    /// The digest signatures with this key use: SHA-256, or SHA-384 for P-384.
    pub fn digest(&self) -> DigestAlg {
        match self {
            Self::P384(_) => DigestAlg::Sha384,
            _ => DigestAlg::Sha256,
        }
    }

    /// The SignerInfo signatureAlgorithm, DER: rsaEncryption with NULL for
    /// RSA (as the TypeScript signer wrote it), ecdsa-with-SHA256/384 for EC.
    pub fn signature_algorithm(&self) -> Vec<u8> {
        match self {
            Self::Rsa(_) => crate::asn1::algorithm(&oids::RSA, true),
            Self::P256(_) => crate::asn1::algorithm(&oids::ECDSA_SHA256, false),
            Self::P384(_) => crate::asn1::algorithm(&oids::ECDSA_SHA384, false),
        }
    }

    /// The matching public key.
    pub fn public_key(&self) -> PublicKey {
        match self {
            Self::Rsa(k) => PublicKey::Rsa(Box::new(k.to_public_key())),
            Self::P256(k) => PublicKey::P256(p256::ecdsa::VerifyingKey::from(k.public_key())),
            Self::P384(k) => PublicKey::P384(p384::ecdsa::VerifyingKey::from(k.public_key())),
        }
    }

    /// Signs a hash made with `self.digest()`. RSA is PKCS#1 v1.5 with
    /// blinding from `rng`; ECDSA is deterministic (RFC 6979) and DER-encoded.
    pub fn sign_hash(&self, hash: &[u8], rng: &mut impl CryptoRngCore) -> Option<Vec<u8>> {
        match self {
            Self::Rsa(k) => {
                let scheme = match self.digest() {
                    DigestAlg::Sha384 => Pkcs1v15Sign::new::<sha2::Sha384>(),
                    _ => Pkcs1v15Sign::new::<sha2::Sha256>(),
                };
                k.sign_with_rng(rng, scheme, hash).ok()
            }
            Self::P256(k) => {
                let signer = p256::ecdsa::SigningKey::from(k);
                let sig: p256::ecdsa::Signature = signer.sign_prehash(hash).ok()?;
                Some(sig.to_der().as_bytes().to_vec())
            }
            Self::P384(k) => {
                let signer = p384::ecdsa::SigningKey::from(k);
                let sig: p384::ecdsa::Signature = signer.sign_prehash(hash).ok()?;
                Some(sig.to_der().as_bytes().to_vec())
            }
        }
    }
}

/// RSASSA-PSS parameters (RFC 4055): the hash, the MGF1 hash and the salt length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PssParams {
    pub hash: DigestAlg,
    pub mgf_hash: DigestAlg,
    pub salt_len: usize,
}

/// A public key from a certificate.
pub enum PublicKey {
    Rsa(Box<RsaPublicKey>),
    P256(p256::ecdsa::VerifyingKey),
    P384(p384::ecdsa::VerifyingKey),
    P521(Box<p521::ecdsa::VerifyingKey>),
    /// An EC key on a curve this crate cannot check (the curve's OID).
    OtherCurve(String),
    /// Another key type (its algorithm OID).
    Other(String),
}

impl PublicKey {
    /// A SubjectPublicKeyInfo, `None` when it does not parse.
    pub fn from_spki_der(der: &[u8]) -> Option<Self> {
        let spki = spki::SubjectPublicKeyInfoRef::from_der(der).ok()?;
        let alg = spki.algorithm.oid;
        let bits = spki.subject_public_key.as_bytes()?;
        if alg == oids::RSA {
            let key = pkcs1::RsaPublicKey::from_der(bits).ok()?;
            let n = BigUint::from_bytes_be(key.modulus.as_bytes());
            let e = BigUint::from_bytes_be(key.public_exponent.as_bytes());
            RsaPublicKey::new_with_max_size(n, e, MAX_RSA_BITS)
                .ok()
                .map(|k| Self::Rsa(Box::new(k)))
        } else if alg == oids::EC_PUBLIC_KEY {
            let curve = spki
                .algorithm
                .parameters
                .and_then(|p| p.decode_as::<der::asn1::ObjectIdentifier>().ok())?;
            if curve == oids::P256 {
                p256::ecdsa::VerifyingKey::from_sec1_bytes(bits)
                    .ok()
                    .map(Self::P256)
            } else if curve == oids::P384 {
                p384::ecdsa::VerifyingKey::from_sec1_bytes(bits)
                    .ok()
                    .map(Self::P384)
            } else if curve == oids::P521 {
                p521::ecdsa::VerifyingKey::from_sec1_bytes(bits)
                    .ok()
                    .map(|k| Self::P521(Box::new(k)))
            } else {
                Some(Self::OtherCurve(curve.to_string()))
            }
        } else {
            Some(Self::Other(alg.to_string()))
        }
    }

    /// `rsa`, `ec` (any curve) or the algorithm OID.
    pub fn key_type(&self) -> &str {
        match self {
            Self::Rsa(_) => "rsa",
            Self::P256(_) | Self::P384(_) | Self::P521(_) | Self::OtherCurve(_) => "ec",
            Self::Other(oid) => oid,
        }
    }

    /// The RSA modulus size in bits, `None` for other keys.
    pub fn rsa_bits(&self) -> Option<usize> {
        match self {
            Self::Rsa(k) => Some(k.n().bits()),
            _ => None,
        }
    }

    /// Whether both are the same key.
    pub fn same_as(&self, other: &PublicKey) -> bool {
        match (self, other) {
            (Self::Rsa(a), Self::Rsa(b)) => a.n() == b.n() && a.e() == b.e(),
            (Self::P256(a), Self::P256(b)) => a == b,
            (Self::P384(a), Self::P384(b)) => a == b,
            (Self::P521(a), Self::P521(b)) => {
                a.to_encoded_point(false) == b.to_encoded_point(false)
            }
            _ => false,
        }
    }

    /// Checks `sig` over a hash made with `alg`: PKCS#1 v1.5 or PSS for RSA,
    /// DER ECDSA for EC. False for every malformed or unsupported input.
    pub fn verify_hash(
        &self,
        alg: DigestAlg,
        hash: &[u8],
        sig: &[u8],
        pss: Option<PssParams>,
    ) -> bool {
        match (self, pss) {
            (Self::Rsa(k), None) => {
                let scheme = match alg {
                    DigestAlg::Sha1 => Pkcs1v15Sign::new::<sha1::Sha1>(),
                    DigestAlg::Sha256 => Pkcs1v15Sign::new::<sha2::Sha256>(),
                    DigestAlg::Sha384 => Pkcs1v15Sign::new::<sha2::Sha384>(),
                    DigestAlg::Sha512 => Pkcs1v15Sign::new::<sha2::Sha512>(),
                };
                k.verify(scheme, hash, sig).is_ok()
            }
            (Self::Rsa(k), Some(p)) => {
                // The rsa crate uses one digest for the hash and MGF1.
                if p.hash != p.mgf_hash || p.hash != alg {
                    return false;
                }
                let scheme = match alg {
                    DigestAlg::Sha1 => Pss::new_with_salt::<sha1::Sha1>(p.salt_len),
                    DigestAlg::Sha256 => Pss::new_with_salt::<sha2::Sha256>(p.salt_len),
                    DigestAlg::Sha384 => Pss::new_with_salt::<sha2::Sha384>(p.salt_len),
                    DigestAlg::Sha512 => Pss::new_with_salt::<sha2::Sha512>(p.salt_len),
                };
                k.verify(scheme, hash, sig).is_ok()
            }
            (Self::P256(k), None) => ecdsa_verify!(p256, k, 32, hash, sig),
            (Self::P384(k), None) => ecdsa_verify!(p384, k, 48, hash, sig),
            (Self::P521(k), None) => ecdsa_verify!(p521, k, 66, hash, sig),
            _ => false,
        }
    }
}

/// ECDSA takes the leftmost field-size bytes of a longer hash, and a shorter
/// hash as its integer value: left zero padding keeps that value and meets
/// the curve crates' minimum prehash length.
fn pad_prehash(hash: &[u8], field: usize) -> Vec<u8> {
    if hash.len() >= field {
        return hash.to_vec();
    }
    let mut out = vec![0u8; field - hash.len()];
    out.extend_from_slice(hash);
    out
}

/// RSASSA-PSS-params from an AlgorithmIdentifier's parameter node (RFC 4055
/// defaults: SHA-1, MGF1 with SHA-1, salt 20).
pub fn pss_params(bytes: &[u8], params: Option<&crate::asn1::Tlv>) -> Option<PssParams> {
    use crate::asn1::{children, parse, small_uint};
    let mut out = PssParams {
        hash: DigestAlg::Sha1,
        mgf_hash: DigestAlg::Sha1,
        salt_len: 20,
    };
    let Some(params) = params else {
        return Some(out);
    };
    if params.tag != 0x30 {
        return None;
    }
    let alg_of = |node: &crate::asn1::Tlv| -> Option<(crate::asn1::Tlv, Vec<crate::asn1::Tlv>)> {
        let inner = parse(bytes, node.value_start)?;
        let kids = children(bytes, &inner)?;
        Some((inner, kids))
    };
    for field in children(bytes, params)? {
        match field.tag {
            0xa0 => {
                let (_, kids) = alg_of(&field)?;
                out.hash = DigestAlg::from_oid_bytes(kids.first()?.value(bytes))?;
            }
            0xa1 => {
                let (_, kids) = alg_of(&field)?;
                if kids.first()?.value(bytes) != oids::MGF1.as_bytes() {
                    return None;
                }
                let hash_alg = children(bytes, kids.get(1)?)?;
                out.mgf_hash = DigestAlg::from_oid_bytes(hash_alg.first()?.value(bytes))?;
            }
            0xa2 => {
                let int = parse(bytes, field.value_start)?;
                out.salt_len = usize::try_from(small_uint(bytes, &int)?).ok()?;
                if out.salt_len > 1024 {
                    return None;
                }
            }
            0xa3 => {}
            _ => return None,
        }
    }
    Some(out)
}
