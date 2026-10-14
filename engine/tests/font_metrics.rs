use tandem_pdf_engine::font_metrics::*;
fn width(value: f64) -> FontAdvance {
    FontAdvance::new(value).unwrap()
}
#[test]
fn finite_signed_fractional_and_zero_values() {
    for value in [0.0, -0.0, 12.5, -88.25, f64::MAX, f64::MIN_POSITIVE] {
        assert_eq!(width(value).value().to_bits(), value.to_bits());
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            FontAdvance::new(value).unwrap_err().kind,
            MetricsErrorKind::NonFinite
        );
    }
    for value in [-(1i64 << 53), 0, 1i64 << 53] {
        assert_eq!(
            FontAdvance::from_integer(value).unwrap().value(),
            value as f64
        );
    }
    for value in [i64::MIN, i64::MAX, (1i64 << 53) + 1, -(1i64 << 53) - 1] {
        assert_eq!(
            FontAdvance::from_integer(value).unwrap_err().kind,
            MetricsErrorKind::InexactInteger
        );
    }
}
#[test]
fn explicit_entries_override_default_even_when_zero() {
    let m = HorizontalMetrics::from_widths(
        10,
        13,
        &[width(0.), width(-0.), width(-10.5), width(500.25)],
        Some(width(777.5)),
    )
    .unwrap();
    assert_eq!(m.first(), 10);
    assert_eq!(m.last(), 13);
    assert_eq!(m.widths().len(), 4);
    for code in [0, 9, 14, 255] {
        let value = m.lookup(code);
        assert_eq!(value.advance().value(), 777.5);
        assert_eq!(value.origin(), AdvanceOrigin::DeclaredMissingWidth);
    }
    for code in 10..=13 {
        assert_eq!(m.lookup(code).origin(), AdvanceOrigin::WidthsEntry);
    }
    assert_eq!(m.lookup(10).advance().value(), 0.);
    assert!(m.lookup(11).advance().value().is_sign_negative());
    assert_eq!(m.lookup(12).advance().value(), -10.5);
    assert_eq!(m.lookup(13).advance().value(), 500.25);
}
#[test]
fn default_missing_zero_and_declared_zero_are_distinct() {
    let implicit = HorizontalMetrics::from_widths(1, 1, &[width(12.)], None).unwrap();
    let explicit = HorizontalMetrics::from_widths(1, 1, &[width(12.)], Some(width(0.))).unwrap();
    assert_eq!(
        implicit.missing().origin(),
        AdvanceOrigin::DefaultMissingWidth
    );
    assert_eq!(
        explicit.missing().origin(),
        AdvanceOrigin::DeclaredMissingWidth
    );
    assert_eq!(implicit.lookup(255).advance().value(), 0.);
    assert_eq!(
        implicit.lookup(0).origin(),
        AdvanceOrigin::DefaultMissingWidth
    );
}
#[test]
fn complete_code_domain_and_exact_range_lengths() {
    let widths: Vec<_> = (0..256).map(|n| width(f64::from(n) - 50.)).collect();
    let m = HorizontalMetrics::from_widths(0, 255, &widths, None).unwrap();
    for code in 0..=255 {
        assert_eq!(m.lookup(code).advance().value(), f64::from(code) - 50.);
        assert_eq!(m.lookup(code).origin(), AdvanceOrigin::WidthsEntry);
    }
    assert_eq!(
        HorizontalMetrics::from_widths(255, 0, &[], None)
            .unwrap_err()
            .kind,
        MetricsErrorKind::ReversedRange
    );
    for widths in [&[][..], &[width(1.), width(2.)][..]] {
        assert!(matches!(
            HorizontalMetrics::from_widths(255, 255, widths, None)
                .unwrap_err()
                .kind,
            MetricsErrorKind::WrongWidthCount { .. }
        ));
    }
    assert_eq!(
        HorizontalMetrics::from_widths(255, 255, &[width(8.)], None)
            .unwrap()
            .lookup(255)
            .advance()
            .value(),
        8.
    );
}
