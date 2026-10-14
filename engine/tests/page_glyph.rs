// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::page_glyph::{
    PageGlyphCommand, PageGlyphLimits, PageGlyphSession, PageGlyphSessionLimits,
};
const PDF: &[u8] = include_bytes!("../../fixtures/p3/simple-subset.pdf");
#[test]
fn actual_embedded_page_retains_source_and_shared_outlines() {
    let document = Document::open(PDF, DocumentLimits::default()).unwrap();
    let mut session = PageGlyphSession::new(&document, PageGlyphSessionLimits::default()).unwrap();
    let first = session.trace_page(0, PageGlyphLimits::default()).unwrap();
    assert!(!first.glyphs().is_empty());
    assert_eq!(
        first.glyphs().len(),
        first.page_program().program().text().codes().len()
    );
    let work = session.usage();
    let load = session.load_usage();
    let second = session.trace_page(0, PageGlyphLimits::default()).unwrap();
    assert_eq!(session.load_usage(), load);
    assert_eq!(session.usage().font, work.font);
    assert_eq!(session.usage().cmap, work.cmap);
    assert_eq!(session.usage().outline, work.outline);
    assert!(session.usage().lookup_work > work.lookup_work);
    for (a, b) in first.glyphs().iter().zip(second.glyphs()) {
        assert!(std::ptr::eq(
            a.embedded_true_type().unwrap().outline(),
            b.embedded_true_type().unwrap().outline()
        ));
        assert_eq!(
            a.embedded_true_type().unwrap().choice().unwrap().glyph_id(),
            Some(a.embedded_true_type().unwrap().outline().id())
        );
    }
}
#[test]
fn owned_page_can_move_to_another_thread_after_source_session_drops() {
    let result = {
        let document = Document::open(PDF, DocumentLimits::default()).unwrap();
        let mut session =
            PageGlyphSession::new(&document, PageGlyphSessionLimits::default()).unwrap();
        session.trace_page(0, PageGlyphLimits::default()).unwrap()
    };
    std::thread::spawn(move || {
        assert!(!result.page_program().content().bytes.is_empty());
        assert!(!result.commands().is_empty());
        for glyph in result.glyphs() {
            assert_eq!(
                result.commands()[glyph.command_range()].len(),
                glyph
                    .embedded_true_type()
                    .unwrap()
                    .outline()
                    .commands()
                    .len()
            );
        }
    })
    .join()
    .unwrap();
}
#[test]
fn second_page_cannot_reset_transform_budget() {
    let document = Document::open(PDF, DocumentLimits::default()).unwrap();
    let mut probe = PageGlyphSession::new(&document, PageGlyphSessionLimits::default()).unwrap();
    probe.trace_page(0, PageGlyphLimits::default()).unwrap();
    let exact = probe.usage().transform_points;
    let mut session = PageGlyphSession::new(
        &document,
        PageGlyphSessionLimits {
            max_transform_points: exact,
            ..PageGlyphSessionLimits::default()
        },
    )
    .unwrap();
    let first = session.trace_page(0, PageGlyphLimits::default()).unwrap();
    assert!(session.trace_page(0, PageGlyphLimits::default()).is_err());
    assert_eq!(session.usage().transform_points, exact);
    assert!(!first.commands().is_empty());
}
#[test]
fn open_failures_retain_font_work_reservations() {
    let document = Document::open(PDF, DocumentLimits::default()).unwrap();
    let mut probe = PageGlyphSession::new(&document, PageGlyphSessionLimits::default()).unwrap();
    probe.trace_page(0, PageGlyphLimits::default()).unwrap();
    let mut limits = PageGlyphSessionLimits::default();
    limits.font.max_work = probe.usage().font.work - 1;
    let mut session = PageGlyphSession::new(&document, limits).unwrap();
    assert!(session.trace_page(0, PageGlyphLimits::default()).is_err());
    assert_eq!(session.usage().font.work, limits.font.max_work);
    assert!(session.trace_page(0, PageGlyphLimits::default()).is_err());
    assert_eq!(session.usage().font.work, limits.font.max_work);
}
#[test]
fn native_origin_shift_and_transformed_commands_are_finite() {
    let document = Document::open(PDF, DocumentLimits::default()).unwrap();
    let mut session = PageGlyphSession::new(&document, PageGlyphSessionLimits::default()).unwrap();
    let result = session.trace_page(0, PageGlyphLimits::default()).unwrap();
    for glyph in result.glyphs() {
        assert_eq!(
            glyph.embedded_true_type().unwrap().origin_shift(),
            i32::from(glyph.embedded_true_type().unwrap().metric_lsb())
                - glyph
                    .embedded_true_type()
                    .unwrap()
                    .outline()
                    .header_bounds()
                    .map_or(0, |b| b.x_min())
        );
        assert!(!glyph.embedded_true_type().unwrap().hhea_range().is_empty());
        assert!(!glyph.embedded_true_type().unwrap().hmtx_range().is_empty());
    }
    for command in result.commands() {
        match command {
            PageGlyphCommand::MoveTo(p) | PageGlyphCommand::LineTo(p) => {
                assert!(p.coordinates().iter().all(|v| v.is_finite()))
            }
            PageGlyphCommand::QuadTo { control, end } => assert!(
                control
                    .coordinates()
                    .iter()
                    .chain(end.coordinates().iter())
                    .all(|v| v.is_finite())
            ),
            PageGlyphCommand::CubicTo { .. } => {
                panic!("TrueType fixture cannot emit cubic commands")
            }
            PageGlyphCommand::Close => {}
        }
    }
}

