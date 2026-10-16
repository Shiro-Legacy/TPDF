// SPDX-License-Identifier: AGPL-3.0-or-later
//! The journaled app document and the core lifecycle ops: transactions,
//! undo/redo/abandon, the batch modes, render bytes read by hayro, page
//! geometry, document facts, and every refusal and reply of open,
//! authenticate, close, undo, redo, saved, security-set, save, validate and
//! page-ids.
use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};
use tandem_pdf_engine::app::info::{self, DocumentInfo, Permissions};
use tandem_pdf_engine::app::{
    AppDocument, AppEngine, AppError, AppSession, Auth, Batch, CompareDocument, MAX_PDF_BYTES,
    MAX_SAVE_BYTES, Need, OP_PERMISSION, SecuritySetting, op_permission,
};
use tandem_pdf_engine::document::Document;
use tandem_pdf_engine::object::{Dictionary, Object, ObjectId};
use tandem_pdf_engine::writer::{DocumentWriteLimits, WriteOptions, write_document};

fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("../fixtures/{name}")).unwrap()
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

/// Object bodies of an `n`-page document: catalog 1, pages 2, font 3, then
/// page `4 + 2i` with content `5 + 2i` saying "Page i".
fn page_objects(n: usize, page_extra: &str) -> Vec<Vec<u8>> {
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")).into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    for i in 0..n {
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] {page_extra} \
                 /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
                5 + 2 * i
            )
            .into_bytes(),
        );
        objects.push(stream(
            "",
            format!("BT /F1 24 Tf 72 700 Td (Page {i}) Tj ET").as_bytes(),
        ));
    }
    objects
}

fn pages_pdf(n: usize) -> Vec<u8> {
    classic(&page_objects(n, ""), "")
}

