// SPDX-License-Identifier: AGPL-3.0-or-later
//! The document layer an app session builds on: owned input, the object
//! convenience API, text strings, resolution, journal primitives, mutations,
//! and the app writer (a full rewrite that drops unused objects, compact
//! syntax).
#[path = "support/pages.rs"]
mod pages;
use std::collections::BTreeMap;

use tandem_pdf_engine::document::{Document, DocumentLimits, ObjectState, TrailerState};
use tandem_pdf_engine::object::{
    Dictionary, Object, ObjectId, decode_text_string, encode_text_string,
};
use tandem_pdf_engine::parser::{ParseLimits, parse_complete};
use tandem_pdf_engine::serialize::{SerializeStyle, WriteLimits, serialize_object_with_style};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::writer::{
    DocumentWriteError, DocumentWriteLimits, Encryption, WriteOptions, rewrite_document,
    rewrite_reachable, write_document,
};

fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("../fixtures/{name}")).unwrap()
}
fn pdf(objects: &[(u32, &[u8])], trailer: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for &(n, value) in objects {
        offsets.push((n, bytes.len()));
        bytes.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        bytes.extend_from_slice(value);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 1\n0000000000 65535 f \n");
    for (n, offset) in offsets {
        bytes.extend_from_slice(format!("{n} 1\n{offset:010} 00000 n \n").as_bytes());
    }
    let size = objects.iter().map(|v| v.0).max().unwrap() + 1;
    bytes.extend_from_slice(
        format!("trailer\n<< /Size {size} {trailer} >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}
fn stream(dictionary: &str, data: &[u8]) -> Vec<u8> {
    let mut bytes = format!("<< {dictionary} /Length {} >>\nstream\n", data.len()).into_bytes();
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(b"\nendstream");
    bytes
}
fn graph() -> Vec<u8> {
    let flate = miniz_oxide::deflate::compress_to_vec_zlib(b"q 1 0 0 1 0 0 cm Q", 6);
    let content = stream("", b"BT /F1 12 Tf (Hi) Tj ET");
    let packed = stream("/Filter /FlateDecode", &flate);
    pdf(
        &[
            (1, b"<< /Type /Catalog /Pages 2 0 R /Extra [5 0 R 6 0 R] >>"),
            (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (
                3,
                b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>",
            ),
            (4, &content),
            (5, b"<< /A (text) /B [1 2.5 /N] /C null >>"),
            (6, &packed),
            (7, b"<< /Title (Base) >>"),
            (8, b"(unreachable)"),
        ],
        "/Root 1 0 R /Info 7 0 R /ID [<01> <02>]",
    )
}
fn limits() -> DocumentLimits {
    DocumentLimits::default()
}
fn app(doc: &Document<'_>) -> Vec<u8> {
    write_document(doc, DocumentWriteLimits::default(), WriteOptions::APP).unwrap()
}
const PLAINTEXT: WriteOptions = WriteOptions {
    encryption: Encryption::Plaintext,
    ..WriteOptions::APP
};
/// The app save with encryption dropped explicitly.
fn plain(doc: &Document<'_>) -> Vec<u8> {
    write_document(doc, DocumentWriteLimits::default(), PLAINTEXT).unwrap()
}
/// Every object, unreachable ones included, so restores are checked exactly.
fn everything(doc: &Document<'_>) -> Vec<u8> {
    let options = WriteOptions {
        strict: false,
        encryption: Encryption::Plaintext,
        ..WriteOptions::default()
    };
    write_document(doc, DocumentWriteLimits::default(), options).unwrap()
}
fn compact(value: &Object) -> String {
    let bytes = serialize_object_with_style(value, WriteLimits::default(), SerializeStyle::Compact)
        .unwrap();
    String::from_utf8(bytes).unwrap()
}
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}
fn dict(entries: &[(&str, Object)]) -> Dictionary {
    entries
        .iter()
        .map(|(k, v)| (k.as_bytes().to_vec(), v.clone()))
        .collect()
}

#[test]
fn owned_open_matches_borrowed_open() {
    let cases: [(&str, &[u8]); 6] = [
        ("hello.pdf", b""),
        ("form.pdf", b""),
        ("nav.pdf", b""),
        ("layers.pdf", b""),
        ("encrypted.pdf", b"pdf4all-user"),
        ("encrypted-aes256.pdf", b"pdf4all-user"),
    ];
    for (name, password) in cases {
        let bytes = fixture(name);
        let borrowed = Document::open_with_password(&bytes, limits(), password).unwrap();
        let owned = Document::open_owned_with_password(bytes.clone(), limits(), password).unwrap();
        assert_eq!(owned.input(), bytes.as_slice());
        assert_eq!(owned.root(), borrowed.root());
        assert_eq!(owned.version(), borrowed.version());
        assert_eq!(owned.trailer(), borrowed.trailer());
        assert_eq!(owned.object_count(), borrowed.object_count());
        for object in borrowed.objects() {
            assert_eq!(owned.object(object.id).unwrap().value, object.value);
            assert_eq!(owned.object(object.id).unwrap().stream, object.stream);
            assert_eq!(
                owned.stream_data(object.id).unwrap(),
                borrowed.stream_data(object.id).unwrap()
            );
        }
        assert_eq!(plain(&owned), plain(&borrowed), "{name}");
        // Clones share the input bytes instead of copying them.
        assert!(std::ptr::eq(owned.clone().input(), owned.input()));
        assert!(std::ptr::eq(borrowed.clone().input(), bytes.as_slice()));
        if password.is_empty() {
            let limits = DocumentWriteLimits::default();
            assert_eq!(
                rewrite_document(&owned, limits).unwrap(),
                rewrite_document(&borrowed, limits).unwrap()
            );
        }
    }
    let owned = Document::open_owned(fixture("hello.pdf"), limits()).unwrap();
    let moved: Document<'static> = owned;
    assert_eq!(moved.version(), "1.4");
}

#[test]
fn object_constructors_and_accessors() {
    assert_eq!(Object::name("Type"), Object::Name(b"Type".to_vec()));
    assert_eq!(Object::name(b"A#B"), Object::Name(b"A#B".to_vec()));
    assert_eq!(Object::string(b"\x00\xff"), Object::String(vec![0, 255]));
    assert_eq!(Object::text("caf"), Object::String(b"caf".to_vec()));
    assert_eq!(
        Object::text("caf\u{e9}"),
        Object::String(b"\xFE\xFF\x00c\x00a\x00f\x00\xe9".to_vec())
    );
    assert_eq!(Object::reference(id(3)), Object::Reference(id(3)));
    assert_eq!(Object::integer(-7), Object::Integer(-7));
    assert_eq!(Object::real(0.25), Some(Object::Real(0.25)));
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(Object::real(bad), None);
    }
    let array = Object::array([Object::integer(1), Object::Null]);
    assert_eq!(array.as_array().unwrap().len(), 2);
    let mut value = Object::dictionary([("Type", Object::name("Annot")), ("Rect", array.clone())]);
    assert_eq!(
        value.as_dict().unwrap().get(b"Type".as_slice()),
        Some(&Object::name("Annot"))
    );
    value
        .as_dict_mut()
        .unwrap()
        .insert(b"F".to_vec(), Object::integer(4));
    assert_eq!(value.as_dict().unwrap().len(), 3);
    let mut list = array.clone();
    list.as_array_mut().unwrap().push(Object::Boolean(true));
    assert_eq!(list.as_array().unwrap()[2].as_bool(), Some(true));

    assert_eq!(Object::name("N").as_name(), Some(b"N".as_slice()));
    assert_eq!(Object::string("s").as_string(), Some(b"s".as_slice()));
    assert_eq!(Object::name("N").as_string(), None);
    assert_eq!(Object::Integer(5).as_i64(), Some(5));
    assert_eq!(Object::Real(5.0).as_i64(), None);
    assert_eq!(Object::Integer(5).as_f64(), Some(5.0));
    assert_eq!(Object::Real(-1.5).as_f64(), Some(-1.5));
    assert_eq!(Object::Null.as_f64(), None);
    assert_eq!(Object::Reference(id(9)).as_reference(), Some(id(9)));
    assert_eq!(Object::Integer(9).as_reference(), None);
    assert!(Object::Null.is_null());
    assert!(!Object::Boolean(false).is_null());
    assert_eq!(Object::Integer(1).as_dict(), None);
    assert_eq!(Object::Integer(1).as_array(), None);
}

fn utf16(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xFE, 0xFF];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    bytes
}

#[test]
fn text_strings_are_encoded_as_specified() {
    // Spec 01 section 3.6: ASCII as bytes, anything else UTF-16BE.
    assert_eq!(encode_text_string(""), b"");
    assert_eq!(encode_text_string("Signature1"), b"Signature1");
    assert_eq!(encode_text_string("tab\tline\nret\r"), b"tab\tline\nret\r");
    assert_eq!(encode_text_string("nul\u{0}del\u{7F}"), b"nul\x00del\x7F");
    assert_eq!(
        encode_text_string("caf\u{e9}"),
        [0xFE, 0xFF, 0, b'c', 0, b'a', 0, b'f', 0, 0xE9]
    );
    assert_eq!(
        encode_text_string("\u{2022} bullet")[..4],
        [0xFE, 0xFF, 0x20, 0x22]
    );
    assert_eq!(
        encode_text_string("\u{20AC}5"),
        [0xFE, 0xFF, 0x20, 0xAC, 0, b'5']
    );
    assert_eq!(
        encode_text_string("\u{1F600}"),
        [0xFE, 0xFF, 0xD8, 0x3D, 0xDE, 0x00]
    );
    for text in [
        "\u{4E2D}\u{6587}",
        "a\u{A0}b",
        "soft\u{AD}hyphen",
        "\u{80}",
        "\u{2022} and \u{4E2D}",
        // PDFDocEncoding would read these bytes back as accents.
        "\u{18}",
        "x\u{1F}",
        // PDFDocEncoding bytes that would look like byte order marks.
        "\u{FE}\u{FF}ab",
        "\u{FF}\u{FE}ab",
        "\u{EF}\u{BB}\u{BF}x",
        "\u{FE}\u{FF}",
    ] {
        assert_eq!(encode_text_string(text), utf16(text), "{text:?}");
    }
    // Every encoding decodes to the text it came from.
    let mut samples: Vec<String> = (0u32..0x3000)
        .chain([0xFB01, 0xFEFF, 0xFFFD, 0x1F600, 0x10FFFF])
        .filter_map(char::from_u32)
        .map(|c| format!("{c}"))
        .collect();
    samples.extend(
        [
            "",
            "Plain ASCII (with) \\ escapes",
            "\u{FE}\u{FF}ab",
            "\u{FF}\u{FE}ab",
            "\u{EF}\u{BB}\u{BF}x",
            "\u{FE}\u{FF}",
            "\u{4E2D}\u{1F600}\u{A0}",
            "\u{18}\u{19}\u{1A}\u{1B}\u{1C}\u{1D}\u{1E}\u{1F}",
        ]
        .map(String::from),
    );
    for text in samples {
        assert_eq!(
            decode_text_string(&encode_text_string(&text)),
            text,
            "{text:?}"
        );
    }
}

#[test]
fn text_string_decoding_follows_byte_order_marks_and_annex_d() {
    assert_eq!(decode_text_string(b"\xFE\xFF\x00A\x4E\x2D"), "A\u{4E2D}");
    assert_eq!(decode_text_string(b"\xFF\xFEA\x00\x2D\x4E"), "A\u{4E2D}");
    assert_eq!(decode_text_string(b"\xEF\xBB\xBFcaf\xC3\xA9"), "caf\u{e9}");
    // An odd trailing byte is dropped; lone surrogates become U+FFFD.
    assert_eq!(decode_text_string(b"\xFE\xFF\x00A\x00"), "A");
    assert_eq!(decode_text_string(b"\xFE\xFF\xD8\x00\x00A"), "\u{FFFD}A");
    assert_eq!(decode_text_string(b"\xEF\xBB\xBF\xFF"), "\u{FFFD}");
    // PDFDocEncoding (ISO 32000-2 Annex D, Table D.2): the codes whose
    // Unicode value differs from the code; every other code reads as its
    // Latin-1 value, the undefined ones included.
    let special: [(u8, char); 40] = [
        (0x18, '\u{2D8}'),
        (0x19, '\u{2C7}'),
        (0x1A, '\u{2C6}'),
        (0x1B, '\u{2D9}'),
        (0x1C, '\u{2DD}'),
        (0x1D, '\u{2DB}'),
        (0x1E, '\u{2DA}'),
        (0x1F, '\u{2DC}'),
        (0x80, '\u{2022}'),
        (0x81, '\u{2020}'),
        (0x82, '\u{2021}'),
        (0x83, '\u{2026}'),
        (0x84, '\u{2014}'),
        (0x85, '\u{2013}'),
        (0x86, '\u{192}'),
        (0x87, '\u{2044}'),
        (0x88, '\u{2039}'),
        (0x89, '\u{203A}'),
        (0x8A, '\u{2212}'),
        (0x8B, '\u{2030}'),
        (0x8C, '\u{201E}'),
        (0x8D, '\u{201C}'),
        (0x8E, '\u{201D}'),
        (0x8F, '\u{2018}'),
        (0x90, '\u{2019}'),
        (0x91, '\u{201A}'),
        (0x92, '\u{2122}'),
        (0x93, '\u{FB01}'),
        (0x94, '\u{FB02}'),
        (0x95, '\u{141}'),
        (0x96, '\u{152}'),
        (0x97, '\u{160}'),
        (0x98, '\u{178}'),
        (0x99, '\u{17D}'),
        (0x9A, '\u{131}'),
        (0x9B, '\u{142}'),
        (0x9C, '\u{153}'),
        (0x9D, '\u{161}'),
        (0x9E, '\u{17E}'),
        (0xA0, '\u{20AC}'),
    ];
    for byte in 0..=255u8 {
        let expected = special
            .iter()
            .find(|(code, _)| *code == byte)
            .map_or(char::from(byte), |(_, c)| *c);
        assert_eq!(
            decode_text_string(&[byte]),
            expected.to_string(),
            "{byte:#x}"
        );
    }
    assert_eq!(
        decode_text_string(b"\x80 \xA0 caf\xE9"),
        "\u{2022} \u{20AC} caf\u{e9}"
    );
}

#[test]
fn resolution_follows_chains_and_maps_missing_to_null() {
    let mut objects: Vec<(u32, Vec<u8>)> = vec![
        (
            1,
            b"<< /Type /Catalog /Pages 2 0 R /Next 10 0 R /Gone 99 0 R >>".to_vec(),
        ),
        (2, b"<< /Type /Pages /Kids [] /Count 0 >>".to_vec()),
        (3, b"4 0 R".to_vec()),
        (4, b"3 0 R".to_vec()),
        (5, stream("/Subtype /Form", b"q Q")),
    ];
    // Following 11, 12, ..., 74 (whose value is 42) takes 64 hops; from 10, 65.
    for n in 10..74 {
        objects.push((n, format!("{} 0 R", n + 1).into_bytes()));
    }
    objects.push((74, b"42".to_vec()));
    let refs: Vec<(u32, &[u8])> = objects.iter().map(|(n, v)| (*n, v.as_slice())).collect();
    let bytes = pdf(&refs, "/Root 1 0 R");
    let doc = Document::open(&bytes, limits()).unwrap();
    let catalog = doc.object(id(1)).unwrap().value.as_dict().unwrap();
    assert_eq!(
        doc.dict_get(catalog, b"Pages").as_dict().unwrap()[b"Count".as_slice()],
        Object::Integer(0)
    );
    assert!(doc.dict_get(catalog, b"Absent").is_null());
    assert!(doc.dict_get(catalog, b"Gone").is_null());
    // A cycle or a chain longer than 64 hops resolves to null.
    assert!(doc.resolve_id(id(3)).is_null());
    assert!(doc.resolve(&Object::Reference(id(4))).is_null());
    assert_eq!(doc.resolve_id(id(11)), &Object::Integer(42));
    assert_eq!(
        doc.resolve(&Object::Reference(id(11))),
        &Object::Integer(42)
    );
    assert!(doc.resolve_id(id(10)).is_null());
    assert!(doc.dict_get(catalog, b"Next").is_null());
    assert!(doc.resolve_id(id(99)).is_null());
    assert_eq!(doc.resolve(&Object::Integer(1)), &Object::Integer(1));
    // The holder is the object whose value the chain ends in.
    assert_eq!(
        doc.resolve_id_with_holder(id(11)),
        (Some(id(74)), &Object::Integer(42))
    );
    assert_eq!(
        doc.resolve_id_with_holder(id(74)),
        (Some(id(74)), &Object::Integer(42))
    );
    let pages = &catalog[b"Pages".as_slice()];
    assert_eq!(doc.resolve_with_holder(pages).0, Some(id(2)));
    assert_eq!(
        doc.resolve_with_holder(&Object::Integer(7)),
        (None, &Object::Integer(7))
    );
    assert_eq!(doc.resolve_id_with_holder(id(3)), (None, &Object::Null));
    assert_eq!(doc.resolve_id_with_holder(id(99)), (None, &Object::Null));
    let reference = Object::Reference(id(5));
    let form = doc.resolve_dict(&reference).unwrap();
    assert_eq!(form[b"Subtype".as_slice()], Object::name("Form"));
    assert_eq!(doc.stream_dict(id(5)).unwrap(), form);
    assert!(doc.stream_dict(id(2)).is_err());
    assert!(doc.stream_dict(id(99)).is_err());
    assert_eq!(doc.resolve_dict(&Object::Reference(id(11))), None);
    assert_eq!(
        doc.decoded_stream(id(5), StreamLimits::default()).unwrap(),
        b"q Q"
    );
}

#[test]
fn mutations_keep_or_strip_filters_as_asked() {
    let bytes = graph();
    let mut doc = Document::open(&bytes, limits()).unwrap();
    assert_eq!(doc.next_id().unwrap(), id(9));
    let raw = miniz_oxide::deflate::compress_to_vec_zlib(b"0 0 m 10 10 l S", 6);
    let filtered = dict(&[
        ("Filter", Object::name("FlateDecode")),
        ("Length", Object::integer(999)),
    ]);
    let grafted = doc
        .allocate_stream_raw(filtered.clone(), raw.clone())
        .unwrap();
    assert_eq!(grafted, id(9));
    assert_eq!(doc.next_id().unwrap(), id(10));
    assert_eq!(doc.stream_data(grafted).unwrap(), Some(raw.as_slice()));
    let stored = doc.stream_dict(grafted).unwrap();
    assert_eq!(stored[b"Filter".as_slice()], Object::name("FlateDecode"));
    assert_eq!(
        stored[b"Length".as_slice()],
        Object::integer(raw.len() as i64)
    );
    assert_eq!(
        doc.decoded_stream(grafted, StreamLimits::default())
            .unwrap(),
        b"0 0 m 10 10 l S"
    );

    let mut with_parms = filtered.clone();
    with_parms.insert(
        b"DecodeParms".to_vec(),
        Object::dictionary([("Predictor", Object::integer(12))]),
    );
    doc.put_stream_decoded(id(6), with_parms, b"plain".to_vec())
        .unwrap();
    let replaced = doc.stream_dict(id(6)).unwrap();
    assert!(!replaced.contains_key(b"Filter".as_slice()));
    assert!(!replaced.contains_key(b"DecodeParms".as_slice()));
    assert_eq!(replaced[b"Length".as_slice()], Object::integer(5));
    assert_eq!(
        doc.decoded_stream(id(6), StreamLimits::default()).unwrap(),
        b"plain"
    );

    // A dictionary swap keeps the bytes exactly, /Length included.
    let swapped = dict(&[("Subtype", Object::name("Form"))]);
    doc.set_stream_dict(id(4), swapped.clone()).unwrap();
    assert_eq!(doc.stream_dict(id(4)).unwrap(), &swapped);
    assert_eq!(
        doc.stream_data(id(4)).unwrap(),
        Some(b"BT /F1 12 Tf (Hi) Tj ET".as_slice())
    );
    assert!(doc.set_stream_dict(id(5), swapped.clone()).is_err());
    assert!(doc.set_stream_dict(id(50), swapped).is_err());

    // put_value creates, replaces, and turns a stream into a plain value.
    doc.put_value(id(20), Object::integer(1)).unwrap();
    doc.put_value(id(20), Object::integer(2)).unwrap();
    assert_eq!(doc.object(id(20)).unwrap().value, Object::integer(2));
    doc.put_value(id(6), Object::name("NotAStream")).unwrap();
    assert_eq!(doc.stream_data(id(6)).unwrap(), None);
    assert!(doc.stream_dict(id(6)).is_err());
    doc.put_stream_raw(id(21), filtered, raw.clone()).unwrap();
    assert_eq!(doc.stream_data(id(21)).unwrap(), Some(raw.as_slice()));

    // Another generation of a live number is refused.
    let other = ObjectId::new(20, 1).unwrap();
    assert!(doc.put_value(other, Object::Null).is_err());
    assert!(
        doc.put_stream_raw(other, Dictionary::new(), vec![])
            .is_err()
    );

    assert!(doc.remove_object(doc.root()).is_err());
    assert!(doc.remove_object(id(77)).is_err());
    doc.remove_object(id(20)).unwrap();
    assert!(doc.object(id(20)).is_err());
    assert!(doc.stream_data(id(20)).is_err());
    assert_eq!(doc.next_id().unwrap(), id(22));
    // The existing APIs keep their semantics.
    let allocated = doc
        .allocate(
            Object::Dictionary(dict(&[("Filter", Object::name("DCTDecode"))])),
            Some(b"x".to_vec()),
        )
        .unwrap();
    assert!(
        !doc.stream_dict(allocated)
            .unwrap()
            .contains_key(b"Filter".as_slice())
    );
    assert!(doc.replace_value(id(4), Object::Null).is_err());
}

#[test]
fn encrypted_stream_caches_follow_mutations() {
    let bytes = fixture("encrypted.pdf");
    let mut doc = Document::open_with_password(&bytes, limits(), b"pdf4all-user").unwrap();
    let page = pages::page_ids(&doc)[0];
    let content = pages::contents(&doc, page)[0];
    let decoded = doc
        .decoded_stream(content, StreamLimits::default())
        .unwrap();
    assert!(latin1(&decoded).contains("PDF4All encrypted"));
    let state = doc.snapshot(content).unwrap();

    doc.put_value(content, Object::integer(1)).unwrap();
    assert_eq!(doc.stream_data(content).unwrap(), None);
    doc.restore(content, Some(state.clone())).unwrap();
    assert_eq!(
        doc.decoded_stream(content, StreamLimits::default())
            .unwrap(),
        decoded
    );

    doc.remove_object(content).unwrap();
    doc.put_value(content, Object::integer(2)).unwrap();
    assert_eq!(doc.stream_data(content).unwrap(), None);
    doc.restore(content, None).unwrap();
    assert!(doc.object(content).is_err());
    doc.restore(content, Some(state)).unwrap();
    assert_eq!(
        doc.decoded_stream(content, StreamLimits::default())
            .unwrap(),
        decoded
    );
}

#[test]
fn trailer_entries_are_journaled_and_written() {
    let bytes = graph();
    let mut doc = Document::open(&bytes, limits()).unwrap();
    let original = doc.trailer_state();
    let before = app(&doc);
    assert_eq!(doc.trailer_entry(b"Info"), Some(&Object::Reference(id(7))));
    assert_eq!(doc.trailer_entry(b"Root"), Some(&Object::Reference(id(1))));
    assert_eq!(doc.trailer_entry(b"Encrypt"), None);

    let info = doc
        .allocate(Object::dictionary([("Title", Object::text("New"))]), None)
        .unwrap();
    doc.set_trailer_entry(b"Info", Some(Object::Reference(info)))
        .unwrap();
    doc.set_trailer_entry(
        b"ID",
        Some(Object::array([
            Object::string(b"\x01"),
            Object::string(b"\x03"),
        ])),
    )
    .unwrap();
    assert_eq!(doc.trailer_entry(b"Info"), Some(&Object::Reference(info)));
    let text = latin1(&app(&doc));
    assert!(
        text.contains(&format!(
            "trailer\n<</Size {}/Info {} 0 R/Root 1 0 R/ID[<01><03>]>>",
            info.number() + 1,
            info.number()
        )),
        "{text}"
    );
    assert!(text.contains("(New)"));

    doc.set_trailer_entry(b"Info", None).unwrap();
    doc.set_trailer_entry(b"ID", Some(Object::Null)).unwrap();
    assert_eq!(doc.trailer_entry(b"Info"), None);
    assert_eq!(doc.trailer_entry(b"ID"), None);
    let text = latin1(&app(&doc));
    assert!(text.contains("trailer\n<</Size 7/Root 1 0 R>>"), "{text}");
    // The raw parsed trailer is unchanged.
    assert_eq!(doc.trailer()[b"Info".as_slice()], Object::Reference(id(7)));

    assert!(
        doc.set_trailer_entry(b"Root", Some(Object::integer(1)))
            .is_err()
    );
    assert!(doc.set_trailer_entry(b"Root", None).is_err());
    // Root must name an existing dictionary that is not a stream.
    for bad in [99, 4, 8] {
        assert!(
            doc.set_trailer_entry(b"Root", Some(Object::Reference(id(bad))))
                .is_err(),
            "{bad}"
        );
    }
    assert_eq!(doc.root(), id(1));
    assert!(
        doc.set_trailer_entry(b"Size", Some(Object::integer(3)))
            .is_err()
    );
    assert!(doc.set_trailer_entry(b"Prev", None).is_err());

    // Encrypt is carried for later. Strict writing refuses it; app writing
    // refuses it unless told to write plaintext, and never writes it yet.
    doc.set_trailer_entry(b"Encrypt", Some(Object::Reference(id(5))))
        .unwrap();
    assert!(matches!(
        rewrite_document(&doc, DocumentWriteLimits::default()),
        Err(DocumentWriteError::Unsupported(
            "unknown non-null trailer entry"
        ))
    ));
    assert!(matches!(
        write_document(&doc, DocumentWriteLimits::default(), WriteOptions::APP),
        Err(DocumentWriteError::Unsupported("encrypted document"))
    ));
    assert!(!latin1(&plain(&doc)).contains("Encrypt"));
    doc.set_trailer_entry(b"Encrypt", None).unwrap();

    let catalog = doc
        .allocate(
            Object::dictionary([
                ("Type", Object::name("Catalog")),
                ("Pages", Object::Reference(id(2))),
            ]),
            None,
        )
        .unwrap();
    doc.set_trailer_entry(b"Root", Some(Object::Reference(catalog)))
        .unwrap();
    assert_eq!(doc.root(), catalog);

    doc.restore_trailer(original.clone()).unwrap();
    doc.restore(info, None).unwrap();
    doc.restore(catalog, None).unwrap();
    assert_eq!(doc.root(), id(1));
    assert_eq!(doc.trailer_state(), original);
    assert_eq!(app(&doc), before);
}

/// Deterministic generator for the journal property test.
struct Rng(u64);
impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n
    }
    fn value(&mut self, depth: usize, ids: &[ObjectId]) -> Object {
        match self.below(if depth > 2 { 7 } else { 9 }) {
            0 => Object::Null,
            1 => Object::Boolean(self.below(2) == 1),
            2 => Object::Integer(self.below(2000) as i64 - 1000),
            3 => Object::Real((self.below(20000) as f64 - 10000.0) / 64.0),
            4 => Object::Name(format!("N{}", self.below(50)).into_bytes()),
            5 => Object::String((0..self.below(6)).map(|_| self.below(256) as u8).collect()),
            6 => Object::Reference(ids[self.below(ids.len() as u64) as usize]),
            7 => Object::Array(
                (0..self.below(4))
                    .map(|_| self.value(depth + 1, ids))
                    .collect(),
            ),
            _ => Object::Dictionary(
                (0..self.below(4))
                    .map(|_| {
                        let key = format!("K{}", self.below(8)).into_bytes();
                        (key, self.value(depth + 1, ids))
                    })
                    .collect(),
            ),
        }
    }
    fn dictionary(&mut self, ids: &[ObjectId]) -> Dictionary {
        match self.value(3, ids) {
            Object::Dictionary(d) => d,
            other => [(b"V".to_vec(), other)].into_iter().collect(),
        }
    }
}