fn unsupported_suffix_pdf(text: &[u8]) -> Vec<u8> {
    use tandem_pdf_engine::{
        embedded_glyph::{GlyphFontLoader, GlyphLoadLimits},
        font_resources::FontIdentity,
        page_program::{PageProgramLimits, PageProgramSession, PageProgramSessionLimits},
    };
    let original = Document::open(PDF, DocumentLimits::default()).unwrap();
    let page = PageProgramSession::new(&original, PageProgramSessionLimits::default())
        .unwrap()
        .trace_page(0, PageProgramLimits::default())
        .unwrap();
    let FontIdentity::Indirect(id) = page.fonts()[0].identity() else {
        panic!("fixture font identity")
    };
    let font = GlyphFontLoader::new(&original, GlyphLoadLimits::default())
        .load(id)
        .unwrap();
    let stream = |bytes: &[u8]| {
        let mut out = format!("<< /Length {} >>\nstream\n", bytes.len()).into_bytes();
        out.extend_from_slice(bytes);
        out.extend_from_slice(b"\nendstream");
        out
    };
    let objects=[b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /Font << /F 4 0 R >> >> /Contents 7 0 R >>".to_vec(),b"<< /Type /Font /Subtype /TrueType /Encoding /WinAnsiEncoding /FirstChar 65 /LastChar 66 /Widths [500 500] /FontDescriptor 5 0 R >>".to_vec(),b"<< /Type /FontDescriptor /Flags 32 /FontFile2 6 0 R >>".to_vec(),stream(font.bytes()),stream(text)];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(object);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in &offsets[1..] {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    out
}
#[test]
fn cli_unsupported_glyph_suffix_publishes_no_supported_prefix() {
    let valid = unsupported_suffix_pdf(b"BT /F 10 Tf (A) Tj ET");
    let doc = Document::open(&valid, DocumentLimits::default()).unwrap();
    let result = PageGlyphSession::new(&doc, PageGlyphSessionLimits::default())
        .unwrap()
        .trace_page(0, PageGlyphLimits::default())
        .unwrap();
    assert_eq!(result.glyphs().len(), 1);
    let invalid = unsupported_suffix_pdf(b"BT /F 10 Tf (AB) Tj ET");
    let path =
        std::env::temp_dir().join(format!("tpdf-page-glyph-atomic-{}.pdf", std::process::id()));
    std::fs::write(&path, invalid).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_pdf-inspect"))
        .arg("--page-glyph=1")
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("UnsupportedCode(66)"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn builtin_pdf(font: &str, content: &str) -> Vec<u8> {
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /Font << /F 4 0 R /Alias 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        format!("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica {font} >>").into_bytes(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).into_bytes(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        out.extend_from_slice(object);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in &offsets[1..] {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    out
}
fn builtin_limits() -> PageGlyphSessionLimits {
    PageGlyphSessionLimits {
        outline_policy: tandem_pdf_engine::page_glyph::PageOutlinePolicy::AllowPinnedBase14,
        ..PageGlyphSessionLimits::default()
    }
}
#[test]
fn builtin_outline_policy_is_independent_and_pdf_width_is_not_native_width() {
    use tandem_pdf_engine::page_program::PageMetricPolicy;
    let bytes = builtin_pdf(
        "/Encoding /WinAnsiEncoding /FirstChar 65 /LastChar 65 /Widths [123]",
        "BT /F 10 Tf (AA) Tj ET",
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    assert!(
        PageGlyphSession::new(&doc, PageGlyphSessionLimits::default())
            .unwrap()
            .trace_page(0, PageGlyphLimits::default())
            .is_err()
    );
    let page = PageGlyphSession::new(&doc, builtin_limits())
        .unwrap()
        .trace_page(0, PageGlyphLimits::default())
        .unwrap();
    assert_eq!(
        page.page_program().metric_policy(),
        PageMetricPolicy::DeclaredOnly
    );
    assert_eq!(page.glyphs()[0].pinned_base14().unwrap().name(), b"A");
    assert!(page.glyphs()[0].embedded_true_type().is_none());
    assert_eq!(
        page.page_program().program().text().codes()[0].width(),
        123.
    );
    assert_eq!(
        page.page_program().program().text().codes()[1].origin().x(),
        1.23
    );
    let no_widths = builtin_pdf("/Encoding /WinAnsiEncoding", "BT /F 10 Tf (A) Tj ET");
    let doc = Document::open(&no_widths, DocumentLimits::default()).unwrap();
    assert!(
        PageGlyphSession::new(&doc, builtin_limits())
            .unwrap()
            .trace_page(0, PageGlyphLimits::default())
            .is_err()
    );
    let mut metrics_only = PageGlyphSessionLimits::default();
    metrics_only.page.metric_policy = PageMetricPolicy::AllowPinnedBase14;
    assert!(
        PageGlyphSession::new(&doc, metrics_only)
            .unwrap()
            .trace_page(0, PageGlyphLimits::default())
            .is_err()
    );
    metrics_only.outline_policy =
        tandem_pdf_engine::page_glyph::PageOutlinePolicy::AllowPinnedBase14;
    assert!(
        PageGlyphSession::new(&doc, metrics_only)
            .unwrap()
            .trace_page(0, PageGlyphLimits::default())
            .is_ok()
    );
}
#[test]
fn builtin_aliases_repeat_without_reloading_and_outputs_own_provenance() {
    let bytes = builtin_pdf(
        "/Encoding /WinAnsiEncoding /FirstChar 65 /LastChar 65 /Widths [500]",
        "BT /F 10 Tf (A) Tj /Alias 10 Tf (A) Tj ET",
    );
    let output = {
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let mut session = PageGlyphSession::new(&doc, builtin_limits()).unwrap();
        let first = session.trace_page(0, PageGlyphLimits::default()).unwrap();
        let before = session.usage();
        let load = session.load_usage();
        assert_eq!(before.cached_fonts, 1);
        assert_eq!(before.cached_glyphs, 1);
        let second = session.trace_page(0, PageGlyphLimits::default()).unwrap();
        assert_eq!(session.load_usage(), load);
        assert_eq!(session.usage().cached_glyphs, 1);
        assert_eq!(session.usage().font.work, 0);
        assert_eq!(session.usage().outline.decode_attempts, 0);
        assert!(std::ptr::eq(
            first.glyphs()[0].pinned_base14().unwrap(),
            second.glyphs()[1].pinned_base14().unwrap()
        ));
        second
    };
    std::thread::spawn(move || {
        assert_eq!(
            output.glyphs()[0]
                .pinned_base14()
                .unwrap()
                .asset_sha256()
                .len(),
            64
        );
        assert!(!output.commands().is_empty());
    })
    .join()
    .unwrap();
}
#[test]
fn builtin_missing_name_retries_consume_work_and_never_publish_prefix() {
    let bytes = builtin_pdf(
        "/Encoding << /BaseEncoding /WinAnsiEncoding /Differences [66 /Asmall] >> /FirstChar 65 /LastChar 66 /Widths [500 500]",
        "BT /F 10 Tf (AB) Tj ET",
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut session = PageGlyphSession::new(&doc, builtin_limits()).unwrap();
    let err = session
        .trace_page(0, PageGlyphLimits::default())
        .unwrap_err();
    assert_eq!(err.code_index, Some(1));
    let work = session.usage().lookup_work;
    assert_eq!(session.usage().transformed_commands, 0);
    assert!(session.trace_page(0, PageGlyphLimits::default()).is_err());
    assert!(session.usage().lookup_work > work);
    assert_eq!(session.usage().transformed_commands, 0);
}
#[test]
fn builtin_empty_glyph_succeeds_but_embedded_programs_never_fall_back() {
    let bytes = builtin_pdf(
        "/Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 32 /Widths [250]",
        "BT /F 10 Tf ( ) Tj ET",
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let page = PageGlyphSession::new(&doc, builtin_limits())
        .unwrap()
        .trace_page(0, PageGlyphLimits::default())
        .unwrap();
    assert_eq!(page.glyphs().len(), 1);
    assert!(page.commands().is_empty());
    assert_eq!(page.glyphs()[0].pinned_base14().unwrap().name(), b"space");
    for entry in ["/FontFile 1", "/FontFile2 99 0 R", "/FontFile3 << >>"] {
        let bytes = builtin_pdf(
            &format!(
                "/Encoding /WinAnsiEncoding /FirstChar 65 /LastChar 65 /Widths [500] /FontDescriptor << /Type /FontDescriptor {entry} >>"
            ),
            "BT /F 10 Tf (A) Tj ET",
        );
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        assert!(
            PageGlyphSession::new(&doc, builtin_limits())
                .unwrap()
                .trace_page(0, PageGlyphLimits::default())
                .is_err()
        );
    }
}
#[test]
fn builtin_cubic_transform_output_budget_is_cumulative() {
    let bytes = builtin_pdf(
        "/Encoding /WinAnsiEncoding /FirstChar 79 /LastChar 79 /Widths [700]",
        "BT /F 10 Tf (O) Tj ET",
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut probe = PageGlyphSession::new(&doc, builtin_limits()).unwrap();
    let page = probe.trace_page(0, PageGlyphLimits::default()).unwrap();
    assert!(
        page.commands()
            .iter()
            .any(|c| matches!(c, PageGlyphCommand::CubicTo { .. }))
    );
    let mut limits = builtin_limits();
    limits.max_transform_points = probe.usage().transform_points;
    let mut session = PageGlyphSession::new(&doc, limits).unwrap();
    session.trace_page(0, PageGlyphLimits::default()).unwrap();
    assert!(session.trace_page(0, PageGlyphLimits::default()).is_err());
    assert_eq!(
        session.usage().transform_points,
        limits.max_transform_points
    );
}
