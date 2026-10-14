// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent finite geometry and exact singularity regressions.
use tandem_pdf_engine::geometry::{
    Affine, DeviceSpace, GeometryError, PageSpace, Point, Rect, UserSpace,
};

#[test]
fn noncommuting_cross_space_composition_matches_independent_integer_coordinates() {
    let first = Affine::<UserSpace, PageSpace>::from_pdf([2., 3., 5., 7., 11., 13.]).unwrap();
    let second =
        Affine::<PageSpace, DeviceSpace>::from_pdf([17., 19., 23., 29., 31., 37.]).unwrap();
    let composed = first.then(second).unwrap();
    assert_eq!(
        composed.coefficients(),
        [103., 125., 246., 298., 517., 623.]
    );
    assert_eq!(
        composed
            .transform_point(Point::new(-2., 4.).unwrap())
            .unwrap()
            .coordinates(),
        [1295., 1565.]
    );
    let inverse = first.inverse().unwrap();
    for (actual, expected) in inverse
        .coefficients()
        .into_iter()
        .zip([-7., 3., 5., -2., 12., -7.])
    {
        assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
    }
}

#[test]
fn exact_rank_one_integer_matrices_cannot_gain_an_inverse_from_normalization_rounding() {
    // The original scaled determinant evaluated to3.469446951953614e-18,
    // despite exact column dependence, producing a huge bogus inverse.
    let regression = Affine::<UserSpace, PageSpace>::from_pdf([1., 3., 15., 45., 0., 0.]).unwrap();
    assert_eq!(regression.inverse(), Err(GeometryError::Singular));
    for a in -5..=5 {
        for b in -5..=5 {
            for multiplier in -17..=17 {
                let coefficients = [
                    f64::from(a),
                    f64::from(b),
                    f64::from(a * multiplier),
                    f64::from(b * multiplier),
                    11.,
                    -7.,
                ];
                assert_eq!(
                    Affine::<UserSpace, PageSpace>::from_pdf(coefficients)
                        .unwrap()
                        .inverse(),
                    Err(GeometryError::Singular),
                    "{coefficients:?}"
                );
            }
        }
    }
}

#[test]
fn representable_power_of_two_scale_inverses_remain_finite_across_exponent_range() {
    for exponent in (-1023..=1023).step_by(31) {
        let scale = 2_f64.powi(exponent);
        let transform =
            Affine::<UserSpace, DeviceSpace>::from_pdf([scale, 0., 0., -scale, 0., 0.]).unwrap();
        let inverse = transform.inverse().unwrap();
        assert!(inverse.coefficients().iter().all(|value| value.is_finite()));
        assert!(inverse.coefficients()[0] > 0.);
        let roundtrip = inverse
            .transform_point(
                transform
                    .transform_point(Point::new(0.25, -0.5).unwrap())
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(roundtrip.coordinates(), [0.25, -0.5]);
    }
}

#[test]
fn transformed_rectangle_bounds_include_all_four_extreme_corners() {
    let rectangle = Rect::<UserSpace>::from_corners(4., -3., -2., 5.).unwrap();
    let transform =
        Affine::<UserSpace, DeviceSpace>::from_pdf([-2., 3., 5., -7., 11., 13.]).unwrap();
    // x=-2x+5y+11 has min(-12) at(4,-3), max40 at(-2,5).
    // y=3x-7y+13 has min(-28) at(-2,5), max46 at(4,-3).
    assert_eq!(
        transform.transform_bounds(rectangle).unwrap().bounds(),
        [-12., -28., 40., 46.]
    );
}

#[test]
fn raster_boundaries_never_saturate_and_zero_area_remains_zero_area() {
    let min = f64::from(i32::MIN);
    let max = f64::from(i32::MAX);
    for (a, b, accepted) in [
        (min, max, true),
        (min - 0.25, max, false),
        (min, max + 0.25, false),
    ] {
        let rectangle = Rect::<DeviceSpace>::from_corners(a, 0., b, 1.).unwrap();
        assert_eq!(rectangle.outward_raster_bounds().is_ok(), accepted);
    }
    let point = Rect::<DeviceSpace>::from_corners(-0.25, 0.75, -0.25, 0.75)
        .unwrap()
        .outward_raster_bounds()
        .unwrap();
    assert_eq!(point.bounds(), [-1, 0, -1, 0]);
    assert_eq!((point.width(), point.height()), (0, 0));
}

#[test]
fn transformed_line_bounds_can_enclose_area_under_explicit_corner_bound_policy() {
    let line = Rect::<UserSpace>::from_corners(0., 0., 0., 1.).unwrap();
    assert!(line.is_empty());
    let shear = Affine::<UserSpace, DeviceSpace>::from_pdf([1., 0., 1., 1., 0., 0.]).unwrap();
    let bounds = shear.transform_bounds(line).unwrap();
    assert_eq!(bounds.bounds(), [0., 0., 1., 1.]);
    assert!(!bounds.is_empty()); // A bounding box, not transformed region semantics.
}
