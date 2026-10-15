// SPDX-License-Identifier: AGPL-3.0-or-later
//! Strict classic-xref PDF documents, including incremental revisions, with
//! in-memory edits and exact per-object snapshots for an undo journal.
//! Classic/stream xrefs and bounded compressed object streams are supported.
//! Standard-handler encryption is decrypted at open (see security.rs); no
//! repair or hybrid xrefs. The input is borrowed or owned (shared by clones);
//! unencrypted stream bytes are ranges of it. Bootstrap metadata must be
//! direct, and only its verified own entry may be supplied when absent from a
//! sparse xref Index.
//! `trailer()` retains the newest trailer unchanged; `root()` inherits an omitted
//! or null Root from the nearest older trailer. Free entries remain tombstones.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::{Deref, Range};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::object::{Dictionary, Object, ObjectId};
use crate::parser::{ParseError, ParseErrorKind, ParseLimits, parse_one};
use crate::security::{
    Authentication, EncryptionInfo, Security, SecurityError, is_signature_dictionary,
};
use crate::stream::StreamLimits;
use crate::stream_object::{StreamObjectError, decode_document_stream};

#[derive(Clone, Copy, Debug)]
pub struct DocumentLimits {
    pub max_input_bytes: usize,
    /// Total entries scanned across all revisions, including superseded/free entries.
    pub max_xref_entries: usize,
    pub max_revisions: usize,
    /// Cumulative structural-stream stage output, including xref and object streams.
    pub max_metadata_decoded_bytes: usize,
    pub max_object_streams: usize,
    /// Total header members scanned, including members superseded by later revisions.
    pub max_compressed_members: usize,
    /// Across the trailer and all live values, including dictionary keys.
    pub max_objects: usize,
    pub max_decoded_bytes: usize,
    /// All bytes processed outside borrowed stream payloads, including trivia.
    pub max_syntax_bytes: usize,
    pub parse: ParseLimits,
}

impl Default for DocumentLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 128 * 1024 * 1024,
            max_xref_entries: 100_000,
            max_revisions: 64,
            max_metadata_decoded_bytes: 64 * 1024 * 1024,
            max_object_streams: 1024,
            max_compressed_members: 100_000,
            max_objects: 1_000_000,
            max_decoded_bytes: 32 * 1024 * 1024,
            max_syntax_bytes: 64 * 1024 * 1024,
            parse: ParseLimits::default(),
        }
    }
}

#[derive(Debug)]
pub enum DocumentErrorKind {
    Invalid(&'static str),
    Unsupported(&'static str),
    LimitExceeded(&'static str),
    MissingObject(ObjectId),
    /// Encrypted, and neither the given nor the empty password authenticates.
    PasswordRequired,
    Parse(ParseError),
    MetadataDecode(crate::stream::StreamError),
    CompressedParse {
        container: ObjectId,
        member: ObjectId,
        source: ParseError,
    },
}

#[derive(Debug)]
pub struct DocumentError {
    pub offset: usize,
    pub kind: DocumentErrorKind,
}

impl fmt::Display for DocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PDF document error at byte {}: ", self.offset)?;
        match &self.kind {
            DocumentErrorKind::Invalid(reason) => write!(f, "invalid {reason}"),
            DocumentErrorKind::Unsupported(reason) => write!(f, "unsupported {reason}"),
            DocumentErrorKind::LimitExceeded(reason) => write!(f, "{reason} limit exceeded"),
            DocumentErrorKind::MissingObject(id) => {
                write!(f, "missing object {} {}", id.number(), id.generation())
            }
            DocumentErrorKind::PasswordRequired => write!(f, "password required"),
            DocumentErrorKind::Parse(error) => write!(f, "{error}"),
            DocumentErrorKind::MetadataDecode(error) => {
                write!(f, "structural stream metadata: {error}")
            }
            DocumentErrorKind::CompressedParse {
                container,
                member,
                source,
            } => write!(
                f,
                "compressed object {} in container {} at decoded offset {}: {source}",
                member.number(),
                container.number(),
                source.offset
            ),
        }
    }
}

impl std::error::Error for DocumentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            DocumentErrorKind::Parse(error) => Some(error),
            DocumentErrorKind::MetadataDecode(error) => Some(error),
            DocumentErrorKind::CompressedParse { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn error(offset: usize, kind: DocumentErrorKind) -> DocumentError {
    DocumentError { offset, kind }
}
fn invalid(offset: usize, reason: &'static str) -> DocumentError {
    error(offset, DocumentErrorKind::Invalid(reason))
}
fn unsupported(offset: usize, reason: &'static str) -> DocumentError {
    error(offset, DocumentErrorKind::Unsupported(reason))
}
fn limit(offset: usize, reason: &'static str) -> DocumentError {
    error(offset, DocumentErrorKind::LimitExceeded(reason))
}

#[derive(Clone, Debug)]
pub struct IndirectObject {
    pub id: ObjectId,
    pub value: Object,
    pub stream: Option<Range<usize>>,
}

/// Borrowed or shared input bytes; cloning a document never copies them.
#[derive(Clone, Debug)]
enum Input<'a> {
    Borrowed(&'a [u8]),
    Shared(Arc<Vec<u8>>),
}
impl Deref for Input<'_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Borrowed(bytes) => bytes,
            Self::Shared(bytes) => bytes,
        }
    }
}

/// Distinguishes opened documents, so a snapshot (whose stream is a range of
/// one input) is never restored into another. Clones keep their origin.
static ORIGINS: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct Document<'a> {
    input: Input<'a>,
    origin: u64,
    /// Owned stream bytes; they shadow `decrypted_streams` and the input range.
    replaced_streams: BTreeMap<ObjectId, Arc<Vec<u8>>>,
    /// Display-only objects (generated appearances); never in `objects()`.
    synthetic: BTreeMap<ObjectId, IndirectObject>,
    decrypted_streams: BTreeMap<ObjectId, Arc<Vec<u8>>>,
    security: Option<Security>,
    trailer: Dictionary,
    /// Effective trailer edits; `None` removes the key from every revision.
    trailer_overrides: BTreeMap<Vec<u8>, Option<Object>>,
    objects: BTreeMap<ObjectId, IndirectObject>,
    root: ObjectId,
    historical_trailers: Vec<Dictionary>,
}

/// Everything `Document` holds for one object number and generation, so
/// `restore` puts it back exactly: the value, the stream's input range, owned
/// (replaced) bytes, and decrypted bytes. Owned bytes are filtered exactly
/// when the dictionary names a /Filter. A state belongs to the document it
/// was taken from and that document's clones.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectState {
    origin: u64,
    value: Object,
    stream: Option<Range<usize>>,
    replaced: Option<Arc<Vec<u8>>>,
    decrypted: Option<Arc<Vec<u8>>>,
}

/// The trailer edits and catalog of a `Document`, for `restore_trailer`.
#[derive(Clone, Debug, PartialEq)]
pub struct TrailerState {
    origin: u64,
    overrides: BTreeMap<Vec<u8>, Option<Object>>,
    root: ObjectId,
}

static NULL: Object = Object::Null;

#[derive(Clone, Copy)]
struct Entry {
    number: u32,
    generation: u16,
    offset: usize,
    live: bool,
    compressed: Option<CompressedEntry>,
}

#[derive(Clone, Copy)]
struct CompressedEntry {
    container: u32,
    index: usize,
}

struct RawObject {
    id: ObjectId,
    value: Object,
    region: Range<usize>,
    suffix: usize,
}

struct Budget {
    limits: DocumentLimits,
    objects: usize,
    bytes: usize,
    syntax: usize,
    xref_entries: usize,
    metadata_bytes: usize,
    compressed_members: usize,
}

impl Budget {
    fn parse(
        &mut self,
        input: &[u8],
        offset: usize,
        charge_syntax: bool,
    ) -> Result<(Object, usize), DocumentError> {
        let available = self.limits.max_syntax_bytes.saturating_sub(self.syntax);
        let bounded = if charge_syntax {
            &input[..input.len().min(available)]
        } else {
            input
        };
        let mut limits = self.limits.parse;
        limits.max_objects = limits
            .max_objects
            .min(self.limits.max_objects.saturating_sub(self.objects));
        limits.max_decoded_bytes = limits
            .max_decoded_bytes
            .min(self.limits.max_decoded_bytes.saturating_sub(self.bytes));
        let parsed = parse_one(bounded, limits).map_err(|mut source| {
            if bounded.len() < input.len() {
                return limit(offset + bounded.len(), "syntax bytes");
            }
            source.offset += offset;
            if let ParseErrorKind::Lex(lex) = &mut source.kind {
                lex.offset += offset;
            }
            error(source.offset, DocumentErrorKind::Parse(source))
        })?;
        if charge_syntax {
            self.syntax(parsed.span.end, offset)?;
        }
        let (objects, bytes) =
            object_cost(&parsed.object).ok_or_else(|| limit(offset, "object accounting"))?;
        self.objects = self
            .objects
            .checked_add(objects)
            .ok_or_else(|| limit(offset, "objects"))?;
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| limit(offset, "decoded bytes"))?;
        Ok((parsed.object, parsed.span.end))
    }

    fn syntax(&mut self, bytes: usize, offset: usize) -> Result<(), DocumentError> {
        self.syntax = self
            .syntax
            .checked_add(bytes)
            .filter(|&n| n <= self.limits.max_syntax_bytes)
            .ok_or_else(|| limit(offset, "syntax bytes"))?;
        Ok(())
    }
}

fn object_cost(value: &Object) -> Option<(usize, usize)> {
    match value {
        Object::Name(bytes) | Object::String(bytes) => Some((1, bytes.len())),
        Object::Array(values) => values.iter().try_fold((1_usize, 0_usize), |(n, b), value| {
            let (vn, vb) = object_cost(value)?;
            Some((n.checked_add(vn)?, b.checked_add(vb)?))
        }),
        Object::Dictionary(values) => {
            values
                .iter()
                .try_fold((1_usize, 0_usize), |(n, b), (key, value)| {
                    let (vn, vb) = object_cost(value)?;
                    Some((
                        n.checked_add(1)?.checked_add(vn)?,
                        b.checked_add(key.len())?.checked_add(vb)?,
                    ))
                })
        }
        _ => Some((1, 0)),
    }
}

