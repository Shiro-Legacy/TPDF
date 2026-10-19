// SPDX-License-Identifier: MIT
//! The "draw a box, everything inside is gone" redaction redesign
//! (docs/rust-and-ux-plan.md §2.4): R1 (every glyph an area box touches
//! goes, with a half-point tolerance; search marks keep the tight rule),
//! R6 (redact-apply's `regions`, boxes held in memory) and redact-inspect
//! (what apply would remove under each box, for the window's tint).
use serde_json::{Value, json};
use tandem_pdf_engine::app::redact::content::{Detail, DrawKind, Limits};
use tandem_pdf_engine::app::redact::{inspect::MAX_ITEMS, marks, region};
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
    // A foreign mark with quads and no tag is an area mark too, rect by
    // rect (over-removal is the safe error).
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
    assert_eq!((found[0].by_area, found[1].by_area), (true, true));
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
        json!([{"rect": [150.0, 322.0, 250.0, 342.0], "kind": "path", "whole": false}]),
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
    let found = inspect(
        &mut engine,
        &mut session,
        json!([{"page": 0, "rect": [0, 0, 612, 792]}]),
    );
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
    assert_eq!(found["truncated"], false);
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

// ---- review fixes: geometry, bounds, numbers ------------------------------

/// The glyphs a walk of the session's page 0 places: code and quad (to a
/// hundredth of a point).
fn walk_glyphs(session: &mut AppSession) -> Vec<(u32, [i64; 8])> {
    let (view, doc) = session.doc.as_mut().unwrap().view_with_document().unwrap();
    let content = region::page_content(doc, Some(view), 0, Detail::Full, Limits::DEFAULT).unwrap();
    (content.draws.iter())
        .filter_map(|d| match &d.kind {
            DrawKind::Text(t) => Some(
                t.glyphs
                    .iter()
                    .map(|g| (g.code, key(&g.quad.map(f64::from)))),
            ),
            _ => None,
        })
        .flatten()
        .collect()
}

fn key(q: &[f64; 8]) -> [i64; 8] {
    q.map(|v| (v * 100.0).round() as i64)
}

/// Inspect one box, apply it as a region, and compare quad for quad: the
/// glyphs inspect lists are exactly those that left the copy (each show
/// holds one glyph, so no quads are joined). Returns inspect's text and
/// the copy's view text.
fn inspect_against_apply(bytes: Vec<u8>, rect: [f64; 4]) -> (String, String, usize) {
    let (mut engine, mut session) = session_with(bytes);
    let regions = json!([{"page": 0, "rect": rect}]);
    let found = inspect(&mut engine, &mut session, regions.clone());
    let source = walk_glyphs(&mut session);
    let copy = apply_regions(&mut engine, &mut session, regions);
    let (_, mut copied) = session_with(copy.clone());
    let kept = walk_glyphs(&mut copied);
    let mut gone: Vec<[i64; 8]> = source.clone().into_iter().map(|(_, q)| q).collect();
    for (_, q) in &kept {
        let at = gone
            .iter()
            .position(|g| g == q)
            .expect("a kept glyph did not move");
        gone.remove(at);
    }
    let mut shown: Vec<[i64; 8]> = (found[0]["glyphs"].as_array().unwrap().iter())
        .map(|q| {
            let v: Vec<f64> = q
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_f64().unwrap())
                .collect();
            key(&<[f64; 8]>::try_from(v).unwrap())
        })
        .collect();
    gone.sort_unstable();
    shown.sort_unstable();
    assert_eq!(shown, gone, "{rect:?}: {}", found[0]);
    (
        found[0]["text"].as_str().unwrap().to_string(),
        view_text(&copy),
        gone.len(),
    )
}

/// "ABCDEFGH", one show per glyph, at 20 pt under `tm`.
fn one_show_per_glyph(page: &str, tm: &str) -> Vec<u8> {
    let shows: String = "ABCDEFGH".chars().map(|c| format!("({c}) Tj ")).collect();
    page_with(page, "", &format!("BT /F1 20 Tf {tm} Tm {shows}ET"), &[])
}

