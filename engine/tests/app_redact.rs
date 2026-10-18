// SPDX-License-Identifier: AGPL-3.0-or-later
//! Redaction marks on the app session: redact-add, redact-set,
//! redact-search and redact-preview as the earlier app engine answers them (refusals in
//! its order, nothing journaled by a refusal, undo and redo exact), the
//! Redact annotations with their dictionaries and outlines, and the content
//! map that tells which glyphs, paths, images, Forms and annotations a mark
//! touches.
use serde_json::{Value, json};
use tandem_pdf_engine::app::redact::content::{self, Content, Detail, DrawKind, Limits, WalkError};
use tandem_pdf_engine::app::redact::marks;
use tandem_pdf_engine::app::redact::region::{self, GlyphRule, Regions};
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::content::{ContentLimits, ContentParser};
use tandem_pdf_engine::document::Document;
use tandem_pdf_engine::object::{Object, ObjectId, decode_text_string};
use tandem_pdf_engine::view::RenderOptions;
use tandem_pdf_engine::writer::{DocumentWriteLimits, WriteOptions, write_document};

fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("../fixtures/{name}")).unwrap()
}

/// A classic-xref PDF from object bodies numbered from 1.
fn classic(objects: &[String]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
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

fn stream(dict: &str, content: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{content}\nendstream",
        content.len()
    )
}

/// One page (object 3, `page` entries added) drawing `content` (object 4)
/// with `resources`; more objects from 5.
fn page_with(page: &str, resources: &str, content: &str, more: &[String]) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] {page} /Resources << {resources} >> /Contents 4 0 R >>"
        ),
        stream("", content),
    ];
    objects.extend(more.iter().cloned());
    classic(&objects)
}

fn page(resources: &str, content: &str, more: &[String]) -> Vec<u8> {
    page_with("", resources, content, more)
}

/// Helvetica with /Widths for A (667), B (667), space (278) and a (556).
fn helv_widths() -> String {
    let mut widths = vec!["0".to_string(); 66];
    widths[0] = "278".into();
    widths[65 - 32] = "667".into();
    widths[66 - 32] = "667".into();
    widths[97 - 32] = "556".into();
    format!(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 97 /Widths [{}] >>",
        widths.join(" ")
    )
}

const HELV: &str =
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";

fn session_with(bytes: Vec<u8>) -> (AppEngine, AppSession) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(&mut session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
    (engine, session)
}

fn op(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    engine.dispatch(session, request, Vec::new()).0
}

fn ok(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    let reply = op(engine, session, request.clone());
    assert_eq!(reply["ok"], true, "{request} -> {reply}");
    reply
}

fn document(session: &AppSession) -> &Document<'static> {
    session.doc.as_ref().unwrap().document()
}

/// Every object written, for exact before/after comparisons.
fn serialized(session: &AppSession) -> Vec<u8> {
    let options = WriteOptions {
        garbage: false,
        ..WriteOptions::APP
    };
    write_document(document(session), DocumentWriteLimits::default(), options).unwrap()
}

fn marks_of(engine: &mut AppEngine, session: &mut AppSession, page: usize) -> Vec<Value> {
    let listed = ok(engine, session, json!({"op": "annots", "page": page}));
    let items = listed["items"].as_array().cloned().unwrap_or_default();
    items
        .into_iter()
        .filter(|item| item["type"] == "Redact")
        .collect()
}

fn numbers(value: &Value) -> Vec<f64> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect()
}

fn near(actual: &[f64], expected: &[f64], tolerance: f64) {
    assert_eq!(actual.len(), expected.len(), "{actual:?} vs {expected:?}");
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() <= tolerance, "{actual:?} vs {expected:?}");
    }
}

fn object_numbers(doc: &Document<'_>, value: &Object) -> Vec<f64> {
    doc.resolve(value)
        .as_array()
        .unwrap()
        .iter()
        .map(|v| doc.resolve(v).as_f64().unwrap())
        .collect()
}

fn refused(reply: &Value, reason: &str) {
    assert_eq!(reply["ok"], false, "{reply}");
    assert_eq!(reply["reason"], reason, "{reply}");
}

/// A refusal leaves every object and the journal as they were.
fn unchanged(
    engine: &mut AppEngine,
    session: &mut AppSession,
    request: Value,
    reason: &str,
) -> Value {
    let before = serialized(session);
    let state = session.journal_state();
    let reply = op(engine, session, request);
    refused(&reply, reason);
    assert_eq!(serialized(session), before);
    assert_eq!(session.journal_state(), state);
    reply
}

fn full(session: &mut AppSession, page: usize) -> Result<Content, WalkError> {
    full_with(session, page, Limits::DEFAULT)
}

fn full_with(session: &mut AppSession, page: usize, limits: Limits) -> Result<Content, WalkError> {
    let (view, doc) = session.doc.as_mut().unwrap().view_with_document().unwrap();
    region::page_content(doc, Some(view), page, Detail::Full, limits)
}

// ---------------------------------------------------------------- redact-add

#[test]
fn an_area_mark_is_one_step_with_its_dictionary_and_an_outline() {
    let (mut engine, mut session) = session_with(page("", "", &[]));
    let reply = ok(
        &mut engine,
        &mut session,
        json!({"id": 7, "op": "redact-add", "page": 0, "rect": [-20, 20, 80, 80]}),
    );
    assert_eq!(
        reply,
        json!({"id": 7, "ok": true, "added": 1, "pages": [0], "revision": 1, "dirty": true,
            "canUndo": true, "canRedo": false})
    );
    let doc = document(&session);
    let annot = doc.resolve_id(id(5)).as_dict().unwrap().clone();
    let keys: Vec<&[u8]> = annot.keys().map(Vec::as_slice).collect();
    assert_eq!(
        keys,
        [b"Type".as_slice(), b"Subtype", b"P", b"F", b"Rect", b"AP"]
    );
    assert_eq!(
        doc.dict_get(&annot, b"Subtype").as_name(),
        Some(b"Redact".as_slice())
    );
    assert_eq!(annot.get(b"P".as_slice()), Some(&Object::Reference(id(3))));
    assert_eq!(annot.get(b"F".as_slice()), Some(&Object::Integer(4)));
    // An area mark keeps the user's rect (clipped to the page).
    assert_eq!(
        object_numbers(doc, annot.get(b"Rect".as_slice()).unwrap()),
        [0.0, 712.0, 80.0, 772.0]
    );
    // The appearance is numbered after the annotation: one closed outline,
    // stroked, nothing filled.
    let ap = doc.dict_get(&annot, b"AP").as_dict().unwrap();
    assert_eq!(ap.get(b"N".as_slice()), Some(&Object::Reference(id(6))));
    let content = doc
        .decoded_stream(id(6), tandem_pdf_engine::stream::StreamLimits::default())
        .unwrap();
    let mut parser = ContentParser::new(&content, ContentLimits::default());
    let mut operators = Vec::new();
    while let Some(operation) = parser.next_operation().expect("the content parses") {
        operators.push(String::from_utf8(operation.operator).unwrap());
    }
    let count = |names: &[&str]| {
        operators
            .iter()
            .filter(|o| names.contains(&o.as_str()))
            .count()
    };
    assert_eq!(count(&["m", "re"]), 1, "one path: {operators:?}");
    assert_eq!(
        count(&["f", "F", "f*", "B", "B*", "b", "b*"]),
        0,
        "{operators:?}"
    );
    assert_eq!(count(&["S", "s"]), 1, "{operators:?}");
    let closed = count(&["re", "s", "h"]) > 0;
    assert!(closed, "a closed outline: {operators:?}");
    // Drawn: inside /Rect, along each of its edges, in a conspicuous colour.
    let image = session
        .view()
        .unwrap()
        .render(0, 2.0, RenderOptions::default())
        .unwrap();
    let mut ink: Option<[u32; 4]> = None;
    let mut colours: std::collections::HashMap<[u8; 3], usize> = Default::default();
    for y in 0..image.height {
        for x in 0..image.width {
            let i = ((y * image.width + x) * 4) as usize;
            let p = [image.pixels[i], image.pixels[i + 1], image.pixels[i + 2]];
            if p.iter().any(|&c| c < 250) {
                let b = ink.get_or_insert([x, y, x, y]);
                *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
                *colours.entry(p).or_default() += 1;
            }
        }
    }
    // Page space [0 20 80 80] at 2 px per point.
    let ink = ink.expect("the outline draws");
    let rect = [0.0, 40.0, 160.0, 160.0];
    for i in 0..4 {
        let (got, want) = (f64::from(ink[i]), rect[i]);
        let inside = if i < 2 {
            got >= want - 1.0
        } else {
            got <= want + 1.0
        };
        assert!(
            inside && (got - want).abs() <= 4.0 + 1.0,
            "{ink:?} vs {rect:?}"
        );
    }
    let colour = colours.into_iter().max_by_key(|&(p, n)| (n, p)).unwrap().0;
    let spread = colour.iter().max().unwrap() - colour.iter().min().unwrap();
    assert!(
        spread > 100,
        "a colour, not white, grey or black: {colour:?}"
    );
    let doc = document(&session);
    let page = doc.resolve_id(id(3)).as_dict().unwrap();
    assert_eq!(
        doc.dict_get(page, b"Annots").as_array().unwrap(),
        [Object::Reference(id(5))]
    );
    let listed = marks_of(&mut engine, &mut session, 0);
    assert_eq!(listed.len(), 1);
    assert_eq!(numbers(&listed[0]["rect"]), [0.0, 20.0, 80.0, 80.0]);
    assert_eq!(listed[0].get("search"), None);
    assert_eq!(listed[0]["editable"], false);

    let after = serialized(&session);
    assert_eq!(
        ok(&mut engine, &mut session, json!({"op": "undo"}))["step"],
        "add redaction mark"
    );
    assert!(marks_of(&mut engine, &mut session, 0).is_empty());
    ok(&mut engine, &mut session, json!({"op": "redo"}));
    assert_eq!(serialized(&session), after);
}

