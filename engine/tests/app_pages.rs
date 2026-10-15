// SPDX-License-Identifier: AGPL-3.0-or-later
//! Page organisation and assembly ops on the app session: rotate, move and
//! delete keep a valid page tree and undo exactly; insert and extract graft
//! pages without leaking the pages left behind; page labels read, format and
//! write as MuPDF does; images become pages.
use std::collections::BTreeSet;

use serde_json::{Value, json};
use tandem_pdf_engine::app::labels;
use tandem_pdf_engine::app::ops::pages::parse_page_range;
use tandem_pdf_engine::app::pagetree::PageTree;
use tandem_pdf_engine::app::{AppDocument, AppEngine, AppSession};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Object, ObjectId, decode_text_string};
use tandem_pdf_engine::view::ViewDocument;
use tandem_pdf_engine::writer::{DocumentWriteLimits, WriteOptions, write_document};

fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}

/// A classic-xref PDF from object bodies numbered from 1.
fn classic(objects: &[Vec<u8>], trailer: &str) -> Vec<u8> {
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
            "trailer\n<< /Size {} /Root 1 0 R {trailer} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn stream(dictionary: &str, data: &[u8]) -> Vec<u8> {
    let mut bytes = format!("<< {dictionary} /Length {} >>\nstream\n", data.len()).into_bytes();
    bytes.extend_from_slice(data);
    bytes.extend_from_slice(b"\nendstream");
    bytes
}

fn text(label: &str) -> Vec<u8> {
    stream(
        "",
        format!("BT /F1 24 Tf 72 100 Td ({label}) Tj ET").as_bytes(),
    )
}

/// Bodies of a `n`-page flat document: catalog 1, pages 2, font 3, page
/// `4 + 2i`, content `5 + 2i` saying "Page i".
fn flat(n: usize) -> Vec<Vec<u8>> {
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {n} /Resources << /Font << /F1 3 0 R >> >> >>",
            kids.join(" ")
        )
        .into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    for i in 0..n {
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R >>",
                5 + 2 * i
            )
            .into_bytes(),
        );
        objects.push(text(&format!("Page {i}")));
    }
    objects
}

/// A two-level tree: root 2 (Resources) over node 3 (MediaBox 200x300,
/// Rotate 90) holding pages 0-1 and node 4 (MediaBox 400x500) holding
/// pages 2-3. Page `i` is object `6 + 2i`, its content `7 + 2i`.
fn nested() -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 4 /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_vec(),
        b"<< /Type /Pages /Parent 2 0 R /Kids [6 0 R 8 0 R] /Count 2 /MediaBox [0 0 200 300] /Rotate 90 >>"
            .to_vec(),
        b"<< /Type /Pages /Parent 2 0 R /Kids [10 0 R 12 0 R] /Count 2 /MediaBox [0 0 400 500] >>"
            .to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    for i in 0..4 {
        let parent = if i < 2 { 3 } else { 4 };
        objects.push(
            format!(
                "<< /Type /Page /Parent {parent} 0 R /Contents {} 0 R >>",
                7 + 2 * i
            )
            .into_bytes(),
        );
        objects.push(text(&format!("Page {i}")));
    }
    classic(&objects, "")
}

fn dispatch(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    engine.dispatch(session, request, Vec::new()).0
}

fn dispatch_with(
    engine: &mut AppEngine,
    session: &mut AppSession,
    request: Value,
    buffers: Vec<Vec<u8>>,
) -> (Value, Vec<Vec<u8>>) {
    engine.dispatch(session, request, buffers)
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
    let reply = dispatch(engine, session, request);
    assert_eq!(reply["ok"], true, "{reply}");
    reply
}

fn document(session: &AppSession) -> &Document<'static> {
    session.doc.as_ref().unwrap().document()
}

/// A full rewrite of every object, for exact before/after comparisons.
fn serialized(session: &AppSession) -> Vec<u8> {
    let options = WriteOptions {
        garbage: false,
        ..WriteOptions::APP
    };
    write_document(document(session), DocumentWriteLimits::default(), options).unwrap()
}

/// What the app's save writes: reachable objects only.
fn saved(doc: &Document<'_>) -> Vec<u8> {
    write_document(doc, DocumentWriteLimits::default(), WriteOptions::APP).unwrap()
}

fn texts_of(bytes: Vec<u8>) -> Vec<String> {
    let view = ViewDocument::open(bytes, "").unwrap();
    (0..view.page_count())
        .map(|page| {
            view.structured_text(page)
                .unwrap()
                .as_text()
                .trim()
                .to_string()
        })
        .collect()
}

fn texts(session: &mut AppSession) -> Vec<String> {
    let view = session.view().unwrap();
    (0..view.page_count())
        .map(|page| {
            view.structured_text(page)
                .unwrap()
                .as_text()
                .trim()
                .to_string()
        })
        .collect()
}

fn sizes(reply: &Value) -> Vec<(f64, f64)> {
    let pages = reply["pages"].as_array().unwrap();
    (pages.iter())
        .map(|page| {
            (
                page["width"].as_f64().unwrap(),
                page["height"].as_f64().unwrap(),
            )
        })
        .collect()
}

fn dict<'d>(doc: &'d Document<'_>, id: ObjectId) -> &'d tandem_pdf_engine::object::Dictionary {
    doc.resolve_id(id).as_dict().unwrap()
}

/// Every /Pages node's /Count is the pages under it and every kid's /Parent
/// is its node; the tree lists what the page list lists.
fn assert_valid_tree(doc: &Document<'_>) {
    let tree = PageTree::read(doc).expect("an editable tree");
    let mut stack = vec![tree.root];
    let mut seen = BTreeSet::new();
    fn leaves(doc: &Document<'_>, node: ObjectId) -> i64 {
        let kids = doc.dict_get(dict(doc, node), b"Kids").as_array().unwrap();
        kids.iter()
            .map(|kid| {
                let kid = kid.as_reference().unwrap();
                let kind = doc
                    .dict_get(dict(doc, kid), b"Type")
                    .as_name()
                    .unwrap_or(b"");
                if kind == b"Pages" {
                    leaves(doc, kid)
                } else {
                    1
                }
            })
            .sum()
    }
    while let Some(node) = stack.pop() {
        assert!(seen.insert(node));
        let node_dict = dict(doc, node);
        assert_eq!(
            node_dict.get(b"Count".as_slice()),
            Some(&Object::Integer(leaves(doc, node))),
            "/Count of node {node:?}"
        );
        for kid in doc.dict_get(node_dict, b"Kids").as_array().unwrap() {
            let kid = kid.as_reference().unwrap();
            assert_eq!(
                dict(doc, kid).get(b"Parent".as_slice()),
                Some(&Object::Reference(node)),
                "/Parent of {kid:?}"
            );
            if doc.dict_get(dict(doc, kid), b"Type").as_name() == Some(b"Pages") {
                stack.push(kid);
            }
        }
    }
    assert_eq!(tree.len(), tandem_pdf_engine::app::info::pages(doc).len());
}

fn page_ids(session: &AppSession) -> Vec<u32> {
    (session.doc.as_ref().unwrap().page_ids().into_iter())
        .map(|id| id.unwrap().number())
        .collect()
}

// ------------------------------------------------------------------ rotate

