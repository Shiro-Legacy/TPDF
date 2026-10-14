// SPDX-License-Identifier: AGPL-3.0-or-later
use std::sync::Arc;
use tandem_pdf_engine::{
    document::Document,
    object::ObjectId,
    page_program::{
        OperationOrigin, PageProgramErrorKind, PageProgramLimits, PageProgramSession,
        PageProgramSessionLimits,
    },
    program::ProgramEventKind,
};
fn id(number: u32) -> ObjectId {
    ObjectId::new(number, 0).unwrap()
}
fn stream(meta: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< /Length {} {meta} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}
fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        out.extend_from_slice(object);
        out.extend_from_slice(b"\nendobj\n");
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
fn image(extra: &str, samples: &[u8]) -> Vec<u8> {
    stream(
        &format!(
            "/Type /XObject /Subtype /Image /Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB {extra}"
        ),
        samples,
    )
}
fn basic(root: &str, extra: &str) -> Vec<u8> {
    basic_with_padding(root, extra, 0)
}
fn basic_with_padding(root: &str, extra: &str, padding: usize) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Resources << /XObject << /Im 5 0 R /Alias 5 0 R >> /Font << /F 6 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream("", root.as_bytes()),
        image(extra, &[255,0,0,0,255,0]),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /FirstChar 65 /LastChar 65 /Widths [500] >>".to_vec(),
    ];
    objects.extend((0..padding).map(|_| b"null".to_vec()));
    pdf(&objects)
}
#[test]
fn image_events_have_original_indices_and_owned_unique_assets() {
    let bytes = basic("/Im Do /Alias Do", "");
    let page = {
        let doc = Document::open(&bytes, Default::default()).unwrap();
        let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
        let page = session.trace_page(0, Default::default()).unwrap();
        assert_eq!(session.image_usage().attempts, 2);
        assert_eq!(session.image_usage().unique_images, 1);
        assert_eq!(session.image_usage().decoded_work_bytes, 6);
        assert_eq!(page.program().usage().operations, 2);
        assert!(page.form_invocations().is_empty());
        assert!(page.form_streams().is_empty());
        page
    };
    assert_eq!(page.images().len(), 1);
    assert_eq!(page.images()[0].object_id(), id(5));
    assert_eq!(page.images()[0].pixel(0, 1), Some([0, 255, 0]));
    assert_eq!(page.program().images().len(), 2);
    for (index, paint) in page.program().images().iter().enumerate() {
        assert_eq!(paint.operation_index(), index);
        assert_eq!(paint.image_id(), 0);
        assert_eq!(
            page.program().events()[index].kind(),
            ProgramEventKind::Image(index)
        );
    }
    assert_eq!(
        page.operation_origin(0),
        Some(&OperationOrigin::PageContent { span: 0..6 })
    );
    assert_eq!(
        page.operation_origin(1),
        Some(&OperationOrigin::PageContent { span: 7..16 })
    );
}
#[test]
fn path_text_image_stroke_share_global_order() {
    let bytes = basic("0 0 2 2 re f BT /F 10 Tf (A) Tj ET /Im Do 0 0 3 3 re S", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    let page = session.trace_page(0, Default::default()).unwrap();
    assert_eq!(
        page.program()
            .events()
            .iter()
            .map(|e| e.operation_index())
            .collect::<Vec<_>>(),
        [1, 4, 5, 6, 8]
    );
    assert!(matches!(
        page.program().events()[0].kind(),
        ProgramEventKind::Path(_)
    ));
    assert!(matches!(
        page.program().events()[1].kind(),
        ProgramEventKind::Text { .. }
    ));
    assert_eq!(
        page.program().events()[3].kind(),
        ProgramEventKind::Image(0)
    );
    assert!(matches!(
        page.program().events()[4].kind(),
        ProgramEventKind::Path(_)
    ));
}
fn nested(extra: &str) -> Vec<u8> {
    pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 9 0 R] /Count 2 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 20] /Resources << /XObject << /Im 5 0 R /Outer 6 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream("", b"/Im Do /Outer Do /Im Do"),
        image("", &[255,0,0,0,255,0]),
        stream("/Subtype /Form /BBox [0 0 10 10] /Matrix [2 0 0 3 4 5] /Resources << /XObject << /Inner 7 0 R >> >>", b"/Inner Do"),
        stream("/Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /Im 8 0 R /Same 5 0 R >> >>", b"/Im Do /Same Do"),
        image(extra, &[0,0,255,255,255,0]),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 20] /Resources << /XObject << /Im 8 0 R >> >> /Contents 10 0 R >>".to_vec(),
        stream("", b"/Im Do"),
    ])
}
#[test]
fn nested_scopes_rebind_names_and_reuse_object_assets_across_pages() {
    let bytes = nested("");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    let page = session.trace_page(0, Default::default()).unwrap();
    assert_eq!(
        page.images()
            .iter()
            .map(|a| a.object_id())
            .collect::<Vec<_>>(),
        [id(5), id(8)]
    );
    assert_eq!(
        page.program()
            .images()
            .iter()
            .map(|a| a.image_id())
            .collect::<Vec<_>>(),
        [0, 1, 0, 0]
    );
    assert_eq!(
        page.program()
            .images()
            .iter()
            .map(|a| a.operation_index())
            .collect::<Vec<_>>(),
        [0, 3, 4, 7]
    );
    assert!(
        matches!(page.operation_origin(3), Some(OperationOrigin::FormContent { invocation, span }) if invocation.index()==1 && *span==(0..6))
    );
    assert_eq!(
        page.program().images()[1].ctm().coefficients(),
        [2., 0., 0., -3., 4., 15.]
    );
    assert!(page.program().images()[1].clip().is_some());
    assert_eq!(
        page.program().images()[0].ctm(),
        page.program().images()[3].ctm()
    );
    let before = session.image_usage();
    let other = session.trace_page(1, Default::default()).unwrap();
    assert_eq!(other.images().len(), 1);
    assert!(Arc::ptr_eq(&page.images()[1], &other.images()[0]));
    assert_eq!(other.program().images()[0].image_id(), 0);
    assert_eq!(
        session.image_usage().decoded_work_bytes,
        before.decoded_work_bytes
    );
    assert_eq!(session.image_usage().attempts, before.attempts + 1);
}
#[test]
fn late_nested_image_refusal_keeps_image_and_form_call_provenance() {
    let bytes = nested("/SMask 5 0 R");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    let error = session.trace_page(0, Default::default()).unwrap_err();
    assert_eq!(error.operation_index, Some(3));
    assert_eq!(error.source_object, Some(id(7)));
    assert_eq!(error.span, Some(0..6));
    assert!(
        matches!(&error.kind, PageProgramErrorKind::Image(e) if e.object==id(5) && e.base_image==Some(id(8)) && e.field=="ColorSpace")
    );
    assert_eq!(
        error
            .diagnostic
            .iter()
            .map(|d| (d.object, d.caller_object))
            .collect::<Vec<_>>(),
        [(id(7), id(6)), (id(6), id(3))]
    );
    assert_eq!(session.usage().program_operations, 0);
    assert_eq!(session.image_usage().attempts, 3);
    assert_eq!(session.image_usage().unique_images, 2);
}
#[test]
fn late_suffix_refusal_never_starts_execution_but_keeps_decoded_cache() {
    let bytes = basic("/Im Do /Unknown sh", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    assert!(session.trace_page(0, Default::default()).is_err());
    assert_eq!(session.usage().program_operations, 0);
    let before = session.image_usage();
    assert!(session.trace_page(0, Default::default()).is_err());
    assert_eq!(
        session.image_usage().decoded_work_bytes,
        before.decoded_work_bytes
    );
    assert_eq!(session.image_usage().attempts, 2);
}
#[test]
fn image_do_refuses_active_text_path_or_clip_before_decode() {
    for root in ["BT /Im Do ET", "0 0 m /Im Do", "0 0 2 2 re W /Im Do"] {
        let bytes = basic(root, "");
        let doc = Document::open(&bytes, Default::default()).unwrap();
        let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
        assert!(matches!(
            session.trace_page(0, Default::default()).unwrap_err().kind,
            PageProgramErrorKind::State(_)
        ));
        assert_eq!(session.image_usage().attempts, 0);
    }
}
#[test]
fn fully_clipped_image_is_still_validated() {
    let bytes = basic("0 0 0 0 re W n /Im Do", "/Interpolate true");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    assert!(matches!(
        session.trace_page(0, Default::default()).unwrap_err().kind,
        PageProgramErrorKind::Image(_)
    ));
    assert_eq!(session.image_usage().attempts, 1);
}
#[test]
fn page_attempt_limits_and_session_image_limits_intersect_with_cache_reuse() {
    let bytes = basic("/Im Do /Im Do", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut limits = PageProgramSessionLimits::default();
    limits.images.max_attempts = 3;
    let mut session = PageProgramSession::new(&doc, limits).unwrap();
    let mut page_limits = PageProgramLimits::default();
    page_limits.images.max_attempts = 1;
    assert!(session.trace_page(0, page_limits).is_err());
    assert_eq!(session.image_usage().attempts, 1);
    let page = session.trace_page(0, Default::default()).unwrap();
    assert_eq!(page.program().images().len(), 2);
    assert_eq!(session.image_usage().attempts, 3);
    assert_eq!(session.image_usage().unique_images, 1);
    assert_eq!(session.image_usage().decoded_work_bytes, 6);
    assert!(session.trace_page(0, Default::default()).is_err());
    assert_eq!(session.image_usage().attempts, 3);
}
#[test]
fn cached_image_rechecks_tighter_shape_caps_and_uses_no_form_depth() {
    let bytes = basic("/Im Do", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    let mut limits = PageProgramLimits::default();
    limits.forms.max_depth = 0;
    session.trace_page(0, limits).unwrap();
    limits.images.max_dimension = 1;
    assert!(matches!(
        session.trace_page(0, limits).unwrap_err().kind,
        PageProgramErrorKind::Image(_)
    ));
    assert_eq!(session.image_usage().attempts, 2);
    assert_eq!(session.image_usage().decoded_work_bytes, 6);
}
#[test]
fn original_do_count_bounds_image_replay_and_origin_storage() {
    let bytes = basic("/Im Do /Im Do", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    for maximum in [1, 2] {
        let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
        let mut limits = PageProgramLimits::default();
        limits.program.max_operations = maximum;
        limits.forms.max_origin_records = maximum;
        let result = session.trace_page(0, limits);
        assert_eq!(result.is_ok(), maximum == 2);
        if let Ok(page) = result {
            assert_eq!(page.program().usage().operations, 2);
        }
    }
}

#[test]
fn frozen_image_handles_names_and_traversal_have_exact_budget_boundaries() {
    use tandem_pdf_engine::form_resources::{FormLimits, FormUsage};
    type Case = (
        &'static str,
        fn(FormUsage) -> usize,
        fn(&mut FormLimits, usize),
    );
    let cases: &[Case] = &[
        (
            "invocations",
            |u| u.invocations,
            |l, n| l.max_invocations = n,
        ),
        (
            "unique streams",
            |u| u.unique_streams,
            |l, n| l.max_unique_streams = n,
        ),
        (
            "scopes",
            |u| u.resource_scopes,
            |l, n| l.max_resource_scopes = n,
        ),
        (
            "bindings",
            |u| u.resource_bindings,
            |l, n| l.max_resource_bindings = n,
        ),
        ("names", |u| u.name_bytes, |l, n| l.max_name_bytes = n),
        (
            "metadata",
            |u| u.metadata_items,
            |l, n| l.max_metadata_items = n,
        ),
        (
            "references",
            |u| u.reference_hops,
            |l, n| l.max_reference_hops = n,
        ),
        (
            "lookup work",
            |u| u.lookup_work,
            |l, n| l.max_lookup_work = n,
        ),
        (
            "origins",
            |u| u.origin_records,
            |l, n| l.max_origin_records = n,
        ),
        (
            "allocations",
            |u| u.allocated_bytes,
            |l, n| l.max_allocated_bytes = n,
        ),
        (
            "diagnostics",
            |u| u.diagnostic_allocated_bytes,
            |l, n| l.max_diagnostic_allocated_bytes = n,
        ),
    ];
    let bytes = nested("");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut baseline = PageProgramSession::new(&doc, Default::default()).unwrap();
    baseline.trace_page(0, Default::default()).unwrap();
    let usage = baseline.form_usage();
    for &(name, get, set) in cases {
        let required = get(usage);
        assert!(required > 0, "{name}");
        for delta in [0, 1] {
            for per_page in [false, true] {
                let mut limits = PageProgramSessionLimits::default();
                let mut page_limits = PageProgramLimits::default();
                set(
                    if per_page {
                        &mut page_limits.forms
                    } else {
                        &mut limits.forms
                    },
                    required - delta,
                );
                let mut session = PageProgramSession::new(&doc, limits).unwrap();
                let result = session.trace_page(0, page_limits);
                assert_eq!(
                    result.is_ok(),
                    delta == 0,
                    "{name} page={per_page}: {result:?}"
                );
                assert!(get(session.form_usage()) <= required - delta, "{name}");
            }
        }
    }
}
#[test]
fn resource_less_cached_form_resolves_images_in_each_callers_scope() {
    let bytes = pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 20] /Resources << /XObject << /Left 6 0 R /Right 9 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream("", b"/Left Do /Right Do"),
        image("", &[255,0,0,0,255,0]),
        stream("/Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /Shared 7 0 R /Im 5 0 R >> >>", b"/Shared Do"),
        stream("/Subtype /Form /BBox [0 0 1 1]", b"/Im Do"),
        image("", &[0,0,255,255,255,0]),
        stream("/Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /Shared 7 0 R /Im 8 0 R >> >>", b"/Shared Do"),
    ]);
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    let page = session.trace_page(0, Default::default()).unwrap();
    assert_eq!(
        page.images()
            .iter()
            .map(|a| a.object_id())
            .collect::<Vec<_>>(),
        [id(5), id(8)]
    );
    assert_eq!(
        page.program()
            .images()
            .iter()
            .map(|a| a.image_id())
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(page.form_streams().len(), 3);
    assert_eq!(page.form_invocations().len(), 4);
    assert_eq!(
        page.form_invocations()[1].object_id(),
        page.form_invocations()[3].object_id()
    );
    assert_ne!(
        page.form_invocations()[1].effective_scope(),
        page.form_invocations()[3].effective_scope()
    );
}
#[test]
fn indirect_aliases_authenticate_the_actual_image_stream_object() {
    let bytes = pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 20] /Resources << /XObject << /Im 6 0 R /Direct 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream("", b"/Im Do /Direct Do"),
        image("", &[255,0,0,0,255,0]),
        b"7 0 R".to_vec(),
        b"5 0 R".to_vec(),
    ]);
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    let page = session.trace_page(0, Default::default()).unwrap();
    assert_eq!(page.images().len(), 1);
    assert_eq!(page.images()[0].object_id(), id(5));
    assert_eq!(page.program().images().len(), 2);
    assert_eq!(session.image_usage().unique_images, 1);
}

