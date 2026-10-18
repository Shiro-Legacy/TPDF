// SPDX-License-Identifier: AGPL-3.0-or-later
//! Regressions from the acceptance review of the navigation and label ops:
//! name and number trees whose /Kids array
//! names itself, /Annots arrays shared by several pages, /PageLabels naming
//! something that is not a number tree, label starts past a 32-bit int, and
//! an incremental update whose first object follows `%%EOF` on its line.
use serde_json::{Value, json};
use tandem_pdf_engine::app::pagetree::{self, NamedDests};
use tandem_pdf_engine::app::{AppEngine, AppSession, labels};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Object, ObjectId};
use tandem_pdf_engine::writer::{DocumentWriteLimits, WriteOptions, write_document};

fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}

/// A classic-xref PDF from object bodies numbered from 1.
fn classic(objects: &[&str]) -> Vec<u8> {
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

fn items(reply: &Value) -> Vec<Value> {
    reply["items"].as_array().unwrap().clone()
}

/// The reviewer's names-bomb.pdf (581 bytes): a link to the named
/// destination (a), and a /Names /Dests tree whose /Kids array 4 0 R holds
/// two direct nodes that both name 4 0 R as their /Kids again.
fn names_bomb() -> Vec<u8> {
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R /Names << /Dests << /Kids 4 0 R >> >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [5 0 R] >>",
        "[ << /Kids 4 0 R /Names [(a) [3 0 R /Fit]] >> << /Kids 4 0 R >> ]",
        "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /Dest (a) >>",
    ])
}

#[test]
fn a_name_tree_whose_kids_array_names_itself_is_walked_once() {
    let bytes = names_bomb();
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    // The root, then the array's two direct nodes; the array is entered once.
    let catalog = doc.resolve_id(doc.root());
    let names = pagetree::stored(&doc, catalog, b"Names");
    let tree = pagetree::stored(&doc, names, b"Dests");
    assert_eq!(pagetree::tree_nodes(&doc, tree).len(), 3);
    assert!(NamedDests::load(&doc).get(&doc, b"a").is_some());

    // links, link-set and link-delete answer (they hung: 2^64 node visits).
    let (mut engine, mut session) = session_with(bytes);
    let links = items(&op(
        &mut engine,
        &mut session,
        json!({"op": "links", "page": 0}),
    ));
    assert_eq!(links.len(), 1);
    assert_eq!(
        (&links[0]["kind"], &links[0]["page"]),
        (&json!("page"), &json!(0))
    );
    let before = serialized(&session);
    op(
        &mut engine,
        &mut session,
        json!({"op": "link-set", "page": 0, "annotId": 5, "rect": [20, 20, 60, 60]}),
    );
    op(
        &mut engine,
        &mut session,
        json!({"op": "link-delete", "page": 0, "linkId": 0}),
    );
    assert!(
        items(&op(
            &mut engine,
            &mut session,
            json!({"op": "links", "page": 0})
        ))
        .is_empty()
    );
    op(&mut engine, &mut session, json!({"op": "undo"}));
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);

    // names-bomb-outline.pdf (651 bytes): a bookmark to (a), same tree.
    let (mut engine, mut session) = session_with(classic(&[
        "<< /Type /Catalog /Pages 2 0 R /Names << /Dests << /Kids 4 0 R >> >> /Outlines 5 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        "[ << /Kids 4 0 R /Names [(a) [3 0 R /Fit]] >> << /Kids 4 0 R >> ]",
        "<< /Type /Outlines /First 6 0 R /Last 6 0 R /Count 1 >>",
        "<< /Title (x) /Parent 5 0 R /Dest (a) >>",
    ]));
    let outline = items(&op(&mut engine, &mut session, json!({"op": "outline"})));
    assert_eq!(outline.len(), 1);
    assert_eq!(
        (&outline[0]["title"], &outline[0]["page"]),
        (&json!("x"), &json!(0))
    );
}

