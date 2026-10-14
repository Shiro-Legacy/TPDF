// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent metrics identity/budget and shared text-accumulator review.
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::font_metrics::{AdvanceOrigin, FontAdvance, HorizontalMetrics};
use tandem_pdf_engine::font_resources::{
    FontResourceErrorKind, FontResourceLimitKind, FontResourceLimits, FontResourceLoader,
};
use tandem_pdf_engine::object::ObjectId;
use tandem_pdf_engine::simple_encoding::{
    Difference, EncodingLimits, NamedEncoding, SimpleEncoding,
};
use tandem_pdf_engine::text_decode::{TextDecodeErrorKind, TextDecodeLimits, decode_encoded_text};
fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}
fn document(extra: &[String]) -> Vec<u8> {
    let mut bodies=vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string(),"<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /Font << /A 4 0 R /Alias 4 0 R /B 5 0 R >> >> >>".to_string(),"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>".to_string()];
    bodies.extend_from_slice(extra);
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in &offsets[1..] {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    bytes
}
#[test]
fn full_byte_range_preserves_signed_advances_without_missing_width_substitution() {
    let input: Vec<_> = (0..256)
        .map(|code| {
            FontAdvance::new(if code == 0 {
                -0.
            } else {
                (f64::from(code) - 128.) / 4.
            })
            .unwrap()
        })
        .collect();
    let metrics =
        HorizontalMetrics::from_widths(0, 255, &input, Some(FontAdvance::new(999.).unwrap()))
            .unwrap();
    for code in 0..=255 {
        assert_eq!(
            metrics.lookup(code).advance().value().to_bits(),
            input[usize::from(code)].value().to_bits()
        );
        assert_eq!(metrics.lookup(code).origin(), AdvanceOrigin::WidthsEntry);
    }
    drop(input);
    assert!(metrics.lookup(0).advance().value().is_sign_negative());
}
#[test]
fn shared_widths_array_does_not_merge_font_specific_missing_width_provenance() {
    let bytes=document(&[
        "<< /Subtype /Type1 /FirstChar 65 /LastChar 67 /Widths 6 0 R /FontDescriptor << /MissingWidth -0.0 >> >>".into(),
        "<< /Subtype /Type1 /FirstChar 65 /LastChar 67 /Widths 6 0 R /FontDescriptor << /MissingWidth 23 >> >>".into(),
        "[0 -10.5 100.25]".into(),
    ]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    let a = loader
        .load_metrics(id(3), b"A")
        .unwrap()
        .metrics()
        .missing();
    assert!(a.advance().value().is_sign_negative());
    assert_eq!(a.origin(), AdvanceOrigin::DeclaredMissingWidth);
    let b = loader
        .load_metrics(id(3), b"B")
        .unwrap()
        .metrics()
        .missing();
    assert_eq!(b.advance().value(), 23.);
    assert_eq!(b.origin(), AdvanceOrigin::DeclaredMissingWidth);
    assert_eq!(loader.usage().metrics, 2);
    assert_eq!(loader.usage().width_entries, 6);
    assert_eq!(loader.usage().retained_width_entries, 6);
    loader.load_metrics(id(3), b"Alias").unwrap();
    assert_eq!(loader.usage().width_entries, 6);
    assert_eq!(loader.usage().metric_attempts, 3);
}
#[test]
fn numeric_width_metadata_exhaustion_retains_exact_index_and_no_partial_cache() {
    let bytes = document(&[
        "<< /Subtype /Type1 /FirstChar 65 /LastChar 66 /Widths [1 2] >>".into(),
        "null".into(),
    ]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut loader = FontResourceLoader::new(
        &doc,
        FontResourceLimits {
            max_metadata_items: 6,
            ..FontResourceLimits::default()
        },
    )
    .unwrap();
    let error = loader.load_metrics(id(3), b"A").unwrap_err();
    assert_eq!(error.width_index, Some(1));
    assert!(matches!(
        error.kind,
        FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::MetadataItems)
    ));
    assert_eq!(loader.usage().metrics, 0);
    assert_eq!(loader.usage().retained_width_entries, 0);
    assert_eq!(loader.usage().width_entries, 2);
}
#[test]
fn failed_metrics_do_not_poison_cached_unicode_mapping() {
    let bytes=document(&["<< /Subtype /Type1 /Encoding /WinAnsiEncoding /FirstChar 0 /LastChar 1 /Widths [1 /Bad] >>".into(),"null".into()]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    let before = loader.load(id(3), b"A").unwrap() as *const _;
    assert_eq!(
        loader.load_metrics(id(3), b"A").unwrap_err().width_index,
        Some(1)
    );
    let after = loader.load(id(3), b"Alias").unwrap() as *const _;
    assert_eq!(before, after);
    assert_eq!(loader.usage().fonts, 1);
    assert_eq!(loader.usage().metrics, 0);
}
#[test]
fn every_defined_simple_slot_has_one_source_byte_and_complete_utf8_span() {
    for named in [
        NamedEncoding::WinAnsi,
        NamedEncoding::MacRoman,
        NamedEncoding::Standard,
        NamedEncoding::MacExpert,
    ] {
        let base = SimpleEncoding::from_named(named);
        let input: Vec<_> = (0..=255)
            .filter(|&code| {
                matches!(
                    base.lookup_unicode(code),
                    tandem_pdf_engine::simple_encoding::GlyphUnicode::Unicode(_)
                )
            })
            .collect();
        let decoded = decode_encoded_text(&base, &input, TextDecodeLimits::default()).unwrap();
        assert_eq!(decoded.codes().len(), input.len());
        assert_eq!(decoded.scalar_count(), input.len());
        let mut end = 0;
        for (index, code) in decoded.codes().iter().enumerate() {
            assert_eq!(code.source_span(), index..index + 1);
            assert_eq!(code.code().to_bytes(), [input[index]]);
            assert_eq!(code.utf8_span().start, end);
            assert!(decoded.text().is_char_boundary(code.utf8_span().end));
            end = code.utf8_span().end;
        }
        assert_eq!(end, decoded.text().len());
    }
}
#[test]
fn opaque_unknown_names_remain_distinct_from_explicit_notdef_after_valid_prefix() {
    let base = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let changed = base
        .with_differences(
            &[
                Difference::Code(0),
                Difference::Name(b"A\0B"),
                Difference::Name(b".notdef"),
            ],
            EncodingLimits::default(),
        )
        .unwrap();
    for (input, expected) in [
        (&[0x80, 0][..], TextDecodeErrorKind::UnknownGlyphName(0)),
        (
            &[0x80, 1][..],
            TextDecodeErrorKind::UndefinedEncodingCode(1),
        ),
    ] {
        let error = decode_encoded_text(&changed, input, TextDecodeLimits::default()).unwrap_err();
        assert_eq!(error.offset, 1);
        assert_eq!(error.kind, expected);
    }
}
