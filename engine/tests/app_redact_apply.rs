// SPDX-License-Identifier: MIT
//! redact-apply on the app session: what the marks cover leaves the copy
//! (text, image pixels, paths, annotations, alternate texts), the session
//! is unchanged, the refusals come in the earlier app engine's order, and the copy is
//! checked by inspection: its text (the view's and the walk's), its image
//! samples, its drawing outside the marks, and every object it holds.
use serde_json::{Value, json};
use tandem_pdf_engine::app::redact::content::{Content, Detail, DrawKind, Limits};
use tandem_pdf_engine::app::redact::region;
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Object, ObjectId};
use tandem_pdf_engine::stream::{self, Filter, StreamLimits};
use tandem_pdf_engine::view::{RenderOptions, ViewDocument};
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
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend(
            body.chars()
                .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')),
        );
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

/// A stream object; `content` is Latin-1 (each char one byte).
fn stream(dict: &str, content: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{content}\nendstream",
        content.chars().count()
    )
}

fn binary(dict: &str, data: &[u8]) -> String {
    stream(
        dict,
        &data.iter().map(|&b| char::from(b)).collect::<String>(),
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

const HELV: &str =
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";

fn text_page(content: &str) -> Vec<u8> {
    page_with("", "/Font << /F1 5 0 R >>", content, &[HELV.to_string()])
}

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

fn all(value: bool) -> Value {
    json!({"info": value, "xmp": value, "attachments": value, "outlines": value, "annots": value,
        "links": value, "fields": value, "layers": value, "struct": value})
}

/// redact-apply's reply and the copy's bytes (empty on a refusal).
fn apply_with(
    engine: &mut AppEngine,
    session: &mut AppSession,
    sanitize: Value,
) -> (Value, Vec<u8>) {
    let request = json!({"id": 9, "op": "redact-apply", "sanitize": sanitize,
        "entropy": {"$buffer": 0}});
    let (reply, mut outputs) = engine.dispatch(session, request, vec![vec![7u8; 32]]);
    let bytes = match reply["ok"] == true {
        true => outputs.remove(0),
        false => Vec::new(),
    };
    (reply, bytes)
}

fn apply(engine: &mut AppEngine, session: &mut AppSession) -> Vec<u8> {
    let (reply, bytes) = apply_with(engine, session, all(true));
    assert_eq!(reply["ok"], true, "{reply}");
    bytes
}

fn mark(engine: &mut AppEngine, session: &mut AppSession, page: usize, rect: [f64; 4]) {
    ok(
        engine,
        session,
        json!({"op": "redact-add", "page": page, "rect": rect}),
    );
}

fn search(engine: &mut AppEngine, session: &mut AppSession, text: &str) {
    ok(
        engine,
        session,
        json!({"op": "redact-search", "text": text}),
    );
}

/// Every object a copy holds, written out: strings and names anywhere, and
/// every stream decoded (what a reader of the file could find).
fn everything(bytes: &[u8], password: &[u8]) -> Vec<u8> {
    let doc = Document::open_with_password(bytes, DocumentLimits::default(), password).unwrap();
    let mut out = Vec::new();
    for object in doc.objects() {
        let mut stack = vec![&object.value];
        while let Some(value) = stack.pop() {
            match value {
                Object::String(s) | Object::Name(s) => {
                    out.extend_from_slice(s);
                    out.push(b'\n');
                }
                Object::Array(items) => stack.extend(items),
                Object::Dictionary(dict) => stack.extend(dict.values()),
                _ => {}
            }
        }
        if object.stream.is_some() {
            if let Ok(decoded) = doc.decoded_stream(object.id, StreamLimits::default()) {
                out.extend_from_slice(&decoded);
                out.push(b'\n');
            }
        }
    }
    out
}

fn holds(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn view_text(bytes: &[u8], password: &str) -> String {
    let view = ViewDocument::open(bytes.to_vec(), password).unwrap();
    (0..view.page_count())
        .map(|p| view.structured_text(p).unwrap().as_text())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every object written, for exact before/after comparisons.
fn serialized(session: &AppSession) -> Vec<u8> {
    let options = WriteOptions {
        garbage: false,
        ..WriteOptions::APP
    };
    let doc = session.doc.as_ref().unwrap().document();
    write_document(doc, DocumentWriteLimits::default(), options).unwrap()
}

fn walk(session: &mut AppSession, page: usize) -> Content {
    let (view, doc) = session.doc.as_mut().unwrap().view_with_document().unwrap();
    region::page_content(doc, Some(view), page, Detail::Full, Limits::DEFAULT).unwrap()
}

/// The glyphs a walk places: code and page-space box.
fn glyphs(content: &Content) -> Vec<(u32, [f64; 4])> {
    (content.draws.iter())
        .filter_map(|d| match &d.kind {
            DrawKind::Text(t) => Some(t.glyphs.iter().map(|g| (g.code, g.bounds))),
            _ => None,
        })
        .flatten()
        .collect()
}

/// The page's /Contents of a copy, decoded and joined.
fn contents(bytes: &[u8], password: &[u8], page: usize) -> Vec<u8> {
    let doc = Document::open_with_password(bytes, DocumentLimits::default(), password).unwrap();
    let pages = tandem_pdf_engine::app::info::pages(&doc);
    let dict = pages[page].dict;
    let mut out = Vec::new();
    for stream in tandem_pdf_engine::app::redact::content::content_streams(&doc, dict) {
        out.extend(doc.decoded_stream(stream, StreamLimits::default()).unwrap());
        out.push(b'\n');
    }
    out
}

fn render(bytes: &[u8], page: usize) -> (usize, usize, Vec<u8>) {
    let view = ViewDocument::open(bytes.to_vec(), "").unwrap();
    let image = view.render(page, 1.0, RenderOptions::default()).unwrap();
    (image.width as usize, image.height as usize, image.pixels)
}

/// The pixels of two drawings agree outside `rects` grown by `margin`.
fn same_outside(
    a: &(usize, usize, Vec<u8>),
    b: &(usize, usize, Vec<u8>),
    rects: &[[f64; 4]],
    margin: f64,
) {
    assert_eq!((a.0, a.1), (b.0, b.1));
    let mut differ = 0;
    for y in 0..a.1 {
        for x in 0..a.0 {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let inside = rects.iter().any(|r| {
                px > r[0] - margin && px < r[2] + margin && py > r[1] - margin && py < r[3] + margin
            });
            let at = (y * a.0 + x) * 4;
            if !inside && a.2[at..at + 3] != b.2[at..at + 3] {
                if std::env::var("REDACT_DEBUG").is_ok() {
                    eprintln!("{x},{y}: {:?} vs {:?}", &a.2[at..at + 3], &b.2[at..at + 3]);
                }
                differ += 1;
            }
        }
    }
    assert_eq!(differ, 0, "pixels differ outside the marks");
}

// ------------------------------------------------- the app's fixture

#[test]
fn every_marked_surface_of_redact_pdf_leaves_the_copy_and_the_session_stays() {
    let (mut engine, mut session) = session_with(fixture("redact.pdf"));
    search(&mut engine, &mut session, "P0B-SECRET-7731");
    let before = serialized(&session);
    let state = session.journal_state();
    let original = glyphs(&walk(&mut session, 0));
    let (reply, bytes) = apply_with(&mut engine, &mut session, all(true));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["marks"], 8);
    assert_eq!(reply["pages"], json!([0, 1]));
    assert_eq!(reply["verified"], json!({"text": 8, "pixels": 8, "raw": 1}));
    // As in reference implementations: the tagged text's structure element lost its
    // alternate texts with the text, so `struct` is 0.
    assert_eq!(
        reply["sanitized"],
        json!({"info": 1, "xmp": true, "attachments": 1, "outlines": 1, "annots": 2,
            "links": 1, "fields": 1, "layers": 1, "struct": 0})
    );
    assert_eq!(
        reply["kept"],
        json!({"info": 0, "xmp": false, "attachments": 0, "outlines": 0, "annots": 0,
            "links": 0, "fields": 0, "layers": 0, "struct": 0})
    );
    assert!(reply.get("revision").is_none(), "no journal state");
    // The session is as it was, marks and all.
    assert_eq!(serialized(&session), before);
    assert_eq!(session.journal_state(), state);
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(preview["marks"], 8);
    // Nothing of the copy holds the token: no object, no string, no stream.
    let found = everything(&bytes, b"");
    assert!(!holds(&found, "P0B-SECRET-7731"));
    assert!(!holds(&bytes, "P0B-SECRET-7731"));
    assert!(!holds(&found, "Redact"));
    let text = view_text(&bytes, "");
    assert!(!text.contains("P0B-SECRET-7731"), "{text}");
    // The inline /ActualText (not the token) reads in place of its span.
    for kept in [
        "visible",
        "here",
        "invisible",
        "P4-ALT-4242",
        "page2-only ok",
    ] {
        assert!(text.contains(kept), "{kept} in {text}");
    }
    // One revision, the kept text where it was.
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    assert_eq!(doc.trailer_chain().count(), 1);
    let (_, mut copy) = session_with(bytes.clone());
    let after = glyphs(&walk(&mut copy, 0));
    assert_eq!(after[..8], original[..8], "\"visible \" is where it was");
    let out = String::from_utf8_lossy(&contents(&bytes, b"", 0)).to_string();
    assert!(
        out.contains("BT /F1 12 Tf 72 740 Td [<76697369626c6520> -8836 <2068657265>] TJ ET"),
        "{out}"
    );
}

#[test]
fn each_sanitize_toggle_left_off_keeps_its_surface() {
    for (key, kept) in [
        ("info", json!(1)),
        ("xmp", json!(true)),
        ("attachments", json!(1)),
        ("outlines", json!(1)),
        ("annots", json!(2)),
        ("links", json!(1)),
        ("fields", json!(1)),
        ("layers", json!(1)),
        ("struct", json!(0)),
    ] {
        let (mut engine, mut session) = session_with(fixture("redact.pdf"));
        search(&mut engine, &mut session, "P0B-SECRET-7731");
        let mut sanitize = all(true);
        sanitize[key] = json!(false);
        let (reply, bytes) = apply_with(&mut engine, &mut session, sanitize);
        assert_eq!(reply["ok"], true, "{key}: {reply}");
        assert_eq!(reply["kept"][key], kept, "{key}");
        let zero = if key == "xmp" { json!(false) } else { json!(0) };
        assert_eq!(reply["sanitized"][key], zero, "{key}");
        // A surface kept by choice still holds the token; the structure
        // tree's went with the redaction of the tagged text.
        let left = holds(&everything(&bytes, b""), "P0B-SECRET-7731");
        assert_eq!(left, key != "struct", "{key}");
    }
}

#[test]
fn a_moved_search_mark_fails_the_raw_check_and_writes_nothing() {
    let (mut engine, mut session) = session_with(text_page(
        "BT /F1 12 Tf 100 700 Td (one SECRET) Tj ET BT /F1 12 Tf 100 600 Td (two SECRET) Tj ET",
    ));
    search(&mut engine, &mut session, "SECRET");
    let listed = ok(
        &mut engine,
        &mut session,
        json!({"op": "annots", "page": 0}),
    );
    let first = listed["items"][0]["id"].clone();
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-set", "page": 0, "annotId": first, "rect": [400, 300, 500, 320]}),
    );
    let before = serialized(&session);
    let state = session.journal_state();
    let (reply, _) = apply_with(&mut engine, &mut session, all(true));
    assert_eq!(reply["reason"], "verify-failed", "{reply}");
    assert_eq!(
        reply["error"],
        "redact-apply: raw check failed: a searched text is still in the marked pages' content"
    );
    assert_eq!(serialized(&session), before);
    assert_eq!(session.journal_state(), state);
}

// ------------------------------------------------- text

#[test]
fn text_under_marks_leaves_its_shows_and_the_rest_keeps_its_place() {
    let content = "BT /F1 12 Tf 14 TL 100 700 Td (Hello SECRET world) Tj\n\
        [(A) -120 (SEC) 30 (RET) 50 (B)] TJ\n\
        (SECRET) ' (x) Tj 2 1 (SECRET) \" (after) Tj ET\n\
        BT /F1 12 Tf 3 Tr 100 500 Td (SECRET hidden) Tj ET";
    let (mut engine, mut session) = session_with(text_page(content));
    let original = glyphs(&walk(&mut session, 0));
    search(&mut engine, &mut session, "SECRET");
    let bytes = apply(&mut engine, &mut session);
    let out = contents(&bytes, b"", 0);
    assert!(!holds(&everything(&bytes, b""), "SECRET"));
    assert!(!holds(&out, "SEC") && !holds(&out, "RET"));
    let text = view_text(&bytes, "");
    for kept in ["Hello", "world", "after", "hidden"] {
        assert!(text.contains(kept), "{kept} in {text}");
    }
    // Every kept glyph is where it was: the removed runs are adjustments.
    let (_, mut copy) = session_with(bytes);
    let after = glyphs(&walk(&mut copy, 0));
    let codes: Vec<u32> = original.iter().map(|g| g.0).collect();
    let secret: Vec<u32> = "SECRET".bytes().map(u32::from).collect();
    let mut gone = vec![false; original.len()];
    for start in 0..original.len().saturating_sub(5) {
        if codes[start..start + 6] == secret[..] {
            gone[start..start + 6].iter_mut().for_each(|g| *g = true);
        }
    }
    let kept: Vec<_> = (original.iter().zip(&gone))
        .filter(|(_, g)| !**g)
        .map(|(o, _)| *o)
        .collect();
    assert_eq!(after.len(), kept.len());
    for ((code, a), (c, b)) in after.iter().zip(&kept) {
        assert_eq!(code, c);
        for i in 0..4 {
            assert!((a[i] - b[i]).abs() < 1e-6, "{code} moved: {a:?} vs {b:?}");
        }
    }
}

#[test]
fn text_inside_a_form_drawn_on_two_pages_is_removed_from_the_one_form() {
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 300 20] /Resources << /Font << /F1 5 0 R >> >>",
        "BT /F1 12 Tf 0 5 Td (formxo SECRET) Tj ET",
    );
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 7 0 R] /Count 2 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Fm 6 0 R >> >> /Contents 4 0 R >>".to_string(),
        stream("", "q 1 0 0 1 100 700 cm /Fm Do Q"),
        HELV.to_string(),
        form,
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Fm 6 0 R >> >> /Contents 8 0 R >>".to_string(),
        stream("", "q 1 0 0 1 100 500 cm /Fm Do Q"),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    search(&mut engine, &mut session, "SECRET");
    let bytes = apply(&mut engine, &mut session);
    assert!(!holds(&everything(&bytes, b""), "SECRET"));
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let form = doc.decoded_stream(id(6), StreamLimits::default()).unwrap();
    assert!(
        holds(&form, "<666f726d786f20>"),
        "{}",
        String::from_utf8_lossy(&form)
    );
    assert_eq!(view_text(&bytes, "").matches("formxo").count(), 2);
}

