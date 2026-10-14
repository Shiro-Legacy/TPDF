// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent real-PDF Form state, scope, affine, lifetime and failure tests.
use tandem_pdf_engine::{
    document::{Document, DocumentLimits},
    font_resources::FontIdentity,
    object::ObjectId,
    page_program::{PageProgram, PageProgramSession, PageProgramSessionLimits},
    path::{DeviceColor, PathCommand, PathEvent},
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
fn stream(meta: &str, raw: &str) -> String {
    format!(
        "<< {meta} /Length {} >>\nstream\n{raw}\nendstream",
        raw.len()
    )
}
fn form(meta: &str, raw: &str) -> String {
    stream(&format!("/Type /XObject /Subtype /Form {meta}"), raw)
}
fn font(width: usize, name: &str) -> String {
    format!(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 65 /LastChar 65 /Widths [{width}] /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /{name}] >> >>"
    )
}
// Catalog1 Pages2 Page3 Content4 FontA5 FontB6; appended objects begin at7.
fn fixture(raw: &str, resources: &str, forms: &[String]) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << {resources} >> /Contents 4 0 R >>"
        ),
        stream("", raw),
        font(500, "A"),
        font(700, "B"),
    ];
    objects.extend_from_slice(forms);
    pdf(&objects)
}
fn trace(bytes: &[u8]) -> PageProgram {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    PageProgramSession::new(&doc, Default::default())
        .unwrap()
        .trace_page(0, Default::default())
        .unwrap()
}
fn refused(bytes: &[u8]) {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    assert!(session.trace_page(0, Default::default()).is_err());
}
const ROOT: &str = "/Font << /F 5 0 R >> /XObject << /X 7 0 R >>";

#[test]
fn inherited_current_font_is_independent_of_local_resource_scope_and_restored() {
    let bytes = fixture(
        "BT /F 10 Tf ET /X Do BT (A) Tj ET",
        ROOT,
        &[form(
            "/BBox [0 0 100 100] /Resources << /Font << /F 6 0 R >> >>",
            "BT (A) Tj /F 20 Tf (A) Tj ET",
        )],
    );
    let result = trace(&bytes);
    let text = result.program().text();
    assert_eq!(text.text(), "ABA");
    assert_eq!(
        text.codes().iter().map(|c| c.width()).collect::<Vec<_>>(),
        [500., 700., 500.]
    );
    assert_eq!(
        text.codes()[0].transform().coefficients(),
        [10., 0., 0., -10., 0., 100.]
    );
    assert_eq!(
        text.codes()[1].transform().coefficients(),
        [20., 0., 0., -20., 5., 100.]
    );
    assert_eq!(
        text.codes()[2].transform().coefficients(),
        [10., 0., 0., -10., 0., 100.]
    );
    assert_eq!(result.fonts().len(), 2);
    assert_eq!(result.fonts()[0].identity(), FontIdentity::Indirect(id(5)));
    assert_eq!(result.fonts()[1].identity(), FontIdentity::Indirect(id(6)));
}

#[test]
fn repeated_resource_less_form_uses_each_callers_scope_not_first_cached_binding() {
    let bytes = fixture(
        "/A Do /B Do /A Do",
        "/XObject << /A 7 0 R /B 8 0 R >>",
        &[
            form(
                "/BBox [0 0 100 100] /Resources << /Font << /F 5 0 R >> /XObject << /Child 9 0 R >> >>",
                "/Child Do",
            ),
            form(
                "/BBox [0 0 100 100] /Resources << /Font << /F 6 0 R >> /XObject << /Child 9 0 R >> >>",
                "/Child Do",
            ),
            form("/BBox [0 0 100 100]", "BT /F 10 Tf (A) Tj ET"),
        ],
    );
    let result = trace(&bytes);
    assert_eq!(result.program().text().text(), "ABA");
    assert_eq!(
        result
            .program()
            .text()
            .codes()
            .iter()
            .map(|c| c.width())
            .collect::<Vec<_>>(),
        [500., 700., 500.]
    );
    assert_eq!(result.fonts().len(), 2);
}

