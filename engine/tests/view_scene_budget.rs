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

/// A 595 × 842 page with `content`.
fn content_page(content: &str) -> ViewDocument {
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(content.as_bytes(), 9);
    let objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << >> /Contents 4 0 R >>"
            .to_vec(),
        stream("/Filter /FlateDecode", &packed),
    ];
    ViewDocument::open(pdf(&objects), "").expect("open")
}

#[test]
fn strokes_count_the_tiles_they_cross() {
    // The follow-up review: strokes were counted by their bounding box,
    // which overcounted thin lines (4,000 lines down the page at 400 %:
    // 426 MB estimated, refused, for 257 MB of WASM) and undercounted long
    // polylines (61 MB for 143). A stroke now counts 80 bytes for each
    // 4-pixel tile its outline crosses: here 822.5 tiles a line.
    let lines: String = (0..4000)
        .map(|i| {
            format!(
                "{:.2} 10 m {:.2} 832 l S\n",
                10.0 + 575.0 * f64::from(i) / 4000.0,
                10.0 + 575.0 * f64::from(i) / 4000.0
            )
        })
        .collect();
    let doc = content_page(&format!("0.3 w {lines}"));
    let (drawn, stats) =
        doc.render_with_stats(0, 4.0, RenderOptions::default(), RenderBudgets::default());
    assert!(drawn.is_ok());
    let tiles = 4000.0 * (822.0 * 4.0 / 4.0 + 0.5) * 80.0;
    assert!(
        stats.scene_bytes >= tiles && stats.scene_bytes < tiles + 2e6,
        "{stats:?}"
    );
    // 45-degree hatching at 400 % draws too (it was refused).
    let hatch: String = (0..4000)
        .map(|i| {
            let c = -842.0 + f64::from(i) * (595.0 + 842.0) / 4000.0;
            let (x0, y0) = (c.max(0.0), (-c).max(0.0));
            let (x1, y1) = ((c + 842.0).min(595.0), (595.0 - c).min(842.0));
            if x1 > x0 {
                format!("{x0:.2} {y0:.2} m {x1:.2} {y1:.2} l S\n")
            } else {
                String::new()
            }
        })
        .collect();
    let doc = content_page(&format!("0.3 w {hatch}"));
    let (drawn, _) =
        doc.render_with_stats(0, 4.0, RenderOptions::default(), RenderBudgets::default());
    assert!(drawn.is_ok());
    // A zigzag polyline in a band 20 points high is counted by its
    // crossings (each segment 2 × 20 points at 400 %: 20.5 tiles), not by
    // its band (5,000 tiles: the bounding box gave about 1 MB).
    let zigzag: String = (0..1000)
        .map(|k| {
            format!(
                "{} {} l ",
                10.0 + 0.5 * f64::from(k),
                if k % 2 == 0 { 400 } else { 420 }
            )
        })
        .collect();
    let doc = content_page(&format!("0.3 w 10 400 m {zigzag}S"));
    let (_, stats) =
        doc.render_with_stats(0, 4.0, RenderOptions::default(), RenderBudgets::default());
    assert!(stats.scene_bytes >= 1000.0 * 20.5 * 80.0, "{stats:?}");
}
