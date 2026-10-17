// SPDX-License-Identifier: AGPL-3.0-or-later
//! Regressions from the M10 fuzz campaign (engine/design/m10-fuzz.md). Every
//! file is built here.
use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession};

fn dispatch(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    engine.dispatch(session, request, Vec::new()).0
}

fn open(bytes: Vec<u8>) -> (AppEngine, AppSession, Value) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let (reply, _) = engine.dispatch(
        &mut session,
        json!({"op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}}),
        vec![bytes],
    );
    (engine, session, reply)
}

/// Page ids and text of an open session.
fn shown(engine: &mut AppEngine, session: &mut AppSession, reply: &Value) -> (Value, String) {
    assert_eq!(reply["ok"], true, "{reply}");
    let ids = dispatch(engine, session, json!({"op": "page-ids"}))["ids"].clone();
    let text = dispatch(engine, session, json!({"op": "text-all"}));
    assert_eq!(text["ok"], true, "{text}");
    (ids, text["text"].as_str().unwrap().to_string())
}

/// What the file shows, and what its validated save shows when reopened.
fn shown_and_saved(bytes: Vec<u8>) -> [(Value, String); 2] {
    let (mut engine, mut session, reply) = open(bytes);
    let before = shown(&mut engine, &mut session, &reply);
    let (reply, mut outputs) = engine.dispatch(
        &mut session,
        json!({"op": "save", "entropy": {"$buffer": 0}}),
        vec![vec![7; 32]],
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let saved = outputs.remove(0);
    let (check, _) = engine.dispatch(
        &mut session,
        json!({"op": "validate", "pdf": {"$buffer": 0}}),
        vec![saved.clone()],
    );
    assert_eq!(check["ok"], true, "{check}");
    let (mut engine, mut session, reply) = open(saved);
    [before, shown(&mut engine, &mut session, &reply)]
}

/// Objects 1..=n in order, a classic xref, and (when `broken`) a
/// `startxref` that points nowhere, so the file is scanned.
fn assemble(objects: &[String], broken: bool) -> Vec<u8> {
    let mut bytes = b"%PDF-1.4\n".to_vec();
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
    let start = if broken { 999_999 } else { xref };
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn content(text: &str) -> String {
    let data = format!("BT /F1 24 Tf 20 200 Td ({text}) Tj ET q /Fm Do Q");
    format!("<< /Length {} >>\nstream\n{data}\nendstream", data.len())
}

/// Two pages (Alpha: object 3, Beta: object 4) sharing font object 7; the
/// font, Beta's page or a form on Beta carries `damaged` (a dictionary).
fn two_pages(font: &str, beta: &str, form: &str) -> Vec<String> {
    let page = |contents: u32, extra: &str| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources << /Font << /F1 7 0 R >> /XObject << /Fm 8 0 R >> >> /Contents {contents} 0 R{extra} >>"
        )
    };
    let form_body = "BT /F1 18 Tf 20 100 Td (Delta) Tj ET";
    vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
        page(5, ""),
        if beta.is_empty() {
            page(6, "")
        } else {
            beta.into()
        },
        content("Alpha"),
        content("Beta"),
        font.into(),
        format!(
            "{} /Length {} >>\nstream\n{form_body}\nendstream",
            form,
            form_body.len()
        ),
    ]
}

fn with(mut objects: Vec<String>, last: &str) -> Vec<String> {
    objects.push(last.into());
    objects
}

const FONT: &str = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";
const FORM: &str =
    "<< /Type /XObject /Subtype /Form /BBox [0 0 300 300] /Resources << /Font << /F1 7 0 R >> >>";

