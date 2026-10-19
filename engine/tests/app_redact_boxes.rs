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

// ---- R2: blanked pixels are the box's black, rounded outward ---------------

/// A 4 by 4 image (object 6, `/Im`) drawn over x 100-200, user y 500-600
/// (page y 192-292), with dictionary entries `dict` and samples `data`.
fn image_page(dict: &str, data: &[u8], more: &[String]) -> Vec<u8> {
    let mut objects = vec![binary(
        &format!("/Type /XObject /Subtype /Image /Width 4 /Height 4 {dict}"),
        data,
    )];
    objects.extend(more.iter().cloned());
    page_with(
        "",
        "/XObject << /Im 6 0 R >>",
        "q 100 0 0 100 100 500 cm /Im Do Q",
        &objects,
    )
}

/// The left half of the image (page x 100-150) and a sliver more.
const HALF_BOX: [f64; 4] = [90.0, 180.0, 150.5, 300.0];

/// The copy's image: its dictionary's colour space and depth (as written)
/// and its samples.
fn copied_image(bytes: &[u8]) -> (String, i64, Vec<u8>) {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    let page = tandem_pdf_engine::app::info::pages(&doc)[0].dict;
    let resources = doc.dict_get(page, b"Resources").as_dict().unwrap();
    let xobjects = doc.dict_get(resources, b"XObject").as_dict().unwrap();
    let id = xobjects
        .get(b"Im".as_slice())
        .unwrap()
        .as_reference()
        .unwrap();
    let dict = doc.stream_dict(id).unwrap();
    let space = match doc.resolve(dict.get(b"ColorSpace".as_slice()).unwrap()) {
        Object::Name(name) => String::from_utf8_lossy(name).to_string(),
        Object::Array(items) => format!(
            "{} {}",
            String::from_utf8_lossy(items[0].as_name().unwrap()),
            doc.resolve(&items[2]).as_i64().unwrap()
        ),
        other => format!("{other:?}"),
    };
    let bits = doc.dict_get(dict, b"BitsPerComponent").as_i64().unwrap();
    (
        space,
        bits,
        doc.decoded_stream(id, StreamLimits::default()).unwrap(),
    )
}

fn blank_with(bytes: Vec<u8>) -> Vec<u8> {
    let (mut engine, mut session) = session_with(bytes);
    apply_regions(
        &mut engine,
        &mut session,
        json!([{"page": 0, "rect": HALF_BOX}]),
    )
}

#[test]
fn cmyk_pixels_under_a_box_become_black_and_round_outward() {
    let white = [0u8; 64];
    let copy = blank_with(image_page(
        "/ColorSpace /DeviceCMYK /BitsPerComponent 8",
        &white,
        &[],
    ));
    let (_, _, data) = copied_image(&copy);
    // Columns 0-1 are under the box, column 2 by half a point: rounded out.
    for y in 0..4 {
        for x in 0..4 {
            let px = &data[(y * 4 + x) * 4..(y * 4 + x) * 4 + 4];
            let expected: &[u8] = if x <= 2 {
                &[0, 0, 0, 255]
            } else {
                &[0, 0, 0, 0]
            };
            assert_eq!(px, expected, "pixel {x},{y}");
        }
    }
    // With /Decode inverted, black is written inverted.
    let copy = blank_with(image_page(
        "/ColorSpace /DeviceCMYK /BitsPerComponent 8 /Decode [1 0 1 0 1 0 1 0]",
        &[255u8; 64],
        &[],
    ));
    let (_, _, data) = copied_image(&copy);
    assert_eq!(&data[..4], &[255, 255, 255, 0]);
    assert_eq!(&data[12..16], &[255, 255, 255, 255]);
}

#[test]
fn indexed_pixels_take_a_dark_entry_or_gain_one() {
    // A palette with a dark entry: it is used.
    let copy = blank_with(image_page(
        "/ColorSpace [/Indexed /DeviceRGB 2 <FFFFFF FF0000 101010>] /BitsPerComponent 8",
        &[0u8; 16],
        &[],
    ));
    let (_, _, data) = copied_image(&copy);
    assert_eq!((data[0], data[3]), (2, 0));
    // No dark entry, room in the palette: black is added as entry 2.
    let copy = blank_with(image_page(
        "/ColorSpace [/Indexed /DeviceRGB 1 <FFFFFF FF0000>] /BitsPerComponent 8",
        &[1u8; 16],
        &[],
    ));
    let (space, _, data) = copied_image(&copy);
    assert_eq!(space, "Indexed 2");
    assert_eq!((data[0], data[3]), (2, 1));
    // A full one-bit palette: widened to 8 bits, black added.
    let copy = blank_with(image_page(
        "/ColorSpace [/Indexed /DeviceRGB 1 <FFFFFF FF0000>] /BitsPerComponent 1",
        &[0x50; 4],
        &[],
    ));
    let (space, bits, data) = copied_image(&copy);
    assert_eq!(bits, 8, "{space}");
    // Row 0 was 0101: columns 0-2 black (2), column 3 kept (1).
    assert_eq!(&data[..4], &[2, 2, 2, 1]);
    // A full 8-bit palette of light colours: rewritten as RGB.
    let entries: String = (0..256).map(|i| format!("FF{:02X}FF", i)).collect();
    let copy = blank_with(image_page(
        &format!("/ColorSpace [/Indexed /DeviceRGB 255 <{entries}>] /BitsPerComponent 8"),
        &[7u8; 16],
        &[],
    ));
    let (space, _, data) = copied_image(&copy);
    assert!(space == "DeviceRGB", "{space}");
    assert_eq!(&data[..3], &[0, 0, 0]);
    assert_eq!(&data[9..12], &[255, 7, 255]);
}

// ---- R3: shared pictures and Forms are copied, not refused -----------------

/// Two pages drawing image 6 (`/Im`) and Form 7 (`/Fm`, text SHAREDFORM)
/// alike: the image over x 100-200, user y 500-600 (page y 192-292), the
/// Form's text at user y 400 (page y 392).
fn shared_pages() -> Vec<u8> {
    let page = |contents: u32| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> /XObject << /Im 6 0 R /Fm 7 0 R >> >> /Contents {contents} 0 R >>"
        )
    };
    let drawn = "q 100 0 0 100 100 500 cm /Im Do Q /Fm Do";
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        page(8),
        page(9),
        courier(),
        binary(
            "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8",
            &[200u8; 16],
        ),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >>",
            "BT /F1 20 Tf 100 400 Td (SHAREDFORM) Tj ET",
        ),
        stream("", drawn),
        stream("", drawn),
    ])
}

fn render(bytes: &[u8], page: usize) -> Vec<u8> {
    let view = ViewDocument::open(bytes.to_vec(), "").unwrap();
    let options = tandem_pdf_engine::view::RenderOptions::default();
    view.render(page, 1.0, options).unwrap().pixels
}

#[test]
fn a_picture_and_a_form_shared_across_pages_are_redacted_everywhere_and_flagged() {
    let source = shared_pages();
    let (mut engine, mut session) = session_with(source.clone());
    mark(&mut engine, &mut session, 0, [90.0, 180.0, 150.0, 300.0]);
    mark(&mut engine, &mut session, 0, [90.0, 370.0, 400.0, 400.0]);
    let preview = ok(&mut engine, &mut session, json!({"op": "redact-preview"}));
    assert_ne!(preview["shared"], json!([]), "{preview}");
    let (reply, copy) = apply_with(&mut engine, &mut session, json!({}));
    // Fourth review: one object, redacted in place, everywhere; the reply
    // says page 2 lost what page 1's boxes cover.
    assert_eq!(reply["overRedacted"], true, "{reply}");
    assert_ne!(render(&source, 1), render(&copy, 1));
    let text = view_text(&copy);
    assert_eq!(text.matches("SHAREDFORM").count(), 0, "{text}");
    // Page 1 draws a copy of the image, half blanked; page 2 the original.
    let doc = Document::open(&copy, DocumentLimits::default()).unwrap();
    let pages = tandem_pdf_engine::app::info::pages(&doc);
    let image_of = |page: usize| {
        let resources = doc
            .dict_get(pages[page].dict, b"Resources")
            .as_dict()
            .unwrap();
        let xobjects = doc.dict_get(resources, b"XObject").as_dict().unwrap();
        let ids: Vec<_> = (xobjects.values())
            .filter_map(|v| v.as_reference())
            .filter(|id| {
                doc.dict_get(doc.stream_dict(*id).unwrap(), b"Subtype")
                    .as_name()
                    == Some(b"Image")
            })
            .collect();
        (ids.iter())
            .map(|id| doc.decoded_stream(*id, StreamLimits::default()).unwrap())
            .collect::<Vec<_>>()
    };
    // The one image, blanked where page 1's box lies, on both pages.
    assert!(!image_of(1).contains(&vec![200u8; 16]));
    let first = image_of(0);
    assert_eq!(first.len(), 1);
    assert_eq!((first[0][0], first[0][3]), (0, 200), "{first:?}");
}

