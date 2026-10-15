// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bookmarks and links on the app session: the outline lists in preorder
//! and its edits keep /First, /Last, /Next, /Prev, /Parent and the signed
//! /Count consistent; links list as MuPDF lists them, in page space, with
//! targets read from the objects (never resolved back from a URI); every
//! write is one journal step that undo restores exactly.
use std::collections::BTreeSet;

use serde_json::{Value, json};
use tandem_pdf_engine::app::ops::nav::links::PageSpace;
use tandem_pdf_engine::app::ops::nav::outline::Outline;
use tandem_pdf_engine::app::ops::nav::scheme_of;
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::document::Document;
use tandem_pdf_engine::object::{Dictionary, Object, ObjectId, decode_text_string};
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

fn strings(objects: &[&str]) -> Vec<String> {
    objects.iter().map(|body| body.to_string()).collect()
}

/// fixtures/nav.pdf's structure: three pages, a four-item outline (Two has
/// a child) and two links on pages 1 and 3.
fn nav() -> Vec<u8> {
    classic(&strings(&[
        "<< /Type /Catalog /Pages 2 0 R /Outlines 10 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [15 0 R 16 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [17 0 R 18 0 R] >>",
        "<< >>",
        "<< >>",
        "<< >>",
        "<< >>",
        "<< /Type /Outlines /First 11 0 R /Last 14 0 R /Count 4 >>",
        "<< /Title (One) /Parent 10 0 R /Next 12 0 R /Dest [3 0 R /Fit] >>",
        "<< /Title (Two) /Parent 10 0 R /Prev 11 0 R /Next 14 0 R /First 13 0 R /Last 13 0 R /Count 1 /Dest [4 0 R /Fit] >>",
        "<< /Title (Two-A) /Parent 12 0 R /Dest [4 0 R /Fit] >>",
        "<< /Title (Three) /Parent 10 0 R /Prev 12 0 R /Dest [5 0 R /Fit] >>",
        "<< /Type /Annot /Subtype /Link /P 3 0 R /Rect [72 680 200 730] /F 4 /A << /S /GoTo /D [4 0 R /Fit] >> >>",
        "<< /Type /Annot /Subtype /Link /P 3 0 R /Rect [72 600 300 630] /F 4 /A << /S /URI /URI (https://example.com/) >> >>",
        "<< /Type /Annot /Subtype /Link /P 5 0 R /Rect [72 680 200 730] /F 4 /A << /S /GoTo /D [4 0 R /Fit] >> >>",
        "<< /Type /Annot /Subtype /Link /P 5 0 R /Rect [72 600 300 630] /F 4 /A << /S /URI /URI (file:///etc/hosts) >> >>",
    ]))
}

fn session_with(bytes: Vec<u8>) -> (AppEngine, AppSession) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(&mut session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
    (engine, session)
}

fn dispatch(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    engine.dispatch(session, request, Vec::new()).0
}

fn op(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    let reply = dispatch(engine, session, request.clone());
    assert_eq!(reply["ok"], true, "{request}: {reply}");
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
fn saved(session: &AppSession) -> Vec<u8> {
    write_document(
        document(session),
        DocumentWriteLimits::default(),
        WriteOptions::APP,
    )
    .unwrap()
}

fn outline(engine: &mut AppEngine, session: &mut AppSession) -> Vec<Value> {
    op(engine, session, json!({"op": "outline"}))["items"]
        .as_array()
        .unwrap()
        .clone()
}

fn titles(engine: &mut AppEngine, session: &mut AppSession) -> Vec<(String, i64, Value)> {
    outline(engine, session)
        .iter()
        .map(|item| {
            (
                item["title"].as_str().unwrap().to_string(),
                item["depth"].as_i64().unwrap(),
                item["page"].clone(),
            )
        })
        .collect()
}

fn links(engine: &mut AppEngine, session: &mut AppSession, page: usize) -> Vec<Value> {
    op(engine, session, json!({"op": "links", "page": page}))["items"]
        .as_array()
        .unwrap()
        .clone()
}

fn dict<'d>(doc: &'d Document<'_>, value: &'d Object) -> &'d Dictionary {
    doc.resolve(value).as_dict().unwrap()
}

/// The /Outlines tree is what ISO 32000-1 12.3.3 asks: every chain's /Prev
/// and /Next mirror each other, /First and /Last name its ends, /Parent
/// names the parent, and /Count is the visible descendants (negated for a
/// closed item, absent on a leaf; the root's absent when nothing shows).
fn assert_valid_outline(doc: &Document<'_>) {
    let catalog = doc.resolve_id(doc.root()).as_dict().unwrap();
    let Some(root) = catalog.get(b"Outlines".as_slice()) else {
        return;
    };
    let root_id = root.as_reference().expect("an indirect root");
    let mut seen = BTreeSet::new();
    // Returns the visible descendants of `node`'s children when open.
    fn chain(doc: &Document<'_>, node: ObjectId, seen: &mut BTreeSet<ObjectId>) -> i64 {
        let node_dict = doc.resolve_id(node).as_dict().unwrap();
        let first = node_dict.get(b"First".as_slice());
        let mut at = first.map(|first| first.as_reference().expect("indirect /First"));
        let mut prev: Option<ObjectId> = None;
        let mut shown = 0i64;
        while let Some(item) = at {
            assert!(seen.insert(item), "item {item:?} walked twice");
            let item_dict = doc.resolve_id(item).as_dict().unwrap();
            assert_eq!(
                item_dict.get(b"Parent".as_slice()),
                Some(&Object::Reference(node)),
                "/Parent of {item:?}"
            );
            assert_eq!(
                item_dict.get(b"Prev".as_slice()),
                prev.map(Object::Reference).as_ref(),
                "/Prev of {item:?}"
            );
            let below = chain(doc, item, seen);
            let count = item_dict.get(b"Count".as_slice()).and_then(Object::as_i64);
            let has_kids = item_dict.contains_key(b"First".as_slice());
            if has_kids {
                let count = count.expect("an item with children has /Count");
                assert_eq!(count.abs(), below, "|/Count| of {item:?}");
                shown += 1 + if count > 0 { below } else { 0 };
            } else {
                assert_eq!(count, None, "a leaf has no /Count ({item:?})");
                shown += 1;
            }
            prev = Some(item);
            at = item_dict
                .get(b"Next".as_slice())
                .map(|next| next.as_reference().expect("indirect /Next"));
        }
        assert_eq!(
            node_dict.get(b"Last".as_slice()),
            prev.map(Object::Reference).as_ref(),
            "/Last of {node:?}"
        );
        shown
    }
    let total = chain(doc, root_id, &mut seen);
    let root_dict = doc.resolve_id(root_id).as_dict().unwrap();
    let count = root_dict.get(b"Count".as_slice()).and_then(Object::as_i64);
    assert_eq!(count, (total > 0).then_some(total), "root /Count");
}

// ------------------------------------------------------------------ outline

#[test]
fn outline_lists_in_preorder_and_add_appends_one_undoable_step() {
    let (mut engine, mut session) = session_with(nav());
    let items = outline(&mut engine, &mut session);
    assert_eq!(
        Value::Array(items),
        json!([
            {"id": 0, "title": "One", "depth": 0, "page": 0},
            {"id": 1, "title": "Two", "depth": 0, "page": 1},
            {"id": 2, "title": "Two-A", "depth": 1, "page": 1},
            {"id": 3, "title": "Three", "depth": 0, "page": 2},
        ])
    );
    let start = serialized(&session);
    let added = op(
        &mut engine,
        &mut session,
        json!({"op": "outline-add", "title": "PDF4All P2 nav", "page": 2}),
    );
    assert_eq!(added["revision"], 1);
    assert_eq!(added["dirty"], true);
    assert_eq!(added["pageCount"], 3);
    let items = outline(&mut engine, &mut session);
    assert_eq!(
        items[4],
        json!({"id": 4, "title": "PDF4All P2 nav", "depth": 0, "page": 2})
    );
    assert_valid_outline(document(&session));
    let doc = document(&session);
    let root = doc.resolve_id(id(10)).as_dict().unwrap();
    assert_eq!(root.get(b"Count".as_slice()), Some(&Object::Integer(5)));
    let undone = op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(undone["step"], "add bookmark");
    assert_eq!(serialized(&session), start, "undo restores every object");
    let redone = op(&mut engine, &mut session, json!({"op": "redo"}));
    assert_eq!(redone["step"], "add bookmark");
    assert_eq!(outline(&mut engine, &mut session).len(), 5);
}

#[test]
fn the_first_bookmark_makes_an_outline_with_a_positive_count() {
    let (mut engine, mut session) = session_with(classic(&strings(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
    ])));
    assert!(outline(&mut engine, &mut session).is_empty());
    op(
        &mut engine,
        &mut session,
        json!({"op": "outline-add", "title": "Début", "page": 0}),
    );
    op(
        &mut engine,
        &mut session,
        json!({"op": "outline-add", "title": "Second", "page": 0}),
    );
    assert_valid_outline(document(&session));
    let doc = document(&session);
    let catalog = doc.resolve_id(doc.root()).as_dict().unwrap();
    let root = dict(doc, catalog.get(b"Outlines".as_slice()).unwrap());
    assert_eq!(
        root.get(b"Type".as_slice()),
        Some(&Object::name("Outlines"))
    );
    assert_eq!(
        root.get(b"Count".as_slice()),
        Some(&Object::Integer(2)),
        "MuPDF writes -1"
    );
    let first = dict(doc, root.get(b"First".as_slice()).unwrap());
    assert_eq!(
        first.get(b"Dest".as_slice()),
        Some(&Object::Array(vec![
            Object::Reference(id(3)),
            Object::name("Fit")
        ]))
    );
    let title = first.get(b"Title".as_slice()).unwrap().as_string().unwrap();
    assert_eq!(decode_text_string(title), "Début");
    assert_eq!(
        titles(&mut engine, &mut session),
        vec![
            ("Début".to_string(), 0, json!(0)),
            ("Second".to_string(), 0, json!(0))
        ]
    );
}

