// SPDX-License-Identifier: MIT
//! Independent whole-document serialization review.
use std::io::Write;
use std::process::{Command, Stdio};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Object, ObjectId};
use tandem_pdf_engine::serialize::SerializeError;
use tandem_pdf_engine::writer::{DocumentWriteError, DocumentWriteLimits, rewrite_document};

fn fixture(values: &[&[u8]], extra: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (index, value) in values.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(value);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in &offsets[1..] {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Root 1 0 R /Size {} {extra} >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    bytes
}
fn open(bytes: &[u8]) -> Document<'_> {
    Document::open(bytes, DocumentLimits::default()).unwrap()
}

#[test]
fn graph_cycles_are_preserved_without_following_references_recursively() {
    let bytes = fixture(&[b"<< /Loop 2 0 R >>", b"[1 0 R 2 0 R]"], "");
    let doc = open(&bytes);
    let output = rewrite_document(&doc, DocumentWriteLimits::default()).unwrap();
    let result = open(&output);
    for object in doc.objects() {
        assert_eq!(result.object(object.id).unwrap().value, object.value);
    }
    assert_eq!(
        rewrite_document(&result, DocumentWriteLimits::default()).unwrap(),
        output
    );
}

#[test]
fn syntax_budget_includes_all_objects_and_generated_trailer() {
    // One empty dictionary=1; array+two references=3; trailer with two key/value
    // pairs=5. This expected count is independent of the serializer internals.
    let bytes = fixture(&[b"<<>>", b"[1 0 R 1 0 R]"], "");
    let doc = open(&bytes);
    let mut limits = DocumentWriteLimits::default();
    limits.syntax.max_objects = 9;
    let output = rewrite_document(&doc, limits).unwrap();
    limits.syntax.max_objects = 8;
    assert!(matches!(
        rewrite_document(&doc, limits),
        Err(DocumentWriteError::Serialize(SerializeError::ObjectLimit))
    ));
    limits.syntax.max_objects = 9;
    limits.syntax.max_output_bytes = output.len();
    assert_eq!(rewrite_document(&doc, limits).unwrap(), output);
    limits.syntax.max_output_bytes -= 1;
    assert!(matches!(
        rewrite_document(&doc, limits),
        Err(DocumentWriteError::Serialize(SerializeError::OutputLimit))
    ));
    assert_eq!(
        doc.object(ObjectId::new(1, 0).unwrap()).unwrap().value,
        Object::Dictionary(Default::default())
    );
}

#[test]
fn signature_type_reference_cycles_and_indirect_fields_fail_closed() {
    for objects in [
        vec![b"<< /FT 2 0 R >>".as_slice(), b"3 0 R", b"/Sig"],
        vec![b"<< /Type 2 0 R >>".as_slice(), b"3 0 R", b"2 0 R"],
    ] {
        let bytes = fixture(&objects, "");
        assert!(rewrite_document(&open(&bytes), DocumentWriteLimits::default()).is_err());
    }
}

#[test]
fn stream_bytes_that_look_like_signatures_are_preserved_literally() {
    let payload = b"\0/Type /Sig /ByteRange [0 9 8 7]\nendstream\xff";
    let mut stream = format!("<< /Length {} >>\nstream\n", payload.len()).into_bytes();
    stream.extend_from_slice(payload);
    stream.extend_from_slice(b"\nendstream");
    let bytes = fixture(&[b"<< /Data 2 0 R >>", &stream], "");
    let output = rewrite_document(&open(&bytes), DocumentWriteLimits::default()).unwrap();
    assert_eq!(
        open(&output)
            .stream_data(ObjectId::new(2, 0).unwrap())
            .unwrap()
            .unwrap(),
        payload
    );
}

#[test]
fn cli_failure_emits_no_partial_pdf_and_preserves_input() {
    let bytes = fixture(&[b"<< /Type /Sig >>"], "");
    let saved = bytes.clone();
    let mut child = Command::new(env!("CARGO_BIN_EXE_pdf-inspect"))
        .args(["--rewrite", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&bytes).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(bytes, saved);
}

#[test]
fn absent_reference_in_unreachable_object_is_reported_with_exact_generation() {
    let bytes = fixture(&[b"<<>>", b"[17 4 R]"], "");
    let error = rewrite_document(&open(&bytes), DocumentWriteLimits::default()).unwrap_err();
    assert!(
        matches!(error, DocumentWriteError::MissingReference(id) if id == ObjectId::new(17, 4).unwrap())
    );
    assert_eq!(
        open(&bytes)
            .object(ObjectId::new(2, 0).unwrap())
            .unwrap()
            .value,
        Object::Array(vec![Object::Reference(ObjectId::new(17, 4).unwrap())])
    );
}