/// The objects of `pages_pdf(n)` with catalog, page tree and font in an
/// object stream, ending with a Flate xref stream.
fn xref_stream_pdf(n: usize) -> Vec<u8> {
    let objects = page_objects(n, "");
    let packed = [1usize, 2, 3];
    let container = objects.len() + 1;
    let xref_id = objects.len() + 2;
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut rows = vec![(0u8, 0u64, 65535u16); xref_id + 1];
    for (index, body) in objects.iter().enumerate() {
        let number = index + 1;
        if packed.contains(&number) {
            continue;
        }
        rows[number] = (1, bytes.len() as u64, 0);
        bytes.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let mut header = String::new();
    let mut members = Vec::new();
    for (slot, &number) in packed.iter().enumerate() {
        header.push_str(&format!("{number} {} ", members.len()));
        members.extend_from_slice(&objects[number - 1]);
        members.push(b' ');
        rows[number] = (2, container as u64, slot as u16);
    }
    let mut data = header.into_bytes();
    let first = data.len();
    data.extend_from_slice(&members);
    rows[container] = (1, bytes.len() as u64, 0);
    bytes.extend_from_slice(format!("{container} 0 obj\n").as_bytes());
    bytes.extend_from_slice(&stream(
        &format!("/Type /ObjStm /N {} /First {first}", packed.len()),
        &data,
    ));
    bytes.extend_from_slice(b"\nendobj\n");
    let xref = bytes.len();
    rows[xref_id] = (1, xref as u64, 0);
    let mut table = Vec::new();
    for (kind, field, generation) in rows {
        table.push(kind);
        table.extend_from_slice(&(field as u32).to_be_bytes());
        table.extend_from_slice(&generation.to_be_bytes());
    }
    let packed_table = miniz_oxide::deflate::compress_to_vec_zlib(&table, 6);
    bytes.extend_from_slice(format!("{xref_id} 0 obj\n").as_bytes());
    bytes.extend_from_slice(&stream(
        &format!(
            "/Type /XRef /Size {} /W [1 4 2] /Root 1 0 R /Filter /FlateDecode",
            xref_id + 1
        ),
        &packed_table,
    ));
    bytes.extend_from_slice(format!("\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    bytes
}

/// `pages_pdf(n)` with an incremental update that rewrites page 0's text.
fn incremental_pdf(n: usize) -> Vec<u8> {
    let mut bytes = pages_pdf(n);
    let prev = String::from_utf8_lossy(&bytes)
        .rsplit("startxref\n")
        .next()
        .and_then(|tail| tail.lines().next())
        .and_then(|line| line.trim().parse::<usize>().ok())
        .unwrap();
    let at = bytes.len();
    bytes.extend_from_slice(b"5 0 obj\n");
    bytes.extend_from_slice(&stream("", b"BT /F1 24 Tf 72 700 Td (Updated) Tj ET"));
    bytes.extend_from_slice(b"\nendobj\n");
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!(
            "xref\n5 1\n{at:010} 00000 n \ntrailer\n<< /Size {} /Root 1 0 R /Prev {prev} >>\n\
             startxref\n{xref}\n%%EOF\n",
            4 + 2 * n
        )
        .as_bytes(),
    );
    bytes
}

fn dispatch_buffers(
    engine: &mut AppEngine,
    session: &mut AppSession,
    request: Value,
    buffers: Vec<Vec<u8>>,
) -> (Value, Vec<Vec<u8>>) {
    engine.dispatch(session, request, buffers)
}

fn dispatch(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    let (reply, outputs) = engine.dispatch(session, request, Vec::new());
    assert!(outputs.is_empty());
    reply
}

fn open(engine: &mut AppEngine, session: &mut AppSession, bytes: Vec<u8>) -> Value {
    let request = json!({"id": 1, "op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}});
    let (reply, _) = dispatch_buffers(engine, session, request, vec![bytes]);
    assert_eq!(reply["ok"], true, "{reply}");
    reply
}

fn session_with(bytes: Vec<u8>) -> (AppEngine, AppSession) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    open(&mut engine, &mut session, bytes);
    (engine, session)
}

/// A deterministic full rewrite of the current state, every object kept.
fn serialized(session: &AppSession) -> Vec<u8> {
    let doc = session.doc.as_ref().unwrap().document();
    let options = WriteOptions {
        garbage: false,
        ..WriteOptions::APP
    };
    write_document(doc, DocumentWriteLimits::default(), options).unwrap()
}

fn position(session: &AppSession) -> i64 {
    session.doc.as_ref().unwrap().position()
}

fn page_text(session: &mut AppSession, page: usize) -> String {
    let view = session.view().unwrap();
    view.structured_text(page).unwrap().as_text()
}

fn content_of(session: &AppSession, page: usize) -> ObjectId {
    let doc = session.doc.as_ref().unwrap();
    let (_, dict) = doc.page(page).unwrap();
    dict.get(b"Contents".as_slice())
        .and_then(Object::as_reference)
        .unwrap()
}

fn replace_text(session: &mut AppSession, page: usize, text: &str) -> Result<(), AppError> {
    let content = content_of(session, page);
    let data = format!("BT /F1 24 Tf 72 700 Td ({text}) Tj ET").into_bytes();
    Ok(session.tx()?.replace_decoded_stream(content, data)?)
}

// ---------------------------------------------------------------- journal

#[test]
fn one_operation_is_one_step_and_undo_redo_restore_exact_bytes() {
    let (_, mut session) = session_with(pages_pdf(2));
    let original = serialized(&session);
    let state = session
        .mutate("edit things", |s| {
            replace_text(s, 0, "Changed")?;
            let mut tx = s.tx()?;
            let new = tx
                .allocate(
                    Object::dictionary([(b"Kind".to_vec(), Object::name("New"))]),
                    None,
                )
                .unwrap();
            tx.update(id(2), |pages| {
                pages
                    .as_dict_mut()
                    .unwrap()
                    .insert(b"Extra".to_vec(), Object::Reference(new));
            })
            .unwrap();
            tx.set_trailer_entry(b"Info", Some(Object::Reference(new)))
                .unwrap();
            Ok(new)
        })
        .unwrap();
    assert_eq!(state, Some(id(8)));
    let edited = serialized(&session);
    assert_ne!(edited, original);
    let doc = session.doc.as_ref().unwrap();
    assert_eq!(
        (doc.position(), doc.steps()),
        (1, vec!["edit things".to_string()])
    );
    assert_eq!(session.revision, 1);
    assert!(session.journal_state().dirty && session.journal_state().can_undo);
    assert_eq!(page_text(&mut session, 0).trim(), "Changed");

    assert_eq!(
        session.doc.as_mut().unwrap().undo().unwrap().as_deref(),
        Some("edit things")
    );
    assert_eq!(serialized(&session), original);
    assert!(!session.journal_state().dirty, "back at the saved position");
    assert_eq!(page_text(&mut session, 0).trim(), "Page 0");
    assert_eq!(
        session.doc.as_mut().unwrap().redo().unwrap().as_deref(),
        Some("edit things")
    );
    assert_eq!(serialized(&session), edited);
    assert_eq!(page_text(&mut session, 0).trim(), "Changed");
    assert_eq!(session.doc.as_mut().unwrap().redo().unwrap(), None);
}

#[test]
fn every_transaction_write_is_undone_exactly() {
    let (_, mut session) = session_with(pages_pdf(3));
    let original = serialized(&session);
    session
        .mutate("all writes", |s| {
            let mut tx = s.tx()?;
            tx.put_value(id(3), Object::name("NotAFont")).unwrap();
            tx.replace_value(
                id(1),
                Object::dictionary([
                    (b"Type".to_vec(), Object::name("Catalog")),
                    (b"Pages".to_vec(), Object::Reference(id(2))),
                    (b"Lang".to_vec(), Object::text("en")),
                ]),
            )
            .unwrap();
            let mut dict = Dictionary::new();
            dict.insert(b"Filter".to_vec(), Object::name("FlateDecode"));
            let raw = miniz_oxide::deflate::compress_to_vec_zlib(b"q Q", 6);
            tx.put_stream_raw(id(5), dict.clone(), raw.clone()).unwrap();
            tx.put_stream_decoded(id(7), dict.clone(), b"0 0 m".to_vec())
                .unwrap();
            tx.set_stream_dict(id(9), dict.clone()).unwrap();
            tx.update(id(9), |value| {
                value
                    .as_dict_mut()
                    .unwrap()
                    .insert(b"Mark".to_vec(), Object::Boolean(true));
            })
            .unwrap();
            let new = tx.allocate_stream_raw(dict, raw).unwrap();
            tx.allocate(
                Object::dictionary([(b"N", Object::Integer(1))]),
                Some(b"bytes".to_vec()),
            )
            .unwrap();
            tx.remove_object(new).unwrap();
            tx.set_trailer_entry(
                b"ID",
                Some(Object::array([
                    Object::string(b"a".to_vec()),
                    Object::string(b"b".to_vec()),
                ])),
            )
            .unwrap();
            let doomed: BTreeSet<_> = [id(4)].into();
            assert!(
                tx.delete_objects(&doomed).is_err(),
                "page 0 is still referenced"
            );
            Ok(())
        })
        .unwrap();
    let edited = serialized(&session);
    assert_ne!(edited, original);
    session.doc.as_mut().unwrap().undo().unwrap();
    assert_eq!(serialized(&session), original);
    session.doc.as_mut().unwrap().redo().unwrap();
    assert_eq!(serialized(&session), edited);
}

#[test]
fn a_failed_write_alone_is_not_a_step() {
    let (_, mut session) = session_with(pages_pdf(1));
    session
        .mutate("refused", |s| {
            let mut tx = s.tx()?;
            assert!(tx.remove_object(id(1)).is_err(), "the catalog stays");
            assert!(tx.replace_value(id(5), Object::Null).is_err(), "a stream");
            assert!(tx.set_trailer_entry(b"Size", None).is_err());
            assert!(tx.update(id(99), |_| ()).is_err());
            Ok(())
        })
        .unwrap();
    let doc = session.doc.as_ref().unwrap();
    assert_eq!((doc.position(), doc.can_undo()), (0, false));
    assert_eq!(
        session.revision, 1,
        "mutate succeeded, so the revision moves"
    );
}

#[test]
fn abandon_restores_and_drops_the_redo_branch_only_after_a_write() {
    let (_, mut session) = session_with(pages_pdf(1));
    let original = serialized(&session);
    session
        .mutate("first", |s| replace_text(s, 0, "One"))
        .unwrap();
    let after_first = serialized(&session);
    session.doc.as_mut().unwrap().undo().unwrap();

    // No write: the redo branch survives.
    let error = session
        .mutate("refused", |_| {
            Err::<(), _>(AppError::reason("invalid", "no"))
        })
        .unwrap_err();
    assert_eq!(error.reason.as_deref(), Some("invalid"));
    assert!(session.journal_state().can_redo);
    assert_eq!(serialized(&session), original);
    assert_eq!(session.revision, 1, "only the successful mutate counted");

    // A write: restored exactly, and the redo branch is gone.
    let error = session
        .mutate("written", |s| {
            replace_text(s, 0, "Two")?;
            Err::<(), _>(AppError::plain("late failure"))
        })
        .unwrap_err();
    assert_eq!(error.message, "late failure");
    assert_eq!(serialized(&session), original);
    let state = session.journal_state();
    assert!(!state.can_redo && !state.can_undo);
    assert_eq!(page_text(&mut session, 0).trim(), "Page 0");
    assert_ne!(after_first, original);
}

#[test]
fn an_operation_that_writes_nothing_records_no_step_but_bumps_the_revision() {
    let (_, mut session) = session_with(pages_pdf(1));
    session
        .mutate("first", |s| replace_text(s, 0, "One"))
        .unwrap();
    session.doc.as_mut().unwrap().undo().unwrap();
    assert_eq!(session.mutate("noop", |_| Ok(7)).unwrap(), Some(7));
    let doc = session.doc.as_ref().unwrap();
    assert_eq!(doc.position(), 0);
    assert!(doc.can_redo(), "an empty operation keeps the redo branch");
    assert_eq!(doc.steps(), ["first"]);
    assert_eq!(session.revision, 2);
}

#[test]
fn a_cut_redo_branch_holding_the_saved_state_makes_it_unreachable() {
    let (mut engine, mut session) = session_with(pages_pdf(1));
    session.mutate("a", |s| replace_text(s, 0, "A")).unwrap();
    session.mutate("b", |s| replace_text(s, 0, "B")).unwrap();
    let saved = dispatch(&mut engine, &mut session, json!({"op": "saved"}));
    assert_eq!(
        (session.saved_position, &saved["dirty"]),
        (2, &json!(false))
    );
    dispatch(&mut engine, &mut session, json!({"op": "undo"}));
    dispatch(&mut engine, &mut session, json!({"op": "undo"}));
    assert!(session.journal_state().dirty);
    // Two new steps reach position 2 again, but not the saved bytes.
    session.mutate("c", |s| replace_text(s, 0, "C")).unwrap();
    assert_eq!(session.saved_position, -1);
    session.mutate("d", |s| replace_text(s, 0, "D")).unwrap();
    assert_eq!(position(&session), 2);
    assert!(session.journal_state().dirty);

    // A cut that leaves the saved state on the kept side changes nothing.
    let (mut engine, mut session) = session_with(pages_pdf(1));
    session.mutate("a", |s| replace_text(s, 0, "A")).unwrap();
    dispatch(&mut engine, &mut session, json!({"op": "saved"}));
    session.mutate("b", |s| replace_text(s, 0, "B")).unwrap();
    dispatch(&mut engine, &mut session, json!({"op": "undo"}));
    assert!(!session.journal_state().dirty);
    session.mutate("c", |s| replace_text(s, 0, "C")).unwrap();
    assert_eq!(session.saved_position, 1);
    dispatch(&mut engine, &mut session, json!({"op": "undo"}));
    assert!(
        !session.journal_state().dirty,
        "undo back to the saved state is clean"
    );

    // So does an abandon that wrote: the rule applies on that path too.
    let (mut engine, mut session) = session_with(pages_pdf(1));
    session.mutate("a", |s| replace_text(s, 0, "A")).unwrap();
    dispatch(&mut engine, &mut session, json!({"op": "saved"}));
    dispatch(&mut engine, &mut session, json!({"op": "undo"}));
    let _ = session.mutate("x", |s| {
        replace_text(s, 0, "X")?;
        Err::<(), _>(AppError::plain("no"))
    });
    assert_eq!(session.saved_position, -1);
}

#[test]
fn batch_run_joins_the_open_operation_and_plan_skips_the_work() {
    let (_, mut session) = session_with(pages_pdf(2));
    let mut ran = false;
    session.batch = Batch::Plan;
    assert_eq!(
        session
            .mutate("planned", |_| {
                ran = true;
                Ok(1)
            })
            .unwrap(),
        None
    );
    assert!(!ran);
    assert_eq!(session.revision, 0);

    session.batch = Batch::Off;
    session
        .mutate("edit batch", |s| {
            replace_text(s, 0, "Zero")?;
            s.batch = Batch::Run;
            let inner = s.mutate("inner", |s| {
                replace_text(s, 1, "One")?;
                Ok(2)
            });
            assert_eq!(inner.unwrap(), Some(2));
            s.batch = Batch::Off;
            // Without the batch flag a nested call joins the operation too.
            s.mutate("nested", |s| replace_text(s, 0, "Nested"))?;
            Ok(())
        })
        .unwrap();
    let doc = session.doc.as_ref().unwrap();
    assert_eq!(
        (doc.position(), doc.steps()),
        (1, vec!["edit batch".to_string()])
    );
    assert_eq!(session.revision, 1, "the batch is one revision");
    assert_eq!(page_text(&mut session, 1).trim(), "One");
    session.doc.as_mut().unwrap().undo().unwrap();
    assert_eq!(page_text(&mut session, 0).trim(), "Page 0");
    assert_eq!(page_text(&mut session, 1).trim(), "Page 1");

    // Batch Run outside an operation runs the work, which cannot write.
    session.batch = Batch::Run;
    let refused = session.mutate("loose", |s| s.tx().map(|_| ()));
    assert_eq!(refused.unwrap_err().message, "no journal operation is open");
}

#[test]
fn writes_need_an_open_operation_and_an_unlocked_document() {
    let (_, mut session) = session_with(pages_pdf(1));
    assert!(session.tx().is_err());
    assert!(session.doc.as_mut().unwrap().transaction().is_none());

    let mut none = AppSession::default();
    let error = none.mutate("set page labels", |_| Ok(())).unwrap_err();
    assert_eq!(
        (error.reason.as_deref(), error.message.as_str()),
        (
            Some("invalid"),
            "set page labels: the open document is not a PDF"
        )
    );

    let (_, mut locked) = session_with(fixture("encrypted.pdf"));
    assert!(locked.locked);
    let error = locked.mutate("x", |s| s.tx().map(|_| ())).unwrap_err();
    assert_eq!(error.reason.as_deref(), Some("read-only"));
}

#[test]
fn a_view_inside_an_operation_sees_its_writes_and_never_outlives_an_abandon() {
    let (_, mut session) = session_with(pages_pdf(1));
    assert_eq!(page_text(&mut session, 0).trim(), "Page 0");
    let _ = session.mutate("measure", |s| {
        replace_text(s, 0, "Pending")?;
        assert_eq!(page_text(s, 0).trim(), "Pending");
        replace_text(s, 0, "Again")?;
        assert_eq!(page_text(s, 0).trim(), "Again");
        Err::<(), _>(AppError::plain("abandon"))
    });
    assert_eq!(page_text(&mut session, 0).trim(), "Page 0");
}

/// A small deterministic generator for the random sequence test.
struct Rng(u64);
impl Rng {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % bound
    }
}

#[test]
fn random_mutate_undo_redo_abandon_sequences_match_a_model() {
    for seed in [1u64, 7, 42, 1234, 99991] {
        let (mut engine, mut session) = session_with(pages_pdf(3));
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        // model[p] is the serialized document at journal position p.
        let mut model = vec![serialized(&session)];
        let mut at = 0usize;
        for round in 0..120 {
            match rng.next(6) {
                0 | 1 => {
                    let writes = 1 + rng.next(3);
                    let picks: Vec<u64> = (0..writes).map(|_| rng.next(1000)).collect();
                    session
                        .mutate(&format!("step {round}"), |s| {
                            for pick in picks {
                                random_write(s, pick);
                            }
                            Ok(())
                        })
                        .unwrap();
                    at += 1;
                    model.truncate(at);
                    model.push(serialized(&session));
                }
                2 => {
                    let wrote = rng.next(2) == 0;
                    let pick = rng.next(1000);
                    let redo = session.journal_state().can_redo;
                    let _ = session.mutate("abandoned", |s| {
                        if wrote {
                            random_write(s, pick);
                        }
                        Err::<(), _>(AppError::plain("abandon"))
                    });
                    if wrote {
                        model.truncate(at + 1);
                    }
                    assert_eq!(session.journal_state().can_redo, redo && !wrote);
                }
                3 => {
                    let reply = dispatch(&mut engine, &mut session, json!({"op": "undo"}));
                    assert_eq!(reply["ok"], at > 0, "{reply}");
                    at = at.saturating_sub(1);
                }
                4 => {
                    let reply = dispatch(&mut engine, &mut session, json!({"op": "redo"}));
                    assert_eq!(reply["ok"], at + 1 < model.len(), "{reply}");
                    if at + 1 < model.len() {
                        at += 1;
                    }
                }
                _ => {
                    session.mutate("empty", |_| Ok(())).unwrap();
                }
            }
            let state = session.journal_state();
            assert_eq!(position(&session), at as i64, "seed {seed} round {round}");
            assert_eq!(
                (state.can_undo, state.can_redo),
                (at > 0, at + 1 < model.len())
            );
            assert_eq!(serialized(&session), model[at], "seed {seed} round {round}");
        }
        // The view agrees with the journal after any history.
        let view_pages = session.view().unwrap().page_count();
        assert_eq!(view_pages, session.page_state().count());
    }
}

fn random_write(session: &mut AppSession, pick: u64) {
    let mut tx = session.tx().unwrap();
    let next = tx.document().next_id().unwrap();
    match pick % 5 {
        0 => {
            tx.allocate(Object::Integer(pick as i64), None).unwrap();
        }
        1 => {
            let target = id(4 + 2 * (pick as u32 % 3));
            tx.update(target, |page| {
                page.as_dict_mut()
                    .unwrap()
                    .insert(b"Rotate".to_vec(), Object::Integer(90 * (pick as i64 % 4)));
            })
            .unwrap();
        }
        2 => {
            let target = id(5 + 2 * (pick as u32 % 3));
            let data = format!("BT /F1 24 Tf 72 700 Td (W{pick}) Tj ET").into_bytes();
            tx.replace_decoded_stream(target, data).unwrap();
        }
        3 => {
            let last = id(next.number() - 1);
            if last.number() > 9 {
                tx.remove_object(last).unwrap();
            } else {
                tx.set_trailer_entry(b"ID", Some(Object::array([Object::Integer(pick as i64)])))
                    .unwrap();
            }
        }
        _ => {
            let info = tx
                .allocate(
                    Object::dictionary([(b"Title".to_vec(), Object::text("T"))]),
                    None,
                )
                .unwrap();
            tx.set_trailer_entry(b"Info", Some(Object::Reference(info)))
                .unwrap();
        }
    }
}

// ---------------------------------------------------------------- render bytes

#[test]
fn render_bytes_are_the_input_until_something_changes() {
    for bytes in [
        pages_pdf(2),
        xref_stream_pdf(2),
        incremental_pdf(2),
        fixture("large.pdf"),
    ] {
        let (_, session) = session_with(bytes.clone());
        assert_eq!(session.snapshot_bytes().unwrap(), bytes);
        let mut doc = AppDocument::load(Arc::new(bytes.clone()), b"").unwrap();
        assert_eq!(doc.render_bytes().unwrap(), bytes);
    }
}

#[test]
fn hayro_reads_an_overlay_on_classic_xref_stream_and_incremental_bases() {
    for (label, bytes) in [
        ("classic", pages_pdf(3)),
        ("xref stream", xref_stream_pdf(3)),
        ("incremental", incremental_pdf(3)),
    ] {
        let (_, mut session) = session_with(bytes.clone());
        let first = if label == "incremental" {
            "Updated"
        } else {
            "Page 0"
        };
        assert_eq!(page_text(&mut session, 0).trim(), first, "{label}");
        let orphan = session
            .mutate("edit", |s| {
                replace_text(s, 1, "Edited")?;
                let mut tx = s.tx()?;
                // Packed into the object stream in the xref-stream base.
                tx.update(id(2), |pages| {
                    let kids = pages.as_dict_mut().unwrap().get_mut(b"Kids".as_slice());
                    let kids = kids.unwrap().as_array_mut().unwrap();
                    kids.swap(0, 2);
                })
                .unwrap();
                tx.update(id(4), |page| {
                    page.as_dict_mut().unwrap().insert(
                        b"MediaBox".to_vec(),
                        Object::array([0, 0, 200, 300].map(Object::Integer)),
                    );
                })
                .unwrap();
                let orphan = tx.allocate(Object::Integer(1), None)?;
                tx.remove_object(orphan)?;
                Ok(orphan)
            })
            .unwrap()
            .unwrap();
        let snapshot = session.snapshot_bytes().unwrap();
        assert!(
            snapshot.starts_with(&bytes),
            "{label}: the base is kept as it was"
        );
        assert!(
            snapshot.len() - bytes.len() < 2048,
            "{label}: only changed objects"
        );
        let view = session.view().unwrap();
        assert_eq!(view.page_count(), 3, "{label}");
        assert_eq!(view.page_size(2).unwrap(), (200.0, 300.0), "{label}");
        assert_eq!(
            view.structured_text(1).unwrap().as_text().trim(),
            "Edited",
            "{label}"
        );
        assert_eq!(
            view.structured_text(0).unwrap().as_text().trim(),
            "Page 2",
            "{label}"
        );
        // The core reader parses the same bytes to the same objects.
        let reread = Document::open(&snapshot, Default::default()).unwrap();
        let doc = session.doc.as_ref().unwrap().document();
        for number in 1..=9 {
            assert_eq!(
                reread.object(id(number)).map(|o| &o.value).ok(),
                doc.object(id(number)).map(|o| &o.value).ok(),
                "{label} object {number}"
            );
        }
        assert!(
            reread.object(orphan).is_err(),
            "{label}: the removed object is free"
        );
    }
}

#[test]
fn encrypted_documents_render_from_a_plaintext_base_and_snapshot_their_own_encryption() {
    let source = fixture("encrypted.pdf");
    let (mut engine, mut session) = session_with(source.clone());
    assert_eq!(
        session.snapshot_bytes().unwrap(),
        source,
        "locked: the source"
    );
    let auth = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "pdf4all-user"}),
    );
    assert_eq!(auth["authenticated"], true);
    assert_eq!(
        session.snapshot_bytes().unwrap(),
        source,
        "unchanged: still encrypted"
    );
    assert!(page_text(&mut session, 0).contains("PDF4All encrypted"));
    session
        .mutate("edit", |s| replace_text(s, 0, "Plain now"))
        .unwrap();
    assert_eq!(page_text(&mut session, 0).trim(), "Plain now");
    let snapshot = session.snapshot_bytes().unwrap();
    let reread = Document::open(&snapshot, Default::default()).unwrap();
    assert!(reread.security().is_none() && reread.trailer_entry(b"Encrypt").is_none());
    dispatch(&mut engine, &mut session, json!({"op": "undo"}));
    assert_eq!(session.snapshot_bytes().unwrap(), source);
    assert!(page_text(&mut session, 0).contains("PDF4All encrypted"));
}

