// SPDX-License-Identifier: AGPL-3.0-or-later
// ASCII and run-length behavior references MuPDF 1.28.1
// source/fitz/filter-basic.c, Copyright (C) 2004-2026 Artifex Software, Inc.
// See NOTICE. Flate decoding uses the separately licensed miniz_oxide crate.
//! Bounded, strict PDF stream decoding with TIFF and PNG prediction.
//! Dictionary interpretation belongs to the caller; filter specifications
//! include validated typed predictor parameters.
//!
//! ASCII and run-length decoders require their end marker and permit only PDF
//! whitespace after it. Flate requires one complete zlib stream with a valid
//! checksum and no trailing bytes. Each stage's output counts toward the total
//! budget, including an identity copy when there are no filters. A chain holds
//! at most its current input and next output, plus fixed decoder state.

use miniz_oxide::inflate::{
    TINFLStatus,
    core::{DecompressorOxide, decompress, inflate_flags},
};
use std::{borrow::Cow, error::Error, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Flate,
    AsciiHex,
    Ascii85,
    RunLength,
    /// PDF LZW, MSB-first codes; true selects the default EarlyChange 1.
    Lzw {
        early_change: bool,
    },
}

/// PDF predictor parameters. TIFF (2) and PNG (10..=15) support packed 1/2/4,
/// byte 8, and big-endian 16-bit components; Colors is 1..=32 and Columns > 0.
/// PNG rows always carry the actual filter byte 0..=4, including for 10..=14.
/// TIFF sub-byte row padding is cleared, matching MuPDF's decoded output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PredictorParams {
    pub predictor: u8,
    pub colors: usize,
    pub bits_per_component: u8,
    pub columns: usize,
}
impl Default for PredictorParams {
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
    /// Maximum length of the initial encoded input; later inputs are bounded by
    /// the preceding stage's output limit.
    pub max_encoded_bytes: usize,
    pub max_decoded_bytes: usize,
    pub max_filters: usize,
    /// Sum of all stage output lengths, not just the final result.
    pub max_total_decoded_bytes: usize,
}
impl Default for StreamLimits {
    fn default() -> Self {
        Self {
            max_encoded_bytes: 64 * 1024 * 1024,
            max_decoded_bytes: 64 * 1024 * 1024,
            max_filters: 8,
            max_total_decoded_bytes: 128 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamError {
    /// Byte offset within the failing operation's input, not the PDF file.
    /// Predictor failures refer to inflated row bytes, not compressed input.
    pub offset: usize,
    /// Zero-based index into the supplied filter chain. None for preflight or
    /// identity-copy errors.
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
impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(index) = self.filter_index {
            write!(f, "PDF stream filter {index} at byte {}: ", self.offset)?;
        } else {
            write!(f, "PDF stream at byte {}: ", self.offset)?;
        }
        match self.kind {
            StreamErrorKind::EncodedInputLimit { limit } => {
                write!(f, "encoded input exceeds {limit} bytes")
            }
            StreamErrorKind::DecodedOutputLimit { limit } => {
                write!(f, "decoded stage exceeds {limit} bytes")
            }
            StreamErrorKind::TotalOutputLimit { limit } => {
                write!(f, "total decoded output exceeds {limit} bytes")
            }
            StreamErrorKind::FilterCountLimit { limit } => {
                write!(f, "filter chain exceeds {limit} stages")
            }
            StreamErrorKind::MissingEndMarker => f.write_str("missing end-of-data marker"),
            StreamErrorKind::InvalidByte(b) => write!(f, "invalid encoded byte 0x{b:02x}"),
            StreamErrorKind::InvalidEndMarker => f.write_str("invalid end-of-data marker"),
            StreamErrorKind::TrailingData => f.write_str("unexpected trailing data"),
            StreamErrorKind::InvalidAscii85Group => f.write_str("invalid ASCII85 group"),
            StreamErrorKind::Ascii85Overflow => f.write_str("ASCII85 group exceeds 32 bits"),
            StreamErrorKind::TruncatedRun => f.write_str("truncated run-length packet"),
            StreamErrorKind::InvalidFlate => f.write_str("invalid zlib stream"),
            StreamErrorKind::TruncatedFlate => f.write_str("truncated zlib stream"),
            StreamErrorKind::FlateChecksumMismatch => f.write_str("zlib checksum mismatch"),
            StreamErrorKind::AllocationFailed => f.write_str("stream allocation failed"),
            StreamErrorKind::InvalidPredictor(reason) => {
                write!(f, "invalid predictor parameters: {reason}")
            }
            StreamErrorKind::UnsupportedPredictor(p) => write!(f, "unsupported predictor {p}"),
            StreamErrorKind::UnsupportedBitsPerComponent(b) => {
                write!(f, "unsupported predictor bit depth {b}")
            }
            StreamErrorKind::TruncatedPredictorRow => f.write_str("truncated predictor row"),
            StreamErrorKind::InvalidPngFilter(p) => write!(f, "invalid PNG row filter {p}"),
            StreamErrorKind::InvalidLzwCode(code) => write!(f, "invalid LZW code {code}"),
            StreamErrorKind::TruncatedLzw => {
                f.write_str("truncated LZW stream or missing EOD code")
            }
            StreamErrorKind::MissingLzwClear => {
                f.write_str("LZW stream must begin with a clear code")
            }
            StreamErrorKind::LzwDictionaryFull => {
                f.write_str("full LZW dictionary requires a clear or EOD code")
            }
            StreamErrorKind::NonZeroLzwPadding => f.write_str("nonzero LZW padding bits after EOD"),
        }
    }
}
impl Error for StreamError {}

/// Actual work charged by a successful decode. Intermediate filter and
/// predictor outputs are counted even when they are absent from final bytes.
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

/// Decode filters without prediction in PDF `/Filter` order. Callers must
/// separately enforce a document-wide budget across calls.
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
    decode_chain(
        data,
        filters.len(),
        |index| filters[index].into(),
        limits,
        false,
    )
}

/// Decode a filter chain with optional Flate/LZW predictors. Both decompressed bytes
/// and predictor output count toward the cumulative output budget; predictor 1
/// performs no extra copy and therefore contributes no extra stage. The filter
/// count limit counts PDF filters, each of which can add at most one predictor.
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
    decode_chain(data, filters.len(), |index| filters[index], limits, false)
}