#[test]
fn a_content_stream_another_page_shares_is_left_to_that_page() {
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 2 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string(),
        stream("", "BT /F1 12 Tf 100 700 Td (shared SECRET) Tj ET"),
        HELV.to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    mark(&mut engine, &mut session, 0, [90.0, 80.0, 300.0, 100.0]);
    let bytes = apply(&mut engine, &mut session);
    assert!(!holds(&contents(&bytes, b"", 0), "SECRET"));
    // The unmarked page draws its content as before (an area mark is the
    // page's own; a search would have marked both).
    assert!(holds(&contents(&bytes, b"", 1), "(shared SECRET) Tj"));
}

#[test]
fn a_form_showing_text_differently_where_it_is_marked_refuses() {
    // The Form inherits its font size from each placement.
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 300 40] /Resources << /Font << /F1 5 0 R >> >>",
        "BT 0 5 Td (SECRET) Tj ET",
    );
    let (mut engine, mut session) = session_with(page_with(
        "",
        "/Font << /F1 5 0 R >> /XObject << /Fm 6 0 R >>",
        "BT /F1 12 Tf ET q 1 0 0 1 100 700 cm /Fm Do Q BT /F1 24 Tf ET q 1 0 0 1 100 500 cm /Fm Do Q",
        &[HELV.to_string(), form],
    ));
    mark(&mut engine, &mut session, 0, [90.0, 50.0, 400.0, 100.0]);
    mark(&mut engine, &mut session, 0, [90.0, 250.0, 400.0, 300.0]);
    let (reply, _) = apply_with(&mut engine, &mut session, all(true));
    assert_eq!(reply["reason"], "invalid", "{reply}");
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .contains("cannot be applied exactly")
    );
}

