// SPDX-License-Identifier: AGPL-3.0-or-later
//! An image whose data is too large to hold (the integration acceptance
//! review, round 3). hayro-syntax holds an image's decoded data whole,
//! within `MAX_IMAGE_DECODED_BYTES` (512 MiB), so a 561 KB file holding one
//! 24000 × 24000 grayscale Flate image (576 MB of samples) decoded 512 MiB
//! before it was refused, and WASM memory grew to 1,030 MB, never given
//! back. MuPDF draws the page, subsampling as it decodes. Such data, behind
//! FlateDecode alone (with or without a predictor on whole bytes), is now
//! decoded a strip at a time into the reduced image; other filter chains
//! that could decode past the limit are refused before decoding. The memory
//! bound is measured on WASM (tests/rust-engine-large-image.test.mjs); here
//! the pages draw, which hayro could not do holding the data.
use miniz_oxide::deflate::core::{
    CompressorOxide, TDEFLFlush, TDEFLStatus, compress, create_comp_flags_from_zip_params,
};
use tandem_pdf_engine::view::{RenderBudgets, RenderOptions, RgbaImage, ViewDocument, ViewError};

/// A classic-xref PDF from object bodies numbered from 1 (object 1 the catalog).
fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// `rows` rows from `row`, zlib-deflated a row at a time (the data is never
/// held whole).
fn deflate_rows(rows: u32, row: impl Fn(u32) -> Vec<u8>) -> Vec<u8> {
    fn feed(d: &mut CompressorOxide, out: &mut Vec<u8>, mut input: &[u8], flush: TDEFLFlush) {
        let mut buf = vec![0; 1 << 16];
        loop {
            let (status, used, written) = compress(d, input, &mut buf, flush);
            out.extend_from_slice(&buf[..written]);
            input = &input[used..];
            match status {
                TDEFLStatus::Done => return,
                TDEFLStatus::Okay
                    if flush == TDEFLFlush::None && input.is_empty() && written < buf.len() =>
                {
                    return;
                }
                TDEFLStatus::Okay => {}
                status => panic!("deflate: {status:?}"),
            }
        }
    }
    let mut d = CompressorOxide::new(create_comp_flags_from_zip_params(9, 15, 0));
    let mut out = Vec::new();
    for index in 0..rows {
        feed(&mut d, &mut out, &row(index), TDEFLFlush::None);
    }
    feed(&mut d, &mut out, &[], TDEFLFlush::Finish);
    out
}

/// A 100 × 100 page drawing image object 5 over the whole page.
fn image_page(image: Vec<u8>) -> ViewDocument {
    let objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject << /Im 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream("", b"q 100 0 0 100 0 0 cm /Im Do Q"),
        image,
    ];
    ViewDocument::open(pdf(&objects), "").expect("open")
}

fn pixel(image: &RgbaImage, x: u32, y: u32) -> [u8; 3] {
    let at = ((y * image.width + x) * 4) as usize;
    [image.pixels[at], image.pixels[at + 1], image.pixels[at + 2]]
}

const SIDE: u32 = 24000;

fn gray_image(extra: &str, data: &[u8]) -> Vec<u8> {
    stream(
        &format!(
            "/Type /XObject /Subtype /Image /Width {SIDE} /Height {SIDE} /BitsPerComponent 8 /ColorSpace /DeviceGray {extra}"
        ),
        data,
    )
}

#[test]
fn an_image_too_large_to_hold_is_drawn_reduced_as_it_decodes() {
    // The review's image: 24000 × 24000 grayscale behind FlateDecode, its
    // top half black and its bottom half white (576 MB of samples).
    let side = SIDE as usize;
    let (black, white) = (vec![0u8; side], vec![255u8; side]);
    let data = deflate_rows(SIDE, |row| {
        if row < SIDE / 2 {
            black.clone()
        } else {
            white.clone()
        }
    });
    assert!(data.len() < 1 << 20, "{} bytes", data.len());
    let doc = image_page(gray_image("/Filter /FlateDecode", &data));
    let image = doc
        .render(0, 1.0, RenderOptions::default())
        .expect("render");
    assert_eq!(pixel(&image, 50, 20), [0, 0, 0]);
    assert_eq!(pixel(&image, 50, 80), [255, 255, 255]);
    // Within a far smaller sample budget, as reduced.
    let budgets = RenderBudgets {
        image_samples: 1 << 16,
        ..RenderBudgets::default()
    };
    let small = doc
        .render_with_budgets(0, 1.0, RenderOptions::default(), budgets)
        .expect("render");
    assert_eq!(pixel(&small, 50, 20), [0, 0, 0]);
    assert_eq!(pixel(&small, 50, 80), [255, 255, 255]);
}

