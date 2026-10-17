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
//! objects, it selects them, as a reader that does not need repair would;
//! objects appended after that chain's `startxref` without an xref of their
//! own are then in no revision, as in MuPDF.
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
//! /Encrypt, so the security handler decrypts them on open. In an encrypted
//! file, object streams are decrypted with the key of the first password
//! that authenticates (`rebuild_with_passwords`; the empty one, as MuPDF
//! tries, by default) and their members written as top-level objects with
//! their strings encrypted under their own object keys, so the reader
//! decrypts each once. When no password authenticates, the output holds the
//! structure only (`RepairReport::locked`): the containers as they are, no
//! members, references and page tree untouched. Lexical damage MuPDF reads
//! through is read through: values parse as `Parser::with_lenient` reads
//! them (a damaged number or unknown keyword is null, a non-hex digit is 0,
//! a '#' in a name without two hex digits stands for itself), Flate data runs to the end of its zlib stream (a truncated
//! one keeps what it produced), /Fl names FlateDecode in object and xref
//! streams, and a container the xref names is read whatever its /Type.
//! Every byte is scanned a bounded number of times: each value parse stops at
//! the next object header (parses retried past one share a budget of one
//! pass over the input), object-stream members are parsed once per distinct
//! offset, and stream data is skipped once read.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Range;

use crate::document::{Document, DocumentError, DocumentErrorKind, DocumentLimits, OpenOptions};
use crate::encrypt::{Drbg, OutputCipher};
use crate::lexer::LexErrorKind;
use crate::object::{Dictionary, Object, ObjectId};
use crate::parser::{
    DuplicateKeyPolicy, ParseError, ParseErrorKind, ParseLimitKind, ParseLimits, Parser,
};
use crate::security::{Security, resolve_encryption};
use crate::serialize::{SerializeError, SerializeStyle, WriteLimits, Writer};
use crate::stream::{
    Filter, FilterSpec, PredictorParams, StreamErrorKind, StreamLimits,
    decode_filter_specs_lenient, decode_filter_specs_with_usage,
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
    /// Objects that xref placed where an object of that number starts but
    /// whose value could not be read even to the end of the input: left out
    /// (null), the xref's other objects kept. Damage: MuPDF repairs the file
    /// when it reads such an object (`pdf_cache_object`).
    pub objects_unreadable: usize,
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
    /// An encrypted file's object streams could not be read, no password
    /// authenticating: the output holds its structure only and needs a
    /// password; rebuild again with the right one.
    pub locked: bool,
    /// An encrypted file with streams that can hold objects (/N and /First,
    /// or /Type /ObjStm): what the rebuild reads, and so its bytes, may
    /// depend on the password. When false, rebuilding with any password
    /// gives the same bytes.
    pub password_dependent: bool,
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
    let limits_for_repair = RepairLimits::for_document(limits);
    let mut repaired = rebuild_with_passwords(&input, limits_for_repair, &[password])?;
    if repaired.report.locked {
        return Err(RepairError::Unsupported(
            "object streams in an encrypted document",
        ));
    }
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
    rebuild_with_passwords(input, limits, &[])
}

/// `rebuild`, reading an encrypted file's object streams with the first of
/// `passwords` that authenticates (the empty password when none is given).
pub fn rebuild_with_passwords(
    input: &[u8],
    limits: RepairLimits,
    passwords: &[&[u8]],
) -> Result<RepairedPdf, RepairError> {
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
        unparsed: BTreeMap::new(),
        failed: BTreeMap::new(),
        key: None,
    };
    scan.run()?;
    let passwords = if passwords.is_empty() {
        &[b"".as_slice()][..]
    } else {
        passwords
    };
    let bytes = scan.finish(version, at == 0, passwords)?;
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
    // A stream naming an external file is copied as it is; the caller's
    // open decides whether to accept it.
    let options = OpenOptions {
        external_streams: true,
        ..OpenOptions::default()
    };
    match Document::open_with(&bytes, verify, b"", options) {
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
    /// Object headers whose value could not be parsed: the number, the id
    /// and the position of the last one.
    unparsed: BTreeMap<u32, (ObjectId, usize)>,
    /// The same headers by position: the id and where the value starts.
    failed: BTreeMap<usize, (ObjectId, usize)>,
    /// The security handler that decrypts object streams.
    key: Option<Security>,
}

