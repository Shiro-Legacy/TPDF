// SPDX-License-Identifier: MIT
//! Dynamic XFA forms materialized at open (design/app-xfa.md): the emitter
//! writes pages, fonts, the field tree and the catalog from hand-built
//! layouts; the session opens the result as an ordinary form, fills write
//! /V and the datasets packet in one undoable step, saves reopen without
//! laying out again; the whole chain runs from a small template; hostile
//! paths, packets and /XFA shapes never stop a fill.
use serde_json::{Value, json};
use tandem_pdf_engine::app::info;
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Dictionary, Object, ObjectId, decode_text_string};
use tandem_pdf_engine::view::RenderOptions;
use tandem_pdf_engine::xfa::emit::{self, OFF_KEY};
use tandem_pdf_engine::xfa::layout::{Item, ItemKind, Page};
use tandem_pdf_engine::xfa::packets::Packets;
use tandem_pdf_engine::xfa::text;
use tandem_pdf_engine::xfa::types::{
    Border, CheckShape, ChoiceOpen, DataPath, Edge, ExclGroup, Field, Fill, Font, Form,
    Item as Choice, Node, Para, Presence, Rect, Rgb, Ui, VAlign, Value as XValue,
};
use tandem_pdf_engine::xfa::{self, DATA_KEY, MARKER_KEY};

// ------------------------------------------------------------- building

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

fn hex(text: &str) -> String {
    let mut out: String = text.bytes().map(|b| format!("{b:02X}")).collect();
    out.push('>');
    out
}

const DATASETS: &str = "<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\">\
<xfa:data><form1><S><Name>Ann</Name><Agree>0</Agree><RG>b</RG><Colour>B</Colour></S></form1>\
</xfa:data></xfa:datasets>";

const METADATA: &str = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"></x:xmpmeta>";

/// A dynamic XFA source: a placeholder page, /Perms, /Names /JavaScript,
/// /Metadata, /Lang, /Extensions, and /XFA as `xfa` says (`None`: the
/// array of template, datasets (ASCIIHex-encoded) and form).
fn source(template: &str, datasets: &str, xfa: Option<&str>) -> Vec<u8> {
    let xfa = xfa.unwrap_or("[(template) 6 0 R (datasets) 7 0 R (form) 11 0 R]");
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R /NeedsRendering true \
         /Perms << /UR3 9 0 R >> /Names << /JavaScript 10 0 R >> /Metadata 8 0 R /Lang (en-CA) \
         /Extensions << /ADBE << /BaseVersion /1.7 /ExtensionLevel 8 >> >> >>"
            .into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 12 0 R >> >> >>"
            .into(),
        stream("", "BT /F1 24 Tf 72 700 Td (Please wait...) Tj ET"),
        format!(
            "<< /Fields [] /XFA {xfa} /DR << /Font << /MyF 12 0 R >> >> /DA (/MyF 0 Tf 0 g) >>"
        ),
        stream("", template),
        stream("/Filter /ASCIIHexDecode", &hex(datasets)),
        stream("/Type /Metadata /Subtype /XML", METADATA),
        "<< /Type /Sig >>".into(),
        "<< /Names [(alert) 13 0 R] >>".into(),
        stream("", "<form checksum=\"x\"/>"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        "<< /S /JavaScript /JS (app.alert\\(1\\)) >>".into(),
        // 14: the whole XDP as one stream, for the single-stream shape.
        stream(
            "",
            &format!(
                "<xdp:xdp xmlns:xdp=\"http://ns.adobe.com/xdp/\">{template}{datasets}</xdp:xdp>"
            ),
        ),
    ])
}

fn path(text: &str) -> DataPath {
    DataPath::parse(text).unwrap()
}

fn field(som: &str, data: Option<&str>, ui: Ui, value: &str) -> Field {
    let mut field = Field {
        ui,
        som: som.into(),
        data: data.map(path),
        value: if value.is_empty() {
            XValue::Empty
        } else {
            XValue::Text(value.into())
        },
        ..Field::default()
    };
    field.font.size = 10.0;
    field
}

fn check_ui() -> Ui {
    Ui::CheckButton {
        shape: CheckShape::Square,
        mark: Default::default(),
        size: 10.0,
        border: Border::none(),
    }
}

fn on_off(on: &str, off: &str) -> Vec<Choice> {
    vec![
        Choice {
            display: on.into(),
            save: on.into(),
        },
        Choice {
            display: off.into(),
            save: off.into(),
        },
    ]
}

fn item(rect: Rect, kind: ItemKind) -> Item {
    Item { rect, kind }
}

fn widget(rect: Rect, field: Field) -> Item {
    item(rect, ItemKind::Widget(Box::new(field)))
}

/// Two pages: page 1 every kind of item and every kind of widget, page 2
/// one line of text. Extra widgets join page 1.
fn layout(extra: Vec<Field>) -> (Form, Vec<Page>) {
    let mut boxed = Border::none();
    boxed.presence = Presence::Visible;
    boxed.edges = [Edge::default(); 4];
    boxed.edges[1].stroke = tandem_pdf_engine::xfa::types::Stroke::Dashed;
    boxed.fill = Some(Fill {
        presence: Presence::Visible,
        color: Rgb(240, 240, 255),
    });
    let bold = Font {
        bold: true,
        underline: true,
        ..Font::default()
    };
    let title = text::block(
        &XValue::Text("Status (Certificate) \\ \u{e9}\u{4e2d}".into()),
        &bold,
        &Para::default(),
        &|_| None,
    );

    let name = field(
        "form1[0].S[0].Name[0]",
        Some("form1[0]/S[0]/Name[0]"),
        Ui::TextEdit {
            multi_line: false,
            comb: None,
        },
        "Ann",
    );
    let mut agree = field(
        "form1[0].S[0].Agree[0]",
        Some("form1[0]/S[0]/Agree[0]"),
        check_ui(),
        "0",
    );
    agree.items = on_off("1", "0");
    let mut a = field("form1[0].S[0].RG[0].A[0]", None, check_ui(), "");
    a.items = on_off("a", "");
    a.group = Some("form1[0].S[0].RG[0]".into());
    let mut b = field("form1[0].S[0].RG[0].B[0]", None, check_ui(), "");
    b.items = on_off("b", "");
    b.group = Some("form1[0].S[0].RG[0]".into());
    b.checked = true;
    let mut colour = field(
        "form1[0].S[0].Colour[0]",
        Some("form1[0]/S[0]/Colour[0]"),
        Ui::ChoiceList {
            open: ChoiceOpen::UserControl,
            text_entry: false,
        },
        "B",
    );
    colour.items = vec![
        Choice {
            display: "Red".into(),
            save: "R".into(),
        },
        Choice {
            display: "Blue".into(),
            save: "B".into(),
        },
    ];
    colour.common.tooltip = "Pick a colour".into();

    let check_at = |x: f64, y: f64| Rect::new(x, y, 10.0, 10.0);
    let mut items = vec![
        item(
            Rect::new(36.0, 36.0, 540.0, 700.0),
            ItemKind::Border(boxed.clone()),
        ),
        item(
            Rect::new(50.0, 50.0, 400.0, 30.0),
            ItemKind::Text {
                block: title,
                v_align: VAlign::Middle,
            },
        ),
        item(
            Rect::new(50.0, 90.0, 400.0, 0.0),
            ItemKind::Line {
                slope_down: false,
                edge: Edge::default(),
            },
        ),
        item(
            Rect::new(480.0, 50.0, 40.0, 30.0),
            ItemKind::Arc {
                circular: true,
                edge: Edge::default(),
                fill: Some(Fill {
                    presence: Presence::Visible,
                    color: Rgb(255, 0, 0),
                }),
            },
        ),
        // A read-only checked box: its mark is in the content.
        item(
            check_at(50.0, 100.0),
            ItemKind::Check {
                border: Border::none(),
                checked: true,
                round: false,
            },
        ),
        widget(Rect::new(100.0, 120.0, 200.0, 20.0), name),
        item(
            check_at(100.0, 150.0),
            ItemKind::Check {
                border: Border::none(),
                checked: false,
                round: false,
            },
        ),
        widget(check_at(100.0, 150.0), agree),
        item(
            check_at(100.0, 170.0),
            ItemKind::Check {
                border: Border::none(),
                checked: false,
                round: true,
            },
        ),
        widget(check_at(100.0, 170.0), a),
        item(
            check_at(130.0, 170.0),
            ItemKind::Check {
                border: Border::none(),
                checked: true,
                round: true,
            },
        ),
        widget(check_at(130.0, 170.0), b),
        widget(Rect::new(100.0, 190.0, 200.0, 20.0), colour),
        item(
            Rect::new(300.0, 300.0, 100.0, 50.0),
            ItemKind::Rectangle(boxed),
        ),
    ];
    for (i, f) in extra.into_iter().enumerate() {
        items.push(widget(
            Rect::new(100.0, 400.0 + 30.0 * i as f64, 200.0, 20.0),
            f,
        ));
    }
    let second = text::block(
        &XValue::Text("Page two".into()),
        &Font::default(),
        &Para::default(),
        &|_| None,
    );
    let pages = vec![
        Page {
            width: 612.0,
            height: 792.0,
            items,
        },
        Page {
            width: 612.0,
            height: 792.0,
            items: vec![item(
                Rect::new(50.0, 50.0, 200.0, 20.0),
                ItemKind::Text {
                    block: second,
                    v_align: VAlign::Top,
                },
            )],
        },
    ];
    let mut form = Form::default();
    form.root.children.push(Node::ExclGroup(ExclGroup {
        som: "form1[0].S[0].RG[0]".into(),
        data: Some(path("form1[0]/S[0]/RG[0]")),
        ..ExclGroup::default()
    }));
    (form, pages)
}

