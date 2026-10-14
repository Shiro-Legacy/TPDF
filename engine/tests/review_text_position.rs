// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent provenance, sticky failure and cumulative T0 limit regressions.
use tandem_pdf_engine::{
    cmap::{CMapLimits, parse_to_unicode},
    content::{ContentLimits, ContentOperand, ContentParser, Operation},
    font_metrics::{FontAdvance, HorizontalMetrics},
    operators::validate_operation,
    simple_encoding::{NamedEncoding, SimpleEncoding},
    text_font::{TextFont, TextFontLimits, TextFonts, UnicodeSource},
    text_position::*,
};
fn metrics(width: f64) -> HorizontalMetrics {
    HorizontalMetrics::from_widths(0, 255, &vec![FontAdvance::new(width).unwrap(); 256], None)
        .unwrap()
}
fn execute(
    raw: &[u8],
    fonts: &TextFonts<'_>,
    limits: TextPositionLimits,
) -> Result<PositionedText, TextPositionError> {
    position_text_content(
        raw,
        fonts,
        TextPositionInitialState::default(),
        ContentLimits::default(),
        limits,
    )
}
#[test]
fn decoded_provenance_is_owned_and_does_not_claim_escaped_file_offsets() {
    let result = {
        let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
        let metrics = metrics(500.);
        let fonts = TextFonts::new(
            &[(
                b"F",
                TextFont::declared(UnicodeSource::Encoding(&encoding), &metrics).unwrap(),
            )],
            TextFontLimits::default(),
        )
        .unwrap();
        let raw = b"/F 8 Tf BT (\\101\\\nB) Tj [<43 44> 125 (E)] TJ ET".to_vec();
        let result = execute(&raw, &fonts, TextPositionLimits::default()).unwrap();
        let mut parser = ContentParser::new(&raw, ContentLimits::default());
        let mut operations = Vec::new();
        while let Some(op) = parser.next_operation().unwrap() {
            operations.push(op);
        }
        for (i, code) in result.codes().iter().enumerate() {
            let op = if i < 2 { 2 } else { 3 };
            assert_eq!(code.operation_index(), op);
            assert_eq!(code.operation_span(), operations[op].span);
            assert_eq!(code.source_span(), i..i + 1);
            assert_eq!(code.utf8_span(), i..i + 1);
        }
        assert_eq!(result.codes()[0].string_span(), 0..1);
        assert_eq!(result.codes()[1].string_span(), 1..2);
        assert_eq!(result.codes()[2].array_index(), Some(0));
        assert_eq!(result.codes()[4].array_index(), Some(2));
        assert_eq!(result.codes()[4].operand_index(), 0);
        assert_eq!(result.codes()[4].string_span(), 0..1);
        result
    };
    assert_eq!(result.text(), "ABCDE");
    assert_eq!(result.source_bytes(), b"ABCDE");
    assert_eq!(result.fonts()[0].name(), b"F");
}
#[test]
fn operation_limit_failure_reports_attempted_operation_and_is_sticky() {
    let fonts = TextFonts::new(&[], TextFontLimits::default()).unwrap();
    let mut positioner = TextPositioner::new(
        &fonts,
        TextPositionInitialState::default(),
        TextPositionLimits {
            max_operations: 1,
            ..TextPositionLimits::default()
        },
    )
    .unwrap();
    let first = Operation {
        operator: b"BT".to_vec(),
        operands: vec![],
        span: 4..6,
    };
    let second = Operation {
        operator: b"ET".to_vec(),
        operands: vec![],
        span: 9..11,
    };
    positioner
        .push(&validate_operation(&first).unwrap())
        .unwrap();
    let failure = positioner
        .push(&validate_operation(&second).unwrap())
        .unwrap_err();
    assert_eq!(failure.operation_index, 1);
    assert_eq!(failure.operation_span, 9..11);
    assert_eq!(
        failure.kind,
        TextPositionErrorKind::LimitExceeded(TextPositionLimitKind::Operations)
    );
    assert_eq!(
        positioner
            .push(&validate_operation(&first).unwrap())
            .unwrap_err(),
        failure
    );
    assert_eq!(positioner.finish().unwrap_err(), failure);
}
#[test]
fn direct_validated_tj_cannot_bypass_element_or_payload_limits() {
    let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let metrics = metrics(500.);
    let fonts = TextFonts::new(
        &[(
            b"F",
            TextFont::declared(UnicodeSource::Encoding(&encoding), &metrics).unwrap(),
        )],
        TextFontLimits::default(),
    )
    .unwrap();
    for (items, limits, kind) in [
        (
            vec![ContentOperand::String(vec![]); 100],
            TextPositionLimits {
                max_tj_elements: 99,
                ..TextPositionLimits::default()
            },
            TextPositionLimitKind::TjElements,
        ),
        (
            vec![
                ContentOperand::String(vec![65; 8]),
                ContentOperand::String(vec![66; 8]),
            ],
            TextPositionLimits {
                max_source_bytes: 15,
                ..TextPositionLimits::default()
            },
            TextPositionLimitKind::SourceBytes,
        ),
    ] {
        let mut positioner =
            TextPositioner::new(&fonts, TextPositionInitialState::default(), limits).unwrap();
        let mut parser = ContentParser::new(b"/F 8 Tf BT", ContentLimits::default());
        while let Some(op) = parser.next_operation().unwrap() {
            positioner.push(&validate_operation(&op).unwrap()).unwrap();
        }
        let operation = Operation {
            operator: b"TJ".to_vec(),
            operands: vec![ContentOperand::Array(items)],
            span: 20..30,
        };
        let error = positioner
            .push(&validate_operation(&operation).unwrap())
            .unwrap_err();
        assert_eq!(error.operation_index, 2);
        assert_eq!(error.kind, TextPositionErrorKind::LimitExceeded(kind));
        assert_eq!(positioner.finish().unwrap_err(), error);
    }
}
#[test]
fn empty_shows_and_numeric_adjustments_consume_cumulative_work() {
    let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let metrics = metrics(500.);
    let fonts = TextFonts::new(
        &[(
            b"F",
            TextFont::declared(UnicodeSource::Encoding(&encoding), &metrics).unwrap(),
        )],
        TextFontLimits::default(),
    )
    .unwrap();
    let raw = b"/F 8 Tf BT [] TJ () Tj [125] TJ ET";
    let result = execute(
        raw,
        &fonts,
        TextPositionLimits {
            max_show_operations: 3,
            max_tj_elements: 1,
            ..TextPositionLimits::default()
        },
    )
    .unwrap();
    assert!(result.codes().is_empty());
    assert_eq!(result.adjustments().len(), 1);
    assert_eq!(
        result.text_objects()[0].text_matrix().coefficients()[4],
        -1.
    );
    let error = execute(
        raw,
        &fonts,
        TextPositionLimits {
            max_show_operations: 2,
            ..TextPositionLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        TextPositionErrorKind::LimitExceeded(TextPositionLimitKind::ShowOperations)
    );
}
#[test]
fn repeated_multiscalar_mapping_charges_actual_output_and_preserves_error_context() {
    let map=parse_to_unicode(b"begincmap 1 begincodespacerange <00><ff> endcodespacerange 1 beginbfchar <41><00660069> endbfchar endcmap",CMapLimits::default()).unwrap();
    let metrics = metrics(500.);
    let fonts = TextFonts::new(
        &[(
            b"F",
            TextFont::declared(UnicodeSource::ToUnicode(&map), &metrics).unwrap(),
        )],
        TextFontLimits::default(),
    )
    .unwrap();
    let raw = b"/F 8 Tf BT [(A) 125 (AA)] TJ ET";
    let good = execute(
        raw,
        &fonts,
        TextPositionLimits {
            max_codes: 3,
            max_scalars: 6,
            max_utf8_bytes: 6,
            ..TextPositionLimits::default()
        },
    )
    .unwrap();
    assert_eq!(good.codes().len(), 3);
    assert_eq!(good.text(), "fififi");
    let error = execute(
        raw,
        &fonts,
        TextPositionLimits {
            max_scalars: 5,
            ..TextPositionLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.operation_index, 2);
    assert_eq!(error.operand_index, Some(0));
    assert_eq!(error.array_index, Some(2));
    assert_eq!(error.string_offset, Some(1));
    assert!(matches!(error.kind, TextPositionErrorKind::Decode(_)));
}
#[test]
fn zero_horizontal_scale_cannot_hide_nonfinite_width_product() {
    let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let metrics = metrics(1e308);
    let fonts = TextFonts::new(
        &[(
            b"F",
            TextFont::declared(UnicodeSource::Encoding(&encoding), &metrics).unwrap(),
        )],
        TextFontLimits::default(),
    )
    .unwrap();
    let error = execute(
        b"/F 1000000 Tf 0 Tz BT (A) Tj ET",
        &fonts,
        TextPositionLimits {
            max_linear: f64::MAX,
            max_position: f64::MAX,
            max_text_value: f64::MAX,
            ..TextPositionLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, TextPositionErrorKind::Nonfinite);
    assert_eq!(error.string_offset, Some(0));
}
