// SPDX-License-Identifier: MIT
//! PKCS#12 (`.p12`/`.pfx`) reading: the MAC check, PBES2 (PBKDF2 with AES or
//! 3DES) and the legacy PKCS#12 PBE ciphers (3DES, RC2-40 and RC2-128) that
//! `openssl pkcs12 -legacy` and older Windows exports use.

use crate::asn1::{Tlv, children, is_oid, parse, small_uint};
use crate::digest::DigestAlg;
use crate::identity::{Found, Identity, SignError};
use crate::keys::{KeyError, SigningKey};
use crate::oids;
use cbc::cipher::{BlockDecryptMut, InnerIvInit, KeyIvInit, block_padding::Pkcs7};
use der::Decode;
use hmac::{Hmac, Mac};
use pkcs12::kdf::{Pkcs12KeyType, derive_key};
use zeroize::Zeroizing;

/// The most KDF iterations a file may ask for. OpenSSL writes 2048; current
/// guidance tops out near 600,000. More is refused as unreadable rather than
/// letting a hostile file keep the main process busy.
pub const MAX_ITERATIONS: u64 = 5_000_000;
/// Nesting deeper than this is refused.
const MAX_DEPTH: usize = 16;

/// The password as BMPString bytes with the two-byte terminator (RFC 7292 B.1):
/// UTF-16 code units, big-endian, so a non-BMP character is a surrogate pair.
fn bmp_password(password: &str) -> Zeroizing<Vec<u8>> {
    let mut out = Zeroizing::new(Vec::with_capacity(password.len() * 2 + 2));
    for unit in password.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out.extend_from_slice(&[0, 0]);
    out
}

fn kdf(
    alg: DigestAlg,
    password: &str,
    salt: &[u8],
    rounds: u64,
    id: Pkcs12KeyType,
    len: usize,
) -> Option<Zeroizing<Vec<u8>>> {
    if rounds == 0 || rounds > MAX_ITERATIONS {
        return None;
    }
    let rounds = i32::try_from(rounds).ok()?;
    let pass = bmp_password(password);
    Some(Zeroizing::new(match alg {
        DigestAlg::Sha1 => derive_key::<sha1::Sha1>(&pass, salt, id, rounds, len),
        DigestAlg::Sha256 => derive_key::<sha2::Sha256>(&pass, salt, id, rounds, len),
        DigestAlg::Sha384 => derive_key::<sha2::Sha384>(&pass, salt, id, rounds, len),
        DigestAlg::Sha512 => derive_key::<sha2::Sha512>(&pass, salt, id, rounds, len),
    }))
}

/// A derived key and IV.
type KeyIv = (Zeroizing<Vec<u8>>, Zeroizing<Vec<u8>>);

/// Why decryption failed: a wrong password (bad padding) or a form this
/// crate does not read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecryptError {
    BadPassword,
    Unreadable,
}

fn pbes2(
    params_der: &[u8],
    ciphertext: &[u8],
    password: &str,
) -> Result<Zeroizing<Vec<u8>>, DecryptError> {
    let params =
        pkcs5::pbes2::Parameters::from_der(params_der).map_err(|_| DecryptError::Unreadable)?;
    match &params.kdf {
        pkcs5::pbes2::Kdf::Pbkdf2(p) if u64::from(p.iteration_count) <= MAX_ITERATIONS => {}
        _ => return Err(DecryptError::Unreadable),
    }
    let mut buffer = Zeroizing::new(ciphertext.to_vec());
    let len = params
        .decrypt_in_place(password.as_bytes(), &mut buffer)
        .map_err(|e| match e {
            pkcs5::Error::DecryptFailed => DecryptError::BadPassword,
            _ => DecryptError::Unreadable,
        })?
        .len();
    buffer.truncate(len);
    Ok(buffer)
}

fn legacy_pbe(
    bytes: &[u8],
    alg: &[Tlv],
    ciphertext: &[u8],
    password: &str,
) -> Result<Zeroizing<Vec<u8>>, DecryptError> {
    let id = alg.first().ok_or(DecryptError::Unreadable)?;
    let params = children(bytes, alg.get(1).ok_or(DecryptError::Unreadable)?)
        .ok_or(DecryptError::Unreadable)?;
    let salt = params.first().ok_or(DecryptError::Unreadable)?.value(bytes);
    let rounds = params
        .get(1)
        .and_then(|n| small_uint(bytes, n))
        .ok_or(DecryptError::Unreadable)?;
    let key_iv = |key_len: usize| -> Result<KeyIv, DecryptError> {
        let key = kdf(
            DigestAlg::Sha1,
            password,
            salt,
            rounds,
            Pkcs12KeyType::EncryptionKey,
            key_len,
        )
        .ok_or(DecryptError::Unreadable)?;
        let iv = kdf(
            DigestAlg::Sha1,
            password,
            salt,
            rounds,
            Pkcs12KeyType::Iv,
            8,
        )
        .ok_or(DecryptError::Unreadable)?;
        Ok((key, iv))
    };
    let plain = if is_oid(bytes, id, &oids::PBE_SHA_3DES) {
        let (key, iv) = key_iv(24)?;
        cbc::Decryptor::<des::TdesEde3>::new_from_slices(&key, &iv)
            .map_err(|_| DecryptError::Unreadable)?
            .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
    } else if is_oid(bytes, id, &oids::PBE_SHA_RC2_40) || is_oid(bytes, id, &oids::PBE_SHA_RC2_128)
    {
        let bits = if is_oid(bytes, id, &oids::PBE_SHA_RC2_40) {
            40
        } else {
            128
        };
        let (key, iv) = key_iv(bits / 8)?;
        let cipher = rc2::Rc2::new_with_eff_key_len(&key, bits);
        cbc::Decryptor::<rc2::Rc2>::inner_iv_slice_init(cipher, &iv)
            .map_err(|_| DecryptError::Unreadable)?
            .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
    } else {
        return Err(DecryptError::Unreadable);
    };
    plain
        .map(Zeroizing::new)
        .map_err(|_| DecryptError::BadPassword)
}

