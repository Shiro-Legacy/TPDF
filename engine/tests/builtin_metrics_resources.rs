use tandem_pdf_engine::{
    builtin_metrics::*,
    document::{Document, DocumentLimits},
    font_resources::*,
    object::ObjectId,
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
fn aliases_share_separate_builtin_cache() {
    let bytes = pdf(
        "",
        "/Resources << /Font << /F 4 0 R /Alias 4 0 R >> >>",
        &[font()],
    );
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    let first = loader.load_builtin_metrics(id(3), b"F").unwrap();
    assert_eq!(first.identity(), FontIdentity::Indirect(id(4)));
    assert_eq!(first.metrics().face(), BuiltinFace::Helvetica);
    let BuiltinMetricLookup::Advance(w) = first.metrics().lookup(65) else {
        panic!()
    };
    assert_eq!(w.value(), 667.);
    let ptr = first as *const _;
    assert_eq!(
        loader.load_builtin_metrics(id(3), b"Alias").unwrap() as *const _,
        ptr
    );
    assert_eq!(loader.usage().builtin_metric_attempts, 2);
    assert_eq!(loader.usage().builtin_metrics, 1);
    assert_eq!(loader.usage().builtin_slots, 256);
    assert_eq!(loader.usage().retained_builtin_slots, 256);
    assert_eq!(loader.usage().fonts, 0);
    assert_eq!(loader.usage().metrics, 0);
    assert!(matches!(
        loader.load_metrics(id(3), b"F").unwrap_err().kind,
        FontResourceErrorKind::NeedsBuiltinOrFontProgramMetrics
    ));
    loader.load(id(3), b"F").unwrap();
    assert_eq!(loader.usage().fonts, 1);
    assert!(
        loader
            .loaded_builtin_metrics(FontIdentity::Indirect(id(4)))
            .is_some()
    );
}
#[test]
fn tounicode_is_independent_and_never_selects_advance() {
    for body in [cmap().to_owned(), cmap().replace("<00660069>", "66")] {
        let bytes=pdf("","/Resources << /Font << /F 4 0 R >> >>",&["<< /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /ToUnicode 5 0 R >>".into(),stream(&body,"")]);
        let doc = open(&bytes);
        let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
        let BuiltinMetricLookup::Advance(w) = loader
            .load_builtin_metrics(id(3), b"F")
            .unwrap()
            .metrics()
            .lookup(65)
        else {
            panic!()
        };
        assert_eq!(w.value(), 667.);
        assert_eq!(loader.usage().maps, 0);
        if body == cmap() {
            let SimpleFontMapping::ToUnicode(map) = loader.load(id(3), b"F").unwrap().mapping()
            else {
                panic!()
            };
            assert_eq!(
                map.lookup(tandem_pdf_engine::cmap::CharacterCode::from_bytes(b"A").unwrap())
                    .unwrap()
                    .scalars(),
                &['f', 'i']
            );
        } else {
            assert!(loader.load(id(3), b"F").is_err());
        }
    }
}
#[test]
fn indirect_metadata_and_differences_select_exact_glyphs() {
    let bytes=pdf("","/Resources << /Font << /F 4 0 R >> >>",&["<< /Subtype /Type1 /BaseFont 5 0 R /Encoding 6 0 R /FontDescriptor 7 0 R /Widths null >>".into(),"/Times-Roman".into(),"<< /BaseEncoding /WinAnsiEncoding /Differences [65 /fi /zerosuperior /.notdef] >>".into(),"<< /Type /FontDescriptor /FontFile null /FontFile2 8 0 R >>".into(),"null".into()]);
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    let m = loader.load_builtin_metrics(id(3), b"F").unwrap().metrics();
    let BuiltinMetricLookup::Advance(w) = m.lookup(65) else {
        panic!()
    };
    assert_eq!(w.value(), 556.);
    assert_eq!(
        m.lookup(66),
        BuiltinMetricLookup::MissingBuiltinMetric(b"zerosuperior")
    );
    assert_eq!(m.lookup(67), BuiltinMetricLookup::UndefinedGlyph);
    assert_eq!(loader.usage().difference_items, 4);
}
#[test]
fn exact_selection_excludes_embedded_aliased_and_explicit_widths() {
    for font in [
        "<< /Subtype /TrueType /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        "<< /Subtype /Type1 /BaseFont /Arial /Encoding /WinAnsiEncoding >>",
        "<< /Subtype /Type1 /BaseFont /ABCDEF+Helvetica /Encoding /WinAnsiEncoding >>",
        "<< /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /Widths (malformed) >>",
        "<< /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /FirstChar 0 /LastChar 0 /Widths [42] >>",
        "<< /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /FirstChar 0 >>",
        "<< /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /LastChar 0 >>",
        "<< /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /FontDescriptor [] >>",
        "<< /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /FontDescriptor << /Type /Wrong >> >>",
    ] {
        let bytes = pdf("", "/Resources << /Font << /F 4 0 R >> >>", &[font.into()]);
        let doc = open(&bytes);
        let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
        assert!(loader.load_builtin_metrics(id(3), b"F").is_err(), "{font}");
        assert_eq!(loader.usage().builtin_metrics, 0);
    }
    for key in ["FontFile", "FontFile2", "FontFile3"] {
        let bytes = pdf(
            "",
            "/Resources << /Font << /F 4 0 R >> >>",
            &[
                format!(
                    "<< /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /FontDescriptor << /{key} 5 0 R >> >>"
                ),
                stream("", ""),
            ],
        );
        let doc = open(&bytes);
        let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
        assert!(matches!(
            loader.load_builtin_metrics(id(3), b"F").unwrap_err().kind,
            FontResourceErrorKind::Unsupported(_)
        ));
        assert_eq!(loader.usage().encoded_bytes, 0);
    }
}
#[test]
fn explicit_encoding_required_even_with_valid_tounicode() {
    let bytes = pdf(
        "",
        "/Resources << /Font << /F 4 0 R >> >>",
        &[
            "<< /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 5 0 R >>".into(),
            stream(cmap(), ""),
        ],
    );
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    loader.load(id(3), b"F").unwrap();
    assert!(matches!(
        loader.load_builtin_metrics(id(3), b"F").unwrap_err().kind,
        FontResourceErrorKind::NeedsFontBuiltinEncoding
    ));
    assert_eq!(loader.usage().builtin_slots, 256);
    assert_eq!(loader.usage().retained_builtin_slots, 0);
}
#[test]
fn exact_attempt_slot_retained_and_cache_limits() {
    let bytes = pdf("", "/Resources << /Font << /F 4 0 R >> >>", &[font()]);
    let doc = open(&bytes);
    let limits = FontResourceLimits {
        max_builtin_metric_attempts: 2,
        max_builtin_metrics: 1,
        max_builtin_slots: 256,
        max_retained_builtin_slots: 256,
        ..FontResourceLimits::default()
    };
    let mut loader = FontResourceLoader::new(&doc, limits).unwrap();
    loader.load_builtin_metrics(id(3), b"F").unwrap();
    loader.load_builtin_metrics(id(3), b"F").unwrap();
    assert!(matches!(
        loader.load_builtin_metrics(id(3), b"F").unwrap_err().kind,
        FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::BuiltinMetricAttempts)
    ));
    for (limits, kind) in [
        (
            FontResourceLimits {
                max_builtin_metric_attempts: 0,
                ..limits
            },
            FontResourceLimitKind::BuiltinMetricAttempts,
        ),
        (
            FontResourceLimits {
                max_builtin_metrics: 0,
                ..limits
            },
            FontResourceLimitKind::BuiltinMetrics,
        ),
        (
            FontResourceLimits {
                max_builtin_slots: 255,
                ..limits
            },
            FontResourceLimitKind::BuiltinSlots,
        ),
        (
            FontResourceLimits {
                max_retained_builtin_slots: 255,
                ..limits
            },
            FontResourceLimitKind::RetainedBuiltinSlots,
        ),
    ] {
        let mut loader = FontResourceLoader::new(&doc, limits).unwrap();
        assert!(
            matches!(loader.load_builtin_metrics(id(3),b"F").unwrap_err().kind,FontResourceErrorKind::LimitExceeded(k)if k==kind)
        );
        assert_eq!(loader.usage().builtin_metrics, 0);
    }
}
#[test]
fn failed_encoding_retries_charge_slots_and_item_indices() {
    let bytes=pdf("","/Resources << /Font << /F 4 0 R >> >>",&["<< /Subtype /Type1 /BaseFont /Helvetica /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 99 0 R] >> >>".into()]);
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(
        &doc,
        FontResourceLimits {
            max_builtin_slots: 512,
            ..FontResourceLimits::default()
        },
    )
    .unwrap();
    for _ in 0..2 {
        let e = loader.load_builtin_metrics(id(3), b"F").unwrap_err();
        assert_eq!(e.difference_index, Some(1));
        assert_eq!(e.font, Some(FontIdentity::Indirect(id(4))));
    }
    assert_eq!(loader.usage().builtin_slots, 512);
    assert_eq!(loader.usage().retained_builtin_slots, 0);
    assert!(matches!(
        loader.load_builtin_metrics(id(3), b"F").unwrap_err().kind,
        FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::BuiltinSlots)
    ));
}
