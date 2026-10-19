//! Existing content: the Text inspector tab, its edit forms, find & replace on this page and
//! the `content` / `text-add` drive paths (renderer.ts R:5661-6428, P3 U2-U5).
#![allow(dead_code)]

use super::Session;
use super::consts::{CONTENT_DETAIL_TEXT, CONTENT_REASON_TEXT, lookup};
use super::dom_ids::*;
use super::modes::journal_of;
use super::types::{
    ContentEdited, ContentLastAdd, ContentPageRefusal, ContentRefusal, ContentReplaced,
    ContentScan, EditTarget, ReplaceMatch, ReplacePreview, ReplaceRejected,
};
use crate::host::util::js_number;
use crate::js::{js_num, js_round, js_trim};
use crate::messages::{MessageKind, user_text};
use serde_json::{Map, Value, json};
use tandem_protocol::engine::{EditTargetKind, EditTargetSupport, EditTargetsReply};

/// One row of `#content-targets` as the UI draws it (R:5785-5806).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContentRow {
    /// `content-target-{i}`.
    pub id: String,
    pub text: String,
    pub meta: String,
    /// `aria-selected` (and the roving tab stop).
    pub selected: bool,
}

/// One "Paragraph, N lines, …" button above the rows (R:5779-5784); `item` is its first run's
/// index in the scan.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParagraphRow {
    pub label: String,
    pub item: usize,
}

/// One row of the find & replace preview (R:6183-6202): a tickable match or a rejected run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReplaceRow {
    /// `content-match-{i}` for a match, None for a rejected run.
    pub checkbox: Option<String>,
    pub text: String,
}

/// One member of a multi-object edit (`{ item, box? }`, R:6055).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectEntry {
    pub item: EditTarget,
    pub r#box: Option<Vec<f64>>,
}

/// A JSON string as `JSON.stringify` writes it.
pub(crate) fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

/// `JSON.stringify(value)` of any JSON value (numbers as JS prints them).
pub(crate) fn json_text(value: &Value) -> String {
    match value {
        Value::Number(n) => n.as_f64().map(js_num).unwrap_or_else(|| n.to_string()),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(json_text).collect::<Vec<_>>().join(",")
        ),
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(k, v)| format!("{}:{}", json_string(k), json_text(v)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        other => other.to_string(),
    }
}

/// JavaScript's `Number(text)` for a form field's value: "" is 0, junk is NaN.
pub(crate) fn js_number_of(text: &str) -> f64 {
    let t = js_trim(text);
    if t.is_empty() {
        return 0.0;
    }
    let radix = |prefix: &str, radix: u32| {
        t.strip_prefix(prefix)
            .or_else(|| t.strip_prefix(&prefix.to_uppercase()))
            .map(|digits| {
                u64::from_str_radix(digits, radix)
                    .map(|v| v as f64)
                    .unwrap_or(f64::NAN)
            })
    };
    if let Some(v) = radix("0x", 16)
        .or_else(|| radix("0o", 8))
        .or_else(|| radix("0b", 2))
    {
        return v;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    // Rust also reads "inf", "nan" and "infinity"; JavaScript does not.
    let body = t.trim_start_matches(['+', '-']);
    if body.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// `Math.round(value * 100) / 100`.
pub(crate) fn round2(value: f64) -> f64 {
    js_round(value * 100.0) / 100.0
}

/// `Math.round(value * 10) / 10`.
fn round1(value: f64) -> f64 {
    js_round(value * 10.0) / 10.0
}

/// `CONTENT_REASON_TEXT[reason ?? ""]`.
fn reason_text(reason: Option<&str>) -> Option<&'static str> {
    lookup(&CONTENT_REASON_TEXT, reason.unwrap_or(""))
}

/// `CONTENT_DETAIL_TEXT[detail]`.
fn detail_text(detail: &str) -> Option<&'static str> {
    lookup(&CONTENT_DETAIL_TEXT, detail)
}

fn is_candidate(item: &EditTarget) -> bool {
    item.support == EditTargetSupport::Candidate
}

fn is_text(item: &EditTarget) -> bool {
    item.kind == EditTargetKind::Text
}

fn has_op(item: &EditTarget, op: &str) -> bool {
    item.object_ops
        .as_ref()
        .is_some_and(|ops| ops.iter().any(|o| o.as_str() == op))
}

/// The refusal detail the content paths record: the reply's `detail`, else `read-only` for a
/// read-only refusal, else `not-candidate` (R:5911, R:6035).
fn content_detail(err: &crate::engine::Refusal) -> String {
    match &err.detail {
        Some(detail) => detail.clone(),
        None if err.reason.as_deref() == Some("read-only") => "read-only".into(),
        None => "not-candidate".into(),
    }
}

/// A text edit or added text with letters the font lacks.
pub fn missing_letters_text(missing: &[String]) -> String {
    let letters: Vec<String> = missing.iter().map(|m| format!("“{m}”")).collect();
    format!(
        "This font has no {}. Use letters already on the page, or cover the text and add new text with the Text tool.",
        letters.join(", ")
    )
}

/// The replace preview's summary (X7: no "runs"): "3 places can change; 1 can't".
fn replace_summary(matches: usize, rejected: usize) -> String {
    let places = |n: usize| format!("{n} place{}", if n == 1 { "" } else { "s" });
    match (matches, rejected) {
        (0, 0) => "Nothing found".into(),
        (m, 0) => format!("{} can change", places(m)),
        (0, r) => format!("Found {}, but none can be changed here", places(r)),
        (m, r) => format!("{} can change; {r} can't (see why below)", places(m)),
    }
}

/// R:5819: `Image 64×64 pt` / `Drawing 120×40 pt`, from the placement box, else the pixels.
pub fn object_row_label(item: &EditTarget) -> String {
    let size = match (&item.placement, &item.image) {
        (Some(placement), _) if placement.r#box.len() >= 4 => {
            let b = &placement.r#box;
            format!(
                "{}×{} pt",
                js_num(round1(b[2] - b[0])),
                js_num(round1(b[3] - b[1]))
            )
        }
        (_, Some(image)) => format!("{}×{} px", image.width, image.height),
        _ => String::new(),
    };
    // X7: a form XObject is a drawing to people; its resource name ("Fm1") is not shown.
    let kind = if item.kind == EditTargetKind::Image {
        "Image"
    } else {
        "Drawing"
    };
    js_trim(&format!("{kind} {size}")).to_string()
}

/// The placement form's four fields for a placement box (the page's own coordinates): left,
/// from the top, width and height on the shown page (`m` from the page's coordinates to it).
pub(crate) fn placement_fields(b: &[f64], m: &[f64]) -> [f64; 4] {
    let r = super::nav::view_box(b, m);
    [r[0], r[1], r[2] - r[0], r[3] - r[1]]
}

/// The placement box for the form's four fields: the shown rectangle back through the
/// inverse of `m`. An edge within rounding of the old box's is that edge exactly, so an
/// unchanged Apply sends the old box (a no-op). None when `m` cannot be inverted.
pub(crate) fn placement_box(old: Option<&[f64]>, m: &[f64], fields: [f64; 4]) -> Option<Vec<f64>> {
    let [x, top, width, height] = fields;
    let (a, b, c, d, e, f) = (m[0], m[1], m[2], m[3], m[4], m[5]);
    let det = a * d - b * c;
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let mut xs = Vec::with_capacity(4);
    let mut ys = Vec::with_capacity(4);
    for (vx, vy) in [
        (x, top),
        (x + width, top),
        (x, top + height),
        (x + width, top + height),
    ] {
        let (u, v) = (vx - e, vy - f);
        xs.push((d * u - c * v) / det);
        ys.push((a * v - b * u) / det);
    }
    let min = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
    let max = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mut out = vec![min(&xs), min(&ys), max(&xs), max(&ys)];
    if let Some(old) = old.filter(|o| o.len() >= 4) {
        for (k, edge) in out.iter_mut().enumerate() {
            if (*edge - old[k]).abs() < 1e-6 {
                *edge = old[k];
            }
        }
    }
    Some(out)
}

/// R:5828: what the row says an Image/Form placement can do.
pub fn object_row_state(item: &EditTarget) -> String {
    if !is_candidate(item) {
        return reason_text(item.reason.as_deref())
            .unwrap_or("Not editable")
            .to_string();
    }
    if !has_op(item, "move") {
        return "Can be replaced, not moved".into();
    }
    match &item.reason {
        Some(reason) if !reason.is_empty() => {
            reason_text(Some(reason)).unwrap_or("Editable").to_string()
        }
        _ => "Editable".into(),
    }
}

