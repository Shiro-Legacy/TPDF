// SPDX-License-Identifier: MIT
//! What a journaled write costs: an edit inside a large object copies only
//! the dictionaries on its path, journal states share the rest with the
//! live document, and undo and redo move states instead of copying them,
//! all while staying byte-exact.
use std::time::Instant;

use serde_json::{Value, json};
use tandem_pdf_engine::app::{AppEngine, AppSession};
use tandem_pdf_engine::document::Document;
use tandem_pdf_engine::object::{Dictionary, Object};
use tandem_pdf_engine::writer::{DocumentWriteLimits, WriteOptions, write_document};

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

fn session_with(bytes: Vec<u8>) -> (AppEngine, AppSession) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = engine.dispatch(&mut session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
    (engine, session)
}

fn op(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    let reply = engine.dispatch(session, request.clone(), Vec::new()).0;
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

/// An outline item written directly into its parent, with `levels` more
/// levels of children below it (first child and next sibling nested), so
/// `2^(levels + 1) - 1` items in all.
fn inline_item(levels: u32, counter: &mut u32) -> String {
    *counter += 1;
    let title = format!("({:0>48} item {counter})", "");
    let below = match levels {
        0 => String::new(),
        _ => format!(
            " /First {} /Next {}",
            inline_item(levels - 1, counter),
            inline_item(levels - 1, counter)
        ),
    };
    format!("<< /Title {title} /Dest [3 0 R /Fit]{below} >>")
}

/// A catalog holding a 1,023-item outline and 20,000 named destinations
/// written inline (about 0.8 MB in one object).
fn inline_catalog() -> Vec<u8> {
    let mut counter = 0;
    let tree = inline_item(9, &mut counter);
    let dests: String = (0..20_000)
        .map(|n| format!("/d{n:05} [3 0 R /XYZ 0 {} 0] ", n % 792))
        .collect();
    classic(&[
        format!(
            "<< /Type /Catalog /Pages 2 0 R /Outlines << /Type /Outlines /First {tree} >> /Dests << {dests}>> >>"
        ),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".into(),
    ])
}

/// The nav reviewer's shape: `depth` levels of two indirect items each,
/// both naming the next level's pair as their children (a DAG the walk
/// lists once and the edits cut back to a tree).
fn deep_outline(depth: u32) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".into(),
        "<< /Type /Outlines /First 5 0 R /Last 6 0 R /Count 2 >>".into(),
    ];
    for level in 0..depth {
        let (a, b) = (5 + 2 * level, 6 + 2 * level);
        let parent = if level == 0 { 4 } else { a - 2 };
        let children = match level + 1 < depth {
            true => format!(" /First {} 0 R /Last {} 0 R /Count 2", a + 2, b + 2),
            false => String::new(),
        };
        objects.push(format!(
            "<< /Title (a{level}) /Parent {parent} 0 R /Next {b} 0 R /Dest [3 0 R /Fit]{children} >>"
        ));
        objects.push(format!(
            "<< /Title (b{level}) /Parent {parent} 0 R /Prev {a} 0 R /Dest [3 0 R /Fit]{children} >>"
        ));
    }
    classic(&objects)
}

fn catalog(session: &AppSession) -> Dictionary {
    let doc = document(session);
    doc.resolve_id(doc.root()).as_dict().unwrap().clone()
}

fn dests(session: &AppSession) -> Dictionary {
    match catalog(session).get(b"Dests".as_slice()) {
        Some(Object::Dictionary(dests)) => dests.clone(),
        other => panic!("/Dests {other:?}"),
    }
}

