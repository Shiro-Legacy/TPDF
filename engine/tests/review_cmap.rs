// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent ToUnicode semantics, segmentation, offsets and budget review.
use tandem_pdf_engine::cmap::{
    CMapErrorKind, CMapLimitKind, CMapLimits, CharacterCode, CodeDecodeError, CodeWidth,
    parse_to_unicode,
};
use tandem_pdf_engine::lexer::LexLimits;
fn code(bytes: &[u8]) -> CharacterCode {
    CharacterCode::from_bytes(bytes).unwrap()
}
fn program(body: &str) -> String {
    format!("begincmap 1 begincodespacerange <00><ff> endcodespacerange {body} endcmap")
}

#[test]
fn prefix_segmentation_matches_exhaustive_finite_code_sets() {
    let input = b"begincmap 4 begincodespacerange <80><82> <0080><0082> <010001><010003> <02000001><02000003> endcodespacerange 2 beginbfchar <80><0041> <0080><0042> endbfchar endcmap";
    let map = parse_to_unicode(input, CMapLimits::default()).unwrap();
    assert_eq!(map.lookup(code(&[0x80])).unwrap().scalars(), ['A']);
    assert_eq!(map.lookup(code(&[0, 0x80])).unwrap().scalars(), ['B']);
    let valid: Vec<Vec<u8>> = (0..3)
        .flat_map(|n| {
            [
                vec![0x80 + n],
                vec![0, 0x80 + n],
                vec![1, 0, 1 + n],
                vec![2, 0, 0, 1 + n],
            ]
        })
        .collect();
    for value in 0u32..=65535 {
        let bytes = (value as u16).to_be_bytes();
        for input in [&bytes[..1], bytes.as_slice()] {
            let matching = valid.iter().find(|candidate| input.starts_with(candidate));
            let expected = if let Some(candidate) = matching {
                Ok((code(candidate), candidate.len()))
            } else if valid.iter().any(|candidate| candidate.starts_with(input)) {
                Err(CodeDecodeError::IncompleteCode)
            } else {
                Err(CodeDecodeError::OutsideCodeSpace)
            };
            assert_eq!(map.decode_code_prefix(input), expected, "{input:?}");
        }
    }
    for candidate in valid {
        assert_eq!(
            map.decode_code_prefix(&candidate).unwrap(),
            (code(&candidate), candidate.len())
        );
    }
    assert_eq!(
        map.decode_code_prefix(&[]),
        Err(CodeDecodeError::EmptyInput)
    );
}

#[test]
fn character_code_constructors_reject_value_truncation_and_keep_leading_zeroes() {
    for (width, max) in [
        (CodeWidth::One, 255),
        (CodeWidth::Two, 65535),
        (CodeWidth::Three, 16777215),
    ] {
        assert_eq!(CharacterCode::new(max, width).unwrap().value(), max);
        assert!(CharacterCode::new(max + 1, width).is_err());
    }
    let widest = CharacterCode::new(u32::MAX, CodeWidth::Four).unwrap();
    assert_eq!(widest.to_bytes(), [255; 4]);
    for width in 1..=4 {
        let bytes = vec![0; width];
        assert_eq!(code(&bytes).to_bytes(), bytes);
    }
}

