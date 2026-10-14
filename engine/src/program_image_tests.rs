// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use crate::text_font::TextFontLimits;
fn operation(bytes: &[u8]) -> Operation {
    ContentParser::new(bytes, ContentLimits::default())
        .next_operation()
        .unwrap()
        .unwrap()
}
fn feed(p: &mut ProgramInterpreter<'_, '_>, bytes: &[u8]) -> Result<(), ProgramError> {
    let mut parser = ContentParser::new(bytes, ContentLimits::default());
    while let Some(op) = parser.next_operation().unwrap() {
        p.push(&validate_operation(&op).unwrap())?;
    }
    Ok(())
}
#[test]
fn image_hook_preserves_state_and_snapshots_full_affine_clip() {
    let fonts = TextFonts::new(&[], TextFontLimits::default()).unwrap();
    let mut p = ProgramInterpreter::new(
        &fonts,
        ProgramInitialState::default(),
        ProgramLimits::default(),
    )
    .unwrap();
    feed(&mut p, b"q 0 0 9 9 re W n 2 3 4 7 8 9 cm").unwrap();
    let before = p.paths.snapshot();
    let used = p.usage();
    let mut op = operation(b"/Im Do");
    op.span = 42..48;
    p.push_image(17, &op).unwrap();
    assert_eq!(p.paths.snapshot(), before);
    assert_eq!(p.text.ctm(), before.ctm);
    assert_eq!(p.usage().operations, used.operations + 1);
    assert_eq!(p.usage().work_units, used.work_units + 1);
    assert_eq!(p.usage().child_operations, used.child_operations);
    let image = p.images[0];
    assert_eq!(image.image_id(), 17);
    assert_eq!(image.ctm().coefficients(), [2., 3., 4., 7., 8., 9.]);
    assert_eq!(image.clip(), before.clip);
    assert!(image.clip().is_some());
    assert_eq!(image.operation_index(), 5);
    assert_eq!(p.events.last().unwrap().span(), 42..48);
    feed(&mut p, b"Q").unwrap();
    p.push_image(9, &op).unwrap();
    let out = p.finish().unwrap();
    assert_eq!(
        out.images()[1].ctm().coefficients(),
        [1., 0., 0., 1., 0., 0.]
    );
    assert_eq!(out.images()[1].clip(), None);
    assert_eq!(out.images()[1].operation_index(), 7);
    assert_eq!(
        out.events()
            .iter()
            .map(ProgramEvent::kind)
            .collect::<Vec<_>>(),
        [
            ProgramEventKind::Path(0),
            ProgramEventKind::Image(0),
            ProgramEventKind::Path(1),
            ProgramEventKind::Image(1)
        ]
    );
}
#[test]
fn image_hook_state_refusals_are_sticky_without_any_new_event() {
    let fonts = TextFonts::new(&[], TextFontLimits::default()).unwrap();
    for prefix in [b"BT".as_slice(), b"0 0 m", b"W", b"0 0 3 3 re W"] {
        let mut p = ProgramInterpreter::new(
            &fonts,
            ProgramInitialState::default(),
            ProgramLimits::default(),
        )
        .unwrap();
        feed(&mut p, prefix).unwrap();
        let events = p.events.len();
        let first = p.push_image(0, &operation(b"/Im Do")).unwrap_err();
        assert!(matches!(first.kind, Failure::State(_)));
        assert!(p.images.is_empty());
        assert_eq!(p.events.len(), events);
        let usage = p.usage();
        assert_eq!(p.push_image(1, &operation(b"/Im Do")).unwrap_err(), first);
        assert_eq!(feed(&mut p, b"q").unwrap_err(), first);
        assert_eq!(p.usage(), usage);
        assert_eq!(p.finish().unwrap_err(), first);
    }
}
#[test]
fn image_hook_validates_operator_shape_and_span_before_publishing() {
    let fonts = TextFonts::new(&[], TextFontLimits::default()).unwrap();
    for bytes in [b"q".as_slice(), b"Do", b"1 Do", b"/Im /Other Do"] {
        let mut p = ProgramInterpreter::new(
            &fonts,
            ProgramInitialState::default(),
            ProgramLimits::default(),
        )
        .unwrap();
        let op = operation(bytes);
        let first = p.push_image(0, &op).unwrap_err();
        assert!(matches!(
            first.kind,
            Failure::State(_) | Failure::Operator(_)
        ));
        assert!(p.events.is_empty());
        assert!(p.images.is_empty());
        assert_eq!(first.span, op.span);
        assert_eq!(p.finish().unwrap_err(), first);
    }
    let mut p = ProgramInterpreter::new(
        &fonts,
        ProgramInitialState::default(),
        ProgramLimits::default(),
    )
    .unwrap();
    let mut op = operation(b"/Im Do");
    op.span = std::ops::Range { start: 9, end: 2 };
    let first = p.push_image(0, &op).unwrap_err();
    assert_eq!(first.kind, Failure::InvalidSpan);
    assert_eq!(p.finish().unwrap_err(), first);
    let mut p = ProgramInterpreter::new(
        &fonts,
        ProgramInitialState::default(),
        ProgramLimits::default(),
    )
    .unwrap();
    assert!(matches!(
        feed(&mut p, b"/Im Do").unwrap_err().kind,
        Failure::Unsupported(OperatorKind::XObject)
    ));
}
#[test]
fn image_hook_operation_event_work_and_allocation_limits_are_exact_and_sticky() {
    let fonts = TextFonts::new(&[], TextFontLimits::default()).unwrap();
    let op = operation(b"/Im Do");
    let mut baseline = ProgramInterpreter::new(
        &fonts,
        ProgramInitialState::default(),
        ProgramLimits::default(),
    )
    .unwrap();
    baseline.push_image(0, &op).unwrap();
    baseline.push_image(1, &op).unwrap();
    let allocated = baseline.usage().allocated_bytes;
    assert!(allocated > 0);
    type Case = (ProgramLimitKind, fn(&mut ProgramLimits, usize), usize);
    let cases: &[Case] = &[
        (ProgramLimitKind::Operations, |l, n| l.max_operations = n, 2),
        (ProgramLimitKind::Events, |l, n| l.max_events = n, 2),
        (ProgramLimitKind::WorkUnits, |l, n| l.max_work_units = n, 2),
        (
            ProgramLimitKind::AllocatedBytes,
            |l, n| l.max_allocated_bytes = n,
            allocated,
        ),
    ];
    for &(kind, set, required) in cases {
        for delta in [0, 1] {
            let mut limits = ProgramLimits::default();
            set(&mut limits, required - delta);
            let mut p =
                ProgramInterpreter::new(&fonts, ProgramInitialState::default(), limits).unwrap();
            p.push_image(0, &op).unwrap();
            let result = p.push_image(1, &op);
            if delta == 0 {
                result.unwrap();
                assert_eq!(p.finish().unwrap().images().len(), 2);
            } else {
                let first = result.unwrap_err();
                assert_eq!(first.kind, Failure::LimitExceeded(kind));
                assert_eq!(first.operation_index, 1);
                assert_eq!(first.span, op.span);
                let usage = p.usage();
                assert_eq!(p.images.len(), 1);
                assert_eq!(p.events.len(), 1);
                assert_eq!(p.push_image(2, &op).unwrap_err(), first);
                assert_eq!(feed(&mut p, b"q").unwrap_err(), first);
                assert_eq!(p.usage(), usage);
                assert_eq!(p.finish().unwrap_err(), first);
            }
        }
    }
}
#[test]
fn second_image_buffer_reservation_failure_retains_first_reservation_without_event() {
    let fonts = TextFonts::new(&[], TextFontLimits::default()).unwrap();
    let limits = ProgramLimits {
        max_allocated_bytes: size_of::<ProgramEvent>(),
        ..ProgramLimits::default()
    };
    let mut p = ProgramInterpreter::new(&fonts, ProgramInitialState::default(), limits).unwrap();
    let first = p.push_image(0, &operation(b"/Im Do")).unwrap_err();
    assert_eq!(
        first.kind,
        Failure::LimitExceeded(ProgramLimitKind::AllocatedBytes)
    );
    assert_eq!(p.usage().allocated_bytes, size_of::<ProgramEvent>());
    assert_eq!(p.usage().operations, 1);
    assert!(p.events.is_empty());
    assert!(p.images.is_empty());
    assert_eq!(p.finish().unwrap_err(), first);
}
