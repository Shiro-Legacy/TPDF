// SPDX-License-Identifier: MIT
//! Annotation ops on the app session: each kind written with its
//! dictionary and an appearance stream that parses, paints inside its
//! /Rect in its colour and reads back as its text; refusals in the earlier app engine's
//! order with nothing journaled, undo and redo exact, deletes that leave
//! nothing reaching the annotation, and the view drawing annotations
//! without an appearance or with NoZoom and NoRotate.
use std::sync::Arc;

use serde_json::{Value, json};
use tandem_pdf_engine::app::annots::appearance;
use tandem_pdf_engine::app::annots::fonts::{self, Base14};
use tandem_pdf_engine::app::annots::{self, PageSpace};
use tandem_pdf_engine::app::edit::{Builder, Edit};
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::content::{ContentLimits, ContentParser, Operation};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Dictionary, Object, ObjectId, decode_text_string};
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::view::{RenderOptions, RgbaImage, ViewDocument};
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

// ------------------------------------------------------------------ appearance checks

/// Operators of ISO 32000-2 §8.4 (graphics state), §8.5 (paths), §8.6.8
/// (colour), §8.8 (Do), §9.3-9.4 (text) and §14.6 (marked content).
const ISO_OPERATORS: &[&str] = &[
    "w", "J", "j", "M", "d", "ri", "i", "gs", "q", "Q", "cm", "m", "l", "c", "v", "y", "h", "re",
    "S", "s", "f", "F", "f*", "B", "B*", "b", "b*", "n", "W", "W*", "CS", "cs", "SC", "SCN", "sc",
    "scn", "G", "g", "RG", "rg", "K", "k", "Do", "BT", "ET", "Tc", "Tw", "Tz", "TL", "Tf", "Tr",
    "Ts", "Td", "TD", "Tm", "T*", "Tj", "TJ", "'", "\"", "BMC", "BDC", "EMC", "MP", "DP",
];

/// The content's operations; it must parse with ISO operators only, `q`/`Q`
/// and `BT`/`ET` in balance, and no `NaN` or `inf`.
fn checked(content: &[u8], label: &str) -> Vec<Operation> {
    let mut parser = ContentParser::new(content, ContentLimits::default());
    let mut ops = Vec::new();
    while let Some(operation) = parser
        .next_operation()
        .unwrap_or_else(|e| panic!("{label}: content parses: {e}"))
    {
        ops.push(operation);
    }
    let (mut q, mut bt) = (0i64, 0i64);
    for op in &ops {
        let name = String::from_utf8_lossy(&op.operator).into_owned();
        assert!(
            ISO_OPERATORS.contains(&name.as_str()),
            "{label}: operator {name}"
        );
        match name.as_str() {
            "q" => q += 1,
            "Q" => q -= 1,
            "BT" => bt += 1,
            "ET" => bt -= 1,
            _ => {}
        }
        assert!(
            q >= 0 && (0..=1).contains(&bt),
            "{label}: nesting at {name}"
        );
    }
    assert_eq!((q, bt), (0, 0), "{label}: q/Q and BT/ET balance");
    let text = String::from_utf8_lossy(content);
    assert!(!text.contains("NaN") && !text.contains("inf"), "{label}");
    ops
}

fn count(ops: &[Operation], names: &[&str]) -> usize {
    ops.iter()
        .filter(|op| names.iter().any(|n| op.operator == n.as_bytes()))
        .count()
}

/// The matrix that maps a form's /BBox, transformed by its /Matrix, onto
/// `rect` (ISO 32000-2 §12.5.5).
fn placement(bbox: [f64; 4], matrix: [f64; 6], rect: [f64; 4]) -> [f64; 6] {
    let shown = annots::transform_rect(&matrix, bbox);
    let sx = (rect[2] - rect[0]) / (shown[2] - shown[0]);
    let sy = (rect[3] - rect[1]) / (shown[3] - shown[1]);
    [
        sx,
        0.0,
        0.0,
        sy,
        rect[0] - shown[0] * sx,
        rect[1] - shown[1] * sy,
    ]
}

