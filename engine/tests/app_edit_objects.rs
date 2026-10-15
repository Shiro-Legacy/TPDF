// SPDX-License-Identifier: AGPL-3.0-or-later
//! Placement edits, the clipboard and edit-batch on the app session: every
//! write is one journal step that undo and redo restore byte for byte, and
//! every refusal comes before the step, writing nothing.
use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession, Batch};
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

const PAGE0: &str = "q 100 0 0 50 72 600 cm /Im1 Do Q\n\
q 60 0 0 40 400 700 cm /Im1 Do Q\n\
q 1 0 0 1 72 200 cm /Fm1 Do Q\n\
BT /F1 14 Tf 72 400 Td (Move me) Tj ET\n";

/// Two pages: page 0 (object 3) draws an image twice, a Form and a run;
/// page 1 (object 8) one run. `form` is the Form's dictionary entries.
fn objects_pdf(form: &str) -> Vec<u8> {
    classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 8 0 R] /Count 2 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> /XObject << /Im1 6 0 R /Fm1 7 0 R >> >> /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        stream("", PAGE0.as_bytes()),
        stream(
            "/Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
            b"\x00\xff",
        ),
        stream(
            &format!("/Type /XObject /Subtype /Form {form} /Resources << /Font << /F1 4 0 R >> >>"),
            b"BT /F1 12 Tf 2 5 Td (FORM) Tj ET",
        ),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 9 0 R >>".to_vec(),
        stream("", b"BT /F1 12 Tf 72 700 Td (Other) Tj ET\n"),
    ])
}

fn session_with(bytes: Vec<u8>) -> (AppEngine, AppSession) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    open(&mut engine, &mut session, bytes);
    (engine, session)
}

fn open(engine: &mut AppEngine, session: &mut AppSession, bytes: Vec<u8>) {
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
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

/// The decoded content of a page's /Contents, joined.
fn page_content(session: &AppSession, page: u32) -> String {
    let doc = document(session);
    let page = doc.resolve_id(id(page)).as_dict().unwrap();
    let contents = page.get(b"Contents".as_slice()).unwrap();
    let ids: Vec<ObjectId> = match doc.resolve(contents) {
        Object::Array(items) => items.iter().filter_map(Object::as_reference).collect(),
        _ => vec![contents.as_reference().unwrap()],
    };
    ids.iter()
        .map(|&i| {
            let bytes = doc.decoded_stream(i, StreamLimits::default()).unwrap();
            String::from_utf8_lossy(&bytes).into_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn scan(engine: &mut AppEngine, session: &mut AppSession, page: u32) -> Value {
    op(engine, session, json!({"op": "edit-targets", "page": page}))
}

/// The `nth` non-text item named `name`.
fn object<'r>(reply: &'r Value, name: &str, nth: usize) -> &'r Value {
    reply["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] != "text" && item["objectName"] == name)
        .nth(nth)
        .unwrap_or_else(|| panic!("no {name} #{nth} in {reply}"))
}

fn text<'r>(reply: &'r Value, text: &str) -> &'r Value {
    reply["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["text"] == text)
        .unwrap_or_else(|| panic!("no {text:?} in {reply}"))
}

fn position(session: &AppSession) -> i64 {
    session.doc.as_ref().unwrap().position()
}

/// A gray PNG of `width` x `height`.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![128; (width * height) as usize])
            .unwrap();
    }
    out
}

/// A 4 x 2 JPEG header whose EXIF orientation is `orientation`.
fn jpeg(orientation: u8) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x22];
    bytes.extend_from_slice(b"Exif\0\0MM\0*\0\0\0\x08\0\x01");
    bytes.extend_from_slice(&[0x01, 0x12, 0x00, 0x03, 0, 0, 0, 1, 0, orientation, 0, 0]);
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes.extend_from_slice(&[
        0xFF, 0xC0, 0x00, 0x0B, 8, 0, 2, 0, 4, 1, 1, 0x11, 0, 0xFF, 0xD9,
    ]);
    bytes
}

fn edit_object(
    engine: &mut AppEngine,
    session: &mut AppSession,
    reply: &Value,
    target: &Value,
    fields: Value,
    buffers: Vec<Vec<u8>>,
) -> Value {
    let mut request = json!({"op": "edit-object", "page": reply["page"],
        "targetId": target["targetId"], "generation": reply["generation"],
        "revision": reply["revision"]});
    if let (Value::Object(request), Value::Object(fields)) = (&mut request, fields) {
        request.extend(fields);
    }
    engine.dispatch(session, request, buffers).0
}

/// Runs `edit`, then proves undo and redo restore the exact bytes.
fn lands(
    session_engine: (&mut AppEngine, &mut AppSession),
    edit: impl FnOnce(&mut AppEngine, &mut AppSession) -> Value,
) -> Value {
    let (engine, session) = session_engine;
    let before = serialized(session);
    let reply = edit(engine, session);
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["dirty"], true);
    let after = serialized(session);
    assert_ne!(before, after);
    op(engine, session, json!({"op": "undo"}));
    assert_eq!(serialized(session), before, "undo restores every byte");
    op(engine, session, json!({"op": "redo"}));
    assert_eq!(serialized(session), after, "redo restores every byte");
    op(engine, session, json!({"op": "undo"}));
    reply
}

/// An object name and occurrence, the request fields, its buffers and a
/// piece of the content it writes.
type Case = (&'static str, usize, Value, Vec<Vec<u8>>, &'static str);