impl Document<'static> {
    /// Open a document that owns its input, for a long-lived session.
    pub fn open_owned(input: Vec<u8>, limits: DocumentLimits) -> Result<Self, DocumentError> {
        Self::open_input(Input::Shared(Arc::new(input)), limits, b"")
    }
    pub fn open_owned_with_password(
        input: Vec<u8>,
        limits: DocumentLimits,
        password: &[u8],
    ) -> Result<Self, DocumentError> {
        Self::open_input(Input::Shared(Arc::new(input)), limits, password)
    }
}

impl<'a> Document<'a> {
    pub fn open(input: &'a [u8], limits: DocumentLimits) -> Result<Self, DocumentError> {
        Self::open_with_password(input, limits, b"")
    }
    /// The password is tried as owner, then user; the empty password is always
    /// accepted when it authenticates.
    pub fn open_with_password(
        input: &'a [u8],
        limits: DocumentLimits,
        password: &[u8],
    ) -> Result<Self, DocumentError> {
        Self::open_input(Input::Borrowed(input), limits, password)
    }
    /// The /Encrypt facts of `input` without authenticating, for a document
    /// that still needs its password; `None` when it is not encrypted. The
    /// whole document is parsed. Once the dictionary is found, later errors
    /// (an unsupported handler, damage) still report it.
    pub fn read_encryption(
        input: &'a [u8],
        limits: DocumentLimits,
    ) -> Result<Option<EncryptionInfo>, DocumentError> {
        let mut info = None;
        match Self::open_reporting(Input::Borrowed(input), limits, b"", &mut info) {
            Err(error) if info.is_none() => Err(error),
            _ => Ok(info),
        }
    }
    fn open_input(
        source: Input<'a>,
        limits: DocumentLimits,
        password: &[u8],
    ) -> Result<Self, DocumentError> {
        Self::open_reporting(source, limits, password, &mut None)
    }
    fn open_reporting(
        source: Input<'a>,
        limits: DocumentLimits,
        password: &[u8],
        info: &mut Option<EncryptionInfo>,
    ) -> Result<Self, DocumentError> {
        let input: &[u8] = &source;
        if input.len() > limits.max_input_bytes {
            return Err(limit(0, "input bytes"));
        }
        if input.len() < 9
            || &input[..5] != b"%PDF-"
            || !matches!(
                &input[5..8],
                b"1.0" | b"1.1" | b"1.2" | b"1.3" | b"1.4" | b"1.5" | b"1.6" | b"1.7" | b"2.0"
            )
            || !matches!(input[8], b'\r' | b'\n')
        {
            return Err(invalid(0, "PDF header (must begin at byte zero)"));
        }
        let (latest_offset, latest_footer) = footer(input)?;
        let mut budget = Budget {
            limits,
            objects: 0,
            bytes: 0,
            syntax: 0,
            xref_entries: 0,
            metadata_bytes: 0,
            compressed_members: 0,
        };
        let mut sections = Vec::new();
        let mut visited = BTreeSet::new();
        let mut index = BTreeMap::new();
        let mut all_offsets = BTreeSet::new();
        let mut next = Some(latest_offset);
        let mut root = None;
        while let Some(offset) = next {
            if sections.len() >= limits.max_revisions {
                return Err(limit(offset, "xref revisions"));
            }
            if offset < 9 || offset >= input.len() || !visited.insert(offset) {
                return Err(invalid(offset, "xref revision offset or cycle"));
            }
            let section = if input[offset].is_ascii_digit() {
                xref_stream_section(input, offset, &mut budget)?
            } else {
                classic_section(input, offset, &mut budget)?
            };
            if sections.is_empty() && section.footer != latest_footer {
                return Err(invalid(section.footer, "latest xref footer"));
            }
            if root.is_none() {
                match section.trailer.get(b"Root".as_slice()) {
                    None | Some(Object::Null) => {}
                    Some(Object::Reference(id)) => root = Some(*id),
                    _ => return Err(invalid(offset, "trailer /Root reference")),
                }
            }
            for entry in section.entries.values() {
                if entry.live && entry.compressed.is_none() {
                    all_offsets.insert(entry.offset);
                }
                index.entry(entry.number).or_insert(*entry);
            }
            next = match section.trailer.get(b"Prev".as_slice()) {
                None | Some(Object::Null) => None,
                Some(Object::Integer(previous)) => {
                    let previous = usize::try_from(*previous)
                        .map_err(|_| invalid(offset, "nonnegative /Prev offset"))?;
                    if previous < 9 || previous >= offset {
                        return Err(invalid(offset, "/Prev must point strictly backwards"));
                    }
                    Some(previous)
                }
                _ => return Err(invalid(offset, "direct integer /Prev")),
            };
            sections.push(section);
        }
        let root = root.ok_or_else(|| invalid(latest_offset, "trailer /Root reference"))?;
        let newest_size = trailer_size(&sections[0].trailer, latest_offset)?;
        if index
            .last_key_value()
            .is_none_or(|(&number, _)| u64::from(number) >= newest_size)
        {
            return Err(invalid(latest_offset, "newest trailer /Size range"));
        }
        // Entry 0 must be free. Its generation is not checked: xref streams with
        // a 1-byte field (MuPDF's object-stream output) cannot store 65535.
        if index.get(&0).is_none_or(|entry| entry.live) {
            return Err(invalid(latest_offset, "object-zero free xref entry"));
        }
        let mut physical: Vec<_> = sections
            .iter()
            .map(|section| section.region.clone())
            .collect();
        physical.sort_unstable_by_key(|region| region.start);
        for pair in physical.windows(2) {
            if pair[0].end > pair[1].start {
                return Err(invalid(pair[1].start, "overlapping xref revision sections"));
            }
        }
        let bootstraps: BTreeMap<usize, ObjectId> = sections
            .iter()
            .filter_map(|section| {
                section
                    .bootstrap
                    .as_ref()
                    .map(|object| (section.region.start, object.id))
            })
            .collect();
        let section_starts: BTreeSet<_> = physical.iter().map(|region| region.start).collect();
        for &offset in &all_offsets {
            let insertion = physical.partition_point(|region| region.start <= offset);
            if insertion > 0
                && offset < physical[insertion - 1].end
                && !bootstraps.contains_key(&offset)
            {
                return Err(invalid(offset, "object offset inside xref metadata"));
            }
        }
        let mut boundaries = all_offsets.clone();
        boundaries.insert(input.len());
        for region in &physical {
            boundaries.insert(region.start);
            boundaries.insert(region.end);
        }
        let first = *boundaries
            .first()
            .ok_or_else(|| invalid(0, "empty document"))?;
        trivia_gap(input, 8..first)?;
        budget.syntax(first, 0)?;
        for region in &physical {
            let next_boundary = boundaries
                .range((
                    std::ops::Bound::Excluded(region.end),
                    std::ops::Bound::Unbounded,
                ))
                .next()
                .copied()
                .unwrap_or(input.len());
            // A new object or section may begin exactly at the preceding EOF.
            if !all_offsets.contains(&region.end) && !section_starts.contains(&region.end) {
                trivia_gap(input, region.end..next_boundary)?;
                budget.syntax(next_boundary - region.end, region.end)?;
            }
        }
        let groups = compressed_groups(&index, limits)?;
        let mut live: Vec<_> = index
            .into_values()
            .filter(|entry| entry.live && entry.compressed.is_none())
            .collect();
        live.sort_unstable_by_key(|entry| entry.offset);
        for pair in live.windows(2) {
            if pair[0].offset == pair[1].offset {
                return Err(invalid(pair[1].offset, "aliased xref object offset"));
            }
        }
        let mut raw = BTreeMap::new();
        for entry in &live {
            if let Some(id) = bootstraps.get(&entry.offset) {
                if id.number() != entry.number || id.generation() != entry.generation {
                    return Err(invalid(entry.offset, "xref bootstrap ID alias"));
                }
                continue;
            }
            let end = boundaries
                .range((
                    std::ops::Bound::Excluded(entry.offset),
                    std::ops::Bound::Unbounded,
                ))
                .next()
                .copied()
                .ok_or_else(|| invalid(entry.offset, "object physical boundary"))?;
            let mut body = Cursor::new(input, entry.offset, end);
            let number = decimal(body.raw_word()?, entry.offset)?;
            let generation_offset = body.pos;
            let generation = decimal(body.word()?, generation_offset)?;
            if number != u64::from(entry.number) || generation != u64::from(entry.generation) {
                return Err(invalid(
                    entry.offset,
                    "indirect-object ID does not match xref",
                ));
            }
            if body.word()? != b"obj" {
                return Err(invalid(body.pos, "indirect-object wrapper"));
            }
            body.trivia();
            let value_start = body.pos;
            budget.syntax(value_start - entry.offset, entry.offset)?;
            let (value, consumed) = budget.parse(&input[value_start..end], value_start, true)?;
            body.pos += consumed;
            let id = ObjectId::new(entry.number, entry.generation)
                .map_err(|_| invalid(entry.offset, "object ID"))?;
            raw.insert(
                id,
                RawObject {
                    id,
                    value,
                    region: entry.offset..end,
                    suffix: body.pos,
                },
            );
        }
        let security = authenticate(&sections, &mut raw, password, latest_offset, info)?;
        let (members, mut streams) =
            load_compressed_members(input, &raw, &groups, &mut budget, security.as_ref())?;
        for object in raw.values() {
            let stream = match streams.get(&object.id) {
                Some(stream) => stream.clone(),
                None => finish_object(input, object, &raw, &members)?,
            };
            let stream_bytes = stream.as_ref().map_or(0, |range| range.len());
            budget.syntax(
                object.region.end - object.suffix - stream_bytes,
                object.suffix,
            )?;
            streams.insert(object.id, stream);
        }
        let mut decrypted_streams = BTreeMap::new();
        if let Some(security) = &security {
            for (id, range) in &streams {
                let (Some(range), Some(object)) = (range, raw.get(id)) else {
                    continue;
                };
                let kind = match &object.value {
                    Object::Dictionary(d) => d.get(b"Type".as_slice()),
                    _ => None,
                };
                let is = |name: &[u8]| matches!(kind, Some(Object::Name(n)) if n == name);
                // Cross-reference streams are never encrypted.
                if is(b"XRef") {
                    continue;
                }
                let method = security.stream_method(is(b"Metadata"));
                let bytes = security.decrypt(method, *id, &input[range.clone()]);
                decrypted_streams.insert(*id, Arc::new(bytes));
            }
        }
        let mut objects = raw
            .into_iter()
            .map(|(id, raw)| {
                let object = IndirectObject {
                    id,
                    value: raw.value,
                    stream: streams.remove(&id).flatten(),
                };
                (id, object)
            })
            .collect::<BTreeMap<_, _>>();
        for (id, value) in members {
            objects.insert(
                id,
                IndirectObject {
                    id,
                    value,
                    stream: None,
                },
            );
        }
        let selected: BTreeMap<_, _> = live
            .iter()
            .map(|entry| ((entry.number, entry.generation), entry.offset))
            .collect();
        for section in &mut sections {
            if let Some(object) = section.bootstrap.take() {
                if selected.get(&(object.id.number(), object.id.generation()))
                    == Some(&section.region.start)
                {
                    objects.insert(object.id, object);
                }
            }
        }
        if !objects.contains_key(&root) {
            return Err(error(latest_offset, DocumentErrorKind::MissingObject(root)));
        }
        let trailer = sections.remove(0).trailer;
        let historical_trailers = sections
            .into_iter()
            .map(|section| section.trailer)
            .collect();
        Ok(Self {
            input: source,
            origin: ORIGINS.fetch_add(1, Ordering::Relaxed),
            replaced_streams: BTreeMap::new(),
            synthetic: BTreeMap::new(),
            decrypted_streams,
            security,
            trailer,
            trailer_overrides: BTreeMap::new(),
            objects,
            root,
            historical_trailers,
        })
    }
    /// The bytes the document was opened from.
    pub fn input(&self) -> &[u8] {
        &self.input
    }
    /// The original validated ASCII version, such as "1.7" or "2.0".
    pub fn version(&self) -> &str {
        std::str::from_utf8(&self.input[5..8]).expect("validated ASCII PDF header")
    }