#[test]
fn alternate_texts_around_removed_text_go_inline_named_and_in_the_tree() {
    let content = "/Span <</MCID 0 /ActualText (SECRET) /Lang (en)>> BDC\n\
        BT /F1 12 Tf 100 700 Td (SECRET) Tj ET EMC\n\
        /Span /P1 BDC BT /F1 12 Tf 100 600 Td (SECRET) Tj ET EMC\n\
        /Span <</ActualText (KEEP)>> BDC BT /F1 12 Tf 100 400 Td (other) Tj ET EMC";
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 6 0 R /MarkInfo << /Marked true >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /StructParents 0 /Resources << /Font << /F1 5 0 R >> /Properties << /P1 << /ActualText (SECRET2) /E (SECRET3) >> >> >> /Contents 4 0 R >>".to_string(),
        stream("", content),
        HELV.to_string(),
        "<< /Type /StructTreeRoot /K [9 0 R] /ParentTree 7 0 R >>".to_string(),
        "<< /Nums [0 [8 0 R]] >>".to_string(),
        "<< /Type /StructElem /S /Span /P 9 0 R /Pg 3 0 R /K 0 /ActualText (SECRET) /Alt (SECRET) >>".to_string(),
        "<< /Type /StructElem /S /P /P 6 0 R /K [8 0 R] /ActualText (SECRET4) >>".to_string(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    search(&mut engine, &mut session, "SECRET");
    let mut sanitize = all(true);
    sanitize["struct"] = json!(false);
    let (reply, bytes) = apply_with(&mut engine, &mut session, sanitize);
    assert_eq!(reply["ok"], true, "{reply}");
    let found = everything(&bytes, b"");
    assert!(
        !holds(&found, "SECRET"),
        "{}",
        String::from_utf8_lossy(&found)
    );
    assert!(
        holds(&found, "KEEP"),
        "an untouched sequence keeps its text"
    );
    assert!(holds(&found, "/Lang (en)"), "other entries stay");
}

#[test]
fn comments_and_unbalanced_saves_do_not_survive_or_unsettle_the_page() {
    let content = "% SECRET in a comment\nQ Q 2 0 0 2 0 0 cm\n\
        BT /F1 6 Tf 50 350 Td (SECRET) Tj ET\n\
        q 0 0 1 rg /Tag BMC BT /F1 6 Tf 50 300 Td (open) Tj";
    let (mut engine, mut session) = session_with(text_page(content));
    search(&mut engine, &mut session, "SECRET");
    let bytes = apply(&mut engine, &mut session);
    let out = String::from_utf8_lossy(&contents(&bytes, b"", 0)).to_string();
    assert!(!out.contains("SECRET"), "{out}");
    assert!(out.starts_with("q\n"), "{out}");
    // The two Q with nothing to restore are gone; the open text object,
    // sequence and save close before the boxes.
    assert!(!out.contains("Q Q 2 0 0 2"), "{out}");
    assert!(out.contains("ET\nEMC\nQ\nQ\nq 0 g"), "{out}");
}

#[test]
fn size_zero_text_goes_and_the_pen_moves_past_it_at_size_one() {
    let (mut engine, mut session) =
        session_with(text_page("BT /F1 0 Tf 100 700 Td (SECRET) Tj ET"));
    mark(&mut engine, &mut session, 0, [90.0, 70.0, 300.0, 110.0]);
    let bytes = apply(&mut engine, &mut session);
    assert!(!holds(&everything(&bytes, b""), "SECRET"));
    // At 0 Tf the pen moves by Tc alone, which no adjustment at size 0 can
    // say: the run's move is an adjustment at size 1, and what follows
    // stays where it was. (Stage B refused this.)
    let content = "BT /F1 0 Tf 5 Tc 100 700 Td (SECRET) Tj /F1 12 Tf 0 Tc (kept) Tj ET";
    let (mut engine, mut session) = session_with(text_page(content));
    let original = glyphs(&walk(&mut session, 0));
    mark(&mut engine, &mut session, 0, [90.0, 85.0, 128.0, 100.0]);
    let bytes = apply(&mut engine, &mut session);
    assert!(!holds(&everything(&bytes, b""), "SECRET"));
    let out = String::from_utf8_lossy(&contents(&bytes, b"", 0)).to_string();
    assert!(
        out.contains("[] TJ /F1 1 Tf [-30000] TJ /F1 0 Tf [] TJ"),
        "{out}"
    );
    let (_, mut copy) = session_with(bytes);
    let after = glyphs(&walk(&mut copy, 0));
    let kept = &original[6..];
    assert_eq!(after.len(), kept.len(), "{after:?}");
    for ((code, a), (c, b)) in after.iter().zip(kept) {
        assert_eq!(code, c);
        for i in 0..4 {
            assert!((a[i] - b[i]).abs() < 1e-6, "{code} moved: {a:?} vs {b:?}");
        }
    }
    // A font an ExtGState sets has no name to set the size with: refused.
    let widths = format!(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 65 /LastChar 90 /Widths [{}] >>",
        "600 ".repeat(26)
    );
    let (mut engine, mut session) = session_with(page_with(
        "",
        "/ExtGState << /G0 << /Font [5 0 R 0] >> >>",
        "BT /G0 gs 5 Tc 100 700 Td (SECRET) Tj ET",
        &[widths],
    ));
    mark(&mut engine, &mut session, 0, [90.0, 70.0, 300.0, 110.0]);
    let (reply, _) = apply_with(&mut engine, &mut session, all(true));
    assert_eq!(reply["reason"], "invalid", "{reply}");
}

#[test]
fn a_run_that_cannot_be_placed_goes_whole_when_it_reaches_a_mark() {
    let font = "<< /Type /Font /Subtype /Type0 /BaseFont /X /Encoding /NoSuchCMap /DescendantFonts [<< /Type /Font /Subtype /CIDFontType2 /BaseFont /X /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> >>] >>";
    let (mut engine, mut session) = session_with(page_with(
        "",
        "/Font << /F1 5 0 R /F2 6 0 R >>",
        "BT /F2 12 Tf 100 700 Td <0053004500430052> Tj ET BT /F1 12 Tf 100 600 Td (kept) Tj ET",
        &[HELV.to_string(), font.to_string()],
    ));
    mark(&mut engine, &mut session, 0, [120.0, 85.0, 140.0, 100.0]);
    let bytes = apply(&mut engine, &mut session);
    let out = String::from_utf8_lossy(&contents(&bytes, b"", 0)).to_string();
    assert!(!out.contains("<0053"), "{out}");
    assert!(view_text(&bytes, "").contains("kept"));
}

// ------------------------------------------------- images

/// A 4 by 4 RGB image, every sample 200, with a 4 by 4 gray soft mask.
fn image_page(content: &str) -> Vec<u8> {
    let rgb = vec![200u8; 4 * 4 * 3];
    let alpha = vec![255u8; 16];
    page_with(
        "",
        "/XObject << /Im 5 0 R >>",
        content,
        &[
            binary(
                "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask 6 0 R",
                &rgb,
            ),
            binary(
                "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8",
                &alpha,
            ),
        ],
    )
}

fn samples(doc: &Document<'_>, image: ObjectId) -> Vec<u8> {
    doc.decoded_stream(image, StreamLimits::default()).unwrap()
}

#[test]
fn image_pixels_under_a_mark_are_overwritten_in_the_image_and_its_mask() {
    // The image covers page space x 100..200, y 92..192: 25 points a pixel.
    let (mut engine, mut session) = session_with(image_page("q 100 0 0 100 100 600 cm /Im Do Q"));
    mark(&mut engine, &mut session, 0, [100.0, 92.0, 150.0, 142.0]);
    let bytes = apply(&mut engine, &mut session);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let rgb = samples(&doc, id(5));
    // The soft mask is blanked in a copy (another image may share it); the
    // original, now unused, is not written.
    let smask = (doc.stream_dict(id(5)).unwrap())
        .get(b"SMask".as_slice())
        .and_then(Object::as_reference)
        .unwrap();
    assert_ne!(smask, id(6));
    assert!(doc.stream_dict(id(6)).is_err());
    let alpha = samples(&doc, smask);
    for y in 0..4 {
        for x in 0..4 {
            let under = x < 2 && y < 2;
            let at = y * 4 + x;
            let expected = if under { [0u8; 3] } else { [200u8; 3] };
            assert_eq!(rgb[at * 3..at * 3 + 3], expected, "{x},{y}");
            assert_eq!(alpha[at], if under { 0 } else { 255 }, "{x},{y}");
        }
    }
    let dict = doc.stream_dict(id(5)).unwrap();
    assert_eq!(
        doc.dict_get(dict, b"Filter").as_name(),
        Some(&b"FlateDecode"[..])
    );
    assert!(holds(&contents(&bytes, b"", 0), "/Im Do"));
}

#[test]
fn an_image_wholly_under_a_mark_is_no_longer_drawn_and_holds_one_pixel() {
    let (mut engine, mut session) = session_with(image_page("q 100 0 0 100 100 600 cm /Im Do Q"));
    mark(&mut engine, &mut session, 0, [90.0, 80.0, 210.0, 200.0]);
    let bytes = apply(&mut engine, &mut session);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    assert!(!holds(&contents(&bytes, b"", 0), "/Im Do"));
    let dict = doc.stream_dict(id(5)).unwrap();
    assert_eq!(doc.dict_get(dict, b"Width").as_i64(), Some(1));
    assert_eq!(samples(&doc, id(5)), vec![0, 0, 0]);
    assert!(
        doc.stream_dict(id(6)).is_err(),
        "its mask is gone from the copy"
    );
}

#[test]
fn inline_images_are_blanked_or_removed() {
    let data: String = (0..16u8).map(|_| char::from(200)).collect();
    let content = format!(
        "q 100 0 0 100 100 600 cm BI /W 4 /H 4 /CS /G /BPC 8 ID {data} EI Q\n\
         q 100 0 0 100 300 600 cm BI /W 4 /H 4 /CS /G /BPC 8 ID {data} EI Q"
    );
    let (mut engine, mut session) = session_with(page_with("", "", &content, &[]));
    mark(&mut engine, &mut session, 0, [100.0, 92.0, 150.0, 142.0]);
    mark(&mut engine, &mut session, 0, [290.0, 80.0, 410.0, 200.0]);
    let bytes = apply(&mut engine, &mut session);
    let text = String::from_utf8_lossy(&contents(&bytes, b"", 0)).to_string();
    assert_eq!(text.matches("BI").count(), 1, "{text}");
    let start = text.find("ID\n").unwrap() + 3;
    let end = start + text[start..].find('>').unwrap() + 1;
    let decoded = stream::decode_filters(
        &text.as_bytes()[start..end],
        &[Filter::AsciiHex, Filter::Flate],
        StreamLimits::default(),
    )
    .unwrap();
    for y in 0..4 {
        for x in 0..4 {
            let under = x < 2 && y < 2;
            assert_eq!(decoded[y * 4 + x], if under { 0 } else { 200 }, "{x},{y}");
        }
    }
}

#[test]
fn an_inline_image_this_version_cannot_decode_refuses() {
    let content = "q 100 0 0 100 100 600 cm BI /W 4 /H 4 /CS /G /BPC 8 /F /DCT ID \u{ff}\u{d8}\u{ff}\u{d9} EI Q";
    let (mut engine, mut session) = session_with(page_with("", "", content, &[]));
    mark(&mut engine, &mut session, 0, [100.0, 92.0, 150.0, 142.0]);
    let (reply, _) = apply_with(&mut engine, &mut session, all(true));
    assert_eq!(reply["reason"], "invalid", "{reply}");
}

// ------------------------------------------------- paths and shadings

#[test]
fn paths_under_a_mark_are_cut_and_draw_the_same_outside_it() {
    // Lines and fills: outside the mark they draw pixel for pixel as
    // before. A curve crossing the mark is split where it crosses; the
    // renderer flattens the parts left anew, so its pixels may differ by
    // antialiasing (checked by points).
    let content = "0 0 1 rg 50 500 300 100 re f\n\
        1 0 0 RG 5 w 50 450 m 350 450 l S\n\
        0 1 0 rg 160 470 m 190 470 l 175 490 l h f\n\
        q 60 410 280 30 re W n 0.5 g 0 0 612 792 re f Q\n\
        0 0 0 RG 1 w 120 430 m 200 520 300 380 330 430 c S";
    let source = page_with("", "", content, &[]);
    let (mut engine, mut session) = session_with(source.clone());
    let rect = [150.0, 280.0, 250.0, 360.0];
    mark(&mut engine, &mut session, 0, rect);
    let bytes = apply(&mut engine, &mut session);
    let out = String::from_utf8_lossy(&contents(&bytes, b"", 0)).to_string();
    // No point of any path lies inside the mark (page y = 792 - user y).
    let mut operands: Vec<f64> = Vec::new();
    let mut points = 0;
    for token in out.split_whitespace() {
        match token.parse::<f64>() {
            Ok(n) => operands.push(n),
            Err(_) => {
                if matches!(token, "m" | "l" | "c") {
                    for pair in operands.chunks(2) {
                        let (x, y) = (pair[0], 792.0 - pair[1]);
                        let e = 1e-6;
                        let inside = x > rect[0] + e
                            && x < rect[2] - e
                            && y > rect[1] + e
                            && y < rect[3] - e;
                        assert!(!inside, "({x}, {y}) inside the mark: {out}");
                        points += 1;
                    }
                }
                operands.clear();
            }
        }
    }
    assert!(points > 10, "{out}");
    // The triangle wholly inside is gone; the curve's parts outside stay
    // curves; the rest draws as before.
    assert!(!out.contains("175 490"), "{out}");
    assert!(out.matches(" c\n").count() >= 2, "{out}");
    let lines = content.rsplit_once('\n').map_or(content, |(head, _)| head);
    let straight = page_with("", "", lines, &[]);
    let (mut engine, mut session) = session_with(straight.clone());
    mark(&mut engine, &mut session, 0, rect);
    let cut = apply(&mut engine, &mut session);
    same_outside(&render(&straight, 0), &render(&cut, 0), &[rect], 2.0);
}

#[test]
fn shadings_are_clipped_out_and_mesh_shadings_refuse() {
    let axial = "<< /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 612 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> /Extend [true true] >>";
    let source = page_with(
        "",
        "/Shading << /Sh 5 0 R >>",
        "/Sh sh",
        &[axial.to_string()],
    );
    let (mut engine, mut session) = session_with(source.clone());
    let rect = [200.0, 200.0, 300.0, 300.0];
    mark(&mut engine, &mut session, 0, rect);
    let bytes = apply(&mut engine, &mut session);
    let out = String::from_utf8_lossy(&contents(&bytes, b"", 0)).to_string();
    assert!(out.contains("W* n") && out.contains("/Sh sh"), "{out}");
    same_outside(&render(&source, 0), &render(&bytes, 0), &[rect], 2.0);
    let mesh = stream(
        "/ShadingType 4 /ColorSpace /DeviceGray /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 612 0 792 0 1]",
        "",
    );
    let (mut engine, mut session) =
        session_with(page_with("", "/Shading << /Sh 5 0 R >>", "/Sh sh", &[mesh]));
    mark(&mut engine, &mut session, 0, rect);
    let (reply, _) = apply_with(&mut engine, &mut session, all(true));
    assert_eq!(reply["reason"], "invalid", "{reply}");
}

// ------------------------------------------------- annotations

#[test]
fn annotations_under_a_mark_go_widgets_with_their_fields() {
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [7 0 R 8 0 R] >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Contents 4 0 R /Annots [5 0 R 6 0 R 7 0 R 8 0 R 9 0 R] >>".to_string(),
        stream("", ""),
        "<< /Type /Annot /Subtype /FreeText /Rect [100 600 200 650] /Contents (SECRET) /DA (/Helv 12 Tf 0 g) >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [100 550 200 580] /A << /S /URI /URI (https://example.com/SECRET) >> >>".to_string(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (under) /V (SECRET) /Rect [100 500 200 530] /P 3 0 R >>".to_string(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (away) /V (kept) /Rect [400 100 500 130] /P 3 0 R >>".to_string(),
        "<< /Type /Annot /Subtype /Text /Rect [400 700 420 720] /Contents (note) >>".to_string(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    mark(&mut engine, &mut session, 0, [90.0, 130.0, 210.0, 300.0]);
    let (reply, bytes) = apply_with(&mut engine, &mut session, all(false));
    assert_eq!(reply["ok"], true, "{reply}");
    let found = everything(&bytes, b"");
    assert!(
        !holds(&found, "SECRET"),
        "{}",
        String::from_utf8_lossy(&found)
    );
    assert!(holds(&found, "kept") && holds(&found, "note"));
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let page = tandem_pdf_engine::app::info::pages(&doc)[0].dict;
    let annots = doc.dict_get(page, b"Annots").as_array().unwrap();
    let refs: Vec<u32> = (annots.iter())
        .filter_map(|a| a.as_reference())
        .map(|r| r.number())
        .collect();
    assert_eq!(refs, [8, 9]);
    let catalog = doc.resolve_id(doc.root()).as_dict().unwrap();
    let form = doc.dict_get(catalog, b"AcroForm").as_dict().unwrap();
    let fields = doc.dict_get(form, b"Fields").as_array().unwrap();
    assert_eq!(fields, [Object::Reference(id(8))]);
}

// ------------------------------------------------- the file as a whole

#[test]
fn an_earlier_revision_holding_the_secret_is_not_written() {
    // Revision 1 shows one secret; revision 2 replaces the content.
    let mut bytes = text_page("BT /F1 12 Tf 100 700 Td (OLDSECRET) Tj ET");
    let previous = {
        let text = String::from_utf8_lossy(&bytes).to_string();
        let at = text.rfind("startxref\n").unwrap() + 10;
        text[at..].lines().next().unwrap().parse::<usize>().unwrap()
    };
    let offset = bytes.len();
    let content = "BT /F1 12 Tf 100 700 Td (Visible NEWSECRET) Tj ET";
    bytes.extend_from_slice(
        format!(
            "4 0 obj\n<< /Length {} >>\nstream\n{content}\nendstream\nendobj\n",
            content.len()
        )
        .as_bytes(),
    );
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!(
            "xref\n4 1\n{offset:010} 00000 n \ntrailer\n<< /Size 6 /Root 1 0 R /Prev {previous} >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    let (mut engine, mut session) = session_with(bytes);
    search(&mut engine, &mut session, "NEWSECRET");
    let out = apply(&mut engine, &mut session);
    for secret in ["OLDSECRET", "NEWSECRET"] {
        assert!(!holds(&out, secret));
        assert!(!holds(&everything(&out, b""), secret));
    }
    let doc = Document::open(&out, DocumentLimits::default()).unwrap();
    assert_eq!(doc.trailer_chain().count(), 1);
    assert!(view_text(&out, "").contains("Visible"));
}

#[test]
fn the_copy_keeps_the_sources_encryption_or_the_pending_setting() {
    let (mut engine, mut session) = session_with(fixture("encrypted-aes256.pdf"));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "pdf4all-user"}),
    );
    mark(&mut engine, &mut session, 0, [30.0, 30.0, 120.0, 80.0]);
    let bytes = apply(&mut engine, &mut session);
    assert!(Document::open(&bytes, DocumentLimits::default()).is_err());
    let doc =
        Document::open_with_password(&bytes, DocumentLimits::default(), b"pdf4all-user").unwrap();
    assert_eq!(doc.encryption().map(|e| e.v), Some(5));
    assert_eq!(doc.trailer_chain().count(), 1);
    // RC4, unlocked with the user password.
    let (mut engine, mut session) = session_with(fixture("encrypted.pdf"));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "pdf4all-user"}),
    );
    search(&mut engine, &mut session, "e");
    let bytes = apply(&mut engine, &mut session);
    let doc =
        Document::open_with_password(&bytes, DocumentLimits::default(), b"pdf4all-user").unwrap();
    assert_eq!(doc.encryption().map(|e| e.v), Some(2));
    assert!(!view_text(&bytes, "pdf4all-user").contains('e'));
    // A plain document with a pending AES-256 setting: the copy has it.
    let (mut engine, mut session) = session_with(text_page("BT /F1 12 Tf 100 700 Td (x) Tj ET"));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "security-set", "mode": "aes-256", "userPassword": "u", "ownerPassword": "o",
            "permissions": {"print": true}}),
    );
    mark(&mut engine, &mut session, 0, [90.0, 70.0, 300.0, 110.0]);
    let bytes = apply(&mut engine, &mut session);
    assert!(Document::open(&bytes, DocumentLimits::default()).is_err());
    let doc = Document::open_with_password(&bytes, DocumentLimits::default(), b"u").unwrap();
    assert_eq!(doc.encryption().map(|e| e.v), Some(5));
}