#[test]
fn inspect_lists_exactly_the_glyphs_apply_removes() {
    let upright = one_show_per_glyph("", "1 0 0 1 100 600");
    for rect in [
        [50.0, 150.0, 138.0, 220.0],
        [113.0, 190.0, 150.0, 210.0],
        [130.0, 100.0, 131.0, 300.0],
        [136.0, 180.0, 137.0, 181.0],
    ] {
        let (_, _, gone) = inspect_against_apply(upright.clone(), rect);
        assert!(gone > 0, "{rect:?}");
    }
    // A 1 by 1 box inside D takes D alone.
    let (text, left, gone) = inspect_against_apply(upright, [140.0, 180.0, 141.0, 181.0]);
    assert_eq!((text.as_str(), gone), ("D", 1), "{left}");
    assert!(left.contains("ABC") && left.contains("EFGH"), "{left}");
}

#[test]
fn inspect_text_of_turned_and_skewed_text_is_what_leaves_the_copy() {
    let c = std::f64::consts::FRAC_1_SQRT_2;
    for tm in [
        format!("{c} {c} {} {c} 100 500", -c),
        "1 0 0.6 1 100 600".to_string(),
        "0 1 -1 0 300 400".to_string(),
    ] {
        let bytes = one_show_per_glyph("", &tm);
        let (mut engine, mut session) = session_with(bytes.clone());
        let before = view_text(&{
            let request = json!({"id": 2, "op": "save", "entropy": {"$buffer": 0}});
            let (_, outputs) = engine.dispatch(&mut session, request, vec![vec![1u8; 32]]);
            outputs[0].clone()
        });
        // A box over the middle of the run, wherever it lies.
        let (view, doc) = session.doc.as_mut().unwrap().view_with_document().unwrap();
        let content =
            region::page_content(doc, Some(view), 0, Detail::Full, Limits::DEFAULT).unwrap();
        let centres: Vec<(f64, f64)> = (content.draws.iter())
            .filter_map(|d| match &d.kind {
                DrawKind::Text(t) => t.glyphs.first().map(|g| {
                    let b = g.bounds;
                    ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0)
                }),
                _ => None,
            })
            .collect();
        let (x, y) = centres[3];
        let rect = [x - 4.0, y - 4.0, x + 4.0, y + 4.0];
        let (text, left, gone) = inspect_against_apply(bytes, rect);
        assert!(gone > 0, "{tm}");
        let squeeze = |s: &str| {
            s.chars()
                .filter(|c| c.is_ascii_uppercase())
                .collect::<String>()
        };
        let mut rest = squeeze(&before);
        for c in squeeze(&text).chars() {
            let at = rest
                .find(c)
                .unwrap_or_else(|| panic!("{tm}: {c} shown but not in the page"));
            rest.remove(at);
        }
        assert_eq!(rest, squeeze(&left), "{tm}: inspect said {text:?}");
    }
}

#[test]
fn inspect_and_apply_agree_on_turned_cropped_and_scaled_pages() {
    for page in [
        "/Rotate 90",
        "/Rotate 270",
        "/CropBox [40 60 560 740]",
        "/UserUnit 2",
    ] {
        let bytes = one_show_per_glyph(page, "1 0 0 1 100 600");
        let (mut engine, mut session) = session_with(bytes.clone());
        let (view, doc) = session.doc.as_mut().unwrap().view_with_document().unwrap();
        let content =
            region::page_content(doc, Some(view), 0, Detail::Full, Limits::DEFAULT).unwrap();
        let bounds: Vec<[f64; 4]> = (content.draws.iter())
            .filter_map(|d| match &d.kind {
                DrawKind::Text(t) => t.glyphs.first().map(|g| g.bounds),
                _ => None,
            })
            .collect();
        drop(engine.dispatch(&mut session, json!({"op": "close"}), Vec::new()));
        // A box over C and D, in this page's space.
        let r = [
            bounds[2][0].min(bounds[3][0]) + 1.0,
            bounds[2][1].min(bounds[3][1]) + 1.0,
            bounds[2][2].max(bounds[3][2]) - 1.0,
            bounds[2][3].max(bounds[3][3]) - 1.0,
        ];
        let (text, left, gone) = inspect_against_apply(bytes, r);
        assert_eq!(gone, 2, "{page}: {text:?} {left:?}");
        assert_eq!(text.replace('\n', ""), "CD", "{page}");
        assert!(
            left.contains("AB") && left.contains("EFGH"),
            "{page}: {left}"
        );
    }
}

