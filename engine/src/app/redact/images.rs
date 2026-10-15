// SPDX-License-Identifier: AGPL-3.0-or-later
//! Image samples for redact-apply: an image XObject or inline image
//! decoded (the strict decoder, else the view's codecs: DCT, JPX, JBIG2,
//! CCITT), the pixels under the marks overwritten, and the samples written
//! back Flate-compressed. A region is a convex quadrilateral in pixel space
//! (x right, y down from the top row); a pixel goes when its square
//! overlaps a region with positive area.
use crate::app::annots::{Point, invert};
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};
use crate::stream::{self, Filter, FilterSpec, PredictorParams, StreamErrorKind, StreamLimits};
use crate::stream_object::StreamObjectErrorKind;
use crate::view::{ImageParams, ViewDocument};

/// redact.ts `MAX_REDACT_IMAGE_PIXELS`.
pub const MAX_IMAGE_PIXELS: u64 = 40_000_000;
/// Rows of regions an image's blanking may visit in all.
const MAX_ROW_WORK: usize = 50_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// Past `MAX_IMAGE_PIXELS` or a work bound.
    TooLarge(String),
    /// An image this version cannot decode or write back.
    Unsupported(String),
}

/// Packed samples, rows top to bottom, each row padded to a byte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raster {
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub bits: u8,
    pub components: u8,
}

impl Raster {
    pub fn stride(&self) -> usize {
        (self.width * usize::from(self.components) * usize::from(self.bits)).div_ceil(8)
    }

    /// Fits the data to the declared size: short data padded with `fill`,
    /// long data cut.
    fn fit(&mut self, fill: u8) {
        let len = self.stride() * self.height;
        self.data.resize(len, fill);
    }

    /// Sets every sample of pixels `x0..x1` of row `y` to `value`.
    fn set(&mut self, y: usize, x0: usize, x1: usize, value: u16) {
        let stride = self.stride();
        let bits = usize::from(self.bits);
        let per = usize::from(self.components) * bits;
        let Some(row) = self.data.get_mut(y * stride..(y + 1) * stride) else {
            return;
        };
        if bits >= 8 {
            let bytes = bits / 8;
            let sample = value.to_be_bytes();
            let pattern = if bytes == 2 {
                &sample[..]
            } else {
                &sample[1..]
            };
            let (from, to) = (x0 * per / 8, (x1 * per / 8).min(row.len()));
            if let Some(slice) = row.get_mut(from..to) {
                for chunk in slice.chunks_mut(bytes) {
                    chunk.copy_from_slice(pattern.get(..chunk.len()).unwrap_or(&[]));
                }
            }
            return;
        }
        let mask = (1u16 << bits) - 1;
        let value = value & mask;
        for bit in (x0 * per..x1 * per).step_by(bits) {
            let (byte, shift) = (bit / 8, 8 - bits - bit % 8);
            if let Some(b) = row.get_mut(byte) {
                let m = (mask << shift) as u8;
                *b = (*b & !m) | (((value << shift) as u8) & m);
            }
        }
    }
}

/// Page-space `rects` over an image whose unit square goes to page space
/// through `matrix`, as quads in the unit square's space; None for a
/// singular matrix.
pub fn unit_regions(rects: &[[f64; 4]], matrix: &[f64; 6]) -> Option<Vec<[Point; 4]>> {
    let det = matrix[0] * matrix[3] - matrix[1] * matrix[2];
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let back = invert(*matrix);
    let to_unit = |(x, y): Point| {
        (
            x * back[0] + y * back[2] + back[4],
            x * back[1] + y * back[3] + back[5],
        )
    };
    Some(
        rects
            .iter()
            .map(|r| [(r[0], r[1]), (r[2], r[1]), (r[2], r[3]), (r[0], r[3])].map(&to_unit))
            .collect(),
    )
}

/// Unit-square quads in the pixel space of a `width` by `height` image.
pub fn pixel_regions(unit: &[[Point; 4]], width: usize, height: usize) -> Vec<[Point; 4]> {
    let (w, h) = (width as f64, height as f64);
    unit.iter()
        .map(|quad| quad.map(|(u, v)| (u * w, (1.0 - v) * h)))
        .collect()
}

