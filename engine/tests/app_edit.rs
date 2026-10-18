// SPDX-License-Identifier: AGPL-3.0-or-later
//! Content editing on the app session: target discovery, text, paragraph
//! and replace edits, text-add, copy-on-write of shared content and Forms.
//! Every write is one journal step that undo restores byte for byte.
use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::document::Document;
use tandem_pdf_engine::object::{Object, ObjectId};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::writer::{DocumentWriteLimits, WriteOptions, write_document};

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

fn stream(dictionary: &str, data: &[u8]) -> Vec<u8> {
    let mut bytes = format!("<< {dictionary} /Length {} >>\nstream\n", data.len()).into_bytes();
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(b"\nendstream");
    bytes
}

const HELVETICA: &[u8] =
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";

/// One page (object 3) drawing `content` (object 5) with Helvetica as /F1.
fn one_page(content: &str, extra: &[Vec<u8>]) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> /XObject << /Fm1 6 0 R >> >> /Contents 5 0 R >>".to_vec(),
        HELVETICA.to_vec(),
        stream("", content.as_bytes()),
    ];
    objects.extend_from_slice(extra);
    classic(&objects)
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

/// Every object written, for exact before/after comparisons.
fn serialized(session: &AppSession) -> Vec<u8> {
    let options = WriteOptions {
        garbage: false,
        ..WriteOptions::APP
    };
    write_document(document(session), DocumentWriteLimits::default(), options).unwrap()
}

fn decoded(session: &AppSession, object: ObjectId) -> String {
    let bytes = document(session)
        .decoded_stream(object, StreamLimits::default())
        .unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The decoded content of page 0's /Contents, joined.
fn page_content(session: &AppSession) -> String {
    let doc = document(session);
    let page = doc.resolve_id(id(3)).as_dict().unwrap();
    let contents = doc.resolve(page.get(b"Contents".as_slice()).unwrap());
    let ids: Vec<ObjectId> = match contents {
        Object::Array(items) => items.iter().filter_map(Object::as_reference).collect(),
        _ => vec![
            page.get(b"Contents".as_slice())
                .unwrap()
                .as_reference()
                .unwrap(),
        ],
    };
    ids.iter()
        .map(|&i| decoded(session, i))
        .collect::<Vec<_>>()
        .join("\n")
}

fn scan(engine: &mut AppEngine, session: &mut AppSession) -> Value {
    op(engine, session, json!({"op": "edit-targets", "page": 0}))
}

fn target<'r>(reply: &'r Value, text: &str) -> &'r Value {
    reply["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["text"] == text)
        .unwrap_or_else(|| panic!("no {text:?} in {reply}"))
}

fn edit(
    engine: &mut AppEngine,
    session: &mut AppSession,
    scan: &Value,
    text: &str,
    fields: Value,
) -> Value {
    let item = target(scan, text);
    let mut request = json!({"op": "edit-text", "page": 0, "targetId": item["targetId"],
        "generation": scan["generation"], "revision": scan["revision"]});
    if let (Value::Object(request), Value::Object(fields)) = (&mut request, fields) {
        request.extend(fields);
    }
    dispatch(engine, session, request)
}