fn packets_of(single: bool) -> Packets {
    if single {
        return Packets {
            whole: Some(id(14)),
            template: "<template/>".into(),
            ..Packets::default()
        };
    }
    Packets {
        template: "<template/>".into(),
        datasets: Some(DATASETS.into()),
        entries: vec![
            (b"template".to_vec(), id(6)),
            (b"datasets".to_vec(), id(7)),
            (b"form".to_vec(), id(11)),
        ],
        ..Packets::default()
    }
}

/// The materialized bytes of the hand-built layout over `datasets`.
fn materialized_with(datasets: &str, single: bool, extra: Vec<Field>) -> Vec<u8> {
    let bytes = source("<template/>", datasets, None);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let (form, pages) = layout(extra);
    emit::materialize(&doc, &packets_of(single), &form, &pages).unwrap()
}

fn materialized() -> Vec<u8> {
    materialized_with(DATASETS, false, Vec::new())
}

// ------------------------------------------------------------- reading

fn root<'d>(doc: &'d Document<'_>) -> &'d Dictionary {
    doc.resolve_id(doc.root()).as_dict().unwrap()
}

fn get<'d>(doc: &'d Document<'_>, dict: &'d Dictionary, key: &[u8]) -> &'d Object {
    doc.resolve(doc.dict_get(dict, key))
}

fn dict_of<'d>(doc: &'d Document<'_>, value: &'d Object) -> &'d Dictionary {
    doc.resolve(value).as_dict().unwrap()
}

fn text_of(value: &Object) -> String {
    decode_text_string(value.as_string().unwrap())
}

/// Every terminal field by full name, depth first.
fn fields(doc: &Document<'_>) -> Vec<(String, Dictionary)> {
    fn walk(
        doc: &Document<'_>,
        node: &Dictionary,
        prefix: &str,
        out: &mut Vec<(String, Dictionary)>,
    ) {
        let name = match node.get(b"T".as_slice()) {
            Some(t) => {
                let t = text_of(doc.resolve(t));
                if prefix.is_empty() {
                    t
                } else {
                    format!("{prefix}.{t}")
                }
            }
            None => prefix.to_string(),
        };
        let kids = doc
            .resolve(doc.dict_get(node, b"Kids"))
            .as_array()
            .unwrap_or(&[]);
        let terminal = node.contains_key(b"FT".as_slice());
        if terminal {
            out.push((name.clone(), node.clone()));
        }
        if !terminal {
            for kid in kids {
                walk(doc, dict_of(doc, kid), &name, out);
            }
        }
    }
    let acro = dict_of(doc, root(doc).get(b"AcroForm".as_slice()).unwrap());
    let mut out = Vec::new();
    for field in get(doc, acro, b"Fields").as_array().unwrap() {
        walk(doc, dict_of(doc, field), "", &mut out);
    }
    out
}

fn field_named(doc: &Document<'_>, name: &str) -> Dictionary {
    fields(doc)
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no field {name}"))
        .1
}

/// The /XFA array's entries: names and stream objects.
fn xfa_entries(doc: &Document<'_>) -> Vec<(String, ObjectId)> {
    let acro = dict_of(doc, root(doc).get(b"AcroForm".as_slice()).unwrap());
    let array = get(doc, acro, b"XFA").as_array().unwrap();
    array
        .chunks(2)
        .map(|pair| {
            (
                String::from_utf8(pair[0].as_string().unwrap().to_vec()).unwrap(),
                pair[1].as_reference().unwrap(),
            )
        })
        .collect()
}

fn datasets_text(doc: &Document<'_>) -> String {
    let (_, id) = xfa_entries(doc)
        .into_iter()
        .find(|(n, _)| n == "datasets")
        .unwrap();
    String::from_utf8(doc.decoded_stream(id, Default::default()).unwrap()).unwrap()
}

// ------------------------------------------------------------- sessions

fn session_with(bytes: Vec<u8>) -> (AppEngine, AppSession, Value) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(&mut session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
    (engine, session, reply)
}

fn ok(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    let reply = engine.dispatch(session, request.clone(), Vec::new()).0;
    assert_eq!(reply["ok"], true, "{request} -> {reply}");
    reply
}

fn listed(engine: &mut AppEngine, session: &mut AppSession, page: usize) -> Vec<Value> {
    ok(engine, session, json!({"op": "fields", "page": page}))["items"]
        .as_array()
        .unwrap()
        .clone()
}

fn listed_named(engine: &mut AppEngine, session: &mut AppSession, name: &str) -> Value {
    listed(engine, session, 0)
        .into_iter()
        .find(|i| i["name"] == name)
        .unwrap_or_else(|| panic!("no field {name}"))
}

fn doc(session: &AppSession) -> &Document<'static> {
    session.doc.as_ref().unwrap().document()
}

fn fill(engine: &mut AppEngine, session: &mut AppSession, widget: &Value, value: Value) {
    ok(
        engine,
        session,
        json!({"op": "field-set", "page": 0, "widgetId": widget["id"], "value": value}),
    );
}

fn save(engine: &mut AppEngine, session: &mut AppSession) -> Vec<u8> {
    let (reply, mut outputs) = engine.dispatch(session, json!({"op": "save"}), Vec::new());
    assert_eq!(reply["ok"], true, "{reply}");
    outputs.pop().unwrap()
}

fn dark_pixels(session: &mut AppSession, page: usize) -> usize {
    let view = session.view().unwrap();
    let image = view.render(page, 1.0, RenderOptions::default()).unwrap();
    image
        .pixels
        .chunks(4)
        .filter(|p| p[..3].iter().map(|&c| u32::from(c)).sum::<u32>() < 600)
        .count()
}

