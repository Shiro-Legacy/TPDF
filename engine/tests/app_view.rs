// SPDX-License-Identifier: MIT
//! The view ops on AppSession: render, thumbnail, search, text-all, layers
//! and layer-set, with the earlier app engine's checks, messages and reply fields, and
//! the layers panel's view-only visibility (ISO 32000-2 §8.11).
use md5::{Digest, Md5};
use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::object::ObjectId;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("../fixtures/{name}")).unwrap()
}

/// A classic-xref PDF from object bodies numbered from 1.
fn classic(objects: &[String], trailer: &str) -> Vec<u8> {
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
            "trailer\n<< /Size {} /Root 1 0 R {trailer} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn stream(dictionary: &str, data: &str) -> String {
    format!(
        "<< {dictionary} /Length {} >>\nstream\n{data}\nendstream",
        data.len() + 1
    )
}

/// Catalog 1, pages 2, font 3, then page `4 + 2i` with content `5 + 2i`.
fn text_pages(contents: &[&str], media: &str) -> Vec<u8> {
    let kids: Vec<String> = (0..contents.len())
        .map(|i| format!("{} 0 R", 4 + 2 * i))
        .collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            contents.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_string(),
    ];
    for (i, content) in contents.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox {media} \
             /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            5 + 2 * i
        ));
        objects.push(stream("", content));
    }
    classic(&objects, "")
}

fn pages_pdf(n: usize) -> Vec<u8> {
    let contents: Vec<String> = (0..n)
        .map(|i| format!("BT /F1 24 Tf 72 700 Td (Page {i}) Tj ET"))
        .collect();
    let contents: Vec<&str> = contents.iter().map(String::as_str).collect();
    text_pages(&contents, "[0 0 612 792]")
}

fn open(engine: &mut AppEngine, session: &mut AppSession, bytes: Vec<u8>) -> Value {
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
    reply
}

fn session_with(bytes: Vec<u8>) -> (AppEngine, AppSession) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    open(&mut engine, &mut session, bytes);
    (engine, session)
}

