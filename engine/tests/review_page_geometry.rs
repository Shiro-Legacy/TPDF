// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent page-placement precision and typed composition review.
use tandem_pdf_engine::geometry::{Point, UserSpace};
use tandem_pdf_engine::object::ObjectId;
use tandem_pdf_engine::page_geometry::PageGeometry;
use tandem_pdf_engine::pages::PageInfo;

fn page(bounds: [f64; 4], rotation: u16, unit: f64) -> PageInfo {
    PageInfo {
        id: ObjectId::new(1, 0).unwrap(),
        media_box: bounds,
        crop_box: bounds,
        rotation,
        user_unit: unit,
        contents: vec![],
    }
}

#[test]
fn cancellation_cannot_return_bounds_inconsistent_with_its_transform() {
    for rotation in [0, 90, 180, 270] {
        let p = page([1e16, 0., 1e16 + 2., 10.], rotation, 0.1);
        // Direct products cancel to 0.25 although (x1-x0)*unit is 0.2.
        // Refuse this loss instead of publishing conflicting placement/bounds.
        assert!(PageGeometry::from_page_info(&p).is_err());
    }
}

#[test]
fn dyadic_boxes_center_and_corners_agree_across_scales_and_rotations() {
    for exponent in [-400, -40, -4, 0, 8, 40, 400] {
        let scale = 2f64.powi(exponent);
        for unit in [0.125, 0.5, 1., 2., 1024.] {
            for rotation in [0, 90, 180, 270] {
                let p = page(
                    [-32. * scale, -16. * scale, 64. * scale, 48. * scale],
                    rotation,
                    unit,
                );
                let g = PageGeometry::from_page_info(&p).unwrap();
                let width = if rotation % 180 == 0 { 96. } else { 64. } * scale * unit;
                let height = if rotation % 180 == 0 { 64. } else { 96. } * scale * unit;
                assert_eq!(g.bounds().bounds(), [0., 0., width, height]);
                assert_eq!(
                    g.user_to_page().transform_bounds(g.crop()).unwrap(),
                    g.bounds()
                );
                let center = Point::<UserSpace>::new(16. * scale, 16. * scale).unwrap();
                assert_eq!(
                    g.user_to_page()
                        .transform_point(center)
                        .unwrap()
                        .coordinates(),
                    [width / 2., height / 2.]
                );
                let pixels = g.page_to_device(144.).unwrap();
                let combined = g.user_to_page().then(pixels).unwrap();
                let mapped = combined.transform_point(center).unwrap();
                assert_eq!(mapped.coordinates(), [width, height]);
                assert_eq!(
                    combined.inverse().unwrap().transform_point(mapped).unwrap(),
                    center
                );
            }
        }
    }
}

#[test]
fn containment_rejects_even_one_representable_step_beyond_media() {
    let p = page([0., 0., 1., 1.], 0, 1.);
    for crop in [
        [-f64::from_bits(1), 0., 1., 1.],
        [0., -f64::from_bits(1), 1., 1.],
        [0., 0., f64::from_bits(1f64.to_bits() + 1), 1.],
        [0., 0., 1., f64::from_bits(1f64.to_bits() + 1)],
    ] {
        let changed = PageInfo {
            crop_box: crop,
            ..p.clone()
        };
        assert!(PageGeometry::from_page_info(&changed).is_err());
    }
    assert_eq!(p.media_box, p.crop_box);
}

#[test]
fn preserving_subunit_geometry_does_not_require_a_native_unit_rectangle() {
    for rotation in [0, 90, 180, 270] {
        let p = page([-0.5, -0.25, -0.375, 0.25], rotation, 0.5);
        let g = PageGeometry::from_page_info(&p).unwrap();
        assert_eq!(
            g.user_to_page().transform_bounds(g.crop()).unwrap(),
            g.bounds()
        );
        let expected = if rotation % 180 == 0 {
            [0., 0., 0.0625, 0.25]
        } else {
            [0., 0., 0.25, 0.0625]
        };
        assert_eq!(g.bounds().bounds(), expected);
    }
}
