// SPDX-License-Identifier: MIT
use std::collections::BTreeMap;
use tandem_pdf_engine::object::{Object, ObjectId};
use tandem_pdf_engine::parser::{
    DuplicateKeyPolicy, ParseErrorKind, ParseLimitKind, ParseLimits, Parser, parse_complete,
    parse_one,
};

fn parse(input: &[u8]) -> Object {
    parse_complete(input, ParseLimits::default()).unwrap()
}

#[test]
fn scalar_values_preserve_pdf_bytes() {
    assert_eq!(parse(b"null"), Object::Null);
    assert_eq!(parse(b"true"), Object::Boolean(true));
    assert_eq!(parse(b"false"), Object::Boolean(false));
    assert_eq!(parse(b"-42"), Object::Integer(-42));
    assert_eq!(parse(b".25"), Object::Real(0.25));
    assert_eq!(parse(b"/a#FFb"), Object::Name(vec![b'a', 255, b'b']));
    assert_eq!(parse(b"<00FF80>"), Object::String(vec![0, 255, 128]));
    assert_eq!(parse(b"(a\\000b)"), Object::String(vec![b'a', 0, b'b']));
}

#[test]
fn nested_containers_and_references() {
    let expected = Object::Dictionary(
        BTreeMap::from([
            (
                b"A".to_vec(),
                Object::Array(vec![
                    Object::Boolean(true),
                    Object::Null,
                    Object::Reference(ObjectId::new(12, 3).unwrap()),
                ]),
            ),
            (
                b"B".to_vec(),
                Object::Dictionary(
                    BTreeMap::from([(b"K".to_vec(), Object::String(b"v".to_vec()))]).into(),
                ),
            ),
        ])
        .into(),
    );
    assert_eq!(
        parse(b"<< /A [true null 12 3 R] /B << /K (v) >> >>"),
        expected
    );
    assert_eq!(parse(b"[]"), Object::Array(vec![]));
    assert_eq!(parse(b"<<>>"), Object::Dictionary(BTreeMap::new().into()));
}

#[test]
fn reference_lookahead_does_not_lose_ordinary_numbers() {
    assert_eq!(
        parse(b"[1 2 3 R 4 5 true 6]"),
        Object::Array(vec![
            Object::Integer(1),
            Object::Reference(ObjectId::new(2, 3).unwrap()),
            Object::Integer(4),
            Object::Integer(5),
            Object::Boolean(true),
            Object::Integer(6),
        ])
    );
    let mut parser = Parser::new(b"1 2 3 R 4", ParseLimits::default());
    assert_eq!(parser.parse_next().unwrap().unwrap().span, 0..1);
    let reference = parser.parse_next().unwrap().unwrap();
    assert_eq!(reference.span, 2..7);
    assert_eq!(
        reference.object,
        Object::Reference(ObjectId::new(2, 3).unwrap())
    );
    assert_eq!(
        parser.parse_next().unwrap().unwrap().object,
        Object::Integer(4)
    );
    assert!(parser.parse_next().unwrap().is_none());
}

#[test]
fn references_have_checked_ranges() {
    assert_eq!(
        parse(b"4294967295 65535 R"),
        Object::Reference(ObjectId::new(u32::MAX, u16::MAX).unwrap())
    );
    assert_eq!(
        parse(b"1 0 R"),
        Object::Reference(ObjectId::new(1, 0).unwrap())
    );
    for input in [
        b"0 0 R".as_slice(),
        b"-1 0 R",
        b"1 -1 R",
        b"4294967296 0 R",
        b"1 65536 R",
    ] {
        let error = parse_complete(input, ParseLimits::default()).unwrap_err();
        assert!(
            matches!(error.kind, ParseErrorKind::InvalidReference { .. }),
            "{input:?}: {error}"
        );
        assert_eq!(error.offset, 0);
    }
    assert!(ObjectId::new(0, 0).is_err());
}

#[test]
fn one_object_reports_its_exact_span() {
    let object = parse_one(b" \n%prefix\r\n[1 0 R]  trailing", ParseLimits::default()).unwrap();
    assert_eq!(object.span, 11..18);
    assert_eq!(
        parse_one(b"42 43 44", ParseLimits::default()).unwrap().span,
        0..2
    );
    assert_eq!(
        parse_one(b"1 (unterminated", ParseLimits::default())
            .unwrap()
            .object,
        Object::Integer(1)
    );
    assert_eq!(
        parse_one(b"1 2 (unterminated", ParseLimits::default())
            .unwrap()
            .span,
        0..1
    );
}

