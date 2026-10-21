//! M9 recovery and save tickets on the in-process engine: ports of tests/m9-recovery.test.mjs,
//! tests/save-ticket.test.mjs and the restore paths of tests/m9-recovery-drive.test.mjs.
//!
//! The Electron app restarted a hung or trapped WASM worker. Here the engine runs in process:
//! no request can be stopped from outside (no watchdog, so no `slow` restore), and the only
//! restart follows a caught engine panic, which the engine answers with `reason: "worker"`.
//! No public hook makes the engine panic, so a trap is injected one step later, where the
//! session takes over: `lane1.trap_pending` names the request that failed (it never reached
//! the engine, as a trapped request never applied) and `run_pending_tasks` runs the session's
//! `workerFailed`: engine restart, then restore from the recovery log. A replay that fails is
//! made by changing one logged answer's fingerprint (the engine is deterministic, so only a
//! second failure could make a real replay differ).
// Some helpers serve only the Unix tests (modes, links).
#![cfg_attr(not(unix), allow(unused_imports, dead_code))]

use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use tandem_app::drive::{DriveRun, Outcome, parse_script, with_out};
use tandem_app::engine::{EngineHost, Reply};
use tandem_app::host::{Host, HostConfig};
use tandem_app::platform::{Answers, Scripted};
use tandem_app::session::Session;
use tandem_app::session::consts::{
    MAX_REPLAY_CLOSES, MAX_WORKER_RESTARTS, MOBILE_RECOVERY_BUDGET_BYTES, OPS,
    RECOVERY_BUDGET_BYTES, REPLAY_BASE_MS, SMALL_RECOVERY_BUDGET_BYTES,
};
use tandem_app::session::recovery_log::{
    fingerprint, op_info_str, recovery_budget_for, replayed, size_of,
};
use tandem_app::session::types::{Culprit, DiskWrite, OpKind, RecoveryLog, Unavailable};

// ------------------------------------------------------------------ helpers

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/../../fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// A temp folder, removed when dropped.
struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp(tag: &str) -> Tmp {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "tandem-t-recovery-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(dir.join("out")).unwrap();
    Tmp(std::fs::canonicalize(&dir).unwrap())
}

/// What a failed step prints: the STATE keys these tests read.
fn brief(state: &Value) -> String {
    json!({
        "doc": state["doc"]["name"], "notes": state["notes"], "dirty": state["dirty"],
        "canUndo": state["canUndo"], "error": state["error"], "status": state["status"],
        "recovery": state["recovery"], "save": state["save"],
    })
    .to_string()
}

/// A headless drive window (as `tandem-headless --drive`) on the startup sample hello.pdf,
/// with copies of the fixtures it opens in a temp folder. The startup sample is the engine's
/// first document, so a document the test opens is generation 2 or more (as in the app).
struct Rig {
    s: Session,
    run: DriveRun,
    dir: Tmp,
}

impl Rig {
    fn new(tag: &str) -> Rig {
        let dir = tmp(tag);
        for (name, from) in [
            ("hello.pdf", "hello.pdf"),
            ("encrypted.pdf", "encrypted.pdf"),
            ("form.pdf", "form.pdf"),
            ("edit-replace.pdf", "p3/edit-replace.pdf"),
        ] {
            std::fs::write(dir.0.join(name), fixture(from)).unwrap();
        }
        let host = Host::new(
            HostConfig {
                user_data: dir.0.join("udata"),
                smoke: true,
                echo_stdout: false,
            },
            Box::new(Scripted::new(Answers::default())),
        );
        let mut s = Session::for_drive(host);
        let out = dir.0.join("out").to_string_lossy().into_owned();
        let run = DriveRun::start(&mut s, "hello.pdf", &out).expect("the startup sample opens");
        Rig { s, run, dir }
    }

    /// A copy of a fixture (or `out/<name>`), as a drive `path`.
    fn path(&self, name: &str) -> String {
        self.dir.0.join(name).to_string_lossy().into_owned()
    }

    fn out(&self, name: &str) -> String {
        self.path(&format!("out/{name}"))
    }

    fn read_out(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.dir.0.join("out").join(name)).unwrap()
    }

    /// One drive line; its `expect` must match. Returns the STATE after it.
    fn step(&mut self, line: Value) -> Value {
        let shown = line.to_string();
        match self.run.line(&mut self.s, line) {
            Outcome::Record(record) => {
                assert_ne!(
                    record.get("ok"),
                    Some(&json!(false)),
                    "{shown}\n  got: {}",
                    brief(&record["state"])
                );
                record["state"].clone()
            }
            other => panic!("{shown}: {other:?}"),
        }
    }

    /// STATE now, checked against `expect` (a drive `wait` of 0 ms).
    fn expect(&mut self, expect: Value) -> Value {
        self.step(json!({"cmd": "wait", "ms": 0, "expect": expect}))
    }

    fn open(&mut self, name: &str) -> Value {
        let path = self.path(name);
        self.step(json!({"cmd": "open", "path": path, "expect": {"doc": {"name": name}}}))
    }

    fn note(&mut self, contents: &str) -> Value {
        self.step(json!({"cmd": "note", "contents": contents}))
    }

    /// The engine answered `reason: "worker"` (a caught panic) to `op`: the session restarts
    /// the engine and restores the document from its recovery log.
    fn trap(&mut self, op: &str, page: Option<i64>) {
        self.s.lane1.trap_pending = Some(Culprit {
            op: json!(op),
            page,
        });
        self.s.run_pending_tasks();
    }

    /// One engine request through the session (logged as the renderer logs it).
    fn ok(&mut self, message: Value) -> Reply {
        self.ok_with(message, Vec::new())
    }

    fn ok_with(&mut self, message: Value, inputs: Vec<Vec<u8>>) -> Reply {
        let shown = message.to_string();
        self.s
            .request(message, inputs)
            .unwrap_or_else(|err| panic!("{shown}: {} ({})", err.message, err.reason_str()))
    }

    /// `annot-add` of a Text note, as save-ticket.test.mjs sent it.
    fn ticket_note(&mut self, contents: &str) -> Reply {
        self.ok(
            json!({"op": "annot-add", "page": 0, "type": "Text", "point": [72, 72],
            "contents": contents, "today": "2026-10-17"}),
        )
    }

    fn ops(&self) -> Vec<String> {
        self.s
            .recovery
            .entries
            .iter()
            .map(|e| e.op.clone())
            .collect()
    }
}

/// The annotations of a saved file (type, contents, rect to 0.01), read in a fresh engine.
fn annots_of(bytes: &[u8]) -> Vec<Value> {
    let mut host = EngineHost::new();
    host.request(
        json!({"op": "open", "name": "x.pdf", "pdf": {"$buffer": 0}}),
        vec![bytes.to_vec()],
    )
    .expect("the saved file opens");
    let reply = host
        .request(json!({"op": "annots", "page": 0}), vec![])
        .expect("annots");
    let items = reply.get("items").and_then(Value::as_array).cloned();
    items
        .unwrap_or_default()
        .iter()
        .map(|item| {
            let rect: Vec<f64> = item["rect"]
                .as_array()
                .map(|r| r.iter().filter_map(Value::as_f64).collect())
                .unwrap_or_default();
            let rect: Vec<f64> = rect.iter().map(|v| (v * 100.0).round() / 100.0).collect();
            json!({"type": item["type"], "contents": item["contents"], "rect": rect})
        })
        .collect()
}

fn contents_of(bytes: &[u8]) -> Vec<String> {
    annots_of(bytes)
        .iter()
        .map(|a| a["contents"].as_str().unwrap_or("").to_string())
        .collect()
}

fn obj(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("an object")
}

/// The renderer's request bookkeeping without an engine (m9-recovery.test.mjs `tracked`):
/// keep before posting, record the answer unless it was refused (`reply` None).
fn tracked(
    log: &mut RecoveryLog,
    message: Value,
    inputs: &[Vec<u8>],
    reply: Option<Value>,
) -> bool {
    let Some(kept) = log.keep(&obj(message), inputs) else {
        return false;
    };
    if let Some(reply) = reply {
        log.record(kept, &reply, &[], 1.0);
    }
    true
}

const ENTROPY: [u8; 32] = [7; 32];

/// `save` with fixed entropy, so two engines write the same bytes.
fn save_bytes(rig: &mut Rig) -> Vec<u8> {
    let reply = rig.ok_with(
        json!({"op": "save", "entropy": {"$buffer": 0}}),
        vec![ENTROPY.to_vec()],
    );
    reply.bytes("bytes").expect("save bytes").to_vec()
}

