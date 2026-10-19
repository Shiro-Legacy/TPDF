//! Assemble, unit 6 (renderer.ts 8955–8990): the page status line, picking the PDF that
//! "Insert pages…" and "Compare with…" read, and the insert request. The insert dialog,
//! `insert-pages` and the extract live with compare (renderer.ts 9243–9387).
#![allow(dead_code)]

use super::Session;
use super::dom_ids as ids;
use super::types::*;

impl Session {
    /// R:8957: `#page-status` (not in STATE).
    pub fn announce_page(&mut self, text: &str) {
        self.dom.set_text(ids::PAGE_STATUS_EL, text);
    }

    /// R:8962: the picked source, or None (a cancel is not an error). `action` is "insert" or
    /// "compare" (TS default "insert").
    pub fn pick_insert_source(&mut self, action: &str) -> Option<InsertSource> {
        let picked = match self.host.insert_source() {
            Ok(picked) => picked,
            Err(reason) => {
                if reason != "cancelled" {
                    let text = if reason == "too-large" {
                        format!("That PDF is larger than the {action} limit.")
                    } else {
                        "Could not read that PDF.".to_string()
                    };
                    self.show_error(&text);
                }
                return None;
            }
        };
        if picked.bytes.len() < 5 {
            self.show_error("That PDF is not readable.");
            return None;
        }
        Some(InsertSource {
            name: picked.name,
            bytes: picked.bytes,
        })
    }

    /// R:8984: "Insert pages…": pick a source, then ask where its pages go and which ones.
    pub fn request_insert(&mut self) {
        if self.doc.is_none() || !self.can_edit() {
            return;
        }
        if let Some(picked) = self.pick_insert_source("insert") {
            self.open_insert_dialog(picked);
        }
    }
}