    /// Raw trailers in newest-to-oldest order, without copying owned payloads.
    pub fn trailer_chain(&self) -> impl Iterator<Item = &Dictionary> {
        std::iter::once(&self.trailer).chain(self.historical_trailers.iter())
    }

    /// Effective non-null entries, newest first per byte-name key, after the
    /// edits made with `set_trailer_entry`. Section-local Size/Prev and
    /// xref-stream keys remain visible; consumers must handle or exclude them
    /// deliberately rather than treating them as inherited content.
    pub fn effective_trailer_entries(&self) -> impl Iterator<Item = (&[u8], &Object)> {
        let mut seen: BTreeSet<&[u8]> = self.trailer_overrides.keys().map(Vec::as_slice).collect();
        let overrides = self
            .trailer_overrides
            .iter()
            .filter_map(|(key, value)| Some((key.as_slice(), value.as_ref()?)));
        overrides.chain(
            self.trailer_chain()
                .flat_map(|trailer| trailer.iter())
                .filter_map(move |(key, value)| {
                    if matches!(value, Object::Null) || !seen.insert(key.as_slice()) {
                        None
                    } else {
                        Some((key.as_slice(), value))
                    }
                }),
        )
    }

    /// The newest trailer as parsed, without `set_trailer_entry` edits.
    pub fn trailer(&self) -> &Dictionary {
        &self.trailer
    }
    /// The non-null value of a trailer key after `set_trailer_entry` edits.
    /// As in MuPDF, pdf.js and ISO 32000 (7.5.6), the newest trailer is
    /// complete: an Info or ID that only an older revision has is not
    /// inherited. Root is the exception and is inherited as `root()` is.
    /// `effective_trailer_entries` keeps the inheriting view.
    pub fn trailer_entry(&self, key: &[u8]) -> Option<&Object> {
        if let Some(value) = self.trailer_overrides.get(key) {
            return value.as_ref();
        }
        let revisions = if key == b"Root" { usize::MAX } else { 1 };
        self.trailer_chain()
            .take(revisions)
            .filter_map(|trailer| trailer.get(key))
            .find(|value| !value.is_null())
    }
    /// Set (or, with `None` or null, remove) a trailer entry such as Info, ID
    /// or Encrypt. Root must reference an existing non-stream dictionary; the
    /// section-local Size and Prev cannot be set.
    pub fn set_trailer_entry(
        &mut self,
        key: &[u8],
        value: Option<Object>,
    ) -> Result<(), DocumentError> {
        let value = value.filter(|value| !value.is_null());
        match (key, &value) {
            (b"Root", Some(Object::Reference(id))) => {
                let catalog = self
                    .objects
                    .get(id)
                    .ok_or_else(|| error(0, DocumentErrorKind::MissingObject(*id)))?;
                if catalog.stream.is_some() || catalog.value.as_dict().is_none() {
                    return Err(invalid(0, "trailer /Root target is not a dictionary"));
                }
                self.root = *id;
            }
            (b"Root", _) => return Err(invalid(0, "trailer /Root reference")),
            (b"Size" | b"Prev", _) => return Err(invalid(0, "section-local trailer entry")),
            _ => {}
        }
        self.trailer_overrides.insert(key.to_vec(), value);
        Ok(())
    }
    pub fn trailer_state(&self) -> TrailerState {
        TrailerState {
            origin: self.origin,
            overrides: self.trailer_overrides.clone(),
            root: self.root,
        }
    }
    /// Put back a `trailer_state` of this document (or a clone). Unchecked
    /// otherwise, like `restore`.
    pub fn restore_trailer(&mut self, state: TrailerState) -> Result<(), DocumentError> {
        if state.origin != self.origin {
            return Err(invalid(0, "trailer state from another document"));
        }
        self.trailer_overrides = state.overrides;
        self.root = state.root;
        Ok(())
    }
    pub fn root(&self) -> ObjectId {
        self.root
    }
    /// The authenticated standard security handler of an encrypted document.
    pub fn security(&self) -> Option<&Security> {
        self.security.as_ref()
    }
    /// The /Encrypt facts of an encrypted document.
    pub fn encryption(&self) -> Option<EncryptionInfo> {
        self.security.as_ref().map(Security::info)
    }
    /// Which password opened an encrypted document.
    pub fn authentication(&self) -> Option<Authentication> {
        self.security.as_ref().map(Security::authentication)
    }
    pub fn object(&self, id: ObjectId) -> Result<&IndirectObject, DocumentError> {
        self.objects
            .get(&id)
            .or_else(|| self.synthetic.get(&id))
            .ok_or_else(|| error(0, DocumentErrorKind::MissingObject(id)))
    }
    /// Follow references for at most 64 hops. A missing object, a cycle or a
    /// longer chain resolves to null, as in MuPDF (which stops after 10 hops).
    /// A stream resolves to its dictionary.
    pub fn resolve<'s>(&'s self, value: &'s Object) -> &'s Object {
        self.follow(None, value, 0).1
    }
    /// What the object `id` resolves to; null when it is missing.
    pub fn resolve_id(&self, id: ObjectId) -> &Object {
        self.resolve_id_with_holder(id).1
    }
    /// `resolve`, also giving the indirect object whose value is the result,
    /// the one to `put_value` to change it in place. The holder is `None` when
    /// `value` is direct or resolves to null through a missing or cyclic chain.
    pub fn resolve_with_holder<'s>(&'s self, value: &'s Object) -> (Option<ObjectId>, &'s Object) {
        self.follow(None, value, 0)
    }
    /// `resolve_id` with the holder, as `resolve_with_holder` gives it.
    pub fn resolve_id_with_holder(&self, id: ObjectId) -> (Option<ObjectId>, &Object) {
        match self.object(id) {
            Ok(object) => self.follow(Some(id), &object.value, 1),
            Err(_) => (None, &NULL),
        }
    }
    fn follow<'s>(
        &'s self,
        mut holder: Option<ObjectId>,
        mut value: &'s Object,
        mut hops: usize,
    ) -> (Option<ObjectId>, &'s Object) {
        while let Object::Reference(id) = value {
            let (true, Ok(object)) = (hops < 64, self.object(*id)) else {
                return (None, &NULL);
            };
            hops += 1;
            (holder, value) = (Some(*id), &object.value);
        }
        (holder, value)
    }
    /// `dictionary[key]`, resolved; null when absent.
    pub fn dict_get<'s>(&'s self, dictionary: &'s Dictionary, key: &[u8]) -> &'s Object {
        dictionary
            .get(key)
            .map_or(&NULL, |value| self.resolve(value))
    }
    /// The dictionary `value` resolves to (a stream's included), if any.
    pub fn resolve_dict<'s>(&'s self, value: &'s Object) -> Option<&'s Dictionary> {
        self.resolve(value).as_dict()
    }
    /// A stream object's dictionary.
    pub fn stream_dict(&self, id: ObjectId) -> Result<&Dictionary, DocumentError> {
        match self.object(id)? {
            IndirectObject {
                stream: Some(_),
                value: Object::Dictionary(dictionary),
                ..
            } => Ok(dictionary),
            _ => Err(invalid(0, "object is not a stream")),
        }
    }
    /// The stream's bytes with every filter decoded.
    pub fn decoded_stream(
        &self,
        id: ObjectId,
        limits: StreamLimits,
    ) -> Result<Vec<u8>, StreamObjectError> {
        decode_document_stream(self, id, limits)
    }

    /// The complete state of an object, or `None` when it does not exist.
    pub fn snapshot(&self, id: ObjectId) -> Option<ObjectState> {
        let object = self.objects.get(&id)?;
        Some(ObjectState {
            origin: self.origin,
            value: object.value.clone(),
            stream: object.stream.clone(),
            replaced: self.replaced_streams.get(&id).cloned(),
            decrypted: self.decrypted_streams.get(&id).cloned(),
        })
    }
    /// Put back a `snapshot` exactly; `None` removes the object. A state
    /// taken from another document is refused. Otherwise unchecked:
    /// references, generations and the catalog are not validated.
    pub fn restore(
        &mut self,
        id: ObjectId,
        state: Option<ObjectState>,
    ) -> Result<(), DocumentError> {
        let Some(state) = state else {
            self.clear(id);
            return Ok(());
        };
        if state.origin != self.origin {
            return Err(invalid(0, "object state from another document"));
        }
        self.clear(id);
        if let Some(bytes) = state.replaced {
            self.replaced_streams.insert(id, bytes);
        }
        if let Some(bytes) = state.decrypted {
            self.decrypted_streams.insert(id, bytes);
        }
        self.objects.insert(
            id,
            IndirectObject {
                id,
                value: state.value,
                stream: state.stream,
            },
        );
        Ok(())
    }
    fn clear(&mut self, id: ObjectId) {
        self.objects.remove(&id);
        self.replaced_streams.remove(&id);
        self.decrypted_streams.remove(&id);
    }

    /// Create or replace a non-stream object.
    pub fn put_value(&mut self, id: ObjectId, value: Object) -> Result<(), DocumentError> {
        self.check_generation(id)?;
        self.replaced_streams.remove(&id);
        self.decrypted_streams.remove(&id);
        self.objects.insert(
            id,
            IndirectObject {
                id,
                value,
                stream: None,
            },
        );
        Ok(())
    }
    /// Create or replace a stream whose `raw` bytes are encoded as the
    /// dictionary's /Filter says; /Length is set to match.
    pub fn put_stream_raw(
        &mut self,
        id: ObjectId,
        mut dictionary: Dictionary,
        raw: Vec<u8>,
    ) -> Result<(), DocumentError> {
        self.check_generation(id)?;
        set_length(&mut dictionary, raw.len())?;
        self.decrypted_streams.remove(&id);
        self.replaced_streams.insert(id, Arc::new(raw));
        self.objects.insert(
            id,
            IndirectObject {
                id,
                value: Object::Dictionary(dictionary),
                stream: Some(0..0),
            },
        );
        Ok(())
    }
    /// Create or replace a stream with unfiltered bytes: /Filter and
    /// /DecodeParms are removed and /Length is set.
    pub fn put_stream_decoded(
        &mut self,
        id: ObjectId,
        mut dictionary: Dictionary,
        decoded: Vec<u8>,
    ) -> Result<(), DocumentError> {
        dictionary.remove(b"Filter".as_slice());
        dictionary.remove(b"DecodeParms".as_slice());
        self.put_stream_raw(id, dictionary, decoded)
    }
    /// A new stream that keeps `raw` and its /Filter (grafted or DCT data).
    pub fn allocate_stream_raw(
        &mut self,
        dictionary: Dictionary,
        raw: Vec<u8>,
    ) -> Result<ObjectId, DocumentError> {
        let id = self.next_id()?;
        self.put_stream_raw(id, dictionary, raw)?;
        Ok(id)
    }
    /// Replace a stream's dictionary exactly, keeping its bytes.
    pub fn set_stream_dict(
        &mut self,
        id: ObjectId,
        dictionary: Dictionary,
    ) -> Result<(), DocumentError> {
        let object = self
            .objects
            .get_mut(&id)
            .ok_or_else(|| error(0, DocumentErrorKind::MissingObject(id)))?;
        if object.stream.is_none() {
            return Err(invalid(0, "dictionary replacement target is not a stream"));
        }
        object.value = Object::Dictionary(dictionary);
        Ok(())
    }
    /// Remove an object with its stream bytes. References to it are not
    /// checked (a non-strict save writes them as null); the catalog is kept.
    pub fn remove_object(&mut self, id: ObjectId) -> Result<(), DocumentError> {
        if id == self.root {
            return Err(invalid(0, "removal of the document catalog"));
        }
        if !self.objects.contains_key(&id) {
            return Err(error(0, DocumentErrorKind::MissingObject(id)));
        }
        self.clear(id);
        Ok(())
    }
    /// Two generations of one number would give duplicate xref rows.
    fn check_generation(&self, id: ObjectId) -> Result<(), DocumentError> {
        let range = ObjectId::new(id.number(), 0).map_err(|_| invalid(0, "object ID"))?
            ..=ObjectId::new(id.number(), u16::MAX).map_err(|_| invalid(0, "object ID"))?;
        if self
            .objects
            .range(range.clone())
            .any(|(other, _)| *other != id)
            || self.synthetic.range(range).next().is_some()
        {
            return Err(invalid(0, "object number in use by another generation"));
        }
        Ok(())
    }
    /// Owned bytes of a stream created or replaced since open.
    pub(crate) fn owned_stream(&self, id: ObjectId) -> Option<&[u8]> {
        self.replaced_streams.get(&id).map(|bytes| bytes.as_slice())
    }
    /// Keys edited with `set_trailer_entry`; `None` marks a removal.
    pub(crate) fn trailer_overrides(&self) -> impl Iterator<Item = (&[u8], Option<&Object>)> {
        self.trailer_overrides
            .iter()
            .map(|(key, value)| (key.as_slice(), value.as_ref()))
    }

    /// Transaction-local new object, written by the next rewrite. A stream
    /// gets unfiltered `bytes` and a matching /Length.
    pub fn allocate(
        &mut self,
        mut value: Object,
        bytes: Option<Vec<u8>>,
    ) -> Result<ObjectId, DocumentError> {
        let id = self.next_id()?;
        if let (Some(bytes), Object::Dictionary(d)) = (&bytes, &mut value) {
            d.remove(b"Filter".as_slice());
            d.remove(b"DecodeParms".as_slice());
            set_length(d, bytes.len())?;
        }
        self.objects.insert(
            id,
            IndirectObject {
                id,
                value,
                stream: bytes.as_ref().map(|_| 0..0),
            },
        );
        if let Some(bytes) = bytes {
            self.replaced_streams.insert(id, Arc::new(bytes));
        }
        Ok(id)
    }
    /// Transaction-local replacement of a non-stream object's value.
    pub fn replace_value(&mut self, id: ObjectId, value: Object) -> Result<(), DocumentError> {
        let object = self
            .objects
            .get_mut(&id)
            .ok_or_else(|| error(0, DocumentErrorKind::MissingObject(id)))?;
        if object.stream.is_some() {
            return Err(invalid(0, "value replacement target is a stream"));
        }
        object.value = value;
        Ok(())
    }
    /// Transaction-local deletion of a set of objects. Refused, with nothing
    /// removed, if the set holds the catalog or a missing object, or if any
    /// object outside the set or the effective trailer still references one.
    /// References inside the set (cycles) are allowed.
    pub fn delete_objects(&mut self, ids: &BTreeSet<ObjectId>) -> Result<(), DocumentError> {
        for id in ids {
            if *id == self.root {
                return Err(invalid(0, "deletion of the document catalog"));
            }
            if !self.objects.contains_key(id) {
                return Err(error(0, DocumentErrorKind::MissingObject(*id)));
            }
        }
        let outside = self
            .objects
            .values()
            .chain(self.synthetic.values())
            .filter(|o| !ids.contains(&o.id))
            .map(|o| &o.value);
        let trailer = self.effective_trailer_entries().map(|(_, value)| value);
        for value in outside.chain(trailer) {
            if references_any(value, ids) {
                return Err(invalid(0, "deletion would leave a dangling reference"));
            }
        }
        for id in ids {
            self.objects.remove(id);
            self.replaced_streams.remove(id);
            self.decrypted_streams.remove(id);
        }
        Ok(())
    }
    /// Set an object exactly, as replayed from history. `bytes` are raw stream
    /// bytes for the stored dictionary; a stream with `None` keeps its current
    /// raw bytes. `stream: false` makes (or keeps) a non-stream object.
    pub(crate) fn put(
        &mut self,
        id: ObjectId,
        value: Object,
        stream: bool,
        bytes: Option<Vec<u8>>,
    ) -> Result<(), DocumentError> {
        let keeps =
            bytes.is_none() && stream && self.objects.get(&id).is_none_or(|o| o.stream.is_none());
        if keeps || (bytes.is_some() && !stream) {
            return Err(invalid(0, "history object stream state"));
        }
        let range = match bytes {
            Some(bytes) => {
                self.replaced_streams.insert(id, Arc::new(bytes));
                self.decrypted_streams.remove(&id);
                Some(0..0)
            }
            None if stream => self.objects[&id].stream.clone(),
            None => {
                self.replaced_streams.remove(&id);
                self.decrypted_streams.remove(&id);
                None
            }
        };
        self.objects.insert(
            id,
            IndirectObject {
                id,
                value,
                stream: range,
            },
        );
        Ok(())
    }
    /// Unchecked removal, for replaying a validated history delta.
    pub(crate) fn remove(&mut self, id: ObjectId) {
        self.clear(id);
    }
    /// The id `allocate` gives next: one past the largest number, generation 0.
    pub fn next_id(&self) -> Result<ObjectId, DocumentError> {
        let last = self
            .objects
            .keys()
            .chain(self.synthetic.keys())
            .map(|id| id.number())
            .max()
            .unwrap_or(0);
        last.checked_add(1)
            .and_then(|n| ObjectId::new(n, 0).ok())
            .ok_or_else(|| limit(0, "object numbers"))
    }
    /// Add a display-only stream (an unfiltered Form, say) under a fresh object
    /// number. It resolves like any object but is never enumerated or written.
    pub(crate) fn add_synthetic_stream(
        &mut self,
        dictionary: Dictionary,
        bytes: Vec<u8>,
    ) -> Result<ObjectId, DocumentError> {
        self.add_synthetic(Object::Dictionary(dictionary), Some(bytes))
    }
    pub(crate) fn add_synthetic(
        &mut self,
        value: Object,
        bytes: Option<Vec<u8>>,
    ) -> Result<ObjectId, DocumentError> {
        let id = self.next_id()?;
        self.synthetic.insert(
            id,
            IndirectObject {
                id,
                value,
                stream: bytes.as_ref().map(|_| 0..0),
            },
        );
        if let Some(bytes) = bytes {
            self.replaced_streams.insert(id, Arc::new(bytes));
        }
        Ok(id)
    }
    pub fn objects(&self) -> impl Iterator<Item = &IndirectObject> {
        self.objects.values()
    }
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }
    /// Transaction-local replacement; only the session publishes serialized bytes.
    pub(crate) fn replace_decoded_stream(
        &mut self,
        id: ObjectId,
        bytes: Vec<u8>,
    ) -> Result<(), DocumentError> {
        let object = self
            .objects
            .get_mut(&id)
            .ok_or_else(|| error(0, DocumentErrorKind::MissingObject(id)))?;
        if object.stream.is_none() {
            return Err(invalid(0, "replacement target is not a stream"));
        }
        let Object::Dictionary(dictionary) = &mut object.value else {
            return Err(invalid(0, "replacement stream dictionary"));
        };
        dictionary.remove(b"Filter".as_slice());
        dictionary.remove(b"DecodeParms".as_slice());
        set_length(dictionary, bytes.len())?;
        self.replaced_streams.insert(id, Arc::new(bytes));
        Ok(())
    }

    pub fn stream_data(&self, id: ObjectId) -> Result<Option<&[u8]>, DocumentError> {
        if let Some(bytes) = self.replaced_streams.get(&id) {
            return Ok(Some(bytes));
        }
        if let Some(bytes) = self.decrypted_streams.get(&id) {
            return Ok(Some(bytes));
        }
        Ok(self
            .object(id)?
            .stream
            .as_ref()
            .map(|range| &self.input[range.clone()]))
    }
}

