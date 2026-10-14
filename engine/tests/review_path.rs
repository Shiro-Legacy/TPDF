// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent flat-arena, clipping and discarded-work checks.
use tandem_pdf_engine::content::ContentLimits;
use tandem_pdf_engine::geometry::{Affine, PageSpace, Point, Rect, UserSpace};
use tandem_pdf_engine::path::{
    InitialClip, PathCommand, PathErrorKind, PathEvent, PathInitialState, PathLimits,
    interpret_path_content,
};
fn run(
    input: &[u8],
    initial: PathInitialState,
    limits: PathLimits,
) -> Result<tandem_pdf_engine::path::PathProgram, tandem_pdf_engine::path::PathError> {
    interpret_path_content(input, initial, ContentLimits::default(), limits)
}
#[test]
fn discard_reuses_capacity_but_never_refunds_constructed_work() {
    let input = b"0 0 m 1 1 l n ".repeat(10_000);
    let limits = PathLimits {
        max_allocated_bytes: 4 * std::mem::size_of::<PathCommand>(),
        max_total_commands: 20_000,
        ..PathLimits::default()
    };
    let program = run(&input, PathInitialState::default(), limits).unwrap();
    assert!(program.paths().is_empty());
    assert!(program.events().is_empty());
    let error = run(
        &input,
        PathInitialState::default(),
        PathLimits {
            max_total_commands: 19_999,
            ..limits
        },
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        PathErrorKind::LimitExceeded("total path commands")
    ));
}
#[test]
fn shared_paint_clip_path_and_branch_restoration_keep_exact_ancestry() {
    let program = run(
        b"q 0 0 10 10 re W B q 1 1 8 8 re W* B* Q 2 2 6 6 re W n Q 3 3 4 4 re W n",
        PathInitialState::default(),
        PathLimits::default(),
    )
    .unwrap();
    assert_eq!(program.paths().len(), 4);
    assert_eq!(program.clips().len(), 4);
    let first = program.clips()[0].path();
    for event in &program.events()[..2] {
        let paint = match event {
            PathEvent::Fill { paint, .. } | PathEvent::Stroke { paint, .. } => paint,
            _ => panic!("expected paint"),
        };
        assert_eq!(paint.path(), first);
        assert_eq!(paint.clip(), None);
    }
    let first_id = match program.events()[2] {
        PathEvent::ClipPush { id, .. } => id,
        _ => panic!("expected clip"),
    };
    assert_eq!(program.clips()[1].parent(), Some(first_id));
    assert_eq!(program.clips()[2].parent(), Some(first_id));
    assert_eq!(program.clips()[3].parent(), None);
    assert!(
        program
            .clips()
            .iter()
            .enumerate()
            .all(|(i, c)| c.parent().is_none_or(|p| p.index() < i))
    );
}
#[test]
fn initial_empty_and_degenerate_rectangle_clips_are_preserved_as_supplied() {
    let identity = Affine::<UserSpace, PageSpace>::from_pdf([1., 0., 0., 1., 0., 0.]).unwrap();
    for clip in [
        InitialClip::Empty,
        InitialClip::Rectangle(Rect::from_corners(0., 0., 0., 5.).unwrap()),
    ] {
        let program = run(
            b"0 0 m 0 0 l S W n",
            PathInitialState::new(identity, clip),
            PathLimits::default(),
        )
        .unwrap();
        assert_eq!(program.initial_clip(), clip);
        assert_eq!(
            program.paths()[0].commands(),
            &[
                PathCommand::MoveTo(Point::new(0., 0.).unwrap()),
                PathCommand::LineTo(Point::new(0., 0.).unwrap())
            ]
        );
        assert!(program.paths()[1].commands().is_empty());
        assert_eq!(program.clips().len(), 1);
    }
}
#[test]
fn overflowing_curve_control_point_refuses_even_with_finite_endpoints() {
    let transform =
        Affine::<UserSpace, PageSpace>::from_pdf([f64::MAX, 0., 0., 1., 0., 0.]).unwrap();
    let initial = PathInitialState::new(transform, InitialClip::Unbounded);
    let error = run(b"0 0 m 2 0 0 1 0 0 c f", initial, PathLimits::default()).unwrap_err();
    assert!(matches!(error.kind, PathErrorKind::Geometry(_)));
    assert_eq!(error.span, 20..21);
}
