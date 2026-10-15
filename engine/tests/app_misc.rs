// SPDX-License-Identifier: AGPL-3.0-or-later
//! Stamps, batch stamping and the compare slot on the app session: the
//! streams and font MuPDF writes, placement on every rotation, inherited
//! resources kept, exact undo; batch-file on a document the session never
//! holds; compare-open, -close, -render and -text with engine.ts's refusals.
use std::f64::consts::FRAC_1_SQRT_2;

use serde_json::{Value, json};
use tandem_pdf_engine::app::ops::stamp::MAX_STAMP_CONTENTS;
use tandem_pdf_engine::app::{AppEngine, AppSession, MAX_PDF_BYTES, MAX_SAVE_BYTES};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Dictionary, Object, ObjectId};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::view::ViewDocument;
use tandem_pdf_engine::writer::{
    DocumentWriteLimits, Encryption, Entropy, WriteOptions, write_document,
};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("../fixtures/{name}")).unwrap()
}

fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}

/// A classic-xref PDF from object bodies numbered from 1.
fn classic(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn stream(data: &str) -> Vec<u8> {
    format!("<< /Length {} >>\nstream\n{data}\nendstream", data.len()).into_bytes()
}

/// nav.pdf's shape: `n` letter pages, each with its own /Resources naming
/// font 3 as /F1; page `i` is object `4 + 2i` saying `words[i]`.
fn pages_pdf(words: &[&str]) -> Vec<u8> {
    let kids: Vec<String> = (0..words.len())
        .map(|i| format!("{} 0 R", 4 + 2 * i))
        .collect();
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            words.len()
        )
        .into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ];
    for (i, word) in words.iter().enumerate() {
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
                5 + 2 * i
            )
            .into_bytes(),
        );
        objects.push(stream(&format!("BT /F1 24 Tf 72 700 Td ({word}) Tj ET")));
    }
    classic(&objects)
}

/// p2-decorate's 400x200 blank page with /Rotate.
fn rotated_pdf(rotate: i32) -> Vec<u8> {
    classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Rotate {rotate} >>")
            .into_bytes(),
    ])
}

/// One page per line, Helvetica 18 at `step` points apart (p6-compare's linesPdf).
fn lines_pdf(lines: &[&str]) -> Vec<u8> {
    let content: String = (lines.iter().enumerate())
        .map(|(i, text)| {
            format!(
                "BT /F1 18 Tf 72 {} Td ({text}) Tj ET\n",
                740 - 30 * i as i64
            )
        })
        .collect();
    classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
        stream(&content),
    ])
}

fn dispatch(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    engine.dispatch(session, request, Vec::new()).0
}

fn dispatch_with(
    engine: &mut AppEngine,
    session: &mut AppSession,
    request: Value,
    buffers: Vec<Vec<u8>>,
) -> (Value, Vec<Vec<u8>>) {
    engine.dispatch(session, request, buffers)
}

fn open(engine: &mut AppEngine, session: &mut AppSession, bytes: Vec<u8>) -> Value {
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    dispatch_with(engine, session, request, vec![bytes]).0
}

fn session_with(bytes: Vec<u8>) -> (AppEngine, AppSession) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let reply = open(&mut engine, &mut session, bytes);
    assert_eq!(reply["ok"], true, "{reply}");
    (engine, session)
}

fn op(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    let reply = dispatch(engine, session, request);
    assert_eq!(reply["ok"], true, "{reply}");
    reply
}

fn document(session: &AppSession) -> &Document<'static> {
    session.doc.as_ref().unwrap().document()
}

/// A full rewrite of every object, for exact before/after comparisons.
fn serialized(session: &AppSession) -> Vec<u8> {
    let options = WriteOptions {
        garbage: false,
        encryption: Encryption::Plaintext,
        ..WriteOptions::APP
    };
    write_document(document(session), DocumentWriteLimits::default(), options).unwrap()
}

fn page_dict<'d>(doc: &'d Document<'_>, page: usize) -> &'d Dictionary {
    tandem_pdf_engine::app::info::pages(doc)[page].dict
}

/// The /Contents entries of a page: an array's items, or the one stream.
fn content_items(doc: &Document<'_>, page: usize) -> Vec<Object> {
    let own = page_dict(doc, page).get(b"Contents".as_slice()).unwrap();
    match doc.resolve(own) {
        Object::Array(items) => items.clone(),
        _ => vec![own.clone()],
    }
}

/// The decoded bytes of each /Contents stream of a page.
fn contents(doc: &Document<'_>, page: usize) -> Vec<Vec<u8>> {
    (content_items(doc, page).iter())
        .filter_map(Object::as_reference)
        .map(|id| doc.decoded_stream(id, StreamLimits::default()).unwrap())
        .collect()
}

fn last_stream(session: &AppSession, page: usize) -> String {
    let streams = contents(document(session), page);
    String::from_utf8_lossy(streams.last().unwrap()).into_owned()
}

fn texts(session: &mut AppSession) -> Vec<String> {
    let view = session.view().unwrap();
    (0..view.page_count())
        .map(|page| view.structured_text(page).unwrap().as_text())
        .collect()
}

/// The first line of page `page` whose text contains `needle`: its bbox
/// (x0, y0, x1, y1, y down) and baseline direction, in display space.
fn line_of(view: &ViewDocument, page: usize, needle: &str) -> ([f64; 4], [f64; 2]) {
    let text = view.structured_text(page).unwrap();
    for block in text.blocks() {
        for line in &block.lines {
            let content: String = line.chars.iter().map(|c| c.c).collect();
            if content.contains(needle) {
                return (line.bbox, line.dir);
            }
        }
    }
    panic!("no line with {needle:?} on page {page}");
}

fn helvetica() -> Object {
    Object::dictionary([
        ("Type", Object::name("Font")),
        ("Subtype", Object::name("Type1")),
        ("BaseFont", Object::name("Helvetica")),
        ("Encoding", Object::name("WinAnsiEncoding")),
    ])
}