/// `decode_filter_specs_with_usage` with Flate as tolerant as MuPDF's
/// filter, for damaged files: bytes after the end of the zlib stream are
/// ignored, and a truncated or corrupt stream or a bad checksum keeps the
/// bytes produced before it.
pub fn decode_filter_specs_lenient(
    data: &[u8],
    filters: &[FilterSpec],
    limits: StreamLimits,
) -> Result<DecodedStream, StreamError> {
    decode_chain(data, filters.len(), |index| filters[index], limits, true)
}

fn decode_chain(
    data: &[u8],
    count: usize,
    specification: impl Fn(usize) -> FilterSpec,
    limits: StreamLimits,
    lenient: bool,
) -> Result<DecodedStream, StreamError> {
    let preflight = |kind| StreamError {
        offset: 0,
        filter_index: None,
        kind,
    };
    if data.len() > limits.max_encoded_bytes {
        return Err(preflight(StreamErrorKind::EncodedInputLimit {
            limit: limits.max_encoded_bytes,
        }));
    }
    if count > limits.max_filters {
        return Err(preflight(StreamErrorKind::FilterCountLimit {
            limit: limits.max_filters,
        }));
    }
    if count == 0 {
        let budget = Budget::new(None, limits, limits.max_total_decoded_bytes);
        let mut output = Vec::new();
        budget.append(&mut output, data, 0)?;
        return Ok(DecodedStream {
            bytes: output,
            usage: DecodeUsage {
                encoded_bytes: data.len(),
                total_decoded_bytes: data.len(),
            },
        });
    }
    // Validate every specification before allocating any decoded output.
    for index in 0..count {
        let spec = specification(index);
        let budget = Budget::new(Some(index), limits, limits.max_total_decoded_bytes);
        if !matches!(spec.filter, Filter::Flate | Filter::Lzw { .. })
            && spec.predictor != PredictorParams::default()
        {
            return Err(budget.error(
                0,
                StreamErrorKind::InvalidPredictor("parameters require FlateDecode or LZWDecode"),
            ));
        }
        PredictorLayout::new(spec.predictor, budget)?;
    }
    let mut current = Cow::Borrowed(data);
    let mut remaining = limits.max_total_decoded_bytes;
    for index in 0..count {
        let spec = specification(index);
        let budget = Budget::new(Some(index), limits, remaining);
        let output = match spec.filter {
            Filter::Flate => flate(&current, budget, lenient)?,
            Filter::AsciiHex => ascii_hex(&current, budget)?,
            Filter::Ascii85 => ascii85(&current, budget)?,
            Filter::RunLength => run_length(&current, budget)?,
            Filter::Lzw { early_change } => lzw(&current, early_change, budget)?,
        };
        remaining -= output.len();
        current = Cow::Owned(output);
        if spec.predictor.predictor != 1 {
            let budget = Budget::new(Some(index), limits, remaining);
            let output = predict(&current, spec.predictor, budget)?;
            remaining -= output.len();
            current = Cow::Owned(output);
        }
    }
    Ok(DecodedStream {
        bytes: current.into_owned(),
        usage: DecodeUsage {
            encoded_bytes: data.len(),
            total_decoded_bytes: limits.max_total_decoded_bytes - remaining,
        },
    })
}

