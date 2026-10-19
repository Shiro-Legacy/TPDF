// SPDX-License-Identifier: MIT
//! PEM: the block scanner, and reading a signing identity from PEM text
//! (a private key, plain or password-protected, plus its certificates).

use crate::identity::{Found, Identity, SignError};
use crate::keys::{KeyError, SigningKey};
use base64ct::{Base64, Encoding};
use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use md5::{Digest, Md5};
use std::collections::HashMap;
use zeroize::Zeroizing;

/// One `-----BEGIN label----- … -----END label-----` block.
pub struct Block {
    pub label: String,
    /// `Name: value` lines before the base64 (legacy encrypted keys).
    pub headers: Vec<(String, String)>,
    /// The decoded body, `None` when it is not base64.
    pub der: Option<Zeroizing<Vec<u8>>>,
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || from > hay.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// The longest label read; longer `-----BEGIN …-----` text is not a marker.
const MAX_LABEL: usize = 64;

fn valid_label(label: &[u8]) -> bool {
    !label.is_empty()
        && label.len() <= MAX_LABEL
        && label
            .iter()
            .all(|&b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b' ')
}

/// Every well-formed block in `text`, in order. A BEGIN line's label must be
/// uppercase letters, digits and spaces, and a block ends at the first END
/// line with the label of a BEGIN still open; everything open before it is
/// then closed. One pass over the text, so a large file of markers costs
/// time in proportion to its size.
pub fn blocks(text: &[u8]) -> Vec<Block> {
    let mut out = Vec::new();
    // Open BEGIN markers: label -> where its body starts (the earliest one).
    let mut open: HashMap<&[u8], usize> = HashMap::new();
    let mut at = 0;
    while let Some(dashes) = find(text, b"-----", at) {
        let rest = &text[dashes + 5..];
        let (is_begin, skip) = if rest.starts_with(b"BEGIN ") {
            (true, 6)
        } else if rest.starts_with(b"END ") {
            (false, 4)
        } else {
            at = dashes + 1;
            continue;
        };
        let label_start = dashes + 5 + skip;
        let window = &text[label_start..text.len().min(label_start + MAX_LABEL + 5)];
        let Some(label_len) = window.windows(5).position(|w| w == b"-----") else {
            at = dashes + 1;
            continue;
        };
        let label = &text[label_start..label_start + label_len];
        let marker_end = label_start + label_len + 5;
        if !valid_label(label) {
            at = dashes + 1;
            continue;
        }
        at = marker_end;
        if is_begin {
            open.entry(label).or_insert(marker_end);
            continue;
        }
        let Some(&body_start) = open.get(label) else {
            continue;
        };
        // A fresh map (not `clear`, which keeps the capacity and would cost its
        // size on every END).
        open = HashMap::new();
        let end = dashes;
        let body = Zeroizing::new(text[body_start..end].to_vec());
        let mut headers = Vec::new();
        let mut b64 = Zeroizing::new(Vec::new());
        for line in body.split(|&b| b == b'\n' || b == b'\r') {
            let trimmed = line.trim_ascii();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(colon) = trimmed.iter().position(|&b| b == b':') {
                let name = String::from_utf8_lossy(&trimmed[..colon])
                    .trim()
                    .to_string();
                let value = String::from_utf8_lossy(&trimmed[colon + 1..])
                    .trim()
                    .to_string();
                headers.push((name, value));
            } else {
                b64.extend(trimmed.iter().filter(|b| !b.is_ascii_whitespace()));
            }
        }
        let der = std::str::from_utf8(&b64)
            .ok()
            .and_then(|s| Base64::decode_vec(s).ok())
            .map(Zeroizing::new);
        out.push(Block {
            label: String::from_utf8_lossy(label).into_owned(),
            headers,
            der,
        });
    }
    out
}

/// PEM text of one certificate, 64 base64 characters a line.
pub fn certificate_pem(der: &[u8]) -> String {
    let b64 = Base64::encode_string(der);
    let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
    for line in b64.as_bytes().chunks(64) {
        out.push_str(&String::from_utf8_lossy(line));
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----");
    out
}

/// OpenSSL's EVP_BytesToKey with MD5 and one round, the key derivation of
/// legacy `Proc-Type: 4,ENCRYPTED` keys.
fn bytes_to_key(password: &[u8], salt: &[u8], len: usize) -> Zeroizing<Vec<u8>> {
    let mut out = Zeroizing::new(Vec::new());
    let mut prev: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::new());
    while out.len() < len {
        let mut h = Md5::new();
        h.update(&prev);
        h.update(password);
        h.update(salt);
        prev = Zeroizing::new(h.finalize().to_vec());
        out.extend_from_slice(&prev);
    }
    out.truncate(len);
    out
}

fn hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Decrypts a legacy encrypted PEM body; `None` on a wrong password or an
/// unknown cipher.
fn legacy_decrypt(block: &Block, der: &[u8], password: &str) -> Option<Zeroizing<Vec<u8>>> {
    let dek = block
        .headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("DEK-Info"))?;
    let (cipher, iv_hex) = dek.1.split_once(',')?;
    let iv = hex(iv_hex.trim())?;
    let salt = iv.get(..8)?;
    let pass = password.as_bytes();
    let plain = match cipher.trim().to_ascii_uppercase().as_str() {
        "DES-EDE3-CBC" => {
            let key = bytes_to_key(pass, salt, 24);
            cbc::Decryptor::<des::TdesEde3>::new_from_slices(&key, &iv)
                .ok()?
                .decrypt_padded_vec_mut::<Pkcs7>(der)
                .ok()?
        }
        "DES-CBC" => {
            let key = bytes_to_key(pass, salt, 8);
            cbc::Decryptor::<des::Des>::new_from_slices(&key, &iv)
                .ok()?
                .decrypt_padded_vec_mut::<Pkcs7>(der)
                .ok()?
        }
        "AES-128-CBC" => {
            let key = bytes_to_key(pass, salt, 16);
            cbc::Decryptor::<aes::Aes128>::new_from_slices(&key, &iv)
                .ok()?
                .decrypt_padded_vec_mut::<Pkcs7>(der)
                .ok()?
        }
        "AES-192-CBC" => {
            let key = bytes_to_key(pass, salt, 24);
            cbc::Decryptor::<aes::Aes192>::new_from_slices(&key, &iv)
                .ok()?
                .decrypt_padded_vec_mut::<Pkcs7>(der)
                .ok()?
        }
        "AES-256-CBC" => {
            let key = bytes_to_key(pass, salt, 32);
            cbc::Decryptor::<aes::Aes256>::new_from_slices(&key, &iv)
                .ok()?
                .decrypt_padded_vec_mut::<Pkcs7>(der)
                .ok()?
        }
        _ => return None,
    };
    Some(Zeroizing::new(plain))
}

