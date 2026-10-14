// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent per-sample ray model: deliberately no scanline intersections/sort/spans.
use tandem_pdf_engine::{path::FillRule, scan_raster::*};
fn edges(points: &[(i32, i32)]) -> Vec<ScanEdge> {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(&(x, y), &(a, b))| ScanEdge {
            start: ScanPoint { x, y },
            end: ScanPoint { x: a, y: b },
        })
        .collect()
}
fn model(edges: &[ScanEdge], rule: FillRule, width: u32, height: u32) -> Vec<bool> {
    (0..height)
        .flat_map(|y| {
            (0..width).map(move |x| {
                let sx = 32 + 64 * i128::from(x);
                let sy = 32 + 64 * i128::from(y);
                let mut winding = 0_i64;
                for edge in edges {
                    let x0 = i128::from(edge.start.x);
                    let y0 = i128::from(edge.start.y);
                    let x1 = i128::from(edge.end.x);
                    let y1 = i128::from(edge.end.y);
                    let cross = (x1 - x0) * (sy - y0) - (sx - x0) * (y1 - y0);
                    if y0 <= sy && sy < y1 && cross > 0 {
                        winding += 1;
                    } else if y1 <= sy && sy < y0 && cross < 0 {
                        winding -= 1;
                    }
                }
                match rule {
                    FillRule::NonZero => winding != 0,
                    FillRule::EvenOdd => winding % 2 != 0,
                }
            })
        })
        .collect()
}
fn rendered(
    session: &mut ScanSession,
    edges: &[ScanEdge],
    rule: FillRule,
    width: u32,
    height: u32,
) -> Result<Vec<bool>, ScanError> {
    let mut pixels = vec![false; (width * height) as usize];
    let mut previous = None;
    session.scan(edges, rule, width, height, |span| {
        assert!(span.x0 < span.x1 && span.x1 <= width && span.y < height);
        if let Some((y, x)) = previous {
            assert!(span.y > y || span.y == y && span.x0 >= x);
        }
        previous = Some((span.y, span.x1));
        for x in span.x0..span.x1 {
            let at = (span.y * width + x) as usize;
            assert!(!pixels[at], "overlapping callback span");
            pixels[at] = true;
        }
    })?;
    Ok(pixels)
}
fn check(edges: &[ScanEdge], width: u32, height: u32) {
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let got = rendered(
            &mut ScanSession::new(Default::default()),
            edges,
            rule,
            width,
            height,
        )
        .unwrap();
        assert_eq!(
            got,
            model(edges, rule, width, height),
            "edges={edges:?},rule={rule:?}"
        );
    }
}
#[test]
fn offcanvas_crossings_are_required_to_establish_visible_winding() {
    check(
        &edges(&[(-1000, -1000), (1000, -1000), (1000, 1000), (-1000, 1000)]),
        8,
        8,
    );
    check(
        &edges(&[(-1000, 0), (160, 0), (160, 1000), (-1000, 1000)]),
        8,
        8,
    );
    check(
        &edges(&[(160, 0), (1000, 0), (1000, 1000), (160, 1000)]),
        8,
        8,
    );
}
#[test]
fn coincident_edges_holes_reversed_contours_and_exact_center_ties() {
    let outer = edges(&[(32, 32), (480, 32), (480, 480), (32, 480)]);
    let inner = edges(&[(96, 96), (96, 416), (416, 416), (416, 96)]);
    let mut hole = outer.clone();
    hole.extend(inner);
    check(&hole, 8, 8);
    let mut duplicate = outer.clone();
    duplicate.extend(outer.clone());
    check(&duplicate, 8, 8);
    let mut opposite = outer;
    opposite.extend(edges(&[(32, 480), (480, 480), (480, 32), (32, 32)]));
    check(&opposite, 8, 8);
    check(&edges(&[(32, 32), (480, 480), (32, 480), (480, 32)]), 8, 8);
}
#[test]
fn horizontal_zero_length_and_collinear_backtracking_edges_do_not_invent_area() {
    check(&edges(&[(0, 32), (400, 32), (0, 32), (0, 32)]), 8, 8);
    check(
        &edges(&[(32, 32), (160, 32), (96, 32), (160, 256), (32, 256)]),
        8,
        8,
    );
    check(&[], 8, 8);
}
#[test]
fn extreme_admitted_coordinates_and_tiny_crossing_slopes_match_i128_model() {
    let bound = 1 << 29;
    check(
        &edges(&[
            (-bound, -bound),
            (bound, -bound),
            (bound, bound),
            (-bound, bound),
        ]),
        8,
        8,
    );
    check(
        &edges(&[(-bound, 31), (bound, 33), (bound, 480), (-bound, 480)]),
        8,
        8,
    );
    check(
        &edges(&[
            (-bound, -bound),
            (bound, bound),
            (bound, -bound),
            (-bound, bound),
        ]),
        8,
        8,
    );
}
#[test]
fn deterministic_polygons_reflections_and_selfcrossings_match_independent_samples() {
    let mut seed = 0x32f7d92a_u32;
    for case in 0..240 {
        let count = 3 + case % 9;
        let mut points = Vec::new();
        for _ in 0..count {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let x = ((seed >> 10) % 1300) as i32 - 400;
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let y = ((seed >> 10) % 1300) as i32 - 400;
            points.push((x, y));
        }
        check(&edges(&points), 8, 8);
        points.reverse();
        check(&edges(&points), 8, 8);
    }
}
#[test]
fn malformed_coordinate_and_dimension_inputs_never_call_callback() {
    let valid = edges(&[(0, 0), (256, 0), (256, 256), (0, 256)]);
    for coordinate in [i32::MIN, -(1 << 29) - 1, (1 << 29) + 1, i32::MAX] {
        let mut bad = valid.clone();
        bad.push(ScanEdge {
            start: ScanPoint {
                x: coordinate,
                y: 0,
            },
            end: ScanPoint { x: 0, y: 0 },
        });
        let mut calls = 0;
        assert!(
            ScanSession::new(Default::default())
                .scan(&bad, FillRule::NonZero, 4, 4, |_| calls += 1)
                .is_err()
        );
        assert_eq!(calls, 0);
    }
    for (width, height) in [(0, 4), (4, 0), ((1 << 23) + 1, 4), (4, (1 << 23) + 1)] {
        let mut calls = 0;
        assert!(
            ScanSession::new(Default::default())
                .scan(&valid, FillRule::NonZero, width, height, |_| calls += 1)
                .is_err()
        );
        assert_eq!(calls, 0);
    }
}
#[test]
fn exact_cumulative_budgets_and_one_less_boundaries() {
    let shape = edges(&[(0, 0), (400, 64), (256, 448), (32, 256)]);
    let mut baseline = ScanSession::new(Default::default());
    let expected = rendered(&mut baseline, &shape, FillRule::NonZero, 8, 8).unwrap();
    let used = baseline.usage();
    let exact = ScanLimits {
        max_edges: used.edges,
        max_rows: used.rows,
        max_work: used.work,
        max_allocated_bytes: used.allocated_bytes,
        max_spans: used.spans,
        max_samples: used.samples,
    };
    let mut s = ScanSession::new(exact);
    assert_eq!(
        rendered(&mut s, &shape, FillRule::NonZero, 8, 8).unwrap(),
        expected
    );
    assert!(rendered(&mut s, &shape, FillRule::NonZero, 8, 8).is_err());
    for which in 0..6 {
        let mut limits = exact;
        match which {
            0 => limits.max_edges -= 1,
            1 => limits.max_rows -= 1,
            2 => limits.max_work -= 1,
            3 => limits.max_allocated_bytes -= 1,
            4 => limits.max_spans -= 1,
            _ => limits.max_samples -= 1,
        };
        let mut s = ScanSession::new(limits);
        assert!(
            rendered(&mut s, &shape, FillRule::NonZero, 8, 8).is_err(),
            "boundary {which}"
        );
        let u = s.usage();
        assert!(
            u.edges <= limits.max_edges
                && u.rows <= limits.max_rows
                && u.work <= limits.max_work
                && u.allocated_bytes <= limits.max_allocated_bytes
                && u.spans <= limits.max_spans
                && u.samples <= limits.max_samples
        );
    }
}
#[test]
fn callback_samples_are_admitted_before_emission_and_retries_stay_bounded() {
    let shape = edges(&[(0, 0), (256, 0), (256, 256), (0, 256)]);
    let limits = ScanLimits {
        max_samples: 0,
        ..Default::default()
    };
    let mut s = ScanSession::new(limits);
    for _ in 0..3 {
        let mut calls = 0;
        assert!(
            s.scan(&shape, FillRule::NonZero, 4, 4, |_| calls += 1)
                .is_err()
        );
        assert_eq!(calls, 0);
        assert_eq!(s.usage().samples, 0);
    }
    assert!(s.usage().edges > 0);
}
#[test]
fn unbalanced_open_edge_soup_refuses_instead_of_losing_unbounded_winding() {
    let edge = ScanEdge {
        start: ScanPoint { x: 64, y: 0 },
        end: ScanPoint { x: 64, y: 256 },
    };
    let mut session = ScanSession::new(Default::default());
    assert!(
        session
            .scan(&[edge], FillRule::NonZero, 4, 4, |_| {})
            .is_err()
    );
}
