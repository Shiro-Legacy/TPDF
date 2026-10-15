// SPDX-License-Identifier: AGPL-3.0-or-later
// Repair behavior references MuPDF 1.28.1 source/pdf/pdf-repair.c,
// Copyright (C) 2004-2026 Artifex Software, Inc. See NOTICE.
//! Tolerant rebuild of damaged PDFs into one clean, strictly parseable revision.
//!
//! Like MuPDF's repair, the scan ignores every xref and finds each `N G obj`
//! after the header. Values parse leniently (duplicate keys keep the last
//! value, a missing `endobj` is tolerated) and a stream whose /Length does not
//! land on `endstream` ends at the next `endstream`. When the file's own xref
//! chain (classic, stream, hybrid, forward /Prev) points at the scanned
//! objects, it selects them, as a reader that does not need repair would.
//! Otherwise later definitions of an object number override earlier ones,
//! whatever their generation; members of a winning object stream compete at
//! the container's position. Root, Info, ID and Encrypt come from the last
//! trailer or xref-stream dictionary holding each; without a usable Root, the
//! last /Type /Catalog object is used.
//!
//! References to missing objects become null (dropped from object lists such
//! as /Kids and /Contents) and references with a stale generation are pointed
//! at the surviving object, as MuPDF resolves them. The page tree is then made
//! consistent, and a linearization dictionary, whose offsets no longer apply,
//! is dropped with its hint stream.
//!
//! The output has a classic xref table. Xref streams and object-stream
//! containers are dropped. Encrypted objects are copied verbatim, keeping
//! /Encrypt, so the security handler decrypts them on open; object streams in
//! encrypted files are refused, as MuPDF cannot read them after repair either.
//! Every byte is scanned a bounded number of times: each value parse stops at
//! the next object header (parses retried past one share a budget of one
//! pass over the input), object-stream members are parsed once per distinct
//! offset, and stream data is skipped once read.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Range;

use crate::document::{Document, DocumentError, DocumentErrorKind, DocumentLimits};
use crate::lexer::LexErrorKind;
use crate::object::{Dictionary, Object, ObjectId};
use crate::parser::{
    DuplicateKeyPolicy, ParseError, ParseErrorKind, ParseLimitKind, ParseLimits, Parser,
};
use crate::serialize::{SerializeError, SerializeStyle, WriteLimits, Writer};
use crate::stream::{
    Filter, FilterSpec, PredictorParams, StreamErrorKind, StreamLimits,
    decode_filter_specs_with_usage,
};

#[derive(Clone, Copy, Debug)]
pub struct RepairLimits {
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    /// Object definitions kept, including superseded ones and object-stream members.
    pub max_objects: usize,
    /// Parsed values across all definitions and trailers, counting dictionary keys.
    pub max_values: usize,
    pub max_object_streams: usize,
    /// Decoded object-stream, xref-stream and length-check bytes across the file.
    pub max_decoded_bytes: usize,
    /// Per value; `max_objects` is further capped by the remaining `max_values`.
    pub parse: ParseLimits,
}

impl Default for RepairLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 128 * 1024 * 1024,
            max_output_bytes: 128 * 1024 * 1024,
            max_objects: 400_000,
            max_values: 2_000_000,
            max_object_streams: 4096,
            max_decoded_bytes: 64 * 1024 * 1024,
            parse: ParseLimits::default(),
        }
    }
}