#[test]
fn refusals_come_in_engine_ts_order_and_change_nothing() {
    let mut engine = AppEngine::new();
    let mut empty = AppSession::default();
    let (reply, _) = apply_with(&mut engine, &mut empty, all(true));
    assert_eq!(reply["reason"], "no-document");
    let (mut engine, mut signed) = session_with(fixture("p0b/signed-intact.pdf"));
    assert_eq!(
        apply_with(&mut engine, &mut signed, all(true)).0["reason"],
        "read-only"
    );
    // No marks comes before a malformed sanitize.
    let (mut engine, mut session) = session_with(text_page("BT /F1 12 Tf 100 700 Td (x) Tj ET"));
    let (reply, _) = apply_with(&mut engine, &mut session, json!({"info": true}));
    assert_eq!(reply["reason"], "no-marks");
    assert_eq!(reply["error"], "redact-apply: there are no marks");
    mark(&mut engine, &mut session, 0, [90.0, 70.0, 300.0, 110.0]);
    let before = serialized(&session);
    let state = session.journal_state();
    let (reply, _) = apply_with(&mut engine, &mut session, json!({"info": true}));
    assert_eq!(
        reply["error"],
        "redact-apply: sanitize.xmp must be true or false"
    );
    assert_eq!(reply["reason"], "invalid");
    assert_eq!(reply["revision"], 1, "a refusal carries the journal state");
    assert_eq!(serialized(&session), before);
    assert_eq!(session.journal_state(), state);
    // A Form placed unmarked elsewhere.
    let (mut engine, mut session) = session_with(fixture("redact-shared-only.pdf"));
    mark(&mut engine, &mut session, 0, [100.0, 320.0, 230.0, 350.0]);
    assert_eq!(
        apply_with(&mut engine, &mut session, all(true)).0["reason"],
        "shared-content"
    );
    // The rewrite apply starts from is limited to 12 MiB: an unreachable
    // stream is not part of it, a page's thumbnail is.
    let big = "x".repeat(12 * 1024 * 1024 + 16);
    for (page, refused) in [("", false), ("/Thumb 6 0 R", true)] {
        let (mut engine, mut session) = session_with(page_with(
            page,
            "/Font << /F1 5 0 R >>",
            "BT /F1 12 Tf 100 700 Td (x) Tj ET",
            &[HELV.to_string(), stream("", &big)],
        ));
        mark(&mut engine, &mut session, 0, [90.0, 70.0, 300.0, 110.0]);
        let (reply, _) = apply_with(&mut engine, &mut session, all(true));
        match refused {
            false => assert_eq!(reply["ok"], true, "{reply}"),
            true => {
                assert_eq!(reply["reason"], "too-large", "{reply}");
                assert_eq!(
                    reply["error"],
                    "Redaction is limited to documents up to 12 MiB in this version"
                );
            }
        }
    }
    // A document that does not permit changes.
    let (mut engine, mut session) = session_with(fixture("encrypted-noedit.pdf"));
    let _ = op(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "pdf4all-user"}),
    );
    assert_eq!(
        apply_with(&mut engine, &mut session, all(true)).0["reason"],
        "read-only"
    );
}

#[test]
fn hostile_content_under_marks_refuses_or_applies_and_never_panics() {
    let cases = [
        "BT /F1 12 Tf 100 700 Td (a) Tj ET \u{0}\u{1}\u{2} ((( <<< ]]] >> > Tj",
        "10 10 m 2 0 0 2 0 0 cm 300 300 l S",
        "q q q q BT BT ET ET Q Q Q Q Q Q Q",
        "BI /W 99999 /H 99999 /CS /RGB /BPC 16 ID x EI",
        "BI /W 3 /H 3 /CS /G /BPC 8 /F /Fl ID garbage EI",
        "/Span <</ActualText [ (x) <</A [1 2 (y)]>> ] >> BDC BT /F1 12 Tf 100 700 Td (SECRET) Tj ET",
        "BT /F1 12 Tf 100 700 Td [(SEC) [1 2] (RET) <</a 1>> (X)] TJ ET",
        "0 0 m 1e300 1e300 l -1e300 5 l h f 100 700 m 100 700 l f",
        "BT /F1 1e30 Tf 100 700 Td (SECRET) Tj ET",
        "BT /F1 12 Tf 0 0 0 0 100 700 Tm (SECRET) Tj ET",
        "BT /F1 12 Tf 100 700 Td (SECRET) Tj ET 0 0 m",
        "/Sh sh 0 0 0 0 0 0 cm /Sh sh",
    ];
    for content in cases {
        let (mut engine, mut session) = session_with(text_page(content));
        mark(&mut engine, &mut session, 0, [0.0, 0.0, 612.0, 792.0]);
        let before = serialized(&session);
        let (reply, bytes) = apply_with(&mut engine, &mut session, all(true));
        assert_eq!(serialized(&session), before, "{content}");
        if reply["ok"] == true {
            assert!(!holds(&everything(&bytes, b""), "SECRET"), "{content}");
        } else {
            assert!(reply["reason"].is_string(), "{content}: {reply}");
        }
    }
}

// ------------------------------------------------- review fixes

#[test]
fn derived_copies_of_the_marked_page_its_images_and_forms_leave_the_copy() {
    let pixels = "THUMB-SECRETPIX!";
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Thumb 6 0 R /PieceInfo << /Illustrator << /Private 7 0 R >> >> /Metadata 8 0 R /Resources << /Font << /F1 5 0 R >> /XObject << /Im 9 0 R /Fm 11 0 R >> >> /Contents 4 0 R >>".to_string(),
        stream(
            "",
            "BT /F1 12 Tf 100 700 Td (KEEP SECRET end) Tj ET q 100 0 0 100 100 500 cm /Im Do Q q 1 0 0 1 100 400 cm /Fm Do Q",
        ),
        HELV.to_string(),
        stream(
            "/Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8",
            pixels,
        ),
        stream("", "BT (SECRET) Tj ET"),
        stream("/Type /Metadata /Subtype /XML", "<x>SECRET page</x>"),
        binary(
            "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8 /Metadata 10 0 R",
            &[200u8; 16],
        ),
        stream("/Type /Metadata /Subtype /XML", "<x>SECRET image</x>"),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 300 20] /Resources << /Font << /F1 5 0 R >> >> /PieceInfo << /A << /Private 12 0 R >> >> /Metadata 13 0 R",
            "BT /F1 12 Tf 0 5 Td (SECRET) Tj ET",
        ),
        stream("", "(SECRET form private)"),
        stream("/Type /Metadata /Subtype /XML", "<x>SECRET form</x>"),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    search(&mut engine, &mut session, "SECRET");
    mark(&mut engine, &mut session, 0, [100.0, 192.0, 150.0, 242.0]);
    let (reply, bytes) = apply_with(&mut engine, &mut session, all(true));
    assert_eq!(reply["ok"], true, "{reply}");
    let found = everything(&bytes, b"");
    assert!(
        !holds(&found, "SECRET"),
        "{}",
        String::from_utf8_lossy(&found)
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let page = tandem_pdf_engine::app::info::pages(&doc)[0].dict;
    for key in [b"Thumb".as_slice(), b"PieceInfo", b"Metadata"] {
        assert!(page.get(key).is_none());
    }
    for object in [9, 11] {
        let dict = doc.stream_dict(id(object)).unwrap();
        assert!(dict.get(b"Metadata".as_slice()).is_none(), "{object}");
        assert!(dict.get(b"PieceInfo".as_slice()).is_none(), "{object}");
    }
    assert!(view_text(&bytes, "").contains("KEEP"));
}

/// One page drawing `content` with resources `resources`, objects from 5:
/// Helvetica, then `more`.
fn with_objects(resources: &str, content: &str, more: &[String]) -> Vec<u8> {
    let mut objects = vec![HELV.to_string()];
    objects.extend(more.iter().cloned());
    page_with("", resources, content, &objects)
}

fn refused_exactly(bytes: Vec<u8>, rect: [f64; 4], what: &str) {
    let (mut engine, mut session) = session_with(bytes);
    mark(&mut engine, &mut session, 0, rect);
    let (reply, _) = apply_with(&mut engine, &mut session, all(true));
    assert_eq!(reply["reason"], "invalid", "{what}: {reply}");
    let error = reply["error"].as_str().unwrap();
    assert!(
        error.contains(what) && error.contains("cannot be applied exactly"),
        "{error}"
    );
}

fn applied_exactly(bytes: Vec<u8>, rect: [f64; 4]) -> Vec<u8> {
    let (mut engine, mut session) = session_with(bytes);
    mark(&mut engine, &mut session, 0, rect);
    apply(&mut engine, &mut session)
}

const CELL: &str = "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 200 50] /XStep 200 /YStep 50 /Resources << /Font << /F1 5 0 R >> >>";

#[test]
fn a_pattern_whose_cell_shows_text_under_a_mark_refuses_and_a_hatch_is_cut() {
    let rect = [100.0, 100.0, 300.0, 200.0];
    let text = with_objects(
        "/Font << /F1 5 0 R >> /Pattern << /P1 6 0 R >>",
        "/Pattern cs /P1 scn 0 0 612 792 re f",
        &[stream(CELL, "BT /F1 12 Tf 10 20 Td (PATSECRET) Tj ET")],
    );
    refused_exactly(text, rect, "pattern");
    // Through a named colour space and as a stroke.
    let stroked = with_objects(
        "/Font << /F1 5 0 R >> /Pattern << /P1 6 0 R >> /ColorSpace << /CS0 [/Pattern] >>",
        "/CS0 CS /P1 SCN 20 w 0 700 m 612 0 l S",
        &[stream(CELL, "BT /F1 12 Tf 10 20 Td (PATSECRET) Tj ET")],
    );
    refused_exactly(stroked, [250.0, 350.0, 350.0, 450.0], "pattern");
    // A cell of lines is a texture: the fill is cut, the cell kept.
    let hatch = with_objects(
        "/Pattern << /P1 6 0 R >>",
        "/Pattern cs /P1 scn 0 0 612 792 re f",
        &[stream(CELL, "0 0 m 200 50 l S")],
    );
    let bytes = applied_exactly(hatch, rect);
    assert!(holds(&contents(&bytes, b"", 0), "/P1 scn"));
    // A shading pattern of a mesh is positional data.
    let mesh = with_objects(
        "/Pattern << /P1 6 0 R >>",
        "/Pattern cs /P1 scn 0 0 612 792 re f",
        &[
            "<< /Type /Pattern /PatternType 2 /Shading 7 0 R >>".to_string(),
            stream(
                "/ShadingType 4 /ColorSpace /DeviceGray /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 612 0 792 0 1]",
                "",
            ),
        ],
    );
    refused_exactly(mesh, rect, "pattern");
    // A pattern in a Form painted under a mark.
    let form = with_objects(
        "/XObject << /Fm 6 0 R >>",
        "/Fm Do",
        &[
            stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Pattern << /P1 7 0 R >> >>",
                "/Pattern cs /P1 scn 0 0 612 792 re f",
            ),
            stream(CELL, "BT /F1 12 Tf 10 20 Td (PATSECRET) Tj ET"),
        ],
    );
    refused_exactly(form, rect, "pattern");
}

