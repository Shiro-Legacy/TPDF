// SPDX-License-Identifier: AGPL-3.0-or-later
//! Standard security handler: reading R2-R4 (RC4/AES-128) and R5/R6
//! (AES-256), re-encrypting with an authenticated handler, and creating a new
//! V5 R6 handler (ISO 32000-2 Algorithms 8, 9 and 10).
//! Hashes and block ciphers come from the RustCrypto md-5, sha2 and aes crates;
//! RC4 and the CBC chaining around AES are the textbook algorithms, tested below.
//! Public-key handlers and named non-Identity crypt filters are refused.
//! Passwords are UTF-8 for the application. R5/R6 use the bytes as given,
//! truncated to 127, without SASLprep, as MuPDF does. R2-R4 hash 32 bytes of
//! PDFDocEncoding: a UTF-8 password is converted first, dropping characters
//! without a code, as MuPDF and Acrobat do; the bytes as given are tried
//! next, for writers that hashed UTF-8 or callers that pass encoded bytes.

use std::fmt;

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit, generic_array::GenericArray};
use aes::{Aes128, Aes256};
use md5::{Digest, Md5};
use sha2::{Sha256, Sha384, Sha512};

use crate::object::{Dictionary, Object, ObjectId, pdf_doc_byte};

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
#[derive(Clone)]
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
    /// The authenticated /Encrypt dictionary and the /ID[0] the key was
    /// derived with, which re-encryption writes back unchanged.
    dictionary: Dictionary,
    first_id: Vec<u8>,
}
/// The file key is never printed.
impl fmt::Debug for Security {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Security")
            .field("strings", &self.strings)
            .field("streams", &self.streams)
            .field("encrypt_metadata", &self.encrypt_metadata)
            .field("revision", &self.revision)
            .field("permissions", &self.permissions)
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

/// Which password opened an encrypted document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Authentication {
    User,
    /// The owner password, or a password that is both.
    Owner,
}

/// What an /Encrypt dictionary declares, read without a password.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncryptionInfo {
    /// /Filter, `Standard` for the standard security handler.
    pub filter: Vec<u8>,
    pub v: i64,
    pub r: i64,
    /// File key length in bits as the handler uses it: 40 for V1, /Length
    /// (default 40) for V2, 128 for AES-128 and /Length (default 128) for
    /// other V4 filters, and 256 for V5.
    pub length: i64,
    /// The stream cipher, or the string cipher when streams use Identity.
    /// `None` when the dictionary names a crypt filter this reader lacks.
    pub method: Option<Method>,
    /// /P as a signed 32-bit value; -4 (everything) when absent.
    pub p: i32,
    pub encrypt_metadata: bool,
}

impl EncryptionInfo {
    pub fn from_dictionary(dict: &Dictionary) -> Self {
        let v = int(dict, b"V").unwrap_or(0);
        let r = int(dict, b"R").unwrap_or(0);
        let stream = crypt_filter(dict, b"StmF", v).ok();
        let method = match stream {
            Some(Method::Identity) => crypt_filter(dict, b"StrF", v).ok(),
            other => other,
        };
        let methods = [stream, crypt_filter(dict, b"StrF", v).ok()];
        let length = match v {
            1 => 40,
            _ if v >= 5 => 256,
            _ if methods.contains(&Some(Method::AesV2)) => 128,
            4 => int(dict, b"Length").unwrap_or(128),
            _ => int(dict, b"Length").unwrap_or(40),
        };
        Self {
            filter: name(dict, b"Filter").unwrap_or_default().to_vec(),
            v,
            r,
            length,
            method,
            p: int(dict, b"P").map_or(-4, |p| p as i32),
            encrypt_metadata: !matches!(
                dict.get(b"EncryptMetadata".as_slice()),
                Some(Object::Boolean(false))
            ),
        }
    }
}

