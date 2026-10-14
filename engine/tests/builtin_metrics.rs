use tandem_pdf_engine::{
    builtin_metrics::*,
    simple_encoding::{Difference, EncodingLimits, NamedEncoding, SimpleEncoding},
};
#[test]
fn exact_faces_and_verified_named_advances() {
    assert_eq!(
        BuiltinFace::from_pdf_name(b"Helvetica"),
        Some(BuiltinFace::Helvetica)
    );
    assert_eq!(
        BuiltinFace::from_pdf_name(b"Times-Roman"),
        Some(BuiltinFace::TimesRoman)
    );
    for name in [
        b"Arial".as_slice(),
        b"Helvetica-Bold",
        b"ABCDEF+Helvetica",
        b"helvetica",
        b"Times",
    ] {
        assert_eq!(BuiltinFace::from_pdf_name(name), None);
    }
    for (face, space, a, i) in [
        (BuiltinFace::Helvetica, 278., 667., 222.),
        (BuiltinFace::TimesRoman, 250., 722., 278.),
    ] {
        assert_eq!(face.lookup_glyph(b"space").unwrap().value(), space);
        assert_eq!(face.lookup_glyph(b"A").unwrap().value(), a);
        assert_eq!(face.lookup_glyph(b"i").unwrap().value(), i);
        assert_eq!(face.lookup_glyph(b"bullet").unwrap().value(), 350.);
        assert_eq!(face.glyphs().count(), 654);
        let names = face.glyphs().map(|(n, _)| n).collect::<Vec<_>>();
        assert!(names.windows(2).all(|p| p[0] < p[1]));
        assert_eq!(BuiltinFace::from_pdf_name(face.pdf_name()), Some(face));
    }
}
#[test]
fn afm_only_and_unknown_names_remain_missing() {
    for face in [BuiltinFace::Helvetica, BuiltinFace::TimesRoman] {
        for name in [
            b"zerosuperior".as_slice(),
            b"H18533",
            b"DefinitelyUnknownMetricGlyph",
            b"A.alt",
            b"A\0",
            b"\xff",
        ] {
            assert_eq!(face.lookup_glyph(name), None, "{name:?}");
        }
        assert!(face.lookup_glyph(b".notdef").is_some());
    }
}
#[test]
fn byte_lookup_does_not_fallback_or_use_unicode() {
    let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi)
        .with_differences(
            &[
                Difference::Code(65),
                Difference::Name(b".notdef"),
                Difference::Name(b"zerosuperior"),
                Difference::Name(b"unknown\0\xff"),
                Difference::Name(b"fi"),
            ],
            EncodingLimits::default(),
        )
        .unwrap();
    let metrics = BuiltinMetrics::from_encoding(BuiltinFace::Helvetica, encoding);
    assert_eq!(metrics.lookup(0), BuiltinMetricLookup::UndefinedGlyph);
    assert_eq!(metrics.lookup(65), BuiltinMetricLookup::UndefinedGlyph);
    assert_eq!(
        metrics.lookup(66),
        BuiltinMetricLookup::MissingBuiltinMetric(b"zerosuperior")
    );
    assert_eq!(
        metrics.lookup(67),
        BuiltinMetricLookup::MissingBuiltinMetric(b"unknown\0\xff")
    );
    let BuiltinMetricLookup::Advance(w) = metrics.lookup(68) else {
        panic!()
    };
    assert_eq!(w.value(), 500.);
    assert_eq!(metrics.glyph_name(68), Some(b"fi".as_slice()));
    let BuiltinMetricLookup::Advance(w) = metrics.lookup(0xa0) else {
        panic!()
    };
    assert_eq!(w.value(), 278.);
}
#[test]
fn all_named_encoding_slot_outcomes_match_measured_subset() {
    for face in [BuiltinFace::Helvetica, BuiltinFace::TimesRoman] {
        for (base, expected) in [
            (NamedEncoding::WinAnsi, (224, 32, 0)),
            (NamedEncoding::MacRoman, (208, 48, 0)),
            (NamedEncoding::Standard, (149, 107, 0)),
            (NamedEncoding::MacExpert, (23, 91, 142)),
        ] {
            let metrics = BuiltinMetrics::from_encoding(face, SimpleEncoding::from_named(base));
            let mut counts = (0, 0, 0);
            for code in 0..=255 {
                match metrics.lookup(code) {
                    BuiltinMetricLookup::Advance(w) => {
                        assert!(w.value().is_finite());
                        counts.0 += 1
                    }
                    BuiltinMetricLookup::UndefinedGlyph => counts.1 += 1,
                    BuiltinMetricLookup::MissingBuiltinMetric(name) => {
                        assert_eq!(metrics.glyph_name(code), Some(name));
                        counts.2 += 1
                    }
                }
            }
            assert_eq!(counts, expected, "{face:?} {base:?}");
        }
    }
}
