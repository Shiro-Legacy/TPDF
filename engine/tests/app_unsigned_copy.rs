// SPDX-License-Identifier: MIT
//! `unsigned-copy`: a signed (read-only) document's bytes without its
//! signatures, for the app's "Make an editable copy". The copy keeps the
//! form values, annotations and its signature fields (empty), has no
//! signature value, certification or validation data, and opens editable.
use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::document::{Document, DocumentLimits};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("../fixtures/{name}")).unwrap()
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

/// The copy and the reply.
fn copy(engine: &mut AppEngine, session: &mut AppSession) -> (Value, Vec<u8>) {
    let request = json!({"id": 7, "op": "unsigned-copy", "entropy": {"$buffer": 0}});
    let (reply, mut outputs) = engine.dispatch(session, request, vec![vec![5u8; 32]]);
    assert_eq!(reply["ok"], true, "{reply}");
    (reply, outputs.remove(0))
}

/// Field name → value, from the `fields` op.
fn values(engine: &mut AppEngine, session: &mut AppSession) -> Vec<(String, Value)> {
    let reply = ok(engine, session, json!({"op": "fields", "page": 0}));
    let mut out: Vec<(String, Value)> = (reply["items"].as_array().unwrap().iter())
        .map(|item| {
            (
                item["name"].as_str().unwrap_or("").to_string(),
                item["value"].clone(),
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn kinds(engine: &mut AppEngine, session: &mut AppSession) -> Vec<(String, Value)> {
    let reply = ok(engine, session, json!({"op": "fields", "page": 0}));
    let mut out: Vec<(String, Value)> = (reply["items"].as_array().unwrap().iter())
        .map(|item| {
            (
                item["name"].as_str().unwrap_or("").to_string(),
                item["kind"].clone(),
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn holds(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// No object of the copy is a signature value or time stamp.
fn no_signature_objects(bytes: &[u8]) {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    for object in doc.objects() {
        if let Some(dict) = object.value.as_dict() {
            assert!(
                dict.get(b"ByteRange".as_slice()).is_none(),
                "{:?}",
                object.id
            );
            assert_ne!(
                doc.dict_get(dict, b"Type").as_name(),
                Some(b"DocTimeStamp".as_slice())
            );
        }
    }
    let root = doc.resolve_id(doc.root()).as_dict().unwrap();
    assert!(root.get(b"Perms".as_slice()).is_none());
    assert!(root.get(b"DSS".as_slice()).is_none());
    assert_eq!(doc.trailer_chain().count(), 1, "one full revision");
}

#[test]
fn a_signed_file_becomes_an_editable_copy_with_an_empty_signature_field() {
    let (mut engine, mut session) = session_with(fixture("p0b/signed-intact.pdf"));
    assert!(session.info().unwrap().signed());
    let before = kinds(&mut engine, &mut session);
    let revision = session.revision;
    let (reply, bytes) = copy(&mut engine, &mut session);
    assert_eq!(reply["signatures"], 1);
    no_signature_objects(&bytes);
    assert!(!holds(&bytes, b"/ByteRange"));
    // Sig1 stays as an empty signature field: no value, no lock, no look.
    {
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let sig = (doc.objects())
            .filter_map(|o| o.value.as_dict())
            .find(|d| doc.dict_get(d, b"FT").as_name() == Some(b"Sig".as_slice()))
            .expect("the signature field stays");
        for key in [b"V".as_slice(), b"Lock", b"AP"] {
            assert!(sig.get(key).is_none(), "{}", String::from_utf8_lossy(key));
        }
    }
    // The session's document is unchanged: still signed, still read-only.
    assert!(session.info().unwrap().signed());
    assert_eq!(session.revision, revision);
    // The copy opens editable and keeps its fields, the signature one empty.
    let (mut engine, mut session) = session_with(bytes);
    let info = session.info().unwrap();
    assert!(!info.signed() && !info.certified && info.signatures == 0);
    assert_eq!(kinds(&mut engine, &mut session), before);
    let fields = ok(
        &mut engine,
        &mut session,
        json!({"op": "fields", "page": 0}),
    );
    let widget = (fields["items"].as_array().unwrap().iter())
        .find(|item| item["name"] == "name")
        .map(|item| item["id"].clone())
        .unwrap();
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-set", "page": 0, "widgetId": widget, "value": "Edited"}),
    );
    ok(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "Text", "point": [100, 100], "contents": "note"}),
    );
}

#[test]
fn a_certified_and_filled_file_keeps_its_form_values() {
    let (mut engine, mut session) = session_with(fixture("p0b/certified-p2-filled.pdf"));
    let info = session.info().unwrap();
    assert!(info.certified);
    let before = values(&mut engine, &mut session);
    assert!(
        before
            .iter()
            .any(|(_, v)| v.as_str().is_some_and(|s| !s.is_empty())),
        "the fixture has a filled value: {before:?}"
    );
    let (_, bytes) = copy(&mut engine, &mut session);
    no_signature_objects(&bytes);
    let (mut engine, mut session) = session_with(bytes);
    let info = session.info().unwrap();
    assert!(!info.signed() && !info.certified);
    // The signature field's value was the signature; every other value stays.
    let after = values(&mut engine, &mut session);
    let unsigned: Vec<_> = before
        .iter()
        .filter(|(name, _)| name != "Sig1")
        .cloned()
        .collect();
    let kept: Vec<_> = after
        .iter()
        .filter(|(name, _)| name != "Sig1")
        .cloned()
        .collect();
    assert_eq!(kept, unsigned);
}

#[test]
fn a_file_with_an_annotation_after_certification_keeps_it() {
    let (mut engine, mut session) = session_with(fixture("p0b/certified-p2-annot.pdf"));
    let before = ok(
        &mut engine,
        &mut session,
        json!({"op": "annots", "page": 0}),
    );
    let (_, bytes) = copy(&mut engine, &mut session);
    let (mut engine, mut session) = session_with(bytes);
    let after = ok(
        &mut engine,
        &mut session,
        json!({"op": "annots", "page": 0}),
    );
    assert_eq!(
        before["items"].as_array().map(Vec::len),
        after["items"].as_array().map(Vec::len)
    );
    assert!(!session.info().unwrap().signed());
}

#[test]
fn an_unsigned_file_or_none_is_refused() {
    let (mut engine, mut session) = session_with(fixture("hello.pdf"));
    let reply = op(&mut engine, &mut session, json!({"op": "unsigned-copy"}));
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["reason"], "not-signed", "{reply}");
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let reply = op(&mut engine, &mut session, json!({"op": "unsigned-copy"}));
    assert_eq!(reply["reason"], "no-document", "{reply}");
}

#[test]
fn a_tampered_signature_goes_too() {
    let (mut engine, mut session) = session_with(fixture("p0b/signed-tampered.pdf"));
    let (_, bytes) = copy(&mut engine, &mut session);
    no_signature_objects(&bytes);
    let (_, session) = session_with(bytes);
    assert!(!session.info().unwrap().signed());
}
