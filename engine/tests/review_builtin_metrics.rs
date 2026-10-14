// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent exact-name, font identity and cumulative M1 budget review.
use tandem_pdf_engine::{
    builtin_metrics::{BuiltinFace, BuiltinMetricLookup, BuiltinMetrics},
    document::{Document, DocumentLimits},
    font_resources::{
        FontIdentity, FontResourceErrorKind, FontResourceLimitKind, FontResourceLimits,
        FontResourceLoader,
    },
    object::ObjectId,
    simple_encoding::{Difference, EncodingLimits, GlyphUnicode, NamedEncoding, SimpleEncoding},
};
fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}
fn document(extra: &[String]) -> Vec<u8> {
    let mut bodies=vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string(),"<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /Font << /A 4 0 R /Alias 4 0 R /B 5 0 R >> >> >>".to_string(),"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>".to_string()];
    bodies.extend_from_slice(extra);
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in &offsets[1..] {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    bytes
}

#[test]
fn shared_encoding_keeps_face_specific_metrics_and_alias_cache_identity() {
    let bytes = document(&[
        "<< /Subtype /Type1 /BaseFont /Helvetica /Encoding 6 0 R >>".into(),
        "<< /Subtype /Type1 /BaseFont /Times-Roman /Encoding 6 0 R >>".into(),
        "<< /BaseEncoding /WinAnsiEncoding /Differences [65 /fi] >>".into(),
    ]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    for (name, expected) in [(b"A".as_slice(), 500.), (b"B", 556.), (b"Alias", 500.)] {
        let BuiltinMetricLookup::Advance(width) = loader
            .load_builtin_metrics(id(3), name)
            .unwrap()
            .metrics()
            .lookup(65)
        else {
            panic!()
        };
        assert_eq!(width.value(), expected);
    }
    assert_eq!(loader.usage().builtin_metrics, 2);
    assert_eq!(loader.usage().builtin_slots, 512);
    assert_eq!(loader.usage().difference_items, 4);
    assert_eq!(loader.usage().builtin_metric_attempts, 3);
    assert_eq!(loader.usage().fonts, 0);
}
#[test]
fn shared_encoding_budget_failure_keeps_prior_cache_and_no_partial_metrics() {
    let bytes=document(&[
 "<< /Subtype /Type1 /BaseFont /Helvetica /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /fi] >> >>".into(),
 "null".into(),
 ]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut loader = FontResourceLoader::new(
        &doc,
        FontResourceLimits {
            max_difference_name_bytes: 2,
            ..FontResourceLimits::default()
        },
    )
    .unwrap();
    let cached = loader.load(id(3), b"A").unwrap() as *const _;
    let error = loader.load_builtin_metrics(id(3), b"Alias").unwrap_err();
    assert_eq!(error.difference_index, Some(1));
    assert!(matches!(
        error.kind,
        FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::DifferenceNameBytes)
    ));
    assert_eq!(loader.usage().builtin_slots, 256);
    assert_eq!(loader.usage().builtin_metrics, 0);
    assert_eq!(loader.usage().retained_builtin_slots, 0);
    assert!(
        loader
            .loaded_builtin_metrics(FontIdentity::Indirect(id(4)))
            .is_none()
    );
    assert_eq!(cached, loader.load(id(3), b"Alias").unwrap() as *const _);
}
#[test]
fn exact_glyph_metrics_do_not_require_unicode_registry_membership() {
    for face in [BuiltinFace::Helvetica, BuiltinFace::TimesRoman] {
        let mut checked = 0;
        for (name, expected) in face.glyphs() {
            if name == b".notdef" {
                continue;
            }
            let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi)
                .with_differences(
                    &[Difference::Code(65), Difference::Name(name)],
                    EncodingLimits::default(),
                )
                .unwrap();
            if matches!(
                encoding.lookup_unicode(65),
                GlyphUnicode::UnknownGlyphName(_)
            ) {
                let metrics = BuiltinMetrics::from_encoding(face, encoding);
                assert_eq!(metrics.lookup(65), BuiltinMetricLookup::Advance(expected));
                checked += 1;
            }
        }
        assert!(checked > 100, "must exercise the metrics-only registry");
    }
}
#[test]
fn indirect_embedded_program_failures_precede_slot_processing_and_encoding() {
    for descriptor in [
        "<< /FontFile 6 0 R >>",  // dangling program reference
        "<< /FontFile2 5 0 R >>", // descriptor dictionary reused as a non-null program declaration
        "<< /FontFile3 false >>", // any non-null declaration excludes fallback
    ] {
        let bytes=document(&[
   "<< /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /FontDescriptor 5 0 R >>".into(),
   descriptor.into(),
  ]);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
        assert!(loader.load_builtin_metrics(id(3), b"A").is_err());
        assert_eq!(loader.usage().builtin_slots, 0);
        assert_eq!(loader.usage().builtin_metrics, 0);
        assert_eq!(loader.usage().difference_items, 0);
    }
}
