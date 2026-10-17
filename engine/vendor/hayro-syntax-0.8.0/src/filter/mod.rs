//! Results from decoding filtered data streams.

mod ascii_85;
pub(crate) mod ascii_hex;
#[cfg(feature = "images")]
mod ccitt;
#[cfg(feature = "images")]
mod dct;
#[cfg(feature = "images")]
mod jbig2;
#[cfg(feature = "images")]
mod jpx;
mod lzw_flate;
mod png;
mod run_length;

use crate::object::Dict;
use crate::object::Name;
use crate::object::dict::keys::*;
use crate::object::stream::{DecodeFailure, FilterResult, ImageDecodeParams};
use alloc::vec::Vec;
use core::ops::Deref;

/// Tandem patch: room for `extra` more bytes in `out`, which grows by
/// doubling but not past `limit + 1` (a decoder stops once past `limit`), so
/// output near the limit does not double its buffer past it.
pub(crate) fn reserve_within(out: &mut Vec<u8>, extra: usize, limit: usize) {
    let wanted = out.len().saturating_add(extra);
    if wanted > out.capacity() {
        let target = out
            .capacity()
            .saturating_mul(2)
            .min(limit.saturating_add(1))
            .max(wanted);
        out.reserve_exact(target - out.len());
    }
}

/// Tandem patch: whether FlateDecode with `params` decodes as a stream
/// (`Stream::decoded_streamed`).
pub(crate) fn flate_streams(params: &Dict<'_>) -> bool {
    lzw_flate::flate::streams(params)
}

/// Tandem patch: FlateDecode with `params` handed to `sink` as it inflates.
pub(crate) fn flate_decode_streamed(
    data: &[u8],
    params: &Dict<'_>,
    sink: &mut dyn FnMut(&[u8]) -> bool,
) -> bool {
    lzw_flate::flate::decode_streamed(data, params, sink)
}

/// A data filter.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Filter {
    /// ASCII hexadecimal encoding.
    AsciiHexDecode,
    /// ASCII base-85 encoding.
    Ascii85Decode,
    /// Lempel-Ziv-Welch (LZW) compression.
    LzwDecode,
    /// DEFLATE compression (zlib/gzip).
    FlateDecode,
    /// Run-length encoding compression.
    RunLengthDecode,
    /// CCITT Group 3 or Group 4 fax compression.
    CcittFaxDecode,
    /// JBIG2 compression for bi-level images.
    Jbig2Decode,
    /// JPEG (DCT) compression.
    DctDecode,
    /// JPEG 2000 compression.
    JpxDecode,
    /// Encryption filter.
    Crypt,
}

impl Filter {
    fn debug_name(&self) -> &'static str {
        match self {
            Self::AsciiHexDecode => "ascii_hex",
            Self::Ascii85Decode => "ascii_85",
            Self::LzwDecode => "lzw",
            Self::FlateDecode => "flate",
            Self::RunLengthDecode => "run-length",
            Self::CcittFaxDecode => "ccit_fax",
            Self::Jbig2Decode => "jbig2",
            Self::DctDecode => "dct",
            Self::JpxDecode => "jpx",
            Self::Crypt => "crypt",
        }
    }

    pub(crate) fn from_name(name: Name<'_>) -> Option<Self> {
        match name.deref() {
            ASCII_HEX_DECODE | ASCII_HEX_DECODE_ABBREVIATION => Some(Self::AsciiHexDecode),
            ASCII85_DECODE | ASCII85_DECODE_ABBREVIATION => Some(Self::Ascii85Decode),
            LZW_DECODE | LZW_DECODE_ABBREVIATION => Some(Self::LzwDecode),
            FLATE_DECODE | FLATE_DECODE_ABBREVIATION => Some(Self::FlateDecode),
            RUN_LENGTH_DECODE | RUN_LENGTH_DECODE_ABBREVIATION => Some(Self::RunLengthDecode),
            CCITTFAX_DECODE | CCITTFAX_DECODE_ABBREVIATION => Some(Self::CcittFaxDecode),
            JBIG2_DECODE => Some(Self::Jbig2Decode),
            DCT_DECODE | DCT_DECODE_ABBREVIATION => Some(Self::DctDecode),
            JPX_DECODE => Some(Self::JpxDecode),
            CRYPT => Some(Self::Crypt),
            _ => {
                warn!("unknown filter: {}", name.as_str());

                None
            }
        }
    }

    pub(crate) fn apply(
        &self,
        data: &[u8],
        params: &Dict<'_>,
        #[cfg_attr(not(feature = "images"), allow(unused))] image_params: &ImageDecodeParams,
        limit: usize,
    ) -> Result<FilterResult<'static>, DecodeFailure> {
        // Tandem patch: these filters stop once their output passes `limit`
        // (they return it then, longer than `limit`), and that is a failure.
        let limited = |data: Option<Vec<u8>>| match data {
            Some(data) if data.len() > limit => Err(DecodeFailure::TooLarge),
            Some(data) => Ok(FilterResult::from_data(data)),
            None => Err(DecodeFailure::StreamDecode),
        };
        let res = match self {
            Self::AsciiHexDecode => limited(ascii_hex::decode(data)),
            Self::Ascii85Decode => limited(ascii_85::decode(data, limit)),
            Self::RunLengthDecode => limited(run_length::decode(data, limit)),
            Self::LzwDecode => limited(lzw_flate::lzw::decode(data, params, limit)),
            Self::FlateDecode => limited(lzw_flate::flate::decode(data, params, limit)),
            #[cfg(feature = "images")]
            Self::DctDecode => {
                dct::decode(data, params, image_params).ok_or(DecodeFailure::ImageDecode)
            }
            #[cfg(feature = "images")]
            Self::CcittFaxDecode => {
                ccitt::decode(data, params, image_params).ok_or(DecodeFailure::ImageDecode)
            }
            #[cfg(feature = "images")]
            Self::Jbig2Decode => {
                jbig2::decode(data, params, image_params).ok_or(DecodeFailure::ImageDecode)
            }
            #[cfg(feature = "images")]
            Self::JpxDecode => jpx::decode(data, image_params).ok_or(DecodeFailure::ImageDecode),
            #[cfg(not(feature = "images"))]
            Self::DctDecode | Self::CcittFaxDecode | Self::Jbig2Decode | Self::JpxDecode => {
                warn!("image decoding is not supported (enable the `images` feature)");
                Err(DecodeFailure::ImageDecode)
            }
            _ => Err(DecodeFailure::StreamDecode),
        };

        if res.is_err() {
            warn!("failed to apply filter {}", self.debug_name());
        }

        res
    }
}
