// SPDX-License-Identifier: AGPL-3.0-or-later
//! Form fields on the app session: the listing reads fields as MuPDF does,
//! fills, adds, changes and deletes write the dictionaries engine.ts writes
//! and undo exactly, appearances are MuPDF's synthesis (compared with its
//! recorded streams), and the view draws widgets without appearances.
use std::collections::BTreeMap;

use serde_json::{Value, json};
use tandem_pdf_engine::app::forms::appearance::{Context, Synth};
use tandem_pdf_engine::app::forms::write::{Sink, appearance_dict};
use tandem_pdf_engine::app::forms::{self, Kind, PageSpace};
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Dictionary, Object, ObjectId, decode_text_string};
use tandem_pdf_engine::parser::{ParseLimits, parse_complete};
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
fn the_listing_reads_every_kind_as_mupdf_does() {
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
    let normal = ap.get(b"N".as_slice()).unwrap().as_reference().unwrap();
    let content = String::from_utf8(
        doc(&session)
            .decoded_stream(normal, Default::default())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        content,
        "/Tx BMC\nq\n1 w\n1 1 218 14 re\nW\nn\nBT\n0 g\n2 4.4 Td\n/Helv 12 Tf\n(Typed text) Tj\nET\nQ\nEMC\n"
    );
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
fn add_writes_mupdfs_dictionaries_and_one_undo_step_each() {
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

/// The synthesis cases recorded from MuPDF (tools/app-forms-oracle.mjs
/// --synthesis), each widget object 6 of a one-page form.
fn synthesis_cases() -> Vec<Value> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/reference/app-forms-20261015/synthesis.json"
    );
    let data: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    data["cases"].as_array().unwrap().clone()
}

