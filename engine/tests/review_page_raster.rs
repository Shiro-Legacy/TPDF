// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent page composition and failure-boundary pixel review.
use tandem_pdf_engine::{
    document::Document,
    geometry::Affine,
    page_glyph::{PageGlyphProgram, PageGlyphSession},
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
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(b"xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").bytes());
    out
}
fn trace(content: &str, width: f64, height: f64) -> PageGlyphProgram {
    let bytes = pdf(content, width, height);
    let doc = Document::open(&bytes, Default::default()).unwrap();
    PageGlyphSession::new(&doc, Default::default())
        .unwrap()
        .trace_page(0, Default::default())
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
    PageRasterSession::new(Default::default())
        .render(
            &trace(content, f64::from(width), f64::from(height)),
            target(width, height),
            Default::default(),
        )
        .unwrap()
}
fn rgba(rgb: &[[u8; 3]]) -> Vec<u8> {
    rgb.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect()
}
#[test]
fn repeated_fractional_clip_is_idempotent_not_coverage_squared() {
    let a = render("0 0 0.5 1 re W n 0 0 1 1 re f", 1, 1);
    let b = render("0 0 0.5 1 re W n 0 0 0.5 1 re W n 0 0 1 1 re f", 1, 1);
    assert_eq!(a, b);
    assert_eq!(a.pixels(), rgba(&[[128; 3]]));
}
#[test]
fn opaque_colors_compose_at_samples_before_single_resolve() {
    let p = render("1 0 0 rg 0 0 0.5 1 re f 0 0 1 rg 0.5 0 0.5 1 re f", 1, 1);
    assert_eq!(p.pixels(), rgba(&[[128, 0, 128]]));
    let q = render("1 0 0 rg 0 0 0.5 1 re f 0 0 1 rg 0 0 0.5 1 re f", 1, 1);
    assert_eq!(q.pixels(), rgba(&[[128, 128, 255]]));
}
#[test]
fn nested_clip_restore_recovers_parent_and_global_paint_order() {
    let p = render(
        "0 0 2 1 re W n q 0 0 1 1 re W n 1 0 0 rg 0 0 3 1 re f Q 0 0 1 rg 1 0 2 1 re f",
        3,
        1,
    );
    assert_eq!(p.pixels(), rgba(&[[255, 0, 0], [0, 0, 255], [255; 3]]));
}
#[test]
fn empty_clip_stays_empty_and_offcanvas_contours_can_fill_canvas() {
    assert_eq!(
        render("0 0 m 0 0 l W n -100 -100 200 200 re f", 1, 1).pixels(),
        rgba(&[[255; 3]])
    );
    assert_eq!(
        render("-100 -100 200 200 re f", 1, 1).pixels(),
        rgba(&[[0; 3]])
    );
}
#[test]
fn original_fractional_page_bounds_are_not_snapped_to_geometry_lattice() {
    let p = trace("-1 -1 5 5 re f", 1.1251, 1.);
    let image = PageRasterSession::new(Default::default())
        .render(&p, target(2, 1), Default::default())
        .unwrap();
    assert_eq!(image.pixels(), rgba(&[[0; 3], [191; 3]]));
}
#[test]
fn translated_page_bounds_clip_larger_canvas_at_original_sample_centers() {
    let p = trace("-1 -1 5 5 re f", 1., 1.);
    let image = PageRasterSession::new(Default::default())
        .render(
            &p,
            RasterTarget {
                width: 2,
                height: 2,
                page_to_device: Affine::from_pdf([1., 0., 0., 1., 0.2, 0.2]).unwrap(),
            },
            Default::default(),
        )
        .unwrap();
    assert_eq!(
        image.pixels(),
        rgba(&[[112; 3], [207; 3], [207; 3], [239; 3]])
    );
}
#[test]
fn fill_rule_holes_and_implicit_closure_have_independent_pixel_expectations() {
    let nonzero = render("0 0 3 3 re 1 1 1 1 re f", 3, 3);
    assert!(
        nonzero
            .pixels()
            .chunks_exact(4)
            .all(|v| v == [0, 0, 0, 255])
    );
    let evenodd = render("0 0 3 3 re 1 1 1 1 re f*", 3, 3);
    assert_eq!(&evenodd.pixels()[16..20], &[255; 4]);
    assert_eq!(&evenodd.pixels()[0..4], &[0, 0, 0, 255]);
    let implicit = render("0 0 m 1 0 l 1 1 l 0 1 l f", 1, 1);
    assert_eq!(implicit.pixels(), rgba(&[[0; 3]]));
}
#[test]
fn collinear_overshoot_curve_is_not_flattened_to_infinite_chord() {
    // A cubic with identical endpoints but distant controls traces a line out and
    // back. It has no fill area, but must require subdivision rather than zero nodes.
    let p = trace("0.5 0.5 m -4 0.5 5 0.5 0.5 0.5 c f", 1., 1.);
    let mut s = PageRasterSession::new(Default::default());
    let image = s.render(&p, target(1, 1), Default::default()).unwrap();
    assert_eq!(image.pixels(), rgba(&[[255; 3]]));
    assert!(s.usage().curve_nodes > 1);
    let mut s = PageRasterSession::new(Default::default());
    let limits = PageRasterLimits {
        max_subdivision_depth: 0,
        ..Default::default()
    };
    assert!(s.render(&p, target(1, 1), limits).is_err());
    assert_eq!(s.scan_usage().edges, 0);
}
#[test]
fn genuine_cubic_loop_is_not_discarded_by_equal_endpoints_or_zero_chord() {
    let p = trace("1 1 m 1 3 3 3 1 1 c f", 4., 4.);
    let mut s = PageRasterSession::new(Default::default());
    let image = s.render(&p, target(4, 4), Default::default()).unwrap();
    assert!(image.pixels().chunks_exact(4).any(|p| p[0] < 255));
    assert!(s.usage().curve_nodes > 1);
}
#[test]
fn unsupported_suffix_refuses_before_geometry_or_pixel_allocation() {
    for content in [
        "0 0 1 1 re f 0 w 0 0 m 1 1 l S",
        "0 0 1 1 re f 1 J [0 1] 0 d 0 0 m 1 1 l S",
        "0 0 1 1 re f 0 0 0 1 k 0 0 1 1 re f",
    ] {
        let p = trace(content, 1., 1.);
        let mut s = PageRasterSession::new(Default::default());
        let e = s.render(&p, target(1, 1), Default::default()).unwrap_err();
        assert!(e.event_index.is_some());
        if content.ends_with(" S") {
            assert!(matches!(
                e.kind,
                PageRasterErrorKind::Stroke(tandem_pdf_engine::stroke::StrokeError {
                    kind: tandem_pdf_engine::stroke::StrokeErrorKind::Unsupported(_),
                    ..
                })
            ));
        }
        assert_eq!(s.usage().allocated_bytes, 0);
        assert_eq!(s.usage().source_commands, 0);
        assert_eq!(s.scan_usage().edges, 0);
    }
}
#[test]
fn exact_output_and_work_budgets_then_one_less_fail_without_returning_partial_image() {
    let p = trace("0 0 2 1 re f", 2., 1.);
    let mut s = PageRasterSession::new(Default::default());
    let image = s.render(&p, target(2, 1), Default::default()).unwrap();
    let u = s.usage();
    for which in 0..6 {
        for less in [false, true] {
            let mut cap = PageRasterLimits::default();
            match which {
                0 => cap.max_flat_edges = u.flat_edges - usize::from(less),
                1 => cap.max_mask_bytes = u.mask_bytes - usize::from(less),
                2 => cap.max_sample_bytes = u.sample_bytes - usize::from(less),
                3 => cap.max_output_bytes = u.output_bytes - usize::from(less),
                4 => cap.max_allocated_bytes = u.allocated_bytes - usize::from(less),
                _ => cap.max_work = u.work - u64::from(less),
            }
            let mut session = PageRasterSession::new(Default::default());
            let result = session.render(&p, target(2, 1), cap);
            assert_eq!(result.is_ok(), !less, "{which} {less}");
            if let Ok(p) = result {
                assert_eq!(p, image);
            }
        }
    }
}
#[test]
fn cumulative_session_failure_preserves_prior_image_and_is_bounded_on_retry() {
    let p = trace("0 0 1 1 re f", 1., 1.);
    let mut limits = PageRasterSessionLimits::default();
    limits.raster.max_output_bytes = 4;
    let mut s = PageRasterSession::new(limits);
    let first = s.render(&p, target(1, 1), Default::default()).unwrap();
    for _ in 0..3 {
        assert!(s.render(&p, target(1, 1), Default::default()).is_err());
    }
    assert_eq!(first.pixels(), rgba(&[[0; 3]]));
    assert_eq!(s.usage().output_bytes, 4);
    assert_eq!(s.usage().page_attempts, 4);
}
#[test]
fn invalid_target_matrices_and_dimensions_fail_without_pixels() {
    let p = trace("0 0 1 1 re f", 1., 1.);
    for matrix in [
        [1., 0., 0., 2., 0., 0.],
        [-1., 0., 0., -1., 0., 0.],
        [1., 0.1, 0., 1., 0., 0.],
        [0., 0., 0., 0., 0., 0.],
    ] {
        let mut s = PageRasterSession::new(Default::default());
        assert!(
            s.render(
                &p,
                RasterTarget {
                    width: 1,
                    height: 1,
                    page_to_device: Affine::from_pdf(matrix).unwrap()
                },
                Default::default()
            )
            .is_err()
        );
        assert_eq!(s.usage().allocated_bytes, 0);
    }
}
fn glyph_program(content: &str, empty: bool) -> PageGlyphProgram {
    fn w(values: &[u16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }
    fn set(bytes: &mut [u8], at: usize, value: u16) {
        bytes[at..at + 2].copy_from_slice(&value.to_be_bytes())
    }
    let mut head = vec![0; 54];
    head[..4].copy_from_slice(&0x10000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5f0f3cf5u32.to_be_bytes());
    set(&mut head, 18, 1000);
    set(&mut head, 50, 1);
    let mut maxp = vec![0; 32];
    maxp[..4].copy_from_slice(&0x10000u32.to_be_bytes());
    set(&mut maxp, 4, 2);
    set(&mut maxp, 6, 4);
    set(&mut maxp, 8, 1);
    let mut glyf = w(&[1, 0, 0, 1000, 1000, 3, 0]);
    glyf.extend([1; 4]);
    glyf.extend(w(&[0, 1000, 0, (-1000i16) as u16, 0, 0, 1000, 0]));
    if empty {
        glyf.clear();
    }
    let mut loca = vec![0; 12];
    loca[8..12].copy_from_slice(&(glyf.len() as u32).to_be_bytes());
    let mut hhea = vec![0; 36];
    hhea[..4].copy_from_slice(&0x10000u32.to_be_bytes());
    set(&mut hhea, 34, 1);
    let mut cmap = w(&[0, 1, 3, 1]);
    cmap.extend(12u32.to_be_bytes());
    cmap.extend(w(&[
        4,
        32,
        0,
        4,
        4,
        1,
        0,
        65,
        65535,
        0,
        65,
        65535,
        (-64i16) as u16,
        1,
        0,
        0,
    ]));
    let tables = [
        (b"head", head),
        (b"maxp", maxp),
        (b"glyf", glyf),
        (b"loca", loca),
        (b"cmap", cmap),
        (b"hhea", hhea),
        (b"hmtx", w(&[1000, 0, 0])),
    ];
    let mut font = vec![0; 12 + 16 * tables.len()];
    font[..4].copy_from_slice(&0x10000u32.to_be_bytes());
    set(&mut font, 4, tables.len() as u16);
    for (i, (tag, data)) in tables.into_iter().enumerate() {
        let at = 12 + 16 * i;
        font[at..at + 4].copy_from_slice(tag);
        let len = font.len() as u32;
        font[at + 8..at + 12].copy_from_slice(&len.to_be_bytes());
        font[at + 12..at + 16].copy_from_slice(&(data.len() as u32).to_be_bytes());
        font.extend(data);
    }
    fn stream(data: &[u8]) -> Vec<u8> {
        let mut out = format!("<< /Length {} >>\nstream\n", data.len()).into_bytes();
        out.extend(data);
        out.extend(b"\nendstream");
        out
    }
    let objects=[b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 2 2] /Resources << /Font << /F 5 0 R >> >> /Contents 4 0 R >>".to_vec(),stream(content.as_bytes()),b"<< /Type /Font /Subtype /TrueType /BaseFont /ReviewSquare /Encoding /WinAnsiEncoding /FirstChar 65 /LastChar 65 /Widths [1000] /FontDescriptor 6 0 R >>".to_vec(),b"<< /Type /FontDescriptor /Flags 32 /FontFile2 7 0 R >>".to_vec(),stream(&font)];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(b"xref\n0 8\n0000000000 65535 f \n");
    for offset in offsets {
        out.extend(format!("{offset:010} 00000 n \n").bytes());
    }
    out.extend(format!("trailer\n<< /Size 8 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").bytes());
    let doc = Document::open(&out, Default::default()).unwrap();
    PageGlyphSession::new(&doc, Default::default())
        .unwrap()
        .trace_page(0, Default::default())
        .unwrap()
}
#[test]
fn glyph_and_path_share_event_order_and_target_transform_once() {
    let p = glyph_program(
        "1 0 0 rg BT /F 1 Tf (A) Tj ET 0 0 1 rg 0.5 0 0.5 1 re f",
        false,
    );
    let image = PageRasterSession::new(Default::default())
        .render(
            &p,
            RasterTarget {
                width: 4,
                height: 4,
                page_to_device: Affine::from_pdf([2., 0., 0., 2., 0., 0.]).unwrap(),
            },
            Default::default(),
        )
        .unwrap();
    for y in 0..4 {
        for x in 0..4 {
            let want = if y >= 2 && x == 0 {
                [255, 0, 0, 255]
            } else if y >= 2 && x == 1 {
                [0, 0, 255, 255]
            } else {
                [255; 4]
            };
            assert_eq!(&image.pixels()[4 * (y * 4 + x)..4 * (y * 4 + x) + 4], &want);
        }
    }
}
#[test]
fn invisible_and_empty_glyphs_preserve_white_without_flattened_geometry() {
    for (content, empty) in [
        ("0 0 0 1 k BT /F 1 Tf 3 Tr (A) Tj ET", false),
        ("BT /F 1 Tf (A) Tj ET", true),
    ] {
        let p = glyph_program(content, empty);
        let mut s = PageRasterSession::new(Default::default());
        let image = s
            .render(
                &p,
                target(2, 2),
                PageRasterLimits {
                    max_flat_edges: 0,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(image.pixels().iter().all(|v| *v == 255));
        assert_eq!(s.usage().flat_edges, 0);
        if !empty {
            assert_eq!(s.usage().source_commands, 0);
        }
    }
}
#[test]
fn half_lattice_tie_rounds_away_and_changes_exact_sample_membership() {
    // Right edge32.5/256 snaps to33/256 and includes the sample at32/256.
    let p = render("0 0 0.126953125 1 re f", 1, 1);
    assert_eq!(p.pixels(), rgba(&[[191; 3]]));
    // The neighboring exact lattice edge32/256 excludes its tied sample.
    let q = render("0 0 0.125 1 re f", 1, 1);
    assert_eq!(q.pixels(), rgba(&[[255; 3]]));
}
