// SPDX-License-Identifier: AGPL-3.0-or-later
//! `pdf-from-images`: one page per PNG or JPEG (engine.ts `pdfFromImages`).
//!
//! A JPEG is embedded as it is (DCTDecode) after reading its headers: size
//! and components from the frame header, an inverted
//! /Decode for Adobe CMYK, the resolution from EXIF, Photoshop or JFIF data,
//! an embedded ICC profile. A PNG is decoded, and its samples (and alpha, as
//! a soft mask) stored Flate-compressed. The page is the image at its
//! resolution, 96 dpi counting as none (72 then), shrunk to 14400 points at
//! most. The EXIF orientation is applied.
//!
//! Each image is read within a budget of stored bytes (its data, mask and
//! ICC profile as written), so a document that cannot be saved stops early
//! instead of holding every image first.
use std::io::Cursor;

use miniz_oxide::deflate::core::{
    CompressorOxide, TDEFLFlush, TDEFLStatus, compress_to_output, create_comp_flags_from_zip_params,
};

use crate::app::edit::{Builder, Edit};
use crate::object::{Dictionary, Object, ObjectId};

#[path = "resolution.rs"]
mod resolution;

/// engine-core.ts `MAX_IMAGE_PAGE_POINTS`.
const MAX_PAGE_POINTS: f64 = 14_400.0;
/// Decoded PNG samples (and the decoder's buffers) one image may use.
const MAX_DECODED_BYTES: usize = 256 * 1024 * 1024;
/// A private key of an image whose placement `cm` carries the image's EXIF
/// orientation (2..8), so a replace of the image (edit-object) takes that
/// turn back out instead of keeping it as the user's.
pub const TURNED_KEY: &[u8] = b"TPDF_Orientation";

/// An image ready to place: its XObject parts and how it is drawn.
pub struct Placed {
    pub width: u32,
    pub height: u32,
    /// Pixels per inch along the image's x axis; 0 when it declares none.
    pub dpi: u32,
    /// EXIF orientation 1..8 (1 when none).
    pub orientation: u8,
    /// A JPEG whose frame is arithmetic coded (SOF9, SOF10), which several
    /// viewers do not decode.
    pub arithmetic: bool,
    image: Dictionary,
    data: Vec<u8>,
    /// Gray samples of the alpha channel, Flate-compressed.
    mask: Option<Vec<u8>>,
    /// ICC profile, Flate-compressed, and its component count.
    icc: Option<(Vec<u8>, u8)>,
}

/// Why an image is not placed.
#[derive(Debug, PartialEq, Eq)]
pub enum ImageError {
    /// Not a PNG or JPEG the app can read; the text says why.
    Unreadable(String),
    /// What it stores would pass the budget it was read with, or a PNG's
    /// samples would pass the decoding budget (256 MiB).
    TooLarge,
}

impl From<String> for ImageError {
    fn from(reason: String) -> Self {
        Self::Unreadable(reason)
    }
}

impl From<&str> for ImageError {
    fn from(reason: &str) -> Self {
        Self::Unreadable(reason.into())
    }
}

/// Reads one image, storing at most `budget` bytes for it.
pub fn read(bytes: &[u8], budget: usize) -> Result<Placed, ImageError> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        png(bytes, budget)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        jpeg(bytes, budget)
    } else {
        Err("not a PNG or JPEG file".into())
    }
}

impl Placed {
    /// The bytes its streams store.
    pub fn stored(&self) -> usize {
        let mask = self.mask.as_ref().map_or(0, Vec::len);
        let icc = self.icc.as_ref().map_or(0, |(icc, _)| icc.len());
        self.data.len().saturating_add(mask).saturating_add(icc)
    }
}