/// Helvetica advances (AFM) of the characters the tests stamp.
fn helvetica_width(text: &str) -> f64 {
    let units: u32 = (text.chars())
        .map(|c| match c {
            'D' | 'R' => 722,
            'A' | 'E' => 667,
            'F' | 'T' => 611,
            ' ' => 278,
            '(' | ')' | '-' => 333,
            'x' => 500,
            'C' => 722,
            'M' => 833,
            '0'..='9' => 556,
            _ => panic!("no width for {c:?}"),
        })
        .sum();
    f64::from(units) / 1000.0
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

// ------------------------------------------------------------------ stamp

#[test]
fn a_watermark_appends_mupdfs_streams_and_one_undo_takes_every_page_back() {
    let (mut engine, mut session) = session_with(pages_pdf(&["one", "two", "three"]));
    let start = serialized(&session);
    let reply = op(
        &mut engine,
        &mut session,
        json!({"id": 7, "op": "stamp", "kind": "watermark", "text": "DRAFT (x)"}),
    );
    assert_eq!(reply["id"], 7);
    assert_eq!(
        (&reply["revision"], &reply["dirty"]),
        (&json!(1), &json!(true))
    );
    assert_eq!(reply["pageCount"], 3);
    assert!(reply.get("bates").is_none());
    let stamped = serialized(&session);
    let doc = document(&session);
    // Every page: [q, its own stream, the stamp], one font object shared.
    let c = FRAC_1_SQRT_2;
    let (u, v) = (helvetica_width("DRAFT (x)") * 48.0 / 2.0, 24.0);
    let (tx, ty) = (
        round2(306.0 - (c * u - c * v)),
        round2(396.0 - (c * u + c * v)),
    );
    let tail = format!(
        "Q\n/PDF4All <</Kind /Watermark>> BDC 0.5 g BT /PDF4AllF 48 Tf 0.7071067811865476 \
         0.7071067811865476 -0.7071067811865476 0.7071067811865476 {tx} {ty} Tm (DRAFT \\(x\\)) Tj ET EMC\n"
    );
    let mut fonts = Vec::new();
    for page in 0..3 {
        let streams = contents(doc, page);
        assert_eq!(streams.len(), 3);
        assert_eq!(streams[0], b"q\n");
        assert_eq!(String::from_utf8_lossy(&streams[2]), tail);
        let font = doc.dict_get(page_dict(doc, page), b"Resources");
        let font = doc
            .dict_get(font.as_dict().unwrap(), b"Font")
            .as_dict()
            .unwrap();
        let keys: Vec<&[u8]> = font.keys().map(Vec::as_slice).collect();
        assert_eq!(keys, [b"F1".as_slice(), b"PDF4AllF"]);
        fonts.push(font.get(b"PDF4AllF".as_slice()).unwrap().clone());
    }
    assert!(fonts.windows(2).all(|pair| pair[0] == pair[1]), "{fonts:?}");
    let font = fonts[0].as_reference().unwrap();
    assert_eq!(doc.object(font).unwrap().value, helvetica());
    // The appended streams carry no filter, as MuPDF writes them.
    let tail_id = doc
        .dict_get(page_dict(doc, 0), b"Contents")
        .as_array()
        .unwrap()[2]
        .as_reference()
        .unwrap();
    let tail_dict = doc.object(tail_id).unwrap().value.as_dict().unwrap();
    assert_eq!(tail_dict.keys().count(), 1, "/Length only");
    assert!(
        texts(&mut session)
            .iter()
            .all(|text| text.contains("DRAFT (x)"))
    );

    let undone = op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(undone["step"], "add watermark");
    assert_eq!(serialized(&session), start);
    let redone = op(&mut engine, &mut session, json!({"op": "redo"}));
    assert_eq!(redone["step"], "add watermark");
    assert_eq!(serialized(&session), stamped);
}

#[test]
fn headers_and_footers_sit_upright_at_the_visual_top_and_bottom_of_every_rotation() {
    for (rotate, header, right) in [
        (0, "1 0 0 1 36 164 Tm", 364.0),
        (90, "0 1 -1 0 36 36 Tm", 164.0),
        (180, "-1 0 0 -1 364 36 Tm", 364.0),
        (270, "0 -1 1 0 364 164 Tm", 164.0),
    ] {
        let (mut engine, mut session) = session_with(rotated_pdf(rotate));
        let reply = op(
            &mut engine,
            &mut session,
            json!({"op": "stamp", "kind": "header", "text": "HEADER"}),
        );
        assert_eq!(reply["rotations"], json!([rotate]));
        assert!(last_stream(&session, 0).contains(header), "R={rotate}");
        op(
            &mut engine,
            &mut session,
            json!({"op": "stamp", "kind": "footer", "text": "ACME-{bates}", "batesStart": 100,
                "align": "right"}),
        );
        let view = session.view().unwrap();
        let (width, height) = view.page_size(0).unwrap();
        let (top, dir) = line_of(view, 0, "HEADER");
        assert_eq!(dir, [1.0, 0.0], "R={rotate}: upright");
        assert!((top[0] - 36.0).abs() < 1.0, "R={rotate}: {top:?}");
        assert!(top[3] < 40.0 && top[1] > 20.0, "R={rotate}: header {top:?}");
        let (bottom, dir) = line_of(view, 0, "ACME-000100");
        assert_eq!(dir, [1.0, 0.0], "R={rotate}: upright");
        assert!(bottom[1] > height - 60.0, "R={rotate}: footer {bottom:?}");
        assert_eq!(width - 36.0, right);
        assert!((bottom[2] - right).abs() <= 2.0, "R={rotate}: {bottom:?}");
    }
}

#[test]
fn right_aligned_text_ends_at_the_margin_by_helvetica_widths() {
    let (mut engine, mut session) = session_with(pages_pdf(&["one", "two", "three"]));
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "footer", "text": "ACME-{bates} {label}", "batesStart": 7,
            "align": "right", "pages": [2, 0, 2]}),
    );
    assert_eq!(reply["bates"], json!({"from": 7, "to": 8}));
    // MuPDF 1.28.1 writes exactly this (measured through engine.ts).
    assert_eq!(
        last_stream(&session, 0),
        "Q\n/PDF4All <</Kind /Footer>> BDC 0 g BT /PDF4AllF 10 Tf 1 0 0 1 502.08 36 Tm (ACME-000007 1) Tj ET EMC\n"
    );
    assert!(last_stream(&session, 2).contains("(ACME-000008 3) Tj"));
    assert_eq!(contents(document(&session), 1).len(), 1, "page 2 untouched");
}

#[test]
fn a_watermark_reads_upward_and_centred_on_every_rotation() {
    for rotate in [0, 90, 180, 270] {
        let (mut engine, mut session) = session_with(rotated_pdf(rotate));
        op(
            &mut engine,
            &mut session,
            json!({"op": "stamp", "kind": "watermark", "text": "DRAFT"}),
        );
        let view = session.view().unwrap();
        let (width, height) = view.page_size(0).unwrap();
        let (bbox, dir) = line_of(view, 0, "DRAFT");
        assert!(
            (dir[0] - FRAC_1_SQRT_2).abs() < 1e-3 && (dir[1] + FRAC_1_SQRT_2).abs() < 1e-3,
            "R={rotate}: {dir:?}"
        );
        let centre = ((bbox[0] + bbox[2]) / 2.0, (bbox[1] + bbox[3]) / 2.0);
        assert!(
            (centre.0 - width / 2.0).abs() < 8.0 && (centre.1 - height / 2.0).abs() < 8.0,
            "R={rotate}: centre {centre:?} of {width}x{height}"
        );
    }
}

