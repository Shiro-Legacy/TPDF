// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::{
    content::ContentLimits,
    geometry::{Affine, DeviceSpace, UserSpace},
    path::*,
    stroke::*,
};
fn path(s: &str) -> PathProgram {
    interpret_path_content(
        s.as_bytes(),
        PathInitialState::default(),
        ContentLimits::default(),
        PathLimits::default(),
    )
    .unwrap()
}
fn expand(
    p: &PathProgram,
    session: &mut StrokeSession,
    limits: StrokeLimits,
) -> Result<StrokeOutline, StrokeError> {
    let PathEvent::Stroke { ref paint, style } = p.events()[0] else {
        panic!("stroke")
    };
    session.expand(
        p.paths()[paint.path().index()].commands(),
        style,
        style.dash().map(|d| &p.dashes()[d.index()]),
        Affine::<UserSpace, DeviceSpace>::from_pdf([1., 0., 0., 1., 0., 0.]).unwrap(),
        limits,
    )
}
fn inside(out: &StrokeOutline, x: f64, y: f64) -> bool {
    let mut winding = 0;
    let mut first = None;
    let mut prev = None;
    let mut edge = |a: [f64; 2], b: [f64; 2]| {
        if (a[1] <= y && b[1] > y) || (b[1] <= y && a[1] > y) {
            let at = a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if at > x {
                winding += if b[1] > a[1] { 1 } else { -1 };
            }
        }
    };
    for c in out.commands() {
        match *c {
            PathCommand::MoveTo(p) => {
                first = Some(p.coordinates());
                prev = first;
            }
            PathCommand::LineTo(p) => {
                let q = p.coordinates();
                edge(prev.unwrap(), q);
                prev = Some(q);
            }
            PathCommand::ClosePath => edge(prev.unwrap(), first.unwrap()),
            _ => panic!("nonlinear stroke output"),
        }
    }
    winding != 0
}
#[test]
fn closed_stroke_components_union_without_filling_the_interior() {
    let p = path("2 w 1 1 8 6 re S");
    let out = expand(
        &p,
        &mut StrokeSession::new(Default::default()),
        Default::default(),
    )
    .unwrap();
    assert!(inside(&out, 1., 3.));
    assert!(inside(&out, 5., 1.));
    assert!(!inside(&out, 5., 4.));
    assert!(!inside(&out, 11., 4.));
}
#[test]
fn round_and_square_caps_preserve_positive_width_geometry() {
    for cap in [0, 1, 2] {
        let p = path(&format!("2 w {cap} J 2 2 m 8 2 l S"));
        let out = expand(
            &p,
            &mut StrokeSession::new(Default::default()),
            Default::default(),
        )
        .unwrap();
        assert!(inside(&out, 5., 2.));
        assert_eq!(inside(&out, 1.5, 2.25), cap != 0);
        assert_eq!(inside(&out, 1.1, 1.1), cap == 2);
    }
}
#[test]
fn stress_cubic_has_bounded_owned_output_and_shared_sections() {
    let out = {
        let p = path("1 w 72 60 m 200 200 400 200 540 60 c S");
        let mut session = StrokeSession::new(Default::default());
        let out = expand(&p, &mut session, Default::default()).unwrap();
        assert!(session.usage().curve_nodes > 1);
        assert!(session.usage().curve_nodes < 2048);
        assert!(session.usage().curve_leaves < 1024);
        assert!(inside(&out, 310.5, 165.));
        assert!(!inside(&out, 310.5, 168.));
        out
    };
    std::thread::spawn(move || assert!(out.commands().len() > 100))
        .join()
        .unwrap();
}
#[test]
fn curve_node_failure_retains_work_and_exact_page_quota_cannot_reset() {
    let p = path("1 w 72 60 m 200 200 400 200 540 60 c S");
    let cap = StrokeLimits {
        max_curve_nodes: 1,
        ..Default::default()
    };
    let mut s = StrokeSession::new(Default::default());
    assert!(expand(&p, &mut s, cap).is_err());
    let usage = s.usage();
    assert_eq!(usage.curve_nodes, 1);
    assert!(usage.work > 0);
    assert!(expand(&p, &mut s, cap).is_err());
    assert_eq!(s.usage().curve_nodes, 2);
    assert_eq!(s.usage().work, usage.work * 2);
    let mut probe = StrokeSession::new(Default::default());
    expand(&p, &mut probe, Default::default()).unwrap();
    let exact = probe.usage().output_commands;
    let mut s = StrokeSession::new(StrokeLimits {
        max_output_commands: exact,
        ..Default::default()
    });
    let good = expand(&p, &mut s, Default::default()).unwrap();
    assert!(expand(&p, &mut s, Default::default()).is_err());
    assert_eq!(s.usage().output_commands, exact);
    assert!(!good.commands().is_empty());
}
#[test]
fn invalid_dash_binding_and_subnormal_radius_fail_closed() {
    let p = path("[1 2] 0 d 0 0 m 5 0 l S");
    let PathEvent::Stroke { style, .. } = p.events()[0] else {
        panic!()
    };
    assert!(matches!(
        validate_style(style, None).unwrap_err().kind,
        StrokeErrorKind::Invariant(_)
    ));
    assert!(validate_style(style, Some(&p.dashes()[0])).is_ok());
    let dotted = path("1 J [0 2] 0 d 0 0 m 5 0 l S");
    let PathEvent::Stroke { style, .. } = dotted.events()[0] else {
        panic!()
    };
    assert!(matches!(
        validate_style(style, Some(&dotted.dashes()[0]))
            .unwrap_err()
            .kind,
        StrokeErrorKind::Unsupported(_)
    ));
    let tiny = path(&format!("{} w 0 0 m 5 0 l S", f64::from_bits(1)));
    assert!(
        expand(
            &tiny,
            &mut StrokeSession::new(Default::default()),
            Default::default()
        )
        .is_err()
    );
}
#[test]
fn checked_public_ledger_helpers_are_transactional() {
    let mut u = StrokeUsage {
        work: u64::MAX,
        ..Default::default()
    };
    let old = u;
    assert!(
        u.accumulate(StrokeUsage {
            attempts: 1,
            work: 1,
            ..Default::default()
        })
        .is_err()
    );
    assert_eq!(u, old);
    assert!(StrokeLimits::default().remaining(u).is_err());
    let a = StrokeLimits {
        max_curve_nodes: 8,
        max_arc_depth: 3,
        ..Default::default()
    };
    let b = StrokeLimits {
        max_curve_nodes: 9,
        max_arc_depth: 8,
        ..Default::default()
    };
    let c = a.intersect(b);
    assert_eq!(c.max_curve_nodes, 8);
    assert_eq!(c.max_arc_depth, 3);
}