#[test]
fn a_soft_mask_showing_an_image_under_a_mark_refuses_and_a_gradient_one_applies() {
    let rect = [100.0, 100.0, 300.0, 200.0];
    let gray = binary(
        "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8",
        &[10, 200, 30, 250],
    );
    let image = with_objects(
        "/ExtGState << /GS1 << /SMask << /S /Luminosity /G 6 0 R >> >> >>",
        "/GS1 gs 0 0 1 rg 0 0 612 792 re f",
        &[
            stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Group << /S /Transparency /CS /DeviceGray >> /Resources << /XObject << /Im 7 0 R >> >>",
                "q 612 0 0 792 0 0 cm /Im Do Q",
            ),
            gray,
        ],
    );
    refused_exactly(image, rect, "soft mask");
    let axial = "<< /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 612 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >>";
    let gradient = with_objects(
        "/ExtGState << /GS1 << /SMask << /S /Luminosity /G 6 0 R >> >> >>",
        "/GS1 gs 0 0 1 rg 0 0 612 792 re f",
        &[
            stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Group << /S /Transparency /CS /DeviceGray >> /Resources << /Shading << /Sh 7 0 R >> >>",
                "/Sh sh",
            ),
            axial.to_string(),
        ],
    );
    applied_exactly(gradient, rect);
}

#[test]
fn type3_glyphs_that_draw_images_refuse_and_glyph_shapes_apply() {
    let font = |proc: &str| {
        (
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /a 7 0 R >> /Encoding << /Differences [97 /a] >> /FirstChar 97 /LastChar 97 /Widths [1000] /Resources << /XObject << /Im 8 0 R >> >> >>".to_string(),
            stream("", proc),
        )
    };
    let image = binary(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
        &[0],
    );
    let page = |proc: &str| {
        let (dict, proc) = font(proc);
        with_objects(
            "/Font << /T3 6 0 R >>",
            "BT /T3 12 Tf 100 700 Td (aaa) Tj ET",
            &[dict, proc, image.clone()],
        )
    };
    let rect = [90.0, 80.0, 300.0, 100.0];
    refused_exactly(
        page("1000 0 d0 q 1000 0 0 1000 0 0 cm /Im Do Q"),
        rect,
        "Type 3",
    );
    let bytes = applied_exactly(
        page("1000 0 0 0 1000 1000 d1 0 0 m 1000 0 l 500 1000 l f"),
        rect,
    );
    assert!(!holds(&contents(&bytes, b"", 0), "(aaa)"));
    // Bitmap glyphs (inline stencil masks, as TeX writes) are glyph shapes.
    applied_exactly(
        page("1000 0 0 0 1000 1000 d1 q 1000 0 0 1000 0 0 cm BI /W 1 /H 1 /IM true ID \u{0} EI Q"),
        rect,
    );
}

/// A Type 0 font (Identity-H, not embedded) whose /ToUnicode reads codes
/// 1 to 5 as S, E, C, R, T.
fn cid_font() -> [String; 2] {
    let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /X def 1 begincodespacerange <0000> <FFFF> endcodespacerange 5 beginbfchar <0001> <0053> <0002> <0045> <0003> <0043> <0004> <0052> <0005> <0054> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end";
    [
        "<< /Type /Font /Subtype /Type0 /BaseFont /Arial /Encoding /Identity-H /DescendantFonts [<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Arial /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 600 >>] /ToUnicode 7 0 R >>".to_string(),
        stream("", cmap),
    ]
}

#[test]
fn a_moved_search_mark_over_cid_or_split_text_fails_the_raw_check() {
    // The raw check reads strings: glyph codes, or a word split over two
    // shows, do not hold the search text, but redact-search finds it.
    let secret = "<000100020003000400020005>";
    let [font, cmap] = cid_font();
    let cid = page_with(
        "",
        "/Font << /F2 6 0 R >>",
        &format!("BT /F2 12 Tf 100 700 Td {secret} Tj ET BT /F2 12 Tf 100 600 Td {secret} Tj ET"),
        &[HELV.to_string(), font, cmap],
    );
    let split = text_page(
        "BT /F1 12 Tf 100 700 Td (one SEC) Tj (RET two) Tj ET BT /F1 12 Tf 100 600 Td (one SEC) Tj (RET two) Tj ET",
    );
    // In a layer the document hides: redact-search reads every layer.
    let [font, cmap] = cid_font();
    let hidden = classic(&[
        "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [8 0 R] /D << /OFF [8 0 R] >> >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F2 6 0 R >> /Properties << /L1 8 0 R >> >> /Contents 4 0 R >>".to_string(),
        stream(
            "",
            &format!("/OC /L1 BDC BT /F2 12 Tf 100 700 Td {secret} Tj ET BT /F2 12 Tf 100 600 Td {secret} Tj ET EMC"),
        ),
        "null".to_string(),
        font,
        cmap,
        "<< /Type /OCG /Name (Hidden) >>".to_string(),
    ]);
    for bytes in [cid, split, hidden] {
        let (mut engine, mut session) = session_with(bytes);
        search(&mut engine, &mut session, "SECRET");
        let listed = ok(
            &mut engine,
            &mut session,
            json!({"op": "annots", "page": 0}),
        );
        assert_eq!(listed["items"].as_array().unwrap().len(), 2, "{listed}");
        let first = listed["items"][0]["id"].clone();
        ok(
            &mut engine,
            &mut session,
            json!({"op": "redact-set", "page": 0, "annotId": first, "rect": [400, 300, 500, 320]}),
        );
        let (reply, _) = apply_with(&mut engine, &mut session, all(true));
        assert_eq!(reply["reason"], "verify-failed", "{reply}");
        assert_eq!(
            reply["error"],
            "redact-apply: raw check failed: a searched text is still in the marked pages' content"
        );
    }
}

/// Two pages: page 1 draws object 5 (and page 2 object 7) at
/// 100..200 x 600..700 user space; `shared` objects from 9.
fn two_pages(first: &str, second: &str, resources: [&str; 2], more: &[String]) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 2 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << {} >> /Contents 4 0 R >>",
            resources[0]
        ),
        stream("", first),
        "null".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << {} >> /Contents 7 0 R >>",
            resources[1]
        ),
        stream("", second),
        "null".to_string(),
    ];
    objects.extend(more.iter().cloned());
    classic(&objects)
}

#[test]
fn a_soft_mask_another_image_shares_is_blanked_in_a_copy() {
    let rgb = |n: u32| {
        binary(
            &format!(
                "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8 /SMask {n} 0 R"
            ),
            &[200u8; 16],
        )
    };
    let source = two_pages(
        "q 100 0 0 100 100 600 cm /Im Do Q",
        "q 100 0 0 100 100 600 cm /Im Do Q",
        ["/XObject << /Im 9 0 R >>", "/XObject << /Im 10 0 R >>"],
        &[
            rgb(11),
            rgb(11),
            binary(
                "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8",
                &[255u8; 16],
            ),
        ],
    );
    let (mut engine, mut session) = session_with(source.clone());
    mark(&mut engine, &mut session, 0, [100.0, 92.0, 150.0, 142.0]);
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(preview["shared"], json!([]));
    let bytes = apply(&mut engine, &mut session);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    assert_eq!(samples(&doc, id(11)), vec![255u8; 16], "page 2's mask");
    let smask = |object: u32| {
        (doc.stream_dict(id(object)).unwrap())
            .get(b"SMask".as_slice())
            .and_then(Object::as_reference)
            .unwrap()
    };
    assert_eq!(smask(10), id(11));
    assert_ne!(smask(9), id(11));
    assert_eq!(samples(&doc, smask(9))[0], 0);
    assert_eq!(render(&source, 1), render(&bytes, 1));
}

#[test]
fn objects_drawn_by_patterns_groups_glyphs_or_as_masks_elsewhere_are_shared() {
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /Font << /F1 12 0 R >> >>",
        "BT /F1 12 Tf 0 50 Td (KEEPME) Tj ET",
    );
    let image = binary(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
        &[0],
    );
    let cases = [
        // A Form a tiling pattern draws on page 2.
        (
            "/Pattern << /P1 10 0 R >>",
            "/Pattern cs /P1 scn 0 0 612 792 re f",
            stream(
                "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 200 200] /XStep 200 /YStep 200 /Resources << /XObject << /X 9 0 R >> >>",
                "/X Do",
            ),
            form.clone(),
        ),
        // A Form a soft-mask group is.
        (
            "/ExtGState << /G << /SMask << /S /Alpha /G 9 0 R >> >> >>",
            "/G gs 0 0 100 100 re f",
            "null".to_string(),
            form.clone(),
        ),
        // An image another image takes as its soft mask.
        (
            "/XObject << /B 10 0 R >>",
            "q 100 0 0 100 100 100 cm /B Do Q",
            binary(
                "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /SMask 9 0 R",
                &[90],
            ),
            image.clone(),
        ),
    ];
    for (resources, second, holder, object) in cases {
        let bytes = two_pages(
            "q 1 0 0 1 100 600 cm /X Do Q",
            second,
            ["/XObject << /X 9 0 R >>", resources],
            &[object, holder, "null".to_string(), HELV.to_string()],
        );
        let (mut engine, mut session) = session_with(bytes);
        mark(&mut engine, &mut session, 0, [90.0, 90.0, 300.0, 200.0]);
        let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
        let shared = preview["shared"].as_array().unwrap();
        assert!(
            shared.iter().any(|s| s["otherPages"] == json!([1])),
            "{second}: {preview}"
        );
        let (reply, _) = apply_with(&mut engine, &mut session, all(true));
        assert_eq!(reply["reason"], "shared-content", "{second}: {reply}");
    }
}

#[test]
fn a_long_tj_array_is_rewritten_in_linear_time() {
    let elements = "(a)".repeat(100_000);
    let (mut engine, mut session) = session_with(text_page(&format!(
        "BT /F1 1 Tf 10 400 Td [{elements}] TJ ET"
    )));
    mark(&mut engine, &mut session, 0, [0.0, 380.0, 200.0, 400.0]);
    let started = std::time::Instant::now();
    apply(&mut engine, &mut session);
    // Quadratic, it took 6.5 s natively at 80,000 elements.
    assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
}

#[test]
fn an_image_past_the_pixel_bound_names_its_page() {
    let huge = stream(
        "/Type /XObject /Subtype /Image /Width 1000000000 /Height 1000000000 /ColorSpace /DeviceGray /BitsPerComponent 8",
        "abc",
    );
    let (mut engine, mut session) = session_with(two_pages(
        "",
        "q 200 0 0 200 50 550 cm /Im Do Q",
        ["", "/XObject << /Im 9 0 R >>"],
        &[huge],
    ));
    mark(&mut engine, &mut session, 1, [40.0, 40.0, 300.0, 250.0]);
    let (reply, _) = apply_with(&mut engine, &mut session, all(true));
    assert_eq!(reply["reason"], "too-large", "{reply}");
    assert_eq!(
        reply["error"],
        "redact-apply: an image on page 2 exceeds 40000000 pixels"
    );
}

#[test]
fn page_content_listed_again_and_again_is_refused_before_it_is_joined() {
    let comment = format!("%{}", "x".repeat(4095));
    let contents: String = " 5 0 R".repeat(70_000);
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Contents [4 0 R{contents}] >>"
        ),
        stream("", "0 0 1 rg 100 100 50 50 re f"),
        stream("", &comment),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    mark(&mut engine, &mut session, 0, [90.0, 600.0, 200.0, 700.0]);
    let (reply, _) = apply_with(&mut engine, &mut session, all(true));
    assert_eq!(reply["reason"], "too-large", "{reply}");
}

#[test]
fn widgets_leave_their_groups_as_one_by_one_removal_would() {
    let widget = |n: u32, state: &str, y: u32| {
        format!(
            "<< /Type /Annot /Subtype /Widget /Parent {n} 0 R /Rect [100 {y} 120 {} ] /AS /Off /AP << /N << /{state} 20 0 R /Off 20 0 R >> >> /P 3 0 R >>",
            y + 20
        )
    };
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R 9 0 R] >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Contents 4 0 R /Annots [6 0 R 7 0 R 8 0 R 10 0 R] >>".to_string(),
        stream("", ""),
        "<< /FT /Btn /Ff 49152 /T (radio) /V /a /DV /b /Kids [6 0 R 7 0 R 8 0 R] >>".to_string(),
        widget(5, "a", 700),
        widget(5, "b", 650),
        widget(5, "c", 100),
        "<< /FT /Tx /T (group) /Kids [10 0 R] >>".to_string(),
        "<< /Type /Annot /Subtype /Widget /Parent 9 0 R /Rect [100 600 200 620] /P 3 0 R >>".to_string(),
    ];
    let mut objects = objects;
    while objects.len() < 19 {
        objects.push("null".to_string());
    }
    objects.push(stream("/BBox [0 0 20 20]", ""));
    let (mut engine, mut session) = session_with(classic(&objects));
    mark(&mut engine, &mut session, 0, [90.0, 60.0, 300.0, 200.0]);
    let (reply, bytes) = apply_with(&mut engine, &mut session, all(false));
    assert_eq!(reply["ok"], true, "{reply}");
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let radio = doc.resolve_id(id(5)).as_dict().unwrap();
    assert_eq!(
        doc.dict_get(radio, b"Kids").as_array().unwrap(),
        [Object::Reference(id(8))]
    );
    assert_eq!(doc.dict_get(radio, b"V").as_name(), Some(&b"Off"[..]));
    assert!(radio.get(b"DV".as_slice()).is_none());
    let catalog = doc.resolve_id(doc.root()).as_dict().unwrap();
    let form = doc.dict_get(catalog, b"AcroForm").as_dict().unwrap();
    assert_eq!(
        doc.dict_get(form, b"Fields").as_array().unwrap(),
        [Object::Reference(id(5))],
        "the emptied group left /Fields"
    );
    let page = tandem_pdf_engine::app::info::pages(&doc)[0].dict;
    assert_eq!(
        doc.dict_get(page, b"Annots").as_array().unwrap(),
        [Object::Reference(id(8))]
    );
}

