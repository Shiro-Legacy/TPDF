// SPDX-License-Identifier: AGPL-3.0-or-later
//! Deterministic full rewrites of the readable, unencrypted document subset.
//!
//! `rewrite_document` (ordinary preservation) retains every effective live
//! object and raw stream. Object/xref streams become dormant infrastructure
//! when a new classic xref is written; keeping them avoids destroying
//! references to those objects. `rewrite_reachable` (collection) writes only
//! objects reachable from the trailer, so objects orphaned by edits are not
//! retained; it validates every object exactly as preservation does first.
//! Signatures, linearization, and unknown meaningful trailer metadata are
//! conservatively refused. No file is overwritten: success returns a new
//! buffer, failure returns no output.
//!
//! `write_document` with `WriteOptions::APP` saves as the app's MuPDF save
//! (`incremental=no,garbage=yes`) does instead: reachable objects in compact
//! syntax, no refusals for signatures, permissions or stale linearization,
//! only Root, Size, Info and ID in the trailer, dangling references as null,
//! and an encrypted document's plaintext without /Encrypt.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::document::Document;
use crate::object::{Object, ObjectId};
use crate::serialize::{SerializeError, SerializeStyle, WriteLimits, Writer};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriteOptions {
    /// Write only objects reachable from the trailer's Root, Info and ID.
    pub garbage: bool,
    /// Flate-compress streams created or replaced since open that have no
    /// /Filter. Original streams keep their bytes.
    pub compress_new_streams: bool,
    /// Refuse what `rewrite_document` refuses. Otherwise only a missing
    /// catalog and limits fail.
    pub strict: bool,
    pub style: SerializeStyle,
}
impl WriteOptions {
    /// The app's save: MuPDF `incremental=no,garbage=yes`, which does not
    /// compress streams.
    pub const APP: Self = Self {
        garbage: true,
        compress_new_streams: false,
        strict: false,
        style: SerializeStyle::Compact,
    };
}
/// `rewrite_document`'s options.
impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            garbage: false,
            compress_new_streams: false,
            strict: true,
            style: SerializeStyle::Canonical,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DocumentWriteLimits {
    /// Output bytes, syntax nesting, and scalar/container/key count across the
    /// entire output document, rather than resetting for each indirect object.
    pub syntax: WriteLimits,
    pub max_indirect_objects: usize,
}
impl Default for DocumentWriteLimits {
    fn default() -> Self {
        Self {
            syntax: WriteLimits {
                max_output_bytes: 128 * 1024 * 1024,
                max_depth: 64,
                max_objects: 1_000_000,
            },
            max_indirect_objects: 100_000,
        }
    }
}
#[derive(Debug)]
pub enum DocumentWriteError {
    Unsupported(&'static str),
    Invalid(&'static str),
    MissingReference(ObjectId),
    ObjectLimit,
    OffsetLimit,
    AllocationFailed,
    Serialize(SerializeError),
}
impl fmt::Display for DocumentWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(reason) => write!(f, "unsupported document rewrite: {reason}"),
            Self::Invalid(reason) => write!(f, "invalid document rewrite: {reason}"),
            Self::MissingReference(id) => write!(
                f,
                "document rewrite references missing object {} {}",
                id.number(),
                id.generation()
            ),
            Self::ObjectLimit => f.write_str("document rewrite indirect-object limit exceeded"),
            Self::OffsetLimit => {
                f.write_str("document rewrite classic xref offset exceeds ten digits")
            }
            Self::AllocationFailed => f.write_str("document rewrite allocation failed"),
            Self::Serialize(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for DocumentWriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Serialize(error) => Some(error),
            _ => None,
        }
    }
}
impl From<SerializeError> for DocumentWriteError {
    fn from(value: SerializeError) -> Self {
        Self::Serialize(value)
    }
}

