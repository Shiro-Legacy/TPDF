// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent page glyph geometry, provenance, ownership and cumulative limits.
use tandem_pdf_engine::{document::Document, page_glyph::*};
fn u16_at(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_be_bytes());
}
fn u32_at(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_be_bytes());
}
fn words(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}
fn glyph_font(units: u16, lsb: i16, empty: bool) -> Vec<u8> {
    let mut head = vec![0; 54];
    u32_at(&mut head, 0, 0x10000);
    u32_at(&mut head, 12, 0x5f0f3cf5);
    u16_at(&mut head, 18, units);
    u16_at(&mut head, 50, 1);
    let mut maxp = vec![0; 32];
    u32_at(&mut maxp, 0, 0x10000);
    u16_at(&mut maxp, 4, 2);
    u16_at(&mut maxp, 6, 1);
    u16_at(&mut maxp, 8, 1);
    // GID1 one off-curve point (100,200), making a move, quadratic and close.
    let mut glyf = words(&[1, 100, 200, 100, 200, 0, 0]);
    glyf.extend([0, 0, 100, 0, 200]);
    if empty {
        glyf.clear();
    }
    let mut loca = vec![0; 12];
    u32_at(&mut loca, 8, glyf.len() as u32);
    let mut hhea = vec![0; 36];
    u32_at(&mut hhea, 0, 0x10000);
    u16_at(&mut hhea, 34, 1);
    // A single long metric: GID1 inherits advance900 but has its own signed lsb.
    let hmtx = words(&[900, 0, lsb as u16]);
    // Microsoft Unicode format4 maps source A to GID1; canonical zero sentinel.
    let mut cmap = words(&[0, 1, 3, 1]);
    cmap.extend(12u32.to_be_bytes());
    cmap.extend(words(&[
        4,
        32,
        0,
        4,
        4,
        1,
        0,
        65,
        65535,
        0,
        65,
        65535,
        (-64i16) as u16,
        1,
        0,
        0,
    ]));
    let tables = [
        (b"head", head),
        (b"maxp", maxp),
        (b"loca", loca),
        (b"glyf", glyf),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"cmap", cmap),
    ];
    let mut bytes = vec![0; 12 + 16 * tables.len()];
    u32_at(&mut bytes, 0, 0x10000);
    u16_at(&mut bytes, 4, tables.len() as u16);
    for (i, (tag, data)) in tables.into_iter().enumerate() {
        let at = 12 + i * 16;
        bytes[at..at + 4].copy_from_slice(tag);
        let offset = bytes.len();
        u32_at(&mut bytes, at + 8, offset as u32);
        u32_at(&mut bytes, at + 12, data.len() as u32);
        bytes.extend(data);
    }
    bytes
}
fn stream(raw: &[u8]) -> Vec<u8> {
    let mut value = format!("<< /Length {} >>\nstream\n", raw.len()).into_bytes();
    value.extend(raw);
    value.extend(b"\nendstream");
    value
}
fn document(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .bytes(),
    );
    out
}
fn pdf(font: &[u8], content: &str, geometry: &str) -> Vec<u8> {
    document(&[
 b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
 format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] {geometry} /Resources << /Font << /F 4 0 R /Alias 4 0 R >> >> /Contents 7 0 R >>").into_bytes(),
 b"<< /Type /Font /Subtype /TrueType /BaseFont /ReviewFont /Encoding /WinAnsiEncoding /FirstChar 65 /LastChar 65 /Widths [250] /FontDescriptor 5 0 R >>".to_vec(),
 b"<< /Type /FontDescriptor /Flags 32 /FontFile2 6 0 R /MissingWidth 250 >>".to_vec(),stream(font),stream(content.as_bytes())])
}
fn near(got: [f64; 2], want: [f64; 2]) {
    for (a, b) in got.into_iter().zip(want) {
        assert!((a - b).abs() < 1e-10, "{got:?} != {want:?}");
    }
}
fn first_point(program: &PageGlyphProgram, index: usize) -> [f64; 2] {
    match program.commands()[program.glyphs()[index].command_range().start] {
        PageGlyphCommand::MoveTo(p) => p.coordinates(),
        other => panic!("unexpected {other:?}"),
    }
}
#[test]
fn drawing_bearing_units_and_pdf_advance_are_independent() {
    for units in [16, 1000, 2048, 16384] {
        let bytes = pdf(
            &glyph_font(units, -40, false),
            "BT /F 10 Tf 1 0 0 1 10 20 Tm (AA) Tj ET",
            "",
        );
        let doc = Document::open(&bytes, Default::default()).unwrap();
        let mut s = PageGlyphSession::new(&doc, Default::default()).unwrap();
        let p = s.trace_page(0, Default::default()).unwrap();
        let x = 10. - 400. / f64::from(units);
        let y = 80. - 2000. / f64::from(units);
        near(first_point(&p, 0), [x, y]);
        near(first_point(&p, 1), [x + 2.5, y]);
        assert_eq!(
            p.glyphs()[0]
                .embedded_true_type()
                .unwrap()
                .outline()
                .header_bounds()
                .unwrap()
                .coordinates(),
            [100, 200, 100, 200]
        );
        assert_eq!(p.page_program().program().text().codes()[0].width(), 250.);
        assert_eq!(p.glyphs()[0].units_per_em(), units);
        assert_eq!(s.usage().outline.decode_attempts, 1);
    }
}
#[test]
fn page_text_and_ctm_transform_each_apply_exactly_once() {
    let bytes = pdf(
        &glyph_font(1000, -40, false),
        "2 1 0.5 3 7 11 cm BT /F 10 Tf 50 Tz 3 Ts 1 0.25 -0.5 2 10 20 Tm (A) Tj ET",
        "/CropBox [5 6 85 96] /Rotate 90 /UserUnit 2",
    );
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let p = PageGlyphSession::new(&doc, Default::default())
        .unwrap()
        .trace_page(0, Default::default())
        .unwrap();
    // Font origin: (-.04,.2); text-size/hscale/rise: (-.2,5).
    // Tm: (7.3,29.95). CTM: (36.575,108.15). R90/crop/unit: (204.3,63.15).
    near(first_point(&p, 0), [204.3, 63.15]);
    match p.commands()[1] {
        PageGlyphCommand::QuadTo { control, end } => {
            near(control.coordinates(), [204.3, 63.15]);
            near(end.coordinates(), [204.3, 63.15]);
        }
        other => panic!("{other:?}"),
    }
}
#[test]
fn alias_repeated_page_cache_is_shared_but_outputs_and_transforms_are_charged() {
    let bytes = pdf(
        &glyph_font(1000, -40, false),
        "BT /F 10 Tf (A) Tj /Alias 10 Tf (AA) Tj ET",
        "",
    );
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut s = PageGlyphSession::new(&doc, Default::default()).unwrap();
    let a = s.trace_page(0, Default::default()).unwrap();
    let before = s.usage();
    let load = s.load_usage();
    let b = s.trace_page(0, Default::default()).unwrap();
    let after = s.usage();
    assert_eq!(
        (
            before.cached_fonts,
            before.cached_glyphs,
            before.occurrences
        ),
        (1, 1, 3)
    );
    assert_eq!(after.font, before.font);
    assert_eq!(after.cmap, before.cmap);
    assert_eq!(after.outline, before.outline);
    assert_eq!(s.load_usage(), load);
    assert_eq!(after.occurrences, 6);
    assert_eq!(after.transformed_commands, 18);
    assert_eq!(after.transform_points, 18);
    assert!(after.allocated_bytes > before.allocated_bytes);
    assert!(std::ptr::eq(
        a.glyphs()[0].embedded_true_type().unwrap().outline(),
        a.glyphs()[2].embedded_true_type().unwrap().outline()
    ));
    assert!(std::ptr::eq(
        a.glyphs()[0].embedded_true_type().unwrap().outline(),
        b.glyphs()[0].embedded_true_type().unwrap().outline()
    ));
    for (i, g) in a.glyphs().iter().enumerate() {
        assert_eq!(g.code_index(), i);
        assert_eq!(g.font_id().number(), 4);
        assert_eq!(g.embedded_true_type().unwrap().program_id().number(), 6);
        assert_eq!(g.command_range(), i * 3..i * 3 + 3);
    }
}
#[test]
fn output_owns_raw_outlines_commands_and_page_provenance_after_inputs_drop() {
    let output = {
        let bytes = pdf(&glyph_font(1000, -40, false), "BT /F 10 Tf (A) Tj ET", "");
        let doc = Document::open(&bytes, Default::default()).unwrap();
        PageGlyphSession::new(&doc, Default::default())
            .unwrap()
            .trace_page(0, Default::default())
            .unwrap()
    };
    assert_eq!(
        output.glyphs()[0]
            .embedded_true_type()
            .unwrap()
            .choice()
            .unwrap()
            .source(),
        65
    );
    assert_eq!(
        output.glyphs()[0]
            .embedded_true_type()
            .unwrap()
            .choice()
            .unwrap()
            .glyph_id()
            .unwrap()
            .get(),
        1
    );
    assert_eq!(
        output.glyphs()[0]
            .embedded_true_type()
            .unwrap()
            .outline()
            .points()
            .len(),
        1
    );
    near(first_point(&output, 0), [-0.4, 98.]);
    let code = &output.page_program().program().text().codes()[0];
    assert!(
        !output
            .page_program()
            .project_range(code.operation_span())
            .unwrap()
            .collect::<Vec<_>>()
            .is_empty()
    );
}
#[test]
fn valid_empty_nonzero_glyph_retains_occurrence_without_commands() {
    let bytes = pdf(&glyph_font(1000, -40, true), "BT /F 10 Tf (AA) Tj ET", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut s = PageGlyphSession::new(&doc, Default::default()).unwrap();
    let p = s.trace_page(0, Default::default()).unwrap();
    assert_eq!(p.glyphs().len(), 2);
    assert!(p.commands().is_empty());
    assert_eq!(p.glyphs()[0].command_range(), 0..0);
    assert_eq!(s.usage().cached_glyphs, 1);
    assert_eq!(s.usage().outline.decode_attempts, 1);
}
#[test]
fn missing_code_refuses_atomically_and_preserves_successful_prior_output() {
    let bytes = pdf(&glyph_font(1000, -40, false), "BT /F 10 Tf (AB) Tj ET", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut s = PageGlyphSession::new(&doc, Default::default()).unwrap();
    let e = s.trace_page(0, Default::default()).unwrap_err();
    assert_eq!(e.code_index, Some(1));
    assert!(e.operation_index.is_some());
    let before = s.usage();
    assert!(s.trace_page(0, Default::default()).is_err());
    assert_eq!(s.usage().occurrences, 4);
    assert_eq!(s.usage().cmap, before.cmap);
}
#[test]
fn exact_cumulative_occurrence_command_and_point_limits_refuse_repeated_page() {
    let bytes = pdf(&glyph_font(1000, -40, false), "BT /F 10 Tf (A) Tj ET", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    for which in 0..3 {
        let mut limits = PageGlyphSessionLimits::default();
        match which {
            0 => limits.max_occurrences = 1,
            1 => limits.max_transformed_commands = 3,
            _ => limits.max_transform_points = 3,
        };
        let mut s = PageGlyphSession::new(&doc, limits).unwrap();
        let first = s.trace_page(0, Default::default()).unwrap();
        assert!(s.trace_page(0, Default::default()).is_err());
        near(first_point(&first, 0), [-0.4, 98.]);
        assert_eq!(s.usage().outline.decode_attempts, 1);
    }
}
#[test]
fn invisible_text_keeps_geometry_and_render_intent_separate() {
    let bytes = pdf(
        &glyph_font(1000, -40, false),
        "0.2 0.4 0.6 rg BT /F 10 Tf 3 Tr (A) Tj ET",
        "",
    );
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let p = PageGlyphSession::new(&doc, Default::default())
        .unwrap()
        .trace_page(0, Default::default())
        .unwrap();
    assert_eq!(
        p.page_program().program().text().codes()[0].rendering_mode(),
        3
    );
    assert_eq!(p.glyphs().len(), 1);
    assert_eq!(p.commands().len(), 3);
    assert!(p.glyphs()[0].context_id() < p.page_program().program().text_contexts().len());
}
#[test]
fn page_failure_does_not_invalidate_prior_owned_output_or_cache() {
    let font = glyph_font(1000, -40, false);
    let bytes=document(&[
 b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),b"<< /Type /Pages /Kids [3 0 R 8 0 R] /Count 2 /Resources << /Font << /F 4 0 R >> >> /MediaBox [0 0 100 100] >>".to_vec(),
 b"<< /Type /Page /Parent 2 0 R /Contents 7 0 R >>".to_vec(),
 b"<< /Type /Font /Subtype /TrueType /BaseFont /Review /Encoding /WinAnsiEncoding /FirstChar 65 /LastChar 65 /Widths [250] /FontDescriptor 5 0 R >>".to_vec(),
 b"<< /Type /FontDescriptor /Flags 32 /FontFile2 6 0 R /MissingWidth 250 >>".to_vec(),stream(&font),stream(b"BT /F 10 Tf (A) Tj ET"),
 b"<< /Type /Page /Parent 2 0 R /Contents 9 0 R >>".to_vec(),stream(b"BT /F 10 Tf (AB) Tj ET")]);
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut s = PageGlyphSession::new(&doc, Default::default()).unwrap();
    let first = s.trace_page(0, Default::default()).unwrap();
    let before = s.usage();
    let failure = s.trace_page(1, Default::default()).unwrap_err();
    assert_eq!(failure.code_index, Some(1));
    assert_eq!(s.usage().outline, before.outline);
    let again = s.trace_page(0, Default::default()).unwrap();
    near(first_point(&first, 0), first_point(&again, 0));
    assert!(std::ptr::eq(
        first.glyphs()[0].embedded_true_type().unwrap().outline(),
        again.glyphs()[0].embedded_true_type().unwrap().outline()
    ));
}
#[test]
fn exact_lookup_and_metrics_budgets_admit_one_page_and_reject_one_less() {
    let bytes = pdf(&glyph_font(1000, -40, false), "BT /F 10 Tf (A) Tj ET", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut baseline = PageGlyphSession::new(&doc, Default::default()).unwrap();
    let p = baseline.trace_page(0, Default::default()).unwrap();
    let used = baseline.usage();
    assert!(used.metrics_work > 0);
    assert!(used.lookup_work > 0);
    assert_eq!(
        p.glyphs()[0].embedded_true_type().unwrap().metric_lsb(),
        -40
    );
    assert_eq!(
        p.glyphs()[0].embedded_true_type().unwrap().origin_shift(),
        -140
    );
    assert_eq!(
        p.glyphs()[0]
            .embedded_true_type()
            .unwrap()
            .hhea_range()
            .len(),
        36
    );
    assert_eq!(
        p.glyphs()[0]
            .embedded_true_type()
            .unwrap()
            .hmtx_range()
            .len(),
        6
    );
    for less in [false, true] {
        for which in 0..2 {
            let mut limits = PageGlyphSessionLimits::default();
            if which == 0 {
                limits.max_lookup_work = used.lookup_work - usize::from(less);
            } else {
                limits.max_metrics_work = used.metrics_work - usize::from(less);
            }
            let mut s = PageGlyphSession::new(&doc, limits).unwrap();
            assert_eq!(s.trace_page(0, Default::default()).is_ok(), !less);
            assert!(s.usage().lookup_work <= limits.max_lookup_work);
            assert!(s.usage().metrics_work <= limits.max_metrics_work);
        }
    }
}
#[test]
fn malformed_metrics_refusal_retains_bounded_work_without_publishing_outline() {
    let mut font = glyph_font(1000, -40, false);
    let record = (0..7)
        .map(|i| 12 + 16 * i)
        .find(|at| &font[*at..*at + 4] == b"hhea")
        .unwrap();
    let at = u32::from_be_bytes(font[record + 8..record + 12].try_into().unwrap()) as usize;
    u16_at(&mut font, at + 34, 0);
    let bytes = pdf(&font, "BT /F 10 Tf (A) Tj ET", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let limits = PageGlyphSessionLimits {
        max_metrics_work: 136,
        ..Default::default()
    };
    let mut s = PageGlyphSession::new(&doc, limits).unwrap();
    assert!(s.trace_page(0, Default::default()).is_err());
    assert_eq!(s.usage().metrics_work, 68);
    assert!(s.trace_page(0, Default::default()).is_err());
    assert_eq!(s.usage().metrics_work, 136);
    assert!(s.trace_page(0, Default::default()).is_err());
    assert_eq!(s.usage().metrics_work, 136);
    assert_eq!(s.usage().cached_glyphs, 0);
    assert_eq!(s.usage().outline.decode_attempts, 0);
}
#[test]
fn glyph_selection_ignores_tounicode_and_retains_cross_stream_source() {
    let bytes=document(&[
 b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
 b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /Font << /F 4 0 R >> >> /Contents [7 0 R 8 0 R] >>".to_vec(),
 b"<< /Type /Font /Subtype /TrueType /BaseFont /Review /Encoding /WinAnsiEncoding /FirstChar 65 /LastChar 65 /Widths [250] /FontDescriptor 5 0 R /ToUnicode 9 0 R >>".to_vec(),
 b"<< /Type /FontDescriptor /Flags 32 /FontFile2 6 0 R >>".to_vec(),stream(&glyph_font(1000,-40,false)),stream(b"BT /F 10 Tf <"),stream(b"41> Tj ET"),
 stream(b"begincmap 1 begincodespacerange <00><ff> endcodespacerange 1 beginbfchar <41><00420043> endbfchar endcmap")]);
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let p = PageGlyphSession::new(&doc, Default::default())
        .unwrap()
        .trace_page(0, Default::default())
        .unwrap();
    assert_eq!(p.glyphs().len(), 1);
    assert_eq!(
        p.glyphs()[0]
            .embedded_true_type()
            .unwrap()
            .choice()
            .unwrap()
            .glyph_id()
            .unwrap()
            .get(),
        1
    );
    let text = p.page_program().program().text();
    assert_eq!(text.codes()[0].utf8_span().len(), 2);
    let pieces = p
        .page_program()
        .project_range(text.codes()[0].operation_span())
        .unwrap()
        .collect::<Vec<_>>();
    assert_eq!(pieces.len(), 3);
    near(first_point(&p, 0), [-0.4, 98.]);
}
#[test]
fn shared_fontfile_does_not_reuse_a_different_font_dictionary_encoding_policy() {
    let bytes=document(&[
 b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
 b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /Font << /F 4 0 R /Other 8 0 R >> >> /Contents 7 0 R >>".to_vec(),
 b"<< /Type /Font /Subtype /TrueType /BaseFont /Review /Encoding /WinAnsiEncoding /FirstChar 65 /LastChar 65 /Widths [250] /FontDescriptor 5 0 R >>".to_vec(),
 b"<< /Type /FontDescriptor /Flags 32 /FontFile2 6 0 R >>".to_vec(),stream(&glyph_font(1000,-40,false)),stream(b"BT /F 10 Tf (A) Tj /Other 10 Tf (A) Tj ET"),
 b"<< /Type /Font /Subtype /TrueType /BaseFont /Review /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /UnknownReviewGlyph] >> /FirstChar 65 /LastChar 65 /Widths [250] /FontDescriptor 5 0 R /ToUnicode 9 0 R >>".to_vec(),
 stream(b"begincmap 1 begincodespacerange <00><ff> endcodespacerange 1 beginbfchar <41><0041> endbfchar endcmap")]);
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut s = PageGlyphSession::new(&doc, Default::default()).unwrap();
    let e = s.trace_page(0, Default::default()).unwrap_err();
    assert_eq!(e.code_index, Some(1));
    assert_eq!(s.usage().cached_fonts, 2);
    assert_eq!(s.usage().cmap.selection_attempts, 2);
}