/// One request; the reply and its output buffers.
fn call(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> (Value, Vec<Vec<u8>>) {
    engine.dispatch(session, request, Vec::new())
}

fn op(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    call(engine, session, request).0
}

/// The error of a refused request, checking it is plain and drops outputs.
fn refused(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> String {
    let (reply, outputs) = call(engine, session, request.clone());
    assert_eq!(reply["ok"], false, "{request} -> {reply}");
    assert!(outputs.is_empty(), "{request}: a refusal transfers nothing");
    assert!(
        reply.get("reason").is_none(),
        "{request}: plain error {reply}"
    );
    for key in ["revision", "dirty", "canUndo", "canRedo"] {
        assert!(reply.get(key).is_some(), "{request}: journal slice {reply}");
    }
    reply["error"].as_str().unwrap().to_string()
}

fn render_text(engine: &mut AppEngine, session: &mut AppSession, page: usize) -> String {
    let reply = op(
        engine,
        session,
        json!({"op": "render", "page": page, "scale": 1}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    reply["text"]
        .as_str()
        .unwrap()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn non_white(pixels: &[u8]) -> usize {
    (pixels.chunks_exact(4))
        .filter(|px| px[..3] != [255, 255, 255])
        .count()
}

// ---------------------------------------------------------------- render

#[test]
fn render_replies_with_pixels_text_lines_and_counts() {
    let (mut engine, mut session) = session_with(pages_pdf(2));
    let (reply, outputs) = call(
        &mut engine,
        &mut session,
        json!({"id": 7, "op": "render", "page": 1, "scale": 1}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["id"], 7);
    assert_eq!(reply["page"], 1);
    assert_eq!(reply["revision"], 0);
    assert_eq!(
        (reply["width"].as_f64(), reply["height"].as_f64()),
        (Some(612.0), Some(792.0))
    );
    assert_eq!(
        (reply["pixelWidth"].as_u64(), reply["pixelHeight"].as_u64()),
        (Some(612), Some(792))
    );
    assert_eq!(reply["pixels"], json!({"$buffer": 0}));
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].len(), 612 * 792 * 4);
    assert!(outputs[0].chunks_exact(4).all(|px| px[3] == 255), "opaque");
    let ink = reply["nonWhite"].as_u64().unwrap() as usize;
    assert_eq!(ink, non_white(&outputs[0]));
    assert!(ink > 100 && ink < 612 * 792 / 2, "{ink}");
    assert_eq!(reply["text"], "Page 1\n\n");
    let lines = reply["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 1);
    let line = &lines[0];
    assert_eq!(line["text"], "Page 1");
    assert_eq!(
        (
            line["x"].as_i64(),
            line["y"].as_i64(),
            line["size"].as_i64()
        ),
        (Some(72), Some(92), Some(24))
    );
    assert_eq!(
        (
            line["family"].as_str(),
            line["weight"].as_str(),
            line["style"].as_str()
        ),
        (Some("sans-serif"), Some("normal"), Some("normal"))
    );
    assert!(
        line["w"].as_i64().unwrap() > 60 && line["h"].as_i64().unwrap() > 20,
        "{line}"
    );
    // Timings are the adapter's to fill.
    assert_eq!(
        (reply["paintMs"].as_u64(), reply["renderMs"].as_u64()),
        (Some(0), Some(0))
    );

    let doubled = op(
        &mut engine,
        &mut session,
        json!({"op": "render", "page": 0, "scale": 2}),
    );
    assert_eq!(
        (
            doubled["pixelWidth"].as_u64(),
            doubled["pixelHeight"].as_u64()
        ),
        (Some(1224), Some(1584))
    );
    let quarter = op(
        &mut engine,
        &mut session,
        json!({"op": "render", "page": 0, "scale": 0.25}),
    );
    assert_eq!(
        (
            quarter["pixelWidth"].as_u64(),
            quarter["pixelHeight"].as_u64()
        ),
        (Some(153), Some(198))
    );
    assert_eq!(session.revision, 0, "rendering changes nothing");
    assert!(!session.journal_state().dirty);
}

#[test]
fn render_and_thumbnail_refuse_as_engine_ts_does() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    for (request, message) in [
        (
            json!({"op": "render", "page": 0, "scale": 1}),
            "render: no document open",
        ),
        (
            json!({"op": "thumbnail", "page": 0, "maxEdge": 256}),
            "thumbnail: no document open",
        ),
        (
            json!({"op": "search", "query": "x"}),
            "search: no document open",
        ),
        (
            json!({"op": "search", "query": ""}),
            "search: no document open",
        ),
        (json!({"op": "text-all"}), "text-all: no document open"),
        (json!({"op": "layers"}), "layers: no document open"),
        (
            json!({"op": "layer-set", "layerId": 0, "visible": true}),
            "layer-set: no document open",
        ),
    ] {
        assert_eq!(refused(&mut engine, &mut session, request), message);
    }

    open(&mut engine, &mut session, pages_pdf(2));
    let nan = json!({"$number": "NaN"});
    for (request, message) in [
        (
            json!({"op": "render", "page": 1.5, "scale": 1}),
            "render: page must be an integer",
        ),
        (
            json!({"op": "render", "page": nan, "scale": 1}),
            "render: page must be an integer",
        ),
        (
            json!({"op": "render", "page": "0", "scale": 1}),
            "render: page must be an integer",
        ),
        (
            json!({"op": "render", "scale": 1}),
            "render: page must be an integer",
        ),
        // The page type is checked before the scale.
        (
            json!({"op": "render", "page": null, "scale": 9}),
            "render: page must be an integer",
        ),
        (
            json!({"op": "render", "page": 0, "scale": 0.1}),
            "render: scale must be between 0.25 and 4",
        ),
        (
            json!({"op": "render", "page": 0, "scale": 4.0001}),
            "render: scale must be between 0.25 and 4",
        ),
        (
            json!({"op": "render", "page": 0, "scale": nan}),
            "render: scale must be between 0.25 and 4",
        ),
        (
            json!({"op": "render", "page": 0, "scale": "1"}),
            "render: scale must be between 0.25 and 4",
        ),
        (
            json!({"op": "render", "page": 0}),
            "render: scale must be between 0.25 and 4",
        ),
        // The scale is checked before the range.
        (
            json!({"op": "render", "page": 9, "scale": 9}),
            "render: scale must be between 0.25 and 4",
        ),
        (
            json!({"op": "render", "page": 2, "scale": 1}),
            "render: page 2 is out of range (0..1)",
        ),
        (
            json!({"op": "render", "page": -1, "scale": 1}),
            "render: page -1 is out of range (0..1)",
        ),
        (
            json!({"op": "render", "page": 1e21, "scale": 1}),
            "render: page 1e+21 is out of range (0..1)",
        ),
        (
            json!({"op": "thumbnail", "page": 0.5, "maxEdge": 256}),
            "thumbnail: page must be an integer",
        ),
        (
            json!({"op": "thumbnail", "page": 0, "maxEdge": 31}),
            "thumbnail: maxEdge must be an integer between 32 and 256",
        ),
        (
            json!({"op": "thumbnail", "page": 0, "maxEdge": 257}),
            "thumbnail: maxEdge must be an integer between 32 and 256",
        ),
        (
            json!({"op": "thumbnail", "page": 0, "maxEdge": 32.5}),
            "thumbnail: maxEdge must be an integer between 32 and 256",
        ),
        (
            json!({"op": "thumbnail", "page": 0, "maxEdge": "256"}),
            "thumbnail: maxEdge must be an integer between 32 and 256",
        ),
        (
            json!({"op": "thumbnail", "page": 0}),
            "thumbnail: maxEdge must be an integer between 32 and 256",
        ),
        (
            json!({"op": "thumbnail", "page": 2, "maxEdge": 32}),
            "thumbnail: page 2 is out of range (0..1)",
        ),
    ] {
        assert_eq!(refused(&mut engine, &mut session, request), message);
    }

    // The edge limit is checked on the wanted size before drawing.
    open(
        &mut engine,
        &mut session,
        text_pages(&[""], "[0 0 2000 1200]"),
    );
    assert_eq!(
        refused(
            &mut engine,
            &mut session,
            json!({"op": "render", "page": 0, "scale": 2.5})
        ),
        "render: 5000×3000 px is over the 4096 px edge limit"
    );
    assert_eq!(
        refused(
            &mut engine,
            &mut session,
            json!({"op": "render", "page": 0, "scale": 2.0481})
        ),
        "render: 4097×2458 px is over the 4096 px edge limit"
    );
    let edge = op(
        &mut engine,
        &mut session,
        json!({"op": "render", "page": 0, "scale": 2.048}),
    );
    assert_eq!(
        (edge["pixelWidth"].as_u64(), edge["pixelHeight"].as_u64()),
        (Some(4096), Some(2458))
    );

    open(&mut engine, &mut session, fixture("encrypted.pdf"));
    assert!(session.locked);
    for (op, request) in [
        ("render", json!({"op": "render", "page": 0, "scale": 1})),
        (
            "thumbnail",
            json!({"op": "thumbnail", "page": 0, "maxEdge": 64}),
        ),
        ("search", json!({"op": "search", "query": "x"})),
        ("text-all", json!({"op": "text-all"})),
        ("layers", json!({"op": "layers"})),
        (
            "layer-set",
            json!({"op": "layer-set", "layerId": 0, "visible": true}),
        ),
    ] {
        assert_eq!(
            refused(&mut engine, &mut session, request),
            format!("{op}: document is encrypted, a password is required")
        );
    }
    // The query is checked before the lock.
    assert_eq!(
        refused(
            &mut engine,
            &mut session,
            json!({"op": "search", "query": " "})
        ),
        "search: query must be 1..256 characters"
    );
    let unlocked = op(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "pdf4all-user"}),
    );
    assert_eq!(unlocked["authenticated"], true, "{unlocked}");
    assert!(render_text(&mut engine, &mut session, 0).contains("PDF4All encrypted"));
}

#[test]
fn thumbnail_pins_the_long_edge_and_carries_no_text() {
    // 2000 x 1200 pt: 256 px needs scale 0.128, under MIN_SCALE.
    let (mut engine, mut session) = session_with(text_pages(&[""], "[0 0 2000 1200]"));
    let small = refused(
        &mut engine,
        &mut session,
        json!({"op": "render", "page": 0, "scale": 0.128}),
    );
    assert_eq!(small, "render: scale must be between 0.25 and 4");
    let (reply, outputs) = call(
        &mut engine,
        &mut session,
        json!({"id": 3, "op": "thumbnail", "page": 0, "maxEdge": 256}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(
        (reply["pixelWidth"].as_u64(), reply["pixelHeight"].as_u64()),
        (Some(256), Some(154))
    );
    assert_eq!(
        (reply["width"].as_f64(), reply["height"].as_f64()),
        (Some(2000.0), Some(1200.0))
    );
    assert_eq!(outputs[0].len(), 256 * 154 * 4);
    for key in ["text", "lines", "nonWhite", "renderMs"] {
        assert!(reply.get(key).is_none(), "{key}: {reply}");
    }
    // A small page is not drawn past MAX_SCALE.
    let (_, mut tiny) = session_with(text_pages(&[""], "[0 0 20 10]"));
    let reply = op(
        &mut engine,
        &mut tiny,
        json!({"op": "thumbnail", "page": 0, "maxEdge": 256}),
    );
    assert_eq!(
        (reply["pixelWidth"].as_u64(), reply["pixelHeight"].as_u64()),
        (Some(80), Some(40))
    );
    // A rotated page keeps its aspect.
    let (_, mut rotated) = session_with(fixture("rotated.pdf"));
    let reply = op(
        &mut engine,
        &mut rotated,
        json!({"op": "thumbnail", "page": 0, "maxEdge": 32}),
    );
    let (w, h) = (
        reply["width"].as_f64().unwrap(),
        reply["height"].as_f64().unwrap(),
    );
    let (pw, ph) = (
        reply["pixelWidth"].as_f64().unwrap(),
        reply["pixelHeight"].as_f64().unwrap(),
    );
    assert_eq!(pw.max(ph), 32.0);
    assert!((pw / ph - w / h).abs() < 0.05, "{reply}");
}

// ---------------------------------------------------------------- search

#[test]
fn search_trims_like_javascript_and_checks_the_query_length() {
    let (mut engine, mut session) = session_with(pages_pdf(2));
    for query in ["", "   ", "\u{FEFF}\u{3000}\n", &"a".repeat(257)] {
        assert_eq!(
            refused(
                &mut engine,
                &mut session,
                json!({"op": "search", "query": query})
            ),
            "search: query must be 1..256 characters"
        );
    }
    assert_eq!(
        refused(&mut engine, &mut session, json!({"op": "search"})),
        "search: query must be 1..256 characters"
    );
    assert_eq!(
        refused(
            &mut engine,
            &mut session,
            json!({"op": "search", "query": 7})
        ),
        "search: query must be 1..256 characters"
    );
    // 256 UTF-16 units pass; JavaScript counts an astral character as two.
    let ok = op(
        &mut engine,
        &mut session,
        json!({"op": "search", "query": "a".repeat(256)}),
    );
    assert_eq!(
        (ok["ok"].as_bool(), ok["total"].as_u64()),
        (Some(true), Some(0))
    );
    let astral = "\u{1F600}".repeat(129);
    assert_eq!(
        refused(
            &mut engine,
            &mut session,
            json!({"op": "search", "query": astral})
        ),
        "search: query must be 1..256 characters"
    );
    let trimmed = op(
        &mut engine,
        &mut session,
        json!({"op": "search", "query": "\u{FEFF}\u{A0} Page 1\t\u{2029}"}),
    );
    assert_eq!(trimmed["total"], 1, "{trimmed}");
    assert_eq!(trimmed["hits"][0]["page"], 1);
    // NEL is not JavaScript whitespace: the query is that one character.
    let nel = op(
        &mut engine,
        &mut session,
        json!({"op": "search", "query": "\u{85}"}),
    );
    assert_eq!(
        (nel["ok"].as_bool(), nel["total"].as_u64()),
        (Some(true), Some(0))
    );
    assert_eq!(nel["searchMs"], 0);
}

#[test]
fn search_hits_follow_page_order_and_the_budget() {
    let (mut engine, mut session) = session_with(pages_pdf(5));
    let hits = |reply: &Value| -> Vec<u64> {
        (reply["hits"].as_array().unwrap().iter())
            .map(|hit| hit["page"].as_u64().unwrap())
            .collect()
    };
    let three = op(
        &mut engine,
        &mut session,
        json!({"op": "search", "query": "Page", "maxHits": 3}),
    );
    assert_eq!(hits(&three), [0, 1, 2]);
    assert_eq!(
        (three["total"].as_u64(), three["truncated"].as_bool()),
        (Some(3), Some(true))
    );
    let quad = three["hits"][0]["quads"].as_array().unwrap();
    assert_eq!(quad.len(), 1);
    let quad: Vec<f64> = quad[0]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(quad.len(), 8);
    // ul, ur, ll, lr in y-down page points: x along the baseline within
    // 0.05 pt of the advances, y within 0.35 em (spec 04 §8 quad tolerance).
    let expected = [72.0, 66.2, 128.04, 66.2, 72.0, 99.176, 128.04, 99.176];
    assert!(
        quad.iter().zip(expected).enumerate().all(|(i, (a, b))| {
            let slack = if i % 2 == 0 { 0.055 } else { 0.35 * 24.0 };
            (a - b).abs() <= slack
        }),
        "{quad:?}"
    );

    // Exactly the budget with nothing after it is not truncated.
    let five = op(
        &mut engine,
        &mut session,
        json!({"op": "search", "query": "Page", "maxHits": 5}),
    );
    assert_eq!(
        (five["total"].as_u64(), five["truncated"].as_bool()),
        (Some(5), Some(false))
    );
    // A budget filled on one page is truncated by a match on a later one.
    let page = op(
        &mut engine,
        &mut session,
        json!({"op": "search", "query": "Page 0", "maxHits": 1}),
    );
    assert_eq!(
        (page["total"].as_u64(), page["truncated"].as_bool()),
        (Some(1), Some(false))
    );
    for (max_hits, total) in [
        (json!(4.9), 4),
        (json!(-3), 1),
        (json!(0), 1),
        (json!(1e9), 5),
        (json!("2"), 5),
        (json!(null), 5),
        (json!({"$number": "Infinity"}), 5),
    ] {
        let reply = op(
            &mut engine,
            &mut session,
            json!({"op": "search", "query": "Page", "maxHits": max_hits}),
        );
        assert_eq!(reply["total"], total, "maxHits {max_hits}: {reply}");
    }
    assert_eq!(
        op(
            &mut engine,
            &mut session,
            json!({"op": "search", "query": "page"})
        )["total"],
        0,
        "case-sensitive"
    );
}

#[test]
fn search_has_no_per_page_quad_cap() {
    // 600 separate "x" on one page: every one is a hit.
    let mut content = String::from("BT /F1 6 Tf ");
    for row in 0..30 {
        content.push_str(&format!("1 0 0 1 20 {} Tm (", 760 - 20 * row));
        content.push_str(&"x    ".repeat(20));
        content.push_str(") Tj ");
    }
    content.push_str("ET");
    let (mut engine, mut session) = session_with(text_pages(&[&content], "[0 0 612 792]"));
    let all = op(
        &mut engine,
        &mut session,
        json!({"op": "search", "query": "x", "maxHits": 2000}),
    );
    assert_eq!(
        (all["total"].as_u64(), all["truncated"].as_bool()),
        (Some(600), Some(false))
    );
    let budget = op(
        &mut engine,
        &mut session,
        json!({"op": "search", "query": "x"}),
    );
    assert_eq!(
        (budget["total"].as_u64(), budget["truncated"].as_bool()),
        (Some(500), Some(true))
    );
}

// ---------------------------------------------------------------- text-all

/// An RC4-40 (V1 R2) document with an empty user password, its owner
/// password "owner" and /P `p`; only the content stream needs encrypting.
fn rc4_pdf(content: &str, p: i32) -> Vec<u8> {
    const PAD: [u8; 32] = [
        0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01,
        0x08, 0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53,
        0x69, 0x7A,
    ];
    fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
        let mut s: Vec<u8> = (0..=255).collect();
        let mut j = 0u8;
        for i in 0..256 {
            j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
            s.swap(i, usize::from(j));
        }
        let (mut i, mut j) = (0u8, 0u8);
        data.iter()
            .map(|b| {
                i = i.wrapping_add(1);
                j = j.wrapping_add(s[usize::from(i)]);
                s.swap(usize::from(i), usize::from(j));
                b ^ s[usize::from(s[usize::from(i)].wrapping_add(s[usize::from(j)]))]
            })
            .collect()
    }
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02X}")).collect::<String>();
    let id = [7u8; 16];
    let mut owner = b"owner".to_vec();
    owner.extend_from_slice(&PAD[..27]);
    let o = rc4(&Md5::digest(&owner)[..5], &PAD);
    let mut seed = PAD.to_vec();
    seed.extend_from_slice(&o);
    seed.extend_from_slice(&p.to_le_bytes());
    seed.extend_from_slice(&id);
    let key = Md5::digest(&seed)[..5].to_vec();
    let u = rc4(&key, &PAD);
    let mut object_key = key.clone();
    object_key.extend_from_slice(&[5, 0, 0, 0, 0]);
    let data = rc4(&Md5::digest(&object_key)[..10], content.as_bytes());
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 3 0 R >> >> /Contents 5 0 R >>"
            .to_string(),
    ];
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
    }
    offsets.push(bytes.len());
    bytes.extend_from_slice(format!("5 0 obj\n<< /Length {} >>\nstream\n", data.len()).as_bytes());
    bytes.extend_from_slice(&data);
    bytes.extend_from_slice(b"\nendstream\nendobj\n");
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size 6 /Root 1 0 R /ID [<{0}> <{0}>] /Encrypt << /Filter /Standard \
             /V 1 /R 2 /O <{1}> /U <{2}> /P {p} >> >>\nstartxref\n{xref}\n%%EOF\n",
            hex(&id),
            hex(&o),
            hex(&u)
        )
        .as_bytes(),
    );
    bytes
}

