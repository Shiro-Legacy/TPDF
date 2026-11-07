//! The Form tool (design form, phase 1): a field placed by a click (its default size, its
//! top-left at the click) or a drag (that box), choice buttons one click each, names read
//! off the page, and the picked field's settings in a popover beside it. The popover shows
//! and changes the Fields panel's own `field-prop-*` inputs, so both apply the same way.

use crate::session::Session;
use crate::session::dom_ids as ids;
use serde_json::{Value, json};
use tandem_protocol::engine::{FieldKind, Line};
use tandem_protocol::ui::FieldPopover;

/// The tool's variants that place a field (the rest, `select`, picks and moves them).
pub const FIELD_KINDS: [&str; 5] = ["text", "checkbox", "radio", "combo", "signature"];

/// A group of choice buttons being placed: the first click waits for the second (a group
/// has at least two), later clicks add to the group.
#[derive(Clone, Debug, PartialEq)]
pub struct RadioDraft {
    pub page: usize,
    pub first: [f64; 4],
    /// The group's first widget once it exists.
    pub group: Option<i64>,
    /// Its name, for the status line.
    pub name: String,
}

/// The variant as a field kind the tool places, if it is one.
pub fn field_kind(variant: Option<&str>) -> Option<&'static str> {
    let variant = variant?;
    FIELD_KINDS.iter().copied().find(|k| *k == variant)
}

/// A field's size when it is placed by a click (points).
pub fn default_size(kind: &str) -> (f64, f64) {
    match kind {
        "checkbox" | "radio" => (14.0, 14.0),
        "combo" => (140.0, 22.0),
        "signature" => (180.0, 40.0),
        _ => (180.0, 22.0),
    }
}

/// The smallest box a drag makes (a smaller drag is taken as a click).
fn min_size(kind: &str) -> (f64, f64) {
    match kind {
        "signature" => (40.0, 20.0),
        _ => (6.0, 6.0),
    }
}

/// A clicked field's box: its default size with the top-left at the point, moved back
/// inside the page when it would cross an edge.
pub fn placed_rect(kind: &str, point: [f64; 2], page: (f64, f64)) -> [f64; 4] {
    let (w, h) = default_size(kind);
    let (w, h) = (w.min(page.0), h.min(page.1));
    let x = point[0].clamp(0.0, (page.0 - w).max(0.0));
    let y = point[1].clamp(0.0, (page.1 - h).max(0.0));
    [x, y, x + w, y + h]
}

/// A drawn box, or the clicked default when the drag was too small to mean a size.
pub fn field_rect(
    kind: &str,
    start: [f64; 2],
    drawn: Option<&[f64]>,
    page: (f64, f64),
) -> [f64; 4] {
    let (min_w, min_h) = min_size(kind);
    match drawn {
        Some(r) if r.len() == 4 && r[2] - r[0] >= min_w && r[3] - r[1] >= min_h => {
            [r[0], r[1], r[2], r[3]]
        }
        _ => placed_rect(kind, start, page),
    }
}

/// A line of page text as a label: trailing colons, stars, underscores and dots gone.
fn label_text(text: &str) -> Option<String> {
    let trimmed = text
        .trim()
        .trim_end_matches(|c: char| matches!(c, ':' | '*' | '_' | '.' | '…') || c.is_whitespace())
        .trim();
    let words = trimmed.chars().filter(|c| c.is_alphanumeric()).count();
    (words > 0 && trimmed.chars().count() <= 80).then(|| trimmed.to_string())
}

/// A line's box in page points (y down): `y` is its baseline, `h` its height.
fn line_box(line: &Line) -> [f64; 4] {
    [
        line.x,
        line.y - line.h,
        line.x + line.w,
        line.y + line.h * 0.25,
    ]
}

