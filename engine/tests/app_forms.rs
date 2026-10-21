// SPDX-License-Identifier: MIT
//! Form fields on the app session: the listing reads every kind of field,
//! fills, adds, changes and deletes write the dictionaries the earlier app engine writes
//! and undo exactly, synthesized appearances meet spec 07 (structure, fonts,
//! and the text the engine's own view extracts from them), and the view
//! draws widgets without appearances.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use tandem_pdf_engine::app::forms::appearance::Context;
use tandem_pdf_engine::app::forms::scripts::Eight;
use tandem_pdf_engine::app::forms::text::Family;
use tandem_pdf_engine::app::forms::write::{Sink, appearance_dict};
use tandem_pdf_engine::app::forms::{self, Kind, PageSpace};
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::content::{ContentLimits, ContentOperand, ContentParser, Operation};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Dictionary, Object, ObjectId, decode_text_string};
use tandem_pdf_engine::serialize::{WriteLimits, serialize_object};
use tandem_pdf_engine::view::{RenderOptions, ViewDocument};
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

fn stream(dictionary: &str, data: &str) -> String {
    format!(
        "<< {dictionary} /Length {} >>\nstream\n{data}\nendstream",
        data.len()
    )
}

const FONT_HELV: &str =
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";

/// One page with `widgets` (objects from 6, in /Annots and /Fields order),
/// /AcroForm 5 with /DR /Helv (the object after the widgets) and `acro`,
/// then `extra` objects.
fn form_with(widgets: &[&str], acro: &str, extra: &[String]) -> Vec<u8> {
    let n = widgets.len();
    let refs: Vec<String> = (0..n).map(|i| format!("{} 0 R", 6 + i)).collect();
    let refs = refs.join(" ");
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [{refs}] >>"
        ),
        stream("", "q Q"),
        format!(
            "<< /Fields [{refs}] /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv {} 0 R >> >> {acro} >>",
            6 + n
        ),
    ];
    objects.extend(
        widgets
            .iter()
            .map(|w| format!("<< /Type /Annot /Subtype /Widget /F 4 /P 3 0 R {w} >>")),
    );
    objects.push(FONT_HELV.into());
    objects.extend(extra.iter().cloned());
    classic(&objects)
}

/// p2-forms' rich form: name (MaxLen 20), notes (multiline), agree
/// (checkbox), size (radio S M L), choice (combo), pick (list), fixed
/// (read-only), amount (/AA JavaScript, in /CO), sig.
fn rich_form() -> Vec<u8> {
    let ap = |w: u32| {
        stream(
            &format!("/Type /XObject /Subtype /Form /BBox [0 0 {w} {w}]"),
            "q Q",
        )
    };
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 6 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 7 0 R \
         /Annots [8 0 R 9 0 R 10 0 R 12 0 R 13 0 R 14 0 R 15 0 R 16 0 R 17 0 R 18 0 R 19 0 R] >>"
            .into(),
        FONT_HELV.into(),
        FONT_HELV.into(),
        "<< /Fields [8 0 R 9 0 R 10 0 R 11 0 R 15 0 R 16 0 R 17 0 R 18 0 R 19 0 R] \
         /DR << /Font << /Helv 5 0 R >> >> /DA (/Helv 12 Tf 0 g) /CO [18 0 R] >>"
            .into(),
        stream("", "q Q"),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V () /DV () /MaxLen 20 \
         /Rect [120 700 340 716] /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) >>"
            .into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /Ff 4096 /T (notes) /V () \
         /Rect [120 658 340 690] /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) >>"
            .into(),
        "<< /Type /Annot /Subtype /Widget /FT /Btn /T (agree) /V /Off /Rect [120 616 132 628] \
         /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) /AP << /N << /Yes 20 0 R /Off 21 0 R >> >> >>"
            .into(),
        "<< /FT /Btn /Ff 32768 /T (size) /Kids [12 0 R 13 0 R 14 0 R] /V /Off >>".into(),
        "<< /Type /Annot /Subtype /Widget /Parent 11 0 R /AS /Off /Rect [120 580 132 592] \
         /P 3 0 R /AP << /N << /S 20 0 R /Off 21 0 R >> >> /DA (/Helv 12 Tf 0 g) >>"
            .into(),
        "<< /Type /Annot /Subtype /Widget /Parent 11 0 R /AS /Off /Rect [160 580 172 592] \
         /P 3 0 R /AP << /N << /M 20 0 R /Off 21 0 R >> >> /DA (/Helv 12 Tf 0 g) >>"
            .into(),
        "<< /Type /Annot /Subtype /Widget /Parent 11 0 R /AS /Off /Rect [200 580 212 592] \
         /P 3 0 R /AP << /N << /L 20 0 R /Off 21 0 R >> >> /DA (/Helv 12 Tf 0 g) >>"
            .into(),
        "<< /Type /Annot /Subtype /Widget /FT /Ch /Ff 131072 /T (choice) /V () \
         /Opt [(one) (two) (three)] /Rect [120 538 340 554] /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) >>"
            .into(),
        "<< /Type /Annot /Subtype /Widget /FT /Ch /T (pick) /V () /Opt [(alpha) (beta) (gamma)] \
         /Rect [120 466 340 526] /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) >>"
            .into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /Ff 1 /T (fixed) /V (locked) \
         /Rect [120 424 340 440] /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) >>"
            .into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (amount) /V () /Rect [120 382 340 398] \
         /F 4 /P 3 0 R /DA (/Helv 12 Tf 0 g) /AA << /F << /S /JavaScript /JS (1) >> >> >>"
            .into(),
        "<< /Type /Annot /Subtype /Widget /FT /Sig /T (sig) /Rect [120 340 340 356] /F 4 /P 3 0 R >>"
            .into(),
        ap(12),
        ap(12),
    ];
    classic(&objects)
}

fn hello() -> Vec<u8> {
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .into(),
        stream("", "BT /F1 24 Tf 72 700 Td (Hello) Tj ET"),
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

fn items(engine: &mut AppEngine, session: &mut AppSession, page: usize) -> Vec<Value> {
    let reply = ok(engine, session, json!({"op": "fields", "page": page}));
    reply["items"].as_array().unwrap().clone()
}

fn item(engine: &mut AppEngine, session: &mut AppSession, name: &str) -> Value {
    items(engine, session, 0)
        .into_iter()
        .find(|item| item["name"] == name)
        .unwrap_or_else(|| panic!("no field {name}"))
}

fn doc(session: &AppSession) -> &Document<'static> {
    session.doc.as_ref().unwrap().document()
}

fn dict(session: &AppSession, n: u32) -> Dictionary {
    doc(session).resolve_id(id(n)).as_dict().unwrap().clone()
}

/// A deterministic full rewrite of the current state, every object kept.
fn serialized(session: &AppSession) -> Vec<u8> {
    let options = WriteOptions {
        garbage: false,
        ..WriteOptions::APP
    };
    write_document(doc(session), DocumentWriteLimits::default(), options).unwrap()
}

/// The app's saved bytes.
fn saved(session: &AppSession) -> Vec<u8> {
    write_document(
        doc(session),
        DocumentWriteLimits::default(),
        WriteOptions::APP,
    )
    .unwrap()
}

fn text(value: &Object) -> String {
    decode_text_string(value.as_string().unwrap())
}

/// Non-white pixels of page 0 inside a page-space rect at 1 px per pt.
fn ink(session: &mut AppSession, rect: [u32; 4]) -> usize {
    let view = session.view().unwrap();
    ink_of(view, rect)
}

fn ink_of(view: &ViewDocument, rect: [u32; 4]) -> usize {
    let image = view.render(0, 1.0, RenderOptions::default()).unwrap();
    let mut dark = 0;
    for y in rect[1]..rect[3] {
        for x in rect[0]..rect[2] {
            let at = ((y * image.width + x) * 4) as usize;
            let rgb = &image.pixels[at..at + 3];
            if rgb.iter().map(|&c| u32::from(c)).sum::<u32>() < 600 {
                dark += 1;
            }
        }
    }
    dark
}

// ---------------------------------------------------------------- listing

