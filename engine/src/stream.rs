// SPDX-License-Identifier: AGPL-3.0-or-later
//! Stream filters (ISO 32000-2 §7.4): ASCIIHexDecode, ASCII85Decode,
//! RunLengthDecode, LZWDecode and FlateDecode, the TIFF and PNG predictors,
//! and the chain driver with its size budgets.
//!
//! The strict entry points refuse damaged data, since a silent partial
//! decode could lose content when a stream is rewritten. The lenient entry
//! point (repair and viewing) keeps what decodes of a damaged Flate stream
//! and accepts a missing end-of-data marker in the other filters.

use std::borrow::Cow;
use std::{error::Error, fmt};

use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Flate,
    AsciiHex,
    Ascii85,
    RunLength,
    Lzw { early_change: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PredictorParams {
    pub predictor: u8,
    pub colors: usize,
    pub bits_per_component: u8,
    pub columns: usize,
}

impl Default for PredictorParams {
    /// ISO 32000-2 Table 10 defaults.
    fn default() -> Self {
        Self {
            predictor: 1,
            colors: 1,
            bits_per_component: 8,
            columns: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterSpec {
    pub filter: Filter,
    pub predictor: PredictorParams,
}

impl From<Filter> for FilterSpec {
    fn from(filter: Filter) -> Self {
        Self {
            filter,
            predictor: PredictorParams::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamLimits {
    pub max_encoded_bytes: usize,
    pub max_decoded_bytes: usize,
    pub max_filters: usize,
    pub max_total_decoded_bytes: usize,
}

impl Default for StreamLimits {
    fn default() -> Self {
        Self {
            max_encoded_bytes: 64 << 20,
            max_decoded_bytes: 64 << 20,
            max_filters: 8,
            max_total_decoded_bytes: 128 << 20,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamError {
    pub offset: usize,
    pub filter_index: Option<usize>,
    pub kind: StreamErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamErrorKind {
    EncodedInputLimit { limit: usize },
    DecodedOutputLimit { limit: usize },
    TotalOutputLimit { limit: usize },
    FilterCountLimit { limit: usize },
    MissingEndMarker,
    InvalidByte(u8),
    InvalidEndMarker,
    TrailingData,
    InvalidAscii85Group,
    Ascii85Overflow,
    TruncatedRun,
    InvalidFlate,
    TruncatedFlate,
    FlateChecksumMismatch,
    AllocationFailed,
    InvalidPredictor(&'static str),
    UnsupportedPredictor(u8),
    UnsupportedBitsPerComponent(u8),
    TruncatedPredictorRow,
    InvalidPngFilter(u8),
    InvalidLzwCode(u16),
    TruncatedLzw,
    MissingLzwClear,
    LzwDictionaryFull,
    NonZeroLzwPadding,
}

impl fmt::Display for StreamErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EncodedInputLimit { limit } => write!(f, "encoded input exceeds {limit} bytes"),
            Self::DecodedOutputLimit { limit } => write!(f, "decoded stage exceeds {limit} bytes"),
            Self::TotalOutputLimit { limit } => {
                write!(f, "total decoded output exceeds {limit} bytes")
            }
            Self::FilterCountLimit { limit } => write!(f, "filter chain exceeds {limit} stages"),
            Self::MissingEndMarker => f.write_str("missing end-of-data marker"),
            Self::InvalidByte(byte) => write!(f, "invalid encoded byte 0x{byte:02x}"),
            Self::InvalidEndMarker => f.write_str("invalid end-of-data marker"),
            Self::TrailingData => f.write_str("unexpected trailing data"),
            Self::InvalidAscii85Group => f.write_str("invalid ASCII85 group"),
            Self::Ascii85Overflow => f.write_str("ASCII85 group exceeds 32 bits"),
            Self::TruncatedRun => f.write_str("truncated run-length packet"),
            Self::InvalidFlate => f.write_str("invalid zlib stream"),
            Self::TruncatedFlate => f.write_str("truncated zlib stream"),
            Self::FlateChecksumMismatch => f.write_str("zlib checksum mismatch"),
            Self::AllocationFailed => f.write_str("stream allocation failed"),
            Self::InvalidPredictor(reason) => write!(f, "invalid predictor parameters: {reason}"),
            Self::UnsupportedPredictor(predictor) => {
                write!(f, "unsupported predictor {predictor}")
            }
            Self::UnsupportedBitsPerComponent(bits) => {
                write!(f, "unsupported predictor bit depth {bits}")
            }
            Self::TruncatedPredictorRow => f.write_str("truncated predictor row"),
            Self::InvalidPngFilter(tag) => write!(f, "invalid PNG row filter {tag}"),
            Self::InvalidLzwCode(code) => write!(f, "invalid LZW code {code}"),
            Self::TruncatedLzw => f.write_str("truncated LZW stream or missing EOD code"),
            Self::MissingLzwClear => f.write_str("LZW stream must begin with a clear code"),
            Self::LzwDictionaryFull => {
                f.write_str("full LZW dictionary requires a clear or EOD code")
            }
            Self::NonZeroLzwPadding => f.write_str("nonzero LZW padding bits after EOD"),
        }
    }
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.filter_index {
            Some(index) => write!(
                f,
                "PDF stream filter {index} at byte {}: {}",
                self.offset, self.kind
            ),
            None => write!(f, "PDF stream at byte {}: {}", self.offset, self.kind),
        }
    }
}

impl Error for StreamError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeUsage {
    pub encoded_bytes: usize,
    pub total_decoded_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedStream {
    pub bytes: Vec<u8>,
    pub usage: DecodeUsage,
}

pub fn decode_filters(
    data: &[u8],
    filters: &[Filter],
    limits: StreamLimits,
) -> Result<Vec<u8>, StreamError> {
    decode_filters_with_usage(data, filters, limits).map(|decoded| decoded.bytes)
}

pub fn decode_filters_with_usage(
    data: &[u8],
    filters: &[Filter],
    limits: StreamLimits,
) -> Result<DecodedStream, StreamError> {
    let specs: Vec<FilterSpec> = filters.iter().copied().map(FilterSpec::from).collect();
    decode_filter_specs_with_usage(data, &specs, limits)
}

pub fn decode_filter_specs(
    data: &[u8],
    filters: &[FilterSpec],
    limits: StreamLimits,
) -> Result<Vec<u8>, StreamError> {
    decode_filter_specs_with_usage(data, filters, limits).map(|decoded| decoded.bytes)
}

pub fn decode_filter_specs_with_usage(
    data: &[u8],
    filters: &[FilterSpec],
    limits: StreamLimits,
) -> Result<DecodedStream, StreamError> {
    decode_chain(data, filters, limits, Mode::Strict)
}

pub fn decode_filter_specs_lenient(
    data: &[u8],
    filters: &[FilterSpec],
    limits: StreamLimits,
) -> Result<DecodedStream, StreamError> {
    decode_chain(data, filters, limits, Mode::Lenient)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Strict,
    Lenient,
}

/// An error inside one stage: the offset into that stage's input.
type StageError = (usize, StreamErrorKind);
type StageResult<T> = Result<T, StageError>;

/// The bytes one stage may produce, and the error for producing more.
#[derive(Clone, Copy)]
struct Budget {
    cap: usize,
    exceeded: fn(usize) -> StreamErrorKind,
    limit: usize,
}

impl Budget {
    /// The cap for a stage after `used` bytes of earlier stages (§3.1 step 6).
    fn new(limits: &StreamLimits, used: usize) -> Self {
        let remaining = limits.max_total_decoded_bytes.saturating_sub(used);
        if remaining < limits.max_decoded_bytes {
            Self {
                cap: remaining,
                exceeded: |limit| StreamErrorKind::TotalOutputLimit { limit },
                limit: limits.max_total_decoded_bytes,
            }
        } else {
            Self {
                cap: limits.max_decoded_bytes,
                exceeded: |limit| StreamErrorKind::DecodedOutputLimit { limit },
                limit: limits.max_decoded_bytes,
            }
        }
    }

    fn error(&self) -> StreamErrorKind {
        (self.exceeded)(self.limit)
    }
}

/// A stage's output buffer: it grows geometrically, never past the cap,
/// and refuses bytes beyond it before they are stored.
struct Output {
    bytes: Vec<u8>,
    budget: Budget,
}

impl Output {
    fn new(budget: Budget) -> Self {
        Self {
            bytes: Vec::new(),
            budget,
        }
    }

    fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Makes room for `additional` more bytes, or fails at input `offset`.
    fn reserve(&mut self, additional: usize, offset: usize) -> StageResult<()> {
        let needed = self
            .bytes
            .len()
            .checked_add(additional)
            .filter(|&needed| needed <= self.budget.cap)
            .ok_or((offset, self.budget.error()))?;
        if needed > self.bytes.capacity() {
            let grown = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(64)
                .min(self.budget.cap)
                .max(needed);
            self.bytes
                .try_reserve_exact(grown - self.bytes.len())
                .map_err(|_| (offset, StreamErrorKind::AllocationFailed))?;
        }
        Ok(())
    }

    fn push(&mut self, byte: u8, offset: usize) -> StageResult<()> {
        self.reserve(1, offset)?;
        self.bytes.push(byte);
        Ok(())
    }

    fn extend(&mut self, bytes: &[u8], offset: usize) -> StageResult<()> {
        self.reserve(bytes.len(), offset)?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn repeat(&mut self, byte: u8, count: usize, offset: usize) -> StageResult<()> {
        self.reserve(count, offset)?;
        self.bytes.resize(self.bytes.len() + count, byte);
        Ok(())
    }
}

fn decode_chain(
    data: &[u8],
    specs: &[FilterSpec],
    limits: StreamLimits,
    mode: Mode,
) -> Result<DecodedStream, StreamError> {
    let chain_error = |offset, kind| StreamError {
        offset,
        filter_index: None,
        kind,
    };
    if data.len() > limits.max_encoded_bytes {
        return Err(chain_error(
            0,
            StreamErrorKind::EncodedInputLimit {
                limit: limits.max_encoded_bytes,
            },
        ));
    }
    if !specs.is_empty() && specs.len() > limits.max_filters {
        return Err(chain_error(
            0,
            StreamErrorKind::FilterCountLimit {
                limit: limits.max_filters,
            },
        ));
    }
    if specs.is_empty() {
        let mut output = Output::new(Budget::new(&limits, 0));
        output
            .extend(data, 0)
            .map_err(|(offset, kind)| chain_error(offset.min(data.len()), kind))?;
        return Ok(DecodedStream {
            usage: DecodeUsage {
                encoded_bytes: data.len(),
                total_decoded_bytes: output.len(),
            },
            bytes: output.bytes,
        });
    }
    let at_stage = |index: usize| {
        move |(offset, kind): StageError| StreamError {
            offset,
            filter_index: Some(index),
            kind,
        }
    };
    // Validate every stage before decoding any.
    let mut geometries = Vec::with_capacity(specs.len());
    for (index, spec) in specs.iter().enumerate() {
        let geometry = validate(spec, &limits).map_err(|kind| at_stage(index)((0, kind)))?;
        geometries.push(geometry);
    }
    let mut current: Cow<'_, [u8]> = Cow::Borrowed(data);
    let mut total = 0usize;
    for (index, (spec, geometry)) in specs.iter().zip(geometries).enumerate() {
        let budget = Budget::new(&limits, total);
        let decoded = decode_stage(spec.filter, &current, budget, mode).map_err(at_stage(index))?;
        total = total.saturating_add(decoded.len());
        current = Cow::Owned(decoded);
        if let Some(geometry) = geometry {
            let budget = Budget::new(&limits, total);
            let predicted = geometry.apply(&current, budget).map_err(at_stage(index))?;
            total = total.saturating_add(predicted.len());
            current = Cow::Owned(predicted);
        }
    }
    Ok(DecodedStream {
        bytes: current.into_owned(),
        usage: DecodeUsage {
            encoded_bytes: data.len(),
            total_decoded_bytes: total,
        },
    })
}

/// Checks a stage's parameters (§3.1 step 4); the row geometry when it
/// has a predictor.
fn validate(spec: &FilterSpec, limits: &StreamLimits) -> Result<Option<Geometry>, StreamErrorKind> {
    let params = spec.predictor;
    if params.predictor != 1 && !matches!(spec.filter, Filter::Flate | Filter::Lzw { .. }) {
        return Err(StreamErrorKind::InvalidPredictor(
            "only FlateDecode and LZWDecode take a predictor",
        ));
    }
    let geometry = Geometry::new(params)?;
    if params.predictor == 1 {
        return Ok(None);
    }
    let budget = Budget::new(limits, 0);
    if geometry.row_bytes > budget.cap {
        return Err(budget.error());
    }
    Ok(Some(geometry))
}

fn decode_stage(filter: Filter, input: &[u8], budget: Budget, mode: Mode) -> StageResult<Vec<u8>> {
    let mut output = Output::new(budget);
    match filter {
        Filter::AsciiHex => ascii_hex(input, &mut output, mode)?,
        Filter::Ascii85 => ascii85(input, &mut output, mode)?,
        Filter::RunLength => run_length(input, &mut output, mode)?,
        Filter::Lzw { early_change } => lzw(input, &mut output, early_change, mode)?,
        Filter::Flate => flate(input, &mut output, mode)?,
    }
    Ok(output.bytes)
}

/// PDF white space (ISO 32000-2 §7.2.3 Table 1).
fn is_white(byte: u8) -> bool {
    matches!(byte, 0x00 | 0x09 | 0x0A | 0x0C | 0x0D | 0x20)
}

/// After an end-of-data marker only white space may follow.
fn only_white_after(input: &[u8], from: usize) -> StageResult<()> {
    let rest = input.get(from..).unwrap_or_default();
    match rest.iter().position(|&b| !is_white(b)) {
        Some(at) => Err((from + at, StreamErrorKind::TrailingData)),
        None => Ok(()),
    }
}

/// ASCIIHexDecode (§7.4.2).
fn ascii_hex(input: &[u8], output: &mut Output, mode: Mode) -> StageResult<()> {
    let mut high: Option<u8> = None;
    let mut end = None;
    for (at, &byte) in input.iter().enumerate() {
        let digit = match byte {
            b'>' => {
                end = Some(at);
                break;
            }
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ if is_white(byte) => continue,
            _ => return Err((at, StreamErrorKind::InvalidByte(byte))),
        };
        match high.take() {
            Some(value) => output.push(value << 4 | digit, at)?,
            None => high = Some(digit),
        }
    }
    match end {
        Some(at) => only_white_after(input, at + 1)?,
        None if mode == Mode::Lenient => {}
        None => return Err((input.len(), StreamErrorKind::MissingEndMarker)),
    }
    // An odd final digit is followed by an implied 0.
    if let Some(value) = high {
        output.push(value << 4, input.len())?;
    }
    Ok(())
}

/// ASCII85Decode (§7.4.3).
fn ascii85(input: &[u8], output: &mut Output, mode: Mode) -> StageResult<()> {
    let mut value = 0u64;
    let mut digits = 0usize;
    let mut group_start = 0usize;
    let mut end = None;
    let mut at = 0usize;
    while let Some(&byte) = input.get(at) {
        match byte {
            b'!'..=b'u' => {
                if digits == 0 {
                    group_start = at;
                }
                value = value * 85 + u64::from(byte - b'!');
                digits += 1;
                if digits == 5 {
                    let word = u32::try_from(value)
                        .map_err(|_| (group_start, StreamErrorKind::Ascii85Overflow))?;
                    output.extend(&word.to_be_bytes(), at)?;
                    value = 0;
                    digits = 0;
                }
            }
            b'z' if digits == 0 => output.extend(&[0; 4], at)?,
            b'z' => return Err((at, StreamErrorKind::InvalidAscii85Group)),
            b'~' => {
                if input.get(at + 1) != Some(&b'>') {
                    return Err((at, StreamErrorKind::InvalidEndMarker));
                }
                end = Some(at);
                break;
            }
            _ if is_white(byte) => {}
            _ => return Err((at, StreamErrorKind::InvalidByte(byte))),
        }
        at += 1;
    }
    let final_at = match end {
        Some(at) => {
            only_white_after(input, at + 2)?;
            at
        }
        None if mode == Mode::Lenient => input.len(),
        None => return Err((input.len(), StreamErrorKind::MissingEndMarker)),
    };
    match digits {
        0 => Ok(()),
        1 => Err((final_at, StreamErrorKind::InvalidAscii85Group)),
        _ => {
            // Pad with `u` (84) to five digits; keep digits - 1 bytes.
            for _ in digits..5 {
                value = value * 85 + 84;
            }
            let word = u32::try_from(value)
                .map_err(|_| (group_start, StreamErrorKind::Ascii85Overflow))?;
            let bytes = word.to_be_bytes();
            output.extend(bytes.get(..digits - 1).unwrap_or_default(), final_at)
        }
    }
}

/// RunLengthDecode (§7.4.5).
fn run_length(input: &[u8], output: &mut Output, mode: Mode) -> StageResult<()> {
    let mut at = 0usize;
    loop {
        let Some(&length) = input.get(at) else {
            if mode == Mode::Lenient {
                return Ok(());
            }
            return Err((input.len(), StreamErrorKind::MissingEndMarker));
        };
        match length {
            0..=127 => {
                let count = usize::from(length) + 1;
                let literal = input
                    .get(at + 1..at + 1 + count)
                    .ok_or((at, StreamErrorKind::TruncatedRun))?;
                output.extend(literal, at)?;
                at += 1 + count;
            }
            128 => return only_white_after(input, at + 1),
            _ => {
                let &byte = input
                    .get(at + 1)
                    .ok_or((at, StreamErrorKind::TruncatedRun))?;
                output.repeat(byte, 257 - usize::from(length), at)?;
                at += 2;
            }
        }
    }
}

/// One LZW table entry: the entry it extends, its last byte, its first
/// byte and its length.
#[derive(Clone, Copy, Default)]
struct LzwEntry {
    prefix: u16,
    last: u8,
    first: u8,
    length: usize,
}

const LZW_CLEAR: u16 = 256;
const LZW_EOD: u16 = 257;
const LZW_FIRST_ENTRY: usize = 258;
const LZW_TABLE_SIZE: usize = 4096;

/// LZWDecode (§7.4.4.2, Table 10 EarlyChange).
fn lzw(input: &[u8], output: &mut Output, early_change: bool, mode: Mode) -> StageResult<()> {
    let mut table = vec![LzwEntry::default(); LZW_TABLE_SIZE];
    for (code, entry) in table.iter_mut().enumerate().take(256) {
        let byte = code as u8;
        *entry = LzwEntry {
            prefix: 0,
            last: byte,
            first: byte,
            length: 1,
        };
    }
    let total_bits = input.len().saturating_mul(8);
    let mut bit = 0usize;
    let mut next = LZW_FIRST_ENTRY;
    let mut previous: Option<u16> = None;
    let mut started = false;
    loop {
        // The decoder adds each entry one code after the encoder, so it
        // widens as soon as its next free entry is 511, 1023, 2047 (one code
        // early) or 512, 1024, 2048.
        let ahead = next + usize::from(early_change);
        let width = match ahead {
            0..512 => 9,
            512..1024 => 10,
            1024..2048 => 11,
            _ => 12,
        };
        let offset = bit / 8;
        if bit + width > total_bits {
            if mode == Mode::Lenient && started {
                return Ok(());
            }
            return Err((input.len(), StreamErrorKind::TruncatedLzw));
        }
        let mut code = 0u16;
        for _ in 0..width {
            let byte = input.get(bit / 8).copied().unwrap_or(0);
            code = code << 1 | u16::from(byte >> (7 - bit % 8) & 1);
            bit += 1;
        }
        if !started && code != LZW_CLEAR {
            return Err((0, StreamErrorKind::MissingLzwClear));
        }
        started = true;
        match code {
            LZW_CLEAR => {
                next = LZW_FIRST_ENTRY;
                previous = None;
                continue;
            }
            LZW_EOD => return lzw_end(input, bit),
            _ => {}
        }
        let index = usize::from(code);
        let Some(prior) = previous else {
            // Right after a clear only the single-byte codes exist.
            if index >= 256 {
                return Err((offset, StreamErrorKind::InvalidLzwCode(code)));
            }
            output.push(code as u8, offset)?;
            previous = Some(code);
            continue;
        };
        if next >= LZW_TABLE_SIZE {
            return Err((offset, StreamErrorKind::LzwDictionaryFull));
        }
        let prior_entry = table[usize::from(prior)];
        let first = if index < next {
            lzw_output(&table, index, output, offset)?;
            table[index].first
        } else if index == next {
            // The entry being defined: the previous string plus its own
            // first byte.
            lzw_output(&table, usize::from(prior), output, offset)?;
            output.push(prior_entry.first, offset)?;
            prior_entry.first
        } else {
            return Err((offset, StreamErrorKind::InvalidLzwCode(code)));
        };
        table[next] = LzwEntry {
            prefix: prior,
            last: first,
            first: prior_entry.first,
            length: prior_entry.length + 1,
        };
        next += 1;
        previous = Some(code);
    }
}

/// Writes the string of table entry `index`, back to front along its
/// prefix chain (no recursion, no scratch space).
fn lzw_output(
    table: &[LzwEntry],
    index: usize,
    output: &mut Output,
    offset: usize,
) -> StageResult<()> {
    let Some(entry) = table.get(index) else {
        return Err((offset, StreamErrorKind::InvalidLzwCode(index as u16)));
    };
    let length = entry.length;
    output.reserve(length, offset)?;
    let start = output.bytes.len();
    output.bytes.resize(start + length, 0);
    let mut code = index;
    for slot in output.bytes[start..].iter_mut().rev() {
        let entry = table.get(code).copied().unwrap_or_default();
        *slot = entry.last;
        code = usize::from(entry.prefix);
    }
    Ok(())
}

/// After the EOD code: zero padding to the byte boundary, then nothing.
fn lzw_end(input: &[u8], bit: usize) -> StageResult<()> {
    let used = bit % 8;
    if used != 0 {
        let byte = input.get(bit / 8).copied().unwrap_or(0);
        if byte & (0xFF >> used) != 0 {
            return Err((bit / 8, StreamErrorKind::NonZeroLzwPadding));
        }
    }
    let end = bit.div_ceil(8);
    if end < input.len() {
        return Err((end, StreamErrorKind::TrailingData));
    }
    Ok(())
}

/// The inflater's window: a power of two of at least 32 KiB, which DEFLATE
/// back-references cannot exceed (RFC 1951).
const WINDOW: usize = 32 * 1024;

/// FlateDecode (§7.4.4, RFC 1950 and RFC 1951).
fn flate(input: &[u8], output: &mut Output, mode: Mode) -> StageResult<()> {
    let lenient = mode == Mode::Lenient;
    // A preset dictionary (FDICT) is refused: PDF defines none.
    if let [cmf, flg, ..] = input {
        if flg & 0x20 != 0 && (u16::from(*cmf) * 256 + u16::from(*flg)) % 31 == 0 {
            return if lenient {
                Ok(())
            } else {
                Err((1, StreamErrorKind::InvalidFlate))
            };
        }
    }
    let mut window = Vec::new();
    window
        .try_reserve_exact(WINDOW)
        .map_err(|_| (0, StreamErrorKind::AllocationFailed))?;
    window.resize(WINDOW, 0);
    let mut inflater = Box::new(DecompressorOxide::new());
    let flags = inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER
        | inflate_flags::TINFL_FLAG_COMPUTE_ADLER32
        | inflate_flags::TINFL_FLAG_HAS_MORE_INPUT;
    let mut read = 0usize;
    let mut position = 0usize;
    loop {
        let rest = input.get(read..).unwrap_or_default();
        let (status, consumed, written) =
            decompress(&mut inflater, rest, &mut window, position, flags);
        read += consumed;
        let produced = window
            .get(position..position + written)
            .ok_or((read, StreamErrorKind::InvalidFlate))?;
        output.extend(produced, read)?;
        position = (position + written) & (WINDOW - 1);
        let problem = match status {
            TINFLStatus::Done => {
                if read < input.len() && !lenient {
                    return Err((read, StreamErrorKind::TrailingData));
                }
                return Ok(());
            }
            TINFLStatus::HasMoreOutput if consumed > 0 || written > 0 => continue,
            TINFLStatus::NeedsMoreInput if read < input.len() && (consumed > 0 || written > 0) => {
                continue;
            }
            TINFLStatus::NeedsMoreInput | TINFLStatus::FailedCannotMakeProgress => {
                StreamErrorKind::TruncatedFlate
            }
            TINFLStatus::Adler32Mismatch => StreamErrorKind::FlateChecksumMismatch,
            _ => StreamErrorKind::InvalidFlate,
        };
        // The lenient reading keeps what decoded before the problem.
        return if lenient {
            Ok(())
        } else {
            Err((read, problem))
        };
    }
}

/// A predictor's row layout (§7.4.4.4, Table 10).
#[derive(Clone, Copy)]
struct Geometry {
    png: bool,
    colors: usize,
    /// Samples in one row: columns × colors.
    samples: usize,
    bits: usize,
    row_bytes: usize,
    pixel_bytes: usize,
}

impl Geometry {
    fn new(params: PredictorParams) -> Result<Self, StreamErrorKind> {
        let png = match params.predictor {
            1 | 2 => false,
            10..=15 => true,
            other => return Err(StreamErrorKind::UnsupportedPredictor(other)),
        };
        let bits = params.bits_per_component;
        if !matches!(bits, 1 | 2 | 4 | 8 | 16) {
            return Err(StreamErrorKind::UnsupportedBitsPerComponent(bits));
        }
        if !(1..=32).contains(&params.colors) {
            return Err(StreamErrorKind::InvalidPredictor("Colors must be 1 to 32"));
        }
        if params.columns == 0 {
            return Err(StreamErrorKind::InvalidPredictor(
                "Columns must be at least 1",
            ));
        }
        let bits = usize::from(bits);
        let pixel_bits = params.colors * bits;
        let row_bits = params
            .columns
            .checked_mul(pixel_bits)
            .ok_or(StreamErrorKind::InvalidPredictor("row size overflows"))?;
        let row_bytes = row_bits.div_ceil(8);
        if row_bytes.checked_add(1).is_none() {
            return Err(StreamErrorKind::InvalidPredictor("row size overflows"));
        }
        Ok(Self {
            png,
            colors: params.colors,
            samples: params.columns * params.colors,
            bits,
            row_bytes,
            pixel_bytes: pixel_bits.div_ceil(8).max(1),
        })
    }

    fn encoded_row(&self) -> usize {
        self.row_bytes + usize::from(self.png)
    }

    fn apply(&self, input: &[u8], budget: Budget) -> StageResult<Vec<u8>> {
        let encoded_row = self.encoded_row();
        if input.len() % encoded_row != 0 {
            return Err((input.len(), StreamErrorKind::TruncatedPredictorRow));
        }
        let rows = input.len() / encoded_row;
        // Each row decodes to at most its encoded size, so this fits.
        let needed = self.row_bytes * rows;
        if needed > budget.cap {
            // The first row that would not fit fails, at its input.
            let fitting = budget.cap / self.row_bytes;
            return Err((fitting * encoded_row, budget.error()));
        }
        let mut output = Output::new(budget);
        output.reserve(needed, 0)?;
        for (row, encoded) in input.chunks_exact(encoded_row).enumerate() {
            if self.png {
                self.png_row(row, encoded, &mut output.bytes, row * encoded_row)?;
            } else {
                output.bytes.extend_from_slice(encoded);
                let start = output.bytes.len() - self.row_bytes;
                self.tiff_row(&mut output.bytes[start..]);
            }
        }
        Ok(output.bytes)
    }

    /// PNG filter types 0-4 (PNG specification §9), bytewise, with "left"
    /// one pixel's bytes back and "above" the previous decoded row.
    fn png_row(
        &self,
        row: usize,
        encoded: &[u8],
        out: &mut Vec<u8>,
        offset: usize,
    ) -> StageResult<()> {
        let Some((&tag, data)) = encoded.split_first() else {
            return Ok(());
        };
        if tag > 4 {
            return Err((offset, StreamErrorKind::InvalidPngFilter(tag)));
        }
        let start = out.len();
        out.extend_from_slice(data);
        let (before, current) = out.split_at_mut(start);
        let above: &[u8] = if row == 0 {
            &[]
        } else {
            before.get(start - self.row_bytes..).unwrap_or_default()
        };
        let bpp = self.pixel_bytes;
        for i in 0..current.len() {
            let left = if i >= bpp { current[i - bpp] } else { 0 };
            let up = above.get(i).copied().unwrap_or(0);
            let upper_left = if i >= bpp {
                above.get(i - bpp).copied().unwrap_or(0)
            } else {
                0
            };
            let predicted = match tag {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                _ => paeth(left, up, upper_left),
            };
            current[i] = current[i].wrapping_add(predicted);
        }
        Ok(())
    }

    /// TIFF Predictor 2 (TIFF 6.0 §14): each sample adds the decoded sample
    /// `colors` positions earlier in the row, modulo 2^bits.
    fn tiff_row(&self, row: &mut [u8]) {
        let samples = self.samples;
        match self.bits {
            8 => {
                for i in self.colors..samples {
                    row[i] = row[i].wrapping_add(row[i - self.colors]);
                }
            }
            16 => {
                for i in self.colors..samples {
                    let sum = sample16(row, i).wrapping_add(sample16(row, i - self.colors));
                    row[2 * i..2 * i + 2].copy_from_slice(&sum.to_be_bytes());
                }
            }
            bits => {
                let mask = (1u16 << bits) - 1;
                let used = samples * bits;
                for i in self.colors..samples {
                    let sum = (sample_small(row, i, bits)
                        + sample_small(row, i - self.colors, bits))
                        & mask;
                    set_sample_small(row, i, bits, sum as u8);
                }
                // The bits past the last sample are zero.
                if used % 8 != 0 {
                    if let Some(last) = row.get_mut(used / 8) {
                        *last &= 0xFF << (8 - used % 8);
                    }
                }
            }
        }
    }
}

fn sample16(row: &[u8], index: usize) -> u16 {
    u16::from_be_bytes([row[2 * index], row[2 * index + 1]])
}

fn sample_small(row: &[u8], index: usize, bits: usize) -> u16 {
    let at = index * bits;
    let shift = 8 - bits - at % 8;
    u16::from(row[at / 8] >> shift) & ((1 << bits) - 1)
}

fn set_sample_small(row: &mut [u8], index: usize, bits: usize, value: u8) {
    let at = index * bits;
    let shift = 8 - bits - at % 8;
    let mask = (((1u16 << bits) - 1) << shift) as u8;
    row[at / 8] = (row[at / 8] & !mask) | (value << shift);
}

/// The PNG Paeth predictor: the neighbour nearest to left + above -
/// upper-left, ties going to left, then above.
fn paeth(left: u8, above: u8, upper_left: u8) -> u8 {
    let estimate = i16::from(left) + i16::from(above) - i16::from(upper_left);
    let to_left = (estimate - i16::from(left)).abs();
    let to_above = (estimate - i16::from(above)).abs();
    let to_upper_left = (estimate - i16::from(upper_left)).abs();
    if to_left <= to_above && to_left <= to_upper_left {
        left
    } else if to_above <= to_upper_left {
        above
    } else {
        upper_left
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every filter and predictor, strict and lenient, on arbitrary bytes:
    /// no panic, output within the cap, offsets within the input.
    #[test]
    fn hostile_input_stays_within_limits_in_both_modes() {
        let limits = StreamLimits {
            max_decoded_bytes: 4096,
            max_total_decoded_bytes: 6144,
            ..StreamLimits::default()
        };
        let predictors = [
            PredictorParams::default(),
            PredictorParams {
                predictor: 2,
                colors: 3,
                bits_per_component: 4,
                columns: 5,
            },
            PredictorParams {
                predictor: 12,
                colors: 2,
                bits_per_component: 16,
                columns: 3,
            },
        ];
        let filters = [
            Filter::Flate,
            Filter::AsciiHex,
            Filter::Ascii85,
            Filter::RunLength,
            Filter::Lzw { early_change: true },
            Filter::Lzw {
                early_change: false,
            },
        ];
        let mut seed = 0x2545_F491_4F6C_DD1D_u64;
        for case in 0..3_000usize {
            let input: Vec<u8> = (0..case % 300)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    (seed >> 24) as u8
                })
                .collect();
            for filter in filters {
                for predictor in predictors {
                    let specs = [
                        FilterSpec {
                            filter,
                            predictor: if matches!(filter, Filter::Flate | Filter::Lzw { .. }) {
                                predictor
                            } else {
                                PredictorParams::default()
                            },
                        },
                        FilterSpec::from(Filter::RunLength),
                    ];
                    for chain in [&specs[..1], &specs[..]] {
                        for result in [
                            decode_filter_specs_with_usage(&input, chain, limits),
                            decode_filter_specs_lenient(&input, chain, limits),
                        ] {
                            match result {
                                Ok(decoded) => {
                                    assert!(decoded.bytes.len() <= limits.max_decoded_bytes);
                                    assert!(
                                        decoded.usage.total_decoded_bytes
                                            <= limits.max_total_decoded_bytes
                                    );
                                }
                                Err(error) => assert!(error.filter_index.is_some()),
                            }
                        }
                    }
                }
            }
        }
    }
}