/// The text that labels a field at `rect`: the nearest line ending to its left on the same
/// row (within 200 pt), else the nearest line just above it (within 30 pt) that overlaps it
/// across. Lines that are only underscores (the blank to fill) are not labels.
pub fn label_near(lines: &[Line], rect: &[f64; 4]) -> Option<String> {
    let mid = (rect[1] + rect[3]) / 2.0;
    let left = lines
        .iter()
        .filter_map(|line| {
            let b = line_box(line);
            let same_row = b[1] <= rect[3] && b[3] >= rect[1] && (b[1] <= mid || b[3] >= mid);
            let gap = rect[0] - b[2];
            (same_row && (-4.0..=200.0).contains(&gap))
                .then(|| label_text(&line.text).map(|t| (gap, t)))
                .flatten()
        })
        .min_by(|a, b| a.0.total_cmp(&b.0));
    if let Some((_, text)) = left {
        return Some(text);
    }
    lines
        .iter()
        .filter_map(|line| {
            let b = line_box(line);
            let gap = rect[1] - b[3];
            let across = b[0] < rect[2] && b[2] > rect[0];
            (across && (-2.0..=30.0).contains(&gap))
                .then(|| label_text(&line.text).map(|t| (gap, t)))
                .flatten()
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, text)| text)
}

/// The words beside a choice button (to its right, on its row, within 40 pt): its choice.
pub fn label_right(lines: &[Line], rect: &[f64; 4]) -> Option<String> {
    let mid = (rect[1] + rect[3]) / 2.0;
    lines
        .iter()
        .filter_map(|line| {
            let b = line_box(line);
            let gap = b[0] - rect[2];
            (b[1] <= mid && b[3] >= mid && (-2.0..=40.0).contains(&gap))
                .then(|| label_text(&line.text).map(|t| (gap, t)))
                .flatten()
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, text)| text.chars().take(64).collect())
}

fn nums(rect: &[f64; 4]) -> Value {
    json!(
        rect.iter()
            .map(|v| (v * 100.0).round() / 100.0)
            .collect::<Vec<_>>()
    )
}

fn word(kind: &str) -> &'static str {
    match kind {
        "checkbox" => "checkbox",
        "radio" => "choice buttons",
        "combo" => "drop-down list",
        "signature" => "signature field",
        _ => "text field",
    }
}

impl Session {
    /// The shown page's size in points.
    fn form_page_size(&self, page: usize) -> Option<(f64, f64)> {
        let size = self.doc.as_ref()?.pages.get(page)?;
        Some((size.width, size.height))
    }

    /// The page's text lines when `page` is the one painted (names are read from them).
    fn form_lines(&self, page: usize) -> Vec<Line> {
        if self.shown_page != page as i64 {
            return Vec::new();
        }
        self.text_layer().map(<[Line]>::to_vec).unwrap_or_default()
    }

    /// Whether fields can be designed here; says why not when they cannot.
    fn form_can_design(&mut self) -> bool {
        let xfa = self.doc.as_ref().is_some_and(|d| d.info.xfa);
        if xfa {
            self.show_error(
                "This form is built in a way (XFA) that doesn't let fields be added here.",
            );
            return false;
        }
        if !self.can_design() {
            self.show_error("This file does not allow adding form fields.");
            return false;
        }
        true
    }

    /// A press and release with a field kind armed: a click (`drawn` None or too small)
    /// places the kind's default box at `start`, a drag the box drawn.
    pub(crate) fn form_place(
        &mut self,
        page: usize,
        kind: &str,
        start: [f64; 2],
        drawn: Option<&[f64]>,
    ) {
        if self.doc.is_none() || !self.form_can_design() {
            return;
        }
        let Some(size) = self.form_page_size(page) else {
            return;
        };
        let rect = field_rect(kind, start, drawn, size);
        if kind == "radio" {
            self.form_radio_click(page, rect);
            return;
        }
        let lines = self.form_lines(page);
        let label = label_near(&lines, &rect);
        let mut request = json!({
            "op": "field-add",
            "page": page,
            "kind": kind,
            "rect": nums(&rect),
            "nameHint": label.clone().unwrap_or_default(),
        });
        if let Some(label) = &label {
            request["label"] = json!(label);
        }
        if kind == "combo" {
            request["options"] = json!(["Choice 1", "Choice 2"]);
        }
        self.form_add(request, kind);
    }

