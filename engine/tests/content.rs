use std::collections::BTreeMap;
use tandem_pdf_engine::content::{
    ContentErrorKind, ContentLimitKind, ContentLimits, ContentOperand, ContentParser, Operation,
};
use tandem_pdf_engine::lexer::{LexErrorKind, LexLimits};

fn operations(data: &[u8]) -> Vec<Operation> {
    let mut parser = ContentParser::new(data, ContentLimits::default());
    let mut output = Vec::new();
    while let Some(operation) = parser.next_operation().unwrap() {
        output.push(operation);
    }
    output
}
fn error(data: &[u8]) -> ContentErrorKind {
    let mut parser = ContentParser::new(data, ContentLimits::default());
    loop {
        match parser.next_operation() {
            Ok(Some(_)) => {}
            Ok(None) => panic!("expected failure"),
            Err(error) => return error.kind,
        }
    }
}

#[test]
fn operations_preserve_numeric_types_operands_operator_bytes_and_spans() {
    let input = b" % leading\n q 1 0 0 1 10. +.5 cm /F1 12 Tf (hello) Tj Q % end";
    let parsed = operations(input);
    assert_eq!(parsed.len(), 5);
    assert_eq!(parsed[0].operator, b"q");
    assert!(parsed[0].operands.is_empty());
    assert_eq!(&input[parsed[0].span.clone()], b"q");
    assert_eq!(parsed[1].operator, b"cm");
    assert_eq!(
        parsed[1].operands,
        vec![
            ContentOperand::Integer(1),
            ContentOperand::Integer(0),
            ContentOperand::Integer(0),
            ContentOperand::Integer(1),
            ContentOperand::Real(10.),
            ContentOperand::Real(0.5)
        ]
    );
    assert_eq!(&input[parsed[1].span.clone()], b"1 0 0 1 10. +.5 cm");
    assert_eq!(parsed[2].operator, b"Tf");
    assert_eq!(
        parsed[2].operands,
        vec![
            ContentOperand::Name(b"F1".to_vec()),
            ContentOperand::Integer(12)
        ]
    );
    assert_eq!(
        parsed[3].operands,
        vec![ContentOperand::String(b"hello".to_vec())]
    );
    assert_eq!(parsed[4].operator, b"Q");
}

#[test]
fn nested_arrays_dictionaries_binary_names_and_strings() {
    let parsed = operations(
        b"[(a) -120 <ff00> [true false null]] TJ << /A#20B /#ff /Nested << /V [1 2] >> >> BDC",
    );
    assert_eq!(parsed.len(), 2);
    assert_eq!(
        parsed[0].operands,
        vec![ContentOperand::Array(vec![
            ContentOperand::String(b"a".to_vec()),
            ContentOperand::Integer(-120),
            ContentOperand::String(vec![255, 0]),
            ContentOperand::Array(vec![
                ContentOperand::Boolean(true),
                ContentOperand::Boolean(false),
                ContentOperand::Null
            ])
        ])]
    );
    let mut inner = BTreeMap::new();
    inner.insert(
        b"V".to_vec(),
        ContentOperand::Array(vec![ContentOperand::Integer(1), ContentOperand::Integer(2)]),
    );
    let mut dictionary = BTreeMap::new();
    dictionary.insert(b"A B".to_vec(), ContentOperand::Name(vec![255]));
    dictionary.insert(b"Nested".to_vec(), ContentOperand::Dictionary(inner));
    assert_eq!(
        parsed[1].operands,
        vec![ContentOperand::Dictionary(dictionary)]
    );
}

#[test]
fn ordinary_unknown_operators_quotes_and_compatibility_are_preserved() {
    let parsed = operations(b"BX true null custom-op false Another EX (a) ' 1 2 (b) \" T* f*");
    let names: Vec<_> = parsed.iter().map(|op| op.operator.as_slice()).collect();
    assert_eq!(
        names,
        vec![
            b"BX".as_slice(),
            b"custom-op",
            b"Another",
            b"EX",
            b"'",
            b"\"",
            b"T*",
            b"f*"
        ]
    );
    assert_eq!(
        parsed[1].operands,
        vec![ContentOperand::Boolean(true), ContentOperand::Null]
    );
    assert_eq!(parsed[2].operands, vec![ContentOperand::Boolean(false)]);
    // Syntax layer deliberately does not validate balancing, arity or types.
    assert!(operations(b"EX 1 2 q BX").len() == 3);
}