/// Levels 4.. of a tree `depth` deep: level i is an array object of two
/// direct nodes whose /Kids is level i + 1, so the paths double per level;
/// the last level's nodes hold `leaf`.
fn doubling_levels(depth: u32, leaf: &str) -> Vec<String> {
    (0..depth)
        .map(|level| {
            let next = match level + 1 < depth {
                true => format!("/Kids {} 0 R", 5 + level),
                false => leaf.to_string(),
            };
            format!("[ << {next} >> << {next} >> ]")
        })
        .collect()
}

#[test]
fn trees_whose_paths_double_per_level_are_walked_in_linear_time() {
    // 60 levels: 2^60 paths, 121 nodes.
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /PageLabels << /Kids 4 0 R >> /Names << /Dests << /Kids 4 0 R >> >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [<< /Subtype /Link /Rect [0 0 9 9] /Dest (z) >>] >>".to_string(),
    ];
    objects.extend(doubling_levels(
        60,
        "/Nums [0 << /S /A >>] /Names [(z) [3 0 R /Fit]]",
    ));
    let objects: Vec<&str> = objects.iter().map(String::as_str).collect();
    let bytes = classic(&objects);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let catalog = doc.resolve_id(doc.root());
    let labels_tree = pagetree::stored(&doc, catalog, b"PageLabels");
    assert_eq!(pagetree::tree_nodes(&doc, labels_tree).len(), 121);
    // The last level's two nodes, each read once.
    assert_eq!(labels::entries(&doc).unwrap().len(), 2);

    let (mut engine, mut session) = session_with(bytes);
    let reply = op(&mut engine, &mut session, json!({"op": "labels"}));
    assert_eq!(reply["items"], json!(["A"]));
    let links = items(&op(
        &mut engine,
        &mut session,
        json!({"op": "links", "page": 0}),
    ));
    assert_eq!(links[0]["page"], 0);
}

/// The reviewer's labels-bomb.pdf (456 bytes): /PageLabels << /Kids 4 0 R >>
/// where 4 0 R holds two direct nodes naming 4 0 R again.
fn labels_bomb() -> Vec<u8> {
    classic(&[
        "<< /Type /Catalog /Pages 2 0 R /PageLabels << /Kids 4 0 R >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        "[ << /Kids 4 0 R /Nums [0 << /S /r >>] >> << /Kids 4 0 R >> ]",
    ])
}

#[test]
fn a_number_tree_whose_kids_array_names_itself_reads_and_edits() {
    // labels trapped the engine at 1.25 GB; labels-set and labels-clear too.
    let (mut engine, mut session) = session_with(labels_bomb());
    let reply = op(&mut engine, &mut session, json!({"op": "labels"}));
    assert_eq!(reply["items"], json!(["i"]));
    let before = serialized(&session);
    let set = op(
        &mut engine,
        &mut session,
        json!({"op": "labels-set", "from": 0, "style": "D", "prefix": "p-"}),
    );
    assert_eq!(set["items"], json!(["p-1"]));
    let clear = op(
        &mut engine,
        &mut session,
        json!({"op": "labels-clear", "from": 0}),
    );
    assert_eq!(clear["items"], json!(["1"]));
    op(&mut engine, &mut session, json!({"op": "undo"}));
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);
}

