// SPDX-License-Identifier: MIT
//! Independent resource-bound and document-boundary review regressions.
#[path = "support/pages.rs"]
mod pages;
use miniz_oxide::deflate::compress_to_vec_zlib;
use tandem_pdf_engine::app::pagetree::PageTree;
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::ObjectId;
use tandem_pdf_engine::stream::{Filter, StreamErrorKind, StreamLimits, decode_filters};

fn pdf(objects: &[Vec<u8>], trailer_extra: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n%\xff\xff\xff\xff\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R {trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}
fn body(value: &str) -> Vec<u8> {
    value.as_bytes().to_vec()
}
fn stream(payload: &[u8]) -> Vec<u8> {
    let mut bytes = format!("<< /Length {} >>\nstream\n", payload.len()).into_bytes();
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(b"\nendstream");
    bytes
}

#[test]
fn null_trailer_feature_entries_are_absent() {
    let input = pdf(
        &[
            body("<< /Type /Catalog /Pages 2 0 R >>"),
            body("<< /Type /Pages /Kids [] /Count 0 >>"),
        ],
        "/Encrypt null /Prev null /XRefStm null",
    );
    let doc = Document::open(&input, DocumentLimits::default()).unwrap();
    assert!(pages::page_ids(&doc).is_empty());
}

#[test]
fn binary_payload_markers_are_borrowed_without_interpretation() {
    let payload = b"\0\xff\nendstream\nendobj\nxref\n0 1\n%%EOF\n";
    let input = pdf(
        &[
            body("<< /Type /Catalog /Pages 2 0 R >>"),
            body("<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 20 30] >>"),
            body("<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>"),
            stream(payload),
        ],
        "",
    );
    let doc = Document::open(&input, DocumentLimits::default()).unwrap();
    let id = ObjectId::new(4, 0).unwrap();
    assert_eq!(doc.stream_data(id).unwrap().unwrap(), payload);
    let range = doc.object(id).unwrap().stream.clone().unwrap();
    assert_eq!(
        doc.stream_data(id).unwrap().unwrap().as_ptr(),
        input[range].as_ptr()
    );
    assert_eq!(pages::contents(&doc, pages::page_ids(&doc)[0]), [id]);
}

#[test]
fn stream_length_must_land_at_exact_payload_boundary() {
    for bad in [
        "<< /Length 0 >>\nstream\n% hidden payload\nendstream",
        "<< /Length 1 >>\nstream\nA  \nendstream",
        "<< /Length 999999 >>\nstream\nA\nendstream",
    ] {
        let input = pdf(&[body("<< /Type /Catalog >>"), body(bad)], "");
        assert!(Document::open(&input, DocumentLimits::default()).is_err());
    }
}

#[test]
fn filter_chain_bounds_intermediate_expansion_and_total_work() {
    // First stage expands five bytes to 128 'f' bytes plus '>'; the second
    // stage decodes those 129 bytes to 64 bytes of 0xff.
    let input = b"\x81f\0>\x80";
    let filters = [Filter::RunLength, Filter::AsciiHex];
    let limits = StreamLimits {
        max_encoded_bytes: 5,
        max_decoded_bytes: 129,
        max_filters: 2,
        max_total_decoded_bytes: 193,
    };
    assert_eq!(
        decode_filters(input, &filters, limits).unwrap(),
        vec![255; 64]
    );
    let error = decode_filters(
        input,
        &filters,
        StreamLimits {
            max_decoded_bytes: 128,
            ..limits
        },
    )
    .unwrap_err();
    assert_eq!(error.filter_index, Some(0));
    assert!(matches!(
        error.kind,
        StreamErrorKind::DecodedOutputLimit { .. }
    ));
    let error = decode_filters(
        input,
        &filters,
        StreamLimits {
            max_total_decoded_bytes: 192,
            ..limits
        },
    )
    .unwrap_err();
    assert_eq!(error.filter_index, Some(1));
    assert!(matches!(
        error.kind,
        StreamErrorKind::TotalOutputLimit { .. }
    ));
}

#[test]
fn flate_expansion_exact_limit_and_zero_output() {
    let expected = vec![b'A'; 65_536];
    let compressed = compress_to_vec_zlib(&expected, 6);
    let limits = StreamLimits {
        max_decoded_bytes: expected.len(),
        max_total_decoded_bytes: expected.len(),
        ..StreamLimits::default()
    };
    assert_eq!(
        decode_filters(&compressed, &[Filter::Flate], limits).unwrap(),
        expected
    );
    assert!(
        decode_filters(
            &compressed,
            &[Filter::Flate],
            StreamLimits {
                max_decoded_bytes: expected.len() - 1,
                ..limits
            }
        )
        .is_err()
    );
    let empty = compress_to_vec_zlib(b"", 6);
    assert!(
        decode_filters(
            &empty,
            &[Filter::Flate],
            StreamLimits {
                max_decoded_bytes: 0,
                max_total_decoded_bytes: 0,
                ..limits
            }
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn deterministic_adversarial_filters_stay_within_limits() {
    let limits = StreamLimits {
        max_encoded_bytes: 256,
        max_decoded_bytes: 128,
        max_filters: 2,
        max_total_decoded_bytes: 192,
    };
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    for case in 0..4000usize {
        let mut input = Vec::new();
        for _ in 0..case % 257 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            input.push(seed as u8);
        }
        for filter in [
            Filter::Flate,
            Filter::AsciiHex,
            Filter::Ascii85,
            Filter::RunLength,
        ] {
            match decode_filters(&input, &[filter], limits) {
                Ok(output) => assert!(output.len() <= limits.max_decoded_bytes),
                Err(error) => assert!(error.offset <= input.len()),
            }
        }
    }
}

#[test]
fn document_prefixes_and_mutations_do_not_panic() {
    let input = pdf(
        &[
            body("<< /Type /Catalog /Pages 2 0 R >>"),
            body("<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 20 30] >>"),
            body("<< /Type /Page /Parent 2 0 R >>"),
        ],
        "",
    );
    for end in 0..input.len() {
        if let Ok(doc) = Document::open(&input[..end], DocumentLimits::default()) {
            // The final line ending is optional; losing any structural byte is not.
            assert!(input[..end].ends_with(b"%%EOF"));
            assert_eq!(pages::page_ids(&doc).len(), 1);
        }
    }
    for index in 0..input.len() {
        for byte in [0, 255, b'0', b'%', b'\n'] {
            let mut changed = input.clone();
            changed[index] = byte;
            if let Ok(doc) = Document::open(&changed, DocumentLimits::default()) {
                let _ = PageTree::read(&doc);
            }
        }
    }
}
