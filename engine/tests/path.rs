// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::content::{ContentLimits, ContentParser};
use tandem_pdf_engine::geometry::{Affine, PageSpace, Point, Rect, UserSpace};
use tandem_pdf_engine::operators::validate_operation;
use tandem_pdf_engine::path::*;
fn run(s: &str) -> Result<PathProgram, PathError> {
    limited(s, PathLimits::default())
}
fn limited(s: &str, limits: PathLimits) -> Result<PathProgram, PathError> {
    interpret_path_content(
        s.as_bytes(),
        PathInitialState::default(),
        ContentLimits::default(),
        limits,
    )
}
fn p(x: f64, y: f64) -> Point<UserSpace> {
    Point::new(x, y).unwrap()
}
fn stroke(program: &PathProgram, n: usize) -> (&PaintEvent, StrokeStyle) {
    match &program.events()[n] {
        PathEvent::Stroke { paint, style } => (paint, *style),
        other => panic!("unexpected {other:?}"),
    }
}
#[test]
fn empty_program_and_empty_path_end_are_valid() {
    let out = run("n S f B").unwrap();
    assert!(out.events().is_empty());
    assert!(out.paths().is_empty());
}
#[test]
fn shorthand_curves_close_and_continuation_preserve_points() {
    let out = run("1 2 m 3 4 5 6 v 7 8 9 10 y h h 2 3 l S").unwrap();
    assert_eq!(
        out.paths()[0].commands(),
        [
            PathCommand::MoveTo(p(1., 2.)),
            PathCommand::CubicTo {
                control1: p(1., 2.),
                control2: p(3., 4.),
                end: p(5., 6.)
            },
            PathCommand::CubicTo {
                control1: p(7., 8.),
                control2: p(9., 10.),
                end: p(9., 10.)
            },
            PathCommand::ClosePath,
            PathCommand::LineTo(p(2., 3.))
        ]
    );
}
#[test]
fn rectangles_preserve_negative_dimensions_and_winding() {
    let out = run("10 20 -3 -4 re S").unwrap();
    assert_eq!(
        out.paths()[0].commands(),
        [
            PathCommand::MoveTo(p(10., 20.)),
            PathCommand::LineTo(p(7., 20.)),
            PathCommand::LineTo(p(7., 16.)),
            PathCommand::LineTo(p(10., 16.)),
            PathCommand::ClosePath
        ]
    );
}
#[test]
fn close_paint_operators_and_fill_stroke_geometry_are_distinct() {
    for op in ["s", "b", "b*"] {
        let out = run(&format!("0 0 m 1 1 l {op}")).unwrap();
        assert_eq!(
            out.paths()[0].commands().last(),
            Some(&PathCommand::ClosePath)
        );
    }
    for op in ["S", "f", "F", "f*", "B", "B*"] {
        let out = run(&format!("0 0 m 1 1 l {op}")).unwrap();
        assert_ne!(
            out.paths()[0].commands().last(),
            Some(&PathCommand::ClosePath)
        );
    }
    let out = run("0 0 m 1 1 l B*").unwrap();
    assert!(matches!(
        out.events()[0],
        PathEvent::Fill {
            rule: FillRule::EvenOdd,
            ..
        }
    ));
    assert!(matches!(out.events()[1], PathEvent::Stroke { .. }));
    assert_eq!(out.paths().len(), 1);
}
#[test]
fn q_q_restores_style_but_does_not_restore_or_clear_path() {
    let out = run("0 0 m q 5 w 1 1 l Q S").unwrap();
    let (_, style) = stroke(&out, 0);
    assert_eq!(style.width(), 1.);
    assert_eq!(out.paths()[0].commands().len(), 2);
    let out = run("q 0 0 m Q 1 1 l S").unwrap();
    assert_eq!(out.paths()[0].commands().len(), 2);
}
#[test]
fn matrix_order_is_new_then_existing_and_initial_transform_is_explicit() {
    let out = run("1 0 0 1 10 20 cm 2 0 0 3 0 0 cm 1 1 m 2 2 l S").unwrap();
    let (paint, _) = stroke(&out, 0);
    assert_eq!(
        paint
            .ctm()
            .transform_point(p(1., 1.))
            .unwrap()
            .coordinates(),
        [12., 23.]
    );
    let initial = PathInitialState::new(
        Affine::<UserSpace, PageSpace>::from_pdf([1., 0., 0., -1., 0., 100.]).unwrap(),
        InitialClip::Rectangle(Rect::from_corners(0., 0., 10., 10.).unwrap()),
    );
    let out = interpret_path_content(
        b"0 0 m 1 1 l S",
        initial,
        ContentLimits::default(),
        PathLimits::default(),
    )
    .unwrap();
    assert_eq!(
        stroke(&out, 0)
            .0
            .ctm()
            .transform_point(p(1., 1.))
            .unwrap()
            .coordinates(),
        [1., 99.]
    );
    assert!(matches!(out.initial_clip(), InitialClip::Rectangle(_)));
}
#[test]
fn mixed_transform_policy_covers_cm_q_and_pending_clip() {
    for text in [
        "0 0 m 1 0 0 1 0 0 cm",
        "q 2 0 0 2 0 0 cm 0 0 m Q",
        "W 1 0 0 1 1 0 cm",
    ] {
        assert!(matches!(
            run(text).unwrap_err().kind,
            PathErrorKind::MixedPathTransformUnsupported
        ));
    }
    assert!(run("q 0 0 m Q n").is_ok());
}
#[test]
fn pending_clip_applies_after_fill_and_stroke_and_q_restores_ancestry() {
    let out = run("q 0 0 10 10 re W B 1 1 2 2 re f Q").unwrap();
    assert_eq!(out.events().len(), 5);
    assert_eq!(out.paths().len(), 2);
    assert_eq!(out.clips().len(), 1);
    let PathEvent::Fill { paint, .. } = &out.events()[0] else {
        panic!()
    };
    assert_eq!(paint.clip(), None);
    assert_eq!(stroke(&out, 1).0.clip(), None);
    let PathEvent::ClipPush { id, .. } = out.events()[2] else {
        panic!()
    };
    assert_eq!(id.index(), 0);
    let PathEvent::Fill { paint, .. } = &out.events()[3] else {
        panic!()
    };
    assert_eq!(paint.clip(), Some(id));
    assert!(matches!(
        out.events()[4],
        PathEvent::ClipRestore { clip: None, .. }
    ));
    assert_eq!(out.clips()[0].path().index(), 0);
    assert_eq!(out.clips()[0].parent(), None);
}
#[test]
fn pending_clip_is_outside_q_state_and_last_rule_wins() {
    let out = run("0 0 1 1 re W q W* Q n").unwrap();
    assert_eq!(out.clips()[0].rule(), FillRule::EvenOdd);
    let out = run("q W Q n").unwrap();
    assert_eq!(out.clips().len(), 1);
    assert!(out.paths()[0].commands().is_empty());
}
#[test]
fn clip_nodes_are_flat_and_refer_only_to_older_parents() {
    let out = run("0 0 3 3 re W n 1 1 2 2 re W* n").unwrap();
    assert_eq!(out.clips()[0].parent(), None);
    assert_eq!(out.clips()[1].parent().unwrap().index(), 0);
    assert_eq!(out.clips()[1].path().index(), 1);
}
#[test]
fn empty_clip_is_retained_and_not_an_unbounded_clip() {
    let out = run("W n 0 0 m 1 1 l S").unwrap();
    assert!(out.paths()[0].commands().is_empty());
    assert_eq!(stroke(&out, 1).0.clip().unwrap().index(), 0);
    let out = interpret_path_content(
        b"0 0 m 1 1 l S",
        PathInitialState::new(
            Affine::from_pdf([1., 0., 0., 1., 0., 0.]).unwrap(),
            InitialClip::Empty,
        ),
        ContentLimits::default(),
        PathLimits::default(),
    )
    .unwrap();
    assert_eq!(out.initial_clip(), InitialClip::Empty);
}
#[test]
fn hairline_caps_joins_miter_and_dash_are_snapshotted() {
    let out = run("0 w 2 J 1 j 7 M [1 0 2] 9 d q [4 3] 1 d 0 0 m 1 1 l S Q 0 0 m 2 2 l S").unwrap();
    let (_, first) = stroke(&out, 0);
    let (_, last) = stroke(&out, 1);
    assert_eq!(first.width(), 0.);
    assert_eq!(first.cap(), LineCap::Square);
    assert_eq!(first.join(), LineJoin::Round);
    assert_eq!(first.miter_limit(), 7.);
    assert_eq!(
        out.dashes()[first.dash().unwrap().index()].values(),
        [4., 3.]
    );
    assert_eq!(
        out.dashes()[last.dash().unwrap().index()].values(),
        [1., 0., 2.]
    );
    assert_eq!(last.dash_phase(), 9.);
    let out = run("[] 4 d 0 0 m 1 1 l S").unwrap();
    assert_eq!(stroke(&out, 0).1.dash(), None);
    assert_eq!(stroke(&out, 0).1.dash_phase(), 4.);
}
#[test]
fn device_colors_have_separate_fill_and_stroke_and_restore() {
    let out = run("1 0 0 rg 0 1 0 0 K q .5 g 0 0 1 1 re B Q 0 0 1 1 re f").unwrap();
    let PathEvent::Fill { paint, .. } = &out.events()[0] else {
        panic!()
    };
    assert_eq!(paint.color(), DeviceColor::Gray(0.5));
    assert_eq!(
        stroke(&out, 1).0.color(),
        DeviceColor::Cmyk([0., 1., 0., 0.])
    );
    let PathEvent::Fill { paint, .. } = &out.events()[2] else {
        panic!()
    };
    assert_eq!(paint.color(), DeviceColor::Rgb([1., 0., 0.]));
    assert!(matches!(
        run("2 g").unwrap_err().kind,
        PathErrorKind::ColorRangeUnsupported
    ));
}
#[test]
fn malformed_execution_and_completion_are_strict() {
    for text in ["1 1 l", "1 1 2 2 3 3 c", "h", "s", "b", "b*"] {
        assert!(matches!(
            run(text).unwrap_err().kind,
            PathErrorKind::MissingCurrentPoint
        ));
    }
    assert!(matches!(
        run("Q").unwrap_err().kind,
        PathErrorKind::StackUnderflow
    ));
    assert!(matches!(
        run("q").unwrap_err().kind,
        PathErrorKind::UnbalancedState
    ));
    for text in ["0 0 m", "W"] {
        assert!(matches!(
            run(text).unwrap_err().kind,
            PathErrorKind::UnfinishedPath
        ));
    }
}
#[test]
fn unsupported_operations_never_return_partial_success() {
    for text in [
        "BT",
        "/X Do",
        "/S sh",
        "/G gs",
        "/DeviceRGB cs",
        "BX",
        "EX",
        "/Tag BMC",
        "1 i",
        "/Perceptual ri",
        "(x) Tj",
    ] {
        assert!(matches!(
            run(&format!("0 0 1 1 re f {text}")).unwrap_err().kind,
            PathErrorKind::Unsupported(_)
        ));
    }
    assert!(matches!(
        run("SomeUnknown").unwrap_err().kind,
        PathErrorKind::Operator(_)
    ));
    assert!(matches!(
        run("BI arbitrary binary").unwrap_err().kind,
        PathErrorKind::Content(_)
    ));
}
#[test]
fn numeric_conversion_and_arithmetic_overflow_fail_explicitly() {
    assert!(matches!(
        run("9007199254740993 0 m").unwrap_err().kind,
        PathErrorKind::InexactInteger
    ));
    assert!(matches!(
        run("1e308 0 1e308 1 re").unwrap_err().kind,
        PathErrorKind::Content(_)
    )); // exponent syntax is refused by lexer.
    let huge = "9".repeat(308); // Valid decimal reals avoid integer parser overflow.
    assert!(matches!(
        run(&format!("{huge}.0 0 {huge}.0 1 re")).unwrap_err().kind,
        PathErrorKind::Geometry(_)
    ));
    assert!(matches!(
        run(&format!("2 0 0 2 0 0 cm {huge}.0 0 m S"))
            .unwrap_err()
            .kind,
        PathErrorKind::Geometry(_)
    ));
    assert!(run("0 0 0 0 1 1 cm 0 0 m 1 1 l S").is_ok()); // Singular does not require inversion.
}
#[test]
fn limits_count_discarded_paths_empty_ops_and_derived_commands() {
    let d = PathLimits::default();
    assert!(
        limited(
            "n n",
            PathLimits {
                max_operations: 2,
                ..d
            }
        )
        .is_ok()
    );
    assert!(
        limited(
            "n n n",
            PathLimits {
                max_operations: 2,
                ..d
            }
        )
        .is_err()
    );
    assert!(
        limited(
            "q q Q Q",
            PathLimits {
                max_save_depth: 2,
                ..d
            }
        )
        .is_ok()
    );
    assert!(
        limited(
            "q q",
            PathLimits {
                max_save_depth: 1,
                ..d
            }
        )
        .is_err()
    );
    assert!(
        limited(
            "0 0 1 1 re n",
            PathLimits {
                max_current_commands: 5,
                max_total_commands: 5,
                ..d
            }
        )
        .is_ok()
    );
    assert!(
        limited(
            "0 0 1 1 re n",
            PathLimits {
                max_current_commands: 4,
                ..d
            }
        )
        .is_err()
    );
    assert!(
        limited(
            "0 0 m n 0 0 m n",
            PathLimits {
                max_total_commands: 1,
                ..d
            }
        )
        .is_err()
    );
    assert!(
        limited(
            "0 0 1 1 re n",
            PathLimits {
                max_paths: 0,
                max_events: 0,
                ..d
            }
        )
        .is_ok()
    );
    assert!(limited("0 0 1 1 re f", PathLimits { max_paths: 0, ..d }).is_err());
    assert!(limited("0 0 1 1 re B", PathLimits { max_events: 1, ..d }).is_err());
    assert!(limited("W n", PathLimits { max_clips: 0, ..d }).is_err());
}
#[test]
fn dash_work_is_cumulative_and_empty_allocation_budget_is_valid() {
    let d = PathLimits::default();
    assert!(
        limited(
            "[1 2 3] 0 d",
            PathLimits {
                max_dash_elements: 2,
                ..d
            }
        )
        .is_err()
    );
    assert!(
        limited(
            "[1 2] 0 d [3 4] 0 d",
            PathLimits {
                max_total_dash_elements: 3,
                ..d
            }
        )
        .is_err()
    );
    assert!(
        limited(
            "",
            PathLimits {
                max_allocated_bytes: 0,
                ..d
            }
        )
        .is_ok()
    );
    assert!(
        limited(
            "0 0 m n",
            PathLimits {
                max_allocated_bytes: 0,
                ..d
            }
        )
        .is_err()
    );
}
#[test]
fn failed_builder_is_sticky_and_errors_keep_operation_span() {
    let mut parser = ContentParser::new(b"  1 1 l n", ContentLimits::default());
    let first = parser.next_operation().unwrap().unwrap();
    let next = parser.next_operation().unwrap().unwrap();
    let mut builder =
        PathInterpreter::new(PathInitialState::default(), PathLimits::default()).unwrap();
    let error = builder
        .push(&validate_operation(&first).unwrap())
        .unwrap_err();
    assert_eq!(error.span, 2..7);
    assert!(matches!(
        builder
            .push(&validate_operation(&next).unwrap())
            .unwrap_err()
            .kind,
        PathErrorKind::Failed
    ));
    assert!(matches!(
        builder.finish().unwrap_err().kind,
        PathErrorKind::Failed
    ));
}
#[test]
fn many_clips_drop_without_recursive_ownership() {
    let out = run(&"W n ".repeat(5000)).unwrap();
    assert_eq!(out.clips().len(), 5000);
    assert_eq!(out.clips().last().unwrap().parent().unwrap().index(), 4998);
    drop(out);
}

#[test]
fn allocation_capacity_is_capped_and_discarded_buffer_can_be_reused() {
    let bytes = std::mem::size_of::<PathCommand>();
    let limits = PathLimits {
        max_allocated_bytes: bytes,
        ..Default::default()
    };
    assert!(limited("0 0 m n 1 1 m n", limits).is_ok());
    assert!(
        limited(
            "0 0 m n",
            PathLimits {
                max_allocated_bytes: bytes - 1,
                ..limits
            }
        )
        .is_err()
    );
    assert!(limited("0 0 m 1 1 l n", limits).is_err());
}
