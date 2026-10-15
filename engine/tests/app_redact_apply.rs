// SPDX-License-Identifier: AGPL-3.0-or-later
//! redact-apply on the app session: what the marks cover leaves the copy
//! (text, image pixels, paths, annotations, alternate texts), the session
//! is unchanged, the refusals come in engine.ts's order, and the copy is
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
    // As engine.ts on MuPDF: the tagged text's structure element lost its
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
        "redact-apply: raw check failed for \"SECRET\""
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
fn size_zero_text_goes_and_spacing_it_cannot_express_refuses() {
    let (mut engine, mut session) =
        session_with(text_page("BT /F1 0 Tf 100 700 Td (SECRET) Tj ET"));
    mark(&mut engine, &mut session, 0, [90.0, 70.0, 300.0, 110.0]);
    let bytes = apply(&mut engine, &mut session);
    assert!(!holds(&everything(&bytes, b""), "SECRET"));
    let (mut engine, mut session) = session_with(text_page(
        "BT /F1 0 Tf 5 Tc 100 700 Td (SECRET) Tj (x) Tj ET",
    ));
    mark(&mut engine, &mut session, 0, [90.0, 70.0, 300.0, 110.0]);
    let (reply, _) = apply_with(&mut engine, &mut session, all(true));
    // At 0 Tf the pen moves by Tc alone, which no adjustment can say.
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
    let alpha = samples(&doc, id(6));
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