/// A signature dictionary whose /Contents string is neither encrypted nor
/// decrypted (ISO 32000-2 7.6.2): /Type /Sig or /DocTimeStamp (or no /Type)
/// with a /ByteRange array and a /Contents string.
pub fn is_signature_dictionary(dict: &Dictionary) -> bool {
    let typed = match dict.get(b"Type".as_slice()) {
        None => true,
        Some(Object::Name(kind)) => kind == b"Sig" || kind == b"DocTimeStamp",
        Some(_) => false,
    };
    typed
        && matches!(dict.get(b"ByteRange".as_slice()), Some(Object::Array(_)))
        && matches!(dict.get(b"Contents".as_slice()), Some(Object::String(_)))
}

fn name<'a>(dict: &'a Dictionary, key: &[u8]) -> Option<&'a [u8]> {
    match dict.get(key) {
        Some(Object::Name(n)) => Some(n),
        _ => None,
    }
}
fn int(dict: &Dictionary, key: &[u8]) -> Option<i64> {
    match dict.get(key) {
        Some(Object::Integer(v)) => Some(*v),
        _ => None,
    }
}
fn string<'a>(dict: &'a Dictionary, key: &[u8]) -> Result<&'a [u8], SecurityError> {
    match dict.get(key) {
        Some(Object::String(s)) => Ok(s),
        _ => Err(SecurityError::Invalid("encryption dictionary string entry")),
    }
}