// ------------------------------------------------------------------ m9-recovery.test.mjs

// was tests/m9-recovery.test.mjs: 'every SessionRequest op is classified, and only those'
#[test]
fn every_engine_op_is_classified_and_only_those() {
    // The engine's op table is its router (engine/src/app/ops/mod.rs `route`, the port of
    // engine.ts's switch); there is no public list, so the arms are read from the source.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../engine/src/app/ops/mod.rs"
    );
    let source = std::fs::read_to_string(path).unwrap();
    let start = source.find("fn route(").expect("the router");
    let end = start + source[start..].find("\n}\n").expect("its end");
    let mut engine_ops: Vec<String> = Vec::new();
    for line in source[start..end].lines().filter(|l| l.contains("Some(")) {
        for (i, part) in line.split('"').enumerate() {
            if i % 2 == 1 {
                engine_ops.push(part.to_string());
            }
        }
    }
    let mut names: Vec<String> = OPS.iter().map(|(op, ..)| op.to_string()).collect();
    let count = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), count, "one entry per op");
    engine_ops.sort();
    assert_eq!(engine_ops.len(), count, "{engine_ops:?}");
    assert_eq!(names, engine_ops, "OPS must list exactly the engine's ops");

    // Each is an op the engine knows (it never answers "unknown op"), and none panics on a
    // request without its fields and without a document.
    let mut host = EngineHost::new();
    for (op, kind, label, _) in OPS {
        let (reply, _) = host.dispatch(json!({"op": op}), vec![]);
        let error = reply["error"].as_str().unwrap_or("");
        assert!(!error.starts_with("unknown op"), "{op}: {error}");
        assert!(host.stopped().is_none(), "{op} stopped the engine: {error}");
        let mut chars = label.chars();
        assert!(
            chars.next().is_some_and(|c| c.is_ascii_uppercase())
                && chars.next().is_some_and(|c| c.is_ascii_lowercase()),
            "{op} has a sentence-case label: {label}"
        );
        assert_eq!(op_info_str(op).map(|(k, ..)| k), Some(kind));
    }
    let (reply, _) = host.dispatch(json!({"op": "nonsense"}), vec![]);
    assert_eq!(reply["error"], "unknown op: nonsense");

    // The classification the design note commits to.
    let kinds = |kind: OpKind| {
        let mut ops: Vec<&str> = OPS
            .iter()
            .filter(|(_, k, ..)| *k == kind)
            .map(|(op, ..)| *op)
            .collect();
        ops.sort();
        ops
    };
    assert_eq!(kinds(OpKind::Base), ["close", "open"]);
    assert_eq!(kinds(OpKind::Unlock), ["authenticate"]);
    assert_eq!(kinds(OpKind::Clipboard), ["clip-copy"]);
    for op in [
        "save",
        "validate",
        "render",
        "thumbnail",
        "extract-pages",
        "pdf-from-images",
        "scan-compose",
        "scan-measure",
        "sign-prepare",
        "batch-file",
        "text-all",
        "edit-targets",
        "edit-replace-preview",
        "redact-preview",
    ] {
        assert_eq!(op_info_str(op).map(|(k, ..)| k), Some(OpKind::Read), "{op}");
        assert!(!replayed(&json!(op)), "{op}");
    }
    for op in [
        "undo",
        "redo",
        "saved",
        "security-set",
        "layer-set",
        "redact-search",
        "redact-apply",
        "clip-paste",
        "insert-pages",
        "stamp",
    ] {
        assert_eq!(
            op_info_str(op).map(|(k, ..)| k),
            Some(OpKind::Mutate),
            "{op}"
        );
        assert!(replayed(&json!(op)), "{op}");
    }
    assert!(!replayed(&json!("nonsense")));
    assert!(!replayed(&json!(5)));
}

// was tests/m9-recovery.test.mjs: 'the budget holds a near-ceiling document, its saved copy and a log'
#[test]
fn the_budget_holds_a_near_ceiling_document_its_saved_copy_and_a_log() {
    use tandem_protocol::limits::{MAX_PDF_BYTES, MAX_SAVE_BYTES};
    assert_eq!(RECOVERY_BUDGET_BYTES, 256 * 1024 * 1024);
    assert_eq!(tandem_app::safe_write::MAX_SAVE_BYTES, MAX_SAVE_BYTES);
    const { assert!(MAX_PDF_BYTES + MAX_SAVE_BYTES < RECOVERY_BUDGET_BYTES) };
    const { assert!(RECOVERY_BUDGET_BYTES - MAX_PDF_BYTES - MAX_SAVE_BYTES >= 64 * 1024 * 1024) };
    let buffer = json!({"op": "x", "bytes": {"$buffer": 0}});
    assert!(size_of(&buffer, &[vec![0; 1000]]) >= 1000);
    let text = json!({"text": "a".repeat(500)});
    assert!(
        size_of(&text, &[]) >= 1000,
        "strings count two bytes a unit"
    );
    // (The JS cyclic-object case has no counterpart: a serde_json value cannot be cyclic.)
}

// was tests/m9-recovery.test.mjs: 'the budget scales to the device: a lower default for mobile and small desktops'
#[test]
fn the_budget_scales_to_the_device() {
    assert_eq!(recovery_budget_for(false, None), RECOVERY_BUDGET_BYTES);
    assert_eq!(recovery_budget_for(false, Some(8.0)), RECOVERY_BUDGET_BYTES);
    assert_eq!(
        recovery_budget_for(false, Some(4.0)),
        SMALL_RECOVERY_BUDGET_BYTES
    );
    assert_eq!(
        recovery_budget_for(false, Some(f64::NAN)),
        RECOVERY_BUDGET_BYTES
    );
    assert_eq!(
        recovery_budget_for(true, Some(8.0)),
        MOBILE_RECOVERY_BUDGET_BYTES
    );
    assert_eq!(MOBILE_RECOVERY_BUDGET_BYTES, 96 * 1024 * 1024);
    assert_eq!(
        RecoveryLog::new(MOBILE_RECOVERY_BUDGET_BYTES).budget,
        MOBILE_RECOVERY_BUDGET_BYTES
    );
    // The desktop session starts with the full budget (Chromium's deviceMemory cap is 8).
    assert_eq!(Session::new().recovery.budget, RECOVERY_BUDGET_BYTES);
}

// was tests/m9-recovery.test.mjs: 'a replay reports its progress and gives up past a budget scaled to the log'
// The replay's clock cannot be injected and its progress is cleared when the restore ends, so
// this checks the budget formula, then a restore past its budget (the log claims a negative
// open time, so the budget is 0 and the replay stops before its first change).
#[test]
fn a_replay_gives_up_past_a_budget_scaled_to_the_log() {
    let mut log = RecoveryLog::new(RECOVERY_BUDGET_BYTES);
    let open = log
        .keep(
            &obj(json!({"op": "open", "pdf": {"$buffer": 0}, "name": "a.pdf"})),
            &[vec![0; 10]],
        )
        .unwrap();
    log.record(open, &json!({"ok": true}), &[], 400.0);
    for i in 0..5 {
        let kept = log.keep(&obj(json!({"op": "undo"})), &[]).unwrap();
        log.record(kept, &json!({"ok": true, "revision": i + 1}), &[], 1000.0);
    }
    assert_eq!(
        log.replay_budget_ms(),
        REPLAY_BASE_MS + 2.0 * (400.0 + 5000.0)
    );

    let mut rig = Rig::new("budget-ms");
    rig.open("hello.pdf");
    for n in ["one", "two", "three"] {
        rig.note(&format!("M9 {n}"));
    }
    rig.s.recovery.opened_ms = -1e9;
    rig.trap("annot-add", Some(0));
    rig.expect(json!({
        "doc": {"name": "hello.pdf"}, "notes": 0, "dirty": false,
        "error": "Adding the annotation failed inside the PDF engine, so it was stopped. Restoring your changes took too long, so the document was reopened as it was opened.",
        "recovery": {"available": true, "entries": 0, "restoring": false, "progress": null,
            "last": {"outcome": "reopened", "dropped": "annot-add", "cause": "trap", "steps": 0, "overBudget": true}},
    }));
}