#[test]
fn targets_report_runs_with_ids_bounds_and_fonts() {
    let bytes = one_page(
        "BT /F1 12 Tf 72 700 Td (ALPHA) Tj ( TAIL) Tj ET\nBT /F1 12 Tf 72 600 Td [(Ker) -50 (ned)] TJ ET\n",
        &[],
    );
    let (mut engine, mut session) = session_with(bytes);
    let reply = scan(&mut engine, &mut session);
    assert_eq!(reply["revision"], 0);
    assert_eq!(reply["transform"], json!([1.0, 0.0, 0.0, -1.0, 0.0, 792.0]));
    let items = reply["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    let alpha = target(&reply, "ALPHA");
    assert_eq!(alpha["support"], "candidate");
    assert_eq!(alpha["tailFollows"], true);
    assert_eq!(
        alpha["font"],
        json!({"name": "Helvetica", "subtype": "Type1", "embedded": false, "subset": false,
            "writingMode": null})
    );
    let id = alpha["targetId"].as_str().unwrap();
    assert!(id.starts_with("p0.c0.31."), "{id}");
    let bounds = alpha["bounds"].as_array().unwrap();
    assert!((bounds[0].as_f64().unwrap() - 72.0).abs() < 0.01);
    assert_eq!(target(&reply, "Kerned")["kerned"], true);
    // A second scan of the same revision names the same targets.
    assert_eq!(scan(&mut engine, &mut session)["items"], reply["items"]);
}

#[test]
fn edit_text_splices_the_operand_and_undo_restores_every_byte() {
    let bytes = one_page(
        "BT /F1 12 Tf 72 700 Td (ALPHA) Tj ET\nBT /F1 12 Tf 72 500 Td (OTHER) Tj ET\n",
        &[],
    );
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    let reply = scan(&mut engine, &mut session);
    let edited = edit(
        &mut engine,
        &mut session,
        &reply,
        "ALPHA",
        json!({"text": "ALP(H)"}),
    );
    assert_eq!(edited["ok"], true, "{edited}");
    assert_eq!(edited["revision"], 1);
    assert_eq!(edited["shift"].as_f64(), Some(0.0));
    assert_eq!(
        page_content(&session),
        "BT /F1 12 Tf 72 700 Td (ALP\\(H\\)) Tj ET\nBT /F1 12 Tf 72 500 Td (OTHER) Tj ET\n"
    );
    let after = serialized(&session);
    let undo = op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(undo["revision"], 2);
    assert_eq!(serialized(&session), before);
    op(&mut engine, &mut session, json!({"op": "redo"}));
    assert_eq!(serialized(&session), after);
    let steps = session.doc.as_ref().unwrap().steps();
    assert_eq!(steps, ["edit text"]);
    // The old handle is stale now.
    let stale = edit(
        &mut engine,
        &mut session,
        &reply,
        "ALPHA",
        json!({"text": "X"}),
    );
    assert_eq!(stale["detail"], "stale-target", "{stale}");
}

#[test]
fn width_gates_report_widths_and_write_nothing() {
    let bytes = one_page(
        "BT /F1 12 Tf 72 700 Td (ALPHA) Tj ET\nBT /F1 12 Tf 300 700 Td (WALL) Tj ET\n",
        &[],
    );
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    let reply = scan(&mut engine, &mut session);
    let wide = edit(
        &mut engine,
        &mut session,
        &reply,
        "ALPHA",
        json!({"text": "ALPHA BRAVO CHARLIE DELTA ECHO FOXTROT"}),
    );
    assert_eq!(wide["detail"], "overflow", "{wide}");
    // ALPHA in Helvetica at 12 pt: (667 + 556 + 667 + 722 + 667) / 1000 x 12.
    assert_eq!(wide["oldWidth"], 39.35);
    assert!(wide["limit"].as_f64().unwrap() < 300.0);
    let missing = edit(
        &mut engine,
        &mut session,
        &reply,
        "ALPHA",
        json!({"text": "AL\u{4e00}"}),
    );
    assert_eq!(missing["detail"], "unsupported-encoding");
    assert_eq!(missing["missing"], json!(["\u{4e00}"]));
    let not_string = edit(
        &mut engine,
        &mut session,
        &reply,
        "ALPHA",
        json!({"text": 7}),
    );
    assert_eq!(not_string["detail"], "unsupported-encoding");
    let same = edit(
        &mut engine,
        &mut session,
        &reply,
        "ALPHA",
        json!({"text": "ALPHA"}),
    );
    assert_eq!(same["ok"], true);
    assert_eq!(same["revision"], 0, "a no-op opens no operation");
    assert_eq!(serialized(&session), before);
}

#[test]
fn move_and_size_write_exact_inverses() {
    let bytes = one_page("BT /F1 12 Tf 72 700 Td (ALPHA) Tj ET\n", &[]);
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    let reply = scan(&mut engine, &mut session);
    let moved = edit(
        &mut engine,
        &mut session,
        &reply,
        "ALPHA",
        json!({"move": {"dx": 10, "dy": -20}, "size": 18}),
    );
    assert_eq!(moved["ok"], true, "{moved}");
    assert_eq!(
        page_content(&session),
        "BT /F1 12 Tf 72 700 Td 10 20 Td /F1 18 Tf (ALPHA) Tj /F1 12 Tf -10 -20 Td ET\n"
    );
    assert_eq!(session.doc.as_ref().unwrap().steps(), ["move text"]);
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
    // Leaving the page is refused with the moved bounds.
    let reply = scan(&mut engine, &mut session);
    let off = edit(
        &mut engine,
        &mut session,
        &reply,
        "ALPHA",
        json!({"move": {"dx": 0, "dy": -200}}),
    );
    assert_eq!(off["detail"], "outside-page", "{off}");
}

#[test]
fn shared_contents_are_copied_before_the_edit() {
    let bytes = classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 6 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 6 0 R >>".to_vec(),
        HELVETICA.to_vec(),
        stream("", b"BT /F1 12 Tf 72 700 Td (SHARED) Tj ET"),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    let reply = scan(&mut engine, &mut session);
    assert_eq!(target(&reply, "SHARED")["cowable"], true);
    let edited = edit(
        &mut engine,
        &mut session,
        &reply,
        "SHARED",
        json!({"text": "MINE"}),
    );
    assert_eq!(edited["ok"], true, "{edited}");
    let doc = document(&session);
    let page0 = doc.resolve_id(id(3)).as_dict().unwrap();
    let copy = page0
        .get(b"Contents".as_slice())
        .unwrap()
        .as_reference()
        .unwrap();
    assert_ne!(copy, id(6));
    assert!(
        page0.contains_key(b"Resources".as_slice()),
        "inherited resources copied"
    );
    assert_eq!(
        decoded(&session, copy),
        "BT /F1 12 Tf 72 700 Td (MINE) Tj ET"
    );
    assert_eq!(
        decoded(&session, id(6)),
        "BT /F1 12 Tf 72 700 Td (SHARED) Tj ET"
    );
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
}

#[test]
fn a_form_is_copied_under_a_fresh_name_with_its_own_resources() {
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 300 100] /Resources 7 0 R",
        b"BT /F1 14 Tf 0 10 Td (FORM TEXT) Tj ET",
    );
    let bytes = one_page(
        "q 1 0 0 1 72 600 cm /Fm1 Do Q q 1 0 0 1 72 400 cm /Fm1 Do Q\n",
        &[form, b"<< /Font << /F1 4 0 R >> >>".to_vec()],
    );
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    let reply = scan(&mut engine, &mut session);
    let items = reply["items"].as_array().unwrap();
    let texts: Vec<&Value> = items.iter().filter(|i| i["text"] == "FORM TEXT").collect();
    assert_eq!(texts.len(), 2, "one target per invocation");
    let request = json!({"op": "edit-text", "page": 0, "targetId": texts[0]["targetId"],
        "generation": reply["generation"], "revision": reply["revision"], "text": "FORM EDIT"});
    op(&mut engine, &mut session, request);
    let doc = document(&session);
    let page = doc.resolve_id(id(3)).as_dict().unwrap();
    let xobjects = doc
        .resolve(page.get(b"Resources".as_slice()).unwrap())
        .as_dict()
        .unwrap();
    let xobjects = doc
        .resolve(xobjects.get(b"XObject".as_slice()).unwrap())
        .as_dict()
        .unwrap();
    let copy = xobjects
        .get(b"Fm1c1".as_slice())
        .unwrap()
        .as_reference()
        .unwrap();
    assert_eq!(
        decoded(&session, copy),
        "BT /F1 14 Tf 0 10 Td (FORM EDIT) Tj ET"
    );
    assert_eq!(
        decoded(&session, id(6)),
        "BT /F1 14 Tf 0 10 Td (FORM TEXT) Tj ET"
    );
    let copy_dict = doc.resolve_id(copy).as_dict().unwrap();
    let resources = copy_dict
        .get(b"Resources".as_slice())
        .unwrap()
        .as_reference();
    assert!(
        resources.is_some_and(|r| r != id(7)),
        "the copy has its own /Resources"
    );
    assert_eq!(
        page_content(&session),
        "q 1 0 0 1 72 600 cm /Fm1c1 Do Q q 1 0 0 1 72 400 cm /Fm1 Do Q\n"
    );
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
}

