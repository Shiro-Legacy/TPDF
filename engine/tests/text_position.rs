use tandem_pdf_engine::cmap::{CMapLimits, parse_to_unicode};
use tandem_pdf_engine::content::{ContentLimits, ContentOperand, Operation};
use tandem_pdf_engine::font_metrics::{AdvanceOrigin, FontAdvance, HorizontalMetrics};
use tandem_pdf_engine::geometry::{Affine, PageSpace, UserSpace};
use tandem_pdf_engine::operators::validate_operation;
use tandem_pdf_engine::simple_encoding::{NamedEncoding, SimpleEncoding};
use tandem_pdf_engine::text_font::*;
use tandem_pdf_engine::text_position::*;

fn run_with(input: &[u8], limits: TextPositionLimits) -> Result<PositionedText, TextPositionError> {
    let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let metrics =
        HorizontalMetrics::from_widths(32, 90, &vec![FontAdvance::new(500.).unwrap(); 59], None)
            .unwrap();
    let font = TextFont::declared(UnicodeSource::Encoding(&encoding), &metrics).unwrap();
    let fonts = TextFonts::new(&[(b"F", font)], TextFontLimits::default()).unwrap();
    position_text_content(
        input,
        &fonts,
        TextPositionInitialState::default(),
        ContentLimits::default(),
        limits,
    )
}
fn run(input: &[u8]) -> PositionedText {
    run_with(input, TextPositionLimits::default()).unwrap()
}
fn tm(x: f64, y: f64) -> [f64; 6] {
    [1., 0., 0., 1., x, y]
}
#[test]
fn basic_spacing_tj_and_flat_provenance() {
    let out = run(b"BT /F 10 Tf 2 Tc 3 Tw [(A ) 200 (B) -100] TJ ET");
    assert_eq!(out.text(), "A B");
    assert_eq!(out.source_bytes(), b"A B");
    assert_eq!(out.codes().len(), 3);
    assert_eq!(out.adjustments().len(), 2);
    assert_eq!(out.codes()[0].before().coefficients(), tm(0., 0.));
    assert_eq!(out.codes()[1].before().coefficients(), tm(7., 0.));
    assert_eq!(out.codes()[1].after().coefficients(), tm(17., 0.));
    assert_eq!(out.codes()[2].before().coefficients(), tm(15., 0.));
    assert_eq!(
        out.text_objects()[0].text_matrix().coefficients(),
        tm(23., 0.)
    );
    assert_eq!(
        out.text_objects()[0].line_matrix().coefficients(),
        tm(0., 0.)
    );
    assert_eq!(out.codes()[0].array_index(), Some(0));
    assert_eq!(out.codes()[2].array_index(), Some(2));
    assert_eq!(out.codes()[2].string_span(), 0..1);
    assert_eq!(out.codes()[2].source_span(), 2..3);
    assert_eq!(out.adjustments()[0].array_index(), 1);
    assert_eq!(
        out.events(),
        [
            TextEvent::Code(0),
            TextEvent::Code(1),
            TextEvent::Adjustment(0),
            TextEvent::Code(2),
            TextEvent::Adjustment(1),
            TextEvent::EndTextObject(0)
        ]
    );
    assert_eq!(out.fonts()[0].name(), b"F");
    assert_eq!(out.fonts()[0].metric_kind(), TextMetricKind::Declared);
}
#[test]
fn quotes_leading_and_text_state_persist() {
    let out=run(b"/F 10 Tf 12 TL BT 1 0 0 1 20 50 Tm (A) Tj (B) ' 3 2 (C) \" 4 -5 TD (D) Tj T* (E) Tj ET BT (F) Tj ET");
    let origins: Vec<_> = out
        .codes()
        .iter()
        .map(|c| c.origin().coordinates())
        .collect();
    assert_eq!(
        origins,
        [
            [20., 50.],
            [20., 38.],
            [20., 26.],
            [24., 21.],
            [24., 16.],
            [0., 0.]
        ]
    );
    assert_eq!(out.codes()[2].character_spacing(), 2.);
    assert_eq!(out.codes()[5].character_spacing(), 2.);
    assert_eq!(out.codes()[5].text_object(), 1);
    assert_eq!(
        out.text_objects()[1].text_matrix().coefficients(),
        tm(7., 0.)
    );
}
#[test]
fn ctm_matrix_composition_rise_scale_and_save_restore() {
    let out=run(b"/F 10 Tf q 0 2 -3 0 100 200 cm 50 Tz 4 Ts 3 Tr BT 1 1 2 1 10 20 Tm (A) Tj ET Q BT (B) Tj ET");
    let a = &out.codes()[0];
    assert_eq!(
        a.transform().coefficients(),
        [-15., 10., -30., 40., 28., 236.]
    );
    assert_eq!(a.origin().coordinates(), [28., 236.]);
    assert_eq!(a.after().coefficients(), [1., 1., 2., 1., 12.5, 22.5]);
    assert_eq!(a.rendering_mode(), 3);
    assert_eq!(a.rise(), 4.);
    assert_eq!(
        out.codes()[1].transform().coefficients(),
        [10., 0., 0., 10., 0., 0.]
    );
    assert_eq!(out.codes()[1].rendering_mode(), 0);
}
#[test]
fn original_byte_controls_word_spacing_and_ligatures_advance_once() {
    let map=parse_to_unicode(b"begincmap 1 begincodespacerange <00> <ff> endcodespacerange 3 beginbfchar <20> <0058> <41> <0020> <42> <00660069D83DDE000000FEFF> endbfchar endcmap",CMapLimits::default()).unwrap();
    let widths = [FontAdvance::new(250.).unwrap(); 256];
    let metrics = HorizontalMetrics::from_widths(0, 255, &widths, None).unwrap();
    let font = TextFont::declared(UnicodeSource::ToUnicode(&map), &metrics).unwrap();
    let fonts = TextFonts::new(&[(b"F", font)], TextFontLimits::default()).unwrap();
    let out = position_text_content(
        b"BT /F 8 Tf 4 Tw ( AB) Tj ET",
        &fonts,
        TextPositionInitialState::default(),
        ContentLimits::default(),
        TextPositionLimits::default(),
    )
    .unwrap();
    assert_eq!(out.text(), "X fi😀\0\u{feff}");
    assert_eq!(out.codes().len(), 3);
    assert_eq!(out.codes()[0].word_spacing(), 4.);
    assert_eq!(out.codes()[1].word_spacing(), 0.);
    assert_eq!(out.codes()[2].before().coefficients(), tm(8., 0.));
    assert_eq!(out.codes()[2].after().coefficients(), tm(10., 0.));
    assert_eq!(&out.text()[out.codes()[2].utf8_span()], "fi😀\0\u{feff}");
}
#[test]
fn signed_zero_fractional_widths_and_singular_matrices_are_forward_geometry() {
    let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let metrics = HorizontalMetrics::from_widths(
        65,
        67,
        &[
            FontAdvance::new(-125.5).unwrap(),
            FontAdvance::new(0.).unwrap(),
            FontAdvance::new(250.25).unwrap(),
        ],
        Some(FontAdvance::new(75.).unwrap()),
    )
    .unwrap();
    let font = TextFont::declared(UnicodeSource::Encoding(&encoding), &metrics).unwrap();
    let fonts = TextFonts::new(&[(b"F", font)], TextFontLimits::default()).unwrap();
    let out = position_text_content(
        b"BT /F -8 Tf -50 Tz 0 0 0 0 2 3 Tm (ABCD) Tj ET",
        &fonts,
        TextPositionInitialState::default(),
        ContentLimits::default(),
        TextPositionLimits::default(),
    )
    .unwrap();
    assert_eq!(out.codes()[0].glyph_displacement(), -0.502);
    assert_eq!(out.codes()[1].glyph_displacement(), 0.);
    for code in out.codes() {
        assert_eq!(code.origin().coordinates(), [2., 3.]);
    }
    assert_eq!(
        out.codes()[3].metric_origin(),
        TextMetricOrigin::Declared(AdvanceOrigin::DeclaredMissingWidth)
    );
}
#[test]
fn empty_shows_numeric_arrays_and_no_font_failures() {
    let out = run(b"BT /F 10 Tf () Tj [] TJ [100 -200 ()] TJ ET");
    assert!(out.codes().is_empty());
    assert_eq!(out.adjustments().len(), 2);
    assert_eq!(
        out.text_objects()[0].text_matrix().coefficients(),
        tm(1., 0.)
    );
    for input in [b"BT [] TJ ET".as_slice(), b"BT () Tj ET", b"BT [100] TJ ET"] {
        assert!(matches!(
            run_with(input, TextPositionLimits::default())
                .unwrap_err()
                .kind,
            TextPositionErrorKind::State(_)
        ));
    }
}
#[test]
fn all_structural_and_unsupported_forms_fail() {
    for input in [
        b"BT BT ET ET".as_slice(),
        b"ET",
        b"BT",
        b"Q",
        b"q",
        b"(A) Tj",
        b"1 2 Td",
        b"BT q ET",
        b"BT Q ET",
        b"BT 1 0 0 1 0 0 cm ET",
        b"1 Tr",
        b"2 Tr",
        b"4 Tr",
        b"7 Tr",
        b"0 g",
        b"BX",
        b"/Unknown 10 Tf",
    ] {
        assert!(
            run_with(input, TextPositionLimits::default()).is_err(),
            "{}",
            String::from_utf8_lossy(input)
        );
    }
}
#[test]
fn sticky_error_and_no_partial_finish() {
    let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let metrics =
        HorizontalMetrics::from_widths(65, 65, &[FontAdvance::new(500.).unwrap()], None).unwrap();
    let font = TextFont::declared(UnicodeSource::Encoding(&encoding), &metrics).unwrap();
    let fonts = TextFonts::new(&[(b"F", font)], TextFontLimits::default()).unwrap();
    let mut interpreter = TextPositioner::new(
        &fonts,
        TextPositionInitialState::default(),
        TextPositionLimits::default(),
    )
    .unwrap();
    let bt = Operation {
        operator: b"BT".to_vec(),
        operands: vec![],
        span: 0..2,
    };
    interpreter.push(&validate_operation(&bt).unwrap()).unwrap();
    let error = interpreter
        .push(&validate_operation(&bt).unwrap())
        .unwrap_err();
    let et = Operation {
        operator: b"ET".to_vec(),
        operands: vec![],
        span: 2..4,
    };
    assert_eq!(
        interpreter
            .push(&validate_operation(&et).unwrap())
            .unwrap_err(),
        error
    );
    assert_eq!(interpreter.finish().unwrap_err(), error);
}
#[test]
fn decoded_string_ranges_are_not_lexical_offsets() {
    let out = run(b"BT /F 10 Tf (A\\102\\\nC) Tj <44 45> Tj ET");
    assert_eq!(out.source_bytes(), b"ABCDE");
    assert_eq!(out.codes()[1].string_span(), 1..2);
    assert_eq!(out.codes()[2].string_span(), 2..3);
    assert_eq!(out.codes()[3].string_span(), 0..1);
    assert_eq!(out.codes()[0].operation_span(), 12..25);
    assert_eq!(out.codes()[3].operation_span(), 26..36);
}
#[test]
fn cumulative_counts_exact_boundaries_and_zero() {
    let input = b"BT /F 10 Tf [(A) 10 (B)] TJ ET";
    let mut limits = TextPositionLimits {
        max_operations: 4,
        max_text_objects: 1,
        max_codes: 2,
        max_tj_elements: 3,
        max_show_operations: 3,
        max_source_bytes: 2,
        max_scalars: 2,
        max_utf8_bytes: 2,
        max_fonts: 1,
        max_font_name_bytes: 1,
        max_total_font_name_bytes: 1,
        ..TextPositionLimits::default()
    };
    run_with(input, limits).unwrap();
    let settings: [fn(&mut TextPositionLimits); 10] = [
        |l| l.max_operations = 3,
        |l| l.max_text_objects = 0,
        |l| l.max_codes = 1,
        |l| l.max_tj_elements = 2,
        |l| l.max_show_operations = 2,
        |l| l.max_source_bytes = 1,
        |l| l.max_scalars = 1,
        |l| l.max_utf8_bytes = 1,
        |l| l.max_fonts = 0,
        |l| l.max_total_font_name_bytes = 0,
    ];
    for set in settings {
        let mut small = limits;
        set(&mut small);
        assert!(run_with(input, small).is_err());
    }
    limits.max_operations = 0;
    assert!(run_with(b"", limits).is_ok());
    assert!(run_with(b"BT ET", limits).is_err());
    assert!(
        run_with(
            b"BT /F 10 Tf [] TJ [] TJ ET",
            TextPositionLimits {
                max_show_operations: 1,
                ..TextPositionLimits::default()
            }
        )
        .is_err()
    );
    assert!(
        run_with(
            b"q q Q Q",
            TextPositionLimits {
                max_stack_depth: 1,
                ..TextPositionLimits::default()
            }
        )
        .is_err()
    );
    assert!(
        run_with(
            b"BT /F 10 Tf (A) Tj (A) Tj ET",
            TextPositionLimits {
                max_codes: 1,
                ..TextPositionLimits::default()
            }
        )
        .is_err()
    );
}
#[test]
fn allocation_boundary_is_monotone_and_repeat_work_is_charged() {
    let program = b"BT /F 10 Tf (AAAA) Tj ET";
    let mut low = 0;
    let mut high = 100_000;
    while low < high {
        let mid = (low + high) / 2;
        if run_with(
            program,
            TextPositionLimits {
                max_allocation_bytes: mid,
                ..TextPositionLimits::default()
            },
        )
        .is_ok()
        {
            high = mid
        } else {
            low = mid + 1
        }
    }
    assert!(low > 1000);
    run_with(
        program,
        TextPositionLimits {
            max_allocation_bytes: low,
            ..TextPositionLimits::default()
        },
    )
    .unwrap();
    assert!(
        run_with(
            program,
            TextPositionLimits {
                max_allocation_bytes: low - 1,
                ..TextPositionLimits::default()
            }
        )
        .is_err()
    );
    assert!(
        run_with(
            b"BT /F 10 Tf (AAAAAAAA) Tj ET",
            TextPositionLimits {
                max_allocation_bytes: low,
                ..TextPositionLimits::default()
            }
        )
        .is_err()
    );
}
#[test]
fn inexact_integer_and_numerical_policy_reject_before_loss() {
    assert_eq!(
        run_with(b"9007199254740993 Tc", TextPositionLimits::default())
            .unwrap_err()
            .kind,
        TextPositionErrorKind::InexactInteger
    );
    for input in [
        b"1000001 Tc".as_slice(),
        b"BT /F 1000000 Tf 1000000 Tz (A) Tj ET",
        b"BT 1 0 0 1 1000000000 0 Tm /F 10 Tf (A) Tj ET",
    ] {
        assert!(run_with(input, TextPositionLimits::default()).is_err());
    }
    let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let metrics =
        HorizontalMetrics::from_widths(65, 65, &[FontAdvance::new(500.).unwrap()], None).unwrap();
    let font = TextFont::declared(UnicodeSource::Encoding(&encoding), &metrics).unwrap();
    let fonts = TextFonts::new(&[(b"F", font)], TextFontLimits::default()).unwrap();
    let initial = TextPositionInitialState::new(
        Affine::<UserSpace, PageSpace>::from_pdf([1., 0., 0., 1., 1e10, 0.]).unwrap(),
    );
    assert!(TextPositioner::new(&fonts, initial, TextPositionLimits::default()).is_err());
    assert!(
        TextPositioner::new(
            &fonts,
            TextPositionInitialState::default(),
            TextPositionLimits {
                max_linear: f64::INFINITY,
                ..TextPositionLimits::default()
            }
        )
        .is_err()
    );
}
#[test]
fn direct_operation_tj_budget_preflight_and_bad_span() {
    let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let metrics =
        HorizontalMetrics::from_widths(65, 65, &[FontAdvance::new(500.).unwrap()], None).unwrap();
    let font = TextFont::declared(UnicodeSource::Encoding(&encoding), &metrics).unwrap();
    let fonts = TextFonts::new(&[(b"F", font)], TextFontLimits::default()).unwrap();
    let mut interpreter = TextPositioner::new(
        &fonts,
        TextPositionInitialState::default(),
        TextPositionLimits {
            max_tj_elements: 0,
            ..TextPositionLimits::default()
        },
    )
    .unwrap();
    for op in [
        Operation {
            operator: b"BT".to_vec(),
            operands: vec![],
            span: 0..2,
        },
        Operation {
            operator: b"Tf".to_vec(),
            operands: vec![
                ContentOperand::Name(b"F".to_vec()),
                ContentOperand::Integer(10),
            ],
            span: 3..11,
        },
    ] {
        interpreter.push(&validate_operation(&op).unwrap()).unwrap();
    }
    let op = Operation {
        operator: b"TJ".to_vec(),
        operands: vec![ContentOperand::Array(vec![ContentOperand::String(vec![
            255,
        ])])],
        span: 12..19,
    };
    assert_eq!(
        interpreter
            .push(&validate_operation(&op).unwrap())
            .unwrap_err()
            .kind,
        TextPositionErrorKind::LimitExceeded(TextPositionLimitKind::TjElements)
    );
    let mut interpreter = TextPositioner::new(
        &fonts,
        TextPositionInitialState::default(),
        TextPositionLimits::default(),
    )
    .unwrap();
    let op = Operation {
        operator: b"BT".to_vec(),
        operands: vec![],
        span: std::ops::Range { start: 2, end: 1 },
    };
    assert_eq!(
        interpreter
            .push(&validate_operation(&op).unwrap())
            .unwrap_err()
            .kind,
        TextPositionErrorKind::InvalidSpan
    );
}