#[test]
fn a_large_overlay_rebases_onto_a_rewrite() {
    let (_, mut session) = session_with(pages_pdf(2));
    let big: Vec<u8> = (0..3u32 << 20).map(|i| (i % 251) as u8).collect();
    session
        .mutate("big", |s| {
            let mut tx = s.tx()?;
            let mut dict = Dictionary::new();
            dict.insert(b"Kind".to_vec(), Object::name("Blob"));
            let blob = tx.allocate_stream_raw(dict, big.clone())?;
            tx.update(id(1), |catalog| {
                catalog
                    .as_dict_mut()
                    .unwrap()
                    .insert(b"Blob".to_vec(), Object::Reference(blob));
            })?;
            Ok(())
        })
        .unwrap();
    let doc = session.doc.as_mut().unwrap();
    let first = doc.render_bytes().unwrap();
    assert!(
        !first.starts_with(&pages_pdf(2)),
        "rebased onto a full rewrite"
    );
    let again = doc.render_bytes().unwrap();
    assert_eq!(
        first, again,
        "the rewrite is the new base, with nothing on top"
    );
    assert_eq!(page_text(&mut session, 1).trim(), "Page 1");
}

#[test]
fn a_rebased_view_follows_a_reference_whose_target_appears_later() {
    let mut objects = page_objects(1, "");
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R /Extra 9 0 R >>".to_vec();
    let (_, mut session) = session_with(classic(&objects, ""));
    let big: Vec<u8> = vec![7; 3 << 20];
    session
        .mutate("big", |s| {
            s.tx()?
                .allocate_stream_raw(Dictionary::new(), big.clone())?;
            Ok(())
        })
        .unwrap();
    session.view().unwrap();
    let extra = |session: &AppSession| {
        let bytes = session.snapshot_bytes().unwrap();
        let doc = Document::open(&bytes, Default::default()).unwrap();
        let catalog = doc.resolve_id(doc.root()).as_dict().unwrap().clone();
        catalog.get(b"Extra".as_slice()).cloned()
    };
    assert_eq!(
        extra(&session),
        Some(Object::Null),
        "a rewrite prints it as null"
    );
    session
        .mutate("fill", |s| {
            let mut tx = s.tx()?;
            while tx.document().next_id()? <= id(9) {
                tx.allocate(
                    Object::dictionary([(b"Marker", Object::Boolean(true))]),
                    None,
                )?;
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(extra(&session), Some(Object::Reference(id(9))));
    session.view().unwrap();
    assert_eq!(extra(&session), Some(Object::Reference(id(9))));
}

// ---------------------------------------------------------------- geometry and facts

#[test]
fn page_geometry_matches_the_view_on_every_fixture_and_odd_boxes() {
    let names = [
        "cjk.pdf",
        "edit-tj.pdf",
        "form.pdf",
        "form2.pdf",
        "hello.pdf",
        "large.pdf",
        "layers.pdf",
        "nav.pdf",
        "redact-shared-only.pdf",
        "redact.pdf",
        "rotated.pdf",
        "rtl.pdf",
    ];
    let mut inputs: Vec<(String, Vec<u8>)> = names
        .iter()
        .map(|name| (name.to_string(), fixture(name)))
        .collect();
    for (label, extra) in [
        ("crop", "/CropBox [10 20 300 400]"),
        ("rotate 90", "/Rotate 90 /CropBox [0 0 100 50]"),
        ("rotate -90", "/Rotate -90"),
        ("rotate 45", "/Rotate 45"),
        ("rotate real", "/Rotate 179.6"),
        ("user unit", "/UserUnit 2.5"),
        ("empty media", "/MediaBox [0 0 0 0]"),
        ("sliver", "/CropBox [0 0 0.5 700]"),
        ("bad media", "/MediaBox /Letter"),
        ("odd numbers", "/MediaBox [0.1 0.2 595.276 841.89]"),
        ("reversed", "/MediaBox [612 792 0 0]"),
    ] {
        let objects = page_objects(1, extra);
        inputs.push((label.to_string(), classic(&objects, "")));
    }
    // Inherited boxes and rotation from the page tree.
    let mut objects = page_objects(2, "");
    objects[1] =
        b"<< /Type /Pages /Kids [4 0 R 6 0 R] /Count 2 /Rotate 270 /MediaBox [0 0 300 500] >>"
            .to_vec();
    objects[3] = b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>".to_vec();
    inputs.push(("inherited".into(), classic(&objects, "")));
    for (label, bytes) in inputs {
        let (_, mut session) = session_with(bytes);
        let state = session.page_state();
        let view = session.view().unwrap();
        assert_eq!(state.count(), view.page_count(), "{label}");
        for (page, size) in state.sizes.iter().enumerate() {
            assert_eq!(*size, view.page_size(page).unwrap(), "{label} page {page}");
        }
    }
}

#[test]
fn rotations_are_the_raw_inherited_rotate_truncated() {
    for (extra, rotation) in [
        ("", 0),
        ("/Rotate 90", 90),
        ("/Rotate -90", 270),
        ("/Rotate 45", 45),
        ("/Rotate 450", 90),
        ("/Rotate 179.6", 179),
        ("/Rotate -0.5", 0),
        ("/Rotate /R", 0),
    ] {
        let (_, session) = session_with(classic(&page_objects(1, extra), ""));
        assert_eq!(session.page_state().rotations, [rotation], "{extra}");
    }
    let (_, session) = session_with(fixture("rotated.pdf"));
    let pages = session.page_state().json();
    assert_eq!(
        pages["rotations"],
        json!([0]),
        "its content is rotated, not the page"
    );
}

#[test]
fn page_lists_follow_the_view_through_odd_trees() {
    // A page listed twice is drawn once; a direct page dictionary is a page
    // without an object number; a Pages node without Kids has no pages.
    let mut objects = page_objects(2, "");
    objects[1] = b"<< /Type /Pages /Kids [4 0 R 4 0 R 9 0 R << /Type /Page /Contents 7 0 R >> 6 0 R] /Count 9 >>".to_vec();
    objects.push(b"<< /Type /Pages /Count 3 >>".to_vec());
    let (mut engine, mut session) = session_with(classic(&objects, ""));
    let ids = dispatch(
        &mut engine,
        &mut session,
        json!({"id": 1, "op": "page-ids"}),
    );
    assert_eq!(ids, json!({"id": 1, "ok": true, "ids": [4, -1, 6]}));
    assert_eq!(session.view().unwrap().page_count(), 3);

    // Without a usable page tree, every dictionary with /Contents.
    let mut objects = page_objects(2, "");
    objects[0] = b"<< /Type /Catalog /Pages 99 0 R >>".to_vec();
    let (mut engine, mut session) = session_with(classic(&objects, ""));
    let ids = dispatch(&mut engine, &mut session, json!({"op": "page-ids"}));
    assert_eq!(ids["ids"], json!([4, 6]));
    assert_eq!(session.view().unwrap().page_count(), 2);
}

fn info_of(bytes: Vec<u8>) -> DocumentInfo {
    let (_, session) = session_with(bytes);
    session.info().unwrap()
}

#[test]
fn detect_reports_every_document_fact() {
    let plain = info_of(fixture("hello.pdf"));
    assert_eq!(
        plain.json(),
        json!({"encrypted": false, "needsPassword": false, "algorithm": "none",
            "repaired": false, "signatures": 0, "certified": false, "xfa": false,
            "attachments": 0, "openAction": false, "tagged": false, "versions": 0,
            "printable": true, "permissions": {"print": true, "copy": true, "edit": true,
            "annotate": true, "form": true}})
    );
    let mut objects = page_objects(1, "");
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R /AcroForm 6 0 R /Perms << /DocMDP 9 0 R >> \
        /OpenAction [4 0 R /Fit] /MarkInfo << /Marked true >> \
        /Names << /EmbeddedFiles << /Kids [10 0 R 10 0 R] /Names [(a.txt) 9 0 R] >> >> >>"
        .to_vec();
    objects.push(b"<< /Fields [7 0 R 8 0 R 7 0 R] /XFA [] >>".to_vec()); // 6
    objects.push(b"<< /FT /Sig /V 9 0 R /Kids [8 0 R] >>".to_vec()); // 7: signed, shared kid
    objects.push(b"<< /FT /Sig /Kids [11 0 R] >>".to_vec()); // 8: blank
    objects.push(b"<< /Type /Sig >>".to_vec()); // 9
    objects.push(
        b"<< /Names [(b.txt) 9 0 R (a.txt) 9 0 R <FEFF0061002E007400780074> 9 0 R] >>".to_vec(),
    ); // 10
    objects.push(b"<< /Parent 8 0 R /FT /Sig /V 9 0 R >>".to_vec()); // 11: signed kid
    let facts = info_of(classic(&objects, ""));
    assert_eq!(
        facts.signatures, 2,
        "own /FT /Sig with own /V, each node once"
    );
    assert!(facts.certified && facts.xfa && facts.open_action && facts.tagged);
    assert_eq!(facts.attachments, 2, "distinct decoded names");
    assert!(facts.signed());

    let mut objects = page_objects(1, "");
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R /AA << >> /MarkInfo << /Marked 1 >> >>".to_vec();
    let facts = info_of(classic(&objects, ""));
    assert!(facts.open_action && !facts.tagged);
    assert_eq!(info_of(incremental_pdf(1)).versions, 1);
}

#[test]
fn encryption_facts_come_from_the_encryption_dictionary_even_while_locked() {
    for (name, algorithm, print, edit) in [
        ("encrypted.pdf", "rc4-128", true, true),
        ("encrypted-noprint.pdf", "rc4-128", false, true),
        ("encrypted-noedit.pdf", "rc4-128", true, false),
        ("encrypted-aes256.pdf", "aes-256", true, true),
    ] {
        let info = info_of(fixture(name));
        assert!(info.encrypted && info.needs_password, "{name}");
        assert_eq!(info.algorithm, algorithm, "{name}");
        assert_eq!(
            (
                info.permissions.print,
                info.printable,
                info.permissions.edit
            ),
            (print, print, edit),
            "{name}"
        );
    }
    for (name, algorithm) in [
        ("encrypted-aes128.pdf", "aes-128"),
        ("encrypted-rc4-40.pdf", "rc4-40"),
        ("encrypted-aes128-objstm.pdf", "aes-128"),
        ("encrypted-aes256-objstm.pdf", "aes-256"),
    ] {
        let bytes = std::fs::read(format!("tests/data/{name}")).unwrap();
        let (_, session) = session_with(bytes);
        let info = session.info().unwrap();
        assert_eq!(
            (info.encrypted, info.algorithm),
            (true, algorithm),
            "{name}"
        );
    }
    // Owner authentication grants every permission; the user gets /P.
    let (mut engine, mut session) = session_with(fixture("encrypted-noedit.pdf"));
    let user = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "pdf4all-user"}),
    );
    assert_eq!(user["info"]["permissions"]["edit"], false);
    assert_eq!(user["info"]["permissions"]["form"], true);
}