#[test]
fn many_widgets_under_a_mark_are_removed_in_linear_time() {
    let n = 32_000u32;
    let refs = (5..5 + n)
        .map(|i| format!("{i} 0 R"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut objects = vec![
        format!("<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [{refs}] >> >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Contents 4 0 R /Annots [{refs}] >>"
        ),
        stream("", ""),
    ];
    for i in 0..n {
        objects.push(format!(
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T (f{i}) /V (SECRET{i}) /Rect [10 {} 20 {}] /P 3 0 R >>",
            10 + i % 700,
            11 + i % 700
        ));
    }
    let (mut engine, mut session) = session_with(classic(&objects));
    mark(&mut engine, &mut session, 0, [0.0, 0.0, 612.0, 792.0]);
    let started = std::time::Instant::now();
    let (reply, bytes) = apply_with(&mut engine, &mut session, all(false));
    assert_eq!(reply["ok"], true, "{reply}");
    // One removal at a time, it took 8.9 s natively.
    assert!(started.elapsed().as_secs() < 4, "{:?}", started.elapsed());
    assert!(!holds(&everything(&bytes, b""), "SECRET"));
}

#[test]
fn mcids_under_a_mark_look_up_a_long_parent_tree_once() {
    // The page's entry is empty (so the view's text reader stays linear):
    // what is measured is finding it, once per MCID it was.
    let (n, entries) = (30_000u32, 45_000u32);
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 6 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /StructParents {entries} /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
        ),
        stream(
            "",
            &format!(
                "BT /F1 1 Tf 10 10 Td {} ET",
                (0..n)
                    .map(|i| format!("/Span <</MCID {i}>> BDC (x) Tj EMC"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        ),
        HELV.to_string(),
        format!(
            "<< /Type /StructTreeRoot /ParentTree << /Nums [{} {entries} []] >> >>",
            (0..entries)
                .map(|i| format!("{i} []"))
                .collect::<Vec<_>>()
                .join(" "),
        ),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    mark(&mut engine, &mut session, 0, [0.0, 700.0, 612.0, 792.0]);
    let started = std::time::Instant::now();
    let (reply, _) = apply_with(&mut engine, &mut session, all(false));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(started.elapsed().as_secs() < 4, "{:?}", started.elapsed());
}

#[test]
fn the_struct_toggle_clears_named_property_lists_and_reports_them() {
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 8 0 R /MarkInfo << /Marked true >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> /Properties << /P0 7 0 R >> >> /StructParents 0 >>".to_string(),
        stream(
            "",
            "/Span /P0 BDC BT /F1 12 Tf 72 400 Td (unrelated words) Tj ET EMC\nBT /F1 12 Tf 72 700 Td (KEEP ZQXSECRETQZ end) Tj ET",
        ),
        HELV.to_string(),
        "null".to_string(),
        "<< /MCID 0 /ActualText (NAMEDALTSECRET) >>".to_string(),
        "<< /Type /StructTreeRoot /K [9 0 R] >>".to_string(),
        "<< /Type /StructElem /S /Span /P 8 0 R /Pg 3 0 R /K 0 >>".to_string(),
    ];
    for chosen in [true, false] {
        let (mut engine, mut session) = session_with(classic(&objects));
        mark(&mut engine, &mut session, 0, [60.0, 70.0, 560.0, 120.0]);
        let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
        assert_eq!(preview["sanitize"]["struct"], 1);
        let mut sanitize = all(true);
        sanitize["struct"] = json!(chosen);
        let (reply, bytes) = apply_with(&mut engine, &mut session, sanitize);
        assert_eq!(reply["ok"], true, "{reply}");
        let (cleared, kept) = if chosen { (1, 0) } else { (0, 1) };
        assert_eq!(reply["sanitized"]["struct"], cleared);
        assert_eq!(reply["kept"]["struct"], kept);
        assert_eq!(holds(&everything(&bytes, b""), "NAMEDALTSECRET"), !chosen);
        assert!(!holds(&everything(&bytes, b""), "ZQXSECRETQZ"));
    }
}

#[test]
fn a_page_past_the_views_pixel_bound_is_checked_at_a_smaller_scale() {
    let bytes = page_with(
        "",
        "/Font << /F1 5 0 R >>",
        "BT /F1 12 Tf 100 8700 Td (SECRET) Tj ET",
        &[HELV.to_string()],
    );
    let text = String::from_utf8_lossy(&bytes)
        .replace("/MediaBox [0 0 612 792]", "/MediaBox [0 0 9000 9000]");
    let (mut engine, mut session) = session_with(fix_xref(text.as_bytes()));
    search(&mut engine, &mut session, "SECRET");
    let bytes = apply(&mut engine, &mut session);
    assert!(!holds(&everything(&bytes, b""), "SECRET"));
}

/// A classic-xref file whose objects moved: the xref rebuilt.
fn fix_xref(bytes: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes).to_string();
    let body = &text[..text.find("xref\n").unwrap()];
    let mut offsets = Vec::new();
    let mut at = 0;
    while let Some(found) = body[at..].find(" 0 obj\n") {
        let start = body[..at + found].rfind('\n').map_or(0, |n| n + 1);
        offsets.push(start);
        at += found + 7;
    }
    let mut out = body.as_bytes().to_vec();
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
    );
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[test]
fn an_image_partly_under_a_mark_is_checked_and_loses_its_alternate_text() {
    // A stencil mask painted with a pattern that shows text.
    let stencil = binary(
        "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ImageMask true",
        &[0, 0],
    );
    let painted = with_objects(
        "/Font << /F1 5 0 R >> /Pattern << /P1 6 0 R >> /XObject << /Im 7 0 R >>",
        "/Pattern cs /P1 scn q 100 0 0 100 100 600 cm /Im Do Q",
        &[
            stream(CELL, "BT /F1 12 Tf 10 20 Td (PATSECRET) Tj ET"),
            stencil,
        ],
    );
    refused_exactly(painted, [100.0, 92.0, 150.0, 142.0], "pattern");
    // The figure's /Alt describes what the blanked pixels showed.
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 7 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /StructParents 0 /Resources << /XObject << /Im 5 0 R >> >> /Contents 4 0 R >>".to_string(),
        stream("", "/Figure <</MCID 0>> BDC q 100 0 0 100 100 600 cm /Im Do Q EMC"),
        binary(
            "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8",
            &[200u8; 16],
        ),
        "<< /Nums [0 [8 0 R]] >>".to_string(),
        "<< /Type /StructTreeRoot /K [8 0 R] /ParentTree 6 0 R >>".to_string(),
        "<< /Type /StructElem /S /Figure /P 7 0 R /Pg 3 0 R /K 0 /Alt (ALTSECRET) >>".to_string(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    mark(&mut engine, &mut session, 0, [100.0, 92.0, 150.0, 142.0]);
    let (reply, bytes) = apply_with(&mut engine, &mut session, all(false));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(!holds(&everything(&bytes, b""), "ALTSECRET"));
    assert!(holds(&contents(&bytes, b"", 0), "/Im Do"));
}

// ------------------------------------------------- acceptance review fixes

/// Page 1 shows SECRETOC; page 2 shows it under optional content (group 7,
/// off in /D when `catalog` has /OCProperties; object 8 a membership or the
/// Form drawing it) and KEEPTEXT outside it.
fn optional_pages(catalog: &str, content: &str, resources: &str, eight: String) -> Vec<u8> {
    classic(&[
        format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>"),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> >> /Contents 5 0 R >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> {resources} >> /Contents 9 0 R >>"),
        stream("", "BT /F1 12 Tf 100 700 Td (SECRETOC page one) Tj ET"),
        HELV.to_string(),
        "<< /Type /OCG /Name (Layer) >>".to_string(),
        eight,
        stream("", &format!("{content} BT /F1 12 Tf 100 300 Td (KEEPTEXT) Tj ET")),
    ])
}

#[test]
fn text_a_membership_shows_only_while_a_group_is_off_is_found_and_removed() {
    // Reference viewers draw page 2's secret in each (the group is off, or there is
    // no /OCProperties and every membership shows); hayro's view with every
    // group on hid it, so redact-search missed it and apply kept it.
    let off = "/OCProperties << /OCGs [7 0 R] /D << /OFF [7 0 R] /Order [7 0 R] >> >>";
    let marked = "/OC /oc1 BDC BT /F1 12 Tf 100 700 Td (SECRETOC page two) Tj ET EMC";
    let named = "/Properties << /oc1 8 0 R >>";
    let form =
        "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> >>";
    let cases = [
        (
            "AllOff",
            off,
            marked,
            named,
            "<< /Type /OCMD /OCGs [7 0 R] /P /AllOff >>".to_string(),
        ),
        (
            "AnyOff",
            off,
            marked,
            named,
            "<< /Type /OCMD /OCGs [7 0 R] /P /AnyOff >>".to_string(),
        ),
        (
            "VE Not",
            off,
            marked,
            named,
            "<< /Type /OCMD /OCGs [7 0 R] /P /AnyOff /VE [/Not 7 0 R] >>".to_string(),
        ),
        (
            "no /OCProperties",
            "",
            marked,
            named,
            "<< /Type /OCMD /OCGs [7 0 R] /P /AllOff >>".to_string(),
        ),
        (
            "a Form's own membership",
            off,
            "/Fm Do",
            "/XObject << /Fm 8 0 R >>",
            stream(
                &format!("{form} /OC << /Type /OCMD /OCGs [7 0 R] /P /AllOff >>"),
                "BT /F1 12 Tf 100 700 Td (SECRETOC page two) Tj ET",
            ),
        ),
    ];
    for (name, catalog, content, resources, eight) in cases {
        let (mut engine, mut session) =
            session_with(optional_pages(catalog, content, resources, eight));
        let found = ok(
            &mut engine,
            &mut session,
            json!({"op": "redact-search", "text": "SECRETOC"}),
        );
        assert_eq!(found["added"], 2, "{name}: {found}");
        assert_eq!(found["pages"], json!([0, 1]), "{name}");
        let bytes = apply(&mut engine, &mut session);
        assert!(!holds(&everything(&bytes, b""), "SECRETOC"), "{name}");
        assert!(view_text(&bytes, "").contains("KEEPTEXT"), "{name}");
    }
}

#[test]
fn the_raw_check_reads_text_a_membership_shows_only_while_a_group_is_off() {
    // Page 2's mark moved off its text (shown in two strings, which only a
    // search finds): the search mark on page 1 remains, so the raw check
    // searches page 2 with every membership shown and finds the text left.
    let off = "/OCProperties << /OCGs [7 0 R] /D << /OFF [7 0 R] >> >>";
    let bytes = optional_pages(
        off,
        "/OC /oc1 BDC BT /F1 12 Tf 100 700 Td (SECR) Tj (ETOC page two) Tj ET EMC",
        "/Properties << /oc1 8 0 R >>",
        "<< /Type /OCMD /OCGs [7 0 R] /P /AllOff >>".to_string(),
    );
    let (mut engine, mut session) = session_with(bytes);
    search(&mut engine, &mut session, "SECRETOC");
    let listed = ok(
        &mut engine,
        &mut session,
        json!({"op": "annots", "page": 1}),
    );
    for item in listed["items"].as_array().unwrap() {
        ok(
            &mut engine,
            &mut session,
            json!({"op": "annot-delete", "page": 1, "annotId": item["id"]}),
        );
    }
    mark(&mut engine, &mut session, 1, [300.0, 100.0, 400.0, 120.0]);
    let (reply, _) = apply_with(&mut engine, &mut session, all(false));
    assert_eq!(reply["reason"], "verify-failed", "{reply}");
    assert_eq!(
        reply["error"],
        "redact-apply: raw check failed: a searched text is still in the marked pages' content"
    );
}

/// One page drawing Form 5 (`bbox`, showing ACCOUNT 12345678 holder at
/// 100 700) twice, the second time 200 points lower.
fn form_twice(bbox: &str) -> Vec<u8> {
    page_with(
        "",
        "/Font << /F1 6 0 R >> /XObject << /Fm 5 0 R >>",
        "q /Fm Do Q q 1 0 0 1 0 -200 cm /Fm Do Q BT /F1 12 Tf 100 300 Td (KEEPTEXT) Tj ET",
        &[
            stream(
                &format!(
                    "/Type /XObject /Subtype /Form /BBox {bbox} /Resources << /Font << /F1 6 0 R >> >>"
                ),
                "BT /F1 12 Tf 100 700 Td (ACCOUNT 12345678 holder) Tj ET",
            ),
            HELV.to_string(),
        ],
    )
}

#[test]
fn a_page_size_form_drawn_twice_and_marked_alike_applies() {
    for bbox in ["[0 0 612 792]", "[90 690 400 715]"] {
        let (mut engine, mut session) = session_with(form_twice(bbox));
        search(&mut engine, &mut session, "12345678");
        let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
        assert_eq!(preview["shared"], json!([]), "{bbox}");
        let bytes = apply(&mut engine, &mut session);
        assert!(!holds(&everything(&bytes, b""), "12345678"), "{bbox}");
        let text = view_text(&bytes, "");
        assert_eq!(text.matches("ACCOUNT").count(), 2, "{bbox}: {text}");
        assert_eq!(text.matches("holder").count(), 2, "{bbox}: {text}");
    }
    // One occurrence marked: the other placement shows it unmarked.
    let (mut engine, mut session) = session_with(form_twice("[0 0 612 792]"));
    search(&mut engine, &mut session, "12345678");
    let listed = ok(
        &mut engine,
        &mut session,
        json!({"op": "annots", "page": 0}),
    );
    let second = listed["items"][1]["id"].clone();
    ok(
        &mut engine,
        &mut session,
        json!({"op": "annot-delete", "page": 0, "annotId": second}),
    );
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(
        preview["shared"],
        json!([{"page": 0, "kind": "form", "otherPages": [0]}])
    );
    let (reply, _) = apply_with(&mut engine, &mut session, all(false));
    assert_eq!(reply["reason"], "shared-content", "{reply}");
}

/// Two pages each drawing letterhead Form 7 (a page-size /BBox, its text at
/// the top) with `drawn`, and body text.
fn letterhead(drawn: &str) -> Vec<u8> {
    let page = |contents: u32| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 8 0 R >> /XObject << /LH 7 0 R >> >> /Contents {contents} 0 R >>"
        )
    };
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        page(5),
        page(6),
        stream(
            "",
            &format!(
                "{drawn} BT /F1 12 Tf 100 400 Td (SECRETBODY one) Tj ET BT /F1 12 Tf 100 300 Td (KEEPTEXT) Tj ET"
            ),
        ),
        stream(
            "",
            &format!("{drawn} BT /F1 12 Tf 100 400 Td (page two body) Tj ET"),
        ),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 8 0 R >> >>",
            "BT /F1 9 Tf 40 760 Td (LETTERHEAD LOGO) Tj ET",
        ),
        HELV.to_string(),
    ])
}