#[test]
fn rename_writes_only_the_title() {
    // Style, an XYZ action, a GoToR, a Named and a JavaScript action: MuPDF's
    // iterator update rewrote or dropped every one of them.
    let (mut engine, mut session) = session_with(classic(&strings(&[
        "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        "<< /Type /Outlines /First 5 0 R /Last 8 0 R /Count 4 >>",
        "<< /Title (A) /Parent 4 0 R /Next 6 0 R /C [1 0 0] /F 3 /A << /S /GoTo /D [3 0 R /XYZ 10 700 0] >> >>",
        "<< /Title (B) /Parent 4 0 R /Prev 5 0 R /Next 7 0 R /A << /S /GoToR /F (other.pdf) /D [0 /Fit] >> >>",
        "<< /Title (C) /Parent 4 0 R /Prev 6 0 R /Next 8 0 R /A << /S /Named /N /LastPage >> >>",
        "<< /Title (D) /Parent 4 0 R /Prev 7 0 R /A << /S /JavaScript /JS (app.alert\\(1\\)) >> >>",
    ])));
    let before: Vec<Object> = (5..=8)
        .map(|n| document(&session).resolve_id(id(n)).clone())
        .collect();
    for (index, title) in ["A2", "B2", "Ç2", "D2"].iter().enumerate() {
        op(
            &mut engine,
            &mut session,
            json!({"op": "outline-set", "outlineId": index, "title": title}),
        );
    }
    for (n, before) in (5..=8).zip(before) {
        let mut expected = before.as_dict().unwrap().clone();
        let after = document(&session)
            .resolve_id(id(n))
            .as_dict()
            .unwrap()
            .clone();
        expected.insert(
            b"Title".to_vec(),
            after.get(b"Title".as_slice()).unwrap().clone(),
        );
        assert_eq!(after, expected, "only /Title changed on {n}");
    }
    let items = outline(&mut engine, &mut session);
    assert_eq!(items[0]["title"], "A2");
    assert_eq!(items[0]["page"], 0);
    assert_eq!(items[1]["uri"], "file:other.pdf#page=1&view=Fit");
    assert_eq!(
        items[1]["page"],
        Value::Null,
        "another file's page is not a page here"
    );
    assert_eq!(items[2]["title"], "Ç2");
    assert_eq!(items[2]["page"], 0, "LastPage of a one-page document");
    assert_eq!(
        items[3],
        json!({"id": 3, "title": "D2", "depth": 0, "page": null})
    );
    let undone = op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(undone["step"], "rename bookmark");
}

/// A consistent outline: `(title, page, open, children)`, written with
/// every link and count right. Pages are objects 3..3+pages; the root is
/// object 3+pages and items follow in preorder.
struct Node(&'static str, usize, bool, Vec<Node>);

fn outline_pdf(pages: usize, nodes: &[Node]) -> Vec<u8> {
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + i)).collect();
    let root = 3 + pages;
    let mut objects = vec![
        format!("<< /Type /Catalog /Pages 2 0 R /Outlines {root} 0 R >>"),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {pages} >>",
            kids.join(" ")
        ),
    ];
    for _ in 0..pages {
        objects.push("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".into());
    }
    // Preorder: each node and its parent's index.
    fn flatten<'n>(
        nodes: &'n [Node],
        parent: Option<usize>,
        out: &mut Vec<(&'n Node, Option<usize>)>,
    ) {
        for node in nodes {
            let index = out.len();
            out.push((node, parent));
            flatten(&node.3, Some(index), out);
        }
    }
    let mut order = Vec::new();
    flatten(nodes, None, &mut order);
    let number = |index: usize| root + 1 + index;
    let children = |parent: Option<usize>| -> Vec<usize> {
        (0..order.len()).filter(|&i| order[i].1 == parent).collect()
    };
    fn shown(index: Option<usize>, order: &[(&Node, Option<usize>)]) -> i64 {
        (order.iter().enumerate())
            .filter(|(_, (_, parent))| *parent == index)
            .map(|(child, (node, _))| 1 + if node.2 { shown(Some(child), order) } else { 0 })
            .sum()
    }
    let mut bodies = Vec::new();
    for (index, (node, parent)) in order.iter().enumerate() {
        let group = children(*parent);
        let at = group.iter().position(|&i| i == index).unwrap();
        let mut body = format!(
            "<< /Title ({}) /Parent {} 0 R /Dest [{} 0 R /Fit]",
            node.0,
            parent.map_or(root, number),
            3 + node.1
        );
        if at > 0 {
            body += &format!(" /Prev {} 0 R", number(group[at - 1]));
        }
        if at + 1 < group.len() {
            body += &format!(" /Next {} 0 R", number(group[at + 1]));
        }
        let kids = children(Some(index));
        if let (Some(first), Some(last)) = (kids.first(), kids.last()) {
            let count = shown(Some(index), &order);
            let count = if node.2 { count } else { -count };
            body += &format!(
                " /First {} 0 R /Last {} 0 R /Count {count}",
                number(*first),
                number(*last)
            );
        }
        bodies.push(body + " >>");
    }
    let top = children(None);
    objects.push(format!(
        "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {} >>",
        number(top[0]),
        number(*top.last().unwrap()),
        shown(None, &order)
    ));
    objects.extend(bodies);
    classic(&objects)
}

fn tree() -> Vec<Node> {
    vec![
        Node(
            "A",
            0,
            true,
            vec![
                Node("A1", 1, true, vec![Node("A1a", 1, true, vec![])]),
                Node("A2", 2, false, vec![Node("A2a", 2, true, vec![])]),
            ],
        ),
        Node(
            "B",
            1,
            false,
            vec![
                Node(
                    "B1",
                    1,
                    true,
                    vec![Node("B1a", 0, true, vec![]), Node("B1b", 0, true, vec![])],
                ),
                Node("B2", 2, true, vec![]),
            ],
        ),
        Node("C", 2, true, vec![]),
        Node(
            "D",
            0,
            true,
            vec![Node("D1", 0, false, vec![Node("D1a", 1, true, vec![])])],
        ),
    ]
}

#[test]
fn delete_takes_the_subtree_and_keeps_links_and_signed_counts() {
    let (mut engine, mut session) = session_with(outline_pdf(3, &tree()));
    assert_valid_outline(document(&session));
    let start = serialized(&session);
    let names = |engine: &mut AppEngine, session: &mut AppSession| -> Vec<String> {
        titles(engine, session)
            .into_iter()
            .map(|(title, _, _)| title)
            .collect()
    };
    assert_eq!(
        names(&mut engine, &mut session),
        [
            "A", "A1", "A1a", "A2", "A2a", "B", "B1", "B1a", "B1b", "B2", "C", "D", "D1", "D1a"
        ]
    );
    // B1 under closed B: B's /Count goes from -5 to -2; the root's is unchanged.
    op(
        &mut engine,
        &mut session,
        json!({"op": "outline-delete", "outlineId": 6}),
    );
    assert_valid_outline(document(&session));
    assert_eq!(
        names(&mut engine, &mut session),
        [
            "A", "A1", "A1a", "A2", "A2a", "B", "B2", "C", "D", "D1", "D1a"
        ]
    );
    // A1a, the only child of open A1 under open A: A1 loses /First, /Last and /Count.
    op(
        &mut engine,
        &mut session,
        json!({"op": "outline-delete", "outlineId": 2}),
    );
    assert_valid_outline(document(&session));
    // The first top-level item, then the last.
    op(
        &mut engine,
        &mut session,
        json!({"op": "outline-delete", "outlineId": 0}),
    );
    assert_valid_outline(document(&session));
    let items = names(&mut engine, &mut session);
    assert_eq!(items, ["B", "B2", "C", "D", "D1", "D1a"]);
    op(
        &mut engine,
        &mut session,
        json!({"op": "outline-delete", "outlineId": 3}),
    );
    assert_valid_outline(document(&session));
    assert_eq!(names(&mut engine, &mut session), ["B", "B2", "C"]);
    for _ in 0..2 {
        op(
            &mut engine,
            &mut session,
            json!({"op": "outline-delete", "outlineId": 0}),
        );
        assert_valid_outline(document(&session));
    }
    assert!(names(&mut engine, &mut session).is_empty());
    let doc = document(&session);
    let root = doc.resolve_id(id(6)).as_dict().unwrap();
    assert_eq!(root.get(b"First".as_slice()), None);
    assert_eq!(root.get(b"Count".as_slice()), None, "nothing is visible");
    // Reachable objects only: the deleted items are not saved.
    let bytes = saved(&session);
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("(A1a)"), "a deleted item is not written");
    for _ in 0..6 {
        let undone = op(&mut engine, &mut session, json!({"op": "undo"}));
        assert_eq!(undone["step"], "delete bookmark");
    }
    assert_eq!(serialized(&session), start);
}

/// A small deterministic generator for the property tests.
struct Rng(u64);

impl Rng {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        if bound == 0 { 0 } else { self.0 % bound }
    }
}

#[test]
fn random_outline_edits_stay_consistent_and_undo_to_the_start() {
    for seed in 1..=8u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let (mut engine, mut session) = session_with(outline_pdf(3, &tree()));
        let start = serialized(&session);
        for step in 0..50 {
            let count = outline(&mut engine, &mut session).len() as u64;
            let request = match rng.next(6) {
                0 => json!({"op": "outline-add", "title": format!("n{step}"), "page": rng.next(3)}),
                1 => {
                    json!({"op": "outline-set", "outlineId": rng.next(count + 1), "title": format!("r{step}")})
                }
                2 | 3 => json!({"op": "outline-delete", "outlineId": rng.next(count + 1)}),
                4 => json!({"op": "undo"}),
                _ => json!({"op": "redo"}),
            };
            let reply = dispatch(&mut engine, &mut session, request.clone());
            let expect_ok = match request["op"].as_str().unwrap() {
                "outline-set" | "outline-delete" => request["outlineId"].as_u64().unwrap() < count,
                "outline-add" => true,
                _ => reply["ok"] == true,
            };
            assert_eq!(
                reply["ok"] == true,
                expect_ok,
                "seed {seed} {request}: {reply}"
            );
            assert_valid_outline(document(&session));
        }
        while dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"] == true {}
        assert_eq!(serialized(&session), start, "seed {seed}");
    }
}

