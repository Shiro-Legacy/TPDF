#![cfg(feature = "raster")]
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent contour continuation, clip accounting and target-space checks.
use tandem_pdf_engine::content::ContentLimits;
use tandem_pdf_engine::geometry::{Affine, Rect};
use tandem_pdf_engine::path::{InitialClip, PathInitialState, PathLimits, interpret_path_content};
use tandem_pdf_engine::raster::{
    RasterErrorKind, RasterLimits, RasterTarget, prepare_orthogonal_raster,
    render_orthogonal_raster,
};
fn target() -> RasterTarget {
    RasterTarget {
        width: 8,
        height: 8,
        page_to_device: Affine::from_pdf([1., 0., 0., 1., 0., 0.]).unwrap(),
    }
}
fn program(s: &str, clip: InitialClip) -> tandem_pdf_engine::path::PathProgram {
    interpret_path_content(
        s.as_bytes(),
        PathInitialState::new(Affine::from_pdf([1., 0., 0., 1., 0., 0.]).unwrap(), clip),
        ContentLimits::default(),
        PathLimits::default(),
    )
    .unwrap()
}
fn black(i: &tandem_pdf_engine::raster::RasterImage) -> usize {
    i.pixels()
        .chunks_exact(4)
        .filter(|p| *p == [0, 0, 0, 255])
        .count()
}
#[test]
fn closed_contour_followed_by_lines_starts_a_new_contour_at_original_start() {
    for (operator, count) in [("f", 6), ("f*", 2)] {
        let p = program(
            &format!("1 1 2 2 re 3 1 l 3 4 l 1 4 l h {operator}"),
            InitialClip::Unbounded,
        );
        let image = render_orthogonal_raster(&p, target(), RasterLimits::default()).unwrap();
        assert_eq!(black(&image), count);
    }
}
#[test]
fn shared_contour_boundary_does_not_create_a_pixel_seam() {
    for operator in ["f", "f*"] {
        let p = program(
            &format!("1 1 3 6 re 4 1 3 6 re {operator}"),
            InitialClip::Unbounded,
        );
        let image = render_orthogonal_raster(&p, target(), RasterLimits::default()).unwrap();
        assert_eq!(black(&image), 36);
        for y in 1..7 {
            assert_eq!(
                &image.pixels()[(y * 8 + 3) * 4..(y * 8 + 5) * 4],
                &[0, 0, 0, 255, 0, 0, 0, 255]
            );
        }
    }
}
#[test]
fn initial_clip_uses_page_to_device_transform_with_axis_exchange() {
    let p = program(
        "0 0 8 8 re f",
        InitialClip::Rectangle(Rect::from_corners(1., 2., 7., 5.).unwrap()),
    );
    let t = RasterTarget {
        page_to_device: Affine::from_pdf([0., 1., -1., 0., 8., 0.]).unwrap(),
        ..target()
    };
    let image = render_orthogonal_raster(&p, t, RasterLimits::default()).unwrap();
    assert_eq!(black(&image), 18);
    for y in 0..8 {
        for x in 0..8 {
            let expected = if (3..6).contains(&x) && (1..7).contains(&y) {
                [0, 0, 0, 255]
            } else {
                [255; 4]
            };
            assert_eq!(
                &image.pixels()[(y * 8 + x) * 4..(y * 8 + x + 1) * 4],
                &expected
            );
        }
    }
}
#[test]
fn sibling_clips_cost_all_retained_masks_even_at_depth_one() {
    let p = program(&"q 1 1 6 6 re W n Q ".repeat(10), InitialClip::Unbounded);
    let exact = RasterLimits {
        max_clip_depth: 1,
        max_mask_bytes: 640,
        ..RasterLimits::default()
    };
    let plan = prepare_orthogonal_raster(&p, target(), exact).unwrap();
    assert_eq!(plan.usage().mask_bytes, 640);
    assert_eq!(black(&plan.render().unwrap()), 0);
    let error = prepare_orthogonal_raster(
        &p,
        target(),
        RasterLimits {
            max_mask_bytes: 639,
            ..exact
        },
    )
    .unwrap_err();
    assert!(matches!(error.kind, RasterErrorKind::Limit("mask bytes")));
}
#[test]
fn invisible_unsupported_geometry_is_still_refused_before_render() {
    for source in [
        "0 0 m 8 0 l 0 4 l f",
        "1.5 1 2 2 re f",
        "1 1 2 2 re S",
        "0 0 0 1 k 1 1 2 2 re f",
    ] {
        let p = program(source, InitialClip::Empty);
        assert!(matches!(
            prepare_orthogonal_raster(&p, target(), RasterLimits::default())
                .unwrap_err()
                .kind,
            RasterErrorKind::Unsupported(_)
        ));
    }
}