// was tests/m9-recovery.test.mjs: 'fingerprints ignore timings and the request id, and see every other field'
#[test]
fn fingerprints_ignore_timings_and_the_request_id_and_see_every_other_field() {
    let base = json!({"id": 4, "ok": true, "revision": 3, "dirty": true, "annotId": 17,
        "openMs": 12, "paintMs": 3, "bytes": {"$buffer": 0}});
    let with = |key: &str, value: Value| {
        let mut v = base.clone();
        v[key] = value;
        v
    };
    let eight = [vec![0u8; 8]];
    let print = fingerprint(&base, &eight);
    let mut volatile = with("id", json!(99));
    volatile["openMs"] = json!(900);
    volatile["paintMs"] = json!(1);
    assert_eq!(print, fingerprint(&volatile, &eight));
    assert_ne!(print, fingerprint(&with("annotId", json!(18)), &eight));
    assert_ne!(print, fingerprint(&with("revision", json!(4)), &eight));
    assert_ne!(print, fingerprint(&base, &[vec![0u8; 9]]));
    assert_eq!(
        fingerprint(&json!({"ok": true, "compareClosed": true}), &[]),
        fingerprint(&json!({"ok": true}), &[]),
        "the compare slot is not part of the document"
    );
}

// was tests/m9-recovery.test.mjs: 'the log keeps changes, skips reads and refusals, and restarts on open and close'
#[test]
fn the_log_keeps_changes_skips_reads_and_refusals_and_restarts_on_open_and_close() {
    let mut log = RecoveryLog::new(10_000);
    assert!(!log.available());
    let pdf = vec![vec![1u8; 1000]];
    tracked(
        &mut log,
        json!({"id": 1, "op": "open", "pdf": {"$buffer": 0}, "name": "a.pdf"}),
        &pdf,
        Some(json!({"ok": true, "pageCount": 1})),
    );
    assert!(log.available());
    assert_eq!(log.base.as_ref().map(Vec::len), Some(1000));
    assert!(!tracked(
        &mut log,
        json!({"id": 2, "op": "render", "page": 0, "scale": 1}),
        &[],
        Some(json!({"ok": true}))
    ));
    tracked(
        &mut log,
        json!({"id": 3, "op": "annot-add", "page": 0, "type": "Text", "contents": "x", "today": "2026-10-16"}),
        &[],
        Some(json!({"ok": true, "revision": 1, "annotId": 9})),
    );
    tracked(
        &mut log,
        json!({"id": 4, "op": "annot-add", "page": 0, "type": "Text", "contents": "refused"}),
        &[],
        None,
    );
    tracked(
        &mut log,
        json!({"id": 5, "op": "authenticate", "password": "wrong"}),
        &[],
        Some(json!({"ok": true, "authenticated": false})),
    );
    tracked(
        &mut log,
        json!({"id": 6, "op": "undo"}),
        &[],
        Some(json!({"ok": true, "revision": 2})),
    );
    let ops: Vec<&str> = log.entries.iter().map(|e| e.op.as_str()).collect();
    assert_eq!(ops, ["annot-add", "undo"]);
    assert!(
        !log.entries[0].message.contains_key("id"),
        "the request id is not kept"
    );
    assert_eq!(
        log.entries[0].message["today"], "2026-10-16",
        "injected inputs are kept"
    );
    assert!(log.bytes() > 1000 && log.bytes() < 2000, "{}", log.bytes());
    tracked(
        &mut log,
        json!({"id": 7, "op": "close"}),
        &[],
        Some(json!({"ok": true})),
    );
    assert!(!log.available());
    assert_eq!(log.entries.len(), 0);
    assert_eq!(log.bytes(), 0);
}

// was tests/m9-recovery.test.mjs: 'a log past its budget turns recovery off for the document, until the next open'
#[test]
fn a_log_past_its_budget_turns_recovery_off_until_the_next_open() {
    let mut log = RecoveryLog::new(4096);
    let ok = || Some(json!({"ok": true}));
    let open = |n: usize| {
        (
            json!({"op": "open", "pdf": {"$buffer": 0}, "name": "a.pdf"}),
            vec![vec![0u8; n]],
        )
    };
    let insert = json!({"op": "insert-pages", "pdf": {"$buffer": 0}, "name": "b.pdf", "at": 0});
    let big = vec![vec![0u8; 5000]];
    let (message, inputs) = open(1000);
    tracked(&mut log, message, &inputs, ok());
    // A refused oversized change keeps recovery on: it changed nothing.
    tracked(&mut log, insert.clone(), &big, None);
    assert!(log.available());
    let small = json!({"op": "annot-add", "page": 0, "type": "Text", "contents": "small"});
    tracked(
        &mut log,
        small,
        &[],
        Some(json!({"ok": true, "revision": 1})),
    );
    tracked(
        &mut log,
        insert,
        &big,
        Some(json!({"ok": true, "revision": 2})),
    );
    assert!(!log.available());
    assert_eq!(log.unavailable, Some(Unavailable::Budget));
    assert_eq!(log.entries.len(), 0, "what it held is freed");
    assert_eq!(log.bytes(), 0);
    assert_eq!(log.fallback(), None);
    let later = json!({"op": "annot-add", "page": 0, "type": "Text", "contents": "later"});
    tracked(
        &mut log,
        later,
        &[],
        Some(json!({"ok": true, "revision": 3})),
    );
    assert_eq!(log.entries.len(), 0);
    let (message, inputs) = open(100);
    tracked(&mut log, message, &inputs, ok());
    assert!(log.available());
    assert_eq!(log.unavailable, None);
    // A document over the budget is never tracked.
    let (message, inputs) = open(5000);
    tracked(&mut log, message, &inputs, ok());
    assert!(!log.available());
    assert_eq!(log.unavailable, Some(Unavailable::Budget));
}

// was tests/m9-recovery.test.mjs: 'a paste is replayable only after a copy the log holds'
#[test]
fn a_paste_is_replayable_only_after_a_copy_the_log_holds() {
    let mut log = RecoveryLog::new(RECOVERY_BUDGET_BYTES);
    let open = json!({"op": "open", "pdf": {"$buffer": 0}, "name": "a.pdf"});
    let pdf = vec![vec![0u8; 10]];
    let ok = || Some(json!({"ok": true}));
    let paste = json!({"op": "clip-paste", "page": 0});
    tracked(&mut log, open.clone(), &pdf, ok());
    tracked(
        &mut log,
        json!({"op": "clip-copy", "page": 0, "targets": []}),
        &[],
        ok(),
    );
    tracked(
        &mut log,
        paste.clone(),
        &[],
        Some(json!({"ok": true, "revision": 1})),
    );
    assert!(log.available());
    assert!(log.clipboard_in_log);
    // The clipboard outlives an open: a paste into the next document has no copy to replay.
    tracked(&mut log, open, &pdf, ok());
    assert!(!log.clipboard_in_log);
    tracked(
        &mut log,
        paste,
        &[],
        Some(json!({"ok": true, "revision": 1})),
    );
    assert_eq!(log.unavailable, Some(Unavailable::Clipboard));
}

// was tests/m9-recovery.test.mjs: 'edit handles of one generation only; the replay closes up to that generation'
#[test]
fn edit_handles_of_one_generation_only() {
    let mut log = RecoveryLog::new(RECOVERY_BUDGET_BYTES);
    let open = json!({"op": "open", "pdf": {"$buffer": 0}, "name": "a.pdf"});
    let pdf = vec![vec![0u8; 10]];
    let edit = |generation: i64, revision: i64, text: &str| {
        json!({"op": "edit-text", "page": 0, "targetId": "t", "generation": generation,
            "revision": revision, "text": text})
    };
    tracked(&mut log, open.clone(), &pdf, Some(json!({"ok": true})));
    tracked(
        &mut log,
        edit(5, 0, "A"),
        &[],
        Some(json!({"ok": true, "revision": 1})),
    );
    assert_eq!(log.generation(), Some(5.0));
    tracked(
        &mut log,
        edit(6, 1, "B"),
        &[],
        Some(json!({"ok": true, "revision": 2})),
    );
    assert_eq!(log.unavailable, Some(Unavailable::Generation));
    tracked(&mut log, open, &pdf, Some(json!({"ok": true})));
    let far = edit(MAX_REPLAY_CLOSES + 2, 0, "A");
    tracked(&mut log, far, &[], Some(json!({"ok": true, "revision": 1})));
    assert_eq!(log.unavailable, Some(Unavailable::Generation));
}

