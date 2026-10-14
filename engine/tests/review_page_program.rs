// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent PP1 resource, provenance, ownership and aggregate-admission tests.
use tandem_pdf_engine::{
    document::{Document, DocumentLimits},
    font_resources::FontIdentity,
    object::ObjectId,
    page_program::{
        ContentSourcePiece, PageMetricPolicy, PageProgramLimits, PageProgramSession,
        PageProgramSessionLimits,
    },
    path::InitialClip,
    text_font::{TextMetricKind, TextMetricOrigin, TextUnicodeKind},
};

fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}
fn pdf(objects: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
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
fn stream(raw: &str) -> String {
    format!("<< /Length {} >>\nstream\n{raw}\nendstream", raw.len())
}
fn declared() -> String {
    format!(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 66 /Widths [{}] >>",
        "500 ".repeat(35)
    )
}
// Catalog1, Pages2, Page3, chosen font4, unused bad font5, streams6+.
fn single(font: String, streams: &[&str], contents: &str, geometry: &str) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /Font << /F 4 0 R /Alias 4 0 R /Bad 5 0 R >> >> >>".into(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 200] {geometry} /Contents {contents} >>"),
        font,
        "<< /Type /Font /Subtype /Type0 /ToUnicode 999 0 R >>".into(),
    ];
    objects.extend(streams.iter().map(|s| stream(s)));
    pdf(&objects)
}
fn simple(raw: &str) -> Vec<u8> {
    single(declared(), &[raw], "6 0 R", "")
}
fn open(bytes: &[u8]) -> Document<'_> {
    Document::open(bytes, DocumentLimits::default()).unwrap()
}

#[test]
fn unused_unsupported_font_is_ignored_but_every_tf_selection_is_prepared() {
    for raw in ["BT /F 10 Tf (A) Tj ET", "BT /F 10 Tf ET"] {
        let bytes = simple(raw);
        let doc = open(&bytes);
        let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
        let result = session.trace_page(0, Default::default()).unwrap();
        assert_eq!(result.fonts().len(), 1);
        assert_eq!(session.font_usage().fonts, 1);
    }
    for raw in ["BT /Bad 10 Tf ET", "BT /Missing 10 Tf ET"] {
        let bytes = simple(raw);
        let doc = open(&bytes);
        let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
        assert!(session.trace_page(0, Default::default()).is_err());
        assert_eq!(session.usage().page_attempts, 1);
    }
}