#[test]
fn edits_inside_a_large_object_share_what_they_do_not_reach() {
    let (mut engine, mut session) = session_with(inline_catalog());
    let start = serialized(&session);
    let untouched = dests(&session);
    let mut states = vec![start.clone()];
    for (step, id) in [0, 1, 500, 1022].into_iter().enumerate() {
        op(
            &mut engine,
            &mut session,
            json!({"op": "outline-set", "outlineId": id, "title": format!("renamed {step}")}),
        );
        assert!(
            dests(&session).shares_entries(&untouched),
            "an outline edit leaves the inline /Dests shared"
        );
        states.push(serialized(&session));
    }
    op(
        &mut engine,
        &mut session,
        json!({"op": "outline-add", "title": "added", "page": 0}),
    );
    op(
        &mut engine,
        &mut session,
        json!({"op": "outline-delete", "outlineId": 3}),
    );
    assert!(dests(&session).shares_entries(&untouched));
    let end = serialized(&session);
    for _ in 0..2 {
        op(&mut engine, &mut session, json!({"op": "undo"}));
    }
    for expected in states.iter().rev() {
        assert_eq!(&serialized(&session), expected);
        assert!(dests(&session).shares_entries(&untouched));
        if session.doc.as_ref().unwrap().can_undo() {
            op(&mut engine, &mut session, json!({"op": "undo"}));
        }
    }
    assert_eq!(serialized(&session), start);
    while session.doc.as_ref().unwrap().can_redo() {
        op(&mut engine, &mut session, json!({"op": "redo"}));
    }
    assert_eq!(serialized(&session), end);
    assert!(dests(&session).shares_entries(&untouched));

    // An abandoned operation puts back the very state it found.
    let refused = engine
        .dispatch(
            &mut session,
            json!({"op": "outline-set", "outlineId": 99_999, "title": "x"}),
            Vec::new(),
        )
        .0;
    assert_eq!(refused["ok"], false);
    assert_eq!(serialized(&session), end);
}

#[test]
fn a_cleared_or_written_clone_never_changes_the_original() {
    let mut original = Dictionary::new();
    for n in 0..150u8 {
        original.insert(vec![b'k', n], Object::Integer(i64::from(n)));
    }
    let mut nested = Dictionary::new();
    nested.insert(b"Inner".to_vec(), Object::Dictionary(original.clone()));
    let snapshot = nested.clone();
    assert!(snapshot.shares_entries(&nested));

    let inner = nested.get_mut(b"Inner".as_slice()).unwrap();
    inner
        .as_dict_mut()
        .unwrap()
        .insert(b"New".to_vec(), Object::Null);
    assert!(!snapshot.shares_entries(&nested));
    assert_eq!(
        snapshot.get(b"Inner".as_slice()),
        Some(&Object::Dictionary(original.clone()))
    );
    assert_eq!(
        nested
            .get(b"Inner".as_slice())
            .unwrap()
            .as_dict()
            .unwrap()
            .len(),
        151
    );

    let mut cleared = original.clone();
    cleared.clear();
    assert!(cleared.is_empty());
    assert_eq!(original.len(), 150);
    let mut kept = original.clone();
    kept.retain(|key, _| key[1] < 10);
    assert_eq!((kept.len(), original.len()), (10, 150));
    for value in original.clone().values_mut() {
        *value = Object::Null;
    }
    assert_eq!(original.get(b"k\x05".as_slice()), Some(&Object::Integer(5)));
    let absent = original.clone();
    let mut probe = absent.clone();
    assert_eq!(probe.get_mut(b"missing".as_slice()), None);
    assert_eq!(probe.remove(b"missing".as_slice()), None);
    assert!(
        probe.shares_entries(&absent),
        "an absent key copies nothing"
    );
    assert!(Dictionary::new() == Dictionary::with_capacity(4));
}