// was tests/m9-recovery.test.mjs: 'an acknowledged save in place keeps its bytes as the fallback; a save a copy changes nothing'
#[test]
fn an_acknowledged_save_in_place_keeps_its_bytes_as_the_fallback() {
    let mut log = RecoveryLog::new(10_000);
    let open = json!({"op": "open", "pdf": {"$buffer": 0}, "name": "a.pdf"});
    tracked(
        &mut log,
        open,
        &[vec![1u8; 1000]],
        Some(json!({"ok": true})),
    );
    assert_eq!(
        log.fallback(),
        log.base,
        "never saved: the opened bytes are the saved state"
    );
    // Save a copy: save and validate are reads, and nothing marks the document saved.
    assert!(!tracked(
        &mut log,
        json!({"op": "save"}),
        &[],
        Some(json!({"ok": true}))
    ));
    let validate = json!({"op": "validate", "pdf": {"$buffer": 0}});
    assert!(!tracked(
        &mut log,
        validate,
        &[vec![0; 1200]],
        Some(json!({"ok": true}))
    ));
    assert_eq!(log.entries.len(), 0);
    assert_eq!(log.fallback(), log.base);
    // Save in place: `saved` is a change (the clean baseline moves) and the written bytes are kept.
    assert!(log.fits_saved(1200));
    let saved = Some(json!({"ok": true, "revision": 0, "dirty": false}));
    tracked(&mut log, json!({"op": "saved"}), &[], saved);
    log.saved_copy(Some(vec![2; 1200]));
    let ops: Vec<&str> = log.entries.iter().map(|e| e.op.as_str()).collect();
    assert_eq!(ops, ["saved"]);
    assert_eq!(log.fallback().map(|b| b.len()), Some(1200));
    // The next save replaces the copy; one that does not fit leaves no fallback (not the stale one).
    assert!(!log.fits_saved(9000));
    log.saved_copy(None);
    assert_eq!(log.fallback(), None);
    assert!(log.available(), "the replay itself stays possible");
}

/// Everything the UI reads back from a document (m9-recovery.test.mjs `snapshot`).
fn snapshot(rig: &mut Rig, page_count: i64) -> Value {
    let mut pages = Vec::new();
    for page in 0..page_count {
        let annots = rig.ok(json!({"op": "annots", "page": page})).value["items"].clone();
        let fields = rig.ok(json!({"op": "fields", "page": page})).value["items"].clone();
        let targets: Vec<Value> =
            rig.ok(json!({"op": "edit-targets", "page": page})).value["items"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|item| {
                    json!([
                        item["targetId"],
                        item.get("text").cloned().unwrap_or(Value::Null)
                    ])
                })
                .collect();
        pages.push(json!({"annots": annots, "fields": fields, "targets": targets}));
    }
    let outline = rig.ok(json!({"op": "outline"})).value["items"].clone();
    let ids = rig.ok(json!({"op": "page-ids"})).value["ids"].clone();
    json!({"pages": pages, "outline": outline, "ids": ids})
}

/// The m9-recovery.test.mjs edits, through the session; returns the page count and the journal
/// state of the last answer.
fn replay_edits(rig: &mut Rig) -> (i64, Value) {
    rig.open("hello.pdf");
    let opened = rig.open("edit-replace.pdf");
    let page_count = opened["doc"]["pageCount"].as_i64().unwrap();
    let today = "2026-10-16";
    let note = rig.ok(
        json!({"op": "annot-add", "page": 0, "type": "Text", "point": [72, 72],
        "contents": "first note", "today": today}),
    );
    assert_eq!(note.value["revision"], 1);
    rig.ok(
        json!({"op": "annot-add", "page": 0, "type": "FreeText", "point": [100, 300],
        "contents": "a text box", "fit": true, "today": today}),
    );
    rig.ok_with(
        json!({"op": "annot-add", "page": 0, "type": "Stamp", "rect": [300, 600, 420, 650],
            "image": {"name": "img-gray.png", "bytes": {"$buffer": 0}},
            "sign": {"name": "Ada Lovelace"}, "contents": "", "today": today}),
        vec![fixture("img-gray.png")],
    );
    let items = rig.ok(json!({"op": "annots", "page": 0})).value["items"].clone();
    let note_id = items
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["contents"] == "first note")
        .map(|item| item["id"].clone())
        .unwrap();
    rig.ok(
        json!({"op": "annot-set", "page": 0, "annotId": note_id, "contents": "first note, edited"}),
    );
    let scan = rig.ok(json!({"op": "edit-targets", "page": 0})).value;
    assert_eq!(
        scan["generation"], 3,
        "the startup sample and hello.pdf came first"
    );
    let preview = rig
        .ok(
            json!({"op": "edit-replace-preview", "page": 0, "generation": scan["generation"],
            "revision": scan["revision"], "find": "ALPHA", "replace": "BETA"}),
        )
        .value;
    let target_ids: Vec<Value> = preview["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["targetId"].clone())
        .collect();
    assert!(!target_ids.is_empty(), "{preview}");
    rig.ok(
        json!({"op": "edit-replace-apply", "page": 0, "generation": preview["generation"],
        "revision": preview["revision"], "find": "ALPHA", "replace": "BETA",
        "previewToken": preview["previewToken"], "targetIds": target_ids}),
    );
    rig.ok(json!({"op": "page-rotate", "page": page_count - 1, "by": 90}));
    rig.ok(json!({"op": "outline-add", "title": "Recovered", "page": 0}));
    rig.ok(json!({"op": "undo"}));
    rig.ok(json!({"op": "redo"}));
    rig.ok(json!({"op": "saved"}));
    rig.ok(json!({"op": "annot-add", "page": 0, "type": "Square", "rect": [100, 100, 200, 160], "contents": ""}));
    rig.ok(json!({"op": "undo"}));
    let circle = rig
        .ok(json!({"op": "annot-add", "page": 0, "type": "Circle", "rect": [100, 400, 200, 460], "contents": ""}))
        .value;
    let journal = json!({"revision": circle["revision"], "dirty": circle["dirty"],
        "canUndo": circle["canUndo"], "canRedo": circle["canRedo"]});
    (page_count, journal)
}

// was tests/m9-recovery.test.mjs: 'replay in a fresh engine reproduces ids, revisions, undo history and the saved bytes'
#[test]
fn a_restore_reproduces_ids_revisions_undo_history_and_the_saved_bytes() {
    let mut restored = Rig::new("replay-a");
    let (page_count, before) = replay_edits(&mut restored);
    assert!(
        restored.s.recovery.available(),
        "{:?}",
        restored.s.recovery.unavailable
    );
    assert_eq!(restored.s.recovery.generation(), Some(3.0));
    assert_eq!(
        restored.ops(),
        [
            "annot-add",
            "annot-add",
            "annot-add",
            "annot-set",
            "edit-replace-apply",
            "page-rotate",
            "outline-add",
            "undo",
            "redo",
            "saved",
            "annot-add",
            "undo",
            "annot-add",
        ]
    );
    // The request that failed never reached the log; the engine restarts and the log replays.
    restored.trap("annot-add", Some(0));
    restored.expect(json!({
        "doc": {"name": "edit-replace.pdf", "pageCount": page_count},
        "revision": before["revision"], "dirty": before["dirty"],
        "canUndo": before["canUndo"], "canRedo": before["canRedo"],
        "recovery": {"available": true, "entries": 13,
            "last": {"outcome": "restored", "dropped": "annot-add", "cause": "trap", "steps": 13}},
    }));

    let mut control = Rig::new("replay-b");
    let (_, control_before) = replay_edits(&mut control);
    assert_eq!(control_before, before);
    let expected = snapshot(&mut control, page_count);
    let actual = snapshot(&mut restored, page_count);
    assert_eq!(
        actual, expected,
        "annotation, field, target and page ids, contents and bookmarks match"
    );
    let first_page = expected["pages"][0]["annots"].as_array().unwrap();
    assert!(
        first_page
            .iter()
            .any(|item| item["contents"] == "first note, edited")
    );

    // Undo history: both undo the circle and the next step, then redo the same.
    for op in ["undo", "undo", "redo"] {
        let a = control.ok(json!({"op": op}));
        let b = restored.ok(json!({"op": op}));
        assert_eq!(
            fingerprint(&a.value, &a.outputs),
            fingerprint(&b.value, &b.outputs),
            "{op}"
        );
    }
    // An edit after the restore gets the id the original would have given.
    let after = json!({"op": "annot-add", "page": 0, "type": "Text", "point": [50, 50],
        "contents": "after", "today": "2026-10-16"});
    let a = control.ok(after.clone());
    let b = restored.ok(after);
    assert_eq!(
        fingerprint(&a.value, &a.outputs),
        fingerprint(&b.value, &b.outputs)
    );
    // The signature's date is the recorded `today`; the entropy is fixed here.
    assert_eq!(
        save_bytes(&mut restored),
        save_bytes(&mut control),
        "the restored document saves byte for byte"
    );
}