#[test]
fn a_predictor_is_undone_a_row_at_a_time() {
    // Every row a PNG Sub row: 64, then zeros (each sample the one before).
    let mut row = vec![0u8; SIDE as usize + 1];
    (row[0], row[1]) = (1, 64);
    let data = deflate_rows(SIDE, |_| row.clone());
    let doc = image_page(gray_image(
        &format!("/Filter /FlateDecode /DecodeParms << /Predictor 15 /Columns {SIDE} >>"),
        &data,
    ));
    let image = doc
        .render(0, 1.0, RenderOptions::default())
        .expect("render");
    assert_eq!(pixel(&image, 10, 10), [64, 64, 64]);
    assert_eq!(pixel(&image, 90, 90), [64, 64, 64]);
}

#[test]
fn other_filter_chains_that_could_pass_the_limit_are_refused_before_decoding() {
    // The same data behind ASCIIHexDecode and FlateDecode: not decoded as a
    // stream, and its encoded bytes could inflate past the 512 MiB hayro
    // holds, so it is refused without decoding (it was refused after
    // decoding 512 MiB).
    let side = SIDE as usize;
    let gray = vec![128u8; side];
    let packed = deflate_rows(SIDE, |_| gray.clone());
    let hex: Vec<u8> = packed
        .iter()
        .flat_map(|byte| format!("{byte:02x}").into_bytes())
        .collect();
    let doc = image_page(gray_image("/Filter [/ASCIIHexDecode /FlateDecode]", &hex));
    assert!(matches!(
        doc.render(0, 1.0, RenderOptions::default()),
        Err(ViewError::TooComplex(_))
    ));
    // Data that cannot inflate that far still decodes: a short image
    // declaring the same size draws what it holds.
    let short = deflate_rows(100, |_| gray.clone());
    let hex: Vec<u8> = short
        .iter()
        .flat_map(|byte| format!("{byte:02x}").into_bytes())
        .collect();
    let doc = image_page(gray_image("/Filter [/ASCIIHexDecode /FlateDecode]", &hex));
    assert!(doc.render(0, 1.0, RenderOptions::default()).is_ok());
}

#[test]
fn samples_reduced_a_strip_at_a_time_decode_as_the_whole_image() {
    // 8-bit samples with the default decode array, or its inverse, are read
    // as they are when an image is reduced a strip at a time; through the
    // decode array otherwise: the same values.
    for (decode, expected) in [("", 64), ("/Decode [1 0]", 191), ("/Decode [0 0.5]", 32)] {
        let data = miniz_oxide::deflate::compress_to_vec_zlib(&[64u8; 400 * 400], 6);
        let doc = image_page(stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width 400 /Height 400 /BitsPerComponent 8 /ColorSpace /DeviceGray {decode} /Filter /FlateDecode"
            ),
            &data,
        ));
        let whole = doc
            .render(0, 1.0, RenderOptions::default())
            .expect("render");
        let budgets = RenderBudgets {
            image_samples: 1 << 14,
            ..RenderBudgets::default()
        };
        let reduced = doc
            .render_with_budgets(0, 1.0, RenderOptions::default(), budgets)
            .expect("render");
        // The whole image is resampled by the renderer (193 for 191).
        let (a, b) = (pixel(&whole, 50, 50)[0], pixel(&reduced, 50, 50)[0]);
        assert!(
            b.abs_diff(expected) <= 1 && a.abs_diff(expected) <= 3,
            "{decode}: whole {a}, reduced {b}"
        );
    }
}
