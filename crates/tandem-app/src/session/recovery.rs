//! Lane 1: renderer.ts 1262–1549, "recovery (M9)": paused reads and Try again, the restart
//! notice, `restoreDocument` (replay into the restarted engine, else reopen from what is on
//! disk) and `showRestored`.
#![allow(dead_code)]

use super::Session;
use super::consts::AUTO_READS;
use super::dom_ids as ids;
use super::recovery_log::{ReplayError, op_info, replayed};
use super::types::{
    MessageKind, ReplayResult, RestoreCause, RestoreLast, RestoreOutcome, RestoreProgress,
    RestoreSnapshot,
};
use crate::js::js_num;
use serde_json::{Value, json};

fn is_auto_read(op: &Value) -> bool {
    op.as_str().is_some_and(|op| AUTO_READS.contains(&op))
}

fn op_text(op: &Value) -> String {
    match op {
        Value::String(s) => s.clone(),
        other => super::shell::js_string(other),
    }
}

/// R:1333 `pauseKeys`: `op` or `op:page`; a page that does not draw does not draw as a
/// thumbnail either.
pub fn pause_keys(op: &Value, page: Option<i64>) -> Vec<String> {
    if !is_auto_read(op) {
        return Vec::new();
    }
    let name = op_text(op);
    match page {
        Some(page) if name == "render" || name == "thumbnail" => {
            vec![
                format!("render:{}", js_num(page as f64)),
                format!("thumbnail:{}", js_num(page as f64)),
            ]
        }
        Some(page) => vec![format!("{name}:{}", js_num(page as f64))],
        None => vec![name],
    }
}

/// R:1405 `restoreNotice`: what was stopped, then what was kept.
pub fn restore_notice(
    snap: &RestoreSnapshot,
    outcome: RestoreOutcome,
    saved_ever: bool,
    over_budget: bool,
) -> String {
    let op = snap
        .culprit
        .as_ref()
        .map(|c| c.op.clone())
        .unwrap_or(Value::Null);
    let label = op_info(&op).map(|(_, label, _)| label);
    let first = match (snap.cause, label) {
        (RestoreCause::Slow, Some(label)) => format!("{label} took too long, so it was stopped."),
        (RestoreCause::Trap, Some(label)) => {
            format!("{label} failed inside the PDF engine, so it was stopped.")
        }
        (_, Some(label)) => format!("{label} stopped the PDF engine unexpectedly."),
        (_, None) => "The PDF engine stopped unexpectedly.".to_string(),
    };
    let op_name = op.as_str().unwrap_or("");
    let save = if op_name == "save" || op_name == "validate" {
        " Nothing was saved."
    } else {
        ""
    };
    let later = match snap.lost {
        0 => String::new(),
        1 => " A change made after it was not applied either.".to_string(),
        n => format!(" {n} changes made after it were not applied either."),
    };
    let page = snap.culprit.as_ref().and_then(|c| c.page);
    let paused = if !is_auto_read(&op) {
        String::new()
    } else if let (true, Some(page)) = (op_name == "render" || op_name == "thumbnail", page) {
        format!(
            " Page {} was not drawn again; choose Try again to draw it.",
            js_num(page as f64 + 1.0)
        )
    } else {
        " It was not tried again; choose Try again to retry it.".to_string()
    };
    match outcome {
        RestoreOutcome::Restored => {
            let kept = if replayed(&op) {
                " That change was not applied; your other changes were restored."
            } else {
                " Your document and changes were restored."
            };
            format!("{first}{save}{kept}{later}{paused}")
        }
        RestoreOutcome::Reopened => {
            let why = if over_budget {
                " Restoring your changes took too long, so the document was reopened"
            } else {
                " Your changes since the last save could not be restored, so the document was reopened"
            };
            let when = if saved_ever {
                "as last saved"
            } else {
                "as it was opened"
            };
            format!("{first}{save}{why} {when}.{paused}")
        }
        RestoreOutcome::Lost => {
            format!(
                "{first}{save} Your unsaved changes could not be restored; open the document again to continue."
            )
        }
    }
}