/// One change set: the state of every touched object before its first touch.
struct ChangeSet {
    before: BTreeMap<ObjectId, Option<ObjectState>>,
    trailer: TrailerState,
    after: BTreeMap<ObjectId, Option<ObjectState>>,
    after_trailer: TrailerState,
    bytes_before: (Vec<u8>, Vec<u8>),
    bytes_after: (Vec<u8>, Vec<u8>),
}

fn mutate(
    doc: &mut Document<'_>,
    rng: &mut Rng,
    set: &mut BTreeMap<ObjectId, Option<ObjectState>>,
) {
    let ids: Vec<ObjectId> = doc.objects().map(|o| o.id).collect();
    let streams: Vec<ObjectId> = ids
        .iter()
        .copied()
        .filter(|id| doc.stream_dict(*id).is_ok())
        .collect();
    let pick = |rng: &mut Rng, from: &[ObjectId]| from[rng.below(from.len() as u64) as usize];
    let mut touch = |doc: &Document<'_>, id: ObjectId| {
        set.entry(id).or_insert_with(|| doc.snapshot(id));
    };
    let fresh = doc.next_id().unwrap();
    match rng.below(9) {
        0 => {
            let target = pick(rng, &ids);
            touch(doc, target);
            let value = rng.value(0, &ids);
            doc.put_value(target, value).unwrap();
        }
        1 => {
            touch(doc, fresh);
            let value = rng.value(0, &ids);
            doc.put_value(fresh, value).unwrap();
        }
        2 => {
            let target = if rng.below(2) == 0 {
                fresh
            } else {
                pick(rng, &ids)
            };
            touch(doc, target);
            let mut dictionary = rng.dictionary(&ids);
            dictionary.insert(b"Filter".to_vec(), Object::name("DCTDecode"));
            let raw = (0..rng.below(40)).map(|_| rng.below(256) as u8).collect();
            doc.put_stream_raw(target, dictionary, raw).unwrap();
        }
        3 => {
            let target = if rng.below(2) == 0 {
                fresh
            } else {
                pick(rng, &ids)
            };
            touch(doc, target);
            let dictionary = rng.dictionary(&ids);
            let decoded = (0..rng.below(40)).map(|_| rng.below(256) as u8).collect();
            doc.put_stream_decoded(target, dictionary, decoded).unwrap();
        }
        4 if !streams.is_empty() => {
            let target = pick(rng, &streams);
            touch(doc, target);
            let dictionary = rng.dictionary(&ids);
            doc.set_stream_dict(target, dictionary).unwrap();
        }
        5 => {
            let target = pick(rng, &ids);
            if target != doc.root() {
                touch(doc, target);
                doc.remove_object(target).unwrap();
            }
        }
        6 => {
            touch(doc, fresh);
            let dictionary = rng.dictionary(&ids);
            let raw = (0..rng.below(40)).map(|_| rng.below(256) as u8).collect();
            assert_eq!(doc.allocate_stream_raw(dictionary, raw).unwrap(), fresh);
        }
        7 => {
            let value = (rng.below(3) > 0).then(|| Object::Reference(pick(rng, &ids)));
            doc.set_trailer_entry(b"Info", value).unwrap();
        }
        _ => {
            // Keep new objects reachable from the catalog.
            let root = doc.root();
            touch(doc, root);
            let mut catalog = doc.object(root).unwrap().value.clone();
            let list = Object::array(ids.iter().rev().take(6).map(|id| Object::Reference(*id)));
            if let Some(d) = catalog.as_dict_mut() {
                d.insert(b"Extra".to_vec(), list);
            }
            doc.put_value(root, catalog).unwrap();
        }
    }
}

