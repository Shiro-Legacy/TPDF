// SPDX-License-Identifier: MIT
//! The "draw a box, everything inside is gone" redaction redesign
//! (docs/rust-and-ux-plan.md §2.4): R1 (every glyph an area box touches
//! goes, with a half-point tolerance; search marks keep the tight rule),
//! R6 (redact-apply's `regions`, boxes held in memory) and redact-inspect
//! (what apply would remove under each box, for the window's tint).
use serde_json::{Value, json};
use tandem_pdf_engine::app::redact::marks;
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::Object;
use tandem_pdf_engine::stream::StreamLimits;
use tandem_pdf_engine::view::ViewDocument;

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

/// Courier with its widths written out: every glyph 600 units, so at
/// 20 pt each is 12 pt wide.
fn courier() -> String {
    let widths = vec!["600"; 95].join(" ");
    format!(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier /FirstChar 32 /LastChar 126 /Widths [{widths}] /Encoding /WinAnsiEncoding >>"
    )
}

/// One page (object 3, `page` entries added) drawing `content` (object 4)
/// with `/F1` Courier (object 5) and `resources`; more objects from 6.
fn page_with(page: &str, resources: &str, content: &str, more: &[String]) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] {page} /Resources << /Font << /F1 5 0 R >> {resources} >> /Contents 4 0 R >>"
        ),
        stream("", content),
        courier(),
    ];
    objects.extend(more.iter().cloned());
    classic(&objects)
}

/// "ABCDEFGH" at 20 pt from x 100 (user y 600): in page space (y down)
/// each glyph is 12 pt wide (A 100-112 ... D 136-148, E 148-160) and its
/// box runs from y 176 to 196 (0.8 and 0.2 em about the baseline at 192).
const LINE: &str = "BT /F1 20 Tf 100 600 Td (ABCDEFGH) Tj ET";

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

/// redact-apply with `extra` fields; the reply and the copy (empty on a
/// refusal).
fn apply_with(engine: &mut AppEngine, session: &mut AppSession, extra: Value) -> (Value, Vec<u8>) {
    let mut request = json!({"id": 9, "op": "redact-apply", "sanitize": all(true),
        "entropy": {"$buffer": 0}});
    if let (Some(fields), Value::Object(more)) = (request.as_object_mut(), extra) {
        fields.extend(more);
    }
    let (reply, mut outputs) = engine.dispatch(session, request, vec![vec![7u8; 32]]);
    let bytes = match reply["ok"] == true {
        true => outputs.remove(0),
        false => Vec::new(),
    };
    (reply, bytes)
}

fn apply(engine: &mut AppEngine, session: &mut AppSession) -> Vec<u8> {
    let (reply, bytes) = apply_with(engine, session, json!({}));
    assert_eq!(reply["ok"], true, "{reply}");
    bytes
}

