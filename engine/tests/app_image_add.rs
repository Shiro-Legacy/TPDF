// SPDX-License-Identifier: MIT
//! `image-add`: a picture drawn into the page as a simple placement, one
//! undoable step, upright on a turned page, and editable afterwards with
//! edit-object like any other picture.
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

/// One Letter page (object 3) with a run and a picture named Im1, turned
/// by `rotate`, drawing `content`.
fn page_pdf(rotate: i64, content: &str) -> Vec<u8> {
    page_pdf_with(&format!("/Rotate {rotate}"), content)
}

/// `page_pdf` with `keys` in the page dictionary.
fn page_pdf_with(keys: &str, content: &str) -> Vec<u8> {
    classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] {keys} /Resources << /Font << /F1 4 0 R >> /XObject << /Im1 6 0 R >> >> /Contents 5 0 R >>").into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        stream("", content.as_bytes()),
        stream(
            "/Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
            b"\x00\xff",
        ),
    ])
}

const CONTENT: &str = "q 100 0 0 50 72 600 cm /Im1 Do Q\nBT /F1 14 Tf 72 400 Td (Words) Tj ET\n";

fn session_with(bytes: Vec<u8>) -> (AppEngine, AppSession) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(&mut session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
    (engine, session)
}

fn op(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    let reply = engine.dispatch(session, request, Vec::new()).0;
    assert_eq!(reply["ok"], true, "{reply}");
    reply
}

fn add(engine: &mut AppEngine, session: &mut AppSession, fields: Value, image: Vec<u8>) -> Value {
    let mut request = json!({"op": "image-add", "page": 0,
        "image": {"name": "picture.png", "bytes": {"$buffer": 0}}});
    if let (Value::Object(request), Value::Object(fields)) = (&mut request, fields) {
        request.extend(fields);
    }
    engine.dispatch(session, request, vec![image]).0
}

fn document(session: &AppSession) -> &Document<'static> {
    session.doc.as_ref().unwrap().document()
}

fn serialized(session: &AppSession) -> Vec<u8> {
    let options = WriteOptions {
        garbage: false,
        ..WriteOptions::APP
    };
    write_document(document(session), DocumentWriteLimits::default(), options).unwrap()
}

/// The decoded /Contents of page object 3, joined.
fn page_content(session: &AppSession) -> String {
    let doc = document(session);
    let page = doc.resolve_id(id(3)).as_dict().unwrap();
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

/// The scanned picture named `name`.
fn picture(engine: &mut AppEngine, session: &mut AppSession, name: &str) -> (Value, Value) {
    let reply = op(engine, session, json!({"op": "edit-targets", "page": 0}));
    let item = reply["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] != "text" && item["objectName"] == name)
        .unwrap_or_else(|| panic!("no {name} in {reply}"))
        .clone();
    (reply, item)
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

#[test]
fn a_picture_lands_centred_at_its_natural_size_in_one_step() {
    let (mut engine, mut session) = session_with(page_pdf(0, CONTENT));
    let before = serialized(&session);
    let reply = add(&mut engine, &mut session, json!({}), png(200, 100));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["dirty"], true);
    // Im1 is taken, so the picture is Im2: 200 x 100 pt about the page centre.
    assert_eq!(reply["name"], "Im2");
    assert_eq!(reply["box"], json!([206.0, 346.0, 406.0, 446.0]));
    assert_eq!(session.doc.as_ref().unwrap().steps(), ["add image"]);
    let content = page_content(&session);
    assert!(
        content.contains("q\n200 0 0 100 206 346 cm\n/Im2 Do\nQ"),
        "{content}"
    );
    assert!(
        content.contains("(Words) Tj"),
        "the page's own content stays"
    );
    let (_, item) = picture(&mut engine, &mut session, "Im2");
    assert_eq!(item["targetId"], reply["targetId"]);
    assert_eq!(
        item["placement"]["box"],
        json!([206.0, 346.0, 406.0, 446.0])
    );
    let after = serialized(&session);
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before, "undo restores every byte");
    op(&mut engine, &mut session, json!({"op": "redo"}));
    assert_eq!(serialized(&session), after, "redo restores every byte");
}

#[test]
fn a_big_picture_is_shrunk_to_half_the_page() {
    let (mut engine, mut session) = session_with(page_pdf(0, CONTENT));
    // 1224 pt across: a quarter of that is half the page's 612.
    let reply = add(&mut engine, &mut session, json!({}), png(1224, 400));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["box"], json!([153.0, 346.0, 459.0, 446.0]));
}

#[test]
fn at_centres_it_and_the_page_edge_holds_it() {
    let (mut engine, mut session) = session_with(page_pdf(0, CONTENT));
    let reply = add(
        &mut engine,
        &mut session,
        json!({"at": [600, 780]}),
        png(200, 100),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["box"], json!([412.0, 692.0, 612.0, 792.0]));
    let reply = add(
        &mut engine,
        &mut session,
        json!({"box": [10, 20, 110, 70]}),
        png(200, 100),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["name"], "Im3");
    assert!(page_content(&session).contains("100 0 0 50 10 20 cm\n/Im3 Do"));
}