// was tests/m9-recovery.test.mjs: 'replay stops at the first answer that differs, and at an open that differs'
// What stays observable from outside: the restore falls back to the opened bytes. The step it
// stopped at (`ReplayMismatch{index, op}`) and a refusal mid-replay are not: `Session::replay`
// is crate-private and nothing can make the restarted engine refuse (see the report).
#[test]
fn a_replay_that_answers_or_opens_differently_reopens_the_opened_bytes() {
    for tamper in ["answer", "open"] {
        let mut rig = Rig::new("mismatch");
        rig.open("hello.pdf");
        rig.note("one");
        rig.note("two");
        if tamper == "answer" {
            rig.s.recovery.entries[1].reply = "tampered".into();
        } else {
            rig.s.recovery.opened = "tampered".into();
        }
        rig.trap("annot-add", Some(0));
        rig.expect(json!({
            "doc": {"name": "hello.pdf"}, "notes": 0, "dirty": false, "canUndo": false,
            "error": "Adding the annotation failed inside the PDF engine, so it was stopped. Your changes since the last save could not be restored, so the document was reopened as it was opened.",
            "recovery": {"available": true, "entries": 0,
                "last": {"outcome": "reopened", "dropped": "annot-add", "cause": "trap", "steps": 0}},
        }));
        // The reopened document is a healthy session.
        rig.step(
            json!({"cmd": "note", "contents": "three", "expect": {"notes": 1, "dirty": true}}),
        );
    }
}

// was tests/m9-recovery.test.mjs: 'an encrypted document replays its unlock and its pending security change'
#[test]
fn an_encrypted_document_replays_its_unlock_and_its_pending_security_change() {
    let mut rig = Rig::new("unlock-security");
    rig.step(json!({"cmd": "open", "path": rig.path("encrypted.pdf"),
        "expect": {"doc": {"name": "encrypted.pdf", "needsPassword": true}}}));
    rig.step(json!({"cmd": "password", "contents": "not-the-password-7Q"}));
    rig.step(json!({"cmd": "password", "contents": "pdf4all-owner",
        "expect": {"doc": {"needsPassword": false}}}));
    rig.note("locked note");
    rig.ok(json!({"op": "security-set", "mode": "none"}));
    assert_eq!(rig.ops(), ["authenticate", "annot-add", "security-set"]);
    rig.trap("annot-add", Some(0));
    rig.expect(
        json!({"doc": {"name": "encrypted.pdf", "needsPassword": false}, "notes": 1, "dirty": true,
        "recovery": {"last": {"outcome": "restored", "steps": 3}}}),
    );
    let plain = save_bytes(&mut rig);
    let mut reopened = EngineHost::new();
    let info = reopened
        .request(
            json!({"op": "open", "pdf": {"$buffer": 0}, "name": "plain.pdf"}),
            vec![plain.clone()],
        )
        .expect("reopen")
        .value["info"]
        .clone();
    assert_eq!(
        info["encrypted"], false,
        "the pending security change was replayed"
    );
    assert!(contents_of(&plain).iter().any(|c| c == "locked note"));
}

// ------------------------------------------------------------------ save-ticket.test.mjs

// was tests/save-ticket.test.mjs: 'an edit made while the save is written stays unsaved; undoing it is clean'
#[test]
fn an_edit_after_the_save_captured_stays_unsaved_and_undoing_it_is_clean() {
    let mut rig = Rig::new("ticket-late-edit");
    rig.open("hello.pdf");
    rig.ticket_note("saved one");
    let save = rig.ok(json!({"op": "save"}));
    let ticket = save.value["ticket"].as_str().expect("a ticket").to_string();
    rig.ticket_note("made during the write");
    let marked = rig.ok(json!({"op": "saved", "ticket": ticket}));
    assert_eq!(
        marked.value["dirty"], true,
        "the edit after the capture is not on disk"
    );
    assert_eq!(
        rig.ok(json!({"op": "undo"})).value["dirty"],
        false,
        "back at the saved state"
    );
    assert_eq!(rig.ok(json!({"op": "redo"})).value["dirty"], true);
    // Nothing changed between the capture and the acknowledgement: clean.
    let again = rig.ok(json!({"op": "save"})).value["ticket"].clone();
    assert_eq!(
        rig.ok(json!({"op": "saved", "ticket": again})).value["dirty"],
        false
    );
    // Without a ticket, `saved` keeps its old meaning: the current state.
    rig.ticket_note("later");
    assert_eq!(rig.ok(json!({"op": "saved"})).value["dirty"], false);
}

// was tests/save-ticket.test.mjs: 'a ticket whose state an edit after an undo removed never reads clean'
#[test]
fn a_ticket_whose_state_was_cut_away_never_reads_clean() {
    let mut rig = Rig::new("ticket-cut");
    rig.open("hello.pdf");
    rig.ticket_note("saved one");
    let ticket = rig.ok(json!({"op": "save"})).value["ticket"].clone();
    rig.ok(json!({"op": "undo"}));
    rig.ticket_note("another branch");
    assert_eq!(
        rig.ok(json!({"op": "saved", "ticket": ticket})).value["dirty"],
        true
    );
    let undo = rig.ok(json!({"op": "undo"}));
    assert_eq!(
        undo.value["dirty"], true,
        "the opened state is not what is on disk"
    );
    assert_eq!(rig.ok(json!({"op": "redo"})).value["dirty"], true);
}

// was tests/save-ticket.test.mjs: 'a security change made while the save is written stays pending'
#[test]
fn a_security_change_after_the_save_captured_stays_pending() {
    let mut rig = Rig::new("ticket-security");
    rig.open("hello.pdf");
    rig.ticket_note("one");
    let ticket = rig.ok(json!({"op": "save"})).value["ticket"].clone();
    rig.ok(json!({"op": "security-set", "mode": "none"}));
    assert_eq!(
        rig.ok(json!({"op": "saved", "ticket": ticket})).value["dirty"],
        true
    );
    let again = rig.ok(json!({"op": "save"})).value["ticket"].clone();
    assert_eq!(
        rig.ok(json!({"op": "saved", "ticket": again})).value["dirty"],
        false
    );
}

// was tests/save-ticket.test.mjs: 'a ticket of another document or not issued by the engine is refused'
#[test]
fn a_ticket_of_another_document_or_not_issued_by_the_engine_is_refused() {
    let mut rig = Rig::new("ticket-invalid");
    rig.open("hello.pdf");
    let ticket = rig.ok(json!({"op": "save"})).value["ticket"].clone();
    let bogus = rig
        .s
        .request(json!({"op": "saved", "ticket": "not a ticket"}), vec![]);
    assert_eq!(bogus.map(|_| ()).unwrap_err().reason_str(), "invalid");
    rig.open("hello.pdf");
    let stale = rig
        .s
        .request(json!({"op": "saved", "ticket": ticket}), vec![]);
    assert_eq!(stale.map(|_| ()).unwrap_err().reason_str(), "invalid");
}

fn journal_of(s: &Session) -> Value {
    json!({"revision": s.revision, "dirty": s.dirty, "canUndo": s.can_undo, "canRedo": s.can_redo})
}

fn journal_reply(reply: &Reply) -> Value {
    let v = &reply.value;
    json!({"revision": v["revision"], "dirty": v["dirty"], "canUndo": v["canUndo"], "canRedo": v["canRedo"]})
}

// was tests/save-ticket.test.mjs: 'a logged saved with its ticket replays to the same dirty state'
#[test]
fn a_logged_saved_with_its_ticket_replays_to_the_same_dirty_state() {
    let mut rig = Rig::new("ticket-replay");
    rig.open("hello.pdf");
    rig.ticket_note("saved one");
    let ticket = rig.ok(json!({"op": "save"})).value["ticket"].clone();
    rig.ticket_note("made during the write");
    let marked = rig.ok(json!({"op": "saved", "ticket": ticket}));
    assert_eq!(rig.ops(), ["annot-add", "annot-add", "saved"]);
    rig.trap("annot-add", Some(0));
    rig.expect(json!({"recovery": {"last": {"outcome": "restored", "steps": 3}}}));
    assert_eq!(journal_of(&rig.s), journal_reply(&marked));
    assert!(rig.s.dirty);
    assert_eq!(
        rig.ok(json!({"op": "undo"})).value["dirty"],
        false,
        "undo after the restore"
    );
}