#[derive(Clone, Copy)]
struct Budget {
    filter_index: Option<usize>,
    limit: usize,
    total_limited: bool,
    total_limit: usize,
}
impl Budget {
    fn new(filter_index: Option<usize>, limits: StreamLimits, remaining: usize) -> Self {
        Self {
            filter_index,
            limit: limits.max_decoded_bytes.min(remaining),
            total_limited: remaining < limits.max_decoded_bytes,
            total_limit: limits.max_total_decoded_bytes,
        }
    }
    fn error(self, offset: usize, kind: StreamErrorKind) -> StreamError {
        StreamError {
            offset,
            filter_index: self.filter_index,
            kind,
        }
    }
    fn limit_error(self, offset: usize) -> StreamError {
        self.error(
            offset,
            if self.total_limited {
                StreamErrorKind::TotalOutputLimit {
                    limit: self.total_limit,
                }
            } else {
                StreamErrorKind::DecodedOutputLimit { limit: self.limit }
            },
        )
    }
    fn reserve(
        self,
        output: &mut Vec<u8>,
        additional: usize,
        offset: usize,
    ) -> Result<(), StreamError> {
        let Some(needed) = output
            .len()
            .checked_add(additional)
            .filter(|&n| n <= self.limit)
        else {
            return Err(self.limit_error(offset));
        };
        if needed > output.capacity() {
            let capacity = output
                .capacity()
                .saturating_mul(2)
                .max(64)
                .max(needed)
                .min(self.limit);
            output
                .try_reserve_exact(capacity - output.len())
                .map_err(|_| self.error(offset, StreamErrorKind::AllocationFailed))?;
        }
        Ok(())
    }
    fn append(self, output: &mut Vec<u8>, bytes: &[u8], offset: usize) -> Result<(), StreamError> {
        self.reserve(output, bytes.len(), offset)?;
        output.extend_from_slice(bytes);
        Ok(())
    }
    fn trailing(self, data: &[u8], start: usize) -> Result<(), StreamError> {
        if let Some(index) = data[start..].iter().position(|&b| !is_whitespace(b)) {
            return Err(self.error(start + index, StreamErrorKind::TrailingData));
        }
        Ok(())
    }
}

fn ascii_hex(data: &[u8], budget: Budget) -> Result<Vec<u8>, StreamError> {
    let mut output = Vec::new();
    let mut high = None;
    for (offset, &byte) in data.iter().enumerate() {
        if byte == b'>' {
            if let Some(nibble) = high {
                budget.append(&mut output, &[nibble << 4], offset)?;
            }
            budget.trailing(data, offset + 1)?;
            return Ok(output);
        }
        if is_whitespace(byte) {
            continue;
        }
        let nibble = match byte {
            b'0'..=b'9' => byte - b'0',
            b'A'..=b'F' => byte - b'A' + 10,
            b'a'..=b'f' => byte - b'a' + 10,
            _ => return Err(budget.error(offset, StreamErrorKind::InvalidByte(byte))),
        };
        if let Some(previous) = high.take() {
            budget.append(&mut output, &[(previous << 4) | nibble], offset)?;
        } else {
            high = Some(nibble);
        }
    }
    Err(budget.error(data.len(), StreamErrorKind::MissingEndMarker))
}