#[test]
fn placeholders_fill_in_order_and_labels_go_in_literally() {
    let (mut engine, mut session) = session_with(pages_pdf(&["one", "two", "three"]));
    op(
        &mut engine,
        &mut session,
        json!({"op": "labels-set", "from": 0, "style": "D", "prefix": "L{bates}$&-"}),
    );
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "header", "text": "[{label}] {bates} {page}/{pages}",
            "batesStart": 9, "batesPad": 2, "pages": [1]}),
    );
    assert_eq!(reply["bates"], json!({"from": 9, "to": 9}));
    assert!(
        last_stream(&session, 1).contains("([L{bates}$&-2] 09 2/3) Tj"),
        "{}",
        last_stream(&session, 1)
    );
    // A label character outside WinAnsi is drawn as `?`; the WinAnsi extras are one byte.
    op(
        &mut engine,
        &mut session,
        json!({"op": "labels-set", "from": 0, "style": "", "prefix": "\u{3b1}"}),
    );
    op(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "footer", "text": "{label} 5\u{20ac} (\\)", "pages": [0]}),
    );
    let streams = contents(document(&session), 0);
    let tail = streams.last().unwrap();
    let at = tail.windows(3).position(|w| w == b"(? ").unwrap();
    assert_eq!(&tail[at..at + 16], b"(? 5\x80 \\(\\\\\\)) Tj");
}

#[test]
fn inherited_resources_and_boxes_are_used_where_they_live() {
    // Page 0 inherits /Resources, /MediaBox (200x300) and /Rotate 90 from node 3.
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 2 /Resources 5 0 R >>".to_vec(),
        b"<< /Type /Pages /Parent 2 0 R /Kids [4 0 R 7 0 R] /Count 2 /MediaBox [0 0 200 300] /Rotate 90 >>".to_vec(),
        b"<< /Type /Page /Parent 3 0 R /Contents 6 0 R >>".to_vec(),
        b"<< /Font << /F1 8 0 R >> /ProcSet [/PDF /Text] >>".to_vec(),
        stream("BT /F1 12 Tf 20 20 Td (inherited) Tj ET"),
        b"<< /Type /Page /Parent 3 0 R /Contents 6 0 R /Resources << /Font 9 0 R >> /CropBox [10 10 110 210] /Rotate 0 >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        b"<< /F1 8 0 R >>".to_vec(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    let start = serialized(&session);
    op(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "header", "text": "TOP"}),
    );
    let doc = document(&session);
    // No /Resources appears on page 0: the stamp font joins the inherited
    // dictionary (object 5), which keeps /F1 and /ProcSet.
    assert!(
        doc.resolve_id(id(4))
            .as_dict()
            .unwrap()
            .get(b"Resources".as_slice())
            .is_none()
    );
    let shared = doc.resolve_id(id(5)).as_dict().unwrap();
    let fonts = doc.dict_get(shared, b"Font").as_dict().unwrap();
    assert!(fonts.get(b"F1".as_slice()).is_some() && fonts.get(b"PDF4AllF".as_slice()).is_some());
    assert!(shared.get(b"ProcSet".as_slice()).is_some());
    // Page 1's own /Font is the indirect object 9, edited in place.
    let own = doc.resolve_id(id(9)).as_dict().unwrap();
    assert_eq!(own.keys().count(), 2);
    // Placement on the inherited 200x300 box turned 90 (visual 300x200),
    // and on page 1's crop box (100x200, upright).
    assert!(
        last_stream(&session, 0).contains("0 1 -1 0 36 36 Tm"),
        "{}",
        last_stream(&session, 0)
    );
    assert!(
        last_stream(&session, 1).contains("1 0 0 1 46 174 Tm"),
        "{}",
        last_stream(&session, 1)
    );
    let all = texts(&mut session);
    assert!(
        all[0].contains("inherited") && all[0].contains("TOP"),
        "{all:?}"
    );
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), start);
}

#[test]
fn stamps_share_one_font_object_and_a_stamp_after_an_undo_gets_a_live_one() {
    let (mut engine, mut session) = session_with(pages_pdf(&["one", "two", "three"]));
    let font_of = |session: &AppSession, page: usize| {
        let doc = document(session);
        let res = doc.dict_get(page_dict(doc, page), b"Resources");
        let fonts = doc.dict_get(res.as_dict().unwrap(), b"Font");
        fonts
            .as_dict()
            .unwrap()
            .get(b"PDF4AllF".as_slice())
            .cloned()
    };
    op(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "header", "text": "a", "pages": [1]}),
    );
    op(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "footer", "text": "b", "pages": [0, 1, 2]}),
    );
    let first = font_of(&session, 1).unwrap();
    assert_eq!(font_of(&session, 0), Some(first.clone()));
    assert_eq!(font_of(&session, 2), Some(first.clone()));
    op(&mut engine, &mut session, json!({"op": "undo"}));
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(font_of(&session, 1), None);
    op(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "watermark", "text": "DRAFT"}),
    );
    let font = font_of(&session, 0).unwrap().as_reference().unwrap();
    assert_eq!(document(&session).object(font).unwrap().value, helvetica());
    let request = json!({"id": 3, "op": "save", "entropy": {"$buffer": 0}});
    let (saved, outputs) = dispatch_with(&mut engine, &mut session, request, vec![vec![7; 32]]);
    assert_eq!(saved["ok"], true, "{saved}");
    let reopened = Document::open(&outputs[0], DocumentLimits::default()).unwrap();
    let page = page_dict(&reopened, 0);
    let res = reopened.dict_get(page, b"Resources");
    let fonts = reopened
        .dict_get(res.as_dict().unwrap(), b"Font")
        .as_dict()
        .unwrap();
    assert_eq!(reopened.dict_get(fonts, b"PDF4AllF"), &helvetica());
}