#[test]
fn inspect_of_a_search_mark_lists_what_apply_removes_with_its_tight_rule() {
    let bytes = one_show_per_glyph("", "1 0 0 1 100 600");
    let (mut engine, mut session) = session_with(bytes);
    ok(
        &mut engine,
        &mut session,
        json!({"op": "redact-search", "text": "CD"}),
    );
    let listed = ok(
        &mut engine,
        &mut session,
        json!({"op": "annots", "page": 0}),
    );
    let item = &listed["items"][0];
    let annot = item["id"].clone();
    // By the mark itself, and as a find hit (`search`) over its rect.
    let by_mark = inspect(
        &mut engine,
        &mut session,
        json!([{"page": 0, "annotId": annot}]),
    );
    let rect = item["rect"].clone();
    let by_rect = inspect(
        &mut engine,
        &mut session,
        json!([{"page": 0, "rect": rect.clone(), "search": true}, {"page": 0, "rect": rect}]),
    );
    let source = walk_glyphs(&mut session);
    let copy = apply(&mut engine, &mut session);
    let (_, mut copied) = session_with(copy);
    let kept = walk_glyphs(&mut copied);
    let mut gone: Vec<[i64; 8]> = source.into_iter().map(|(_, q)| q).collect();
    for (_, q) in &kept {
        let at = gone.iter().position(|g| g == q).unwrap();
        gone.remove(at);
    }
    gone.sort_unstable();
    let quads = |found: &Value| {
        let mut out: Vec<[i64; 8]> = (found["glyphs"].as_array().unwrap().iter())
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
        out.sort_unstable();
        out
    };
    assert_eq!(gone.len(), 2);
    assert_eq!(quads(&by_mark[0]), gone, "{}", by_mark[0]);
    assert_eq!(quads(&by_rect[0]), gone, "{}", by_rect[0]);
    assert_eq!(by_mark[0]["text"], "CD");
    // As a plain box the same rect takes B and E too (the two-point margin).
    assert_eq!(by_rect[1]["text"], "BCDE", "{}", by_rect[1]);
    // A mark that is not a redaction mark is refused.
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "redact-inspect", "regions": [{"page": 0, "annotId": 99999}]}),
    );
    assert_eq!(reply["detail"], "regions", "{reply}");
}

// ---- R4: content that cannot be cut exactly is flattened, with consent ----

/// Page 1: a mesh shading (Gouraud triangles) over its top half with
/// MESHSECRET drawn on it, and KEEPTEXT lower down; page 2: PAGETWO.
fn mesh_pages() -> Vec<u8> {
    // Two triangles covering x 0-612, page y 0-396 (user y 396-792):
    // flag, x, y, gray, each one byte, through /Decode.
    let tri: Vec<u8> = vec![
        0, 0, 128, 0, 0, 255, 128, 128, 0, 0, 255, 255, //
        0, 255, 128, 64, 0, 255, 255, 200, 0, 0, 255, 32,
    ];
    let mesh = binary(
        "/ShadingType 4 /ColorSpace /DeviceGray /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 612 0 792 0 1]",
        &tri,
    );
    let page = |contents: u32| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> /Shading << /Sh 6 0 R >> >> /Contents {contents} 0 R >>"
        )
    };
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        page(7),
        page(8),
        courier(),
        mesh,
        stream(
            "",
            "q 0 396 612 396 re W n /Sh sh Q BT /F1 20 Tf 100 600 Td (MESHSECRET) Tj ET BT /F1 20 Tf 100 200 Td (KEEPTEXT) Tj ET",
        ),
        stream("", "BT /F1 20 Tf 100 600 Td (PAGETWO) Tj ET"),
    ])
}

#[test]
fn a_mesh_shading_under_a_box_asks_to_flatten_and_with_consent_the_page_becomes_a_picture() {
    let (mut engine, mut session) = session_with(mesh_pages());
    let over_mesh = [90.0, 170.0, 330.0, 200.0];
    let found = inspect(
        &mut engine,
        &mut session,
        json!([{"page": 0, "rect": over_mesh}, {"page": 0, "rect": [90.0, 570.0, 330.0, 600.0]}]),
    );
    assert_eq!(found[0]["needsFlatten"], true, "{}", found[0]);
    assert_eq!(found[1]["needsFlatten"], false, "{}", found[1]);
    let regions = json!([{"page": 0, "rect": over_mesh}]);
    let (reply, _) = apply_with(
        &mut engine,
        &mut session,
        json!({ "regions": regions.clone() }),
    );
    assert_eq!(reply["reason"], "needs-flatten", "{reply}");
    assert_eq!(reply["pages"], json!([0]), "{reply}");
    assert!(!reply["error"].as_str().unwrap().contains("MESH"));
    let (reply, copy) = apply_with(
        &mut engine,
        &mut session,
        json!({ "regions": regions, "flatten": [0] }),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["flattened"], json!([0]));
    // The secret is gone from every object; page 1 is one picture of about
    // 200 dpi; page 2 keeps its text.
    assert!(!holds(&everything(&copy), "MESHSECRET"));
    let text = view_text(&copy);
    assert!(
        text.contains("PAGETWO") && !text.contains("KEEPTEXT"),
        "{text}"
    );
    let doc = Document::open(&copy, DocumentLimits::default()).unwrap();
    let pages = tandem_pdf_engine::app::info::pages(&doc);
    let resources = doc.dict_get(pages[0].dict, b"Resources").as_dict().unwrap();
    let xobjects = doc.dict_get(resources, b"XObject").as_dict().unwrap();
    let picture = xobjects
        .get(b"RdPage".as_slice())
        .unwrap()
        .as_reference()
        .unwrap();
    let dict = doc.stream_dict(picture).unwrap();
    assert_eq!(doc.dict_get(dict, b"Width").as_i64(), Some(1700));
    assert_eq!(doc.dict_get(dict, b"Height").as_i64(), Some(2200));
    // A flatten page that is not marked is left alone.
    let (reply, _) = apply_with(
        &mut engine,
        &mut session,
        json!({"regions": [{"page": 1, "rect": [90.0, 170.0, 330.0, 200.0]}], "flatten": [0]}),
    );
    assert_eq!(reply["flattened"], json!([]), "{reply}");
    let (reply, _) = apply_with(
        &mut engine,
        &mut session,
        json!({"regions": [{"page": 1, "rect": [90.0, 170.0, 330.0, 200.0]}], "flatten": [7]}),
    );
    assert_eq!(reply["detail"], "flatten", "{reply}");
}

// ---- D5: signed files are redacted into an unsigned copy, with consent ----

#[test]
fn a_signed_file_is_redacted_only_into_an_unsigned_copy_with_consent() {
    for name in ["p0b/signed-intact.pdf", "p0b/certified-p2.pdf"] {
        let source = fixture(name);
        let before = Document::open(&source, DocumentLimits::default()).unwrap();
        assert!(tandem_pdf_engine::app::info::detect(&before, false, false).signed());
        let (mut engine, mut session) = session_with(source);
        let regions = json!([{"page": 0, "rect": [0.0, 0.0, 300.0, 300.0]}]);
        let (reply, _) = apply_with(
            &mut engine,
            &mut session,
            json!({ "regions": regions.clone() }),
        );
        assert_eq!(reply["reason"], "signed", "{name}: {reply}");
        let (reply, copy) = apply_with(
            &mut engine,
            &mut session,
            json!({ "regions": regions, "unsign": true }),
        );
        assert_eq!(reply["ok"], true, "{name}: {reply}");
        assert_eq!(reply["unsigned"], true, "{name}: {reply}");
        let after = Document::open(&copy, DocumentLimits::default()).unwrap();
        let info = tandem_pdf_engine::app::info::detect(&after, false, false);
        assert!(!info.signed(), "{name}: {}", info.json());
        // No signature dictionary is left in the copy.
        assert!(holds(&everything(&fixture(name)), "adbe.pkcs7"), "{name}");
        assert!(!holds(&everything(&copy), "adbe.pkcs7"), "{name}");
        // The session's document is still the signed original.
        let (_, mut again) = session_with(fixture(name));
        let (reply, _) = apply_with(
            &mut engine,
            &mut again,
            json!({"regions": [{"page": 0, "rect": [0, 0, 10, 10]}]}),
        );
        assert_eq!(reply["reason"], "signed");
    }
}

// ---- acceptance: fixtures/redact.pdf (plan §2.5) --------------------------

const TOKEN: &str = "P0B-SECRET-7731";