fn crypt_filter(dict: &Dictionary, key: &[u8], version: i64) -> Result<Method, SecurityError> {
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
    /// common no-password case. For R2-R4 the PDFDocEncoding of a UTF-8
    /// password is tried before the bytes as given.
    pub fn authenticate(
        dict: &Dictionary,
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
            dictionary: dict.clone(),
            first_id: first_id.to_vec(),
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
        if [strings, streams].contains(&Method::AesV3) {
            return Err(SecurityError::Unsupported("AESV3 below revision 5"));
        }
        // AES-128 keys are 128 bits whatever /Length says, as in MuPDF.
        let length = match version {
            1 => 40,
            _ if [strings, streams].contains(&Method::AesV2) => 128,
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
        // The key and whether `password` is the owner password.
        let try_password = |password: &[u8]| {
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
                return Some((key, true));
            }
            let key = user_key(password);
            check_user(&key).then_some((key, false))
        };
        let encoded = pdf_doc_password(password);
        let (key, owner) = encoded
            .iter()
            .map(Vec::as_slice)
            .chain([password])
            .find_map(try_password)
            .ok_or(SecurityError::Password)?;
        security.key = key;
        security.owner = owner;
        Ok(security)
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
        match method {
            Method::Identity => data.to_vec(),
            Method::Rc4 => rc4(&self.object_key(method, id), data),
            Method::AesV2 => aes_cbc_padded(&self.object_key(method, id), data),
            Method::AesV3 => aes_cbc_padded(&self.key, data),
        }
    }

    /// Encrypt a string or stream of object `id` as `decrypt` reverses it;
    /// AES output is `iv` followed by the PKCS#7-padded CBC ciphertext.
    /// `None` only if the key does not fit the method, which authentication
    /// and `create_r6` rule out.
    pub(crate) fn encrypt(
        &self,
        method: Method,
        id: ObjectId,
        data: &[u8],
        iv: [u8; 16],
    ) -> Option<Vec<u8>> {
        match method {
            Method::Identity => Some(data.to_vec()),
            Method::Rc4 => Some(rc4(&self.object_key(method, id), data)),
            Method::AesV2 => aes_cbc_encrypt(&self.object_key(method, id), iv, data),
            Method::AesV3 => aes_cbc_encrypt(&self.key, iv, data),
        }
    }

    /// Algorithm 1: the file key, the object number's low 3 bytes and the
    /// generation's low 2 bytes (and "sAlT" for AES), hashed with MD5.
    fn object_key(&self, method: Method, id: ObjectId) -> Vec<u8> {
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
    }

    /// The /Encrypt facts of this handler.
    pub fn info(&self) -> EncryptionInfo {
        EncryptionInfo::from_dictionary(&self.dictionary)
    }
    pub fn authentication(&self) -> Authentication {
        if self.owner {
            Authentication::Owner
        } else {
            Authentication::User
        }
    }
    /// The /Encrypt dictionary this handler was authenticated with or created.
    pub fn dictionary(&self) -> &Dictionary {
        &self.dictionary
    }
    /// The /ID[0] the R2-R4 key was derived with (empty without /ID).
    pub fn first_id(&self) -> &[u8] {
        &self.first_id
    }
    #[cfg(test)]
    pub(crate) fn file_key(&self) -> &[u8] {
        &self.key
    }

    /// A new V5 R6 (AES-256) handler, authenticated as owner, with the
    /// dictionary MuPDF writes for `encrypt=aes-256`. `random` is the file
    /// key (32 bytes), the user validation and key salts, the owner
    /// validation and key salts (8 bytes each) and the /Perms filler (4).
    /// An empty owner password becomes the user password (Algorithm 9 uses
    /// the user password when there is no owner password).
    pub(crate) fn create_r6(
        user_password: &[u8],
        owner_password: &[u8],
        permissions: i32,
        random: &[u8; 68],
    ) -> Self {
        let user = &user_password[..user_password.len().min(127)];
        let owner = if owner_password.is_empty() {
            user
        } else {
            &owner_password[..owner_password.len().min(127)]
        };
        let (key, salts) = random.split_at(32);
        let (perms_filler, salts) = (&salts[32..], &salts[..32]);
        // Algorithm 8: /U and /UE.
        let mut u = hash_r56(6, user, &salts[..8], &[]);
        u.extend_from_slice(&salts[..16]);
        let ue = aes256_wrap(&hash_r56(6, user, &salts[8..16], &[]), key);
        // Algorithm 9: /O and /OE, over the 48-byte /U.
        let mut o = hash_r56(6, owner, &salts[16..24], &u);
        o.extend_from_slice(&salts[16..32]);
        let oe = aes256_wrap(&hash_r56(6, owner, &salts[24..32], &u), key);
        // Algorithm 10: /Perms.
        let mut block = [0xFF; 16];
        block[..4].copy_from_slice(&permissions.to_le_bytes());
        block[8..12].copy_from_slice(b"Tadb");
        block[12..].copy_from_slice(perms_filler);
        let perms = aes256_wrap(key, &block);
        let name = |n: &[u8]| Object::Name(n.to_vec());
        let mut filter = Dictionary::new();
        filter.insert(b"AuthEvent".to_vec(), name(b"DocOpen"));
        filter.insert(b"CFM".to_vec(), name(b"AESV3"));
        filter.insert(b"Length".to_vec(), Object::Integer(32));
        let mut filters = Dictionary::new();
        filters.insert(b"StdCF".to_vec(), Object::Dictionary(filter));
        let mut dictionary = Dictionary::new();
        for (key, value) in [
            (b"Filter".as_slice(), name(b"Standard")),
            (b"R", Object::Integer(6)),
            (b"V", Object::Integer(5)),
            (b"Length", Object::Integer(256)),
            (b"P", Object::Integer(i64::from(permissions))),
            (b"EncryptMetadata", Object::Boolean(true)),
            (b"StmF", name(b"StdCF")),
            (b"StrF", name(b"StdCF")),
            (b"CF", Object::Dictionary(filters)),
            (b"O", Object::String(o)),
            (b"U", Object::String(u)),
            (b"OE", Object::String(oe)),
            (b"UE", Object::String(ue)),
            (b"Perms", Object::String(perms)),
        ] {
            dictionary.insert(key.to_vec(), value);
        }
        Self {
            key: key.to_vec(),
            strings: Method::AesV3,
            streams: Method::AesV3,
            encrypt_metadata: true,
            revision: 6,
            permissions,
            owner: true,
            dictionary,
            first_id: Vec::new(),
        }
    }
}

/// /P for MuPDF's `permissions=granted`: the permission bits 3-6 and 9-12
/// of `granted`, the reserved bits 7-8 and 13-32 set, and bit 10 (deprecated
/// accessibility extraction) always set, as ISO 32000-2 Table 22 requires of
/// writers. MuPDF leaves bit 10 to `granted`.
pub fn standard_permissions(granted: u32) -> i32 {
    ((granted & 0xF3C) | 0x200 | 0xFFFF_F0C0) as i32
}