#[test]
fn the_listing_reads_every_kind() {
    let (mut engine, mut session) = session_with(rich_form());
    let listed = items(&mut engine, &mut session, 0);
    let names: Vec<String> = listed
        .iter()
        .map(|i| {
            format!(
                "{}:{}",
                i["name"].as_str().unwrap(),
                i["kind"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(
        names,
        [
            "name:text",
            "notes:text",
            "agree:checkbox",
            "size:radio",
            "size:radio",
            "size:radio",
            "choice:combo",
            "pick:list",
            "fixed:text",
            "amount:text",
            "sig:signature",
        ]
    );
    let by = |name: &str| listed.iter().find(|i| i["name"] == name).unwrap().clone();
    assert_eq!(
        by("name"),
        json!({"id": 8, "name": "name", "label": "name", "kind": "text", "value": "",
            "readOnly": false, "required": false, "scripted": false, "rect": [120, 76, 340, 92],
            "default": "", "maxLen": 20})
    );
    assert_eq!(by("notes")["multiline"], true);
    assert_eq!(
        (by("agree")["on"].clone(), by("agree")["value"].clone()),
        (json!("Yes"), json!(false))
    );
    let ons: Vec<&Value> = listed
        .iter()
        .filter(|i| i["kind"] == "radio")
        .map(|i| &i["on"])
        .collect();
    assert_eq!(ons, [&json!("S"), &json!("M"), &json!("L")]);
    assert_eq!(by("choice")["options"], json!(["one", "two", "three"]));
    assert_eq!(by("choice")["edit"], false);
    assert_eq!(by("pick")["options"], json!(["alpha", "beta", "gamma"]));
    assert_eq!(
        (
            by("fixed")["readOnly"].clone(),
            by("fixed")["value"].clone()
        ),
        (json!(true), json!("locked"))
    );
    let scripted: Vec<&Value> = listed
        .iter()
        .filter(|i| i["scripted"] == true)
        .map(|i| &i["name"])
        .collect();
    assert_eq!(scripted, [&json!("amount")]);
    assert_eq!(by("sig")["signed"], false);
}

#[test]
fn inherited_attributes_hierarchies_and_radio_groups_across_pages() {
    // A text field whose kids inherit /FT, /Ff, /V and /DA; a radio group with
    // a kid on each page; a page with /Tabs /S and a structure tree.
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R /StructTreeRoot 15 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [7 0 R 8 0 R 10 0 R] /Tabs /S >>"
            .into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Rotate 90 /Annots [11 0 R] >>".into(),
        "<< /Fields [6 0 R 9 0 R] /DA (/Helv 12 Tf 0 g) >>".into(),
        "<< /T (outer) /FT /Tx /Ff 4099 /V (shared) /TU (Outer label) /Kids [7 0 R 8 0 R] >>".into(),
        "<< /Type /Annot /Subtype /Widget /Parent 6 0 R /T (a) /Rect [72 700 172 720] /P 3 0 R >>".into(),
        "<< /Type /Annot /Subtype /Widget /Parent 6 0 R /Rect [72 650 172 670] /P 3 0 R >>".into(),
        "<< /FT /Btn /Ff 49152 /T (r) /V /B /DV /A /Kids [10 0 R 11 0 R] >>".into(),
        "<< /Type /Annot /Subtype /Widget /Parent 9 0 R /Rect [72 600 88 616] /AS /Off \
         /AP << /N << /A 12 0 R /Off 12 0 R >> >> >>"
            .into(),
        "<< /Type /Annot /Subtype /Widget /Parent 9 0 R /Rect [72 600 88 616] /AS /B \
         /AP << /N << /Off 12 0 R /B 12 0 R >> >> >>"
            .into(),
        stream("/Type /XObject /Subtype /Form /BBox [0 0 16 16]", "q Q"),
        "null".into(),
        "null".into(),
        "<< /Type /StructTreeRoot /K [<< /K [<< /Type /OBJR /Obj 10 0 R >> << /Type /OBJR /Obj 7 0 R >>] >>] >>"
            .into(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    let reply = ok(
        &mut engine,
        &mut session,
        json!({"op": "fields", "page": 0}),
    );
    assert_eq!(reply["tabs"], "S");
    let listed = reply["items"].as_array().unwrap();
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0]["name"], "outer.a");
    assert_eq!(listed[0]["label"], "Outer label");
    assert_eq!(listed[0]["value"], "shared");
    assert_eq!(listed[0]["kind"], "text");
    assert_eq!(
        (listed[0]["readOnly"].clone(), listed[0]["required"].clone()),
        (json!(true), json!(true))
    );
    assert_eq!(listed[0]["multiline"], true);
    assert_eq!(listed[0]["structIndex"], 1);
    assert_eq!(listed[1]["name"], "outer");
    assert!(listed[1].get("structIndex").is_none());
    assert_eq!(listed[2]["name"], "r");
    assert_eq!(listed[2]["on"], "A");
    assert_eq!(
        listed[2]["value"], false,
        "its own /AS wins over the group's /V"
    );
    assert_eq!(listed[2]["default"], "A");
    assert_eq!(listed[2]["structIndex"], 0);
    // The other page lists the group's other kid, rotated into page space.
    let other = items(&mut engine, &mut session, 1);
    assert_eq!(other.len(), 1);
    assert_eq!(
        (other[0]["name"].clone(), other[0]["value"].clone()),
        (json!("r"), json!(true))
    );
    assert_eq!(other[0]["rect"], json!([600, 72, 616, 88]));
    // Setting the first kid switches the kid on the other page off.
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-set", "page": 0, "widgetId": 10, "value": true}),
    );
    assert_eq!(
        dict(&session, 9).get(b"V".as_slice()),
        Some(&Object::name("A"))
    );
    assert_eq!(
        dict(&session, 10).get(b"AS".as_slice()),
        Some(&Object::name("A"))
    );
    assert_eq!(
        dict(&session, 11).get(b"AS".as_slice()),
        Some(&Object::name("Off"))
    );
    assert_eq!(items(&mut engine, &mut session, 1)[0]["value"], false);
}

#[test]
fn values_read_names_strings_rich_text_streams_and_nothing_else() {
    let bytes = form_with(
        &[
            "/FT /Tx /T (rich) /V 11 0 R /Rect [72 700 252 724]",
            "/FT /Tx /T (named) /V /Hello /Rect [72 650 252 674]",
            "/FT /Sig /T (signed) /V 12 0 R /Rect [72 600 252 640]",
            "/FT /Ch /Ff 2097152 /T (many) /V [(a) (b)] /Opt [(a) (b)] /Rect [300 600 400 680]",
        ],
        "",
        &[
            stream("", "Rich value"),
            "<< /Type /Sig /Filter /Adobe.PPKLite >>".into(),
        ],
    );
    let (mut engine, mut session) = session_with(bytes);
    let values: Vec<Value> = items(&mut engine, &mut session, 0)
        .iter()
        .map(|i| i["value"].clone())
        .collect();
    assert_eq!(
        values,
        [json!("Rich value"), json!("Hello"), json!(""), json!("")]
    );
    assert_eq!(item(&mut engine, &mut session, "signed")["signed"], true);
}

// ---------------------------------------------------------------- fill

#[test]
fn fills_write_values_appearances_and_undo_exactly() {
    let (mut engine, mut session) = session_with(rich_form());
    let original = serialized(&session);
    let fills = [
        (8, json!("Typed text")),
        (9, json!("line one\nline two")),
        (10, json!(true)),
        (13, json!(true)),
        (15, json!("two")),
        (16, json!("beta")),
    ];
    let mut states = vec![original.clone()];
    for (n, (widget, value)) in fills.iter().enumerate() {
        let reply = ok(
            &mut engine,
            &mut session,
            json!({"op": "field-set", "page": 0, "widgetId": widget, "value": value}),
        );
        assert_eq!(reply["revision"], n as u64 + 1);
        states.push(serialized(&session));
    }
    let steps = session.doc.as_ref().unwrap().steps();
    assert!(steps.iter().all(|s| s == "fill field") && steps.len() == 6);
    let name = dict(&session, 8);
    assert_eq!(text(name.get(b"V".as_slice()).unwrap()), "Typed text");
    let ap = doc(&session)
        .dict_get(&name, b"AP")
        .as_dict()
        .unwrap()
        .clone();
    assert!(ap.get(b"N".as_slice()).unwrap().as_reference().is_some());
    // The saved appearance: ISO operators, /Tx marked content, the DA font
    // Helv at 12, and the value drawn inside the widget.
    let bytes = serialized(&session);
    let stored = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let normal = stored_normal(&stored, id(8)).unwrap();
    let ops = normal.ops();
    assert_eq!(structure_errors(&ops), Vec::<String>::new());
    assert!(tx_marked(&ops));
    assert!(ops.iter().any(|op| {
        op.operator == b"Tf"
            && op.operands.first() == Some(&ContentOperand::Name(b"Helv".to_vec()))
            && op.operands.get(1).and_then(number) == Some(12.0)
    }));
    let drawn = normal.draw();
    assert_eq!(drawn.text.trim(), "Typed text");
    assert_eq!(drawn.ink_outside(normal.rect), 0);
    for ch in &drawn.chars {
        let [x, y] = ch.origin;
        assert!(
            (120.0..=340.0).contains(&x) && (700.0..=716.0).contains(&y),
            "{}",
            ch.c
        );
    }
    assert_eq!(
        dict(&session, 10).get(b"AS".as_slice()),
        Some(&Object::name("Yes"))
    );
    assert_eq!(
        dict(&session, 11).get(b"V".as_slice()),
        Some(&Object::name("M"))
    );
    let values: Vec<Value> = items(&mut engine, &mut session, 0)
        .iter()
        .map(|i| i["value"].clone())
        .collect();
    assert_eq!(
        values,
        [
            json!("Typed text"),
            json!("line one\nline two"),
            json!(true),
            json!(false),
            json!(true),
            json!(false),
            json!("two"),
            json!("beta"),
            json!("locked"),
            json!(""),
            json!(""),
        ]
    );
    for state in states.iter().rev().skip(1) {
        ok(&mut engine, &mut session, json!({"op": "undo"}));
        assert_eq!(&serialized(&session), state);
    }
    for state in states.iter().skip(1) {
        ok(&mut engine, &mut session, json!({"op": "redo"}));
        assert_eq!(&serialized(&session), state);
    }
}

#[test]
fn fill_refusals_change_nothing() {
    let (mut engine, mut session) = session_with(rich_form());
    let before = serialized(&session);
    let cases = [
        (
            8,
            json!("x".repeat(21)),
            "this field holds at most 20 characters",
        ),
        (
            8,
            json!("x".repeat(4097)),
            "a value holds at most 4096 characters",
        ),
        (
            15,
            json!("bogus"),
            "\"bogus\" is not one of this field's options",
        ),
        (16, json!("delta"), "is not one of this field's options"),
        (17, json!("nope"), "that field is read-only"),
        (19, json!("nope"), "click the signature field to sign it"),
        (
            12,
            json!(false),
            "a radio cannot be turned off, only switched",
        ),
        (999_999, json!("nope"), "no form field 999999 on this page"),
        (8, json!(true), "a text takes a string"),
        (10, json!("yes"), "a checkbox takes true or false"),
    ];
    for (widget, value, message) in cases {
        let reply = op(
            &mut engine,
            &mut session,
            json!({"op": "field-set", "page": 0, "widgetId": widget, "value": value}),
        );
        assert_eq!(reply["ok"], false);
        assert!(
            reply["error"].as_str().unwrap().contains(message),
            "{reply}"
        );
    }
    let bad_id = op(
        &mut engine,
        &mut session,
        json!({"op": "field-set", "page": 0, "widgetId": 1.5, "value": "x"}),
    );
    assert_eq!(
        (bad_id["reason"].clone(), bad_id["detail"].clone()),
        (json!("invalid"), json!("invalid"))
    );
    // The same state again is no step and no revision.
    let same = ok(
        &mut engine,
        &mut session,
        json!({"op": "field-set", "page": 0, "widgetId": 10, "value": false}),
    );
    assert_eq!(same["revision"], 0);
    assert_eq!(serialized(&session), before);
    assert!(!session.journal_state().can_undo);
}

// ---------------------------------------------------------------- add

#[test]
fn add_writes_field_dictionaries_and_one_undo_step_each() {
    let (mut engine, mut session) = session_with(hello());
    let original = serialized(&session);
    let text = ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "text",
        "name": "t1", "rect": [72, 72, 252, 96], "maxLen": 10, "label": "Name", "default": "Ann"}),
    );
    assert_eq!(
        (text["widgetId"].clone(), text["name"].clone()),
        (json!(8), json!("t1"))
    );
    // /AcroForm 7 with /Fields, /DA and /DR /Font /Helv 6.
    let acro = dict(&session, 7);
    assert_eq!(
        acro.get(b"Fields".as_slice()),
        Some(&Object::array([Object::Reference(id(8))]))
    );
    assert_eq!(
        acro.get(b"DA".as_slice()),
        Some(&Object::text("/Helv 12 Tf 0 g"))
    );
    assert_eq!(
        dict(&session, 1).get(b"AcroForm".as_slice()),
        Some(&Object::Reference(id(7)))
    );
    assert_eq!(
        dict(&session, 6).get(b"BaseFont".as_slice()),
        Some(&Object::name("Helvetica"))
    );
    let widget = dict(&session, 8);
    let keys: Vec<&[u8]> = widget.keys().map(Vec::as_slice).collect();
    assert_eq!(
        keys,
        [
            b"Type".as_slice(),
            b"Subtype",
            b"FT",
            b"T",
            b"Rect",
            b"F",
            b"P",
            b"DA",
            b"MK",
            b"TU",
            b"DV",
            b"V",
            b"MaxLen",
            b"AP",
        ]
    );
    assert_eq!(
        widget.get(b"Rect".as_slice()),
        Some(&Object::array([696, 252, 720].iter().fold(
            vec![Object::Integer(72)],
            |mut v, n| {
                v.push(Object::Integer(*n));
                v
            }
        )))
    );
    assert_eq!(
        dict(&session, 3).get(b"Annots".as_slice()),
        Some(&Object::array([Object::Reference(id(8))]))
    );
    let checkbox = ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "checkbox",
        "name": "c1", "rect": [72, 110, 88, 126], "default": true}),
    );
    let c = dict(&session, checkbox["widgetId"].as_u64().unwrap() as u32);
    assert_eq!(c.get(b"AS".as_slice()), Some(&Object::name("Yes")));
    assert_eq!(c.get(b"V".as_slice()), Some(&Object::name("Yes")));
    let states = doc(&session).dict_get(doc(&session).dict_get(&c, b"AP").as_dict().unwrap(), b"N");
    let states: Vec<&[u8]> = states
        .as_dict()
        .unwrap()
        .keys()
        .map(Vec::as_slice)
        .collect();
    assert_eq!(states, [b"Off".as_slice(), b"Yes"]);
    let radio = ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "radio", "name": "g",
        "kids": [{"on": "A", "rect": [72, 180, 88, 196]}, {"on": "B", "rect": [110, 180, 126, 196]}], "default": "B"}),
    );
    let ids: Vec<u32> = radio["widgetIds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as u32)
        .collect();
    assert_eq!(radio["widgetId"], ids[0]);
    let kid = dict(&session, ids[1]);
    assert_eq!(kid.get(b"AS".as_slice()), Some(&Object::name("B")));
    assert!(kid.get(b"T".as_slice()).is_none() && kid.get(b"FT".as_slice()).is_none());
    let parent_id = kid
        .get(b"Parent".as_slice())
        .unwrap()
        .as_reference()
        .unwrap();
    let parent = dict(&session, parent_id.number());
    assert_eq!(parent.get(b"Ff".as_slice()), Some(&Object::Integer(32768)));
    assert_eq!(parent.get(b"V".as_slice()), Some(&Object::name("B")));
    assert_eq!(parent.get(b"DV".as_slice()), Some(&Object::name("B")));
    let signature = ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "signature",
        "rect": [300, 300, 400, 340]}),
    );
    assert_eq!(signature["name"], "Signature1");
    let sig = dict(&session, signature["widgetId"].as_u64().unwrap() as u32);
    assert!(sig.get(b"AP".as_slice()).is_none() && sig.get(b"V".as_slice()).is_none());
    let steps = session.doc.as_ref().unwrap().steps();
    assert_eq!(steps, ["add field"; 4]);
    for _ in 0..4 {
        ok(&mut engine, &mut session, json!({"op": "undo"}));
    }
    assert_eq!(serialized(&session), original);
    // The listing gives back the requested rects.
    for _ in 0..4 {
        ok(&mut engine, &mut session, json!({"op": "redo"}));
    }
    let rects: Vec<Value> = items(&mut engine, &mut session, 0)
        .iter()
        .map(|i| i["rect"].clone())
        .collect();
    assert_eq!(
        rects,
        [
            json!([72, 72, 252, 96]),
            json!([72, 110, 88, 126]),
            json!([72, 180, 88, 196]),
            json!([110, 180, 126, 196]),
            json!([300, 300, 400, 340])
        ]
    );
}

#[test]
fn a_shared_annots_array_is_copied_and_the_rect_follows_rotation_and_crop() {
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /CropBox [50 60 562 732] /Rotate 90 /Annots 5 0 R >>"
            .into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots 5 0 R >>".into(),
        "[]".into(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    let rect = [72.0, 72.0, 152.0, 92.0];
    let added = ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "text",
        "name": "n1", "rect": rect}),
    );
    let n = added["widgetId"].as_u64().unwrap() as u32;
    let shared = doc(&session)
        .resolve_id(id(5))
        .as_array()
        .map(<[Object]>::len);
    assert_eq!(shared, Some(0), "the shared array is untouched");
    assert_eq!(
        doc(&session)
            .dict_get(&dict(&session, 4), b"Annots")
            .as_array()
            .map(<[Object]>::len),
        Some(0)
    );
    assert_eq!(
        items(&mut engine, &mut session, 0)[0]["rect"],
        json!([72, 72, 152, 92])
    );
    let page = dict(&session, 3);
    let space = PageSpace::of(doc(&session), &page);
    let pdf = forms::pdf_rect(doc(&session), &dict(&session, n));
    assert_eq!(space.to_page(pdf), rect);
    let mk = doc(&session)
        .dict_get(&dict(&session, n), b"MK")
        .as_dict()
        .unwrap()
        .clone();
    assert_eq!(mk.get(b"R".as_slice()), Some(&Object::Integer(90)));
}

#[test]
fn add_refusals_change_nothing() {
    let (mut engine, mut session) = session_with(hello());
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "text", "name": "dup", "rect": [72, 72, 252, 96]}),
    );
    let before = serialized(&session);
    let cases = [
        (json!({"kind": "text", "name": "dup"}), "duplicate-name"),
        (json!({"kind": "text", "name": "a.b"}), "invalid-name"),
        (json!({"kind": "text", "name": " lead"}), "invalid-name"),
        (
            json!({"kind": "text", "name": "trail\u{feff}"}),
            "invalid-name",
        ),
        (
            json!({"kind": "text", "name": "x".repeat(129)}),
            "invalid-name",
        ),
        (json!({"kind": "text"}), "invalid-name"),
        (
            json!({"kind": "text", "name": "s", "rect": [72, 72, 75, 96]}),
            "invalid",
        ),
        (
            json!({"kind": "text", "name": "s", "rect": [72, 72, 900, 96]}),
            "invalid",
        ),
        (
            json!({"kind": "text", "name": "s", "rect": [72, 72, "1", 96]}),
            "invalid",
        ),
        (
            json!({"kind": "signature", "name": "s", "rect": [72, 72, 100, 80]}),
            "invalid",
        ),
        (json!({"kind": "signature", "name": "s"}), "invalid"),
        (
            json!({"kind": "radio", "name": "s", "kids": [{"on": "A", "rect": [72, 72, 90, 90]}]}),
            "invalid",
        ),
        (
            json!({"kind": "radio", "name": "s", "kids": [{"on": "Off", "rect": [72, 72, 90, 90]}, {"on": "B", "rect": [72, 72, 90, 90]}]}),
            "invalid",
        ),
        (
            json!({"kind": "radio", "name": "s", "kids": [{"on": "A", "rect": [72, 72, 90, 90]}, {"on": "A", "rect": [72, 72, 90, 90]}]}),
            "invalid",
        ),
        (
            json!({"kind": "combo", "name": "s", "options": []}),
            "invalid",
        ),
        (
            json!({"kind": "combo", "name": "s", "options": ["a", "a"]}),
            "invalid",
        ),
        (
            json!({"kind": "text", "name": "s", "maxLen": 4097}),
            "invalid",
        ),
        (
            json!({"kind": "text", "name": "s", "maxLen": 3, "default": "abcd"}),
            "invalid",
        ),
        (
            json!({"kind": "checkbox", "name": "s", "multiline": true}),
            "invalid",
        ),
        (
            json!({"kind": "text", "name": "s", "edit": true}),
            "invalid",
        ),
        (
            json!({"kind": "text", "name": "s", "label": "a\u{7}b"}),
            "invalid",
        ),
        (
            json!({"kind": "text", "name": "s", "required": 1}),
            "invalid",
        ),
        (json!({"kind": "bogus", "name": "s"}), "invalid"),
    ];
    for (request, detail) in cases {
        let mut request = request;
        request["op"] = json!("field-add");
        request["page"] = json!(0);
        let reply = op(&mut engine, &mut session, request.clone());
        assert_eq!(reply["ok"], false, "{request}");
        assert_eq!(reply["detail"], detail, "{request} -> {reply}");
    }
    assert_eq!(serialized(&session), before);
}

