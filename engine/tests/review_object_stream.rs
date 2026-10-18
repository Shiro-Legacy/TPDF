// SPDX-License-Identifier: MIT
//! Independent object-stream boundary and cumulative resource regressions.
#[path = "support/pages.rs"]
mod pages;
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Object, ObjectId};

fn fixture(header: &[u8], body: &[u8]) -> (Vec<u8>, usize) {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut entries = [[0u64; 3]; 21];
    entries[0] = [0, 0, 65535];
    for (n, value) in [
        (1, b"<< /Type /Catalog /Pages 2 0 R >>".as_slice()),
        (2, b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>"),
        (
            3,
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Rotate 8 0 R >>",
        ),
    ] {
        entries[n] = [1, bytes.len() as u64, 0];
        bytes.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        bytes.extend_from_slice(value);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let decoded = [header, body].concat();
    entries[10] = [1, bytes.len() as u64, 0];
    bytes.extend_from_slice(
        format!(
            "10 0 obj\n<< /Type /ObjStm /N 2 /First {} /Length {} >>\nstream\n",
            header.len(),
            decoded.len()
        )
        .as_bytes(),
    );
    bytes.extend_from_slice(&decoded);
    bytes.extend_from_slice(b"\nendstream\nendobj\n");
    entries[8] = [2, 10, 0];
    entries[9] = [2, 10, 1];
    let xref = bytes.len();
    entries[20] = [1, xref as u64, 0];
    let payload: Vec<u8> = entries
        .into_iter()
        .flat_map(|[kind, first, second]| {
            let mut row = vec![kind as u8];
            row.extend_from_slice(&(first as u32).to_be_bytes());
            row.extend_from_slice(&(second as u16).to_be_bytes());
            row
        })
        .collect();
    bytes.extend_from_slice(
        format!(
            "20 0 obj\n<< /Type /XRef /Root 1 0 R /Size 21 /W [1 4 2] /Length {} >>\nstream\n",
            payload.len()
        )
        .as_bytes(),
    );
    bytes.extend_from_slice(&payload);
    bytes.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    (bytes, decoded.len() + payload.len())
}

#[test]
fn first_is_not_an_artificial_lexical_separator() {
    // A reader that joins the bytes reads the final header number as 2900
    // and then cannot load the rotation object. The strict reader must refuse this ambiguity.
    let (bytes, _) = fixture(b"8 0 9 2", b"900");
    assert!(Document::open(&bytes, DocumentLimits::default()).is_err());
    // A real delimiter is sufficient; whitespace is not mandatory.
    let (bytes, _) = fixture(b"8 0 9 2", b"[]90");
    assert!(Document::open(&bytes, DocumentLimits::default()).is_ok());
}

#[test]
fn member_offsets_bound_adjacent_numeric_values() {
    let (bytes, _) = fixture(b"8 0 9 2 ", b"900");
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    for (n, expected) in [(8, 90), (9, 0)] {
        assert_eq!(
            doc.object(ObjectId::new(n, 0).unwrap()).unwrap().value,
            Object::Integer(expected)
        );
    }
    assert_eq!(pages::first_rotation(&doc), 90);
}

#[test]
fn xref_and_object_stream_share_one_exact_metadata_budget() {
    let (bytes, cost) = fixture(b"8 0 9 2 ", b"900");
    let limits = DocumentLimits {
        max_metadata_decoded_bytes: cost,
        max_compressed_members: 2,
        max_object_streams: 1,
        ..DocumentLimits::default()
    };
    assert!(Document::open(&bytes, limits).is_ok());
    assert!(
        Document::open(
            &bytes,
            DocumentLimits {
                max_metadata_decoded_bytes: cost - 1,
                ..limits
            }
        )
        .is_err()
    );
    assert!(
        Document::open(
            &bytes,
            DocumentLimits {
                max_compressed_members: 1,
                ..limits
            }
        )
        .is_err()
    );
    assert!(
        Document::open(
            &bytes,
            DocumentLimits {
                max_object_streams: 0,
                ..limits
            }
        )
        .is_err()
    );
}
