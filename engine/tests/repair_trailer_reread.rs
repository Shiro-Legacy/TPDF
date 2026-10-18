// SPDX-License-Identifier: MIT
//! A trailer whose dictionary holds an object header (final review, p7
//! seed 41 i=67): a `2 0 obj` spliced into an unterminated hex string of
//! the trailer's /ID. A header inside an unterminated trailer string is
//! read as part of the string, so the trailer keeps /Info and /Encrypt and
//! the header does not become a later object 2 that replaces the page tree
//! (spec 03 section 4.4 items 2 and 7). Every file is built here.
use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::Object;
use tandem_pdf_engine::repair::{RepairLimits, rebuild};

fn content(text: &str) -> String {
    let data = format!("BT /F1 24 Tf 20 200 Td ({text}) Tj ET");
    format!("<< /Length {} >>\nstream\n{data}\nendstream", data.len())
}

/// Two pages (Alpha, Beta), an /Info dictionary (object 8), a classic
/// xref that `startxref` misses (so the file is scanned) and `trailer`.
fn file(trailer: &str) -> Vec<u8> {
    let page = |contents: u32| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources << /Font << /F1 7 0 R >> >> /Contents {contents} 0 R >>"
        )
    };
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
        page(5),
        page(6),
        content("Alpha"),
        content("Beta"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        "<< /Title (Kept title) >>".into(),
    ];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
    }
    bytes.extend_from_slice(b"xref\n0 9\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer\n{trailer}\nstartxref\n999999\n%%EOF\n").as_bytes());
    bytes
}

/// The review's shape: the /ID's second string loses its `>`, a header
/// and a Pages dictionary are spliced in (its last kid glued to hex
/// digits), and the string ends inside the copy; /Info follows.
const SPLICED: &str = "<< /Size 9 /Root 1 0 R /ID [<0123456789ABCDEF><89ABj\n\n2 0 obj\n<< /Type /Pages /Kids [3 0 R 4 0 R9ABCDEF>] /Info 8 0 R >>";

fn dispatch(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    engine.dispatch(session, request, Vec::new()).0
}

/// Page count and text as the app shows them.
fn shown(bytes: Vec<u8>) -> (usize, String) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let (reply, _) = engine.dispatch(
        &mut session,
        json!({"op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}}),
        vec![bytes],
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let ids = dispatch(&mut engine, &mut session, json!({"op": "page-ids"}));
    let text = dispatch(&mut engine, &mut session, json!({"op": "text-all"}));
    (
        ids["ids"].as_array().map_or(0, Vec::len),
        text["text"].as_str().unwrap_or_default().to_string(),
    )
}

/// The rebuilt file's trailer /Info title.
fn title(bytes: &[u8]) -> Option<Vec<u8>> {
    let repaired = rebuild(bytes, RepairLimits::default()).unwrap();
    let doc = Document::open(&repaired.bytes, DocumentLimits::default()).unwrap();
    let info = doc.trailer().get(b"Info".as_slice())?.clone();
    let info = doc.resolve(&info).as_dict()?.clone();
    match info.get(b"Title".as_slice()) {
        Some(Object::String(s)) => Some(s.clone()),
        _ => None,
    }
}

#[test]
fn a_header_inside_a_trailer_string_is_read_as_the_string() {
    let bytes = file(SPLICED);
    let (pages, text) = shown(bytes.clone());
    assert_eq!(pages, 2, "{text}");
    assert!(text.contains("Alpha") && text.contains("Beta"), "{text}");
    assert_eq!(title(&bytes).as_deref(), Some(b"Kept title".as_slice()));
}

#[test]
fn an_intact_trailer_and_a_trailer_cut_before_the_next_object_are_unchanged() {
    // Intact: the reread is never reached.
    let intact = file("<< /Size 9 /Root 1 0 R /Info 8 0 R >>");
    assert_eq!(shown(intact.clone()).0, 2);
    assert_eq!(title(&intact).as_deref(), Some(b"Kept title".as_slice()));
    // A trailer that lost its `>>` before a whole object at a line start
    // is not read through it: that object (a new page tree) is the next
    // definition, as for any dictionary cut short (spec 03 section 4.4 item
    // 3). Another viewer drops the trailer too but does not take the object
    // (2 pages): a recorded difference for a glued whole object that
    // redefines an intact one with a broken xref.
    let cut = file(
        "<< /Size 9 /Root 1 0 R /Info 8 0 R\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj",
    );
    let (pages, text) = shown(cut);
    assert_eq!(pages, 1, "{text}");
    assert!(text.contains("Alpha") && !text.contains("Beta"), "{text}");
}