// ---------------------------------------------------------------- props and delete

#[test]
fn props_change_what_engine_ts_changes() {
    let (mut engine, mut session) = session_with(hello());
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "text", "name": "t", "rect": [72, 72, 252, 96]}),
    );
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "combo", "name": "d",
        "options": ["one", "two"], "rect": [72, 120, 252, 144], "default": "two"}),
    );
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "radio", "name": "g",
        "kids": [{"on": "A", "rect": [72, 180, 88, 196]}, {"on": "B", "rect": [110, 180, 126, 196]}]}),
    );
    let t = item(&mut engine, &mut session, "t")["id"].clone();
    let d = item(&mut engine, &mut session, "d")["id"].clone();
    let kid = items(&mut engine, &mut session, 0)
        .into_iter()
        .find(|i| i["on"] == "B")
        .unwrap()["id"]
        .clone();
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-set", "page": 0, "widgetId": t, "value": "abcdef"}),
    );
    let refused = op(
        &mut engine,
        &mut session,
        json!({"op": "field-props", "page": 0, "widgetId": t, "maxLen": 3}),
    );
    assert_eq!(
        (refused["ok"].clone(), refused["detail"].clone()),
        (json!(false), json!("invalid"))
    );
    let before = serialized(&session);
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-props", "page": 0, "widgetId": t,
        "rect": [100, 200, 280, 260], "multiline": true, "label": "Label", "required": true}),
    );
    let n = t.as_u64().unwrap() as u32;
    let widget = dict(&session, n);
    assert_eq!(
        widget.get(b"Ff".as_slice()),
        Some(&Object::Integer(4096 | 2))
    );
    assert_eq!(text(widget.get(b"TU".as_slice()).unwrap()), "Label");
    let listed = item(&mut engine, &mut session, "t");
    assert_eq!(listed["rect"], json!([100, 200, 280, 260]));
    assert_eq!(listed["multiline"], true);
    ok(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
    // Options without the value or the default reset the value and drop /DV.
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-props", "page": 0, "widgetId": d, "options": ["x", "y"]}),
    );
    let combo = item(&mut engine, &mut session, "d");
    assert_eq!(
        (combo["value"].clone(), combo["options"].clone()),
        (json!("x"), json!(["x", "y"]))
    );
    assert!(combo.get("default").is_none());
    // A kid renames its group and the group's flags change on the parent.
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-props", "page": 0, "widgetId": kid, "name": "h", "readOnly": true, "default": "B"}),
    );
    let group: Vec<Value> = items(&mut engine, &mut session, 0)
        .into_iter()
        .filter(|i| i["name"] == "h")
        .collect();
    assert_eq!(group.len(), 2);
    assert!(
        group
            .iter()
            .all(|i| i["readOnly"] == true && i["default"] == "B")
    );
    // Name, label and flags never redraw: the step touches only the parent.
    let steps = session.doc.as_ref().unwrap().steps();
    assert_eq!(steps.last().map(String::as_str), Some("change field"));
}

#[test]
fn delete_removes_widgets_and_fields_and_undoes_exactly() {
    let (mut engine, mut session) = session_with(hello());
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "text", "name": "t", "rect": [72, 72, 252, 96]}),
    );
    let radio = ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "radio", "name": "g",
        "kids": [{"on": "A", "rect": [72, 180, 88, 196]}, {"on": "B", "rect": [110, 180, 126, 196]}], "default": "B"}),
    );
    let kids: Vec<Value> = radio["widgetIds"].as_array().unwrap().clone();
    let before = serialized(&session);
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-delete", "page": 0, "widgetId": kids[1]}),
    );
    let parent = dict(&session, kids[0].as_u64().unwrap() as u32)
        .get(b"Parent".as_slice())
        .unwrap()
        .as_reference()
        .unwrap();
    assert!(
        dict(&session, parent.number())
            .get(b"DV".as_slice())
            .is_none(),
        "the default named the deleted kid"
    );
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-delete", "page": 0, "widgetId": kids[0]}),
    );
    let t = item(&mut engine, &mut session, "t")["id"].clone();
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-delete", "page": 0, "widgetId": t}),
    );
    assert!(items(&mut engine, &mut session, 0).is_empty());
    let acro = doc(&session)
        .dict_get(&dict(&session, 1), b"AcroForm")
        .as_dict()
        .unwrap()
        .clone();
    assert_eq!(
        acro.get(b"Fields".as_slice()),
        Some(&Object::Array(Vec::new()))
    );
    assert_eq!(
        dict(&session, 3).get(b"Annots".as_slice()),
        Some(&Object::Array(Vec::new()))
    );
    let saved = saved(&session);
    let reopened = Document::open(&saved, DocumentLimits::default()).unwrap();
    assert!(
        reopened.objects().all(|o| {
            o.value
                .as_dict()
                .is_none_or(|d| d.get(b"Subtype".as_slice()) != Some(&Object::name("Widget")))
        }),
        "no widget survives the save"
    );
    for _ in 0..3 {
        ok(&mut engine, &mut session, json!({"op": "undo"}));
    }
    assert_eq!(serialized(&session), before);
    assert_eq!(
        session.doc.as_ref().unwrap().steps()[2..],
        ["delete field"; 3]
    );
}

#[test]
fn delete_drops_structure_references_to_the_widget() {
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R /StructTreeRoot 8 0 R /MarkInfo << /Marked true >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [6 0 R 7 0 R] /StructParents 0 >>".into(),
        "null".into(),
        "<< /Fields [6 0 R 7 0 R] /DA (/Helv 12 Tf 0 g) >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (gone) /Rect [72 700 252 724] /StructParent 1 >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (kept) /Rect [72 650 252 674] /StructParent 2 >>".into(),
        "<< /Type /StructTreeRoot /K [9 0 R] >>".into(),
        "<< /S /Form /K [<< /Type /OBJR /Obj 6 0 R >> << /Type /OBJR /Obj 7 0 R >>] >>".into(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    let before = serialized(&session);
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-delete", "page": 0, "widgetId": 6}),
    );
    let element = dict(&session, 9);
    let kids = element.get(b"K".as_slice()).unwrap().as_array().unwrap();
    assert_eq!(kids.len(), 1, "the OBJR naming the deleted widget went");
    let bytes = saved(&session);
    let reopened = Document::open(&bytes, DocumentLimits::default()).unwrap();
    assert!(reopened.object(id(6)).is_err(), "the save drops the widget");
    assert!(reopened.object(id(7)).is_ok());
    ok(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
}

#[test]
fn hierarchies_signed_fields_and_scripts_are_refused() {
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [7 0 R 8 0 R] >>".into(),
        "null".into(),
        "<< /Fields [6 0 R 8 0 R] /DA (/Helv 12 Tf 0 g) /CO [8 0 R] >>".into(),
        "<< /T (outer) /Kids [7 0 R] >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (inner) /Parent 6 0 R /Rect [72 650 252 680] >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (calc) /Rect [72 600 252 630] >>".into(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    let before = serialized(&session);
    for request in [
        json!({"op": "field-props", "page": 0, "widgetId": 7, "name": "x"}),
        json!({"op": "field-props", "page": 0, "widgetId": 7, "rect": [72, 72, 252, 96]}),
        json!({"op": "field-delete", "page": 0, "widgetId": 7}),
    ] {
        let reply = op(&mut engine, &mut session, request);
        assert_eq!(reply["detail"], "invalid", "{reply}");
        assert!(
            reply["error"]
                .as_str()
                .unwrap()
                .contains("part of a field hierarchy")
        );
    }
    let scripted = op(
        &mut engine,
        &mut session,
        json!({"op": "field-props", "page": 0, "widgetId": 8, "label": "x"}),
    );
    assert_eq!(scripted["detail"], "scripted");
    let none = op(
        &mut engine,
        &mut session,
        json!({"op": "field-props", "page": 0, "widgetId": 8}),
    );
    assert_eq!(none["detail"], "scripted", "the editable check comes first");
    assert_eq!(serialized(&session), before);
}

// ---------------------------------------------------------------- appearances

/// The synthesis cases (`tests/data/widget-cases.json`): each a widget
/// dictionary, object 6 of a one-page form, and the value set on it.
fn synthesis_cases() -> Vec<Value> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/widget-cases.json");
    let data: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    data["cases"].as_array().unwrap().clone()
}

/// The case's PDF with the case's value written as /V.
fn synthesis_pdf(case: &Value) -> Vec<u8> {
    let mut widget = case["widget"].as_str().unwrap().to_string();
    if let Some(text) = case["V"]["text"].as_str() {
        // The case's value replaces the one its widget starts with.
        if let Some(at) = widget.find("/V (") {
            let end = at + widget[at..].find(')').unwrap() + 1;
            widget.replace_range(at..end, "");
        }
        if text.chars().any(|c| u32::from(c) > 0xff) {
            // UTF-16 with its byte order mark.
            let hex: String = (text.encode_utf16())
                .map(|unit| format!("{unit:04X}"))
                .collect();
            widget = format!("{widget} /V <FEFF{hex}>");
        } else {
            let escaped: String = text
                .chars()
                .flat_map(|c| match c {
                    '(' | ')' | '\\' => vec!['\\', c],
                    '\n' => vec!['\\', 'n'],
                    '\r' => vec!['\\', 'r'],
                    c if u32::from(c) > 0x7f => format!("\\{:03o}", u32::from(c)).chars().collect(),
                    c => vec![c],
                })
                .collect();
            widget = format!("{widget} /V ({escaped})");
        }
    }
    let objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Annots [6 0 R] >>".into(),
        stream("", "q Q"),
        "<< /Fields [6 0 R] /DA (/Helv 12 Tf 0 g) /DR << /Font << /Helv 7 0 R /TiRo 8 0 R /Cour 9 0 R /ZaDb 10 0 R >> >> >>".into(),
        format!("<< /Type /Annot /Subtype /Widget /F 4 /P 3 0 R {widget} >>"),
        FONT_HELV.into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /ZapfDingbats >>".into(),
    ];
    classic(&objects)
}

/// An object an appearance needs: its value and, for a stream, its decoded
/// data.
type Loaded = (Object, Option<Vec<u8>>);

/// The objects an appearance writes, kept in memory with stream data.
#[derive(Default)]
struct Collect {
    objects: BTreeMap<ObjectId, Loaded>,
    fonts: BTreeMap<Vec<u8>, ObjectId>,
}

impl Collect {
    fn put(&mut self, value: Object, data: Option<Vec<u8>>) -> Result<ObjectId, ()> {
        let id = id(10_000 + self.objects.len() as u32);
        self.objects.insert(id, (value, data));
        Ok(id)
    }
}

impl Sink for Collect {
    type Error = ();
    fn stream(&mut self, dictionary: Dictionary, content: Vec<u8>) -> Result<ObjectId, ()> {
        self.put(Object::Dictionary(dictionary), Some(content))
    }
    fn object(&mut self, value: Object) -> Result<ObjectId, ()> {
        self.put(value, None)
    }
    fn fonts(&mut self) -> &mut BTreeMap<Vec<u8>, ObjectId> {
        &mut self.fonts
    }
}

/// Object `id` from the sink, else from the document.
fn load(doc: &Document<'_>, sink: &Collect, id: ObjectId) -> Option<Loaded> {
    if let Some(loaded) = sink.objects.get(&id) {
        return Some(loaded.clone());
    }
    let object = doc.object(id).ok()?;
    let data = doc.decoded_stream(id, Default::default()).ok();
    Some((object.value.clone(), data))
}

fn references(value: &Object, out: &mut Vec<ObjectId>) {
    match value {
        Object::Reference(id) => out.push(*id),
        Object::Array(items) => items.iter().for_each(|item| references(item, out)),
        Object::Dictionary(dict) => dict.values().for_each(|item| references(item, out)),
        _ => {}
    }
}

/// Every object `root` reaches.
fn gather(load: &dyn Fn(ObjectId) -> Option<Loaded>, root: ObjectId) -> BTreeMap<ObjectId, Loaded> {
    let mut out = BTreeMap::new();
    let mut queue = std::collections::VecDeque::from([root]);
    while let Some(next) = queue.pop_front() {
        if out.contains_key(&next) {
            continue;
        }
        let Some(loaded) = load(next) else {
            continue;
        };
        let mut found = Vec::new();
        references(&loaded.0, &mut found);
        queue.extend(found);
        out.insert(next, loaded);
    }
    out
}

fn renumber(value: &mut Object, map: &BTreeMap<ObjectId, u32>) {
    match value {
        Object::Reference(old) => {
            *value = map
                .get(old)
                .map_or(Object::Null, |n| Object::Reference(id(*n)))
        }
        Object::Array(items) => items.iter_mut().for_each(|item| renumber(item, map)),
        Object::Dictionary(dict) => dict.values_mut().for_each(|item| renumber(item, map)),
        _ => {}
    }
}

fn numbers(value: Option<&Object>) -> Option<Vec<f64>> {
    value?.as_array()?.iter().map(Object::as_f64).collect()
}

/// A classic-xref PDF from object bodies (bytes) numbered from 1.
fn classic_bytes(objects: &[Vec<u8>]) -> Vec<u8> {
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

/// A one-page PDF that draws the form `root` (with the `objects` it
/// reaches) as a viewer draws an appearance: its /BBox, transformed by its
/// /Matrix, mapped onto `rect` (ISO 32000-2 §12.5.5).
fn appearance_page(
    objects: &BTreeMap<ObjectId, Loaded>,
    root: ObjectId,
    rect: [f64; 4],
) -> Vec<u8> {
    let mut map = BTreeMap::new();
    map.insert(root, 5);
    for old in objects.keys().filter(|old| **old != root) {
        let next = 5 + map.len() as u32;
        map.insert(*old, next);
    }
    let mut bodies: Vec<(u32, Vec<u8>)> = Vec::new();
    for (old, (value, data)) in objects {
        let mut value = value.clone();
        renumber(&mut value, &map);
        let body = match (data, &mut value) {
            (Some(data), Object::Dictionary(dict)) => {
                dict.remove(b"Filter".as_slice());
                dict.remove(b"DecodeParms".as_slice());
                dict.insert(b"Length".to_vec(), Object::Integer(data.len() as i64));
                let mut body = serialize_object(&value, WriteLimits::default()).unwrap();
                body.extend_from_slice(b"\nstream\n");
                body.extend_from_slice(data);
                body.extend_from_slice(b"\nendstream");
                body
            }
            _ => serialize_object(&value, WriteLimits::default()).unwrap(),
        };
        bodies.push((map[old], body));
    }
    bodies.sort();
    let form = objects[&root].0.as_dict().unwrap();
    let bbox = numbers(form.get(b"BBox".as_slice())).unwrap();
    let m = numbers(form.get(b"Matrix".as_slice())).unwrap_or(vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let corners = [
        (bbox[0], bbox[1]),
        (bbox[2], bbox[1]),
        (bbox[0], bbox[3]),
        (bbox[2], bbox[3]),
    ]
    .map(|(x, y)| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]));
    let xs = corners.map(|c| c.0);
    let ys = corners.map(|c| c.1);
    let lo = |v: [f64; 4]| v.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = |v: [f64; 4]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let scale = |from: f64, to: f64| if from.abs() < 1e-9 { 1.0 } else { to / from };
    let sx = scale(hi(xs) - lo(xs), rect[2] - rect[0]);
    let sy = scale(hi(ys) - lo(ys), rect[3] - rect[1]);
    let (e, f) = (rect[0] - lo(xs) * sx, rect[1] - lo(ys) * sy);
    let content = format!("q {sx} 0 0 {sy} {e} {f} cm /A Do Q");
    let mut out: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
          /Resources << /XObject << /A 5 0 R >> >> >>"
            .to_vec(),
        stream("", &content).into_bytes(),
    ];
    out.extend(bodies.into_iter().map(|(_, body)| body));
    classic_bytes(&out)
}