#[test]
fn every_object_action_is_one_step_undo_and_redo_restore() {
    let (mut engine, mut session) = session_with(objects_pdf("/BBox [0 0 100 20]"));
    let cases: [Case; 8] = [
        (
            "Im1",
            0,
            json!({"action": "move", "box": [80, 600, 180, 650]}),
            vec![],
            "100 0 0 50 80 600 cm",
        ),
        (
            "Im1",
            0,
            json!({"action": "resize", "box": [72, 600, 272, 700]}),
            vec![],
            "200 0 0 100 72 600 cm",
        ),
        ("Im1", 0, json!({"action": "delete"}), vec![], ""),
        (
            "Im1",
            1,
            json!({"action": "duplicate", "box": [410, 710, 470, 750]}),
            vec![],
            "Q\nq 60 0 0 40 410 710 cm /Im1 Do Q",
        ),
        (
            "Im1",
            0,
            json!({"action": "transform", "transform": "rotate-cw"}),
            vec![],
            "0 -100 50 0 97 675 cm",
        ),
        (
            "Im1",
            0,
            json!({"action": "crop", "box": [92, 610, 152, 640]}),
            vec![],
            "q\n92 610 60 30 re W n 100 0 0 50 72 600 cm",
        ),
        (
            "Im1",
            0,
            json!({"action": "replace", "image": {"name": "a.png", "bytes": {"$buffer": 0}}}),
            vec![png(3, 1)],
            "100 0 0 33.3333 72 600 cm /Im1r1 Do Q",
        ),
        (
            "Fm1",
            0,
            json!({"action": "move", "box": [100, 220, 200, 240]}),
            vec![],
            "1 0 0 1 100 220 cm /Fm1 Do Q",
        ),
    ];
    for (name, nth, fields, buffers, expect) in cases {
        let reply = scan(&mut engine, &mut session, 0);
        let target = object(&reply, name, nth).clone();
        let action = fields["action"].as_str().unwrap().to_string();
        let before = page_content(&session, 3);
        let edited = lands((&mut engine, &mut session), |engine, session| {
            let edited = edit_object(engine, session, &reply, &target, fields, buffers);
            if edited["ok"] == true {
                let content = page_content(session, 3);
                if expect.is_empty() {
                    assert!(!content.contains("100 0 0 50 72 600 cm"), "{content}");
                } else {
                    assert!(content.contains(expect), "{action}: {content}");
                }
                assert_eq!(session.doc.as_ref().unwrap().steps(), ["edit object"]);
            }
            edited
        });
        assert_eq!(edited["revision"], reply["revision"].as_u64().unwrap() + 1);
        assert_eq!(page_content(&session, 3), before);
    }
}

#[test]
fn a_replaced_photo_is_turned_as_its_exif_orientation_says() {
    let (mut engine, mut session) = session_with(objects_pdf("/BBox [0 0 100 20]"));
    let reply = scan(&mut engine, &mut session, 0);
    let target = object(&reply, "Im1", 0).clone();
    let edited = edit_object(
        &mut engine,
        &mut session,
        &reply,
        &target,
        json!({"action": "replace", "image": {"name": "photo.jpg", "bytes": {"$buffer": 0}}}),
        vec![jpeg(6)],
    );
    assert_eq!(edited["ok"], true, "{edited}");
    // 4 x 2 stored, 2 x 4 shown: refitted into the 100 x 50 box's height.
    assert!(page_content(&session, 3).contains("0 -50 25 0 72 650 cm /Im1r1 Do Q"));
    let after = scan(&mut engine, &mut session, 0);
    let placed = object(&after, "Im1r1", 0);
    assert_eq!(
        placed["placement"]["box"],
        json!([72.0, 600.0, 97.0, 650.0])
    );
    assert_eq!(placed["placement"]["orient"]["swap"], true);
}

#[test]
fn edit_objects_writes_every_element_in_one_step() {
    let (mut engine, mut session) = session_with(objects_pdf("/BBox [0 0 100 20]"));
    let reply = scan(&mut engine, &mut session, 0);
    let items = json!([
        {"targetId": object(&reply, "Im1", 0)["targetId"], "action": "move", "box": [80, 600, 180, 650]},
        {"targetId": object(&reply, "Im1", 1)["targetId"], "action": "duplicate", "box": [410, 710, 470, 750]},
        {"targetId": object(&reply, "Fm1", 0)["targetId"], "action": "delete"},
        {"targetId": text(&reply, "Move me")["targetId"], "action": "delete"},
    ]);
    let refused = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "edit-objects", "page": 0, "generation": reply["generation"],
            "revision": reply["revision"], "items": items}),
    );
    assert_eq!(refused["detail"], "not-candidate", "{refused}");
    assert_eq!(refused["targetId"], text(&reply, "Move me")["targetId"]);
    assert_eq!(position(&session), 0);
    let items = json!(items.as_array().unwrap()[..3]);
    let edited = lands((&mut engine, &mut session), |engine, session| {
        let edited = op(
            engine,
            session,
            json!({"op": "edit-objects", "page": 0, "generation": reply["generation"],
                "revision": reply["revision"], "items": items}),
        );
        assert_eq!(edited["count"], 3);
        let content = page_content(session, 3);
        assert!(content.contains("100 0 0 50 80 600 cm"), "{content}");
        assert!(content.contains("60 0 0 40 410 710 cm"), "{content}");
        assert!(!content.contains("/Fm1 Do"), "{content}");
        assert_eq!(session.doc.as_ref().unwrap().steps(), ["edit objects"]);
        edited
    });
    assert_eq!(edited["revision"], 1);
}