    /// Sends one field-add; on success the field is picked with its popover open.
    fn form_add(&mut self, request: Value, kind: &str) -> Option<i64> {
        match self.request(request, vec![]) {
            Ok(reply) => {
                let name = reply
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let id = reply.get("widgetId").and_then(Value::as_i64);
                self.form_added(&reply, &format!("Added {} {name}", word(kind)));
                if kind != "radio" {
                    // Placed: back to picking, with the new field open to set it up.
                    self.ui.variant = Some("select".into());
                }
                if let Some(id) = id {
                    self.form_open_popover(id);
                }
                id
            }
            Err(err) => {
                self.refuse_designer("add", "", &err);
                None
            }
        }
    }

    fn form_added(&mut self, reply: &crate::engine::Reply, said: &str) {
        self.set_status_line(ids::FIELDS_STATUS_EL, said.to_string(), "success");
        self.clear_error();
        self.apply_edit(&crate::session::reply_journal_of(reply), "");
        self.refresh_fields();
        self.announce(said);
    }

    /// One click with Choice buttons armed: the first waits for the second, the second
    /// makes the group of two, the rest join it.
    fn form_radio_click(&mut self, page: usize, rect: [f64; 4]) {
        let lines = self.form_lines(page);
        let choice = |rect: &[f64; 4], n: usize| {
            label_right(&lines, rect)
                .filter(|t| t != "Off")
                .unwrap_or_else(|| format!("Choice {n}"))
        };
        let draft = self.ui.radio_draft.clone().filter(|d| d.page == page);
        match draft {
            Some(RadioDraft {
                group: Some(group),
                name,
                ..
            }) => {
                let request = json!({
                    "op": "field-kid-add", "page": page, "widgetId": group, "rect": nums(&rect),
                });
                let mut request = request;
                if let Some(on) = label_right(&lines, &rect).filter(|t| t != "Off") {
                    request["on"] = json!(on);
                }
                let mut reply = self.request(request.clone(), vec![]);
                if reply.is_err() && request.get("on").is_some() {
                    // The words beside it name another button already: number it instead.
                    if let Some(map) = request.as_object_mut() {
                        map.remove("on");
                    }
                    reply = self.request(request, vec![]);
                }
                match reply {
                    Ok(reply) => {
                        let on = reply.get("on").and_then(Value::as_str).unwrap_or("");
                        self.form_added(&reply, &format!("Added choice {on} to {name}"));
                        if let Some(id) = reply.get("widgetId").and_then(Value::as_i64) {
                            self.form_open_popover(id);
                        }
                    }
                    Err(err) if crate::session::refusal_detail_of(&err) == "stale-target" => {
                        // The group went (undone or deleted): this click starts a new one.
                        self.ui.radio_draft = None;
                        self.form_radio_click(page, rect);
                    }
                    Err(err) => self.refuse_designer("add", &name, &err),
                }
            }
            Some(RadioDraft { first, .. }) => {
                let (a, mut b) = (choice(&first, 1), choice(&rect, 2));
                if a == b {
                    b = format!("{b} 2");
                }
                let label = label_near(&lines, &first);
                let mut request = json!({
                    "op": "field-add", "page": page, "kind": "radio",
                    "nameHint": label.clone().unwrap_or_default(),
                    "kids": [{"on": a, "rect": nums(&first)}, {"on": b, "rect": nums(&rect)}],
                });
                if let Some(label) = &label {
                    request["label"] = json!(label);
                }
                let id = self.form_add(request, "radio");
                let name = self
                    .fields
                    .iter()
                    .find(|f| Some(f.id) == id)
                    .map(|f| f.name.clone())
                    .unwrap_or_default();
                self.ui.radio_draft = id.map(|id| RadioDraft {
                    page,
                    first,
                    group: Some(id),
                    name,
                });
            }
            None => {
                self.ui.radio_draft = Some(RadioDraft {
                    page,
                    first: rect,
                    group: None,
                    name: String::new(),
                });
                self.form_close_popover();
                self.announce("Click where the next choice goes");
            }
        }
    }

