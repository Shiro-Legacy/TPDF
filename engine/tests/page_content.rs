// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::lexer::{LexLimits, Lexer, TokenKind};
use tandem_pdf_engine::object::ObjectId;
use tandem_pdf_engine::page_content::{
    PageContent, PageContentErrorKind, PageContentLimits, load_page_content,
};
use tandem_pdf_engine::pages::{PageInfo, PageLimits, collect_pages};

fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}
fn pdf(contents: &str, streams: &[(&str, &[u8])], extra: &[String]) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] {contents} >>").into_bytes(),
    ];
    for (metadata, raw) in streams {
        let mut value = format!("<< /Length {} {metadata} >>\nstream\n", raw.len()).into_bytes();
        value.extend_from_slice(raw);
        value.extend_from_slice(b"\nendstream");
        objects.push(value);
    }
    objects.extend(extra.iter().map(|v| v.as_bytes().to_vec()));
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, value) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(value);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}
fn load(
    bytes: &[u8],
    limits: PageContentLimits,
) -> Result<PageContent, tandem_pdf_engine::page_content::PageContentError> {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    let pages = collect_pages(&doc, PageLimits::default()).unwrap();
    load_page_content(&doc, &pages[0], limits)
}
fn forged(contents: Vec<ObjectId>) -> PageInfo {
    PageInfo {
        id: id(3),
        media_box: [0., 0., 10., 10.],
        crop_box: [0., 0., 10., 10.],
        rotation: 0,
        user_unit: 1.,
        contents,
    }
}
#[test]
fn direct_and_array_of_one_preserve_distinct_final_padding() {
    for (contents, expected, padding) in [
        ("/Contents 4 0 R", b"q".as_slice(), None),
        ("/Contents [4 0 R]", b"q ".as_slice(), Some(1)),
    ] {
        let data = load(
            &pdf(contents, &[("", b"q")], &[]),
            PageContentLimits::default(),
        )
        .unwrap();
        assert_eq!(data.bytes, expected);
        assert_eq!(data.segments[0].object, id(4));
        assert_eq!(data.segments[0].range, 0..1);
        assert_eq!(data.segments[0].padding, padding);
        assert_eq!(data.usage.encoded_bytes, 1);
        assert_eq!(data.usage.total_decoded_bytes, 1);
    }
}
#[test]
fn empty_members_still_pad_and_segments_exclude_padding() {
    let data = load(
        &pdf(
            "/Contents [4 0 R 5 0 R 4 0 R]",
            &[("", b""), ("", b"q")],
            &[],
        ),
        PageContentLimits::default(),
    )
    .unwrap();
    assert_eq!(data.bytes, b" q  ");
    assert_eq!(
        data.segments
            .iter()
            .map(|v| (v.range.clone(), v.padding))
            .collect::<Vec<_>>(),
        [(0..0, Some(0)), (1..2, Some(2)), (3..3, Some(3))]
    );
}
#[test]
fn join_spaces_preserve_comments_literals_and_number_boundaries() {
    for (left, right, expected, token) in [
        (
            b"% comment".as_slice(),
            b"q\nQ".as_slice(),
            b"% comment q\nQ ".as_slice(),
            TokenKind::Keyword(b"Q".to_vec()),
        ),
        (
            b"(hel",
            b"lo) Tj",
            b"(hel lo) Tj ",
            TokenKind::String(b"hel lo".to_vec()),
        ),
        (b"1", b"2 m", b"1 2 m ", TokenKind::Integer(1)),
    ] {
        let data = load(
            &pdf("/Contents [4 0 R 5 0 R]", &[("", left), ("", right)], &[]),
            PageContentLimits::default(),
        )
        .unwrap();
        assert_eq!(data.bytes, expected);
        assert_eq!(
            Lexer::new(&data.bytes, LexLimits::default())
                .next_token()
                .unwrap()
                .unwrap()
                .kind,
            token
        );
    }
}
#[test]
fn absent_null_and_empty_array_have_no_bytes_or_usage() {
    for contents in ["", "/Contents null", "/Contents []", "/Contents 4 0 R"] {
        let extra = if contents.ends_with("R") {
            vec!["null".into()]
        } else {
            vec![]
        };
        let data = load(
            &pdf(contents, &[], &extra),
            PageContentLimits {
                max_streams: 0,
                max_referenced_encoded_bytes: 0,
                max_decoded_work_bytes: 0,
                max_joined_bytes: 0,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(data.bytes.is_empty() && data.segments.is_empty());
        assert_eq!(data.usage.total_decoded_bytes, 0);
    }
}
#[test]
fn repeated_references_charge_encoded_decode_and_joined_totals() {
    let bytes = pdf("/Contents [4 0 R 4 0 R]", &[("", b"abc")], &[]);
    let exact = PageContentLimits {
        max_streams: 2,
        max_referenced_encoded_bytes: 6,
        max_decoded_work_bytes: 6,
        max_joined_bytes: 8,
        ..Default::default()
    };
    let data = load(&bytes, exact).unwrap();
    assert_eq!(data.bytes, b"abc abc ");
    assert_eq!(data.usage.encoded_bytes, 6);
    assert_eq!(data.usage.total_decoded_bytes, 6);
    for limited in [
        PageContentLimits {
            max_streams: 1,
            ..exact
        },
        PageContentLimits {
            max_referenced_encoded_bytes: 5,
            ..exact
        },
        PageContentLimits {
            max_decoded_work_bytes: 5,
            ..exact
        },
        PageContentLimits {
            max_joined_bytes: 7,
            ..exact
        },
    ] {
        assert!(load(&bytes, limited).is_err());
    }
}
#[test]
fn multistage_work_budget_counts_intermediates_not_just_final_bytes() {
    let bytes = pdf(
        "/Contents [4 0 R 4 0 R]",
        &[("/Filter [/ASCIIHexDecode /RunLengthDecode]", b"0261626380>")],
        &[],
    );
    let exact = PageContentLimits {
        max_referenced_encoded_bytes: 22,
        max_decoded_work_bytes: 16,
        max_joined_bytes: 8,
        ..Default::default()
    };
    let data = load(&bytes, exact).unwrap();
    assert_eq!(data.bytes, b"abc abc ");
    assert_eq!(data.usage.total_decoded_bytes, 16);
    assert!(
        load(
            &bytes,
            PageContentLimits {
                max_decoded_work_bytes: 15,
                ..exact
            }
        )
        .is_err()
    );
    // The larger intermediate output is permitted despite a smaller joined cap.
    let single = pdf(
        "/Contents 4 0 R",
        &[("/Filter [/ASCIIHexDecode /RunLengthDecode]", b"0261626380>")],
        &[],
    );
    assert_eq!(
        load(
            &single,
            PageContentLimits {
                max_joined_bytes: 3,
                max_decoded_work_bytes: 8,
                ..exact
            }
        )
        .unwrap()
        .bytes,
        b"abc"
    );
}
#[test]
fn png_predictor_work_counts_inflate_and_reconstructed_outputs() {
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&[0, b'a', b'b', 0, b'c', b'd'], 6);
    let bytes = pdf(
        "/Contents 4 0 R",
        &[(
            "/Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 2 >>",
            &compressed,
        )],
        &[],
    );
    let exact = PageContentLimits {
        max_decoded_work_bytes: 10,
        max_joined_bytes: 4,
        ..Default::default()
    };
    let data = load(&bytes, exact).unwrap();
    assert_eq!(data.bytes, b"abcd");
    assert_eq!(data.usage.total_decoded_bytes, 10);
    assert!(
        load(
            &bytes,
            PageContentLimits {
                max_decoded_work_bytes: 9,
                ..exact
            }
        )
        .is_err()
    );
}
#[test]
fn individual_stream_limits_are_preserved() {
    let bytes = pdf("/Contents 4 0 R", &[("", b"abcd")], &[]);
    for which in 0..3 {
        let mut limits = PageContentLimits::default();
        match which {
            0 => limits.stream.max_encoded_bytes = 3,
            1 => limits.stream.max_decoded_bytes = 3,
            _ => limits.stream.max_total_decoded_bytes = 3,
        }
        assert!(matches!(
            load(&bytes, limits).unwrap_err().kind,
            PageContentErrorKind::Stream(_)
        ));
    }
}
#[test]
fn empty_stream_zero_budget_and_array_padding_budget() {
    let limits = PageContentLimits {
        max_referenced_encoded_bytes: 0,
        max_decoded_work_bytes: 0,
        max_joined_bytes: 0,
        ..Default::default()
    };
    assert!(
        load(&pdf("/Contents 4 0 R", &[("", b"")], &[]), limits)
            .unwrap()
            .bytes
            .is_empty()
    );
    assert!(load(&pdf("/Contents [4 0 R]", &[("", b"")], &[]), limits).is_err());
}
#[test]
fn resolves_indirect_arrays_but_refuses_mismatched_inventory() {
    let bytes = pdf("/Contents 5 0 R", &[("", b"q")], &["[4 0 R]".into()]);
    assert_eq!(
        load(&bytes, PageContentLimits::default()).unwrap().bytes,
        b"q "
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    for content in [vec![], vec![id(5)], vec![id(4), id(4)]] {
        assert!(matches!(
            load_page_content(&doc, &forged(content), PageContentLimits::default())
                .unwrap_err()
                .kind,
            PageContentErrorKind::Invalid(_)
        ));
    }
}
#[test]
fn indirect_cycle_and_reference_depth_are_bounded() {
    let bytes = pdf("/Contents 4 0 R", &[], &["5 0 R".into(), "4 0 R".into()]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    assert!(matches!(
        load_page_content(&doc, &forged(vec![]), PageContentLimits::default())
            .unwrap_err()
            .kind,
        PageContentErrorKind::ReferenceCycle(_)
    ));
    for n in [64, 65] {
        let mut extra = (4..4 + n - 1)
            .map(|v| format!("{} 0 R", v + 1))
            .collect::<Vec<_>>();
        extra.push("[]".into());
        let bytes = pdf("/Contents 4 0 R", &[], &extra);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        assert_eq!(
            load_page_content(&doc, &forged(vec![]), PageContentLimits::default()).is_ok(),
            n == 64
        );
    }
}
#[test]
fn nonstream_members_and_stream_aliases_are_refused() {
    for (contents, extra, ids) in [
        ("/Contents [4 0 R]", vec!["42".into()], vec![id(4)]),
        ("/Contents 5 0 R", vec!["4 0 R".into()], vec![id(4)]),
    ] {
        let streams = if contents.ends_with("5 0 R") {
            vec![("", b"q".as_slice())]
        } else {
            vec![]
        };
        let bytes = pdf(contents, &streams, &extra);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        assert!(matches!(
            load_page_content(&doc, &forged(ids), PageContentLimits::default())
                .unwrap_err()
                .kind,
            PageContentErrorKind::Invalid(_)
        ));
    }
}
#[test]
fn many_small_parts_join_with_bounded_geometric_capacity() {
    let refs = "4 0 R ".repeat(10_000);
    let bytes = pdf(&format!("/Contents [{refs}]"), &[("", b"q")], &[]);
    let limits = PageContentLimits {
        max_joined_bytes: 20_000,
        ..Default::default()
    };
    let data = load(&bytes, limits).unwrap();
    assert_eq!(data.bytes, b"q ".repeat(10_000));
    assert!(data.bytes.capacity() <= 20_000);
}