/// R:6085: the one-line undo label and announcement for one placement action.
pub fn object_announcement(
    item: &EditTarget,
    action: &str,
    r#box: Option<&[f64]>,
    transform: Option<&str>,
) -> String {
    let kind = if item.kind == EditTargetKind::Image {
        "image"
    } else {
        "drawing"
    };
    match (action, transform) {
        ("transform", Some(t)) if !t.is_empty() => {
            return format!(
                "{} {kind}",
                if t.starts_with("rotate") {
                    "Rotated"
                } else {
                    "Flipped"
                }
            );
        }
        _ => {}
    }
    match action {
        "crop" => return format!("Cropped {kind}"),
        "delete" => return format!("Deleted {kind}"),
        "duplicate" => return format!("Duplicated {kind}"),
        "replace" => return format!("Replaced {kind} with a new image"),
        _ => {}
    }
    let Some(b) = r#box.filter(|b| b.len() >= 4) else {
        return if action == "move" {
            format!("Moved {kind}")
        } else {
            format!("Resized {kind}")
        };
    };
    if action == "move" {
        // Round 2: in the panel's own terms (`b` is the shown box, y down from the top).
        return format!(
            "Moved {kind} to {} from the left, {} from the top",
            js_num(round1(b[0])),
            js_num(round1(b[1]))
        );
    }
    format!(
        "Resized {kind} to {}×{}",
        js_num(round1(b[2] - b[0])),
        js_num(round1(b[3] - b[1]))
    )
}

/// The row's shown text (R:5795): an object names its kind, size and name; an empty string is
/// a deleted run.
fn row_text(item: &EditTarget) -> String {
    if !is_text(item) {
        return object_row_label(item);
    }
    match item.text.as_deref() {
        Some(text) if !text.is_empty() => text.to_string(),
        _ => item.kind.as_str().to_string(),
    }
}

/// The row's meta line (R:5799-5803).
fn row_meta(item: &EditTarget) -> String {
    let font_name = item
        .font
        .as_ref()
        .and_then(|f| f.name.clone().or_else(|| f.subtype.clone()));
    let state = if !is_text(item) {
        object_row_state(item)
    } else if is_candidate(item) {
        if item.cowable == Some(true) {
            "Editable here (the same text elsewhere stays as it is)".into()
        } else {
            "Editable".into()
        }
    } else {
        reason_text(item.reason.as_deref())
            .unwrap_or("Not editable")
            .to_string()
    };
    match font_name.map(|n| font_display_name(&n)) {
        Some(name) if !name.is_empty() => format!("{name} — {state}"),
        _ => state,
    }
}

/// A font's name as people know it (X7): no subset tag ("ABCDEF+"), file extension or
/// variable-font axes ("NotoSansHebrew[wdth,wght].ttf" → "NotoSansHebrew").
pub fn font_display_name(name: &str) -> String {
    let mut name = name;
    if let Some((tag, rest)) = name.split_once('+')
        && tag.len() == 6
        && tag.bytes().all(|b| b.is_ascii_uppercase())
    {
        name = rest;
    }
    for ext in [".ttf", ".otf", ".ttc", ".TTF", ".OTF", ".TTC"] {
        if let Some(stem) = name.strip_suffix(ext) {
            name = stem;
        }
    }
    let name = match name.find('[') {
        Some(i) => &name[..i],
        None => name,
    };
    name.trim().to_string()
}

/// Round 2: why a page has no editable text, in plain words, with what still works.
pub fn refused_page_text(reason: Option<&str>) -> &'static str {
    match reason {
        Some("too-large") => {
            "This page's text can't be edited: the page holds more content than the editor can safely read. You can still add text boxes and comments."
        }
        Some("read-only") => {
            "This page's text can't be edited: the document doesn't allow copying its text."
        }
        Some("no-document") => "No document open yet.",
        _ => {
            "This page's text can't be edited: its content uses a form the editor can't read safely. You can still add text boxes and comments."
        }
    }
}

impl Session {
    /// R:5711.
    pub fn last_added_target_id(&self) -> Option<String> {
        self.content_last_add.as_ref().map(|a| a.target_id.clone())
    }

    /// R:5724: the panel is only scanned while the Text tab is the visible one.
    pub fn content_panel_visible(&self) -> bool {
        self.inspector_open && !self.dom.hidden(TEXT_PANEL_EL)
    }

    /// R:5728.
    pub fn refresh_content_targets(&mut self, force: bool) {
        if !force && !self.content_panel_visible() {
            return;
        }
        self.content_seq += 1;
        let seq = self.content_seq;
        if self.doc.as_ref().is_none_or(|d| d.info.needs_password) {
            self.content_scan = None;
            self.render_content_targets(Some("No document open yet."));
            return;
        }
        let started_generation = self.generation;
        let page = self.wanted_page;
        let reply = match self.request(json!({ "op": "edit-targets", "page": page }), Vec::new()) {
            Ok(reply) => reply,
            Err(err) => {
                // Round 2 (edit-forms BLOCKING): a refused page has no targets. The renderer
                // kept the last scan, which after an open was the previous document's: its
                // text and images stayed selectable on the new page. The page gets none, the
                // panel says why in plain words and the hit layer is repainted.
                if seq == self.content_seq {
                    self.content_scan = None;
                    self.content_refused = Some(ContentPageRefusal {
                        page,
                        generation: started_generation,
                        reason: err.reason.clone().unwrap_or_default(),
                    });
                    let text = refused_page_text(err.reason.as_deref());
                    self.render_content_targets(Some(text));
                    // The window says it once on the page too, without covering another notice.
                    if !self.headless
                        && self.ui_mode == super::types::UiMode::Edit
                        && self.dom.hidden(ERROR_EL)
                    {
                        self.show_message(text, MessageKind::Info);
                    }
                    let (w, h) = self.canvas_client();
                    self.paint_hits(w, h, self.shown_page);
                }
                return;
            }
        };
        if seq != self.content_seq || started_generation != self.generation {
            return;
        }
        let reply: EditTargetsReply = super::parse_reply_or_default(&reply.value);
        if self.content_refused.take().is_some()
            && self.notice_kind == MessageKind::Info
            && self
                .notice_raw
                .starts_with("This page's text can't be edited")
        {
            self.clear_error();
        }
        self.content_scan = Some(ContentScan {
            page,
            generation: reply.generation,
            revision: reply.revision,
            transform: reply.transform6().map(|m| m.to_vec()),
            items: reply.items,
            paragraphs: reply.paragraphs,
            doc_generation: Some(started_generation),
        });
        self.render_content_targets(None);
        let (w, h) = self.canvas_client();
        self.paint_hits(w, h, self.shown_page);
    }

    /// R:5767. The rows themselves are [`Session::content_rows`] and
    /// [`Session::content_paragraph_rows`].
    pub fn render_content_targets(&mut self, problem: Option<&str>) {
        self.dom.set_hidden(CONTENT_FORM, true);
        self.dom.set_hidden(CONTENT_OBJECT_FORM, true);
        self.content_target = None;
        self.content_object_target = None;
        self.content_index = -1;
        self.clear_replace_preview(false);
        let count = self.content_scan.as_ref().map_or(0, |s| s.items.len());
        self.dom.set_hidden(CONTENT_TARGETS_EL, count == 0);
        self.dom.set_hidden(CONTENT_EMPTY_EL, count > 0);
        self.dom.set_text(
            CONTENT_EMPTY_EL,
            problem.unwrap_or("No content on this page."),
        );
    }

    /// The listbox rows (R:5785-5810).
    pub fn content_rows(&self) -> Vec<ContentRow> {
        let Some(scan) = &self.content_scan else {
            return Vec::new();
        };
        scan.items
            .iter()
            .enumerate()
            .map(|(index, item)| ContentRow {
                id: format!("content-target-{index}"),
                text: row_text(item),
                meta: row_meta(item),
                selected: index as i64 == self.content_index,
            })
            .collect()
    }