#[test]
fn odd_outlines_list_and_edit_without_panicking() {
    // A /Next cycle, an item shared by two parents, a /First back to an
    // ancestor, items written directly into others, junk titles and counts,
    // a dangling /Next and a non-dictionary item.
    let objects = strings(&[
        "<< /Type /Catalog /Pages 2 0 R /Outlines << /First 5 0 R /Count -7.6 >> >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        "<< /Title (a) /Next 6 0 R /First 8 0 R /Count 9223372036854775807 /Dest [3 0 R /Fit] >>",
        "<< /Title /NotAString /Next << /Title (direct) /Next << /Title (deeper) /Next 7 0 R /First << /Title (inner) >> >> >> >>",
        "<< /Title <FEFF00E9> /Next 5 0 R /First 8 0 R /Count -2 /Dest [99 0 R /Fit] >>",
        "<< /Title (shared) /Next 9 0 R /First 5 0 R >>",
        "<< /Title (end) /Next 99 0 R /First 10 0 R /Dest (nowhere) >>",
        "42",
    ]);
    let (mut engine, mut session) = session_with(classic(&objects));
    let listed = titles(&mut engine, &mut session);
    let names: Vec<&str> = listed.iter().map(|(title, _, _)| title.as_str()).collect();
    assert_eq!(
        names,
        ["a", "shared", "end", "", "direct", "deeper", "inner", "é"]
    );
    let items = outline(&mut engine, &mut session);
    assert_eq!(
        items[7]["uri"], "#page=1&view=Fit",
        "a dead destination is no page"
    );
    assert_eq!(items[7]["page"], Value::Null);
    assert_eq!(items[2]["uri"], "#nameddest=nowhere");
    let start = serialized(&session);
    let mut rng = Rng(0x5eed);
    for step in 0..40 {
        let count = outline(&mut engine, &mut session).len() as u64;
        let request = match rng.next(4) {
            0 => json!({"op": "outline-add", "title": format!("n{step}"), "page": rng.next(2)}),
            1 => json!({"op": "outline-set", "outlineId": rng.next(count.max(1)), "title": "r"}),
            2 => json!({"op": "outline-delete", "outlineId": rng.next(count.max(1))}),
            _ => json!({"op": "undo"}),
        };
        let reply = dispatch(&mut engine, &mut session, request.clone());
        if request["op"] != "undo" && count > 0 {
            assert_eq!(reply["ok"], true, "{request}: {reply}");
        }
        let _ = Outline::read(document(&session));
    }
    while dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"] == true {}
    assert_eq!(serialized(&session), start);

    // Deleting a direct item gives its neighbours objects in the same step.
    let (mut engine, mut session) = session_with(classic(&objects));
    op(
        &mut engine,
        &mut session,
        json!({"op": "outline-delete", "outlineId": 4}),
    );
    let names: Vec<String> = titles(&mut engine, &mut session)
        .into_iter()
        .map(|(title, _, _)| title)
        .collect();
    assert_eq!(names, ["a", "shared", "end", "", "deeper", "inner", "é"]);
    let deeper = Outline::read(document(&session));
    assert!(
        deeper.items[4].place.keys.is_empty(),
        "the next item is its own object now"
    );
    op(
        &mut engine,
        &mut session,
        json!({"op": "outline-add", "title": "z", "page": 1}),
    );
    let doc = document(&session);
    let catalog = doc.resolve_id(doc.root()).as_dict().unwrap();
    assert!(
        catalog
            .get(b"Outlines".as_slice())
            .unwrap()
            .as_reference()
            .is_some(),
        "the root becomes an object for /Parent to name"
    );
}

#[test]
fn outline_refusals_name_the_problem_and_change_nothing() {
    let (mut engine, mut session) = session_with(nav());
    for (request, message) in [
        (
            json!({"op": "outline-add", "title": "   ", "page": 0}),
            "outline-add: a bookmark needs a title",
        ),
        (
            json!({"op": "outline-add", "title": "x".repeat(4097), "page": 0}),
            "outline-add: title must be a string of at most 4096 characters",
        ),
        (
            json!({"op": "outline-add", "title": 5, "page": 0}),
            "outline-add: title must be a string of at most 4096 characters",
        ),
        (
            json!({"op": "outline-add", "title": "fine", "page": 3}),
            "outline-add: page must be an integer in 0..2",
        ),
        (
            json!({"op": "outline-add", "page": 3}),
            "outline-add: page must be an integer in 0..2",
        ),
        (
            json!({"op": "outline-set", "outlineId": 9, "title": "x"}),
            "outline-set: no bookmark 9",
        ),
        (
            json!({"op": "outline-set", "outlineId": -1, "title": "x"}),
            "outline-set: id must be a non-negative integer",
        ),
        (
            json!({"op": "outline-set", "outlineId": 1.5, "title": "x"}),
            "outline-set: id must be a non-negative integer",
        ),
        (
            json!({"op": "outline-set", "outlineId": 9, "title": ""}),
            "outline-set: a bookmark needs a title",
        ),
        (
            json!({"op": "outline-delete", "outlineId": 1e300}),
            "outline-delete: no bookmark 1e+300",
        ),
        (
            json!({"op": "outline-delete"}),
            "outline-delete: id must be a non-negative integer",
        ),
    ] {
        let reply = dispatch(&mut engine, &mut session, request.clone());
        assert_eq!(reply["ok"], false, "{request}");
        assert_eq!(reply["error"], message, "{request}");
        assert_eq!(reply["revision"], 0);
    }
    // A 4096-unit title of astral characters is 2048 of them.
    let astral = "𝄞".repeat(2048);
    op(
        &mut engine,
        &mut session,
        json!({"op": "outline-add", "title": astral, "page": 0}),
    );
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "outline-add", "title": "𝄞".repeat(2049), "page": 0}),
    );
    assert_eq!(reply["ok"], false);

    let mut empty = AppSession::default();
    for (request, message) in [
        (json!({"op": "outline"}), "outline: no document open"),
        (json!({"op": "links", "page": 0}), "links: no document open"),
        (
            json!({"op": "outline-add", "title": "x", "page": 0}),
            "outline-add: no document open",
        ),
        (
            json!({"op": "link-set", "page": 0, "annotId": 1, "rect": [0, 0, 1, 1]}),
            "link-set: no document open",
        ),
    ] {
        let reply = dispatch(&mut engine, &mut empty, request.clone());
        assert_eq!(reply["error"], message, "{request}");
    }
    let reply = dispatch(
        &mut engine,
        &mut empty,
        json!({"op": "outline-add", "title": "x", "page": 0}),
    );
    assert_eq!(reply["reason"], "no-document");
}

// -------------------------------------------------------------------- links

#[test]
fn nav_links_list_as_page_uri_and_blocked_in_page_space() {
    let (mut engine, mut session) = session_with(nav());
    let first = links(&mut engine, &mut session, 0);
    assert_eq!(
        Value::Array(first),
        json!([
            {"id": 0, "annotId": 15, "rect": [72.0, 62.0, 200.0, 112.0], "kind": "page", "page": 1},
            {"id": 1, "annotId": 16, "rect": [72.0, 162.0, 300.0, 192.0], "kind": "uri", "uri": "https://example.com/"},
        ])
    );
    assert!(links(&mut engine, &mut session, 1).is_empty());
    let third = links(&mut engine, &mut session, 2);
    assert_eq!(third[1]["kind"], "blocked");
    assert_eq!(third[1]["uri"], "file:///etc/hosts");
    let reply = dispatch(&mut engine, &mut session, json!({"op": "links", "page": 3}));
    assert_eq!(reply["error"], "links: page must be an integer in 0..2");
    assert_eq!(reply["reason"], "invalid");
}