    /// A drive click (`drawn` None) or drag with a field kind armed in the Form tool: places
    /// it as the window's press would. Returns whether the Form tool took it.
    pub(crate) fn drive_form_place(
        &mut self,
        page: usize,
        start: [f64; 2],
        drawn: Option<[f64; 4]>,
    ) -> bool {
        if self.ui.tool != "form" {
            return false;
        }
        let Some(kind) = field_kind(self.ui.variant.as_deref()) else {
            return false;
        };
        self.form_place(page, kind, start, drawn.as_ref().map(|r| &r[..]));
        true
    }

    /// Picks the field and opens its settings beside it.
    pub(crate) fn form_open_popover(&mut self, id: i64) {
        self.ui.field_pop = Some(id);
        self.on_field_select(&id.to_string());
    }

    pub(crate) fn form_close_popover(&mut self) -> bool {
        self.ui.field_pop.take().is_some()
    }

    /// The popover's view, while the Form tool has a field of the shown page picked.
    pub(crate) fn ui_field_popover_view(&self) -> Option<FieldPopover> {
        if self.ui.tool != "form" {
            return None;
        }
        let id = self.ui.field_pop?;
        let item = self.fields.iter().find(|f| f.id == id)?;
        if item.rect.len() != 4 || self.fields_page < 0 {
            return None;
        }
        let kind = match item.kind {
            FieldKind::Text => "text",
            FieldKind::Checkbox => "checkbox",
            FieldKind::Radio => "radio",
            FieldKind::Combo => "combo",
            FieldKind::List => "list",
            FieldKind::Signature => "signature",
            FieldKind::Button | FieldKind::Unknown => "button",
        };
        let mut rect = [item.rect[0], item.rect[1], item.rect[2], item.rect[3]];
        let mut buttons = Vec::new();
        if item.kind == FieldKind::Radio {
            // The popover sits beside the whole group, not over its other buttons.
            for f in self
                .fields
                .iter()
                .filter(|f| f.kind == FieldKind::Radio && f.name == item.name && f.rect.len() == 4)
            {
                rect = [
                    rect[0].min(f.rect[0]),
                    rect[1].min(f.rect[1]),
                    rect[2].max(f.rect[2]),
                    rect[3].max(f.rect[3]),
                ];
                buttons.extend(f.on.clone());
            }
        }
        Some(FieldPopover {
            id,
            page: self.fields_page as usize,
            rect,
            kind: kind.into(),
            name: item.name.clone(),
            buttons,
        })
    }

    /// The Form tool's own keys: Delete removes the picked field. Returns whether it took it.
    pub(crate) fn ui_form_key(&mut self, key: &str) -> bool {
        if self.ui.tool != "form"
            || self.any_dialog_open()
            || self.direct_active.is_some()
            || !matches!(key, "Delete" | "Backspace")
        {
            return false;
        }
        let Some(id) = self.ui.field_pop else {
            return false;
        };
        if !self.fields.iter().any(|f| f.id == id) {
            return false;
        }
        self.on_field_select(&id.to_string());
        self.delete_designed_field();
        self.ui.field_pop = None;
        true
    }