/// The winning definition of each object number, and the containers among
/// them (expanded, not written).
type Selected = (BTreeMap<u32, usize>, BTreeSet<usize>);

/// A value parsed past an event bound and its length, or where its parse
/// failed (None when it was not tried, or parsed without `endobj` after it).
type Reparsed = Result<(Object, usize), Option<usize>>;

/// How an encrypted file's object streams are read.
enum Handler {
    /// Not encrypted, or /Encrypt names no handler.
    None,
    /// Decrypted with an authenticated key.
    Key(Security),
    /// Encrypted, and no password authenticates.
    Locked,
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
            if let Some(end) = end {
                let gap_end = next.as_ref().map_or(body.len(), |e| e.start);
                self.loose_dictionaries(end, gap_end)?;
            }
        }
        Ok(())
    }

    /// Trailer entries from dictionaries outside any object between `from`
    /// and `bound`, where no `trailer` keyword precedes them (a damaged
    /// keyword, `8railer`): MuPDF's repair takes /Root, /Info, /ID and
    /// /Encrypt from every dictionary it meets outside an object. Failed
    /// parses are charged to the one-pass budget `reparse` uses.
    fn loose_dictionaries(&mut self, mut from: usize, bound: usize) -> Result<(), RepairError> {
        let body = self.body;
        while from < bound {
            let Some(offset) = body[from..bound].windows(2).position(|w| w == b"<<") else {
                break;
            };
            let at = from + offset;
            if self.wasted > body.len() {
                break;
            }
            match self.parse(&body[at..bound])? {
                Ok((Object::Dictionary(dictionary), consumed)) => {
                    self.harvest(&dictionary);
                    from = at + consumed.max(2);
                }
                Ok((_, consumed)) => from = at + consumed.max(2),
                Err(error) => {
                    self.wasted += error.offset.max(1);
                    from = at + 2;
                }
            }
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
            .with_lenient()
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
    /// `(the trailer)`, or a dictionary (`/A 3 0 obj`). Accepted only when
    /// `endobj` or `stream` follows; failed attempts share a budget of one
    /// pass over the input. A failed parse gives where it failed.
    fn reparse(&mut self, value_start: usize) -> Result<Reparsed, RepairError> {
        let body = self.body;
        if self.wasted > body.len() {
            return Ok(Err(None));
        }
        match self.parse(&body[value_start..])? {
            Ok((value, consumed)) => {
                let end = value_start + consumed;
                let rest = &body[skip_trivia(body, end, body.len().min(end + GAP))..];
                if rest.starts_with(b"endobj")
                    || matches!(value, Object::Dictionary(_)) && rest.starts_with(b"stream")
                {
                    return Ok(Ok((value, consumed)));
                }
                self.wasted += consumed;
                Ok(Err(None))
            }
            Err(error) => {
                self.wasted += error.offset.max(1);
                Ok(Err(Some(value_start + error.offset)))
            }
        }
    }

    fn object(
        &mut self,
        id: ObjectId,
        start: usize,
        value_start: usize,
        mut bound: usize,
    ) -> Result<Option<usize>, RepairError> {
        let body = self.body;
        // A dictionary is read through any object header inside it (`/A 3 0
        // obj` reads as null), as MuPDF's repair parses a dictionary at each
        // `obj`; the header inside is then not an object.
        let dictionary = body[skip_trivia(body, value_start, bound)..bound].starts_with(b"<<");
        let mut resume = None;
        let parsed = match self.parse(&body[value_start..bound])? {
            Ok(parsed) => Some(parsed),
            // A dictionary is not read past a header that starts a line and
            // whose own object parses through `endobj` (or to `stream`):
            // that is the next object, and the dictionary lost its end
            // (`/Foo 1` and no `>>`); MuPDF's repair reads the next object
            // at its header too.
            Err(error)
                if bound < body.len()
                    && dictionary
                    && !open_string(&error)
                    && line_start(body, bound)
                    && self.whole_object_at(bound) =>
            {
                None
            }
            Err(error) if bound < body.len() && (dictionary || open_string(&error)) => {
                match self.reparse(value_start)? {
                    Ok(parsed) => {
                        bound = body.len();
                        Some(parsed)
                    }
                    // MuPDF's repair goes on after the token its dictionary
                    // parse failed on: when that is the number of the header
                    // inside (`/A /B 3 0 obj`, a key that is not a name), that
                    // header is no object either, unless its own object
                    // parses through `endobj` (a header glued to the damaged
                    // tail, `/Foo (abc)4 0 obj`): then it is the next object.
                    Err(Some(failed))
                        if dictionary
                            && failed >= bound
                            && body[bound..failed]
                                .iter()
                                .all(|&b| b.is_ascii_digit() || is_ws(b)) =>
                    {
                        if !self.whole_object_at(bound) {
                            resume = Some(failed + 1);
                        }
                        None
                    }
                    Err(_) => None,
                }
            }
            Err(_) => None,
        };
        let Some((value, consumed)) = parsed else {
            self.report.objects_dropped += 1;
            self.unparsed.insert(id.number(), (id, start));
            self.failed.insert(start, (id, value_start));
            return Ok(resume);
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

    /// Whether an object header starts at `at` (an event) and its value,
    /// read alone up to the next event, is followed by `endobj`, or is a
    /// dictionary followed by `stream`. Each header is checked at most once,
    /// by the object before it, within its own extent: linear in all.
    fn whole_object_at(&self, at: usize) -> bool {
        let body = self.body;
        let Some(Event {
            start,
            keyword_end,
            kind: Kind::Object(_),
        }) = next_event(body, at)
        else {
            return false;
        };
        if start != at {
            return false;
        }
        let bound = next_event(body, keyword_end).map_or(body.len(), |e| e.start);
        let parsed = Parser::new(&body[keyword_end..bound], self.limits.parse)
            .with_duplicate_key_policy(DuplicateKeyPolicy::KeepLast)
            .with_lenient()
            .parse_next();
        let Ok(Some(parsed)) = parsed else {
            return false;
        };
        let end = keyword_end + parsed.span.end;
        let rest = &body[skip_trivia(body, end, bound)..bound];
        rest.starts_with(b"endobj")
            || matches!(parsed.object, Object::Dictionary(_)) && rest.starts_with(b"stream")
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
    fn select(&mut self, handler: &Handler) -> Result<Selected, RepairError> {
        let locked = matches!(handler, Handler::Locked);
        if let Some(xref) = self.read_xref() {
            let saved = (
                self.defs.len(),
                self.report.clone(),
                self.decoded,
                self.values,
            );
            if let Some(selected) = self.select_by_xref(&xref, locked)? {
                self.report.xref_intact = true;
                return Ok(selected);
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
        if locked && !containers.is_empty() {
            // Members cannot be read: the containers stay ordinary streams.
            self.report.locked = true;
            return Ok((self.winners(), BTreeSet::new()));
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

    /// The object at `start` read past the event bound to the end of the
    /// input, accepted when `endobj` or `stream` follows; its definition's
    /// index. Every attempt is charged to the one-pass budget `reparse`
    /// shares, so overlapping reads stay linear.
    fn read_at(
        &mut self,
        id: ObjectId,
        start: usize,
        value_start: usize,
        dropped: bool,
    ) -> Result<Option<usize>, RepairError> {
        let body = self.body;
        let Ok((value, consumed)) = self.reparse(value_start)? else {
            return Ok(None);
        };
        self.wasted += consumed;
        let value_end = value_start + consumed;
        let mut stream = None;
        if let Object::Dictionary(dictionary) = &value {
            if type_is(&value, b"XRef") {
                return Ok(None);
            }
            let p = skip_trivia(body, value_end, body.len().min(value_end + GAP));
            if body[p..].starts_with(b"stream") {
                stream = Some(stream_data(body, dictionary, p + 6).0);
            }
        }
        self.define(Def {
            id,
            order: (start, 0),
            value,
            stream,
        })?;
        if dropped {
            self.report.objects_dropped = self.report.objects_dropped.saturating_sub(1);
        }
        Ok(Some(self.defs.len() - 1))
    }

    /// Select the definitions the xref names, and the containers among them;
    /// None when any in-use entry does not point at an object header of that
    /// number, or names members that cannot be read (`locked`). An object
    /// the scan did not define there (its value did not parse before the
    /// next header, or the scan did not reach its header) is read at its
    /// offset to the end of the input (`read_at`), else left out
    /// (`objects_unreadable`). A
    /// container is any stream with /N and /First that entries name, as
    /// MuPDF reads it.
    fn select_by_xref(
        &mut self,
        xref: &BTreeMap<u32, Entry>,
        locked: bool,
    ) -> Result<Option<Selected>, RepairError> {
        let body = self.body;
        // The scan is over: the reads here get their own one-pass budget, so
        // an object the scan's failed rereads came close to exhausting it
        // for is still read at its offset (two passes in all).
        self.wasted = 0;
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
                        // The object is there but did not parse before the
                        // next header (one inside it, `/A 3 0 obj`), or the
                        // scan never reached its header (glued to the damaged
                        // tail of the object before it, or passed over while
                        // reading that object): read it as MuPDF reads an
                        // object at its offset, else leave it out, keeping
                        // the xref's other objects.
                        None => {
                            let found = match self.failed.get(&at) {
                                Some(&(id, value_start)) => Some((id, value_start, true)),
                                None => header_at(body, at).map(|(id, v)| (id, v, false)),
                            };
                            match found {
                                Some((id, value_start, dropped)) if id.number() == number => {
                                    match self.read_at(id, at, value_start, dropped)? {
                                        Some(i) => {
                                            winners.insert(number, i);
                                        }
                                        None => self.report.objects_unreadable += 1,
                                    }
                                }
                                _ => return Ok(None),
                            }
                        }
                        _ => return Ok(None),
                    }
                }
                Entry::Member(container, index) => members.push((number, container, index)),
            }
        }
        let mut containers = BTreeMap::<u32, usize>::new();
        for &(_, container, _) in &members {
            match winners.get(&container) {
                Some(&i) if is_container(&self.defs[i]) => {
                    containers.insert(container, i);
                }
                _ => return Ok(None),
            }
        }
        if locked && !containers.is_empty() {
            return Ok(None);
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
        let mut containers: BTreeSet<usize> = containers.into_values().collect();
        containers.extend(
            winners
                .values()
                .copied()
                .filter(|&i| is_object_stream(&self.defs[i])),
        );
        Ok(Some((winners, containers)))
    }

    /// Follow the xref chain from the last `startxref` in the final 1,024
    /// bytes (MuPDF's `pdf_read_start_xref` window), newest section first,
    /// accepting forward /Prev and hybrid /XRefStm. None when any part of it
    /// cannot be read. Objects after it (an update whose xref was lost, or
    /// was never written) are not in the chain, so the last complete
    /// revision is read, as MuPDF reads it; past the window, as in MuPDF,
    /// no `startxref` is found and the whole file is scanned.
    fn read_xref(&mut self) -> Option<BTreeMap<u32, Entry>> {
        let body = self.body;
        let tail = body.len().saturating_sub(1024);
        let marker = tail + body[tail..].windows(9).rposition(|w| w == b"startxref")?;
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
        let Decoded::Bytes(bytes) = self.decode(&body[data], &specs, true) else {
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

    /// Decode against the shared budget, `lenient` as MuPDF's Flate filter
    /// reads damage. A failure is charged the most its input could have
    /// inflated to.
    fn decode(&mut self, data: &[u8], specs: &[FilterSpec], lenient: bool) -> Decoded {
        let remaining = self.limits.max_decoded_bytes.saturating_sub(self.decoded);
        let limits = StreamLimits {
            max_encoded_bytes: data.len(),
            max_decoded_bytes: remaining,
            max_filters: 1,
            max_total_decoded_bytes: remaining,
        };
        let decoded = match lenient {
            true => decode_filter_specs_lenient(data, specs, limits),
            false => decode_filter_specs_with_usage(data, specs, limits),
        };
        match decoded {
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
                    match self.decode(&body[start..end], &specs, false) {
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

    /// The handler that reads object streams: authenticated with the first
    /// of `passwords` that opens `encryption` (its indirect values resolved
    /// from the last definitions), `Locked` when none does.
    fn handler(&self, encryption: Option<&Object>, passwords: &[&[u8]]) -> Handler {
        let latest = self.winners();
        let lookup = |id: ObjectId| {
            let def = &self.defs[*latest.get(&id.number())?];
            def.stream.is_none().then_some(&def.value)
        };
        let dictionary = match encryption {
            None => return Handler::None,
            Some(Object::Dictionary(dictionary)) => dictionary.clone(),
            Some(Object::Reference(id)) => match lookup(*id) {
                Some(Object::Dictionary(dictionary)) => dictionary.clone(),
                _ => return Handler::None,
            },
            Some(_) => return Handler::None,
        };
        let dictionary = resolve_encryption(&dictionary, &lookup);
        let first_id = match self.trailer.get(b"ID".as_slice()) {
            Some(Object::Array(items)) => match items.first() {
                Some(Object::String(id)) => id.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        passwords
            .iter()
            .find_map(|password| Security::authenticate(&dictionary, &first_id, password).ok())
            .map_or(Handler::Locked, Handler::Key)
    }

    fn finish(
        &mut self,
        version: [u8; 3],
        at_start: bool,
        passwords: &[&[u8]],
    ) -> Result<Vec<u8>, RepairError> {
        let encryption = self.encryption();
        let encryption_from_scan =
            encryption.is_some() && !self.trailer.contains_key(b"Encrypt".as_slice());
        self.report.password_dependent = encryption.is_some()
            && (self.defs.iter()).any(|def| is_container(def) || is_object_stream(def));
        let handler = self.handler(encryption.as_ref(), passwords);
        if let Handler::Key(security) = &handler {
            self.key = Some(security.clone());
        }
        let (winners, containers) = self.select(&handler)?;
        let locked = self.report.locked;
        let mut written = winners;
        self.report.objects_overridden = self.defs.len() - written.len();
        written.retain(|_, index| !containers.contains(index));
        self.drop_linearization(&mut written);
        if !locked {
            self.placeholder_pages(&mut written);
        }

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
        // An /Encrypt that names no dictionary is kept as it is (a reference
        // to nothing becomes null): the reader ignores it, as MuPDF does.
        let encrypt = match &encryption {
            None => None,
            Some(Object::Dictionary(encrypt)) => Some(Object::Dictionary(encrypt.clone())),
            encrypt => Some(
                resolve(encrypt.as_ref())
                    .filter(|id| {
                        matches!(
                            self.defs[written[&id.number()]].value,
                            Object::Dictionary(_)
                        )
                    })
                    .map_or_else(
                        || encrypt.clone().unwrap_or(Object::Null),
                        Object::Reference,
                    ),
            ),
        };
        let lookup = |id: ObjectId| {
            let def = &self.defs[*written.get(&id.number())?];
            def.stream.is_none().then_some(&def.value)
        };
        let encrypt_dictionary = match &encrypt {
            Some(Object::Dictionary(encrypt)) => Some(resolve_encryption(encrypt, &lookup)),
            Some(Object::Reference(id)) => match lookup(*id) {
                Some(Object::Dictionary(encrypt)) => Some(resolve_encryption(encrypt, &lookup)),
                _ => None,
            },
            _ => None,
        };
        let encrypt_id = match &encrypt {
            Some(Object::Reference(id)) => Some(*id),
            _ => None,
        };
        let (cipher, encrypt_metadata) =
            encrypt_dictionary
                .as_ref()
                .map_or((Cipher::None, true), |d| {
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
        let revision = encrypt_dictionary
            .as_ref()
            .and_then(|d| match d.get(b"R".as_slice()) {
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
        // Without the members, the catalog may be one: keep the trailer's.
        let unread_root = match self.trailer.get(b"Root".as_slice()) {
            Some(Object::Reference(id)) if locked && root.is_none() => Some(*id),
            _ => None,
        };
        let root = match root.or(unread_root) {
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
        // A locked structure keeps its references to the members it lacks.
        for value in trailer.values_mut().filter(|_| !locked) {
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
            if !locked {
                self.report.references_repaired += fix_references(&mut def.value, &ids, false);
            }
        }
        if !locked {
            self.report.page_tree_repaired = self.repair_page_tree(&mut written, root);
        }
        self.report.objects_recovered = written.len();
        let largest = written.keys().next_back().copied().unwrap_or(0);
        trailer.insert(b"Size".to_vec(), Object::Integer(i64::from(largest) + 1));
        let report = &mut self.report;
        report.damaged = !at_start
            || !report.xref_intact
            || report.objects_unreadable > 0
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
        // Members of decrypted object streams get their strings encrypted
        // under their own object keys, so the reader decrypts them once. The
        // output is internal: a fixed seed gives distinct, repeatable IVs.
        let mut members_cipher = (self.key.as_ref())
            .map(|key| OutputCipher::new(Cow::Borrowed(key), Drbg::new(b"Tandem PDF repair")));
        let mut offsets = Vec::new();
        for &index in written.values() {
            let def = &self.defs[index];
            let member = def.order.1 > 0 && Some(def.id) != encrypt_id;
            let encrypted;
            let value = match members_cipher.as_mut().filter(|_| member) {
                Some(cipher) => {
                    encrypted = (cipher.strings(&def.value, def.id, max_depth))
                        .map_err(|_| RepairError::Invalid("object-stream member strings"))?;
                    &encrypted
                }
                None => &def.value,
            };
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
                _ => write_value(&mut writer, value, max_depth)?,
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
            // A linearization dictionary has no /Type; a catalog (or any
            // typed object) carrying /Linearized is kept.
            if dictionary
                .get(b"Linearized".as_slice())
                .is_none_or(|value| matches!(value, Object::Null))
                || matches!(dictionary.get(b"Type".as_slice()), Some(Object::Name(_)))
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
    /// The number of entries in the /Kids of definition `index`: None when
    /// it has none (or null), 0 when it is not an array (or names none).
    fn kids_len(&self, written: &BTreeMap<u32, usize>, index: usize) -> Option<usize> {
        let Object::Dictionary(dictionary) = &self.defs[index].value else {
            return None;
        };
        match dictionary.get(b"Kids".as_slice()) {
            None | Some(Object::Null) => None,
            Some(Object::Array(kids)) => Some(kids.len()),
            Some(Object::Reference(id)) => Some(match written.get(&id.number()) {
                Some(&i) => match (&self.defs[i].stream, &self.defs[i].value) {
                    (None, Object::Array(kids)) => kids.len(),
                    _ => 0,
                },
                None => 0,
            }),
            Some(_) => Some(0),
        }
    }

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

    /// A page the tree lists whose object could not be parsed is kept as an
    /// empty dictionary, so it stays a (blank) page, as MuPDF shows it, and
    /// the page count is the tree's.
    fn placeholder_pages(&mut self, written: &mut BTreeMap<u32, usize>) {
        if self.unparsed.is_empty() {
            return;
        }
        let mut listed = BTreeSet::new();
        for &index in written.values() {
            let Object::Dictionary(dictionary) = &self.defs[index].value else {
                continue;
            };
            let kids = match dictionary.get(b"Kids".as_slice()) {
                Some(Object::Array(kids)) => kids,
                Some(Object::Reference(id)) => match written.get(&id.number()) {
                    Some(&i) => match &self.defs[i].value {
                        Object::Array(kids) => kids,
                        _ => continue,
                    },
                    None => continue,
                },
                _ => continue,
            };
            listed.extend(kids.iter().filter_map(|kid| match kid {
                Object::Reference(id) => Some(id.number()),
                _ => None,
            }));
        }
        let missing: Vec<(ObjectId, usize)> = (self.unparsed.iter())
            .filter(|(number, _)| listed.contains(*number) && !written.contains_key(*number))
            .map(|(_, &found)| found)
            .collect();
        for (id, start) in missing {
            self.defs.push(Def {
                id,
                order: (start, 0),
                value: Object::Dictionary(Dictionary::new()),
                stream: None,
            });
            written.insert(id.number(), self.defs.len() - 1);
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
        // The root is walked whatever its /Type when it lists kids, as MuPDF
        // and the strict page walk read it.
        let top = pages
            .and_then(|pages| self.node(written, &pages))
            .filter(|&(i, id)| {
                node_kind(&self.defs[i].value, self.kids_len(written, i)) == Some(true)
                    || self.frame(written, (i, id), false).listed
            });
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
            // MuPDF reads a catalog without /Type (or with a null one, such as
            // a damaged name) as it is: typing it is not damage.
            let untyped = catalog.get(b"Type".as_slice()).is_none_or(Object::is_null);
            changed |= set(catalog, b"Type", Object::Name(b"Catalog".to_vec())) && !untyped;
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
            let kind = node_kind(&self.defs[index].value, self.kids_len(written, index));
            let contents = match (&self.defs[index].value, kind) {
                (Object::Dictionary(page), Some(false)) => self.contents(written, page),
                _ => None,
            };
            let Object::Dictionary(dictionary) = &mut self.defs[index].value else {
                continue;
            };
            // A kid MuPDF reads as this kind is typed without counting as
            // damage (a page whose /Type is damaged: MuPDF reads a page).
            let alike = kind.is_some_and(|node| read_alike(dictionary, node));
            match kind {
                Some(true) if depth < 256 => {
                    visited.insert(id.number());
                    changed |= set(dictionary, b"Type", Object::Name(b"Pages".to_vec())) && !alike;
                    changed |= set(dictionary, b"Parent", Object::Reference(parent));
                    stack.push(self.frame(written, (index, id), media));
                }
                Some(false) => {
                    visited.insert(id.number());
                    if !other_kind(dictionary) {
                        changed |=
                            set(dictionary, b"Type", Object::Name(b"Page".to_vec())) && !alike;
                    }
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
        let id = container.id;
        let start = stream.data.start;
        let ends = match stream.keyword {
            Some(keyword) => stream_ends(body, start, keyword),
            None => vec![stream.data.end],
        };
        // Each candidate end as it decodes, then as MuPDF's filter reads it.
        let mut bytes = None;
        'search: for lenient in [false, true] {
            for &end in &ends {
                let data = match &self.key {
                    Some(key) => key.decrypt(key.stream_method(false), id, &body[start..end]),
                    None => body[start..end].to_vec(),
                };
                match self.decode(&data, &specs, lenient) {
                    Decoded::Bytes(decoded) => {
                        bytes = Some(decoded);
                        if let Some(stream) = &mut self.defs[index].stream {
                            stream.data.end = end;
                        }
                        break 'search;
                    }
                    Decoded::Failed => {}
                    Decoded::Limit => {
                        return Err(RepairError::LimitExceeded("object-stream bytes"));
                    }
                }
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
    let found = body[start..].windows(9).position(|w| w == b"endstream");
    // A declared length that lands on a damaged keyword (`endobj` within
    // 32 bytes, before the next `endstream`) is still the data: searching on
    // would swallow the objects after the stream.
    if let Some(declared) = declared {
        let keyword = found.map_or(body.len(), |found| start + found);
        let end = start.saturating_add(declared);
        let near = keyword.min(end.saturating_add(32));
        if end <= near && body[end..near].windows(6).any(|w| w == b"endobj") {
            let stream = Stream {
                data: start..end,
                keyword: None,
                length,
            };
            return (stream, end);
        }
    }
    match found {
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
        && (number_start == 0
            || is_delimiter(body[number_start - 1])
            // An update's first object may follow `%%EOF` with no line break.
            || body[..number_start].ends_with(b"%%EOF"));
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

/// Whether `at` starts a line: only spaces or tabs between it and the
/// previous line break (or the start of the input).
fn line_start(body: &[u8], at: usize) -> bool {
    let before = body[..at]
        .iter()
        .rposition(|&b| !matches!(b, b' ' | b'\t' | 0 | 12));
    before.is_none_or(|i| matches!(body[i], b'\n' | b'\r'))
}

/// The object header `N G obj` starting exactly at `at`, as an xref entry
/// locates one: its id and where its value starts.
fn header_at(body: &[u8], at: usize) -> Option<(ObjectId, usize)> {
    let digits = |from: usize, max: usize| {
        let count = body
            .get(from..)?
            .iter()
            .take(max + 1)
            .take_while(|b| b.is_ascii_digit())
            .count();
        (1..=max).contains(&count).then_some(from + count)
    };
    let spaces = |from: usize| {
        let mut p = from;
        while p < body.len() && is_ws(body[p]) {
            p += 1;
        }
        (p > from).then_some(p)
    };
    let number_end = digits(at, 10)?;
    let generation_start = spaces(number_end)?;
    let generation_end = digits(generation_start, 5)?;
    let keyword = spaces(generation_end)?;
    if !body[keyword..].starts_with(b"obj")
        || body.get(keyword + 3).is_some_and(|&b| !is_delimiter(b))
    {
        return None;
    }
    let decimal = |range: Range<usize>| -> Option<u64> {
        std::str::from_utf8(&body[range]).ok()?.parse().ok()
    };
    let number = u32::try_from(decimal(at..number_end)?).ok()?;
    let generation = u16::try_from(decimal(generation_start..generation_end)?).ok()?;
    Some((ObjectId::new(number, generation).ok()?, keyword + 3))
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
        Some(Object::Name(name)) if name == b"FlateDecode" || name == b"Fl" => {}
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

/// /Type names of other kinds of objects. A kid typed so is a page only
/// when it looks like one (`node_kind`), and keeps its /Type: every reader
/// reads a /Type other than /Pages as a page, and retyping it would break it
/// for its own use (a font, an annotation).
const NOT_PAGES: [&[u8]; 20] = [
    b"Catalog",
    b"Font",
    b"FontDescriptor",
    b"Annot",
    b"XObject",
    b"ExtGState",
    b"Pattern",
    b"Action",
    b"Outlines",
    b"OCG",
    b"OCMD",
    b"Sig",
    b"Metadata",
    b"Filespec",
    b"EmbeddedFile",
    b"StructTreeRoot",
    b"StructElem",
    b"Encoding",
    b"Mask",
    b"Group",
];

/// Whether a page-tree kid without a /Type (or with a null one) is a node:
/// it has /Kids (`kids`: None when missing or null, else the number of
/// entries, 0 when it is not an array), unless that list is empty and the
/// kid looks like a page (`contents`: a /Contents reference or array;
/// `media`: a /MediaBox). Shared by the repair and the app's page walk
/// (`app::info`), so a file reads the same whether or not it was repaired.
/// MuPDF (`pdf_lookup_page_loc`) reads such a kid as a node when it has
/// /Kids and no /MediaBox; with a /MediaBox its kids are hidden behind one
/// page, and with empty /Kids and /Contents its content is.
pub(crate) fn untyped_kid_is_node(kids: Option<usize>, contents: bool, media: bool) -> bool {
    match kids {
        None => false,
        Some(0) => !(contents || media),
        Some(_) => true,
    }
}

/// Some(true) for a Pages node, Some(false) for a page, None for neither, as
/// MuPDF's `pdf_lookup_page_loc` reads a kid: /Type /Pages is a node and any
/// other /Type (a damaged name, a number, a string) a page. A kid typed as
/// another kind of object (`NOT_PAGES`) is a page when it has /Contents (a
/// reference or array), or a /MediaBox and no non-empty /Kids; otherwise it
/// is left out (MuPDF and the strict walk show a blank page). Without /Type
/// (or with null), `untyped_kid_is_node`. `kids` as there.
fn node_kind(value: &Object, kids: Option<usize>) -> Option<bool> {
    let Object::Dictionary(dictionary) = value else {
        return None;
    };
    let contents = matches!(
        dictionary.get(b"Contents".as_slice()),
        Some(Object::Reference(_) | Object::Array(_))
    );
    let media = present(dictionary, b"MediaBox");
    match dictionary.get(b"Type".as_slice()) {
        Some(Object::Name(name)) if name == b"Pages" => Some(true),
        Some(Object::Name(name)) if NOT_PAGES.contains(&name.as_slice()) => {
            (contents || media && kids.unwrap_or(0) == 0).then_some(false)
        }
        None | Some(Object::Null) => Some(untyped_kid_is_node(kids, contents, media)),
        Some(_) => Some(false),
    }
}

/// Whether MuPDF reads the kid as `node_kind` classified it (`node`), so
/// typing it is not damage. MuPDF: /Type /Pages, or no /Type with /Kids and
/// no /MediaBox, is a node; anything else a page.
fn read_alike(dictionary: &Dictionary, node: bool) -> bool {
    let mupdf_node = match dictionary.get(b"Type".as_slice()) {
        Some(kind) if !kind.is_null() => matches!(kind, Object::Name(name) if name == b"Pages"),
        _ => present(dictionary, b"Kids") && !present(dictionary, b"MediaBox"),
    };
    mupdf_node == node
}

/// A /Type naming another kind of object (`NOT_PAGES`).
fn other_kind(dictionary: &Dictionary) -> bool {
    matches!(dictionary.get(b"Type".as_slice()),
        Some(Object::Name(name)) if NOT_PAGES.contains(&name.as_slice()))
}

fn is_object_stream(def: &Def) -> bool {
    def.stream.is_some() && type_is(&def.value, b"ObjStm")
}

/// A stream that can hold members: /N and /First, whatever its /Type.
fn is_container(def: &Def) -> bool {
    let integer = |key: &[u8]| matches!(&def.value, Object::Dictionary(d) if matches!(d.get(key), Some(Object::Integer(_))));
    def.stream.is_some() && integer(b"N") && integer(b"First")
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
