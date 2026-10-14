use tandem_pdf_engine::cmap::CodeWidth;
use tandem_pdf_engine::simple_encoding::{
    Difference, EncodingLimits, NamedEncoding, SimpleEncoding,
};
use tandem_pdf_engine::text_decode::{
    TextDecodeErrorKind, TextDecodeLimitKind, TextDecodeLimits, decode_encoded_text,
};
#[test]
fn exact_pdf_characters_and_spans_keep_input_borrow() {
    let bytes = [0xff, 0x41, 0x80, 0xa0, 0xad, 0x7f, 0xff];
    let input = &bytes[1..6];
    let decoded = {
        let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
        decode_encoded_text(&encoding, input, TextDecodeLimits::default()).unwrap()
    };
    assert_eq!(decoded.text(), "A€ -•");
    assert_eq!(decoded.input().as_ptr(), input.as_ptr());
    for (i, (span, code)) in [
        (0..1, 0x41),
        (1..4, 0x80),
        (4..5, 0xa0),
        (5..6, 0xad),
        (6..9, 0x7f),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(decoded.codes()[i].source_span(), i..i + 1);
        assert_eq!(decoded.codes()[i].utf8_span(), span);
        assert_eq!(decoded.codes()[i].code().value(), code);
        assert_eq!(decoded.codes()[i].code().width(), CodeWidth::One);
    }
}
#[test]
fn undefined_and_unknown_are_different_and_no_partial_text_escapes() {
    let base = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let encoding = base
        .with_differences(
            &[
                Difference::Code(66),
                Difference::Name(b".notdef"),
                Difference::Name(b"uni0041"),
            ],
            EncodingLimits::default(),
        )
        .unwrap();
    let undefined = decode_encoded_text(&encoding, b"AB", TextDecodeLimits::default()).unwrap_err();
    assert_eq!(undefined.offset, 1);
    assert_eq!(
        undefined.kind,
        TextDecodeErrorKind::UndefinedEncodingCode(66)
    );
    let unknown = decode_encoded_text(&encoding, b"AC", TextDecodeLimits::default()).unwrap_err();
    assert_eq!(unknown.offset, 1);
    assert_eq!(unknown.kind, TextDecodeErrorKind::UnknownGlyphName(67));
    assert_eq!(
        decode_encoded_text(&base, b"ABC", TextDecodeLimits::default())
            .unwrap()
            .text(),
        "ABC"
    );
}
#[test]
fn exact_caps_and_repeated_multibyte_output_are_charged() {
    let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let exact = TextDecodeLimits {
        max_input_bytes: 3,
        max_codes: 3,
        max_scalars: 3,
        max_utf8_bytes: 9,
    };
    let decoded = decode_encoded_text(&encoding, &[0x80; 3], exact).unwrap();
    assert_eq!(decoded.text(), "€€€");
    for (limits, kind, offset) in [
        (
            TextDecodeLimits {
                max_input_bytes: 2,
                ..exact
            },
            TextDecodeLimitKind::InputBytes,
            0,
        ),
        (
            TextDecodeLimits {
                max_codes: 2,
                ..exact
            },
            TextDecodeLimitKind::Codes,
            2,
        ),
        (
            TextDecodeLimits {
                max_scalars: 2,
                ..exact
            },
            TextDecodeLimitKind::Scalars,
            2,
        ),
        (
            TextDecodeLimits {
                max_utf8_bytes: 8,
                ..exact
            },
            TextDecodeLimitKind::Utf8Bytes,
            2,
        ),
    ] {
        let err = decode_encoded_text(&encoding, &[0x80; 3], limits).unwrap_err();
        assert_eq!(err.offset, offset);
        assert_eq!(err.kind, TextDecodeErrorKind::LimitExceeded(kind));
    }
    let zero = TextDecodeLimits {
        max_input_bytes: 0,
        max_codes: 0,
        max_scalars: 0,
        max_utf8_bytes: 0,
    };
    assert_eq!(
        decode_encoded_text(&encoding, b"", zero).unwrap().text(),
        ""
    );
}
#[test]
fn registry_unicode_is_preserved_without_normalization() {
    let base = SimpleEncoding::from_named(NamedEncoding::Standard);
    let encoding = base
        .with_differences(
            &[
                Difference::Code(0),
                Difference::Name(b"fi"),
                Difference::Name(b"Aacute"),
            ],
            EncodingLimits::default(),
        )
        .unwrap();
    let decoded = decode_encoded_text(&encoding, &[0, 1], TextDecodeLimits::default()).unwrap();
    assert_eq!(decoded.text(), "ﬁÁ");
    assert_eq!(decoded.scalar_count(), 2);
    assert_eq!(decoded.codes()[0].utf8_span(), 0..3);
}
fn pdf() -> Vec<u8> {
    let cmap = "begincmap 1 begincodespacerange <00><ff> endcodespacerange 1 beginbfchar <41><00660069> endbfchar endcmap";
    let objects=["<< /Type /Catalog /Pages 2 0 R >>".to_string(),"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Resources << /Font << /F 4 0 R /E 6 0 R >> >> >>".into(),"<< /Subtype /Type1 /ToUnicode 5 0 R /Encoding /WinAnsiEncoding >>".into(),format!("<< /Length {} >>\nstream\n{cmap}\nendstream",cmap.len()),"<< /Subtype /Type1 /Encoding /WinAnsiEncoding >>".into()];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, value) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{value}\nendobj\n", i + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}
#[test]
fn font_dispatch_keeps_tounicode_authoritative_and_no_font_borrow() {
    use tandem_pdf_engine::{
        document::{Document, DocumentLimits},
        font_resources::{FontResourceLimits, FontResourceLoader},
        font_text::decode_declared_text,
        object::ObjectId,
    };
    let raw = b"A";
    let decoded = {
        let bytes = pdf();
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
        let page = ObjectId::new(3, 0).unwrap();
        let font = loader.load(page, b"F").unwrap();
        let err = decode_declared_text(font, b"B", TextDecodeLimits::default()).unwrap_err();
        assert!(matches!(err.kind, TextDecodeErrorKind::MissingMapping(_)));
        let result = decode_declared_text(font, raw, TextDecodeLimits::default()).unwrap();
        assert_eq!(
            decode_declared_text(
                loader.load(page, b"E").unwrap(),
                raw,
                TextDecodeLimits::default()
            )
            .unwrap()
            .text(),
            "A"
        );
        result
    };
    assert_eq!(decoded.text(), "fi");
    assert_eq!(decoded.codes().len(), 1);
    assert_eq!(decoded.input().as_ptr(), raw.as_ptr());
}
