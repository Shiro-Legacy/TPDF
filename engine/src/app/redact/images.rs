// SPDX-License-Identifier: MIT
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
/// Pixels one apply decodes in all (images, masks, inline images).
pub const MAX_TOTAL_PIXELS: u64 = 400_000_000;
/// Rows of regions an image's blanking may visit in all.
const MAX_ROW_WORK: usize = 50_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// Past `MAX_IMAGE_PIXELS`.
    Pixels,
    /// Past `MAX_TOTAL_PIXELS` or a work bound.
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
    /// Bytes a row takes (saturating: `check_bytes` refuses such sizes).
    pub fn stride(&self) -> usize {
        let bits = (self.width as u64)
            .saturating_mul(u64::from(self.components))
            .saturating_mul(u64::from(self.bits));
        usize::try_from(bits.div_ceil(8)).unwrap_or(usize::MAX)
    }

    /// Fits the data to the declared size: short data padded with `fill`,
    /// long data cut.
    fn fit(&mut self, fill: u8) {
        let len = self.stride().saturating_mul(self.height);
        self.data.resize(len, fill);
    }

    /// Sets pixels `x0..x1` of row `y` to `values`, one per component (a
    /// single value sets every component).
    fn set_pixels(&mut self, y: usize, x0: usize, x1: usize, values: &[u16]) {
        let n = usize::from(self.components);
        if values.len() <= 1 || n <= 1 {
            self.set(y, x0, x1, values.first().copied().unwrap_or(0));
            return;
        }
        let stride = self.stride();
        let bits = usize::from(self.bits);
        let Some(row) = self.data.get_mut(y * stride..(y + 1) * stride) else {
            return;
        };
        let mask = if bits >= 16 {
            u16::MAX
        } else {
            (1u16 << bits) - 1
        };
        for x in x0..x1 {
            for c in 0..n {
                let value = values.get(c).copied().unwrap_or(0) & mask;
                let bit = (x * n + c) * bits;
                if bits >= 8 {
                    let bytes = value.to_be_bytes();
                    let pattern = if bits == 16 { &bytes[..] } else { &bytes[1..] };
                    for (k, p) in pattern.iter().enumerate() {
                        if let Some(b) = row.get_mut(bit / 8 + k) {
                            *b = *p;
                        }
                    }
                } else {
                    let (byte, shift) = (bit / 8, 8 - bits - bit % 8);
                    if let Some(b) = row.get_mut(byte) {
                        let m = (mask << shift) as u8;
                        *b = (*b & !m) | (((value << shift) as u8) & m);
                    }
                }
            }
        }
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
                    for (b, p) in chunk.iter_mut().zip(pattern) {
                        *b = *p;
                    }
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

/// Blanks every pixel a region overlaps, even by a sliver (rows and
/// columns rounded outward), with `values` (one per component, or one for
/// all). Returns the pixels blanked (each once).
pub fn blank(
    raster: &mut Raster,
    regions: &[[Point; 4]],
    values: &[u16],
) -> Result<u64, ImageError> {
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
                        raster.set_pixels(y, c, d, values);
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

/// Takes an image's declared pixels from what one apply may decode;
/// refuses past `MAX_IMAGE_PIXELS` or the pixels left.
fn charge(pixels: u64, room: &mut u64) -> Result<(), ImageError> {
    if pixels > MAX_IMAGE_PIXELS {
        return Err(ImageError::Pixels);
    }
    *room = room.checked_sub(pixels).ok_or_else(|| {
        ImageError::TooLarge(format!(
            "the images under the marks exceed {MAX_TOTAL_PIXELS} pixels in all"
        ))
    })?;
    Ok(())
}

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

/// Palette entries (RGB, 0 to 1) at most this bright in every channel
/// stand for the black box in an Indexed image.
const DARK_ENOUGH: f64 = 0.15;

/// What blanking writes into an image (R2): the black of the box in the
/// image's own colour space.
#[derive(Clone, Debug, PartialEq)]
pub enum Black {
    /// These samples, one per component (or one for all).
    Samples(Vec<u16>),
    /// An Indexed image whose palette has no dark entry: black is added to
    /// the palette as entry `index` (`lookup` is the new table, in the base
    /// space), the samples widened to 8 bits first when `widen`.
    Extend {
        lookup: Vec<u8>,
        index: u16,
        widen: bool,
    },
    /// An Indexed image with a full 8-bit palette and no dark entry: the
    /// image is rewritten as 8-bit DeviceRGB through `palette` (RGB bytes).
    Rgb { palette: Vec<[u8; 3]> },
}

/// A colour space's family name and its array (when it is one).
fn family<'d>(doc: &'d Document<'_>, space: &'d Object) -> (Vec<u8>, &'d [Object]) {
    let space = doc.resolve(space);
    match space.as_array() {
        Some(items) => (
            (items.first().map(|f| doc.resolve(f)))
                .and_then(Object::as_name)
                .unwrap_or(b"")
                .to_vec(),
            items,
        ),
        None => (space.as_name().unwrap_or(b"").to_vec(), &[]),
    }
}

/// The colour a box is drawn in (black), in the colour space of family
/// `name` with `n` components, as component values (before /Decode).
fn black_in(doc: &Document<'_>, name: &[u8], items: &[Object], n: usize) -> Vec<f64> {
    match name {
        b"DeviceCMYK" | b"CMYK" => vec![0.0, 0.0, 0.0, 1.0],
        b"ICCBased" if n == 4 => vec![0.0, 0.0, 0.0, 1.0],
        // Full colorant: the darkest a separation or DeviceN can paint
        // without evaluating its tint transform.
        b"Separation" | b"DeviceN" => vec![1.0; n],
        b"Lab" => {
            // L 0, a and b 0 (or the nearest the range allows).
            let range: Vec<f64> = (items.get(1).map(|d| doc.resolve(d)))
                .and_then(|d| {
                    doc.dict_get(d.as_dict()?, b"Range")
                        .as_array()
                        .map(<[Object]>::to_vec)
                })
                .map(|r| r.iter().filter_map(|v| doc.resolve(v).as_f64()).collect())
                .unwrap_or_else(|| vec![-100.0, 100.0, -100.0, 100.0]);
            let mid = |i: usize| {
                let (lo, hi) = (
                    range.get(i).copied().unwrap_or(-100.0),
                    range.get(i + 1).copied().unwrap_or(100.0),
                );
                0.0f64.max(lo).min(hi)
            };
            vec![0.0, mid(0), mid(2)]
        }
        _ => vec![0.0; n],
    }
}

/// The /Decode range of component `i` of an image (`max` its largest
/// sample), with the colour space's default.
fn decode_range(
    doc: &Document<'_>,
    dict: &Dictionary,
    name: &[u8],
    items: &[Object],
    i: usize,
    max: f64,
) -> (f64, f64) {
    let given = (doc.dict_get(dict, b"Decode").as_array()).map(|d| {
        d.iter()
            .filter_map(|v| doc.resolve(v).as_f64())
            .collect::<Vec<_>>()
    });
    if let Some(d) = given {
        if let (Some(&lo), Some(&hi)) = (d.get(2 * i), d.get(2 * i + 1)) {
            return (lo, hi);
        }
    }
    match name {
        b"Indexed" | b"I" => (0.0, max),
        b"Lab" if i > 0 => {
            let range: Vec<f64> = (items.get(1).map(|d| doc.resolve(d)))
                .and_then(|d| {
                    doc.dict_get(d.as_dict()?, b"Range")
                        .as_array()
                        .map(<[Object]>::to_vec)
                })
                .map(|r| r.iter().filter_map(|v| doc.resolve(v).as_f64()).collect())
                .unwrap_or_else(|| vec![-100.0, 100.0, -100.0, 100.0]);
            (
                range.get(2 * (i - 1)).copied().unwrap_or(-100.0),
                range.get(2 * (i - 1) + 1).copied().unwrap_or(100.0),
            )
        }
        b"Lab" => (0.0, 100.0),
        _ => (0.0, 1.0),
    }
}

/// The sample that decodes to `value` through `(lo, hi)`, the nearest one.
fn sample_for(value: f64, (lo, hi): (f64, f64), max: f64) -> u16 {
    if (hi - lo).abs() < f64::EPSILON || !value.is_finite() {
        return 0;
    }
    let s = ((value - lo) / (hi - lo) * max).round();
    s.clamp(0.0, max) as u16
}

/// An Indexed colour space's base family, base components, top index,
/// lookup table and (a Lab base) its /Range.
type Palette = (Vec<u8>, usize, usize, Vec<u8>, Vec<f64>);

/// An Indexed colour space's base family, base components, top index and
/// lookup table.
fn palette(doc: &Document<'_>, items: &[Object]) -> Option<Palette> {
    let base = items.get(1)?;
    let (base_name, base_items) = family(doc, base);
    let n = usize::from(components(doc, base)?);
    // A Lab base's /Range (a and b).
    let range: Vec<f64> = (base_items.get(1).map(|d| doc.resolve(d)))
        .and_then(|d| {
            doc.dict_get(d.as_dict()?, b"Range")
                .as_array()
                .map(<[Object]>::to_vec)
        })
        .map(|r| r.iter().filter_map(|v| doc.resolve(v).as_f64()).collect())
        .unwrap_or_default();
    let hival = usize::try_from(doc.resolve(items.get(2)?).as_i64()?)
        .ok()?
        .min(255);
    let lookup = match doc.resolve(items.get(3)?) {
        Object::String(bytes) => bytes.clone(),
        _ => {
            let id = items.get(3)?.as_reference()?;
            doc.decoded_stream(id, StreamLimits::default()).ok()?
        }
    };
    Some((base_name, n, hival, lookup, range))
}

/// A palette entry as RGB (0 to 1), from its base space's bytes.
fn entry_rgb(base: &[u8], entry: &[u8]) -> [f64; 3] {
    let v = |i: usize| f64::from(entry.get(i).copied().unwrap_or(0)) / 255.0;
    match (base, entry.len()) {
        (b"DeviceCMYK" | b"CMYK", _) | (b"ICCBased", 4) => {
            let k = v(3);
            [
                (1.0 - v(0)) * (1.0 - k),
                (1.0 - v(1)) * (1.0 - k),
                (1.0 - v(2)) * (1.0 - k),
            ]
        }
        (b"DeviceRGB" | b"RGB" | b"CalRGB", _) | (b"ICCBased", 3) => [v(0), v(1), v(2)],
        (b"Lab", _) => [v(0); 3],
        (b"DeviceGray" | b"G" | b"CalGray", _) | (b"ICCBased", 1) => [v(0); 3],
        // Separation, DeviceN: more colorant is darker.
        _ => {
            let ink = (0..entry.len()).map(v).fold(0.0, f64::max);
            [1.0 - ink; 3]
        }
    }
}

/// The largest sample of a depth.
fn max_sample(bits: u8) -> u16 {
    if bits >= 16 {
        u16::MAX
    } else {
        (1u16 << bits) - 1
    }
}

/// Black as a palette entry in base family `base` with `n` components
/// (`range`: a Lab base's /Range for a and b); None for a base whose black
/// cannot be written as bytes simply.
fn black_entry(base: &[u8], n: usize, range: &[f64]) -> Option<Vec<u8>> {
    match (base, n) {
        (b"DeviceCMYK" | b"CMYK", 4) | (b"ICCBased", 4) => Some(vec![0, 0, 0, 255]),
        (b"DeviceRGB" | b"RGB" | b"CalRGB", 3) | (b"ICCBased", 3) => Some(vec![0, 0, 0]),
        (b"DeviceGray" | b"G" | b"CalGray", 1) | (b"ICCBased", 1) => Some(vec![0]),
        // L 0; a and b 0 (or the nearest the range allows) as bytes over it.
        (b"Lab", 3) => {
            let byte = |lo: f64, hi: f64| {
                let v = 0.0f64.max(lo).min(hi);
                if hi > lo {
                    ((v - lo) / (hi - lo) * 255.0).round().clamp(0.0, 255.0) as u8
                } else {
                    0
                }
            };
            let r = |i: usize, d: f64| range.get(i).copied().unwrap_or(d);
            Some(vec![
                0,
                byte(r(0, -100.0), r(1, 100.0)),
                byte(r(2, -100.0), r(3, 100.0)),
            ])
        }
        // Full colorant.
        (b"Separation" | b"DeviceN", n) if n > 0 => Some(vec![255; n]),
        _ => None,
    }
}

/// Whether an image over base family `base` may be rewritten as RGB (a
/// spot or Lab base would lose its colours: it keeps its darkest entry).
fn rgb_allowed(base: &[u8]) -> bool {
    matches!(
        base,
        b"DeviceRGB"
            | b"RGB"
            | b"CalRGB"
            | b"DeviceGray"
            | b"G"
            | b"CalGray"
            | b"DeviceCMYK"
            | b"CMYK"
            | b"ICCBased"
    )
}

/// R2: what blanking writes into an image with dictionary `dict` (its
/// decoded form: no filters) and `raster`'s depth and components: the box's
/// black in its colour space, through its /Decode; a stencil mask's "no
/// paint". `palette_edits` allows adding black to an Indexed palette or
/// rewriting the image as RGB (not for inline images, which keep their
/// dictionary: they take the darkest entry).
pub fn black(
    doc: &Document<'_>,
    dict: &Dictionary,
    raster: &Raster,
    stencil: bool,
    palette_edits: bool,
) -> Black {
    if stencil {
        return Black::Samples(vec![blank_value(doc, dict, true)]);
    }
    let max = f64::from(max_sample(raster.bits));
    let n = usize::from(raster.components);
    let Some(space) = dict.get(b"ColorSpace".as_slice()) else {
        return Black::Samples(vec![0]);
    };
    let (name, items) = family(doc, space);
    if matches!(name.as_slice(), b"Indexed" | b"I") {
        let Some((base, base_n, hival, lookup, base_range)) = palette(doc, items) else {
            return Black::Samples(vec![0]);
        };
        let range = decode_range(doc, dict, &name, items, 0, max);
        let entries: Vec<[f64; 3]> = (0..=hival)
            .map(|i| {
                entry_rgb(
                    &base,
                    lookup.get(i * base_n..(i + 1) * base_n).unwrap_or(&[]),
                )
            })
            .collect();
        let darkest = (entries.iter().enumerate())
            .min_by(|a, b| {
                let m = |c: &[f64; 3]| c.iter().copied().fold(0.0, f64::max);
                m(a.1).total_cmp(&m(b.1))
            })
            .map_or(0, |(i, _)| i);
        let dark = entries
            .get(darkest)
            .is_some_and(|c| c.iter().all(|v| *v <= DARK_ENOUGH));
        let take = |i: usize| Black::Samples(vec![sample_for(i as f64, range, max)]);
        if dark || !palette_edits {
            return take(darkest);
        }
        let mut table = lookup
            .get(..(hival + 1) * base_n)
            .unwrap_or(&lookup)
            .to_vec();
        table.resize((hival + 1) * base_n, 0);
        let room = (max as usize) > hival;
        let color_key = dict
            .get(b"Mask".as_slice())
            .is_some_and(|m| doc.resolve(m).as_array().is_some());
        match black_entry(&base, base_n, &base_range) {
            Some(entry) if hival < 255 => {
                table.extend(entry);
                return Black::Extend {
                    lookup: table,
                    index: (hival + 1) as u16,
                    widen: !room,
                };
            }
            _ => {}
        }
        if color_key || !rgb_allowed(&base) {
            // A colour-key mask names palette indices; a spot or Lab base
            // would lose its colours as RGB: the darkest entry.
            return take(darkest);
        }
        let palette = entries
            .iter()
            .map(|c| c.map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8))
            .collect();
        return Black::Rgb { palette };
    }
    let colour = black_in(doc, &name, items, n);
    let samples = (0..n)
        .map(|i| {
            let range = decode_range(doc, dict, &name, items, i, max);
            sample_for(colour.get(i).copied().unwrap_or(0.0), range, max)
        })
        .collect();
    Black::Samples(samples)
}

