// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent failed-work reservations, error context and document-scoped caches.
use std::rc::Rc;
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::font_resources::{
    FontIdentity, FontResourceErrorKind, FontResourceLimitKind, FontResourceLimits,
    FontResourceLoader, SimpleFontMapping,
};
use tandem_pdf_engine::object::ObjectId;
use tandem_pdf_engine::simple_encoding::GlyphUnicode;
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
fn stream(text: &str) -> String {
    format!("<< /Length {} >>\nstream\n{text}\nendstream", text.len())
}
fn cmap() -> &'static str {
    "begincmap 1 begincodespacerange <00><ff> endcodespacerange 1 beginbfchar <41><0042> endbfchar endcmap"
}
#[test]
fn failed_cmap_parses_consume_reservations_without_publishing_a_font() {
    let malformed = "begincmap 1 begincodespacerange <00><ff> endcodespacerange 1 beginbfchar <41> 00 endbfchar endcmap";
    let bytes = document(&[
        "<< /Subtype /Type1 /ToUnicode 6 0 R /Encoding /WinAnsiEncoding >>".into(),
        "null".into(),
        stream(malformed),
    ]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let limits = FontResourceLimits {
        max_cmap_entries: 4,
        max_cmap_payload_bytes: 32,
        cmap: tandem_pdf_engine::cmap::CMapLimits {
            max_mappings: 2,
            max_owned_bytes: 16,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut loader = FontResourceLoader::new(&doc, limits).unwrap();
    for attempt in 1..=2 {
        let error = loader.load(id(3), b"A").unwrap_err();
        assert!(matches!(error.kind, FontResourceErrorKind::CMap(_)));
        assert_eq!(loader.usage().cmap_entries, attempt * 2);
        assert_eq!(loader.usage().cmap_payload_bytes, attempt * 16);
        assert_eq!(loader.usage().cmap_input_bytes, attempt * malformed.len());
        assert_eq!(loader.usage().fonts, 0);
        assert_eq!(loader.usage().maps, 0);
        assert!(loader.loaded(FontIdentity::Indirect(id(4))).is_none());
    }
    assert!(loader.load(id(3), b"A").is_err());
    assert_eq!(loader.usage().cmap_entries, 4);
    assert_eq!(loader.usage().cmap_payload_bytes, 32);
}
#[test]
fn map_sharing_preserves_authority_and_charges_selection_without_redecode() {
    let bytes = document(&[
        "<< /Subtype /Type1 /ToUnicode 6 0 R /Encoding 999 0 R >>".into(),
        "<< /Subtype /TrueType /ToUnicode 6 0 R >>".into(),
        stream(cmap()),
    ]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
    let first = match loader.load(id(3), b"A").unwrap().mapping() {
        SimpleFontMapping::ToUnicode(map) => Rc::clone(map),
        _ => panic!("wrong mapping"),
    };
    let initial = loader.usage();
    let second = match loader.load(id(3), b"B").unwrap().mapping() {
        SimpleFontMapping::ToUnicode(map) => Rc::clone(map),
        _ => panic!("wrong mapping"),
    };
    assert!(Rc::ptr_eq(&first, &second));
    let alias = loader.load(id(3), b"Alias").unwrap();
    assert_eq!(alias.identity(), FontIdentity::Indirect(id(4)));
    let final_usage = loader.usage();
    assert_eq!(final_usage.calls, initial.calls + 2);
    assert!(final_usage.reference_hops > initial.reference_hops);
    assert_eq!(final_usage.encoded_bytes, initial.encoded_bytes);
    assert_eq!(final_usage.decoded_work_bytes, initial.decoded_work_bytes);
    assert_eq!(final_usage.cmap_input_bytes, initial.cmap_input_bytes);
    assert_eq!(final_usage.maps, 1);
    assert_eq!(final_usage.fonts, 2);
}
#[test]
fn differences_nested_reference_and_name_work_errors_keep_original_index() {
    for (differences, name_cap, missing) in
        [("[0 /A 999 0 R]", 100, true), ("[0 /A /longer]", 1, false)]
    {
        let bytes = document(&[
            format!(
                "<< /Subtype /Type1 /Encoding << /BaseEncoding /WinAnsiEncoding /Differences {differences} >> >>"
            ),
            "null".into(),
        ]);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let limits = FontResourceLimits {
            max_difference_name_bytes: name_cap,
            ..FontResourceLimits::default()
        };
        let mut loader = FontResourceLoader::new(&doc, limits).unwrap();
        let error = loader.load(id(3), b"A").unwrap_err();
        assert_eq!(error.difference_index, Some(2));
        assert_eq!(error.page, Some(id(3)));
        assert_eq!(error.font, Some(FontIdentity::Indirect(id(4))));
        if missing {
            assert!(matches!(error.kind, FontResourceErrorKind::Document(_)));
        } else {
            assert!(matches!(
                error.kind,
                FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::DifferenceNameBytes)
            ));
        }
        assert_eq!(loader.usage().fonts, 0);
    }
}
#[test]
fn identical_object_numbers_in_distinct_documents_do_not_share_encoding_state() {
    for glyph in ["A", "B", ".notdef", "Opaque"] {
        let bytes = document(&[
            format!(
                "<< /Subtype /Type1 /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /{glyph}] >> >>"
            ),
            "null".into(),
        ]);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let mut loader = FontResourceLoader::new(&doc, FontResourceLimits::default()).unwrap();
        let font = loader.load(id(3), b"A").unwrap();
        let SimpleFontMapping::ExplicitEncoding { encoding, .. } = font.mapping() else {
            panic!("wrong map")
        };
        assert_eq!(encoding.glyph_name(65), Some(glyph.as_bytes()));
        match glyph {
            "A" => assert_eq!(encoding.lookup_unicode(65), GlyphUnicode::Unicode(&['A'])),
            "B" => assert_eq!(encoding.lookup_unicode(65), GlyphUnicode::Unicode(&['B'])),
            ".notdef" => assert_eq!(encoding.lookup_unicode(65), GlyphUnicode::UndefinedCode),
            _ => assert_eq!(
                encoding.lookup_unicode(65),
                GlyphUnicode::UnknownGlyphName(b"Opaque")
            ),
        }
    }
}
