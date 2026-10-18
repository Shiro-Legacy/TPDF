mod budget;
mod image;
mod mask;

pub(crate) use image::{DecodedImage, decode_image, decodes_alike};
pub(crate) use mask::{DecodedMask, decode_mask};

use crate::InterpreterWarning;
use crate::color::ColorSpace;
use crate::function::interpolate;
use crate::x_object::image::{ImageKind, ImageXObject};
use hayro_syntax::bit_reader::BitReader;
use hayro_syntax::object::Array;
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::object::stream::{
    DecodeFailure, FilterResult, ImageColorSpace, ImageDecodeParams,
};
use smallvec::SmallVec;
use std::borrow::Cow;
use std::iter;

struct DecodeContext<'a> {
    decoded: FilterResult<'a>,
    width: u32,
    height: u32,
    scale_factors: (f32, f32),
    color_space: ColorSpace,
    bits_per_component: u8,
    decode_arr: SmallVec<[(f32, f32); 4]>,
    /// Tandem patch: the data is not decoded here (`decoded` is empty) but
    /// as the image is reduced (`decode_image_streamed`).
    streamed: bool,
}

// Tandem patch: `streamable`, whether the caller can decode the data as it
// is reduced.
fn decode_context<'a>(
    obj: &ImageXObject<'a>,
    target_dimension: Option<(u32, u32)>,
    streamable: bool,
) -> Option<DecodeContext<'a>> {
    let dict = obj.stream.dict();
    let dict_bpc = dict
        .get::<u8>(BPC)
        .or_else(|| dict.get::<u8>(BITS_PER_COMPONENT));
    let color_space = obj.color_space.clone();
    let is_indexed = obj.color_space.as_ref().is_some_and(|cs| cs.is_indexed());

    // Tandem patch: decoders that size their output from declared sizes
    // stay within the image's budget (a JPX image may skip more levels).
    let Some(target_dimension) = budget::fit(obj, target_dimension) else {
        return over_budget(obj);
    };

    let decode_params = ImageDecodeParams {
        is_indexed,
        bpc: dict_bpc,
        num_components: color_space.as_ref().map(|c| c.num_components()),
        target_dimension,
        width: obj.width,
        height: obj.height,
    };

    // Tandem patch: data that may decode past what hayro-syntax holds for an
    // image is decoded as the image is reduced, or refused before it is
    // decoded; data decoding past its limit is over budget.
    let streamed = match budget::past_holding(obj, &decode_params) {
        Some(_) if streamable && obj.stream.streams_decoded() => true,
        Some(true) => return over_budget(obj),
        _ => false,
    };
    let decoded = if streamed {
        FilterResult {
            data: Cow::Borrowed(&[]),
            image_data: None,
        }
    } else {
        match obj.stream.decoded_image(&decode_params) {
            Ok(decoded) => decoded,
            Err(DecodeFailure::TooLarge) => return over_budget(obj),
            Err(_) => {
                (obj.warning_sink)(InterpreterWarning::ImageDecodeFailure);
                return None;
            }
        }
    };

    let (mut scale_x, mut scale_y) = (1.0, 1.0);

    let (width, height) = decoded
        .image_data
        .as_ref()
        .map(|d| {
            scale_x = obj.width as f32 / d.width as f32;
            scale_y = obj.height as f32 / d.height as f32;

            (d.width, d.height)
        })
        .unwrap_or((obj.width, obj.height));

    let color_space = color_space
        .or_else(|| {
            decoded
                .image_data
                .as_ref()
                .map(|i| i.color_space)
                .and_then(|c| {
                    c.and_then(|c| match c {
                        ImageColorSpace::Gray => Some(ColorSpace::device_gray()),
                        ImageColorSpace::Rgb => Some(ColorSpace::device_rgb()),
                        ImageColorSpace::Cmyk => Some(ColorSpace::device_cmyk()),
                        ImageColorSpace::Unknown(_) => None,
                    })
                })
        })
        .unwrap_or(ColorSpace::device_gray());

    let fallback_bpc = if obj.kind == ImageKind::StencilMask {
        1
    } else {
        8
    };

    let bits_per_component = decoded
        .image_data
        .as_ref()
        .map(|i| i.bits_per_component)
        .or(dict_bpc)
        .unwrap_or(fallback_bpc);

    let decode_arr = dict
        .get::<Array<'_>>(D)
        .or_else(|| dict.get::<Array<'_>>(DECODE))
        .map(|a| a.iter::<(f32, f32)>().collect::<SmallVec<_>>())
        .unwrap_or(color_space.default_decode_arr(bits_per_component as f32));

    Some(DecodeContext {
        decoded,
        width,
        height,
        scale_factors: (scale_x, scale_y),
        color_space,
        bits_per_component,
        decode_arr,
        streamed,
    })
}

