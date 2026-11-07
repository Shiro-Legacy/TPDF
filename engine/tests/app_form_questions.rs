// SPDX-License-Identifier: MIT
//! The question form (`form-questions`, `form-set`): questions laid out on the form's
//! own pages with their labels drawn, fields made, moved, renamed and
//! deleted with the list, pages added and taken away, hand-moved fields
//! pinned, outside field changes followed, one undo step, and the record
//! kept across a save.
use std::collections::BTreeSet;

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
                "choices": q["choices"], "required": q["required"], "other": q["other"],
                "rows": q["rows"], "steps": q["steps"], "style": q["style"],
                "newPage": q["newPage"]})
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
    // The blank page became the form's when the field was put on it.
    let listed = form(&mut engine, &mut session);
    assert_eq!(listed["formPages"], json!([0]));
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

/// The /MK /CA of a widget, if any.
fn caption(session: &AppSession, widget: i64) -> Option<String> {
    let doc = session.doc.as_ref().unwrap().document();
    let id = tandem_pdf_engine::object::ObjectId::new(widget as u32, 0).unwrap();
    let dict = doc.resolve_id(id).as_dict()?;
    let mk = doc.dict_get(dict, b"MK").as_dict()?;
    let ca = doc.dict_get(mk, b"CA").as_string()?;
    Some(String::from_utf8_lossy(ca).into_owned())
}

fn by_name<'a>(items: &'a [Value], name: &str) -> Vec<&'a Value> {
    items.iter().filter(|f| f["name"] == name).collect()
}

// A section is a heading with no fields; "new page" starts it on the next form page.
#[test]
fn a_section_draws_a_heading_and_can_start_a_page() {
    let (mut engine, mut session) = session_with(blank());
    let reply = set(
        &mut engine,
        &mut session,
        json!([
            {"text": "Name", "type": "short"},
            {"text": "About you", "help": "Tell us a little more.", "type": "section"},
            {"text": "Age", "type": "number"},
        ]),
    );
    let q = &reply["questions"];
    assert_eq!(q.as_array().unwrap().len(), 3);
    assert_eq!(q[1]["type"], "section");
    assert_eq!(q[1]["widgetIds"], json!([]));
    assert_eq!(fields(&mut engine, &mut session, 0).len(), 2);
    let page = &text(&mut engine, &mut session)[0];
    assert!(page.contains("About you") && page.contains("Tell us a little more."));
    // Below the section: the age field is lower than the name field.
    assert!(rect(&q[2])[1] > rect(&q[0])[1] + 40.0);
    // Starting a new page.
    let mut list = back(&reply);
    list[1]["newPage"] = json!(true);
    let reply = set(&mut engine, &mut session, json!(list));
    assert_eq!(page_count(&mut engine, &mut session), 2);
    assert_eq!(reply["questions"][1]["newPage"], true);
    assert_eq!(reply["questions"][2]["page"], 1);
    assert!(text(&mut engine, &mut session)[1].contains("About you"));
    // Laid out again from outside (a field change), the section stays.
    let age = reply["questions"][2]["widgetIds"][0].as_i64().unwrap();
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-props", "page": 1, "widgetId": age, "required": true}),
    );
    let listed = form(&mut engine, &mut session);
    assert_eq!(listed["questions"][1]["type"], "section");
    assert_eq!(listed["questions"][2]["required"], true);
    // Taken away, the page it started goes too.
    let mut list = back(&listed);
    list.remove(1);
    set(&mut engine, &mut session, json!(list));
    assert_eq!(page_count(&mut engine, &mut session), 1);
    assert!(!text(&mut engine, &mut session)[0].contains("About you"));
}

