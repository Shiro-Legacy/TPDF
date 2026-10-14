// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::{
    document::Document,
    geometry::Affine,
    image_sampling::ImageSamplingError,
    page_glyph::{PageGlyphProgram, PageGlyphSession},
    page_raster::*,
};
fn stream(meta: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< /Length {} {meta} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}
fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        out.extend_from_slice(object);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn program(content: &str) -> PageGlyphProgram {
    program_geometry(content, "")
}
fn program_geometry(content: &str, geometry: &str) -> PageGlyphProgram {
    let bytes = pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 2 2] {geometry} /Resources << /XObject << /Im 5 0 R >> >> /Contents 4 0 R >>").into_bytes(),
        stream("", content.as_bytes()),
        stream("/Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8", &[255,0,0, 0,255,0, 0,0,255, 0,0,0]),
    ]);
    let doc = Document::open(&bytes, Default::default()).unwrap();
    PageGlyphSession::new(&doc, Default::default())
        .unwrap()
        .trace_page(0, Default::default())
        .unwrap()
}
fn target() -> RasterTarget {
    RasterTarget {
        width: 2,
        height: 2,
        page_to_device: Affine::from_pdf([1., 0., 0., 1., 0., 0.]).unwrap(),
    }
}
fn render(content: &str) -> PageRasterImage {
    PageRasterSession::new(Default::default())
        .render(&program(content), target(), Default::default())
        .unwrap()
}
fn rgba(rgb: &[[u8; 3]]) -> Vec<u8> {
    rgb.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect()
}
#[test]
fn source_rows_orientation_reflection_and_owned_pixels() {
    assert_eq!(
        render("2 0 0 2 0 0 cm /Im Do").pixels(),
        rgba(&[[255, 0, 0], [0, 255, 0], [0, 0, 255], [0, 0, 0]])
    );
    assert_eq!(
        render("-2 0 0 2 2 0 cm /Im Do").pixels(),
        rgba(&[[0, 255, 0], [255, 0, 0], [0, 0, 0], [0, 0, 255]])
    );
    assert_eq!(
        render("2 0 0 -2 0 2 cm /Im Do").pixels(),
        rgba(&[[0, 0, 255], [0, 0, 0], [255, 0, 0], [0, 255, 0]])
    );
}
#[test]
fn image_and_path_paints_preserve_order_and_clip_restoration() {
    assert_eq!(
        render(
            "0 1 1 rg 0 0 2 2 re f q 0 0 1 2 re W n 2 0 0 2 0 0 cm /Im Do Q 1 0 1 rg 1 0 1 1 re f"
        )
        .pixels(),
        rgba(&[[255, 0, 0], [0, 255, 255], [0, 0, 255], [255, 0, 255]])
    );
    assert_eq!(
        render("0 1 1 rg 0 0 2 2 re f 2 0 0 2 0 0 cm /Im Do").pixels(),
        render("2 0 0 2 0 0 cm /Im Do").pixels()
    );
}
#[test]
fn subpixel_image_uses_sixteen_samples_and_white_background() {
    // Image lies in the top-left quarter pixel: four source texels contribute
    // one sample each, while the other twelve samples remain white.
    assert_eq!(
        render(".5 0 0 .5 0 1.5 cm /Im Do").pixels(),
        rgba(&[
            [207, 207, 207],
            [255, 255, 255],
            [255, 255, 255],
            [255, 255, 255]
        ])
    );
}
#[test]
fn singular_suffix_is_refused_before_sample_allocation() {
    let p = program("q 2 0 0 2 0 0 cm /Im Do Q 0 0 0 0 1 1 cm /Im Do");
    let mut session = PageRasterSession::new(Default::default());
    let err = session
        .render(&p, target(), Default::default())
        .unwrap_err();
    assert_eq!(
        err.kind,
        PageRasterErrorKind::ImageSampling(ImageSamplingError::Singular)
    );
    assert_eq!(err.operation_index, Some(5));
    assert_eq!(session.image_usage().preparations, 2);
    assert_eq!(session.image_usage().samples, 0);
    assert_eq!(session.scan_usage().samples, 0);
}
#[test]
fn exact_sampling_limits_and_failed_page_charges_are_retained() {
    let p = program("2 0 0 2 0 0 cm /Im Do");
    let mut reference = PageRasterSession::new(Default::default());
    reference.render(&p, target(), Default::default()).unwrap();
    let used = reference.image_usage();
    assert_eq!(used.preparations, 1);
    assert_eq!(used.samples, 64);
    for field in 0..3 {
        let mut page = PageRasterLimits::default();
        match field {
            0 => page.images.max_preparations = used.preparations,
            1 => page.images.max_samples = used.samples,
            _ => page.images.max_work = used.work,
        }
        PageRasterSession::new(Default::default())
            .render(&p, target(), page)
            .unwrap();
        match field {
            0 => page.images.max_preparations -= 1,
            1 => page.images.max_samples -= 1,
            _ => page.images.max_work -= 1,
        }
        let mut session = PageRasterSession::new(Default::default());
        assert!(session.render(&p, target(), page).is_err());
        let failed = session.image_usage();
        session.render(&p, target(), Default::default()).unwrap();
        assert_eq!(session.image_usage().samples, failed.samples + used.samples);
        assert_eq!(session.image_usage().work, failed.work + used.work);
        let mut global = PageRasterSessionLimits::default();
        global.images.max_preparations = used.preparations;
        global.images.max_samples = used.samples;
        global.images.max_work = used.work;
        let mut session = PageRasterSession::new(global);
        session.render(&p, target(), Default::default()).unwrap();
        assert!(session.render(&p, target(), Default::default()).is_err());
        assert_eq!(session.image_usage(), used);
    }
}
#[test]
fn clipped_candidates_still_consume_sampling_budget() {
    let p = program("0 0 0 0 re W n 2 0 0 2 0 0 cm /Im Do");
    let mut session = PageRasterSession::new(Default::default());
    let image = session.render(&p, target(), Default::default()).unwrap();
    assert!(image.pixels().iter().all(|&v| v == 255));
    assert_eq!(session.image_usage().samples, 64);
}
#[test]
fn off_canvas_image_has_no_candidates_but_is_still_validated() {
    let p = program("2 0 0 2 10 10 cm /Im Do");
    let mut session = PageRasterSession::new(Default::default());
    let image = session.render(&p, target(), Default::default()).unwrap();
    assert!(image.pixels().iter().all(|&v| v == 255));
    assert_eq!(session.image_usage().preparations, 1);
    assert_eq!(session.image_usage().samples, 0);
}

