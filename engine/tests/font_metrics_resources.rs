use tandem_pdf_engine::{
    document::{Document, DocumentLimits},
    font_metrics::*,
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
fn declared() -> String {
    "<< /Subtype /TrueType /FirstChar 65 /LastChar 67 /Widths [0 -12.5 700.25] >>".into()
}
#[test]
fn lazy_independent_cache_and_aliases() {
    let bytes = pdf(
        "",
        "/Resources << /Font << /F 4 0 R /Alias 4 0 R >> >>",
        &[declared()],
    );
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    let m = loader.load_metrics(id(3), b"F").unwrap();
    assert_eq!(m.identity(), FontIdentity::Indirect(id(4)));
    assert_eq!(m.metrics().lookup(65).advance().value(), 0.);
    assert_eq!(m.metrics().lookup(66).advance().value(), -12.5);
    assert_eq!(m.metrics().lookup(67).advance().value(), 700.25);
    let ptr = m as *const _;
    assert_eq!(
        loader.load_metrics(id(3), b"Alias").unwrap() as *const _,
        ptr
    );
    assert_eq!(loader.usage().metrics, 1);
    assert_eq!(loader.usage().width_entries, 3);
    assert_eq!(loader.usage().retained_width_entries, 3);
    assert_eq!(loader.usage().metric_attempts, 2);
    assert_eq!(loader.usage().fonts, 0);
    assert!(
        loader
            .loaded_metrics(FontIdentity::Indirect(id(4)))
            .is_some()
    );
    assert!(matches!(
        loader.load(id(3), b"F").unwrap_err().kind,
        FontResourceErrorKind::NeedsFontBuiltinEncoding
    ));
}
#[test]
fn missing_metrics_does_not_change_unicode_loading() {
    let bytes = pdf("", "/Resources << /Font << /F 4 0 R >> >>", &[font()]);
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    assert!(matches!(
        loader.load_metrics(id(3), b"F").unwrap_err().kind,
        FontResourceErrorKind::NeedsBuiltinOrFontProgramMetrics
    ));
    loader.load(id(3), b"F").unwrap();
    assert_eq!(loader.usage().fonts, 1);
    assert_eq!(loader.usage().metrics, 0);
}
#[test]
fn malformed_unicode_does_not_change_explicit_metrics() {
    let bad = cmap().replace("<00660069>", "66");
    let bytes = pdf(
        "",
        "/Resources << /Font << /F 4 0 R >> >>",
        &[
            "<< /Subtype /Type1 /ToUnicode 5 0 R /FirstChar 0 /LastChar 0 /Widths [42] >>".into(),
            stream(&bad, ""),
        ],
    );
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    assert_eq!(
        loader
            .load_metrics(id(3), b"F")
            .unwrap()
            .metrics()
            .lookup(0)
            .advance()
            .value(),
        42.
    );
    assert_eq!(loader.usage().maps, 0);
    assert!(loader.load(id(3), b"F").is_err());
    assert_eq!(
        loader
            .loaded_metrics(FontIdentity::Indirect(id(4)))
            .unwrap()
            .metrics()
            .lookup(1)
            .origin(),
        AdvanceOrigin::DefaultMissingWidth
    );
}
#[test]
fn indirect_widths_endpoints_and_missing_width() {
    let bytes=pdf("","/Resources << /Font << /F 4 0 R >> >>",&["<< /Subtype /Type1 /FirstChar 5 0 R /LastChar 6 0 R /Widths 7 0 R /FontDescriptor 8 0 R >>".into(),"254".into(),"255".into(),"[9 0 R -0.0]".into(),"<< /MissingWidth 10 0 R >>".into(),"500.5".into(),"-6.25".into()]);
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    let m = loader.load_metrics(id(3), b"F").unwrap().metrics();
    assert_eq!(m.lookup(254).advance().value(), 500.5);
    assert!(m.lookup(255).advance().value().is_sign_negative());
    assert_eq!(m.lookup(0).advance().value(), -6.25);
    assert_eq!(m.lookup(0).origin(), AdvanceOrigin::DeclaredMissingWidth);
}
#[test]
fn malformed_declarations_and_indexed_failures() {
    for fields in [
        "/FirstChar 1 /LastChar 0 /Widths []",
        "/FirstChar -1 /LastChar 0 /Widths [1 2]",
        "/FirstChar 0 /LastChar 256 /Widths []",
        "/FirstChar 0.0 /LastChar 0 /Widths [1]",
        "/FirstChar 0 /Widths [1]",
        "/FirstChar 0 /LastChar 0",
        "/FirstChar 0 /LastChar 0 /Widths 5",
        "/FirstChar 0 /LastChar 1 /Widths [1]",
        "/FirstChar 0 /LastChar 0 /Widths [1 2]",
        "/FirstChar 0 /LastChar 0 /Widths [1] /FontDescriptor []",
        "/FirstChar 0 /LastChar 0 /Widths [1] /FontDescriptor << /MissingWidth (bad) >>",
    ] {
        let bytes = pdf(
            "",
            "/Resources << /Font << /F 4 0 R >> >>",
            &[format!("<< /Subtype /Type1 {fields} >>")],
        );
        let doc = open(&bytes);
        let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
        assert!(loader.load_metrics(id(3), b"F").is_err(), "{fields}");
        assert_eq!(loader.usage().metrics, 0);
    }
    for item in ["/Bad", "9007199254740993", "99 0 R", "5 0 R"] {
        let bytes = pdf(
            "",
            "/Resources << /Font << /F 4 0 R >> >>",
            &[
                format!("<< /Subtype /Type1 /FirstChar 0 /LastChar 1 /Widths [1 {item}] >>"),
                "5 0 R".into(),
            ],
        );
        let doc = open(&bytes);
        let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
        let e = loader.load_metrics(id(3), b"F").unwrap_err();
        assert_eq!(e.width_index, Some(1), "{item}: {e}");
        assert_eq!(e.page, Some(id(3)));
        assert_eq!(e.font, Some(FontIdentity::Indirect(id(4))));
        assert_eq!(loader.usage().width_entries, 2);
        assert_eq!(loader.usage().retained_width_entries, 0);
    }
}
#[test]
fn distinct_attempt_work_and_retained_caps() {
    let bytes = pdf("", "/Resources << /Font << /F 4 0 R >> >>", &[declared()]);
    let doc = open(&bytes);
    let limits = FontResourceLimits {
        max_metric_attempts: 2,
        max_metrics: 1,
        max_width_entries: 3,
        max_retained_width_entries: 3,
        ..FontResourceLimits::default()
    };
    let mut loader = FontResourceLoader::new(&doc, limits).unwrap();
    loader.load_metrics(id(3), b"F").unwrap();
    loader.load_metrics(id(3), b"F").unwrap();
    assert!(matches!(
        loader.load_metrics(id(3), b"F").unwrap_err().kind,
        FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::MetricAttempts)
    ));
    for (limits, kind) in [
        (
            FontResourceLimits {
                max_metric_attempts: 0,
                ..limits
            },
            FontResourceLimitKind::MetricAttempts,
        ),
        (
            FontResourceLimits {
                max_metrics: 0,
                ..limits
            },
            FontResourceLimitKind::Metrics,
        ),
        (
            FontResourceLimits {
                max_width_entries: 2,
                ..limits
            },
            FontResourceLimitKind::WidthEntries,
        ),
        (
            FontResourceLimits {
                max_retained_width_entries: 2,
                ..limits
            },
            FontResourceLimitKind::RetainedWidthEntries,
        ),
    ] {
        let mut loader = FontResourceLoader::new(&doc, limits).unwrap();
        assert!(
            matches!(loader.load_metrics(id(3),b"F").unwrap_err().kind,FontResourceErrorKind::LimitExceeded(k)if k==kind)
        );
        assert_eq!(loader.usage().metrics, 0);
    }
}
#[test]
fn malformed_retries_consume_width_work() {
    let bytes = pdf(
        "",
        "/Resources << /Font << /F 4 0 R >> >>",
        &["<< /Subtype /Type1 /FirstChar 0 /LastChar 1 /Widths [1 /Bad] >>".into()],
    );
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(
        &doc,
        FontResourceLimits {
            max_width_entries: 4,
            ..FontResourceLimits::default()
        },
    )
    .unwrap();
    assert!(loader.load_metrics(id(3), b"F").is_err());
    assert!(loader.load_metrics(id(3), b"F").is_err());
    assert_eq!(loader.usage().width_entries, 4);
    assert!(matches!(
        loader.load_metrics(id(3), b"F").unwrap_err().kind,
        FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::WidthEntries)
    ));
    assert_eq!(loader.usage().retained_width_entries, 0);
}
fn cid_font(descendant_extra: &str) -> Vec<u8> {
    pdf(
        "",
        "/Resources << /Font << /F 4 0 R >> >>",
        &[
            "<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /DescendantFonts [5 0 R] >>"
                .into(),
            format!("<< /Type /Font /Subtype /CIDFontType2 {descendant_extra} >>"),
        ],
    )
}
#[test]
fn cid_widths_parse_both_group_forms_defaults_and_refuse_overlap() {
    let bytes = cid_font("/DW 500 /W [1 [100 200 200] 10 20 300 65535 [7]]");
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    let m = loader.load_metrics(id(3), b"F").unwrap().metrics();
    assert!(m.is_cid());
    let w = |cid| m.lookup_cid(cid).advance().value();
    assert_eq!(
        [w(0), w(1), w(2), w(3), w(4), w(10), w(20), w(21), w(65535)],
        [500., 100., 200., 200., 500., 300., 300., 500., 7.]
    );
    assert_eq!(
        m.lookup_cid(4).origin(),
        AdvanceOrigin::DeclaredMissingWidth
    );
    assert_eq!(m.cid_ranges().len(), 4);
    let bytes = cid_font("");
    let doc = open(&bytes);
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    let m = loader.load_metrics(id(3), b"F").unwrap().metrics();
    assert_eq!(m.lookup_cid(9).advance().value(), 1000.);
    for bad in [
        "/W [1 5 100 3 [200]]",
        "/W [1 [100] 2]",
        "/W [5 1 100]",
        "/W [70000 [1]]",
        "/W [1 (x)]",
    ] {
        let bytes = cid_font(bad);
        let doc = open(&bytes);
        let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
        assert!(loader.load_metrics(id(3), b"F").is_err(), "{bad}");
    }
    for (encoding, subtype) in [
        ("/Identity-V", "CIDFontType2"),
        ("/Identity-H", "CIDFontType0"),
    ] {
        let bytes = pdf(
            "",
            "/Resources << /Font << /F 4 0 R >> >>",
            &[
                format!("<< /Subtype /Type0 /Encoding {encoding} /DescendantFonts [5 0 R] >>"),
                format!("<< /Subtype /{subtype} >>"),
            ],
        );
        let doc = open(&bytes);
        let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
        assert!(matches!(
            loader.load_metrics(id(3), b"F").unwrap_err().kind,
            FontResourceErrorKind::Unsupported(_)
        ));
    }
}