/// An object header `3 0 obj` damaged into a later dictionary where a value
/// belongs (the p22 token fuzz, seeds 304-306: `/Type 3 0 obj /Font`,
/// `/Subtype 3 0 obj /Form`, `/Rotate 3 0 obj 0`), with the xref broken:
/// MuPDF's repair reads `3 0 obj` there as a damaged value (null) and its
/// dictionary parse goes on, so the header is no object. The scan took it
/// as a later definition of object 3 (`/Font`, a name), which replaced the
/// first page; the view and the save lost page 1 and its text (with a
/// second such header the file was refused).
#[test]
fn a_header_read_as_a_damaged_value_does_not_replace_an_object() {
    let beta = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Rotate 3 0 obj 0 /Resources << /Font << /F1 7 0 R >> /XObject << /Fm 8 0 R >> >> /Contents 6 0 R >>";
    let cases = [
        (
            "font /Type",
            two_pages(
                "<< /Type 3 0 obj /Font /Subtype /Type1 /BaseFont /Helvetica >>",
                "",
                FORM,
            ),
            "AlphaDeltaBetaDelta",
        ),
        (
            "form /Subtype",
            two_pages(
                FONT,
                "",
                "<< /Type /XObject /Subtype 3 0 obj /Form /BBox [0 0 300 300] /Resources << /Font << /F1 7 0 R >> >>",
            ),
            "AlphaBeta",
        ),
        // `0` is then a key that is not a name: the page reads blank, in MuPDF too.
        ("page /Rotate", two_pages(FONT, beta, FORM), "AlphaDelta"),
        (
            "two headers",
            two_pages(
                "<< /Type 3 0 obj /Font /Subtype 4 0 obj /Type1 /BaseFont /Helvetica >>",
                "",
                FORM,
            ),
            "AlphaDeltaBetaDelta",
        ),
        // A copy of a whole object (a page dictionary and `stream`) inside an
        // array of a later object, its header mid-line: the shape of the p7
        // fuzz's seed 37 i=439 (a copy of the object stream's header inside
        // the xref stream's /W array) moved off the line start. The copy
        // replaced page 1, even with an intact xref. (At a line start, as in
        // that input, the copy is still read as the next object: the rule
        // for a dictionary that lost its `>>`; design/m10-fuzz.md.)
        (
            "array copy",
            with(
                two_pages(FONT, "", FORM),
                "<< /Type /Foo /W [1 3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] >>\nstream\nxyz ] (tail) 1 >>",
            ),
            "AlphaDeltaBetaDelta",
        ),
    ];
    // Expected: MuPDF 1.28.1 (engine.ts) on the same files, intact and broken.
    for (label, objects, expected) in cases {
        for broken in [false, true] {
            for (stage, (ids, text)) in ["view", "save"]
                .iter()
                .zip(shown_and_saved(assemble(&objects, broken)))
            {
                let text: String = text.split_whitespace().collect();
                assert_eq!(
                    (ids, text.as_str()),
                    (json!([3, 4]), expected),
                    "{label} broken={broken} {stage}"
                );
            }
        }
    }
}

/// A /Contents array with an entry that is no stream (the p22 token fuzz,
/// seed 306 i=38: `/Contents [ /Xk2 0.000001 8 0 R ]`): MuPDF skips the
/// entry and draws the rest; hayro stopped reading the array at it, so the
/// view showed the page blank and its text was not found (the save kept
/// it). The view now leaves such entries out.
#[test]
fn a_contents_array_entry_that_is_no_stream_is_skipped() {
    let beta = |contents: &str| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Resources << /Font << /F1 7 0 R >> /XObject << /Fm 8 0 R >> >> /Contents {contents} >>"
        )
    };
    for contents in [
        "[ /Xk2 0.000001 6 0 R ]",
        "[ 6 0 R 99 0 R ]",
        "[ 7 0 R 6 0 R ]",
        "[ null 6 0 R (x) ]",
    ] {
        let objects = two_pages(FONT, &beta(contents), FORM);
        for broken in [false, true] {
            for (stage, (ids, text)) in ["view", "save"]
                .iter()
                .zip(shown_and_saved(assemble(&objects, broken)))
            {
                let text: String = text.split_whitespace().collect();
                assert_eq!(
                    (ids, text.as_str()),
                    (json!([3, 4]), "AlphaDeltaBetaDelta"),
                    "{contents} broken={broken} {stage}"
                );
            }
        }
    }
}