const PAGE_HEIGHT: f64 = 792.0;
/// Pixels per point of every appearance render.
const SCALE: f64 = 2.0;

/// An extracted character's baseline origin, page space with y up.
struct Char {
    c: char,
    origin: [f64; 2],
}

/// What the engine's view makes of a page.
struct Drawn {
    text: String,
    chars: Vec<Char>,
    image: tandem_pdf_engine::view::RgbaImage,
}

fn draw(page: Vec<u8>) -> Drawn {
    let view = ViewDocument::open(page, "").unwrap();
    let stext = view.structured_text(0).unwrap();
    let chars = (stext.blocks().iter())
        .flat_map(|block| &block.lines)
        .flat_map(|line| &line.chars)
        .map(|ch| Char {
            c: ch.c,
            origin: [ch.origin[0], PAGE_HEIGHT - ch.origin[1]],
        })
        .collect();
    let image = view
        .render(
            0,
            SCALE,
            RenderOptions {
                annotations: false,
                ..Default::default()
            },
        )
        .unwrap();
    Drawn {
        text: stext.as_text(),
        chars,
        image,
    }
}

fn inked(rgb: &[u8]) -> bool {
    rgb.iter().any(|&c| c < 245)
}

fn compact(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

impl Drawn {
    /// Pixel bounds (x0, y0, x1, y1, rows down) of a page rect, y up.
    fn pixels_of(&self, r: [f64; 4]) -> [u32; 4] {
        let clamp_x = |v: f64| (v.max(0.0) as u32).min(self.image.width);
        let clamp_y = |v: f64| (v.max(0.0) as u32).min(self.image.height);
        [
            clamp_x((r[0] * SCALE).floor()),
            clamp_y(((PAGE_HEIGHT - r[3]) * SCALE).floor()),
            clamp_x((r[2] * SCALE).ceil()),
            clamp_y(((PAGE_HEIGHT - r[1]) * SCALE).ceil()),
        ]
    }
    fn inked_at(&self, x: u32, y: u32) -> bool {
        let at = ((y * self.image.width + x) * 4) as usize;
        inked(&self.image.pixels[at..at + 3])
    }
    /// Pixels painted anything but white inside the page rect `r`.
    fn ink(&self, r: [f64; 4]) -> usize {
        let [x0, y0, x1, y1] = self.pixels_of(r);
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| self.inked_at(x, y))
            .count()
    }
    /// Painted pixels farther than one pixel outside `r`.
    fn ink_outside(&self, r: [f64; 4]) -> usize {
        let [x0, y0, x1, y1] = self.pixels_of(r);
        let (x0, y0) = (x0.saturating_sub(1), y0.saturating_sub(1));
        let (x1, y1) = (x1 + 1, y1 + 1);
        (0..self.image.height)
            .flat_map(|y| (0..self.image.width).map(move |x| (x, y)))
            .filter(|&(x, y)| !(x >= x0 && x < x1 && y >= y0 && y < y1) && self.inked_at(x, y))
            .count()
    }
}

fn operations(content: &[u8]) -> Vec<Operation> {
    let mut parser = ContentParser::new(content, ContentLimits::default());
    let mut out = Vec::new();
    loop {
        match parser.next_operation() {
            Ok(Some(op)) => out.push(op),
            Ok(None) => return out,
            Err(error) => panic!(
                "content does not parse: {error}\n{}",
                String::from_utf8_lossy(content)
            ),
        }
    }
}

/// The operators of ISO 32000-2 Annex A.
const ISO_OPERATORS: [&str; 73] = [
    "b", "B", "b*", "B*", "BDC", "BI", "BMC", "BT", "BX", "c", "cm", "CS", "cs", "d", "d0", "d1",
    "Do", "DP", "EI", "EMC", "ET", "EX", "f", "F", "f*", "G", "g", "gs", "h", "i", "ID", "j", "J",
    "K", "k", "l", "m", "M", "MP", "n", "q", "Q", "re", "RG", "rg", "ri", "s", "S", "SC", "sc",
    "SCN", "scn", "sh", "T*", "Tc", "Td", "TD", "Tf", "Tj", "TJ", "TL", "Tm", "Tr", "Ts", "Tw",
    "Tz", "v", "w", "W", "W*", "y", "'", "\"",
];

/// ISO operators only, finite numbers, and balanced q/Q, BT/ET and marked
/// content.
fn structure_errors(ops: &[Operation]) -> Vec<String> {
    let mut errors = Vec::new();
    let (mut q, mut bt, mut marked) = (0i64, 0i64, 0i64);
    for op in ops {
        let operator = String::from_utf8_lossy(&op.operator).into_owned();
        if !ISO_OPERATORS.contains(&operator.as_str()) {
            errors.push(format!("operator {operator} is not ISO"));
        }
        let finite = op.operands.iter().all(|operand| match operand {
            ContentOperand::Real(v) => v.is_finite(),
            ContentOperand::Array(items) => items
                .iter()
                .all(|item| !matches!(item, ContentOperand::Real(v) if !v.is_finite())),
            _ => true,
        });
        if !finite {
            errors.push(format!("{operator} has a number that is not finite"));
        }
        match operator.as_str() {
            "q" => q += 1,
            "Q" => q -= 1,
            "BT" => bt += 1,
            "ET" => bt -= 1,
            "BMC" | "BDC" => marked += 1,
            "EMC" => marked -= 1,
            _ => {}
        }
        if q < 0 || !(0..=1).contains(&bt) || marked < 0 {
            errors.push(format!("unbalanced at {operator}"));
        }
    }
    if q != 0 || bt != 0 || marked != 0 {
        errors.push(format!(
            "unbalanced: q {q}, BT {bt}, marked content {marked}"
        ));
    }
    errors
}

/// The content has a `/Tx BMC` with its `EMC` (ISO 32000-2 §12.7.4.3).
fn tx_marked(ops: &[Operation]) -> bool {
    let tx = ContentOperand::Name(b"Tx".to_vec());
    let open = ops
        .iter()
        .position(|op| op.operator == b"BMC" && op.operands == [tx.clone()]);
    open.is_some_and(|at| ops[at..].iter().any(|op| op.operator == b"EMC"))
}

fn number(operand: &ContentOperand) -> Option<f64> {
    match operand {
        ContentOperand::Integer(v) => Some(*v as f64),
        ContentOperand::Real(v) => Some(*v),
        _ => None,
    }
}

/// One text-showing operation: the font resource it shows in and its bytes.
#[derive(Debug)]
struct Show {
    font: Vec<u8>,
    bytes: Vec<u8>,
}

