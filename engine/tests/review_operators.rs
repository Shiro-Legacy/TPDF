// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent local operator validation and borrowed-payload checks.
use tandem_pdf_engine::content::{ContentOperand as V, Operation};
use tandem_pdf_engine::operators::{OperatorErrorKind, OperatorKind, validate_operation};
fn operation(operator: &[u8], operands: Vec<V>) -> Operation {
    Operation {
        operator: operator.to_vec(),
        operands,
        span: 17..23,
    }
}

#[test]
fn binary_text_payload_is_borrowed_and_double_quote_operand_order_matters() {
    let op = operation(
        b"\"",
        vec![V::Integer(2), V::Real(1.5), V::String(vec![0, 255, b'('])],
    );
    let valid = validate_operation(&op).unwrap();
    assert_eq!(valid.kind(), OperatorKind::SpacedNextLineShowText);
    assert!(std::ptr::eq(valid.operation(), &op));
    assert_eq!(valid.operands().as_ptr(), op.operands.as_ptr());
    assert!(
        validate_operation(&operation(
            b"\"",
            vec![V::String(vec![]), V::Integer(2), V::Integer(3)]
        ))
        .is_err()
    );
}

#[test]
fn text_array_refuses_nested_operands_and_nonfinite_adjustments() {
    for operand in [
        V::Array(vec![]),
        V::Name(b"A".to_vec()),
        V::Boolean(true),
        V::Null,
        V::Real(f64::NAN),
        V::Real(f64::INFINITY),
    ] {
        let op = operation(
            b"TJ",
            vec![V::Array(vec![V::String(vec![0, 255]), operand])],
        );
        let error = validate_operation(&op).unwrap_err();
        assert_eq!(error.offset, 17);
    }
    assert!(
        validate_operation(&operation(
            b"TJ",
            vec![V::Array(vec![
                V::String(vec![0]),
                V::Integer(-20),
                V::Real(0.5)
            ])]
        ))
        .is_ok()
    );
}

#[test]
fn pattern_color_accepts_name_only_but_plain_color_rejects_names() {
    for op in [b"SCN".as_slice(), b"scn"] {
        assert!(validate_operation(&operation(op, vec![V::Name(b"Pattern".to_vec())])).is_ok());
        assert!(
            validate_operation(&operation(
                op,
                vec![V::Name(b"Pattern".to_vec()), V::Integer(1)]
            ))
            .is_err()
        );
    }
    assert!(validate_operation(&operation(b"SC", vec![V::Name(b"Pattern".to_vec())])).is_err());
    assert_eq!(
        validate_operation(&operation(b"VendorOp", vec![]))
            .unwrap_err()
            .kind,
        OperatorErrorKind::UnknownOperator
    );
}

#[test]
fn degenerate_dash_arrays_and_out_of_range_integer_modes_are_refused() {
    for values in [
        vec![V::Integer(0)],
        vec![V::Integer(0), V::Real(-0.0)],
        vec![V::Integer(-1), V::Integer(2)],
    ] {
        assert!(
            validate_operation(&operation(b"d", vec![V::Array(values), V::Integer(0)])).is_err()
        );
    }
    assert!(validate_operation(&operation(b"d", vec![V::Array(vec![]), V::Integer(0)])).is_ok());
    assert!(
        validate_operation(&operation(
            b"d",
            vec![V::Array(vec![V::Integer(0), V::Integer(2)]), V::Integer(0)]
        ))
        .is_ok()
    );
    for (op, valid, invalid) in [(b"J".as_slice(), 2, 3), (b"j", 2, 3), (b"Tr", 7, 8)] {
        assert!(validate_operation(&operation(op, vec![V::Integer(valid)])).is_ok());
        assert_eq!(
            validate_operation(&operation(op, vec![V::Integer(invalid)]))
                .unwrap_err()
                .kind,
            OperatorErrorKind::Range
        );
    }
}
