// SPDX-License-Identifier: MIT
//! Integers past i64 (spec 01 section 4.1 item 4): another viewer wraps them,
//! so `/UserUnit 1000000000000000000000000000000` would be saved as
//! 5076944270305263616. Here such an integer is the real its digits name;
//! where an integer is required (/Length, an xref offset, an object number)
//! it is refused, never wrapped into a plausible value.
//! Every file is built here.
use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Object, ObjectId};

const BIG: &str = "1000000000000000000000000000000";

fn content(text: &str) -> (String, usize) {
    let data = format!("BT /F1 24 Tf 20 200 Td ({text}) Tj ET");
    let len = data.len();
    (data, len)
}

/// Catalog, pages, one page (`page` entries), its content (`length` for
/// /Length, or the data's), a font and an object 6 holding `extra`; a
/// classic xref whose entries come from `offset` (index, real offset).
fn file(
    page: &str,
    length: Option<&str>,
    extra: &str,
    offset: impl Fn(usize, usize) -> String,
) -> Vec<u8> {
    let (data, len) = content("Alpha");
    let length = length.map_or(len.to_string(), str::to_string);
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /Extra 6 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] {page} /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
        ),
        format!("<< /Length {length} >>\nstream\n{data}\nendstream"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        extra.into(),
    ];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
    for (index, at) in offsets.into_iter().enumerate() {
        bytes.extend_from_slice(format!("{} 00000 n \n", offset(index, at)).as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}

fn plain(index: usize, at: usize) -> String {
    let _ = index;
    format!("{at:010}")
}

fn dispatch(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    engine.dispatch(session, request, Vec::new()).0
}

/// The app's open (page count, text) and its validated save.
fn open_and_save(bytes: Vec<u8>) -> (usize, String, Vec<u8>) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let (reply, _) = engine.dispatch(
        &mut session,
        json!({"op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}}),
        vec![bytes],
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let pages = reply["pageCount"].as_u64().unwrap_or(0) as usize;
    let text = dispatch(&mut engine, &mut session, json!({"op": "text-all"}));
    let text = text["text"].as_str().unwrap_or_default().to_string();
    let (saved, mut outputs) = engine.dispatch(
        &mut session,
        json!({"op": "save", "entropy": {"$buffer": 0}}),
        vec![vec![7; 32]],
    );
    assert_eq!(saved["ok"], true, "{saved}");
    let saved = outputs.remove(0);
    let (check, _) = engine.dispatch(
        &mut session,
        json!({"op": "validate", "pdf": {"$buffer": 0}}),
        vec![saved.clone()],
    );
    assert_eq!(check["ok"], true, "{check}");
    (pages, text, saved)
}

/// A real's value in `doc`.
fn real(value: &Object) -> Option<f64> {
    match value {
        Object::Real(v) => Some(*v),
        Object::Integer(v) => Some(*v as f64),
        _ => None,
    }
}

fn page_dict(doc: &Document<'_>) -> tandem_pdf_engine::object::Dictionary {
    doc.resolve_id(ObjectId::new(3, 0).unwrap())
        .as_dict()
        .unwrap()
        .clone()
}

#[test]
fn an_overflowing_integer_in_a_dictionary_and_an_array_keeps_its_value() {
    let extra = format!("<< /A [1 {BIG} -{BIG} 99999999999999999999] /B {BIG} >>");
    let bytes = file(&format!("/UserUnit {BIG}"), None, &extra, plain);
    let (pages, text, saved) = open_and_save(bytes);
    assert_eq!(pages, 1);
    assert!(text.contains("Alpha"), "{text}");
    let doc = Document::open(&saved, DocumentLimits::default()).unwrap();
    let unit = real(page_dict(&doc).get(b"UserUnit".as_slice()).unwrap()).unwrap();
    assert!((unit / 1e30 - 1.0).abs() < 1e-6, "{unit}");
    let written = String::from_utf8_lossy(&saved).into_owned();
    assert!(
        !written.contains("5076944270305263616"),
        "the wrapped value"
    );
    let six = doc
        .resolve_id(ObjectId::new(6, 0).unwrap())
        .as_dict()
        .unwrap()
        .clone();
    let a = six.get(b"A".as_slice()).unwrap().as_array().unwrap();
    let values: Vec<f64> = a.iter().map(|v| real(v).unwrap()).collect();
    for (got, want) in values.iter().zip([1.0, 1e30, -1e30, 1e20]) {
        assert!((got - want).abs() <= want.abs() * 1e-6, "{values:?}");
    }
    let b = real(six.get(b"B".as_slice()).unwrap()).unwrap();
    assert!((b / 1e30 - 1.0).abs() < 1e-6, "{b}");
}

#[test]
fn an_overflowing_length_is_refused_and_the_stream_found_by_its_end() {
    // 2^64 + 5 wrapped to 5: a plausible, wrong length.
    let bytes = file("", Some("18446744073709551621"), "<< >>", plain);
    let (pages, text, saved) = open_and_save(bytes);
    assert_eq!(pages, 1);
    assert!(text.contains("Alpha"), "{text}");
    let doc = Document::open(&saved, DocumentLimits::default()).unwrap();
    let id = ObjectId::new(4, 0).unwrap();
    let data = doc
        .decoded_stream(id, tandem_pdf_engine::stream::StreamLimits::default())
        .unwrap();
    assert_eq!(data, content("Alpha").0.as_bytes());
}

#[test]
fn an_overflowing_xref_offset_is_refused_not_wrapped() {
    // Page 1's entry: 2^64 + its offset, which wraps to its offset.
    let bytes = file("", None, "<< >>", |index, at| match index {
        2 => format!("{}", (u128::from(u64::MAX) + 1 + at as u128)),
        _ => format!("{at:010}"),
    });
    // The strict xref refuses it; the repaired file still shows the page.
    assert!(Document::open(&bytes, DocumentLimits::default()).is_err());
    let (pages, text, _) = open_and_save(bytes);
    assert_eq!(pages, 1);
    assert!(text.contains("Alpha"), "{text}");
    // An object number past i64 is no header.
    let mut header = b"%PDF-1.4\n".to_vec();
    header.extend_from_slice(b"18446744073709551617 0 obj\n<< /Type /Catalog >>\nendobj\n");
    assert!(Document::open(&header, DocumentLimits::default()).is_err());
}
