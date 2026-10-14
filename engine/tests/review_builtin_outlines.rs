// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent R20 static asset, PDF binding and policy review.
use tandem_pdf_engine::{document::Document, page_glyph::*, page_program::PageMetricPolicy};
fn stream(raw: &[u8]) -> Vec<u8> {
    let mut out = format!("<< /Length {} >>\nstream\n", raw.len()).into_bytes();
    out.extend(raw);
    out.extend(b"\nendstream");
    out
}
fn pdf(font: &str, descriptor: &str, content: &str, other: Option<&str>) -> Vec<u8> {
    let objects=[b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /Font << /F 4 0 R /Alias 4 0 R /Other 8 0 R >> >> /Contents 6 0 R >>".to_vec(),font.as_bytes().to_vec(),descriptor.as_bytes().to_vec(),stream(content.as_bytes()),stream(b"begincmap 1 begincodespacerange <00><ff> endcodespacerange 1 beginbfchar <41><00580059> endbfchar endcmap"),other.unwrap_or(font).as_bytes().to_vec()];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
    }
    let at = out.len();
    out.extend(b"xref\n0 9\n0000000000 65535 f \n");
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size 9 /Root 1 0 R >>\nstartxref\n{at}\n%%EOF\n").bytes());
    out
}
fn font(face: &str, encoding: &str, widths: &str, extra: &str) -> String {
    format!(
        "<< /Type /Font /Subtype /Type1 /BaseFont /{face} /Encoding {encoding} {widths} {extra} >>"
    )
}
fn declared() -> &'static str {
    "/FirstChar 65 /LastChar 65 /Widths [200]"
}
fn enabled() -> PageGlyphSessionLimits {
    PageGlyphSessionLimits {
        outline_policy: PageOutlinePolicy::AllowPinnedBase14,
        ..Default::default()
    }
}
#[test]
fn outline_and_metric_policies_are_independent_and_explicit() {
    let declared = font("Helvetica", "/WinAnsiEncoding", declared(), "");
    let fallback = font("Helvetica", "/WinAnsiEncoding", "", "");
    for (dictionary, needs_metrics) in [(&declared, false), (&fallback, true)] {
        let bytes = pdf(dictionary, "null", "BT /F 10 Tf (A) Tj ET", None);
        let doc = Document::open(&bytes, Default::default()).unwrap();
        for outline in [false, true] {
            for metrics in [false, true] {
                let mut limits = PageGlyphSessionLimits::default();
                if outline {
                    limits.outline_policy = PageOutlinePolicy::AllowPinnedBase14;
                }
                if metrics {
                    limits.page.metric_policy = PageMetricPolicy::AllowPinnedBase14;
                }
                let result = PageGlyphSession::new(&doc, limits)
                    .unwrap()
                    .trace_page(0, Default::default());
                assert_eq!(
                    result.is_ok(),
                    outline && (!needs_metrics || metrics),
                    "outline{outline} metrics{metrics} needs{needs_metrics}"
                );
            }
        }
    }
}
#[test]
fn encoding_name_selects_outline_while_tounicode_and_pdf_width_remain_independent() {
    let bytes = pdf(
        &font(
            "Helvetica",
            "<< /BaseEncoding /WinAnsiEncoding /Differences [65 /fi] >>",
            declared(),
            "/ToUnicode 7 0 R",
        ),
        "null",
        "BT /F 10 Tf (AA) Tj ET",
        None,
    );
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let p = PageGlyphSession::new(&doc, enabled())
        .unwrap()
        .trace_page(0, Default::default())
        .unwrap();
    assert_eq!(p.glyphs().len(), 2);
    for g in p.glyphs() {
        assert!(g.embedded_true_type().is_none());
        assert_eq!(g.pinned_base14().unwrap().name(), b"fi");
        assert_eq!(g.units_per_em(), 1000);
    }
    let codes = p.page_program().program().text().codes();
    assert_eq!(codes[0].utf8_span().len(), 2);
    assert_eq!(codes[0].width(), 200.);
    assert_eq!(codes[1].origin().x() - codes[0].origin().x(), 2.);
    assert_eq!(
        p.glyphs()[0].pinned_base14().unwrap() as *const _,
        p.glyphs()[1].pinned_base14().unwrap() as *const _
    );
}
#[test]
fn exact_faces_only_no_subset_prefix_style_or_case_guessing() {
    for face in [
        "Arial",
        "Helvetica-Bold",
        "ABCDEF+Helvetica",
        "helvetica",
        "Times",
    ] {
        let bytes = pdf(
            &font(face, "/WinAnsiEncoding", declared(), ""),
            "null",
            "BT /F 10 Tf (A) Tj ET",
            None,
        );
        let doc = Document::open(&bytes, Default::default()).unwrap();
        assert!(
            PageGlyphSession::new(&doc, enabled())
                .unwrap()
                .trace_page(0, Default::default())
                .is_err(),
            "{face}"
        );
    }
}
#[test]
fn embedded_or_malformed_program_declarations_never_fall_back() {
    for key in ["FontFile", "FontFile2", "FontFile3"] {
        for value in ["42", "<< >>", "999 0 R", "6 0 R"] {
            let descriptor = format!("<< /Type /FontDescriptor /{key} {value} >>");
            let bytes = pdf(
                &font(
                    "Helvetica",
                    "/WinAnsiEncoding",
                    declared(),
                    "/FontDescriptor 5 0 R",
                ),
                &descriptor,
                "BT /F 10 Tf (A) Tj ET",
                None,
            );
            let doc = Document::open(&bytes, Default::default()).unwrap();
            assert!(
                PageGlyphSession::new(&doc, enabled())
                    .unwrap()
                    .trace_page(0, Default::default())
                    .is_err(),
                "{key} {value}"
            );
        }
    }
    for descriptor in ["42", "/FontDescriptor", "<< /Type /Font >>"] {
        let bytes = pdf(
            &font(
                "Helvetica",
                "/WinAnsiEncoding",
                declared(),
                "/FontDescriptor 5 0 R",
            ),
            descriptor,
            "BT /F 10 Tf (A) Tj ET",
            None,
        );
        let doc = Document::open(&bytes, Default::default()).unwrap();
        assert!(
            PageGlyphSession::new(&doc, enabled())
                .unwrap()
                .trace_page(0, Default::default())
                .is_err()
        );
    }
}
#[test]
fn null_programs_and_valid_empty_glyph_are_distinct_from_notdef_or_unknown_names() {
    let descriptor = "<< /Type /FontDescriptor /FontFile null /FontFile2 null /FontFile3 null >>";
    let good = font(
        "Times-Roman",
        "<< /BaseEncoding /WinAnsiEncoding /Differences [65 /space] >>",
        declared(),
        "/FontDescriptor 5 0 R /ToUnicode 7 0 R",
    );
    let bytes = pdf(&good, descriptor, "BT /F 10 Tf (A) Tj ET", None);
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let p = PageGlyphSession::new(&doc, enabled())
        .unwrap()
        .trace_page(0, Default::default())
        .unwrap();
    assert_eq!(p.glyphs()[0].pinned_base14().unwrap().name(), b"space");
    assert!(p.commands().is_empty());
    for name in [".notdef", "A.alt", "UnknownReviewName", "zerosuperior"] {
        let dictionary = font(
            "Helvetica",
            &format!("<< /BaseEncoding /WinAnsiEncoding /Differences [65 /{name}] >>"),
            declared(),
            "/ToUnicode 7 0 R",
        );
        let bytes = pdf(&dictionary, "null", "BT /F 10 Tf (A) Tj ET", None);
        let doc = Document::open(&bytes, Default::default()).unwrap();
        let e = PageGlyphSession::new(&doc, enabled())
            .unwrap()
            .trace_page(0, Default::default())
            .unwrap_err();
        assert_eq!(e.code_index, Some(0), "{name}");
    }
}
#[test]
fn source_and_commands_survive_document_input_and_session_drop() {
    let p = {
        let bytes = pdf(
            &font("Times-Roman", "/WinAnsiEncoding", declared(), ""),
            "null",
            "BT /F 10 Tf (A) Tj ET",
            None,
        );
        let doc = Document::open(&bytes, Default::default()).unwrap();
        PageGlyphSession::new(&doc, enabled())
            .unwrap()
            .trace_page(0, Default::default())
            .unwrap()
    };
    assert_eq!(p.glyphs()[0].pinned_base14().unwrap().name(), b"A");
    assert!(!p.commands().is_empty());
    assert_eq!(p.glyphs()[0].font_id().number(), 4);
}
#[test]
fn aliases_share_cache_while_same_face_dictionaries_keep_their_own_encoding() {
    let normal = font("Helvetica", "/WinAnsiEncoding", declared(), "");
    let other = font(
        "Helvetica",
        "<< /BaseEncoding /WinAnsiEncoding /Differences [65 /space] >>",
        declared(),
        "",
    );
    let bytes = pdf(
        &normal,
        "null",
        "BT /F 10 Tf (A) Tj /Alias 10 Tf (A) Tj /Other 10 Tf (A) Tj ET",
        Some(&other),
    );
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut s = PageGlyphSession::new(&doc, enabled()).unwrap();
    let a = s.trace_page(0, Default::default()).unwrap();
    let before = s.usage();
    let load = s.load_usage();
    assert_eq!(before.cached_fonts, 2);
    assert_eq!(a.glyphs()[0].pinned_base14().unwrap().name(), b"A");
    assert_eq!(a.glyphs()[2].pinned_base14().unwrap().name(), b"space");
    s.trace_page(0, Default::default()).unwrap();
    let after = s.usage();
    assert_eq!(s.load_usage(), load);
    assert_eq!(after.occurrences, 6);
    assert_eq!(after.transformed_commands, 2 * before.transformed_commands);
    assert!(after.lookup_work > before.lookup_work);
    assert!(after.allocated_bytes > before.allocated_bytes);
    assert_eq!(after.font, before.font);
    assert_eq!(after.cmap, before.cmap);
}
#[test]
fn every_static_record_matches_metric_membership_ranges_and_closed_command_grammar() {
    use tandem_pdf_engine::{
        builtin_metrics::BuiltinFace,
        builtin_outlines::{BuiltinOutline, BuiltinOutlineCommand as C, MAX_GLYPH_NAME_BYTES},
    };
    let mut totals = [0usize; 4];
    for (face, expected) in [
        (BuiltinFace::Helvetica, 12788),
        (BuiltinFace::TimesRoman, 17838),
    ] {
        let records = BuiltinOutline::glyphs(face).collect::<Vec<_>>();
        assert_eq!(records.len(), 654);
        let metric_names = face.glyphs().map(|(name, _)| name).collect::<Vec<_>>();
        assert_eq!(
            records.iter().map(|r| r.name()).collect::<Vec<_>>(),
            metric_names
        );
        let mut end = 0;
        let mut seen = std::collections::BTreeSet::new();
        for record in records {
            assert_eq!(record.face(), face);
            assert_eq!(record.units_per_em(), 1000);
            assert!(!record.name().is_empty());
            assert!(record.name().len() <= MAX_GLYPH_NAME_BYTES);
            assert!(seen.insert(record.native_gid()));
            assert_eq!(record.command_range().start, end);
            assert_eq!(record.command_range().len(), record.commands().len());
            end = record.command_range().end;
            assert!(std::ptr::eq(
                BuiltinOutline::lookup(face, record.name()).unwrap(),
                record
            ));
            let mut open = false;
            for command in record.commands() {
                let coordinates = match *command {
                    C::MoveTo { x, y } => {
                        assert!(!open, "unclosed {}", String::from_utf8_lossy(record.name()));
                        open = true;
                        totals[0] += 1;
                        vec![x, y]
                    }
                    C::LineTo { x, y } => {
                        assert!(open);
                        totals[1] += 1;
                        vec![x, y]
                    }
                    C::CubicTo {
                        x1,
                        y1,
                        x2,
                        y2,
                        x,
                        y,
                    } => {
                        assert!(open);
                        totals[2] += 1;
                        vec![x1, y1, x2, y2, x, y]
                    }
                    C::Close => {
                        assert!(open);
                        open = false;
                        totals[3] += 1;
                        vec![]
                    }
                };
                assert!(coordinates.iter().all(|v| v.unsigned_abs() <= 1 << 20));
            }
            assert!(!open);
        }
        assert_eq!(end, expected);
        assert!(
            BuiltinOutline::lookup(face, b"space")
                .unwrap()
                .commands()
                .is_empty()
        );
        assert_eq!(
            BuiltinOutline::lookup(face, b".notdef")
                .unwrap()
                .native_gid(),
            0
        );
    }
    assert_eq!(totals, [2429, 13943, 11825, 2429]);
    assert!(BuiltinOutline::static_bytes() < 2 * 1024 * 1024);
}
#[test]
fn exact_static_lookup_rejects_long_names_case_aliases_and_invalid_bytes() {
    use tandem_pdf_engine::{builtin_metrics::BuiltinFace, builtin_outlines::BuiltinOutline};
    for face in [BuiltinFace::Helvetica, BuiltinFace::TimesRoman] {
        for name in [
            b"A.alt".as_slice(),
            b"SPACE",
            b"A\0",
            b"\xff",
            b"UnknownReviewName",
        ] {
            assert!(BuiltinOutline::lookup(face, name).is_none());
        }
        assert!(BuiltinOutline::lookup(face, &[b'A'; 129]).is_none());
        assert!(BuiltinOutline::lookup_work_bound(face) > 128);
    }
}
#[test]
fn failed_names_repay_lookup_work_and_stop_at_the_cumulative_limit() {
    let dictionary = font(
        "Helvetica",
        "<< /BaseEncoding /WinAnsiEncoding /Differences [65 /UnknownReviewName] >>",
        declared(),
        "/ToUnicode 7 0 R",
    );
    let bytes = pdf(&dictionary, "null", "BT /F 10 Tf (A) Tj ET", None);
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut baseline = PageGlyphSession::new(&doc, enabled()).unwrap();
    assert!(baseline.trace_page(0, Default::default()).is_err());
    let first = baseline.usage();
    assert!(baseline.trace_page(0, Default::default()).is_err());
    let second = baseline.usage();
    assert!(second.lookup_work > first.lookup_work);
    assert_eq!(second.cached_glyphs, 0);
    assert_eq!(second.cached_fonts, 1);
    let mut limits = enabled();
    limits.max_lookup_work = second.lookup_work;
    let mut s = PageGlyphSession::new(&doc, limits).unwrap();
    for _ in 0..4 {
        assert!(s.trace_page(0, Default::default()).is_err());
        assert!(s.usage().lookup_work <= limits.max_lookup_work);
    }
    assert_eq!(s.usage().cached_glyphs, 0);
}
#[test]
fn builtin_occurrence_commands_points_and_cache_choices_have_exact_budget_edges() {
    let bytes = pdf(
        &font("Helvetica", "/WinAnsiEncoding", declared(), ""),
        "null",
        "BT /F 10 Tf (A) Tj ET",
        None,
    );
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut baseline = PageGlyphSession::new(&doc, enabled()).unwrap();
    baseline.trace_page(0, Default::default()).unwrap();
    let used = baseline.usage();
    assert_eq!(used.cached_glyphs, 1);
    assert!(used.transform_points > 0);
    assert!(used.transformed_commands > 0);
    for less in [false, true] {
        for which in 0..5 {
            let mut limits = enabled();
            match which {
                0 => limits.max_cached_glyphs = used.cached_glyphs - usize::from(less),
                1 => {
                    limits.max_transformed_commands = used.transformed_commands - usize::from(less)
                }
                2 => limits.max_transform_points = used.transform_points - usize::from(less),
                3 => limits.max_lookup_work = used.lookup_work - usize::from(less),
                _ => limits.max_allocated_bytes = used.allocated_bytes - usize::from(less),
            }
            let mut s = PageGlyphSession::new(&doc, limits).unwrap();
            assert_eq!(
                s.trace_page(0, Default::default()).is_ok(),
                !less,
                "{which} {less}"
            );
        }
    }
}
#[test]
fn canonical_cubics_use_font_origin_and_noncommuting_page_transform_once() {
    use tandem_pdf_engine::builtin_outlines::BuiltinOutlineCommand as C;
    let bytes = pdf(
        &font(
            "Helvetica",
            "<< /BaseEncoding /WinAnsiEncoding /Differences [65 /O] >>",
            declared(),
            "",
        ),
        "null",
        "2 1 0.5 3 7 11 cm BT /F 10 Tf 50 Tz 3 Ts 1 0.25 -0.5 2 10 20 Tm (A) Tj ET",
        None,
    );
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let p = PageGlyphSession::new(&doc, enabled())
        .unwrap()
        .trace_page(0, Default::default())
        .unwrap();
    let raw = p.glyphs()[0].pinned_base14().unwrap().commands();
    assert_eq!(raw.len(), p.commands().len());
    assert!(raw.iter().any(|c| matches!(c, C::CubicTo { .. })));
    let expected = |x: i32, y: i32| {
        let x = f64::from(x) / 1000. * 5.;
        let y = f64::from(y) / 1000. * 10. + 3.;
        let tx = x - 0.5 * y + 10.;
        let ty = 0.25 * x + 2. * y + 20.;
        [2. * tx + 0.5 * ty + 7., 100. - (tx + 3. * ty + 11.)]
    };
    let check = |got: [f64; 2], want: [f64; 2]| {
        assert!(
            (got[0] - want[0]).abs() < 1e-11 && (got[1] - want[1]).abs() < 1e-11,
            "{got:?} {want:?}"
        );
    };
    for (a, b) in raw.iter().zip(p.commands()) {
        match (a, b) {
            (C::MoveTo { x, y }, PageGlyphCommand::MoveTo(p))
            | (C::LineTo { x, y }, PageGlyphCommand::LineTo(p)) => {
                check(p.coordinates(), expected(*x, *y))
            }
            (
                C::CubicTo {
                    x1,
                    y1,
                    x2,
                    y2,
                    x,
                    y,
                },
                PageGlyphCommand::CubicTo {
                    control1,
                    control2,
                    end,
                },
            ) => {
                check(control1.coordinates(), expected(*x1, *y1));
                check(control2.coordinates(), expected(*x2, *y2));
                check(end.coordinates(), expected(*x, *y));
            }
            (C::Close, PageGlyphCommand::Close) => {}
            _ => panic!("command kind mismatch"),
        }
    }
}
#[test]
fn canonical_cubic_raster_path_honors_invisibility_and_flattening_caps() {
    use tandem_pdf_engine::{
        geometry::Affine,
        page_raster::{PageRasterLimits, PageRasterSession, RasterTarget},
    };
    let target = RasterTarget {
        width: 100,
        height: 100,
        page_to_device: Affine::from_pdf([1., 0., 0., 1., 0., 0.]).unwrap(),
    };
    for mode in [0, 3] {
        let bytes = pdf(
            &font(
                "Times-Roman",
                "<< /BaseEncoding /WinAnsiEncoding /Differences [65 /O] >>",
                declared(),
                "",
            ),
            "null",
            &format!("BT /F 40 Tf {mode} Tr 1 0 0 1 10 20 Tm (A) Tj ET"),
            None,
        );
        let doc = Document::open(&bytes, Default::default()).unwrap();
        let p = PageGlyphSession::new(&doc, enabled())
            .unwrap()
            .trace_page(0, Default::default())
            .unwrap();
        let mut raster = PageRasterSession::new(Default::default());
        let image = raster.render(&p, target, Default::default()).unwrap();
        if mode == 0 {
            assert!(image.pixels().chunks_exact(4).any(|p| p[0] < 255));
            assert!(raster.usage().curve_nodes > 0);
            assert!(
                PageRasterSession::new(Default::default())
                    .render(
                        &p,
                        target,
                        PageRasterLimits {
                            max_subdivision_depth: 0,
                            ..Default::default()
                        }
                    )
                    .is_err()
            );
        } else {
            assert!(image.pixels().iter().all(|p| *p == 255));
            assert_eq!(raster.usage().source_commands, 0);
        }
    }
}
