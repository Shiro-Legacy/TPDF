// SPDX-License-Identifier: MIT
//! The question form (`form-questions`, `form-set`): questions laid out on the form's
//! own pages with their labels drawn, fields made, moved, renamed and
//! deleted with the list, pages added and taken away, hand-moved fields
//! pinned, outside field changes followed, one undo step, and the record
//! kept across a save.
use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::writer::{DocumentWriteLimits, WriteOptions, write_document};

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

/// One blank Letter page (File › New).
fn blank() -> Vec<u8> {
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".into(),
        "<< /Length 0 >>\nstream\n\nendstream".into(),
    ])
}

/// One page of someone else's text.
fn letter() -> Vec<u8> {
    let data = "BT /F1 12 Tf 72 700 Td (Dear reader) Tj ET";
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{data}\nendstream", data.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ])
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

fn set(engine: &mut AppEngine, session: &mut AppSession, questions: Value) -> Value {
    ok(
        engine,
        session,
        json!({"op": "form-set", "questions": questions}),
    )
}

fn form(engine: &mut AppEngine, session: &mut AppSession) -> Value {
    ok(engine, session, json!({"op": "form-questions"}))
}

fn text(engine: &mut AppEngine, session: &mut AppSession) -> Vec<String> {
    let reply = ok(engine, session, json!({"op": "text-all"}));
    reply["text"]
        .as_str()
        .unwrap()
        .split('\u{c}')
        .map(str::to_string)
        .collect()
}

fn page_count(engine: &mut AppEngine, session: &mut AppSession) -> usize {
    ok(engine, session, json!({"op": "page-ids"}))["ids"]
        .as_array()
        .unwrap()
        .len()
}

fn fields(engine: &mut AppEngine, session: &mut AppSession, page: usize) -> Vec<Value> {
    ok(engine, session, json!({"op": "fields", "page": page}))["items"]
        .as_array()
        .unwrap()
        .clone()
}

fn rect(item: &Value) -> [f64; 4] {
    let r: Vec<f64> = item["rect"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    [r[0], r[1], r[2], r[3]]
}

/// The question list as the reply shows it, ready to send back.
fn back(reply: &Value) -> Vec<Value> {
    reply["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| {
            json!({"id": q["id"], "text": q["text"], "help": q["help"], "type": q["type"],
                "choices": q["choices"], "required": q["required"]})
        })
        .collect()
}

#[test]
fn questions_are_laid_out_on_a_new_documents_page() {
    let (mut engine, mut session) = session_with(blank());
    let reply = set(
        &mut engine,
        &mut session,
        json!([
            {"text": "What is your email address?", "type": "email", "nameHint": "Email address", "required": true},
            {"text": "Do you drive?", "type": "yesno"},
            {"text": "Favourite colour", "type": "dropdown", "choices": ["Red", "Green"]},
            {"text": "Toppings", "type": "several", "choices": ["Cheese", "Ham"]},
            {"text": "Tell us more", "type": "paragraph", "help": "A few lines is fine."},
            {"text": "I agree to the terms", "type": "agree"},
            {"text": "Signature", "type": "signature"},
        ]),
    );
    // The one blank page is the form page: no page added.
    assert_eq!(reply["formPages"], json!([0]));
    assert_eq!(page_count(&mut engine, &mut session), 1);
    let questions = reply["questions"].as_array().unwrap();
    assert_eq!(questions.len(), 7);
    assert_eq!(questions[0]["name"], "Email address");
    assert_eq!(questions[0]["nameAuto"], true);
    assert_eq!(questions[0]["required"], true);
    assert_eq!(questions[1]["choices"], json!(["Yes", "No"]));
    assert_eq!(questions[3]["name"], "Toppings");
    assert_eq!(questions[3]["widgetIds"].as_array().unwrap().len(), 2);
    let items = fields(&mut engine, &mut session, 0);
    let names: Vec<&str> = items.iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"Toppings - Cheese"), "{names:?}");
    assert!(names.contains(&"Toppings - Ham"));
    let email = items.iter().find(|i| i["name"] == "Email address").unwrap();
    assert_eq!(email["tu"], "What is your email address?");
    assert_eq!(rect(email)[0], 54.0);
    // Blocks go down the page, never overlapping.
    let tops: Vec<f64> = questions
        .iter()
        .map(|q| q["rect"][1].as_f64().unwrap())
        .collect();
    assert!(tops.windows(2).all(|w| w[0] < w[1]), "{tops:?}");
    let page = &text(&mut engine, &mut session)[0];
    for words in [
        "What is your email address? *",
        "Do you drive?",
        "Yes",
        "Cheese",
        "A few lines is fine.",
        "I agree to the terms",
    ] {
        assert!(page.contains(words), "{words} not in {page}");
    }
    // One undo step takes it all away.
    let undo = ok(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(undo["step"], "change form", "{undo}");
    assert!(fields(&mut engine, &mut session, 0).is_empty());
    assert_eq!(form(&mut engine, &mut session)["questions"], json!([]));
    assert!(text(&mut engine, &mut session)[0].trim().is_empty());
}