#[test]
fn a_box_at_the_page_edge_takes_what_runs_past_it() {
    // A and B run past the left edge (x -24 to 0); the box's left side is
    // the edge, so it is not inset.
    let bytes = one_show_per_glyph("", "1 0 0 1 -24 600");
    let (text, left, gone) = inspect_against_apply(bytes, [0.0, 150.0, 1.0, 220.0]);
    assert!(gone >= 2, "{text:?} {left:?}");
    assert!(!left.contains('A') && !left.contains('B'), "{left}");
}

#[test]
fn a_glyph_without_width_goes_when_a_box_covers_its_ink() {
    // /Widths 0: every X sits at the pen (x 100); its ink runs to its
    // right. Before, its box had no width and a box over the ink missed it.
    let zero = vec!["0"; 95].join(" ");
    let font = format!(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier /FirstChar 32 /LastChar 126 /Widths [{zero}] >>"
    );
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string(),
        stream("", "BT /F1 20 Tf 100 600 Td (XXXX) Tj ET"),
    ];
    objects.push(font);
    let (mut engine, mut session) = session_with(classic(&objects));
    let bytes = apply_regions(
        &mut engine,
        &mut session,
        json!([{"page": 0, "rect": [104.0, 170.0, 112.0, 200.0]}]),
    );
    let (_, mut copied) = session_with(bytes);
    assert!(walk_glyphs(&mut copied).is_empty());
}

fn numbers_finite(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Array(items) => items.iter().all(numbers_finite),
        Value::Object(map) => map.values().all(numbers_finite),
        _ => true,
    }
}

#[test]
fn inspect_never_reports_numbers_that_are_not_finite() {
    let huge = "9".repeat(400);
    let content = format!(
        "BT /F1 20 Tf 1 0 0 1 {huge} 600 Tm (FARAWAY) Tj ET BT /F1 20 Tf {huge} 0 0 {huge} 100 600 Tm (HUGE) Tj ET {LINE}"
    );
    let (mut engine, mut session) = session_with(page_with("", "", &content, &[]));
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "redact-inspect", "regions": [{"page": 0, "rect": [0, 0, 612, 792]}]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(numbers_finite(&reply["regions"]), "{reply}");
}

#[test]
fn many_boxes_over_many_glyphs_give_a_bounded_reply() {
    // 20,000 glyphs on 100 lines, 2,000 boxes over them.
    let lines: String = (0..100)
        .map(|i| {
            format!(
                "BT /F1 4 Tf 10 {} Td ({}) Tj ET ",
                780 - i * 7,
                "W".repeat(200)
            )
        })
        .collect();
    let (mut engine, mut session) = session_with(page_with("", "", &lines, &[]));
    let boxes: Vec<Value> = (0..2000)
        .map(|i| {
            let (x, y) = (
                10.0 + f64::from(i % 40) * 12.0,
                10.0 + f64::from(i / 40) * 14.0,
            );
            json!({"page": 0, "rect": [x, y, x + 12.0, y + 14.0]})
        })
        .collect();
    let found = inspect(&mut engine, &mut session, json!(boxes));
    let items: usize = (found.iter())
        .map(|f| {
            ["glyphs", "images", "shapes"]
                .iter()
                .map(|k| f[k].as_array().unwrap().len())
                .sum::<usize>()
        })
        .sum();
    assert!(items <= 2 * MAX_ITEMS, "{items}");
    assert!(found.iter().any(|f| f["truncated"] == true));
    let size = serde_json::to_string(&found).unwrap().len();
    assert!(size < 8 << 20, "{size}");
}