fn outputs(doc: &Document<'_>) -> (Vec<u8>, Vec<u8>) {
    (plain(doc), everything(doc))
}

fn journal_round_trip(mut doc: Document<'_>, seed: u64) {
    let mut rng = Rng(seed);
    let start = outputs(&doc);
    let mut done = Vec::new();
    for _ in 0..12 {
        let trailer = doc.trailer_state();
        let bytes_before = outputs(&doc);
        let mut before = BTreeMap::new();
        for _ in 0..1 + rng.below(5) {
            mutate(&mut doc, &mut rng, &mut before);
        }
        let after = before.keys().map(|id| (*id, doc.snapshot(*id))).collect();
        done.push(ChangeSet {
            before,
            trailer,
            after,
            after_trailer: doc.trailer_state(),
            bytes_before,
            bytes_after: outputs(&doc),
        });
    }
    // Undo every change set, newest first, then redo them all.
    for set in done.iter().rev() {
        assert_eq!(outputs(&doc), set.bytes_after);
        for (id, state) in &set.before {
            doc.restore(*id, state.clone()).unwrap();
        }
        doc.restore_trailer(set.trailer.clone()).unwrap();
        assert_eq!(outputs(&doc), set.bytes_before);
    }
    assert_eq!(outputs(&doc), start);
    for set in &done {
        for (id, state) in &set.after {
            doc.restore(*id, state.clone()).unwrap();
        }
        doc.restore_trailer(set.after_trailer.clone()).unwrap();
        assert_eq!(outputs(&doc), set.bytes_after);
    }
}