// "Other": one more button or checkbox and a text box for the answer.
#[test]
fn an_other_choice_adds_a_text_box() {
    let (mut engine, mut session) = session_with(blank());
    let reply = set(
        &mut engine,
        &mut session,
        json!([
            {"text": "Favourite colour", "type": "one", "choices": ["Red", "Blue"], "other": true},
            {"text": "Pets", "type": "several", "choices": ["Cat", "Dog"], "other": true},
        ]),
    );
    let q = &reply["questions"];
    assert_eq!(q[0]["choices"], json!(["Red", "Blue"]));
    assert_eq!(q[0]["other"], true);
    assert_eq!(q[0]["widgetIds"].as_array().unwrap().len(), 4);
    let items = fields(&mut engine, &mut session, 0);
    let radio = by_name(&items, "Favourite colour");
    let ons: Vec<&str> = radio.iter().map(|r| r["on"].as_str().unwrap()).collect();
    assert_eq!(ons, ["Red", "Blue", "Other"]);
    let other = by_name(&items, "Favourite colour - Other");
    assert_eq!(other.len(), 1);
    assert_eq!(other[0]["kind"], "text");
    assert_eq!(by_name(&items, "Pets - Other")[0]["kind"], "checkbox");
    assert_eq!(by_name(&items, "Pets - Other text")[0]["kind"], "text");
    assert_eq!(by_name(&items, "Pets - Cat").len(), 1);
    assert!(text(&mut engine, &mut session)[0].contains("Other"));
    assert_eq!(reply["unlisted"], json!([]));
    // "Other" is the box's own choice.
    let refused = op(
        &mut engine,
        &mut session,
        json!({"op": "form-set", "questions": [
            {"text": "X", "type": "one", "choices": ["A", "Other"], "other": true}]}),
    );
    assert_eq!(refused["ok"], false);
    // Renaming the question renames the box; turning Other off takes it away.
    let mut list = back(&reply);
    list[0]["text"] = json!("Colour");
    list[0]["nameHint"] = json!("Colour");
    list[0]["other"] = json!(true);
    list[1]["other"] = json!(true);
    set(&mut engine, &mut session, json!(list.clone()));
    let items = fields(&mut engine, &mut session, 0);
    assert_eq!(by_name(&items, "Colour - Other").len(), 1, "{items:?}");
    list[0]["other"] = json!(false);
    let reply = set(&mut engine, &mut session, json!(list));
    assert_eq!(reply["questions"][0]["other"], false);
    let items = fields(&mut engine, &mut session, 0);
    assert!(by_name(&items, "Colour - Other").is_empty());
    assert_eq!(by_name(&items, "Colour").len(), 2);
}

// A rating: stars (Acrobat's star buttons) or numbered buttons, 3 to 10 steps.
#[test]
fn a_rating_is_stars_or_numbers() {
    let (mut engine, mut session) = session_with(blank());
    let reply = set(
        &mut engine,
        &mut session,
        json!([{"text": "How was the food?", "type": "rating"}]),
    );
    let q = &reply["questions"][0];
    assert_eq!(q["steps"], 5);
    assert_eq!(q["style"], "stars");
    assert_eq!(q["choices"], json!(["1", "2", "3", "4", "5"]));
    let ids: Vec<i64> = q["widgetIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(ids.len(), 5);
    assert!(
        ids.iter()
            .all(|&id| caption(&session, id).as_deref() == Some("H"))
    );
    // Seven numbered buttons, all on one row.
    let mut list = back(&reply);
    list[0]["steps"] = json!(7);
    list[0]["style"] = json!("numbers");
    let reply = set(&mut engine, &mut session, json!(list));
    let q = &reply["questions"][0];
    assert_eq!(q["steps"], 7);
    assert_eq!(q["style"], "numbers");
    let ids: Vec<i64> = q["widgetIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert!(ids.iter().all(|&id| caption(&session, id).is_none()));
    let items = fields(&mut engine, &mut session, 0);
    let tops: BTreeSet<i64> = items.iter().map(|f| rect(f)[1] as i64).collect();
    assert_eq!(tops.len(), 1, "one row: {items:?}");
    let refused = op(
        &mut engine,
        &mut session,
        json!({"op": "form-set", "questions": [{"text": "X", "type": "rating", "steps": 11}]}),
    );
    assert_eq!(refused["ok"], false);
}

// A Likert grid: one choice-button group per statement, the scale over the columns.
#[test]
fn a_likert_grid_has_a_group_per_statement() {
    let (mut engine, mut session) = session_with(blank());
    let scale = ["Disagree", "Neutral", "Agree"];
    let reply = set(
        &mut engine,
        &mut session,
        json!([{"text": "The course", "type": "likert", "choices": scale,
            "rows": ["Was clear", "Was useful"], "required": true}]),
    );
    let q = &reply["questions"][0];
    assert_eq!(q["rows"], json!(["Was clear", "Was useful"]));
    assert_eq!(q["choices"], json!(scale));
    assert_eq!(q["required"], true);
    let items = fields(&mut engine, &mut session, 0);
    for row in ["The course - Was clear", "The course - Was useful"] {
        let group = by_name(&items, row);
        let ons: Vec<&str> = group.iter().map(|r| r["on"].as_str().unwrap()).collect();
        assert_eq!(ons, scale, "{row}");
        assert!(group.iter().all(|r| r["required"] == true));
    }
    let page = &text(&mut engine, &mut session)[0];
    for words in [
        "The course *",
        "Disagree",
        "Neutral",
        "Agree",
        "Was clear",
        "Was useful",
    ] {
        assert!(page.contains(words), "{words}: {page}");
    }
    // A third statement, then the form survives a save.
    let mut list = back(&reply);
    list[0]["rows"] = json!(["Was clear", "Was useful", "Was fun"]);
    let reply = set(&mut engine, &mut session, json!(list));
    assert_eq!(
        reply["questions"][0]["widgetIds"].as_array().unwrap().len(),
        9
    );
    let bytes = write_document(
        session.doc.as_ref().unwrap().document(),
        DocumentWriteLimits::default(),
        WriteOptions::APP,
    )
    .unwrap();
    let (mut engine, mut session) = session_with(bytes);
    let listed = form(&mut engine, &mut session);
    assert_eq!(listed["questions"][0]["type"], "likert");
    assert_eq!(listed["questions"][0]["rows"].as_array().unwrap().len(), 3);
    assert_eq!(listed["unlisted"], json!([]));
    let again = set(&mut engine, &mut session, json!(back(&listed)));
    assert_eq!(
        again["questions"][0]["rect"],
        listed["questions"][0]["rect"]
    );
    // A grid's scale is 2 to 7 choices.
    let refused = op(
        &mut engine,
        &mut session,
        json!({"op": "form-set", "questions": [{"text": "X", "type": "likert",
            "choices": ["1","2","3","4","5","6","7","8"], "rows": ["A"]}]}),
    );
    assert_eq!(refused["ok"], false);
}

fn overlaps(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

fn boxed(v: &Value) -> [f64; 4] {
    let r: Vec<f64> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_f64().unwrap())
        .collect();
    [r[0], r[1], r[2], r[3]]
}

// The title keeps clear of a field placed by hand at the top (the owner's case: a
// checkbox drawn first, then a title added).
#[test]
fn a_title_does_not_cover_a_field_placed_by_hand() {
    let (mut engine, mut session) = session_with(blank());
    let drawn = ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "checkbox", "rect": [54, 60, 68, 74], "nameHint": ""}),
    );
    let id = drawn["widgetId"].as_i64().unwrap();
    let reply = set(
        &mut engine,
        &mut session,
        json!([{"widgetId": id, "text": "What is your email?", "type": "agree"}]),
    );
    let reply = ok(
        &mut engine,
        &mut session,
        json!({"op": "form-set", "title": "Test", "questions": back(&reply)}),
    );
    let field = [54.0, 60.0, 68.0, 74.0];
    let title = boxed(&reply["titleBox"]);
    assert!(!overlaps(title, field), "title {title:?}");
    let label = boxed(&reply["questions"][0]["box"]);
    assert!(!overlaps(title, label), "title {title:?} label {label:?}");
}