struct Section {
    region: Range<usize>,
    footer: usize,
    entries: BTreeMap<u32, Entry>,
    trailer: Dictionary,
    bootstrap: Option<IndirectObject>,
}

fn set_length(dictionary: &mut Dictionary, length: usize) -> Result<(), DocumentError> {
    let length = i64::try_from(length).map_err(|_| invalid(0, "stream length"))?;
    dictionary.insert(b"Length".to_vec(), Object::Integer(length));
    Ok(())
}

/// Whether `value` directly contains a reference to one of `ids`. Object depth
/// is capped at open, so this recursion is bounded; references are not followed.
fn references_any(value: &Object, ids: &BTreeSet<ObjectId>) -> bool {
    match value {
        Object::Reference(id) => ids.contains(id),
        Object::Array(items) => items.iter().any(|v| references_any(v, ids)),
        Object::Dictionary(items) => items.values().any(|v| references_any(v, ids)),
        _ => false,
    }
}

fn trailer_size(trailer: &Dictionary, offset: usize) -> Result<u64, DocumentError> {
    match trailer.get(b"Size".as_slice()) {
        Some(Object::Integer(size)) if *size > 0 && *size <= i64::from(u32::MAX) + 1 => {
            Ok(*size as u64)
        }
        _ => Err(invalid(offset, "trailer /Size")),
    }
}

