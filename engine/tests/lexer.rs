use tandem_pdf_engine::lexer::{LexErrorKind, LexLimits, Lexer, Token, TokenKind};

fn tokens(bytes: &[u8]) -> Vec<Token> {
    let mut lexer = Lexer::new(bytes, LexLimits::default());
    let mut tokens = Vec::new();
    while let Some(token) = lexer.next_token().expect("valid PDF token") {
        tokens.push(token);
    }
    tokens
}
fn kinds(bytes: &[u8]) -> Vec<TokenKind> {
    tokens(bytes).into_iter().map(|t| t.kind).collect()
}
fn error(bytes: &[u8]) -> LexErrorKind {
    Lexer::new(bytes, LexLimits::default())
        .next_token()
        .unwrap_err()
        .kind
}

#[test]
fn delimiters_spans_and_keywords() {
    let input = b" \t[<< /A 1 >>]true false null R obj endobj stream";
    let values = tokens(input);
    assert_eq!(
        values[0],
        Token {
            kind: TokenKind::ArrayStart,
            span: 2..3
        }
    );
    assert_eq!(
        values[1],
        Token {
            kind: TokenKind::DictStart,
            span: 3..5
        }
    );
    assert_eq!(
        values[2],
        Token {
            kind: TokenKind::Name(b"A".to_vec()),
            span: 6..8
        }
    );
    assert_eq!(
        values[3],
        Token {
            kind: TokenKind::Integer(1),
            span: 9..10
        }
    );
    assert_eq!(values[4].kind, TokenKind::DictEnd);
    assert_eq!(values[5].kind, TokenKind::ArrayEnd);
    for (token, text) in values[6..]
        .iter()
        .zip(["true", "false", "null", "R", "obj", "endobj", "stream"])
    {
        assert_eq!(token.kind, TokenKind::Keyword(text.as_bytes().to_vec()));
        assert_eq!(&input[token.span.clone()], text.as_bytes());
    }
}

#[test]
fn all_pdf_whitespace_and_comments_are_trivia() {
    assert_eq!(
        kinds(b"\0\t\n\x0c\r %comment\r\n42%until CR\r/A%EOF"),
        vec![TokenKind::Integer(42), TokenKind::Name(b"A".to_vec())]
    );
    for input in [b"".as_slice(), b"%EOF", b"\0\t\n\x0c\r "] {
        let mut lexer = Lexer::new(input, LexLimits::default());
        assert_eq!(lexer.next_token().unwrap(), None);
        assert_eq!(lexer.position(), input.len());
        assert_eq!(lexer.next_token().unwrap(), None);
    }
    // Vertical tab is not PDF whitespace.
    assert_eq!(error(b"\x0b"), LexErrorKind::UnexpectedByte(11));
}

#[test]
fn names_decode_hex_and_preserve_arbitrary_bytes() {
    assert_eq!(
        kinds(b"/ /A#20B /#2f#28#29#3C#3e#5B#5d#23 /\xff\x01"),
        vec![
            TokenKind::Name(vec![]),
            TokenKind::Name(b"A B".to_vec()),
            TokenKind::Name(b"/()<>[]#".to_vec()),
            TokenKind::Name(vec![255, 1]),
        ]
    );
}

#[test]
fn invalid_name_escapes_and_nulls_are_explicit_errors() {
    for input in [b"/#".as_slice(), b"/#0", b"/#G0", b"/#0G", b"/#0 ", b"/#0/"] {
        assert_eq!(error(input), LexErrorKind::InvalidNameEscape);
    }
    assert_eq!(error(b"/#00"), LexErrorKind::NullInName);
    assert_eq!(kinds(b"/#01"), vec![TokenKind::Name(vec![1])]);
}

#[test]
fn name_boundaries_do_not_consume_next_token() {
    assert_eq!(
        kinds(b"/A/B[]<<>>()%comment"),
        vec![
            TokenKind::Name(b"A".to_vec()),
            TokenKind::Name(b"B".to_vec()),
            TokenKind::ArrayStart,
            TokenKind::ArrayEnd,
            TokenKind::DictStart,
            TokenKind::DictEnd,
            TokenKind::String(vec![]),
        ]
    );
}

#[test]
fn literal_strings_nest_without_recursion() {
    assert_eq!(
        kinds(b"(a(b(c)d)e)()"),
        vec![
            TokenKind::String(b"a(b(c)d)e".to_vec()),
            TokenKind::String(vec![])
        ]
    );
    let mut nested = vec![b'('; 100_000];
    nested.extend(std::iter::repeat_n(b')', 100_000));
    match &kinds(&nested)[0] {
        TokenKind::String(bytes) => assert_eq!(bytes.len(), 199_998),
        _ => panic!(),
    }
}