fn apply_regions(engine: &mut AppEngine, session: &mut AppSession, regions: Value) -> Vec<u8> {
    let (reply, bytes) = apply_with(engine, session, json!({ "regions": regions }));
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

fn view_text(bytes: &[u8]) -> String {
    let view = ViewDocument::open(bytes.to_vec(), "").unwrap();
    (0..view.page_count())
        .map(|p| view.structured_text(p).unwrap().as_text())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every object a copy holds, written out: strings and names anywhere,
/// and every stream decoded.
fn everything(bytes: &[u8]) -> Vec<u8> {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
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

// ---- R1 -------------------------------------------------------------------

#[test]
fn a_glyph_cut_by_an_area_box_edge_goes_whole() {
    // Two points into D: before R1 its central half (139-145) was not
    // met and D stayed in the file under the black box.
    let (mut engine, mut session) = session_with(page_with("", "", LINE, &[]));
    mark(&mut engine, &mut session, 0, [50.0, 150.0, 138.0, 220.0]);
    let text = view_text(&apply(&mut engine, &mut session));
    assert!(text.contains("EFGH"), "{text}");
    assert!(!text.contains('D'), "{text}");
    assert!(!text.contains('A'), "{text}");
}

#[test]
fn a_box_grazing_a_glyph_by_less_than_half_a_point_keeps_it() {
    let (mut engine, mut session) = session_with(page_with("", "", LINE, &[]));
    mark(&mut engine, &mut session, 0, [50.0, 150.0, 136.3, 220.0]);
    let text = view_text(&apply(&mut engine, &mut session));
    assert!(text.contains("DEFGH"), "{text}");
    assert!(!text.contains('C'), "{text}");
}

#[test]
fn a_box_grazing_the_line_above_keeps_its_descenders() {
    // Line one's boxes run to y 196; line two (user y 570) from 206 to 226.
    let content = format!("{LINE} BT /F1 20 Tf 100 570 Td (LINETWO) Tj ET");
    for (top, kept) in [(195.7, true), (195.0, false)] {
        let (mut engine, mut session) = session_with(page_with("", "", &content, &[]));
        mark(&mut engine, &mut session, 0, [50.0, top, 400.0, 240.0]);
        let text = view_text(&apply(&mut engine, &mut session));
        assert!(!text.contains("LINETWO"), "{top}: {text}");
        assert_eq!(text.contains("ABCDEFGH"), kept, "{top}: {text}");
    }
}

#[test]
fn search_marks_keep_the_tight_rule() {
    let (mut engine, mut session) = session_with(page_with("", "", LINE, &[]));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-search", "text": "CD"}),
    );
    let text = view_text(&apply(&mut engine, &mut session));
    // The two-point margin reaches into B and E; they stay.
    assert!(text.contains("AB"), "{text}");
    assert!(text.contains("EFGH"), "{text}");
    assert!(!text.contains("CD"), "{text}");
}

#[test]
fn a_mark_knows_it_is_an_area_mark_after_a_save() {
    let (mut engine, mut session) = session_with(page_with("", "", LINE, &[]));
    mark(&mut engine, &mut session, 0, [50.0, 150.0, 138.0, 220.0]);
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-search", "text": "GH"}),
    );
    let request = json!({"id": 2, "op": "save", "entropy": {"$buffer": 0}});
    let (reply, outputs) = engine.dispatch(&mut session, request, vec![vec![1u8; 32]]);
    assert_eq!(reply["ok"], true, "{reply}");
    let doc = Document::open(&outputs[0], DocumentLimits::default()).unwrap();
    let found = marks::marks(&doc);
    assert_eq!(found.len(), 2);
    assert!(found[0].by_area && found[0].search.is_none());
    assert!(!found[1].by_area && found[1].search.as_deref() == Some("GH"));
    // A foreign mark with quads and no tag (a text mark) is not an area mark.
    let foreign = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R 5 0 R] >>"
            .to_string(),
        "<< /Type /Annot /Subtype /Redact /Rect [10 10 50 30] /QuadPoints [10 30 50 30 10 10 50 10] >>"
            .to_string(),
        "<< /Type /Annot /Subtype /Redact /Rect [10 10 50 30] >>".to_string(),
    ]);
    let doc = Document::open(&foreign, DocumentLimits::default()).unwrap();
    let found = marks::marks(&doc);
    assert_eq!((found[0].by_area, found[1].by_area), (false, true));
}

// ---- R6 -------------------------------------------------------------------

#[test]
fn regions_redact_without_marks_and_leave_the_session_alone() {
    let (mut engine, mut session) = session_with(page_with("", "", LINE, &[]));
    let before = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    let bytes = apply_regions(
        &mut engine,
        &mut session,
        json!([{"page": 0, "rect": [50.0, 150.0, 138.0, 220.0]}]),
    );
    let text = view_text(&bytes);
    assert!(text.contains("EFGH") && !text.contains('D'), "{text}");
    assert!(!holds(&everything(&bytes), "ABCD"));
    // Nothing was written into the session: no marks, no journal step.
    let after = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(before, after);
    assert_eq!(after["marks"], 0);
    // The copy holds no mark either.
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    assert_eq!(marks::count(&doc), 0);
}