#[test]
fn absent_null_and_explicit_empty_resources_have_distinct_lookup_semantics() {
    for resources in ["", "/Resources null"] {
        let bytes = fixture(
            "/X Do",
            ROOT,
            &[form(
                &format!("/BBox [0 0 100 100] {resources}"),
                "BT /F 10 Tf (A) Tj ET",
            )],
        );
        assert_eq!(trace(&bytes).program().text().text(), "A");
    }
    for resources in ["/Resources << >>", "/Resources << /Font << >> >>"] {
        let meta = format!("/BBox [0 0 100 100] {resources}");
        refused(&fixture(
            "/X Do",
            ROOT,
            &[form(&meta, "BT /F 10 Tf (A) Tj ET")],
        ));
        let bytes = fixture("BT /F 10 Tf ET /X Do", ROOT, &[form(&meta, "BT (A) Tj ET")]);
        assert_eq!(trace(&bytes).program().text().text(), "A");
    }
    // A different category present in local resources cannot merge a parent Font.
    refused(&fixture(
        "/X Do",
        ROOT,
        &[form(
            "/BBox [0 0 10 10] /Resources << /XObject << >> >>",
            "BT /F 10 Tf ET",
        )],
    ));
}

#[test]
fn full_affine_bbox_and_clip_ancestry_are_preserved_and_caller_state_returns() {
    let bytes = fixture(
        "0 0 90 90 re W n 0.25 g 2 1 1 3 10 20 cm /X Do 0 0 1 1 re f",
        ROOT,
        &[form(
            "/BBox [1 2 5 7] /Matrix [1 2 3 1 4 5]",
            "1 0 0 rg 0 0 10 10 re f",
        )],
    );
    let result = trace(&bytes);
    let paths = result.program().paths();
    let fills = paths
        .events()
        .iter()
        .filter_map(|event| match event {
            PathEvent::Fill { paint, .. } => Some(paint),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(fills.len(), 2);
    assert_eq!(fills[0].ctm().coefficients(), [4., -7., 7., -6., 23., 61.]);
    assert_eq!(fills[1].ctm().coefficients(), [2., -1., 1., -3., 10., 80.]);
    assert_eq!(fills[0].color(), DeviceColor::Rgb([1., 0., 0.]));
    assert_eq!(fills[1].color(), DeviceColor::Gray(0.25));
    let inner = &paths.clips()[fills[0].clip().unwrap().index()];
    assert_eq!(inner.parent(), fills[1].clip());
    assert_eq!(inner.ctm().coefficients(), [4., -7., 7., -6., 23., 61.]);
    let corners = paths.paths()[inner.path().index()]
        .commands()
        .iter()
        .filter_map(|c| match c {
            PathCommand::MoveTo(p) | PathCommand::LineTo(p) => Some(p.coordinates()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(corners, [[1., 2.], [5., 2.], [5., 7.], [1., 7.]]);
}

#[test]
fn all_text_parameters_and_color_restore_across_form_not_just_font_size() {
    let raw =
        "0.25 g BT /F 10 Tf 2 Tc 3 Tw 80 Tz 12 TL 3 Tr 4 Ts ET /X Do BT (A A) Tj T* (A) Tj ET";
    let parent_font = format!(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 32 /LastChar 65 /Widths [{}] /Encoding /WinAnsiEncoding >>",
        "500 ".repeat(34)
    );
    let bytes = fixture(
        raw,
        &ROOT.replace("5 0 R", "8 0 R"),
        &[
            form(
                "/BBox [0 0 100 100]",
                "1 0 0 rg BT (A A) Tj /F 20 Tf 9 Tc 2 Tw 50 Tz 30 TL 0 Tr 2 Ts ET",
            ),
            parent_font,
        ],
    );
    let result = trace(&bytes);
    let codes = result.program().text().codes();
    assert_eq!(codes.len(), 7);
    for (i, x) in [(0, 0.), (1, 5.6), (2, 13.6), (3, 0.), (4, 5.6), (5, 13.6)] {
        let actual = codes[i].transform().coefficients();
        for (a, e) in actual.into_iter().zip([8., 0., 0., -10., x, 96.]) {
            assert!((a - e).abs() < 1e-12, "code {i}: {actual:?}");
        }
    }
    assert_eq!(
        codes[6].transform().coefficients(),
        [8., 0., 0., -10., 0., 108.]
    );
    for (i, code) in codes.iter().enumerate() {
        assert_eq!(code.character_spacing(), 2.);
        assert_eq!(code.word_spacing(), if i == 1 || i == 4 { 3. } else { 0. });
        assert_eq!(code.font_size(), 10.);
        assert_eq!(code.horizontal_scale(), 0.8);
        assert_eq!(code.rise(), 4.);
        assert_eq!(code.rendering_mode(), 3);
    }
    let contexts = result.program().text_contexts();
    assert!(
        contexts
            .iter()
            .any(|c| c.fill_color() == DeviceColor::Rgb([1., 0., 0.]))
    );
    assert!(
        contexts
            .iter()
            .any(|c| c.fill_color() == DeviceColor::Gray(0.25))
    );
}

#[test]
fn form_frame_floor_and_unfinished_state_cannot_escape_into_parent() {
    for raw in ["Q", "q", "BT", "0 0 m", "0 0 1 1 re W", "q Q Q", "BT q ET"] {
        refused(&fixture(
            "q /X Do Q",
            ROOT,
            &[form("/BBox [0 0 10 10]", raw)],
        ));
    }
    for prefix in ["BT /X Do ET", "0 0 m /X Do", "0 0 1 1 re W /X Do"] {
        refused(&fixture(prefix, ROOT, &[form("/BBox [0 0 10 10]", "")]));
    }
    trace(&fixture(
        "q /X Do Q",
        ROOT,
        &[form("/BBox [0 0 10 10]", "q 1 0 0 1 2 3 cm Q")],
    ));
}

#[test]
fn only_active_ancestry_is_a_cycle_and_unused_bad_forms_are_lazy() {
    let resources = "/XObject << /X 7 0 R /Alias 7 0 R /Bad 8 0 R >>";
    let good = form("/BBox [0 0 10 10]", "0 0 1 1 re f");
    let bad = form("/BBox /Broken /Group << /S /Transparency >>", "/Bad Do");
    let bytes = fixture("/X Do /Alias Do /X Do", resources, &[good.clone(), bad]);
    let result = trace(&bytes);
    assert_eq!(
        result
            .program()
            .paths()
            .events()
            .iter()
            .filter(|e| matches!(e, PathEvent::Fill { .. }))
            .count(),
        3
    );
    refused(&fixture(
        "/X Do",
        resources,
        &[form("/BBox [0 0 10 10]", "/Alias Do"), good],
    ));
    refused(&fixture(
        "/X Do",
        ROOT,
        &[
            form(
                "/BBox [0 0 10 10] /Resources << /XObject << /Y 8 0 R >> >>",
                "/Y Do",
            ),
            form(
                "/BBox [0 0 10 10] /Resources << /XObject << /Z 7 0 R >> >>",
                "/Z Do",
            ),
        ],
    ));
}

#[test]
fn used_form_metadata_rejects_alternate_semantics_and_malformed_geometry() {
    for extra in [
        "/Group << /S /Transparency >>",
        "/Ref << >>",
        "/OPI << >>",
        "/Subtype2 /PS",
        "/FormType 2",
        "/Matrix [1 0 0 0 0 0]",
        "/Resources << /ColorSpace << /DefaultRGB /DeviceRGB >> >>",
    ] {
        refused(&fixture(
            "/X Do",
            ROOT,
            &[form(&format!("/BBox [0 0 10 10] {extra}"), "0 0 1 1 re f")],
        ));
    }
    for bbox in [
        "",
        "/BBox [0 0 0 10]",
        "/BBox [1 0 0 10]",
        "/BBox [0 0 10]",
        "/BBox /Bad",
    ] {
        refused(&fixture("/X Do", ROOT, &[form(bbox, "")]));
    }
    trace(&fixture(
        "/X Do",
        ROOT,
        &[form(
            "/BBox [0 0 10 10] /Group null /OC null /Ref null /OPI null /Subtype2 null /FormType 1",
            "",
        )],
    ));
}

#[test]
fn owned_form_text_and_paths_outlive_all_borrowed_inputs_and_session() {
    let result = {
        let bytes = fixture(
            "/X Do /X Do",
            ROOT,
            &[form(
                "/BBox [0 0 10 10]",
                "0 0 1 1 re f BT /F 10 Tf (A) Tj ET",
            )],
        );
        trace(&bytes)
    };
    assert_eq!(result.program().text().text(), "AA");
    assert_eq!(result.program().paths().clips().len(), 2);
    assert_eq!(result.program().text().codes()[0].width(), 500.);
}

#[test]
fn repeated_nested_parse_failures_retain_attempts_and_work() {
    let bytes = fixture(
        "0 g /X Do 0 g",
        ROOT,
        &[
            form(
                "/BBox [0 0 10 10] /Resources << /XObject << /Y 8 0 R >> >>",
                "0 g /Y Do 0 g",
            ),
            form("/BBox [0 0 10 10]", "0 g ["),
        ],
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut session = PageProgramSession::new(
        &doc,
        PageProgramSessionLimits {
            max_page_attempts: 2,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(session.trace_page(0, Default::default()).is_err());
    let first = session.usage();
    assert!(first.parser_input_bytes > 0);
    assert!(first.parser_operations > 0);
    assert!(session.trace_page(0, Default::default()).is_err());
    let second = session.usage();
    assert_eq!(second.page_attempts, 2);
    assert!(second.parser_input_bytes >= 2 * first.parser_input_bytes);
    assert!(second.parser_operations >= 2 * first.parser_operations);
    assert!(session.trace_page(0, Default::default()).is_err());
    assert_eq!(session.usage(), second);
}

#[test]
fn every_execution_origin_and_reused_stream_invocation_remains_resolvable() {
    use tandem_pdf_engine::form_resources::{FormPhase, OperationOrigin};
    let raw = "BT /F 10 Tf (A) Tj ET";
    let result = {
        let bytes = fixture("/X Do /X Do", ROOT, &[form("/BBox [0 0 10 10]", raw)]);
        trace(&bytes)
    };
    assert_eq!(result.form_streams().len(), 1);
    assert_eq!(result.form_streams()[0].object_id(), id(7));
    assert_eq!(result.form_streams()[0].bytes(), raw.as_bytes());
    let invocations = result.form_invocations();
    assert_eq!(invocations.len(), 2);
    assert_eq!(invocations[0].stream_id(), invocations[1].stream_id());
    for (index, inv) in invocations.iter().enumerate() {
        assert_eq!(inv.id().index(), index);
        assert_eq!(inv.object_id(), id(7));
        assert_eq!(inv.parent(), None);
        assert_eq!(inv.caller_scope(), inv.effective_scope());
        assert_eq!(inv.execution_range(), 6 * index..6 * index + 6);
        assert_eq!(
            inv.call_origin(),
            &OperationOrigin::PageContent {
                span: 6 * index..6 * index + 5
            }
        );
        assert_eq!(
            result.operation_origin(6 * index),
            Some(&OperationOrigin::FormBoundary {
                invocation: inv.id(),
                phase: FormPhase::Enter
            })
        );
        assert_eq!(
            result.operation_origin(6 * index + 5),
            Some(&OperationOrigin::FormBoundary {
                invocation: inv.id(),
                phase: FormPhase::Leave
            })
        );
        for (offset, span) in [(1, 0..2), (2, 3..11), (3, 12..18), (4, 19..21)] {
            assert_eq!(
                result.operation_origin(6 * index + offset),
                Some(&OperationOrigin::FormContent {
                    invocation: inv.id(),
                    span
                })
            );
        }
        let code = &result.program().text().codes()[index];
        assert_eq!(code.operation_index(), 6 * index + 3);
        assert_eq!(
            &result.form_streams()[inv.stream_id().index()].bytes()[code.operation_span()],
            b"(A) Tj"
        );
        assert_eq!(result.fonts()[0].scope_id(), inv.effective_scope());
    }
    assert_eq!(result.operation_origin(12), None);
    assert_eq!(result.program().usage().operations, 12);
}

#[test]
fn nested_calls_record_local_callsite_and_distinct_preorder_origins() {
    use tandem_pdf_engine::form_resources::OperationOrigin;
    let result = trace(&fixture(
        "/X Do",
        ROOT,
        &[
            form(
                "/BBox [0 0 10 10] /Resources << /XObject << /Child 8 0 R >> >>",
                "0 g /Child Do 1 g",
            ),
            form("/BBox [0 0 10 10]", "0 0 1 1 re f"),
        ],
    ));
    let parent = &result.form_invocations()[0];
    let child = &result.form_invocations()[1];
    assert_eq!(child.parent(), Some(parent.id()));
    assert_eq!(
        child.call_origin(),
        &OperationOrigin::FormContent {
            invocation: parent.id(),
            span: 4..13
        }
    );
    assert!(parent.execution_range().start < child.execution_range().start);
    assert!(child.execution_range().end < parent.execution_range().end);
    assert_eq!(
        &result.form_streams()[parent.stream_id().index()].bytes()[4..13],
        b"/Child Do"
    );
    assert_eq!(
        result.resource_scopes()[child.effective_scope().index()].provider(),
        Some(id(7))
    );
}

#[test]
fn all_form_counter_partitions_have_exact_and_one_less_boundaries() {
    use tandem_pdf_engine::form_resources::{FormLimits, FormUsage};
    type Getter = fn(FormUsage) -> usize;
    type Setter = fn(&mut FormLimits, usize);
    let cases: &[(&str, Getter, Setter)] = &[
        (
            "invocations",
            |u| u.invocations,
            |l, n| l.max_invocations = n,
        ),
        (
            "unique_streams",
            |u| u.unique_streams,
            |l, n| l.max_unique_streams = n,
        ),
        (
            "resource_scopes",
            |u| u.resource_scopes,
            |l, n| l.max_resource_scopes = n,
        ),
        (
            "resource_bindings",
            |u| u.resource_bindings,
            |l, n| l.max_resource_bindings = n,
        ),
        ("name_bytes", |u| u.name_bytes, |l, n| l.max_name_bytes = n),
        (
            "metadata_items",
            |u| u.metadata_items,
            |l, n| l.max_metadata_items = n,
        ),
        (
            "reference_hops",
            |u| u.reference_hops,
            |l, n| l.max_reference_hops = n,
        ),
        (
            "lookup_work",
            |u| u.lookup_work,
            |l, n| l.max_lookup_work = n,
        ),
        (
            "origin_records",
            |u| u.origin_records,
            |l, n| l.max_origin_records = n,
        ),
        (
            "allocated_bytes",
            |u| u.allocated_bytes,
            |l, n| l.max_allocated_bytes = n,
        ),
        (
            "diagnostic_allocated_bytes",
            |u| u.diagnostic_allocated_bytes,
            |l, n| l.max_diagnostic_allocated_bytes = n,
        ),
    ];
    let bytes = fixture(
        "/X Do /X Do",
        ROOT,
        &[form(
            "/BBox [0 0 10 10] /Resources << /Font << /F 5 0 R >> >>",
            "BT /F 10 Tf (A) Tj ET",
        )],
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut baseline = PageProgramSession::new(&doc, Default::default()).unwrap();
    baseline.trace_page(0, Default::default()).unwrap();
    let usage = baseline.form_usage();
    for &(name, get, set) in cases {
        let amount = get(usage);
        assert!(amount > 0, "counter {name}");
        for delta in [0, 1] {
            let mut limits = PageProgramSessionLimits::default();
            set(&mut limits.forms, amount - delta);
            let mut session = PageProgramSession::new(&doc, limits).unwrap();
            let result = session.trace_page(0, Default::default());
            assert_eq!(
                result.is_ok(),
                delta == 0,
                "{name} limit {}: {result:?}",
                amount - delta
            );
            assert!(get(session.form_usage()) <= amount - delta, "{name}");
        }
    }
}

#[test]
fn invocations_are_charged_once_but_replay_and_failures_remain_cumulative() {
    let bytes = fixture(
        "/X Do /X Do",
        ROOT,
        &[form("/BBox [0 0 10 10]", "0 0 1 1 re f")],
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut limits = PageProgramSessionLimits::default();
    limits.forms.max_invocations = 4;
    let mut session = PageProgramSession::new(&doc, limits).unwrap();
    session.trace_page(0, Default::default()).unwrap();
    let first = session.form_usage();
    let parser = session.usage().parser_operations;
    assert_eq!(first.invocations, 2);
    assert_eq!(first.unique_streams, 1);
    session.trace_page(0, Default::default()).unwrap();
    let second = session.form_usage();
    assert_eq!(second.invocations, 4);
    assert_eq!(second.unique_streams, 1);
    assert!(second.lookup_work > first.lookup_work);
    assert!(second.allocated_bytes > first.allocated_bytes);
    assert_eq!(session.usage().parser_operations, 2 * parser);
    assert!(session.trace_page(0, Default::default()).is_err());
    assert_eq!(session.form_usage().invocations, 4);
    // A failure before resolution still consumes the logical Do attempt.
    let bytes = fixture("/Absent Do", ROOT, &[]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut session = PageProgramSession::new(&doc, limits).unwrap();
    for n in 1..=4 {
        assert!(session.trace_page(0, Default::default()).is_err());
        assert_eq!(session.form_usage().invocations, n);
    }
    assert!(session.trace_page(0, Default::default()).is_err());
    assert_eq!(session.form_usage().invocations, 4);
}

#[test]
fn no_do_pages_need_no_diagnostic_storage_and_keep_operation_numbers() {
    use tandem_pdf_engine::form_resources::OperationOrigin;
    let bytes = fixture("BT /F 10 Tf (A) Tj ET", ROOT, &[]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut limits = PageProgramSessionLimits::default();
    limits.forms.max_diagnostic_allocated_bytes = 0;
    let mut session = PageProgramSession::new(&doc, limits).unwrap();
    let result = session.trace_page(0, Default::default()).unwrap();
    assert_eq!(session.form_usage().diagnostic_allocated_bytes, 0);
    assert_eq!(result.program().text().codes()[0].operation_index(), 2);
    assert_eq!(
        result.operation_origin(2),
        Some(&OperationOrigin::PageContent { span: 12..18 })
    );
    let bytes = fixture("/X Do", ROOT, &[form("/BBox [0 0 1 1]", "")]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    assert!(
        PageProgramSession::new(&doc, limits)
            .unwrap()
            .trace_page(0, Default::default())
            .is_err()
    );
}

#[test]
fn direct_same_name_fonts_from_distinct_providers_never_alias() {
    let bytes = fixture(
        "/A Do /B Do /A Do",
        "/XObject << /A 7 0 R /B 8 0 R >>",
        &[
            form(
                &format!(
                    "/BBox [0 0 10 10] /Resources << /Font << /F {} >> >>",
                    font(500, "A")
                ),
                "BT /F 10 Tf (A) Tj ET",
            ),
            form(
                &format!(
                    "/BBox [0 0 10 10] /Resources << /Font << /F {} >> >>",
                    font(700, "B")
                ),
                "BT /F 10 Tf (A) Tj ET",
            ),
        ],
    );
    let result = trace(&bytes);
    assert_eq!(result.program().text().text(), "ABA");
    assert_eq!(result.fonts().len(), 2);
    assert_ne!(result.fonts()[0].identity(), result.fonts()[1].identity());
    assert_ne!(result.fonts()[0].scope_id(), result.fonts()[1].scope_id());
}

#[test]
fn clipped_away_and_unshown_font_selections_still_require_valid_binding() {
    for raw in ["BT /Bad 10 Tf ET", "BT /Bad 10 Tf (A) Tj ET"] {
        refused(&fixture(
            "/X Do",
            ROOT,
            &[form(
                "/BBox [200 200 210 210] /Resources << /Font << /Bad << /Subtype /Type0 >> >> >>",
                raw,
            )],
        ));
    }
}

#[test]
fn nested_failure_owns_original_object_and_callsite_chain_after_inputs_drop() {
    let error = {
        let bytes = fixture(
            "0 g /X Do",
            ROOT,
            &[
                form(
                    "/BBox [0 0 10 10] /Resources << /XObject << /Child 8 0 R >> >>",
                    "0 g /Child Do",
                ),
                form("/BBox [0 0 10 10]", "0 g ["),
            ],
        );
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        PageProgramSession::new(&doc, Default::default())
            .unwrap()
            .trace_page(0, Default::default())
            .unwrap_err()
    };
    assert_eq!(error.diagnostic.len(), 2);
    assert_eq!(error.diagnostic[1].object, id(7));
    assert_eq!(error.diagnostic[1].caller_object, id(3));
    assert_eq!(error.diagnostic[1].call_span, 4..9);
    assert_eq!(error.diagnostic[0].object, id(8));
    assert_eq!(error.diagnostic[0].caller_object, id(7));
    assert_eq!(error.diagnostic[0].call_span, 4..13);
    assert_eq!(error.source_object, Some(id(8)));
    assert!(error.diagnostic[0].span.start <= 5);
    assert!(error.diagnostic[0].span.end <= 5);
}

#[test]
fn per_page_depth_and_replay_caps_intersect_session_caps() {
    use tandem_pdf_engine::page_program::PageProgramLimits;
    let bytes = fixture(
        "/X Do /X Do",
        ROOT,
        &[
            form(
                "/BBox [0 0 10 10] /Resources << /XObject << /Child 8 0 R >> >>",
                "/Child Do",
            ),
            form("/BBox [0 0 10 10]", ""),
        ],
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    for (session_depth, page_depth, success) in
        [(2, 2, true), (1, 2, false), (2, 1, false), (0, 2, false)]
    {
        let mut limits = PageProgramSessionLimits::default();
        limits.forms.max_depth = session_depth;
        let mut page = PageProgramLimits::default();
        page.forms.max_depth = page_depth;
        let mut session = PageProgramSession::new(&doc, limits).unwrap();
        assert_eq!(session.trace_page(0, page).is_ok(), success);
    }
    for (session_invocations, page_invocations, success) in
        [(4, 4, true), (3, 4, false), (4, 3, false)]
    {
        let mut limits = PageProgramSessionLimits::default();
        limits.forms.max_invocations = session_invocations;
        let mut page = PageProgramLimits::default();
        page.forms.max_invocations = page_invocations;
        let mut session = PageProgramSession::new(&doc, limits).unwrap();
        assert_eq!(session.trace_page(0, page).is_ok(), success);
        assert!(session.form_usage().invocations <= session_invocations.min(page_invocations));
    }
}

#[test]
fn parser_resume_keeps_root_consumption_on_both_sides_of_child_calls() {
    use tandem_pdf_engine::page_program::PageProgramLimits;
    let bytes = fixture(
        "0 g /X Do 0 g /X Do 0 g",
        ROOT,
        &[form("/BBox [0 0 10 10]", "")],
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    for (operation_cap, success) in [(5, true), (4, false)] {
        let mut page = PageProgramLimits::default();
        page.parser.max_operations = operation_cap;
        let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
        assert_eq!(
            session.trace_page(0, page).is_ok(),
            success,
            "root operation cap {operation_cap}"
        );
    }
}

#[test]
fn structural_depth_clamp_and_deep_error_chain_stay_within_reserved_64_records() {
    use tandem_pdf_engine::{
        content::ContentLimits, lexer::LexLimits, page_program::PageProgramLimits,
    };
    let chain = |depth: usize, bottom: &str| {
        (0..depth)
            .map(|i| {
                if i + 1 == depth {
                    form("/BBox [0 0 10 10]", bottom)
                } else {
                    form(
                        &format!(
                            "/BBox [0 0 10 10] /Resources << /XObject << /Next {} 0 R >> >>",
                            8 + i
                        ),
                        "/Next Do",
                    )
                }
            })
            .collect::<Vec<_>>()
    };
    let mut limits = PageProgramSessionLimits::default();
    limits.forms.max_depth = usize::MAX;
    let mut page = PageProgramLimits::default();
    page.forms.max_depth = usize::MAX;
    page.parser = ContentLimits {
        max_operations: 16,
        max_nodes: 64,
        max_owned_bytes: 128,
        lex: LexLimits {
            max_token_bytes: 128,
            max_tokens: 64,
        },
        ..Default::default()
    };
    for (depth, bottom, success) in [(64, "", true), (64, "[", false), (65, "", false)] {
        let bytes = fixture("/X Do", ROOT, &chain(depth, bottom));
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let mut session = PageProgramSession::new(&doc, limits).unwrap();
        let result = session.trace_page(0, page);
        assert_eq!(
            result.is_ok(),
            success,
            "depth={depth},bottom={bottom:?},result={result:?}"
        );
        if let Err(error) = result {
            assert!(error.diagnostic.len() <= 64);
            if depth == 64 {
                assert_eq!(error.diagnostic.len(), 64);
                assert_eq!(error.diagnostic[63].caller_object, id(3));
                assert_eq!(error.diagnostic[0].object, id(70));
                assert_eq!(error.diagnostic[0].caller_object, id(69));
                assert_eq!(error.diagnostic[0].call_span, 0..8);
                assert_eq!(error.source_object, Some(id(70)));
            }
        }
    }
}

#[test]
fn raster_uses_true_sheared_bbox_sides_and_restores_parent_for_later_paint() {
    use tandem_pdf_engine::{
        geometry::Affine,
        page_glyph::PageGlyphSession,
        page_raster::{PageRasterSession, RasterTarget},
    };
    let bytes = fixture(
        "/X Do 1 0 0 rg 0 3 1 1 re f",
        ROOT,
        &[form(
            "/BBox [0 0 2 2] /Matrix [1 0 1 1 0 0]",
            "-10 -10 20 20 re f",
        )],
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let glyphs = PageGlyphSession::new(&doc, Default::default())
        .unwrap()
        .trace_page(0, Default::default())
        .unwrap();
    let image = PageRasterSession::new(Default::default())
        .render(
            &glyphs,
            RasterTarget {
                width: 4,
                height: 4,
                page_to_device: Affine::from_pdf([1., 0., 0., 1., 0., -96.]).unwrap(),
            },
            Default::default(),
        )
        .unwrap();
    // Independent analytic inverse x=u+v,y=v with 16 exact dyadic samples/pixel.
    // Source BBox is 0<=u<2,0<=v<2; its page AABB would incorrectly fill 4x2.
    let mut expected = Vec::new();
    for y in 0..4 {
        for x in 0..4 {
            let mut sums = [0u32; 3];
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = f64::from(x) + (f64::from(sx) + 0.5) / 4.;
                    let py = 4. - f64::from(y) - (f64::from(sy) + 0.5) / 4.;
                    let color = if px < 1. && py >= 3. {
                        [255, 0, 0]
                    } else if (0. ..2.).contains(&py) && (0. ..2.).contains(&(px - py)) {
                        [0, 0, 0]
                    } else {
                        [255, 255, 255]
                    };
                    for c in 0..3 {
                        sums[c] += color[c];
                    }
                }
            }
            expected.extend(sums.map(|sum| ((sum + 8) / 16) as u8));
            expected.push(255);
        }
    }
    assert_eq!(image.pixels(), expected);
}

#[test]
fn later_page_owned_ids_resolve_after_unrelated_session_cache_entries() {
    let (first, second) = {
        let bytes=pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Resources << /XObject << /X 7 0 R >> >> /Contents 9 0 R >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Resources << /XObject << /X 8 0 R >> >> /Contents 10 0 R >>".into(),
            font(500,"A"),font(700,"B"),
            form("/BBox [0 0 10 10] /Resources << /Font << /F 5 0 R >> >>","BT /F 10 Tf (A) Tj ET"),
            form("/BBox [1 1 9 9] /Resources << /Font << /F 6 0 R >> >>","BT /F 20 Tf (A) Tj ET"),
            stream("","/X Do"),stream("","/X Do")]);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
        let first = session.trace_page(0, Default::default()).unwrap();
        let second = session.trace_page(1, Default::default()).unwrap();
        (first, second)
    };
    assert_eq!(first.program().text().text(), "A");
    assert_eq!(second.program().text().text(), "B");
    for (result, object, font_object) in [(&first, 7, 5), (&second, 8, 6)] {
        let inv = &result.form_invocations()[0];
        assert_eq!(inv.object_id(), id(object));
        assert_eq!(
            result.form_streams()[inv.stream_id().index()].object_id(),
            id(object)
        );
        assert_eq!(
            result.resource_scopes()[inv.effective_scope().index()].provider(),
            Some(id(object))
        );
        assert_eq!(result.fonts()[0].scope_id(), inv.effective_scope());
        assert_eq!(
            result.fonts()[0].identity(),
            FontIdentity::Indirect(id(font_object))
        );
    }
}

#[test]
fn cached_decoded_form_obeys_a_later_stricter_per_page_stream_cap() {
    use tandem_pdf_engine::page_program::PageProgramLimits;
    let bytes = fixture("/X Do", ROOT, &[form("/BBox [0 0 10 10]", "0 g")]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    let mut page = PageProgramLimits::default();
    page.forms.max_stream_decoded_bytes = 3;
    session.trace_page(0, page).unwrap();
    let first = session.usage();
    page.forms.max_stream_decoded_bytes = 2;
    assert!(session.trace_page(0, page).is_err());
    assert_eq!(session.form_usage().unique_streams, 1);
    assert_eq!(session.usage().encoded_bytes - first.encoded_bytes, 5); // Only root content reloaded.
}

#[test]
fn first_form_decode_and_boundary_failures_preserve_original_root_callsite() {
    use tandem_pdf_engine::page_program::PageProgramLimits;
    let bytes = fixture(
        "0 g /X Do",
        ROOT,
        &[form("/BBox [0 0 10 10] /Filter /ASCIIHexDecode", "GG>")],
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    for _ in 0..2 {
        let failure = session.trace_page(0, Default::default()).unwrap_err();
        assert_eq!(failure.source_object, Some(id(3)));
        assert_eq!(failure.span, Some(4..9));
        assert_eq!(failure.operation_index, Some(1));
    }
    let bytes = fixture("0 g /X Do", ROOT, &[form("/BBox [0 0 10 10]", "")]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut page = PageProgramLimits::default();
    page.program.path.max_clips = 0;
    let failure = PageProgramSession::new(&doc, Default::default())
        .unwrap()
        .trace_page(0, page)
        .unwrap_err();
    // It is acceptable to report the synthetic child boundary with its owned
    // invocation callsite chain, or the original caller Do span directly.
    let caller_report = failure.source_object == Some(id(3)) && failure.span == Some(4..9);
    let boundary_report = failure.source_object == Some(id(7))
        && failure
            .diagnostic
            .iter()
            .any(|f| f.object == id(7) && f.caller_object == id(3) && f.call_span == (4..9));
    assert!(
        caller_report || boundary_report,
        "lost Form entry callsite: {failure:?}"
    );
    assert_eq!(failure.operation_index, Some(1));
}

#[test]
fn decoded_bytes_and_reference_aliases_share_stream_identity_but_not_execution() {
    let bytes = fixture(
        "/X Do /Alias Do",
        "/XObject << /X 7 0 R /Alias 8 0 R >>",
        &[
            form("/BBox [0 0 10 10] /Filter /ASCIIHexDecode", "302067>"),
            "7 0 R".into(),
        ],
    );
    let result = trace(&bytes);
    assert_eq!(result.form_streams().len(), 1);
    assert_eq!(result.form_streams()[0].bytes(), b"0 g");
    assert_eq!(result.form_invocations().len(), 2);
    assert_eq!(result.form_invocations()[0].object_id(), id(7));
    assert_eq!(result.form_invocations()[1].object_id(), id(7));
    let bytes = fixture(
        "/X Do",
        ROOT,
        &[
            form(
                "/BBox [0 0 10 10] /Resources << /XObject << /Again 8 0 R >> >>",
                "/Again Do",
            ),
            "9 0 R".into(),
            "7 0 R".into(),
        ],
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    let error = session.trace_page(0, Default::default()).unwrap_err();
    assert_eq!(error.source_object, Some(id(7)));
    assert_eq!(session.form_usage().unique_streams, 1);
    assert_eq!(session.form_usage().invocations, 2);
}
