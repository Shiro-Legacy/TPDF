//! Compare, contract-compare.md (renderer.ts 8991–9388): the compare slot's open, close and
//! refresh, the line and pixel diff, the drive's `compare`, STATE `compare`; then the insert
//! dialog, `insert-pages`, the extract, and the `assemble`/`extract` smokes that follow it.
#![allow(dead_code)]

use super::Session;
use super::dom_ids as ids;
use super::field_designer::{js_to_number, refusal_detail};
use super::markup::{ComparePaint, CompareRow, collapse_spaces};
use super::types::*;
use crate::engine::{Refusal, Reply};
use crate::host::SaveOptions;
use crate::js::{is_js_space, js_num, js_round, js_trim};
use crate::messages;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use tandem_protocol::engine::{CompareTextReply, PageReply};
use tandem_protocol::limits::MAX_PIXEL_EDGE;

/// engine-shared.ts `normaliseLine`: §9.7: trim and collapse runs of whitespace.
pub fn normalise_line(text: &str) -> String {
    collapse_spaces(js_trim(text))
}

/// engine-shared.ts `parsePageRange`: "1-3, 5" as 0-based pages, ascending and unique; an
/// empty text is every page; None is "not a range I can use".
pub fn parse_page_range(text: &str, page_count: i64) -> Option<Vec<i64>> {
    if page_count < 0 {
        return None;
    }
    let trimmed = js_trim(text);
    if trimmed.is_empty() {
        return Some((0..page_count).collect());
    }
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let number = |s: &str| s.parse::<f64>().unwrap_or(f64::NAN);
    let mut wanted = BTreeSet::new();
    for item in trimmed.split(',') {
        let token = js_trim(item);
        // /^(\d*)\s*-\s*(\d*)$/
        let lead = token.bytes().take_while(u8::is_ascii_digit).count();
        let range = token[lead..]
            .trim_start_matches(is_js_space)
            .strip_prefix('-')
            .map(|rest| {
                let rest = rest.trim_start_matches(is_js_space);
                (&token[..lead], rest)
            });
        let (mut first, mut last) = match range {
            Some((a, b)) if digits(b) && (!a.is_empty() || !b.is_empty()) => (
                if a.is_empty() { 1.0 } else { number(a) },
                if b.is_empty() {
                    page_count as f64
                } else {
                    number(b)
                },
            ),
            _ if !token.is_empty() && digits(token) => (number(token), number(token)),
            _ => return None,
        };
        if first > last {
            std::mem::swap(&mut first, &mut last);
        }
        if first < 1.0 || last > page_count as f64 {
            return None;
        }
        for page in first as i64..=last as i64 {
            wanted.insert(page - 1);
        }
    }
    Some(wanted.into_iter().collect())
}

/// R:9308: `stem pages 2-4.pdf`, the page list compacted into ascending ranges.
pub fn extract_file_name(name: &str, pages: &[i64]) -> String {
    let lower = name.to_ascii_lowercase();
    let stem = if lower.ends_with(".pdf") {
        &name[..name.len() - 4]
    } else {
        name
    };
    let stem = if stem.is_empty() { "document" } else { stem };
    let sorted: Vec<i64> = pages
        .iter()
        .map(|page| page + 1)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut parts = Vec::new();
    let mut index = 0;
    while index < sorted.len() {
        let start = sorted[index];
        let mut end = start;
        while index + 1 < sorted.len() && sorted[index + 1] == end + 1 {
            index += 1;
            end = sorted[index];
        }
        parts.push(if start == end {
            start.to_string()
        } else {
            format!("{start}-{end}")
        });
        index += 1;
    }
    format!("{stem} pages {}.pdf", parts.join(", "))
}

/// `x.toFixed(1)` for a non-negative x: an exact tie rounds up, as JS does (Rust rounds it to even).
fn to_fixed1(x: f64) -> String {
    let exact = format!("{x:.40}");
    let tie = exact.split_once('.').is_some_and(|(_, frac)| {
        frac.len() > 2 && frac.as_bytes()[1] == b'5' && frac[2..].bytes().all(|b| b == b'0')
    });
    if !tie || x < 0.0 {
        return format!("{x:.1}");
    }
    let n = (x * 10.0).floor() as i64 + 1;
    format!("{}.{}", n / 10, n % 10)
}