#[test]
fn refusals_write_nothing() {
    let (mut engine, mut session) = session_with(page_pdf(0, CONTENT));
    let before = serialized(&session);
    let cases = [
        (
            json!({"box": [500, 700, 700, 800]}),
            png(2, 2),
            "out-of-bounds",
        ),
        (
            json!({"box": [10, 10, 10.5, 20]}),
            png(2, 2),
            "out-of-bounds",
        ),
        (json!({"at": [1]}), png(2, 2), "invalid"),
        (json!({}), b"GIF89a....".to_vec(), "invalid-image"),
        (json!({}), Vec::new(), "invalid-image"),
    ];
    for (fields, image, detail) in cases {
        let reply = add(&mut engine, &mut session, fields.clone(), image);
        assert_eq!(reply["ok"], false, "{fields}: {reply}");
        assert_eq!(reply["detail"], detail, "{fields}: {reply}");
    }
    let unnamed = engine
        .dispatch(
            &mut session,
            json!({"op": "image-add", "page": 0, "image": {"bytes": {"$buffer": 0}}}),
            vec![png(2, 2)],
        )
        .0;
    assert_eq!(unnamed["detail"], "invalid-image", "{unnamed}");
    assert_eq!(serialized(&session), before);
    assert!(session.doc.as_ref().unwrap().steps().is_empty());
}

#[test]
fn unbalanced_content_is_refused() {
    let (mut engine, mut session) = session_with(page_pdf(
        0,
        "q 2 0 0 2 0 0 cm\nBT /F1 14 Tf 72 400 Td (Words) Tj ET\n",
    ));
    let reply = add(&mut engine, &mut session, json!({}), png(20, 10));
    assert_eq!(reply["ok"], false, "{reply}");
    assert_eq!(reply["detail"], "unbalanced-content");
}

#[test]
fn on_a_turned_page_the_picture_reads_upright_and_stays_editable() {
    let (mut engine, mut session) = session_with(page_pdf(90, CONTENT));
    let reply = add(&mut engine, &mut session, json!({}), png(200, 100));
    assert_eq!(reply["ok"], true, "{reply}");
    // Shown 200 wide and 100 high on the turned page: 100 x 200 in user
    // space, the picture's right edge running up the page.
    assert_eq!(reply["box"], json!([256.0, 296.0, 356.0, 496.0]));
    let content = page_content(&session);
    assert!(
        content.contains("0 200 -100 0 356 296 cm\n/Im2 Do"),
        "{content}"
    );
    let (_, item) = picture(&mut engine, &mut session, "Im2");
    assert_eq!(item["support"], "candidate", "{item}");
    assert_eq!(item["targetId"], reply["targetId"]);
    assert_eq!(
        item["placement"]["box"],
        json!([256.0, 296.0, 356.0, 496.0])
    );
    assert_eq!(item["placement"]["orient"]["swap"], true);
    let (reply, item) = picture(&mut engine, &mut session, "Im2");
    op(
        &mut engine,
        &mut session,
        json!({"op": "edit-object", "page": 0, "targetId": item["targetId"],
            "generation": reply["generation"], "revision": reply["revision"],
            "action": "move", "box": [100, 100, 200, 300]}),
    );
    assert!(page_content(&session).contains("0 200 -100 0 200 100 cm\n/Im2 Do"));
}

/// edit-object on the page's first picture (Im1).
fn edit_im1(engine: &mut AppEngine, session: &mut AppSession, fields: Value) -> Value {
    let (reply, item) = picture(engine, session, "Im1");
    let mut request = json!({"op": "edit-object", "page": 0, "targetId": item["targetId"],
        "generation": reply["generation"], "revision": reply["revision"]});
    if let (Value::Object(request), Value::Object(fields)) = (&mut request, fields) {
        request.extend(fields);
    }
    engine.dispatch(session, request, Vec::new()).0
}

#[test]
fn a_picture_on_a_turned_page_moves_turns_and_flips() {
    let (mut engine, mut session) = session_with(page_pdf(90, CONTENT));
    let (_, item) = picture(&mut engine, &mut session, "Im1");
    assert_eq!(item["support"], "candidate", "{item}");
    let moved = edit_im1(
        &mut engine,
        &mut session,
        json!({"action": "move", "box": [80, 600, 180, 650]}),
    );
    assert_eq!(moved["ok"], true, "{moved}");
    assert!(page_content(&session).contains("q 100 0 0 50 80 600 cm /Im1 Do Q"));
    // Tokens are in user space (the app swaps the flips on a quarter-turned
    // page): flip-h mirrors along the page's width.
    let flipped = edit_im1(
        &mut engine,
        &mut session,
        json!({"action": "transform", "transform": "flip-h"}),
    );
    assert_eq!(flipped["ok"], true, "{flipped}");
    assert!(
        page_content(&session).contains("-100 0 0 50 180 600 cm"),
        "{}",
        page_content(&session)
    );
    op(&mut engine, &mut session, json!({"op": "undo"}));
    // A turn reads the same either way.
    let turned = edit_im1(
        &mut engine,
        &mut session,
        json!({"action": "transform", "transform": "rotate-cw"}),
    );
    assert_eq!(turned["ok"], true, "{turned}");
    assert!(
        page_content(&session).contains("0 -100 50 0 105 675 cm"),
        "{}",
        page_content(&session)
    );
}

