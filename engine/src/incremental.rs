// SPDX-License-Identifier: AGPL-3.0-or-later
//! Incremental updates: the bytes a document was opened from, unchanged,
//! followed by the objects that changed since, one classic xref section for
//! them and a trailer whose /Prev is the original's last `startxref` (a
//! table or a cross-reference stream, so an original written with xref
//! streams stays readable). A signed original keeps its signatures valid:
//! every signed byte range is a prefix of the result.
//!
//! An encrypted document's changed objects are encrypted with its own
//! handler and file key; /Encrypt stays in the trailer as the original has
//! it (a reference or a dictionary). /ID keeps its first element, which R2-R4
//! keys are derived from, and gets a new second one (ISO 32000-2 14.4: it
//! changes when the file is updated), the leading 16 bytes of a SHA-256 of
//! the update, so the same update gives the same bytes.
use std::borrow::Cow;
use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use crate::document::Document;
use crate::encrypt::{Drbg, OutputCipher};
use crate::object::{Object, ObjectId};
use crate::serialize::{SerializeError, SerializeStyle, WriteLimits, Writer};
use crate::writer::{DocumentWriteError, DocumentWriteLimits, Entropy};

/// The largest offset a classic xref row can hold.
const MAX_OFFSET: u64 = 9_999_999_999;

/// The original bytes of `document` followed by an incremental update that
/// holds the current state of each object in `changed`: its value (and
/// stream), or a free entry when the document no longer has it. The update
/// counts against `limits` together with the original. An encrypted document
/// needs `entropy` (AES initialization vectors); a document whose /Encrypt
/// was not authenticated, or whose /Encrypt dictionary changed, is refused.
pub fn write_incremental(
    document: &Document<'_>,
    changed: &BTreeSet<ObjectId>,
    limits: DocumentWriteLimits,
    entropy: Option<Entropy>,
) -> Result<Vec<u8>, DocumentWriteError> {
    let original = document.input();
    if changed.len() > limits.max_indirect_objects {
        return Err(DocumentWriteError::ObjectLimit);
    }
    let prev = startxref(original).ok_or(DocumentWriteError::Invalid("original startxref"))?;
    let encrypt = document
        .trailer_entry(b"Encrypt")
        .filter(|value| !value.is_null());
    let security = document.security();
    match (encrypt, security) {
        (Some(_), None) => {
            return Err(DocumentWriteError::Unsupported(
                "/Encrypt without an authenticated security handler",
            ));
        }
        (None, Some(_)) => {
            return Err(DocumentWriteError::Unsupported(
                "encryption removed in an incremental update",
            ));
        }
        (Some(Object::Reference(id)), _) if changed.contains(id) => {
            return Err(DocumentWriteError::Unsupported(
                "/Encrypt changed in an incremental update",
            ));
        }
        _ => {}
    }
    let mut cipher = match security {
        Some(security) => {
            let entropy = entropy.ok_or(DocumentWriteError::Invalid(
                "encrypted output needs entropy",
            ))?;
            Some(OutputCipher::new(
                Cow::Borrowed(security),
                Drbg::new(&entropy.0),
            ))
        }
        None => None,
    };
    let room = limits
        .syntax
        .max_output_bytes
        .checked_sub(original.len())
        .ok_or(SerializeError::OutputLimit)?;
    let mut writer = Writer::with_style(
        WriteLimits {
            max_output_bytes: room,
            ..limits.syntax
        },
        SerializeStyle::Compact,
    );
    // The original may end without an end-of-line.
    if !matches!(original.last(), Some(b'\n' | b'\r')) {
        writer.append(b"\n")?;
    }
    let offset = |writer: &Writer| original.len().saturating_add(writer.len());
    let mut entries = Vec::new();
    for &id in changed {
        let Ok(object) = document.object(id) else {
            entries.push((id, None));
            continue;
        };
        let at = offset(&writer);
        if at as u64 > MAX_OFFSET {
            return Err(DocumentWriteError::OffsetLimit);
        }
        entries.push((id, Some(at)));
        writer.append(format!("{} {} obj\n", id.number(), id.generation()).as_bytes())?;
        let mut crypt = cipher.as_mut().filter(|cipher| !cipher.exempt(object));
        let value = match crypt.as_mut() {
            Some(crypt) => Cow::Owned(crypt.strings(&object.value, id, limits.syntax.max_depth)?),
            None => Cow::Borrowed(&object.value),
        };
        let data = document
            .stream_data(id)
            .map_err(|_| DocumentWriteError::Invalid("stream lookup"))?;
        match (data, value.as_ref()) {
            (Some(data), Object::Dictionary(dictionary)) => {
                let data = match crypt.as_mut() {
                    Some(crypt) => {
                        let left = room.saturating_sub(writer.len());
                        Cow::Owned(crypt.stream(dictionary, id, data, left)?)
                    }
                    None => Cow::Borrowed(data),
                };
                writer.stream_dictionary(dictionary, data.len())?;
                writer.append(b"\nstream\n")?;
                writer.append(&data)?;
                writer.append(b"\nendstream")?;
            }
            (Some(_), _) => return Err(DocumentWriteError::Invalid("stream dictionary")),
            (None, value) => writer.value(value, 0)?,
        }
        writer.append(b"\nendobj\n")?;
    }
    let xref = offset(&writer);
    if xref as u64 > MAX_OFFSET {
        return Err(DocumentWriteError::OffsetLimit);
    }
    xref_section(&mut writer, &entries)?;
    let body = writer.finish();

    let size = document
        .trailer_chain()
        .filter_map(|trailer| trailer.get(b"Size".as_slice())?.as_i64())
        .filter(|&size| size > 0)
        .chain(document.next_id().ok().map(|id| i64::from(id.number())))
        .chain(changed.last().map(|id| i64::from(id.number()) + 1))
        .max()
        .unwrap_or(1);
    let size = Object::Integer(size);
    let root = Object::Reference(document.root());
    let info = document.trailer_entry(b"Info");
    let id = file_id(document.trailer_entry(b"ID"), encrypt.is_some(), &body);
    let prev = Object::Integer(i64::try_from(prev).unwrap_or(i64::MAX));
    let mut trailer = Writer::with_style(
        WriteLimits {
            max_output_bytes: room.saturating_sub(body.len()),
            ..limits.syntax
        },
        SerializeStyle::Compact,
    );
    trailer.append(b"trailer\n")?;
    let fields = [
        (b"Size".as_slice(), Some(&size)),
        (b"Root", Some(&root)),
        (b"Info", info),
        (b"ID", id.as_ref()),
        (b"Encrypt", encrypt),
        (b"Prev", Some(&prev)),
    ];
    trailer.dictionary_entries(fields.into_iter().filter_map(|(k, v)| Some((k, v?))))?;
    trailer.append(format!("\nstartxref\n{xref}\n%%EOF\n").as_bytes())?;
    let trailer = trailer.finish();

    let total = original
        .len()
        .checked_add(body.len())
        .and_then(|n| n.checked_add(trailer.len()))
        .ok_or(SerializeError::OutputLimit)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(total)
        .map_err(|_| DocumentWriteError::AllocationFailed)?;
    bytes.extend_from_slice(original);
    bytes.extend_from_slice(&body);
    bytes.extend_from_slice(&trailer);
    Ok(bytes)
}

