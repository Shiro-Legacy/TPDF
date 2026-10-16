//! Tandem patch: what an image's decoders allocate, read before they run.
//! The image filters size their output from declared sizes before decoding
//! anything (the image dictionary for DCT, the parameters for CCITT, the
//! stream's own headers for JBIG2 and JPX), so a few bytes could ask for
//! gigabytes; on wasm32 that, or a size product past `usize`, stops the
//! engine. Sizes are in u64; an image past its budget is not decoded.

use crate::x_object::image::ImageXObject;
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::object::{Array, Dict, Name, Object, Stream};
use std::iter;

/// What one JPX sample costs: hayro-jpeg2000 decodes into 32-bit floats.
const JPX_SAMPLE_COST: u64 = 4;

/// The target dimension `obj` decodes at within its budget (a JPX image
/// skips more resolution levels when that fits it), or `None` when it would
/// pass the budget.
pub(super) fn fit(
    obj: &ImageXObject<'_>,
    target: Option<(u32, u32)>,
) -> Option<Option<(u32, u32)>> {
    let budget = obj.max_samples;
    let components = obj
        .color_space
        .as_ref()
        .map(|cs| u64::from(cs.num_components()));
    // One row is never cut short, so it must fit whatever the data holds.
    if u64::from(obj.width).saturating_mul(components.unwrap_or(1)) > budget {
        return None;
    }
    let dict = obj.stream.dict();
    let mut target = target;
    for (index, (filter, params)) in filters(dict).into_iter().enumerate() {
        // A filter's headers are read only when its input is the stream's
        // own bytes.
        let raw = (index == 0).then(|| obj.stream.raw_data());
        let samples = match filter.as_ref() {
            DCT_DECODE | DCT_DECODE_ABBREVIATION => {
                dct_samples(raw.as_deref(), obj.width, obj.height, components)
            }
            CCITTFAX_DECODE | CCITTFAX_DECODE_ABBREVIATION => {
                // As hayro-syntax sizes its output.
                let columns = params.get::<usize>(COLUMNS).unwrap_or(1728) as u32;
                let rows = params.get::<u32>(ROWS).unwrap_or(0).max(obj.height);
                area(columns, rows)
            }
            JBIG2_DECODE => match &raw {
                Some(raw) => {
                    let globals = params
                        .get::<Stream<'_>>(JBIG2_GLOBALS)
                        .and_then(|g| g.decoded().ok());
                    jbig2_samples(raw, globals.as_deref())
                }
                None => u64::MAX,
            },
            JPX_DECODE => match &raw {
                Some(raw) => {
                    let (samples, fitted) = jpx_fit(raw, target, budget)?;
                    target = fitted;
                    samples
                }
                None => u64::MAX,
            },
            _ => 0,
        };
        if samples > budget {
            return None;
        }
    }
    Some(target)
}

fn area(width: u32, height: u32) -> u64 {
    u64::from(width) * u64::from(height)
}