#[test]
fn deferred_lexical_errors_are_eventually_reported() {
    let mut parser = Parser::new(b"1 2 (unterminated", ParseLimits::default());
    assert_eq!(
        parser.parse_next().unwrap().unwrap().object,
        Object::Integer(1)
    );
    assert_eq!(
        parser.parse_next().unwrap().unwrap().object,
        Object::Integer(2)
    );
    assert!(matches!(
        parser.parse_next().unwrap_err().kind,
        ParseErrorKind::Lex(_)
    ));
    assert!(parser.parse_next().is_err());
    assert!(matches!(
        parse_complete(b"1 (unterminated", ParseLimits::default())
            .unwrap_err()
            .kind,
        ParseErrorKind::Lex(_)
    ));
}

#[test]
fn complete_input_rejects_trailing_objects_and_accepts_comments() {
    assert_eq!(
        parse(b"  true % trailing comment\r\n"),
        Object::Boolean(true)
    );
    let error = parse_complete(b"1 2", ParseLimits::default()).unwrap_err();
    assert!(matches!(error.kind, ParseErrorKind::TrailingData));
    assert_eq!(error.offset, 2);
    for input in [b"".as_slice(), b" \n%comment"] {
        assert!(matches!(
            parse_complete(input, ParseLimits::default())
                .unwrap_err()
                .kind,
            ParseErrorKind::EmptyInput
        ));
    }
}

#[test]
fn invalid_container_shapes_and_truncation_fail() {
    for input in [
        b"[".as_slice(),
        b"[1",
        b"<<",
        b"<</A",
        b"<</A 1",
        b"[<< /A [true] >>",
    ] {
        assert!(
            matches!(
                parse_complete(input, ParseLimits::default())
                    .unwrap_err()
                    .kind,
                ParseErrorKind::UnexpectedEnd { .. }
            ),
            "{input:?}"
        );
    }
    for input in [
        b"]".as_slice(),
        b">>",
        b"[>>]",
        b"<< /A >>",
        b"<< 1 2 >>",
        b"<< (key) 2 >>",
        b"<< /A 1 ]",
    ] {
        assert!(
            matches!(
                parse_complete(input, ParseLimits::default())
                    .unwrap_err()
                    .kind,
                ParseErrorKind::UnexpectedToken { .. }
            ),
            "{input:?}"
        );
    }
}

#[test]
fn keywords_are_not_general_objects() {
    for input in [b"R".as_slice(), b"True", b"undefined", b"NaN", b"q"] {
        assert!(
            matches!(
                parse_complete(input, ParseLimits::default())
                    .unwrap_err()
                    .kind,
                ParseErrorKind::UnknownKeyword(_)
            ),
            "{input:?}"
        );
    }
    for input in [
        b"stream".as_slice(),
        b"endstream",
        b"obj",
        b"endobj",
        b"xref",
        b"trailer",
        b"startxref",
    ] {
        assert!(
            matches!(
                parse_complete(input, ParseLimits::default())
                    .unwrap_err()
                    .kind,
                ParseErrorKind::Unsupported(_)
            ),
            "{input:?}"
        );
    }
    assert!(parse_complete(b"<< /Length 0 >> stream\nendstream", ParseLimits::default()).is_err());
    assert!(parse_complete(b"1 0 obj null endobj", ParseLimits::default()).is_err());
}

#[test]
fn duplicate_names_use_decoded_key_and_explicit_policy() {
    let input = b"<< /A 1 /#41 2 >>";
    let error = parse_complete(input, ParseLimits::default()).unwrap_err();
    assert!(matches!(error.kind, ParseErrorKind::DuplicateKey(ref key) if key == b"A"));
    assert_eq!(error.offset, 8);
    let value = Parser::new(input, ParseLimits::default())
        .with_duplicate_key_policy(DuplicateKeyPolicy::KeepLast)
        .parse_complete()
        .unwrap();
    assert_eq!(
        value,
        Object::Dictionary(BTreeMap::from([(b"A".to_vec(), Object::Integer(2))]).into())
    );
}