#[test]
fn permission_needs_follow_the_op_table() {
    assert_eq!(OP_PERMISSION.len(), 31);
    assert_eq!(op_permission("field-set"), Need::Form);
    assert_eq!(op_permission("field-add"), Need::EditAnnotate);
    assert_eq!(op_permission("clip-copy"), Need::Copy);
    assert_eq!(op_permission("undo"), Need::Any);
    assert_eq!(op_permission("anything-else"), Need::Edit);
    let mut info = info_of(fixture("hello.pdf"));
    info.permissions = Permissions::from_bits(0);
    assert!(
        Need::Edit.allowed(&info),
        "an unencrypted document permits everything"
    );
    info.encrypted = true;
    assert!(!Need::Any.allowed(&info));
    info.permissions.form = true;
    assert!(Need::Any.allowed(&info) && !Need::Edit.allowed(&info));
    info.permissions.edit = true;
    assert!(!Need::EditAnnotate.allowed(&info));
    info.permissions.annotate = true;
    assert!(Need::EditAnnotate.allowed(&info));
    assert_eq!(
        Permissions::from_bits(4 | 32 | 256).names(),
        ["print", "annotate", "form"]
    );
    assert_eq!(Permissions::ALL.bits(), 4 + 8 + 16 + 32 + 256);
}

#[test]
fn require_mutable_refuses_in_engine_order() {
    let none = AppSession::default();
    let error = none.require_mutable("labels-set").unwrap_err();
    assert_eq!(
        (error.reason.as_deref(), error.message.as_str()),
        (Some("no-document"), "labels-set: no document open")
    );

    let (_, locked) = session_with(fixture("encrypted-noedit.pdf"));
    let error = locked.require_mutable("labels-set").unwrap_err();
    assert_eq!(
        error.message,
        "labels-set: this document is read-only until it is unlocked"
    );

    let (mut engine, mut user) = session_with(fixture("encrypted-noedit.pdf"));
    dispatch(
        &mut engine,
        &mut user,
        json!({"op": "authenticate", "password": "pdf4all-user"}),
    );
    for (op, message) in [
        (
            "labels-set",
            "labels-set: this document does not permit changes to its content",
        ),
        (
            "annot-add",
            "annot-add: this document does not permit annotations",
        ),
        (
            "field-add",
            "field-add: this document does not permit authoring form fields",
        ),
    ] {
        let error = user.require_mutable(op).unwrap_err();
        assert_eq!(
            (error.reason.as_deref(), error.message.as_str()),
            (Some("read-only"), message)
        );
    }
    assert!(
        user.require_mutable("undo").is_ok(),
        "form is allowed, so any is"
    );
    assert!(user.require_mutable("field-set").is_ok());

    let mut objects = page_objects(1, "");
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R /Perms << /DocMDP << >> >> >>".to_vec();
    let (_, signed) = session_with(classic(&objects, ""));
    let error = signed.require_mutable("undo").unwrap_err();
    assert_eq!(error.message, "undo: this document is read-only");
}

#[test]
fn check_page_accepts_javascript_integers_in_range() {
    let (_, session) = session_with(pages_pdf(3));
    for page in [json!(0), json!(2), json!(1.0), json!(-0.0)] {
        assert!(session.check_page(Some(&page), "x").is_ok(), "{page}");
    }
    for page in [
        json!(3),
        json!(-1),
        json!(0.5),
        json!("1"),
        json!(null),
        json!(1e300),
    ] {
        let error = session.check_page(Some(&page), "labels-set").unwrap_err();
        assert_eq!(
            (error.reason.as_deref(), error.message.as_str()),
            (
                Some("invalid"),
                "labels-set: page must be an integer in 0..2"
            )
        );
    }
    assert!(session.check_page(None, "x").is_err());
    assert_eq!(
        AppSession::default()
            .check_page(Some(&json!(0)), "x")
            .unwrap_err()
            .message,
        "x: page must be an integer in 0..0"
    );
}

// ---------------------------------------------------------------- open

#[test]
fn open_replies_with_pages_info_and_a_fresh_session_state() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    engine.next_generation();
    let reply = open(&mut engine, &mut session, fixture("hello.pdf"));
    assert_eq!(
        reply,
        json!({"id": 1, "ok": true, "pageCount": 1, "pages": [{"width": 612.0, "height": 792.0}],
            "rotations": [0], "info": info_of(fixture("hello.pdf")).json(), "openMs": 0,
            "revision": 0, "auth": "none"})
    );
    assert_eq!(session.generation, 2);
    assert_eq!(session.name.as_deref(), Some("a.pdf"));
    assert_eq!(session.source_bytes, fixture("hello.pdf").len());
    assert!(!session.locked && session.locked_bytes.is_none() && session.signed_source.is_none());
    assert_eq!(
        (session.saved_position, session.journal_state().dirty),
        (0, false)
    );

    // Everything document-related resets; clipboard, compare and batch stay.
    session.security = Some(SecuritySetting::None);
    session.security_dirty = true;
    session.password = Some("p".into());
    session.auth = Auth::Owner;
    session.revision = 5;
    session.batch = Batch::Plan;
    open(&mut engine, &mut session, pages_pdf(4));
    assert!(session.security.is_none() && !session.security_dirty && session.password.is_none());
    assert_eq!(
        (session.auth, session.revision, session.batch),
        (Auth::None, 0, Batch::Plan)
    );
    assert_eq!(session.page_state().count(), 4);
}

#[test]
fn open_refuses_in_engine_order_and_keeps_the_previous_document() {
    let (mut engine, mut session) = session_with(fixture("hello.pdf"));
    let generation = session.generation;
    let cases: Vec<(Value, Vec<Vec<u8>>, &str)> = vec![
        (
            json!({"op": "open", "pdf": {"$buffer": 0}}),
            vec![b"%PDF-".to_vec()],
            "open: missing document name",
        ),
        (
            json!({"op": "open", "name": "", "pdf": {"$buffer": 0}}),
            vec![b"%PDF-".to_vec()],
            "open: missing document name",
        ),
        (
            json!({"op": "open", "name": 7}),
            vec![],
            "open: missing document name",
        ),
        (json!({"op": "open", "name": "a"}), vec![], "open: no bytes"),
        (
            json!({"op": "open", "name": "a", "pdf": null}),
            vec![],
            "open: no bytes",
        ),
        (
            json!({"op": "open", "name": "a", "pdf": 0}),
            vec![],
            "open: no bytes",
        ),
        (
            json!({"op": "open", "name": "a", "pdf": ""}),
            vec![],
            "open: no bytes",
        ),
        (
            json!({"op": "open", "name": "a", "pdf": {"$number": "NaN"}}),
            vec![],
            "open: no bytes",
        ),
        (
            json!({"op": "open", "name": "a", "pdf": "%PDF-1.7"}),
            vec![],
            "open: expected an ArrayBuffer",
        ),
        (
            json!({"op": "open", "name": "a", "pdf": {"0": 1}}),
            vec![],
            "open: expected an ArrayBuffer",
        ),
        (
            json!({"op": "open", "name": "a", "pdf": {"$buffer": 0}}),
            vec![b"%PDF".to_vec()],
            "open: not a PDF (too few bytes)",
        ),
        (
            json!({"op": "open", "name": "a", "pdf": {"$buffer": 0}}),
            vec![Vec::new()],
            "open: not a PDF (too few bytes)",
        ),
    ];
    for (request, buffers, message) in cases {
        let (reply, _) = dispatch_buffers(&mut engine, &mut session, request.clone(), buffers);
        assert_eq!(
            (&reply["ok"], &reply["error"]),
            (&json!(false), &json!(message)),
            "{request}"
        );
        assert!(reply.get("reason").is_none(), "plain: {request}");
    }
    for junk in [b"%PDF-1.7\ngarbage".to_vec(), vec![0; 64]] {
        let request = json!({"op": "open", "name": "a", "pdf": {"$buffer": 0}});
        let (reply, _) = dispatch_buffers(&mut engine, &mut session, request, vec![junk]);
        let error = reply["error"].as_str().unwrap();
        assert!(
            error.starts_with("open: the file could not be read ("),
            "{error}"
        );
        assert!(reply.get("reason").is_none());
    }
    let mut objects = page_objects(0, "");
    objects.truncate(3);
    let request = json!({"op": "open", "name": "a", "pdf": {"$buffer": 0}});
    let (reply, _) = dispatch_buffers(
        &mut engine,
        &mut session,
        request,
        vec![classic(&objects, "")],
    );
    assert_eq!(reply["error"], "open: PDF has no pages");
    assert_eq!(session.generation, generation);
    assert_eq!(session.page_state().count(), 1, "hello.pdf is still open");

    // Over the ceiling, with a compare slot: the slot goes first and the
    // failure says so.
    session.compare = Some(CompareDocument::open("b.pdf", fixture("hello.pdf")).unwrap());
    let request = json!({"op": "open", "name": "a", "pdf": {"$buffer": 0}});
    let (reply, _) = dispatch_buffers(
        &mut engine,
        &mut session,
        request,
        vec![vec![0; MAX_PDF_BYTES + 1]],
    );
    assert_eq!(
        reply["error"],
        "open: PDF exceeds the 125829120 byte ceiling"
    );
    assert_eq!(reply["compareClosed"], true);
    assert!(session.compare.is_none());
}

#[test]
fn open_repairs_damaged_files_and_reports_them() {
    let good = fixture("hello.pdf");
    let text = String::from_utf8_lossy(&good).into_owned();
    let at = text.rfind("startxref").unwrap();
    let offset: usize = text[at + 9..]
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let broken = format!("{}startxref\n{}\n%%EOF\n", &text[..at], offset - 5).into_bytes();
    let (mut engine, mut session) = session_with(broken);
    assert!(session.info().unwrap().repaired);
    assert!(page_text(&mut session, 0).contains("PDF4All"));
    let saved = dispatch_buffers(&mut engine, &mut session, json!({"op": "save"}), vec![]);
    assert_eq!(saved.0["ok"], true, "{}", saved.0);

    // Bytes before the header and trailing garbage need rebuilding but are
    // not damage.
    let mut shifted = b"junk\n".to_vec();
    shifted.extend_from_slice(&good);
    let (_, session) = session_with(shifted);
    assert_eq!(session.page_state().count(), 1);
}

