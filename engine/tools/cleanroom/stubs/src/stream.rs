use std::{error::Error, fmt};

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
    fn default() -> Self {
        todo!("clean-room: spec 02")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterSpec {
    pub filter: Filter,
    pub predictor: PredictorParams,
}

impl From<Filter> for FilterSpec {
    fn from(_filter: Filter) -> Self {
        todo!("clean-room: spec 02")
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
        todo!("clean-room: spec 02")
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

impl fmt::Display for StreamError {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("clean-room: spec 02")
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
    _data: &[u8],
    _filters: &[Filter],
    _limits: StreamLimits,
) -> Result<Vec<u8>, StreamError> {
    todo!("clean-room: spec 02")
}

pub fn decode_filters_with_usage(
    _data: &[u8],
    _filters: &[Filter],
    _limits: StreamLimits,
) -> Result<DecodedStream, StreamError> {
    todo!("clean-room: spec 02")
}

pub fn decode_filter_specs(
    _data: &[u8],
    _filters: &[FilterSpec],
    _limits: StreamLimits,
) -> Result<Vec<u8>, StreamError> {
    todo!("clean-room: spec 02")
}

pub fn decode_filter_specs_with_usage(
    _data: &[u8],
    _filters: &[FilterSpec],
    _limits: StreamLimits,
) -> Result<DecodedStream, StreamError> {
    todo!("clean-room: spec 02")
}

pub fn decode_filter_specs_lenient(
    _data: &[u8],
    _filters: &[FilterSpec],
    _limits: StreamLimits,
) -> Result<DecodedStream, StreamError> {
    todo!("clean-room: spec 02")
}
