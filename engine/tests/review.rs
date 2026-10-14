// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent adversarial review checks for the R1/R2 engine boundary.
use tandem_pdf_engine::lexer::{LexLimits, Lexer, TokenKind};
use tandem_pdf_engine::object::Object;
use tandem_pdf_engine::parser::{ParseErrorKind, ParseLimits, Parser, parse_complete, parse_one};

#[test]
fn binary_stream_suffix_does_not_invalidate_a_complete_dictionary() {
    let input = b"<< /Length 7 >>stream\n\xff\0(\\)>%\nendstream";
    let parsed = parse_one(input, ParseLimits::default()).unwrap();
    assert_eq!(parsed.span, 0..15);
    assert_eq!(
        &input[parsed.span.end..],
        b"stream\n\xff\0(\\)>%\nendstream"
    );
    assert!(matches!(parsed.object, Object::Dictionary(_)));
    assert!(parse_complete(input, ParseLimits::default()).is_err());
}

#[test]
fn all_single_binary_suffixes_preserve_a_prior_scalar() {
    for byte in 0..=255u8 {
        let input = [b'1', b' ', byte];
        let parsed = parse_one(&input, ParseLimits::default()).unwrap();
        assert_eq!(parsed.object, Object::Integer(1), "suffix {byte}");
        assert_eq!(parsed.span, 0..1, "suffix {byte}");
    }
}

#[test]
fn reference_detection_retains_intervening_tokens_and_comments() {
    let input = b"1%a\r2%b\n3%c\r\nR 4 5 6 7 8 0 R";
    let mut parser = Parser::new(input, ParseLimits::default());
    let mut objects = Vec::new();
    let mut last_end = 0;
    while let Some(parsed) = parser.parse_next().unwrap() {
        assert!(parsed.span.start >= last_end);
        assert_eq!(
            parse_complete(&input[parsed.span.clone()], ParseLimits::default()).unwrap(),
            parsed.object
        );
        last_end = parsed.span.end;
        objects.push(parsed.object);
    }
    assert_eq!(objects.len(), 7);
    assert_eq!(objects[0], Object::Integer(1));
    assert!(matches!(objects[1], Object::Reference(_)));
    assert_eq!(
        &objects[2..6],
        &[
            Object::Integer(4),
            Object::Integer(5),
            Object::Integer(6),
            Object::Integer(7)
        ]
    );
    assert!(matches!(objects[6], Object::Reference(_)));
}

#[test]
fn deferred_token_limit_does_not_erase_already_buffered_scalars() {
    let limits = ParseLimits {
        lex: LexLimits {
            max_token_bytes: 32,
            max_tokens: 2,
        },
        ..ParseLimits::default()
    };
    let mut parser = Parser::new(b"1 2 3", limits);
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
}

#[test]
fn all_byte_strings_survive_octal_and_hex_encoding() {
    let mut octal = String::from("(");
    let mut hex = String::from("<");
    for byte in 0..=255u8 {
        octal.push_str(&format!("\\{byte:03o}"));
        hex.push_str(&format!("{byte:02x}"));
    }
    octal.push(')');
    hex.push('>');
    let expected = Object::String((0..=255u8).collect());
    assert_eq!(
        parse_complete(octal.as_bytes(), ParseLimits::default()).unwrap(),
        expected
    );
    assert_eq!(
        parse_complete(hex.as_bytes(), ParseLimits::default()).unwrap(),
        expected
    );
}

#[test]
fn numeric_extremes_remain_finite_or_fail_explicitly() {
    for length in [20, 309, 310, 1000, 100_000] {
        let huge = format!("{}.0", "9".repeat(length));
        match parse_complete(huge.as_bytes(), ParseLimits::default()) {
            Ok(Object::Real(value)) => assert!(value.is_finite()),
            Err(error) => assert!(matches!(error.kind, ParseErrorKind::Lex(_))),
            other => panic!("unexpected numeric result: {other:?}"),
        }
    }
    let tiny = format!("-0.{}1", "0".repeat(1000));
    let Object::Real(value) = parse_complete(tiny.as_bytes(), ParseLimits::default()).unwrap()
    else {
        panic!()
    };
    assert_eq!(value, 0.0);
    assert!(value.is_sign_negative());
}

#[test]
fn every_prefix_of_compound_objects_terminates_with_valid_offsets() {
    let input = b"  << /A [(a\\000b) <00ff> << /B /C#20D >> 1 0 R] /N -.005 >> %end";
    for end in 0..=input.len() {
        let prefix = &input[..end];
        match parse_one(prefix, ParseLimits::default()) {
            Ok(parsed) => {
                assert!(parsed.span.start < parsed.span.end);
                assert!(parsed.span.end <= end);
                assert_eq!(
                    parse_complete(&prefix[parsed.span], ParseLimits::default()).unwrap(),
                    parsed.object
                );
            }
            Err(error) => assert!(error.offset <= end),
        }
    }
}

#[test]
fn deterministic_mixed_syntax_stress_has_bounded_progress_and_terminal_errors() {
    const ALPHABET: &[u8] = b"[]<>()/%#\\0123456789Rnulltruefalse .+-\r\n\0\xff";
    let mut seed = 0xD1B5_4A32_D192_ED03u64;
    let limits = ParseLimits {
        max_depth: 8,
        max_objects: 48,
        max_decoded_bytes: 128,
        lex: LexLimits {
            max_token_bytes: 64,
            max_tokens: 64,
        },
    };
    for case in 0..20_000usize {
        let len = case % 257;
        let mut input = Vec::with_capacity(len);
        for _ in 0..len {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            input.push(ALPHABET[seed as usize % ALPHABET.len()]);
        }
        let mut parser = Parser::new(&input, limits);
        let mut end = 0;
        let mut count = 0;
        loop {
            match parser.parse_next() {
                Ok(Some(parsed)) => {
                    assert!(parsed.span.start >= end);
                    assert!(parsed.span.end > parsed.span.start);
                    assert!(parsed.span.end <= input.len());
                    end = parsed.span.end;
                    count += 1;
                    assert!(count <= limits.max_objects);
                }
                Ok(None) => {
                    assert!(parser.parse_next().unwrap().is_none());
                    break;
                }
                Err(error) => {
                    assert!(error.offset <= input.len());
                    assert!(parser.parse_next().is_err());
                    break;
                }
            }
        }
        let mut lexer = Lexer::new(&input, limits.lex);
        let mut end = 0;
        while let Ok(Some(token)) = lexer.next_token() {
            assert!(token.span.start >= end && token.span.end > token.span.start);
            end = token.span.end;
            if let TokenKind::Real(value) = token.kind {
                assert!(value.is_finite());
            }
        }
    }
}
