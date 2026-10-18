// SPDX-License-Identifier: MIT
use tandem_pdf_engine::document::{Document, DocumentErrorKind, DocumentLimits};
use tandem_pdf_engine::object::{Object, ObjectId};

struct Fixture {
    bytes: Vec<u8>,
    offsets: Vec<usize>,
    xref: usize,
}
fn fixture(bodies: &[&[u8]], extra: &str) -> Fixture {
    let mut bytes = b"%PDF-1.7\n%\x80\x81\x82\x83\n".to_vec();
    let mut offsets = vec![0];
    for (index, body) in bodies.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes(),
    );
    for offset in offsets.iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R {extra} >>\nstartxref\n{xref}\n%%EOF\n",
            bodies.len() + 1
        )
        .as_bytes(),
    );
    Fixture {
        bytes,
        offsets,
        xref,
    }
}
fn id(number: u32) -> ObjectId {
    ObjectId::new(number, 0).unwrap()
}
fn replace_same(bytes: &mut [u8], old: &[u8], new: &[u8]) {
    assert_eq!(old.len(), new.len());
    let position = bytes
        .windows(old.len())
        .position(|window| window == old)
        .unwrap();
    bytes[position..position + old.len()].copy_from_slice(new);
}
fn open(bytes: &[u8]) -> Result<Document<'_>, tandem_pdf_engine::document::DocumentError> {
    Document::open(bytes, DocumentLimits::default())
}

#[test]
fn opens_owned_values_and_resolves_exact_ids() {
    let file = fixture(&[b"<< /Type /Catalog >>", b"[true (hello) 1 0 R]"], "");
    let document = open(&file.bytes).unwrap();
    assert_eq!(document.root(), id(1));
    assert_eq!(document.object_count(), 2);
    assert_eq!(document.objects().count(), 2);
    assert_eq!(
        document.trailer().get(b"Size".as_slice()),
        Some(&Object::Integer(3))
    );
    assert!(matches!(
        document.object(id(2)).unwrap().value,
        Object::Array(_)
    ));
    assert!(document.stream_data(id(1)).unwrap().is_none());
    assert!(matches!(
        document
            .object(ObjectId::new(1, 1).unwrap())
            .unwrap_err()
            .kind,
        DocumentErrorKind::MissingObject(_)
    ));
}

#[test]
fn direct_streams_borrow_arbitrary_binary_and_marker_bytes() {
    let payload = b"\x00endstream\nendobj\xff%noise";
    let mut body = format!("<< /Length {} >>\nstream\n", payload.len()).into_bytes();
    body.extend_from_slice(payload);
    body.extend_from_slice(b"\nendstream");
    let file = fixture(&[b"<<>>", &body], "");
    let document = open(&file.bytes).unwrap();
    let bytes = document.stream_data(id(2)).unwrap().unwrap();
    assert_eq!(bytes, payload);
    let range = document.object(id(2)).unwrap().stream.clone().unwrap();
    assert_eq!(bytes.as_ptr(), file.bytes[range].as_ptr());
}

#[test]
fn indirect_stream_lengths_resolve_forward_and_backward() {
    for bodies in [
        vec![
            b"<<>>".as_slice(),
            b"<< /Length 3 0 R >>\nstream\nabc\nendstream",
            b"3",
        ],
        vec![
            b"<<>>".as_slice(),
            b"3",
            b"<< /Length 2 0 R >>\nstream\nabc\nendstream",
        ],
    ] {
        let file = fixture(&bodies, "");
        let document = open(&file.bytes).unwrap();
        let stream = document
            .objects()
            .find(|object| object.stream.is_some())
            .unwrap();
        assert_eq!(document.stream_data(stream.id).unwrap().unwrap(), b"abc");
    }
}