#[test]
fn paragraphs_rewrap_in_one_span_and_restore_the_line_matrix() {
    let bytes = one_page(
        "BT /F1 12 Tf 72 700 Td (The first line of it) Tj 0 -14 Td (and the second line) Tj 0 -14 Td (and a third) Tj ET\n",
        &[],
    );
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    let reply = scan(&mut engine, &mut session);
    let paragraph = &reply["paragraphs"][0];
    assert_eq!(paragraph["lines"], 3);
    assert_eq!(paragraph["leading"], 14.0);
    let request = json!({"op": "edit-paragraph", "page": 0, "paragraphId": paragraph["paragraphId"],
        "generation": reply["generation"], "revision": reply["revision"],
        "text": "One two three four five six seven eight nine ten"});
    let edited = op(&mut engine, &mut session, request);
    assert!(edited["lines"].as_u64().unwrap() >= 2);
    let content = page_content(&session);
    assert!(
        content.starts_with("BT /F1 12 Tf 72 700 Td (One"),
        "{content}"
    );
    assert!(content.contains("0 -14 Td ("), "{content}");
    assert_eq!(session.doc.as_ref().unwrap().steps(), ["edit paragraph"]);
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
}

#[test]
fn text_add_appends_a_stream_and_finds_the_new_run() {
    let bytes = one_page("BT /F1 12 Tf 72 700 Td (ALPHA) Tj ET\n", &[]);
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    let request = json!({"op": "text-add", "page": 0, "point": [100, 200], "text": "Hello\nWorld",
        "font": "Times-Roman", "size": 20, "color": [0, 0, 1]});
    let added = op(&mut engine, &mut session, request);
    let reply = scan(&mut engine, &mut session);
    assert_eq!(added["targetId"], target(&reply, "Hello")["targetId"]);
    let content = page_content(&session);
    assert!(
        content.ends_with("q BT /PDF4AllF1 20 Tf 0 Tc 0 Tw 100 Tz 0 TL 0 Ts 0 Tr 1 0 0 1 100 592 Tm 0 0 1 rg (Hello) Tj 0 -24 Td (World) Tj ET Q"),
        "{content}"
    );
    assert_eq!(session.doc.as_ref().unwrap().steps(), ["add text"]);
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
    let bad = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "text-add", "page": 0, "point": [100, 200], "text": "\u{4e00}",
            "font": "Helvetica", "size": 12, "color": [0, 0, 0]}),
    );
    assert_eq!(bad["detail"], "unsupported-encoding");
}

