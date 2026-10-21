// SPDX-License-Identifier: MIT
use serde_json::{Value, json};
use tandem_pdf_engine::app::{
    AppDocument, AppEngine, AppError, AppSession, Auth, Batch, Buffers, Clipboard, CompareDocument,
    SecuritySetting, error_reply,
};

/// An open document at journal position 0 (the placeholder's state).
fn document() -> AppDocument {
    let bytes = std::fs::read("../fixtures/hello.pdf").unwrap();
    AppDocument::load(bytes.into(), b"").unwrap()
}

/// A compare slot holding hello.pdf.
fn compare_slot() -> CompareDocument {
    let bytes = std::fs::read("../fixtures/hello.pdf").unwrap();
    CompareDocument::open("b.pdf", bytes).unwrap()
}

fn dispatch(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    let (reply, outputs) = engine.dispatch(session, request, Vec::new());
    assert!(outputs.is_empty());
    reply
}

#[test]
fn close_resets_the_document_fields_and_bumps_the_generation() {
    let mut engine = AppEngine::new();
    let mut session = AppSession {
        doc: Some(document()),
        name: Some("a.pdf".into()),
        source_bytes: 10,
        locked: true,
        auth: Auth::Owner,
        password: Some("pw".into()),
        locked_bytes: Some(vec![1].into()),
        signed_source: Some(vec![2]),
        security: Some(SecuritySetting::None),
        security_dirty: true,
        security_version: 2,
        saved_position: -1,
        revision: 7,
        generation: 3,
        clipboard: Some(Clipboard),
        compare: Some(compare_slot()),
        batch: Batch::Plan,
    };
    assert_eq!(
        dispatch(&mut engine, &mut session, json!({"id": 4, "op": "close"})),
        json!({"id": 4, "ok": true})
    );
    assert!(session.doc.is_none() && session.name.is_none() && session.password.is_none());
    assert!(session.locked_bytes.is_none() && session.signed_source.is_none());
    assert!(session.security.is_none() && !session.security_dirty && !session.locked);
    assert_eq!(
        (
            session.auth,
            session.source_bytes,
            session.revision,
            session.saved_position
        ),
        (Auth::None, 0, 0, 0)
    );
    assert_eq!(
        session.generation, 1,
        "the engine counter, not the session's"
    );
    assert!(session.clipboard.is_some() && session.compare.is_some());
    assert_eq!(session.batch, Batch::Plan);

    // The counter is engine-wide: another session's close takes the next value.
    let mut other = AppSession::default();
    dispatch(&mut engine, &mut other, json!({"id": 5, "op": "close"}));
    dispatch(&mut engine, &mut session, json!({"op": "close"}));
    assert_eq!((other.generation, session.generation), (2, 3));
}

#[test]
fn close_without_an_id_replies_without_one() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    assert_eq!(
        dispatch(&mut engine, &mut session, json!({"op": "close"})),
        json!({"ok": true})
    );
}

#[test]
fn unknown_ops_are_plain_errors_with_the_journal_slice() {
    let mut engine = AppEngine::new();
    let mut session = AppSession {
        revision: 2,
        ..AppSession::default()
    };
    for (request, error) in [
        (json!({"id": 1, "op": "nope"}), "unknown op: nope"),
        (json!({"id": 1}), "unknown op: undefined"),
        (json!({"id": 1, "op": null}), "unknown op: null"),
        (json!({"id": 1, "op": 1.5}), "unknown op: 1.5"),
        (json!({"id": 1, "op": ["close"]}), "unknown op: close"),
        (json!({"id": 1, "op": {}}), "unknown op: [object Object]"),
        (json!({"id": 1, "op": "Close"}), "unknown op: Close"),
    ] {
        assert_eq!(
            dispatch(&mut engine, &mut session, request),
            json!({"id": 1, "ok": false, "error": error, "revision": 2, "dirty": false,
                "canUndo": false, "canRedo": false})
        );
    }
    // A request that is not an object has no id and no op.
    for request in [json!(null), json!("close"), json!([{"op": "close"}])] {
        assert_eq!(
            dispatch(&mut engine, &mut session, request),
            json!({"ok": false, "error": "unknown op: undefined", "revision": 2,
                "dirty": false, "canUndo": false, "canRedo": false})
        );
    }
    assert_eq!(session.generation, 0, "a failed dispatch changes nothing");
}