#[test]
fn a_page_made_elsewhere_is_left_alone_and_a_form_page_added() {
    let (mut engine, mut session) = session_with(letter());
    let reply = set(
        &mut engine,
        &mut session,
        json!([{"text": "Name", "type": "short"}]),
    );
    assert_eq!(reply["formPages"], json!([1]));
    assert_eq!(reply["madePages"], json!([1]));
    assert_eq!(page_count(&mut engine, &mut session), 2);
    assert!(fields(&mut engine, &mut session, 0).is_empty());
    assert_eq!(fields(&mut engine, &mut session, 1).len(), 1);
    let pages = text(&mut engine, &mut session);
    assert_eq!(pages[0].trim(), "Dear reader");
    assert!(pages[1].contains("Name"));
}

#[test]
fn many_questions_flow_onto_added_pages_and_the_pages_go_with_them() {
    let (mut engine, mut session) = session_with(blank());
    let many: Vec<Value> = (1..=30)
        .map(|n| json!({"text": format!("Question {n}"), "type": "short"}))
        .collect();
    let reply = set(&mut engine, &mut session, json!(many));
    assert_eq!(reply["formPages"], json!([0, 1, 2]));
    assert_eq!(page_count(&mut engine, &mut session), 3);
    let questions = reply["questions"].as_array().unwrap();
    assert_eq!(questions[11]["page"], 0);
    assert_eq!(questions[12]["page"], 1);
    // Down to five: the two added pages go again.
    let five: Vec<Value> = back(&reply).into_iter().take(5).collect();
    let reply = set(&mut engine, &mut session, json!(five));
    assert_eq!(reply["formPages"], json!([0]));
    assert_eq!(page_count(&mut engine, &mut session), 1);
    assert_eq!(fields(&mut engine, &mut session, 0).len(), 5);
}