    /// The paragraph buttons (R:5779-5784): one per paragraph whose first run is listed.
    pub fn content_paragraph_rows(&self) -> Vec<ParagraphRow> {
        let Some(scan) = &self.content_scan else {
            return Vec::new();
        };
        scan.paragraphs
            .iter()
            .filter_map(|paragraph| {
                let first_id = paragraph.target_ids.first()?;
                let index = scan
                    .items
                    .iter()
                    .position(|item| &item.target_id == first_id)?;
                let first = &scan.items[index];
                let font = first
                    .font
                    .as_ref()
                    .and_then(|f| f.name.clone())
                    .unwrap_or_else(|| "Text".into());
                Some(ParagraphRow {
                    label: format!(
                        "Paragraph, {} lines, {font} {}",
                        paragraph.lines,
                        js_num(paragraph.font_size)
                    ),
                    item: index,
                })
            })
            .collect()
    }

    /// A click on a paragraph button (R:5783): the on-page paragraph editor.
    pub fn on_content_paragraph_click(&mut self, item: usize) {
        if let Some(first) = self.content_item(item as i64) {
            self.open_inline_text(&first, false);
        }
    }

    /// R:5813.
    pub fn content_item(&self, index: i64) -> Option<EditTarget> {
        let scan = self.content_scan.as_ref()?;
        usize::try_from(index)
            .ok()
            .and_then(|i| scan.items.get(i))
            .cloned()
    }

    /// R:5835: roving tabindex over one listbox (arrows, Home, End). Focus is the UI's.
    pub fn select_content_row(&mut self, index: i64, focus: bool) {
        let _ = focus;
        let rows = self.content_scan.as_ref().map_or(0, |s| s.items.len()) as i64;
        if rows == 0 {
            return;
        }
        let bounded = index.max(0).min(rows - 1);
        self.content_index = bounded;
        let run_item = self.content_item(bounded);
        let disabled = !run_item.is_some_and(|item| is_text(&item) && is_candidate(&item));
        self.dom.set_disabled(CONTENT_SCOPE_RUN, disabled);
        if disabled && self.dom.checked(CONTENT_SCOPE_RUN) {
            self.dom.set_checked(CONTENT_SCOPE_RUN, false);
            self.dom.set_checked(CONTENT_SCOPE_PAGE, true);
        }
    }

    /// R:5850.
    pub fn open_content_form(&mut self, item: EditTarget) {
        self.dom.set_hidden(CONTENT_OBJECT_FORM, true);
        self.content_object_target = None;
        let text = item.text.clone().unwrap_or_default();
        let font = item
            .font
            .as_ref()
            .and_then(|f| f.name.clone())
            .filter(|n| !n.is_empty());
        let label = format!(
            "Replace {}{}",
            json_string(&text),
            font.map(|name| format!(" ({name})")).unwrap_or_default()
        );
        self.dom.set_text(CONTENT_EDIT_LABEL, label);
        self.dom.set_value(CONTENT_NEW_TEXT, text);
        // A deleted run keeps an empty operator; the form shows nothing to type into.
        self.dom.set_text(
            CONTENT_HINT,
            if item.tail_follows == Some(true) {
                "Text follows on this line: the new text must be the same width."
            } else {
                "Shorter text leaves a gap; longer text may not fit."
            },
        );
        // U3: a CID run can only be written with characters its own font already draws here.
        let repertoire = item.repertoire.clone().filter(|r| !r.is_empty());
        self.dom
            .set_hidden(CONTENT_REPERTOIRE, repertoire.is_none());
        self.dom.set_text(
            CONTENT_REPERTOIRE,
            repertoire
                .map(|r| {
                    format!("Only characters already in this font on this page can be used: {r}")
                })
                .unwrap_or_default(),
        );
        self.content_target = Some(item);
        self.dom.set_hidden(CONTENT_FORM, false);
    }

    /// R:5871 (focus returns to the list: the UI's).
    pub fn close_content_form(&mut self, focus_list: bool) {
        self.dom.set_hidden(CONTENT_FORM, true);
        self.content_target = None;
        if self
            .content_scan
            .as_ref()
            .is_none_or(|s| s.items.is_empty())
        {
            return;
        }
        if focus_list && self.content_index >= 0 {
            self.select_content_row(self.content_index, true);
        }
    }

    /// R:5881: selecting a row: candidates open the form, everything else says why not.
    pub fn activate_content_row(&mut self, index: i64) {
        self.select_content_row(index, true);
        let Some(item) = self.content_item(index) else {
            return;
        };
        if !is_text(&item) {
            if is_candidate(&item) {
                self.open_content_object_form(item, true);
            } else {
                let text = reason_text(item.reason.as_deref())
                    .unwrap_or("That object cannot be edited yet.");
                self.announce(text);
            }
            return;
        }
        if !is_candidate(&item) {
            let text =
                reason_text(item.reason.as_deref()).unwrap_or("That content cannot be edited yet.");
            self.announce(text);
            return;
        }
        self.open_content_form(item);
    }

    /// R:5901: one `edit-text` request. Returns false and records the refusal token on any
    /// refusal, so the panel and the smoke share one path.
    pub fn apply_content_edit(
        &mut self,
        scan: &ContentScan,
        item: &EditTarget,
        text: &str,
    ) -> bool {
        let request = json!({
            "op": "edit-text", "page": scan.page, "targetId": item.target_id,
            "generation": scan.generation, "revision": scan.revision, "text": text,
        });
        let reply = match self.request(request, Vec::new()) {
            Ok(reply) => reply,
            Err(err) => {
                let detail = content_detail(&err);
                self.content_refusals.push(ContentRefusal {
                    text: item.text.clone().unwrap_or_default(),
                    detail: detail.clone(),
                });
                // U3: a repertoire refusal names the characters this font cannot draw.
                let missing: Vec<String> = err
                    .reply
                    .get("missing")
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let text = if detail == "unsupported-encoding" && !missing.is_empty() {
                    missing_letters_text(&missing)
                } else {
                    detail_text(&detail)
                        .map(str::to_string)
                        .unwrap_or(err.message.clone())
                };
                self.announce(&text);
                // Edit-forms review #2: in the window a refusal says why and what to do (the
                // editor stays open); a drive keeps STATE `error` as it was.
                if !self.headless {
                    self.show_error(&text);
                }
                return false;
            }
        };
        let from = item.text.clone().unwrap_or_default();
        self.content_edited = Some(ContentEdited::Text {
            from: from.clone(),
            to: text.to_string(),
        });
        self.content_last_shift = reply.get("shift").and_then(Value::as_f64).unwrap_or(0.0);
        let mut notes = Vec::new();
        if self.content_last_shift != 0.0 {
            notes.push(format!(
                "shifted following text by {} pt",
                js_num(self.content_last_shift)
            ));
        }
        if item.kerned == Some(true) {
            notes.push("kerning in this run was dropped".to_string());
        }
        if !notes.is_empty() {
            self.dom.set_text(CONTENT_HINT, notes.join("; "));
        }
        let label = if text.is_empty() {
            format!("Deleted {}", json_string(&from))
        } else {
            format!("Replaced {} with {}", json_string(&from), json_string(text))
        };
        self.apply_edit(&journal_of(&reply.value), &label);
        true
    }

    /// R:5937: the drive's `text-add {page, point, text, font, size, color}`.
    pub fn drive_text_add(&mut self, contents: Option<&str>) {
        let Some(contents) = contents.filter(|c| !c.is_empty()) else {
            return;
        };
        if self.doc.is_none() {
            return;
        }
        let result = (|| -> Result<(), String> {
            let value: Value = serde_json::from_str(contents).map_err(|e| e.to_string())?;
            let mut request = Map::new();
            request.insert("op".into(), json!("text-add"));
            // `{op: "text-add", ...value}`: a key in the contents overrides, `op` included.
            if let Value::Object(map) = value {
                for (key, v) in map {
                    request.insert(crate::engine::esc_key(&key), v);
                }
            }
            let reply = self
                .request(Value::Object(request), Vec::new())
                .map_err(|e| e.message)?;
            let Some(target_id) = reply.get("targetId").and_then(Value::as_str) else {
                return Err("Could not add text.".into());
            };
            let bounds = reply.get("bounds").and_then(number_list);
            self.content_last_add = Some(ContentLastAdd {
                target_id: target_id.to_string(),
                bounds,
            });
            self.apply_edit(&journal_of(&reply.value), "Added text");
            self.refresh_content_targets(true);
            Ok(())
        })();
        if let Err(message) = result {
            self.show_error(&message);
        }
    }

