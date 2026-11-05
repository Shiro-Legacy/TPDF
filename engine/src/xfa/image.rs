// SPDX-License-Identifier: MIT
//! Image values (`image` in a draw's or field's `value`, or base64 data
//! bound to an image field): read into `types::ImageData` by `template`
//! and `bind`, prepared for `emit` here.
//!
//! - The content is base64 (`transferEncoding="base64"`, XFA's default
//!   for images; `none` is not read): decoded here, whitespace skipped,
//!   at most `MAX_IMAGE_BYTES` decoded. Anything else is no image.
//! - JPEG (`image/jpeg`, `image/jpg`, `image/pjpeg`) is embedded as it is
//!   (DCTDecode; its size and components read from the frame header), PNG
//!   (`image/png`) decoded and stored Flate-compressed with its alpha as a
//!   soft mask: both by the engine's image reader
//!   (`app::ops::pages::images`), the one the images-to-PDF op uses. The
//!   bytes decide (by signature), the content type only rules other types
//!   out: GIF, BMP, TIFF and the rest draw nothing.
//! - An image wider or taller than `MAX_IMAGE_PIXELS` draws nothing (a
//!   PNG's header is checked before it is decoded). A malformed image
//!   draws nothing. Never panics.
//! - `href` (an external file) is never fetched: nothing is drawn.

use std::sync::Arc;

use super::types::{Aspect, ImageData};
use super::xml::Element;
use crate::app::ops::pages::images::{self, Placed};

/// Decoded bytes of one image, at most.
pub const MAX_IMAGE_BYTES: usize = 16 << 20;
/// Pixels along either side of one image, at most.
pub const MAX_IMAGE_PIXELS: u32 = 10_000;
/// Bytes one image may store in the document (its samples Flate-compressed,
/// its mask, its ICC profile), at most.
const MAX_STORED_BYTES: usize = 64 << 20;

/// The `image` element `e` as a value.
pub fn read_element(e: &Element) -> ImageData {
    let content_type = e.attr("contentType").unwrap_or("").trim().to_lowercase();
    let href = e.attr("href").unwrap_or("").trim().to_string();
    let aspect = Aspect::parse(e.attr("aspect").unwrap_or(""));
    let encoding = e.attr("transferEncoding").unwrap_or("base64").trim();
    let bytes = if href.is_empty() && encoding != "none" {
        decode_base64(&e.text())
    } else {
        None
    };
    ImageData {
        content_type,
        bytes,
        href,
        aspect,
    }
}

/// `template`'s value with a data node's content: the node's base64 text
/// (its own `contentType` and `href` when it has them), keeping the
/// template's aspect. An empty node is an empty image.
pub fn from_data(template: Option<&ImageData>, node: &Element) -> ImageData {
    let base = template.cloned().unwrap_or_default();
    let content_type = node
        .attr("contentType")
        .map(|c| c.trim().to_lowercase())
        .filter(|c| !c.is_empty())
        .unwrap_or(base.content_type);
    let href = node.attr("href").unwrap_or("").trim().to_string();
    let bytes = if href.is_empty() {
        decode_base64(&node.text())
    } else {
        None
    };
    ImageData {
        content_type,
        bytes,
        href,
        aspect: base.aspect,
    }
}

/// Standard base64 (`+/`, `=` padding, whitespace anywhere); `None` for
/// anything else, for nothing, or past `MAX_IMAGE_BYTES`.
pub fn decode_base64(text: &str) -> Option<Arc<[u8]>> {
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0u32;
    let mut padded = false;
    for b in text.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                padded = true;
                continue;
            }
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        if padded {
            return None;
        }
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            if out.len() >= MAX_IMAGE_BYTES {
                return None;
            }
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    (!out.is_empty()).then(|| Arc::from(out))
}

/// The kinds of image drawn.
fn drawable_type(content_type: &str) -> bool {
    matches!(
        content_type,
        "" | "image/jpeg" | "image/jpg" | "image/pjpeg" | "image/png" | "image/x-png"
    )
}

