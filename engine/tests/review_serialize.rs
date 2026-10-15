// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent object-writer review; these checks make no PDF-save claim.
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};
use tandem_pdf_engine::object::{Object, ObjectId};
use tandem_pdf_engine::serialize::{SerializeError, WriteLimits, serialize_object};

#[test]
fn wire_syntax_has_unambiguous_binary_names_strings_and_reference_boundaries() {
    let object = Object::Array(vec![
        Object::Name(Vec::new()),
        Object::Name(b"true# /%[]()<>\x01\xff".to_vec()),
        Object::String(vec![0, 10, 255]),
        Object::Integer(1),
        Object::Integer(0),
        Object::Reference(ObjectId::new(u32::MAX, u16::MAX).unwrap()),
        Object::Real(-0.0),
        Object::Real(2.0),
    ]);
    assert_eq!(
        serialize_object(&object, WriteLimits::default()).unwrap(),
        b"[/ /true#23#20#2F#25#5B#5D#28#29#3C#3E#01#FF <000AFF> 1 0 4294967295 65535 R -0.0 2.0]"
    );
}

#[test]
fn real_wire_numbers_preserve_all_finite_exponent_bands_without_own_parser() {
    // Inspect the emitted grammar and use Rust's independent float conversion,
    // rather than routing this evidence through the engine's PDF parser.
    for exponent in 0..2047u64 {
        for mantissa in [0, 1, (1u64 << 52) - 1] {
            for sign in [0, 1u64 << 63] {
                let value = f64::from_bits(sign | exponent << 52 | mantissa);
                let wire = serialize_object(&Object::Real(value), WriteLimits::default()).unwrap();
                assert_eq!(wire.iter().filter(|&&b| b == b'.').count(), 1);
                assert!(
                    wire.iter()
                        .enumerate()
                        .all(|(i, b)| b.is_ascii_digit() || *b == b'.' || (i == 0 && *b == b'-'))
                );
                let text = std::str::from_utf8(&wire).unwrap();
                assert_eq!(text.parse::<f64>().unwrap().to_bits(), value.to_bits());
            }
        }
    }
}

#[test]
fn hard_depth_cap_applies_to_manually_constructed_values() {
    let mut value = Object::Integer(7);
    for _ in 0..256 {
        value = Object::Array(vec![value]);
    }
    let limits = WriteLimits {
        max_depth: usize::MAX,
        ..WriteLimits::default()
    };
    let bytes = serialize_object(&value, limits).unwrap();
    assert_eq!(bytes.len(), 513);
    value = Object::Array(vec![value]);
    assert_eq!(
        serialize_object(&value, limits),
        Err(SerializeError::DepthLimit)
    );
}

#[test]
fn escaped_name_expansion_and_dictionary_keys_are_charged_exactly() {
    let value = Object::Dictionary(BTreeMap::from([(vec![255; 32], Object::Null)]).into());
    // << + space + / + 32*#FF + space + null + space + >>
    let size = 2 + 1 + 1 + 32 * 3 + 1 + 4 + 1 + 2;
    let exact = WriteLimits {
        max_output_bytes: size,
        max_objects: 3,
        max_depth: 1,
    };
    assert_eq!(serialize_object(&value, exact).unwrap().len(), size);
    assert_eq!(
        serialize_object(
            &value,
            WriteLimits {
                max_output_bytes: size - 1,
                ..exact
            }
        ),
        Err(SerializeError::OutputLimit)
    );
    assert_eq!(
        serialize_object(
            &value,
            WriteLimits {
                max_objects: 2,
                ..exact
            }
        ),
        Err(SerializeError::ObjectLimit)
    );
}

#[test]
fn failure_does_not_mutate_owned_input_or_return_partial_bytes() {
    let value = Object::Array(vec![
        Object::String(vec![0, 255]),
        Object::Dictionary(
            BTreeMap::from([(b"valid".to_vec(), Object::Name(vec![b'a', 0, b'b']))]).into(),
        ),
    ]);
    let before = value.clone();
    for _ in 0..2 {
        assert_eq!(
            serialize_object(&value, WriteLimits::default()),
            Err(SerializeError::NullInName)
        );
    }
    assert_eq!(value, before);
    let value = Object::Array(vec![Object::Integer(1), Object::Real(f64::INFINITY)]);
    assert_eq!(
        serialize_object(&value, WriteLimits::default()),
        Err(SerializeError::NonfiniteReal)
    );
    assert!(
        matches!(&value, Object::Array(v) if matches!(v[1], Object::Real(x) if x == f64::INFINITY))
    );
}

#[test]
fn canonical_cli_is_deterministic_and_errors_have_no_partial_stdout() {
    for (source, expected) in [
        (
            b"<< /z (\\000\\377) /a -0. >>".as_slice(),
            Some(b"<< /a -0.0 /z <00FF> >>\n".as_slice()),
        ),
        (b"[1 (unterminated", None),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_pdf-inspect"))
            .arg("--canonical")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(source).unwrap();
        let output = child.wait_with_output().unwrap();
        if let Some(expected) = expected {
            assert!(output.status.success());
            assert_eq!(output.stdout, expected);
        } else {
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
        }
    }
}