#[test]
fn text_all_joins_trimmed_pages_and_honours_the_copy_bit() {
    let (mut engine, mut session) = session_with(text_pages(
        &[
            "BT /F1 12 Tf 72 700 Td (  first ) Tj 0 -20 Td (line two) Tj ET",
            "",
            "BT /F1 12 Tf 72 700 Td (last) Tj ET",
        ],
        "[0 0 612 792]",
    ));
    let reply = op(
        &mut engine,
        &mut session,
        json!({"id": 2, "op": "text-all"}),
    );
    assert_eq!(
        reply,
        json!({"id": 2, "ok": true, "text": "first \n\nline two\u{c}\u{c}last"}),
        "pages trimmed and joined by form feeds"
    );

    // The copy bit (16) cleared; the empty user password opens it.
    let restricted = rc4_pdf("BT /F1 24 Tf 72 700 Td (secret words) Tj ET", -20);
    let (mut engine, mut session) = session_with(restricted.clone());
    assert!(!session.locked);
    let (reply, outputs) = call(&mut engine, &mut session, json!({"op": "text-all"}));
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["reason"], "read-only");
    assert_eq!(
        reply["error"],
        "text-all: this document does not permit copying"
    );
    assert!(outputs.is_empty());
    // Display and search do not need the copy bit, as in the earlier app engine.
    assert_eq!(render_text(&mut engine, &mut session, 0), "secret words");
    assert_eq!(
        op(
            &mut engine,
            &mut session,
            json!({"op": "search", "query": "secret"})
        )["total"],
        1
    );
    // With the copy bit set the same file exports its text.
    let (mut engine, mut session) =
        session_with(rc4_pdf("BT /F1 24 Tf 72 700 Td (open words) Tj ET", -4));
    assert_eq!(
        op(&mut engine, &mut session, json!({"op": "text-all"}))["text"],
        "open words"
    );
}

