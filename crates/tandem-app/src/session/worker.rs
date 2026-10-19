//! Lane 1: renderer.ts 1037–1260, "worker IPC", on the in-process engine.
//!
//! The engine is synchronous, so the worker plumbing (ready, timeouts, ids, the save gate,
//! the restore barrier) collapses: what stays is what the STATE and the user see. A request
//! is refused before it is sent when its read is paused (M9) or the engine was stopped for
//! good; `annot-add`'s date and `redact-apply`'s entropy are drawn here so the recovery log
//! replays them; a successful change resets the restart count; a refused `open` resets the
//! log. A `reason: "worker"` reply (a caught engine panic, the WASM trap of the Electron app)
//! restarts the engine after the caller handled the refusal (`trap_pending`), and the
//! document is restored from the log (recovery.rs).
//!
//! Not reachable in process: the 20 s / 120 s / 10 s watchdogs (no request can be stopped
//! from outside), so the `slow` restore cause and the abandoned-thumbnail path never occur.
#![allow(dead_code)]

use super::Session;
use super::consts::MAX_WORKER_RESTARTS;
use super::recovery_log::replayed;
use super::types::{Culprit, RestoreCause, RestoreSnapshot};
use crate::engine::{EngineResult, Refusal};
use serde_json::{Value, json};
use std::time::Instant;

/// R:1193 `workerFailure(reason, message)`.
pub fn worker_failure(reason: &str, message: impl Into<String>) -> Refusal {
    Refusal::reason(reason, message)
}

/// The `page` a request names (`typeof message.page === "number"`).
pub(crate) fn request_page(message: &Value) -> Option<i64> {
    message
        .get("page")
        .and_then(Value::as_f64)
        .map(|p| p as i64)
}

impl Session {
    /// R:1075 `request(message)`: one engine request, as every part of the renderer sends it.
    /// Binary fields are `{"$buffer": i}` placeholders into `inputs`.
    pub fn request(&mut self, message: Value, inputs: Vec<Vec<u8>>) -> EngineResult {
        self.request_with(message, inputs, false)
    }

    /// `request(message, [], …, ownsGate = true, replaying = true)`: a replay's own request,
    /// never logged and never refused as paused.
    pub(crate) fn request_replaying(
        &mut self,
        message: Value,
        inputs: Vec<Vec<u8>>,
    ) -> EngineResult {
        self.request_with(message, inputs, true)
    }

    fn request_with(
        &mut self,
        mut message: Value,
        mut inputs: Vec<Vec<u8>>,
        replaying: bool,
    ) -> EngineResult {
        let op = message.get("op").cloned().unwrap_or(Value::Null);
        let page = request_page(&message);
        // M9: a read that hung the engine is not sent again automatically while the document is unchanged.
        if !replaying && self.is_paused(&op, page) {
            return Err(worker_failure("skipped", self.paused_text(&op, page)));
        }
        // Nothing is posted to an engine that answered `reason: "worker"`: it is replaced (and
        // the document restored) first, as the TS request waited for the next task and the
        // restore barrier.
        self.settle_trap();
        if self.worker_stopped {
            return Err(worker_failure(
                "worker",
                "the PDF worker has stopped; reopen the app",
            ));
        }
        // M9: the date and entropy the engine would draw are drawn here, so the log replays
        // them; a save ticket issued before a restore is rebuilt for the restored session.
        if !replaying {
            with_recorded_inputs(&mut message, &mut inputs);
            if op.as_str() == Some("saved")
                && let Some(ticket) = message
                    .get("ticket")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            {
                message["ticket"] = Value::String(self.recovery.ticket_now(&ticket));
            }
        }
        let kept = if replaying {
            None
        } else {
            message
                .as_object()
                .and_then(|map| self.recovery.keep(map, &inputs))
        };
        let started = Instant::now();
        let result = self.engine.request(message, inputs);
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        match result {
            Err(mut refusal) => {
                // R:1165: a refused open leaves no document to restore; any other refusal changed nothing.
                if kept.as_ref().is_some_and(|k| k.op == "open") {
                    self.recovery.reset();
                }
                if refusal.message.is_empty() {
                    refusal.message = "worker error".into();
                }
                // R:1176: an engine that stopped after an internal error lost its session: it is
                // restarted after the caller handled this reply. Later `worker` replies from the
                // same engine change nothing.
                if refusal.reason.as_deref() == Some("worker") && self.lane1.trap_pending.is_none()
                {
                    self.lane1.trap_pending = Some(Culprit {
                        op: op.clone(),
                        page,
                    });
                }
                Err(refusal)
            }
            Ok(reply) => {
                // A save's ticket names the state it captured: the log notes where that is.
                if !replaying && op.as_str() == Some("save") {
                    self.recovery.note_save(reply.get("ticket"));
                }
                if let Some(kept) = kept {
                    self.recovery.record(kept, &reply.value, &reply.outputs, ms);
                }
                // A change the user made went through: the restart cap starts over.
                if !replaying && replayed(&op) {
                    self.worker_restarts = 0;
                }
                Ok(reply)
            }
        }
    }