/// Wall-clock cost of the cases above and of large.pdf edits (release
/// build): `cargo test --release --test app_journal -- --ignored --nocapture`.
#[test]
#[ignore]
fn journal_costs() {
    let time = |label: &str, run: &mut dyn FnMut()| {
        let start = Instant::now();
        run();
        println!("{label}: {:.1} ms", start.elapsed().as_secs_f64() * 1e3);
    };

    let (mut engine, mut session) = session_with(inline_catalog());
    time("inline catalog: 20 outline listings", &mut || {
        for _ in 0..20 {
            op(&mut engine, &mut session, json!({"op": "outline"}));
        }
    });
    time("inline catalog: 20 outline-set", &mut || {
        for n in 0..20 {
            let request =
                json!({"op": "outline-set", "outlineId": n * 50, "title": format!("r{n}")});
            op(&mut engine, &mut session, request);
        }
    });
    time(
        "inline catalog: 5 outline-add, 5 outline-delete",
        &mut || {
            for n in 0..5 {
                op(
                    &mut engine,
                    &mut session,
                    json!({"op": "outline-add", "title": format!("n{n}"), "page": 0}),
                );
                op(
                    &mut engine,
                    &mut session,
                    json!({"op": "outline-delete", "outlineId": 7 * n + 1}),
                );
            }
        },
    );
    time("inline catalog: 30 undo, 30 redo", &mut || {
        for _ in 0..30 {
            op(&mut engine, &mut session, json!({"op": "undo"}));
        }
        for _ in 0..30 {
            op(&mut engine, &mut session, json!({"op": "redo"}));
        }
    });

    let (mut engine, mut session) = session_with(deep_outline(19));
    time("deep outline (19 levels): 10 outline-set", &mut || {
        for n in 0..10 {
            op(
                &mut engine,
                &mut session,
                json!({"op": "outline-set", "outlineId": n, "title": format!("r{n}")}),
            );
        }
    });
    time("deep outline: outline-add, outline-delete", &mut || {
        op(
            &mut engine,
            &mut session,
            json!({"op": "outline-add", "title": "added", "page": 0}),
        );
        op(
            &mut engine,
            &mut session,
            json!({"op": "outline-delete", "outlineId": 2}),
        );
    });

    let large = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/large.pdf"
    ))
    .unwrap();
    let (mut engine, mut session) = session_with(large);
    time("large.pdf: 20 page-rotate", &mut || {
        for page in 0..20 {
            op(
                &mut engine,
                &mut session,
                json!({"op": "page-rotate", "page": page * 25, "by": 90}),
            );
        }
    });
    time("large.pdf: header stamp on 500 pages", &mut || {
        op(
            &mut engine,
            &mut session,
            json!({"op": "stamp", "kind": "header", "text": "Page {page}"}),
        );
    });
    time("large.pdf: 10 annot-add", &mut || {
        for n in 0..10 {
            op(
                &mut engine,
                &mut session,
                json!({"op": "annot-add", "page": 250, "type": "Text",
                "point": [20 + n * 30, 100], "contents": "note"}),
            );
        }
    });
    time("large.pdf: 31 undo, 31 redo", &mut || {
        for _ in 0..31 {
            op(&mut engine, &mut session, json!({"op": "undo"}));
        }
        for _ in 0..31 {
            op(&mut engine, &mut session, json!({"op": "redo"}));
        }
    });
}

#[test]
fn a_reference_cut_copies_only_its_path() {
    use std::collections::BTreeSet;
    use tandem_pdf_engine::app::edit::{Builder, Edit};
    use tandem_pdf_engine::app::pagetree::cut_references;
    use tandem_pdf_engine::document::DocumentLimits;
    use tandem_pdf_engine::object::ObjectId;

    let id = |n| ObjectId::new(n, 0).unwrap();
    let dests: String = (0..2000)
        .map(|n| format!("/d{n:04} [3 0 R /Fit] "))
        .collect();
    let bytes = classic(&[
        format!(
            "<< /Type /Catalog /Pages 2 0 R /Dests << {dests}>> /StructTreeRoot << /K [<< /S /Annot /K [<< /Type /OBJR /Obj 5 0 R >> 0] >>] >> >>"
        ),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".into(),
        "<< >>".into(),
        "<< /Type /Annot /Subtype /Text /Rect [0 0 9 9] >>".into(),
    ]);
    let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    let mut builder = Builder::from_document(doc);
    let catalog = |builder: &Builder| {
        let doc = builder.document();
        doc.resolve_id(doc.root()).as_dict().unwrap().clone()
    };
    let before = catalog(&builder);
    let dropped = BTreeSet::from([id(5)]);
    cut_references(&mut builder, &BTreeSet::new(), &dropped, &dropped).unwrap();
    let after = catalog(&builder);
    let tree = after.get(b"StructTreeRoot".as_slice()).unwrap();
    let expected = "<< /K [<< /S /Annot /K [0] >>] >>";
    let parsed = tandem_pdf_engine::parser::parse_one(
        expected.as_bytes(),
        tandem_pdf_engine::parser::ParseLimits::default(),
    )
    .unwrap();
    assert_eq!(tree, &parsed.object);
    let shared = |dict: &Dictionary| match dict.get(b"Dests".as_slice()) {
        Some(Object::Dictionary(dests)) => dests.clone(),
        other => panic!("{other:?}"),
    };
    assert!(shared(&after).shares_entries(&shared(&before)));

    // A value nested deeper than the parser allows is still cut, in place.
    let mut deep = Object::Array(vec![Object::Reference(id(5)), Object::Integer(1)]);
    for _ in 0..400 {
        deep = Object::dictionary([(b"A".to_vec(), deep)]);
    }
    builder.put_value(id(4), deep).unwrap();
    cut_references(&mut builder, &BTreeSet::new(), &dropped, &dropped).unwrap();
    let mut value = builder.document().resolve_id(id(4));
    for _ in 0..400 {
        value = value.as_dict().unwrap().get(b"A".as_slice()).unwrap();
    }
    assert_eq!(value, &Object::Array(vec![Object::Integer(1)]));
}
