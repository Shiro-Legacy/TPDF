// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent R20 static asset, PDF binding and policy review.
#[test]
fn every_static_record_matches_metric_membership_ranges_and_closed_command_grammar() {
    use tandem_pdf_engine::{
        builtin_metrics::BuiltinFace,
        builtin_outlines::{BuiltinOutline, BuiltinOutlineCommand as C, MAX_GLYPH_NAME_BYTES},
    };
    let mut totals = [0usize; 4];
    for (face, expected) in [
        (BuiltinFace::Helvetica, 12788),
        (BuiltinFace::TimesRoman, 17838),
    ] {
        let records = BuiltinOutline::glyphs(face).collect::<Vec<_>>();
        assert_eq!(records.len(), 654);
        let metric_names = face.glyphs().map(|(name, _)| name).collect::<Vec<_>>();
        assert_eq!(
            records.iter().map(|r| r.name()).collect::<Vec<_>>(),
            metric_names
        );
        let mut end = 0;
        let mut seen = std::collections::BTreeSet::new();
        for record in records {
            assert_eq!(record.face(), face);
            assert_eq!(record.units_per_em(), 1000);
            assert!(!record.name().is_empty());
            assert!(record.name().len() <= MAX_GLYPH_NAME_BYTES);
            assert!(seen.insert(record.native_gid()));
            assert_eq!(record.command_range().start, end);
            assert_eq!(record.command_range().len(), record.commands().len());
            end = record.command_range().end;
            assert!(std::ptr::eq(
                BuiltinOutline::lookup(face, record.name()).unwrap(),
                record
            ));
            let mut open = false;
            for command in record.commands() {
                let coordinates = match *command {
                    C::MoveTo { x, y } => {
                        assert!(!open, "unclosed {}", String::from_utf8_lossy(record.name()));
                        open = true;
                        totals[0] += 1;
                        vec![x, y]
                    }
                    C::LineTo { x, y } => {
                        assert!(open);
                        totals[1] += 1;
                        vec![x, y]
                    }
                    C::CubicTo {
                        x1,
                        y1,
                        x2,
                        y2,
                        x,
                        y,
                    } => {
                        assert!(open);
                        totals[2] += 1;
                        vec![x1, y1, x2, y2, x, y]
                    }
                    C::Close => {
                        assert!(open);
                        open = false;
                        totals[3] += 1;
                        vec![]
                    }
                };
                assert!(coordinates.iter().all(|v| v.unsigned_abs() <= 1 << 20));
            }
            assert!(!open);
        }
        assert_eq!(end, expected);
        assert!(
            BuiltinOutline::lookup(face, b"space")
                .unwrap()
                .commands()
                .is_empty()
        );
        assert_eq!(
            BuiltinOutline::lookup(face, b".notdef")
                .unwrap()
                .native_gid(),
            0
        );
    }
    assert_eq!(totals, [2429, 13943, 11825, 2429]);
    assert!(BuiltinOutline::static_bytes() < 2 * 1024 * 1024);
}
#[test]
fn exact_static_lookup_rejects_long_names_case_aliases_and_invalid_bytes() {
    use tandem_pdf_engine::{builtin_metrics::BuiltinFace, builtin_outlines::BuiltinOutline};
    for face in [BuiltinFace::Helvetica, BuiltinFace::TimesRoman] {
        for name in [
            b"A.alt".as_slice(),
            b"SPACE",
            b"A\0",
            b"\xff",
            b"UnknownReviewName",
        ] {
            assert!(BuiltinOutline::lookup(face, name).is_none());
        }
        assert!(BuiltinOutline::lookup(face, &[b'A'; 129]).is_none());
        assert!(BuiltinOutline::lookup_work_bound(face) > 128);
    }
}
