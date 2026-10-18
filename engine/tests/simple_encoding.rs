// SPDX-License-Identifier: MIT
use std::collections::BTreeSet;
use tandem_pdf_engine::object::{Object, ObjectId};
use tandem_pdf_engine::simple_encoding::{
    Difference, EncodingErrorKind, EncodingLimits, GlyphUnicode, NamedEncoding, SimpleEncoding,
};

fn base() -> SimpleEncoding {
    SimpleEncoding::from_named(NamedEncoding::WinAnsi)
}
fn apply(entries: &[Difference<'_>]) -> SimpleEncoding {
    base()
        .with_differences(entries, EncodingLimits::default())
        .unwrap()
}
#[test]
fn names_are_exact_and_base_tables_exhaustive() {
    let mut all_names = BTreeSet::new();
    for (name, undefined) in [
        (NamedEncoding::WinAnsi, 32),
        (NamedEncoding::MacRoman, 48),
        (NamedEncoding::Standard, 107),
        (NamedEncoding::MacExpert, 91),
    ] {
        assert_eq!(NamedEncoding::from_pdf_name(name.pdf_name()), Some(name));
        let encoding = SimpleEncoding::from_named(name);
        let mut missing = 0;
        for byte in 0..=255 {
            match encoding.glyph_name(byte) {
                None => {
                    missing += 1;
                    assert_eq!(encoding.lookup_unicode(byte), GlyphUnicode::UndefinedCode);
                }
                Some(glyph) => {
                    all_names.insert(glyph.to_vec());
                    let GlyphUnicode::Unicode(chars) = encoding.lookup_unicode(byte) else {
                        panic!("named table glyph is not mapped");
                    };
                    assert_eq!(chars.len(), 1);
                    assert_ne!(chars[0], '\0');
                }
            }
        }
        assert_eq!(missing, undefined);
    }
    assert_eq!(all_names.len(), 378);
    for unknown in [
        b"winansiencoding".as_slice(),
        b"Identity-H",
        b"PDFDocEncoding",
        b"WinAnsiEncodingX",
        b"",
    ] {
        assert!(NamedEncoding::from_pdf_name(unknown).is_none());
    }
}
#[test]
fn pdf_tables_preserve_known_codec_differences() {
    let win = base();
    for byte in [0x7f, 0x81, 0x8d, 0x8f, 0x90, 0x9d] {
        assert_eq!(
            win.lookup_unicode(byte),
            GlyphUnicode::Unicode(&['\u{2022}'])
        );
    }
    assert_eq!(win.lookup_unicode(0xa0), GlyphUnicode::Unicode(&[' ']));
    assert_eq!(win.lookup_unicode(0xad), GlyphUnicode::Unicode(&['-']));
    let mac = SimpleEncoding::from_named(NamedEncoding::MacRoman);
    assert_eq!(mac.lookup_unicode(0xca), GlyphUnicode::Unicode(&[' ']));
    assert_eq!(mac.lookup_unicode(0xdb), GlyphUnicode::Unicode(&['\u{a4}']));
    for byte in [0x00, 0x1f, 0x7f, 0xad, 0xb0, 0xb2, 0xf0] {
        assert_eq!(mac.lookup_unicode(byte), GlyphUnicode::UndefinedCode);
    }
}
#[test]
fn resets_last_assignment_and_final_255_are_explicit() {
    let enc = apply(&[
        Difference::Code(65),
        Difference::Name(b"B"),
        Difference::Name(b"C"),
        Difference::Code(65),
        Difference::Name(b"D"),
        Difference::Code(255),
        Difference::Name(b"A"),
        Difference::Code(0),
        Difference::Name(b"space"),
    ]);
    assert_eq!(enc.glyph_name(65), Some(b"D".as_slice()));
    assert_eq!(enc.lookup_unicode(66), GlyphUnicode::Unicode(&['C']));
    assert_eq!(enc.lookup_unicode(255), GlyphUnicode::Unicode(&['A']));
    assert_eq!(enc.lookup_unicode(0), GlyphUnicode::Unicode(&[' ']));
    assert_eq!(enc.glyph_name(67), base().glyph_name(67));
}
#[test]
fn undefined_notdef_unknown_and_unicode_stay_distinct() {
    let enc = apply(&[
        Difference::Code(0),
        Difference::Name(b".notdef"),
        Difference::Name(b"notInRegistry"),
        Difference::Name(b"A"),
    ]);
    assert_eq!(enc.glyph_name(0), Some(b".notdef".as_slice()));
    assert_eq!(enc.lookup_unicode(0), GlyphUnicode::UndefinedCode);
    assert_eq!(
        enc.lookup_unicode(1),
        GlyphUnicode::UnknownGlyphName(b"notInRegistry")
    );
    assert_eq!(enc.lookup_unicode(2), GlyphUnicode::Unicode(&['A']));
    assert_eq!(enc.glyph_name(3), None);
}
#[test]
fn opaque_names_are_owned_without_utf8_or_c_string_conversion() {
    let values = [
        b"A\0B".to_vec(),
        vec![0xff, 0x80, 0],
        b"A.alt".to_vec(),
        b"uni0041".to_vec(),
        b"u1F600".to_vec(),
        b"f_f_i".to_vec(),
        b"65".to_vec(),
    ];
    let enc = {
        let differences = std::iter::once(Difference::Code(0))
            .chain(values.iter().map(|v| Difference::Name(v)))
            .collect::<Vec<_>>();
        base()
            .with_differences(&differences, EncodingLimits::default())
            .unwrap()
    };
    for (code, expected) in values.iter().enumerate() {
        assert_eq!(enc.glyph_name(code as u8), Some(expected.as_slice()));
        assert_eq!(
            enc.lookup_unicode(code as u8),
            GlyphUnicode::UnknownGlyphName(expected)
        );
    }
    drop(values);
    assert_eq!(enc.glyph_name(0), Some(b"A\0B".as_slice()));
}
#[test]
fn invalid_difference_order_and_empty_name_have_exact_indices() {
    for (diff, index, kind) in [
        (
            vec![Difference::Name(b"A")],
            0,
            EncodingErrorKind::NameBeforeCode,
        ),
        (
            vec![
                Difference::Code(255),
                Difference::Name(b"A"),
                Difference::Name(b"B"),
            ],
            2,
            EncodingErrorKind::CodeOverflow,
        ),
        (
            vec![Difference::Code(0), Difference::Name(b"")],
            1,
            EncodingErrorKind::EmptyName,
        ),
    ] {
        let err = base()
            .with_differences(&diff, EncodingLimits::default())
            .unwrap_err();
        assert_eq!(err.index, Some(index));
        assert_eq!(err.kind, kind);
    }
}
#[test]
fn object_adapter_checks_integers_types_and_array_shape() {
    assert_eq!(
        base()
            .with_object_differences(&Object::Null, EncodingLimits::default())
            .unwrap_err()
            .kind,
        EncodingErrorKind::ExpectedArray
    );
    for value in [
        Object::Integer(-1),
        Object::Integer(256),
        Object::Integer(i64::MAX),
    ] {
        let err = base()
            .with_object_differences(&Object::Array(vec![value]), EncodingLimits::default())
            .unwrap_err();
        assert_eq!(err.index, Some(0));
        assert_eq!(err.kind, EncodingErrorKind::CodeOutOfRange);
    }
    for value in [
        Object::Real(1.),
        Object::Boolean(false),
        Object::Null,
        Object::String(b"A".to_vec()),
        Object::Array(vec![]),
        Object::Reference(ObjectId::new(1, 0).unwrap()),
        Object::Dictionary(Default::default()),
    ] {
        let err = base()
            .with_object_differences(
                &Object::Array(vec![Object::Integer(0), value]),
                EncodingLimits::default(),
            )
            .unwrap_err();
        assert_eq!(err.index, Some(1));
        assert_eq!(err.kind, EncodingErrorKind::ExpectedCodeOrName);
    }
    let arr = Object::Array(vec![Object::Integer(255), Object::Name(b"A".to_vec())]);
    assert_eq!(
        base()
            .with_object_differences(&arr, EncodingLimits::default())
            .unwrap()
            .lookup_unicode(255),
        GlyphUnicode::Unicode(&['A'])
    );
}
#[test]
fn exact_limits_and_overwritten_names_are_charged() {
    let differences = [
        Difference::Code(0),
        Difference::Name(b"XYZ"),
        Difference::Code(0),
        Difference::Name(b"ABC"),
    ];
    let limits = EncodingLimits {
        max_items: 4,
        max_name_bytes: 3,
        max_processed_name_bytes: 6,
    };
    assert_eq!(
        base()
            .with_differences(&differences, limits)
            .unwrap()
            .glyph_name(0),
        Some(b"ABC".as_slice())
    );
    for limited in [
        EncodingLimits {
            max_items: 3,
            ..limits
        },
        EncodingLimits {
            max_name_bytes: 2,
            ..limits
        },
        EncodingLimits {
            max_processed_name_bytes: 5,
            ..limits
        },
    ] {
        assert!(matches!(
            base()
                .with_differences(&differences, limited)
                .unwrap_err()
                .kind,
            EncodingErrorKind::LimitExceeded(_)
        ));
    }
    let err = base()
        .with_differences(
            &differences,
            EncodingLimits {
                max_processed_name_bytes: 5,
                ..limits
            },
        )
        .unwrap_err();
    assert_eq!(err.index, Some(3));
}
#[test]
fn zero_budgets_allow_empty_differences_and_base_is_immutable_on_failure() {
    let before = apply(&[Difference::Code(65), Difference::Name(b"ownedUnknown")]);
    let result = before
        .with_differences(
            &[],
            EncodingLimits {
                max_items: 0,
                max_name_bytes: 0,
                max_processed_name_bytes: 0,
            },
        )
        .unwrap();
    assert_eq!(before, result);
    let err = before.with_differences(
        &[
            Difference::Code(65),
            Difference::Name(b"B"),
            Difference::Name(b""),
        ],
        EncodingLimits::default(),
    );
    assert!(err.is_err());
    assert_eq!(before.glyph_name(65), Some(b"ownedUnknown".as_slice()));
}
#[test]
fn item_work_cap_counts_code_only_entries() {
    let entries = vec![Difference::Code(0); 4096];
    assert!(
        base()
            .with_differences(&entries, EncodingLimits::default())
            .is_ok()
    );
    let entries = vec![Difference::Code(0); 4097];
    let err = base()
        .with_differences(&entries, EncodingLimits::default())
        .unwrap_err();
    assert_eq!(err.index, None);
    assert!(matches!(err.kind, EncodingErrorKind::LimitExceeded(_)));
}
#[test]
fn default_name_length_and_processed_limits_are_enforced() {
    let name = vec![b'x'; 127];
    assert!(
        base()
            .with_differences(
                &[Difference::Code(0), Difference::Name(&name)],
                EncodingLimits::default()
            )
            .is_ok()
    );
    let longer = vec![b'x'; 128];
    assert!(
        base()
            .with_differences(
                &[Difference::Code(0), Difference::Name(&longer)],
                EncodingLimits::default()
            )
            .is_err()
    );
    let mut entries = Vec::new();
    for _ in 0..12 {
        entries.push(Difference::Code(0));
        entries.extend(std::iter::repeat_n(Difference::Name(&name), 255));
    }
    let err = base()
        .with_differences(&entries, EncodingLimits::default())
        .unwrap_err();
    assert_eq!(
        err.kind,
        EncodingErrorKind::LimitExceeded("processed name bytes")
    );
}
#[test]
fn all_table_registry_names_work_as_differences_and_aliases_are_exact() {
    let mut names = BTreeSet::new();
    for base_name in [
        NamedEncoding::WinAnsi,
        NamedEncoding::MacRoman,
        NamedEncoding::Standard,
        NamedEncoding::MacExpert,
    ] {
        let enc = SimpleEncoding::from_named(base_name);
        for code in 0..=255 {
            if let Some(name) = enc.glyph_name(code) {
                if names.insert(name.to_vec()) {
                    let changed = apply(&[Difference::Code(0), Difference::Name(name)]);
                    assert_eq!(changed.lookup_unicode(0), enc.lookup_unicode(code));
                }
            }
        }
    }
    assert_eq!(names.len(), 378);
}