fn ascii85(data: &[u8], budget: Budget) -> Result<Vec<u8>, StreamError> {
    let mut output = Vec::new();
    let mut word = 0u64;
    let mut count = 0usize;
    for (offset, &byte) in data.iter().enumerate() {
        match byte {
            b'!'..=b'u' => {
                word = word * 85 + u64::from(byte - b'!');
                count += 1;
                if count == 5 {
                    let value = u32::try_from(word)
                        .map_err(|_| budget.error(offset, StreamErrorKind::Ascii85Overflow))?;
                    budget.append(&mut output, &value.to_be_bytes(), offset)?;
                    word = 0;
                    count = 0;
                }
            }
            b'z' if count == 0 => budget.append(&mut output, &[0; 4], offset)?,
            b'z' => return Err(budget.error(offset, StreamErrorKind::InvalidAscii85Group)),
            b'~' => {
                if data.get(offset + 1) != Some(&b'>') {
                    return Err(budget.error(offset, StreamErrorKind::InvalidEndMarker));
                }
                if count == 1 {
                    return Err(budget.error(offset, StreamErrorKind::InvalidAscii85Group));
                }
                if count > 1 {
                    for _ in count..5 {
                        word = word * 85 + 84;
                    }
                    let value = u32::try_from(word)
                        .map_err(|_| budget.error(offset, StreamErrorKind::Ascii85Overflow))?;
                    budget.append(&mut output, &value.to_be_bytes()[..count - 1], offset)?;
                }
                budget.trailing(data, offset + 2)?;
                return Ok(output);
            }
            b if is_whitespace(b) => {}
            _ => return Err(budget.error(offset, StreamErrorKind::InvalidByte(byte))),
        }
    }
    Err(budget.error(data.len(), StreamErrorKind::MissingEndMarker))
}

fn run_length(data: &[u8], budget: Budget) -> Result<Vec<u8>, StreamError> {
    let mut output = Vec::new();
    let mut offset = 0;
    while let Some(&control) = data.get(offset) {
        let packet_start = offset;
        offset += 1;
        match control {
            128 => {
                budget.trailing(data, offset)?;
                return Ok(output);
            }
            0..=127 => {
                let count = usize::from(control) + 1;
                let Some(end) = offset.checked_add(count).filter(|&n| n <= data.len()) else {
                    return Err(budget.error(packet_start, StreamErrorKind::TruncatedRun));
                };
                budget.append(&mut output, &data[offset..end], packet_start)?;
                offset = end;
            }
            129..=255 => {
                let Some(&byte) = data.get(offset) else {
                    return Err(budget.error(packet_start, StreamErrorKind::TruncatedRun));
                };
                offset += 1;
                let count = 257 - usize::from(control);
                budget.reserve(&mut output, count, packet_start)?;
                output.resize(output.len() + count, byte);
            }
        }
    }
    Err(budget.error(data.len(), StreamErrorKind::MissingEndMarker))
}

