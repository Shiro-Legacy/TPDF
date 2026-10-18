// SPDX-License-Identifier: MIT
//! Tolerant open and repair (spec 03): any bytes in, one well-formed
//! revision out that the strict reader (`document::Document`) accepts, with
//! a report of what was done and whether the input was damaged.
//!
//! The file's own cross-reference chain is followed when it describes the
//! objects (ISO 32000-2 §7.5.4 to §7.5.8). Otherwise the document is
//! reconstructed from the object definitions found in the bytes. Every value
//! is read with the lenient parser. The output is one classic-xref revision,
//! written by this module's own serializer, which keeps each dictionary's
//! key order and writes each string in its shorter form.
//!
//! How the bytes are scanned:
//!
//! - One pass finds every `N G obj` header and `trailer` keyword at a token
//!   boundary (strings and stream data are not recognised in this pass), and
//!   every `endstream` keyword.
//! - The candidates are then read in file order. A candidate whose value
//!   reads completely (to `endobj`, `stream`, the next candidate or the end)
//!   covers the candidates inside it, so a header inside a string or read as
//!   damaged tokens is no definition. One that does not read leaves the next
//!   candidate to be read on its own.
//! - Reading past the next candidate costs work; that work shares one budget
//!   (a small multiple of the input), after which each value is read only up
//!   to the next candidate. Work therefore stays linear on any input.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Range;

use crate::document::{Document, DocumentError, DocumentErrorKind, DocumentLimits, OpenOptions};
use crate::encrypt::{Drbg, OutputCipher};
use crate::lexer::{Lexer, TokenKind};
use crate::object::{Dictionary, Object, ObjectId};
use crate::parser::{DuplicateKeyPolicy, ParseErrorKind, ParseLimitKind, ParseLimits, Parser};
use crate::security::{EncryptionInfo, Method, Security, resolve_encryption};
use crate::serialize::SerializeError;
use crate::stream::{
    Filter, FilterSpec, PredictorParams, StreamErrorKind, StreamLimits,
    decode_filter_specs_lenient, decode_filter_specs_with_usage,
};

// ------------------------------------------------------------------ interface

#[derive(Clone, Copy, Debug)]
pub struct RepairLimits {
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    /// Object definitions kept while reading, superseded ones and
    /// object-stream members included.
    pub max_objects: usize,
    /// Parsed values across all definitions and trailer dictionaries,
    /// dictionary keys included.
    pub max_values: usize,
    pub max_object_streams: usize,
    /// Bytes decoded from object streams, xref streams and stream-extent
    /// checks, across the whole input.
    pub max_decoded_bytes: usize,
    /// Cross-reference sections followed in the file's own chain. A longer
    /// chain is not followed; the file is reconstructed instead.
    pub max_revisions: usize,
    /// Limits for each parsed value.
    pub parse: ParseLimits,
}

impl Default for RepairLimits {
    /// The limits the app opens files with.
    fn default() -> Self {
        Self::for_document(crate::app::document::limits())
    }
}

impl RepairLimits {
    /// Limits no looser than the ones the rebuilt file must later open under.
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
            max_revisions: limits.max_revisions,
            parse: limits.parse,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepairReport {
    pub objects_recovered: usize,
    pub objects_overridden: usize,
    pub objects_dropped: usize,
    pub object_streams_expanded: usize,
    pub streams_length_recovered: usize,
    pub references_repaired: usize,
    pub catalog_from_scan: bool,
    pub page_tree_repaired: bool,
    pub linearized: bool,
    pub xref_intact: bool,
    pub objects_unreadable: usize,
    pub xref_sections: usize,
    pub xref_size: u64,
    pub damaged: bool,
    pub strict_error: Option<String>,
    pub locked: bool,
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
    /// open the input (a strict limit) or the repaired file under the
    /// caller's limits.
    Document(DocumentError),
}

impl fmt::Display for RepairError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "PDF repair: invalid {reason}"),
            Self::Unsupported(reason) => write!(f, "PDF repair: unsupported {reason}"),
            Self::LimitExceeded(reason) => write!(f, "PDF repair: {reason} limit exceeded"),
            Self::Serialize(error) => write!(f, "PDF repair: {error}"),
            Self::Document(error) => write!(f, "{error}"),
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

#[derive(Debug)]
pub struct TolerantPdf {
    pub bytes: Vec<u8>,
    pub repair: Option<RepairReport>,
    pub strict_error: Option<DocumentError>,
    pub original: Option<Vec<u8>>,
    pub needs_password: bool,
}

/// The input opened strictly, or rebuilt when the strict reader refuses it
/// for anything but a password or a size limit.
pub fn open_tolerant(
    input: Vec<u8>,
    limits: DocumentLimits,
    password: &[u8],
) -> Result<TolerantPdf, RepairError> {
    if input.len() > limits.max_input_bytes {
        return Err(RepairError::LimitExceeded("input bytes"));
    }
    let strict = Document::open_with_password(&input, limits, password).err();
    let Some(strict) = strict else {
        return Ok(TolerantPdf {
            bytes: input,
            repair: None,
            strict_error: None,
            original: None,
            needs_password: false,
        });
    };
    match strict.kind {
        DocumentErrorKind::PasswordRequired => {
            return Ok(TolerantPdf {
                bytes: input,
                repair: None,
                strict_error: None,
                original: None,
                needs_password: true,
            });
        }
        DocumentErrorKind::LimitExceeded(_) => return Err(RepairError::Document(strict)),
        _ => {}
    }
    let repaired = rebuild_with_passwords(&input, RepairLimits::for_document(limits), &[password])?;
    let mut report = repaired.report;
    if report.locked {
        return Err(RepairError::Unsupported(
            "object streams in an encrypted document",
        ));
    }
    report.strict_error = Some(strict.to_string());
    let needs_password = match Document::open_with_password(&repaired.bytes, limits, password) {
        Ok(_) => false,
        Err(error) if matches!(error.kind, DocumentErrorKind::PasswordRequired) => true,
        Err(error) => return Err(RepairError::Document(error)),
    };
    Ok(TolerantPdf {
        bytes: repaired.bytes,
        repair: Some(report),
        strict_error: Some(strict),
        original: Some(input),
        needs_password,
    })
}

pub fn rebuild(input: &[u8], limits: RepairLimits) -> Result<RepairedPdf, RepairError> {
    rebuild_with_passwords(input, limits, &[])
}

/// One strict revision rebuilt from `input`. The first password that
/// authenticates reads encrypted object streams; an empty list means the
/// empty password.
pub fn rebuild_with_passwords(
    input: &[u8],
    limits: RepairLimits,
    passwords: &[&[u8]],
) -> Result<RepairedPdf, RepairError> {
    if input.len() > limits.max_input_bytes {
        return Err(RepairError::LimitExceeded("input bytes"));
    }
    let passwords: Vec<&[u8]> = if passwords.is_empty() {
        vec![b"".as_slice()]
    } else {
        passwords.to_vec()
    };
    Repair::new(input, limits, passwords).run()
}

/// Whether a page-tree kid without a /Type (or with a null one) is an
/// intermediate node: `kids` is `None` without /Kids, its length otherwise
/// (0 when /Kids is not an array); `contents` is a /Contents reference or
/// array; `media` a /MediaBox.
pub(crate) fn untyped_kid_is_node(kids: Option<usize>, contents: bool, media: bool) -> bool {
    match kids {
        None => false,
        Some(0) => !(contents || media),
        Some(_) => true,
    }
}

// ------------------------------------------------------------------ constants

/// How far into the input the `%PDF-` header is looked for. The app skips
/// junk before the header within a shorter distance; this reaches further.
const HEADER_SEARCH: usize = 64 * 1024;
/// White space an xref offset (or `startxref`) may point at before the
/// header or keyword it names: the end of the previous line.
const OFFSET_SLACK: usize = 32;
/// Read-through work allowed, as a multiple of the input, plus a floor.
const BUDGET_FACTOR: usize = 4;
const BUDGET_FLOOR: usize = 1 << 20;
/// Nesting read in a dictionary found outside every object.
const LOOSE_DEPTH: usize = 32;
/// Output a strict decode of a stream may produce beyond the lenient decode
/// of its shortest candidate end (the ends differ by at most two bytes).
const DECODE_SLACK: usize = 64 * 1024;
/// The longest damaged `endstream` keyword read before `endobj`.
const DAMAGED_KEYWORD: usize = 16;
/// Parent hops followed when a rebuilt tree copies inherited attributes.
const MAX_INHERIT: usize = 64;
/// Free rows a subsection of the output table spans between objects.
const XREF_GAP: u32 = 8;
/// US Letter in default user space units (8.5 x 11 in at 72 per inch).
const LETTER: [i64; 4] = [0, 0, 612, 792];
/// Arrays (by their key) that list objects: a dangling entry is removed.
const LIST_KEYS: [&[u8]; 8] = [
    b"Kids",
    b"Contents",
    b"Annots",
    b"Fields",
    b"OCGs",
    b"Order",
    b"ON",
    b"OFF",
];
/// Attributes a page inherits from its ancestors (ISO 32000-2 §7.7.3.4).
const INHERITED: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];
/// Seed for the initialization vectors of re-encrypted member strings. The
/// rebuilt bytes never leave the engine, so a fixed seed keeps the output
/// deterministic without weakening a saved file.
const MEMBER_SEED: &[u8] = b"Tandem PDF repair: object-stream members";
/// /Type values ISO 32000-2 defines for objects other than page-tree nodes
/// and pages. A kid typed as one of these is a page only when it looks like
/// one (section 4.10 rule 1).
const ISO_TYPES: [&[u8]; 74] = [
    b"Catalog",
    b"Font",
    b"FontDescriptor",
    b"FontFile",
    b"Encoding",
    b"CMap",
    b"CIDFont",
    b"Annot",
    b"XObject",
    b"ExtGState",
    b"Pattern",
    b"Shading",
    b"Halftone",
    b"Action",
    b"Outlines",
    b"Border",
    b"Filespec",
    b"EmbeddedFile",
    b"Metadata",
    b"ObjStm",
    b"XRef",
    b"Group",
    b"Mask",
    b"OCG",
    b"OCMD",
    b"StructTreeRoot",
    b"StructElem",
    b"MCR",
    b"OBJR",
    b"Namespace",
    b"Sig",
    b"SigRef",
    b"SigFieldLock",
    b"TransformParams",
    b"DocTimeStamp",
    b"DSS",
    b"VRI",
    b"Thread",
    b"Bead",
    b"Trans",
    b"Template",
    b"PageLabel",
    b"Collection",
    b"CollectionItem",
    b"CollectionSchema",
    b"CollectionField",
    b"CollectionSort",
    b"CollectionColors",
    b"CollectionSubitem",
    b"CollectionSplit",
    b"Folder",
    b"Navigator",
    b"Rendition",
    b"MediaClip",
    b"MediaPermissions",
    b"MediaPlayerInfo",
    b"MediaPlayers",
    b"MediaDuration",
    b"MediaCriteria",
    b"MediaScreenParams",
    b"MediaOffset",
    b"Timespan",
    b"SoftwareIdentifier",
    b"Viewport",
    b"Measure",
    b"PtData",
    b"3D",
    b"3DRef",
    b"3DView",
    b"3DBG",
    b"3DNode",
    b"DPart",
    b"DPartRoot",
    b"EncryptedPayload",
];

// ------------------------------------------------------------------ bytes

/// White space (ISO 32000-2 §7.2.3, Table 1).
fn is_ws(byte: u8) -> bool {
    matches!(byte, 0 | 9 | 10 | 12 | 13 | 32)
}

/// Delimiters (§7.2.3, Table 2).
fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn is_regular(byte: u8) -> bool {
    !is_ws(byte) && !is_delimiter(byte)
}

fn skip_ws(data: &[u8], mut at: usize, end: usize) -> usize {
    let end = end.min(data.len());
    while at < end && is_ws(data[at]) {
        at += 1;
    }
    at
}

/// Past white space and comments.
fn skip_trivia(data: &[u8], mut at: usize, end: usize) -> usize {
    let end = end.min(data.len());
    while at < end {
        if is_ws(data[at]) {
            at += 1;
        } else if data[at] == b'%' {
            while at < end && !matches!(data[at], b'\r' | b'\n') {
                at += 1;
            }
        } else {
            break;
        }
    }
    at
}

/// Whether `keyword` begins at `at` and is not followed by a regular byte.
fn keyword_at(data: &[u8], at: usize, keyword: &[u8]) -> bool {
    let Some(rest) = data.get(at..) else {
        return false;
    };
    rest.starts_with(keyword) && rest.get(keyword.len()).is_none_or(|&b| !is_regular(b))
}

fn starts_at(data: &[u8], at: usize, end: usize, prefix: &[u8]) -> bool {
    data.get(at..end.min(data.len()))
        .is_some_and(|rest| rest.starts_with(prefix))
}

/// An unsigned decimal token at `at` (at most 20 digits) and its end.
fn raw_uint(data: &[u8], at: usize) -> Option<(u64, usize)> {
    let mut end = at;
    let mut value: u64 = 0;
    while let Some(&byte) = data.get(end) {
        if !byte.is_ascii_digit() {
            break;
        }
        value = value.checked_mul(10)?.checked_add(u64::from(byte - b'0'))?;
        end += 1;
        if end - at > 20 {
            return None;
        }
    }
    if end == at || data.get(end).is_some_and(|&b| is_regular(b)) {
        return None;
    }
    Some((value, end))
}

/// `N G obj` at `at`: the number, generation and where the value begins.
fn read_header(data: &[u8], at: usize) -> Option<(u32, u16, usize)> {
    let (number, end) = raw_uint(data, at)?;
    let gap = skip_ws(data, end, data.len());
    if gap == end {
        return None;
    }
    let (generation, end) = raw_uint(data, gap)?;
    let gap = skip_ws(data, end, data.len());
    if gap == end || !keyword_at(data, gap, b"obj") {
        return None;
    }
    let number = u32::try_from(number).ok().filter(|&n| n > 0)?;
    let generation = u16::try_from(generation).ok()?;
    Some((number, generation, gap + 3))
}

fn type_is(value: &Object, kind: &[u8]) -> bool {
    matches!(value, Object::Dictionary(d)
        if matches!(d.get(b"Type".as_slice()), Some(Object::Name(n)) if n == kind))
}

fn integer(dict: &Dictionary, key: &[u8]) -> Option<i64> {
    match dict.get(key) {
        Some(Object::Integer(value)) => Some(*value),
        _ => None,
    }
}

/// Values and dictionary keys in `value`, as the parser counts them.
fn object_cost(value: &Object) -> usize {
    match value {
        Object::Array(items) => items
            .iter()
            .fold(1usize, |n, item| n.saturating_add(object_cost(item))),
        Object::Dictionary(dict) => dict.values().fold(1usize, |n, item| {
            n.saturating_add(1).saturating_add(object_cost(item))
        }),
        _ => 1,
    }
}

/// A container stream (§7.5.7): /Type /ObjStm, or integer /N and /First.
fn is_container(value: &Object) -> bool {
    let Object::Dictionary(dict) = value else {
        return false;
    };
    if type_is(value, b"ObjStm") {
        return true;
    }
    integer(dict, b"N").is_some() && integer(dict, b"First").is_some() && !type_is(value, b"XRef")
}

