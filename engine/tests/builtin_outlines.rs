// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::builtin_metrics::BuiltinFace;
use tandem_pdf_engine::builtin_outlines::{BuiltinOutline, BuiltinOutlineCommand};
#[test]
fn exact_asset_membership_matches_independent_pinned_metric_membership() {
    for face in [BuiltinFace::Helvetica, BuiltinFace::TimesRoman] {
        let outlines: Vec<_> = BuiltinOutline::glyphs(face).collect();
        let metrics: Vec<_> = face.glyphs().collect();
        assert_eq!(outlines.len(), 654);
        assert_eq!(outlines.len(), metrics.len());
        for (outline, (name, _)) in outlines.into_iter().zip(metrics) {
            assert_eq!(outline.name(), name);
            assert_eq!(outline.face(), face);
            assert!(std::ptr::eq(
                outline,
                BuiltinOutline::lookup(face, name).unwrap()
            ));
        }
        assert!(BuiltinOutline::lookup(face, b".notdef").is_some());
        assert!(
            BuiltinOutline::lookup(face, b"space")
                .unwrap()
                .commands()
                .is_empty()
        );
        for name in [
            b"Space".as_slice(),
            b"A.alt",
            b"no-such-glyph",
            &[b'A'; 129],
        ] {
            assert!(BuiltinOutline::lookup(face, name).is_none());
        }
    }
}
#[test]
fn capital_a_retains_afm_drawing_bounds_and_independent_advance() {
    // Independently specified from the pinned URW AFM A records. These bounds
    // are in the actual font origin, without a TrueType side-bearing shift.
    for (face, bounds, advance) in [
        (BuiltinFace::Helvetica, [17, 0, 653, 729], 667.),
        (BuiltinFace::TimesRoman, [15, 0, 706, 674], 722.),
    ] {
        let a = BuiltinOutline::lookup(face, b"A").unwrap();
        assert_eq!(a.units_per_em(), 1000);
        assert_eq!(face.lookup_glyph(b"A").unwrap().value(), advance);
        let mut points = Vec::new();
        for command in a.commands() {
            match *command {
                BuiltinOutlineCommand::MoveTo { x, y } | BuiltinOutlineCommand::LineTo { x, y } => {
                    points.push((x, y))
                }
                BuiltinOutlineCommand::CubicTo {
                    x1,
                    y1,
                    x2,
                    y2,
                    x,
                    y,
                } => points.extend([(x1, y1), (x2, y2), (x, y)]),
                BuiltinOutlineCommand::Close => {}
            }
        }
        assert_eq!(
            [
                points.iter().map(|p| p.0).min().unwrap(),
                points.iter().map(|p| p.1).min().unwrap(),
                points.iter().map(|p| p.0).max().unwrap(),
                points.iter().map(|p| p.1).max().unwrap()
            ],
            bounds
        );
    }
}