/// Tandem patch: decoded images a render keeps, so an image drawn many
/// times is decoded once.
pub(crate) const REDUCED_CACHE_BYTES: usize = 32 << 20;

/// Tandem patch: whole-number box factors for an image of `width` ×
/// `height` samples drawn at `target` pixels, along the axes where it is
/// drawn more than BOX_REDUCTION times smaller; the renderer's resampler does
/// the rest.
pub(crate) fn reduction(width: u32, height: u32, target: Option<(u32, u32)>) -> (usize, usize) {
    const BOX_REDUCTION: u32 = 8;
    let Some((target_width, target_height)) = target else {
        return (1, 1);
    };
    let factor = |from: u32, to: u32| {
        let ratio = from / to.max(1);
        if ratio > BOX_REDUCTION {
            (ratio / (BOX_REDUCTION / 2)) as usize
        } else {
            1
        }
    };
    (factor(width, target_width), factor(height, target_height))
}

/// Tandem patch: the largest whole-number box factors that keep an image of
/// `width` × `height` samples at least `target` pixels, for an image whose
/// samples pass its budget.
pub(crate) fn fit_reduction(
    width: u32,
    height: u32,
    target: Option<(u32, u32)>,
) -> (usize, usize) {
    target.map_or((1, 1), |(target_width, target_height)| {
        let factor = |from: u32, to: u32| (from / to.max(1)).max(1) as usize;
        (factor(width, target_width), factor(height, target_height))
    })
}

/// Tandem patch: `data` (`width` × `height` pixels of `n` bytes) averaged
/// over `kx` × `ky` blocks; partial blocks at the edges average what they
/// hold. `None` unless `data` holds exactly those pixels.
pub(crate) fn box_reduce(
    data: &[u8],
    width: usize,
    height: usize,
    n: usize,
    kx: usize,
    ky: usize,
) -> Option<Vec<u8>> {
    let row = width.checked_mul(n)?;
    if width == 0 || n == 0 || row.checked_mul(height)? != data.len() {
        return None;
    }
    let (kx, ky) = (kx.clamp(1, width), ky.clamp(1, height.max(1)));
    let (out_width, out_height) = (width.div_ceil(kx), height.div_ceil(ky));
    let mut out = Vec::with_capacity(out_width * out_height * n);
    let mut sums = vec![0u64; out_width * n];
    for block_rows in data.chunks(row * ky) {
        sums.iter_mut().for_each(|s| *s = 0);
        let row_count = (block_rows.len() / row) as u64;
        for row in block_rows.chunks_exact(row) {
            for (bx, block) in row.chunks(kx * n).enumerate() {
                let sum = &mut sums[bx * n..(bx + 1) * n];
                for pixel in block.chunks_exact(n) {
                    for (s, v) in sum.iter_mut().zip(pixel) {
                        *s += u64::from(*v);
                    }
                }
            }
        }
        for bx in 0..out_width {
            let count = ((bx * kx + kx).min(width) - bx * kx) as u64 * row_count;
            for s in &sums[bx * n..(bx + 1) * n] {
                out.push(((s + count / 2) / count) as u8);
            }
        }
    }
    Some(out)
}

/// Tandem patch: the rows an image's samples are unpacked into within
/// `budget` samples: the declared `height` for 1 to 7 and 9 to 15 bits per
/// component (missing data read as 0, as reference viewers pad it), else the rows its
/// `len` bytes hold; `None` when even those pass the budget.
pub(crate) fn unpacked_rows(
    len: usize,
    width: u32,
    height: u32,
    n: usize,
    bits: u8,
    budget: u64,
) -> Option<u32> {
    let row = u64::from(width).saturating_mul(n as u64);
    if matches!(bits, 1..=7 | 9..=15) && row.saturating_mul(u64::from(height)) <= budget {
        return Some(height);
    }
    let row_bytes = row.saturating_mul(u64::from(bits)).div_ceil(8).max(1);
    let held = (len as u64).div_ceil(row_bytes).min(u64::from(height));
    (row.saturating_mul(held) <= budget).then_some(held as u32)
}

/// Tandem patch: `None`, after telling the warning sink that `obj` passed
/// its budget.
pub(crate) fn over_budget<T>(obj: &ImageXObject<'_>) -> Option<T> {
    warn!("image exceeds its decoding budget");
    (obj.warning_sink)(InterpreterWarning::BudgetExceeded);
    None
}