// ---------------------------------------------------------------- emitter

#[test]
fn the_emitter_writes_pages_fonts_and_content() {
    let bytes = materialized();
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let pages = info::pages(&doc);
    assert_eq!(pages.len(), 2);
    let fonts = |page: usize| -> Vec<(String, String)> {
        let resources = get(&doc, pages[page].dict, b"Resources").as_dict().unwrap();
        let fonts = get(&doc, resources, b"Font").as_dict().unwrap();
        fonts
            .iter()
            .map(|(k, v)| {
                let base = get(&doc, dict_of(&doc, v), b"BaseFont")
                    .as_name()
                    .unwrap()
                    .to_vec();
                (
                    String::from_utf8(k.clone()).unwrap(),
                    String::from_utf8(base).unwrap(),
                )
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    };
    assert_eq!(
        fonts(0),
        [
            ("XF14".to_string(), "ZapfDingbats".to_string()),
            ("XF2".into(), "Helvetica-Bold".into())
        ],
        "the bold title and the static check mark"
    );
    assert_eq!(fonts(1), [("XF1".to_string(), "Helvetica".to_string())]);
    // One font object per face: page 2's Helvetica is /DR's /Helv.
    let acro = dict_of(&doc, root(&doc).get(b"AcroForm".as_slice()).unwrap());
    let dr_font = get(&doc, get(&doc, acro, b"DR").as_dict().unwrap(), b"Font")
        .as_dict()
        .unwrap();
    let page2 = get(
        &doc,
        get(&doc, pages[1].dict, b"Resources").as_dict().unwrap(),
        b"Font",
    )
    .as_dict()
    .unwrap();
    assert_eq!(
        dr_font.get(b"Helv".as_slice()),
        page2.get(b"XF1".as_slice())
    );
    assert!(dr_font.contains_key(b"ZaDb".as_slice()) && dr_font.contains_key(b"MyF".as_slice()));

    let contents = pages[0]
        .dict
        .get(b"Contents".as_slice())
        .and_then(Object::as_reference)
        .unwrap();
    let content =
        String::from_utf8(doc.decoded_stream(contents, Default::default()).unwrap()).unwrap();
    // The border: fill, then edges inside the box (y-up: 792 - 36 - 700 = 56).
    assert!(
        content.contains("0.941 0.941 1 rg 36 56 540 700 re f"),
        "{content}"
    );
    assert!(content.contains("[3 3] 0 d"), "the dashed right edge");
    assert!(
        content.contains("36 755.75 m 576 755.75 l S"),
        "the top edge inside the box: {content}"
    );
    // The title: clipped, escaped, WinAnsi with ? for what it lacks, underlined.
    assert!(content.contains("50 712 400 30 re W n"), "{content}");
    assert!(
        content.contains("(Status \\(Certificate\\) \\\\ \\351?) Tj"),
        "{content}"
    );
    assert!(content.contains("re f\n"), "the underline");
    // The arc, the static mark, no mark for the fillable boxes.
    assert!(content.contains(" c h B Q"), "{content}");
    assert_eq!(content.matches("(4) Tj").count(), 1, "{content}");
    assert!(content.contains("/XF14 8 Tf"), "the mark at 0.8 of the box");
}

#[test]
fn the_emitter_writes_the_field_tree_and_the_catalog() {
    let bytes = materialized();
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let names: Vec<String> = fields(&doc).into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        [
            "form1[0].S[0].Name[0]",
            "form1[0].S[0].Agree[0]",
            "form1[0].S[0].RG[0]",
            "form1[0].S[0].Colour[0]"
        ]
    );
    let acro = dict_of(&doc, root(&doc).get(b"AcroForm".as_slice()).unwrap());
    assert_eq!(
        get(&doc, acro, b"Fields").as_array().unwrap().len(),
        1,
        "one root, form1[0]"
    );
    assert_eq!(get(&doc, acro, b"NeedAppearances").as_bool(), Some(true));

    let name = field_named(&doc, "form1[0].S[0].Name[0]");
    assert_eq!(get(&doc, &name, b"FT").as_name(), Some(b"Tx".as_slice()));
    assert_eq!(text_of(get(&doc, &name, b"V")), "Ann");
    assert_eq!(text_of(get(&doc, &name, DATA_KEY)), "form1[0]/S[0]/Name[0]");
    assert_eq!(
        get(&doc, &name, b"Subtype").as_name(),
        Some(b"Widget".as_slice()),
        "merged with its widget"
    );
    assert_eq!(get(&doc, &name, b"Rect").as_array().unwrap().len(), 4);

    let agree = field_named(&doc, "form1[0].S[0].Agree[0]");
    assert_eq!(get(&doc, &agree, b"FT").as_name(), Some(b"Btn".as_slice()));
    assert_eq!(get(&doc, &agree, b"V").as_name(), Some(b"Off".as_slice()));
    assert_eq!(get(&doc, &agree, b"AS").as_name(), Some(b"Off".as_slice()));
    assert_eq!(text_of(get(&doc, &agree, OFF_KEY)), "0");
    let states = get(&doc, get(&doc, &agree, b"AP").as_dict().unwrap(), b"N")
        .as_dict()
        .unwrap();
    let mut keys: Vec<&[u8]> = states.keys().map(Vec::as_slice).collect();
    keys.sort();
    assert_eq!(
        keys,
        [b"1".as_slice(), b"Off"],
        "the on state is the on save value"
    );

    let group = field_named(&doc, "form1[0].S[0].RG[0]");
    assert_eq!(
        get(&doc, &group, b"Ff").as_i64().unwrap() & (1 << 15),
        1 << 15
    );
    assert_eq!(get(&doc, &group, b"V").as_name(), Some(b"b".as_slice()));
    assert_eq!(text_of(get(&doc, &group, DATA_KEY)), "form1[0]/S[0]/RG[0]");
    let kids = get(&doc, &group, b"Kids").as_array().unwrap();
    let shown: Vec<Vec<u8>> = kids
        .iter()
        .map(|k| {
            get(&doc, dict_of(&doc, k), b"AS")
                .as_name()
                .unwrap()
                .to_vec()
        })
        .collect();
    assert_eq!(shown, [b"Off".to_vec(), b"b".to_vec()]);

    let colour = field_named(&doc, "form1[0].S[0].Colour[0]");
    assert_eq!(get(&doc, &colour, b"FT").as_name(), Some(b"Ch".as_slice()));
    assert_eq!(get(&doc, &colour, b"Ff").as_i64(), Some(1 << 17));
    assert_eq!(text_of(get(&doc, &colour, b"V")), "B");
    assert_eq!(text_of(get(&doc, &colour, b"TU")), "Pick a colour");
    let opt: Vec<Vec<String>> = get(&doc, &colour, b"Opt")
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| pair.as_array().unwrap().iter().map(text_of).collect())
        .collect();
    assert_eq!(opt, [["R", "Red"], ["B", "Blue"]]);
    assert!(
        get(&doc, &colour, b"DA")
            .as_string()
            .unwrap()
            .starts_with(b"/Helv 10 Tf")
    );

    // The catalog: the marker, kept and dropped entries.
    let catalog = root(&doc);
    assert_eq!(
        get(&doc, catalog, MARKER_KEY).as_i64(),
        Some(xfa::MARKER_VERSION)
    );
    assert_eq!(get(&doc, catalog, b"NeedsRendering").as_bool(), Some(true));
    for gone in [&b"Perms"[..], b"Names", b"OpenAction", b"AA"] {
        assert!(
            !catalog.contains_key(gone),
            "{}",
            String::from_utf8_lossy(gone)
        );
    }
    assert_eq!(text_of(get(&doc, catalog, b"Lang")), "en-CA");
    let adbe = get(
        &doc,
        get(&doc, catalog, b"Extensions").as_dict().unwrap(),
        b"ADBE",
    )
    .as_dict()
    .unwrap();
    assert_eq!(get(&doc, adbe, b"ExtensionLevel").as_i64(), Some(8));
    let metadata = catalog
        .get(b"Metadata".as_slice())
        .unwrap()
        .as_reference()
        .unwrap();
    assert_eq!(
        doc.decoded_stream(metadata, Default::default()).unwrap(),
        METADATA.as_bytes()
    );
    // /XFA keeps template and datasets (raw, its filter kept), drops form.
    let entries = xfa_entries(&doc);
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["template", "datasets"]);
    let datasets = doc.stream_dict(entries[1].1).unwrap();
    assert_eq!(
        datasets.get(b"Filter".as_slice()),
        Some(&Object::name("ASCIIHexDecode"))
    );
    assert_eq!(datasets_text(&doc), DATASETS);
    assert!(!xfa::is_dynamic(&doc), "the marker stops a second layout");
}

