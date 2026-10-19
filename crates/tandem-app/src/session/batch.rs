//! Batch: stamp a whole folder (renderer.ts R:10123-10358; contract-batch.md). Main lists
//! the folder's PDFs and reads and writes `pdf4all-out/<name>`; the engine stamps each file
//! as a transient document (`batch-file`), never the open one. The state survives opening
//! and closing documents. The run is synchronous: a Cancel pressed in a UI takes effect
//! between files only if the UI gets a turn there (the drive's `cancelAfter` always does).

use super::Session;
use super::decorate::{bates_fields_ok, pad_number};
use super::dom_ids::*;
use super::types::*;
use crate::engine::Refusal;
use crate::js::js_trim;
use serde_json::{Value, json};

fn row_json(row: &BatchRow) -> Value {
    let mut value = json!({ "name": row.name, "status": row.status.as_str(), "pages": row.pages, "detail": row.detail });
    if let Some(bates) = row.bates {
        value["bates"] = json!({ "from": bates.from, "to": bates.to });
    }
    value
}

impl Session {
    /// R:10125: STATE `batch` (spec §4.6).
    pub fn batch_summary(&self) -> Value {
        let Some(state) = &self.batch_state else {
            return Value::Null;
        };
        let done = state
            .files
            .iter()
            .filter(|r| matches!(r.status, BatchStatus::Ok | BatchStatus::Failed))
            .count();
        let ok = state
            .files
            .iter()
            .filter(|r| r.status == BatchStatus::Ok)
            .count();
        let failed = state
            .files
            .iter()
            .filter(|r| r.status == BatchStatus::Failed)
            .count();
        json!({
            "folder": state.folder, "out": state.out,
            "action": state.action.as_ref().map_or(Value::Null, |a| serde_json::to_value(a).unwrap_or(Value::Null)),
            "total": state.files.len(), "done": done, "ok": ok, "failed": failed,
            "running": state.running, "cancelled": state.cancelled, "report": state.report,
            "bates": { "start": state.bates.start, "next": state.bates.next, "pad": state.bates.pad },
            "files": state.files.iter().map(row_json).collect::<Vec<_>>(),
        })
    }

    /// R:10137.
    pub fn ensure_batch_state(&mut self) -> &mut BatchState {
        let (next, pad) = (self.bates_next, self.bates_pad);
        self.batch_state.get_or_insert_with(|| BatchState {
            bates: BatchBates {
                start: next,
                next,
                pad,
            },
            ..BatchState::default()
        })
    }

    /// R:10162: one table row's cells (File, Status, Pages, Bates, Detail), as the UI draws them.
    pub fn batch_row_cells(&self, row: &BatchRow) -> [String; 5] {
        let pad = self
            .batch_state
            .as_ref()
            .map_or(self.bates_pad, |s| s.bates.pad);
        let range = row.bates.map_or(String::new(), |b| {
            format!(
                "{}\u{2013}{}",
                pad_number(b.from, pad),
                pad_number(b.to, pad)
            )
        });
        let pages = if row.status == BatchStatus::Ok {
            row.pages.to_string()
        } else {
            String::new()
        };
        [
            row.name.clone(),
            row.status.as_str().to_string(),
            pages,
            range,
            row.detail.clone(),
        ]
    }

    /// R:10143: the controls and the live summary, from `batchState` (the table is the UI's).
    pub fn render_batch(&mut self) {
        self.ensure_batch_state();
        let summary = self.batch_summary();
        let Some(state) = &self.batch_state else {
            return;
        };
        let (folder, running, empty) = (
            state.folder.clone().unwrap_or_default(),
            state.running,
            state.files.is_empty(),
        );
        self.dom.set_value(BATCH_FOLDER_INPUT, folder);
        self.dom.set_disabled(BATCH_PICK_BUTTON, running);
        self.dom.set_disabled(BATCH_START_BUTTON, running || empty);
        self.dom.set_disabled(BATCH_CLOSE_BUTTON, running);
        if running {
            let text = format!(
                "{} of {} done, {} failed",
                summary["done"], summary["total"], summary["failed"]
            );
            self.dom.set_text(BATCH_SUMMARY_EL, text);
        }
    }

