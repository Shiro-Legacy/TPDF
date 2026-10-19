//! Decoration (renderer.ts R:9533-9849, unit 8): page labels, optional-content layers, the
//! watermark / header / footer stamp dialog with Bates numbering, the labels dialog, the
//! decorate smoke and `logEdit`; plus the note / text-box dialog (R:10045-10088).

use super::Session;
use super::dom_ids::*;
use super::types::*;
use crate::engine::Refusal;
use crate::js::{js_num, js_trim};
use serde_json::{Value, json};
use tandem_protocol::engine::JournalState;

/// `parsePageRange` (engine-shared.ts:189): the engine's own port.
pub(crate) fn parse_page_range(text: &str, count: i64) -> Option<Vec<usize>> {
    if count < 0 {
        return None;
    }
    tandem_pdf_engine::app::ops::pages::parse_page_range(text, count as usize)
}

/// `/^\d{1,n}$/`: JS `\d` is ASCII digits only.
pub(crate) fn digits(text: &str, max: usize) -> bool {
    (1..=max).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit())
}

/// The Bates fields' check, shared by the stamp dialog and Batch (§9.11): a start of 1-15
/// digits and a pad of 1-2 digits within 1..15.
pub(crate) fn bates_fields_ok(start: &str, pad: &str) -> bool {
    digits(start, 15) && digits(pad, 2) && (1..=15).contains(&pad.parse::<i64>().unwrap_or(0))
}

/// `String(n).padStart(pad, "0")`.
pub(crate) fn pad_number(n: i64, pad: i64) -> String {
    let text = n.to_string();
    let width = pad.max(0) as usize;
    if text.len() >= width {
        text
    } else {
        format!("{}{text}", "0".repeat(width - text.len()))
    }
}

/// The journal slice of a mutation reply.
pub(crate) fn journal_of(reply: &Value) -> JournalState {
    super::parse_reply_or_default(reply)
}

/// `reply.items` as labels.
fn label_items(reply: &Value) -> Vec<String> {
    reply
        .get("items")
        .and_then(super::parse_reply)
        .unwrap_or_default()
}

fn layer_items(reply: &Value) -> Vec<LayerItem> {
    reply
        .get("items")
        .and_then(super::parse_reply)
        .unwrap_or_default()
}

impl Session {
    /// R:9540: the page labels, fetched on open and after anything that can move them: a
    /// labels op, a page op, or an undo/redo of either. The worker's sequence and generation
    /// guards are gone: the engine answers at once.
    pub fn refresh_labels(&mut self) {
        self.labels_seq += 1;
        if self.doc.as_ref().is_none_or(|doc| doc.info.needs_password) {
            self.labels.clear();
            self.apply_label_text();
            return;
        }
        let Ok(reply) = self.request(json!({ "op": "labels" }), vec![]) else {
            return;
        };
        self.labels = label_items(&reply.value);
        self.apply_label_text();
    }

    /// R:9560: `""` when the label is just the 1-based index, `" (iii)"` when the document says more.
    pub fn label_suffix(&self, page: usize) -> String {
        match self.labels.get(page) {
            Some(label) if !label.is_empty() && *label != (page + 1).to_string() => {
                format!(" ({label})")
            }
            _ => String::new(),
        }
    }

    /// R:9566: wherever a page number is shown, the label joins it when it says something new.
    pub fn apply_label_text(&mut self) {
        let page = self.wanted_page.max(0) as usize;
        let suffix = if self.doc.is_some() {
            self.label_suffix(page)
        } else {
            String::new()
        };
        self.dom.set_hidden(PAGE_LABEL_EL, suffix.is_empty());
        self.dom.set_text(PAGE_LABEL_EL, js_trim(&suffix));
        let (aria, caption) = match &self.doc {
            None => ("Page".to_string(), "No page".to_string()),
            Some(doc) => {
                let text = format!(
                    "Page {}{suffix} of {}",
                    self.wanted_page + 1,
                    doc.page_count
                );
                let caption = if doc.info.needs_password {
                    "Password required".to_string()
                } else {
                    text.clone()
                };
                (text, caption)
            }
        };
        self.dom.set_attr(PAGE_INPUT, "aria-label", aria);
        self.dom.set_text(CAPTION, caption);
    }