// was tests/save-ticket.test.mjs: 'a ticketed log of a document opened second replays at its generation'
#[test]
fn a_ticketed_log_of_a_document_opened_second_replays_at_its_generation() {
    let mut rig = Rig::new("ticket-generation");
    rig.open("hello.pdf");
    let ticket = rig.ok(json!({"op": "save"})).value["ticket"].clone();
    let text = ticket.as_str().unwrap();
    assert!(text.starts_with("2:"), "{text}");
    rig.ok(json!({"op": "saved", "ticket": ticket}));
    rig.ticket_note("three, unsaved");
    assert_eq!(
        rig.s.recovery.generation(),
        Some(2.0),
        "the generation comes from the ticket"
    );
    rig.trap("annot-add", Some(0));
    rig.expect(json!({"dirty": true, "recovery": {"last": {"outcome": "restored", "steps": 2}}}));
    let items = rig.ok(json!({"op": "annots", "page": 0})).value["items"].clone();
    assert_eq!(items.as_array().map(Vec::len), Some(1));
    assert_eq!(
        rig.ok(json!({"op": "undo"})).value["dirty"],
        false,
        "the save is the clean baseline"
    );
}

// was tests/save-ticket.test.mjs: 'two saves in place, a security change on an earlier document, then a replay'
#[test]
fn two_saves_a_security_change_on_an_earlier_document_then_a_restore() {
    let mut rig = Rig::new("ticket-two-saves");
    // The engine counts security changes over the session, not per document.
    rig.ok(json!({"op": "security-set", "mode": "none"}));
    rig.open("hello.pdf");
    rig.ticket_note("one");
    let first = rig.ok(json!({"op": "save"})).value["ticket"].clone();
    rig.ok(json!({"op": "saved", "ticket": first}));
    rig.ticket_note("two");
    let second = rig.ok(json!({"op": "save"})).value["ticket"].clone();
    rig.ticket_note("made during the second write");
    let marked = rig.ok(json!({"op": "saved", "ticket": second}));
    assert_eq!(marked.value["dirty"], true);
    rig.trap("annot-add", Some(0));
    rig.expect(json!({"recovery": {"last": {"outcome": "restored", "steps": 5}}}));
    assert_eq!(journal_of(&rig.s), journal_reply(&marked));
    assert_eq!(rig.ok(json!({"op": "undo"})).value["dirty"], false);
}

// was tests/save-ticket.test.mjs: 'a saved that arrives after a restore is accepted by the restored session'
// (In the desktop app `saved` follows its save at once, so this order needs a restore between
// them; the session still rebuilds such a ticket, which is what this checks.)
#[test]
fn a_saved_that_arrives_after_a_restore_is_accepted_by_the_restored_session() {
    let mut rig = Rig::new("ticket-late");
    rig.ok(json!({"op": "security-set", "mode": "none"}));
    rig.open("hello.pdf");
    rig.ticket_note("one");
    let ticket = rig.ok(json!({"op": "save"})).value["ticket"].clone();
    rig.trap("annot-add", Some(0));
    rig.expect(json!({"recovery": {"last": {"outcome": "restored", "steps": 1}}}));
    let late = rig.ok(json!({"op": "saved", "ticket": ticket}));
    assert_eq!(late.value["dirty"], false);
}

// ------------------------------------------------------------------ m9-recovery-drive.test.mjs

/// m9-recovery-drive.test.mjs `EDITS`, with the fixture copy.
fn edits(rig: &mut Rig) -> Value {
    let hello = rig.path("hello.pdf");
    rig.step(
        json!({"cmd": "open", "path": hello, "expect": {"doc": {"name": "hello.pdf"}, "error": "",
        "recovery": {"available": true, "entries": 0}}}),
    );
    rig.step(json!({"cmd": "freetext", "contents": "M9 box one", "expect": {"annots": {"FreeText": 1}, "dirty": true}}));
    rig.step(json!({"cmd": "note", "contents": "M9 note two", "expect": {"notes": 1}}));
    rig.step(json!({"cmd": "undo", "expect": {"notes": 0, "canRedo": true}}));
    rig.step(json!({"cmd": "redo", "expect": {"notes": 1}}));
    rig.step(json!({"cmd": "note", "contents": "M9 note three",
        "expect": {"notes": 2, "recovery": {"available": true, "entries": 5}}}))
}

// was tests/m9-recovery-drive.test.mjs: 'a hung request is dropped and reported; the edits before it, their ids and undo history survive; the save matches'
// In process nothing hangs; the same restore follows an engine panic (cause `trap`).
#[test]
fn a_trapped_request_is_dropped_and_reported_and_the_edits_ids_undo_history_and_save_survive() {
    let mut rig = Rig::new("drive-trap");
    let before = edits(&mut rig);
    rig.trap("annot-add", Some(0));
    let after = rig.expect(json!({
        "doc": {"name": "hello.pdf", "pageCount": 1}, "notes": 2, "annots": {"FreeText": 1, "Text": 2},
        "dirty": true, "canUndo": true, "canRedo": false,
        "error": "Adding the annotation failed inside the PDF engine, so it was stopped. That change was not applied; your other changes were restored.",
        "recovery": {"available": true, "entries": 5,
            "last": {"outcome": "restored", "dropped": "annot-add", "cause": "trap", "lost": 0, "steps": 5}},
    }));
    assert_eq!(
        after["revision"], before["revision"],
        "the restored journal is the one the window showed"
    );
    assert_eq!(
        after["fill"]["boxes"], before["fill"]["boxes"],
        "annotation ids and rects survive the restart"
    );
    assert_eq!(
        after["pages"]["ids"], before["pages"]["ids"],
        "page ids survive the restart"
    );
    rig.step(json!({"cmd": "undo", "expect": {"notes": 1, "canRedo": true}}));
    rig.step(json!({"cmd": "redo", "expect": {"notes": 2, "canRedo": false}}));
    let recovered_path = rig.out("recovered.pdf");
    rig.step(json!({"cmd": "save-to", "path": recovered_path,
        "expect": {"save": {"ok": true, "name": "recovered.pdf"}}}));

    let mut control = Rig::new("drive-control");
    edits(&mut control);
    control.step(json!({"cmd": "undo", "expect": {"notes": 1}}));
    control.step(json!({"cmd": "redo", "expect": {"notes": 2}}));
    let control_path = control.out("control.pdf");
    control.step(json!({"cmd": "save-to", "path": control_path,
        "expect": {"save": {"ok": true, "name": "control.pdf"}}}));
    let recovered = rig.read_out("recovered.pdf");
    let expected = control.read_out("control.pdf");
    assert_eq!(
        annots_of(&recovered),
        annots_of(&expected),
        "the same annotations as without the trap"
    );
    assert_eq!(
        recovered, expected,
        "byte-identical to the same edits without the trap"
    );
}

// was tests/m9-recovery-drive.test.mjs: 'a replay that fails reopens the last saved state and says the unsaved changes are gone'
#[test]
fn a_replay_that_fails_reopens_the_last_saved_state_and_says_the_unsaved_changes_are_gone() {
    let mut rig = Rig::new("drive-fallback");
    rig.open("hello.pdf");
    rig.step(json!({"cmd": "note", "contents": "M9 one", "expect": {"notes": 1}}));
    let fb = rig.out("fb.pdf");
    rig.step(
        json!({"cmd": "save-to", "path": fb, "expect": {"save": {"ok": true, "name": "fb.pdf"}}}),
    );
    rig.step(json!({"cmd": "open", "path": fb, "expect": {"doc": {"name": "fb.pdf"}, "notes": 1, "dirty": false}}));
    rig.step(json!({"cmd": "note", "contents": "M9 two", "expect": {"notes": 2}}));
    rig.step(json!({"cmd": "save-in-place", "expect": {"save": {"ok": true, "name": "fb.pdf"}, "dirty": false,
        "recovery": {"available": true, "saved": true, "entries": 2}}}));
    rig.step(json!({"cmd": "note", "contents": "M9 three, unsaved", "expect": {"notes": 3, "dirty": true}}));
    // The replay fails on "two" (Electron: it hung again), so the saved bytes are reopened.
    rig.s.recovery.entries[0].reply = "tampered".into();
    rig.trap("annot-add", Some(0));
    rig.expect(json!({
        "doc": {"name": "fb.pdf", "pageCount": 1}, "notes": 2, "dirty": false, "canUndo": false,
        "error": "Adding the annotation failed inside the PDF engine, so it was stopped. Your changes since the last save could not be restored, so the document was reopened as last saved.",
        "recovery": {"available": true, "entries": 0, "last": {"outcome": "reopened", "dropped": "annot-add", "cause": "trap"}},
    }));
    // The reopened document is a healthy session: edit and save in place again.
    rig.step(json!({"cmd": "note", "contents": "M9 four", "expect": {"notes": 3, "dirty": true}}));
    rig.step(json!({"cmd": "save-in-place", "expect": {"save": {"ok": true, "name": "fb.pdf"}, "dirty": false}}));
    assert_eq!(
        contents_of(&rig.read_out("fb.pdf")),
        ["M9 one", "M9 two", "M9 four"]
    );
}