#[test]
fn link_targets_follow_mupdf_except_where_it_resolved_uris_to_pages() {
    let actions = [
        (
            "/Dest [4 0 R /XYZ 10 700 2]",
            json!({"kind": "page", "page": 1}),
        ),
        ("/Dest [1 /Fit]", json!({"kind": "page", "page": 1})),
        (
            "/Dest [9 /Fit]",
            json!({"kind": "blocked", "uri": "#page=10&view=Fit"}),
        ),
        (
            "/Dest [99 0 R /FitH 500]",
            json!({"kind": "blocked", "uri": "#page=1&view=FitH,500"}),
        ),
        (
            "/Dest [6 0 R /Fit]",
            json!({"kind": "blocked", "uri": "#page=1&view=Fit"}),
        ),
        (
            "/Dest [99 0 R /XYZ 10 700 2]",
            json!({"kind": "blocked", "uri": "#page=1&zoom=200,10,700"}),
        ),
        (
            "/Dest [99 0 R /FitR 1 2 3 4]",
            json!({"kind": "blocked", "uri": "#page=1&viewrect=1,2,2,2"}),
        ),
        (
            "/Dest [99 0 R /FitV 30]",
            json!({"kind": "blocked", "uri": "#page=1&view=FitV,30"}),
        ),
        (
            "/Dest [99 0 R]",
            json!({"kind": "blocked", "uri": "#page=1"}),
        ),
        ("/Dest /named1", json!({"kind": "page", "page": 2})),
        ("/Dest (named1)", json!({"kind": "page", "page": 2})),
        (
            "/Dest (missing)",
            json!({"kind": "blocked", "uri": "#nameddest=missing"}),
        ),
        (
            "/Dest (sp ace&=#)",
            json!({"kind": "blocked", "uri": "#nameddest=sp%20ace%26%3D%23"}),
        ),
        (
            "/A << /S /GoTo /D (named1) >>",
            json!({"kind": "page", "page": 2}),
        ),
        (
            "/A << /S /URI /URI (https://example.com/#page=3) >>",
            json!({"kind": "uri", "uri": "https://example.com/#page=3"}),
        ),
        (
            "/A << /S /URI /URI (HTTPS://Example.com/) >>",
            json!({"kind": "uri", "uri": "HTTPS://Example.com/"}),
        ),
        (
            "/A << /S /URI /URI (rel/x.html) >>",
            json!({"kind": "uri", "uri": "https://base.example/rel/x.html"}),
        ),
        (
            "/A << /S /URI /URI (javascript:alert\\(1\\)) >>",
            json!({"kind": "blocked", "uri": "javascript:alert(1)"}),
        ),
        (
            "/A << /S /URI /URI <FEFF006D00610069006C0074006F003A00E9> >>",
            json!({"kind": "uri", "uri": "mailto:é"}),
        ),
        (
            "/A << /S /GoToR /F (a b%.pdf) /D /nm >>",
            json!({"kind": "blocked", "uri": "file:a%20b%25.pdf#nameddest=nm"}),
        ),
        (
            "/A << /S /GoToR /F << /F (f.pdf) /UF (/u.pdf) >> /D [2 /XYZ 1.5 0.1 0] >>",
            json!({"kind": "blocked", "uri": "file:///u.pdf#page=3&zoom=nan,1.5,.1"}),
        ),
        (
            "/A << /S /GoToR /F (x.pdf) >>",
            json!({"kind": "blocked", "uri": "file:x.pdf#page=1"}),
        ),
        (
            "/A << /S /Launch /F (app.exe) >>",
            json!({"kind": "blocked", "uri": "file:app.exe"}),
        ),
        (
            "/A << /S /Named /N /NextPage >>",
            json!({"kind": "page", "page": 1}),
        ),
        (
            "/A << /S /Named /N /PrevPage >>",
            json!({"kind": "page", "page": 0}),
        ),
        (
            "/A << /S /Named /N /LastPage >>",
            json!({"kind": "page", "page": 2}),
        ),
        (
            "/AA << /U << /S /URI /URI (https://aa.example/) >> >>",
            json!({"kind": "uri", "uri": "https://aa.example/"}),
        ),
        (
            "/AA << /D << /S /URI /URI (https://d.example/) >> >>",
            json!({"kind": "uri", "uri": "https://d.example/"}),
        ),
        (
            "/Dest null /A << /S /URI /URI (https://destnull/) >>",
            json!({"kind": "uri", "uri": "https://destnull/"}),
        ),
        (
            "/Dest [4 0 R /Fit] /A << /S /URI /URI (https://both/) >>",
            json!({"kind": "page", "page": 1}),
        ),
    ];
    let unlisted = [
        "",
        "/Dest []",
        "/Dest 5",
        "/Dest << /D [4 0 R /Fit] >>",
        "/A << /S /Named /N /Print >>",
        "/A << /S /JavaScript /JS (x) >>",
        "/A << /S /GoTo >>",
        "/A << /S /GoToR /D [0 /Fit] >>",
        "/A << /S /Launch /Win << /F (cmd.exe) >> >>",
        "/A [<< /S /URI /URI (https://arr/) >>]",
    ];
    let mut annots: Vec<String> = actions
        .iter()
        .map(|(action, _)| action.to_string())
        .chain(unlisted.iter().map(|action| action.to_string()))
        .enumerate()
        .map(|(i, action)| {
            format!(
                "<< /Type /Annot /Subtype /Link /Rect [10 {} 100 {}] {action} >>",
                10 + i,
                18 + i
            )
        })
        .collect();
    annots.push("<< /Type /Annot /Subtype /Link /A << /S /URI /URI (https://norect/) >> >>".into());
    annots.push(
        "<< /Type /Annot /Subtype /Text /Rect [0 0 9 9] /A << /S /URI /URI (https://text/) >> >>"
            .into(),
    );
    let (mut engine, mut session) = session_with(classic(&[
        "<< /Type /Catalog /Pages 2 0 R /Dests << /named1 [5 0 R /Fit] >> /URI << /Base (https://base.example/) >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>".into(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [{}] >>", annots.join(" ")),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".into(),
        "<< /Foo 1 >>".into(),
    ]));
    let listed = links(&mut engine, &mut session, 0);
    assert_eq!(listed.len(), actions.len());
    for (item, (action, expected)) in listed.iter().zip(&actions) {
        let mut got = serde_json::Map::new();
        for key in ["kind", "page", "uri"] {
            if let Some(value) = item.get(key) {
                got.insert(key.into(), value.clone());
            }
        }
        assert_eq!(Value::Object(got), *expected, "{action}");
        assert_eq!(
            item["annotId"], 0,
            "a link written into /Annots has no object"
        );
    }
}

#[test]
fn rects_follow_rotation_crop_box_and_user_unit() {
    let doc_bytes = classic(&strings(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R 6 0 R] /Count 4 /Rotate 90 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [7 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Rotate 180 /Annots [7 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /CropBox [100 50 500 750] /Rotate -90 /Annots [7 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [-100 -100 512 692] /UserUnit 2 /Rotate 0 /Annots [7 0 R] >>",
        "<< /Type /Annot /Subtype /Link /Rect [200 720 72 700] /A << /S /URI /URI (https://x/) >> >>",
    ]));
    let (mut engine, mut session) = session_with(doc_bytes);
    // MuPDF 1.28.1's getLinks bounds for the same pages.
    let expected = [
        [700.0, 72.0, 720.0, 200.0],
        [412.0, 700.0, 540.0, 720.0],
        [30.0, 300.0, 50.0, 428.0],
        [344.0, -56.0, 600.0, -16.0],
    ];
    for (page, rect) in expected.iter().enumerate() {
        let items = links(&mut engine, &mut session, page);
        assert_eq!(items[0]["rect"], json!(rect), "page {page}");
    }
    // Page space round-trips through /Rect.
    let doc = document(&session);
    for page in tandem_pdf_engine::app::info::pages(doc) {
        let space = PageSpace::of(doc, page.dict);
        let rect = [10.5, 20.25, 30.0, 40.0];
        let back = space.to_page(space.to_user(rect));
        for (a, b) in back.iter().zip(rect) {
            assert!((a - b).abs() < 1e-9, "{back:?}");
        }
    }
}

#[test]
fn link_add_writes_a_goto_or_uri_annotation_in_one_step() {
    let (mut engine, mut session) = session_with(nav());
    let start = serialized(&session);
    let page = op(
        &mut engine,
        &mut session,
        json!({"op": "link-add", "page": 0, "rect": [72, 400, 200, 430], "target": {"page": 2}}),
    );
    assert_eq!(page["revision"], 1);
    let uri = op(
        &mut engine,
        &mut session,
        json!({"op": "link-add", "page": 0, "rect": [72, 350, 200, 380], "target": {"uri": "https://example.org/x"}}),
    );
    assert_eq!(uri["revision"], 2);
    let items = links(&mut engine, &mut session, 0);
    let kinds: Vec<&str> = items
        .iter()
        .map(|item| item["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["page", "uri", "page", "uri"]);
    assert_eq!(items[2]["rect"], json!([72.0, 400.0, 200.0, 430.0]));
    assert_eq!(items[2]["page"], 2);
    assert_eq!(items[3]["uri"], "https://example.org/x");
    let doc = document(&session);
    let annot_id = id(items[2]["annotId"].as_u64().unwrap() as u32);
    let annot = doc.resolve_id(annot_id).as_dict().unwrap();
    assert_eq!(
        annot.get(b"Rect".as_slice()),
        Some(&Object::Array(vec![
            Object::Real(72.0),
            Object::Real(362.0),
            Object::Real(200.0),
            Object::Real(392.0)
        ]))
    );
    let action = dict(doc, annot.get(b"A".as_slice()).unwrap());
    assert_eq!(action.get(b"S".as_slice()), Some(&Object::name("GoTo")));
    assert_eq!(
        action.get(b"D".as_slice()),
        Some(&Object::Array(vec![
            Object::Reference(id(5)),
            Object::name("Fit")
        ]))
    );
    let border = dict(doc, annot.get(b"BS".as_slice()).unwrap());
    assert_eq!(border.get(b"W".as_slice()), Some(&Object::Integer(0)));
    assert_eq!(annot.get(b"P".as_slice()), Some(&Object::Reference(id(3))));
    for step in ["add link", "add link"] {
        assert_eq!(
            op(&mut engine, &mut session, json!({"op": "undo"}))["step"],
            step
        );
    }
    assert_eq!(serialized(&session), start);
}

#[test]
fn link_add_on_a_page_written_into_kids_gives_it_an_object() {
    let (mut engine, mut session) = session_with(classic(&strings(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [<< /Type /Page /MediaBox [0 0 300 300] /Annots /Junk >> 3 0 R] /Count 2 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
    ])));
    let start = serialized(&session);
    op(
        &mut engine,
        &mut session,
        json!({"op": "link-add", "page": 1, "rect": [10, 10, 50, 50], "target": {"page": 0}}),
    );
    op(
        &mut engine,
        &mut session,
        json!({"op": "link-add", "page": 0, "rect": [10, 10, 50, 50], "target": {"uri": "mailto:a@b.c"}}),
    );
    let ids = session.doc.as_ref().unwrap().page_ids();
    assert!(ids.iter().all(Option::is_some), "{ids:?}");
    assert_eq!(links(&mut engine, &mut session, 1)[0]["page"], 0);
    assert_eq!(links(&mut engine, &mut session, 0)[0]["kind"], "uri");
    while dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"] == true {}
    assert_eq!(serialized(&session), start);
}

#[test]
fn link_refusals_match_engine_ts() {
    let (mut engine, mut session) = session_with(nav());
    for (request, message) in [
        (
            json!({"op": "link-add", "page": 0, "rect": [72, 900, 200, 930], "target": {"page": 1}}),
            "link-add: rect [72, 900, 200, 930] is off the page",
        ),
        (
            json!({"op": "link-add", "page": 0, "rect": [-20, 400, 200, 430], "target": {"page": 1}}),
            "link-add: rect [-20, 400, 200, 430] is off the page",
        ),
        (
            json!({"op": "link-add", "page": 0, "rect": [72, 400, 72, 430], "target": {"page": 1}}),
            "link-add: rect must have a positive area",
        ),
        (
            json!({"op": "link-add", "page": 0, "rect": [72, 400, 200], "target": {"page": 1}}),
            "link-add: rect must be 4 finite numbers",
        ),
        (
            json!({"op": "link-add", "page": 0, "rect": [72, 400, 200, {"$number": "NaN"}], "target": {"page": 1}}),
            "link-add: rect must be 4 finite numbers",
        ),
        (
            json!({"op": "link-add", "page": 9, "rect": [72, 400, 200, 430], "target": {"page": 1}}),
            "link-add: page must be an integer in 0..2",
        ),
        (
            json!({"op": "link-add", "page": 0, "rect": [72, 400, 200, 430], "target": {"page": 7}}),
            "link-add: page must be an integer in 0..2",
        ),
        (
            json!({"op": "link-add", "page": 0, "rect": [72, 400, 200, 430], "target": {"page": null}}),
            "link-add: page must be an integer in 0..2",
        ),
        (
            json!({"op": "link-add", "page": 0, "rect": [72, 400, 200, 430], "target": {"uri": "file:///etc/hosts"}}),
            "link-add: file: is not a link this app opens",
        ),
        (
            json!({"op": "link-add", "page": 0, "rect": [72, 400, 200, 430], "target": {"uri": "JavaScript:alert(1)"}}),
            "link-add: javascript: is not a link this app opens",
        ),
        (
            json!({"op": "link-add", "page": 0, "rect": [72, 400, 200, 430], "target": {"uri": "  https://example.org/pad "}}),
            "link-add: a URI without a scheme is not a link this app opens",
        ),
        (
            json!({"op": "link-add", "page": 0, "rect": [72, 400, 200, 430], "target": {"uri": format!("mailto:{}@example.com", "x".repeat(2048))}}),
            "link-add: a link URI holds at most 2048 characters",
        ),
        (
            json!({"op": "link-add", "page": 0, "rect": [72, 400, 200, 430], "target": {}}),
            "link-add: target must be {page} or {uri}",
        ),
        (
            json!({"op": "link-add", "page": 0, "rect": [72, 400, 200, 430], "target": "https://x/"}),
            "link-add: target must be {page} or {uri}",
        ),
        (
            json!({"op": "link-add", "page": 0, "rect": [72, 400, 200, 430]}),
            "link-add: target must be {page} or {uri}",
        ),
        (
            json!({"op": "link-delete", "page": 0, "linkId": 9}),
            "link-delete: no link 9 on this page",
        ),
        (
            json!({"op": "link-delete", "page": 0, "linkId": "x"}),
            "link-delete: no link x on this page",
        ),
        (
            json!({"op": "link-delete", "page": 0, "linkId": -1}),
            "link-delete: no link -1 on this page",
        ),
        (
            json!({"op": "link-delete", "page": 0}),
            "link-delete: no link undefined on this page",
        ),
    ] {
        let reply = dispatch(&mut engine, &mut session, request.clone());
        assert_eq!(reply["ok"], false, "{request}");
        assert_eq!(reply["error"], message, "{request}");
        assert_eq!(reply["dirty"], false);
    }
    assert_eq!(
        dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"],
        false
    );
    assert_eq!(scheme_of("Mailto:x"), "mailto:");
    assert_eq!(scheme_of("1x:y"), "");
    assert_eq!(scheme_of("no-colon"), "");
}

/// fixtures/p6/links.pdf's structure: L1 (10) URI, L2 (11) /Dest to page
/// 3, L5 (12) no target, L3 (13) ftp, L4 (14) on rotated page 2.
fn links_pdf() -> Vec<u8> {
    classic(&strings(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [10 0 R 11 0 R 12 0 R 13 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Rotate 90 /Annots [14 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        "<< >>",
        "<< >>",
        "<< >>",
        "<< >>",
        "<< /Type /Annot /Subtype /Link /P 3 0 R /Rect [72 700 200 720] /Border [0 0 0] /F 4 /A << /S /URI /URI (https://example.org/one) >> >>",
        "<< /Type /Annot /Subtype /Link /P 3 0 R /Rect [72 650 200 670] /Border [0 0 0] /F 4 /Dest [5 0 R /Fit] >>",
        "<< /Type /Annot /Subtype /Link /P 3 0 R /Rect [72 630 200 640] /Border [0 0 0] /F 4 >>",
        "<< /Type /Annot /Subtype /Link /P 3 0 R /Rect [72 600 200 620] /Border [0 0 0] /F 4 /A << /S /URI /URI (ftp://example.org/x) >> >>",
        "<< /Type /Annot /Subtype /Link /P 4 0 R /Rect [72 700 200 720] /Border [0 0 0] /F 4 /A << /S /URI /URI (https://example.org/rot) >> >>",
    ]))
}

#[test]
fn link_set_refuses_with_one_token_and_detects_no_ops() {
    let (mut engine, mut session) = session_with(links_pdf());
    let ids: Vec<u64> = links(&mut engine, &mut session, 0)
        .iter()
        .map(|item| item["annotId"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, [10, 11, 13], "the bare L5 is not a link MuPDF lists");
    for (request, detail, message) in [
        (
            json!({"page": 0, "annotId": 10}),
            "target",
            "link-set: rect or target is required",
        ),
        (
            json!({"page": 0, "annotId": 10, "rect": [72, 900, 200, 930]}),
            "rect",
            "link-set: rect [72, 900, 200, 930] is off the page",
        ),
        (
            json!({"page": 0, "annotId": 10, "rect": null}),
            "rect",
            "link-set: rect must be 4 finite numbers",
        ),
        (
            json!({"page": 0, "annotId": 0, "rect": [80, 80, 210, 100]}),
            "no-such-annot",
            "link-set: page 0 has no link 0",
        ),
        (
            json!({"page": 0, "annotId": "x", "rect": [80, 80, 210, 100]}),
            "no-such-annot",
            "link-set: page 0 has no link x",
        ),
        (
            json!({"page": 0, "annotId": 12, "rect": [80, 80, 210, 100]}),
            "no-such-annot",
            "link-set: page 0 has no link 12",
        ),
        (
            json!({"page": 0, "annotId": 14, "rect": [80, 80, 210, 100]}),
            "no-such-annot",
            "link-set: page 0 has no link 14",
        ),
        (
            json!({"page": 0, "annotId": 10, "target": {}}),
            "target",
            "link-set: target must be {page} or {uri}",
        ),
        (
            json!({"page": 0, "annotId": 10, "target": null}),
            "target",
            "link-set: target must be {page} or {uri}",
        ),
        (
            json!({"page": 0, "annotId": 10, "target": {"page": 9}}),
            "target",
            "link-set: page must be an integer in 0..2",
        ),
        (
            json!({"page": 0, "annotId": 10, "target": {"uri": "ftp://nope"}}),
            "scheme",
            "link-set: ftp: is not a link this app opens",
        ),
        (
            json!({"page": 0, "annotId": 10, "target": {"uri": ""}}),
            "scheme",
            "link-set: a URI without a scheme is not a link this app opens",
        ),
        (
            json!({"page": 0, "annotId": 10, "target": {"uri": format!("https://{}", "x".repeat(2041))}}),
            "scheme",
            "link-set: a link URI holds at most 2048 characters",
        ),
    ] {
        let mut request = request;
        request["op"] = "link-set".into();
        let reply = dispatch(&mut engine, &mut session, request.clone());
        assert_eq!(reply["ok"], false, "{request}");
        assert_eq!(reply["reason"], "invalid", "{request}");
        assert_eq!(reply["detail"], detail, "{request}");
        assert_eq!(reply["error"], message, "{request}");
        assert_eq!(reply["revision"], 0);
    }
    let bad_page = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "link-set", "page": 7, "annotId": 10, "rect": [80, 80, 210, 100]}),
    );
    assert_eq!(bad_page["reason"], "invalid");
    assert_eq!(bad_page.get("detail"), None);

    // A non-link annotation on the page.
    let (mut engine2, mut session2) = session_with(classic(&strings(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>",
        "<< /Type /Annot /Subtype /Text /Rect [0 0 9 9] >>",
    ])));
    let reply = dispatch(
        &mut engine2,
        &mut session2,
        json!({"op": "link-set", "page": 0, "annotId": 4, "rect": [80, 80, 210, 100]}),
    );
    assert_eq!(reply["detail"], "not-link");
    assert_eq!(reply["error"], "link-set: annotation 4 is not a link");

    let same = op(
        &mut engine,
        &mut session,
        json!({"op": "link-set", "page": 0, "annotId": 10, "rect": [72.004, 72, 200, 92], "target": {"uri": "https://example.org/one"}}),
    );
    assert_eq!(same["noop"], true);
    assert_eq!(same["revision"], 0);
    assert_eq!(same.get("pageCount"), None);
    let same_page = op(
        &mut engine,
        &mut session,
        json!({"op": "link-set", "page": 0, "annotId": 11, "target": {"page": 2}}),
    );
    assert_eq!(same_page["noop"], true);
    assert_eq!(
        dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"],
        false
    );
}

#[test]
fn link_set_moves_and_retargets_in_one_step_that_undo_restores() {
    let (mut engine, mut session) = session_with(links_pdf());
    let start = serialized(&session);
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "link-set", "page": 0, "annotId": 11, "rect": [300, 300, 400, 330], "target": {"uri": "https://example.org/both"}}),
    );
    assert_eq!(reply["revision"], 1);
    assert!(reply["rotations"].is_array());
    let doc = document(&session);
    let annot = doc.resolve_id(id(11)).as_dict().unwrap();
    assert_eq!(
        annot.get(b"Dest".as_slice()),
        None,
        "/Dest would shadow the new /A"
    );
    assert_eq!(
        annot.get(b"F".as_slice()),
        Some(&Object::Integer(4)),
        "the rest stays"
    );
    assert_eq!(
        annot.get(b"Rect".as_slice()),
        Some(&Object::Array(vec![
            Object::Real(300.0),
            Object::Real(462.0),
            Object::Real(400.0),
            Object::Real(492.0)
        ]))
    );
    let item = links(&mut engine, &mut session, 0)[1].clone();
    assert_eq!(item["uri"], "https://example.org/both");
    assert_eq!(item["rect"], json!([300.0, 300.0, 400.0, 330.0]));
    assert_eq!(
        op(&mut engine, &mut session, json!({"op": "undo"}))["step"],
        "edit link"
    );
    assert_eq!(serialized(&session), start);
    op(&mut engine, &mut session, json!({"op": "redo"}));
    // Back to a page, then a rect on the rotated page.
    op(
        &mut engine,
        &mut session,
        json!({"op": "link-set", "page": 0, "annotId": 11, "target": {"page": 0}}),
    );
    let item = links(&mut engine, &mut session, 0)[1].clone();
    assert_eq!(
        (item["kind"].as_str(), item["page"].as_u64()),
        (Some("page"), Some(0))
    );
    op(
        &mut engine,
        &mut session,
        json!({"op": "link-set", "page": 1, "annotId": 14, "rect": [650, 100, 690, 250]}),
    );
    assert_eq!(
        links(&mut engine, &mut session, 1)[0]["rect"],
        json!([650.0, 100.0, 690.0, 250.0])
    );
    // A rect-only move of a blocked link is allowed.
    op(
        &mut engine,
        &mut session,
        json!({"op": "link-set", "page": 0, "annotId": 13, "rect": [72, 200, 200, 220]}),
    );
    while dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"] == true {}
    assert_eq!(serialized(&session), start);
}