#[test]
fn stream_length_boundaries_are_exact() {
    for body in [
        b"<< /Length 4 >>\nstream\nabc\nendstream".as_slice(), // EOL included in length is valid
        b"<< /Length 3 >>\r\nstream\r\nabc\r\nendstream",
        b"<< /Length 0 >>\nstream\nendstream",
    ] {
        assert!(
            open(&fixture(&[b"<<>>", body], "").bytes).is_ok(),
            "{body:?}"
        );
    }
    for body in [
        b"<< /Length 2 >>\nstream\nabc\nendstream".as_slice(),
        b"<< /Length 5 >>\nstream\nabc\nendstream",
        b"<< /Length -1 >>\nstream\nabc\nendstream",
        b"<< /Length 999999999 >>\nstream\nabc\nendstream",
        b"<< /Length 3.0 >>\nstream\nabc\nendstream",
        b"<< >>\nstream\nabc\nendstream",
        b"<< /Length 3 >>\nstream \nabc\nendstream",
        b"<< /Length 3 >>\nstream\rabc\nendstream",
        b"<< /Length 0 >>\nstream\n%ignored\nendstream",
        b"<< /Length 0 >>\nstream\n  endstream",
        b"3\nstream\nabc\nendstream",
    ] {
        assert!(
            open(&fixture(&[b"<<>>", body], "").bytes).is_err(),
            "{body:?}"
        );
    }
}

#[test]
fn missing_cyclic_and_nonscalar_lengths_fail() {
    for bodies in [
        vec![
            b"<<>>".as_slice(),
            b"<< /Length 2 0 R >>\nstream\nabc\nendstream",
        ],
        vec![
            b"<<>>".as_slice(),
            b"<< /Length 3 0 R >>\nstream\nabc\nendstream",
            b"2 0 R",
        ],
        vec![
            b"<<>>".as_slice(),
            b"<< /Length 9 0 R >>\nstream\nabc\nendstream",
        ],
        vec![
            b"<<>>".as_slice(),
            b"<< /Length 3 0 R >>\nstream\nabc\nendstream",
            b"(3)",
        ],
    ] {
        assert!(open(&fixture(&bodies, "").bytes).is_err());
    }
}

#[test]
fn unsupported_features_are_explicit() {
    for extra in ["/XRefStm 10", "/Encrypt 2 0 R"] {
        let file = fixture(&[b"<<>>", b"<<>>"], extra);
        assert!(matches!(
            open(&file.bytes).unwrap_err().kind,
            DocumentErrorKind::Unsupported(_)
        ));
    }
    let mut file = fixture(&[b"<<>>"], "");
    file.bytes.truncate(file.xref);
    file.bytes.extend_from_slice(
        format!(
            "2 0 obj\n<< /Type /XRef >>\nendobj\nstartxref\n{}\n%%EOF",
            file.xref
        )
        .as_bytes(),
    );
    // Recognized bootstrap syntax still requires valid Length/W/Size metadata.
    assert!(open(&file.bytes).is_err());
}

#[test]
fn validates_header_footer_and_trailer() {
    for extra in ["/Size 0", "/Root 9 0 R", "/Root null"] {
        // Duplicate trailer keys are also invalid, so construct replacement below.
        assert!(open(&fixture(&[b"<<>>"], extra).bytes).is_err());
    }
    let file = fixture(&[b"<<>>"], "");
    for end in [0, 8, file.xref, file.bytes.len() - 7] {
        assert!(open(&file.bytes[..end]).is_err());
    }
    let mut bytes = file.bytes.clone();
    bytes[0] = b'!';
    assert!(open(&bytes).is_err());
    let mut bytes = file.bytes.clone();
    replace_same(&mut bytes, b"/Root 1 0 R", b"/Root 9 0 R");
    assert!(matches!(
        open(&bytes).unwrap_err().kind,
        DocumentErrorKind::MissingObject(_)
    ));
    let mut bytes = file.bytes.clone();
    replace_same(&mut bytes, b"/Size 2", b"/Size 1");
    assert!(open(&bytes).is_err());
    let mut bytes = file.bytes.clone();
    bytes.extend_from_slice(b"garbage");
    assert!(open(&bytes).is_err());
}