fn png(bytes: &[u8], budget: usize) -> Result<Placed, ImageError> {
    let limits = png::Limits {
        bytes: MAX_DECODED_BYTES,
    };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), limits);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    // A chunk CRC that does not match is read through (the zlib checksum is
    // already not checked); a stream that ends early is still refused.
    decoder.ignore_checksums(true);
    let mut reader = decoder.read_info().map_err(png_error)?;
    let (width, height, dpi, icc) = {
        let info = reader.info();
        let dpi = png_resolution(info);
        (
            info.width,
            info.height,
            dpi,
            info.icc_profile.as_ref().map(|icc| icc.to_vec()),
        )
    };
    // A valid image past the decoding budget is too large, not unreadable.
    let size = reader
        .output_buffer_size()
        .filter(|&size| size <= MAX_DECODED_BYTES)
        .ok_or(ImageError::TooLarge)?;
    let mut samples = zeroed(size)?;
    let frame = reader.next_frame(&mut samples).map_err(png_error)?;
    samples.truncate(frame.buffer_size());
    let (colors, alpha) = match frame.color_type {
        png::ColorType::Grayscale => (1, false),
        png::ColorType::GrayscaleAlpha => (1, true),
        png::ColorType::Rgb => (3, false),
        png::ColorType::Rgba => (3, true),
        png::ColorType::Indexed => return Err("an indexed image was not expanded".into()),
    };
    if frame.bit_depth != png::BitDepth::Eight {
        return Err("the image did not decode to 8 bits".into());
    }
    let (color, mask) = if alpha {
        // Colour moves to the front of the buffer, alpha into its own.
        let channels = colors + 1;
        let pixels = samples.len() / channels;
        let mut mask = zeroed(pixels)?;
        for (pixel, a) in (0..pixels).zip(mask.iter_mut()) {
            let from = pixel * channels;
            samples.copy_within(from..from + colors, pixel * colors);
            *a = samples.get(from + colors).copied().unwrap_or(255);
        }
        samples.truncate(pixels * colors);
        let opaque = mask.iter().all(|&a| a == 255);
        (samples, (!opaque).then_some(mask))
    } else {
        (samples, None)
    };
    let components = u8::try_from(colors).unwrap_or(3);
    let mut image = image_dictionary(width, height);
    image.insert(b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec()));
    let data = deflate(&color, budget)?;
    drop(color);
    let mut left = budget.saturating_sub(data.len());
    let mask = match mask {
        Some(mask) => {
            let mask = deflate(&mask, left)?;
            left = left.saturating_sub(mask.len());
            Some(mask)
        }
        None => None,
    };
    let icc = match icc {
        Some(icc) => Some((deflate(&icc, left)?, components)),
        None => None,
    };
    let placed = Placed {
        width,
        height,
        dpi,
        orientation: 1,
        arithmetic: false,
        image,
        data,
        mask,
        icc,
    };
    Ok(placed.with_color(components))
}

/// Pixels per inch from the `pHYs` chunk's x value; 0 when it declares none.
fn png_resolution(info: &png::Info<'_>) -> u32 {
    info.pixel_dims
        .and_then(|dims| resolution::png_dpi(dims.xppu, matches!(dims.unit, png::Unit::Meter)))
        .unwrap_or(0)
}

/// A PNG decoder error: past the decoding budget is too large, anything
/// else unreadable.
fn png_error(error: png::DecodingError) -> ImageError {
    match error {
        png::DecodingError::LimitsExceeded => ImageError::TooLarge,
        error => ImageError::Unreadable(error.to_string()),
    }
}

/// A zeroed buffer, or an error when the memory is not there (an
/// allocation failure would abort the whole engine on wasm32).
fn zeroed(size: usize) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(size)
        .map_err(|_| "not enough memory to decode the image")?;
    buffer.resize(size, 0);
    Ok(buffer)
}

/// `compress_to_vec_zlib(bytes, 6)`, refused past `cap` bytes; memory it
/// cannot get is an error rather than an abort.
pub(crate) fn deflate(bytes: &[u8], cap: usize) -> Result<Vec<u8>, ImageError> {
    let mut compressor = CompressorOxide::new(create_comp_flags_from_zip_params(6, 1, 0));
    let mut out = Vec::new();
    let mut failure = None;
    let (status, _) = compress_to_output(&mut compressor, bytes, TDEFLFlush::Finish, |chunk| {
        if out.len().saturating_add(chunk.len()) > cap {
            failure = Some(ImageError::TooLarge);
            return false;
        }
        if out.try_reserve(chunk.len()).is_err() {
            failure = Some("not enough memory to store the image".into());
            return false;
        }
        out.extend_from_slice(chunk);
        true
    });
    match (failure, status) {
        (Some(failure), _) => Err(failure),
        (None, TDEFLStatus::Done) => Ok(out),
        (None, _) => Err("the image could not be compressed".into()),
    }
}