#[test]
fn link_delete_removes_the_entry_and_what_named_it() {
    // Link 6 is tagged (an OBJR in the structure tree) and has a reply (IRT);
    // link 7 is listed on both pages, so deleting it from one keeps it whole.
    let objects = strings(&[
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 9 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [6 0 R 7 0 R 8 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [7 0 R] >>",
        "<< >>",
        "<< /Type /Annot /Subtype /Link /Rect [10 10 50 50] /A << /S /URI /URI (https://tagged/) >> /StructParent 0 >>",
        "<< /Type /Annot /Subtype /Link /Rect [60 10 90 50] /Dest [4 0 R /Fit] >>",
        "<< /Type /Annot /Subtype /Text /Rect [0 0 9 9] /IRT 6 0 R >>",
        "<< /Type /StructTreeRoot /K [10 0 R] >>",
        "<< /Type /StructElem /S /Link /P 9 0 R /K [<< /Type /OBJR /Obj 6 0 R /Pg 3 0 R >> 0] >>",
    ]);
    let (mut engine, mut session) = session_with(classic(&objects));
    let start = serialized(&session);
    op(
        &mut engine,
        &mut session,
        json!({"op": "link-delete", "page": 0, "linkId": 0}),
    );
    let doc = document(&session);
    let elem = doc.resolve_id(id(10)).as_dict().unwrap();
    assert_eq!(
        elem.get(b"K".as_slice()),
        Some(&Object::Array(vec![Object::Integer(0)]))
    );
    let text = doc.resolve_id(id(8)).as_dict().unwrap();
    assert_eq!(text.get(b"IRT".as_slice()), None);
    let bytes = saved(&session);
    assert!(!String::from_utf8_lossy(&bytes).contains("https://tagged/"));
    assert_eq!(links(&mut engine, &mut session, 0)[0]["annotId"], 7);
    op(
        &mut engine,
        &mut session,
        json!({"op": "link-delete", "page": 0, "linkId": 0}),
    );
    assert_eq!(
        links(&mut engine, &mut session, 1)[0]["annotId"],
        7,
        "page 2 keeps it"
    );
    assert!(links(&mut engine, &mut session, 0).is_empty());
    for _ in 0..2 {
        assert_eq!(
            op(&mut engine, &mut session, json!({"op": "undo"}))["step"],
            "delete link"
        );
    }
    assert_eq!(serialized(&session), start);
}

#[test]
fn nav_ops_refuse_signed_and_locked_documents() {
    let (mut engine, mut session) = session_with(classic(&strings(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R /Outlines 8 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [9 0 R] >>",
        "<< >>",
        "<< /Fields [6 0 R] /SigFlags 3 >>",
        "<< /Type /Annot /Subtype /Widget /FT /Sig /T (sig1) /Rect [72 72 300 120] /P 3 0 R /V 7 0 R >>",
        "<< /Type /Sig /Filter /Adobe.PPKLite >>",
        "<< /Type /Outlines /First 10 0 R /Last 10 0 R /Count 1 >>",
        "<< /Type /Annot /Subtype /Link /Rect [10 10 50 50] /Dest [3 0 R /Fit] >>",
        "<< /Title (only) /Parent 8 0 R /Dest [3 0 R /Fit] >>",
    ])));
    assert_eq!(
        outline(&mut engine, &mut session).len(),
        1,
        "reading works on a signed file"
    );
    assert_eq!(links(&mut engine, &mut session, 0).len(), 1);
    for request in [
        json!({"op": "outline-add", "title": "x", "page": 0}),
        json!({"op": "outline-set", "outlineId": 0, "title": "x"}),
        json!({"op": "outline-delete", "outlineId": 0}),
        json!({"op": "link-add", "page": 0, "rect": [72, 400, 200, 430], "target": {"page": 0}}),
        json!({"op": "link-delete", "page": 0, "linkId": 0}),
        json!({"op": "link-set", "page": 0, "annotId": 9, "rect": [72, 400, 200, 430]}),
    ] {
        let reply = dispatch(&mut engine, &mut session, request.clone());
        assert_eq!(reply["reason"], "read-only", "{request}");
        assert_eq!(
            reply["error"],
            format!(
                "{}: this document is read-only",
                request["op"].as_str().unwrap()
            )
        );
    }
}