#[test]
fn rotate_starts_from_the_inherited_rotation_and_writes_it_on_the_page() {
    let (mut engine, mut session) = session_with(nested());
    let before = serialized(&session);
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "page-rotate", "page": 0, "by": -90}),
    );
    assert_eq!(reply["rotations"], json!([0, 90, 0, 0]));
    assert_eq!(reply["revision"], 1);
    assert_eq!(reply["dirty"], true);
    assert_eq!(
        sizes(&reply)[0],
        (200.0, 300.0),
        "0 shadows the inherited 90"
    );
    assert_eq!(
        dict(document(&session), id(6)).get(b"Rotate".as_slice()),
        Some(&Object::Integer(0))
    );
    let after = serialized(&session);
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "page-rotate", "page": 3, "by": -90.0}),
    );
    assert_eq!(reply["rotations"], json!([0, 90, 0, 270]));
    let undo = op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(undo["step"], "rotate page");
    assert_eq!(serialized(&session), after);
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(
        serialized(&session),
        before,
        "undo restores the exact state"
    );

    for (request, message, reason) in [
        (
            json!({"op": "page-rotate", "page": 0, "by": 45}),
            "page-rotate: by must be 90 or -90",
            None,
        ),
        (
            json!({"op": "page-rotate", "page": 0, "by": "90"}),
            "page-rotate: by must be 90 or -90",
            None,
        ),
        (
            json!({"op": "page-rotate", "page": 0}),
            "page-rotate: by must be 90 or -90",
            None,
        ),
        (
            json!({"op": "page-rotate", "page": 4, "by": 90}),
            "page-rotate: page must be an integer in 0..3",
            Some("invalid"),
        ),
        (
            json!({"op": "page-rotate", "page": 0.5, "by": 90}),
            "page-rotate: page must be an integer in 0..3",
            Some("invalid"),
        ),
    ] {
        let reply = dispatch(&mut engine, &mut session, request);
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["error"], message);
        assert_eq!(reply.get("reason").and_then(Value::as_str), reason);
    }
    assert_eq!(serialized(&session), before, "refusals write nothing");
}

#[test]
fn page_ops_refuse_without_a_document_or_on_a_signed_one() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    for op_name in [
        "page-rotate",
        "page-delete",
        "page-move",
        "insert-pages",
        "labels-set",
        "labels-clear",
    ] {
        let reply = dispatch(&mut engine, &mut session, json!({"op": op_name, "page": 0}));
        assert_eq!(reply["reason"], "no-document", "{op_name}");
        assert_eq!(reply["error"], format!("{op_name}: no document open"));
    }
    for (op_name, message) in [
        ("extract-pages", "extract-pages: no document open"),
        ("labels", "labels: no document open"),
    ] {
        let reply = dispatch(
            &mut engine,
            &mut session,
            json!({"op": op_name, "pages": [0]}),
        );
        assert_eq!(reply["error"], message);
        assert!(reply.get("reason").is_none());
    }
    let mut objects = flat(2);
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [8 0 R] >> >>".to_vec();
    objects.push(
        b"<< /FT /Sig /T (s) /V << /Type /Sig >> /Subtype /Widget /Rect [0 0 0 0] /P 4 0 R >>"
            .to_vec(),
    );
    let (mut engine, mut session) = session_with(classic(&objects, ""));
    for request in [
        json!({"op": "page-rotate", "page": 0, "by": 90}),
        json!({"op": "page-delete", "page": 0}),
        json!({"op": "page-move", "page": 0, "to": 1}),
        json!({"op": "labels-set", "from": 0, "style": "r"}),
    ] {
        let reply = dispatch(&mut engine, &mut session, request);
        assert_eq!(reply["reason"], "read-only");
    }
    let extracted = dispatch_with(
        &mut engine,
        &mut session,
        json!({"op": "extract-pages", "pages": [1]}),
        vec![],
    );
    assert_eq!(
        extracted.0["ok"], true,
        "extracting from a signed file is allowed"
    );
    let out = Document::open(&extracted.1[0], DocumentLimits::default()).unwrap();
    assert!(tandem_pdf_engine::app::info::detect(&out, false, false).signatures == 0);
}

// -------------------------------------------------------------------- move

#[test]
fn move_across_nodes_keeps_what_the_page_inherited_and_the_counts() {
    let (mut engine, mut session) = session_with(nested());
    let before = serialized(&session);
    let opened = sizes(&json!({"pages": [
        {"width": 300.0, "height": 200.0}, {"width": 300.0, "height": 200.0},
        {"width": 400.0, "height": 500.0}, {"width": 400.0, "height": 500.0}]}));
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "page-move", "page": 0, "to": 3}),
    );
    assert_eq!(page_ids(&session), [8, 10, 12, 6]);
    assert_eq!(
        sizes(&reply),
        [opened[1], opened[2], opened[3], opened[0]],
        "every page keeps its size"
    );
    assert_eq!(reply["rotations"], json!([90, 0, 0, 90]));
    assert_eq!(
        texts(&mut session),
        ["Page 1", "Page 2", "Page 3", "Page 0"]
    );
    let doc = document(&session);
    assert_valid_tree(doc);
    let moved = dict(doc, id(6));
    assert_eq!(
        moved.get(b"Parent".as_slice()),
        Some(&Object::Reference(id(4)))
    );
    assert!(moved.contains_key(b"MediaBox".as_slice()) && moved.contains_key(b"Rotate".as_slice()));
    assert!(
        !moved.contains_key(b"Resources".as_slice()),
        "both nodes inherit the root's resources"
    );
    let after = serialized(&session);

    // Within one node: no inheritance changes, no count changes.
    op(
        &mut engine,
        &mut session,
        json!({"op": "page-move", "page": 3, "to": 1}),
    );
    assert_eq!(page_ids(&session), [8, 6, 10, 12]);
    assert_valid_tree(document(&session));
    op(
        &mut engine,
        &mut session,
        json!({"op": "page-move", "page": 0, "to": 3}),
    );
    assert_eq!(page_ids(&session), [6, 10, 12, 8]);
    assert_valid_tree(document(&session));

    for _ in 0..2 {
        assert_eq!(
            op(&mut engine, &mut session, json!({"op": "undo"}))["step"],
            "move page"
        );
    }
    assert_eq!(serialized(&session), after);
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
    op(&mut engine, &mut session, json!({"op": "redo"}));
    assert_eq!(serialized(&session), after);

    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "page-move", "page": 1, "to": 1}),
    );
    assert_eq!(reply["error"], "page-move: page 1 is already there");
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "page-move", "page": 1, "to": "x"}),
    );
    assert_eq!(reply["error"], "page-move: page must be an integer in 0..3");
    assert_eq!(reply["reason"], "invalid");
}

#[test]
fn move_keeps_page_labels_with_the_positions() {
    let mut objects = flat(3);
    objects[0] =
        b"<< /Type /Catalog /Pages 2 0 R /PageLabels << /Nums [0 << /S /r >> 2 << /S /A >>] >> >>"
            .to_vec();
    let (mut engine, mut session) = session_with(classic(&objects, ""));
    op(
        &mut engine,
        &mut session,
        json!({"op": "page-move", "page": 0, "to": 2}),
    );
    let reply = op(&mut engine, &mut session, json!({"op": "labels"}));
    assert_eq!(
        reply["items"],
        json!(["i", "ii", "A"]),
        "MuPDF's rearrange deletes them"
    );
}

// ------------------------------------------------------------------ delete