#[test]
fn regions_are_applied_with_the_documents_marks() {
    let content = format!(
        "{LINE} BT /F1 20 Tf 100 500 Td (KEEPLINE) Tj ET BT /F1 20 Tf 100 400 Td (MARKLINE) Tj ET"
    );
    let (mut engine, mut session) = session_with(page_with("", "", &content, &[]));
    // MARKLINE: baseline y 392, box 376-396.
    mark(&mut engine, &mut session, 0, [50.0, 370.0, 400.0, 400.0]);
    let (reply, bytes) = apply_with(
        &mut engine,
        &mut session,
        json!({"regions": [{"page": 0, "rect": [50.0, 150.0, 400.0, 220.0]}]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["marks"], 2, "{reply}");
    let text = view_text(&bytes);
    assert!(text.contains("KEEPLINE"), "{text}");
    assert!(
        !text.contains("MARKLINE") && !text.contains("ABCD"),
        "{text}"
    );
}

#[test]
fn the_old_form_of_redact_apply_is_unchanged() {
    let (mut engine, mut session) = session_with(page_with("", "", LINE, &[]));
    let (reply, _) = apply_with(&mut engine, &mut session, json!({}));
    assert_eq!(reply["reason"], "no-marks", "{reply}");
    let (reply, _) = apply_with(&mut engine, &mut session, json!({"regions": []}));
    assert_eq!(reply["reason"], "no-marks", "{reply}");
}

#[test]
fn bad_regions_are_refused_with_what_was_wrong() {
    let (mut engine, mut session) = session_with(page_with("", "", LINE, &[]));
    for regions in [
        json!({"page": 0}),
        json!([5]),
        json!([{"page": 3, "rect": [0, 0, 10, 10]}]),
        json!([{"page": 0, "rect": [0, 0, 10]}]),
        json!([{"page": 0, "rect": [10, 10, 10.5, 10.5]}]),
    ] {
        let (reply, _) = apply_with(
            &mut engine,
            &mut session,
            json!({ "regions": regions.clone() }),
        );
        assert_eq!(reply["reason"], "invalid", "{regions}: {reply}");
        assert_eq!(reply["detail"], "regions", "{regions}: {reply}");
        let reply = op(
            &mut engine,
            &mut session,
            json!({"op": "redact-inspect", "regions": regions.clone()}),
        );
        assert_eq!(reply["detail"], "regions", "{regions}: {reply}");
    }
    let many: Vec<Value> = (0..2001)
        .map(|_| json!({"page": 0, "rect": [0, 0, 10, 10]}))
        .collect();
    let (reply, _) = apply_with(&mut engine, &mut session, json!({ "regions": many }));
    assert_eq!(reply["reason"], "too-many", "{reply}");
}

// ---- redact-inspect --------------------------------------------------------

fn inspect(engine: &mut AppEngine, session: &mut AppSession, regions: Value) -> Vec<Value> {
    let reply = ok(
        engine,
        session,
        json!({"op": "redact-inspect", "regions": regions}),
    );
    reply["regions"].as_array().unwrap().clone()
}

#[test]
fn inspect_shows_the_glyphs_apply_takes_and_what_they_spell() {
    let (mut engine, mut session) = session_with(page_with("", "", LINE, &[]));
    let found = inspect(
        &mut engine,
        &mut session,
        json!([
            {"page": 0, "rect": [50.0, 150.0, 138.0, 220.0]},
            {"page": 0, "rect": [50.0, 150.0, 136.3, 220.0]},
            {"page": 0, "rect": [300.0, 300.0, 400.0, 400.0]},
        ]),
    );
    assert_eq!(found.len(), 3);
    assert_eq!(found[0]["text"], "ABCD", "{}", found[0]);
    // One quad for the run A..D: x 100 to 148.
    let glyphs = found[0]["glyphs"].as_array().unwrap();
    assert_eq!(glyphs.len(), 1, "{}", found[0]);
    let xs: Vec<f64> = (0..4).map(|i| glyphs[0][2 * i].as_f64().unwrap()).collect();
    let (lo, hi) = xs
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), &x| (a.min(x), b.max(x)));
    assert!(
        (lo - 100.0).abs() < 0.01 && (hi - 148.0).abs() < 0.01,
        "{xs:?}"
    );
    assert_eq!(found[1]["text"], "ABC", "{}", found[1]);
    assert_eq!(found[2]["text"], "");
    assert_eq!(found[2]["glyphs"], json!([]));
    assert_eq!(found[0]["needsFlatten"], false);
    // Inspect writes nothing.
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_eq!(preview["marks"], 0);
}

