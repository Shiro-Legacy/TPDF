// SPDX-License-Identifier: AGPL-3.0-or-later
//! Standard security handler for reading: R2-R4 (RC4/AES-128) and R5/R6 (AES-256).
//! Hashes and block ciphers come from the RustCrypto md-5, sha2 and aes crates;
//! RC4 and the CBC chaining around AES are the textbook algorithms, tested below.
//! Public-key handlers and named non-Identity crypt filters are refused.

use std::collections::BTreeMap;

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit, generic_array::GenericArray};
use aes::{Aes128, Aes256};
use md5::{Digest, Md5};
use sha2::{Sha256, Sha384, Sha512};

use crate::object::{Object, ObjectId};

const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Identity,
    Rc4,
    AesV2,
    AesV3,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SecurityError {
    /// Neither the supplied password nor the empty password opens the document.
    Password,
    Unsupported(&'static str),
    Invalid(&'static str),
}

/// Authenticated document key and the facts the application reports.
#[derive(Clone, Debug)]
pub struct Security {
    key: Vec<u8>,
    strings: Method,
    streams: Method,
    encrypt_metadata: bool,
    pub revision: i64,
    /// The raw /P bits as a signed 32-bit value.
    pub permissions: i32,
    /// True when the owner password authenticated (all permissions apply).
    pub owner: bool,
}

fn name<'a>(dict: &'a BTreeMap<Vec<u8>, Object>, key: &[u8]) -> Option<&'a [u8]> {
    match dict.get(key) {
        Some(Object::Name(n)) => Some(n),
        _ => None,
    }
}
fn int(dict: &BTreeMap<Vec<u8>, Object>, key: &[u8]) -> Option<i64> {
    match dict.get(key) {
        Some(Object::Integer(v)) => Some(*v),
        _ => None,
    }
}
fn string<'a>(dict: &'a BTreeMap<Vec<u8>, Object>, key: &[u8]) -> Result<&'a [u8], SecurityError> {
    match dict.get(key) {
        Some(Object::String(s)) => Ok(s),
        _ => Err(SecurityError::Invalid("encryption dictionary string entry")),
    }
}

fn crypt_filter(
    dict: &BTreeMap<Vec<u8>, Object>,
    key: &[u8],
    version: i64,
) -> Result<Method, SecurityError> {
    if version < 4 {
        return Ok(Method::Rc4);
    }
    let selected = name(dict, key).unwrap_or(b"Identity");
    if selected == b"Identity" {
        return Ok(Method::Identity);
    }
    let Some(Object::Dictionary(filters)) = dict.get(b"CF".as_slice()) else {
        return Err(SecurityError::Invalid("encryption /CF dictionary"));
    };
    let Some(Object::Dictionary(filter)) = filters.get(selected) else {
        return Err(SecurityError::Invalid("named crypt filter"));
    };
    match name(filter, b"CFM").unwrap_or(b"None") {
        b"V2" => Ok(Method::Rc4),
        b"AESV2" => Ok(Method::AesV2),
        b"AESV3" => Ok(Method::AesV3),
        _ => Err(SecurityError::Unsupported("crypt filter method")),
    }
}

