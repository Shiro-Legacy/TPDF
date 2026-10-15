// SPDX-License-Identifier: AGPL-3.0-or-later
//! Encrypted output for `writer`. Randomness comes from the caller as 32
//! bytes of entropy, expanded with HMAC_DRBG (NIST SP 800-90A Rev. 1, SHA-256,
//! no reseeding or prediction resistance): one instance for the secret file
//! key, another for the public salts, IVs and /ID, so no output byte serves
//! both even when entropy is reused.
//! Every string and stream of a written object is encrypted with its object
//! key except what ISO 32000-2 7.6.2 exempts: /ID, the encryption dictionary
//! (written directly in the trailer, so every indirect object is encrypted),
//! cross-reference streams, the /Contents of signature dictionaries, and
//! Metadata streams when /EncryptMetadata is false. Strings inside encrypted
//! streams are covered by the stream. The reader decrypts the same set.
use std::borrow::Cow;

use sha2::{Digest, Sha256};

use crate::document::IndirectObject;
use crate::object::{Dictionary, Object, ObjectId};
use crate::security::{Method, Security, is_signature_dictionary};
use crate::serialize::SerializeError;
use crate::writer::DocumentWriteError;

/// Personalization strings (SP 800-90A 8.7.1) that separate the public
/// values from the file key drawn from the same entropy.
const PUBLIC: &[u8] = b"Tandem PDF writer";
const SECRET: &[u8] = b"Tandem PDF writer: file key";
/// Bytes per Generate request; SP 800-90A allows up to 2^16 for HMAC_DRBG.
const REQUEST: usize = 1024;

/// HMAC_DRBG with SHA-256. Output is drawn in `REQUEST`-byte Generate
/// calls and handed out in order, so a seed fixes every byte.
pub(crate) struct Drbg {
    key: [u8; 32],
    value: [u8; 32],
    buffer: Vec<u8>,
    used: usize,
}

impl Drbg {
    /// Instantiate with `entropy` (the caller's seed, also serving as the
    /// nonce) for public values: salts, IVs and /ID.
    pub(crate) fn new(entropy: &[u8]) -> Self {
        Self::instantiate(&[entropy, PUBLIC])
    }
    /// Instantiate with `entropy` for the file key alone.
    pub(crate) fn secret(entropy: &[u8]) -> Self {
        Self::instantiate(&[entropy, SECRET])
    }

    fn instantiate(seed: &[&[u8]]) -> Self {
        let mut drbg = Self {
            key: [0; 32],
            value: [1; 32],
            buffer: Vec::new(),
            used: 0,
        };
        drbg.update(seed);
        drbg
    }

    fn update(&mut self, data: &[&[u8]]) {
        let provided = data.iter().any(|part| !part.is_empty());
        for round in [0u8, 1] {
            if round == 1 && !provided {
                break;
            }
            let mut parts = vec![self.value.as_slice(), std::slice::from_ref(&round)];
            parts.extend_from_slice(data);
            self.key = hmac(&self.key, &parts);
            self.value = hmac(&self.key, &[&self.value]);
        }
    }

    /// One Generate call without additional input.
    fn generate(&mut self, out: &mut [u8]) {
        for chunk in out.chunks_mut(32) {
            self.value = hmac(&self.key, &[&self.value]);
            chunk.copy_from_slice(&self.value[..chunk.len()]);
        }
        self.update(&[]);
    }

    pub(crate) fn fill(&mut self, out: &mut [u8]) {
        let mut filled = 0;
        while filled < out.len() {
            if self.used == self.buffer.len() {
                let mut block = vec![0; REQUEST];
                self.generate(&mut block);
                self.buffer = block;
                self.used = 0;
            }
            let take = (out.len() - filled).min(self.buffer.len() - self.used);
            out[filled..filled + take].copy_from_slice(&self.buffer[self.used..self.used + take]);
            filled += take;
            self.used += take;
        }
    }

    pub(crate) fn bytes<const N: usize>(&mut self) -> [u8; N] {
        let mut out = [0; N];
        self.fill(&mut out);
        out
    }
}

/// HMAC-SHA-256 with a 32-byte key (RFC 2104).
fn hmac(key: &[u8; 32], parts: &[&[u8]]) -> [u8; 32] {
    let mut inner_pad = [0x36; 64];
    let mut outer_pad = [0x5C; 64];
    for (i, byte) in key.iter().enumerate() {
        inner_pad[i] ^= byte;
        outer_pad[i] ^= byte;
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    for part in parts {
        inner.update(part);
    }
    let digest = Sha256::new()
        .chain_update(outer_pad)
        .chain_update(inner.finalize())
        .finalize();
    let mut out = [0; 32];
    out.copy_from_slice(&digest);
    out
}

/// A trailer /ID with `first` kept and a fresh second element, or two equal
/// fresh elements when there is no first one (a file written for the first
/// time, ISO 32000-2 14.4).
pub(crate) fn file_id(first: Option<Vec<u8>>, drbg: &mut Drbg) -> Object {
    let second = drbg.bytes::<16>().to_vec();
    let first = first.unwrap_or_else(|| second.clone());
    Object::Array(vec![Object::String(first), Object::String(second)])
}

/// The handler an output is encrypted with and the randomness for its IVs.
pub(crate) struct OutputCipher<'s> {
    security: Cow<'s, Security>,
    drbg: Drbg,
}