#[test]
fn literal_string_escapes_and_octal_low_byte() {
    let input = br"(\n\r\t\b\f\(\)\\\q\0\12\123\777\400\378\8)";
    let expected = b"\n\r\t\x08\x0c()\\q\0\nS\xff\0\x1f88";
    assert_eq!(kinds(input), vec![TokenKind::String(expected.to_vec())]);
}

#[test]
fn literal_string_eol_normalization_and_continuation() {
    assert_eq!(
        kinds(b"(a\rb\nc\r\nd\n\re)"),
        vec![TokenKind::String(b"a\nb\nc\nd\n\ne".to_vec())]
    );
    assert_eq!(
        kinds(b"(a\\\rb\\\nc\\\r\nd)"),
        vec![TokenKind::String(b"abcd".to_vec())]
    );
    assert_eq!(kinds(b"(\0\xff)"), vec![TokenKind::String(vec![0, 255])]);
}

#[test]
fn unterminated_literal_strings_and_dangling_escape() {
    for input in [
        b"(".as_slice(),
        b"(abc",
        b"(a(b)",
        b"(\\",
        b"(\\1",
        b"(\\12",
        b"(\\\r",
    ] {
        assert_eq!(error(input), LexErrorKind::UnterminatedString);
    }
}

#[test]
fn hex_strings_ignore_whitespace_and_pad_last_nibble() {
    assert_eq!(
        kinds(b"<> <0> <Ff0> <0 1\0\t\n\x0c\rA b>"),
        vec![
            TokenKind::String(vec![]),
            TokenKind::String(vec![0]),
            TokenKind::String(vec![255, 0]),
            TokenKind::String(vec![1, 171]),
        ]
    );
}

#[test]
fn malformed_hex_strings() {
    for input in [b"<".as_slice(), b"<0", b"<abcd "] {
        assert_eq!(error(input), LexErrorKind::UnterminatedHexString);
    }
    for (input, byte) in [
        (b"<G>".as_slice(), b'G'),
        (b"<0%comment>", b'%'),
        (b"<\x0b>", 11),
    ] {
        assert_eq!(error(input), LexErrorKind::InvalidHexDigit(byte));
    }
}

#[test]
fn integer_and_real_grammar() {
    assert_eq!(
        kinds(b"0 -0 +0 0001 -9223372036854775808 +9223372036854775807 1. .5 -.5 +.5 -0. +001.25"),
        vec![
            TokenKind::Integer(0),
            TokenKind::Integer(0),
            TokenKind::Integer(0),
            TokenKind::Integer(1),
            TokenKind::Integer(i64::MIN),
            TokenKind::Integer(i64::MAX),
            TokenKind::Real(1.0),
            TokenKind::Real(0.5),
            TokenKind::Real(-0.5),
            TokenKind::Real(0.5),
            TokenKind::Real(-0.0),
            TokenKind::Real(1.25),
        ]
    );
    let TokenKind::Real(zero) = kinds(b"-0.")[0] else {
        panic!()
    };
    assert!(zero.is_sign_negative());
}

#[test]
fn rejects_numeric_recovery_and_exponents() {
    for input in [
        b"+".as_slice(),
        b"-",
        b".",
        b"+.",
        b"-.",
        b"--1",
        b"1.2.3",
        b"1e3",
        b"1-2",
        b"1+2",
        b"123abc",
    ] {
        assert!(
            matches!(error(input), LexErrorKind::InvalidNumber),
            "{input:?}"
        );
    }
}

#[test]
fn reads_integer_overflow_as_real_and_preserves_f64_precision() {
    // Past i64 an integer is the real its digits name (never wrapped).
    assert_eq!(
        kinds(b"9223372036854775808 -9223372036854775809 1000000000000000000000000000000"),
        vec![
            TokenKind::Real(9_223_372_036_854_775_808.0),
            TokenKind::Real(-9_223_372_036_854_775_809.0),
            TokenKind::Real(1e30),
        ]
    );
    assert_eq!(
        error("9".repeat(400).as_bytes()),
        LexErrorKind::NumberOutOfRange
    );
    let huge = format!("{}.0", "9".repeat(400));
    assert_eq!(error(huge.as_bytes()), LexErrorKind::NumberOutOfRange);
    assert_eq!(kinds(b"16777217.0"), vec![TokenKind::Real(16_777_217.0)]);
}

#[test]
fn invalid_delimiters_and_nonprintable_keywords() {
    for byte in [b')', b'>', b'{', b'}', 1, 127, 255] {
        assert_eq!(error(&[byte]), LexErrorKind::UnexpectedByte(byte));
    }
    assert_eq!(
        kinds(b"NaN infinity abc#20def"),
        vec![
            TokenKind::Keyword(b"NaN".to_vec()),
            TokenKind::Keyword(b"infinity".to_vec()),
            TokenKind::Keyword(b"abc#20def".to_vec()),
        ]
    );
}