#[test]
fn add_refusals_write_nothing() {
    let (mut engine, mut session) = session_with(page("", "", &[]));
    for rect in [
        json!([1, 1, 1.5, 4]),
        json!([700, 10, 800, 20]),
        json!([1, 2, 3]),
    ] {
        let reply = unchanged(
            &mut engine,
            &mut session,
            json!({"op": "redact-add", "page": 0, "rect": rect}),
            "invalid",
        );
        assert_eq!(reply.get("detail"), None);
    }
    let reply = unchanged(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [1, 2, {"$number": "NaN"}, 4]}),
        "invalid",
    );
    assert_eq!(
        reply["error"],
        "redact-add: rect must be four finite numbers"
    );
    let reply = unchanged(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [1, 1, 1.5, 4]}),
        "invalid",
    );
    assert_eq!(
        reply["error"],
        "redact-add: the clamped rect must be at least 1 by 1 point"
    );
    let reply = unchanged(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 1, "rect": [10, 10, 40, 40]}),
        "invalid",
    );
    assert_eq!(
        reply["error"],
        "redact-add: page must be an integer in 0..0"
    );
    // The rect is clamped to the page, not refused.
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [600, 780, 700, 900]}),
    );
    assert_eq!(
        numbers(&marks_of(&mut engine, &mut session, 0)[0]["rect"]),
        [600.0, 780.0, 612.0, 792.0]
    );
    let (mut engine, mut session) = session_with(fixture("p0b/signed-intact.pdf"));
    unchanged(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [10, 10, 40, 40]}),
        "read-only",
    );
}

/// `count` Redact annotations on one page (objects from 5).
fn many_marks(count: usize) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 {} 0 R >> >> /Contents 4 0 R /Annots [{}] >>",
            count + 5,
            (0..count)
                .map(|i| format!("{} 0 R", i + 5))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        stream("", "BT /F1 12 Tf 72 700 Td (hit hit) Tj ET"),
    ];
    for i in 0..count {
        objects.push(format!(
            "<< /Type /Annot /Subtype /Redact /Rect [10 {} 20 {}] >>",
            i % 700,
            i % 700 + 5
        ));
    }
    objects.push(HELV.to_string());
    classic(&objects)
}

#[test]
fn the_mark_cap_counts_the_whole_document() {
    let (mut engine, mut session) = session_with(many_marks(marks::MAX_REDACT_MARKS));
    let reply = unchanged(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [100, 100, 200, 150]}),
        "too-many",
    );
    assert_eq!(reply["error"], "redact-add: at most 2000 marks");
    unchanged(
        &mut engine,
        &mut session,
        json!({"op": "redact-search", "text": "hit"}),
        "too-many",
    );
    // One short of the cap: one hit fits, two do not.
    let (mut engine, mut session) = session_with(many_marks(marks::MAX_REDACT_MARKS - 1));
    unchanged(
        &mut engine,
        &mut session,
        json!({"op": "redact-search", "text": "hit"}),
        "too-many",
    );
    assert_eq!(
        ok(
            &mut engine,
            &mut session,
            json!({"op": "redact-search", "text": "hit hit"})
        )["added"],
        1
    );
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(preview["marks"], 2000);
}

// ---------------------------------------------------------------- redact-search

#[test]
fn search_marks_every_hit_with_quads_and_the_tag() {
    let (mut engine, mut session) = session_with(fixture("redact.pdf"));
    let reply = ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-search", "text": "P0B-SECRET-7731"}),
    );
    assert_eq!(reply["added"], 8);
    assert_eq!(reply["pages"], json!([0, 1]));
    assert_eq!(reply["revision"], 1);
    let listed = marks_of(&mut engine, &mut session, 0);
    assert_eq!(listed.len(), 7);
    assert!(listed.iter().all(|m| m["search"] == true));
    let doc = document(&session);
    let first = id(listed[0]["id"].as_u64().unwrap() as u32);
    let annot = doc.resolve_id(first).as_dict().unwrap();
    let keys: Vec<&[u8]> = annot.keys().map(Vec::as_slice).collect();
    assert_eq!(
        keys,
        [
            b"Type".as_slice(),
            b"Subtype",
            b"P",
            b"F",
            b"Rect",
            b"QuadPoints",
            b"PDF4AllSearch",
            b"AP"
        ]
    );
    // The visible hit's quad: x along the baseline from the advances (to
    // 0.05 pt), top and bottom within 0.35 em (12 pt text).
    let quads = object_numbers(doc, annot.get(b"QuadPoints".as_slice()).unwrap());
    assert_eq!(quads.len(), 8);
    let old = [
        106.672, 754.9, 216.704, 754.9, 106.672, 734.412, 216.704, 734.412,
    ];
    for (i, (got, want)) in quads.iter().zip(old).enumerate() {
        let tolerance = if i % 2 == 0 { 0.05 } else { 0.35 * 12.0 };
        assert!((got - want).abs() <= tolerance, "{quads:?}");
    }
    // /Rect holds the quads, grown by at most 2 pt on each side; the listed
    // rect is the same in page space.
    let xs = [quads[0], quads[2], quads[4], quads[6]];
    let ys = [quads[1], quads[3], quads[5], quads[7]];
    let min = |v: [f64; 4]| v.into_iter().fold(f64::INFINITY, f64::min);
    let max = |v: [f64; 4]| v.into_iter().fold(f64::NEG_INFINITY, f64::max);
    let union = [min(xs), min(ys), max(xs), max(ys)];
    let rect = object_numbers(doc, annot.get(b"Rect".as_slice()).unwrap());
    for i in 0..2 {
        assert!(
            rect[i] <= union[i] && rect[i] >= union[i] - 2.0,
            "{rect:?} {union:?}"
        );
        assert!(
            rect[i + 2] >= union[i + 2] && rect[i + 2] <= union[i + 2] + 2.0,
            "{rect:?} {union:?}"
        );
    }
    near(
        &numbers(&listed[0]["rect"]),
        &[rect[0], 792.0 - rect[3], rect[2], 792.0 - rect[1]],
        0.001,
    );
    let tag = doc.dict_get(annot, b"PDF4AllSearch").as_string().unwrap();
    assert_eq!(decode_text_string(tag), "P0B-SECRET-7731");
    assert_eq!(
        ok(&mut engine, &mut session, json!({"op": "undo"}))["step"],
        "mark redaction search hits"
    );
    assert!(marks_of(&mut engine, &mut session, 0).is_empty());
    assert!(marks_of(&mut engine, &mut session, 1).is_empty());
}