#[test]
fn locked_documents_report_structure_without_the_password() {
    let source = fixture("encrypted.pdf");
    let (_, session) = session_with(source.clone());
    assert!(session.locked);
    assert_eq!(session.locked_bytes.as_deref(), Some(&source));
    assert_eq!(
        session.page_state().json()["pages"],
        json!([{"width": 612.0, "height": 792.0}])
    );

    // Object streams cannot be read without the key: MuPDF reports no pages
    // and keeps the document open for the password.
    let bytes = std::fs::read("tests/data/encrypted-aes256-objstm.pdf").unwrap();
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let reply = open(&mut engine, &mut session, bytes.clone());
    assert_eq!(
        (&reply["info"]["needsPassword"], &reply["pageCount"]),
        (&json!(true), &json!(0))
    );
    assert_eq!(reply["info"]["algorithm"], "aes-256");
    let unlocked = Document::open_with_password(&bytes, Default::default(), b"u").unwrap();
    let auth = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "u"}),
    );
    assert_eq!(
        (&auth["auth"], &auth["pageCount"]),
        (&json!("user"), &json!(info::pages(&unlocked).len()))
    );
    assert!(auth["pageCount"].as_u64().unwrap() > 0);
}

// ---------------------------------------------------------------- authenticate and close

#[test]
fn authenticate_refuses_then_unlocks_a_fresh_copy() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let none = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "x"}),
    );
    assert_eq!(none["error"], "authenticate: no document open");

    open(&mut engine, &mut session, fixture("hello.pdf"));
    let plain = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "x"}),
    );
    assert_eq!(plain["error"], "authenticate: document is not locked");

    open(&mut engine, &mut session, fixture("encrypted-aes256.pdf"));
    let generation = session.generation;
    let message = "authenticate: password must be a string of at most 1024 characters";
    for password in [
        json!(null),
        json!(5),
        json!({"p": 1}),
        json!("a".repeat(1025)),
        json!("😀".repeat(513)),
    ] {
        let reply = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "authenticate", "password": password}),
        );
        assert_eq!(
            (&reply["error"], reply.get("reason")),
            (&json!(message), None)
        );
    }
    let missing = dispatch(&mut engine, &mut session, json!({"op": "authenticate"}));
    assert_eq!(missing["error"], message);
    let ok = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "😀".repeat(512)}),
    );
    assert_eq!(ok["authenticated"], false, "1024 UTF-16 units is allowed");

    let wrong = dispatch(
        &mut engine,
        &mut session,
        json!({"id": 3, "op": "authenticate", "password": "wrong"}),
    );
    assert_eq!(wrong, json!({"id": 3, "ok": true, "authenticated": false}));
    assert!(session.locked && session.password.is_none());

    let right = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "pdf4all-user"}),
    );
    assert_eq!(right["authenticated"], true);
    assert_eq!(right["auth"], "user");
    assert_eq!(
        (&right["pageCount"], &right["rotations"]),
        (&json!(1), &json!([0]))
    );
    assert_eq!(right["info"]["needsPassword"], false);
    assert_eq!(right["info"]["encrypted"], true);
    assert!(right.get("revision").is_none() && right.get("password").is_none());
    assert!(!session.locked && session.locked_bytes.is_none());
    assert_eq!(session.password.as_deref(), Some("pdf4all-user"));
    assert_eq!(
        session.generation, generation,
        "authenticate keeps the generation"
    );
    assert!(page_text(&mut session, 0).contains("AES-256"));

    let mut owner = AppSession::default();
    open(&mut engine, &mut owner, fixture("encrypted-aes256.pdf"));
    let reply = dispatch(
        &mut engine,
        &mut owner,
        json!({"op": "authenticate", "password": "pdf4all-owner"}),
    );
    assert_eq!(reply["auth"], "owner");
}

#[test]
fn close_drops_the_document_and_keeps_clipboard_compare_and_batch() {
    let (mut engine, mut session) = session_with(fixture("hello.pdf"));
    session.compare = Some(CompareDocument::open("b.pdf", fixture("hello.pdf")).unwrap());
    session.batch = Batch::Run;
    let generation = session.generation;
    let reply = dispatch(&mut engine, &mut session, json!({"id": 2, "op": "close"}));
    assert_eq!(reply, json!({"id": 2, "ok": true}));
    assert!(session.doc.is_none() && session.compare.is_some());
    assert_eq!(
        (session.batch, session.generation),
        (Batch::Run, generation + 1)
    );
    assert_eq!(session.snapshot_bytes(), None);
}

// ---------------------------------------------------------------- undo and redo

#[test]
fn undo_and_redo_reply_with_the_edit_state_and_step() {
    let (mut engine, mut session) = session_with(pages_pdf(2));
    let nothing = dispatch(&mut engine, &mut session, json!({"id": 1, "op": "undo"}));
    assert_eq!(
        nothing,
        json!({"id": 1, "ok": false, "error": "undo: there is nothing to undo", "revision": 0,
            "dirty": false, "canUndo": false, "canRedo": false})
    );
    let nothing = dispatch(&mut engine, &mut session, json!({"op": "redo"}));
    assert_eq!(nothing["error"], "redo: there is nothing to redo");

    session
        .mutate("rotate page", |s| {
            s.tx()?
                .update(id(4), |page| {
                    page.as_dict_mut()
                        .unwrap()
                        .insert(b"Rotate".to_vec(), Object::Integer(90));
                })
                .map_err(|e| AppError::plain(e.to_string()))
        })
        .unwrap();
    let undo = dispatch(&mut engine, &mut session, json!({"id": 2, "op": "undo"}));
    assert_eq!(
        undo,
        json!({"id": 2, "ok": true, "revision": 2, "dirty": false, "canUndo": false,
            "canRedo": true, "pageCount": 2, "pages": [{"width": 612.0, "height": 792.0},
            {"width": 612.0, "height": 792.0}], "rotations": [0, 0], "step": "rotate page"})
    );
    let redo = dispatch(&mut engine, &mut session, json!({"op": "redo"}));
    assert_eq!(
        (&redo["step"], &redo["rotations"], &redo["revision"]),
        (&json!("rotate page"), &json!([90, 0]), &json!(3))
    );
    assert_eq!(redo["pages"][0], json!({"width": 792.0, "height": 612.0}));

    let mut empty = AppSession::default();
    let reply = dispatch(&mut engine, &mut empty, json!({"op": "undo"}));
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (&json!("no-document"), &json!("undo: no document open"))
    );
    let (mut engine, mut locked) = session_with(fixture("encrypted.pdf"));
    let reply = dispatch(&mut engine, &mut locked, json!({"op": "redo"}));
    assert_eq!(reply["reason"], "read-only");
}

// ---------------------------------------------------------------- security-set

#[test]
fn security_set_validates_in_order_and_stores_a_pending_setting() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "security-set", "mode": "none"}),
    );
    assert_eq!(reply["error"], "security-set: no document open");
    open(&mut engine, &mut session, fixture("encrypted.pdf"));
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "security-set", "mode": "none"}),
    );
    assert_eq!(reply["error"], "security-set: the document is locked");
    dispatch(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "pdf4all-user"}),
    );
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "security-set", "mode": "none"}),
    );
    assert_eq!(
        reply["error"],
        "security-set: open the document with its owner password to change security"
    );
    let mut objects = page_objects(1, "");
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R /Perms << /DocMDP << >> >> >>".to_vec();
    open(&mut engine, &mut session, classic(&objects, ""));
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "security-set", "mode": "keep"}),
    );
    assert_eq!(
        reply["error"],
        "security-set: signed or certified documents keep their security"
    );

    open(&mut engine, &mut session, fixture("hello.pdf"));
    for (request, message) in [
        (
            json!({}),
            "security-set: mode must be one of keep, none, aes-256",
        ),
        (
            json!({"mode": "rc4-128"}),
            "security-set: mode must be one of keep, none, aes-256",
        ),
        (
            json!({"mode": "aes-256"}),
            "security-set: user password must be a string",
        ),
        (
            json!({"mode": "aes-256", "userPassword": 42}),
            "security-set: user password must be a string",
        ),
        (
            json!({"mode": "aes-256", "userPassword": "x".repeat(1025)}),
            "security-set: user password must be at most 1024 characters",
        ),
        (
            json!({"mode": "aes-256", "userPassword": "a,b"}),
            "security-set: user password cannot contain a comma",
        ),
        (
            json!({"mode": "aes-256", "userPassword": ""}),
            "security-set: AES-256 needs a non-empty user password",
        ),
        (
            json!({"mode": "aes-256", "userPassword": "u", "ownerPassword": null}),
            "security-set: owner password must be a string",
        ),
        (
            json!({"mode": "aes-256", "userPassword": "u", "ownerPassword": "a,b"}),
            "security-set: owner password cannot contain a comma",
        ),
        (
            json!({"mode": "aes-256", "userPassword": "u", "permissions": {"print": "yes"}}),
            "security-set: print must be true or false",
        ),
        (
            json!({"mode": "aes-256", "userPassword": "u", "permissions": {"form": null, "copy": 1}}),
            "security-set: copy must be true or false",
        ),
    ] {
        let mut request = request;
        request["op"] = json!("security-set");
        let reply = dispatch(&mut engine, &mut session, request.clone());
        assert_eq!(
            (&reply["error"], reply.get("reason")),
            (&json!(message), None),
            "{request}"
        );
        assert!(session.security.is_none() && session.revision == 0);
    }

    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"id": 4, "op": "security-set",
        "mode": "aes-256", "userPassword": "solo", "permissions": {"print": true, "annotate": true, "extra": 1}}),
    );
    assert_eq!(
        reply,
        json!({"id": 4, "ok": true, "revision": 1, "dirty": true, "canUndo": false, "canRedo": false,
            "security": {"mode": "aes-256", "algorithm": "aes-256", "permissions": ["print", "annotate"],
                "auth": "none", "pending": true}})
    );
    assert_eq!(
        session.security,
        Some(SecuritySetting::Aes256 {
            user_password: "solo".into(),
            owner_password: "solo".into(),
            permissions: 36
        })
    );
    for (mode, algorithm, names) in [
        (
            "none",
            "none",
            json!(["print", "edit", "copy", "annotate", "form"]),
        ),
        (
            "keep",
            "none",
            json!(["print", "edit", "copy", "annotate", "form"]),
        ),
    ] {
        let reply = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "security-set", "mode": mode,
            "userPassword": 5}),
        );
        assert_eq!(
            reply["security"],
            json!({"mode": mode, "algorithm": algorithm, "permissions": names,
            "auth": "none", "pending": true})
        );
    }
    assert_eq!(session.revision, 3);
    let (mut engine, mut owner) = session_with(fixture("encrypted-noedit.pdf"));
    dispatch(
        &mut engine,
        &mut owner,
        json!({"op": "authenticate", "password": "pdf4all-owner"}),
    );
    let reply = dispatch(
        &mut engine,
        &mut owner,
        json!({"op": "security-set", "mode": "keep"}),
    );
    assert_eq!(reply["security"]["algorithm"], "rc4-128");
    assert_eq!(reply["security"]["auth"], "owner");
}

// ---------------------------------------------------------------- save, validate, saved

fn save(engine: &mut AppEngine, session: &mut AppSession) -> (Value, Option<Vec<u8>>) {
    let (reply, mut outputs) =
        dispatch_buffers(engine, session, json!({"id": 5, "op": "save"}), vec![]);
    (reply, outputs.pop())
}

fn validate(engine: &mut AppEngine, session: &mut AppSession, bytes: Vec<u8>) -> Value {
    let request = json!({"id": 6, "op": "validate", "pdf": {"$buffer": 0}});
    dispatch_buffers(engine, session, request, vec![bytes]).0
}