#[test]
fn replace_preview_and_apply_write_one_step() {
    let bytes = one_page(
        "BT /F1 12 Tf 72 700 Td (cat sat) Tj ET\nBT /F1 12 Tf 72 600 Td (a cat) Tj ET\nBT /F1 12 Tf 72 500 Td (dog) Tj ET\n",
        &[],
    );
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    let preview = op(
        &mut engine,
        &mut session,
        json!({"op": "edit-replace-preview", "page": 0, "find": "cat", "replace": "cut"}),
    );
    let matches = preview["matches"].as_array().unwrap();
    assert_eq!(matches.len(), 2, "{preview}");
    assert_eq!(matches[0]["next"], "cut sat");
    let ids: Vec<Value> = matches.iter().map(|m| m["targetId"].clone()).collect();
    let stale = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "edit-replace-apply", "page": 0, "find": "cat", "replace": "cut",
            "previewToken": "nope", "targetIds": ids}),
    );
    assert_eq!(stale["detail"], "stale-preview");
    let applied = op(
        &mut engine,
        &mut session,
        json!({"op": "edit-replace-apply", "page": 0, "find": "cat", "replace": "cut",
            "previewToken": preview["previewToken"], "targetIds": ids}),
    );
    assert_eq!(applied["replaced"], 2);
    assert!(page_content(&session).contains("(cut sat) Tj"));
    assert!(page_content(&session).contains("(a cut) Tj"));
    assert_eq!(session.doc.as_ref().unwrap().steps(), ["replace text"]);
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
}

#[test]
fn hostile_content_is_refused_or_classified_never_a_panic() {
    let cases: &[&[u8]] = &[
        b"BT /F1 12 Tf 72 700 Td (unterminated",
        b"[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[",
        b"Q Q Q",
        b"q q q q q q q q q q q q q q q q q q q q q q q q q q q q q q q q q q q",
        b"BI /W 4 /H 4 /BPC 8 /CS /G ID \x00\x01EI\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00 EI",
        b"BI /IM true /W 8 /H 1 ID \xffEI Q",
        b"BT /F1 1e400 Tf 72 700 Td (x) Tj ET",
        b"BT /F1 12 Tf 0 0 0 0 0 0 Tm (x) Tj ET",
        b"/P <</MCID 0 /Flag true>> BDC BT /F1 12 Tf 72 700 Td (x) Tj ET EMC",
        b"<41 42",
        b"((((((((((((((((((((((((((((((((((((((((((((((((((",
        b"/Fm1 Do /Fm1 Do",
        b"BT /F1 12 Tf 72 700 Td (\xff\xfe) Tj ET",
    ];
    for content in cases {
        let content = String::from_utf8_lossy(content).into_owned();
        let bytes = one_page(&content, &[]);
        let (mut engine, mut session) = session_with(bytes);
        let reply = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "edit-targets", "page": 0}),
        );
        if reply["ok"] == false {
            assert!(
                matches!(reply["reason"].as_str(), Some("invalid" | "too-large")),
                "{content:?}: {reply}"
            );
        }
    }
    // A Form that draws itself stops at the cycle.
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /Fm1 6 0 R >> >>",
        b"/Fm1 Do",
    );
    let bytes = one_page("/Fm1 Do", &[form]);
    let (mut engine, mut session) = session_with(bytes);
    let reply = scan(&mut engine, &mut session);
    assert_eq!(reply["items"].as_array().unwrap().len(), 2);
}