// Words on the page move: a question's fields go with its label (and it is pinned), a
// section's heading and the title keep the place they were put, until put back in line.
#[test]
fn labels_headings_and_the_title_move_on_the_page() {
    let (mut engine, mut session) = session_with(blank());
    let reply = ok(
        &mut engine,
        &mut session,
        json!({"op": "form-set", "title": "Sign-up", "questions": [
            {"text": "Name", "type": "short"},
            {"text": "About you", "type": "section"},
            {"text": "Age", "type": "number"},
        ]}),
    );
    let q = &reply["questions"];
    assert_eq!(q[0]["boxPage"], 0);
    let name_box = boxed(&q[0]["box"]);
    let name_rect = rect(&q[0]);
    // The question: its field moves by the same amount, and it is pinned.
    let mut list = back(&reply);
    list[0]["moveBy"] = json!([100, 300]);
    let reply = set(&mut engine, &mut session, json!(list));
    let q = &reply["questions"];
    assert_eq!(q[0]["pinned"], true);
    let moved = rect(&q[0]);
    assert_eq!(
        [moved[0], moved[1]],
        [name_rect[0] + 100.0, name_rect[1] + 300.0]
    );
    let new_box = boxed(&q[0]["box"]);
    assert!(
        (new_box[1] - name_box[1] - 300.0).abs() < 1.0,
        "{new_box:?}"
    );
    // The section heading.
    let heading = boxed(&q[1]["box"]);
    let mut list = back(&reply);
    list[1]["moveBy"] = json!([30, 400]);
    let reply = set(&mut engine, &mut session, json!(list));
    let q = &reply["questions"];
    assert_eq!(q[1]["pinned"], true);
    let at = boxed(&q[1]["box"]);
    assert!(
        (at[0] - heading[0] - 30.0).abs() < 1.0 && (at[1] - heading[1] - 400.0).abs() < 1.0,
        "{heading:?} {at:?}"
    );
    // Laid out again (from outside), it stays.
    let age = q[2]["widgetIds"][0].as_i64().unwrap();
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-props", "page": 0, "widgetId": age, "required": true}),
    );
    let listed = form(&mut engine, &mut session);
    assert_eq!(boxed(&listed["questions"][1]["box"]), at);
    // The title, then back at the top.
    let title = boxed(&listed["titleBox"]);
    let reply = ok(
        &mut engine,
        &mut session,
        json!({"op": "form-set", "titleMoveBy": [50, 500], "questions": back(&listed)}),
    );
    assert_eq!(reply["titleMoved"], true);
    let moved = boxed(&reply["titleBox"]);
    assert!(
        (moved[1] - title[1] - 500.0).abs() < 1.0,
        "{title:?} {moved:?}"
    );
    let reply = ok(
        &mut engine,
        &mut session,
        json!({"op": "form-set", "titleBack": true, "questions": back(&reply)}),
    );
    assert_eq!(reply["titleMoved"], false);
    // A section put back in line.
    let mut list = back(&reply);
    list[1]["pinned"] = json!(false);
    let reply = set(&mut engine, &mut session, json!(list));
    assert_eq!(reply["questions"][1]["pinned"], false);
    // A move never leaves the page.
    let mut list = back(&reply);
    list[2]["moveBy"] = json!([5000, 5000]);
    let reply = set(&mut engine, &mut session, json!(list));
    let r = rect(&reply["questions"][2]);
    assert!(r[2] <= 612.0 && r[3] <= 792.0, "{r:?}");
}