/// `bytes` (object numbers as the session has them) with the normal
/// appearance of annotation `number` drawn by page 0's content where the
/// annotation shows it, the page's annotations dropped: what a viewer
/// shows, as page content the text extraction reads.
fn flattened(bytes: &[u8], number: u32) -> Vec<u8> {
    let doc =
        Document::open_shared(Arc::new(bytes.to_vec()), DocumentLimits::default(), b"").unwrap();
    let page = tandem_pdf_engine::app::info::pages(&doc)[0]
        .id
        .expect("an indirect page");
    let dict = doc.resolve_id(id(number)).as_dict().unwrap();
    let form = doc.dict_get(dict, b"AP").as_dict().unwrap()[b"N".as_slice()]
        .as_reference()
        .unwrap();
    let rect = annots::rect(&doc, dict.get(b"Rect".as_slice()).unwrap()).unwrap();
    let form_dict = doc.resolve_id(form).as_dict().unwrap();
    let bbox = annots::rect(&doc, form_dict.get(b"BBox".as_slice()).unwrap()).unwrap();
    let matrix = match form_dict
        .get(b"Matrix".as_slice())
        .and_then(|m| annots::numbers(&doc, m))
    {
        Some(m) if m.len() == 6 => [m[0], m[1], m[2], m[3], m[4], m[5]],
        _ => appearance::IDENTITY,
    };
    let m = placement(bbox, matrix, rect);
    let numbers: Vec<String> = m.iter().map(|v| format!("{v:.6}")).collect();
    let content = format!("q {} cm /A Do Q", numbers.join(" "));
    let mut builder = Builder::from_document(doc);
    let stream = builder.allocate(Object::Null).unwrap();
    builder
        .put_stream_raw(stream, Dictionary::new(), content.into_bytes())
        .unwrap();
    builder
        .update(page, |page| {
            let page = page.as_dict_mut().unwrap();
            let mut xobjects = Dictionary::new();
            xobjects.insert(b"A".to_vec(), Object::Reference(form));
            let mut resources = Dictionary::new();
            resources.insert(b"XObject".to_vec(), Object::Dictionary(xobjects));
            page.insert(b"Resources".to_vec(), Object::Dictionary(resources));
            page.insert(b"Contents".to_vec(), Object::Reference(stream));
            page.remove(b"Annots".as_slice());
        })
        .unwrap();
    write_document(
        builder.document(),
        DocumentWriteLimits::default(),
        WriteOptions::APP,
    )
    .unwrap()
}

/// The text lines the engine extracts from page 0 of `pdf`, each with its
/// characters' baseline origins (page space, y down).
fn text_lines(pdf: &[u8]) -> Vec<(String, Vec<[f64; 2]>)> {
    let text = ViewDocument::open(pdf.to_vec(), "")
        .unwrap()
        .structured_text(0)
        .unwrap();
    let mut lines = Vec::new();
    for block in text.blocks() {
        for line in &block.lines {
            let s: String = line.chars.iter().map(|c| c.c).collect();
            if !s.trim().is_empty() {
                lines.push((
                    normalised(&s),
                    line.chars.iter().map(|c| c.origin).collect(),
                ));
            }
        }
    }
    lines
}

/// What annotation `number` shows, as extracted text lines.
fn drawn_lines(session: &AppSession, number: u32) -> Vec<String> {
    text_lines(&flattened(&serialized(session), number))
        .into_iter()
        .map(|(s, _)| s)
        .collect()
}

/// Whitespace runs as one space, trimmed.
fn normalised(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn pixel(image: &RgbaImage, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * image.width + x) * 4) as usize;
    [image.pixels[i], image.pixels[i + 1], image.pixels[i + 2]]
}

fn inked(p: [u8; 3]) -> bool {
    p.iter().any(|&c| c < 250)
}

/// A user-space `rect` on a 792 pt tall unturned page, in pixels at `scale`.
fn pixels_of(rect: [f64; 4], scale: f64) -> [f64; 4] {
    [
        rect[0] * scale,
        (792.0 - rect[3]) * scale,
        rect[2] * scale,
        (792.0 - rect[1]) * scale,
    ]
}

/// Every non-white pixel lies inside the pixel box `r`, give or take a
/// pixel, and some pixel does.
fn assert_ink_inside(image: &RgbaImage, r: [f64; 4], label: &str) {
    let ink = ink_box(&image.pixels, image.width, image.height)
        .unwrap_or_else(|| panic!("{label}: nothing drawn"));
    assert!(
        f64::from(ink[0]) >= r[0] - 1.0
            && f64::from(ink[1]) >= r[1] - 1.0
            && f64::from(ink[2]) <= r[2] + 1.0
            && f64::from(ink[3]) <= r[3] + 1.0,
        "{label}: ink {ink:?} outside {r:?}"
    );
}