impl RepairLimits {
    /// Limits no looser than `limits`, which the rebuilt file must open under.
    pub fn for_document(limits: DocumentLimits) -> Self {
        Self {
            max_input_bytes: limits.max_input_bytes,
            max_output_bytes: limits.max_input_bytes,
            max_objects: limits
                .max_xref_entries
                .saturating_add(limits.max_compressed_members),
            max_values: limits.max_objects,
            max_object_streams: limits.max_object_streams,
            max_decoded_bytes: limits.max_metadata_decoded_bytes,
            parse: limits.parse,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepairReport {
    /// Objects written to the repaired file.
    pub objects_recovered: usize,
    /// Definitions superseded by a later definition of the same object number,
    /// or, when the xref selected the objects, not listed in it.
    pub objects_overridden: usize,
    /// Object headers whose value could not be parsed, object-stream members
    /// that could not be parsed or share an offset, and containers that could
    /// not be decoded.
    pub objects_dropped: usize,
    pub object_streams_expanded: usize,
    /// Written streams whose length came from the `endstream` search.
    pub streams_length_recovered: usize,
    /// Dangling references removed (null in arrays) or stale generations fixed.
    pub references_repaired: usize,
    /// No trailer Root named a catalog; the last /Type /Catalog object was used.
    pub catalog_from_scan: bool,
    /// Kids, Count, Parent, Type, MediaBox or Contents entries were fixed, or a
    /// page tree was built from the /Type /Page objects.
    pub page_tree_repaired: bool,
    /// A linearization dictionary (and its hint stream) was dropped.
    pub linearized: bool,
    /// The file's own xref chain described its objects and selected them.
    pub xref_intact: bool,
    /// Sections in that chain, a hybrid /XRefStm counted with its table;
    /// 0 when the chain could not be read.
    pub xref_sections: usize,
    /// One past the largest object number the chain accounts for: its
    /// largest /Size or listed number (in use or free); 0 when not read.
    pub xref_size: u64,
    /// The input was damaged: its header was not at byte 0, its xref did not
    /// describe its objects (MuPDF's wasRepaired), or stream lengths, the
    /// catalog or the page tree had to be recovered. False when the file was
    /// rebuilt only because the strict reader refuses a valid structure
    /// (linearization, hybrid xref, duplicate keys, header layout).
    pub damaged: bool,
    /// Why the strict open failed, when the caller records it; `TolerantPdf`
    /// keeps the typed error.
    pub strict_error: Option<String>,
}

#[derive(Debug)]
pub struct RepairedPdf {
    pub bytes: Vec<u8>,
    pub report: RepairReport,
}

#[derive(Debug)]
pub enum RepairError {
    Invalid(&'static str),
    Unsupported(&'static str),
    LimitExceeded(&'static str),
    Serialize(SerializeError),
    /// The rebuilt bytes did not open strictly, or `open_tolerant` could not
    /// open the input (a strict limit, or the repaired file under the caller's
    /// limits).
    Document(DocumentError),
}

impl fmt::Display for RepairError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "PDF repair: invalid {reason}"),
            Self::Unsupported(reason) => write!(f, "PDF repair: unsupported {reason}"),
            Self::LimitExceeded(reason) => write!(f, "PDF repair: {reason} limit exceeded"),
            Self::Serialize(error) => write!(f, "PDF repair: {error}"),
            Self::Document(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for RepairError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Serialize(error) => Some(error),
            Self::Document(error) => Some(error),
            _ => None,
        }
    }
}

impl From<SerializeError> for RepairError {
    fn from(error: SerializeError) -> Self {
        Self::Serialize(error)
    }
}

/// The bytes to open strictly with the caller's limits (and password, unless
/// `needs_password`). When `repair` is set they were rebuilt: they hold one
/// revision with fresh offsets, so incremental saves, signatures and revision
/// history must refer to `original`, and a session has to keep this state to
/// refuse incremental writes, as MuPDF does for repaired files. Show a file as
/// repaired only when `repair.damaged` is set.
#[derive(Debug)]
pub struct TolerantPdf {
    pub bytes: Vec<u8>,
    pub repair: Option<RepairReport>,
    /// The strict refusal of the input, when it was rebuilt.
    pub strict_error: Option<DocumentError>,
    /// The input, when it was rebuilt.
    pub original: Option<Vec<u8>>,
    /// The password did not authenticate. Open `bytes` strictly with the right
    /// one; when `repair` is None the input may still need repair after that.
    pub needs_password: bool,
}

/// Open strictly first; rebuild only when that fails for a reason other than
/// the password or a limit. The returned bytes open strictly with `limits`.
pub fn open_tolerant(
    input: Vec<u8>,
    limits: DocumentLimits,
    password: &[u8],
) -> Result<TolerantPdf, RepairError> {
    if input.len() > limits.max_input_bytes {
        return Err(RepairError::LimitExceeded("input bytes"));
    }
    let needs_password = match Document::open_with_password(&input, limits, password) {
        Ok(_) => false,
        Err(error) if matches!(error.kind, DocumentErrorKind::PasswordRequired) => true,
        Err(error) if matches!(error.kind, DocumentErrorKind::LimitExceeded(_)) => {
            return Err(RepairError::Document(error));
        }
        Err(error) => return rebuild_refused(input, error, limits, password),
    };
    Ok(TolerantPdf {
        bytes: input,
        repair: None,
        strict_error: None,
        original: None,
        needs_password,
    })
}

fn rebuild_refused(
    input: Vec<u8>,
    strict: DocumentError,
    limits: DocumentLimits,
    password: &[u8],
) -> Result<TolerantPdf, RepairError> {
    let mut repaired = rebuild(&input, RepairLimits::for_document(limits))?;
    repaired.report.strict_error = Some(strict.to_string());
    let needs_password = match Document::open_with_password(&repaired.bytes, limits, password) {
        Ok(_) => false,
        Err(error) if matches!(error.kind, DocumentErrorKind::PasswordRequired) => true,
        Err(error) => return Err(RepairError::Document(error)),
    };
    Ok(TolerantPdf {
        bytes: repaired.bytes,
        repair: Some(repaired.report),
        strict_error: Some(strict),
        original: Some(input),
        needs_password,
    })
}

/// Rebuild `input` from a full object scan. The result opens with
/// `Document::open` (or needs a password); offsets are relative to the header,
/// and any bytes before it are dropped.
pub fn rebuild(input: &[u8], limits: RepairLimits) -> Result<RepairedPdf, RepairError> {
    if input.len() > limits.max_input_bytes {
        return Err(RepairError::LimitExceeded("input bytes"));
    }
    let (at, body, version) = header(input);
    let mut scan = Scan {
        body,
        limits,
        defs: Vec::new(),
        trailer: BTreeMap::new(),
        trailers: BTreeMap::new(),
        xref_streams: BTreeMap::new(),
        values: 0,
        decoded: 0,
        wasted: 0,
        report: RepairReport::default(),
    };
    scan.run()?;
    let bytes = scan.finish(version, at == 0)?;
    let verify = DocumentLimits {
        max_input_bytes: usize::MAX,
        max_xref_entries: usize::MAX,
        max_revisions: usize::MAX,
        max_metadata_decoded_bytes: usize::MAX,
        max_object_streams: usize::MAX,
        max_compressed_members: usize::MAX,
        max_objects: usize::MAX,
        max_decoded_bytes: usize::MAX,
        max_syntax_bytes: usize::MAX,
        parse: limits.parse,
    };
    match Document::open(&bytes, verify) {
        Err(error) if !matches!(error.kind, DocumentErrorKind::PasswordRequired) => {
            return Err(RepairError::Document(error));
        }
        _ => {}
    }
    Ok(RepairedPdf {
        bytes,
        report: scan.report,
    })
}

struct Stream {
    data: Range<usize>,
    /// Where `endstream` starts when the length was recovered.
    keyword: Option<usize>,
    length: Option<ObjectId>,
}

struct Def {
    id: ObjectId,
    /// File position, then 1-based member index within an object stream.
    order: (usize, usize),
    value: Object,
    stream: Option<Stream>,
}

/// An in-use xref entry: a file offset, or a container number and index.
#[derive(Clone, Copy)]
enum Entry {
    Offset(usize),
    Member(u32, usize),
}

/// Each object number's newest entry in one section; None when free.
type Section = BTreeMap<u32, Option<Entry>>;

enum Decoded {
    Bytes(Vec<u8>),
    Failed,
    Limit,
}

#[derive(Clone, Copy, PartialEq)]
enum Cipher {
    None,
    Rc4,
    Aes,
}

struct Scan<'a> {
    body: &'a [u8],
    limits: RepairLimits,
    defs: Vec<Def>,
    trailer: BTreeMap<&'static [u8], Object>,
    /// Classic trailer dictionaries by the position of their keyword.
    trailers: BTreeMap<usize, Dictionary>,
    /// Xref streams by object position: dictionary and data.
    xref_streams: BTreeMap<usize, (Dictionary, Range<usize>)>,
    values: usize,
    decoded: usize,
    /// Bytes parsed past an event bound without success.
    wasted: usize,
    report: RepairReport,
}

enum Kind {
    Object(ObjectId),
    Trailer,
}

struct Event {
    start: usize,
    keyword_end: usize,
    kind: Kind,
}

struct Frame {
    index: usize,
    id: ObjectId,
    kids: Vec<Object>,
    /// Kids was an array or a reference to one.
    listed: bool,
    next: usize,
    kept: Vec<Object>,
    count: usize,
    media: bool,
}

/// Trivia allowed between a stream's data and `endstream`, or after it before
/// `endobj`; bounding it keeps the scan linear.
const GAP: usize = 256;
/// Strings longer than this are written as literal strings, which are never
/// longer than the token they were read from; hex doubles them.
const LONG_STRING: usize = 4096;
/// The most a Flate stream can inflate per input byte; failed decodes are
/// charged this much.
const INFLATE_RATIO: usize = 1032;
const MAX_SECTIONS: usize = 4096;
/// Arrays under these keys list objects; a dangling entry is dropped rather
/// than replaced with null.
const LISTS: [&[u8]; 8] = [
    b"Kids",
    b"Contents",
    b"Annots",
    b"Fields",
    b"OCGs",
    b"Order",
    b"ON",
    b"OFF",
];

impl Scan<'_> {
    fn run(&mut self) -> Result<(), RepairError> {
        let body = self.body;
        let mut next = next_event(body, 0);
        while let Some(event) = next {
            let following = next_event(body, event.keyword_end);
            let bound = following.as_ref().map_or(body.len(), |e| e.start);
            let end = match event.kind {
                Kind::Object(id) => self.object(id, event.start, event.keyword_end, bound)?,
                Kind::Trailer => self.trailer_dictionary(event.start, bound)?,
            };
            next = match (end, following) {
                (Some(end), Some(following)) if end > following.start => next_event(body, end),
                (_, following) => following,
            };
        }
        Ok(())
    }

    /// Parse one value; the inner error when it is absent or malformed.
    fn parse(&mut self, input: &[u8]) -> Result<Result<(Object, usize), ParseError>, RepairError> {
        let remaining = self.limits.max_values.saturating_sub(self.values);
        let mut limits = self.limits.parse;
        limits.max_objects = limits.max_objects.min(remaining);
        let parsed = Parser::new(input, limits)
            .with_duplicate_key_policy(DuplicateKeyPolicy::KeepLast)
            .parse_next();
        match parsed {
            Ok(Some(parsed)) => {
                self.values += cost(&parsed.object);
                Ok(Ok((parsed.object, parsed.span.end)))
            }
            Ok(None) => Ok(Err(ParseError {
                offset: input.len(),
                kind: ParseErrorKind::EmptyInput,
            })),
            Err(error)
                if matches!(
                    error.kind,
                    ParseErrorKind::LimitExceeded(ParseLimitKind::Objects)
                ) && remaining < self.limits.parse.max_objects =>
            {
                Err(RepairError::LimitExceeded("values"))
            }
            Err(error) => Ok(Err(error)),
        }
    }

    /// Parse past an event bound that may lie inside a literal string, such as
    /// `(the trailer)`. Accepted only when `endobj` or `stream` follows; failed
    /// attempts share a budget of one pass over the input.
    fn reparse(&mut self, value_start: usize) -> Result<Option<(Object, usize)>, RepairError> {
        let body = self.body;
        if self.wasted > body.len() {
            return Ok(None);
        }
        match self.parse(&body[value_start..])? {
            Ok((value, consumed)) => {
                let end = value_start + consumed;
                let rest = &body[skip_trivia(body, end, body.len().min(end + GAP))..];
                if rest.starts_with(b"endobj")
                    || matches!(value, Object::Dictionary(_)) && rest.starts_with(b"stream")
                {
                    return Ok(Some((value, consumed)));
                }
                self.wasted += consumed;
            }
            Err(error) => self.wasted += error.offset.max(1),
        }
        Ok(None)
    }

    fn object(
        &mut self,
        id: ObjectId,
        start: usize,
        value_start: usize,
        mut bound: usize,
    ) -> Result<Option<usize>, RepairError> {
        let body = self.body;
        let parsed = match self.parse(&body[value_start..bound])? {
            Ok(parsed) => Some(parsed),
            Err(error) if bound < body.len() && open_string(&error) => {
                let parsed = self.reparse(value_start)?;
                if parsed.is_some() {
                    bound = body.len();
                }
                parsed
            }
            Err(_) => None,
        };
        let Some((value, consumed)) = parsed else {
            self.report.objects_dropped += 1;
            return Ok(None);
        };
        let value_end = value_start + consumed;
        let mut end = value_end;
        let mut stream = None;
        let mut p = skip_trivia(body, value_end, bound);
        if let Object::Dictionary(dictionary) = &value {
            if body[p..bound].starts_with(b"stream") {
                let (found, after) = stream_data(body, dictionary, p + 6);
                stream = Some(found);
                end = after;
                p = skip_trivia(body, after, body.len().min(after + GAP));
            }
        }
        let limit = if stream.is_some() { body.len() } else { bound };
        if body[p..limit].starts_with(b"endobj") {
            end = p + 6;
        }
        if let (Some(found), Object::Dictionary(dictionary)) = (&stream, &value) {
            if type_is(&value, b"XRef") {
                self.harvest(dictionary);
                self.xref_streams
                    .insert(start, (dictionary.clone(), found.data.clone()));
                return Ok(Some(end));
            }
        }
        self.define(Def {
            id,
            order: (start, 0),
            value,
            stream,
        })?;
        Ok(Some(end))
    }

    fn trailer_dictionary(
        &mut self,
        keyword: usize,
        bound: usize,
    ) -> Result<Option<usize>, RepairError> {
        let body = self.body;
        let start = keyword + 7;
        let Ok((Object::Dictionary(dictionary), consumed)) = self.parse(&body[start..bound])?
        else {
            return Ok(None);
        };
        self.harvest(&dictionary);
        self.trailers.insert(keyword, dictionary);
        Ok(Some(start + consumed))
    }

    fn harvest(&mut self, dictionary: &Dictionary) {
        for key in [b"Root".as_slice(), b"Info", b"ID", b"Encrypt"] {
            match dictionary.get(key) {
                None | Some(Object::Null) => {}
                Some(value) => {
                    self.trailer.insert(key, value.clone());
                }
            }
        }
    }

    fn define(&mut self, def: Def) -> Result<(), RepairError> {
        if self.defs.len() >= self.limits.max_objects {
            return Err(RepairError::LimitExceeded("objects"));
        }
        self.defs.push(def);
        Ok(())
    }

    fn winners(&self) -> BTreeMap<u32, usize> {
        let mut winners = BTreeMap::<u32, usize>::new();
        for (index, def) in self.defs.iter().enumerate() {
            let slot = winners.entry(def.id.number()).or_insert(index);
            if self.defs[*slot].order < def.order {
                *slot = index;
            }
        }
        winners
    }

    /// The trailer's /Encrypt, or, when no trailer survived, the last standard
    /// encryption dictionary found, which nothing else references.
    fn encryption(&self) -> Option<Object> {
        if let Some(value) = self.trailer.get(b"Encrypt".as_slice()) {
            return Some(value.clone());
        }
        if !self.trailers.is_empty() || !self.xref_streams.is_empty() {
            return None;
        }
        self.defs
            .iter()
            .filter(|def| {
                def.stream.is_none()
                    && matches!(&def.value, Object::Dictionary(d)
                        if matches!(d.get(b"Filter".as_slice()), Some(Object::Name(n)) if n == b"Standard")
                            && matches!(d.get(b"O".as_slice()), Some(Object::String(_)))
                            && matches!(d.get(b"U".as_slice()), Some(Object::String(_))))
            })
            .max_by_key(|def| def.order)
            .map(|def| Object::Reference(def.id))
    }

    /// The winning definition of each object number, and the object-stream
    /// containers among them, which are expanded but not written.
    fn select(
        &mut self,
        encrypted: bool,
    ) -> Result<(BTreeMap<u32, usize>, BTreeSet<usize>), RepairError> {
        if let Some(xref) = self.read_xref() {
            let saved = (
                self.defs.len(),
                self.report.clone(),
                self.decoded,
                self.values,
            );
            if let Some(winners) = self.select_by_xref(&xref, encrypted)? {
                self.report.xref_intact = true;
                let containers = winners
                    .values()
                    .copied()
                    .filter(|&i| is_object_stream(&self.defs[i]))
                    .collect();
                return Ok((winners, containers));
            }
            self.defs.truncate(saved.0);
            self.report = saved.1;
            self.decoded = saved.2;
            self.values = saved.3;
        }
        let containers: BTreeSet<usize> = self
            .winners()
            .into_values()
            .filter(|&i| is_object_stream(&self.defs[i]))
            .collect();
        if encrypted && !containers.is_empty() {
            return Err(RepairError::Unsupported(
                "object streams in an encrypted document",
            ));
        }
        if containers.len() > self.limits.max_object_streams {
            return Err(RepairError::LimitExceeded("object streams"));
        }
        for &index in &containers {
            if self.expand(index)? {
                self.report.object_streams_expanded += 1;
            } else {
                self.report.objects_dropped += 1;
            }
        }
        Ok((self.winners(), containers))
    }

    /// Select the definitions the xref names; None when any in-use entry does
    /// not point at a scanned object of that number.
    fn select_by_xref(
        &mut self,
        xref: &BTreeMap<u32, Entry>,
        encrypted: bool,
    ) -> Result<Option<BTreeMap<u32, usize>>, RepairError> {
        let body = self.body;
        let starts: BTreeMap<usize, usize> = self
            .defs
            .iter()
            .enumerate()
            .map(|(i, def)| (def.order.0, i))
            .collect();
        let mut winners = BTreeMap::new();
        let mut members = Vec::new();
        for (&number, &entry) in xref {
            match entry {
                Entry::Offset(offset) => {
                    let Some(at) = skip_whitespace(body, offset) else {
                        return Ok(None);
                    };
                    match starts.get(&at) {
                        Some(&i) if self.defs[i].id.number() == number => {
                            winners.insert(number, i);
                        }
                        _ if self.xref_streams.contains_key(&at) => {}
                        _ => return Ok(None),
                    }
                }
                Entry::Member(container, index) => members.push((number, container, index)),
            }
        }
        let mut containers = BTreeMap::<u32, usize>::new();
        for &(_, container, _) in &members {
            match winners.get(&container) {
                Some(&i) if is_object_stream(&self.defs[i]) => {
                    containers.insert(container, i);
                }
                _ => return Ok(None),
            }
        }
        if encrypted && !containers.is_empty() {
            return Err(RepairError::Unsupported(
                "object streams in an encrypted document",
            ));
        }
        if containers.len() > self.limits.max_object_streams {
            return Err(RepairError::LimitExceeded("object streams"));
        }
        for &index in containers.values() {
            if !self.expand(index)? {
                return Ok(None);
            }
            self.report.object_streams_expanded += 1;
        }
        let positions: BTreeMap<(usize, usize), usize> = self
            .defs
            .iter()
            .enumerate()
            .filter(|(_, def)| def.order.1 > 0)
            .map(|(i, def)| (def.order, i))
            .collect();
        for (number, container, index) in members {
            let position = self.defs[containers[&container]].order.0;
            match positions.get(&(position, index.saturating_add(1))) {
                Some(&i) if self.defs[i].id.number() == number => {
                    winners.insert(number, i);
                }
                _ => return Ok(None),
            }
        }
        Ok(Some(winners))
    }

    /// Follow the xref chain from the last `startxref`, newest section first,
    /// accepting forward /Prev and hybrid /XRefStm. None when any part of it
    /// cannot be read, or objects follow it.
    fn read_xref(&mut self) -> Option<BTreeMap<u32, Entry>> {
        let body = self.body;
        let tail = body.len().saturating_sub(1024 + 9);
        let marker = tail + body[tail..].windows(9).rposition(|w| w == b"startxref")?;
        // Objects after it are an update whose xref was lost.
        if self.defs.last().is_some_and(|def| def.order.0 > marker)
            || self.trailers.range(marker..).next().is_some()
            || self.xref_streams.range(marker..).next().is_some()
        {
            return None;
        }
        let (mut offset, _) = integer_at(body, skip_whitespace(body, marker + 9)?)?;
        let mut entries = Section::new();
        let mut visited = BTreeSet::new();
        let mut budget = self.limits.max_objects;
        let declared = |dictionary: &Dictionary| match dictionary.get(b"Size".as_slice()) {
            Some(Object::Integer(size)) => u64::try_from(*size).unwrap_or(0),
            _ => 0,
        };
        let mut size = 0;
        loop {
            if !visited.insert(offset) || visited.len() > MAX_SECTIONS {
                return None;
            }
            let at = skip_whitespace(body, offset)?;
            let (mut section, trailer) = if body[at..].starts_with(b"xref") {
                self.xref_table(at + 4, &mut budget)?
            } else {
                self.xref_stream(at, &mut budget)?
            };
            size = size.max(declared(&trailer));
            if let Some(Object::Integer(stream)) = trailer.get(b"XRefStm".as_slice()) {
                let at = skip_whitespace(body, usize::try_from(*stream).ok()?)?;
                let (hybrid, dictionary) = self.xref_stream(at, &mut budget)?;
                size = size.max(declared(&dictionary));
                for (number, entry) in hybrid {
                    if !matches!(section.get(&number), Some(Some(_))) {
                        section.insert(number, entry);
                    }
                }
            }
            for (number, entry) in section {
                entries.entry(number).or_insert(entry);
            }
            match trailer.get(b"Prev".as_slice()) {
                Some(Object::Integer(prev)) => offset = usize::try_from(*prev).ok()?,
                _ => break,
            }
        }
        let listed = entries.keys().next_back().map_or(0, |&n| u64::from(n) + 1);
        self.report.xref_sections = visited.len();
        self.report.xref_size = size.max(listed);
        Some(
            entries
                .into_iter()
                .filter_map(|(number, entry)| Some((number, entry?)))
                .collect(),
        )
    }

    fn xref_table(&self, mut p: usize, budget: &mut usize) -> Option<(Section, Dictionary)> {
        let body = self.body;
        let mut section = Section::new();
        loop {
            p = skip_trivia(body, p, body.len());
            if body[p..].starts_with(b"trailer") {
                return Some((section, self.trailers.get(&p)?.clone()));
            }
            let (first, next) = integer_at(body, p)?;
            let (count, next) = integer_at(body, skip_whitespace(body, next)?)?;
            p = next;
            *budget = budget.checked_sub(count)?;
            for i in 0..count {
                let (offset, next) = integer_at(body, skip_whitespace(body, p)?)?;
                let (_, next) = integer_at(body, skip_whitespace(body, next)?)?;
                let kind = skip_whitespace(body, next)?;
                p = kind + 1;
                let entry = match body.get(kind)? {
                    b'n' if offset > 0 => Some(Entry::Offset(offset)),
                    b'n' | b'f' => None,
                    _ => return None,
                };
                let number = u32::try_from(first.checked_add(i)?).ok()?;
                section.entry(number).or_insert(entry);
            }
        }
    }

    fn xref_stream(&mut self, at: usize, budget: &mut usize) -> Option<(Section, Dictionary)> {
        let body = self.body;
        let (dictionary, data) = self.xref_streams.get(&at)?.clone();
        let specs = flate_specs(&dictionary)?;
        let Decoded::Bytes(bytes) = self.decode(&body[data], &specs) else {
            return None;
        };
        let integer = |value: &Object| match value {
            Object::Integer(n) => usize::try_from(*n).ok(),
            _ => None,
        };
        let widths: Vec<usize> = match dictionary.get(b"W".as_slice()) {
            Some(Object::Array(items)) if items.len() == 3 => items
                .iter()
                .map(|item| integer(item).filter(|&w| w <= 8))
                .collect::<Option<_>>()?,
            _ => return None,
        };
        let row = widths.iter().sum::<usize>();
        let size = integer(dictionary.get(b"Size".as_slice())?)?;
        let index: Vec<usize> = match dictionary.get(b"Index".as_slice()) {
            None => vec![0, size],
            Some(Object::Array(items)) if items.len() % 2 == 0 => {
                items.iter().map(integer).collect::<Option<_>>()?
            }
            _ => return None,
        };
        if row == 0 {
            return None;
        }
        let mut rows = bytes.chunks_exact(row);
        let mut section = Section::new();
        for pair in index.chunks_exact(2) {
            *budget = budget.checked_sub(pair[1])?;
            for i in 0..pair[1] {
                let row = rows.next()?;
                let mut fields = [0_u64; 3];
                let mut at = 0;
                for (field, &width) in fields.iter_mut().zip(&widths) {
                    *field = row[at..at + width]
                        .iter()
                        .fold(0, |n, &b| (n << 8) | u64::from(b));
                    at += width;
                }
                if widths[0] == 0 {
                    fields[0] = 1;
                }
                let entry = match fields[0] {
                    1 if fields[1] > 0 => Some(Entry::Offset(usize::try_from(fields[1]).ok()?)),
                    2 => Some(Entry::Member(
                        u32::try_from(fields[1]).ok()?,
                        usize::try_from(fields[2]).ok()?,
                    )),
                    _ => None,
                };
                let number = u32::try_from(pair[0].checked_add(i)?).ok()?;
                section.entry(number).or_insert(entry);
            }
        }
        Some((section, dictionary))
    }

    /// Decode against the shared budget. A failure is charged the most its
    /// input could have inflated to.
    fn decode(&mut self, data: &[u8], specs: &[FilterSpec]) -> Decoded {
        let remaining = self.limits.max_decoded_bytes.saturating_sub(self.decoded);
        let limits = StreamLimits {
            max_encoded_bytes: data.len(),
            max_decoded_bytes: remaining,
            max_filters: 1,
            max_total_decoded_bytes: remaining,
        };
        match decode_filter_specs_with_usage(data, specs, limits) {
            Ok(decoded) => {
                self.decoded += decoded.usage.total_decoded_bytes;
                Decoded::Bytes(decoded.bytes)
            }
            Err(error)
                if matches!(
                    error.kind,
                    StreamErrorKind::DecodedOutputLimit { .. }
                        | StreamErrorKind::TotalOutputLimit { .. }
                ) =>
            {
                Decoded::Limit
            }
            Err(_) => {
                self.decoded += data.len().saturating_mul(INFLATE_RATIO).min(remaining);
                Decoded::Failed
            }
        }
    }

    /// Where a recovered stream's data ends: before the EOL in front of
    /// `endstream` unless only a longer candidate decodes (the CR of a CRLF,
    /// or the whole EOL, may be data) or, under AES, is whole blocks.
    fn settle(&mut self, index: usize, cipher: Cipher, encrypt_metadata: bool) {
        let body = self.body;
        let def = &self.defs[index];
        let (Some(stream), Object::Dictionary(dictionary)) = (&def.stream, &def.value) else {
            return;
        };
        let Some(keyword) = stream.keyword else {
            return;
        };
        let start = stream.data.start;
        let ends = stream_ends(body, start, keyword);
        if ends.len() < 2 {
            return;
        }
        let cipher = if type_is(&def.value, b"Metadata") && !encrypt_metadata {
            Cipher::None
        } else {
            cipher
        };
        let flate = match dictionary.get(b"Filter".as_slice()) {
            Some(Object::Name(name)) => name == b"FlateDecode",
            Some(Object::Array(names)) => {
                matches!(names.first(), Some(Object::Name(name)) if name == b"FlateDecode")
            }
            _ => false,
        };
        let chosen = match cipher {
            Cipher::Aes => ends.iter().copied().find(|&end| (end - start) % 16 == 0),
            Cipher::None if flate => {
                let specs = [FilterSpec::from(Filter::Flate)];
                let mut chosen = None;
                for &end in &ends {
                    match self.decode(&body[start..end], &specs) {
                        Decoded::Bytes(_) => chosen = Some(end),
                        Decoded::Failed => continue,
                        Decoded::Limit => {}
                    }
                    break;
                }
                chosen
            }
            _ => None,
        };
        if let (Some(end), Some(stream)) = (chosen, &mut self.defs[index].stream) {
            stream.data.end = end;
        }
    }

    fn finish(&mut self, version: [u8; 3], at_start: bool) -> Result<Vec<u8>, RepairError> {
        let encryption = self.encryption();
        let encryption_from_scan =
            encryption.is_some() && !self.trailer.contains_key(b"Encrypt".as_slice());
        let (winners, containers) = self.select(encryption.is_some())?;
        let mut written = winners;
        self.report.objects_overridden = self.defs.len() - written.len();
        written.retain(|_, index| !containers.contains(index));
        self.drop_linearization(&mut written);

        // An indirect /Length is used when it lands on `endstream`.
        let body = self.body;
        for &index in written.values() {
            let Some(stream) = &self.defs[index].stream else {
                continue;
            };
            let resolved = stream
                .length
                .and_then(|id| written.get(&id.number()))
                .and_then(|&target| match self.defs[target].value {
                    Object::Integer(length) => usize::try_from(length).ok(),
                    _ => None,
                })
                .filter(|&length| declared_end(body, stream.data.start, length).is_some());
            if let (Some(length), Some(stream)) = (resolved, &mut self.defs[index].stream) {
                stream.data.end = stream.data.start + length;
                stream.keyword = None;
            }
        }

        let ids: BTreeMap<u32, u16> = written
            .iter()
            .map(|(&number, &i)| (number, self.defs[i].id.generation()))
            .collect();
        let resolve = |value: Option<&Object>| match value {
            Some(Object::Reference(id)) => ids
                .get(&id.number())
                .and_then(|&generation| ObjectId::new(id.number(), generation).ok()),
            _ => None,
        };
        let encrypt = match &encryption {
            None => None,
            Some(Object::Dictionary(encrypt)) => Some(Object::Dictionary(encrypt.clone())),
            encrypt => Some(Object::Reference(
                resolve(encrypt.as_ref())
                    .filter(|id| {
                        matches!(
                            self.defs[written[&id.number()]].value,
                            Object::Dictionary(_)
                        )
                    })
                    .ok_or(RepairError::Invalid("encryption dictionary"))?,
            )),
        };
        let encrypt_dictionary = match &encrypt {
            Some(Object::Dictionary(encrypt)) => Some(encrypt),
            Some(Object::Reference(id)) => match &self.defs[written[&id.number()]].value {
                Object::Dictionary(encrypt) => Some(encrypt),
                _ => None,
            },
            _ => None,
        };
        let (cipher, encrypt_metadata) = encrypt_dictionary.map_or((Cipher::None, true), |d| {
            (
                stream_cipher(d),
                !matches!(
                    d.get(b"EncryptMetadata".as_slice()),
                    Some(Object::Boolean(false))
                ),
            )
        });
        // Revisions 2-4 derive the key from the file identifier, which went
        // with the trailer; the file would only ask for a password forever.
        let revision = encrypt_dictionary.and_then(|d| match d.get(b"R".as_slice()) {
            Some(Object::Integer(r)) => Some(*r),
            _ => None,
        });
        if encryption_from_scan
            && revision.is_none_or(|r| r <= 4)
            && !matches!(self.trailer.get(b"ID".as_slice()), Some(Object::Array(_)))
        {
            return Err(RepairError::Invalid(
                "encryption without the file identifier",
            ));
        }
        for index in written.values().copied().collect::<Vec<_>>() {
            self.settle(index, cipher, encrypt_metadata);
        }
        self.report.streams_length_recovered = written
            .values()
            .filter(|&&i| {
                self.defs[i]
                    .stream
                    .as_ref()
                    .is_some_and(|s| s.keyword.is_some())
            })
            .count();

        let root = resolve(self.trailer.get(b"Root".as_slice()))
            .filter(|id| is_catalog(&self.defs[written[&id.number()]].value));
        let root = match root {
            Some(root) => root,
            None => {
                self.report.catalog_from_scan = true;
                written
                    .values()
                    .filter(|&&i| type_is(&self.defs[i].value, b"Catalog"))
                    .max_by_key(|&&i| self.defs[i].order)
                    .map(|&i| self.defs[i].id)
                    .ok_or(RepairError::Invalid("document catalog"))?
            }
        };
        let mut trailer = Dictionary::new();
        trailer.insert(b"Root".to_vec(), Object::Reference(root));
        match self.trailer.get(b"Info".as_slice()) {
            Some(Object::Dictionary(info)) => {
                trailer.insert(b"Info".to_vec(), Object::Dictionary(info.clone()));
            }
            info => {
                if let Some(id) = resolve(info) {
                    trailer.insert(b"Info".to_vec(), Object::Reference(id));
                }
            }
        }
        if let Some(id @ Object::Array(_)) = self.trailer.get(b"ID".as_slice()) {
            trailer.insert(b"ID".to_vec(), id.clone());
        }
        if let Some(encrypt) = encrypt {
            trailer.insert(b"Encrypt".to_vec(), encrypt);
        }
        for value in trailer.values_mut() {
            self.report.references_repaired += fix_references(value, &ids, false);
        }
        for &index in written.values() {
            let def = &mut self.defs[index];
            // The written /Length is the recovered byte count, in place.
            if let (Some(stream), Object::Dictionary(dictionary)) = (&def.stream, &mut def.value) {
                if let Some(length) = dictionary.get_mut(b"Length".as_slice()) {
                    *length = Object::Integer(i64::try_from(stream.data.len()).unwrap_or(0));
                }
            }
            self.report.references_repaired += fix_references(&mut def.value, &ids, false);
        }
        self.report.page_tree_repaired = self.repair_page_tree(&mut written, root);
        self.report.objects_recovered = written.len();
        let largest = written.keys().next_back().copied().unwrap_or(0);
        trailer.insert(b"Size".to_vec(), Object::Integer(i64::from(largest) + 1));
        let report = &mut self.report;
        report.damaged = !at_start
            || !report.xref_intact
            || report.streams_length_recovered > 0
            || report.catalog_from_scan
            || report.page_tree_repaired;

        let max_depth = self.limits.parse.max_depth;
        let mut writer = Writer::with_style(
            WriteLimits {
                max_output_bytes: self.limits.max_output_bytes,
                max_depth,
                max_objects: usize::MAX,
            },
            SerializeStyle::Canonical,
        );
        // Source key order survives repair, as it does in MuPDF.
        writer.keep_order();
        writer.append(b"%PDF-")?;
        writer.append(&version)?;
        writer.append(b"\n%\xE2\xE3\xCF\xD3\n")?;
        let mut offsets = Vec::new();
        for &index in written.values() {
            let def = &self.defs[index];
            offsets.push((def.id, writer.len()));
            writer
                .append(format!("{} {} obj\n", def.id.number(), def.id.generation()).as_bytes())?;
            match (&def.stream, &def.value) {
                (Some(stream), Object::Dictionary(dictionary)) => {
                    let data = &body[stream.data.clone()];
                    if long_string(&def.value) {
                        let mut dictionary = dictionary.clone();
                        let length =
                            i64::try_from(data.len()).map_err(|_| SerializeError::OutputLimit)?;
                        dictionary.insert(b"Length".to_vec(), Object::Integer(length));
                        write_long(&mut writer, &Object::Dictionary(dictionary), 0, max_depth)?;
                    } else {
                        writer.stream_dictionary(dictionary, data.len())?;
                    }
                    writer.append(b"\nstream\n")?;
                    writer.append(data)?;
                    writer.append(b"\nendstream")?;
                }
                _ => write_value(&mut writer, &def.value, max_depth)?,
            }
            writer.append(b"\nendobj\n")?;
        }
        let xref = writer.len();
        writer.append(b"xref\n0 1\n0000000000 65535 f \n")?;
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
        write_value(&mut writer, &Object::Dictionary(trailer), max_depth)?;
        writer.append(format!("\nstartxref\n{xref}\n%%EOF\n").as_bytes())?;
        Ok(writer.finish())
    }

    /// Drop linearization dictionaries and the hint streams they locate; their
    /// offsets do not apply to the rebuilt file.
    fn drop_linearization(&mut self, written: &mut BTreeMap<u32, usize>) {
        let mut hints = BTreeSet::new();
        written.retain(|_, &mut index| {
            let Object::Dictionary(dictionary) = &self.defs[index].value else {
                return true;
            };
            if dictionary
                .get(b"Linearized".as_slice())
                .is_none_or(|value| matches!(value, Object::Null))
            {
                return true;
            }
            if let Some(Object::Array(hint)) = dictionary.get(b"H".as_slice()) {
                if let Some(Object::Integer(offset)) = hint.first() {
                    hints.extend(usize::try_from(*offset).ok());
                }
            }
            self.report.linearized = true;
            false
        });
        if !hints.is_empty() {
            written.retain(|_, &mut index| {
                let def = &self.defs[index];
                !(def.stream.is_some() && def.order.1 == 0 && hints.contains(&def.order.0))
            });
        }
    }

    /// The written non-stream dictionary `value` refers to, when it does.
    fn node(&self, written: &BTreeMap<u32, usize>, value: &Object) -> Option<(usize, ObjectId)> {
        let Object::Reference(id) = value else {
            return None;
        };
        let &index = written.get(&id.number())?;
        let def = &self.defs[index];
        (def.id == *id && def.stream.is_none() && matches!(def.value, Object::Dictionary(_)))
            .then_some((index, *id))
    }

    fn frame(
        &self,
        written: &BTreeMap<u32, usize>,
        (index, id): (usize, ObjectId),
        media: bool,
    ) -> Frame {
        let empty = Dictionary::new();
        let dictionary = match &self.defs[index].value {
            Object::Dictionary(dictionary) => dictionary,
            _ => &empty,
        };
        let kids = match dictionary.get(b"Kids".as_slice()) {
            Some(Object::Array(kids)) => Some(kids.clone()),
            Some(Object::Reference(kids)) => written.get(&kids.number()).and_then(|&i| {
                match (&self.defs[i].stream, &self.defs[i].value) {
                    (None, Object::Array(kids)) => Some(kids.clone()),
                    _ => None,
                }
            }),
            _ => None,
        };
        Frame {
            index,
            id,
            listed: kids.is_some(),
            kids: kids.unwrap_or_default(),
            next: 0,
            kept: Vec::new(),
            count: 0,
            media: media || present(dictionary, b"MediaBox"),
        }
    }

    /// The page's /Contents when it names anything but streams: only its
    /// stream references, or None to remove it.
    fn contents(
        &self,
        written: &BTreeMap<u32, usize>,
        dictionary: &Dictionary,
    ) -> Option<Option<Object>> {
        let is_stream = |value: &Object| match value {
            Object::Reference(id) => written
                .get(&id.number())
                .is_some_and(|&i| self.defs[i].stream.is_some()),
            _ => false,
        };
        let filter = |items: &[Object]| -> Option<Option<Object>> {
            if items.iter().all(is_stream) {
                return None;
            }
            Some(Some(Object::Array(
                items
                    .iter()
                    .filter(|item| is_stream(item))
                    .cloned()
                    .collect(),
            )))
        };
        match dictionary.get(b"Contents".as_slice())? {
            Object::Null => None,
            value @ Object::Reference(id) if !is_stream(value) => {
                match written.get(&id.number()).map(|&i| &self.defs[i]) {
                    Some(Def {
                        stream: None,
                        value: Object::Array(items),
                        ..
                    }) => filter(items),
                    _ => Some(None),
                }
            }
            Object::Reference(_) => None,
            Object::Array(items) => filter(items),
            _ => Some(None),
        }
    }

    /// Make the page tree pass the strict page walk: drop Kids that are
    /// missing, shared or not page-tree nodes, fix /Parent, /Type and /Count,
    /// keep only stream references in /Contents, and give pages without an
    /// inherited MediaBox US Letter, as MuPDF does. Without a usable /Pages, a
    /// flat tree of the /Type /Page objects is built. True when anything changed.
    fn repair_page_tree(&mut self, written: &mut BTreeMap<u32, usize>, root: ObjectId) -> bool {
        let catalog = written[&root.number()];
        let Object::Dictionary(dictionary) = &self.defs[catalog].value else {
            return false;
        };
        let mut changed = false;
        let pages = dictionary.get(b"Pages".as_slice()).cloned();
        let top = pages
            .and_then(|pages| self.node(written, &pages))
            .filter(|&(i, _)| node_kind(&self.defs[i].value) == Some(true));
        let top = match top {
            Some(top) => top,
            None => {
                let pages: Vec<Object> = written
                    .values()
                    .filter(|&&i| {
                        self.defs[i].stream.is_none() && type_is(&self.defs[i].value, b"Page")
                    })
                    .map(|&i| Object::Reference(self.defs[i].id))
                    .collect();
                let number = written
                    .keys()
                    .next_back()
                    .map_or(1, |n| n.saturating_add(1));
                let Ok(id) = ObjectId::new(number, 0) else {
                    return false;
                };
                if pages.is_empty() || written.contains_key(&number) {
                    return false;
                }
                let mut tree = Dictionary::new();
                tree.insert(b"Type".to_vec(), Object::Name(b"Pages".to_vec()));
                tree.insert(b"Kids".to_vec(), Object::Array(pages));
                self.defs.push(Def {
                    id,
                    order: (usize::MAX, 0),
                    value: Object::Dictionary(tree),
                    stream: None,
                });
                written.insert(number, self.defs.len() - 1);
                changed = true;
                (self.defs.len() - 1, id)
            }
        };
        if let Object::Dictionary(catalog) = &mut self.defs[catalog].value {
            changed |= set(catalog, b"Type", Object::Name(b"Catalog".to_vec()));
            changed |= set(catalog, b"Pages", Object::Reference(top.1));
        }
        if let Object::Dictionary(node) = &mut self.defs[top.0].value {
            changed |= set(node, b"Type", Object::Name(b"Pages".to_vec()));
            changed |= node.remove(b"Parent".as_slice()).is_some();
        }
        let mut visited = BTreeSet::from([top.1.number()]);
        let mut stack = vec![self.frame(written, top, false)];
        while let Some(frame) = stack.last_mut() {
            if frame.next == frame.kids.len() {
                let Some(frame) = stack.pop() else { break };
                let count = i64::try_from(frame.count).unwrap_or(i64::MAX);
                let indirect = match self.defs[frame.index].value {
                    Object::Dictionary(ref node) => match node.get(b"Count".as_slice()) {
                        Some(Object::Reference(id)) => {
                            written.get(&id.number()).map(|&i| &self.defs[i].value)
                        }
                        _ => None,
                    },
                    _ => None,
                };
                let counted = indirect == Some(&Object::Integer(count));
                if let Object::Dictionary(node) = &mut self.defs[frame.index].value {
                    if !frame.listed || frame.kept.len() != frame.kids.len() {
                        node.insert(b"Kids".to_vec(), Object::Array(frame.kept));
                        changed = true;
                    }
                    if !counted {
                        changed |= set(node, b"Count", Object::Integer(count));
                    }
                }
                if let Some(parent) = stack.last_mut() {
                    parent.kept.push(Object::Reference(frame.id));
                    parent.count += frame.count;
                }
                continue;
            }
            let kid = frame.kids[frame.next].clone();
            frame.next += 1;
            let (parent, media, depth) = (frame.id, frame.media, stack.len());
            let target = self
                .node(written, &kid)
                .filter(|(_, id)| !visited.contains(&id.number()));
            let Some((index, id)) = target else {
                changed = true;
                continue;
            };
            let kind = node_kind(&self.defs[index].value);
            let contents = match (&self.defs[index].value, kind) {
                (Object::Dictionary(page), Some(false)) => self.contents(written, page),
                _ => None,
            };
            let Object::Dictionary(dictionary) = &mut self.defs[index].value else {
                continue;
            };
            match kind {
                Some(true) if depth < 256 => {
                    visited.insert(id.number());
                    changed |= set(dictionary, b"Type", Object::Name(b"Pages".to_vec()));
                    changed |= set(dictionary, b"Parent", Object::Reference(parent));
                    stack.push(self.frame(written, (index, id), media));
                }
                Some(false) => {
                    visited.insert(id.number());
                    changed |= set(dictionary, b"Type", Object::Name(b"Page".to_vec()));
                    changed |= set(dictionary, b"Parent", Object::Reference(parent));
                    if !media && !present(dictionary, b"MediaBox") {
                        let letter = [0, 0, 612, 792].map(Object::Integer).to_vec();
                        dictionary.insert(b"MediaBox".to_vec(), Object::Array(letter));
                        changed = true;
                    }
                    match contents {
                        Some(Some(contents)) => {
                            dictionary.insert(b"Contents".to_vec(), contents);
                            changed = true;
                        }
                        Some(None) => {
                            dictionary.remove(b"Contents".as_slice());
                            changed = true;
                        }
                        None => {}
                    }
                    if let Some(frame) = stack.last_mut() {
                        frame.kept.push(kid);
                        frame.count += 1;
                    }
                }
                _ => changed = true,
            }
        }
        changed
    }

    /// Add an object stream's members as definitions at its position, parsing
    /// each distinct offset once. False when the container cannot be decoded.
    fn expand(&mut self, index: usize) -> Result<bool, RepairError> {
        let container = &self.defs[index];
        let (Object::Dictionary(dictionary), Some(stream)) = (&container.value, &container.stream)
        else {
            return Ok(false);
        };
        let position = container.order.0;
        let integer = |key: &[u8]| match dictionary.get(key) {
            Some(Object::Integer(n)) => usize::try_from(*n).ok(),
            _ => None,
        };
        let (Some(count), Some(first), Some(specs)) =
            (integer(b"N"), integer(b"First"), flate_specs(dictionary))
        else {
            return Ok(false);
        };
        let body = self.body;
        let start = stream.data.start;
        let ends = match stream.keyword {
            Some(keyword) => stream_ends(body, start, keyword),
            None => vec![stream.data.end],
        };
        let mut bytes = None;
        for end in ends {
            match self.decode(&body[start..end], &specs) {
                Decoded::Bytes(decoded) => {
                    bytes = Some(decoded);
                    if let Some(stream) = &mut self.defs[index].stream {
                        stream.data.end = end;
                    }
                    break;
                }
                Decoded::Failed => {}
                Decoded::Limit => return Err(RepairError::LimitExceeded("object-stream bytes")),
            }
        }
        let Some(bytes) = bytes else {
            return Ok(false);
        };
        let first = first.min(bytes.len());
        let remaining = self.limits.max_objects.saturating_sub(self.defs.len());
        let numbers: Vec<usize> = bytes[..first]
            .split(|&b| is_ws(b))
            .filter(|word| !word.is_empty())
            .map_while(|word| std::str::from_utf8(word).ok()?.parse().ok())
            .take(count.min(remaining.saturating_add(1)).saturating_mul(2))
            .collect();
        if numbers.len() / 2 > remaining {
            return Err(RepairError::LimitExceeded("objects"));
        }
        let mut members: Vec<(usize, usize, u32)> = Vec::new();
        for (i, pair) in numbers.chunks_exact(2).enumerate() {
            let number = u32::try_from(pair[0]).ok();
            let start = first.checked_add(pair[1]).filter(|&s| s < bytes.len());
            match (number, start) {
                (Some(number), Some(start)) => members.push((start, i + 1, number)),
                _ => self.report.objects_dropped += 1,
            }
        }
        // Each region runs to the next distinct offset, so members are parsed
        // at most once per decoded byte; a repeated offset is an alias.
        members.sort_unstable();
        for (k, &(start, member, number)) in members.iter().enumerate() {
            if k > 0 && members[k - 1].0 == start {
                self.report.objects_dropped += 1;
                continue;
            }
            let end = members[k + 1..]
                .iter()
                .map(|m| m.0)
                .find(|&s| s > start)
                .unwrap_or(bytes.len());
            let parsed = self.parse(&bytes[start..end])?;
            match (ObjectId::new(number, 0), parsed) {
                (Ok(id), Ok((value, _))) => self.define(Def {
                    id,
                    order: (position, member),
                    value,
                    stream: None,
                })?,
                _ => self.report.objects_dropped += 1,
            }
        }
        Ok(true)
    }
}

/// Data range and the position after `endstream` (or the end of input).
fn stream_data(body: &[u8], dictionary: &Dictionary, mut start: usize) -> (Stream, usize) {
    if body[start..].starts_with(b"\r\n") {
        start += 2;
    } else if matches!(body.get(start), Some(b'\n' | b'\r')) {
        start += 1;
    }
    let (declared, length) = match dictionary.get(b"Length".as_slice()) {
        Some(Object::Integer(n)) => (usize::try_from(*n).ok(), None),
        Some(Object::Reference(id)) => (None, Some(*id)),
        _ => (None, None),
    };
    if let Some(declared) = declared {
        if let Some(after) = declared_end(body, start, declared) {
            let stream = Stream {
                data: start..start + declared,
                keyword: None,
                length,
            };
            return (stream, after);
        }
    }
    match body[start..].windows(9).position(|w| w == b"endstream") {
        Some(found) => {
            let keyword = start + found;
            let stream = Stream {
                data: start..stream_ends(body, start, keyword)[0],
                keyword: Some(keyword),
                length,
            };
            (stream, keyword + 9)
        }
        None => {
            let stream = Stream {
                data: start..stream_ends(body, start, body.len())[0],
                keyword: Some(body.len()),
                length,
            };
            (stream, body.len())
        }
    }
}

/// Possible data ends before `endstream`, shortest first: without the EOL in
/// front of it, without only the LF of a CRLF, and up to the keyword.
fn stream_ends(body: &[u8], start: usize, keyword: usize) -> Vec<usize> {
    let mut ends = vec![keyword];
    let mut end = keyword;
    if end > start && body[end - 1] == b'\n' {
        end -= 1;
        ends.push(end);
    }
    if end > start && body[end - 1] == b'\r' {
        end -= 1;
        ends.push(end);
    }
    ends.reverse();
    ends
}

/// Where `endstream` ends if the data has exactly `length` bytes.
fn declared_end(body: &[u8], start: usize, length: usize) -> Option<usize> {
    let end = start.checked_add(length).filter(|&end| end <= body.len())?;
    let end = skip_trivia(body, end, body.len().min(end + GAP));
    body[end..].starts_with(b"endstream").then_some(end + 9)
}

/// The next object header, or `trailer` keyword before `<<`, starting at or
/// after `from`.
fn next_event(body: &[u8], from: usize) -> Option<Event> {
    let delimited = |end: usize| body.get(end).is_none_or(|&b| is_delimiter(b));
    let mut i = from;
    while i + 3 <= body.len() {
        let rest = &body[i..];
        if rest.starts_with(b"obj") && delimited(i + 3) {
            if let Some((start, id)) = object_header(body, i).filter(|&(start, _)| start >= from) {
                return Some(Event {
                    start,
                    keyword_end: i + 3,
                    kind: Kind::Object(id),
                });
            }
        } else if rest.starts_with(b"trailer")
            && (i == 0 || is_ws(body[i - 1]))
            && delimited(i + 7)
            && body[skip_trivia(body, i + 7, body.len().min(i + 7 + GAP))..].starts_with(b"<<")
        {
            return Some(Event {
                start: i,
                keyword_end: i + 7,
                kind: Kind::Trailer,
            });
        }
        i += 1;
    }
    None
}

/// The `N G` before an `obj` keyword at `keyword`.
fn object_header(body: &[u8], keyword: usize) -> Option<(usize, ObjectId)> {
    let skip_ws = |mut j: usize| {
        while j > 0 && is_ws(body[j - 1]) {
            j -= 1;
        }
        j
    };
    let digits = |end: usize, max: usize| {
        let mut j = end;
        while j > 0 && end - j <= max && body[j - 1].is_ascii_digit() {
            j -= 1;
        }
        j
    };
    let generation_end = skip_ws(keyword);
    let generation_start = digits(generation_end, 5);
    let number_end = skip_ws(generation_start);
    let number_start = digits(number_end, 10);
    let valid = generation_end < keyword
        && generation_start < generation_end
        && number_end < generation_start
        && number_start < number_end
        && (number_start == 0 || is_delimiter(body[number_start - 1]));
    if !valid {
        return None;
    }
    let decimal = |range: Range<usize>| -> Option<u64> {
        std::str::from_utf8(&body[range]).ok()?.parse().ok()
    };
    let number = u32::try_from(decimal(number_start..number_end)?).ok()?;
    let generation = u16::try_from(decimal(generation_start..generation_end)?).ok()?;
    Some((number_start, ObjectId::new(number, generation).ok()?))
}

fn skip_trivia(body: &[u8], mut p: usize, bound: usize) -> usize {
    while p < bound {
        if is_ws(body[p]) {
            p += 1;
        } else if body[p] == b'%' {
            while p < bound && !matches!(body[p], b'\r' | b'\n') {
                p += 1;
            }
        } else {
            break;
        }
    }
    p
}

/// The first non-whitespace position within `GAP` bytes of `at`.
fn skip_whitespace(body: &[u8], at: usize) -> Option<usize> {
    let bound = body.len().min(at.checked_add(GAP)?);
    let mut p = at;
    while p < bound && is_ws(body[p]) {
        p += 1;
    }
    (p < body.len()).then_some(p)
}

/// The unsigned decimal at `at` and the position after it.
fn integer_at(body: &[u8], at: usize) -> Option<(usize, usize)> {
    let digits = body
        .get(at..)?
        .iter()
        .take(20)
        .take_while(|b| b.is_ascii_digit())
        .count();
    let value = std::str::from_utf8(&body[at..at + digits])
        .ok()?
        .parse()
        .ok()?;
    Some((value, at + digits))
}

/// The header position, the input from the `%PDF-` header found in the
/// first kilobyte (all of it when there is none), and the version to write.
fn header(input: &[u8]) -> (usize, &[u8], [u8; 3]) {
    let window = &input[..input.len().min(1024)];
    let Some(at) = window.windows(5).position(|w| w == b"%PDF-") else {
        return (0, input, *b"1.7");
    };
    let version = match input.get(at + 5..at + 8) {
        Some(
            v @ (b"1.0" | b"1.1" | b"1.2" | b"1.3" | b"1.4" | b"1.5" | b"1.6" | b"1.7" | b"2.0"),
        ) => [v[0], v[1], v[2]],
        _ => *b"1.7",
    };
    (at, &input[at..], version)
}

/// Direct FlateDecode (with optional predictor) or no filter, as object
/// streams use; `None` for anything else.
fn flate_specs(dictionary: &Dictionary) -> Option<Vec<FilterSpec>> {
    // An array must hold exactly one entry; `None` rejects longer chains.
    let single = |key: &[u8]| match dictionary.get(key) {
        Some(Object::Array(values)) => match values.as_slice() {
            [value] => Some(Some(value)),
            _ => None,
        },
        value => Some(value),
    };
    match single(b"Filter")? {
        None | Some(Object::Null) => return Some(Vec::new()),
        Some(Object::Name(name)) if name == b"FlateDecode" => {}
        _ => return None,
    }
    let mut predictor = PredictorParams::default();
    match single(b"DecodeParms")? {
        None | Some(Object::Null) => {}
        Some(Object::Dictionary(parameters)) => {
            let integer = |key: &[u8]| match parameters.get(key) {
                Some(Object::Integer(n)) => usize::try_from(*n).ok(),
                _ => None,
            };
            if let Some(value) = integer(b"Predictor") {
                predictor.predictor = u8::try_from(value).ok()?;
            }
            if let Some(value) = integer(b"Colors") {
                predictor.colors = value;
            }
            if let Some(value) = integer(b"BitsPerComponent") {
                predictor.bits_per_component = u8::try_from(value).ok()?;
            }
            if let Some(value) = integer(b"Columns") {
                predictor.columns = value;
            }
        }
        _ => return None,
    }
    Some(vec![FilterSpec {
        filter: Filter::Flate,
        predictor,
    }])
}

/// How the security handler encrypts ordinary streams.
fn stream_cipher(encrypt: &Dictionary) -> Cipher {
    if !matches!(encrypt.get(b"V".as_slice()), Some(Object::Integer(4 | 5))) {
        return Cipher::Rc4;
    }
    let filter = match encrypt.get(b"StmF".as_slice()) {
        Some(Object::Name(name)) => name.as_slice(),
        _ => b"Identity",
    };
    let method = match encrypt.get(b"CF".as_slice()) {
        Some(Object::Dictionary(filters)) => match filters.get(filter) {
            Some(Object::Dictionary(filter)) => filter.get(b"CFM".as_slice()),
            _ => None,
        },
        _ => None,
    };
    match method {
        Some(Object::Name(name)) if name == b"AESV2" || name == b"AESV3" => Cipher::Aes,
        Some(Object::Name(name)) if name == b"V2" => Cipher::Rc4,
        _ => Cipher::None,
    }
}

fn dangling(value: &Object, ids: &BTreeMap<u32, u16>) -> bool {
    matches!(value, Object::Reference(id) if !ids.contains_key(&id.number()))
}

/// Drop dangling references (null inside arrays other than object lists) and
/// point stale generations at the surviving object. Returns the number changed.
fn fix_references(value: &mut Object, ids: &BTreeMap<u32, u16>, list: bool) -> usize {
    match value {
        Object::Reference(id) => match ids.get(&id.number()) {
            Some(&generation) if generation == id.generation() => 0,
            Some(&generation) => {
                if let Ok(fixed) = ObjectId::new(id.number(), generation) {
                    *id = fixed;
                }
                1
            }
            None => {
                *value = Object::Null;
                1
            }
        },
        Object::Array(items) => {
            let before = items.len();
            if list {
                items.retain(|item| !dangling(item, ids));
            }
            before - items.len()
                + items
                    .iter_mut()
                    .map(|item| fix_references(item, ids, list))
                    .sum::<usize>()
        }
        Object::Dictionary(entries) => {
            let before = entries.len();
            entries.retain(|_, item| !dangling(item, ids));
            before - entries.len()
                + entries
                    .iter_mut()
                    .map(|(key, item)| fix_references(item, ids, LISTS.contains(&key.as_slice())))
                    .sum::<usize>()
        }
        _ => 0,
    }
}

/// Values the parser charged for `value`: one per node and one per key.
fn cost(value: &Object) -> usize {
    match value {
        Object::Array(items) => 1 + items.iter().map(cost).sum::<usize>(),
        Object::Dictionary(entries) => 1 + entries.values().map(|v| 1 + cost(v)).sum::<usize>(),
        _ => 1,
    }
}

fn long_string(value: &Object) -> bool {
    match value {
        Object::String(bytes) => bytes.len() > LONG_STRING,
        Object::Array(items) => items.iter().any(long_string),
        Object::Dictionary(entries) => entries.values().any(long_string),
        _ => false,
    }
}

fn write_value(
    writer: &mut Writer,
    value: &Object,
    max_depth: usize,
) -> Result<(), SerializeError> {
    if long_string(value) {
        write_long(writer, value, 0, max_depth)
    } else {
        writer.value(value, 0)
    }
}

/// The serializer's canonical form, except that long strings are literal.
fn write_long(
    writer: &mut Writer,
    value: &Object,
    depth: usize,
    max_depth: usize,
) -> Result<(), SerializeError> {
    match value {
        Object::String(bytes) if bytes.len() > LONG_STRING => writer.append(&literal(bytes)),
        Object::Array(items) => {
            if depth >= max_depth.min(256) {
                return Err(SerializeError::DepthLimit);
            }
            writer.append(b"[")?;
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    writer.append(b" ")?;
                }
                write_long(writer, item, depth + 1, max_depth)?;
            }
            writer.append(b"]")
        }
        Object::Dictionary(entries) => {
            if depth >= max_depth.min(256) {
                return Err(SerializeError::DepthLimit);
            }
            writer.append(b"<<")?;
            for (key, item) in entries {
                writer.append(b" ")?;
                writer.value(&Object::Name(key.clone()), depth + 1)?;
                writer.append(b" ")?;
                write_long(writer, item, depth + 1, max_depth)?;
            }
            writer.append(b" >>")
        }
        _ => writer.value(value, depth),
    }
}