fn flate(data: &[u8], budget: Budget, lenient: bool) -> Result<Vec<u8>, StreamError> {
    // Keep decoder state on the stack, avoiding an unreportable Box allocation.
    let mut decoder = DecompressorOxide::new();
    let mut output = Vec::new();
    let mut input_offset = 0usize;
    let mut output_offset = 0usize;
    let initial_size = data.len().saturating_mul(2).min(4096).min(budget.limit);
    budget.reserve(&mut output, initial_size, 0)?;
    output.resize(initial_size, 0);
    let flags = inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER
        | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
    loop {
        let (status, consumed, produced) = decompress(
            &mut decoder,
            &data[input_offset..],
            &mut output,
            output_offset,
            flags,
        );
        // Treat a violated dependency contract as an error instead of indexing
        // or adding unchecked values obtained from the decoder.
        if consumed > data.len() - input_offset || produced > output.len() - output_offset {
            return Err(budget.error(input_offset, StreamErrorKind::InvalidFlate));
        }
        input_offset += consumed;
        output_offset += produced;
        let stopped = !matches!(status, TINFLStatus::Done | TINFLStatus::HasMoreOutput);
        if lenient && stopped {
            output.truncate(output_offset);
            return Ok(output);
        }
        match status {
            TINFLStatus::Done => {
                if input_offset != data.len() && !lenient {
                    return Err(budget.error(input_offset, StreamErrorKind::TrailingData));
                }
                output.truncate(output_offset);
                return Ok(output);
            }
            TINFLStatus::HasMoreOutput => {
                if output.len() == budget.limit {
                    return Err(budget.limit_error(input_offset));
                }
                // Growing strictly on each continuation prevents zero-progress
                // loops, including for a zero-sized initial output buffer.
                let new_len = output.len().saturating_mul(2).max(1).min(budget.limit);
                let additional = new_len - output.len();
                budget.reserve(&mut output, additional, input_offset)?;
                output.resize(new_len, 0);
            }
            TINFLStatus::Adler32Mismatch => {
                return Err(budget.error(input_offset, StreamErrorKind::FlateChecksumMismatch));
            }
            TINFLStatus::FailedCannotMakeProgress | TINFLStatus::NeedsMoreInput => {
                return Err(budget.error(input_offset, StreamErrorKind::TruncatedFlate));
            }
            _ => return Err(budget.error(input_offset, StreamErrorKind::InvalidFlate)),
        }
    }
}

fn is_whitespace(byte: u8) -> bool {
    matches!(byte, 0 | 9 | 10 | 12 | 13 | 32)
}

// Predictor behavior references MuPDF 1.28.1 source/fitz/filter-predict.c,
// Copyright (C) 2004-2021 Artifex Software, Inc. Partial rows and invalid PNG
// filter bytes fail explicitly instead of following MuPDF recovery behavior.
struct PredictorLayout {
    row_bytes: usize,
    bytes_per_pixel: usize,
    samples: usize,
}
impl PredictorLayout {
    fn new(parameters: PredictorParams, budget: Budget) -> Result<Self, StreamError> {
        let invalid = |reason| budget.error(0, StreamErrorKind::InvalidPredictor(reason));
        if !matches!(parameters.predictor, 1 | 2 | 10..=15) {
            return Err(budget.error(
                0,
                StreamErrorKind::UnsupportedPredictor(parameters.predictor),
            ));
        }
        if !matches!(parameters.bits_per_component, 1 | 2 | 4 | 8 | 16) {
            return Err(budget.error(
                0,
                StreamErrorKind::UnsupportedBitsPerComponent(parameters.bits_per_component),
            ));
        }
        if parameters.colors == 0 || parameters.colors > 32 {
            return Err(invalid("Colors must be 1..=32"));
        }
        if parameters.columns == 0 {
            return Err(invalid("Columns must be positive"));
        }
        let bits = usize::from(parameters.bits_per_component);
        let samples = parameters
            .colors
            .checked_mul(parameters.columns)
            .ok_or_else(|| invalid("row sample count overflows usize"))?;
        let row_bytes = samples
            .checked_mul(bits)
            .and_then(|n| n.checked_add(7))
            .map(|n| n / 8)
            .ok_or_else(|| invalid("row byte count overflows usize"))?;
        let bytes_per_pixel = (parameters.colors * bits).div_ceil(8);
        if parameters.predictor != 1 && row_bytes > budget.limit {
            return Err(budget.limit_error(0));
        }
        Ok(Self {
            row_bytes,
            bytes_per_pixel,
            samples,
        })
    }
}