/// D3/§9.8: the differing pixels of two RGBA buffers on a maxW × maxH grid (white outside a
/// page), and B's overlay: each differing pixel inside B red at 60 %.
pub fn diff_pixels(a: (&[u8], i64, i64), b: (&[u8], i64, i64)) -> (i64, Vec<u8>) {
    let (pa, aw, ah) = a;
    let (pb, bw, bh) = b;
    let (width, height) = (aw.max(bw), ah.max(bh));
    let mut overlay = vec![0u8; (bw.max(0) * bh.max(0) * 4) as usize];
    // An index past a short buffer reads `undefined` in JS: the delta is NaN, never "differing".
    let at = |pixels: &[u8], index: i64| {
        pixels
            .get(index as usize)
            .map_or(f64::NAN, |v| f64::from(*v))
    };
    let mut differing = 0;
    for y in 0..height {
        for x in 0..width {
            let in_a = x < aw && y < ah;
            let in_b = x < bw && y < bh;
            let ia = (y * aw + x) * 4;
            let ib = (y * bw + x) * 4;
            let channel = |k: i64| {
                let va = if in_a { at(pa, ia + k) } else { 255.0 };
                let vb = if in_b { at(pb, ib + k) } else { 255.0 };
                (va - vb).abs()
            };
            let delta = channel(0) + channel(1) + channel(2);
            if delta > 48.0 {
                differing += 1;
                if in_b {
                    overlay[ib as usize] = 255;
                    overlay[ib as usize + 3] = 153;
                }
            }
        }
    }
    (differing, overlay)
}

/// A render reply with its pixels.
fn render_of(mut reply: Reply) -> RenderReply {
    let pixels = reply.take_bytes("pixels").unwrap_or_default();
    let mut parsed: RenderReply = super::parse_reply_or_default(&reply.value);
    parsed.pixels = pixels;
    parsed
}

impl Session {
    /// R:8994: "Compare with…" (⌥⌘K) and the drive's `compare` with a line-level `from`.
    pub fn request_compare(&mut self) {
        self.clear_error();
        if let Some(picked) = self.pick_insert_source("compare") {
            self.open_compare(picked.bytes, &picked.name);
        }
    }

    /// R:9002: the bytes go to the engine; no copy is kept (§9.18).
    pub fn open_compare(&mut self, bytes: Vec<u8>, name: &str) {
        let reply = match self.request(
            json!({"op": "compare-open", "pdf": {"$buffer": 0}, "name": name}),
            vec![bytes],
        ) {
            Ok(reply) => reply,
            Err(err) => {
                // A refusal keeps the previous slot (§9.7).
                self.compare_last = Some(CompareLast {
                    action: CompareAction::Open,
                    ok: false,
                    detail: Some(err.reason.clone().unwrap_or_else(|| "unreadable".into())),
                });
                let message = self.message_of(&err);
                self.show_error(&format!(
                    "Could not compare with {name}: {}",
                    messages::user_text(&message)
                ));
                return;
            }
        };
        let reply: tandem_protocol::engine::CompareOpenReply =
            super::parse_reply_or_default(&reply.value);
        self.compare_doc = Some(CompareDoc {
            name: reply.name,
            page_count: reply.page_count,
            pages: reply.pages,
            page_a: self.wanted_page,
            page_b: 0,
            text: None,
            pixels: None,
        });
        self.compare_b = None;
        self.compare_last = Some(CompareLast {
            action: CompareAction::Open,
            ok: true,
            detail: None,
        });
        self.clear_error(); // §9.19(c)
        self.set_compare_show_diff(false); // §9.13: unchecked on every open
        self.dom.set_attr(
            ids::COMPARE_PAGE_BINPUT,
            "max",
            reply.page_count.to_string(),
        );
        self.dom.set_hidden(ids::READER_EL, true);
        self.dom.set_hidden(ids::COMPARE_VIEW, false);
        self.refresh_compare(true, true);
    }

    /// R:9025: drops the compare state and shows #reader again; the engine slot is the caller's business.
    pub fn clear_compare(&mut self) {
        self.compare_gen += 1;
        self.compare_doc = None;
        self.compare_b = None;
        self.set_compare_show_diff(false);
        self.dom.set_text(ids::COMPARE_SUMMARY_EL, "");
        self.lane4.compare_rows.clear();
        self.lane4.compare_paint = None;
        if !self.dom.hidden(ids::COMPARE_VIEW) {
            self.dom.set_hidden(ids::COMPARE_VIEW, true);
            self.dom.set_hidden(ids::READER_EL, false);
            self.sync_zoom();
            self.drain();
        }
    }