fn classic_section(
    input: &[u8],
    offset: usize,
    budget: &mut Budget,
) -> Result<Section, DocumentError> {
    let mut cursor = Cursor::new(input, offset, input.len());
    if cursor.raw_word()? != b"xref" {
        return Err(invalid(offset, "xref table marker"));
    }
    let mut entries = BTreeMap::new();
    loop {
        cursor.trivia();
        let subsection_offset = cursor.pos;
        let word = cursor.word()?;
        if word == b"trailer" {
            break;
        }
        let first = decimal(word, subsection_offset)?;
        let count_offset = cursor.pos;
        let count = decimal(cursor.word()?, count_offset)?;
        if count == 0 {
            return Err(invalid(subsection_offset, "empty xref subsection"));
        }
        let end = first
            .checked_add(count)
            .filter(|&number| number <= u64::from(u32::MAX) + 1)
            .ok_or_else(|| invalid(subsection_offset, "xref subsection range"))?;
        let count = usize::try_from(count).map_err(|_| limit(subsection_offset, "xref entries"))?;
        budget.xref_entries = budget
            .xref_entries
            .checked_add(count)
            .filter(|&count| count <= budget.limits.max_xref_entries)
            .ok_or_else(|| limit(subsection_offset, "xref entries"))?;
        cursor.line_end()?;
        for number in first..end {
            let entry_offset = cursor.pos;
            let entry = cursor.xref_entry(number as u32)?;
            if entry.live && (entry.number == 0 || entry.offset < 9 || entry.offset >= offset) {
                return Err(invalid(
                    entry_offset,
                    "live xref entry offset or object number",
                ));
            }
            if entries.insert(number as u32, entry).is_some() {
                return Err(invalid(entry_offset, "duplicate xref object number"));
            }
        }
    }
    cursor.trivia();
    let trailer_start = cursor.pos;
    budget.syntax(trailer_start - offset, offset)?;
    let (value, consumed) = budget.parse(&input[trailer_start..], trailer_start, true)?;
    cursor.pos += consumed;
    let Object::Dictionary(trailer) = value else {
        return Err(invalid(trailer_start, "trailer dictionary"));
    };
    for (key, feature) in [(b"XRefStm".as_slice(), "hybrid xref streams")] {
        if trailer
            .get(key)
            .is_some_and(|value| !matches!(value, Object::Null))
        {
            return Err(unsupported(trailer_start, feature));
        }
    }
    let size = trailer_size(&trailer, trailer_start)?;
    if entries
        .last_key_value()
        .is_some_and(|(&number, _)| u64::from(number) >= size)
    {
        return Err(invalid(trailer_start, "trailer /Size range"));
    }
    let suffix = cursor.pos;
    let footer = section_footer(&mut cursor, offset)?;
    budget.syntax(cursor.pos - suffix, suffix)?;
    Ok(Section {
        region: offset..cursor.pos,
        footer,
        entries,
        trailer,
        bootstrap: None,
    })
}

fn direct_integer(
    value: Option<&Object>,
    offset: usize,
    name: &'static str,
) -> Result<usize, DocumentError> {
    match value {
        Some(Object::Integer(value)) => usize::try_from(*value).map_err(|_| invalid(offset, name)),
        Some(Object::Reference(_)) => {
            Err(unsupported(offset, "indirect structural-stream metadata"))
        }
        _ => Err(invalid(offset, name)),
    }
}