/// The x range of a convex quad within the band `y0..y1`, if it has one.
fn span_in_band(quad: &[Point; 4], y0: f64, y1: f64) -> Option<(f64, f64)> {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut take = |x: f64| {
        lo = lo.min(x);
        hi = hi.max(x);
    };
    for i in 0..4 {
        let (a, b) = (quad[i], quad[(i + 1) % 4]);
        if a.1 >= y0 && a.1 <= y1 {
            take(a.0);
        }
        for y in [y0, y1] {
            if (a.1 - y) * (b.1 - y) < 0.0 {
                let t = (y - a.1) / (b.1 - a.1);
                take(a.0 + (b.0 - a.0) * t);
            }
        }
    }
    (lo <= hi).then_some((lo, hi))
}

/// Blanks every pixel a region overlaps with `value`. Returns the pixels
/// blanked (each once).
pub fn blank(raster: &mut Raster, regions: &[[Point; 4]], value: u16) -> Result<u64, ImageError> {
    let (w, h) = (raster.width, raster.height);
    // Each region's rows, swept top to bottom with the regions open on a
    // row, so each row's spans merge before they are written.
    let mut spans: Vec<(usize, usize, &[Point; 4])> = Vec::new();
    for quad in regions {
        let (y_lo, y_hi) = (quad.iter().map(|p| p.1))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), y| {
                (a.min(y), b.max(y))
            });
        if !(y_lo.is_finite() && y_hi.is_finite()) || y_hi <= 0.0 || y_lo >= h as f64 {
            continue;
        }
        let first = y_lo.max(0.0).floor() as usize;
        let last = (y_hi.min(h as f64).ceil() as usize).min(h);
        if first < last {
            spans.push((first, last, quad));
        }
    }
    spans.sort_by_key(|(first, _, _)| *first);
    let mut next = 0;
    let mut open: Vec<(usize, &[Point; 4])> = Vec::new();
    let mut work = 0usize;
    let mut blanked = 0u64;
    let mut y = spans.first().map_or(h, |(first, _, _)| *first);
    let mut row: Vec<(usize, usize)> = Vec::new();
    while y < h {
        while let Some(&(first, last, quad)) = spans.get(next).filter(|(f, _, _)| *f <= y) {
            if last > y && first <= y {
                open.push((last, quad));
            }
            next += 1;
        }
        open.retain(|(last, _)| *last > y);
        if open.is_empty() {
            match spans.get(next) {
                Some(&(first, _, _)) => {
                    y = first;
                    continue;
                }
                None => break,
            }
        }
        work = work.saturating_add(open.len());
        if work > MAX_ROW_WORK {
            return Err(ImageError::TooLarge(
                "the marks cover too many image rows".into(),
            ));
        }
        row.clear();
        for (_, quad) in &open {
            let Some((x0, x1)) = span_in_band(quad, y as f64, y as f64 + 1.0) else {
                continue;
            };
            if x1 <= 0.0 || x0 >= w as f64 {
                continue;
            }
            let a = x0.max(0.0).floor() as usize;
            let b = (x1.min(w as f64).ceil() as usize).min(w);
            if a < b {
                row.push((a, b));
            }
        }
        row.sort_unstable();
        let mut merged: Option<(usize, usize)> = None;
        for &(a, b) in row.iter().chain(std::iter::once(&(usize::MAX, usize::MAX))) {
            match merged.as_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => {
                    if let Some((c, d)) = merged.take() {
                        raster.set(y, c, d, value);
                        blanked += (d - c) as u64;
                    }
                    merged = (a != usize::MAX).then_some((a, b));
                }
            }
        }
        y += 1;
    }
    Ok(blanked)
}

/// Bytes an image's samples may take (the walk's decoded-stream bound).
const MAX_IMAGE_BYTES: usize = 256 << 20;

