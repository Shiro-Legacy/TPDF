// SPDX-License-Identifier: MIT
//! Acceptance cases for spec 01 (PDF syntax), section 6.3: the new cases the
//! clean-room rewrite must meet. Every input is built here.
use tandem_pdf_engine::content::{ContentLimits, ContentParser};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::lexer::{LexLimits, Lexer, TokenKind};
use tandem_pdf_engine::object::{
    Dictionary, Object, ObjectId, decode_text_string, encode_text_string,
};
use tandem_pdf_engine::parser::{DuplicateKeyPolicy, ParseLimits, Parser, parse_complete};
use tandem_pdf_engine::serialize::{SerializeStyle, WriteLimits, serialize_object_with_style};
use tandem_pdf_engine::writer::{DocumentWriteLimits, WriteOptions, write_document};

fn compact(value: &Object) -> String {
    String::from_utf8(
        serialize_object_with_style(value, WriteLimits::default(), SerializeStyle::Compact)
            .unwrap(),
    )
    .unwrap()
}

fn lenient(input: &[u8]) -> Object {
    Parser::new(input, ParseLimits::default())
        .with_duplicate_key_policy(DuplicateKeyPolicy::KeepLast)
        .with_lenient()
        .parse_complete()
        .unwrap()
}

/// A classic-xref PDF of the given objects.
fn pdf(objects: &[(u32, &[u8])]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let size = objects.iter().map(|(n, _)| *n).max().unwrap_or(0) + 1;
    let mut offsets = vec![None; size as usize];
    for (number, body) in objects {
        offsets[*number as usize] = Some(out.len());
        out.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for offset in &offsets[1..] {
        match offset {
            Some(offset) => out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes()),
            None => out.extend_from_slice(b"0000000000 00000 f \n"),
        }
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    out
}

fn app_save(bytes: &[u8]) -> String {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    let written = write_document(&doc, DocumentWriteLimits::default(), WriteOptions::APP).unwrap();
    String::from_utf8_lossy(&written).into_owned()
}

/// 6.3 case 1 (section 4.1 item 1): in lenient mode `#00` stands for itself.
#[test]
fn a_lenient_null_escape_stands_for_itself() {
    let mut lexer = Lexer::new(b"/A#00B /A#G0 /A#0 /A#41", LexLimits::default()).with_lenient();
    let mut names = Vec::new();
    while let Some(token) = lexer.next_token().unwrap() {
        names.push(token.kind);
    }
    assert_eq!(
        names,
        [
            TokenKind::Name(b"A#00B".to_vec()),
            TokenKind::Name(b"A#G0".to_vec()),
            TokenKind::Name(b"A#0".to_vec()),
            TokenKind::Name(b"AA".to_vec()),
        ]
    );
    assert_eq!(
        lenient(b"<</A#00B 1 /K [/A#00B]>>"),
        Object::Dictionary(Dictionary::from([
            (b"A#00B".to_vec(), Object::Integer(1)),
            (
                b"K".to_vec(),
                Object::Array(vec![Object::Name(b"A#00B".to_vec())])
            ),
        ]))
    );
    // Strict mode still refuses it.
    assert!(
        Lexer::new(b"/A#00B", LexLimits::default())
            .next_token()
            .is_err()
    );
}

/// 6.3 case 2 (section 3.7.2): an empty name keeps a space before a regular
/// token, references and reals included, and reads back identically.
#[test]
fn empty_names_before_references_and_reals_read_back() {
    let empty = || Object::Name(Vec::new());
    let reference = Object::Reference(ObjectId::new(1, 0).unwrap());
    for (value, expected) in [
        (
            Object::Dictionary(Dictionary::from([(Vec::new(), Object::Integer(5))])),
            "<</ 5>>",
        ),
        (
            Object::Array(vec![empty(), empty(), Object::Integer(3)]),
            "[// 3]",
        ),
        (
            Object::Array(vec![
                empty(),
                reference.clone(),
                empty(),
                Object::Real(2.5),
                empty(),
            ]),
            "[/ 1 0 R/ 2.5/]",
        ),
        (
            Object::Dictionary(Dictionary::from([
                (Vec::new(), reference.clone()),
                (b"A".to_vec(), empty()),
                (b"B".to_vec(), Object::Real(-0.25)),
            ])),
            "<</ 1 0 R/A//B -.25>>",
        ),
    ] {
        let text = compact(&value);
        assert_eq!(text, expected);
        assert_eq!(
            parse_complete(text.as_bytes(), ParseLimits::default()).unwrap(),
            value,
            "{text}"
        );
    }
}

/// 6.3 case 3 (section 3.7.5): for random binary64 values in every exponent
/// band, subnormals and both zeros, the compact token reads back as the same
/// bits (or the equal Integer for integral values below 2^63), holds only
/// `-`, digits and at most one `.`, and never starts with `0.` or `-0.`.
#[test]
fn compact_reals_read_back_to_the_same_bits() {
    const TWO_63: f64 = 9_223_372_036_854_775_808_u64 as f64;
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut values = vec![
        0.0,
        -0.0,
        f64::MIN_POSITIVE,
        5e-324,
        -5e-324,
        f64::MAX,
        -f64::MAX,
    ];
    for i in 0..100_000u64 {
        let bits = next();
        let value = match i % 4 {
            // Any finite value.
            0 => f64::from_bits(bits),
            // Subnormals.
            1 => f64::from_bits(bits & 0x800F_FFFF_FFFF_FFFF),
            // Exponents near 1, where most PDF numbers live.
            2 => {
                f64::from_bits((bits & 0x800F_FFFF_FFFF_FFFF) | ((1013 + (bits >> 52) % 40) << 52))
            }
            // Short decimals.
            _ => (bits % 2_000_000) as f64 / 1000.0 - 1000.0,
        };
        if value.is_finite() {
            values.push(value);
        }
    }
    for value in values {
        let text = compact(&Object::Real(value));
        assert!(
            text.bytes()
                .all(|b| b == b'-' || b == b'.' || b.is_ascii_digit()),
            "{value:e}: {text}"
        );
        assert!(text.matches('.').count() <= 1, "{value:e}: {text}");
        assert!(
            !text.starts_with("0.") && !text.starts_with("-0."),
            "{value:e}: {text}"
        );
        let read = parse_complete(text.as_bytes(), ParseLimits::default())
            .unwrap_or_else(|e| panic!("{value:e}: {text}: {e}"));
        if value.fract() == 0.0 && (-TWO_63..TWO_63).contains(&value) {
            assert_eq!(read, Object::Integer(value as i64), "{value:e}: {text}");
        } else {
            let Object::Real(back) = read else {
                panic!("{value:e}: {text} read as {read:?}");
            };
            assert_eq!(back.to_bits(), value.to_bits(), "{value:e}: {text}");
        }
    }
}

/// 6.3 case 4 (section 3.7.3): a /Type that prints as null (a reference to a
/// missing object) is not moved, and a /Subtype that prints as null is not
/// moved either.
#[test]
fn a_type_that_prints_as_null_keeps_the_original_order() {
    let text = app_save(&pdf(&[
        (1, b"<</Pages 2 0 R/Type/Catalog/A 3 0 R/B 4 0 R/C 6 0 R>>"),
        (2, b"<</Kids[]/Count 0/Type/Pages>>"),
        (3, b"<</X 1/Type 9 0 R/Subtype/S>>"),
        (4, b"<</X 1/Subtype 9 0 R/Type/T>>"),
        (6, b"<</X 1/Subtype/S/Type 7 0 R>>"),
        (7, b"/T"),
    ]));
    for expected in [
        "3 0 obj\n<</X 1/Type null/Subtype/S>>\nendobj\n",
        "4 0 obj\n<</Type/T/X 1/Subtype null>>\nendobj\n",
        "6 0 obj\n<</Type 7 0 R/Subtype/S/X 1>>\nendobj\n",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
}

/// Owner decision S3: /Type then /Subtype go first in stream dictionaries
/// too; /Length keeps its place or goes last.
#[test]
fn stream_dictionaries_put_type_and_subtype_first() {
    let bytes = pdf(&[
        (1, b"<</Type/Catalog/Pages 2 0 R/X 3 0 R>>"),
        (2, b"<</Type/Pages/Kids[]/Count 0>>"),
        (
            3,
            b"<</Length 3/BBox[0 0 1 1]/Subtype/Form/Type/XObject>>\nstream\nq Q\nendstream",
        ),
    ]);
    let mut doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    // A new stream without /Length gets it last, after the moved keys.
    let entries = Dictionary::from([
        (b"BBox".to_vec(), Object::Array(vec![Object::Integer(0); 4])),
        (b"Subtype".to_vec(), Object::Name(b"Form".to_vec())),
        (b"Type".to_vec(), Object::Name(b"XObject".to_vec())),
    ]);
    let new = doc
        .allocate(Object::Dictionary(entries), Some(b"q Q".to_vec()))
        .unwrap();
    let catalog_id = ObjectId::new(1, 0).unwrap();
    let mut catalog = doc
        .object(catalog_id)
        .unwrap()
        .value
        .as_dict()
        .unwrap()
        .clone();
    catalog.insert(b"Y".to_vec(), Object::Reference(new));
    doc.replace_value(catalog_id, Object::Dictionary(catalog))
        .unwrap();
    let written = write_document(&doc, DocumentWriteLimits::default(), WriteOptions::APP).unwrap();
    let text = String::from_utf8_lossy(&written);
    for expected in [
        "3 0 obj\n<</Type/XObject/Subtype/Form/Length 3/BBox[0 0 1 1]>>\nstream\n",
        "4 0 obj\n<</Type/XObject/Subtype/Form/BBox[0 0 0 0]/Length 3>>\nstream\n",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
}

/// 6.3 case 5 (section 3.6).
#[test]
fn text_strings_follow_the_marks_and_reject_accent_codes() {
    let encoded = encode_text_string("\u{18}x");
    assert_eq!(encoded, [0xFE, 0xFF, 0x00, 0x18, 0x00, b'x']);
    assert_eq!(decode_text_string(&encoded), "\u{18}x");
    for text in [
        "\u{1F}",
        "a\u{19}b",
        "\u{18}",
        "plain",
        "",
        "caf\u{e9}",
        "\u{1F600}",
    ] {
        assert_eq!(
            decode_text_string(&encode_text_string(text)),
            text,
            "{text:?}"
        );
    }
    assert_eq!(
        decode_text_string(&[0xFE, 0xFF, 0xD8, 0x00, 0x00, 0x41]),
        "\u{FFFD}A"
    );
    assert_eq!(
        decode_text_string(&[0xEF, 0xBB, 0xBF, 0xC3, 0x41]),
        "\u{FFFD}A"
    );
    // A little-endian mark is read too.
    assert_eq!(
        decode_text_string(&[0xFF, 0xFE, 0x41, 0x00, 0x2D, 0x4E]),
        "A\u{4E2D}"
    );
}

/// 6.3 case 6 (section 2.3): object size and copy-on-write sharing.
#[test]
fn dictionaries_share_until_written() {
    #[cfg(target_pointer_width = "64")]
    assert_eq!(std::mem::size_of::<Object>(), 32);
    let base: Dictionary = (0..20)
        .map(|i| (format!("K{i}").into_bytes(), Object::Integer(i)))
        .collect();
    type Write = (&'static str, fn(&mut Dictionary));
    let writes: [Write; 7] = [
        ("insert", |d| {
            d.insert(b"New".to_vec(), Object::Null);
        }),
        ("remove", |d| {
            d.remove(b"K3".as_slice());
        }),
        ("retain", |d| d.retain(|_, _| true)),
        ("clear", |d| d.clear()),
        ("get_mut", |d| {
            d.get_mut(b"K4".as_slice());
        }),
        ("iter_mut", |d| d.iter_mut().for_each(drop)),
        ("values_mut", |d| d.values_mut().for_each(drop)),
    ];
    for (label, write) in writes {
        // Either side may be the one written.
        for side in 0..2 {
            let mut a = base.clone();
            let mut b = a.clone();
            assert!(a.shares_entries(&b) && b.shares_entries(&a), "{label}");
            if side == 0 {
                write(&mut a);
            } else {
                write(&mut b);
            }
            assert!(
                !a.shares_entries(&b) && !b.shares_entries(&a),
                "{label} side {side}"
            );
            assert_eq!(base.len(), 20, "{label}");
            assert_eq!(base.get(b"K3".as_slice()), Some(&Object::Integer(3)));
        }
    }
    // Reading, and get_mut of an absent key, do not write.
    let mut a = base.clone();
    let b = a.clone();
    assert!(a.get_mut(b"Absent".as_slice()).is_none());
    assert!(a.get(b"K1".as_slice()).is_some());
    assert_eq!(a.iter().count(), 20);
    assert!(a.shares_entries(&b) && b.shares_entries(&a));
    // A written clone leaves nested dictionaries an edit does not reach
    // shared.
    let mut outer = Dictionary::new();
    outer.insert(b"Inner".to_vec(), Object::Dictionary(base.clone()));
    outer.insert(b"Other".to_vec(), Object::Integer(1));
    let mut copy = outer.clone();
    copy.insert(b"Other".to_vec(), Object::Integer(2));
    let inner = |d: &Dictionary| {
        d.get(b"Inner".as_slice())
            .unwrap()
            .as_dict()
            .unwrap()
            .clone()
    };
    assert!(inner(&copy).shares_entries(&inner(&outer)));
    assert_eq!(outer.get(b"Other".as_slice()), Some(&Object::Integer(1)));
}

/// 6.3 case 7 (section 2.1, `seek`): after inline image data, tokenizing
/// resumes past the data, whatever bytes the data holds; data that ends the
/// input ends the operations.
#[test]
fn tokenizing_resumes_after_inline_image_data() {
    let operators = |bytes: &[u8]| -> Vec<Vec<u8>> {
        let mut parser = ContentParser::new(bytes, ContentLimits::default());
        let mut out = Vec::new();
        while let Some(operation) = parser.next_operation().unwrap() {
            out.push(operation.operator);
        }
        out
    };
    // Two data bytes that would otherwise open a literal and a hex string.
    assert_eq!(
        operators(b"q BI /W 2 /H 1 /CS /G /BPC 8 ID (< EI 1 0 0 rg Q"),
        [b"q".to_vec(), b"BI".to_vec(), b"rg".to_vec(), b"Q".to_vec()]
    );
    assert_eq!(
        operators(b"q BI /W 1 /H 1 /CS /G /BPC 8 ID ) EI"),
        [b"q".to_vec(), b"BI".to_vec()]
    );
}