/// `image` read for embedding: `None` when it is not a JPEG or PNG this
/// reader takes, is malformed, or passes the limits.
pub fn prepare(image: &ImageData) -> Option<Placed> {
    if !image.href.is_empty() || !drawable_type(&image.content_type) {
        return None;
    }
    let bytes = image.bytes.as_deref()?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let png = bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    let jpeg = bytes.starts_with(&[0xFF, 0xD8]);
    if !png && !jpeg {
        return None;
    }
    if png {
        // IHDR's width and height, before anything is decoded.
        let be32 = |at: usize| {
            bytes
                .get(at..at + 4)
                .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        };
        if bytes.get(12..16) != Some(b"IHDR".as_slice()) {
            return None;
        }
        let (w, h) = (be32(16)?, be32(20)?);
        if w == 0 || h == 0 || w > MAX_IMAGE_PIXELS || h > MAX_IMAGE_PIXELS {
            return None;
        }
    }
    let placed = images::read(bytes, MAX_STORED_BYTES).ok()?;
    let fits = |v: u32| v > 0 && v <= MAX_IMAGE_PIXELS;
    (fits(placed.width) && fits(placed.height)).then_some(placed)
}

/// The drawn size of an image of `w` by `h` points in a box of `bw` by
/// `bh`. It sits at the box's top-left corner (XFA places images there),
/// clipped to the box.
pub fn place(aspect: Aspect, w: f64, h: f64, bw: f64, bh: f64) -> (f64, f64) {
    if !(w > 0.0 && h > 0.0) {
        return (0.0, 0.0);
    }
    let (dw, dh) = match aspect {
        Aspect::Actual => (w, h),
        Aspect::None => (bw, bh),
        Aspect::Width => (bw, h * bw / w),
        Aspect::Height => (w * bh / h, bh),
        Aspect::Fit => {
            let scale = (bw / w).min(bh / h);
            (w * scale, h * scale)
        }
    };
    (dw.max(0.0), dh.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_reads_and_refuses() {
        assert_eq!(decode_base64("aGVs\nbG8=").as_deref(), Some(&b"hello"[..]));
        assert_eq!(decode_base64("AAEC").as_deref(), Some(&[0u8, 1, 2][..]));
        assert_eq!(decode_base64(""), None);
        assert_eq!(decode_base64("a*b"), None);
        assert_eq!(decode_base64("aG=Vs"), None);
    }

    #[test]
    fn aspects_place_the_image() {
        assert_eq!(place(Aspect::Fit, 20.0, 10.0, 100.0, 100.0), (100.0, 50.0));
        assert_eq!(
            place(Aspect::Actual, 20.0, 10.0, 100.0, 100.0),
            (20.0, 10.0)
        );
        assert_eq!(place(Aspect::Width, 20.0, 10.0, 40.0, 5.0), (40.0, 20.0));
        assert_eq!(place(Aspect::Height, 20.0, 10.0, 40.0, 5.0), (10.0, 5.0));
        assert_eq!(place(Aspect::None, 20.0, 10.0, 40.0, 5.0), (40.0, 5.0));
        assert_eq!(place(Aspect::Fit, 0.0, 10.0, 40.0, 5.0), (0.0, 0.0));
    }

    #[test]
    fn other_types_and_hrefs_are_not_prepared() {
        let mut image = ImageData {
            content_type: "image/gif".into(),
            bytes: Some(Arc::from(&b"GIF89a"[..])),
            ..ImageData::default()
        };
        assert!(prepare(&image).is_none());
        image.content_type = "image/png".into();
        assert!(prepare(&image).is_none());
        image.bytes = Some(Arc::from(&b"\x89PNG\r\n\x1a\nbroken"[..]));
        assert!(prepare(&image).is_none());
        image.href = "pic.png".into();
        assert!(prepare(&image).is_none());
    }
}
