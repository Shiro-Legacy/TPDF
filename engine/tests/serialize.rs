// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::BTreeMap;
use tandem_pdf_engine::object::{Object, ObjectId};
use tandem_pdf_engine::parser::{ParseLimits, parse_complete};
use tandem_pdf_engine::serialize::{SerializeError, WriteLimits, serialize_object};

fn write(object: &Object) -> Vec<u8> {
    serialize_object(object, WriteLimits::default()).unwrap()
}
#[test]
fn exact_canonical_syntax_and_order() {
    let value = Object::Dictionary(BTreeMap::from([
        (b"Z".to_vec(), Object::String(vec![0, 255])),
        (
            b"A /#".to_vec(),
            Object::Array(vec![
                Object::Integer(-4),
                Object::Boolean(true),
                Object::Reference(ObjectId::new(2, 3).unwrap()),
            ]),
        ),
    ]));
    assert_eq!(
        write(&value),
        b"<< /A#20#2F#23 [-4 true 2 3 R] /Z <00FF> >>"
    );
    assert_eq!(
        parse_complete(&write(&value), ParseLimits::default()).unwrap(),
        value
    );
}
#[test]
fn all_binary_string_and_nonzero_name_bytes_roundtrip() {
    for value in [
        Object::String((0..=255).collect()),
        Object::Name((1..=255).collect()),
        Object::String(vec![]),
        Object::Name(vec![]),
    ] {
        assert_eq!(
            parse_complete(&write(&value), ParseLimits::default()).unwrap(),
            value
        );
    }
}
#[test]
fn real_type_and_exact_bits_survive_extremes() {
    for value in [
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.1,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        f64::MAX,
        -f64::MAX,
        1e-100,
        1e100,
    ] {
        let bytes = write(&Object::Real(value));
        assert!(!bytes.contains(&b'e') && !bytes.contains(&b'E'));
        let Object::Real(actual) = parse_complete(&bytes, ParseLimits::default()).unwrap() else {
            panic!("real became another type")
        };
        assert_eq!(actual.to_bits(), value.to_bits());
    }
}
#[test]
fn rejects_invalid_manually_constructed_values() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            serialize_object(&Object::Real(value), WriteLimits::default()),
            Err(SerializeError::NonfiniteReal)
        );
    }
    assert_eq!(
        serialize_object(&Object::Name(vec![0]), WriteLimits::default()),
        Err(SerializeError::NullInName)
    );
    assert_eq!(
        serialize_object(
            &Object::Dictionary(BTreeMap::from([(vec![0], Object::Null)])),
            WriteLimits::default()
        ),
        Err(SerializeError::NullInName)
    );
}
#[test]
fn output_budgets_are_exact_and_zero_safe() {
    for value in [
        Object::Null,
        Object::String(vec![0, 255]),
        Object::Name(b"# /".to_vec()),
        Object::Real(-0.0),
        Object::Array(vec![]),
        Object::Dictionary(BTreeMap::new()),
    ] {
        let expected = write(&value);
        assert_eq!(
            serialize_object(
                &value,
                WriteLimits {
                    max_output_bytes: expected.len(),
                    ..WriteLimits::default()
                }
            )
            .unwrap(),
            expected
        );
        assert_eq!(
            serialize_object(
                &value,
                WriteLimits {
                    max_output_bytes: expected.len() - 1,
                    ..WriteLimits::default()
                }
            ),
            Err(SerializeError::OutputLimit)
        );
    }
    assert_eq!(
        serialize_object(
            &Object::Null,
            WriteLimits {
                max_output_bytes: 0,
                ..WriteLimits::default()
            }
        ),
        Err(SerializeError::OutputLimit)
    );
}
#[test]
fn depth_and_objects_include_dictionary_keys() {
    let value = Object::Dictionary(BTreeMap::from([(
        b"A".to_vec(),
        Object::Array(vec![Object::Integer(1)]),
    )]));
    assert!(
        serialize_object(
            &value,
            WriteLimits {
                max_depth: 2,
                max_objects: 4,
                ..WriteLimits::default()
            }
        )
        .is_ok()
    );
    assert_eq!(
        serialize_object(
            &value,
            WriteLimits {
                max_depth: 1,
                ..WriteLimits::default()
            }
        ),
        Err(SerializeError::DepthLimit)
    );
    assert_eq!(
        serialize_object(
            &value,
            WriteLimits {
                max_objects: 3,
                ..WriteLimits::default()
            }
        ),
        Err(SerializeError::ObjectLimit)
    );
    assert_eq!(
        serialize_object(
            &Object::Null,
            WriteLimits {
                max_objects: 0,
                ..WriteLimits::default()
            }
        ),
        Err(SerializeError::ObjectLimit)
    );
}
#[test]
fn integers_and_refs_preserve_ranges() {
    for value in [
        Object::Integer(i64::MIN),
        Object::Integer(i64::MAX),
        Object::Reference(ObjectId::new(u32::MAX, u16::MAX).unwrap()),
    ] {
        assert_eq!(
            parse_complete(&write(&value), ParseLimits::default()).unwrap(),
            value
        );
    }
}
#[test]
fn deterministic_generated_object_trees_roundtrip() {
    let mut seed = 0x54414e44454du64;
    for _ in 0..5000 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let value = Object::Array(vec![
            Object::Integer(seed as i64),
            Object::String(seed.to_le_bytes().to_vec()),
            Object::Real((seed as i64) as f64 / 13.0),
            Object::Boolean(seed & 1 == 0),
            Object::Null,
        ]);
        assert_eq!(
            parse_complete(&write(&value), ParseLimits::default()).unwrap(),
            value
        );
    }
}