    /// R:5949: "Add text" in the panel.
    pub fn submit_content_add(&mut self) {
        if self.doc.is_none() || !self.can_edit() {
            return;
        }
        let point = match self.content_add_point {
            Some(point) => point.to_vec(),
            None => match self.shape_box() {
                Some(b) if b.len() >= 2 => vec![b[0], b[1]],
                _ => vec![72.0, 72.0],
            },
        };
        let text = self.dom.value(CONTENT_ADD_TEXT);
        if !self.add_text_at(self.wanted_page, &point, &text) {
            return;
        }
        let last = self.last_added_target_id();
        let index = self.content_scan.as_ref().and_then(|s| {
            s.items
                .iter()
                .position(|item| Some(&item.target_id) == last.as_ref())
        });
        if let Some(index) = index {
            self.select_content_row(index as i64, true);
        }
        self.dom.set_value(CONTENT_ADD_TEXT, "");
        self.content_add_point = None;
    }

    /// R:5960: the text form's Apply (or Delete with "").
    pub fn submit_content_edit(&mut self, text: &str) {
        let (Some(scan), Some(item)) = (self.content_scan.clone(), self.content_target.clone())
        else {
            return;
        };
        if self.apply_content_edit(&scan, &item, text) {
            self.close_content_form(false);
        }
    }

    /// R:5969: the first image the picker returns, or None with a status line. The desktop
    /// host always has a picker, so "Choosing an image needs the desktop app." cannot arise.
    pub fn pick_one_image(&mut self) -> Option<(String, Vec<u8>)> {
        match self.host.pick_images() {
            Err(reason) => {
                if reason != "cancelled" {
                    self.announce(if reason == "too-large" {
                        "That image is larger than this app will read."
                    } else {
                        "That image could not be read."
                    });
                }
                None
            }
            Ok(images) => images
                .into_iter()
                .next()
                .map(|first| (first.name, first.bytes)),
        }
    }

    /// The height of the scanned page, for the placement form's "From the top".
    fn content_page_height(&self) -> Option<f64> {
        let page = self.content_scan.as_ref()?.page;
        self.page_size(page).map(|s| s.height).filter(|h| *h > 0.0)
    }

    /// The placement form's page transform: the page's own coordinates (the placement box) to
    /// the shown page, y down from its top edge. The scan's transform carries /Rotate and a
    /// MediaBox or CropBox not at 0 (review fix 4); without one, a flip by the page height,
    /// else the box as it is.
    fn content_form_matrix(&self) -> Vec<f64> {
        if let Some(m) = self
            .content_scan
            .as_ref()
            .and_then(|s| s.transform.clone())
            .filter(|m| m.len() == 6)
        {
            return m;
        }
        match self.content_page_height() {
            Some(h) => vec![1.0, 0.0, 0.0, -1.0, 0.0, h],
            None => vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        }
    }

    /// R:5992: one placement form, filled from the item's own box (focus is the UI's).
    pub fn open_content_object_form(&mut self, item: EditTarget, focus: bool) {
        let _ = focus;
        self.dom.set_hidden(CONTENT_FORM, true);
        self.content_target = None;
        let b = item
            .placement
            .as_ref()
            .map(|p| p.r#box.clone())
            .filter(|b| b.len() >= 4);
        let can_move = b.is_some() && has_op(&item, "move");
        self.dom
            .set_text(CONTENT_OBJECT_LABEL, object_row_label(&item));
        for id in [CONTENT_BOX_X, CONTENT_BOX_Y, CONTENT_BOX_W, CONTENT_BOX_H] {
            self.dom.set_disabled(id, !can_move);
        }
        // Debt sweep 2 §9.8: shown to 2 decimals; an unchanged field passes the unrounded value through on Apply.
        let image = item.image.as_ref();
        // Edit-forms review: "From the top" is measured from the shown page's top edge, as in
        // the field designer (the placement box is in the page's own, bottom-up coordinates).
        let m = self.content_form_matrix();
        let values = match &b {
            Some(b) => placement_fields(b, &m).map(round2),
            None => [
                0.0,
                0.0,
                image.map_or(0.0, |i| i.width as f64),
                image.map_or(0.0, |i| i.height as f64),
            ],
        };
        for (id, value) in [CONTENT_BOX_X, CONTENT_BOX_Y, CONTENT_BOX_W, CONTENT_BOX_H]
            .into_iter()
            .zip(values)
        {
            self.dom.set_value(id, js_num(value));
        }
        self.dom
            .set_hidden(CONTENT_REPLACE_BUTTON, !has_op(&item, "replace"));
        self.dom
            .set_hidden(CONTENT_DELETE_OBJECT_BUTTON, !has_op(&item, "delete"));
        self.dom.set_text(
            CONTENT_OBJECT_HINT,
            if has_op(&item, "move") {
                "Moving or deleting changes only this spot on the page. To remove something for good, use Redact."
            } else {
                "This placement is skewed or nested: the new image is stretched into it."
            },
        );
        self.content_object_target = Some(item);
        self.sync_inspector_transform();
        self.dom.set_hidden(CONTENT_OBJECT_FORM, false);
    }

    /// R:6016.
    pub fn close_content_object_form(&mut self, focus_list: bool) {
        self.dom.set_hidden(CONTENT_OBJECT_FORM, true);
        self.content_object_target = None;
        if self
            .content_scan
            .as_ref()
            .is_none_or(|s| s.items.is_empty())
        {
            return;
        }
        if focus_list && self.content_index >= 0 {
            self.select_content_row(self.content_index, true);
        }
    }

    /// R:6026: one `edit-object` request: the whole refusal travels back, and nothing lands on
    /// a refusal. `extra` is spread into the request; `inputs` are the buffers its
    /// `{"$buffer": i}` placeholders name (a replacement image's bytes).
    pub fn apply_object_edit(
        &mut self,
        scan: &ContentScan,
        item: &EditTarget,
        action: &str,
        extra: Map<String, Value>,
        inputs: Vec<Vec<u8>>,
    ) -> bool {
        let mut request = Map::new();
        for (key, value) in [
            ("op", json!("edit-object")),
            ("page", json!(scan.page)),
            ("targetId", json!(item.target_id)),
            ("generation", json!(scan.generation)),
            ("revision", json!(scan.revision)),
            ("action", json!(action)),
        ] {
            request.insert(key.into(), value);
        }
        for (key, value) in &extra {
            request.insert(key.clone(), value.clone());
        }
        let name = item.object_name.clone().unwrap_or_default();
        let reply = match self.request(Value::Object(request), inputs) {
            Ok(reply) => reply,
            Err(err) => {
                let detail = content_detail(&err);
                self.content_refusals.push(ContentRefusal {
                    text: name,
                    detail: detail.clone(),
                });
                let text = detail_text(&detail)
                    .map(str::to_string)
                    .unwrap_or(err.message);
                self.announce(&text);
                self.show_error(&text);
                return false;
            }
        };
        // P6 U3 §1.2: the same box again is a no-op reply: no journal op, nothing to mirror.
        if reply.get("noop") == Some(&Value::Bool(true)) {
            return true;
        }
        self.content_edited = Some(ContentEdited::Object {
            kind: item.kind.as_str().to_string(),
            action: action.to_string(),
            name,
            count: None,
            skipped: None,
        });
        // Round 2: the announcement uses the placement form's numbers (the shown page, from
        // its top), not the PDF's y-up box.
        let b = extra.get("box").and_then(number_list).map(|b| {
            match scan
                .transform
                .as_deref()
                .filter(|m| m.len() == 6 && b.len() >= 4)
            {
                Some(m) => {
                    let [x, top, w, h] = placement_fields(&b, m);
                    vec![x, top, x + w, top + h]
                }
                None => b,
            }
        });
        let transform = extra.get("transform").and_then(Value::as_str);
        let label = object_announcement(item, action, b.as_deref(), transform);
        self.apply_edit(&journal_of(&reply.value), &label);
        true
    }

    /// R:6055: one `edit-objects` (2..n members, all or nothing); `content.edited` carries `count`.
    pub fn apply_objects_edit(
        &mut self,
        scan: &ContentScan,
        entries: &[ObjectEntry],
        action: &str,
        transform: Option<&str>,
    ) -> bool {
        let names = entries
            .iter()
            .map(|e| e.item.object_name.clone().unwrap_or_default())
            .collect::<Vec<_>>()
            .join(",");
        let items: Vec<Value> = entries
            .iter()
            .map(|entry| {
                let mut member = Map::new();
                member.insert("targetId".into(), json!(entry.item.target_id));
                member.insert("action".into(), json!(action));
                if let Some(t) = transform.filter(|t| !t.is_empty()) {
                    member.insert("transform".into(), json!(t));
                }
                if let Some(b) = &entry.r#box {
                    member.insert("box".into(), json!(b));
                }
                Value::Object(member)
            })
            .collect();
        let request = json!({
            "op": "edit-objects", "page": scan.page, "generation": scan.generation,
            "revision": scan.revision, "items": items,
        });
        let reply = match self.request(request, Vec::new()) {
            Ok(reply) => reply,
            Err(err) => {
                let detail = content_detail(&err);
                self.content_refusals.push(ContentRefusal {
                    text: names,
                    detail: detail.clone(),
                });
                let text = detail_text(&detail)
                    .map(str::to_string)
                    .unwrap_or(err.message);
                self.announce(&text);
                self.show_error(&text);
                return false;
            }
        };
        if reply.get("noop") == Some(&Value::Bool(true)) {
            return true;
        }
        let count = reply
            .get("count")
            .and_then(Value::as_f64)
            .map_or(entries.len() as i64, |c| c as i64);
        self.content_edited = Some(ContentEdited::Object {
            kind: "objects".into(),
            action: action.to_string(),
            name: names,
            count: Some(count),
            skipped: None,
        });
        let verb = match action {
            "move" => "Moved",
            "delete" => "Deleted",
            "resize" => "Resized",
            "transform" if transform.is_some_and(|t| t.starts_with("rotate")) => "Rotated",
            "transform" => "Flipped",
            _ => "Duplicated",
        };
        self.apply_edit(
            &journal_of(&reply.value),
            &format!("{verb} {count} objects"),
        );
        true
    }