#[test]
fn file_words_and_references_are_forbidden_everywhere() {
    for word in [
        "R",
        "obj",
        "endobj",
        "stream",
        "endstream",
        "xref",
        "trailer",
        "startxref",
        "newobj",
        "ID",
        "EI",
    ] {
        for input in [
            format!("1 0 {word}"),
            format!("BX {word}"),
            format!("[{word}] op"),
            format!("<< /A {word} >> op"),
        ] {
            assert!(
                matches!(
                    error(input.as_bytes()),
                    ContentErrorKind::StructuralKeyword(_)
                ),
                "{input}"
            );
        }
    }
    assert_eq!(
        operations(b"/R (R) /BI (BI) /ID (EI) op")[0].operands.len(),
        6
    );
}

#[test]
fn inline_images_parse_as_one_operation_without_lexing_data() {
    let ops = |input: &[u8]| {
        let mut parser = ContentParser::new(input, ContentLimits::default());
        let mut out = Vec::new();
        while let Some(op) = parser.next_operation().unwrap() {
            out.push(op);
        }
        out
    };
    // Unfiltered: length from W*H*BPC*colors, so data containing "EI" is fine.
    let raw = b"q BI /W 2 /H 1 /CS /G /BPC 8 ID \n EI Q";
    let parsed = ops(raw);
    assert_eq!(parsed.len(), 3);
    assert_eq!(parsed[1].operator, b"BI");
    assert_eq!(
        parsed[1].operands[1],
        ContentOperand::String(b"\n ".to_vec())
    );
    assert_eq!(&raw[parsed[1].span.clone()], &raw[2..raw.len() - 2]);
    assert_eq!(parsed[2].operator, b"Q");
    // Filtered without /L: data ends at the first whitespace-delimited EI.
    let hex = ops(b"BI /W 1 /H 1 /CS /G /BPC 8 /F /AHx ID ff> EI 1 0 0 rg");
    assert_eq!(hex[0].operands[1], ContentOperand::String(b"ff>".to_vec()));
    assert_eq!(hex[1].operator, b"rg");
    // /L is authoritative.
    let declared = ops(b"BI /F /Fl /L 4 ID EI\xffx EI Q");
    assert_eq!(
        declared[0].operands[1],
        ContentOperand::String(b"EI\xffx".to_vec())
    );
    // Operands before BI, a missing EI and stray ID/EI are errors.
    for bad in [
        b"1 BI ID x EI".as_slice(),
        b"BI /W 1 ID",
        b"ID",
        b"EI",
        b"BI /W 1 /H 1 /CS /G /BPC 8 ID xx EI",
    ] {
        let mut parser = ContentParser::new(bad, ContentLimits::default());
        let result = (|| {
            while parser.next_operation()?.is_some() {}
            Ok::<_, tandem_pdf_engine::content::ContentError>(())
        })();
        assert!(result.is_err(), "{:?}", String::from_utf8_lossy(bad));
    }
}

#[test]
fn dangling_operands_and_malformed_containers_fail() {
    for input in [
        b"1".as_slice(),
        b"true",
        b"[1]",
        b"<< /MCID 0 >>",
        b"q 1 %end",
    ] {
        assert_eq!(error(input), ContentErrorKind::DanglingOperands);
    }
    for input in [b"[".as_slice(), b"[1", b"<<", b"<< /A", b"<< /A [1"] {
        assert_eq!(error(input), ContentErrorKind::UnexpectedEnd);
    }
    for input in [
        b"]".as_slice(),
        b">>",
        b"[>>]op",
        b"<< 1 2 >> op",
        b"<< /A >> op",
    ] {
        assert!(matches!(error(input), ContentErrorKind::UnexpectedToken(_)));
    }
    assert!(matches!(
        error(b"[q]op"),
        ContentErrorKind::UnexpectedKeyword(_)
    ));
    assert!(matches!(
        error(b"<< /A q >> op"),
        ContentErrorKind::UnexpectedKeyword(_)
    ));
    assert_eq!(
        error(b"<< /A 1 /#41 2 >> op"),
        ContentErrorKind::DuplicateKey(b"A".to_vec())
    );
}