/// Reads PEM text: the last private-key block is the key, and every
/// CERTIFICATE block is a certificate. A key that cannot be read or decrypted
/// is `bad-password`, as before the port.
pub fn parse_pem(text: &[u8], password: &str) -> Result<Identity, SignError> {
    let all = blocks(text);
    if all.is_empty() {
        return Err(SignError::Unreadable);
    }
    let mut found = Found::default();
    for block in &all {
        if block.label.contains("PRIVATE KEY") {
            let der = block.der.as_ref().ok_or(SignError::BadPassword)?;
            let legacy = block
                .headers
                .iter()
                .any(|(n, v)| n.eq_ignore_ascii_case("Proc-Type") && v.contains("ENCRYPTED"));
            let key = if block.label == "ENCRYPTED PRIVATE KEY" {
                let plain = crate::pkcs12::decrypt_encrypted_private_key_info(
                    der,
                    &crate::pkcs12::Pass::new(password),
                )
                .map_err(|e| match e {
                    crate::pkcs12::DecryptError::TooCostly => SignError::TooCostly,
                    _ => SignError::BadPassword,
                })?;
                SigningKey::from_pkcs8_der(&plain)
            } else {
                let plain = if legacy {
                    legacy_decrypt(block, der, password).ok_or(SignError::BadPassword)?
                } else {
                    Zeroizing::new(der.to_vec())
                };
                match block.label.as_str() {
                    "PRIVATE KEY" => SigningKey::from_pkcs8_der(&plain),
                    "RSA PRIVATE KEY" => SigningKey::from_pkcs1_der(&plain),
                    "EC PRIVATE KEY" => SigningKey::from_sec1_der(&plain),
                    "DSA PRIVATE KEY" => Err(KeyError::Unsupported),
                    _ => Err(KeyError::Malformed),
                }
            };
            found.key = Some(match key {
                Ok(k) => Ok(k),
                Err(KeyError::Unsupported) => Err(KeyError::Unsupported),
                Err(KeyError::Malformed) => return Err(SignError::BadPassword),
            });
        } else if block.label == "CERTIFICATE"
            && let Some(der) = &block.der
        {
            found.certs.push(der.to_vec());
        }
    }
    found.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanner_skips_garbage_and_mismatched_ends() {
        let text = b"junk\n-----BEGIN lower-----\nAAAA\n-----END lower-----\n-----BEGIN CERTIFICATE-----\nAAEC\n-----END CERTIFICATE-----\n-----BEGIN X-----\nAAAA\n-----END Y-----\n";
        let found = blocks(text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].label, "CERTIFICATE");
        assert_eq!(
            found[0].der.as_deref().map(Vec::as_slice),
            Some([0u8, 1, 2].as_slice())
        );
        let pem = certificate_pem(&[0u8, 1, 2]);
        assert_eq!(
            pem,
            "-----BEGIN CERTIFICATE-----\nAAEC\n-----END CERTIFICATE-----"
        );
        assert!(matches!(
            parse_pem(b"nothing", ""),
            Err(SignError::Unreadable)
        ));
    }

    #[test]
    fn many_markers_are_read_in_one_pass() {
        // 256 KB of BEGIN lines took seconds with a rescan per marker.
        let text = "-----BEGIN A-----\n".repeat(256 * 1024 / 18);
        let start = std::time::Instant::now();
        assert!(blocks(text.as_bytes()).is_empty());
        let mixed: String = (0..5000)
            .map(|i| format!("-----BEGIN L{i}-----\n"))
            .collect();
        assert!(blocks(mixed.as_bytes()).is_empty());
        let ends = "-----END A-----\n".repeat(20_000);
        assert!(blocks(format!("-----BEGIN A-----\nAAEC\n{ends}").as_bytes()).len() == 1);
        assert!(start.elapsed().as_secs() < 5, "{:?}", start.elapsed());
    }

    #[test]
    fn bytes_to_key_matches_openssl() {
        // `openssl enc -aes-128-cbc -k pw -S 0102030405060708 -P -md md5 -nosalt`-style vector:
        // key = MD5(pw || salt) for a 16-byte key.
        let key = bytes_to_key(b"pw", &[1, 2, 3, 4, 5, 6, 7, 8], 16);
        let mut h = Md5::new();
        h.update(b"pw");
        h.update([1u8, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(key.as_slice(), h.finalize().as_slice());
    }
}