#[test]
fn search_refusals_write_nothing() {
    let (mut engine, mut session) = session_with(fixture("redact.pdf"));
    let reply = unchanged(
        &mut engine,
        &mut session,
        json!({"op": "redact-search", "text": "NO-SUCH-P4-TOKEN"}),
        "no-match",
    );
    assert_eq!(reply["error"], "redact-search: no match");
    for text in [
        json!(""),
        json!("a\u{7}b"),
        json!("x".repeat(257)),
        json!(5),
        json!(null),
    ] {
        let reply = unchanged(
            &mut engine,
            &mut session,
            json!({"op": "redact-search", "text": text}),
            "invalid",
        );
        assert_eq!(
            reply["error"],
            "redact-search: text must be 1..256 characters with no controls"
        );
    }
    // 256 UTF-16 units: an astral character counts twice.
    let long = format!("{}\u{1F600}", "x".repeat(254));
    unchanged(
        &mut engine,
        &mut session,
        json!({"op": "redact-search", "text": long}),
        "no-match",
    );
}

#[test]
fn search_finds_text_in_a_hidden_layer() {
    let content = "/OC /oc1 BDC BT /F1 12 Tf 72 700 Td (hidden secret) Tj ET EMC";
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [6 0 R] /D << /OFF [6 0 R] >> >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> /Properties << /oc1 6 0 R >> >> /Contents 4 0 R >>".into(),
        stream("", content),
        HELV.into(),
        "<< /Type /OCG /Name (Hidden) >>".into(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    // The ordinary search honours the layer; redaction's does not.
    assert_eq!(
        ok(
            &mut engine,
            &mut session,
            json!({"op": "search", "query": "secret"})
        )["total"],
        0
    );
    assert_eq!(
        ok(
            &mut engine,
            &mut session,
            json!({"op": "redact-search", "text": "secret"})
        )["added"],
        1
    );
}

#[test]
fn a_page_too_complex_to_search_refuses_instead_of_missing_its_hits() {
    // More than the view's 1 MiB of shown text bytes.
    let shown = "a".repeat((1 << 20) + 16);
    let content = format!("BT /F1 12 Tf 72 700 Td ({shown}) Tj ET");
    let (mut engine, mut session) =
        session_with(page("/Font << /F1 5 0 R >>", &content, &[HELV.into()]));
    let reply = unchanged(
        &mut engine,
        &mut session,
        json!({"op": "redact-search", "text": "a"}),
        "scan-budget",
    );
    assert_eq!(
        reply["error"],
        "redact-search: the text of page 1 is too complex to search; mark it by area"
    );
}

// ---------------------------------------------------------------- redact-set

#[test]
fn set_moves_an_area_mark_exactly_and_a_same_rect_is_a_noop() {
    let (mut engine, mut session) = session_with(fixture("p6/direct3.pdf"));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [300, 600, 400, 650]}),
    );
    let mark = marks_of(&mut engine, &mut session, 0)[0].clone();
    assert_eq!(numbers(&mark["rect"]), [300.0, 600.0, 400.0, 650.0]);
    let moved = ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-set", "page": 0, "annotId": mark["id"], "rect": [340, 600, 440, 650]}),
    );
    assert_eq!(moved.get("noop"), None);
    assert_eq!(moved["revision"], 2);
    near(
        &numbers(&marks_of(&mut engine, &mut session, 0)[0]["rect"]),
        &[340.0, 600.0, 440.0, 650.0],
        0.01,
    );
    let before = serialized(&session);
    let same = ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-set", "page": 0, "annotId": mark["id"], "rect": [340.004, 600, 440, 650]}),
    );
    assert_eq!(same["noop"], true);
    assert_eq!(same["revision"], 2);
    assert_eq!(serialized(&session), before);
    // Clamped before the no-op test and the write.
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-set", "page": 0, "annotId": mark["id"], "rect": [580, 600, 680, 650]}),
    );
    near(
        &numbers(&marks_of(&mut engine, &mut session, 0)[0]["rect"]),
        &[580.0, 600.0, 612.0, 650.0],
        0.01,
    );
    assert_eq!(
        ok(&mut engine, &mut session, json!({"op": "undo"}))["step"],
        "move redaction mark"
    );
    assert_eq!(serialized(&session), before);
}

#[test]
fn a_moved_search_mark_becomes_an_area_mark_and_undo_restores_it() {
    let (mut engine, mut session) = session_with(fixture("p6/direct3.pdf"));
    assert_eq!(
        ok(
            &mut engine,
            &mut session,
            json!({"op": "redact-search", "text": "Move me"})
        )["added"],
        1
    );
    let mark = marks_of(&mut engine, &mut session, 0)[0].clone();
    assert_eq!(mark["search"], true);
    let number = mark["id"].as_u64().unwrap() as u32;
    let before = serialized(&session);
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-set", "page": 0, "annotId": number, "rect": [70, 80, 100, 96]}),
    );
    let doc = document(&session);
    let annot = doc.resolve_id(id(number)).as_dict().unwrap();
    assert!(!annot.contains_key(b"QuadPoints".as_slice()));
    assert!(!annot.contains_key(b"PDF4AllSearch".as_slice()));
    let listed = marks_of(&mut engine, &mut session, 0);
    near(
        &numbers(&listed[0]["rect"]),
        &[70.0, 80.0, 100.0, 96.0],
        0.01,
    );
    assert_eq!(listed[0].get("search"), None);
    ok(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
    assert_eq!(marks_of(&mut engine, &mut session, 0)[0]["search"], true);
}

#[test]
fn set_refusals_are_tagged_and_write_nothing() {
    let (mut engine, mut session) = session_with(fixture("redact.pdf"));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [400, 300, 500, 320]}),
    );
    let mark = marks_of(&mut engine, &mut session, 0)[0]["id"].clone();
    let cases = [
        (
            json!({"annotId": mark, "rect": [1, 1, 1.5, 4]}),
            "rect",
            "redact-set: the clamped rect must be at least 1 by 1 point",
        ),
        (
            json!({"annotId": mark, "rect": [700, 10, 800, 20]}),
            "rect",
            "redact-set: the clamped rect must be at least 1 by 1 point",
        ),
        (
            json!({"annotId": mark, "rect": [1, 2, 3]}),
            "rect",
            "redact-set: rect must be four finite numbers",
        ),
        (
            json!({"annotId": 999999, "rect": [10, 10, 40, 40]}),
            "no-such-annot",
            "redact-set: no annotation 999999 on this page",
        ),
        (
            json!({"annotId": 1.5, "rect": [10, 10, 40, 40]}),
            "no-such-annot",
            "redact-set: id must be an integer",
        ),
        (
            json!({"annotId": 12, "rect": [10, 10, 40, 40]}),
            "not-redact",
            "redact-set: only a redaction mark can be moved",
        ),
    ];
    for (fields, detail, message) in cases {
        let mut request = json!({"op": "redact-set", "page": 0});
        request
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        let reply = unchanged(&mut engine, &mut session, request, "invalid");
        assert_eq!(reply["detail"], detail, "{reply}");
        assert_eq!(reply["error"], message);
    }
    // A bad page is checkPage's refusal, without a detail.
    let reply = unchanged(
        &mut engine,
        &mut session,
        json!({"op": "redact-set", "page": 9, "annotId": mark, "rect": [10, 10, 40, 40]}),
        "invalid",
    );
    assert_eq!(reply.get("detail"), None);
    let (mut engine, mut session) = session_with(fixture("p0b/signed-intact.pdf"));
    unchanged(
        &mut engine,
        &mut session,
        json!({"op": "redact-set", "page": 0, "annotId": 1, "rect": [10, 10, 40, 40]}),
        "read-only",
    );
}

#[test]
fn edit_batch_moves_and_deletes_marks_in_one_step() {
    let (mut engine, mut session) = session_with(fixture("hello.pdf"));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [100, 100, 200, 150]}),
    );
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [100, 200, 200, 250]}),
    );
    let listed = marks_of(&mut engine, &mut session, 0);
    let reply = ok(
        &mut engine,
        &mut session,
        json!({"op": "edit-batch", "page": 0, "items": [
            {"op": "redact-set", "annotId": listed[0]["id"], "rect": [110, 100, 210, 150]},
            {"op": "annot-delete", "annotId": listed[1]["id"]},
        ]}),
    );
    assert_eq!(reply["revision"], 3);
    let after = marks_of(&mut engine, &mut session, 0);
    assert_eq!(after.len(), 1);
    near(
        &numbers(&after[0]["rect"]),
        &[110.0, 100.0, 210.0, 150.0],
        0.01,
    );
    ok(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(marks_of(&mut engine, &mut session, 0).len(), 2);
}

