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

/// A stamp's look beyond its text (new in the Rust app): what the stamp dialog's Size,
/// Colour, Opacity and "Behind the page" fields say. A field at the engine's own look for
/// the kind is left out of the request, so a plain header writes the bytes it always did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StampLook {
    pub size: Option<i64>,
    /// `#rrggbb`.
    pub color: Option<String>,
    /// Percent, 1..=100.
    pub opacity: Option<i64>,
    pub behind: bool,
}

impl StampLook {
    /// What the dialog starts with: a watermark is 48 pt grey at 30%, behind the page's
    /// content; a header or footer 10 pt black, solid, on top.
    pub fn starting(kind: &str) -> Self {
        if kind == "watermark" {
            Self {
                size: Some(48),
                color: Some("#808080".into()),
                opacity: Some(30),
                behind: true,
            }
        } else {
            Self {
                size: Some(10),
                color: Some("#000000".into()),
                opacity: Some(100),
                behind: false,
            }
        }
    }

    /// Adds the fields that differ from the engine's own look for `kind` to a `stamp`
    /// request (or a `batch-file` action).
    pub fn write(&self, kind: &str, message: &mut Value) {
        let engine = if kind == "watermark" {
            (48, "#808080")
        } else {
            (10, "#000000")
        };
        if let Some(size) = self.size.filter(|s| *s != engine.0) {
            message["size"] = json!(size);
        }
        if let Some(color) = (self.color.as_deref())
            .map(str::to_ascii_lowercase)
            .filter(|c| c != engine.1)
        {
            message["color"] = json!(color);
        }
        if let Some(opacity) = self.opacity.filter(|o| *o < 100) {
            message["opacity"] = json!(opacity as f64 / 100.0);
        }
        if self.behind && kind == "watermark" {
            message["behind"] = json!(true);
        }
    }
}

/// One header, footer or watermark the document already has, as the engine's `stamps` lists
/// it: the dialog updates or removes it in place rather than adding another on top.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StampItem {
    /// A number, or "legacy-watermark" (…) for a stamp from before stamps had numbers.
    pub id: Value,
    pub kind: String,
    pub recorded: bool,
    /// The 0-based pages it is on.
    pub found: Vec<i64>,
    pub text: String,
    pub align: Option<String>,
    pub size: Option<f64>,
    pub color: Option<String>,
    pub opacity: Option<f64>,
    pub behind: bool,
    /// The 0-based pages its request named (None: every page).
    pub pages: Option<Vec<i64>>,
    pub bates_start: Option<i64>,
    pub bates_pad: Option<i64>,
}

/// Which existing stamp the stamp dialog's primary button updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StampTarget {
    /// `stamp_existing[i]`.
    One(usize),
    /// Every stamp of the kind (several from older files or Batch become one).
    All,
}

/// "1-3, 5" for 0-based pages.
pub(crate) fn range_text(pages: &[i64]) -> String {
    let mut pages: Vec<i64> = pages.to_vec();
    pages.sort_unstable();
    pages.dedup();
    let mut parts = Vec::new();
    let mut i = 0;
    while i < pages.len() {
        let start = pages[i];
        let mut end = start;
        while i + 1 < pages.len() && pages[i + 1] == end + 1 {
            i += 1;
            end = pages[i];
        }
        parts.push(if start == end {
            (start + 1).to_string()
        } else {
            format!("{}-{}", start + 1, end + 1)
        });
        i += 1;
    }
    parts.join(", ")
}

/// How a stamp reads in the dialog's chooser: `"DRAFT", 48 pt, pages 1-3`.
pub(crate) fn stamp_label(item: &StampItem, page_count: i64) -> String {
    let mut label = format!("\u{201c}{}\u{201d}", item.text);
    if let Some(size) = item.size {
        label.push_str(&format!(", {} pt", js_num(size)));
    }
    let all = item.found.len() as i64 == page_count;
    if !all && !item.found.is_empty() {
        let pages = range_text(&item.found);
        let word = if item.found.len() == 1 {
            "page"
        } else {
            "pages"
        };
        label.push_str(&format!(", {word} {pages}"));
    }
    label
}

