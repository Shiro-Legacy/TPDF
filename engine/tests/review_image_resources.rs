// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent adversarial review of document-bound image metadata and budgets.
use std::sync::Arc;
use tandem_pdf_engine::{
    document::{Document, DocumentLimits},
    image_resources::{ImageErrorKind, ImageLimits, ImageLoader},
    object::ObjectId,
};
fn id(number: u32) -> ObjectId {
    ObjectId::new(number, 0).unwrap()
}
fn image_pdf(metadata: &str, samples: &[u8], extra: &[Vec<u8>]) -> Vec<u8> {
    let mut image=format!("<< /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB {metadata} /Length {} >>\nstream\n",samples.len()).into_bytes();
    image.extend_from_slice(samples);
    image.extend_from_slice(b"\nendstream");
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [] /Count 0 >>".to_vec(),
        image,
    ];
    objects.extend_from_slice(extra);
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
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
#[test]
fn reference_chain_boundary_and_cycles_are_distinct_and_metered() {
    for hops in [64, 65] {
        let objects = (0..hops)
            .map(|i| {
                if i + 1 == hops {
                    b"null".to_vec()
                } else {
                    format!("{} 0 R", i + 5).into_bytes()
                }
            })
            .collect::<Vec<_>>();
        let bytes = image_pdf("/Interpolate 4 0 R", &[1, 2, 3], &objects);
        let document = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let mut loader = ImageLoader::new(&document, ImageLimits::default());
        let result = loader.load(id(3));
        if hops == 64 {
            assert!(result.is_ok());
        } else {
            assert!(matches!(
                result.unwrap_err().kind,
                ImageErrorKind::Limit("reference depth")
            ));
        }
        assert_eq!(loader.usage().reference_hops, 64);
    }
    let bytes = image_pdf(
        "/Interpolate 4 0 R",
        &[1, 2, 3],
        &[b"5 0 R".to_vec(), b"4 0 R".to_vec()],
    );
    let document = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut loader = ImageLoader::new(&document, ImageLimits::default());
    assert!(
        matches!(loader.load(id(3)).unwrap_err().kind,ImageErrorKind::ReferenceCycle(n) if n==id(4))
    );
    assert_eq!(loader.usage().reference_hops, 2);
}
#[test]
fn cached_assets_survive_zero_decode_page_budgets_without_fictitious_work() {
    let bytes = image_pdf("", &[7, 8, 9], &[]);
    let document = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut loader = ImageLoader::new(&document, ImageLimits::default());
    let asset = loader.load(id(3)).unwrap();
    let before = loader.usage();
    loader.begin_page(ImageLimits {
        max_unique_images: 0,
        max_metadata_items: 0,
        max_reference_hops: 0,
        max_encoded_bytes: 0,
        max_decoded_work_bytes: 0,
        max_decoded_rows: 0,
        max_decoded_pixels: 0,
        max_retained_sample_bytes: 0,
        max_allocated_bytes: 0,
        max_filter_specs: 0,
        ..ImageLimits::default()
    });
    let reused = loader.load(id(3)).unwrap();
    assert!(Arc::ptr_eq(&asset, &reused));
    let after = loader.usage();
    assert_eq!(before.allocated_bytes, after.allocated_bytes);
    assert_eq!(before.decoded_work_bytes, after.decoded_work_bytes);
    assert_eq!(before.metadata_items, after.metadata_items);
    assert_eq!(after.attempts, before.attempts + 1);
    assert!(after.lookup_work > before.lookup_work);
    drop(loader);
    drop(document);
    drop(bytes);
    assert_eq!(reused.pixel(0, 0), Some([7, 8, 9]));
}
#[test]
fn every_optional_metadata_reference_resolves_before_null_is_treated_as_absence() {
    for field in [
        "Type",
        "ImageMask",
        "Interpolate",
        "Mask",
        "SMask",
        "Alternates",
        "OPI",
        "FFilter",
        "FDecodeParms",
        "Matte",
        "Intent",
        "SMaskInData",
        "Decode",
        "Filter",
        "DecodeParms",
    ] {
        let bytes = image_pdf(&format!("/{field} 4 0 R"), &[1, 2, 3], &[b"null".to_vec()]);
        let document = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let mut loader = ImageLoader::new(&document, ImageLimits::default());
        assert!(loader.load(id(3)).is_ok(), "null field {field}");
        assert_eq!(loader.usage().reference_hops, 1);
        let bytes = image_pdf(&format!("/{field} 3 0 R"), &[1, 2, 3], &[]);
        let document = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let mut loader = ImageLoader::new(&document, ImageLimits::default());
        let error = loader.load(id(3)).unwrap_err();
        if field == "SMask" {
            assert!(matches!(error.kind, ImageErrorKind::ReferenceCycle(n) if n == id(3)));
            assert_eq!(error.base_image, Some(id(3)));
        } else {
            assert!(
                matches!(
                    error.kind,
                    ImageErrorKind::Invalid("stream-valued metadata")
                ),
                "stream field {field}"
            );
        }
    }
}
#[test]
fn mismatch_cannot_enter_cache_and_success_in_another_document_cannot_alias_it() {
    let bytes = image_pdf("", &[1, 2], &[]);
    let document = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut loader = ImageLoader::new(&document, ImageLimits::default());
    for attempt in 1..=3 {
        assert!(loader.load(id(3)).is_err());
        assert_eq!(loader.usage().attempts, attempt);
        assert_eq!(loader.usage().unique_images, attempt);
        assert_eq!(loader.usage().retained_sample_bytes, attempt * 3);
        assert_eq!(loader.usage().decoded_work_bytes, attempt * 2);
    }
    let second = image_pdf("", &[3, 4, 5], &[]);
    let other = Document::open(&second, DocumentLimits::default()).unwrap();
    let asset = ImageLoader::new(&other, ImageLimits::default())
        .load(id(3))
        .unwrap();
    assert_eq!(asset.samples(), [3, 4, 5]);
    assert!(loader.load(id(3)).is_err());
}

#[test]
fn document_index_lookup_work_is_reserved_before_missing_object_errors() {
    let bytes = image_pdf("", &[1, 2, 3], &[]);
    let document = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut no_work = ImageLoader::new(
        &document,
        ImageLimits {
            max_lookup_work: 1,
            max_allocated_bytes: 0,
            ..ImageLimits::default()
        },
    );
    assert!(matches!(
        no_work.load(id(999)).unwrap_err().kind,
        ImageErrorKind::Limit("lookup work")
    ));
    assert_eq!(no_work.usage().allocated_bytes, 0);
    let mut full = ImageLoader::new(
        &document,
        ImageLimits {
            max_allocated_bytes: 0,
            ..ImageLimits::default()
        },
    );
    assert!(matches!(
        full.load(id(999)).unwrap_err().kind,
        ImageErrorKind::Document(_)
    ));
    assert_eq!(full.usage().allocated_bytes, 0);
    assert!(full.usage().lookup_work > 1);
}

#[test]
fn unrelated_document_objects_cannot_make_index_search_work_free() {
    let bytes = image_pdf("/Interpolate 4 0 R", &[1, 2, 3], &[b"false".to_vec()]);
    let document = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut small = ImageLoader::new(&document, ImageLimits::default());
    small.load(id(3)).unwrap();
    let small_work = small.usage().lookup_work;
    let extra = vec![b"false".to_vec(); 257];
    let bytes = image_pdf("/Interpolate 4 0 R", &[1, 2, 3], &extra);
    let document = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut large = ImageLoader::new(&document, ImageLimits::default());
    large.load(id(3)).unwrap();
    assert!(large.usage().lookup_work > small_work);
}
