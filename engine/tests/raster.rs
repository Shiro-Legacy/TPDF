#![cfg(feature = "raster")]
use tandem_pdf_engine::{
    content::ContentLimits,
    geometry::{Affine, DeviceSpace, PageSpace, Rect},
    path::*,
    raster::*,
};
fn program(s: &str) -> PathProgram {
    interpret_path_content(
        s.as_bytes(),
        PathInitialState::default(),
        ContentLimits::default(),
        PathLimits::default(),
    )
    .unwrap()
}
fn target() -> RasterTarget {
    RasterTarget {
        width: 10,
        height: 10,
        page_to_device: Affine::<PageSpace, DeviceSpace>::from_pdf([1., 0., 0., 1., 0., 0.])
            .unwrap(),
    }
}
fn render(s: &str) -> RasterImage {
    render_orthogonal_raster(&program(s), target(), RasterLimits::default()).unwrap()
}
fn at(i: &RasterImage, x: usize, y: usize) -> [u8; 4] {
    i.pixels()[(y * 10 + x) * 4..(y * 10 + x + 1) * 4]
        .try_into()
        .unwrap()
}
fn black_count(i: &RasterImage) -> usize {
    i.pixels()
        .chunks_exact(4)
        .filter(|p| p[0] == 0 && p[1] == 0 && p[2] == 0)
        .count()
}
#[test]
fn empty_white_image() {
    let i = render("");
    assert_eq!((i.width(), i.height(), i.stride()), (10, 10, 40));
    assert_eq!(i.pixels(), &[255; 400]);
}
#[test]
fn rectangle_and_negative_winding() {
    for s in ["1 2 3 2 re f", "4 4 -3 -2 re f"] {
        let i = render(s);
        assert_eq!(black_count(&i), 6);
        for y in 0..10 {
            for x in 0..10 {
                assert_eq!(
                    at(&i, x, y),
                    if (1..4).contains(&x) && (2..4).contains(&y) {
                        [0, 0, 0, 255]
                    } else {
                        [255; 4]
                    }
                );
            }
        }
    }
}
#[test]
fn concave_orthogonal_implicit_close() {
    let i = render("1 1 m 7 1 l 7 3 l 3 3 l 3 7 l 1 7 l f");
    assert_eq!(black_count(&i), 20);
    assert_eq!(at(&i, 2, 5), [0, 0, 0, 255]);
    assert_eq!(at(&i, 4, 4), [255; 4]);
}
#[test]
fn compound_fill_rules_and_cancellation() {
    assert_eq!(black_count(&render("1 1 8 8 re 3 3 4 4 re f")), 64);
    assert_eq!(black_count(&render("1 1 8 8 re 3 3 4 4 re f*")), 48);
    assert_eq!(black_count(&render("1 1 8 8 re 7 3 -4 4 re f")), 48);
    assert_eq!(black_count(&render("1 1 8 8 re 9 1 -8 8 re f")), 0);
}
#[test]
fn clip_restore_and_paint_order() {
    let i = render("q 1 1 2 2 re W n 0 0 10 10 re f Q 1 0 0 rg 7 7 2 2 re f");
    assert_eq!(black_count(&i), 4);
    assert_eq!(at(&i, 7, 7), [255, 0, 0, 255]);
    assert_eq!(at(&i, 5, 5), [255; 4]);
}
#[test]
fn nested_clip_and_empty_clip() {
    assert_eq!(
        black_count(&render("1 1 8 8 re W n 3 3 2 2 re W n 0 0 10 10 re f")),
        4
    );
    assert_eq!(black_count(&render("W n 0 0 10 10 re f")), 0);
}
#[test]
fn fill_before_pending_clip() {
    let i = render("1 1 8 8 re W f 1 0 0 rg 0 0 10 10 re f");
    assert_eq!(at(&i, 1, 1), [255, 0, 0, 255]);
    assert_eq!(at(&i, 0, 0), [255; 4]);
}
#[test]
fn initial_rectangle_and_empty_clips() {
    for (clip, n) in [
        (
            InitialClip::Rectangle(Rect::from_corners(2., 2., 5., 4.).unwrap()),
            6,
        ),
        (InitialClip::Empty, 0),
    ] {
        let p = interpret_path_content(
            b"0 0 10 10 re f",
            PathInitialState::new(Affine::from_pdf([1., 0., 0., 1., 0., 0.]).unwrap(), clip),
            ContentLimits::default(),
            PathLimits::default(),
        )
        .unwrap();
        assert_eq!(
            black_count(&render_orthogonal_raster(&p, target(), RasterLimits::default()).unwrap()),
            n
        );
    }
}
#[test]
fn composition_and_reflection() {
    let p = program("2 0 0 2 0 0 cm 1 1 2 2 re f");
    let mut t = target();
    t.page_to_device = Affine::from_pdf([-1., 0., 0., 1., 10., 0.]).unwrap();
    let i = render_orthogonal_raster(&p, t, RasterLimits::default()).unwrap();
    assert_eq!(black_count(&i), 16);
    assert_eq!(at(&i, 4, 2), [0, 0, 0, 255]);
}
#[test]
fn unsupported_geometry_and_paints() {
    for s in [
        "1.5 1 2 2 re f",
        "0 0 m 8 0 l 0 4 l f",
        "1 1 2 0 re f",
        "1 1 m 2 2 l f",
        "1 1 2 2 re S",
        "0 0 0 1 k 1 1 2 2 re f",
        "-1 1 2 2 re f",
        "1 1 m 2 1 3 2 3 3 c h f",
        "0 0 m 4 0 l 4 4 l 2 4 l 2 0 l 0 0 l f",
    ] {
        let e =
            prepare_orthogonal_raster(&program(s), target(), RasterLimits::default()).unwrap_err();
        assert!(
            matches!(e.kind, RasterErrorKind::Unsupported(_)),
            "{s}: {e}"
        );
    }
}
#[test]
fn unsupported_suffix_is_transactional() {
    let e = prepare_orthogonal_raster(
        &program("1 1 2 2 re f 3 3 2 2 re B"),
        target(),
        RasterLimits::default(),
    )
    .unwrap_err();
    assert_eq!(e.kind, RasterErrorKind::Unsupported("stroke"));
    assert!(e.span.start > 10);
}
#[test]
fn empty_move_close_runs_do_not_reach_backend() {
    let mut s = "1 1 m h ".repeat(100);
    s.push_str("2 2 3 3 re f");
    assert_eq!(black_count(&render(&s)), 9);
}
#[test]
fn repeated_close_and_duplicate_points() {
    assert_eq!(
        black_count(&render("1 1 m 1 1 l 3 1 l 3 3 l 1 3 l h h f")),
        4
    );
}
#[test]
fn gray_and_ordered_colors() {
    let i = render("0.5 g 1 1 3 3 re f 0 1 0 rg 2 2 3 3 re f");
    assert_eq!(at(&i, 1, 1), [128, 128, 128, 255]);
    assert_eq!(at(&i, 2, 2), [0, 255, 0, 255]);
}
#[test]
fn exact_work_storage_and_mask_limits() {
    let p = program("1 1 5 5 re W n 0 0 10 10 re f");
    let usage = prepare_orthogonal_raster(&p, target(), RasterLimits::default())
        .unwrap()
        .usage();
    let exact = RasterLimits {
        max_storage_bytes: usage.planned_storage_bytes,
        max_work: usage.work,
        max_mask_bytes: usage.mask_bytes,
        max_total_commands: usage.transformed_commands,
        ..RasterLimits::default()
    };
    assert!(prepare_orthogonal_raster(&p, target(), exact).is_ok());
    for l in [
        RasterLimits {
            max_work: usage.work - 1,
            ..exact
        },
        RasterLimits {
            max_storage_bytes: usage.planned_storage_bytes - 1,
            ..exact
        },
        RasterLimits {
            max_mask_bytes: usage.mask_bytes - 1,
            ..exact
        },
        RasterLimits {
            max_total_commands: usage.transformed_commands - 1,
            ..exact
        },
    ] {
        assert!(matches!(
            prepare_orthogonal_raster(&p, target(), l).unwrap_err().kind,
            RasterErrorKind::Limit(_)
        ));
    }
}
#[test]
fn dimension_event_depth_and_command_limits() {
    let p = program("1 1 5 5 re W n 2 2 2 2 re W n 0 0 10 10 re f");
    for l in [
        RasterLimits {
            max_clip_depth: 1,
            ..RasterLimits::default()
        },
        RasterLimits {
            max_events: 2,
            ..RasterLimits::default()
        },
        RasterLimits {
            max_commands_per_path: 4,
            ..RasterLimits::default()
        },
        RasterLimits {
            max_pixels: 99,
            ..RasterLimits::default()
        },
        RasterLimits {
            max_output_bytes: 399,
            ..RasterLimits::default()
        },
    ] {
        assert!(prepare_orthogonal_raster(&p, target(), l).is_err());
    }
    let mut t = target();
    t.width = 4097;
    assert!(prepare_orthogonal_raster(&program(""), t, RasterLimits::default()).is_err());
}
#[test]
fn repeated_render_is_immutable() {
    let p = prepare_orthogonal_raster(&program("1 1 4 4 re f"), target(), RasterLimits::default())
        .unwrap();
    assert_eq!(p.render().unwrap(), p.render().unwrap());
}