fn predict(
    data: &[u8],
    parameters: PredictorParams,
    budget: Budget,
) -> Result<Vec<u8>, StreamError> {
    let layout = PredictorLayout::new(parameters, budget)?;
    let png = parameters.predictor >= 10;
    let encoded_row = layout
        .row_bytes
        .checked_add(usize::from(png))
        .ok_or_else(|| {
            budget.error(
                0,
                StreamErrorKind::InvalidPredictor("encoded row size overflows usize"),
            )
        })?;
    if data.len() % encoded_row != 0 {
        return Err(budget.error(data.len(), StreamErrorKind::TruncatedPredictorRow));
    }
    let rows = data.len() / encoded_row;
    // Output cannot exceed input, but use checked arithmetic to keep this
    // invariant explicit before reserving or indexing any output row.
    let output_size = rows.checked_mul(layout.row_bytes).ok_or_else(|| {
        budget.error(
            0,
            StreamErrorKind::InvalidPredictor("output size overflows usize"),
        )
    })?;
    let mut output = Vec::new();
    budget.reserve(&mut output, output_size, 0)?;
    output.resize(output_size, 0);
    for (row, encoded) in data.chunks_exact(encoded_row).enumerate() {
        let start = row * layout.row_bytes;
        if png {
            let mode = encoded[0];
            if mode > 4 {
                return Err(
                    budget.error(row * encoded_row, StreamErrorKind::InvalidPngFilter(mode))
                );
            }
            for (column, &byte) in encoded[1..].iter().enumerate() {
                let left = if column >= layout.bytes_per_pixel {
                    output[start + column - layout.bytes_per_pixel]
                } else {
                    0
                };
                let above = if row > 0 {
                    output[start + column - layout.row_bytes]
                } else {
                    0
                };
                let upper_left = if row > 0 && column >= layout.bytes_per_pixel {
                    output[start + column - layout.row_bytes - layout.bytes_per_pixel]
                } else {
                    0
                };
                let prediction = match mode {
                    0 => 0,
                    1 => left,
                    2 => above,
                    3 => ((u16::from(left) + u16::from(above)) / 2) as u8,
                    4 => paeth(left, above, upper_left),
                    _ => unreachable!(),
                };
                output[start + column] = byte.wrapping_add(prediction);
            }
        } else {
            let output_row = &mut output[start..start + layout.row_bytes];
            let mask = (1u32 << parameters.bits_per_component) - 1;
            for sample in 0..layout.samples {
                let previous = if sample >= parameters.colors {
                    get_sample(
                        output_row,
                        sample - parameters.colors,
                        parameters.bits_per_component,
                    )
                } else {
                    0
                };
                let encoded = get_sample(encoded, sample, parameters.bits_per_component);
                put_sample(
                    output_row,
                    sample,
                    parameters.bits_per_component,
                    (encoded + previous) & mask,
                );
            }
        }
    }
    Ok(output)
}

fn paeth(left: u8, above: u8, upper_left: u8) -> u8 {
    let a = i32::from(left);
    let b = i32::from(above);
    let c = i32::from(upper_left);
    let estimate = a + b - c;
    let da = (estimate - a).abs();
    let db = (estimate - b).abs();
    let dc = (estimate - c).abs();
    if da <= db && da <= dc {
        left
    } else if db <= dc {
        above
    } else {
        upper_left
    }
}
fn get_sample(row: &[u8], sample: usize, bits: u8) -> u32 {
    match bits {
        16 => u32::from(u16::from_be_bytes([row[sample * 2], row[sample * 2 + 1]])),
        8 => u32::from(row[sample]),
        _ => {
            let bit = sample * usize::from(bits);
            u32::from(row[bit / 8] >> (8 - usize::from(bits) - bit % 8)) & ((1u32 << bits) - 1)
        }
    }
}
fn put_sample(row: &mut [u8], sample: usize, bits: u8, value: u32) {
    match bits {
        16 => {
            let bytes = (value as u16).to_be_bytes();
            row[sample * 2..sample * 2 + 2].copy_from_slice(&bytes);
        }
        8 => row[sample] = value as u8,
        _ => {
            let bit = sample * usize::from(bits);
            row[bit / 8] |= (value as u8) << (8 - usize::from(bits) - bit % 8);
        }
    }
}