/// Refuses samples past `MAX_IMAGE_BYTES`.
fn check_bytes(raster: &Raster) -> Result<(), ImageError> {
    match raster.stride().checked_mul(raster.height) {
        Some(bytes) if bytes <= MAX_IMAGE_BYTES => Ok(()),
        _ => Err(ImageError::TooLarge(format!(
            "an image's samples exceed {MAX_IMAGE_BYTES} bytes"
        ))),
    }
}

/// Components of a colour space, None when unknown.
pub fn components(doc: &Document<'_>, space: &Object) -> Option<u8> {
    let space = doc.resolve(space);
    if let Some(name) = space.as_name() {
        return match name {
            b"DeviceGray" | b"G" | b"CalGray" | b"Indexed" | b"I" | b"Separation" => Some(1),
            b"DeviceRGB" | b"RGB" | b"CalRGB" | b"Lab" => Some(3),
            b"DeviceCMYK" | b"CMYK" => Some(4),
            _ => None,
        };
    }
    let items = space.as_array()?;
    let family = items
        .first()
        .map(|f| doc.resolve(f))
        .and_then(Object::as_name)?;
    match family {
        b"ICCBased" => {
            let stream = items.get(1)?.as_reference()?;
            let n = doc.dict_get(doc.stream_dict(stream).ok()?, b"N").as_i64()?;
            u8::try_from(n).ok().filter(|n| (1..=32).contains(n))
        }
        b"DeviceN" => {
            let names = doc.resolve(items.get(1)?).as_array()?;
            u8::try_from(names.len())
                .ok()
                .filter(|n| (1..=32).contains(n))
        }
        b"Indexed" | b"I" | b"Separation" | b"CalGray" | b"DeviceGray" | b"G" => Some(1),
        b"CalRGB" | b"Lab" | b"DeviceRGB" | b"RGB" => Some(3),
        b"DeviceCMYK" | b"CMYK" => Some(4),
        _ => None,
    }
}

fn indexed(doc: &Document<'_>, space: &Object) -> bool {
    let space = doc.resolve(space);
    let family = match space.as_array() {
        Some(items) => items.first().map(|f| doc.resolve(f)),
        None => Some(space),
    };
    matches!(family.and_then(Object::as_name), Some(b"Indexed" | b"I"))
}

/// The filter names of a dictionary's /Filter (or inline /F).
fn filter_names(doc: &Document<'_>, value: Option<&Object>) -> Vec<Vec<u8>> {
    match value.map(|v| doc.resolve(v)) {
        Some(Object::Name(name)) => vec![name.clone()],
        Some(Object::Array(items)) => items
            .iter()
            .filter_map(|i| doc.resolve(i).as_name().map(<[u8]>::to_vec))
            .collect(),
        _ => Vec::new(),
    }
}

fn is_codec(name: &[u8]) -> bool {
    matches!(
        name,
        b"DCTDecode" | b"DCT" | b"JPXDecode" | b"JBIG2Decode" | b"CCITTFaxDecode" | b"CCF"
    )
}

/// The value that blanks an image: no paint for a stencil mask, else 0.
pub fn blank_value(doc: &Document<'_>, dict: &Dictionary, mask: bool) -> u16 {
    if !mask {
        return 0;
    }
    // A sample of 0 paints the mask unless /Decode is [1 0].
    let inverted = doc
        .dict_get(dict, b"Decode")
        .as_array()
        .and_then(|d| d.first())
        .and_then(|v| doc.resolve(v).as_f64())
        .is_some_and(|v| v == 1.0);
    if inverted { 0 } else { 1 }
}

/// An image XObject read for blanking.
pub struct Decoded {
    pub raster: Raster,
    /// The dictionary to write the samples with (no filters, the decoded
    /// size, colour space and depth).
    pub dict: Dictionary,
    /// A JPX image's own alpha (`SMaskInData`), as an 8-bit gray raster.
    pub alpha: Option<Raster>,
}

fn unsupported(id: ObjectId, why: &str) -> ImageError {
    ImageError::Unsupported(format!("image {} {why}", id.number()))
}

