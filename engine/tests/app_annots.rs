// SPDX-License-Identifier: AGPL-3.0-or-later
//! Annotation ops on the app session: each kind written as MuPDF writes it
//! (dictionary and appearance stream), refusals in engine.ts order with
//! nothing journaled, undo and redo exact, deletes that leave nothing
//! reaching the annotation, and the view drawing what MuPDF draws for
//! annotations without an appearance or with NoZoom and NoRotate.
use serde_json::{Value, json};
use tandem_pdf_engine::app::annots::{self, PageSpace};
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Dictionary, Object, ObjectId, decode_text_string};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::view::RenderOptions;
use tandem_pdf_engine::writer::{DocumentWriteLimits, WriteOptions, write_document};

fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
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

/// One page (object 3) with `page` entries and annotations from object 5.
fn page_with(page: &str, annots: &[&str]) -> Vec<u8> {
    let list: Vec<String> = (0..annots.len())
        .map(|i| format!("{} 0 R", 5 + i))
        .collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] {page} /Contents 4 0 R /Annots [{}] >>",
            list.join(" ")
        ),
        "<< /Length 0 >>\nstream\n\nendstream".to_string(),
    ];
    objects.extend(annots.iter().map(|a| a.to_string()));
    classic(&objects)
}

fn blank() -> Vec<u8> {
    page_with("", &[])
}

fn session_with(bytes: Vec<u8>) -> (AppEngine, AppSession) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(&mut session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
    (engine, session)
}

fn dispatch(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    engine.dispatch(session, request, Vec::new()).0
}

fn op(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    let reply = dispatch(engine, session, request);
    assert_eq!(reply["ok"], true, "{reply}");
    reply
}

fn document(session: &AppSession) -> &Document<'static> {
    session.doc.as_ref().unwrap().document()
}

/// Every object as written, for exact before/after comparisons.
fn serialized(session: &AppSession) -> Vec<u8> {
    let options = WriteOptions {
        garbage: false,
        ..WriteOptions::APP
    };
    write_document(document(session), DocumentWriteLimits::default(), options).unwrap()
}

fn saved(session: &AppSession) -> Vec<u8> {
    write_document(
        document(session),
        DocumentWriteLimits::default(),
        WriteOptions::APP,
    )
    .unwrap()
}

fn items(engine: &mut AppEngine, session: &mut AppSession) -> Vec<Value> {
    let reply = op(engine, session, json!({"op": "annots", "page": 0}));
    reply["items"].as_array().unwrap().clone()
}

fn last_id(engine: &mut AppEngine, session: &mut AppSession) -> u32 {
    let items = items(engine, session);
    items.last().unwrap()["id"].as_u64().unwrap() as u32
}

fn annot(session: &AppSession, number: u32) -> Dictionary {
    document(session)
        .resolve_id(id(number))
        .as_dict()
        .unwrap()
        .clone()
}

/// An annotation's normal appearance: its stream dictionary and content.
fn appearance(session: &AppSession, number: u32) -> (Dictionary, String) {
    let doc = document(session);
    let dict = annot(session, number);
    let ap = doc.dict_get(&dict, b"AP").as_dict().unwrap();
    let n = ap.get(b"N".as_slice()).unwrap().as_reference().unwrap();
    let form = doc.resolve_id(n).as_dict().unwrap().clone();
    let content = doc.decoded_stream(n, StreamLimits::default()).unwrap();
    (form, String::from_utf8(content).unwrap())
}

fn nums(doc: &Document<'_>, value: Option<&Object>) -> Vec<f64> {
    annots::numbers(doc, value.unwrap()).unwrap()
}

fn assert_near(actual: &[f64], expected: &[f64], label: &str) {
    assert_eq!(actual.len(), expected.len(), "{label}: {actual:?}");
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() < 1e-3, "{label}: {actual:?} vs {expected:?}");
    }
}

fn rect_of(item: &Value) -> Vec<f64> {
    item["rect"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect()
}

// ------------------------------------------------------------------ add

#[test]
fn every_kind_is_written_as_mupdf_writes_it() {
    let (mut engine, mut session) = session_with(blank());
    let quad = json!([[100, 100, 200, 100, 100, 120, 200, 120]]);
    let cases = [
        (
            json!({"type": "Text", "point": [100, 100], "contents": "note"}),
            "add note",
            "1 1 0 rg\n1 w\n0.5 0.5 15 15 re\nb\n1 0 0 -1 4 12 cm\n0 g\n0 0 8 1 re\n0 2 8 1 re\n0 4 8 1 re\n0 6 8 1 re\nf\n",
            vec![100.0, 672.0, 120.0, 692.0],
        ),
        (
            json!({"type": "Highlight", "quads": quad, "contents": "hl"}),
            "add highlight",
            "/H gs\n1 1 0 rg\n100 672 m\n95.2858 676.7142 95.2858 687.2858 100 692 c\n200 692 l\n204.7142 687.2858 204.7142 676.7142 200 672 c\nf\n",
            vec![95.2858, 670.75, 204.7142, 693.25],
        ),
        (
            json!({"type": "Underline", "quads": quad, "contents": ""}),
            "add underline",
            "1 0 0 RG\n1.25 w\n100 674.8571 m\n200 674.8571 l\nS\n",
            vec![98.75, 670.75, 201.25, 693.25],
        ),
        (
            json!({"type": "StrikeOut", "quads": quad, "contents": ""}),
            "add strikeout",
            "1 0 0 RG\n1.25 w\n100 680.5714 m\n200 680.5714 l\nS\n",
            vec![98.75, 670.75, 201.25, 693.25],
        ),
        (
            json!({"type": "FreeText", "point": [300, 100], "contents": "free text"}),
            "add text box",
            "0 0 0 RG\n0 w\n300 652 160 40 re\nW\nn\nq\n1 0 0 1 300 652 cm\nBT\n0 0 0 rg\n0 44.8 Td\n0 -14.4 Td\n/Helv 12 Tf\n(free text) Tj\nET\nQ\n",
            vec![300.0, 652.0, 460.0, 692.0],
        ),
        (
            json!({"type": "Square", "rect": [100, 200, 200, 260], "contents": "sq"}),
            "add square",
            "1 w\n1 0 0 RG\n100 532 100 60 re\nS\n",
            vec![99.0, 531.0, 201.0, 593.0],
        ),
        (
            json!({"type": "Circle", "rect": [100, 200, 200, 260], "contents": ""}),
            "add circle",
            "1 w\n1 0 0 RG\n150 592 m\n177.59575 592 200 578.55746 200 562 c\n200 545.44259 177.59575 532 150 532 c\n122.40425 532 100 545.44259 100 562 c\n100 578.55746 122.40425 592 150 592 c\nS\n",
            vec![99.0, 531.0, 201.0, 593.0],
        ),
        (
            json!({"type": "Line", "points": [[100, 400], [300, 420]], "contents": ""}),
            "add line",
            "1 w\n1 0 0 RG\n100 392 m\n300 372 l\nS\n",
            vec![99.0, 371.0, 301.0, 393.0],
        ),
        (
            json!({"type": "Ink", "strokes": [[[100, 450], [150, 500], [200, 450]]], "contents": "ink"}),
            "add ink",
            "1.5 w\n0 0 1 RG\n1 J\n1 j\n100 342 m\n150 292 l\n200 342 l\nS\n",
            vec![92.5, 284.5, 207.5, 349.5],
        ),
        (
            json!({"type": "Stamp", "rect": [100, 550, 260, 610], "icon": "Draft", "contents": "st"}),
            "add stamp",
            "1 0 0 rg\n1 0 0 RG\n.99994519 .0104717849 -.0104717849 .99994519 0 0 cm\n2 w\n2 2 186 44 re\nS\nBT\n/Times 30 Tf\n43.34 13 Td\n(DRAFT) Tj\nET\n",
            vec![100.0, 190.94737, 260.0, 233.05263],
        ),
    ];
    for (index, (request, step, content, rect)) in cases.into_iter().enumerate() {
        let mut request = request;
        request["op"] = json!("annot-add");
        request["page"] = json!(0);
        let reply = op(&mut engine, &mut session, request.clone());
        assert_eq!(reply["revision"], 3 * index + 1, "undo and redo count too");
        assert_eq!(reply["pageCount"], 1);
        let number = last_id(&mut engine, &mut session);
        let dict = annot(&session, number);
        let doc = document(&session);
        let kind = request["type"].as_str().unwrap();
        assert_eq!(
            doc.dict_get(&dict, b"Subtype").as_name(),
            Some(kind.as_bytes())
        );
        assert_eq!(dict.get(b"P".as_slice()), Some(&Object::Reference(id(3))));
        let flags = if kind == "Text" { 28 } else { 4 };
        assert_eq!(dict.get(b"F".as_slice()), Some(&Object::Integer(flags)));
        assert_near(&nums(doc, dict.get(b"Rect".as_slice())), &rect, kind);
        for junk in [b"CL".as_slice(), b"T", b"M", b"NM"] {
            assert!(!dict.contains_key(junk), "{kind} has no {junk:?}");
        }
        let (_, actual) = appearance(&session, number);
        assert_eq!(actual, content, "{kind}");
        let undone = op(&mut engine, &mut session, json!({"op": "undo"}));
        assert_eq!(undone["step"], step);
        op(&mut engine, &mut session, json!({"op": "redo"}));
    }
    let listed = items(&mut engine, &mut session);
    let kinds: Vec<&str> = listed.iter().map(|i| i["type"].as_str().unwrap()).collect();
    assert_eq!(
        kinds,
        [
            "Text",
            "Highlight",
            "Underline",
            "StrikeOut",
            "FreeText",
            "Square",
            "Circle",
            "Line",
            "Ink",
            "Stamp"
        ]
    );
    assert!(listed.iter().all(|item| item["editable"] == true));
    // The rects the renderer sees: the request's own geometry.
    assert_near(&rect_of(&listed[0]), &[100.0, 100.0, 120.0, 120.0], "note");
    assert_near(&rect_of(&listed[1]), &[100.0, 100.0, 200.0, 120.0], "quads");
    assert_near(
        &rect_of(&listed[5]),
        &[100.0, 200.0, 200.0, 260.0],
        "square",
    );
    assert_near(&rect_of(&listed[7]), &[100.0, 400.0, 300.0, 420.0], "line");
    assert_near(&rect_of(&listed[8]), &[100.0, 450.0, 200.0, 500.0], "ink");
    assert_near(
        &rect_of(&listed[9]),
        &[100.0, 558.947, 260.0, 601.053],
        "stamp",
    );
}

#[test]
fn a_note_brings_a_popup_and_the_dictionaries_keep_mupdf_order() {
    let (mut engine, mut session) = session_with(blank());
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "Text", "point": [100, 100], "contents": "note"}),
    );
    let doc = document(&session);
    let page = doc.resolve_id(id(3)).as_dict().unwrap();
    // The note, its popup and its appearance take the next numbers in that order.
    assert_eq!(
        doc.dict_get(page, b"Annots"),
        &Object::Array(vec![Object::Reference(id(5)), Object::Reference(id(6))])
    );
    let note = annot(&session, 5);
    let keys: Vec<&[u8]> = note.keys().map(Vec::as_slice).collect();
    assert_eq!(
        keys,
        [
            b"Type".as_slice(),
            b"Subtype",
            b"Rect",
            b"C",
            b"Popup",
            b"P",
            b"F",
            b"Contents",
            b"AP"
        ]
    );
    let popup = annot(&session, 6);
    assert_eq!(
        popup.get(b"Parent".as_slice()),
        Some(&Object::Reference(id(5)))
    );
    assert_eq!(popup.get(b"Open".as_slice()), Some(&Object::Boolean(false)));
    // Beside the note, not in the page corner.
    assert_near(
        &nums(doc, popup.get(b"Rect".as_slice())),
        &[120.0, 592.0, 320.0, 692.0],
        "popup",
    );
    let ap = doc.dict_get(&note, b"AP").as_dict().unwrap();
    assert_eq!(ap.get(b"N".as_slice()), Some(&Object::Reference(id(7))));
    let (form, _) = appearance(&session, 5);
    assert_eq!(
        form.get(b"BBox".as_slice()),
        Some(&Object::Array([0, 0, 16, 16].map(Object::Integer).to_vec()))
    );
    // Listing skips the popup.
    assert_eq!(items(&mut engine, &mut session).len(), 1);
}