// PDF LZW behavior references MuPDF 1.28.1 source/fitz/filter-lzw.c,
// Copyright (C) 2004-2022 Artifex Software, Inc. The dictionary is fixed-size,
// prefix links always point backward, and expansion is bounded before copying.
fn lzw(data: &[u8], early_change: bool, budget: Budget) -> Result<Vec<u8>, StreamError> {
    const CLEAR: usize = 256;
    const EOD: usize = 257;
    const FIRST: usize = 258;
    const CAPACITY: usize = 4096;
    let mut prefixes = [0u16; CAPACITY];
    let mut suffixes = [0u8; CAPACITY];
    let mut first_bytes = [0u8; CAPACITY];
    let mut lengths = [0u16; CAPACITY];
    for literal in 0..256 {
        suffixes[literal] = literal as u8;
        first_bytes[literal] = literal as u8;
        lengths[literal] = 1;
    }
    let mut scratch = [0u8; CAPACITY];
    let mut reader = LzwBits {
        data,
        position: 0,
        buffer: 0,
        bits: 0,
    };
    let mut width = 9;
    let mut next_code = FIRST;
    let mut previous = None;
    let mut initial = true;
    let mut output = Vec::new();
    loop {
        let offset = reader.offset();
        let code = usize::from(
            reader
                .code(width)
                .ok_or_else(|| budget.error(offset, StreamErrorKind::TruncatedLzw))?,
        );
        if initial {
            if code != CLEAR {
                return Err(budget.error(offset, StreamErrorKind::MissingLzwClear));
            }
            initial = false;
        }
        if code == CLEAR {
            width = 9;
            next_code = FIRST;
            previous = None;
            continue;
        }
        if code == EOD {
            if reader.buffer != 0 {
                return Err(budget.error(
                    reader.position.saturating_sub(1),
                    StreamErrorKind::NonZeroLzwPadding,
                ));
            }
            if reader.position != data.len() {
                return Err(budget.error(reader.position, StreamErrorKind::TrailingData));
            }
            return Ok(output);
        }
        if next_code == CAPACITY {
            return Err(budget.error(offset, StreamErrorKind::LzwDictionaryFull));
        }
        if let Some(previous_code) = previous {
            if code > next_code {
                return Err(budget.error(offset, StreamErrorKind::InvalidLzwCode(code as u16)));
            }
            prefixes[next_code] = previous_code as u16;
            first_bytes[next_code] = first_bytes[previous_code];
            suffixes[next_code] = if code == next_code {
                first_bytes[previous_code]
            } else {
                first_bytes[code]
            };
            lengths[next_code] = lengths[previous_code] + 1;
            next_code += 1;
            if width < 12 && next_code == (1usize << width) - usize::from(early_change) {
                width += 1;
            }
        } else if code >= CLEAR {
            return Err(budget.error(offset, StreamErrorKind::InvalidLzwCode(code as u16)));
        }
        let length = usize::from(lengths[code]);
        budget.reserve(&mut output, length, offset)?;
        let mut cursor = code;
        let mut start = CAPACITY;
        while cursor >= FIRST {
            if start == 0 || usize::from(prefixes[cursor]) >= cursor {
                return Err(budget.error(offset, StreamErrorKind::InvalidLzwCode(code as u16)));
            }
            start -= 1;
            scratch[start] = suffixes[cursor];
            cursor = usize::from(prefixes[cursor]);
        }
        if cursor >= CLEAR || start == 0 {
            return Err(budget.error(offset, StreamErrorKind::InvalidLzwCode(code as u16)));
        }
        start -= 1;
        scratch[start] = cursor as u8;
        if CAPACITY - start != length {
            return Err(budget.error(offset, StreamErrorKind::InvalidLzwCode(code as u16)));
        }
        output.extend_from_slice(&scratch[start..]);
        previous = Some(code);
    }
}

struct LzwBits<'a> {
    data: &'a [u8],
    position: usize,
    buffer: u32,
    bits: u8,
}
impl LzwBits<'_> {
    fn offset(&self) -> usize {
        self.position - usize::from(self.bits.div_ceil(8))
    }
    fn code(&mut self, width: u8) -> Option<u16> {
        while self.bits < width {
            self.buffer = (self.buffer << 8) | u32::from(*self.data.get(self.position)?);
            self.position += 1;
            self.bits += 8;
        }
        self.bits -= width;
        let code = (self.buffer >> self.bits) as u16;
        self.buffer &= (1u32 << self.bits) - 1;
        Some(code)
    }
}