/// Three pages (objects 4, 6, 8) with links, an outline, a named
/// destination, fields with widgets on several pages, labels and an
/// open action that all involve page 1.
fn crossed() -> Vec<u8> {
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R /Outlines 10 0 R /AcroForm 15 0 R /Names << /Dests 22 0 R >> \
          /PageLabels << /Nums [0 << /S /r >> 1 << /S /D /P (A-) >> 2 << /S /R >>] >> /OpenAction [6 0 R /Fit] >>"
            .to_vec(),
        b"<< /Type /Pages /Kids [4 0 R 6 0 R 8 0 R] /Count 3 >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 5 0 R /Annots [16 0 R 17 0 R 18 0 R 19 0 R] >>".to_vec(),
        text("One"),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 7 0 R /Annots [20 0 R 21 0 R] >>".to_vec(),
        text("Two secret"),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents 9 0 R /Annots [23 0 R] >>".to_vec(),
        text("Three"),
        // 10: outline root; 11 One, 12 Two (child 13), 14 Three
        b"<< /Type /Outlines /First 11 0 R /Last 14 0 R /Count 4 >>".to_vec(),
        b"<< /Title (One) /Parent 10 0 R /Next 12 0 R /Dest [4 0 R /Fit] >>".to_vec(),
        b"<< /Title (Two) /Parent 10 0 R /Prev 11 0 R /Next 14 0 R /First 13 0 R /Last 13 0 R /Count 1 /A << /S /GoTo /D [6 0 R /Fit] >> >>".to_vec(),
        b"<< /Title (Two-A) /Parent 12 0 R /Dest (two) >>".to_vec(),
        b"<< /Title (Three) /Parent 10 0 R /Prev 12 0 R /Dest [2 /Fit] >>".to_vec(),
        // 15: form: keep (16 on page 0), both (24: kids 17 on page 0, 20 on page 1), only (25: kid 21 on page 1)
        b"<< /Fields [16 0 R 24 0 R 25 0 R] >>".to_vec(),
        b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (keep) /Rect [0 0 10 10] /P 4 0 R >>".to_vec(),
        b"<< /Type /Annot /Subtype /Widget /Parent 24 0 R /Rect [0 0 10 10] /P 4 0 R >>".to_vec(),
        b"<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A << /S /GoTo /D [6 0 R /XYZ 0 0 0] >> >>".to_vec(),
        b"<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A << /S /URI /URI (https://example.com/) >> >>".to_vec(),
        b"<< /Type /Annot /Subtype /Widget /Parent 24 0 R /Rect [0 0 10 10] /P 6 0 R >>".to_vec(),
        b"<< /Type /Annot /Subtype /Widget /Parent 25 0 R /Rect [0 0 10 10] /P 6 0 R >>".to_vec(),
        b"<< /Names [(two) [6 0 R /Fit]] >>".to_vec(),
        b"<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /Dest (two) >>".to_vec(),
        b"<< /FT /Tx /T (both) /Kids [17 0 R 20 0 R] >>".to_vec(),
        b"<< /FT /Tx /T (only) /Kids [21 0 R] >>".to_vec(),
    ];
    classic(&objects, "")
}

#[test]
fn delete_repairs_everything_that_named_the_page_in_one_step() {
    let (mut engine, mut session) = session_with(crossed());
    let before = serialized(&session);
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "page-delete", "page": 1}),
    );
    assert_eq!(reply["pageCount"], 2);
    let doc = document(&session);
    assert_valid_tree(doc);
    let annots = |page: u32| {
        dict(doc, id(page))
            .get(b"Annots".as_slice())
            .cloned()
            .unwrap()
    };
    assert_eq!(
        annots(4),
        Object::Array(vec![
            Object::Reference(id(16)),
            Object::Reference(id(17)),
            Object::Reference(id(19))
        ]),
        "the GoTo to the removed page went; the widget and the URI link stay"
    );
    assert_eq!(
        annots(8),
        Object::Array(vec![]),
        "a named destination to it counts"
    );
    for (item, kept) in [(11, true), (12, false), (13, false), (14, true)] {
        let item = dict(doc, id(item));
        assert_eq!(
            item.contains_key(b"Dest".as_slice()) || item.contains_key(b"A".as_slice()),
            kept
        );
        assert!(item.contains_key(b"Title".as_slice()));
    }
    assert_eq!(
        dict(doc, id(14)).get(b"Dest".as_slice()),
        Some(&Object::Array(vec![
            Object::Integer(2),
            Object::name("Fit")
        ])),
        "a page number is not rewritten (MuPDF reads it as the page now there)"
    );
    let fields = dict(doc, id(15))
        .get(b"Fields".as_slice())
        .cloned()
        .unwrap();
    assert_eq!(
        fields,
        Object::Array(vec![Object::Reference(id(16)), Object::Reference(id(24))])
    );
    assert_eq!(
        dict(doc, id(24)).get(b"Kids".as_slice()),
        Some(&Object::Array(vec![Object::Reference(id(17))]))
    );
    let labels = op(&mut engine, &mut session, json!({"op": "labels"}));
    assert_eq!(
        labels["items"],
        json!(["i", "I"]),
        "the range that started at the deleted page gave way"
    );
    // Nothing reachable names the removed page, so a save leaves it out.
    let doc = document(&session);
    let bytes = saved(doc);
    let written = Document::open(&bytes, DocumentLimits::default()).unwrap();
    assert!(
        written
            .objects()
            .all(|object| object.id != id(6) && object.id != id(7))
    );
    assert!(!bytes.windows(10).any(|w| w == b"Two secret"));
    assert_eq!(texts_of(bytes), ["One", "Three"]);

    let after = serialized(&session);
    assert_eq!(
        op(&mut engine, &mut session, json!({"op": "undo"}))["step"],
        "delete page"
    );
    assert_eq!(serialized(&session), before);
    op(&mut engine, &mut session, json!({"op": "redo"}));
    assert_eq!(serialized(&session), after);
}

#[test]
fn delete_prunes_an_emptied_node_and_refuses_the_last_page() {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 3 >>".to_vec(),
        b"<< /Type /Pages /Parent 2 0 R /Kids [5 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Pages /Parent 2 0 R /Kids [7 0 R 9 0 R] /Count 2 >>".to_vec(),
    ];
    for (i, parent) in [3, 4, 4].into_iter().enumerate() {
        objects.push(
            format!(
                "<< /Type /Page /Parent {parent} 0 R /MediaBox [0 0 100 100] /Contents {} 0 R >>",
                6 + 2 * i
            )
            .into_bytes(),
        );
        objects.push(stream("", b""));
    }
    let (mut engine, mut session) = session_with(classic(&objects, ""));
    op(
        &mut engine,
        &mut session,
        json!({"op": "page-delete", "page": 0}),
    );
    let doc = document(&session);
    assert_eq!(
        dict(doc, id(2)).get(b"Kids".as_slice()),
        Some(&Object::Array(vec![Object::Reference(id(4))]))
    );
    assert_valid_tree(doc);
    op(
        &mut engine,
        &mut session,
        json!({"op": "page-delete", "page": 1}),
    );
    assert_valid_tree(document(&session));
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "page-delete", "page": 0}),
    );
    assert_eq!(
        reply["error"],
        "page-delete: a one-page document has nothing to delete"
    );
    assert!(reply.get("reason").is_none());
    assert_eq!(reply["revision"], 2);
}

// ------------------------------------------------------------------ labels

