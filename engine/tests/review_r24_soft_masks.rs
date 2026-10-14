// SPDX-License-Identifier: AGPL-3.0-or-later
// Independently authored R24 review regressions.
use tandem_pdf_engine::{
    document::Document,
    image_resources::{ImageErrorKind, ImageLimits, ImageLoader},
    object::ObjectId,
};
fn oid(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}
fn image(extra: &str, data: &[u8]) -> Vec<u8> {
    let mut out=format!("<< /Subtype /Image /Width 4 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length {} {} >>\nstream\n",data.len(),extra).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}
fn document(objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(obj);
        out.extend_from_slice(b"\nendobj\n");
    }
    let start = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}
fn fixture(base: &str, mask: &str, alpha: &[u8], mut rest: Vec<Vec<u8>>) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog >>".to_vec(),
        image(base, &[9, 8, 7, 6]),
        image(mask, alpha),
    ];
    objects.append(&mut rest);
    document(objects)
}
#[test]
fn stream_alias_depth_boundary_counts_final_stream_hop() {
    for hops in [64u32, 65] {
        let mut rest = Vec::new();
        for i in 0..hops - 1 {
            rest.push(format!("{} 0 R", if i == hops - 2 { 3 } else { 5 + i }).into_bytes());
        }
        let bytes = fixture("/SMask 4 0 R", "", &[255; 4], rest);
        let doc = Document::open(&bytes, Default::default()).unwrap();
        let mut loader = ImageLoader::new(&doc, Default::default());
        let result = loader.load(oid(2));
        assert_eq!(result.is_ok(), hops == 64);
        assert_eq!(loader.usage().reference_hops, 64);
        if let Ok(asset) = result {
            assert_eq!(asset.opaque_mask().unwrap().object_id(), oid(3));
        } else {
            assert_eq!(loader.usage().mask_certificate_attempts, 0);
        }
    }
}
#[test]
fn failed_certificate_allowance_preserves_decoded_mask_but_not_base() {
    let bytes = fixture("/SMask 3 0 R", "", &[255; 4], vec![]);
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut loader = ImageLoader::new(&doc, Default::default());
    loader.begin_page(ImageLimits {
        max_mask_certificate_attempts: 0,
        ..Default::default()
    });
    let err = loader.load(oid(2)).unwrap_err();
    assert_eq!(err.base_image, Some(oid(2)));
    assert_eq!(err.object, oid(3));
    assert_eq!(loader.usage().unique_images, 2);
    assert_eq!(loader.usage().opacity_scan_samples, 0);
    loader.begin_page(Default::default());
    let asset = loader.load(oid(2)).unwrap();
    assert_eq!(loader.usage().unique_images, 3);
    assert_eq!(loader.usage().mask_certificate_attempts, 1);
    assert_eq!(asset.opaque_mask().unwrap().samples(), &[255; 4]);
}
#[test]
fn failed_scan_is_reserved_again_after_page_reset() {
    let bytes = fixture("/SMask 3 0 R", "/Decode [1 0]", &[0, 0, 0, 1], vec![]);
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut loader = ImageLoader::new(
        &doc,
        ImageLimits {
            max_opacity_scan_samples: 8,
            max_mask_certificate_attempts: 2,
            ..Default::default()
        },
    );
    for i in 1..=2 {
        loader.begin_page(ImageLimits {
            max_opacity_scan_samples: 4,
            max_mask_certificate_attempts: 1,
            ..Default::default()
        });
        assert!(matches!(
            loader.load(oid(2)).unwrap_err().kind,
            ImageErrorKind::Unsupported("nonopaque soft mask sample")
        ));
        assert_eq!(loader.usage().opacity_scan_samples, 4 * i);
    }
    loader.begin_page(Default::default());
    assert!(matches!(
        loader.load(oid(2)).unwrap_err().kind,
        ImageErrorKind::Limit("mask certificate attempts")
    ));
    assert_eq!(loader.usage().opacity_scan_samples, 8);
}
#[test]
fn ordinary_cached_mask_cannot_hide_indirect_role_semantics() {
    for role in ["/TR 4 0 R", "/SMaskInData 4 0 R"] {
        let bytes = fixture(
            "/SMask 3 0 R",
            role,
            &[255; 4],
            vec![if role.starts_with("/TR ") {
                b"/Identity".to_vec()
            } else {
                b"0".to_vec()
            }],
        );
        let doc = Document::open(&bytes, Default::default()).unwrap();
        let mut loader = ImageLoader::new(&doc, Default::default());
        loader.load(oid(3)).unwrap();
        let err = loader.load(oid(2)).unwrap_err();
        assert_eq!(err.object, oid(3));
        assert_eq!(err.base_image, Some(oid(2)));
        assert_eq!(loader.usage().mask_certificate_attempts, 0);
    }
}
#[test]
fn alias_to_null_charges_attempt_and_stream_metadata_stays_forbidden() {
    let bytes = fixture(
        "/SMask 4 0 R",
        "",
        &[255; 4],
        vec![b"5 0 R".to_vec(), b"null".to_vec()],
    );
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut loader = ImageLoader::new(&doc, Default::default());
    assert!(loader.load(oid(2)).unwrap().opaque_mask().is_none());
    assert_eq!(loader.usage().mask_dependency_attempts, 1);
    assert_eq!(loader.usage().reference_hops, 2);
    let bytes = fixture(
        "/SMask 3 0 R",
        "/TR 4 0 R",
        &[255; 4],
        vec![image("", &[255; 4])],
    );
    let doc = Document::open(&bytes, Default::default()).unwrap();
    let mut loader = ImageLoader::new(&doc, Default::default());
    let err = loader.load(oid(2)).unwrap_err();
    assert!(matches!(
        err.kind,
        ImageErrorKind::Invalid("stream-valued metadata")
    ));
    assert_eq!(err.object, oid(3));
    assert_eq!(err.field, "TR");
}