/// `#rrggbb`, as a colour field gives it.
fn is_hex_color(text: &str) -> bool {
    text.strip_prefix('#')
        .is_some_and(|hex| hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
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
        self.stamp_dialog_defaults();
        self.set_status_line(STAMP_STATUS_EL, "", "");
        self.load_stamp_existing();
        self.dom.set_open(STAMP_DIALOG, true);
    }

    /// The `stamps` of the dialog's kind; the newest is chosen to update, its fields filled
    /// in. None: the dialog adds.
    pub fn load_stamp_existing(&mut self) {
        let kind = self.dom.value(STAMP_KIND_SELECT);
        self.stamp_existing = match self.request(json!({ "op": "stamps" }), vec![]) {
            Ok(reply) => reply
                .get("items")
                .and_then(super::parse_reply::<Vec<StampItem>>)
                .unwrap_or_default()
                .into_iter()
                .filter(|item| item.kind == kind)
                .collect(),
            Err(_) => Vec::new(),
        };
        let newest = self.stamp_existing.len().checked_sub(1);
        self.choose_stamp(newest.map(StampTarget::One));
    }

    /// Updates `target` (None adds a new stamp): fills the fields from it and says
    /// "Update" or "Add".
    pub fn choose_stamp(&mut self, target: Option<StampTarget>) {
        let target = target.filter(|t| match t {
            StampTarget::One(i) => *i < self.stamp_existing.len(),
            StampTarget::All => !self.stamp_existing.is_empty(),
        });
        self.stamp_target = target;
        let item = match target {
            Some(StampTarget::One(i)) => self.stamp_existing.get(i).cloned(),
            Some(StampTarget::All) => self.stamp_existing.last().cloned(),
            None => None,
        };
        if let Some(item) = item {
            self.fill_stamp_fields(&item);
        }
        self.sync_stamp_heading();
    }

    /// The dialog's fields as `item` was made.
    fn fill_stamp_fields(&mut self, item: &StampItem) {
        let page_count = self.doc.as_ref().map_or(0, |doc| doc.page_count);
        self.dom.set_value(STAMP_TEXT_INPUT, item.text.clone());
        self.dom
            .set_value(STAMP_ALIGN_SELECT, item.align.as_deref().unwrap_or("left"));
        let look = StampLook::starting(&item.kind);
        let size = item
            .size
            .map_or(look.size.unwrap_or(10), |s| s.round() as i64);
        self.dom.set_value(STAMP_SIZE_INPUT, size.to_string());
        let engine = if item.kind == "watermark" {
            "#808080"
        } else {
            "#000000"
        };
        self.dom
            .set_value(STAMP_COLOR_INPUT, item.color.as_deref().unwrap_or(engine));
        let opacity = item.opacity.map_or(100, |o| (o * 100.0).round() as i64);
        self.dom
            .set_value(STAMP_OPACITY_SELECT, opacity.to_string());
        self.dom.set_checked(STAMP_BEHIND_CHECK, item.behind);
        self.dom.set_text(STAMP_BEHIND_NOTE_EL, "");
        // The pages its request named; a stamp without a record keeps the pages it is on.
        let pages = match (&item.pages, item.recorded) {
            (Some(pages), _) => range_text(pages),
            (None, false) if (item.found.len() as i64) < page_count => range_text(&item.found),
            _ => String::new(),
        };
        self.dom.set_value(STAMP_PAGES_INPUT, pages);
        if let Some(start) = item.bates_start {
            self.dom
                .set_value(STAMP_BATES_START_INPUT, start.to_string());
        }
        if let Some(pad) = item.bates_pad {
            self.dom.set_value(STAMP_BATES_PAD_INPUT, pad.to_string());
        }
    }

    /// The ids "Update" replaces and "Remove" removes.
    pub fn stamp_target_ids(&self) -> Vec<Value> {
        match self.stamp_target {
            Some(StampTarget::One(i)) => self
                .stamp_existing
                .get(i)
                .map(|item| vec![item.id.clone()])
                .unwrap_or_default(),
            Some(StampTarget::All) => self.stamp_existing.iter().map(|i| i.id.clone()).collect(),
            None => Vec::new(),
        }
    }

    /// The dialog's chooser: one line per existing stamp of the kind, which is chosen, and
    /// whether "Replace all" is.
    pub fn stamp_choices(&self) -> Value {
        let page_count = self.doc.as_ref().map_or(0, |doc| doc.page_count);
        let items: Vec<Value> = (self.stamp_existing.iter().enumerate())
            .map(|(i, item)| {
                json!({
                    "label": stamp_label(item, page_count),
                    "chosen": self.stamp_target == Some(StampTarget::One(i)),
                })
            })
            .collect();
        json!({ "stamps": items, "all": self.stamp_target == Some(StampTarget::All) })
    }

    /// "Remove": takes the chosen stamp (or all of the kind) off every page, one undo step.
    pub fn remove_stamp_dialog(&mut self) -> bool {
        self.clear_error();
        let ids = self.stamp_target_ids();
        let kind = self.dom.value(STAMP_KIND_SELECT);
        if ids.is_empty() {
            return false;
        }
        self.dom.set_open(STAMP_DIALOG, false);
        self.remove_stamps(&kind, ids)
    }

    /// One `stamp-remove` request.
    pub fn remove_stamps(&mut self, kind: &str, ids: Vec<Value>) -> bool {
        let reply = match self.request(json!({ "op": "stamp-remove", "id": ids }), vec![]) {
            Ok(reply) => reply,
            Err(err) => {
                self.decorate_last(DecorateLast {
                    action: "stamp-remove".into(),
                    kind: Some(kind.into()),
                    ok: false,
                    ..DecorateLast::default()
                });
                let message = self.message_of(&err);
                self.show_error(&message);
                return false;
            }
        };
        self.apply_edit(&journal_of(&reply.value), "");
        self.decorate_last(DecorateLast {
            action: "stamp-remove".into(),
            kind: Some(kind.into()),
            ok: true,
            ..DecorateLast::default()
        });
        let what = if kind == "watermark" {
            "watermark"
        } else {
            kind
        };
        self.announce_page(&format!("Removed the {what}"));
        true
    }

    /// The chooser's "Add a new one": the starting fields, the existing stamps still listed.
    pub fn stamp_dialog_defaults_keep_existing(&mut self) {
        let existing = std::mem::take(&mut self.stamp_existing);
        self.stamp_dialog_defaults();
        self.stamp_existing = existing;
    }

    /// The dialog's starting fields for its kind (adding a new stamp).
    pub fn stamp_dialog_defaults(&mut self) {
        self.stamp_existing.clear();
        self.stamp_target = None;
        self.sync_stamp_heading();
        let text = if self.dom.value(STAMP_KIND_SELECT) == "watermark" {
            "DRAFT"
        } else {
            "Page {page} of {pages}"
        };
        self.dom.set_value(STAMP_TEXT_INPUT, text);
        self.dom.set_value(STAMP_PAGES_INPUT, "");
        self.dom.set_value(STAMP_ALIGN_SELECT, "left");
        self.reset_stamp_look();
        self.stamp_page_covered = None;
        self.sync_stamp_behind();
        self.dom
            .set_value(STAMP_BATES_START_INPUT, self.bates_next.to_string());
        self.dom
            .set_value(STAMP_BATES_PAD_INPUT, self.bates_pad.to_string());
    }

    /// R:9660: the heading and Align follow the "What" select (also its `change` listener).
    pub fn sync_stamp_heading(&mut self) {
        let kind = self.dom.value(STAMP_KIND_SELECT);
        let updating = self.stamp_target.is_some();
        let heading = match (kind.as_str(), updating) {
            ("watermark", false) => "Watermark",
            ("watermark", true) => "Update watermark",
            ("header", false) => "Add header",
            ("header", true) => "Update header",
            (_, false) => "Add footer",
            (_, true) => "Update footer",
        };
        self.dom.set_text(STAMP_DIALOG_HEADING, heading);
        self.dom
            .set_text(STAMP_SAVE_BUTTON, if updating { "Update" } else { "Add" });
        self.dom.set_hidden(STAMP_REMOVE_BUTTON, !updating);
        self.dom
            .set_disabled(STAMP_ALIGN_SELECT, kind == "watermark");
        // "Behind the page" is a watermark's alone.
        self.dom
            .set_disabled(STAMP_BEHIND_CHECK, kind != "watermark");
        self.dom.set_hidden(STAMP_BEHIND_CHECK, kind != "watermark");
    }

    /// A watermark starts on top (still see-through) on a page that a picture covers, such
    /// as a scan: behind it, the picture would hide it. Says why under the box.
    pub fn sync_stamp_behind(&mut self) {
        if self.dom.value(STAMP_KIND_SELECT) != "watermark" {
            self.dom.set_text(STAMP_BEHIND_NOTE_EL, "");
            return;
        }
        let covered = match self.stamp_page_covered {
            Some(covered) => covered,
            None => {
                let covered = self.page_starts_covered();
                self.stamp_page_covered = Some(covered);
                covered
            }
        };
        if covered {
            self.dom.set_checked(STAMP_BEHIND_CHECK, false);
        }
        self.dom.set_text(
            STAMP_BEHIND_NOTE_EL,
            if covered {
                "This page is covered by a picture (a scan?), so the watermark starts on top of it."
            } else {
                ""
            },
        );
    }

    /// Whether a picture covers 90% or more of the page shown (a scanned page): the page's
    /// editable objects, as the content panel lists them. A filled shape is not looked at.
    fn page_starts_covered(&mut self) -> bool {
        let page = self.wanted_page;
        let Some(size) = self.page_size(page) else {
            return false;
        };
        let area = size.width * size.height;
        if area <= 0.0 {
            return false;
        }
        let Ok(reply) = self.request(json!({ "op": "edit-targets", "page": page }), vec![]) else {
            return false;
        };
        let reply: tandem_protocol::engine::EditTargetsReply =
            super::parse_reply_or_default(&reply.value);
        reply.items.iter().any(|item| {
            let rect = item
                .placement
                .as_ref()
                .map(|p| p.r#box.clone())
                .or_else(|| item.bounds.clone())
                .unwrap_or_default();
            item.kind == tandem_protocol::engine::EditTargetKind::Image
                && rect.len() == 4
                && ((rect[2] - rect[0]) * (rect[3] - rect[1])).abs() >= 0.9 * area
        })
    }

    /// The look fields at the kind's starting look (on open, and when "Add" changes: a
    /// header at a watermark's 48 pt would not fit).
    pub fn reset_stamp_look(&mut self) {
        let kind = self.dom.value(STAMP_KIND_SELECT);
        let look = StampLook::starting(&kind);
        let number = |n: Option<i64>| n.map(|n| n.to_string()).unwrap_or_default();
        self.dom.set_value(STAMP_SIZE_INPUT, number(look.size));
        self.dom
            .set_value(STAMP_COLOR_INPUT, look.color.unwrap_or_default());
        self.dom
            .set_value(STAMP_OPACITY_SELECT, number(look.opacity));
        self.dom.set_checked(STAMP_BEHIND_CHECK, look.behind);
    }

    /// The look the dialog's fields say, or the problem to show.
    pub fn stamp_look_of_dialog(&self, kind: &str) -> Result<StampLook, String> {
        let size = js_trim(&self.dom.value(STAMP_SIZE_INPUT)).to_string();
        let size = match size.parse::<f64>() {
            Ok(n) if n.fract() == 0.0 && (4.0..=200.0).contains(&n) => n as i64,
            _ => return Err("Size must be a whole number from 4 to 200".into()),
        };
        let color = js_trim(&self.dom.value(STAMP_COLOR_INPUT)).to_string();
        if !is_hex_color(&color) {
            return Err("Colour must look like #336699".into());
        }
        let Some(opacity) = (js_trim(&self.dom.value(STAMP_OPACITY_SELECT)).parse::<i64>())
            .ok()
            .filter(|o| (1..=100).contains(o))
        else {
            return Err("Opacity must be a whole percentage from 1 to 100".into());
        };
        Ok(StampLook {
            size: Some(size),
            color: Some(color),
            opacity: Some(opacity),
            behind: kind == "watermark" && self.dom.checked(STAMP_BEHIND_CHECK),
        })
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
        let align = match self.dom.value(STAMP_ALIGN_SELECT).as_str() {
            _ if kind == "watermark" => None,
            "right" => Some("right"),
            "center" => Some("center"),
            _ => None,
        };
        let look = match self.stamp_look_of_dialog(kind) {
            Ok(look) => look,
            Err(problem) => {
                if self.headless {
                    self.show_error(&problem);
                } else {
                    self.set_status_line(STAMP_STATUS_EL, problem, "error");
                }
                return false;
            }
        };
        self.dom.set_open(STAMP_DIALOG, false);
        let all = pages.len() as i64 == page_count;
        let replace = self.stamp_target_ids();
        self.stamp_replace = (!replace.is_empty()).then_some(replace);
        let ok = self.add_stamp(kind, &text, (!all).then_some(pages), bates, align, &look);
        self.stamp_replace = None;
        ok
    }

    /// R:9696: one `stamp` request; `bates` is `(batesStart, batesPad)`.
    pub fn add_stamp(
        &mut self,
        kind: &str,
        text: &str,
        pages: Option<Vec<usize>>,
        bates: Option<(i64, i64)>,
        align: Option<&str>,
        look: &StampLook,
    ) -> bool {
        self.stamp_bates = None;
        let mut message = json!({ "op": "stamp", "kind": kind, "text": text });
        look.write(kind, &mut message);
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
        let updating = self.stamp_replace.is_some();
        if let Some(ids) = &self.stamp_replace {
            message["replace"] = json!(ids);
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
            "{} {kind} {} {count} page{}{suffix}",
            if updating { "Updated" } else { "Added" },
            if updating { "on" } else { "to" },
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
        let plain = StampLook::default();
        let watermark_ok = self.add_stamp("watermark", &text, None, None, None, &plain);
        let header_ok =
            self.add_stamp("header", "Page {page} of {pages}", None, None, None, &plain);
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