#[test]
fn save_refuses_in_engine_order() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let (reply, bytes) = save(&mut engine, &mut session);
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (&json!("no-document"), &json!("save: no document open"))
    );
    assert!(bytes.is_none());

    open(&mut engine, &mut session, fixture("encrypted.pdf"));
    session.source_bytes = MAX_SAVE_BYTES + 1;
    let (reply, _) = save(&mut engine, &mut session);
    assert_eq!(reply["reason"], "too-large");
    assert_eq!(
        reply["error"],
        "save: 52428801 bytes is over the 52428800 byte save ceiling"
    );
    session.source_bytes = MAX_SAVE_BYTES;
    let (reply, _) = save(&mut engine, &mut session);
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (
            &json!("encrypted"),
            &json!("save: this document is encrypted; enter its password before saving")
        )
    );

    let mut objects = page_objects(1, "");
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R] >> >>".to_vec();
    objects.push(b"<< /FT /Sig /V << /Type /Sig >> >>".to_vec());
    open(&mut engine, &mut session, classic(&objects, ""));
    let (reply, _) = save(&mut engine, &mut session);
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (
            &json!("read-only"),
            &json!("save: signed or certified documents are read-only")
        )
    );
    assert!(
        session.signed_source.is_some(),
        "a signed source is kept for signing"
    );

    // Encrypted output needs the adapter's entropy; without it the writer
    // refuses rather than writing a predictable key.
    open(&mut engine, &mut session, fixture("encrypted.pdf"));
    dispatch(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "pdf4all-owner"}),
    );
    let (reply, bytes) = save(&mut engine, &mut session);
    assert_eq!(reply["reason"], "invalid", "{reply}");
    assert!(bytes.is_none());

    // keep: the source's own handler, then validate reopens it with the password.
    let (reply, bytes) = save_with_entropy(&mut engine, &mut session);
    assert_eq!(reply["ok"], true, "{reply}");
    let bytes = bytes.unwrap();
    assert!(Document::open(&bytes, Default::default()).is_err());
    assert!(Document::open_with_password(&bytes, Default::default(), b"pdf4all-user").is_ok());
    assert_eq!(validate(&mut engine, &mut session, bytes)["ok"], true);

    // aes-256: a new handler with the new passwords.
    dispatch(
        &mut engine,
        &mut session,
        json!({"op": "security-set", "mode": "aes-256", "userPassword": "u"}),
    );
    let (reply, bytes) = save_with_entropy(&mut engine, &mut session);
    assert_eq!(reply["ok"], true, "{reply}");
    let bytes = bytes.unwrap();
    let reread = Document::open_with_password(&bytes, Default::default(), b"u").unwrap();
    assert_eq!(reread.security().map(|security| security.revision), Some(6));
    assert_eq!(validate(&mut engine, &mut session, bytes)["ok"], true);
}

fn save_with_entropy(engine: &mut AppEngine, session: &mut AppSession) -> (Value, Option<Vec<u8>>) {
    let request = json!({"id": 5, "op": "save", "entropy": {"$buffer": 0}});
    let (reply, mut outputs) = dispatch_buffers(engine, session, request, vec![vec![7; 32]]);
    (reply, outputs.pop())
}

#[test]
fn save_validate_saved_round_trip_with_tickets() {
    let (mut engine, mut session) = session_with(pages_pdf(2));
    session.mutate("a", |s| replace_text(s, 0, "A")).unwrap();
    let (reply, bytes) = save(&mut engine, &mut session);
    let bytes = bytes.unwrap();
    assert_eq!(
        reply,
        json!({"id": 5, "ok": true, "bytes": {"$buffer": 0}, "saveMs": 0, "dirty": true,
            "revision": 1, "security": {"mode": "keep", "algorithm": "none",
            "permissions": ["print", "edit", "copy", "annotate", "form"], "auth": "none",
            "pending": false}, "ticket": format!("{}:1:0", session.generation)})
    );
    assert!(bytes.starts_with(b"%PDF-1.7\n"));
    let reread = Document::open(&bytes, Default::default()).unwrap();
    assert_eq!(reread.trailer_chain().count(), 1);
    let validated = validate(&mut engine, &mut session, bytes);
    assert_eq!(
        validated,
        json!({"id": 6, "ok": true, "pageCount": 2, "bytes": {"$input": 0},
        "dirty": true, "revision": 1})
    );

    // A late acknowledgement marks the saved state, not the newer edit.
    let ticket = reply["ticket"].clone();
    session.mutate("b", |s| replace_text(s, 0, "B")).unwrap();
    let saved = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "saved", "ticket": ticket}),
    );
    assert_eq!((&saved["dirty"], session.saved_position), (&json!(true), 1));
    dispatch(&mut engine, &mut session, json!({"op": "undo"}));
    assert!(!session.journal_state().dirty, "the saved state is clean");

    // A ticket's state may be on the redo branch; once cut away, nothing is clean.
    let ticket = save(&mut engine, &mut session).0["ticket"].clone();
    dispatch(&mut engine, &mut session, json!({"op": "undo"}));
    dispatch(
        &mut engine,
        &mut session,
        json!({"op": "saved", "ticket": ticket.clone()}),
    );
    assert_eq!(session.saved_position, 1);
    assert!(session.journal_state().dirty);
    dispatch(&mut engine, &mut session, json!({"op": "redo"}));
    assert!(!session.journal_state().dirty);
    dispatch(&mut engine, &mut session, json!({"op": "undo"}));
    session.mutate("c", |s| replace_text(s, 0, "C")).unwrap();
    assert_eq!(session.saved_position, -1, "mutate's own rule");
    dispatch(
        &mut engine,
        &mut session,
        json!({"op": "saved", "ticket": ticket}),
    );
    assert_eq!(session.saved_position, -1, "the ticket's state is gone");

    // Ticket-less requests keep engine.ts semantics; bad tickets are refused.
    let saved = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "saved", "ticket": null}),
    );
    assert_eq!(
        (&saved["dirty"], session.saved_position),
        (&json!(false), 1)
    );
    for ticket in [
        json!("x"),
        json!(5),
        json!("1:2"),
        json!("1:2:3:4"),
        json!("1:-2:3"),
    ] {
        let reply = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "saved", "ticket": ticket}),
        );
        assert_eq!(reply["reason"], "invalid", "{ticket}");
    }
    let other = format!("{}:0:0", session.generation + 1);
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "saved", "ticket": other}),
    );
    assert_eq!(
        reply["error"],
        "saved: the save ticket belongs to another document"
    );
    let none = dispatch(
        &mut engine,
        &mut AppSession::default(),
        json!({"op": "saved"}),
    );
    assert_eq!(
        (&none["reason"], &none["error"]),
        (&json!("no-document"), &json!("saved: no document open"))
    );
}

#[test]
fn saved_clears_a_security_change_only_when_the_ticket_wrote_it() {
    let (mut engine, mut session) = session_with(fixture("encrypted.pdf"));
    dispatch(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "pdf4all-owner"}),
    );
    dispatch(
        &mut engine,
        &mut session,
        json!({"op": "security-set", "mode": "none"}),
    );
    let (reply, bytes) = save(&mut engine, &mut session);
    assert_eq!(reply["security"]["mode"], "none");
    let bytes = bytes.unwrap();
    assert!(
        Document::open(&bytes, Default::default())
            .unwrap()
            .trailer_entry(b"Encrypt")
            .is_none()
    );
    assert_eq!(validate(&mut engine, &mut session, bytes)["ok"], true);
    let ticket = reply["ticket"].clone();
    dispatch(
        &mut engine,
        &mut session,
        json!({"op": "security-set", "mode": "none"}),
    );
    dispatch(
        &mut engine,
        &mut session,
        json!({"op": "saved", "ticket": ticket}),
    );
    assert!(
        session.security_dirty && session.password.is_some(),
        "a later setting is still pending"
    );
    let saved = dispatch(&mut engine, &mut session, json!({"op": "saved"}));
    assert_eq!(saved["dirty"], false);
    assert!(
        session.password.is_none(),
        "removing encryption drops the held password"
    );
    assert_eq!(session.security, Some(SecuritySetting::None));
}

#[test]
fn save_rewrites_every_fixture_and_validate_accepts_it() {
    for name in [
        "hello.pdf",
        "form.pdf",
        "form2.pdf",
        "large.pdf",
        "rotated.pdf",
        "cjk.pdf",
        "rtl.pdf",
        "nav.pdf",
        "layers.pdf",
        "redact.pdf",
        "edit-tj.pdf",
    ] {
        let (mut engine, mut session) = session_with(fixture(name));
        let (reply, bytes) = save(&mut engine, &mut session);
        assert_eq!(reply["ok"], true, "{name}: {reply}");
        let validated = validate(&mut engine, &mut session, bytes.unwrap());
        assert_eq!(validated["ok"], true, "{name}: {validated}");
    }
}

#[test]
fn validate_refuses_in_engine_order_without_touching_the_session() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let reply = dispatch(&mut engine, &mut session, json!({"op": "validate"}));
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (&json!("no-document"), &json!("validate: no bytes"))
    );
    let reply = validate(&mut engine, &mut session, vec![0; 3]);
    assert_eq!(
        (&reply["reason"], &reply["error"]),
        (&json!("no-document"), &json!("validate: no document open"))
    );

    open(&mut engine, &mut session, pages_pdf(2));
    let before = serialized(&session);
    for (request, buffers, reason, message) in [
        (
            json!({"op": "validate", "pdf": 0}),
            vec![],
            "no-document",
            "validate: no bytes",
        ),
        (
            json!({"op": "validate", "pdf": "x"}),
            vec![],
            "invalid",
            "validate: expected an ArrayBuffer",
        ),
        (
            json!({"op": "validate", "pdf": {"$buffer": 0}}),
            vec![vec![0; 3]],
            "invalid",
            "validate: not a PDF (too few bytes)",
        ),
        (
            json!({"op": "validate", "pdf": {"$buffer": 0}}),
            vec![vec![0; MAX_SAVE_BYTES + 1]],
            "too-large",
            "validate: PDF exceeds the 52428800 byte save ceiling",
        ),
        (
            json!({"op": "validate", "pdf": {"$buffer": 0}}),
            vec![vec![7; 64]],
            "invalid",
            "validate: the saved copy needed repair",
        ),
        (
            json!({"op": "validate", "pdf": {"$buffer": 0}}),
            vec![pages_pdf(3)],
            "invalid",
            "validate: the saved copy has 3 pages, expected 2",
        ),
        (
            json!({"op": "validate", "pdf": {"$buffer": 0}}),
            vec![fixture("encrypted.pdf")],
            "invalid",
            "validate: the saved copy did not open with its own password",
        ),
    ] {
        let (reply, _) = dispatch_buffers(&mut engine, &mut session, request.clone(), buffers);
        assert_eq!(
            (&reply["reason"], &reply["error"]),
            (&json!(reason), &json!(message)),
            "{request}"
        );
    }
    let mut objects = page_objects(2, "");
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R /MarkInfo << /Marked true >> >>".to_vec();
    let reply = validate(&mut engine, &mut session, classic(&objects, ""));
    assert_eq!(
        reply["error"],
        "validate: the saved copy changed the document's declared features"
    );
    assert_eq!(serialized(&session), before);
    assert_eq!(session.revision, 0);

    // The copy opens with the session's password and must keep its permissions.
    let (mut engine, mut session) = session_with(fixture("encrypted-noprint.pdf"));
    dispatch(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "pdf4all-user"}),
    );
    assert_eq!(
        validate(&mut engine, &mut session, fixture("encrypted-noprint.pdf"))["ok"],
        true
    );
    let reply = validate(&mut engine, &mut session, fixture("encrypted.pdf"));
    assert_eq!(
        reply["error"],
        "validate: the saved copy has different permissions than the source"
    );
    dispatch(&mut engine, &mut session, json!({"op": "close"}));
    open(&mut engine, &mut session, fixture("encrypted.pdf"));
    dispatch(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "pdf4all-owner"}),
    );
    dispatch(
        &mut engine,
        &mut session,
        json!({"op": "security-set", "mode": "aes-256",
        "userPassword": "pdf4all-user", "permissions": {"print": true}}),
    );
    let reply = validate(&mut engine, &mut session, fixture("encrypted-noprint.pdf"));
    assert_eq!(
        reply["error"],
        "validate: the saved copy's print permission does not match what was requested"
    );
    dispatch(
        &mut engine,
        &mut session,
        json!({"op": "security-set", "mode": "none"}),
    );
    let reply = validate(&mut engine, &mut session, fixture("encrypted.pdf"));
    assert_eq!(
        reply["error"],
        "validate: the saved copy changed the document's declared features"
    );
}

#[test]
fn page_ids_never_refuse() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    assert_eq!(
        dispatch(&mut engine, &mut session, json!({"op": "page-ids"})),
        json!({"ok": true, "ids": []})
    );
    open(&mut engine, &mut session, fixture("hello.pdf"));
    assert_eq!(
        dispatch(&mut engine, &mut session, json!({"op": "page-ids"}))["ids"],
        json!([3])
    );
    open(&mut engine, &mut session, fixture("encrypted.pdf"));
    assert_eq!(
        dispatch(&mut engine, &mut session, json!({"op": "page-ids"}))["ids"],
        json!([4])
    );
}