/// Reads image XObject `id`'s samples.
pub fn read(
    doc: &Document<'_>,
    view: Option<&ViewDocument>,
    id: ObjectId,
) -> Result<Decoded, ImageError> {
    let dict = doc
        .stream_dict(id)
        .map_err(|_| unsupported(id, "is not a stream"))?;
    if dict.get(b"F".as_slice()).is_some() {
        return Err(unsupported(id, "keeps its data in an external file"));
    }
    let int = |key: &[u8]| doc.dict_get(dict, key).as_i64().filter(|v| *v > 0);
    let (Some(width), Some(height)) = (int(b"Width"), int(b"Height")) else {
        return Err(unsupported(id, "has no size"));
    };
    let pixels = (width as u64).saturating_mul(height as u64);
    if pixels > MAX_IMAGE_PIXELS {
        return Err(ImageError::TooLarge(format!(
            "an image exceeds {MAX_IMAGE_PIXELS} pixels"
        )));
    }
    let mask = doc.dict_get(dict, b"ImageMask").as_bool() == Some(true);
    let space = dict.get(b"ColorSpace".as_slice());
    let declared_components = match mask {
        true => Some(1),
        false => space.and_then(|s| components(doc, s)),
    };
    let declared_bits = match mask {
        true => Some(1),
        false => int(b"BitsPerComponent").and_then(|b| u8::try_from(b).ok()),
    };
    let filters = filter_names(doc, dict.get(b"Filter".as_slice()));
    let codec = filters.iter().any(|f| is_codec(f));
    let (w, h) = (width as usize, height as usize);
    let mut out_dict = dict.clone();
    for key in [
        b"Filter".as_slice(),
        b"DecodeParms",
        b"Length",
        b"DL",
        b"Alternates",
        b"OPI",
        b"SMaskInData",
    ] {
        out_dict.remove(key);
    }
    let mut alpha = None;
    let raster = if !codec {
        let (Some(bits), Some(comps)) = (declared_bits, declared_components) else {
            return Err(unsupported(id, "has no depth or colour space"));
        };
        if !matches!(bits, 1 | 2 | 4 | 8 | 16) {
            return Err(unsupported(id, "has an invalid depth"));
        }
        let mut raster = Raster {
            data: Vec::new(),
            width: w,
            height: h,
            bits,
            components: comps,
        };
        check_bytes(&raster)?;
        let expected = raster.stride().saturating_mul(h);
        let room = expected.saturating_add(1 << 20);
        let limits = StreamLimits {
            max_decoded_bytes: room,
            max_total_decoded_bytes: room.saturating_mul(4),
            ..StreamLimits::default()
        };
        raster.data = match doc.decoded_stream(id, limits) {
            Ok(bytes) => bytes,
            Err(error) if over_limit(&error.kind) => {
                return Err(unsupported(id, "holds far more data than its size"));
            }
            Err(_) => view
                .and_then(|v| v.decoded_stream(id.number(), id.generation()))
                .ok_or_else(|| unsupported(id, "does not decode"))?,
        };
        raster.fit(0);
        raster
    } else {
        let view = view.ok_or_else(|| unsupported(id, "needs the view to decode"))?;
        let params = ImageParams {
            indexed: space.is_some_and(|s| indexed(doc, s)),
            bits: declared_bits,
            components: declared_components,
            width: u32::try_from(w).unwrap_or(u32::MAX),
            height: u32::try_from(h).unwrap_or(u32::MAX),
        };
        let samples = view
            .decoded_image(id.number(), id.generation(), params)
            .ok_or_else(|| unsupported(id, "does not decode"))?;
        let width = samples.width.map_or(w, |v| v as usize);
        let height = samples.height.map_or(h, |v| v as usize);
        if (width as u64).saturating_mul(height as u64) > MAX_IMAGE_PIXELS {
            return Err(ImageError::TooLarge(format!(
                "an image exceeds {MAX_IMAGE_PIXELS} pixels"
            )));
        }
        let bits = samples.bits.or(declared_bits).unwrap_or(8);
        let comps = declared_components
            .or(samples.components)
            .ok_or_else(|| unsupported(id, "has an unknown colour space"))?;
        if !matches!(bits, 1 | 2 | 4 | 8 | 16) {
            return Err(unsupported(id, "has an invalid depth"));
        }
        if space.is_none() && !mask {
            let name = match comps {
                1 => "DeviceGray",
                3 => "DeviceRGB",
                4 => "DeviceCMYK",
                _ => return Err(unsupported(id, "has an unknown colour space")),
            };
            out_dict.insert(b"ColorSpace".to_vec(), Object::name(name));
        }
        let mut raster = Raster {
            data: samples.data,
            width,
            height,
            bits,
            components: comps,
        };
        check_bytes(&raster)?;
        raster.fit(0);
        // A one-bit image the codec gave as bytes goes back to one bit.
        if declared_bits == Some(1) && bits == 8 && comps == 1 {
            raster = pack_bits(&raster);
        }
        if let Some(data) = samples.alpha.filter(|_| {
            doc.dict_get(dict, b"SMaskInData")
                .as_i64()
                .is_some_and(|v| v > 0)
        }) {
            let mut a = Raster {
                data,
                width,
                height,
                bits,
                components: 1,
            };
            check_bytes(&a)?;
            a.fit(255);
            alpha = Some(a);
        }
        out_dict.insert(b"Width".to_vec(), Object::Integer(width as i64));
        out_dict.insert(b"Height".to_vec(), Object::Integer(height as i64));
        raster
    };
    out_dict.insert(
        b"BitsPerComponent".to_vec(),
        Object::Integer(i64::from(raster.bits)),
    );
    Ok(Decoded {
        raster,
        dict: out_dict,
        alpha,
    })
}