fn shows(ops: &[Operation]) -> Vec<Show> {
    let mut out = Vec::new();
    let mut font = Vec::new();
    let mut saved = Vec::new();
    for op in ops {
        match op.operator.as_slice() {
            b"q" => saved.push(font.clone()),
            b"Q" => font = saved.pop().unwrap_or_default(),
            b"Tf" => {
                if let Some(ContentOperand::Name(name)) = op.operands.first() {
                    font = name.clone();
                }
            }
            b"Tj" | b"TJ" | b"'" | b"\"" => {
                let mut bytes = Vec::new();
                for operand in &op.operands {
                    match operand {
                        ContentOperand::String(s) => bytes.extend_from_slice(s),
                        ContentOperand::Array(items) => {
                            for item in items {
                                if let ContentOperand::String(s) = item {
                                    bytes.extend_from_slice(s);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                out.push(Show {
                    font: font.clone(),
                    bytes,
                });
            }
            _ => {}
        }
    }
    out
}

/// One stored appearance stream and the objects it reaches.
struct View {
    /// `N`, `D`, or `N/<state>`.
    key: String,
    root: ObjectId,
    rect: [f64; 4],
    objects: BTreeMap<ObjectId, Loaded>,
}

impl View {
    fn of(
        load: &dyn Fn(ObjectId) -> Option<Loaded>,
        key: String,
        root: ObjectId,
        rect: [f64; 4],
    ) -> Self {
        Self {
            key,
            root,
            rect,
            objects: gather(load, root),
        }
    }
    fn dict(&self) -> &Dictionary {
        self.objects[&self.root].0.as_dict().unwrap()
    }
    fn ops(&self) -> Vec<Operation> {
        operations(self.objects[&self.root].1.as_deref().unwrap())
    }
    fn resolve(&self, value: &Object) -> Object {
        match value {
            Object::Reference(id) => self
                .objects
                .get(id)
                .map_or(Object::Null, |(value, _)| value.clone()),
            value => value.clone(),
        }
    }
    fn data(&self, value: &Object) -> Option<&[u8]> {
        let id = value.as_reference()?;
        self.objects.get(&id)?.1.as_deref()
    }
    /// /Resources /Font, resource name → font dictionary.
    fn fonts(&self) -> BTreeMap<Vec<u8>, Dictionary> {
        let resources = self.dict().get(b"Resources".as_slice());
        let resources = self.resolve(resources.unwrap_or(&Object::Null));
        let fonts = (resources.as_dict())
            .and_then(|r| r.get(b"Font".as_slice()))
            .map_or(Object::Null, |f| self.resolve(f));
        let Some(fonts) = fonts.as_dict() else {
            return BTreeMap::new();
        };
        (fonts.iter())
            .filter_map(|(name, font)| Some((name.clone(), self.resolve(font).as_dict()?.clone())))
            .collect()
    }
    fn draw(&self) -> Drawn {
        draw(appearance_page(&self.objects, self.root, self.rect))
    }
}

fn rect_of(doc: &Document<'_>, widget: &Dictionary) -> [f64; 4] {
    let r = numbers(Some(doc.dict_get(widget, b"Rect"))).unwrap();
    [
        r[0].min(r[2]),
        r[1].min(r[3]),
        r[0].max(r[2]),
        r[1].max(r[3]),
    ]
}

/// Every form of an /AP dictionary.
fn stored_forms(
    load: &dyn Fn(ObjectId) -> Option<Loaded>,
    ap: &Dictionary,
    rect: [f64; 4],
) -> Vec<View> {
    let mut forms = Vec::new();
    for (key, entry) in ap.iter() {
        let key = String::from_utf8_lossy(key).into_owned();
        match load_entry(load, entry) {
            Object::Dictionary(states) => {
                for (state, value) in states.iter() {
                    let state = String::from_utf8_lossy(state);
                    let root = value.as_reference().expect("an indirect form");
                    forms.push(View::of(load, format!("{key}/{state}"), root, rect));
                }
            }
            _ => {
                let root = entry.as_reference().expect("an indirect form");
                forms.push(View::of(load, key, root, rect));
            }
        }
    }
    forms
}

/// An /AP entry: a state dictionary (resolved), or the form's reference.
fn load_entry(load: &dyn Fn(ObjectId) -> Option<Loaded>, entry: &Object) -> Object {
    match entry {
        Object::Reference(id) => match load(*id) {
            Some((value @ Object::Dictionary(_), None)) => value,
            _ => entry.clone(),
        },
        value => value.clone(),
    }
}

/// The widget `widget`'s stored /AP /N form in `doc`.
fn stored_normal(doc: &Document<'_>, widget: ObjectId) -> Option<View> {
    let dict = doc.resolve_id(widget).as_dict()?;
    let ap = doc.dict_get(dict, b"AP").as_dict()?;
    let root = ap.get(b"N".as_slice())?.as_reference()?;
    let sink = Collect::default();
    let loader = |id: ObjectId| load(doc, &sink, id);
    Some(View::of(&loader, "N".into(), root, rect_of(doc, dict)))
}

const MULTILINE: i64 = 1 << 12;
const PASSWORD: i64 = 1 << 13;
const COMBO: i64 = 1 << 17;
const PUSHBUTTON: i64 = 1 << 16;
const COMB: i64 = 1 << 24;

/// The font name a /DA string sets (the operand before its last `Tf`),
/// `Helv` when it sets none.
fn da_font_name(da: &[u8]) -> Vec<u8> {
    let ops = operations(da);
    let tf = ops.iter().rev().find(|op| op.operator == b"Tf");
    match tf.and_then(|op| op.operands.first()) {
        Some(ContentOperand::Name(name)) => name.clone(),
        _ => b"Helv".to_vec(),
    }
}

/// The text a widget shows (spec 07 section 3), from its dictionary: None
/// for check boxes and radio buttons.
fn shown_text(doc: &Document<'_>, widget: &Dictionary) -> Option<String> {
    let flags = doc.dict_get(widget, b"Ff").as_i64().unwrap_or(0);
    let string = |value: &Object| doc.resolve(value).as_string().map(decode_text_string);
    let value = string(doc.dict_get(widget, b"V")).unwrap_or_default();
    match doc.dict_get(widget, b"FT").as_name()? {
        b"Tx" => {
            let chars: Vec<char> = value.chars().collect();
            let max = doc.dict_get(widget, b"MaxLen").as_i64().unwrap_or(0);
            Some(if flags & PASSWORD != 0 {
                "*".repeat(chars.len())
            } else if flags & COMB != 0 && flags & MULTILINE == 0 && max > 0 {
                chars.iter().take(max as usize).collect()
            } else {
                value
            })
        }
        b"Ch" => {
            let options: Vec<(String, String)> = (doc.dict_get(widget, b"Opt").as_array())
                .unwrap_or(&[])
                .iter()
                .filter_map(|option| match doc.resolve(option) {
                    Object::Array(pair) => Some((string(pair.first()?)?, string(pair.get(1)?)?)),
                    option => {
                        let text = string(option)?;
                        Some((text.clone(), text))
                    }
                })
                .collect();
            if flags & COMBO != 0 {
                let shown = options.iter().find(|(export, _)| *export == value);
                Some(shown.map_or(value, |(_, display)| display.clone()))
            } else {
                let top = doc.dict_get(widget, b"TI").as_i64().unwrap_or(0).max(0) as usize;
                let shown: Vec<&str> = options.iter().skip(top).map(|(_, d)| d.as_str()).collect();
                Some(shown.join("\n"))
            }
        }
        b"Btn" if flags & PUSHBUTTON != 0 => {
            let mk = doc.dict_get(widget, b"MK").as_dict();
            Some(
                mk.and_then(|mk| string(doc.dict_get(mk, b"CA")))
                    .unwrap_or_default(),
            )
        }
        _ => None,
    }
}

/// Each case's appearance, stored through `appearance_dict` and drawn onto
/// a fresh page, meets spec 07 sections 3 and 9.2: (a) /BBox and /Matrix,
/// (b) content structure, (c) lossy only where no font encodes the value,
/// (d) ink only inside /Rect and wherever something is drawn, (e) the
/// extracted text is the text shown, (f) single-line baselines inside
/// /Rect, (g) the fonts the content names are its resources.
#[test]
fn synthesis_cases_meet_the_spec() {
    let cases = synthesis_cases();
    assert_eq!(cases.len(), 102);
    let mut failures = Vec::new();
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let bytes = synthesis_pdf(case);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let widget = doc.resolve_id(id(6)).as_dict().unwrap();
        let synth = Context::of(&doc).synthesize(widget).unwrap();
        // (c)
        if synth.lossy() != (name == "tx_arrow") {
            failures.push(format!("{name}: lossy {}", synth.lossy()));
        }
        let mut sink = Collect::default();
        let ap = appearance_dict(&mut sink, &synth).unwrap();
        let rect = rect_of(&doc, widget);
        let loader = |id: ObjectId| load(&doc, &sink, id);
        let forms = stored_forms(&loader, &ap, rect);
        let flags = doc.dict_get(widget, b"Ff").as_i64().unwrap_or(0);
        let kind = doc.dict_get(widget, b"FT").as_name().unwrap_or(b"");
        let variable = kind == b"Tx" || kind == b"Ch";
        let single =
            (kind == b"Tx" && flags & MULTILINE == 0) || (kind == b"Ch" && flags & COMBO != 0);
        let mk = doc.dict_get(widget, b"MK").as_dict();
        let rotate = mk
            .and_then(|mk| doc.dict_get(mk, b"R").as_i64())
            .unwrap_or(0)
            .rem_euclid(360);
        let expected = shown_text(&doc, widget).map(|text| compact(&text));
        let da_font = da_font_name(doc.dict_get(widget, b"DA").as_string().unwrap_or(b""));
        let mut ink = BTreeMap::new();
        let mut images = BTreeMap::new();
        for form in &forms {
            let what = format!("{name} {}", form.key);
            // (a)
            let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
            let (w, h) = if rotate == 90 || rotate == 270 {
                (h, w)
            } else {
                (w, h)
            };
            let bbox = numbers(form.dict().get(b"BBox".as_slice())).unwrap_or_default();
            let close = |a: &[f64], b: &[f64]| {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
            };
            if !close(&bbox, &[0.0, 0.0, w, h]) {
                failures.push(format!("{what}: /BBox {bbox:?}, not [0 0 {w} {h}]"));
            }
            let matrix = numbers(form.dict().get(b"Matrix".as_slice()))
                .unwrap_or(vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
            let turned: [f64; 6] = match rotate {
                90 => [0.0, 1.0, -1.0, 0.0, 0.0, 0.0],
                180 => [-1.0, 0.0, 0.0, -1.0, 0.0, 0.0],
                270 => [0.0, -1.0, 1.0, 0.0, 0.0, 0.0],
                _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            };
            if !close(&matrix, &turned) {
                failures.push(format!("{what}: /Matrix {matrix:?}, not {turned:?}"));
            }
            // (b)
            let ops = form.ops();
            for error in structure_errors(&ops) {
                failures.push(format!("{what}: {error}"));
            }
            if variable && !tx_marked(&ops) {
                failures.push(format!("{what}: no /Tx BMC … EMC"));
            }
            // (g)
            let named: BTreeSet<Vec<u8>> = (ops.iter())
                .filter(|op| op.operator == b"Tf")
                .filter_map(|op| match op.operands.first() {
                    Some(ContentOperand::Name(name)) => Some(name.clone()),
                    _ => None,
                })
                .collect();
            // The DA font may be listed though fallback fonts draw all text.
            let resources: BTreeSet<Vec<u8>> = form.fonts().into_keys().collect();
            let unused = (resources.iter()).any(|r| !named.contains(r) && *r != da_font);
            if !named.is_subset(&resources) || unused {
                failures.push(format!(
                    "{what}: content fonts {named:?}, resources {resources:?}"
                ));
            }
            // (d)
            let drawn = form.draw();
            let outside = drawn.ink_outside(rect);
            if outside > 0 {
                failures.push(format!("{what}: {outside} pixels painted outside /Rect"));
            }
            ink.insert(form.key.clone(), drawn.ink(rect));
            // (e), (f)
            if let Some(want) = &expected
                && !synth.lossy()
            {
                let got = compact(&drawn.text);
                let clipped = ["ch_list_long", "tx_tiny"].contains(&name);
                let same = if clipped {
                    want.starts_with(&got)
                } else {
                    &got == want
                };
                if !same || (name == "ch_list_long" && got.is_empty()) {
                    failures.push(format!("{what}: extracts {got:?}, shows {want:?}"));
                }
                // Text without CJK draws without a CJK font installed.
                let cjk = want.chars().any(|c| u32::from(c) >= 0x2E80);
                if !want.is_empty() && !cjk && ink[&form.key] == 0 {
                    failures.push(format!("{what}: no ink"));
                }
                if single {
                    for ch in &drawn.chars {
                        let [x, y] = ch.origin;
                        let inside = x >= rect[0] - 0.01
                            && x <= rect[2] + 0.01
                            && y >= rect[1]
                            && y <= rect[3];
                        if !inside {
                            failures.push(format!(
                                "{what}: {} on a baseline at {x},{y} outside /Rect",
                                ch.c
                            ));
                        }
                    }
                }
            }
            images.insert(form.key.clone(), drawn);
        }
        // (d) an on state paints a mark the Off state does not.
        if let Some(off) = images.get("N/Off") {
            let on = (images.iter()).find(|(key, _)| key.starts_with("N/") && *key != "N/Off");
            let marked = on.is_some_and(|(_, on): (_, &Drawn)| {
                let [x0, y0, x1, y1] = on.pixels_of(rect);
                (y0..y1)
                    .flat_map(|y| (x0..x1).map(move |x| (x, y)))
                    .any(|(x, y)| {
                        let at = ((y * on.image.width + x) * 4) as usize;
                        on.image.pixels[at..at + 3] != off.image.pixels[at..at + 3]
                    })
            });
            if !marked {
                failures.push(format!("{name}: the on state draws no mark"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// What the engine's text extraction reads for each of `codes` shown in
/// the simple font `font` (direct values): a viewer's view of its glyph
/// names (Adobe Glyph List and `uniXXXX`, ISO 32000-2 §9.10.2).
fn read_codes(font: &Dictionary, codes: &[u8]) -> BTreeMap<u8, String> {
    let mut content = String::new();
    for (line, code) in codes.iter().enumerate() {
        let y = 780 - 6 * line;
        content.push_str(&format!("BT /F 4 Tf 10 {y} Td <{code:02X}> Tj ET\n"));
    }
    let objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
          /Resources << /Font << /F 5 0 R >> >> >>"
            .to_vec(),
        stream("", &content).into_bytes(),
        serialize_object(&Object::Dictionary(font.clone()), WriteLimits::default()).unwrap(),
    ];
    let view = ViewDocument::open(classic_bytes(&objects), "").unwrap();
    let stext = view.structured_text(0).unwrap();
    let mut out: BTreeMap<u8, String> = BTreeMap::new();
    for line in stext.blocks().iter().flat_map(|block| &block.lines) {
        for ch in &line.chars {
            let line = ((780.0 - (PAGE_HEIGHT - ch.origin[1])) / 6.0).round();
            if let Some(code) = codes.get(line as usize) {
                out.entry(*code).or_default().push(ch.c);
            }
        }
    }
    out
}

/// An 8-bit fallback font's faults: a non-embedded font names its glyphs
/// in /Differences by glyph list names and declares the advances the
/// layout uses; an embedded one (owner decision W1) carries its program
/// and a /ToUnicode.
fn eight_bit_faults(form: &View, font: &Dictionary, family: Family, base: &[u8]) -> Vec<String> {
    let mut faults = Vec::new();
    let get = |key: &[u8]| {
        font.get(key)
            .map(|v| form.resolve(v))
            .unwrap_or(Object::Null)
    };
    if get(b"Type") != Object::name("Font") {
        faults.push("no /Type /Font".to_string());
    }
    let first = get(b"FirstChar").as_i64().unwrap_or(-1);
    let last = get(b"LastChar").as_i64().unwrap_or(-2);
    let widths = numbers(Some(&get(b"Widths"))).unwrap_or_default();
    if first < 0 || widths.len() as i64 != last - first + 1 {
        faults.push(format!("/Widths of {} for {first}..{last}", widths.len()));
    }
    let descriptor = get(b"FontDescriptor");
    let program = (descriptor.as_dict()).and_then(|d| {
        [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"]
            .iter()
            .find_map(|key| d.get(*key))
    });
    if let Some(program) = program {
        if form.data(program).is_none_or(|d| d.is_empty()) {
            faults.push("an empty font program".into());
        }
        if font
            .get(b"ToUnicode".as_slice())
            .and_then(|t| form.data(t))
            .is_none()
        {
            faults.push("an embedded font without /ToUnicode".into());
        }
        return faults;
    }
    if get(b"Subtype") != Object::name("Type1") {
        faults.push("not /Type1".into());
    }
    if get(b"BaseFont") != Object::name(base) {
        faults.push(format!(
            "/BaseFont {:?}, not {}",
            get(b"BaseFont"),
            String::from_utf8_lossy(base)
        ));
    }
    let encoding = get(b"Encoding");
    let encoding = encoding.as_dict();
    if encoding.and_then(|e| e.get(b"BaseEncoding".as_slice()))
        != Some(&Object::name("WinAnsiEncoding"))
    {
        faults.push("no /BaseEncoding /WinAnsiEncoding".into());
    }
    let differences = encoding
        .and_then(|e| e.get(b"Differences".as_slice()))
        .and_then(Object::as_array)
        .unwrap_or(&[]);
    if differences.is_empty() {
        faults.push("no /Differences".into());
    }
    // Each named code reads back as one character (ISO 32000-2 §9.10.2)
    // and draws (spec 07 section 6.3).
    let mut named = Vec::new();
    let mut code = 0i64;
    for entry in differences {
        match entry {
            Object::Integer(next) => code = *next,
            Object::Name(name) => {
                named.push((code, String::from_utf8_lossy(name).into_owned()));
                code += 1;
            }
            _ => faults.push("not a code or a name in /Differences".into()),
        }
    }
    let mut probe = Dictionary::new();
    for (key, value) in font.iter() {
        if key.as_slice() != b"FontDescriptor" && key.as_slice() != b"ToUnicode" {
            probe.insert(key.clone(), form.resolve(value));
        }
    }
    if let Some(Object::Dictionary(encoding)) = probe.get_mut(b"Encoding".as_slice()) {
        encoding
            .values_mut()
            .for_each(|value| *value = form.resolve(value));
    }
    let codes: Vec<u8> = named
        .iter()
        .filter_map(|(code, _)| u8::try_from(*code).ok())
        .collect();
    let read = read_codes(&probe, &codes);
    for (code, name) in &named {
        let text = u8::try_from(*code)
            .ok()
            .and_then(|code| read.get(&code))
            .cloned();
        let text = text.unwrap_or_default();
        let mut chars = text.chars();
        let one = match (chars.next(), chars.next()) {
            (Some(c), None) => u32::from(c) >= 0xA0 && c != '\u{FFFD}',
            _ => false,
        };
        if !one {
            faults.push(format!("{name} at {code:#x} reads as {text:?}"));
        }
        let at = usize::try_from(*code - first).ok();
        if !at.and_then(|at| widths.get(at)).is_some_and(|w| *w > 0.0) {
            faults.push(format!("{name} at {code:#x} draws nothing"));
        }
    }
    let declared = [Eight::Greek, Eight::Cyrillic, Eight::Latin]
        .iter()
        .any(|eight| {
            let advances = eight.widths(family);
            (widths.iter().enumerate()).all(|(at, w)| {
                let code = first as usize + at;
                code < 256 && *w == advances[code].round()
            })
        });
    if !declared {
        faults.push("/Widths are not an 8-bit encoding's advances".into());
    }
    faults
}

/// The CJK names Adobe documents for its non-embedded fonts, by ordering
/// (owner decision W2).
const ADOBE_CJK: [(&str, &[&str]); 4] = [
    (
        "GB1",
        &["STSong-Light", "STHeiti-Regular", "AdobeSongStd-Light"],
    ),
    (
        "CNS1",
        &["MSung-Light", "MHei-Medium", "AdobeMingStd-Light"],
    ),
    (
        "Japan1",
        &[
            "HeiseiMin-W3",
            "KozMinPro-Regular",
            "HeiseiKakuGo-W5",
            "KozGoPro-Medium",
        ],
    ),
    (
        "Korea1",
        &[
            "HYSMyeongJo-Medium",
            "HYGoThic-Medium",
            "AdobeMyungjoStd-Medium",
        ],
    ),
];

/// A CID fallback font's faults: Type0 over the collection's UTF-16 CMap,
/// a CIDFontType0 of that collection, and a descriptor with every key
/// ISO 32000-2 Table 120 requires.
fn cid_faults(form: &View, font: &Dictionary) -> Vec<String> {
    let mut faults = Vec::new();
    let get = |dict: &Dictionary, key: &[u8]| {
        dict.get(key)
            .map(|v| form.resolve(v))
            .unwrap_or(Object::Null)
    };
    let cmap = get(font, b"Encoding");
    let ordering = match cmap.as_name().unwrap_or(b"") {
        b"UniGB-UTF16-H" => "GB1",
        b"UniCNS-UTF16-H" => "CNS1",
        b"UniJIS-UTF16-H" => "Japan1",
        b"UniKS-UTF16-H" => "Korea1",
        _ => {
            faults.push(format!("/Encoding {cmap:?}"));
            return faults;
        }
    };
    let descendant = get(font, b"DescendantFonts");
    let descendant = descendant
        .as_array()
        .and_then(|d| d.first())
        .map(|d| form.resolve(d));
    let Some(Object::Dictionary(descendant)) = descendant else {
        faults.push("no descendant".into());
        return faults;
    };
    if get(&descendant, b"Subtype") != Object::name("CIDFontType0") {
        faults.push("the descendant is not CIDFontType0".into());
    }
    let info = get(&descendant, b"CIDSystemInfo");
    let info = info.as_dict();
    let text = |key: &[u8]| {
        info.and_then(|i| i.get(key))
            .and_then(Object::as_string)
            .map(|s| String::from_utf8_lossy(s).into_owned())
    };
    if text(b"Registry").as_deref() != Some("Adobe")
        || text(b"Ordering").as_deref() != Some(ordering)
    {
        faults.push(format!("/CIDSystemInfo {info:?} for {ordering}"));
    }
    if info
        .and_then(|i| i.get(b"Supplement".as_slice()))
        .and_then(Object::as_i64)
        .is_none_or(|s| s < 0)
    {
        faults.push("no /Supplement".into());
    }
    let base = get(&descendant, b"BaseFont");
    let base = String::from_utf8_lossy(base.as_name().unwrap_or(b"")).into_owned();
    let names = ADOBE_CJK.iter().find(|(o, _)| *o == ordering).unwrap().1;
    if !names.contains(&base.as_str()) {
        faults.push(format!("{ordering} named {base}"));
    }
    let type0 = get(font, b"BaseFont");
    let type0 = String::from_utf8_lossy(type0.as_name().unwrap_or(b"")).into_owned();
    let cmap = String::from_utf8_lossy(cmap.as_name().unwrap_or(b"")).into_owned();
    if type0 != base && type0 != format!("{base}-{cmap}") {
        faults.push(format!("Type0 /BaseFont {type0} for {base}"));
    }
    let descriptor = get(&descendant, b"FontDescriptor");
    let Some(descriptor) = descriptor.as_dict() else {
        faults.push("no descriptor".into());
        return faults;
    };
    for key in [
        "Type",
        "FontName",
        "Flags",
        "FontBBox",
        "ItalicAngle",
        "Ascent",
        "Descent",
        "CapHeight",
        "StemV",
    ] {
        if descriptor.get(key.as_bytes()).is_none() {
            faults.push(format!("the descriptor lacks /{key}"));
        }
    }
    if get(descriptor, b"FontName") != Object::name(base.as_bytes()) {
        faults.push("/FontName is not the /BaseFont".into());
    }
    if get(descriptor, b"Flags").as_i64().unwrap_or(0) & 4 == 0 {
        faults.push("not Symbolic".into());
    }
    let ascent = get(descriptor, b"Ascent").as_f64().unwrap_or(0.0);
    let descent = get(descriptor, b"Descent").as_f64().unwrap_or(0.0);
    if !(ascent > 0.0 && descent < 0.0) {
        faults.push(format!("Ascent {ascent}, Descent {descent}"));
    }
    let bbox = numbers(Some(&get(descriptor, b"FontBBox"))).unwrap_or_default();
    let holds = bbox.len() == 4
        && bbox[0] <= 0.0
        && bbox[1] <= descent
        && bbox[2] >= 1000.0
        && bbox[3] >= ascent;
    if !holds {
        faults.push(format!("/FontBBox {bbox:?}"));
    }
    faults
}

/// Every fallback font of every case is well formed (spec 07 sections 5.2
/// to 5.4 and owner decisions W1, W2).
#[test]
fn fallback_fonts_are_well_formed() {
    let (mut eight_bit, mut cid) = (0, 0);
    let mut failures = Vec::new();
    for case in synthesis_cases() {
        let name = case["name"].as_str().unwrap();
        let widget = case["widget"].as_str().unwrap();
        let (family, base): (Family, &[u8]) = if widget.contains("/TiRo") {
            (Family::Times, b"Times-Roman")
        } else if widget.contains("/Cour") {
            (Family::Courier, b"Courier")
        } else {
            (Family::Helvetica, b"Helvetica")
        };
        let bytes = synthesis_pdf(&case);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let dict = doc.resolve_id(id(6)).as_dict().unwrap();
        let synth = Context::of(&doc).synthesize(dict).unwrap();
        let mut sink = Collect::default();
        let ap = appearance_dict(&mut sink, &synth).unwrap();
        let loader = |id: ObjectId| load(&doc, &sink, id);
        for form in stored_forms(&loader, &ap, rect_of(&doc, dict)) {
            for (key, font) in form.fonts() {
                let what = format!("{name} {} {}", form.key, String::from_utf8_lossy(&key));
                let standard = font.get(b"FontDescriptor".as_slice()).is_none()
                    && font
                        .get(b"Encoding".as_slice())
                        .is_none_or(|e| e.as_name().is_some());
                let faults = if font.get(b"Subtype".as_slice()) == Some(&Object::name("Type0")) {
                    cid += 1;
                    cid_faults(&form, &font)
                } else if !standard {
                    eight_bit += 1;
                    eight_bit_faults(&form, &font, family, base)
                } else {
                    continue;
                };
                failures.extend(faults.into_iter().map(|fault| format!("{what}: {fault}")));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(
        eight_bit >= 20 && cid >= 10,
        "{eight_bit} 8-bit and {cid} CID fonts"
    );
}

#[test]
fn the_view_draws_widgets_without_appearances_and_saves_never_hold_them() {
    // Object 10 is the /DR font, 11 an empty appearance stream.
    let bytes = form_with(
        &[
            "/FT /Tx /T (a) /V (Hello world) /Rect [72 700 252 724] /DA (/Helv 12 Tf 0 g)",
            "/FT /Btn /T (b) /V /Yes /AS /Yes /Rect [72 650 92 670] /DA (/ZaDb 0 Tf 0 g)",
            "/FT /Tx /T (c) /V (Stored) /Rect [72 600 252 624] /DA (/Helv 12 Tf 0 g) /AP << /N 11 0 R >>",
            "/FT /Sig /T (d) /Rect [300 600 400 640]",
        ],
        "",
        &[stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 180 24]",
            "",
        )],
    );
    let (_, mut session) = session_with(bytes);
    // y down: a at 68..92, b at 122..142, c at 168..192, d at 152..192.
    assert!(
        ink(&mut session, [72, 68, 252, 92]) > 20,
        "the text field draws its value"
    );
    assert!(
        ink(&mut session, [72, 122, 92, 142]) > 5,
        "the check box draws its check"
    );
    assert_eq!(
        ink(&mut session, [72, 168, 252, 192]),
        0,
        "a stored appearance is kept"
    );
    assert_eq!(ink(&mut session, [300, 152, 400, 192]), 0, "no SIGN marker");
    let bytes = saved(&session);
    let reopened = Document::open(&bytes, DocumentLimits::default()).unwrap();
    assert!(
        reopened
            .resolve_id(id(6))
            .as_dict()
            .unwrap()
            .get(b"AP".as_slice())
            .is_none()
    );
    assert!(
        reopened.next_id().unwrap().number() <= 12,
        "no synthesized object is saved"
    );
}

#[test]
fn need_appearances_redraws_every_widget_but_signatures() {
    let bytes = form_with(
        &[
            "/FT /Tx /T (a) /V (Needs appearance) /Rect [72 700 252 724] /DA (/Helv 12 Tf 0 g) /AP << /N 11 0 R >>",
            "/FT /Btn /T (b) /V /Yes /AS /Yes /Rect [72 650 92 670] /DA (/ZaDb 0 Tf 0 g) /AP << /N << /Yes 11 0 R /Off 11 0 R >> >>",
            "/FT /Ch /T (c) /V (b) /Opt [(a) (b)] /Rect [300 600 400 680] /DA (/Helv 12 Tf 0 g)",
            "/FT /Btn /Ff 65536 /T (d) /Rect [300 700 400 730] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] /BG [0.75] /CA (Push) >>",
        ],
        "/NeedAppearances true",
        &[stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 20 20]",
            "",
        )],
    );
    let (_, mut session) = session_with(bytes);
    assert!(ink(&mut session, [72, 68, 252, 92]) > 20);
    assert!(ink(&mut session, [72, 122, 92, 142]) > 5);
    assert!(
        ink(&mut session, [300, 112, 400, 192]) > 100,
        "the list and its selection"
    );
    assert!(
        ink(&mut session, [300, 62, 400, 92]) > 500,
        "the push button's bevel and caption"
    );
}

// ---------------------------------------------------------------- hostile input

#[test]
fn hostile_fields_never_panic() {
    let bodies = [
        // A /Parent cycle, a /Kids cycle and odd values everywhere.
        "<< /Type /Annot /Subtype /Widget /Parent 7 0 R /T (a) /Rect [72 700 252 724] /DA (/Helv -5 Tf 9 9 rg) >>",
        "<< /Parent 6 0 R /Kids [6 0 R 7 0 R] /FT /Tx /Ff 1e30 /MaxLen -4 /V [1 2] /Q 99 >>",
        "<< /Type /Annot /Subtype /Widget /FT /Ch /Opt (x) /TI -9 /V 5 /Rect [0 0 0 0] /MK << /R 45 /BG [1 2] /CA 7 >> >>",
        "<< /Type /Annot /Subtype /Widget /FT /Btn /Ff 32768 /Rect [72 1e30 90 90] /AP << /N 7 0 R /D [1] >> /BS << /W -3 >> >>",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /Ff 16777216 /MaxLen 100000000 /V (abc) /Rect [1 1 2 2] /DA (/Helv 1e-30 Tf) >>",
        "<< /Type /Annot /Subtype /Widget /FT /Btn /Ff 65536 /Rect [1 1 2 2] /DA (Tf Tf rg /) /MK << /CA (\\(\\)) >> >>",
    ];
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [6 0 R 7 0 R 8 0 R 9 0 R 10 0 R 11 0 R 12 0 R] >>"
            .into(),
        "null".into(),
        "<< /Fields [6 0 R 7 0 R 6 0 R] /DR << /Font << /Helv 99 0 R >> >> /NeedAppearances true /CO 5 >>".into(),
    ];
    objects.extend(bodies.iter().map(|b| b.to_string()));
    objects.push("<< /Type /Annot /Subtype /Widget /FT /Sig /V << >> /Rect [1 1 50 50] >>".into());
    let (mut engine, mut session) = session_with(classic(&objects));
    let listed = items(&mut engine, &mut session, 0);
    assert!(!listed.is_empty());
    let _ = ink(&mut session, [0, 0, 612, 792]);
    for n in 6..=12 {
        for request in [
            json!({"op": "field-set", "page": 0, "widgetId": n, "value": "x"}),
            json!({"op": "field-set", "page": 0, "widgetId": n, "value": true}),
            json!({"op": "field-props", "page": 0, "widgetId": n, "rect": [10, 10, 100, 100], "name": "q"}),
            json!({"op": "field-props", "page": 0, "widgetId": n, "options": ["a"], "maxLen": 1, "multiline": true}),
            json!({"op": "field-delete", "page": 0, "widgetId": n}),
        ] {
            let _ = op(&mut engine, &mut session, request);
        }
    }
    let _ = op(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "text", "name": "new", "rect": [10, 10, 100, 40]}),
    );
    let _ = op(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "signature", "rect": [10, 100, 100, 140]}),
    );
    let _ = ink(&mut session, [0, 0, 612, 792]);
    while op(&mut engine, &mut session, json!({"op": "undo"}))["ok"] == true {}
    let _ = Kind::Text;
}

// ---------------------------------------------------------------- review fixes

/// The save op's bytes.
fn save_op(engine: &mut AppEngine, session: &mut AppSession) -> Vec<u8> {
    let (reply, mut outputs) = engine.dispatch(session, json!({"op": "save"}), Vec::new());
    assert_eq!(reply["ok"], true, "{reply}");
    outputs.pop().unwrap()
}

/// A text widget the structure tree reaches 255 levels down (the app
/// parser allows 256): it opens, fills, renders, deletes and saves, and the
/// saved bytes open again.
#[test]
fn deep_documents_open_edit_render_and_save() {
    let mut tree = "<< /Type /OBJR /Obj 6 0 R >>".to_string();
    for _ in 0..127 {
        tree = format!("<< /K [{tree}] >>");
    }
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R /StructTreeRoot 7 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [6 0 R] /Tabs /S >>".into(),
        stream("", "q Q"),
        "<< /Fields [6 0 R] /DA (/Helv 12 Tf 0 g) >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (deep) /Rect [72 700 252 724] /DA (/Helv 12 Tf 0 g) >>".into(),
        format!("<< /Type /StructTreeRoot /K {tree} >>"),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    assert!(!session.doc.as_ref().unwrap().info(false).repaired);
    // The earlier app engine reads structure order 64 dictionaries deep at most.
    let listed = items(&mut engine, &mut session, 0);
    assert_eq!(listed[0]["structIndex"], Value::Null);
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-set", "page": 0, "widgetId": 6, "value": "x"}),
    );
    let saved = save_op(&mut engine, &mut session);
    let (_, reopened) = session_with(saved);
    assert!(!reopened.doc.as_ref().unwrap().info(false).repaired);
    // The delete cuts the structure reference 255 levels down.
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-delete", "page": 0, "widgetId": 6}),
    );
    ok(
        &mut engine,
        &mut session,
        json!({"op": "render", "page": 0, "scale": 1}),
    );
    let saved = save_op(&mut engine, &mut session);
    session_with(saved);
    ok(&mut engine, &mut session, json!({"op": "undo"}));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "render", "page": 0, "scale": 1}),
    );
    let saved = save_op(&mut engine, &mut session);
    let (mut engine, mut session) = session_with(saved);
    assert_eq!(items(&mut engine, &mut session, 0)[0]["value"], "x");

    // The catalog itself 200 levels deep: the view's copy is cut where its
    // parser stops (64), the document and its save are not.
    let mut deep = "1".to_string();
    for _ in 0..200 {
        deep = format!("[{deep}]");
    }
    let objects = vec![
        format!("<< /Type /Catalog /Pages 2 0 R /Deep {deep} >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".into(),
        stream("", "0 0 1 rg 72 72 100 100 re f"),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    ok(
        &mut engine,
        &mut session,
        json!({"op": "render", "page": 0, "scale": 1}),
    );
    assert!(ink(&mut session, [72, 620, 172, 720]) > 9000);
    let saved = save_op(&mut engine, &mut session);
    assert!(saved.windows(200).any(|w| w.iter().all(|&b| b == b'[')));
}

/// A rect of the page's full reported size is on the page, and a listed
/// rect sent back is accepted, on pages whose boxes are fractional and off
/// the origin (`PageSpace` and `info::page_size` are single precision).
#[test]
fn full_page_rects_on_fractional_pages_are_on_the_page() {
    for page in [
        "/MediaBox [0.3 0.7 612.1 792.9]",
        "/MediaBox [10.5 20.25 600.3 800.7] /CropBox [11.1 21.3 590.9 790.3] /Rotate 270",
        "/MediaBox [0.1 0.2 300.3 400.4] /Rotate 90 /UserUnit 1.5",
    ] {
        let objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            format!("<< /Type /Page /Parent 2 0 R {page} >>"),
        ];
        let (mut engine, mut session) = session_with(classic(&objects));
        let size = {
            let page = dict(&session, 3);
            tandem_pdf_engine::app::info::page_size(doc(&session), &page)
        };
        let full = json!([0, 0, size.0, size.1]);
        let added = ok(
            &mut engine,
            &mut session,
            json!({"op": "field-add", "page": 0, "kind": "text", "name": "f", "rect": full}),
        );
        let n = added["widgetId"].clone();
        let listed = items(&mut engine, &mut session, 0)[0]["rect"].clone();
        for v in listed.as_array().unwrap() {
            let v = v.as_f64().unwrap();
            assert_eq!(
                f64::from(v as f32),
                v,
                "{page}: {listed} is single precision"
            );
        }
        if page.contains("UserUnit") {
            // The single-precision inverse transform rounds past the page
            // here, and the listed rect is refused as the earlier app engine refuses it.
            assert_eq!(listed, json!([0, 0, 600.300048828125, 450.3000183105469]));
        } else {
            ok(
                &mut engine,
                &mut session,
                json!({"op": "field-props", "page": 0, "widgetId": n, "rect": listed}),
            );
        }
        let inner = json!([1.5, 2.25, size.0 - 0.5, size.1]);
        ok(
            &mut engine,
            &mut session,
            json!({"op": "field-props", "page": 0, "widgetId": n, "rect": inner}),
        );
        let refused = op(
            &mut engine,
            &mut session,
            json!({"op": "field-props", "page": 0, "widgetId": n,
            "rect": [0, 0, size.0 + 0.01, size.1]}),
        );
        assert_eq!(refused["ok"], false, "{page}");
    }
}

/// field-props reads a /Ff a reference holds, so the other flags stay.
#[test]
fn indirect_flags_keep_their_other_bits() {
    let bytes = form_with(
        &[
            "/FT /Ch /Ff 9 0 R /T (c) /V (a) /Opt [(a) (b)] /Rect [72 700 252 724] /DA (/Helv 12 Tf 0 g)",
            "/FT /Tx /Ff 10 0 R /T (t) /V (x) /Rect [72 600 252 624] /DA (/Helv 12 Tf 0 g)",
        ],
        "",
        &["131072".into(), "8192".into()],
    );
    let (mut engine, mut session) = session_with(bytes);
    for (read_only, bits) in [(true, 131073), (false, 131072)] {
        ok(
            &mut engine,
            &mut session,
            json!({"op": "field-props", "page": 0, "widgetId": 6, "readOnly": read_only}),
        );
        let combo = item(&mut engine, &mut session, "c");
        assert_eq!(combo["kind"], "combo");
        assert_eq!(combo["readOnly"], read_only);
        assert_eq!(
            dict(&session, 6).get(b"Ff".as_slice()),
            Some(&Object::Integer(bits))
        );
    }
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-props", "page": 0, "widgetId": 7, "multiline": true}),
    );
    // Password (8192) stays with multiline (4096).
    assert_eq!(
        dict(&session, 7).get(b"Ff".as_slice()),
        Some(&Object::Integer(8192 | 4096))
    );
}