    /// R:10166: File ▸ Batch… (no document needed).
    pub fn open_batch_dialog(&mut self) {
        let running = self.ensure_batch_state().running;
        if !running {
            self.dom
                .set_value(BATCH_BATES_START_INPUT, self.bates_next.to_string());
            self.dom
                .set_value(BATCH_BATES_PAD_INPUT, self.bates_pad.to_string());
            self.on_batch_kind_change();
        }
        self.render_batch();
        self.dom.set_open(BATCH_DIALOG, true);
    }

    /// R:10956: Align is off for a watermark.
    pub fn on_batch_kind_change(&mut self) {
        let watermark = self.dom.value(BATCH_KIND_SELECT) == "watermark";
        self.dom.set_disabled(BATCH_ALIGN_SELECT, watermark);
    }

    /// R:10178 `#batch-pick`: main's folder listing becomes the queued rows (§9.14).
    pub fn pick_batch_folder(&mut self) -> bool {
        if self.ensure_batch_state().running {
            return false;
        }
        let picked = match self.host.pick_folder() {
            Ok(picked) => picked,
            Err(reason) => {
                if reason != "cancelled" {
                    let text = if reason == "too-many-files" {
                        "Batch: the folder has more than 200 PDF files".to_string()
                    } else {
                        format!("Batch: the folder could not be read ({reason})")
                    };
                    self.show_error(&text);
                }
                return false;
            }
        };
        let count = picked.files.len();
        let state = self.ensure_batch_state();
        state.folder = Some(picked.folder.to_string_lossy().into_owned());
        state.out = Some(picked.out.to_string_lossy().into_owned());
        state.report = None;
        state.cancelled = false;
        state.files = picked
            .files
            .into_iter()
            .map(|(name, _)| BatchRow {
                name,
                status: BatchStatus::Queued,
                pages: 0,
                detail: String::new(),
                bates: None,
            })
            .collect();
        let text = if count > 0 {
            format!(
                "{count} PDF file{} queued",
                if count == 1 { "" } else { "s" }
            )
        } else {
            "No PDF files in this folder".to_string()
        };
        self.dom.set_text(BATCH_SUMMARY_EL, text);
        self.render_batch();
        true
    }

    /// R:10205 `#batch-start`: the action fields, checked as the stamp dialog checks them.
    pub fn start_batch(&mut self) -> bool {
        self.clear_error();
        let state = self.ensure_batch_state();
        if state.running {
            self.show_error("A batch is already running");
            return false;
        }
        if state.files.is_empty() {
            self.show_error("Batch: choose a folder with PDF files first");
            return false;
        }
        let kind = match self.dom.value(BATCH_KIND_SELECT).as_str() {
            "watermark" => "watermark",
            "header" => "header",
            _ => "footer",
        };
        let text = self.dom.value(BATCH_TEXT_INPUT);
        let start = js_trim(&self.dom.value(BATCH_BATES_START_INPUT)).to_string();
        let pad = js_trim(&self.dom.value(BATCH_BATES_PAD_INPUT)).to_string();
        let bates = text.contains("{bates}");
        if bates && !bates_fields_ok(&start, &pad) {
            self.show_error("Bates start must be a whole number and digits 1\u{2013}15");
            return false;
        }
        let pages = js_trim(&self.dom.value(BATCH_PAGES_INPUT)).to_string();
        let action = BatchAction {
            kind: kind.into(),
            text,
            pages: (!pages.is_empty()).then_some(pages),
            align: (kind != "watermark" && self.dom.value(BATCH_ALIGN_SELECT) == "right")
                .then(|| "right".into()),
            bates_pad: bates.then(|| pad.parse().unwrap_or(0)),
        };
        if bates {
            self.bates_next = start.parse().unwrap_or(0);
        }
        self.run_batch(action);
        true
    }

