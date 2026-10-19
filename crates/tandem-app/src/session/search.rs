//! Search: the find bar in toolbar row 1 (renderer.ts R:5347-5430).
#![allow(dead_code)]

use super::Session;
use super::dom_ids::{SEARCH_INPUT, SEARCH_STATUS_EL};
use crate::js::js_trim;
use crate::messages::user_text;
use serde_json::json;
use tandem_protocol::engine::SearchReply;
use tandem_protocol::limits::MAX_HITS_DEFAULT;

impl Session {
    /// R:5349: typing in the find field (`text` is the field's new value). The TS waited
    /// SEARCH_DEBOUNCE_MS before searching; that delay is not observable to a drive (its `find`
    /// runs `start_search` at once), so the search runs now. A UI that wants the debounce
    /// calls this after its own 200 ms.
    pub fn on_search_input(&mut self, text: &str) {
        self.dom.set_value(SEARCH_INPUT, text);
        let query = js_trim(text).to_string();
        if query.is_empty() {
            self.clear_search();
            return;
        }
        self.set_search_status("Searching…");
        self.start_search();
    }

    /// R:5366.
    pub fn start_search(&mut self) {
        let query = js_trim(&self.dom.value(SEARCH_INPUT)).to_string();
        if query.is_empty() {
            self.clear_search();
            return;
        }
        let started_generation = self.generation;
        self.search_seq += 1;
        let seq = self.search_seq;
        self.search_query = query.clone();
        self.set_search_status("Searching…");
        let reply = match self.request(
            json!({ "op": "search", "query": query, "maxHits": MAX_HITS_DEFAULT }),
            Vec::new(),
        ) {
            Ok(reply) => reply,
            Err(err) => {
                if seq == self.search_seq && started_generation == self.generation {
                    self.set_search_status(&format!("Search failed: {}", user_text(&err.message)));
                }
                return;
            }
        };
        // A newer query, or a closed document, owns the UI now.
        if seq != self.search_seq
            || started_generation != self.generation
            || query != self.search_query
        {
            return;
        }
        let reply: SearchReply = serde_json::from_value(reply.value).unwrap_or_default();
        self.search_hits = reply.hits;
        self.search_truncated = reply.truncated;
        self.search_index = if self.search_hits.is_empty() { -1 } else { 0 };
        if self.search_index == 0 {
            let page = self.search_hits[0].page;
            self.go_to(page);
        }
        self.repaint();
        self.update_search_status();
        self.sync_markup_buttons();
    }

    /// R:5398: the ‹ and › buttons (and Enter / Shift+Enter); wraps at both ends.
    pub fn move_hit(&mut self, delta: i64) {
        if self.search_hits.is_empty() {
            return;
        }
        let count = self.search_hits.len() as i64;
        self.search_index = (self.search_index + delta + count).rem_euclid(count);
        let page = self.search_hits[self.search_index as usize].page;
        self.go_to(page);
        self.repaint();
        self.update_search_status();
    }

    /// R:5406.
    pub fn clear_search(&mut self) {
        self.search_seq += 1; // any in-flight reply is for the old query
        self.search_hits.clear();
        self.search_index = -1;
        self.search_query.clear();
        self.search_truncated = false;
        self.set_search_status("");
        self.repaint();
        self.sync_controls(); // no match means no markup to add
    }

    /// R:5417.
    pub fn update_search_status(&mut self) {
        if self.search_query.is_empty() {
            return;
        }
        if self.search_hits.is_empty() {
            self.set_search_status("No matches");
            return;
        }
        let text = search_status_text(
            self.search_index,
            self.search_hits.len(),
            self.search_truncated,
        );
        self.set_search_status(&text);
    }

    /// R:5427.
    pub fn set_search_status(&mut self, text: &str) {
        self.dom.set_text(SEARCH_STATUS_EL, text);
    }
}

/// R:5423: "3 of 12", or "3 of 500 — showing first 500 matches" when the engine stopped early.
fn search_status_text(index: i64, count: usize, truncated: bool) -> String {
    let position = format!("{} of {count}", index + 1);
    if truncated {
        format!("{position} — showing first {count} matches")
    } else {
        position
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_words() {
        assert_eq!(search_status_text(2, 12, false), "3 of 12");
        assert_eq!(
            search_status_text(0, 500, true),
            "1 of 500 — showing first 500 matches"
        );
    }
}