#[test]
fn every_prefix_of_a_content_stream_scans_without_a_panic() {
    let content = "q 1 0 0 1 72 600 cm /Fm1 Do Q /P <</MCID 0 /ActualText (A)>> BDC BT /F1 12 Tf 13 TL 72 700 Td [(AB) -20 (C)] TJ T* (D) ' ET EMC BI /W 1 /H 1 /BPC 8 /CS /G ID \x00 EI";
    for end in 0..=content.len() {
        if !content.is_char_boundary(end) {
            continue;
        }
        let bytes = one_page(&content[..end], &[]);
        let (mut engine, mut session) = session_with(bytes);
        let _ = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "edit-targets", "page": 0}),
        );
    }
}

#[test]
fn a_stream_with_a_bad_checksum_is_read_as_the_view_reads_it() {
    let content = b"BT /F1 12 Tf 72 700 Td (DAMAGED) Tj ET";
    let mut packed = miniz_oxide::deflate::compress_to_vec_zlib(content, 6);
    if let Some(last) = packed.last_mut() {
        *last ^= 0xff;
    }
    let bytes = classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        HELVETICA.to_vec(),
        stream("/Filter /FlateDecode", &packed),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let reply = scan(&mut engine, &mut session);
    assert_eq!(target(&reply, "DAMAGED")["support"], "candidate");
    let edited = edit(
        &mut engine,
        &mut session,
        &reply,
        "DAMAGED",
        json!({"text": "DAMAGES"}),
    );
    assert_eq!(edited["ok"], true, "{edited}");
    assert_eq!(
        decoded(&session, id(5)),
        "BT /F1 12 Tf 72 700 Td (DAMAGES) Tj ET"
    );
}

#[test]
fn booleans_are_operands_and_named_encodings_decode() {
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R /F2 6 0 R >> >> /Contents 5 0 R >>".to_vec(),
        HELVETICA.to_vec(),
        stream(
            "",
            b"/Span <</Flag true /Other null>> BDC BT /F1 12 Tf 72 700 Td (TRUE) Tj ET EMC\nBI /IM true /W 8 /H 1 ID \x01 EI\nBT /F2 12 Tf 72 600 Td (\xd5quote) Tj ET\n",
        ),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /MacRomanEncoding >>"
            .to_vec(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    let reply = scan(&mut engine, &mut session);
    assert_eq!(target(&reply, "TRUE")["support"], "candidate", "{reply}");
    // MacRoman 0xD5 is a right single quote, not WinAnsi's O tilde.
    assert_eq!(
        target(&reply, "\u{2019}quote")["support"],
        "candidate",
        "{reply}"
    );
    let edited = edit(
        &mut engine,
        &mut session,
        &reply,
        "\u{2019}quote",
        json!({"text": "\u{2019}quota"}),
    );
    assert_eq!(edited["ok"], true, "{edited}");
    let content = document(&session)
        .decoded_stream(id(5), StreamLimits::default())
        .unwrap();
    assert!(content.windows(8).any(|w| w == b"(\xd5quota)"));
}

#[test]
fn the_scan_cache_follows_undo_and_stale_paragraph_ids_refuse() {
    let bytes = one_page(
        "BT /F1 12 Tf 72 700 Td (line one here) Tj 0 -14 Td (line two here) Tj ET\n",
        &[],
    );
    let (mut engine, mut session) = session_with(bytes);
    let reply = scan(&mut engine, &mut session);
    op(
        &mut engine,
        &mut session,
        json!({"op": "edit-text", "page": 0,
        "targetId": target(&reply, "line one here")["targetId"],
        "generation": reply["generation"], "revision": reply["revision"], "text": "line 1 here"}),
    );
    let paragraph = &reply["paragraphs"][0]["paragraphId"];
    let stale = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "edit-paragraph", "page": 0, "paragraphId": paragraph,
            "generation": reply["generation"], "revision": reply["revision"], "text": "x"}),
    );
    assert_eq!(stale["detail"], "stale-target", "{stale}");
    op(&mut engine, &mut session, json!({"op": "undo"}));
    let again = scan(&mut engine, &mut session);
    assert_eq!(
        target(&again, "line one here")["targetId"],
        target(&reply, "line one here")["targetId"]
    );
    let missing = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "edit-paragraph", "page": 0, "paragraphId": "para.nope",
            "generation": again["generation"], "revision": again["revision"], "text": "x"}),
    );
    assert_eq!(missing["detail"], "stale-target");
}

