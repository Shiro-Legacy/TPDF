// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::BTreeMap;
use tandem_pdf_engine::content::{ContentOperand as V, Operation};
use tandem_pdf_engine::operators::{OperatorErrorKind as E, OperatorKind as K, validate_operation};
fn op(word: &[u8], operands: Vec<V>) -> Operation {
    Operation {
        operator: word.to_vec(),
        operands,
        span: 13..42,
    }
}
fn valid(word: &[u8], operands: Vec<V>, kind: K) {
    let operation = op(word, operands);
    let result = validate_operation(&operation).unwrap();
    assert_eq!(result.kind(), kind);
    assert!(std::ptr::eq(result.operation(), &operation));
    assert!(std::ptr::eq(
        result.operands(),
        operation.operands.as_slice()
    ));
}
fn bad(word: &[u8], operands: Vec<V>, kind: E) {
    let error = validate_operation(&op(word, operands)).unwrap_err();
    assert_eq!(error.kind, kind);
    assert_eq!(error.offset, 13);
}
fn n(n: i64) -> V {
    V::Integer(n)
}
fn name(s: &[u8]) -> V {
    V::Name(s.to_vec())
}
#[test]
fn path_arities_aliases_and_matrix_shape() {
    valid(b"cm", vec![n(1), n(0), n(0), n(1), n(20), n(30)], K::Concat);
    valid(b"m", vec![n(3), V::Real(-4.5)], K::MoveTo);
    valid(b"c", vec![n(1); 6], K::CurveTo);
    valid(b"v", vec![n(1); 4], K::CurveToV);
    valid(b"y", vec![n(1); 4], K::CurveToY);
    valid(b"re", vec![n(1); 4], K::Rectangle);
    for (word, kind) in [
        (b"q".as_slice(), K::Save),
        (b"Q", K::Restore),
        (b"h", K::ClosePath),
        (b"S", K::Stroke),
        (b"s", K::CloseStroke),
        (b"f", K::Fill),
        (b"F", K::Fill),
        (b"f*", K::EvenOddFill),
        (b"B", K::FillStroke),
        (b"B*", K::EvenOddFillStroke),
        (b"b", K::CloseFillStroke),
        (b"b*", K::CloseEvenOddFillStroke),
        (b"n", K::EndPath),
        (b"W", K::Clip),
        (b"W*", K::EvenOddClip),
    ] {
        valid(word, vec![], kind);
        bad(word, vec![n(0)], E::Arity);
    }
    bad(b"cm", vec![n(1); 5], E::Arity);
    bad(b"l", vec![V::Boolean(true), n(1)], E::OperandType);
    bad(b"m", vec![V::Real(f64::INFINITY), n(1)], E::NonfiniteNumber);
}
#[test]
fn dash_and_line_state_ranges_are_strict() {
    valid(b"d", vec![V::Array(vec![]), n(0)], K::Dash);
    valid(
        b"d",
        vec![V::Array(vec![n(0), n(4), V::Real(2.5)]), n(3)],
        K::Dash,
    );
    for (pattern, phase) in [
        (vec![n(0), n(0)], n(0)),
        (vec![n(-1), n(2)], n(0)),
        (vec![], n(-1)),
    ] {
        bad(b"d", vec![V::Array(pattern), phase], E::Range);
    }
    bad(
        b"d",
        vec![V::Array(vec![name(b"bad")]), n(0)],
        E::OperandType,
    );
    valid(b"J", vec![n(2)], K::LineCap);
    valid(b"j", vec![n(0)], K::LineJoin);
    bad(b"J", vec![V::Real(1.0)], E::OperandType);
    bad(b"j", vec![n(3)], E::Range);
    valid(b"w", vec![n(0)], K::LineWidth);
    bad(b"w", vec![n(-1)], E::Range);
    valid(b"M", vec![n(1)], K::MiterLimit);
    bad(b"M", vec![V::Real(0.99)], E::Range);
    valid(b"i", vec![n(100)], K::Flatness);
    bad(b"i", vec![n(101)], E::Range);
}
#[test]
fn text_arrays_and_quoted_operators_preserve_binary_payloads() {
    valid(b"Tf", vec![name(b"F1"), n(-12)], K::Font);
    valid(b"Tj", vec![V::String(vec![0, 255])], K::ShowText);
    valid(
        b"TJ",
        vec![V::Array(vec![
            V::String(b"a".to_vec()),
            V::Real(-25.5),
            V::String(vec![]),
        ])],
        K::ShowTextArray,
    );
    valid(b"TJ", vec![V::Array(vec![])], K::ShowTextArray);
    valid(b"'", vec![V::String(vec![0])], K::NextLineShowText);
    valid(
        b"\"",
        vec![n(1), n(2), V::String(vec![255])],
        K::SpacedNextLineShowText,
    );
    bad(b"Tj", vec![name(b"string-is-required")], E::OperandType);
    bad(b"TJ", vec![V::Array(vec![V::Null])], E::OperandType);
    bad(
        b"TJ",
        vec![V::Array(vec![V::Array(vec![])])],
        E::OperandType,
    );
    bad(
        b"TJ",
        vec![V::Array(vec![V::Real(f64::NAN)])],
        E::NonfiniteNumber,
    );
    valid(b"Tr", vec![n(7)], K::TextRenderingMode);
    bad(b"Tr", vec![n(8)], E::Range);
}
#[test]
fn color_rules_keep_pattern_names_separate_from_numeric_components() {
    valid(b"rg", vec![V::Real(-0.1), n(2), n(0)], K::FillRgb); // interpretation clamps device colors later
    valid(b"SC", vec![n(1); 32], K::StrokeColor);
    valid(b"scn", vec![name(b"P1")], K::FillColorPattern);
    valid(b"SCN", vec![n(1), n(0), name(b"P2")], K::StrokeColorPattern);
    bad(b"SC", vec![name(b"P1")], E::OperandType);
    bad(b"scn", vec![name(b"P1"), n(0)], E::OperandType);
    bad(b"SCN", vec![], E::Arity);
    bad(b"sc", vec![n(0); 33], E::Range);
    bad(b"k", vec![n(0); 3], E::Arity);
}
#[test]
fn marked_content_and_resources_have_name_and_property_types() {
    valid(
        b"BDC",
        vec![
            name(b"Span"),
            V::Dictionary(BTreeMap::from([(
                b"ActualText".to_vec(),
                V::String(vec![0, 1]),
            )])),
        ],
        K::BeginMarkedContentProperties,
    );
    valid(
        b"DP",
        vec![name(b"Tag"), name(b"Properties1")],
        K::MarkedPointProperties,
    );
    valid(b"Do", vec![name(b"Image1")], K::XObject);
    valid(b"gs", vec![name(b"GS1")], K::ExtGState);
    bad(
        b"BDC",
        vec![name(b"Tag"), V::Boolean(false)],
        E::OperandType,
    );
    bad(b"Do", vec![V::String(b"X".to_vec())], E::OperandType);
}
#[test]
fn known_syntax_does_not_claim_state_balance_or_execute_unknowns() {
    valid(b"ET", vec![], K::EndText); // local schema only; interpreter enforces BT/ET pairing
    valid(b"BX", vec![], K::BeginCompatibility);
    valid(b"EX", vec![], K::EndCompatibility);
    bad(b"FutureOp", vec![], E::UnknownOperator);
    for word in [b"ID", b"EI"] {
        bad(word, vec![], E::InlineImageUnsupported);
    }
    bad(b"BI", vec![], E::Arity);
    valid(
        b"BI",
        vec![V::Dictionary(Default::default()), V::String(vec![0])],
        K::InlineImage,
    );
}