#[test]
fn hostile_link_pages_never_panic() {
    let huge = "1e308";
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /Names << /Dests << /Kids [5 0 R] >> >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {huge} {huge}] /UserUnit -3 /Rotate 2147483647.9 /Annots [6 0 R 6 0 R null 7 0 R << /Subtype /Link /Rect [1 2] /Dest /loop >> 99 0 R 8 0 R] >>"
        ),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 0 0] /Rotate -2147483648 /Annots 9 0 R >>".into(),
        "<< /Kids [5 0 R] /Names [(loop) /loop (x) 5 0 R] >>".into(),
        format!("<< /Type /Annot /Subtype /Link /Rect [-{huge} -{huge} {huge} {huge}] /Dest [4 0 R /XYZ {huge} -{huge} {huge}] >>"),
        "<< /Type /Annot /Subtype /Link /Rect [0 0 1 1] /A << /S /GoToR /F << /UF <FEFFD800> >> /D [-9223372036854775808 /FitR 1 2 3] >> >>".into(),
        "<< /Type /Annot /Subtype /Link /Rect 7 0 R /A << /S /Named /N /NextPage >> >>".into(),
        "[8 0 R << /Subtype /Link /Rect [0 0 1 1] /A << /S /URI /URI 9 0 R >> >>]".into(),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    for page in 0..2 {
        let _ = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "links", "page": page}),
        );
    }
    let start = serialized(&session);
    for request in [
        json!({"op": "link-add", "page": 1, "rect": [0, 0, 100, 100], "target": {"page": 0}}),
        json!({"op": "link-add", "page": 0, "rect": [0, 0, 100, 100], "target": {"uri": "https://ok/"}}),
        json!({"op": "link-set", "page": 0, "annotId": 6, "rect": [0, 0, 10, 10], "target": {"page": 1}}),
        json!({"op": "link-set", "page": 0, "annotId": 7, "target": {"uri": "https://new/"}}),
        json!({"op": "link-delete", "page": 0, "linkId": 1}),
        json!({"op": "link-delete", "page": 1, "linkId": 0}),
        json!({"op": "outline-add", "title": "x", "page": 1}),
    ] {
        let _ = dispatch(&mut engine, &mut session, request);
        for page in 0..2 {
            let _ = dispatch(
                &mut engine,
                &mut session,
                json!({"op": "links", "page": page}),
            );
        }
        let _ = dispatch(&mut engine, &mut session, json!({"op": "outline"}));
    }
    while dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"] == true {}
    assert_eq!(serialized(&session), start);
}

#[test]
fn a_huge_outline_lists_its_first_hundred_thousand_and_still_edits() {
    let total = 100_002usize;
    let mut objects = strings(&[
        "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
    ]);
    objects.push(format!(
        "<< /Type /Outlines /First 5 0 R /Last {} 0 R /Count {total} >>",
        4 + total
    ));
    for i in 0..total {
        let number = 5 + i;
        let mut body = format!("<< /Title (t{i}) /Parent 4 0 R /Dest [3 0 R /Fit]");
        if i > 0 {
            body += &format!(" /Prev {} 0 R", number - 1);
        }
        if i + 1 < total {
            body += &format!(" /Next {} 0 R", number + 1);
        }
        objects.push(body + " >>");
    }
    let (mut engine, mut session) = session_with(classic(&objects));
    let items = outline(&mut engine, &mut session);
    assert_eq!(items.len(), 100_000);
    assert_eq!(items[99_999]["title"], "t99999");
    op(
        &mut engine,
        &mut session,
        json!({"op": "outline-delete", "outlineId": 100_001}),
    );
    op(
        &mut engine,
        &mut session,
        json!({"op": "outline-add", "title": "end", "page": 0}),
    );
    assert_valid_outline(document(&session));
    let doc = document(&session);
    let root = doc.resolve_id(id(4)).as_dict().unwrap();
    assert_eq!(
        root.get(b"Count".as_slice()),
        Some(&Object::Integer(total as i64))
    );
}

// ------------------------------------------------------------ review fixes

#[test]
fn link_add_replaces_a_non_array_annots_in_the_page_not_what_it_names() {
    // Page 1's /Annots names its own font, page 2's names the page itself.
    let objects = strings(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Annots 4 0 R >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots 5 0 R >>",
    ]);
    let (mut engine, mut session) = session_with(classic(&objects));
    let start = serialized(&session);
    let font = document(&session).resolve_id(id(4)).clone();
    for page in 0..2 {
        let added = op(
            &mut engine,
            &mut session,
            json!({"op": "link-add", "page": page, "rect": [10, 10, 50, 50], "target": {"uri": "https://e.com/"}}),
        );
        assert_eq!(added["pageCount"], 2);
    }
    let doc = document(&session);
    assert_eq!(doc.resolve_id(id(4)), &font, "the font is untouched");
    for (page, link) in [(3, 6), (5, 7)] {
        let dict = doc.resolve_id(id(page)).as_dict().unwrap();
        assert_eq!(dict.get(b"Type".as_slice()), Some(&Object::name("Page")));
        assert_eq!(
            dict.get(b"Annots".as_slice()),
            Some(&Object::Array(vec![Object::Reference(id(link))]))
        );
    }
    let (mut engine2, mut reopened) = session_with(saved(&session));
    assert_eq!(document(&reopened).resolve_id(id(4)), &font);
    for page in 0..2 {
        let listed = links(&mut engine2, &mut reopened, page);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["uri"], "https://e.com/");
    }
    while dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"] == true {}
    assert_eq!(serialized(&session), start);
}

/// `<< /First T(d-1) /Next T(d-1) >>`, nested `depth` levels: 2^depth - 1
/// items written into one another in a few hundred KB.
fn direct_tree(depth: u32) -> String {
    match depth {
        0 => String::new(),
        _ => {
            let below = direct_tree(depth - 1);
            let link = |key: &str| {
                if below.is_empty() {
                    String::new()
                } else {
                    format!(" /{key} {below}")
                }
            };
            format!(
                "<< /Title (d) /Dest [3 0 R /Fit]{}{} >>",
                link("First"),
                link("Next")
            )
        }
    }
}

