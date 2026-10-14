// SPDX-License-Identifier: AGPL-3.0-or-later
// Repair behavior references MuPDF 1.28.1 source/pdf/pdf-repair.c,
// Copyright (C) 2004-2026 Artifex Software, Inc. See NOTICE.
//! Tolerant rebuild of damaged PDFs into one clean, strictly parseable revision.
//!
//! Like MuPDF's repair, the scan ignores every xref and finds each `N G obj`
//! after the header. Values parse leniently (duplicate keys keep the last
//! value, a missing `endobj` is tolerated) and a stream whose /Length does not
//! land on `endstream` ends at the next `endstream`. Later definitions of an
//! object number override earlier ones, whatever their generation; members of
//! a winning object stream compete at the container's position. Root, Info, ID
//! and Encrypt come from the last trailer or xref-stream dictionary holding
//! each; without a usable Root, the last /Type /Catalog object is used.
//! References to missing objects become null and references with a stale
//! generation are pointed at the surviving one, as MuPDF resolves them.
//!
//! The output has a classic xref table. Xref streams and object-stream
//! containers are dropped. Encrypted objects are copied verbatim, keeping
//! /Encrypt, so the security handler decrypts them on open; object streams in
//! encrypted files are refused. Every byte is scanned a bounded number of
//! times: each value parse stops at the next object header, and stream data
//! is skipped once read.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Range;

use crate::document::{Document, DocumentError, DocumentErrorKind, DocumentLimits};
use crate::object::{Object, ObjectId};
use crate::parser::{DuplicateKeyPolicy, ParseErrorKind, ParseLimitKind, ParseLimits, Parser};
use crate::serialize::{SerializeError, WriteLimits, Writer};
use crate::stream::{
    Filter, FilterSpec, PredictorParams, StreamErrorKind, StreamLimits,
    decode_filter_specs_with_usage,
};

type Dictionary = BTreeMap<Vec<u8>, Object>;