/// Readies `raster` and `dict` for `black` and returns the samples to
/// blank with: a palette gains its black entry (the samples widened to 8
/// bits when they had no room), or the image becomes 8-bit DeviceRGB.
pub fn prepare(
    doc: &Document<'_>,
    raster: &mut Raster,
    dict: &mut Dictionary,
    plan: Black,
) -> Result<Vec<u16>, ImageError> {
    match plan {
        Black::Samples(samples) => Ok(samples),
        Black::Extend {
            lookup,
            index,
            widen,
        } => {
            let space = dict
                .get(b"ColorSpace".as_slice())
                .map(|s| doc.resolve(s).clone());
            let Some(Object::Array(mut items)) = space else {
                return Ok(vec![0]);
            };
            if items.len() < 4 {
                return Ok(vec![0]);
            }
            items[2] = Object::Integer(i64::from(index));
            items[3] = Object::String(lookup);
            dict.insert(b"ColorSpace".to_vec(), Object::Array(items));
            let max = f64::from(max_sample(raster.bits));
            let decode = (doc.dict_get(dict, b"Decode").as_array()).map(|d| {
                d.iter()
                    .filter_map(|v| doc.resolve(v).as_f64())
                    .collect::<Vec<_>>()
            });
            if widen {
                *raster = widened(raster, decode.as_deref(), max)?;
                dict.insert(b"BitsPerComponent".to_vec(), Object::Integer(8));
                dict.remove(b"Decode".as_slice());
                return Ok(vec![index]);
            }
            let range = match decode.as_deref() {
                Some([lo, hi, ..]) => (*lo, *hi),
                _ => (0.0, max),
            };
            Ok(vec![sample_for(f64::from(index), range, max)])
        }
        Black::Rgb { palette } => {
            let max = f64::from(max_sample(raster.bits));
            let decode = (doc.dict_get(dict, b"Decode").as_array()).map(|d| {
                d.iter()
                    .filter_map(|v| doc.resolve(v).as_f64())
                    .collect::<Vec<_>>()
            });
            let indices = widened(raster, decode.as_deref(), max)?;
            let mut rgb = Raster {
                data: Vec::with_capacity(indices.data.len().saturating_mul(3)),
                width: raster.width,
                height: raster.height,
                bits: 8,
                components: 3,
            };
            check_bytes(&rgb)?;
            for &i in &indices.data {
                let c = palette
                    .get(usize::from(i))
                    .or(palette.last())
                    .copied()
                    .unwrap_or([0; 3]);
                rgb.data.extend_from_slice(&c);
            }
            *raster = rgb;
            dict.insert(b"ColorSpace".to_vec(), Object::name("DeviceRGB"));
            dict.insert(b"BitsPerComponent".to_vec(), Object::Integer(8));
            dict.remove(b"Decode".as_slice());
            Ok(vec![0, 0, 0])
        }
    }
}