// was tests/m9-recovery-drive.test.mjs: 'a save past its time limit is dropped, never marks the document saved, and is not replayed'
// The restore half only: a save that stops the engine says nothing was saved and keeps every
// change. That the failed save itself never marks the document saved needs an engine that
// panics on `save` (see the report).
#[test]
fn a_trapped_save_says_nothing_was_saved_and_keeps_every_change() {
    let mut rig = Rig::new("drive-save");
    rig.open("hello.pdf");
    rig.step(json!({"cmd": "note", "contents": "M9 kept", "expect": {"notes": 1, "dirty": true}}));
    rig.trap("save", None);
    rig.expect(json!({
        "doc": {"name": "hello.pdf"}, "notes": 1, "dirty": true,
        "error": "Saving failed inside the PDF engine, so it was stopped. Nothing was saved. Your document and changes were restored.",
        "recovery": {"available": true, "entries": 1, "paused": [], "last": {"outcome": "restored", "dropped": "save", "steps": 1}},
    }));
    let saved = rig.out("saved.pdf");
    rig.step(json!({"cmd": "save-to", "path": saved, "expect": {"save": {"ok": true, "name": "saved.pdf"}, "dirty": true}}));
    assert_eq!(contents_of(&rig.read_out("saved.pdf")), ["M9 kept"]);
}

// was tests/m9-recovery-drive.test.mjs: 'past the recovery budget a restart keeps the old behaviour and says the changes were too large'
#[test]
fn past_the_recovery_budget_a_restart_closes_the_document_and_says_the_changes_were_too_large() {
    let mut rig = Rig::new("drive-budget");
    rig.s.recovery.budget = 4096;
    let hello = rig.path("hello.pdf");
    rig.step(json!({"cmd": "open", "path": hello, "expect": {"doc": {"name": "hello.pdf"}, "recovery": {"available": true}}}));
    rig.step(json!({"cmd": "note", "contents": "M9 small", "expect": {"notes": 1, "recovery": {"available": true, "entries": 1}}}));
    rig.step(json!({"cmd": "note", "contents": format!("M9 large {}", "x".repeat(3000)),
        "expect": {"notes": 2, "recovery": {"available": false, "unavailable": "budget", "entries": 0, "bytes": 0}}}));
    rig.trap("annot-add", Some(0));
    rig.expect(json!({
        "doc": null,
        "error": "The PDF worker stopped (the PDF engine hit an internal error). Restarted it (1 of 3); open the document again to continue. Your unsaved changes could not be kept: they were too large to keep a recovery copy.",
        "recovery": {"available": false, "entries": 0},
    }));
    rig.step(json!({"cmd": "open", "path": hello,
        "expect": {"doc": {"name": "hello.pdf"}, "notes": 0, "error": "", "recovery": {"available": true}}}));
}

// was tests/m9-recovery-drive.test.mjs: 'an unlocked document is restored with its unlock and its page change'
#[test]
fn an_unlocked_document_is_restored_with_its_unlock_and_its_page_change() {
    let mut rig = Rig::new("drive-unlock");
    rig.step(json!({"cmd": "open", "path": rig.path("encrypted.pdf"),
        "expect": {"doc": {"name": "encrypted.pdf", "needsPassword": true}}}));
    rig.step(json!({"cmd": "password", "contents": "pdf4all-user", "expect": {"doc": {"needsPassword": false}}}));
    rig.step(json!({"cmd": "rotate-right", "expect": {"pages": {"rotate": [90]}, "dirty": true, "recovery": {"entries": 2}}}));
    rig.trap("annot-add", Some(0));
    rig.expect(json!({
        "doc": {"name": "encrypted.pdf", "needsPassword": false}, "pages": {"rotate": [90]}, "dirty": true, "notes": 0,
        "error": "Adding the annotation failed inside the PDF engine, so it was stopped. That change was not applied; your other changes were restored.",
        "recovery": {"available": true, "entries": 2, "last": {"outcome": "restored", "steps": 2}},
    }));
}

// was tests/m9-recovery-drive.test.mjs: 'a page whose drawing hangs is restored blank and not drawn again until Try again'
#[test]
fn a_page_whose_drawing_traps_is_restored_and_not_drawn_again_until_try_again() {
    let notice = "Showing the page failed inside the PDF engine, so it was stopped. Your document and changes were restored. Page 1 was not drawn again; choose Try again to draw it.";
    let mut rig = Rig::new("drive-page");
    rig.open("hello.pdf");
    for (n, contents) in ["M9 kept one", "M9 kept two", "M9 three"]
        .iter()
        .enumerate()
    {
        rig.step(json!({"cmd": "note", "contents": contents, "expect": {"notes": n + 1}}));
    }
    // A render that stopped the engine left nothing in the page cache; the injected one did.
    rig.s.cache.entries.clear();
    rig.s.cache.bytes = 0;
    rig.trap("render", Some(0));
    let paused = json!(["render:0", "thumbnail:0"]);
    let state = rig.expect(json!({
        "doc": {"name": "hello.pdf"}, "notes": 3, "dirty": true, "error": notice,
        "recovery": {"available": true, "entries": 3, "paused": paused,
            "last": {"outcome": "restored", "dropped": "render", "cause": "trap", "steps": 3}},
    }));
    let status = state["status"].as_str().unwrap_or("");
    assert!(status.starts_with("Page 1 could not be drawn"), "{status}");
    // No loop: later steps see the same restore as the last one, and one restart in all.
    rig.expect(json!({"error": notice, "recovery": {"last": {"dropped": "render", "steps": 3}}}));
    assert_eq!(rig.s.worker_restarts, 1);
    // An edit does not retry the page (each retry could stop the engine again).
    rig.step(json!({"cmd": "undo", "expect": {"notes": 2, "recovery": {"paused": paused}}}));
    rig.expect(json!({"notes": 2, "recovery": {"paused": paused, "last": {"dropped": "render", "steps": 3}}}));
    // Try again draws it.
    rig.step(json!({"cmd": "page-retry", "expect": {"recovery": {"paused": []}}}));
    let state =
        rig.expect(json!({"doc": {"name": "hello.pdf"}, "notes": 2, "recovery": {"paused": []}}));
    let status = state["status"].as_str().unwrap_or("");
    assert!(status.starts_with("Page 1 of 1 — rendered"), "{status}");
    // The undo went through, so the restart count started over; Try again stopped nothing.
    assert_eq!(rig.s.worker_restarts, 0);
    assert_eq!(rig.s.engine.stopped(), None);
}

// was tests/m9-recovery-drive.test.mjs: 'a save in place written during a failed replay is what the window reopens, and main hears dirty meanwhile'
// In process a save in place can land and still be unmarked only when its `saved` stops the
// engine; that state (`disk_write` done, not yet marked saved) is set here directly. The
// "main hears dirty meanwhile" half has no counterpart: the restore runs to its end at once.
#[test]
fn a_failed_restore_reopens_a_save_in_place_that_landed_but_was_not_marked_saved() {
    let mut rig = Rig::new("drive-write");
    rig.open("hello.pdf");
    rig.step(json!({"cmd": "note", "contents": "M9 one", "expect": {"notes": 1}}));
    let fb = rig.out("fb.pdf");
    rig.step(
        json!({"cmd": "save-to", "path": fb, "expect": {"save": {"ok": true, "name": "fb.pdf"}}}),
    );
    rig.step(json!({"cmd": "open", "path": fb, "expect": {"doc": {"name": "fb.pdf"}, "notes": 1, "dirty": false}}));
    rig.step(json!({"cmd": "note", "contents": "M9 two", "expect": {"notes": 2, "dirty": true}}));
    let written = save_bytes(&mut rig);
    std::fs::write(&fb, &written).unwrap();
    rig.s.disk_write = Some(DiskWrite {
        done: Some(true),
        bytes: written,
        dialog: false,
    });
    rig.s.recovery.entries[0].reply = "tampered".into();
    rig.trap("saved", None);
    rig.expect(json!({
        "doc": {"name": "fb.pdf"}, "notes": 2, "dirty": false,
        "error": "Recording the save failed inside the PDF engine, so it was stopped. Your changes since the last save could not be restored, so the document was reopened as last saved.",
        "recovery": {"available": true, "entries": 0, "restoring": false, "last": {"outcome": "reopened", "dropped": "saved"}},
    }));
    assert_eq!(contents_of(&rig.read_out("fb.pdf")), ["M9 one", "M9 two"]);
}

