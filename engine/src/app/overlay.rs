// SPDX-License-Identifier: AGPL-3.0-or-later
//! Current bytes of a journaled document for readers of bytes (hayro, the
//! test harness): a render base followed by one incremental update holding
//! the objects changed since the base was made, whose xref points back at the
//! base's. Unchanged objects are never serialized again; a new revision
//! costs its changed objects plus one copy of the base.
//!
//! The base is the document's own input when it is unencrypted. Otherwise,
//! or when the overlay has grown past half the base, it is a plaintext full
//! rewrite of the current state without renumbering or garbage collection,
//! so every object the overlay can refer to is in it.
use std::collections::BTreeSet;
use std::sync::Arc;

use crate::document::Document;
use crate::object::{Object, ObjectId};
use crate::serialize::{SerializeError, SerializeStyle, Writer};
use crate::writer::{
    DocumentWriteError, DocumentWriteLimits, Encryption, WriteOptions, write_document,
};

/// The output bound of an overlay or a rewritten base.
const MAX_BYTES: usize = 2 * crate::app::MAX_PDF_BYTES;

/// How an overlay writes an object the live document no longer has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Removed {
    /// A free xref entry, as MuPDF's own xref marks a deleted object.
    Free,
    /// An in-use entry for `null`: hayro skips free entries and would still
    /// read the base's definition.
    Null,
}

/// The bytes an overlay is appended to.
#[derive(Clone, Debug)]
pub(crate) struct Base {
    pub(crate) bytes: Arc<Vec<u8>>,
    /// Offset of the base's newest xref section.
    xref: usize,
    /// Object numbers in use, one past the largest.
    size: u64,
    /// Missing objects a rewrite printed references to as null.
    nulled: BTreeSet<ObjectId>,
}

impl Base {
    /// The document's own input, when it is unencrypted and ends with a
    /// readable `startxref`.
    pub(crate) fn input(doc: &Document<'_>, input: Arc<Vec<u8>>) -> Option<Self> {
        if doc.security().is_some() || doc.trailer_entry(b"Encrypt").is_some() {
            return None;
        }
        let xref = startxref(&input)?;
        let size = match doc.trailer().get(b"Size".as_slice()) {
            Some(Object::Integer(size)) => u64::try_from(*size).unwrap_or(0),
            _ => 0,
        };
        Some(Self {
            bytes: input,
            xref,
            size,
            nulled: BTreeSet::new(),
        })
    }