    /// R:9577: the optional-content layers; hidden entirely when the document has none.
    pub fn refresh_layers(&mut self) {
        self.layers_seq += 1;
        if self.doc.as_ref().is_none_or(|doc| doc.info.needs_password) {
            self.layers.clear();
            self.render_layers();
            return;
        }
        let Ok(reply) = self.request(json!({ "op": "layers" }), vec![]) else {
            return;
        };
        self.layers = layer_items(&reply.value);
        self.render_layers();
    }

    /// R:9596. The list itself (one checkbox per layer, checked = visible, disabled while no
    /// document or locked) is drawn by the UI from `layers` and [`Session::layer_boxes_disabled`].
    pub fn render_layers(&mut self) {
        let count = self.layers.len();
        self.dom.set_hidden(LAYERS_EL, count == 0);
        self.dom
            .set_hidden(LAYERS_NOTE_EL, self.layers_persisted || count == 0);
        self.set_empty(
            LAYERS_EMPTY_EL,
            count,
            "This document has no optional-content layers.",
            "No document open yet.",
        );
    }

    /// R:9610: `box.disabled` of every layer checkbox.
    pub fn layer_boxes_disabled(&self) -> bool {
        self.doc.as_ref().is_none_or(|doc| doc.info.needs_password)
    }

    /// R:9624: a view-only toggle (measured): the engine changes the in-memory document and
    /// writes nothing, so the caches are cleared and the page repainted rather than
    /// pretending the document changed. A layer checkbox's `change` calls this.
    pub fn set_layer(&mut self, layer_id: i64, visible: bool) {
        let reply = match self.request(
            json!({ "op": "layer-set", "layerId": layer_id, "visible": visible }),
            vec![],
        ) {
            Ok(reply) => reply,
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
                self.refresh_layers();
                return;
            }
        };
        self.layers = layer_items(&reply.value);
        let persisted = reply
            .get("persisted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.layers_persisted = persisted;
        self.render_layers();
        let name = self
            .layers
            .iter()
            .find(|entry| entry.id == layer_id)
            .map_or_else(|| js_num(layer_id as f64 + 1.0), |item| item.name.clone());
        let text = format!(
            "Layer {name} {}{}",
            if visible { "shown" } else { "hidden" },
            if persisted {
                ""
            } else {
                " (view only, not saved)"
            }
        );
        self.announce_page(&text);
        self.cache.entries.clear();
        self.cache.bytes = 0;
        // drain() skips a page whose key is already shown; the toggle changed its pixels.
        self.shown_key = None;
        self.thumb_cache.entries.clear();
        self.thumb_cache.bytes = 0;
        self.thumb_painted.clear();
        self.invalidate_thumb(self.wanted_page.max(0));
        self.drain();
    }

    /// R:9645: "Watermark…" and "Header/footer…" share one dialog; the kind picker picks it.
    pub fn open_stamp_dialog(&mut self, kind: Option<&str>) {
        if self.doc.is_none() || !self.can_edit() {
            return;
        }
        if let Some(kind) = kind {
            self.dom.set_value(STAMP_KIND_SELECT, kind);
        }
        self.sync_stamp_heading();
        let text = if self.dom.value(STAMP_KIND_SELECT) == "watermark" {
            "DRAFT"
        } else {
            "Page {page} of {pages}"
        };
        self.dom.set_value(STAMP_TEXT_INPUT, text);
        self.dom.set_value(STAMP_PAGES_INPUT, "");
        self.dom.set_value(STAMP_ALIGN_SELECT, "left");
        self.dom
            .set_value(STAMP_BATES_START_INPUT, self.bates_next.to_string());
        self.dom
            .set_value(STAMP_BATES_PAD_INPUT, self.bates_pad.to_string());
        self.set_status_line(STAMP_STATUS_EL, "", "");
        self.dom.set_open(STAMP_DIALOG, true);
    }

    /// R:9660: the heading and Align follow the "What" select (also its `change` listener).
    pub fn sync_stamp_heading(&mut self) {
        let kind = self.dom.value(STAMP_KIND_SELECT);
        let heading = match kind.as_str() {
            "watermark" => "Watermark",
            "header" => "Add header",
            _ => "Add footer",
        };
        self.dom.set_text(STAMP_DIALOG_HEADING, heading);
        self.dom
            .set_disabled(STAMP_ALIGN_SELECT, kind == "watermark");
    }

