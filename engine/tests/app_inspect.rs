// SPDX-License-Identifier: MIT
//! The `inspect` op (feature `inspect`): the test suite's raw reader of the
//! open document, in place of a second PDF library reopening the engine's
//! output. Summary, page dictionary, objects and streams, text, the compare
//! slot, and every refusal.
use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession};

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
        bytes.extend_from_slice(body.as_bytes());
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

fn dispatch(
    engine: &mut AppEngine,
    session: &mut AppSession,
    request: Value,
    buffers: Vec<Vec<u8>>,
) -> (Value, Vec<Vec<u8>>) {
    engine.dispatch(session, request, buffers)
}

fn open(engine: &mut AppEngine, session: &mut AppSession, bytes: Vec<u8>) {
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = dispatch(engine, session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
}

fn inspect(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    let (reply, outputs) = dispatch(engine, session, request, Vec::new());
    assert!(outputs.is_empty(), "no output buffers without `data`");
    reply
}

/// A one-page document whose page dictionary has a `$`-prefixed key, a
/// string, a real and a nested direct dictionary.
fn odd_page() -> Vec<u8> {
    let content = "BT /F1 24 Tf 72 700 Td (odd) Tj ET";
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300.5 200] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> /$marker (hi) /Note <FEFF00480069> \
         /Flag true /Nothing null >>"
            .into(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len() + 1
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ])
}

#[test]
fn the_summary_names_the_pages_trailer_info_and_objects() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    open(&mut engine, &mut session, fixture("hello.pdf"));
    let reply = inspect(&mut engine, &mut session, json!({"id": 2, "op": "inspect"}));
    assert_eq!(reply["id"], 2);
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["pageCount"], 1);
    assert_eq!(reply["locked"], false);
    assert_eq!(reply["repaired"], false);
    assert_eq!(reply["version"], "1.4", "hello.pdf declares 1.4");
    assert_eq!(reply["info"]["encrypted"], false);
    assert_eq!(reply["info"]["permissions"]["copy"], true);
    // The trailer's /Root is a reference, left for the reader to follow.
    let root = &reply["trailer"]["Root"]["ref"];
    assert!(root.is_array(), "{reply}");
    let pages = reply["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 1);
    assert!(pages[0].is_array(), "page 0 is an indirect object");
    let objects = reply["objects"].as_array().unwrap();
    assert!(
        objects.contains(&pages[0]) && objects.contains(root),
        "every object is listed: {objects:?}"
    );
    assert!(
        objects
            .windows(2)
            .all(|pair| pair[0][0].as_u64() < pair[1][0].as_u64()),
        "ascending"
    );
}

#[test]
fn a_page_dictionary_comes_back_raw_with_every_value_kind_tagged() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    open(&mut engine, &mut session, odd_page());
    let reply = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "page": 0}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["page"], 0);
    assert_eq!(reply["object"], json!([3, 0]));
    let dict = &reply["value"]["dict"];
    assert_eq!(dict["Type"], json!({"name": "Page"}));
    assert_eq!(dict["Parent"], json!({"ref": [2, 0]}));
    assert_eq!(dict["MediaBox"], json!([0, 0, 300.5, 200]));
    assert_eq!(dict["Flag"], true);
    assert_eq!(dict["Nothing"], Value::Null, "a null entry reads as null");
    assert_eq!(
        dict["Resources"]["dict"]["Font"]["dict"]["F1"],
        json!({"ref": [5, 0]}),
        "direct dictionaries nest"
    );
    assert_eq!(
        dict["Note"],
        json!({"string": "\u{fe}\u{ff}\0H\0i", "text": "Hi"})
    );
    // A key beginning with `$` travels escaped, as every data key does.
    assert_eq!(dict["$$marker"], json!({"string": "hi", "text": "hi"}));
    assert!(dict.get("$marker").is_none());

    let out = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "page": 1}),
    );
    assert_eq!(out["ok"], false);
    assert_eq!(out["error"], "inspect: page 1 is out of range (0..0)");
    let bad = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "page": "x"}),
    );
    assert_eq!(bad["error"], "inspect: page x is out of range (0..0)");
}