fn over_limit(kind: &StreamObjectErrorKind) -> bool {
    matches!(kind, StreamObjectErrorKind::Decode(error)
        if matches!(error.kind, StreamErrorKind::DecodedOutputLimit { .. }
            | StreamErrorKind::TotalOutputLimit { .. }))
}

/// One-byte gray samples (0 or 255) packed to one bit (128 and up is 1).
fn pack_bits(raster: &Raster) -> Raster {
    let mut out = Raster {
        data: Vec::new(),
        width: raster.width,
        height: raster.height,
        bits: 1,
        components: 1,
    };
    let stride = out.stride();
    out.data = vec![0; stride * raster.height];
    for y in 0..raster.height {
        for x in 0..raster.width {
            if raster
                .data
                .get(y * raster.width + x)
                .is_some_and(|v| *v >= 128)
            {
                if let Some(b) = out.data.get_mut(y * stride + x / 8) {
                    *b |= 0x80 >> (x % 8);
                }
            }
        }
    }
    out
}

/// Flate-compressed samples and the dictionary entries that say so.
pub fn encode(raster: &Raster, dict: &mut Dictionary) -> Vec<u8> {
    dict.insert(b"Filter".to_vec(), Object::name("FlateDecode"));
    dict.remove(b"DecodeParms".as_slice());
    miniz_oxide::deflate::compress_to_vec_zlib(&raster.data, 6)
}

/// The one-pixel blank image an image wholly under marks becomes.
pub fn single_pixel(raster: &Raster, value: u16) -> Raster {
    let mut one = Raster {
        data: Vec::new(),
        width: 1,
        height: 1,
        bits: raster.bits,
        components: raster.components,
    };
    one.data = vec![0; one.stride()];
    one.set(0, 0, 1, value);
    one
}

/// An inline image: its dictionary entries (abbreviations or full names).
pub struct Inline<'a> {
    pub entries: Vec<(Vec<u8>, InlineValue<'a>)>,
}

/// An inline image dictionary value, with its source bytes.
#[derive(Clone, Debug)]
pub struct InlineValue<'a> {
    pub source: &'a [u8],
    pub value: Object,
}

impl Inline<'_> {
    fn get(&self, long: &[u8], short: &[u8]) -> Option<&Object> {
        self.entries
            .iter()
            .find(|(k, _)| k == long || k == short)
            .map(|(_, v)| &v.value)
    }
}

