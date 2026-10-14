// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::geometry::Affine;
use tandem_pdf_engine::image_sampling::{
    ImageSamplingError as E, ImageSamplingLimits as Limits, ImageSamplingSession as Session,
};
fn session() -> Session {
    Session::new(Limits::default())
}
fn selected(
    s: &mut Session,
    p: &tandem_pdf_engine::image_sampling::ImagePlacement,
    x: u32,
    y: u32,
) -> Option<(u32, u32)> {
    s.sample(p, x, y).unwrap().map(|t| (t.row(), t.column()))
}
#[test]
fn identity_is_top_down_and_outside_is_not_clamped() {
    let mut s = session();
    let p = s
        .prepare(Affine::from_pdf([1., 0., 0., 1., 0., 0.]).unwrap(), 2, 2)
        .unwrap();
    assert_eq!(selected(&mut s, &p, 0, 0), Some((1, 0)));
    assert_eq!(selected(&mut s, &p, 3, 0), Some((1, 1)));
    assert_eq!(selected(&mut s, &p, 0, 3), Some((0, 0)));
    assert_eq!(selected(&mut s, &p, 3, 3), Some((0, 1)));
    assert_eq!(selected(&mut s, &p, 4, 0), None);
    assert_eq!(selected(&mut s, &p, 0, 4), None);
}
#[test]
fn exact_internal_row_boundary_floors_before_y_reversal() {
    let mut s = session();
    let p = s
        .prepare(
            Affine::from_pdf([1., 0., 0., 1., 0.125, 0.125]).unwrap(),
            4,
            4,
        )
        .unwrap();
    assert_eq!(selected(&mut s, &p, 2, 2), Some((2, 2)));
    assert_eq!(selected(&mut s, &p, 0, 0), Some((3, 0)));
    assert_eq!(selected(&mut s, &p, 4, 0), None);
    assert_eq!(selected(&mut s, &p, 0, 4), None);
}
#[test]
fn reflection_keeps_device_edge_ownership_and_clamps_only_exact_endpoint() {
    let mut s = session();
    let p = s
        .prepare(
            Affine::from_pdf([-1., 0., 0., 1., 1.125, 0.125]).unwrap(),
            2,
            2,
        )
        .unwrap();
    assert_eq!(selected(&mut s, &p, 0, 0), Some((1, 1)));
    assert_eq!(selected(&mut s, &p, 4, 0), None);
    assert_eq!(selected(&mut s, &p, 2, 2), Some((1, 1)));
}
#[test]
fn rotation_and_shear_have_true_quadrilateral_footprints() {
    let mut s = session();
    let rotation = s
        .prepare(Affine::from_pdf([0., 1., -1., 0., 1., 0.]).unwrap(), 2, 2)
        .unwrap();
    assert_eq!(selected(&mut s, &rotation, 0, 0), Some((0, 0)));
    assert_eq!(selected(&mut s, &rotation, 3, 3), Some((1, 1)));
    let shear = s
        .prepare(Affine::from_pdf([1., 0., 1., 1., 0., 0.]).unwrap(), 2, 2)
        .unwrap();
    assert_eq!(selected(&mut s, &shear, 0, 3), None);
    assert_eq!(selected(&mut s, &shear, 4, 3), Some((0, 0)));
}
#[test]
fn negative_determinant_does_not_reverse_stored_row_order() {
    let mut s = session();
    let p = s
        .prepare(Affine::from_pdf([1., 0., 0., -1., 0., 1.]).unwrap(), 1, 4)
        .unwrap();
    for y in 0..4 {
        assert_eq!(selected(&mut s, &p, 0, y), Some((y, 0)));
    }
}
#[test]
fn cancellation_does_not_turn_nonzero_determinant_into_singularity() {
    let mut s = session();
    let delta = 2f64.powi(-52);
    let p = s
        .prepare(
            Affine::from_pdf([1., 1. + delta, 1. - delta, 1., 0.125, 0.125]).unwrap(),
            7,
            11,
        )
        .unwrap();
    assert_eq!(selected(&mut s, &p, 0, 0), None); // This origin is not a top/left vertex.
    let tiny = s
        .prepare(
            Affine::from_pdf([2f64.powi(-80), 0., 0., 2f64.powi(-80), 0.125, 0.125]).unwrap(),
            65536,
            65536,
        )
        .unwrap();
    assert_eq!(selected(&mut s, &tiny, 0, 0), Some((65535, 0)));
}
#[test]
fn singular_matrices_refuse_exactly() {
    let mut s = session();
    for a in [
        [0.; 6],
        [1., 3., 15., 45., 0., 0.],
        [2., -3., -4., 6., 1., 1.],
    ] {
        assert!(matches!(
            s.prepare(Affine::from_pdf(a).unwrap(), 2, 2),
            Err(E::Singular)
        ));
    }
    assert_eq!(s.usage().preparations, 3);
    assert_eq!(s.usage().work, 3072);
}
#[test]
fn dimension_and_lattice_admission_is_explicit() {
    let mut s = session();
    let identity = Affine::from_pdf([1., 0., 0., 1., 0., 0.]).unwrap();
    for (w, h) in [(0, 1), (1, 0), (65537, 1), (1, 65537)] {
        assert!(matches!(s.prepare(identity, w, h), Err(E::Dimensions)));
    }
    let p = s.prepare(identity, 65536, 65536).unwrap();
    assert_eq!((p.width(), p.height()), (65536, 65536));
    for tiny in [2f64.powi(-81), f64::from_bits(1)] {
        assert!(matches!(
            s.prepare(Affine::from_pdf([tiny, 0., 0., 1., 0., 0.]).unwrap(), 1, 1),
            Err(E::NumericDomain)
        ));
    }
    assert!(matches!(
        s.prepare(
            Affine::from_pdf([1., 0., 0., 1., 2097152., 0.]).unwrap(),
            1,
            1
        ),
        Err(E::NumericDomain)
    ));
    assert!(
        s.prepare(
            Affine::from_pdf([1., 0., 0., 1., 2097151., 0.]).unwrap(),
            1,
            1
        )
        .is_ok()
    );
}
#[test]
fn sample_index_cap_retains_failure_work() {
    let mut s = session();
    let p = s
        .prepare(
            Affine::from_pdf([1., 0., 0., 1., 2097151., 0.]).unwrap(),
            1,
            1,
        )
        .unwrap();
    assert_eq!(selected(&mut s, &p, (1 << 23) - 1, 0), Some((0, 0)));
    assert_eq!(s.sample(&p, 1 << 23, 0), Err(E::NumericDomain));
    assert_eq!(s.sample(&p, 0, 1 << 23), Err(E::NumericDomain));
    assert_eq!(s.usage().samples, 3);
    assert_eq!(s.usage().work, 1024 + 3 * 16384);
}
#[test]
fn owned_placement_is_reusable_but_samples_are_always_charged() {
    let p = {
        let mut s = session();
        s.prepare(Affine::from_pdf([1., 0., 0., 1., 0., 0.]).unwrap(), 2, 2)
            .unwrap()
    };
    let mut s = Session::new(Limits {
        max_preparations: 0,
        max_samples: 2,
        max_work: 32768,
    });
    assert_eq!(selected(&mut s, &p, 0, 0), Some((1, 0)));
    assert_eq!(selected(&mut s, &p, 0, 0), Some((1, 0)));
    assert_eq!(s.sample(&p, 0, 0), Err(E::Limit("samples")));
    assert_eq!(s.usage().work, 32768);
}
#[test]
fn exact_and_one_less_work_limits_keep_attempts() {
    let matrix = Affine::from_pdf([1., 0., 0., 1., 0., 0.]).unwrap();
    let mut short = Session::new(Limits {
        max_preparations: 2,
        max_samples: 1,
        max_work: 1023,
    });
    assert!(matches!(short.prepare(matrix, 1, 1), Err(E::Limit("work"))));
    assert!(matches!(short.prepare(matrix, 1, 1), Err(E::Limit("work"))));
    assert_eq!(short.usage().preparations, 2);
    assert_eq!(short.usage().work, 0);
    let mut exact = Session::new(Limits {
        max_preparations: 1,
        max_samples: 2,
        max_work: 17408,
    });
    let p = exact.prepare(matrix, 1, 1).unwrap();
    assert_eq!(selected(&mut exact, &p, 0, 0), Some((0, 0)));
    assert_eq!(exact.sample(&p, 0, 0), Err(E::Limit("work")));
    assert_eq!(exact.usage().samples, 2);
    assert_eq!(exact.usage().work, 17408);
    let mut below = Session::new(Limits {
        max_preparations: 1,
        max_samples: 1,
        max_work: 17407,
    });
    let p = below.prepare(matrix, 1, 1).unwrap();
    assert_eq!(below.sample(&p, 0, 0), Err(E::Limit("work")));
    assert_eq!(below.usage().work, 1024);
}
#[test]
fn exact_and_one_less_attempt_limits_never_reset() {
    let matrix = Affine::from_pdf([1., 0., 0., 1., 0., 0.]).unwrap();
    let mut s = Session::new(Limits {
        max_preparations: 1,
        max_samples: 0,
        max_work: u64::MAX,
    });
    let p = s.prepare(matrix, 1, 1).unwrap();
    assert!(matches!(
        s.prepare(matrix, 1, 1),
        Err(E::Limit("preparations"))
    ));
    assert_eq!(s.sample(&p, 0, 0), Err(E::Limit("samples")));
    assert_eq!(s.usage().work, 1024);
}