#[test]
fn items_written_into_others_are_walked_up_to_a_budget() {
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".into(),
        format!("<< /Type /Outlines /First {} >>", direct_tree(15)),
    ];
    let (mut engine, mut session) = session_with(classic(&objects));
    assert_eq!(
        Outline::read(document(&session)).len(),
        tandem_pdf_engine::app::ops::nav::outline::MAX_DIRECT_ITEMS
    );
    let items = outline(&mut engine, &mut session);
    assert_eq!(
        items.len(),
        tandem_pdf_engine::app::ops::nav::outline::MAX_DIRECT_ITEMS
    );
    let start = serialized(&session);
    op(
        &mut engine,
        &mut session,
        json!({"op": "outline-set", "outlineId": 1000, "title": "renamed"}),
    );
    op(
        &mut engine,
        &mut session,
        json!({"op": "outline-delete", "outlineId": 3}),
    );
    op(
        &mut engine,
        &mut session,
        json!({"op": "outline-add", "title": "end", "page": 0}),
    );
    let items = outline(&mut engine, &mut session);
    assert_eq!(items.last().unwrap()["title"], "end");
    assert_eq!(items.last().unwrap()["depth"], 0);
    while dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"] == true {}
    assert_eq!(serialized(&session), start);
}

#[test]
fn listings_stop_at_a_byte_budget_when_one_string_is_shared() {
    let long = format!("https://e/{}", "a".repeat(1 << 20));
    let count = 300;
    let mut objects = strings(&[
        "<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    ]);
    let annots = vec!["5 0 R"; count].join(" ");
    objects.push(format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [{annots} 8 0 R] >>"
    ));
    objects.push(format!("({long})"));
    objects.push(
        "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A << /S /URI /URI 4 0 R >> >>".into(),
    );
    objects.push(format!(
        "<< /Type /Outlines /First 9 0 R /Last {} 0 R /Count {count} >>",
        8 + count
    ));
    objects.push("<< >>".into());
    objects.push(
        "<< /Type /Annot /Subtype /Link /Rect [20 20 30 30] /A << /S /URI /URI (https://last/) >> >>"
            .into(),
    );
    for i in 0..count {
        let number = 9 + i;
        let mut body = "<< /Title 4 0 R /Parent 6 0 R /Dest [3 0 R /Fit]".to_string();
        if i > 0 {
            body += &format!(" /Prev {} 0 R", number - 1);
        }
        if i + 1 < count {
            body += &format!(" /Next {} 0 R", number + 1);
        }
        objects.push(body + " >>");
    }
    let (mut engine, mut session) = session_with(classic(&objects));
    let clipped = format!(
        "{}\u{2026}",
        &long[..tandem_pdf_engine::app::ops::nav::target::MAX_TEXT_BYTES]
    );
    let budget = tandem_pdf_engine::app::ops::nav::MAX_LISTED_BYTES;
    let reply = op(&mut engine, &mut session, json!({"op": "links", "page": 0}));
    assert_eq!(reply["truncated"], true);
    let items = reply["items"].as_array().unwrap();
    assert_eq!(items.len(), budget / clipped.len());
    for item in items {
        assert_eq!(item["kind"], "blocked", "a clipped URI is never followed");
        assert_eq!(item["uri"].as_str().unwrap(), clipped);
    }
    assert!(reply.to_string().len() < budget + budget / 8);
    let reply = op(&mut engine, &mut session, json!({"op": "outline"}));
    assert_eq!(reply["truncated"], true);
    let items = reply["items"].as_array().unwrap();
    assert_eq!(items.len(), budget / clipped.len());
    assert_eq!(items[0]["title"].as_str().unwrap(), clipped);
    assert!(reply.to_string().len() < budget + budget / 8);
    // The edit ops use the same list.
    let start = serialized(&session);
    op(
        &mut engine,
        &mut session,
        json!({"op": "link-delete", "page": 0, "linkId": 0}),
    );
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), start);
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "link-delete", "page": 0, "linkId": count}),
    );
    assert_eq!(
        reply["error"],
        format!("link-delete: no link {count} on this page")
    );
    // An unshared long URI is clipped and blocked; short replies are whole.
    let (mut engine, mut session) = session_with(nav());
    let reply = op(&mut engine, &mut session, json!({"op": "links", "page": 0}));
    assert!(reply.get("truncated").is_none());
    let reply = op(&mut engine, &mut session, json!({"op": "outline"}));
    assert!(reply.get("truncated").is_none());
}

#[test]
fn outline_add_never_writes_into_an_outlines_entry_that_is_not_an_outline() {
    for root in ["2 0 R", "1 0 R", "4 0 R"] {
        let objects = strings(&[
            &format!("<< /Type /Catalog /Pages 2 0 R /Outlines {root} >>"),
            "<< /Type /Pages /Kids [3 0 R 5 0 R 6 0 R] /Count 3 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /First 3 0 R >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        ]);
        let (mut engine, mut session) = session_with(classic(&objects));
        assert!(outline(&mut engine, &mut session).is_empty(), "{root}");
        let before: Vec<Object> = (1..=6)
            .map(|n| document(&session).resolve_id(id(n)).clone())
            .collect();
        let start = serialized(&session);
        let added = op(
            &mut engine,
            &mut session,
            json!({"op": "outline-add", "title": "A", "page": 2}),
        );
        assert_eq!(added["pageCount"], 3);
        let doc = document(&session);
        for n in 2..=6 {
            assert_eq!(
                doc.resolve_id(id(n)),
                &before[n as usize - 1],
                "{root}: {n}"
            );
        }
        assert_valid_outline(doc);
        let catalog = doc.resolve_id(id(1)).as_dict().unwrap();
        let outlines = catalog.get(b"Outlines".as_slice()).unwrap();
        assert_eq!(
            dict(doc, outlines).get(b"Type".as_slice()),
            Some(&Object::name("Outlines"))
        );
        let (mut engine2, mut reopened) = session_with(saved(&session));
        let last = op(
            &mut engine2,
            &mut reopened,
            json!({"op": "links", "page": 2}),
        );
        assert_eq!(last["page"], 2, "{root}: three pages");
        assert_eq!(
            titles(&mut engine2, &mut reopened),
            [("A".to_string(), 0, json!(2))]
        );
        op(&mut engine, &mut session, json!({"op": "undo"}));
        assert_eq!(serialized(&session), start);
    }
}

#[test]
fn text_strings_read_as_utf8_when_they_are_and_pdfdoc_otherwise() {
    let titles_in = [
        ("<C3A9>", "é"),
        ("<E974E9>", "été"),
        ("<C3A980>", "Ã©•"),
        ("<C3A918>", "Ã©˘"),
        ("<18191A41>", "˘ˇˆA"),
        ("<9F41AD>", "\u{9F}A\u{AD}"),
        ("<FEFF00E9>", "é"),
        ("<EFBBBF41C3A9>", "Aé"),
        ("<F09F9880>", "😀"),
        ("<E2808B>", "\u{200B}"),
        ("(plain)", "plain"),
    ];
    let mut objects = strings(&[
        "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [5 0 R 6 0 R] >>",
    ]);
    let count = titles_in.len();
    objects.push(format!(
        "<< /Type /Outlines /First 7 0 R /Last {} 0 R /Count {count} >>",
        6 + count
    ));
    objects.push(
        "<< /Type /Annot /Subtype /Link /Rect [10 10 50 50] /A << /S /URI /URI (https://ex.org/\\303\\251t\\303\\251) >> >>"
            .into(),
    );
    objects.push(
        "<< /Type /Annot /Subtype /Link /Rect [60 10 90 50] /A << /S /URI /URI (https://ex.org/\\351) >> >>"
            .into(),
    );
    for (i, (title, _)) in titles_in.iter().enumerate() {
        let number = 7 + i;
        let mut body = format!("<< /Title {title} /Parent 4 0 R /Dest [3 0 R /Fit]");
        if i > 0 {
            body += &format!(" /Prev {} 0 R", number - 1);
        }
        if i + 1 < count {
            body += &format!(" /Next {} 0 R", number + 1);
        }
        objects.push(body + " >>");
    }
    let (mut engine, mut session) = session_with(classic(&objects));
    let listed: Vec<String> = titles(&mut engine, &mut session)
        .into_iter()
        .map(|(title, _, _)| title)
        .collect();
    let wanted: Vec<&str> = titles_in.iter().map(|(_, text)| *text).collect();
    assert_eq!(listed, wanted);
    let items = links(&mut engine, &mut session, 0);
    assert_eq!(items[0]["kind"], "uri");
    assert_eq!(items[0]["uri"], "https://ex.org/été");
    assert_eq!(
        items[1]["uri"], "https://ex.org/é",
        "not UTF-8: PDFDocEncoding"
    );
    let reply = op(
        &mut engine,
        &mut session,
        json!({"op": "link-set", "page": 0, "annotId": 5, "target": {"uri": "https://ex.org/été"}}),
    );
    assert_eq!(reply["noop"], true);
    // What the app writes reads back as written.
    op(
        &mut engine,
        &mut session,
        json!({"op": "outline-set", "outlineId": 0, "title": "Ünïcode ˘"}),
    );
    assert_eq!(outline(&mut engine, &mut session)[0]["title"], "Ünïcode ˘");
}

/// A one-page file (the page is object 10) whose outline root is object 3
/// and whose items are objects 4 on, each `<< /Dest [10 0 R /Fit] item >>`.
fn plain_outline(root: &str, items: &[&str]) -> Vec<u8> {
    let mut objects = strings(&[
        "<< /Type /Catalog /Pages 2 0 R /Outlines 3 0 R >>",
        "<< /Type /Pages /Kids [10 0 R] /Count 1 >>",
    ]);
    objects.push(root.to_string());
    for item in items {
        objects.push(format!("<< /Dest [10 0 R /Fit] {item} >>"));
    }
    while objects.len() < 9 {
        objects.push("<< >>".into());
    }
    objects.push("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".into());
    classic(&objects)
}

#[test]
fn deleting_an_item_on_a_cycle_or_shared_by_two_parents_removes_it() {
    let cycle = [
        "/Title (A) /Parent 3 0 R /Next 5 0 R",
        "/Title (B) /Parent 3 0 R /Prev 4 0 R /Next 6 0 R",
        "/Title (C) /Parent 3 0 R /Prev 5 0 R /Next 4 0 R",
    ];
    let pair = [
        "/Title (A) /Parent 3 0 R /Next 5 0 R /Prev 5 0 R",
        "/Title (B) /Parent 3 0 R /Next 4 0 R /Prev 4 0 R",
    ];
    // S (6) is the child of both P (4) and Q (5).
    let shared = [
        "/Title (P) /Parent 3 0 R /Next 5 0 R /First 6 0 R /Last 6 0 R /Count 1",
        "/Title (Q) /Parent 3 0 R /Prev 4 0 R /First 6 0 R /Last 6 0 R /Count 1",
        "/Title (S) /Parent 4 0 R",
    ];
    // S (8) follows X (6) under P (4) and Y (7) under Q (5).
    let shared_next = [
        "/Title (P) /Parent 3 0 R /Next 5 0 R /First 6 0 R /Last 8 0 R /Count 2",
        "/Title (Q) /Parent 3 0 R /Prev 4 0 R /First 7 0 R /Last 8 0 R /Count 2",
        "/Title (X) /Parent 4 0 R /Next 8 0 R",
        "/Title (Y) /Parent 5 0 R /Next 8 0 R",
        "/Title (S) /Parent 4 0 R /Prev 6 0 R",
    ];
    let cases: [(&[&str], usize, &[&str]); 6] = [
        (&cycle, 0, &["B", "C"]),
        (&cycle, 1, &["A", "C"]),
        (&cycle, 2, &["A", "B"]),
        (&pair, 0, &["B"]),
        (&shared, 1, &["P", "Q"]),
        (&shared_next, 2, &["P", "X", "Q", "Y"]),
    ];
    for (items, delete, left) in cases {
        let root = "<< /Type /Outlines /First 4 0 R /Last 5 0 R >>";
        let (mut engine, mut session) = session_with(plain_outline(root, items));
        let start = serialized(&session);
        op(
            &mut engine,
            &mut session,
            json!({"op": "outline-delete", "outlineId": delete}),
        );
        let names: Vec<String> = titles(&mut engine, &mut session)
            .into_iter()
            .map(|(title, _, _)| title)
            .collect();
        assert_eq!(names, left, "{items:?} delete {delete}");
        assert_valid_outline(document(&session));
        // The next edit sees the same outline.
        op(
            &mut engine,
            &mut session,
            json!({"op": "outline-add", "title": "Z", "page": 0}),
        );
        assert_valid_outline(document(&session));
        let names: Vec<String> = titles(&mut engine, &mut session)
            .into_iter()
            .map(|(title, _, _)| title)
            .collect();
        assert_eq!(names.last().map(String::as_str), Some("Z"));
        assert_eq!(names.len(), left.len() + 1);
        while dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"] == true {}
        assert_eq!(serialized(&session), start);
    }
}

#[test]
fn file_specifications_naming_a_url_list_as_that_url() {
    let actions = [
        (
            "/A << /S /GoToR /F << /FS /URL /F (https://ex.org/doc.pdf) >> /D [2 /Fit] >>",
            json!({"kind": "uri", "uri": "https://ex.org/doc.pdf#page=3&view=Fit"}),
        ),
        (
            "/A << /S /GoToR /F << /FS /URL /F (https://ex.org/d.pdf#frag) >> /D [2 /Fit] >>",
            json!({"kind": "uri", "uri": "https://ex.org/d.pdf#frag&page=3&view=Fit"}),
        ),
        (
            "/A << /S /GoToR /F << /FS /URL /F (https://ex.org/d.pdf) >> /D (str) >>",
            json!({"kind": "uri", "uri": "https://ex.org/d.pdf#nameddest=str"}),
        ),
        (
            "/A << /S /GoToR /F << /FS /URL /UF (https://ex.org/a b.pdf) /F (x) >> /D [0 /Fit] >>",
            json!({"kind": "uri", "uri": "https://ex.org/a b.pdf#page=1&view=Fit"}),
        ),
        (
            "/A << /S /GoToR /F << /FS /URL /F (rel.pdf) >> /D [0 /XYZ 1 2 3] >>",
            json!({"kind": "blocked", "uri": "rel.pdf#page=1&zoom=300,1,2"}),
        ),
        // MuPDF lists no link for these two.
        (
            "/A << /S /GoToR /F << /FS /URL /F (https://ex.org/d.pdf) >> >>",
            json!({"kind": "uri", "uri": "https://ex.org/d.pdf"}),
        ),
        (
            "/A << /S /Launch /F << /FS /URL /F (https://ex.org/doc.pdf) >> >>",
            json!({"kind": "uri", "uri": "https://ex.org/doc.pdf"}),
        ),
        // /Unix and /DOS are ignored, as MuPDF ignores them.
        (
            "/A << /S /GoToR /F << /Unix (/tmp/u.pdf) /F (f.pdf) >> /D [0 /Fit] >>",
            json!({"kind": "blocked", "uri": "file:f.pdf#page=1&view=Fit"}),
        ),
        (
            "/A << /S /GoToR /F << /FS /Other /F (o.pdf) >> /D [0 /Fit] >>",
            json!({"kind": "blocked", "uri": "file:o.pdf#page=1&view=Fit"}),
        ),
    ];
    let mut objects = strings(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    ]);
    let annots: Vec<String> = (0..actions.len())
        .map(|i| format!("{} 0 R", 4 + i))
        .collect();
    objects.push(format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [{}] >>",
        annots.join(" ")
    ));
    for (action, _) in &actions {
        objects.push(format!(
            "<< /Type /Annot /Subtype /Link /Rect [10 10 50 50] {action} >>"
        ));
    }
    let (mut engine, mut session) = session_with(classic(&objects));
    let items = links(&mut engine, &mut session, 0);
    assert_eq!(items.len(), actions.len());
    for (item, (action, wanted)) in items.iter().zip(&actions) {
        assert_eq!(item["kind"], wanted["kind"], "{action}");
        assert_eq!(item["uri"], wanted["uri"], "{action}");
    }
}