#[test]
fn ids_are_echoed_as_sent() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    for id in [json!(null), json!("a"), json!(1.5), json!({"x": 1})] {
        let reply = dispatch(&mut engine, &mut session, json!({"id": id, "op": "?"}));
        assert_eq!(reply["id"], id);
    }
}

#[test]
fn tagged_errors_carry_reason_detail_and_extra_under_the_journal_slice() {
    let mut session = AppSession {
        doc: Some(document()),
        revision: 9,
        saved_position: -1,
        ..AppSession::default()
    };
    let request = json!({"id": 3, "op": "x"});
    assert_eq!(
        error_reply(&request, AppError::plain("boom"), &session),
        json!({"id": 3, "ok": false, "error": "boom", "revision": 9, "dirty": true,
            "canUndo": false, "canRedo": false})
    );
    assert_eq!(
        error_reply(&request, AppError::reason("read-only", "x: no"), &session),
        json!({"id": 3, "ok": false, "error": "x: no", "reason": "read-only", "revision": 9,
            "dirty": true, "canUndo": false, "canRedo": false})
    );
    let error = AppError::detail("invalid", "glyph", "x: bad")
        .with_extra(json!({"count": 2, "revision": 100, "error": "overridden"}))
        .with_extra(json!({"count": 3}))
        .with_extra(json!(5));
    assert_eq!(
        error_reply(&request, error, &session),
        json!({"id": 3, "ok": false, "error": "overridden", "reason": "invalid",
            "detail": "glyph", "count": 3, "revision": 9, "dirty": true,
            "canUndo": false, "canRedo": false})
    );
    // An empty reason is falsy in the earlier app engine and is left out.
    assert_eq!(
        error_reply(&request, AppError::detail("", "", "m"), &session)
            .as_object()
            .map(|reply| reply.contains_key("reason") || reply.contains_key("detail")),
        Some(false)
    );

    // Dirty tracks the saved position and the security flag, only with a document.
    session.saved_position = 0;
    assert!(!session.journal_state().dirty);
    session.security_dirty = true;
    assert!(session.journal_state().dirty);
    session.doc = None;
    assert!(!session.journal_state().dirty);
}

#[test]
fn state_lists_the_adapter_fields() {
    let session = AppSession {
        name: Some("n.pdf".into()),
        source_bytes: 12,
        locked: true,
        auth: Auth::User,
        saved_position: -1,
        revision: 4,
        generation: 6,
        ..AppSession::default()
    };
    assert_eq!(
        session.state_json(),
        json!({"revision": 4, "generation": 6, "savedPosition": -1, "locked": true,
            "auth": "user", "name": "n.pdf", "sourceBytes": 12, "dirty": false,
            "canUndo": false, "canRedo": false})
    );
    assert_eq!(AppSession::default().state_json()["name"], Value::Null);
    assert_eq!(AppSession::default().snapshot_bytes(), None);
}