/// The objects of `bytes` whose strings, names or decoded stream hold
/// `needle`.
fn holders(bytes: &[u8], needle: &str) -> Vec<tandem_pdf_engine::object::ObjectId> {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    let mut out = Vec::new();
    for object in doc.objects() {
        let mut found = false;
        let mut stack = vec![&object.value];
        while let Some(value) = stack.pop() {
            match value {
                Object::String(s) | Object::Name(s) => {
                    found |= s.windows(needle.len()).any(|w| w == needle.as_bytes());
                }
                Object::Array(items) => stack.extend(items),
                Object::Dictionary(dict) => stack.extend(dict.values()),
                _ => {}
            }
        }
        if object.stream.is_some() {
            if let Ok(decoded) = doc.decoded_stream(object.id, StreamLimits::default()) {
                found |= decoded
                    .windows(needle.len())
                    .any(|w| w == needle.as_bytes());
            }
        }
        if found {
            out.push(object.id);
        }
    }
    out
}

#[test]
fn one_box_over_page_one_of_the_redact_fixture_removes_every_occurrence_on_it() {
    // Page 1 shows the secret as text (visible, invisible, white, under a
    // picture, in a layer, tagged), as a bitmap picture, in a Form, in a
    // FreeText comment and in a form field; page 2 draws the same Form.
    let (mut engine, mut session) = session_with(fixture("redact.pdf"));
    let page_one = json!([{"page": 0, "rect": [0, 0, 612, 792]}]);
    let (reply, copy) = apply_with(
        &mut engine,
        &mut session,
        json!({ "regions": page_one.clone() }),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["verified"]["text"], 1);
    let view = ViewDocument::open(copy.clone(), "").unwrap();
    let one = view.structured_text(0).unwrap().as_text();
    assert!(!one.contains(TOKEN) && !one.contains("SECRET"), "{one}");
    // The only object left holding it is the Form page 2 draws (its own
    // placement, outside the box).
    let doc = Document::open(&copy, DocumentLimits::default()).unwrap();
    let pages = tandem_pdf_engine::app::info::pages(&doc);
    let two = doc.dict_get(pages[1].dict, b"Resources").as_dict().unwrap();
    let form = doc
        .dict_get(two, b"XObject")
        .as_dict()
        .unwrap()
        .get(b"Fm1".as_slice())
        .unwrap()
        .as_reference()
        .unwrap();
    // Fourth review: Fm1 is redacted in place, so page 2 (which draws it
    // outside any box) loses the secret too, and the reply says so.
    let _ = (form, two);
    assert!(holders(&copy, TOKEN).is_empty());
    assert_eq!(reply["overRedacted"], true, "{reply}");
    // Page 1 keeps no annotation the box touched (the comment, the field)
    // and draws no picture or Form of the original.
    assert!(
        doc.dict_get(pages[0].dict, b"Annots")
            .as_array()
            .is_none_or(|a| a.is_empty())
    );
    // Kept metadata quoting the secret fails the raw check.
    let (reply, _) = apply_with(
        &mut engine,
        &mut session,
        json!({"regions": page_one, "sanitize": all(false)}),
    );
    assert_eq!(reply["reason"], "verify-failed", "{reply}");
    assert_eq!(reply["detail"], "raw", "{reply}");
    // A box over each page: the secret is in no object of the copy.
    let (reply, copy) = apply_with(
        &mut engine,
        &mut session,
        json!({"regions": [{"page": 0, "rect": [0, 0, 612, 792]}, {"page": 1, "rect": [0, 0, 612, 792]}]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(holders(&copy, TOKEN).is_empty());
    assert!(!holds(&everything(&copy), "SECRET"));
}

// ---- review probes (be611db..47dd3f9): originals reachable another way ----

const SECRET: &str = "TOPSECRETXYZ";

fn sec_form(y: u32) -> String {
    stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >>",
        &format!("BT /F1 20 Tf 100 {y} Td ({SECRET}) Tj ET"),
    )
}

fn gray_image() -> String {
    binary(
        "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8",
        &[200u8; 16],
    )
}

/// The copy holds the secret nowhere and does not show it.
fn secret_gone(reply: &Value, copy: &[u8], what: &str) {
    assert_eq!(reply["ok"], true, "{what}: {reply}");
    assert!(!holds(&everything(copy), SECRET), "{what}: in the file");
    assert!(!view_text(copy).contains(SECRET), "{what}: in the text");
}

#[test]
fn r3_an_original_reachable_through_shared_or_inherited_resources_is_redacted() {
    // Two pages share one resources dictionary (an object, or inherited
    // from the page tree) naming an image both draw and a Form (the secret)
    // only page 1 draws; boxes over page 1's image and the Form's text.
    let shared = |pages_node: &str, page_resources: &str, more: Option<String>| {
        let page = |contents: u32| {
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] {page_resources} /Contents {contents} 0 R >>"
            )
        };
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            format!("<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 {pages_node} >>"),
            page(8),
            page(9),
            courier(),
            gray_image(),
            sec_form(400),
            stream("", "q 100 0 0 100 100 500 cm /Im Do Q /Sec Do"),
            stream("", "q 100 0 0 100 100 500 cm /Im Do Q"),
        ];
        objects.extend(more);
        classic(&objects)
    };
    let names = "<< /Font << /F1 5 0 R >> /XObject << /Im 6 0 R /Sec 7 0 R >> >>";
    for (what, bytes) in [
        (
            "an object",
            shared("", "/Resources 10 0 R", Some(names.to_string())),
        ),
        (
            "inherited",
            shared(&format!("/Resources {names}"), "", None),
        ),
    ] {
        let source = bytes.clone();
        let (mut engine, mut session) = session_with(bytes);
        let regions = json!([
            {"page": 0, "rect": [90.0, 180.0, 150.0, 300.0]},
            {"page": 0, "rect": [90.0, 370.0, 400.0, 400.0]}
        ]);
        let (reply, copy) = apply_with(&mut engine, &mut session, json!({ "regions": regions }));
        secret_gone(&reply, &copy, what);
        // Page 2 still draws the image unblanked (its own placement).
        // Fourth review: the picture is blanked in place, page 2 included,
        // and the reply says so.
        assert_eq!(reply["overRedacted"], true, "{what}: {reply}");
        let _ = &source;
    }
}

fn mesh() -> String {
    let tri: Vec<u8> = vec![
        0, 0, 128, 0, 0, 255, 128, 128, 0, 0, 255, 255, //
        0, 255, 128, 64, 0, 255, 255, 200, 0, 0, 255, 32,
    ];
    binary(
        "/ShadingType 4 /ColorSpace /DeviceGray /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 612 0 792 0 1]",
        &tri,
    )
}

#[test]
fn r4_a_flattened_page_leaves_nothing_in_resources_it_shares() {
    let page = |contents: u32| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources 10 0 R /Contents {contents} 0 R >>"
        )
    };
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        page(8),
        page(9),
        courier(),
        mesh(),
        sec_form(600),
        stream("", "q 0 396 612 396 re W n /Sh sh Q /Sec Do"),
        stream("", "BT /F1 20 Tf 100 200 Td (PAGETWO) Tj ET"),
        "<< /Font << /F1 5 0 R >> /Shading << /Sh 6 0 R >> /XObject << /Sec 7 0 R >> >>"
            .to_string(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let regions = json!([{"page": 0, "rect": [90.0, 170.0, 330.0, 200.0]}]);
    let (reply, _) = apply_with(
        &mut engine,
        &mut session,
        json!({ "regions": regions.clone() }),
    );
    assert_eq!(reply["reason"], "needs-flatten", "{reply}");
    let (reply, copy) = apply_with(
        &mut engine,
        &mut session,
        json!({ "regions": regions, "flatten": [0] }),
    );
    secret_gone(&reply, &copy, "flattened");
    assert!(view_text(&copy).contains("PAGETWO"));
}

#[test]
fn r4_flattening_turned_cropped_and_scaled_pages_burns_the_box_over_the_secret() {
    for extra in [
        "/Rotate 90",
        "/Rotate 270 /CropBox [50 100 600 780]",
        "/UserUnit 2",
    ] {
        let page = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] {extra} /Resources << /Font << /F1 5 0 R >> /Shading << /Sh 6 0 R >> >> /Contents 7 0 R >>"
        );
        let bytes = classic(&[
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            page,
            "null".to_string(),
            courier(),
            mesh(),
            stream(
                "",
                &format!("/Sh sh BT /F1 20 Tf 100 600 Td ({SECRET}) Tj ET"),
            ),
        ]);
        let (mut engine, mut session) = session_with(bytes);
        ok(
            &mut engine,
            &mut session,
            json!({"op": "redact-search", "text": SECRET}),
        );
        let (reply, copy) = apply_with(&mut engine, &mut session, json!({ "flatten": [0] }));
        secret_gone(&reply, &copy, extra);
    }
}

