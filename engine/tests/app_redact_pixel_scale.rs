// SPDX-License-Identifier: AGPL-3.0-or-later
//! redact-apply's pixel check on pages the view refuses to draw at full
//! scale (final review, minor 1): a page of 12,000 or 20,000 page-size
//! fills passes the render's scene budget at scale 1 but draws at 0.5, and
//! apply refused `verify-failed` "pixels check failed" (every mark failed
//! on any render error). The check now draws such a page again at half
//! the scale, down to a quarter, refusing (`scan-budget`) when a mark it
//! checks at full scale keeps under two pixels at the reduced scale, or
//! when the page still cannot be drawn.
use serde_json::{Value, json};
use tandem_pdf_engine::app::redact::marks::Mark;
use tandem_pdf_engine::app::redact::verify::{self, PixelRefusal};
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::view::{RenderBudgets, RenderOptions, ViewDocument};

fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
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

/// A letter page of `fills` page-size grey fills, then `more` content.
fn fills_pdf(fills: usize, more: &str) -> Vec<u8> {
    let mut content = String::new();
    for i in 0..fills {
        content.push_str(&format!("0.{} g 0 0 612 792 re f\n", i % 7 + 1));
    }
    content.push_str(more);
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(content.as_bytes(), 9);
    let mut body = format!(
        "<< /Filter /FlateDecode /Length {} >>\nstream\n",
        packed.len()
    )
    .into_bytes();
    body.extend_from_slice(&packed);
    body.extend_from_slice(b"\nendstream");
    pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        body,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    ])
}

fn area(rect: [f64; 4]) -> Mark {
    Mark {
        page: 0,
        id: None,
        rect,
        quads: Vec::new(),
        search: None,
    }
}

/// The scene bytes the page's render estimates at `scale`.
fn scene(view: &ViewDocument, scale: f64) -> f64 {
    let unbounded = RenderBudgets {
        scene_bytes: f64::MAX,
        ..RenderBudgets::default()
    };
    let (drawn, stats) = view.render_with_stats(0, scale, RenderOptions::default(), unbounded);
    assert!(drawn.is_ok());
    stats.scene_bytes
}

/// Budgets that refuse the page at `refused` and draw it at `drawn`.
fn budgets_between(view: &ViewDocument, refused: f64, drawn: f64) -> RenderBudgets {
    let (high, low) = (scene(view, refused), scene(view, drawn));
    assert!(low < high, "{low} {high}");
    RenderBudgets {
        scene_bytes: (low + high) / 2.0,
        ..RenderBudgets::default()
    }
}

/// A black box from (100, 100) to (300, 150) in PDF space, page space
/// [100, 642, 300, 692], over 100 fills; `over` is drawn on top of it.
fn boxed(over: &str) -> ViewDocument {
    let bytes = fills_pdf(100, &format!("0 g 100 100 200 50 re f {over}"));
    ViewDocument::open(bytes, "").unwrap()
}

#[test]
fn a_page_refused_at_full_scale_is_checked_at_half_and_at_a_quarter() {
    let clean = boxed("");
    let marked = [area([110.0, 650.0, 290.0, 685.0])];
    // Within the app's budgets the page draws at full scale.
    assert!(verify::pixels(&clean, &marked).unwrap().is_empty());
    let half = budgets_between(&clean, 1.0, 0.5);
    let quarter = budgets_between(&clean, 0.5, 0.25);
    for budgets in [half, quarter] {
        assert!(
            clean
                .render_with_budgets(0, 1.0, RenderOptions::default(), budgets)
                .is_err()
        );
        assert!(
            verify::pixels_with(&clean, &marked, budgets)
                .unwrap()
                .is_empty()
        );
    }
    // A white hairline (0.5 pt, an eighth of a pixel at a quarter scale)
    // left over the box still fails its mark at each reduced scale.
    let lined = boxed("1 G 0.5 w 150 120 m 250 120 l S");
    for budgets in [half, quarter] {
        let failed = verify::pixels_with(&lined, &marked, budgets).unwrap();
        assert_eq!(failed.into_iter().collect::<Vec<_>>(), vec![0]);
    }
}