    /// R:6099: the object form's Apply (`box` reads the four fields), Delete and Replace….
    pub fn submit_object_edit(&mut self, what: &str) {
        let (Some(scan), Some(item)) = (
            self.content_scan.clone(),
            self.content_object_target.clone(),
        ) else {
            return;
        };
        let ok = match what {
            "delete" => self.apply_object_edit(&scan, &item, "delete", Map::new(), Vec::new()),
            "replace" => {
                let Some((name, bytes)) = self.pick_one_image() else {
                    return;
                };
                let mut extra = Map::new();
                extra.insert(
                    "image".into(),
                    json!({ "name": name, "bytes": { "$buffer": 0 } }),
                );
                self.apply_object_edit(&scan, &item, "replace", extra, vec![bytes])
            }
            _ => {
                let old = item
                    .placement
                    .as_ref()
                    .map(|p| p.r#box.clone())
                    .filter(|b| b.len() >= 4);
                // §9.8: a field still showing round2 of the placement's own value passes the unrounded value through.
                // The fields are the box on the shown page (from its top edge).
                let m = self.content_form_matrix();
                let exact = old.as_ref().map(|o| placement_fields(o, &m));
                let ids = [CONTENT_BOX_X, CONTENT_BOX_Y, CONTENT_BOX_W, CONTENT_BOX_H];
                let mut same = [false; 4];
                let mut v = [0.0; 4];
                for k in 0..4 {
                    let text = self.dom.value(ids[k]);
                    same[k] = exact.is_some_and(|e| text == js_num(round2(e[k])));
                    v[k] = match exact {
                        Some(e) if same[k] => e[k],
                        _ => js_number_of(&text),
                    };
                }
                if !v.iter().all(|n| n.is_finite()) {
                    self.announce("That box is not four numbers.");
                    return;
                }
                // An unchanged edge is the placement's own edge, so an unchanged Apply sends placement.box exactly (a no-op).
                let Some(new_box) = placement_box(old.as_deref(), &m, v) else {
                    self.announce("That box is not four numbers.");
                    return;
                };
                let action = match &exact {
                    Some(e) if (v[2] - e[2]).abs() < 0.05 && (v[3] - e[3]).abs() < 0.05 => "move",
                    _ => "resize",
                };
                let mut extra = Map::new();
                extra.insert("box".into(), json!(new_box));
                self.apply_object_edit(&scan, &item, action, extra, Vec::new())
            }
        };
        if ok {
            self.close_content_object_form(false);
        }
    }

    /// R:6132: one preview request for the current scan. `#content-scope-run` limits it to the
    /// selected text candidate. Nothing is written and no operation is opened.
    pub fn preview_replace(&mut self) {
        let Some(scan) = self.content_scan.clone() else {
            self.announce("Open a page first.");
            return;
        };
        let find = self.dom.value(CONTENT_FIND_INPUT);
        let replace = self.dom.value(CONTENT_REPLACE_WITH);
        if find.is_empty() {
            self.announce("Type the text to find.");
            return;
        }
        let target_id = match &self.content_target {
            Some(target) if self.dom.checked(CONTENT_SCOPE_RUN) && is_text(target) => {
                Some(target.target_id.clone())
            }
            _ => None,
        };
        let mut request = json!({
            "op": "edit-replace-preview", "page": scan.page, "generation": scan.generation,
            "revision": scan.revision, "find": find, "replace": replace,
        });
        if let Some(id) = &target_id {
            request["targetId"] = json!(id);
        }
        match self.request(request, Vec::new()) {
            Ok(reply) => {
                let preview = ReplacePreview {
                    page: scan.page,
                    generation: scan.generation,
                    revision: scan.revision,
                    find,
                    replace,
                    token: reply
                        .get("previewToken")
                        .map(token_text)
                        .unwrap_or_default(),
                    target_id,
                    matches: list_of::<ReplaceMatch>(reply.get("matches")),
                    rejected: list_of::<ReplaceRejected>(reply.get("rejected")),
                };
                let text = format!(
                    "{}.",
                    replace_summary(preview.matches.len(), preview.rejected.len())
                );
                self.replace_preview = Some(preview);
                self.render_replace_preview();
                // Round 2: the results show (a re-drawn panel had folded the section shut).
                self.dom.set_open(CONTENT_REPLACE_PANEL, true);
                self.announce(&text);
            }
            Err(err) => {
                self.clear_replace_preview(false);
                self.announce(&err.message);
            }
        }
    }

    /// R:6165.
    pub fn clear_replace_preview(&mut self, focus_list: bool) {
        self.replace_preview = None;
        self.set_status_line(CONTENT_REPLACE_SUMMARY, "", "");
        self.dom.set_disabled(CONTENT_REPLACE_APPLY, true);
        if focus_list {
            self.dom.set_open(CONTENT_REPLACE_PANEL, false);
        }
    }

    /// R:6174: one list: ticked matches first, then the rejected runs with their reason (the
    /// rows are [`Session::replace_rows`]).
    pub fn render_replace_preview(&mut self) {
        let Some(preview) = &self.replace_preview else {
            self.set_status_line(CONTENT_REPLACE_SUMMARY, "", "");
            self.dom.set_disabled(CONTENT_REPLACE_APPLY, true);
            return;
        };
        let (matches, rejected) = (preview.matches.len(), preview.rejected.len());
        for index in 0..matches {
            self.dom.set_checked(&content_match(index), true);
        }
        self.set_status_line(
            CONTENT_REPLACE_SUMMARY,
            replace_summary(matches, rejected),
            "info",
        );
        self.sync_replace_apply();
    }

    /// The preview list's rows (R:6183-6202).
    pub fn replace_rows(&self) -> Vec<ReplaceRow> {
        let Some(preview) = &self.replace_preview else {
            return Vec::new();
        };
        let mut rows: Vec<ReplaceRow> = preview
            .matches
            .iter()
            .enumerate()
            .map(|(index, m)| ReplaceRow {
                checkbox: Some(content_match(index)),
                text: format!(
                    " {} → {}{}",
                    m.text,
                    m.next,
                    if m.count > 1 {
                        format!(" ({})", m.count)
                    } else {
                        String::new()
                    }
                ),
            })
            .collect();
        rows.extend(preview.rejected.iter().map(|r| ReplaceRow {
            checkbox: None,
            text: format!(
                "{}: {}",
                r.text,
                detail_text(&r.detail)
                    .or_else(|| reason_text(Some(&r.detail)))
                    .unwrap_or("it can't be changed here")
            ),
        }));
        rows
    }

    /// A match checkbox changed (R:6190).
    pub fn on_content_match_change(&mut self, index: usize, checked: bool) {
        self.dom.set_checked(&content_match(index), checked);
        self.sync_replace_apply();
    }

    /// R:6208.
    pub fn checked_replace_ids(&self) -> Vec<String> {
        let Some(preview) = &self.replace_preview else {
            return Vec::new();
        };
        preview
            .matches
            .iter()
            .enumerate()
            .filter(|(index, _)| self.dom.checked(&content_match(*index)))
            .map(|(_, m)| m.target_id.clone())
            .collect()
    }

    /// R:6215.
    pub fn sync_replace_apply(&mut self) {
        let none = self.checked_replace_ids().is_empty();
        self.dom.set_disabled(CONTENT_REPLACE_APPLY, none);
    }

    /// R:6220: one `edit-replace-apply`; all ticked runs or none.
    pub fn apply_replace(&mut self) {
        let Some(preview) = self.replace_preview.clone() else {
            return;
        };
        let target_ids = self.checked_replace_ids();
        if target_ids.is_empty() {
            return;
        }
        let mut request = json!({
            "op": "edit-replace-apply", "page": preview.page, "generation": preview.generation,
            "revision": preview.revision, "find": preview.find, "replace": preview.replace,
            "previewToken": preview.token, "targetIds": target_ids,
        });
        if let Some(id) = preview.target_id.as_ref().filter(|id| !id.is_empty()) {
            request["targetId"] = json!(id);
        }
        let reply = match self.request(request, Vec::new()) {
            Ok(reply) => reply,
            Err(err) => {
                let detail = err.detail.clone().unwrap_or_else(|| "invalid".into());
                let text = detail_text(&detail)
                    .map(str::to_string)
                    .unwrap_or(err.message);
                self.announce(&text);
                self.set_status_line(CONTENT_REPLACE_SUMMARY, user_text(&text), "error");
                return;
            }
        };
        let replaced = js_number_value(reply.get("replaced"), target_ids.len() as f64);
        // Round 2: "Replaced “ALPHA” in 3 places" (no "runs"), said in the panel and the notice
        // bar; an earlier refusal's notice goes.
        let label = format!(
            "Replaced “{}” in {} place{}",
            preview.find,
            js_num(replaced),
            if replaced == 1.0 { "" } else { "s" }
        );
        self.dom.set_open(CONTENT_REPLACE_PANEL, true);
        self.apply_edit(&journal_of(&reply.value), &label);
        self.clear_error();
        if !self.headless {
            self.show_message(&format!("{label}."), MessageKind::Success);
        }
        self.set_status_line(CONTENT_REPLACE_SUMMARY, format!("{label}."), "success");
    }

    /// R:6244: the U5 drive path: preview the whole page (or one run), then apply every match.
    pub fn smoke_replace(&mut self, wanted: &Map<String, Value>) {
        if let Some(page) = wanted.get("page").and_then(Value::as_f64) {
            self.go_to(page.trunc() as i64);
        }
        self.refresh_content_targets(true);
        let Some(scan) = self.content_scan.clone() else {
            return;
        };
        let find = wanted
            .get("find")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let replace = wanted
            .get("replace")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let target_item = wanted
            .get("target")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .and_then(|t| {
                scan.items
                    .iter()
                    .find(|item| is_text(item) && item.text.as_deref() == Some(t))
            })
            .cloned();
        let mut request = json!({
            "op": "edit-replace-preview", "page": scan.page, "generation": scan.generation,
            "revision": scan.revision, "find": find, "replace": replace,
        });
        if let Some(item) = &target_item {
            request["targetId"] = json!(item.target_id);
        }
        let preview = match self.request(request, Vec::new()) {
            Ok(reply) => reply,
            Err(err) => {
                self.content_replaced = Some(ContentReplaced {
                    find,
                    replace,
                    applied: 0,
                    matches: 0,
                    rejected: vec![ContentRefusal {
                        text: String::new(),
                        detail: "invalid".into(),
                    }],
                });
                self.announce(&err.message);
                return;
            }
        };
        let matches = list_of::<ReplaceMatch>(preview.get("matches"));
        let rejected: Vec<ContentRefusal> = list_of::<ReplaceRejected>(preview.get("rejected"))
            .into_iter()
            .map(|r| ContentRefusal {
                text: r.text,
                detail: r.detail,
            })
            .collect();
        if wanted.get("preview") == Some(&Value::Bool(true)) || matches.is_empty() {
            self.content_replaced = Some(ContentReplaced {
                find,
                replace,
                applied: 0,
                matches: matches.len() as i64,
                rejected,
            });
            return;
        }
        let mut request = json!({
            "op": "edit-replace-apply", "page": scan.page, "generation": scan.generation,
            "revision": scan.revision, "find": find, "replace": replace,
            "targetIds": matches.iter().map(|m| m.target_id.clone()).collect::<Vec<_>>(),
        });
        if let Some(token) = preview.get("previewToken") {
            request["previewToken"] = token.clone();
        }
        if let Some(item) = &target_item {
            request["targetId"] = json!(item.target_id);
        }
        match self.request(request, Vec::new()) {
            Ok(applied) => {
                let count = js_number_value(applied.get("replaced"), matches.len() as f64);
                self.content_replaced = Some(ContentReplaced {
                    find: find.clone(),
                    replace,
                    applied: count as i64,
                    matches: matches.len() as i64,
                    rejected,
                });
                self.apply_edit(
                    &journal_of(&applied.value),
                    &format!("Replaced {find} in {} runs", js_num(count)),
                );
            }
            Err(err) => {
                let mut rejected = rejected;
                rejected.push(ContentRefusal {
                    text: String::new(),
                    detail: err.detail.clone().unwrap_or_else(|| "invalid".into()),
                });
                self.content_replaced = Some(ContentReplaced {
                    find,
                    replace,
                    applied: 0,
                    matches: matches.len() as i64,
                    rejected,
                });
                self.announce(&err.message);
            }
        }
    }

    /// R:6289: STATE `content` (and the `P3_EDIT` payload).
    pub fn content_summary(&self) -> Value {
        let empty = Vec::new();
        let items = self.content_scan.as_ref().map_or(&empty, |s| &s.items);
        let m = self.content_scan.as_ref().and_then(|s| s.transform.clone());
        let bounds: Vec<Value> = items
            .iter()
            .filter(|item| is_text(item) && item.bounds.is_some())
            .map(|item| {
                let mut entry = Map::new();
                entry.insert("targetId".into(), json!(item.target_id));
                entry.insert(
                    "bounds".into(),
                    numbers(item.bounds.as_deref().unwrap_or(&[])),
                );
                if let Some(size) = item.font_size {
                    entry.insert("fontSize".into(), js_number(size));
                }
                if let Some(paragraph) = &item.paragraph_id {
                    entry.insert("paragraphId".into(), json!(paragraph));
                }
                Value::Object(entry)
            })
            .collect();
        let objects: Vec<Value> = items
            .iter()
            .filter(|item| !is_text(item))
            .map(|item| {
                let b = item.placement.as_ref().map(|p| p.r#box.clone());
                let placement = item.placement.clone();
                let rounded = |values: &[f64]| {
                    Value::Array(values.iter().map(|v| js_number(round2(*v))).collect())
                };
                let view = match (&b, &m) {
                    (Some(b), Some(m)) => rounded(&super::nav::view_box(b, m)),
                    _ => Value::Null,
                };
                let mut entry = Map::new();
                entry.insert("targetId".into(), json!(item.target_id));
                entry.insert("kind".into(), json!(item.kind.as_str()));
                entry.insert(
                    "name".into(),
                    json!(item.object_name.clone().unwrap_or_default()),
                );
                entry.insert("box".into(), b.as_deref().map_or(Value::Null, rounded));
                entry.insert("view".into(), view);
                entry.insert(
                    "ops".into(),
                    json!(
                        item.object_ops
                            .as_ref()
                            .map(|ops| ops.iter().map(|o| o.as_str()).collect::<Vec<_>>())
                            .unwrap_or_default()
                    ),
                );
                // P6 U6 §2.6: `orient` verbatim (null without a placement); `full` (view space) only when cropped.
                entry.insert(
                    "orient".into(),
                    placement.as_ref().and_then(|p| p.orient).map_or(
                        Value::Null,
                        |o| json!({ "swap": o.swap, "sx": o.sx, "sy": o.sy }),
                    ),
                );
                if let (Some(full), Some(m)) = (placement.as_ref().and_then(|p| p.full.clone()), &m)
                {
                    entry.insert("full".into(), rounded(&super::nav::view_box(&full, m)));
                }
                Value::Object(entry)
            })
            .collect();
        let last_add = self.content_last_add.as_ref().map_or(Value::Null, |add| {
            json!({
                "targetId": add.target_id,
                "bounds": add.bounds.as_deref().map_or(Value::Null, numbers),
            })
        });
        json!({
            "targets": items.len(),
            "paragraphs": self.content_scan.as_ref().map_or(0, |s| s.paragraphs.len()),
            "bounds": bounds,
            "objects": objects,
            "candidates": items.iter().filter(|item| is_candidate(item)).count(),
            // print-edit D7: print-to-PDF glyph lines on the shown page.
            "glyphLines": items.iter().filter(|item| item.glyphs.is_some()).count(),
            "edited": serde_json::to_value(&self.content_edited).unwrap_or(Value::Null),
            "last": { "shift": js_number(self.content_last_shift) },
            "lastAdd": last_add,
            "replaced": serde_json::to_value(&self.content_replaced).unwrap_or(Value::Null),
            "refused": serde_json::to_value(&self.content_refusals).unwrap_or(Value::Null),
        })
    }

    /// R:6311.
    pub fn log_content_smoke(&mut self) {
        let payload = json!({ "content": self.content_summary() });
        self.console_line("P3_EDIT", payload);
    }

    /// R:6316: `edit-request {op:'content-edit'}` from the menu: show the list, select the
    /// first text candidate.
    pub fn open_content_editor(&mut self) {
        self.select_tab_by_id(&INSPECTOR_TABS, &INSPECTOR_PANELS, "tab-text");
        self.set_inspector_open(true);
        self.refresh_content_targets(true);
        let first = self.content_scan.as_ref().and_then(|s| {
            s.items
                .iter()
                .position(|item| is_text(item) && is_candidate(item))
        });
        self.select_content_row(first.map_or(0, |i| i as i64), true);
    }

    /// R:6325: `--edit=content` and the drive's `content` step, logged as one `P3_EDIT` line.
    pub fn smoke_content(&mut self, contents: Option<&str>) {
        self.content_smoke_active = true;
        self.content_refusals.clear();
        self.content_edited = None;
        self.content_replaced = None;
        let mut wanted = Map::new();
        // §4.3: a plain string (`"hello"`) is not a step either; only null (the --edit=content smoke) reaches the self-test.
        if let Some(text) = contents {
            if !js_trim(text).starts_with('{') {
                self.show_error("content: give text, find or action");
                self.log_content_smoke();
                return;
            }
            match serde_json::from_str::<Value>(text) {
                Err(err) => {
                    self.show_error(&format!("content: {err}"));
                    self.log_content_smoke();
                    return;
                }
                Ok(parsed) => {
                    // A well-formed content step starts clean (like find), so a stale refusal never outlives it.
                    self.clear_error();
                    let parsed = match parsed {
                        Value::Object(map) => map,
                        _ => Map::new(),
                    };
                    // A locked fixture is unlocked first with the payload's password (the smoke's only password path).
                    if let Some(password) = parsed.get("password").filter(|p| truthy(p)) {
                        let password = password
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| json_text(password));
                        self.smoke_password(Some(&password));
                    }
                    let has = |key: &str| parsed.get(key).is_some_and(Value::is_string);
                    if !has("text") && !has("find") && !has("action") && !has("password") {
                        self.show_error("content: give text, find or action");
                        self.log_content_smoke();
                        return;
                    }
                    wanted = parsed;
                }
            }
        }
        if wanted.get("find").is_some_and(Value::is_string) {
            self.smoke_replace(&wanted);
            self.log_content_smoke();
            return;
        }
        self.refresh_content_targets(true);
        let items = self
            .content_scan
            .as_ref()
            .map(|s| s.items.clone())
            .unwrap_or_default();
        let target = wanted.get("target");
        if let Some(action) = wanted.get("action").and_then(Value::as_str) {
            let action = action.to_string();
            let item = items.iter().find(|entry| {
                !is_text(entry)
                    && match target {
                        None => {
                            is_candidate(entry)
                                && entry.object_ops.as_ref().is_some_and(|ops| !ops.is_empty())
                        }
                        Some(t) => t
                            .as_str()
                            .is_some_and(|t| entry.object_name.as_deref() == Some(t)),
                    }
            });
            match (item.cloned(), self.content_scan.clone()) {
                (Some(item), Some(scan)) => {
                    self.smoke_object_action(&scan, &item, &action, wanted.get("box"))
                }
                _ => {
                    let shown = json_text(target.unwrap_or(&json!("")));
                    self.announce(&format!("No object target {shown} on this page"));
                }
            }
            self.log_content_smoke();
            return;
        }
        if let Some(text) = wanted.get("text").and_then(Value::as_str) {
            let text = text.to_string();
            let target_id = match wanted.get("targetId") {
                Some(Value::String(id)) if id == "$lastAdd" => {
                    self.last_added_target_id().map(Value::String)
                }
                other => other.cloned(),
            };
            let item = items.iter().find(|entry| {
                is_text(entry)
                    && match &target_id {
                        Some(id) if truthy(id) => id.as_str() == Some(entry.target_id.as_str()),
                        _ => match target {
                            None => is_candidate(entry),
                            Some(t) => t.as_str().is_some_and(|t| entry.text.as_deref() == Some(t)),
                        },
                    }
            });
            match (item.cloned(), self.content_scan.clone()) {
                (Some(item), Some(scan)) => {
                    self.apply_content_edit(&scan, &item, &text);
                }
                _ => {
                    let shown = json_text(target.unwrap_or(&json!("")));
                    self.announce(&format!("No text target {shown} on this page"));
                }
            }
            self.log_content_smoke();
            return;
        }
        let candidate_text = |item: &EditTarget, text: &str| {
            is_text(item) && is_candidate(item) && item.text.as_deref() == Some(text)
        };
        let first = items
            .iter()
            .find(|item| is_candidate(item) && item.text.as_deref() == Some("ALPHA"))
            .or_else(|| {
                items
                    .iter()
                    .find(|item| is_text(item) && is_candidate(item))
            })
            .cloned();
        if let (Some(first), Some(scan)) = (first, self.content_scan.clone()) {
            self.apply_content_edit(&scan, &first, "BETA");
        }
        // Re-scan after every write: handles are revision-bound and growth changes the
        // target ordering. Exercise available-space growth, then an actual overflow.
        self.refresh_content_targets(true);
        let items = self
            .content_scan
            .as_ref()
            .map(|s| s.items.clone())
            .unwrap_or_default();
        let has_tail = items
            .iter()
            .any(|item| is_text(item) && item.text.as_deref() == Some(" TAIL"));
        let mut next = if has_tail {
            items
                .iter()
                .find(|item| candidate_text(item, " TAIL"))
                .cloned()
        } else {
            items
                .iter()
                .find(|item| candidate_text(item, "BETA"))
                .cloned()
        };
        if !has_tail && let (Some(item), Some(scan)) = (next.clone(), self.content_scan.clone()) {
            self.apply_content_edit(&scan, &item, "BRAVO");
            self.refresh_content_targets(true);
            let items = self
                .content_scan
                .as_ref()
                .map(|s| s.items.clone())
                .unwrap_or_default();
            next = items
                .iter()
                .find(|item| candidate_text(item, "BRAVO"))
                .or_else(|| items.iter().find(|item| candidate_text(item, "BETA")))
                .cloned();
        }
        if let (Some(item), Some(scan)) = (next, self.content_scan.clone()) {
            self.apply_content_edit(&scan, &item, &"BRAVO".repeat(120));
        }
        self.log_content_smoke();
    }

    /// R:6405: U4 drive path: one placement action from a script line (`box` or the picker's image).
    pub fn smoke_object_action(
        &mut self,
        scan: &ContentScan,
        item: &EditTarget,
        action: &str,
        r#box: Option<&Value>,
    ) {
        if action == "delete" {
            self.apply_object_edit(scan, item, "delete", Map::new(), Vec::new());
            return;
        }
        if action == "replace" {
            let Some((name, bytes)) = self.pick_one_image() else {
                return;
            };
            let mut extra = Map::new();
            extra.insert(
                "image".into(),
                json!({ "name": name, "bytes": { "$buffer": 0 } }),
            );
            self.apply_object_edit(scan, item, "replace", extra, vec![bytes]);
            return;
        }
        if action != "move" && action != "resize" {
            self.announce(&format!("Unknown object action {}", json_string(action)));
            return;
        }
        let valid = r#box
            .and_then(Value::as_array)
            .filter(|b| b.len() == 4 && b.iter().all(|v| v.as_f64().is_some_and(f64::is_finite)));
        let Some(b) = valid else {
            self.announce("An object action needs a box of four numbers.");
            return;
        };
        let mut extra = Map::new();
        extra.insert("box".into(), Value::Array(b.clone()));
        self.apply_object_edit(scan, item, action, extra, Vec::new());
    }
}

/// JavaScript truthiness of a JSON value.
pub(crate) fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|v| v != 0.0 && !v.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// An array of numbers, or None.
fn number_list(value: &Value) -> Option<Vec<f64>> {
    value.as_array()?.iter().map(Value::as_f64).collect()
}

/// Numbers as JSON the way JavaScript prints them.
fn numbers(values: &[f64]) -> Value {
    Value::Array(values.iter().map(|v| js_number(*v)).collect())
}

/// `String(reply.previewToken ?? "")`.
fn token_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => json_text(other),
    }
}