#[test]
fn image_placement_obeys_page_rotation_crop_and_user_unit() {
    let colors = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [0, 0, 0]];
    for (angle, order) in [
        (0, [0, 1, 2, 3]),
        (90, [2, 0, 3, 1]),
        (180, [3, 2, 1, 0]),
        (270, [1, 3, 0, 2]),
    ] {
        let p = program_geometry("2 0 0 2 0 0 cm /Im Do", &format!("/Rotate {angle}"));
        let out = PageRasterSession::new(Default::default())
            .render(&p, target(), Default::default())
            .unwrap();
        assert_eq!(
            out.pixels(),
            rgba(&order.map(|i| colors[i])),
            "rotation {angle}"
        );
    }
    let p = program_geometry("2 0 0 2 0 0 cm /Im Do", "/CropBox [0 0 1 2]");
    let out = PageRasterSession::new(Default::default())
        .render(&p, target(), Default::default())
        .unwrap();
    assert_eq!(
        out.pixels(),
        rgba(&[colors[0], [255; 3], colors[2], [255; 3]])
    );
    let p = program_geometry("2 0 0 2 0 0 cm /Im Do", "/UserUnit 2");
    let t = RasterTarget {
        width: 4,
        height: 4,
        ..target()
    };
    let out = PageRasterSession::new(Default::default())
        .render(&p, t, Default::default())
        .unwrap();
    for y in 0..4 {
        for x in 0..4 {
            assert_eq!(
                &out.pixels()[(y * 4 + x) * 4..(y * 4 + x) * 4 + 3],
                &colors[(y / 2) * 2 + x / 2]
            );
        }
    }
}
