// SPDX-License-Identifier: AGPL-3.0-or-later
use miniz_oxide::deflate::compress_to_vec_zlib;
use std::sync::Arc;
use tandem_pdf_engine::{
    document::{Document, DocumentLimits},
    image_resources::{ImageAsset, ImageComponents, ImageErrorKind, ImageLimits, ImageLoader},
    object::ObjectId,
};
fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}
fn pdf(meta: &str, raw: &[u8], extra: &[Vec<u8>]) -> Vec<u8> {
    let mut stream = format!(
        "<< /Subtype /Image {meta} /Length {} >>\nstream\n",
        raw.len()
    )
    .into_bytes();
    stream.extend_from_slice(raw);
    stream.extend_from_slice(b"\nendstream");
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [] /Count 0 >>".to_vec(),
        stream,
    ];
    objects.extend_from_slice(extra);
    let mut output = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(output.len());
        output.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        output.extend_from_slice(body);
        output.extend_from_slice(b"\nendobj\n");
    }
    let start = output.len();
    output.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        output.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    output.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    output
}
const RGB: &str = "/Type /XObject /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB";
const GRAY: &str = "/Width 3 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray";
fn load(bytes: &[u8]) -> Arc<ImageAsset> {
    let d = Document::open(bytes, DocumentLimits::default()).unwrap();
    ImageLoader::new(&d, ImageLimits::default())
        .load(id(3))
        .unwrap()
}
fn refuses(meta: &str, raw: &[u8], extra: &[Vec<u8>]) {
    let bytes = pdf(meta, raw, extra);
    let d = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut l = ImageLoader::new(&d, ImageLimits::default());
    assert!(l.load(id(3)).is_err(), "{meta}");
    assert_eq!(l.usage().attempts, 1);
}
fn ascii85(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for group in bytes.chunks(4) {
        let mut word = [0; 4];
        word[..group.len()].copy_from_slice(group);
        let mut v = u32::from_be_bytes(word);
        let mut digits = [b'!'; 5];
        for d in digits.iter_mut().rev() {
            *d += (v % 85) as u8;
            v /= 85;
        }
        out.extend_from_slice(&digits[..group.len() + 1]);
    }
    out.extend_from_slice(b"~>");
    out
}
fn lzw_literals(bytes: &[u8]) -> Vec<u8> {
    assert!(bytes.len() < 200);
    let codes = std::iter::once(256u16)
        .chain(bytes.iter().map(|&b| u16::from(b)))
        .chain(std::iter::once(257));
    let mut out = Vec::new();
    let (mut buffer, mut n) = (0u32, 0);
    for c in codes {
        buffer = (buffer << 9) | u32::from(c);
        n += 9;
        while n >= 8 {
            n -= 8;
            out.push((buffer >> n) as u8);
        }
    }
    if n > 0 {
        out.push((buffer << (8 - n)) as u8);
    }
    out
}
#[test]
fn owned_gray_rgb_and_decode_inversion() {
    let gray = load(&pdf(&format!("{GRAY} /Decode [1 0]"), &[0, 127, 255], &[]));
    assert_eq!(gray.samples(), &[0, 127, 255]);
    assert_eq!(gray.components(), ImageComponents::Gray);
    assert_eq!(gray.pixel(0, 0), Some([255; 3]));
    assert_eq!(gray.pixel(0, 1), Some([128; 3]));
    assert_eq!(gray.pixel(0, 2), Some([0; 3]));
    assert_eq!(gray.pixel(0, 3), None);
    assert_eq!(gray.pixel(usize::MAX, 0), None);
    let rgb = load(&pdf(
        &format!("{RGB} /Decode [1 0 0 1 1 0]"),
        &[10, 20, 30],
        &[],
    ));
    assert_eq!(rgb.pixel(0, 0), Some([245, 20, 225]));
    assert_eq!(rgb.row_bytes(), 3);
    assert_eq!(rgb.object_id(), id(3));
    assert_eq!(rgb.decode_inverted(), &[true, false, true]);
}
#[test]
fn every_lossless_filter_and_chain_preserves_samples() {
    let data = [10, 20, 30];
    for (filter, raw) in [
        ("/FlateDecode", compress_to_vec_zlib(&data, 6)),
        ("/ASCIIHexDecode", b"0a141e>".to_vec()),
        ("/ASCII85Decode", ascii85(&data)),
        ("/RunLengthDecode", vec![2, 10, 20, 30, 128]),
        ("/LZWDecode", lzw_literals(&data)),
    ] {
        let a = load(&pdf(&format!("{RGB} /Filter {filter}"), &raw, &[]));
        assert_eq!(a.samples(), data);
        assert_eq!(a.filters().len(), 1);
    }
    for early in [0, 1] {
        let a = load(&pdf(
            &format!("{RGB} /Filter /LZWDecode /DecodeParms << /EarlyChange {early} >>"),
            &lzw_literals(&data),
            &[],
        ));
        assert_eq!(a.samples(), data);
    }
    let z = compress_to_vec_zlib(&data, 6);
    let hex = format!(
        "{}>",
        z.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let a = load(&pdf(
        &format!("{RGB} /Filter [/ASCIIHexDecode /FlateDecode] /DecodeParms [null null]"),
        hex.as_bytes(),
        &[],
    ));
    assert_eq!(a.samples(), data);
    assert_eq!(a.filters().len(), 2);
}
#[test]
fn png_and_tiff_prediction_match_image_rows() {
    let gray = "/Width 3 /Height 2 /BitsPerComponent 8 /ColorSpace /DeviceGray";
    for predictor in 10..=15 {
        let raw = compress_to_vec_zlib(&[0, 10, 20, 30, 2, 1, 2, 3], 6);
        let a = load(&pdf(
            &format!(
                "{gray} /Filter /FlateDecode /DecodeParms << /Predictor {predictor} /Colors 1 /Columns 3 /BitsPerComponent 8 >>"
            ),
            &raw,
            &[],
        ));
        assert_eq!(a.samples(), &[10, 20, 30, 11, 22, 33]);
    }
    let raw = compress_to_vec_zlib(&[10, 10, 10, 11, 11, 11], 6);
    let a = load(&pdf(
        &format!("{gray} /Filter /FlateDecode /DecodeParms << /Predictor 2 /Columns 3 >>"),
        &raw,
        &[],
    ));
    assert_eq!(a.samples(), &[10, 20, 30, 11, 22, 33]);
    let rgb = "/Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB";
    let raw = compress_to_vec_zlib(&[1, 10, 20, 30, 1, 2, 3], 6);
    let a = load(&pdf(
        &format!(
            "{rgb} /Filter /FlateDecode /DecodeParms << /Predictor 15 /Columns 2 /Colors 3 >>"
        ),
        &raw,
        &[],
    ));
    assert_eq!(a.samples(), &[10, 20, 30, 11, 22, 33]);
}
#[test]
fn indirect_metadata_is_metered_and_no_stream_metadata_is_admitted() {
    let meta = "/Width 4 0 R /Height 1 /BitsPerComponent 8 /ColorSpace 5 0 R /Decode 6 0 R /Filter 7 0 R /DecodeParms 8 0 R";
    let bytes = pdf(
        meta,
        b"0a141e>",
        &[
            b"1".to_vec(),
            b"/DeviceRGB".to_vec(),
            b"[1 0 0 1 0 1]".to_vec(),
            b"/ASCIIHexDecode".to_vec(),
            b"null".to_vec(),
        ],
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut loader = ImageLoader::new(&doc, ImageLimits::default());
    assert_eq!(loader.load(id(3)).unwrap().pixel(0, 0), Some([245, 20, 30]));
    assert_eq!(loader.usage().reference_hops, 5);
    refuses(
        &format!("{RGB} /Mask 4 0 R"),
        &[1, 2, 3],
        &[b"4 0 R".to_vec()],
    );
    refuses(
        &format!("{RGB} /SMask 4 0 R"),
        &[1, 2, 3],
        &[b"99 0 R".to_vec()],
    );
    refuses(
        "/Width 3 0 R /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
        &[1, 2, 3],
        &[],
    );
}
#[test]
fn unsupported_image_semantics_and_malformed_optionals_refuse() {
    for suffix in [
        "/ImageMask true",
        "/ImageMask 0",
        "/Interpolate true",
        "/Interpolate /false",
        "/Mask []",
        "/SMask /None",
        "/SMaskInData 1",
        "/SMaskInData false",
        "/Alternates []",
        "/OPI <<>>",
        "/FFilter /FlateDecode",
        "/Matte [0 0 0]",
        "/Intent /Custom",
        "/Intent 1",
    ] {
        refuses(&format!("{RGB} {suffix}"), &[1, 2, 3], &[]);
    }
    let accepted = load(&pdf(
        &format!("{RGB} /ImageMask false /Interpolate false /Mask null /SMask null /SMaskInData 0"),
        &[1, 2, 3],
        &[],
    ));
    assert_eq!(accepted.samples(), &[1, 2, 3]);
}
#[test]
fn malformed_dimensions_color_and_decode_refuse() {
    for (w, h, bpc, color) in [
        ("0", "1", "8", "/DeviceRGB"),
        ("-1", "1", "8", "/DeviceRGB"),
        ("1.0", "1", "8", "/DeviceRGB"),
        ("16385", "1", "8", "/DeviceRGB"),
        ("1", "1", "16", "/DeviceRGB"),
        ("1", "1", "8", "/RGB"),
        ("1", "1", "8", "[/DeviceRGB]"),
        ("1", "1", "8", "/DeviceCMYK"),
    ] {
        refuses(
            &format!("/Width {w} /Height {h} /BitsPerComponent {bpc} /ColorSpace {color}"),
            &[1, 2, 3],
            &[],
        );
    }
    for decode in [
        "[0 1]",
        "[0 2 0 1 0 1]",
        "[0 0 0 1 0 1]",
        "true",
        "[null 1 0 1 0 1]",
    ] {
        refuses(&format!("{RGB} /Decode {decode}"), &[1, 2, 3], &[]);
    }
    refuses(
        "/Type /Other /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
        &[1, 2, 3],
        &[],
    );
}
#[test]
fn truncated_extra_and_malformed_encoded_samples_never_publish() {
    for bytes in [&[1, 2][..], &[1, 2, 3, 4][..]] {
        refuses(RGB, bytes, &[]);
    }
    for (filter, data) in [
        ("/ASCIIHexDecode", b"010203".as_slice()),
        ("/ASCII85Decode", b"!!"),
        ("/FlateDecode", b"bad"),
        ("/RunLengthDecode", b"\x02\x01\x80"),
        ("/LZWDecode", b"bad"),
        ("/DCTDecode", b"bad"),
        ("/Fl", b"bad"),
    ] {
        refuses(&format!("{RGB} /Filter {filter}"), data, &[]);
    }
}
#[test]
fn predictor_metadata_must_match_and_unknown_parameters_refuse() {
    for parms in [
        "<< /Predictor 15 /Columns 2 /Colors 3 >>",
        "<< /Predictor 2 /Columns 1 /Colors 1 >>",
        "<< /Predictor 2 /Columns 1 /Colors 3 /BitsPerComponent 4 >>",
        "<< /Predictor 1 /Colors 0 >>",
        "<< /Predictor 1 /Columns 0 >>",
        "<< /Predictor 99 /Columns 1 /Colors 3 >>",
        "<< /Bogus 1 >>",
        "<< /Predictor true >>",
        "[]",
    ] {
        refuses(
            &format!("{RGB} /Filter /FlateDecode /DecodeParms {parms}"),
            &compress_to_vec_zlib(&[0, 1, 2, 3], 6),
            &[],
        );
    }
    refuses(&format!("{RGB} /DecodeParms <<>>"), &[1, 2, 3], &[]);
    refuses(
        &format!("{RGB} /Filter /ASCIIHexDecode /DecodeParms << /Predictor 1 >>"),
        b"010203>",
        &[],
    );
}
#[test]
fn caches_own_bytes_recheck_tighter_limits_and_do_not_recharge_decode() {
    let bytes = pdf(GRAY, &[1, 2, 3], &[]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut l = ImageLoader::new(&doc, ImageLimits::default());
    let first = l.load(id(3)).unwrap();
    let before = l.usage();
    let second = l.load(id(3)).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    let after = l.usage();
    assert_eq!(after.encoded_bytes, before.encoded_bytes);
    assert_eq!(after.decoded_work_bytes, before.decoded_work_bytes);
    assert_eq!(after.allocated_bytes, before.allocated_bytes);
    assert_eq!(after.attempts, before.attempts + 1);
    assert!(after.lookup_work > before.lookup_work);
    l.begin_page(ImageLimits {
        max_dimension: 2,
        ..ImageLimits::default()
    });
    assert!(l.load(id(3)).is_err());
    l.begin_page(ImageLimits::default());
    assert!(l.load(id(3)).is_ok());
    drop(l);
    drop(doc);
    drop(bytes);
    assert_eq!(first.pixel(0, 1), Some([2; 3]));
}
#[test]
fn per_page_reset_never_refuels_session_counters() {
    let bytes = pdf(RGB, &[1, 2, 3], &[]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut l = ImageLoader::new(
        &doc,
        ImageLimits {
            max_attempts: 2,
            ..ImageLimits::default()
        },
    );
    l.begin_page(ImageLimits {
        max_attempts: 1,
        ..ImageLimits::default()
    });
    l.load(id(3)).unwrap();
    assert!(l.load(id(3)).is_err());
    l.begin_page(ImageLimits {
        max_attempts: 1,
        ..ImageLimits::default()
    });
    l.load(id(3)).unwrap();
    l.begin_page(ImageLimits::default());
    assert!(l.load(id(3)).is_err());
    assert_eq!(l.usage().attempts, 2);
}
#[test]
fn failed_decode_consumes_reserved_work_and_retries_do_not_reset() {
    let bytes = pdf(&format!("{RGB} /Filter /FlateDecode"), b"bad", &[]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut l = ImageLoader::new(
        &doc,
        ImageLimits {
            max_decoded_work_bytes: 32,
            max_decode_bytes: 16,
            max_stage_bytes: 16,
            ..ImageLimits::default()
        },
    );
    for attempt in 1..=2 {
        assert!(matches!(
            l.load(id(3)).unwrap_err().kind,
            ImageErrorKind::Decode(_)
        ));
        assert_eq!(l.usage().decoded_work_bytes, attempt * 16);
        assert_eq!(l.usage().decoded_rows, attempt);
        assert_eq!(l.usage().retained_sample_bytes, attempt * 3);
    }
    let before = l.usage();
    assert!(l.load(id(3)).is_err());
    assert_eq!(l.usage().decoded_work_bytes, before.decoded_work_bytes);
    assert_eq!(l.usage().attempts, 3);
}
#[test]
fn exact_and_one_less_cumulative_counters() {
    let bytes = pdf(
        "/Width 4 0 R /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Filter /ASCIIHexDecode",
        b"010203>",
        &[b"1".to_vec()],
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut l = ImageLoader::new(&doc, ImageLimits::default());
    l.load(id(3)).unwrap();
    let u = l.usage();
    macro_rules! check {
        ($field:ident,$cap:ident) => {{
            let n = u.$field;
            assert!(n > 0);
            let mut limits = ImageLimits::default();
            limits.$cap = n;
            assert!(
                ImageLoader::new(&doc, limits).load(id(3)).is_ok(),
                stringify!($cap)
            );
            limits.$cap = n - 1;
            assert!(
                ImageLoader::new(&doc, limits).load(id(3)).is_err(),
                stringify!($cap)
            );
        }};
    }
    check!(attempts, max_attempts);
    check!(unique_images, max_unique_images);
    check!(metadata_items, max_metadata_items);
    check!(reference_hops, max_reference_hops);
    check!(lookup_work, max_lookup_work);
    check!(encoded_bytes, max_encoded_bytes);
    check!(decoded_work_bytes, max_decoded_work_bytes);
    check!(decoded_rows, max_decoded_rows);
    check!(decoded_pixels, max_decoded_pixels);
    check!(retained_sample_bytes, max_retained_sample_bytes);
    check!(filter_specs, max_filter_specs);
    // Identity has a known exact decode-allocation reservation, with no adaptive
    // large filter output ceiling; assert its last byte of admission independently.
    let bytes = pdf(RGB, &[1, 2, 3], &[]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut l = ImageLoader::new(&doc, ImageLimits::default());
    l.load(id(3)).unwrap();
    let n = l.usage().allocated_bytes;
    assert!(
        ImageLoader::new(
            &doc,
            ImageLimits {
                max_allocated_bytes: n,
                ..ImageLimits::default()
            }
        )
        .load(id(3))
        .is_ok()
    );
    assert!(
        ImageLoader::new(
            &doc,
            ImageLimits {
                max_allocated_bytes: n - 1,
                ..ImageLimits::default()
            }
        )
        .load(id(3))
        .is_err()
    );
}

#[test]
fn external_file_stream_is_refused_before_image_loading() {
    let bytes = pdf(&format!("{RGB} /F (remote)"), &[1, 2, 3], &[]);
    let error = Document::open(&bytes, DocumentLimits::default())
        .err()
        .unwrap();
    assert!(matches!(
        error.kind,
        tandem_pdf_engine::document::DocumentErrorKind::Unsupported("external-file streams")
    ));
}

const INDEXED: &str =
    "/Width 4 /Height 1 /BitsPerComponent 8 /ColorSpace [/Indexed /DeviceRGB 1 <ff000000ff00>]";
#[test]
fn indexed_palette_color_key_compares_raw_samples_before_clipping() {
    let asset = load(&pdf(
        &format!("{INDEXED} /Decode [0 255] /Mask [1 1]"),
        &[0, 1, 2, 255],
        &[],
    ));
    assert_eq!(asset.components(), ImageComponents::IndexedRgb);
    assert_eq!(asset.pixel_rgba(0, 0), Some([255, 0, 0, 255]));
    assert_eq!(asset.pixel_rgba(0, 1), Some([0, 255, 0, 0]));
    assert_eq!(asset.pixel_rgba(0, 2), Some([0, 255, 0, 255]));
    assert_eq!(asset.pixel_rgba(0, 3), Some([0, 255, 0, 255]));
    assert_eq!(asset.pixel_rgba(1, 0), None);
}
#[test]
fn indirect_string_and_unfiltered_stream_palettes_agree() {
    let meta = "/Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace [/Indexed /DeviceRGB 0 4 0 R]";
    let a = load(&pdf(meta, &[0], &[b"<123456>".to_vec()]));
    let b = load(&pdf(
        meta,
        &[0],
        &[b"<< /Length 3 >>\nstream\n\x12\x34\x56\nendstream".to_vec()],
    ));
    assert_eq!(a.pixel(0, 0), Some([18, 52, 86]));
    assert_eq!(a.pixel(0, 0), b.pixel(0, 0));
}
#[test]
fn indexed_palette_mask_and_decode_errors_fail_closed() {
    for extra in [
        "/Decode [0 1]",
        "/Decode [255 0]",
        "/Mask [2 1]",
        "/Mask [-1 1]",
        "/Mask [1.0 1]",
        "/Mask [0 256]",
        "/Mask [0]",
        "/Mask [0 1 2 3]",
    ] {
        refuses(&format!("{INDEXED} {extra}"), &[0, 1, 2, 3], &[]);
    }
    for colors in [
        "[/Indexed /DeviceRGB 1 <ff0000>]",
        "[/Indexed /DeviceRGB 0 <ff000000>]",
        "[/Indexed /DeviceRGB 256 <ff0000>]",
        "[/Indexed /DeviceCMYK 0 <00000000>]",
        "[/Indexed /DeviceRGB 1 4 0 R]",
    ] {
        refuses(
            &format!("/Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace {colors}"),
            &[0],
            &[b"4 0 R".to_vec()],
        );
    }
}
#[test]
fn indexed_palette_retention_is_charged_and_failed_load_is_not_cached() {
    let bytes = pdf(INDEXED, &[0, 1, 0, 1], &[]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut loader = ImageLoader::new(
        &doc,
        ImageLimits {
            max_retained_sample_bytes: 9,
            ..Default::default()
        },
    );
    assert!(loader.load(id(3)).is_err());
    let first = loader.usage();
    assert_eq!(first.retained_sample_bytes, 6);
    assert!(loader.load(id(3)).is_err());
    assert!(loader.usage().attempts > first.attempts);
    let mut loader = ImageLoader::new(
        &doc,
        ImageLimits {
            max_retained_sample_bytes: 10,
            ..Default::default()
        },
    );
    loader.load(id(3)).unwrap();
    assert_eq!(loader.usage().retained_sample_bytes, 10);
}
#[test]
fn standard_image_intents_are_accepted() {
    for intent in [
        "AbsoluteColorimetric",
        "RelativeColorimetric",
        "Saturation",
        "Perceptual",
    ] {
        let image = load(&pdf(&format!("{RGB} /Intent /{intent}"), &[1, 2, 3], &[]));
        assert_eq!(image.pixel(0, 0), Some([1, 2, 3]));
    }
}
const CAL: &str = "[/CalRGB << /WhitePoint [0.95043 1 1.09] /Gamma [2.2 2.2 2.2] \
    /Matrix [0.41239 0.21264 0.01933 0.35758 0.71517 0.11919 0.18045 0.07218 0.9504] >>]";
#[test]
fn cal_rgb_matches_reference_conversion_within_one_level() {
    let raw: Vec<u8> = (0..16u8)
        .flat_map(|i| [i * 17, 255 - i * 17, ((u16::from(i) * 53) % 256) as u8])
        .collect();
    // MuPDF 1.28.1 (LittleCMS) render of the same 16x1 CalRGB image.
    let reference = [
        0x00, 0xff, 0x00, 0x09, 0xef, 0x32, 0x1d, 0xde, 0x6a, 0x30, 0xcd, 0xa0, 0x42, 0xbc, 0xd5,
        0x54, 0xac, 0x02, 0x66, 0x9a, 0x3c, 0x78, 0x89, 0x74, 0x89, 0x78, 0xaa, 0x9a, 0x66, 0xde,
        0xab, 0x54, 0x09, 0xbc, 0x42, 0x45, 0xcd, 0x30, 0x7d, 0xde, 0x1c, 0xb2, 0xee, 0x08, 0xe7,
        0xff, 0x00, 0x14,
    ];
    let image = load(&pdf(
        &format!("/Width 16 /Height 1 /BitsPerComponent 8 /ColorSpace {CAL} /Intent /Perceptual"),
        &raw,
        &[],
    ));
    assert_eq!(image.samples(), raw.as_slice());
    for column in 0..16 {
        let pixel = image.pixel(0, column).unwrap();
        for c in 0..3 {
            let expected: u8 = reference[column * 3 + c];
            assert!(pixel[c].abs_diff(expected) <= 1, "{column}/{c}: {pixel:?}");
        }
    }
    // Defaults: identity matrix and unit gamma with a D65 white point are
    // linear sRGB primaries, so mid-gray is sRGB-encoded rather than copied.
    let linear = load(&pdf(
        "/Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace [/CalRGB << /WhitePoint [0.95047 1 1.08883] /Matrix [0.4124564 0.2126729 0.0193339 0.3575761 0.7151522 0.1191920 0.1804375 0.0721750 0.9503041] >>]",
        &[0, 128, 255],
        &[],
    ));
    let [r, g, b] = linear.pixel(0, 0).unwrap();
    assert_eq!((r, b), (0, 255));
    assert!(g.abs_diff(188) <= 1, "{g}");
}
#[test]
fn malformed_or_unsupported_cal_rgb_refuses() {
    for space in [
        "[/CalRGB]",
        "[/CalRGB << >>]",
        "[/CalRGB << /WhitePoint [0.95 0.9 1.09] >>]",
        "[/CalRGB << /WhitePoint [0 1 1.09] >>]",
        "[/CalRGB << /WhitePoint [0.95 1 1.09] /BlackPoint [0.1 0 0] >>]",
        "[/CalRGB << /WhitePoint [0.95 1 1.09] /Gamma [0 1 1] >>]",
        "[/CalRGB << /WhitePoint [0.95 1 1.09] /Gamma [1 1] >>]",
        "[/CalRGB << /WhitePoint [0.95 1 1.09] /Matrix [1 0 0 0 1 0 0 0] >>]",
        "[/CalRGB << /WhitePoint [0.95 1 /One] >>]",
        "[/CalRGB << /WhitePoint [0.95 1 1.09] >> 1]",
    ] {
        refuses(
            &format!("/Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace {space}"),
            &[1, 2, 3],
            &[],
        );
    }
}