/// Content streams of random tokens: every op either answers or refuses.
#[test]
fn random_content_never_panics() {
    const WORDS: [&str; 40] = [
        "q",
        "Q",
        "BT",
        "ET",
        "/F1",
        "12",
        "Tf",
        "Td",
        "TD",
        "T*",
        "Tm",
        "cm",
        "(abc)",
        "<414>",
        "[(a) -5 (b)]",
        "Tj",
        "TJ",
        "'",
        "\"",
        "/Fm1",
        "Do",
        "re",
        "W",
        "n",
        "0",
        "1",
        "-3.5",
        "BDC",
        "/OC",
        "<</MCID 0>>",
        "EMC",
        "BMC",
        "TL",
        "Tz",
        "Tc",
        "Tw",
        "Tr",
        "BI",
        "ID",
        "EI",
    ];
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..400 {
        let count = 5 + (next() % 60) as usize;
        let content: Vec<&str> = (0..count)
            .map(|_| WORDS[(next() % WORDS.len() as u64) as usize])
            .collect();
        let bytes = one_page(&content.join(" "), &[]);
        let (mut engine, mut session) = session_with(bytes);
        let reply = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "edit-targets", "page": 0}),
        );
        let preview = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "edit-replace-preview", "page": 0, "find": "a", "replace": "bb"}),
        );
        assert!(preview["ok"].is_boolean());
        let Some(items) = reply["items"].as_array() else {
            continue;
        };
        for item in items.iter().take(4) {
            for fields in [
                json!({"text": "zz"}),
                json!({"move": {"dx": 3, "dy": 4}}),
                json!({"size": 30}),
            ] {
                let revision = session.revision;
                let mut request = json!({"op": "edit-text", "page": 0, "targetId": item["targetId"],
                    "generation": reply["generation"], "revision": revision});
                if let (Value::Object(r), Value::Object(f)) = (&mut request, fields) {
                    r.extend(f);
                }
                let answer = dispatch(&mut engine, &mut session, request);
                assert!(answer["ok"].is_boolean());
            }
        }
        for paragraph in reply["paragraphs"].as_array().into_iter().flatten() {
            let revision = session.revision;
            let answer = dispatch(
                &mut engine,
                &mut session,
                json!({"op": "edit-paragraph", "page": 0, "paragraphId": paragraph["paragraphId"],
                    "generation": reply["generation"], "revision": revision, "text": "new text"}),
            );
            assert!(answer["ok"].is_boolean());
        }
    }
}

/// A page (object 3) with /Contents `contents`, its streams numbered from
/// 5, Helvetica as /F1 (4) and /Fm1 (object `form`) in its resources.
fn page_of(page_extra: &str, contents: &str, form: u32, rest: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] {page_extra} /Resources << /Font << /F1 4 0 R >> /XObject << /Fm1 {form} 0 R >> >> /Contents {contents} >>"
        )
        .into_bytes(),
        HELVETICA.to_vec(),
    ];
    objects.extend_from_slice(rest);
    objects
}

const FORM_HELLO: &str =
    "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >>";

/// Review: a BDC's /ActualText in one /Contents element was spliced into
/// a run's stream in a later element at the tag's own offsets.
#[test]
fn an_actual_text_opened_in_another_element_is_not_rewritten() {
    let bytes = classic(&page_of(
        "",
        "[5 0 R 6 0 R]",
        7,
        &[
            stream("", b"/Span<</ActualText(Hello)>>BDC"),
            stream(
                "",
                b"q Q q Q q Q q Q BT /F1 12 Tf 72 700 Td (Hello) Tj ET EMC",
            ),
        ],
    ));
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    let reply = scan(&mut engine, &mut session);
    let hello = target(&reply, "Hello");
    assert_eq!(hello["support"], "unsupported");
    assert_eq!(hello["reason"], "ambiguous-text-association");
    let refused = edit(
        &mut engine,
        &mut session,
        &reply,
        "Hello",
        json!({"text": "Jello"}),
    );
    assert_eq!(refused["ok"], false, "{refused}");
    let preview = op(
        &mut engine,
        &mut session,
        json!({"op": "edit-replace-preview", "page": 0, "find": "Hello", "replace": "Jello"}),
    );
    assert_eq!(preview["matches"], json!([]));
    assert_eq!(
        preview["rejected"][0]["detail"],
        "ambiguous-text-association"
    );
    assert_eq!(serialized(&session), before);

    // The same tag in the run's own element is rewritten with it.
    let bytes = classic(&page_of(
        "",
        "[5 0 R 6 0 R]",
        7,
        &[
            stream("", b"q Q"),
            stream(
                "",
                b"/Span<</ActualText(Hello)>>BDC BT /F1 12 Tf 72 700 Td (Hello) Tj ET EMC",
            ),
        ],
    ));
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    let reply = scan(&mut engine, &mut session);
    let edited = edit(
        &mut engine,
        &mut session,
        &reply,
        "Hello",
        json!({"text": "Jello"}),
    );
    assert_eq!(edited["ok"], true, "{edited}");
    assert_eq!(
        decoded(&session, id(6)),
        "/Span<</ActualText(Jello)>>BDC BT /F1 12 Tf 72 700 Td (Jello) Tj ET EMC"
    );
    assert_eq!(decoded(&session, id(5)), "q Q");
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
}

