//! What the renderer did with a canvas: decode a picked PNG/JPEG upright (`createImageBitmap`,
//! EXIF orientation applied), scale it (`drawImage` with high-quality smoothing) and encode RGBA
//! as PNG (`OffscreenCanvas.convertToBlob`).
//!
//! PNG goes through the `png` crate (the engine's own pin). JPEG is decoded by the engine's
//! view codecs (hayro's DCT decoder, reached through a one-image PDF), so no new decoder is
//! added. JPEG export uses the app's own encoder (`crate::jpeg`).

use std::io::Cursor;
use tandem_pdf_engine::view::{ImageParams, ViewDocument};

/// Straight RGBA8 pixels, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Decoded pixels past this are refused (a 16384 × 16384 RGBA image is 1 GiB).
const MAX_DECODED_BYTES: usize = 256 << 20;

/// Decodes a PNG or JPEG (by its magic bytes) to upright RGBA. None when it does not decode.
pub fn decode(bytes: &[u8]) -> Option<Rgba> {
    if bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]) {
        decode_png(bytes)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        let image = decode_jpeg(bytes)?;
        Some(orient(image, exif_orientation(bytes)))
    } else {
        None
    }
}

fn decode_png(bytes: &[u8]) -> Option<Rgba> {
    let limits = png::Limits {
        bytes: MAX_DECODED_BYTES,
    };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), limits);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let size = reader
        .output_buffer_size()
        .filter(|&size| size <= MAX_DECODED_BYTES)?;
    let mut samples = vec![0; size];
    let frame = reader.next_frame(&mut samples).ok()?;
    samples.truncate(frame.buffer_size());
    if frame.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let channels = match frame.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return None,
    };
    let data = to_rgba(&samples, channels)?;
    Some(Rgba {
        width: frame.width,
        height: frame.height,
        data,
    })
}

/// 1 (gray), 2 (gray + alpha), 3 (RGB), 4 (RGBA) channels → RGBA.
fn to_rgba(samples: &[u8], channels: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(samples.len() / channels * 4);
    for px in samples.chunks_exact(channels) {
        match channels {
            1 => out.extend_from_slice(&[px[0], px[0], px[0], 255]),
            2 => out.extend_from_slice(&[px[0], px[0], px[0], px[1]]),
            3 => out.extend_from_slice(&[px[0], px[1], px[2], 255]),
            4 => out.extend_from_slice(px),
            _ => return None,
        }
    }
    Some(out)
}

/// A JPEG's frame header: (width, height, components).
fn jpeg_frame(bytes: &[u8]) -> Option<(u32, u32, u8)> {
    let mut at = 2;
    while at + 4 <= bytes.len() {
        if bytes[at] != 0xff {
            return None;
        }
        let marker = bytes[at + 1];
        if marker == 0xff {
            at += 1;
            continue;
        }
        if matches!(marker, 0xd8 | 0x01 | 0xd0..=0xd7) {
            at += 2;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        // SOF0-SOF15 except DHT (C4), JPG (C8) and DAC (CC).
        if matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            let body = bytes.get(at + 4..at + 2 + length)?;
            if body.len() < 6 {
                return None;
            }
            let height = u32::from(u16::from_be_bytes([body[1], body[2]]));
            let width = u32::from(u16::from_be_bytes([body[3], body[4]]));
            return Some((width, height, body[5]));
        }
        if marker == 0xda {
            return None;
        }
        at += 2 + length;
    }
    None
}