#[test]
fn text_all_on_large_pdf_has_every_page() {
    let (mut engine, mut session) = session_with(fixture("large.pdf"));
    let reply = op(&mut engine, &mut session, json!({"op": "text-all"}));
    let text = reply["text"].as_str().unwrap();
    assert_eq!(text.split('\u{c}').count(), 500);
    assert!(text.split('\u{c}').all(|page| page.contains("PDF4All")));
}

// ---------------------------------------------------------------- layers

/// Groups Base (6) and Notes (7) on one page, each marking one word, with
/// `config` as /D; `variant` moves /D or /OCProperties into objects 8, 9.
fn layers_pdf(config: &str, variant: &str, annots: &str) -> Vec<u8> {
    let content = "/OC /ocA BDC BT /F1 24 Tf 72 700 Td (base) Tj ET EMC\n\
                   /OC /ocB BDC BT /F1 24 Tf 72 600 Td (notes) Tj ET EMC";
    let groups = "/OCGs [6 0 R 7 0 R]";
    let (catalog, extra): (String, Vec<String>) = match variant {
        "indirect-config" => (
            format!("/OCProperties << {groups} /D 8 0 R >>"),
            vec![format!("<< {config} >>")],
        ),
        "indirect-properties" => (
            "/OCProperties 8 0 R".to_string(),
            vec![format!("<< {groups} /D << {config} >> >>")],
        ),
        _ => (
            format!("/OCProperties << {groups} /D << {config} >> >>"),
            Vec::new(),
        ),
    };
    let mut objects = vec![
        format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] {annots} /Resources << \
             /Font << /F1 4 0 R >> /Properties << /ocA 6 0 R /ocB 7 0 R >> >> /Contents 5 0 R >>"
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        stream("", content),
        "<< /Type /OCG /Name (Base) >>".to_string(),
        "<< /Type /OCG /Name (Notes) >>".to_string(),
    ];
    objects.extend(extra);
    classic(&objects, "")
}

fn items(reply: &Value) -> Vec<(u64, String, bool)> {
    (reply["items"].as_array().unwrap().iter())
        .map(|item| {
            (
                item["id"].as_u64().unwrap(),
                item["name"].as_str().unwrap().to_string(),
                item["visible"].as_bool().unwrap(),
            )
        })
        .collect()
}

fn item(id: u64, name: &str, visible: bool) -> (u64, String, bool) {
    (id, name.to_string(), visible)
}

#[test]
fn layers_list_in_descending_object_order_with_default_visibility() {
    // Descending object number, whatever /OCGs says.
    let (mut engine, mut session) = session_with(layers_pdf("", "", ""));
    let listed = op(&mut engine, &mut session, json!({"id": 4, "op": "layers"}));
    assert_eq!(listed["id"], 4);
    assert_eq!(
        items(&listed),
        [item(0, "Notes", true), item(1, "Base", true)]
    );

    let groups = |ocgs: &str, config: &str| -> Vec<(u64, String, bool)> {
        let objects = [
            format!(
                "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [{ocgs}] /D << {config} >> >> >>"
            ),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_string(),
            "<< /Type /OCG /Name (A4) >>".to_string(),
            "<< /Type /OCG /Name (B5) >>".to_string(),
            "<< /Type /OCG >>".to_string(),
            "<< /Type /OCG /Name <FEFF00C40031> >>".to_string(),
        ];
        let (mut engine, mut session) = session_with(classic(&objects, ""));
        items(&op(&mut engine, &mut session, json!({"op": "layers"})))
    };
    assert_eq!(
        groups("5 0 R 4 0 R 7 0 R", ""),
        [
            item(0, "\u{C4}1", true),
            item(1, "B5", true),
            item(2, "A4", true)
        ]
    );
    // Each group once; names, strings, nulls and missing objects are not groups.
    assert_eq!(
        groups("4 0 R 4 0 R /Junk (s) null 99 0 R 6 0 R", ""),
        [item(0, "layer 1", true), item(1, "A4", true)]
    );
    // /BaseState /OFF, then /ON, then /OFF, which wins over /ON.
    assert_eq!(
        groups(
            "4 0 R 5 0 R 6 0 R",
            "/BaseState /OFF /ON [5 0 R 6 0 R] /OFF [6 0 R]"
        ),
        [
            item(0, "layer 1", false),
            item(1, "B5", true),
            item(2, "A4", false)
        ]
    );
    assert_eq!(
        groups("4 0 R 5 0 R", "/BaseState /Unchanged /OFF [4 0 R]"),
        [item(0, "B5", true), item(1, "A4", false)]
    );
    assert!(groups("", "").is_empty());
    let (mut engine, mut session) = session_with(pages_pdf(1));
    assert_eq!(
        items(&op(&mut engine, &mut session, json!({"op": "layers"}))),
        []
    );
}

#[test]
fn layer_set_is_view_only_and_every_view_honours_it() {
    for variant in ["", "indirect-config", "indirect-properties"] {
        let source = layers_pdf("/Order [6 0 R 7 0 R] /ON [6 0 R 7 0 R]", variant, "");
        let (mut engine, mut session) = session_with(source.clone());
        assert_eq!(
            render_text(&mut engine, &mut session, 0),
            "base notes",
            "{variant}"
        );
        let (shown, shown_pixels) = call(
            &mut engine,
            &mut session,
            json!({"op": "render", "page": 0, "scale": 1}),
        );
        let thumb = call(
            &mut engine,
            &mut session,
            json!({"op": "thumbnail", "page": 0, "maxEdge": 128}),
        )
        .1;

        let hidden = op(
            &mut engine,
            &mut session,
            json!({"id": 9, "op": "layer-set", "layerId": 1, "visible": false}),
        );
        assert_eq!(hidden["ok"], true, "{hidden}");
        assert_eq!(hidden["id"], 9);
        assert_eq!(hidden["persisted"], false);
        assert_eq!(
            items(&hidden),
            [item(0, "Notes", true), item(1, "Base", false)],
            "{variant}"
        );
        assert!(
            hidden.get("revision").is_none(),
            "no journal slice: {hidden}"
        );
        assert_eq!(session.revision, 0, "no revision bump");
        assert!(!session.journal_state().can_undo && !session.journal_state().dirty);
        assert_eq!(
            items(&op(&mut engine, &mut session, json!({"op": "layers"})))[1],
            item(1, "Base", false)
        );

        // Render, thumbnail, search and text-all all leave the hidden layer out.
        assert_eq!(
            render_text(&mut engine, &mut session, 0),
            "notes",
            "{variant}"
        );
        let (render, pixels) = call(
            &mut engine,
            &mut session,
            json!({"op": "render", "page": 0, "scale": 1}),
        );
        assert!(
            render["nonWhite"].as_u64() < shown["nonWhite"].as_u64(),
            "{variant}"
        );
        assert_ne!(pixels, shown_pixels);
        assert_ne!(
            call(
                &mut engine,
                &mut session,
                json!({"op": "thumbnail", "page": 0, "maxEdge": 128})
            )
            .1,
            thumb
        );
        assert_eq!(
            op(
                &mut engine,
                &mut session,
                json!({"op": "search", "query": "base"})
            )["total"],
            0
        );
        assert_eq!(
            op(
                &mut engine,
                &mut session,
                json!({"op": "search", "query": "notes"})
            )["total"],
            1
        );
        assert_eq!(
            op(&mut engine, &mut session, json!({"op": "text-all"}))["text"],
            "notes"
        );
        assert!(
            op(&mut engine, &mut session, json!({"op": "undo"}))["ok"] == false,
            "nothing to undo"
        );

        // Nothing is written: the snapshot and the save keep /OCProperties as it was.
        let snapshot = session.snapshot_bytes().unwrap();
        assert_eq!(
            snapshot, source,
            "{variant}: the document itself is untouched"
        );
        let (save, outputs) = call(&mut engine, &mut session, json!({"op": "save"}));
        assert_eq!(save["ok"], true, "{save}");
        assert_eq!(save["dirty"], false);
        let (_, mut reopened) = session_with(outputs[0].clone());
        assert_eq!(render_text(&mut engine, &mut reopened, 0), "base notes");

        // Showing it again restores the unedited view exactly.
        let again = op(
            &mut engine,
            &mut session,
            json!({"op": "layer-set", "layerId": 1, "visible": true}),
        );
        assert_eq!(
            items(&again),
            [item(0, "Notes", true), item(1, "Base", true)]
        );
        assert_eq!(
            call(
                &mut engine,
                &mut session,
                json!({"op": "render", "page": 0, "scale": 1})
            )
            .1,
            shown_pixels
        );
    }
}