/// The linearization parameter dictionary of Annex F (not a catalog that
/// happens to carry /Linearized).
fn is_linearization(value: &Object) -> bool {
    let Object::Dictionary(dict) = value else {
        return false;
    };
    [b"Linearized".as_slice(), b"L", b"H", b"O", b"E", b"N", b"T"]
        .iter()
        .all(|key| dict.contains_key(*key))
        && !type_is(value, b"Catalog")
        && !dict.contains_key(b"Pages".as_slice())
}

/// The offsets of the hint streams a linearization dictionary's /H names.
fn hint_offsets(value: &Object) -> Vec<usize> {
    let Some(Object::Array(items)) = value.as_dict().and_then(|d| d.get(b"H".as_slice())) else {
        return Vec::new();
    };
    items
        .iter()
        .step_by(2)
        .take(2)
        .filter_map(|item| match item {
            Object::Integer(offset) => usize::try_from(*offset).ok(),
            _ => None,
        })
        .collect()
}

/// The stream filters a dictionary names, abbreviations included (§8.9.7).
/// `None` for a filter or parameter this engine does not decode.
fn filter_specs(dict: &Dictionary) -> Option<Vec<FilterSpec>> {
    let names: Vec<&[u8]> = match dict.get(b"Filter".as_slice()) {
        None | Some(Object::Null) => Vec::new(),
        Some(Object::Name(name)) => vec![name.as_slice()],
        Some(Object::Array(items)) if items.len() <= 8 => items
            .iter()
            .map(|item| item.as_name())
            .collect::<Option<_>>()?,
        _ => return None,
    };
    let parameters: Vec<Option<&Dictionary>> = match dict.get(b"DecodeParms".as_slice()) {
        None | Some(Object::Null) => vec![None; names.len()],
        Some(Object::Dictionary(d)) if names.len() == 1 => vec![Some(d)],
        Some(Object::Array(items)) if items.len() == names.len() => items
            .iter()
            .map(|item| match item {
                Object::Dictionary(d) => Some(Some(d)),
                Object::Null => Some(None),
                _ => None,
            })
            .collect::<Option<_>>()?,
        _ => return None,
    };
    names
        .iter()
        .zip(parameters)
        .map(|(name, parameters)| {
            let mut filter = match *name {
                b"FlateDecode" | b"Fl" => Filter::Flate,
                b"ASCIIHexDecode" | b"AHx" => Filter::AsciiHex,
                b"ASCII85Decode" | b"A85" => Filter::Ascii85,
                b"RunLengthDecode" | b"RL" => Filter::RunLength,
                b"LZWDecode" | b"LZW" => Filter::Lzw { early_change: true },
                _ => return None,
            };
            let mut predictor = PredictorParams::default();
            if let Some(parameters) = parameters {
                for (key, value) in parameters.iter() {
                    let Object::Integer(value) = value else {
                        if value.is_null() {
                            continue;
                        }
                        return None;
                    };
                    match key.as_slice() {
                        b"Predictor" => predictor.predictor = u8::try_from(*value).ok()?,
                        b"Colors" => predictor.colors = usize::try_from(*value).ok()?,
                        b"BitsPerComponent" => {
                            predictor.bits_per_component = u8::try_from(*value).ok()?
                        }
                        b"Columns" => predictor.columns = usize::try_from(*value).ok()?,
                        b"EarlyChange" => {
                            if let Filter::Lzw { .. } = filter {
                                filter = Filter::Lzw {
                                    early_change: *value != 0,
                                };
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some(FilterSpec { filter, predictor })
        })
        .collect()
}

// ------------------------------------------------------------------ the scan's units

#[derive(Clone, Copy, Debug)]
enum CandKind {
    Header(u32, u16),
    Trailer,
}

/// A place where a definition or a trailer may begin.
#[derive(Clone, Copy, Debug)]
struct Cand {
    pos: usize,
    value: usize,
    kind: CandKind,
}

/// Every `N G obj` header and `trailer` keyword at a token boundary, and
/// every `endstream` keyword, in file order.
fn raw_scan(data: &[u8]) -> (Vec<Cand>, Vec<usize>) {
    let mut cands = Vec::new();
    let mut previous: Option<Range<usize>> = None;
    let mut before: Option<Range<usize>> = None;
    let mut at = 0;
    while at < data.len() {
        let byte = data[at];
        if is_ws(byte) {
            at += 1;
            continue;
        }
        // `%%EOF` ends at the marker (addendum A1): an update's first header
        // may follow it on the same line.
        if data[at..].starts_with(b"%%EOF") {
            before = None;
            previous = None;
            at += 5;
            continue;
        }
        if is_delimiter(byte) {
            before = previous.take();
            at += 1;
            continue;
        }
        let start = at;
        while at < data.len() && is_regular(data[at]) {
            at += 1;
        }
        let token = &data[start..at];
        if token == b"obj" {
            let number = before
                .clone()
                .and_then(|range| std::str::from_utf8(&data[range]).ok())
                .and_then(|text| text.parse::<u32>().ok())
                .filter(|&n| n > 0);
            let generation = previous
                .clone()
                .and_then(|range| std::str::from_utf8(&data[range]).ok())
                .and_then(|text| text.parse::<u16>().ok());
            if let (Some(n), Some(g), Some(range)) = (number, generation, &before) {
                cands.push(Cand {
                    pos: range.start,
                    value: at,
                    kind: CandKind::Header(n, g),
                });
            }
        } else if token == b"trailer" {
            cands.push(Cand {
                pos: start,
                value: at,
                kind: CandKind::Trailer,
            });
        }
        let digits = token.len() <= 10 && token.iter().all(u8::is_ascii_digit);
        before = previous.take();
        previous = digits.then_some(start..at);
    }
    let mut endstreams = Vec::new();
    let mut from = 0;
    while let Some(found) = data
        .get(from..)
        .and_then(|rest| rest.windows(9).position(|w| w == b"endstream"))
    {
        endstreams.push(from + found);
        from += found + 9;
    }
    (cands, endstreams)
}

/// A stream's data: where it begins and where it may end, best first.
#[derive(Clone, Debug)]
struct Extent {
    start: usize,
    ends: Vec<usize>,
    /// Not given by a usable /Length, or begun after a nonstandard EOL.
    recovered: bool,
    /// An indirect /Length, checked once the objects are known.
    length_ref: Option<ObjectId>,
    /// Both end keywords after a right /Length are damaged (addendum A10):
    /// the data is kept, and the result is damaged.
    damaged: bool,
}

/// One definition of an object: top-level or an object-stream member.
#[derive(Clone, Debug)]
struct Def {
    id: ObjectId,
    pos: usize,
    /// 0 for a top-level definition, the member index plus one otherwise.
    sub: usize,
    /// The value reads completely (members always do).
    complete: bool,
    value: Object,
    stream: Option<Extent>,
    end: usize,
    member: bool,
}

impl Def {
    fn rank(&self) -> (bool, usize, usize) {
        (self.complete, self.pos, self.sub)
    }
}

enum Parsed {
    Value(Object, usize),
    Failed(usize),
}

impl Parsed {
    fn reached(&self) -> usize {
        match self {
            Self::Value(_, end) | Self::Failed(end) => *end,
        }
    }
}

#[derive(Default)]
struct Scan {
    defs: Vec<Def>,
    /// Headers whose value could not be read: number to generation.
    failed: BTreeMap<u32, u16>,
    failed_count: usize,
    trailers: Vec<(usize, Dictionary)>,
    /// What objects and trailers occupy, in file order.
    covered: Vec<Range<usize>>,
    max_number: u32,
}

impl Scan {
    fn has_containers(&self) -> bool {
        self.defs
            .iter()
            .any(|def| def.stream.is_some() && is_container(&def.value))
    }
}

#[derive(Clone, Default)]
struct TrailerEntries {
    root: Option<Object>,
    info: Option<Object>,
    id: Option<Object>,
    encrypt: Option<Object>,
}

impl TrailerEntries {
    /// Keeps the first non-null value of each entry it is given.
    fn set(&mut self, key: &[u8], value: &Object) {
        if value.is_null() {
            return;
        }
        let slot = match key {
            b"Root" => &mut self.root,
            b"Info" => &mut self.info,
            b"ID" => &mut self.id,
            b"Encrypt" => &mut self.encrypt,
            _ => return,
        };
        if slot.is_none() {
            *slot = Some(value.clone());
        }
    }
}

/// The document's encryption as the output keeps it.
#[derive(Default)]
struct Crypt {
    /// The trailer's /Encrypt as written (a reference names the object).
    entry: Option<Object>,
    /// The encryption dictionary, when /Encrypt names one.
    dict: Option<Dictionary>,
    reference: Option<ObjectId>,
    security: Option<Security>,
    /// Streams are AES-encrypted: their data is a whole number of blocks.
    aes: bool,
    id: Option<Object>,
}

struct Member {
    number: u32,
    value: Option<Object>,
}

/// The objects one revision holds, before the catalog and page tree.
struct Selection {
    objects: BTreeMap<u32, Def>,
    unreadable: BTreeMap<u32, u16>,
    trailer: TrailerEntries,
    crypt: Crypt,
    locked: bool,
    has_containers: bool,
    max_number: u32,
}

#[derive(Clone, Copy)]
enum Entry {
    Free,
    Used(usize),
    Packed(u32, usize),
}

struct Section {
    entries: Vec<(u32, Entry)>,
    trailer: Dictionary,
    /// Where the section's parts begin (bounds for the objects before them).
    starts: Vec<usize>,
    /// Positions of the xref streams the section was read from.
    streams: Vec<usize>,
    size: u64,
}

/// An object of the output.
struct OutObj {
    generation: u16,
    value: Object,
    data: Option<Range<usize>>,
    order: (usize, usize),
    synthesized: bool,
    member: bool,
}

impl OutObj {
    fn synthesized(generation: u16, value: Object) -> Self {
        Self {
            generation,
            value,
            data: None,
            order: (usize::MAX, 0),
            synthesized: true,
            member: false,
        }
    }
}

#[derive(Default)]
struct TreeOutcome {
    changed: bool,
    rebuilt: bool,
    blank: bool,
}

// ------------------------------------------------------------------ the repair

struct Repair<'a> {
    data: &'a [u8],
    header: Option<usize>,
    limits: RepairLimits,
    passwords: Vec<&'a [u8]>,
    report: RepairReport,
    values: usize,
    objects: usize,
    decoded: usize,
    containers: usize,
    budget: usize,
    endstreams: Vec<usize>,
    cands: Vec<Cand>,
    strong: Vec<Option<bool>>,
}

impl<'a> Repair<'a> {
    fn new(input: &'a [u8], limits: RepairLimits, passwords: Vec<&'a [u8]>) -> Self {
        let search = &input[..input.len().min(HEADER_SEARCH)];
        let header = search.windows(5).position(|w| w == b"%PDF-");
        let data = &input[header.unwrap_or(0)..];
        let (cands, endstreams) = raw_scan(data);
        let strong = vec![None; cands.len()];
        Self {
            data,
            header,
            limits,
            passwords,
            report: RepairReport::default(),
            values: 0,
            objects: 0,
            decoded: 0,
            containers: 0,
            budget: data
                .len()
                .saturating_mul(BUDGET_FACTOR)
                .saturating_add(BUDGET_FLOOR),
            endstreams,
            cands,
            strong,
        }
    }

    fn run(mut self) -> Result<RepairedPdf, RepairError> {
        let scan = self.scan()?;
        let selection = match self.chain(&scan)? {
            Some(selection) => selection,
            None => self.reconstruct(scan)?,
        };
        self.finish(selection)
    }

    // -------------------------------------------------------------- values

    /// One value read leniently from `input[start..end]`, within the
    /// per-value limits and what remains of `max_values`.
    fn parse_in(
        &self,
        input: &[u8],
        start: usize,
        end: usize,
        depth: usize,
    ) -> Result<Parsed, RepairError> {
        let Some(slice) = input.get(start..end.min(input.len())) else {
            return Ok(Parsed::Failed(start));
        };
        let left = self.limits.max_values.saturating_sub(self.values);
        let mut limits = self.limits.parse;
        limits.max_depth = limits.max_depth.min(depth);
        let bound = left < limits.max_objects;
        limits.max_objects = limits.max_objects.min(left);
        let mut parser = Parser::new(slice, limits)
            .with_duplicate_key_policy(DuplicateKeyPolicy::KeepLast)
            .with_lenient();
        match parser.parse_next() {
            Ok(Some(parsed)) => Ok(Parsed::Value(parsed.object, start + parsed.span.end)),
            Ok(None) => Ok(Parsed::Failed(start + slice.len())),
            Err(error) => {
                if bound
                    && matches!(
                        error.kind,
                        ParseErrorKind::LimitExceeded(ParseLimitKind::Objects)
                    )
                {
                    return Err(RepairError::LimitExceeded("values"));
                }
                Ok(Parsed::Failed(start + error.offset.min(slice.len())))
            }
        }
    }

    fn parse_value(&self, start: usize, end: usize, depth: usize) -> Result<Parsed, RepairError> {
        self.parse_in(self.data, start, end, depth)
    }

    fn charge_values(&mut self, value: &Object) -> Result<(), RepairError> {
        self.values = self.values.saturating_add(object_cost(value));
        if self.values > self.limits.max_values {
            return Err(RepairError::LimitExceeded("values"));
        }
        Ok(())
    }

    fn count_object(&mut self) -> Result<(), RepairError> {
        self.objects += 1;
        if self.objects > self.limits.max_objects {
            return Err(RepairError::LimitExceeded("objects"));
        }
        Ok(())
    }

    fn spend(&mut self, from: usize, reached: usize) {
        self.budget = self.budget.saturating_sub(reached.saturating_sub(from));
    }

    /// How far a value may be read: past the next candidate while the
    /// read-through budget lasts, else up to it.
    fn window(&self, index: usize) -> usize {
        if self.budget > 0 {
            self.data.len()
        } else {
            self.next_pos(index)
        }
    }

    /// The window for candidate `index`'s value that begins at `start`, and
    /// whether all of it counts as read. A value that begins with a number
    /// makes the parser read one or two tokens ahead (to find `N G R`), and
    /// that token may run to the end of the window: such a value is read no
    /// further than the next candidate, and the window is charged whole
    /// (addendum A5).
    fn value_window(&self, index: usize, start: usize) -> (usize, bool) {
        let next = self.next_pos(index);
        let first = skip_trivia(self.data, start, next);
        let numeric = self
            .data
            .get(first)
            .is_some_and(|&b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.'));
        if numeric {
            (next, true)
        } else {
            (self.window(index), false)
        }
    }

    fn next_pos(&self, index: usize) -> usize {
        self.cands
            .get(index + 1)
            .map_or(self.data.len(), |cand| cand.pos)
    }

    /// Where the first candidate at or after `at` begins.
    fn next_cand_from(&self, at: usize) -> usize {
        let index = self.cands.partition_point(|cand| cand.pos < at);
        self.cands
            .get(index)
            .map_or(self.data.len(), |cand| cand.pos)
    }

    /// /Length lands where a damaged `endstream` keyword and then `endobj`
    /// follow (section 4.6 rule 3): the end of `endobj`. Landing inside a
    /// real `endstream` keyword is not this case.
    fn damaged_endstream(&self, at: usize, limit: usize) -> Option<(usize, bool)> {
        let data = self.data;
        // Landing inside a real `endstream`, or just after one, is a /Length
        // that counted the keyword: not this case.
        let first = self.endstreams.partition_point(|&p| p < at);
        if first > 0
            && self
                .endstreams
                .get(first - 1)
                .is_some_and(|&p| at <= p + 9 + 2)
        {
            return None;
        }
        let token = |from: usize| {
            let start = skip_ws(data, from, limit);
            let mut end = start;
            while end < limit && is_regular(data[end]) {
                end += 1;
            }
            (start, end)
        };
        let (start, end) = token(at);
        if end == start
            || end - start > DAMAGED_KEYWORD
            || matches!(data.get(start..end), Some(b"endobj" | b"endstream"))
        {
            return None;
        }
        let next = skip_ws(data, end, limit);
        if keyword_at(data, next, b"endobj") && next + 6 <= limit {
            return Some((next + 6, false));
        }
        // Addendum A10: `endobj` damaged too (or missing), and the next
        // definition (or the end of what may be read) follows.
        let (second, close) = token(end);
        let after = if second < close && close - second <= DAMAGED_KEYWORD {
            close
        } else {
            end
        };
        let next = skip_ws(data, after, limit);
        let follows = next >= limit
            || read_header(data, next).is_some()
            || keyword_at(data, next, b"xref")
            || keyword_at(data, next, b"trailer")
            || keyword_at(data, next, b"startxref");
        follows.then_some((after, true))
    }

    fn cand_at(&self, at: usize) -> bool {
        self.cands
            .binary_search_by(|cand| cand.pos.cmp(&at))
            .is_ok()
    }

    // -------------------------------------------------------------- decoding

    /// `raw` through `specs` within what remains of `max_decoded_bytes`,
    /// and within `cap` when given. `None` when the data does not decode. An
    /// attempt that fails is charged what it may have produced (addendum A5):
    /// its cap, or the encoded size without one.
    fn decode(
        &mut self,
        raw: &[u8],
        specs: &[FilterSpec],
        lenient: bool,
        cap: Option<usize>,
    ) -> Result<Option<Vec<u8>>, RepairError> {
        let remaining = self.limits.max_decoded_bytes.saturating_sub(self.decoded);
        let exceeded = || RepairError::LimitExceeded("object-stream bytes");
        if specs.is_empty() {
            if raw.len() > remaining {
                return Err(exceeded());
            }
            self.decoded += raw.len();
            return Ok(Some(raw.to_vec()));
        }
        let limit = cap.map_or(remaining, |cap| cap.min(remaining));
        let limits = StreamLimits {
            max_encoded_bytes: raw.len(),
            max_decoded_bytes: limit,
            max_filters: 8,
            max_total_decoded_bytes: limit,
        };
        let result = if lenient {
            decode_filter_specs_lenient(raw, specs, limits)
        } else {
            decode_filter_specs_with_usage(raw, specs, limits)
        };
        let failed = |repair: &mut Self| -> Result<Option<Vec<u8>>, RepairError> {
            let charge = cap.map_or(raw.len(), |_| limit);
            repair.decoded = repair.decoded.saturating_add(charge);
            if repair.decoded > repair.limits.max_decoded_bytes {
                return Err(exceeded());
            }
            Ok(None)
        };
        match result {
            Ok(decoded) => {
                let used = decoded.usage.total_decoded_bytes.max(decoded.bytes.len());
                self.decoded = self.decoded.saturating_add(used);
                if self.decoded > self.limits.max_decoded_bytes {
                    return Err(exceeded());
                }
                Ok(Some(decoded.bytes))
            }
            Err(error)
                if matches!(
                    error.kind,
                    StreamErrorKind::DecodedOutputLimit { .. }
                        | StreamErrorKind::TotalOutputLimit { .. }
                ) =>
            {
                if limit < remaining {
                    failed(self)
                } else {
                    Err(exceeded())
                }
            }
            Err(_) => failed(self),
        }
    }

    /// Data from `start` to one of `ends` (best first) through `specs`,
    /// after `prepare` (decryption). One lenient decode of the first end
    /// gives the output's size; each end is then tried strictly, capped
    /// near that size, and the first that decodes is taken. Without one, the
    /// lenient output of the first end. Every attempt is charged.
    fn decode_ends(
        &mut self,
        start: usize,
        ends: &[usize],
        specs: &[FilterSpec],
        prepare: &dyn Fn(&'a [u8]) -> Cow<'a, [u8]>,
    ) -> Result<Option<(usize, Vec<u8>)>, RepairError> {
        let data = self.data;
        let Some(&first) = ends.first() else {
            return Ok(None);
        };
        let Some(raw) = data.get(start..first) else {
            return Ok(None);
        };
        let Some(probe) = self.decode(&prepare(raw), specs, true, None)? else {
            return Ok(None);
        };
        if ends.len() == 1 || specs.is_empty() {
            return Ok(Some((first, probe)));
        }
        let cap = probe.len().saturating_add(DECODE_SLACK);
        for &end in ends {
            let Some(raw) = data.get(start..end) else {
                continue;
            };
            if let Some(bytes) = self.decode(&prepare(raw), specs, false, Some(cap))? {
                return Ok(Some((end, bytes)));
            }
        }
        Ok(Some((first, probe)))
    }

    // -------------------------------------------------------------- stream extents

    /// The data of the stream whose `stream` keyword is at `keyword`, and
    /// where its definition ends. `bound` is the candidate the stream
    /// belongs to in the scan: its data then never runs past a later
    /// definition that is complete.
    fn stream_extent(
        &mut self,
        dict: &Dictionary,
        keyword: usize,
        limit: usize,
        bound: Option<usize>,
    ) -> Result<Option<(Extent, usize)>, RepairError> {
        let data = self.data;
        let limit = limit.min(data.len());
        let (start, standard, alternative) = data_start(data, keyword + 6);
        let length = dict.get(b"Length".as_slice());
        let declared = match length {
            Some(Object::Integer(n)) => usize::try_from(*n).ok(),
            _ => None,
        };
        let length_ref = match length {
            Some(Object::Reference(id)) => Some(*id),
            _ => None,
        };
        if let Some(declared) = declared {
            for begin in std::iter::once(start).chain(alternative) {
                let Some(end) = begin.checked_add(declared).filter(|&end| end <= limit) else {
                    continue;
                };
                if let Some(after) = endstream_at(data, end, limit) {
                    let extent = Extent {
                        start: begin,
                        ends: vec![end],
                        recovered: !standard || begin != start,
                        length_ref: None,
                        damaged: false,
                    };
                    let close = close_object(data, after, self.next_cand_from(after).min(limit));
                    return Ok(Some((extent, close)));
                }
                if let Some((after, damaged)) = self.damaged_endstream(end, limit) {
                    let extent = Extent {
                        start: begin,
                        ends: vec![end],
                        recovered: true,
                        length_ref: None,
                        damaged,
                    };
                    return Ok(Some((extent, after)));
                }
            }
        }
        let first = self.endstreams.partition_point(|&p| p < start);
        let found = self
            .endstreams
            .get(first)
            .copied()
            .filter(|&p| p.saturating_add(9) <= limit);
        if let Some(found) = found {
            if let Some(index) = bound {
                if self.complete_between(index, start, found)? {
                    return Ok(None);
                }
            }
            let extent = Extent {
                start,
                ends: eol_ends(data, start, found),
                recovered: true,
                length_ref,
                damaged: false,
            };
            let close = close_object(data, found + 9, self.next_cand_from(found + 9).min(limit));
            return Ok(Some((extent, close)));
        }
        // A file cut inside the data keeps it only when /Length ends there.
        if let Some(declared) = declared {
            if start.checked_add(declared) == Some(data.len()) && limit == data.len() {
                if let Some(index) = bound {
                    if self.complete_between(index, start, data.len())? {
                        return Ok(None);
                    }
                }
                let extent = Extent {
                    start,
                    ends: vec![data.len()],
                    recovered: true,
                    length_ref: None,
                    damaged: false,
                };
                return Ok(Some((extent, data.len())));
            }
        }
        Ok(None)
    }

    /// Whether a candidate after `index` that begins in `from..to` is a
    /// complete definition (its value followed by `endobj` or `stream`).
    fn complete_between(
        &mut self,
        index: usize,
        from: usize,
        to: usize,
    ) -> Result<bool, RepairError> {
        let first = self.cands.partition_point(|cand| cand.pos < from);
        let mut at = first.max(index + 1);
        while let Some(cand) = self.cands.get(at).copied() {
            if cand.pos >= to {
                break;
            }
            self.budget = self.budget.saturating_sub(16);
            if self.strong(at)? {
                return Ok(true);
            }
            at += 1;
        }
        Ok(false)
    }

    /// Whether candidate `index`, read up to the next candidate, is a
    /// complete definition. Cached: each candidate is tested once.
    fn strong(&mut self, index: usize) -> Result<bool, RepairError> {
        if let Some(known) = self.strong.get(index).copied().flatten() {
            return Ok(known);
        }
        let Some(cand) = self.cands.get(index).copied() else {
            return Ok(false);
        };
        let result = match cand.kind {
            CandKind::Trailer => false,
            CandKind::Header(..) => {
                let end = self.next_pos(index);
                match self.parse_value(cand.value, end, self.limits.parse.max_depth)? {
                    Parsed::Value(value, after) => {
                        let next = skip_trivia(self.data, after, end);
                        keyword_at(self.data, next, b"endobj")
                            || (matches!(value, Object::Dictionary(_))
                                && starts_at(self.data, next, end, b"stream"))
                    }
                    Parsed::Failed(_) => false,
                }
            }
        };
        if let Some(slot) = self.strong.get_mut(index) {
            *slot = Some(result);
        }
        Ok(result)
    }

    // -------------------------------------------------------------- the scan

    fn scan(&mut self) -> Result<Scan, RepairError> {
        let mut scan = Scan::default();
        let mut covered_until = 0;
        for index in 0..self.cands.len() {
            let cand = self.cands[index];
            if let CandKind::Header(number, _) = cand.kind {
                scan.max_number = scan.max_number.max(number);
            }
            if cand.pos < covered_until {
                continue;
            }
            let next = self.next_pos(index);
            match cand.kind {
                CandKind::Trailer => match self.eval_trailer(index)? {
                    Some((dict, end)) => {
                        self.charge_values(&Object::Dictionary(dict.clone()))?;
                        scan.trailers.push((cand.pos, dict));
                        scan.covered.push(cand.pos..end);
                        covered_until = end;
                    }
                    None => scan.covered.push(cand.pos..next),
                },
                CandKind::Header(number, generation) => match self.eval_header(index)? {
                    Some(def) => {
                        self.count_object()?;
                        if def.complete {
                            covered_until = def.end;
                            scan.covered.push(cand.pos..def.end);
                        } else {
                            scan.covered.push(cand.pos..def.end.max(next));
                        }
                        scan.defs.push(def);
                    }
                    None => {
                        scan.failed.insert(number, generation);
                        scan.failed_count += 1;
                        scan.covered.push(cand.pos..next);
                    }
                },
            }
        }
        Ok(scan)
    }

    fn eval_trailer(&mut self, index: usize) -> Result<Option<(Dictionary, usize)>, RepairError> {
        let cand = self.cands[index];
        let (window, whole) = self.value_window(index, cand.value);
        let parsed = self.parse_value(cand.value, window, self.limits.parse.max_depth)?;
        self.spend(cand.value, if whole { window } else { parsed.reached() });
        match parsed {
            Parsed::Value(Object::Dictionary(dict), end) => Ok(Some((dict, end))),
            _ => Ok(None),
        }
    }

    /// Candidate `index` read as a definition. `None` when its value (or
    /// its stream's data) cannot be read.
    fn eval_header(&mut self, index: usize) -> Result<Option<Def>, RepairError> {
        let cand = self.cands[index];
        let CandKind::Header(number, generation) = cand.kind else {
            return Ok(None);
        };
        let Ok(id) = ObjectId::new(number, generation) else {
            return Ok(None);
        };
        let data = self.data;
        let (window, whole) = self.value_window(index, cand.value);
        let parsed = self.parse_value(cand.value, window, self.limits.parse.max_depth)?;
        self.spend(cand.value, if whole { window } else { parsed.reached() });
        let Parsed::Value(value, end) = parsed else {
            return Ok(None);
        };
        // A comment that runs on is read no further than the next candidate.
        let next = skip_trivia(data, end, self.next_cand_from(end));
        let mut def = Def {
            id,
            pos: cand.pos,
            sub: 0,
            complete: true,
            value,
            stream: None,
            end,
            member: false,
        };
        if keyword_at(data, next, b"endobj") {
            def.end = next + 6;
        } else if starts_at(data, next, data.len(), b"stream") {
            let Object::Dictionary(dict) = &def.value else {
                def.complete = false;
                self.charge_values(&def.value)?;
                return Ok(Some(def));
            };
            match self.stream_extent(dict, next, data.len(), Some(index))? {
                Some((extent, after)) => {
                    def.stream = Some(extent);
                    def.end = after;
                }
                None => return Ok(None),
            }
        } else {
            def.complete = next >= data.len()
                || self.cand_at(next)
                || keyword_at(data, next, b"xref")
                || keyword_at(data, next, b"startxref");
        }
        self.charge_values(&def.value)?;
        Ok(Some(def))
    }

    /// Dictionaries outside every object that hold a trailer entry: read as
    /// trailers during reconstruction (section 4.4 item 7).
    fn loose_dictionaries(
        &mut self,
        covered: &[Range<usize>],
    ) -> Result<Vec<(usize, Dictionary)>, RepairError> {
        let data = self.data;
        let mut gaps = Vec::new();
        let mut at = 0;
        for region in covered {
            if region.start > at {
                gaps.push(at..region.start);
            }
            at = at.max(region.end);
        }
        if at < data.len() {
            gaps.push(at..data.len());
        }
        let mut found = Vec::new();
        for gap in gaps {
            let mut at = gap.start;
            while at < gap.end && self.budget > 0 {
                let Some(open) = data[at..gap.end]
                    .windows(2)
                    .position(|w| w == b"<<")
                    .map(|p| at + p)
                else {
                    break;
                };
                let parsed = self.parse_value(open, gap.end, LOOSE_DEPTH)?;
                self.spend(open, parsed.reached());
                match parsed {
                    Parsed::Value(value, end) => {
                        let keeps = value.as_dict().is_some_and(|dict| {
                            [b"Root".as_slice(), b"Info", b"ID", b"Encrypt"]
                                .iter()
                                .any(|key| dict.contains_key(*key))
                        });
                        if keeps {
                            self.charge_values(&value)?;
                            if let Object::Dictionary(dict) = value {
                                found.push((open, dict));
                            }
                        }
                        at = end.max(open + 2);
                    }
                    Parsed::Failed(_) => at = open + 2,
                }
            }
        }
        Ok(found)
    }

    // -------------------------------------------------------------- the chain

    /// The revision the file's own cross-reference chain describes, when it
    /// is usable (section 4.3). Fills `xref_sections` and `xref_size`.
    fn chain(&mut self, scan: &Scan) -> Result<Option<Selection>, RepairError> {
        let Some(start) = crate::incremental::startxref(self.data) else {
            return Ok(None);
        };
        let saved = (
            self.values,
            self.objects,
            self.decoded,
            self.containers,
            self.report.objects_dropped,
        );
        self.values = 0;
        self.objects = 0;
        let result = self.chain_from(start, scan)?;
        if result.is_none() {
            (
                self.values,
                self.objects,
                self.decoded,
                self.containers,
                self.report.objects_dropped,
            ) = saved;
            self.report.object_streams_expanded = 0;
            self.report.objects_unreadable = 0;
        }
        Ok(result)
    }

    fn chain_from(&mut self, start: usize, scan: &Scan) -> Result<Option<Selection>, RepairError> {
        let data = self.data;
        let mut sections: Vec<Section> = Vec::new();
        let mut visited = BTreeSet::new();
        let mut next = Some(start);
        let mut usable = true;
        let mut entries = 0usize;
        let mut looped = false;
        while let Some(offset) = next {
            if !visited.insert(offset) {
                looped = true;
                break;
            }
            if sections.len() >= self.limits.max_revisions {
                usable = false;
                break;
            }
            let Some(section) = self.read_section(offset)? else {
                usable = false;
                break;
            };
            entries = entries.saturating_add(section.entries.len());
            if entries > self.limits.max_objects {
                usable = false;
                break;
            }
            next = match section.trailer.get(b"Prev".as_slice()) {
                None | Some(Object::Null) => None,
                Some(Object::Integer(previous)) => match usize::try_from(*previous) {
                    Ok(previous) => Some(previous),
                    Err(_) => {
                        usable = false;
                        None
                    }
                },
                Some(_) => {
                    usable = false;
                    None
                }
            };
            sections.push(section);
        }
        self.report.xref_sections = sections.len();
        self.report.xref_size = sections
            .iter()
            .map(|section| {
                let listed = section
                    .entries
                    .iter()
                    .map(|(number, _)| u64::from(*number) + 1)
                    .max()
                    .unwrap_or(0);
                section.size.max(listed)
            })
            .max()
            .unwrap_or(0);
        if !usable || sections.is_empty() {
            return Ok(None);
        }

        // Addendum A8: a chain that ends at a /Prev loop is usable only when
        // its sections give an entry for every number below the newest /Size.
        if looped {
            let size = sections.first().map_or(0, |section| section.size);
            let mut listed: BTreeSet<u32> = BTreeSet::new();
            for section in &sections {
                listed.extend(
                    section
                        .entries
                        .iter()
                        .map(|(number, _)| *number)
                        .filter(|&number| u64::from(number) < size),
                );
            }
            if (listed.len() as u64) < size {
                return Ok(None);
            }
        }

        // Newest first: an entry hides every older entry for its number.
        let mut merged: BTreeMap<u32, Entry> = BTreeMap::new();
        for section in &sections {
            for &(number, entry) in &section.entries {
                merged.entry(number).or_insert(entry);
            }
        }
        if merged
            .get(&0)
            .is_some_and(|entry| !matches!(entry, Entry::Free))
        {
            return Ok(None);
        }
        let mut bounds: BTreeSet<usize> = BTreeSet::from([data.len()]);
        let mut structural: BTreeSet<usize> = BTreeSet::new();
        for section in &sections {
            bounds.extend(section.starts.iter().copied());
            structural.extend(section.streams.iter().copied());
            for (_, entry) in &section.entries {
                if let Entry::Used(offset) = entry {
                    bounds.insert(*offset);
                }
            }
        }
        let mut located: BTreeMap<u32, (usize, u16, usize)> = BTreeMap::new();
        let mut packed: BTreeMap<u32, Vec<(u32, usize)>> = BTreeMap::new();
        for (&number, &entry) in &merged {
            match entry {
                Entry::Free => {}
                Entry::Used(offset) => {
                    let Some(found) = locate(data, offset, number) else {
                        return Ok(None);
                    };
                    bounds.insert(found.0);
                    located.insert(number, found);
                }
                Entry::Packed(container, index) => {
                    packed.entry(container).or_default().push((number, index));
                }
            }
        }
        if packed
            .keys()
            .any(|container| !located.contains_key(container))
        {
            return Ok(None);
        }

        let mut objects: BTreeMap<u32, Def> = BTreeMap::new();
        let mut unreadable: BTreeMap<u32, u16> = BTreeMap::new();
        for (&number, &(pos, generation, value)) in &located {
            let end = bounds
                .range(pos + 1..)
                .next()
                .copied()
                .unwrap_or(data.len());
            let Ok(id) = ObjectId::new(number, generation) else {
                return Ok(None);
            };
            match self.read_located(id, pos, value, end)? {
                Some(def) => {
                    self.count_object()?;
                    objects.insert(number, def);
                }
                None => {
                    self.report.objects_unreadable += 1;
                    unreadable.insert(number, generation);
                }
            }
        }

        let mut trailer = TrailerEntries::default();
        for section in &sections {
            for (key, value) in section.trailer.iter() {
                trailer.set(key, value);
            }
        }
        // Addendum A4: /Encrypt naming an object the chain does not hold
        // makes the chain unusable; reconstruction looks for the dictionary
        // by type, and never shows ciphertext as text (P5).
        if let Some(Object::Reference(id)) = &trailer.encrypt {
            if !objects.contains_key(&id.number()) {
                return Ok(None);
            }
        }
        let crypt = {
            let lookup = |number: u32| objects.get(&number).map(|def| (def.id, &def.value));
            setup_encryption(
                trailer.encrypt.as_ref(),
                trailer.id.as_ref(),
                &lookup,
                &self.passwords,
            )?
        };
        let has_containers = !packed.is_empty() || scan.has_containers();
        let locked = crypt.dict.is_some() && crypt.security.is_none() && has_containers;

        let mut members: BTreeMap<u32, Def> = BTreeMap::new();
        let mut superseded = 0usize;
        for (container, wanted) in &packed {
            let Some(holder) = objects.get(container).cloned() else {
                // The container is located but does not read.
                for &(number, _) in wanted {
                    self.report.objects_unreadable += 1;
                    unreadable.insert(number, 0);
                }
                continue;
            };
            if holder.stream.is_none() {
                return Ok(None);
            }
            if locked {
                continue;
            }
            let Some(list) = self.expand(&holder, crypt.security.as_ref())? else {
                return Ok(None);
            };
            self.report.object_streams_expanded += 1;
            let mut taken = BTreeSet::new();
            for &(number, index) in wanted {
                let Some(member) = list.get(index) else {
                    return Ok(None);
                };
                if member.number != number {
                    return Ok(None);
                }
                taken.insert(index);
                match &member.value {
                    Some(value) => {
                        let Ok(id) = ObjectId::new(number, 0) else {
                            return Ok(None);
                        };
                        members.insert(
                            number,
                            Def {
                                id,
                                pos: holder.pos,
                                sub: index + 1,
                                complete: true,
                                value: value.clone(),
                                stream: None,
                                end: holder.end,
                                member: true,
                            },
                        );
                    }
                    None => {
                        self.report.objects_unreadable += 1;
                        unreadable.insert(number, 0);
                    }
                }
            }
            superseded += list
                .iter()
                .enumerate()
                .filter(|(index, member)| member.value.is_some() && !taken.contains(index))
                .count();
        }

        // Structural objects are not written: xref streams, containers (unless
        // locked), and linearization with its hint streams.
        let hints: Vec<usize> = objects
            .values()
            .filter(|def| is_linearization(&def.value))
            .flat_map(|def| hint_offsets(&def.value))
            .collect();
        let selected: BTreeSet<usize> = located.values().map(|found| found.0).collect();
        let mut kept: BTreeMap<u32, Def> = BTreeMap::new();
        for (number, def) in objects {
            let xref = structural.contains(&def.pos)
                || (def.stream.is_some() && type_is(&def.value, b"XRef"));
            let container = !locked
                && def.stream.is_some()
                && (packed.contains_key(&number) || type_is(&def.value, b"ObjStm"));
            let linearization = is_linearization(&def.value);
            let hint = def.stream.is_some()
                && hints
                    .iter()
                    .any(|&offset| begins_at(self.data, def.pos, offset));
            if linearization {
                self.report.linearized = true;
            }
            if !(xref || container || linearization || hint) {
                kept.insert(number, def);
            }
        }
        kept.extend(members);

        self.report.objects_overridden = superseded
            + scan
                .defs
                .iter()
                .filter(|def| !selected.contains(&def.pos) && !structural_def(def))
                .count();
        self.report.xref_intact = true;
        let max_number = merged
            .keys()
            .copied()
            .max()
            .unwrap_or(0)
            .max(scan.max_number);
        Ok(Some(Selection {
            objects: kept,
            unreadable,
            trailer,
            crypt,
            locked,
            has_containers,
            max_number,
        }))
    }

    /// The object whose header the chain located at `pos`, read up to
    /// `end` (the next place the chain locates something).
    fn read_located(
        &mut self,
        id: ObjectId,
        pos: usize,
        value_start: usize,
        end: usize,
    ) -> Result<Option<Def>, RepairError> {
        let data = self.data;
        let Parsed::Value(value, after) =
            self.parse_value(value_start, end, self.limits.parse.max_depth)?
        else {
            return Ok(None);
        };
        let next = skip_trivia(data, after, end);
        let mut def = Def {
            id,
            pos,
            sub: 0,
            complete: true,
            value,
            stream: None,
            end: after,
            member: false,
        };
        if let Object::Dictionary(dict) = &def.value {
            if starts_at(data, next, end, b"stream") {
                match self.stream_extent(dict, next, end, None)? {
                    Some((extent, close)) => {
                        def.stream = Some(extent);
                        def.end = close;
                    }
                    None => return Ok(None),
                }
            }
        }
        self.charge_values(&def.value)?;
        Ok(Some(def))
    }

    /// The cross-reference section at `offset` (classic or stream).
    fn read_section(&mut self, offset: usize) -> Result<Option<Section>, RepairError> {
        let data = self.data;
        if offset >= data.len() {
            return Ok(None);
        }
        let at = skip_ws(data, offset, offset.saturating_add(OFFSET_SLACK + 1));
        if keyword_at(data, at, b"xref") {
            return self.classic_section(at);
        }
        match read_header(data, at) {
            Some((_, _, value)) => self.stream_section(at, value),
            None => Ok(None),
        }
    }

    fn classic_section(&mut self, at: usize) -> Result<Option<Section>, RepairError> {
        let data = self.data;
        let mut cursor = at + 4;
        let mut entries: Vec<(u32, Entry)> = Vec::new();
        loop {
            cursor = skip_trivia(data, cursor, data.len());
            if keyword_at(data, cursor, b"trailer") {
                cursor += 7;
                break;
            }
            let Some((first, end)) = raw_uint(data, cursor) else {
                return Ok(None);
            };
            cursor = skip_ws(data, end, data.len());
            let Some((count, end)) = raw_uint(data, cursor) else {
                return Ok(None);
            };
            cursor = end;
            let mut row = 0u64;
            while row < count {
                cursor = skip_ws(data, cursor, data.len());
                let Some((offset, end)) = raw_uint(data, cursor) else {
                    return Ok(None);
                };
                cursor = skip_ws(data, end, data.len());
                let Some((generation, end)) = raw_uint(data, cursor) else {
                    return Ok(None);
                };
                cursor = skip_ws(data, end, data.len());
                let used = if keyword_at(data, cursor, b"n") {
                    true
                } else if keyword_at(data, cursor, b"f") {
                    false
                } else {
                    return Ok(None);
                };
                cursor += 1;
                let Some(number) = first.checked_add(row).and_then(|n| u32::try_from(n).ok())
                else {
                    return Ok(None);
                };
                let entry = if used {
                    let (Ok(offset), Ok(_)) = (usize::try_from(offset), u16::try_from(generation))
                    else {
                        return Ok(None);
                    };
                    Entry::Used(offset)
                } else {
                    Entry::Free
                };
                if entries.len() >= self.limits.max_objects {
                    return Ok(None);
                }
                entries.push((number, entry));
                row += 1;
            }
        }
        let Parsed::Value(Object::Dictionary(trailer), _) =
            self.parse_value(cursor, data.len(), self.limits.parse.max_depth)?
        else {
            return Ok(None);
        };
        self.charge_values(&Object::Dictionary(trailer.clone()))?;
        let mut section = Section {
            entries,
            size: size_of(&trailer),
            trailer,
            starts: vec![at],
            streams: Vec::new(),
        };
        // A hybrid file (§7.5.8.4): the table's in-use entries come first,
        // the stream supplies numbers the table omits or marks free.
        match section.trailer.get(b"XRefStm".as_slice()) {
            None | Some(Object::Null) => {}
            Some(Object::Integer(offset)) => {
                let Ok(offset) = usize::try_from(*offset) else {
                    return Ok(None);
                };
                if offset >= data.len() {
                    return Ok(None);
                }
                let at = skip_ws(data, offset, offset.saturating_add(OFFSET_SLACK + 1));
                let Some((_, _, value)) = read_header(data, at) else {
                    return Ok(None);
                };
                let Some(stream) = self.stream_section(at, value)? else {
                    return Ok(None);
                };
                let in_use = |entry: &Entry| !matches!(entry, Entry::Free);
                let used: BTreeSet<u32> = section
                    .entries
                    .iter()
                    .filter(|(_, entry)| in_use(entry))
                    .map(|(number, _)| *number)
                    .collect();
                let supplied: Vec<(u32, Entry)> = stream
                    .entries
                    .iter()
                    .filter(|(number, entry)| !used.contains(number) && in_use(entry))
                    .copied()
                    .collect();
                let listed: BTreeSet<u32> = supplied.iter().map(|(number, _)| *number).collect();
                let mut merged: Vec<(u32, Entry)> = section
                    .entries
                    .iter()
                    .filter(|(number, entry)| in_use(entry) || !listed.contains(number))
                    .copied()
                    .collect();
                merged.extend(supplied);
                section.entries = merged;
                section.starts.extend(stream.starts);
                section.streams.extend(stream.streams);
                section.size = section.size.max(stream.size);
            }
            Some(_) => return Ok(None),
        }
        Ok(Some(section))
    }

    /// A cross-reference stream whose object header is at `at` (§7.5.8).
    fn stream_section(&mut self, at: usize, value: usize) -> Result<Option<Section>, RepairError> {
        let data = self.data;
        let Parsed::Value(Object::Dictionary(dict), after) =
            self.parse_value(value, data.len(), self.limits.parse.max_depth)?
        else {
            return Ok(None);
        };
        let next = skip_trivia(data, after, data.len());
        if !starts_at(data, next, data.len(), b"stream") {
            return Ok(None);
        }
        let Some((extent, _)) = self.stream_extent(&dict, next, data.len(), None)? else {
            return Ok(None);
        };
        let Some(Object::Array(widths)) = dict.get(b"W".as_slice()) else {
            return Ok(None);
        };
        if widths.len() != 3 {
            return Ok(None);
        }
        let mut w = [0usize; 3];
        for (slot, width) in w.iter_mut().zip(widths) {
            match width.as_i64().and_then(|n| usize::try_from(n).ok()) {
                Some(n) if n <= 8 => *slot = n,
                _ => return Ok(None),
            }
        }
        let row = w[0] + w[1] + w[2];
        if row == 0 {
            return Ok(None);
        }
        let size = size_of(&dict);
        let index: Vec<(u64, u64)> = match dict.get(b"Index".as_slice()) {
            None | Some(Object::Null) => vec![(0, size)],
            Some(Object::Array(items)) if items.len() % 2 == 0 => {
                let mut pairs = Vec::new();
                for pair in items.chunks(2) {
                    let (Some(start), Some(count)) = (
                        pair.first().and_then(Object::as_i64),
                        pair.get(1).and_then(Object::as_i64),
                    ) else {
                        return Ok(None);
                    };
                    let (Ok(start), Ok(count)) = (u64::try_from(start), u64::try_from(count))
                    else {
                        return Ok(None);
                    };
                    pairs.push((start, count));
                }
                pairs
            }
            _ => return Ok(None),
        };
        let Some(specs) = filter_specs(&dict) else {
            return Ok(None);
        };
        let plain = |raw: &'a [u8]| Cow::Borrowed(raw);
        let Some((_, rows)) = self.decode_ends(extent.start, &extent.ends, &specs, &plain)? else {
            return Ok(None);
        };
        let mut entries = Vec::new();
        let mut taken = 0usize;
        for (start, count) in index {
            let mut k = 0u64;
            while k < count {
                let Some(bytes) = rows.get(taken * row..(taken + 1) * row) else {
                    return Ok(None);
                };
                taken += 1;
                let field = |from: usize, width: usize| {
                    bytes[from..from + width]
                        .iter()
                        .fold(0u64, |n, &b| (n << 8) | u64::from(b))
                };
                let kind = if w[0] == 0 { 1 } else { field(0, w[0]) };
                let second = field(w[0], w[1]);
                let third = field(w[0] + w[1], w[2]);
                let Some(number) = start.checked_add(k).and_then(|n| u32::try_from(n).ok()) else {
                    return Ok(None);
                };
                let entry = match kind {
                    1 => {
                        let (Ok(offset), Ok(_)) = (usize::try_from(second), u16::try_from(third))
                        else {
                            return Ok(None);
                        };
                        Entry::Used(offset)
                    }
                    2 => {
                        let (Ok(container), Ok(index)) =
                            (u32::try_from(second), usize::try_from(third))
                        else {
                            return Ok(None);
                        };
                        Entry::Packed(container, index)
                    }
                    _ => Entry::Free,
                };
                if entries.len() >= self.limits.max_objects {
                    return Ok(None);
                }
                entries.push((number, entry));
                k += 1;
            }
        }
        self.charge_values(&Object::Dictionary(dict.clone()))?;
        Ok(Some(Section {
            entries,
            trailer: dict,
            starts: vec![at],
            streams: vec![at],
            size,
        }))
    }

    // -------------------------------------------------------------- object streams

    /// The members of a container (§7.5.7), in its header's order. `None`
    /// when the container does not decode.
    fn expand(
        &mut self,
        container: &Def,
        security: Option<&Security>,
    ) -> Result<Option<Vec<Member>>, RepairError> {
        self.containers += 1;
        if self.containers > self.limits.max_object_streams {
            return Err(RepairError::LimitExceeded("object streams"));
        }
        let (Object::Dictionary(dict), Some(extent)) = (&container.value, &container.stream) else {
            return Ok(None);
        };
        let (Some(count), Some(first)) = (integer(dict, b"N"), integer(dict, b"First")) else {
            return Ok(None);
        };
        let (Ok(count), Ok(first)) = (usize::try_from(count), usize::try_from(first)) else {
            return Ok(None);
        };
        let Some(specs) = filter_specs(dict) else {
            return Ok(None);
        };
        let plain = |raw: &'a [u8]| -> Cow<'a, [u8]> {
            match security {
                Some(security) => {
                    Cow::Owned(security.decrypt(security.stream_method(false), container.id, raw))
                }
                None => Cow::Borrowed(raw),
            }
        };
        let Some((_, bytes)) = self.decode_ends(extent.start, &extent.ends, &specs, &plain)? else {
            return Ok(None);
        };
        let Some(head) = bytes.get(..first) else {
            return Ok(None);
        };
        let mut lexer = Lexer::new(head, self.limits.parse.lex);
        let mut pairs: Vec<(i64, i64)> = Vec::new();
        while pairs.len() < count {
            let number = lexer.next_token();
            let offset = lexer.next_token();
            let (Ok(Some(number)), Ok(Some(offset))) = (number, offset) else {
                break;
            };
            let (TokenKind::Integer(number), TokenKind::Integer(offset)) =
                (number.kind, offset.kind)
            else {
                break;
            };
            pairs.push((number, offset));
        }
        let body = &bytes[first..];
        let mut offsets: Vec<usize> = pairs
            .iter()
            .filter_map(|&(_, offset)| usize::try_from(offset).ok())
            .filter(|&offset| offset <= body.len())
            .collect();
        offsets.sort_unstable();
        offsets.dedup();
        let mut read = BTreeSet::new();
        let mut members = Vec::with_capacity(pairs.len());
        for &(number, offset) in &pairs {
            let number = u32::try_from(number).ok().filter(|&n| n > 0);
            let offset = usize::try_from(offset)
                .ok()
                .filter(|&offset| offset <= body.len());
            let value = match (number, offset) {
                (Some(_), Some(offset)) if read.insert(offset) => {
                    let next = offsets
                        .get(offsets.partition_point(|&o| o <= offset))
                        .copied()
                        .unwrap_or(body.len());
                    match self.parse_in(body, offset, next, self.limits.parse.max_depth)? {
                        Parsed::Value(value, _) => Some(value),
                        Parsed::Failed(_) => None,
                    }
                }
                _ => None,
            };
            match &value {
                Some(value) => {
                    self.charge_values(value)?;
                    self.count_object()?;
                }
                None => self.report.objects_dropped += 1,
            }
            members.push(Member {
                number: number.unwrap_or(0),
                value,
            });
        }
        Ok(Some(members))
    }

    // -------------------------------------------------------------- reconstruction

    /// The document rebuilt from the definitions in the bytes (section 4.4).
    fn reconstruct(&mut self, scan: Scan) -> Result<Selection, RepairError> {
        self.report.xref_intact = false;
        self.report.objects_dropped += scan.failed_count;
        let loose = self.loose_dictionaries(&scan.covered)?;
        let has_containers = scan.has_containers();
        let Scan {
            defs,
            failed,
            trailers,
            max_number,
            ..
        } = scan;

        // Trailers: `trailer` dictionaries, xref-stream dictionaries and
        // loose dictionaries; the latest in file order that has an entry
        // gives it.
        let mut all: Vec<(usize, &Dictionary)> = Vec::new();
        all.extend(trailers.iter().map(|(pos, dict)| (*pos, dict)));
        all.extend(loose.iter().map(|(pos, dict)| (*pos, dict)));
        for def in &defs {
            if def.stream.is_some() && type_is(&def.value, b"XRef") {
                if let Object::Dictionary(dict) = &def.value {
                    all.push((def.pos, dict));
                }
            }
        }
        all.sort_by_key(|(pos, _)| std::cmp::Reverse(*pos));
        let mut trailer = TrailerEntries::default();
        for (_, dict) in &all {
            for (key, value) in dict.iter() {
                trailer.set(key, value);
            }
        }

        // The best top-level definition of each number, for the encryption
        // dictionary and its indirect entries.
        let mut top: BTreeMap<u32, usize> = BTreeMap::new();
        for (index, def) in defs.iter().enumerate() {
            let number = def.id.number();
            if top
                .get(&number)
                .is_none_or(|&best| defs[best].rank() < def.rank())
            {
                top.insert(number, index);
            }
        }
        let named = match &trailer.encrypt {
            None => false,
            Some(Object::Reference(id)) => top.contains_key(&id.number()),
            Some(_) => true,
        };
        if !named {
            // The trailer was lost, or names an object that was: an
            // encryption dictionary found by its type, so ciphertext is never
            // read as plain text.
            let found = defs
                .iter()
                .filter(|def| def.stream.is_none())
                .filter(|def| {
                    def.value.as_dict().is_some_and(|dict| {
                        matches!(dict.get(b"Filter".as_slice()), Some(Object::Name(n)) if n == b"Standard")
                            && dict.contains_key(b"O".as_slice())
                            && dict.contains_key(b"U".as_slice())
                    })
                })
                .max_by_key(|def| def.rank());
            match found {
                Some(def) => trailer.encrypt = Some(Object::Reference(def.id)),
                // /Encrypt names a dictionary that is gone and none is found:
                // the strings and streams cannot be read (P5, addendum A4).
                None if trailer.encrypt.is_some() => {
                    return Err(RepairError::Invalid("encryption dictionary"));
                }
                None => {}
            }
        }
        let crypt = {
            let lookup = |number: u32| {
                top.get(&number)
                    .map(|&index| (defs[index].id, &defs[index].value))
            };
            setup_encryption(
                trailer.encrypt.as_ref(),
                trailer.id.as_ref(),
                &lookup,
                &self.passwords,
            )?
        };
        let locked = crypt.dict.is_some() && crypt.security.is_none() && has_containers;

        // Members compete at their container's position.
        let mut members = Vec::new();
        if !locked {
            for def in &defs {
                if def.stream.is_none() || !is_container(&def.value) {
                    continue;
                }
                match self.expand(def, crypt.security.as_ref())? {
                    Some(list) => {
                        self.report.object_streams_expanded += 1;
                        for (index, member) in list.into_iter().enumerate() {
                            let (Some(value), Ok(id)) =
                                (member.value, ObjectId::new(member.number, 0))
                            else {
                                continue;
                            };
                            members.push(Def {
                                id,
                                pos: def.pos,
                                sub: index + 1,
                                complete: true,
                                value,
                                stream: None,
                                end: def.end,
                                member: true,
                            });
                        }
                    }
                    None => self.report.objects_dropped += 1,
                }
            }
        }

        let hints: Vec<usize> = defs
            .iter()
            .filter(|def| is_linearization(&def.value))
            .flat_map(|def| hint_offsets(&def.value))
            .collect();
        let mut candidates: Vec<Def> = Vec::new();
        for def in defs {
            let xref = def.stream.is_some() && type_is(&def.value, b"XRef");
            let container = !locked && def.stream.is_some() && is_container(&def.value);
            let linearization = is_linearization(&def.value);
            let hint = def.stream.is_some()
                && hints
                    .iter()
                    .any(|&offset| begins_at(self.data, def.pos, offset));
            if linearization {
                self.report.linearized = true;
            }
            if !(xref || container || linearization || hint) {
                candidates.push(def);
            }
        }
        candidates.extend(members);
        let mut winners: BTreeMap<u32, usize> = BTreeMap::new();
        for (index, def) in candidates.iter().enumerate() {
            let number = def.id.number();
            match winners.get(&number) {
                Some(&best) => {
                    self.report.objects_overridden += 1;
                    if candidates[best].rank() < def.rank() {
                        winners.insert(number, index);
                    }
                }
                None => {
                    winners.insert(number, index);
                }
            }
        }
        let chosen: BTreeSet<usize> = winners.values().copied().collect();
        let objects: BTreeMap<u32, Def> = candidates
            .into_iter()
            .enumerate()
            .filter(|(index, _)| chosen.contains(index))
            .map(|(_, def)| (def.id.number(), def))
            .collect();
        Ok(Selection {
            objects,
            unreadable: failed,
            trailer,
            crypt,
            locked,
            has_containers,
            max_number,
        })
    }

    // -------------------------------------------------------------- output

    fn finish(&mut self, selection: Selection) -> Result<RepairedPdf, RepairError> {
        let Selection {
            objects,
            unreadable,
            trailer,
            crypt,
            locked,
            has_containers,
            max_number,
        } = selection;
        self.report.locked = locked;
        self.report.password_dependent = crypt.dict.is_some() && has_containers;

        let lengths: BTreeMap<u32, i64> = objects
            .iter()
            .filter_map(|(number, def)| def.value.as_i64().map(|n| (*number, n)))
            .collect();
        let mut out: BTreeMap<u32, OutObj> = BTreeMap::new();
        for (number, def) in objects {
            let data = match &def.stream {
                Some(extent) => {
                    self.report.damaged |= extent.damaged;
                    let (range, recovered) = self.resolve_extent(&def, extent, &lengths, &crypt)?;
                    if recovered {
                        self.report.streams_length_recovered += 1;
                    }
                    Some(range)
                }
                None => None,
            };
            out.insert(
                number,
                OutObj {
                    generation: def.id.generation(),
                    value: def.value,
                    data,
                    order: (def.pos, def.sub),
                    synthesized: false,
                    member: def.member,
                },
            );
        }
        let mut next = u64::from(max_number)
            .max(out.keys().next_back().map_or(0, |&n| u64::from(n)))
            .max(unreadable.keys().next_back().map_or(0, |&n| u64::from(n)))
            + 1;
        let mut allocate = move || -> Result<u32, RepairError> {
            let number = u32::try_from(next).map_err(|_| RepairError::Invalid("object numbers"))?;
            next += 1;
            Ok(number)
        };

        let info = match &trailer.info {
            Some(Object::Reference(id)) => out
                .get(&id.number())
                .filter(|o| o.data.is_none() && o.value.as_dict().is_some())
                .and_then(|o| ObjectId::new(id.number(), o.generation).ok())
                .map(Object::Reference),
            Some(value @ Object::Dictionary(_)) => Some(value.clone()),
            _ => None,
        };
        let info_number = match &info {
            Some(Object::Reference(id)) => Some(id.number()),
            _ => None,
        };

        let root = if locked {
            // The structure only: references and the page tree as they are.
            match &trailer.root {
                Some(Object::Reference(id)) => *id,
                _ => {
                    let found =
                        catalog_by_type(&out).ok_or(RepairError::Invalid("document catalog"))?;
                    ObjectId::new(found, generation_of(&out, found))
                        .map_err(|_| RepairError::Invalid("document catalog"))?
                }
            }
        } else {
            let from_trailer = match &trailer.root {
                Some(Object::Reference(id)) => dict_of(&out, id.number())
                    .filter(|dict| {
                        matches!(dict.get(b"Type".as_slice()), Some(Object::Name(n)) if n == b"Catalog")
                            || dict.contains_key(b"Pages".as_slice())
                    })
                    .map(|_| id.number()),
                _ => None,
            };
            let catalog = match from_trailer {
                Some(number) => number,
                None => {
                    self.report.catalog_from_scan = true;
                    match catalog_by_type(&out).or_else(|| catalog_by_pages(&out)) {
                        Some(number) => number,
                        None => {
                            if page_pool(&out, None, info_number).is_empty() {
                                return Err(RepairError::Invalid("document catalog"));
                            }
                            let number = allocate()?;
                            let catalog = Dictionary::from([(
                                b"Type".to_vec(),
                                Object::Name(b"Catalog".to_vec()),
                            )]);
                            out.insert(number, OutObj::synthesized(0, Object::Dictionary(catalog)));
                            number
                        }
                    }
                }
            };
            let tree = page_tree(
                &mut out,
                catalog,
                info_number,
                &unreadable,
                &mut allocate,
                &mut self.report.references_repaired,
            )?;
            self.report.page_tree_repaired = tree.changed || tree.rebuilt || tree.blank;
            if tree.rebuilt || tree.blank {
                self.report.damaged = true;
            }
            let generations: BTreeMap<u32, u16> =
                out.iter().map(|(n, o)| (*n, o.generation)).collect();
            let mut repaired = 0usize;
            for object in out.values_mut() {
                if !object.synthesized {
                    fix_references(&mut object.value, &generations, &mut repaired);
                }
            }
            self.report.references_repaired += repaired;
            ObjectId::new(catalog, generation_of(&out, catalog))
                .map_err(|_| RepairError::Invalid("document catalog"))?
        };

        let mut trailer_out = Dictionary::new();
        let size = out.keys().next_back().map_or(1, |&n| u64::from(n) + 1);
        trailer_out.insert(
            b"Size".to_vec(),
            Object::Integer(i64::try_from(size).unwrap_or(i64::MAX)),
        );
        trailer_out.insert(b"Root".to_vec(), Object::Reference(root));
        if let Some(info) = info {
            trailer_out.insert(b"Info".to_vec(), info);
        }
        if let Some(id) = &crypt.id {
            trailer_out.insert(b"ID".to_vec(), id.clone());
        }
        match &crypt.entry {
            Some(Object::Reference(id)) => {
                if let Some(object) = out.get(&id.number()) {
                    if let Ok(id) = ObjectId::new(id.number(), object.generation) {
                        trailer_out.insert(b"Encrypt".to_vec(), Object::Reference(id));
                    }
                }
            }
            Some(value) => {
                trailer_out.insert(b"Encrypt".to_vec(), value.clone());
            }
            None => {}
        }

        self.report.objects_recovered = out.values().filter(|o| !o.synthesized).count();
        self.report.damaged = self.report.damaged
            || self.header.is_some_and(|h| h > 0)
            || !self.report.xref_intact
            || self.report.objects_unreadable > 0
            || self.report.catalog_from_scan;

        let bytes = self.write(&out, &trailer_out, &crypt)?;
        // The postcondition: the output opens strictly (or asks for its
        // password), each value within the caller's per-value limits.
        let check = DocumentLimits {
            max_input_bytes: usize::MAX,
            max_xref_entries: usize::MAX,
            max_revisions: usize::MAX,
            max_metadata_decoded_bytes: usize::MAX,
            max_object_streams: usize::MAX,
            max_compressed_members: usize::MAX,
            max_objects: usize::MAX,
            max_decoded_bytes: usize::MAX,
            max_syntax_bytes: usize::MAX,
            parse: self.limits.parse,
        };
        let options = OpenOptions {
            external_streams: true,
            ..OpenOptions::default()
        };
        match Document::open_with(&bytes, check, b"", options) {
            Ok(_) => {}
            Err(error) if matches!(error.kind, DocumentErrorKind::PasswordRequired) => {}
            Err(error) => return Err(RepairError::Document(error)),
        }
        Ok(RepairedPdf {
            bytes,
            report: std::mem::take(&mut self.report),
        })
    }

    /// The data range of a selected stream, and whether it was recovered.
    fn resolve_extent(
        &mut self,
        def: &Def,
        extent: &Extent,
        lengths: &BTreeMap<u32, i64>,
        crypt: &Crypt,
    ) -> Result<(Range<usize>, bool), RepairError> {
        let data = self.data;
        if let Some(reference) = extent.length_ref {
            let end = lengths
                .get(&reference.number())
                .and_then(|&n| usize::try_from(n).ok())
                .and_then(|length| extent.start.checked_add(length))
                .filter(|&end| end <= data.len());
            if let Some(end) = end {
                if endstream_at(data, end, data.len()).is_some() {
                    return Ok((extent.start..end, false));
                }
            }
        }
        let first = extent.ends.first().copied().unwrap_or(extent.start);
        if extent.ends.len() <= 1 {
            return Ok((extent.start..first, extent.recovered));
        }
        let encrypted = crypt.dict.is_some();
        // Without object streams the output must not depend on the password
        // (section 4.12): the key is then not used for the check.
        let keyed = encrypted && self.report.password_dependent;
        if encrypted && (crypt.security.is_none() || !keyed) {
            // Without the key, AES data is still a whole number of blocks.
            let end = extent
                .ends
                .iter()
                .copied()
                .find(|&end| crypt.aes && (end - extent.start) % 16 == 0)
                .unwrap_or(first);
            return Ok((extent.start..end, true));
        }
        let Some(specs) = def.value.as_dict().and_then(filter_specs) else {
            return Ok((extent.start..first, true));
        };
        if specs.is_empty() {
            return Ok((extent.start..first, true));
        }
        let metadata = type_is(&def.value, b"Metadata");
        let security = crypt.security.as_ref().filter(|_| keyed);
        let id = def.id;
        let prepare = |raw: &'a [u8]| -> Cow<'a, [u8]> {
            match security {
                Some(security) => {
                    Cow::Owned(security.decrypt(security.stream_method(metadata), id, raw))
                }
                None => Cow::Borrowed(raw),
            }
        };
        let end = self
            .decode_ends(extent.start, &extent.ends, &specs, &prepare)?
            .map_or(first, |(end, _)| end);
        Ok((extent.start..end, true))
    }

    /// The output file: header, objects in number order, one classic xref
    /// table, the trailer.
    fn write(
        &self,
        out: &BTreeMap<u32, OutObj>,
        trailer: &Dictionary,
        crypt: &Crypt,
    ) -> Result<Vec<u8>, RepairError> {
        let data = self.data;
        let mut writer = Out::new(self.limits.max_output_bytes);
        let version = self
            .header
            .and_then(|_| data.get(5..8))
            .filter(|v| {
                matches!(
                    *v,
                    b"1.0" | b"1.1" | b"1.2" | b"1.3" | b"1.4" | b"1.5" | b"1.6" | b"1.7" | b"2.0"
                )
            })
            .unwrap_or(b"1.7");
        writer.raw(b"%PDF-")?;
        writer.raw(version)?;
        writer.raw(b"\n%\xE2\xE3\xCF\xD3\n")?;
        let mut cipher = crypt
            .security
            .as_ref()
            .map(|security| OutputCipher::new(Cow::Borrowed(security), Drbg::new(MEMBER_SEED)));
        let depth = self.limits.parse.max_depth;
        let mut offsets: BTreeMap<u32, (usize, u16)> = BTreeMap::new();
        for (&number, object) in out {
            offsets.insert(number, (writer.len(), object.generation));
            writer.raw(format!("{number} {} obj\n", object.generation).as_bytes())?;
            // Members were decrypted with their container; the reader decrypts
            // every top-level string with its own object's key.
            let encrypt = object.member
                && crypt.dict.is_some()
                && crypt.reference.is_none_or(|id| id.number() != number);
            let value: Cow<Object> = match (&mut cipher, encrypt) {
                (Some(cipher), true) => {
                    let failed = || RepairError::Invalid("object-stream member strings");
                    let id = ObjectId::new(number, object.generation).map_err(|_| failed())?;
                    Cow::Owned(
                        cipher
                            .strings(&object.value, id, depth)
                            .map_err(|_| failed())?,
                    )
                }
                _ => Cow::Borrowed(&object.value),
            };
            match &object.data {
                Some(range) => {
                    let Some(bytes) = data.get(range.clone()) else {
                        return Err(RepairError::Invalid("stream data"));
                    };
                    let mut dict = value.as_dict().cloned().unwrap_or_default();
                    dict.insert(
                        b"Length".to_vec(),
                        Object::Integer(i64::try_from(bytes.len()).unwrap_or(i64::MAX)),
                    );
                    writer.value(&Object::Dictionary(dict), 0)?;
                    writer.raw(b"\nstream\n")?;
                    writer.raw(bytes)?;
                    writer.raw(b"\nendstream\nendobj\n")?;
                }
                None => {
                    writer.value(&value, 0)?;
                    writer.raw(b"\nendobj\n")?;
                }
            }
        }
        let xref = writer.len();
        writer.raw(b"xref\n")?;
        let mut groups: Vec<(u32, u32)> = vec![(0, 0)];
        for &number in offsets.keys() {
            match groups.last_mut() {
                Some(group) if number <= group.1.saturating_add(XREF_GAP) => group.1 = number,
                _ => groups.push((number, number)),
            }
        }
        for (first, last) in groups {
            let count = u64::from(last) - u64::from(first) + 1;
            writer.raw(format!("{first} {count}\n").as_bytes())?;
            for number in first..=last {
                let row = match (number, offsets.get(&number)) {
                    (0, _) => "0000000000 65535 f \n".to_string(),
                    (_, Some((offset, generation))) => {
                        format!("{offset:010} {generation:05} n \n")
                    }
                    (_, None) => "0000000000 00000 f \n".to_string(),
                };
                writer.raw(row.as_bytes())?;
            }
        }
        writer.raw(b"trailer\n")?;
        writer.value(&Object::Dictionary(trailer.clone()), 0)?;
        writer.raw(format!("\nstartxref\n{xref}\n%%EOF\n").as_bytes())?;
        Ok(writer.finish())
    }
}

// ------------------------------------------------------------------ helpers of the extent rules

/// Whether a definition whose header is at `pos` begins at `offset`, after
/// the white-space slack an xref offset may have (addendum A7: a hint
/// stream is the stream /H names, not one that merely contains the offset).
fn begins_at(data: &[u8], pos: usize, offset: usize) -> bool {
    offset < data.len() && skip_ws(data, offset, offset.saturating_add(OFFSET_SLACK + 1)) == pos
}

/// Where stream data begins after the `stream` keyword's end: the best
/// guess, whether the EOL was standard (CRLF or LF, §7.3.8.1), and another
/// start to try with a declared /Length.
fn data_start(data: &[u8], after: usize) -> (usize, bool, Option<usize>) {
    match (data.get(after), data.get(after + 1)) {
        (Some(b'\r'), Some(b'\n')) => (after + 2, true, None),
        (Some(b'\n'), _) => (after + 1, true, None),
        (Some(b'\r'), _) => (after + 1, false, None),
        (Some(b' ' | b'\t'), _) => {
            let mut at = after;
            while matches!(data.get(at), Some(b' ' | b'\t')) {
                at += 1;
            }
            let eol = match (data.get(at), data.get(at + 1)) {
                (Some(b'\r'), Some(b'\n')) => at + 2,
                (Some(b'\n' | b'\r'), _) => at + 1,
                _ => at,
            };
            if eol != at {
                (eol, false, Some(after + 1))
            } else {
                (after + 1, false, (at != after + 1).then_some(at))
            }
        }
        _ => (after.min(data.len()), false, None),
    }
}

/// After optional white space at `at`, the end of an `endstream` keyword.
fn endstream_at(data: &[u8], at: usize, limit: usize) -> Option<usize> {
    let at = skip_ws(data, at, limit);
    starts_at(data, at, limit, b"endstream").then_some(at + 9)
}

/// Past `endobj` when it follows `at` (after white space and comments).
fn close_object(data: &[u8], at: usize, limit: usize) -> usize {
    let next = skip_trivia(data, at, limit);
    if keyword_at(data, next, b"endobj") && next + 6 <= limit {
        next + 6
    } else {
        at
    }
}

/// Where data that runs up to an `endstream` at `found` may end: without
/// the EOL before the keyword first (§7.3.8.1), then with its bytes kept.
fn eol_ends(data: &[u8], start: usize, found: usize) -> Vec<usize> {
    if found >= start + 2 && data.get(found - 2..found) == Some(b"\r\n".as_slice()) {
        vec![found - 2, found - 1, found]
    } else if found > start && matches!(data.get(found - 1), Some(b'\n' | b'\r')) {
        vec![found - 1, found]
    } else {
        vec![found]
    }
}

/// The header an xref offset names: at the offset, or after white space
/// that runs straight up to it. Its position, generation and value start.
fn locate(data: &[u8], offset: usize, number: u32) -> Option<(usize, u16, usize)> {
    if offset >= data.len() {
        return None;
    }
    let at = skip_ws(data, offset, offset.saturating_add(OFFSET_SLACK + 1));
    let (found, generation, value) = read_header(data, at)?;
    (found == number).then_some((at, generation, value))
}

fn size_of(dict: &Dictionary) -> u64 {
    match dict.get(b"Size".as_slice()) {
        Some(Object::Integer(size)) => u64::try_from(*size).unwrap_or(0),
        _ => 0,
    }
}

/// A definition the output never holds: xref streams, containers and the
/// linearization dictionary.
fn structural_def(def: &Def) -> bool {
    (def.stream.is_some() && (type_is(&def.value, b"XRef") || is_container(&def.value)))
        || is_linearization(&def.value)
}

/// The encryption the output keeps: /Encrypt as written, its dictionary
/// (indirect entries resolved for authentication), /ID made direct, and
/// the handler of the first password that authenticates.
fn setup_encryption<'o>(
    encrypt: Option<&Object>,
    id: Option<&Object>,
    lookup: &dyn Fn(u32) -> Option<(ObjectId, &'o Object)>,
    passwords: &[&[u8]],
) -> Result<Crypt, RepairError> {
    let id = match id {
        Some(value @ Object::Array(_)) => Some(value.clone()),
        Some(Object::Reference(reference)) => lookup(reference.number())
            .map(|(_, value)| value)
            .filter(|value| matches!(value, Object::Array(_)))
            .cloned(),
        _ => None,
    };
    let mut crypt = Crypt {
        id,
        ..Crypt::default()
    };
    let dict = match encrypt {
        None | Some(Object::Null) => None,
        Some(Object::Dictionary(dict)) => {
            crypt.entry = Some(Object::Dictionary(dict.clone()));
            Some(dict.clone())
        }
        Some(Object::Reference(reference)) => match lookup(reference.number()) {
            Some((actual, Object::Dictionary(dict))) => {
                crypt.entry = Some(Object::Reference(actual));
                crypt.reference = Some(actual);
                Some(dict.clone())
            }
            Some((actual, _)) => {
                crypt.entry = Some(Object::Reference(actual));
                None
            }
            None => None,
        },
        Some(other) => {
            crypt.entry = Some(other.clone());
            None
        }
    };
    let Some(dict) = dict else {
        return Ok(crypt);
    };
    let resolve = |reference: ObjectId| lookup(reference.number()).map(|(_, value)| value);
    let resolved = resolve_encryption(&dict, &resolve);
    let info = EncryptionInfo::from_dictionary(&resolved);
    let first_id: Option<Vec<u8>> = match &crypt.id {
        Some(Object::Array(items)) => match items.first() {
            Some(Object::String(first)) => Some(first.clone()),
            _ => None,
        },
        _ => None,
    };
    if (2..=4).contains(&info.r) && first_id.is_none() {
        return Err(RepairError::Invalid(
            "encryption without the file identifier",
        ));
    }
    crypt.aes = matches!(info.method, Some(Method::AesV2 | Method::AesV3));
    let first = first_id.unwrap_or_default();
    crypt.security = passwords
        .iter()
        .find_map(|password| Security::authenticate(&resolved, &first, password).ok());
    crypt.dict = Some(dict);
    Ok(crypt)
}

// ------------------------------------------------------------------ catalog and page tree

fn resolve<'o>(value: &'o Object, out: &'o BTreeMap<u32, OutObj>) -> &'o Object {
    static NULL: Object = Object::Null;
    let mut value = value;
    for _ in 0..8 {
        match value {
            Object::Reference(id) => match out.get(&id.number()) {
                Some(object) if object.data.is_none() => value = &object.value,
                _ => return &NULL,
            },
            _ => return value,
        }
    }
    &NULL
}

fn get<'o>(dict: &'o Dictionary, key: &[u8], out: &'o BTreeMap<u32, OutObj>) -> &'o Object {
    static NULL: Object = Object::Null;
    resolve(dict.get(key).unwrap_or(&NULL), out)
}

fn dict_of(out: &BTreeMap<u32, OutObj>, number: u32) -> Option<&Dictionary> {
    out.get(&number)
        .filter(|o| o.data.is_none())
        .and_then(|o| o.value.as_dict())
}

fn generation_of(out: &BTreeMap<u32, OutObj>, number: u32) -> u16 {
    out.get(&number).map_or(0, |o| o.generation)
}

/// The dictionary latest in file order typed /Catalog.
fn catalog_by_type(out: &BTreeMap<u32, OutObj>) -> Option<u32> {
    out.iter()
        .filter(|(_, o)| o.data.is_none() && type_is(&o.value, b"Catalog"))
        .max_by_key(|(_, o)| o.order)
        .map(|(n, _)| *n)
}

/// The dictionary latest in file order whose /Pages names a page-tree node.
fn catalog_by_pages(out: &BTreeMap<u32, OutObj>) -> Option<u32> {
    out.iter()
        .filter(|(_, o)| {
            o.data.is_none()
                && o.value.as_dict().is_some_and(|dict| {
                    get(dict, b"Pages", out)
                        .as_dict()
                        .is_some_and(|node| classify(node, out) == Kid::Node)
                })
        })
        .max_by_key(|(_, o)| o.order)
        .map(|(n, _)| *n)
}

/// Every /Type /Page dictionary but the catalog's and /Info's, in file order.
fn page_pool(out: &BTreeMap<u32, OutObj>, catalog: Option<u32>, info: Option<u32>) -> Vec<u32> {
    let mut pool: Vec<(u32, (usize, usize))> = out
        .iter()
        .filter(|(n, o)| {
            o.data.is_none()
                && Some(**n) != catalog
                && Some(**n) != info
                && type_is(&o.value, b"Page")
        })
        .map(|(n, o)| (*n, o.order))
        .collect();
    pool.sort_by_key(|(_, order)| *order);
    pool.into_iter().map(|(n, _)| n).collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kid {
    Node,
    Page,
    Omit,
}

/// How the page walk reads a kid (section 4.10 rule 1).
fn classify(dict: &Dictionary, out: &BTreeMap<u32, OutObj>) -> Kid {
    let kids = match get(dict, b"Kids", out) {
        Object::Null => None,
        Object::Array(items) => Some(items.len()),
        _ => Some(0),
    };
    let contents = matches!(
        dict.get(b"Contents".as_slice()),
        Some(Object::Reference(_) | Object::Array(_))
    );
    let media = !get(dict, b"MediaBox", out).is_null();
    match get(dict, b"Type", out) {
        Object::Null => {
            if untyped_kid_is_node(kids, contents, media) {
                Kid::Node
            } else {
                Kid::Page
            }
        }
        Object::Name(name) if name == b"Pages" => Kid::Node,
        Object::Name(name) if name == b"Page" => Kid::Page,
        Object::Name(name) if ISO_TYPES.contains(&name.as_slice()) => {
            if contents || (media && kids.unwrap_or(0) == 0) {
                Kid::Page
            } else {
                Kid::Omit
            }
        }
        _ => Kid::Page,
    }
}

fn has_media_box(dict: &Dictionary, out: &BTreeMap<u32, OutObj>) -> bool {
    matches!(get(dict, b"MediaBox", out), Object::Array(_))
}

fn letter() -> Object {
    Object::Array(LETTER.iter().map(|&n| Object::Integer(n)).collect())
}

fn reference(number: u32, generation: u16) -> Result<Object, RepairError> {
    ObjectId::new(number, generation)
        .map(Object::Reference)
        .map_err(|_| RepairError::Invalid("object number"))
}

/// Sets `key` to `value` in object `number`'s dictionary; whether it changed.
fn put(out: &mut BTreeMap<u32, OutObj>, number: u32, key: &[u8], value: Object) -> bool {
    let Some(dict) = out.get_mut(&number).and_then(|o| o.value.as_dict_mut()) else {
        return false;
    };
    if dict.get(key) == Some(&value) {
        return false;
    }
    dict.insert(key.to_vec(), value);
    true
}

/// A node being walked: its kids, those kept, and the pages below it.
struct Frame {
    node: u32,
    kids: Vec<Object>,
    at: usize,
    media: bool,
    kept: Vec<Object>,
    changed: bool,
    count: usize,
}

fn open_frame(out: &BTreeMap<u32, OutObj>, node: u32, media: bool) -> Frame {
    let dict = dict_of(out, node);
    let kids = dict
        .and_then(|dict| get(dict, b"Kids", out).as_array())
        .map(<[Object]>::to_vec)
        .unwrap_or_default();
    // A /Kids array object is written into the node when the tree changes.
    let indirect = matches!(
        dict.and_then(|dict| dict.get(b"Kids".as_slice())),
        Some(Object::Reference(_))
    );
    let has_box = dict.is_some_and(|dict| has_media_box(dict, out));
    Frame {
        node,
        kids,
        at: 0,
        media: media || has_box,
        kept: Vec::new(),
        changed: indirect,
        count: 0,
    }
}

/// The catalog's page tree made to satisfy the strict page walk (section
/// 4.10): kept when usable, rebuilt over every page otherwise.
fn page_tree(
    out: &mut BTreeMap<u32, OutObj>,
    catalog: u32,
    info: Option<u32>,
    unreadable: &BTreeMap<u32, u16>,
    allocate: &mut dyn FnMut() -> Result<u32, RepairError>,
    repaired: &mut usize,
) -> Result<TreeOutcome, RepairError> {
    let mut outcome = TreeOutcome::default();
    let pages_value = dict_of(out, catalog).and_then(|d| d.get(b"Pages".as_slice()).cloned());
    let root = match pages_value {
        Some(Object::Reference(id)) => {
            let number = id.number();
            let usable = number != catalog
                && Some(number) != info
                && dict_of(out, number).is_some_and(|dict| classify(dict, out) == Kid::Node);
            usable.then_some(number)
        }
        Some(Object::Dictionary(dict)) if classify(&dict, out) == Kid::Node => {
            let number = allocate()?;
            out.insert(number, OutObj::synthesized(0, Object::Dictionary(dict)));
            put(out, catalog, b"Pages", reference(number, 0)?);
            outcome.changed = true;
            Some(number)
        }
        _ => None,
    };
    let mut total = 0usize;
    if let Some(root) = root {
        let mut visited: BTreeSet<u32> = BTreeSet::from([root]);
        if let Some(dict) = out.get_mut(&root).and_then(|o| o.value.as_dict_mut()) {
            if dict.remove(b"Parent".as_slice()).is_some() {
                outcome.changed = true;
            }
            if dict.get(b"Type".as_slice()).is_none_or(Object::is_null) {
                dict.insert(b"Type".to_vec(), Object::Name(b"Pages".to_vec()));
                outcome.changed = true;
            }
        }
        let mut stack = vec![open_frame(out, root, false)];
        while let Some(frame) = stack.last_mut() {
            if frame.at >= frame.kids.len() {
                let Some(frame) = stack.pop() else {
                    break;
                };
                if frame.changed {
                    put(out, frame.node, b"Kids", Object::Array(frame.kept));
                    outcome.changed = true;
                }
                let count = Object::Integer(i64::try_from(frame.count).unwrap_or(i64::MAX));
                if put(out, frame.node, b"Count", count) {
                    outcome.changed = true;
                }
                match stack.last_mut() {
                    Some(parent) => parent.count += frame.count,
                    None => total = frame.count,
                }
                continue;
            }
            let kid = frame.kids[frame.at].clone();
            frame.at += 1;
            let node = frame.node;
            let media = frame.media;
            let target = match &kid {
                Object::Reference(id) => {
                    let number = id.number();
                    if number == catalog || Some(number) == info || !visited.insert(number) {
                        None
                    } else if let Some(object) = out.get(&number) {
                        if object.generation != id.generation() {
                            *repaired += 1;
                        }
                        Some(number)
                    } else if let Some(&generation) = unreadable.get(&number) {
                        // A listed page whose value was lost: a blank page.
                        let page =
                            Dictionary::from([(b"Type".to_vec(), Object::Name(b"Page".to_vec()))]);
                        out.insert(
                            number,
                            OutObj::synthesized(generation, Object::Dictionary(page)),
                        );
                        outcome.blank = true;
                        Some(number)
                    } else {
                        *repaired += 1;
                        None
                    }
                }
                Object::Dictionary(dict) => {
                    let number = allocate()?;
                    out.insert(
                        number,
                        OutObj::synthesized(0, Object::Dictionary(dict.clone())),
                    );
                    visited.insert(number);
                    Some(number)
                }
                _ => None,
            };
            let kind =
                target.and_then(|number| dict_of(out, number).map(|dict| classify(dict, out)));
            let (Some(number), Some(kind)) = (target, kind) else {
                if let Some(frame) = stack.last_mut() {
                    frame.changed = true;
                }
                continue;
            };
            let parent = reference(node, generation_of(out, node))?;
            let kid_ref = reference(number, generation_of(out, number))?;
            if kid != kid_ref || kind == Kid::Omit {
                if let Some(frame) = stack.last_mut() {
                    frame.changed = true;
                }
            }
            match kind {
                Kid::Omit => {}
                Kid::Page => {
                    let mut changed = put(out, number, b"Parent", parent);
                    let typed = dict_of(out, number).is_some_and(|dict| {
                        matches!(dict.get(b"Type".as_slice()), Some(Object::Name(name))
                            if name == b"Page" || ISO_TYPES.contains(&name.as_slice()))
                    });
                    if !typed {
                        changed |= put(out, number, b"Type", Object::Name(b"Page".to_vec()));
                    }
                    let boxed = dict_of(out, number).is_some_and(|dict| has_media_box(dict, out));
                    if !media && !boxed {
                        changed |= put(out, number, b"MediaBox", letter());
                    }
                    outcome.changed |= changed;
                    if let Some(frame) = stack.last_mut() {
                        frame.kept.push(kid_ref);
                        frame.count += 1;
                    }
                }
                Kid::Node => {
                    let mut changed = put(out, number, b"Parent", parent);
                    let untyped = dict_of(out, number).is_some_and(|dict| {
                        dict.get(b"Type".as_slice()).is_none_or(Object::is_null)
                    });
                    if untyped {
                        changed |= put(out, number, b"Type", Object::Name(b"Pages".to_vec()));
                    }
                    outcome.changed |= changed;
                    if let Some(frame) = stack.last_mut() {
                        frame.kept.push(kid_ref);
                    }
                    let frame = open_frame(out, number, media);
                    stack.push(frame);
                }
            }
        }
    }
    if total > 0 {
        return Ok(outcome);
    }
    let pool = page_pool(out, Some(catalog), info);
    if pool.is_empty() {
        return Ok(outcome);
    }
    // Rebuild: a new root over every page, in file order (rule 4).
    let root = allocate()?;
    let root_ref = reference(root, 0)?;
    let mut kids = Vec::with_capacity(pool.len());
    for &number in &pool {
        let inherited = inherited_attributes(out, number);
        for (key, value) in inherited {
            put(out, number, &key, value);
        }
        put(out, number, b"Parent", root_ref.clone());
        let boxed = dict_of(out, number).is_some_and(|dict| has_media_box(dict, out));
        if !boxed {
            put(out, number, b"MediaBox", letter());
        }
        kids.push(reference(number, generation_of(out, number))?);
    }
    let count = Object::Integer(i64::try_from(kids.len()).unwrap_or(i64::MAX));
    let node = Dictionary::from([
        (b"Type".to_vec(), Object::Name(b"Pages".to_vec())),
        (b"Kids".to_vec(), Object::Array(kids)),
        (b"Count".to_vec(), count),
    ]);
    out.insert(root, OutObj::synthesized(0, Object::Dictionary(node)));
    put(out, catalog, b"Pages", root_ref);
    outcome.rebuilt = true;
    Ok(outcome)
}

/// The attributes a page lacks and inherits through its own /Parent chain,
/// copied before the page moves under a new root.
fn inherited_attributes(out: &BTreeMap<u32, OutObj>, number: u32) -> Vec<(Vec<u8>, Object)> {
    let Some(page) = dict_of(out, number) else {
        return Vec::new();
    };
    let mut found: Vec<(Vec<u8>, Object)> = Vec::new();
    let mut parent = page.get(b"Parent".as_slice());
    let mut seen = BTreeSet::from([number]);
    for _ in 0..MAX_INHERIT {
        let Some(Object::Reference(id)) = parent else {
            break;
        };
        if !seen.insert(id.number()) {
            break;
        }
        let Some(node) = dict_of(out, id.number()) else {
            break;
        };
        for key in INHERITED {
            let known = page.contains_key(key) || found.iter().any(|(k, _)| k.as_slice() == key);
            if !known {
                if let Some(value) = node.get(key) {
                    found.push((key.to_vec(), value.clone()));
                }
            }
        }
        parent = node.get(b"Parent".as_slice());
    }
    found
}

// ------------------------------------------------------------------ references

/// References to numbers absent from the output become null (removed from
/// lists of objects); references with another generation name the object.
fn fix_references(value: &mut Object, generations: &BTreeMap<u32, u16>, count: &mut usize) {
    match value {
        Object::Reference(id) => match generations.get(&id.number()) {
            Some(&generation) if generation == id.generation() => {}
            Some(&generation) => {
                if let Ok(fixed) = ObjectId::new(id.number(), generation) {
                    *id = fixed;
                    *count += 1;
                }
            }
            None => {
                *value = Object::Null;
                *count += 1;
            }
        },
        Object::Array(items) => {
            for item in items {
                fix_references(item, generations, count);
            }
        }
        Object::Dictionary(dict) => {
            for (key, item) in dict.iter_mut() {
                let list = LIST_KEYS.contains(&key.as_slice());
                match item {
                    Object::Array(items) if list => {
                        fix_list(items, generations, count, key.as_slice() == b"Order");
                    }
                    other => fix_references(other, generations, count),
                }
            }
        }
        _ => {}
    }
}

fn fix_list(
    items: &mut Vec<Object>,
    generations: &BTreeMap<u32, u16>,
    count: &mut usize,
    nested: bool,
) {
    items.retain(|item| match item {
        Object::Reference(id) if !generations.contains_key(&id.number()) => {
            *count += 1;
            false
        }
        _ => true,
    });
    for item in items {
        match item {
            Object::Array(inner) if nested => fix_list(inner, generations, count, true),
            other => fix_references(other, generations, count),
        }
    }
}

// ------------------------------------------------------------------ serializer

/// Output bytes within `max_output_bytes`. Values keep their dictionaries'
/// key order; strings take the shorter of the literal and hex forms.
struct Out {
    buffer: Vec<u8>,
    limit: usize,
    /// The last byte written ends a token a regular byte would extend.
    regular: bool,
}

impl Out {
    fn new(limit: usize) -> Self {
        Self {
            buffer: Vec::new(),
            limit,
            regular: false,
        }
    }

    fn len(&self) -> usize {
        self.buffer.len()
    }

    fn finish(self) -> Vec<u8> {
        self.buffer
    }

    fn put(&mut self, bytes: &[u8]) -> Result<(), SerializeError> {
        let total = self
            .buffer
            .len()
            .checked_add(bytes.len())
            .ok_or(SerializeError::OutputLimit)?;
        if total > self.limit {
            return Err(SerializeError::OutputLimit);
        }
        self.buffer
            .try_reserve(bytes.len())
            .map_err(|_| SerializeError::AllocationFailed)?;
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    fn raw(&mut self, bytes: &[u8]) -> Result<(), SerializeError> {
        self.put(bytes)?;
        self.regular = false;
        Ok(())
    }

    /// A token that begins with a regular byte, separated from the last.
    fn word(&mut self, bytes: &[u8]) -> Result<(), SerializeError> {
        if self.regular {
            self.put(b" ")?;
        }
        self.put(bytes)?;
        self.regular = true;
        Ok(())
    }

    fn value(&mut self, value: &Object, depth: usize) -> Result<(), SerializeError> {
        if depth > 512 {
            return Err(SerializeError::DepthLimit);
        }
        match value {
            Object::Null => self.word(b"null"),
            Object::Boolean(true) => self.word(b"true"),
            Object::Boolean(false) => self.word(b"false"),
            Object::Integer(n) => self.word(n.to_string().as_bytes()),
            Object::Real(n) => {
                if !n.is_finite() {
                    return Err(SerializeError::NonfiniteReal);
                }
                // The shortest digits that read back exactly, positional;
                // a point keeps it a real.
                let mut text = format!("{n}");
                if !text.contains('.') {
                    text.push_str(".0");
                }
                self.word(text.as_bytes())
            }
            Object::Reference(id) => {
                self.word(format!("{} {} R", id.number(), id.generation()).as_bytes())
            }
            Object::Name(name) => self.name(name),
            Object::String(bytes) => self.string(bytes),
            Object::Array(items) => {
                self.raw(b"[")?;
                for item in items {
                    self.value(item, depth + 1)?;
                }
                self.raw(b"]")
            }
            Object::Dictionary(dict) => {
                self.raw(b"<<")?;
                for (key, item) in dict.iter() {
                    self.name(key)?;
                    self.value(item, depth + 1)?;
                }
                self.raw(b">>")
            }
        }
    }

    /// A name (§7.3.5): bytes outside `!`..`~`, delimiters and `#` as
    /// `#` and two hex digits.
    fn name(&mut self, name: &[u8]) -> Result<(), SerializeError> {
        if name.contains(&0) {
            return Err(SerializeError::NullInName);
        }
        let mut text = Vec::with_capacity(name.len() + 1);
        text.push(b'/');
        for &byte in name {
            if (0x21..=0x7E).contains(&byte) && !is_delimiter(byte) && byte != b'#' {
                text.push(byte);
            } else {
                text.extend_from_slice(format!("#{byte:02X}").as_bytes());
            }
        }
        self.put(&text)?;
        self.regular = true;
        Ok(())
    }

    /// A string (§7.3.4): literal, with balanced parentheses left as they
    /// are, unless the hex form is shorter.
    fn string(&mut self, bytes: &[u8]) -> Result<(), SerializeError> {
        let mut escape = vec![false; bytes.len()];
        let mut open: Vec<usize> = Vec::new();
        for (at, &byte) in bytes.iter().enumerate() {
            match byte {
                b'(' => open.push(at),
                // A `)` without an open `(` before it is escaped.
                b')' if open.pop().is_none() => escape[at] = true,
                _ => {}
            }
        }
        for at in open {
            escape[at] = true;
        }
        let literal = bytes
            .iter()
            .zip(&escape)
            .fold(2usize, |n, (&byte, &escaped)| {
                let cost = if escaped || matches!(byte, b'\\' | b'\r') {
                    2
                } else {
                    1
                };
                n.saturating_add(cost)
            });
        let hex = bytes.len().saturating_mul(2).saturating_add(2);
        let mut text = Vec::new();
        if literal <= hex {
            text.try_reserve(literal)
                .map_err(|_| SerializeError::AllocationFailed)?;
            text.push(b'(');
            for (&byte, &escaped) in bytes.iter().zip(&escape) {
                match byte {
                    b'\\' => text.extend_from_slice(b"\\\\"),
                    b'\r' => text.extend_from_slice(b"\\r"),
                    _ if escaped => text.extend_from_slice(&[b'\\', byte]),
                    _ => text.push(byte),
                }
            }
            text.push(b')');
        } else {
            const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
            text.try_reserve(hex)
                .map_err(|_| SerializeError::AllocationFailed)?;
            text.push(b'<');
            for &byte in bytes {
                text.push(DIGITS[usize::from(byte >> 4)]);
                text.push(DIGITS[usize::from(byte & 15)]);
            }
            text.push(b'>');
        }
        self.put(&text)?;
        self.regular = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentErrorKind;

    #[test]
    fn untyped_kids_follow_the_rule() {
        assert!(!untyped_kid_is_node(None, false, false));
        assert!(untyped_kid_is_node(Some(0), false, false));
        assert!(!untyped_kid_is_node(Some(0), true, false));
        assert!(!untyped_kid_is_node(Some(0), false, true));
        assert!(untyped_kid_is_node(Some(2), true, true));
    }

    #[test]
    fn strings_keep_balanced_parentheses() {
        let mut out = Out::new(1 << 20);
        out.value(&Object::String(b")a(b)c(".to_vec()), 0).unwrap();
        assert_eq!(out.finish(), b"(\\)a(b)c\\()".to_vec());
        let mut out = Out::new(1 << 20);
        out.value(&Object::String(vec![b'\\'; 3]), 0).unwrap();
        assert_eq!(out.finish(), b"(\\\\\\\\\\\\)".to_vec());
        let mut out = Out::new(1 << 20);
        out.value(&Object::String(vec![b'\r'; 3]), 0).unwrap();
        assert_eq!(out.finish(), b"(\\r\\r\\r)".to_vec());
    }

    #[test]
    fn tokens_are_separated_only_where_they_would_merge() {
        let mut out = Out::new(1 << 20);
        let value = Object::Array(vec![
            Object::Integer(1),
            Object::Real(2.0),
            Object::Name(b"A".to_vec()),
            Object::Name(Vec::new()),
            Object::Boolean(true),
            Object::String(b"x".to_vec()),
            Object::Null,
        ]);
        out.value(&value, 0).unwrap();
        assert_eq!(out.finish(), b"[1 2.0/A/ true(x)null]".to_vec());
    }

    #[test]
    fn hostile_bytes_never_panic() {
        let mut seed = 0x2545_F491_4F6C_DD1D_u64;
        const ALPHABET: &[u8] = b"0123456789 obj endobj stream endstream trailer<<>>[]()/%\nR xref";
        for case in 0..300usize {
            let input: Vec<u8> = (0..case * 3)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    ALPHABET[(seed % ALPHABET.len() as u64) as usize]
                })
                .collect();
            let input = [b"%PDF-1.7\n".as_slice(), &input].concat();
            let _ = rebuild(&input, RepairLimits::default());
        }
    }

    /// Addendum A1: an update whose first header follows `%%EOF` on its
    /// line is found by reconstruction; other bytes there are not objects.
    #[test]
    fn a_header_glued_to_the_eof_marker_is_found() {
        fn file(glued: &str) -> Vec<u8> {
            let mut out = b"%PDF-1.7\n".to_vec();
            let mut offsets = Vec::new();
            for body in [
                "<< /Type /Catalog /Pages 2 0 R >>",
                "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>",
            ] {
                offsets.push(out.len());
                out.extend_from_slice(
                    format!("{} 0 obj\n{body}\nendobj\n", offsets.len()).as_bytes(),
                );
            }
            let xref = out.len();
            out.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
            for offset in &offsets {
                out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
            }
            out.extend_from_slice(
                format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF{glued}")
                    .as_bytes(),
            );
            let mut update = Vec::new();
            for (number, body) in [
                (1, "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>"),
                (4, "<< /Type /Outlines /First 5 0 R /Last 5 0 R /Count 1 >>"),
                (5, "<< /Title (Chapter) /Parent 4 0 R /Dest [3 0 R /Fit] >>"),
            ] {
                update.push((number, out.len()));
                out.extend_from_slice(format!("{number} 0 obj\n{body}\nendobj\n").as_bytes());
            }
            let second = out.len();
            out.extend_from_slice(b"xref\n0 1\n0000000000 65535 f \n1 1\n");
            out.extend_from_slice(format!("{:010} 00000 n \n4 2\n", update[0].1).as_bytes());
            for (_, offset) in &update[1..] {
                out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
            }
            // The final startxref is wrong: the file is reconstructed.
            out.extend_from_slice(
                format!(
                    "trailer\n<< /Size 6 /Root 1 0 R /Prev {xref} >>\nstartxref\n{}\n%%EOF\n",
                    second + 3
                )
                .as_bytes(),
            );
            out
        }
        let outline = |bytes: &[u8]| {
            let repaired = rebuild(bytes, RepairLimits::default()).unwrap();
            assert!(repaired.report.damaged);
            let doc = Document::open(&repaired.bytes, DocumentLimits::default()).unwrap();
            let catalog = doc.resolve_id(doc.root()).as_dict().unwrap().clone();
            let outlines = doc.dict_get(&catalog, b"Outlines").as_dict().cloned();
            outlines.map(|outlines| {
                let first = doc.dict_get(&outlines, b"First").as_dict().unwrap().clone();
                (
                    doc.dict_get(&outlines, b"Count").as_i64(),
                    doc.dict_get(&first, b"Title")
                        .as_string()
                        .map(<[u8]>::to_vec),
                )
            })
        };
        assert_eq!(
            outline(&file("")),
            Some((Some(1), Some(b"Chapter".to_vec())))
        );
        // Bytes after the marker that are no header stay ignored.
        assert_eq!(
            outline(&file("junk 9 R 2 0 R)\n")),
            Some((Some(1), Some(b"Chapter".to_vec())))
        );
        let scan = raw_scan(b"%%EOFjunk 0 obj %%EOF7 0 obj %%EOF 8 0 obj");
        let headers: Vec<u32> = scan
            .0
            .iter()
            .filter_map(|cand| match cand.kind {
                CandKind::Header(number, _) => Some(number),
                CandKind::Trailer => None,
            })
            .collect();
        assert_eq!(headers, [7, 8]);
    }

    /// A classic-xref file of `objects` numbered from 1 (each a full body).
    fn classic(objects: &[Vec<u8>], trailer: &str) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} {trailer} >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    fn stream_object(dictionary: &str, data: &[u8], eol: &[u8]) -> Vec<u8> {
        [
            format!("{dictionary}\nstream\n").as_bytes(),
            data,
            eol,
            b"endstream",
        ]
        .concat()
    }

    /// Decode attempts that fail are charged (addendum A5): a large
    /// compressible stream whose /Length is wrong and whose tail is cut
    /// does not inflate once per candidate end for free.
    #[test]
    fn failed_decode_attempts_are_charged() {
        let mut data = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0u8; 4 << 20], 6);
        data.truncate(data.len() - 2);
        let file = classic(
            &[
                b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
                b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
                b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>".to_vec(),
                stream_object("<< /Filter /FlateDecode /Length 5 >>", &data, b"\r\n"),
            ],
            "/Root 1 0 R",
        );
        let small = RepairLimits {
            max_decoded_bytes: 6 << 20,
            ..RepairLimits::default()
        };
        assert!(matches!(
            rebuild(&file, small),
            Err(RepairError::LimitExceeded("object-stream bytes"))
        ));
        let repaired = rebuild(&file, RepairLimits::default()).unwrap();
        assert!(repaired.report.xref_intact && repaired.report.streams_length_recovered == 1);
    }

    fn encrypted_fixture() -> Vec<u8> {
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../fixtures/encrypted.pdf"
        ))
        .unwrap()
    }

    /// Addendum A4: /Encrypt naming an absent object never yields ciphertext
    /// read as text: the dictionary is found by type, or the file refused.
    #[test]
    fn a_dangling_encrypt_entry_never_shows_ciphertext() {
        let source = encrypted_fixture();
        let at = source
            .windows(14)
            .position(|w| w == b"/Encrypt 7 0 R")
            .unwrap();
        let mut moved = source.clone();
        moved[at + 9] = b'9';
        assert!(Document::open(&moved, DocumentLimits::default()).is_err());
        let repaired = rebuild(&moved, RepairLimits::default()).unwrap();
        assert!(!repaired.report.xref_intact && repaired.report.damaged);
        let error = Document::open(&repaired.bytes, DocumentLimits::default()).unwrap_err();
        assert!(matches!(error.kind, DocumentErrorKind::PasswordRequired));
        let doc = Document::open_with_password(
            &repaired.bytes,
            DocumentLimits::default(),
            b"pdf4all-user",
        )
        .unwrap();
        assert!(doc.security().is_some());
        // Without any encryption dictionary left, the file is refused.
        let at = moved
            .windows(17)
            .position(|w| w == b"/Filter /Standard")
            .unwrap();
        let mut gone = moved;
        gone[at + 16] = b'x';
        assert!(matches!(
            rebuild(&gone, RepairLimits::default()),
            Err(RepairError::Invalid("encryption dictionary"))
        ));
    }

    /// Section 4.12: without object streams the output does not depend on
    /// the password, even when the password's key would pick another end for
    /// an RC4 stream whose ciphertext ends in CR before the EOL.
    #[test]
    fn a_recovered_rc4_stream_ends_alike_with_any_password() {
        let source = encrypted_fixture();
        let doc = Document::open_with_password(&source, DocumentLimits::default(), b"pdf4all-user")
            .unwrap();
        let security = doc.security().unwrap().clone();
        let encrypt_id = match doc.trailer().get(b"Encrypt".as_slice()) {
            Some(Object::Reference(id)) => *id,
            other => panic!("{other:?}"),
        };
        let encrypt = crate::serialize::serialize_object(
            &doc.object(encrypt_id).unwrap().value,
            Default::default(),
        )
        .unwrap();
        let id = crate::serialize::serialize_object(
            doc.trailer().get(b"ID".as_slice()).unwrap(),
            Default::default(),
        )
        .unwrap();
        let stream_id = ObjectId::new(4, 0).unwrap();
        let method = security.stream_method(false);
        let cipher = (0..4096u32)
            .map(|pad| {
                let tag = pad.wrapping_mul(0x9E37_79B1);
                let text = format!("BT /F1 12 Tf 20 100 Td (Hidden {tag:08x}) Tj ET");
                let data = miniz_oxide::deflate::compress_to_vec_zlib(text.as_bytes(), 6);
                security.encrypt(method, stream_id, &data, [0; 16]).unwrap()
            })
            .find(|cipher| cipher.last() == Some(&b'\r'))
            .unwrap();
        let mut objects = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>".to_vec(),
            stream_object(
                &format!("<< /Filter /FlateDecode /Length {} >>", cipher.len() + 7),
                &cipher,
                // The ciphertext's own CR and this LF read as one CRLF.
                b"\n",
            ),
        ];
        objects.push(b"null".to_vec());
        objects.push(b"null".to_vec());
        objects.push(encrypt);
        let trailer = format!(
            "/Root 1 0 R /Encrypt 7 0 R /ID {}",
            String::from_utf8_lossy(&id)
        );
        let file = classic(&objects, &trailer);
        let without = rebuild_with_passwords(&file, RepairLimits::default(), &[]).unwrap();
        let with =
            rebuild_with_passwords(&file, RepairLimits::default(), &[b"pdf4all-user"]).unwrap();
        assert!(!without.report.password_dependent && without.report.xref_intact);
        assert_eq!(without.bytes, with.bytes);
    }
}