#[test]
fn free_text_fits_its_lines_and_refuses_what_it_cannot_write() {
    let (mut engine, mut session) = session_with(blank());
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "FreeText", "point": [300, 200], "contents": "fit me\nline2", "fit": true}),
    );
    assert_eq!(reply["revision"], 1);
    let item = items(&mut engine, &mut session).pop().unwrap();
    // MuPDF: /Rect [300 570.4 329.84 599.2] in user space.
    assert_near(&rect_of(&item), &[300.0, 192.8, 329.84, 221.6], "fit");
    let number = item["id"].as_u64().unwrap() as u32;
    let (_, content) = appearance(&session, number);
    assert_eq!(content.matches(" Tj").count(), 2);
    assert!(content.contains("0 33.6 Td\n0 -14.4 Td\n/Helv 12 Tf\n(fit me) Tj"));

    let refused = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "FreeText", "point": [72, 300], "contents": "Omega Ω, Ω and 中", "fit": true}),
    );
    assert_eq!(refused["reason"], "invalid");
    assert_eq!(refused["detail"], "unsupported-encoding");
    assert_eq!(refused["missing"], json!(["Ω", "中"]));
    assert_eq!(refused["revision"], 1);
    let wide = "abcdefghij".repeat(20);
    let refused = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "FreeText", "point": [72, 300], "contents": wide, "fit": true}),
    );
    assert_eq!(refused["detail"], "overflow");
    assert_eq!(
        refused["error"],
        "The text does not fit on the page; use shorter lines"
    );

    // Without fit: the fixed box, moved inside the page; long text wraps
    // after a space and keeps it, a CRLF is one break.
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "FreeText", "point": [600, 780],
            "contents": "a long line of text that will need to wrap across the box width\r\nend"}),
    );
    let item = items(&mut engine, &mut session).pop().unwrap();
    assert_eq!(rect_of(&item), [452.0, 752.0, 612.0, 792.0]);
    let (_, content) = appearance(&session, item["id"].as_u64().unwrap() as u32);
    let lines: Vec<&str> = content.lines().filter(|l| l.ends_with(" Tj")).collect();
    assert_eq!(
        lines,
        [
            "(a long line of text that will ) Tj",
            "(need to wrap across the box ) Tj",
            "(width) Tj",
            "(end) Tj"
        ]
    );
}

#[test]
fn refusals_come_in_engine_ts_order_and_journal_nothing() {
    let (mut engine, mut session) = session_with(blank());
    let start = serialized(&session);
    let cases = [
        (
            json!({"page": 5, "type": "Text", "point": [10, 10], "contents": "x"}),
            "annot-add: page must be an integer in 0..0",
        ),
        (
            json!({"page": 0, "type": "Popup", "point": [10, 10], "contents": "x"}),
            "annot-add: type must be one of Text, Highlight, Underline, StrikeOut, FreeText, Square, Circle, Line, Ink, Stamp",
        ),
        (
            json!({"page": 0, "type": "Text", "point": [10, 10], "contents": 42}),
            "annot-add: contents must be a string of at most 4096 characters",
        ),
        (
            json!({"page": 0, "type": "Text", "point": [10], "contents": ""}),
            "annot-add: point must be 2 finite numbers",
        ),
        (
            json!({"page": 0, "type": "Text", "point": [700, 1.5], "contents": ""}),
            "annot-add: point [700, 1.5] is off the page",
        ),
        (
            json!({"page": 0, "type": "Highlight", "quads": [], "contents": ""}),
            "annot-add: quads must be 1..64 quads",
        ),
        (
            json!({"page": 0, "type": "Highlight", "quads": [[1, 2, 3]], "contents": ""}),
            "annot-add: quad 0 must be 8 finite numbers",
        ),
        (
            json!({"page": 0, "type": "Square", "rect": [100, 100, 100, 200], "contents": ""}),
            "annot-add: rect must have a positive area",
        ),
        (
            json!({"page": 0, "type": "Square", "rect": [100, 100, 700, 200], "contents": ""}),
            "annot-add: rect [100, 100, 700, 200] is off the page",
        ),
        (
            json!({"page": 0, "type": "Line", "points": [[1, 1], [1, 1]], "contents": ""}),
            "annot-add: a Line needs two distinct points",
        ),
        (
            json!({"page": 0, "type": "Ink", "strokes": [[[1, 1]]], "contents": ""}),
            "annot-add: stroke 0 must be 2..2048 points",
        ),
        (
            json!({"page": 0, "type": "Ink", "strokes": [[[1, 1], [1, 900]]], "contents": ""}),
            "annot-add: stroke 0 point 1: point [1, 900] is off the page",
        ),
        (
            json!({"page": 0, "type": "Stamp", "rect": [1, 1, 9, 9], "icon": "Draft", "signature": true, "contents": ""}),
            "annot-add: a Stamp signature takes an image and no icon or sign name",
        ),
        (
            json!({"page": 0, "type": "Stamp", "rect": [1, 1, 9, 9], "icon": "Nope", "contents": ""}),
            "annot-add: icon must be one of Approved, Draft, Confidential, Final, NotApproved, Void",
        ),
        (
            json!({"page": 0, "type": "Stamp", "rect": [1, 1, 9, 9], "contents": ""}),
            "annot-add: a Stamp takes one of Approved, Draft, Confidential, Final, NotApproved, Void or an image",
        ),
        (
            json!({"page": 0, "type": "Stamp", "rect": [1, 1, 9, 9], "image": {"name": "a.png", "bytes": {"$buffer": 0}}, "sign": {"name": "  "}, "contents": ""}),
            "annot-add: a signature name must be 1..200 characters",
        ),
        (
            json!({"page": 0, "type": "Stamp", "rect": [1, 1, 9, 9], "image": {"name": "", "bytes": {"$buffer": 0}}, "contents": ""}),
            "annot-add: a stamp image needs a name of at most 255 characters",
        ),
        (
            json!({"page": 0, "type": "Stamp", "rect": [1, 1, 9, 9], "image": {"name": "a.png", "bytes": "x"}, "contents": ""}),
            "annot-add: stamp image a.png has no bytes",
        ),
        (
            json!({"page": 0, "type": "Square", "rect": [1, 1, 9, 9], "signature": true, "contents": ""}),
            "annot-add: signature applies to Ink only and must be true or false",
        ),
        (
            json!({"page": 0, "type": "Ink", "strokes": [[[1, 1], [2, 2]]], "signature": "yes", "contents": ""}),
            "annot-add: signature applies to Ink only and must be true or false",
        ),
    ];
    for (mut request, message) in cases {
        request["op"] = json!("annot-add");
        let reply = engine
            .dispatch(&mut session, request, vec![b"not an image".to_vec()])
            .0;
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error"], message);
        assert_eq!(reply["revision"], 0);
        assert_eq!(reply["dirty"], false);
    }
    let unreadable = engine
        .dispatch(
            &mut session,
            json!({"op": "annot-add", "page": 0, "type": "Stamp", "rect": [1, 1, 9, 9],
                "image": {"name": "a.png", "bytes": {"$buffer": 0}}, "contents": ""}),
            vec![b"not an image".to_vec()],
        )
        .0;
    assert!(
        unreadable["error"]
            .as_str()
            .unwrap()
            .starts_with("annot-add: a.png is not a PNG or JPEG the app can read")
    );
    for (request, message) in [
        (
            json!({"op": "annot-set", "page": 0, "annotId": 1.5, "contents": "x"}),
            "annot-set: id must be an integer",
        ),
        (
            json!({"op": "annot-set", "page": 0, "annotId": 999, "contents": "x"}),
            "annot-set: no annotation 999 on this page",
        ),
        (
            json!({"op": "annot-delete", "page": 0, "annotId": 3}),
            "annot-delete: no annotation 3 on this page",
        ),
        (
            json!({"op": "annot-move", "page": 0, "annotId": 3, "point": [1]}),
            "annot-move: point must be 2 finite numbers",
        ),
    ] {
        let reply = dispatch(&mut engine, &mut session, request);
        assert_eq!(reply["error"], message);
    }
    assert_eq!(serialized(&session), start);
    let undo = dispatch(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(undo["ok"], false, "nothing was journaled");
}

#[test]
fn signed_and_locked_documents_list_but_refuse_edits() {
    let signed = classic(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R] >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [5 0 R 6 0 R] >>".into(),
        "<< >>".into(),
        "<< /Type /Annot /Subtype /Square /Rect [10 10 50 50] >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Sig /T (s) /Rect [0 0 0 0] /V << /Type /Sig >> >>"
            .into(),
    ]);
    let (mut engine, mut session) = session_with(signed);
    let listed = items(&mut engine, &mut session);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["editable"], false);
    let refused = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "annot-delete", "page": 0, "annotId": 5}),
    );
    assert_eq!(refused["reason"], "read-only");
    assert_eq!(refused["error"], "annot-delete: this document is read-only");
    let none = dispatch(
        &mut engine,
        &mut AppSession::default(),
        json!({"op": "annots", "page": 0}),
    );
    assert_eq!(none["error"], "annots: no document open");
    assert!(none.get("reason").is_none());
}