/// An Indexed raster's samples as 8-bit indices (through its /Decode).
fn widened(raster: &Raster, decode: Option<&[f64]>, max: f64) -> Result<Raster, ImageError> {
    let mut out = Raster {
        data: Vec::new(),
        width: raster.width,
        height: raster.height,
        bits: 8,
        components: 1,
    };
    check_bytes(&out)?;
    out.data.reserve(raster.width.saturating_mul(raster.height));
    let bits = usize::from(raster.bits);
    let stride = raster.stride();
    let (lo, hi) = match decode {
        Some([lo, hi, ..]) => (*lo, *hi),
        _ => (0.0, max),
    };
    for y in 0..raster.height {
        let row = raster.data.get(y * stride..(y + 1) * stride).unwrap_or(&[]);
        for x in 0..raster.width {
            let s = match bits {
                16 => {
                    let at = x * 2;
                    u16::from_be_bytes([
                        row.get(at).copied().unwrap_or(0),
                        row.get(at + 1).copied().unwrap_or(0),
                    ])
                }
                8 => u16::from(row.get(x).copied().unwrap_or(0)),
                _ => {
                    let bit = x * bits;
                    let byte = row.get(bit / 8).copied().unwrap_or(0);
                    u16::from((byte >> (8 - bits - bit % 8)) & ((1u8 << bits) - 1))
                }
            };
            let index = lo + f64::from(s) * (hi - lo) / max.max(1.0);
            out.data.push(index.round().clamp(0.0, 255.0) as u8);
        }
    }
    Ok(out)
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

/// Reads image XObject `id`'s samples, its pixels taken from `room`; the
/// view decodes it as object `origin` (a copy's original, R3; else `id`).
pub fn read(
    doc: &Document<'_>,
    view: Option<&ViewDocument>,
    id: ObjectId,
    origin: ObjectId,
    room: &mut u64,
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
    charge((width as u64).saturating_mul(height as u64), room)?;
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
        // Derived copies of the samples (XMP thumbnails, application data).
        b"Metadata",
        b"PieceInfo",
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
                .and_then(|v| v.decoded_stream(origin.number(), origin.generation()))
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
            .decoded_image(origin.number(), origin.generation(), params)
            .ok_or_else(|| unsupported(id, "does not decode"))?;
        let width = samples.width.map_or(w, |v| v as usize);
        let height = samples.height.map_or(h, |v| v as usize);
        // A codec's own size past the declared one is charged too.
        let more = ((width as u64).saturating_mul(height as u64))
            .saturating_sub((w as u64).saturating_mul(h as u64));
        if (width as u64).saturating_mul(height as u64) > MAX_IMAGE_PIXELS {
            return Err(ImageError::Pixels);
        }
        charge(more, room)?;
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
pub fn single_pixel(raster: &Raster, values: &[u16]) -> Raster {
    let mut one = Raster {
        data: Vec::new(),
        width: 1,
        height: 1,
        bits: raster.bits,
        components: raster.components,
    };
    one.data = vec![0; one.stride()];
    one.set_pixels(0, 0, 1, values);
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
    room: &mut u64,
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
    charge((width as u64).saturating_mul(height as u64), room)?;
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
        assert_eq!(blank(&mut r, &regions, &[0]).unwrap(), 6);
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