impl Security {
    /// Authenticate `password` as owner, then user; an empty password is the
    /// common no-password case.
    pub fn authenticate(
        dict: &BTreeMap<Vec<u8>, Object>,
        first_id: &[u8],
        password: &[u8],
    ) -> Result<Self, SecurityError> {
        if name(dict, b"Filter") != Some(b"Standard") {
            return Err(SecurityError::Unsupported("non-Standard security handler"));
        }
        let version = int(dict, b"V").unwrap_or(0);
        let revision = int(dict, b"R").ok_or(SecurityError::Invalid("encryption /R"))?;
        if !matches!(version, 1 | 2 | 4 | 5) || !(2..=6).contains(&revision) {
            return Err(SecurityError::Unsupported("encryption version or revision"));
        }
        let permissions = int(dict, b"P").ok_or(SecurityError::Invalid("encryption /P"))? as i32;
        let strings = crypt_filter(dict, b"StrF", version)?;
        let streams = crypt_filter(dict, b"StmF", version)?;
        let encrypt_metadata = !matches!(
            dict.get(b"EncryptMetadata".as_slice()),
            Some(Object::Boolean(false))
        );
        let o = string(dict, b"O")?;
        let u = string(dict, b"U")?;
        let mut security = Self {
            key: Vec::new(),
            strings,
            streams,
            encrypt_metadata,
            revision,
            permissions,
            owner: false,
        };
        if revision >= 5 {
            if o.len() < 48 || u.len() < 48 {
                return Err(SecurityError::Invalid("encryption /O or /U length"));
            }
            let password = &password[..password.len().min(127)];
            let hash = |salt: &[u8], extra: &[u8]| hash_r56(revision, password, salt, extra);
            let (intermediate, wrapped, owner) = if hash(&o[32..40], &u[..48]) == o[..32] {
                (hash(&o[40..48], &u[..48]), string(dict, b"OE")?, true)
            } else if hash(&u[32..40], &[]) == u[..32] {
                (hash(&u[40..48], &[]), string(dict, b"UE")?, false)
            } else {
                return Err(SecurityError::Password);
            };
            if wrapped.len() < 32 {
                return Err(SecurityError::Invalid("encryption /OE or /UE length"));
            }
            security.key = aes256_cbc_raw(&intermediate, &[0; 16], &wrapped[..32]);
            security.owner = owner;
            return Ok(security);
        }
        let length = match version {
            1 => 40,
            _ => int(dict, b"Length").unwrap_or(if version == 4 { 128 } else { 40 }),
        };
        if length % 8 != 0 || !(40..=128).contains(&length) || revision == 2 && length != 40 {
            return Err(SecurityError::Invalid("encryption key /Length"));
        }
        let n = (length / 8) as usize;
        if o.len() < 32 || u.len() < 32 {
            return Err(SecurityError::Invalid("encryption /O or /U length"));
        }
        let user_key = |password: &[u8]| {
            let mut md5 = Md5::new();
            md5.update(padded(password));
            md5.update(&o[..32]);
            md5.update((permissions as u32).to_le_bytes());
            md5.update(first_id);
            if revision >= 4 && !encrypt_metadata {
                md5.update([0xFF; 4]);
            }
            let mut key = md5.finalize().to_vec();
            if revision >= 3 {
                for _ in 0..50 {
                    key = Md5::digest(&key[..n]).to_vec();
                }
            }
            key.truncate(n);
            key
        };
        let check_user = |key: &[u8]| {
            if revision == 2 {
                rc4(key, &PAD) == u[..32]
            } else {
                let mut md5 = Md5::new();
                md5.update(PAD);
                md5.update(first_id);
                let mut value = md5.finalize().to_vec();
                for i in 0..20u8 {
                    let round: Vec<u8> = key.iter().map(|b| b ^ i).collect();
                    value = rc4(&round, &value);
                }
                value[..16] == u[..16]
            }
        };
        // Algorithm 7: the owner password recovers the user password from /O.
        let mut owner_key = Md5::digest(padded(password)).to_vec();
        if revision >= 3 {
            for _ in 0..50 {
                owner_key = Md5::digest(&owner_key).to_vec();
            }
        }
        owner_key.truncate(n);
        let mut recovered = o[..32].to_vec();
        if revision == 2 {
            recovered = rc4(&owner_key, &recovered);
        } else {
            for i in (0..20u8).rev() {
                let round: Vec<u8> = owner_key.iter().map(|b| b ^ i).collect();
                recovered = rc4(&round, &recovered);
            }
        }
        let key = user_key(&recovered);
        if check_user(&key) {
            security.key = key;
            security.owner = true;
            return Ok(security);
        }
        let key = user_key(password);
        if check_user(&key) {
            security.key = key;
            return Ok(security);
        }
        Err(SecurityError::Password)
    }

    pub fn string_method(&self) -> Method {
        self.strings
    }
    /// Metadata streams stay in plaintext when /EncryptMetadata is false.
    pub fn stream_method(&self, metadata: bool) -> Method {
        if metadata && !self.encrypt_metadata {
            Method::Identity
        } else {
            self.streams
        }
    }

    pub fn decrypt(&self, method: Method, id: ObjectId, data: &[u8]) -> Vec<u8> {
        let object_key = || {
            let mut md5 = Md5::new();
            md5.update(&self.key);
            md5.update(&id.number().to_le_bytes()[..3]);
            md5.update(id.generation().to_le_bytes());
            if method == Method::AesV2 {
                md5.update(b"sAlT");
            }
            let mut key = md5.finalize().to_vec();
            key.truncate((self.key.len() + 5).min(16));
            key
        };
        match method {
            Method::Identity => data.to_vec(),
            Method::Rc4 => rc4(&object_key(), data),
            Method::AesV2 => aes_cbc_padded(&object_key(), data),
            Method::AesV3 => aes_cbc_padded(&self.key, data),
        }
    }
}

fn padded(password: &[u8]) -> [u8; 32] {
    let mut out = PAD;
    let len = password.len().min(32);
    out[..len].copy_from_slice(&password[..len]);
    out[len..].copy_from_slice(&PAD[..32 - len]);
    out
}

fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|byte| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            byte ^ s[s[i as usize].wrapping_add(s[j as usize]) as usize]
        })
        .collect()
}