#[test]
fn empty_comment_whitespace_and_repeated_eof() {
    for input in [b"".as_slice(), b" %EOF", b"\0\t\r\n\x0c "] {
        let mut parser = ContentParser::new(input, ContentLimits::default());
        assert_eq!(parser.next_operation().unwrap(), None);
        assert_eq!(parser.next_operation().unwrap(), None);
        assert_eq!(parser.position(), input.len());
    }
}

#[test]
fn input_operations_and_operand_limits_are_exact_and_cumulative() {
    let limits = ContentLimits {
        max_input_bytes: 1,
        max_operations: 1,
        max_operands: 0,
        ..ContentLimits::default()
    };
    assert_eq!(
        ContentParser::new(b"q", limits)
            .next_operation()
            .unwrap()
            .unwrap()
            .operator,
        b"q"
    );
    assert_eq!(
        ContentParser::new(b" q", limits)
            .next_operation()
            .unwrap_err()
            .kind,
        ContentErrorKind::LimitExceeded(ContentLimitKind::InputBytes)
    );
    let limits = ContentLimits {
        max_input_bytes: 100,
        ..limits
    };
    let mut parser = ContentParser::new(b"q Q", limits);
    parser.next_operation().unwrap();
    assert_eq!(
        parser.next_operation().unwrap_err().kind,
        ContentErrorKind::LimitExceeded(ContentLimitKind::Operations)
    );
    assert_eq!(
        ContentParser::new(b"1 q", limits)
            .next_operation()
            .unwrap_err()
            .kind,
        ContentErrorKind::LimitExceeded(ContentLimitKind::Operands)
    );
    let limits = ContentLimits {
        max_operands: 2,
        ..limits
    };
    assert_eq!(
        ContentParser::new(b"1 2 q", limits)
            .next_operation()
            .unwrap()
            .unwrap()
            .operands
            .len(),
        2
    );
    assert_eq!(
        ContentParser::new(b"1 2 3 q", limits)
            .next_operation()
            .unwrap_err()
            .kind,
        ContentErrorKind::LimitExceeded(ContentLimitKind::Operands)
    );
}

#[test]
fn node_counts_include_containers_keys_and_previous_operations() {
    let limits = ContentLimits {
        max_nodes: 4,
        ..ContentLimits::default()
    };
    let mut parser = ContentParser::new(b"<< /A [1] >> q 0 Q", limits);
    assert!(parser.next_operation().unwrap().is_some());
    assert_eq!(
        parser.next_operation().unwrap_err().kind,
        ContentErrorKind::LimitExceeded(ContentLimitKind::Nodes)
    );
    let mut parser = ContentParser::new(
        b"q Q",
        ContentLimits {
            max_nodes: 0,
            ..limits
        },
    );
    assert!(parser.next_operation().unwrap().is_some());
    assert!(parser.next_operation().unwrap().is_some());
}