/// An inline image's filters as the strict decoder takes them; None for a
/// filter it does not have (DCT, CCITT).
fn inline_filters(image: &Inline<'_>) -> Option<Vec<FilterSpec>> {
    let names: Vec<Vec<u8>> = match image.get(b"Filter", b"F") {
        None => Vec::new(),
        Some(Object::Name(n)) => vec![n.clone()],
        Some(Object::Array(items)) => items
            .iter()
            .filter_map(|i| i.as_name().map(<[u8]>::to_vec))
            .collect(),
        Some(_) => return None,
    };
    let parms: Vec<Option<&Dictionary>> = match image.get(b"DecodeParms", b"DP") {
        Some(Object::Dictionary(d)) => vec![Some(d)],
        Some(Object::Array(items)) => items.iter().map(Object::as_dict).collect(),
        _ => Vec::new(),
    };
    names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let parm = parms.get(i).copied().flatten();
            let int = |key: &[u8], default: i64| {
                parm.and_then(|p| p.get(key))
                    .and_then(Object::as_i64)
                    .unwrap_or(default)
            };
            let filter = match name.as_slice() {
                b"FlateDecode" | b"Fl" => Filter::Flate,
                b"ASCIIHexDecode" | b"AHx" => Filter::AsciiHex,
                b"ASCII85Decode" | b"A85" => Filter::Ascii85,
                b"RunLengthDecode" | b"RL" => Filter::RunLength,
                b"LZWDecode" | b"LZW" => Filter::Lzw {
                    early_change: int(b"EarlyChange", 1) != 0,
                },
                _ => return None,
            };
            let predictor = PredictorParams {
                predictor: u8::try_from(int(b"Predictor", 1)).ok()?,
                colors: usize::try_from(int(b"Colors", 1)).ok()?,
                bits_per_component: u8::try_from(int(b"BitsPerComponent", 8)).ok()?,
                columns: usize::try_from(int(b"Columns", 1)).ok()?,
            };
            Some(FilterSpec { filter, predictor })
        })
        .collect()
}

/// An inline image's samples; `space` gives the components of a colour
/// space it names from the resources.
pub fn read_inline(
    image: &Inline<'_>,
    data: &[u8],
    named: impl Fn(&[u8]) -> Option<u8>,
) -> Result<(Raster, bool), ImageError> {
    let bad = |why: &str| ImageError::Unsupported(format!("an inline image {why}"));
    let int = |long: &[u8], short: &[u8]| {
        image
            .get(long, short)
            .and_then(Object::as_i64)
            .filter(|v| *v > 0)
    };
    let (Some(width), Some(height)) = (int(b"Width", b"W"), int(b"Height", b"H")) else {
        return Err(bad("has no size"));
    };
    if (width as u64).saturating_mul(height as u64) > MAX_IMAGE_PIXELS {
        return Err(ImageError::TooLarge(format!(
            "an image exceeds {MAX_IMAGE_PIXELS} pixels"
        )));
    }
    let mask = image.get(b"ImageMask", b"IM").and_then(Object::as_bool) == Some(true);
    let comps = match (mask, image.get(b"ColorSpace", b"CS")) {
        (true, _) => Some(1),
        (false, Some(Object::Name(n))) => match n.as_slice() {
            b"G" | b"DeviceGray" | b"CalGray" | b"I" | b"Indexed" => Some(1),
            b"RGB" | b"DeviceRGB" | b"CalRGB" => Some(3),
            b"CMYK" | b"DeviceCMYK" => Some(4),
            other => named(other),
        },
        (false, Some(Object::Array(items))) => match items.first().and_then(Object::as_name) {
            Some(b"I" | b"Indexed") => Some(1),
            _ => None,
        },
        _ => None,
    };
    let bits = match mask {
        true => Some(1),
        false => int(b"BitsPerComponent", b"BPC").and_then(|b| u8::try_from(b).ok()),
    };
    let (Some(comps), Some(bits)) = (comps, bits) else {
        return Err(bad("has no depth or colour space"));
    };
    if !matches!(bits, 1 | 2 | 4 | 8 | 16) {
        return Err(bad("has an invalid depth"));
    }
    let filters =
        inline_filters(image).ok_or_else(|| bad("uses a filter this version cannot decode"))?;
    let mut raster = Raster {
        data: Vec::new(),
        width: width as usize,
        height: height as usize,
        bits,
        components: comps,
    };
    check_bytes(&raster)?;
    let room = raster
        .stride()
        .saturating_mul(raster.height)
        .saturating_add(1 << 20);
    let limits = StreamLimits {
        max_encoded_bytes: data.len().max(1),
        max_decoded_bytes: room,
        max_total_decoded_bytes: room.saturating_mul(4),
        ..StreamLimits::default()
    };
    // Data ends where `EI` begins, white space and all: binary data may end
    // in a byte that reads as white space, so the whole is tried first.
    let trimmed = data
        .iter()
        .rposition(|b| !matches!(b, 0 | 9 | 10 | 12 | 13 | 32))
        .map_or(&[][..], |end| data.get(..=end).unwrap_or(&[]));
    raster.data = stream::decode_filter_specs(data, &filters, limits)
        .or_else(|error| match filters.is_empty() {
            true => Err(error),
            false => stream::decode_filter_specs(trimmed, &filters, limits),
        })
        .map_err(|e| ImageError::Unsupported(format!("an inline image does not decode ({e})")))?;
    raster.fit(0);
    Ok((raster, mask))
}