fn xref_stream_section(
    input: &[u8],
    offset: usize,
    budget: &mut Budget,
) -> Result<Section, DocumentError> {
    let mut cursor = Cursor::new(input, offset, input.len());
    let number = u32::try_from(decimal(cursor.raw_word()?, offset)?)
        .map_err(|_| invalid(offset, "xref stream object number"))?;
    let generation_offset = cursor.pos;
    let generation = u16::try_from(decimal(cursor.word()?, generation_offset)?)
        .map_err(|_| invalid(generation_offset, "xref stream generation"))?;
    let id =
        ObjectId::new(number, generation).map_err(|_| invalid(offset, "xref stream object ID"))?;
    if cursor.word()? != b"obj" {
        return Err(invalid(cursor.pos, "xref stream object wrapper"));
    }
    cursor.trivia();
    let dictionary_start = cursor.pos;
    budget.syntax(dictionary_start - offset, offset)?;
    let (value, consumed) = budget.parse(&input[dictionary_start..], dictionary_start, true)?;
    cursor.pos += consumed;
    let Object::Dictionary(trailer) = &value else {
        return Err(invalid(dictionary_start, "xref stream dictionary"));
    };
    if !matches!(trailer.get(b"Type".as_slice()), Some(Object::Name(name)) if name == b"XRef") {
        return Err(invalid(dictionary_start, "xref stream /Type /XRef"));
    }
    for key in [
        b"Length".as_slice(),
        b"W",
        b"Index",
        b"Size",
        b"Filter",
        b"DecodeParms",
        b"Prev",
    ] {
        if matches!(trailer.get(key), Some(Object::Reference(_))) {
            return Err(unsupported(
                dictionary_start,
                "indirect structural-stream metadata",
            ));
        }
    }
    for (key, feature) in [
        (b"XRefStm".as_slice(), "hybrid xref streams"),
        (b"F", "external-file streams"),
    ] {
        if trailer
            .get(key)
            .is_some_and(|value| !matches!(value, Object::Null))
        {
            return Err(unsupported(dictionary_start, feature));
        }
    }
    let size = trailer_size(trailer, dictionary_start)?;
    if u64::from(number) >= size {
        return Err(invalid(
            dictionary_start,
            "xref stream self object outside /Size",
        ));
    }
    let length = direct_integer(
        trailer.get(b"Length".as_slice()),
        dictionary_start,
        "xref stream direct /Length",
    )?;
    let widths = match trailer.get(b"W".as_slice()) {
        Some(Object::Array(widths)) if widths.len() == 3 => widths,
        _ => {
            return Err(invalid(
                dictionary_start,
                "xref stream /W array of three widths",
            ));
        }
    };
    let mut w = [0_usize; 3];
    for (index, value) in widths.iter().enumerate() {
        w[index] = direct_integer(Some(value), dictionary_start, "xref stream /W width")?;
        if w[index] > 8 {
            return Err(invalid(
                dictionary_start,
                "xref stream /W width exceeds eight bytes",
            ));
        }
    }
    let row_size = w
        .iter()
        .try_fold(0_usize, |sum, &width| sum.checked_add(width))
        .filter(|&size| size != 0)
        .ok_or_else(|| invalid(dictionary_start, "xref stream zero or overflowing row size"))?;
    let ranges = match trailer.get(b"Index".as_slice()) {
        None | Some(Object::Null) => vec![(0_u64, size)],
        Some(Object::Array(index)) if index.len() % 2 == 0 && !index.is_empty() => {
            let mut ranges = Vec::new();
            for pair in index.chunks_exact(2) {
                let start =
                    direct_integer(Some(&pair[0]), dictionary_start, "xref stream Index start")?
                        as u64;
                let count =
                    direct_integer(Some(&pair[1]), dictionary_start, "xref stream Index count")?
                        as u64;
                let end = start
                    .checked_add(count)
                    .filter(|&end| count != 0 && end <= size)
                    .ok_or_else(|| invalid(dictionary_start, "xref stream Index range"))?;
                ranges.push((start, end));
            }
            ranges
        }
        _ => return Err(invalid(dictionary_start, "xref stream /Index pairs")),
    };
    let mut sorted_ranges = ranges.clone();
    sorted_ranges.sort_unstable();
    if sorted_ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(invalid(
            dictionary_start,
            "overlapping xref stream Index ranges",
        ));
    }
    let count = ranges
        .iter()
        .try_fold(0_usize, |sum, &(start, end)| {
            usize::try_from(end - start)
                .ok()
                .and_then(|count| sum.checked_add(count))
        })
        .ok_or_else(|| limit(dictionary_start, "xref entries"))?;
    budget.xref_entries = budget
        .xref_entries
        .checked_add(count)
        .filter(|&count| count <= budget.limits.max_xref_entries)
        .ok_or_else(|| limit(dictionary_start, "xref entries"))?;
    let expected = count
        .checked_mul(row_size)
        .ok_or_else(|| limit(dictionary_start, "xref metadata bytes"))?;
    let remaining = budget
        .limits
        .max_metadata_decoded_bytes
        .saturating_sub(budget.metadata_bytes);
    if expected > remaining {
        return Err(limit(dictionary_start, "xref metadata bytes"));
    }
    let suffix = cursor.pos;
    if cursor.word()? != b"stream" {
        return Err(invalid(cursor.pos, "xref stream marker"));
    }
    cursor.stream_eol()?;
    let start = cursor.pos;
    let end = start
        .checked_add(length)
        .filter(|&end| end <= input.len())
        .ok_or_else(|| invalid(start, "xref stream length"))?;
    cursor.pos = end;
    consume_optional_eol(&mut cursor);
    if cursor.raw_word()? != b"endstream" {
        return Err(invalid(end, "xref stream declared end boundary"));
    }
    if cursor.word()? != b"endobj" {
        return Err(invalid(cursor.pos, "xref stream endobj"));
    }
    let footer = section_footer(&mut cursor, offset)?;
    budget.syntax(cursor.pos - suffix - length, suffix)?;
    let (decoded, cost) =
        decode_structural_metadata(&input[start..end], trailer, remaining, dictionary_start)?;
    budget.metadata_bytes = budget
        .metadata_bytes
        .checked_add(cost)
        .filter(|&bytes| bytes <= budget.limits.max_metadata_decoded_bytes)
        .ok_or_else(|| limit(dictionary_start, "xref metadata bytes"))?;
    if decoded.len() != expected {
        return Err(invalid(
            dictionary_start,
            "xref stream decoded row byte count",
        ));
    }
    let mut entries = BTreeMap::new();
    let mut row_offset = 0;
    for (first, last) in ranges {
        for number in first..last {
            let row = &decoded[row_offset..row_offset + row_size];
            row_offset += row_size;
            let kind = if w[0] == 0 {
                1
            } else {
                big_endian(&row[..w[0]])
            };
            let first = big_endian(&row[w[0]..w[0] + w[1]]);
            let second = big_endian(&row[w[0] + w[1]..]);
            if kind == 2 {
                let container = u32::try_from(first)
                    .ok()
                    .filter(|&number| number != 0)
                    .ok_or_else(|| {
                        invalid(dictionary_start, "compressed object container number")
                    })?;
                let index = usize::try_from(second)
                    .map_err(|_| invalid(dictionary_start, "compressed object member index"))?;
                if number == 0 {
                    return Err(invalid(dictionary_start, "compressed object zero"));
                }
                entries.insert(
                    number as u32,
                    Entry {
                        number: number as u32,
                        generation: 0,
                        offset: 0,
                        live: true,
                        compressed: Some(CompressedEntry { container, index }),
                    },
                );
                continue;
            }
            if kind > 2 {
                return Err(invalid(dictionary_start, "unknown xref stream entry type"));
            }
            let generation = u16::try_from(second)
                .map_err(|_| invalid(dictionary_start, "xref stream generation range"))?;
            let entry_offset = usize::try_from(first)
                .map_err(|_| invalid(dictionary_start, "xref stream offset range"))?;
            if kind == 1 && (number == 0 || entry_offset < 9 || entry_offset > offset) {
                return Err(invalid(dictionary_start, "xref stream direct entry offset"));
            }
            if kind == 0 && first > u64::from(u32::MAX) {
                return Err(invalid(
                    dictionary_start,
                    "xref stream free-list object number",
                ));
            }
            entries.insert(
                number as u32,
                Entry {
                    number: number as u32,
                    generation,
                    offset: entry_offset,
                    live: kind == 1,
                    compressed: None,
                },
            );
        }
    }
    match entries.get(&id.number()) {
        Some(entry)
            if !entry.live || entry.offset != offset || entry.generation != id.generation() =>
        {
            return Err(invalid(
                dictionary_start,
                "conflicting xref stream self entry",
            ));
        }
        Some(_) => {}
        None => {
            // MuPDF-compatible omission policy: only this verified bootstrap
            // object may be supplied when absent from its own sparse Index.
            budget.xref_entries = budget
                .xref_entries
                .checked_add(1)
                .filter(|&count| count <= budget.limits.max_xref_entries)
                .ok_or_else(|| limit(dictionary_start, "xref entries"))?;
            entries.insert(
                id.number(),
                Entry {
                    number: id.number(),
                    generation: id.generation(),
                    offset,
                    live: true,
                    compressed: None,
                },
            );
        }
    }
    // The public trailer and the resolvable stream object both own metadata.
    // Charge the extra copy before cloning, including keys and byte payloads.
    let (extra_objects, extra_bytes) =
        object_cost(&value).ok_or_else(|| limit(dictionary_start, "metadata ownership"))?;
    budget.objects = budget
        .objects
        .checked_add(extra_objects)
        .filter(|&count| count <= budget.limits.max_objects)
        .ok_or_else(|| limit(dictionary_start, "objects"))?;
    budget.bytes = budget
        .bytes
        .checked_add(extra_bytes)
        .filter(|&count| count <= budget.limits.max_decoded_bytes)
        .ok_or_else(|| limit(dictionary_start, "decoded bytes"))?;
    let trailer = trailer.clone();
    Ok(Section {
        region: offset..cursor.pos,
        footer,
        entries,
        trailer,
        bootstrap: Some(IndirectObject {
            id,
            value,
            stream: Some(start..end),
        }),
    })
}

fn big_endian(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(0, |number, &byte| (number << 8) | u64::from(byte))
}

fn consume_optional_eol(cursor: &mut Cursor<'_>) {
    if cursor.pos < cursor.end && cursor.input[cursor.pos] == b'\r' {
        cursor.pos += 1;
        if cursor.pos < cursor.end && cursor.input[cursor.pos] == b'\n' {
            cursor.pos += 1;
        }
    } else if cursor.pos < cursor.end && cursor.input[cursor.pos] == b'\n' {
        cursor.pos += 1;
    }
}

fn decode_structural_metadata(
    data: &[u8],
    dictionary: &Dictionary,
    remaining: usize,
    offset: usize,
) -> Result<(Vec<u8>, usize), DocumentError> {
    use crate::stream::{Filter, FilterSpec, PredictorParams, StreamLimits, decode_filter_specs};
    for key in [b"Filter".as_slice(), b"DecodeParms"] {
        if matches!(dictionary.get(key), Some(Object::Reference(_))) {
            return Err(unsupported(offset, "indirect structural-stream metadata"));
        }
    }
    let filter = match dictionary.get(b"Filter".as_slice()) {
        None | Some(Object::Null) => None,
        Some(Object::Name(name)) if name == b"FlateDecode" => Some(Filter::Flate),
        Some(Object::Array(values)) if values.len() == 1 => match &values[0] {
            Object::Name(name) if name == b"FlateDecode" => Some(Filter::Flate),
            Object::Reference(_) => {
                return Err(unsupported(offset, "indirect structural-stream metadata"));
            }
            _ => {
                return Err(unsupported(
                    offset,
                    "structural stream filter (only direct FlateDecode supported)",
                ));
            }
        },
        _ => {
            return Err(unsupported(
                offset,
                "structural stream filter (only direct FlateDecode supported)",
            ));
        }
    };
    let parameters = match dictionary.get(b"DecodeParms".as_slice()) {
        None | Some(Object::Null) => None,
        Some(Object::Dictionary(parameters)) if filter.is_some() => Some(parameters),
        Some(Object::Array(values)) if values.len() == 1 && filter.is_some() => match &values[0] {
            Object::Null => None,
            Object::Dictionary(parameters) => Some(parameters),
            Object::Reference(_) => {
                return Err(unsupported(offset, "indirect structural-stream metadata"));
            }
            _ => return Err(invalid(offset, "xref stream DecodeParms")),
        },
        _ => return Err(invalid(offset, "xref stream DecodeParms shape")),
    };
    let mut predictor = PredictorParams::default();
    if let Some(parameters) = parameters {
        // In key order, so an error does not depend on source order.
        for (key, value) in parameters.sorted_entries() {
            if matches!(value, Object::Null) {
                continue;
            }
            let integer = direct_integer(Some(value), offset, "xref predictor integer parameter")?;
            match key.as_slice() {
                b"Predictor" => {
                    predictor.predictor = u8::try_from(integer)
                        .map_err(|_| invalid(offset, "xref predictor range"))?
                }
                b"Colors" => predictor.colors = integer,
                b"BitsPerComponent" => {
                    predictor.bits_per_component = u8::try_from(integer)
                        .map_err(|_| invalid(offset, "xref predictor bit depth"))?
                }
                b"Columns" => predictor.columns = integer,
                _ => return Err(unsupported(offset, "xref stream decode parameter")),
            }
        }
    }
    let specs = filter
        .map(|filter| FilterSpec { filter, predictor })
        .into_iter()
        .collect::<Vec<_>>();
    let decoded = decode_filter_specs(
        data,
        &specs,
        StreamLimits {
            max_encoded_bytes: data.len(),
            max_decoded_bytes: remaining,
            max_filters: 1,
            max_total_decoded_bytes: remaining,
        },
    )
    .map_err(|source| error(offset, DocumentErrorKind::MetadataDecode(source)))?;
    let cost = if filter.is_some() && predictor.predictor != 1 {
        let base = decoded
            .len()
            .checked_mul(2)
            .ok_or_else(|| limit(offset, "xref metadata bytes"))?;
        if predictor.predictor >= 10 {
            let row_bytes = predictor
                .colors
                .checked_mul(predictor.columns)
                .and_then(|n| n.checked_mul(usize::from(predictor.bits_per_component)))
                .and_then(|n| n.checked_add(7))
                .map(|n| n / 8)
                .filter(|&n| n != 0)
                .ok_or_else(|| invalid(offset, "xref predictor row size"))?;
            base.checked_add(decoded.len() / row_bytes)
                .ok_or_else(|| limit(offset, "xref metadata bytes"))?
        } else {
            base
        }
    } else {
        decoded.len()
    };
    Ok((decoded, cost))
}