// Choice buttons added outside the question list (the designer, the Form tool) show their
// words on a blank page; a page made elsewhere is left alone.
#[test]
fn choice_buttons_added_by_hand_show_their_words() {
    let (mut engine, mut session) = session_with(blank());
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "radio", "name": "Test", "kids": [
            {"on": "Apple", "rect": [72, 72, 88, 88]},
            {"on": "Yam", "rect": [72, 96, 88, 112]},
            {"on": "Kiwi", "rect": [72, 120, 88, 136]},
        ]}),
    );
    let page = &text(&mut engine, &mut session)[0];
    for word in ["Apple", "Yam", "Kiwi"] {
        assert!(page.contains(word), "{word}: {page}");
    }
    assert_eq!(form(&mut engine, &mut session)["formPages"], json!([0]));
    // A question added next flows clear of them.
    let reply = set(
        &mut engine,
        &mut session,
        json!([{"text": "Name", "type": "short"}]),
    );
    let r = rect(&reply["questions"][0]);
    assert!(r[1] > 136.0 || r[3] < 72.0, "{r:?}");
    // One undo takes the buttons and their words away together.
    ok(&mut engine, &mut session, json!({"op": "undo"}));
    ok(&mut engine, &mut session, json!({"op": "undo"}));
    assert!(!text(&mut engine, &mut session)[0].contains("Apple"));

    let (mut engine, mut session) = session_with(letter());
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "radio", "name": "Test", "kids": [
            {"on": "Apple", "rect": [72, 300, 88, 316]},
            {"on": "Yam", "rect": [72, 330, 88, 346]},
        ]}),
    );
    assert!(!text(&mut engine, &mut session)[0].contains("Apple"));
    assert_eq!(form(&mut engine, &mut session)["formPages"], json!([]));
}

fn field_by_name(engine: &mut AppEngine, session: &mut AppSession, name: &str) -> Value {
    fields(engine, session, 0)
        .into_iter()
        .find(|f| f["name"] == name)
        .unwrap_or_else(|| panic!("no field {name}"))
}

