// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent review tests: orthogonal inverse by integer dot products, not determinants.
use tandem_pdf_engine::{
    geometry::Affine,
    image_sampling::{ImageSamplingLimits, ImageSamplingSession},
};

#[test]
fn all_square_symmetries_preserve_device_edges_and_top_down_rational_cells() {
    let mut session = ImageSamplingSession::new(ImageSamplingLimits::default());
    for (a, b, c, d) in [
        (1, 0, 0, 1),
        (-1, 0, 0, 1),
        (1, 0, 0, -1),
        (-1, 0, 0, -1),
        (0, 1, 1, 0),
        (0, -1, 1, 0),
        (0, 1, -1, 0),
        (0, -1, -1, 0),
    ] {
        let tx = -(a + c).min(0);
        let ty = -(b + d).min(0);
        for (width, height) in [(1, 1), (3, 7), (4, 4), (65535, 65536)] {
            let placement = session
                .prepare(
                    Affine::from_pdf([
                        f64::from(a),
                        f64::from(b),
                        f64::from(c),
                        f64::from(d),
                        f64::from(tx) + 0.125,
                        f64::from(ty) + 0.125,
                    ])
                    .unwrap(),
                    width,
                    height,
                )
                .unwrap();
            for x in 0..=5 {
                for y in 0..=5 {
                    let actual = session
                        .sample(&placement, x, y)
                        .unwrap()
                        .map(|t| (t.row(), t.column()));
                    let expected = if x < 4 && y < 4 {
                        // Orthonormal columns: inverse is transpose; all coordinates are quarters.
                        let dx = i64::from(x) - 4 * i64::from(tx);
                        let dy = i64::from(y) - 4 * i64::from(ty);
                        let u = i64::from(a) * dx + i64::from(b) * dy;
                        let v = i64::from(c) * dx + i64::from(d) * dy;
                        let col = ((i64::from(width) * u) / 4).min(i64::from(width) - 1) as u32;
                        let row =
                            ((i64::from(height) * (4 - v)) / 4).min(i64::from(height) - 1) as u32;
                        Some((row, col))
                    } else {
                        None
                    };
                    assert_eq!(
                        actual,
                        expected,
                        "matrix {:?}, dimensions {:?}, sample {:?}",
                        (a, b, c, d),
                        (width, height),
                        (x, y)
                    );
                }
            }
        }
    }
}

#[test]
fn dyadic_step_on_either_side_of_a_diagonal_changes_coverage_exactly() {
    let mut session = ImageSamplingSession::new(ImageSamplingLimits::default());
    // Lower edge y=x+epsilon; sample (1/8,1/8) is above, on or below it.
    for (epsilon, inside) in [
        (-2f64.powi(-80), true),
        (0., false),
        (2f64.powi(-80), false),
    ] {
        let placement = session
            .prepare(
                Affine::from_pdf([1., 1., 0., 1., 0., epsilon]).unwrap(),
                65536,
                65535,
            )
            .unwrap();
        let texel = session.sample(&placement, 0, 0).unwrap();
        assert_eq!(texel.is_some(), inside, "epsilon {epsilon}");
        if let Some(t) = texel {
            assert_eq!((t.row(), t.column()), (65534, 8192));
        }
    }
}

#[test]
fn adjacent_sheared_images_own_their_common_edge_once() {
    let mut session = ImageSamplingSession::new(ImageSamplingLimits::default());
    for reflection in [1., -1.] {
        let first = session
            .prepare(
                Affine::from_pdf([reflection, 0., 1., 1., 1.125, 0.125]).unwrap(),
                3,
                5,
            )
            .unwrap();
        let second = session
            .prepare(
                Affine::from_pdf([reflection, 0., 1., 1., 1.125 + reflection, 0.125]).unwrap(),
                3,
                5,
            )
            .unwrap();
        for k in 1..4 {
            let x = (4. + reflection * 4.) as u32 + k;
            let count = [
                session.sample(&first, x, k).unwrap(),
                session.sample(&second, x, k).unwrap(),
            ]
            .iter()
            .filter(|v| v.is_some())
            .count();
            assert_eq!(count, 1, "reflection {reflection}, edge sample {k}");
        }
    }
}

#[test]
fn exact_candidate_bounds_contain_every_admitted_sample_under_shear_and_crop() {
    let mut session = ImageSamplingSession::new(ImageSamplingLimits::default());
    for a in -2..=2 {
        for b in -2..=2 {
            for c in -2..=2 {
                for d in -2..=2 {
                    if a * d == b * c {
                        continue;
                    }
                    for translation in [-2.125, 0.125, 2.125] {
                        let p = session
                            .prepare(
                                Affine::from_pdf([
                                    f64::from(a),
                                    f64::from(b),
                                    f64::from(c),
                                    f64::from(d),
                                    translation,
                                    translation,
                                ])
                                .unwrap(),
                                7,
                                11,
                            )
                            .unwrap();
                        for (w, h) in [(0, 0), (1, 1), (7, 13), (20, 20)] {
                            let [x0, y0, x1, y1] = p.sample_bounds(w, h);
                            assert!(x0 <= x1 && x1 <= w && y0 <= y1 && y1 <= h);
                            for y in 0..h {
                                for x in 0..w {
                                    if session.sample(&p, x, y).unwrap().is_some() {
                                        assert!(
                                            x >= x0 && x < x1 && y >= y0 && y < y1,
                                            "lost sample ({x},{y}) in {w}x{h}, matrix {:?}, bounds {:?}",
                                            (a, b, c, d, translation),
                                            [x0, y0, x1, y1]
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn candidate_bounds_remain_safe_at_coordinate_and_lattice_limits() {
    let mut session = ImageSamplingSession::new(ImageSamplingLimits::default());
    let p = session
        .prepare(
            Affine::from_pdf([1., 0., 0., 1., 2097151., 2097151.]).unwrap(),
            65536,
            65536,
        )
        .unwrap();
    assert_eq!(
        p.sample_bounds(1 << 23, 1 << 23),
        [(1 << 23) - 4, (1 << 23) - 4, 1 << 23, 1 << 23]
    );
    assert!(
        session
            .sample(&p, (1 << 23) - 1, (1 << 23) - 1)
            .unwrap()
            .is_some()
    );
    let p = session
        .prepare(
            Affine::from_pdf([2f64.powi(-80), 0., 0., 2f64.powi(-80), 0.125, 0.125]).unwrap(),
            1,
            1,
        )
        .unwrap();
    assert_eq!(p.sample_bounds(4, 4), [0, 0, 1, 1]);
    assert!(session.sample(&p, 0, 0).unwrap().is_some());
    let p = session
        .prepare(
            Affine::from_pdf([1., 0., 0., 1., -2097152., -2097152.]).unwrap(),
            1,
            1,
        )
        .unwrap();
    assert_eq!(p.sample_bounds(u32::MAX, u32::MAX), [0; 4]);
}