#[test]
fn a_letterhead_form_is_not_touched_by_a_mark_over_other_content() {
    for drawn in ["q /LH Do Q", "q 1 0 0 1 0 0 cm /LH Do Q", "/LH Do"] {
        let (mut engine, mut session) = session_with(letterhead(drawn));
        mark(&mut engine, &mut session, 0, [90.0, 375.0, 330.0, 400.0]);
        let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
        assert_eq!(preview["shared"], json!([]), "{drawn}");
        let bytes = apply(&mut engine, &mut session);
        assert!(!holds(&everything(&bytes, b""), "SECRETBODY"), "{drawn}");
        let text = view_text(&bytes, "");
        assert_eq!(text.matches("LETTERHEAD").count(), 2, "{drawn}: {text}");
        // A mark over the letterhead itself on page 1 alone is shared.
        let (mut engine, mut session) = session_with(letterhead(drawn));
        mark(&mut engine, &mut session, 0, [30.0, 20.0, 200.0, 40.0]);
        let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
        assert_eq!(
            preview["shared"],
            json!([{"page": 0, "kind": "form", "otherPages": [1]}]),
            "{drawn}"
        );
    }
}

#[test]
fn the_structure_element_of_redacted_content_loses_its_title() {
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 6 0 R /MarkInfo << /Marked true >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /StructParents 0 /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string(),
        stream(
            "",
            "/P <</MCID 0>> BDC BT /F1 12 Tf 100 700 Td (SECRET) Tj ET EMC\n\
            /P <</MCID 1>> BDC BT /F1 12 Tf 100 400 Td (other) Tj ET EMC",
        ),
        HELV.to_string(),
        "<< /Type /StructTreeRoot /K [9 0 R] /ParentTree 7 0 R >>".to_string(),
        "<< /Nums [0 [8 0 R 10 0 R]] >>".to_string(),
        "<< /Type /StructElem /S /P /P 9 0 R /Pg 3 0 R /K 0 /T (SECRETT) /ActualText (SECRETA) /Alt (SECRETB) /E (SECRETE) /Lang (en) /ID (id8) >>".to_string(),
        "<< /Type /StructElem /S /Sect /P 6 0 R /K [8 0 R 10 0 R] /T (SECTIONT) >>".to_string(),
        "<< /Type /StructElem /S /P /P 9 0 R /Pg 3 0 R /K 1 /T (OTHERT) >>".to_string(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    search(&mut engine, &mut session, "SECRET");
    let (reply, bytes) = apply_with(&mut engine, &mut session, all(false));
    assert_eq!(reply["ok"], true, "{reply}");
    let found = everything(&bytes, b"");
    for gone in ["SECRETT", "SECRETA", "SECRETB", "SECRETE"] {
        assert!(!holds(&found, gone), "{gone}");
    }
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let element = doc.resolve_id(id(8)).as_dict().unwrap();
    for key in [b"T".as_slice(), b"ActualText", b"Alt", b"E"] {
        assert!(
            element.get(key).is_none(),
            "{}",
            String::from_utf8_lossy(key)
        );
    }
    for key in [b"Lang".as_slice(), b"ID", b"K", b"S"] {
        assert!(
            element.get(key).is_some(),
            "{}",
            String::from_utf8_lossy(key)
        );
    }
    // An ancestor's title and an untouched element's stay.
    assert!(holds(&found, "SECTIONT") && holds(&found, "OTHERT"));
}

#[test]
fn sanitize_clears_rich_values_choices_configuration_names_and_chained_link_targets() {
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [7 0 R 11 0 R] >> /OCProperties << /OCGs [6 0 R] /D << /Name (SECRETCFG1) >> /Configs [<< /Name (SECRETCFG2) >> 9 0 R] >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R /Annots [7 0 R 8 0 R 11 0 R] >>".to_string(),
        stream("", "BT /F1 12 Tf 100 700 Td (body text) Tj ET"),
        HELV.to_string(),
        "<< /Type /OCG /Name (Layer) >>".to_string(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (f) /V (SECRETV) /RV (<p>SECRETRV</p>) /Rect [300 600 400 620] /P 3 0 R /DA (/Helv 0 Tf 0 g) >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [100 600 200 620] /A << /S /URI /URI (http://a.example/SECRETURI) /Next [<< /S /URI /URI (SECRETNEXT1) >> 10 0 R] >> >>".to_string(),
        "<< /Name (SECRETCFG3) >>".to_string(),
        "<< /S /Launch /Win << /F (SECRETWIN) /P (SECRETP) >> /Unix (SECRETUNIX) /Next << /S /Launch /F (SECRETNEXT3) >> >>".to_string(),
        "<< /Type /Annot /Subtype /Widget /FT /Ch /Ff 131072 /T (c) /V (pick) /Opt [(pick) (other)] /I [0] /Rect [300 500 400 520] /P 3 0 R >>".to_string(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    mark(&mut engine, &mut session, 0, [90.0, 80.0, 200.0, 100.0]);
    let bytes = apply(&mut engine, &mut session);
    let found = everything(&bytes, b"");
    // The choice's selected index is gone with its value.
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let choice = doc.resolve_id(id(11)).as_dict().unwrap();
    assert!(choice.get(b"V".as_slice()).is_none() && choice.get(b"I".as_slice()).is_none());
    assert!(
        choice.get(b"Opt".as_slice()).is_some(),
        "the options are the form's"
    );
    for gone in [
        "SECRETV",
        "SECRETRV",
        "SECRETCFG1",
        "SECRETCFG2",
        "SECRETCFG3",
        "SECRETURI",
        "SECRETNEXT1",
        "SECRETWIN",
        "SECRETP",
        "SECRETUNIX",
        "SECRETNEXT3",
    ] {
        assert!(!holds(&found, gone), "{gone}");
    }
    // Off, each surface stays.
    let (mut engine, mut session) = session_with(classic(&objects));
    mark(&mut engine, &mut session, 0, [90.0, 80.0, 200.0, 100.0]);
    let (reply, bytes) = apply_with(&mut engine, &mut session, all(false));
    assert_eq!(reply["ok"], true, "{reply}");
    let found = everything(&bytes, b"");
    for kept in [
        "SECRETRV",
        "SECRETCFG1",
        "SECRETCFG3",
        "SECRETNEXT1",
        "SECRETWIN",
    ] {
        assert!(holds(&found, kept), "{kept}");
    }
}

// ------------------------------------------- acceptance review fixes (round 2)

/// Two pages drawing Form 8 (box `bbox`; `extra`, then LEFTPART at 10 40
/// and OFFPAGE at 330 40 of its space) with `first` and `second`.
fn off_edge(first: &str, second: &str, bbox: &str, extra: &str) -> Vec<u8> {
    let page = |contents: u32| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> /XObject << /F 8 0 R >> >> /Contents {contents} 0 R >>"
        )
    };
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        page(5),
        page(7),
        stream("", first),
        HELV.to_string(),
        stream("", second),
        stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox {bbox} /Resources << /Font << /F1 6 0 R >> >>"
            ),
            &format!(
                "{extra} BT /F1 20 Tf 10 40 Td (LEFTPART) Tj ET BT /F1 20 Tf 330 40 Td (OFFPAGE) Tj ET"
            ),
        ),
    ])
}

/// The Form at the right edge: OFFPAGE lies past it.
const AT_EDGE: &str = "q 1 0 0 1 300 600 cm /F Do Q BT /F1 12 Tf 72 300 Td (BODYTEXT) Tj ET";
/// The Form wholly on the page.
const ON_PAGE: &str = "q 1 0 0 1 50 300 cm /F Do Q BT /F1 12 Tf 72 200 Td (PAGETWO) Tj ET";
/// A mark on page 1's right edge, level with OFFPAGE where `AT_EDGE` draws it.
const EDGE_MARK: [f64; 4] = [560.0, 120.0, 612.0, 170.0];

