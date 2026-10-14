// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::{
    document::{Document, DocumentLimits},
    object::ObjectId,
    page_program::{PageProgramLimits, PageProgramSession, PageProgramSessionLimits},
};
fn stream(meta: &str, bytes: &[u8]) -> Vec<u8> {
    let mut out = format!("<< /Length {} {meta} >>\nstream\n", bytes.len()).into_bytes();
    out.extend_from_slice(bytes);
    out.extend_from_slice(b"\nendstream");
    out
}
fn fixture(root: &str, form: &[u8], meta: &str) -> Vec<u8> {
    let objects=[b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Resources << /XObject << /Fm 5 0 R >> >> /Contents 4 0 R >>".to_vec(),stream("",root.as_bytes()),stream(&format!("/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources <<>> {meta}"),form)];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(obj);
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
#[test]
fn compressed_predictor_form_is_decoded_once_but_replayed_on_every_invocation() {
    let body = b"0 0 3 3 re f";
    let mut predicted = vec![0];
    predicted.extend_from_slice(body);
    let encoded = miniz_oxide::deflate::compress_to_vec_zlib(&predicted, 6);
    let root = "/Fm Do /Fm Do";
    let bytes = fixture(
        root,
        &encoded,
        &format!(
            "/Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors 1 /BitsPerComponent 8 /Columns {} >>",
            body.len()
        ),
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    let page = session.trace_page(0, Default::default()).unwrap();
    assert_eq!(page.form_streams().len(), 1);
    assert_eq!(page.form_streams()[0].bytes(), body);
    assert_eq!(page.form_invocations().len(), 2);
    assert_eq!(session.form_usage().invocations, 2);
    assert_eq!(
        session.usage().decoded_work_bytes,
        root.len() + predicted.len() + body.len()
    );
    let before = session.usage();
    let form_before = session.form_usage();
    let second = session.trace_page(0, Default::default()).unwrap();
    assert_eq!(second.form_invocations().len(), 2);
    assert_eq!(
        session.usage().decoded_work_bytes - before.decoded_work_bytes,
        root.len()
    );
    assert_eq!(
        session.form_usage().unique_streams,
        form_before.unique_streams
    );
    assert_eq!(session.form_usage().invocations, 4);
}
#[test]
fn cached_form_still_obeys_a_later_tighter_stream_length_cap() {
    let body = b"0 0 3 3 re f";
    let bytes = fixture("/Fm Do", body, "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut session = PageProgramSession::new(&doc, Default::default()).unwrap();
    session.trace_page(0, Default::default()).unwrap();
    let before = session.usage();
    let mut limits = PageProgramLimits::default();
    limits.forms.max_stream_decoded_bytes = body.len() - 1;
    let error = session.trace_page(0, limits).unwrap_err();
    assert!(format!("{error}").contains("form decoded stream bytes"));
    assert_eq!(
        session.usage().decoded_work_bytes - before.decoded_work_bytes,
        6
    );
    assert_eq!(session.form_usage().invocations, 2);
}
#[test]
fn failed_filter_retains_budget_and_original_root_callsite() {
    let root = "q Q /Fm Do";
    let bytes = fixture(root, b"not zlib", "/Filter /FlateDecode");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let limits = PageProgramSessionLimits {
        max_decoded_work_bytes: 256,
        ..Default::default()
    };
    let mut session = PageProgramSession::new(&doc, limits).unwrap();
    let first = session.trace_page(0, Default::default()).unwrap_err();
    assert_eq!(first.source_object, Some(ObjectId::new(3, 0).unwrap()));
    assert_eq!(first.span, Some(4..10));
    assert!(first.diagnostic.is_empty());
    assert_eq!(session.usage().decoded_work_bytes, 256);
    let used = session.usage();
    assert!(session.trace_page(0, Default::default()).is_err());
    assert!(session.usage().decoded_work_bytes >= used.decoded_work_bytes);
    assert_eq!(session.usage().page_attempts, 2);
}
#[test]
fn first_do_diagnostic_admission_failure_is_small_and_source_aware() {
    let bytes = fixture("q Q /Fm Do", b"", "");
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut limits = PageProgramSessionLimits::default();
    limits.forms.max_diagnostic_allocated_bytes = 0;
    let mut session = PageProgramSession::new(&doc, limits).unwrap();
    let e = session.trace_page(0, Default::default()).unwrap_err();
    assert_eq!(e.source_object, Some(ObjectId::new(3, 0).unwrap()));
    assert_eq!(e.span, Some(4..10));
    assert!(e.diagnostic.is_empty());
    assert_eq!(session.form_usage().invocations, 1);
    assert_eq!(session.form_usage().diagnostic_allocated_bytes, 0);
}