#[test]
fn placement_refusals_write_nothing() {
    let (mut engine, mut session) = session_with(objects_pdf("/BBox [0 0 100 20]"));
    let before = serialized(&session);
    let reply = scan(&mut engine, &mut session, 0);
    let im1 = object(&reply, "Im1", 0).clone();
    let fm1 = object(&reply, "Fm1", 0).clone();
    let run = text(&reply, "Move me").clone();
    let big = vec![0u8; 4 * 1024 * 1024 + 1];
    let cases: Vec<(&Value, Value, Vec<Vec<u8>>, &str)> = vec![
        (
            &im1,
            json!({"action": "move", "box": [72, 600, 72, 650]}),
            vec![],
            "out-of-bounds",
        ),
        (
            &im1,
            json!({"action": "move", "box": [-1, 600, 99, 650]}),
            vec![],
            "out-of-bounds",
        ),
        (
            &im1,
            json!({"action": "move", "box": [0, 0, 20000, 650]}),
            vec![],
            "too-large",
        ),
        (
            &im1,
            json!({"action": "move", "box": "x"}),
            vec![],
            "out-of-bounds",
        ),
        (
            &im1,
            json!({"action": "move", "box": [1, 2, 3, {"$number": "NaN"}]}),
            vec![],
            "out-of-bounds",
        ),
        (
            &im1,
            json!({"action": "transform", "transform": "spin"}),
            vec![],
            "invalid-transform",
        ),
        (
            &im1,
            json!({"action": "crop", "box": [60, 600, 172, 650]}),
            vec![],
            "out-of-bounds",
        ),
        (
            &im1,
            json!({"action": "duplicate"}),
            vec![],
            "out-of-bounds",
        ),
        (&im1, json!({"action": "teleport"}), vec![], "not-candidate"),
        (&im1, json!({"action": 7}), vec![], "not-candidate"),
        (&fm1, json!({"action": "replace"}), vec![], "not-candidate"),
        (
            &run,
            json!({"action": "move", "box": [0, 0, 10, 10]}),
            vec![],
            "not-candidate",
        ),
        (
            &im1,
            json!({"action": "replace", "image": {"name": "x.png", "bytes": {"$buffer": 0}}}),
            vec![b"not an image".to_vec()],
            "invalid-image",
        ),
        (
            &im1,
            json!({"action": "replace", "image": {"name": "", "bytes": {"$buffer": 0}}}),
            vec![png(1, 1)],
            "invalid-image",
        ),
        (
            &im1,
            json!({"action": "replace", "image": {"name": "x.png"}}),
            vec![],
            "invalid-image",
        ),
        (
            &im1,
            json!({"action": "replace", "image": {"name": "x.png", "bytes": {"$buffer": 0}}}),
            vec![big],
            "too-large",
        ),
    ];
    for (target, fields, buffers, detail) in cases {
        let refused = edit_object(
            &mut engine,
            &mut session,
            &reply,
            target,
            fields.clone(),
            buffers,
        );
        assert_eq!(refused["ok"], false, "{fields}");
        assert_eq!(refused["detail"], detail, "{fields}: {refused}");
        assert_eq!(position(&session), 0);
    }
    // A moved-to-the-same-box move is a no-op: no step, still clean.
    let same = edit_object(
        &mut engine,
        &mut session,
        &reply,
        &im1,
        json!({"action": "move", "box": [72, 600, 172, 650]}),
        vec![],
    );
    assert_eq!(
        (
            same["ok"].clone(),
            same["noop"].clone(),
            same["dirty"].clone()
        ),
        (json!(true), json!(true), json!(false))
    );
    // A stale revision refuses before the reason.
    let stale = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "edit-object", "page": 0, "targetId": im1["targetId"], "generation": reply["generation"],
            "revision": 9, "action": "move", "box": [80, 600, 180, 650]}),
    );
    assert_eq!(stale["detail"], "stale-target");
    for items in [
        json!([]),
        json!("x"),
        json!([{"targetId": "a", "action": "delete"}]),
        json!([{"targetId": "a", "action": "crop"}, {"targetId": "b", "action": "delete"}]),
        json!([{"targetId": "a", "action": "delete"}, {"targetId": "a", "action": "delete"}]),
    ] {
        let refused = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "edit-objects", "page": 0, "generation": reply["generation"],
                "revision": reply["revision"], "items": items}),
        );
        assert_eq!(refused["detail"], "invalid-selection", "{items}: {refused}");
    }
    assert_eq!(serialized(&session), before);
    assert_eq!(position(&session), 0);
}

#[test]
fn a_placement_without_area_refuses_geometry_but_deletes() {
    let (mut engine, mut session) = session_with(objects_pdf("/BBox [0 0 0 20]"));
    let reply = scan(&mut engine, &mut session, 0);
    let fm1 = object(&reply, "Fm1", 0).clone();
    for fields in [
        json!({"action": "move", "box": [100, 220, 200, 240]}),
        json!({"action": "transform", "transform": "flip-h"}),
        json!({"action": "duplicate", "box": [100, 220, 200, 240]}),
    ] {
        let refused = edit_object(&mut engine, &mut session, &reply, &fm1, fields, vec![]);
        assert_eq!(refused["detail"], "unsupported-transform", "{refused}");
    }
    assert_eq!(position(&session), 0);
    let deleted = edit_object(
        &mut engine,
        &mut session,
        &reply,
        &fm1,
        json!({"action": "delete"}),
        vec![],
    );
    assert_eq!(deleted["ok"], true, "{deleted}");
    assert!(!page_content(&session, 3).contains("Infinity"));
}

fn copy(engine: &mut AppEngine, session: &mut AppSession, page: u32, targets: Value) -> Value {
    let reply = scan(engine, session, page);
    dispatch(
        engine,
        session,
        json!({"op": "clip-copy", "page": page, "generation": reply["generation"],
            "revision": reply["revision"], "targets": targets}),
    )
}

/// One page drawing `content` with Helvetica as /F1; `extra` objects from 6.
fn plain_page(content: &str, extra: &[Vec<u8>]) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".to_vec(),
        stream("", content.as_bytes()),
    ];
    objects.extend_from_slice(extra);
    classic(&objects)
}

#[test]
fn the_clipboard_crosses_documents_and_sessions_and_pastes_in_one_step() {
    let (mut engine, mut source) = session_with(objects_pdf("/BBox [0 0 100 20]"));
    let reply = scan(&mut engine, &mut source, 0);
    let copied = copy(
        &mut engine,
        &mut source,
        0,
        json!([{"kind": "image", "id": object(&reply, "Im1", 0)["targetId"]},
            {"kind": "form", "id": object(&reply, "Fm1", 0)["targetId"]},
            {"kind": "text", "id": text(&reply, "Move me")["targetId"]}]),
    );
    assert_eq!(copied["ok"], true, "{copied}");
    assert_eq!(copied["count"], 3);
    assert_eq!(
        copied["kinds"],
        json!({"image": 1, "form": 1, "text": 1, "field": 0, "link": 0, "mark": 0})
    );
    assert_eq!(copied["revision"], 0, "a copy writes nothing");
    assert_eq!(position(&source), 0);
    // Another session of the engine takes a copy of the clipboard, as the
    // test harness's `clipboardFrom` does; a new document keeps it.
    let mut target = AppSession {
        clipboard: source.clipboard.clone(),
        ..AppSession::default()
    };
    open(
        &mut engine,
        &mut target,
        plain_page("BT /F1 12 Tf 72 700 Td (Hi) Tj ET\n", &[]),
    );
    let pasted = lands((&mut engine, &mut target), |engine, session| {
        let pasted = op(engine, session, json!({"op": "clip-paste", "page": 0}));
        assert_eq!(pasted["names"], json!(["PA1", "PA2"]));
        assert_eq!(pasted["count"], 3);
        assert_eq!(pasted["targetIds"].as_array().unwrap().len(), 1);
        let content = page_content(session, 3);
        assert!(
            content.contains("100 0 0 50 72 600 cm\n/PA1 Do"),
            "{content}"
        );
        assert!(content.contains("/PF1 14 Tf"), "{content}");
        let doc = document(session);
        let page = doc.resolve_id(id(3)).as_dict().unwrap();
        let resources = doc
            .resolve(page.get(b"Resources".as_slice()).unwrap())
            .as_dict()
            .unwrap();
        let xobjects = doc
            .resolve(resources.get(b"XObject".as_slice()).unwrap())
            .as_dict()
            .unwrap();
        let image = doc
            .resolve(xobjects.get(b"PA1".as_slice()).unwrap())
            .as_dict()
            .unwrap();
        assert_eq!(image.get(b"Width".as_slice()), Some(&Object::Integer(2)));
        let fonts = doc
            .resolve(resources.get(b"Font".as_slice()).unwrap())
            .as_dict()
            .unwrap();
        let font = doc
            .resolve(fonts.get(b"PF1".as_slice()).unwrap())
            .as_dict()
            .unwrap();
        assert_eq!(
            font.get(b"BaseFont".as_slice()),
            Some(&Object::Name(b"Helvetica".to_vec()))
        );
        // The Form's own font came along, a copy shared with PF1.
        let form = doc
            .resolve(xobjects.get(b"PA2".as_slice()).unwrap())
            .as_dict()
            .unwrap();
        let form_fonts = doc
            .resolve(form.get(b"Resources".as_slice()).unwrap())
            .as_dict()
            .unwrap();
        let form_fonts = doc
            .resolve(form_fonts.get(b"Font".as_slice()).unwrap())
            .as_dict()
            .unwrap();
        assert_eq!(
            form_fonts.get(b"F1".as_slice()),
            fonts.get(b"PF1".as_slice())
        );
        assert_eq!(session.doc.as_ref().unwrap().steps(), ["paste"]);
        pasted
    });
    assert_eq!(pasted["revision"], 1);
    // The pasted text scans as a candidate on the new element.
    op(&mut engine, &mut target, json!({"op": "redo"}));
    let after = scan(&mut engine, &mut target, 0);
    let moved = text(&after, "Move me");
    assert_eq!(moved["support"], "candidate");
    assert!(moved["targetId"].as_str().unwrap().starts_with("p0.c1."));
    // A second paste steps on and takes the next names.
    let again = op(
        &mut engine,
        &mut target,
        json!({"op": "clip-paste", "page": 0}),
    );
    assert_eq!(again["names"], json!(["PA3", "PA4"]));
    assert!(page_content(&target, 3).contains("/PF2 14 Tf"));
}