#[test]
fn mixed_mapping_forms_charge_every_expanded_scalar_and_token() {
    let input = program(
        "1 beginbfrange <01><03><d83dde000041> endbfrange 1 beginbfchar <04><ffff0000feff> endbfchar 1 beginbfrange <05><06>[<d800dc00><00660069>] endbfrange",
    );
    let limits = CMapLimits {
        max_records: 4,
        max_mappings: 6,
        max_utf16_units: 3,
        max_owned_bytes: 48,
        max_codespaces: 1,
        max_depth: 1,
        max_input_bytes: input.len(),
        lex: LexLimits {
            max_tokens: 27,
            ..LexLimits::default()
        },
    };
    let map = parse_to_unicode(input.as_bytes(), limits).unwrap();
    assert_eq!(
        map.lookup(code(&[3])).unwrap().scalars(),
        ['\u{1f600}', 'C']
    );
    assert_eq!(
        map.lookup(code(&[4])).unwrap().scalars(),
        ['\u{ffff}', '\0', '\u{feff}']
    );
    assert_eq!(map.lookup(code(&[5])).unwrap().scalars(), ['\u{10000}']);
    assert_eq!(map.lookup(code(&[6])).unwrap().scalars(), ['f', 'i']);
    for (limited, kind) in [
        (
            CMapLimits {
                max_owned_bytes: 47,
                ..limits
            },
            CMapLimitKind::OwnedBytes,
        ),
        (
            CMapLimits {
                max_records: 3,
                ..limits
            },
            CMapLimitKind::Records,
        ),
        (
            CMapLimits {
                max_mappings: 5,
                ..limits
            },
            CMapLimitKind::Mappings,
        ),
        (
            CMapLimits {
                max_utf16_units: 2,
                ..limits
            },
            CMapLimitKind::Utf16Units,
        ),
        (
            CMapLimits {
                max_depth: 0,
                ..limits
            },
            CMapLimitKind::Depth,
        ),
    ] {
        assert_eq!(
            parse_to_unicode(input.as_bytes(), limited)
                .unwrap_err()
                .kind,
            CMapErrorKind::LimitExceeded(kind)
        );
    }
    assert!(matches!(
        parse_to_unicode(
            input.as_bytes(),
            CMapLimits {
                lex: LexLimits {
                    max_tokens: 26,
                    ..limits.lex
                },
                ..limits
            }
        )
        .unwrap_err()
        .kind,
        CMapErrorKind::Lex(_)
    ));
}

#[test]
fn odd_destinations_drop_only_their_entries() {
    for body in [
        "1 beginbfchar <01><10400> endbfchar",
        "1 beginbfrange <01><02>[<0041><10400>] endbfrange",
        "1 beginbfrange <01><02><10400> endbfrange",
    ] {
        // Odd destinations drop their entries; the valid one survives.
        let input = program(body);
        let map = parse_to_unicode(input.as_bytes(), CMapLimits::default()).unwrap();
        assert!(map.len() <= 1, "{body}");
    }
}

#[test]
fn interval_projection_catches_cross_width_ambiguity_in_both_insertion_orders() {
    for spaces in ["<01><01><00ff><0200>", "<00ff><0200><01><01>"] {
        let input = format!("begincmap 2 begincodespacerange {spaces} endcodespacerange endcmap");
        assert_eq!(
            parse_to_unicode(input.as_bytes(), CMapLimits::default())
                .unwrap_err()
                .kind,
            CMapErrorKind::AmbiguousCodeSpaces
        );
    }
    for spaces in ["<02><02><00ff><01ff>", "<00ff><01ff><02><02>"] {
        let input = format!("begincmap 2 begincodespacerange {spaces} endcodespacerange endcmap");
        assert!(parse_to_unicode(input.as_bytes(), CMapLimits::default()).is_ok());
    }
}

#[test]
fn mapping_limit_precedes_range_expansion_and_destination_parse() {
    let input=b"begincmap 1 begincodespacerange <0000><ffff> endcodespacerange 1 beginbfrange <0000><ffff>";
    let error = parse_to_unicode(
        input,
        CMapLimits {
            max_mappings: 1,
            ..CMapLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        CMapErrorKind::LimitExceeded(CMapLimitKind::Mappings)
    );
    let input = program(&format!(
        "1 beginbfchar <01><{}> endbfchar",
        "0041".repeat(64)
    ));
    assert_eq!(
        parse_to_unicode(input.as_bytes(), CMapLimits::default())
            .unwrap()
            .lookup(code(&[1]))
            .unwrap()
            .scalars()
            .len(),
        64
    );
    let input = program(&format!(
        "1 beginbfchar <01><{}> endbfchar",
        "0041".repeat(65)
    ));
    assert_eq!(
        parse_to_unicode(input.as_bytes(), CMapLimits::default())
            .unwrap_err()
            .kind,
        CMapErrorKind::LimitExceeded(CMapLimitKind::Utf16Units)
    );
}