#[test]
fn stamp_refusals_carry_engine_ts_messages_and_record_nothing() {
    let (mut engine, mut session) = session_with(pages_pdf(&["one", "two", "three"]));
    let start = serialized(&session);
    for (request, reason, message) in [
        (
            json!({"kind": "banner", "text": "x"}),
            None,
            "stamp: kind must be one of watermark, header, footer",
        ),
        (
            json!({"kind": "header"}),
            None,
            "stamp: text must be 1..200 characters",
        ),
        (
            json!({"kind": "header", "text": ""}),
            None,
            "stamp: text must be 1..200 characters",
        ),
        (
            json!({"kind": "header", "text": "x".repeat(201)}),
            None,
            "stamp: text must be 1..200 characters",
        ),
        (
            json!({"kind": "header", "text": "plain \u{1F600}"}),
            None,
            "stamp: \"\u{1F600}\" (U+1F600) is not a WinAnsi character",
        ),
        (
            json!({"kind": "header", "text": "a\u{1}"}),
            None,
            "stamp: \"\\u0001\" (U+1) is not a WinAnsi character",
        ),
        (
            json!({"kind": "header", "text": "a\u{7f}"}),
            None,
            "stamp: \"\u{7f}\" (U+7F) is not a WinAnsi character",
        ),
        (
            json!({"kind": "footer", "text": "x", "align": "centre"}),
            Some("invalid"),
            "stamp: invalid-align: align must be left or right, not \"centre\"",
        ),
        (
            json!({"kind": "footer", "text": "x", "align": null}),
            Some("invalid"),
            "stamp: invalid-align: align must be left or right, not null",
        ),
        (
            json!({"kind": "footer", "text": "x", "align": {"$number": "NaN"}}),
            Some("invalid"),
            "stamp: invalid-align: align must be left or right, not null",
        ),
        (
            json!({"kind": "footer", "text": "x", "align": [1.5, {"$$a": "b"}]}),
            Some("invalid"),
            "stamp: invalid-align: align must be left or right, not [1.5,{\"$a\":\"b\"}]",
        ),
        (
            json!({"kind": "footer", "text": "x", "batesPad": 0}),
            Some("invalid"),
            "stamp: invalid-bates: batesPad must be an integer in 1..15",
        ),
        (
            json!({"kind": "footer", "text": "x", "batesPad": 16, "batesStart": -1}),
            Some("invalid"),
            "stamp: invalid-bates: batesPad must be an integer in 1..15",
        ),
        (
            json!({"kind": "footer", "text": "{bates}", "batesStart": 1.5}),
            Some("invalid"),
            "stamp: invalid-bates: batesStart must be an integer in 0..999999999999999",
        ),
        (
            json!({"kind": "footer", "text": "{bates}", "batesStart": "100"}),
            Some("invalid"),
            "stamp: invalid-bates: batesStart must be an integer in 0..999999999999999",
        ),
        (
            json!({"kind": "footer", "text": "{bates}"}),
            Some("invalid"),
            "stamp: bates-start-required: the text has {bates} but no batesStart",
        ),
        (
            json!({"kind": "footer", "text": "{bates}", "batesStart": 999_999_999_999_998u64}),
            Some("invalid"),
            "stamp: invalid-bates: batesStart must be an integer with batesStart..batesStart+2 in 0..999999999999999",
        ),
        (
            json!({"kind": "footer", "text": "plain", "batesStart": 999_999_999_999_999u64, "pages": [0, 1]}),
            Some("invalid"),
            "stamp: invalid-bates: batesStart must be an integer with batesStart..batesStart+1 in 0..999999999999999",
        ),
        (
            json!({"kind": "header", "text": "x", "pages": []}),
            Some("invalid"),
            "stamp: pages must be 1..2000 page numbers",
        ),
        (
            json!({"kind": "header", "text": "x", "pages": null}),
            Some("invalid"),
            "stamp: pages must be 1..2000 page numbers",
        ),
        (
            json!({"kind": "header", "text": "x", "pages": [0, 3]}),
            Some("invalid"),
            "stamp: page 1 must be an integer in 0..2",
        ),
    ] {
        let mut request = request;
        request["op"] = json!("stamp");
        let reply = dispatch(&mut engine, &mut session, request.clone());
        assert_eq!(reply["ok"], false, "{request}");
        assert_eq!(reply["error"], message, "{request}");
        assert_eq!(
            reply.get("reason").and_then(Value::as_str),
            reason,
            "{request}"
        );
        assert_eq!(
            (&reply["revision"], &reply["dirty"]),
            (&json!(0), &json!(false))
        );
    }
    // The ceiling itself: pad 15 and start 10^15 - 1 on one page.
    let top = op(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "footer", "text": "{bates}", "batesStart": 999_999_999_999_999u64,
            "batesPad": 15, "pages": [0]}),
    );
    assert_eq!(
        top["bates"],
        json!({"from": 999_999_999_999_999u64, "to": 999_999_999_999_999u64})
    );
    assert!(last_stream(&session, 0).contains("(999999999999999) Tj"));
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), start);

    // Read-only documents: no document, signed, locked, no edit permission.
    let mut empty = AppSession::default();
    let reply = dispatch(
        &mut engine,
        &mut empty,
        json!({"op": "stamp", "kind": "header", "text": "x"}),
    );
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (&json!("no-document"), &json!("stamp: no document open"))
    );
    let signed = classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] /SigFlags 3 >> >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_vec(),
        b"<< /Type /Annot /Subtype /Widget /FT /Sig /T (s) /Rect [0 0 1 1] /P 3 0 R /V << /Type /Sig >> >>".to_vec(),
    ]);
    let (mut engine, mut session) = session_with(signed);
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "header", "text": "x"}),
    );
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (
            &json!("read-only"),
            &json!("stamp: this document is read-only")
        )
    );
    let (mut engine, mut session) = session_with(fixture("encrypted.pdf"));
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "header", "text": "x"}),
    );
    assert_eq!(
        reply["error"],
        "stamp: this document is read-only until it is unlocked"
    );
}

#[test]
fn a_page_written_into_kids_gets_its_own_object_and_odd_pages_never_panic() {
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [<< /Type /Page /MediaBox [0 0 300 300] /Contents 4 0 R >> 3 0 R 5 0 R 6 0 R 7 0 R] /Count 5 /Rotate 45 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources 12 /Contents 99 0 R /MediaBox [0 0 50] >>".to_vec(),
        stream("0 0 m 10 10 l S"),
        b"<< /Type /Page /Parent 2 0 R /Resources << /Font 7 >> /Contents [4 0 R 5 /x] /Rotate -450.7 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources 98 0 R /Contents 4 0 R /CropBox [500 500 600 600] /MediaBox [0 0 100 100] /UserUnit 2 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /PDF4AllF 3 0 R >> >> /Contents 3 >>".to_vec(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    let start = serialized(&session);
    for kind in ["watermark", "header", "footer"] {
        let reply = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "stamp", "kind": kind, "text": "Odd {page}/{pages} {label}", "align": "right"}),
        );
        assert_eq!(reply["ok"], true, "{kind}: {reply}");
        assert_eq!(reply["pageCount"], 5);
    }
    let doc = document(&session);
    let ids = session.doc.as_ref().unwrap().page_ids();
    assert!(ids.iter().all(Option::is_some), "{ids:?}");
    // Every stamped page names a stamp font it can draw with, except page 4,
    // whose own /PDF4AllF (a page object) is left as it was.
    for page in 0..4 {
        let res = doc.dict_get(page_dict(doc, page), b"Resources");
        let fonts = doc
            .dict_get(res.as_dict().unwrap(), b"Font")
            .as_dict()
            .unwrap();
        assert_eq!(
            doc.dict_get(fonts, b"PDF4AllF"),
            &helvetica(),
            "page {page}"
        );
    }
    let items = content_items(doc, 2);
    assert_eq!(items.len(), 3 * 2 + 3, "page 2 keeps its stream list");
    assert_eq!(
        &items[3..6],
        [
            Object::Reference(id(4)),
            Object::Integer(5),
            Object::name("x")
        ]
    );
    let _ = texts(&mut session);
    while dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"] == true {}
    assert_eq!(serialized(&session), start);
}