/// Review: a run in a Form drawn under a page-level ActualText (inline,
/// or its MCID's StructElem) was a candidate, so the tag went stale.
#[test]
fn a_form_run_under_an_enclosing_actual_text_is_not_a_candidate() {
    let form = stream(FORM_HELLO, b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET");
    let run = |content: &str, catalog: &str, extra: &[Vec<u8>]| {
        let mut objects = page_of(
            "/StructParents 0",
            "5 0 R",
            6,
            &[stream("", content.as_bytes()), form.clone()],
        );
        objects[0] = format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>").into_bytes();
        objects.extend_from_slice(extra);
        let (mut engine, mut session) = session_with(classic(&objects));
        target(&scan(&mut engine, &mut session), "Hello").clone()
    };
    let plain = run("q 1 0 0 1 0 0 cm /Fm1 Do Q", "", &[]);
    assert_eq!(plain["support"], "candidate", "{plain}");
    let inline = run(
        "/Span<</ActualText(Hello)>>BDC q 1 0 0 1 0 0 cm /Fm1 Do Q EMC",
        "",
        &[],
    );
    assert_eq!(inline["reason"], "ambiguous-text-association", "{inline}");
    let tree = |element: &str| {
        vec![
            b"<< /Type /StructTreeRoot /ParentTree 8 0 R >>".to_vec(),
            b"<< /Nums [0 [9 0 R]] >>".to_vec(),
            element.as_bytes().to_vec(),
        ]
    };
    let tagged = run(
        "/P <</MCID 0>> BDC q 1 0 0 1 0 0 cm /Fm1 Do Q EMC",
        "/StructTreeRoot 7 0 R",
        &tree("<< /Type /StructElem /S /P /ActualText (Hello) >>"),
    );
    assert_eq!(tagged["reason"], "ambiguous-text-association", "{tagged}");
    let untagged = run(
        "/P <</MCID 0>> BDC q 1 0 0 1 0 0 cm /Fm1 Do Q EMC",
        "/StructTreeRoot 7 0 R",
        &tree("<< /Type /StructElem /S /P >>"),
    );
    assert_eq!(untagged["support"], "candidate", "{untagged}");
}

/// The ParentTree is a number tree: /Kids within /Limits are followed and
/// the StructElem's /ActualText is rewritten; a tree whose kids repeat is
/// walked once per node (it was exponential).
#[test]
fn parent_tree_kids_are_followed_once_each() {
    let tagged = |tree: Vec<Vec<u8>>| {
        let mut objects = page_of(
            "/StructParents 0",
            "5 0 R",
            6,
            &[
                stream(
                    "",
                    b"/P <</MCID 0>> BDC BT /F1 12 Tf 72 700 Td (Hello) Tj ET EMC",
                ),
                b"<< >>".to_vec(),
                b"<< /Type /StructTreeRoot /ParentTree 8 0 R >>".to_vec(),
            ],
        );
        objects[0] = b"<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 7 0 R >>".to_vec();
        objects.extend(tree);
        session_with(classic(&objects))
    };
    let (mut engine, mut session) = tagged(vec![
        b"<< /Kids [9 0 R 10 0 R] >>".to_vec(),
        b"<< /Limits [5 9] /Nums [5 []] >>".to_vec(),
        b"<< /Limits [0 3] /Kids [11 0 R] >>".to_vec(),
        b"<< /Limits [0 0] /Nums [0 [12 0 R]] >>".to_vec(),
        b"<< /Type /StructElem /S /P /ActualText (Hello) >>".to_vec(),
    ]);
    let before = serialized(&session);
    let reply = scan(&mut engine, &mut session);
    let edited = edit(
        &mut engine,
        &mut session,
        &reply,
        "Hello",
        json!({"text": "Jello"}),
    );
    assert_eq!(edited["ok"], true, "{edited}");
    let element = document(&session).resolve_id(id(12)).as_dict().unwrap();
    assert_eq!(
        element.get(b"ActualText".as_slice()),
        Some(&Object::String(b"Jello".to_vec()))
    );
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);

    // Forty levels of `<< /Kids [n 0 R n 0 R] >>`: 2^33 visits before.
    let mut tree: Vec<Vec<u8>> = (0..40)
        .map(|level| format!("<< /Kids [{0} 0 R {0} 0 R] >>", 9 + level).into_bytes())
        .collect();
    tree.push(b"<< /Nums [] >>".to_vec());
    let (mut engine, mut session) = tagged(tree);
    let reply = scan(&mut engine, &mut session);
    assert_eq!(target(&reply, "Hello")["support"], "candidate");
}