/// Decodes a baseline or progressive JPEG with the engine's DCT codec: the file becomes the
/// one image of a minimal PDF, which the view decodes.
fn decode_jpeg(bytes: &[u8]) -> Option<Rgba> {
    let (width, height, components) = jpeg_frame(bytes)?;
    if width == 0 || height == 0 {
        return None;
    }
    let space = match components {
        1 => "/DeviceGray",
        3 => "/DeviceRGB",
        4 => "/DeviceCMYK",
        _ => return None,
    };
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] >>".to_string(),
    ];
    for (i, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    offsets.push(pdf.len());
    pdf.extend_from_slice(
        format!(
            "4 0 obj\n<< /Type /XObject /Subtype /Image /Width {width} /Height {height} \
             /ColorSpace {space} /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
            bytes.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(bytes);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");
    let xref = pdf.len();
    pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let view = ViewDocument::open(pdf, "").ok()?;
    let samples = view.decoded_image(
        4,
        0,
        ImageParams {
            indexed: false,
            bits: Some(8),
            components: Some(components),
            width,
            height,
        },
    )?;
    let (width, height) = (
        samples.width.unwrap_or(width),
        samples.height.unwrap_or(height),
    );
    let channels = usize::from(samples.components.unwrap_or(components));
    let pixels = (width as usize).checked_mul(height as usize)?;
    if pixels.checked_mul(4)? > MAX_DECODED_BYTES || samples.data.len() < pixels * channels {
        return None;
    }
    let data = if channels == 4 {
        // CMYK: the plain conversion (no profile).
        let mut out = Vec::with_capacity(pixels * 4);
        for px in samples.data[..pixels * 4].as_chunks::<4>().0 {
            let k = 255 - u32::from(px[3]);
            let channel = |c: u8| ((255 - u32::from(c)) * k / 255) as u8;
            out.extend_from_slice(&[channel(px[0]), channel(px[1]), channel(px[2]), 255]);
        }
        out
    } else {
        to_rgba(&samples.data[..pixels * channels], channels)?
    };
    Some(Rgba {
        width,
        height,
        data,
    })
}

/// The EXIF Orientation (1-8) of a JPEG's APP1 segment; 1 when there is none.
pub fn exif_orientation(bytes: &[u8]) -> u16 {
    exif_orientation_of(bytes).unwrap_or(1)
}

fn exif_orientation_of(bytes: &[u8]) -> Option<u16> {
    let mut at = 2;
    while at + 4 <= bytes.len() {
        if bytes[at] != 0xff {
            return None;
        }
        let marker = bytes[at + 1];
        let length = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        if marker == 0xda || length < 2 {
            return None;
        }
        let body = bytes.get(at + 4..at + 2 + length)?;
        if marker == 0xe1 && body.starts_with(b"Exif\0\0") {
            return tiff_orientation(&body[6..]);
        }
        at += 2 + length;
    }
    None
}

fn tiff_orientation(tiff: &[u8]) -> Option<u16> {
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b = tiff.get(at..at + 2)?;
        Some(if little {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b = tiff.get(at..at + 4)?;
        Some(if little {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    };
    let ifd = u32_at(4)? as usize;
    let count = usize::from(u16_at(ifd)?);
    for i in 0..count {
        let entry = ifd + 2 + i * 12;
        if u16_at(entry)? == 0x0112 {
            let value = u16_at(entry + 8)?;
            return (1..=8).contains(&value).then_some(value);
        }
    }
    None
}

/// The image as it is meant to be seen under EXIF `orientation` (1-8).
pub fn orient(image: Rgba, orientation: u16) -> Rgba {
    if !(2..=8).contains(&orientation) {
        return image;
    }
    let (w, h) = (image.width as usize, image.height as usize);
    let swap = orientation >= 5;
    let (ow, oh) = if swap { (h, w) } else { (w, h) };
    let mut data = vec![0; ow * oh * 4];
    for y in 0..oh {
        for x in 0..ow {
            // The source pixel shown at (x, y).
            let (sx, sy) = match orientation {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (y, h - 1 - x),
                7 => (w - 1 - y, h - 1 - x),
                _ => (w - 1 - y, x),
            };
            let from = (sy * w + sx) * 4;
            let to = (y * ow + x) * 4;
            data[to..to + 4].copy_from_slice(&image.data[from..from + 4]);
        }
    }
    Rgba {
        width: ow as u32,
        height: oh as u32,
        data,
    }
}

/// `drawImage` into a `width` × `height` canvas: an area average (each output pixel is the
/// coverage-weighted mean of the source pixels under it), alpha-weighted.
pub fn scale(image: &Rgba, width: u32, height: u32) -> Rgba {
    if width == image.width && height == image.height {
        return image.clone();
    }
    let (sw, sh) = (image.width as usize, image.height as usize);
    let (dw, dh) = (width.max(1) as usize, height.max(1) as usize);
    let fx = sw as f64 / dw as f64;
    let fy = sh as f64 / dh as f64;
    // Per destination column: the source columns it covers and their weights.
    let spans = |count: usize, factor: f64, limit: usize| -> Vec<Vec<(usize, f64)>> {
        (0..count)
            .map(|d| {
                let start = d as f64 * factor;
                let end = start + factor;
                let mut span = Vec::new();
                let mut s = start.floor() as usize;
                while (s as f64) < end && s < limit {
                    let lo = start.max(s as f64);
                    let hi = end.min(s as f64 + 1.0);
                    if hi > lo {
                        span.push((s, hi - lo));
                    }
                    s += 1;
                }
                if span.is_empty() {
                    span.push((limit.saturating_sub(1).min(start as usize), 1.0));
                }
                span
            })
            .collect()
    };
    let columns = spans(dw, fx, sw);
    let rows = spans(dh, fy, sh);
    let mut data = vec![0; dw * dh * 4];
    for (y, row) in rows.iter().enumerate() {
        for (x, column) in columns.iter().enumerate() {
            let mut sum = [0.0f64; 4];
            let mut total = 0.0;
            for &(sy, wy) in row {
                for &(sx, wx) in column {
                    let w = wx * wy;
                    let p = &image.data[(sy * sw + sx) * 4..][..4];
                    let a = f64::from(p[3]) * w;
                    sum[0] += f64::from(p[0]) * a;
                    sum[1] += f64::from(p[1]) * a;
                    sum[2] += f64::from(p[2]) * a;
                    sum[3] += a;
                    total += w;
                }
            }
            let out = &mut data[(y * dw + x) * 4..][..4];
            if sum[3] > 0.0 {
                for c in 0..3 {
                    out[c] = (sum[c] / sum[3]).round().clamp(0.0, 255.0) as u8;
                }
            }
            out[3] = if total > 0.0 {
                (sum[3] / total).round().clamp(0.0, 255.0) as u8
            } else {
                0
            };
        }
    }
    Rgba {
        width: dw as u32,
        height: dh as u32,
        data,
    }
}

/// RGBA8 → PNG (no metadata). Err when the buffer does not match the size or encoding fails.
pub fn encode_png(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4));
    if width == 0 || height == 0 || expected != Some(data.len()) {
        return Err("the image has no pixels".into());
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(data).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// RGBA pixels as a JPEG at quality 90 (the TS used the canvas's encoder at 0.9), through
/// the app's own baseline encoder (`crate::jpeg`).
pub fn encode_jpeg(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 || data.len() < (width * height * 4) as usize {
        return Err("The page could not be drawn for export. Try a lower resolution.".into());
    }
    Ok(crate::jpeg::encode_rgba(width, height, data, 90, 72))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32) -> Rgba {
        let data = (0..width * height)
            .flat_map(|i| [i as u8, 0, 0, 255])
            .collect();
        Rgba {
            width,
            height,
            data,
        }
    }

    #[test]
    fn png_round_trips() {
        let source = image(3, 2);
        let png = encode_png(&source.data, 3, 2).unwrap();
        assert_eq!(decode(&png), Some(source));
    }

    #[test]
    fn orientation_six_turns_clockwise() {
        // 3 wide, 2 high: pixel values 0 1 2 / 3 4 5.
        let turned = orient(image(3, 2), 6);
        assert_eq!((turned.width, turned.height), (2, 3));
        let reds: Vec<u8> = turned.data.chunks(4).map(|p| p[0]).collect();
        assert_eq!(reds, vec![3, 0, 4, 1, 5, 2]);
    }

    #[test]
    fn scaling_averages() {
        let mut source = image(2, 2);
        for (i, p) in source.data.chunks_mut(4).enumerate() {
            p[0] = [0, 100, 200, 100][i];
        }
        let half = scale(&source, 1, 1);
        assert_eq!(half.data, vec![100, 0, 0, 255]);
    }

    #[test]
    fn exif_orientation_is_read_from_app1() {
        // SOI, APP1 "Exif" with a big-endian TIFF holding one entry: Orientation = 6.
        let mut tiff = b"MM\0\x2a\0\0\0\x08\0\x01".to_vec();
        tiff.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0]);
        tiff.extend_from_slice(&[0, 0, 0, 0]);
        let mut body = b"Exif\0\0".to_vec();
        body.extend_from_slice(&tiff);
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
        jpeg.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(&body);
        jpeg.extend_from_slice(&[0xff, 0xda, 0, 2]);
        assert_eq!(exif_orientation(&jpeg), 6);
        assert_eq!(exif_orientation(&[0xff, 0xd8, 0xff, 0xda, 0, 2]), 1);
    }
}