    /// R:9041.
    pub fn close_compare(&mut self) {
        // A dead engine already dropped the slot.
        let _ = self.request(json!({"op": "compare-close"}), vec![]);
        self.clear_compare();
        self.compare_last = Some(CompareLast {
            action: CompareAction::Close,
            ok: true,
            detail: None,
        });
        self.announce("Compare closed");
    }

    /// R:9052.
    pub fn set_compare_show_diff(&mut self, on: bool) {
        self.compare_show_diff = on;
        self.dom.set_checked(ids::COMPARE_SHOW_DIFF_INPUT, on);
        self.dom.set_hidden(ids::COMPARE_OVERLAY, !on);
    }

    /// R:9059: §9.12: a hand-set page of B (0-based), clamped; holds until A's page changes.
    pub fn set_compare_page_b(&mut self, page: f64) {
        let Some(c) = self.compare_doc.as_mut() else {
            return;
        };
        c.page_b = (page.trunc().max(0.0) as i64).min(c.page_count - 1);
        self.refresh_compare(false, true);
    }

    /// R:9069: runs at the end of goTo/applyEdit/applyPageEdit/openDocument while compare is
    /// open. `speak` false (after an edit) keeps that edit's own announcement.
    pub fn refresh_compare(&mut self, reset_b: bool, speak: bool) {
        let wanted = self.wanted_page;
        let Some(c) = self.compare_doc.as_mut() else {
            return;
        };
        self.compare_gen += 1;
        let turn = self.compare_gen;
        if reset_b || wanted != c.page_a {
            c.page_a = wanted;
            c.page_b = wanted.min(c.page_count - 1);
        }
        let (page_b, name_b) = (c.page_b, c.name.clone());
        self.dom
            .set_value(ids::COMPARE_PAGE_BINPUT, (page_b + 1).to_string());
        // §9.19(g): each side names its file.
        let name_a = self
            .doc
            .as_ref()
            .map(|doc| doc.name.clone())
            .unwrap_or_default();
        self.dom.set_text(ids::COMPARE_NAME_A, name_a);
        self.dom.set_text(ids::COMPARE_NAME_B, name_b);
        if let Err(err) = self.run_compare(turn, speak)
            && turn == self.compare_gen
        {
            let message = self.message_of(&err);
            self.show_error(&message);
        }
    }

    /// The page cache's `get` (engine-shared.ts `createRenderCache`).
    fn cached_render(&self, key: &str) -> Option<RenderReply> {
        self.cache
            .entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }

    /// The page cache's `set`: oldest out first; the newest entry always stays.
    fn cache_render(&mut self, key: String, value: RenderReply) {
        let cache = &mut self.cache;
        if let Some(index) = cache.entries.iter().position(|(k, _)| *k == key)
            && let Some((_, old)) = cache.entries.remove(index)
        {
            cache.bytes -= old.pixels.len();
        }
        cache.bytes += value.pixels.len();
        cache.entries.push_back((key, value));
        while cache.entries.len() > 1
            && (cache.entries.len() > cache.limit_entries || cache.bytes > cache.limit_bytes)
        {
            if let Some((_, evicted)) = cache.entries.pop_front() {
                cache.bytes -= evicted.pixels.len();
            }
        }
    }