#[test]
fn text_past_the_page_edge_inside_a_form_is_shared_where_another_placement_shows_it() {
    // The glyph rule takes a glyph past the page edge when a mark reaches
    // that edge, and apply rewrites the Form once for all its placements:
    // where another placement shows OFFPAGE it would lose it (review probe
    // p33). `shared` compares what apply changes, so it reports them.
    let other_page = json!([{"page": 0, "kind": "form", "otherPages": [1]}]);
    let both = json!([
        {"page": 0, "kind": "form", "otherPages": [0]},
        {"page": 0, "kind": "form", "otherPages": [1]},
    ]);
    let cases = [
        ("another page", AT_EDGE, "[0 0 420 100]", "", &other_page),
        ("a tight box", AT_EDGE, "[0 30 420 70]", "", &other_page),
        (
            "a path beside it",
            AT_EDGE,
            "[0 0 420 100]",
            "0 0 1 rg 330 30 80 40 re f",
            &other_page,
        ),
        (
            "the same page",
            "q 1 0 0 1 300 600 cm /F Do Q q 1 0 0 1 50 200 cm /F Do Q BT /F1 12 Tf 72 100 Td (BODYTEXT) Tj ET",
            "[0 0 420 100]",
            "",
            &both,
        ),
        (
            "a placement wholly past the edge",
            "q 1 0 0 1 620 600 cm /F Do Q BT /F1 12 Tf 72 300 Td (BODYTEXT) Tj ET",
            "[0 0 420 100]",
            "",
            &other_page,
        ),
    ];
    for (name, first, bbox, extra, shared) in cases {
        let (mut engine, mut session) = session_with(off_edge(first, ON_PAGE, bbox, extra));
        mark(&mut engine, &mut session, 0, EDGE_MARK);
        let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
        assert_eq!(&preview["shared"], shared, "{name}: {preview}");
        let (reply, _) = apply_with(&mut engine, &mut session, all(false));
        assert_eq!(reply["reason"], "shared-content", "{name}: {reply}");
    }
    // Both placements past the edge and marked alike: nothing shows it.
    let (mut engine, mut session) = session_with(off_edge(AT_EDGE, AT_EDGE, "[0 0 420 100]", ""));
    mark(&mut engine, &mut session, 0, EDGE_MARK);
    mark(&mut engine, &mut session, 1, EDGE_MARK);
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(preview["shared"], json!([]), "{preview}");
    let bytes = apply(&mut engine, &mut session);
    assert!(!holds(&everything(&bytes, b""), "OFFPAGE"));
    assert_eq!(view_text(&bytes, "").matches("LEFTPART").count(), 2);
}

/// Two pages each drawing Form 8 (a page-size box; `form` its content)
/// with `/F Do`; page 1 also shows SECRETBODY at 100 400.
fn form_on_two_pages(form: &str) -> Vec<u8> {
    let page = |contents: u32| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> /XObject << /F 8 0 R >> >> /Contents {contents} 0 R >>"
        )
    };
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        page(5),
        page(7),
        stream(
            "",
            "/F Do BT /F1 12 Tf 100 400 Td (SECRETBODY one) Tj ET BT /F1 12 Tf 100 300 Td (KEEPTEXT) Tj ET",
        ),
        HELV.to_string(),
        stream("", "/F Do BT /F1 12 Tf 100 400 Td (page two body) Tj ET"),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> >>",
            form,
        ),
    ])
}

#[test]
fn a_form_is_touched_where_apply_changes_it_clips_included() {
    // A page-size clip in a letterhead under a mark over body text: apply
    // cuts the clip where nothing of the Form is drawn, so nothing changes
    // at the other placement.
    let letterhead = "q 0 0 612 792 re W n BT /F1 9 Tf 40 760 Td (LETTERHEAD LOGO) Tj ET Q";
    let (mut engine, mut session) = session_with(form_on_two_pages(letterhead));
    mark(&mut engine, &mut session, 0, [90.0, 375.0, 330.0, 400.0]);
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(preview["shared"], json!([]), "{preview}");
    let bytes = apply(&mut engine, &mut session);
    assert!(!holds(&everything(&bytes, b""), "SECRETBODY"));
    assert_eq!(view_text(&bytes, "").matches("LETTERHEAD").count(), 2);
    // A mark over the left edge of the first M, short of its middle: the
    // glyph stays, so without a clip the Form does not change and page 2
    // draws as before.
    let wide = "BT /F1 40 Tf 100 600 Td (MMMM) Tj ET";
    let sliver = [90.0, 160.0, 104.0, 200.0];
    let source = form_on_two_pages(wide);
    let (mut engine, mut session) = session_with(source.clone());
    mark(&mut engine, &mut session, 0, sliver);
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(preview["shared"], json!([]), "{preview}");
    let bytes = apply(&mut engine, &mut session);
    same_outside(&render(&source, 1), &render(&bytes, 1), &[], 0.0);
    assert!(view_text(&bytes, "").contains("MMMM"));
    // Under a clip apply cuts, the same mark hides that sliver of the M
    // wherever the Form is drawn: shared with page 2. So is text at size 0
    // under the mark, which apply removes from the Form (page 2's text
    // would lose it).
    let clipped = format!("q 50 50 500 700 re W n {wide} Q");
    let zero = "BT /F1 0 Tf 100 600 Td (ZEROSIZE) Tj ET BT /F1 40 Tf 300 600 Td (MMMM) Tj ET";
    for form in [clipped.as_str(), zero] {
        let (mut engine, mut session) = session_with(form_on_two_pages(form));
        mark(&mut engine, &mut session, 0, sliver);
        let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
        assert_eq!(
            preview["shared"],
            json!([{"page": 0, "kind": "form", "otherPages": [1]}]),
            "{form}: {preview}"
        );
    }
}

/// Every object a copy holds, read as the app reads it (a stream naming
/// an external file from its own bytes).
fn everything_read(bytes: &[u8]) -> Vec<u8> {
    let options = tandem_pdf_engine::document::OpenOptions {
        external_streams: true,
        ..Default::default()
    };
    let doc = Document::open_with(bytes, DocumentLimits::default(), b"", options).unwrap();
    let mut out = Vec::new();
    for object in doc.objects() {
        if object.stream.is_some() {
            if let Ok(decoded) = doc.decoded_stream(object.id, StreamLimits::default()) {
                out.extend_from_slice(&decoded);
                out.push(b'\n');
            }
        }
    }
    out
}

/// Page 1 shows AREAONE and `one`; page 2 (content object 7) shows `two`,
/// directly or (`form`) in Form 8, in a stream naming an external file.
/// `catalog` adds catalog entries (object 9 is a group).
fn external_file(form: bool, catalog: &str, one: &str, two: &str) -> Vec<u8> {
    let shown =
        format!("BT /F1 12 Tf 100 600 Td ({two}) Tj ET BT /F1 12 Tf 100 300 Td (KEEPEX) Tj ET");
    let (seven, eight) = match form {
        true => (
            stream("", "/Fx Do"),
            stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> >> /F (missing.bin)",
                &shown,
            ),
        ),
        false => (stream("/F (missing.bin)", &shown), "null".to_string()),
    };
    classic(&[
        format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>"),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> >> /Contents 5 0 R >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> /XObject << /Fx 8 0 R >> >> /Contents 7 0 R >>".to_string(),
        stream(
            "",
            &format!("BT /F1 12 Tf 100 700 Td (AREAONE) Tj ET BT /F1 12 Tf 100 600 Td ({one}) Tj ET"),
        ),
        HELV.to_string(),
        seven,
        eight,
        "<< /Type /OCG /Name (Layer) >>".to_string(),
    ])
}

#[test]
fn a_copy_keeping_a_stream_that_names_an_external_file_verifies() {
    // The copy is read as the app reads documents: before, its strict
    // reopen refused the /F stream ("output is not one compact revision").
    for form in [false, true] {
        let (mut engine, mut session) =
            session_with(external_file(form, "", "first page", "second page"));
        mark(&mut engine, &mut session, 0, [90.0, 80.0, 200.0, 100.0]);
        let bytes = apply(&mut engine, &mut session);
        let found = everything_read(&bytes);
        assert!(!holds(&found, "AREAONE"), "form {form}");
        assert!(holds(&found, "KEEPEX"), "form {form}");
        let options = tandem_pdf_engine::document::OpenOptions {
            external_streams: true,
            ..Default::default()
        };
        let doc = Document::open_with(&bytes, DocumentLimits::default(), b"", options).unwrap();
        let named = if form { 8 } else { 7 };
        let dict = doc.stream_dict(id(named)).unwrap();
        assert_eq!(
            dict.get(b"F".as_slice()),
            Some(&Object::String(b"missing.bin".to_vec())),
            "form {form}"
        );
    }
}

#[test]
fn search_with_optional_content_reads_what_the_view_reads() {
    // The unlayered view starts from the cached view's preparation: a
    // stream naming an external file is read from its own bytes (before,
    // one hit, and page 2 kept the secret), and is checked so in the copy
    // (the Form keeps its /F: the copy's view reads it as the session's).
    let layered = "/OCProperties << /OCGs [9 0 R] /D << /Order [9 0 R] >> >>";
    for catalog in ["", layered] {
        for form in [false, true] {
            let (mut engine, mut session) =
                session_with(external_file(form, catalog, "SECRETEX one", "SECRETEX two"));
            let found = ok(
                &mut engine,
                &mut session,
                json!({"op": "redact-search", "text": "SECRETEX"}),
            );
            assert_eq!(found["added"], 2, "{catalog} form {form}: {found}");
            assert_eq!(found["pages"], json!([0, 1]), "{catalog} form {form}");
            let bytes = apply(&mut engine, &mut session);
            let left = everything_read(&bytes);
            assert!(!holds(&left, "SECRETEX"), "{catalog} form {form}");
            assert!(holds(&left, "KEEPEX"), "{catalog} form {form}");
        }
    }
    // Page content encoded with an image filter is left out, as the cached
    // view (and reference viewers) leave it: before, the layered search refused.
    let samples = |catalog: &str| {
        classic(&[
            format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>"),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> >> /Contents 5 0 R >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> >> /Contents [7 0 R 8 0 R] >>".to_string(),
            stream("", "BT /F1 12 Tf 100 700 Td (SECRETSM one) Tj ET"),
            HELV.to_string(),
            stream("", "BT /F1 12 Tf 100 700 Td (SECRETSM two) Tj ET"),
            stream("/Filter /DCTDecode", "not a picture"),
            "<< /Type /OCG /Name (Layer) >>".to_string(),
        ])
    };
    for catalog in ["", layered] {
        let (mut engine, mut session) = session_with(samples(catalog));
        let found = ok(
            &mut engine,
            &mut session,
            json!({"op": "redact-search", "text": "SECRETSM"}),
        );
        assert_eq!(found["added"], 2, "{catalog}: {found}");
    }
}

#[test]
fn pages_past_the_views_resource_budget_refuse_search_and_apply() {
    // 1,100 pages sharing one /Resources of 4,000 fonts: past the view's
    // budget, which the unlayered view and apply's copy now keep too
    // (before: the layered search built the view, 396 MB at 1,200 pages in
    // review; apply opened its copy's view, 473 MB).
    let pages = 1100;
    let fonts: String = (0..4000).map(|i| format!("/F{i} 5 0 R ")).collect();
    let build = |catalog: &str| {
        let mut objects = vec![
            format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>"),
            format!(
                "<< /Type /Pages /Kids [{}] /Count {pages} >>",
                (0..pages)
                    .map(|i| format!("{} 0 R ", 8 + i))
                    .collect::<String>()
            ),
            format!("<< /Font << {fonts} >> >>"),
            stream("", "BT /F0 12 Tf 100 700 Td (SECRETBIG text) Tj ET"),
            HELV.to_string(),
            "<< /Type /OCG /Name (Layer) >>".to_string(),
            "null".to_string(),
        ];
        objects.extend((0..pages).map(|_| {
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources 3 0 R /Contents 4 0 R >>"
                .to_string()
        }));
        classic(&objects)
    };
    let layered = "/OCProperties << /OCGs [6 0 R] /D << /Order [6 0 R] >> >>";
    for catalog in ["", layered] {
        let (mut engine, mut session) = session_with(build(catalog));
        let reply = op(
            &mut engine,
            &mut session,
            json!({"op": "redact-search", "text": "SECRETBIG"}),
        );
        assert_eq!(reply["ok"], false, "{catalog}: {reply}");
        let error = reply["error"].as_str().unwrap();
        assert!(
            error.contains("larger than the view's limit"),
            "{catalog}: {error}"
        );
        mark(&mut engine, &mut session, 0, [90.0, 80.0, 300.0, 100.0]);
        let (reply, _) = apply_with(&mut engine, &mut session, all(false));
        assert_eq!(reply["reason"], "too-large", "{catalog}: {reply}");
        assert_eq!(
            reply["error"],
            "redact-apply: the pages' resources are larger than the view's limit, so the copy cannot be checked"
        );
    }
}
