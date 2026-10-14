use tandem_pdf_engine::builtin_metrics::{BuiltinFace, BuiltinMetrics};
use tandem_pdf_engine::cmap::{CMapLimits, parse_to_unicode};
use tandem_pdf_engine::font_metrics::{AdvanceOrigin, FontAdvance, HorizontalMetrics};
use tandem_pdf_engine::simple_encoding::{
    Difference, EncodingLimits, NamedEncoding, SimpleEncoding,
};
use tandem_pdf_engine::text_decode::{TextDecodeErrorKind, TextDecodeLimits};
use tandem_pdf_engine::text_font::*;

fn widths() -> HorizontalMetrics {
    HorizontalMetrics::from_widths(
        65,
        65,
        &[FontAdvance::new(-12.5).unwrap()],
        Some(FontAdvance::new(37.25).unwrap()),
    )
    .unwrap()
}
#[test]
fn mapping_and_advance_are_independent_and_preserve_provenance() {
    let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let metrics = widths();
    let font = TextFont::declared(UnicodeSource::Encoding(&encoding), &metrics).unwrap();
    assert_eq!(
        font.decode(b"AB", TextDecodeLimits::default())
            .unwrap()
            .text(),
        "AB"
    );
    assert_eq!(
        font.advance(65).unwrap(),
        (
            FontAdvance::new(-12.5).unwrap(),
            TextMetricOrigin::Declared(AdvanceOrigin::WidthsEntry)
        )
    );
    assert_eq!(
        font.advance(66).unwrap(),
        (
            FontAdvance::new(37.25).unwrap(),
            TextMetricOrigin::Declared(AdvanceOrigin::DeclaredMissingWidth)
        )
    );
    let encoding = encoding
        .with_differences(
            &[Difference::Code(65), Difference::Name(b"unregistered")],
            EncodingLimits::default(),
        )
        .unwrap();
    let font = TextFont::declared(UnicodeSource::Encoding(&encoding), &metrics).unwrap();
    assert!(matches!(
        font.decode(b"A", TextDecodeLimits::default())
            .unwrap_err()
            .kind,
        TextDecodeErrorKind::UnknownGlyphName(65)
    ));
    assert_eq!(font.advance(65).unwrap().0.value(), -12.5);
}
#[test]
fn builtin_unicode_override_does_not_change_advance_or_fallback() {
    let metrics = BuiltinMetrics::from_encoding(
        BuiltinFace::Helvetica,
        SimpleEncoding::from_named(NamedEncoding::WinAnsi),
    );
    let map = parse_to_unicode(b"begincmap 1 begincodespacerange <00> <ff> endcodespacerange 1 beginbfchar <41> <00660069> endbfchar endcmap", CMapLimits::default()).unwrap();
    let font = TextFont::builtin(&metrics, Some(&map)).unwrap();
    let decoded = font.decode(b"A", TextDecodeLimits::default()).unwrap();
    assert_eq!(decoded.text(), "fi");
    assert_eq!(decoded.codes().len(), 1);
    assert_eq!(font.advance(65).unwrap().0.value(), 667.0);
    assert!(matches!(
        font.decode(b"B", TextDecodeLimits::default())
            .unwrap_err()
            .kind,
        TextDecodeErrorKind::MissingMapping(_)
    ));
    assert_eq!(font.unicode_kind(), TextUnicodeKind::ToUnicode);
    assert_eq!(
        font.metric_kind(),
        TextMetricKind::Builtin(BuiltinFace::Helvetica)
    );
}
#[test]
fn wider_or_vertical_maps_are_not_simple_font_bindings() {
    let metrics = widths();
    for source in [
        b"begincmap 1 begincodespacerange <0000> <ffff> endcodespacerange endcmap".as_slice(),
        b"begincmap /WMode 1 def 1 begincodespacerange <00> <ff> endcodespacerange endcmap",
    ] {
        let map = parse_to_unicode(source, CMapLimits::default()).unwrap();
        assert_eq!(
            TextFont::declared(UnicodeSource::ToUnicode(&map), &metrics).unwrap_err(),
            TextFontError::UnsupportedMapping
        );
    }
}
#[test]
fn registry_owns_byte_names_once_and_borrows_font_policy() {
    let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let metrics = widths();
    let font = TextFont::declared(UnicodeSource::Encoding(&encoding), &metrics).unwrap();
    let fonts = {
        let name = vec![0xff, b'F'];
        TextFonts::new(&[(&name, font), (b"", font)], TextFontLimits::default()).unwrap()
    };
    assert_eq!(fonts.len(), 2);
    let (id, got) = fonts.get(&[0xff, b'F']).unwrap();
    assert_eq!(id.index(), 0);
    assert_eq!(fonts.name(id), Some([0xff, b'F'].as_slice()));
    assert!(std::ptr::eq(got, fonts.by_id(id).unwrap()));
    assert!(fonts.get(b"unknown").is_none());
    assert_eq!(
        TextFonts::new(&[(b"F", font), (b"F", font)], TextFontLimits::default()).unwrap_err(),
        TextFontError::DuplicateName
    );
    assert_eq!(
        TextFonts::new(&[(b"F\0", font)], TextFontLimits::default()).unwrap_err(),
        TextFontError::NameContainsNul
    );
    assert!(!fonts.is_empty());
}
#[test]
fn every_registry_limit_has_an_exact_edge() {
    let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let metrics = widths();
    let font = TextFont::declared(UnicodeSource::Encoding(&encoding), &metrics).unwrap();
    let entries = [(b"AB".as_slice(), font), (b"C".as_slice(), font)];
    let measured = TextFonts::new(&entries, TextFontLimits::default())
        .unwrap()
        .capacity_bytes();
    let exact = TextFontLimits {
        max_fonts: 2,
        max_name_bytes: 2,
        max_total_name_bytes: 3,
        max_capacity_bytes: measured,
    };
    assert!(TextFonts::new(&entries, exact).is_ok());
    for (limits, kind) in [
        (
            TextFontLimits {
                max_fonts: 1,
                ..exact
            },
            TextFontLimitKind::Fonts,
        ),
        (
            TextFontLimits {
                max_name_bytes: 1,
                ..exact
            },
            TextFontLimitKind::NameBytes,
        ),
        (
            TextFontLimits {
                max_total_name_bytes: 2,
                ..exact
            },
            TextFontLimitKind::TotalNameBytes,
        ),
        (
            TextFontLimits {
                max_capacity_bytes: measured - 1,
                ..exact
            },
            TextFontLimitKind::CapacityBytes,
        ),
    ] {
        assert_eq!(
            TextFonts::new(&entries, limits).unwrap_err(),
            TextFontError::LimitExceeded(kind)
        );
    }
    assert!(
        TextFonts::new(
            &[],
            TextFontLimits {
                max_fonts: 0,
                max_name_bytes: 0,
                max_total_name_bytes: 0,
                max_capacity_bytes: 0
            }
        )
        .unwrap()
        .is_empty()
    );
}
#[test]
fn builtin_undefined_and_missing_are_not_zero_widths() {
    let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi)
        .with_differences(
            &[
                Difference::Code(65),
                Difference::Name(b".notdef"),
                Difference::Name(b"unregistered"),
            ],
            EncodingLimits::default(),
        )
        .unwrap();
    let metrics = BuiltinMetrics::from_encoding(BuiltinFace::Helvetica, encoding);
    let font = TextFont::builtin(&metrics, None).unwrap();
    assert_eq!(
        font.advance(65).unwrap_err(),
        TextFontError::UndefinedBuiltinCode(65)
    );
    assert_eq!(
        font.advance(66).unwrap_err(),
        TextFontError::MissingBuiltinMetric(66)
    );
}