fn view_image(session: &mut AppSession, scale: f64) -> RgbaImage {
    session
        .view()
        .unwrap()
        .render(0, scale, RenderOptions::default())
        .unwrap()
}

// ------------------------------------------------------------------ add

/// Checks the stored appearance of annotation `number`, the only one on a
/// page drawn through the view at 2 px per point: it paints only inside
/// /Rect, in the colour asked for.
fn assert_drawn_in_colour(session: &mut AppSession, number: u32, kind: &str) {
    let dict = annot(session, number);
    let doc = document(session);
    let rect = annots::rect(doc, dict.get(b"Rect".as_slice()).unwrap()).unwrap();
    let color = dict.get(b"C".as_slice()).map(|c| nums(doc, Some(c)));
    let image = view_image(session, 2.0);
    assert_ink_inside(&image, pixels_of(rect, 2.0), kind);
    let ink = ink_box(&image.pixels, image.width, image.height).unwrap();
    let mut counts: std::collections::HashMap<[u8; 3], usize> = Default::default();
    let mut darkest = [255u8; 3];
    for y in ink[1]..=ink[3] {
        for x in ink[0]..=ink[2] {
            let p = pixel(&image, x, y);
            if !inked(p) {
                continue;
            }
            *counts.entry(p).or_default() += 1;
            let sum = |p: [u8; 3]| p.iter().map(|&c| u32::from(c)).sum::<u32>();
            if sum(p) < sum(darkest) {
                darkest = p;
            }
        }
    }
    let common = counts.iter().max_by_key(|&(p, n)| (*n, *p)).unwrap().0;
    match (kind, color.as_deref()) {
        // A highlight is a light colour multiplied onto white: yellowish.
        ("Highlight", _) => assert!(
            common[0] > 200 && common[1] > 200 && common[2] < 100,
            "{kind}: {common:?}"
        ),
        // A note: its background in /C.
        ("Text", Some(c)) => {
            for (got, want) in common.iter().zip(c) {
                assert!(
                    (f64::from(*got) - want * 255.0).abs() < 8.0,
                    "{kind}: {common:?}"
                );
            }
        }
        // Black text, no colour of its own.
        ("FreeText", _) => {
            assert!(darkest.iter().all(|&v| v < 64), "{kind}: {darkest:?}");
            for (p, _) in counts {
                let spread = p.iter().max().unwrap() - p.iter().min().unwrap();
                assert!(spread < 24, "{kind}: grey only, not {p:?}");
            }
        }
        // Strokes in /C over white: a channel /C has full stays full, the
        // others reach dark at the stroke's core.
        (_, Some(c)) => {
            for (p, _) in counts {
                for (channel, want) in p.iter().zip(c) {
                    if *want >= 1.0 {
                        assert!(*channel >= 240, "{kind}: {p:?} is not in {c:?}");
                    }
                }
            }
            for (channel, want) in darkest.iter().zip(c) {
                if *want <= 0.0 {
                    assert!(*channel < 100, "{kind}: {darkest:?} is not {c:?}");
                }
            }
        }
        (_, None) => panic!("{kind} has a /C"),
    }
}