// ------------------------------------------------------------------ set, move

#[test]
fn set_rewrites_contents_and_refits_a_text_box_from_its_top_left() {
    let (mut engine, mut session) = session_with(blank());
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "FreeText", "point": [300, 100], "contents": "a"}),
    );
    let number = last_id(&mut engine, &mut session);
    let before = serialized(&session);
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "annot-set", "page": 0, "annotId": number, "contents": "changed\nsecond line", "fit": true}),
    );
    assert_eq!(reply["revision"], 2);
    let item = items(&mut engine, &mut session).pop().unwrap();
    assert_eq!(item["contents"], "changed\nsecond line");
    // MuPDF: /Rect [300 663.2 361.196 692], the top kept.
    assert_near(&rect_of(&item), &[300.0, 100.0, 361.196, 128.8], "refit");
    let (_, content) = appearance(&session, number);
    assert!(content.contains("(changed) Tj") && content.contains("(second line) Tj"));
    let after = serialized(&session);
    assert_eq!(
        op(&mut engine, &mut session, json!({"op": "undo"}))["step"],
        "edit annotation"
    );
    assert_eq!(serialized(&session), before);
    op(&mut engine, &mut session, json!({"op": "redo"}));
    assert_eq!(serialized(&session), after);

    // A note keeps its appearance; a Square is not editable as text but its
    // contents are; a Popup is not listed, so it is not found.
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "Text", "point": [10, 10], "contents": "n"}),
    );
    let note = last_id(&mut engine, &mut session);
    let ap = annot(&session, note).get(b"AP".as_slice()).cloned();
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-set", "page": 0, "annotId": note, "contents": "é"}),
    );
    let dict = annot(&session, note);
    assert_eq!(dict.get(b"AP".as_slice()).cloned(), ap);
    assert_eq!(
        decode_text_string(
            dict.get(b"Contents".as_slice())
                .unwrap()
                .as_string()
                .unwrap()
        ),
        "é"
    );
    let popup = note + 1;
    let refused = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "annot-set", "page": 0, "annotId": popup, "contents": "x"}),
    );
    assert_eq!(
        refused["error"],
        format!("annot-set: no annotation {popup} on this page")
    );
}

#[test]
fn move_keeps_size_clamps_and_turns_a_landing_in_place_into_a_no_op() {
    let (mut engine, mut session) = session_with(blank());
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "FreeText", "point": [72, 300], "contents": "Jane Doe", "fit": true}),
    );
    let number = last_id(&mut engine, &mut session);
    let first = rect_of(&items(&mut engine, &mut session)[0]);
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-move", "page": 0, "annotId": number, "point": [100, 400]}),
    );
    let moved = rect_of(&items(&mut engine, &mut session)[0]);
    assert_near(
        &moved,
        &[
            100.0,
            400.0,
            100.0 + first[2] - first[0],
            400.0 + first[3] - first[1],
        ],
        "moved",
    );
    let same = op(
        &mut engine,
        &mut session,
        json!({"op": "annot-move", "page": 0, "annotId": number, "point": [100.004, 400]}),
    );
    assert_eq!(same["noop"], true);
    assert_eq!(same["revision"], 2);
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-move", "page": 0, "annotId": number, "point": [600, 790]}),
    );
    let edge = rect_of(&items(&mut engine, &mut session)[0]);
    assert!((edge[2] - 612.0).abs() < 1e-6 && (edge[3] - 792.0).abs() < 1e-6);
    assert_eq!(
        op(&mut engine, &mut session, json!({"op": "undo"}))["step"],
        "move text box"
    );

    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "Ink", "strokes": [[[100, 100], [150, 150], [200, 110]], [[220, 120], [300, 100]]], "contents": "", "signature": true}),
    );
    let sig = last_id(&mut engine, &mut session);
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "annot-move", "page": 0, "annotId": sig, "point": [150, 200], "width": 100}),
    );
    assert_eq!(reply["ok"], true);
    assert_eq!(
        op(&mut engine, &mut session, json!({"op": "undo"}))["step"],
        "resize signature"
    );
    op(&mut engine, &mut session, json!({"op": "redo"}));
    let listed = items(&mut engine, &mut session);
    let drawn = listed.iter().find(|i| i["id"] == sig).unwrap();
    assert_near(&rect_of(drawn), &[150.0, 200.0, 250.0, 225.0], "resized");
    let dict = annot(&session, sig);
    let doc = document(&session);
    let bs = doc.dict_get(&dict, b"BS").as_dict().unwrap();
    assert_eq!(bs.get(b"W".as_slice()), Some(&Object::Real(0.75)));
    for (request, message) in [
        (
            json!({"op": "annot-move", "page": 0, "annotId": number, "point": [1, 1], "width": 5}),
            "annot-move: width applies to drawn signatures only",
        ),
        (
            json!({"op": "annot-move", "page": 0, "annotId": sig, "point": [1, 1], "width": "wide"}),
            "annot-move: width must be a finite number",
        ),
    ] {
        assert_eq!(
            dispatch(&mut engine, &mut session, request)["error"],
            message
        );
    }
}

// ------------------------------------------------------------------ delete

#[test]
fn delete_takes_the_popup_and_cuts_references_once_no_page_lists_it() {
    // A note (5) with its popup (6), a reply to it (7, /IRT), a second page
    // listing the note too, and a structure element naming it (OBJR).
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 9 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 8 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [5 0 R 6 0 R 7 0 R] >>".into(),
        "<< >>".into(),
        "<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /Popup 6 0 R /Contents (secret) /StructParent 0 >>".into(),
        "<< /Type /Annot /Subtype /Popup /Rect [0 0 1 1] /Parent 5 0 R >>".into(),
        "<< /Type /Annot /Subtype /Text /Rect [40 10 60 30] /IRT 5 0 R /Contents (reply) >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [5 0 R] >>".into(),
        "<< /Type /StructTreeRoot /K [10 0 R] >>".into(),
        "<< /Type /StructElem /S /Annot /K [<< /Type /OBJR /Obj 5 0 R >>] >>".into(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-delete", "page": 0, "annotId": 5}),
    );
    let listed: Vec<u64> = (items(&mut engine, &mut session).iter())
        .map(|i| i["id"].as_u64().unwrap())
        .collect();
    assert_eq!(listed, [7]);
    {
        let doc = document(&session);
        let page = |n| doc.resolve_id(id(n)).as_dict().unwrap().clone();
        assert_eq!(
            doc.dict_get(&page(3), b"Annots"),
            &Object::Array(vec![Object::Reference(id(7))])
        );
        // The second page still lists the note, so it stays whole there,
        // popup included: the popup left this page's /Annots, as in MuPDF,
        // and the note still names it.
        assert_eq!(
            doc.dict_get(&page(8), b"Annots"),
            &Object::Array(vec![Object::Reference(id(5))])
        );
    }
    assert!(annot(&session, 7).contains_key(b"IRT".as_slice()));
    assert_eq!(
        annot(&session, 5).get(b"Popup".as_slice()),
        Some(&Object::Reference(id(6)))
    );
    assert_eq!(
        annot(&session, 6).get(b"Parent".as_slice()),
        Some(&Object::Reference(id(5)))
    );
    let middle = serialized(&session);
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-delete", "page": 1, "annotId": 5}),
    );
    let after = serialized(&session);
    let doc = document(&session);
    let page = |n| doc.resolve_id(id(n)).as_dict().unwrap().clone();
    assert_eq!(
        doc.dict_get(&page(8), b"Annots"),
        &Object::Array(Vec::new())
    );
    assert!(!annot(&session, 7).contains_key(b"IRT".as_slice()));
    let save = saved(&session);
    assert!(
        !save.windows(6).any(|w| w == b"secret"),
        "nothing writes it"
    );
    assert!(!save.windows(4).any(|w| w == b"OBJR"), "the OBJR goes too");
    assert_eq!(
        op(&mut engine, &mut session, json!({"op": "undo"}))["step"],
        "delete annotation"
    );
    assert_eq!(serialized(&session), middle);
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
    op(&mut engine, &mut session, json!({"op": "redo"}));
    op(&mut engine, &mut session, json!({"op": "redo"}));
    assert_eq!(serialized(&session), after);

    // Not every listed kind can be deleted.
    let (mut engine, mut session) = session_with(page_with(
        "",
        &["<< /Type /Annot /Subtype /Caret /Rect [10 10 30 30] >>"],
    ));
    let refused = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "annot-delete", "page": 0, "annotId": 5}),
    );
    assert_eq!(
        refused["error"],
        "annot-delete: only editable annotations and Redact marks can be deleted"
    );
}