/// `BI ... ID ... EI` for blanked samples: the image's own entries but its
/// filters and length, then Flate and hex data.
pub fn write_inline(image: &Inline<'_>, raster: &Raster) -> Vec<u8> {
    let mut out = b"BI".to_vec();
    for (key, value) in &image.entries {
        if matches!(
            key.as_slice(),
            b"Filter" | b"F" | b"DecodeParms" | b"DP" | b"Length" | b"L"
        ) {
            continue;
        }
        out.push(b' ');
        out.extend_from_slice(crate::app::ops::edit::plan::name_source(key).as_bytes());
        out.push(b' ');
        out.extend_from_slice(value.source);
    }
    out.extend_from_slice(b" /F [/AHx /Fl] ID\n");
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&raster.data, 6);
    for chunk in compressed.chunks(64) {
        for b in chunk {
            out.extend_from_slice(format!("{b:02x}").as_bytes());
        }
        out.push(b'\n');
    }
    out.extend_from_slice(b">\nEI");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raster(width: usize, height: usize, bits: u8, components: u8) -> Raster {
        let mut r = Raster {
            data: Vec::new(),
            width,
            height,
            bits,
            components,
        };
        r.data = vec![0xff; r.stride() * height];
        r
    }

    #[test]
    fn pixels_a_region_overlaps_are_blanked() {
        let mut r = raster(10, 10, 8, 3);
        let unit =
            unit_regions(&[[2.0, 3.0, 4.5, 5.0]], &[10.0, 0.0, 0.0, 10.0, 0.0, 0.0]).unwrap();
        let regions = pixel_regions(&unit, 10, 10);
        // Unit square y up: page y 3..5 is rows 5..7 from the top.
        assert_eq!(blank(&mut r, &regions, 0).unwrap(), 6);
        let at = |x: usize, y: usize| r.data[(y * 10 + x) * 3];
        assert_eq!(
            (at(2, 5), at(4, 6), at(5, 5), at(2, 4), at(2, 7)),
            (0, 0, 255, 255, 255)
        );
    }

    #[test]
    fn packed_samples_keep_their_neighbours() {
        let mut r = raster(10, 1, 1, 1);
        r.set(0, 3, 5, 0);
        assert_eq!(r.data, vec![0b1110_0111, 0xff]);
        let mut r = raster(3, 1, 4, 1);
        r.set(0, 1, 2, 0x5);
        assert_eq!(r.data, vec![0xf5, 0xff]);
        let mut r = raster(2, 1, 16, 1);
        r.set(0, 1, 2, 0x1234);
        assert_eq!(r.data, vec![0xff, 0xff, 0x12, 0x34]);
    }
}