#[test]
fn the_emitter_refuses_no_pages_and_flattens_deep_names() {
    let bytes = source("<template/>", DATASETS, None);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let error = emit::materialize(&doc, &packets_of(false), &Form::default(), &[]).unwrap_err();
    assert!(matches!(error, xfa::XfaError::Write(_)), "{error}");

    let deep = "a[0].b[0].c[0].d[0].e[0].f[0].g[0].h[0].i[0].j[0]";
    let bytes = materialized_with(
        DATASETS,
        false,
        vec![
            field(
                deep,
                None,
                Ui::TextEdit {
                    multi_line: true,
                    comb: None,
                },
                "",
            ),
            {
                let mut comb = field(
                    "form1[0].S[0].Comb[0]",
                    None,
                    Ui::TextEdit {
                        multi_line: false,
                        comb: Some(6),
                    },
                    "",
                );
                comb.max_chars = 6;
                comb
            },
        ],
    );
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let all = fields(&doc);
    let (name, dict) = all.iter().find(|(n, _)| n.ends_with("j[0]")).unwrap();
    assert_eq!(name, "a[0]_b[0]_c[0].d[0].e[0].f[0].g[0].h[0].i[0].j[0]");
    assert_eq!(get(&doc, dict, b"Ff").as_i64(), Some(1 << 12));
    let comb = field_named(&doc, "form1[0].S[0].Comb[0]");
    assert_eq!(get(&doc, &comb, b"Ff").as_i64(), Some(1 << 24));
    assert_eq!(get(&doc, &comb, b"MaxLen").as_i64(), Some(6));
}

// ---------------------------------------------------------------- session

#[test]
fn a_materialized_form_opens_fills_syncs_undoes_and_saves() {
    let (mut engine, mut session, reply) = session_with(materialized());
    assert_eq!(reply["info"]["xfaRendered"], true, "{reply}");
    assert_eq!(reply["info"]["xfa"], true);
    let kinds: Vec<String> = listed(&mut engine, &mut session, 0)
        .iter()
        .map(|i| {
            format!(
                "{}:{}:{}",
                i["name"].as_str().unwrap(),
                i["kind"].as_str().unwrap(),
                i["value"]
            )
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "form1[0].S[0].Name[0]:text:\"Ann\"",
            "form1[0].S[0].Agree[0]:checkbox:false",
            "form1[0].S[0].RG[0]:radio:false",
            "form1[0].S[0].RG[0]:radio:true",
            "form1[0].S[0].Colour[0]:combo:\"B\"",
        ]
    );
    assert!(dark_pixels(&mut session, 0) > 500, "page 1 is drawn");
    assert!(dark_pixels(&mut session, 1) > 10, "page 2 is drawn");

    let name = listed_named(&mut engine, &mut session, "form1[0].S[0].Name[0]");
    fill(&mut engine, &mut session, &name, json!("Bob & Co"));
    let agree = listed_named(&mut engine, &mut session, "form1[0].S[0].Agree[0]");
    assert_eq!(agree["on"], "1");
    fill(&mut engine, &mut session, &agree, json!(true));
    {
        let doc = doc(&session);
        assert_eq!(
            text_of(get(doc, &field_named(doc, "form1[0].S[0].Name[0]"), b"V")),
            "Bob & Co"
        );
        let agree = field_named(doc, "form1[0].S[0].Agree[0]");
        assert_eq!(get(doc, &agree, b"V").as_name(), Some(b"1".as_slice()));
        let data = datasets_text(doc);
        assert!(data.contains("<Name>Bob &amp; Co</Name>"), "{data}");
        assert!(data.contains("<Agree>1</Agree>"), "{data}");
        assert!(
            !data.starts_with("<?xml"),
            "a packet in the middle of the XDP has no declaration"
        );
    }
    // Off writes the off save value.
    fill(&mut engine, &mut session, &agree, json!(false));
    assert!(datasets_text(doc(&session)).contains("<Agree>0</Agree>"));
    fill(&mut engine, &mut session, &agree, json!(true));
    // A radio button writes its group's value.
    let radios: Vec<Value> = listed(&mut engine, &mut session, 0)
        .into_iter()
        .filter(|i| i["kind"] == "radio")
        .collect();
    assert_eq!(radios[0]["on"], "a");
    fill(&mut engine, &mut session, &radios[0], json!(true));
    assert!(datasets_text(doc(&session)).contains("<RG>a</RG>"));
    let colour = listed_named(&mut engine, &mut session, "form1[0].S[0].Colour[0]");
    fill(&mut engine, &mut session, &colour, json!("R"));
    assert!(datasets_text(doc(&session)).contains("<Colour>R</Colour>"));

    // Undo restores /V and the packet together.
    for _ in 0..4 {
        ok(&mut engine, &mut session, json!({"op": "undo"}));
    }
    {
        let doc = doc(&session);
        assert_eq!(
            text_of(get(doc, &field_named(doc, "form1[0].S[0].Name[0]"), b"V")),
            "Bob & Co"
        );
        assert_eq!(
            get(doc, &field_named(doc, "form1[0].S[0].Agree[0]"), b"V").as_name(),
            Some(b"1".as_slice())
        );
        let data = datasets_text(doc);
        assert!(
            data.contains("<Agree>1</Agree>") && data.contains("<RG>b</RG>"),
            "{data}"
        );
    }
    ok(&mut engine, &mut session, json!({"op": "undo"}));
    ok(&mut engine, &mut session, json!({"op": "undo"}));
    {
        let doc = doc(&session);
        assert_eq!(
            text_of(get(doc, &field_named(doc, "form1[0].S[0].Name[0]"), b"V")),
            "Ann"
        );
        assert_eq!(
            get(doc, &field_named(doc, "form1[0].S[0].Agree[0]"), b"V").as_name(),
            Some(b"Off".as_slice())
        );
        assert_eq!(datasets_text(doc), DATASETS);
    }
    ok(&mut engine, &mut session, json!({"op": "redo"}));
    assert!(datasets_text(doc(&session)).contains("<Name>Bob &amp; Co</Name>"));

    // Save, reopen: still ours, values kept, laid out once.
    let saved = save(&mut engine, &mut session);
    let (mut engine, mut session, reply) = session_with(saved.clone());
    assert_eq!(reply["info"]["xfaRendered"], true);
    assert_eq!(reply["pageCount"], 2);
    let name = listed_named(&mut engine, &mut session, "form1[0].S[0].Name[0]");
    assert_eq!(name["value"], "Bob & Co");
    assert_eq!(
        session.doc.as_ref().unwrap().input().as_slice(),
        saved.as_slice(),
        "not materialized again"
    );
    assert!(datasets_text(doc(&session)).contains("<Name>Bob &amp; Co</Name>"));
}

