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
fn mixed_fills_keep_order_and_implicit_contour_closure() {
    let image = render(
        "1 0 0 rg 0 0 2 2 re f 0 0 1 rg 1 0 m 2 0 l 2 2 l 1 2 l f",
        2,
        2,
    );
    assert_eq!(
        image.pixels(),
        [
            255, 0, 0, 255, 0, 0, 255, 255, 255, 0, 0, 255, 0, 0, 255, 255
        ]
    );
}
#[test]
fn repeated_fractional_clip_is_idempotent_at_sample_resolution() {
    let once = render("0.2 0.2 0.6 0.6 re W n 0 0 1 1 re f", 1, 1);
    let twice = render(
        "0.2 0.2 0.6 0.6 re W n 0.2 0.2 0.6 0.6 re W n 0 0 1 1 re f",
        1,
        1,
    );
    assert_eq!(once, twice);
    assert_eq!(once.pixels(), [191, 191, 191, 255]);
}
#[test]
fn initial_page_boundary_is_classified_without_quantization() {
    let p = program("-2 -2 5 5 re f", 1.1251, 1.);
    let image = PageRasterSession::new(PageRasterSessionLimits::default())
        .render(&p, target(2, 1), PageRasterLimits::default())
        .unwrap();
    assert_eq!(image.pixels(), [0, 0, 0, 255, 191, 191, 191, 255]);
}
#[test]
fn unsupported_suffix_refuses_before_any_pixel_allocation() {
    for content in [
        "0 0 2 2 re f 0 w 0 0 m 2 2 l S",
        "0 0 2 2 re f 0 0 0 1 k 0 0 1 1 re f",
    ] {
        let p = program(content, 2., 2.);
        let mut session = PageRasterSession::new(PageRasterSessionLimits::default());
        let e = session
            .render(&p, target(2, 2), PageRasterLimits::default())
            .unwrap_err();
        assert!(matches!(
            e.kind,
            PageRasterErrorKind::Unsupported(_)
                | PageRasterErrorKind::Stroke(tandem_pdf_engine::stroke::StrokeError {
                    kind: tandem_pdf_engine::stroke::StrokeErrorKind::Unsupported(_),
                    ..
                })
        ));
        assert_eq!(session.usage().allocated_bytes, 0);
        assert_eq!(session.scan_usage().samples, 0);
    }
}
#[test]
fn collinear_cubic_overshoot_cannot_pass_zero_depth_flatness() {
    let p = program("0 0 m 3 0 -2 0 1 0 c 1 1 l 0 1 l f", 4., 2.);
    let mut session = PageRasterSession::new(PageRasterSessionLimits::default());
    let limits = PageRasterLimits {
        max_subdivision_depth: 0,
        ..PageRasterLimits::default()
    };
    assert!(matches!(
        session.render(&p, target(4, 2), limits).unwrap_err().kind,
        PageRasterErrorKind::Limit("curve subdivision depth")
    ));
    assert_eq!(session.scan_usage().samples, 0);
    assert_eq!(session.usage().curve_nodes, 1);
}
#[test]
fn exact_repeated_render_budget_and_owned_output() {
    let p = program("0 0 2 2 re f", 2., 2.);
    let mut limits = PageRasterSessionLimits::default();
    limits.raster.max_pixels = 4;
    let mut session = PageRasterSession::new(limits);
    let first = session
        .render(&p, target(2, 2), PageRasterLimits::default())
        .unwrap();
    assert!(
        session
            .render(&p, target(2, 2), PageRasterLimits::default())
            .is_err()
    );
    assert_eq!(session.usage().pixels, 4);
    drop(p);
    drop(session);
    assert!(first.pixels().chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
}
#[test]
fn actual_embedded_glyph_page_renders_without_optional_backend() {
    let bytes = include_bytes!("../../fixtures/p3/simple-subset.pdf");
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    let p = PageGlyphSession::new(&doc, PageGlyphSessionLimits::default())
        .unwrap()
        .trace_page(0, PageGlyphLimits::default())
        .unwrap();
    let bounds = p.page_program().geometry().bounds();
    let image = PageRasterSession::new(PageRasterSessionLimits::default())
        .render(
            &p,
            target(bounds.width().ceil() as u32, bounds.height().ceil() as u32),
            PageRasterLimits::default(),
        )
        .unwrap();
    assert!(image.pixels().chunks_exact(4).any(|p| p[0] != 255));
    assert!(image.pixels().chunks_exact(4).all(|p| p[3] == 255));
}

#[test]
fn distant_target_translation_cannot_silently_change_page_extent() {
    let p = program("", 1.1, 1.);
    let mut session = PageRasterSession::new(PageRasterSessionLimits::default());
    let mut t = target(2, 2);
    t.page_to_device = Affine::from_pdf([1., 0., 0., 1., 1e15, 0.]).unwrap();
    let e = session
        .render(&p, t, PageRasterLimits::default())
        .unwrap_err();
    assert_eq!(
        e.kind,
        PageRasterErrorKind::InvalidTarget("page transform loses extent precision")
    );
    assert_eq!(session.usage().allocated_bytes, 0);
    assert_eq!(session.scan_usage().samples, 0);
}