    /// A plaintext rewrite of every object of the current state.
    pub(crate) fn rewrite(doc: &Document<'_>) -> Result<Self, DocumentWriteError> {
        let options = WriteOptions {
            garbage: false,
            encryption: Encryption::Plaintext,
            ..WriteOptions::APP
        };
        let bytes = write_document(doc, limits(), options)?;
        let xref = startxref(&bytes).ok_or(DocumentWriteError::Invalid("rewrite xref"))?;
        Ok(Self {
            bytes: Arc::new(bytes),
            xref,
            size: 0,
            nulled: missing_targets(doc),
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether an object the base refers to as null now exists, so objects
    /// the overlay does not hold would read differently.
    pub(crate) fn stale(&self, doc: &Document<'_>, changed: &BTreeSet<ObjectId>) -> bool {
        (self.nulled.intersection(changed)).any(|id| doc.object(*id).is_ok())
    }
}

/// Targets of references to objects the document does not have.
fn missing_targets(doc: &Document<'_>) -> BTreeSet<ObjectId> {
    let mut missing = BTreeSet::new();
    let mut values: Vec<&Object> = doc.objects().map(|object| &object.value).collect();
    values.extend(doc.effective_trailer_entries().map(|(_, value)| value));
    while let Some(value) = values.pop() {
        match value {
            Object::Reference(id) if doc.object(*id).is_err() => {
                missing.insert(*id);
            }
            Object::Array(items) => values.extend(items),
            Object::Dictionary(items) => values.extend(items.values()),
            _ => {}
        }
    }
    missing
}

pub(crate) fn limits() -> DocumentWriteLimits {
    let mut limits = DocumentWriteLimits::default();
    limits.syntax.max_output_bytes = MAX_BYTES;
    limits.syntax.max_objects = 16_000_000;
    limits.max_indirect_objects = 2_000_000;
    limits
}

/// The offset after the last `startxref` keyword in the final kilobyte.
fn startxref(bytes: &[u8]) -> Option<usize> {
    let tail = &bytes[bytes.len().saturating_sub(1024)..];
    let at = tail.windows(9).rposition(|w| w == b"startxref")?;
    let rest = &tail[at + 9..];
    let digits: Vec<u8> = rest
        .iter()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .take_while(u8::is_ascii_digit)
        .collect();
    std::str::from_utf8(&digits).ok()?.parse().ok()
}

/// An incremental update of `base` holding the current state of `changed`
/// (objects that no longer exist as `removed` says) and the current
/// trailer. Empty when nothing changed.
pub(crate) fn overlay(
    doc: &Document<'_>,
    base: &Base,
    changed: &BTreeSet<ObjectId>,
    trailer_changed: bool,
    removed: Removed,
) -> Result<Vec<u8>, DocumentWriteError> {
    if changed.is_empty() && !trailer_changed {
        return Ok(Vec::new());
    }
    let limits = limits();
    let mut writer = Writer::with_style(limits.syntax, SerializeStyle::Compact);
    // The base may end without an end-of-line.
    writer.append(b"\n")?;
    let at = |writer: &Writer| base.len() + writer.len();
    let mut entries = Vec::new();
    for &id in changed {
        let Ok(object) = doc.object(id) else {
            if removed == Removed::Free {
                entries.push((id, None));
                continue;
            }
            entries.push((id, Some(at(&writer))));
            let null = format!("{} {} obj\nnull\nendobj\n", id.number(), id.generation());
            writer.append(null.as_bytes())?;
            continue;
        };
        entries.push((id, Some(at(&writer))));
        writer.append(format!("{} {} obj\n", id.number(), id.generation()).as_bytes())?;
        let data = doc
            .stream_data(id)
            .map_err(|_| DocumentWriteError::Invalid("stream lookup"))?;
        match (data, &object.value) {
            (Some(data), Object::Dictionary(dictionary)) => {
                writer.stream_dictionary(dictionary, data.len())?;
                writer.append(b"\nstream\n")?;
                writer.append(data)?;
                writer.append(b"\nendstream")?;
            }
            (Some(_), _) => return Err(DocumentWriteError::Invalid("stream dictionary")),
            (None, value) => writer.value(value, 0)?,
        }
        writer.append(b"\nendobj\n")?;
    }
    let xref = at(&writer);
    if xref as u128 > 9_999_999_999 {
        return Err(DocumentWriteError::OffsetLimit);
    }
    writer.append(b"xref\n")?;
    let mut rest = entries.as_slice();
    while let Some((&(first, _), _)) = rest.split_first() {
        let run = 1 + rest
            .windows(2)
            .take_while(|pair| pair[0].0.number().checked_add(1) == Some(pair[1].0.number()))
            .count();
        let (section, tail) = rest.split_at(run.min(rest.len()));
        writer.append(format!("{} {}\n", first.number(), section.len()).as_bytes())?;
        for &(id, offset) in section {
            let line = match offset {
                Some(offset) if offset as u128 <= 9_999_999_999 => {
                    format!("{offset:010} {:05} n \n", id.generation())
                }
                Some(_) => return Err(DocumentWriteError::OffsetLimit),
                None => format!("0000000000 {:05} f \n", id.generation().saturating_add(1)),
            };
            writer.append(line.as_bytes())?;
        }
        rest = tail;
    }
    let live = doc
        .next_id()
        .map_or(u64::from(u32::MAX), |id| u64::from(id.number()));
    let listed = changed.last().map_or(0, |id| u64::from(id.number()) + 1);
    let size = base.size.max(live).max(listed);
    let size = Object::Integer(size.min(i64::MAX as u64) as i64);
    let prev = Object::Integer(base.xref as i64);
    let root = Object::Reference(doc.root());
    let info = doc.trailer_entry(b"Info");
    let id = doc.trailer_entry(b"ID");
    let entries = [
        (b"Size".as_slice(), Some(&size)),
        (b"Prev", Some(&prev)),
        (b"Info", info),
        (b"Root", Some(&root)),
        (b"ID", id),
    ];
    writer.append(b"trailer\n")?;
    writer.dictionary_entries(entries.into_iter().filter_map(|(k, v)| Some((k, v?))))?;
    writer.append(format!("\nstartxref\n{xref}\n%%EOF\n").as_bytes())?;
    Ok(writer.finish())
}

/// `bytes`, a complete PDF, followed by an incremental update that
/// redefines `id` as `value` (the view's private copy, never saved).
pub(crate) fn append(
    mut bytes: Vec<u8>,
    doc: &Document<'_>,
    id: ObjectId,
    value: &Object,
) -> Result<Vec<u8>, DocumentWriteError> {
    let prev = startxref(&bytes).ok_or(DocumentWriteError::Invalid("render xref"))?;
    let mut writer = Writer::with_style(limits().syntax, SerializeStyle::Compact);
    writer.append(b"\n")?;
    let offset = bytes.len() + writer.len();
    writer.append(format!("{} {} obj\n", id.number(), id.generation()).as_bytes())?;
    writer.value(value, 0)?;
    writer.append(b"\nendobj\n")?;
    let xref = bytes.len() + writer.len();
    if xref as u128 > 9_999_999_999 {
        return Err(DocumentWriteError::OffsetLimit);
    }
    writer.append(
        format!(
            "xref\n{} 1\n{offset:010} {:05} n \n",
            id.number(),
            id.generation()
        )
        .as_bytes(),
    )?;
    let live = doc
        .next_id()
        .map_or(u64::from(u32::MAX), |id| u64::from(id.number()));
    let size = live.max(u64::from(id.number()) + 1);
    let size = Object::Integer(size.min(i64::MAX as u64) as i64);
    let prev = Object::Integer(i64::try_from(prev).unwrap_or(i64::MAX));
    let root = Object::Reference(doc.root());
    let entries = [
        (b"Size".as_slice(), Some(&size)),
        (b"Prev", Some(&prev)),
        (b"Info", doc.trailer_entry(b"Info")),
        (b"Root", Some(&root)),
        (b"ID", doc.trailer_entry(b"ID")),
    ];
    writer.append(b"trailer\n")?;
    writer.dictionary_entries(entries.into_iter().filter_map(|(k, v)| Some((k, v?))))?;
    writer.append(format!("\nstartxref\n{xref}\n%%EOF\n").as_bytes())?;
    let update = writer.finish();
    bytes
        .try_reserve_exact(update.len())
        .map_err(|_| DocumentWriteError::Serialize(SerializeError::AllocationFailed))?;
    bytes.extend_from_slice(&update);
    Ok(bytes)
}

/// `base` followed by `overlay`.
pub(crate) fn join(base: &Base, overlay: &[u8]) -> Result<Vec<u8>, SerializeError> {
    let mut bytes = Vec::new();
    let total = base
        .len()
        .checked_add(overlay.len())
        .ok_or(SerializeError::OutputLimit)?;
    bytes
        .try_reserve_exact(total)
        .map_err(|_| SerializeError::AllocationFailed)?;
    bytes.extend_from_slice(&base.bytes);
    bytes.extend_from_slice(overlay);
    Ok(bytes)
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
        assert_eq!(startxref(b""), None);
    }
}
