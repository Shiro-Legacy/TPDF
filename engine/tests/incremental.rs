// SPDX-License-Identifier: MIT
use tandem_pdf_engine::document::{Document, DocumentErrorKind, DocumentLimits};
use tandem_pdf_engine::object::{Object, ObjectId};

struct File {
    bytes: Vec<u8>,
    latest: usize,
    offsets: Vec<usize>,
}
fn base() -> File {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (number, body) in [
        (1, b"<<>>".as_slice()),
        (2, b"(old)"),
        (3, b"<< /Length 3 >>\nstream\nabc\nendstream"),
    ] {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let latest = bytes.len();
    bytes.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{latest}\n%%EOF\n").as_bytes(),
    );
    File {
        bytes,
        latest,
        offsets,
    }
}
fn append(
    file: &mut File,
    changes: &[(u32, u16, Option<&[u8]>)],
    trailer: &str,
    prev: Option<usize>,
) {
    let previous = prev.unwrap_or(file.latest);
    let mut entries = Vec::new();
    for &(number, generation, body) in changes {
        let offset = if let Some(body) = body {
            let offset = file.bytes.len();
            file.bytes
                .extend_from_slice(format!("{number} {generation} obj\n").as_bytes());
            file.bytes.extend_from_slice(body);
            file.bytes.extend_from_slice(b"\nendobj\n");
            offset
        } else {
            0
        };
        entries.push((number, generation, offset, body.is_some()));
    }
    file.latest = file.bytes.len();
    file.bytes.extend_from_slice(b"xref\n");
    for (number, generation, offset, live) in entries {
        file.bytes.extend_from_slice(
            format!(
                "{number} 1\n{offset:010} {generation:05} {} \n",
                if live { 'n' } else { 'f' }
            )
            .as_bytes(),
        );
    }
    file.bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size 4 /Prev {previous} {trailer} >>\nstartxref\n{}\n%%EOF\n",
            file.latest
        )
        .as_bytes(),
    );
}
fn id(number: u32, generation: u16) -> ObjectId {
    ObjectId::new(number, generation).unwrap()
}
fn open(bytes: &[u8]) -> Result<Document<'_>, tandem_pdf_engine::document::DocumentError> {
    Document::open(bytes, DocumentLimits::default())
}

#[test]
fn newest_values_replace_old_while_unchanged_stream_remains_readable() {
    let mut file = base();
    append(&mut file, &[(2, 0, Some(b"(new)"))], "", None);
    let document = open(&file.bytes).unwrap();
    assert_eq!(
        document.object(id(2, 0)).unwrap().value,
        Object::String(b"new".to_vec())
    );
    assert_eq!(document.stream_data(id(3, 0)).unwrap().unwrap(), b"abc");
    assert_eq!(document.object_count(), 3);
    assert_eq!(document.root(), id(1, 0));
    assert!(!document.trailer().contains_key(b"Root".as_slice()));
}

#[test]
fn root_changes_and_null_root_inherits_nearest_revision() {
    let mut file = base();
    append(
        &mut file,
        &[(2, 0, Some(b"<< /New true >>"))],
        "/Root 2 0 R",
        None,
    );
    append(&mut file, &[(1, 0, Some(b"<<>>"))], "/Root null", None);
    let document = open(&file.bytes).unwrap();
    assert_eq!(document.root(), id(2, 0));
    assert_eq!(
        document.trailer().get(b"Root".as_slice()),
        Some(&Object::Null)
    );
}

#[test]
fn stale_malformed_body_is_not_parsed() {
    let mut file = base();
    let start = file.offsets[2];
    // Corrupt the old object wrapper and content without changing any offsets.
    file.bytes[start..start + 12].fill(b'!');
    append(&mut file, &[(2, 0, Some(b"42"))], "", None);
    assert_eq!(
        open(&file.bytes).unwrap().object(id(2, 0)).unwrap().value,
        Object::Integer(42)
    );
}

#[test]
fn newer_free_entries_and_generations_do_not_resurrect_old_objects() {
    let mut file = base();
    append(&mut file, &[(2, 1, None)], "", None);
    let document = open(&file.bytes).unwrap();
    assert_eq!(document.object_count(), 2);
    assert!(matches!(
        document.object(id(2, 0)).unwrap_err().kind,
        DocumentErrorKind::MissingObject(_)
    ));
    append(&mut file, &[(2, 1, Some(b"99"))], "", None);
    let document = open(&file.bytes).unwrap();
    assert_eq!(
        document.object(id(2, 1)).unwrap().value,
        Object::Integer(99)
    );
    assert!(document.object(id(2, 0)).is_err());
}

#[test]
fn repeated_unchanged_offsets_across_revisions_are_not_aliases() {
    let mut file = base();
    let previous = file.latest;
    file.latest = file.bytes.len();
    file.bytes.extend_from_slice(format!("xref\n2 1\n{:010} 00000 n \ntrailer\n<< /Size 4 /Prev {previous} >>\nstartxref\n{}\n%%EOF\n", file.offsets[2], file.latest).as_bytes());
    assert_eq!(
        open(&file.bytes).unwrap().object(id(2, 0)).unwrap().value,
        Object::String(b"old".to_vec())
    );
}