#[test]
fn link_add_and_delete_on_a_shared_annots_array_change_only_their_page() {
    let shared = |annots: &str| {
        classic(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots 6 0 R >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots 6 0 R >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots 6 0 R >>",
            annots,
            "<< /Type /Annot /Subtype /Link /Rect [10 10 50 50] /A << /S /URI /URI (https://a.example/) >> >>",
        ])
    };
    let counts = |engine: &mut AppEngine, session: &mut AppSession| -> Vec<usize> {
        (0..3)
            .map(|page| items(&op(engine, session, json!({"op": "links", "page": page}))).len())
            .collect()
    };
    let (mut engine, mut session) = session_with(shared("[]"));
    let before = serialized(&session);
    op(
        &mut engine,
        &mut session,
        json!({"op": "link-add", "page": 0, "rect": [10, 10, 50, 50], "target": {"uri": "https://b.example/"}}),
    );
    assert_eq!(counts(&mut engine, &mut session), [1, 0, 0]);
    // Page 0 has its own array; the others still share the empty one.
    let doc = document(&session);
    let page = doc.resolve_id(id(3)).as_dict().unwrap();
    assert_eq!(
        page.get(b"Annots".as_slice())
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(doc.resolve_id(id(6)).as_array().unwrap().len(), 0);
    let reopened = session_with(saved(&session));
    let (mut engine2, mut session2) = reopened;
    assert_eq!(counts(&mut engine2, &mut session2), [1, 0, 0]);
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);

    // link-delete on page 1 leaves pages 0 and 2 their link.
    let (mut engine, mut session) = session_with(shared("[7 0 R]"));
    assert_eq!(counts(&mut engine, &mut session), [1, 1, 1]);
    let before = serialized(&session);
    op(
        &mut engine,
        &mut session,
        json!({"op": "link-delete", "page": 1, "linkId": 0}),
    );
    assert_eq!(counts(&mut engine, &mut session), [1, 0, 1]);
    // The annotation is still listed elsewhere, so it is kept.
    assert!(document(&session).resolve_id(id(7)).as_dict().is_some());
    op(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(serialized(&session), before);

    // An array only this page names is still edited where it is.
    let (mut engine, mut session) = session_with(classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots 4 0 R >>",
        "[]",
    ]));
    op(
        &mut engine,
        &mut session,
        json!({"op": "link-add", "page": 0, "rect": [10, 10, 50, 50], "target": {"page": 0}}),
    );
    assert_eq!(
        document(&session)
            .resolve_id(id(4))
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn page_labels_naming_the_page_tree_or_a_field_are_never_written_into() {
    // /PageLabels names the /Pages root: no labels, and labels-set writes a
    // new tree into the catalog, leaving the page tree whole.
    let pages_root = |target: &str| {
        classic(&[
            &format!("<< /Type /Catalog /Pages 2 0 R /PageLabels {target} >>"),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
            "<< /FT /Tx /T (f) /Kids [6 0 R] >>",
            "<< /Parent 5 0 R /Subtype /Widget /Rect [0 0 1 1] >>",
        ])
    };
    for target in ["2 0 R", "1 0 R", "3 0 R"] {
        let (mut engine, mut session) = session_with(pages_root(target));
        let reply = op(&mut engine, &mut session, json!({"op": "labels"}));
        assert_eq!(reply["items"], json!(["1", "2"]), "{target}");
        let set = op(
            &mut engine,
            &mut session,
            json!({"op": "labels-set", "from": 1, "style": "r"}),
        );
        assert_eq!(set["items"], json!(["1", "i"]), "{target}");
        assert_eq!(set["pageCount"], 2);
        let doc = document(&session);
        let tree = doc.resolve_id(id(2)).as_dict().unwrap();
        assert!(tree.get(b"Kids".as_slice()).is_some(), "{target}");
        assert!(tree.get(b"Nums".as_slice()).is_none(), "{target}");
        // The session and the saved file agree.
        let (mut engine2, mut session2) = session_with(saved(&session));
        let reopened = op(&mut engine2, &mut session2, json!({"op": "labels"}));
        assert_eq!(reopened["items"], json!(["1", "i"]), "{target}");
    }

    // A field with /Kids is read as a number tree (it has no /Nums) but is
    // never rewritten: the catalog gets a new tree.
    let (mut engine, mut session) = session_with(pages_root("5 0 R"));
    op(
        &mut engine,
        &mut session,
        json!({"op": "labels-set", "from": 0, "style": "A"}),
    );
    let field = document(&session).resolve_id(id(5)).as_dict().unwrap();
    assert!(field.get(b"Kids".as_slice()).is_some());
    assert!(field.get(b"Nums".as_slice()).is_none());
    let (mut engine2, mut session2) = session_with(saved(&session));
    let reopened = op(&mut engine2, &mut session2, json!({"op": "labels"}));
    assert_eq!(reopened["items"], json!(["A", "B"]));

    // A number tree object is still rewritten in place.
    let (mut engine, mut session) = session_with(classic(&[
        "<< /Type /Catalog /Pages 2 0 R /PageLabels 4 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        "<< /Nums [0 << /S /r >>] >>",
    ]));
    op(
        &mut engine,
        &mut session,
        json!({"op": "labels-set", "from": 0, "style": "R"}),
    );
    let doc = document(&session);
    let catalog = doc.resolve_id(doc.root()).as_dict().unwrap();
    assert_eq!(
        catalog.get(b"PageLabels".as_slice()),
        Some(&Object::Reference(id(4)))
    );
}

#[test]
fn label_starts_past_a_32_bit_int_are_refused() {
    let page = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>";
    let bytes = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        page,
        page,
    ]);
    let (mut engine, mut session) = session_with(bytes);
    let set = op(
        &mut engine,
        &mut session,
        json!({"op": "labels-set", "from": 0, "style": "D", "start": 2_147_483_647_i64}),
    );
    assert_eq!(set["items"], json!(["2147483647", "2147483648"]));
    let revision = set["revision"].clone();
    for start in [
        json!(2_147_483_648_i64),
        json!(9_007_199_254_740_992_i64),
        json!(1e20),
    ] {
        let reply = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "labels-set", "from": 0, "style": "D", "start": start}),
        );
        assert_eq!(reply["ok"], false, "{start}");
        assert_eq!(
            reply["error"], "labels-set: start must be at most 2147483647",
            "{start}"
        );
        assert!(reply.get("reason").is_none(), "plain: {reply}");
    }
    let state = op(&mut engine, &mut session, json!({"op": "labels"}));
    assert_eq!(state["items"], json!(["2147483647", "2147483648"]));
    let again = op(
        &mut engine,
        &mut session,
        json!({"op": "labels-set", "from": 1, "style": "r", "start": 3}),
    );
    assert_eq!(again["revision"].as_i64(), revision.as_i64().map(|r| r + 1));
}

