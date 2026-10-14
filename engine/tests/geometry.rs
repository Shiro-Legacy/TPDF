// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::geometry::{
    Affine, DeviceSpace, GeometryError as E, PageSpace, Point, Rect, UserSpace,
};
fn point(x: f64, y: f64) -> Point<UserSpace> {
    Point::new(x, y).unwrap()
}
fn close(actual: [f64; 2], expected: [f64; 2]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() <= 1e-7 * (1.0 + b.abs()), "{a} != {b}");
    }
}
#[test]
fn composition_order_is_point_travel_order_not_ambiguous_multiplication() {
    let translate = Affine::<UserSpace, UserSpace>::translation(10., 20.).unwrap();
    let scale = Affine::scale(2., -3.).unwrap();
    close(
        translate
            .then(scale)
            .unwrap()
            .transform_point(point(2., 3.))
            .unwrap()
            .coordinates(),
        [24., -69.],
    );
    close(
        scale
            .then(translate)
            .unwrap()
            .transform_point(point(2., 3.))
            .unwrap()
            .coordinates(),
        [14., 11.],
    );
    let page = Affine::<UserSpace, PageSpace>::from_pdf([0., 1., -1., 0., 100., 0.]).unwrap();
    let device = Affine::<PageSpace, DeviceSpace>::from_pdf([2., 0., 0., -2., 0., 200.]).unwrap();
    close(
        page.then(device)
            .unwrap()
            .transform_point(point(10., 20.))
            .unwrap()
            .coordinates(),
        [160., 180.],
    );
}
#[test]
fn inverse_and_composition_properties_for_deterministic_well_conditioned_maps() {
    let mut state = 123456789_u64;
    let mut random = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((state >> 32) as f64 / u32::MAX as f64) * 20. - 10.
    };
    for _ in 0..10_000 {
        let (a, b, c, d, e, f) = (random(), random(), random(), random(), random(), random());
        if (a * d - b * c).abs() < 0.01 {
            continue;
        }
        let matrix = Affine::<UserSpace, DeviceSpace>::from_pdf([a, b, c, d, e, f]).unwrap();
        let original = point(random(), random());
        let back = matrix
            .inverse()
            .unwrap()
            .transform_point(matrix.transform_point(original).unwrap())
            .unwrap();
        close(back.coordinates(), original.coordinates());
        close(
            matrix
                .then(matrix.inverse().unwrap())
                .unwrap()
                .transform_point(original)
                .unwrap()
                .coordinates(),
            original.coordinates(),
        );
    }
}
#[test]
fn finite_inputs_and_overflow_singularities_are_explicit() {
    for n in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(Point::<UserSpace>::new(n, 0.), Err(E::Nonfinite));
        assert_eq!(
            Rect::<UserSpace>::from_corners(0., 0., n, 1.),
            Err(E::Nonfinite)
        );
        assert_eq!(
            Affine::<UserSpace, UserSpace>::from_pdf([1., 0., 0., 1., n, 0.]),
            Err(E::Nonfinite)
        );
    }
    assert_eq!(
        Affine::<UserSpace, UserSpace>::from_pdf([1., 2., 2., 4., 1., 2.])
            .unwrap()
            .inverse(),
        Err(E::Singular)
    );
    let giant = Affine::<UserSpace, UserSpace>::scale(f64::MAX, 1.).unwrap();
    assert_eq!(giant.transform_point(point(2., 0.)), Err(E::Nonfinite));
    assert_eq!(giant.then(giant), Err(E::Nonfinite));
    assert_eq!(
        Rect::<UserSpace>::from_corners(-f64::MAX, 0., f64::MAX, 1.),
        Err(E::Nonfinite)
    );
    assert_eq!(
        Affine::<UserSpace, UserSpace>::scale(f64::from_bits(1), 1.)
            .unwrap()
            .inverse(),
        Err(E::Singular)
    );
}
#[test]
fn inverse_avoids_naive_determinant_overflow_and_underflow() {
    for scale in [1e200, 1e-200] {
        let matrix = Affine::<UserSpace, UserSpace>::scale(scale, scale).unwrap();
        close(
            matrix
                .then(matrix.inverse().unwrap())
                .unwrap()
                .transform_point(point(2., -3.))
                .unwrap()
                .coordinates(),
            [2., -3.],
        );
    }
}
#[test]
fn rectangle_transform_contains_all_corners_with_rotation_and_shear() {
    let rect = Rect::<UserSpace>::from_corners(3., 5., -1., 2.).unwrap();
    assert_eq!(rect.bounds(), [-1., 2., 3., 5.]);
    let shear = Affine::<UserSpace, PageSpace>::from_pdf([1., 0., 2., 1., 0., 0.]).unwrap();
    assert_eq!(
        shear.transform_bounds(rect).unwrap().bounds(),
        [3., 2., 13., 5.]
    );
    let quarter = Affine::<UserSpace, PageSpace>::from_pdf([0., 1., -1., 0., 0., 0.]).unwrap();
    assert_eq!(
        quarter.transform_bounds(rect).unwrap().bounds(),
        [-5., -1., -2., 3.]
    );
}
#[test]
fn intersection_and_empty_raster_bounds_do_not_invent_area() {
    let a = Rect::<DeviceSpace>::from_corners(-0.1, 0.5, 10.2, 20.).unwrap();
    assert_eq!(a.outward_raster_bounds().unwrap().bounds(), [-1, 0, 11, 20]);
    let b = Rect::from_corners(10.2, 0., 15., 10.).unwrap();
    assert!(a.intersect(b).is_none());
    let c = Rect::from_corners(5., 5., 15., 25.).unwrap();
    assert_eq!(a.intersect(c).unwrap().bounds(), [5., 5., 10.2, 20.]);
    let empty = Rect::<DeviceSpace>::from_corners(0.25, 0.25, 0.25, 10.25).unwrap();
    assert!(empty.is_empty());
    assert_eq!(empty.outward_raster_bounds().unwrap().width(), 0);
    let beyond = Rect::<DeviceSpace>::from_corners(0., 0., f64::from(i32::MAX) + 0.01, 1.).unwrap();
    assert_eq!(beyond.outward_raster_bounds(), Err(E::IntegerBounds));
    let full = Rect::<DeviceSpace>::from_corners(
        f64::from(i32::MIN),
        f64::from(i32::MIN),
        f64::from(i32::MAX),
        f64::from(i32::MAX),
    )
    .unwrap()
    .outward_raster_bounds()
    .unwrap();
    assert_eq!(full.width(), u64::from(u32::MAX));
    assert_eq!(full.height(), u64::from(u32::MAX));
}