#[test]
fn a_copied_object_never_drags_pages_along() {
    // The Form's resources name page 3 and the page tree; the copy nulls them.
    let (mut engine, mut session) = session_with(objects_pdf(
        "/BBox [0 0 100 20] /Extra << /P 3 0 R /Tree 2 0 R /Root 1 0 R >>",
    ));
    let reply = scan(&mut engine, &mut session, 0);
    let fm1 = object(&reply, "Fm1", 0).clone();
    let copied = copy(
        &mut engine,
        &mut session,
        0,
        json!([{"kind": "form", "id": fm1["targetId"]}]),
    );
    assert_eq!(copied["ok"], true, "{copied}");
    let objects = document(&session).object_count();
    let pasted = op(
        &mut engine,
        &mut session,
        json!({"op": "clip-paste", "page": 1}),
    );
    assert_eq!(pasted["names"], json!(["PA1"]));
    let doc = document(&session);
    // The Form, its font and its stream: nothing else was added.
    assert_eq!(doc.object_count(), objects + 3);
    let page = doc.resolve_id(id(8)).as_dict().unwrap();
    let resources = doc
        .resolve(page.get(b"Resources".as_slice()).unwrap())
        .as_dict()
        .unwrap();
    let xobjects = doc
        .resolve(resources.get(b"XObject".as_slice()).unwrap())
        .as_dict()
        .unwrap();
    let form = doc
        .resolve(xobjects.get(b"PA1".as_slice()).unwrap())
        .as_dict()
        .unwrap();
    let extra = doc
        .resolve(form.get(b"Extra".as_slice()).unwrap())
        .as_dict()
        .unwrap();
    for key in [b"P".as_slice(), b"Tree", b"Root"] {
        assert_eq!(extra.get(key), Some(&Object::Null));
    }
}

#[test]
fn clipboard_refusals_keep_the_clipboard_and_write_nothing() {
    let (mut engine, mut session) = session_with(objects_pdf("/BBox [0 0 100 20]"));
    let empty = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "clip-paste", "page": 0}),
    );
    assert_eq!(empty["detail"], "empty-clipboard");
    let reply = scan(&mut engine, &mut session, 0);
    let im1 = object(&reply, "Im1", 0)["targetId"].clone();
    for (targets, detail) in [
        (json!([]), "invalid-selection"),
        (
            json!([{"kind": "image", "id": im1}, {"kind": "form", "id": im1}]),
            "invalid-selection",
        ),
        (json!([{"kind": "image", "id": 5}]), "invalid-selection"),
        (json!([{"kind": "link", "id": "5"}]), "invalid-selection"),
        (json!([{"kind": "picture", "id": im1}]), "invalid-selection"),
        (
            json!([{"kind": "image", "id": "p0.c0.1.deadbeef"}]),
            "stale-target",
        ),
        (json!([{"kind": "form", "id": im1}]), "not-candidate"),
    ] {
        let refused = copy(&mut engine, &mut session, 0, targets.clone());
        assert_eq!(refused["detail"], detail, "{targets}: {refused}");
    }
    assert!(
        session.clipboard.is_none(),
        "a refused copy leaves no clipboard"
    );
    let copied = copy(
        &mut engine,
        &mut session,
        0,
        json!([{"kind": "image", "id": im1}]),
    );
    assert_eq!(copied["ok"], true);
    // The unknown copy keeps the old clipboard.
    assert_eq!(
        copy(
            &mut engine,
            &mut session,
            0,
            json!([{"kind": "image", "id": "x"}])
        )["detail"],
        "stale-target"
    );
    assert_eq!(session.clipboard.as_ref().map(|c| c.len()), Some(1));
    let before = serialized(&session);
    for at in [
        json!("x"),
        json!([1]),
        json!([1, {"$number": "Infinity"}]),
        json!(null),
    ] {
        let refused = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "clip-paste", "page": 0, "at": at}),
        );
        assert_eq!(refused["detail"], "out-of-bounds", "{at}");
    }
    // Unbalanced content takes no paste.
    let mut other = AppSession {
        clipboard: session.clipboard.clone(),
        ..AppSession::default()
    };
    open(
        &mut engine,
        &mut other,
        plain_page("q BT /F1 12 Tf 72 700 Td (Hi) Tj ET\n", &[]),
    );
    let unbalanced = dispatch(
        &mut engine,
        &mut other,
        json!({"op": "clip-paste", "page": 0}),
    );
    assert_eq!(unbalanced["detail"], "unbalanced-content", "{unbalanced}");
    assert_eq!(position(&other), 0);
    // A page smaller than the clipboard refuses.
    open(
        &mut engine,
        &mut other,
        classic(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 50 50] >>".to_vec(),
        ]),
    );
    let small = dispatch(
        &mut engine,
        &mut other,
        json!({"op": "clip-paste", "page": 0}),
    );
    assert_eq!(small["detail"], "out-of-bounds", "{small}");
    assert_eq!(position(&other), 0);
    assert_eq!(serialized(&session), before);
    // `at` puts the union's top-left there, clamped into the page.
    let placed = op(
        &mut engine,
        &mut session,
        json!({"op": "clip-paste", "page": 0, "at": [580, 400]}),
    );
    assert_eq!(placed["names"], json!(["PA1"]));
    assert!(
        page_content(&session, 3).contains("100 0 0 50 512 350 cm\n/PA1 Do"),
        "{}",
        page_content(&session, 3)
    );
}