/// A copy of `bytes`, within `cap`.
fn stored_copy(bytes: &[u8], cap: usize) -> Result<Vec<u8>, ImageError> {
    if bytes.len() > cap {
        return Err(ImageError::TooLarge);
    }
    let mut out = Vec::new();
    out.try_reserve_exact(bytes.len())
        .map_err(|_| "not enough memory to store the image")?;
    out.extend_from_slice(bytes);
    Ok(out)
}

fn image_dictionary(width: u32, height: u32) -> Dictionary {
    let mut image = Dictionary::new();
    image.insert(b"Type".to_vec(), Object::Name(b"XObject".to_vec()));
    image.insert(b"Subtype".to_vec(), Object::Name(b"Image".to_vec()));
    image.insert(b"Width".to_vec(), Object::Integer(i64::from(width)));
    image.insert(b"Height".to_vec(), Object::Integer(i64::from(height)));
    image.insert(b"BitsPerComponent".to_vec(), Object::Integer(8));
    image
}

impl Placed {
    /// Notes the device colour space; an ICC profile replaces it when written.
    fn with_color(mut self, components: u8) -> Self {
        let space: &[u8] = match components {
            1 => b"DeviceGray",
            4 => b"DeviceCMYK",
            _ => b"DeviceRGB",
        };
        self.image
            .insert(b"ColorSpace".to_vec(), Object::Name(space.to_vec()));
        self
    }
}

/// What the JPEG headers say.
#[derive(Debug, Default)]
struct JpegInfo {
    width: u32,
    height: u32,
    components: u8,
    adobe: bool,
    icc: Vec<(u8, u8, Vec<u8>)>,
    arithmetic: bool,
    /// The first EXIF APP1 segment's declarations.
    exif: Option<resolution::Exif>,
    /// The first Photoshop APP13 resolution.
    photoshop: Option<u32>,
    /// The first JFIF APP0 segment's resolution (outer `Some` once read).
    jfif: Option<Option<u32>>,
}

fn jpeg(bytes: &[u8], budget: usize) -> Result<Placed, ImageError> {
    let info = jpeg_info(bytes)?;
    let (dpi, orientation) = info.resolution();
    let mut image = image_dictionary(info.width, info.height);
    if info.adobe && info.components == 4 {
        let decode = [1, 0, 1, 0, 1, 0, 1, 0].map(Object::Integer);
        image.insert(b"Decode".to_vec(), Object::Array(decode.to_vec()));
    }
    image.insert(b"Filter".to_vec(), Object::Name(b"DCTDecode".to_vec()));
    let data = stored_copy(bytes, budget)?;
    let icc = match icc_profile(info.icc) {
        Some(icc) => Some((
            deflate(&icc, budget.saturating_sub(data.len()))?,
            info.components,
        )),
        None => None,
    };
    let placed = Placed {
        width: info.width,
        height: info.height,
        dpi,
        orientation,
        arithmetic: info.arithmetic,
        image,
        data,
        mask: None,
        icc,
    };
    Ok(placed.with_color(info.components))
}

impl JpegInfo {
    /// Pixels per inch (0 when none) and EXIF orientation (1 when none):
    /// EXIF, then Photoshop, then JFIF for the resolution.
    fn resolution(&self) -> (u32, u8) {
        let exif = self.exif.unwrap_or_default();
        let dpi = resolution::jpeg_dpi(exif.dpi, self.photoshop, self.jfif.flatten());
        (dpi, exif.orientation.unwrap_or(1))
    }
}

fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]))
}

