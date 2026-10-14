// SPDX-License-Identifier: AGPL-3.0-or-later
use miniz_oxide::deflate::compress_to_vec_zlib;
use std::sync::Arc;
use tandem_pdf_engine::{
    document::Document,
    image_resources::{ImageAsset, ImageErrorKind, ImageLimits, ImageLoader},
    object::ObjectId,
    page_program::{PageProgramErrorKind, PageProgramSession},
};
fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}
fn stream(meta: &str, bytes: &[u8]) -> Vec<u8> {
    let mut b = format!("<< /Length {} {meta} >>\nstream\n", bytes.len()).into_bytes();
    b.extend(bytes);
    b.extend(b"\nendstream");
    b
}
fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut b = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![];
    for (i, o) in objects.iter().enumerate() {
        offsets.push(b.len());
        b.extend(format!("{} 0 obj\n", i + 1).bytes());
        b.extend(o);
        b.extend(b"\nendobj\n");
    }
    let x = b.len();
    b.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for o in offsets {
        b.extend(format!("{o:010} 00000 n \n").bytes());
    }
    b.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n",
            objects.len() + 1
        )
        .bytes(),
    );
    b
}
const GRAY: &str = "/Subtype /Image /Width 3 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray";
fn objects(base: &str, mask: &str, raw: &[u8]) -> Vec<Vec<u8>> {
    vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Resources << /XObject << /Im 5 0 R /Other 7 0 R /Form 8 0 R >> >> /Contents 4 0 R >>".to_vec(),stream("",b"/Im Do"),stream(&format!("{GRAY} {base}"),&[17,80,203]),stream(mask,raw),stream(GRAY,&[10,20,30]),stream("/Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /Im 5 0 R >> >>",b"/Im Do")]
}
fn bytes(mask: &str, raw: &[u8]) -> Vec<u8> {
    pdf(&objects("/SMask 6 0 R", mask, raw))
}
fn load(b: &[u8]) -> Arc<ImageAsset> {
    let d = Document::open(b, Default::default()).unwrap();
    ImageLoader::new(&d, Default::default())
        .load(id(5))
        .unwrap()
}
#[test]
fn owned_opacity_proof_normal_inverted_and_alias_identity() {
    for (decode, raw) in [("", [255; 3]), ("/Decode [1 0]", [0; 3])] {
        let mut o = objects("/SMask 9 0 R", &format!("{GRAY} {decode}"), &raw);
        o.push(b"10 0 R".to_vec());
        o.push(b"6 0 R".to_vec());
        let a = load(&pdf(&o));
        let m = a.opaque_mask().unwrap();
        assert_eq!(m.object_id(), id(6));
        assert_eq!(m.pixel(0, 2), Some([255; 3]));
        assert_eq!(a.pixel(0, 0), Some([17; 3]));
        assert!(m.opaque_mask().is_none());
    }
}
#[test]
fn full_scan_reservation_and_failed_attempts_are_retained() {
    for index in 0..3 {
        let mut raw = [255; 3];
        raw[index] = 254;
        let b = bytes(GRAY, &raw);
        let d = Document::open(&b, Default::default()).unwrap();
        let mut l = ImageLoader::new(
            &d,
            ImageLimits {
                max_opacity_scan_samples: 6,
                ..Default::default()
            },
        );
        for attempt in 1..=2 {
            let e = l.load(id(5)).unwrap_err();
            assert_eq!(e.base_image, Some(id(5)));
            assert_eq!(e.object, id(6));
            assert_eq!(e.field, "opacity");
            assert_eq!(l.usage().opacity_scan_samples, 3 * attempt);
            assert_eq!(l.usage().mask_certificate_attempts, attempt);
        }
        assert!(l.load(id(5)).is_err());
        assert_eq!(l.usage().opacity_scan_samples, 6);
        assert!(l.load(id(7)).is_ok());
        assert_eq!(l.usage().mask_dependency_attempts, 3);
    }
}
#[test]
fn all_new_counters_have_exact_page_and_session_boundaries() {
    let b = bytes(GRAY, &[255; 3]);
    let d = Document::open(&b, Default::default()).unwrap();
    for which in 0..3 {
        for per_page in [false, true] {
            for below in [false, true] {
                let n = if which == 2 { 3 } else { 1 };
                let mut limits = ImageLimits::default();
                match which {
                    0 => limits.max_mask_dependency_attempts = n - usize::from(below),
                    1 => limits.max_mask_certificate_attempts = n - usize::from(below),
                    _ => limits.max_opacity_scan_samples = n - usize::from(below),
                }
                let mut l = ImageLoader::new(
                    &d,
                    if per_page {
                        ImageLimits::default()
                    } else {
                        limits
                    },
                );
                if per_page {
                    l.begin_page(limits);
                }
                assert_eq!(
                    l.load(id(5)).is_ok(),
                    !below,
                    "counter={which} page={per_page}"
                );
            }
        }
    }
}
#[test]
fn cache_order_and_shared_mask_reuse_no_decode_or_scan() {
    for mask_first in [false, true] {
        let mut o = objects("/SMask 6 0 R", GRAY, &[255; 3]);
        o[6] = stream(&format!("{GRAY} /SMask 6 0 R"), &[9, 8, 7]);
        let b = pdf(&o);
        let d = Document::open(&b, Default::default()).unwrap();
        let mut l = ImageLoader::new(&d, Default::default());
        let direct = if mask_first {
            Some(l.load(id(6)).unwrap())
        } else {
            None
        };
        let a = l.load(id(5)).unwrap();
        let other = l.load(id(7)).unwrap();
        let m = direct.unwrap_or_else(|| l.load(id(6)).unwrap());
        assert!(Arc::ptr_eq(a.opaque_mask().unwrap(), &m));
        assert!(Arc::ptr_eq(other.opaque_mask().unwrap(), &m));
        assert_eq!(l.usage().mask_certificate_attempts, 1);
        assert_eq!(l.usage().opacity_scan_samples, 3);
        let u = l.usage();
        l.begin_page(ImageLimits {
            max_unique_images: 0,
            max_mask_certificate_attempts: 0,
            max_opacity_scan_samples: 0,
            max_encoded_bytes: 0,
            max_decoded_work_bytes: 0,
            max_allocated_bytes: 0,
            ..Default::default()
        });
        assert!(Arc::ptr_eq(&a, &l.load(id(5)).unwrap()));
        assert_eq!(l.usage().allocated_bytes, u.allocated_bytes);
        assert_eq!(l.usage().decoded_work_bytes, u.decoded_work_bytes);
        assert_eq!(
            l.usage().mask_dependency_attempts,
            u.mask_dependency_attempts + 1
        );
    }
}
#[test]
fn cached_base_rechecks_mask_filter_and_role_caps() {
    let b = bytes(
        &format!("{GRAY} /Filter /FlateDecode"),
        &compress_to_vec_zlib(&[255; 3], 6),
    );
    let d = Document::open(&b, Default::default()).unwrap();
    let mut l = ImageLoader::new(&d, Default::default());
    l.load(id(5)).unwrap();
    l.begin_page(ImageLimits {
        max_filters: 0,
        ..Default::default()
    });
    let e = l.load(id(5)).unwrap_err();
    assert_eq!(e.object, id(6));
    assert_eq!(e.base_image, Some(id(5)));
    assert!(matches!(e.kind, ImageErrorKind::Limit("filters")));
}
#[test]
fn page_resets_do_not_refuel_session_dependency_budget() {
    let b = bytes(GRAY, &[255; 3]);
    let d = Document::open(&b, Default::default()).unwrap();
    let mut l = ImageLoader::new(
        &d,
        ImageLimits {
            max_mask_dependency_attempts: 2,
            ..Default::default()
        },
    );
    for _ in 0..2 {
        l.begin_page(ImageLimits {
            max_mask_dependency_attempts: 1,
            ..Default::default()
        });
        l.load(id(5)).unwrap();
        assert!(l.load(id(5)).is_err());
    }
    l.begin_page(Default::default());
    assert!(l.load(id(5)).is_err());
    assert_eq!(l.usage().mask_dependency_attempts, 2);
}
#[test]
fn document_identity_never_shares_certificates() {
    let a = load(&bytes(GRAY, &[255; 3]));
    let b = bytes(GRAY, &[255, 254, 255]);
    let d = Document::open(&b, Default::default()).unwrap();
    assert!(
        ImageLoader::new(&d, Default::default())
            .load(id(5))
            .is_err()
    );
    assert_eq!(a.opaque_mask().unwrap().pixel(0, 1), Some([255; 3]));
}
#[test]
fn role_rejects_ordinary_cached_transfer_function_and_nested_mask() {
    for extra in ["/TR /Identity", "/SMaskInData 0", "/SMask 7 0 R"] {
        let b = bytes(&format!("{GRAY} {extra}"), &[255; 3]);
        let d = Document::open(&b, Default::default()).unwrap();
        let mut l = ImageLoader::new(&d, Default::default());
        if extra != "/SMask 7 0 R" {
            l.load(id(6)).unwrap();
        }
        let e = l.load(id(5)).unwrap_err();
        assert_eq!(e.object, id(6));
        assert_eq!(e.base_image, Some(id(5)));
    }
}
#[test]
fn malformed_masks_and_late_nonopaque_bytes_never_publish_base() {
    for (meta, raw) in [
        (format!("{GRAY} /Matte [0]"), vec![255; 3]),
        (format!("{GRAY} /Mask []"), vec![255; 3]),
        (format!("{GRAY} /Interpolate true"), vec![255; 3]),
        (
            "/Subtype /Image /Width 3 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB".into(),
            vec![255; 9],
        ),
        (
            "/Subtype /Image /Width 1 /Height 3 /BitsPerComponent 8 /ColorSpace /DeviceGray".into(),
            vec![255; 3],
        ),
        (GRAY.into(), vec![255; 2]),
        (GRAY.into(), vec![255; 4]),
        (format!("{GRAY} /TR 99 0 R"), vec![255; 3]),
        (format!("{GRAY} /Decode [0 0.5]"), vec![255; 3]),
    ] {
        let b = bytes(&meta, &raw);
        let d = Document::open(&b, Default::default()).unwrap();
        let mut l = ImageLoader::new(&d, Default::default());
        for _ in 0..2 {
            assert!(l.load(id(5)).is_err(), "{meta}");
        }
    }
}
#[test]
fn aliases_null_dangling_self_and_nested_dependencies() {
    for base in [
        "/SMask 5 0 R",
        "/SMask 99 0 R",
        "/SMask <<>>",
        "/SMask /None",
    ] {
        let b = pdf(&objects(base, GRAY, &[255; 3]));
        let d = Document::open(&b, Default::default()).unwrap();
        assert!(
            ImageLoader::new(&d, Default::default())
                .load(id(5))
                .is_err()
        );
    }
    for nested in ["/SMask 5 0 R", "/SMask 6 0 R", "/SMask 7 0 R"] {
        let b = bytes(&format!("{GRAY} {nested}"), &[255; 3]);
        let d = Document::open(&b, Default::default()).unwrap();
        assert!(
            ImageLoader::new(&d, Default::default())
                .load(id(5))
                .is_err()
        );
    }
    let mut o = objects("/SMask 9 0 R", GRAY, &[255; 3]);
    o.push(b"10 0 R".to_vec());
    o.push(b"9 0 R".to_vec());
    let b = pdf(&o);
    let d = Document::open(&b, Default::default()).unwrap();
    assert!(matches!(
        ImageLoader::new(&d, Default::default())
            .load(id(5))
            .unwrap_err()
            .kind,
        ImageErrorKind::ReferenceCycle(_)
    ));
    o[9] = b"null".to_vec();
    assert!(load(&pdf(&o)).opaque_mask().is_none());
}
#[test]
fn exact_allocation_includes_mask_asset_certificate_and_cache_growth() {
    let b = bytes(GRAY, &[255; 3]);
    let d = Document::open(&b, Default::default()).unwrap();
    let mut l = ImageLoader::new(&d, Default::default());
    l.load(id(5)).unwrap();
    let n = l.usage().allocated_bytes;
    for delta in [0, 1] {
        let mut l = ImageLoader::new(
            &d,
            ImageLimits {
                max_allocated_bytes: n - delta,
                ..Default::default()
            },
        );
        assert_eq!(l.load(id(5)).is_ok(), delta == 0);
    }
}
#[test]
fn page_validation_covers_clipped_offscreen_and_late_form_failures() {
    for content in [
        "0 0 0 0 re W n /Im Do",
        "1 0 0 1 100 100 cm /Im Do",
        "/Other Do /Im Do",
        "/Other Do /Form Do",
    ] {
        let mut o = objects("/SMask 6 0 R", GRAY, &[255, 255, 254]);
        o[3] = stream("", content.as_bytes());
        let b = pdf(&o);
        let d = Document::open(&b, Default::default()).unwrap();
        let mut s = PageProgramSession::new(&d, Default::default()).unwrap();
        let e = s.trace_page(0, Default::default()).unwrap_err();
        let PageProgramErrorKind::Image(e) = e.kind else {
            panic!("wrong refusal: {e:?}")
        };
        assert_eq!(e.base_image, Some(id(5)));
        assert_eq!(e.object, id(6));
        assert_eq!(s.image_usage().opacity_scan_samples, 3);
    }
}
#[test]
fn page_assets_retain_complete_mask_lifetime() {
    let page = {
        let b = bytes(GRAY, &[255; 3]);
        let d = Document::open(&b, Default::default()).unwrap();
        PageProgramSession::new(&d, Default::default())
            .unwrap()
            .trace_page(0, Default::default())
            .unwrap()
    };
    assert_eq!(page.images()[0].opaque_mask().unwrap().samples(), &[255; 3]);
}