fn batch(engine: &mut AppEngine, session: &mut AppSession, items: Value) -> Value {
    let reply = scan(engine, session, 0);
    dispatch(
        engine,
        session,
        json!({"op": "edit-batch", "page": 0, "generation": reply["generation"],
            "revision": reply["revision"], "items": items}),
    )
}

#[test]
fn a_batch_moves_runs_and_placements_in_one_step() {
    let (mut engine, mut session) = session_with(objects_pdf("/BBox [0 0 100 20]"));
    let reply = scan(&mut engine, &mut session, 0);
    let items = json!([
        {"op": "edit-text", "targetId": text(&reply, "Move me")["targetId"], "move": {"dx": 0, "dy": 20}},
        {"op": "edit-object", "targetId": object(&reply, "Im1", 0)["targetId"], "action": "move", "box": [82, 590, 182, 640]},
        {"op": "edit-object", "targetId": object(&reply, "Fm1", 0)["targetId"], "action": "delete"},
    ]);
    let edited = lands((&mut engine, &mut session), |engine, session| {
        let edited = batch(engine, session, items);
        assert_eq!(edited["count"], 3, "{edited}");
        let content = page_content(session, 3);
        assert!(content.contains("100 0 0 50 82 590 cm"), "{content}");
        assert!(content.contains("0 -20 Td (Move me)"), "{content}");
        assert!(!content.contains("/Fm1 Do"), "{content}");
        assert_eq!(session.doc.as_ref().unwrap().steps(), ["edit batch"]);
        edited
    });
    assert_eq!(edited["revision"], 1);
    assert_eq!(session.batch, Batch::Off);
    // Same-box moves only: nothing to do, no step.
    let reply = scan(&mut engine, &mut session, 0);
    let same = batch(
        &mut engine,
        &mut session,
        json!([
            {"op": "edit-object", "targetId": object(&reply, "Im1", 0)["targetId"], "action": "move", "box": [72, 600, 172, 650]},
            {"op": "edit-object", "targetId": object(&reply, "Im1", 1)["targetId"], "action": "move", "box": [400, 700, 460, 740]},
        ]),
    );
    assert_eq!(
        (same["count"].clone(), same["noop"].clone()),
        (json!(0), json!(true)),
        "{same}"
    );
}

#[test]
fn a_batch_refuses_before_its_step() {
    let (mut engine, mut session) = session_with(objects_pdf("/BBox [0 0 100 20]"));
    // A step to undo, so a refusal can be seen to keep the redo branch.
    let reply = scan(&mut engine, &mut session, 0);
    let im1 = object(&reply, "Im1", 0).clone();
    edit_object(
        &mut engine,
        &mut session,
        &reply,
        &im1,
        json!({"action": "delete"}),
        vec![],
    );
    op(&mut engine, &mut session, json!({"op": "undo"}));
    let before = serialized(&session);
    let reply = scan(&mut engine, &mut session, 0);
    let run = text(&reply, "Move me")["targetId"].clone();
    let im1 = object(&reply, "Im1", 0)["targetId"].clone();
    let content = json!({"op": "edit-text", "targetId": run, "move": {"dx": 1, "dy": 0}});
    let cases = [
        (json!([content]), "invalid-selection", None),
        (json!([content, content]), "invalid-selection", Some(1)),
        (
            json!([content, {"op": "edit-object", "targetId": im1, "action": "move", "box": [0, 0, 1, 1], "page": 1}]),
            "invalid-selection",
            Some(1),
        ),
        (
            json!([content, {"op": "edit-text", "targetId": im1, "move": {"dx": 1, "dy": 0}, "text": "x"}]),
            "invalid-selection",
            None,
        ),
        (
            json!([content, {"op": "edit-object", "targetId": "p0.c0.1.deadbeef", "action": "delete"}]),
            "stale-target",
            Some(1),
        ),
        (
            json!([{"op": "edit-object", "targetId": im1, "action": "delete"},
                {"op": "edit-object", "targetId": run, "action": "delete"}]),
            "not-candidate",
            Some(1),
        ),
        // Ops whose groups are not in this tree refuse as engine.ts's
        // dispatch of an unknown op would: untagged, so stale-target.
        (
            json!([content, {"op": "annot-delete", "annotId": 99}]),
            "stale-target",
            Some(1),
        ),
        (
            json!([content, {"op": "link-delete", "annotId": 99}]),
            "stale-target",
            Some(1),
        ),
        (
            json!([content, {"op": "field-props", "widgetId": 5, "rect": [0, 0, 10, 10]}]),
            "stale-target",
            Some(1),
        ),
    ];
    for (items, detail, index) in cases {
        let refused = batch(&mut engine, &mut session, items.clone());
        assert_eq!(refused["detail"], detail, "{items}: {refused}");
        if let Some(index) = index {
            assert_eq!(refused["index"], index, "{refused}");
        }
        assert_eq!(session.batch, Batch::Off);
        assert_eq!(refused["canRedo"], true, "the redo branch survives");
    }
    let many: Vec<Value> = (0..513)
        .map(|n| json!({"op": "annot-delete", "annotId": n}))
        .collect();
    assert_eq!(
        batch(&mut engine, &mut session, json!(many))["detail"],
        "too-large"
    );
    assert_eq!(serialized(&session), before);
}

