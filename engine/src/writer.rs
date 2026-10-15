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
//! only Root, Size, and the newest trailer's Info and ID in the trailer, and
//! dangling references as null. An encrypted document is refused unless the
//! caller picks `Encryption::Plaintext`, `Keep` or `Aes256` (see encrypt.rs).
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::document::Document;
use crate::encrypt::{Drbg, OutputCipher, file_id};
use crate::object::{Dictionary, Object, ObjectId};
use crate::security::{Security, standard_permissions};
use crate::serialize::{SerializeError, SerializeStyle, WriteLimits, Writer};

/// What a write does about encryption. Encrypted output needs `entropy`.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Encryption<'a> {
    /// Fail with `Unsupported("encrypted document")` on an encrypted
    /// document (one opened with a security handler, or with an /Encrypt
    /// trailer entry set) in a non-strict write.
    Refuse,
    /// Write the decrypted objects without /Encrypt (MuPDF `encrypt=none`).
    Plaintext,
    /// Re-encrypt with the document's own handler: its /Encrypt dictionary
    /// (algorithm, passwords, permissions) unchanged, written directly in
    /// the trailer, and the /ID[0] its R2-R4 key depends on. A document
    /// without a handler is written in plaintext (MuPDF `encrypt=keep`).
    /// A strict write refuses a document with a handler whatever this is.
    Keep,
    /// A new standard security handler V5 R6 (AES-256, MuPDF
    /// `encrypt=aes-256`). Passwords are bytes (UTF-8, truncated to 127);
    /// an empty owner password becomes the user password. `permissions` are
    /// the granted /P bits; /P is `security::standard_permissions` of them.
    /// A strict write refuses a document with a handler whatever this is.
    Aes256 {
        user_password: &'a [u8],
        owner_password: &'a [u8],
        permissions: u32,
    },
}
/// Passwords are never printed.
impl fmt::Debug for Encryption<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refuse => f.write_str("Refuse"),
            Self::Plaintext => f.write_str("Plaintext"),
            Self::Keep => f.write_str("Keep"),
            Self::Aes256 { permissions, .. } => f
                .debug_struct("Aes256")
                .field("permissions", permissions)
                .finish_non_exhaustive(),
        }
    }
}