#[test]
fn delete_removes_an_entry_that_reaches_the_annotation_through_a_chain() {
    // Page /Annots [4 0 R], where object 4 is `5 0 R` and 5 the annotation.
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>".into(),
        "5 0 R".into(),
        "<< /Type /Annot /Subtype /Square /Rect [10 10 50 50] /Contents (chained) >>".into(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    let listed: Vec<u64> = (items(&mut engine, &mut session).iter())
        .map(|i| i["id"].as_u64().unwrap())
        .collect();
    assert_eq!(listed, [5]);
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-delete", "page": 0, "annotId": 5}),
    );
    let after = serialized(&session);
    assert!(items(&mut engine, &mut session).is_empty());
    let doc = document(&session);
    let page = doc.resolve_id(id(3)).as_dict().unwrap();
    assert_eq!(doc.dict_get(page, b"Annots"), &Object::Array(Vec::new()));
    let save = saved(&session);
    assert!(
        !save.windows(7).any(|w| w == b"chained"),
        "nothing keeps object 4 or 5"
    );
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
    op(&mut engine, &mut session, json!({"op": "redo"}));
    assert_eq!(serialized(&session), after);
}

// ------------------------------------------------------------------ listing

#[test]
fn listing_follows_mupdf_and_reports_geometry_in_page_space() {
    let bytes = page_with(
        "/Rotate 90",
        &[
            "<< /Type /Annot /Subtype /Square /Rect [99 99 201 201] /RD [1 1 1 1] /Contents <FEFF00E9> /Subj (s) >>",
            "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] >>",
            "<< /Type /Annot /Subtype /Widget /Rect [0 0 10 10] >>",
            "<< /Type /Annot /Subtype /Highlight /Rect [0 0 1 1] /QuadPoints [100 120 200 120 100 100 200 100] >>",
            "<< /Type /Annot /Subtype /Polygon /Rect [0 0 1 1] /Vertices [100 100 150 200 200 100] >>",
            "<< /Type /Annot /Subtype /Redact /Rect [10 10 50 50] /PDF4AllSearch (x) >>",
            "<< /Type /Annot /Rect [0 0 10 10] >>",
            "<< /Type /Annot /Subtype /Foo /Rect [1 2 3 4] >>",
        ],
    );
    let mut bytes = String::from_utf8(bytes).unwrap();
    // A repeated reference and a direct dictionary in /Annots.
    bytes = bytes.replace(
        "/Annots [5 0 R",
        "/Annots [<< /Subtype /Circle /Rect [1 1 9 9] >> 5 0 R 5 0 R",
    );
    let bytes = fix_xref(bytes.into_bytes());
    let (mut engine, mut session) = session_with(bytes);
    let listed = items(&mut engine, &mut session);
    let rows: Vec<(u64, &str, bool)> = (listed.iter())
        .map(|i| {
            (
                i["id"].as_u64().unwrap(),
                i["type"].as_str().unwrap(),
                i["editable"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (0, "Circle", false),
            (5, "Square", true),
            (8, "Highlight", true),
            (9, "Polygon", false),
            (10, "Redact", false),
            (12, "Foo", false)
        ]
    );
    // Page space on a page turned 90 degrees: x' = y, y' = x.
    assert_near(
        &rect_of(&listed[1]),
        &[100.0, 100.0, 200.0, 200.0],
        "square",
    );
    assert_near(&rect_of(&listed[2]), &[100.0, 100.0, 120.0, 200.0], "quads");
    assert_near(
        &rect_of(&listed[3]),
        &[100.0, 100.0, 200.0, 200.0],
        "vertices",
    );
    assert_eq!(listed[1]["contents"], "é");
    assert_eq!(listed[1]["subject"], "s");
    assert_eq!(listed[4]["search"], true);
    assert!(listed[1].get("search").is_none());
}

/// Rebuilds the xref of a classic PDF whose bodies were edited in place.
fn fix_xref(bytes: Vec<u8>) -> Vec<u8> {
    let text = String::from_utf8(bytes).unwrap();
    let body = &text[..text.find("xref\n").unwrap()];
    let mut offsets = Vec::new();
    let mut at = 0;
    while let Some(found) = body[at..].find(" 0 obj\n") {
        let line = body[..at + found].rfind('\n').map_or(0, |n| n + 1);
        offsets.push(line);
        at += found + 7;
    }
    let mut out = body.to_string();
    let xref = out.len();
    out.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        offsets.len() + 1
    ));
    for offset in &offsets {
        out.push_str(&format!("{offset:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        offsets.len() + 1
    ));
    out.into_bytes()
}

#[test]
fn rotated_pages_keep_what_the_user_sees() {
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 600] /Rotate 90 >>".into(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "Text", "point": [12, 12], "contents": "top left"}),
    );
    assert_eq!(reply["pages"], json!([{"width": 600.0, "height": 300.0}]));
    assert_eq!(
        rect_of(&items(&mut engine, &mut session)[0]),
        [12.0, 12.0, 32.0, 32.0]
    );
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "FreeText", "point": [100, 100], "contents": "turned"}),
    );
    let number = last_id(&mut engine, &mut session);
    let dict = annot(&session, number);
    assert_eq!(dict.get(b"Rotate".as_slice()), Some(&Object::Integer(90)));
    let (_, content) = appearance(&session, number);
    assert!(content.contains("0 1 -1 0 140 100 cm"), "{content}");
    // The note is drawn upright at the visible top left.
    let image = session
        .view()
        .unwrap()
        .render(0, 1.0, RenderOptions::default())
        .unwrap();
    let ink = ink_box(&image.pixels, image.width, image.height).unwrap();
    assert!(
        ink[0] >= 11 && ink[1] >= 11 && ink[0] < 40 && ink[1] < 40,
        "{ink:?}"
    );
}

/// The bounds of non-white pixels.
fn ink_box(pixels: &[u8], width: u32, height: u32) -> Option<[u32; 4]> {
    let mut found: Option<[u32; 4]> = None;
    for y in 0..height {
        for x in 0..width {
            let i = ((y * width + x) * 4) as usize;
            if pixels[i..i + 3].iter().any(|&c| c < 250) {
                let b = found.get_or_insert([x, y, x, y]);
                *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            }
        }
    }
    found
}

// ------------------------------------------------------------------ images

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let data: Vec<u8> = (0..width * height).map(|i| (i % 251) as u8).collect();
        writer.write_image_data(&data).unwrap();
    }
    out
}