// Object depth is already capped by Document::open, but enforce the writer's
// chosen limit too. Never follow references recursively; every target is scanned
// once in the document object inventory, so cycles do not amplify this walk.
fn inspect(
    value: &Object,
    document: &Document<'_>,
    depth: usize,
    limits: WriteLimits,
    count: &mut usize,
) -> Result<(), DocumentWriteError> {
    *count = count
        .checked_add(1)
        .filter(|&n| n <= limits.max_objects)
        .ok_or(SerializeError::ObjectLimit)?;
    match value {
        Object::Reference(id) => {
            document
                .object(*id)
                .map_err(|_| DocumentWriteError::MissingReference(*id))?;
        }
        Object::Array(values) => {
            if depth >= limits.max_depth.min(256) {
                return Err(SerializeError::DepthLimit.into());
            }
            for value in values {
                inspect(value, document, depth + 1, limits, count)?;
            }
        }
        Object::Dictionary(values) => {
            if depth >= limits.max_depth.min(256) {
                return Err(SerializeError::DepthLimit.into());
            }
            for (key, value) in values {
                *count = count
                    .checked_add(1)
                    .filter(|&n| n <= limits.max_objects)
                    .ok_or(SerializeError::ObjectLimit)?;
                if matches!(
                    key.as_slice(),
                    b"ByteRange" | b"DocMDP" | b"FieldMDP" | b"Perms"
                ) && !matches!(value, Object::Null)
                {
                    return Err(DocumentWriteError::Unsupported(
                        "signature or permissions metadata",
                    ));
                }
                if matches!(key.as_slice(), b"Type" | b"FT") {
                    // Follow only this scalar metadata, with a fixed hop bound.
                    let mut resolved = value;
                    let mut hops = 0;
                    while let Object::Reference(id) = resolved {
                        if hops == 64 {
                            return Err(DocumentWriteError::Invalid(
                                "signature type reference cycle or depth",
                            ));
                        }
                        let object = document
                            .object(*id)
                            .map_err(|_| DocumentWriteError::MissingReference(*id))?;
                        if object.stream.is_some() {
                            return Err(DocumentWriteError::Invalid(
                                "signature type references a stream",
                            ));
                        }
                        resolved = &object.value;
                        hops += 1;
                    }
                    if matches!(resolved, Object::Name(name) if name == b"Sig") {
                        return Err(DocumentWriteError::Unsupported(
                            "signature dictionary or signature form field",
                        ));
                    }
                }
                if key == b"Linearized" && !matches!(value, Object::Null) {
                    return Err(DocumentWriteError::Unsupported("linearized document"));
                }
                inspect(value, document, depth + 1, limits, count)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Return a deterministic PDF with a fresh classic xref and no revision chain.
/// This preserves the effective object graph and raw stream payloads; it makes
/// no claim about signature preservation, garbage collection or rendering.
pub fn rewrite_document(
    document: &Document<'_>,
    limits: DocumentWriteLimits,
) -> Result<Vec<u8>, DocumentWriteError> {
    write_document(document, limits, WriteOptions::default())
}

/// Like `rewrite_document`, but objects not reachable from the trailer's Root
/// and Info are omitted, as MuPDF's `garbage=yes` does (without renumbering).
/// Refusals still consider every object, reachable or not.
pub fn rewrite_reachable(
    document: &Document<'_>,
    limits: DocumentWriteLimits,
) -> Result<Vec<u8>, DocumentWriteError> {
    let options = WriteOptions {
        garbage: true,
        ..WriteOptions::default()
    };
    write_document(document, limits, options)
}

/// Objects reachable from `roots`, by an explicit worklist (no recursion
/// through references), so cycles and long chains are bounded by the
/// inventory. Missing targets were already refused by `inspect`.
fn reachable<'v>(
    document: &Document<'_>,
    roots: impl Iterator<Item = &'v Object>,
) -> BTreeSet<ObjectId> {
    let mut seen = BTreeSet::new();
    let mut values: Vec<&Object> = roots.collect();
    while let Some(value) = values.pop() {
        match value {
            Object::Reference(id) => {
                if seen.insert(*id) {
                    if let Ok(object) = document.object(*id) {
                        values.push(&object.value);
                    }
                }
            }
            Object::Array(items) => values.extend(items),
            Object::Dictionary(items) => values.extend(items.values()),
            _ => {}
        }
    }
    seen
}

/// References in `values` whose targets are not in `written`, found with a
/// worklist so nesting depth cannot exhaust the stack.
fn dangling<'v>(
    values: impl Iterator<Item = &'v Object>,
    written: &BTreeSet<ObjectId>,
) -> BTreeSet<ObjectId> {
    let mut missing = BTreeSet::new();
    let mut values: Vec<&Object> = values.collect();
    while let Some(value) = values.pop() {
        match value {
            Object::Reference(id) if !written.contains(id) => {
                missing.insert(*id);
            }
            Object::Array(items) => values.extend(items),
            Object::Dictionary(items) => values.extend(items.values()),
            _ => {}
        }
    }
    missing
}

fn unfiltered(dictionary: &BTreeMap<Vec<u8>, Object>) -> bool {
    dictionary
        .get(b"Filter".as_slice())
        .is_none_or(|filter| matches!(filter, Object::Null))
}

/// Write a full PDF with a fresh classic xref, without renumbering.
pub fn write_document(
    document: &Document<'_>,
    limits: DocumentWriteLimits,
    options: WriteOptions,
) -> Result<Vec<u8>, DocumentWriteError> {
    if document.object_count() > limits.max_indirect_objects {
        return Err(DocumentWriteError::ObjectLimit);
    }
    let mut count = 0;
    if options.strict {
        strict_checks(document, limits, &mut count)?;
    }
    let mut trailer = BTreeMap::new();
    for (key, value) in document.effective_trailer_entries() {
        if matches!(value, Object::Null) {
            continue;
        }
        match key {
            b"Root" | b"Size" => {}
            b"Info" | b"ID" => {
                if options.strict {
                    inspect(value, document, 0, limits.syntax, &mut count)?;
                }
                trailer.insert(key, value);
            }
            _ => {} // Section-local keys were validated above and are rebuilt.
        }
    }
    let root = Object::Reference(document.root());
    let kept = options
        .garbage
        .then(|| reachable(document, trailer.values().copied().chain([&root])));
    let written = || {
        document
            .objects()
            .filter(|o| kept.as_ref().is_none_or(|k| k.contains(&o.id)))
    };
    let mut nulls = BTreeSet::new();
    if !options.strict {
        let ids: BTreeSet<ObjectId> = written().map(|o| o.id).collect();
        if !ids.contains(&document.root()) {
            return Err(DocumentWriteError::MissingReference(document.root()));
        }
        nulls = dangling(
            written().map(|o| &o.value).chain(trailer.values().copied()),
            &ids,
        );
        trailer.retain(|_, value| !matches!(value, Object::Reference(id) if nulls.contains(id)));
    }
    let largest = written()
        .map(|o| o.id.number())
        .max()
        .ok_or(DocumentWriteError::Invalid("empty document"))?;
    let size = Object::Integer(i64::from(largest) + 1);
    let mut writer = Writer::with_style(limits.syntax, options.style);
    writer.null_references(nulls);
    writer.append(b"%PDF-")?;
    writer.append(document.version().as_bytes())?;
    writer.append(b"\n%\xE2\xE3\xCF\xD3\n")?;
    let mut offsets = Vec::new();
    offsets
        .try_reserve_exact(document.object_count())
        .map_err(|_| DocumentWriteError::AllocationFailed)?;
    for object in written() {
        let offset = writer.len();
        if offset as u128 > 9_999_999_999 {
            return Err(DocumentWriteError::OffsetLimit);
        }
        offsets.push((object.id, offset));
        writer.append(
            format!("{} {} obj\n", object.id.number(), object.id.generation()).as_bytes(),
        )?;
        if let Some(data) = document
            .stream_data(object.id)
            .map_err(|_| DocumentWriteError::Invalid("stream lookup"))?
        {
            let Object::Dictionary(dictionary) = &object.value else {
                return Err(DocumentWriteError::Invalid("stream dictionary"));
            };
            let compress = options.compress_new_streams
                && unfiltered(dictionary)
                && document.owned_stream(object.id).is_some();
            if compress {
                let data = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
                let mut dictionary = dictionary.clone();
                dictionary.remove(b"DecodeParms".as_slice());
                dictionary.insert(b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec()));
                writer.stream_dictionary(&dictionary, data.len())?;
                writer.append(b"\nstream\n")?;
                writer.append(&data)?;
            } else {
                writer.stream_dictionary(dictionary, data.len())?;
                writer.append(b"\nstream\n")?;
                writer.append(data)?;
            }
            writer.append(b"\nendstream")?;
        } else {
            writer.value(&object.value, 0)?;
        }
        writer.append(b"\nendobj\n")?;
    }
    let xref = writer.len();
    if xref as u128 > 9_999_999_999 {
        return Err(DocumentWriteError::OffsetLimit);
    }
    writer.append(b"xref\n0 1\n0000000000 65535 f \n")?;
    // Sparse subsections are bounded by the live inventory, never /Size.
    let mut start = 0;
    while start < offsets.len() {
        let mut end = start + 1;
        while end < offsets.len()
            && offsets[end - 1].0.number().checked_add(1) == Some(offsets[end].0.number())
        {
            end += 1;
        }
        writer.append(format!("{} {}\n", offsets[start].0.number(), end - start).as_bytes())?;
        for &(id, offset) in &offsets[start..end] {
            writer.append(format!("{offset:010} {:05} n \n", id.generation()).as_bytes())?;
        }
        start = end;
    }
    writer.append(b"trailer\n")?;
    if options.style == SerializeStyle::Compact {
        // MuPDF's trailer order.
        let entries = [
            (b"Size".as_slice(), Some(&size)),
            (b"Info", trailer.get(b"Info".as_slice()).copied()),
            (b"Root", Some(&root)),
            (b"ID", trailer.get(b"ID".as_slice()).copied()),
        ];
        writer.dictionary_entries(entries.into_iter().filter_map(|(k, v)| Some((k, v?))))?;
    } else {
        trailer.insert(b"Size".as_slice(), &size);
        trailer.insert(b"Root".as_slice(), &root);
        writer.dictionary_entries(trailer)?;
    }
    writer.append(format!("\nstartxref\n{xref}\n%%EOF\n").as_bytes())?;
    Ok(writer.finish())
}

/// `rewrite_document`'s refusals, over every object and trailer revision.
fn strict_checks(
    document: &Document<'_>,
    limits: DocumentWriteLimits,
    count: &mut usize,
) -> Result<(), DocumentWriteError> {
    // Strings and streams are held decrypted; writing them under the original
    // /Encrypt would corrupt the file. Encrypted saving is M8 work.
    if document.security().is_some() {
        return Err(DocumentWriteError::Unsupported("encrypted document"));
    }
    for object in document.objects() {
        inspect(&object.value, document, 0, limits.syntax, count)?;
    }
    if document
        .trailer_overrides()
        .any(|(key, value)| value.is_some() && !matches!(key, b"Root" | b"Info" | b"ID"))
    {
        return Err(DocumentWriteError::Unsupported(
            "unknown non-null trailer entry",
        ));
    }
    // Unknown historical trailer semantics also cause refusal, even when a
    // newer revision obscures them. Section-local stream keys are recognized
    // only in a trailer that actually declares Type XRef.
    for section in document.trailer_chain() {
        let xref =
            matches!(section.get(b"Type".as_slice()), Some(Object::Name(name)) if name == b"XRef");
        for (key, value) in section {
            if matches!(value, Object::Null) {
                continue;
            }
            if matches!(
                key.as_slice(),
                b"Root" | b"Size" | b"Prev" | b"Info" | b"ID"
            ) {
                continue;
            }
            if xref
                && matches!(
                    key.as_slice(),
                    b"Type" | b"W" | b"Index" | b"Length" | b"Filter" | b"DecodeParms"
                )
            {
                continue;
            }
            return Err(DocumentWriteError::Unsupported(
                "unknown non-null trailer entry",
            ));
        }
    }
    Ok(())
}
