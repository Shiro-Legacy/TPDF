use tandem_pdf_engine::geometry::{Point, UserSpace};
use tandem_pdf_engine::object::ObjectId;
use tandem_pdf_engine::page_geometry::PageGeometry;
use tandem_pdf_engine::pages::PageInfo;
fn page() -> PageInfo {
    PageInfo {
        id: ObjectId::new(1, 0).unwrap(),
        media_box: [-100., -200., 300., 400.],
        crop_box: [-10., 20., 90., 220.],
        rotation: 0,
        user_unit: 2.,
        contents: vec![],
    }
}
#[test]
fn quarter_turns_keep_exact_corner_positions() {
    let mut p = page();
    for (rotation, expected, bounds) in [
        (
            0,
            [[0., 400.], [200., 400.], [200., 0.], [0., 0.]],
            [0., 0., 200., 400.],
        ),
        (
            90,
            [[0., 0.], [0., 200.], [400., 200.], [400., 0.]],
            [0., 0., 400., 200.],
        ),
        (
            180,
            [[200., 0.], [0., 0.], [0., 400.], [200., 400.]],
            [0., 0., 200., 400.],
        ),
        (
            270,
            [[400., 200.], [400., 0.], [0., 0.], [0., 200.]],
            [0., 0., 400., 200.],
        ),
    ] {
        p.rotation = rotation;
        let geometry = PageGeometry::from_page_info(&p).unwrap();
        assert_eq!(geometry.bounds().bounds(), bounds);
        for ((x, y), want) in [(-10., 20.), (90., 20.), (90., 220.), (-10., 220.)]
            .into_iter()
            .zip(expected)
        {
            let point = Point::<UserSpace>::new(x, y).unwrap();
            let mapped = geometry.user_to_page().transform_point(point).unwrap();
            assert_eq!(mapped.coordinates(), want);
            assert_eq!(
                geometry
                    .user_to_page()
                    .inverse()
                    .unwrap()
                    .transform_point(mapped)
                    .unwrap(),
                point
            );
        }
    }
}
#[test]
fn mutated_inventory_cannot_bypass_invariants() {
    for rotation in [1, 45, 360, u16::MAX] {
        let mut p = page();
        p.rotation = rotation;
        assert!(PageGeometry::from_page_info(&p).is_err());
    }
    for unit in [0., -1., 75_001., f64::NAN, f64::INFINITY] {
        let mut p = page();
        p.user_unit = unit;
        assert!(PageGeometry::from_page_info(&p).is_err());
    }
    for crop in [
        [0., 0., 0., 1.],
        [1., 0., 0., 1.],
        [-101., 0., 1., 1.],
        [0., 0., f64::INFINITY, 1.],
        [0., f64::NAN, 1., 1.],
    ] {
        let mut p = page();
        p.crop_box = crop;
        assert!(PageGeometry::from_page_info(&p).is_err());
    }
    let mut p = page();
    p.media_box = [300., 400., -100., -200.];
    assert!(PageGeometry::from_page_info(&p).is_err());
}
#[test]
fn device_transform_has_explicit_positive_scale() {
    let geometry = PageGeometry::from_page_info(&page()).unwrap();
    let device = geometry.page_to_device(144.).unwrap();
    assert_eq!(
        device.transform_bounds(geometry.bounds()).unwrap().bounds(),
        [0., 0., 400., 800.]
    );
    for dpi in [
        0.,
        -1.,
        f64::NAN,
        f64::INFINITY,
        f64::from_bits(1),
        f64::MAX,
    ] {
        assert!(geometry.page_to_device(dpi).is_err());
    }
}
#[test]
fn subunit_boxes_are_preserved_without_native_repair() {
    let mut p = page();
    p.crop_box = [0., 0., 0.25, 0.5];
    p.user_unit = 1.;
    let geometry = PageGeometry::from_page_info(&p).unwrap();
    assert_eq!(geometry.bounds().bounds(), [0., 0., 0.25, 0.5]);
}
#[test]
fn overflow_and_underflow_refuse_without_poisoned_values() {
    let mut p = page();
    p.media_box = [0., 0., f64::MAX, 1.];
    p.crop_box = p.media_box;
    p.user_unit = 2.;
    assert!(PageGeometry::from_page_info(&p).is_err());
    p.media_box = [0., 0., 1., 1.];
    p.crop_box = [0., 0., f64::MIN_POSITIVE, 1.];
    p.user_unit = f64::MIN_POSITIVE;
    assert!(PageGeometry::from_page_info(&p).is_err());
}