#[test]
fn builtin_substitution_is_opt_in_and_never_repairs_malformed_widths() {
    let bare = "<< /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";
    let bytes = single(bare.into(), &["BT /F 10 Tf (A) Tj ET"], "6 0 R", "");
    let doc = open(&bytes);
    let mut strict = PageProgramSession::new(&doc, Default::default()).unwrap();
    assert!(strict.trace_page(0, Default::default()).is_err());
    let mut opted = PageProgramSession::new(
        &doc,
        PageProgramSessionLimits {
            metric_policy: PageMetricPolicy::AllowPinnedBase14,
            ..Default::default()
        },
    )
    .unwrap();
    let result = opted.trace_page(0, Default::default()).unwrap();
    assert_eq!(result.metric_policy(), PageMetricPolicy::AllowPinnedBase14);
    assert!(matches!(
        result.program().text().codes()[0].metric_origin(),
        TextMetricOrigin::Builtin(_)
    ));
    for broken in [
        "/Widths /Bad",
        "/FirstChar 65 /LastChar 65 /Widths []",
        "/FirstChar 65 /LastChar 65 /Widths [(500)]",
        "/FirstChar 65",
    ] {
        let font = format!(
            "<< /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding {broken} >>"
        );
        let bytes = single(font, &["BT /F 10 Tf (A) Tj ET"], "6 0 R", "");
        let doc = open(&bytes);
        let mut session = PageProgramSession::new(
            &doc,
            PageProgramSessionLimits {
                metric_policy: PageMetricPolicy::AllowPinnedBase14,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            session.trace_page(0, Default::default()).is_err(),
            "{broken}"
        );
        assert_eq!(session.font_usage().builtin_metric_attempts, 0, "{broken}");
    }
}

#[test]
fn authoritative_unicode_and_builtin_source_widths_stay_independent() {
    let map = "begincmap 1 begincodespacerange <00><ff> endcodespacerange 2 beginbfchar <20><0042> <41><0020> endbfchar endcmap";
    let font =
        "<< /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /ToUnicode 7 0 R >>";
    let bytes = single(
        font.into(),
        &["BT /F 10 Tf 3 Tw ( A) Tj ET", map],
        "6 0 R",
        "",
    );
    let doc = open(&bytes);
    let mut session = PageProgramSession::new(
        &doc,
        PageProgramSessionLimits {
            metric_policy: PageMetricPolicy::AllowPinnedBase14,
            ..Default::default()
        },
    )
    .unwrap();
    let result = session.trace_page(0, Default::default()).unwrap();
    let text = result.program().text();
    assert_eq!(text.text(), "B ");
    assert_eq!(text.fonts()[0].unicode_kind(), TextUnicodeKind::ToUnicode);
    assert!(matches!(
        text.fonts()[0].metric_kind(),
        TextMetricKind::Builtin(_)
    ));
    assert_eq!(text.codes()[0].width(), 278.);
    assert_eq!(text.codes()[1].width(), 667.);
    assert_eq!(text.codes()[0].word_displacement(), 3.);
    assert_eq!(text.codes()[1].word_displacement(), 0.);
}

#[test]
fn joined_string_and_operation_spans_retain_synthetic_padding_and_occurrences() {
    let left = "0 0 1 1 re f BT /F 10 Tf (A";
    let right = "B) Tj ET";
    let bytes = single(
        declared(),
        &[left, right, "% skipped", "invalid\n0 g"],
        "[6 0 R 7 0 R 8 0 R 9 0 R 8 0 R 9 0 R]",
        "",
    );
    let result = {
        let doc = open(&bytes);
        let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
        session.trace_page(0, Default::default()).unwrap()
    };
    drop(bytes);
    // Result owns content, font descriptors, and text after session/doc/input drop.
    let content = result.content();
    assert_eq!(content.segments.len(), 6);
    assert_eq!(content.segments[2].object, content.segments[4].object);
    assert_ne!(content.segments[2].range, content.segments[4].range);
    assert_eq!(content.segments[0].padding, Some(left.len()));
    let text = result.program().text();
    assert_eq!(text.text(), "A B");
    assert_eq!(text.source_bytes(), b"A B");
    for (i, code) in text.codes().iter().enumerate() {
        assert_eq!(code.operation_index(), 4);
        assert_eq!(code.string_span(), i..i + 1);
        assert_eq!(&content.bytes[code.operation_span()], b"(A B) Tj");
        assert!(code.operation_span().start < left.len());
        assert!(code.operation_span().end > left.len() + 1);
    }
    let span = text.codes()[0].operation_span();
    assert_eq!(
        result
            .project_range(span.clone())
            .unwrap()
            .collect::<Vec<_>>(),
        vec![
            ContentSourcePiece::Stream {
                occurrence: 0,
                object: id(6),
                decoded_range: left.len() - 2..left.len(),
                logical_range: span.start..left.len()
            },
            ContentSourcePiece::Padding {
                occurrence: 0,
                logical_offset: left.len()
            },
            ContentSourcePiece::Stream {
                occurrence: 1,
                object: id(7),
                decoded_range: 0..5,
                logical_range: left.len() + 1..span.end
            },
        ]
    );
    assert!(result.project_range(0..content.bytes.len() + 1).is_none());
    assert_eq!(result.project_range(0..0).unwrap().count(), 0);
    assert_eq!(result.page_id(), id(3));
}

#[test]
fn crop_rotation_and_userunit_are_applied_exactly_once_to_both_branches() {
    let bytes = single(
        declared(),
        &["0 0 2 3 re f BT /F 10 Tf 1 0 0 1 12 23 Tm (A) Tj ET"],
        "6 0 R",
        "/CropBox [10 20 90 180] /Rotate 90 /UserUnit 2",
    );
    let doc = open(&bytes);
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    let result = session.trace_page(0, Default::default()).unwrap();
    assert_eq!(result.geometry().bounds().bounds(), [0., 0., 320., 160.]);
    assert_eq!(
        result.program().paths().initial_ctm().coefficients(),
        [0., 2., 2., 0., -40., -20.]
    );
    assert_eq!(
        result.program().paths().initial_clip(),
        InitialClip::Rectangle(result.geometry().bounds())
    );
    assert_eq!(
        result.program().text().codes()[0]
            .transform()
            .coefficients(),
        [0., 20., 20., 0., 6., 4.]
    );
}

#[test]
fn unsupported_suffix_never_publishes_a_prefix_and_prior_output_survives() {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /Resources << /Font << /F 5 0 R >> >> >>"
            .into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Contents 6 0 R >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Contents 7 0 R >>".into(),
        declared(),
        stream("BT /F 10 Tf (A) Tj ET"),
        String::new(),
    ];
    for suffix in [
        "/X Do",
        "BT q ET",
        "BT 2 Tr (A) Tj ET",
        "0 0 m BT ET",
        "W BT ET",
        "/Span BMC EMC EMC",
        "/Span <<>> BDC",
        "/Pattern cs",
    ] {
        objects[6] = stream(&format!("BT /F 10 Tf (B) Tj ET {suffix}"));
        let bytes = pdf(&objects);
        let doc = open(&bytes);
        let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
        let first = session.trace_page(0, Default::default()).unwrap();
        assert!(
            session.trace_page(1, Default::default()).is_err(),
            "{suffix}"
        );
        assert_eq!(first.program().text().text(), "A");
        assert_eq!(session.usage().page_attempts, 2);
    }
}