/// A literal string that reads back as `bytes`. Balanced parentheses stay
/// bare; the others, backslashes and CRs are escaped.
fn literal(bytes: &[u8]) -> Vec<u8> {
    let mut escape = vec![false; bytes.len()];
    let mut depth = 0usize;
    for (i, &byte) in bytes.iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' if depth == 0 => escape[i] = true,
            b')' => depth -= 1,
            _ => {}
        }
    }
    let mut open = 0usize;
    for (i, &byte) in bytes.iter().enumerate().rev() {
        match byte {
            b')' if !escape[i] => open += 1,
            b'(' if open == 0 => escape[i] = true,
            b'(' => open -= 1,
            _ => {}
        }
    }
    let mut out = Vec::with_capacity(bytes.len() + 2);
    out.push(b'(');
    for (i, &byte) in bytes.iter().enumerate() {
        match byte {
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'(' | b')' if escape[i] => out.extend_from_slice(&[b'\\', byte]),
            _ => out.push(byte),
        }
    }
    out.push(b')');
    out
}

fn open_string(error: &ParseError) -> bool {
    matches!(&error.kind, ParseErrorKind::Lex(error) if error.kind == LexErrorKind::UnterminatedString)
}

fn set(dictionary: &mut Dictionary, key: &[u8], value: Object) -> bool {
    if dictionary.get(key) == Some(&value) {
        return false;
    }
    dictionary.insert(key.to_vec(), value);
    true
}