#[test]
fn cycles_forward_prev_and_prev_inside_other_regions_fail() {
    for previous in [0, 1, usize::MAX, 999999, 10] {
        let mut file = base();
        append(&mut file, &[(2, 0, Some(b"1"))], "", Some(previous));
        assert!(open(&file.bytes).is_err(), "prev={previous}");
    }
    let mut file = base();
    let self_offset = file.bytes.len();
    append(&mut file, &[], "", Some(self_offset));
    assert!(open(&file.bytes).is_err());
    let mut file = base();
    let offset = file.offsets[3] + 10;
    append(&mut file, &[(2, 0, Some(b"1"))], "", Some(offset));
    assert!(open(&file.bytes).is_err());
}

#[test]
fn section_footer_mismatch_and_metadata_offsets_fail() {
    let mut file = base();
    let old_footer = format!("startxref\n{}", file.latest);
    let bad_footer = format!("startxref\n{}", file.latest + 1);
    let location = file
        .bytes
        .windows(old_footer.len())
        .position(|window| window == old_footer.as_bytes())
        .unwrap();
    file.bytes[location..location + old_footer.len()].copy_from_slice(bad_footer.as_bytes());
    append(&mut file, &[(2, 0, Some(b"1"))], "", None);
    assert!(open(&file.bytes).is_err());
    let mut file = base();
    let previous = file.latest;
    file.latest = file.bytes.len();
    file.bytes.extend_from_slice(format!("xref\n2 1\n{previous:010} 00000 n \ntrailer\n<< /Size 4 /Prev {previous} >>\nstartxref\n{}\n%%EOF\n", file.latest).as_bytes());
    assert!(
        open(&file.bytes)
            .unwrap_err()
            .to_string()
            .contains("inside xref metadata")
    );
}

#[test]
fn latest_size_and_root_requirements_cover_inherited_index() {
    let mut file = base();
    append(&mut file, &[(1, 1, None)], "", None);
    assert!(matches!(
        open(&file.bytes).unwrap_err().kind,
        DocumentErrorKind::MissingObject(_)
    ));
    let mut file = base();
    append(&mut file, &[(2, 0, Some(b"1"))], "", None);
    let suffix = &mut file.bytes[file.latest..];
    let at = suffix
        .windows(7)
        .position(|part| part == b"/Size 4")
        .unwrap();
    suffix[at + 6] = b'3';
    assert!(open(&file.bytes).unwrap_err().to_string().contains("Size"));
}

#[test]
fn revision_and_entry_budgets_are_global() {
    let mut file = base();
    append(&mut file, &[(2, 0, Some(b"1"))], "", None);
    append(&mut file, &[(2, 0, Some(b"2"))], "", None);
    let defaults = DocumentLimits::default();
    assert!(
        Document::open(
            &file.bytes,
            DocumentLimits {
                max_revisions: 3,
                max_xref_entries: 6,
                ..defaults
            }
        )
        .is_ok()
    );
    for limits in [
        DocumentLimits {
            max_revisions: 2,
            ..defaults
        },
        DocumentLimits {
            max_revisions: 0,
            ..defaults
        },
        DocumentLimits {
            max_xref_entries: 5,
            ..defaults
        },
    ] {
        assert!(matches!(
            Document::open(&file.bytes, limits).unwrap_err().kind,
            DocumentErrorKind::LimitExceeded(_)
        ));
    }
    assert!(
        Document::open(
            &file.bytes,
            DocumentLimits {
                max_objects: 15,
                ..defaults
            }
        )
        .is_err()
    );
}

#[test]
fn historical_encryption_and_hybrid_metadata_remain_explicitly_unsupported() {
    for extra in ["/Encrypt 1 0 R", "/XRefStm 10"] {
        let mut file = base();
        append(&mut file, &[(2, 0, Some(b"1"))], extra, None);
        assert!(matches!(
            open(&file.bytes).unwrap_err().kind,
            DocumentErrorKind::Unsupported(_)
        ));
    }
}

#[test]
fn inherited_trailer_metadata_and_version_are_available_without_rewriting_latest() {
    let mut file = base();
    let needle = b"/Root 1 0 R";
    let start = file
        .bytes
        .windows(needle.len())
        .position(|part| part == needle)
        .unwrap();
    file.bytes.splice(
        start..start + needle.len(),
        b"/Root 1 0 R /Info 2 0 R /ID [(old-a) (old-b)] /Custom true"
            .iter()
            .copied(),
    );
    append(
        &mut file,
        &[(2, 0, Some(b"<< /Author (test) >>"))],
        "/Info null /Custom null",
        None,
    );
    let document = open(&file.bytes).unwrap();
    assert_eq!(document.version(), "1.7");
    assert_eq!(document.trailer_chain().count(), 2);
    assert_eq!(
        document.trailer().get(b"Info".as_slice()),
        Some(&Object::Null)
    );
    let effective: std::collections::BTreeMap<_, _> =
        document.effective_trailer_entries().collect();
    assert_eq!(
        effective.get(b"Info".as_slice()),
        Some(&&Object::Reference(id(2, 0)))
    );
    assert_eq!(
        effective.get(b"Custom".as_slice()),
        Some(&&Object::Boolean(true))
    );
    assert!(matches!(
        effective.get(b"ID".as_slice()),
        Some(Object::Array(_))
    ));
    assert!(effective.contains_key(b"Prev".as_slice()));
}