// ---------------------------------------------------------------- redact-preview

#[test]
fn preview_counts_marks_shared_graphics_and_the_inventory() {
    let (mut engine, mut session) = session_with(fixture("redact.pdf"));
    let empty = ok(
        &mut engine,
        &mut session,
        json!({"id": 3, "op": "redact-preview"}),
    );
    assert_eq!(
        empty,
        json!({"id": 3, "ok": true, "marks": 0, "pages": [], "pageCounts": [], "shared": [],
            "sanitize": {"info": 1, "xmp": true, "attachments": 1, "outlines": 1, "annots": 2,
                "links": 1, "fields": 1, "layers": 1, "struct": 2},
            "inlineAlt": 0})
    );
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-search", "text": "P0B-SECRET-7731"}),
    );
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(preview["marks"], 8);
    assert_eq!(preview["pages"], json!([0, 1]));
    assert_eq!(
        preview["pageCounts"],
        json!([{"page": 0, "marks": 7}, {"page": 1, "marks": 1}])
    );
    assert_eq!(preview["shared"], json!([]));
    assert_eq!(preview["inlineAlt"], 1);

    let (mut engine, mut session) = session_with(fixture("redact-shared-only.pdf"));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [100, 320, 230, 350]}),
    );
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(
        preview["shared"],
        json!([{"page": 0, "kind": "form", "otherPages": [1]}])
    );
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 1, "rect": [100, 320, 230, 350]}),
    );
    assert_eq!(
        ok(&mut engine, &mut session, json!({"op": "redact-preview"}))["shared"],
        json!([])
    );

    let mut session = AppSession::default();
    refused(
        &op(&mut engine, &mut session, json!({"op": "redact-preview"})),
        "no-document",
    );
}

#[test]
fn shared_graphics_are_found_at_any_depth_and_in_appearances() {
    // Image 7 is drawn through Form 6 on page 1 and directly on page 2;
    // annotation 10 on page 2 draws Form 6 in its appearance.
    let image = stream(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
        "\u{0}",
    );
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Fm 6 0 R >> >> /Contents 5 0 R >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Im 7 0 R >> >> /Contents 8 0 R /Annots [10 0 R] >>".into(),
        stream("", "q 1 0 0 1 100 600 cm /Fm Do Q"),
        stream("/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /XObject << /Im 7 0 R >> >>", "q 100 0 0 100 0 0 cm /Im Do Q"),
        image,
        stream("", "q 50 0 0 50 300 300 cm /Im Do Q"),
        "null".into(),
        "<< /Type /Annot /Subtype /Stamp /Rect [400 400 500 500] /AP << /N 6 0 R >> >>".into(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    // Page space y = 792 - user y: the Form covers 100..200 by 92..192.
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [120, 120, 140, 140]}),
    );
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(
        preview["shared"],
        json!([
            {"page": 0, "kind": "form", "otherPages": [1]},
            {"page": 0, "kind": "image", "otherPages": [1]},
            {"page": 0, "kind": "image", "otherPages": [1]},
        ])
    );
    // Marking the page-2 image whole and the stamp: what page 2's mark
    // touches (all of the image) is drawn on page 1 with only part marked.
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 1, "rect": [290, 430, 360, 500]}),
    );
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 1, "rect": [380, 280, 520, 400]}),
    );
    assert_eq!(
        ok(&mut engine, &mut session, json!({"op": "redact-preview"}))["shared"],
        json!([{"page": 1, "kind": "image", "otherPages": [0]}])
    );
    // Covering the Form on page 1 too leaves nothing shared.
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [100, 92, 200, 192]}),
    );
    assert_eq!(
        ok(&mut engine, &mut session, json!({"op": "redact-preview"}))["shared"],
        json!([])
    );
}

#[test]
fn preview_refuses_a_marked_page_too_complex_to_check() {
    let content = "0 0 m 1 1 l S\n".repeat(20);
    let (mut engine, mut session) = session_with(page("", &content, &[]));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [10, 10, 40, 40]}),
    );
    let doc = document(&session);
    let tight = Limits {
        tokens: 30,
        ..Limits::DEFAULT
    };
    let all = marks::marks(doc);
    assert_eq!(
        region::shared(doc, None, &all, tight),
        Err(region::SharedError::TooComplex(0))
    );
    // An unreadable marked page refuses as invalid.
    let (mut engine, mut session) = session_with(page("", "BT (unterminated Tj ET", &[]));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [10, 10, 40, 40]}),
    );
    let reply = op(&mut engine, &mut session, json!({"op": "redact-preview"}));
    refused(&reply, "invalid");
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .starts_with("redact-preview: the content of page 1 cannot be read")
    );
}