#[test]
fn token_byte_limit_exact_boundary_for_every_kind() {
    for input in [
        b"[]".as_slice(),
        b"<<",
        b">>",
        b"/A#20B",
        b"(a\\\r\nb)",
        b"<1 2>",
        b"keyword",
        b"-123",
        b"+1.5",
    ] {
        let first_end = if input == b"[]" { 1 } else { input.len() };
        let mut exact = Lexer::new(
            input,
            LexLimits {
                max_token_bytes: first_end,
                max_tokens: 10,
            },
        );
        assert_eq!(exact.next_token().unwrap().unwrap().span.end, first_end);
        let mut short = Lexer::new(
            input,
            LexLimits {
                max_token_bytes: first_end - 1,
                max_tokens: 10,
            },
        );
        let failure = short.next_token().unwrap_err();
        assert_eq!(
            failure.kind,
            LexErrorKind::TokenTooLong {
                limit: first_end - 1
            }
        );
        assert_eq!(failure.offset, first_end - 1);
    }
}

#[test]
fn token_limit_counts_encoded_bytes_even_if_no_output() {
    for input in [b"<          >".as_slice(), b"(\\\n\\\n\\\n)"] {
        let mut lexer = Lexer::new(
            input,
            LexLimits {
                max_token_bytes: 4,
                max_tokens: 10,
            },
        );
        assert_eq!(
            lexer.next_token().unwrap_err().kind,
            LexErrorKind::TokenTooLong { limit: 4 }
        );
    }
}

#[test]
fn token_count_limit_allows_trivia_and_eof_at_exact_budget() {
    let limits = LexLimits {
        max_token_bytes: 10,
        max_tokens: 1,
    };
    let mut exact = Lexer::new(b"1 %comment", limits);
    assert!(exact.next_token().unwrap().is_some());
    assert_eq!(exact.next_token().unwrap(), None);
    let mut over = Lexer::new(b"1 \t 2", limits);
    over.next_token().unwrap();
    let failure = over.next_token().unwrap_err();
    assert_eq!(failure.kind, LexErrorKind::TooManyTokens { limit: 1 });
    assert_eq!(failure.offset, 4);
    assert_eq!(
        Lexer::new(
            b" %comment",
            LexLimits {
                max_tokens: 0,
                max_token_bytes: 0
            }
        )
        .next_token()
        .unwrap(),
        None
    );
    assert_eq!(
        Lexer::new(
            b"1",
            LexLimits {
                max_tokens: 0,
                max_token_bytes: 0
            }
        )
        .next_token()
        .unwrap_err()
        .kind,
        LexErrorKind::TooManyTokens { limit: 0 }
    );
}

#[test]
fn failures_are_sticky_with_stable_offsets() {
    let mut lexer = Lexer::new(b" /#GG 42", LexLimits::default());
    let failure = lexer.next_token().unwrap_err();
    assert_eq!(failure.offset, 2);
    let position = lexer.position();
    assert_eq!(lexer.next_token().unwrap_err(), failure);
    assert_eq!(lexer.position(), position);
    assert!(failure.to_string().contains("byte 2"));
}

#[test]
fn every_byte_and_deterministic_hostile_inputs_terminate_within_bounds() {
    let limits = LexLimits {
        max_token_bytes: 128,
        max_tokens: 16,
    };
    let mut state = 0x717u64;
    for length in 0..512 {
        let mut input = Vec::with_capacity(length);
        for _ in 0..length {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            input.push(state as u8);
        }
        let mut lexer = Lexer::new(&input, limits);
        let mut count = 0;
        while let Ok(Some(token)) = lexer.next_token() {
            count += 1;
            assert!(count <= limits.max_tokens);
            assert!(token.span.start < token.span.end);
            assert!(token.span.len() <= limits.max_token_bytes);
            assert!(token.span.end <= input.len());
        }
        assert!(lexer.position() <= input.len());
    }
    for byte in 0..=255u8 {
        let input = [byte; 256];
        let mut lexer = Lexer::new(&input, limits);
        for _ in 0..=limits.max_tokens {
            if !matches!(lexer.next_token(), Ok(Some(_))) {
                break;
            }
        }
        assert!(lexer.position() <= input.len());
    }
}

fn lenient(bytes: &[u8]) -> Vec<TokenKind> {
    let mut lexer = Lexer::new(bytes, LexLimits::default()).with_lenient();
    let mut kinds = Vec::new();
    while let Some(token) = lexer.next_token().expect("lenient token") {
        kinds.push(token.kind);
    }
    kinds
}