    /// R:9666: "Apply".
    pub fn save_stamp_dialog(&mut self) -> bool {
        self.clear_error();
        let Some(page_count) = self.doc.as_ref().map(|doc| doc.page_count) else {
            self.dom.set_open(STAMP_DIALOG, false);
            return false;
        };
        self.set_status_line(STAMP_STATUS_EL, "", "");
        let range = self.dom.value(STAMP_PAGES_INPUT);
        let Some(pages) = parse_page_range(&range, page_count) else {
            // Walkthrough pages#6: the window says it in the dialog, which stays open.
            if self.headless {
                self.show_error(&format!(
                    "Pages must look like \"1-3, 5\" (this document has {page_count})"
                ));
            } else {
                let name = self
                    .doc
                    .as_ref()
                    .map(|d| d.name.clone())
                    .unwrap_or_default();
                self.set_status_line(
                    STAMP_STATUS_EL,
                    super::shell::range_problem(&range, page_count, &name),
                    "error",
                );
            }
            return false;
        };
        let kind = match self.dom.value(STAMP_KIND_SELECT).as_str() {
            "watermark" => "watermark",
            "header" => "header",
            _ => "footer",
        };
        let text = self.dom.value(STAMP_TEXT_INPUT);
        // §9.11: the Bates fields count only when the text uses {bates}; an empty start is a
        // refusal, never 0.
        let mut bates = None;
        if text.contains("{bates}") {
            let start = js_trim(&self.dom.value(STAMP_BATES_START_INPUT)).to_string();
            let pad = js_trim(&self.dom.value(STAMP_BATES_PAD_INPUT)).to_string();
            if !bates_fields_ok(&start, &pad) {
                self.show_error("Bates start must be a whole number and digits 1\u{2013}15");
                self.decorate_last(DecorateLast {
                    action: "stamp".into(),
                    kind: Some(kind.into()),
                    ok: false,
                    ..DecorateLast::default()
                });
                return false;
            }
            bates = Some((
                start.parse::<i64>().unwrap_or(0),
                pad.parse::<i64>().unwrap_or(0),
            ));
        }
        let align = (kind != "watermark" && self.dom.value(STAMP_ALIGN_SELECT) == "right")
            .then_some("right");
        self.dom.set_open(STAMP_DIALOG, false);
        let all = pages.len() as i64 == page_count;
        self.add_stamp(kind, &text, (!all).then_some(pages), bates, align)
    }

    /// R:9696: one `stamp` request; `bates` is `(batesStart, batesPad)`.
    pub fn add_stamp(
        &mut self,
        kind: &str,
        text: &str,
        pages: Option<Vec<usize>>,
        bates: Option<(i64, i64)>,
        align: Option<&str>,
    ) -> bool {
        self.stamp_bates = None;
        let mut message = json!({ "op": "stamp", "kind": kind, "text": text });
        if let Some(pages) = &pages {
            message["pages"] = json!(pages);
        }
        if let Some((start, pad)) = bates {
            message["batesStart"] = json!(start);
            message["batesPad"] = json!(pad);
        }
        if let Some(align) = align {
            message["align"] = json!(align);
        }
        let reply = match self.request(message, vec![]) {
            Ok(reply) => reply,
            Err(err) => return self.stamp_failed(kind, &err),
        };
        let page_count = self.doc.as_ref().map_or(0, |doc| doc.page_count);
        let count = pages.as_ref().map_or(page_count, |p| p.len() as i64);
        let stamped: Vec<i64> = match &pages {
            Some(p) => p.iter().map(|&i| i as i64).collect(),
            None => (0..page_count).collect(),
        };
        self.apply_edit(&journal_of(&reply.value), "");
        let mut suffix = String::new();
        let range: Option<BatesRange> = reply.get("bates").and_then(super::parse_reply);
        if let Some(range) = range {
            let pad = bates.map_or(6, |(_, pad)| pad);
            self.stamp_bates = Some(range);
            self.bates_next = range.to + 1;
            self.bates_pad = pad;
            self.bates_last = Some(BatesLast {
                from: range.from,
                to: range.to,
                pages: stamped.clone(),
            });
            let from = pad_number(range.from, pad);
            let to = pad_number(range.to, pad);
            suffix = if from == to {
                format!(" (Bates {from})")
            } else {
                format!(" (Bates {from}\u{2013}{to})")
            };
        }
        self.decorate_last(DecorateLast {
            action: "stamp".into(),
            kind: Some(kind.into()),
            pages: Some(stamped),
            ok: true,
            detail: None,
            bates: self.stamp_bates,
        });
        self.announce_page(&format!(
            "Added {kind} to {count} page{}{suffix}",
            if count == 1 { "" } else { "s" }
        ));
        true
    }