impl<'s> OutputCipher<'s> {
    pub(crate) fn new(security: Cow<'s, Security>, drbg: Drbg) -> Self {
        Self { security, drbg }
    }

    /// Objects whose strings and stream stay in plaintext: cross-reference
    /// streams (any /Type /XRef object, as the reader skips them). A source
    /// object that held /Encrypt is an ordinary object in the output.
    pub(crate) fn exempt(&self, object: &IndirectObject) -> bool {
        has_type(&object.value, b"XRef")
    }

    /// `value` with every string encrypted under object `id`'s key, except
    /// signature /Contents. Containers deeper than `max_depth` fail as the
    /// serializer would.
    pub(crate) fn strings(
        &mut self,
        value: &Object,
        id: ObjectId,
        max_depth: usize,
    ) -> Result<Object, DocumentWriteError> {
        let mut value = value.clone();
        self.encrypt_strings(&mut value, id, 0, max_depth.min(256))?;
        Ok(value)
    }

    fn encrypt_strings(
        &mut self,
        value: &mut Object,
        id: ObjectId,
        depth: usize,
        max_depth: usize,
    ) -> Result<(), DocumentWriteError> {
        match value {
            Object::String(bytes) => {
                let method = self.security.string_method();
                *bytes = self.encrypt(method, id, bytes)?;
            }
            Object::Array(items) => {
                if depth >= max_depth {
                    return Err(SerializeError::DepthLimit.into());
                }
                for item in items {
                    self.encrypt_strings(item, id, depth + 1, max_depth)?;
                }
            }
            Object::Dictionary(entries) => {
                if depth >= max_depth {
                    return Err(SerializeError::DepthLimit.into());
                }
                let signature = is_signature_dictionary(entries);
                for (key, item) in entries.iter_mut() {
                    if !(signature && key.as_slice() == b"Contents") {
                        self.encrypt_strings(item, id, depth + 1, max_depth)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// The (already filtered) stream bytes of object `id` encrypted, failing
    /// before any work when the result could not fit in `room` output bytes.
    pub(crate) fn stream(
        &mut self,
        dictionary: &Dictionary,
        id: ObjectId,
        data: &[u8],
        room: usize,
    ) -> Result<Vec<u8>, DocumentWriteError> {
        if data.len() > room.saturating_sub(32) {
            return Err(SerializeError::OutputLimit.into());
        }
        let metadata =
            matches!(dictionary.get(b"Type".as_slice()), Some(Object::Name(n)) if n == b"Metadata");
        let method = self.security.stream_method(metadata);
        self.encrypt(method, id, data)
    }

    fn encrypt(
        &mut self,
        method: Method,
        id: ObjectId,
        data: &[u8],
    ) -> Result<Vec<u8>, DocumentWriteError> {
        let iv = match method {
            Method::AesV2 | Method::AesV3 => self.drbg.bytes(),
            _ => [0; 16],
        };
        self.security
            .encrypt(method, id, data, iv)
            .ok_or(DocumentWriteError::Invalid("encryption key length"))
    }
}

fn has_type(value: &Object, kind: &[u8]) -> bool {
    matches!(value, Object::Dictionary(d)
        if matches!(d.get(b"Type".as_slice()), Some(Object::Name(n)) if n == kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn hmac_matches_rfc4231_case_2_shape() {
        // RFC 4231 test case 2 uses a 4-byte key; with zero padding to 32
        // bytes HMAC is unchanged, since keys shorter than the block are
        // zero-padded by definition.
        let mut key = [0; 32];
        key[..4].copy_from_slice(b"Jefe");
        assert_eq!(
            hmac(&key, &[b"what do ya want ", b"for nothing?"]).to_vec(),
            hex("5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843")
        );
    }

    #[test]
    fn drbg_matches_rfc6979_deterministic_k() {
        // RFC 6979 A.2.5 (P-256, SHA-256, "sample") instantiates HMAC_DRBG
        // with int2octets(x) || bits2octets(h1); its first candidate k is the
        // first Generate output.
        let x = hex("C9AFA9D845BA75166B5C215767B1D6934E50C3DB36E89B127B8A622B120F6721");
        let h1 = Sha256::digest(b"sample");
        let mut drbg = Drbg::instantiate(&[&x, &h1]);
        let mut k = [0; 32];
        drbg.generate(&mut k);
        assert_eq!(
            k.to_vec(),
            hex("A6E3C57DD01ABE90086538398355DD4C3B17AA873382B0F24D6129493D8AAD60")
        );
    }

    #[test]
    fn drbg_buffers_whole_requests() {
        // Values from an independent Python HMAC_DRBG over the same seed.
        let seed: Vec<u8> = (0..32).collect();
        let mut drbg = Drbg::new(&seed);
        assert_eq!(
            drbg.bytes::<16>().to_vec(),
            hex("5f3194069221284fb4ee50bf0ffb546d")
        );
        let mut rest = vec![0; REQUEST - 32];
        drbg.fill(&mut rest);
        assert_eq!(
            drbg.bytes::<16>().to_vec(),
            hex("d1570df54b6f04c596fc4fcfa56f47b2")
        );
        assert_eq!(
            drbg.bytes::<16>().to_vec(),
            hex("35057a131e1ad157d99093fd52aece03")
        );
    }
}