// ---- R5: the raw check for area boxes -------------------------------------

/// "ACCOUNT 12345678" at user y 600 (page y 176-196), x 100 to 292; a box
/// over all of it.
const SECRET_LINE: &str = "BT /F1 20 Tf 100 600 Td (ACCOUNT 12345678) Tj ET";
const SECRET_BOX: [f64; 4] = [90.0, 170.0, 300.0, 200.0];

fn secret_page(page: &str, resources: &str, content: &str, more: &[String]) -> Vec<u8> {
    page_with(page, resources, &format!("{SECRET_LINE} {content}"), more)
}

#[test]
fn the_raw_check_finds_box_text_left_in_a_comment_or_the_properties() {
    // A note away from the box quoting the boxed text: kept when comments
    // are not removed, so the check fails; removed by sanitize, it passes.
    let note =
        "<< /Type /Annot /Subtype /Text /Contents (ACCOUNT 12345678) /Rect [400 100 420 120] >>";
    let bytes = secret_page("/Annots [6 0 R]", "", "", &[note.to_string()]);
    let (mut engine, mut session) = session_with(bytes);
    let regions = json!([{"page": 0, "rect": SECRET_BOX}]);
    let (reply, _) = apply_with(
        &mut engine,
        &mut session,
        json!({"regions": regions.clone(), "sanitize": all(false)}),
    );
    assert_eq!(reply["reason"], "verify-failed", "{reply}");
    assert_eq!(reply["detail"], "raw", "{reply}");
    assert_eq!(reply["pages"], json!([0]), "{reply}");
    assert!(!reply["error"].as_str().unwrap().contains("12345678"));
    let bytes = apply_regions(&mut engine, &mut session, regions);
    assert!(!holds(&everything(&bytes), "12345678"));
    // The same in the document information.
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string(),
        stream("", SECRET_LINE),
        courier(),
        "<< /Subject (about ACCOUNT 12345678) >>".to_string(),
    ];
    // The xref offsets are unchanged: only the trailer grows.
    let bytes = (String::from_utf8_lossy(&classic(&objects)))
        .replace("/Root 1 0 R", "/Root 1 0 R /Info 6 0 R")
        .into_bytes();
    let (mut engine, mut session) = session_with(bytes);
    let (reply, _) = apply_with(
        &mut engine,
        &mut session,
        json!({"regions": [{"page": 0, "rect": SECRET_BOX}], "sanitize": all(false)}),
    );
    assert_eq!(reply["detail"], "raw", "{reply}");
}

#[test]
fn the_raw_check_finds_box_text_in_content_the_page_does_not_show() {
    // A Form the page's resources list but never draw holds the text.
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >>",
        "BT /F1 20 Tf 100 100 Td (ACCOUNT 12345678) Tj ET",
    );
    let bytes = secret_page("", "/XObject << /Unused 6 0 R >>", "", &[form]);
    let (mut engine, mut session) = session_with(bytes);
    let (reply, _) = apply_with(
        &mut engine,
        &mut session,
        json!({"regions": [{"page": 0, "rect": SECRET_BOX}]}),
    );
    assert_eq!(reply["reason"], "verify-failed", "{reply}");
    assert_eq!(reply["detail"], "raw", "{reply}");
}

#[test]
fn the_raw_check_allows_text_the_copy_still_shows_outside_the_boxes() {
    // The same words drawn again below the box stay, and apply succeeds.
    let again = "BT /F1 20 Tf 100 300 Td (ACCOUNT 12345678) Tj ET";
    let (mut engine, mut session) = session_with(secret_page("", "", again, &[]));
    let bytes = apply_regions(
        &mut engine,
        &mut session,
        json!([{"page": 0, "rect": SECRET_BOX}]),
    );
    assert_eq!(view_text(&bytes).matches("ACCOUNT 12345678").count(), 1);
}