    /// R:9087.
    fn run_compare(&mut self, turn: i64, speak: bool) -> Result<(), Refusal> {
        let locked = self.doc.as_ref().is_none_or(|doc| doc.info.needs_password);
        let (Some(doc), Some(c)) = (
            self.doc.clone().filter(|_| !locked),
            self.compare_doc.clone(),
        ) else {
            // §9.19(h): a locked main shows nothing of the previous document.
            if let Some(c) = self.compare_doc.as_mut() {
                c.text = None;
                c.pixels = None;
            }
            self.dom.set_text(ids::COMPARE_SUMMARY_EL, "");
            self.lane4.compare_rows.clear();
            self.lane4.compare_paint = None;
            return Ok(());
        };
        let page_a = c.page_a.min(doc.page_count - 1);
        let page_b = c.page_b;
        let missing = || Refusal::plain("Cannot read properties of undefined (reading 'width')");
        let sa = usize::try_from(page_a)
            .ok()
            .and_then(|p| doc.pages.get(p))
            .ok_or_else(missing)?;
        let sb = usize::try_from(page_b)
            .ok()
            .and_then(|p| c.pages.get(p))
            .ok_or_else(missing)?;
        // §9.8: one deterministic scale from both pages' pt sizes (no zoom, no dpr).
        let largest = sa.width.max(sa.height).max(sb.width).max(sb.height);
        let scale = 1f64.min((MAX_PIXEL_EDGE as f64 / largest * 100.0).floor() / 100.0);
        let key_a = format!("{page_a}:{}:{}", js_num(scale), self.revision);
        let a = match self.cached_render(&key_a) {
            Some(a) => a,
            None => render_of(self.request(
                json!({"op": "render", "page": page_a, "scale": scale}),
                vec![],
            )?),
        };
        if turn != self.compare_gen {
            return Ok(());
        }
        if a.revision == self.revision {
            self.cache_render(key_a, a.clone());
        }
        let key_b = format!("cmp:{page_b}:{}", js_num(scale));
        let b = match &self.compare_b {
            Some(cached) if cached.key == key_b => cached.reply.clone(),
            _ => {
                let mut reply = self.request(
                    json!({"op": "compare-render", "page": page_b, "scale": scale}),
                    vec![],
                )?;
                let pixels = reply.take_bytes("pixels").unwrap_or_default();
                let mut parsed: CompareRenderReply =
                    super::parse_reply_or_default(&reply.value);
                parsed.pixels = pixels;
                parsed
            }
        };
        if turn != self.compare_gen {
            return Ok(());
        }
        self.compare_b = Some(CompareB {
            key: key_b,
            reply: b.clone(),
        });
        let mut rows = Vec::new();
        let text = match self.request(
            json!({"op": "compare-text", "pageA": page_a, "pageB": page_b}),
            vec![],
        ) {
            Ok(reply) => {
                let diff: CompareTextReply =
                    super::parse_reply_or_default(&reply.value);
                for (side, page, lines) in [
                    ("Only left", page_a, &diff.removed),
                    ("Only right", page_b, &diff.added),
                ] {
                    for line in lines {
                        let bbox: Vec<String> =
                            line.bbox.iter().map(|v| js_num(js_round(*v))).collect();
                        rows.push(CompareRow {
                            side: side.into(),
                            text: line.text.clone(),
                            title: format!("Page {}, {} pt", page + 1, bbox.join(" ")),
                        });
                    }
                }
                CompareText {
                    same: diff.same,
                    removed: diff.removed.len() as i64,
                    added: diff.added.len() as i64,
                    refused: None,
                    removed_lines: diff
                        .removed
                        .iter()
                        .take(50)
                        .map(|line| normalise_line(&line.text))
                        .collect(),
                    added_lines: diff
                        .added
                        .iter()
                        .take(50)
                        .map(|line| normalise_line(&line.text))
                        .collect(),
                }
            }
            Err(err) => {
                if err.reason_str().is_empty() {
                    return Err(err);
                }
                CompareText {
                    refused: err.reason.clone(),
                    ..CompareText::default()
                }
            }
        };
        if turn != self.compare_gen {
            return Ok(());
        }
        let pixels = self.paint_compare(&a, &b, scale);
        if let Some(c) = self.compare_doc.as_mut() {
            c.text = Some(text.clone());
            c.pixels = Some(pixels.clone());
        }
        self.lane4.compare_rows = rows;
        let percent = format!("{} % of pixels differ", to_fixed1(pixels.ratio * 100.0));
        // §9.19(f): each refusal reason in its own words.
        let summary = match text.refused.as_deref() {
            Some(refused) => {
                let words = match refused {
                    "too-many" => "more than 2000 lines on a page, only pixels compared",
                    "read-only" => "copying is not permitted, only pixels compared",
                    _ => "the text could not be compared, only how the pages look",
                };
                format!("Page {} vs {}: {words}; {percent}", page_a + 1, page_b + 1)
            }
            None => format!(
                "Page {} vs {}: {} lines same, {} removed, {} added; {percent}",
                page_a + 1,
                page_b + 1,
                text.same,
                text.removed,
                text.added
            ),
        };
        self.dom.set_text(ids::COMPARE_SUMMARY_EL, summary.clone());
        if speak {
            self.announce(&summary);
        }
        Ok(())
    }