#[test]
fn aliases_share_cache_and_repeated_pages_charge_both_parser_passes() {
    let raw = "BT /F 10 Tf (A) Tj /Alias 10 Tf (B) Tj ET";
    let bytes = simple(raw);
    let doc = open(&bytes);
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    let first = session.trace_page(0, Default::default()).unwrap();
    assert_eq!(first.fonts().len(), 2);
    assert_eq!(first.fonts()[0].identity(), FontIdentity::Indirect(id(4)));
    assert_eq!(first.fonts()[1].identity(), first.fonts()[0].identity());
    let font_before = session.font_usage();
    let before = session.usage();
    assert_eq!(font_before.fonts, 1);
    assert_eq!(font_before.metrics, 1);
    assert_eq!(before.parser_input_bytes, 2 * raw.len());
    assert_eq!(before.parser_operations, 12);
    let second = session.trace_page(0, Default::default()).unwrap();
    assert_eq!(second.program().text().text(), "AB");
    assert_eq!(session.usage().parser_operations, 24);
    assert_eq!(session.usage().parser_input_bytes, 4 * raw.len());
    assert_eq!(session.font_usage().fonts, font_before.fonts);
    assert_eq!(session.font_usage().metrics, font_before.metrics);
    assert!(session.font_usage().calls > font_before.calls);
    assert!(session.usage().allocated_bytes > before.allocated_bytes);
    drop(first);
    let before_drop = session.usage();
    drop(second);
    assert_eq!(session.usage(), before_drop);
}