#[test]
fn image_stamps_and_signatures_draw_the_image_over_their_rect() {
    let (mut engine, mut session) = session_with(blank());
    let add = |engine: &mut AppEngine, session: &mut AppSession, extra: Value| {
        let mut request = json!({"op": "annot-add", "page": 0, "type": "Stamp", "rect": [100, 100, 300, 160],
            "image": {"name": "g.png", "bytes": {"$buffer": 0}}, "contents": "c", "today": "2026-10-15"});
        for (key, value) in extra.as_object().unwrap() {
            request[key] = value.clone();
        }
        let reply = engine.dispatch(session, request, vec![png(30, 9)]).0;
        assert_eq!(reply["ok"], true, "{reply}");
        reply
    };
    add(&mut engine, &mut session, json!({}));
    add(&mut engine, &mut session, json!({"sign": {"name": "Ada"}}));
    add(&mut engine, &mut session, json!({"signature": true}));
    let listed = items(&mut engine, &mut session);
    let rows: Vec<(&str, &str)> = (listed.iter())
        .map(|i| {
            (
                i["contents"].as_str().unwrap(),
                i["subject"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("c", ""),
            ("Signed by Ada on 2026-10-15", "Signature appearance"),
            ("", "Signature appearance")
        ]
    );
    for item in &listed {
        assert_eq!(rect_of(item), [100.0, 100.0, 300.0, 160.0]);
        let number = item["id"].as_u64().unwrap() as u32;
        let dict = annot(&session, number);
        assert!(!dict.contains_key(b"Name".as_slice()), "no leftover icon");
        let (form, content) = appearance(&session, number);
        assert_eq!(content, "/I Do\n");
        let doc = document(&session);
        let resources = doc.dict_get(&form, b"Resources").as_dict().unwrap();
        let image = doc.dict_get(resources, b"XObject").as_dict().unwrap();
        let image = doc.dict_get(image, b"I").as_dict().unwrap();
        assert_eq!(image.get(b"Width".as_slice()), Some(&Object::Integer(30)));
    }
    assert_eq!(
        op(&mut engine, &mut session, json!({"op": "undo"}))["step"],
        "add signature"
    );
}

// ------------------------------------------------------------------ view

#[test]
fn the_view_draws_annotations_without_an_appearance_and_saves_none() {
    let bytes = page_with(
        "",
        &[
            "<< /Type /Annot /Subtype /Square /Rect [100 600 200 700] /C [1 0 0] /IC [0 0 1] /BS << /W 3 >> >>",
        ],
    );
    let (_, mut session) = session_with(bytes);
    let image = session
        .view()
        .unwrap()
        .render(0, 1.0, RenderOptions::default())
        .unwrap();
    let at = |x: u32, y: u32| {
        let i = ((y * image.width + x) * 4) as usize;
        [image.pixels[i], image.pixels[i + 1], image.pixels[i + 2]]
    };
    assert_eq!(at(150, 142), [0, 0, 255], "the interior is filled");
    assert_eq!(at(100, 142), [255, 0, 0], "the border is stroked");
    let save = saved(&session);
    assert!(
        !save.windows(3).any(|w| w == b"/AP"),
        "the save has no appearance"
    );
}

#[test]
fn the_view_draws_a_note_at_its_icon_size_from_the_top_left() {
    // MuPDF's own note: a 16 x 16 icon in a 20 x 20 rect.
    let bytes = page_with(
        "",
        &[
            "<< /Type /Annot /Subtype /Text /Rect [100 672 120 692] /F 28 /AP << /N 6 0 R >> >>",
            "<< /Type /XObject /Subtype /Form /BBox [0 0 16 16] /Length 18 >>\nstream\n0 g 0 0 16 16 re f\nendstream",
        ],
    );
    let (_, mut session) = session_with(bytes);
    let image = session
        .view()
        .unwrap()
        .render(0, 1.0, RenderOptions::default())
        .unwrap();
    assert_eq!(
        ink_box(&image.pixels, image.width, image.height),
        Some([100, 100, 115, 115])
    );
}

// ------------------------------------------------------------------ robustness

struct Rng(u64);
impl Rng {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % bound.max(1)
    }
    fn value(&mut self) -> Value {
        match self.next(9) {
            0 => json!(null),
            1 => json!(self.next(700) as f64 - 50.0),
            2 => json!({"$number": "NaN"}),
            3 => json!("x"),
            4 => json!([self.next(700), self.next(800)]),
            5 => json!([
                [self.next(600), self.next(800)],
                [self.next(600), self.next(800)]
            ]),
            6 => json!([[self.next(600), self.next(800), 1, 2, 3, 4, 5, 6]]),
            7 => json!(1e300),
            _ => json!({"name": "a"}),
        }
    }
}

#[test]
fn random_requests_never_panic_and_undo_to_the_start() {
    let kinds = [
        "Text",
        "Highlight",
        "Underline",
        "StrikeOut",
        "FreeText",
        "Square",
        "Circle",
        "Line",
        "Ink",
        "Stamp",
        "Redact",
    ];
    for seed in 1..=8u64 {
        let mut rng = Rng(seed * 0x9E37_79B9_7F4A_7C15);
        let rotate = [0, 90, 180, 270][rng.next(4) as usize];
        let (mut engine, mut session) = session_with(page_with(&format!("/Rotate {rotate}"), &[]));
        let start = serialized(&session);
        for _ in 0..80 {
            let ids: Vec<u64> = (items(&mut engine, &mut session).iter())
                .map(|i| i["id"].as_u64().unwrap())
                .collect();
            let target = ids
                .get(rng.next(ids.len() as u64 + 1) as usize)
                .copied()
                .unwrap_or(4);
            let request = match rng.next(6) {
                0 | 1 => {
                    let kind = kinds[rng.next(kinds.len() as u64) as usize];
                    let x = rng.next(500) as f64;
                    let y = rng.next(700) as f64;
                    json!({"op": "annot-add", "page": 0, "type": kind, "contents": "c",
                        "point": [x, y], "quads": [[x, y, x + 50.0, y, x, y + 10.0, x + 50.0, y + 10.0]],
                        "rect": [x, y, x + 40.0, y + 30.0], "points": [[x, y], [x + 30.0, y + 40.0]],
                        "strokes": [[[x, y], [x + 10.0, y + 20.0], [x + 30.0, y]]], "icon": "Final",
                        "fit": rng.next(2) == 0, "signature": rng.next(3) == 0})
                }
                2 => {
                    json!({"op": "annot-set", "page": 0, "annotId": target, "contents": "new\ntext", "fit": rng.next(2) == 0})
                }
                3 => {
                    json!({"op": "annot-move", "page": 0, "annotId": target, "point": rng.value(), "width": rng.value()})
                }
                4 => json!({"op": "annot-delete", "page": 0, "annotId": target}),
                _ => json!({"op": if rng.next(2) == 0 { "undo" } else { "redo" }}),
            };
            let mut request = request;
            if rng.next(4) == 0 {
                let keys = [
                    "point", "quads", "rect", "points", "strokes", "contents", "page", "annotId",
                ];
                request[keys[rng.next(keys.len() as u64) as usize]] = rng.value();
            }
            let reply = dispatch(&mut engine, &mut session, request);
            assert!(reply["ok"].is_boolean());
            let _ = session
                .view()
                .unwrap()
                .render(0, 0.25, RenderOptions::default());
        }
        while dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"] == true {}
        assert_eq!(serialized(&session), start, "seed {seed}");
    }
}

#[test]
fn page_space_matches_the_view_on_every_rotation() {
    for rotate in [0, 90, 180, 270, -90, 45, 450] {
        let bytes = page_with(&format!("/Rotate {rotate} /CropBox [10 20 310 620]"), &[]);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let page = doc.resolve_id(id(3)).as_dict().unwrap();
        let space = PageSpace::of(&doc, page);
        let (w, h) = tandem_pdf_engine::app::info::page_size(&doc, page);
        assert_eq!((space.width, space.height), (w, h), "{rotate}");
        for p in [(10.0, 20.0), (310.0, 620.0), (100.5, 333.25)] {
            let back = space.to_user(space.to_page(p));
            assert!((back.0 - p.0).abs() < 1e-9 && (back.1 - p.1).abs() < 1e-9);
        }
        let snapped = match rotate {
            45 | 90 | 450 => 90,
            -90 => 270,
            other => other,
        };
        let expected = match snapped {
            90 => (600.0, 0.0),
            180 => (300.0, 600.0),
            270 => (0.0, 300.0),
            _ => (0.0, 0.0),
        };
        assert_eq!(space.to_page((10.0, 620.0)), expected, "{rotate}");
        assert_eq!(space.rotate, snapped, "{rotate}");
    }
}

// ------------------------------------------------------------------ hostile and foreign files

/// Page count and pages of a saved file reopened.
fn reopened_pages(bytes: &[u8]) -> usize {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    tandem_pdf_engine::app::info::pages(&doc).len()
}

#[test]
fn delete_never_cuts_the_page_tree_or_what_a_popup_entry_wrongly_names() {
    // /Popup naming the page, the page tree root, the catalog, the content
    // stream, and a page that calls itself a Popup: only the annotation goes.
    for (page, popup) in [
        ("", "3 0 R"),
        ("", "2 0 R"),
        ("", "1 0 R"),
        ("", "4 0 R"),
        ("/Subtype /Popup", "3 0 R"),
    ] {
        let bytes = page_with(
            page,
            &[&format!(
                "<< /Type /Annot /Subtype /Square /Rect [10 10 30 30] /Popup {popup} >>"
            )],
        );
        let (mut engine, mut session) = session_with(bytes);
        let before = serialized(&session);
        let reply = op(
            &mut engine,
            &mut session,
            json!({"op": "annot-delete", "page": 0, "annotId": 5}),
        );
        assert_eq!(reply["pageCount"], 1, "{popup}");
        assert!(items(&mut engine, &mut session).is_empty());
        let save = op(&mut engine, &mut session, json!({"op": "save"}));
        assert_eq!(save["ok"], true);
        assert_eq!(reopened_pages(&saved(&session)), 1, "{page} {popup}");
        session
            .view()
            .unwrap()
            .render(0, 0.25, RenderOptions::default())
            .unwrap();
        op(&mut engine, &mut session, json!({"op": "undo"}));
        assert_eq!(serialized(&session), before);
    }

    // A page listed as an annotation of itself leaves /Annots and stays a page.
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Subtype /Square /Rect [0 0 9 9] /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [3 0 R] >>".into(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-delete", "page": 0, "annotId": 3}),
    );
    assert!(items(&mut engine, &mut session).is_empty());
    assert_eq!(reopened_pages(&saved(&session)), 1);

    // /Popup naming another page's Square, and a Popup whose /Parent is
    // another note: neither is the deleted note's, so both stay.
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [5 0 R 6 0 R 8 0 R 9 0 R] >>"
            .into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [7 0 R] >>".into(),
        "<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /Popup 7 0 R >>".into(),
        "<< /Type /Annot /Subtype /Text /Rect [40 10 60 30] /Popup 9 0 R >>".into(),
        "<< /Type /Annot /Subtype /Square /Rect [10 10 90 90] >>".into(),
        "<< /Type /Annot /Subtype /Text /Rect [70 10 90 30] /Popup 9 0 R >>".into(),
        "<< /Type /Annot /Subtype /Popup /Rect [0 0 1 1] /Parent 8 0 R >>".into(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-delete", "page": 0, "annotId": 5}),
    );
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-delete", "page": 0, "annotId": 6}),
    );
    let doc = document(&session);
    let annots_of = |n| {
        doc.dict_get(doc.resolve_id(id(n)).as_dict().unwrap(), b"Annots")
            .clone()
    };
    assert_eq!(
        annots_of(3),
        Object::Array(vec![Object::Reference(id(8)), Object::Reference(id(9))])
    );
    assert_eq!(annots_of(4), Object::Array(vec![Object::Reference(id(7))]));
    assert_eq!(
        annot(&session, 8).get(b"Popup".as_slice()),
        Some(&Object::Reference(id(9)))
    );
}

#[test]
fn remove_many_takes_marks_on_several_pages_in_one_pass() {
    use tandem_pdf_engine::app::edit::{Builder, Edit};
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [5 0 R 6 0 R 8 0 R] >>"
            .into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [7 0 R] >>".into(),
        "<< /Type /Annot /Subtype /Redact /Rect [10 10 30 30] /Popup 6 0 R >>".into(),
        "<< /Type /Annot /Subtype /Popup /Rect [0 0 1 1] /Parent 5 0 R >>".into(),
        "<< /Type /Annot /Subtype /Redact /Rect [10 10 90 90] >>".into(),
        "<< /Type /Annot /Subtype /Text /Rect [70 10 90 30] /IRT 7 0 R >>".into(),
    ]);
    let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    let mut builder = Builder::from_document(doc);
    annots::remove_many(&mut builder, &[(0, id(5)), (1, id(7))]).unwrap();
    let doc = builder.document();
    let get = |n| doc.resolve_id(id(n)).as_dict().unwrap().clone();
    assert_eq!(
        doc.dict_get(&get(3), b"Annots"),
        &Object::Array(vec![Object::Reference(id(8))])
    );
    assert_eq!(doc.dict_get(&get(4), b"Annots"), &Object::Array(Vec::new()));
    assert!(!get(8).contains_key(b"IRT".as_slice()));
    assert!(matches!(
        annots::remove_many(&mut builder, &[(2, id(5))]),
        Err(annots::AnnotError::NoPage)
    ));
}