#[test]
fn the_inventory_walks_trees_not_the_document() {
    // An outline item whose destination reaches the page tree, and a
    // structure element pointing at a page with an /Alt-carrying annotation:
    // neither page nor annotation is counted.
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R /Outlines 5 0 R /StructTreeRoot 7 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [9 0 R] >>".into(),
        stream("", ""),
        "<< /Type /Outlines /First 6 0 R /Last 6 0 R /Count 1 >>".into(),
        "<< /Title (One) /Parent 5 0 R /Dest [3 0 R /Fit] /Next 6 0 R >>".into(),
        "<< /Type /StructTreeRoot /K [8 0 R] >>".into(),
        "<< /Type /StructElem /S /Figure /P 7 0 R /Pg 3 0 R /Alt (picture) /E (exp) /K [<< /Type /OBJR /Obj 9 0 R >>] >>".into(),
        "<< /Type /Annot /Subtype /Square /Rect [0 0 10 10] /Alt (x) /Title (y) >>".into(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(
        preview["sanitize"],
        json!({"info": 0, "xmp": false, "attachments": 0, "outlines": 1, "annots": 1,
            "links": 0, "fields": 0, "layers": 0, "struct": 2})
    );
}

// ---------------------------------------------------------------- content map

fn glyph_xs(content: &Content) -> Vec<(f64, f64)> {
    content
        .draws
        .iter()
        .filter_map(|d| match &d.kind {
            DrawKind::Text(text) => Some(text.glyphs.iter().map(|g| (g.bounds[0], g.bounds[2]))),
            _ => None,
        })
        .flatten()
        .collect()
}

#[test]
fn glyphs_are_placed_from_widths_spacing_and_adjustments() {
    let content = "BT /F1 10 Tf 100 700 Td (AB) Tj 2 Tc [(A) -1000 (B)] TJ 50 Tz ( a) Tj ET";
    let (_, mut session) = session_with(page("/Font << /F1 5 0 R >>", content, &[helv_widths()]));
    let map = full(&mut session, 0).unwrap();
    assert_eq!(map.draws.len(), 3);
    let DrawKind::Text(first) = &map.draws[0].kind else {
        panic!("text")
    };
    assert!(first.measured);
    assert_eq!(first.glyphs.len(), 2);
    assert_eq!(first.glyphs[1].bytes, 1..2);
    assert_eq!(first.glyphs[1].code, u32::from(b'B'));
    // A at 100, 6.67 wide, 0.2 em below the baseline to 0.8 above.
    near(&first.glyphs[0].bounds, &[100.0, 84.0, 106.67, 94.0], 1e-9);
    assert_eq!(&content[map.draws[0].start..map.draws[0].op.end], "(AB) Tj");
    assert_eq!(
        &content[map.draws[1].start..map.draws[1].op.end],
        "[(A) -1000 (B)] TJ"
    );
    // After AB (13.34), with Tc 2: A, then 1000 units back... forward 10 pt.
    let xs = glyph_xs(&map);
    near(&[xs[2].0], &[113.34], 1e-9);
    near(&[xs[3].0], &[113.34 + 6.67 + 2.0 + 10.0], 1e-9);
    let DrawKind::Text(array) = &map.draws[1].kind else {
        panic!("text")
    };
    assert!(array.array);
    assert_eq!((array.glyphs[0].operand, array.glyphs[1].operand), (0, 2));
    assert!((array.glyphs[0].advance - 8.67).abs() < 1e-9);
    // Half horizontal scaling: the space (2.78 + 2) then a (5.56 + 2), halved.
    let start = 113.34 + 6.67 + 2.0 + 10.0 + 6.67 + 2.0;
    near(
        &[xs[4].0, xs[4].1, xs[5].0],
        &[start, start + 1.39, start + 2.39],
        1e-9,
    );
}

#[test]
fn fonts_without_widths_are_measured_by_the_view() {
    let content = "BT /F1 10 Tf 100 700 Td (AV) Tj ET";
    let (_, mut session) = session_with(page("/Font << /F1 5 0 R >>", content, &[HELV.into()]));
    let map = full(&mut session, 0).unwrap();
    let DrawKind::Text(text) = &map.draws[0].kind else {
        panic!("text")
    };
    assert!(text.measured);
    // Helvetica's A and V are both 667.
    near(
        &[text.glyphs[1].bounds[0], text.glyphs[1].bounds[2]],
        &[106.67, 113.34],
        1e-9,
    );
}

#[test]
fn two_byte_codes_use_the_cmap_and_w() {
    let font = "<< /Type /Font /Subtype /Type0 /BaseFont /X /Encoding /Identity-H /DescendantFonts [6 0 R] >>";
    let cid = "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /X /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 500 /W [1 [250 750]] >>";
    let content = "BT /F1 10 Tf 100 700 Td <000100020003> Tj ET";
    let (_, mut session) = session_with(page(
        "/Font << /F1 5 0 R >>",
        content,
        &[font.into(), cid.into()],
    ));
    let map = full(&mut session, 0).unwrap();
    let DrawKind::Text(text) = &map.draws[0].kind else {
        panic!("text")
    };
    assert!(text.measured);
    let spans: Vec<_> = text
        .glyphs
        .iter()
        .map(|g| (g.code, g.bytes.clone()))
        .collect();
    assert_eq!(spans, [(1, 0..2), (2, 2..4), (3, 4..6)]);
    let xs = glyph_xs(&map);
    near(&[xs[0].1, xs[1].1, xs[2].1], &[102.5, 110.0, 115.0], 1e-9);
}

#[test]
fn a_run_that_cannot_be_measured_gets_one_box_holding_it_and_slack_after_it() {
    // A Type 0 font whose CMap cannot be read: code lengths unknown.
    let font = "<< /Type /Font /Subtype /Type0 /BaseFont /X /Encoding /No-Such-CMap /DescendantFonts [6 0 R] >>";
    let cid = "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /X /DW 1000 >>";
    let content = "BT /F1 10 Tf 100 700 Td <00010002> Tj /F2 10 Tf (AB) Tj 0 -20 Td (A) Tj ET";
    let resources = "/Font << /F1 5 0 R /F2 7 0 R >>";
    let (_, mut session) = session_with(page(
        resources,
        content,
        &[font.into(), cid.into(), helv_widths()],
    ));
    let map = full(&mut session, 0).unwrap();
    let DrawKind::Text(unknown) = &map.draws[0].kind else {
        panic!("text")
    };
    assert!(!unknown.measured);
    assert!(unknown.glyphs.is_empty());
    // Four bytes, each at most 1000 units: 40 pt either way.
    near(&map.draws[0].bounds[..1], &[60.0], 1e-9);
    near(&map.draws[0].bounds[2..3], &[140.0], 1e-9);
    let DrawKind::Text(after) = &map.draws[1].kind else {
        panic!("text")
    };
    assert!(after.measured);
    assert_eq!(after.slack, 40.0);
    near(
        &[after.glyphs[0].bounds[0], after.glyphs[0].bounds[2]],
        &[60.0, 146.67],
        1e-9,
    );
    // A new line is placed exactly again.
    let DrawKind::Text(next) = &map.draws[2].kind else {
        panic!("text")
    };
    assert_eq!(next.slack, 0.0);
    near(&[next.glyphs[0].bounds[0]], &[100.0], 1e-9);
}

#[test]
fn paths_images_inline_images_shadings_and_forms_are_mapped() {
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Matrix [2 0 0 2 0 0] /Resources << /XObject << /Im 6 0 R >> >>",
        "/Span << /ActualText (x) /MCID 3 >> BDC 0 0 5 5 re f EMC q 10 0 0 10 0 0 cm /Im Do Q",
    );
    let image = stream(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
        "\u{0}",
    );
    let content = "10 w 100 100 m 200 100 l S\n50 50 100 100 re W n\n0 0 1000 1000 re f\nq 1 0 0 1 300 300 cm /Fm Do Q\nq 20 0 0 20 400 400 cm BI /W 2 /H 1 /CS /G /BPC 8 ID \u{1}\u{2} EI Q\nq 30 0 0 30 450 450 cm BI /W 1 /H 1 /F /AHx ID 00> EI Q\n/Sh sh";
    let resources = "/XObject << /Fm 5 0 R >> /Shading << /Sh 7 0 R >>";
    let shading = "<< /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 1 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >>";
    let (_, mut session) = session_with(page(resources, content, &[form, image, shading.into()]));
    let map = full(&mut session, 0).unwrap();
    let kinds: Vec<&str> = map
        .draws
        .iter()
        .map(|d| match d.kind {
            DrawKind::Text(_) => "text",
            DrawKind::Path { .. } => "path",
            DrawKind::Image {
                object: Some(_), ..
            } => "image",
            DrawKind::Image { object: None, .. } => "inline",
            DrawKind::Shading => "shading",
            DrawKind::Form { .. } => "form",
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "path", "path", "form", "path", "image", "inline", "inline", "shading"
        ]
    );
    // The stroke grows by half its width, times the miter limit's reach.
    let stroke = &map.draws[0];
    assert_eq!(stroke.start, 5);
    near(&stroke.bounds, &[50.0, 642.0, 250.0, 742.0], 1e-9);
    // The clip only clips: the fill keeps its own bounds and carries it.
    near(&map.draws[1].bounds, &[0.0, -208.0, 1000.0, 792.0], 1e-9);
    near(
        &map.draws[1].clip.unwrap(),
        &[50.0, 642.0, 150.0, 742.0],
        1e-9,
    );
    let DrawKind::Form {
        object, entered, ..
    } = map.draws[2].kind
    else {
        panic!("form")
    };
    assert_eq!((object, entered), (id(5), true));
    near(&map.draws[2].bounds, &[300.0, 472.0, 320.0, 492.0], 1e-9);
    let inner = &map.draws[3];
    assert_eq!(
        (inner.parent, inner.stream, inner.mcid, inner.alt),
        (Some(2), id(5), Some(3), true)
    );
    near(&inner.bounds, &[300.0, 482.0, 310.0, 492.0], 1e-9);
    assert_eq!(map.draws[4].parent, Some(2));
    near(&map.draws[4].bounds, &[300.0, 472.0, 320.0, 492.0], 1e-9);
    // Inline images end at EI, by size or by search.
    let bytes = map.streams.get(&id(4)).unwrap();
    for index in [5, 6] {
        let op = &bytes[map.draws[index].op.clone()];
        assert!(
            op.starts_with(b"BI") && op.ends_with(b"EI"),
            "{}",
            String::from_utf8_lossy(op)
        );
    }
    near(&map.draws[7].bounds, &[50.0, 642.0, 150.0, 742.0], 1e-9);
    assert_eq!(map.inline_alt, 1);
}

#[test]
fn a_self_invoking_form_is_drawn_once() {
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /Fm 5 0 R >> >>",
        "/Fm Do",
    );
    let (_, mut session) = session_with(page("/XObject << /Fm 5 0 R >>", "/Fm Do", &[form]));
    let map = full(&mut session, 0).unwrap();
    let entered: Vec<bool> = map
        .draws
        .iter()
        .map(|d| matches!(d.kind, DrawKind::Form { entered: true, .. }))
        .collect();
    assert_eq!(entered, [true, false]);
}

