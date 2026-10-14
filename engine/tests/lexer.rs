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
fn rejects_numeric_overflow_but_preserves_f64_precision() {
    for input in [b"9223372036854775808".as_slice(), b"-9223372036854775809"] {
        assert_eq!(error(input), LexErrorKind::NumberOutOfRange);
    }
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