// ---------------------------------------------------------------- hostile

#[test]
fn hostile_paths_packets_and_shapes_never_stop_a_fill() {
    let missing = field(
        "form1[0].S[0].New[0]",
        Some("form1[0]/New[0]/X[2]"),
        Ui::TextEdit {
            multi_line: false,
            comb: None,
        },
        "",
    );
    let huge = field(
        "form1[0].S[0].Huge[0]",
        Some("form1[0]/H[999999999]"),
        Ui::TextEdit {
            multi_line: false,
            comb: None,
        },
        "",
    );
    let (mut engine, mut session, _) =
        session_with(materialized_with(DATASETS, false, vec![missing, huge]));
    let new = listed_named(&mut engine, &mut session, "form1[0].S[0].New[0]");
    fill(&mut engine, &mut session, &new, json!("made"));
    let data = datasets_text(doc(&session));
    assert!(
        data.contains("<New><X/><X/><X>made</X></New>"),
        "a missing path is made: {data}"
    );
    let huge = listed_named(&mut engine, &mut session, "form1[0].S[0].Huge[0]");
    fill(&mut engine, &mut session, &huge, json!("big"));
    assert_eq!(
        datasets_text(doc(&session)),
        data,
        "a huge index is skipped"
    );
    assert_eq!(
        listed_named(&mut engine, &mut session, "form1[0].S[0].Huge[0]")["value"],
        "big"
    );

    // A datasets packet that is not XML: the fill stands, the packet as it was.
    let (mut engine, mut session, _) =
        session_with(materialized_with("not <xml", false, Vec::new()));
    let name = listed_named(&mut engine, &mut session, "form1[0].S[0].Name[0]");
    fill(&mut engine, &mut session, &name, json!("x"));
    assert_eq!(
        listed_named(&mut engine, &mut session, "form1[0].S[0].Name[0]")["value"],
        "x"
    );
    assert_eq!(datasets_text(doc(&session)), "not <xml");

    // /XFA as one stream: kept as it is, the fill stands.
    let (mut engine, mut session, reply) =
        session_with(materialized_with(DATASETS, true, Vec::new()));
    assert_eq!(reply["info"]["xfaRendered"], true);
    {
        let doc = doc(&session);
        let acro = dict_of(doc, root(doc).get(b"AcroForm".as_slice()).unwrap());
        let whole = acro.get(b"XFA".as_slice()).unwrap().as_reference().unwrap();
        let text =
            String::from_utf8(doc.decoded_stream(whole, Default::default()).unwrap()).unwrap();
        assert!(text.starts_with("<xdp:xdp") && text.contains("<Name>Ann</Name>"));
    }
    let name = listed_named(&mut engine, &mut session, "form1[0].S[0].Name[0]");
    fill(&mut engine, &mut session, &name, json!("y"));
    assert_eq!(
        listed_named(&mut engine, &mut session, "form1[0].S[0].Name[0]")["value"],
        "y"
    );
}

// ---------------------------------------------------------------- end to end

const TEMPLATE: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/2.8/">
<subform name="form1" layout="tb" locale="en_US">
  <pageSet>
    <pageArea name="Page1" id="Page1">
      <contentArea x="0.25in" y="0.25in" w="8in" h="10.5in"/>
      <medium stock="letter" short="8.5in" long="11in"/>
    </pageArea>
  </pageSet>
  <subform name="S" layout="tb" w="8in">
    <draw name="Title" w="8in" h="24pt">
      <value><exData contentType="text/html"><body xmlns="http://www.w3.org/1999/xhtml"><p>Status <span style="font-weight:bold">Certificate</span></p></body></exData></value>
      <font typeface="Arial" size="14pt"/>
    </draw>
    <field name="Name" w="4in" h="9mm">
      <ui><textEdit/></ui>
      <caption reserve="1in"><value><text>Name</text></value></caption>
      <border><edge/></border>
    </field>
    <field name="City" w="4in" h="9mm">
      <ui><textEdit multiLine="1"/></ui>
      <caption reserve="1in"><value><text>City</text></value></caption>
    </field>
    <field name="Agree" w="3in" h="9mm">
      <ui><checkButton size="10pt"/></ui>
      <items><integer>1</integer><integer>0</integer></items>
      <caption placement="right" reserve="2in"><value><text>I agree</text></value></caption>
    </field>
    <field name="Colour" w="4in" h="9mm">
      <ui><choiceList/></ui>
      <items><text>Red</text><text>Blue</text></items>
      <items save="1" presence="hidden"><text>R</text><text>B</text></items>
    </field>
  </subform>
</subform>
</template>"#;

const E2E_DATASETS: &str = "<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\">\
<xfa:data><form1><S><Name>Ann</Name><City/><Agree>0</Agree><Colour>B</Colour></S></form1></xfa:data>\
</xfa:datasets>";

#[test]
fn a_dynamic_form_is_materialized_at_open_and_fills_through() {
    let bytes = source(TEMPLATE, E2E_DATASETS, None);
    {
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        assert!(xfa::is_dynamic(&doc));
        let out = xfa::materialize(&doc).unwrap();
        let out = Document::open(&out, DocumentLimits::default()).unwrap();
        assert!(!xfa::is_dynamic(&out));
    }
    let (mut engine, mut session, reply) = session_with(bytes);
    assert_eq!(reply["info"]["xfaRendered"], true, "{reply}");
    let items = listed(&mut engine, &mut session, 0);
    let by = |suffix: &str| {
        items
            .iter()
            .find(|i| i["name"].as_str().unwrap().ends_with(suffix))
            .cloned()
            .unwrap()
    };
    assert_eq!(by("Name[0]")["kind"], "text");
    assert_eq!(by("Name[0]")["value"], "Ann");
    assert_eq!(by("City[0]")["multiline"], true);
    assert_eq!(by("Agree[0]")["kind"], "checkbox");
    assert_eq!(by("Agree[0]")["value"], false);
    assert_eq!(by("Colour[0]")["kind"], "combo");
    assert_eq!(by("Colour[0]")["value"], "B");
    // The placeholder is gone: the page shows the form's own text.
    let text = ok(&mut engine, &mut session, json!({"op": "text-all"}));
    let shown = text["text"].as_str().unwrap();
    assert!(!shown.contains("Please wait"), "{shown}");
    assert!(
        shown.contains("Status") && shown.contains("Certificate") && shown.contains("I agree"),
        "{shown}"
    );
    assert!(dark_pixels(&mut session, 0) > 100);

    fill(&mut engine, &mut session, &by("Name[0]"), json!("Zed"));
    fill(&mut engine, &mut session, &by("Agree[0]"), json!(true));
    let data = datasets_text(doc(&session));
    assert!(
        data.contains("<Name>Zed</Name>") && data.contains("<Agree>1</Agree>"),
        "{data}"
    );
    let catalog = root(doc(&session));
    assert!(
        !catalog.contains_key(b"Perms".as_slice()) && !catalog.contains_key(b"Names".as_slice())
    );
}

#[test]
fn a_form_that_does_not_lay_out_opens_as_it_was() {
    // A template that is not XML: the placeholder and the original stay.
    let bytes = source("<template", E2E_DATASETS, None);
    let (_, session, reply) = session_with(bytes.clone());
    assert_eq!(reply["info"]["xfaRendered"], false);
    assert_eq!(reply["info"]["xfa"], true);
    assert_eq!(
        session.doc.as_ref().unwrap().input().as_slice(),
        bytes.as_slice()
    );
}