/// Decrypts under an AlgorithmIdentifier node (PBES2 or a PKCS#12 PBE).
fn decrypt_alg(
    bytes: &[u8],
    alg: &Tlv,
    ciphertext: &[u8],
    password: &str,
) -> Result<Zeroizing<Vec<u8>>, DecryptError> {
    let kids = children(bytes, alg).ok_or(DecryptError::Unreadable)?;
    let id = kids.first().ok_or(DecryptError::Unreadable)?;
    if is_oid(bytes, id, &oids::PBES2) {
        let params = kids.get(1).ok_or(DecryptError::Unreadable)?;
        pbes2(params.raw(bytes), ciphertext, password)
    } else {
        legacy_pbe(bytes, &kids, ciphertext, password)
    }
}

/// The plaintext PKCS#8 of an EncryptedPrivateKeyInfo (PEM `ENCRYPTED
/// PRIVATE KEY`, or a PKCS#12 shrouded key bag).
pub fn decrypt_encrypted_private_key_info(
    der: &[u8],
    password: &str,
) -> Result<Zeroizing<Vec<u8>>, DecryptError> {
    let top = parse(der, 0).ok_or(DecryptError::Unreadable)?;
    let parts = children(der, &top).ok_or(DecryptError::Unreadable)?;
    let (alg, data) = (parts.first(), parts.get(1));
    let (Some(alg), Some(data)) = (alg, data) else {
        return Err(DecryptError::Unreadable);
    };
    decrypt_alg(der, alg, data.value(der), password)
}

fn key_from(
    result: Result<SigningKey, KeyError>,
) -> Result<Result<SigningKey, KeyError>, SignError> {
    match result {
        Ok(k) => Ok(Ok(k)),
        Err(KeyError::Unsupported) => Ok(Err(KeyError::Unsupported)),
        // As before the port: a key that does not import is unreadable.
        Err(KeyError::Malformed) => Err(SignError::Unreadable),
    }
}

fn decrypt_err(e: DecryptError) -> SignError {
    match e {
        DecryptError::BadPassword => SignError::BadPassword,
        DecryptError::Unreadable => SignError::Unreadable,
    }
}

/// Walks ContentInfo, SafeContents and SafeBag nodes, collecting the key and
/// the certificates.
fn bags(bytes: &[u8], password: &str, found: &mut Found, depth: usize) -> Result<(), SignError> {
    if depth > MAX_DEPTH {
        return Err(SignError::Unreadable);
    }
    let Some(top) = parse(bytes, 0) else {
        return Ok(());
    };
    if top.tag != 0x30 {
        return Ok(());
    }
    let parts = children(bytes, &top).ok_or(SignError::Unreadable)?;
    if let Some(first) = parts.first().filter(|p| p.tag == 0x06) {
        let explicit = parts.iter().find(|p| p.tag == 0xa0);
        if let Some(explicit) = explicit {
            let inner = || -> Result<Tlv, SignError> {
                children(bytes, explicit)
                    .and_then(|k| k.first().copied())
                    .ok_or(SignError::Unreadable)
            };
            if is_oid(bytes, first, &oids::DATA) {
                let oct = inner()?;
                return bags(oct.value(bytes), password, found, depth + 1);
            }
            if is_oid(bytes, first, &oids::ENCRYPTED_DATA) {
                let encrypted = inner()?;
                let info = children(bytes, &encrypted)
                    .and_then(|k| k.get(1).copied())
                    .ok_or(SignError::Unreadable)?;
                let info_parts = children(bytes, &info).ok_or(SignError::Unreadable)?;
                let (Some(alg), Some(content)) = (info_parts.get(1), info_parts.get(2)) else {
                    return Err(SignError::Unreadable);
                };
                let plain =
                    decrypt_alg(bytes, alg, content.value(bytes), password).map_err(decrypt_err)?;
                return bags(&plain, password, found, depth + 1);
            }
            if is_oid(bytes, first, &oids::SHROUDED_KEY_BAG) {
                let epki = inner()?;
                let plain = decrypt_encrypted_private_key_info(epki.raw(bytes), password)
                    .map_err(decrypt_err)?;
                found.key = Some(key_from(SigningKey::from_pkcs8_der(&plain))?);
                return Ok(());
            }
            if is_oid(bytes, first, &oids::KEY_BAG) {
                let info = inner()?;
                found.key = Some(key_from(SigningKey::from_pkcs8_der(info.raw(bytes)))?);
                return Ok(());
            }
            if is_oid(bytes, first, &oids::CERT_BAG) {
                let bag = inner()?;
                let bag = children(bytes, &bag).ok_or(SignError::Unreadable)?;
                if bag
                    .first()
                    .is_some_and(|id| is_oid(bytes, id, &oids::X509_CERT))
                {
                    let oct = bag
                        .get(1)
                        .and_then(|w| children(bytes, w))
                        .and_then(|k| k.first().copied())
                        .ok_or(SignError::Unreadable)?;
                    found.certs.push(oct.value(bytes).to_vec());
                }
                return Ok(());
            }
        }
    }
    for part in parts.iter().filter(|p| p.tag == 0x30) {
        bags(part.raw(bytes), password, found, depth + 1)?;
    }
    Ok(())
}