#[must_use]
fn fix_image_length<T: Copy>(
    image: &mut Vec<T>,
    width: u32,
    height: &mut u32,
    filler: T,
    num_components: usize,
) -> Option<()> {
    // Tandem patch: sizes in u64 (on wasm32 `row_len * height` wrapped and
    // cut the data to nothing while the height stayed); a row longer than
    // the largest budget is refused rather than padded.
    let row_len = u64::from(width) * num_components as u64;
    if row_len == 0 || row_len > crate::MAX_IMAGE_SAMPLES.max(image.len() as u64) {
        return None;
    }
    let len = image.len() as u64;

    if row_len.saturating_mul(u64::from(*height)) <= len {
        // Too much data (or just the right amount), truncate it.
        image.truncate((row_len * u64::from(*height)) as usize);
    } else {
        // Too little data, adapt the height and pad.
        *height = len.div_ceil(row_len) as u32;

        if !len.is_multiple_of(row_len) {
            image.extend(iter::repeat_n(filler, (row_len - len % row_len) as usize));
        }
    }

    if width == 0 || *height == 0 {
        None
    } else {
        Some(())
    }
}

fn decode_u8_samples(
    data: &[u8],
    width: u32,
    height: u32,
    color_space: &ColorSpace,
    bits_per_component: u8,
    decode: &[(f32, f32)],
) -> Option<Vec<u8>> {
    let source_max = 2.0_f32.powi(bits_per_component as i32) - 1.0;
    let num_components = color_space.num_components() as usize;
    let ranges = color_space.component_ranges();
    let indexed_hival = color_space.indexed_hival();

    let decode_component = |value: u32, index: usize| {
        let component_index = index % num_components;
        let (decode_min, decode_max) = *decode.get(component_index)?;
        let decoded = interpolate(value as f32, 0.0, source_max, decode_min, decode_max);

        if let Some(hival) = indexed_hival {
            Some((decoded + 0.5).clamp(0.0, hival as f32) as u8)
        } else {
            let (range_min, range_max) = *ranges.get(component_index)?;
            let normalized = if range_min == range_max {
                0.0
            } else {
                (decoded - range_min) / (range_max - range_min)
            };
            Some((normalized * 255.0 + 0.5) as u8)
        }
    };

    match bits_per_component {
        1..8 | 9..16 => {
            let mut buf = Vec::with_capacity(samples(width, height, num_components)?);
            for_each_sample(
                data,
                width,
                height,
                num_components,
                bits_per_component,
                |value, index| {
                    buf.push(decode_component(value, index)?);
                    Some(())
                },
            )?;

            Some(buf)
        }
        8 => Some(
            data.iter()
                .enumerate()
                .map(|(index, value)| decode_component(*value as u32, index))
                .collect::<Option<Vec<_>>>()?,
        ),
        16 => Some(
            data.chunks_exact(2)
                .enumerate()
                .map(|(index, value)| {
                    decode_component(u16::from_be_bytes([value[0], value[1]]) as u32, index)
                })
                .collect::<Option<Vec<_>>>()?,
        ),
        _ => {
            warn!("unsupported bits per component: {bits_per_component}");
            None
        }
    }
}

fn unpack_samples(
    data: &[u8],
    width: u32,
    height: u32,
    num_components: usize,
    bits_per_component: u8,
) -> Option<Vec<u16>> {
    match bits_per_component {
        1..8 | 9..16 => {
            let mut buf = Vec::with_capacity(samples(width, height, num_components)?);
            for_each_sample(
                data,
                width,
                height,
                num_components,
                bits_per_component,
                |value, _| {
                    buf.push(value as u16);
                    Some(())
                },
            )?;

            Some(buf)
        }
        8 => Some(data.iter().map(|value| *value as u16).collect()),
        16 => Some(
            data.chunks_exact(2)
                .map(|value| u16::from_be_bytes([value[0], value[1]]))
                .collect(),
        ),
        _ => {
            warn!("unsupported bits per component: {bits_per_component}");
            None
        }
    }
}

/// Tandem patch: `width` × `height` × `n` as a `usize` (callers bound it
/// with `unpacked_rows`).
fn samples(width: u32, height: u32, n: usize) -> Option<usize> {
    let samples = u64::from(width)
        .checked_mul(u64::from(height))?
        .checked_mul(n as u64)?;
    usize::try_from(samples).ok()
}

fn for_each_sample(
    data: &[u8],
    width: u32,
    height: u32,
    num_components: usize,
    bits_per_component: u8,
    mut visit: impl FnMut(u32, usize) -> Option<()>,
) -> Option<()> {
    let mut reader = BitReader::new(data);
    let mut index = 0;

    for _ in 0..height {
        for _ in 0..width {
            for _ in 0..num_components {
                // See `stream_ccit_not_enough_data`, some images seemingly don't have
                // enough data, so we just pad with zeroes in this case.
                visit(reader.read(bits_per_component).unwrap_or(0), index)?;
                index += 1;
            }
        }

        reader.align();
    }

    Some(())
}