fn present(dictionary: &Dictionary, key: &[u8]) -> bool {
    dictionary
        .get(key)
        .is_some_and(|value| !matches!(value, Object::Null))
}

/// Some(true) for a Pages node, Some(false) for a page, None for neither. As in
/// MuPDF, an untyped dictionary is a node when it has Kids.
fn node_kind(value: &Object) -> Option<bool> {
    let Object::Dictionary(dictionary) = value else {
        return None;
    };
    match dictionary.get(b"Type".as_slice()) {
        Some(Object::Name(name)) if name == b"Pages" => Some(true),
        Some(Object::Name(name)) if name == b"Page" => Some(false),
        None | Some(Object::Null) => Some(dictionary.contains_key(b"Kids".as_slice())),
        _ => None,
    }
}

fn is_object_stream(def: &Def) -> bool {
    def.stream.is_some() && type_is(&def.value, b"ObjStm")
}

fn type_is(value: &Object, name: &[u8]) -> bool {
    matches!(value, Object::Dictionary(d)
        if matches!(d.get(b"Type".as_slice()), Some(Object::Name(n)) if n == name))
}

fn is_catalog(value: &Object) -> bool {
    type_is(value, b"Catalog")
        || matches!(value, Object::Dictionary(d) if d.contains_key(b"Pages".as_slice()))
}

fn is_ws(byte: u8) -> bool {
    matches!(byte, 0 | b'\t' | b'\n' | 12 | b'\r' | b' ')
}

fn is_delimiter(byte: u8) -> bool {
    is_ws(byte) || b"()<>[]{}/%".contains(&byte)
}