#[test]
fn cumulative_program_and_page_limits_cannot_reset_on_repeated_calls() {
    let bytes = simple("BT /F 10 Tf (A) Tj ET");
    let doc = open(&bytes);
    let mut session = PageProgramSession::new(
        &doc,
        PageProgramSessionLimits {
            max_page_attempts: 2,
            max_program_operations: 4,
            ..Default::default()
        },
    )
    .unwrap();
    let first = session.trace_page(0, Default::default()).unwrap();
    assert_eq!(first.program().usage().operations, 4);
    assert!(session.trace_page(0, Default::default()).is_err());
    assert!(session.usage().program_operations <= 4);
    assert!(session.trace_page(0, Default::default()).is_err());
    assert!(session.usage().page_attempts <= 2);
}

#[test]
fn repeated_stream_occurrences_and_failed_parse_reservations_are_cumulative() {
    let bytes = single(declared(), &["0 g"], "[6 0 R 6 0 R]", "");
    let doc = open(&bytes);
    let mut session = PageProgramSession::new(
        &doc,
        PageProgramSessionLimits {
            max_content_references: 2,
            ..Default::default()
        },
    )
    .unwrap();
    let first = session.trace_page(0, Default::default()).unwrap();
    assert_eq!(first.content().bytes, b"0 g 0 g ");
    assert_eq!(session.usage().content_references, 2);
    assert_eq!(session.usage().encoded_bytes, 6);
    assert_eq!(session.usage().decoded_work_bytes, 6);
    assert!(session.trace_page(0, Default::default()).is_err());
    assert_eq!(session.usage().content_references, 2);

    let broken = simple("0 g 0 g [");
    let doc = open(&broken);
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    assert!(session.trace_page(0, Default::default()).is_err());
    let first = session.usage();
    assert!(first.parser_input_bytes > 0);
    assert!(first.parser_operations > 0);
    assert!(session.trace_page(0, Default::default()).is_err());
    let second = session.usage();
    assert!(second.parser_input_bytes >= first.parser_input_bytes);
    assert!(second.parser_operations >= first.parser_operations);
}

#[test]
fn identical_names_in_distinct_page_resources_keep_independent_direct_font_identity() {
    let first_font = declared();
    let second_font = declared().replace(
        "/Encoding /WinAnsiEncoding",
        "/Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /B] >>",
    );
    let bytes = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Resources << /Font << /F {first_font} >> >> /Contents 5 0 R >>"
        ),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Resources << /Font << /F {second_font} >> >> /Contents 5 0 R >>"
        ),
        stream("BT /F 10 Tf (A) Tj ET"),
    ]);
    let doc = open(&bytes);
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    let first = session.trace_page(0, Default::default()).unwrap();
    let second = session.trace_page(1, Default::default()).unwrap();
    assert_eq!(first.program().text().text(), "A");
    assert_eq!(second.program().text().text(), "B");
    assert_eq!(session.font_usage().fonts, 2);
    assert_eq!(session.font_usage().metrics, 2);
}