#[test]
fn reordering_moves_the_fields_and_renaming_follows_the_question() {
    let (mut engine, mut session) = session_with(blank());
    let reply = set(
        &mut engine,
        &mut session,
        json!([
            {"text": "First name", "type": "short", "nameHint": "First name"},
            {"text": "Last name", "type": "short", "nameHint": "Last name"},
        ]),
    );
    let ids: Vec<Value> = reply["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| q["widgetIds"][0].clone())
        .collect();
    let mut list = back(&reply);
    list.swap(0, 1);
    list[1]["text"] = json!("Given name");
    list[1]["nameHint"] = json!("Given name");
    let reply = set(&mut engine, &mut session, json!(list));
    let questions = reply["questions"].as_array().unwrap();
    // The same fields, swapped down the page.
    assert_eq!(questions[0]["widgetIds"][0], ids[1]);
    assert_eq!(questions[1]["widgetIds"][0], ids[0]);
    assert!(questions[0]["rect"][1].as_f64() < questions[1]["rect"][1].as_f64());
    assert_eq!(questions[1]["name"], "Given name");
    let page = &text(&mut engine, &mut session)[0];
    assert!(page.find("Last name").unwrap() < page.find("Given name").unwrap());
    // A changed type makes the field again.
    let mut list = back(&reply);
    list[0]["type"] = json!("one");
    list[0]["choices"] = json!(["A", "B", "C"]);
    let reply = set(&mut engine, &mut session, json!(list));
    let first = &reply["questions"][0];
    assert_eq!(first["type"], "one");
    assert_eq!(first["choices"], json!(["A", "B", "C"]));
    assert_eq!(first["name"], "Last name");
}

#[test]
fn a_field_moved_by_hand_is_pinned_and_its_label_follows() {
    let (mut engine, mut session) = session_with(blank());
    let reply = set(
        &mut engine,
        &mut session,
        json!([
            {"text": "Moved", "type": "short"},
            {"text": "Stays in line", "type": "short"},
        ]),
    );
    let moved = reply["questions"][0]["widgetIds"][0].as_i64().unwrap();
    // Moved from outside the list (the popover, a drag): the form follows
    // in that step.
    let request =
        json!({"op": "field-props", "page": 0, "widgetId": moved, "rect": [300, 400, 500, 422]});
    ok(&mut engine, &mut session, request);
    let listed = form(&mut engine, &mut session);
    assert_eq!(listed["questions"][0]["pinned"], true);
    assert_eq!(
        listed["questions"][0]["rect"],
        json!([300.0, 400.0, 500.0, 422.0])
    );
    // The other question moved up into the gap.
    assert_eq!(listed["questions"][1]["rect"][1], 71.0);
    let undo = ok(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(undo["step"], "change field");
    assert_eq!(
        form(&mut engine, &mut session)["questions"][0]["pinned"],
        false
    );
    ok(&mut engine, &mut session, json!({"op": "redo"}));
    // Put back in line.
    let mut list = back(&listed);
    list[0]["pinned"] = json!(false);
    let reply = set(&mut engine, &mut session, json!(list));
    assert_eq!(reply["questions"][0]["pinned"], false);
    assert_eq!(reply["questions"][0]["rect"][1], 71.0);
}

#[test]
fn outside_changes_reach_the_labels() {
    let (mut engine, mut session) = session_with(blank());
    let reply = set(
        &mut engine,
        &mut session,
        json!([
            {"text": "Phone", "type": "phone"},
            {"text": "City", "type": "short"},
        ]),
    );
    let phone = reply["questions"][0]["widgetIds"][0].as_i64().unwrap();
    let request = json!({"op": "field-props", "page": 0, "widgetId": phone, "required": true, "label": "Phone number"});
    ok(&mut engine, &mut session, request);
    let listed = form(&mut engine, &mut session);
    assert_eq!(listed["questions"][0]["text"], "Phone number");
    assert_eq!(listed["questions"][0]["required"], true);
    assert!(text(&mut engine, &mut session)[0].contains("Phone number *"));
    // Deleting the field takes the question and its label away.
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-delete", "page": 0, "widgetId": phone}),
    );
    let listed = form(&mut engine, &mut session);
    assert_eq!(listed["questions"].as_array().unwrap().len(), 1);
    assert_eq!(listed["questions"][0]["rect"][1], 71.0);
    assert!(!text(&mut engine, &mut session)[0].contains("Phone"));
}

#[test]
fn a_field_drawn_on_the_page_is_unlisted_until_adopted() {
    let (mut engine, mut session) = session_with(blank());
    set(
        &mut engine,
        &mut session,
        json!([{"text": "Name", "type": "short"}]),
    );
    let drawn = ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "text", "rect": [54, 300, 254, 322], "nameHint": ""}),
    );
    let id = drawn["widgetId"].as_i64().unwrap();
    let listed = form(&mut engine, &mut session);
    assert_eq!(listed["unlisted"][0]["widgetIds"], json!([id]));
    assert_eq!(listed["unlisted"][0]["name"], "Text 1");
    assert_eq!(listed["unlisted"][0]["onFormPage"], true);
    let mut list = back(&listed);
    list.push(json!({"widgetId": id, "text": "Where do you live?", "type": "short", "nameHint": "Where do you live"}));
    let reply = set(&mut engine, &mut session, json!(list));
    let adopted = &reply["questions"][1];
    assert_eq!(adopted["pinned"], true);
    assert_eq!(adopted["rect"], json!([54.0, 300.0, 254.0, 322.0]));
    assert_eq!(adopted["name"], "Where do you live");
    assert_eq!(reply["unlisted"], json!([]));
    assert!(text(&mut engine, &mut session)[0].contains("Where do you live?"));
}

