// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Object, ObjectId};
use tandem_pdf_engine::serialize::SerializeError;
use tandem_pdf_engine::writer::{DocumentWriteError, DocumentWriteLimits, rewrite_document};

fn id(n: u32, g: u16) -> ObjectId {
    ObjectId::new(n, g).unwrap()
}
fn pdf(version: &str, objects: &[(u32, u16, &[u8])], trailer: &str) -> Vec<u8> {
    let mut bytes = format!("%PDF-{version}\n").into_bytes();
    let mut offsets = Vec::new();
    for &(n, g, value) in objects {
        offsets.push((n, g, bytes.len()));
        bytes.extend_from_slice(format!("{n} {g} obj\n").as_bytes());
        bytes.extend_from_slice(value);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 1\n0000000000 65535 f \n");
    for (n, g, offset) in offsets {
        bytes.extend_from_slice(format!("{n} 1\n{offset:010} {g:05} n \n").as_bytes());
    }
    let size = u64::from(objects.iter().map(|v| v.0).max().unwrap()) + 1;
    bytes.extend_from_slice(
        format!("trailer\n<< /Size {size} {trailer} >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}
fn open(bytes: &[u8]) -> Document<'_> {
    Document::open(bytes, DocumentLimits::default()).unwrap()
}
fn rewrite(doc: &Document<'_>) -> Vec<u8> {
    rewrite_document(doc, DocumentWriteLimits::default()).unwrap()
}
fn simple(extra: &str) -> Vec<u8> {
    pdf(
        "1.7",
        &[
            (1, 0, b"<< /Type /Catalog /Pages 2 0 R >>"),
            (2, 0, b"<< /Type /Pages /Kids [] /Count 0 >>"),
        ],
        &format!("/Root 1 0 R {extra}"),
    )
}

#[test]
fn preserves_every_live_object_and_header_version_with_deterministic_output() {
    let bytes = pdf(
        "2.0",
        &[
            (1, 2, b"<< /Type /Catalog /Pages 7 4 R /Version /2.0 >>"),
            (7, 4, b"<< /Type /Pages /Kids [] /Count 0 >>"),
            (42, 0, b"[(unused) <00ff> /A#20B 1.0 null]"),
        ],
        "/Root 1 2 R /Info 42 0 R /ID [<0102> <ff00>]",
    );
    let doc = open(&bytes);
    let output = rewrite(&doc);
    assert!(output.starts_with(b"%PDF-2.0\n"));
    let result = open(&output);
    assert_eq!(result.root(), id(1, 2));
    assert_eq!(result.object_count(), doc.object_count());
    for object in doc.objects() {
        assert_eq!(result.object(object.id).unwrap().value, object.value);
    }
    for key in [b"Info".as_slice(), b"ID"] {
        assert_eq!(result.trailer().get(key), doc.trailer().get(key));
    }
    assert_eq!(rewrite(&result), output);
}
#[test]
fn sparse_high_object_numbers_do_not_allocate_from_size() {
    let bytes = pdf(
        "1.7",
        &[
            (1, 0, b"<< /Type /Catalog >>"),
            (4_000_000_000, 3, b"(orphan retained)"),
        ],
        "/Root 1 0 R",
    );
    let output = rewrite(&open(&bytes));
    assert!(output.len() < 500);
    assert_eq!(
        open(&output).object(id(4_000_000_000, 3)).unwrap().value,
        Object::String(b"orphan retained".to_vec())
    );
}
#[test]
fn raw_binary_stream_and_filter_metadata_survive_with_direct_length() {
    let payload = b"\x00\xffendstream\nendobj\r\n";
    let mut stream = b"<< /Length 5 0 R /Filter /UnknownPrivateFilter /DecodeParms << /Vendor 7 >> /Custom (kept) >>\nstream\n".to_vec();
    stream.extend_from_slice(payload);
    stream.extend_from_slice(b"\nendstream");
    let length = payload.len().to_string();
    let bytes = pdf(
        "1.7",
        &[
            (1, 0, b"<< /Type /Catalog /Private 4 0 R >>"),
            (4, 0, &stream),
            (5, 0, length.as_bytes()),
        ],
        "/Root 1 0 R",
    );
    let doc = open(&bytes);
    let output = rewrite(&doc);
    let result = open(&output);
    assert_eq!(result.stream_data(id(4, 0)).unwrap().unwrap(), payload);
    let Object::Dictionary(before) = &doc.object(id(4, 0)).unwrap().value else {
        panic!()
    };
    let Object::Dictionary(after) = &result.object(id(4, 0)).unwrap().value else {
        panic!()
    };
    for (key, value) in before {
        if key != b"Length" {
            assert_eq!(after.get(key), Some(value));
        }
    }
    assert_eq!(
        after.get(b"Length".as_slice()),
        Some(&Object::Integer(payload.len() as i64))
    );
    assert_eq!(
        result.object(id(5, 0)).unwrap().value,
        Object::Integer(payload.len() as i64)
    );
}
#[test]
fn incremental_root_info_and_id_are_inherited_and_old_revision_dropped() {
    let mut bytes = simple("/Info 2 0 R /ID [(original) (latest)]");
    let first = open(&bytes);
    let previous = std::str::from_utf8(&bytes)
        .unwrap()
        .rsplit("startxref\n")
        .next()
        .unwrap()
        .split('\n')
        .next()
        .unwrap()
        .parse::<usize>()
        .unwrap();
    assert!(first.trailer().contains_key(b"Info".as_slice()));
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 1\n0000000000 65535 f \ntrailer\n<< /Size 3 /Prev {previous} >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    let doc = open(&bytes);
    assert!(!doc.trailer().contains_key(b"Info".as_slice()));
    let output = rewrite(&doc);
    let result = open(&output);
    assert_eq!(
        result.trailer().get(b"Info".as_slice()),
        Some(&Object::Reference(id(2, 0)))
    );
    assert_eq!(
        result.trailer().get(b"ID".as_slice()),
        Some(&Object::Array(vec![
            Object::String(b"original".to_vec()),
            Object::String(b"latest".to_vec())
        ]))
    );
    assert!(!result.trailer().contains_key(b"Prev".as_slice()));
    assert_eq!(result.trailer_chain().count(), 1);
}
#[test]
fn signature_markers_are_rejected_even_in_nested_unreachable_values() {
    for marker in [
        "/Type /Sig",
        "/FT /Sig",
        "/ByteRange [0 1 2 3]",
        "/Perms << >>",
        "/DocMDP 1 0 R",
        "/FieldMDP << >>",
        "/Type 8 0 R",
    ] {
        let extra = format!("[<< /Nested [<< {marker} >>] >>]");
        let bytes = pdf(
            "1.7",
            &[
                (1, 0, b"<< /Type /Catalog >>"),
                (6, 0, extra.as_bytes()),
                (8, 0, b"/Sig"),
            ],
            "/Root 1 0 R",
        );
        assert!(
            matches!(
                rewrite_document(&open(&bytes), DocumentWriteLimits::default()),
                Err(DocumentWriteError::Unsupported(_))
            ),
            "{marker}"
        );
    }
}
#[test]
fn linearized_unknown_trailer_and_dangling_references_fail_explicitly() {
    let bytes = pdf(
        "1.7",
        &[(1, 0, b"<< /Type /Catalog /Linearized 1 >>")],
        "/Root 1 0 R",
    );
    assert!(matches!(
        rewrite_document(&open(&bytes), DocumentWriteLimits::default()),
        Err(DocumentWriteError::Unsupported("linearized document"))
    ));
    let bytes = simple("/PrivateTrailerKey 7");
    assert!(matches!(
        rewrite_document(&open(&bytes), DocumentWriteLimits::default()),
        Err(DocumentWriteError::Unsupported(
            "unknown non-null trailer entry"
        ))
    ));
    let bytes = pdf(
        "1.7",
        &[(1, 0, b"<< /Type /Catalog /Missing 999 0 R >>")],
        "/Root 1 0 R",
    );
    assert!(
        matches!(rewrite_document(&open(&bytes), DocumentWriteLimits::default()), Err(DocumentWriteError::MissingReference(missing)) if missing == id(999,0))
    );
}
#[test]
fn exact_output_object_and_global_syntax_limits() {
    let bytes = simple("");
    let doc = open(&bytes);
    let output = rewrite(&doc);
    let mut limits = DocumentWriteLimits::default();
    limits.syntax.max_output_bytes = output.len();
    assert_eq!(rewrite_document(&doc, limits).unwrap(), output);
    limits.syntax.max_output_bytes -= 1;
    assert!(matches!(
        rewrite_document(&doc, limits),
        Err(DocumentWriteError::Serialize(SerializeError::OutputLimit))
    ));
    let limits = DocumentWriteLimits {
        max_indirect_objects: 1,
        ..DocumentWriteLimits::default()
    };
    assert!(matches!(
        rewrite_document(&doc, limits),
        Err(DocumentWriteError::ObjectLimit)
    ));
    let mut limits = DocumentWriteLimits::default();
    limits.syntax.max_objects = 10;
    assert!(matches!(
        rewrite_document(&doc, limits),
        Err(DocumentWriteError::Serialize(SerializeError::ObjectLimit))
    ));
    let mut limits = DocumentWriteLimits::default();
    limits.syntax.max_depth = 0;
    assert!(matches!(
        rewrite_document(&doc, limits),
        Err(DocumentWriteError::Serialize(SerializeError::DepthLimit))
    ));
    assert_eq!(rewrite(&doc), output); // errors leave source document intact
}
#[test]
fn null_optional_entries_do_not_prevent_rewrite() {
    let bytes = pdf(
        "1.7",
        &[(
            1,
            0,
            b"<< /Type /Catalog /ByteRange null /Perms null /Linearized null >>",
        )],
        "/Root 1 0 R /Whatever null",
    );
    assert_eq!(open(&rewrite(&open(&bytes))).object_count(), 1);
}
#[test]
fn dormant_xref_stream_infrastructure_is_preserved_as_an_ordinary_stream() {
    let bytes = pdf(
        "1.7",
        &[
            (1, 0, b"<< /Type /Catalog /Metadata 6 0 R >>"),
            (
                6,
                0,
                b"<< /Type /XRef /Length 3 /W [1 1 1] >>\nstream\nABC\nendstream",
            ),
        ],
        "/Root 1 0 R",
    );
    let doc = open(&bytes);
    let output = rewrite(&doc);
    let result = open(&output);
    assert_eq!(result.stream_data(id(6, 0)).unwrap().unwrap(), b"ABC");
    assert_eq!(
        result.object(id(6, 0)).unwrap().value,
        doc.object(id(6, 0)).unwrap().value
    );
}