#[test]
fn an_object_reads_by_id_and_a_stream_hands_its_bytes_out() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    open(&mut engine, &mut session, odd_page());
    let font = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "object": [5, 0]}),
    );
    assert_eq!(font["ok"], true, "{font}");
    assert_eq!(font["object"], json!([5, 0]));
    assert_eq!(
        font["value"]["dict"]["BaseFont"],
        json!({"name": "Helvetica"})
    );

    let (stream, outputs) = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "object": [4, 0], "data": "decoded"}),
        Vec::new(),
    );
    assert_eq!(stream["ok"], true, "{stream}");
    assert!(stream["value"]["stream"]["Length"].is_number(), "{stream}");
    assert_eq!(stream["data"], json!({"$buffer": 0}));
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0], b"BT /F1 24 Tf 72 700 Td (odd) Tj ET\n");
    assert_eq!(
        stream["value"]["length"],
        outputs[0].len(),
        "unfiltered: raw is decoded"
    );
    let (raw, outputs) = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "object": [4, 0], "data": "raw"}),
        Vec::new(),
    );
    assert_eq!(raw["ok"], true, "{raw}");
    assert_eq!(outputs[0], b"BT /F1 24 Tf 72 700 Td (odd) Tj ET\n");

    // hello.pdf's content stream is Flate-compressed: decoded differs from raw.
    let mut other = AppSession::default();
    open(&mut engine, &mut other, fixture("hello.pdf"));
    let page = inspect(&mut engine, &mut other, json!({"op": "inspect", "page": 0}));
    let contents = page["value"]["dict"]["Contents"]["ref"].clone();
    let (decoded, data) = dispatch(
        &mut engine,
        &mut other,
        json!({"op": "inspect", "object": contents, "data": "decoded"}),
        Vec::new(),
    );
    assert_eq!(decoded["ok"], true, "{decoded}");
    assert!(
        String::from_utf8_lossy(&data[0]).contains("PDF4All"),
        "the decoded content shows the page's text"
    );

    let missing = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "object": [99, 0]}),
    );
    assert_eq!(missing["ok"], true, "{missing}");
    assert_eq!(missing["value"], Value::Null);
    assert_eq!(missing["missing"], true);
    let not_stream = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "object": [5, 0], "data": "decoded"}),
    );
    assert_eq!(not_stream["error"], "inspect: object 5 0 is not a stream");
    for id in [
        json!(5),
        json!([0, 0]),
        json!([5]),
        json!(["5", 0]),
        json!([5, -1]),
    ] {
        let bad = inspect(
            &mut engine,
            &mut session,
            json!({"op": "inspect", "object": id}),
        );
        assert_eq!(
            bad["error"], "inspect: object must be [number, generation]",
            "{id}"
        );
    }
    let bad = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "object": [4, 0], "data": "hex"}),
    );
    assert_eq!(bad["error"], "inspect: data must be \"decoded\" or \"raw\"");
}

#[test]
fn text_lists_the_lines_and_on_request_every_character() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    open(&mut engine, &mut session, fixture("hello.pdf"));
    let reply = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "text": 0}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["page"], 0);
    assert_eq!(reply["text"], "PDF4All\n\n");
    let lines = reply["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["text"], "PDF4All");
    assert_eq!(lines[0]["size"], 36);
    assert!(lines[0]["top"].as_i64().unwrap() < lines[0]["y"].as_i64().unwrap());
    assert!(reply.get("chars").is_none());

    let with = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "text": 0, "chars": true}),
    );
    let chars = with["chars"].as_array().unwrap();
    let word: String = chars.iter().map(|c| c["c"].as_str().unwrap()).collect();
    assert_eq!(word, "PDF4All");
    assert!(
        chars
            .iter()
            .all(|c| c["x"].is_number() && c["y"].is_number() && c["size"].is_number())
    );
    assert_eq!(chars[0]["quad"].as_array().unwrap().len(), 8);
    assert!(
        chars[0]["x"].as_f64().unwrap() < chars[6]["x"].as_f64().unwrap(),
        "left to right"
    );

    let bad = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "text": 0, "chars": 1}),
    );
    assert_eq!(bad["error"], "inspect: chars must be true or false");
    let out = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "text": 3}),
    );
    assert_eq!(out["error"], "inspect: page 3 is out of range (0..0)");
}

#[test]
fn a_locked_document_shows_its_structure_but_no_text() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    open(&mut engine, &mut session, fixture("encrypted.pdf"));
    assert!(session.locked);
    let reply = inspect(&mut engine, &mut session, json!({"op": "inspect"}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["locked"], true);
    assert_eq!(reply["info"]["needsPassword"], true);
    assert_eq!(reply["info"]["encrypted"], true);
    assert!(
        reply["trailer"]["Encrypt"].is_object(),
        "the /Encrypt entry is readable"
    );
    let page = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "page": 0}),
    );
    assert_eq!(page["ok"], true, "the page tree is readable while locked");
    let text = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "text": 0}),
    );
    assert_eq!(
        text["error"],
        "inspect: document is encrypted, a password is required"
    );
}

#[test]
fn the_compare_slot_is_read_with_compare_true() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let none = inspect(&mut engine, &mut session, json!({"op": "inspect"}));
    assert_eq!(none["error"], "inspect: no document open");
    let no_slot = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "compare": true}),
    );
    assert_eq!(no_slot["error"], "inspect: no compare document is open");

    open(&mut engine, &mut session, fixture("hello.pdf"));
    let request = json!({"op": "compare-open", "name": "b.pdf", "pdf": {"$buffer": 0}});
    let (opened, _) = dispatch(&mut engine, &mut session, request, vec![fixture("nav.pdf")]);
    assert_eq!(opened["ok"], true, "{opened}");
    let main = inspect(&mut engine, &mut session, json!({"op": "inspect"}));
    let other = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "compare": true}),
    );
    assert_eq!(main["pageCount"], 1);
    assert_eq!(other["pageCount"], 3);
    let text = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "compare": true, "text": 1}),
    );
    assert_eq!(text["ok"], true, "{text}");
    assert!(text["text"].as_str().unwrap().contains("two"), "{text}");
    // `compare` must be exactly true; anything else reads the main document.
    let loose = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "compare": 1}),
    );
    assert_eq!(loose["pageCount"], 1);
}

#[test]
fn inspect_changes_nothing() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    open(&mut engine, &mut session, fixture("hello.pdf"));
    let before = session.journal_state();
    let page = inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "page": 0}),
    );
    let contents = page["value"]["dict"]["Contents"]["ref"].clone();
    dispatch(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "object": contents, "data": "decoded"}),
        Vec::new(),
    );
    inspect(
        &mut engine,
        &mut session,
        json!({"op": "inspect", "text": 0, "chars": true}),
    );
    assert_eq!(session.journal_state(), before);
    assert_eq!(session.revision, 0);
}
