// SPDX-License-Identifier: MIT
//! Independent content grammar, sticky failure, and exact budget review.
use tandem_pdf_engine::content::{
    ContentErrorKind, ContentLimitKind, ContentLimits, ContentOperand, ContentParser,
};
use tandem_pdf_engine::lexer::LexLimits;

#[test]
fn spans_preserve_internal_trivia_and_unknown_operator_bytes() {
    let bytes = b"\n% lead\n /A (B) % mid\n xCustom % trailing\nq";
    let mut parser = ContentParser::new(bytes, ContentLimits::default());
    let first = parser.next_operation().unwrap().unwrap();
    assert_eq!(first.operator, b"xCustom");
    assert_eq!(
        first.operands,
        [
            ContentOperand::Name(b"A".to_vec()),
            ContentOperand::String(b"B".to_vec())
        ]
    );
    let start = bytes.iter().position(|&byte| byte == b'/').unwrap();
    let end = bytes
        .windows(7)
        .position(|part| part == b"xCustom")
        .unwrap()
        + 7;
    assert_eq!(first.span, start..end);
    assert_eq!(
        parser.next_operation().unwrap().unwrap().span,
        bytes.len() - 1..bytes.len()
    );
    assert!(parser.next_operation().unwrap().is_none());
    assert!(parser.next_operation().unwrap().is_none());
}

#[test]
fn bi_outside_operator_position_is_refused_and_sticky() {
    // An inline image is only an operation; nested or after operands it is an error.
    for bytes in [
        b"[BI \xff (".as_slice(),
        b"<< /K BI \xff (",
        b"1 BI /W 1 ID x EI",
    ] {
        let mut parser = ContentParser::new(bytes, ContentLimits::default());
        let error = parser.next_operation().unwrap_err();
        let offset = bytes.windows(2).position(|part| part == b"BI").unwrap();
        assert_eq!(error.offset, offset);
        let position = parser.position();
        for _ in 0..3 {
            assert_eq!(parser.next_operation().unwrap_err(), error);
            assert_eq!(parser.position(), position);
        }
    }
    // A malformed inline dictionary fails before any data is consumed.
    let mut parser = ContentParser::new(b"BI 5 ID \xff EI", ContentLimits::default());
    assert!(matches!(
        parser.next_operation().unwrap_err().kind,
        ContentErrorKind::UnexpectedToken(_)
    ));
    let mut parser = ContentParser::new(b"/BI (ID EI BI) Ext", ContentLimits::default());
    assert_eq!(parser.next_operation().unwrap().unwrap().operands.len(), 2);
}

#[test]
fn content_never_infers_file_references_or_accepts_nested_operators() {
    for bytes in [
        b"1 0 R".as_slice(),
        b"[1 0 R] Ext",
        b"<< /A 1 0 R >> Ext",
        b"[q] Ext",
        b"<< /A q >> Ext",
        b"endobj",
        b"ID",
        b"EI",
    ] {
        assert!(
            ContentParser::new(bytes, ContentLimits::default())
                .next_operation()
                .is_err(),
            "{bytes:?}"
        );
    }
    let mut parser = ContentParser::new(b"1 0 cm", ContentLimits::default());
    assert_eq!(
        parser.next_operation().unwrap().unwrap().operands,
        [ContentOperand::Integer(1), ContentOperand::Integer(0)]
    );
}

#[test]
fn dictionary_keys_charge_nodes_and_decoded_payload_exactly() {
    let bytes = b"<< /A 0 >> op";
    let limits = ContentLimits {
        max_nodes: 3,
        max_owned_bytes: 3,
        lex: LexLimits {
            max_tokens: 5,
            ..LexLimits::default()
        },
        ..ContentLimits::default()
    };
    assert!(ContentParser::new(bytes, limits).next_operation().is_ok());
    for (limited, kind) in [
        (
            ContentLimits {
                max_nodes: 2,
                ..limits
            },
            ContentLimitKind::Nodes,
        ),
        (
            ContentLimits {
                max_owned_bytes: 2,
                ..limits
            },
            ContentLimitKind::OwnedBytes,
        ),
    ] {
        assert_eq!(
            ContentParser::new(bytes, limited)
                .next_operation()
                .unwrap_err()
                .kind,
            ContentErrorKind::LimitExceeded(kind)
        );
    }
    assert!(
        ContentParser::new(
            bytes,
            ContentLimits {
                lex: LexLimits {
                    max_tokens: 4,
                    ..limits.lex
                },
                ..limits
            }
        )
        .next_operation()
        .is_err()
    );
    assert!(
        ContentParser::new(b"<< /A 1 /#41 2 >> Ext", ContentLimits::default())
            .next_operation()
            .is_err()
    );
}

#[test]
fn cumulative_limits_survive_releasing_prior_operations() {
    let bytes = b"(ab) q (cd) Q";
    for (limits, expected) in [
        (
            ContentLimits {
                max_owned_bytes: 5,
                ..ContentLimits::default()
            },
            ContentLimitKind::OwnedBytes,
        ),
        (
            ContentLimits {
                max_nodes: 1,
                ..ContentLimits::default()
            },
            ContentLimitKind::Nodes,
        ),
        (
            ContentLimits {
                max_operations: 1,
                ..ContentLimits::default()
            },
            ContentLimitKind::Operations,
        ),
    ] {
        let mut parser = ContentParser::new(bytes, limits);
        drop(parser.next_operation().unwrap().unwrap());
        assert_eq!(
            parser.next_operation().unwrap_err().kind,
            ContentErrorKind::LimitExceeded(expected)
        );
    }
    let mut parser = ContentParser::new(
        bytes,
        ContentLimits {
            max_owned_bytes: 6,
            max_nodes: 2,
            max_operations: 2,
            ..ContentLimits::default()
        },
    );
    assert!(parser.next_operation().unwrap().is_some());
    assert!(parser.next_operation().unwrap().is_some());
    assert!(parser.next_operation().unwrap().is_none());
}

#[test]
fn hard_depth_limit_and_zero_operand_limit_are_independent() {
    for (depth, accepted) in [(256, true), (257, false)] {
        let bytes = [
            vec![b'['; depth],
            b"0".to_vec(),
            vec![b']'; depth],
            b" Ext".to_vec(),
        ]
        .concat();
        assert_eq!(
            ContentParser::new(
                &bytes,
                ContentLimits {
                    max_depth: usize::MAX,
                    ..ContentLimits::default()
                }
            )
            .next_operation()
            .is_ok(),
            accepted
        );
    }
    let limits = ContentLimits {
        max_operands: 0,
        ..ContentLimits::default()
    };
    assert!(ContentParser::new(b"q", limits).next_operation().is_ok());
    assert_eq!(
        ContentParser::new(b"[] q", limits)
            .next_operation()
            .unwrap_err()
            .kind,
        ContentErrorKind::LimitExceeded(ContentLimitKind::Operands)
    );
}

#[test]
fn eof_pending_operands_is_sticky_after_an_earlier_valid_operation() {
    let bytes = b"q 1 % unfinished operation";
    let mut parser = ContentParser::new(bytes, ContentLimits::default());
    assert_eq!(parser.next_operation().unwrap().unwrap().operator, b"q");
    let error = parser.next_operation().unwrap_err();
    assert_eq!(error.kind, ContentErrorKind::DanglingOperands);
    assert_eq!(error.offset, bytes.len());
    assert_eq!(parser.next_operation().unwrap_err(), error);
}