// "Starts with" / "Starts checked" changed on an existing field shows on the field while it
// still holds its old default (or nothing); a value someone typed is kept.
#[test]
fn a_changed_default_shows_on_the_field() {
    let (mut engine, mut session) = session_with(blank());
    let add = |engine: &mut AppEngine, session: &mut AppSession, request: Value| {
        ok(engine, session, request)["widgetId"].as_i64().unwrap()
    };
    let text = add(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "text", "name": "T", "rect": [72, 400, 272, 422]}),
    );
    let check = add(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "checkbox", "name": "C", "rect": [72, 440, 88, 456]}),
    );
    let combo = add(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "combo", "name": "D", "rect": [72, 480, 272, 502],
            "options": ["One", "Two"]}),
    );
    let radio = ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "radio", "name": "R", "kids": [
            {"on": "Left", "rect": [72, 520, 88, 536]}, {"on": "Right", "rect": [100, 520, 116, 536]}]}),
    );
    let radio = radio["widgetId"].as_i64().unwrap();
    for (id, default) in [
        (text, json!("Hello")),
        (check, json!(true)),
        (combo, json!("Two")),
        (radio, json!("Right")),
    ] {
        ok(
            &mut engine,
            &mut session,
            json!({"op": "field-props", "page": 0, "widgetId": id, "default": default}),
        );
    }
    assert_eq!(
        field_by_name(&mut engine, &mut session, "T")["value"],
        "Hello"
    );
    assert_eq!(field_by_name(&mut engine, &mut session, "C")["value"], true);
    assert_eq!(
        field_by_name(&mut engine, &mut session, "D")["value"],
        "Two"
    );
    let right = fields(&mut engine, &mut session, 0)
        .into_iter()
        .find(|f| f["name"] == "R" && f["on"] == "Right")
        .unwrap();
    assert_eq!(right["value"], true, "{right}");
    // Typed over, the value stays when the default changes again.
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-set", "page": 0, "widgetId": text, "value": "Mine"}),
    );
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-props", "page": 0, "widgetId": text, "default": "Other"}),
    );
    assert_eq!(
        field_by_name(&mut engine, &mut session, "T")["value"],
        "Mine"
    );
    // "Most characters" can come down with a shorter default in the same change.
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-set", "page": 0, "widgetId": text, "value": ""}),
    );
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-props", "page": 0, "widgetId": text, "maxLen": 3, "default": "Hi"}),
    );
    let refused = op(
        &mut engine,
        &mut session,
        json!({"op": "field-props", "page": 0, "widgetId": text, "maxLen": 1}),
    );
    assert_eq!(refused["ok"], false);
    assert!(refused.to_string().contains("max-len"), "{refused}");
}

// A choice button deleted from a group whose /Kids is an indirect array (other tools write
// them so) leaves the others in the group.
#[test]
fn deleting_one_button_of_an_indirect_group_keeps_the_rest() {
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R] >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [7 0 R 8 0 R 9 0 R] >>".into(),
        "<< /Length 0 >>\nstream\n\nendstream".into(),
        "<< /FT /Btn /Ff 49152 /T (Pick) /V /Off /Kids 6 0 R >>".into(),
        "[7 0 R 8 0 R 9 0 R]".into(),
        "<< /Type /Annot /Subtype /Widget /Parent 5 0 R /Rect [72 700 88 716] /P 3 0 R /AS /Off /AP << /N << /A 10 0 R /Off 10 0 R >> >> >>".into(),
        "<< /Type /Annot /Subtype /Widget /Parent 5 0 R /Rect [72 670 88 686] /P 3 0 R /AS /Off /AP << /N << /B 10 0 R /Off 10 0 R >> >> >>".into(),
        "<< /Type /Annot /Subtype /Widget /Parent 5 0 R /Rect [72 640 88 656] /P 3 0 R /AS /Off /AP << /N << /C 10 0 R /Off 10 0 R >> >> >>".into(),
        "<< /Type /XObject /Subtype /Form /BBox [0 0 16 16] /Length 0 >>\nstream\n\nendstream".into(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let before = fields(&mut engine, &mut session, 0);
    assert_eq!(before.len(), 3, "{before:?}");
    let first = before[0]["id"].as_i64().unwrap();
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-delete", "page": 0, "widgetId": first}),
    );
    let after = fields(&mut engine, &mut session, 0);
    assert_eq!(after.len(), 2, "{after:?}");
    assert!(after.iter().all(|f| f["name"] == "Pick"));
}

// Rating stars: grey outlines on the page, buttons without a border or background.
#[test]
fn rating_stars_are_drawn_and_buttons_see_through() {
    let (mut engine, mut session) = session_with(blank());
    let reply = set(
        &mut engine,
        &mut session,
        json!([{"text": "Stars", "type": "rating"}]),
    );
    let id = reply["questions"][0]["widgetIds"][0].as_i64().unwrap();
    let doc = session.doc.as_ref().unwrap().document();
    let widget = doc
        .resolve_id(tandem_pdf_engine::object::ObjectId::new(id as u32, 0).unwrap())
        .as_dict()
        .unwrap();
    let mk = doc.dict_get(widget, b"MK").as_dict().unwrap();
    assert!(doc.dict_get(mk, b"BG").as_array().is_none());
    assert!(
        doc.dict_get(widget, b"AP").as_dict().is_some(),
        "appearances drawn again"
    );
}