#[test]
fn layer_visibility_overrides_the_default_configuration_and_survives_edits() {
    // Base starts hidden by /D: showing it puts it in /ON and takes it out of /OFF.
    let (mut engine, mut session) = session_with(layers_pdf("/BaseState /OFF /ON [7 0 R]", "", ""));
    assert_eq!(
        items(&op(&mut engine, &mut session, json!({"op": "layers"}))),
        [item(0, "Notes", true), item(1, "Base", false)]
    );
    assert_eq!(render_text(&mut engine, &mut session, 0), "notes");
    op(
        &mut engine,
        &mut session,
        json!({"op": "layer-set", "layerId": 1, "visible": true}),
    );
    op(
        &mut engine,
        &mut session,
        json!({"op": "layer-set", "layerId": 0, "visible": false}),
    );
    assert_eq!(render_text(&mut engine, &mut session, 0), "base");

    // An edit keeps the panel's visibility; so do undo and redo.
    let content = ObjectId::new(5, 0).unwrap();
    session
        .mutate("edit", |s| {
            let data = b"/OC /ocA BDC BT /F1 24 Tf 72 700 Td (BASE) Tj ET EMC\n\
                         /OC /ocB BDC BT /F1 24 Tf 72 600 Td (NOTES) Tj ET EMC"
                .to_vec();
            Ok(s.tx()?.replace_decoded_stream(content, data)?)
        })
        .unwrap();
    assert_eq!(render_text(&mut engine, &mut session, 0), "BASE");
    session.doc.as_mut().unwrap().undo().unwrap();
    assert_eq!(render_text(&mut engine, &mut session, 0), "base");
    assert_eq!(
        items(&op(&mut engine, &mut session, json!({"op": "layers"}))),
        [item(0, "Notes", false), item(1, "Base", true)]
    );

    // A new document starts from its own configuration.
    open(
        &mut engine,
        &mut session,
        layers_pdf("/BaseState /OFF /ON [7 0 R]", "", ""),
    );
    assert_eq!(render_text(&mut engine, &mut session, 0), "notes");
}

#[test]
fn annotations_in_hidden_layers_are_not_drawn() {
    let ap = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 100 100]",
        "1 0 0 rg 0 0 100 100 re f",
    );
    let page = |oc: &str, config: &str| {
        let objects = [
            format!(
                "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [5 0 R 6 0 R] /D << {config} >> >> >>"
            ),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>".to_string(),
            format!(
                "<< /Type /Annot /Subtype /Square /Rect [100 100 200 200] /F 4 {oc} /AP << /N 7 0 R >> >>"
            ),
            "<< /Type /OCG /Name (A) >>".to_string(),
            "<< /Type /OCG /Name (B) >>".to_string(),
            ap.clone(),
        ];
        classic(&objects, "")
    };
    let ink = |bytes: Vec<u8>, toggle: Option<(u64, bool)>| {
        let (mut engine, mut session) = session_with(bytes);
        if let Some((layer, visible)) = toggle {
            let reply = op(
                &mut engine,
                &mut session,
                json!({"op": "layer-set", "layerId": layer, "visible": visible}),
            );
            assert_eq!(reply["ok"], true, "{reply}");
        }
        op(
            &mut engine,
            &mut session,
            json!({"op": "render", "page": 0, "scale": 1}),
        )["nonWhite"]
            .as_u64()
            .unwrap()
    };
    // Layer 1 is group A (5), layer 0 is B (6).
    assert_eq!(ink(page("/OC 5 0 R", ""), None), 10_000);
    assert_eq!(ink(page("/OC 5 0 R", ""), Some((1, false))), 0);
    assert_eq!(ink(page("/OC 5 0 R", "/OFF [5 0 R]"), None), 0);
    assert_eq!(
        ink(page("/OC 5 0 R", "/OFF [5 0 R]"), Some((1, true))),
        10_000
    );
    assert_eq!(
        ink(page("/OC 5 0 R", ""), Some((0, false))),
        10_000,
        "another group"
    );
    let all_on = "/OC << /Type /OCMD /OCGs [5 0 R 6 0 R] /P /AllOn >>";
    let any_on = "/OC << /Type /OCMD /OCGs [5 0 R 6 0 R] >>";
    // A direct membership dictionary is drawn by its policy (§8.11.2.2).
    assert_eq!(ink(page(all_on, "/OFF [5 0 R]"), None), 0);
    assert_eq!(ink(page(any_on, "/OFF [5 0 R]"), None), 10_000);
    assert_eq!(ink(page(any_on, "/OFF [5 0 R 6 0 R]"), None), 0);
}

#[test]
fn layer_set_refuses_as_engine_ts_does() {
    let (mut engine, mut session) = session_with(pages_pdf(1));
    assert_eq!(
        refused(
            &mut engine,
            &mut session,
            json!({"op": "layer-set", "layerId": 0, "visible": false})
        ),
        "layer-set: layer must be an integer in 0..0"
    );
    open(&mut engine, &mut session, layers_pdf("", "", ""));
    for (request, message) in [
        (
            json!({"op": "layer-set", "layerId": 2, "visible": false}),
            "layer-set: layer must be an integer in 0..1",
        ),
        (
            json!({"op": "layer-set", "layerId": -1, "visible": false}),
            "layer-set: layer must be an integer in 0..1",
        ),
        (
            json!({"op": "layer-set", "layerId": 0.5, "visible": false}),
            "layer-set: layer must be an integer in 0..1",
        ),
        (
            json!({"op": "layer-set", "layerId": "0", "visible": false}),
            "layer-set: layer must be an integer in 0..1",
        ),
        (
            json!({"op": "layer-set", "visible": false}),
            "layer-set: layer must be an integer in 0..1",
        ),
        // The layer is checked before the visibility.
        (
            json!({"op": "layer-set", "layerId": 9, "visible": 1}),
            "layer-set: layer must be an integer in 0..1",
        ),
        (
            json!({"op": "layer-set", "layerId": 0, "visible": 1}),
            "layer-set: visible must be true or false",
        ),
        (
            json!({"op": "layer-set", "layerId": 0, "visible": "false"}),
            "layer-set: visible must be true or false",
        ),
        (
            json!({"op": "layer-set", "layerId": 0}),
            "layer-set: visible must be true or false",
        ),
    ] {
        assert_eq!(refused(&mut engine, &mut session, request), message);
    }
    assert_eq!(
        render_text(&mut engine, &mut session, 0),
        "base notes",
        "nothing changed"
    );
}