#[test]
fn d5_a_signature_field_written_directly_into_the_form_goes() {
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /SigFlags 3 /Fields [<< /FT /Sig /T (s) /V 8 0 R >> << /T (parent) /Kids [<< /FT /Sig /T (k) /V 8 0 R >>] >>] >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string(),
        stream("", &format!("BT /F1 20 Tf 100 600 Td ({SECRET}) Tj ET")),
        courier(),
        "null".to_string(),
        "null".to_string(),
        "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 0 0 0] /Contents <00> >>".to_string(),
    ]);
    let before = Document::open(&bytes, DocumentLimits::default()).unwrap();
    assert!(tandem_pdf_engine::app::info::detect(&before, false, false).signed());
    let (mut engine, mut session) = session_with(bytes);
    let regions = json!([{"page": 0, "rect": [90.0, 170.0, 330.0, 200.0]}]);
    let (reply, copy) = apply_with(
        &mut engine,
        &mut session,
        json!({ "regions": regions, "unsign": true }),
    );
    secret_gone(&reply, &copy, "direct signature");
    let after = Document::open(&copy, DocumentLimits::default()).unwrap();
    assert!(!tandem_pdf_engine::app::info::detect(&after, false, false).signed());
    assert!(!holds(&everything(&copy), "adbe.pkcs7"));
}

#[test]
fn the_raw_backstop_reads_every_content_stream_of_the_copy() {
    // A hidden stamp on page 2 whose appearance (no /Contents) draws the
    // boxed text: nothing shows it, so the copy fails closed when comments
    // are kept.
    let stamp =
        "<< /Type /Annot /Subtype /Stamp /F 2 /Rect [100 100 300 130] /AP << /N 7 0 R >> >>";
    let appearance = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 200 30] /Resources << /Font << /F1 5 0 R >> >>",
        "BT /F1 12 Tf 2 8 Td (ACCOUNT 12345678) Tj ET",
    );
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 8 0 R >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [6 0 R] /Contents 9 0 R >>".to_string(),
        courier(),
        stamp.to_string(),
        appearance,
        stream("", SECRET_LINE),
        stream("", ""),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let regions = json!([{"page": 0, "rect": SECRET_BOX}]);
    let (reply, _) = apply_with(
        &mut engine,
        &mut session,
        json!({"regions": regions.clone(), "sanitize": all(false)}),
    );
    assert_eq!(reply["reason"], "verify-failed", "{reply}");
    assert_eq!(reply["detail"], "raw", "{reply}");
    // Comments removed by sanitize: the copy passes.
    let copy = apply_regions(&mut engine, &mut session, regions);
    assert!(!holds(&everything(&copy), "12345678"));
}

#[test]
fn flattening_too_many_large_pages_is_refused_before_drawing_them() {
    // Five pages of 5,000 by 5,000 points: each drawn at the largest size
    // (16.7 million pixels), more than four such pages in all.
    let kids: String = (0..5).map(|i| format!("{} 0 R ", 3 + i)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{kids}] /Count 5 >>"),
    ];
    for _ in 0..5 {
        objects.push(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 5000 5000] /Contents 8 0 R >>".to_string(),
        );
    }
    objects.push(stream("", "0 0 1 rg 10 10 100 100 re f"));
    let (mut engine, mut session) = session_with(classic(&objects));
    let regions: Vec<Value> = (0..5)
        .map(|p| json!({"page": p, "rect": [0, 0, 50, 50]}))
        .collect();
    let started = std::time::Instant::now();
    let (reply, _) = apply_with(
        &mut engine,
        &mut session,
        json!({"regions": regions, "flatten": [0, 1, 2, 3, 4]}),
    );
    assert_eq!(reply["reason"], "too-large", "{reply}");
    // Refused before any page is drawn.
    assert!(started.elapsed().as_secs() < 30);
}

#[test]
fn indexed_spot_and_lab_images_gain_black_in_their_own_space() {
    // Separation base, no dark entry (tint 0 and 0x40): full tint is added.
    let separation = "[/Indexed [/Separation /Spot /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 1 0 0] /N 1 >>] 1 <0040>]";
    let copy = blank_with(image_page(
        &format!("/ColorSpace {separation} /BitsPerComponent 8"),
        &[0u8; 16],
        &[],
    ));
    let (space, _, data) = copied_image(&copy);
    assert_eq!(space, "Indexed 2");
    assert_eq!((data[0], data[3]), (2, 0));
    // A full palette over a spot colour stays Indexed (no RGB rewrite).
    let entries: String = (0..256).map(|_| "10".to_string()).collect();
    let copy = blank_with(image_page(
        &format!(
            "/ColorSpace [/Indexed [/Separation /Spot /DeviceGray << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >>] 255 <{entries}>] /BitsPerComponent 8"
        ),
        &[7u8; 16],
        &[],
    ));
    let (space, _, _) = copied_image(&copy);
    assert_eq!(space, "Indexed 255");
    // A 16-bit gray image inverted by /Decode: black is 65535.
    let copy = blank_with(image_page(
        "/ColorSpace /DeviceGray /BitsPerComponent 16 /Decode [1 0]",
        &[0u8; 32],
        &[],
    ));
    let (_, bits, data) = copied_image(&copy);
    assert_eq!(bits, 16);
    assert_eq!((data[0], data[1], data[6], data[7]), (0xff, 0xff, 0, 0));
}

#[test]
fn r4_a_picture_in_a_pattern_only_the_flattened_page_paints_is_emptied() {
    // Page 2 paints with the pattern too: a pattern is never copied, so it
    // is emptied all the same and the reply says page 2 is over-redacted
    // (third review: uncertainty fails closed).
    for (page_two, over) in [
        ("BT /F1 20 Tf 100 200 Td (PAGETWO) Tj ET", true),
        ("/Pattern cs /P1 scn 300 300 100 100 re f", true),
    ] {
        flattened_pattern(page_two, over);
    }
}

fn flattened_pattern(page_two: &str, over: bool) {
    // Page 1 fills a rect with tiling pattern 11, whose cell draws image 12
    // (every sample 0x5A), and has a mesh shading under the box; both pages
    // share resources object 10, which names the pattern; page 2 paints
    // nothing with it.
    let page = |contents: u32| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources 10 0 R /Contents {contents} 0 R >>"
        )
    };
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        page(8),
        page(9),
        courier(),
        mesh(),
        "null".to_string(),
        stream(
            "",
            "q 0 396 612 396 re W n /Sh sh Q /Pattern cs /P1 scn 100 100 200 200 re f",
        ),
        stream("", page_two),
        "<< /Font << /F1 5 0 R >> /Shading << /Sh 6 0 R >> /Pattern << /P1 11 0 R >> >>"
            .to_string(),
        stream(
            "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 50 50] /XStep 50 /YStep 50 /Resources << /XObject << /Pic 12 0 R >> >>",
            "q 50 0 0 50 0 0 cm /Pic Do Q",
        ),
        binary(
            "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8",
            &[0x5A; 16],
        ),
    ]);
    let source = bytes.clone();
    let (mut engine, mut session) = session_with(bytes);
    let regions = json!([{"page": 0, "rect": [90.0, 170.0, 330.0, 200.0]}]);
    let (reply, copy) = apply_with(
        &mut engine,
        &mut session,
        json!({ "regions": regions, "flatten": [0] }),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    // No 4 by 4 picture is left (the flattened page's own picture shows the
    // pattern outside the box, as drawn).
    let pictures = |bytes: &[u8]| {
        let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
        (doc.objects())
            .filter(|o| o.stream.is_some())
            .filter(|o| {
                let d = o.value.as_dict().unwrap();
                doc.dict_get(d, b"Subtype").as_name() == Some(b"Image")
                    && doc.dict_get(d, b"Width").as_i64() == Some(4)
            })
            .count()
    };
    assert_eq!(pictures(&source), 1);
    assert_eq!(pictures(&copy), 0, "{page_two}");
    // The shared resources name the pattern on page 2 either way.
    assert_eq!(reply["overRedacted"], over, "{reply}");
    if !page_two.contains("Pattern") {
        assert_eq!(render(&source, 1), render(&copy, 1));
    }
}

// ---- third review probes (47dd3f9..0908aed): uncertainty fails closed ----

/// Never under-redacted: either apply refuses, or the copy holds `needle`
/// nowhere (a refusal is reported, for the tests that require success).
fn report_needle(name: &str, reply: &Value, copy: &[u8], needle: &str) {
    if reply["ok"] != true {
        eprintln!("PROBE {name}: refused {}", reply["reason"]);
        return;
    }
    assert!(
        !holds(&everything(copy), needle),
        "{name}: {needle} left in the file"
    );
}

fn e3_secret_image() -> String {
    binary(
        "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8",
        &[0x5A; 16],
    )
}