fn section_footer(cursor: &mut Cursor<'_>, section_offset: usize) -> Result<usize, DocumentError> {
    cursor.trivia();
    let footer = cursor.pos;
    if cursor.raw_word()? != b"startxref" {
        return Err(invalid(footer, "section startxref marker"));
    }
    let position = cursor.pos;
    if decimal(cursor.word()?, position)? != section_offset as u64 {
        return Err(invalid(position, "section startxref offset mismatch"));
    }
    cursor.line_end()?;
    if cursor.end.saturating_sub(cursor.pos) < 5
        || &cursor.input[cursor.pos..cursor.pos + 5] != b"%%EOF"
    {
        return Err(invalid(cursor.pos, "section %%EOF marker"));
    }
    cursor.pos += 5;
    if cursor.pos < cursor.end {
        if !is_ws(cursor.input[cursor.pos]) {
            return Err(invalid(cursor.pos, "section %%EOF delimiter"));
        }
        if matches!(cursor.input[cursor.pos], b'\r' | b'\n') {
            cursor.line_end()?;
        }
    }
    Ok(footer)
}

fn trivia_gap(input: &[u8], region: Range<usize>) -> Result<(), DocumentError> {
    let mut cursor = Cursor::new(input, region.start, region.end);
    cursor.trivia();
    if cursor.pos != region.end || cursor.boundary_comment {
        return Err(invalid(
            cursor.pos,
            "unindexed data between physical regions",
        ));
    }
    Ok(())
}

type CompressedGroups = BTreeMap<ObjectId, Vec<(ObjectId, usize)>>;
type OwnedMembers = BTreeMap<ObjectId, Object>;
type StreamRanges = BTreeMap<ObjectId, Option<Range<usize>>>;

fn compressed_groups(
    index: &BTreeMap<u32, Entry>,
    limits: DocumentLimits,
) -> Result<CompressedGroups, DocumentError> {
    let mut groups = BTreeMap::<ObjectId, Vec<(ObjectId, usize)>>::new();
    for entry in index.values().filter(|entry| entry.live) {
        let Some(compressed) = entry.compressed else {
            continue;
        };
        let missing_container = ObjectId::new(compressed.container, 0)
            .map_err(|_| invalid(0, "object-stream container ID"))?;
        let container = index
            .get(&compressed.container)
            .filter(|entry| entry.live)
            .ok_or_else(|| error(0, DocumentErrorKind::MissingObject(missing_container)))?;
        if container.compressed.is_some() {
            return Err(unsupported(
                0,
                "compressed or recursive object-stream containers",
            ));
        }
        let container_id = ObjectId::new(container.number, container.generation)
            .map_err(|_| invalid(container.offset, "object-stream container ID"))?;
        if !groups.contains_key(&container_id) && groups.len() >= limits.max_object_streams {
            return Err(limit(container.offset, "object-stream containers"));
        }
        let member = ObjectId::new(entry.number, 0)
            .map_err(|_| invalid(container.offset, "compressed member object number"))?;
        groups
            .entry(container_id)
            .or_default()
            .push((member, compressed.index));
    }
    Ok(groups)
}

fn load_compressed_members(
    input: &[u8],
    raw: &BTreeMap<ObjectId, RawObject>,
    groups: &CompressedGroups,
    budget: &mut Budget,
    security: Option<&Security>,
) -> Result<(OwnedMembers, StreamRanges), DocumentError> {
    let mut members = BTreeMap::new();
    let mut streams = BTreeMap::new();
    for (&container_id, selected) in groups {
        let container = raw
            .get(&container_id)
            .ok_or_else(|| error(0, DocumentErrorKind::MissingObject(container_id)))?;
        let offset = container.region.start;
        let Object::Dictionary(dictionary) = &container.value else {
            return Err(invalid(offset, "object-stream container dictionary"));
        };
        if !matches!(dictionary.get(b"Type".as_slice()), Some(Object::Name(name)) if name == b"ObjStm")
        {
            return Err(invalid(offset, "object-stream container /Type /ObjStm"));
        }
        let count = direct_integer(
            dictionary.get(b"N".as_slice()),
            offset,
            "object-stream direct /N",
        )?;
        let first = direct_integer(
            dictionary.get(b"First".as_slice()),
            offset,
            "object-stream direct /First",
        )?;
        direct_integer(
            dictionary.get(b"Length".as_slice()),
            offset,
            "object-stream direct /Length",
        )?;
        if count == 0 {
            return Err(invalid(offset, "nonempty object-stream /N"));
        }
        budget.compressed_members = budget
            .compressed_members
            .checked_add(count)
            .filter(|&count| count <= budget.limits.max_compressed_members)
            .ok_or_else(|| limit(offset, "compressed members"))?;
        if selected.iter().any(|(_, index)| *index >= count) {
            return Err(invalid(offset, "compressed member index outside /N"));
        }
        let range = finish_object(input, container, raw, &members)?
            .ok_or_else(|| invalid(offset, "object-stream container has no stream"))?;
        let remaining = budget
            .limits
            .max_metadata_decoded_bytes
            .saturating_sub(budget.metadata_bytes);
        let plain = security
            .map(|s| s.decrypt(s.stream_method(false), container_id, &input[range.clone()]));
        let payload = plain.as_deref().unwrap_or(&input[range.clone()]);
        let (decoded, cost) = decode_structural_metadata(payload, dictionary, remaining, offset)?;
        budget.metadata_bytes = budget
            .metadata_bytes
            .checked_add(cost)
            .filter(|&bytes| bytes <= budget.limits.max_metadata_decoded_bytes)
            .ok_or_else(|| limit(offset, "structural metadata bytes"))?;
        if first > decoded.len() {
            return Err(invalid(offset, "object-stream First exceeds decoded bytes"));
        }
        budget.syntax(first, offset)?;
        let mut cursor = Cursor::new(&decoded, 0, first);
        let mut header = Vec::new();
        header
            .try_reserve_exact(count)
            .map_err(|_| limit(offset, "compressed member header allocation"))?;
        let mut numbers = BTreeSet::new();
        for _ in 0..count {
            let header_offset = cursor.pos;
            let encoded_number = cursor
                .word()
                .and_then(|word| decimal(word, header_offset))
                .map_err(|_| invalid(offset, "compressed member header object number"))?;
            let number = u32::try_from(encoded_number)
                .map_err(|_| invalid(offset, "compressed member object number"))?;
            let relative_offset = cursor.pos;
            let encoded_relative = cursor
                .word()
                .and_then(|word| decimal(word, relative_offset))
                .map_err(|_| invalid(offset, "compressed member header offset"))?;
            let relative = usize::try_from(encoded_relative)
                .map_err(|_| invalid(offset, "compressed member offset"))?;
            if number == 0 || !numbers.insert(number) {
                return Err(invalid(
                    offset,
                    "duplicate or zero compressed member object number",
                ));
            }
            let start = first
                .checked_add(relative)
                .filter(|&start| start < decoded.len())
                .ok_or_else(|| {
                    invalid(offset, "compressed member offset outside decoded stream")
                })?;
            if header
                .last()
                .is_some_and(|&(_, previous)| start <= previous)
            {
                return Err(invalid(
                    offset,
                    "compressed member offsets must increase strictly",
                ));
            }
            header.push((number, start));
        }
        // First is a structural boundary, not permission to shorten the final
        // header integer. Member slices have their own independent boundaries.
        if cursor.pos == first && decoded.get(first).is_some_and(|&byte| !is_delimiter(byte)) {
            return Err(invalid(offset, "object-stream First splits a header token"));
        }
        cursor.trivia();
        if cursor.pos != first || cursor.boundary_comment {
            return Err(invalid(
                offset,
                "object-stream header must contain exactly N pairs",
            ));
        }
        if let Some(&(_, first_member)) = header.first() {
            trivia_gap(&decoded, first..first_member)
                .map_err(|_| invalid(offset, "nontrivial object-stream prefix gap"))?;
            budget.syntax(first_member - first, offset)?;
        }
        for &(member_id, member_index) in selected {
            let (number, start) = header[member_index];
            if number != member_id.number() {
                return Err(invalid(
                    offset,
                    "compressed member index/object-number mismatch",
                ));
            }
            let end = header
                .get(member_index + 1)
                .map_or(decoded.len(), |&(_, start)| start);
            let (value, consumed) =
                budget
                    .parse(&decoded[start..end], 0, true)
                    .map_err(|source| {
                        compressed_error(source, offset, container_id, member_id, start)
                    })?;
            trivia_gap(&decoded, start + consumed..end).map_err(|_| {
                invalid(
                    offset,
                    "compressed member trailing syntax or stream/wrapper",
                )
            })?;
            budget.syntax(end - start - consumed, offset)?;
            members.insert(member_id, value);
        }
        streams.insert(container_id, Some(range));
    }
    Ok((members, streams))
}

fn compressed_error(
    mut error: DocumentError,
    offset: usize,
    container: ObjectId,
    member: ObjectId,
    decoded_start: usize,
) -> DocumentError {
    if let DocumentErrorKind::Parse(mut source) = error.kind {
        source.offset += decoded_start;
        if let ParseErrorKind::Lex(lex) = &mut source.kind {
            lex.offset += decoded_start;
        }
        error.kind = DocumentErrorKind::CompressedParse {
            container,
            member,
            source,
        };
    }
    error.offset = offset;
    error
}

/// Find the effective /Encrypt, authenticate, and decrypt every string in the
/// directly stored objects. Object-stream members are decrypted with their
/// container; xref streams, the encryption dictionary and signature
/// /Contents are never encrypted. `info` receives the dictionary's facts
/// before authentication.
fn authenticate(
    sections: &[Section],
    raw: &mut BTreeMap<ObjectId, RawObject>,
    password: &[u8],
    offset: usize,
    info: &mut Option<EncryptionInfo>,
) -> Result<Option<Security>, DocumentError> {
    let effective = |key: &[u8]| {
        sections
            .iter()
            .find_map(|s| s.trailer.get(key).filter(|v| !matches!(v, Object::Null)))
    };
    let Some(encrypt) = effective(b"Encrypt") else {
        return Ok(None);
    };
    let (dictionary, encrypt_id) = match encrypt {
        Object::Dictionary(d) => (d.clone(), None),
        Object::Reference(id) => match raw.get(id).map(|o| &o.value) {
            Some(Object::Dictionary(d)) => (d.clone(), Some(*id)),
            Some(_) => return Err(invalid(offset, "encryption dictionary")),
            // Spec: the encryption dictionary is never inside an object stream.
            None => return Err(error(offset, DocumentErrorKind::MissingObject(*id))),
        },
        _ => return Err(invalid(offset, "trailer /Encrypt")),
    };
    let first_id = match effective(b"ID") {
        Some(Object::Array(items)) => match items.first() {
            Some(Object::String(id)) => id.clone(),
            _ => return Err(invalid(offset, "trailer /ID")),
        },
        // Algorithms 2-7 hash the ID; R5/R6 do not use it.
        _ => Vec::new(),
    };
    *info = Some(EncryptionInfo::from_dictionary(&dictionary));
    let security =
        Security::authenticate(&dictionary, &first_id, password).map_err(|e| match e {
            SecurityError::Password => error(offset, DocumentErrorKind::PasswordRequired),
            SecurityError::Unsupported(reason) => unsupported(offset, reason),
            SecurityError::Invalid(reason) => invalid(offset, reason),
        })?;
    let method = security.string_method();
    for (id, object) in raw.iter_mut() {
        let xref = matches!(&object.value, Object::Dictionary(d)
            if matches!(d.get(b"Type".as_slice()), Some(Object::Name(n)) if n == b"XRef"));
        if Some(*id) != encrypt_id && !xref {
            decrypt_strings(&mut object.value, &security, method, *id);
        }
    }
    Ok(Some(security))
}