    /// What the chip says while the Form tool is on.
    pub(crate) fn form_chip(&self) -> &'static str {
        match field_kind(self.ui.variant.as_deref()) {
            Some("radio")
                if self
                    .ui
                    .radio_draft
                    .as_ref()
                    .is_some_and(|d| d.group.is_none()) =>
            {
                "Click where the next choice goes"
            }
            Some("radio") => "Click where each choice button goes",
            Some("checkbox") => "Click where the checkbox goes",
            Some("combo") => "Click or drag where the drop-down list goes",
            Some("signature") => "Click or drag where the signature goes",
            Some(_) => "Click or drag where the text field goes",
            None => "Click a field to change it, or pick a kind to add",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, x: f64, baseline: f64, w: f64) -> Line {
        Line {
            text: text.into(),
            x,
            y: baseline,
            w,
            h: 12.0,
            size: 12.0,
            ..Line::default()
        }
    }

    #[test]
    fn a_click_places_the_default_box_inside_the_page() {
        assert_eq!(
            placed_rect("text", [72.0, 100.0], (612.0, 792.0)),
            [72.0, 100.0, 252.0, 122.0]
        );
        assert_eq!(
            placed_rect("checkbox", [600.0, 790.0], (612.0, 792.0)),
            [598.0, 778.0, 612.0, 792.0]
        );
        // A drag smaller than a field means a click.
        let tiny = [72.0, 72.0, 74.0, 73.0];
        assert_eq!(
            field_rect("text", [72.0, 72.0], Some(&tiny), (612.0, 792.0)),
            [72.0, 72.0, 252.0, 94.0]
        );
        let drawn = [72.0, 72.0, 300.0, 120.0];
        assert_eq!(
            field_rect("text", [72.0, 72.0], Some(&drawn), (612.0, 792.0)),
            drawn
        );
        // A signature drawn under 40 × 20 takes its default size.
        let small = [72.0, 72.0, 100.0, 90.0];
        assert_eq!(
            field_rect("signature", [72.0, 72.0], Some(&small), (612.0, 792.0)),
            [72.0, 72.0, 252.0, 112.0]
        );
    }

    #[test]
    fn labels_come_from_the_left_then_from_above() {
        let lines = vec![
            line("Personal details", 72.0, 60.0, 120.0),
            line("Email address:", 72.0, 110.0, 80.0),
            line("________________", 160.0, 110.0, 150.0),
            line("Phone", 72.0, 140.0, 40.0),
        ];
        // Right of "Email address:" on its row.
        let field = [160.0, 96.0, 340.0, 114.0];
        assert_eq!(label_near(&lines, &field).as_deref(), Some("Email address"));
        // Under "Phone", nothing to its left.
        let below = [72.0, 146.0, 252.0, 168.0];
        assert_eq!(label_near(&lines, &below).as_deref(), Some("Phone"));
        // Far from any text: no label.
        assert_eq!(label_near(&lines, &[400.0, 500.0, 500.0, 520.0]), None);
        // Underscores alone are the blank, not a label.
        assert_eq!(label_text("_____"), None);
        assert_eq!(label_text("Name *").as_deref(), Some("Name"));
    }

    #[test]
    fn a_choice_button_is_named_by_the_words_to_its_right() {
        let lines = vec![
            line("Yes", 92.0, 112.0, 20.0),
            line("No", 92.0, 132.0, 14.0),
        ];
        assert_eq!(
            label_right(&lines, &[72.0, 100.0, 86.0, 114.0]).as_deref(),
            Some("Yes")
        );
        assert_eq!(
            label_right(&lines, &[72.0, 120.0, 86.0, 134.0]).as_deref(),
            Some("No")
        );
        assert_eq!(label_right(&lines, &[300.0, 120.0, 314.0, 134.0]), None);
    }

    #[test]
    fn only_field_kinds_place_fields() {
        assert_eq!(field_kind(Some("combo")), Some("combo"));
        assert_eq!(field_kind(Some("select")), None);
        assert_eq!(field_kind(None), None);
    }
}