/// 32 bytes from a cryptographically secure source (in the app, the
/// renderer's `crypto.getRandomValues`), expanded by HMAC_DRBG; they also
/// serve as its nonce. The same entropy, document and options give the same
/// output bytes, including the AES-256 file key, so every save needs fresh
/// entropy.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Entropy(pub [u8; 32]);
impl fmt::Debug for Entropy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Entropy(..)")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriteOptions<'a> {
    /// Write only objects reachable from the trailer's Root, Info and ID.
    pub garbage: bool,
    /// Flate-compress streams created or replaced since open that have no
    /// /Filter. Original streams keep their bytes.
    pub compress_new_streams: bool,
    /// Refuse what `rewrite_document` refuses, encrypted documents included.
    /// Otherwise only a missing catalog, limits and `encryption` fail.
    pub strict: bool,
    pub style: SerializeStyle,
    pub encryption: Encryption<'a>,
    /// Randomness for encrypted output: file key, salts, IVs and /ID[1].
    /// When present, a written /ID also gets a fresh second element (or, if
    /// encrypting without one, a new /ID), as MuPDF's full save does; when
    /// absent, /ID is copied and encrypted output is refused.
    pub entropy: Option<Entropy>,
}
impl WriteOptions<'_> {
    /// The app's save: MuPDF `incremental=no,garbage=yes`, which does not
    /// compress streams.
    pub const APP: Self = Self {
        garbage: true,
        compress_new_streams: false,
        strict: false,
        style: SerializeStyle::Compact,
        encryption: Encryption::Refuse,
        entropy: None,
    };
}
/// `rewrite_document`'s options.
impl Default for WriteOptions<'_> {
    fn default() -> Self {
        Self {
            garbage: false,
            compress_new_streams: false,
            strict: true,
            style: SerializeStyle::Canonical,
            encryption: Encryption::Refuse,
            entropy: None,
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

fn unfiltered(dictionary: &Dictionary) -> bool {
    dictionary
        .get(b"Filter".as_slice())
        .is_none_or(|filter| matches!(filter, Object::Null))
}

/// Write a full PDF with a fresh classic xref, without renumbering.
pub fn write_document(
    document: &Document<'_>,
    limits: DocumentWriteLimits,
    options: WriteOptions<'_>,
) -> Result<Vec<u8>, DocumentWriteError> {
    if document.object_count() > limits.max_indirect_objects {
        return Err(DocumentWriteError::ObjectLimit);
    }
    // Owned trailer values; declared first so they outlive `trailer`.
    let new_id: Object;
    let new_encrypt: Object;
    let mut count = 0;
    let mut trailer = BTreeMap::new();
    if options.strict {
        strict_checks(document, limits, &mut count)?;
        for (key, value) in document.effective_trailer_entries() {
            if matches!(value, Object::Null) {
                continue;
            }
            match key {
                b"Root" | b"Size" => {}
                b"Info" | b"ID" => {
                    inspect(value, document, 0, limits.syntax, &mut count)?;
                    trailer.insert(key, value);
                }
                _ => {} // Section-local keys were validated above and are rebuilt.
            }
        }
    } else {
        let encrypted =
            document.security().is_some() || document.trailer_entry(b"Encrypt").is_some();
        if encrypted && options.encryption == Encryption::Refuse {
            return Err(DocumentWriteError::Unsupported("encrypted document"));
        }
        for key in [b"Info".as_slice(), b"ID"] {
            if let Some(value) = document.trailer_entry(key) {
                trailer.insert(key, value);
            }
        }
    }
    let mut drbg = options.entropy.map(|entropy| Drbg::new(&entropy.0));
    let security = output_security(document, options, drbg.as_mut())?;
    if let Some(drbg) = drbg.as_mut() {
        let first = match &security {
            // The R2-R4 file key is derived from the authenticated /ID[0].
            Some(security) if security.revision <= 4 => Some(security.first_id().to_vec()),
            _ => match trailer.get(b"ID".as_slice()) {
                Some(Object::Array(items)) => match items.first() {
                    Some(Object::String(first)) => Some(first.clone()),
                    _ => None,
                },
                _ => None,
            },
        };
        if security.is_some() || trailer.contains_key(b"ID".as_slice()) {
            new_id = file_id(first, drbg);
            trailer.insert(b"ID", &new_id);
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
    if !written().any(|o| o.id == document.root()) {
        return Err(DocumentWriteError::MissingReference(document.root()));
    }
    let mut cipher = None;
    if let Some(security) = security {
        // Direct, so a source object that held /Encrypt is encrypted like
        // any other.
        new_encrypt = Object::Dictionary(security.dictionary().clone());
        trailer.insert(b"Encrypt", &new_encrypt);
        let drbg = drbg.take().ok_or(DocumentWriteError::Invalid(NO_ENTROPY))?;
        cipher = Some(OutputCipher::new(security, drbg));
    }
    let mut nulls = BTreeSet::new();
    if !options.strict {
        let ids: BTreeSet<ObjectId> = written().map(|o| o.id).collect();
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
        let mut crypt = cipher.as_mut().filter(|c| !c.exempt(object));
        let value = match crypt.as_mut() {
            Some(crypt) => {
                Cow::Owned(crypt.strings(&object.value, object.id, limits.syntax.max_depth)?)
            }
            None => Cow::Borrowed(&object.value),
        };
        if let Some(data) = document
            .stream_data(object.id)
            .map_err(|_| DocumentWriteError::Invalid("stream lookup"))?
        {
            let Object::Dictionary(dictionary) = value.as_ref() else {
                return Err(DocumentWriteError::Invalid("stream dictionary"));
            };
            let compress = options.compress_new_streams
                && unfiltered(dictionary)
                && document.owned_stream(object.id).is_some();
            let (dictionary, data) = if compress {
                let data = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
                let mut dictionary = dictionary.clone();
                dictionary.remove(b"DecodeParms".as_slice());
                dictionary.insert(b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec()));
                (Cow::Owned(dictionary), Cow::Owned(data))
            } else {
                (Cow::Borrowed(dictionary), Cow::Borrowed(data))
            };
            let data = match crypt.as_mut() {
                Some(crypt) => {
                    let room = limits.syntax.max_output_bytes.saturating_sub(writer.len());
                    Cow::Owned(crypt.stream(&dictionary, object.id, &data, room)?)
                }
                None => data,
            };
            writer.stream_dictionary(&dictionary, data.len())?;
            writer.append(b"\nstream\n")?;
            writer.append(&data)?;
            writer.append(b"\nendstream")?;
        } else {
            writer.value(&value, 0)?;
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
            (b"Encrypt", trailer.get(b"Encrypt".as_slice()).copied()),
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

const NO_ENTROPY: &str = "encrypted output needs entropy";

/// The handler to encrypt the output with, if any. A new handler's file key
/// comes from its own DRBG; its salts and /Perms filler from `drbg`.
fn output_security<'d>(
    document: &'d Document<'_>,
    options: WriteOptions<'_>,
    drbg: Option<&mut Drbg>,
) -> Result<Option<Cow<'d, Security>>, DocumentWriteError> {
    let security = match options.encryption {
        Encryption::Refuse | Encryption::Plaintext => return Ok(None),
        Encryption::Keep => match document.security() {
            Some(security) => Cow::Borrowed(security),
            None if document.trailer_entry(b"Encrypt").is_some() => {
                return Err(DocumentWriteError::Unsupported(
                    "/Encrypt without an authenticated security handler",
                ));
            }
            None => return Ok(None),
        },
        Encryption::Aes256 {
            user_password,
            owner_password,
            permissions,
        } => {
            let (Some(entropy), Some(drbg)) = (options.entropy, drbg) else {
                return Err(DocumentWriteError::Invalid(NO_ENTROPY));
            };
            let mut random = [0; 68];
            random[..32].copy_from_slice(&Drbg::secret(&entropy.0).bytes::<32>());
            drbg.fill(&mut random[32..]);
            Cow::Owned(Security::create_r6(
                user_password,
                owner_password,
                standard_permissions(permissions),
                &random,
            ))
        }
    };
    Ok(Some(security))
}

/// `rewrite_document`'s refusals, over every object and trailer revision.
fn strict_checks(
    document: &Document<'_>,
    limits: DocumentWriteLimits,
    count: &mut usize,
) -> Result<(), DocumentWriteError> {
    // A strict rewrite refuses a document with a handler whatever
    // `encryption` says; encrypted output is for non-strict writes.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentLimits;

    #[test]
    fn public_values_never_reveal_the_file_key() {
        // With reused entropy, a plaintext save's /ID[1] and a Keep save's
        // IVs come from the public DRBG; an AES-256 file key from its own.
        let entropy = Entropy([42; 32]);
        let options = |encryption| WriteOptions {
            encryption,
            entropy: Some(entropy),
            ..WriteOptions::APP
        };
        let source = std::fs::read("../fixtures/encrypted.pdf").unwrap();
        let open = |bytes: &[u8], password: &[u8]| {
            Document::open_owned_with_password(bytes.to_vec(), DocumentLimits::default(), password)
                .unwrap()
        };
        let doc = open(&source, b"pdf4all-user");
        let write = |encryption| {
            write_document(&doc, DocumentWriteLimits::default(), options(encryption)).unwrap()
        };
        let aes = write(Encryption::Aes256 {
            user_password: b"u",
            owner_password: b"o",
            permissions: 0,
        });
        let key = open(&aes, b"u").security().unwrap().file_key().to_vec();
        assert_eq!(key, Drbg::secret(&entropy.0).bytes::<32>());
        let mut public = vec![0; 4096];
        Drbg::new(&entropy.0).fill(&mut public);
        assert!(!public.windows(8).any(|w| key.windows(8).any(|k| k == w)));
        let plain = open(&write(Encryption::Plaintext), b"");
        let Some(Object::Array(id)) = plain.trailer_entry(b"ID") else {
            panic!("/ID");
        };
        assert_eq!(id.get(1), Some(&Object::String(public[..16].to_vec())));
    }
}