#[test]
fn restoring_snapshots_reproduces_identical_bytes() {
    let bytes = graph();
    for seed in 1..=12 {
        journal_round_trip(Document::open(&bytes, limits()).unwrap(), seed);
        journal_round_trip(
            Document::open_owned(bytes.clone(), limits()).unwrap(),
            seed + 100,
        );
    }
    let encrypted = fixture("encrypted-aes256.pdf");
    for seed in 1..=4 {
        let doc = Document::open_with_password(&encrypted, limits(), b"pdf4all-user").unwrap();
        journal_round_trip(doc, seed + 200);
    }
}

#[test]
fn compact_style_writes_the_specified_syntax() {
    // Token syntax only: these tokens in this order.
    let widget = Object::array([
        Object::name("FT"),
        Object::name("Sig"),
        Object::name("T"),
        Object::text("Signature1"),
        Object::name("Rect"),
        Object::array([0, 0, 0, 0].map(Object::integer)),
    ]);
    assert_eq!(compact(&widget), "[/FT/Sig/T(Signature1)/Rect[0 0 0 0]]");
    // /Type, then /Subtype (only after /Type), then the other keys in
    // insertion order (spec 01 section 3.7.3).
    let field = Object::dictionary([
        ("FT", Object::name("Sig")),
        ("T", Object::text("Signature1")),
        ("F", Object::integer(4)),
        ("P", Object::Reference(id(4))),
        ("Rect", Object::array([0, 0, 0, 0].map(Object::integer))),
        ("Subtype", Object::name("Widget")),
        ("Type", Object::name("Annot")),
    ]);
    assert_eq!(
        compact(&field),
        "<</Type/Annot/Subtype/Widget/FT/Sig/T(Signature1)/F 4/P 4 0 R/Rect[0 0 0 0]>>"
    );
    let untyped = Object::dictionary([
        ("Z", Object::integer(1)),
        ("Subtype", Object::integer(1)),
        ("A", Object::integer(1)),
    ]);
    assert_eq!(compact(&untyped), "<</Z 1/Subtype 1/A 1>>");
    let nested = Object::array([Object::dictionary([
        ("Subtype", Object::name("S")),
        ("Type", Object::Null),
        ("A", Object::dictionary([("Type", Object::name("T"))])),
    ])]);
    assert_eq!(compact(&nested), "[<</Subtype/S/Type null/A<</Type/T>>>>]");
    let matrix = Object::dictionary([(
        "Matrix",
        Object::array([0.0, 1.0, -1.0, 0.0, 0.0, -0.0].map(Object::Real)),
    )]);
    assert_eq!(compact(&matrix), "<</Matrix[0 1 -1 0 0 0]>>");
    let signature = Object::dictionary([
        ("Filter", Object::name("Adobe.PPKLite")),
        ("SubFilter", Object::name("adbe.pkcs7.detached")),
        ("Type", Object::name("Sig")),
    ]);
    assert_eq!(
        compact(&signature),
        "<</Type/Sig/Filter/Adobe.PPKLite/SubFilter/adbe.pkcs7.detached>>"
    );
    let mixed = Object::array([
        Object::array([Object::integer(1), Object::name("X")]),
        Object::string("q"),
        Object::string("q"),
        Object::dictionary([
            ("A", Object::Null),
            ("B", Object::Dictionary(Dictionary::new())),
        ]),
        Object::Reference(id(1)),
        Object::integer(1),
        Object::Boolean(true),
        Object::Boolean(false),
        Object::Null,
        Object::Array(vec![]),
    ]);
    assert_eq!(
        compact(&mixed),
        "[[1/X](q)(q)<</A null/B<<>>>>1 0 R 1 true false null[]]"
    );
    let strings: [(&[u8], &str); 16] = [
        (b"", "()"),
        (b"abc", "(abc)"),
        (b"a(b)c\\d", "(a\\(b\\)c\\\\d)"),
        (b"line\nbreak\r\t\x08\x0c", "(line\\nbreak\\r\\t\\b\\f)"),
        (b"\x00\x01ab", "<00016162>"),
        (b"\x80\x81\x82abc", "<808182616263>"),
        (b"\xfe\xff\x00A\x00B", "<FEFF00410042>"),
        (b"ab\x00", "(ab\\000)"),
        (b"(", "(\\()"),
        (b"abcd\xff", "(abcd\\377)"),
        (b")))))", "(\\)\\)\\)\\)\\))"),
        (b"a\x7fb", "(a\\177b)"),
        // A UTF-16 byte order mark always means hex; a UTF-8 one does not.
        (b"\xfe\xffN-N-N-N-N-", "<FEFF4E2D4E2D4E2D4E2D4E2D>"),
        (b"\xff\xfeABCDEFGH", "<FFFE4142434445464748>"),
        (b"\xef\xbb\xbfABCDEFGHI", "(\\357\\273\\277ABCDEFGHI)"),
        (b"\xfeabcd", "(\\376abcd)"),
    ];
    for (value, expected) in strings {
        assert_eq!(compact(&Object::string(value)), expected);
    }
    assert_eq!(
        compact(&Object::array(
            ["A B", "a#b", "x/y", "\u{e9}", "Type"].map(Object::name)
        )),
        "[/A#20B/a#23b/x#2Fy/#C3#A9/Type]"
    );
    // Text strings (spec 01 section 3.6) in the compact style.
    assert_eq!(
        compact(&Object::text("caf\u{e9}")),
        "<FEFF00630061006600E9>"
    );
    assert_eq!(
        compact(&Object::text("\u{4E2D}\u{4E2D}\u{4E2D}\u{4E2D}")),
        "<FEFF4E2D4E2D4E2D4E2D>"
    );
    assert_eq!(compact(&Object::text("\u{4E2D}")), "<FEFF4E2D>");
    let error = serialize_object_with_style(
        &Object::Real(f64::NAN),
        WriteLimits::default(),
        SerializeStyle::Compact,
    );
    assert!(error.is_err());
    // The canonical style stays the default.
    assert_eq!(
        serialize_object_with_style(&widget, WriteLimits::default(), SerializeStyle::default())
            .unwrap(),
        b"[/FT /Sig /T <5369676E617475726531> /Rect [0 0 0 0]]"
    );
}

