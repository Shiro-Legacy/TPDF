// SPDX-License-Identifier: AGPL-3.0-or-later
#[path = "support/pages.rs"]
mod pages;
use miniz_oxide::deflate::compress_to_vec_zlib;
use tandem_pdf_engine::document::{Document, DocumentErrorKind, DocumentLimits};
use tandem_pdf_engine::object::ObjectId;

#[derive(Default)]
struct Options {
    compress: bool,
    header: Option<Vec<u8>>,
    n: Option<String>,
    first: Option<String>,
    length: Option<String>,
    extra: String,
}
struct File {
    bytes: Vec<u8>,
    xref: usize,
    rows: usize,
    container_offsets: Vec<usize>,
    decoded_bytes: usize,
    member_count: usize,
}
type Members = Vec<(u32, Vec<u8>)>;
fn standard() -> Members {
    vec![
        (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
        (2, b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec()),
        (
            3,
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents 4 0 R >>".to_vec(),
        ),
    ]
}
fn id(number: u32) -> ObjectId {
    ObjectId::new(number, 0).unwrap()
}
fn row(fields: [u64; 3]) -> Vec<u8> {
    let mut output = vec![fields[0] as u8];
    output.extend_from_slice(&(fields[1] as u32).to_be_bytes());
    output.extend_from_slice(&(fields[2] as u16).to_be_bytes());
    output
}
fn build(groups: &[Members], options: Options) -> File {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut entries = vec![[0, 0, 0]; 21];
    entries[0] = [0, 0, 65535];
    entries[4] = [1, bytes.len() as u64, 0];
    bytes.extend_from_slice(b"4 0 obj\n<< /Length 3 >>\nstream\nq Q\nendstream\nendobj\n");
    let mut container_offsets = Vec::new();
    let mut decoded_bytes = 0;
    let mut member_count = 0;
    for (group_index, members) in groups.iter().enumerate() {
        let container = 10 + group_index as u32;
        let mut header = Vec::new();
        let mut body = Vec::new();
        for (index, (number, value)) in members.iter().enumerate() {
            header.extend_from_slice(format!("{number} {} ", body.len()).as_bytes());
            body.extend_from_slice(value);
            body.push(b'\n');
            entries[*number as usize] = [2, u64::from(container), index as u64];
        }
        let header = options.header.clone().unwrap_or(header);
        let first = header.len();
        let decoded = [header, body].concat();
        decoded_bytes += decoded.len();
        member_count += members.len();
        let payload = if options.compress {
            compress_to_vec_zlib(&decoded, 6)
        } else {
            decoded
        };
        let position = bytes.len();
        container_offsets.push(position);
        entries[container as usize] = [1, position as u64, 0];
        bytes.extend_from_slice(
            format!(
                "{container} 0 obj\n<< /Type /ObjStm /N {} /First {} /Length {} {} {} >>\nstream\n",
                options
                    .n
                    .clone()
                    .unwrap_or_else(|| members.len().to_string()),
                options.first.clone().unwrap_or_else(|| first.to_string()),
                options
                    .length
                    .clone()
                    .unwrap_or_else(|| payload.len().to_string()),
                if options.compress {
                    "/Filter /FlateDecode"
                } else {
                    ""
                },
                options.extra
            )
            .as_bytes(),
        );
        bytes.extend_from_slice(&payload);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
    }
    let xref = bytes.len();
    entries[20] = [1, xref as u64, 0];
    let payload: Vec<u8> = entries.into_iter().flat_map(row).collect();
    bytes.extend_from_slice(
        format!(
            "20 0 obj\n<< /Type /XRef /Size 21 /Root 1 0 R /W [1 4 2] /Length {} >>\nstream\n",
            payload.len()
        )
        .as_bytes(),
    );
    let rows = bytes.len();
    bytes.extend_from_slice(&payload);
    bytes.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    File {
        bytes,
        xref,
        rows,
        container_offsets,
        decoded_bytes,
        member_count,
    }
}
fn open(bytes: &[u8]) -> Result<Document<'_>, tandem_pdf_engine::document::DocumentError> {
    Document::open(bytes, DocumentLimits::default())
}
fn set_row(file: &mut File, number: usize, fields: [u64; 3]) {
    file.bytes[file.rows + number * 7..file.rows + (number + 1) * 7].copy_from_slice(&row(fields));
}
fn append_classic(file: &mut File, number: u32, generation: u16, body: Option<&[u8]>) {
    let position = file.bytes.len();
    if let Some(body) = body {
        file.bytes
            .extend_from_slice(format!("{number} {generation} obj\n").as_bytes());
        file.bytes.extend_from_slice(body);
        file.bytes.extend_from_slice(b"\nendobj\n");
    }
    let previous = file.xref;
    file.xref = file.bytes.len();
    file.bytes.extend_from_slice(format!("xref\n{number} 1\n{:010} {generation:05} {} \ntrailer\n<< /Size 21 /Prev {previous} >>\nstartxref\n{}\n%%EOF\n",if body.is_some(){position}else{0},if body.is_some(){'n'}else{'f'},file.xref).as_bytes());
}