// ---------------------------------------------------------------- review fixes

#[test]
fn one_object_may_hold_the_whole_value_budget() {
    // A flat /Kids of 120,000 pages: 360,000 tokens and 120,000 values in
    // one array, past the default per-object parser limits.
    let n = 120_000;
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + i)).collect();
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {n} /MediaBox [0 0 612 792] >>",
            kids.join(" ")
        )
        .into_bytes(),
    ];
    objects.extend((0..n).map(|_| b"<< /Type /Page /Parent 2 0 R >>".to_vec()));
    let (mut engine, mut session) = session_with(classic(&objects, ""));
    let info = session.info().unwrap();
    assert_eq!((session.page_state().count(), info.repaired), (n, false));
    let (reply, bytes) = save(&mut engine, &mut session);
    assert_eq!(reply["ok"], true, "{reply}");
    let checked = validate(&mut engine, &mut session, bytes.unwrap());
    assert_eq!(checked["pageCount"], n, "{checked}");
}

#[test]
fn allocation_never_reuses_a_number() {
    // /Size 10 with objects 1-4 and a dangling /Outlines 6 0 R: MuPDF
    // allocates from /Size, so the reference stays dangling.
    let objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".to_vec(),
        stream("", b"0 0 m"),
    ];
    let text = String::from_utf8(classic(&objects, "")).unwrap();
    let bytes = text.replace("/Size 5 ", "/Size 10 ").into_bytes();
    let (_, mut session) = session_with(bytes);
    let ids = session
        .mutate("add", |s| {
            let mut tx = s.tx()?;
            let a = tx.allocate(Object::Integer(42), None)?;
            let b = tx.allocate_stream_raw(Dictionary::new(), b"raw".to_vec())?;
            let c = tx.allocate(
                Object::dictionary([(b"N".to_vec(), Object::Integer(1))]),
                Some(b"decoded".to_vec()),
            )?;
            Ok([a, b, c])
        })
        .unwrap()
        .unwrap();
    assert_eq!(ids, [id(10), id(11), id(12)]);
    let doc = session.doc.as_ref().unwrap().document();
    assert!(
        doc.object(id(6)).is_err(),
        "the dangling reference stays dangling"
    );

    // A deleted number is not reissued, nor one an undo or abandon took back.
    let (_, mut session) = session_with(pages_pdf(2));
    let added = session
        .mutate("delete then add", |s| {
            let mut tx = s.tx()?;
            tx.remove_object(id(7))?;
            Ok(tx.allocate(Object::Integer(1), None)?)
        })
        .unwrap();
    assert_eq!(added, Some(id(8)), "not the removed 7");
    session.doc.as_mut().unwrap().undo().unwrap();
    let _ = session.mutate("abandoned", |s| {
        s.tx()?.allocate(Object::Integer(2), None)?;
        Err::<(), _>(AppError::plain("no"))
    });
    let next = session
        .mutate("add", |s| Ok(s.tx()?.allocate(Object::Integer(3), None)?))
        .unwrap();
    assert_eq!(next, Some(id(10)), "8 went back by undo, 9 by abandon");
    let refused = session.mutate("bad stream", |s| {
        Ok(s.tx()?.allocate(Object::Integer(1), Some(Vec::new()))?)
    });
    assert!(refused.is_err(), "a stream value must be a dictionary");

    // Many allocations on a large document stay linear.
    let (_, mut session) = session_with(pages_pdf(25_000));
    session
        .mutate("bulk", |s| {
            let mut tx = s.tx()?;
            for i in 0..10_000 {
                tx.allocate(Object::Integer(i), None)?;
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(
        session.doc.as_ref().unwrap().document().object_count(),
        3 + 50_000 + 10_000
    );
}

#[test]
fn the_view_reads_a_removed_object_as_null() {
    for bytes in [pages_pdf(1), xref_stream_pdf(1)] {
        let (_, mut session) = session_with(bytes);
        assert_eq!(page_text(&mut session, 0).trim(), "Page 0");
        let content = content_of(&session, 0);
        session
            .mutate("remove", |s| Ok(s.tx()?.remove_object(content)?))
            .unwrap();
        assert_eq!(page_text(&mut session, 0).trim(), "", "no content left");
        let snapshot = session.snapshot_bytes().unwrap();
        let reread = Document::open(&snapshot, Default::default()).unwrap();
        assert!(reread.object(content).is_err(), "MuPDF's copy sees it free");
        session.doc.as_mut().unwrap().undo().unwrap();
        assert_eq!(page_text(&mut session, 0).trim(), "Page 0");
    }
}

#[test]
fn keep_keeps_what_the_last_acknowledged_save_wrote() {
    // An encrypted source saved without encryption: keep stays plaintext.
    let (mut engine, mut session) = session_with(fixture("encrypted.pdf"));
    let auth = json!({"op": "authenticate", "password": "pdf4all-owner"});
    assert_eq!(dispatch(&mut engine, &mut session, auth)["auth"], "owner");
    dispatch(
        &mut engine,
        &mut session,
        json!({"op": "security-set", "mode": "none"}),
    );
    let (reply, _) = save(&mut engine, &mut session);
    assert_eq!(reply["ok"], true, "{reply}");
    let ticket = reply["ticket"].clone();
    dispatch(
        &mut engine,
        &mut session,
        json!({"op": "saved", "ticket": ticket}),
    );
    let kept = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "security-set", "mode": "keep"}),
    );
    assert_eq!(
        kept["security"],
        json!({"mode": "keep", "algorithm": "none", "auth": "owner", "pending": true,
            "permissions": ["print", "edit", "copy", "annotate", "form"]})
    );
    let (reply, bytes) = save(&mut engine, &mut session);
    assert_eq!(reply["ok"], true, "{reply}");
    let bytes = bytes.unwrap();
    assert!(
        Document::open(&bytes, Default::default())
            .unwrap()
            .security()
            .is_none()
    );
    assert_eq!(validate(&mut engine, &mut session, bytes)["ok"], true);

    // A plain source after an acknowledged AES-256 save: keep is AES-256
    // with the permissions written, never plaintext.
    let (mut engine, mut session) = session_with(fixture("hello.pdf"));
    let set = json!({"op": "security-set", "mode": "aes-256", "userPassword": "u",
        "permissions": {"print": true}});
    dispatch(&mut engine, &mut session, set);
    dispatch(&mut engine, &mut session, json!({"op": "saved"}));
    let kept = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "security-set", "mode": "keep"}),
    );
    assert_eq!(
        (
            &kept["security"]["algorithm"],
            &kept["security"]["permissions"]
        ),
        (&json!("aes-256"), &json!(["print"]))
    );
    let (reply, bytes) = save_with_entropy(&mut engine, &mut session);
    assert_eq!(reply["ok"], true, "{reply}");
    let bytes = bytes.unwrap();
    let reread = Document::open_with_password(&bytes, Default::default(), b"u").unwrap();
    let security = reread.security().unwrap();
    // Print (4), plus bit 10 (0x200) that ISO 32000-2 writers always set.
    assert_eq!(
        (security.revision, security.permissions & 0xF3C),
        (6, 4 | 0x200)
    );
    // A new document starts from its own security.
    open(&mut engine, &mut session, fixture("hello.pdf"));
    assert_eq!(session.security_state()["algorithm"], "none");
}

/// ISO 32000-2 Algorithm 2.B, for R6 test files.
fn hash_r6(password: &[u8], salt: &[u8], extra: &[u8]) -> Vec<u8> {
    use aes::Aes128;
    use aes::cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
    use sha2::{Digest, Sha256, Sha384, Sha512};
    let mut k = Sha256::new()
        .chain_update(password)
        .chain_update(salt)
        .chain_update(extra)
        .finalize()
        .to_vec();
    let mut round = 0usize;
    loop {
        let k1 = [password, &k, extra].concat().repeat(64);
        let cipher = Aes128::new(GenericArray::from_slice(&k[..16]));
        let mut previous: [u8; 16] = k[16..32].try_into().unwrap();
        let mut e = Vec::new();
        for chunk in k1.chunks_exact(16) {
            let mut block = GenericArray::clone_from_slice(chunk);
            block.iter_mut().zip(previous).for_each(|(a, b)| *a ^= b);
            cipher.encrypt_block(&mut block);
            previous.copy_from_slice(&block);
            e.extend_from_slice(&block);
        }
        k = match e[..16].iter().map(|&b| u32::from(b)).sum::<u32>() % 3 {
            0 => Sha256::digest(&e).to_vec(),
            1 => Sha384::digest(&e).to_vec(),
            _ => Sha512::digest(&e).to_vec(),
        };
        round += 1;
        if round >= 64 && usize::from(*e.last().unwrap()) <= round - 32 {
            break;
        }
    }
    k.truncate(32);
    k
}

fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: Vec<u8> = (0..=255).collect();
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|byte| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            byte ^ s[s[i as usize].wrapping_add(s[j as usize]) as usize]
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// A one-page document with no strings or streams, encrypted with the
/// standard handler at `revision` (3: RC4-128, 4: AES-128, 6: AES-256) for
/// these exact password bytes.
fn encrypted_pdf(revision: u8, user: &[u8], owner: &[u8]) -> Vec<u8> {
    use md5::{Digest, Md5};
    const PAD: [u8; 32] = [
        0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01,
        0x08, 0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53,
        0x69, 0x7A,
    ];
    let padded = |password: &[u8]| {
        let mut out = password[..password.len().min(32)].to_vec();
        out.extend_from_slice(&PAD[..32 - out.len()]);
        out
    };
    let first_id = b"0123456789abcdef";
    let p: i32 = -4;
    let encrypt = if revision == 6 {
        let (salts_u, salts_o) = (b"uvalsalt" as &[u8], b"ovalsalt" as &[u8]);
        let mut u = hash_r6(user, salts_u, &[]);
        u.extend_from_slice(salts_u);
        u.extend_from_slice(b"ukeysalt");
        let mut o = hash_r6(owner, salts_o, &u);
        o.extend_from_slice(salts_o);
        o.extend_from_slice(b"okeysalt");
        format!(
            "<< /Filter /Standard /V 5 /R 6 /Length 256 /P {p} /StmF /StdCF /StrF /StdCF \
             /CF << /StdCF << /CFM /AESV3 /Length 32 >> >> /O <{}> /U <{}> /OE <{}> /UE <{}> \
             /Perms <{}> >>",
            hex(&o),
            hex(&u),
            hex(&[7; 32]),
            hex(&[9; 32]),
            hex(&[1; 16])
        )
    } else {
        let n = 16;
        let mut owner_key = Md5::digest(padded(owner)).to_vec();
        for _ in 0..50 {
            owner_key = Md5::digest(&owner_key).to_vec();
        }
        let mut o = padded(user);
        for i in 0..20u8 {
            let round: Vec<u8> = owner_key[..n].iter().map(|b| b ^ i).collect();
            o = rc4(&round, &o);
        }
        let mut md5 = Md5::new();
        md5.update(padded(user));
        md5.update(&o);
        md5.update((p as u32).to_le_bytes());
        md5.update(first_id);
        let mut key = md5.finalize().to_vec();
        for _ in 0..50 {
            key = Md5::digest(&key[..n]).to_vec();
        }
        let mut u = Md5::new()
            .chain_update(PAD)
            .chain_update(first_id)
            .finalize()
            .to_vec();
        for i in 0..20u8 {
            let round: Vec<u8> = key[..n].iter().map(|b| b ^ i).collect();
            u = rc4(&round, &u);
        }
        u.extend_from_slice(&[0; 16]);
        let filters = if revision == 4 {
            "/V 4 /StmF /StdCF /StrF /StdCF /CF << /StdCF << /CFM /AESV2 /Length 16 >> >>"
        } else {
            "/V 2"
        };
        format!(
            "<< /Filter /Standard {filters} /R {revision} /Length 128 /P {p} /O <{}> /U <{}> >>",
            hex(&o),
            hex(&u)
        )
    };
    let objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".to_vec(),
        encrypt.into_bytes(),
    ];
    let id = hex(first_id);
    classic(&objects, &format!("/Encrypt 4 0 R /ID [<{id}> <{id}>]"))
}