// A new document's page with a field drawn on it before any question: typing the
// field's question makes the page the form's, draws the label, and the type can change.
#[test]
fn a_field_drawn_first_on_a_new_page_makes_it_the_form_page() {
    let (mut engine, mut session) = session_with(blank());
    let drawn = ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "checkbox", "rect": [54, 700, 68, 714], "nameHint": ""}),
    );
    let id = drawn["widgetId"].as_i64().unwrap();
    let listed = form(&mut engine, &mut session);
    assert_eq!(listed["formPages"], json!([]));
    let reply = set(
        &mut engine,
        &mut session,
        json!([{"widgetId": id, "text": "What is your email address?", "type": "agree"}]),
    );
    assert_eq!(reply["formPages"], json!([0]));
    assert_eq!(page_count(&mut engine, &mut session), 1);
    let q = &reply["questions"][0];
    assert_eq!(q["foreign"], false);
    assert_eq!(q["pinned"], true);
    assert!(text(&mut engine, &mut session)[0].contains("What is your email address?"));
    // Its type can change: the checkbox becomes a text field in its place.
    let mut list = back(&reply);
    list[0]["type"] = json!("email");
    let reply = set(&mut engine, &mut session, json!(list));
    let q = &reply["questions"][0];
    assert_eq!(q["type"], "email");
    let items = fields(&mut engine, &mut session, 0);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "text");
    assert!(text(&mut engine, &mut session)[0].contains("What is your email address?"));
}

#[test]
fn the_form_survives_a_save() {
    let (mut engine, mut session) = session_with(blank());
    let reply = ok(
        &mut engine,
        &mut session,
        json!({"op": "form-set", "title": "Sign-up", "questions": [
            {"text": "Email", "type": "email", "typeAuto": true},
            {"text": "Pick one", "type": "one", "choices": ["Left", "Right"]},
        ]}),
    );
    assert_eq!(reply["title"], "Sign-up");
    let bytes = write_document(
        session.doc.as_ref().unwrap().document(),
        DocumentWriteLimits::default(),
        WriteOptions::APP,
    )
    .unwrap();
    let (mut engine, mut session) = session_with(bytes);
    let listed = form(&mut engine, &mut session);
    assert_eq!(listed["title"], "Sign-up");
    assert_eq!(listed["questions"][0]["type"], "email");
    assert_eq!(listed["questions"][0]["typeAuto"], true);
    assert_eq!(listed["questions"][1]["choices"], json!(["Left", "Right"]));
    assert_eq!(listed["questions"][1]["pinned"], false);
    // Laying it out again moves nothing.
    let again = set(&mut engine, &mut session, json!(back(&listed)));
    assert_eq!(
        again["questions"][1]["rect"],
        listed["questions"][1]["rect"]
    );
}

#[test]
fn refusals_say_what_is_wrong() {
    let (mut engine, mut session) = session_with(blank());
    let cases = [
        (json!([{"text": "日本語", "type": "short"}]), "not-drawable"),
        (
            json!([{"text": "One", "type": "one", "choices": ["Only"]}]),
            "invalid",
        ),
        (
            json!([{"text": "Dup", "type": "one", "choices": ["A", "A"]}]),
            "invalid",
        ),
        (json!([{"text": "Kind", "type": "slider"}]), "invalid"),
        (
            json!([{"id": 99, "text": "Gone", "type": "short"}]),
            "stale-target",
        ),
    ];
    for (questions, reason) in cases {
        let reply = op(
            &mut engine,
            &mut session,
            json!({"op": "form-set", "questions": questions}),
        );
        assert_eq!(reply["ok"], false, "{reply}");
        assert_eq!(reply["reason"], reason, "{reply}");
    }
    // Nothing was written.
    assert!(fields(&mut engine, &mut session, 0).is_empty());
    let undo = op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_ne!(undo["step"], "change form");
}

#[test]
fn a_field_on_a_page_made_elsewhere_takes_the_question_as_its_description() {
    let (mut engine, mut session) = session_with(letter());
    let drawn = ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "text", "rect": [72, 120, 272, 142], "name": "reader"}),
    );
    let id = drawn["widgetId"].as_i64().unwrap();
    let listed = form(&mut engine, &mut session);
    assert_eq!(listed["unlisted"][0]["onFormPage"], false);
    let reply = set(
        &mut engine,
        &mut session,
        json!([{"widgetId": id, "text": "Who is reading?", "type": "short"}]),
    );
    assert_eq!(reply["questions"][0]["foreign"], true);
    assert_eq!(reply["formPages"], json!([]));
    assert_eq!(page_count(&mut engine, &mut session), 1);
    let item = &fields(&mut engine, &mut session, 0)[0];
    assert_eq!(item["tu"], "Who is reading?");
    assert_eq!(rect(item), [72.0, 120.0, 272.0, 142.0]);
    assert_eq!(text(&mut engine, &mut session)[0].trim(), "Dear reader");
}