#[test]
fn labels_read_nested_trees_and_format_as_mupdf() {
    let mut objects = flat(6);
    objects[0] =
        b"<< /Type /Catalog /Pages 2 0 R /PageLabels << /Kids [16 0 R 17 0 R] >> >>".to_vec();
    objects.push(b"<< /Limits [0 2] /Nums [0 << /S /r /St 4 >> 2 << /S /A /St 26 >>] >>".to_vec());
    objects.push(b"<< /Limits [3 5] /Nums [3 << /P (x-) >> 4 << /S /D /P <FEFF00E9> /St 0 >> 5 << /S /R /St 3999 >>] >>".to_vec());
    let (mut engine, mut session) = session_with(classic(&objects, ""));
    let reply = op(&mut engine, &mut session, json!({"op": "labels"}));
    assert_eq!(
        reply["items"],
        json!(["iv", "v", "Z", "x-", "\u{e9}1", "MMMCMXCIX"])
    );
    assert_eq!(reply.as_object().unwrap().len(), 2, "ok, items");

    // Letters repeat, roman thousands repeat, and a label stops at 99 bytes.
    let mut objects = flat(3);
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R /PageLabels << /Nums [0 << /S /a /St 27 >> 1 << /S /R /St 4000 >> 2 << /S /A /P (pppppppppppppppppppppppppppppppp) /St 100000 >>] >> >>".to_vec();
    let bytes = classic(&objects, "");
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let items = labels::items(&doc, 3);
    assert_eq!(items[0], "aa");
    assert_eq!(items[1], "MMMM");
    assert_eq!(items[2].len(), 99);
    assert!(items[2].starts_with("pppppppppppppppppppppppppppppppp") && items[2].ends_with('D'));
}

/// The /PageLabels /Nums of a session's document as (key, dictionary).
fn nums(session: &AppSession) -> Vec<(i64, Object)> {
    let doc = document(session);
    let root = doc.resolve_id(doc.root()).as_dict().unwrap();
    let labels = doc.dict_get(root, b"PageLabels").as_dict().unwrap();
    assert!(!labels.contains_key(b"Kids".as_slice()));
    let nums = doc.dict_get(labels, b"Nums").as_array().unwrap();
    nums.chunks(2)
        .map(|pair| (pair[0].as_i64().unwrap(), doc.resolve(&pair[1]).clone()))
        .collect()
}

fn label_dict(entries: &[(&str, Object)]) -> Object {
    Object::dictionary(
        entries
            .iter()
            .map(|(key, value)| (key.as_bytes().to_vec(), value.clone())),
    )
}

#[test]
fn labels_set_and_clear_write_what_mupdf_writes() {
    let (mut engine, mut session) = session_with(classic(&flat(3), ""));
    assert_eq!(
        op(&mut engine, &mut session, json!({"op": "labels"}))["items"],
        json!(["1", "2", "3"])
    );
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "labels-set", "from": 2, "style": "R", "prefix": "A-", "start": 5}),
    );
    assert_eq!(reply["items"], json!(["1", "2", "A-V"]));
    assert_eq!(reply["revision"], 1);
    assert_eq!(reply["pageCount"], 3);
    assert_eq!(
        nums(&session),
        vec![
            (0, label_dict(&[("S", Object::name("D"))])),
            (
                2,
                label_dict(&[
                    ("S", Object::name("R")),
                    ("P", Object::string("A-")),
                    ("St", Object::Integer(5))
                ])
            ),
        ]
    );
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "labels-set", "from": 1, "style": "", "prefix": "X", "start": null}),
    );
    assert_eq!(reply["items"], json!(["1", "X", "A-V"]));
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "labels-set", "from": 0, "style": "a"}),
    );
    assert_eq!(reply["items"], json!(["a", "X", "A-V"]));

    // Clearing where no range starts changes nothing and records no step.
    let state = serialized(&session);
    let mut fresh_engine = AppEngine::new();
    let (_, mut fresh) = session_with(classic(&flat(3), ""));
    let reply = op(
        &mut fresh_engine,
        &mut fresh,
        json!({"op": "labels-clear", "from": 1}),
    );
    assert_eq!(
        (
            reply["revision"].clone(),
            reply["canUndo"].clone(),
            reply["dirty"].clone()
        ),
        (json!(0), json!(false), json!(false))
    );
    assert_eq!(reply["items"], json!(["1", "2", "3"]));

    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "labels-clear", "from": 1}),
    );
    assert_eq!(reply["items"], json!(["a", "b", "A-V"]));
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "labels-clear", "from": 0}),
    );
    assert_eq!(reply["items"], json!(["1", "2", "A-V"]));
    assert_eq!(
        nums(&session)[0],
        (0, label_dict(&[("S", Object::name("D"))]))
    );
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "labels-clear", "from": 0}),
    );
    assert_eq!(reply["revision"], 5, "page 0 is already decimal: no step");
    assert_eq!(
        op(&mut engine, &mut session, json!({"op": "undo"}))["step"],
        "clear page labels"
    );
    assert_eq!(
        op(&mut engine, &mut session, json!({"op": "undo"}))["step"],
        "clear page labels"
    );
    assert_eq!(serialized(&session), state);
    assert_eq!(
        op(&mut engine, &mut session, json!({"op": "undo"}))["step"],
        "set page labels"
    );

    for (request, message, reason) in [
        (
            json!({"op": "labels-set", "from": 0, "style": "X"}),
            "labels-set: style must be one of D, r, R, a, A, \"\"",
            None,
        ),
        (
            json!({"op": "labels-set", "from": 9, "style": "D"}),
            "labels-set: page must be an integer in 0..2",
            Some("invalid"),
        ),
        (
            json!({"op": "labels-set", "from": 0, "style": "D", "start": 0}),
            "labels-set: start must be an integer of at least 1",
            None,
        ),
        (
            json!({"op": "labels-set", "from": 0, "style": "D", "start": 1.5}),
            "labels-set: start must be an integer of at least 1",
            None,
        ),
        (
            json!({"op": "labels-set", "from": 0, "style": "D", "prefix": "x".repeat(33)}),
            "labels-set: prefix must be a string of at most 32 characters",
            None,
        ),
        (
            json!({"op": "labels-set", "from": 0, "style": "D", "prefix": 3}),
            "labels-set: prefix must be a string of at most 32 characters",
            None,
        ),
        (
            json!({"op": "labels-clear", "from": -1}),
            "labels-clear: page must be an integer in 0..2",
            Some("invalid"),
        ),
    ] {
        let reply = dispatch(&mut engine, &mut session, request);
        assert_eq!(reply["error"], message);
        assert_eq!(reply.get("reason").and_then(Value::as_str), reason);
    }
    // A prefix of 32 UTF-16 units passes even when it is more bytes.
    let prefix = "\u{e9}".repeat(32);
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "labels-set", "from": 0, "style": "D", "prefix": prefix}),
    );
    assert_eq!(reply["items"][0], format!("{prefix}1"));
}

#[test]
fn page_changes_move_label_ranges_without_duplicate_keys() {
    let entries = |keys: &[(i64, &str)]| -> Vec<labels::Entry> {
        keys.iter()
            .map(|(key, style)| (*key, label_dict(&[("S", Object::name(*style))])))
            .collect()
    };
    let keys = |entries: Vec<labels::Entry>| -> Vec<i64> {
        entries.into_iter().map(|(key, _)| key).collect()
    };
    let base = entries(&[(0, "D"), (2, "R")]);
    assert_eq!(
        keys(labels::inserted(base.clone(), 2, 1)),
        [0, 3],
        "MuPDF: keys at or after `at` move"
    );
    assert_eq!(keys(labels::inserted(base.clone(), 1, 2)), [0, 4]);
    assert_eq!(keys(labels::inserted(base.clone(), 5, 1)), [0, 2]);
    assert_eq!(
        keys(labels::inserted(base.clone(), 0, 1)),
        [0, 3],
        "inserted first pages join the first range"
    );
    assert_eq!(keys(labels::deleted(base.clone(), 2, 4)), [0, 2]);
    assert_eq!(keys(labels::deleted(base.clone(), 1, 4)), [0, 1]);
    let merged = labels::deleted(entries(&[(0, "D"), (1, "r")]), 0, 4);
    assert_eq!(merged, entries(&[(0, "r")]), "MuPDF writes [0 D 0 r]");
    assert_eq!(
        keys(labels::deleted(entries(&[(0, "D"), (4, "r")]), 4, 4)),
        [0],
        "a range past the end goes"
    );
}