#[test]
fn random_stamps_undo_and_redo_restore_exact_states() {
    struct Rng(u64);
    impl Rng {
        fn next(&mut self, bound: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % bound
        }
    }
    let (mut engine, mut session) = session_with(pages_pdf(&["one", "two", "three", "four"]));
    let mut states = vec![serialized(&session)];
    let mut position = 0usize;
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    for _ in 0..60 {
        match rng.next(5) {
            0 | 1 => {
                let kind = ["watermark", "header", "footer"][rng.next(3) as usize];
                let pages: Vec<u64> = (0..1 + rng.next(3)).map(|_| rng.next(4)).collect();
                op(
                    &mut engine,
                    &mut session,
                    json!({"op": "stamp", "kind": kind, "text": "S {bates}", "batesStart": rng.next(50), "pages": pages}),
                );
                states.truncate(position + 1);
                states.push(serialized(&session));
                position += 1;
            }
            2 | 3 if position > 0 => {
                op(&mut engine, &mut session, json!({"op": "undo"}));
                position -= 1;
                assert_eq!(serialized(&session), states[position]);
            }
            _ if position + 1 < states.len() => {
                op(&mut engine, &mut session, json!({"op": "redo"}));
                position += 1;
                assert_eq!(serialized(&session), states[position]);
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------------------ batch-file

fn batch(
    engine: &mut AppEngine,
    session: &mut AppSession,
    bytes: Option<Vec<u8>>,
    action: Value,
) -> (Value, Vec<Vec<u8>>) {
    let request = json!({"id": 5, "op": "batch-file", "name": "x.pdf",
        "bytes": if bytes.is_some() { json!({"$buffer": 0}) } else { json!(null) }, "action": action});
    dispatch_with(engine, session, request, bytes.into_iter().collect())
}

fn footer() -> Value {
    json!({"kind": "footer", "text": "N-{bates}", "batesStart": 7, "batesPad": 3})
}

#[test]
fn batch_file_stamps_a_separate_document_and_leaves_the_session_alone() {
    let (mut engine, mut session) = session_with(pages_pdf(&["hello"]));
    op(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "header", "text": "Open doc"}),
    );
    let before = serialized(&session);
    let state = session.journal_state();
    let (reply, outputs) = batch(
        &mut engine,
        &mut session,
        Some(pages_pdf(&["one", "two", "three"])),
        footer(),
    );
    assert_eq!(
        reply,
        json!({"id": 5, "ok": true, "pages": 3, "pageCount": 3, "bytes": {"$buffer": 0},
            "bates": {"from": 7, "to": 9}})
    );
    let view = ViewDocument::open(outputs[0].clone(), "").unwrap();
    for (page, want) in ["N-007", "N-008", "N-009"].iter().enumerate() {
        let text = view.structured_text(page).unwrap().as_text();
        assert!(text.contains(want) && !text.contains("Open doc"), "{text}");
    }
    assert_eq!(serialized(&session), before);
    assert_eq!(session.journal_state(), state);
    // A page range, a watermark without {bates}, and no session document.
    let mut alone = AppSession::default();
    let action = json!({"kind": "watermark", "text": "DRAFT", "pages": " 3, 1 "});
    let (reply, outputs) = batch(
        &mut engine,
        &mut alone,
        Some(pages_pdf(&["a", "b", "c"])),
        action,
    );
    assert_eq!((&reply["pages"], reply.get("bates")), (&json!(2), None));
    let view = ViewDocument::open(outputs[0].clone(), "").unwrap();
    let marked: Vec<bool> = (0..3)
        .map(|page| {
            view.structured_text(page)
                .unwrap()
                .as_text()
                .contains("DRAFT")
        })
        .collect();
    assert_eq!(marked, [true, false, true]);
    assert!(alone.doc.is_none() && alone.journal_state().revision == 0);
    // A locked session document does not block it.
    let (mut engine, mut locked) = session_with(fixture("encrypted.pdf"));
    let (reply, _) = batch(&mut engine, &mut locked, Some(pages_pdf(&["a"])), footer());
    assert_eq!(reply["ok"], true, "{reply}");
}

#[test]
fn batch_file_refusals_are_replies_without_journal_fields() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let no_pages = b"%PDF-1.4\n1 0 obj <</Type/Catalog/Pages 2 0 R>> endobj\n2 0 obj <</Type/Pages/Kids[5 0 R]/Count 3>> endobj\ntrailer <</Root 1 0 R>>\n%%EOF\n".to_vec();
    let signed = classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /Perms << /DocMDP 4 0 R >> >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_vec(),
        b"<< /Type /Sig >>".to_vec(),
    ]);
    let nav = || Some(pages_pdf(&["one", "two", "three"]));
    let ceiling = MAX_PDF_BYTES;
    for (bytes, action, reason, detail, error) in [
        (None, footer(), "not-pdf", None, "batch-file: no bytes".to_string()),
        (Some(vec![0; ceiling + 1]), json!({"kind": "side"}), "too-large", None, format!("batch-file: over the {ceiling} byte ceiling")),
        (nav(), json!({"kind": "footer", "text": "{bates}"}), "stamp-error", Some("bates-start-required"), "stamp: bates-start-required: the text has {bates} but no batesStart".into()),
        (nav(), json!({"kind": "footer", "text": "{bates}", "batesStart": -1}), "stamp-error", Some("invalid-bates"), "stamp: invalid-bates: batesStart must be an integer in 0..999999999999999".into()),
        (nav(), json!({"kind": "footer", "text": "x", "align": "centre"}), "stamp-error", Some("invalid-align"), "stamp: invalid-align: align must be left or right, not \"centre\"".into()),
        (nav(), json!({"kind": "footer", "text": ""}), "stamp-error", Some("text"), "stamp: text must be 1..200 characters".into()),
        (nav(), json!({"kind": "side", "text": "x"}), "stamp-error", Some("text"), "stamp: kind must be one of watermark, header, footer".into()),
        (nav(), json!("not an object"), "stamp-error", Some("text"), "stamp: kind must be one of watermark, header, footer".into()),
        (Some(b"%PDF".to_vec()), footer(), "not-pdf", None, "batch-file: too few bytes for a PDF".into()),
        (Some(vec![b'A'; 100]), footer(), "not-pdf", None, String::new()),
        (Some(no_pages), footer(), "not-pdf", None, "batch-file: the file has no pages".into()),
        (Some(fixture("encrypted.pdf")), footer(), "encrypted", None, "batch-file: encrypted files are skipped".into()),
        (Some(fixture("encrypted-noedit.pdf")), footer(), "encrypted", None, "batch-file: encrypted files are skipped".into()),
        (Some(signed), footer(), "read-only", None, "batch-file: signed or certified files are read-only".into()),
        (nav(), json!({"kind": "footer", "text": "x", "pages": "9"}), "bad-pages", None, "batch-file: pages \"9\" do not fit 3 pages".into()),
        (nav(), json!({"kind": "footer", "text": "x", "pages": "x"}), "bad-pages", None, "batch-file: pages \"x\" do not fit 3 pages".into()),
        (nav(), json!({"kind": "footer", "text": "x", "pages": 2}), "bad-pages", None, "batch-file: pages 2 do not fit 3 pages".into()),
        (nav(), json!({"kind": "footer", "text": "x", "pages": null}), "bad-pages", None, "batch-file: pages null do not fit 3 pages".into()),
        (nav(), json!({"kind": "footer", "text": "{bates}", "batesStart": 999_999_999_999_998u64, "batesPad": 15}), "stamp-error", Some("invalid-bates"), "stamp: invalid-bates: batesStart must be an integer with batesStart..batesStart+2 in 0..999999999999999".into()),
    ] {
        let (reply, outputs) = batch(&mut engine, &mut session, bytes, action.clone());
        assert!(outputs.is_empty());
        let keys: Vec<&str> = reply.as_object().unwrap().keys().map(String::as_str).collect();
        let mut want = vec!["id", "ok", "reason"];
        want.extend(detail.map(|_| "detail"));
        want.push("error");
        assert_eq!(keys.iter().copied().collect::<std::collections::BTreeSet<_>>(),
            want.into_iter().collect(), "{action}: {reply}");
        assert_eq!((&reply["ok"], &reply["reason"]), (&json!(false), &json!(reason)), "{action}: {reply}");
        assert_eq!(reply.get("detail").and_then(Value::as_str), detail, "{reply}");
        if !error.is_empty() {
            assert_eq!(reply["error"], error, "{reply}");
        }
    }
    // An unreadable file names the parser's reason.
    let (reply, _) = batch(&mut engine, &mut session, Some(vec![b'A'; 100]), footer());
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .starts_with("batch-file: the file could not be read (")
    );
}

