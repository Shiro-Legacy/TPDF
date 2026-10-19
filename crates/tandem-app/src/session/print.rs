//! Print (renderer.ts R:11816-11954; spec §3.C.5) and the drop zone (R:11444-11499).
//!
//! The Electron renderer rendered each page at 150 DPI into JPEG sheets and printed the
//! window. Here the session hands `self.host` a PDF of the chosen pages (`extract-pages`:
//! the pages as the engine holds them now, vector and exact), so `bytes` in `P1_PRINT` and
//! STATE `print` is that PDF's size, still capped at `PRINT_MAX_BYTES`. The per-page
//! "Preparing page i of n" status steps are kept.

use super::Session;
use super::consts::{PRINT_MAX_BYTES, PRINT_REASON_TEXT, lookup};
use super::decorate::parse_page_range;
use super::dom_ids::*;
use super::types::PendingAction;
use serde_json::{Map, json};
use std::time::Instant;

fn pages_word(n: usize) -> String {
    format!("{n} page{}", if n == 1 { "" } else { "s" })
}

impl Session {
    /// R:11817: the print sheets are not kept (the host receives one PDF).
    pub fn clear_print_sheets(&mut self) {}

    /// R:11821.
    pub fn print_available(&self) -> bool {
        self.doc
            .as_ref()
            .is_some_and(|doc| !doc.info.needs_password && doc.info.printable)
    }

    /// R:11825: "Print…" / File ▸ Print… without a range.
    pub fn open_print_dialog(&mut self) {
        if self.printing || self.dom.is_open(PRINT_DIALOG) || self.doc.is_none() {
            return;
        }
        if !self.print_available() {
            self.set_status("This document does not permit printing");
            return;
        }
        self.update_print_range();
        self.dom.set_open(PRINT_DIALOG, true);
    }

    /// R:11838: the line under the range input: the page count, or why the text is not usable
    /// (also the range input's `input` listener).
    pub fn update_print_range(&mut self) {
        let range = self.dom.value(PRINT_RANGE_INPUT);
        let pages = self
            .doc
            .as_ref()
            .and_then(|doc| parse_page_range(&range, doc.page_count));
        let text = match (&pages, &self.doc) {
            (Some(p), _) => pages_word(p.len()),
            (None, Some(doc)) => super::shell::range_problem(&range, doc.page_count, &doc.name),
            (None, None) => "Open a document first.".to_string(),
        };
        self.set_status_line(
            PRINT_STATUS_EL,
            text,
            if pages.is_some() { "info" } else { "error" },
        );
        self.dom.set_disabled(PRINT_START_BUTTON, pages.is_none());
    }

    /// R:11847: Escape while the print is being prepared.
    pub fn cancel_print(&mut self) {
        self.print_token += 1;
        self.set_status("Print cancelled");
    }

    /// R:11180: the dialog's "Print".
    pub fn on_print_start(&mut self) {
        let range = self.dom.value(PRINT_RANGE_INPUT);
        self.dom.set_open(PRINT_DIALOG, false);
        self.start_print(&range);
    }

    /// R:11852.
    pub fn start_print(&mut self, range_text: &str) {
        let Some(page_count) = self.doc.as_ref().map(|doc| doc.page_count) else {
            return;
        };
        if self.printing || !self.print_available() {
            return;
        }
        let Some(pages) = parse_page_range(range_text, page_count) else {
            let name = self
                .doc
                .as_ref()
                .map(|d| d.name.clone())
                .unwrap_or_default();
            self.show_error_as(
                "That page range is not valid.",
                &super::shell::range_problem(range_text, page_count, &name),
            );
            return;
        };
        if pages.is_empty() {
            self.show_error("That page range selects no pages.");
            return;
        }
        // Redaction §2.3 step 7: a printed page still shows what unredacted boxes cover.
        if !self.confirm_pending_save(PendingAction::Print) {
            return;
        }
        let generation = self.generation;
        self.print_token += 1;
        let job = self.print_token;
        let started = Instant::now();
        self.printing = true;
        self.clear_error();
        self.sync_controls();
        self.print_run(&pages, job, generation, started);
        self.clear_print_sheets();
        self.printing = false;
        self.sync_controls();
    }