    fn stamp_failed(&mut self, kind: &str, err: &Refusal) -> bool {
        self.decorate_last(DecorateLast {
            action: "stamp".into(),
            kind: Some(kind.into()),
            ok: false,
            ..DecorateLast::default()
        });
        let message = self.message_of(err);
        self.show_error(&message);
        false
    }

    /// R:9725: "Page labels…".
    pub fn open_labels_dialog(&mut self) {
        let Some(page_count) = self.doc.as_ref().map(|doc| doc.page_count) else {
            return;
        };
        if !self.can_edit() {
            return;
        }
        self.dom
            .set_value(LABELS_FROM_INPUT, (self.wanted_page + 1).to_string());
        self.dom
            .set_attr(LABELS_FROM_INPUT, "max", page_count.to_string());
        self.dom.set_value(LABELS_STYLE_SELECT, "D");
        self.dom.set_value(LABELS_PREFIX_INPUT, "");
        self.dom.set_value(LABELS_START_INPUT, "1");
        self.dom.set_open(LABELS_DIALOG, true);
    }

    /// R:9738: the 0-based page the labels dialog acts on, or None when out of range.
    pub fn labels_from(&self) -> Option<i64> {
        let doc = self.doc.as_ref()?;
        let from = js_number_of(&self.dom.value(LABELS_FROM_INPUT));
        if from.fract() != 0.0 || !from.is_finite() || from < 1.0 || from > doc.page_count as f64 {
            return None;
        }
        Some(from as i64 - 1)
    }

    fn labels_from_refused(&mut self) {
        let count = self.doc.as_ref().map_or(1, |doc| doc.page_count);
        self.show_error(&format!("Enter a page between 1 and {count}"));
    }

    /// R:9745: "Set labels".
    pub fn save_labels_dialog(&mut self) -> bool {
        let Some(from) = self.labels_from() else {
            self.labels_from_refused();
            return false;
        };
        let style = self.dom.value(LABELS_STYLE_SELECT);
        let prefix = slice_utf16(&self.dom.value(LABELS_PREFIX_INPUT), 32);
        let start = js_number_of(&self.dom.value(LABELS_START_INPUT));
        // `Number(x) || 1`: NaN and 0 fall back to 1.
        let start = if start.is_nan() || start == 0.0 {
            1.0
        } else {
            start
        };
        self.dom.set_open(LABELS_DIALOG, false);
        let message = json!({
            "op": "labels-set", "from": from, "style": style, "prefix": prefix,
            "start": crate::host::util::js_number(start),
        });
        self.labels_reply(message, &format!("Page labels set from page {}", from + 1))
    }

    /// R:9768: "Clear from this page".
    pub fn clear_labels(&mut self) -> bool {
        let Some(from) = self.labels_from() else {
            self.labels_from_refused();
            return false;
        };
        self.dom.set_open(LABELS_DIALOG, false);
        self.labels_reply(
            json!({ "op": "labels-clear", "from": from }),
            &format!("Cleared page labels from page {}", from + 1),
        )
    }