const PIC: &str = "ZZZZZZZZZZZZZZZZ";

/// A two-page doc whose pages share resources object 10; page 1 draws the
/// secret Form 7; page 2's contents (object 9) are `page2`.
fn shared_two(page2: String, resources: String, extra: &[String]) -> Vec<u8> {
    let page = |contents: u32| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources 10 0 R /Contents {contents} 0 R >>"
        )
    };
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        page(8),
        page(9),
        courier(),
        e3_secret_image(),
        sec_form(400),
        stream("", "q 100 0 0 100 100 500 cm /Pic Do Q /Sec Do"),
        page2,
        resources,
    ];
    objects.extend(extra.iter().cloned());
    classic(&objects)
}

fn boxes() -> Value {
    // The secret text (baseline y=400) and the picture (100..200, 500..600).
    json!([
        {"page": 0, "rect": [90.0, 370.0, 400.0, 400.0]},
        {"page": 0, "rect": [90.0, 180.0, 210.0, 300.0]}
    ])
}

/// R3: page 2 shares the resources but its content cannot be decoded:
/// `shared` counts every touched object it can reach as drawn there, so
/// the originals are kept (not redacted in place) and the backstop skips them.
#[test]
fn e3_unreadable_page_keeps_original() {
    let bytes = shared_two(
        "<< /Filter /FlateDecode /Length 6 >>\nstream\nxyzxyz\nendstream".to_string(),
        "<< /Font << /F1 5 0 R >> /XObject << /Pic 6 0 R /Sec 7 0 R >> >>".to_string(),
        &[],
    );
    let (mut engine, mut session) = session_with(bytes);
    let (reply, copy) = apply_with(&mut engine, &mut session, json!({ "regions": boxes() }));
    report_needle(
        "e3_unreadable_page_keeps_original/text",
        &reply,
        &copy,
        SECRET,
    );
    report_needle("e3_unreadable_page_keeps_original/pic", &reply, &copy, PIC);
}

/// The same, flattened.
#[test]
fn e3_unreadable_page_flatten() {
    let bytes = shared_two(
        "<< /Filter /FlateDecode /Length 6 >>\nstream\nxyzxyz\nendstream".to_string(),
        "<< /Font << /F1 5 0 R >> /XObject << /Pic 6 0 R /Sec 7 0 R >> >>".to_string(),
        &[],
    );
    let (mut engine, mut session) = session_with(bytes);
    let (reply, copy) = apply_with(
        &mut engine,
        &mut session,
        json!({ "regions": boxes(), "flatten": [0] }),
    );
    report_needle("e3_unreadable_page_flatten/text", &reply, &copy, SECRET);
    report_needle("e3_unreadable_page_flatten/pic", &reply, &copy, PIC);
}

/// R3, a valid one-page file: the page's resources also hold an unused
/// ExtGState whose soft mask group is the secret Form, and an unused image
/// whose /SMask is the secret picture. `hidden_draws` counts both as drawn
/// on the page outside the walk.
#[test]
fn e3_unused_softmask_keeps_original() {
    for flatten in [false, true] {
        let bytes = classic(&[
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources 10 0 R /Contents 8 0 R >>".to_string(),
            "null".to_string(),
            courier(),
            e3_secret_image(),
            sec_form(400),
            stream("", "q 100 0 0 100 100 500 cm /Pic Do Q /Sec Do"),
            "null".to_string(),
            "<< /Font << /F1 5 0 R >> /XObject << /Pic 6 0 R /Sec 7 0 R /Orphan 11 0 R >> /ExtGState << /GS << /SMask << /S /Luminosity /G 7 0 R >> >> >> >>".to_string(),
            binary(
                "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /SMask 6 0 R",
                &[0x11],
            ),
        ]);
        let (mut engine, mut session) = session_with(bytes);
        let extra = match flatten {
            true => json!({ "regions": boxes(), "flatten": [0] }),
            false => json!({ "regions": boxes() }),
        };
        let (reply, copy) = apply_with(&mut engine, &mut session, extra);
        let name = format!("e3_unused_softmask flatten={flatten}");
        report_needle(&format!("{name}/text"), &reply, &copy, SECRET);
        report_needle(&format!("{name}/pic"), &reply, &copy, PIC);
    }
}

/// R3, valid: an unused tiling pattern in the page's resources whose cell
/// draws the secret Form; and (b) an unused Type 3 font whose glyph draws it.
#[test]
fn e3_unused_pattern_and_type3_keep_original() {
    for (name, res, extra) in [
        (
            "pattern",
            "<< /Font << /F1 5 0 R >> /XObject << /Pic 6 0 R /Sec 7 0 R >> /Pattern << /P 11 0 R >> >>",
            stream(
                "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 50 50] /XStep 50 /YStep 50 /Resources << /XObject << /Sec 7 0 R /Pic 6 0 R >> >>",
                "/Sec Do /Pic Do",
            ),
        ),
        (
            "type3",
            "<< /Font << /F1 5 0 R /T3 11 0 R >> /XObject << /Pic 6 0 R /Sec 7 0 R >> >>",
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1 1] /FontMatrix [0.001 0 0 0.001 0 0] /FirstChar 65 /LastChar 65 /Widths [1000] /Encoding << /Differences [65 /a] >> /CharProcs << /a 12 0 R >> /Resources << /XObject << /Sec 7 0 R /Pic 6 0 R >> >> >>".to_string(),
        ),
    ] {
        let bytes = classic(&[
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources 10 0 R /Contents 8 0 R >>".to_string(),
            "null".to_string(),
            courier(),
            e3_secret_image(),
            sec_form(400),
            stream("", "q 100 0 0 100 100 500 cm /Pic Do Q /Sec Do"),
            "null".to_string(),
            res.to_string(),
            extra,
            stream("", "1000 0 d0 /Sec Do /Pic Do"),
        ]);
        for flatten in [false, true] {
            let (mut engine, mut session) = session_with(bytes.clone());
            let extra = match flatten {
                true => json!({ "regions": boxes(), "flatten": [0] }),
                false => json!({ "regions": boxes() }),
            };
            let (reply, copy) = apply_with(&mut engine, &mut session, extra);
            let n = format!("e3_unused_{name} flatten={flatten}");
            report_needle(&format!("{n}/text"), &reply, &copy, SECRET);
            report_needle(&format!("{n}/pic"), &reply, &copy, PIC);
        }
    }
}

/// Design question: page 2 draws the secret Form at zero size / clipped
/// away / off the page; `shared` counts it as shown there.
#[test]
fn e3_invisible_second_placement() {
    for (name, content) in [
        ("zero", "q 0 0 0 0 0 0 cm /Sec Do Q"),
        ("clip", "q 0 0 0 0 re W n /Sec Do Q"),
        ("offpage", "q 1 0 0 1 5000 5000 cm /Sec Do Q"),
        ("tr7", "q 3 Tr /Sec Do Q"),
    ] {
        let bytes = shared_two(
            stream("", content),
            "<< /Font << /F1 5 0 R >> /XObject << /Pic 6 0 R /Sec 7 0 R >> >>".to_string(),
            &[],
        );
        let (mut engine, mut session) = session_with(bytes);
        let (reply, copy) = apply_with(&mut engine, &mut session, json!({ "regions": boxes() }));
        assert_eq!(reply["ok"], true, "{name}: {reply}");
        report_needle(&format!("e3_invisible_{name}/text"), &reply, &copy, SECRET);
    }
}

const ACCT: &str = "ACCTSECRET99";

/// R3, valid and ordinary: a header Form (object 7) drawn on both pages
/// through resources inherited from /Pages holds a name and an account
/// number. Page 1 boxes both; page 2 boxes only the account number. The
/// name shows unboxed on page 2, so `shared` lists the Form and it is kept
/// whole, though every placement boxes the account number.
#[test]
fn e3_partial_boxes_on_two_pages() {
    for shared_res in [false, true] {
        let res = "<< /Font << /F1 5 0 R >> /XObject << /Hdr 7 0 R >> >>";
        let (pages, page) = match shared_res {
            false => (
                format!("<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /Resources {res} >>"),
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 8 0 R >>".to_string(),
            ),
            true => (
                "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources 9 0 R /Contents 8 0 R >>".to_string(),
            ),
        };
        let bytes = classic(&[
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            pages,
            page.clone(),
            page,
            courier(),
            "null".to_string(),
            stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >>",
                &format!(
                    "BT /F1 20 Tf 100 400 Td (NAMEVISIBLE) Tj ET BT /F1 20 Tf 100 300 Td ({ACCT}) Tj ET"
                ),
            ),
            stream("", "/Hdr Do"),
            res.to_string(),
        ]);
        let (mut engine, mut session) = session_with(bytes);
        let regions = json!([
            {"page": 0, "rect": [90.0, 370.0, 400.0, 400.0]},
            {"page": 0, "rect": [90.0, 470.0, 400.0, 500.0]},
            {"page": 1, "rect": [90.0, 470.0, 400.0, 500.0]}
        ]);
        let (reply, copy) = apply_with(&mut engine, &mut session, json!({ "regions": regions }));
        assert_eq!(reply["ok"], true, "{reply}");
        report_needle(
            &format!("e3_partial_boxes shared_res={shared_res}"),
            &reply,
            &copy,
            ACCT,
        );
    }
}