#[test]
fn identity_and_flate_members_are_owned_and_page_tree_resolves() {
    for compress in [false, true] {
        let file = build(
            &[standard()],
            Options {
                compress,
                ..Options::default()
            },
        );
        let document = open(&file.bytes).unwrap();
        assert_eq!(document.object_count(), 6);
        assert_eq!(pages::page_ids(&document).len(), 1);
        assert!(document.object(id(1)).unwrap().stream.is_none());
        assert_eq!(document.stream_data(id(4)).unwrap().unwrap(), b"q Q");
        assert!(document.stream_data(id(10)).unwrap().is_some());
    }
}

#[test]
fn member_indices_header_ids_and_offsets_are_validated() {
    let mut file = build(&[standard()], Options::default());
    set_row(&mut file, 1, [2, 10, 1]);
    assert!(
        open(&file.bytes)
            .unwrap_err()
            .to_string()
            .contains("index/object-number mismatch")
    );
    let mut file = build(&[standard()], Options::default());
    set_row(&mut file, 1, [2, 10, 3]);
    assert!(
        open(&file.bytes)
            .unwrap_err()
            .to_string()
            .contains("outside /N")
    );
    for header in [
        b"1 0 1 10 3 20 ".as_slice(),
        b"1 0 2 0 3 20 ",
        b"1 10 2 0 3 20 ",
        b"0 0 2 10 3 20 ",
        b"1 0 2 999999 3 9999999 ",
        b"1 0 2 10 3 20 junk ",
    ] {
        assert!(
            open(
                &build(
                    &[standard()],
                    Options {
                        header: Some(header.to_vec()),
                        ..Options::default()
                    }
                )
                .bytes
            )
            .is_err(),
            "{header:?}"
        );
    }
}

#[test]
fn compressed_or_missing_containers_fail_without_recursion() {
    let mut file = build(&[standard()], Options::default());
    set_row(&mut file, 10, [2, 10, 0]);
    assert!(matches!(
        open(&file.bytes).unwrap_err().kind,
        DocumentErrorKind::Unsupported(_)
    ));
    let mut file = build(&[standard()], Options::default());
    set_row(&mut file, 10, [2, 1, 0]);
    assert!(open(&file.bytes).is_err());
    let mut file = build(&[standard()], Options::default());
    set_row(&mut file, 1, [2, 19, 0]);
    assert!(matches!(
        open(&file.bytes).unwrap_err().kind,
        DocumentErrorKind::MissingObject(_)
    ));
    let mut file = build(&[standard()], Options::default());
    set_row(&mut file, 1, [2, 4, 0]);
    assert!(
        open(&file.bytes)
            .unwrap_err()
            .to_string()
            .contains("/Type /ObjStm")
    );
}

#[test]
fn metadata_requires_bounded_direct_integers() {
    for options in [
        Options {
            n: Some("0".into()),
            ..Options::default()
        },
        Options {
            n: Some("999999999".into()),
            ..Options::default()
        },
        Options {
            first: Some("999999999".into()),
            ..Options::default()
        },
        Options {
            first: Some("0".into()),
            ..Options::default()
        },
        Options {
            n: Some("1 0 R".into()),
            ..Options::default()
        },
        Options {
            first: Some("1 0 R".into()),
            ..Options::default()
        },
        Options {
            length: Some("1 0 R".into()),
            ..Options::default()
        },
        Options {
            extra: "/Filter 1 0 R".into(),
            ..Options::default()
        },
    ] {
        assert!(open(&build(&[standard()], options).bytes).is_err());
    }
}

#[test]
fn member_slices_reject_wrappers_streams_and_trailing_objects() {
    for invalid in [
        b"1 0 obj null endobj".as_slice(),
        b"<< /Length 0 >> stream\nendstream",
        b"null true",
        b"[",
        b"(unterminated",
    ] {
        let mut members = standard();
        members[0].1 = invalid.to_vec();
        assert!(
            open(&build(&[members], Options::default()).bytes).is_err(),
            "{invalid:?}"
        );
    }
}