#[test]
fn a_mark_too_thin_for_the_reduced_scale_is_refused_not_passed() {
    let clean = boxed("");
    let half = budgets_between(&clean, 1.0, 0.5);
    // 8 pt tall: 4 pixels inside the margin at full scale, 1 at half.
    let thin = [area([110.0, 655.0, 290.0, 663.0])];
    assert!(verify::pixels(&clean, &thin).unwrap().is_empty());
    assert_eq!(
        verify::pixels_with(&clean, &thin, half),
        Err(PixelRefusal::TooThin(0))
    );
    // 3 pt tall: not checked at full scale either (inside the two-pixel
    // margin), so the reduced scale changes nothing for it.
    let unchecked = [area([110.0, 660.0, 290.0, 663.0])];
    assert!(
        verify::pixels_with(&clean, &unchecked, half)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_page_refused_at_every_scale_is_refused_too_complex() {
    let clean = boxed("");
    let tiny = RenderBudgets {
        scene_bytes: scene(&clean, 0.25) / 2.0,
        ..RenderBudgets::default()
    };
    let marked = [area([110.0, 650.0, 290.0, 685.0])];
    assert_eq!(
        verify::pixels_with(&clean, &marked, tiny),
        Err(PixelRefusal::TooComplex(0))
    );
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

fn apply(engine: &mut AppEngine, session: &mut AppSession) -> (Value, Vec<u8>) {
    let off = false;
    let sanitize = json!({"info": off, "xmp": off, "attachments": off, "outlines": off,
        "annots": off, "links": off, "fields": off, "layers": off, "struct": off});
    let request = json!({"id": 9, "op": "redact-apply", "sanitize": sanitize,
        "entropy": {"$buffer": 0}});
    let (reply, mut outputs) = engine.dispatch(session, request, vec![vec![7u8; 32]]);
    let bytes = match reply["ok"] == true {
        true => outputs.remove(0),
        false => Vec::new(),
    };
    (reply, bytes)
}

const SECRET: &str = "0 g BT /F1 24 Tf 72 700 Td (SECRET TEXT) Tj ET\n";

#[test]
fn page_size_fills_past_the_scene_budget_apply_at_half_scale() {
    // The review's probe: 9,000 fills applied; 12,000 and 20,000 refused.
    for fills in [12_000, 20_000] {
        let (mut engine, mut session) = session_with(fills_pdf(fills, SECRET));
        let full = op(
            &mut engine,
            &mut session,
            json!({"op": "render", "page": 0, "scale": 1}),
        );
        assert_eq!(full["ok"], false, "{fills}: {full}");
        let half = op(
            &mut engine,
            &mut session,
            json!({"op": "render", "page": 0, "scale": 0.5}),
        );
        assert_eq!(half["ok"], true, "{fills}");
        let found = op(
            &mut engine,
            &mut session,
            json!({"op": "redact-search", "text": "SECRET"}),
        );
        assert_eq!(found["ok"], true, "{found}");
        let (reply, bytes) = apply(&mut engine, &mut session);
        assert_eq!(reply["ok"], true, "{fills}: {reply}");
        assert_eq!(reply["verified"]["pixels"], 1, "{reply}");
        let copy = ViewDocument::open(bytes, "").unwrap();
        let text = copy.structured_text(0).unwrap().as_text();
        assert!(!text.contains("SECRET"), "{fills}");
    }
}

#[test]
fn a_thin_mark_on_a_page_past_the_scene_budget_refuses_scan_budget() {
    let (mut engine, mut session) = session_with(fills_pdf(12_000, SECRET));
    // 6 pt tall, away from the text: 2 pixels inside the margin at full
    // scale, none at half.
    let added = op(
        &mut engine,
        &mut session,
        json!({"op": "redact-add", "page": 0, "rect": [100, 300, 300, 306]}),
    );
    assert_eq!(added["ok"], true, "{added}");
    let (reply, _) = apply(&mut engine, &mut session);
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["reason"], "scan-budget", "{reply}");
    assert_eq!(
        reply["error"],
        "redact-apply: page 1 is too complex to draw at a resolution that verifies its thinnest marks"
    );
}