/// R4 (flatten) with two pages sharing resources 10 that hold an unused
/// tiling pattern whose cell draws the secret Form and picture; page 2
/// draws nothing of them.
#[test]
fn e3_flatten_shared_unused_pattern() {
    let bytes = shared_two(
        stream("", "BT /F1 20 Tf 100 200 Td (PAGETWO) Tj ET"),
        "<< /Font << /F1 5 0 R >> /XObject << /Pic 6 0 R /Sec 7 0 R >> /Pattern << /P 11 0 R >> >>"
            .to_string(),
        &[stream(
            "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 50 50] /XStep 50 /YStep 50 /Resources << /XObject << /Sec 7 0 R /Pic 6 0 R >> >>",
            "/Sec Do /Pic Do",
        )],
    );
    let (mut engine, mut session) = session_with(bytes);
    let (reply, copy) = apply_with(
        &mut engine,
        &mut session,
        json!({ "regions": boxes(), "flatten": [0] }),
    );
    report_needle(
        "e3_flatten_shared_unused_pattern/text",
        &reply,
        &copy,
        SECRET,
    );
    report_needle("e3_flatten_shared_unused_pattern/pic", &reply, &copy, PIC);
}

/// Page 2 draws the secret Form only inside an optional-content layer that
/// is off, or through a hidden annotation's appearance.
#[test]
fn e3_hidden_layer_or_annotation_elsewhere() {
    // (a) hidden layer
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [11 0 R] /D << /OFF [11 0 R] >> >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources 10 0 R /Contents 8 0 R >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources 10 0 R /Contents 9 0 R >>".to_string(),
        courier(),
        e3_secret_image(),
        sec_form(400),
        stream("", "q 100 0 0 100 100 500 cm /Pic Do Q /Sec Do"),
        stream("", "/OC /L BDC /Sec Do q 100 0 0 100 100 500 cm /Pic Do Q EMC"),
        "<< /Font << /F1 5 0 R >> /XObject << /Pic 6 0 R /Sec 7 0 R >> /Properties << /L 11 0 R >> >>".to_string(),
        "<< /Type /OCG /Name (Hidden) >>".to_string(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let (reply, copy) = apply_with(&mut engine, &mut session, json!({ "regions": boxes() }));
    report_needle("e3_hidden_layer/text", &reply, &copy, SECRET);
    report_needle("e3_hidden_layer/pic", &reply, &copy, PIC);
    // (b) hidden annotation (F 2) on page 2 whose appearance draws the Form
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources 10 0 R /Contents 8 0 R >>"
            .to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 9 0 R /Annots [11 0 R] >>"
            .to_string(),
        courier(),
        e3_secret_image(),
        sec_form(400),
        stream("", "q 100 0 0 100 100 500 cm /Pic Do Q /Sec Do"),
        stream("", "BT /F1 20 Tf 100 200 Td (PAGETWO) Tj ET"),
        "<< /Font << /F1 5 0 R >> /XObject << /Pic 6 0 R /Sec 7 0 R >> >>".to_string(),
        "<< /Type /Annot /Subtype /Square /Rect [0 0 1 1] /F 2 /AP << /N 12 0 R >> >>".to_string(),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /Sec 7 0 R /Pic 6 0 R >> >>",
            "/Sec Do /Pic Do",
        ),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let (reply, copy) = apply_with(&mut engine, &mut session, json!({ "regions": boxes() }));
    report_needle("e3_hidden_annot/text", &reply, &copy, SECRET);
    report_needle("e3_hidden_annot/pic", &reply, &copy, PIC);
}

/// A page thumbnail (/Thumb) is a picture of the unredacted page.
#[test]
fn e3_thumbnail() {
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R /Thumb 6 0 R >>".to_string(),
        stream("", &format!("BT /F1 20 Tf 100 400 Td ({SECRET}) Tj ET")),
        courier(),
        binary("/Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8", &[0x5A; 16]),
    ]);
    for flatten in [false, true] {
        let (mut engine, mut session) = session_with(bytes.clone());
        let extra = match flatten {
            true => json!({ "regions": boxes(), "flatten": [0] }),
            false => json!({ "regions": boxes() }),
        };
        let (reply, copy) = apply_with(&mut engine, &mut session, extra);
        report_needle(
            &format!("e3_thumbnail flatten={flatten}"),
            &reply,
            &copy,
            PIC,
        );
    }
}

#[test]
fn e3_partial_boxes_where() {
    // The header Form drawn on both pages through inherited resources; the
    // account number boxed on both, the name on page 1 only: it goes from
    // every object (the name may go from page 2 too: over-redaction).
    let res = "<< /Font << /F1 5 0 R >> /XObject << /Hdr 7 0 R >> >>";
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /Resources {res} >>"),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 8 0 R >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 8 0 R >>".to_string(),
        courier(),
        "null".to_string(),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >>",
            &format!(
                "BT /F1 20 Tf 100 400 Td (NAMEVISIBLE) Tj ET BT /F1 20 Tf 100 300 Td ({ACCT}) Tj ET"
            ),
        ),
        stream("", "/Hdr Do"),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let regions = json!([
        {"page": 0, "rect": [90.0, 370.0, 400.0, 400.0]},
        {"page": 0, "rect": [90.0, 470.0, 400.0, 500.0]},
        {"page": 1, "rect": [90.0, 470.0, 400.0, 500.0]}
    ]);
    let (reply, copy) = apply_with(&mut engine, &mut session, json!({ "regions": regions }));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(!holds(&everything(&copy), ACCT));
    assert_eq!(reply["overRedacted"], true, "{reply}");
}

/// R4 at 0908aed: two pages share resources 10 whose /Font holds a Type 3
/// font; only the flattened page 1 shows text with it (its glyph draws the
/// secret picture). `hidden_unused` counts page 2's resources /Font entries
/// (and what they reach) as used, so the picture is not emptied.
#[test]
fn e3_flatten_shared_type3_font() {
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources 10 0 R /Contents 8 0 R >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources 10 0 R /Contents 9 0 R >>".to_string(),
        courier(),
        e3_secret_image(),
        "null".to_string(),
        stream("", "BT /T3 20 Tf 100 400 Td (A) Tj ET"),
        stream("", "BT /F1 20 Tf 100 200 Td (PAGETWO) Tj ET"),
        "<< /Font << /F1 5 0 R /T3 11 0 R >> >>".to_string(),
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] /FirstChar 65 /LastChar 65 /Widths [1000] /Encoding << /Differences [65 /a] >> /CharProcs << /a 12 0 R >> /Resources << /XObject << /Pic 6 0 R >> >> >>".to_string(),
        stream("", "1000 0 d0 q 1000 0 0 1000 0 0 cm /Pic Do Q"),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let regions = json!([{"page": 0, "rect": [90.0, 360.0, 400.0, 400.0]}]);
    let (reply, copy) = apply_with(
        &mut engine,
        &mut session,
        json!({ "regions": regions, "flatten": [0] }),
    );
    report_needle("e3_flatten_shared_type3_font", &reply, &copy, PIC);
}

#[test]
fn d5_a_signed_widget_only_a_page_lists_and_the_security_store_go() {
    // The signed widget (field and widget in one) is in the page's /Annots
    // but not in /Fields; the catalog has /DSS.
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R /DSS << /Certs [] >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R /Annots [6 0 R] >>".to_string(),
        stream("", &format!("BT /F1 20 Tf 100 600 Td ({SECRET}) Tj ET")),
        courier(),
        "<< /Type /Annot /Subtype /Widget /FT /Sig /T (s) /V 7 0 R /Rect [400 700 500 750] /P 3 0 R >>".to_string(),
        "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 0 0 0] /Contents <00> >>".to_string(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let regions = json!([{"page": 0, "rect": [90.0, 170.0, 330.0, 200.0]}]);
    let (reply, _) = apply_with(
        &mut engine,
        &mut session,
        json!({ "regions": regions.clone() }),
    );
    assert_eq!(reply["reason"], "signed", "{reply}");
    let (reply, copy) = apply_with(
        &mut engine,
        &mut session,
        json!({ "regions": regions, "unsign": true }),
    );
    secret_gone(&reply, &copy, "annots-only signature");
    assert!(!holds(&everything(&copy), "adbe.pkcs7"));
    let doc = Document::open(&copy, DocumentLimits::default()).unwrap();
    let catalog = doc.resolve_id(doc.root()).as_dict().unwrap();
    assert!(catalog.get(b"DSS".as_slice()).is_none());
}

// ---- fourth review probes (0908aed..e21e0a1): no copies, in place ---------

const ZPIX: &str = "ZZZZZZZZ";

fn hdr_form() -> String {
    stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >>",
        &format!(
            "BT /F1 20 Tf 100 400 Td (NAMEVISIBLE) Tj ET BT /F1 20 Tf 100 300 Td ({ACCT}) Tj ET"
        ),
    )
}

