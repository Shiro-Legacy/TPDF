use tandem_pdf_engine::{
    cmap::CharacterCode,
    document::{Document, DocumentLimits},
    font_resources::*,
    object::ObjectId,
    simple_encoding::{GlyphUnicode, NamedEncoding},
};
fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}
fn pdf(parent: &str, page: &str, extra: &[String]) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        format!("<< /Type /Pages /Kids [3 0 R] /Count 1 {parent} >>"),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] {page} >>"),
    ];
    objects.extend_from_slice(extra);
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}
fn font() -> String {
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into()
}
fn stream(body: &str, meta: &str) -> String {
    format!(
        "<< /Length {} {meta} >>\nstream\n{body}\nendstream",
        body.len()
    )
}
fn cmap() -> &'static str {
    "begincmap 1 begincodespacerange <00><ff> endcodespacerange 1 beginbfchar <41><00660069> endbfchar endcmap"
}
fn open(bytes: &[u8]) -> Document<'_> {
    Document::open(bytes, DocumentLimits::default()).unwrap()
}
#[test]
fn inherited_bindings_aliases_and_byte_names() {
    let bytes = pdf(
        "/Resources << /Font << /F 4 0 R /Alias 4 0 R /A#ff 4 0 R >> >>",
        "",
        &[font()],
    );
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    let b = loader.page_bindings(id(3)).unwrap();
    assert_eq!(b.resource_owner(), Some(id(2)));
    assert_eq!(b.len(), 3);
    assert_eq!(b.target(b"F"), Some(Some(id(4))));
    assert_eq!(
        b.names().collect::<Vec<_>>(),
        [b"Alias".as_slice(), b"A\xff", b"F"]
    );
    let first = loader.load(id(3), b"F").unwrap() as *const _;
    let second = loader.load(id(3), b"Alias").unwrap() as *const _;
    assert_eq!(first, second);
    let loaded = loader.load(id(3), b"A\xff").unwrap();
    assert_eq!(loaded.identity(), FontIdentity::Indirect(id(4)));
    assert_eq!(loaded.subtype(), SimpleFontSubtype::Type1);
    assert_eq!(loaded.base_font(), Some(b"Helvetica".as_slice()));
    let SimpleFontMapping::ExplicitEncoding { base, encoding } = loaded.mapping() else {
        panic!()
    };
    assert_eq!(*base, NamedEncoding::WinAnsi);
    assert_eq!(encoding.lookup_unicode(0xa0), GlyphUnicode::Unicode(&[' ']));
    assert_eq!(loader.usage().fonts, 1);
    assert_eq!(loader.usage().calls, 4);
}
#[test]
fn whole_resources_replace_ancestors_and_null_inherits() {
    for page in [
        "/Resources << >>",
        "/Resources << /XObject << >> >>",
        "/Resources << /Font null >>",
    ] {
        let bytes = pdf("/Resources << /Font << /F 4 0 R >> >>", page, &[font()]);
        let doc = open(&bytes);
        let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
        assert!(loader.page_bindings(id(3)).unwrap().is_empty());
        assert!(matches!(
            loader.load(id(3), b"F").unwrap_err().kind,
            FontResourceErrorKind::MissingFontResource
        ));
    }
    for page in ["/Resources null", "/Resources 5 0 R"] {
        let bytes = pdf(
            "/Resources << /Font << /F 4 0 R >> >>",
            page,
            &[font(), "null".into()],
        );
        let doc = open(&bytes);
        let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
        assert_eq!(loader.page_bindings(id(3)).unwrap().len(), 1);
    }
}
#[test]
fn direct_fonts_and_lazy_unsupported_fonts() {
    let bytes = pdf(
        "",
        "/Resources << /Font << /Direct << /Subtype /TrueType /Encoding /MacRomanEncoding >> /Bad 4 0 R >> >>",
        &["<< /Subtype /Type3 >>".into()],
    );
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    assert_eq!(
        loader.page_bindings(id(3)).unwrap().target(b"Direct"),
        Some(None)
    );
    assert!(matches!(
        loader.load(id(3), b"Bad").unwrap_err().kind,
        FontResourceErrorKind::Unsupported(_)
    ));
    assert!(
        matches!(loader.load(id(3),b"Direct").unwrap().identity(),FontIdentity::Direct{resource_owner,..}if resource_owner==id(3))
    );
    assert_eq!(loader.usage().fonts, 1);
}
#[test]
fn authoritative_tounicode_no_encoding_and_shared_map() {
    let bytes = pdf(
        "",
        "/Resources << /Font << /F 4 0 R /G 6 0 R >> >>",
        &[
            "<< /Subtype /TrueType /ToUnicode 5 0 R /Encoding 42 >>".into(),
            stream(cmap(), ""),
            "<< /Subtype /Type1 /ToUnicode 5 0 R >>".into(),
        ],
    );
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    let SimpleFontMapping::ToUnicode(map) = loader.load(id(3), b"F").unwrap().mapping() else {
        panic!()
    };
    assert_eq!(
        map.lookup(CharacterCode::from_bytes(b"A").unwrap())
            .unwrap()
            .scalars(),
        &['f', 'i']
    );
    assert!(
        map.lookup(CharacterCode::from_bytes(b"B").unwrap())
            .is_none()
    );
    let pointer = std::rc::Rc::as_ptr(map);
    let first_usage = loader.usage();
    let SimpleFontMapping::ToUnicode(other) = loader.load(id(3), b"G").unwrap().mapping() else {
        panic!()
    };
    assert_eq!(pointer, std::rc::Rc::as_ptr(other));
    let usage = loader.usage();
    assert_eq!(usage.fonts, 2);
    assert_eq!(usage.maps, 1);
    assert_eq!(usage.encoded_bytes, cmap().len());
    assert_eq!(usage.decoded_work_bytes, cmap().len());
    assert_eq!(usage.cmap_entries, 1);
    assert_eq!(first_usage.cmap_input_bytes, usage.cmap_input_bytes);
}
#[test]
fn invalid_tounicode_does_not_fall_back() {
    let malformed = cmap().replace("<00660069>", "66");
    let bytes = pdf(
        "",
        "/Resources << /Font << /F 4 0 R >> >>",
        &[
            "<< /Subtype /Type1 /ToUnicode 5 0 R /Encoding /WinAnsiEncoding >>".into(),
            stream(&malformed, ""),
        ],
    );
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    let e = loader.load(id(3), b"F").unwrap_err();
    assert!(matches!(e.kind, FontResourceErrorKind::CMap(_)));
    assert_eq!(e.page, Some(id(3)));
    assert_eq!(e.font, Some(FontIdentity::Indirect(id(4))));
    assert_eq!(e.stream, Some(id(5)));
    assert_eq!(loader.usage().fonts, 0);
    assert_eq!(loader.usage().maps, 0);
    assert_eq!(loader.usage().cmap_entries, 262144);
    assert_eq!(loader.usage().cmap_payload_bytes, 16 * 1024 * 1024);
    assert!(loader.load(id(3), b"F").is_err());
    assert_eq!(loader.usage().cmap_entries, 524288);
}
#[test]
fn cmap_dependencies_width_and_vertical_refuse() {
    for (body, meta) in [
        (cmap().to_owned(), "/UseCMap /Identity-H"),
        (
            cmap()
                .replace("<00><ff>", "<0000><ffff>")
                .replace("<41>", "<0041>"),
            "",
        ),
        (cmap().replace("begincmap", "begincmap /WMode 1 def"), ""),
    ] {
        let bytes = pdf(
            "",
            "/Resources << /Font << /F 4 0 R >> >>",
            &[
                "<< /Subtype /Type1 /ToUnicode 5 0 R >>".into(),
                stream(&body, meta),
            ],
        );
        let doc = open(&bytes);
        let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
        assert!(matches!(
            loader.load(id(3), b"F").unwrap_err().kind,
            FontResourceErrorKind::Unsupported(_)
        ));
        assert_eq!(loader.usage().fonts, 0);
    }
}
#[test]
fn explicit_encoding_differences_resolve_references() {
    let bytes = pdf(
        "",
        "/Resources << /Font << /F 4 0 R >> >>",
        &[
            "<< /Subtype /Type1 /Encoding 5 0 R >>".into(),
            "<< /Type /Encoding /BaseEncoding 6 0 R /Differences 7 0 R >>".into(),
            "/WinAnsiEncoding".into(),
            "[8 0 R /A /unknown 65 /B 255 /.notdef]".into(),
            "65".into(),
        ],
    );
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    let SimpleFontMapping::ExplicitEncoding { encoding, .. } =
        loader.load(id(3), b"F").unwrap().mapping()
    else {
        panic!()
    };
    assert_eq!(encoding.lookup_unicode(65), GlyphUnicode::Unicode(&['B']));
    assert_eq!(
        encoding.lookup_unicode(66),
        GlyphUnicode::UnknownGlyphName(b"unknown")
    );
    assert_eq!(encoding.lookup_unicode(255), GlyphUnicode::UndefinedCode);
    assert_eq!(loader.usage().difference_items, 7);
    assert_eq!(loader.usage().difference_name_bytes, 16);
}
#[test]
fn missing_builtin_invalid_types_and_named_maps_are_distinct() {
    for (font, expected) in [
        ("<< /Subtype /Type1 >>", 0),
        (
            "<< /Subtype /Type1 /Encoding << /Differences [65 /A] >> >>",
            0,
        ),
        ("<< /Subtype /Type1 /Encoding /Identity-H >>", 1),
        (
            "<< /Subtype /Type1 /ToUnicode /Identity-H /Encoding /WinAnsiEncoding >>",
            1,
        ),
        ("<< /Subtype /Type1 /ToUnicode 42 >>", 2),
        (
            "<< /Subtype /Type1 /Type /Wrong /Encoding /WinAnsiEncoding >>",
            2,
        ),
    ] {
        let bytes = pdf("", "/Resources << /Font << /F 4 0 R >> >>", &[font.into()]);
        let doc = open(&bytes);
        let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
        let e = loader.load(id(3), b"F").unwrap_err();
        assert!(
            match expected {
                0 => matches!(e.kind, FontResourceErrorKind::NeedsFontBuiltinEncoding),
                1 => matches!(e.kind, FontResourceErrorKind::Unsupported(_)),
                _ => matches!(e.kind, FontResourceErrorKind::Invalid(_)),
            },
            "{font}: {e}"
        );
    }
}
#[test]
fn page_membership_resource_cycles_and_stream_dictionary_rejection() {
    let bytes = pdf("", "/Resources 4 0 R", &["5 0 R".into(), "4 0 R".into()]);
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    assert!(matches!(
        loader.page_bindings(id(4)).unwrap_err().kind,
        FontResourceErrorKind::NotPage
    ));
    assert!(matches!(
        loader.page_bindings(id(3)).unwrap_err().kind,
        FontResourceErrorKind::ReferenceCycle(_)
    ));
    let bytes = pdf(
        "",
        "/Resources << /Font << /F 4 0 R >> >>",
        &[stream("", "/Subtype /Type1 /Encoding /WinAnsiEncoding")],
    );
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    assert!(matches!(
        loader.load(id(3), b"F").unwrap_err().kind,
        FontResourceErrorKind::Invalid(_)
    ));
}
#[test]
fn exact_binding_font_name_call_and_reference_limits() {
    let bytes = pdf("", "/Resources << /Font << /F 4 0 R >> >>", &[font()]);
    let doc = open(&bytes);
    let limits = FontResourceLimits {
        max_bindings: 1,
        max_fonts: 1,
        max_name_bytes: 10,
        max_calls: 2,
        max_reference_hops: 3,
        ..FontResourceLimits::default()
    };
    let mut loader = FontResourceLoader::new(&doc, limits).unwrap();
    loader.load(id(3), b"F").unwrap();
    loader.load(id(3), b"F").unwrap();
    assert_eq!(loader.usage().reference_hops, 3);
    assert!(matches!(
        loader.load(id(3), b"F").unwrap_err().kind,
        FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::Calls)
    ));
    assert!(loader.loaded(FontIdentity::Indirect(id(4))).is_some());
    for (limits, kind) in [
        (
            FontResourceLimits {
                max_bindings: 0,
                ..limits
            },
            FontResourceLimitKind::Bindings,
        ),
        (
            FontResourceLimits {
                max_fonts: 0,
                ..limits
            },
            FontResourceLimitKind::Fonts,
        ),
        (
            FontResourceLimits {
                max_name_bytes: 9,
                ..limits
            },
            FontResourceLimitKind::NameBytes,
        ),
        (
            FontResourceLimits {
                max_reference_hops: 1,
                ..limits
            },
            FontResourceLimitKind::ReferenceHops,
        ),
        (
            FontResourceLimits {
                max_reference_depth: 0,
                ..limits
            },
            FontResourceLimitKind::ReferenceDepth,
        ),
        (
            FontResourceLimits {
                max_parent_depth: 0,
                ..limits
            },
            FontResourceLimitKind::ParentDepth,
        ),
    ] {
        let mut loader = FontResourceLoader::new(&doc, limits).unwrap();
        assert!(
            matches!(loader.load(id(3),b"F").unwrap_err().kind,FontResourceErrorKind::LimitExceeded(k)if k==kind)
        );
    }
}
#[test]
fn exact_map_and_difference_budgets() {
    let bytes = pdf(
        "",
        "/Resources << /Font << /F 4 0 R >> >>",
        &[
            "<< /Subtype /Type1 /ToUnicode 5 0 R >>".into(),
            stream(cmap(), ""),
        ],
    );
    let doc = open(&bytes);
    let limits = FontResourceLimits {
        max_maps: 1,
        max_encoded_bytes: cmap().len(),
        max_decoded_work_bytes: cmap().len(),
        max_cmap_input_bytes: cmap().len(),
        max_cmap_entries: 1,
        max_cmap_payload_bytes: 8,
        ..FontResourceLimits::default()
    };
    let mut loader = FontResourceLoader::new(&doc, limits).unwrap();
    loader.load(id(3), b"F").unwrap();
    assert_eq!(loader.usage().cmap_payload_bytes, 8);
    for limits in [
        FontResourceLimits {
            max_maps: 0,
            ..limits
        },
        FontResourceLimits {
            max_encoded_bytes: cmap().len() - 1,
            ..limits
        },
        FontResourceLimits {
            max_decoded_work_bytes: cmap().len() - 1,
            ..limits
        },
        FontResourceLimits {
            max_cmap_input_bytes: cmap().len() - 1,
            ..limits
        },
        FontResourceLimits {
            max_cmap_entries: 0,
            ..limits
        },
        FontResourceLimits {
            max_cmap_payload_bytes: 7,
            ..limits
        },
    ] {
        let mut loader = FontResourceLoader::new(&doc, limits).unwrap();
        assert!(loader.load(id(3), b"F").is_err());
        assert_eq!(loader.usage().fonts, 0);
    }
    let bytes=pdf("","/Resources << /Font << /F 4 0 R >> >>",&["<< /Subtype /Type1 /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /A 65 /B] >> >>".into()]);
    let doc = open(&bytes);
    for (limits, ok) in [
        (
            FontResourceLimits {
                max_difference_items: 4,
                max_difference_name_bytes: 2,
                ..FontResourceLimits::default()
            },
            true,
        ),
        (
            FontResourceLimits {
                max_difference_items: 3,
                ..FontResourceLimits::default()
            },
            false,
        ),
        (
            FontResourceLimits {
                max_difference_name_bytes: 1,
                ..FontResourceLimits::default()
            },
            false,
        ),
    ] {
        let mut loader = FontResourceLoader::new(&doc, limits).unwrap();
        assert_eq!(loader.load(id(3), b"F").is_ok(), ok);
    }
}
#[test]
fn decoder_failure_reserves_attempted_work() {
    let bytes = pdf(
        "",
        "/Resources << /Font << /F 4 0 R >> >>",
        &[
            "<< /Subtype /Type1 /ToUnicode 5 0 R >>".into(),
            stream("not-zlib", "/Filter /FlateDecode"),
        ],
    );
    let doc = open(&bytes);
    let limits = FontResourceLimits {
        max_decoded_work_bytes: 100,
        ..FontResourceLimits::default()
    };
    let mut loader = FontResourceLoader::new(&doc, limits).unwrap();
    assert!(loader.load(id(3), b"F").is_err());
    assert_eq!(loader.usage().decoded_work_bytes, 100);
    assert!(loader.load(id(3), b"F").is_err());
    assert_eq!(loader.usage().decoded_work_bytes, 100);
    assert_eq!(loader.usage().encoded_bytes, 16);
}
#[test]
fn stream_metadata_reference_work_is_bounded() {
    let bytes = pdf(
        "",
        "/Resources << /Font << /F 4 0 R >> >>",
        &[
            "<< /Subtype /Type1 /ToUnicode 5 0 R >>".into(),
            stream(cmap(), "/Filter 6 0 R"),
            "null".into(),
        ],
    );
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    loader.load(id(3), b"F").unwrap();
    assert_eq!(loader.usage().reference_hops, 5);
    assert_eq!(loader.usage().metadata_items, 1);
    let mut loader = FontResourceLoader::new(
        &doc,
        FontResourceLimits {
            max_metadata_items: 0,
            ..FontResourceLimits::default()
        },
    )
    .unwrap();
    assert!(matches!(
        loader.load(id(3), b"F").unwrap_err().kind,
        FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::MetadataItems)
    ));
}