#[test]
fn many_empty_shows_paths_and_second_child_failure_cannot_escape_execution_caps() {
    let bytes = simple(&format!(
        "BT /F 10 Tf {} ET {}",
        "() Tj ".repeat(64),
        "0 0 m n ".repeat(64)
    ));
    let doc = open(&bytes);
    let mut session = PageProgramSession::new(
        &doc,
        PageProgramSessionLimits {
            max_program_operations: 194,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(session.trace_page(0, Default::default()).is_err());
    assert!(session.usage().program_operations <= 194);

    let bytes = simple("q Q");
    let doc = open(&bytes);
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    let mut limits = PageProgramLimits::default();
    limits.program.text.max_stack_depth = 0;
    assert!(session.trace_page(0, limits).is_err());
    assert!(session.usage().program_operations > 0);
    assert!(session.usage().allocated_bytes > 0);
}

#[test]
fn page_transparency_groups_and_default_color_overrides_are_explicit_refusals() {
    for group in ["<< /S /Transparency >>", "7 0 R"] {
        let bytes = single(
            declared(),
            &["0.5 g 0 0 2 2 re f", ""],
            "6 0 R",
            &format!("/Group {group}"),
        );
        let doc = open(&bytes);
        let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
        assert!(
            session.trace_page(0, Default::default()).is_err(),
            "group {group}"
        );
    }
    let bytes = single(declared(), &["0.5 g 0 0 2 2 re f"], "6 0 R", "/Group null");
    let doc = open(&bytes);
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    assert!(session.trace_page(0, Default::default()).is_ok());
    for name in ["DefaultGray", "DefaultRGB", "DefaultCMYK"] {
        for inherited in [true, false] {
            let resources = format!("/Resources << /ColorSpace << /{name} /DeviceRGB >> >>");
            let bytes = pdf(&[
                "<< /Type /Catalog /Pages 2 0 R >>".into(),
                format!(
                    "<< /Type /Pages /Kids [3 0 R] /Count 1 {} >>",
                    if inherited { resources.as_str() } else { "" }
                ),
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Contents 4 0 R {} >>",
                    if inherited { "" } else { resources.as_str() }
                ),
                stream("0.5 g 0 0 2 2 re f"),
            ]);
            let doc = open(&bytes);
            let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
            assert!(
                session.trace_page(0, Default::default()).is_err(),
                "{name} inherited={inherited}"
            );
        }
    }
    let bytes = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /ColorSpace << /DefaultGray /DeviceRGB >> >> >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Contents 4 0 R /Resources << /ColorSpace << /DefaultGray null /DefaultRGB null /DefaultCMYK null >> >> >>".into(),
        stream("0.5 g 0 0 2 2 re f"),
    ]);
    let doc = open(&bytes);
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    assert!(session.trace_page(0, Default::default()).is_ok());
}

#[test]
fn capped_last_join_growth_cannot_be_refunded_below_actual_full_requests() {
    let references = "6 0 R ".repeat(129);
    let bytes = single(declared(), &[""], &format!("[{references}]"), "");
    let doc = open(&bytes);
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    let mut limits = PageProgramLimits::default();
    limits.content.max_joined_bytes = 129;
    let result = session.trace_page(0, limits).unwrap();
    assert_eq!(result.content().bytes.len(), 129);
    // The joiner requests 64, 128, then its capped final target 129.
    let known_requests = 64
        + 128
        + 129
        + 129 * std::mem::size_of::<tandem_pdf_engine::page_content::ContentSegment>();
    assert!(session.usage().allocated_bytes >= known_requests);
}

#[test]
fn every_parser_partition_has_exact_repeated_page_edges() {
    let bytes = simple("BT /F 10 Tf (A) Tj ET");
    let doc = open(&bytes);
    let mut baseline = PageProgramSession::new(&doc, Default::default()).unwrap();
    baseline.trace_page(0, Default::default()).unwrap();
    let used = baseline.usage();
    for kind in 0..5 {
        for delta in [0, 1] {
            let mut limits = PageProgramSessionLimits::default();
            match kind {
                0 => limits.max_parser_input_bytes = 2 * used.parser_input_bytes - delta,
                1 => limits.max_parser_nodes = 2 * used.parser_nodes - delta,
                2 => limits.max_parser_owned_bytes = 2 * used.parser_owned_bytes - delta,
                3 => limits.max_parser_tokens = 2 * used.parser_tokens - delta,
                _ => limits.max_parser_operations = 2 * used.parser_operations - delta,
            }
            let mut session = PageProgramSession::new(&doc, limits).unwrap();
            session.trace_page(0, Default::default()).unwrap();
            assert_eq!(
                session.trace_page(0, Default::default()).is_ok(),
                delta == 0,
                "kind={kind}, delta={delta}"
            );
            let after = session.usage();
            assert!(after.parser_input_bytes <= limits.max_parser_input_bytes);
            assert!(after.parser_nodes <= limits.max_parser_nodes);
            assert!(after.parser_owned_bytes <= limits.max_parser_owned_bytes);
            assert!(after.parser_tokens <= limits.max_parser_tokens);
            assert!(after.parser_operations <= limits.max_parser_operations);
        }
    }
}