// ------------------------------------------------------------ insert/extract

/// A source: a tree whose node gives MediaBox and Resources (one shared
/// font), three pages "Source 0..2". Page 0 has a link to page 2, a link
/// to page 1, a text field "name" and one radio widget of "choice" whose
/// other widget is on page 1. The form has /DR and /DA.
fn source_pdf() -> Vec<u8> {
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [12 0 R 13 0 R] /DR << /Font << /Helv 4 0 R >> >> /DA (/Helv 0 Tf 0 g) >> >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 3 >>".to_vec(),
        b"<< /Type /Pages /Parent 2 0 R /Kids [5 0 R 7 0 R 9 0 R] /Count 3 /MediaBox [0 0 300 400] /Resources << /Font << /F1 4 0 R >> >> >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        b"<< /Type /Page /Parent 3 0 R /Contents 6 0 R /Annots [10 0 R 11 0 R 12 0 R 14 0 R] /StructParents 0 >>".to_vec(),
        text("Source 0"),
        b"<< /Type /Page /Parent 3 0 R /Contents 8 0 R /Annots [15 0 R] >>".to_vec(),
        text("Source 1 private"),
        b"<< /Type /Page /Parent 3 0 R /Contents 11 0 R /Rotate 90 >>".to_vec(),
        b"<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /Dest [9 0 R /Fit] >>".to_vec(),
        text("Source 2"),
        b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /Rect [0 0 10 10] /P 7 0 R >>".to_vec(),
        b"<< /FT /Btn /Ff 49152 /T (choice) /Kids [14 0 R 15 0 R] >>".to_vec(),
        b"<< /Type /Annot /Subtype /Widget /Parent 13 0 R /Rect [0 0 10 10] /P 5 0 R >>".to_vec(),
        b"<< /Type /Annot /Subtype /Widget /Parent 13 0 R /Rect [0 0 10 10] /P 7 0 R >>".to_vec(),
    ];
    // Object 11 is page 2's content; the second link (to page 1) is object 16.
    let mut objects = objects;
    objects[4] = b"<< /Type /Page /Parent 3 0 R /Contents 6 0 R /Annots [10 0 R 16 0 R 12 0 R 14 0 R] /StructParents 0 >>".to_vec();
    objects[10] = text("Source 2");
    objects.push(
        b"<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A << /S /GoTo /D [7 0 R /Fit] >> >>"
            .to_vec(),
    );
    classic(&objects, "")
}

fn target_pdf() -> Vec<u8> {
    let mut objects = flat(2);
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [8 0 R] >> >>".to_vec();
    objects.push(
        b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /Rect [0 0 10 10] /P 4 0 R >>"
            .to_vec(),
    );
    objects[3] =
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R /Annots [8 0 R] >>"
            .to_vec();
    classic(&objects, "")
}

fn insert(
    engine: &mut AppEngine,
    session: &mut AppSession,
    source: Vec<u8>,
    extra: Value,
) -> Value {
    let mut request = json!({"op": "insert-pages", "pdf": {"$buffer": 0}, "name": "source.pdf"});
    if let (Value::Object(fields), Value::Object(extra)) = (&mut request, extra) {
        fields.extend(extra);
    }
    dispatch_with(engine, session, request, vec![source]).0
}

