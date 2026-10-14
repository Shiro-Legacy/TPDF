// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent source borrowing, Unicode-byte boundaries and repeated-work review.
use tandem_pdf_engine::cmap::{CMapLimits, CharacterCode, ToUnicodeMap, parse_to_unicode};
use tandem_pdf_engine::text_decode::{
    TextDecodeErrorKind, TextDecodeLimitKind, TextDecodeLimits, decode_mapped_text,
};
fn map() -> ToUnicodeMap {
    parse_to_unicode(b"begincmap 4 begincodespacerange <80><82><0040><0042><010001><010003><02000001><02000003> endcodespacerange 4 beginbfchar <80><0000feff><0040><006600660069><010001><00410301><02000001><dbffdfffffff> endbfchar endcmap",CMapLimits::default()).unwrap()
}

#[test]
fn borrowed_subslice_offsets_are_local_and_multiscalar_spans_are_complete() {
    let backing = [9, 9, 0x80, 0, 0x40, 1, 0, 1, 2, 0, 0, 1, 9];
    let input = &backing[2..12];
    let decoded = decode_mapped_text(&map(), input, TextDecodeLimits::default()).unwrap();
    assert!(std::ptr::eq(decoded.input(), input));
    assert_eq!(
        decoded.text().as_bytes(),
        b"\0\xef\xbb\xbfffiA\xcc\x81\xf4\x8f\xbf\xbf\xef\xbf\xbf"
    );
    assert_eq!(decoded.scalar_count(), 9);
    for (entry, (source, utf8)) in
        decoded
            .codes()
            .iter()
            .zip([(0..1, 0..4), (1..3, 4..7), (3..6, 7..10), (6..10, 10..17)])
    {
        assert_eq!(entry.source_span(), source.clone());
        assert_eq!(entry.utf8_span(), utf8.clone());
        assert_eq!(
            entry.code(),
            CharacterCode::from_bytes(&input[source]).unwrap()
        );
        assert!(std::str::from_utf8(&decoded.text().as_bytes()[utf8]).is_ok());
    }
    let mut range = decoded.codes()[0].utf8_span();
    range.end = 0;
    assert!(range.is_empty());
    assert_eq!(decoded.codes()[0].utf8_span(), 0..4);
}

#[test]
fn exact_mixed_width_caps_report_the_code_start_that_exceeds_them() {
    let input = [0x80, 0, 0x40, 1, 0, 1, 2, 0, 0, 1];
    let limits = TextDecodeLimits {
        max_input_bytes: 10,
        max_codes: 4,
        max_scalars: 9,
        max_utf8_bytes: 17,
    };
    assert!(decode_mapped_text(&map(), &input, limits).is_ok());
    for (limited, kind, offset) in [
        (
            TextDecodeLimits {
                max_input_bytes: 9,
                ..limits
            },
            TextDecodeLimitKind::InputBytes,
            0,
        ),
        (
            TextDecodeLimits {
                max_codes: 3,
                ..limits
            },
            TextDecodeLimitKind::Codes,
            6,
        ),
        (
            TextDecodeLimits {
                max_scalars: 8,
                ..limits
            },
            TextDecodeLimitKind::Scalars,
            6,
        ),
        (
            TextDecodeLimits {
                max_utf8_bytes: 16,
                ..limits
            },
            TextDecodeLimitKind::Utf8Bytes,
            6,
        ),
    ] {
        let error = decode_mapped_text(&map(), &input, limited).unwrap_err();
        assert_eq!(error.offset, offset);
        assert_eq!(error.kind, TextDecodeErrorKind::LimitExceeded(kind));
    }
}

#[test]
fn unicode_scalars_and_utf8_bytes_have_different_boundaries() {
    let map=parse_to_unicode(b"begincmap 1 begincodespacerange <01><0a> endcodespacerange 1 beginbfrange <01><0a>[<0000><007f><0080><07ff><0800><d7ff><e000><ffff><d800dc00><dbffdfff>] endbfrange endcmap",CMapLimits::default()).unwrap();
    let input = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let decoded = decode_mapped_text(&map, &input, TextDecodeLimits::default()).unwrap();
    assert_eq!(decoded.text().as_bytes(),b"\x00\x7f\xc2\x80\xdf\xbf\xe0\xa0\x80\xed\x9f\xbf\xee\x80\x80\xef\xbf\xbf\xf0\x90\x80\x80\xf4\x8f\xbf\xbf");
    assert_eq!(decoded.scalar_count(), 10);
    assert_eq!(
        decoded
            .codes()
            .iter()
            .map(|entry| entry.utf8_span().len())
            .collect::<Vec<_>>(),
        [1, 1, 2, 2, 3, 3, 3, 3, 4, 4]
    );
}

#[test]
fn repeated_multi_scalar_mappings_charge_every_occurrence() {
    let input = [0x80; 3];
    let limits = TextDecodeLimits {
        max_input_bytes: 3,
        max_codes: 3,
        max_scalars: 6,
        max_utf8_bytes: 12,
    };
    let decoded = decode_mapped_text(&map(), &input, limits).unwrap();
    assert_eq!(decoded.scalar_count(), 6);
    assert_eq!(decoded.text().len(), 12);
    for limited in [
        TextDecodeLimits {
            max_scalars: 5,
            ..limits
        },
        TextDecodeLimits {
            max_utf8_bytes: 11,
            ..limits
        },
    ] {
        assert_eq!(
            decode_mapped_text(&map(), &input, limited)
                .unwrap_err()
                .offset,
            2
        );
    }
}

#[test]
fn missing_incomplete_and_invalid_are_distinct_after_a_multibyte_prefix() {
    for (tail, kind) in [
        (
            vec![0x81],
            TextDecodeErrorKind::MissingMapping(CharacterCode::from_bytes(&[0x81]).unwrap()),
        ),
        (vec![1, 0], TextDecodeErrorKind::IncompleteCode),
        (vec![1, 1], TextDecodeErrorKind::InvalidCode),
    ] {
        let mut input = vec![0, 0x40];
        input.extend(tail);
        let original = input.clone();
        let error = decode_mapped_text(&map(), &input, TextDecodeLimits::default()).unwrap_err();
        assert_eq!(error.offset, 2);
        assert_eq!(error.kind, kind);
        assert_eq!(input, original);
    }
}