/// The marker segments before the first scan.
fn jpeg_info(bytes: &[u8]) -> Result<JpegInfo, String> {
    let mut info = JpegInfo::default();
    let mut frame = false;
    let mut at = 2;
    loop {
        let (Some(&0xFF), Some(&marker)) = (bytes.get(at), bytes.get(at + 1)) else {
            return Err("the JPEG ends before its image data".into());
        };
        match marker {
            0xFF => {
                at += 1;
                continue;
            }
            0x01 | 0xD0..=0xD8 => {
                at += 2;
                continue;
            }
            0xD9 | 0xDA => break,
            _ => {}
        }
        let length = usize::from(be16(bytes, at + 2).ok_or("truncated JPEG marker")?);
        let segment = (length >= 2)
            .then(|| bytes.get(at + 4..at + 2 + length))
            .flatten()
            .ok_or("truncated JPEG marker")?;
        match marker {
            0xC0 | 0xC1 | 0xC2 | 0xC9 | 0xCA => {
                let (Some(&precision), Some(height), Some(width), Some(&components)) = (
                    segment.first(),
                    be16(segment, 1),
                    be16(segment, 3),
                    segment.get(5),
                ) else {
                    return Err("truncated JPEG frame header".into());
                };
                if precision != 8 {
                    return Err(format!("{precision}-bit JPEG samples are not supported"));
                }
                if !matches!(components, 1 | 3 | 4) {
                    return Err(format!("bad number of components in JPEG: {components}"));
                }
                info.width = u32::from(width);
                info.height = u32::from(height);
                info.components = components;
                info.arithmetic = matches!(marker, 0xC9 | 0xCA);
                frame = true;
            }
            0xC3 | 0xC5..=0xC7 | 0xCB | 0xCD..=0xCF => {
                return Err("lossless and hierarchical JPEG are not supported".into());
            }
            0xE2 if segment.starts_with(b"ICC_PROFILE\0") => {
                if let (Some(&sequence), Some(&count), Some(data)) =
                    (segment.get(12), segment.get(13), segment.get(14..))
                {
                    info.icc.push((sequence, count, data.to_vec()));
                }
            }
            0xEE if segment.starts_with(b"Adobe") => info.adobe = true,
            0xE0 if info.jfif.is_none() && segment.starts_with(b"JFIF\0") => {
                info.jfif = Some(resolution::jfif_dpi(segment));
            }
            0xE1 if info.exif.is_none() && segment.starts_with(b"Exif\0") => {
                info.exif = Some(resolution::exif(segment));
            }
            0xED if info.photoshop.is_none() && segment.starts_with(b"Photoshop 3.0\0") => {
                info.photoshop = resolution::photoshop_dpi(segment);
            }
            _ => {}
        }
        at += 2 + length;
    }
    if !frame {
        return Err("the JPEG has no frame header".into());
    }
    Ok(info)
}

/// The APP2 ICC chunks in order, when they are all there.
fn icc_profile(mut chunks: Vec<(u8, u8, Vec<u8>)>) -> Option<Vec<u8>> {
    let count = chunks.first()?.1;
    chunks.sort_by_key(|chunk| chunk.0);
    let complete = chunks.len() == usize::from(count)
        && (chunks.iter().enumerate())
            .all(|(index, chunk)| chunk.1 == count && usize::from(chunk.0) == index + 1);
    complete.then(|| chunks.into_iter().flat_map(|chunk| chunk.2).collect())
}

/// JavaScript `Math.round(x * 100) / 100` for positive `x`.
fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// A PDF number: an integer when the value is one.
fn number(value: f64) -> Object {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        Object::Integer(value as i64)
    } else {
        Object::real(value).unwrap_or(Object::Integer(0))
    }
}

/// The streams `store` writes for an image (dictionary and raw bytes),
/// before their objects exist: the ICC profile, the soft mask and the
/// image, whose dictionary names the first two once they are written.
struct Streams {
    icc: Option<(Dictionary, Vec<u8>)>,
    mask: Option<(Dictionary, Vec<u8>)>,
    image: Dictionary,
    data: Vec<u8>,
}

impl Placed {
    fn streams(self) -> Streams {
        let Placed {
            width,
            height,
            image,
            data,
            mask,
            icc,
            ..
        } = self;
        let icc = icc.map(|(profile, components)| {
            let mut stream = Dictionary::new();
            stream.insert(b"N".to_vec(), Object::Integer(i64::from(components)));
            if let Some(space) = image.get(b"ColorSpace".as_slice()).cloned() {
                stream.insert(b"Alternate".to_vec(), space);
            }
            stream.insert(b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec()));
            (stream, profile)
        });
        let mask = mask.map(|mask| {
            let mut stream = image_dictionary(width, height);
            stream.insert(b"ColorSpace".to_vec(), Object::Name(b"DeviceGray".to_vec()));
            stream.insert(b"Filter".to_vec(), Object::Name(b"FlateDecode".to_vec()));
            (stream, mask)
        });
        Streams {
            icc,
            mask,
            image,
            data,
        }
    }
}

/// Stores the image, with its ICC profile and soft mask, as objects of
/// `target`; returns the image XObject.
pub fn store<E: Edit>(
    target: &mut E,
    placed: Placed,
) -> Result<ObjectId, crate::document::DocumentError> {
    write(target, placed.streams())
}