#[test]
fn a_squiggly_draws_a_bounded_zigzag_however_many_thin_quads_it_has() {
    use tandem_pdf_engine::app::annots::appearance::{self, Mark};
    // 400 quads 590 points long and a thousandth high: MuPDF's h/7 steps
    // would take over a billion points.
    let quads: Vec<annots::Quad> = (0..400)
        .map(|i| {
            let y = 100.0 + f64::from(i);
            [(10.0, y + 0.001), (600.0, y + 0.001), (10.0, y), (600.0, y)]
        })
        .collect();
    let (form, rect) = appearance::text_line(Mark::Squiggly, &quads, None, 1.0);
    assert!(form.content.len() < 2 << 20, "{}", form.content.len());
    assert!(rect.iter().all(|v| v.is_finite()));
    // An ordinary squiggle is MuPDF's: steps of h/7.
    let one = [[(0.0, 14.0), (14.0, 14.0), (0.0, 0.0), (14.0, 0.0)]];
    let (form, _) = appearance::text_line(Mark::Squiggly, &one, None, 1.0);
    let text = String::from_utf8(form.content).unwrap();
    assert_eq!(text.matches(" l\n").count(), 7, "{text}");

    // The same in a file without an appearance: the view draws it at once.
    let flat: Vec<String> = quads
        .iter()
        .flatten()
        .map(|(x, y)| format!("{x} {y}"))
        .collect();
    let bytes = page_with(
        "",
        &[&format!(
            "<< /Type /Annot /Subtype /Squiggly /Rect [0 0 1 1] /C [1 0 0] /QuadPoints [{}] >>",
            flat.join(" ")
        )],
    );
    let (_, mut session) = session_with(bytes);
    session
        .view()
        .unwrap()
        .render(0, 0.25, RenderOptions::default())
        .unwrap();
}

#[test]
fn free_text_lays_out_inside_its_rect_differences() {
    // MuPDF 1.28.1 (setContents + update) on this box draws the border at
    // 120.5 610.5 149 79 and the text frame at 120 610.
    let bytes = page_with(
        "",
        &[
            "<< /Type /Annot /Subtype /FreeText /Rect [100 600 300 700] /RD [20 10 30 10] /DA (/Helv 12 Tf 0 0 0 rg) /Contents (x) >>",
        ],
    );
    let (mut engine, mut session) = session_with(bytes);
    assert_near(
        &rect_of(&items(&mut engine, &mut session)[0]),
        &[120.0, 102.0, 270.0, 182.0],
        "listed",
    );
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-set", "page": 0, "annotId": 5, "contents": "changed"}),
    );
    let (form, content) = appearance(&session, 5);
    assert!(
        content
            .starts_with("0 0 0 RG\n1 w\n120.5 610.5 149 79 re\nS\n121 611 148 78 re\nW\nn\nq\n"),
        "{content}"
    );
    assert!(
        content.contains(" 120 610 cm\nBT\n0 0 0 rg\n2 82.8 Td\n"),
        "{content}"
    );
    let doc = document(&session);
    assert_eq!(
        nums(doc, form.get(b"BBox".as_slice())),
        [100.0, 600.0, 300.0, 700.0]
    );
    assert_eq!(
        nums(doc, annot(&session, 5).get(b"Rect".as_slice())),
        [100.0, 600.0, 300.0, 700.0]
    );
    // Moving keeps the margins: the listed box lands on the point, its size kept.
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-move", "page": 0, "annotId": 5, "point": [50, 60]}),
    );
    assert_near(
        &rect_of(&items(&mut engine, &mut session)[0]),
        &[50.0, 60.0, 200.0, 140.0],
        "moved",
    );
}

#[test]
fn a_stamp_signature_with_rect_differences_moves_at_a_stable_size() {
    let form = "<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Length 9 >>\nstream\n0 0 1 1 re f\nendstream";
    for with_ap in [true, false] {
        let ap = if with_ap { "/AP << /N 6 0 R >>" } else { "" };
        let stamp = format!(
            "<< /Type /Annot /Subtype /Stamp /Rect [100 600 200 650] /RD [5 5 5 5] /Subj (Signature appearance) {ap} >>"
        );
        let (mut engine, mut session) = session_with(page_with("", &[&stamp, form]));
        let first = rect_of(&items(&mut engine, &mut session)[0]);
        let (w, h) = (first[2] - first[0], first[3] - first[1]);
        if with_ap {
            // MuPDF's getRect: /Rect less /RD.
            assert_near(&first, &[105.0, 147.0, 195.0, 187.0], "listed");
        }
        for (step, at) in [50.0, 60.0, 70.0].into_iter().enumerate() {
            op(
                &mut engine,
                &mut session,
                json!({"op": "annot-move", "page": 0, "annotId": 5, "point": [at, at]}),
            );
            let moved = rect_of(&items(&mut engine, &mut session)[0]);
            assert_near(
                &moved,
                &[at, at, at + w, at + h],
                &format!("{with_ap} move {step}"),
            );
        }
    }
}

#[test]
fn moving_a_drawn_signature_without_strokes_is_refused() {
    for ink in ["/InkList []", "/InkList [[]]", ""] {
        let bytes = page_with(
            "",
            &[&format!(
                "<< /Type /Annot /Subtype /Ink /Rect [100 100 300 200] {ink} /Subj (Drawn signature) >>"
            )],
        );
        let (mut engine, mut session) = session_with(bytes);
        let refused = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "annot-move", "page": 0, "annotId": 5, "point": [50, 50]}),
        );
        assert_eq!(
            refused["error"], "annot-move: this annotation has no geometry to move",
            "{ink}"
        );
        let state = op(
            &mut engine,
            &mut session,
            json!({"op": "annots", "page": 0}),
        );
        assert_eq!(state["items"].as_array().unwrap().len(), 1);
        assert_eq!(
            dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"],
            false
        );
    }
}

#[test]
fn annotations_without_an_appearance_list_and_save_the_rect_mupdf_derives() {
    // MuPDF 1.28.1's getRect on each, and the /Rect and /RD its update writes.
    let bytes = page_with(
        "",
        &[
            "<< /Type /Annot /Subtype /Text /Rect [72 400 200 450] >>",
            "<< /Type /Annot /Subtype /Stamp /Rect [100 400 300 600] /Name /Approved >>",
            "<< /Type /Annot /Subtype /Square /Rect [100 100 200 150] /BS << /W 1 >> >>",
            "<< /Type /Annot /Subtype /Circle /Rect [250 100 350 150] /BS << /W 4 >> >>",
            "<< /Type /Annot /Subtype /Caret /Rect [50 50 70 70] /C [0 0 1] >>",
            "<< /Type /Annot /Subtype /Redact /Rect [100 600 300 700] /QuadPoints [110 690 200 690 110 680 200 680] >>",
            "<< /Type /Annot /Subtype /FreeText /Rect [100 600 300 700] >>",
        ],
    );
    let (mut engine, mut session) = session_with(bytes);
    let listed = items(&mut engine, &mut session);
    let expected = [
        [72.0, 342.0, 92.0, 362.0],
        [100.0, 265.68421, 300.0, 318.31579],
        [100.5, 642.5, 199.5, 691.5],
        [252.0, 644.0, 348.0, 690.0],
        [50.0, 725.0, 70.0, 739.0],
        [109.0, 101.0, 201.0, 113.0],
        [100.0, 92.0, 300.0, 192.0],
    ];
    for (item, rect) in listed.iter().zip(expected) {
        assert_near(&rect_of(item), &rect, item["type"].as_str().unwrap());
    }
    // The view draws the stamp in that rect, not stretched over /Rect.
    let image = session
        .view()
        .unwrap()
        .render(0, 1.0, RenderOptions::default())
        .unwrap();
    let stamp_region = |x: u32, y: u32| {
        let i = ((y * image.width + x) * 4) as usize;
        image.pixels[i..i + 3].iter().any(|&c| c < 250)
    };
    assert!(
        (200..260).all(|y| !stamp_region(200, y)),
        "nothing above the stamp"
    );

    let set = |engine: &mut AppEngine, session: &mut AppSession, number: u32| {
        op(
            engine,
            session,
            json!({"op": "annot-set", "page": 0, "annotId": number, "contents": "changed"}),
        );
        let doc = document(session);
        let dict = annot(session, number);
        let rd = dict.get(b"RD".as_slice()).map(|rd| nums(doc, Some(rd)));
        (nums(doc, dict.get(b"Rect".as_slice())), rd)
    };
    let (rect, _) = set(&mut engine, &mut session, 5);
    assert_eq!(rect, [72.0, 430.0, 92.0, 450.0]);
    let (rect, _) = set(&mut engine, &mut session, 6);
    assert_near(&rect, &[100.0, 473.68421, 300.0, 526.31579], "stamp");
    let (rect, rd) = set(&mut engine, &mut session, 7);
    assert_eq!(
        (rect, rd),
        (vec![100.0, 100.0, 200.0, 150.0], Some(vec![0.5; 4]))
    );
    let (_, rd) = set(&mut engine, &mut session, 8);
    assert_eq!(rd, Some(vec![2.0; 4]));
    // The listing is unchanged by the edits that wrote those rects.
    let after = items(&mut engine, &mut session);
    for (item, rect) in after.iter().zip(expected).take(4) {
        assert_near(&rect_of(item), &rect, "after set");
    }
}

