// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::{
    document::{Document, DocumentLimits},
    geometry::Affine,
    page_glyph::{PageGlyphLimits, PageGlyphProgram, PageGlyphSession, PageGlyphSessionLimits},
    page_raster::*,
};
fn pdf(content: &str, width: f64, height: f64) -> Vec<u8> {
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Contents 4 0 R >>")
            .into_bytes(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        )
        .into_bytes(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(object);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in &offsets[1..] {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    out
}
fn program(content: &str, width: f64, height: f64) -> PageGlyphProgram {
    let bytes = pdf(content, width, height);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    PageGlyphSession::new(&doc, PageGlyphSessionLimits::default())
        .unwrap()
        .trace_page(0, PageGlyphLimits::default())
        .unwrap()
}
fn target(width: u32, height: u32) -> RasterTarget {
    RasterTarget {
        width,
        height,
        page_to_device: Affine::from_pdf([1., 0., 0., 1., 0., 0.]).unwrap(),
    }
}
fn render(content: &str, width: u32, height: u32) -> PageRasterImage {
    let p = program(content, f64::from(width), f64::from(height));
    PageRasterSession::new(PageRasterSessionLimits::default())
        .render(&p, target(width, height), PageRasterLimits::default())
        .unwrap()
}
#[test]
fn stroke_uses_stroke_color_clip_and_mixed_paint_order() {
    let image = render(
        "1 0 0 rg 0 0 8 4 re f q 0 0 4 4 re W n 0 0 1 RG 2 w 1 2 m 7 2 l S Q 0 1 0 rg 6 0 2 4 re f",
        8,
        4,
    );
    let mut expected = Vec::new();
    for y in 0..4 {
        for x in 0..8 {
            expected.extend(if x >= 6 {
                [0, 255, 0, 255]
            } else if (1..4).contains(&x) && (1..3).contains(&y) {
                [0, 0, 255, 255]
            } else {
                [255, 0, 0, 255]
            });
        }
    }
    assert_eq!(image.pixels(), expected);
}
#[test]
fn stroke_width_is_expanded_before_shear_scale_and_reflection() {
    for (a, e) in [(2., 0.), (-2., 8.)] {
        let content = format!("{a} 0 1 1 {e} 0 cm 1 w 1 1 m 3 1 l S");
        let image = render(&content, 8, 3);
        let mut expected = Vec::new();
        for y in 0..3 {
            for x in 0..8 {
                let mut inside = 0u16;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let device_x = f64::from(x) + (f64::from(sx) + 0.5) / 4.;
                        let user_y = 3. - (f64::from(y) + (f64::from(sy) + 0.5) / 4.);
                        let user_x = (device_x - user_y - e) / a;
                        // Device scan intervals own the left boundary. Reflection
                        // swaps which original endpoint owns an exact sample tie.
                        let along = if a > 0. {
                            (1. ..3.).contains(&user_x)
                        } else {
                            user_x > 1. && user_x <= 3.
                        };
                        inside += u16::from(along && (0.5..1.5).contains(&user_y));
                    }
                }
                let v = ((255 * (16 - inside) + 8) / 16) as u8;
                expected.extend([v, v, v, 255]);
            }
        }
        assert_eq!(image.pixels(), expected);
    }
}
#[test]
fn failed_curve_suffix_keeps_spent_stroke_work_without_scanning_prefix() {
    let p = program("2 w 1 2 m 7 2 l S 1 w 1 1 m 2 4 6 4 7 1 c S", 8., 6.);
    let mut session = PageRasterSession::new(Default::default());
    let mut limits = PageRasterLimits::default();
    limits.stroke.max_curve_nodes = 1;
    assert!(session.render(&p, target(8, 6), limits).is_err());
    let first = session.stroke_usage();
    assert!(first.work > 0);
    assert!(first.components > 0);
    assert!(first.allocated_bytes > 0);
    assert_eq!(session.scan_usage().samples, 0);
    assert!(session.render(&p, target(8, 6), limits).is_err());
    assert_eq!(session.stroke_usage().work, first.work * 2);
    assert_eq!(
        session.stroke_usage().allocated_bytes,
        first.allocated_bytes * 2
    );
    assert_eq!(session.scan_usage().samples, 0);
}
#[test]
fn page_and_session_stroke_caps_both_apply_across_events() {
    let p = program("1 w 1 1 m 7 1 l S 1 3 m 7 3 l S", 8., 4.);
    let mut limits = PageRasterSessionLimits::default();
    limits.raster.stroke.max_attempts = 1;
    let mut session = PageRasterSession::new(limits);
    assert!(
        session
            .render(&p, target(8, 4), Default::default())
            .is_err()
    );
    assert_eq!(session.stroke_usage().attempts, 1);
    assert_eq!(session.scan_usage().samples, 0);
    let p = program("1 w 1 1 m 7 1 l S", 8., 4.);
    let mut limits = PageRasterSessionLimits::default();
    limits.stroke.max_attempts = 1;
    let mut session = PageRasterSession::new(limits);
    let first = session
        .render(&p, target(8, 4), Default::default())
        .unwrap();
    assert!(
        session
            .render(&p, target(8, 4), Default::default())
            .is_err()
    );
    assert_eq!(session.stroke_usage().attempts, 1);
    drop(session);
    drop(p);
    assert!(first.pixels().chunks_exact(4).any(|p| p[0] < 255));
}

#[test]
fn scanner_failure_retains_completed_stroke_expansion_usage() {
    let p = program("1 w 1 1 m 7 1 l S", 8., 4.);
    let mut limits = PageRasterSessionLimits::default();
    limits.scan.max_samples = 0;
    let mut session = PageRasterSession::new(limits);
    let error = session
        .render(&p, target(8, 4), Default::default())
        .unwrap_err();
    assert!(matches!(error.kind, PageRasterErrorKind::Scan(_)));
    assert_eq!(session.stroke_usage().attempts, 1);
    assert!(session.stroke_usage().components > 0);
    assert!(session.stroke_usage().allocated_bytes > 0);
    assert!(session.usage().allocated_bytes > 0);
    assert_eq!(session.scan_usage().samples, 0);
}