// ---------------------------------------------------------------- views after edits

#[test]
fn render_after_an_edit_shows_it_and_reports_the_revision() {
    let (mut engine, mut session) = session_with(pages_pdf(2));
    let content = ObjectId::new(5, 0).unwrap();
    session
        .mutate("edit", |s| {
            Ok(s.tx()?.replace_decoded_stream(
                content,
                b"BT /F1 24 Tf 72 700 Td (Edited) Tj ET".to_vec(),
            )?)
        })
        .unwrap();
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "render", "page": 0, "scale": 1}),
    );
    assert_eq!(
        (reply["revision"].as_u64(), reply["text"].as_str()),
        (Some(1), Some("Edited\n\n"))
    );
    let thumb = op(
        &mut engine,
        &mut session,
        json!({"op": "thumbnail", "page": 0, "maxEdge": 64}),
    );
    assert_eq!(thumb["revision"], 1);
    assert_eq!(
        op(
            &mut engine,
            &mut session,
            json!({"op": "search", "query": "Edited"})
        )["total"],
        1
    );
    assert_eq!(
        op(&mut engine, &mut session, json!({"op": "text-all"}))["text"],
        "Edited\u{c}Page 1"
    );
}

// ---------------------------------------------------------------- limits

/// A classic-xref PDF from raw object bodies numbered from 1.
fn classic_raw(objects: &[Vec<u8>]) -> Vec<u8> {
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

fn flate(dictionary: &str, data: &[u8]) -> Vec<u8> {
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
    let mut body = format!(
        "<< {dictionary} /Filter /FlateDecode /Length {} >>\nstream\n",
        packed.len()
    )
    .into_bytes();
    body.extend_from_slice(&packed);
    body.extend_from_slice(b"\nendstream");
    body
}

const HELVETICA: &str = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";

/// Page 0 draws `content` (Flate), page 1 shows "Page 1". Both have /F1
/// Helvetica (7); page 0 also /X (8) and /F2 (9), the first `extra` objects.
fn heavy_pdf(content: &[u8], extra: &[Vec<u8>]) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources \
          << /Font << /F1 7 0 R /F2 9 0 R >> /XObject << /X 8 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        flate("", content),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
          /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>"
            .to_vec(),
        stream("", "BT /F1 24 Tf 72 700 Td (Page 1) Tj ET").into_bytes(),
        HELVETICA.as_bytes().to_vec(),
    ];
    objects.extend_from_slice(extra);
    classic_raw(&objects)
}

/// Render refuses the page; search and text-all read it as empty.
fn refused_as_too_much(bytes: Vec<u8>, reason: &str) {
    let (mut engine, mut session) = session_with(bytes);
    let error = refused(
        &mut engine,
        &mut session,
        json!({"op": "render", "page": 0, "scale": 1}),
    );
    assert_eq!(error, format!("render: page is too complex: {reason}"));
    let found = op(
        &mut engine,
        &mut session,
        json!({"op": "search", "query": "Page"}),
    );
    assert_eq!(
        (found["ok"].as_bool(), found["total"].as_u64()),
        (Some(true), Some(1))
    );
    assert_eq!(found["hits"][0]["page"], 1);
    let all = op(&mut engine, &mut session, json!({"op": "text-all"}));
    assert_eq!(all["text"], "\u{c}Page 1", "{reason}");
    // The session still works.
    assert_eq!(render_text(&mut engine, &mut session, 1), "Page 1");
}

#[test]
fn a_page_showing_more_text_than_the_limit_is_refused() {
    // One Tj of a million glyphs (14 KB compressed) took the WASM engine down.
    let limit = usize::try_from(tandem_pdf_engine::view::MAX_SHOWN_BYTES).unwrap();
    let mut content = b"BT /F1 1 Tf 0 0 Td (".to_vec();
    content.resize(content.len() + limit + 1, b'a');
    content.extend_from_slice(b") Tj ET");
    let shown = "page shows more text than the limit";
    refused_as_too_much(heavy_pdf(&content, &[]), shown);
    // A form's text counts each time it is drawn.
    let form = format!("BT /F1 1 Tf 0 0 Td ({}) Tj ET", "a".repeat(1024));
    let form = stream("/Type /XObject /Subtype /Form /BBox [0 0 612 792]", &form);
    let draws = "/X Do ".repeat(1025);
    refused_as_too_much(
        heavy_pdf(draws.as_bytes(), &[form.clone().into_bytes()]),
        shown,
    );
    // Within the limit it renders.
    let draws = "/X Do ".repeat(1000);
    let (mut engine, mut session) = session_with(heavy_pdf(draws.as_bytes(), &[form.into_bytes()]));
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "render", "page": 0, "scale": 0.25}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
}

#[test]
fn a_page_whose_text_expands_past_the_character_limit_is_refused() {
    let limit = tandem_pdf_engine::view::MAX_PAGE_CHARS;
    let too_much = "page has more text than the limit";
    // A ToUnicode mapping each byte to four characters.
    let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
                1 begincodespacerange <00> <FF> endcodespacerange \
                1 beginbfchar <61> <0041004200430044> endbfchar \
                endcmap CMapName currentdict /CMap defineresource pop end end";
    let mut content = b"BT /F2 1 Tf 0 0 Td (".to_vec();
    content.resize(content.len() + limit / 4 + 1, b'a');
    content.extend_from_slice(b") Tj ET");
    let extra = [
        stream("/Type /XObject /Subtype /Form /BBox [0 0 1 1]", "").into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 10 0 R >>".to_vec(),
        stream("", cmap).into_bytes(),
    ];
    refused_as_too_much(heavy_pdf(&content, &extra), too_much);
    // ActualText strings add up across marked-content sequences.
    let half = "x".repeat(limit / 2 + 1);
    let content = format!(
        "/Span << /ActualText ({half}) >> BDC BT /F1 12 Tf 72 700 Td (a) Tj ET EMC \
         /Span << /ActualText ({half}) >> BDC BT /F1 12 Tf 72 600 Td (b) Tj ET EMC"
    );
    refused_as_too_much(heavy_pdf(content.as_bytes(), &[]), too_much);
    // One sequence within the limit is read.
    let content =
        format!("/Span << /ActualText ({half}) >> BDC BT /F1 12 Tf 72 700 Td (a) Tj ET EMC");
    let (mut engine, mut session) = session_with(heavy_pdf(content.as_bytes(), &[]));
    let all = op(&mut engine, &mut session, json!({"op": "text-all"}));
    assert_eq!(all["text"], format!("{half}\u{c}Page 1"));
}