    /// R:9161: diff A and B, and put A, B and the overlay on the view's canvases.
    /// ponytail: no anti-aliasing tolerance beyond the threshold, no alignment search.
    pub fn paint_compare(
        &mut self,
        a: &RenderReply,
        b: &CompareRenderReply,
        scale: f64,
    ) -> ComparePixels {
        let (differing, overlay) = diff_pixels(
            (&a.pixels, a.pixel_width, a.pixel_height),
            (&b.pixels, b.pixel_width, b.pixel_height),
        );
        self.lane4.compare_paint = Some(ComparePaint {
            a: (a.pixel_width, a.pixel_height, a.pixels.clone()),
            b: (b.pixel_width, b.pixel_height, b.pixels.clone()),
            overlay,
        });
        // §9.19(e): the overlay is B's grid; differing/total still count maxW × maxH.
        let total = a.pixel_width.max(b.pixel_width) * a.pixel_height.max(b.pixel_height);
        let ratio = js_round(differing as f64 / total as f64 * 10000.0) / 10000.0;
        ComparePixels {
            differing,
            total,
            ratio,
            scale,
        }
    }

    /// R:9201: drive `compare`: `{close}`, `{pageB}`, `{page}` or `{showDiff}`.
    pub fn drive_compare(&mut self, contents: &str) {
        self.clear_error();
        let Some(wanted) = self.drive_wanted(Some(contents)) else {
            return;
        };
        if wanted.get("close") == Some(&json!(true)) {
            self.close_compare();
            return;
        }
        if self.compare_doc.is_none() {
            self.show_error("compare: no compare document is open");
            return;
        }
        if let Some(on) = wanted.get("showDiff").and_then(Value::as_bool) {
            self.set_compare_show_diff(on);
            return;
        }
        if let Some(page) = wanted.get("pageB") {
            match page.as_f64().filter(|page| page.fract() == 0.0) {
                Some(page) => self.set_compare_page_b(page),
                None => self.show_error("compare: pageB must be an integer"),
            }
            return;
        }
        if let Some(page) = wanted.get("page") {
            let contents = json!({"page": page}).to_string();
            self.drive_goto(Some(&contents));
            return;
        }
        self.show_error("compare: contents must be {close}, {pageB}, {page} or {showDiff}");
    }

    /// R:9229: STATE `compare`.
    pub fn compare_summary(&self) -> Value {
        let c = self.compare_doc.as_ref();
        json!({
            "open": c.is_some(),
            "name": c.map(|c| c.name.clone()),
            "pageCount": c.map(|c| c.page_count),
            "pageA": c.map_or(0, |c| c.page_a),
            "pageB": c.map_or(0, |c| c.page_b),
            "text": c.and_then(|c| c.text.clone()),
            "pixels": c.and_then(|c| c.pixels.clone()),
            "showDiff": self.compare_show_diff,
            "last": self.compare_last.as_ref().map(|last| {
                let mut out = json!({"action": last.action.as_str(), "ok": last.ok});
                if let Some(detail) = &last.detail {
                    out["detail"] = json!(detail);
                }
                out
            }),
        })
    }

    /// R:9243.
    pub fn open_insert_dialog(&mut self, source: InsertSource) {
        self.dom.set_text(
            ids::INSERT_SOURCE_EL,
            format!("Insert pages from {}", source.name),
        );
        self.insert_source = Some(source);
        // The number is the 1-based page the new pages follow; 0 puts them first.
        self.dom
            .set_value(ids::INSERT_AFTER_INPUT, (self.wanted_page + 1).to_string());
        let max = self.doc.as_ref().map_or(1, |doc| doc.page_count);
        self.dom
            .set_attr(ids::INSERT_AFTER_INPUT, "max", max.to_string());
        self.dom.set_value(ids::INSERT_RANGE_INPUT, "");
        self.dom.set_open(ids::INSERT_DIALOG, true);
    }

    /// R:9255.
    pub fn save_insert_dialog(&mut self) -> bool {
        let (Some(count), Some(source)) = (
            self.doc.as_ref().map(|doc| doc.page_count),
            self.insert_source.clone(),
        ) else {
            self.dom.set_open(ids::INSERT_DIALOG, false);
            self.insert_source = None; // the dialog's close listener (R:10933)
            return false;
        };
        let at = js_to_number(&self.dom.value(ids::INSERT_AFTER_INPUT));
        if at.fract() != 0.0 || !at.is_finite() || at < 0.0 || at > count as f64 {
            self.show_error(&format!(
                "Enter a page number between 0 (before the first) and {count}"
            ));
            return false;
        }
        self.dom.set_open(ids::INSERT_DIALOG, false);
        self.insert_source = None; // the dialog's close listener (R:10933)
        let range = self.dom.value(ids::INSERT_RANGE_INPUT);
        self.insert_pages(&source, at as i64, &range)
    }