#[test]
fn ids_keep_the_double_javascript_sent() {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    for text in [
        "23.172468304866854",
        "961.1757480989835",
        "4497660818060879.5",
    ] {
        let request = serde_json::from_str(&format!(r#"{{"id":{text},"op":"nope"}}"#)).unwrap();
        let reply = dispatch(&mut engine, &mut session, request);
        let sent = text.parse::<f64>().unwrap();
        assert_eq!(
            reply["id"].as_f64().map(f64::to_bits),
            Some(sent.to_bits()),
            "{text}"
        );
        // Key order is serde_json's: sorted alone, insertion order in the
        // workspace, where tandem-app turns on `preserve_order`.
        assert!(
            reply.to_string().contains(&format!(r#""id":{text},"#)),
            "{reply}"
        );
        let expected: Value = serde_json::from_str(&format!(
            r#"{{"canRedo":false,"canUndo":false,"dirty":false,"error":"unknown op: nope","id":{text},"ok":false,"revision":0}}"#
        ))
        .unwrap();
        assert_eq!(reply, expected);
    }
}

#[test]
fn debug_state_lists_the_fields_tests_read() {
    let session = AppSession {
        doc: Some(document()),
        password: Some("pw".into()),
        locked_bytes: Some(vec![0; 5].into()),
        security: Some(SecuritySetting::Aes256 {
            user_password: "u".into(),
            owner_password: "o".into(),
            permissions: 4,
        }),
        security_dirty: true,
        batch: Batch::Plan,
        clipboard: Some(Clipboard),
        ..AppSession::default()
    };
    let debug = session.debug_json();
    assert_eq!(
        debug,
        json!({"revision": 0, "generation": 0, "savedPosition": 0, "locked": false,
            "auth": "none", "name": null, "sourceBytes": 0, "dirty": true, "canUndo": false,
            "canRedo": false, "password": "pw", "securityDirty": true, "lockedBytes": 5,
            "signedSource": null, "batch": "plan", "clipboard": true, "compare": null,
            "journal": {"position": 0, "steps": []},
            "security": {"mode": "aes-256", "userPassword": "u", "ownerPassword": "o",
                "permissions": 4}})
    );
    let empty = AppSession::default().debug_json();
    assert_eq!(
        (&empty["security"], &empty["journal"], &empty["batch"]),
        (&Value::Null, &Value::Null, &json!(false))
    );
    let compared = AppSession {
        compare: Some(compare_slot()),
        ..AppSession::default()
    };
    let slot = compared.debug_json()["compare"].clone();
    let serial = slot["serial"].as_u64().unwrap();
    assert_eq!(
        slot,
        json!({"name": "b.pdf", "pageCount": 1, "sourceBytes": compared.compare.as_ref().unwrap().source_bytes(),
            "serial": serial})
    );
    assert_eq!(compared.debug_json()["clipboard"], json!(false));
}

#[test]
fn buffers_resolve_placeholders_once_and_number_outputs() {
    let mut buffers = Buffers::new(vec![b"one".to_vec(), b"two".to_vec()]);
    let first = json!({"$buffer": 1});
    assert_eq!(buffers.take(Some(&first)), Some(b"two".to_vec()));
    assert_eq!(buffers.take(Some(&first)), None, "taken once");
    assert_eq!(buffers.take(Some(&json!({"$buffer": 2}))), None);
    assert_eq!(buffers.take(Some(&json!({"$buffer": 0, "n": 1}))), None);
    assert_eq!(buffers.take(None), None);
    assert_eq!(buffers.put(b"a".to_vec()), json!({"$buffer": 0}));
    assert_eq!(buffers.put(b"b".to_vec()), json!({"$buffer": 1}));
}

#[test]
fn buffers_peek_and_echo_inputs() {
    let mut buffers = Buffers::new(vec![b"pdf".to_vec()]);
    let placeholder = json!({"$buffer": 0});
    assert_eq!(buffers.input(Some(&placeholder)), Some(&b"pdf"[..]));
    assert_eq!(buffers.input(Some(&json!({"$buffer": 1}))), None);
    assert_eq!(buffers.echo(Some(&placeholder)), Some(json!({"$input": 0})));
    assert_eq!(buffers.take(Some(&placeholder)), Some(b"pdf".to_vec()));
    assert_eq!(buffers.input(Some(&placeholder)), None, "taken");
    assert_eq!(
        buffers.echo(Some(&placeholder)),
        Some(json!({"$input": 0})),
        "the caller still holds the original"
    );
    for value in [json!({"$buffer": 1}), json!({"$$buffer": 0}), json!(0)] {
        assert_eq!(buffers.echo(Some(&value)), None, "{value}");
    }
    assert_eq!(buffers.echo(None), None);
}