/// Malformed values in every field of these ops refuse; none panics.
#[test]
fn hostile_requests_never_panic() {
    let (mut engine, mut session) = session_with(objects_pdf("/BBox [0 0 100 20]"));
    let reply = scan(&mut engine, &mut session, 0);
    let im1 = object(&reply, "Im1", 0)["targetId"].clone();
    let weird = [
        json!(null),
        json!(-1),
        json!(1e308),
        json!({"$number": "NaN"}),
        json!("x"),
        json!([]),
        json!([1e308, -1e308, 1e308, 1e308]),
        json!([0, 0, 1, 1, 1]),
        json!({"dx": 1e308, "dy": {"$number": "-Infinity"}}),
        json!([[[[[]]]]]),
    ];
    for value in &weird {
        for request in [
            json!({"op": "edit-object", "page": 0, "targetId": im1, "generation": reply["generation"],
                "revision": reply["revision"], "action": "move", "box": value}),
            json!({"op": "edit-object", "page": 0, "targetId": im1, "generation": reply["generation"],
                "revision": reply["revision"], "action": "transform", "transform": value, "box": value}),
            json!({"op": "edit-object", "page": 0, "targetId": im1, "generation": reply["generation"],
                "revision": reply["revision"], "action": "replace", "image": value}),
            json!({"op": "edit-objects", "page": value, "items": value}),
            json!({"op": "edit-objects", "page": 0, "generation": reply["generation"],
                "revision": reply["revision"], "items": [{"targetId": im1, "action": "move", "box": value},
                {"targetId": value, "action": "delete"}]}),
            json!({"op": "clip-copy", "page": 0, "targets": [{"kind": "image", "id": value}]}),
            json!({"op": "clip-copy", "page": value, "targets": value}),
            json!({"op": "clip-paste", "page": 0, "at": value}),
            json!({"op": "edit-batch", "page": 0, "items": [value, value]}),
            json!({"op": "edit-batch", "page": 0, "generation": reply["generation"],
                "revision": reply["revision"], "items": [
                {"op": "edit-text", "targetId": im1, "move": value},
                {"op": "edit-object", "targetId": im1, "action": "move", "box": value}]}),
        ] {
            let reply = dispatch(&mut engine, &mut session, request.clone());
            assert!(reply.get("ok").is_some(), "{request}");
        }
    }
}

/// The dictionary of XObject `name` on page object `page`.
fn xobject<'d>(
    session: &'d AppSession,
    page: u32,
    name: &str,
) -> &'d tandem_pdf_engine::object::Dictionary {
    let doc = document(session);
    let page = doc.resolve_id(id(page)).as_dict().unwrap();
    let resources = doc
        .resolve(page.get(b"Resources".as_slice()).unwrap())
        .as_dict()
        .unwrap();
    let xobjects = doc
        .resolve(resources.get(b"XObject".as_slice()).unwrap())
        .as_dict()
        .unwrap();
    doc.resolve(xobjects.get(name.as_bytes()).unwrap())
        .as_dict()
        .unwrap()
}

/// Replaces the first placement of `name` on page 0 with `bytes`.
fn replace(engine: &mut AppEngine, session: &mut AppSession, name: &str, bytes: Vec<u8>) -> Value {
    let reply = scan(engine, session, 0);
    let target = object(&reply, name, 0).clone();
    edit_object(
        engine,
        session,
        &reply,
        &target,
        json!({"action": "replace", "image": {"name": "picture", "bytes": {"$buffer": 0}}}),
        vec![bytes],
    )
}

#[test]
fn a_replace_takes_the_turn_of_the_one_before_back_out() {
    let key = b"TPDF_Orientation".as_slice();
    let (mut engine, mut session) = session_with(objects_pdf("/BBox [0 0 100 20]"));
    let turned = replace(&mut engine, &mut session, "Im1", jpeg(6));
    assert_eq!(turned["ok"], true, "{turned}");
    assert!(page_content(&session, 3).contains("0 -50 25 0 72 650 cm /Im1r1 Do Q"));
    assert_eq!(
        xobject(&session, 3, "Im1r1").get(key),
        Some(&Object::Integer(6))
    );
    // An upright picture next: the photo's turn goes and the box is
    // refitted as an upright one is (anchored at its bottom-left).
    lands((&mut engine, &mut session), |engine, session| {
        let upright = replace(engine, session, "Im1r1", png(3, 1));
        let content = page_content(session, 3);
        assert!(
            content.contains("25 0 0 8.3333 72 600 cm /Im1r1r1 Do Q"),
            "{content}"
        );
        assert_eq!(xobject(session, 3, "Im1r1r1").get(key), None);
        upright
    });
    // A turn the user made stays: rotate the photo, then replace it.
    let reply = scan(&mut engine, &mut session, 0);
    let photo = object(&reply, "Im1r1", 0).clone();
    let rotated = edit_object(
        &mut engine,
        &mut session,
        &reply,
        &photo,
        json!({"action": "transform", "transform": "rotate-cw"}),
        vec![],
    );
    assert_eq!(rotated["ok"], true, "{rotated}");
    assert!(page_content(&session, 3).contains("-50 0 0 -25 109.5 637.5 cm /Im1r1 Do Q"));
    let kept = replace(&mut engine, &mut session, "Im1r1", png(3, 1));
    assert_eq!(kept["ok"], true, "{kept}");
    let content = page_content(&session, 3);
    assert!(
        content.contains("0 -25 50 0 59.5 637.5 cm /Im1r1r1 Do Q"),
        "{content}"
    );
    // A photo with the same turn on a turned placement keeps its `cm`.
    let (mut engine, mut session) = session_with(objects_pdf("/BBox [0 0 100 20]"));
    replace(&mut engine, &mut session, "Im1", jpeg(6));
    let again = replace(&mut engine, &mut session, "Im1r1", jpeg(6));
    assert_eq!(again["ok"], true, "{again}");
    let content = page_content(&session, 3);
    assert!(
        content.contains("0 -50 25 0 72 650 cm /Im1r1r1 Do Q"),
        "{content}"
    );
    assert_eq!(
        xobject(&session, 3, "Im1r1r1").get(key),
        Some(&Object::Integer(6))
    );
}

/// A page drawing a Form whose /BBox is 1e-12 wide at 1 x 1 pt, and a run
/// with a 1e12 leading.
fn tiny_form_pdf() -> Vec<u8> {
    classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> /XObject << /Fm1 6 0 R >> >> /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        stream(
            "",
            b"q 1000000000000 0 0 1000000000000 72 200 cm /Fm1 Do Q\n\
BT /F1 12 Tf 1000000000000 TL 72 400 Td (Lead) Tj ET\n",
        ),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 0.000000000001 0.000000000001]",
            b"0 0 0.000000000001 0.000000000001 re f",
        ),
    ])
}