    /// R:9272: one `insert-pages`; the range text is parsed by the engine, which knows the source count.
    pub fn insert_pages(&mut self, source: &InsertSource, at: i64, range: &str) -> bool {
        let request = json!({"op": "insert-pages", "pdf": {"$buffer": 0}, "name": source.name, "at": at, "range": range});
        match self.request(request, vec![source.bytes.clone()]) {
            Ok(reply) => {
                let inserted = reply.get("inserted").and_then(Value::as_i64).unwrap_or(0);
                let field_names: Vec<String> = reply
                    .get("fieldNames")
                    .and_then(super::parse_reply)
                    .unwrap_or_default();
                let page_reply: PageReply = super::parse_reply_or_default(&reply.value);
                self.assemble_summary = Some(AssembleSummary {
                    inserted: Some(inserted),
                    source: Some(source.name.clone()),
                    field_names: Some(field_names),
                    last: Some(LastAction {
                        action: "insert".into(),
                        ok: true,
                        detail: None,
                    }),
                });
                let page = at.max(0).min(page_reply.page_count - 1).max(0) as usize;
                self.apply_page_edit(&page_reply, "", page);
                let text = match (inserted, at) {
                    (1, 0) => "Inserted 1 page before page 1".to_string(),
                    (1, _) => format!("Inserted 1 page after page {at}"),
                    (_, 0) => format!("Inserted {inserted} pages before page 1"),
                    _ => format!("Inserted {inserted} pages after page {at}"),
                };
                self.announce_page(&text);
                true
            }
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
                // A2 §8.15: a refusal leaves the summary (created if needed) with only its `last`.
                let mut summary = self.assemble_summary.take().unwrap_or_default();
                summary.last = Some(LastAction {
                    action: "insert".into(),
                    ok: false,
                    detail: Some(refusal_detail(&err)),
                });
                self.assemble_summary = Some(summary);
                false
            }
        }
    }

    /// R:9293: "Extract pages…": ask which pages, then write them to a new file.
    pub fn request_extract(&mut self) {
        if self.doc.as_ref().is_none_or(|doc| doc.info.needs_password) {
            return;
        }
        self.dom
            .set_value(ids::EXTRACT_RANGE_INPUT, (self.wanted_page + 1).to_string());
        self.dom.set_open(ids::EXTRACT_DIALOG, true);
    }

    /// R:9301.
    pub fn save_extract_dialog(&mut self) {
        let text = self.dom.value(ids::EXTRACT_RANGE_INPUT);
        self.dom.set_open(ids::EXTRACT_DIALOG, false);
        self.extract_pages(&text);
    }

    /// R:9324: one `extract-pages` plus the save; the open document's own bytes never change.
    pub fn extract_pages(&mut self, range: &str) -> bool {
        let Some(doc) = self.doc.clone() else {
            return false;
        };
        let pages = match parse_page_range(range, doc.page_count) {
            Some(pages) if !pages.is_empty() => pages,
            _ => {
                self.show_error(&format!(
                    "Pages must look like \"1-3, 5\" (this document has {})",
                    doc.page_count
                ));
                return false;
            }
        };
        let Some(url) = doc.url.clone() else {
            self.show_error(messages::open::SAMPLE);
            return false;
        };
        // Redaction §2.3 step 7: extracted pages still show everything under unredacted boxes.
        if !self.confirm_pending_save(PendingAction::Export) {
            return false;
        }
        self.extract_summary = None;
        let name = extract_file_name(&doc.name, &pages);
        let bytes = match self
            .engine
            .request(json!({"op": "extract-pages", "pages": pages}), vec![])
        {
            Ok(mut reply) => reply.take_bytes("bytes").filter(|bytes| bytes.len() >= 5),
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
                return false;
            }
        };
        let Some(bytes) = bytes else {
            self.show_error("extract-pages: no bytes");
            return false;
        };
        let options = SaveOptions {
            suggested_name: Some(name),
            ..SaveOptions::default()
        };
        match self.host.save_file(&bytes, &url, &options) {
            Ok(saved) => {
                self.extract_summary = Some(ExtractSummary {
                    pages: pages.clone(),
                    bytes: bytes.len() as i64,
                });
                let count = if pages.len() == 1 {
                    "1 page".to_string()
                } else {
                    format!("{} pages", pages.len())
                };
                self.announce_page(&format!("Extracted {count} to {}", saved.name));
                true
            }
            Err(fail) => {
                let reason = fail.as_str();
                let text = if reason == "cancelled" {
                    String::new()
                } else {
                    format!("The extract was not saved ({reason}).")
                };
                self.show_error(&text);
                if reason == "cancelled" {
                    self.clear_error();
                }
                false
            }
        }
    }

    /// R:9366: `--edit=assemble`: insert every page of the picked source after page 1 and log.
    pub fn smoke_assemble(&mut self) {
        if self.doc.is_none() {
            self.log_edit();
            return;
        }
        if let Some(source) = self.pick_insert_source("insert") {
            self.insert_pages(&source, 1, "");
        }
        self.log_edit();
    }

    /// R:9380: `--edit=extract`: page 2 on its own, written out and logged.
    pub fn smoke_extract(&mut self) {
        let Some(count) = self.doc.as_ref().map(|doc| doc.page_count) else {
            self.log_edit();
            return;
        };
        self.extract_pages(&count.min(2).to_string());
        self.log_edit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_ranges_parse_like_engine_shared() {
        assert_eq!(parse_page_range("", 3), Some(vec![0, 1, 2]));
        assert_eq!(parse_page_range("1-3, 5", 5), Some(vec![0, 1, 2, 4]));
        assert_eq!(parse_page_range("3-1", 3), Some(vec![0, 1, 2]));
        assert_eq!(parse_page_range("-2", 5), Some(vec![0, 1]));
        assert_eq!(parse_page_range("4 - ", 5), Some(vec![3, 4]));
        assert_eq!(parse_page_range("2,2", 5), Some(vec![1]));
        assert_eq!(parse_page_range("0", 5), None);
        assert_eq!(parse_page_range("6", 5), None);
        assert_eq!(parse_page_range("-", 5), None);
        assert_eq!(parse_page_range("a", 5), None);
        assert_eq!(parse_page_range("1,,2", 5), None);
    }

    #[test]
    fn extract_names_compact_ranges() {
        assert_eq!(
            extract_file_name("nav.PDF", &[1, 2, 3, 5, 3]),
            "nav pages 2-4, 6.pdf"
        );
        assert_eq!(extract_file_name(".pdf", &[0]), "document pages 1.pdf");
    }

    #[test]
    fn fixed_and_lines() {
        assert_eq!(to_fixed1(0.25), "0.3");
        assert_eq!(to_fixed1(12.34), "12.3");
        assert_eq!(to_fixed1(0.0), "0.0");
        assert_eq!(to_fixed1(100.0), "100.0");
        assert_eq!(normalise_line("  a \t b "), "a b");
    }

    #[test]
    fn pixel_diff_counts_the_union_grid() {
        // A: 1x1 black; B: 2x1 white. (0,0) differs, (1,0) is white on both sides.
        let a = [0u8, 0, 0, 255];
        let b = [255u8; 8];
        let (differing, overlay) = diff_pixels((&a, 1, 1), (&b, 2, 1));
        assert_eq!(differing, 1);
        assert_eq!(overlay, vec![255, 0, 0, 153, 0, 0, 0, 0]);
        let mut session = Session::new();
        let ra = RenderReply {
            pixel_width: 1,
            pixel_height: 1,
            pixels: a.to_vec(),
            ..RenderReply::default()
        };
        let rb = CompareRenderReply {
            pixel_width: 2,
            pixel_height: 1,
            pixels: b.to_vec(),
            ..CompareRenderReply::default()
        };
        let pixels = session.paint_compare(&ra, &rb, 1.0);
        assert_eq!((pixels.differing, pixels.total, pixels.ratio), (1, 2, 0.5));
    }

    #[test]
    fn closed_compare_summary_shape() {
        let session = Session::new();
        assert_eq!(
            session.compare_summary(),
            json!({"open": false, "name": null, "pageCount": null, "pageA": 0, "pageB": 0, "text": null, "pixels": null, "showDiff": false, "last": null})
        );
    }
}