/// glued.pdf from the review: a one-page file and an incremental update
/// adding a bookmark, whose first object follows the first `%%EOF` with no
/// line break (or `glue` between them). `startxref` is the update's xref
/// offset, or `final_offset`.
fn glued(glue: &str, final_offset: Option<usize>) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = [0usize; 6];
    let object = |out: &mut Vec<u8>, offsets: &mut [usize; 6], n: usize, body: &str| {
        offsets[n] = out.len();
        out.extend_from_slice(format!("{n} 0 obj\n{body}\nendobj\n").as_bytes());
    };
    for (n, body) in [
        (1, "<< /Type /Catalog /Pages 2 0 R >>"),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>"),
    ] {
        object(&mut out, &mut offsets, n, body);
    }
    let first = out.len();
    out.extend_from_slice(b"xref\n0 4\n0000000000 65535 f\r\n");
    for offset in &offsets[1..=3] {
        out.extend_from_slice(format!("{offset:010} 00000 n\r\n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{first}\n%%EOF{glue}").as_bytes(),
    );
    for (n, body) in [
        (1, "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>"),
        (4, "<< /Type /Outlines /First 5 0 R /Last 5 0 R /Count 1 >>"),
        (5, "<< /Title (Chapter) /Parent 4 0 R /Dest [3 0 R /Fit] >>"),
    ] {
        object(&mut out, &mut offsets, n, body);
    }
    let second = out.len();
    out.extend_from_slice(
        format!(
            "xref\n0 2\n0000000000 65535 f\r\n{:010} 00000 n\r\n4 2\n{:010} 00000 n\r\n{:010} 00000 n\r\n",
            offsets[1], offsets[4], offsets[5]
        )
        .as_bytes(),
    );
    let startxref = final_offset.unwrap_or(second);
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size 6 /Root 1 0 R /Prev {first} >>\nstartxref\n{startxref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    out
}

#[test]
fn an_update_whose_first_object_follows_eof_on_its_line_is_read() {
    // The strict reader takes it, as reference viewers do: two sections, not repaired.
    let bytes = glued("", None);
    let strict = Document::open(&bytes, DocumentLimits::default()).unwrap();
    assert_eq!(strict.trailer_chain().count(), 2);
    let (mut engine, mut session) = session_with(bytes);
    let info = session.info().unwrap();
    let unglued = session_with(glued("\n", None)).1.info().unwrap();
    assert_eq!((info.versions, info.repaired), (unglued.versions, false));
    assert_eq!(info.versions, 1, "one update");
    let outline = items(&op(&mut engine, &mut session, json!({"op": "outline"})));
    assert_eq!(outline.len(), 1);
    assert_eq!(outline[0]["title"], "Chapter");
    // An unedited save keeps the bookmark.
    let (mut engine2, mut session2) = session_with(saved(&session));
    assert_eq!(
        items(&op(&mut engine2, &mut session2, json!({"op": "outline"}))).len(),
        1
    );

    // When the file needs repair for another reason (a wrong final
    // startxref), the repair scan finds the glued object too.
    let (mut engine, mut session) = session_with(glued("", Some(7)));
    assert!(session.info().unwrap().repaired);
    let outline = items(&op(&mut engine, &mut session, json!({"op": "outline"})));
    assert_eq!(outline.len(), 1, "{outline:?}");

    // Data glued to %%EOF that no xref entry names is still refused by the
    // strict reader; the app's open reads the file another way.
    let junk = glued("junk\n", None);
    assert!(Document::open(&junk, DocumentLimits::default()).is_err());
    let (mut engine, mut session) = session_with(junk);
    assert_eq!(
        items(&op(&mut engine, &mut session, json!({"op": "outline"}))).len(),
        1
    );
    // With a line break it is an ordinary update.
    let plain = glued("\n", None);
    assert!(Document::open(&plain, DocumentLimits::default()).is_ok());
}

/// Opens `bytes` and renders page 0: what the app asks on every open.
fn opens_and_renders(bytes: Vec<u8>) -> (AppEngine, AppSession) {
    let (mut engine, mut session) = session_with(bytes);
    for request in [
        json!({"op": "outline"}),
        json!({"op": "labels"}),
        json!({"op": "links", "page": 0}),
        json!({"op": "annots", "page": 0}),
        json!({"op": "fields", "page": 0}),
        json!({"op": "render", "page": 0, "scale": 0.5}),
    ] {
        op(&mut engine, &mut session, request);
    }
    (engine, session)
}

#[test]
fn other_trees_whose_kids_array_names_itself_open() {
    // The same shape under /Names /EmbeddedFiles (counted by `open`'s info)
    // and /AcroForm /Fields (its signatures counted): each hung the open.
    let (_, session) = opens_and_renders(classic(&[
        "<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles << /Kids 4 0 R >> >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        "[ << /Kids 4 0 R /Names [(a) 5 0 R] >> << /Kids 4 0 R /Names [(b) 5 0 R] >> ]",
        "<< /Type /Filespec /F (a.txt) >>",
    ]));
    assert_eq!(session.info().unwrap().attachments, 2);
    let (_, session) = opens_and_renders(classic(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [<< /T (x) /Kids 4 0 R >>] >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        "[ << /T (a) /Kids 4 0 R /FT /Sig /V << >> >> << /T (b) /Kids 4 0 R >> ]",
    ]));
    assert_eq!(session.info().unwrap().signatures, 1);

    // A page tree whose /Kids array lists direct /Pages nodes naming it
    // again: the page list grew without bound (2 GB in a second).
    let (_, session) = opens_and_renders(classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids 4 0 R /Count 1 >>",
        "<< /Type /Page /MediaBox [0 0 612 792] >>",
        "[ << /Type /Pages /Kids 4 0 R >> 3 0 R << /Type /Pages /Kids 4 0 R >> ]",
    ]));
    assert_eq!(session.doc.as_ref().unwrap().page_count(), 1);
}