    fn print_run(&mut self, pages: &[usize], job: i64, generation: i64, started: Instant) {
        let n = pages.len();
        for index in 0..n {
            if job != self.print_token || generation != self.generation {
                return;
            }
            self.set_status(&format!(
                "Preparing page {} of {n} for printing…",
                index + 1
            ));
        }
        let pdf = match self.request(json!({ "op": "extract-pages", "pages": pages }), vec![]) {
            Ok(reply) => reply.bytes("bytes").map(<[u8]>::to_vec).unwrap_or_default(),
            Err(err) => {
                let text = self.message_of(&err);
                self.show_error(&text);
                return;
            }
        };
        if job != self.print_token || self.doc.is_none() {
            return;
        }
        let bytes = pdf.len();
        if bytes > PRINT_MAX_BYTES {
            self.show_error("Too many pages to print at once; choose a smaller range");
            return;
        }
        let prep_ms = started.elapsed().as_millis() as u64;
        self.console_line(
            "P1_PRINT",
            json!({ "pages": n, "prepMs": prep_ms, "bytes": bytes }),
        );
        self.set_status(&format!("Printing {}…", pages_word(n)));
        let result = self.host.print(&pdf);
        let mut last = Map::new();
        last.insert("ok".into(), json!(result.is_ok()));
        if let Err(reason) = result {
            last.insert("reason".into(), json!(reason));
        }
        last.insert("pages".into(), json!(n));
        last.insert("bytes".into(), json!(bytes));
        self.last_print = Some(last);
        match result {
            Ok(()) => self.set_status(&format!("Sent {} to the printer", pages_word(n))),
            Err("cancelled") => self.set_status("Print cancelled"),
            Err(reason) => {
                let text = lookup(&PRINT_REASON_TEXT, reason)
                    .unwrap_or("The document could not be printed.");
                self.show_error(text);
            }
        }
    }

    // ------------------------------------------------------------- drop zone (R:11452)

    /// `dragenter` on the window: only a drag carrying files shows the overlay.
    pub fn on_drag_enter(&mut self, has_files: bool) {
        if !has_files {
            return;
        }
        self.drop_depth += 1;
        self.dom.set_hidden(DROP_HINT, false);
    }

    /// `dragover` (its default is always cancelled; the UI answers "copy" for files).
    pub fn on_drag_over(&mut self, has_files: bool) {
        if has_files {
            self.dom.set_hidden(DROP_HINT, false);
        }
    }

    /// `dragleave`: some drags report no types on the way out, so this is counted, not filtered.
    pub fn on_drag_leave(&mut self) {
        self.drop_depth = (self.drop_depth - 1).max(0);
        if self.drop_depth == 0 {
            self.dom.set_hidden(DROP_HINT, true);
        }
    }

    /// `drop`: only the first file is used: `(name, path)` as the OS gave them.
    pub fn on_drop(&mut self, first: Option<(&str, &str)>) {
        self.drop_depth = 0;
        self.dom.set_hidden(DROP_HINT, true);
        if let Some((name, path)) = first {
            self.open_dropped(name, path);
        }
    }

    /// R:11483: main resolves the path and opens it the normal way (discard question, size
    /// ceiling, not-a-PDF → an `open-error` event). One drop at a time per window.
    pub fn open_dropped(&mut self, name: &str, path: &str) {
        if self.drop_busy {
            return;
        }
        self.drop_busy = true;
        self.clear_error();
        self.set_status(&format!("Opening {name}…"));
        self.host.open_path(path);
        self.drop_busy = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_depth_counts() {
        let mut s = Session::new();
        s.on_drag_enter(true);
        s.on_drag_enter(true);
        assert!(!s.dom.hidden(DROP_HINT));
        s.on_drag_leave();
        assert!(!s.dom.hidden(DROP_HINT));
        s.on_drag_leave();
        assert!(s.dom.hidden(DROP_HINT));
        s.on_drag_leave();
        assert_eq!(s.drop_depth, 0);
    }

    #[test]
    fn print_range_line() {
        let mut s = Session::new();
        s.update_print_range();
        assert_eq!(s.dom.text(PRINT_STATUS_EL), "Open a document first.");
        assert!(s.dom.disabled(PRINT_START_BUTTON));
        s.doc = Some(super::super::types::OpenDoc {
            page_count: 3,
            ..Default::default()
        });
        s.dom.set_value(PRINT_RANGE_INPUT, "1-2");
        s.update_print_range();
        assert_eq!(s.dom.text(PRINT_STATUS_EL), "2 pages");
    }
}
