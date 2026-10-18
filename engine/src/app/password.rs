// SPDX-License-Identifier: MIT
//! Password bytes for the standard security handler. A password is
//! converted to PDFDocEncoding for R2-R4, dropping characters it cannot
//! encode, and its UTF-8 bytes are used unchanged for R5/R6 (no SASLprep:
//! "ﬁ" does not match "fi" and a soft hyphen is not removed). Either way the password
//! ends at its first NUL, as a C string does.
use crate::document::Document;
use crate::object::{Object, ObjectId, decode_text_string};
use crate::security::{Security, SecurityError, resolve_encryption};

/// The byte strings to try for `password`, the revision's encoding first. The other
/// encoding follows, so an R2-R4 file whose producer wrote UTF-8 bytes (or
/// an R5/R6 file with Latin-1 bytes) also opens. An empty candidate is
/// tried only for the empty password: once converted to nothing, a
/// password can unlock no more than the empty one that `open` tried.
pub fn candidates(password: &str, revision: Option<i64>) -> Vec<Vec<u8>> {
    let password = password.split('\0').next().unwrap_or_default();
    if password.is_empty() {
        return vec![Vec::new()];
    }
    let utf8 = password.as_bytes().to_vec();
    let doc = pdf_doc_bytes(password);
    let ordered = match revision {
        Some(revision) if revision >= 5 => [utf8, doc],
        _ => [doc, utf8],
    };
    let mut out: Vec<Vec<u8>> = Vec::new();
    for candidate in ordered {
        if !candidate.is_empty() && !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

/// `text` in PDFDocEncoding, without the characters it lacks. The codes
/// ISO 32000-2 Table D.2 leaves undefined (0x7F, 0x9F, 0xAD) encode nothing.
pub fn pdf_doc_bytes(text: &str) -> Vec<u8> {
    let table: Vec<(char, u8)> = (0..=u8::MAX)
        .filter(|code| !matches!(code, 0x7F | 0x9F | 0xAD))
        .filter_map(|code| Some((decode_text_string(&[code]).chars().next()?, code)))
        .collect();
    text.chars()
        .filter_map(|c| {
            table
                .iter()
                .find(|(known, _)| *known == c)
                .map(|&(_, code)| code)
        })
        .collect()
}

/// The /R of a document's encryption dictionary, readable while locked.
pub fn revision(doc: &Document<'_>) -> Option<i64> {
    let encrypt = doc.resolve(doc.trailer_entry(b"Encrypt")?);
    match doc.resolve(encrypt.as_dict()?.get(b"R".as_slice())?) {
        Object::Integer(revision) => Some(*revision),
        _ => None,
    }
}

/// True when every one of `passwords` is wrong for the standard security
/// handler of `doc`, a document the strict reader opened locked: reading the
/// same bytes with them would ask for a password again. The handler is read
/// as the strict reader reads it (the newest non-null /Encrypt and /ID, the
/// raw encryption dictionary with its indirect values resolved). False when
/// one of them authenticates, or the handler cannot be checked this way.
pub fn all_wrong(doc: &Document<'_>, passwords: &[&[u8]]) -> bool {
    let effective = |key: &[u8]| {
        doc.trailer_chain()
            .find_map(|trailer| trailer.get(key).filter(|value| !value.is_null()))
    };
    let dictionary = match effective(b"Encrypt") {
        Some(Object::Dictionary(dictionary)) => dictionary,
        Some(Object::Reference(id)) => match doc.resolve_id(*id) {
            Object::Dictionary(dictionary) => dictionary,
            _ => return false,
        },
        _ => return false,
    };
    let lookup = |id: ObjectId| Some(doc.resolve_id(id));
    let dictionary = resolve_encryption(dictionary, &lookup);
    let first_id = match effective(b"ID") {
        Some(Object::Array(items)) => match items.first() {
            Some(Object::String(id)) => id.as_slice(),
            _ => return false,
        },
        _ => &[],
    };
    !passwords.is_empty()
        && passwords.iter().all(|password| {
            matches!(
                Security::authenticate(&dictionary, first_id, password),
                Err(SecurityError::Password)
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_follow_the_revision() {
        assert_eq!(candidates("", Some(4)), [b"".to_vec()]);
        assert_eq!(candidates("abc", None), [b"abc".to_vec()]);
        assert_eq!(candidates("ab\0cd", Some(6)), [b"ab".to_vec()]);
        let latin = vec![b'p', 0xE4, b's'];
        let utf8 = "päs".as_bytes().to_vec();
        assert_eq!(candidates("päs", Some(3)), [latin.clone(), utf8.clone()]);
        assert_eq!(candidates("päs", Some(6)), [utf8.clone(), latin]);
        assert_eq!(candidates("中文", Some(4)), ["中文".as_bytes().to_vec()]);
    }

    #[test]
    fn pdf_doc_bytes_match_the_measured_table() {
        assert_eq!(pdf_doc_bytes("ﬁx€•˘"), [0x93, b'x', 0xA0, 0x80, 0x18]);
        assert_eq!(
            pdf_doc_bytes("a\u{AD}\u{7F}\u{9F}\u{A0}\u{80}\u{18}b"),
            b"ab"
        );
        assert_eq!(pdf_doc_bytes("\u{1}\u{17}"), [1, 0x17]);
        assert_eq!(pdf_doc_bytes("a\u{308}"), b"a");
    }
}