    fn labels_reply(&mut self, message: Value, announcement: &str) -> bool {
        match self.request(message, vec![]) {
            Ok(reply) => {
                self.labels = label_items(&reply.value);
                self.apply_edit(&journal_of(&reply.value), "");
                self.apply_label_text();
                self.announce_page(announcement);
                true
            }
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
                false
            }
        }
    }

    /// R:9789 (A2 §8.3): record the last dialog-backed decorate action without dropping the
    /// stamps view. The labels and layers are read live: a clear moves them backwards.
    pub fn decorate_last(&mut self, last: DecorateLast) {
        let previous = self.decorate_summary.take();
        self.decorate_summary = Some(DecorateSummary {
            stamps: previous
                .as_ref()
                .map_or_else(DecorateStamps::default, |s| s.stamps),
            layers_saved: previous
                .as_ref()
                .map_or(self.layers_persisted, |s| s.layers_saved),
            labels: self.labels.clone(),
            layers: self.decorate_layers(),
            last: Some(last),
        });
    }

    fn decorate_layers(&self) -> Vec<DecorateLayer> {
        self.layers
            .iter()
            .map(|item| DecorateLayer {
                name: item.name.clone(),
                visible: item.visible,
            })
            .collect()
    }

    /// R:9804 `--edit=decorate`: watermark and header on every page, labels from page 1 in
    /// roman numerals, and on a document with layers the second layer hidden (view only).
    pub fn smoke_decorate(&mut self, contents: Option<&str>) {
        let Some(count) = self.doc.as_ref().map(|doc| doc.page_count) else {
            self.log_edit();
            return;
        };
        let text = contents.unwrap_or("PDF4All P2 decorate").to_string();
        let watermark_ok = self.add_stamp("watermark", &text, None, None, None);
        let header_ok = self.add_stamp("header", "Page {page} of {pages}", None, None, None);
        match self.request(
            json!({ "op": "labels-set", "from": 0, "style": "r" }),
            vec![],
        ) {
            Ok(reply) => {
                self.labels = label_items(&reply.value);
                self.apply_edit(&journal_of(&reply.value), "");
            }
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
            }
        }
        self.apply_label_text();
        // The contract's "hide layer 1": mupdf lists layers in the reverse of /OCGs, so the
        // fixture's own order decides which one that is; the logged name says which.
        if self.layers.len() > 1 {
            self.set_layer(1, false);
        }
        self.decorate_summary = Some(DecorateSummary {
            stamps: DecorateStamps {
                watermark: if watermark_ok { count } else { 0 },
                header: if header_ok { count } else { 0 },
            },
            labels: self.labels.clone(),
            layers: self.decorate_layers(),
            layers_saved: self.layers_persisted,
            last: None,
        });
        self.log_edit();
    }

    /// R:9837: `P2_EDIT.decorate` after an undo/redo: the stamps and labels moved, the layers
    /// did not (a toggle is not journaled).
    pub fn refresh_decorate_summary(&mut self) {
        let labels = self.labels.clone();
        let layers = self.decorate_layers();
        if let Some(summary) = self.decorate_summary.as_mut() {
            summary.labels = labels;
            summary.layers = layers;
        }
    }

    /// R:9847: one fixed line the smoke waits for by name: the state after the edit.
    pub fn log_edit(&mut self) {
        let summary = self.edit_summary();
        self.console_line("P2_EDIT", summary);
    }

    // ------------------------------------------------------------- note / text-box dialog

    /// R:10045 `pagePoint(event)`: the UI hands page points already (y down, PDF points);
    /// None while the shown page has no size, as the TS returns for an unlaid canvas.
    pub fn page_point(&self, point: Point) -> Option<Point> {
        let doc = self.doc.as_ref()?;
        let size = doc.pages.get(usize::try_from(self.shown_page).ok()?)?;
        (size.width >= 1.0 && size.height >= 1.0).then_some(point)
    }

    /// R:10055: the Comments list's "edit" button.
    pub fn open_note_dialog(&mut self, annot_id: i64) {
        let Some(contents) = self
            .notes
            .iter()
            .find(|note| note.id == annot_id)
            .map(|n| n.contents.clone())
        else {
            return;
        };
        self.editing_note = Some(annot_id);
        self.box_point = None;
        self.dom.set_text(NOTE_DIALOG_HEADING, "Edit note");
        self.dom.set_value(NOTE_TEXT_INPUT, contents);
        self.dom.set_open(NOTE_DIALOG, true);
    }

    /// R:10068: the same dialog in add mode: the typed text becomes the new box's contents.
    pub fn open_text_box_dialog(&mut self, point: Point) {
        if self.doc.is_none() || !self.can_annotate() {
            return;
        }
        self.editing_note = None;
        self.box_point = Some(point);
        self.dom.set_text(NOTE_DIALOG_HEADING, "Add text box");
        self.dom.set_value(NOTE_TEXT_INPUT, "");
        self.dom.set_open(NOTE_DIALOG, true);
    }

    /// R:10078: "Save". Closing the dialog clears `editingNote`/`boxPoint` (its `close`
    /// listener, R:10893), so both are read first.
    pub fn save_note_dialog(&mut self) {
        let annot_id = self.editing_note;
        let point = self.box_point;
        self.close_note_dialog();
        let text = slice_utf16(
            &self.dom.value(NOTE_TEXT_INPUT),
            tandem_protocol::limits::MAX_NOTE_CHARS,
        );
        if let Some(point) = point {
            self.add_text_box_at(self.wanted_page.max(0), point, &text);
            return;
        }
        let Some(annot_id) = annot_id else {
            return;
        };
        self.edit_note(annot_id, &text);
    }

    /// `noteDialog.close()` with its `close` listener (R:10893).
    pub fn close_note_dialog(&mut self) {
        self.dom.set_open(NOTE_DIALOG, false);
        self.editing_note = None;
        self.box_point = None;
    }
}

