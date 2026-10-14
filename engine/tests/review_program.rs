// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent mixed coordinator state, provenance, and resource boundary tests.
use tandem_pdf_engine::{
    content::{ContentLimits, ContentOperand, ContentParser, Operation},
    font_metrics::{FontAdvance, HorizontalMetrics},
    operators::validate_operation,
    path::{DeviceColor, InitialClip},
    program::*,
    simple_encoding::{NamedEncoding, SimpleEncoding},
    text_font::{TextFont, TextFontLimits, TextFonts, UnicodeSource},
    text_position::{TextEvent, TextPositionLimits},
};
fn with_fonts<T>(f: impl FnOnce(&TextFonts<'_>) -> T) -> T {
    let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let metrics =
        HorizontalMetrics::from_widths(0, 255, &vec![FontAdvance::new(500.).unwrap(); 256], None)
            .unwrap();
    let font = TextFont::declared(UnicodeSource::Encoding(&encoding), &metrics).unwrap();
    let fonts = TextFonts::new(&[(b"F", font)], TextFontLimits::default()).unwrap();
    f(&fonts)
}
fn run(
    input: &[u8],
    fonts: &TextFonts<'_>,
    limits: ProgramLimits,
) -> Result<Program, ProgramError> {
    interpret_program_content(
        input,
        fonts,
        ProgramInitialState::default(),
        ContentLimits::default(),
        limits,
    )
}
fn feed(interpreter: &mut ProgramInterpreter<'_, '_>, input: &[u8]) -> Result<(), ProgramError> {
    let mut parser = ContentParser::new(input, ContentLimits::default());
    while let Some(operation) = parser.next_operation().unwrap() {
        interpreter.push(&validate_operation(&operation).unwrap())?;
    }
    Ok(())
}
#[test]
fn second_broadcast_child_failure_retains_first_child_cost_and_sticks() {
    with_fonts(|fonts| {
        let mut interpreter = ProgramInterpreter::new(
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
        let operation = Operation {
            operator: b"q".to_vec(),
            operands: vec![],
            span: 8..9,
        };
        let validated = validate_operation(&operation).unwrap();
        let error = interpreter.push(&validated).unwrap_err();
        assert!(matches!(error.kind, ProgramErrorKind::Text(_)));
        assert_eq!(error.operation_index, 0);
        assert_eq!(error.span, 8..9);
        let used = interpreter.usage();
        assert_eq!(used.operations, 1);
        assert_eq!(used.child_operations, 2);
        assert_eq!(used.work_units, 3);
        assert!(used.allocated_bytes > 0);
        assert_eq!(interpreter.push(&validated).unwrap_err(), error);
        assert_eq!(interpreter.usage(), used);
        assert_eq!(interpreter.finish().unwrap_err(), error);
    });
}
#[test]
fn empty_pending_clip_cannot_be_discarded_at_text_boundary() {
    with_fonts(|fonts| {
        for input in [
            b"W BT ET".as_slice(),
            b"W* q Q BT ET",
            b"0 0 m BT ET",
            b"BT 0 0 m ET",
            b"BT n ET",
        ] {
            let error = run(input, fonts, ProgramLimits::default()).unwrap_err();
            assert!(matches!(error.kind, ProgramErrorKind::State(_)));
        }
        assert!(run(b"q W n Q BT ET", fonts, ProgramLimits::default()).is_ok());
    });
}
#[test]
fn path_only_operations_do_not_reset_text_or_global_event_indices() {
    with_fonts(|fonts| {
        let raw = b"/F 8 Tf 1 0 0 rg q 0 0 2 2 re B BT (A) Tj ET Q";
        let result = run(raw, fonts, ProgramLimits::default()).unwrap();
        assert_eq!(result.text().codes()[0].operation_index(), 6);
        assert_eq!(
            result
                .events()
                .iter()
                .map(|e| e.operation_index())
                .collect::<Vec<_>>(),
            [4, 4, 6, 7]
        );
        assert!(matches!(
            result.events()[0].kind(),
            ProgramEventKind::Path(0)
        ));
        assert!(matches!(
            result.events()[1].kind(),
            ProgramEventKind::Path(1)
        ));
        assert_eq!(result.usage().operations, 9);
        assert_eq!(result.usage().child_operations, 11);
        let mut parser = ContentParser::new(raw, ContentLimits::default());
        let mut spans = Vec::new();
        while let Some(op) = parser.next_operation().unwrap() {
            spans.push(op.span);
        }
        for event in result.events() {
            assert_eq!(event.span(), spans[event.operation_index()]);
        }
        assert_eq!(result.text().codes()[0].operation_span(), spans[6]);
    });
}
#[test]
fn color_clip_and_invisible_intent_snapshots_survive_nested_restores() {
    with_fonts(|fonts| {
        let raw=b"/F 8 Tf 1 0 0 rg q 0 0 20 20 re W n BT (A) Tj 0 1 0 rg 3 Tr (B) Tj ET q 2 2 5 5 re W n BT (C) Tj ET Q BT (D) Tj ET Q BT (E) Tj ET";
        let result = run(raw, fonts, ProgramLimits::default()).unwrap();
        let mut contexts = Vec::new();
        for event in result.events() {
            if let ProgramEventKind::Text { event, context } = event.kind() {
                if let TextEvent::Code(code) = result.text().events()[event] {
                    contexts.push((code, result.text_contexts()[context]));
                }
            }
        }
        assert_eq!(contexts.len(), 5);
        let clips = [Some(0), Some(0), Some(1), Some(0), None];
        let modes = [0, 3, 3, 3, 0];
        for (i, (code, context)) in contexts.iter().enumerate() {
            assert_eq!(*code, i);
            assert_eq!(context.clip().map(|v| v.index()), clips[i]);
            assert_eq!(result.text().codes()[i].rendering_mode(), modes[i]);
            assert_eq!(
                context.fill_color(),
                if i == 0 || i == 4 {
                    DeviceColor::Rgb([1., 0., 0.])
                } else {
                    DeviceColor::Rgb([0., 1., 0.])
                }
            );
        }
        assert_eq!(result.paths().clips()[1].parent().unwrap().index(), 0);
    });
}
#[test]
fn aggregate_allocation_work_and_event_edges_are_enforced_before_publication() {
    with_fonts(|fonts| {
        let raw = b"/F 8 Tf q 0 0 3 4 re W B BT [(A) 125 (B)] TJ ET Q";
        let full = run(raw, fonts, ProgramLimits::default()).unwrap();
        let used = full.usage();
        for (limits, kind) in [
            (
                ProgramLimits {
                    max_operations: used.operations - 1,
                    ..ProgramLimits::default()
                },
                ProgramLimitKind::Operations,
            ),
            (
                ProgramLimits {
                    max_work_units: used.work_units - 1,
                    ..ProgramLimits::default()
                },
                ProgramLimitKind::WorkUnits,
            ),
            (
                ProgramLimits {
                    max_events: full.events().len() - 1,
                    ..ProgramLimits::default()
                },
                ProgramLimitKind::Events,
            ),
            (
                ProgramLimits {
                    max_text_contexts: 0,
                    ..ProgramLimits::default()
                },
                ProgramLimitKind::TextContexts,
            ),
        ] {
            assert_eq!(
                run(raw, fonts, limits).unwrap_err().kind,
                ProgramErrorKind::LimitExceeded(kind)
            );
        }
        let edge_limits = ProgramLimits {
            max_operations: used.operations,
            max_work_units: used.work_units,
            max_events: full.events().len(),
            max_text_contexts: full.text_contexts().len(),
            ..ProgramLimits::default()
        };
        let mut low = 0;
        let mut high = used.allocated_bytes;
        while low < high {
            let mid = low + (high - low) / 2;
            if run(
                raw,
                fonts,
                ProgramLimits {
                    max_allocated_bytes: mid,
                    ..edge_limits
                },
            )
            .is_ok()
            {
                high = mid
            } else {
                low = mid + 1
            }
        }
        let exact = ProgramLimits {
            max_operations: used.operations,
            max_work_units: used.work_units,
            max_events: full.events().len(),
            max_text_contexts: full.text_contexts().len(),
            max_allocated_bytes: low,
            ..ProgramLimits::default()
        };
        assert!(run(raw, fonts, exact).is_ok());
        assert!(low > 0);
        assert!(
            run(
                raw,
                fonts,
                ProgramLimits {
                    max_allocated_bytes: low - 1,
                    ..exact
                }
            )
            .is_err()
        );
        let mut interpreter = ProgramInterpreter::new(
            fonts,
            ProgramInitialState::default(),
            ProgramLimits {
                max_allocated_bytes: low - 1,
                ..exact
            },
        )
        .unwrap();
        assert!(feed(&mut interpreter, raw).is_err());
        assert!(interpreter.usage().allocated_bytes < low);
        assert!(interpreter.finish().is_err());
    });
}
#[test]
fn failed_late_tj_preflight_retains_array_traversal_work() {
    with_fonts(|fonts| {
        let mut interpreter = ProgramInterpreter::new(
            fonts,
            ProgramInitialState::default(),
            ProgramLimits {
                max_events: 0,
                ..ProgramLimits::default()
            },
        )
        .unwrap();
        feed(&mut interpreter, b"/F 8 Tf BT").unwrap();
        let before = interpreter.usage();
        let mut values = vec![ContentOperand::String(vec![]); 100];
        values.push(ContentOperand::Integer(0));
        let operation = Operation {
            operator: b"TJ".to_vec(),
            operands: vec![ContentOperand::Array(values)],
            span: 20..30,
        };
        let error = interpreter
            .push(&validate_operation(&operation).unwrap())
            .unwrap_err();
        assert_eq!(
            error.kind,
            ProgramErrorKind::LimitExceeded(ProgramLimitKind::Events)
        );
        assert!(
            interpreter.usage().work_units >= before.work_units + 102,
            "must retain global operation plus admitted101-element traversal even though event preflight fails"
        );
        assert_eq!(
            interpreter.usage().child_operations,
            before.child_operations
        );
        assert_eq!(interpreter.finish().unwrap_err(), error);
    });
}
#[test]
fn empty_initial_clip_preserves_validation_and_position_trace() {
    with_fonts(|fonts| {
        let initial =
            ProgramInitialState::new(ProgramInitialState::default().ctm(), InitialClip::Empty);
        let result = interpret_program_content(
            b"/F 8 Tf BT (A) Tj ET 0 0 2 3 re f",
            fonts,
            initial,
            ContentLimits::default(),
            ProgramLimits::default(),
        )
        .unwrap();
        assert_eq!(result.paths().initial_clip(), InitialClip::Empty);
        assert_eq!(result.text().codes().len(), 1);
        assert_eq!(result.paths().events().len(), 1);
        assert!(
            interpret_program_content(
                b"/F 8 Tf BT (A) Tj ET /X Do",
                fonts,
                initial,
                ContentLimits::default(),
                ProgramLimits::default()
            )
            .is_err()
        );
    });
}
