use super::*;
use crate::font_metrics::{FontAdvance, HorizontalMetrics};
use crate::simple_encoding::{NamedEncoding, SimpleEncoding};
use crate::text_font::{TextFont, TextFontLimits, UnicodeSource};
fn feed(p: &mut ProgramInterpreter<'_, '_>, bytes: &[u8]) -> Result<(), ProgramError> {
    let mut parser = ContentParser::new(bytes, ContentLimits::default());
    while let Some(op) = parser.next_operation().unwrap() {
        p.push(&validate_operation(&op).unwrap())?;
    }
    Ok(())
}
fn select(p: &mut ProgramInterpreter<'_, '_>, id: usize) -> Result<(), ProgramError> {
    let mut parser = ContentParser::new(b"/F 10 Tf", ContentLimits::default());
    let op = parser.next_operation().unwrap().unwrap();
    p.push_resolved_font(
        &validate_operation(&op).unwrap(),
        TextFontId::from_index(id),
    )
}
fn bbox() -> Rect<UserSpace> {
    Rect::from_corners(0., 0., 20., 20.).unwrap()
}
#[test]
fn indexed_font_inheritance_shadowing_and_restore_keep_global_indices() {
    let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let m0 =
        HorizontalMetrics::from_widths(65, 65, &[FontAdvance::new(500.).unwrap()], None).unwrap();
    let m1 =
        HorizontalMetrics::from_widths(65, 65, &[FontAdvance::new(800.).unwrap()], None).unwrap();
    let entries: &[(&[u8], TextFont<'_>)] = &[
        (
            b"F",
            TextFont::declared(UnicodeSource::Encoding(&encoding), &m0).unwrap(),
        ),
        (
            b"F",
            TextFont::declared(UnicodeSource::Encoding(&encoding), &m1).unwrap(),
        ),
    ];
    assert!(TextFonts::new(entries, TextFontLimits::default()).is_err());
    let fonts = TextFonts::new_indexed(entries, TextFontLimits::default()).unwrap();
    assert!(fonts.get(b"F").is_none());
    let mut p = ProgramInterpreter::new(
        &fonts,
        ProgramInitialState::default(),
        ProgramLimits::default(),
    )
    .unwrap();
    select(&mut p, 0).unwrap();
    let frame = p
        .enter_form(Affine::translation(4., 6.).unwrap(), bbox())
        .unwrap();
    feed(&mut p, b"BT (A) Tj ET").unwrap();
    select(&mut p, 1).unwrap();
    feed(&mut p, b"BT (A) Tj ET").unwrap();
    p.leave_form(frame).unwrap();
    feed(&mut p, b"BT (A) Tj ET").unwrap();
    let out = p.finish().unwrap();
    assert_eq!(
        out.text()
            .codes()
            .iter()
            .map(|v| v.font_id().index())
            .collect::<Vec<_>>(),
        [0, 1, 0]
    );
    assert_eq!(
        out.text()
            .codes()
            .iter()
            .map(|v| v.operation_index())
            .collect::<Vec<_>>(),
        [3, 7, 11]
    );
    assert_eq!(out.usage().operations, 13);
    let boundary: Vec<_> = out
        .events()
        .iter()
        .filter(|v| matches!(v.kind(), ProgramEventKind::Path(_)))
        .collect();
    assert_eq!(
        boundary
            .iter()
            .map(|v| v.operation_index())
            .collect::<Vec<_>>(),
        [1, 9]
    );
    assert!(boundary.iter().all(|v| v.span() == (0..0)));
    assert!(out.text_contexts()[0].clip().is_some());
    assert!(out.text_contexts().last().unwrap().clip().is_none());
}
#[test]
fn form_save_floor_and_failed_boundaries_are_sticky() {
    let fonts = TextFonts::new(&[], TextFontLimits::default()).unwrap();
    let mut p = ProgramInterpreter::new(
        &fonts,
        ProgramInitialState::default(),
        ProgramLimits::default(),
    )
    .unwrap();
    let frame = p.enter_form(Affine::identity(), bbox()).unwrap();
    let first = feed(&mut p, b"Q").unwrap_err();
    assert!(matches!(
        first.kind,
        Failure::State("Q crosses implicit Form save")
    ));
    assert_eq!(p.leave_form(frame).unwrap_err(), first);
    assert_eq!(p.finish().unwrap_err(), first);
    let mut p = ProgramInterpreter::new(
        &fonts,
        ProgramInitialState::default(),
        ProgramLimits::default(),
    )
    .unwrap();
    let frame = p.enter_form(Affine::identity(), bbox()).unwrap();
    feed(&mut p, b"q").unwrap();
    assert!(matches!(
        p.leave_form(frame).unwrap_err().kind,
        Failure::State("unbalanced Form graphics saves")
    ));
}
#[test]
fn boundary_operations_use_one_index_but_charge_internal_work_and_storage() {
    let fonts = TextFonts::new(&[], TextFontLimits::default()).unwrap();
    let mut p = ProgramInterpreter::new(
        &fonts,
        ProgramInitialState::default(),
        ProgramLimits::default(),
    )
    .unwrap();
    let frame = p.enter_form(Affine::identity(), bbox()).unwrap();
    assert_eq!(p.usage().operations, 1);
    assert_eq!(p.usage().child_operations, 11); // q and cm dispatch to both children.
    assert!(p.usage().allocated_bytes > 0);
    p.leave_form(frame).unwrap();
    let usage = p.usage();
    assert_eq!(usage.operations, 2);
    assert_eq!(usage.child_operations, 13);
    p.finish().unwrap();
    let limits = ProgramLimits {
        max_operations: 1,
        ..ProgramLimits::default()
    };
    let mut p = ProgramInterpreter::new(&fonts, ProgramInitialState::default(), limits).unwrap();
    let frame = p.enter_form(Affine::identity(), bbox()).unwrap();
    let before = p.usage();
    assert!(p.leave_form(frame).is_err());
    assert_eq!(p.usage().allocated_bytes, before.allocated_bytes);
    assert_eq!(p.usage().operations, 1);
}
#[test]
fn nested_frames_enforce_lifo_and_reject_active_paths() {
    let fonts = TextFonts::new(&[], TextFontLimits::default()).unwrap();
    let mut p = ProgramInterpreter::new(
        &fonts,
        ProgramInitialState::default(),
        ProgramLimits::default(),
    )
    .unwrap();
    let outer = p.enter_form(Affine::identity(), bbox()).unwrap();
    let _inner = p.enter_form(Affine::identity(), bbox()).unwrap();
    assert!(matches!(
        p.leave_form(outer).unwrap_err().kind,
        Failure::State("stale or out-of-order Form frame")
    ));
    let mut p = ProgramInterpreter::new(
        &fonts,
        ProgramInitialState::default(),
        ProgramLimits::default(),
    )
    .unwrap();
    feed(&mut p, b"0 0 m").unwrap();
    assert!(matches!(
        p.enter_form(Affine::identity(), bbox()).unwrap_err().kind,
        Failure::State("Form boundary with active or pending path is unsupported")
    ));
}

#[test]
fn resolved_font_misuse_fails_stickily_before_publishing_state() {
    let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let metrics =
        HorizontalMetrics::from_widths(65, 65, &[FontAdvance::new(500.).unwrap()], None).unwrap();
    let font = TextFont::declared(UnicodeSource::Encoding(&encoding), &metrics).unwrap();
    let fonts = TextFonts::new_indexed(&[(b"F", font)], TextFontLimits::default()).unwrap();
    for (bytes, id) in [
        (b"/F 10 Tf".as_slice(), 7),
        (b"/Other 10 Tf".as_slice(), 0),
        (b"q".as_slice(), 0),
    ] {
        let mut p = ProgramInterpreter::new(
            &fonts,
            ProgramInitialState::default(),
            ProgramLimits::default(),
        )
        .unwrap();
        let mut parser = ContentParser::new(bytes, ContentLimits::default());
        let op = parser.next_operation().unwrap().unwrap();
        let error = p
            .push_resolved_font(
                &validate_operation(&op).unwrap(),
                TextFontId::from_index(id),
            )
            .unwrap_err();
        assert_eq!(p.usage().operations, 1);
        assert_eq!(p.usage().child_operations, usize::from(bytes != b"q"));
        assert_eq!(feed(&mut p, b"q").unwrap_err(), error);
        assert_eq!(p.finish().unwrap_err(), error);
    }
}