#[test]
fn scoped_document_index_work_grows_with_unused_objects_before_lookup() {
    let usage = |root: &str, padding| {
        let bytes = basic_with_padding(root, "", padding);
        let doc = Document::open(&bytes, Default::default()).unwrap();
        let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
        session.trace_page(0, Default::default()).unwrap();
        session.form_usage().lookup_work
    };
    let padding = 64;
    // Empty root traverses one page object; image dispatch also resolves its stream.
    assert_eq!(usage("", padding) - usage("", 0), 2 * padding);
    assert_eq!(usage("/Im Do", padding) - usage("/Im Do", 0), 4 * padding);
    let bytes = basic("/Im Do", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut limits = PageProgramSessionLimits::default();
    limits.forms.max_lookup_work = 1;
    let mut session = PageProgramSession::new(&doc, limits).unwrap();
    let error = session.trace_page(0, Default::default()).unwrap_err();
    assert!(
        matches!(error.kind, PageProgramErrorKind::Form(e) if matches!(e.kind,tandem_pdf_engine::form_resources::FormErrorKind::Limit("lookup work")))
    );
    assert_eq!(session.form_usage().lookup_work, 1);
    assert_eq!(session.form_usage().resource_scopes, 0);
    assert_eq!(session.image_usage().attempts, 0);
}
#[test]
fn image_error_box_is_reserved_before_loader_failure_and_stays_charged() {
    let bytes = basic("/Im Do", "/Interpolate true");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut baseline = PageProgramSession::new(&doc, Default::default()).unwrap();
    assert!(matches!(
        baseline.trace_page(0, Default::default()).unwrap_err().kind,
        PageProgramErrorKind::Image(_)
    ));
    let required = baseline.form_usage().allocated_bytes;
    for delta in [0, 1] {
        let mut limits = PageProgramSessionLimits::default();
        limits.forms.max_allocated_bytes = required - delta;
        let mut session = PageProgramSession::new(&doc, limits).unwrap();
        let error = session.trace_page(0, Default::default()).unwrap_err();
        assert_eq!(error.source_object, Some(id(3)));
        assert_eq!(error.operation_index, Some(0));
        assert_eq!(error.span, Some(0..6));
        if delta == 0 {
            assert!(matches!(error.kind, PageProgramErrorKind::Image(_)));
            assert_eq!(session.form_usage().allocated_bytes, required);
            assert_eq!(session.image_usage().attempts, 1);
        } else {
            assert!(matches!(
                error.kind,
                PageProgramErrorKind::LimitExceeded("image diagnostic allocation")
            ));
            assert_eq!(
                session.form_usage().allocated_bytes,
                required - std::mem::size_of::<tandem_pdf_engine::image_resources::ImageError>()
            );
            assert_eq!(session.image_usage().attempts, 0);
        }
        assert_eq!(session.usage().program_operations, 0);
    }
}
