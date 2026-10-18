// SPDX-License-Identifier: AGPL-3.0-or-later
//! Signature ops on the app session: sign-list reports fields and signer
//! data as the app expects; sign-prepare writes each placeholder exactly
//! once, signs an existing field in place, appends an incremental update to
//! a signed source without changing its bytes, refuses in a fixed order,
//! and never touches the session's document or journal; a signed source the
//! strict reader refuses only for its layout (hybrid or linearized xref,
//! offsets on line ends, text after %%EOF) is extended by an update after
//! its bytes.
//! Then the core incremental writer on classic, xref-stream, encrypted and
//! rebuilt originals.
use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::incremental::{
    Original, startxref, write_incremental, write_incremental_after,
};
use tandem_pdf_engine::object::{Dictionary, Object, ObjectId, decode_text_string};
use tandem_pdf_engine::repair::{RepairLimits, rebuild};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::view::ViewDocument;
use tandem_pdf_engine::writer::{
    DocumentWriteError, DocumentWriteLimits, Encryption, Entropy, WriteOptions, write_document,
};

const SENTINEL: &str = "[0 1000000000 1000000000 1000000000]";

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
            "trailer\n<< /Size {} /Root 1 0 R /ID [<00112233445566778899aabbccddeeff> <00112233445566778899aabbccddeeff>] >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn stream(dictionary: &str, data: &str) -> String {
    format!(
        "<< {dictionary} /Length {} >>\nstream\n{data}\nendstream",
        data.len()
    )
}

/// A one-page form: catalog 1, pages 2, page 3 (with `annots`), content 4,
/// then `rest` from 5.
fn form_pdf(fields: &str, annots: &str, rest: &[&str]) -> Vec<u8> {
    let mut objects = vec![
        format!("<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [{fields}] >> >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [{annots}] >>"
        ),
        stream("", "BT /F1 24 Tf 72 700 Td (Form) Tj ET"),
    ];
    objects.extend(rest.iter().map(|body| body.to_string()));
    classic(&objects)
}

/// Two pages without a form; the second turned by /Rotate 90.
fn two_pages() -> Vec<u8> {
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".into(),
        stream("", "BT /F1 24 Tf 72 700 Td (One) Tj ET"),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Rotate 90 /Contents 4 0 R >>".into(),
    ])
}

fn call(
    engine: &mut AppEngine,
    session: &mut AppSession,
    request: Value,
    inputs: Vec<Vec<u8>>,
) -> (Value, Vec<Vec<u8>>) {
    engine.dispatch(session, request, inputs)
}

fn session_with(bytes: Vec<u8>) -> (AppEngine, AppSession) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = call(&mut engine, &mut session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
    (engine, session)
}

/// The buffer a `{"$buffer": n}` or `{"$bytes": n}` reply field names.
fn output<'o>(field: &Value, outputs: &'o [Vec<u8>]) -> &'o [u8] {
    let index = field
        .get("$buffer")
        .or_else(|| field.get("$bytes"))
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("not a buffer: {field}"));
    &outputs[index as usize]
}

/// A sign-prepare request with the app's defaults, `fields` on top.
fn prepare_request(fields: Value) -> Value {
    let mut request = json!({"id": 7, "op": "sign-prepare", "name": "P0B Signer",
        "reason": "unit test", "page": 0, "rect": null, "certify": false,
        "when": 1_791_979_200_000_i64});
    if let (Value::Object(request), Value::Object(fields)) = (&mut request, fields) {
        request.extend(fields);
    }
    request
}

/// sign-prepare's reply and output bytes.
fn prepare(engine: &mut AppEngine, session: &mut AppSession, fields: Value) -> (Value, Vec<u8>) {
    let (reply, outputs) = call(engine, session, prepare_request(fields), Vec::new());
    if reply["ok"] != true {
        return (reply, Vec::new());
    }
    let bytes = output(&reply["bytes"], &outputs).to_vec();
    (reply, bytes)
}

/// Everything about the session a sign op must leave alone.
fn untouched(session: &AppSession) -> (u64, i64, bool, bool, u64, Vec<u8>) {
    let doc = session.doc.as_ref().unwrap();
    (
        session.revision,
        doc.position(),
        doc.can_undo(),
        doc.can_redo(),
        doc.content_state(),
        session.snapshot_bytes().unwrap(),
    )
}

fn find(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    haystack
        .windows(needle.len())
        .enumerate()
        .filter(|(_, w)| *w == needle)
        .map(|(at, _)| at)
        .collect()
}

fn text(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

/// The placeholders sit where the reply says, once each.
fn check_placeholders(reply: &Value, bytes: &[u8], from: usize) {
    let contents_at = reply["contentsAt"].as_u64().unwrap() as usize;
    let range_at = reply["byteRangeAt"].as_u64().unwrap() as usize;
    assert_eq!(reply["contentsLen"], 16384);
    assert_eq!(&bytes[range_at..range_at + 36], SENTINEL.as_bytes());
    assert_eq!(bytes[contents_at - 1], b'<');
    assert_eq!(bytes[contents_at + 16384], b'>');
    assert!(
        bytes[contents_at..contents_at + 16384]
            .iter()
            .all(|&b| b == b'0')
    );
    assert_eq!(&bytes[contents_at - 10..contents_at], b"/Contents<");
    let tail = &bytes[from..];
    assert_eq!(find(tail, SENTINEL.as_bytes()).len(), 1);
    assert_eq!(find(tail, b"/Contents<").len(), 1);
}

fn open(bytes: &[u8]) -> Document<'_> {
    Document::open(bytes, DocumentLimits::default()).unwrap()
}

fn dict<'d>(doc: &'d Document<'_>, value: &'d Object) -> &'d Dictionary {
    doc.resolve(value).as_dict().unwrap()
}

/// Object `n` of `doc`, a dictionary.
fn object<'d>(doc: &'d Document<'_>, n: u32) -> &'d Dictionary {
    doc.resolve_id(id(n)).as_dict().unwrap()
}

/// The catalog's /AcroForm /Fields of `doc`.
fn fields<'d>(doc: &'d Document<'_>) -> &'d [Object] {
    let catalog = doc.resolve_id(doc.root()).as_dict().unwrap();
    let acro = dict(doc, catalog.get(b"AcroForm".as_slice()).unwrap());
    doc.resolve(acro.get(b"Fields".as_slice()).unwrap())
        .as_array()
        .unwrap()
}

#[test]
fn sign_list_reports_the_signed_fixtures() {
    for (name, certified, range, contents, versions) in [
        ("signed-intact", None, [0, 1355, 8065, 636], 3354, 2),
        ("signed-tampered", None, [0, 1355, 8065, 636], 3354, 2),
        ("signed-untrusted", None, [0, 1355, 8109, 636], 3376, 2),
        ("certified-p2", Some(2), [0, 1383, 8093, 755], 3354, 2),
        (
            "certified-p2-filled",
            Some(2),
            [0, 1383, 8093, 755],
            3354,
            3,
        ),
    ] {
        let source = fixture(&format!("p0b/{name}.pdf"));
        let (mut engine, mut session) = session_with(source.clone());
        let before = untouched(&session);
        let (reply, outputs) = call(
            &mut engine,
            &mut session,
            json!({"id": 3, "op": "sign-list"}),
            Vec::new(),
        );
        assert_eq!(reply["ok"], true, "{name}: {reply}");
        let entries = reply["signatures"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "{name}");
        let entry = &entries[0];
        assert_eq!(
            (&entry["field"], &entry["page"], &entry["empty"]),
            (&json!("Sig1"), &json!(1), &json!(false))
        );
        assert_eq!(entry["rect"], json!([0.0, 792.0, 0.0, 792.0]), "{name}");
        assert_eq!(entry["byteRange"], json!(range), "{name}");
        assert_eq!(entry["subFilter"], "adbe.pkcs7.detached");
        assert_eq!(entry["filter"], "Adobe.PPKLite");
        assert_eq!(entry["reason"], "p0b-probe3");
        assert!(entry["m"].as_str().unwrap().starts_with("D:2026"));
        let bytes = output(&entry["contents"], &outputs);
        assert!(entry["contents"].get("$bytes").is_some(), "a Uint8Array");
        assert_eq!(bytes.len(), contents, "{name}");
        assert_eq!(
            entry.get("docmdp").cloned(),
            certified.map(|p| json!(p as f64))
        );
        assert_eq!(
            reply["certified"],
            certified.map_or(Value::Null, |p| json!(p as f64))
        );
        assert_eq!(reply["versions"], versions, "{name}");
        assert_eq!(output(&reply["bytes"], &outputs), source.as_slice());
        assert_eq!(untouched(&session), before);
    }
}