fn decrypt_strings(
    value: &mut Object,
    security: &Security,
    method: crate::security::Method,
    id: ObjectId,
) {
    match value {
        Object::String(bytes) => *bytes = security.decrypt(method, id, bytes),
        Object::Array(items) => items
            .iter_mut()
            .for_each(|item| decrypt_strings(item, security, method, id)),
        Object::Dictionary(entries) => {
            let signature = is_signature_dictionary(entries);
            for (key, item) in entries.iter_mut() {
                if !(signature && key.as_slice() == b"Contents") {
                    decrypt_strings(item, security, method, id);
                }
            }
        }
        _ => {}
    }
}

fn finish_object(
    input: &[u8],
    object: &RawObject,
    objects: &BTreeMap<ObjectId, RawObject>,
    members: &OwnedMembers,
) -> Result<Option<Range<usize>>, DocumentError> {
    let mut cursor = Cursor::new(input, object.suffix, object.region.end);
    cursor.trivia();
    let marker_offset = cursor.pos;
    let marker = cursor.word()?;
    let stream = if marker == b"stream" {
        let Object::Dictionary(dictionary) = &object.value else {
            return Err(invalid(marker_offset, "stream requires a dictionary"));
        };
        if dictionary
            .get(b"F".as_slice())
            .is_some_and(|value| !matches!(value, Object::Null))
        {
            return Err(unsupported(marker_offset, "external-file streams"));
        }
        let length_value = dictionary
            .get(b"Length".as_slice())
            .ok_or_else(|| invalid(marker_offset, "stream /Length"))?;
        let length_value = if let Object::Reference(id) = length_value {
            objects
                .get(id)
                .map(|target| &target.value)
                .or_else(|| members.get(id))
                .ok_or_else(|| error(marker_offset, DocumentErrorKind::MissingObject(*id)))?
        } else {
            length_value
        };
        let length = match length_value {
            Object::Integer(length) => usize::try_from(*length)
                .map_err(|_| invalid(marker_offset, "nonnegative stream length"))?,
            _ => {
                return Err(invalid(
                    marker_offset,
                    "stream length must resolve directly to an integer",
                ));
            }
        };
        cursor.stream_eol()?;
        let start = cursor.pos;
        let end = start
            .checked_add(length)
            .filter(|&end| end <= object.region.end)
            .ok_or_else(|| invalid(start, "stream length exceeds object region"))?;
        cursor.pos = end;
        if input.get(cursor.pos) == Some(&b'\r') {
            cursor.pos += 1;
            if input.get(cursor.pos) == Some(&b'\n') {
                cursor.pos += 1;
            }
        } else if input.get(cursor.pos) == Some(&b'\n') {
            cursor.pos += 1;
        }
        // No whitespace/comment scan here: declared length must land at the
        // actual boundary, not before arbitrary payload that resembles trivia.
        let end_marker = cursor.raw_word()?;
        if end_marker != b"endstream" {
            return Err(invalid(end, "endstream at declared stream boundary"));
        }
        if cursor.word()? != b"endobj" {
            return Err(invalid(cursor.pos, "endobj after stream"));
        }
        Some(start..end)
    } else if marker == b"endobj" {
        None
    } else {
        return Err(invalid(
            marker_offset,
            "endobj or stream marker (possibly overlapping objects)",
        ));
    };
    cursor.trivia();
    if cursor.pos != object.region.end || cursor.boundary_comment {
        return Err(invalid(
            cursor.pos,
            "unindexed or overlapping data between objects",
        ));
    }
    Ok(stream)
}

fn footer(input: &[u8]) -> Result<(usize, usize), DocumentError> {
    let end = trim_ws(input).len();
    if end < 5 || &input[end - 5..end] != b"%%EOF" {
        return Err(invalid(end, "final %%EOF marker"));
    }
    if end < 6 || !matches!(input[end - 6], b'\r' | b'\n') {
        return Err(invalid(end - 5, "%%EOF line boundary"));
    }
    let before_eof = trim_ws(&input[..end - 5]);
    let mut number_start = before_eof.len();
    while number_start > 0 && before_eof[number_start - 1].is_ascii_digit() {
        number_start -= 1;
    }
    if number_start == before_eof.len() || number_start == 0 || !is_ws(before_eof[number_start - 1])
    {
        return Err(invalid(number_start, "startxref offset"));
    }
    let offset = usize::try_from(decimal(&before_eof[number_start..], number_start)?)
        .map_err(|_| invalid(number_start, "startxref range"))?;
    let before_number = trim_ws(&before_eof[..number_start]);
    if !before_number.ends_with(b"startxref") {
        return Err(invalid(number_start, "startxref marker"));
    }
    let marker = before_number.len() - 9;
    if marker > 0 && !is_ws(input[marker - 1]) {
        return Err(invalid(marker, "startxref delimiter"));
    }
    Ok((offset, marker))
}

fn is_ws(byte: u8) -> bool {
    matches!(byte, 0 | b'\t' | b'\n' | 12 | b'\r' | b' ')
}
fn is_delimiter(byte: u8) -> bool {
    is_ws(byte) || b"()<>[]{}/%".contains(&byte)
}
fn trim_ws(mut input: &[u8]) -> &[u8] {
    while input.last().is_some_and(|&byte| is_ws(byte)) {
        input = &input[..input.len() - 1];
    }
    input
}
fn decimal(input: &[u8], offset: usize) -> Result<u64, DocumentError> {
    if input.is_empty() || input.len() > 20 || !input.iter().all(u8::is_ascii_digit) {
        return Err(invalid(offset, "unsigned decimal integer"));
    }
    input.iter().try_fold(0_u64, |number, &digit| {
        number
            .checked_mul(10)
            .and_then(|n| n.checked_add(u64::from(digit - b'0')))
            .ok_or_else(|| invalid(offset, "decimal integer overflow"))
    })
}

struct Cursor<'a> {
    input: &'a [u8],
    pos: usize,
    end: usize,
    boundary_comment: bool,
}
impl<'a> Cursor<'a> {
    fn new(input: &'a [u8], pos: usize, end: usize) -> Self {
        Self {
            input,
            pos,
            end,
            boundary_comment: false,
        }
    }
    fn trivia(&mut self) {
        while self.pos < self.end {
            if is_ws(self.input[self.pos]) {
                self.pos += 1;
            } else if self.input[self.pos] == b'%' {
                while self.pos < self.end && !matches!(self.input[self.pos], b'\r' | b'\n') {
                    self.pos += 1;
                }
                if self.pos == self.end
                    && self
                        .input
                        .get(self.pos)
                        .is_some_and(|byte| !matches!(byte, b'\r' | b'\n'))
                {
                    self.boundary_comment = true;
                }
            } else {
                break;
            }
        }
    }
    fn word(&mut self) -> Result<&'a [u8], DocumentError> {
        self.trivia();
        self.raw_word()
    }
    fn raw_word(&mut self) -> Result<&'a [u8], DocumentError> {
        let start = self.pos;
        while self.pos < self.end && !is_delimiter(self.input[self.pos]) {
            self.pos += 1;
            if self.pos - start > 20 {
                return Err(invalid(start, "oversized structural token"));
            }
        }
        if self.pos == start {
            return Err(invalid(start, "structural token"));
        }
        Ok(&self.input[start..self.pos])
    }
    fn line_end(&mut self) -> Result<(), DocumentError> {
        while self.pos < self.end && matches!(self.input[self.pos], b' ' | b'\t') {
            self.pos += 1;
        }
        if self.pos >= self.end {
            return Err(invalid(self.pos, "xref line ending"));
        }
        match self.input[self.pos] {
            b'\n' => self.pos += 1,
            b'\r' => {
                self.pos += 1;
                if self.pos < self.end && self.input[self.pos] == b'\n' {
                    self.pos += 1;
                }
            }
            _ => return Err(invalid(self.pos, "xref line ending")),
        }
        Ok(())
    }
    fn stream_eol(&mut self) -> Result<(), DocumentError> {
        if self.pos < self.end && self.input[self.pos] == b'\n' {
            self.pos += 1;
            return Ok(());
        }
        if self.pos + 1 < self.end && &self.input[self.pos..self.pos + 2] == b"\r\n" {
            self.pos += 2;
            return Ok(());
        }
        Err(invalid(self.pos, "LF or CRLF immediately after stream"))
    }
    fn xref_entry(&mut self, number: u32) -> Result<Entry, DocumentError> {
        let start = self.pos;
        if self.end.saturating_sub(start) < 18 {
            return Err(invalid(start, "truncated xref entry"));
        }
        let bytes = &self.input[start..start + 18];
        if bytes[10] != b' ' || bytes[16] != b' ' || !matches!(bytes[17], b'n' | b'f') {
            return Err(invalid(start, "fixed-width xref entry"));
        }
        let offset = usize::try_from(decimal(&bytes[..10], start)?)
            .map_err(|_| invalid(start, "xref offset range"))?;
        let generation = u16::try_from(decimal(&bytes[11..16], start + 11)?)
            .map_err(|_| invalid(start + 11, "xref generation range"))?;
        self.pos += 18;
        self.line_end()?;
        Ok(Entry {
            number,
            generation,
            offset,
            live: bytes[17] == b'n',
            compressed: None,
        })
    }
}