#[test]
fn every_kind_is_written_with_its_appearance() {
    let (mut engine, mut session) = session_with(blank());
    let quad = json!([[100, 100, 200, 100, 100, 120, 200, 120]]);
    let cases = [
        (
            json!({"type": "Text", "point": [100, 100], "contents": "note"}),
            "add note",
        ),
        (
            json!({"type": "Highlight", "quads": quad, "contents": "hl"}),
            "add highlight",
        ),
        (
            json!({"type": "Underline", "quads": quad, "contents": ""}),
            "add underline",
        ),
        (
            json!({"type": "StrikeOut", "quads": quad, "contents": ""}),
            "add strikeout",
        ),
        (
            json!({"type": "FreeText", "point": [300, 100], "contents": "free text"}),
            "add text box",
        ),
        (
            json!({"type": "Square", "rect": [100, 200, 200, 260], "contents": "sq"}),
            "add square",
        ),
        (
            json!({"type": "Circle", "rect": [100, 200, 200, 260], "contents": ""}),
            "add circle",
        ),
        (
            json!({"type": "Line", "points": [[100, 400], [300, 420]], "contents": ""}),
            "add line",
        ),
        (
            json!({"type": "Ink", "strokes": [[[100, 450], [150, 500], [200, 450]]], "contents": "ink"}),
            "add ink",
        ),
        (
            json!({"type": "Stamp", "rect": [100, 550, 260, 610], "icon": "Draft", "contents": "st"}),
            "add stamp",
        ),
    ];
    // The request geometry in user space (the page is 792 pt tall).
    let quads = [100.0, 672.0, 200.0, 692.0];
    let shape = [100.0, 532.0, 200.0, 592.0];
    let stamp = appearance::icon_fit([100.0, 182.0, 260.0, 242.0]);
    for (index, (request, step)) in cases.iter().cloned().enumerate() {
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
        for junk in [b"CL".as_slice(), b"T", b"M", b"NM"] {
            assert!(!dict.contains_key(junk), "{kind} has no {junk:?}");
        }
        let rect = nums(doc, dict.get(b"Rect".as_slice()));
        let rect = [rect[0], rect[1], rect[2], rect[3]];
        let rd = dict.get(b"RD".as_slice()).map(|rd| nums(doc, Some(rd)));
        let encloses = |inner: [f64; 4], most: f64| {
            (0..2).all(|i| rect[i] <= inner[i] + 1e-6 && rect[i] >= inner[i] - most)
                && (2..4).all(|i| rect[i] >= inner[i] - 1e-6 && rect[i] <= inner[i] + most)
        };
        match kind {
            "Text" => assert_near(&rect, &[100.0, 672.0, 120.0, 692.0], kind),
            "Highlight" | "Underline" | "StrikeOut" => {
                assert!(encloses(quads, 20.0), "{kind}: {rect:?}")
            }
            "FreeText" => assert_near(&rect, &[300.0, 652.0, 460.0, 692.0], kind),
            "Square" | "Circle" => {
                let allowance = appearance::border_allowance(1.0);
                assert_near(&rect, &appearance::grow(shape, allowance), kind);
                assert_eq!(rd, Some(vec![allowance; 4]), "{kind}");
            }
            "Line" => assert!(
                encloses(appearance::grow([100.0, 372.0, 300.0, 392.0], 0.5), 30.0),
                "{kind}: {rect:?}"
            ),
            "Ink" => {
                let allowance = appearance::ink_allowance(1.5);
                assert_near(
                    &rect,
                    &appearance::grow([100.0, 292.0, 200.0, 342.0], allowance),
                    kind,
                );
                assert_eq!(rd, Some(vec![allowance; 4]), "{kind}");
            }
            _ => {
                assert_near(&rect, &stamp, kind);
                let aspect = (rect[2] - rect[0]) / (rect[3] - rect[1]);
                assert!((2.0..=5.0).contains(&aspect), "{kind}: {rect:?}");
            }
        }
        let (form, content) = appearance(&session, number);
        checked(content.as_bytes(), kind);
        let bbox = nums(doc, form.get(b"BBox".as_slice()));
        let bbox = [bbox[0], bbox[1], bbox[2], bbox[3]];
        match kind {
            // Fixed-size icons: a box of their own, the /Rect's shape.
            "Text" | "Stamp" => {
                let (bw, bh) = (bbox[2] - bbox[0], bbox[3] - bbox[1]);
                let (rw, rh) = (rect[2] - rect[0], rect[3] - rect[1]);
                assert!((bw / bh - rw / rh).abs() < 1e-3, "{kind}: {bbox:?}");
            }
            _ => {
                let inner = annots::inset(rect, rd.as_ref().map(|r| [r[0], r[1], r[2], r[3]]));
                assert!(
                    (0..2).all(|i| bbox[i] <= inner[i] + 1e-6)
                        && (2..4).all(|i| bbox[i] >= inner[i] - 1e-6),
                    "{kind}: /BBox {bbox:?} holds /Rect less /RD {inner:?}"
                );
            }
        }
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
        &appearance::icon_fit([100.0, 550.0, 260.0, 610.0]),
        "stamp",
    );

    // Each kind alone on a page: where and in what colour it paints, and
    // what text it shows.
    for (request, _) in cases {
        let (mut engine, mut session) = session_with(blank());
        let mut request = request;
        request["op"] = json!("annot-add");
        request["page"] = json!(0);
        op(&mut engine, &mut session, request.clone());
        let kind = request["type"].as_str().unwrap();
        let number = last_id(&mut engine, &mut session);
        assert_drawn_in_colour(&mut session, number, kind);
        match kind {
            "Stamp" => assert_eq!(drawn_lines(&session, number), ["DRAFT"]),
            "FreeText" => assert_eq!(drawn_lines(&session, number), ["free text"]),
            _ => {}
        }
    }
}