#[test]
fn rejects_xref_aliases_offset_mismatch_and_overlaps() {
    let file = fixture(&[b"<< /A (long value) >>", b"42"], "");
    let mut alias = file.bytes.clone();
    replace_same(
        &mut alias,
        format!("{:010} 00000 n", file.offsets[2]).as_bytes(),
        format!("{:010} 00000 n", file.offsets[1]).as_bytes(),
    );
    assert!(open(&alias).unwrap_err().to_string().contains("aliased"));
    let mut mismatch = file.bytes.clone();
    replace_same(&mut mismatch, b"2 0 obj", b"2 1 obj");
    assert!(
        open(&mismatch)
            .unwrap_err()
            .to_string()
            .contains("does not match")
    );
    let mut overlap = file.bytes.clone();
    replace_same(
        &mut overlap,
        format!("{:010} 00000 n", file.offsets[2]).as_bytes(),
        format!("{:010} 00000 n", file.offsets[1] + 15).as_bytes(),
    );
    assert!(open(&overlap).is_err());
    let mut bad_offset = file.bytes.clone();
    replace_same(
        &mut bad_offset,
        format!("{:010} 00000 n", file.offsets[1]).as_bytes(),
        b"9999999999 00000 n",
    );
    assert!(open(&bad_offset).is_err());
}

#[test]
fn fixed_width_xref_and_free_zero_are_validated() {
    let file = fixture(&[b"<<>>"], "");
    // Entry 0 must be free; its generation is not checked (other readers
    // accept 0, and 1-byte xref-stream fields cannot store 65535).
    let mut zero = file.bytes.clone();
    replace_same(&mut zero, b"0000000000 65535 f", b"0000000000 00000 f");
    assert!(open(&zero).is_ok());
    for (old, new) in [
        (
            b"0000000000 65535 f".as_slice(),
            b"0000000000 65535 n".as_slice(),
        ),
        (b" 00000 n ", b" 99999 n "),
        (b" 00000 n ", b" 00000 z "),
    ] {
        let mut bytes = file.bytes.clone();
        replace_same(&mut bytes, old, new);
        assert!(open(&bytes).is_err());
    }
}

#[test]
fn supports_multiple_subsections_and_nonzero_generation() {
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let first = bytes.len();
    bytes.extend_from_slice(b"1 2 obj\n<<>>\nendobj\n");
    let third = bytes.len();
    bytes.extend_from_slice(b"3 0 obj\n17\nendobj\n");
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 2\n0000000000 65535 f \n{first:010} 00002 n \n3 1\n{third:010} 00000 n \ntrailer\n<< /Size 4 /Root 1 2 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    let document = open(&bytes).unwrap();
    assert_eq!(document.root(), ObjectId::new(1, 2).unwrap());
    assert_eq!(document.object(id(3)).unwrap().value, Object::Integer(17));
    let bad = String::from_utf8(bytes)
        .unwrap()
        .replace("3 1\n", "1 1\n")
        .into_bytes();
    assert!(
        open(&bad)
            .unwrap_err()
            .to_string()
            .contains("duplicate xref")
    );
}

#[test]
fn cumulative_budgets_cover_trailer_and_multiple_objects() {
    let file = fixture(&[b"<<>>", b"(abc)", b"(def)"], "");
    let defaults = DocumentLimits::default();
    for limits in [
        DocumentLimits {
            max_input_bytes: file.bytes.len() - 1,
            ..defaults
        },
        DocumentLimits {
            max_xref_entries: 3,
            ..defaults
        },
        DocumentLimits {
            max_objects: 7,
            ..defaults
        }, // trailer5 + dict1 + string1 + string1
        DocumentLimits {
            max_decoded_bytes: 13,
            ..defaults
        }, // trailer8 + strings6
        DocumentLimits {
            max_syntax_bytes: file.bytes.len() - 1,
            ..defaults
        },
    ] {
        assert!(Document::open(&file.bytes, limits).is_err(), "{limits:?}");
    }
    assert!(
        Document::open(
            &file.bytes,
            DocumentLimits {
                max_syntax_bytes: file.bytes.len(),
                max_objects: 8,
                max_decoded_bytes: 14,
                ..defaults
            }
        )
        .is_ok()
    );
}