/// Each damaged token reads as spec 01 section 4.1 item 4 lists it: a keyword
/// where a dictionary value reads as null, otherwise the number stated.
#[test]
fn lenient_numbers_read_as_observed() {
    let keyword = |text: &str| TokenKind::Keyword(text.as_bytes().to_vec());
    let cases: [(&str, TokenKind); 31] = [
        ("1e5", keyword("1e5")),
        ("1.0E-4", keyword("1.0E-4")),
        ("1.2.3", keyword("1.2.3")),
        ("1..5", keyword("1..5")),
        (".5.", keyword(".5.")),
        ("-+5", keyword("-+5")),
        ("1,5", keyword("1,5")),
        ("12abc", keyword("12abc")),
        ("0x10", keyword("0x10")),
        ("7g2", keyword("7g2")),
        ("0_612", keyword("0_612")),
        ("3.40282347e38", keyword("3.40282347e38")),
        ("-.5", TokenKind::Real(-0.5)),
        ("+.5", TokenKind::Real(0.5)),
        ("5.", TokenKind::Real(5.0)),
        (".", TokenKind::Real(0.0)),
        ("-.", TokenKind::Real(0.0)),
        ("0.-5", TokenKind::Real(0.0)),
        ("1.5-3", TokenKind::Real(1.5)),
        ("1-.5", TokenKind::Real(1.0)),
        // Another viewer wraps the integer part past u32 (0.5); the value
        // read here is kept whole.
        ("4294967296.5-", TokenKind::Real(4_294_967_296.5)),
        ("-", TokenKind::Integer(0)),
        ("+", TokenKind::Integer(0)),
        ("--", TokenKind::Integer(0)),
        ("--5", TokenKind::Integer(-5)),
        ("+-5", TokenKind::Integer(0)),
        ("5-", TokenKind::Integer(5)),
        ("-5-", TokenKind::Integer(-5)),
        // Past i64 the value is the real the digits name; wrapping would
        // silently save a different number.
        (
            "18446744073709551617-",
            TokenKind::Real(18_446_744_073_709_551_617.0),
        ),
        ("--99999999999999999999", TokenKind::Real(-1e20)),
        ("-9223372036854775808-", TokenKind::Integer(i64::MIN)),
    ];
    for (text, expected) in cases {
        assert_eq!(lenient(text.as_bytes()), [expected], "{text}");
    }
    // Valid numbers read as the strict lexer reads them.
    assert_eq!(
        lenient(b"-42 .25 9999999999 4294967296.5"),
        [
            TokenKind::Integer(-42),
            TokenKind::Real(0.25),
            TokenKind::Integer(9_999_999_999),
            TokenKind::Real(4_294_967_296.5),
        ]
    );
    // A huge fraction stays finite.
    let long = format!("1.{}-", "9".repeat(5000));
    let [TokenKind::Real(value)] = lenient(long.as_bytes())[..] else {
        panic!("one real");
    };
    assert!((value - 2.0).abs() < 1e-9);
    // The strict lexer still refuses every one of them.
    for text in ["1e5", "--5", "5-", "-", "18446744073709551617-"] {
        assert!(
            Lexer::new(text.as_bytes(), LexLimits::default())
                .next_token()
                .is_err()
        );
    }
}

#[test]
fn lenient_hex_strings_keywords_and_stray_delimiters() {
    assert_eq!(lenient(b"<ABQD>"), [TokenKind::String(vec![0xAB, 0x0D])]);
    assert_eq!(lenient(b"<A B Q>"), [TokenKind::String(vec![0xAB, 0x00])]);
    assert_eq!(lenient(b"<AB\xA5>"), [TokenKind::String(vec![0xAB, 0x00])]);
    assert_eq!(
        lenient(b"\xA5 true1 { } ) >"),
        [
            TokenKind::Keyword(vec![0xA5]),
            TokenKind::Keyword(b"true1".to_vec()),
            TokenKind::Keyword(b"{".to_vec()),
            TokenKind::Keyword(b"}".to_vec()),
            TokenKind::Keyword(b")".to_vec()),
            TokenKind::Keyword(b">".to_vec()),
        ]
    );
    assert_eq!(lenient(b"/A#G0"), [TokenKind::Name(b"A#G0".to_vec())]);
    // Still refused: an unterminated hex string.
    let mut lexer = Lexer::new(b"<ABC", LexLimits::default()).with_lenient();
    assert!(lexer.next_token().is_err());
    assert_eq!(error(b"<ABQD>"), LexErrorKind::InvalidHexDigit(b'Q'));
}