fn check_mac(bytes: &[u8], mac_data: &Tlv, safe: &[u8], password: &str) -> Result<(), SignError> {
    let mac = children(bytes, mac_data).ok_or(SignError::Unreadable)?;
    let digest_info = mac
        .first()
        .and_then(|d| children(bytes, d))
        .ok_or(SignError::Unreadable)?;
    let alg_id = digest_info
        .first()
        .and_then(|a| children(bytes, a))
        .and_then(|k| k.first().copied())
        .ok_or(SignError::Unreadable)?;
    let alg = DigestAlg::from_oid_bytes(alg_id.value(bytes)).ok_or(SignError::Unreadable)?;
    let expected = digest_info
        .get(1)
        .ok_or(SignError::Unreadable)?
        .value(bytes);
    let salt = mac.get(1).ok_or(SignError::Unreadable)?.value(bytes);
    let rounds = match mac.get(2) {
        Some(n) => small_uint(bytes, n).ok_or(SignError::Unreadable)?,
        None => 1,
    };
    let len = alg.hash(&[]).len();
    let key =
        kdf(alg, password, salt, rounds, Pkcs12KeyType::Mac, len).ok_or(SignError::Unreadable)?;
    let ok = match alg {
        DigestAlg::Sha1 => verify_hmac::<Hmac<sha1::Sha1>>(&key, safe, expected),
        DigestAlg::Sha256 => verify_hmac::<Hmac<sha2::Sha256>>(&key, safe, expected),
        DigestAlg::Sha384 => verify_hmac::<Hmac<sha2::Sha384>>(&key, safe, expected),
        DigestAlg::Sha512 => verify_hmac::<Hmac<sha2::Sha512>>(&key, safe, expected),
    };
    if ok {
        Ok(())
    } else {
        Err(SignError::BadPassword)
    }
}

fn verify_hmac<M: Mac + hmac::digest::KeyInit>(key: &[u8], data: &[u8], expected: &[u8]) -> bool {
    let Ok(mut mac) = <M as hmac::digest::KeyInit>::new_from_slice(key) else {
        return false;
    };
    mac.update(data);
    mac.verify_slice(expected).is_ok()
}

/// Reads a PKCS#12 file: checks the MAC (a mismatch is `bad-password`),
/// decrypts the bags, and returns the key with its certificates, leaf first.
pub fn parse_p12(file: &[u8], password: &str) -> Result<Identity, SignError> {
    let top = parse(file, 0).ok_or(SignError::Unreadable)?;
    let pfx = children(file, &top).ok_or(SignError::Unreadable)?;
    let auth = pfx
        .get(1)
        .and_then(|a| children(file, a))
        .ok_or(SignError::Unreadable)?;
    let oct = auth
        .get(1)
        .and_then(|w| children(file, w))
        .and_then(|k| k.first().copied())
        .ok_or(SignError::Unreadable)?;
    let safe = oct.value(file);
    if let Some(mac_data) = pfx.get(2) {
        check_mac(file, mac_data, safe, password)?;
    }
    let mut found = Found::default();
    bags(safe, password, &mut found, 0)?;
    found.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bmp_password_is_utf16_with_terminator() {
        assert_eq!(bmp_password("ab").as_slice(), &[0, b'a', 0, b'b', 0, 0]);
        assert_eq!(bmp_password("").as_slice(), &[0, 0]);
        assert_eq!(bmp_password("\u{1F600}").len(), 6);
    }

    #[test]
    fn garbage_is_unreadable_and_huge_iterations_refused() {
        assert!(matches!(
            parse_p12(&[0x30, 0x00], "x"),
            Err(SignError::Unreadable)
        ));
        assert!(matches!(
            parse_p12(b"not der", "x"),
            Err(SignError::Unreadable)
        ));
        assert!(
            kdf(
                DigestAlg::Sha1,
                "x",
                b"salt",
                MAX_ITERATIONS + 1,
                Pkcs12KeyType::Mac,
                20
            )
            .is_none()
        );
    }
}
