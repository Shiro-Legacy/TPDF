// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::pages::{PageLimits, collect_pages};

fn pdf(objects: &[&str]) -> Vec<u8> {
    let mut result = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(result.len());
        result.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, object).as_bytes());
    }
    let xref = result.len();
    result.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        result.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    result.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    result
}

#[test]
fn inherited_geometry_crop_rotation_and_unit() {
    let bytes = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [10 20 210 320] /CropBox [0 0 180 280] /Rotate -90 >>",
        "<< /Type /Page /Parent 2 0 R /UserUnit 2 >>",
    ]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let pages = collect_pages(&doc, PageLimits::default()).unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].rotation, 270);
    assert_eq!(pages[0].crop_box, [10.0, 20.0, 180.0, 280.0]);
    assert_eq!(pages[0].display_bounds(), [0.0, 0.0, 520.0, 340.0]);
}

#[test]
fn siblings_keep_order_and_override_inheritance_independently() {
    let bytes = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 100 200] >>",
        "<< /Type /Page /Parent 2 0 R /Rotate 90 >>",
        "<< /Type /Page /Parent 2 0 R >>",
    ]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let pages = collect_pages(&doc, PageLimits::default()).unwrap();
    assert_eq!(
        pages.iter().map(|p| p.id.number()).collect::<Vec<_>>(),
        vec![3, 4]
    );
    assert_eq!(pages[0].display_bounds(), [0.0, 0.0, 200.0, 100.0]);
    assert_eq!(pages[1].display_bounds(), [0.0, 0.0, 100.0, 200.0]);
}

#[test]
fn rejects_wrong_counts_shared_nodes_and_bad_parent() {
    for (kids, count, parent) in [("3 0 R", 2, 2), ("3 0 R 3 0 R", 2, 2), ("3 0 R", 1, 1)] {
        let pages =
            format!("<< /Type /Pages /Kids [{kids}] /Count {count} /MediaBox [0 0 10 20] >>");
        let page = format!("<< /Type /Page /Parent {parent} 0 R >>");
        let bytes = pdf(&["<< /Type /Catalog /Pages 2 0 R >>", &pages, &page]);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        assert!(collect_pages(&doc, PageLimits::default()).is_err());
    }
}

#[test]
fn rejects_bad_geometry_and_rotation() {
    for extra in [
        "/MediaBox [0 0 0 20]",
        "/MediaBox [0 0 10 20] /CropBox [30 30 40 40]",
        "/MediaBox [0 0 10 20] /Rotate 45",
        "/MediaBox [0 0 10 20] /UserUnit 0",
        "/MediaBox [0 0 10 20] /UserUnit 75001",
        "",
    ] {
        let page = format!("<< /Type /Page /Parent 2 0 R {extra} >>");
        let bytes = pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            &page,
        ]);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        assert!(
            collect_pages(&doc, PageLimits::default()).is_err(),
            "{extra}"
        );
    }
}

#[test]
fn enforces_page_node_and_depth_limits() {
    let bytes = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 10 20] >>",
        "<< /Type /Page /Parent 2 0 R >>",
    ]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    for limits in [
        PageLimits {
            max_pages: 0,
            ..PageLimits::default()
        },
        PageLimits {
            max_nodes: 1,
            ..PageLimits::default()
        },
        PageLimits {
            max_depth: 0,
            ..PageLimits::default()
        },
    ] {
        assert!(collect_pages(&doc, limits).is_err());
    }
    assert_eq!(
        collect_pages(
            &doc,
            PageLimits {
                max_pages: 1,
                max_nodes: 2,
                max_depth: 1,
                max_total_content_refs: 0,
            }
        )
        .unwrap()
        .len(),
        1
    );
}

#[test]
fn validates_content_stream_references() {
    let bytes = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 10 20] >>",
        "<< /Type /Page /Parent 2 0 R /Contents [4 0 R] >>",
        "<< /Length 0 >>\nstream\n\nendstream",
    ]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let pages = collect_pages(&doc, PageLimits::default()).unwrap();
    assert_eq!(pages[0].contents[0].number(), 4);
}

#[test]
fn reversed_diagonal_boxes_are_normalized() {
    let bytes = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [100 200 0 0] >>",
        "<< /Type /Page /Parent 2 0 R /CropBox [90 190 10 20] >>",
    ]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let pages = collect_pages(&doc, PageLimits::default()).unwrap();
    assert_eq!(pages[0].media_box, [0.0, 0.0, 100.0, 200.0]);
    assert_eq!(pages[0].display_bounds(), [0.0, 0.0, 80.0, 170.0]);
}