/// The case's PDF with the value MuPDF set written as /V.
fn synthesis_pdf(case: &Value) -> Vec<u8> {
    let mut widget = case["widget"].as_str().unwrap().to_string();
    if let Some(text) = case["V"]["text"].as_str() {
        // The value MuPDF set replaces the one the case starts with.
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

/// Where the Rust synthesis deliberately differs from MuPDF's stream, and why
/// (design/app-forms.md lists them).
const DIFFERENCES: [(&str, &str); 16] = [
    (
        "tx_password",
        "a password field draws asterisks, never the value",
    ),
    (
        "tx_missing_font",
        "the DA's font name stays the resource name",
    ),
    (
        "ch_combo_pairs",
        "a combo box shows the selected option's display text",
    ),
    ("ch_list", "the selected list option is highlighted"),
    ("ch_list_ti", "the selected list option is highlighted"),
    ("ch_list_long", "the selected list option is highlighted"),
    ("btn_check_red", "the check mark takes the DA colour"),
    ("btn_check_ca", "the check mark is the /MK /CA character"),
    ("btn_radio_red", "the dot takes the DA colour"),
    ("btn_radio_ca", "the mark is the /MK /CA character"),
    (
        "btn_push_bg",
        "the push button background fills the whole box",
    ),
    ("ch_list_cyr", "the selected list option is highlighted"),
    (
        "tx_cyr_euro",
        "a character the script's font lacks is drawn in a font that has it, not as a bullet",
    ),
    (
        "tx_latin_le",
        "a character WinAnsi lacks is drawn in the KOI8-U font, not as a bullet",
    ),
    (
        "tx_polish",
        "no font encodes it: lossy, so written as no appearance (MuPDF embeds a font)",
    ),
    (
        "tx_arrow",
        "no font encodes it: lossy, so written as no appearance (MuPDF embeds a font)",
    ),
];

/// The cases no font of the synthesis encodes (MuPDF embeds a font program).
const LOSSY: [&str; 2] = ["tx_polish", "tx_arrow"];

fn mupdf_streams(case: &Value) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    if !case["N"].is_null() {
        out.push(("N".into(), case["N"].clone()));
    }
    if let Some(states) = case["states"].as_object() {
        for (state, value) in states {
            out.push((format!("N/{state}"), value.clone()));
        }
    }
    if !case["D"].is_null() {
        out.push(("D".into(), case["D"].clone()));
    }
    out
}

fn rust_streams(synth: &Synth) -> Vec<(String, String, [f64; 4], i64)> {
    let row = |key: String, form: &forms::appearance::Form| {
        let content = String::from_utf8_lossy(&form.content).into_owned();
        (
            key,
            content,
            [0.0, 0.0, form.width, form.height],
            form.rotate,
        )
    };
    match synth {
        Synth::Single(form) => vec![row("N".into(), form)],
        Synth::States { off, on_name, on } => vec![
            row("N/Off".into(), off),
            row(format!("N/{}", String::from_utf8_lossy(on_name)), on),
        ],
        Synth::Push { normal, down } => vec![row("N".into(), normal), row("D".into(), down)],
    }
}

/// A content stream's tokens: numbers, literal strings decoded, the rest
/// as they are.
#[derive(Debug)]
enum Token {
    Number(f64),
    String(Vec<u8>),
    Other(String),
}

fn tokens(content: &[u8]) -> Vec<Token> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(&byte) = content.get(at) {
        match byte {
            b' ' | b'\n' | b'\r' | b'\t' => at += 1,
            b'[' | b']' => {
                out.push(Token::Other(char::from(byte).to_string()));
                at += 1;
            }
            b'<' => {
                let end = at + content[at..].iter().position(|&c| c == b'>').unwrap();
                let hex = std::str::from_utf8(&content[at + 1..end]).unwrap();
                let bytes = (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                    .collect();
                out.push(Token::String(bytes));
                at = end + 1;
            }
            b'(' => {
                let mut text = Vec::new();
                at += 1;
                while let Some(&c) = content.get(at) {
                    at += 1;
                    match c {
                        b')' => break,
                        b'\\' => {
                            let next = content[at];
                            if next.is_ascii_digit() {
                                let digits = &content[at..at + 3];
                                text.push(
                                    u8::from_str_radix(std::str::from_utf8(digits).unwrap(), 8)
                                        .unwrap(),
                                );
                                at += 3;
                            } else {
                                text.push(next);
                                at += 1;
                            }
                        }
                        c => text.push(c),
                    }
                }
                out.push(Token::String(text));
            }
            _ => {
                let start = at;
                while content
                    .get(at)
                    .is_some_and(|c| !b" \n\r\t[]()<".contains(c))
                {
                    at += 1;
                }
                let word = String::from_utf8_lossy(&content[start..at]).into_owned();
                out.push(match word.parse::<f64>() {
                    Ok(number) => Token::Number(number),
                    Err(_) => Token::Other(word),
                });
            }
        }
    }
    out
}

/// Equal streams: the same operators and strings, numbers within MuPDF's
/// single precision.
fn same_content(ours: &[u8], theirs: &[u8]) -> bool {
    let (a, b) = (tokens(ours), tokens(theirs));
    a.len() == b.len()
        && a.iter().zip(&b).all(|pair| match pair {
            (Token::Number(x), Token::Number(y)) => (x - y).abs() <= 1e-5 * x.abs().max(1.0),
            (Token::String(x), Token::String(y)) => x == y,
            (Token::Other(x), Token::Other(y)) => x == y,
            _ => false,
        })
}

#[test]
fn synthesis_matches_mupdfs_recorded_streams() {
    let mut same = 0;
    let mut compared = 0;
    let mut mismatched = Vec::new();
    let mut differing = Vec::new();
    for case in synthesis_cases() {
        let name = case["name"].as_str().unwrap();
        let bytes = synthesis_pdf(&case);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let widget = doc.resolve_id(id(6)).as_dict().unwrap();
        let synth = Context::of(&doc).synthesize(widget).unwrap();
        assert_eq!(synth.lossy(), LOSSY.contains(&name), "{name} lossy");
        let ours: BTreeMap<String, (String, [f64; 4], i64)> = rust_streams(&synth)
            .into_iter()
            .map(|(key, content, bbox, rotate)| (key, (content, bbox, rotate)))
            .collect();
        for (key, theirs) in mupdf_streams(&case) {
            compared += 1;
            let Some((content, bbox, _)) = ours.get(&key) else {
                mismatched.push(format!("{name} {key}: missing"));
                continue;
            };
            let their_bbox: Vec<f64> = theirs["bbox"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect();
            assert_eq!(bbox.to_vec(), their_bbox, "{name} {key} bbox");
            let their_content = theirs["content"].as_str().unwrap();
            // The recorded JSON holds Latin-1 bytes as characters.
            let their_bytes: Vec<u8> = their_content.chars().map(|c| c as u8).collect();
            if same_content(content.as_bytes(), &their_bytes) {
                same += 1;
            } else if DIFFERENCES.iter().any(|(case, _)| *case == name) {
                differing.push(name.to_string());
            } else {
                mismatched.push(format!(
                    "{name} {key}:\n  ours   {content:?}\n  mupdf  {their_content:?}"
                ));
            }
        }
    }
    assert!(
        mismatched.is_empty(),
        "{same}/{compared} the same\n{}",
        mismatched.join("\n")
    );
    // Every listed difference is still one.
    for (case, _) in DIFFERENCES {
        assert!(
            differing.iter().any(|name| name == case),
            "{case} now matches MuPDF"
        );
    }
    assert!(same >= 101, "{same}/{compared}");
}

/// Objects an appearance writes, kept in memory.
#[derive(Default)]
struct Collect {
    objects: BTreeMap<ObjectId, Object>,
    fonts: BTreeMap<Vec<u8>, ObjectId>,
}

impl Sink for Collect {
    type Error = ();
    fn stream(&mut self, dictionary: Dictionary, _: Vec<u8>) -> Result<ObjectId, ()> {
        self.object(Object::Dictionary(dictionary))
    }
    fn object(&mut self, value: Object) -> Result<ObjectId, ()> {
        let id = id(1000 + self.objects.len() as u32);
        self.objects.insert(id, value);
        Ok(id)
    }
    fn fonts(&mut self) -> &mut BTreeMap<Vec<u8>, ObjectId> {
        &mut self.fonts
    }
}

/// A recorded MuPDF object, parsed.
fn parsed(text: &str) -> Dictionary {
    // The recorded JSON holds Latin-1 bytes as characters.
    let bytes: Vec<u8> = text.chars().map(|c| c as u8).collect();
    let object = parse_complete(&bytes, ParseLimits::default()).unwrap();
    object.as_dict().unwrap().clone()
}

/// The Greek, Cyrillic and CID fonts are the dictionaries MuPDF writes:
/// encodings, /Differences, /Widths (within one unit: the AFM against the
/// CFF advances) and the CID font, descendant and descriptor.
#[test]
fn fallback_fonts_match_mupdfs() {
    let mut checked = 0;
    for case in synthesis_cases() {
        let name = case["name"].as_str().unwrap();
        let Some(fonts) = case["N"]["fonts"].as_object() else {
            continue;
        };
        let bytes = synthesis_pdf(&case);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let widget = doc.resolve_id(id(6)).as_dict().unwrap();
        let synth = Context::of(&doc).synthesize(widget).unwrap();
        let mut sink = Collect::default();
        let ap = appearance_dict(&mut sink, &synth).unwrap();
        let resolve = |value: &Object| match value {
            Object::Reference(id) => sink.objects.get(id).cloned(),
            value => Some(value.clone()),
        };
        let Some(Object::Dictionary(normal)) = ap.get(b"N".as_slice()).and_then(resolve) else {
            continue;
        };
        let ours = normal
            .get(b"Resources".as_slice())
            .and_then(Object::as_dict)
            .and_then(|r| r.get(b"Font".as_slice()))
            .and_then(Object::as_dict)
            .cloned()
            .unwrap_or_default();
        for (key, record) in fonts {
            let mut theirs = parsed(record["font"].as_str().unwrap());
            if theirs.get(b"Encoding".as_slice()) == Some(&Object::name("WinAnsiEncoding"))
                || LOSSY.contains(&name)
            {
                // The /DR font both name, or MuPDF's embedded font.
                continue;
            }
            let font = ours.get(key.as_bytes()).and_then(resolve);
            let Some(Object::Dictionary(mut font)) = font else {
                panic!("{name}: no font {key}");
            };
            if let Some(descendants) = record.get("descendant") {
                let descendant = font
                    .get(b"DescendantFonts".as_slice())
                    .and_then(Object::as_array)
                    .and_then(|d| d.first())
                    .and_then(resolve);
                let Some(Object::Dictionary(mut descendant)) = descendant else {
                    panic!("{name}: {key} has no descendant");
                };
                let descriptor = descendant
                    .get(b"FontDescriptor".as_slice())
                    .and_then(resolve);
                let descriptor = descriptor.and_then(|d| d.as_dict().cloned()).unwrap();
                let their_descriptor = parsed(record["descriptor"].as_str().unwrap());
                assert_eq!(descriptor, their_descriptor, "{name} {key} descriptor");
                let mut their_descendant = parsed(descendants.as_str().unwrap());
                for dict in [&mut descendant, &mut their_descendant] {
                    dict.remove(b"FontDescriptor".as_slice());
                }
                assert_eq!(descendant, their_descendant, "{name} {key} descendant");
                for dict in [&mut font, &mut theirs] {
                    dict.remove(b"DescendantFonts".as_slice());
                }
            }
            let widths = |dict: &mut Dictionary| -> Vec<f64> {
                let widths = dict.remove(b"Widths".as_slice());
                let widths = widths.as_ref().and_then(Object::as_array).unwrap_or(&[]);
                widths.iter().map(|w| w.as_f64().unwrap()).collect()
            };
            let (a, b) = (widths(&mut font), widths(&mut theirs));
            assert_eq!(a.len(), b.len(), "{name} {key} widths");
            for (code, (x, y)) in a.iter().zip(&b).enumerate() {
                assert!((x - y).abs() <= 1.0, "{name} {key} width {code}: {x} {y}");
            }
            assert_eq!(font, theirs, "{name} {key}");
            checked += 1;
        }
    }
    assert!(checked >= 30, "{checked}");
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

/// A text widget the structure tree reaches 255 levels down (MuPDF reads
/// it; the app parser allows 256): it opens, fills, renders, deletes and
/// saves, and the saved bytes open again.
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
    // engine.ts reads structure order 64 dictionaries deep at most.
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
/// the origin (`PageSpace` is single precision, as MuPDF's transform and
/// `info::page_size` are).
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
            // MuPDF's inverse transform rounds past the page here too, and
            // refuses the listed rect as engine.ts does.
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
    assert!(started.elapsed().as_secs() < 10);
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
    assert!(started.elapsed().as_secs() < 10);
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

/// Greek, Cyrillic and CJK fills write MuPDF's fallback fonts; a text no
/// font encodes gets no appearance and /NeedAppearances, undone exactly.
#[test]
fn fills_outside_win_ansi_use_fallback_fonts_or_need_appearances() {
    let (mut engine, mut session) = session_with(hello());
    let added = ok(
        &mut engine,
        &mut session,
        json!({"op": "field-add", "page": 0, "kind": "text", "name": "t", "rect": [72, 72, 252, 96]}),
    );
    let n = added["widgetId"].as_u64().unwrap() as u32;
    let normal = |session: &AppSession| {
        let widget = dict(session, n);
        let ap = doc(session).dict_get(&widget, b"AP").as_dict()?.clone();
        let normal = ap.get(b"N".as_slice())?.as_reference()?;
        Some(doc(session).stream_dict(normal).unwrap().clone())
    };
    let font_names = |stream: &Dictionary| -> Vec<Vec<u8>> {
        let resources = doc(&session).dict_get(stream, b"Resources");
        let fonts = doc(&session).dict_get(resources.as_dict().unwrap(), b"Font");
        fonts.as_dict().unwrap().keys().cloned().collect()
    };
    let _ = font_names;
    for (value, font) in [
        ("Привет", b"HelvCYR".as_slice()),
        ("Ελληνικά", b"HelvGRK"),
        ("日本語", b"Song"),
    ] {
        ok(
            &mut engine,
            &mut session,
            json!({"op": "field-set", "page": 0, "widgetId": n, "value": value}),
        );
        let stream = normal(&session).unwrap();
        let resources = doc(&session)
            .dict_get(&stream, b"Resources")
            .as_dict()
            .unwrap()
            .clone();
        let fonts = doc(&session)
            .dict_get(&resources, b"Font")
            .as_dict()
            .unwrap()
            .clone();
        assert!(fonts.get(font).is_some(), "{value}");
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
        json!({"op": "field-set", "page": 0, "widgetId": n, "value": "Łódź"}),
    );
    assert!(normal(&session).is_none(), "no appearance holds a ?");
    assert_eq!(
        acro(&session).get(b"NeedAppearances".as_slice()),
        Some(&Object::Boolean(true))
    );
    // The view still draws the field (with ? for what no font has).
    assert!(ink(&mut session, [72, 72, 252, 96]) > 20);
    assert!(String::from_utf8_lossy(&saved(&session)).contains("/NeedAppearances true"));
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

/// Where Rust is better than MuPDF's bullets and broken hex: a Common
/// character in a CJK run, any plane, is UTF-16 in the CID font; a
/// Cyrillic letter KOI8-U lacks is lossy.
#[test]
fn cjk_runs_take_every_common_character() {
    let content = |text: &str| {
        let case = json!({
            "widget": "/FT /Tx /T (x) /Rect [100 700 280 724] /DA (/Helv 12 Tf 0 g)",
            "V": {"text": text},
        });
        let bytes = synthesis_pdf(&case);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        let widget = doc.resolve_id(id(6)).as_dict().unwrap();
        let synth = Context::of(&doc).synthesize(widget).unwrap();
        let form = synth.forms()[0].clone();
        (String::from_utf8(form.content).unwrap(), synth.lossy())
    };
    let (text, lossy) = content("日→𠀀。");
    assert!(!lossy);
    assert!(
        text.contains("/Song 12 Tf\n<65e52192d840dc003002> Tj"),
        "{text}"
    );
    let (text, lossy) = content("カタカナー");
    assert!(!lossy);
    assert!(
        text.contains("/Mincho 12 Tf\n<30ab30bf30ab30ca30fc> Tj"),
        "{text}"
    );
    let (_, lossy) = content("Ўў");
    assert!(lossy);
}