/// Compact reals (spec 01 section 3.7.5): the shortest decimal that reads
/// back as the same binary64, positional, without a `0` before the point;
/// integral values below 2^63 print as integers.
#[test]
fn compact_reals_are_the_shortest_exact_decimal() {
    const TWO_63: f64 = 9_223_372_036_854_775_808_u64 as f64;
    let matrix = Object::dictionary([(
        "Matrix",
        Object::array([0.0, 1.0, -1.0, 0.0, 0.0, -0.0].map(Object::Real)),
    )]);
    assert_eq!(compact(&matrix), "<</Matrix[0 1 -1 0 0 0]>>");
    let reals: [(f64, &str); 18] = [
        (0.5, ".5"),
        (-0.5, "-.5"),
        (0.1, ".1"),
        (612.0, "612"),
        (-0.0, "0"),
        (1e-7, ".0000001"),
        (123456.789, "123456.789"),
        (72.12345678, "72.12345678"),
        (1.5e10, "15000000000"),
        (1.0 / 3.0, ".3333333333333333"),
        (std::f64::consts::PI, "3.141592653589793"),
        (0.30000001192092896, ".30000001192092896"),
        (2.675, "2.675"),
        (1e-40, ".0000000000000000000000000000000000000001"),
        (-2147483648.0, "-2147483648"),
        (16777217.0, "16777217"),
        // Integral at 2^63 and beyond: a trailing point keeps them reals.
        (1e20, "100000000000000000000."),
        (-3e19, "-30000000000000000000."),
    ];
    for (value, expected) in reals {
        assert_eq!(compact(&Object::Real(value)), expected, "{value}");
    }
    let mut values = vec![
        0.5,
        1.0 / 3.0,
        2.0 / 3.0,
        0.000123,
        100.25,
        4600095744.0,
        9.999999,
        223459952.0,
        -2044019712.0,
        0.8,
        3.4e38,
        1e300,
        5e-324,
        -f64::MAX,
        f64::MIN_POSITIVE,
        8388608.5,
        123456789012.0,
        1234.5678,
        0.333333333,
        TWO_63,
        -TWO_63,
    ];
    values.extend((0..64).map(|i| f64::from_bits(0x3FF0_0000_0000_0001 + i * 0x0123_4567_89AB)));
    for value in values {
        let text = compact(&Object::Real(value));
        assert!(!text.contains(['e', 'E']), "{value}: {text}");
        assert!(
            !text.starts_with("0.") && !text.starts_with("-0."),
            "{value}: {text}"
        );
        let read = parse_complete(text.as_bytes(), ParseLimits::default()).unwrap();
        let integral = value.fract() == 0.0 && value.abs() < TWO_63 || value == -TWO_63;
        if integral {
            assert_eq!(read, Object::Integer(value as i64), "{value}: {text}");
        } else {
            assert_eq!(
                read.as_f64().map(f64::to_bits),
                Some(value.to_bits()),
                "{value}: {text}"
            );
        }
    }
}

#[test]
fn app_save_of_hello_writes_compact_object_syntax() {
    let bytes = fixture("hello.pdf");
    let doc = Document::open(&bytes, limits()).unwrap();
    let text = latin1(&app(&doc));
    // Spec 01 section 3.7 (integers only, so no real formatting is involved).
    for expected in [
        "1 0 obj\n<</Type/Catalog/Pages 2 0 R>>\nendobj\n",
        "2 0 obj\n<</Type/Pages/Kids[3 0 R]/Count 1>>\nendobj\n",
        "3 0 obj\n<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]/Resources<</Font<</F1 4 0 R>>>>/Contents 5 0 R>>\nendobj\n",
        "4 0 obj\n<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>\nendobj\n",
        "5 0 obj\n<</Length 39>>\nstream\nBT /F1 36 Tf 72 700 Td (PDF4All) Tj ET\n\nendstream\nendobj\n",
        "trailer\n<</Size 6/Root 1 0 R>>\nstartxref\n",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
    let bytes = app(&doc);
    let reopened = Document::open(&bytes, limits()).unwrap();
    assert_eq!(reopened.object_count(), 5);
    assert_eq!(app(&reopened), bytes);
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xCBF2_9CE4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3)
    })
}

