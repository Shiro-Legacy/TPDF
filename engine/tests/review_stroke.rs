// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent analytic geometry and cumulative admission review for R21.
use tandem_pdf_engine::{
    content::ContentLimits,
    geometry::{Affine, DeviceSpace, Point, UserSpace},
    path::{PathCommand, PathEvent, PathProgram, interpret_path_content},
    stroke::*,
};
fn parse(s: &str) -> PathProgram {
    interpret_path_content(
        s.as_bytes(),
        Default::default(),
        ContentLimits::default(),
        Default::default(),
    )
    .unwrap()
}
fn map(a: [f64; 6]) -> Affine<UserSpace, DeviceSpace> {
    Affine::from_pdf(a).unwrap()
}
fn identity() -> Affine<UserSpace, DeviceSpace> {
    map([1., 0., 0., 1., 0., 0.])
}
fn expand(
    p: &PathProgram,
    session: &mut StrokeSession,
    m: Affine<UserSpace, DeviceSpace>,
    limits: StrokeLimits,
) -> Result<StrokeOutline, StrokeError> {
    let PathEvent::Stroke { paint, style } = &p.events()[0] else {
        panic!("stroke expected")
    };
    session.expand(
        p.paths()[paint.path().index()].commands(),
        *style,
        style.dash().map(|id| &p.dashes()[id.index()]),
        m,
        limits,
    )
}
fn outline(s: &str) -> StrokeOutline {
    expand(
        &parse(s),
        &mut StrokeSession::new(Default::default()),
        identity(),
        Default::default(),
    )
    .unwrap()
}
fn xy(p: Point<UserSpace>) -> [f64; 2] {
    [p.x(), p.y()]
}
fn cross(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}
fn inside(o: &StrokeOutline, p: [f64; 2]) -> bool {
    let mut winding = 0i32;
    let mut first = None;
    let mut last = None;
    let mut edge = |a: [f64; 2], b: [f64; 2]| {
        let side = cross(a, b, p);
        if a[1] <= p[1] && p[1] < b[1] && side > 0. {
            winding += 1;
        }
        if b[1] <= p[1] && p[1] < a[1] && side < 0. {
            winding -= 1;
        }
    };
    for c in o.commands() {
        match *c {
            PathCommand::MoveTo(q) => {
                assert!(last.is_none());
                first = Some(xy(q));
                last = first;
            }
            PathCommand::LineTo(q) => {
                let q = xy(q);
                edge(last.unwrap(), q);
                last = Some(q);
            }
            PathCommand::ClosePath => {
                edge(last.unwrap(), first.unwrap());
                first = None;
                last = None;
            }
            _ => panic!("stroke output must contain only closed polygon components"),
        }
    }
    assert!(last.is_none());
    winding != 0
}
#[test]
fn line_caps_match_independent_distance_and_projection_model() {
    for cap in 0..3 {
        let out = outline(&format!("2 w {cap} J 2 3 m 8 11 l S"));
        for ix in -12..50 {
            for iy in -8..62 {
                let p = [f64::from(ix) * 0.25 + 0.013, f64::from(iy) * 0.25 + 0.019];
                let v = [p[0] - 2., p[1] - 3.];
                let t = (6. * v[0] + 8. * v[1]) / 10.;
                let n = (-8. * v[0] + 6. * v[1]) / 10.;
                let expected = match cap {
                    0 => t > 0. && t < 10. && n.abs() < 1.,
                    2 => t > -1. && t < 11. && n.abs() < 1.,
                    _ => {
                        let along = if t < 0. {
                            t
                        } else if t > 10. {
                            t - 10.
                        } else {
                            0.
                        };
                        along * along + n * n < 1.
                    }
                };
                // Exclude a conservative device band around any analytic boundary.
                let distance = if cap == 1 {
                    let along = if t < 0. {
                        t
                    } else if t > 10. {
                        t - 10.
                    } else {
                        0.
                    };
                    (along * along + n * n).sqrt() - 1.
                } else {
                    (n.abs() - 1.)
                        .abs()
                        .min((t - if cap == 2 { -1. } else { 0. }).abs())
                        .min((t - if cap == 2 { 11. } else { 10. }).abs())
                };
                if distance.abs() > 1. / 32. {
                    assert_eq!(inside(&out, p), expected, "cap {cap}, point {p:?}");
                }
            }
        }
    }
}
#[test]
fn right_angle_outer_joins_distinguish_bevel_round_and_miter_on_both_sides() {
    for sign in [-1., 1.] {
        for join in 0..3 {
            let out = outline(&format!("2 w {join} j -4 0 m 0 0 l 0 {} l S", sign * 4.));
            for ix in 1..16 {
                for iy in 1..16 {
                    let x = f64::from(ix) / 16. + 0.007;
                    let y = f64::from(iy) / 16. + 0.011;
                    let p = [x, -sign * y];
                    let value = match join {
                        0 => x.max(y),
                        1 => (x * x + y * y).sqrt(),
                        _ => x + y,
                    };
                    if (value - 1.).abs() > 1. / 32. {
                        assert_eq!(inside(&out, p), value < 1., "join{join} sign{sign} {p:?}");
                    }
                }
            }
        }
    }
    let clipped = outline("2 w 1 M 0 j -4 0 m 0 0 l 0 4 l S");
    assert!(!inside(&clipped, [0.75, -0.75]));
    let miter = outline("2 w 2 M 0 j -4 0 m 0 0 l 0 4 l S");
    assert!(inside(&miter, [0.75, -0.75]));
}
#[test]
fn closed_seam_has_no_caps_and_components_union_preserves_hole() {
    let a = outline("1 w 0 J 0 j 0 0 4 4 re S");
    let b = outline("1 w 1 J 0 j 0 0 4 4 re S");
    assert_eq!(a.commands(), b.commands());
    assert!(!inside(&a, [2., 2.]));
    assert!(inside(&a, [2., 0.1]));
    assert!(inside(&a, [-0.25, -0.25]));
    assert!(!inside(&a, [-0.75, -0.75]));
    let crossing = outline("1 w 0 J -3 -3 m 3 3 l -3 3 m 3 -3 l S");
    assert!(inside(&crossing, [0.03, 0.01]));
}
#[test]
fn affine_admission_keeps_user_geometry_and_owned_output_lifetime() {
    let saved = {
        let p = parse("2 w 2 J 2 3 m 8 11 l S");
        let mut session = StrokeSession::new(Default::default());
        let out = expand(
            &p,
            &mut session,
            map([-2., 0.5, 1., 3., 7., -4.]),
            Default::default(),
        )
        .unwrap();
        assert!(session.usage().output_points > 0);
        out
    };
    let ordinary = outline("2 w 2 J 2 3 m 8 11 l S");
    assert_eq!(saved.commands(), ordinary.commands());
    assert!(inside(&saved, [4., 6.]));
}
#[test]
fn dash_binding_cannot_silently_drop_a_saved_pattern_and_empty_dash_is_solid() {
    let p = parse("[1 2] 0 d 0 0 m 4 0 l S");
    let PathEvent::Stroke { paint, style } = &p.events()[0] else {
        panic!()
    };
    let mut session = StrokeSession::new(Default::default());
    let e = session
        .expand(
            p.paths()[paint.path().index()].commands(),
            *style,
            None,
            identity(),
            Default::default(),
        )
        .unwrap_err();
    assert!(matches!(e.kind, StrokeErrorKind::Invariant(_)));
    // Bound correctly, [1 2] on 0..4 leaves dashes 0..1 and 3..4: two
    // separate capped components instead of the solid line's one.
    let dashed = expand(&p, &mut session, identity(), Default::default()).unwrap();
    assert_eq!(session.usage().attempts, 2);
    let moves = |o: &StrokeOutline| {
        o.commands()
            .iter()
            .filter(|c| matches!(c, PathCommand::MoveTo(_)))
            .count()
    };
    assert_eq!(moves(&dashed), 2);
    assert_eq!(moves(&outline("0 0 m 4 0 l S")), 1);
    let solid = outline("0 0 m 4 0 l S");
    let empty = outline("[] 100 d 0 0 m 4 0 l S");
    assert_eq!(solid.commands(), empty.commands());
    let p = parse("0 0 m 4 0 l S");
    let PathEvent::Stroke { paint, style } = &p.events()[0] else {
        panic!()
    };
    let error = session
        .expand(
            p.paths()[paint.path().index()].commands(),
            *style,
            Some(&parse("[1 2] 0 d 0 0 m 4 0 l S").dashes()[0]),
            identity(),
            Default::default(),
        )
        .unwrap_err();
    assert!(matches!(error.kind, StrokeErrorKind::Invariant(_)));
}
#[test]
fn hairlines_zero_tangents_reversals_and_folds_refuse_with_bounded_attempts() {
    for content in [
        "0 w 0 0 m 4 0 l S",
        "0 0 m 0 0 l S",
        "0 0 m 4 0 l 0 0 l S",
        "0 0 m 0 0 1 1 2 0 c S",
        "0 0 m 2 3 -2 3 0 0 c S",
        "100 w 0 0 m 1 1 2 1 3 0 c S",
    ] {
        let p = parse(content);
        let mut session = StrokeSession::new(Default::default());
        let cap = StrokeLimits {
            max_curve_nodes: 128,
            max_subdivision_depth: 8,
            ..StrokeLimits::default()
        };
        assert!(
            expand(&p, &mut session, identity(), cap).is_err(),
            "{content}"
        );
        assert_eq!(session.usage().attempts, 1);
        assert!(session.usage().curve_nodes <= 128);
    }
}
#[test]
fn stress_regular_cubic_is_admitted_and_straight_cubic_matches_line_samples() {
    let mut session = StrokeSession::new(Default::default());
    let stress = expand(
        &parse("1 w 72 60 m 200 200 400 200 540 60 c S"),
        &mut session,
        identity(),
        Default::default(),
    )
    .unwrap();
    assert!(session.usage().curve_nodes > 1);
    assert!(session.usage().curve_leaves > 1);
    assert!(inside(&stress, [286.5, 165.]));
    assert!(!inside(&stress, [286.5, 167.]));
    let straight = outline("2 w 0 0 m 1 0 3 0 4 0 c S");
    let line = outline("2 w 0 0 m 4 0 l S");
    for x in -4..21 {
        for y in -8..9 {
            let p = [f64::from(x) / 4. + 0.013, f64::from(y) / 4. + 0.019];
            assert_eq!(inside(&straight, p), inside(&line, p));
        }
    }
}
#[test]
fn failed_suffix_retains_allocations_and_work_and_never_replaces_prior_owned_output() {
    let mut session = StrokeSession::new(Default::default());
    let prior = expand(
        &parse("1 w 0 0 m 4 0 l S"),
        &mut session,
        identity(),
        Default::default(),
    )
    .unwrap();
    let saved = prior.commands().to_vec();
    let before = session.usage();
    let bad = parse("1 w 0 0 m 4 0 l 4 0 4 0 5 1 c S");
    assert!(expand(&bad, &mut session, identity(), Default::default()).is_err());
    let after = session.usage();
    assert!(after.work > before.work);
    assert!(after.allocated_bytes >= before.allocated_bytes);
    assert_eq!(after.attempts, before.attempts + 1);
    assert_eq!(prior.commands(), saved);
}
#[test]
fn exact_then_one_less_counters_and_cumulative_second_call_are_enforced() {
    let p = parse("2 w 1 J 1 j 0 0 m 10 0 l 15 0 15 10 20 10 c 30 10 l S");
    let mut session = StrokeSession::new(Default::default());
    expand(&p, &mut session, identity(), Default::default()).unwrap();
    let used = session.usage();
    macro_rules! check {
        ($limit:ident,$usage:ident) => {{
            assert!(used.$usage > 0, stringify!($usage));
            let mut exact = StrokeLimits::default();
            exact.$limit = used.$usage;
            let mut s = StrokeSession::new(exact);
            assert!(
                expand(&p, &mut s, identity(), Default::default()).is_ok(),
                stringify!($usage)
            );
            assert!(
                expand(&p, &mut s, identity(), Default::default()).is_err(),
                stringify!($usage)
            );
            let mut short = exact;
            short.$limit -= 1;
            assert!(
                expand(
                    &p,
                    &mut StrokeSession::new(Default::default()),
                    identity(),
                    short
                )
                .is_err(),
                stringify!($usage)
            );
        }};
    }
    check!(max_attempts, attempts);
    check!(max_input_commands, input_commands);
    check!(max_subpaths, subpaths);
    check!(max_line_segments, line_segments);
    check!(max_curve_nodes, curve_nodes);
    check!(max_curve_leaves, curve_leaves);
    check!(max_join_attempts, join_attempts);
    check!(max_cap_attempts, cap_attempts);
    check!(max_arc_nodes, arc_nodes);
    check!(max_arc_vertices, arc_vertices);
    check!(max_components, components);
    check!(max_output_commands, output_commands);
    check!(max_output_points, output_points);
    check!(max_allocated_bytes, allocated_bytes);
    check!(max_work, work);
}
#[test]
fn usage_helpers_fail_atomically_on_overflow_or_inconsistent_remaining() {
    let mut u = StrokeUsage {
        work: u64::MAX,
        ..Default::default()
    };
    let prior = u;
    assert!(
        u.accumulate(StrokeUsage {
            attempts: 1,
            work: 1,
            ..Default::default()
        })
        .is_err()
    );
    assert_eq!(u, prior);
    assert!(
        StrokeLimits::default()
            .remaining(StrokeUsage {
                input_commands: usize::MAX,
                ..Default::default()
            })
            .is_err()
    );
    let cap = StrokeLimits {
        max_subdivision_depth: usize::MAX,
        max_arc_depth: usize::MAX,
        ..Default::default()
    }
    .intersect(StrokeLimits {
        max_subdivision_depth: usize::MAX,
        max_arc_depth: usize::MAX,
        ..Default::default()
    });
    assert_eq!(cap.max_subdivision_depth, 24);
    assert_eq!(cap.max_arc_depth, 24);
}
#[test]
fn close_then_nonmove_continuation_is_never_silently_discarded() {
    for source in [
        "1 w 0 0 m 4 0 l 4 4 l h 9 9 l S",
        "1 w 0 0 m 4 0 l 4 4 l h 2 0 3 1 9 9 c S",
    ] {
        let p = parse(source);
        let mut session = StrokeSession::new(Default::default());
        let error = expand(&p, &mut session, identity(), Default::default()).unwrap_err();
        assert!(matches!(
            error.kind,
            StrokeErrorKind::Unsupported(_) | StrokeErrorKind::Invariant(_)
        ));
        assert!(error.command_index.is_some());
    }
}
#[test]
fn move_only_zero_length_and_redundant_close_have_distinct_policies() {
    let p = parse("1 w 0 0 m 4 0 l S");
    let PathEvent::Stroke { style, .. } = p.events()[0] else {
        panic!()
    };
    let origin = Point::<UserSpace>::new(0., 0.).unwrap();
    let mut session = StrokeSession::new(Default::default());
    let empty = session
        .expand(
            &[PathCommand::MoveTo(origin)],
            style,
            None,
            identity(),
            Default::default(),
        )
        .unwrap();
    assert!(empty.commands().is_empty());
    assert!(
        session
            .expand(
                &[PathCommand::MoveTo(origin), PathCommand::LineTo(origin)],
                style,
                None,
                identity(),
                Default::default()
            )
            .is_err()
    );
    assert!(
        session
            .expand(
                &[PathCommand::MoveTo(origin), PathCommand::ClosePath],
                style,
                None,
                identity(),
                Default::default()
            )
            .is_err()
    );
    let regular = outline("1 w 0 0 m 4 0 l 4 4 l h S");
    let redundant = outline("1 w 0 0 m 4 0 l 4 4 l h h S");
    assert_eq!(regular.commands(), redundant.commands());
}
#[test]
fn rounded_zero_cross_product_cannot_authorize_collinear_join() {
    // Exact integer cross is one; binary64 products round it to zero.
    let n = 1_073_741_824_f64;
    assert_eq!(n * n - (n - 1.) * (n + 1.), 0.);
    let p = parse("1 w 0 0 m 1073741824 1073741823 l 2147483649 2147483647 l S");
    let mut s = StrokeSession::new(Default::default());
    let error = expand(
        &p,
        &mut s,
        map([1. / 2048., 0., 0., 1. / 2048., 0., 0.]),
        Default::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        StrokeErrorKind::Unsupported(_) | StrokeErrorKind::Arithmetic(_)
    ));
    assert!(s.usage().join_attempts > 0);
}
#[test]
fn structural_coordinate_singular_and_subdivision_bounds_do_not_follow_raised_work_caps() {
    let line = parse("1 w 0 0 m 4 0 l S");
    for m in [
        map([1., 0., 0., 0., 0., 0.]),
        map([1., 0., 0., 1., 2_097_153., 0.]),
    ] {
        assert!(
            expand(
                &line,
                &mut StrokeSession::new(Default::default()),
                m,
                Default::default()
            )
            .is_err()
        );
    }
    let curve = parse("1 w 72 60 m 200 200 400 200 540 60 c S");
    let limits = StrokeLimits {
        max_subdivision_depth: 0,
        ..StrokeLimits::default()
    };
    assert!(
        expand(
            &curve,
            &mut StrokeSession::new(Default::default()),
            identity(),
            limits
        )
        .is_err()
    );
    let round = parse("2 w 1 J 0 0 m 4 0 l S");
    let limits = StrokeLimits {
        max_arc_depth: 0,
        ..StrokeLimits::default()
    };
    assert!(
        expand(
            &round,
            &mut StrokeSession::new(Default::default()),
            identity(),
            limits
        )
        .is_err()
    );
}