#[derive(Clone, Copy, Debug)]
pub struct RepairLimits {
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    /// Object definitions kept, including superseded ones and object-stream members.
    pub max_objects: usize,
    /// Parsed values across all definitions and trailers, counting dictionary keys.
    pub max_values: usize,
    pub max_object_streams: usize,
    /// Decoded object-stream bytes across all containers.
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepairReport {
    /// Objects written to the repaired file.
    pub objects_recovered: usize,
    /// Definitions superseded by a later definition of the same object number.
    pub objects_overridden: usize,
    /// Object headers whose value could not be parsed, object-stream members
    /// that could not be parsed, and containers that could not be decoded.
    pub objects_dropped: usize,
    pub object_streams_expanded: usize,
    /// Written streams whose length came from the `endstream` search.
    pub streams_length_recovered: usize,
    /// Dangling references removed (null in arrays) or stale generations fixed.
    pub references_repaired: usize,
    /// No trailer Root named a catalog; the last /Type /Catalog object was used.
    pub catalog_from_scan: bool,
    /// Why the strict open failed, when the caller records it.
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
    /// open the input (password, or the repaired file under the caller's limits).
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

/// The bytes to open strictly, and the repair report when they were rebuilt.
#[derive(Debug)]
pub struct TolerantPdf {
    pub bytes: Vec<u8>,
    pub repair: Option<RepairReport>,
}

/// Open strictly first; rebuild only when that fails for a reason other than
/// the password. The returned bytes open strictly with `limits` and `password`.
pub fn open_tolerant(
    input: Vec<u8>,
    limits: DocumentLimits,
    password: &[u8],
) -> Result<TolerantPdf, RepairError> {
    if input.len() > limits.max_input_bytes {
        return Err(RepairError::LimitExceeded("input bytes"));
    }
    let strict = match Document::open_with_password(&input, limits, password) {
        Ok(_) => {
            return Ok(TolerantPdf {
                bytes: input,
                repair: None,
            });
        }
        Err(error) if matches!(error.kind, DocumentErrorKind::PasswordRequired) => {
            return Err(RepairError::Document(error));
        }
        Err(error) => error,
    };
    let mut repaired = rebuild(
        &input,
        RepairLimits {
            max_input_bytes: limits.max_input_bytes,
            parse: limits.parse,
            ..RepairLimits::default()
        },
    )?;
    repaired.report.strict_error = Some(strict.to_string());
    Document::open_with_password(&repaired.bytes, limits, password)
        .map_err(RepairError::Document)?;
    Ok(TolerantPdf {
        bytes: repaired.bytes,
        repair: Some(repaired.report),
    })
}

/// Rebuild `input` from a full object scan. The result opens with
/// `Document::open` (or needs a password); offsets are relative to the header,
/// and any bytes before it are dropped.
pub fn rebuild(input: &[u8], limits: RepairLimits) -> Result<RepairedPdf, RepairError> {
    if input.len() > limits.max_input_bytes {
        return Err(RepairError::LimitExceeded("input bytes"));
    }
    let (body, version) = header(input);
    let mut scan = Scan {
        body,
        limits,
        defs: Vec::new(),
        trailer: BTreeMap::new(),
        values: 0,
        decoded: 0,
        report: RepairReport::default(),
    };
    scan.run()?;
    let bytes = scan.finish(version)?;
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
    length: Option<ObjectId>,
    recovered: bool,
}

struct Def {
    id: ObjectId,
    /// File position, then 1-based member index within an object stream.
    order: (usize, usize),
    value: Object,
    stream: Option<Stream>,
}

struct Scan<'a> {
    body: &'a [u8],
    limits: RepairLimits,
    defs: Vec<Def>,
    trailer: BTreeMap<&'static [u8], Object>,
    values: usize,
    decoded: usize,
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

impl Scan<'_> {
    fn run(&mut self) -> Result<(), RepairError> {
        let body = self.body;
        let mut next = next_event(body, 0);
        while let Some(event) = next {
            let following = next_event(body, event.keyword_end);
            let bound = following.as_ref().map_or(body.len(), |e| e.start);
            let end = match event.kind {
                Kind::Object(id) => self.object(id, event.start, event.keyword_end, bound)?,
                Kind::Trailer => self.trailer_dictionary(event.keyword_end, bound)?,
            };
            next = match (end, following) {
                (Some(end), Some(following)) if end > following.start => next_event(body, end),
                (_, following) => following,
            };
        }
        Ok(())
    }

    /// Parse one value; `None` when it is absent or malformed.
    fn parse(&mut self, input: &[u8]) -> Result<Option<(Object, usize)>, RepairError> {
        let remaining = self.limits.max_values.saturating_sub(self.values);
        let mut limits = self.limits.parse;
        limits.max_objects = limits.max_objects.min(remaining);
        let parsed = Parser::new(input, limits)
            .with_duplicate_key_policy(DuplicateKeyPolicy::KeepLast)
            .parse_next();
        match parsed {
            Ok(Some(parsed)) => {
                self.values += cost(&parsed.object);
                Ok(Some((parsed.object, parsed.span.end)))
            }
            Err(error)
                if matches!(
                    error.kind,
                    ParseErrorKind::LimitExceeded(ParseLimitKind::Objects)
                ) && remaining < self.limits.parse.max_objects =>
            {
                Err(RepairError::LimitExceeded("values"))
            }
            _ => Ok(None),
        }
    }

    fn object(
        &mut self,
        id: ObjectId,
        start: usize,
        value_start: usize,
        bound: usize,
    ) -> Result<Option<usize>, RepairError> {
        let body = self.body;
        let Some((value, consumed)) = self.parse(&body[value_start..bound])? else {
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
        let structural = |name: &[u8]| {
            stream.is_some()
                && matches!(&value, Object::Dictionary(d)
                    if matches!(d.get(b"Type".as_slice()), Some(Object::Name(n)) if n == name))
        };
        if structural(b"XRef") {
            if let Object::Dictionary(dictionary) = &value {
                self.harvest(dictionary);
            }
            return Ok(Some(end));
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
        start: usize,
        bound: usize,
    ) -> Result<Option<usize>, RepairError> {
        let body = self.body;
        let Some((Object::Dictionary(dictionary), consumed)) = self.parse(&body[start..bound])?
        else {
            return Ok(None);
        };
        self.harvest(&dictionary);
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

    fn finish(&mut self, version: [u8; 3]) -> Result<Vec<u8>, RepairError> {
        let encrypted = self.trailer.contains_key(b"Encrypt".as_slice());
        let containers: BTreeSet<usize> = self
            .winners()
            .into_values()
            .filter(|&i| self.defs[i].stream.is_some() && type_is(&self.defs[i].value, b"ObjStm"))
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
        let mut written = self.winners();
        self.report.objects_overridden = self.defs.len() - written.len();
        written.retain(|_, index| !containers.contains(index));
        self.report.objects_recovered = written.len();

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
                stream.recovered = false;
            }
        }
        self.report.streams_length_recovered = written
            .values()
            .filter(|&&i| self.defs[i].stream.as_ref().is_some_and(|s| s.recovered))
            .count();

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
        let mut trailer = BTreeMap::<&[u8], Object>::new();
        trailer.insert(b"Root", Object::Reference(root));
        match self.trailer.get(b"Info".as_slice()) {
            Some(Object::Dictionary(info)) => {
                trailer.insert(b"Info", Object::Dictionary(info.clone()));
            }
            info => {
                if let Some(id) = resolve(info) {
                    trailer.insert(b"Info", Object::Reference(id));
                }
            }
        }
        if let Some(id @ Object::Array(_)) = self.trailer.get(b"ID".as_slice()) {
            trailer.insert(b"ID", id.clone());
        }
        match self.trailer.get(b"Encrypt".as_slice()) {
            None => {}
            Some(Object::Dictionary(encrypt)) => {
                trailer.insert(b"Encrypt", Object::Dictionary(encrypt.clone()));
            }
            encrypt => {
                let id = resolve(encrypt)
                    .filter(|id| {
                        matches!(
                            self.defs[written[&id.number()]].value,
                            Object::Dictionary(_)
                        )
                    })
                    .ok_or(RepairError::Invalid("encryption dictionary"))?;
                trailer.insert(b"Encrypt", Object::Reference(id));
            }
        }
        for value in trailer.values_mut() {
            self.report.references_repaired += fix_references(value, &ids);
        }
        for &index in written.values() {
            let def = &mut self.defs[index];
            if let (Some(_), Object::Dictionary(dictionary)) = (&def.stream, &mut def.value) {
                dictionary.remove(b"Length".as_slice());
            }
            self.report.references_repaired += fix_references(&mut def.value, &ids);
        }
        let largest = written.keys().next_back().copied().unwrap_or(0);
        trailer.insert(b"Size", Object::Integer(i64::from(largest) + 1));

        let mut writer = Writer::new(WriteLimits {
            max_output_bytes: self.limits.max_output_bytes,
            max_depth: self.limits.parse.max_depth,
            max_objects: usize::MAX,
        });
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
                    writer.stream_dictionary(dictionary, data.len())?;
                    writer.append(b"\nstream\n")?;
                    writer.append(data)?;
                    writer.append(b"\nendstream")?;
                }
                _ => writer.value(&def.value, 0)?,
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
        writer.dictionary_entries(trailer.iter().map(|(key, value)| (*key, value)))?;
        writer.append(format!("\nstartxref\n{xref}\n%%EOF\n").as_bytes())?;
        Ok(writer.finish())
    }

    /// Add an object stream's members as definitions at its position. False
    /// when the container cannot be decoded.
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
        let data = &body[stream.data.clone()];
        let remaining = self.limits.max_decoded_bytes.saturating_sub(self.decoded);
        let decoded = match decode_filter_specs_with_usage(
            data,
            &specs,
            StreamLimits {
                max_encoded_bytes: data.len(),
                max_decoded_bytes: remaining,
                max_filters: 1,
                max_total_decoded_bytes: remaining,
            },
        ) {
            Ok(decoded) => decoded,
            Err(error)
                if matches!(
                    error.kind,
                    StreamErrorKind::DecodedOutputLimit { .. }
                        | StreamErrorKind::TotalOutputLimit { .. }
                ) =>
            {
                return Err(RepairError::LimitExceeded("object-stream bytes"));
            }
            Err(_) => return Ok(false),
        };
        self.decoded += decoded.usage.total_decoded_bytes;
        let bytes = decoded.bytes;
        let first = first.min(bytes.len());
        let numbers: Vec<usize> = bytes[..first]
            .split(|&b| is_ws(b))
            .filter(|word| !word.is_empty())
            .map_while(|word| std::str::from_utf8(word).ok()?.parse().ok())
            .take(count.saturating_mul(2))
            .collect();
        let members: Vec<(usize, u32, usize)> = numbers
            .chunks_exact(2)
            .enumerate()
            .filter_map(|(i, pair)| {
                let number = u32::try_from(pair[0]).ok()?;
                let start = first.checked_add(pair[1]).filter(|&s| s < bytes.len())?;
                Some((i + 1, number, start))
            })
            .collect();
        let mut starts: Vec<usize> = members.iter().map(|m| m.2).collect();
        starts.sort_unstable();
        starts.dedup();
        for (member, number, start) in members {
            let end = starts
                .get(starts.partition_point(|&s| s <= start))
                .copied()
                .unwrap_or(bytes.len());
            let parsed = self.parse(&bytes[start..end])?;
            match (ObjectId::new(number, 0), parsed) {
                (Ok(id), Some((value, _))) => self.define(Def {
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
                length,
                recovered: false,
            };
            return (stream, after);
        }
    }
    let (end, after) = match body[start..].windows(9).position(|w| w == b"endstream") {
        Some(found) => {
            let mut end = start + found;
            if end > start && body[end - 1] == b'\n' {
                end -= 1;
            }
            if end > start && body[end - 1] == b'\r' {
                end -= 1;
            }
            (end, start + found + 9)
        }
        None => (body.len(), body.len()),
    };
    let stream = Stream {
        data: start..end,
        length,
        recovered: true,
    };
    (stream, after)
}

/// Where `endstream` ends if the data has exactly `length` bytes.
fn declared_end(body: &[u8], start: usize, length: usize) -> Option<usize> {
    let end = start.checked_add(length).filter(|&end| end <= body.len())?;
    let end = skip_trivia(body, end, body.len().min(end + GAP));
    body[end..].starts_with(b"endstream").then_some(end + 9)
}

/// The next object header or `trailer` keyword starting at or after `from`.
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
        } else if rest.starts_with(b"trailer") && (i == 0 || is_ws(body[i - 1])) && delimited(i + 7)
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

/// Trivia allowed between a stream's data and `endstream`, or after it before
/// `endobj`; bounding it keeps the scan linear.
const GAP: usize = 256;

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

/// The input from the `%PDF-` header found in the first kilobyte (all of it
/// when there is none) and the version to write.
fn header(input: &[u8]) -> (&[u8], [u8; 3]) {
    let window = &input[..input.len().min(1024)];
    let Some(at) = window.windows(5).position(|w| w == b"%PDF-") else {
        return (input, *b"1.7");
    };
    let version = match input.get(at + 5..at + 8) {
        Some(
            v @ (b"1.0" | b"1.1" | b"1.2" | b"1.3" | b"1.4" | b"1.5" | b"1.6" | b"1.7" | b"2.0"),
        ) => [v[0], v[1], v[2]],
        _ => *b"1.7",
    };
    (&input[at..], version)
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

/// Drop dangling references (null inside arrays) and point stale generations
/// at the surviving object. Returns the number changed.
fn fix_references(value: &mut Object, ids: &BTreeMap<u32, u16>) -> usize {
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
        Object::Array(items) => items.iter_mut().map(|v| fix_references(v, ids)).sum(),
        Object::Dictionary(entries) => {
            let before = entries.len();
            entries.retain(
                |_, v| !matches!(v, Object::Reference(id) if !ids.contains_key(&id.number())),
            );
            let removed = before - entries.len();
            removed
                + entries
                    .values_mut()
                    .map(|v| fix_references(v, ids))
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