// was tests/m9-recovery-drive.test.mjs: 'a hang after a save in place restores every edit since (generation ≥ 2)' (both the one- and two-save cases)
#[test]
fn a_trap_after_a_save_in_place_restores_every_edit_since_at_generation_2_or_more() {
    for saves in [1, 2] {
        let mut rig = Rig::new("drive-generation");
        rig.open("hello.pdf");
        let fb = rig.out("fb.pdf");
        rig.step(json!({"cmd": "save-to", "path": fb, "expect": {"save": {"ok": true, "name": "fb.pdf"}}}));
        rig.step(
            json!({"cmd": "open", "path": fb, "expect": {"doc": {"name": "fb.pdf"}, "notes": 0}}),
        );
        rig.step(json!({"cmd": "note", "contents": "M9 two", "expect": {"notes": 1}}));
        rig.step(json!({"cmd": "save-in-place", "expect": {"save": {"ok": true, "name": "fb.pdf"}, "dirty": false}}));
        if saves == 2 {
            rig.step(
                json!({"cmd": "note", "contents": "M9 two and a half", "expect": {"notes": 2}}),
            );
            rig.step(json!({"cmd": "save-in-place", "expect": {"save": {"ok": true, "name": "fb.pdf"}, "dirty": false}}));
        }
        let notes = saves + 1;
        rig.step(json!({"cmd": "note", "contents": "M9 three, unsaved", "expect": {"notes": notes, "dirty": true}}));
        assert!(rig.s.recovery.generation().is_some_and(|g| g >= 2.0));
        rig.trap("annot-add", Some(0));
        rig.expect(json!({
            "doc": {"name": "fb.pdf"}, "notes": notes, "dirty": true, "canUndo": true,
            "error": "Adding the annotation failed inside the PDF engine, so it was stopped. That change was not applied; your other changes were restored.",
            "recovery": {"available": true, "last": {"outcome": "restored", "dropped": "annot-add"}},
        }));
        // The saved baseline survived the replay: undoing the unsaved note is clean.
        rig.step(json!({"cmd": "undo", "expect": {"notes": notes - 1, "dirty": false}}));
    }
}

// was tests/m9-recovery-drive.test.mjs: 'drive script: the recovery log over opens, edits, saves and an unlock'
// The same script (tests/drive/recovery-log.jsonl), run in process on copies of its fixtures.
#[test]
fn the_recovery_log_drive_script_over_opens_edits_saves_and_an_unlock() {
    let mut rig = Rig::new("drive-script");
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/drive/recovery-log.jsonl"
    );
    let text = std::fs::read_to_string(script).unwrap();
    // Inside a JSON string: a Windows path's backslashes must be escaped.
    let base = json!(format!("{}/", rig.dir.0.to_string_lossy())).to_string();
    let base = &base[1..base.len() - 1];
    let text = text.replace("\"path\": \"fixtures/", &format!("\"path\": \"{base}"));
    let out = rig.dir.0.join("out").to_string_lossy().into_owned();
    let lines = parse_script(&with_out(&text, &out)).unwrap();
    assert!(lines.len() >= 12);
    let mut report = String::new();
    for line in lines {
        let state = rig.step(line);
        report.push_str(&state.to_string());
    }
    assert!(
        !report.contains("pdf4all-user"),
        "the password is in no STATE"
    );
    assert!(Path::new(&out).join("recovery-log.pdf").is_file());
}

// worker-build.test.mjs's trap case, as the session sees it: a read that stopped the engine.
#[test]
fn a_trap_on_a_search_restores_the_document_and_the_next_open_works() {
    let mut rig = Rig::new("drive-search");
    rig.open("hello.pdf");
    rig.step(json!({"cmd": "note", "contents": "kept across the trap", "expect": {"notes": 1, "dirty": true}}));
    rig.trap("search", None);
    rig.expect(json!({
        "doc": {"name": "hello.pdf"}, "notes": 1, "dirty": true,
        "error": "Searching failed inside the PDF engine, so it was stopped. Your document and changes were restored.",
        "recovery": {"last": {"outcome": "restored", "dropped": "search", "cause": "trap"}},
    }));
    let hello = rig.path("hello.pdf");
    rig.step(json!({"cmd": "open", "path": hello,
        "expect": {"doc": {"name": "hello.pdf", "pageCount": 1}, "notes": 0, "error": ""}}));
}

// The restart cap (renderer.ts MAX_WORKER_RESTARTS): restores that change nothing count up;
// the fourth stop in a row stops the engine for good, and a change in between starts over.
#[test]
fn the_engine_stops_for_good_after_three_restarts_in_a_row() {
    let mut rig = Rig::new("drive-cap");
    rig.open("hello.pdf");
    rig.note("M9 one");
    rig.trap("search", None);
    rig.trap("search", None);
    assert_eq!(rig.s.worker_restarts, 2);
    rig.note("M9 two");
    assert_eq!(
        rig.s.worker_restarts, 0,
        "a change that went through starts the count over"
    );
    for n in 1..=MAX_WORKER_RESTARTS {
        rig.trap("search", None);
        rig.expect(json!({"doc": {"name": "hello.pdf"}, "notes": 2, "recovery": {"last": {"outcome": "restored"}}}));
        assert_eq!(rig.s.worker_restarts, n);
    }
    rig.trap("search", None);
    rig.expect(json!({
        "doc": null,
        "error": "The PDF worker stopped (the PDF engine hit an internal error) and has already been restarted 3 times. Reopen the app to continue.",
        "recovery": {"available": false},
    }));
    assert!(rig.s.worker_stopped);
    let refused = rig.s.request(json!({"op": "page-ids"}), vec![]);
    assert_eq!(refused.map(|_| ()).unwrap_err().reason_str(), "worker");
}

// Found while porting 'a restore does not wait on an Untitled document's Save dialog': after a
// failed restore reopens an Untitled document, its next Save (which asks for a path) is an
// ordinary save, not one that was in flight during the restore (save.rs: only a save started
// before the reopen is late).
#[test]
fn after_a_fallback_reopen_of_an_untitled_document_its_next_save_marks_it_saved() {
    let mut rig = Rig::new("drive-untitled");
    rig.step(json!({"cmd": "new", "expect": {"doc": {"name": "Untitled.pdf"}}}));
    rig.step(json!({"cmd": "note", "contents": "M9 one", "expect": {"notes": 1}}));
    rig.s.recovery.entries[0].reply = "tampered".into();
    rig.trap("annot-add", Some(0));
    rig.expect(json!({"doc": {"name": "Untitled.pdf"}, "notes": 0,
        "recovery": {"last": {"outcome": "reopened"}}}));
    rig.step(json!({"cmd": "note", "contents": "M9 two", "expect": {"notes": 1, "dirty": true}}));
    let saved = rig.out("saved.pdf");
    rig.step(json!({"cmd": "save-in-place", "path": saved,
        "expect": {"save": {"ok": true, "name": "saved.pdf"}, "doc": {"name": "saved.pdf"}, "dirty": false}}));
    assert_eq!(contents_of(&rig.read_out("saved.pdf")), ["M9 two"]);
}

// Found while porting: every change the window makes must reach the recovery log, or a
// restore silently drops it. A Fill text box goes through the session's `rq` (hit.rs), which
// now goes through `Session::request` and its log.
#[test]
fn a_fill_text_box_is_in_the_recovery_log_and_survives_a_restore() {
    let mut rig = Rig::new("drive-fill");
    rig.open("hello.pdf");
    rig.step(json!({"cmd": "note", "contents": "M9 kept", "expect": {"notes": 1}}));
    let before = rig.step(
        json!({"cmd": "fill-text", "contents": {"page": 0, "x": 72, "y": 500, "text": "Filled box"},
        "expect": {"annots": {"FreeText": 1}, "dirty": true}}),
    );
    assert_eq!(
        rig.ops(),
        ["annot-add", "annot-add"],
        "the text box is logged"
    );
    rig.trap("annot-add", Some(0));
    rig.expect(
        json!({"notes": 1, "annots": {"FreeText": 1}, "revision": before["revision"],
        "recovery": {"last": {"outcome": "restored", "steps": 2}}}),
    );
}