#[test]
fn a_page_master_field_on_every_page_is_one_field_with_a_widget_per_page() {
    let bytes = source("<template/>", DATASETS, None);
    let source_doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let (form, mut pages) = layout(Vec::new());
    let footer = field(
        "form1[0].Footer[0].Ref[0]",
        Some("form1[0]/Ref[0]"),
        Ui::TextEdit {
            multi_line: false,
            comb: None,
        },
        "R-1",
    );
    for page in &mut pages {
        page.items
            .push(widget(Rect::new(400.0, 750.0, 150.0, 20.0), footer.clone()));
    }
    let out = emit::materialize(&source_doc, &packets_of(false), &form, &pages).unwrap();
    let made = Document::open(&out, DocumentLimits::default()).unwrap();
    let doc = &made;
    let all = fields(doc);
    let matching: Vec<&(String, Dictionary)> = all
        .iter()
        .filter(|(n, _)| n == "form1[0].Footer[0].Ref[0]")
        .collect();
    assert_eq!(matching.len(), 1, "one field, no duplicate /T");
    let field = &matching[0].1;
    assert_eq!(text_of(get(doc, field, b"V")), "R-1");
    assert_eq!(text_of(get(doc, field, DATA_KEY)), "form1[0]/Ref[0]");
    assert!(
        !field.contains_key(b"Subtype".as_slice()),
        "not merged with a widget"
    );
    let kids = get(doc, field, b"Kids").as_array().unwrap();
    assert_eq!(kids.len(), 2);
    let page_ids: Vec<ObjectId> = info::pages(doc).iter().map(|p| p.id.unwrap()).collect();
    for (kid, page) in kids.iter().zip(&page_ids) {
        let widget = dict_of(doc, kid);
        assert!(!widget.contains_key(b"T".as_slice()));
        assert_eq!(
            get(doc, widget, b"Subtype").as_name(),
            Some(b"Widget".as_slice())
        );
        assert!(widget.get(b"Parent".as_slice()).is_some());
        assert_eq!(
            widget.get(b"P".as_slice()).and_then(Object::as_reference),
            Some(*page)
        );
        let annots = get(
            doc,
            info::pages(doc)[page_ids.iter().position(|p| p == page).unwrap()].dict,
            b"Annots",
        );
        assert!(annots.as_array().unwrap().contains(kid));
    }

    // A fill on page 1 is the field's value on both pages, and in the data.
    let (mut engine, mut session, _) = session_with(out);
    let on_page = |engine: &mut AppEngine, session: &mut AppSession, page: usize| {
        listed(engine, session, page)
            .into_iter()
            .find(|i| i["name"] == "form1[0].Footer[0].Ref[0]")
            .unwrap()
    };
    let first = on_page(&mut engine, &mut session, 0);
    fill(&mut engine, &mut session, &first, json!("R-2"));
    assert_eq!(on_page(&mut engine, &mut session, 1)["value"], "R-2");
    assert!(datasets_text(session_doc(&session)).contains("<Ref>R-2</Ref>"));
}

fn session_doc(session: &AppSession) -> &Document<'static> {
    doc(session)
}

// ------------------------------------------------------------------ scripts

/// A form whose scripts show and hide sections: `initialize` hides `B`
/// unless the kind is `b`; a `change` on the kind group hides one of `A`
/// and `B` and shows the other, and writes a floating field.
const SCRIPTED_TEMPLATE: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.6/">
<subform name="form1" layout="tb" locale="en_US">
  <pageSet>
    <pageArea name="Page1" id="Page1">
      <contentArea x="0.25in" y="0.25in" w="8in" h="10.5in"/>
      <medium stock="letter" short="8.5in" long="11in"/>
    </pageArea>
  </pageSet>
  <variables>
    <script name="sco" contentType="application/x-javascript">
      function showA() { S.A.presence = "visible"; S.B.presence = "hidden"; Label.value = "alpha"; }
      function showB() { S.A.presence = "hidden"; S.B.presence = "visible"; Label.value = "beta"; }
    </script>
    <text name="Label">none</text>
  </variables>
  <subform name="S" layout="tb" w="8in">
    <field name="Name" w="4in" h="9mm">
      <ui><textEdit/></ui>
      <caption reserve="1in"><value><text>Name</text></value></caption>
    </field>
    <exclGroup name="Kind" layout="tb">
      <event activity="change"><script contentType="application/x-javascript">
        switch (this.rawValue) {
          case "a":
            sco.showA();
            break;
          case "b":
            sco.showB();
            break;
          default:
        }
        Tag.rawValue = Label.value;
      </script></event>
      <field name="KA" w="2in" h="9mm"><ui><checkButton/></ui><items><text>a</text><text>0</text></items>
        <caption placement="right"><value><text>Kind A</text></value></caption></field>
      <field name="KB" w="2in" h="9mm"><ui><checkButton/></ui><items><text>b</text><text>0</text></items>
        <caption placement="right"><value><text>Kind B</text></value></caption></field>
    </exclGroup>
    <field name="Tag" w="2in" h="9mm" access="readOnly" id="tag1"><ui><textEdit/></ui></field>
    <subform name="A" layout="tb" w="8in">
      <draw name="TA" w="8in" h="12pt"><value><text>Section Alpha</text></value></draw>
      <field name="FA" w="4in" h="9mm"><ui><textEdit/></ui></field>
    </subform>
    <subform name="B" layout="tb" w="8in" presence="hidden">
      <event activity="initialize"><script contentType="application/x-javascript">
        if (Kind.rawValue == "b") { this.presence = "visible"; A.presence = "hidden"; }
      </script></event>
      <draw name="TB" w="8in" h="12pt"><value><text>Section Beta</text></value></draw>
      <field name="FB" w="4in" h="9mm"><ui><textEdit/></ui></field>
    </subform>
  </subform>
</subform>
</template>"#;

const SCRIPTED_DATASETS: &str = "<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\">\
<xfa:data><form1><S><Name>Ann</Name><Kind/><Tag/><A><FA/></A><B><FB/></B></S></form1></xfa:data>\
</xfa:datasets>";

fn names(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .map(|i| {
            i["name"]
                .as_str()
                .unwrap()
                .rsplit('.')
                .next()
                .unwrap()
                .to_string()
        })
        .collect()
}

fn ids(items: &[Value]) -> Vec<i64> {
    items.iter().map(|i| i["id"].as_i64().unwrap()).collect()
}