fn expect_limit(input: &[u8], limits: ParseLimits, limit: ParseLimitKind) {
    let error = parse_complete(input, limits).unwrap_err();
    assert!(
        matches!(error.kind, ParseErrorKind::LimitExceeded(actual) if actual == limit),
        "{error}"
    );
}

#[test]
fn container_depth_is_bounded_including_hard_ceiling() {
    let limits = ParseLimits {
        max_depth: 0,
        ..ParseLimits::default()
    };
    assert_eq!(parse_complete(b"1", limits).unwrap(), Object::Integer(1));
    expect_limit(b"[]", limits, ParseLimitKind::Depth);
    let limits = ParseLimits {
        max_depth: 2,
        ..ParseLimits::default()
    };
    assert!(parse_complete(b"[<< /A 1 >>]", limits).is_ok());
    expect_limit(b"[<< /A [] >>]", limits, ParseLimitKind::Depth);
    let input = [vec![b'['; 257], vec![b']'; 257]].concat();
    expect_limit(
        &input,
        ParseLimits {
            max_depth: usize::MAX,
            ..limits
        },
        ParseLimitKind::Depth,
    );
}

#[test]
fn object_limit_counts_containers_values_and_keys() {
    let limits = ParseLimits {
        max_objects: 3,
        ..ParseLimits::default()
    };
    assert!(parse_complete(b"<< /K 1 >>", limits).is_ok());
    assert!(parse_complete(b"[1 2]", limits).is_ok());
    expect_limit(b"[1 2 3]", limits, ParseLimitKind::Objects);
    expect_limit(
        b"<< /K 1 >>",
        ParseLimits {
            max_objects: 2,
            ..limits
        },
        ParseLimitKind::Objects,
    );
    expect_limit(
        b"null",
        ParseLimits {
            max_objects: 0,
            ..limits
        },
        ParseLimitKind::Objects,
    );
    let mut parser = Parser::new(
        b"1 2",
        ParseLimits {
            max_objects: 1,
            ..limits
        },
    );
    assert!(parser.parse_next().is_ok());
    assert!(matches!(
        parser.parse_next().unwrap_err().kind,
        ParseErrorKind::LimitExceeded(ParseLimitKind::Objects)
    ));
}

#[test]
fn decoded_budget_includes_names_keys_strings_and_overwritten_values() {
    let limits = ParseLimits {
        max_decoded_bytes: 3,
        ..ParseLimits::default()
    };
    assert!(parse_complete(b"<< /K (ab) >>", limits).is_ok());
    expect_limit(b"<< /K (abc) >>", limits, ParseLimitKind::DecodedBytes);
    expect_limit(b"[/ab (cd)]", limits, ParseLimitKind::DecodedBytes);
    assert!(
        parse_complete(
            b"[null 1 true]",
            ParseLimits {
                max_decoded_bytes: 0,
                ..limits
            }
        )
        .is_ok()
    );
    let error = Parser::new(b"<< /A (a) /A (b) >>", limits)
        .with_duplicate_key_policy(DuplicateKeyPolicy::KeepLast)
        .parse_complete()
        .unwrap_err();
    assert!(matches!(
        error.kind,
        ParseErrorKind::LimitExceeded(ParseLimitKind::DecodedBytes)
    ));
}

#[test]
fn lexical_limits_are_enforced_through_parser() {
    let mut limits = ParseLimits::default();
    limits.lex.max_token_bytes = 2;
    assert!(matches!(
        parse_complete(b"(abc)", limits).unwrap_err().kind,
        ParseErrorKind::Lex(_)
    ));
    let mut limits = ParseLimits::default();
    limits.lex.max_tokens = 2;
    assert!(matches!(
        parse_complete(b"[1]", limits).unwrap_err().kind,
        ParseErrorKind::Lex(_)
    ));
}

#[test]
fn malformed_errors_include_actionable_offsets() {
    let error = parse_complete(b" [1 >>]", ParseLimits::default()).unwrap_err();
    assert_eq!(error.offset, 4);
    assert!(error.to_string().contains("byte 4"));
    assert!(error.to_string().contains("expected object"));
}

fn lenient(input: &[u8]) -> Result<Object, ParseErrorKind> {
    Parser::new(input, ParseLimits::default())
        .with_lenient()
        .parse_next()
        .map(|parsed| parsed.unwrap().object)
        .map_err(|error| error.kind)
}