/// `store`, unless one of `candidates` already holds this image as `store`
/// writes it (the same samples, dictionary, soft mask and ICC profile):
/// that object is returned and nothing is written, so identical images
/// share one image object. Only candidates
/// whose samples have the image's length are compared, byte for byte.
pub fn store_shared<E: Edit>(
    target: &mut E,
    placed: Placed,
    candidates: &[ObjectId],
) -> Result<ObjectId, crate::document::DocumentError> {
    let streams = placed.streams();
    let doc = target.document();
    if let Some(&id) = candidates.iter().find(|&&id| holds(doc, id, &streams)) {
        return Ok(id);
    }
    write(target, streams)
}

/// Image `id` is what `write` makes of `streams`.
fn holds(doc: &crate::document::Document<'_>, id: ObjectId, streams: &Streams) -> bool {
    let bare = |dict: &Dictionary| {
        let mut dict = dict.clone();
        dict.remove(b"Length".as_slice());
        dict
    };
    let dict_of = |id: ObjectId| {
        doc.object(id)
            .ok()
            .filter(|object| object.stream.is_some())
            .and_then(|object| object.value.as_dict())
    };
    let same = |id: ObjectId, dict: &Dictionary, data: &[u8]| {
        doc.stream_data(id).ok().flatten() == Some(data)
            && dict_of(id).is_some_and(|stored| bare(stored) == bare(dict))
    };
    let Some(image) = dict_of(id) else {
        return false;
    };
    if doc.stream_data(id).ok().flatten().map(<[u8]>::len) != Some(streams.data.len()) {
        return false;
    }
    let mut expected = streams.image.clone();
    if let Some((dict, profile)) = &streams.icc {
        let space = image.get(b"ColorSpace".as_slice());
        let profile_id = match space.and_then(Object::as_array) {
            Some([Object::Name(name), Object::Reference(profile)]) if name == b"ICCBased" => {
                Some(*profile)
            }
            _ => None,
        };
        match (profile_id, space) {
            (Some(profile_id), Some(space)) if same(profile_id, dict, profile) => {
                expected.insert(b"ColorSpace".to_vec(), space.clone());
            }
            _ => return false,
        }
    }
    if let Some((dict, mask)) = &streams.mask {
        match image
            .get(b"SMask".as_slice())
            .and_then(Object::as_reference)
        {
            Some(mask_id) if same(mask_id, dict, mask) => {
                expected.insert(b"SMask".to_vec(), Object::Reference(mask_id));
            }
            _ => return false,
        }
    }
    same(id, &expected, &streams.data)
}

fn write<E: Edit>(
    target: &mut E,
    streams: Streams,
) -> Result<ObjectId, crate::document::DocumentError> {
    let Streams {
        icc,
        mask,
        mut image,
        data,
    } = streams;
    if let Some((stream, profile)) = icc {
        let id = target.allocate(Object::Null)?;
        target.put_stream_raw(id, stream, profile)?;
        let space = Object::Array(vec![
            Object::Name(b"ICCBased".to_vec()),
            Object::Reference(id),
        ]);
        image.insert(b"ColorSpace".to_vec(), space);
    }
    if let Some((stream, mask)) = mask {
        let id = target.allocate(Object::Null)?;
        target.put_stream_raw(id, stream, mask)?;
        image.insert(b"SMask".to_vec(), Object::Reference(id));
    }
    let image_id = target.allocate(Object::Null)?;
    target.put_stream_raw(image_id, image, data)?;
    Ok(image_id)
}

