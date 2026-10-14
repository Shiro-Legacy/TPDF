use tandem_pdf_engine::cmap::{CMapLimits, CharacterCode, ToUnicodeMap, parse_to_unicode};
use tandem_pdf_engine::text_decode::*;
fn map() -> ToUnicodeMap {
    parse_to_unicode(b"begincmap 4 begincodespacerange <00><7f><8000><80ff><900000><9000ff><ff000000><ffffffff> endcodespacerange 6 beginbfchar <00><0000feff><01><00660069><02><00650301><8001><d83dde00><900001><0041d801dc00><ffffffff><005a> endbfchar endcmap",CMapLimits::default()).unwrap()
}
#[test]
fn source_and_utf8_spans_preserve_all_sequences() {
    let map = map();
    let bytes = [0, 1, 2, 0x80, 1, 0x90, 0, 1, 255, 255, 255, 255];
    let decoded = decode_mapped_text(&map, &bytes, TextDecodeLimits::default()).unwrap();
    assert_eq!(decoded.text(), "\0\u{feff}fie\u{301}\u{1f600}A\u{10400}Z");
    assert_eq!(decoded.scalar_count(), 10);
    assert!(std::ptr::eq(decoded.input().as_ptr(), bytes.as_ptr()));
    let expected = [
        (0..1, 0..4),
        (1..2, 4..6),
        (2..3, 6..9),
        (3..5, 9..13),
        (5..8, 13..18),
        (8..12, 18..19),
    ];
    for (item, (source, utf8)) in decoded.codes().iter().zip(expected) {
        assert_eq!(item.source_span(), source);
        assert_eq!(item.utf8_span(), utf8);
        assert_eq!(
            item.code(),
            CharacterCode::from_bytes(&bytes[source]).unwrap()
        );
        assert!(decoded.text().is_char_boundary(item.utf8_span().start));
        assert!(decoded.text().is_char_boundary(item.utf8_span().end));
    }
    assert_eq!(decoded.codes().len(), 6);
}
#[test]
fn result_borrows_input_but_not_map() {
    let bytes = [1];
    let decoded = {
        let map = map();
        decode_mapped_text(&map, &bytes, TextDecodeLimits::default()).unwrap()
    };
    assert_eq!(decoded.text(), "fi");
    assert_eq!(decoded.input(), &bytes);
    let mut range = decoded.codes()[0].source_span();
    range.end = 0;
    assert!(range.is_empty());
    assert_eq!(decoded.codes()[0].source_span(), 0..1);
}
#[test]
fn empty_is_valid_with_zero_caps() {
    let decoded = decode_mapped_text(
        &map(),
        &[],
        TextDecodeLimits {
            max_input_bytes: 0,
            max_codes: 0,
            max_scalars: 0,
            max_utf8_bytes: 0,
        },
    )
    .unwrap();
    assert!(decoded.text().is_empty());
    assert!(decoded.codes().is_empty());
    assert!(decoded.input().is_empty());
    assert_eq!(decoded.scalar_count(), 0);
}
#[test]
fn missing_invalid_and_incomplete_are_distinct() {
    for (bytes, offset, kind) in [
        (
            vec![1, 3],
            1,
            TextDecodeErrorKind::MissingMapping(CharacterCode::from_bytes(&[3]).unwrap()),
        ),
        (vec![1, 0x81], 1, TextDecodeErrorKind::InvalidCode),
        (vec![1, 0x80], 1, TextDecodeErrorKind::IncompleteCode),
        (vec![1, 0x90, 0], 1, TextDecodeErrorKind::IncompleteCode),
        (vec![1, 0x90, 1], 1, TextDecodeErrorKind::InvalidCode),
        (vec![1, 0xff, 0, 0], 1, TextDecodeErrorKind::IncompleteCode),
    ] {
        let e = decode_mapped_text(&map(), &bytes, TextDecodeLimits::default()).unwrap_err();
        assert_eq!(e.offset, offset);
        assert_eq!(e.kind, kind);
        assert!(e.to_string().contains("source byte 1"));
    }
}
#[test]
fn exact_all_limits_and_repeated_work() {
    let bytes = [0x80, 1, 0x80, 1];
    let limits = TextDecodeLimits {
        max_input_bytes: 4,
        max_codes: 2,
        max_scalars: 2,
        max_utf8_bytes: 8,
    };
    let decoded = decode_mapped_text(&map(), &bytes, limits).unwrap();
    assert_eq!(decoded.text(), "\u{1f600}\u{1f600}");
    for (limits, kind, offset) in [
        (
            TextDecodeLimits {
                max_input_bytes: 3,
                ..limits
            },
            TextDecodeLimitKind::InputBytes,
            0,
        ),
        (
            TextDecodeLimits {
                max_codes: 1,
                ..limits
            },
            TextDecodeLimitKind::Codes,
            2,
        ),
        (
            TextDecodeLimits {
                max_scalars: 1,
                ..limits
            },
            TextDecodeLimitKind::Scalars,
            2,
        ),
        (
            TextDecodeLimits {
                max_utf8_bytes: 7,
                ..limits
            },
            TextDecodeLimitKind::Utf8Bytes,
            2,
        ),
    ] {
        let e = decode_mapped_text(&map(), &bytes, limits).unwrap_err();
        assert_eq!(e.offset, offset);
        assert_eq!(e.kind, TextDecodeErrorKind::LimitExceeded(kind));
    }
    let e = decode_mapped_text(
        &map(),
        &[1, 1],
        TextDecodeLimits {
            max_scalars: 3,
            ..TextDecodeLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.offset, 1);
    assert_eq!(
        e.kind,
        TextDecodeErrorKind::LimitExceeded(TextDecodeLimitKind::Scalars)
    );
}
#[test]
fn zero_limits_fail_before_output_growth() {
    for (limits, kind) in [
        (
            TextDecodeLimits {
                max_codes: 0,
                ..TextDecodeLimits::default()
            },
            TextDecodeLimitKind::Codes,
        ),
        (
            TextDecodeLimits {
                max_scalars: 0,
                ..TextDecodeLimits::default()
            },
            TextDecodeLimitKind::Scalars,
        ),
        (
            TextDecodeLimits {
                max_utf8_bytes: 0,
                ..TextDecodeLimits::default()
            },
            TextDecodeLimitKind::Utf8Bytes,
        ),
    ] {
        let e = decode_mapped_text(&map(), &[1], limits).unwrap_err();
        assert_eq!(e.offset, 0);
        assert_eq!(e.kind, TextDecodeErrorKind::LimitExceeded(kind));
    }
}
#[test]
fn long_repetition_has_contiguous_spans_and_bounded_expansion() {
    let bytes = vec![1; 100_000];
    let decoded = decode_mapped_text(&map(), &bytes, TextDecodeLimits::default()).unwrap();
    assert_eq!(decoded.codes().len(), bytes.len());
    assert_eq!(decoded.scalar_count(), 200_000);
    assert_eq!(decoded.text().len(), 200_000);
    for (i, item) in decoded.codes().iter().enumerate() {
        assert_eq!(item.source_span(), i..i + 1);
        assert_eq!(item.utf8_span(), i * 2..i * 2 + 2);
    }
    let e = decode_mapped_text(
        &map(),
        &bytes,
        TextDecodeLimits {
            max_utf8_bytes: 199_999,
            ..TextDecodeLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.offset, 99_999);
}
#[test]
fn typed_width_collisions_remain_separate() {
    let map=parse_to_unicode(b"begincmap 2 begincodespacerange <01><01><0001><0001> endcodespacerange 2 beginbfchar <01><0041><0001><0042> endbfchar endcmap",CMapLimits::default()).unwrap();
    let decoded = decode_mapped_text(&map, &[1, 0, 1, 1], TextDecodeLimits::default()).unwrap();
    assert_eq!(decoded.text(), "ABA");
    assert_eq!(decoded.codes()[0].code().width().byte_len(), 1);
    assert_eq!(decoded.codes()[1].code().width().byte_len(), 2);
}