/// `Number(text)` for a form field's value: blank is 0, otherwise a decimal (or hex/binary/
/// octal) literal with surrounding whitespace, else NaN.
pub(crate) fn js_number_of(text: &str) -> f64 {
    let t = js_trim(text);
    if t.is_empty() {
        return 0.0;
    }
    let radix = |prefix: &[&str], radix: u32| {
        prefix.iter().find_map(|p| t.strip_prefix(p)).map(|rest| {
            if rest.is_empty() {
                f64::NAN
            } else {
                u64::from_str_radix(rest, radix).map_or(f64::NAN, |v| v as f64)
            }
        })
    };
    if let Some(v) = radix(&["0x", "0X"], 16)
        .or_else(|| radix(&["0b", "0B"], 2))
        .or_else(|| radix(&["0o", "0O"], 8))
    {
        return v;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let ok = t
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'));
    if !ok {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// `text.slice(0, n)` in UTF-16 units (a surrogate pair is never split here).
pub(crate) fn slice_utf16(text: &str, n: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for c in text.chars() {
        used += c.len_utf16();
        if used > n {
            break;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bates_and_padding() {
        assert!(bates_fields_ok("1", "6"));
        assert!(bates_fields_ok("000000000000001", "15"));
        assert!(!bates_fields_ok("", "6"));
        assert!(!bates_fields_ok("1", "16"));
        assert!(!bates_fields_ok("1", "0"));
        assert!(!bates_fields_ok("1.5", "6"));
        assert!(!bates_fields_ok("1234567890123456", "6"));
        assert_eq!(pad_number(123, 6), "000123");
        assert_eq!(pad_number(1234567, 6), "1234567");
    }

    #[test]
    fn js_numbers_and_slices() {
        assert_eq!(js_number_of(" 3 "), 3.0);
        assert_eq!(js_number_of(""), 0.0);
        assert!(js_number_of("2a").is_nan());
        assert_eq!(js_number_of("0x10"), 16.0);
        assert_eq!(js_number_of("1e2"), 100.0);
        assert_eq!(slice_utf16("ab😀c", 3), "ab");
        assert_eq!(slice_utf16("abc", 2), "ab");
    }

    #[test]
    fn labels_and_decorate_summary() {
        let mut s = Session::new();
        s.labels = vec!["1".into(), "ii".into()];
        assert_eq!(s.label_suffix(0), "");
        assert_eq!(s.label_suffix(1), " (ii)");
        assert_eq!(s.label_suffix(5), "");
        s.layers = vec![LayerItem {
            id: 1,
            name: "A".into(),
            visible: true,
        }];
        s.decorate_last(DecorateLast {
            action: "stamp".into(),
            ok: false,
            ..DecorateLast::default()
        });
        let summary = s.decorate_summary.clone().unwrap();
        assert_eq!(summary.labels.len(), 2);
        assert_eq!(summary.layers[0].name, "A");
        assert!(summary.last.is_some());
    }
}
