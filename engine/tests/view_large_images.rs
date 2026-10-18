// SPDX-License-Identifier: MIT
//! An image whose data is too large to hold (the integration acceptance
//! review, round 3). hayro-syntax holds an image's decoded data whole,
//! within `MAX_IMAGE_DECODED_BYTES` (512 MiB), so a 561 KB file holding one
//! 24000 × 24000 grayscale Flate image (576 MB of samples) decoded 512 MiB
//! before it was refused, and WASM memory grew to 1,030 MB, never given
//! back. Reference viewers draw the page, subsampling as they decode. Such data, behind
//! FlateDecode alone (with or without a predictor on whole bytes), is now
//! decoded a row at a time into running sums of the reduced image's blocks;
//! other filter chains that could decode past the limit are refused before
//! decoding. The re-review (fix round 3) found the first version gathered a
//! strip of `ky` rows, nearly the whole image for one drawn 2 points tall:
//! the working set is now a row and the sums whatever the reduction
//! (`RenderStats::image_working_peak`), and a page decodes at most 2^30
//! samples reduced (`MAX_REDUCED_SAMPLES`). The WASM memory bound is in
//! tests/rust-engine-large-image.test.mjs.
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
    image_page_with(b"q 100 0 0 100 0 0 cm /Im Do Q", vec![image])
}

