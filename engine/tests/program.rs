use tandem_pdf_engine::content::{ContentLimits, ContentParser};
use tandem_pdf_engine::font_metrics::{FontAdvance, HorizontalMetrics};
use tandem_pdf_engine::geometry::{Affine, PageSpace, Rect, UserSpace};
use tandem_pdf_engine::operators::validate_operation;
use tandem_pdf_engine::path::{DeviceColor, InitialClip, PathEvent, PathLimits};
use tandem_pdf_engine::program::*;
use tandem_pdf_engine::simple_encoding::{NamedEncoding, SimpleEncoding};
use tandem_pdf_engine::text_font::*;
use tandem_pdf_engine::text_position::{TextEvent, TextPositionLimits};
fn font_data() -> (SimpleEncoding, HorizontalMetrics) {
    (
        SimpleEncoding::from_named(NamedEncoding::WinAnsi),
        HorizontalMetrics::from_widths(32, 90, &[FontAdvance::new(500.).unwrap(); 59], None)
            .unwrap(),
    )
}
fn with_fonts<T>(f: impl FnOnce(&TextFonts<'_>) -> T) -> T {
    let (e, m) = font_data();
    let font = TextFont::declared(UnicodeSource::Encoding(&e), &m).unwrap();
    let fonts = TextFonts::new(&[(b"F", font)], TextFontLimits::default()).unwrap();
    f(&fonts)
}
fn limited(input: &[u8], limits: ProgramLimits) -> Result<Program, ProgramError> {
    with_fonts(|fonts| {
        interpret_program_content(
            input,
            fonts,
            ProgramInitialState::default(),
            ContentLimits::default(),
            limits,
        )
    })
}
fn run(input: &[u8]) -> Program {
    limited(input, ProgramLimits::default()).unwrap()
}
fn kinds(p: &Program) -> Vec<ProgramEventKind> {
    p.events().iter().map(ProgramEvent::kind).collect()
}
#[test]
fn mixed_order_and_global_indices_use_original_operations() {
    let p = run(b"0 0 5 5 re f BT /F 10 Tf (A) Tj ET 1 1 4 4 re S");
    assert_eq!(
        kinds(&p),
        [
            ProgramEventKind::Path(0),
            ProgramEventKind::Text {
                event: 0,
                context: 0
            },
            ProgramEventKind::Text {
                event: 1,
                context: 0
            },
            ProgramEventKind::Path(1)
        ]
    );
    assert_eq!(
        p.events()
            .iter()
            .map(|e| e.operation_index())
            .collect::<Vec<_>>(),
        [1, 4, 5, 7]
    );
    assert_eq!(p.text().codes()[0].operation_index(), 4);
    assert_eq!(p.text().text_objects()[0].operation_index(), 5);
    assert_eq!(p.usage().operations, 8);
    assert_eq!(p.usage().child_operations, 8);
    assert_eq!(p.usage().work_units, 17);
    assert_eq!(p.text().text(), "A");
}
#[test]
fn paint_then_clip_then_text_and_restore_keep_exact_event_order() {
    let p = run(b"/F 10 Tf q 0 0 10 10 re W B BT (A) Tj ET Q BT (B) Tj ET");
    assert!(matches!(p.paths().events()[0], PathEvent::Fill { .. }));
    assert!(matches!(p.paths().events()[1], PathEvent::Stroke { .. }));
    assert!(matches!(p.paths().events()[2], PathEvent::ClipPush { .. }));
    assert!(matches!(
        p.paths().events()[3],
        PathEvent::ClipRestore { .. }
    ));
    assert_eq!(
        kinds(&p),
        [
            ProgramEventKind::Path(0),
            ProgramEventKind::Path(1),
            ProgramEventKind::Path(2),
            ProgramEventKind::Text {
                event: 0,
                context: 0
            },
            ProgramEventKind::Text {
                event: 1,
                context: 0
            },
            ProgramEventKind::Path(3),
            ProgramEventKind::Text {
                event: 2,
                context: 1
            },
            ProgramEventKind::Text {
                event: 3,
                context: 1
            }
        ]
    );
    assert_eq!(p.text_contexts()[0].clip().unwrap().index(), 0);
    assert_eq!(p.text_contexts()[1].clip(), None);
}
#[test]
fn color_inside_text_and_invisible_intent_capture_context() {
    let p = run(b"/F 10 Tf BT 1 0 0 rg (A) Tj 0 1 0 rg 3 Tr (B) Tj 2 w ET");
    assert_eq!(p.text_contexts().len(), 2);
    assert_eq!(
        p.text_contexts()[0].fill_color(),
        DeviceColor::Rgb([1., 0., 0.])
    );
    assert_eq!(
        p.text_contexts()[1].fill_color(),
        DeviceColor::Rgb([0., 1., 0.])
    );
    assert_eq!(p.text().codes()[0].rendering_mode(), 0);
    assert_eq!(p.text().codes()[1].rendering_mode(), 3);
    assert_eq!(p.text().codes()[0].operation_index(), 3);
    assert_eq!(p.text().codes()[1].operation_index(), 6);
}
#[test]
fn one_persistent_stack_restores_font_spacing_color_and_noncommuting_ctm() {
    let p=run(b"/F 10 Tf 2 Tc q 0 2 -3 0 20 30 cm 1 g /F 8 Tf 4 Tc BT 1 0 0 1 10 20 Tm (A) Tj ET Q BT (B) Tj ET");
    assert_eq!(p.text().codes()[0].origin().coordinates(), [-40., 50.]);
    assert_eq!(p.text().codes()[0].font_size(), 8.);
    assert_eq!(p.text().codes()[0].character_spacing(), 4.);
    assert_eq!(p.text_contexts()[0].fill_color(), DeviceColor::Gray(1.));
    assert_eq!(p.text().codes()[1].origin().coordinates(), [0., 0.]);
    assert_eq!(p.text().codes()[1].font_size(), 10.);
    assert_eq!(p.text().codes()[1].character_spacing(), 2.);
    assert_eq!(p.text_contexts()[1].fill_color(), DeviceColor::Gray(0.));
}
#[test]
fn current_paths_survive_allowed_state_changes_without_segment_resets() {
    let p = run(b"0 0 m q 2 w 1 0 0 RG 10 10 l Q S");
    assert_eq!(p.paths().paths()[0].commands().len(), 2);
    let PathEvent::Stroke { paint, style } = &p.paths().events()[0] else {
        panic!()
    };
    assert_eq!(paint.color(), DeviceColor::Gray(0.));
    assert_eq!(style.width(), 1.);
}
#[test]
fn strict_path_text_boundaries_and_unknown_suffix_are_atomic() {
    for input in [
        b"W BT ET".as_slice(),
        b"0 0 m BT ET",
        b"BT 0 0 m ET",
        b"BT n ET",
        b"BT W ET",
        b"BT q ET",
        b"BT Q ET",
        b"BT 1 0 0 1 0 0 cm ET",
        b"/F 10 Tf BT (A) Tj ET /X Do",
        b"0 0 1 1 re f /OC /Layer BDC",
        b"0 0 1 1 re f /DeviceRGB cs",
    ] {
        assert!(
            limited(input, ProgramLimits::default()).is_err(),
            "{}",
            String::from_utf8_lossy(input)
        );
    }
    let p = run(b"W n BT /F 10 Tf (A) Tj ET");
    assert_eq!(p.paths().clips().len(), 1);
    assert_eq!(p.text_contexts()[0].clip().unwrap().index(), 0);
}
#[test]
fn initial_clip_geometry_is_retained_without_culling_text_validation() {
    with_fonts(|fonts| {
        for clip in [
            InitialClip::Empty,
            InitialClip::Rectangle(Rect::<PageSpace>::from_corners(0., 0., 50., 20.).unwrap()),
        ] {
            let initial = ProgramInitialState::new(
                Affine::<UserSpace, PageSpace>::from_pdf([0., -2., 3., 0., 50., 100.]).unwrap(),
                clip,
            );
            let p = interpret_program_content(
                b"BT /F 10 Tf 1 0 0 1 5 7 Tm (A) Tj ET",
                fonts,
                initial,
                ContentLimits::default(),
                ProgramLimits::default(),
            )
            .unwrap();
            assert_eq!(p.paths().initial_clip(), clip);
            assert_eq!(p.text().codes()[0].origin().coordinates(), [71., 90.]);
            assert!(
                interpret_program_content(
                    b"BT /F 10 Tf <00> Tj ET",
                    fonts,
                    initial,
                    ContentLimits::default(),
                    ProgramLimits::default()
                )
                .is_err()
            );
        }
    });
}
#[test]
fn second_child_failure_retains_charges_and_is_sticky() {
    with_fonts(|fonts| {
        let limits = ProgramLimits {
            text: TextPositionLimits {
                max_stack_depth: 0,
                ..TextPositionLimits::default()
            },
            ..ProgramLimits::default()
        };
        let mut i = ProgramInterpreter::new(fonts, ProgramInitialState::default(), limits).unwrap();
        let mut parser = ContentParser::new(b"q Q", ContentLimits::default());
        let q = parser.next_operation().unwrap().unwrap();
        let error = i.push(&validate_operation(&q).unwrap()).unwrap_err();
        assert!(matches!(error.kind, ProgramErrorKind::Text(_)));
        assert_eq!(i.usage().operations, 1);
        assert_eq!(i.usage().child_operations, 2);
        assert_eq!(i.usage().work_units, 3);
        assert!(i.usage().allocated_bytes > 0);
        let usage = i.usage();
        let q = parser.next_operation().unwrap().unwrap();
        assert_eq!(i.push(&validate_operation(&q).unwrap()).unwrap_err(), error);
        assert_eq!(i.usage(), usage);
        assert_eq!(i.finish().unwrap_err(), error);
    });
}
#[test]
fn aggregate_limits_admit_exact_counts_and_refuse_before_show_decode() {
    let input = b"BT /F 10 Tf [(A) 10 (B)] TJ ET";
    let limits = ProgramLimits {
        max_operations: 4,
        max_events: 4,
        max_text_contexts: 1,
        max_work_units: 13,
        ..ProgramLimits::default()
    };
    let p = limited(input, limits).unwrap();
    assert_eq!(p.events().len(), 4);
    assert_eq!(p.usage().work_units, 13);
    for set in [
        |l: &mut ProgramLimits| l.max_operations = 3,
        |l: &mut ProgramLimits| l.max_events = 3,
        |l: &mut ProgramLimits| l.max_text_contexts = 0,
        |l: &mut ProgramLimits| l.max_work_units = 12,
    ] {
        let mut small = limits;
        set(&mut small);
        assert!(limited(input, small).is_err());
    }
    let error = limited(
        b"BT /F 10 Tf <0000> Tj ET",
        ProgramLimits {
            max_events: 1,
            ..ProgramLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(
        error.kind,
        ProgramErrorKind::LimitExceeded(ProgramLimitKind::Events)
    );
    assert_eq!(error.operation_index, 2);
    assert!(
        limited(
            b"q q Q Q",
            ProgramLimits {
                max_save_depth: 1,
                ..ProgramLimits::default()
            }
        )
        .is_err()
    );
}
#[test]
fn shared_allocation_boundary_accounts_both_children_and_parent_buffers() {
    let input = b"q 0 0 2 2 re W f BT /F 10 Tf (AB) Tj ET Q";
    let p = run(input);
    let exact = p.usage().allocated_bytes;
    assert!(exact > 1000);
    limited(
        input,
        ProgramLimits {
            max_allocated_bytes: exact,
            ..ProgramLimits::default()
        },
    )
    .unwrap();
    // P0 may shrink geometric requests under a tighter allowance; find the
    // actual minimal sufficient cap rather than assuming one final request plan.
    let (mut low, mut high) = (0, exact);
    while low < high {
        let mid = (low + high) / 2;
        if limited(
            input,
            ProgramLimits {
                max_allocated_bytes: mid,
                ..ProgramLimits::default()
            },
        )
        .is_ok()
        {
            high = mid
        } else {
            low = mid + 1;
        }
    }
    let p = limited(
        input,
        ProgramLimits {
            max_allocated_bytes: low,
            ..ProgramLimits::default()
        },
    )
    .unwrap();
    assert!(p.usage().allocated_bytes <= low);
    assert!(
        limited(
            input,
            ProgramLimits {
                max_allocated_bytes: low - 1,
                ..ProgramLimits::default()
            }
        )
        .is_err()
    );
}
#[test]
fn parent_budget_caps_second_child_before_reserve() {
    with_fonts(|fonts| {
        let mut probe = ProgramInterpreter::new(
            fonts,
            ProgramInitialState::default(),
            ProgramLimits {
                text: TextPositionLimits {
                    max_stack_depth: 0,
                    ..TextPositionLimits::default()
                },
                ..ProgramLimits::default()
            },
        )
        .unwrap();
        let op = ContentParser::new(b"q", ContentLimits::default())
            .next_operation()
            .unwrap()
            .unwrap();
        probe.push(&validate_operation(&op).unwrap()).unwrap_err();
        let first = probe.usage().allocated_bytes;
        let mut limited = ProgramInterpreter::new(
            fonts,
            ProgramInitialState::default(),
            ProgramLimits {
                max_allocated_bytes: first,
                ..ProgramLimits::default()
            },
        )
        .unwrap();
        assert!(limited.push(&validate_operation(&op).unwrap()).is_err());
        assert_eq!(limited.usage().allocated_bytes, first);
        assert_eq!(limited.usage().child_operations, 2);
    });
}
#[test]
fn zero_budget_empty_trace_and_nonfinite_second_shared_step() {
    let p = limited(
        b"",
        ProgramLimits {
            max_operations: 0,
            max_events: 0,
            max_text_contexts: 0,
            max_work_units: 0,
            max_allocated_bytes: 0,
            ..ProgramLimits::default()
        },
    )
    .unwrap();
    assert_eq!(p.usage(), ProgramUsage::default());
    assert!(limited(b"1 0 0 1 10000000000 0 cm", ProgramLimits::default()).is_err());
    let p = run(b"1 0 -0 1 -0 0 cm BT /F 10 Tf (A) Tj ET");
    assert_eq!(p.text().codes()[0].origin().coordinates(), [0., 0.]);
}
#[test]
fn empty_path_events_and_numeric_only_arrays_are_counted_exactly() {
    let p = limited(
        b"n S f B BT /F 10 Tf [10 -20] TJ ET",
        ProgramLimits {
            max_events: 3,
            ..ProgramLimits::default()
        },
    )
    .unwrap();
    assert!(p.paths().events().is_empty());
    assert_eq!(p.events().len(), 3);
    assert_eq!(
        p.text().events(),
        [
            TextEvent::Adjustment(0),
            TextEvent::Adjustment(1),
            TextEvent::EndTextObject(0)
        ]
    );
}
#[test]
fn parent_path_and_text_child_work_limits_remain_separate() {
    let p = limited(
        b"0 g BT /F 10 Tf (A) Tj ET",
        ProgramLimits {
            path: PathLimits {
                max_operations: 1,
                ..PathLimits::default()
            },
            text: TextPositionLimits {
                max_operations: 4,
                ..TextPositionLimits::default()
            },
            max_operations: 5,
            ..ProgramLimits::default()
        },
    )
    .unwrap();
    assert_eq!(p.text().codes()[0].operation_index(), 3);
    assert_eq!(p.usage().operations, 5);
}