/// The name check walks a node's /Kids one at a time and keeps only names
/// a request can equal: a long /T over many kids, and kids repeated many
/// times, answer at once.
#[test]
fn field_name_walks_stay_bounded() {
    // Object 6 has a 64 KiB /T and 4,000 kids; 7 is a field named dup.
    let long = "n".repeat(64 * 1024);
    let kids: Vec<String> = (0..4000).map(|i| format!("{} 0 R", 9 + i)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [] >>".into(),
        "null".into(),
        "<< /Fields [6 0 R 7 0 R] /DA (/Helv 12 Tf 0 g) >>".into(),
        format!("<< /T ({long}) /Kids [{}] >>", kids.join(" ")),
        "<< /T (dup) /FT /Tx >>".into(),
        "null".into(),
    ];
    objects.extend((0..4000).map(|i| format!("<< /T (k{i}) /Parent 6 0 R >>")));
    let (mut engine, mut session) = session_with(classic(&objects));
    let started = std::time::Instant::now();
    let added = ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "text", "name": "fresh", "rect": [72, 72, 200, 96]}),
    );
    assert_eq!(added["name"], "fresh");
    let refused = op(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "text", "name": "dup", "rect": [72, 72, 200, 96]}),
    );
    assert_eq!(refused["detail"], "duplicate-name", "{refused}");
    assert!(started.elapsed().as_secs() < 10);

    // Two fields sharing one /Kids array of 200,000 entries naming one of
    // them: too many nodes, found within the visit bound.
    let shared = vec!["7 0 R"; 200_000].join(" ");
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [] >>".into(),
        "null".into(),
        "<< /Fields [6 0 R] /DA (/Helv 12 Tf 0 g) >>".into(),
        "<< /T (a) /Kids 8 0 R >>".into(),
        "<< /T (b) /Kids 8 0 R >>".into(),
        format!("[{shared}]"),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    let started = std::time::Instant::now();
    let refused = op(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "signature", "rect": [72, 72, 200, 96]}),
    );
    assert_eq!(refused["detail"], "too-many", "{refused}");
    assert!(started.elapsed().as_secs() < 10);
}