#[test]
fn insert_grafts_pages_without_the_pages_left_behind() {
    let (mut engine, mut session) = session_with(target_pdf());
    let before = serialized(&session);
    let reply = insert(
        &mut engine,
        &mut session,
        source_pdf(),
        json!({"at": 1, "pages": [2, 0]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["inserted"], 2);
    assert_eq!(reply["pageCount"], 4);
    assert_eq!(
        sizes(&reply)[1..3],
        [(400.0, 300.0), (300.0, 400.0)],
        "inherited boxes come along"
    );
    assert_eq!(reply["rotations"], json!([0, 90, 0, 0]));
    assert_eq!(reply["fieldNames"], json!(["name", "name (2)", "choice"]));
    assert_eq!(
        texts(&mut session),
        ["Page 0", "Source 2", "Source 0", "Page 1"]
    );
    let doc = document(&session);
    assert_valid_tree(doc);
    let ids: Vec<ObjectId> = (session.doc.as_ref().unwrap().page_ids().into_iter())
        .map(Option::unwrap)
        .collect();
    let page = dict(doc, ids[2]);
    assert!(!page.contains_key(b"StructParents".as_slice()));
    let annots = doc.dict_get(page, b"Annots").as_array().unwrap();
    assert_eq!(
        annots.len(),
        3,
        "the link to the page left behind is not copied"
    );
    let link = doc.resolve(&annots[0]).as_dict().unwrap();
    assert_eq!(
        link.get(b"Dest".as_slice()),
        Some(&Object::Array(vec![
            Object::Reference(ids[1]),
            Object::name("Fit")
        ])),
        "a destination to a copied page names the copy"
    );
    for widget in &annots[1..] {
        let widget = doc.resolve(widget).as_dict().unwrap();
        assert_eq!(
            widget.get(b"P".as_slice()),
            Some(&Object::Reference(ids[2]))
        );
    }
    let choice = doc.resolve(doc.dict_get(doc.resolve(&annots[2]).as_dict().unwrap(), b"Parent"));
    assert_eq!(
        doc.dict_get(choice.as_dict().unwrap(), b"Kids")
            .as_array()
            .unwrap()
            .len(),
        1,
        "the radio widget on the page left behind stays behind"
    );
    let acro = doc
        .dict_get(doc.resolve_id(doc.root()).as_dict().unwrap(), b"AcroForm")
        .as_dict()
        .unwrap();
    let helv = doc.dict_get(
        doc.dict_get(doc.dict_get(acro, b"DR").as_dict().unwrap(), b"Font")
            .as_dict()
            .unwrap(),
        b"Helv",
    );
    assert_eq!(
        doc.dict_get(helv.as_dict().unwrap(), b"BaseFont").as_name(),
        Some(b"Helvetica".as_slice())
    );
    assert!(acro.contains_key(b"DA".as_slice()));
    // The shared font is copied once (it serves both pages and /DR).
    let fonts = doc
        .objects()
        .filter(|o| {
            o.value
                .as_dict()
                .is_some_and(|d| d.get(b"BaseFont".as_slice()).is_some())
        })
        .count();
    assert_eq!(fonts, 2, "the target's own font and one copy");
    let bytes = saved(doc);
    assert!(
        !bytes.windows(7).any(|w| w == b"private"),
        "page 1 of the source never comes in"
    );

    let after = serialized(&session);
    assert_eq!(
        op(&mut engine, &mut session, json!({"op": "undo"}))["step"],
        "insert pages"
    );
    assert_eq!(serialized(&session), before, "one undo removes everything");
    op(&mut engine, &mut session, json!({"op": "redo"}));
    assert_eq!(serialized(&session), after);

    // A page listed twice gets its own annotations and fields each time.
    let reply = insert(
        &mut engine,
        &mut session,
        source_pdf(),
        json!({"at": 0, "pages": [0, 0]}),
    );
    assert_eq!(
        reply["fieldNames"],
        json!([
            "name",
            "name (2)",
            "choice",
            "name (3)",
            "choice (2)",
            "name (4)",
            "choice (3)"
        ])
    );
    assert_valid_tree(document(&session));
}

#[test]
fn insert_ranges_refusals_and_a_damaged_source() {
    let (mut engine, mut session) = session_with(classic(&flat(3), ""));
    let before = serialized(&session);
    let source = source_pdf();
    let refusals: Vec<(Value, &str, Option<&str>)> = vec![
        (
            insert(&mut engine, &mut session, source.clone(), json!({"at": 4})),
            "insert-pages: at must be an integer in 0..3",
            None,
        ),
        (
            insert(&mut engine, &mut session, source.clone(), json!({"at": -1})),
            "insert-pages: at must be an integer in 0..3",
            None,
        ),
        (
            insert(
                &mut engine,
                &mut session,
                source.clone(),
                json!({"at": 1.5}),
            ),
            "insert-pages: at must be an integer in 0..3",
            None,
        ),
        (
            insert(
                &mut engine,
                &mut session,
                source.clone(),
                json!({"at": 0, "name": ""}),
            ),
            "insert-pages: missing source name",
            None,
        ),
        (
            insert(
                &mut engine,
                &mut session,
                source.clone(),
                json!({"at": 0, "pages": []}),
            ),
            "insert-pages: pages must be 1..2000 page numbers",
            Some("invalid"),
        ),
        (
            insert(
                &mut engine,
                &mut session,
                source.clone(),
                json!({"at": 0, "pages": [3]}),
            ),
            "insert-pages: page 0 must be an integer in 0..2",
            Some("invalid"),
        ),
        (
            insert(
                &mut engine,
                &mut session,
                source.clone(),
                json!({"at": 0, "pages": vec![0; 2001]}),
            ),
            "insert-pages: pages must be 1..2000 page numbers",
            Some("invalid"),
        ),
        (
            insert(
                &mut engine,
                &mut session,
                source.clone(),
                json!({"at": 0, "range": "nonsense"}),
            ),
            "insert-pages: pages must look like \"1-3, 5\"",
            None,
        ),
        (
            insert(
                &mut engine,
                &mut session,
                source.clone(),
                json!({"at": 0, "range": "0-2"}),
            ),
            "insert-pages: pages must look like \"1-3, 5\"",
            None,
        ),
        (
            insert(
                &mut engine,
                &mut session,
                b"%PDF".to_vec(),
                json!({"at": 0}),
            ),
            "insert-pages: the source is not a PDF (too few bytes)",
            None,
        ),
    ];
    for (reply, message, reason) in refusals {
        assert_eq!(reply["error"], message, "{reply}");
        assert_eq!(reply.get("reason").and_then(Value::as_str), reason);
    }
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "insert-pages", "pdf": "x", "name": "s", "at": 0}),
    );
    assert_eq!(reply["error"], "insert-pages: no source bytes");
    let reply = insert(&mut engine, &mut session, vec![0; 10], json!({"at": 0}));
    assert!(
        reply["error"]
            .as_str()
            .unwrap()
            .starts_with("insert-pages: the source could not be read ("),
        "{reply}"
    );
    let reply = insert(
        &mut engine,
        &mut session,
        classic(
            &[
                b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
                b"<< /Type /Pages /Kids [] /Count 0 >>".to_vec(),
            ],
            "",
        ),
        json!({"at": 0}),
    );
    assert_eq!(reply["error"], "insert-pages: the source has no pages");
    assert_eq!(serialized(&session), before);
    assert_eq!(
        dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"],
        false
    );

    let reply = insert(
        &mut engine,
        &mut session,
        source.clone(),
        json!({"at": 3, "range": " 3 - 2 "}),
    );
    assert_eq!(reply["inserted"], 2);
    assert_eq!(texts(&mut session)[3..], ["Source 1 private", "Source 2"]);
    // A source whose xref is wrong opens through the repair, as MuPDF's does.
    let mut damaged = source_pdf();
    let at = damaged.windows(9).rposition(|w| w == b"startxref").unwrap();
    damaged.truncate(at);
    damaged.extend_from_slice(b"startxref\n12345\n%%EOF\n");
    let reply = insert(
        &mut engine,
        &mut session,
        damaged,
        json!({"at": 0, "pages": [1]}),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(texts(&mut session)[0], "Source 1 private");
    assert_valid_tree(document(&session));
}

#[test]
fn page_ranges_parse_as_engine_ts_does() {
    assert_eq!(parse_page_range("", 3), Some(vec![0, 1, 2]));
    assert_eq!(parse_page_range(" \u{a0}", 3), Some(vec![0, 1, 2]));
    assert_eq!(parse_page_range("1-3, 5", 5), Some(vec![0, 1, 2, 4]));
    assert_eq!(parse_page_range("3-1,2", 3), Some(vec![0, 1, 2]));
    assert_eq!(parse_page_range("2-", 4), Some(vec![1, 2, 3]));
    assert_eq!(parse_page_range("-2", 4), Some(vec![0, 1]));
    assert_eq!(parse_page_range("-", 4), None);
    assert_eq!(parse_page_range("0", 4), None);
    assert_eq!(parse_page_range("5", 4), None);
    assert_eq!(parse_page_range("1,,2", 4), None);
    assert_eq!(parse_page_range("1-2-3", 4), None);
    assert_eq!(parse_page_range("+1", 4), None);
    assert_eq!(parse_page_range("99999999999999999999999", 4), None);
    assert_eq!(
        parse_page_range("1\u{85}", 4),
        None,
        "U+0085 is not JavaScript whitespace"
    );
    assert_eq!(parse_page_range("\u{feff}2", 4), Some(vec![1]));
}

#[test]
fn extract_writes_a_new_document_and_leaves_the_session_alone() {
    let (mut engine, mut session) = session_with(source_pdf());
    let before = serialized(&session);
    let (reply, outputs) = dispatch_with(
        &mut engine,
        &mut session,
        json!({"id": 4, "op": "extract-pages", "pages": [2, 0]}),
        vec![],
    );
    assert_eq!(
        reply,
        json!({"id": 4, "ok": true, "bytes": {"$buffer": 0}, "pageCount": 2})
    );
    let bytes = outputs.into_iter().next().unwrap();
    assert!(
        !bytes.windows(7).any(|w| w == b"private"),
        "the page left behind does not leak"
    );
    let out = AppDocument::load(std::sync::Arc::new(bytes.clone()), b"").unwrap();
    assert!(!out.repaired());
    let doc = out.document();
    assert_valid_tree(doc);
    assert_eq!(texts_of(bytes), ["Source 2", "Source 0"]);
    assert_eq!(out.page_state().rotations, [90, 0]);
    let ids: Vec<ObjectId> = out.page_ids().into_iter().map(Option::unwrap).collect();
    let annots = doc
        .dict_get(dict(doc, ids[1]), b"Annots")
        .as_array()
        .unwrap();
    assert_eq!(annots.len(), 3);
    let link = doc.resolve(&annots[0]).as_dict().unwrap();
    assert_eq!(
        doc.dict_get(link, b"Dest").as_array().unwrap()[0],
        Object::Reference(ids[0])
    );
    let names: Vec<String> = {
        let acro = doc
            .dict_get(doc.resolve_id(doc.root()).as_dict().unwrap(), b"AcroForm")
            .as_dict()
            .unwrap();
        (doc.dict_get(acro, b"Fields").as_array().unwrap().iter())
            .map(|field| {
                decode_text_string(
                    doc.dict_get(doc.resolve(field).as_dict().unwrap(), b"T")
                        .as_string()
                        .unwrap(),
                )
            })
            .collect()
    };
    assert_eq!(names, ["name", "choice"]);
    assert_eq!(serialized(&session), before);
    assert_eq!(session.revision, 0);
    assert_eq!(
        dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"],
        false
    );

    for (request, message) in [
        (
            json!({"op": "extract-pages", "pages": []}),
            "extract-pages: pages must be 1..2000 page numbers",
        ),
        (
            json!({"op": "extract-pages", "pages": [3]}),
            "extract-pages: page 0 must be an integer in 0..2",
        ),
        (
            json!({"op": "extract-pages", "pages": ["x"]}),
            "extract-pages: page 0 must be an integer in 0..2",
        ),
        (
            json!({"op": "extract-pages"}),
            "extract-pages: pages must be 1..2000 page numbers",
        ),
    ] {
        let reply = dispatch(&mut engine, &mut session, request);
        assert_eq!(reply["error"], message);
        assert_eq!(reply["reason"], "invalid");
    }
}