/// A 4 by 4 gray picture: top rows 'A', bottom rows 'Z' (the secret).
fn two_tone() -> String {
    let mut px = vec![0x41u8; 8];
    px.extend_from_slice(&[0x5A; 8]);
    binary(
        "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8",
        &px,
    )
}

/// Page 1 draws picture 6 and header Form 7 (own resources); page 2 draws
/// `page2` with resources object 10; more objects from 11.
fn p4_pages(page2: &str, res2: &str, extra: &[String], page2_extra: &str) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> /XObject << /Pic 6 0 R /Hdr 7 0 R >> >> /Contents 8 0 R >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources 10 0 R /Contents 9 0 R {page2_extra} >>"),
        courier(),
        two_tone(),
        hdr_form(),
        stream("", "q 100 0 0 100 100 500 cm /Pic Do Q /Hdr Do"),
        stream("", page2),
        res2.to_string(),
    ];
    objects.extend(extra.iter().cloned());
    classic(&objects)
}

/// Page 1's boxes: the account number line and the picture's bottom half.
fn p1_boxes() -> Value {
    json!([
        {"page": 0, "rect": [90.0, 470.0, 400.0, 500.0]},
        {"page": 0, "rect": [90.0, 244.0, 210.0, 300.0]}
    ])
}

/// Applied, nothing secret left anywhere, and the over-redaction flag.
fn p4_check(name: &str, reply: &Value, copy: &[u8], over: bool) {
    assert_eq!(reply["ok"], true, "{name}: {reply}");
    let all = everything(copy);
    assert!(!holds(&all, ACCT), "{name}: the account number is left");
    assert!(
        !holds(&all, ZPIX),
        "{name}: the picture's boxed rows are left"
    );
    assert_eq!(reply["overRedacted"], over, "{name}: {reply}");
}

#[test]
fn p4_a_second_placement_never_keeps_what_a_box_covers() {
    let base = "<< /Font << /F1 5 0 R >> /XObject << /Pic 6 0 R /Hdr 7 0 R >> /ExtGState << /G0 << /ca 0 /CA 0 >> /G1 << /SMask << /S /Luminosity /G 11 0 R >> >> >> >>";
    let group = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Group << /S /Transparency /CS /DeviceGray >>",
        "0 g 0 0 612 792 re f",
    );
    for (name, content) in [
        (
            "control_visible",
            "/Hdr Do q 100 0 0 100 100 500 cm /Pic Do Q",
        ),
        (
            "clip_partial",
            "q 0 350 612 442 re W n /Hdr Do Q q 100 550 100 50 re W n 100 0 0 100 100 500 cm /Pic Do Q",
        ),
        (
            "offpage_partial",
            "q 1 0 0 1 0 -350 cm /Hdr Do Q q 100 0 0 100 100 -50 cm /Pic Do Q",
        ),
        (
            "alpha0",
            "/G0 gs /Hdr Do q 100 0 0 100 100 500 cm /Pic Do Q",
        ),
        (
            "smask_black",
            "/G1 gs /Hdr Do q 100 0 0 100 100 500 cm /Pic Do Q",
        ),
        (
            "textclip_tr7",
            "BT /F1 1 Tf 7 Tr 1 1 Td (.) Tj ET /Hdr Do q 100 0 0 100 100 500 cm /Pic Do Q",
        ),
        (
            "evenodd_empty",
            "0 0 612 792 re 0 0 612 792 re W* n /Hdr Do q 100 0 0 100 100 500 cm /Pic Do Q",
        ),
        (
            "covered_white",
            "/Hdr Do q 100 0 0 100 100 500 cm /Pic Do Q 1 g 0 0 612 792 re f",
        ),
    ] {
        let bytes = p4_pages(content, base, std::slice::from_ref(&group), "");
        let (mut engine, mut session) = session_with(bytes);
        let (reply, copy) = apply_with(&mut engine, &mut session, json!({ "regions": p1_boxes() }));
        p4_check(name, &reply, &copy, true);
    }
}

#[test]
fn p4_uses_no_walk_can_prove_are_flagged() {
    let pattern = stream(
        "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 50 50] /XStep 50 /YStep 50 /Resources << /XObject << /Pic 6 0 R >> >>",
        "q 50 0 0 50 0 0 cm /Pic Do Q",
    );
    let masked = binary(
        "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8 /SMask 6 0 R",
        &[0x80; 16],
    );
    let stamp = "<< /Type /Annot /Subtype /Stamp /Rect [0 0 612 792] /F 4 /AP << /N 12 0 R >> >>"
        .to_string();
    let ap = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /XObject << /Hdr 7 0 R >> >>",
        "/Hdr Do",
    );
    let cases: Vec<(&str, &str, &str, Vec<String>, &str)> = vec![
        (
            "pattern_only",
            "/Pattern cs /P scn 0 0 300 300 re f",
            "<< /Pattern << /P 11 0 R >> >>",
            vec![pattern.clone()],
            "",
        ),
        (
            "pattern_plus_visible",
            "/Pattern cs /P scn 0 0 300 300 re f q 100 0 0 100 100 500 cm /Pic Do Q",
            "<< /Pattern << /P 11 0 R >> /XObject << /Pic 6 0 R >> >>",
            vec![pattern.clone()],
            "",
        ),
        (
            "smask_only",
            "q 100 0 0 100 300 500 cm /M Do Q",
            "<< /XObject << /M 11 0 R >> >>",
            vec![masked.clone()],
            "",
        ),
        (
            "smask_plus_visible",
            "q 100 0 0 100 300 500 cm /M Do Q q 100 0 0 100 100 500 cm /Pic Do Q",
            "<< /XObject << /M 11 0 R /Pic 6 0 R >> >>",
            vec![masked.clone()],
            "",
        ),
        (
            "annot_plus_visible",
            "/Hdr Do",
            "<< /Font << /F1 5 0 R >> /XObject << /Hdr 7 0 R >> >>",
            vec![stamp.clone(), ap.clone()],
            "/Annots [11 0 R]",
        ),
        (
            "annot_only",
            "BT /F1 20 Tf 100 100 Td (PAGETWO) Tj ET",
            "<< /Font << /F1 5 0 R >> >>",
            vec![stamp.clone(), ap.clone()],
            "/Annots [11 0 R]",
        ),
    ];
    for (name, content, res, extra, page2_extra) in cases {
        let bytes = p4_pages(content, res, &extra, page2_extra);
        let (mut engine, mut session) = session_with(bytes);
        let (reply, copy) = apply_with(&mut engine, &mut session, json!({ "regions": p1_boxes() }));
        p4_check(name, &reply, &copy, true);
    }
}

#[test]
fn p4_a_page_listed_twice_turned_or_boxed_differently() {
    // The page tree lists the same page twice.
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 3 0 R] /Count 2 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> /XObject << /Pic 6 0 R /Hdr 7 0 R >> >> /Contents 8 0 R >>".to_string(),
        "null".to_string(),
        courier(),
        two_tone(),
        hdr_form(),
        stream("", "q 100 0 0 100 100 500 cm /Pic Do Q /Hdr Do"),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let (reply, copy) = apply_with(&mut engine, &mut session, json!({ "regions": p1_boxes() }));
    // The app reads the page once.
    let over = reply["overRedacted"] == true;
    p4_check("listed twice", &reply, &copy, over);
    // Page 2 turned, cropped or scaled.
    for extra in [
        "/Rotate 90",
        "/Rotate 270 /CropBox [50 100 600 780]",
        "/UserUnit 2",
    ] {
        let bytes = p4_pages(
            "/Hdr Do q 100 0 0 100 100 500 cm /Pic Do Q",
            "<< /Font << /F1 5 0 R >> /XObject << /Pic 6 0 R /Hdr 7 0 R >> >>",
            &[],
            extra,
        );
        let (mut engine, mut session) = session_with(bytes);
        let (reply, copy) = apply_with(&mut engine, &mut session, json!({ "regions": p1_boxes() }));
        p4_check(extra, &reply, &copy, true);
    }
    // Each page boxes a different line of the shared header: both lose
    // both, and the reply says so.
    let bytes = p4_pages(
        "/Hdr Do",
        "<< /Font << /F1 5 0 R >> /XObject << /Hdr 7 0 R >> >>",
        &[],
        "",
    );
    let (mut engine, mut session) = session_with(bytes);
    let regions = json!([
        {"page": 0, "rect": [90.0, 470.0, 400.0, 500.0]},
        {"page": 1, "rect": [90.0, 370.0, 400.0, 400.0]}
    ]);
    let (reply, copy) = apply_with(&mut engine, &mut session, json!({ "regions": regions }));
    // (The picture is not boxed here.)
    assert_eq!(reply["overRedacted"], true, "{reply}");
    assert!(!holds(&everything(&copy), ACCT));
    assert!(!view_text(&copy).contains("NAMEVISIBLE"));
    // A Type 3 glyph without resources draws the header on page 2.
    let t3 = "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] /FirstChar 65 /LastChar 65 /Widths [1000] /Encoding << /Differences [65 /a] >> /CharProcs << /a 12 0 R >> >>".to_string();
    let bytes = p4_pages(
        "/Hdr Do BT /T3 1 Tf 0 0 Td (A) Tj ET",
        "<< /Font << /F1 5 0 R /T3 11 0 R >> /XObject << /Hdr 7 0 R >> >>",
        &[t3, stream("", "1000 0 d0 /Hdr Do")],
        "",
    );
    let (mut engine, mut session) = session_with(bytes);
    let (reply, copy) = apply_with(&mut engine, &mut session, json!({ "regions": p1_boxes() }));
    p4_check("type 3", &reply, &copy, true);
}