/// Output length and FNV-1a digest of `rewrite_document` (equal to
/// `rewrite_reachable`'s for these files) as built at commit 817588c, before
/// `write_document` existed.
const REWRITES: [(&str, usize, u64); 33] = [
    ("cjk.pdf", 6293, 0xAEF6CD48455A5F14),
    ("edit-tj.pdf", 1967, 0xCD24F01093BCCC28),
    ("form.pdf", 964, 0x091DDA467CDDEBEB),
    ("hello.pdf", 620, 0xD6135F5FFC6F88C7),
    ("large.pdf", 478566, 0xEDE33CC4B5E13F47),
    ("layers.pdf", 957, 0x0BDCB622B30E608C),
    ("nav.pdf", 2442, 0x39D96DDA10F2C2D9),
    ("p0b/edit-coded.pdf", 3999, 0x2F7CE476CAD6A7DA),
    ("p0b/edit-embedded.pdf", 1667201, 0xC1DEF93493A18590),
    ("p0b/edit-shared.pdf", 1619, 0xAE71F9E37D799EB1),
    ("p0b/edit-subset.pdf", 6207, 0x168DFBE007DED6FE),
    ("p0b/edit-tagged.pdf", 1238, 0x6BF6B7130E3690FF),
    ("p0b/edit-transformed.pdf", 1125, 0xDC01C22CD7244F1D),
    ("p3/edit-objects.pdf", 2996, 0x31F9BD366D1B5637),
    ("p3/edit-replace.pdf", 1154, 0x09827695283BD647),
    ("p3/edit-subset2.pdf", 8995, 0x8FA0A2C65188C3B7),
    ("p3/edit-tail.pdf", 1029, 0x0948543823036531),
    ("p3/field-defs.pdf", 1455, 0xA1793682C1AC0717),
    ("p3/form-design.pdf", 1591, 0xCCB7601CC8175DAE),
    ("p3/simple-subset.pdf", 2647, 0x590794DA7B2B0CD7),
    ("p3/targets-limits.pdf", 2694, 0x1D85AB0AB2A5C186),
    ("p3/targets-streams.pdf", 2022, 0xAE3F3782CE83523C),
    ("p3/targets-syntax.pdf", 953, 0x23E13DB0F02904E4),
    ("p3/targets-unsupported.pdf", 800, 0x619FE4794F736C60),
    ("p6/direct3.pdf", 2208, 0x0BF386305B9F0AE7),
    ("p6/links.pdf", 2118, 0x0F5583E033D31530),
    ("p6/objects6.pdf", 2385, 0xA2DF7C3347308148),
    ("p6/paragraph.pdf", 1830, 0xD04855E4F925D832),
    ("p6/quartz.pdf", 19220, 0x40CF53A15D683A33),
    ("p6/tabs.pdf", 5057, 0x6F5CCBD2E6EF9A06),
    ("redact-shared-only.pdf", 1204, 0xC96907D72A327D4E),
    ("rotated.pdf", 787, 0xCF815281AF36E00C),
    ("rtl.pdf", 21099, 0xF85C36C031B4E098),
];
/// `rewrite_document`'s and `rewrite_reachable`'s refusals at 817588c.
const REFUSALS: [(&str, &str); 16] = [
    ("form2.pdf", "signature dictionary or signature form field"),
    (
        "p0b/certified-p2-annot.pdf",
        "signature or permissions metadata",
    ),
    (
        "p0b/certified-p2-filled.pdf",
        "signature or permissions metadata",
    ),
    ("p0b/certified-p2.pdf", "signature or permissions metadata"),
    (
        "p0b/signed-intact.pdf",
        "signature dictionary or signature form field",
    ),
    (
        "p0b/signed-tampered.pdf",
        "signature dictionary or signature form field",
    ),
    (
        "p0b/signed-untrusted.pdf",
        "signature dictionary or signature form field",
    ),
    (
        "p3/form-xfa.pdf",
        "signature dictionary or signature form field",
    ),
    ("p5/certified-p1.pdf", "signature or permissions metadata"),
    (
        "p5/empty-sigfield.pdf",
        "signature dictionary or signature form field",
    ),
    (
        "p5/signed-ec.pdf",
        "signature dictionary or signature form field",
    ),
    (
        "p5/signed-expired.pdf",
        "signature dictionary or signature form field",
    ),
    (
        "p5/signed-false-ca.pdf",
        "signature dictionary or signature form field",
    ),
    (
        "p5/signed-sha1-notime.pdf",
        "signature dictionary or signature form field",
    ),
    (
        "p5/signed-sha1.pdf",
        "signature dictionary or signature form field",
    ),
    ("redact.pdf", "signature dictionary or signature form field"),
];

#[test]
fn default_options_reproduce_the_existing_rewrites() {
    let limits_w = DocumentWriteLimits::default();
    let digest = |result: Result<Vec<u8>, DocumentWriteError>| {
        result.map(|bytes| (bytes.len(), fnv(&bytes)))
    };
    let garbage = WriteOptions {
        garbage: true,
        ..WriteOptions::default()
    };
    for (name, length, hash) in REWRITES {
        let bytes = fixture(name);
        let doc = Document::open(&bytes, limits()).unwrap();
        for result in [
            rewrite_document(&doc, limits_w),
            rewrite_reachable(&doc, limits_w),
            write_document(&doc, limits_w, WriteOptions::default()),
            write_document(&doc, limits_w, garbage),
        ] {
            assert_eq!(digest(result).unwrap(), (length, hash), "{name}");
        }
        let saved = app(&doc);
        let reopened = Document::open(&saved, limits()).unwrap();
        assert_eq!(
            pages::page_ids(&reopened).len(),
            pages::page_ids(&doc).len()
        );
    }
    for (name, reason) in REFUSALS {
        let bytes = fixture(name);
        let doc = Document::open(&bytes, limits()).unwrap();
        for result in [
            rewrite_document(&doc, limits_w),
            rewrite_reachable(&doc, limits_w),
        ] {
            assert!(
                matches!(result, Err(DocumentWriteError::Unsupported(r)) if r == reason),
                "{name}"
            );
        }
    }
    // The orphan in `graph` and an edit made with the older API, where the
    // two rewrites differ.
    let bytes = graph();
    let mut doc = Document::open(&bytes, limits()).unwrap();
    assert_eq!(
        digest(rewrite_document(&doc, limits_w)).unwrap(),
        (804, 0xDB8A_7ED6_E8EF_A334)
    );
    assert_eq!(
        digest(rewrite_reachable(&doc, limits_w)).unwrap(),
        (744, 0x46A1_F24F_C141_BA15)
    );
    let form = dict(&[
        ("Filter", Object::name("FlateDecode")),
        ("Subtype", Object::name("Form")),
    ]);
    doc.allocate(Object::Dictionary(form), Some(b"0 0 m 1 1 l S".to_vec()))
        .unwrap();
    let value = dict(&[
        ("R", Object::Real(1.0 / 3.0)),
        ("S", Object::string(b"a(b)\xff")),
        ("N", Object::name("A B")),
    ]);
    doc.replace_value(id(5), Object::Dictionary(value)).unwrap();
    doc.delete_objects(&[id(8)].into_iter().collect()).unwrap();
    assert_eq!(
        digest(rewrite_document(&doc, limits_w)).unwrap(),
        (859, 0x8793_050C_AE94_6F85)
    );
    assert_eq!(
        digest(rewrite_reachable(&doc, limits_w)).unwrap(),
        (756, 0xB821_43D2_4E33_707B)
    );
}

#[test]
fn huge_reals_survive_a_save_and_reopen() {
    let bytes = pdf(
        &[
            (1, b"<< /Type /Catalog /Pages 2 0 R >>"),
            (
                2,
                b"<< /Type /Pages /Kids [] /Count 0 /X 100000000000000000000.0 /Y -30000000000000000000.0 >>",
            ),
        ],
        "/Root 1 0 R",
    );
    let mut doc = Document::open(&bytes, limits()).unwrap();
    let mut pages = doc.object(id(2)).unwrap().value.clone();
    let entries = pages.as_dict_mut().unwrap();
    for (key, value) in [("Z", 1e300), ("W", 9.3e18), ("V", 1.5e10), ("U", -2.5e9)] {
        entries.insert(key.as_bytes().to_vec(), Object::real(value).unwrap());
    }
    doc.put_value(id(2), pages).unwrap();
    let saved = app(&doc);
    let text = latin1(&saved);
    // Shortest round-trip binary64 reals, positional (spec 01 section 3.7.5, S2).
    let z = format!("1{}.", "0".repeat(300));
    let expected = format!(
        "/X 100000000000000000000./Y -30000000000000000000./Z {z}/W 9300000000000000000./V 15000000000/U -2500000000>>"
    );
    assert!(text.contains(&expected), "{text}");
    let reopened = Document::open(&saved, limits()).unwrap();
    let pages = reopened.object(id(2)).unwrap().value.as_dict().unwrap();
    assert_eq!(pages[b"X".as_slice()], Object::Real(1e20));
    assert_eq!(pages[b"Z".as_slice()], Object::Real(1e300));
    assert_eq!(pages[b"U".as_slice()], Object::Integer(-2_500_000_000));
    assert_eq!(app(&reopened), saved);
    // The canonical style keeps its explicit decimal point.
    let canonical = rewrite_document(&doc, DocumentWriteLimits::default()).unwrap();
    assert!(Document::open(&canonical, limits()).is_ok());
}

