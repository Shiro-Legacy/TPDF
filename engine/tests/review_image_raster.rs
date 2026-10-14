// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::{
    document::{Document, DocumentLimits},
    geometry::Affine,
    image_sampling::{ImageSamplingError, ImageSamplingLimits},
    page_glyph::{PageGlyphProgram, PageGlyphSession},
    page_raster::{
        PageRasterErrorKind, PageRasterLimits, PageRasterSession, PageRasterSessionLimits,
        RasterTarget,
    },
};
fn program(content: &str) -> PageGlyphProgram {
    let objects=[b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] /Resources << /XObject << /Im 5 0 R >> >> /Contents 4 0 R >>".to_vec(),format!("<< /Length {} >>\nstream\n{content}\nendstream",content.len()).into_bytes(),b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Length 3 >>\nstream\n\xff\x00\x00\nendstream".to_vec()];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let document = Document::open(&bytes, DocumentLimits::default()).unwrap();
    PageGlyphSession::new(&document, Default::default())
        .unwrap()
        .trace_page(0, Default::default())
        .unwrap()
}
fn target() -> RasterTarget {
    RasterTarget {
        width: 1,
        height: 1,
        page_to_device: Affine::from_pdf([1., 0., 0., 1., 0., 0.]).unwrap(),
    }
}
#[test]
fn fractional_image_and_later_path_compose_before_single_sample_resolve() {
    let p = program("0 0 1 rg 0 0 1 1 re f q 0.5 0 0 1 0 0 cm /Im Do Q 0 1 0 rg 0 0 0.5 0.5 re f");
    let raster = PageRasterSession::new(Default::default())
        .render(&p, target(), Default::default())
        .unwrap();
    assert_eq!(raster.pixels(), [64, 64, 128, 255]);
}
#[test]
fn empty_clip_cannot_evade_candidate_sample_cap() {
    let p = program("0 0 0 0 re W n /Im Do /Im Do");
    let mut s = PageRasterSession::new(Default::default());
    let limits = PageRasterLimits {
        images: ImageSamplingLimits {
            max_samples: 31,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(matches!(
        s.render(&p, target(), limits).unwrap_err().kind,
        PageRasterErrorKind::ImageSampling(ImageSamplingError::Limit("samples"))
    ));
    assert_eq!(s.image_usage().preparations, 2);
    assert_eq!(s.image_usage().samples, 31);
    let mut s = PageRasterSession::new(Default::default());
    let limits = PageRasterLimits {
        images: ImageSamplingLimits {
            max_samples: 32,
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(s.render(&p, target(), limits).unwrap().pixels(), [255; 4]);
    assert_eq!(s.image_usage().samples, 32);
}
#[test]
fn failed_page_samples_remain_charged_and_exact_session_remainder_succeeds() {
    let p = program("/Im Do");
    let limits = PageRasterSessionLimits {
        images: ImageSamplingLimits {
            max_samples: 31,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut s = PageRasterSession::new(limits);
    let short = PageRasterLimits {
        images: ImageSamplingLimits {
            max_samples: 15,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(s.render(&p, target(), short).is_err());
    assert_eq!(s.image_usage().samples, 15);
    assert_eq!(
        s.render(&p, target(), Default::default()).unwrap().pixels(),
        [255, 0, 0, 255]
    );
    assert_eq!(s.image_usage().samples, 31);
    assert!(s.render(&p, target(), Default::default()).is_err());
    assert_eq!(s.image_usage().samples, 31);
    assert_eq!(s.usage().page_attempts, 3);
}
#[test]
fn singular_image_refuses_during_whole_page_preflight_before_pixel_buffers() {
    let p = program("0 0 0 0 0 0 cm /Im Do");
    let mut s = PageRasterSession::new(Default::default());
    assert!(matches!(
        s.render(&p, target(), Default::default()).unwrap_err().kind,
        PageRasterErrorKind::ImageSampling(ImageSamplingError::Singular)
    ));
    assert_eq!(s.usage().allocated_bytes, 0);
    assert_eq!(s.image_usage().preparations, 1);
    assert_eq!(s.image_usage().samples, 0);
    assert_eq!(s.scan_usage().samples, 0);
}