#[test]
fn damaged_batch_and_compare_inputs_are_refused_or_stamped_never_a_panic() {
    let source = pages_pdf(&["one", "two", "three"]);
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = |bound: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % bound as u64) as usize
    };
    for round in 0..150 {
        let mut bytes = source.clone();
        if round % 2 == 0 {
            bytes.truncate(next(bytes.len()));
        } else {
            for _ in 0..6 {
                let at = next(bytes.len());
                bytes[at] = next(256) as u8;
            }
        }
        let (reply, _) = batch(&mut engine, &mut session, Some(bytes.clone()), footer());
        assert!(
            reply["ok"] == true || reply["reason"].is_string(),
            "{reply}"
        );
        let request = json!({"op": "compare-open", "name": "b.pdf", "pdf": {"$buffer": 0}});
        let (reply, _) = dispatch_with(&mut engine, &mut session, request, vec![bytes]);
        if reply["ok"] == true {
            let _ = dispatch(
                &mut engine,
                &mut session,
                json!({"op": "compare-render", "page": 0, "scale": 0.25}),
            );
        }
    }
}

// ------------------------------------------------------------------ hostile pages

/// `pages` pages that all draw one shared /Contents array (object 4) of
/// `entries` references to an empty stream.
fn shared_contents_pdf(entries: usize, pages: usize) -> Vec<u8> {
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 6 + i)).collect();
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {pages} >>",
            kids.join(" ")
        )
        .into_bytes(),
        b"<< >>".to_vec(),
        format!("[{}]", "5 0 R ".repeat(entries)).into_bytes(),
        stream(""),
    ];
    for _ in 0..pages {
        objects.push(
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>".to_vec(),
        );
    }
    classic(&objects)
}

#[test]
fn a_shared_contents_array_is_copied_only_within_the_budget() {
    // 3,000 pages sharing 99,000 entries: under 1 MB, and 297 million
    // copied entries used to abort the WASM engine.
    let hostile = shared_contents_pdf(99_000, 3_000);
    assert!(hostile.len() < 1_000_000);
    let too_large =
        format!("stamp: the pages would list more than {MAX_STAMP_CONTENTS} content streams");
    let (mut engine, mut session) = session_with(hostile.clone());
    let state = session.journal_state();
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "footer", "text": "x"}),
    );
    assert_eq!(
        (&reply["ok"], &reply["reason"], &reply["error"]),
        (&json!(false), &json!("too-large"), &json!(too_large))
    );
    assert_eq!(session.journal_state(), state);
    let (reply, outputs) = batch(&mut engine, &mut session, Some(hostile), footer());
    assert_eq!(
        reply,
        json!({"id": 5, "ok": false, "reason": "too-large", "error": too_large})
    );
    assert!(outputs.is_empty());

    // Four pages of `edge + 1` shared entries pass the budget by 4; three fit.
    let edge = MAX_STAMP_CONTENTS / 4 - 2;
    let (mut engine, mut session) = session_with(shared_contents_pdf(edge + 1, 4));
    let state = session.journal_state();
    let shared = Object::Reference(id(4));
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "header", "text": "x"}),
    );
    assert_eq!(reply["reason"], "too-large", "{reply}");
    assert_eq!(session.journal_state(), state);
    op(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "header", "text": "x", "pages": [0, 1, 2]}),
    );
    let doc = document(&session);
    for page in 0..3 {
        let items = content_items(doc, page);
        assert_eq!(items.len(), edge + 3);
        assert!(
            items[1..=edge + 1]
                .iter()
                .all(|item| item == &Object::Reference(id(5)))
        );
    }
    assert_eq!(page_dict(doc, 3).get(b"Contents".as_slice()), Some(&shared));
    op(&mut engine, &mut session, json!({"op": "undo"}));
    let doc = document(&session);
    for page in 0..4 {
        assert_eq!(
            page_dict(doc, page).get(b"Contents".as_slice()),
            Some(&shared)
        );
    }
}

#[test]
fn the_stamp_font_joins_resources_inherited_through_a_direct_parent() {
    // The page's /Parent is a direct dictionary holding the page's fonts,
    // which MuPDF reads through /Parent (the view, as hayro, inherits only
    // along /Kids). A page-level /Resources would hide them from MuPDF.
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [4 0 R] /Count 1 >>".to_vec(),
        stream("BT /F1 12 Tf 72 72 Td (hi) Tj ET"),
        b"<< /Type /Page /Parent << /Type /Pages /Resources << /Font << /F1 5 0 R >> >> >> /MediaBox [0 0 200 200] /Contents 3 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".to_vec(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    let start = serialized(&session);
    op(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "footer", "text": "S"}),
    );
    let doc = document(&session);
    let page = doc.resolve_id(id(4)).as_dict().unwrap();
    assert!(page.get(b"Resources".as_slice()).is_none());
    let parent = page.get(b"Parent".as_slice()).unwrap().as_dict().unwrap();
    let resources = doc.dict_get(parent, b"Resources").as_dict().unwrap();
    let fonts = doc.dict_get(resources, b"Font").as_dict().unwrap();
    assert_eq!(fonts.get(b"F1".as_slice()), Some(&Object::Reference(id(5))));
    assert_eq!(doc.dict_get(fonts, b"PDF4AllF"), &helvetica());
    let all = texts(&mut session);
    assert!(all[0].contains("hi") && all[0].contains('S'), "{all:?}");
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), start);
}

#[test]
fn a_dictionary_the_brute_force_page_list_takes_for_a_page_is_not_stamped() {
    // No /Kids: every dictionary with /Contents is a page, so the Text
    // annotation (object 4) is listed after the page.
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 5 0 R /Annots [4 0 R] >>"
            .to_vec(),
        b"<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /Contents (my note) >>".to_vec(),
        stream("0 0 m 10 10 l S"),
    ];
    let bytes = classic(&objects);
    let (mut engine, mut session) = session_with(bytes.clone());
    assert_eq!(
        session.doc.as_ref().unwrap().page_ids(),
        [Some(id(3)), Some(id(4))]
    );
    let start = serialized(&session);
    let state = session.journal_state();
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "footer", "text": "x"}),
    );
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (
            &json!("invalid"),
            &json!("stamp: not-a-page: page 1 has a /Contents that is not content")
        )
    );
    assert_eq!(serialized(&session), start);
    assert_eq!(session.journal_state(), state);
    let (reply, outputs) = batch(&mut engine, &mut session, Some(bytes), footer());
    assert_eq!(
        (&reply["reason"], &reply["detail"]),
        (&json!("stamp-error"), &json!("not-a-page"))
    );
    assert!(outputs.is_empty());
    // The real page alone stamps, and the note keeps its text.
    op(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "footer", "text": "x", "pages": [0]}),
    );
    let doc = document(&session);
    let note = doc.resolve_id(id(4)).as_dict().unwrap();
    assert_eq!(
        note.get(b"Contents".as_slice()),
        Some(&Object::string(b"my note".to_vec()))
    );
    assert_eq!(content_items(doc, 0).len(), 3);
    // A /Type /Page dictionary keeps a /Contents that is not content between
    // the stamp's streams, as engine.ts keeps it.
    let junk = classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents (junk) >>".to_vec(),
    ]);
    let (mut engine, mut session) = session_with(junk);
    op(
        &mut engine,
        &mut session,
        json!({"op": "stamp", "kind": "footer", "text": "x"}),
    );
    let items = content_items(document(&session), 0);
    assert_eq!(items.len(), 3);
    assert_eq!(items[1], Object::string(b"junk".to_vec()));
}