#[test]
fn every_write_needs_the_catalog() {
    let bytes = graph();
    let limits_w = DocumentWriteLimits::default();
    let mut doc = Document::open(&bytes, limits()).unwrap();
    let state = doc.snapshot(id(1));
    doc.restore(id(1), None).unwrap();
    let garbage = WriteOptions {
        garbage: true,
        ..WriteOptions::default()
    };
    for options in [WriteOptions::default(), garbage, WriteOptions::APP] {
        assert!(
            matches!(
                write_document(&doc, limits_w, options),
                Err(DocumentWriteError::MissingReference(missing)) if missing == id(1)
            ),
            "{options:?}"
        );
    }
    doc.restore(id(1), state).unwrap();
    assert!(rewrite_document(&doc, limits_w).is_ok());
    // A catalog set through the trailer must exist.
    assert!(
        doc.set_trailer_entry(b"Root", Some(Object::Reference(id(42))))
            .is_err()
    );
    assert!(rewrite_document(&doc, limits_w).is_ok());
}

/// A base revision with Info and ID, and an update whose trailer has
/// neither.
fn updated_without_info() -> Vec<u8> {
    let mut bytes = pdf(
        &[
            (1, b"<< /Type /Catalog /Pages 2 0 R >>"),
            (2, b"<< /Type /Pages /Kids [] /Count 0 >>"),
            (3, b"<< /Producer (Base) >>"),
        ],
        "/Root 1 0 R /Info 3 0 R /ID [<0A> <0B>]",
    );
    let base_xref = {
        let text = latin1(&bytes);
        let at = text.rfind("startxref\n").unwrap() + "startxref\n".len();
        text[at..].lines().next().unwrap().parse::<usize>().unwrap()
    };
    let offset = bytes.len();
    bytes
        .extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [] /Count 0 /Edited true >>\nendobj\n");
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!(
            "xref\n0 1\n0000000000 65535 f \n2 1\n{offset:010} 00000 n \ntrailer\n<< /Size 4 /Root 1 0 R /Prev {base_xref} >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    bytes
}

#[test]
fn info_and_id_come_from_the_newest_trailer() {
    let bytes = updated_without_info();
    let mut doc = Document::open(&bytes, limits()).unwrap();
    assert_eq!(doc.trailer_entry(b"Info"), None);
    assert_eq!(doc.trailer_entry(b"ID"), None);
    assert_eq!(doc.trailer_entry(b"Root"), Some(&Object::Reference(id(1))));
    // Other viewers read only the newest trailer, as ISO 32000 7.5.6
    // requires it to be complete; the app save drops the older Info.
    let text = latin1(&app(&doc));
    assert!(text.contains("trailer\n<</Size 3/Root 1 0 R>>"), "{text}");
    assert!(!text.contains("3 0 obj"));
    // The strict rewrite keeps its inheriting view.
    let strict = latin1(&rewrite_document(&doc, DocumentWriteLimits::default()).unwrap());
    assert!(strict.contains("/Info 3 0 R"), "{strict}");
    // An edit is still the newest value.
    doc.set_trailer_entry(b"Info", Some(Object::Reference(id(3))))
        .unwrap();
    assert!(latin1(&app(&doc)).contains("trailer\n<</Size 4/Info 3 0 R/Root 1 0 R>>"));

    // A signed fixture whose newest xref stream has no Info: it is saved
    // with trailer <</Size 14/Root 1 0 R/ID[...]>>, its Size
    // counting the dropped xref stream object 13.
    let bytes = fixture("p0b/certified-p2-annot.pdf");
    let doc = Document::open(&bytes, limits()).unwrap();
    assert_eq!(doc.trailer_entry(b"Info"), None);
    assert!(doc.trailer_entry(b"ID").is_some());
    let text = latin1(&app(&doc));
    assert!(
        text.contains("trailer\n<</Size 13/Root 1 0 R/ID["),
        "{text}"
    );
}

#[test]
fn states_belong_to_their_document() {
    let large = fixture("large.pdf");
    let hello = fixture("hello.pdf");
    let source = Document::open(&large, limits()).unwrap();
    let stream = source
        .objects()
        .filter(|o| o.stream.is_some())
        .last()
        .unwrap()
        .id;
    let state = source.snapshot(stream);
    let trailer = source.trailer_state();
    let mut target = Document::open(&hello, limits()).unwrap();
    let before = app(&target);
    assert!(target.restore(stream, state.clone()).is_err());
    assert!(target.restore_trailer(trailer.clone()).is_err());
    assert_eq!(app(&target), before);
    // The same bytes opened again are another document too.
    let mut again = Document::open(&large, limits()).unwrap();
    assert!(again.restore(stream, state.clone()).is_err());
    // A clone shares the input, so its states apply both ways.
    let mut clone = source.clone();
    clone.remove_object(stream).unwrap();
    clone.restore(stream, state).unwrap();
    clone.restore_trailer(trailer).unwrap();
    assert_eq!(
        clone.stream_data(stream).unwrap(),
        source.stream_data(stream).unwrap()
    );
    let mut original = source.clone();
    original.restore(stream, clone.snapshot(stream)).unwrap();
    // Removal needs no state, so it is never refused.
    target.restore(id(5), None).unwrap();
}

#[test]
fn app_writer_accepts_signature_fields_and_encrypted_documents() {
    let limits_w = DocumentWriteLimits::default();
    let bytes = fixture("p5/empty-sigfield.pdf");
    let doc = Document::open(&bytes, limits()).unwrap();
    assert!(matches!(
        rewrite_reachable(&doc, limits_w),
        Err(DocumentWriteError::Unsupported(
            "signature dictionary or signature form field"
        ))
    ));
    let saved = app(&doc);
    let text = latin1(&saved);
    // Type, then Subtype, then the other keys in order.
    assert!(
        text.contains(
            "<</Type/Annot/Subtype/Widget/FT/Sig/T(Signature1)/F 4/P 4 0 R/Rect[72 112 340 172]/DA(/Helv 12 Tf 0 g)>>"
        ),
        "{text}"
    );
    assert!(text.contains("/SigFlags 1"));
    let reopened = Document::open(&saved, limits()).unwrap();
    assert_eq!(pages::page_ids(&reopened).len(), 2);

    for name in ["encrypted.pdf", "encrypted-aes256.pdf"] {
        let bytes = fixture(name);
        let doc = Document::open_with_password(&bytes, limits(), b"pdf4all-user").unwrap();
        assert!(matches!(
            rewrite_reachable(&doc, limits_w),
            Err(DocumentWriteError::Unsupported("encrypted document"))
        ));
        // The app save keeps encryption, so dropping it must be asked for.
        assert!(matches!(
            write_document(&doc, limits_w, WriteOptions::APP),
            Err(DocumentWriteError::Unsupported("encrypted document"))
        ));
        let strict_plaintext = WriteOptions {
            encryption: Encryption::Plaintext,
            ..WriteOptions::default()
        };
        assert!(matches!(
            write_document(&doc, limits_w, strict_plaintext),
            Err(DocumentWriteError::Unsupported("encrypted document"))
        ));
        let saved = plain(&doc);
        assert!(!latin1(&saved).contains("/Encrypt"));
        let plain = Document::open(&saved, limits()).unwrap();
        assert!(plain.security().is_none());
        assert_eq!(plain.trailer_entry(b"Encrypt"), None);
        assert_eq!(plain.trailer_entry(b"ID"), doc.trailer_entry(b"ID"));
        let page = pages::page_ids(&plain)[0];
        let content = plain
            .decoded_stream(pages::contents(&plain, page)[0], StreamLimits::default())
            .unwrap();
        assert!(latin1(&content).contains("PDF4All encrypted"), "{name}");
        let original = pages::page_ids(&doc)[0];
        assert_eq!(
            doc.decoded_stream(pages::contents(&doc, original)[0], StreamLimits::default())
                .unwrap(),
            content
        );
        // Integral reals print as integers (spec 01 section 3.7.5).
        for object in plain.objects().filter(|o| o.stream.is_none()) {
            let original = &doc.object(object.id).unwrap().value;
            assert_eq!(compact(&object.value), compact(original), "{name}");
        }
    }
}

#[test]
fn app_writer_tolerates_what_strict_writing_refuses() {
    let limits_w = DocumentWriteLimits::default();
    let bytes = pdf(
        &[
            (
                1,
                b"<< /Type /Catalog /Pages 2 0 R /Extra 9 0 R /Arr [9 0 R 1 0 R] /Perms << /DocMDP 4 0 R >> >>",
            ),
            (2, b"<< /Type /Pages /Kids [] /Count 0 >>"),
            (3, b"<< /Linearized 1 /L 100 >>"),
            (4, b"<< /Type /Sig /ByteRange [0 0 0 0] >>"),
        ],
        "/Root 1 0 R /Weird 5 /Info 9 0 R",
    );
    let doc = Document::open(&bytes, limits()).unwrap();
    assert!(rewrite_document(&doc, limits_w).is_err());
    let text = latin1(&app(&doc));
    // The app save writes dangling references as null and drops the stale
    // linearization dictionary and unknown trailer entries.
    assert!(
        text.contains("1 0 obj\n<</Type/Catalog/Pages 2 0 R/Extra null/Arr[null 1 0 R]/Perms<</DocMDP 4 0 R>>>>"),
        "{text}"
    );
    assert!(!text.contains("Linearized"));
    assert!(text.contains("trailer\n<</Size 5/Root 1 0 R>>"), "{text}");
    let everything = latin1(&everything(&doc));
    assert!(everything.contains("3 0 obj\n<< /L 100 /Linearized 1 >>"));
    assert!(everything.contains("/Extra null"));

    // A missing catalog is still an error.
    let mut broken = Document::open(&bytes, limits()).unwrap();
    broken.restore(id(1), None).unwrap();
    assert!(matches!(
        write_document(&broken, limits_w, WriteOptions::APP),
        Err(DocumentWriteError::MissingReference(_))
    ));
}

#[test]
fn compressing_new_streams_is_deterministic_and_leaves_originals() {
    let bytes = graph();
    let mut doc = Document::open(&bytes, limits()).unwrap();
    let content = b"0 0 m 100 100 l S ".repeat(20);
    doc.put_stream_decoded(id(4), Dictionary::new(), content.clone())
        .unwrap();
    let image = doc
        .allocate_stream_raw(
            dict(&[("Filter", Object::name("DCTDecode"))]),
            b"\xff\xd8 not decoded".to_vec(),
        )
        .unwrap();
    let mut catalog = doc.object(id(1)).unwrap().value.clone();
    catalog
        .as_dict_mut()
        .unwrap()
        .insert(b"Image".to_vec(), Object::Reference(image));
    doc.put_value(id(1), catalog).unwrap();
    let options = WriteOptions {
        compress_new_streams: true,
        ..WriteOptions::APP
    };
    let saved = write_document(&doc, DocumentWriteLimits::default(), options).unwrap();
    assert_eq!(
        saved,
        write_document(&doc, DocumentWriteLimits::default(), options).unwrap()
    );
    let reopened = Document::open(&saved, limits()).unwrap();
    let written = reopened.stream_dict(id(4)).unwrap();
    assert_eq!(written[b"Filter".as_slice()], Object::name("FlateDecode"));
    assert!(reopened.stream_data(id(4)).unwrap().unwrap().len() < content.len());
    assert_eq!(
        reopened
            .decoded_stream(id(4), StreamLimits::default())
            .unwrap(),
        content
    );
    // Raw-filtered new streams and original streams keep their bytes.
    assert_eq!(
        reopened.stream_data(image).unwrap(),
        doc.stream_data(image).unwrap()
    );
    assert_eq!(
        reopened.stream_data(id(6)).unwrap(),
        doc.stream_data(id(6)).unwrap()
    );
    // The document itself is not changed by compression.
    assert!(
        !doc.stream_dict(id(4))
            .unwrap()
            .contains_key(b"Filter".as_slice())
    );
    assert!(latin1(&app(&doc)).contains("4 0 obj\n<</Length 360>>\nstream\n"));
}

#[test]
fn text_strings_without_a_mark_read_as_utf8_when_they_are() {
    // Spec 01 section 3.6 rule 4: valid UTF-8 holding a non-ASCII character
    // is UTF-8.
    assert_eq!(decode_text_string(b"caf\xC3\xA9"), "caf\u{e9}");
    assert_eq!(decode_text_string("日本".as_bytes()), "日本");
    assert_eq!(decode_text_string("\u{1F600}".as_bytes()), "\u{1F600}");
    assert_eq!(
        decode_text_string(b"caf\xC3\xA9\x80"),
        "caf\u{c3}\u{a9}\u{2022}"
    );
    // Not UTF-8: Latin-1 bytes, overlong forms, encoded surrogates, code
    // points past U+10FFFF.
    assert_eq!(decode_text_string(b"\xE9t\xE9"), "\u{e9}t\u{e9}");
    assert_eq!(decode_text_string(b"a\xC0\xAFb"), "a\u{c0}\u{af}b");
    assert_eq!(
        decode_text_string(b"\xED\xA0\x80"),
        "\u{ed}\u{20ac}\u{2022}"
    );
    assert_eq!(
        decode_text_string(b"\xF4\x90\x80\x80"),
        "\u{f4}\u{2019}\u{2022}\u{2022}"
    );
    // A two-byte sequence that reads as both is UTF-8.
    assert_eq!(decode_text_string(b"\xC3\xA9"), "\u{e9}");
    // A byte PDFDocEncoding reads as an accent (0x18..=0x1F) makes the whole
    // string PDFDocEncoding; other controls do not.
    assert_eq!(
        decode_text_string(b"caf\xC3\xA9\x18"),
        "caf\u{c3}\u{a9}\u{2d8}"
    );
    assert_eq!(decode_text_string(b"\xC3\xA9\x1F"), "\u{c3}\u{a9}\u{2dc}");
    assert_eq!(decode_text_string(b"\xC3\xA9\x17"), "\u{e9}\u{17}");
    assert_eq!(decode_text_string(b"\xC3\xA9\x7F"), "\u{e9}\u{7f}");
}

#[test]
fn a_text_prefix_keeps_whole_characters() {
    use tandem_pdf_engine::object::decode_text_prefix;
    assert_eq!(
        decode_text_prefix(b"plain", 5),
        ("plain".to_string(), false)
    );
    assert_eq!(decode_text_prefix(b"plain", 3), ("pla".to_string(), true));
    // A cut inside a UTF-8 character drops it and still reads UTF-8.
    let utf8 = "aé日".as_bytes();
    assert_eq!(decode_text_prefix(utf8, 5), ("aé".to_string(), true));
    assert_eq!(decode_text_prefix(utf8, 2), ("a".to_string(), true));
    let marked = [&[0xEF, 0xBB, 0xBF][..], "é日".as_bytes()].concat();
    assert_eq!(decode_text_prefix(&marked, 7), ("é".to_string(), true));
    // UTF-16: an odd byte and half a surrogate pair are left out.
    let wide = [0xFE, 0xFF, 0x00, 0x41, 0xD8, 0x3D, 0xDE, 0x00];
    assert_eq!(
        decode_text_prefix(&wide, 8),
        ("A\u{1F600}".to_string(), false)
    );
    assert_eq!(decode_text_prefix(&wide, 7), ("A".to_string(), true));
    assert_eq!(decode_text_prefix(&wide, 6), ("A".to_string(), true));
    // Uncut, a lone surrogate is still U+FFFD.
    assert_eq!(
        decode_text_prefix(&wide[..6], 6),
        ("A\u{FFFD}".to_string(), false)
    );
    // PDFDocEncoding is one character a byte.
    assert_eq!(
        decode_text_prefix(b"\x80\xE9x", 2),
        ("\u{2022}\u{e9}".to_string(), true)
    );
    // The whole string picks the encoding: a prefix that is valid UTF-8 is
    // still PDFDocEncoding when a later byte is not UTF-8 or is an accent,
    // and a prefix of ASCII reads as the UTF-8 string it starts.
    assert_eq!(
        decode_text_prefix(b"\xC3\xA9\xC3\xA9\xE9", 4),
        ("\u{c3}\u{a9}\u{c3}\u{a9}".to_string(), true)
    );
    assert_eq!(
        decode_text_prefix(b"\xC3\xA9\x18", 2),
        ("\u{c3}\u{a9}".to_string(), true)
    );
    assert_eq!(
        decode_text_prefix(b"ab\xC3\xA9", 3),
        ("ab".to_string(), true)
    );
}