#[test]
fn inspect_matches_apply_glyph_for_glyph() {
    // Whatever inspect shows goes, and nothing else on the line does.
    let content = format!("{LINE} BT /F1 20 Tf 100 570 Td (LINETWO) Tj ET");
    for rect in [
        [50.0, 150.0, 138.0, 220.0],
        [113.0, 190.0, 150.0, 210.0],
        [50.0, 195.0, 400.0, 240.0],
        [130.0, 100.0, 131.0, 300.0],
    ] {
        let (mut engine, mut session) = session_with(page_with("", "", &content, &[]));
        let found = inspect(
            &mut engine,
            &mut session,
            json!([{"page": 0, "rect": rect}]),
        );
        let shown = found[0]["text"].as_str().unwrap().replace('\n', "");
        let bytes = apply_regions(
            &mut engine,
            &mut session,
            json!([{"page": 0, "rect": rect}]),
        );
        let left = view_text(&bytes);
        let before = "ABCDEFGHLINETWO";
        let kept: String = left.chars().filter(|c| c.is_ascii_uppercase()).collect();
        assert_eq!(
            kept.len() + shown.len(),
            before.len(),
            "{rect:?}: shown {shown:?}, left {left:?}"
        );
        for c in shown.chars() {
            assert!(
                kept.matches(c).count() < before.matches(c).count(),
                "{rect:?}: {c}"
            );
        }
    }
}

#[test]
fn inspect_lists_pictures_shapes_and_annotations_under_a_box() {
    let image = binary(
        "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8",
        &[0, 60, 120, 200],
    );
    let widget = "<< /Type /Annot /Subtype /Widget /FT /Tx /T (account) /V (12345678) /Rect [100 400 300 420] /P 3 0 R >>";
    let note = "<< /Type /Annot /Subtype /FreeText /Contents (call   me\nlater) /DA (/Helv 10 Tf 0 g) /Rect [100 300 300 330] >>";
    let content = "q 100 0 0 100 100 500 cm /Im Do Q 0 0 1 rg 50 450 400 20 re f";
    let bytes = page_with(
        "/Annots [7 0 R 8 0 R]",
        "/XObject << /Im 6 0 R >>",
        content,
        &[image, widget.to_string(), note.to_string()],
    );
    let (mut engine, mut session) = session_with(bytes);
    // Page y down: the picture is 192..292, the bar 322..342, the widget
    // 372..392, the note 462..492. The box covers x 150-250, y 250-480.
    let found = inspect(
        &mut engine,
        &mut session,
        json!([{"page": 0, "rect": [150.0, 250.0, 250.0, 480.0]}]),
    );
    let found = &found[0];
    assert_eq!(
        found["images"],
        json!([{"rect": [150.0, 250.0, 200.0, 292.0], "whole": false}]),
        "{found}"
    );
    assert_eq!(
        found["shapes"],
        json!([{"rect": [150.0, 322.0, 250.0, 342.0], "kind": "path"}]),
        "{found}"
    );
    let annots = found["annots"].as_array().unwrap();
    assert_eq!(annots.len(), 2, "{found}");
    assert_eq!(annots[0]["kind"], "widget");
    assert_eq!(annots[0]["label"], "account");
    assert_eq!(annots[1]["kind"], "freetext");
    assert_eq!(annots[1]["label"], "call me later");
}

#[test]
fn inspect_on_the_redact_fixture_finds_every_surface_of_page_one() {
    let (mut engine, mut session) = session_with(fixture("redact.pdf"));
    let started = std::time::Instant::now();
    let found = inspect(
        &mut engine,
        &mut session,
        json!([{"page": 0, "rect": [0, 0, 612, 792]}]),
    );
    let spent = started.elapsed();
    let found = &found[0];
    assert!(
        found["text"].as_str().unwrap().contains("P0B-SECRET-7731"),
        "{found}"
    );
    assert!(!found["images"].as_array().unwrap().is_empty(), "{found}");
    let kinds: Vec<&str> = (found["annots"].as_array().unwrap().iter())
        .filter_map(|a| a["kind"].as_str())
        .collect();
    assert!(
        kinds.contains(&"widget") && kinds.contains(&"freetext"),
        "{kinds:?}"
    );
    // Cheap enough for a pointer-up (debug build, shared machine).
    assert!(spent.as_millis() < 2000, "{spent:?}");
}

#[test]
fn inspect_works_on_a_signed_document_and_refuses_a_locked_one() {
    let (mut engine, mut session) = session_with(fixture("p0b/signed-intact.pdf"));
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [0, 0, 100, 100]}),
    );
    assert_eq!(reply["reason"], "read-only", "{reply}");
    let found = inspect(
        &mut engine,
        &mut session,
        json!([{"page": 0, "rect": [0, 0, 612, 792]}]),
    );
    assert_eq!(found.len(), 1);
    let (mut engine, mut session) = session_with(fixture("encrypted.pdf"));
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "redact-inspect", "regions": [{"page": 0, "rect": [0, 0, 10, 10]}]}),
    );
    assert_eq!(reply["ok"], false, "{reply}");
}