#[test]
fn batch_file_refuses_a_result_past_the_save_ceiling() {
    // One unfiltered 51 MiB content stream: under the input ceiling, over
    // the save ceiling.
    let size = MAX_SAVE_BYTES + (1 << 20);
    let mut content = format!("<< /Length {size} >>\nstream\n").into_bytes();
    content.resize(content.len() + size, b' ');
    content.extend_from_slice(b"\nendstream");
    let bytes = classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>".to_vec(),
        content,
    ]);
    assert!(bytes.len() < MAX_PDF_BYTES);
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let (reply, outputs) = batch(&mut engine, &mut session, Some(bytes), footer());
    assert_eq!(
        reply,
        json!({"id": 5, "ok": false, "reason": "too-large",
            "error": format!("batch-file: the result would exceed the {MAX_SAVE_BYTES} byte save ceiling")})
    );
    assert!(outputs.is_empty());
    assert!(session.doc.is_none() && session.journal_state().revision == 0);
}

// ------------------------------------------------------------------ compare

fn compare_open(
    engine: &mut AppEngine,
    session: &mut AppSession,
    name: &str,
    bytes: Vec<u8>,
) -> Value {
    let request = json!({"id": 9, "op": "compare-open", "name": name, "pdf": {"$buffer": 0}});
    dispatch_with(engine, session, request, vec![bytes]).0
}

#[test]
fn compare_open_reports_the_pages_and_keeps_the_slot_through_open_and_close() {
    let (mut engine, mut session) = session_with(lines_pdf(&["main"]));
    let opened = compare_open(
        &mut engine,
        &mut session,
        "nav.pdf",
        pages_pdf(&["one", "two", "three"]),
    );
    assert_eq!(
        opened,
        json!({"id": 9, "ok": true, "name": "nav.pdf", "pageCount": 3,
            "pages": [{"width": 612.0, "height": 792.0}, {"width": 612.0, "height": 792.0},
                {"width": 612.0, "height": 792.0}]})
    );
    let debug = session.debug_json();
    assert_eq!(debug["compare"]["name"], "nav.pdf");
    assert_eq!(debug["compare"]["pageCount"], 3);
    let serial = debug["compare"]["serial"].clone();
    op(&mut engine, &mut session, json!({"op": "close"}));
    assert_eq!(
        session.debug_json()["compare"]["serial"],
        serial,
        "close keeps the slot"
    );
    open(&mut engine, &mut session, lines_pdf(&["again"]));
    assert_eq!(
        session.debug_json()["compare"]["serial"],
        serial,
        "open keeps the slot"
    );
    let closed = op(
        &mut engine,
        &mut session,
        json!({"id": 4, "op": "compare-close"}),
    );
    assert_eq!(closed, json!({"id": 4, "ok": true}));
    assert!(session.compare.is_none());
    assert_eq!(
        op(&mut engine, &mut session, json!({"op": "compare-close"}))["ok"],
        true
    );
}

#[test]
fn compare_open_refusals_keep_the_previous_slot() {
    let (mut engine, mut session) = session_with(lines_pdf(&["main"]));
    let main_bytes = session.source_bytes;
    assert_eq!(
        compare_open(&mut engine, &mut session, "b.pdf", lines_pdf(&["kept"]))["ok"],
        true
    );
    let serial = session.debug_json()["compare"]["serial"].clone();
    let ceiling = MAX_PDF_BYTES;
    for (name, bytes, reason, error) in [
        (json!(""), lines_pdf(&["x"]), None, "compare-open: missing document name".to_string()),
        (json!(5), lines_pdf(&["x"]), None, "compare-open: missing document name".into()),
        (json!("c.pdf"), b"%PDF".to_vec(), Some("invalid"), "compare-open: not a PDF (too few bytes)".into()),
        (json!("c.pdf"), vec![0; ceiling - main_bytes + 1], Some("too-large"), format!("compare-open: the two files together exceed the {ceiling} byte ceiling")),
        (json!("c.pdf"), vec![b'A'; 64], Some("invalid"), String::new()),
        (json!("c.pdf"), fixture("encrypted.pdf"), Some("encrypted"), "compare-open: an encrypted file cannot be compared".into()),
        (json!("c.pdf"), b"%PDF-1.4\n1 0 obj <</Type/Catalog/Pages 2 0 R>> endobj\n2 0 obj <</Type/Pages/Kids[]/Count 0>> endobj\ntrailer <</Root 1 0 R>>\n%%EOF\n".to_vec(), Some("invalid"), "compare-open: the file has no pages".into()),
    ] {
        let request = json!({"op": "compare-open", "name": name, "pdf": {"$buffer": 0}});
        let (reply, _) = dispatch_with(&mut engine, &mut session, request, vec![bytes]);
        assert_eq!(reply["ok"], false, "{reply}");
        assert_eq!(reply.get("reason").and_then(Value::as_str), reason, "{reply}");
        if !error.is_empty() {
            assert_eq!(reply["error"], error, "{reply}");
        }
        assert_eq!(session.debug_json()["compare"]["serial"], serial, "{reply}");
    }
    // The pair rule is the main document's source plus the compare file; at the
    // ceiling it opens, and an open past it closes the slot first.
    let request = json!({"op": "compare-open", "name": "c.pdf", "pdf": 7});
    let (reply, _) = dispatch_with(&mut engine, &mut session, request, vec![]);
    assert_eq!(reply["reason"], "invalid");
    let mut big = lines_pdf(&["big"]);
    big.resize(ceiling - main_bytes, b' ');
    assert_eq!(
        compare_open(&mut engine, &mut session, "big.pdf", big)["ok"],
        true
    );
    let garbage = vec![b'A'; 4096];
    let reply = open(&mut engine, &mut session, garbage);
    assert_eq!(
        (&reply["ok"], &reply["compareClosed"]),
        (&json!(false), &json!(true))
    );
    assert!(session.compare.is_none());
    assert_eq!(
        session.name.as_deref(),
        Some("a.pdf"),
        "the main document stays"
    );
}