#[test]
fn sign_list_without_a_signature_or_its_source() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let request = json!({"id": 1, "op": "sign-list"});
    let (reply, _) = call(&mut engine, &mut session, request.clone(), Vec::new());
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (&json!("no-document"), &json!("sign-list: no document open"))
    );
    let (mut engine, mut session) = session_with(fixture("edit-tj.pdf"));
    let (reply, outputs) = call(&mut engine, &mut session, request.clone(), Vec::new());
    assert_eq!(reply["signatures"], json!([]));
    assert_eq!(reply["certified"], Value::Null);
    assert!(output(&reply["bytes"], &outputs).is_empty());

    let (mut engine, mut session) = session_with(fixture("p0b/signed-intact.pdf"));
    session.signed_source = None;
    let (reply, outputs) = call(&mut engine, &mut session, request.clone(), Vec::new());
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (
            &json!("too-large"),
            &json!("sign-list: source is too large to verify")
        )
    );
    assert!(outputs.is_empty(), "a refusal drops the contents it copied");

    let (mut engine, mut session) = session_with(fixture("encrypted.pdf"));
    let (reply, _) = call(&mut engine, &mut session, request, Vec::new());
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (
            &json!("encrypted"),
            &json!("sign-list: encrypted document needs a password")
        )
    );
}

#[test]
fn sign_list_walks_hierarchies_and_marks_malformed_values() {
    let bytes = form_pdf(
        "5 0 R 9 0 R 10 0 R 11 0 R 12 0 R 14 0 R",
        "6 0 R 7 0 R 9 0 R 12 0 R",
        &[
            // 5: a /Sig parent whose kids are widgets 6 and 7.
            "<< /FT /Sig /T (Parent) /Kids [6 0 R 7 0 R] >>",
            "<< /Type /Annot /Subtype /Widget /Parent 5 0 R /T (A) /Rect [72 100 272 160] >>",
            "<< /Type /Annot /Subtype /Widget /Parent 5 0 R /T (B) /Rect [300 100 500 160] /V 8 0 R >>",
            "<< /Type /Sig /ByteRange [0 10 20 30.0] /Contents <0102> /Name (Ann) /Location (Here) /ContactInfo (x@y) /Reference [<< /TransformMethod /UR3 >> << /TransformMethod /DocMDP /TransformParams << /P 3 >> >>] >>",
            // 9: a widget whose /V is not a dictionary; 10: a negative range;
            // 11: a three-number range; 12: a name in the range; 13 its /V.
            "<< /Type /Annot /Subtype /Widget /FT /Sig /T (Str) /Rect [0 0 50 50] /V (x) >>",
            "<< /FT /Sig /T (Neg) /V << /ByteRange [0 -1 2 3] /Contents <00> >> >>",
            "<< /FT /Sig /T (Three) /V << /ByteRange [0 1 2] /Contents <00> >> >>",
            "<< /Type /Annot /Subtype /Widget /FT /Sig /T (Named) /Rect [0 0 50 50] /V 13 0 R >>",
            "<< /ByteRange [0 /Two 2 3] /Contents <00> >>",
            // 14: a text field and an untitled /Sig kid without /V.
            "<< /FT /Tx /T (Text) /Kids [15 0 R] >>",
            "<< /FT /Sig >>",
        ],
    );
    let (mut engine, mut session) = session_with(bytes);
    let (reply, outputs) = call(
        &mut engine,
        &mut session,
        json!({"op": "sign-list"}),
        Vec::new(),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let entries = reply["signatures"].as_array().unwrap();
    let names: Vec<&str> = entries
        .iter()
        .map(|e| e["field"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "Parent", "Parent.A", "Parent.B", "Str", "Neg", "Three", "Named", "Text"
        ]
    );
    // The parent maps to the last widget under it; kids to their own.
    assert_eq!(entries[0]["page"], 1);
    assert_eq!(entries[0]["rect"], json!([300.0, 632.0, 500.0, 692.0]));
    assert_eq!(entries[1]["rect"], json!([72.0, 632.0, 272.0, 692.0]));
    let b = &entries[2];
    assert_eq!(b["empty"], false);
    assert_eq!(b["byteRange"], json!([0, 10, 20, 30]));
    assert_eq!(output(&b["contents"], &outputs), [1, 2]);
    assert_eq!(
        (&b["name"], &b["location"], &b["contact"]),
        (&json!("Ann"), &json!("Here"), &json!("x@y"))
    );
    assert_eq!(b["docmdp"], json!(3.0));
    assert_eq!((&b["subFilter"], &b["filter"]), (&json!(""), &json!("")));
    for entry in &entries[3..7] {
        assert_eq!(
            (&entry["status"], &entry["reason"]),
            (&json!("unsupported"), &json!("malformed")),
            "{entry}"
        );
    }
    // Field nodes that are not widgets on a page.
    assert_eq!(
        (&entries[4]["page"], &entries[4]["rect"]),
        (&json!(0), &Value::Null)
    );
    assert_eq!(entries[7]["empty"], true);
    assert_eq!(reply["certified"], Value::Null);
}

#[test]
fn sign_list_stops_at_the_field_node_limits() {
    // 4097 fields: too many. A /Kids cycle stops at the depth limit.
    let fields: Vec<String> = (0..4097).map(|i| format!("<< /T (F{i}) >>")).collect();
    let bytes = classic(&[
        format!(
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [{}] >> >>",
            fields.join(" ")
        ),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".into(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let (reply, _) = call(
        &mut engine,
        &mut session,
        json!({"op": "sign-list"}),
        Vec::new(),
    );
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (
            &json!("too-many"),
            &json!("sign-list: this form is too large to verify")
        )
    );
    let cyclic = form_pdf(
        "5 0 R",
        "",
        &["<< /FT /Sig /T (Loop) /Kids [5 0 R 5 0 R] >>"],
    );
    let (mut engine, mut session) = session_with(cyclic);
    let (reply, _) = call(
        &mut engine,
        &mut session,
        json!({"op": "sign-list"}),
        Vec::new(),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    // Depths 1..=8 of a binary tree: 255 visits, every one an entry.
    assert_eq!(reply["signatures"].as_array().unwrap().len(), 255);
}

#[test]
fn sign_list_refuses_an_encrypted_form_with_signature_fields() {
    let bytes = form_pdf(
        "5 0 R",
        "5 0 R",
        &["<< /Type /Annot /Subtype /Widget /FT /Sig /T (S) /Rect [0 0 50 50] >>"],
    );
    let doc = open(&bytes);
    let options = WriteOptions {
        encryption: Encryption::Aes256 {
            user_password: b"u",
            owner_password: b"o",
            permissions: 4 | 8 | 16 | 32 | 256,
        },
        entropy: Some(Entropy([9; 32])),
        ..WriteOptions::APP
    };
    let encrypted = write_document(&doc, DocumentWriteLimits::default(), options).unwrap();
    let (mut engine, mut session) = session_with(encrypted);
    let (reply, _) = call(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "u"}),
        Vec::new(),
    );
    assert_eq!(reply["authenticated"], true, "{reply}");
    let (reply, _) = call(
        &mut engine,
        &mut session,
        json!({"op": "sign-list"}),
        Vec::new(),
    );
    assert_eq!(
        reply["error"],
        "sign-list: encrypted signed documents cannot be verified in this version"
    );
    let (reply, _) = prepare(&mut engine, &mut session, json!({}));
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (
            &json!("encrypted"),
            &json!("sign-prepare: encrypted documents cannot be signed in this version")
        )
    );
}