impl Session {
    /// R:1340 `pauseRead`.
    pub(crate) fn pause_read(&mut self, culprit: &super::types::Culprit) {
        for key in pause_keys(&culprit.op, culprit.page) {
            if !self.paused_reads.contains(&key) {
                self.paused_reads.push(key);
            }
        }
        self.render_paused();
    }

    /// R:1345 `isPaused`.
    pub fn is_paused(&self, op: &Value, page: Option<i64>) -> bool {
        pause_keys(op, page)
            .iter()
            .any(|key| self.paused_reads.contains(key))
    }

    /// R:1350 `pausedText`: the refusal a paused read gets.
    pub fn paused_text(&self, op: &Value, page: Option<i64>) -> String {
        let name = op.as_str().unwrap_or("");
        match page {
            Some(page) if name == "render" || name == "thumbnail" => format!(
                "Page {} could not be drawn because it took too long, so it is not shown, printed or exported. Choose Try again, or open the document again.",
                js_num(page as f64 + 1.0)
            ),
            _ => format!(
                "{} took too long, so it is paused. Choose Try again, or open the document again.",
                op_info(op).map_or("This", |(_, label, _)| label)
            ),
        }
    }

    /// R:1357 `renderPaused`: the notice bar with Try again, shown while anything is paused.
    pub(crate) fn render_paused(&mut self) {
        let pages: Vec<String> = self
            .paused_reads
            .iter()
            .filter_map(|key| key.strip_prefix("render:"))
            .map(|page| js_num(page.parse::<f64>().unwrap_or(f64::NAN) + 1.0))
            .collect();
        self.dom
            .set_hidden(ids::PAUSED_BAR, self.paused_reads.is_empty());
        let text = if !pages.is_empty() {
            let lead = if pages.len() == 1 {
                format!("Page {} is", pages[0])
            } else {
                format!("Pages {} are", pages.join(", "))
            };
            format!("{lead} not drawn: drawing took too long.")
        } else if !self.paused_reads.is_empty() {
            "Part of this document was not read: it took too long.".to_string()
        } else {
            String::new()
        };
        self.dom.set_text(ids::PAUSED_BAR_TEXT, text);
    }

    /// R:1366 `retryPaused` (the `#page-retry` button and the drive's `page-retry`): the paused
    /// reads are sent once more.
    pub fn retry_paused(&mut self) {
        if self.paused_reads.is_empty() {
            return;
        }
        let thumbs: Vec<i64> = self
            .paused_reads
            .iter()
            .filter_map(|key| key.strip_prefix("thumbnail:"))
            .filter_map(|page| page.parse::<i64>().ok())
            .collect();
        for page in thumbs {
            self.thumb_tried.remove(&page);
        }
        self.paused_reads.clear();
        self.render_paused();
        if self.doc.is_none() {
            return;
        }
        self.shown_key = None;
        self.refresh_notes();
        self.refresh_fields();
        self.refresh_links();
        self.refresh_outline();
        self.refresh_labels();
        self.refresh_layers();
        self.drain();
        self.pump_thumbs();
    }

    /// `#page-retry` click (R:1331).
    pub fn on_page_retry(&mut self) {
        self.retry_paused();
        self.run_pending_tasks();
    }