#[test]
fn text_all_and_search_stay_right_across_reader_cache_resets() {
    // 150 pages, each with its own font dictionary: past the reader cache's
    // font and page counts, so its cache starts afresh along the way.
    let n = 150;
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + 3 * i)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")),
    ];
    for i in 0..n {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 {} 0 R >> >> /Contents {} 0 R >>",
            5 + 3 * i,
            4 + 3 * i
        ));
        objects.push(stream(
            "",
            &format!("BT /F1 24 Tf 72 700 Td (Page {i}) Tj ET"),
        ));
        let base = ["Helvetica", "Times-Roman", "Courier"][i % 3];
        objects.push(format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /{base} /Name /F{i} >>"
        ));
    }
    let (mut engine, mut session) = session_with(classic(&objects, ""));
    let all = op(&mut engine, &mut session, json!({"op": "text-all"}));
    let expected: Vec<String> = (0..n).map(|i| format!("Page {i}")).collect();
    assert_eq!(all["text"], expected.join("\u{c}"));
    let found = op(
        &mut engine,
        &mut session,
        json!({"op": "search", "query": "Page 14", "maxHits": 2000}),
    );
    let pages: Vec<u64> = (found["hits"].as_array().unwrap().iter())
        .map(|hit| hit["page"].as_u64().unwrap())
        .collect();
    assert_eq!(
        pages,
        [14, 140, 141, 142, 143, 144, 145, 146, 147, 148, 149]
    );
    let fresh = op(
        &mut engine,
        &mut session,
        json!({"op": "render", "page": 149, "scale": 1}),
    );
    assert_eq!(fresh["text"], "Page 149\n\n");
}

// ---------------------------------------------------------------- object streams

/// A PDF with an xref stream: `top` objects at the top level, `stored` ones
/// in one object stream (numbered `container`), the xref stream after it.
fn objstm_pdf(top: &[(u32, String)], container: u32, stored: &[(u32, String)]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut rows: Vec<(u32, [u8; 7])> = Vec::new();
    let type1 = |offset: usize| {
        let mut row = [1, 0, 0, 0, 0, 0, 0];
        row[1..5].copy_from_slice(&u32::try_from(offset).unwrap().to_be_bytes());
        row
    };
    for (number, body) in top {
        rows.push((*number, type1(bytes.len())));
        bytes.extend_from_slice(format!("{number} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let (mut header, mut data) = (String::new(), String::new());
    for (index, (number, body)) in stored.iter().enumerate() {
        header.push_str(&format!("{number} {} ", data.len()));
        data.push_str(body);
        data.push('\n');
        let mut row = [2, 0, 0, 0, 0, 0, 0];
        row[1..5].copy_from_slice(&container.to_be_bytes());
        row[5..7].copy_from_slice(&u16::try_from(index).unwrap().to_be_bytes());
        rows.push((*number, row));
    }
    let content = format!("{header}{data}");
    rows.push((container, type1(bytes.len())));
    bytes.extend_from_slice(
        format!(
            "{container} 0 obj\n<< /Type /ObjStm /N {} /First {} /Length {} >>\nstream\n{content}\nendstream\nendobj\n",
            stored.len(),
            header.len(),
            content.len()
        )
        .as_bytes(),
    );
    let xref_number = container + 1;
    let xref = bytes.len();
    rows.push((xref_number, type1(xref)));
    let size = rows.iter().map(|(number, _)| number + 1).max().unwrap();
    let mut table = vec![[0u8; 7]; size as usize];
    for (number, row) in rows {
        table[number as usize] = row;
    }
    let table: Vec<u8> = table.concat();
    bytes.extend_from_slice(
        format!(
            "{xref_number} 0 obj\n<< /Type /XRef /Size {size} /W [1 4 2] /Root 1 0 R /Length {} >>\nstream\n",
            table.len()
        )
        .as_bytes(),
    );
    bytes.extend_from_slice(&table);
    bytes.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    bytes
}

/// A page with one blue 100×100 annotation `annot` (object 9, or direct in
/// the page with `direct`), stored in an object stream with the page; group
/// Watermark is object 4.
fn objstm_annot(config: &str, annot: &str, direct: bool) -> Vec<u8> {
    let annot = format!("<< /Type /Annot /Rect [100 100 200 200] {annot} /AP << /N 5 0 R >> >>");
    let annots = if direct {
        annot.clone()
    } else {
        "9 0 R".to_string()
    };
    let top = [
        (
            1,
            format!(
                "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [4 0 R] /D << {config} >> >> >>"
            ),
        ),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string()),
        (4, "<< /Type /OCG /Name (Watermark) >>".to_string()),
        (
            5,
            stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 100 100]",
                "0 0 1 rg 0 0 100 100 re f",
            ),
        ),
    ];
    let stored = [
        (
            3,
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [{annots}] >>"),
        ),
        (9, annot),
    ];
    objstm_pdf(&top, 6, &stored)
}

#[test]
fn hidden_annotations_in_object_streams_are_not_drawn() {
    let ink = |bytes: Vec<u8>, hide: bool| {
        let (mut engine, mut session) = session_with(bytes);
        if hide {
            let reply = op(
                &mut engine,
                &mut session,
                json!({"op": "layer-set", "layerId": 0, "visible": false}),
            );
            assert_eq!(reply["ok"], true, "{reply}");
        }
        let reply = op(
            &mut engine,
            &mut session,
            json!({"op": "render", "page": 0, "scale": 1}),
        );
        assert_eq!(reply["ok"], true, "{reply}");
        reply["nonWhite"].as_u64().unwrap()
    };
    // No blue pixels in any hidden case.
    for direct in [false, true] {
        let shown = objstm_annot("", "/Subtype /Square /F 4 /OC 4 0 R", direct);
        assert_eq!(ink(shown.clone(), false), 10_000, "direct {direct}");
        assert_eq!(ink(shown, true), 0, "layer-set, direct {direct}");
        let off = objstm_annot("/OFF [4 0 R]", "/Subtype /Square /F 4 /OC 4 0 R", direct);
        assert_eq!(ink(off, false), 0, "default off, direct {direct}");
        let noview = objstm_annot("", "/Subtype /Square /F 32", direct);
        assert_eq!(ink(noview, false), 0, "NoView, direct {direct}");
        let popup = objstm_annot("", "/Subtype /Popup /F 4", direct);
        assert_eq!(ink(popup, false), 0, "Popup, direct {direct}");
        let membership = "/Subtype /Square /F 4 /OC << /Type /OCMD /OCGs 4 0 R /P /AllOn >>";
        assert_eq!(
            ink(objstm_annot("/OFF [4 0 R]", membership, direct), false),
            0
        );
        assert_eq!(ink(objstm_annot("", membership, direct), false), 10_000);
    }
    // The view's redefinitions never reach the saved bytes.
    let hidden = objstm_annot("/OFF [4 0 R]", "/Subtype /Square /F 4 /OC 4 0 R", false);
    let (mut engine, mut session) = session_with(hidden);
    op(
        &mut engine,
        &mut session,
        json!({"op": "render", "page": 0, "scale": 1}),
    );
    let saved = session.doc.as_mut().unwrap().snapshot().unwrap();
    let saved = String::from_utf8_lossy(&saved);
    assert!(!saved.contains("/F 6"), "{saved}");
}

// ---------------------------------------------------------------- page boxes

/// One page per entry of `boxes`, each a blue rectangle over the whole of
/// Letter and "Hi" at (120, 120).
fn boxes_pdf(boxes: &[&str]) -> Vec<u8> {
    let kids: Vec<String> = (0..boxes.len()).map(|i| format!("{} 0 R", 5 + i)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            boxes.len()
        ),
        stream(
            "",
            "0 0 1 rg 0 0 612 792 re f 1 0 0 rg BT /F1 24 Tf 120 120 Td (Hi) Tj ET",
        ),
        HELVETICA.to_string(),
    ];
    for page in boxes {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R {page} /Contents 3 0 R /Resources << /Font << /F1 4 0 R >> >> >>"
        ));
    }
    classic(&objects, "")
}

/// The six pages of `degenerate_boxes` and what `render` replies for each:
/// (width, height, nonWhite, text) under spec 04 §5.1. The content fills
/// the page blue and shows `Hi` at (120, 120).
const DEGENERATE_BOXES: [&str; 6] = [
    "/MediaBox [0 0 0 0]",
    "",
    "/MediaBox [0 0 0 0] /CropBox [100 100 400 400]",
    "/MediaBox [0 0 612 792] /CropBox [700 700 800 800]",
    "/MediaBox [0 0 612 792] /CropBox [0 0 0 0]",
    "/MediaBox [0 0 612 792] /CropBox [612 0 700 792]",
];