    /// R:10236: one file's refusal text: the reason, plus the engine's detail token.
    pub fn batch_failure(&self, err: &Refusal) -> String {
        let reason = err.reason_str();
        if reason.is_empty() || reason == "worker" {
            return self.message_of(err);
        }
        match err
            .reply
            .get("detail")
            .and_then(Value::as_str)
            .filter(|d| !d.is_empty())
        {
            Some(detail) => format!("{reason}: {detail}"),
            None => reason.to_string(),
        }
    }

    /// One file: exists probe, read, `batch-file` on a transient document, write.
    fn batch_one(
        &mut self,
        name: &str,
        action: &BatchAction,
    ) -> Result<(i64, Option<BatesRange>), Refusal> {
        let failure = |reason: &str, message: &str| Refusal::reason(reason, message);
        match self.host.batch_exists(name) {
            Err(reason) => return Err(failure(reason, reason)),
            Ok(true) => return Err(failure("exists", "exists")),
            Ok(false) => {}
        }
        let bytes = self
            .host
            .batch_read(name)
            .map_err(|reason| failure(reason, "unreadable"))?;
        if bytes.is_empty() {
            return Err(failure("unreadable", "unreadable"));
        }
        let mut wire = serde_json::to_value(action).unwrap_or_else(|_| json!({}));
        wire["batesStart"] = json!(self.bates_next);
        let reply = self.request(
            json!({ "op": "batch-file", "name": name, "bytes": { "$buffer": 0 }, "action": wire }),
            vec![bytes],
        )?;
        let out = reply.bytes("bytes").map(<[u8]>::to_vec).unwrap_or_default();
        self.host
            .batch_write(name, &out)
            .map_err(|reason| failure(reason, reason))?;
        let pages = reply.get("pages").and_then(Value::as_i64).unwrap_or(0);
        let bates = reply.get("bates").and_then(super::parse_reply);
        Ok((pages, bates))
    }

    /// R:10247: the queue (contract-batch.md §3.2, §9.11, §9.13): one file at a time, and the
    /// counter moves only after a write lands.
    pub fn run_batch(&mut self, action: BatchAction) {
        let pad = action.bates_pad.unwrap_or(self.bates_pad);
        let next = self.bates_next;
        let state = self.ensure_batch_state();
        for row in &mut state.files {
            *row = BatchRow {
                name: std::mem::take(&mut row.name),
                ..BatchRow::default()
            };
        }
        state.action = Some(action.clone());
        state.running = true;
        state.cancelled = false;
        state.report = None;
        state.bates = BatchBates {
            start: next,
            next,
            pad,
        };
        self.render_batch();
        let total = self.batch_state.as_ref().map_or(0, |s| s.files.len());
        for index in 0..total {
            let cancel_after = self.batch_cancel_after;
            let Some(state) = self.batch_state.as_mut() else {
                break;
            };
            if state.cancelled || cancel_after.is_some_and(|after| index as i64 >= after) {
                state.cancelled = true;
                break;
            }
            state.files[index].status = BatchStatus::Running;
            let name = state.files[index].name.clone();
            self.render_batch();
            let outcome = self.batch_one(&name, &action);
            let failure = outcome
                .as_ref()
                .err()
                .map(|err| (self.batch_failure(err), err.reason_str() == "worker"));
            if let Ok((_, Some(range))) = &outcome {
                self.bates_next = range.to + 1;
                self.bates_pad = pad;
            }
            let bates_next = self.bates_next;
            let Some(state) = self.batch_state.as_mut() else {
                break;
            };
            let row = &mut state.files[index];
            match (outcome, failure) {
                (Ok((pages, bates)), _) => {
                    row.status = BatchStatus::Ok;
                    row.pages = pages;
                    row.detail.clear();
                    row.bates = bates;
                }
                (Err(_), failure) => {
                    let (detail, worker) = failure.unwrap_or_default();
                    row.status = BatchStatus::Failed;
                    row.pages = 0;
                    row.detail = detail;
                    // A timeout or a dead worker stops the queue (§9.11); the rest stay queued.
                    if worker {
                        state.cancelled = true;
                    }
                }
            }
            state.bates.next = bates_next;
            self.render_batch();
        }
        let Some(state) = self.batch_state.as_ref() else {
            return;
        };
        let done: Vec<&BatchRow> = state
            .files
            .iter()
            .filter(|r| matches!(r.status, BatchStatus::Ok | BatchStatus::Failed))
            .collect();
        let ok = done.iter().filter(|r| r.status == BatchStatus::Ok).count();
        let failed = done.len() - ok;
        let lines: String = done
            .iter()
            .map(|row| {
                let bates = row.bates.map_or(String::new(), |b| {
                    format!("{}-{}", pad_number(b.from, pad), pad_number(b.to, pad))
                });
                format!(
                    "{}\t{}\t{}\t{}\t{bates}\n",
                    row.name,
                    row.status.as_str(),
                    row.pages,
                    row.detail
                )
            })
            .collect();
        let cancelled = state.cancelled;
        let mut report_text = String::new();
        if !lines.is_empty() {
            let report = match self.host.batch_write("report.txt", lines.as_bytes()) {
                Ok(_) => "ok".to_string(),
                Err(reason) => reason.to_string(),
            };
            report_text = if report == "ok" {
                ", report in pdf4all-out".to_string()
            } else {
                format!(
                    "; report not written: {}",
                    if report == "exists" {
                        "report.txt exists"
                    } else {
                        report.as_str()
                    }
                )
            };
            if let Some(state) = self.batch_state.as_mut() {
                state.report = Some(report);
            }
        }
        let text = format!(
            "Batch {}: {ok} ok, {failed} failed{report_text}",
            if cancelled { "cancelled" } else { "done" }
        );
        self.dom.set_text(BATCH_SUMMARY_EL, text.clone());
        self.announce(&text);
        if let Some(state) = self.batch_state.as_mut() {
            state.running = false;
        }
        self.batch_cancel_after = None;
        // A second Start continues the counter (contract-batch.md §9.13, D1), never the stale prefill.
        self.dom
            .set_value(BATCH_BATES_START_INPUT, self.bates_next.to_string());
        self.dom
            .set_value(BATCH_BATES_PAD_INPUT, self.bates_pad.to_string());
        self.render_batch();
    }