/// The view reads a list box's showing options and its /V once, and a long
/// inherited value no further than it draws, within a work budget.
#[test]
fn the_view_bounds_its_synthesis_work() {
    // 100 list boxes sharing one /Opt and /V of 20,000 strings (object 107,
    // after the widgets and /DR): quadratic work would take minutes.
    let options: Vec<String> = (0..20_000).map(|i| format!("(o{i})")).collect();
    let options = options.join(" ");
    let widgets: Vec<String> = (0..100)
        .map(|i| {
            let y = 700 - (i % 10) * 70;
            let x = 20 + (i / 10) * 55;
            format!(
                "/FT /Ch /T (l{i}) /Opt 107 0 R /V 107 0 R /Rect [{x} {y} {} {}] /DA (/Helv 12 Tf 0 g)",
                x + 50,
                y + 60
            )
        })
        .collect();
    let refs: Vec<&str> = widgets.iter().map(String::as_str).collect();
    let bytes = form_with(&refs, "", &[format!("[{options}]")]);
    let (mut engine, mut session) = session_with(bytes);
    let started = std::time::Instant::now();
    ok(
        &mut engine,
        &mut session,
        json!({"op": "render", "page": 0, "scale": 1}),
    );
    // Unbounded, this work takes minutes; bounded, about 2 to 8 s here,
    // 15 s under the gates' load (October 19), hence 30.
    assert!(started.elapsed().as_secs() < 30, "{:?}", started.elapsed());
    // Every option is selected: the first lines are highlighted.
    assert!(ink(&mut session, [20, 32, 70, 92]) > 50);

    let value = "word ".repeat(800_000);
    let kids: Vec<String> = (0..1000).map(|i| format!("{} 0 R", 7 + i)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [{}] >>",
            kids.join(" ")
        ),
        stream("", "q Q"),
        "<< /Fields [6 0 R] /DA (/Helv 12 Tf 0 g) >>".into(),
        format!(
            "<< /FT /Tx /Ff 4096 /T (big) /V ({value}) /Kids [{}] >>",
            kids.join(" ")
        ),
    ];
    objects.extend((0..1000).map(|i| {
        let x = 10 + (i % 20) * 30;
        let y = 10 + (i / 20) * 15;
        format!(
            "<< /Type /Annot /Subtype /Widget /Parent 6 0 R /Rect [{x} {y} {} {}] /DA (/Helv 12 Tf 0 g) >>",
            x + 28,
            y + 14
        )
    }));
    let (mut engine, mut session) = session_with(classic(&objects));
    let started = std::time::Instant::now();
    ok(
        &mut engine,
        &mut session,
        json!({"op": "render", "page": 0, "scale": 1}),
    );
    // Unbounded, this work takes minutes; bounded, about 2 to 8 s here,
    // 15 s under the gates' load (October 19), hence 30.
    assert!(started.elapsed().as_secs() < 30, "{:?}", started.elapsed());
}

/// The listing cuts a long value and refuses a page whose strings pass the
/// ceiling, rather than copying an inherited value into every item.
#[test]
fn the_listing_cuts_long_strings_and_refuses_huge_pages() {
    let value = "v".repeat(1 << 20);
    let listing = |count: usize| {
        let kids: Vec<String> = (0..count).map(|i| format!("{} 0 R", 7 + i)).collect();
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [{}] >>",
                kids.join(" ")
            ),
            "null".into(),
            "<< /Fields [6 0 R] /DA (/Helv 12 Tf 0 g) >>".into(),
            format!(
                "<< /FT /Tx /T ({}) /V ({value}) /Kids [{}] >>",
                "t".repeat(10_000),
                kids.join(" ")
            ),
        ];
        objects.extend((0..count).map(|_| {
            "<< /Type /Annot /Subtype /Widget /Parent 6 0 R /Rect [10 10 50 30] >>".to_string()
        }));
        let (mut engine, mut session) = session_with(classic(&objects));
        op(
            &mut engine,
            &mut session,
            json!({"op": "fields", "page": 0}),
        )
    };
    let reply = listing(100);
    let items = reply["items"].as_array().unwrap();
    assert_eq!(items.len(), 100);
    assert_eq!(items[0]["value"].as_str().unwrap().len(), 65_536);
    assert_eq!(items[0]["name"].as_str().unwrap().len(), 4096);
    let reply = listing(600);
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["reason"], "too-large", "{reply}");
}

/// Cyrillic, Greek, CJK and extended-Latin fills store an appearance in the
/// DA font and one fallback font that extracts as the value; a text no font
/// encodes gets no appearance (and no /NeedAppearances: see
/// `a_lossy_text_leaves_every_other_widget_as_it_was`), undone exactly.
#[test]
fn fills_outside_win_ansi_use_fallback_fonts_or_no_appearance() {
    let (mut engine, mut session) = session_with(hello());
    let added = ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "text", "name": "t", "rect": [72, 72, 252, 96]}),
    );
    let n = added["widgetId"].as_u64().unwrap() as u32;
    let normal = |session: &AppSession| stored_normal(doc(session), id(n));
    for value in ["Привет", "Ελληνικά", "日本語", "Łódź"] {
        ok(
            &mut engine,
            &mut session,
            json!({"op": "field-set", "page": 0, "widgetId": n, "value": value}),
        );
        let stored = normal(&session).unwrap_or_else(|| panic!("{value}: no appearance"));
        let fonts: Vec<Vec<u8>> = stored.fonts().into_keys().collect();
        assert_eq!(fonts.len(), 2, "{value}: the DA font and one fallback");
        assert!(fonts.contains(&b"Helv".to_vec()), "{value}");
        let drawn = stored.draw();
        assert_eq!(compact(&drawn.text), value, "{value}");
        assert!(!drawn.text.contains('?'), "{value}");
    }
    let acro = |session: &AppSession| {
        let catalog = doc(session)
            .resolve_id(doc(session).root())
            .as_dict()
            .unwrap()
            .clone();
        doc(session)
            .dict_get(&catalog, b"AcroForm")
            .as_dict()
            .unwrap()
            .clone()
    };
    assert!(acro(&session).get(b"NeedAppearances".as_slice()).is_none());
    let before = serialized(&session);
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-set", "page": 0, "widgetId": n, "value": "a → b"}),
    );
    assert!(normal(&session).is_none(), "no appearance holds a ?");
    assert!(acro(&session).get(b"NeedAppearances".as_slice()).is_none());
    // The view still draws the field (with ? for what no font has).
    assert!(ink(&mut session, [72, 72, 252, 96]) > 20);
    assert!(!String::from_utf8_lossy(&saved(&session)).contains("/NeedAppearances"));
    ok(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
}

/// The appearance and removal helpers work on any `Edit` target, such as
/// the scratch `Builder` a redaction or a batch file edits.
#[test]
fn helpers_work_on_a_builder() {
    use tandem_pdf_engine::app::edit::{Builder, Edit};
    use tandem_pdf_engine::app::forms::write::{EditSink, refresh};
    use tandem_pdf_engine::app::ops::fields::remove_widget;
    let bytes = form_with(
        &[
            "/FT /Tx /T (a) /V (Hello) /Rect [72 700 252 724] /DA (/Helv 12 Tf 0 g)",
            "/FT /Tx /T (b) /V (World) /Rect [72 600 252 624] /DA (/Helv 12 Tf 0 g)",
        ],
        "",
        &[],
    );
    let doc =
        Document::open_shared(std::sync::Arc::new(bytes), DocumentLimits::default(), b"").unwrap();
    let mut builder = Builder::from_document(doc);
    let mut sink = EditSink::new(&mut builder);
    assert!(refresh(&mut sink, id(6), None).unwrap());
    remove_widget(&mut builder, id(3), id(7)).unwrap();
    let doc = builder.document();
    let widget = doc.resolve_id(id(6)).as_dict().unwrap();
    assert!(doc.dict_get(widget, b"AP").as_dict().is_some());
    let page = doc.resolve_id(id(3)).as_dict().unwrap();
    let annots = doc.dict_get(page, b"Annots").as_array().unwrap();
    assert_eq!(annots, &[Object::Reference(id(6))]);
    let acro = doc.resolve_id(id(5)).as_dict().unwrap();
    let fields = doc.dict_get(acro, b"Fields").as_array().unwrap();
    assert_eq!(fields, &[Object::Reference(id(6))]);
}

/// A Common character in a CJK run, of any plane, stays in that run: each
/// text is one show in one CID font, and extracts as itself.
#[test]
fn cjk_runs_take_every_common_character() {
    for value in ["日→𠀀。", "日→。", "カタカナー"] {
        let case = json!({
            "widget": "/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)",
            "V": {"text": value},
        });
        let bytes = synthesis_pdf(&case);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let widget = doc.resolve_id(id(6)).as_dict().unwrap();
        let synth = Context::of(&doc).synthesize(widget).unwrap();
        assert!(!synth.lossy(), "{value}");
        let mut sink = Collect::default();
        let ap = appearance_dict(&mut sink, &synth).unwrap();
        let loader = |id: ObjectId| load(&doc, &sink, id);
        let forms = stored_forms(&loader, &ap, rect_of(&doc, widget));
        let shows = shows(&forms[0].ops());
        assert_eq!(shows.len(), 1, "{value}: one run, {shows:?}");
        let fonts = forms[0].fonts();
        let font = &fonts[&shows[0].font];
        assert_eq!(
            font.get(b"Subtype".as_slice()),
            Some(&Object::name("Type0")),
            "{value}"
        );
        // Big-endian UTF-16 under the predefined UTF-16 CMap (§4.5).
        let utf16: Vec<u8> = value.encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(shows[0].bytes, utf16, "{value}");
        if value.chars().all(|c| u32::from(c) <= 0xFFFF) {
            assert_eq!(compact(&forms[0].draw().text), value);
        }
    }
}