#[test]
fn stamps_on_turned_pages_draw_inside_their_rect() {
    for rotate in [90, 180, 270] {
        let (mut engine, mut session) = session_with(page_with(&format!("/Rotate {rotate}"), &[]));
        op(
            &mut engine,
            &mut session,
            json!({"op": "annot-add", "page": 0, "type": "Stamp", "rect": [100, 100, 260, 160], "icon": "Approved", "contents": ""}),
        );
        let request = json!({"op": "annot-add", "page": 0, "type": "Stamp", "rect": [300, 300, 400, 360],
            "image": {"name": "g.png", "bytes": {"$buffer": 0}}, "contents": ""});
        let reply = engine.dispatch(&mut session, request, vec![png(30, 9)]).0;
        assert_eq!(reply["ok"], true, "{reply}");
        let listed = items(&mut engine, &mut session);
        for item in &listed {
            let rect = rect_of(item);
            let image = session
                .view()
                .unwrap()
                .render(0, 1.0, RenderOptions::default())
                .unwrap();
            // Only the pixels near this stamp's rect: the other is far away.
            let mut found: Option<[f64; 4]> = None;
            for y in 0..image.height {
                for x in 0..image.width {
                    let (fx, fy) = (f64::from(x), f64::from(y));
                    let near = fx >= rect[0] - 60.0
                        && fx <= rect[2] + 60.0
                        && fy >= rect[1] - 60.0
                        && fy <= rect[3] + 60.0;
                    let i = ((y * image.width + x) * 4) as usize;
                    if near && image.pixels[i..i + 3].iter().any(|&c| c < 250) {
                        let b = found.get_or_insert([fx, fy, fx, fy]);
                        *b = [b[0].min(fx), b[1].min(fy), b[2].max(fx), b[3].max(fy)];
                    }
                }
            }
            let ink = found.unwrap();
            assert!(
                ink[0] >= rect[0] - 1.0
                    && ink[1] >= rect[1] - 1.0
                    && ink[2] <= rect[2] + 1.0
                    && ink[3] <= rect[3] + 1.0,
                "/Rotate {rotate}: {ink:?} outside {rect:?}"
            );
            assert!(
                ink[2] - ink[0] > (rect[2] - rect[0]) * 0.8,
                "/Rotate {rotate}: {ink:?} fills {rect:?}"
            );
        }
    }

    // A foreign appearance turned by its /Matrix on an unturned page.
    let bytes = page_with(
        "",
        &[
            "<< /Type /Annot /Subtype /Stamp /Rect [100 400 300 600] /AP << /N 6 0 R >> >>",
            "<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Matrix [0 1 -1 0 0 0] /Length 12 >>\nstream\n0 0 1 1 re f\nendstream",
        ],
    );
    let (_, mut session) = session_with(bytes);
    let image = session
        .view()
        .unwrap()
        .render(0, 1.0, RenderOptions::default())
        .unwrap();
    assert_eq!(
        ink_box(&image.pixels, image.width, image.height),
        Some([100, 192, 299, 391])
    );
}

#[test]
fn pages_sharing_one_annots_array_draw_its_annotations_on_each() {
    let page = |n: usize| {
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots {n} 0 R >>")
    };
    let pages = 40;
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {pages} >>",
            (0..pages)
                .map(|i| format!("{} 0 R", 4 + i))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    ];
    let square = "<< /Subtype /Square /Rect [100 600 200 700] /IC [0 0 1] >>";
    objects.push(format!("[{}]", vec![square; pages].join(" ")));
    objects.extend((0..pages).map(|_| page(3)));
    let (_, mut session) = session_with(classic(&objects));
    let view = session.view().unwrap();
    for index in [0, pages - 1] {
        let image = view.render(index, 1.0, RenderOptions::default()).unwrap();
        let i = ((142 * image.width + 150) * 4) as usize;
        assert_eq!(image.pixels[i..i + 3], [0, 0, 255], "page {index}");
    }
}

// ------------------------------------------------------- acceptance fixes

/// Three pages sharing /Annots 5 0 R (a Square), a fourth written into
/// /Kids sharing it too.
fn shared_annots() -> Vec<u8> {
    let page =
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots 5 0 R >>";
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 7 0 R 8 0 R << /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots 5 0 R >>] /Count 4 >>".into(),
        page.into(),
        "<< /Length 0 >>\nstream\n\nendstream".into(),
        "[6 0 R]".into(),
        "<< /Type /Annot /Subtype /Square /Rect [10 10 50 50] /Contents (shared) >>".into(),
        page.into(),
        page.into(),
    ])
}

fn listed_types(engine: &mut AppEngine, session: &mut AppSession, page: usize) -> Vec<String> {
    let reply = op(engine, session, json!({"op": "annots", "page": page}));
    (reply["items"].as_array().unwrap().iter())
        .map(|item| item["type"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn an_add_on_a_page_sharing_its_annots_array_lands_on_that_page_alone() {
    let (mut engine, mut session) = session_with(shared_annots());
    let before = serialized(&session);
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "Square", "rect": [200, 200, 300, 300], "contents": "p0"}),
    );
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "Text", "point": [100, 100], "contents": "note"}),
    );
    // The page written into /Kids gets its copy there.
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 3, "type": "Circle", "rect": [200, 200, 300, 300], "contents": "p3"}),
    );
    // So does a link (MuPDF's createLink).
    op(
        &mut engine,
        &mut session,
        json!({"op": "link-add", "page": 1, "rect": [72, 400, 200, 430], "target": {"uri": "https://example.org/x"}}),
    );
    assert_eq!(
        listed_types(&mut engine, &mut session, 0),
        ["Square", "Square", "Text"]
    );
    assert_eq!(listed_types(&mut engine, &mut session, 1), ["Square"]);
    assert_eq!(listed_types(&mut engine, &mut session, 2), ["Square"]);
    assert_eq!(
        listed_types(&mut engine, &mut session, 3),
        ["Square", "Circle"]
    );
    let doc = document(&session);
    let shared = doc.resolve_id(id(5)).as_array().unwrap();
    assert_eq!(
        shared,
        [Object::Reference(id(6))],
        "the shared array is unchanged"
    );
    let pages = tandem_pdf_engine::app::info::pages(doc);
    let annots_of = |page: usize| pages[page].dict.get(b"Annots".as_slice()).unwrap().clone();
    // The shared Square, the new Square, the note and its popup.
    assert!(matches!(annots_of(0), Object::Array(ref a) if a.len() == 4));
    assert!(matches!(annots_of(1), Object::Array(ref a) if a.len() == 2));
    assert_eq!(annots_of(2), Object::Reference(id(5)));
    assert!(matches!(annots_of(3), Object::Array(ref a) if a.len() == 2));
    // The save keeps it so: page 2 lists the Square alone.
    let (mut engine2, mut reopened) = session_with(saved(&session));
    assert_eq!(listed_types(&mut engine2, &mut reopened, 2), ["Square"]);
    assert_eq!(
        listed_types(&mut engine2, &mut reopened, 0),
        ["Square", "Square", "Text"]
    );
    for _ in 0..4 {
        op(&mut engine, &mut session, json!({"op": "undo"}));
    }
    assert_eq!(serialized(&session), before);
}

#[test]
fn free_text_draws_greek_cyrillic_cjk_and_latin_letters_in_fallback_fonts() {
    let (mut engine, mut session) = session_with(blank());
    let text = "Ωμέγα Жук 日本語 Zażółć";
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "FreeText", "point": [40, 60], "contents": text}),
    );
    let number = last_id(&mut engine, &mut session);
    let (form, content) = latin1_appearance(&session, number);
    let doc = document(&session);
    let resources = doc.dict_get(&form, b"Resources").as_dict().unwrap();
    let fonts = doc.dict_get(resources, b"Font").as_dict().unwrap();
    let names: Vec<&[u8]> = fonts.keys().map(Vec::as_slice).collect();
    assert_eq!(
        names,
        [
            b"Helv".as_slice(),
            b"HelvGRK",
            b"HelvCYR",
            b"Song",
            b"HelvLAT"
        ]
    );
    assert!(!content.contains('?'), "{content}");
    // MuPDF's runs: ISO 8859-7, KOI8-U, UTF-16 in Song (with the space).
    assert!(
        content.contains("/HelvGRK 12 Tf\n(\u{d9}\u{ec}\u{dd}\u{e3}\u{e1} ) Tj\n/HelvCYR 12 Tf\n(\u{f6}\u{d5}\u{cb} ) Tj\n/Song 12 Tf\n<65e5672c8a9e0020> Tj\n"),
        "{content}"
    );
    // A CJK font is three objects: font, CIDFont, descriptor.
    let song = fonts.get(b"Song".as_slice()).unwrap().as_reference();
    let song = doc.resolve_id(song.expect("an object")).as_dict().unwrap();
    assert_eq!(
        doc.dict_get(song, b"Subtype").as_name(),
        Some(b"Type0".as_slice())
    );
    let descendant = doc.dict_get(song, b"DescendantFonts").as_array().unwrap()[0]
        .as_reference()
        .expect("an object");
    let descendant = doc.resolve_id(descendant).as_dict().unwrap();
    let descriptor = descendant.get(b"FontDescriptor".as_slice()).unwrap();
    assert!(descriptor.as_reference().is_some());
    let latin = doc.dict_get(fonts, b"HelvLAT").as_dict().unwrap();
    let encoding = doc.dict_get(latin, b"Encoding").as_dict().unwrap();
    let differences = doc.dict_get(encoding, b"Differences").as_array().unwrap();
    assert!(differences.contains(&Object::name("zdotaccent")));
    assert!(differences.contains(&Object::name("lslash")));
    // The saved file draws each letter: its pixels differ from a `?` and
    // from nothing (the view draws the base 14 fonts with URW's glyphs).
    let drawn = |text: &str| {
        let (mut engine, mut session) = session_with(blank());
        op(
            &mut engine,
            &mut session,
            json!({"op": "annot-add", "page": 0, "type": "FreeText", "point": [40, 60], "contents": text}),
        );
        let (_, mut reopened) = session_with(saved(&session));
        let image = reopened
            .view()
            .unwrap()
            .render(0, 2.0, RenderOptions::default())
            .unwrap();
        let mut region = Vec::new();
        for y in 120..200 {
            for x in 80..160 {
                let i = ((y * image.width + x) * 4) as usize;
                region.push(image.pixels[i]);
            }
        }
        region
    };
    let question = drawn("?");
    for letter in ["Ω", "μ", "Ж", "ż", "ł", "ő", "ș"] {
        let region = drawn(letter);
        assert!(region.iter().any(|&v| v < 128), "{letter} draws");
        assert_ne!(region, question, "{letter} is not a ?");
    }

    // Editing a foreign Cyrillic FreeText's contents keeps its script.
    let bytes = page_with(
        "",
        &[
            "<< /Type /Annot /Subtype /FreeText /Rect [100 600 300 700] /DA (/Helv 12 Tf 0 g) /Contents (x) >>",
        ],
    );
    let (mut engine, mut session) = session_with(bytes);
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-set", "page": 0, "annotId": 5, "contents": "Привет мир"}),
    );
    let (form, content) = latin1_appearance(&session, 5);
    assert!(content.contains("/HelvCYR 12 Tf\n"), "{content}");
    assert!(!content.contains('?'), "{content}");
    let resources = document(&session).dict_get(&form, b"Resources");
    assert!(
        resources
            .as_dict()
            .unwrap()
            .get(b"Font".as_slice())
            .is_some()
    );
    // Moving a FreeText without an appearance writes one with its script.
    let bytes = page_with(
        "",
        &["<< /Type /Annot /Subtype /FreeText /Rect [100 600 300 700] /Contents <FEFF03A9> >>"],
    );
    let (mut engine, mut session) = session_with(bytes);
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-move", "page": 0, "annotId": 5, "point": [50, 50]}),
    );
    let (_, content) = latin1_appearance(&session, 5);
    assert!(
        content.contains("/HelvGRK 12 Tf\n(\u{d9}) Tj\n"),
        "{content}"
    );
}