/// Adds the image and a page showing it to `doc`; returns the page object.
pub fn add_page(
    doc: &mut Builder,
    placed: Placed,
) -> Result<ObjectId, crate::document::DocumentError> {
    let (dpi, orientation) = (placed.dpi, placed.orientation);
    let (width, height) = (placed.width, placed.height);
    let image_id = store(doc, placed)?;

    // engine.ts: a declared resolution other than 96, else 72; x for both axes.
    let dpi = if dpi > 0 && dpi != 96 {
        f64::from(dpi)
    } else {
        72.0
    };
    let (across, down) = match orientation {
        5..=8 => (height, width),
        _ => (width, height),
    };
    let natural = (f64::from(across) * 72.0 / dpi, f64::from(down) * 72.0 / dpi);
    let shrink = 1f64
        .min(MAX_PAGE_POINTS / natural.0)
        .min(MAX_PAGE_POINTS / natural.1);
    let (w, h) = (round2(natural.0 * shrink), round2(natural.1 * shrink));
    // The image's unit square onto the page, upright as EXIF says.
    let matrix = match orientation {
        2 => [-w, 0.0, 0.0, h, w, 0.0],
        3 => [-w, 0.0, 0.0, -h, w, h],
        4 => [w, 0.0, 0.0, -h, 0.0, h],
        5 => [0.0, -h, -w, 0.0, w, h],
        6 => [0.0, -h, w, 0.0, 0.0, h],
        7 => [0.0, h, w, 0.0, 0.0, 0.0],
        8 => [0.0, h, -w, 0.0, w, 0.0],
        _ => [w, 0.0, 0.0, h, 0.0, 0.0],
    };
    let matrix: Vec<String> = matrix
        .iter()
        .map(|&v| crate::app::request::js_number(v))
        .collect();
    let content = format!("q {} cm /Im0 Do Q", matrix.join(" "));
    let contents = doc.allocate(Object::Null)?;
    doc.put_stream_raw(contents, Dictionary::new(), content.into_bytes())?;
    if (2..=8).contains(&orientation) {
        doc.update(image_id, |value| {
            if let Some(image) = value.as_dict_mut() {
                image.insert(TURNED_KEY.to_vec(), Object::Integer(orientation.into()));
            }
        })?;
    }

    let mut xobjects = Dictionary::new();
    xobjects.insert(b"Im0".to_vec(), Object::Reference(image_id));
    let mut resources = Dictionary::new();
    resources.insert(b"XObject".to_vec(), Object::Dictionary(xobjects));
    let resources = doc.allocate(Object::Dictionary(resources))?;

    let mut page = Dictionary::new();
    page.insert(b"Type".to_vec(), Object::Name(b"Page".to_vec()));
    let media = [0.0, 0.0, w, h].map(number);
    page.insert(b"MediaBox".to_vec(), Object::Array(media.to_vec()));
    page.insert(b"Rotate".to_vec(), Object::Integer(0));
    page.insert(b"Resources".to_vec(), Object::Reference(resources));
    page.insert(b"Contents".to_vec(), Object::Reference(contents));
    doc.allocate(Object::Dictionary(page))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icc_chunks() {
        let chunks = vec![(2, 2, b"cd".to_vec()), (1, 2, b"ab".to_vec())];
        assert_eq!(icc_profile(chunks), Some(b"abcd".to_vec()));
        assert_eq!(icc_profile(vec![(1, 2, b"ab".to_vec())]), None);
    }

    #[test]
    fn truncated_jpegs_and_pngs_are_refused_without_panicking() {
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16];
        jpeg.extend_from_slice(b"JFIF\0\x01\x01\x01\0\x48\0\x48\0\0");
        jpeg.extend_from_slice(&[
            0xFF, 0xC0, 0, 17, 8, 0, 2, 0, 3, 3, 1, 0x11, 0, 2, 0x11, 1, 3, 0x11, 1,
        ]);
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0, 2, 0xFF, 0xD9]);
        let placed = read(&jpeg, usize::MAX).expect("headers are enough");
        assert_eq!((placed.width, placed.height, placed.dpi), (3, 2, 72));
        for end in 0..jpeg.len() {
            let _ = read(&jpeg[..end], usize::MAX);
        }
        assert!(read(b"\x89PNG\r\n\x1a\nnot really", usize::MAX).is_err());
        assert!(read(b"GIF89a", usize::MAX).is_err());
        assert_eq!(
            read(&jpeg, jpeg.len() - 1).err(),
            Some(ImageError::TooLarge)
        );
    }

    #[test]
    fn deflate_matches_miniz_and_stops_at_its_cap() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut noise = |len: usize| -> Vec<u8> {
            (0..len)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    state as u8
                })
                .collect()
        };
        for input in [
            vec![],
            vec![0; 1],
            vec![7; 1_000_000],
            noise(100),
            noise(300_000),
        ] {
            let expected = miniz_oxide::deflate::compress_to_vec_zlib(&input, 6);
            assert_eq!(deflate(&input, usize::MAX).unwrap(), expected);
            assert_eq!(deflate(&input, expected.len()).unwrap(), expected);
            if !expected.is_empty() {
                assert_eq!(
                    deflate(&input, expected.len() - 1),
                    Err(ImageError::TooLarge)
                );
            }
        }
    }
}