/// `Number(value ?? fallback)`.
fn js_number_value(value: Option<&Value>, fallback: f64) -> f64 {
    match value {
        None | Some(Value::Null) => fallback,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::String(s)) => js_number_of(s),
        Some(_) => f64::NAN,
    }
}

/// A reply list, items that do not parse dropped.
fn list_of<T: serde::de::DeserializeOwned>(value: Option<&Value>) -> Vec<T> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(super::parse_reply).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tandem_protocol::engine::{EditObjectImage, Placement};

    fn fixture(name: &str) -> Vec<u8> {
        let path = format!("{}/../../fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read(path).unwrap()
    }

    #[test]
    fn a_refused_page_has_no_targets_from_the_last_document() {
        // Round 2 BLOCKING: edit-objects.pdf's text and images must not stay selectable on a
        // page whose scan is refused; the panel says why and the Text tool still adds text.
        let mut s = Session::new();
        s.open_document(
            "edit-objects.pdf",
            fixture("p3/edit-objects.pdf"),
            None,
            false,
        );
        s.set_mode(super::super::types::UiMode::Edit);
        assert!(s.content_scan.as_ref().is_some_and(|c| !c.items.is_empty()));
        s.open_document(
            "targets-unsupported.pdf",
            fixture("p3/targets-unsupported.pdf"),
            None,
            false,
        );
        s.refresh_content_targets(true);
        assert!(s.content_scan.is_none(), "the old scan is gone");
        assert!(
            !s.lane2.hit.nodes.iter().any(|n| n.target_id().is_some()),
            "no content targets on the refused page"
        );
        let text = s.dom.text(CONTENT_EMPTY_EL);
        assert!(
            text.starts_with("This page's text can't be edited"),
            "{text}"
        );
        assert!(!text.contains("edit-targets"), "{text}");
        assert_eq!(
            s.content_refused.as_ref().map(|r| r.reason.as_str()),
            Some("invalid")
        );
        // The Text tool on that page opens a text box (a comment) where the click was.
        s.on_text_add_tool_click(false);
        s.on_edit_request(
            "click",
            Some(json!({"page": 0, "x": 100, "y": 100}).to_string()),
        );
        let active = s.direct_active.as_ref().map(|a| a.kind);
        assert_eq!(active, Some(super::super::types::DirectKind::Textbox));
    }

    #[test]
    fn the_placement_form_reads_the_shown_page() {
        // Review fix 4: a /Rotate 90 page whose CropBox is [10 20 210 320]: the shown page is
        // 300 wide and 200 tall, its top edge the box's left edge.
        let m = [0.0, 1.0, 1.0, 0.0, -20.0, -10.0];
        let placed = [30.0, 50.0, 70.0, 150.0];
        let fields = placement_fields(&placed, &m);
        assert_eq!(fields, [30.0, 20.0, 100.0, 40.0]);
        // Unchanged, Apply sends the placement's own box.
        assert_eq!(
            placement_box(Some(&placed), &m, fields).unwrap(),
            placed.to_vec()
        );
        // 15 points lower on the shown page is 15 to the right in the page's coordinates.
        let moved = placement_box(Some(&placed), &m, [30.0, 35.0, 100.0, 40.0]).unwrap();
        assert_eq!(moved, vec![45.0, 50.0, 85.0, 150.0]);
        // An unrotated page with its MediaBox at [0 100 612 892]: from the top is from y 892.
        let m = [1.0, 0.0, 0.0, -1.0, 0.0, 892.0];
        let placed = [72.0, 700.0, 172.0, 800.0];
        assert_eq!(placement_fields(&placed, &m), [72.0, 92.0, 100.0, 100.0]);
        assert_eq!(
            placement_box(Some(&placed), &m, [72.0, 0.0, 100.0, 100.0]).unwrap(),
            vec![72.0, 792.0, 172.0, 892.0]
        );
        assert!(placement_box(None, &[0.0; 6], fields).is_none());
    }

    fn image(b: Vec<f64>) -> EditTarget {
        EditTarget {
            kind: EditTargetKind::Image,
            placement: Some(Placement {
                r#box: b,
                ..Placement::default()
            }),
            ..EditTarget::default()
        }
    }

    #[test]
    fn font_names_for_people() {
        assert_eq!(
            font_display_name("NotoSansHebrew[wdth,wght].ttf"),
            "NotoSansHebrew"
        );
        assert_eq!(font_display_name("ABCDEF+Helvetica"), "Helvetica");
        assert_eq!(font_display_name("Times-Roman"), "Times-Roman");
    }

    #[test]
    fn object_labels() {
        assert_eq!(
            object_row_label(&image(vec![10.0, 20.0, 74.0, 84.04])),
            "Image 64×64 pt"
        );
        let form = EditTarget {
            kind: EditTargetKind::Form,
            object_name: Some("Fm1".into()),
            ..EditTarget::default()
        };
        assert_eq!(object_row_label(&form), "Drawing");
        let pixels = EditTarget {
            kind: EditTargetKind::Image,
            image: Some(EditObjectImage {
                width: 8,
                height: 4,
                ..EditObjectImage::default()
            }),
            ..EditTarget::default()
        };
        assert_eq!(object_row_label(&pixels), "Image 8×4 px");
        let unnamed = EditTarget {
            kind: EditTargetKind::Form,
            ..image(vec![0.0, 0.0, 1.0, 2.0])
        };
        assert_eq!(object_row_label(&unnamed), "Drawing 1×2 pt");
    }

    #[test]
    fn object_states_and_announcements() {
        let item = image(vec![10.0, 20.0, 74.0, 84.0]);
        assert_eq!(object_row_state(&item), "Not editable");
        let shared = EditTarget {
            reason: Some("shared-placement".into()),
            ..item.clone()
        };
        assert_eq!(
            object_row_state(&shared),
            "This object is used in several places: edit one placement at a time"
        );
        assert_eq!(
            object_announcement(&item, "move", Some(&[12.34, 5.0, 0.0, 0.0]), None),
            "Moved image to 12.3 from the left, 5 from the top"
        );
        assert_eq!(
            object_announcement(&item, "resize", Some(&[0.0, 0.0, 10.25, 4.0]), None),
            "Resized image to 10.3×4"
        );
        assert_eq!(
            object_announcement(&item, "transform", None, Some("flip-h")),
            "Flipped image"
        );
        assert_eq!(
            object_announcement(&item, "move", None, None),
            "Moved image"
        );
    }

    #[test]
    fn js_number_parsing() {
        assert_eq!(js_number_of(""), 0.0);
        assert_eq!(js_number_of(" 12.5 "), 12.5);
        assert_eq!(js_number_of("0x10"), 16.0);
        assert!(js_number_of("inf").is_nan());
        assert!(js_number_of("1,5").is_nan());
        assert_eq!(js_number_of("-Infinity"), f64::NEG_INFINITY);
        assert_eq!(js_num(round2(1.005)), "1");
        assert_eq!(
            json_text(&json!({"a": [1.0, 2.5], "b": "x"})),
            r#"{"a":[1,2.5],"b":"x"}"#
        );
    }
}