#[test]
fn free_and_direct_replacements_leave_stale_compressed_members_unparsed() {
    let mut members = standard();
    members[2].1 = b"THIS IS NOT A PDF OBJECT".to_vec();
    let mut file = build(&[members], Options::default());
    append_classic(&mut file, 3, 1, None);
    let document = open(&file.bytes).unwrap();
    assert!(document.object(id(3)).is_err());
    append_classic(
        &mut file,
        3,
        1,
        Some(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 30] >>"),
    );
    let document = open(&file.bytes).unwrap();
    assert!(document.object(id(3)).is_err());
    assert!(document.object(ObjectId::new(3, 1).unwrap()).is_ok());
    let mut members = standard();
    members[0].1 = b"invalid stale root".to_vec();
    let mut file = build(&[members], Options::default());
    append_classic(&mut file, 1, 0, Some(b"<< /Type /Catalog /Pages 2 0 R >>"));
    assert!(open(&file.bytes).is_ok());
}

#[test]
fn normal_stream_length_can_resolve_a_compressed_integer() {
    let mut members = standard();
    members.push((7, b"3".to_vec()));
    let mut file = build(&[members], Options::default());
    append_classic(
        &mut file,
        4,
        0,
        Some(b"<< /Length 7 0 R >>\nstream\nq Q\nendstream"),
    );
    assert_eq!(
        open(&file.bytes)
            .unwrap()
            .stream_data(id(4))
            .unwrap()
            .unwrap(),
        b"q Q"
    );
}

#[test]
fn each_container_is_decoded_once_and_global_limits_cover_all_members() {
    let file = build(
        &[standard()],
        Options {
            compress: true,
            ..Options::default()
        },
    );
    let total = 21 * 7 + file.decoded_bytes;
    let limits = DocumentLimits {
        max_object_streams: 1,
        max_compressed_members: file.member_count,
        max_metadata_decoded_bytes: total,
        ..DocumentLimits::default()
    };
    assert!(Document::open(&file.bytes, limits).is_ok());
    for limited in [
        DocumentLimits {
            max_object_streams: 0,
            ..limits
        },
        DocumentLimits {
            max_compressed_members: 2,
            ..limits
        },
        DocumentLimits {
            max_metadata_decoded_bytes: total - 1,
            ..limits
        },
    ] {
        assert!(Document::open(&file.bytes, limited).is_err());
    }
    let members = standard();
    let file = build(
        &[members[..1].to_vec(), members[1..].to_vec()],
        Options::default(),
    );
    assert!(open(&file.bytes).is_ok());
    assert!(
        Document::open(
            &file.bytes,
            DocumentLimits {
                max_object_streams: 1,
                ..DocumentLimits::default()
            }
        )
        .is_err()
    );
    assert!(
        Document::open(
            &file.bytes,
            DocumentLimits {
                max_compressed_members: 2,
                ..DocumentLimits::default()
            }
        )
        .is_err()
    );
}

#[test]
fn compressed_parse_errors_name_container_and_decoded_offset() {
    let mut members = standard();
    members[1].1 = b"[".to_vec();
    let file = build(&[members], Options::default());
    let error = open(&file.bytes).unwrap_err();
    assert_eq!(error.offset, file.container_offsets[0]);
    assert!(
        matches!(error.kind,DocumentErrorKind::CompressedParse{container,member,..} if container==id(10)&&member==id(2))
    );
}

#[test]
fn generated_container_mutations_and_truncations_do_not_panic() {
    let file = build(
        &[standard()],
        Options {
            compress: true,
            ..Options::default()
        },
    );
    for end in 0..file.bytes.len() {
        let _ = open(&file.bytes[..end]);
    }
    for position in (0..file.bytes.len()).step_by(3) {
        for byte in [0, b'%', b'9', 255] {
            let mut bytes = file.bytes.clone();
            bytes[position] = byte;
            let _ = open(&bytes);
        }
    }
}

#[test]
fn first_cannot_artificially_truncate_a_header_integer() {
    let members = vec![(1, b"900".to_vec()), (2, b"0".to_vec())];
    let file = build(
        &[members],
        Options {
            header: Some(b"1 0 2 2".to_vec()),
            ..Options::default()
        },
    );
    assert!(
        open(&file.bytes)
            .unwrap_err()
            .to_string()
            .contains("First splits a header token")
    );
}