#[test]
fn a_new_widget_is_visible_or_invisible_and_the_session_stays_as_it_was() {
    let (mut engine, mut session) = session_with(two_pages());
    let before = untouched(&session);
    let (reply, bytes) = prepare(
        &mut engine,
        &mut session,
        json!({"rect": [100, 100, 300, 160], "reason": "Approve (final)", "timezoneOffset": 240}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(untouched(&session), before);
    assert_eq!(
        (&reply["page"], &reply["rect"], &reply["revisions"]),
        (&json!(0), &json!([100.0, 100.0, 300.0, 160.0]), &json!(1))
    );
    check_placeholders(&reply, &bytes, 0);
    let all = text(&bytes);
    for needle in [
        "/SubFilter/adbe.pkcs7.detached",
        "/SigFlags 3",
        "/FT/Sig/T(Signature1)/Rect[100 632 300 692]/AP<</N ",
        "/Matrix[1 0 0 1 0 0]",
        "/Name(P0B Signer)/M(D:20261014120000Z)/Reason(Approve \\(final\\))",
        "/Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding",
    ] {
        assert!(all.contains(needle), "{needle} in output");
    }
    let doc = open(&bytes);
    let widget_ref = fields(&doc).last().unwrap();
    let widget = dict(&doc, widget_ref);
    // A new widget (6), its font (7), appearance (8) and signature (9).
    assert_eq!(widget_ref, &Object::Reference(id(6)));
    assert_eq!(widget.get(b"P".as_slice()), Some(&Object::Reference(id(3))));
    let page = object(&doc, 3);
    assert_eq!(
        page.get(b"Annots".as_slice()),
        Some(&Object::array([Object::Reference(id(6))]))
    );
    let ap = doc.decoded_stream(id(8), StreamLimits::default()).unwrap();
    assert_eq!(
        text(&ap),
        "q 0.93 0.95 1 rg 0 0 200 60 re f 0.1 0.2 0.5 RG 1 w 0.5 0.5 199 59 re S BT /Helv 9 Tf \
         0 0 0.15 rg 8 47 Td (Digitally signed by P0B Signer) Tj 0 -11 Td (2026-10-14 08:00) Tj \
         0 -11 Td (Reason: Approve \\(final\\)) Tj ET Q\n"
    );
    let sig = object(&doc, 9);
    assert_eq!(
        sig.get(b"Contents".as_slice()),
        Some(&Object::String(vec![0; 8192]))
    );
    assert!(sig.get(b"Reference".as_slice()).is_none());
    assert!(
        doc.resolve_id(doc.root())
            .as_dict()
            .unwrap()
            .get(b"Perms".as_slice())
            .is_none()
    );
    // hayro draws it.
    assert!(ViewDocument::open(bytes.clone(), "").is_ok());

    let (reply, bytes) = prepare(&mut engine, &mut session, json!({"certify": true}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["rect"], json!([0.0, 0.0, 0.0, 0.0]));
    check_placeholders(&reply, &bytes, 0);
    let all = text(&bytes);
    assert!(all.contains("/FT/Sig/T(Signature1)/Rect[0 0 0 0]/V 7 0 R>>"));
    assert!(all.contains("/Perms<</DocMDP 7 0 R>>"));
    assert!(all.contains(
        "/Reference[<</Type/SigRef/TransformMethod/DocMDP/TransformParams<</Type/TransformParams/P 2/V/1.2>>>>]"
    ));
    assert!(!all.contains("/AP"));
    assert_eq!(untouched(&session), before);

    // Reopened, the prepared copy is signed and certified: read-only.
    let (mut engine, mut certified) = session_with(bytes);
    let info = certified.info().unwrap();
    assert!(info.certified && info.signatures == 1);
    assert!(certified.signed_source.is_some());
    let (reply, _) = prepare(
        &mut engine,
        &mut certified,
        json!({"rect": [72, 72, 272, 132]}),
    );
    assert_eq!(reply["reason"], "certified");
    let (reply, bytes) = prepare(&mut engine, &mut certified, json!({}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["revisions"], 2);
    assert!(text(&bytes).contains("/T(Signature2)"));
}

#[test]
fn the_appearance_turns_with_the_page_and_shows_the_signer_s_local_time() {
    let (mut engine, mut session) = session_with(two_pages());
    let (reply, bytes) = prepare(
        &mut engine,
        &mut session,
        json!({"page": 1, "rect": [72, 500, 300, 560], "timezoneOffset": -120, "reason": ""}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let all = text(&bytes);
    assert!(all.contains("/BBox[0 0 228 60]/Matrix[0 1 -1 0 0 0]"));
    // Page space on a quarter-turned page: x down the user y axis.
    assert!(
        all.contains("/Rect[500 72 560 300]"),
        "{}",
        &all[all.len() - 900..]
    );
    assert!(all.contains("(2026-10-14 14:00) Tj ET Q"), "no reason line");

    // /Rotate inherited from /Pages, and -270 counts as 90.
    let inherited = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /Rotate -270 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".into(),
    ]);
    let (mut engine, mut session) = session_with(inherited);
    let (reply, bytes) = prepare(
        &mut engine,
        &mut session,
        json!({"rect": [100, 100, 300, 200]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(text(&bytes).contains("/Matrix[0 1 -1 0 0 0]"));
    for (rotate, matrix) in [
        (180, "/Matrix[-1 0 0 -1 200 100]"),
        (270, "/Matrix[0 -1 1 0 0 200]"),
        (45, "/Matrix[1 0 0 1 0 0]"),
    ] {
        let turned = classic(&[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Rotate {rotate} >>"),
        ]);
        let (mut engine, mut session) = session_with(turned);
        let (reply, bytes) = prepare(
            &mut engine,
            &mut session,
            json!({"rect": [100, 100, 300, 200]}),
        );
        assert_eq!(reply["ok"], true, "{reply}");
        assert!(text(&bytes).contains(matrix), "{rotate}: {matrix}");
    }
}

/// empty-sigfield.pdf's shape: Signature1 (7) and Signature2 (8, with a
/// grey appearance 9) on page 1.
fn empty_fields() -> Vec<u8> {
    form_pdf(
        "5 0 R 6 0 R",
        "5 0 R 6 0 R",
        &[
            "<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature1) /F 4 /P 3 0 R /Rect [72 112 340 172] /DA (/Helv 12 Tf 0 g) >>",
            "<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature2) /F 4 /P 3 0 R /Rect [360 112 540 172] /AP << /N 7 0 R >> >>",
            &stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 180 60]",
                "0.8 g 0 0 180 60 re f",
            ),
        ],
    )
}

#[test]
fn an_empty_field_is_signed_in_place() {
    let (mut engine, mut session) = session_with(empty_fields());
    let before = untouched(&session);
    let (reply, bytes) = prepare(&mut engine, &mut session, json!({"field": "Signature1"}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(untouched(&session), before);
    assert_eq!(
        (&reply["field"], &reply["rect"], &reply["revisions"]),
        (
            &json!("Signature1"),
            &json!([72.0, 620.0, 340.0, 680.0]),
            &json!(1)
        )
    );
    check_placeholders(&reply, &bytes, 0);
    let doc = open(&bytes);
    let listed: Vec<&Object> = fields(&doc).iter().collect();
    assert_eq!(
        listed,
        [&Object::Reference(id(5)), &Object::Reference(id(6))]
    );
    let widget = object(&doc, 5);
    let keys: Vec<&[u8]> = widget.keys().map(Vec::as_slice).collect();
    assert_eq!(
        keys,
        [
            b"Type".as_slice(),
            b"Subtype",
            b"FT",
            b"T",
            b"F",
            b"P",
            b"Rect",
            b"DA",
            b"AP",
            b"V"
        ]
    );
    let other = object(&doc, 6);
    assert!(other.get(b"V".as_slice()).is_none());
    assert_eq!(
        dict(&doc, other.get(b"AP".as_slice()).unwrap()).get(b"N".as_slice()),
        Some(&Object::Reference(id(7)))
    );
    let all = text(&bytes);
    assert!(all.contains("/AcroForm<</Fields[5 0 R 6 0 R]/SigFlags 3>>"));
    assert!(all.contains("8 47 Td (Digitally signed by P0B Signer)"));
    assert!(all.contains("/BBox[0 0 268 60]"));
}

#[test]
fn field_signatures_are_refused_in_check_field_order() {
    let mut cases = vec![
        // Not a field the form lists.
        (
            form_pdf(
                "",
                "5 0 R",
                &[
                    "<< /Type /Annot /Subtype /Widget /FT /Sig /T (Orphan) /Rect [72 600 272 660] /F 4 /P 3 0 R >>",
                ],
            ),
            "Orphan",
            "invalid",
            "sign-prepare: Orphan is not in the document's /AcroForm /Fields",
        ),
        // A kid of a /Sig parent.
        (
            form_pdf(
                "5 0 R",
                "6 0 R",
                &[
                    "<< /FT /Sig /T (Parent) /Kids [6 0 R] >>",
                    "<< /Type /Annot /Subtype /Widget /Parent 5 0 R /T (Kid) /Rect [72 600 272 660] /F 4 /P 3 0 R >>",
                ],
            ),
            "Parent.Kid",
            "invalid",
            "sign-prepare: Parent.Kid is part of a field hierarchy",
        ),
        // A parent that holds /V.
        (
            form_pdf(
                "5 0 R",
                "6 0 R",
                &[
                    "<< /FT /Sig /T (P) /Kids [6 0 R] /V << /Type /Sig >> >>",
                    "<< /Type /Annot /Subtype /Widget /Parent 5 0 R /T (S) /Rect [72 600 272 660] >>",
                ],
            ),
            "P.S",
            "already-signed",
            "sign-prepare: P.S is already signed",
        ),
        (
            form_pdf(
                "5 0 R",
                "5 0 R",
                &["<< /Type /Annot /Subtype /Widget /FT /Sig /T (Tiny) /Rect [72 112 102 122] >>"],
            ),
            "Tiny",
            "invalid",
            "sign-prepare: Tiny is too small for a signature appearance",
        ),
        (
            form_pdf(
                "5 0 R",
                "5 0 R",
                &[
                    "<< /Type /Annot /Subtype /Widget /FT /Sig /T (RO) /Ff 1 /Rect [72 72 272 132] >>",
                ],
            ),
            "RO",
            "invalid",
            "sign-prepare: RO is read-only",
        ),
        (
            empty_fields(),
            "Nope",
            "no-such-field",
            "sign-prepare: page 0 has no signature field Nope",
        ),
    ];
    // A text field of that name is no signature field.
    cases.push((
        form_pdf(
            "5 0 R",
            "5 0 R",
            &["<< /Type /Annot /Subtype /Widget /FT /Tx /T (Text) /Rect [72 72 272 132] >>"],
        ),
        "Text",
        "no-such-field",
        "sign-prepare: page 0 has no signature field Text",
    ));
    for (bytes, field, reason, message) in cases {
        let (mut engine, mut session) = session_with(bytes);
        let before = untouched(&session);
        let (reply, _) = prepare(&mut engine, &mut session, json!({"field": field}));
        assert_eq!(
            (&reply["reason"], &reply["error"]),
            (&json!(reason), &json!(message)),
            "{field}"
        );
        assert_eq!(reply["dirty"], false);
        assert_eq!(untouched(&session), before);
    }
}

#[test]
fn requests_are_refused_in_engine_order() {
    let invalid_signer = "sign-prepare: invalid signer, reason, or timestamp";
    let cases = [
        (json!({"name": ""}), "invalid", invalid_signer),
        (json!({"name": "x".repeat(201)}), "invalid", invalid_signer),
        (json!({"name": 5}), "invalid", invalid_signer),
        (json!({"reason": "a\nb"}), "invalid", invalid_signer),
        (json!({"reason": "\u{7f}"}), "invalid", invalid_signer),
        (
            json!({"reason": "é".repeat(201)}),
            "invalid",
            invalid_signer,
        ),
        (json!({"certify": 1}), "invalid", invalid_signer),
        (json!({"when": "now"}), "invalid", invalid_signer),
        (
            json!({"when": {"$number": "NaN"}}),
            "invalid",
            invalid_signer,
        ),
        (json!({"when": 9e15}), "invalid", invalid_signer),
        (
            json!({"field": "Signature1", "rect": [1, 2, 3, 4]}),
            "invalid",
            "sign-prepare: a field signature takes rect null and the field's page",
        ),
        (
            json!({"field": ""}),
            "invalid",
            "sign-prepare: a field signature takes rect null and the field's page",
        ),
        (
            json!({"field": 3}),
            "invalid",
            "sign-prepare: a field signature takes rect null and the field's page",
        ),
        (json!({"page": 2}), "invalid", "sign-prepare: invalid page"),
        (
            json!({"page": 0.5}),
            "invalid",
            "sign-prepare: invalid page",
        ),
        (json!({"page": -1}), "invalid", "sign-prepare: invalid page"),
        (
            json!({"rect": [1, 2, 3]}),
            "invalid",
            "sign-prepare: invalid rectangle",
        ),
        (
            json!({"rect": [1, 2, 3, {"$number": "Infinity"}]}),
            "invalid",
            "sign-prepare: invalid rectangle",
        ),
        (
            json!({"rect": [1, 1, 2, 2]}),
            "invalid",
            "sign-prepare: the signature needs at least 40x20 pt of the page",
        ),
        // Clamped to the page first: 600..612 is too narrow.
        (
            json!({"rect": [600, 100, 900, 200]}),
            "invalid",
            "sign-prepare: the signature needs at least 40x20 pt of the page",
        ),
    ];
    let (mut engine, mut session) = session_with(two_pages());
    let before = untouched(&session);
    for (fields, reason, message) in cases {
        let (reply, _) = prepare(&mut engine, &mut session, fields.clone());
        assert_eq!(
            (&reply["reason"], &reply["error"]),
            (&json!(reason), &json!(message)),
            "{fields}"
        );
    }
    let mut missing = prepare_request(json!({}));
    missing.as_object_mut().unwrap().remove("rect");
    let (reply, _) = call(&mut engine, &mut session, missing, Vec::new());
    assert_eq!(reply["error"], "sign-prepare: invalid rectangle");
    assert_eq!(untouched(&session), before);

    // Before the signer checks: no document, a lock, the source size.
    let mut empty = AppSession::default();
    let (reply, _) = prepare(&mut engine, &mut empty, json!({"name": ""}));
    assert_eq!(reply["reason"], "no-document");
    let (mut engine, mut locked) = session_with(fixture("encrypted-aes256.pdf"));
    let (reply, _) = prepare(&mut engine, &mut locked, json!({"name": ""}));
    assert_eq!(reply["reason"], "encrypted");
    let (mut engine, mut large) = session_with(two_pages());
    large.source_bytes = 50 * 1024 * 1024 + 1;
    let (reply, _) = prepare(&mut engine, &mut large, json!({"name": ""}));
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (
            &json!("too-large"),
            &json!("sign-prepare: source exceeds save limit")
        )
    );

    // Certification: P=1 refuses everything; a second certification and a
    // new visible field on a certified document are refused.
    let (mut engine, mut p1) = session_with(fixture("p5/certified-p1.pdf"));
    let (reply, _) = prepare(&mut engine, &mut p1, json!({}));
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (
            &json!("certified"),
            &json!("sign-prepare: this certified document does not allow changes")
        )
    );
    let (mut engine, mut signed) = session_with(fixture("p0b/signed-intact.pdf"));
    let (reply, _) = prepare(&mut engine, &mut signed, json!({"certify": true}));
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (
            &json!("already-signed"),
            &json!("sign-prepare: certification must be the first signature")
        )
    );
    let (mut engine, mut p2) = session_with(fixture("p0b/certified-p2.pdf"));
    let (reply, _) = prepare(&mut engine, &mut p2, json!({"rect": [72, 72, 272, 132]}));
    assert_eq!(
        reply["error"],
        "sign-prepare: a certified document accepts only an invisible signature"
    );
}

#[test]
fn a_pending_aes_256_save_is_refused_rather_than_signed_in_plaintext() {
    let (mut engine, mut session) = session_with(two_pages());
    let (reply, _) = call(
        &mut engine,
        &mut session,
        json!({"op": "security-set", "mode": "aes-256", "userPassword": "u"}),
        Vec::new(),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let (reply, _) = prepare(&mut engine, &mut session, json!({}));
    assert_eq!(reply["reason"], "encrypted");
    let (reply, _) = call(
        &mut engine,
        &mut session,
        json!({"op": "security-set", "mode": "none"}),
        Vec::new(),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let (reply, _) = prepare(&mut engine, &mut session, json!({}));
    assert_eq!(reply["ok"], true, "{reply}");
}

#[test]
fn unsaved_edits_are_in_the_copy_and_stay_undoable() {
    let (mut engine, mut session) = session_with(two_pages());
    let (reply, _) = call(
        &mut engine,
        &mut session,
        json!({"op": "page-rotate", "page": 0, "by": 90}),
        Vec::new(),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let before = untouched(&session);
    let (reply, bytes) = prepare(
        &mut engine,
        &mut session,
        json!({"rect": [100, 100, 300, 200]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(
        text(&bytes).contains("/Matrix[0 1 -1 0 0 0]"),
        "the rotation is in the copy"
    );
    assert_eq!(untouched(&session), before);
    let (reply, _) = call(&mut engine, &mut session, json!({"op": "undo"}), Vec::new());
    assert_eq!(reply["step"], "rotate page");
}

/// The update after `source`: its xref rows (number, in use) and trailer.
fn update_of(bytes: &[u8], source: usize) -> (Vec<(u32, bool)>, String) {
    let update = text(&bytes[source..]);
    let xref = update.rfind("\nxref\n").unwrap();
    let trailer = update.rfind("trailer\n").unwrap();
    let mut rows = Vec::new();
    let mut lines = update[xref + 6..trailer].lines();
    while let Some(header) = lines.next() {
        let mut parts = header.split(' ');
        let first: u32 = parts.next().unwrap().parse().unwrap();
        let count: u32 = parts.next().unwrap().parse().unwrap();
        for n in 0..count {
            let row = lines.next().unwrap();
            rows.push((first + n, row.ends_with("n ")));
        }
    }
    (rows, update[trailer..].to_string())
}

#[test]
fn a_signed_source_gets_an_incremental_update_of_only_what_changed() {
    let (mut engine, mut fresh) = session_with(empty_fields());
    let (_, first) = prepare(&mut engine, &mut fresh, json!({"field": "Signature1"}));
    let (mut engine, mut session) = session_with(first.clone());
    assert_eq!(session.signed_source.as_deref(), Some(first.as_slice()));
    let before = untouched(&session);

    let (reply, bytes) = prepare(&mut engine, &mut session, json!({"field": "Signature2"}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(untouched(&session), before);
    assert_eq!(&bytes[..first.len()], first.as_slice());
    assert_eq!(reply["revisions"], 2);
    check_placeholders(&reply, &bytes, first.len());
    let (rows, trailer) = update_of(&bytes, first.len());
    // Widget 6 in place; the AcroForm lives in the catalog (1); new font,
    // appearance and signature from the source's /Size on.
    let size = open(&first)
        .trailer()
        .get(b"Size".as_slice())
        .and_then(Object::as_i64)
        .unwrap() as u32;
    assert_eq!(
        rows,
        [
            (1, true),
            (6, true),
            (size, true),
            (size + 1, true),
            (size + 2, true)
        ]
    );
    let prev = startxref(&first).unwrap();
    assert!(trailer.contains(&format!("/Prev {prev}>>")), "{trailer}");
    assert!(
        trailer.contains("/ID[<00112233445566778899AABBCCDDEEFF><"),
        "{trailer}"
    );
    assert!(
        !trailer
            .contains("/ID[<00112233445566778899AABBCCDDEEFF><00112233445566778899AABBCCDDEEFF>]")
    );
    let doc = open(&bytes);
    assert_eq!(doc.trailer_chain().count(), 2);
    let widget = object(&doc, 6);
    assert!(widget.get(b"V".as_slice()).is_some());

    // A third: a new invisible widget appended to the page and the form.
    let (mut engine, mut twice) = session_with(bytes.clone());
    let (reply, third) = prepare(&mut engine, &mut twice, json!({}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["revisions"], 3);
    let (rows, _) = update_of(&third, bytes.len());
    let numbers: Vec<u32> = rows.iter().map(|(n, _)| *n).collect();
    let size = open(&bytes)
        .trailer()
        .get(b"Size".as_slice())
        .and_then(Object::as_i64)
        .unwrap() as u32;
    assert_eq!(numbers, [1, 3, size, size + 1]);
    let doc = open(&third);
    let names: Vec<String> = fields(&doc)
        .iter()
        .map(|f| match dict(&doc, f).get(b"T".as_slice()) {
            Some(Object::String(t)) => decode_text_string(t),
            _ => String::new(),
        })
        .collect();
    assert_eq!(names, ["Signature1", "Signature2", "Signature3"]);
    let (reply, outputs) = call(
        &mut engine,
        &mut twice,
        json!({"op": "sign-list"}),
        Vec::new(),
    );
    assert_eq!(reply["versions"], 2, "a full rewrite and one update");
    assert_eq!(output(&reply["bytes"], &outputs), bytes.as_slice());
}

/// A signed document whose catalog, form and page sit in an object stream
/// behind a cross-reference stream.
fn signed_xref_stream() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] /SigFlags 3 >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Sig /T (Sig1) /Rect [0 0 0 0] /P 3 0 R /V 5 0 R >>"
            .into(),
        "<< /Type /Sig /ByteRange [0 1 2 3] /Contents <00> >>".into(),
    ];
    let packed = [1usize, 2, 3, 4];
    let container = 6;
    let xref_id = 7;
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut rows = vec![(0u8, 0u64, 65535u16); xref_id + 1];
    rows[5] = (1, bytes.len() as u64, 0);
    bytes.extend_from_slice(format!("5 0 obj\n{}\nendobj\n", objects[4]).as_bytes());
    let mut header = String::new();
    let mut members = Vec::new();
    for (slot, &number) in packed.iter().enumerate() {
        header.push_str(&format!("{number} {} ", members.len()));
        members.extend_from_slice(objects[number - 1].as_bytes());
        members.push(b' ');
        rows[number] = (2, container as u64, slot as u16);
    }
    let mut data = header.into_bytes();
    let first = data.len();
    data.extend_from_slice(&members);
    rows[container] = (1, bytes.len() as u64, 0);
    bytes.extend_from_slice(
        format!(
            "{container} 0 obj\n<< /Type /ObjStm /N 4 /First {first} /Length {} >>\nstream\n",
            data.len()
        )
        .as_bytes(),
    );
    bytes.extend_from_slice(&data);
    bytes.extend_from_slice(b"\nendstream\nendobj\n");
    let xref = bytes.len();
    rows[xref_id] = (1, xref as u64, 0);
    let mut table = Vec::new();
    for (kind, field, generation) in rows {
        table.push(kind);
        table.extend_from_slice(&(field as u32).to_be_bytes());
        table.extend_from_slice(&generation.to_be_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "{xref_id} 0 obj\n<< /Type /XRef /Size {} /W [1 4 2] /Root 1 0 R /Length {} >>\nstream\n",
            xref_id + 1,
            table.len()
        )
        .as_bytes(),
    );
    bytes.extend_from_slice(&table);
    bytes.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    bytes
}

#[test]
fn an_xref_stream_source_stays_readable_after_the_update() {
    let source = signed_xref_stream();
    let (mut engine, mut session) = session_with(source.clone());
    assert!(session.info().unwrap().signatures == 1);
    let (reply, bytes) = prepare(&mut engine, &mut session, json!({}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(&bytes[..source.len()], source.as_slice());
    let (rows, trailer) = update_of(&bytes, source.len());
    let numbers: Vec<u32> = rows.iter().map(|(n, _)| *n).collect();
    // The catalog (form) and page come out of the object stream.
    assert_eq!(numbers, [1, 3, 8, 9]);
    assert!(trailer.contains(&format!("/Prev {}>>", startxref(&source).unwrap())));
    assert!(!trailer.contains("/XRef") && !trailer.contains("/W"));
    let doc = open(&bytes);
    assert_eq!(fields(&doc).len(), 2);
    assert!(ViewDocument::open(bytes.clone(), "").is_ok());
    let (mut engine, mut reopened) = session_with(bytes);
    assert_eq!(reopened.info().unwrap().signatures, 2);
    let (reply, _) = call(
        &mut engine,
        &mut reopened,
        json!({"op": "sign-list"}),
        Vec::new(),
    );
    assert_eq!(reply["versions"], 2);
}

#[test]
fn a_damaged_signed_source_is_not_extended() {
    // Wrong xref offsets: the session opens it through the repair scan, but
    // an update could not keep the earlier signature's bytes meaningful.
    let mut bytes = signed_xref_stream();
    let at = find(&bytes, b"startxref\n")[0] + 10;
    bytes[at] = b'1';
    let (mut engine, mut session) = session_with(bytes);
    assert!(session.signed_source.is_some());
    let (reply, _) = prepare(&mut engine, &mut session, json!({}));
    assert_eq!(reply["reason"], "invalid");
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .starts_with("sign-prepare: the signed file needs repair")
    );
}

/// The signed page of the layout tests: catalog 1 (form), pages 2, page 3,
/// widget 4 (Sig1, signed by 7), content 5, font 6, signature 7.
fn signed_objects() -> Vec<String> {
    vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R] /SigFlags 3 >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 6 0 R >> >> /Contents 5 0 R /Annots [4 0 R] >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Sig /T (Sig1) /Rect [0 0 0 0] /F 4 /P 3 0 R /V 7 0 R >>".into(),
        stream("", "BT /F1 24 Tf 72 700 Td (Signed) Tj ET"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 10 20 30] /Contents <00> >>".into(),
    ]
}

/// `signed_objects` as MS Office writes files: a classic table (/Size 10)
/// marks the font free and its /XRefStm, xref stream 9, places it in object
/// stream 8.
fn signed_hybrid() -> Vec<u8> {
    let objects = signed_objects();
    let mut bytes = b"%PDF-1.5\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = [0usize; 10];
    for (index, body) in objects.iter().enumerate() {
        let number = index + 1;
        if number != 6 {
            offsets[number] = bytes.len();
            bytes.extend_from_slice(format!("{number} 0 obj\n{body}\nendobj\n").as_bytes());
        }
    }
    let data = format!("6 0 {}", objects[5]);
    offsets[8] = bytes.len();
    bytes.extend_from_slice(
        format!(
            "8 0 obj\n<< /Type /ObjStm /N 1 /First 4 /Length {} >>\nstream\n{data}\nendstream\nendobj\n",
            data.len()
        )
        .as_bytes(),
    );
    let mut rows = Vec::new();
    for number in 0..10 {
        let (kind, container) = if number == 6 { (2u8, 8u32) } else { (0, 0) };
        rows.push(kind);
        rows.extend_from_slice(&container.to_be_bytes());
        rows.extend_from_slice(&0u16.to_be_bytes());
    }
    offsets[9] = bytes.len();
    bytes.extend_from_slice(
        format!(
            "9 0 obj\n<< /Type /XRef /Size 10 /W [1 4 2] /Length {} >>\nstream\n",
            rows.len()
        )
        .as_bytes(),
    );
    bytes.extend_from_slice(&rows);
    bytes.extend_from_slice(b"\nendstream\nendobj\n");
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 10\n0000000000 65535 f \n");
    for (number, offset) in offsets.iter().enumerate().skip(1) {
        let row = match number {
            6 => "0000000000 00000 f \n".to_string(),
            _ => format!("{offset:010} 00000 n \n"),
        };
        bytes.extend_from_slice(row.as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size 10 /Root 1 0 R /XRefStm {} >>\nstartxref\n{xref}\n%%EOF\n",
            offsets[9]
        )
        .as_bytes(),
    );
    bytes
}

/// `signed_objects` in a linearized layout: parameter dictionary 8, then a
/// first-page section (3, 5 and 8; /Size 9) whose /Prev points forward to
/// the main section (/Size 8) at the end.
fn signed_linearized() -> Vec<u8> {
    let objects = signed_objects();
    let mut bytes = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let lin_at = bytes.len();
    let lin = |length: usize| {
        format!(
            "8 0 obj\n<< /Linearized 1 /L {length:010} /H [0 0] /O 3 /E 0 /N 1 /T 0 >>\nendobj\n"
        )
    };
    let first = |page: usize, content: usize, main: usize| {
        format!(
            "xref\n3 1\n{page:010} 00000 n \n5 1\n{content:010} 00000 n \n8 1\n{lin_at:010} 00000 n \ntrailer\n<< /Size 9 /Root 1 0 R /Prev {main:010} >>\nstartxref\n0\n%%EOF\n"
        )
    };
    bytes.extend_from_slice(lin(0).as_bytes());
    let first_at = bytes.len();
    bytes.extend_from_slice(first(0, 0, 0).as_bytes());
    let mut offsets = [0usize; 8];
    for number in [3, 5, 1, 2, 4, 6, 7] {
        offsets[number] = bytes.len();
        let body = &objects[number - 1];
        bytes.extend_from_slice(format!("{number} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let main = bytes.len();
    bytes.extend_from_slice(
        format!(
            "xref\n0 3\n0000000000 65535 f \n{:010} 00000 n \n{:010} 00000 n \n4 1\n{:010} 00000 n \n6 2\n{:010} 00000 n \n{:010} 00000 n \ntrailer\n<< /Size 8 >>\nstartxref\n{first_at}\n%%EOF\n",
            offsets[1], offsets[2], offsets[4], offsets[6], offsets[7]
        )
        .as_bytes(),
    );
    let head = lin(bytes.len()) + &first(offsets[3], offsets[5], main);
    bytes[lin_at..lin_at + head.len()].copy_from_slice(head.as_bytes());
    bytes
}

/// `bytes` with every in-use row of its last xref table moved by `delta`.
fn rows_moved(bytes: &[u8], delta: i64) -> Vec<u8> {
    let at = *find(bytes, b"\nxref\n").last().unwrap();
    let lines: Vec<Vec<u8>> = bytes[at..]
        .split(|&b| b == b'\n')
        .map(|line| match line.strip_suffix(b" 00000 n ") {
            Some(offset) if offset.len() == 10 => {
                let offset: i64 = text(offset).parse().unwrap();
                format!("{:010} 00000 n ", offset + delta).into_bytes()
            }
            _ => line.to_vec(),
        })
        .collect();
    [&bytes[..at], lines.join(&b'\n').as_slice()].concat()
}

/// `bytes` with its last `startxref` value moved by `delta`.
fn startxref_moved(bytes: &[u8], delta: i64) -> Vec<u8> {
    let at = find(bytes, b"startxref\n").last().unwrap() + 10;
    let end = at
        + bytes[at..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
    let value: i64 = text(&bytes[at..end]).parse().unwrap();
    [
        &bytes[..at],
        (value + delta).to_string().as_bytes(),
        &bytes[end..],
    ]
    .concat()
}

/// sign-list's `versions` and the number of signature entries.
fn listed(engine: &mut AppEngine, session: &mut AppSession) -> (Value, usize) {
    let (reply, _) = call(engine, session, json!({"op": "sign-list"}), Vec::new());
    assert_eq!(reply["ok"], true, "{reply}");
    let signed = reply["signatures"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["empty"] == false)
        .count();
    (reply["versions"].clone(), signed)
}

#[test]
fn hybrid_and_linearized_signed_sources_take_an_update_after_their_bytes() {
    // (layout, source, xref sections, versions, /Size)
    for (label, source, sections, versions, size) in [
        ("hybrid", signed_hybrid(), 1, 1, 10_u32),
        ("linearized", signed_linearized(), 2, 1, 9),
    ] {
        // The strict reader refuses both; the rebuild reads their own xref.
        assert!(
            Document::open(&source, DocumentLimits::default()).is_err(),
            "{label}"
        );
        let report = rebuild(&source, RepairLimits::default()).unwrap().report;
        assert!(report.xref_intact && !report.damaged, "{label}: {report:?}");
        assert_eq!(
            (report.xref_sections, report.xref_size),
            (sections, u64::from(size)),
            "{label}"
        );
        let (mut engine, mut session) = session_with(source.clone());
        let info = session.info().unwrap();
        assert_eq!((info.repaired, info.signatures), (false, 1), "{label}");
        assert_eq!(listed(&mut engine, &mut session), (json!(versions), 1));
        let before = untouched(&session);

        let (reply, bytes) = prepare(
            &mut engine,
            &mut session,
            json!({"rect": [100, 100, 300, 160]}),
        );
        assert_eq!(reply["ok"], true, "{label}: {reply}");
        assert_eq!(untouched(&session), before);
        assert_eq!(&bytes[..source.len()], source.as_slice(), "{label}");
        assert_eq!(reply["revisions"], 2);
        check_placeholders(&reply, &bytes, source.len());
        // The catalog and page, then the widget, font, appearance and
        // signature from the original's /Size on: never a number its
        // object stream, xref stream or parameter dictionary had.
        let (rows, trailer) = update_of(&bytes, source.len());
        let numbers: Vec<u32> = rows.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            numbers,
            [1, 3, size, size + 1, size + 2, size + 3],
            "{label}"
        );
        assert!(trailer.starts_with(&format!("trailer\n<</Size {}/Root 1 0 R", size + 4)));
        let prev = startxref(&source).unwrap();
        assert!(trailer.contains(&format!("/Prev {prev}>>")), "{trailer}");

        // The output still reads from its own xref, the font in the object
        // stream and the first signature included.
        let report = rebuild(&bytes, RepairLimits::default()).unwrap().report;
        assert!(report.xref_intact && !report.damaged, "{label}: {report:?}");
        assert_eq!(report.xref_sections, sections + 1);
        let (mut engine, mut reopened) = session_with(bytes.clone());
        let info = reopened.info().unwrap();
        assert_eq!((info.repaired, info.signatures), (false, 2), "{label}");
        let doc = reopened.doc.as_ref().unwrap().document();
        let font = object(doc, 6).get(b"BaseFont".as_slice());
        assert_eq!(
            font.and_then(Object::as_name),
            Some(b"Helvetica".as_slice())
        );
        assert_eq!(fields(doc).len(), 2);
        assert_eq!(listed(&mut engine, &mut reopened), (json!(versions + 1), 2));

        // A third signature follows the second.
        let (reply, third) = prepare(&mut engine, &mut reopened, json!({}));
        assert_eq!(reply["ok"], true, "{label}: {reply}");
        assert_eq!(reply["revisions"], 3);
        assert_eq!(&third[..bytes.len()], bytes.as_slice());
        let (rows, _) = update_of(&third, bytes.len());
        let numbers: Vec<u32> = rows.iter().map(|(n, _)| *n).collect();
        assert_eq!(numbers, [1, 3, size + 4, size + 5], "{label}");
        let (mut engine, mut thrice) = session_with(third);
        assert_eq!(listed(&mut engine, &mut thrice), (json!(versions + 2), 3));
    }
}

#[test]
fn xref_quirks_read_without_repair_take_an_update() {
    let clean = classic(&signed_objects());
    let xref = find(&clean, b"\nxref\n")[0] + 1;
    for (label, source) in [
        (
            "offsets on the line end before each object",
            rows_moved(&clean, -1),
        ),
        (
            "startxref on the line end before xref",
            startxref_moved(&clean, -1),
        ),
        (
            "text after %%EOF",
            [clean.as_slice(), b"trailing text\n"].concat(),
        ),
    ] {
        assert!(
            Document::open(&source, DocumentLimits::default()).is_err(),
            "{label}"
        );
        let (mut engine, mut session) = session_with(source.clone());
        assert!(!session.info().unwrap().repaired, "{label}");
        assert_eq!(listed(&mut engine, &mut session), (json!(1), 1), "{label}");
        let (reply, bytes) = prepare(&mut engine, &mut session, json!({}));
        assert_eq!(reply["ok"], true, "{label}: {reply}");
        assert_eq!(&bytes[..source.len()], source.as_slice(), "{label}");
        assert_eq!(reply["revisions"], 2);
        let (rows, trailer) = update_of(&bytes, source.len());
        assert_eq!(
            rows,
            [(1, true), (3, true), (8, true), (9, true)],
            "{label}"
        );
        // /Prev names the table itself, wherever startxref landed.
        assert!(
            trailer.contains(&format!("/Prev {xref}>>")),
            "{label}: {trailer}"
        );
        let (mut engine, mut reopened) = session_with(bytes.clone());
        assert_eq!(reopened.info().unwrap().signatures, 2, "{label}");
        assert_eq!(listed(&mut engine, &mut reopened), (json!(2), 2), "{label}");
    }
    // Offsets inside the objects are damage (spec 03 section 4.3).
    let (mut engine, mut session) = session_with(rows_moved(&clean, 1));
    assert!(session.info().unwrap().repaired);
    assert_eq!(listed(&mut engine, &mut session), (json!(1), 1));
    let (reply, _) = prepare(&mut engine, &mut session, json!({}));
    assert_eq!(reply["reason"], "invalid");
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .starts_with("sign-prepare: the signed file needs repair")
    );
}

#[test]
fn hostile_requests_and_forms_never_panic() {
    let hostile = [
        json!(null),
        json!(true),
        json!(-1e308),
        json!(1e308),
        json!("x"),
        json!([]),
        json!([1e308, -1e308, 1e308, -1e308]),
        json!({"$number": "NaN"}),
        json!({"$number": "-Infinity"}),
        json!({"a": 1}),
        json!("\u{0}\u{ffff}"),
    ];
    let documents = [
        two_pages(),
        empty_fields(),
        form_pdf("5 0 R", "5 0 R 5 0 R 6 0 R", &[
            "<< /Type /Annot /Subtype /Widget /Parent 5 0 R /T (Self) /Rect [999999999999999999999999999999999999999.0 -999999999999999999999999999999999999999.0 /nan 3] /FT /Sig /Ff 999999999999999999999999999999999999999.5 >>",
            "<< /Type /Annot /Subtype /Widget /Parent 6 0 R /FT /Sig /T 7 /Rect 5 >>",
        ]),
        classic(&[
            "<< /Type /Catalog /Pages 2 0 R /AcroForm 3 0 R /Perms << /DocMDP << /Reference 4 >> >> >>".into(),
            "<< /Type /Pages /Kids [5 0 R] /Count 1 >>".into(),
            "<< /Fields 7 >>".into(),
            "null".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 999999999999999999999999999999999999999.0 999999999999999999999999999999999999999.0] /UserUnit 999999999999999999999999999999999999999.0 /Rotate -2147483649 /Annots 9 >>".into(),
        ]),
    ];
    for bytes in documents {
        let (mut engine, mut session) = session_with(bytes);
        let (reply, _) = call(
            &mut engine,
            &mut session,
            json!({"op": "sign-list"}),
            Vec::new(),
        );
        assert!(reply.get("ok").is_some());
        for key in [
            "name",
            "reason",
            "page",
            "rect",
            "certify",
            "when",
            "field",
            "timezoneOffset",
        ] {
            for value in &hostile {
                let (reply, _) = prepare(&mut engine, &mut session, json!({ key: value }));
                assert!(reply.get("ok").is_some(), "{key}: {value}");
            }
        }
        for rect in [
            json!([0, 0, 612, 792]),
            json!([-1e300, -1e300, 1e300, 1e300]),
            json!([500, 500, 100, 100]),
        ] {
            let (reply, _) = prepare(&mut engine, &mut session, json!({"rect": rect}));
            assert!(reply.get("ok").is_some());
        }
        for field in ["Signature1", "Self", "Signature2"] {
            let (reply, _) = prepare(&mut engine, &mut session, json!({"field": field}));
            assert!(reply.get("ok").is_some());
        }
    }
}

// The core incremental writer.

fn limits() -> DocumentWriteLimits {
    DocumentWriteLimits::default()
}

#[test]
fn an_incremental_update_writes_only_the_changed_objects() {
    let source = two_pages();
    let mut doc =
        Document::open_shared(Arc::new(source.clone()), DocumentLimits::default(), b"").unwrap();
    doc.put_value(id(6), Object::text("new")).unwrap();
    let mut page = object(&doc, 3).clone();
    page.insert(b"Extra".to_vec(), Object::Reference(id(6)));
    doc.put_value(id(3), Object::Dictionary(page)).unwrap();
    doc.remove_object(id(4)).unwrap();
    let changed = BTreeSet::from([id(3), id(4), id(6)]);
    let bytes = write_incremental(&doc, &changed, limits(), None).unwrap();
    assert_eq!(&bytes[..source.len()], source.as_slice());
    let (rows, trailer) = update_of(&bytes, source.len());
    assert_eq!(rows, [(3, true), (4, false), (6, true)]);
    assert!(
        trailer.starts_with("trailer\n<</Size 7/Root 1 0 R/ID["),
        "{trailer}"
    );
    let reread = open(&bytes);
    assert_eq!(reread.resolve_id(id(6)), &Object::text("new"));
    assert!(reread.object(id(4)).is_err());
    // Nothing changed: an empty section, still a valid update.
    let unchanged = write_incremental(&doc, &BTreeSet::new(), limits(), None).unwrap();
    assert_eq!(open(&unchanged).trailer_chain().count(), 2);
    // The whole output counts against the limit.
    let mut tight = limits();
    tight.syntax.max_output_bytes = source.len() + 10;
    assert!(matches!(
        write_incremental(&doc, &changed, tight, None),
        Err(DocumentWriteError::Serialize(_))
    ));
}

#[test]
fn an_encrypted_original_gets_encrypted_objects_and_keeps_its_handler() {
    for (name, password) in [
        ("encrypted.pdf", b"pdf4all-user".as_slice()),
        ("encrypted-aes256.pdf", b"pdf4all-user"),
    ] {
        let source = fixture(name);
        let mut doc = Document::open_shared(
            Arc::new(source.clone()),
            DocumentLimits::default(),
            password,
        )
        .unwrap();
        let next = doc.next_id().unwrap();
        doc.put_value(next, Object::text("secret text")).unwrap();
        let stream_id = ObjectId::new(next.number() + 1, 0).unwrap();
        doc.put_stream_decoded(stream_id, Dictionary::new(), b"stream secret".to_vec())
            .unwrap();
        let changed = BTreeSet::from([next, stream_id]);
        assert!(matches!(
            write_incremental(&doc, &changed, limits(), None),
            Err(DocumentWriteError::Invalid(_))
        ));
        let bytes = write_incremental(&doc, &changed, limits(), Some(Entropy([3; 32]))).unwrap();
        assert_eq!(&bytes[..source.len()], source.as_slice());
        assert!(find(&bytes, b"secret").is_empty(), "{name}: encrypted");
        let reread =
            Document::open_shared(Arc::new(bytes.clone()), DocumentLimits::default(), password)
                .unwrap();
        assert_eq!(
            reread.resolve_id(next),
            &Object::text("secret text"),
            "{name}"
        );
        assert_eq!(
            reread
                .decoded_stream(stream_id, StreamLimits::default())
                .unwrap()
                .as_slice(),
            b"stream secret"
        );
        assert!(ViewDocument::open(bytes.clone(), "pdf4all-user").is_ok());
        let (_, trailer) = update_of(&bytes, source.len());
        assert!(trailer.contains("/Encrypt"), "{name}: {trailer}");
        // The handler's own dictionary cannot change incrementally.
        if let Some(Object::Reference(encrypt)) = doc.trailer_entry(b"Encrypt").cloned() {
            let changed = BTreeSet::from([encrypt]);
            assert!(matches!(
                write_incremental(&doc, &changed, limits(), Some(Entropy([3; 32]))),
                Err(DocumentWriteError::Unsupported(_))
            ));
        }
    }
    // Without the password: refused.
    let locked = Document::open_shared_locked(
        Arc::new(fixture("encrypted.pdf")),
        DocumentLimits::default(),
    )
    .unwrap();
    assert!(matches!(
        write_incremental(&locked, &BTreeSet::new(), limits(), Some(Entropy([1; 32]))),
        Err(DocumentWriteError::Unsupported(_))
    ));
}

#[test]
fn an_update_after_rebuilt_bytes_keeps_the_original_numbers_and_size() {
    let source = signed_hybrid();
    let rebuilt = rebuild(&source, RepairLimits::default()).unwrap();
    let mut doc = Document::open_owned(rebuilt.bytes, DocumentLimits::default()).unwrap();
    // The rebuild has neither the object stream nor the xref stream.
    assert_eq!(
        doc.trailer().get(b"Size".as_slice()),
        Some(&Object::Integer(8))
    );
    let original = Original {
        bytes: &source,
        size: rebuilt.report.xref_size,
    };
    let unchanged =
        write_incremental_after(&doc, original, &BTreeSet::new(), limits(), None).unwrap();
    assert_eq!(&unchanged[..source.len()], source.as_slice());
    assert!(text(&unchanged[source.len()..]).contains("trailer\n<</Size 10/Root 1 0 R"));
    let mut page = object(&doc, 3).clone();
    page.insert(b"Marked".to_vec(), Object::Boolean(true));
    doc.put_value(id(3), Object::Dictionary(page)).unwrap();
    let bytes =
        write_incremental_after(&doc, original, &BTreeSet::from([id(3)]), limits(), None).unwrap();
    let (rows, trailer) = update_of(&bytes, source.len());
    assert_eq!(rows, [(3, true)]);
    assert!(
        trailer.starts_with("trailer\n<</Size 10/Root 1 0 R"),
        "{trailer}"
    );
    let reread = Document::open_owned(
        rebuild(&bytes, RepairLimits::default()).unwrap().bytes,
        DocumentLimits::default(),
    )
    .unwrap();
    assert_eq!(
        object(&reread, 3).get(b"Marked".as_slice()),
        Some(&Object::Boolean(true))
    );
    assert!(object(&reread, 6).get(b"BaseFont".as_slice()).is_some());
}

#[test]
fn a_page_written_into_kids_signs_its_fields_but_takes_no_new_widget() {
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [3 0 R] >> >>".into(),
        "<< /Type /Pages /Kids [<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [3 0 R] >>] /Count 1 >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Sig /T (S) /Rect [72 72 272 132] >>".into(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let (reply, bytes) = prepare(&mut engine, &mut session, json!({"field": "S"}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["rect"], json!([72.0, 660.0, 272.0, 720.0]));
    assert!(object(&open(&bytes), 3).get(b"V".as_slice()).is_some());
    let (reply, _) = prepare(&mut engine, &mut session, json!({}));
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (
            &json!("invalid"),
            &json!("sign-prepare: the page is not an indirect object")
        )
    );
}

/// Each page's /Annots entries, in the order of the page tree root 2's /Kids.
fn annots_by_page(doc: &Document<'_>) -> Vec<Vec<Object>> {
    let kids = doc
        .resolve(object(doc, 2).get(b"Kids".as_slice()).unwrap())
        .as_array()
        .unwrap();
    kids.iter()
        .map(|kid| {
            (dict(doc, kid).get(b"Annots".as_slice()))
                .and_then(|annots| doc.resolve(annots).as_array())
                .map(<[Object]>::to_vec)
                .unwrap_or_default()
        })
        .collect()
}

/// sign-list's field name and page for each entry, on a session opened on
/// `bytes`.
fn signed_pages(bytes: &[u8]) -> Vec<(String, u64)> {
    let (mut engine, mut session) = session_with(bytes.to_vec());
    let (reply, _) = call(
        &mut engine,
        &mut session,
        json!({"op": "sign-list"}),
        Vec::new(),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    (reply["signatures"].as_array().unwrap().iter())
        .map(|entry| {
            let field = entry["field"].as_str().unwrap().to_string();
            (field, entry["page"].as_u64().unwrap())
        })
        .collect()
}

#[test]
fn a_new_widget_on_pages_sharing_an_annots_array_lands_on_its_own_page_alone() {
    // Pages 3, 4 and 5 share array 6, which holds note 7.
    let page = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots 6 0 R >>";
    let unsigned = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>".into(),
        page.into(),
        page.into(),
        page.into(),
        "[7 0 R]".into(),
        "<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /Contents (note) >>".into(),
    ]);
    let note = Object::Reference(id(7));
    let (mut engine, mut session) = session_with(unsigned);
    let (reply, bytes) = prepare(
        &mut engine,
        &mut session,
        json!({"page": 1, "rect": [100, 100, 300, 160]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let doc = open(&bytes);
    let widget = fields(&doc).to_vec();
    let [widget] = widget.as_slice() else {
        panic!("one field: {widget:?}");
    };
    // The page signed gets its own array (copy on write); the
    // shared one is unchanged and still the other pages'.
    assert_eq!(
        annots_by_page(&doc),
        [
            vec![note.clone()],
            vec![note.clone(), widget.clone()],
            vec![note.clone()]
        ]
    );
    for page in [3, 5] {
        let annots = object(&doc, page).get(b"Annots".as_slice());
        assert_eq!(annots, Some(&Object::Reference(id(6))));
    }
    assert_eq!(doc.resolve_id(id(6)), &Object::array([note.clone()]));
    assert_eq!(signed_pages(&bytes), [("Signature1".to_string(), 2)]);

    // A signed source: pages 3 and 8 share array 9, which holds the signed
    // widget 4; the update rewrites page 3, not the array.
    let mut objects = signed_objects();
    objects[1] = "<< /Type /Pages /Kids [3 0 R 8 0 R] /Count 2 >>".into();
    objects[2] = objects[2].replace("/Annots [4 0 R]", "/Annots 9 0 R");
    objects.push("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots 9 0 R >>".into());
    objects.push("[4 0 R]".into());
    let source = classic(&objects);
    let (mut engine, mut session) = session_with(source.clone());
    let (reply, bytes) = prepare(
        &mut engine,
        &mut session,
        json!({"rect": [100, 100, 300, 160]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(&bytes[..source.len()], source.as_slice());
    let (rows, _) = update_of(&bytes, source.len());
    let numbers: Vec<u32> = rows.iter().map(|(n, _)| *n).collect();
    assert!(numbers.contains(&3) && !numbers.contains(&9), "{numbers:?}");
    let doc = open(&bytes);
    let signed = Object::Reference(id(4));
    let new = fields(&doc).last().unwrap().clone();
    assert_eq!(
        annots_by_page(&doc),
        [vec![signed.clone(), new], vec![signed.clone()]]
    );
    assert_eq!(
        object(&doc, 8).get(b"Annots".as_slice()),
        Some(&Object::Reference(id(9)))
    );
    // Sig1 is on both pages (the last one wins), the new one on page 1.
    assert_eq!(
        signed_pages(&bytes),
        [("Sig1".to_string(), 2), ("Signature1".to_string(), 1)]
    );
}

#[test]
fn a_new_widget_goes_once_into_an_annots_array_that_is_also_the_form_s_fields() {
    // Page 3's /Annots and the form's /Fields are array 4, holding field 5.
    let source = classic(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields 4 0 R >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots 4 0 R >>".into(),
        "[5 0 R]".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (t) /Rect [72 72 172 92] /P 3 0 R >>".into(),
    ]);
    let text_field = Object::Reference(id(5));
    let (mut engine, mut session) = session_with(source.clone());
    let (reply, bytes) = prepare(
        &mut engine,
        &mut session,
        json!({"rect": [100, 100, 300, 160], "certify": true}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let doc = open(&bytes);
    let first = fields(&doc).last().unwrap().clone();
    assert_eq!(fields(&doc), [text_field.clone(), first.clone()]);
    assert_eq!(
        annots_by_page(&doc),
        [vec![text_field.clone(), first.clone()]]
    );
    assert_eq!(signed_pages(&bytes), [("Signature1".to_string(), 1)]);

    // A second signature on the certified copy, as an update.
    let (mut engine, mut certified) = session_with(bytes.clone());
    let (reply, second) = prepare(&mut engine, &mut certified, json!({}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["revisions"], 2);
    let doc = open(&second);
    let last = fields(&doc).last().unwrap().clone();
    assert_eq!(
        fields(&doc),
        [text_field.clone(), first.clone(), last.clone()]
    );
    assert_eq!(
        annots_by_page(&doc),
        [vec![text_field.clone(), first, last]]
    );
    assert_eq!(
        signed_pages(&second),
        [("Signature1".to_string(), 1), ("Signature2".to_string(), 1)]
    );

    // Two pages share the array that is also /Fields: the form gets its own
    // copy too, so the widget is on the page signed alone.
    let page = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots 4 0 R >>";
    let source = classic(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields 4 0 R >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 2 >>".into(),
        page.into(),
        "[5 0 R]".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (t) /Rect [72 72 172 92] /P 3 0 R >>".into(),
        page.into(),
    ]);
    let (mut engine, mut session) = session_with(source);
    let (reply, bytes) = prepare(
        &mut engine,
        &mut session,
        json!({"page": 1, "rect": [100, 100, 300, 160]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let doc = open(&bytes);
    let new = fields(&doc).last().unwrap().clone();
    assert_eq!(fields(&doc), [text_field.clone(), new.clone()]);
    assert_eq!(
        annots_by_page(&doc),
        [vec![text_field.clone()], vec![text_field.clone(), new]]
    );
    assert_eq!(doc.resolve_id(id(4)), &Object::array([text_field]));
    assert_eq!(signed_pages(&bytes), [("Signature1".to_string(), 2)]);
}

fn tail(clean: &[u8], length: usize) -> Vec<u8> {
    [clean, &vec![b'x'; length]].concat()
}

/// The shortest tail after %%EOF at which the writer's locator no longer
/// finds the last `startxref`.
fn locator_limit(clean: &[u8]) -> usize {
    let mut limit = 0;
    while startxref(&tail(clean, limit)).is_some() {
        limit += 1;
        assert!(limit < 1 << 16, "the locator searches the whole file");
    }
    assert!(limit > 0);
    limit
}

#[test]
#[ignore = "clean-room: spec 03 section 4.3 (the reader uses the writer's locator)"]
fn a_tail_just_inside_the_locator_takes_an_update() {
    let clean = classic(&signed_objects());
    let limit = locator_limit(&clean);
    for length in [limit / 2, limit - 1] {
        let (mut engine, mut session) = session_with(tail(&clean, length));
        let (reply, _) = prepare(&mut engine, &mut session, json!({}));
        assert_eq!(reply["ok"], true, "{length}: {reply}");
    }
}

#[test]
fn a_signed_source_with_a_long_blank_tail_takes_an_update_after_it() {
    let clean = classic(&signed_objects());
    let xref = find(&clean, b"\nxref\n")[0] + 1;
    // The strict reader reads past trailing whitespace, NUL included.
    let source = [clean.as_slice(), &[0u8; 3000]].concat();
    let (mut engine, mut session) = session_with(source.clone());
    assert!(!session.info().unwrap().repaired);
    let (reply, bytes) = prepare(&mut engine, &mut session, json!({}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(&bytes[..source.len()], source.as_slice());
    assert_eq!(reply["revisions"], 2);
    let (_, trailer) = update_of(&bytes, source.len());
    assert!(trailer.contains(&format!("/Prev {xref}>>")), "{trailer}");
    let (mut engine, mut reopened) = session_with(bytes);
    let info = reopened.info().unwrap();
    assert!(!info.repaired && info.signatures == 2);
    assert_eq!(listed(&mut engine, &mut reopened), (json!(2), 2));

    // Text after %%EOF: a short tail takes the update; once the writer's
    // locator no longer finds the last `startxref`, the documented refusal.
    // The case just inside the locator's reach is
    // `a_tail_just_inside_the_locator_takes_an_update` (spec 03 section 4.3).
    let limit = locator_limit(&clean);
    for length in [0, 1000, limit, limit + 1, limit + 4, limit + 8, limit + 16] {
        let (mut engine, mut session) = session_with(tail(&clean, length));
        let (reply, _) = prepare(&mut engine, &mut session, json!({}));
        if length < limit {
            assert_eq!(reply["ok"], true, "{length}: {reply}");
            continue;
        }
        assert_eq!(reply["reason"], "invalid", "{length}: {reply}");
        assert!(
            reply["error"]
                .as_str()
                .unwrap()
                .starts_with("sign-prepare: the signed file needs repair"),
            "{length}: {reply}"
        );
    }
    let text = [clean.as_slice(), "junk ".repeat(limit / 5 + 1).as_bytes()].concat();
    let (mut engine, mut session) = session_with(text);
    let (reply, _) = prepare(&mut engine, &mut session, json!({}));
    assert_eq!(reply["reason"], "invalid");
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .starts_with("sign-prepare: the signed file needs repair"),
        "{reply}"
    );
}
