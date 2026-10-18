// SPDX-License-Identifier: MIT
use miniz_oxide::deflate::compress_to_vec_zlib;
use tandem_pdf_engine::document::{Document, DocumentErrorKind, DocumentLimits};
use tandem_pdf_engine::object::{Object, ObjectId};

fn id(number: u32) -> ObjectId {
    ObjectId::new(number, 0).unwrap()
}
fn prefix() -> (Vec<u8>, usize, usize) {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let root = bytes.len();
    bytes.extend_from_slice(b"1 0 obj\n<<>>\nendobj\n");
    let xref = bytes.len();
    (bytes, root, xref)
}
fn row(widths: [usize; 3], fields: [u64; 3]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (width, field) in widths.into_iter().zip(fields) {
        bytes.extend_from_slice(&field.to_be_bytes()[8 - width..]);
    }
    bytes
}
fn rows() -> Vec<u8> {
    let (_, root, xref) = prefix();
    [
        row([1, 4, 2], [0, 0, 65535]),
        row([1, 4, 2], [1, root as u64, 0]),
        row([1, 4, 2], [1, xref as u64, 0]),
    ]
    .concat()
}
fn xref_file(metadata: &str, data: &[u8]) -> Vec<u8> {
    let (mut bytes, _, xref) = prefix();
    bytes.extend_from_slice(
        format!(
            "2 0 obj\n<< /Type /XRef /Size 3 /Root 1 0 R /Length {} {metadata} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    bytes
}
fn open(bytes: &[u8]) -> Result<Document<'_>, tandem_pdf_engine::document::DocumentError> {
    Document::open(bytes, DocumentLimits::default())
}

#[test]
fn pure_unfiltered_and_flate_xref_streams_register_bootstrap_once() {
    for (metadata, payload) in [
        ("/W [1 4 2]", rows()),
        (
            "/W [1 4 2] /Filter /FlateDecode",
            compress_to_vec_zlib(&rows(), 6),
        ),
    ] {
        let bytes = xref_file(metadata, &payload);
        let document = open(&bytes).unwrap();
        assert_eq!(document.object_count(), 2);
        assert_eq!(document.root(), id(1));
        assert_eq!(document.stream_data(id(2)).unwrap().unwrap(), payload);
        assert!(matches!(
            document.object(id(2)).unwrap().value,
            Object::Dictionary(_)
        ));
    }
}

#[test]
fn sparse_index_can_omit_only_verified_bootstrap_self_entry() {
    let bytes = xref_file("/W [1 4 2] /Index [0 2]", &rows()[..14]);
    assert_eq!(open(&bytes).unwrap().object_count(), 2);
    let bytes = xref_file(
        "/W [1 4 2] /Index [0 1 2 1]",
        &[&rows()[..7], &rows()[14..]].concat(),
    );
    assert!(open(&bytes).is_err()); // Missing ordinary objects are never invented.
    let bytes = xref_file(
        "/W [1 4 2] /Index [2 1 0 2]",
        &[&rows()[14..], &rows()[..14]].concat(),
    );
    assert!(open(&bytes).is_ok()); // Index order need not be sorted.
}

#[test]
fn conflicting_self_entries_are_rejected() {
    for fields in [[0, 0, 0], [1, 9, 0], [1, prefix().2 as u64, 1]] {
        let mut payload = rows();
        payload[14..].copy_from_slice(&row([1, 4, 2], fields));
        assert!(
            open(&xref_file("/W [1 4 2]", &payload))
                .unwrap_err()
                .to_string()
                .contains("conflicting")
        );
    }
}

fn classic_prefix() -> (Vec<u8>, usize, usize) {
    let (mut bytes, root, _) = prefix();
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 2\n0000000000 65535 f \n{root:010} 00000 n \ntrailer\n<< /Size 2 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    (bytes, root, xref)
}
fn append_xref(
    bytes: &mut Vec<u8>,
    number: u32,
    size: u32,
    previous: usize,
    metadata: &str,
    payload: &[u8],
) -> usize {
    let xref = bytes.len();
    bytes.extend_from_slice(format!("{number} 0 obj\n<< /Type /XRef /Size {size} /Prev {previous} /Length {} {metadata} >>\nstream\n",payload.len()).as_bytes());
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    xref
}

#[test]
fn mixed_classic_stream_chains_and_zero_width_defaults() {
    let (mut bytes, root, previous) = classic_prefix();
    // W[0]=0 defaults to type1, W[2]=0 defaults to generation0.
    append_xref(
        &mut bytes,
        2,
        3,
        previous,
        "/W [0 4 0] /Index [1 1]",
        &row([0, 4, 0], [1, root as u64, 0]),
    );
    let document = open(&bytes).unwrap();
    assert_eq!(document.root(), id(1));
    assert_eq!(document.object_count(), 2);
    let previous = bytes.windows(9).rposition(|p| p == b"startxref").unwrap();
    let offset = std::str::from_utf8(&bytes[previous + 9..])
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let replacement = bytes.len();
    bytes.extend_from_slice(b"1 0 obj\n<< /Updated true >>\nendobj\n");
    let latest = bytes.len();
    bytes.extend_from_slice(format!("xref\n1 1\n{replacement:010} 00000 n \ntrailer\n<< /Size 3 /Prev {offset} >>\nstartxref\n{latest}\n%%EOF\n").as_bytes());
    assert!(
        matches!(open(&bytes).unwrap().object(id(1)).unwrap().value,Object::Dictionary(ref value) if value.get(b"Updated".as_slice())==Some(&Object::Boolean(true)))
    );
}

#[test]
fn metadata_budget_is_cumulative_across_stream_revisions() {
    let mut bytes = xref_file("/W [1 4 2]", &rows());
    let previous = prefix().2;
    append_xref(
        &mut bytes,
        3,
        4,
        previous,
        "/W [1 4 2] /Index [1 1]",
        &row([1, 4, 2], [1, prefix().1 as u64, 0]),
    );
    let limits = DocumentLimits {
        max_metadata_decoded_bytes: 28,
        ..DocumentLimits::default()
    };
    assert_eq!(Document::open(&bytes, limits).unwrap().object_count(), 3);
    assert!(
        Document::open(
            &bytes,
            DocumentLimits {
                max_metadata_decoded_bytes: 27,
                ..limits
            }
        )
        .is_err()
    );
    assert!(
        Document::open(
            &bytes,
            DocumentLimits {
                max_xref_entries: 4,
                ..limits
            }
        )
        .is_err()
    ); // 3old+1new+1injected
}

#[test]
fn png_predictor_metadata_counts_inflate_and_reconstruction() {
    let mut predicted = Vec::new();
    for row in rows().chunks_exact(7) {
        predicted.push(0);
        predicted.extend_from_slice(row);
    }
    let bytes = xref_file(
        "/W [1 4 2] /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 7 >>",
        &compress_to_vec_zlib(&predicted, 6),
    );
    let limits = DocumentLimits {
        max_metadata_decoded_bytes: 45,
        ..DocumentLimits::default()
    };
    assert!(Document::open(&bytes, limits).is_ok());
    assert!(
        Document::open(
            &bytes,
            DocumentLimits {
                max_metadata_decoded_bytes: 44,
                ..limits
            }
        )
        .is_err()
    );
}

#[test]
fn invalid_widths_and_index_ranges_fail_before_large_allocations() {
    for metadata in [
        "/W [0 0 0]",
        "/W [1 4]",
        "/W [1 4 9]",
        "/W [-1 4 2]",
        "/W [1 4 2.0]",
        "/W [1 4 2] /Index [0]",
        "/W [1 4 2] /Index []",
        "/W [1 4 2] /Index [0 2 1 2]",
        "/W [1 4 2] /Index [0 4]",
        "/W [1 4 2] /Index [1 -1]",
        "/W [1 4 2] /Index [0 9223372036854775807]",
        "/W [1 4 2] /Index [0 0]",
    ] {
        assert!(open(&xref_file(metadata, &rows())).is_err(), "{metadata}");
    }
}

#[test]
fn decoded_rows_must_have_exact_length() {
    assert!(open(&xref_file("/W [1 4 2]", &rows()[..20])).is_err());
    let mut payload = rows();
    payload.push(0);
    assert!(open(&xref_file("/W [1 4 2]", &payload)).is_err());
    let bytes = xref_file("/W [1 4 2] /Filter /FlateDecode", b"invalid zlib");
    assert!(matches!(
        open(&bytes).unwrap_err().kind,
        DocumentErrorKind::MetadataDecode(_)
    ));
    let bytes = xref_file(
        "/W [1 4 2] /Filter /FlateDecode",
        &compress_to_vec_zlib(&vec![0; 10000], 6),
    );
    assert!(
        Document::open(
            &bytes,
            DocumentLimits {
                max_metadata_decoded_bytes: 30,
                ..DocumentLimits::default()
            }
        )
        .is_err()
    );
}

#[test]
fn entry_type_offset_and_generation_ranges_are_checked() {
    let mut payload = rows();
    payload[7] = 2;
    assert!(open(&xref_file("/W [1 4 2]", &payload)).is_err()); // Type2 now requires a valid ObjStm target.
    payload[7] = 3;
    assert!(matches!(
        open(&xref_file("/W [1 4 2]", &payload)).unwrap_err().kind,
        DocumentErrorKind::Invalid(_)
    ));
    for fields in [[1, u64::MAX, 0], [1, 9, 65536], [0, u64::MAX, 0]] {
        let payload = [row([1, 8, 8], [0, 0, 65535]), row([1, 8, 8], fields)].concat();
        assert!(open(&xref_file("/W [1 8 8] /Index [0 2]", &payload)).is_err());
    }
}

#[test]
fn indirect_bootstrap_metadata_and_unknown_filters_are_explicitly_refused() {
    for metadata in [
        "/W 1 0 R",
        "/W [1 1 0 R 2]",
        "/W [1 4 2] /Index 1 0 R",
        "/W [1 4 2] /Filter 1 0 R",
        "/W [1 4 2] /Filter [1 0 R]",
        "/W [1 4 2] /Filter /LZWDecode",
        "/W [1 4 2] /Filter /FlateDecode /DecodeParms 1 0 R",
        "/W [1 4 2] /Filter /FlateDecode /DecodeParms << /Columns 1 0 R >>",
    ] {
        assert!(
            matches!(
                open(&xref_file(metadata, &rows())).unwrap_err().kind,
                DocumentErrorKind::Unsupported(_)
            ),
            "{metadata}"
        );
    }
}

#[test]
fn xref_metadata_mutations_and_truncations_do_not_panic() {
    let bytes = xref_file(
        "/W [1 4 2] /Filter /FlateDecode",
        &compress_to_vec_zlib(&rows(), 6),
    );
    for end in 0..bytes.len() {
        let _ = open(&bytes[..end]);
    }
    for position in 0..bytes.len() {
        for byte in [0, b'%', b'9', 255] {
            let mut changed = bytes.clone();
            changed[position] = byte;
            let _ = open(&changed);
        }
    }
}