#[test]
fn p4_a_soft_mask_is_blanked_in_place() {
    let mut mask = vec![0xFFu8; 8];
    mask.extend_from_slice(&[0x51; 8]); // 'Q': the secret alpha
    for (name, rect) in [
        ("half", [90.0, 244.0, 210.0, 300.0]),
        ("whole", [90.0, 180.0, 210.0, 300.0]),
    ] {
        for named in [true, false] {
            let res = if named {
                "<< /XObject << /A 6 0 R /S 7 0 R >> >>"
            } else {
                "<< /XObject << /A 6 0 R >> >>"
            };
            let bytes = classic(&[
                "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
                "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources {res} /Contents 4 0 R >>"
                ),
                stream("", "q 100 0 0 100 100 500 cm /A Do Q"),
                "null".to_string(),
                binary(
                    "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8 /SMask 7 0 R",
                    &[0x30; 16],
                ),
                binary(
                    "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8",
                    &mask,
                ),
            ]);
            let (mut engine, mut session) = session_with(bytes);
            let (reply, copy) = apply_with(
                &mut engine,
                &mut session,
                json!({"regions": [{"page": 0, "rect": rect}]}),
            );
            assert_eq!(reply["ok"], true, "{name} {named}: {reply}");
            assert!(!holds(&everything(&copy), "QQQQQQQQ"), "{name} {named}");
            // The unused name reaches the mask: the reply says so.
            assert_eq!(reply["overRedacted"], named, "{name} {named}: {reply}");
        }
    }
}

#[test]
fn p4_flattening_with_a_stamp_and_a_tagged_alternate_text() {
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 9 0 R /MarkInfo << /Marked true >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /StructParents 0 /Resources << /Font << /F1 5 0 R >> /XObject << /Hdr 7 0 R >> /Shading << /Sh 6 0 R >> >> /Contents 8 0 R /Annots [11 0 R] >>".to_string(),
        "null".to_string(),
        courier(),
        "<< /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 612 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> >>".to_string(),
        hdr_form(),
        stream("", "/P << /MCID 0 >> BDC /Hdr Do EMC"),
        "<< /Type /StructTreeRoot /K [10 0 R] /ParentTree << /Nums [0 [10 0 R]] >> >>".to_string(),
        format!("<< /Type /StructElem /S /P /P 9 0 R /Pg 3 0 R /K 0 /Alt ({ACCT}) >>"),
        "<< /Type /Annot /Subtype /Stamp /Rect [0 0 612 792] /F 4 /AP << /N 12 0 R >> >>".to_string(),
        stream("/Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /XObject << /Hdr 7 0 R >> >>", "/Hdr Do"),
    ]);
    let regions = json!([{"page": 0, "rect": [90.0, 470.0, 400.0, 500.0]}]);
    for flatten in [false, true] {
        let (mut engine, mut session) = session_with(bytes.clone());
        let mut extra = json!({ "regions": regions.clone() });
        if flatten {
            extra["flatten"] = json!([0]);
        }
        let (reply, copy) = apply_with(&mut engine, &mut session, extra);
        assert_eq!(reply["ok"], true, "{flatten}: {reply}");
        assert!(!holds(&everything(&copy), ACCT), "{flatten}");
    }
}

#[test]
fn p4_signature_corners() {
    let page = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R ANNOTS >>";
    let sig = "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 0 0 0] /Contents <00> >>".to_string();
    let cases: Vec<(&str, String, String, Vec<String>, &str)> = vec![
        ("orphan_sigdict", "<< /Type /Catalog /Pages 2 0 R >>".into(), page.replace("ANNOTS", ""), vec![sig.clone()], "adbe.pkcs7"),
        ("inherited_ft_annots_only", "<< /Type /Catalog /Pages 2 0 R >>".into(), page.replace("ANNOTS", "/Annots [7 0 R]"),
            vec![sig.clone(), "<< /Type /Annot /Subtype /Widget /Rect [0 0 10 10] /V 6 0 R /Parent 8 0 R >>".into(), "<< /FT /Sig /T (p) /Kids [7 0 R] >>".into()], "adbe.pkcs7"),
        ("dss_only", "<< /Type /Catalog /Pages 2 0 R /DSS << /Certs [] >> >>".into(), page.replace("ANNOTS", ""), vec!["null".into()], "DSS"),
        ("ur3", "<< /Type /Catalog /Pages 2 0 R /Perms << /UR3 6 0 R >> >>".into(), page.replace("ANNOTS", ""), vec!["<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 0 0 0] /Contents <00> /Reference [<< /TransformMethod /UR3 >>] >>".into()], "adbe.pkcs7"),
        ("doctimestamp", "<< /Type /Catalog /Pages 2 0 R /AcroForm << /SigFlags 3 /Fields [7 0 R] >> >>".into(), page.replace("ANNOTS", "/Annots [7 0 R]"),
            vec!["<< /Type /DocTimeStamp /Filter /Adobe.PPKLite /SubFilter /ETSI.RFC3161 /ByteRange [0 0 0 0] /Contents <00> >>".into(), "<< /Type /Annot /Subtype /Widget /FT /Sig /T (ts) /Rect [0 0 0 0] /V 6 0 R >>".into()], "RFC3161"),
    ];
    for (name, catalog, page_obj, extra, left) in cases {
        let mut objects = vec![
            catalog,
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            page_obj,
            stream("", &format!("BT /F1 20 Tf 100 300 Td ({ACCT}) Tj ET")),
            courier(),
        ];
        objects.extend(extra);
        let bytes = classic(&objects);
        let regions = json!([{"page": 0, "rect": [90.0, 470.0, 400.0, 500.0]}]);
        let (mut engine, mut session) = session_with(bytes.clone());
        let (reply, _) = apply_with(
            &mut engine,
            &mut session,
            json!({ "regions": regions.clone() }),
        );
        assert_eq!(reply["reason"], "signed", "{name}: {reply}");
        let rights = if name == "ur3" {
            json!("usage-rights")
        } else {
            Value::Null
        };
        assert_eq!(reply["detail"], rights, "{name}: {reply}");
        let (reply, copy) = apply_with(
            &mut engine,
            &mut session,
            json!({ "regions": regions, "unsign": true }),
        );
        assert_eq!(reply["ok"], true, "{name}: {reply}");
        assert_eq!(reply["unsigned"], true, "{name}");
        let all = everything(&copy);
        assert!(!holds(&all, left), "{name}: {left} left");
        assert!(!holds(&all, ACCT), "{name}");
    }
}

#[test]
fn invisible_ocr_text_elsewhere_does_not_block_a_box() {
    // A scan's invisible text layer (render mode 3): the boxed word also
    // appears, unboxed, further down the page and on page 2.
    let text = |y: u32| format!("BT 3 Tr /F1 20 Tf 100 {y} Td (INVISIBLEWORD) Tj ET");
    let page = |contents: u32| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents {contents} 0 R >>"
        )
    };
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        page(6),
        page(7),
        courier(),
        stream("", &format!("{} {}", text(600), text(300))),
        stream("", &text(600)),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let (reply, copy) = apply_with(
        &mut engine,
        &mut session,
        json!({"regions": [{"page": 0, "rect": [90.0, 170.0, 300.0, 200.0]}]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(view_text(&copy).matches("INVISIBLEWORD").count(), 2);
}