#[test]
fn walks_stop_at_their_limits_and_never_panic_on_garbage() {
    // Each level draws the next twice: 2^20 draws without a limit.
    let mut more = Vec::new();
    for level in 0..20u32 {
        let next = if level == 19 {
            String::new()
        } else {
            "/F Do /F Do".into()
        };
        let resources = if level == 19 {
            String::new()
        } else {
            format!("/Resources << /XObject << /F {} 0 R >> >>", level + 6)
        };
        more.push(stream(
            &format!("/Type /XObject /Subtype /Form /BBox [0 0 1 1] {resources}"),
            &next,
        ));
    }
    let (_, mut session) = session_with(page("/XObject << /F 5 0 R >>", "/F Do", &more));
    assert!(matches!(
        full(&mut session, 0),
        Err(WalkError::TooComplex(_))
    ));
    let small = Limits {
        draws: 100,
        ..Limits::DEFAULT
    };
    assert!(matches!(
        full_with(&mut session, 0, small),
        Err(WalkError::TooComplex(_))
    ));
    let garbage: [&[u8]; 9] = [
        b"BT (a) Tj ] ET",
        b"[ [ [ 1 2 Tj",
        b"<< /A [ >> Tj",
        b"BI /W 100000 /H 100000 /CS /RGB /BPC 8 ID xx",
        b"q q q Q Q Q Q Q Q cm cm 1 2 3 4 5 6 7 cm",
        b"BT /F9 1e308 Tf (aaaa) Tj 1e308 Tz [(a) 1e308 (b)] TJ ET",
        b"BDC EMC EMC /x BDC /y << /MCID 1.5 >> BDC",
        b"\xff\xfe\x00 Do sh /Nope Do",
        b"0 0 m 1e308 1e308 l 1e308 w S",
    ];
    for content in garbage {
        let content = String::from_utf8_lossy(content).into_owned();
        let (_, mut session) =
            session_with(page("/Font << /F9 5 0 R >>", &content, &[HELV.into()]));
        let _ = full(&mut session, 0);
    }
}

#[test]
fn marks_map_to_glyphs_and_annotations() {
    let (mut engine, mut session) = session_with(fixture("redact.pdf"));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-search", "text": "P0B-SECRET-7731"}),
    );
    // An area mark over the FreeText, the widget and the link.
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [100, 360, 200, 480]}),
    );
    let doc = document(&session);
    let all = marks::marks(doc);
    assert_eq!(all.len(), 9);
    assert_eq!(all[0].search.as_deref(), Some("P0B-SECRET-7731"));
    let regions = Regions::of(&all);
    let touched = region::annotations(doc, &regions);
    let kinds: Vec<&[u8]> = touched.iter().map(|a| a.subtype.as_slice()).collect();
    assert_eq!(kinds, [b"FreeText".as_slice(), b"Widget", b"Link"]);
    let map = full(&mut session, 0).unwrap();
    let doc = document(&session);
    let regions = Regions::of(&marks::marks(doc));
    // Every glyph of every shown "P0B-SECRET-7731" lies under a mark; the
    // first run's "visible " and " here" do not.
    let mut hidden = String::new();
    let mut kept = String::new();
    let bytes = map.streams.get(&id(6)).unwrap().clone();
    let first = &map.draws[0];
    let DrawKind::Text(text) = &first.kind else {
        panic!("text")
    };
    let operand = &bytes[first.start + 1..first.op.start - 2];
    for glyph in &text.glyphs {
        let c = char::from(operand[glyph.bytes.start]);
        if regions.touches(0, &glyph.bounds) {
            hidden.push(c);
        } else {
            kept.push(c);
        }
    }
    // The spaces beside the hit lie under the quads' two-point margin.
    assert_eq!(hidden.trim(), "P0B-SECRET-7731");
    assert_eq!(kept, "visiblehere");
    // Apply removes the hit; the spaces the margin partly hides stay.
    let rule = GlyphRule::new(doc, &marks::marks(doc));
    let under: String = text
        .glyphs
        .iter()
        .filter(|g| rule.removes(0, &g.bounds))
        .map(|g| char::from(operand[g.bytes.start]))
        .collect();
    assert_eq!(under, "P0B-SECRET-7731");
}

#[test]
fn count_alt_follows_the_regex() {
    assert_eq!(
        content::count_alt(b"/ActualText /Alt /E /E1 /Ex /Alt_ /Alt/E% /ActualTextX"),
        5
    );
}

#[test]
fn an_operation_split_across_contents_streams_keeps_its_operands() {
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents [4 0 R 5 0 R] >>".into(),
        stream("", "1 0 0 1 100 100"),
        stream("", "cm 0 0 10 10 re f"),
    ]);
    let (_, mut session) = session_with(bytes);
    let map = full(&mut session, 0).unwrap();
    assert!(map.split);
    assert_eq!(map.draws.len(), 1);
    near(&map.draws[0].bounds, &[100.0, 682.0, 110.0, 692.0], 1e-9);
    assert_eq!((map.draws[0].stream, map.draws[0].start), (id(5), 3));
}

#[test]
fn the_rect_index_answers_as_a_scan_would() {
    // A small linear congruential generator: deterministic rects.
    let mut seed = 12345u64;
    let mut next = |scale: f64| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as f64 / f64::from(1u32 << 31) * scale
    };
    let mut rect = |size: f64| {
        let (x, y) = (next(612.0), next(792.0));
        [x, y, x + next(size), y + next(size)]
    };
    let mut all = Vec::new();
    for i in 0..300 {
        let size = if i % 50 == 0 { 600.0 } else { 20.0 };
        all.push(marks::Mark {
            page: 0,
            id: None,
            rect: rect(size),
            quads: Vec::new(),
            search: None,
        });
    }
    let regions = Regions::of(&all);
    let rects: Vec<[f64; 4]> = all.iter().map(|m| m.rect).collect();
    for i in 0..3000 {
        // Small boxes (glyphs), points, and edges shared with a mark.
        let mut q = rect(if i % 3 == 0 { 0.0 } else { 40.0 });
        if i % 7 == 0 {
            let r = rects[i % rects.len()];
            q = [r[2], r[1], r[2] + 3.0, r[1] + 1.0];
        }
        let scan = rects.iter().any(|r| region::intersects(r, &q));
        assert_eq!(regions.touches(0, &q), scan, "{q:?}");
        let inside = rects.iter().any(|r| {
            r[0] - 0.01 <= q[0] && q[2] <= r[2] + 0.01 && r[1] - 0.01 <= q[1] && q[3] <= r[3] + 0.01
        });
        assert_eq!(regions.contains(0, &q), inside, "{q:?}");
        let meets =
            (rects.iter()).any(|r| q[0] <= r[2] && q[2] >= r[0] && q[1] <= r[3] && q[3] >= r[1]);
        assert_eq!(regions.meets(0, &q), meets, "{q:?}");
        let mut touching: Vec<[f64; 4]> = (rects.iter())
            .copied()
            .filter(|r| region::intersects(r, &q))
            .collect();
        let mut found = regions.touching(0, &q);
        let order = |a: &[f64; 4], b: &[f64; 4]| a.partial_cmp(b).unwrap();
        touching.sort_by(order);
        found.sort_by(order);
        assert_eq!(found, touching, "{q:?}");
        assert!(!regions.touches(1, &q));
    }
}

#[test]
fn an_inline_image_ends_at_the_ei_followed_by_content() {
    // Filtered data holding " EI " followed by binary bytes.
    let content =
        "q 10 0 0 10 50 50 cm BI /W 4 /H 1 /F /Fl ID x EI \u{1}\u{2}\u{3}\u{4} EI Q\n0 0 5 5 re f";
    let (_, mut session) = session_with(page("", content, &[]));
    let map = full(&mut session, 0).unwrap();
    assert_eq!(map.draws.len(), 2);
    let bytes = map.streams.get(&id(4)).unwrap();
    let image = &bytes[map.draws[0].op.clone()];
    assert!(
        image.ends_with(b"\x04 EI"),
        "{}",
        String::from_utf8_lossy(image)
    );
    assert!(matches!(
        map.draws[1].kind,
        DrawKind::Path { fill: true, .. }
    ));
}

// ------------------------------------------------- review regressions (stage A)

/// The characters (single-byte codes) of the glyphs on page 0 apply removes.
fn removed(session: &mut AppSession) -> String {
    let map = full(session, 0).unwrap();
    let doc = document(session);
    let rule = GlyphRule::new(doc, &marks::marks(doc));
    let mut out = String::new();
    for draw in &map.draws {
        if let DrawKind::Text(text) = &draw.kind {
            for glyph in &text.glyphs {
                if rule.removes(0, &glyph.bounds) {
                    out.push(char::from(glyph.code as u8));
                }
            }
        }
    }
    out
}

fn marked_session(content: &str, font: &str, marks: &[[f64; 4]]) -> (AppEngine, AppSession) {
    let (mut engine, mut session) =
        session_with(page("/Font << /F1 5 0 R >>", content, &[font.to_string()]));
    for rect in marks {
        ok(
            &mut engine,
            &mut session,
            json!({"op": "redact-add", "page": 0, "rect": rect}),
        );
    }
    (engine, session)
}