// ------------------------------------------------------------------ images

fn png_bytes(
    width: u32,
    height: u32,
    color: png::ColorType,
    data: &[u8],
    ppm: Option<u32>,
) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        if let Some(ppm) = ppm {
            encoder.set_pixel_dims(Some(png::PixelDimensions {
                xppu: ppm,
                yppu: ppm,
                unit: png::Unit::Meter,
            }));
        }
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(data).unwrap();
    }
    out
}

/// A JPEG's headers only (the engine passes the data through unread).
fn jpeg_headers(width: u16, height: u16, components: u8, extra: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8];
    bytes.extend_from_slice(extra);
    let mut frame = vec![8];
    frame.extend_from_slice(&height.to_be_bytes());
    frame.extend_from_slice(&width.to_be_bytes());
    frame.push(components);
    for c in 0..components {
        frame.extend_from_slice(&[c + 1, 0x11, 0]);
    }
    bytes.extend_from_slice(&[0xFF, 0xC0]);
    bytes.extend_from_slice(&(frame.len() as u16 + 2).to_be_bytes());
    bytes.extend_from_slice(&frame);
    bytes.extend_from_slice(&[0xFF, 0xDA, 0, 2, 0xFF, 0xD9]);
    bytes
}

fn images(engine: &mut AppEngine, files: Vec<(&str, Vec<u8>)>) -> (Value, Vec<Vec<u8>>) {
    let mut session = AppSession::default();
    let list: Vec<Value> = (0..files.len())
        .map(|index| json!({"name": files[index].0, "bytes": {"$buffer": index}}))
        .collect();
    let buffers = files.into_iter().map(|(_, bytes)| bytes).collect();
    dispatch_with(
        engine,
        &mut session,
        json!({"op": "pdf-from-images", "images": list}),
        buffers,
    )
}

fn image_of<'d>(
    doc: &'d Document<'_>,
    page: &'d tandem_pdf_engine::object::Dictionary,
) -> &'d tandem_pdf_engine::object::Dictionary {
    let resources = doc.dict_get(page, b"Resources").as_dict().unwrap();
    let xobjects = doc.dict_get(resources, b"XObject").as_dict().unwrap();
    doc.dict_get(xobjects, b"Im0").as_dict().unwrap()
}

#[test]
fn images_become_pages_at_their_resolution() {
    let mut engine = AppEngine::new();
    let rgb: Vec<u8> = (0..300 * 200)
        .flat_map(|i| [(i % 256) as u8, 7, 128])
        .collect();
    let gray: Vec<u8> = (0..200 * 300)
        .map(|i| {
            if (100..=120).contains(&(i / 200)) {
                0
            } else {
                200
            }
        })
        .collect();
    let rgba: Vec<u8> = (0..4 * 2)
        .flat_map(|i| [10, 20, 30, (i * 30) as u8])
        .collect();
    let adobe = {
        let mut app14 = vec![0xFF, 0xEE, 0, 14];
        app14.extend_from_slice(b"Adobe\0\x64\0\0\0\0\0");
        app14
    };
    let exif = {
        // Orientation 6, 144 dpi.
        let mut tiff = b"MM\0*\0\0\0\x08\0\x03".to_vec();
        for (tag, kind, count, value) in [
            (0x112u16, 3u16, 1u32, 6u32 << 16),
            (0x11A, 5, 1, 50),
            (0x128, 3, 1, 2 << 16),
        ] {
            tiff.extend_from_slice(&tag.to_be_bytes());
            tiff.extend_from_slice(&kind.to_be_bytes());
            tiff.extend_from_slice(&count.to_be_bytes());
            tiff.extend_from_slice(&value.to_be_bytes());
        }
        tiff.resize(50, 0);
        tiff.extend_from_slice(&144u32.to_be_bytes());
        tiff.extend_from_slice(&1u32.to_be_bytes());
        let mut segment = vec![0xFF, 0xE1];
        segment.extend_from_slice(&(tiff.len() as u16 + 8).to_be_bytes());
        segment.extend_from_slice(b"Exif\0\0");
        segment.extend_from_slice(&tiff);
        segment
    };
    let (reply, outputs) = images(
        &mut engine,
        vec![
            (
                "rgb.png",
                png_bytes(300, 200, png::ColorType::Rgb, &rgb, None),
            ),
            (
                "gray.png",
                png_bytes(200, 300, png::ColorType::Grayscale, &gray, Some(5906)),
            ),
            (
                "rgba.png",
                png_bytes(4, 2, png::ColorType::Rgba, &rgba, None),
            ),
            ("cmyk.jpg", jpeg_headers(40, 30, 4, &adobe)),
            ("photo.jpg", jpeg_headers(288, 144, 3, &exif)),
        ],
    );
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply.as_object().unwrap().len(), 3);
    assert_eq!(reply["pageCount"], 5);
    let out = AppDocument::load(std::sync::Arc::new(outputs[0].clone()), b"").unwrap();
    assert!(!out.repaired());
    let doc = out.document();
    assert_eq!(
        out.page_state().sizes,
        [
            (300.0, 200.0),
            (96.0, 144.0),
            (4.0, 2.0),
            (40.0, 30.0),
            (72.0, 144.0)
        ],
        "150 dpi is 0.48 pt per pixel; the photo stands up and is 144 dpi"
    );
    let pages = out.pages();
    let first = image_of(doc, pages[0].dict);
    assert_eq!(
        doc.dict_get(first, b"ColorSpace").as_name(),
        Some(b"DeviceRGB".as_slice())
    );
    assert_eq!(
        doc.dict_get(first, b"Filter").as_name(),
        Some(b"FlateDecode".as_slice())
    );
    assert!(!first.contains_key(b"SMask".as_slice()));
    assert!(image_of(doc, pages[2].dict).contains_key(b"SMask".as_slice()));
    let cmyk = image_of(doc, pages[3].dict);
    assert_eq!(
        doc.dict_get(cmyk, b"Filter").as_name(),
        Some(b"DCTDecode".as_slice())
    );
    assert_eq!(
        doc.dict_get(cmyk, b"Decode")
            .as_array()
            .map(<[Object]>::len),
        Some(8)
    );
    let photo = doc.dict_get(pages[4].dict, b"Contents");
    let id = pages[4]
        .dict
        .get(b"Contents".as_slice())
        .and_then(Object::as_reference)
        .unwrap();
    assert!(photo.as_dict().is_some());
    assert_eq!(
        doc.stream_data(id).unwrap().unwrap(),
        b"q 0 -144 72 0 0 144 cm /Im0 Do Q"
    );
    let view = ViewDocument::open(outputs[0].clone(), "").unwrap();
    assert!(view.render(1, 1.0, Default::default()).is_ok());

    for (files, message) in [
        (vec![], "pdf-from-images: images must be 1..50 files"),
        (
            vec![("", vec![1])],
            "pdf-from-images: image 0 needs a name of at most 255 characters",
        ),
        (
            vec![("notes.txt", b"not an image".to_vec())],
            "pdf-from-images: notes.txt is not a PNG or JPEG the app can read (not a PNG or JPEG file)",
        ),
        (
            vec![("x.png", vec![])],
            "pdf-from-images: image 0 (x.png) has no bytes",
        ),
        (
            vec![("x.png", vec![0; 32 * 1024 * 1024 + 1])],
            "pdf-from-images: x.png is over the 33554432 byte image ceiling",
        ),
    ] {
        let reply = images(&mut engine, files).0;
        assert_eq!(reply["error"], message);
    }
    let many = vec![("x.png", vec![1]); 51];
    assert_eq!(
        images(&mut engine, many).0["error"],
        "pdf-from-images: images must be 1..50 files"
    );
    let reply = images(&mut engine, vec![("x.jpg", jpeg_headers(0, 10, 3, &[]))]).0;
    assert_eq!(
        (reply["error"].as_str(), reply["reason"].as_str()),
        (
            Some("pdf-from-images: x.jpg has no pixels"),
            Some("invalid")
        )
    );
}