#[test]
fn borrowed_payload_does_not_consume_decoded_or_syntax_budget() {
    let payload = vec![b'x'; 10000];
    let mut body = b"<< /Length 10000 >>\nstream\n".to_vec();
    body.extend_from_slice(&payload);
    body.extend_from_slice(b"\nendstream");
    let file = fixture(&[b"<<>>", &body], "");
    let limits = DocumentLimits {
        max_syntax_bytes: file.bytes.len() - payload.len(),
        max_decoded_bytes: 14,
        ..DocumentLimits::default()
    };
    assert!(Document::open(&file.bytes, limits).is_ok());
    assert!(
        Document::open(
            &file.bytes,
            DocumentLimits {
                max_syntax_bytes: limits.max_syntax_bytes - 1,
                ..limits
            }
        )
        .is_err()
    );
}

#[test]
fn truncation_and_deterministic_mutation_do_not_panic() {
    let file = fixture(
        &[
            b"<< /Type /Catalog >>",
            b"<< /Length 3 >>\nstream\nabc\nendstream",
        ],
        "",
    );
    for end in 0..file.bytes.len() {
        let _ = open(&file.bytes[..end]);
    }
    for position in 0..file.bytes.len() {
        for replacement in [0, b'%', b'9', 255] {
            let mut bytes = file.bytes.clone();
            bytes[position] = replacement;
            let _ = open(&bytes);
        }
    }
}

#[test]
fn structural_markers_cannot_begin_inside_comments_or_whitespace() {
    let file = fixture(&[b"<<>>", b"42"], "");
    let mut bytes = file.bytes.clone();
    // Removing the EOL makes the first object's xref target lie inside the
    // binary header comment, despite its otherwise-valid object wrapper.
    bytes[file.offsets[1] - 1] = b' ';
    assert!(open(&bytes).is_err());
    let mut bytes = file.bytes.clone();
    bytes[file.offsets[2] - 1] = b'%';
    assert!(open(&bytes).is_err());
    let mut bad_footer = file.bytes.clone();
    replace_same(&mut bad_footer, b"\n%%EOF", b" %%EOF");
    assert!(open(&bad_footer).is_err());
    let mut bytes = file.bytes.clone();
    let original = format!("startxref\n{}", file.xref);
    let changed = format!("startxref\n{}", file.xref - 1);
    replace_same(&mut bytes, original.as_bytes(), changed.as_bytes());
    assert!(open(&bytes).is_err());
}

#[test]
fn external_file_streams_fail_explicitly() {
    let file = fixture(
        &[
            b"<<>>",
            b"<< /Length 0 /F (external.bin) >>\nstream\nendstream",
        ],
        "",
    );
    assert!(matches!(
        open(&file.bytes).unwrap_err().kind,
        DocumentErrorKind::Unsupported("external-file streams")
    ));
    assert!(
        open(
            &fixture(
                &[b"<<>>", b"<< /Length 0 /F null >>\nstream\nendstream"],
                ""
            )
            .bytes
        )
        .is_ok()
    );
}

#[test]
fn dormant_infrastructure_streams_are_ordinary_resolvable_objects() {
    let file = fixture(
        &[
            b"<<>>",
            b"<< /Type /XRef /Length 0 >>\nstream\nendstream",
            b"<< /Type /ObjStm /N 0 /First 0 /Length 0 >>\nstream\nendstream",
        ],
        "",
    );
    let document = open(&file.bytes).unwrap();
    assert_eq!(document.stream_data(id(2)).unwrap(), Some(b"".as_slice()));
    assert_eq!(document.stream_data(id(3)).unwrap(), Some(b"".as_slice()));
}
