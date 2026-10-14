// SPDX-License-Identifier: AGPL-3.0-or-later
//! Native-confirmed joining boundaries and adversarial page decode budgets.
use miniz_oxide::deflate::compress_to_vec_zlib;
use tandem_pdf_engine::content::{ContentLimits, ContentOperand, ContentParser};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::page_content::{PageContentLimits, load_page_content};
use tandem_pdf_engine::pages::{PageLimits, collect_pages};

fn fixture(parts: &[Vec<u8>], contents: &str, filter: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut values = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 20] /Contents {contents} >>")
            .into_bytes(),
    ];
    values.extend(parts.iter().map(|part| {
        let mut value = format!("<< /Length {} {filter} >>\nstream\n", part.len()).into_bytes();
        value.extend_from_slice(part);
        value.extend_from_slice(b"\nendstream");
        value
    }));
    let mut offsets = Vec::new();
    for (index, value) in values.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(value);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", values.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Root 1 0 R /Size {} >>\nstartxref\n{xref}\n%%EOF\n",
            values.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

#[test]
fn one_stream_and_array_of_one_have_different_native_padding() {
    for (contents, expected, padding) in [
        ("4 0 R", b"q".as_slice(), None),
        ("[4 0 R]", b"q ".as_slice(), Some(1)),
    ] {
        let input = fixture(&[b"q".to_vec()], contents, "");
        let document = Document::open(&input, DocumentLimits::default()).unwrap();
        let pages = collect_pages(&document, PageLimits::default()).unwrap();
        let output = load_page_content(&document, &pages[0], PageContentLimits::default()).unwrap();
        assert_eq!(output.bytes, expected);
        assert_eq!(output.segments[0].range, 0..1);
        assert_eq!(output.segments[0].padding, padding);
        assert_eq!(output.usage.total_decoded_bytes, 1); // Inserted padding is joined storage, not decoder work.
    }
}

#[test]
fn native_space_join_preserves_comment_and_literal_continuity() {
    for (parts, expected, operand) in [
        (
            [b"%comment".as_slice(), b"q\nQ".as_slice()],
            b"%comment q\nQ ".as_slice(),
            None,
        ),
        (
            [b"(A".as_slice(), b"B) Tj".as_slice()],
            b"(A B) Tj ".as_slice(),
            Some(ContentOperand::String(b"A B".to_vec())),
        ),
        (
            [b"1".as_slice(), b"2 cm".as_slice()],
            b"1 2 cm ".as_slice(),
            Some(ContentOperand::Integer(1)),
        ),
    ] {
        let input = fixture(&parts.map(Vec::from), "[4 0 R 5 0 R]", "");
        let document = Document::open(&input, DocumentLimits::default()).unwrap();
        let pages = collect_pages(&document, PageLimits::default()).unwrap();
        let output = load_page_content(&document, &pages[0], PageContentLimits::default()).unwrap();
        assert_eq!(output.bytes, expected);
        let mut parser = ContentParser::new(&output.bytes, ContentLimits::default());
        let operation = parser.next_operation().unwrap().unwrap();
        if let Some(operand) = operand {
            assert_eq!(operation.operands[0], operand);
        } else {
            assert_eq!(operation.operator, b"Q");
            assert!(parser.next_operation().unwrap().is_none());
        }
    }
}

#[test]
fn empty_members_charge_padding_and_have_exact_zero_length_segments() {
    let input = fixture(&[vec![], b"q".to_vec(), vec![]], "[4 0 R 5 0 R 6 0 R]", "");
    let document = Document::open(&input, DocumentLimits::default()).unwrap();
    let pages = collect_pages(&document, PageLimits::default()).unwrap();
    let limits = PageContentLimits {
        max_joined_bytes: 4,
        max_decoded_work_bytes: 1,
        ..PageContentLimits::default()
    };
    let output = load_page_content(&document, &pages[0], limits).unwrap();
    assert_eq!(output.bytes, b" q  ");
    assert_eq!(
        output
            .segments
            .iter()
            .map(|part| (part.range.clone(), part.padding))
            .collect::<Vec<_>>(),
        [(0..0, Some(0)), (1..2, Some(2)), (3..3, Some(3))]
    );
    assert!(
        load_page_content(
            &document,
            &pages[0],
            PageContentLimits {
                max_joined_bytes: 3,
                ..limits
            }
        )
        .is_err()
    );
}

#[test]
fn repeated_predictor_stream_charges_raw_input_and_all_intermediates_again() {
    let payload = compress_to_vec_zlib(&[0, 1, 2, 3], 6);
    let input = fixture(
        std::slice::from_ref(&payload),
        "[4 0 R 4 0 R]",
        "/Filter /FlateDecode /DecodeParms << /Predictor 15 /Columns 3 >>",
    );
    let document = Document::open(&input, DocumentLimits::default()).unwrap();
    let pages = collect_pages(&document, PageLimits::default()).unwrap();
    let limits = PageContentLimits {
        max_streams: 2,
        max_referenced_encoded_bytes: payload.len() * 2,
        max_decoded_work_bytes: 14,
        max_joined_bytes: 8,
        ..PageContentLimits::default()
    };
    let output = load_page_content(&document, &pages[0], limits).unwrap();
    assert_eq!(output.bytes, [1, 2, 3, 32, 1, 2, 3, 32]);
    assert_eq!(output.usage.total_decoded_bytes, 14);
    assert_eq!(output.usage.encoded_bytes, payload.len() * 2);
    for limited in [
        PageContentLimits {
            max_streams: 1,
            ..limits
        },
        PageContentLimits {
            max_referenced_encoded_bytes: payload.len() * 2 - 1,
            ..limits
        },
        PageContentLimits {
            max_decoded_work_bytes: 13,
            ..limits
        },
        PageContentLimits {
            max_joined_bytes: 7,
            ..limits
        },
    ] {
        assert!(load_page_content(&document, &pages[0], limited).is_err());
    }
}

#[test]
fn forged_page_inventory_cannot_reorder_or_remove_contents() {
    let input = fixture(&[b"q".to_vec(), b"Q".to_vec()], "[4 0 R 5 0 R]", "");
    let document = Document::open(&input, DocumentLimits::default()).unwrap();
    let mut pages = collect_pages(&document, PageLimits::default()).unwrap();
    pages[0].contents.reverse();
    assert!(load_page_content(&document, &pages[0], PageContentLimits::default()).is_err());
    pages[0].contents.clear();
    assert!(load_page_content(&document, &pages[0], PageContentLimits::default()).is_err());
}