#[test]
fn growth_charges_full_requested_capacity_not_only_retained_delta() {
    use std::mem::size_of;
    use tandem_pdf_engine::text_decode::DecodedCode;
    let fixed = 1
        + size_of::<PositionedFont>()
        + 4
        + size_of::<TextObjectEnd>()
        + 4 * 4 * (size_of::<DecodedCode>() + 1);
    let retained = fixed + 4 * size_of::<CodePlacement>() + 8 * size_of::<TextEvent>() + 4;
    let cumulative = fixed + 7 * size_of::<CodePlacement>() + 15 * size_of::<TextEvent>() + 7;
    let program = b"BT /F 10 Tf (AAAA) Tj ET";
    let limits = |n| TextPositionLimits {
        max_allocation_bytes: n,
        ..TextPositionLimits::default()
    };
    assert!(run_with(program, limits(retained)).is_err());
    assert!(run_with(program, limits(cumulative - 1)).is_err());
    run_with(program, limits(cumulative)).unwrap();
}

#[test]
fn allocation_limited_decode_reports_allocation_not_utf8_policy() {
    use std::mem::size_of;
    use tandem_pdf_engine::text_decode::{DecodedCode, TextDecodeErrorKind, TextDecodeLimitKind};
    let allocation = size_of::<PositionedFont>() + 1 + 1 + 4 * size_of::<DecodedCode>();
    let error = run_with(
        b"BT /F 10 Tf (A) Tj ET",
        TextPositionLimits {
            max_allocation_bytes: allocation,
            ..TextPositionLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        TextPositionErrorKind::LimitExceeded(TextPositionLimitKind::AllocationBytes)
    );
    assert_eq!(error.string_offset, Some(0));
    let error = run_with(
        b"BT /F 10 Tf (A) Tj ET",
        TextPositionLimits {
            max_utf8_bytes: 0,
            ..TextPositionLimits::default()
        },
    )
    .unwrap_err();
    assert!(matches!(error.kind, TextPositionErrorKind::Decode(ref e)
        if e.kind == TextDecodeErrorKind::LimitExceeded(TextDecodeLimitKind::Utf8Bytes)));
}