#[test]
fn compare_render_draws_the_compare_document_as_render_draws_it() {
    let nav = pages_pdf(&["one", "two", "three"]);
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let empty = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "compare-render", "page": 0, "scale": 1}),
    );
    assert_eq!(
        (&empty["reason"], &empty["error"]),
        (
            &json!("no-compare-document"),
            &json!("compare-render: no compare document is open")
        )
    );
    assert_eq!(
        compare_open(&mut engine, &mut session, "nav.pdf", nav.clone())["ok"],
        true
    );
    let (reply, pixels) = dispatch_with(
        &mut engine,
        &mut session,
        json!({"id": 2, "op": "compare-render", "page": 2, "scale": 1}),
        vec![],
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(reply.get("revision").is_none());
    assert_eq!(reply["page"], 2);
    assert_eq!(reply["lines"][0]["text"], "three");
    let (mut other_engine, mut other) = session_with(nav);
    let (main, main_pixels) = dispatch_with(
        &mut other_engine,
        &mut other,
        json!({"op": "render", "page": 2, "scale": 1}),
        vec![],
    );
    assert_eq!(pixels, main_pixels);
    for key in [
        "width",
        "height",
        "pixelWidth",
        "pixelHeight",
        "text",
        "lines",
        "nonWhite",
    ] {
        assert_eq!(reply[key], main[key], "{key}");
    }
    for (request, error) in [
        (
            json!({"page": 3, "scale": 1}),
            "render: page 3 is out of range (0..2)",
        ),
        (
            json!({"page": 0.5, "scale": 1}),
            "render: page must be an integer",
        ),
        (
            json!({"page": 0, "scale": 5}),
            "render: scale must be between 0.25 and 4",
        ),
    ] {
        let mut request = request;
        request["op"] = json!("compare-render");
        let reply = dispatch(&mut engine, &mut session, request);
        assert_eq!(
            (&reply["error"], reply.get("reason")),
            (&json!(error), None)
        );
    }
}

#[test]
fn compare_text_diffs_lines_and_refuses_as_engine_ts() {
    let (mut engine, mut session) = session_with(lines_pdf(&["alpha", "beta", "gamma"]));
    let refused = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "compare-text", "pageA": 0, "pageB": 0}),
    );
    assert_eq!(refused["reason"], "no-compare-document");
    compare_open(
        &mut engine,
        &mut session,
        "b.pdf",
        lines_pdf(&["alpha", "gamma", "delta"]),
    );
    let diff = op(
        &mut engine,
        &mut session,
        json!({"id": 3, "op": "compare-text", "pageA": 0, "pageB": 0}),
    );
    assert_eq!(
        (&diff["same"], &diff["moved"], &diff["revision"]),
        (&json!(2), &json!(0), &json!(0))
    );
    assert_eq!((&diff["pageA"], &diff["pageB"]), (&json!(0), &json!(0)));
    assert_eq!(diff["removed"].as_array().unwrap().len(), 1);
    assert_eq!(diff["removed"][0]["text"], "beta");
    assert_eq!(diff["added"][0]["text"], "delta");
    // Whole points, y down: Helvetica 18 at baseline 792 - 710.
    let bbox: Vec<i64> = (diff["removed"][0]["bbox"].as_array().unwrap().iter())
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(bbox[0], 72);
    assert!(bbox[1] < 82 && bbox[3] > 82 && bbox[2] > 100, "{bbox:?}");
    for (request, reason, error) in [
        (
            json!({"pageA": 1, "pageB": 0}),
            "invalid",
            "compare-text: page must be an integer in 0..0",
        ),
        (
            json!({"pageA": 0, "pageB": 1}),
            "invalid",
            "compare-text: page must be an integer in 0..0",
        ),
        (
            json!({"pageA": 0}),
            "invalid",
            "compare-text: page must be an integer in 0..0",
        ),
    ] {
        let mut request = request;
        request["op"] = json!("compare-text");
        let reply = dispatch(&mut engine, &mut session, request);
        assert_eq!(
            (&reply["reason"], &reply["error"]),
            (&json!(reason), &json!(error))
        );
    }
    // A journal step stays the only step: compare ops write nothing.
    op(
        &mut engine,
        &mut session,
        json!({"op": "page-rotate", "page": 0, "by": 90}),
    );
    let state = session.journal_state();
    op(
        &mut engine,
        &mut session,
        json!({"op": "compare-text", "pageA": 0, "pageB": 0}),
    );
    dispatch_with(
        &mut engine,
        &mut session,
        json!({"op": "compare-render", "page": 0, "scale": 1}),
        vec![],
    );
    op(&mut engine, &mut session, json!({"op": "compare-close"}));
    assert_eq!(session.journal_state(), state);
    assert_eq!(session.doc.as_ref().unwrap().steps(), ["rotate page"]);

    // The main document's copy bit, its lock, and no main document at all.
    let mut entropy = [0u8; 32];
    entropy[0] = 1;
    let nocopy = write_document(
        &Document::open(&lines_pdf(&["secret"]), DocumentLimits::default()).unwrap(),
        DocumentWriteLimits::default(),
        WriteOptions {
            encryption: Encryption::Aes256 {
                user_password: b"",
                owner_password: b"own",
                permissions: 4,
            },
            entropy: Some(Entropy(entropy)),
            ..WriteOptions::APP
        },
    )
    .unwrap();
    let (mut engine, mut session) = session_with(nocopy);
    compare_open(&mut engine, &mut session, "b.pdf", lines_pdf(&["x"]));
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "compare-text", "pageA": 0, "pageB": 0}),
    );
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (
            &json!("read-only"),
            &json!("compare-text: this document does not permit copying")
        )
    );
    let rendered = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "compare-render", "page": 0, "scale": 1}),
    );
    assert_eq!(
        rendered["ok"], true,
        "compare-render needs no copy permission"
    );
    let (mut engine, mut session) = session_with(fixture("encrypted.pdf"));
    compare_open(&mut engine, &mut session, "b.pdf", lines_pdf(&["x"]));
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "compare-text", "pageA": 0, "pageB": 0}),
    );
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (
            &json!("read-only"),
            &json!("compare-text: document is encrypted, a password is required")
        )
    );
    op(&mut engine, &mut session, json!({"op": "close"}));
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "compare-text", "pageA": 0, "pageB": 0}),
    );
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (
            &json!("no-document"),
            &json!("compare-text: no document open")
        )
    );
}

#[test]
fn compare_text_refuses_a_page_of_more_than_2000_lines() {
    let lines: Vec<String> = (0..2001).map(|i| format!("l{i}")).collect();
    let content: String = (lines.iter().enumerate())
        .map(|(i, text)| {
            format!(
                "BT /F1 0.3 Tf 72 {} Td ({text}) Tj ET\n",
                780.0 - 0.36 * i as f64
            )
        })
        .collect();
    let many = classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        stream(&content),
    ]);
    let (mut engine, mut session) = session_with(many);
    compare_open(&mut engine, &mut session, "b.pdf", lines_pdf(&["x"]));
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "compare-text", "pageA": 0, "pageB": 0}),
    );
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (
            &json!("too-many"),
            &json!("compare-text: a page has more than 2000 lines")
        )
    );
}
