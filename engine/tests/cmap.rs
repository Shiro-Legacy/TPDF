use tandem_pdf_engine::cmap::*;
use tandem_pdf_engine::lexer::LexLimits;
fn code(bytes: &[u8]) -> CharacterCode {
    CharacterCode::from_bytes(bytes).unwrap()
}
fn parse(body: &str) -> ToUnicodeMap {
    parse_to_unicode(body.as_bytes(), CMapLimits::default()).unwrap()
}
fn wrapped(body: &str) -> String {
    format!("begincmap 1 begincodespacerange <00> <ff> endcodespacerange {body} endcmap")
}
fn failure(body: &str) -> CMapError {
    parse_to_unicode(body.as_bytes(), CMapLimits::default()).unwrap_err()
}
fn scalars(map: &ToUnicodeMap, bytes: &[u8]) -> Vec<u32> {
    map.lookup(code(bytes))
        .unwrap()
        .scalars()
        .iter()
        .map(|c| *c as u32)
        .collect()
}
#[test]
fn character_codes_preserve_width_and_unsigned_values() {
    for width in 1..=4 {
        let bytes = vec![255; width];
        let c = code(&bytes);
        assert_eq!(c.to_bytes(), bytes);
        assert_eq!(c.width().byte_len(), width);
    }
    assert_ne!(code(&[1]), code(&[0, 1]));
    assert_eq!(code(&[255; 4]).value(), u32::MAX);
    assert!(CharacterCode::from_bytes(&[]).is_err());
    assert!(CharacterCode::from_bytes(&[0; 5]).is_err());
    assert!(CharacterCode::new(256, CodeWidth::One).is_err());
    assert!(CodeWidth::from_byte_len(0).is_err());
}
#[test]
fn bfchar_unicode_sequences_and_literal_bytes() {
    let map = parse(&wrapped(
        "4 beginbfchar <01> <0000feff> <02> <d801dc00> <03> <006600690301> (A) (\0B) endbfchar",
    ));
    assert_eq!(map.len(), 4);
    assert_eq!(scalars(&map, &[1]), [0, 0xfeff]);
    assert_eq!(scalars(&map, &[2]), [0x10400]);
    assert_eq!(scalars(&map, &[3]), [0x66, 0x69, 0x301]);
    assert_eq!(scalars(&map, b"A"), [66]);
    assert!(map.lookup(code(&[9])).is_none());
}
#[test]
fn ranges_arrays_and_final_unit_increment() {
    let map = parse(&wrapped(
        "3 beginbfrange <01> <03> <00610041> <04> <05> [<d83dde00> <00660069>] <06> <07> <d801dc00> endbfrange",
    ));
    assert_eq!(scalars(&map, &[3]), [97, 67]);
    assert_eq!(scalars(&map, &[4]), [0x1f600]);
    assert_eq!(scalars(&map, &[5]), [102, 105]);
    assert_eq!(scalars(&map, &[7]), [0x10401]);
}
#[test]
fn variable_width_codes_and_prefix_decoding() {
    let map = parse(
        "begincmap 4 begincodespacerange <01> <01> <0001> <0002> <020000> <02ffff> <ff000000> <ffffffff> endcodespacerange 4 beginbfchar <01> <0041> <0001> <0042> <020000> <0043> <ffffffff> <0044> endbfchar endcmap",
    );
    assert_eq!(scalars(&map, &[1]), [65]);
    assert_eq!(scalars(&map, &[0, 1]), [66]);
    assert_eq!(scalars(&map, &[255; 4]), [68]);
    assert_eq!(map.decode_code_prefix(&[1, 9]).unwrap(), (code(&[1]), 1));
    assert_eq!(map.decode_code_prefix(&[0, 1]).unwrap(), (code(&[0, 1]), 2));
    assert_eq!(
        map.decode_code_prefix(&[]),
        Err(CodeDecodeError::EmptyInput)
    );
    assert_eq!(
        map.decode_code_prefix(&[0]),
        Err(CodeDecodeError::IncompleteCode)
    );
    assert_eq!(
        map.decode_code_prefix(&[2, 0]),
        Err(CodeDecodeError::IncompleteCode)
    );
    assert_eq!(
        map.decode_code_prefix(&[0, 3]),
        Err(CodeDecodeError::OutsideCodeSpace)
    );
    assert_eq!(
        map.decode_code_prefix(&[3]),
        Err(CodeDecodeError::OutsideCodeSpace)
    );
}
#[test]
fn exact_wrapper_and_typed_metadata() {
    let text = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Ordering (UCS) /Supplement 0 /Registry (Adobe) >> def /CMapName /A#42 def /CMapType 2 def /WMode 1 def 1 begincodespacerange <00> <ff> endcodespacerange endcmap CMapName currentdict /CMap defineresource pop end end";
    let map = parse(text);
    assert_eq!(map.name(), Some(b"AB".as_slice()));
    assert_eq!(map.writing_mode(), 1);
    assert!(map.is_empty());
    for malformed in [
        text.replace("12 dict", "11 dict"),
        text.replace("end end", "end"),
        text.replace("/CMapType 2", "/CMapType 1"),
        text.replace("/WMode 1", "/WMode 2"),
        text.replace("/Supplement 0", "/Supplement -1"),
        text.replace("/CMapName /A#42 def", ""),
        text.replace("/Ordering (UCS)", "/Ordering (UCS) /Ordering (UCS)"),
        text.replace("/Registry (Adobe)", ""),
    ] {
        assert!(
            parse_to_unicode(malformed.as_bytes(), CMapLimits::default()).is_err(),
            "{malformed}"
        );
    }
}
#[test]
fn exact_section_counts_and_keywords() {
    assert!(parse(&wrapped("0 beginbfchar endbfchar")).is_empty());
    let hundred = (0..100)
        .map(|i| format!("<{i:02x}> <0041>"))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        parse(&wrapped(&format!("100 beginbfchar {hundred} endbfchar"))).len(),
        100
    );
    for section in [
        "-1 beginbfchar endbfchar",
        "101 beginbfchar endbfchar",
        "1 beginbfchar endbfchar",
        "0 beginbfchar <01> <0041> endbfchar",
        "0 beginbfcharGarbage endbfchar",
        "0 beginbfchar endbfcharGarbage",
        "0 begincidchar endcidchar",
        "/Identity-H usecmap",
        "1.0 beginbfchar endbfchar",
    ] {
        assert!(
            parse_to_unicode(wrapped(section).as_bytes(), CMapLimits::default()).is_err(),
            "{section}"
        );
    }
}
#[test]
fn invalid_unicode_destinations_are_dropped_not_repaired() {
    for destination in [
        "<>",
        "<00>",
        "<10400>",
        "<d800>",
        "<dc00>",
        "<d8000041>",
        "<0041dc00>",
        "<d800d800>",
    ] {
        let input = wrapped(&format!(
            "2 beginbfchar <01> {destination} <02> <0042> endbfchar"
        ));
        let map = parse_to_unicode(input.as_bytes(), CMapLimits::default()).unwrap();
        assert_eq!(map.len(), 1, "{destination}");
        let code = |v| CharacterCode::new(v, CodeWidth::One).unwrap();
        assert!(map.lookup(code(1)).is_none(), "{destination}");
        assert_eq!(map.lookup(code(2)).unwrap().scalars(), ['B']);
    }
}
#[test]
fn strict_range_errors() {
    for section in [
        "1 beginbfrange <02> <01> <0041> endbfrange",
        "1 beginbfrange <01> <0002> <0041> endbfrange",
        "1 beginbfrange <01> <02> [<0041>] endbfrange",
        "1 beginbfrange <01> <01> [<0041><0042>] endbfrange",
        "1 beginbfrange <01> <02> <ffff> endbfrange",
        "1 beginbfrange <01> <01> 4 endbfrange",
    ] {
        assert!(
            parse_to_unicode(wrapped(section).as_bytes(), CMapLimits::default()).is_err(),
            "{section}"
        );
    }
    // Incrementing into a surrogate invalidates only that entry.
    for section in [
        "1 beginbfrange <01> <02> <d7ff> endbfrange",
        "1 beginbfrange <01> <02> <d800dfff> endbfrange",
    ] {
        let map = parse_to_unicode(wrapped(section).as_bytes(), CMapLimits::default()).unwrap();
        assert_eq!(map.len(), 1, "{section}");
    }
    assert!(matches!(failure("begincmap 1 begincodespacerange <00000000> <ffffffff> endcodespacerange 1 beginbfrange <00010000> <ffffffff> <0041> endbfrange endcmap").kind,CMapErrorKind::Unsupported(_)));
}
#[test]
fn overlap_and_coverage_are_strict() {
    for spaces in [
        "<00><ff><ff><ff>",
        "<00><7f><20><30>",
        "<00><ff><0000><ffff>",
        "<0100><02ff><01><01>",
    ] {
        assert!(
            parse_to_unicode(
                format!("begincmap 2 begincodespacerange {spaces} endcodespacerange endcmap")
                    .as_bytes(),
                CMapLimits::default()
            )
            .is_err()
        );
    }
    for records in [
        "2 beginbfchar <01><0041><01><0042> endbfchar",
        "1 beginbfchar <01><0041> endbfchar 1 beginbfrange <01><03><0041> endbfrange",
        "2 beginbfrange <01><03><0041><03><05><0041> endbfrange",
    ] {
        assert!(matches!(
            failure(&wrapped(records)).kind,
            CMapErrorKind::DuplicateMapping(_)
        ));
    }
    assert!(
        failure("begincmap endcmap")
            .to_string()
            .contains("requires a codespace")
    );
    assert!(failure("begincmap 1 begincodespacerange <01><01> endcodespacerange 1 beginbfchar <02><0041> endbfchar endcmap").to_string().contains("outside declared"));
    assert_eq!(parse("begincmap 1 beginbfchar <01><0041> endbfchar 1 begincodespacerange <01><01> endcodespacerange endcmap").len(),1);
}
#[test]
fn exact_resource_boundaries() {
    let text = wrapped("1 beginbfchar <01><d801dc000041> endbfchar");
    let limits = CMapLimits {
        max_input_bytes: text.len(),
        max_codespaces: 1,
        max_records: 2,
        max_mappings: 1,
        max_utf16_units: 3,
        max_owned_bytes: 8,
        max_depth: 0,
        ..CMapLimits::default()
    };
    assert!(parse_to_unicode(text.as_bytes(), limits).is_ok());
    for (limits, kind) in [
        (
            CMapLimits {
                max_input_bytes: text.len() - 1,
                ..limits
            },
            CMapLimitKind::InputBytes,
        ),
        (
            CMapLimits {
                max_codespaces: 0,
                ..limits
            },
            CMapLimitKind::CodeSpaces,
        ),
        (
            CMapLimits {
                max_records: 1,
                ..limits
            },
            CMapLimitKind::Records,
        ),
        (
            CMapLimits {
                max_mappings: 0,
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
                max_owned_bytes: 7,
                ..limits
            },
            CMapLimitKind::OwnedBytes,
        ),
    ] {
        assert_eq!(
            parse_to_unicode(text.as_bytes(), limits).unwrap_err().kind,
            CMapErrorKind::LimitExceeded(kind)
        );
    }
    assert!(matches!(
        parse_to_unicode(
            text.as_bytes(),
            CMapLimits {
                lex: LexLimits {
                    max_token_bytes: 1,
                    max_tokens: 100
                },
                ..limits
            }
        )
        .unwrap_err()
        .kind,
        CMapErrorKind::Lex(_)
    ));
    assert!(matches!(
        parse_to_unicode(
            text.as_bytes(),
            CMapLimits {
                lex: LexLimits {
                    max_token_bytes: 100,
                    max_tokens: 0
                },
                ..limits
            }
        )
        .unwrap_err()
        .kind,
        CMapErrorKind::Lex(_)
    ));
    let array = wrapped("1 beginbfrange <01><01>[<0041>] endbfrange");
    assert_eq!(
        parse_to_unicode(
            array.as_bytes(),
            CMapLimits {
                max_depth: 0,
                ..CMapLimits::default()
            }
        )
        .unwrap_err()
        .kind,
        CMapErrorKind::LimitExceeded(CMapLimitKind::Depth)
    );
}
#[test]
fn expansion_is_charged_before_iteration() {
    let text = "begincmap 1 begincodespacerange <0000><ffff> endcodespacerange 1 beginbfrange <0000><ffff><0000> endbfrange endcmap";
    assert_eq!(
        parse_to_unicode(
            text.as_bytes(),
            CMapLimits {
                max_mappings: 65535,
                ..CMapLimits::default()
            }
        )
        .unwrap_err()
        .kind,
        CMapErrorKind::LimitExceeded(CMapLimitKind::Mappings)
    );
    let text = wrapped("1 beginbfrange <00><ff><0041> endbfrange");
    assert_eq!(
        parse_to_unicode(
            text.as_bytes(),
            CMapLimits {
                max_owned_bytes: 1023,
                ..CMapLimits::default()
            }
        )
        .unwrap_err()
        .kind,
        CMapErrorKind::LimitExceeded(CMapLimitKind::OwnedBytes)
    );
    assert_eq!(
        parse_to_unicode(
            text.as_bytes(),
            CMapLimits {
                max_owned_bytes: 1024,
                ..CMapLimits::default()
            }
        )
        .unwrap()
        .len(),
        256
    );
}
#[test]
fn malformed_source_and_unknown_constructs() {
    for input in [
        "",
        "begincmap",
        "begincmap endcmap trailing",
        "begincmap 1 begincodespacerange <><ff> endcodespacerange endcmap",
        "begincmap 1 begincodespacerange <ff><00> endcodespacerange endcmap",
        "begincmap /CMapName /A def /CMapName /B def endcmap",
        "begincmap /Foo 1 def endcmap",
        "begincmap 1 begincodespacerange <00><ff> endcodespacerange save restore endcmap",
    ] {
        assert!(
            parse_to_unicode(input.as_bytes(), CMapLimits::default()).is_err(),
            "{input}"
        );
    }
}
#[test]
fn deterministic_mutations_and_all_prefixes_terminate() {
    let text = wrapped("2 beginbfchar <01><d801dc00><02><00660069> endbfchar").into_bytes();
    for end in 0..text.len() {
        assert!(parse_to_unicode(&text[..end], CMapLimits::default()).is_err());
    }
    let mut seed = 0x5eed_u64;
    for _ in 0..1000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let mut bytes = text.clone();
        let i = (seed as usize) % bytes.len();
        bytes[i] = (seed >> 32) as u8;
        let _ = parse_to_unicode(&bytes, CMapLimits::default());
    }
}