#[test]
fn a_choice_reveals_the_section_its_script_shows() {
    let (mut engine, mut session, reply) =
        session_with(source(SCRIPTED_TEMPLATE, SCRIPTED_DATASETS, None));
    assert_eq!(reply["info"]["xfaRendered"], true, "{reply}");
    let at_open = listed(&mut engine, &mut session, 0);
    assert_eq!(
        names(&at_open),
        ["Name[0]", "Kind[0]", "Kind[0]", "FA[0]"],
        "B is hidden at open; Tag is read-only, so it has no widget"
    );
    let text = ok(&mut engine, &mut session, json!({"op": "text-all"}));
    let shown = text["text"].as_str().unwrap();
    assert!(
        shown.contains("Section Alpha") && !shown.contains("Section Beta"),
        "{shown}"
    );
    assert!(!shown.contains("beta"), "{shown}");
    let open_ids = ids(&at_open);
    let catalog = root(doc(&session));
    assert!(
        catalog.contains_key(b"TPDFXfaSig".as_slice()),
        "the signature is stored"
    );
    let open_signature = catalog.get(b"TPDFXfaSig".as_slice()).cloned();

    // A text fill changes no scripted state: the widgets stay as they are.
    fill(&mut engine, &mut session, &at_open[0], json!("Zed"));
    let after_name = listed(&mut engine, &mut session, 0);
    assert_eq!(
        ids(&after_name),
        open_ids,
        "same widgets after a plain fill"
    );
    assert_eq!(after_name[0]["value"], "Zed");
    assert!(datasets_text(doc(&session)).contains("<Name>Zed</Name>"));

    // Choosing kind B: B appears, A goes, the floating field is written,
    // the datasets carry both the choice and the script's value.
    let kb = at_open.iter().find(|i| i["on"] == "b").unwrap().clone();
    fill(&mut engine, &mut session, &kb, json!(true));
    let after_b = listed(&mut engine, &mut session, 0);
    assert_eq!(
        names(&after_b),
        ["Name[0]", "Kind[0]", "Kind[0]", "FB[0]"],
        "{after_b:?}"
    );
    let by = |items: &[Value], suffix: &str| {
        items
            .iter()
            .find(|i| i["name"].as_str().unwrap().ends_with(suffix))
            .cloned()
            .unwrap()
    };
    let chosen = |items: &[Value]| items.iter().find(|i| i["on"] == "b").unwrap().clone();
    assert_eq!(chosen(&after_b)["value"], true);
    assert_eq!(
        by(&after_b, "Name[0]")["value"],
        "Zed",
        "the earlier fill survives"
    );
    let text = ok(&mut engine, &mut session, json!({"op": "text-all"}));
    let shown = text["text"].as_str().unwrap();
    assert!(
        !shown.contains("Section Alpha") && shown.contains("Section Beta"),
        "{shown}"
    );
    assert!(
        shown.contains("beta"),
        "the read-only Tag shows the script's value: {shown}"
    );
    let data = datasets_text(doc(&session));
    assert!(
        data.contains("<Kind>b</Kind>") && data.contains("<Tag>beta</Tag>"),
        "{data}"
    );
    let catalog = root(doc(&session));
    assert_ne!(
        catalog.get(b"TPDFXfaSig".as_slice()).cloned(),
        open_signature
    );
    assert_eq!(session.doc.as_ref().unwrap().page_count(), 1);

    // Undo brings the whole document back: A shown, B hidden, the old widgets.
    let undone = ok(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(undone["ok"], true, "{undone}");
    let after_undo = listed(&mut engine, &mut session, 0);
    assert_eq!(ids(&after_undo), open_ids);
    assert_eq!(by(&after_undo, "Name[0]")["value"], "Zed");
    assert!(!datasets_text(doc(&session)).contains("<Kind>b</Kind>"));
    let redone = ok(&mut engine, &mut session, json!({"op": "redo"}));
    assert_eq!(redone["ok"], true, "{redone}");
    assert_eq!(names(&listed(&mut engine, &mut session, 0))[3], "FB[0]");

    // Saved and reopened: the marker keeps the pages, the choice shows.
    let saved = save(&mut engine, &mut session);
    let (mut engine, mut session, reply) = session_with(saved);
    assert_eq!(reply["info"]["xfaRendered"], true);
    let reopened = listed(&mut engine, &mut session, 0);
    assert_eq!(names(&reopened)[3], "FB[0]");
    assert_eq!(chosen(&reopened)["value"], true);

    // Opened fresh with the choice in the data: `initialize` alone shows B.
    let data = SCRIPTED_DATASETS.replace("<Kind/>", "<Kind>b</Kind>");
    let (mut engine, mut session, _) = session_with(source(SCRIPTED_TEMPLATE, &data, None));
    let fresh = listed(&mut engine, &mut session, 0);
    assert_eq!(names(&fresh)[3], "FB[0]", "{fresh:?}");
    let text = ok(&mut engine, &mut session, json!({"op": "text-all"}));
    let shown = text["text"].as_str().unwrap();
    assert!(
        shown.contains("Section Beta") && !shown.contains("beta"),
        "change handlers do not run at open: {shown}"
    );
}

const CALCULATED_TEMPLATE: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.6/">
<subform name="form1" layout="tb" locale="en_US">
  <pageSet>
    <pageArea name="Page1" id="Page1">
      <contentArea x="0.25in" y="0.25in" w="8in" h="10.5in"/>
      <medium stock="letter" short="8.5in" long="11in"/>
    </pageArea>
  </pageSet>
  <subform name="S" layout="tb" w="8in">
    <field name="Qty" w="4in" h="9mm"><ui><textEdit/></ui></field>
    <field name="Price" w="4in" h="9mm"><ui><textEdit/></ui></field>
    <field name="Total" w="4in" h="9mm"><ui><textEdit/></ui>
      <calculate><script contentType="application/x-javascript">Qty.rawValue * Price.rawValue</script></calculate>
    </field>
  </subform>
</subform>
</template>"#;

#[test]
fn a_fill_recalculates_the_field_that_depends_on_it() {
    let datasets = "<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\">\
        <xfa:data><form1><S><Qty>2</Qty><Price>5</Price><Total/></S></form1></xfa:data>\
        </xfa:datasets>";
    let (mut engine, mut session, reply) =
        session_with(source(CALCULATED_TEMPLATE, datasets, None));
    assert_eq!(reply["info"]["xfaRendered"], true, "{reply}");
    let at_open = listed(&mut engine, &mut session, 0);
    assert_eq!(names(&at_open), ["Qty[0]", "Price[0]", "Total[0]"]);
    let total = |items: &[Value]| items[2]["value"].clone();
    assert_eq!(total(&at_open), "10", "calculated at open");
    assert!(datasets_text(doc(&session)).contains("<Total>10</Total>"));

    fill(&mut engine, &mut session, &at_open[0], json!("3"));
    let after = listed(&mut engine, &mut session, 0);
    assert_eq!(after[0]["value"], "3");
    assert_eq!(total(&after), "15", "{after:?}");
    let data = datasets_text(doc(&session));
    assert!(
        data.contains("<Qty>3</Qty>") && data.contains("<Total>15</Total>"),
        "{data}"
    );
}

#[test]
fn a_formcalc_calculation_follows_a_fill() {
    // The same form with its total in FormCalc: a script without a
    // contentType, as Designer writes one.
    let javascript =
        r#"<script contentType="application/x-javascript">Qty.rawValue * Price.rawValue</script>"#;
    assert!(CALCULATED_TEMPLATE.contains(javascript));
    let template = CALCULATED_TEMPLATE.replace(
        javascript,
        "<script>if (HasValue(Qty)) then\n  Round(Qty * Price, 2) ; the line total\nendif</script>",
    );
    let datasets = "<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\">\
        <xfa:data><form1><S><Qty>2</Qty><Price>5.5</Price><Total/></S></form1></xfa:data>\
        </xfa:datasets>";
    let (mut engine, mut session, reply) = session_with(source(&template, datasets, None));
    assert_eq!(reply["info"]["xfaRendered"], true, "{reply}");
    let at_open = listed(&mut engine, &mut session, 0);
    assert_eq!(names(&at_open), ["Qty[0]", "Price[0]", "Total[0]"]);
    let total = |items: &[Value]| items[2]["value"].clone();
    assert_eq!(total(&at_open), "11", "calculated at open");
    assert!(datasets_text(doc(&session)).contains("<Total>11</Total>"));

    fill(&mut engine, &mut session, &at_open[0], json!("3"));
    let after = listed(&mut engine, &mut session, 0);
    assert_eq!(after[0]["value"], "3");
    assert_eq!(total(&after), "16.5", "{after:?}");
    let data = datasets_text(doc(&session));
    assert!(
        data.contains("<Qty>3</Qty>") && data.contains("<Total>16.5</Total>"),
        "{data}"
    );
}

#[test]
fn a_fill_whose_relayout_refuses_still_stands() {
    // A script that never ends: the budget stops it, the choice is kept and
    // the page still lays out (with what ran before the stop).
    let template = SCRIPTED_TEMPLATE.replace(
        "sco.showB();",
        "sco.showB(); while (true) { Tag.rawValue = Tag.rawValue + \"x\" }",
    );
    let (mut engine, mut session, _) = session_with(source(&template, SCRIPTED_DATASETS, None));
    let items = listed(&mut engine, &mut session, 0);
    let kb = items.iter().find(|i| i["on"] == "b").unwrap().clone();
    fill(&mut engine, &mut session, &kb, json!(true));
    let after = listed(&mut engine, &mut session, 0);
    assert_eq!(names(&after)[3], "FB[0]", "{after:?}");
    assert!(datasets_text(doc(&session)).contains("<Kind>b</Kind>"));
    // A single-stream /XFA: the datasets are not written, so no relayout.
    let bytes = source(SCRIPTED_TEMPLATE, SCRIPTED_DATASETS, Some("14 0 R"));
    let (mut engine, mut session, reply) = session_with(bytes);
    assert_eq!(reply["info"]["xfaRendered"], true, "{reply}");
    let items = listed(&mut engine, &mut session, 0);
    let before = ids(&items);
    let kb = items.iter().find(|i| i["on"] == "b").unwrap().clone();
    fill(&mut engine, &mut session, &kb, json!(true));
    let after = listed(&mut engine, &mut session, 0);
    assert_eq!(ids(&after), before);
    assert_eq!(
        after.iter().find(|i| i["on"] == "b").unwrap()["value"],
        true
    );
}

/// A table of rows with add and delete buttons, as Designer writes one:
/// `Row` repeats (`occur max="-1"`), each holding an `Add_Delete` subform
/// with two push buttons (`bind match="none"`) whose click scripts call
/// the row's instance manager, and a total calculated in FormCalc.
const ROWS_TEMPLATE: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.6/">
<subform name="form1" layout="tb" locale="en_US">
  <pageSet>
    <pageArea name="Page1" id="Page1">
      <contentArea x="0.25in" y="0.25in" w="8in" h="10.5in"/>
      <medium stock="letter" short="8.5in" long="11in"/>
    </pageArea>
  </pageSet>
  <subform name="Section4" layout="tb" w="8in">
    <subform layout="tb" w="8in">
      <subform name="Header" layout="tb" w="8in">
        <draw name="H" w="8in" h="12pt"><value><text>Directors</text></value></draw>
      </subform>
      <subform name="Row" layout="lr-tb" w="8in">
        <subform name="Add_Delete" layout="tb">
          <field name="Add" w="5mm" h="4mm">
            <ui><button highlight="inverted"/></ui>
            <caption><value><text>+</text></value></caption>
            <bind match="none"/>
            <event activity="click"><script contentType="application/x-javascript">
Row.instanceManager.addInstance(0);
</script></event>
          </field>
          <field name="Delete" w="5mm" h="4mm">
            <ui><button highlight="inverted"/></ui>
            <caption><value><text>-</text></value></caption>
            <bind match="none"/>
            <event activity="click"><script contentType="application/x-javascript">
this.parent.parent.instanceManager.removeInstance(this.parent.parent.index);
</script></event>
          </field>
        </subform>
        <field name="Name" w="3in" h="9mm"><ui><textEdit/></ui></field>
        <occur max="-1"/>
      </subform>
      <field name="Count" w="2in" h="9mm" access="readOnly"><ui><textEdit/></ui>
        <calculate><script>Row.instanceManager.count</script></calculate>
      </field>
    </subform>
  </subform>
</subform>
</template>"#;

const ROWS_DATASETS: &str = "<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\">\
<xfa:data><form1><Section4><Header xfa:dataNode=\"dataGroup\"/>\
<Row><Add_Delete xfa:dataNode=\"dataGroup\"/><Name>Ann</Name></Row>\
<Count/></Section4></form1></xfa:data></xfa:datasets>";

#[test]
fn add_and_delete_row_buttons_change_the_rows() {
    let (mut engine, mut session, reply) = session_with(source(ROWS_TEMPLATE, ROWS_DATASETS, None));
    assert_eq!(reply["info"]["xfaRendered"], true, "{reply}");
    let at_open = listed(&mut engine, &mut session, 0);
    assert_eq!(names(&at_open), ["Add[0]", "Delete[0]", "Name[0]"]);
    let add = at_open[0].clone();
    assert_eq!(add["kind"], "button");
    assert_eq!(add["label"], "+", "the caption labels the button");
    assert_eq!(add["value"], "");
    assert_eq!(add["scripted"], false);
    assert!(
        add["name"]
            .as_str()
            .unwrap()
            .ends_with("Row[0].Add_Delete[0].Add[0]"),
        "{add}"
    );
    let widget = doc(&session).resolve_id(id(add["id"].as_u64().unwrap() as u32));
    let widget = widget.as_dict().unwrap();
    assert_eq!(
        widget.get(b"Ff".as_slice()),
        Some(&Object::Integer(1 << 16))
    );
    assert!(widget.get(DATA_KEY).is_none(), "a button bears no data");
    assert!(widget.get(xfa::CLICK_KEY).is_some());
    let count = |session: &AppSession| {
        let data = datasets_text(doc(session));
        (
            data.matches("<Row>").count() + data.matches("<Row/>").count(),
            data,
        )
    };

    // Add: a second, blank row with its own buttons; the first keeps its name.
    fill(&mut engine, &mut session, &add, json!(""));
    let two = listed(&mut engine, &mut session, 0);
    assert_eq!(
        names(&two),
        [
            "Add[0]",
            "Delete[0]",
            "Name[0]",
            "Add[0]",
            "Delete[0]",
            "Name[0]"
        ],
        "{two:?}"
    );
    let row_names: Vec<String> = two
        .iter()
        .filter(|i| i["kind"] == "text")
        .map(|i| i["name"].as_str().unwrap().to_string())
        .collect();
    assert!(row_names[0].contains("Row[0]") && row_names[1].contains("Row[1]"));
    assert_eq!(two[2]["value"], "Ann");
    assert_eq!(two[5]["value"], "");
    let (rows, data) = count(&session);
    assert_eq!(rows, 2, "{data}");
    assert!(
        data.contains("<Count>2</Count>"),
        "the count recalculates: {data}"
    );

    // Fill the new row, then delete the first: the second moves up.
    fill(&mut engine, &mut session, &two[5], json!("Bo"));
    let two = listed(&mut engine, &mut session, 0);
    fill(&mut engine, &mut session, &two[1], json!(""));
    let one = listed(&mut engine, &mut session, 0);
    assert_eq!(names(&one), ["Add[0]", "Delete[0]", "Name[0]"], "{one:?}");
    assert_eq!(one[2]["value"], "Bo");
    let (rows, data) = count(&session);
    assert_eq!(rows, 1, "{data}");
    assert!(!data.contains("Ann"), "{data}");

    // Delete at occur min (1) is refused: nothing changes.
    let before = ids(&one);
    fill(&mut engine, &mut session, &one[1], json!(""));
    let still = listed(&mut engine, &mut session, 0);
    assert_eq!(ids(&still), before, "the last row stays");

    // Undo the delete: both rows are back, with their names.
    ok(&mut engine, &mut session, json!({"op": "undo"}));
    let back = listed(&mut engine, &mut session, 0);
    assert_eq!(names(&back).len(), 6, "{back:?}");
    assert_eq!(back[2]["value"], "Ann");
    assert_eq!(back[5]["value"], "Bo");
    assert_eq!(count(&session).0, 2);

    // Saved and reopened: the rows stay.
    let saved = save(&mut engine, &mut session);
    let (mut engine, mut session, _) = session_with(saved);
    assert_eq!(listed(&mut engine, &mut session, 0).len(), 6);
}
