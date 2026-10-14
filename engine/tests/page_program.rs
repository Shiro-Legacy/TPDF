// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::content::{ContentLimits, ContentParser};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::page_program::{
    ContentSourcePiece, PageProgramLimits, PageProgramSession, PageProgramSessionLimits,
};
fn pdf(contents: &str, streams: &[(&str, &[u8])], extra: &[String]) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] {contents} >>").into_bytes(),
    ];
    for (metadata, raw) in streams {
        let mut value = format!("<< /Length {} {metadata} >>\nstream\n", raw.len()).into_bytes();
        value.extend_from_slice(raw);
        value.extend_from_slice(b"\nendstream");
        objects.push(value);
    }
    objects.extend(extra.iter().map(|v| v.as_bytes().to_vec()));
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, value) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(value);
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
fn empty_endpath_does_not_block_text() {
    let input = pdf("/Contents 4 0 R", &[("", b"n BT ET")], &[]);
    let document = Document::open(&input, DocumentLimits::default()).unwrap();
    let mut session =
        PageProgramSession::new(&document, PageProgramSessionLimits::default()).unwrap();
    let result = session.trace_page(0, PageProgramLimits::default()).unwrap();
    assert_eq!(result.program().text().text_objects().len(), 1);
    assert_eq!(session.usage().parser_operations, 6);
}
#[test]
fn projection_distinguishes_stream_pieces_and_synthetic_padding() {
    let input = pdf(
        "/Contents [4 0 R 5 0 R]",
        &[("", b"0 0 m"), ("", b"1 1 l S")],
        &[],
    );
    let document = Document::open(&input, DocumentLimits::default()).unwrap();
    let mut session =
        PageProgramSession::new(&document, PageProgramSessionLimits::default()).unwrap();
    let result = session.trace_page(0, PageProgramLimits::default()).unwrap();
    let pieces: Vec<_> = result.project_range(3..9).unwrap().collect();
    assert!(
        matches!(&pieces[..], [ContentSourcePiece::Stream {occurrence:0, decoded_range, logical_range, ..}, ContentSourcePiece::Padding {occurrence:0,logical_offset:5}, ContentSourcePiece::Stream {occurrence:1,..}] if *decoded_range == (3..5) && *logical_range == (3..5))
    );
    assert!(result.project_range(0..100).is_none());
    assert_eq!(result.project_range(5..5).unwrap().count(), 0);
}
#[test]
fn parser_usage_counts_distinct_lexing_and_retained_payload() {
    let mut parser = ContentParser::new(b"/F 12 Tf (AB) Tj", ContentLimits::default());
    while parser.next_operation().unwrap().is_some() {}
    let usage = parser.usage();
    assert_eq!(usage.operations, 2);
    assert_eq!(usage.tokens, 5);
    assert_eq!(usage.nodes, 3);
    assert_eq!(usage.owned_bytes, 7); // F + Tf + AB + Tj
}
#[test]
fn parser_error_preserves_exact_origin_offset() {
    let input = pdf("/Contents 4 0 R", &[("", b"q Q [")], &[]);
    let document = Document::open(&input, DocumentLimits::default()).unwrap();
    let mut session =
        PageProgramSession::new(&document, PageProgramSessionLimits::default()).unwrap();
    let failure = session
        .trace_page(0, PageProgramLimits::default())
        .unwrap_err();
    assert_eq!(failure.operation_index, Some(2));
    if let tandem_pdf_engine::page_program::PageProgramErrorKind::Parser(source) = failure.kind {
        assert_eq!(failure.span, Some(source.offset..source.offset));
    } else {
        panic!("expected parser error");
    }
}
#[test]
fn schema_array_walk_is_admitted_against_execution_work() {
    let input = pdf("/Contents 4 0 R", &[("", b"[1 2 3 4 5] 0 d")], &[]);
    let document = Document::open(&input, DocumentLimits::default()).unwrap();
    let mut session =
        PageProgramSession::new(&document, PageProgramSessionLimits::default()).unwrap();
    let mut limits = PageProgramLimits::default();
    limits.program.max_work_units = 4;
    assert!(session.trace_page(0, limits).is_err());
    assert_eq!(session.usage().program_operations, 0);
    assert_eq!(session.usage().program_work_units, 0);
}
