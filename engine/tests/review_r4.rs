// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent incremental/xref-stream review fixtures.
use tandem_pdf_engine::document::{Document, DocumentErrorKind, DocumentLimits};
use tandem_pdf_engine::object::{Object, ObjectId};
use tandem_pdf_engine::pages::{PageLimits, collect_pages};

fn add_object(bytes: &mut Vec<u8>, number: u32, generation: u16, body: &[u8]) -> usize {
    let offset = bytes.len();
    bytes.extend_from_slice(format!("{number} {generation} obj\n").as_bytes());
    bytes.extend_from_slice(body);
    bytes.extend_from_slice(b"\nendobj\n");
    offset
}
fn base(page: &[u8]) -> (Vec<u8>, usize) {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let values = [
        b"<< /Type /Catalog /Pages 2 0 R >>".as_slice(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>",
        page,
        b"<< /Length 4 >>\nstream\nABCD\nendstream",
    ];
    let mut offsets = Vec::new();
    for (index, value) in values.into_iter().enumerate() {
        offsets.push(add_object(&mut bytes, index as u32 + 1, 0, value));
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    (bytes, xref)
}
fn revision(
    bytes: &mut Vec<u8>,
    previous: usize,
    entries: &[(u32, u16, Option<&[u8]>)],
    root: &str,
) -> usize {
    let mut offsets = Vec::new();
    for &(number, generation, value) in entries {
        offsets.push(value.map(|value| add_object(bytes, number, generation, value)));
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n");
    for (&(number, generation, _), offset) in entries.iter().zip(offsets) {
        bytes.extend_from_slice(
            format!(
                "{number} 1\n{:010} {generation:05} {} \n",
                offset.unwrap_or(0),
                if offset.is_some() { 'n' } else { 'f' }
            )
            .as_bytes(),
        );
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 5 /Prev {previous} {root} >>\nstartxref\n{xref}\n%%EOF\n")
            .as_bytes(),
    );
    xref
}
fn id(number: u32, generation: u16) -> ObjectId {
    ObjectId::new(number, generation).unwrap()
}

#[test]
fn superseded_invalid_body_is_not_parsed_and_old_stream_remains_borrowed() {
    let (mut input, previous) = base(b"this stale body is not a PDF object");
    revision(
        &mut input,
        previous,
        &[(
            3,
            0,
            Some(b"<< /Type /Page /Parent 2 0 R /Rotate 90 /Contents 4 0 R >>"),
        )],
        "",
    );
    let doc = Document::open(&input, DocumentLimits::default()).unwrap();
    assert_eq!(doc.root(), id(1, 0));
    assert!(!doc.trailer().contains_key(b"Root".as_slice()));
    let pages = collect_pages(&doc, PageLimits::default()).unwrap();
    assert_eq!(pages[0].rotation, 90);
    assert_eq!(pages[0].display_bounds(), [0.0, 0.0, 300.0, 200.0]);
    let stream = doc.stream_data(id(4, 0)).unwrap().unwrap();
    assert_eq!(stream, b"ABCD");
    let range = doc.object(id(4, 0)).unwrap().stream.clone().unwrap();
    assert_eq!(stream.as_ptr(), input[range].as_ptr());
}

#[test]
fn newest_free_entry_tombstones_older_live_object() {
    let (mut input, previous) = base(b"<< /Type /Page /Parent 2 0 R >>");
    revision(&mut input, previous, &[(3, 1, None)], "/Root null");
    let doc = Document::open(&input, DocumentLimits::default()).unwrap();
    assert!(matches!(
        doc.object(id(3, 0)).unwrap_err().kind,
        DocumentErrorKind::MissingObject(_)
    ));
    assert!(matches!(
        doc.object(id(3, 1)).unwrap_err().kind,
        DocumentErrorKind::MissingObject(_)
    ));
    assert!(collect_pages(&doc, PageLimits::default()).is_err());
}

#[test]
fn generation_replacement_is_keyed_by_object_number() {
    let (mut input, previous) = base(b"<< /Type /Page /Parent 2 0 R >>");
    revision(
        &mut input,
        previous,
        &[
            (
                2,
                0,
                Some(b"<< /Type /Pages /Kids [3 1 R] /Count 1 /MediaBox [0 0 200 300] >>"),
            ),
            (3, 1, Some(b"<< /Type /Page /Parent 2 0 R /Rotate 180 >>")),
        ],
        "/Root 1 0 R",
    );
    let doc = Document::open(&input, DocumentLimits::default()).unwrap();
    assert!(doc.object(id(3, 0)).is_err());
    assert!(matches!(
        doc.object(id(3, 1)).unwrap().value,
        Object::Dictionary(_)
    ));
    assert_eq!(
        collect_pages(&doc, PageLimits::default()).unwrap()[0].rotation,
        180
    );
}

#[test]
fn all_revisions_and_superseded_entries_charge_global_budgets() {
    let (mut input, mut previous) = base(b"<< /Type /Page /Parent 2 0 R >>");
    for _ in 0..3 {
        previous = revision(
            &mut input,
            previous,
            &[(3, 0, Some(b"<< /Type /Page /Parent 2 0 R >>"))],
            "",
        );
    }
    let exact = DocumentLimits {
        max_revisions: 4,
        max_xref_entries: 8,
        ..DocumentLimits::default()
    };
    assert!(Document::open(&input, exact).is_ok());
    for limited in [
        DocumentLimits {
            max_revisions: 3,
            ..exact
        },
        DocumentLimits {
            max_xref_entries: 7,
            ..exact
        },
    ] {
        assert!(matches!(
            Document::open(&input, limited).unwrap_err().kind,
            DocumentErrorKind::LimitExceeded(_)
        ));
    }
}

#[test]
fn live_objects_cannot_point_inside_previous_xref_metadata() {
    let (mut input, previous) = base(b"<< /Type /Page /Parent 2 0 R >>");
    let latest = input.len();
    input.extend_from_slice(format!("xref\n3 1\n{previous:010} 00000 n \ntrailer\n<< /Size 5 /Root 1 0 R /Prev {previous} >>\nstartxref\n{latest}\n%%EOF\n").as_bytes());
    assert!(Document::open(&input, DocumentLimits::default()).is_err());
}

#[test]
fn forward_and_self_prev_links_are_rejected() {
    let (original, previous) = base(b"<< /Type /Page /Parent 2 0 R >>");
    for increment in [0, 1] {
        let mut input = original.clone();
        let latest = input.len();
        input.extend_from_slice(format!("xref\n0 1\n0000000000 65535 f \ntrailer\n<< /Size 5 /Root 1 0 R /Prev {} >>\nstartxref\n{latest}\n%%EOF\n", latest + increment).as_bytes());
        assert!(Document::open(&input, DocumentLimits::default()).is_err());
    }
    assert!(previous < original.len());
}

fn stream_revision(
    input: &mut Vec<u8>,
    previous: usize,
    number: u32,
    rotation: u16,
    predictor: bool,
    index: &str,
    replacement_rows: Option<&[u8]>,
) -> usize {
    let page_offset = add_object(
        input,
        3,
        0,
        format!("<< /Type /Page /Parent 2 0 R /Rotate {rotation} /Contents 4 0 R >>").as_bytes(),
    );
    let mut row = vec![1];
    row.extend_from_slice(&(page_offset as u32).to_be_bytes());
    row.push(0);
    if let Some(replacement) = replacement_rows {
        row = replacement.to_vec();
    }
    let mut encoded = Vec::new();
    if predictor {
        encoded.push(2);
    }
    encoded.extend_from_slice(&row);
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&encoded, 6);
    let offset = input.len();
    let parameters = if predictor {
        "/DecodeParms << /Predictor 12 /Columns 6 >>"
    } else {
        ""
    };
    let mut body = format!("<< /Type /XRef /Size {} /Prev {previous} /Filter /FlateDecode {parameters} /Length {} /W [1 4 1] /Index {index} >>\nstream\n", number + 1, compressed.len()).into_bytes();
    body.extend_from_slice(&compressed);
    body.extend_from_slice(b"\nendstream");
    assert_eq!(add_object(input, number, 0, &body), offset);
    input.extend_from_slice(format!("startxref\n{offset}\n%%EOF\n").as_bytes());
    offset
}

#[test]
fn multiple_predictor_metadata_sections_charge_all_intermediates() {
    let (mut input, previous) = base(b"<< /Type /Page /Parent 2 0 R >>");
    let previous = stream_revision(&mut input, previous, 5, 90, true, "[3 1]", None);
    stream_revision(&mut input, previous, 6, 180, true, "[3 1]", None);
    // Two sections, each seven inflated bytes plus six predictor output bytes.
    let exact = DocumentLimits {
        max_metadata_decoded_bytes: 26,
        max_xref_entries: 9,
        ..DocumentLimits::default()
    };
    let doc = Document::open(&input, exact).unwrap();
    assert_eq!(
        collect_pages(&doc, PageLimits::default()).unwrap()[0].rotation,
        180
    );
    assert!(doc.stream_data(id(5, 0)).unwrap().is_some());
    assert!(doc.stream_data(id(6, 0)).unwrap().is_some());
    assert!(
        Document::open(
            &input,
            DocumentLimits {
                max_metadata_decoded_bytes: 25,
                ..exact
            }
        )
        .is_err()
    );
    assert!(
        Document::open(
            &input,
            DocumentLimits {
                max_xref_entries: 8,
                ..exact
            }
        )
        .is_err()
    );
}

#[test]
fn sparse_xref_bootstrap_is_resolved_once_and_latest_page_wins() {
    let (mut input, previous) = base(b"<< /Type /Page /Parent 2 0 R >>");
    stream_revision(&mut input, previous, 5, 270, false, "[3 1]", None);
    let doc = Document::open(&input, DocumentLimits::default()).unwrap();
    assert_eq!(doc.object_count(), 5);
    assert!(doc.stream_data(id(5, 0)).unwrap().is_some());
    assert_eq!(
        collect_pages(&doc, PageLimits::default()).unwrap()[0].rotation,
        270
    );
    assert_eq!(doc.stream_data(id(4, 0)).unwrap().unwrap(), b"ABCD");
}

#[test]
fn compressed_unknown_and_conflicting_self_entries_are_never_silently_used() {
    for (index, record, expect_unsupported) in [
        ("[3 1]", [2, 0, 0, 0, 4, 0], false),
        ("[3 1]", [3, 0, 0, 0, 4, 0], false),
        ("[5 1]", [0, 0, 0, 0, 0, 0], false),
    ] {
        let (mut input, previous) = base(b"<< /Type /Page /Parent 2 0 R >>");
        stream_revision(&mut input, previous, 5, 90, false, index, Some(&record));
        let error = Document::open(&input, DocumentLimits::default()).unwrap_err();
        assert_eq!(
            matches!(error.kind, DocumentErrorKind::Unsupported(_)),
            expect_unsupported
        );
    }
}