/// Review: `aw ac string "` sets Tw and Tc for the runs after it.
#[test]
fn a_double_quote_show_sets_the_spacing_later_runs_are_measured_with() {
    let bytes = one_page(
        "BT /F1 12 Tf 72 700 Td 0 1 () \" (AB) Tj ET BT 0 Tc /F1 12 Tf 158 688 Td (WALL) Tj ET",
        &[],
    );
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    let reply = scan(&mut engine, &mut session);
    let preview = op(
        &mut engine,
        &mut session,
        json!({"op": "edit-replace-preview", "page": 0, "find": "AB", "replace": "ABABABABAB",
            "generation": reply["generation"], "revision": reply["revision"]}),
    );
    // (667 + 667) / 1000 x 12 + 2 x 1 pt of Tc; the wall is at x 158.
    let rejected = &preview["rejected"][0];
    assert_eq!(rejected["oldWidth"], 18.01, "{preview}");
    assert_eq!(rejected["newWidth"], 90.04, "{preview}");
    assert_eq!(rejected["detail"], "overflow", "{preview}");
    assert_eq!(rejected["limit"], 157.0, "{preview}");
    let wide = edit(
        &mut engine,
        &mut session,
        &reply,
        "AB",
        json!({"text": "ABABABABAB"}),
    );
    assert_eq!(wide["detail"], "overflow", "{wide}");
    assert_eq!(serialized(&session), before);
}

/// Review: each element's copy-on-write is charged from the scan's bytes
/// (the earlier app engine), so an apply the preview accepts does not fail on the
/// second shared element.
#[test]
fn copy_on_write_is_charged_per_element() {
    let content = |y: u32| {
        let mut data = format!("BT /F1 12 Tf 72 {y} Td (cat) Tj ET\n%").into_bytes();
        data.extend(std::iter::repeat_n(b'x', 550_000));
        stream("", &data)
    };
    let bytes = classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents [6 0 R 7 0 R] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents [6 0 R 7 0 R] >>".to_vec(),
        HELVETICA.to_vec(),
        content(700),
        content(600),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    let reply = scan(&mut engine, &mut session);
    let preview = op(
        &mut engine,
        &mut session,
        json!({"op": "edit-replace-preview", "page": 0, "find": "cat", "replace": "cot",
            "generation": reply["generation"], "revision": reply["revision"]}),
    );
    assert_eq!(preview["matches"].as_array().unwrap().len(), 2);
    let applied = op(
        &mut engine,
        &mut session,
        json!({"op": "edit-replace-apply", "page": 0, "find": "cat", "replace": "cot",
            "generation": preview["generation"], "revision": preview["revision"],
            "previewToken": preview["previewToken"],
            "targetIds": preview["matches"].as_array().unwrap().iter()
                .map(|m| m["targetId"].clone()).collect::<Vec<_>>()}),
    );
    assert_eq!(applied["replaced"], 2);
    assert!(decoded(&session, id(6)).contains("(cat)"));
    assert!(page_content(&session).contains("(cot) Tj"));
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
}

/// A preview asked for another revision than the session's refuses
/// (the earlier app engine ignored both numbers).
#[test]
fn a_stale_replace_preview_refuses() {
    let bytes = one_page("BT /F1 12 Tf 72 700 Td (ALPHA) Tj ET\n", &[]);
    let (mut engine, mut session) = session_with(bytes);
    let reply = scan(&mut engine, &mut session);
    let ask = |generation: &Value, revision: &Value| {
        json!({"op": "edit-replace-preview", "page": 0, "find": "ALPHA", "replace": "BETA",
            "generation": generation, "revision": revision})
    };
    let stale = dispatch(
        &mut engine,
        &mut session,
        ask(&reply["generation"], &json!(7)),
    );
    assert_eq!(stale["detail"], "stale-target", "{stale}");
    let stale = dispatch(
        &mut engine,
        &mut session,
        ask(&json!(99), &reply["revision"]),
    );
    assert_eq!(stale["detail"], "stale-target", "{stale}");
    let fresh = op(
        &mut engine,
        &mut session,
        ask(&reply["generation"], &reply["revision"]),
    );
    assert_eq!(fresh["matches"].as_array().unwrap().len(), 1);
}
