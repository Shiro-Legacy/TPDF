// SPDX-License-Identifier: AGPL-3.0-or-later
//! What vello keeps of a page's fills, strokes and clips until the page is
//! composed (the redaction re-review, fix round 3): a page of letter-size
//! fills grew WASM memory by about 32 KB a fill until the render's scene
//! budget (then 1 GiB of estimate) refused it, so 28,000 fills, 62 KB of
//! file (or 2.3 KB of identical fills), reached 844 MB. The budget is now
//! 384 MiB of the same estimate (`RenderBudgets::scene_bytes`,
//! `hayro::MAX_RENDER_SCENE_BYTES`); proved by its counter
//! (`RenderStats::scene_bytes`).
use tandem_pdf_engine::view::{RenderBudgets, RenderOptions, ViewDocument, ViewError};

fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
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
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// A letter-size page of `fills` page-size grey fills.
fn fills_page(fills: usize) -> ViewDocument {
    let content = "0.5 g 0 0 612 792 re f\n".repeat(fills);
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(content.as_bytes(), 9);
    let objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Contents 4 0 R >>"
            .to_vec(),
        stream("/Filter /FlateDecode", &packed),
    ];
    ViewDocument::open(pdf(&objects), "").expect("open")
}

#[test]
fn page_size_fills_are_kept_within_a_384_mib_scene() {
    // A letter-size fill at 100 % is estimated at 38,304 bytes (199 tile
    // rows of 4 wide tiles): 10,000 draw, 11,000 pass 384 MiB and are
    // refused.
    let budget = f64::from(3u32 << 27);
    assert_eq!(RenderBudgets::default().scene_bytes, budget);
    let (drawn, stats) = fills_page(10000).render_with_stats(
        0,
        1.0,
        RenderOptions::default(),
        RenderBudgets::default(),
    );
    assert!(drawn.is_ok());
    assert!(
        stats.scene_bytes >= 10000.0 * 38304.0 && stats.scene_bytes <= budget,
        "{stats:?}"
    );
    let (refused, stats) = fills_page(11000).render_with_stats(
        0,
        1.0,
        RenderOptions::default(),
        RenderBudgets::default(),
    );
    assert!(matches!(refused, Err(ViewError::TooComplex(_))));
    assert!(stats.scene_bytes <= budget, "{stats:?}");
}