#[test]
fn a_picture_pasted_onto_a_turned_page_looks_as_it_did() {
    let (mut engine, mut source) = session_with(page_pdf(0, CONTENT));
    let (reply, item) = picture(&mut engine, &mut source, "Im1");
    let copied = op(
        &mut engine,
        &mut source,
        json!({"op": "clip-copy", "page": 0, "generation": reply["generation"],
            "revision": reply["revision"], "targets": [{"kind": "image", "id": item["targetId"]}]}),
    );
    assert_eq!(copied["count"], 1, "{copied}");
    let mut turned = AppSession {
        clipboard: source.clipboard.clone(),
        ..AppSession::default()
    };
    let request = json!({"id": 1, "op": "open", "name": "b.pdf", "pdf": {"$buffer": 0}});
    let (opened, _) = engine.dispatch(&mut turned, request, vec![page_pdf(90, CONTENT)]);
    assert_eq!(opened["ok"], true, "{opened}");
    let pasted = op(
        &mut engine,
        &mut turned,
        json!({"op": "clip-paste", "page": 0}),
    );
    let name = pasted["names"][0].as_str().unwrap().to_string();
    let (_, item) = picture(&mut engine, &mut turned, &name);
    // 100 wide and 50 high where it was copied, so 50 x 100 in the turned
    // page's user space, and turned with the page.
    let [x0, y0, x1, y1] = [0, 1, 2, 3].map(|i| item["placement"]["box"][i].as_f64().unwrap());
    assert_eq!(((x1 - x0), (y1 - y0)), (50.0, 100.0), "{item}");
    assert_eq!(item["placement"]["orient"]["swap"], true, "{item}");
    assert_eq!(item["support"], "candidate");
    // Upright as shown, as image-add draws it: its right edge runs up the
    // page (the cm's x axis is +y), its top to the page's left.
    let content = page_content(&turned);
    let block = content
        .split("\nq\n")
        .find(|b| b.contains(&format!("/{name} Do")))
        .unwrap_or_else(|| panic!("{content}"));
    let cm: Vec<f64> = block
        .split_whitespace()
        .take(4)
        .map(|v| v.parse().unwrap())
        .collect();
    assert_eq!(cm, [0.0, 100.0, -50.0, 0.0], "{block}");
}

#[test]
fn a_page_with_a_user_unit_keeps_its_pictures_locked() {
    let (mut engine, mut session) = session_with(page_pdf_with("/UserUnit 2", CONTENT));
    let (_, item) = picture(&mut engine, &mut session, "Im1");
    assert_eq!(item["support"], "unsupported", "{item}");
    let before = serialized(&session);
    let reply = add(&mut engine, &mut session, json!({}), png(20, 10));
    assert_eq!(reply["ok"], false, "{reply}");
    assert_eq!(reply["detail"], "unsupported-transform");
    assert_eq!(serialized(&session), before);
}

#[test]
fn a_photo_is_turned_as_its_exif_orientation_says() {
    let (mut engine, mut session) = session_with(page_pdf(0, CONTENT));
    // 4 x 2 stored, 2 x 4 shown.
    let reply = add(&mut engine, &mut session, json!({}), jpeg(6));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["box"], json!([305.0, 394.0, 307.0, 398.0]));
    assert!(page_content(&session).contains("0 -4 2 0 305 398 cm\n/Im2 Do"));
}

#[test]
fn the_added_picture_moves_resizes_and_deletes_like_any_other() {
    let (mut engine, mut session) = session_with(page_pdf(0, CONTENT));
    let added = add(&mut engine, &mut session, json!({}), png(200, 100));
    assert_eq!(added["ok"], true, "{added}");
    for (fields, expect) in [
        (
            json!({"action": "move", "box": [100, 100, 300, 200]}),
            "200 0 0 100 100 100 cm",
        ),
        (
            json!({"action": "resize", "box": [100, 100, 500, 300]}),
            "400 0 0 200 100 100 cm",
        ),
    ] {
        let (reply, item) = picture(&mut engine, &mut session, "Im2");
        let mut request = json!({"op": "edit-object", "page": 0, "targetId": item["targetId"],
            "generation": reply["generation"], "revision": reply["revision"]});
        if let (Value::Object(request), Value::Object(fields)) = (&mut request, fields) {
            request.extend(fields);
        }
        op(&mut engine, &mut session, request);
        assert!(
            page_content(&session).contains(expect),
            "{}",
            page_content(&session)
        );
    }
    let (reply, item) = picture(&mut engine, &mut session, "Im2");
    op(
        &mut engine,
        &mut session,
        json!({"op": "edit-object", "page": 0, "targetId": item["targetId"],
            "generation": reply["generation"], "revision": reply["revision"], "action": "delete"}),
    );
    assert!(!page_content(&session).contains("/Im2 Do"));
}