    /// R:10320 drive `batch {kind, text, pages?, align?, batesStart?, batesPad?, cancelAfter?}`
    /// with the line's `folder` pre-answer (§9.9): pick, fill the fields, run the whole queue.
    pub fn drive_batch(&mut self, contents: &str) {
        self.clear_error();
        if contents.is_empty() {
            self.show_error("batch: contents required");
            return;
        }
        let Some(wanted) = self.drive_wanted(Some(contents)) else {
            return;
        };
        if self.batch_state.as_ref().is_some_and(|s| s.running) {
            self.show_error("A batch is already running");
            return;
        }
        let kind = match wanted.get("kind").and_then(Value::as_str) {
            Some(kind @ ("watermark" | "header" | "footer")) => kind.to_string(),
            _ => {
                self.show_error("batch: kind must be watermark, header or footer");
                return;
            }
        };
        let align = wanted.get("align");
        if let Some(align) = align
            && align != "left"
            && align != "right"
        {
            self.show_error("Align must be left or right");
            return;
        }
        // `Number.isInteger(after) && after >= 0` (a present `null` is refused too).
        let after = match wanted.get("cancelAfter") {
            None => None,
            Some(value) => match value.as_f64().filter(|v| v.fract() == 0.0 && *v >= 0.0) {
                Some(v) => Some(v as i64),
                None => {
                    self.show_error("batch: cancelAfter must be a whole number");
                    return;
                }
            },
        };
        if !self.pick_batch_folder() {
            if self.dom.hidden(ERROR_EL) {
                self.show_error("batch: no folder was given");
            }
            return;
        }
        let text_of = |key: &str| match wanted.get(key) {
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        self.dom.set_value(BATCH_KIND_SELECT, kind);
        self.dom.set_value(BATCH_TEXT_INPUT, text_of("text"));
        self.dom.set_value(BATCH_PAGES_INPUT, text_of("pages"));
        let right = align.and_then(Value::as_str) == Some("right");
        self.dom
            .set_value(BATCH_ALIGN_SELECT, if right { "right" } else { "left" });
        let start = match wanted.get("batesStart") {
            Some(value) => super::cert_sign::js_string(Some(value)),
            None => self.bates_next.to_string(),
        };
        let pad = match wanted.get("batesPad") {
            Some(value) => super::cert_sign::js_string(Some(value)),
            None => self.bates_pad.to_string(),
        };
        self.dom.set_value(BATCH_BATES_START_INPUT, start);
        self.dom.set_value(BATCH_BATES_PAD_INPUT, pad);
        self.batch_cancel_after = after;
        if !self.start_batch() {
            self.batch_cancel_after = None;
        }
    }

    /// R:10947 "Cancel": stop after the running file, or close the dialog.
    pub fn on_batch_cancel(&mut self) {
        match self.batch_state.as_mut() {
            Some(state) if state.running => state.cancelled = true,
            _ => self.dom.set_open(BATCH_DIALOG, false),
        }
    }

    /// R:10951 "Close" (disabled while running).
    pub fn on_batch_close(&mut self) {
        if !self.batch_state.as_ref().is_some_and(|s| s.running) {
            self.dom.set_open(BATCH_DIALOG, false);
        }
    }

    /// R:10952 (§9.14): Escape while running cancels after the running file instead of hiding
    /// the progress; `false` = the dialog closes as usual.
    pub fn on_batch_dialog_cancel(&mut self) -> bool {
        match self.batch_state.as_mut() {
            Some(state) if state.running => {
                state.cancelled = true;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_counts_rows() {
        let mut s = Session::new();
        assert!(s.batch_summary().is_null());
        let state = s.ensure_batch_state();
        assert_eq!(state.bates.pad, 6);
        state.files = vec![
            BatchRow {
                name: "a.pdf".into(),
                status: BatchStatus::Ok,
                pages: 2,
                detail: String::new(),
                bates: Some(BatesRange { from: 1, to: 2 }),
            },
            BatchRow {
                name: "b.pdf".into(),
                status: BatchStatus::Failed,
                pages: 0,
                detail: "exists".into(),
                bates: None,
            },
            BatchRow {
                name: "c.pdf".into(),
                ..BatchRow::default()
            },
        ];
        let summary = s.batch_summary();
        assert_eq!(summary["done"], json!(2));
        assert_eq!(summary["ok"], json!(1));
        assert_eq!(summary["files"][0]["bates"], json!({"from": 1, "to": 2}));
        assert!(summary["files"][1].get("bates").is_none());
        assert_eq!(
            s.batch_row_cells(&s.batch_state.clone().unwrap().files[0])[3],
            "000001\u{2013}000002"
        );
    }

    #[test]
    fn drive_batch_refusals() {
        let mut s = Session::new();
        s.drive_batch("");
        assert_eq!(s.notice_raw, "batch: contents required");
        s.drive_batch(r#"{"kind":"x"}"#);
        assert_eq!(
            s.notice_raw,
            "batch: kind must be watermark, header or footer"
        );
        s.drive_batch(r#"{"kind":"footer","align":"middle"}"#);
        assert_eq!(s.notice_raw, "Align must be left or right");
        s.drive_batch(r#"{"kind":"footer","cancelAfter":1.5}"#);
        assert_eq!(s.notice_raw, "batch: cancelAfter must be a whole number");
        s.drive_batch(r#"{"kind":"footer"}"#);
        assert_eq!(s.notice_raw, "batch: no folder was given");
    }

    #[test]
    fn batch_failure_words() {
        let s = Session::new();
        let mut err = Refusal::reason("encrypted", "batch-file: encrypted");
        *err.reply = json!({"detail": "needs-password"});
        assert_eq!(s.batch_failure(&err), "encrypted: needs-password");
        assert_eq!(s.batch_failure(&Refusal::plain("boom")), "boom");
        assert_eq!(
            s.batch_failure(&Refusal::reason("worker", "stopped")),
            "stopped"
        );
    }
}