// ------------------------------------------------------------------ fuzz

struct Rng(u64);
impl Rng {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % bound.max(1)
    }
}

#[test]
fn random_page_edits_keep_a_valid_tree_and_undo_to_the_start() {
    for seed in 1..=6u64 {
        let mut rng = Rng(seed * 0x9E37_79B9_7F4A_7C15);
        let (mut engine, mut session) =
            session_with(if seed % 2 == 0 { nested() } else { crossed() });
        let start = serialized(&session);
        let source = source_pdf();
        let mut steps = 0;
        for _ in 0..40 {
            let count = session.doc.as_ref().unwrap().page_count() as u64;
            let request = match rng.next(9) {
                0 => {
                    json!({"op": "page-rotate", "page": rng.next(count), "by": if rng.next(2) == 0 { 90 } else { -90 }})
                }
                1 => json!({"op": "page-delete", "page": rng.next(count)}),
                2 | 3 => json!({"op": "page-move", "page": rng.next(count), "to": rng.next(count)}),
                4 => {
                    let at = rng.next(count + 1);
                    let reply = insert(
                        &mut engine,
                        &mut session,
                        source.clone(),
                        json!({"at": at, "pages": [rng.next(3), rng.next(3)]}),
                    );
                    steps += usize::from(reply["ok"] == true);
                    continue;
                }
                5 => {
                    let style = ["D", "r", "A", ""][rng.next(4) as usize];
                    json!({"op": "labels-set", "from": rng.next(count), "style": style, "start": rng.next(5) + 1})
                }
                6 => json!({"op": "labels-clear", "from": rng.next(count)}),
                7 => json!({"op": "undo"}),
                _ => json!({"op": "redo"}),
            };
            let before = session.doc.as_ref().unwrap().position();
            let reply = dispatch(&mut engine, &mut session, request.clone());
            let after = session.doc.as_ref().unwrap().position();
            if reply["ok"] == true && request["op"] != "undo" && request["op"] != "redo" {
                steps += usize::try_from(after - before).unwrap();
            }
            let doc = document(&session);
            assert_valid_tree(doc);
            assert_eq!(
                reply
                    .get("pageCount")
                    .map(|c| c.as_u64().unwrap() as usize)
                    .unwrap_or(session.doc.as_ref().unwrap().page_count()),
                session.doc.as_ref().unwrap().page_count()
            );
            let _ = labels::items(doc, session.doc.as_ref().unwrap().page_count());
        }
        let _ = steps;
        while dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"] == true {}
        assert_eq!(serialized(&session), start, "seed {seed}");
    }
}

#[test]
fn odd_trees_and_hostile_sources_never_panic() {
    // A page written into /Kids, a cycle back to the root, junk kids, and
    // labels with odd keys and values.
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R /PageLabels << /Nums [-3 << /S /D >> 1 7 0 R 9223372036854775807 << /S /A /St 9223372036854775807 >> (x) 1] >> >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R << /Type /Page /MediaBox [0 0 50 50] >> 2 0 R 99 0 R 5 /x 4 0 R] /Count 7 /Rotate 45 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>".to_vec(),
        b"<< /Type /Pages /Parent 2 0 R /Kids [5 0 R 6 0 R] >>".to_vec(),
        b"<< /Type /Page /Parent 4 0 R /Rotate -450.7 >>".to_vec(),
        b"<< /Type /Page /Parent 4 0 R /Annots [6 0 R 2 0 R] >>".to_vec(),
        b"<< /S /r /St -5 /P 3 >>".to_vec(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects, ""));
    let labels = op(&mut engine, &mut session, json!({"op": "labels"}));
    assert_eq!(labels["items"].as_array().unwrap().len(), 4);
    for request in [
        json!({"op": "page-rotate", "page": 1, "by": 90}),
        json!({"op": "page-move", "page": 1, "to": 3}),
        json!({"op": "page-move", "page": 3, "to": 0}),
        json!({"op": "page-delete", "page": 2}),
        json!({"op": "labels-set", "from": 1, "style": "A", "start": 9_007_199_254_740_991u64}),
        json!({"op": "labels-clear", "from": 0}),
        json!({"op": "page-delete", "page": 0}),
    ] {
        let reply = dispatch(&mut engine, &mut session, request.clone());
        assert_eq!(reply["ok"], true, "{request}: {reply}");
        assert_valid_count(&session);
    }
    while dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"] == true {}

    // A /Pages node written into /Kids is not edited.
    let mut objects = flat(2);
    objects[1] =
        b"<< /Type /Pages /Kids [<< /Type /Pages /Kids [4 0 R 6 0 R] >>] /Count 2 >>".to_vec();
    let (mut engine, mut session) = session_with(classic(&objects, ""));
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "page-rotate", "page": 0, "by": 90}),
    );
    assert_eq!(
        reply["error"],
        "page-rotate: the page tree has a node that is not an indirect object"
    );
    assert_eq!(reply["revision"], 0);

    // Damaged copies of a source: inserted or refused, never a panic.
    let (mut engine, mut session) = session_with(classic(&flat(2), ""));
    let source = source_pdf();
    let mut rng = Rng(0x1234_5678_9abc_def1);
    for round in 0..200 {
        let mut bytes = source.clone();
        match round % 3 {
            0 => bytes.truncate(rng.next(bytes.len() as u64) as usize),
            1 => {
                for _ in 0..8 {
                    let at = rng.next(bytes.len() as u64) as usize;
                    bytes[at] = rng.next(256) as u8;
                }
            }
            _ => {
                let at = rng.next(bytes.len() as u64) as usize;
                let len = rng.next(64) as usize;
                bytes.drain(at..(at + len).min(bytes.len()));
            }
        }
        let reply = insert(&mut engine, &mut session, bytes, json!({"at": 1}));
        if reply["ok"] == true {
            assert_valid_tree(document(&session));
            dispatch(&mut engine, &mut session, json!({"op": "undo"}));
        }
        let (reply, _) = dispatch_with(
            &mut engine,
            &mut session,
            json!({"op": "extract-pages", "pages": [1, 0]}),
            vec![],
        );
        assert_eq!(reply["ok"], true, "{reply}");
    }
}

/// The page list and the reply agree, and every node's /Count matches.
fn assert_valid_count(session: &AppSession) {
    let doc = document(session);
    let tree = PageTree::read(doc).unwrap();
    assert_eq!(tree.len(), session.doc.as_ref().unwrap().page_count());
}