    /// R:1434 `restoreDocument`: reopens the document in the restarted engine and replays its
    /// log (`replay` checks every answer against the original, within a time budget scaled to
    /// the log). On a mismatch, a refusal, another stop or the budget, the document is
    /// reopened from what is on disk instead: the save in place being written if one is, else
    /// the last saved copy (or the opened bytes when it was never saved here).
    pub(crate) fn restore_document(&mut self, snap: RestoreSnapshot) {
        self.restore_running = true;
        let started_generation = self.generation;
        let had_changes = !self.recovery.entries.is_empty();
        let mut saved_ever = self.recovery.saved.is_some();
        let mut over_budget = false;
        let mut fallback_bytes: Option<Vec<u8>> = None;
        let name = snap.doc.name.clone();
        self.set_status(&format!("Restoring {name}…"));
        let replayed_result = self.replay(|s, done, total| {
            s.restore_progress = Some(RestoreProgress {
                done: done as i64,
                total: total as i64,
            });
            if total >= 20 {
                s.set_status(&format!("Restoring {name}… {done} of {total} changes"));
            }
        });
        let result: Option<ReplayResult> = match replayed_result {
            Ok(result) => {
                // Save tickets issued before the restart are rebuilt for this session from now on.
                self.recovery.restored();
                Some(result)
            }
            Err(err) => {
                over_budget = matches!(err, ReplayError::OverBudget { .. });
                eprintln!("recovery: replay failed ({err})");
                // An engine that failed again is replaced now, inside the restore, so the
                // fallback below opens in the new engine (M9 review).
                self.settle_trap();
                // A save in place main is writing decides what is on disk; one that first asks
                // the user for a path is not waited for.
                if let Some(write) = &self.disk_write {
                    if !write.dialog && write.done == Some(true) {
                        fallback_bytes = Some(write.bytes.clone());
                        saved_ever = true;
                    }
                }
                None
            }
        };
        self.restore_running = false;
        self.restore_progress = None;
        // A newer open took over while the replay ran, or the restart cap stopped the engine.
        if started_generation != self.generation || self.worker_stopped {
            self.restore_dirty = false;
            self.sync_dirty();
            return;
        }
        let dropped = snap
            .culprit
            .as_ref()
            .and_then(|c| c.op.as_str())
            .map(str::to_string);
        if let Some(result) = result {
            self.restore_last = Some(RestoreLast {
                outcome: RestoreOutcome::Restored,
                dropped,
                cause: snap.cause,
                lost: snap.lost,
                steps: result.steps as i64,
                over_budget: None,
            });
            self.restore_dirty = false;
            self.show_restored(snap, result);
            return;
        }
        let from_write = fallback_bytes.is_some();
        let bytes = fallback_bytes.or_else(|| self.recovery.fallback());
        let outcome = match &bytes {
            None => RestoreOutcome::Lost,
            Some(_) if had_changes || from_write => RestoreOutcome::Reopened,
            Some(_) => RestoreOutcome::Restored,
        };
        self.restore_last = Some(RestoreLast {
            outcome,
            dropped,
            cause: snap.cause,
            lost: snap.lost,
            steps: 0,
            over_budget: over_budget.then_some(true),
        });
        let Some(bytes) = bytes else {
            self.recovery.reset();
            self.restore_dirty = false;
            self.sync_dirty();
            // The engine may hold a half-replayed document: close it.
            let _ = self.request(json!({"op": "close"}), Vec::new());
            self.show_error(&restore_notice(
                &snap,
                RestoreOutcome::Lost,
                saved_ever,
                false,
            ));
            return;
        };
        let notice = restore_notice(&snap, outcome, saved_ever, over_budget);
        // openDocument starts a new log from these bytes: the reopened state is the new base.
        self.restore_dirty = false;
        self.fallback_generation = self.generation + 1;
        self.open_document(&snap.doc.name, bytes, snap.doc.url.clone(), true);
        let Some(doc) = self.doc.as_mut() else {
            return; // the open failed and said why
        };
        if snap.doc.untitled == Some(true) && doc.url == snap.doc.url {
            doc.untitled = Some(true);
        }
        if snap.fill_save == super::types::FillSave::Copied {
            self.fill_save = super::types::FillSave::Copied; // the window still edits the "(filled)" copy it saved
        }
        self.show_message(&notice, MessageKind::Warning); // X8: handled and recovered, so amber
        self.announce(&notice);
    }