#[test]
fn apply_removes_every_glyph_inside_a_mark_however_small() {
    // Page space y = 792 - user y. A 3-point line under a 3.8-point mark.
    let (_, mut session) = marked_session(
        "BT /F1 3 Tf 100 600 Td (iiiiiiiiii) Tj ET",
        HELV,
        &[[99.0, 189.2, 200.0, 193.0]],
    );
    assert_eq!(removed(&mut session), "iiiiiiiiii");
    // An 8-point "i" (100 to 101.78) under a mark starting at 99.9.
    let (_, mut session) = marked_session(
        "BT /F1 8 Tf 100 700 Td (i) Tj ET",
        HELV,
        &[[99.9, 80.0, 300.0, 100.0]],
    );
    assert_eq!(removed(&mut session), "i");
    // Two-point text under a mark four points tall.
    let (_, mut session) = marked_session(
        "BT /F1 2 Tf 100 600 Td (SECRET) Tj ET",
        HELV,
        &[[99.0, 189.0, 120.0, 193.0]],
    );
    assert_eq!(removed(&mut session), "SECRET");
    // A narrow mark over most of the 2 in 1234: only the 2 goes.
    let (_, mut session) = marked_session(
        "BT /F1 10 Tf 100 600 Td (1234) Tj ET",
        HELV,
        &[[105.5, 182.0, 109.5, 194.0]],
    );
    assert_eq!(removed(&mut session), "2");
}

#[test]
fn apply_removes_glyphs_without_width_or_height_under_a_mark() {
    // B has width 0 and D is past /LastChar (MissingWidth 0).
    let font = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 65 /LastChar 67 /Widths [667 0 667] >>";
    let (_, mut session) = marked_session(
        "BT /F1 12 Tf 100 700 Td (AB) Tj (CD) Tj ET",
        font,
        &[[90.0, 70.0, 300.0, 110.0]],
    );
    assert_eq!(removed(&mut session), "ABCD");
    for content in [
        "BT /F1 0 Tf 100 700 Td (SECRET) Tj ET",
        "BT /F1 12 Tf 0 Tz 100 700 Td (SECRET) Tj ET",
        "BT /F1 12 Tf 0 0 0 0 100 700 Tm (SECRET) Tj ET",
    ] {
        let (_, mut session) = marked_session(content, HELV, &[[90.0, 70.0, 300.0, 110.0]]);
        assert_eq!(removed(&mut session), "SECRET", "{content}");
        // No mark there: nothing goes.
        let (_, mut session) = marked_session(content, HELV, &[[300.0, 300.0, 400.0, 400.0]]);
        assert_eq!(removed(&mut session), "", "{content}");
    }
}

#[test]
fn a_search_mark_removes_its_hit_at_any_size_and_past_the_page_edge() {
    for (size, x) in [("0.005", 100), ("0.5", 100), ("12", 100), ("12", 590)] {
        let content = format!("BT /F1 {size} Tf {x} 700 Td (xSECRETy) Tj ET");
        let (mut engine, mut session) =
            session_with(page("/Font << /F1 5 0 R >>", &content, &[HELV.into()]));
        ok(
            &mut engine,
            &mut session,
            json!({"op": "redact-search", "text": "SECRET"}),
        );
        // Tiny, x and y lie wholly under the mark's two-point margin; at 12
        // points the margin only partly hides them. Off the page edge, RETy
        // are not seen: the mark reaching the edge takes them.
        let expected = match (size, x) {
            ("12", 100) => "SECRET",
            ("12", _) => "SECRETy",
            _ => "xSECRETy",
        };
        assert_eq!(removed(&mut session), expected, "{size} at {x}");
    }
}

/// A page drawing `count` inline images whose `EI` text never follows.
fn inline_images(count: usize) -> String {
    "BI /F /AHx ID x EI \u{80}\n".repeat(count)
}

#[test]
fn inline_images_of_unknown_length_are_read_in_one_pass() {
    // 50,000 images (1.1 MB): the end search was quadratic.
    let (mut engine, mut session) = session_with(page("", &inline_images(50_000), &[]));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [10, 10, 600, 780]}),
    );
    let started = std::time::Instant::now();
    ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    let preview = started.elapsed();
    let map = full(&mut session, 0).unwrap();
    assert_eq!(map.draws.len(), 50_000);
    eprintln!("50,000 inline images: preview {preview:?}");
    assert!(preview.as_secs() < 10, "{preview:?}");
    // A Form whose image is followed by a megabyte of binary data, drawn
    // 20,000 times: each draw reads the megabyte again, so the walk stops
    // at its budget.
    let tail = "\u{1}".repeat(1 << 20);
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 10 10]",
        &format!("BI /F /AHx ID x EI \u{1}{tail}"),
    );
    let (mut engine, mut session) = session_with(page(
        "/XObject << /Fm 5 0 R >>",
        &"/Fm Do ".repeat(20_000),
        &[form],
    ));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [0, 700, 100, 792]}),
    );
    let started = std::time::Instant::now();
    let refused = op(&mut engine, &mut session, json!({"op": "redact-preview"}));
    let elapsed = started.elapsed();
    eprintln!("Form with a binary tail drawn 20,000 times: preview {elapsed:?}");
    assert_eq!(refused["reason"], "scan-budget", "{refused}");
    assert!(elapsed.as_secs() < 10, "{elapsed:?}");
}

/// `pages` pages sharing one /Annots array of `annots` stamps, all with one
/// appearance whose /XObject has `entries` names for the page's image.
fn appearance_fanout(pages: usize, annots: usize, entries: usize) -> Vec<u8> {
    let image = "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8";
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        String::new(),
        stream("", "q 100 0 0 100 100 100 cm /Im Do Q"),
        stream(image, "\u{0}"),
    ];
    let names: String = (0..entries).map(|i| format!("/x{i} 4 0 R ")).collect();
    objects.push(stream(
        &format!("/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << {names} >> >>"),
        "/x0 Do",
    ));
    let refs: String = (0..annots).map(|i| format!("{} 0 R ", 7 + i)).collect();
    objects.push(format!("[{refs}]"));
    for _ in 0..annots {
        objects.push(
            "<< /Type /Annot /Subtype /Stamp /Rect [500 500 510 510] /AP << /N 5 0 R >> >>".into(),
        );
    }
    let first = objects.len() + 1;
    for _ in 0..pages {
        objects.push("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Im 4 0 R >> >> /Contents 3 0 R /Annots 6 0 R >>".into());
    }
    let kids: String = (0..pages).map(|i| format!("{} 0 R ", first + i)).collect();
    objects[1] = format!("<< /Type /Pages /Kids [{kids}] /Count {pages} >>");
    classic(&objects)
}

#[test]
fn shared_reads_each_appearance_and_resource_tree_once() {
    let (mut engine, mut session) = session_with(appearance_fanout(300, 300, 65_537));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [90, 582, 210, 702]}),
    );
    let started = std::time::Instant::now();
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    let elapsed = started.elapsed();
    eprintln!("300 pages x 300 stamps: preview {elapsed:?}");
    assert!(elapsed.as_secs() < 10, "{elapsed:?}");
    // The stamps draw the image on every page (on page 1 untouched too),
    // and so do the other pages' contents.
    let shared = preview["shared"].as_array().unwrap();
    assert_eq!(shared.len(), 300 + 299);
    assert_eq!(
        shared[0],
        json!({"page": 0, "kind": "image", "otherPages": [0]})
    );
    assert_eq!(shared[598]["otherPages"], json!([299]));
}

/// Image 5 drawn on the page under the mark, and an annotation (a Stamp
/// away from the mark, or a Widget under it) whose appearance draws it.
fn annotation_drawing_the_image(direct: bool, widget: bool) -> Vec<u8> {
    let image = "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8";
    let (rect, subtype) = if widget {
        ("[100 100 200 200]", "/Widget /FT /Btn /T (b)")
    } else {
        ("[300 300 400 400]", "/Stamp")
    };
    let annot = format!("<< /Type /Annot /Subtype {subtype} /Rect {rect} /AP << /N 6 0 R >> >>");
    let listed = if direct {
        format!("/Annots [{annot}]")
    } else {
        "/Annots [7 0 R]".to_string()
    };
    let more = vec![
        stream(image, "\u{0}"),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /XObject << /Im 5 0 R >> >>",
            "q 100 0 0 100 0 0 cm /Im Do Q",
        ),
        annot,
    ];
    page_with(
        &listed,
        "/XObject << /Im 5 0 R >>",
        "q 100 0 0 100 100 100 cm /Im Do Q",
        &more,
    )
}