#[test]
fn failed_content_decode_keeps_pre_admitted_reservations_across_retries() {
    let bytes = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Contents 4 0 R >>".into(),
        "<< /Length 3 /Filter /ASCIIHexDecode >>\nstream\nGG>\nendstream".into(),
    ]);
    let doc = open(&bytes);
    let mut session = PageProgramSession::new(
        &doc,
        PageProgramSessionLimits {
            max_decoded_work_bytes: 14,
            max_encoded_bytes: 6,
            ..Default::default()
        },
    )
    .unwrap();
    let mut limits = PageProgramLimits::default();
    limits.content.max_decoded_work_bytes = 7;
    limits.content.max_referenced_encoded_bytes = 3;
    limits.content.max_joined_bytes = 8;
    for attempt in 1..=2 {
        assert!(session.trace_page(0, limits).is_err());
        assert_eq!(session.usage().decoded_work_bytes, 7 * attempt);
        assert_eq!(session.usage().encoded_bytes, 3 * attempt);
    }
    assert!(session.trace_page(0, limits).is_err());
    assert_eq!(session.usage().decoded_work_bytes, 14);
    assert_eq!(session.usage().encoded_bytes, 6);
}

#[test]
fn one_bad_page_geometry_does_not_block_independent_page_traces() {
    let bytes = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Contents 6 0 R >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [1000000000000000 0 1000000000000016 10] /UserUnit 1.1 /Contents 6 0 R >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Contents 6 0 R >>".into(),
        stream("0 0 1 1 re f"),
    ]);
    let doc = open(&bytes);
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    let first = session.trace_page(0, Default::default()).unwrap();
    assert!(session.trace_page(1, Default::default()).is_err());
    let last = session.trace_page(2, Default::default()).unwrap();
    assert_eq!(first.program().paths().events().len(), 1);
    assert_eq!(last.program().paths().events().len(), 1);
}

#[test]
fn malformed_authoritative_map_never_falls_back_and_failure_work_is_retained() {
    let malformed = "begincmap 1 begincodespacerange <00><ff> endcodespacerange 1 beginbfchar <41> 00 endbfchar endcmap";
    let font = declared().replace("/Subtype /Type1", "/Subtype /Type1 /ToUnicode 7 0 R");
    let bytes = single(font, &["BT /F 10 Tf (A) Tj ET", malformed], "6 0 R", "");
    let doc = open(&bytes);
    let mut limits = PageProgramSessionLimits {
        metric_policy: PageMetricPolicy::AllowPinnedBase14,
        ..Default::default()
    };
    limits.fonts.cmap.max_mappings = 2;
    limits.fonts.cmap.max_owned_bytes = 16;
    limits.fonts.max_cmap_entries = 4;
    limits.fonts.max_cmap_payload_bytes = 32;
    let mut session = PageProgramSession::new(&doc, limits).unwrap();
    for attempt in 1..=2 {
        let failure = session.trace_page(0, Default::default()).unwrap_err();
        assert_eq!(failure.operation_index, Some(1));
        assert_eq!(
            session.font_usage().encoded_bytes,
            attempt * malformed.len()
        );
        assert_eq!(
            session.font_usage().decoded_work_bytes,
            attempt * malformed.len()
        );
        assert_eq!(session.font_usage().cmap_entries, attempt * 2);
        assert_eq!(session.font_usage().cmap_payload_bytes, attempt * 16);
        assert_eq!(session.font_usage().fonts, 0);
        assert_eq!(session.font_usage().maps, 0);
        assert_eq!(session.font_usage().builtin_metric_attempts, 0);
    }
}