/// IV-prefixed CBC with PKCS#7 padding. Like pdf.js and Acrobat, a trailing
/// partial block is dropped and malformed padding leaves the data unpadded.
fn aes_cbc_padded(key: &[u8], data: &[u8]) -> Vec<u8> {
    if data.len() < 32 {
        return Vec::new();
    }
    let body = &data[16..16 + (data.len() - 16) / 16 * 16];
    let mut out = if key.len() == 32 {
        aes256_cbc_raw(key, &data[..16], body)
    } else {
        cbc_decrypt(
            &Aes128::new(GenericArray::from_slice(key)),
            &data[..16],
            body,
        )
    };
    let pad = usize::from(*out.last().expect("nonempty"));
    if (1..=16).contains(&pad)
        && out[out.len() - pad..]
            .iter()
            .all(|&b| usize::from(b) == pad)
    {
        out.truncate(out.len() - pad);
    }
    out
}

fn aes256_cbc_raw(key: &[u8], iv: &[u8], data: &[u8]) -> Vec<u8> {
    cbc_decrypt(&Aes256::new(GenericArray::from_slice(key)), iv, data)
}

fn cbc_decrypt(cipher: &impl BlockDecrypt, iv: &[u8], data: &[u8]) -> Vec<u8> {
    let mut previous: [u8; 16] = iv.try_into().expect("16-byte IV");
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(16) {
        let mut block = GenericArray::clone_from_slice(chunk);
        cipher.decrypt_block(&mut block);
        out.extend(block.iter().zip(previous).map(|(a, b)| a ^ b));
        previous.copy_from_slice(chunk);
    }
    out
}

/// ISO 32000-2 Algorithm 2.B (R6), or plain SHA-256 for the deprecated R5.
fn hash_r56(revision: i64, password: &[u8], salt: &[u8], extra: &[u8]) -> Vec<u8> {
    let mut k = Sha256::new()
        .chain_update(password)
        .chain_update(salt)
        .chain_update(extra)
        .finalize()
        .to_vec();
    if revision == 5 {
        return k;
    }
    let mut round = 0usize;
    loop {
        let one: Vec<u8> = [password, &k, extra].concat();
        let k1 = one.repeat(64);
        let cipher = Aes128::new(GenericArray::from_slice(&k[..16]));
        let mut previous: [u8; 16] = k[16..32].try_into().expect("32-byte hash");
        let mut e = Vec::with_capacity(k1.len());
        for chunk in k1.chunks_exact(16) {
            let mut block = GenericArray::clone_from_slice(chunk);
            block.iter_mut().zip(previous).for_each(|(a, b)| *a ^= b);
            cipher.encrypt_block(&mut block);
            previous.copy_from_slice(&block);
            e.extend_from_slice(&block);
        }
        let selector = e[..16].iter().map(|&b| u32::from(b)).sum::<u32>() % 3;
        k = match selector {
            0 => Sha256::digest(&e).to_vec(),
            1 => Sha384::digest(&e).to_vec(),
            _ => Sha512::digest(&e).to_vec(),
        };
        round += 1;
        if round >= 64 && usize::from(*e.last().expect("nonempty")) <= round - 32 {
            break;
        }
    }
    k.truncate(32);
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc4_matches_rfc6229_vector() {
        // RFC 6229, 40-bit key 0102030405, keystream offset 0.
        let stream = rc4(&[1, 2, 3, 4, 5], &[0; 16]);
        assert_eq!(
            stream,
            [
                0xb2, 0x39, 0x63, 0x05, 0xf0, 0x3d, 0xc0, 0x27, 0xcc, 0xc3, 0x52, 0x4a, 0x0a, 0x11,
                0x18, 0xa8
            ]
        );
    }

    #[test]
    fn aes128_cbc_matches_sp800_38a_vector() {
        // NIST SP 800-38A F.2.2 CBC-AES128.Decrypt, first block, plus a full
        // PKCS#7 padding block encrypted under the same chain.
        let key = hex("2b7e151628aed2a6abf7158809cf4f3c");
        let iv = hex("000102030405060708090a0b0c0d0e0f");
        let ct = hex("7649abac8119b246cee98e9b12e9197d");
        let plain = cbc_decrypt(&Aes128::new(GenericArray::from_slice(&key)), &iv, &ct);
        assert_eq!(plain, hex("6bc1bee22e409f96e93d7e117393172a"));
        let cipher = Aes128::new(GenericArray::from_slice(&key));
        let mut pad = GenericArray::clone_from_slice(&[16u8; 16]);
        pad.iter_mut().zip(&ct).for_each(|(a, b)| *a ^= b);
        cipher.encrypt_block(&mut pad);
        let data = [iv.as_slice(), &ct, &pad].concat();
        assert_eq!(aes_cbc_padded(&key, &data), plain);
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
}