/// A 100 × 100 page with `content` and image object 5 (`extra` from 5 on).
fn image_page_with(content: &[u8], extra: Vec<Vec<u8>>) -> ViewDocument {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject << /Im 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream("", content),
    ];
    objects.extend(extra);
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
fn samples_reduced_decode_as_the_whole_image() {
    // 8-bit samples with the default decode array, or its inverse, are read
    // through a table of each code when an image is reduced; through the
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

#[test]
fn an_image_drawn_tiny_is_reduced_within_a_row_and_its_sums() {
    // The re-review's case: the 24000 × 24000 image drawn 2 points tall,
    // so each output row is 12,000 rows of data. The first version gathered
    // them as one strip (288 MB, then copied: 584 MB native RSS, 1,030 MB
    // of WASM); the reduction now holds a row, the block sums and the
    // reduced image.
    let row = vec![128u8; SIDE as usize];
    let data = deflate_rows(SIDE, |_| row.clone());
    let doc = image_page_with(
        b"q 100 0 0 2 0 0 cm /Im Do Q",
        vec![gray_image("/Filter /FlateDecode", &data)],
    );
    let (image, stats) =
        doc.render_with_stats(0, 1.0, RenderOptions::default(), RenderBudgets::default());
    let image = image.expect("render");
    assert!(pixel(&image, 50, 99)[0].abs_diff(128) <= 2);
    assert!(
        stats.image_working_peak > 0 && stats.image_working_peak < 1 << 20,
        "{stats:?}"
    );
    // Drawn over the whole page the same holds (100 output rows).
    let doc = image_page(gray_image("/Filter /FlateDecode", &data));
    let (_, stats) =
        doc.render_with_stats(0, 1.0, RenderOptions::default(), RenderBudgets::default());
    assert!(stats.image_working_peak < 1 << 20, "{stats:?}");
}

#[test]
fn a_page_decodes_a_bounded_number_of_samples_reduced() {
    // An image decoded reduced is decoded in full each time it is drawn at
    // another size (the re-review: one 24000² image drawn at ten sizes,
    // 18 s on WASM). Once is 576 M samples; a second size passes the
    // page's 2^30 and the render fails instead.
    let row = vec![128u8; SIDE as usize];
    let data = deflate_rows(SIDE, |_| row.clone());
    let doc = image_page_with(
        b"q 100 0 0 100 0 0 cm /Im Do Q q 50 0 0 50 0 0 cm /Im Do Q",
        vec![gray_image("/Filter /FlateDecode", &data)],
    );
    assert!(matches!(
        doc.render(0, 1.0, RenderOptions::default()),
        Err(ViewError::TooComplex(_))
    ));
}

/// The re-review's image variants: `bpc` bits, `n` components in `cs`, the
/// four quadrants' samples, extra dictionary entries and a predictor (1 PNG
/// Sub, 2 TIFF).
struct Variant {
    name: &'static str,
    bpc: u32,
    cs: &'static str,
    n: usize,
    q: [&'static [u32]; 4],
    extra: &'static str,
    predictor: u8,
}

fn pack(samples: &[u32], bpc: u32) -> Vec<u8> {
    let mut out = Vec::new();
    match bpc {
        8 => out.extend(samples.iter().map(|&s| s as u8)),
        16 => samples
            .iter()
            .for_each(|&s| out.extend_from_slice(&[(s >> 8) as u8, s as u8])),
        _ => {
            let (mut acc, mut bits) = (0u32, 0);
            for &s in samples {
                acc = (acc << bpc) | s;
                bits += bpc;
                if bits == 8 {
                    out.push(acc as u8);
                    (acc, bits) = (0, 0);
                }
            }
            if bits > 0 {
                out.push((acc << (8 - bits)) as u8);
            }
        }
    }
    out
}

fn variant_image(v: &Variant, side: u32) -> Vec<Vec<u8>> {
    let row_of = |top: bool| {
        let (l, r) = if top {
            (v.q[0], v.q[1])
        } else {
            (v.q[2], v.q[3])
        };
        let samples: Vec<u32> = (0..side as usize)
            .flat_map(|x| if x < side as usize / 2 { l } else { r }.iter().copied())
            .collect();
        let raw = pack(&samples, v.bpc);
        let bpp = (v.n * v.bpc as usize).div_ceil(8);
        let sub = |i: usize| raw[i].wrapping_sub(if i >= bpp { raw[i - bpp] } else { 0 });
        match v.predictor {
            1 => std::iter::once(1).chain((0..raw.len()).map(sub)).collect(),
            2 => (0..raw.len()).map(sub).collect(),
            _ => raw,
        }
    };
    let (top, bottom) = (row_of(true), row_of(false));
    let data = deflate_rows(side, |row| {
        if row < side / 2 {
            top.clone()
        } else {
            bottom.clone()
        }
    });
    let parms = match v.predictor {
        0 => String::new(),
        p => format!(
            "/DecodeParms << /Predictor {} /Colors {} /BitsPerComponent {} /Columns {side} >>",
            if p == 1 { 15 } else { 2 },
            v.n,
            v.bpc
        ),
    };
    let mut objects = vec![stream(
        &format!(
            "/Type /XObject /Subtype /Image /Width {side} /Height {side} /BitsPerComponent {} /ColorSpace {} {} /Filter /FlateDecode {parms}",
            v.bpc, v.cs, v.extra
        ),
        &data,
    )];
    if v.extra.contains("6 0 R") {
        objects.push(stream(
            "/Type /XObject /Subtype /Image /Width 2 /Height 2 /BitsPerComponent 8 /ColorSpace /DeviceGray",
            &[255, 0, 0, 255],
        ));
    }
    objects
}

#[test]
fn images_reduced_a_row_at_a_time_draw_as_the_whole_image_does() {
    // Each of the re-review's 18 variants (the streamed 24000² gray image is
    // above) decoded reduced, forced by a small sample budget, against the
    // same image decoded whole: the four quadrants agree.
    let pal = "[/Indexed /DeviceRGB 3 <ff000000ff000000ffffff00>]";
    let rgb: [&[u32]; 4] = [&[255, 0, 0], &[0, 255, 0], &[0, 0, 255], &[255, 255, 0]];
    let gray8: [&[u32]; 4] = [&[0], &[85], &[170], &[255]];
    let gray16: [&[u32]; 4] = [&[0], &[0x5555], &[0xAAAA], &[0xFFFF]];
    let codes: [&[u32]; 4] = [&[0], &[1], &[2], &[3]];
    let keyed: [&[u32]; 4] = [&[0], &[85], &[170], &[0]];
    let v = |name, bpc, cs, n, q, extra, predictor| Variant {
        name,
        bpc,
        cs,
        n,
        q,
        extra,
        predictor,
    };
    let variants = [
        v("g8", 8, "/DeviceGray", 1, gray8, "", 0),
        v("g8inv", 8, "/DeviceGray", 1, gray8, "/Decode [1 0]", 0),
        v("g8half", 8, "/DeviceGray", 1, gray8, "/Decode [0 0.5]", 0),
        v("g16", 16, "/DeviceGray", 1, gray16, "", 0),
        v("g4", 4, "/DeviceGray", 1, [&[0], &[5], &[10], &[15]], "", 0),
        v("g2", 2, "/DeviceGray", 1, codes, "", 0),
        v("g1", 1, "/DeviceGray", 1, [&[0], &[1], &[1], &[0]], "", 0),
        v(
            "g1dec",
            1,
            "/DeviceGray",
            1,
            [&[0], &[1], &[1], &[0]],
            "/Decode [1 0]",
            0,
        ),
        v("rgb8", 8, "/DeviceRGB", 3, rgb, "", 0),
        v(
            "rgb8dec",
            8,
            "/DeviceRGB",
            3,
            rgb,
            "/Decode [1 0 0 1 1 0]",
            0,
        ),
        v(
            "cmyk8",
            8,
            "/DeviceCMYK",
            4,
            [
                &[255, 0, 0, 0],
                &[0, 255, 0, 0],
                &[0, 0, 255, 0],
                &[0, 0, 0, 128],
            ],
            "",
            0,
        ),
        v("idx8", 8, pal, 1, codes, "", 0),
        v("idx2", 2, pal, 1, codes, "", 0),
        v("rgb8png", 8, "/DeviceRGB", 3, rgb, "", 1),
        v("rgb8tiff", 8, "/DeviceRGB", 3, rgb, "", 2),
        v("g16png", 16, "/DeviceGray", 1, gray16, "", 1),
        v("g8key", 8, "/DeviceGray", 1, keyed, "/Mask [0 0]", 0),
        v("g8smask", 8, "/DeviceGray", 1, keyed, "/SMask 6 0 R", 0),
    ];
    let reduced = RenderBudgets {
        image_samples: 1 << 16,
        ..RenderBudgets::default()
    };
    for variant in &variants {
        let doc = image_page_with(
            b"q 100 0 0 100 0 0 cm /Im Do Q",
            variant_image(variant, 600),
        );
        let whole = doc
            .render(0, 1.0, RenderOptions::default())
            .expect("render");
        let (small, stats) = doc.render_with_stats(0, 1.0, RenderOptions::default(), reduced);
        let small = small.expect("render reduced");
        assert!(
            stats.image_working_peak > 0,
            "{}: not reduced",
            variant.name
        );
        for (x, y) in [(25, 25), (75, 25), (25, 75), (75, 75)] {
            let (a, b) = (pixel(&whole, x, y), pixel(&small, x, y));
            let close = a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 3);
            assert!(
                close,
                "{} at ({x}, {y}): whole {a:?}, reduced {b:?}",
                variant.name
            );
        }
    }
}

#[test]
fn a_wide_image_is_reduced_within_a_bounded_row() {
    // The follow-up review: a row is decoded, converted and summed whole,
    // so its buffers grew with the image's width (2^25 samples wide, 20
    // rows: 108 MB gray, 208 MB Indexed). Rows of the reduced path have at
    // most 2^20 samples, and every buffer counts in the working set: the
    // row, what the inflater holds, the components and an Indexed image's
    // RGB. These images are 520 rows high (545 MB of samples, decoded as
    // they inflate).
    let (width, height) = (1u32 << 20, 520);
    let draw = |width: u32, cs: &str| {
        let row = vec![1u8; width as usize];
        let data = deflate_rows(height, |_| row.clone());
        let doc = image_page(stream(
            &format!(
                "/Type /XObject /Subtype /Image /Width {width} /Height {height} /BitsPerComponent 8 /ColorSpace {cs} /Filter /FlateDecode"
            ),
            &data,
        ));
        doc.render_with_stats(0, 1.0, RenderOptions::default(), RenderBudgets::default())
    };
    let palette = "[/Indexed /DeviceRGB 1 <000000ff0000>]";
    for (cs, colour, most) in [
        ("/DeviceGray", [1, 1, 1], 8 << 20),
        (palette, [255, 0, 0], 12 << 20),
    ] {
        let (image, stats) = draw(width, cs);
        let image = image.expect("render");
        let [r, g, b] = pixel(&image, 50, 50);
        assert!(
            r.abs_diff(colour[0]) <= 2 && g.abs_diff(colour[1]) <= 2 && b.abs_diff(colour[2]) <= 2,
            "{cs}: {:?}",
            [r, g, b]
        );
        assert!(
            stats.image_working_peak > width as usize && stats.image_working_peak < most,
            "{cs}: {stats:?}"
        );
    }
    // A row wider than that is refused before anything is decoded.
    assert!(matches!(
        draw(width + 8, "/DeviceGray").0,
        Err(ViewError::TooComplex(_))
    ));
}