/// The filters of `dict` with their parameters, as hayro-syntax pairs them.
fn filters<'a>(dict: &Dict<'a>) -> Vec<(Name<'a>, Dict<'a>)> {
    if let Some(name) = dict
        .get::<Name<'_>>(F)
        .or_else(|| dict.get::<Name<'_>>(FILTER))
    {
        let params = dict
            .get::<Dict<'_>>(DP)
            .or_else(|| dict.get::<Dict<'_>>(DECODE_PARMS))
            .unwrap_or_default();
        return vec![(name, params)];
    }
    let Some(names) = dict
        .get::<Array<'_>>(F)
        .or_else(|| dict.get::<Array<'_>>(FILTER))
    else {
        return Vec::new();
    };
    let mut params = dict
        .get::<Array<'_>>(DP)
        .or_else(|| dict.get::<Array<'_>>(DECODE_PARMS))
        .map(|a| a.iter::<Object<'_>>());
    names
        .iter::<Name<'_>>()
        .map(|name| {
            let params = params
                .as_mut()
                .and_then(|p| p.next())
                .and_then(|p| p.into_dict())
                .unwrap_or_default();
            (name, params)
        })
        .collect()
}

fn be16(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        data.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn be32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        data.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// The bytes zune-jpeg allocates for a DCT image of `width` × `height` in a
/// colour space of `components`: its output at the frame's size (patched
/// down to the dictionary's when larger, as hayro-syntax does), and for a
/// progressive frame 16-bit coefficients of every component. Without the
/// frame (`jpeg` not the stream's own bytes), the dictionary's size at four
/// components. hayro-syntax refuses sides past 65,535.
fn dct_samples(jpeg: Option<&[u8]>, width: u32, height: u32, components: Option<u64>) -> u64 {
    if width > u32::from(u16::MAX) || height > u32::from(u16::MAX) {
        return 0;
    }
    let declared = area(width, height);
    let Some(jpeg) = jpeg else {
        return declared.saturating_mul(4);
    };
    // A JPEG without a frame header does not decode.
    let Some((frame_width, frame_height, inputs, progressive)) = jpeg_frame(jpeg) else {
        return 0;
    };
    let pixels = area(frame_width, frame_height).min(declared);
    // zune-jpeg writes the frame's own components unless the colour space
    // asks for another count.
    let outputs = match (components, inputs) {
        (Some(n), _) if !matches!(n, 1 | 3 | 4) => n,
        (_, 1 | 2) => 1,
        (_, 4) => 4,
        _ => 3,
    };
    let coefficients = if progressive { 2 * inputs } else { 0 };
    pixels.saturating_mul(outputs.saturating_add(coefficients))
}

/// The first frame header of a JPEG, found as hayro-syntax finds it: its
/// width, height, component count and whether it is progressive.
fn jpeg_frame(data: &[u8]) -> Option<(u32, u32, u64, bool)> {
    let mut at = 0usize;
    while at.checked_add(1)? < data.len() {
        if data[at] != 0xFF {
            at += 1;
            continue;
        }
        match data[at + 1] {
            marker @ 0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                let height = be16(data, at.checked_add(5)?)?;
                let width = be16(data, at.checked_add(7)?)?;
                let components = *data.get(at.checked_add(9)?)?;
                let progressive = matches!(marker, 0xC2 | 0xC6 | 0xCA | 0xCE);
                return Some((
                    u32::from(width),
                    u32::from(height),
                    u64::from(components),
                    progressive,
                ));
            }
            0xFF => at += 1,
            0xD8 | 0xD9 | 0x01 | 0x00 => at += 2,
            _ => {
                let length = usize::from(be16(data, at.checked_add(2)?)?);
                at = at.checked_add(2)?.checked_add(length)?;
            }
        }
    }
    None
}

/// The bytes a JBIG2 stream (with its globals) has hayro decode into: its
/// page as a byte a pixel (hayro-syntax's output, allocated from the page
/// information before decoding) and a bit a pixel (hayro-jbig2's page), the
/// largest region bitmap at a bit a pixel, and the referred-to segment
/// numbers hayro-jbig2 reserves room for before reading them. A stream that
/// stops parsing costs what was read before it: hayro-jbig2 refuses it.
fn jbig2_samples(data: &[u8], globals: Option<&[u8]>) -> u64 {
    let mut page = 0u64;
    let mut region = 0u64;
    let mut reserved = 0u64;
    let mut width = 0u64;
    let mut stripes = None::<u64>;
    let mut striped = false;
    for part in globals.into_iter().chain(iter::once(data)) {
        let mut at = 0;
        while at < part.len() {
            let Some((kind, body)) = jbig2_segment(part, &mut at, &mut reserved) else {
                break;
            };
            match kind {
                // Page information: width, height.
                48 => match (be32(body, 0), be32(body, 4)) {
                    (Some(w), Some(0xFFFF_FFFF)) => {
                        width = width.max(u64::from(w));
                        striped = true;
                    }
                    (Some(w), Some(h)) => page = page.max(area(w, h)),
                    _ => {}
                },
                // End of stripe: the last row.
                50 => {
                    if let Some(row) = be32(body, 0) {
                        stripes = stripes.max(Some(u64::from(row) + 1));
                    }
                }
                // Regions: the region segment information field.
                4 | 6 | 7 | 20 | 22 | 23 | 36 | 38 | 39 | 40 | 42 | 43 => {
                    if let (Some(w), Some(h)) = (be32(body, 0), be32(body, 4)) {
                        region = region.max(area(w, h).div_ceil(8));
                    }
                }
                _ => {}
            }
        }
    }
    // A striped page takes its height from its stripes (without any, it
    // does not decode).
    if striped && let Some(rows) = stripes {
        page = page.max(width.saturating_mul(rows));
    }
    page.saturating_add(page / 8)
        .saturating_add(region)
        .saturating_add(reserved)
}

/// The segment at `*at`, as hayro-jbig2 reads a sequential segment: its
/// type and data, `*at` moved past it; `None` when it ends early. `reserved`
/// grows to the bytes hayro-jbig2 reserves for the referred-to segment
/// numbers once their retention bits are read.
fn jbig2_segment<'a>(
    data: &'a [u8],
    at: &mut usize,
    reserved: &mut u64,
) -> Option<(u8, &'a [u8])> {
    let number = be32(data, *at)?;
    let flags = *data.get(at.checked_add(4)?)?;
    let first = *data.get(*at + 5)?;
    let short = first >> 5;
    let mut pos = *at + 6;
    let count = match short {
        0..=4 => u64::from(short),
        7 => {
            let count = u64::from(be32(data, *at + 5)? & 0x1FFF_FFFF);
            pos = (*at + 9).checked_add(usize::try_from((count + 1).div_ceil(8)).ok()?)?;
            if pos > data.len() {
                return None;
            }
            count
        }
        _ => return None,
    };
    *reserved = (*reserved).max(count.saturating_mul(4));
    let size = if number <= 256 {
        1
    } else if number <= 65536 {
        2
    } else {
        4
    };
    pos = pos.checked_add(usize::try_from(count.checked_mul(size)?).ok()?)?;
    pos = pos.checked_add(if flags & 0x40 != 0 { 4 } else { 1 })?;
    let length = be32(data, pos)?;
    pos += 4;
    let length = if length == 0xFFFF_FFFF {
        // An immediate generic region of unknown length ends at its end
        // sequence and a row count.
        let marker: [u8; 2] = if *data.get(pos.checked_add(17)?)? & 1 != 0 {
            [0x00, 0x00]
        } else {
            [0xFF, 0xAC]
        };
        let from = pos + 18;
        let end = data
            .get(from..)?
            .windows(6)
            .position(|w| w[..2] == marker)?;
        from + end + 6 - pos
    } else {
        usize::try_from(length).ok()?
    };
    let body = data.get(pos..pos.checked_add(length)?)?;
    *at = pos + length;
    Some((flags & 0x3F, body))
}

/// The cost of a JPX image (its samples at the resolution decoded, each
/// JPX_SAMPLE_COST) and the target dimension to decode it at: the caller's,
/// or a smaller one that skips more resolution levels to fit `budget`.
/// `None` when no level fits. A stream that does not parse costs nothing:
/// hayro-jpeg2000 refuses it before allocating.
fn jpx_fit(
    data: &[u8],
    target: Option<(u32, u32)>,
    budget: u64,
) -> Option<(u64, Option<(u32, u32)>)> {
    let (streams, palette) = jpx_codestreams(data);
    let mut most = 0;
    let mut fitted = target;
    for stream in streams {
        let Some((width, height, components, levels)) = jpx_header(stream) else {
            continue;
        };
        // hayro-jpeg2000 skips levels by the target, never past the
        // fewest any component has, and none for a palette.
        let levels = if palette { 0 } else { levels };
        let wanted = target.map_or(0, |(tw, th)| {
            let log = |from: u32, to: u32| from.checked_div(to).and_then(u32::checked_ilog2);
            log(width, tw).unwrap_or(0).min(log(height, th).unwrap_or(0))
        });
        let cost = |skip: u32| {
            let side = |v: u32| u64::from(v).div_ceil(1 << skip);
            side(width)
                .saturating_mul(side(height))
                .saturating_mul(components)
                .saturating_mul(JPX_SAMPLE_COST)
        };
        let skip = (wanted.min(levels)..=levels).find(|&skip| cost(skip) <= budget)?;
        if skip > wanted.min(levels) {
            let side = |v: u32| v.checked_shr(skip).unwrap_or(0).max(1);
            fitted = Some((side(width), side(height)));
        }
        most = most.max(cost(skip));
    }
    Some((most, fitted))
}

/// The codestreams of a JPX stream (a raw codestream, or the codestream
/// boxes of a JP2 file), and whether it has a palette.
fn jpx_codestreams(data: &[u8]) -> (Vec<&[u8]>, bool) {
    if data.starts_with(&[0xFF, 0x4F, 0xFF, 0x51]) {
        return (vec![data], false);
    }
    let mut streams = Vec::new();
    let mut palette = false;
    for (kind, content) in jp2_boxes(data) {
        match &kind {
            b"jp2c" => streams.push(content),
            b"jp2h" => palette |= jp2_boxes(content).any(|(kind, _)| &kind == b"pclr"),
            _ => {}
        }
    }
    (streams, palette)
}

/// The boxes of a JP2 box sequence, as type and content.
fn jp2_boxes(data: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut at = 0usize;
    iter::from_fn(move || {
        let length = be32(data, at)?;
        let kind: [u8; 4] = data.get(at + 4..at.checked_add(8)?)?.try_into().ok()?;
        let (header, size) = match length {
            0 => (8, data.len() - at),
            1 => (
                16,
                usize::try_from(
                    (u64::from(be32(data, at + 8)?) << 32) | u64::from(be32(data, at + 12)?),
                )
                .ok()?,
            ),
            n => (8, usize::try_from(n).ok()?),
        };
        if size < header {
            return None;
        }
        let end = at.checked_add(size)?.min(data.len());
        let content = data.get(at + header..end)?;
        at = end;
        Some((kind, content))
    })
}

/// A codestream's image size, component count and the fewest decomposition
/// levels its main header gives a component.
fn jpx_header(stream: &[u8]) -> Option<(u32, u32, u64, u32)> {
    if be16(stream, 0)? != 0xFF4F || be16(stream, 2)? != 0xFF51 {
        return None;
    }
    let width = be32(stream, 8)?.saturating_sub(be32(stream, 16)?);
    let height = be32(stream, 12)?.saturating_sub(be32(stream, 20)?);
    let components = be16(stream, 40)?;
    let mut levels = None::<u32>;
    let mut at = 4usize.checked_add(usize::from(be16(stream, 4)?))?;
    while let Some(marker) = be16(stream, at) {
        if matches!(marker, 0xFF90 | 0xFFD9) {
            break;
        }
        let length = usize::from(be16(stream, at + 2)?);
        let nl = match marker {
            0xFF52 => stream.get(at + 9),
            0xFF53 => stream.get(at + 5 + if components < 257 { 1 } else { 2 }),
            _ => None,
        };
        if let Some(nl) = nl {
            levels = Some(levels.map_or(u32::from(*nl), |l| l.min(u32::from(*nl))));
        }
        at = at.checked_add(2 + length)?;
    }
    Some((width, height, u64::from(components), levels.unwrap_or(0).min(32)))
}