#[test]
fn owned_payload_and_operator_length_limits_count_decoded_bytes() {
    let limits = ContentLimits {
        max_owned_bytes: 6,
        ..ContentLimits::default()
    };
    let mut parser = ContentParser::new(b"/A#20B <ff00> q Q", limits);
    assert!(parser.next_operation().unwrap().is_some());
    assert_eq!(
        parser.next_operation().unwrap_err().kind,
        ContentErrorKind::LimitExceeded(ContentLimitKind::OwnedBytes)
    );
    let limits = ContentLimits {
        max_operator_bytes: 2,
        ..ContentLimits::default()
    };
    assert!(ContentParser::new(b"op", limits).next_operation().is_ok());
    assert_eq!(
        ContentParser::new(b"ops", limits)
            .next_operation()
            .unwrap_err()
            .kind,
        ContentErrorKind::LimitExceeded(ContentLimitKind::OperatorBytes)
    );
    let mut parser = ContentParser::new(
        b"true false null op",
        ContentLimits {
            max_owned_bytes: 2,
            ..ContentLimits::default()
        },
    );
    assert!(parser.next_operation().unwrap().is_some());
}

#[test]
fn depth_token_count_and_token_size_limits_are_bounded() {
    let limits = ContentLimits {
        max_depth: 2,
        ..ContentLimits::default()
    };
    assert!(
        ContentParser::new(b"[[1]] q", limits)
            .next_operation()
            .is_ok()
    );
    assert_eq!(
        ContentParser::new(b"[[[1]]] q", limits)
            .next_operation()
            .unwrap_err()
            .kind,
        ContentErrorKind::LimitExceeded(ContentLimitKind::Depth)
    );
    assert!(
        ContentParser::new(
            b"1 q",
            ContentLimits {
                max_depth: 0,
                ..limits
            }
        )
        .next_operation()
        .is_ok()
    );
    let mut excessive = vec![b'['; 300];
    excessive.extend(std::iter::repeat_n(b']', 300));
    excessive.extend_from_slice(b" q");
    assert_eq!(
        ContentParser::new(
            &excessive,
            ContentLimits {
                max_depth: usize::MAX,
                ..limits
            }
        )
        .next_operation()
        .unwrap_err()
        .kind,
        ContentErrorKind::LimitExceeded(ContentLimitKind::Depth)
    );
    let mut parser = ContentParser::new(
        b"q Q",
        ContentLimits {
            lex: LexLimits {
                max_token_bytes: 2,
                max_tokens: 1,
            },
            ..limits
        },
    );
    parser.next_operation().unwrap();
    assert!(
        matches!(parser.next_operation().unwrap_err().kind,ContentErrorKind::Lex(ref e) if matches!(e.kind,LexErrorKind::TooManyTokens {limit:1}))
    );
    assert!(
        matches!(ContentParser::new(b"/AB q",ContentLimits {lex:LexLimits {max_token_bytes:2,max_tokens:10},..limits}).next_operation().unwrap_err().kind,ContentErrorKind::Lex(ref e) if matches!(e.kind,LexErrorKind::TokenTooLong {limit:2}))
    );
}

#[test]
fn failure_is_sticky_and_success_does_not_read_next_operation() {
    let mut parser = ContentParser::new(b"q (unterminated", ContentLimits::default());
    assert_eq!(parser.next_operation().unwrap().unwrap().operator, b"q");
    assert_eq!(parser.position(), 1);
    let failure = parser.next_operation().unwrap_err();
    let position = parser.position();
    assert_eq!(parser.next_operation().unwrap_err(), failure);
    assert_eq!(parser.position(), position);
}

#[test]
fn deterministic_hostile_content_is_bounded_and_cannot_panic() {
    let mut state = 0xabcdefu64;
    let limits = ContentLimits {
        max_input_bytes: 512,
        max_operations: 20,
        max_nodes: 100,
        max_owned_bytes: 128,
        max_depth: 8,
        lex: LexLimits {
            max_token_bytes: 64,
            max_tokens: 100,
        },
        ..ContentLimits::default()
    };
    for length in 0..512 {
        let mut input = Vec::new();
        for _ in 0..length {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            input.push(state as u8);
        }
        let mut parser = ContentParser::new(&input, limits);
        let mut count = 0;
        while let Ok(Some(operation)) = parser.next_operation() {
            count += 1;
            assert!(count <= 20);
            assert!(operation.span.start < operation.span.end);
            assert!(operation.span.end <= length);
        }
        assert!(parser.position() <= length);
    }
}