/// `appearance` with the content's bytes read as Latin-1.
fn latin1_appearance(session: &AppSession, number: u32) -> (Dictionary, String) {
    let doc = document(session);
    let dict = annot(session, number);
    let ap = doc.dict_get(&dict, b"AP").as_dict().unwrap();
    let n = ap.get(b"N".as_slice()).unwrap().as_reference().unwrap();
    let form = doc.resolve_id(n).as_dict().unwrap().clone();
    let content = doc.decoded_stream(n, StreamLimits::default()).unwrap();
    (form, content.iter().map(|&b| char::from(b)).collect())
}

#[test]
fn an_icon_stamp_fits_inside_a_rect_wider_than_its_icon() {
    let (mut engine, mut session) = session_with(blank());
    // MuPDF 1.28.1: [115 10 305 60] and [134 10 476 100].
    for (rect, expected) in [
        ([10.0, 10.0, 410.0, 60.0], [115.0, 10.0, 305.0, 60.0]),
        ([10.0, 10.0, 600.0, 100.0], [134.0, 10.0, 476.0, 100.0]),
    ] {
        op(
            &mut engine,
            &mut session,
            json!({"op": "annot-add", "page": 0, "type": "Stamp", "rect": rect, "icon": "Approved", "contents": ""}),
        );
        let listed = items(&mut engine, &mut session);
        assert_near(&rect_of(listed.last().unwrap()), &expected, "listed");
        let number = last_id(&mut engine, &mut session);
        let saved_rect = nums(
            document(&session),
            annot(&session, number).get(b"Rect".as_slice()),
        );
        assert_near(
            &saved_rect,
            &[
                expected[0],
                792.0 - expected[3],
                expected[2],
                792.0 - expected[1],
            ],
            "saved",
        );
    }
    // A foreign stamp without an appearance lists and writes the same fit.
    let bytes = page_with(
        "",
        &["<< /Type /Annot /Subtype /Stamp /Rect [300 600 500 650] /Name /Approved >>"],
    );
    let (mut engine, mut session) = session_with(bytes);
    let listed = items(&mut engine, &mut session);
    assert_near(
        &rect_of(&listed[0]),
        &[305.0, 142.0, 495.0, 192.0],
        "foreign",
    );
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-set", "page": 0, "annotId": 5, "contents": "x"}),
    );
    let rect = nums(
        document(&session),
        annot(&session, 5).get(b"Rect".as_slice()),
    );
    assert_near(&rect, &[305.0, 600.0, 495.0, 650.0], "set");
}

#[test]
fn identical_stamp_images_share_one_image_object() {
    let (mut engine, mut session) = session_with(page_with("", &[]));
    let add = |engine: &mut AppEngine, session: &mut AppSession, image: Vec<u8>, extra: Value| {
        let mut request = json!({"op": "annot-add", "page": 0, "type": "Stamp", "rect": [100, 100, 300, 160],
            "image": {"name": "g.png", "bytes": {"$buffer": 0}}, "contents": ""});
        for (key, value) in extra.as_object().unwrap() {
            request[key] = value.clone();
        }
        let reply = engine.dispatch(session, request, vec![image]).0;
        assert_eq!(reply["ok"], true, "{reply}");
        let number = last_id(engine, session);
        let (form, _) = appearance(session, number);
        let doc = document(session);
        let resources = doc.dict_get(&form, b"Resources").as_dict().unwrap();
        let xobjects = doc.dict_get(resources, b"XObject").as_dict().unwrap();
        xobjects
            .get(b"I".as_slice())
            .unwrap()
            .as_reference()
            .unwrap()
    };
    let first = add(&mut engine, &mut session, png(30, 9), json!({}));
    let second = add(&mut engine, &mut session, png(30, 9), json!({}));
    let signature = add(
        &mut engine,
        &mut session,
        png(30, 9),
        json!({"signature": true}),
    );
    assert_eq!((second, signature), (first, first), "one image object");
    let other = add(&mut engine, &mut session, png(31, 9), json!({}));
    assert_ne!(other, first);
    // Undo and redo are exact.
    let before = serialized(&session);
    for _ in 0..3 {
        op(&mut engine, &mut session, json!({"op": "undo"}));
    }
    for _ in 0..3 {
        op(&mut engine, &mut session, json!({"op": "redo"}));
    }
    assert_eq!(serialized(&session), before);
    // Once the stamp that stored it is undone, nothing shows the image and
    // a new stamp stores it anew.
    for _ in 0..4 {
        op(&mut engine, &mut session, json!({"op": "undo"}));
    }
    let again = add(&mut engine, &mut session, png(30, 9), json!({}));
    assert_ne!(again, first, "nothing left to share");
    // A stamp deleted on another page no longer offers its image either;
    // one on any page does.
    let (mut engine, mut session) = session_with(shared_annots());
    let on_page = |engine: &mut AppEngine, session: &mut AppSession, page: usize| {
        let request = json!({"op": "annot-add", "page": page, "type": "Stamp", "rect": [100, 100, 300, 160],
            "image": {"name": "g.png", "bytes": {"$buffer": 0}}, "contents": ""});
        let reply = engine.dispatch(session, request, vec![png(30, 9)]).0;
        assert_eq!(reply["ok"], true, "{reply}");
        let listed = op(engine, session, json!({"op": "annots", "page": page}));
        let item = listed["items"].as_array().unwrap().last().unwrap().clone();
        let number = item["id"].as_u64().unwrap() as u32;
        let (form, _) = appearance(session, number);
        let doc = document(session);
        let resources = doc.dict_get(&form, b"Resources").as_dict().unwrap();
        let xobjects = doc.dict_get(resources, b"XObject").as_dict().unwrap();
        (
            number,
            xobjects
                .get(b"I".as_slice())
                .unwrap()
                .as_reference()
                .unwrap(),
        )
    };
    let (stamp, image) = on_page(&mut engine, &mut session, 1);
    assert_eq!(on_page(&mut engine, &mut session, 2).1, image);
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-delete", "page": 1, "annotId": stamp}),
    );
    assert_eq!(
        on_page(&mut engine, &mut session, 3).1,
        image,
        "page 2 still shows it"
    );
}

#[test]
fn a_delete_cuts_a_name_or_number_tree_pair_whole() {
    // Malformed trees naming the annotation (5): the cut takes its key too,
    // so the pairs after it keep their places.
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R /Names << /IDS 7 0 R >> /PageLabels 8 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [5 0 R] >>"
            .into(),
        "<< /Length 0 >>\nstream\n\nendstream".into(),
        "<< /Type /Annot /Subtype /Square /Rect [10 10 50 50] >>".into(),
        "<< /S /D >>".into(),
        "<< /Names [(a) 5 0 R (b) 6 0 R] >>".into(),
        "<< /Nums [0 5 0 R 1 6 0 R 2 [5 0 R 6 0 R]] >>".into(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-delete", "page": 0, "annotId": 5}),
    );
    let doc = document(&session);
    let names = doc.resolve_id(id(7)).as_dict().unwrap();
    assert_eq!(
        names.get(b"Names".as_slice()).unwrap().as_array().unwrap(),
        [Object::String(b"b".to_vec()), Object::Reference(id(6))]
    );
    let nums = doc.resolve_id(id(8)).as_dict().unwrap();
    assert_eq!(
        nums.get(b"Nums".as_slice()).unwrap().as_array().unwrap(),
        [
            Object::Integer(1),
            Object::Reference(id(6)),
            Object::Integer(2),
            Object::Array(vec![Object::Reference(id(6))])
        ]
    );
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
}