#[test]
fn a_note_brings_a_popup_and_the_dictionaries_keep_their_order() {
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
    // Its icon form is square, from the origin (its size is the icon's).
    let (form, _) = appearance(&session, 5);
    let bbox = nums(doc, form.get(b"BBox".as_slice()));
    assert!(
        bbox[0] == 0.0 && bbox[1] == 0.0 && bbox[2] > 0.0 && bbox[2] == bbox[3],
        "{bbox:?}"
    );
    // Listing skips the popup.
    assert_eq!(items(&mut engine, &mut session).len(), 1);
}

/// The width a fit box gives `lines`: the widest Helvetica 12 advance
/// plus 0.5.
fn fit_width(lines: &[&str]) -> f64 {
    lines
        .iter()
        .map(|line| Base14::Helvetica.width(&fonts::encode(line), 12.0))
        .fold(0.0, f64::max)
        + 0.5
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
    // The point is the left edge and the first line's vertical centre.
    let lh = appearance::line_height(12.0);
    let w = fit_width(&["fit me", "line2"]);
    let top = 200.0 - lh / 2.0;
    assert_near(
        &rect_of(&item),
        &[300.0, top, 300.0 + w, top + 2.0 * lh],
        "fit",
    );
    let number = item["id"].as_u64().unwrap() as u32;
    assert_eq!(drawn_lines(&session, number), ["fit me", "line2"]);

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
    // after a space, a CRLF is one break.
    op(
        &mut engine,
        &mut session,
        json!({"op": "annot-add", "page": 0, "type": "FreeText", "point": [600, 780],
            "contents": "a long line of text that will need to wrap across the box width\r\nend"}),
    );
    let item = items(&mut engine, &mut session).pop().unwrap();
    assert_eq!(rect_of(&item), [452.0, 752.0, 612.0, 792.0]);
    // The 40 pt box shows the lines that fit in it, wrapped there.
    let shown = drawn_lines(&session, item["id"].as_u64().unwrap() as u32);
    let wrapped = [
        "a long line of text that will",
        "need to wrap across the box",
        "width",
        "end",
    ];
    assert!(
        (2..=wrapped.len()).contains(&shown.len())
            && shown.iter().zip(wrapped).all(|(line, want)| line == want),
        "{shown:?}"
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
    // The top left kept; two lines tall, as wide as the wider line.
    let lh = appearance::line_height(12.0);
    let w = fit_width(&["changed", "second line"]);
    assert_near(
        &rect_of(&item),
        &[300.0, 100.0, 300.0 + w, 100.0 + 2.0 * lh],
        "refit",
    );
    assert_eq!(drawn_lines(&session, number), ["changed", "second line"]);
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
        // popup included: the popup left this page's /Annots, and the note
        // still names it.
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
fn listing_reports_kinds_and_geometry_in_page_space() {
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
    checked(content.as_bytes(), "turned");
    // The text box's ink lies inside its listed rect (page space, as the
    // user sees the page).
    let listed = rect_of(&items(&mut engine, &mut session)[1]);
    let image = view_image(&mut session, 2.0);
    let mut found: Option<[f64; 4]> = None;
    for y in 0..image.height {
        for x in 0..image.width {
            let (px, py) = (f64::from(x) / 2.0, f64::from(y) / 2.0);
            let near = px >= listed[0] - 40.0
                && px <= listed[2] + 40.0
                && py >= listed[1] - 40.0
                && py <= listed[3] + 40.0;
            if near && inked(pixel(&image, x, y)) {
                let b = found.get_or_insert([px, py, px, py]);
                *b = [b[0].min(px), b[1].min(py), b[2].max(px), b[3].max(py)];
            }
        }
    }
    let ink = found.expect("the text box draws");
    assert!(
        ink[0] >= listed[0] - 0.5
            && ink[1] >= listed[1] - 0.5
            && ink[2] <= listed[2] + 0.5
            && ink[3] <= listed[3] + 0.5,
        "{ink:?} inside {listed:?}"
    );
    // It reads `turned`, its letters advancing left to right as the user
    // sees the page.
    let lines = text_lines(&flattened(&serialized(&session), number));
    assert_eq!(lines.len(), 1, "{lines:?}");
    let (text, origins) = &lines[0];
    assert_eq!(text, "turned");
    for pair in origins.windows(2) {
        assert!(pair[1][0] > pair[0][0], "{origins:?}");
        assert!((pair[1][1] - pair[0][1]).abs() < 0.5, "{origins:?}");
    }
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
    // A foreign note: a 16 x 16 form in a 20 x 20 rect.
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
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
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
    use tandem_pdf_engine::app::annots::appearance::Mark;
    // 400 quads 590 points long and a thousandth high: a wave whose step
    // follows the height would take over a billion points.
    let quads: Vec<annots::Quad> = (0..400)
        .map(|i| {
            let y = 100.0 + f64::from(i);
            [(10.0, y + 0.001), (600.0, y + 0.001), (10.0, y), (600.0, y)]
        })
        .collect();
    let (form, rect) = appearance::text_line(Mark::Squiggly, &quads, None, 1.0);
    assert!(form.content.len() < 2 << 20, "{}", form.content.len());
    assert!(rect.iter().all(|v| v.is_finite()));
    // An ordinary squiggle: a 14 pt high, 140 pt long quad waves with a
    // handful of vertices, neither a straight line nor a dense scribble.
    let one = [[
        (100.0, 614.0),
        (240.0, 614.0),
        (100.0, 600.0),
        (240.0, 600.0),
    ]];
    let (form, _) = appearance::text_line(Mark::Squiggly, &one, None, 1.0);
    let ops = checked(&form.content, "squiggly");
    let vertices = count(&ops, &["m", "l", "c", "v", "y"]);
    assert!((10..=200).contains(&vertices), "{vertices}");
    // Drawn without an appearance, its ink spans the quad.
    let bytes = page_with(
        "",
        &[
            "<< /Type /Annot /Subtype /Squiggly /Rect [0 0 1 1] /C [1 0 0] /QuadPoints [100 614 240 614 100 600 240 600] >>",
        ],
    );
    let (_, mut session) = session_with(bytes);
    let image = view_image(&mut session, 4.0);
    let ink = ink_box(&image.pixels, image.width, image.height).unwrap();
    assert!(f64::from(ink[2] - ink[0]) / 4.0 >= 0.9 * 140.0, "{ink:?}");

    // The thin quads in a file without an appearance: the view draws them
    // at once.
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
    checked(content.as_bytes(), "changed");
    let doc = document(&session);
    assert_eq!(
        nums(doc, form.get(b"BBox".as_slice())),
        [100.0, 600.0, 300.0, 700.0]
    );
    assert_eq!(
        nums(doc, annot(&session, 5).get(b"Rect".as_slice())),
        [100.0, 600.0, 300.0, 700.0]
    );
    // Everything is drawn inside /Rect less /RD, the border (width 1 by
    // default) along its edges.
    let inner = [120.0, 610.0, 270.0, 690.0];
    let box_ = pixels_of(inner, 2.0);
    let image = view_image(&mut session, 2.0);
    assert_ink_inside(&image, box_, "changed");
    let ink = ink_box(&image.pixels, image.width, image.height).unwrap();
    for (got, want) in ink.iter().zip(box_) {
        assert!((f64::from(*got) - want).abs() <= 4.0, "{ink:?} vs {box_:?}");
    }
    // The text reads `changed`, inside that box (page space: y down).
    let lines = text_lines(&flattened(&serialized(&session), 5));
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].0, "changed");
    for o in &lines[0].1 {
        assert!(
            o[0] >= 120.0 && o[0] <= 270.0 && o[1] >= 102.0 && o[1] <= 182.0,
            "{o:?}"
        );
    }
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
            // The listed box: /Rect less /RD.
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
fn annotations_without_an_appearance_list_and_save_the_synthesized_rect() {
    // The box each lists, and the /Rect and /RD an edit writes with the
    // appearance it makes.
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
    let kinds: Vec<&str> = listed.iter().map(|i| i["type"].as_str().unwrap()).collect();
    assert_eq!(
        kinds,
        [
            "Text", "Stamp", "Square", "Circle", "Caret", "Redact", "FreeText"
        ]
    );
    // A note at its size from the top left; a Square and Circle less half
    // their border; a FreeText its /Rect.
    let fixed = [
        (0, [72.0, 342.0, 92.0, 362.0]),
        (2, [100.5, 642.5, 199.5, 691.5]),
        (3, [252.0, 644.0, 348.0, 690.0]),
        (6, [100.0, 92.0, 300.0, 192.0]),
    ];
    for (index, rect) in fixed {
        assert_near(&rect_of(&listed[index]), &rect, kinds[index]);
    }
    // A stamp: its box fitted about the centre of /Rect, as wide as /Rect.
    let stamp = rect_of(&listed[1]);
    assert_near(
        &stamp,
        &appearance::icon_fit([100.0, 192.0, 300.0, 392.0]),
        "stamp",
    );
    let (w, h) = (stamp[2] - stamp[0], stamp[3] - stamp[1]);
    assert!(
        (w - 200.0).abs() < 1e-6 && (2.0..=5.0).contains(&(w / h)),
        "{stamp:?}"
    );
    assert!(
        ((stamp[1] + stamp[3]) / 2.0 - 292.0).abs() < 1e-6,
        "{stamp:?}"
    );
    // A Redact mark: its quads (page space [110 102 200 112]), give or
    // take 2 pt on each side.
    let redact = rect_of(&listed[5]);
    let quad = [110.0, 102.0, 200.0, 112.0];
    for i in 0..2 {
        assert!(
            redact[i] <= quad[i] && redact[i] >= quad[i] - 2.0,
            "{redact:?}"
        );
        assert!(
            redact[i + 2] >= quad[i + 2] && redact[i + 2] <= quad[i + 2] + 2.0,
            "{redact:?}"
        );
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
        (194..stamp[1].floor() as u32 - 1).all(|y| !stamp_region(200, y)),
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
    assert_near(
        &rect,
        &appearance::icon_fit([100.0, 400.0, 300.0, 600.0]),
        "stamp",
    );
    let (rect, rd) = set(&mut engine, &mut session, 7);
    assert_eq!(
        (rect, rd),
        (vec![100.0, 100.0, 200.0, 150.0], Some(vec![0.5; 4]))
    );
    let (_, rd) = set(&mut engine, &mut session, 8);
    assert_eq!(rd, Some(vec![2.0; 4]));
    // The listing is unchanged by the edits that wrote those rects.
    let after = items(&mut engine, &mut session);
    for index in 0..4 {
        assert_near(
            &rect_of(&after[index]),
            &rect_of(&listed[index]),
            "after set",
        );
    }
}

/// Spec decision A3: a Caret without an appearance lists, and draws in,
/// its whole /Rect.
#[test]
fn a_caret_without_an_appearance_lists_its_rect() {
    let bytes = page_with(
        "",
        &["<< /Type /Annot /Subtype /Caret /Rect [50 50 70 70] /C [0 0 1] >>"],
    );
    let (mut engine, mut session) = session_with(bytes);
    assert_near(
        &rect_of(&items(&mut engine, &mut session)[0]),
        &[50.0, 722.0, 70.0, 742.0],
        "caret",
    );
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
    // So does a link.
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

/// The /Resources /Font of a form: resource names and font dictionaries.
fn form_fonts(doc: &Document<'_>, form: &Dictionary) -> Vec<(Vec<u8>, Dictionary)> {
    let resources = doc.dict_get(form, b"Resources").as_dict().unwrap();
    let fonts = doc.dict_get(resources, b"Font").as_dict().unwrap();
    fonts
        .iter()
        .map(|(name, font)| (name.clone(), doc.resolve(font).as_dict().unwrap().clone()))
        .collect()
}

/// Some font names `glyphs` in its /Differences (AGL names), or the fonts
/// beside the /DA font map their codes to Unicode through /ToUnicode.
fn fonts_name_their_glyphs(doc: &Document<'_>, fonts: &[(Vec<u8>, Dictionary)], glyphs: &[&str]) {
    let named = fonts.iter().any(|(_, font)| {
        let differences = doc
            .dict_get(font, b"Encoding")
            .as_dict()
            .and_then(|e| doc.dict_get(e, b"Differences").as_array());
        differences.is_some_and(|d| {
            glyphs.iter().all(|g| {
                d.iter()
                    .any(|n| doc.resolve(n).as_name() == Some(g.as_bytes()))
            })
        })
    });
    let mapped = fonts
        .iter()
        .filter(|(_, font)| doc.dict_get(font, b"Subtype").as_name() != Some(b"Type0"))
        .filter(|(name, _)| name.as_slice() != b"Helv")
        .all(|(_, font)| font.contains_key(b"ToUnicode".as_slice()));
    assert!(named || mapped, "{glyphs:?}");
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
    checked(
        &content.chars().map(|c| c as u8).collect::<Vec<_>>(),
        "scripts",
    );
    let doc = document(&session);
    let fonts = form_fonts(doc, &form);
    // The /DA font and at least one more font for each script: Greek,
    // Cyrillic, CJK and Latin letters WinAnsi lacks.
    assert!(fonts.iter().any(|(name, _)| name == b"Helv"));
    assert!(fonts.len() >= 5, "{} fonts", fonts.len());
    // The text reads back whole: no `?`.
    assert_eq!(drawn_lines(&session, number).join(" "), text);
    // A CJK font is three objects: font, CIDFont, descriptor.
    let resources = doc.dict_get(&form, b"Resources").as_dict().unwrap();
    let font_refs = doc.dict_get(resources, b"Font").as_dict().unwrap();
    let type0: Vec<ObjectId> = font_refs
        .values()
        .filter_map(Object::as_reference)
        .filter(|&f| {
            let font = doc.resolve_id(f).as_dict();
            font.is_some_and(|f| doc.dict_get(f, b"Subtype").as_name() == Some(b"Type0"))
        })
        .collect();
    assert_eq!(type0.len(), 1, "one indirect CJK font");
    let cjk = doc.resolve_id(type0[0]).as_dict().unwrap();
    let descendant = doc.dict_get(cjk, b"DescendantFonts").as_array().unwrap()[0]
        .as_reference()
        .expect("an object");
    let descendant = doc.resolve_id(descendant).as_dict().unwrap();
    let descriptor = descendant.get(b"FontDescriptor".as_slice()).unwrap();
    assert!(descriptor.as_reference().is_some());
    fonts_name_their_glyphs(doc, &fonts, &["zdotaccent", "lslash"]);
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
        assert!(region != question, "{letter} is not a ?");
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
    assert_eq!(drawn_lines(&session, 5), ["Привет мир"]);
    let (form, _) = latin1_appearance(&session, 5);
    assert!(form_fonts(document(&session), &form).len() >= 2);
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
    assert_eq!(drawn_lines(&session, 5), ["Ω"]);
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
    for rect in [[10.0, 10.0, 410.0, 60.0], [10.0, 10.0, 600.0, 100.0]] {
        op(
            &mut engine,
            &mut session,
            json!({"op": "annot-add", "page": 0, "type": "Stamp", "rect": rect, "icon": "Approved", "contents": ""}),
        );
        // The icon's aspect (2:1 to 5:1) fitted about the centre: these
        // wide requests keep their height.
        let expected = appearance::icon_fit(rect);
        let listed = rect_of(items(&mut engine, &mut session).last().unwrap());
        assert_near(&listed, &expected, "listed");
        let (w, h) = (listed[2] - listed[0], listed[3] - listed[1]);
        assert!((h - (rect[3] - rect[1])).abs() < 1e-6, "{listed:?}");
        assert!((2.0..=5.0).contains(&(w / h)), "{listed:?}");
        assert!(((listed[0] + listed[2]) / 2.0 - (rect[0] + rect[2]) / 2.0).abs() < 1e-6);
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
    let fit = appearance::icon_fit([300.0, 600.0, 500.0, 650.0]);
    let listed = items(&mut engine, &mut session);
    assert_near(
        &rect_of(&listed[0]),
        &[fit[0], 792.0 - fit[3], fit[2], 792.0 - fit[1]],
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
    assert_near(&rect, &fit, "set");
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