    /// R:1514 `showRestored`: the replay matched: the window shows the document as it was, at
    /// the page it was on.
    pub(crate) fn show_restored(&mut self, snap: RestoreSnapshot, result: ReplayResult) {
        let journal = &result.journal;
        self.revision = journal.get("revision").and_then(Value::as_i64).unwrap_or(0);
        self.dirty = journal.get("dirty") == Some(&Value::Bool(true));
        self.can_undo = journal.get("canUndo") == Some(&Value::Bool(true));
        self.can_redo = journal.get("canRedo") == Some(&Value::Bool(true));
        self.auth = snap.auth;
        self.security_state = snap.security_state.clone();
        self.fill_save = snap.fill_save;
        self.layers_persisted = snap.layers_persisted;
        // The clipboard came back only if the copy that filled it was replayed.
        self.clipboard_info = if self.recovery.clipboard_in_log {
            snap.clipboard_info.clone()
        } else {
            None
        };
        let name = snap.doc.name.clone();
        let page_count = snap.doc.page_count;
        let needs_password = snap.doc.info.needs_password;
        let info = snap.doc.info.clone();
        self.doc = Some(snap.doc.clone());
        self.dom
            .set_text(super::shell::DOCUMENT_TITLE, format!("{name} — Tandem PDF"));
        self.dom.set_text(ids::DOCUMENT_TITLE_EL, name.clone());
        self.sync_dirty();
        self.render_banner(&info);
        self.wanted_page = snap.wanted_page.min(page_count - 1).max(0);
        self.shown_page = -1;
        self.shown_key = None;
        self.refresh_thumbs();
        self.refresh_notes();
        self.refresh_fields();
        self.refresh_outline();
        self.refresh_links();
        self.refresh_labels();
        self.refresh_layers();
        self.refresh_redact_preview();
        self.sync_controls();
        let notice = restore_notice(&snap, RestoreOutcome::Restored, false, false);
        self.show_message(&notice, MessageKind::Warning); // X8: the document came back, so amber
        self.announce(&notice);
        self.set_status(&format!("Restored {name}"));
        if needs_password {
            self.show_password_form();
        } else {
            self.drain();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::types::Culprit;

    fn snap(op: &str, page: Option<i64>, cause: RestoreCause, lost: i64) -> RestoreSnapshot {
        RestoreSnapshot {
            culprit: Some(Culprit {
                op: json!(op),
                page,
            }),
            cause,
            lost,
            ..RestoreSnapshot::default()
        }
    }

    #[test]
    fn keys() {
        assert_eq!(
            pause_keys(&json!("render"), Some(2)),
            vec!["render:2", "thumbnail:2"]
        );
        assert_eq!(pause_keys(&json!("annots"), Some(1)), vec!["annots:1"]);
        assert_eq!(pause_keys(&json!("outline"), None), vec!["outline"]);
        assert!(pause_keys(&json!("annot-add"), Some(1)).is_empty());
    }

    #[test]
    fn notices() {
        assert_eq!(
            restore_notice(
                &snap("render", Some(1), RestoreCause::Trap, 0),
                RestoreOutcome::Restored,
                false,
                false
            ),
            "Showing the page failed inside the PDF engine, so it was stopped. Your document and changes were restored. Page 2 was not drawn again; choose Try again to draw it."
        );
        assert_eq!(
            restore_notice(
                &snap("annot-add", Some(0), RestoreCause::Stopped, 2),
                RestoreOutcome::Restored,
                false,
                false
            ),
            "Adding the annotation stopped the PDF engine unexpectedly. That change was not applied; your other changes were restored. 2 changes made after it were not applied either."
        );
        assert_eq!(
            restore_notice(
                &snap("save", None, RestoreCause::Slow, 0),
                RestoreOutcome::Reopened,
                true,
                true
            ),
            "Saving took too long, so it was stopped. Nothing was saved. Restoring your changes took too long, so the document was reopened as last saved."
        );
        assert_eq!(
            restore_notice(
                &snap("zzz", None, RestoreCause::Trap, 0),
                RestoreOutcome::Lost,
                false,
                false
            ),
            "The PDF engine stopped unexpectedly. Your unsaved changes could not be restored; open the document again to continue."
        );
    }

    #[test]
    fn paused_bar_text() {
        let mut s = Session::new();
        s.paused_reads = vec!["render:0".into(), "thumbnail:0".into(), "render:3".into()];
        s.render_paused();
        assert_eq!(
            s.dom.text(ids::PAUSED_BAR_TEXT),
            "Pages 1, 4 are not drawn: drawing took too long."
        );
        assert!(!s.dom.hidden(ids::PAUSED_BAR));
        assert_eq!(
            s.paused_text(&json!("outline"), None),
            "Reading the bookmarks took too long, so it is paused. Choose Try again, or open the document again."
        );
    }
}