    /// Runs the engine restart a `reason: "worker"` reply scheduled (R:1185), if the engine
    /// was not stopped for good meanwhile.
    pub(crate) fn settle_trap(&mut self) {
        while let Some(culprit) = self.lane1.trap_pending.take() {
            if self.worker_stopped {
                break;
            }
            self.worker_failed(
                "the PDF engine hit an internal error",
                Some(culprit),
                RestoreCause::Trap,
            );
        }
    }

    /// The work the renderer scheduled for "the next macrotask" (R:1185, R:1257): an engine
    /// restart after a `worker` reply, then the restart notice shown again after the callers'
    /// own errors. The UI calls this after each event it hands the session; `driven` calls it
    /// after each request.
    pub fn run_pending_tasks(&mut self) {
        self.settle_trap();
        if let Some((notice, epoch)) = self.lane1.reshow_notice.take()
            && epoch == self.lane1.worker_epoch
            && self.doc.is_none()
        {
            self.show_error(&notice);
        }
    }

    /// R:1039 `startWorker`: a fresh engine.
    pub(crate) fn start_worker(&mut self) {
        self.worker_stopped = false;
        self.engine.restart();
        self.lane1.worker_epoch += 1;
    }

    /// R:1207 `workerFailed`: the engine lost the open document. When the recovery log can
    /// rebuild it, `restoreDocument` reopens it in the restarted engine and replays the
    /// changes since it was opened, without `culprit`. A failure during a restore only
    /// restarts the engine: the restore sees its own request fail and falls back. A read that
    /// failed is paused, so no restore issues it again.
    pub(crate) fn worker_failed(
        &mut self,
        reason: &str,
        culprit: Option<Culprit>,
        cause: RestoreCause,
    ) {
        // In process nothing else is pending: the culprit is the request that failed, and no
        // later change was posted behind it (`lost` is 0).
        let blamed = culprit;
        let lost = 0;
        if !self.restore_running
            && self.doc.is_some()
            && let Some(blamed) = &blamed
        {
            self.pause_read(blamed);
        }
        let restore = match &self.doc {
            Some(doc) if !self.restore_running && self.recovery.available() => {
                Some(RestoreSnapshot {
                    doc: doc.clone(),
                    wanted_page: self.wanted_page,
                    auth: self.auth,
                    security_state: self.security_state.clone(),
                    fill_save: self.fill_save,
                    layers_persisted: self.layers_persisted,
                    clipboard_info: self.clipboard_info.clone(),
                    dirty: self.dirty,
                    culprit: blamed.clone(),
                    cause,
                    lost,
                })
            }
            _ => None,
        };
        let untracked = if !self.restore_running && self.doc.is_some() {
            self.recovery.unavailable
        } else {
            None
        };
        self.clipboard_info = None; // §9.15: the clipboard lived in the dead engine
        self.clear_compare(); // §9.6: so did the compare slot
        // Main keeps asking before a close or an open while the restore runs (M9 review).
        if let Some(snap) = &restore {
            self.restore_dirty = snap.dirty;
        }
        self.reset_document();
        if self.worker_restarts >= MAX_WORKER_RESTARTS {
            self.worker_stopped = true;
            self.recovery.reset();
            self.restore_dirty = false;
            self.sync_dirty();
            let notice = format!(
                "The PDF worker stopped ({reason}) and has already been restarted {MAX_WORKER_RESTARTS} times. Reopen the app to continue."
            );
            self.stopped_notice = Some(notice.clone());
            self.show_error(&notice);
            self.set_status("The PDF worker stopped");
            return;
        }
        self.worker_restarts += 1;
        self.start_worker();
        if self.restore_running {
            return;
        }
        if let Some(snap) = restore {
            self.restore_document(snap);
            return;
        }
        self.recovery.reset();
        let kept = untracked
            .and_then(|why| super::consts::lookup(&super::consts::UNTRACKED_TEXT, why.as_str()))
            .map(|text| format!(" Your unsaved changes could not be kept: {text}"))
            .unwrap_or_default();
        let notice = format!(
            "The PDF worker stopped ({reason}). Restarted it ({} of {MAX_WORKER_RESTARTS}); open the document again to continue.{kept}",
            self.worker_restarts
        );
        self.show_error(&notice);
        // The callers of the failed request show their own errors first; this notice is the one that stays.
        self.lane1.reshow_notice = Some((notice, self.lane1.worker_epoch));
    }
}

