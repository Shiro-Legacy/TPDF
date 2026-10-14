// SPDX-License-Identifier: AGPL-3.0-or-later
//! The document layer an app session builds on: owned input, the object
//! convenience API, text strings, resolution, journal primitives, mutations,
//! and the app writer (MuPDF `incremental=no,garbage=yes`, compact syntax).
use std::collections::BTreeMap;

use tandem_pdf_engine::document::{Document, DocumentLimits, ObjectState, TrailerState};
use tandem_pdf_engine::object::{
    Dictionary, Object, ObjectId, decode_text_string, encode_text_string,
};
use tandem_pdf_engine::pages::{PageLimits, collect_pages};
use tandem_pdf_engine::serialize::{SerializeStyle, WriteLimits, serialize_object_with_style};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::writer::{
    DocumentWriteError, DocumentWriteLimits, WriteOptions, rewrite_document, rewrite_reachable,
    write_document,
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
/// Every object, unreachable ones included, so restores are checked exactly.
fn everything(doc: &Document<'_>) -> Vec<u8> {
    let options = WriteOptions {
        strict: false,
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
            assert_eq!(owned.snapshot(object.id), borrowed.snapshot(object.id));
            assert_eq!(
                owned.stream_data(object.id).unwrap(),
                borrowed.stream_data(object.id).unwrap()
            );
        }
        assert_eq!(app(&owned), app(&borrowed), "{name}");
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
    assert_eq!(
        Object::text("caf\u{e9}"),
        Object::String(b"caf\xe9".to_vec())
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

#[test]
fn text_strings_use_pdfdoc_encoding_when_possible() {
    assert_eq!(encode_text_string(""), b"");
    assert_eq!(encode_text_string("Signature1"), b"Signature1");
    assert_eq!(encode_text_string("tab\tline\nret\r"), b"tab\tline\nret\r");
    assert_eq!(encode_text_string("\u{2022}"), [0x80]);
    assert_eq!(encode_text_string("\u{20AC}5"), [0xA0, b'5']);
    assert_eq!(
        encode_text_string("\u{2014}\u{FB02}\u{17E}"),
        [0x84, 0x94, 0x9E]
    );
    assert_eq!(encode_text_string("\u{2D8}\u{2DC}"), [0x18, 0x1F]);
    assert_eq!(encode_text_string("caf\u{e9} \u{ff}"), b"caf\xe9 \xff");
    let utf16 = |s: &str| {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in s.encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        bytes
    };
    // Characters PDFDocEncoding does not define force UTF-16BE.
    for text in [
        "\u{4E2D}\u{6587}",
        "a\u{A0}b",
        "soft\u{AD}hyphen",
        "\u{80}",
        "\u{9F}",
        "del\u{7F}",
        "\u{18}",
        "nul\u{0}",
        "\u{1F600}",
        "\u{2022} and \u{4E2D}",
    ] {
        assert_eq!(encode_text_string(text), utf16(text), "{text:?}");
    }
    assert_eq!(
        encode_text_string("\u{1F600}"),
        [0xFE, 0xFF, 0xD8, 0x3D, 0xDE, 0x00]
    );
    for text in [
        "",
        "Plain ASCII (with) \\ escapes",
        "\u{2022}\u{2020}\u{2021}\u{2026}\u{2014}\u{2013}\u{192}\u{2044}",
        "\u{2039}\u{203A}\u{2212}\u{2030}\u{201E}\u{201C}\u{201D}\u{2018}\u{2019}\u{201A}",
        "\u{2122}\u{FB01}\u{FB02}\u{141}\u{152}\u{160}\u{178}\u{17D}\u{131}\u{142}\u{153}",
        "\u{161}\u{17E}\u{20AC}\u{2D8}\u{2C7}\u{2C6}\u{2D9}\u{2DD}\u{2DB}\u{2DA}\u{2DC}",
        "\u{A1}\u{BF}\u{C0}\u{FF}",
        "\u{4E2D}\u{1F600}\u{A0}",
    ] {
        assert_eq!(decode_text_string(&encode_text_string(text)), text);
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
    // Annex D: every defined PDFDocEncoding code round-trips.
    let special = [
        (0x18, '\u{2D8}'),
        (0x1F, '\u{2DC}'),
        (0x80, '\u{2022}'),
        (0x8A, '\u{2212}'),
        (0x93, '\u{FB01}'),
        (0x9E, '\u{17E}'),
        (0xA0, '\u{20AC}'),
    ];
    for (byte, expected) in special {
        assert_eq!(decode_text_string(&[byte]), expected.to_string());
    }
    let mut defined = 0;
    for byte in 0..=255u8 {
        let text = decode_text_string(&[byte]);
        assert_eq!(text.chars().count(), 1);
        if encode_text_string(&text) == [byte] {
            defined += 1;
        } else {
            // Undefined codes decode as their Latin-1 value.
            assert!(
                matches!(byte, 0..=8 | 0x0B | 0x0C | 0x0E..=0x17 | 0x7F | 0x9F | 0xAD),
                "{byte:#x}"
            );
            assert_eq!(text, char::from(byte).to_string());
        }
    }
    assert_eq!(defined, 256 - 21 - 3);
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
        doc.dict_get(catalog, b"Pages").unwrap().as_dict().unwrap()[b"Count".as_slice()],
        Object::Integer(0)
    );
    assert!(doc.dict_get(catalog, b"Absent").unwrap().is_null());
    assert!(doc.dict_get(catalog, b"Gone").unwrap().is_null());
    assert!(doc.resolve(&Object::Reference(id(3))).is_err());
    assert_eq!(
        doc.resolve(&Object::Reference(id(11))).unwrap(),
        &Object::Integer(42)
    );
    assert!(doc.resolve(&Object::Reference(id(10))).is_err());
    assert!(doc.dict_get(catalog, b"Next").is_err());
    assert_eq!(
        doc.resolve(&Object::Integer(1)).unwrap(),
        &Object::Integer(1)
    );
    let reference = Object::Reference(id(5));
    let form = doc.resolve_dict(&reference).unwrap().unwrap();
    assert_eq!(form[b"Subtype".as_slice()], Object::name("Form"));
    assert_eq!(doc.stream_dict(id(5)).unwrap(), form);
    assert!(doc.stream_dict(id(2)).is_err());
    assert!(doc.stream_dict(id(99)).is_err());
    assert_eq!(doc.resolve_dict(&Object::Reference(id(11))).unwrap(), None);
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
    let page = collect_pages(&doc, PageLimits::default()).unwrap()[0].clone();
    let content = page.contents[0];
    let decoded = doc
        .decoded_stream(content, StreamLimits::default())
        .unwrap();
    assert!(latin1(&decoded).contains("PDF4All encrypted"));
    let state = doc.snapshot(content).unwrap();

    doc.put_value(content, Object::integer(1)).unwrap();
    assert_eq!(doc.stream_data(content).unwrap(), None);
    doc.restore(content, Some(state.clone()));
    assert_eq!(
        doc.decoded_stream(content, StreamLimits::default())
            .unwrap(),
        decoded
    );

    doc.remove_object(content).unwrap();
    doc.put_value(content, Object::integer(2)).unwrap();
    assert_eq!(doc.stream_data(content).unwrap(), None);
    doc.restore(content, None);
    assert!(doc.object(content).is_err());
    doc.restore(content, Some(state));
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
    assert!(
        doc.set_trailer_entry(b"Size", Some(Object::integer(3)))
            .is_err()
    );
    assert!(doc.set_trailer_entry(b"Prev", None).is_err());

    // Encrypt is carried for later; strict writing refuses it, app writing
    // does not write it yet.
    doc.set_trailer_entry(b"Encrypt", Some(Object::Reference(id(5))))
        .unwrap();
    assert!(matches!(
        rewrite_document(&doc, DocumentWriteLimits::default()),
        Err(DocumentWriteError::Unsupported(
            "unknown non-null trailer entry"
        ))
    ));
    assert!(!latin1(&app(&doc)).contains("Encrypt"));

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

    doc.restore_trailer(original.clone());
    doc.restore(info, None);
    doc.restore(catalog, None);
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
    (app(doc), everything(doc))
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
            doc.restore(*id, state.clone());
        }
        doc.restore_trailer(set.trailer.clone());
        assert_eq!(outputs(&doc), set.bytes_before);
    }
    assert_eq!(outputs(&doc), start);
    for set in &done {
        for (id, state) in &set.after {
            doc.restore(*id, state.clone());
        }
        doc.restore_trailer(set.after_trailer.clone());
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
fn compact_style_matches_mupdf_golden_syntax() {
    let widget = Object::array([
        Object::name("FT"),
        Object::name("Sig"),
        Object::name("T"),
        Object::text("Signature1"),
        Object::name("Rect"),
        Object::array([0, 0, 0, 0].map(Object::integer)),
    ]);
    assert_eq!(compact(&widget), "[/FT/Sig/T(Signature1)/Rect[0 0 0 0]]");
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
        "<</Filter/Adobe.PPKLite/SubFilter/adbe.pkcs7.detached/Type/Sig>>"
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
    // MuPDF 1.28.1 output for the same values.
    let reals: [(f64, &str); 22] = [
        (0.5, ".5"),
        (-0.5, "-.5"),
        (0.1, ".1"),
        (1.0 / 3.0, ".33333335"),
        (2.0 / 3.0, ".6666667"),
        (612.0, "612"),
        (1e-7, ".0000001"),
        (1e20, "100000000000000000000"),
        (123456.789, "123456.79"),
        (0.000123, ".000123"),
        (-0.0, "0"),
        (std::f64::consts::PI, "3.1415928"),
        (100.25, "100.25"),
        (72.12345678, "72.12346"),
        (1.5e10, "15000001000"),
        (4600095744.0, "4600095900"),
        (9.999999, "9.999999"),
        (1e-40, ".0000000000000000000000000000000000000001"),
        (223459952.0, "223459952"),
        (-2044019712.0, "-2044019712"),
        (0.30000001192092896, ".3"),
        (0.8, ".8"),
    ];
    for (value, expected) in reals {
        assert_eq!(compact(&Object::Real(value)), expected, "{value}");
    }
    let strings: [(&[u8], &str); 12] = [
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
    assert_eq!(compact(&Object::text("\u{2022}x")), "<8078>");
    assert_eq!(compact(&Object::text("\u{2022} list")), "(\\200 list)");
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

#[test]
fn app_save_of_hello_matches_mupdf_object_syntax() {
    let bytes = fixture("hello.pdf");
    let doc = Document::open(&bytes, limits()).unwrap();
    let text = latin1(&app(&doc));
    // MuPDF writes dictionaries in insertion order; this crate sorts keys.
    for expected in [
        "1 0 obj\n<</Pages 2 0 R/Type/Catalog>>\nendobj\n",
        "2 0 obj\n<</Count 1/Kids[3 0 R]/Type/Pages>>\nendobj\n",
        "3 0 obj\n<</Contents 5 0 R/MediaBox[0 0 612 792]/Parent 2 0 R/Resources<</Font<</F1 4 0 R>>>>/Type/Page>>\nendobj\n",
        "4 0 obj\n<</BaseFont/Helvetica/Encoding/WinAnsiEncoding/Subtype/Type1/Type/Font>>\nendobj\n",
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

#[test]
fn default_options_reproduce_the_existing_rewrites() {
    let limits_w = DocumentWriteLimits::default();
    for name in ["hello.pdf", "form.pdf", "nav.pdf", "layers.pdf", "cjk.pdf"] {
        let bytes = fixture(name);
        let doc = Document::open(&bytes, limits()).unwrap();
        assert_eq!(
            write_document(&doc, limits_w, WriteOptions::default()).ok(),
            rewrite_document(&doc, limits_w).ok()
        );
        let garbage = WriteOptions {
            garbage: true,
            ..WriteOptions::default()
        };
        assert_eq!(
            write_document(&doc, limits_w, garbage).ok(),
            rewrite_reachable(&doc, limits_w).ok()
        );
        let saved = app(&doc);
        let reopened = Document::open(&saved, limits()).unwrap();
        let pages = collect_pages(&reopened, PageLimits::default()).unwrap();
        assert_eq!(
            pages.len(),
            collect_pages(&doc, PageLimits::default()).unwrap().len()
        );
    }
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
    assert!(
        text.contains(
            "/FT/Sig/P 4 0 R/Rect[72 112 340 172]/Subtype/Widget/T(Signature1)/Type/Annot>>"
        ),
        "{text}"
    );
    assert!(text.contains("/SigFlags 1"));
    let reopened = Document::open(&saved, limits()).unwrap();
    assert_eq!(
        collect_pages(&reopened, PageLimits::default())
            .unwrap()
            .len(),
        2
    );

    for name in ["encrypted.pdf", "encrypted-aes256.pdf"] {
        let bytes = fixture(name);
        let doc = Document::open_with_password(&bytes, limits(), b"pdf4all-user").unwrap();
        assert!(matches!(
            rewrite_reachable(&doc, limits_w),
            Err(DocumentWriteError::Unsupported("encrypted document"))
        ));
        let saved = app(&doc);
        assert!(!latin1(&saved).contains("/Encrypt"));
        let plain = Document::open(&saved, limits()).unwrap();
        assert!(plain.security().is_none());
        assert_eq!(plain.trailer_entry(b"Encrypt"), None);
        assert_eq!(plain.trailer_entry(b"ID"), doc.trailer_entry(b"ID"));
        let page = collect_pages(&plain, PageLimits::default()).unwrap()[0].clone();
        let content = plain
            .decoded_stream(page.contents[0], StreamLimits::default())
            .unwrap();
        assert!(latin1(&content).contains("PDF4All encrypted"), "{name}");
        let original = collect_pages(&doc, PageLimits::default()).unwrap()[0].clone();
        assert_eq!(
            doc.decoded_stream(original.contents[0], StreamLimits::default())
                .unwrap(),
            content
        );
        // Integral reals print as integers, as MuPDF prints them.
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
    // MuPDF writes dangling references as null and drops the stale
    // linearization dictionary and unknown trailer entries.
    assert!(
        text.contains("1 0 obj\n<</Arr[null 1 0 R]/Extra null/Pages 2 0 R/Perms<</DocMDP 4 0 R>>/Type/Catalog>>"),
        "{text}"
    );
    assert!(!text.contains("Linearized"));
    assert!(text.contains("trailer\n<</Size 5/Root 1 0 R>>"), "{text}");
    let everything = latin1(&everything(&doc));
    assert!(everything.contains("3 0 obj\n<< /L 100 /Linearized 1 >>"));
    assert!(everything.contains("/Extra null"));

    // A missing catalog is still an error.
    let mut broken = Document::open(&bytes, limits()).unwrap();
    broken.restore(id(1), None);
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