fn render_reply(
    engine: &mut AppEngine,
    session: &mut AppSession,
    page: usize,
) -> (f64, f64, u64, String, bool) {
    let reply = op(
        engine,
        session,
        json!({"op": "render", "page": page, "scale": 1}),
    );
    assert_eq!(reply["ok"], true, "{page}: {reply}");
    (
        reply["width"].as_f64().unwrap(),
        reply["height"].as_f64().unwrap(),
        reply["nonWhite"].as_u64().unwrap(),
        reply["text"].as_str().unwrap().to_string(),
        reply["lines"].as_array().unwrap().is_empty(),
    )
}

#[test]
fn degenerate_page_boxes_draw_and_read() {
    let (mut engine, mut session) = session_with(boxes_pdf(&DEGENERATE_BOXES));
    // An empty or absent MediaBox is US Letter.
    for (page, label) in DEGENERATE_BOXES.iter().enumerate().take(2) {
        assert_eq!(
            render_reply(&mut engine, &mut session, page),
            (612.0, 792.0, 484_704, "Hi\n\n".to_string(), false),
            "{label}"
        );
    }
    // Search reads every page's text, in each page's frame.
    let found = op(
        &mut engine,
        &mut session,
        json!({"op": "search", "query": "Hi"}),
    );
    assert_eq!(found["total"], 6);
    let quad = &found["hits"][3]["quads"][0];
    assert_eq!(
        (quad[0].as_f64(), quad[1].as_f64().map(f64::round)),
        (Some(120.0), Some(646.0))
    );
}

#[test]
fn a_crop_box_outside_the_media_box_shows_the_media_box() {
    let (mut engine, mut session) = session_with(boxes_pdf(&DEGENERATE_BOXES));
    // The crop inside the Letter page that stands in for an empty MediaBox.
    assert_eq!(
        render_reply(&mut engine, &mut session, 2),
        (300.0, 300.0, 90_000, "Hi\n\n".to_string(), false)
    );
    for (page, label) in DEGENERATE_BOXES.iter().enumerate().skip(3) {
        assert_eq!(
            render_reply(&mut engine, &mut session, page),
            (612.0, 792.0, 484_704, "Hi\n\n".to_string(), false),
            "{label}"
        );
    }
    let all = op(&mut engine, &mut session, json!({"op": "text-all"}));
    assert_eq!(all["text"], "Hi\u{c}Hi\u{c}Hi\u{c}Hi\u{c}Hi\u{c}Hi");
}

// ---------------------------------------------------------------- measurements

/// A dense text page: 90 lines of 7 pt Helvetica, about 11,000 glyphs, with
/// a ruled grid of 200 strokes.
fn dense_pdf() -> Vec<u8> {
    let mut content = String::from("0.5 w ");
    for i in 0..100 {
        content.push_str(&format!("{} 20 m {} 772 l S ", 20 + i * 5, 20 + i * 5));
        content.push_str(&format!("20 {} m 592 {} l S ", 20 + i * 7, 20 + i * 7));
    }
    content.push_str("BT /F1 7 Tf 8 TL 24 760 Td ");
    for line in 0..90 {
        content.push_str(&format!(
            "(Line {line:02}: the quick brown fox jumps over the lazy dog, 0123456789 and more words to fill it.) '"
        ));
    }
    content.push_str(" ET");
    text_pages(&[&content], "[0 0 612 792]")
}

/// `cargo test --release --test app_view -- --ignored --nocapture`.
#[test]
#[ignore = "measurement"]
fn measure_render_and_revision_costs() {
    use std::time::{Duration, Instant};
    let median = |mut runs: Vec<Duration>| {
        runs.sort();
        runs[runs.len() / 2]
    };
    for (label, bytes, pages) in [
        ("large.pdf", fixture("large.pdf"), vec![0usize, 250, 499]),
        ("quartz.pdf", fixture("p6/quartz.pdf"), vec![0]),
        ("dense", dense_pdf(), vec![0]),
    ] {
        let (mut engine, mut session) = session_with(bytes);
        let started = Instant::now();
        session.view().unwrap();
        eprintln!("{label}: first view {:?}", started.elapsed());
        for &page in &pages {
            for scale in [1.0, 2.0] {
                let mut render = Vec::new();
                let mut draw = Vec::new();
                for _ in 0..7 {
                    let started = Instant::now();
                    let reply = op(
                        &mut engine,
                        &mut session,
                        json!({"op": "render", "page": page, "scale": scale}),
                    );
                    assert_eq!(reply["ok"], true, "{reply}");
                    render.push(started.elapsed());
                    let view = session.view().unwrap();
                    let started = Instant::now();
                    view.render(page, scale, Default::default()).unwrap();
                    draw.push(started.elapsed());
                }
                eprintln!(
                    "{label} page {page} scale {scale}: render op {:?} (pixels alone {:?})",
                    median(render),
                    median(draw)
                );
            }
        }
        let mut thumbs = Vec::new();
        for _ in 0..7 {
            let started = Instant::now();
            op(
                &mut engine,
                &mut session,
                json!({"op": "thumbnail", "page": 0, "maxEdge": 256}),
            );
            thumbs.push(started.elapsed());
        }
        eprintln!("{label}: thumbnail {:?}", median(thumbs));
        let started = Instant::now();
        let reply = op(
            &mut engine,
            &mut session,
            json!({"op": "search", "query": "zzz-not-there"}),
        );
        assert_eq!(reply["total"], 0);
        eprintln!("{label}: search every page {:?}", started.elapsed());
    }

    // The renderer-visible cost of a revision: one small edit on large.pdf,
    // then the render of the edited page (view rebuild included), and undo.
    let (mut engine, mut session) = session_with(fixture("large.pdf"));
    op(
        &mut engine,
        &mut session,
        json!({"op": "render", "page": 0, "scale": 1}),
    );
    let content = {
        let doc = session.doc.as_ref().unwrap();
        let (_, page) = doc.page(0).unwrap();
        page.get(b"Contents".as_slice())
            .and_then(|value| value.as_reference())
            .unwrap()
    };
    let mut edits = Vec::new();
    for round in 0..7 {
        let started = Instant::now();
        session
            .mutate("edit", |s| {
                let data = format!("BT /F1 24 Tf 72 700 Td (Edit {round}) Tj ET").into_bytes();
                Ok(s.tx()?.replace_decoded_stream(content, data)?)
            })
            .unwrap();
        let mutated = started.elapsed();
        let reply = op(
            &mut engine,
            &mut session,
            json!({"op": "render", "page": 0, "scale": 1}),
        );
        assert!(
            reply["text"]
                .as_str()
                .unwrap()
                .contains(&format!("Edit {round}"))
        );
        edits.push((mutated, started.elapsed()));
    }
    edits.sort_by_key(|(_, total)| *total);
    let (mutated, total) = edits[edits.len() / 2];
    eprintln!("large.pdf: edit {mutated:?}, edit + render of the page {total:?}");
    let started = Instant::now();
    session.doc.as_mut().unwrap().undo().unwrap();
    op(
        &mut engine,
        &mut session,
        json!({"op": "render", "page": 0, "scale": 1}),
    );
    eprintln!("large.pdf: undo + render {:?}", started.elapsed());
    let started = Instant::now();
    op(
        &mut engine,
        &mut session,
        json!({"op": "layer-set", "layerId": 0, "visible": false}),
    );
    eprintln!(
        "large.pdf: layer-set without layers {:?}",
        started.elapsed()
    );
}