/// The update's /ID: the original's first element and a digest of `body`.
/// Without a usable /ID, an unencrypted file gets two digests and an
/// encrypted one keeps what it had (a new first element would change the
/// key readers derive).
fn file_id(original: Option<&Object>, encrypted: bool, body: &[u8]) -> Option<Object> {
    let digest = Sha256::digest(body);
    let second = Object::String(digest.get(..16).unwrap_or_default().to_vec());
    match original.and_then(Object::as_array) {
        Some([first @ Object::String(_), ..]) => Some(Object::Array(vec![first.clone(), second])),
        _ if encrypted => original.cloned(),
        _ => Some(Object::Array(vec![second.clone(), second])),
    }
}

/// An xref table of `entries` in ascending object order: an offset for an
/// object in use, None for a free one.
fn xref_section(
    writer: &mut Writer,
    entries: &[(ObjectId, Option<usize>)],
) -> Result<(), DocumentWriteError> {
    writer.append(b"xref\n")?;
    let mut rest = entries;
    while let Some((&(first, _), _)) = rest.split_first() {
        let run = 1 + rest
            .windows(2)
            .take_while(|pair| pair[0].0.number().checked_add(1) == Some(pair[1].0.number()))
            .count();
        let (section, tail) = rest.split_at(run.min(rest.len()));
        writer.append(format!("{} {}\n", first.number(), section.len()).as_bytes())?;
        for &(id, offset) in section {
            let line = match offset {
                Some(offset) => format!("{offset:010} {:05} n \n", id.generation()),
                None => format!("0000000000 {:05} f \n", id.generation().saturating_add(1)),
            };
            writer.append(line.as_bytes())?;
        }
        rest = tail;
    }
    Ok(())
}

/// The offset after the last `startxref` keyword in the final kilobyte.
pub fn startxref(bytes: &[u8]) -> Option<usize> {
    let tail = bytes.get(bytes.len().saturating_sub(1024)..)?;
    let at = tail.windows(9).rposition(|w| w == b"startxref")?;
    let rest = tail.get(at + 9..)?;
    let digits: Vec<u8> = rest
        .iter()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .take_while(u8::is_ascii_digit)
        .collect();
    std::str::from_utf8(&digits).ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startxref_reads_the_last_keyword() {
        assert_eq!(
            startxref(b"%PDF-1.7\nstartxref\n12\n%%EOF\nstartxref\r\n 345 \n%%EOF"),
            Some(345)
        );
        assert_eq!(startxref(b"no keyword"), None);
        assert_eq!(startxref(b"startxref\n%%EOF"), None);
        assert_eq!(startxref(b"startxref\n99999999999999999999999"), None);
        assert_eq!(startxref(b""), None);
    }

    #[test]
    fn the_second_identifier_follows_the_update() {
        let first = Object::String(b"0123456789abcdef".to_vec());
        let original = Object::Array(vec![first.clone(), first.clone()]);
        let a = file_id(Some(&original), false, b"a");
        let b = file_id(Some(&original), true, b"b");
        let (Some(Object::Array(a)), Some(Object::Array(b))) = (a, b) else {
            panic!("an /ID array");
        };
        assert_eq!((a.first(), b.first()), (Some(&first), Some(&first)));
        assert_ne!(a.get(1), b.get(1));
        assert_eq!(file_id(None, true, b"a"), None);
        let Some(Object::Array(fresh)) = file_id(None, false, b"a") else {
            panic!("a new /ID");
        };
        assert_eq!(fresh.first(), fresh.get(1));
    }
}