/// R:1394 `withRecordedInputs`: `annot-add`'s local date and `redact-apply`'s 32 bytes of
/// entropy, drawn here so a replay writes what the original wrote.
pub(crate) fn with_recorded_inputs(message: &mut Value, inputs: &mut Vec<Vec<u8>>) {
    let Some(map) = message.as_object_mut() else {
        return;
    };
    match map.get("op").and_then(Value::as_str) {
        Some("annot-add") if !map.contains_key("today") => {
            map.insert("today".into(), json!(crate::engine::local_date()));
        }
        Some("redact-apply") if !map.contains_key("entropy") => {
            inputs.push(crate::engine::entropy(32));
            map.insert("entropy".into(), json!({ "$buffer": inputs.len() - 1 }));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_inputs() {
        let mut message = json!({"op": "annot-add", "page": 0});
        let mut inputs = Vec::new();
        with_recorded_inputs(&mut message, &mut inputs);
        assert!(message["today"].as_str().is_some_and(|d| d.len() == 10));
        let mut message = json!({"op": "redact-apply"});
        with_recorded_inputs(&mut message, &mut inputs);
        assert_eq!(message["entropy"], json!({"$buffer": 0}));
        assert_eq!(inputs[0].len(), 32);
    }

    #[test]
    fn opens_records_and_pauses() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/hello.pdf");
        let bytes = std::fs::read(path).unwrap();
        let mut s = Session::new();
        s.request(
            json!({"op": "open", "pdf": {"$buffer": 0}, "name": "hello.pdf"}),
            vec![bytes],
        )
        .unwrap();
        assert!(s.recovery.available());
        s.paused_reads.push("render:0".into());
        let err = s
            .request(json!({"op": "render", "page": 0, "scale": 1}), vec![])
            .unwrap_err();
        assert_eq!(err.reason_str(), "skipped");
        assert!(err.message.starts_with("Page 1 could not be drawn"));
        let bad = s.request(
            json!({"op": "open", "pdf": {"$buffer": 0}, "name": "x.pdf"}),
            vec![vec![1, 2, 3]],
        );
        assert!(bad.is_err());
        assert!(!s.recovery.available());
    }
}
