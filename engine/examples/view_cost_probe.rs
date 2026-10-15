//! Costs of the app's view (render bytes, view redefinitions, reparse):
//!   view_cost_probe FILE [PAGE]
//! opens FILE as the app does, then times the first view, a render of PAGE,
//! a small edit (a note on PAGE) with the view and render after it, and an
//! undo with the view and render after it. Medians of 5 runs, milliseconds.
use serde_json::{Value, json};
use std::time::Instant;
use tandem_pdf_engine::app::{AppEngine, AppSession};

fn op(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    let (reply, _) = engine.dispatch(session, request, Vec::new());
    assert_eq!(reply["ok"], true, "{reply}");
    reply
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).expect("file");
    let page: usize = args.get(2).map_or(0, |p| p.parse().expect("page"));
    let mut rows: Vec<(&str, Vec<f64>)> = [
        "open",
        "first view",
        "render",
        "edit",
        "edit view",
        "edit render",
        "undo",
        "undo view",
        "undo render",
    ]
    .into_iter()
    .map(|name| (name, Vec::new()))
    .collect();
    for _ in 0..5 {
        let mut engine = AppEngine::new();
        let mut session = AppSession::default();
        let open = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
        let start = Instant::now();
        let (reply, _) = engine.dispatch(&mut session, open, vec![bytes.clone()]);
        assert_eq!(reply["ok"], true, "{reply}");
        rows[0].1.push(ms(start));
        let start = Instant::now();
        session.doc.as_mut().unwrap().view().expect("view");
        rows[1].1.push(ms(start));
        let render = json!({"op": "render", "page": page, "scale": 1});
        let start = Instant::now();
        op(&mut engine, &mut session, render.clone());
        rows[2].1.push(ms(start));
        let note = json!({"op": "annot-add", "page": page, "type": "Text", "point": [100, 100], "contents": "x"});
        let start = Instant::now();
        op(&mut engine, &mut session, note);
        rows[3].1.push(ms(start));
        let start = Instant::now();
        session.doc.as_mut().unwrap().view().expect("view");
        rows[4].1.push(ms(start));
        let start = Instant::now();
        op(&mut engine, &mut session, render.clone());
        rows[5].1.push(ms(start));
        let start = Instant::now();
        op(&mut engine, &mut session, json!({"op": "undo"}));
        rows[6].1.push(ms(start));
        let start = Instant::now();
        session.doc.as_mut().unwrap().view().expect("view");
        rows[7].1.push(ms(start));
        let start = Instant::now();
        op(&mut engine, &mut session, render);
        rows[8].1.push(ms(start));
    }
    for (name, values) in rows {
        println!("{name:12} {:9.2}", median(values));
    }
}