/// A UTF-8 password in PDFDocEncoding, as R2-R4 hash it (MuPDF's
/// `pdf_docenc_from_utf8`: characters without a code are dropped). `None`
/// when it is not UTF-8, is unchanged, or would drop to the empty password.
fn pdf_doc_password(password: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(password).ok()?;
    let encoded: Vec<u8> = text.chars().filter_map(pdf_doc_byte).collect();
    (!encoded.is_empty() && encoded != password).then_some(encoded)
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
    let mut out = match key.len() {
        32 => aes256_cbc_raw(key, &data[..16], body),
        16 => cbc_decrypt(
            &Aes128::new(GenericArray::from_slice(key)),
            &data[..16],
            body,
        ),
        _ => return Vec::new(),
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

/// `iv`, then the CBC encryption of `data` with PKCS#7 padding; `None` for a
/// key that is neither 16 nor 32 bytes.
fn aes_cbc_encrypt(key: &[u8], iv: [u8; 16], data: &[u8]) -> Option<Vec<u8>> {
    let pad = 16 - data.len() % 16;
    let mut padded = Vec::new();
    padded.try_reserve_exact(data.len() + pad).ok()?;
    padded.extend_from_slice(data);
    padded.resize(data.len() + pad, pad as u8);
    let mut out = Vec::new();
    out.try_reserve_exact(16 + padded.len()).ok()?;
    out.extend_from_slice(&iv);
    match key.len() {
        16 => cbc_encrypt(
            &Aes128::new(GenericArray::from_slice(key)),
            iv,
            &padded,
            &mut out,
        ),
        32 => cbc_encrypt(
            &Aes256::new(GenericArray::from_slice(key)),
            iv,
            &padded,
            &mut out,
        ),
        _ => return None,
    }
    Some(out)
}

fn cbc_encrypt(cipher: &impl BlockEncrypt, iv: [u8; 16], data: &[u8], out: &mut Vec<u8>) {
    let mut previous = iv;
    for chunk in data.chunks_exact(16) {
        let mut block = GenericArray::clone_from_slice(chunk);
        block.iter_mut().zip(previous).for_each(|(a, b)| *a ^= b);
        cipher.encrypt_block(&mut block);
        previous.copy_from_slice(&block);
        out.extend_from_slice(&block);
    }
}

/// AES-256 CBC with a zero IV and no padding over whole blocks: /UE, /OE
/// and (one block, so ECB) /Perms. `key` is 32 bytes by construction.
fn aes256_wrap(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    if key.len() == 32 {
        cbc_encrypt(
            &Aes256::new(GenericArray::from_slice(key)),
            [0; 16],
            data,
            &mut out,
        );
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

    #[test]
    fn r5_r6_hash_matches_qpdf_derived_vectors() {
        // Computed with pypdf 6.18.1's AlgV5.calculate_hash (a port of
        // qpdf's), independent of this code: password, salt, /U data, then
        // the R6 (Algorithm 2.B) and R5 (SHA-256) hashes.
        let vectors = [
            (
                "",
                "0000000000000000",
                "",
                "439feba099a63d0d035a1e5fb67ff307329189584956425aff2d3bd3d15edc60",
                "af5570f5a1810b7af78caf4bc70a660f0df51e42baf91d4de5b2328de0e83dfc",
            ),
            (
                "75736572",
                "0102030405060708",
                "",
                "17424b40ead366f7ddef0ff073608aa68ba701714b5cef3409b94c4ffa763726",
                "ad7c98e251cb1c7b3e2830b6f0becbd8352a6ab712232d6e82f2fca45f304dcc",
            ),
            (
                "70c3a4737377c3b67264",
                "a1b2c3d4e5f60718",
                "",
                "64c342569fdf2ea82efd48a974798956b3e27ab54e2f65ef296c23dca967d5ca",
                "837791642b6bdf4fd564526a17f2a8e21e2a501def668cd3247b17d25c3b759b",
            ),
            (
                "6f776e6572",
                "1112131415161718",
                "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f\
                 202122232425262728292a2b2c2d2e2f",
                "80b268f0e8a1b52dfc72e41219d2797adb6129b7e8a8b24ed813cfd05ddcd3e2",
                "581ef060b54822a003fd46cc68ef355525624034430e05f0a1db2072d5eecacd",
            ),
        ];
        for (password, salt, udata, r6, r5) in vectors {
            let udata = hex(&udata.replace(' ', ""));
            let (password, salt) = (hex(password), hex(salt));
            assert_eq!(hash_r56(6, &password, &salt, &udata), hex(r6));
            assert_eq!(hash_r56(5, &password, &salt, &udata), hex(r5));
        }
        // 127 bytes of "x" with 48 bytes of /U data.
        let udata: Vec<u8> = (100..148).collect();
        assert_eq!(
            hash_r56(6, &[b'x'; 127], &hex("ffeeddccbbaa9988"), &udata),
            hex("c2fa550d3fceb9dc0cbb4e9696aeb4110a13ee6f47e4ae9a96542643d3c16871")
        );
    }

    #[test]
    fn created_r6_handler_authenticates_and_carries_perms() {
        let random: [u8; 68] = std::array::from_fn(|i| (i * 7 + 3) as u8);
        let p = standard_permissions(4 | 32);
        assert_eq!(p, -3356, "MuPDF's -3868 with bit 10 set");
        let created = Security::create_r6(b"user", b"owner", p, &random);
        let dict = created.dictionary();
        for (password, owner) in [(b"user".as_slice(), false), (b"owner", true)] {
            let opened = Security::authenticate(dict, b"", password).unwrap();
            assert_eq!(opened.key, random[..32]);
            assert_eq!((opened.owner, opened.permissions), (owner, p));
            assert_eq!(
                (opened.strings, opened.streams),
                (Method::AesV3, Method::AesV3)
            );
        }
        assert_eq!(
            Security::authenticate(dict, b"", b"").unwrap_err(),
            SecurityError::Password
        );
        // Algorithm 10: /Perms decrypts (AES-256 ECB) to P, 0xFFFFFFFF, "Tadb".
        let Some(Object::String(perms)) = dict.get(b"Perms".as_slice()) else {
            panic!("no /Perms");
        };
        let block = aes256_cbc_raw(&random[..32], &[0; 16], perms);
        assert_eq!(block[..4], p.to_le_bytes());
        assert_eq!(block[4..12], *b"\xFF\xFF\xFF\xFFTadb");
        assert_eq!(block[12..], random[64..]);
        // An empty owner password is the user password.
        let same = Security::create_r6(b"user", b"", p, &random);
        let opened = Security::authenticate(same.dictionary(), b"", b"user").unwrap();
        assert!(opened.owner);
        // Passwords are truncated to 127 bytes.
        let long = Security::create_r6(&[b'x'; 200], b"o", p, &random);
        assert!(Security::authenticate(long.dictionary(), b"", &[b'x'; 127]).is_ok());
        assert!(Security::authenticate(long.dictionary(), b"", &[b'x'; 126]).is_err());
    }

    #[test]
    fn encrypt_reverses_decrypt_for_every_method() {
        let random: [u8; 68] = std::array::from_fn(|i| i as u8);
        let mut security = Security::create_r6(b"u", b"o", -4, &random);
        let id = ObjectId::new(12, 3).unwrap();
        for key in [vec![7; 5], vec![9; 16], random[..32].to_vec()] {
            security.key = key;
            for method in [Method::Identity, Method::Rc4, Method::AesV2, Method::AesV3] {
                // AES needs a 16- or 32-byte key, which authentication
                // guarantees (AESV2 object keys are 16 bytes from an
                // 11-byte or longer file key).
                let short = match method {
                    Method::AesV2 => security.key.len() < 11,
                    Method::AesV3 => ![16, 32].contains(&security.key.len()),
                    _ => false,
                };
                if short {
                    assert_eq!(security.encrypt(method, id, b"x", [1; 16]), None);
                    continue;
                }
                for len in [0, 1, 15, 16, 17, 100] {
                    let data: Vec<u8> = (0..len).map(|i| (i * 31) as u8).collect();
                    let sealed = security.encrypt(method, id, &data, [5; 16]).unwrap();
                    assert_eq!(
                        security.decrypt(method, id, &sealed),
                        data,
                        "{method:?} {len}"
                    );
                }
            }
        }
    }

    #[test]
    fn short_aes_keys_neither_panic_nor_authenticate_shorter() {
        // AESV2 uses a 128-bit key whatever /Length says (as MuPDF does);
        // AESV3 below revision 5 is refused rather than run with a short key.
        assert!(aes_cbc_padded(&[0; 10], &[0; 32]).is_empty());
        assert_eq!(aes_cbc_encrypt(&[0; 10], [0; 16], b""), None);
        let mut dict = Dictionary::new();
        for (key, value) in [
            (b"Filter".as_slice(), Object::Name(b"Standard".to_vec())),
            (b"V", Object::Integer(4)),
            (b"R", Object::Integer(4)),
            (b"Length", Object::Integer(40)),
            (b"P", Object::Integer(-4)),
            (b"O", Object::String(vec![0; 32])),
            (b"U", Object::String(vec![0; 32])),
            (b"StmF", Object::Name(b"StdCF".to_vec())),
            (b"StrF", Object::Name(b"StdCF".to_vec())),
        ] {
            dict.insert(key.to_vec(), value);
        }
        let cf = |method: &[u8]| {
            let mut filter = Dictionary::new();
            filter.insert(b"CFM".to_vec(), Object::Name(method.to_vec()));
            let mut filters = Dictionary::new();
            filters.insert(b"StdCF".to_vec(), Object::Dictionary(filter));
            Object::Dictionary(filters)
        };
        dict.insert(b"CF".to_vec(), cf(b"AESV3"));
        assert_eq!(
            Security::authenticate(&dict, b"", b"").unwrap_err(),
            SecurityError::Unsupported("AESV3 below revision 5")
        );
        dict.insert(b"CF".to_vec(), cf(b"AESV2"));
        assert_eq!(EncryptionInfo::from_dictionary(&dict).length, 128);
    }

    #[test]
    fn passwords_convert_to_pdfdoc_as_mupdf_does() {
        // MuPDF 1.28.1, measured by tools/docenc-password-oracle.mjs on every
        // code point to U+00FF but the comma, the table's other characters
        // and four without a code: each maps to its byte or is dropped.
        let measured: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string("reference/app-encrypt-20261014/docenc-passwords.json")
                .unwrap(),
        )
        .unwrap();
        let measured = measured["passwords"].as_object().unwrap();
        assert_eq!(measured.len(), 298);
        for (code, byte) in measured {
            let c = char::from_u32(u32::from_str_radix(code, 16).unwrap()).unwrap();
            let byte = byte.as_str().unwrap();
            let expected = (byte != "dropped").then(|| u8::from_str_radix(byte, 16).unwrap());
            assert_eq!(pdf_doc_byte(c), expected, "{c:?}");
        }
        assert_eq!(
            pdf_doc_password("pässwort€中".as_bytes()),
            Some(b"p\xE4sswort\xA0".to_vec())
        );
        // Unchanged, not UTF-8, or nothing left: only the bytes as given.
        for password in [b"ascii".as_slice(), b"p\xE4ss", "中".as_bytes(), b""] {
            assert_eq!(pdf_doc_password(password), None);
        }
    }

    #[test]
    fn debug_never_prints_the_file_key() {
        let random = [0xAB; 68];
        let security = Security::create_r6(b"u", b"o", -4, &random);
        let debug = format!("{security:?}");
        assert!(!debug.contains("171"), "{debug}");
        assert!(debug.contains("revision: 6"), "{debug}");
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
}