#[test]
fn link_checks_are_shared_for_clip_paste() {
    use tandem_pdf_engine::app::ops::nav::check_link_rect;
    let size = (612.0, 792.0);
    assert_eq!(
        check_link_rect(size, Some(&json!([50, 60, 10, 20])), "link-add").unwrap(),
        [10.0, 20.0, 50.0, 60.0]
    );
    let error = check_link_rect(size, Some(&json!([10, 10, 10, 50])), "link-add").unwrap_err();
    assert_eq!(error.message, "link-add: rect must have a positive area");
    let error = check_link_rect(size, Some(&json!([0, 0, 613, 50])), "link-add").unwrap_err();
    assert_eq!(
        error.message,
        "link-add: rect [0, 0, 613, 50] is off the page"
    );
    let error = check_link_rect(size, None, "link-add").unwrap_err();
    assert_eq!(error.message, "link-add: rect must be 4 finite numbers");
}

#[test]
fn remove_many_deletes_links_on_several_pages_with_one_reference_pass() {
    use tandem_pdf_engine::app::ops::nav::links;
    let objects = strings(&[
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 11 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [6 0 R 7 0 R 8 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [9 0 R 8 0 R] >>",
        "<< >>",
        "<< /Type /Annot /Subtype /Link /Rect [10 10 50 50] /A << /S /URI /URI (https://six/) >> /StructParent 0 >>",
        "<< /Type /Annot /Subtype /Link /Rect [60 10 90 50] /Dest [4 0 R /Fit] >>",
        "<< /Type /Annot /Subtype /Link /Rect [100 10 140 50] /A << /S /URI /URI (https://eight/) >> >>",
        "<< /Type /Annot /Subtype /Link /Rect [10 60 50 90] /A << /S /URI /URI (https://nine/) >> >>",
        "<< /Type /Annot /Subtype /Text /Rect [0 0 9 9] /IRT 9 0 R >>",
        "<< /Type /StructTreeRoot /K [12 0 R] >>",
        "<< /Type /StructElem /S /Link /P 11 0 R /K [<< /Type /OBJR /Obj 6 0 R /Pg 3 0 R >> 0] >>",
    ]);
    let (mut engine, mut session) = session_with(classic(&objects));
    let start = serialized(&session);
    session
        .mutate("delete links", |session| {
            let mut tx = session.tx()?;
            links::remove_many(&mut tx, &[(0, 2), (0, 0), (1, 0), (0, 99)])?;
            Ok(())
        })
        .unwrap();
    let page0: Vec<Value> = links(&mut engine, &mut session, 0)
        .iter()
        .map(|link| link["annotId"].clone())
        .collect();
    assert_eq!(page0, [json!(7)]);
    let page1: Vec<Value> = links(&mut engine, &mut session, 1)
        .iter()
        .map(|link| link["annotId"].clone())
        .collect();
    assert_eq!(page1, [json!(8)], "8 is still listed on page 2");
    let doc = document(&session);
    let elem = doc.resolve_id(id(12)).as_dict().unwrap();
    assert_eq!(
        elem.get(b"K".as_slice()),
        Some(&Object::Array(vec![Object::Integer(0)]))
    );
    let text = doc.resolve_id(id(10)).as_dict().unwrap();
    assert_eq!(text.get(b"IRT".as_slice()), None);
    let bytes = saved(&session);
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("https://six/") && !text.contains("https://nine/"));
    assert!(text.contains("https://eight/"));
    assert_eq!(
        op(&mut engine, &mut session, json!({"op": "undo"}))["step"],
        "delete links"
    );
    assert_eq!(serialized(&session), start);
}

#[test]
fn link_delete_never_cuts_a_page_listed_as_a_link() {
    // Page 4 also says it is a link and page 3 lists it in /Annots: the
    // entry goes, the page stays in the tree (annot-delete's rule, shared).
    let objects = strings(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Subtype /Link /Rect [10 10 50 50] /A << /S /URI /URI (https://x/) >> >>",
    ]);
    let (mut engine, mut session) = session_with(classic(&objects));
    assert_eq!(links(&mut engine, &mut session, 0)[0]["annotId"], 4);
    op(
        &mut engine,
        &mut session,
        json!({"op": "link-delete", "page": 0, "linkId": 0}),
    );
    assert!(links(&mut engine, &mut session, 0).is_empty());
    let doc = document(&session);
    assert_eq!(
        doc.dict_get(doc.resolve_id(id(2)).as_dict().unwrap(), b"Kids"),
        &Object::Array(vec![Object::Reference(id(3)), Object::Reference(id(4))])
    );
    assert_eq!(session.page_state().count(), 2);
}