#[test]
fn numbers_past_the_written_limit_refuse_before_the_step() {
    let (mut engine, mut session) = session_with(tiny_form_pdf());
    let reply = scan(&mut engine, &mut session, 0);
    let form = object(&reply, "Fm1", 0).clone();
    let lead = text(&reply, "Lead")["targetId"].clone();
    assert_eq!(form["support"], "candidate", "{form}");
    let before = serialized(&session);
    for fields in [
        json!({"action": "move", "box": [80, 200, 180, 250]}),
        json!({"action": "resize", "box": [72, 200, 172, 300]}),
        json!({"action": "transform", "transform": "rotate-cw"}),
        json!({"action": "duplicate", "box": [300, 300, 400, 400]}),
    ] {
        let refused = edit_object(
            &mut engine,
            &mut session,
            &reply,
            &form,
            fields.clone(),
            vec![],
        );
        assert_eq!(
            refused["detail"], "unsupported-transform",
            "{fields}: {refused}"
        );
    }
    let batched = batch(
        &mut engine,
        &mut session,
        json!([
            {"op": "edit-text", "targetId": lead, "move": {"dx": 1, "dy": 0}},
            {"op": "edit-object", "targetId": form["targetId"], "action": "move", "box": [80, 200, 180, 250]},
        ]),
    );
    assert_eq!(batched["detail"], "unsupported-transform", "{batched}");
    assert_eq!(batched["index"], 1, "{batched}");
    assert_eq!(serialized(&session), before);
    assert_eq!(position(&session), 0);
    // Copied elsewhere, neither the Form nor the run (its leading) pastes.
    for (kind, target) in [("form", form["targetId"].clone()), ("text", lead)] {
        let copied = copy(
            &mut engine,
            &mut session,
            0,
            json!([{"kind": kind, "id": target}]),
        );
        assert_eq!(copied["ok"], true, "{copied}");
        let mut other = AppSession {
            clipboard: session.clipboard.clone(),
            ..AppSession::default()
        };
        open(
            &mut engine,
            &mut other,
            plain_page("BT /F1 12 Tf 72 700 Td (Hi) Tj ET\n", &[]),
        );
        let refused = dispatch(
            &mut engine,
            &mut other,
            json!({"op": "clip-paste", "page": 0}),
        );
        assert_eq!(
            refused["detail"], "unsupported-transform",
            "{kind}: {refused}"
        );
        assert_eq!(refused["kind"], kind);
        assert_eq!(position(&other), 0);
    }
    let deleted = edit_object(
        &mut engine,
        &mut session,
        &reply,
        &form,
        json!({"action": "delete"}),
        vec![],
    );
    assert_eq!(deleted["ok"], true, "{deleted}");
}

#[test]
fn a_batch_names_no_reason_for_an_unavailable_action() {
    // A doubled `cm` is not a simple placement: the image offers replace.
    let (mut engine, mut session) = session_with(classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> /XObject << /Im1 6 0 R >> >> /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        stream(
            "",
            b"q 100 0 0 50 72 600 cm 1 0 0 1 0 0 cm /Im1 Do Q\nBT /F1 12 Tf 72 400 Td (Run) Tj ET\n",
        ),
        stream(
            "/Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
            b"\x00\xff",
        ),
    ]));
    let reply = scan(&mut engine, &mut session, 0);
    let im1 = object(&reply, "Im1", 0)["targetId"].clone();
    let refused = batch(
        &mut engine,
        &mut session,
        json!([
            {"op": "edit-text", "targetId": text(&reply, "Run")["targetId"], "move": {"dx": 1, "dy": 0}},
            {"op": "edit-object", "targetId": im1, "action": "move", "box": [80, 600, 180, 650]},
        ]),
    );
    assert_eq!(refused["detail"], "unsupported-transform", "{refused}");
    assert_eq!(refused["reason"], "unsupported-transform");
    assert_eq!(
        refused["error"],
        format!(
            "edit-batch: move is not available for {}",
            im1.as_str().unwrap()
        )
    );
}

/// Every op these tests dispatch is routed now that all groups are merged;
/// a missing one is a failure, never a skipped test.
fn routed(engine: &mut AppEngine, session: &mut AppSession, op: &str) -> bool {
    let reply = dispatch(engine, session, json!({"op": op, "page": 0}));
    let missing = (reply["error"].as_str()).is_some_and(|error| error.starts_with("unknown op"));
    assert!(!missing, "{op} is not routed");
    true
}

fn listed(engine: &mut AppEngine, session: &mut AppSession, op: &str) -> Vec<Value> {
    op_items(&self::op(engine, session, json!({"op": op, "page": 0})))
}

fn op_items(reply: &Value) -> Vec<Value> {
    reply["items"].as_array().cloned().unwrap_or_default()
}

fn near(value: &Value, expected: [f64; 4]) -> bool {
    let numbers: Vec<f64> = (value.as_array().into_iter().flatten())
        .filter_map(Value::as_f64)
        .collect();
    numbers.len() == 4
        && numbers
            .iter()
            .zip(expected)
            .all(|(a, b)| (a - b).abs() <= 0.01)
}

/// Page 0 (object 3) draws Im1 and holds a text field `mx`, a URI link and
/// a Redact mark.
fn members_pdf() -> Vec<u8> {
    classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 8 0 R >> >> >> >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Im1 5 0 R >> >> /Contents 4 0 R /Annots [6 0 R 7 0 R 9 0 R] >>".to_vec(),
        stream("", b"q 100 0 0 50 72 600 cm /Im1 Do Q\n"),
        stream(
            "/Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
            b"\x00\xff",
        ),
        b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (mx) /V (hello) /Rect [72 300 272 330] /P 3 0 R /F 4 >>".to_vec(),
        b"<< /Type /Annot /Subtype /Link /Rect [72 100 200 120] /Border [0 0 0] /A << /S /URI /URI (https://example.org/) >> >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        b"<< /Type /Annot /Subtype /Redact /Rect [300 300 400 350] >>".to_vec(),
    ])
}