#[test]
fn shared_counts_direct_annotations_and_skips_the_widgets_apply_removes() {
    for (direct, widget) in [(false, false), (true, false), (false, true), (true, true)] {
        let (mut engine, mut session) = session_with(annotation_drawing_the_image(direct, widget));
        ok(
            &mut engine,
            &mut session,
            json!({"op": "redact-add", "page": 0, "rect": [90, 582, 210, 702]}),
        );
        let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
        // The stamp lies away from the mark and keeps drawing the image;
        // the widget lies under it and goes with the redaction (stage B),
        // so its appearance is no draw elsewhere.
        let expected = match widget {
            false => json!([{"page": 0, "kind": "image", "otherPages": [0]}]),
            true => json!([]),
        };
        assert_eq!(
            preview["shared"], expected,
            "direct {direct}, widget {widget}"
        );
    }
}

#[test]
fn a_shared_object_marked_in_different_places_is_shared() {
    // Form 5 shows SECRETNAME and harmless; pages 1 and 2 draw it alike.
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 300 100] /Resources << /Font << /F1 6 0 R >> >>",
        "BT /F1 12 Tf 10 60 Td (SECRETNAME) Tj 0 -40 Td (harmless) Tj ET",
    );
    let page_dict = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Fm 5 0 R >> >> /Contents 4 0 R >>";
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 7 0 R] /Count 2 >>".into(),
        page_dict.into(),
        stream("", "q 1 0 0 1 100 600 cm /Fm Do Q"),
        form,
        HELV.into(),
        page_dict.into(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    // SECRETNAME (page y 120..132) on page 1, harmless (160..172) on page 2.
    let secret = [105.0, 116.0, 200.0, 136.0];
    let harmless = [105.0, 156.0, 200.0, 176.0];
    for (page, rect) in [(0, secret), (1, harmless)] {
        ok(
            &mut engine,
            &mut session,
            json!({"op": "redact-add", "page": page, "rect": rect}),
        );
    }
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(
        preview["shared"],
        json!([
            {"page": 0, "kind": "form", "otherPages": [1]},
            {"page": 1, "kind": "form", "otherPages": [0]},
        ])
    );
    // The same marks on both pages: each covers what the other touches.
    for (page, rect) in [(0, harmless), (1, secret)] {
        ok(
            &mut engine,
            &mut session,
            json!({"op": "redact-add", "page": page, "rect": rect}),
        );
    }
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(preview["shared"], json!([]));
}

#[test]
fn a_mark_lands_only_on_its_page_when_pages_share_an_annots_array() {
    let page_dict = |contents: u32| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents {contents} 0 R /Annots 6 0 R >>"
        )
    };
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 8 0 R 9 0 R] /Count 3 >>".into(),
        page_dict(4),
        stream("", "BT /F1 12 Tf 100 700 Td (SECRET) Tj ET"),
        HELV.into(),
        "[7 0 R]".into(),
        "<< /Type /Annot /Subtype /Square /Rect [10 10 20 20] >>".into(),
        page_dict(10),
        page_dict(4),
        stream("", "BT /F1 12 Tf 100 600 Td (nothing) Tj ET"),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    let added = ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [10, 10, 100, 100]}),
    );
    assert_eq!(added["pages"], json!([0]));
    let pages_of = |session: &AppSession| -> Vec<usize> {
        marks::marks(document(session))
            .iter()
            .map(|m| m.page)
            .collect()
    };
    assert_eq!(pages_of(&session), [0]);
    // Every page still lists the Square.
    for page in 0..3 {
        let listed = ok(
            &mut engine,
            &mut session,
            json!({"op": "annots", "page": page}),
        );
        assert!(
            listed["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|i| i["type"] == "Square"),
            "page {page}"
        );
    }
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(preview["marks"], 1);
    assert_eq!(preview["pages"], json!([0]));
    // The search finds pages 1 and 3 (one content stream): each its own mark.
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-search", "text": "SECRET"}),
    );
    assert_eq!(pages_of(&session), [0, 0, 2]);
    ok(&mut engine, &mut session, json!({"op": "undo"}));
    ok(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
}

#[test]
fn a_marked_page_with_mismatched_brackets_drops_the_broken_operand() {
    let content = "BT /F1 12 Tf 72 700 Td (Hello secret) Tj ET [ 1 2 >> 0 0 m 10 10 l S";
    let (mut engine, mut session) =
        session_with(page("/Font << /F1 5 0 R >>", content, &[HELV.into()]));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [60, 80, 200, 100]}),
    );
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(preview["marks"], 1);
    // The broken operand is dropped: the path runs from 0,0 to 10,10
    // (stroked with miter joins: grown by half the width times 10).
    let map = full(&mut session, 0).unwrap();
    let path = map.draws.last().unwrap();
    assert!(matches!(path.kind, DrawKind::Path { stroke: true, .. }));
    near(&path.bounds, &[-5.0, 777.0, 15.0, 797.0], 1e-9);
}

#[test]
fn fonts_are_read_once_per_walk_and_their_reading_is_charged() {
    // A direct Type 0 font whose descendant has 65,536 widths, set 20,000 times.
    let descendant = "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /X /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /W [0 65535 500] >>";
    let direct = "/Font << /F1 << /Type /Font /Subtype /Type0 /BaseFont /X /Encoding /Identity-H /DescendantFonts [5 0 R] >> >>";
    let content = format!("BT {}<0001> Tj ET", "/F1 12 Tf ".repeat(20_000));
    let (_, mut session) = session_with(page(direct, &content, &[descendant.into()]));
    let started = std::time::Instant::now();
    let map = full(&mut session, 0).unwrap();
    let elapsed = started.elapsed();
    eprintln!("20,000 Tf of a direct Type 0 font: {elapsed:?}");
    assert!(elapsed.as_secs() < 10, "{elapsed:?}");
    let DrawKind::Text(text) = &map.draws[0].kind else {
        panic!("text")
    };
    assert_eq!(text.glyphs.len(), 1);
    // 130 fonts with their own 65,536 widths pass the walk's token budget.
    let mut more = Vec::new();
    let mut fonts = String::new();
    let mut shows = String::new();
    for i in 0..130 {
        more.push(format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /X /Encoding /Identity-H /DescendantFonts [{} 0 R] >>",
            6 + 2 * i
        ));
        more.push(descendant.to_string());
        fonts.push_str(&format!("/F{i} {} 0 R ", 5 + 2 * i));
        shows.push_str(&format!("/F{i} 12 Tf <0001> Tj "));
    }
    let (_, mut session) = session_with(page(
        &format!("/Font << {fonts} >>"),
        &format!("BT {shows}ET"),
        &more,
    ));
    assert!(matches!(
        full(&mut session, 0),
        Err(WalkError::TooComplex(_))
    ));
}

#[test]
fn many_touched_draws_of_one_object_are_compared_by_what_their_marks_touch() {
    // Page 1 draws the image 3,000 times under one mark; page 2 once,
    // half under a mark.
    let image = "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8";
    let mut tiles = String::new();
    for i in 0..3000 {
        let (x, y) = (50 + (i % 60) * 8, 100 + (i / 60) * 8);
        tiles.push_str(&format!("q 6 0 0 6 {x} {y} cm /Im Do Q\n"));
    }
    let page_dict = |contents: u32| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Im 5 0 R >> >> /Contents {contents} 0 R >>"
        )
    };
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 2 >>".into(),
        page_dict(4),
        stream("", &tiles),
        stream(image, "\u{0}"),
        page_dict(7),
        stream("", "q 100 0 0 100 100 100 cm /Im Do Q"),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [40, 280, 560, 700]}),
    );
    // Page 2's image (page y 592..692) half marked: what page 1's marks
    // touch (the whole image) shows there; what page 2's touch (its left
    // half) is under page 1's mark everywhere.
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 1, "rect": [90, 580, 150, 700]}),
    );
    let started = std::time::Instant::now();
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    let elapsed = started.elapsed();
    eprintln!("3,000 touched draws of one image: preview {elapsed:?}");
    assert!(elapsed.as_secs() < 10, "{elapsed:?}");
    assert_eq!(
        preview["shared"],
        json!([{"page": 0, "kind": "image", "otherPages": [1]}])
    );
    // Page 2's image marked whole: every draw covers every other.
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 1, "rect": [90, 580, 210, 700]}),
    );
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(preview["shared"], json!([]));
}