fn authenticate(engine: &mut AppEngine, bytes: &[u8], password: &str) -> Value {
    let mut session = AppSession::default();
    open(engine, &mut session, bytes.to_vec());
    let request = json!({"op": "authenticate", "password": password});
    let reply = dispatch(engine, &mut session, request);
    match reply["authenticated"].as_bool() {
        Some(true) => reply["auth"].clone(),
        _ => json!(false),
    }
}

#[test]
fn passwords_are_encoded_for_the_handler_revision() {
    let (user, owner) = ("pässwörd ﬁ€", "ownér");
    let mut engine = AppEngine::new();
    for revision in [3, 4, 6] {
        // MuPDF's bytes: PDFDocEncoding for R2-R4, UTF-8 for R5/R6.
        let encode = |text: &str| match revision {
            6 => text.as_bytes().to_vec(),
            _ => tandem_pdf_engine::app::password::pdf_doc_bytes(text),
        };
        let bytes = encrypted_pdf(revision, &encode(user), &encode(owner));
        let (_, session) = session_with(bytes.clone());
        assert!(session.locked, "R{revision}");
        assert_eq!(authenticate(&mut engine, &bytes, "wrong"), false);
        assert_eq!(
            authenticate(&mut engine, &bytes, user),
            "user",
            "R{revision}"
        );
        assert_eq!(
            authenticate(&mut engine, &bytes, owner),
            "owner",
            "R{revision}"
        );
        assert_eq!(
            authenticate(&mut engine, &bytes, &format!("{user}\0tail")),
            "user",
            "R{revision}: a password ends at NUL"
        );

        // Another producer's encoding opens too.
        let other = match revision {
            6 => tandem_pdf_engine::app::password::pdf_doc_bytes(user),
            _ => user.as_bytes().to_vec(),
        };
        let bytes = encrypted_pdf(revision, &other, b"o");
        assert_eq!(
            authenticate(&mut engine, &bytes, user),
            "user",
            "R{revision}"
        );

        // validate reopens a copy with the password that unlocked the source.
        let bytes = encrypted_pdf(revision, &encode(user), &encode(owner));
        let mut session = AppSession::default();
        open(&mut engine, &mut session, bytes.clone());
        dispatch(
            &mut engine,
            &mut session,
            json!({"op": "authenticate", "password": user}),
        );
        let checked = validate(&mut engine, &mut session, bytes);
        assert_eq!(checked["ok"], true, "R{revision}: {checked}");
    }
}

#[test]
fn an_empty_owner_password_does_not_open_a_user_password_document() {
    let mut engine = AppEngine::new();
    for revision in [3, 4, 6] {
        let bytes = encrypted_pdf(revision, b"u", b"");
        let mut session = AppSession::default();
        let reply = open(&mut engine, &mut session, bytes.clone());
        assert_eq!(reply["info"]["needsPassword"], true, "R{revision}");
        assert_eq!(reply["info"]["permissions"]["edit"], true);
        let undo = dispatch(&mut engine, &mut session, json!({"op": "undo"}));
        assert_eq!(
            undo["error"],
            "undo: this document is read-only until it is unlocked"
        );
        let set = dispatch(
            &mut engine,
            &mut session,
            json!({"op": "security-set", "mode": "none"}),
        );
        assert_eq!(set["error"], "security-set: the document is locked");
        assert_eq!(authenticate(&mut engine, &bytes, ""), false, "R{revision}");
        assert_eq!(
            authenticate(&mut engine, &bytes, "u"),
            "user",
            "R{revision}"
        );

        // Both empty: the empty password is the owner's, as in MuPDF.
        let bytes = encrypted_pdf(revision, b"", b"");
        let (_, session) = session_with(bytes);
        assert!(!session.locked, "R{revision}");
        let security = session.doc.as_ref().unwrap().document().security();
        assert!(security.unwrap().owner, "R{revision}");
    }
}

/// `n` incremental updates of `pages_pdf(1)`, each rewriting the catalog.
fn many_revisions(n: usize) -> Vec<u8> {
    let mut bytes = pages_pdf(1);
    let tail = |bytes: &[u8]| -> usize {
        let text = String::from_utf8_lossy(&bytes[bytes.len() - 32..]).into_owned();
        text.rsplit("startxref\n")
            .next()
            .and_then(|rest| rest.lines().next())
            .and_then(|line| line.trim().parse().ok())
            .unwrap()
    };
    for _ in 0..n {
        let prev = tail(&bytes);
        let at = bytes.len();
        bytes.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        let xref = bytes.len();
        bytes.extend_from_slice(
            format!(
                "xref\n1 1\n{at:010} 00000 n \ntrailer\n<< /Size 6 /Root 1 0 R /Prev {prev} >>\n\
                 startxref\n{xref}\n%%EOF\n"
            )
            .as_bytes(),
        );
    }
    bytes
}

#[test]
fn many_revisions_open_and_past_the_cap_are_rebuilt() {
    let (_, session) = session_with(many_revisions(600));
    let info = session.info().unwrap();
    assert_eq!((info.versions, info.repaired), (600, false));

    let (_, mut session) = session_with(many_revisions(70_000));
    let info = session.info().unwrap();
    assert_eq!((info.versions, info.repaired), (0, true));
    assert_eq!(page_text(&mut session, 0).trim(), "Page 0");
}

#[test]
fn junk_before_the_header_is_skipped_without_repair() {
    let junk = b"\x00garbage".to_vec();
    for bytes in [
        pages_pdf(2),
        xref_stream_pdf(2),
        incremental_pdf(2),
        fixture("large.pdf"),
    ] {
        let shifted = [junk.as_slice(), &bytes].concat();
        let (_, mut session) = session_with(shifted);
        let info = session.info().unwrap();
        assert!(!info.repaired);
        assert_eq!(
            session.page_state().json(),
            AppDocument::load(Arc::new(bytes), b"")
                .unwrap()
                .page_state()
                .json()
        );
        assert!(!page_text(&mut session, 0).trim().is_empty());
    }
    let encrypted = [junk.as_slice(), &encrypted_pdf(4, b"u", b"o")].concat();
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let reply = open(&mut engine, &mut session, encrypted.clone());
    assert_eq!(
        (&reply["info"]["needsPassword"], &reply["info"]["repaired"]),
        (&json!(true), &json!(false))
    );
    assert_eq!(authenticate(&mut engine, &encrypted, "u"), "user");

    // Offsets that count the junk are wrong once it is skipped, as they are
    // for MuPDF's bias: the file is repaired.
    let mut absolute = junk.clone();
    let mut startxref = false;
    for line in String::from_utf8(pages_pdf(1))
        .unwrap()
        .split_inclusive('\n')
    {
        let line = if startxref {
            format!("{}\n", line.trim().parse::<usize>().unwrap() + junk.len())
        } else if line.len() == 20 && line.ends_with(" n \n") {
            let offset: usize = line[..10].parse().unwrap();
            format!("{:010}{}", offset + junk.len(), &line[10..])
        } else {
            line.to_string()
        };
        startxref = line == "startxref\n";
        absolute.extend_from_slice(line.as_bytes());
    }
    let (_, mut session) = session_with(absolute);
    assert!(session.info().unwrap().repaired);
    assert_eq!(page_text(&mut session, 0).trim(), "Page 0");
}

// ---------------------------------------------------------------- measurements

/// `cargo test --release --test app_core -- --ignored --nocapture`.
#[test]
#[ignore = "measurement"]
fn measure_open_edit_view_on_large_documents() {
    use std::time::Instant;
    let big = {
        // ~100 MiB: 400 pages, each with a 256 KiB image-like stream.
        let mut objects = page_objects(400, "");
        for i in 0..400 {
            let blob: Vec<u8> = (0..256 * 1024u32)
                .map(|b| (b.wrapping_mul(31) ^ i) as u8)
                .collect();
            objects.push(stream("/Kind /Blob", &blob));
        }
        let mut catalog = b"<< /Type /Catalog /Pages 2 0 R /Blobs [".to_vec();
        for i in 0..400 {
            catalog.extend_from_slice(format!("{} 0 R ", 804 + i).as_bytes());
        }
        catalog.extend_from_slice(b"] >>");
        objects[0] = catalog;
        classic(&objects, "")
    };
    for (label, bytes) in [("large.pdf", fixture("large.pdf")), ("generated", big)] {
        let size = bytes.len();
        let started = Instant::now();
        let (_, mut session) = session_with(bytes);
        let opened = started.elapsed();
        let started = Instant::now();
        session.view().unwrap();
        let first_view = started.elapsed();
        let started = Instant::now();
        let state = session.page_state();
        let pages = started.elapsed();
        let started = Instant::now();
        session
            .mutate("edit", |s| replace_text(s, 0, "Edited"))
            .unwrap();
        let edit = started.elapsed();
        let started = Instant::now();
        let text = session
            .view()
            .unwrap()
            .structured_text(0)
            .unwrap()
            .as_text();
        let edited_view = started.elapsed();
        let started = Instant::now();
        session.doc.as_mut().unwrap().undo().unwrap();
        session.view().unwrap();
        let undo_view = started.elapsed();
        eprintln!(
            "{label}: {size} bytes, {} pages; open {opened:?}, first view {first_view:?}, \
             page state {pages:?}, edit {edit:?}, view after edit {edited_view:?} ({}), \
             undo + view {undo_view:?}",
            state.count(),
            text.trim()
        );
    }
}

/// A NUL in an AES-256 password is refused like a comma: readers end the
/// password there, the writer hashes every byte, so no save would open with
/// it (engine/design/app-accept-session.md).
#[test]
fn security_set_refuses_a_nul_in_a_password() {
    let (mut engine, mut session) = session_with(fixture("hello.pdf"));
    for (request, message) in [
        (
            json!({"op": "security-set", "mode": "aes-256", "userPassword": "ab\u{0}cd"}),
            "security-set: user password cannot contain a NUL character",
        ),
        (
            json!({"op": "security-set", "mode": "aes-256", "userPassword": "abcd", "ownerPassword": "o\u{0}"}),
            "security-set: owner password cannot contain a NUL character",
        ),
    ] {
        let reply = dispatch(&mut engine, &mut session, request);
        assert_eq!(reply["error"], message, "{reply}");
    }
    let state = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "security-set", "mode": "keep"}),
    );
    assert_eq!(state["ok"], true, "{state}");
}

/// The wrong-password check against a strictly opened locked document
/// agrees with the reload it saves, for every candidate encoding.
#[test]
fn wrong_passwords_are_known_without_a_reload() {
    use tandem_pdf_engine::app::password;
    for (name, user, owner) in [
        ("encrypted.pdf", "pdf4all-user", "pdf4all-owner"),
        ("encrypted-aes256.pdf", "pdf4all-user", "pdf4all-owner"),
    ] {
        let doc = AppDocument::load(Arc::new(fixture(name)), b"").unwrap();
        assert!(doc.is_locked() && doc.opened_strictly(), "{name}");
        let revision = password::revision(doc.document());
        for (text, wrong) in [
            ("nope", true),
            ("", true),
            (user, false),
            (owner, false),
            ("pdf4all-user\u{0}x", false),
        ] {
            let candidates = password::candidates(text, revision);
            let candidates: Vec<&[u8]> = candidates.iter().map(Vec::as_slice).collect();
            assert_eq!(
                password::all_wrong(doc.document(), &candidates),
                wrong,
                "{name} {text:?}"
            );
            let reloaded = AppDocument::load_any(Arc::new(fixture(name)), &candidates).unwrap();
            assert_eq!(reloaded.is_locked(), wrong, "{name} {text:?}");
        }
        assert!(!password::all_wrong(doc.document(), &[]));
    }
    let plain = AppDocument::load(Arc::new(fixture("hello.pdf")), b"").unwrap();
    assert!(!password::all_wrong(plain.document(), &[b"x".as_slice()]));
    // Through the op: still locked after a wrong password, unlocked after the right one.
    let (mut engine, mut session) = session_with(fixture("encrypted-aes256.pdf"));
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "nope"}),
    );
    assert_eq!(reply["authenticated"], false, "{reply}");
    assert!(
        reply.get("pageCount").is_none() && reply.get("auth").is_none(),
        "{reply}"
    );
    let reply = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "pdf4all-owner"}),
    );
    assert_eq!(reply["authenticated"], true, "{reply}");
    assert_eq!(reply["auth"], "owner", "{reply}");
}