#[test]
fn dispatched_batch_items_join_the_one_step() {
    let (mut engine, mut session) = session_with(members_pdf());
    if !routed(&mut engine, &mut session, "fields") || !routed(&mut engine, &mut session, "links") {
        return;
    }
    let field = listed(&mut engine, &mut session, "fields")
        .into_iter()
        .find(|row| row["name"] == "mx")
        .unwrap()["id"]
        .clone();
    let link = listed(&mut engine, &mut session, "links")[0]["annotId"].clone();
    let reply = scan(&mut engine, &mut session, 0);
    let items = json!([
        {"op": "field-props", "widgetId": field, "rect": [80, 310, 280, 340]},
        {"op": "link-set", "annotId": link, "rect": [80, 110, 208, 130]},
        {"op": "edit-object", "targetId": object(&reply, "Im1", 0)["targetId"], "action": "move", "box": [82, 590, 182, 640]},
    ]);
    // The plans write nothing: a refusal at the last item leaves it all.
    let before = serialized(&session);
    let refused = batch(
        &mut engine,
        &mut session,
        json!([items[0], items[1], {"op": "field-props", "widgetId": 9999, "rect": [0, 0, 10, 10]}]),
    );
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["index"], 2, "{refused}");
    assert_eq!(serialized(&session), before);
    assert_eq!((position(&session), session.batch), (0, Batch::Off));
    let edited = lands((&mut engine, &mut session), |engine, session| {
        let edited = batch(engine, session, items);
        assert_eq!(edited["ok"], true, "{edited}");
        assert_eq!(session.doc.as_ref().unwrap().steps(), ["edit batch"]);
        assert_eq!(session.batch, Batch::Off);
        assert!(page_content(session, 3).contains("100 0 0 50 82 590 cm"));
        let fields = listed(engine, session, "fields");
        let moved = fields.iter().find(|row| row["id"] == field).unwrap();
        assert!(near(&moved["rect"], [80.0, 310.0, 280.0, 340.0]), "{moved}");
        let links = listed(engine, session, "links");
        assert!(
            near(&links[0]["rect"], [80.0, 110.0, 208.0, 130.0]),
            "{links:?}"
        );
        edited
    });
    assert_eq!(edited["revision"], 1);
}

#[test]
fn pasted_fields_and_links_join_the_paste_step() {
    let (mut engine, mut source) = session_with(members_pdf());
    if !routed(&mut engine, &mut source, "fields") || !routed(&mut engine, &mut source, "links") {
        return;
    }
    let field = listed(&mut engine, &mut source, "fields")
        .into_iter()
        .find(|row| row["name"] == "mx")
        .unwrap()["id"]
        .clone();
    let link = listed(&mut engine, &mut source, "links")[0]["annotId"].clone();
    let reply = scan(&mut engine, &mut source, 0);
    let copied = copy(
        &mut engine,
        &mut source,
        0,
        json!([{"kind": "image", "id": object(&reply, "Im1", 0)["targetId"]},
            {"kind": "field", "id": field}, {"kind": "link", "id": link}]),
    );
    assert_eq!(copied["ok"], true, "{copied}");
    let mut target = AppSession {
        clipboard: source.clipboard.clone(),
        ..AppSession::default()
    };
    open(
        &mut engine,
        &mut target,
        plain_page("BT /F1 12 Tf 72 700 Td (Hi) Tj ET\n", &[]),
    );
    let pasted = lands((&mut engine, &mut target), |engine, session| {
        let pasted = op(engine, session, json!({"op": "clip-paste", "page": 0}));
        assert_eq!(pasted["count"], 3, "{pasted}");
        assert_eq!(pasted["fields"], json!(["mx"]));
        assert_eq!(pasted["linkIds"].as_array().map(Vec::len), Some(1));
        assert_eq!(session.doc.as_ref().unwrap().steps(), ["paste"]);
        assert_eq!(session.batch, Batch::Off);
        let fields = listed(engine, session, "fields");
        assert!(fields.iter().any(|row| row["name"] == "mx"), "{fields:?}");
        assert_eq!(listed(engine, session, "links").len(), 1);
        pasted
    });
    assert_eq!(pasted["revision"], 1);
}

#[test]
fn a_pasted_mark_is_checked_before_the_step() {
    let (mut engine, mut session) = session_with(members_pdf());
    if !routed(&mut engine, &mut session, "annots") {
        return;
    }
    let mark = listed(&mut engine, &mut session, "annots")
        .into_iter()
        .find(|row| row["type"] == "Redact")
        .unwrap()["id"]
        .clone();
    let reply = scan(&mut engine, &mut session, 0);
    let im1 = object(&reply, "Im1", 0).clone();
    let copied = copy(
        &mut engine,
        &mut session,
        0,
        json!([{"kind": "image", "id": im1["targetId"]}, {"kind": "mark", "id": mark}]),
    );
    assert_eq!(copied["ok"], true, "{copied}");
    // A step to undo, so a refusal can be seen to keep the redo branch.
    edit_object(
        &mut engine,
        &mut session,
        &reply,
        &im1,
        json!({"action": "delete"}),
        vec![],
    );
    op(&mut engine, &mut session, json!({"op": "undo"}));
    let before = serialized(&session);
    let pasted = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "clip-paste", "page": 0}),
    );
    assert_eq!(session.batch, Batch::Off);
    if pasted["ok"] == true {
        assert_eq!(pasted["markIds"].as_array().map(Vec::len), Some(1));
        return;
    }
    // redact-add's own refusal comes before the step.
    assert_eq!(
        (pasted["index"].clone(), pasted["kind"].clone()),
        (json!(1), json!("mark"))
    );
    assert_eq!(pasted["canRedo"], true, "{pasted}");
    assert_eq!(serialized(&session), before);
}

#[test]
fn an_image_page_made_from_a_photo_replaces_upright() {
    let mut engine = AppEngine::new();
    let mut maker = AppSession::default();
    let (reply, outputs) = engine.dispatch(
        &mut maker,
        json!({"op": "pdf-from-images", "images": [{"name": "photo.jpg", "bytes": {"$buffer": 0}}]}),
        vec![jpeg(6)],
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let mut session = AppSession::default();
    open(&mut engine, &mut session, outputs[0].clone());
    let page = session.doc.as_ref().unwrap().page(0).unwrap().0.unwrap();
    let page = page.number();
    assert_eq!(page_content(&session, page), "q 0 -4 2 0 0 4 cm /Im0 Do Q");
    let key = b"TPDF_Orientation".as_slice();
    assert_eq!(
        xobject(&session, page, "Im0").get(key),
        Some(&Object::Integer(6))
    );
    // The 2 x 4 pt page shows the photo upright; a 1 x 2 picture fits it
    // upright too, not turned as the photo's samples were.
    let replaced = replace(&mut engine, &mut session, "Im0", png(1, 2));
    assert_eq!(replaced["ok"], true, "{replaced}");
    let content = page_content(&session, page);
    assert_eq!(content, "q 2 0 0 4 0 0 cm /Im0r1 Do Q");
}