fn dictionary(pairs: &[(&str, Object)]) -> Object {
    let mut dictionary = tandem_pdf_engine::object::Dictionary::new();
    for (key, value) in pairs {
        dictionary.insert(key.as_bytes().to_vec(), value.clone());
    }
    Object::Dictionary(dictionary)
}

/// Lenient values as spec 01 section 4.2 lists them (observed black-box in a
/// page dictionary).
#[test]
fn lenient_values_read_as_observed() {
    let int = Object::Integer;
    let reference = |n| Object::Reference(ObjectId::new(n, 0).unwrap());
    let cases: Vec<(&[u8], Object)> = vec![
        (
            b"<</A 1e5 /B [1 1e5 3] /C 7>>",
            dictionary(&[
                ("A", Object::Null),
                ("B", Object::Array(vec![int(1), Object::Null, int(3)])),
                ("C", int(7)),
            ]),
        ),
        (
            b"<</A 1 bogus /B 3>>",
            dictionary(&[("A", Object::Null), ("B", int(3))]),
        ),
        (
            b"<</A 1 2 bogus /B 3>>",
            dictionary(&[("A", Object::Null), ("B", int(3))]),
        ),
        (
            b"<</A 1 0 R1 /B 2 /C 1 } /D 2>>",
            dictionary(&[
                ("A", Object::Null),
                ("B", int(2)),
                ("C", Object::Null),
                ("D", int(2)),
            ]),
        ),
        (
            b"<</X 4 0\xA5R /Y [1 4 0\xA5R 3] /Z 4 0 R>>",
            dictionary(&[
                ("X", Object::Null),
                (
                    "Y",
                    Object::Array(vec![int(1), int(4), Object::Null, int(3)]),
                ),
                ("Z", reference(4)),
            ]),
        ),
        (
            b"<</A endobj /B stream /C xref /D R /E nul /F true1>>",
            dictionary(&[
                ("A", Object::Null),
                ("B", Object::Null),
                ("C", Object::Null),
                ("D", Object::Null),
                ("E", Object::Null),
                ("F", Object::Null),
            ]),
        ),
        (
            b"<</A [1 bogus 2.5.5 3] /B [1 { 2 } ) 3] /C [1 0 obj 2]>>",
            dictionary(&[
                (
                    "A",
                    Object::Array(vec![int(1), Object::Null, Object::Null, int(3)]),
                ),
                (
                    "B",
                    Object::Array(vec![
                        int(1),
                        Object::Null,
                        int(2),
                        Object::Null,
                        Object::Null,
                        int(3),
                    ]),
                ),
                (
                    "C",
                    Object::Array(vec![int(1), int(0), Object::Null, int(2)]),
                ),
            ]),
        ),
        (
            b"<</MediaB792] /A [1 >> 2] /B <</X ]>> /C 1 ]>>",
            dictionary(&[
                ("MediaB792", Object::Null),
                ("A", Object::Array(vec![int(1), Object::Null, int(2)])),
                ("B", dictionary(&[("X", Object::Null)])),
                ("C", Object::Null),
            ]),
        ),
        (
            b"<</A --5 /B 5- /C <ABQD> /D 0 0 R /E [0 0 R]>>",
            dictionary(&[
                ("A", int(-5)),
                ("B", int(5)),
                ("C", Object::String(vec![0xAB, 0x0D])),
                ("D", Object::Null),
                ("E", Object::Array(vec![Object::Null])),
            ]),
        ),
    ];
    for (input, expected) in cases {
        let text = String::from_utf8_lossy(input);
        assert_eq!(
            lenient(input).unwrap_or_else(|e| panic!("{text}: {e:?}")),
            expected,
            "{text}"
        );
    }
    // A key that is not a name fails the value (spec 01 section 4.2 item 5);
    // so does a value that is itself an unknown keyword.
    for input in [
        b"<</A 1 2 /B 3>>".as_slice(),
        b"<</A (x) (y) /B 2>>",
        b"<</A {1} /B 2>>",
        b"<</A ]] /B 2>>",
        b"bogus",
        b"endobj",
    ] {
        assert!(
            lenient(input).is_err(),
            "{}",
            String::from_utf8_lossy(input)
        );
    }
    // The strict parser is unchanged.
    for input in [b"<</A 1e5>>".as_slice(), b"<</A 1 bogus /B 3>>", b"[0 0 R]"] {
        assert!(parse_complete(input, ParseLimits::default()).is_err());
    }
}