/// A character above U+FFFF, a surrogate pair under the UTF-16 CMap,
/// extracts as itself.
#[test]
fn a_character_above_the_bmp_extracts_as_itself() {
    let case = json!({
        "widget": "/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)",
        "V": {"text": "日𠀀。"},
    });
    let bytes = synthesis_pdf(&case);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let widget = doc.resolve_id(id(6)).as_dict().unwrap();
    let synth = Context::of(&doc).synthesize(widget).unwrap();
    let mut sink = Collect::default();
    let ap = appearance_dict(&mut sink, &synth).unwrap();
    let loader = |id: ObjectId| load(&doc, &sink, id);
    let forms = stored_forms(&loader, &ap, rect_of(&doc, widget));
    assert_eq!(compact(&forms[0].draw().text), "日𠀀。");
}

/// Owner decision W5: the Cyrillic layout is wider than KOI8-U, so
/// Belarusian `Ўў` draws (no `?`, a stored appearance).
#[test]
fn a_cyrillic_letter_koi8_u_lacks_draws() {
    let case = json!({
        "widget": "/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)",
        "V": {"text": "Ўў"},
    });
    let bytes = synthesis_pdf(&case);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let widget = doc.resolve_id(id(6)).as_dict().unwrap();
    let synth = Context::of(&doc).synthesize(widget).unwrap();
    assert!(!synth.lossy());
    let mut sink = Collect::default();
    let ap = appearance_dict(&mut sink, &synth).unwrap();
    let loader = |id: ObjectId| load(&doc, &sink, id);
    let forms = stored_forms(&loader, &ap, rect_of(&doc, widget));
    assert_eq!(compact(&forms[0].draw().text), "Ўў");
}

// ------------------------------------------------- acceptance review fixes

fn acro_form(session: &AppSession) -> Dictionary {
    let doc = doc(session);
    let catalog = doc.resolve_id(doc.root()).as_dict().unwrap();
    doc.dict_get(catalog, b"AcroForm")
        .as_dict()
        .unwrap()
        .clone()
}

/// A fill, an add or a redraw whose text no font encodes leaves that one
/// widget without an appearance and sets nothing document-wide: every
/// other widget keeps its stored appearance, in the view and the save.
#[test]
fn a_lossy_text_leaves_every_other_widget_as_it_was() {
    // 11 is the /DR font, 12 an empty appearance, 13 one that fills a box.
    // A synthesized appearance of c, o or q would draw their /MK border.
    let bytes = form_with(
        &[
            "/FT /Tx /T (t) /V () /Rect [72 700 252 724] /DA (/Helv 12 Tf 0 g) /MK << /BC [0] >>",
            "/FT /Btn /T (c) /V /Off /AS /Off /Rect [72 650 92 670] /DA (/ZaDb 0 Tf 0 g) \
             /MK << /BC [0] /BG [1] >> /AP << /N << /Yes 12 0 R /Off 12 0 R >> >>",
            "/FT /Tx /T (o) /V (kept) /Rect [72 600 252 624] /DA (/Helv 12 Tf 0 g) \
             /MK << /BC [0] >> /AP << /N 12 0 R >>",
            "/FT /Tx /T (p) /V <FEFF2192> /Rect [300 600 480 624] /DA (/Helv 12 Tf 0 g) \
             /AP << /N 13 0 R >>",
            "/FT /Ch /Ff 131072 /T (q) /V (one) /Opt [(one) (two)] /Rect [300 700 480 724] \
             /DA (/Helv 12 Tf 0 g) /MK << /BC [0] >> /AP << /N 12 0 R >>",
        ],
        "",
        &[
            stream("/Type /XObject /Subtype /Form /BBox [0 0 180 24]", ""),
            stream(
                "/Type /XObject /Subtype /Form /BBox [0 0 180 24]",
                "0 g 0 0 180 24 re f",
            ),
        ],
    );
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    // y down: t 68..92, c 122..142, o 168..192, p 168..192 at x 300, q 68..92 at x 300.
    let others = [[72, 122, 92, 142], [72, 168, 252, 192], [300, 68, 480, 92]];
    let blank = |session: &mut AppSession| others.map(|rect| ink(session, rect));
    assert_eq!(
        blank(&mut session),
        [0, 0, 0],
        "stored appearances draw nothing"
    );
    let kept: Vec<Dictionary> = [7, 8, 10].iter().map(|&n| dict(&session, n)).collect();
    let unchanged = |session: &AppSession| {
        assert_eq!(
            [7, 8, 10].map(|n| dict(session, n)).to_vec(),
            kept,
            "the other widgets are untouched"
        );
        assert!(
            acro_form(session)
                .get(b"NeedAppearances".as_slice())
                .is_none()
        );
        assert!(!String::from_utf8_lossy(&saved(session)).contains("NeedAppearances"));
    };
    // field-set: Vietnamese, Hebrew, an arrow and an emoji (Polish has the
    // Latin font: `latin_letters_winansi_lacks_store_an_appearance`).
    for value in ["Việt", "שלום", "a → b", "🙂"] {
        ok(
            &mut engine,
            &mut session,
            json!({"op": "field-set", "page": 0, "widgetId": 6, "value": value}),
        );
        assert_eq!(text(dict(&session, 6).get(b"V".as_slice()).unwrap()), value);
        assert!(dict(&session, 6).get(b"AP".as_slice()).is_none(), "{value}");
        unchanged(&session);
        assert_eq!(blank(&mut session), [0, 0, 0], "{value}");
        assert!(
            ink(&mut session, [72, 68, 252, 92]) > 20,
            "the view draws {value}"
        );
    }
    // field-add: a text default and a combo's options no font encodes.
    let text_added = ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "text", "name": "n",
               "default": "a → b", "rect": [72, 300, 252, 324]}),
    );
    let combo_added = ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "combo", "name": "city",
               "options": ["Việt", "Kraków"], "default": "Việt", "rect": [72, 340, 252, 364]}),
    );
    for added in [&text_added, &combo_added] {
        let n = added["widgetId"].as_u64().unwrap() as u32;
        assert!(dict(&session, n).get(b"AP".as_slice()).is_none());
    }
    unchanged(&session);
    assert_eq!(blank(&mut session), [0, 0, 0]);
    // field-props: a redraw of a stored value no font encodes.
    assert!(ink(&mut session, [300, 168, 480, 192]) > 1000);
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-props", "page": 0, "widgetId": 9, "rect": [300, 160, 480, 190]}),
    );
    assert!(dict(&session, 9).get(b"AP".as_slice()).is_none());
    unchanged(&session);
    assert_eq!(blank(&mut session), [0, 0, 0]);
    for _ in 0..7 {
        ok(&mut engine, &mut session, json!({"op": "undo"}));
    }
    assert_eq!(serialized(&session), before);
}

/// A radio group: the kid `n` (6, 7, 8) of field 5 on page 0, each with
/// `kids[n - 6]` added; 9 is an empty appearance stream.
fn radio_group(kids: [&str; 3], value: &str) -> Vec<u8> {
    let kid = |extra: &str, y: u32| {
        format!(
            "<< /Type /Annot /Subtype /Widget /Parent 5 0 R /P 3 0 R /Rect [72 {y} 88 {}] \
             /MK << /BC [0] /BG [1] >> {extra} >>",
            y + 16
        )
    };
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [6 0 R 7 0 R 8 0 R] >>"
            .into(),
        "<< /Fields [5 0 R] /DA (/ZaDb 0 Tf 0 g) >>".into(),
        format!("<< /FT /Btn /Ff 49152 /T (choice) /V {value} /Kids [6 0 R 7 0 R 8 0 R] >>"),
        kid(kids[0], 700),
        kid(kids[1], 670),
        kid(kids[2], 640),
        stream("/Type /XObject /Subtype /Form /BBox [0 0 16 16]", "q Q"),
    ])
}

fn values(engine: &mut AppEngine, session: &mut AppSession, page: usize) -> Vec<Value> {
    (items(engine, session, page).iter())
        .map(|item| item["value"].clone())
        .collect()
}

fn state(session: &AppSession, n: u32) -> Object {
    dict(session, n)
        .get(b"AS".as_slice())
        .cloned()
        .unwrap_or(Object::Null)
}

/// A radio fill turns on only the kids that have the chosen state: a kid
/// without /AP /N states is on only when it is the one filled; a check
/// box's widgets without states follow the box unless their own /AS names
/// another state.
#[test]
fn a_button_fill_turns_on_only_widgets_with_that_state() {
    // One kid without /AP among two that have one.
    let (mut engine, mut session) = session_with(radio_group(
        [
            "/AS /Off",
            "/AS /Off /AP << /N << /B 9 0 R /Off 9 0 R >> >>",
            "/AS /Off /AP << /N << /C 9 0 R /Off 9 0 R >> >>",
        ],
        "/Off",
    ));
    let original = serialized(&session);
    let set = |engine: &mut AppEngine, session: &mut AppSession, n: u32| {
        ok(
            engine,
            session,
            json!({"op": "field-set", "page": 0, "widgetId": n, "value": true}),
        );
    };
    let off = ink(&mut session, [72, 76, 88, 92]);
    set(&mut engine, &mut session, 7);
    assert_eq!(values(&mut engine, &mut session, 0), [false, true, false]);
    assert_eq!(state(&session, 6), Object::name("Off"));
    // The view still draws the kid without /AP off.
    assert_eq!(ink(&mut session, [72, 76, 88, 92]), off);
    set(&mut engine, &mut session, 8);
    assert_eq!(values(&mut engine, &mut session, 0), [false, false, true]);
    // The kid without /AP, filled, takes the on state Yes.
    set(&mut engine, &mut session, 6);
    assert_eq!(values(&mut engine, &mut session, 0), [true, false, false]);
    assert_eq!(state(&session, 6), Object::name("Yes"));
    assert_eq!(
        dict(&session, 5).get(b"V".as_slice()),
        Some(&Object::name("Yes"))
    );
    for _ in 0..3 {
        ok(&mut engine, &mut session, json!({"op": "undo"}));
    }
    assert_eq!(serialized(&session), original);

    // Every kid without /AP: still one on, and switching to another kid
    // turns the first off, though both took Yes.
    let (mut engine, mut session) = session_with(radio_group(["/AS /Off"; 3], "/Off"));
    set(&mut engine, &mut session, 7);
    assert_eq!(values(&mut engine, &mut session, 0), [false, true, false]);
    set(&mut engine, &mut session, 8);
    assert_eq!(values(&mut engine, &mut session, 0), [false, false, true]);
    assert_eq!(
        [6, 7, 8].map(|n| state(&session, n)),
        [
            Object::name("Off"),
            Object::name("Off"),
            Object::name("Yes")
        ]
    );
    // A kid whose own /AS names another state is switched off.
    let (mut engine, mut session) = session_with(radio_group(
        [
            "/AS /A",
            "/AS /Off /AP << /N << /B 9 0 R /Off 9 0 R >> >>",
            "/AS /Off /AP << /N << /C 9 0 R /Off 9 0 R >> >>",
        ],
        "/A",
    ));
    assert_eq!(values(&mut engine, &mut session, 0), [true, false, false]);
    set(&mut engine, &mut session, 7);
    assert_eq!(values(&mut engine, &mut session, 0), [false, true, false]);
    assert_eq!(state(&session, 6), Object::name("Off"));

    // The same across pages: page 0's kid without /AP, page 1's with one.
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [7 0 R] >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [8 0 R] >>".into(),
        "<< /Fields [6 0 R] /DA (/ZaDb 0 Tf 0 g) >>".into(),
        "<< /FT /Btn /Ff 49152 /T (r) /V /A /Kids [7 0 R 8 0 R] >>".into(),
        "<< /Type /Annot /Subtype /Widget /Parent 6 0 R /Rect [72 600 88 616] /AS /A >>".into(),
        "<< /Type /Annot /Subtype /Widget /Parent 6 0 R /Rect [72 600 88 616] /AS /Off \
         /AP << /N << /B 9 0 R /Off 9 0 R >> >> >>"
            .into(),
        stream("/Type /XObject /Subtype /Form /BBox [0 0 16 16]", "q Q"),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    assert_eq!(values(&mut engine, &mut session, 0), [true]);
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-set", "page": 1, "widgetId": 8, "value": true}),
    );
    assert_eq!(values(&mut engine, &mut session, 0), [false]);
    assert_eq!(values(&mut engine, &mut session, 1), [true]);
    assert_eq!(state(&session, 7), Object::name("Off"));

    // A check box with three widgets, none with /AP: two follow the box, the
    // one whose /AS names another state does not.
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [6 0 R 7 0 R 8 0 R] >>"
            .into(),
        "<< /Fields [5 0 R] /DA (/ZaDb 0 Tf 0 g) >>".into(),
        "<< /FT /Btn /T (box) /V /Off /Kids [6 0 R 7 0 R 8 0 R] >>".into(),
        "<< /Type /Annot /Subtype /Widget /Parent 5 0 R /Rect [72 700 88 716] /AS /Off >>".into(),
        "<< /Type /Annot /Subtype /Widget /Parent 5 0 R /Rect [72 670 88 686] /AS /Off >>".into(),
        "<< /Type /Annot /Subtype /Widget /Parent 5 0 R /Rect [72 640 88 656] /AS /No >>".into(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    set(&mut engine, &mut session, 6);
    assert_eq!(values(&mut engine, &mut session, 0), [true, true, false]);
    assert_eq!(
        [6, 7, 8].map(|n| state(&session, n)),
        [
            Object::name("Yes"),
            Object::name("Yes"),
            Object::name("Off")
        ]
    );
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-set", "page": 0, "widgetId": 7, "value": false}),
    );
    assert_eq!(values(&mut engine, &mut session, 0), [false, false, false]);
}

/// field-delete works on a page written directly into /Kids, as the earlier app engine
/// deletes there; undo restores it exactly.
#[test]
fn a_field_on_a_page_written_into_kids_can_be_deleted() {
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 3 0 R >>".into(),
        "<< /Type /Pages /Kids [<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Annots [4 0 R 5 0 R] >>] /Count 1 >>"
            .into(),
        "<< /Fields [4 0 R 5 0 R] /DA (/Helv 12 Tf 0 g) >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (a) /V (one) /Rect [72 700 252 724] >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (b) /V (two) /Rect [72 650 252 674] >>".into(),
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let before = serialized(&session);
    assert_eq!(items(&mut engine, &mut session, 0).len(), 2);
    ok(
        &mut engine,
        &mut session,
        json!({"op": "field-delete", "page": 0, "widgetId": 4}),
    );
    let left = items(&mut engine, &mut session, 0);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0]["name"], "b");
    let fields = acro_form(&session);
    assert_eq!(
        doc(&session).dict_get(&fields, b"Fields"),
        &Object::Array(vec![Object::Reference(id(5))])
    );
    let bytes = saved(&session);
    let (mut engine2, mut reopened) = session_with(bytes);
    let names: Vec<Value> = (items(&mut engine2, &mut reopened, 0).iter())
        .map(|item| item["name"].clone())
        .collect();
    assert_eq!(names, [json!("b")]);
    ok(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
}
