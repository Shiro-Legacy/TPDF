//! Navigation, the render loop, the toolbar's enable rules and page paint (R:2768-3120), with
//! the pure geometry the hit layer shares (`editorBox`, `viewBox`, `pdfBox`, `gestureRect`,
//! `snapRect`, `readingOrder`, `tabOrder`).
#![allow(dead_code)]

use super::Session;
use super::dom_ids as ids;
use super::types::*;
use crate::engine::Refusal;
use crate::js::{js_num, js_round};
use serde_json::json;
use tandem_protocol::engine::FieldKind;

/// R:2736 `target()`: the one wanted render (page + scale; the revision is in the key).
#[derive(Clone, Debug, PartialEq)]
pub struct RenderGoal {
    pub page: i64,
    pub zoom: f64,
    pub scale: f64,
    pub key: String,
}

/// R:3051 `editorBox()`: an element's CSS box for a y-down page rect.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EditorBox {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
    pub font_px: f64,
}

impl Session {
    // ------------------------------------------------------------------ navigation

    /// R:2770 `goTo(page)`. `page` is truncated by the caller (TS `Math.trunc`).
    pub fn go_to(&mut self, page: i64) {
        let Some(doc) = &self.doc else { return };
        if doc.page_count < 1 || doc.info.needs_password {
            return;
        }
        let next_page = page.max(0).min(doc.page_count - 1);
        if next_page != self.wanted_page {
            self.clear_direct_focus();
            self.clear_error();
        }
        self.wanted_page = next_page;
        // Leaving the page commits the open editor there (its own captured page), as click-away does.
        if self
            .direct_active
            .as_ref()
            .is_some_and(|a| a.page != self.wanted_page)
        {
            self.drop_direct();
        }
        self.sync_controls();
        self.refresh_notes();
        self.refresh_fields();
        self.refresh_links();
        let force = self.ui_mode == UiMode::Edit;
        self.refresh_content_targets(force);
        self.drain();
        if self.compare_doc.is_some() {
            self.refresh_compare(false, true);
        }
    }

    /// R:2786 `drain()`: one render at a time toward `target()`; a newer wanted state wins.
    pub fn drain(&mut self) {
        if self.in_flight.is_some() || self.doc.as_ref().is_none_or(|d| d.info.needs_password) {
            return;
        }
        let started_generation = self.generation;
        while self.doc.is_some() && started_generation == self.generation {
            let Some(goal) = self.target() else { return };
            if self.shown_key.as_deref() == Some(goal.key.as_str()) && goal.zoom == self.shown_zoom
            {
                return;
            }
            self.in_flight = Some(goal.key.clone());
            let cached = self.cache.get(&goal.key);
            let result = match cached {
                Some(result) => Ok(result),
                None => self.render_request(goal.page, goal.scale),
            };
            let result = match result {
                Ok(result) => result,
                Err(err) => {
                    // A newer document owns inFlight and the cache now.
                    if started_generation != self.generation {
                        return;
                    }
                    self.in_flight = None;
                    if err.reason_str() == "skipped" {
                        // M9: this page hung the worker at this revision; show it blank instead of asking again.
                        self.last_result = None;
                        self.shown_page = goal.page;
                        self.shown_key = Some(goal.key.clone());
                        self.shown_zoom = goal.zoom;
                        self.lane2.canvas_blank = true;
                        self.set_status(&err.message);
                        self.pump_thumbs();
                        return;
                    }
                    self.show_error(&err.message);
                    return;
                }
            };
            // Too late: the page belongs to a document that is no longer open.
            if started_generation != self.generation {
                return;
            }
            self.cache.set(&goal.key, result.clone());
            self.in_flight = None;
            let now = self.target();
            match &now {
                Some(now) if now.key == goal.key && now.zoom == goal.zoom => {}
                _ => continue, // a newer wanted state won
            }
            self.shown_page = goal.page;
            self.shown_key = Some(goal.key.clone());
            self.shown_zoom = goal.zoom;
            self.lane2.canvas_blank = false;
            self.paint(&result, goal.zoom, goal.page);
            let render_ms = result.render_ms;
            self.last_result = Some(result);
            let page_count = self.doc.as_ref().map_or(0, |d| d.page_count);
            let suffix = self.label_suffix(goal.page as usize);
            self.set_status(&format!(
                "Page {}{} of {} — rendered in {} ms at {}%",
                goal.page + 1,
                suffix,
                page_count,
                js_num(render_ms),
                js_num(js_round(goal.zoom * 100.0))
            ));
            self.pump_thumbs(); // page renders first: thumbnails only start once the wanted page is painted
        }
    }

    /// The `render` request of `drain` (pixels moved into the reply).
    fn render_request(&mut self, page: i64, scale: f64) -> Result<RenderReply, Refusal> {
        let mut reply = self.rq(json!({"op": "render", "page": page, "scale": scale}))?;
        let pixels = reply.take_bytes("pixels").unwrap_or_default();
        let mut result: RenderReply = serde_json::from_value(reply.value).unwrap_or_default();
        result.pixels = pixels;
        Ok(result)
    }

    /// R:2829 `syncControls()`: every toolbar button's enable rule and permission title.
    pub fn sync_controls(&mut self) {
        let writable = self.editable();
        // Per-permission gates from `/P`: annotate for notes/markup/links, edit for page
        // and structure work, form for field fill, copy for extract/export.
        let can_annotate = writable && self.permitted("annotate");
        let can_edit = writable && self.permitted("edit");
        let has_doc = self.doc.is_some();
        let can_copy =
            self.doc.as_ref().is_some_and(|d| !d.info.needs_password) && self.permitted("copy");
        let total = self.doc.as_ref().map_or(0, |d| d.page_count);
        let page_value = if total > 0 {
            (self.wanted_page + 1).to_string()
        } else {
            String::new()
        };
        self.dom.set_value(ids::PAGE_INPUT, page_value);
        self.dom
            .set_attr(ids::PAGE_INPUT, "max", total.max(1).to_string());
        self.dom.set_disabled(ids::PAGE_INPUT, total < 2);
        self.dom
            .set_disabled(ids::PREV_BUTTON, total < 2 || self.wanted_page <= 0);
        self.dom
            .set_disabled(ids::NEXT_BUTTON, total < 2 || self.wanted_page >= total - 1);
        // Plain Save rewrites the file the document came from; a fixture or a read-only
        // document has nowhere safe to go, Save a copy still does.
        let has_url = self.doc.as_ref().is_some_and(|d| d.url.is_some());
        let locked = self.doc.as_ref().is_some_and(|d| d.info.needs_password);
        self.dom.set_disabled(
            ids::SAVE_BUTTON,
            self.saving || !self.dirty || !has_url || !writable,
        );
        self.dom
            .set_disabled(ids::SAVE_COPY_BUTTON, self.saving || !has_url || locked);
        self.dom
            .set_disabled(ids::UNDO_BUTTON, self.saving || !self.can_undo);
        self.dom
            .set_disabled(ids::REDO_BUTTON, self.saving || !self.can_redo);
        let annotate_title = self.permission_title(can_annotate);
        self.set_tool(ids::NOTE_ADD_BUTTON, !can_annotate || !has_doc);
        self.dom
            .set_attr(ids::NOTE_ADD_BUTTON, "title", annotate_title.clone());
        self.apply_label_text();
        let hit = self.current_hit();
        for (button, _, _) in ids::MARKUP_BUTTONS {
            self.dom.set_attr(button, "title", annotate_title.clone());
        }
        self.sync_markup_buttons();
        self.set_tool(ids::FREE_TEXT_ADD_BUTTON, !can_annotate || !has_doc);
        self.dom
            .set_attr(ids::FREE_TEXT_ADD_BUTTON, "title", annotate_title.clone());
        // Shapes, stamps and the signature all place themselves off the current hit or the
        // page centre, so none of them needs a search hit to be usable.
        for (button, _, _) in ids::SHAPE_BUTTONS {
            self.dom.set_disabled(button, !can_annotate || !has_doc);
            self.dom.set_attr(button, "title", annotate_title.clone());
        }
        self.set_tool(ids::INK_DRAW_BUTTON, !can_annotate || !has_doc);
        self.dom
            .set_attr(ids::INK_DRAW_BUTTON, "title", annotate_title.clone());
        for button in [
            ids::STAMP_ICON_BUTTON,
            ids::SIGN_BUTTON,
            ids::REDACT_AREA_BUTTON,
        ] {
            self.dom.set_disabled(button, !can_annotate || !has_doc);
            self.dom.set_attr(button, "title", annotate_title.clone());
        }
        self.redact_ui_render();
        let edit_title = self.permission_title(can_edit);
        self.dom
            .set_disabled(ids::BOOKMARK_ADD_BUTTON, !can_edit || !has_doc);
        self.dom
            .set_attr(ids::BOOKMARK_ADD_BUTTON, "title", edit_title.clone());
        // A link is anchored on the current search match, so both add buttons need one.
        for button in [ids::LINK_PAGE_ADD_BUTTON, ids::LINK_URI_ADD_BUTTON] {
            self.dom
                .set_disabled(button, !can_annotate || !has_doc || hit.is_none());
            self.dom.set_attr(button, "title", annotate_title.clone());
        }
        self.dom
            .set_disabled(ids::LINK_DRAW_BUTTON, !can_annotate || !has_doc);
        let link_title = if can_annotate {
            "Drag on the page to draw a link".to_string()
        } else {
            annotate_title.clone()
        };
        self.dom
            .set_attr(ids::LINK_DRAW_BUTTON, "title", link_title);
        // P5 U3 §2.4 / review R-U3-1: the page/rect Sign path is live whenever a PDF is open.
        self.dom
            .set_disabled(ids::CERT_SIGN_OPEN_BUTTON, !has_doc || locked);
        self.sync_tool_chip();
        self.set_tool(ids::TEXT_ADD_TOOL_BUTTON, !has_doc || !can_edit);
        self.set_tool(ids::TEXTBOX_TOOL_BUTTON, !has_doc || !can_annotate);
        let can_design = self.can_design();
        self.set_tool(ids::SIGFIELD_DRAW_BUTTON, !has_doc || !can_design);
        let sigfield_title = if self.dom.disabled(ids::SIGFIELD_DRAW_BUTTON) && has_doc {
            let title = self.permission_title(false);
            if title.is_empty() {
                "This document is read-only".to_string()
            } else {
                title
            }
        } else {
            "Drag on the page to place a signature field".to_string()
        };
        self.dom
            .set_attr(ids::SIGFIELD_DRAW_BUTTON, "title", sigfield_title);
        self.dom
            .set_disabled(ids::INSERT_BUTTON, !can_edit || !has_doc);
        self.dom
            .set_attr(ids::INSERT_BUTTON, "title", edit_title.clone());
        // Extracting writes a copy and never touches the open document, so a signed or
        // certified file (read-only for edits) can still be extracted from.
        let copy_title = if can_copy {
            String::new()
        } else {
            self.permission_title(false)
        };
        for button in [ids::EXTRACT_BUTTON, ids::EXPORT_BUTTON] {
            self.dom.set_disabled(button, !can_copy);
            self.dom.set_attr(button, "title", copy_title.clone());
        }
        for button in [
            ids::WATERMARK_BUTTON,
            ids::HEADER_BUTTON,
            ids::PAGE_LABELS_BUTTON,
        ] {
            self.dom.set_disabled(button, !can_edit || !has_doc);
            self.dom.set_attr(button, "title", edit_title.clone());
        }
        let can_security = self.can_change_security();
        self.dom.set_disabled(ids::SECURITY_BUTTON, !can_security);
        let security_title = if can_security {
            String::new()
        } else {
            self.security_refusal()
        };
        self.dom
            .set_attr(ids::SECURITY_BUTTON, "title", security_title);
        let print_available = self.print_available();
        self.dom
            .set_disabled(ids::PRINT_BUTTON, self.printing || !print_available);
        let unprintable = self
            .doc
            .as_ref()
            .is_some_and(|d| !d.info.needs_password && !d.info.printable);
        let print_title = if unprintable {
            "This document does not permit printing"
        } else {
            ""
        };
        self.dom.set_attr(ids::PRINT_BUTTON, "title", print_title);
        self.dom
            .set_disabled(ids::THUMBS_TOGGLE, !has_doc || locked);
        let verify = self
            .doc
            .as_ref()
            .is_some_and(|d| d.info.signatures > 0 || d.info.certified);
        self.dom.set_hidden(ids::VERIFY_OPEN_BUTTON, !verify);
        self.sync_page_tools();
        let page = self.wanted_page;
        self.set_thumb_current(page as usize);
        self.sync_zoom();
    }

    /// `button.disabled = disabled`; a disabled toggle also loses `aria-pressed`.
    fn set_tool(&mut self, id: &str, disabled: bool) {
        self.dom.set_disabled(id, disabled);
        if disabled {
            set_pressed(&mut self.dom, id, false);
        }
    }

    /// R:2925 `permissionTitle(allowed)`: the `title` of a button a `/P` permission (or the lock) disabled.
    pub fn permission_title(&self, allowed: bool) -> String {
        if allowed {
            return String::new();
        }
        let Some(doc) = &self.doc else {
            return String::new();
        };
        let info = &doc.info;
        if info.needs_password {
            return "Enter the document password first".into();
        }
        if info.signatures > 0 || info.certified {
            return "This document is read-only".into();
        }
        super::consts::PERMISSION_TITLE.into()
    }

    /// R:2935 `canChangeSecurity()`: changing security is an owner act; an unencrypted document has no owner.
    pub fn can_change_security(&self) -> bool {
        let Some(doc) = &self.doc else { return false };
        let info = &doc.info;
        if info.needs_password || info.signatures > 0 || info.certified {
            return false;
        }
        !info.encrypted || self.auth == AuthLevel::Owner
    }

    /// R:2942 `securityRefusal()`.
    pub fn security_refusal(&self) -> String {
        match &self.doc {
            None => "Enter the document password first".into(),
            Some(doc) if doc.info.needs_password => "Enter the document password first".into(),
            Some(doc) if doc.info.signatures > 0 || doc.info.certified => {
                "This document is read-only".into()
            }
            Some(_) => "Open this document with its owner password to change its security".into(),
        }
    }

    /// R:2950 `syncPageTools()`: page tools follow the current page.
    pub fn sync_page_tools(&mut self) {
        let total = self.doc.as_ref().map_or(0, |d| d.page_count);
        let writable = self.editable() && self.permitted("edit");
        let page = self.wanted_page;
        let title = self.permission_title(writable);
        for (button, op) in ids::PAGE_TOOL_BUTTONS {
            let enabled = match op {
                "rotate-right" | "rotate-left" => writable && total > 0,
                "move-up" => writable && page > 0,
                "move-down" => writable && page < total - 1,
                _ => writable && total > 1,
            };
            self.dom.set_disabled(button, !enabled);
            self.dom.set_attr(button, "title", title.clone());
            // The accessible name carries the page the button will act on ("Delete page 3").
            let n = page + 1;
            let label = match op {
                "rotate-right" => format!("Rotate page {n} right"),
                "rotate-left" => format!("Rotate page {n} left"),
                "move-up" => format!("Move page {n} up"),
                "move-down" => format!("Move page {n} down"),
                _ => format!("Delete page {n}"),
            };
            self.dom.set_attr(button, "aria-label", label);
        }
    }

    /// R:2974 `paint()`: the canvas CSS size, the text layer, the hit layer and the Text tab.
    /// The pixels themselves are the UI's to draw (from `last_result`).
    pub fn paint(&mut self, result: &RenderReply, zoom: f64, page: i64) {
        let css_width = js_round(result.width * zoom);
        let css_height = js_round(result.height * zoom);
        self.lane2.canvas_css = [css_width, css_height];
        self.paint_text(&result.lines, zoom, css_width, css_height, page);
        if self.notes_need_text && page == self.wanted_page {
            self.notes_need_text = false;
            self.render_notes();
        }
        self.paint_hits(css_width, css_height, page);
        self.dom.set_text(ids::EXTRACTED, result.text.clone());
        self.dom
            .set_hidden(ids::TEXT_EMPTY_EL, !result.text.is_empty());
    }

    /// R:2992 `repaint()`: from the last render, e.g. when highlights or the hit index change.
    pub fn repaint(&mut self) {
        if self.shown_page < 0 {
            return;
        }
        let Some(result) = self.last_result.take() else {
            return;
        };
        let (zoom, page) = (self.shown_zoom, self.shown_page);
        self.paint(&result, zoom, page);
        if self.last_result.is_none() {
            self.last_result = Some(result);
        }
    }

    /// `canvas.clientWidth` / `clientHeight`: the canvas CSS size of the last paint.
    pub fn canvas_client(&self) -> (f64, f64) {
        (self.lane2.canvas_css[0], self.lane2.canvas_css[1])
    }
}

/// `button.setAttribute("aria-pressed", …)`; the virtual DOM also mirrors it in `checked`.
pub fn set_pressed(dom: &mut crate::dom::Dom, id: &str, on: bool) {
    dom.set_attr(id, "aria-pressed", if on { "true" } else { "false" });
    dom.set_checked(id, on);
}

/// `pressed(button)`: `aria-pressed === "true"`.
pub fn is_pressed(dom: &crate::dom::Dom, id: &str) -> bool {
    dom.attr(id, "aria-pressed").as_deref() == Some("true")
}

fn cmp(a: f64, b: f64) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}

fn is_checked_radio(item: &FieldItem) -> bool {
    item.kind == FieldKind::Radio && item.value == FieldValue::Bool(true)
}

/// The radio kid a group collapses to: its checked kid in `items`, else `item`.
fn radio_stop(item: &FieldItem, items: &[FieldItem]) -> FieldItem {
    items
        .iter()
        .find(|kid| kid.name == item.name && is_checked_radio(kid))
        .unwrap_or(item)
        .clone()
}

/// R:2996 `readingOrder()`: rows (vertical overlap ≥ 50 % of the shorter height) top→bottom,
/// left→right inside a row; a radio group once (its checked kid, else its first); signatures dropped.
pub fn reading_order(items: &[FieldItem]) -> Vec<FieldItem> {
    let mut sorted: Vec<&FieldItem> = items
        .iter()
        .filter(|i| i.kind != FieldKind::Signature)
        .collect();
    sorted.sort_by(|a, b| cmp(a.rect[1], b.rect[1]).then(cmp(a.rect[0], b.rect[0])));
    let mut rows: Vec<Vec<&FieldItem>> = Vec::new();
    for item in sorted {
        let row = rows.iter_mut().find(|group| {
            group.iter().any(|other| {
                let overlap =
                    (item.rect[3].min(other.rect[3]) - item.rect[1].max(other.rect[1])).max(0.0);
                overlap >= (item.rect[3] - item.rect[1]).min(other.rect[3] - other.rect[1]) * 0.5
            })
        });
        match row {
            Some(row) => row.push(item),
            None => rows.push(vec![item]),
        }
    }
    let top = |row: &Vec<&FieldItem>| row.iter().map(|i| i.rect[1]).fold(f64::INFINITY, f64::min);
    rows.sort_by(|a, b| cmp(top(a), top(b)));
    let mut seen = std::collections::BTreeSet::new();
    let mut ordered = Vec::new();
    for mut row in rows {
        row.sort_by(|a, b| cmp(a.rect[0], b.rect[0]));
        for item in row {
            if item.kind != FieldKind::Radio {
                ordered.push(item.clone());
            } else if seen.insert(item.name.clone()) {
                ordered.push(radio_stop(item, items));
            }
        }
    }
    ordered
}

/// R:3011 `tabsName()`: the page's `/Tabs` name as the fields reply gives it.
pub fn tabs_name(value: Option<&str>) -> Option<String> {
    value
        .filter(|v| ["R", "C", "S", "A", "W"].contains(v))
        .map(str::to_string)
}

/// R:3015 `collapseRadios()`: a radio group once, everything else as given; signatures dropped.
fn collapse_radios(list: &[FieldItem], items: &[FieldItem]) -> Vec<FieldItem> {
    let mut seen = std::collections::BTreeSet::new();
    let mut ordered = Vec::new();
    for item in list {
        if item.kind == FieldKind::Signature {
            continue;
        }
        if item.kind != FieldKind::Radio {
            ordered.push(item.clone());
        } else if seen.insert(item.name.clone()) {
            ordered.push(radio_stop(item, items));
        }
    }
    ordered
}

/// R:3030 `tabOrder()`: Tab order per the page's `/Tabs` (R/null reading order; A/W the reply's
/// order; C columns; S by `structIndex`, the rest in reading order after, or 'fallback').
pub fn tab_order(items: &[FieldItem], tabs: Option<&str>) -> (Vec<FieldItem>, &'static str) {
    match tabs {
        Some("A") => return (collapse_radios(items, items), "A"),
        Some("W") => return (collapse_radios(items, items), "W"),
        Some("C") => {
            let mut sorted: Vec<&FieldItem> = items
                .iter()
                .filter(|i| i.kind != FieldKind::Signature)
                .collect();
            sorted.sort_by(|a, b| cmp(a.rect[0], b.rect[0]).then(cmp(a.rect[1], b.rect[1])));
            let mut columns: Vec<Vec<&FieldItem>> = Vec::new();
            for item in sorted {
                let column = columns.iter_mut().find(|group| {
                    group.iter().any(|other| {
                        let overlap = (item.rect[2].min(other.rect[2])
                            - item.rect[0].max(other.rect[0]))
                        .max(0.0);
                        overlap
                            >= (item.rect[2] - item.rect[0]).min(other.rect[2] - other.rect[0])
                                * 0.5
                    })
                });
                match column {
                    Some(column) => column.push(item),
                    None => columns.push(vec![item]),
                }
            }
            let left =
                |c: &Vec<&FieldItem>| c.iter().map(|i| i.rect[0]).fold(f64::INFINITY, f64::min);
            columns.sort_by(|a, b| cmp(left(a), left(b)));
            let mut list = Vec::new();
            for mut column in columns {
                column.sort_by(|a, b| cmp(a.rect[1], b.rect[1]).then(cmp(a.rect[0], b.rect[0])));
                list.extend(column.into_iter().cloned());
            }
            return (collapse_radios(&list, items), "C");
        }
        Some("S") => {
            let mut indexed: Vec<FieldItem> = items
                .iter()
                .filter(|i| i.struct_index.is_some())
                .cloned()
                .collect();
            if indexed.is_empty() {
                return (reading_order(items), "fallback");
            }
            indexed.sort_by_key(|i| i.struct_index.unwrap_or(0));
            let rest: Vec<FieldItem> = items
                .iter()
                .filter(|i| i.struct_index.is_none())
                .cloned()
                .collect();
            indexed.extend(reading_order(&rest));
            return (collapse_radios(&indexed, items), "S");
        }
        _ => {}
    }
    (reading_order(items), "R")
}

/// R:3051 `editorBox(rect, zoom)`.
pub fn editor_box(rect: &[f64], zoom: f64) -> EditorBox {
    let at = |i: usize| rect.get(i).copied().unwrap_or(f64::NAN);
    let (x, y, x2, y2) = (at(0), at(1), at(2), at(3));
    let width = jmax(1.0, (x2 - x) * zoom);
    let height = jmax(1.0, (y2 - y) * zoom);
    EditorBox {
        left: x * zoom,
        top: y * zoom,
        width,
        height,
        font_px: jmax(10.0, jmin(400.0, height * 0.6)),
    }
}

/// R:3056 `viewBox(box, m)`: a PDF user-space box (y up) through the page transform → the y-down view rect.
pub fn view_box(bx: &[f64], m: &[f64]) -> Vec<f64> {
    let (a, b, c, d, e, f) = (m[0], m[1], m[2], m[3], m[4], m[5]);
    let mut xs = Vec::with_capacity(4);
    let mut ys = Vec::with_capacity(4);
    for (x, y) in [
        (bx[0], bx[1]),
        (bx[2], bx[1]),
        (bx[0], bx[3]),
        (bx[2], bx[3]),
    ] {
        xs.push(a * x + c * y + e);
        ys.push(b * x + d * y + f);
    }
    vec![min_of(&xs), min_of(&ys), max_of(&xs), max_of(&ys)]
}

/// R:3062 `pdfBox(rect, m)`: the exact inverse of `viewBox`, rounded to 4 decimals.
pub fn pdf_box(rect: &[f64], m: &[f64]) -> Result<Vec<f64>, Refusal> {
    let (a, b, c, d, e, f) = (m[0], m[1], m[2], m[3], m[4], m[5]);
    let det = a * d - b * c;
    if det == 0.0 || !det.is_finite() {
        return Err(Refusal::plain(
            "pdfBox: the page transform is not invertible",
        ));
    }
    let mut xs = Vec::with_capacity(4);
    let mut ys = Vec::with_capacity(4);
    for (x, y) in [
        (rect[0], rect[1]),
        (rect[2], rect[1]),
        (rect[0], rect[3]),
        (rect[2], rect[3]),
    ] {
        let (u, v) = (x - e, y - f);
        xs.push((d * u - c * v) / det);
        ys.push((a * v - b * u) / det);
    }
    Ok(vec![
        round4(min_of(&xs)),
        round4(min_of(&ys)),
        round4(max_of(&xs)),
        round4(max_of(&ys)),
    ])
}

/// `Math.round(value * 1e4) / 1e4 + 0` (the `+ 0` turns -0 into 0).
pub fn round4(value: f64) -> f64 {
    js_round(value * 1e4) / 1e4 + 0.0
}

/// `Math.round(value * 100) / 100` (R:7600 `round2`).
pub fn round2(value: f64) -> f64 {
    js_round(value * 100.0) / 100.0
}

/// `Math.min(...values)` (NaN-propagating, +∞ for none).
pub fn min_of(values: &[f64]) -> f64 {
    values.iter().fold(f64::INFINITY, |m, &v| {
        if v.is_nan() || m.is_nan() {
            f64::NAN
        } else {
            m.min(v)
        }
    })
}

/// `Math.max(...values)` (NaN-propagating, −∞ for none).
pub fn max_of(values: &[f64]) -> f64 {
    values.iter().fold(f64::NEG_INFINITY, |m, &v| {
        if v.is_nan() || m.is_nan() {
            f64::NAN
        } else {
            m.max(v)
        }
    })
}

/// JS `Math.max(a, b)` / `Math.min(a, b)`.
pub fn jmax(a: f64, b: f64) -> f64 {
    max_of(&[a, b])
}
pub fn jmin(a: f64, b: f64) -> f64 {
    min_of(&[a, b])
}

/// R:3074 `gestureRect()`'s options.
#[derive(Clone, Debug, PartialEq)]
pub struct GestureOptions {
    pub min: f64,
    pub lock_aspect: bool,
    pub page: [f64; 2],
    pub within: Option<Vec<f64>>,
}

/// R:3074 `gestureRect()`: the rect a grip/side/corner drag of (dx, dy) page pt makes from `initial`.
/// Grip moves and stays on the page; sides stretch; corners with `lockAspect` scale about the
/// opposite corner by s = max of the two side ratios, clamped so each side stays ≥ min and the
/// corner stays on the page.
pub fn gesture_rect(
    initial: &[f64],
    handle: &str,
    dx: f64,
    dy: f64,
    options: &GestureOptions,
) -> Vec<f64> {
    let (x0, y0, x1, y1) = (initial[0], initial[1], initial[2], initial[3]);
    let [pw, ph] = options.page;
    let min = options.min;
    let (w0, h0) = (x1 - x0, y1 - y0);
    // P6 U6 §2.3/§9.18: `within` (crop mode: the uncropped box ∩ the page) replaces the page clamp.
    let (lx, ly, rx, ry) = match &options.within {
        Some(w) => (w[0], w[1], w[2], w[3]),
        None => (0.0, 0.0, pw, ph),
    };
    if handle == "grip" {
        let x = jmax(lx, jmin(rx - w0, x0 + dx));
        let y = jmax(ly, jmin(ry - h0, y0 + dy));
        return vec![x, y, x + w0, y + h0];
    }
    let (west, east, north, south) = (
        handle.contains('w'),
        handle.contains('e'),
        handle.contains('n'),
        handle.contains('s'),
    );
    if options.lock_aspect
        && options.within.is_none()
        && (west || east)
        && (north || south)
        && w0 > 0.0
        && h0 > 0.0
    {
        let sx = (w0 + if east { dx } else { -dx }) / w0;
        let sy = (h0 + if south { dy } else { -dy }) / h0;
        let most = jmin(
            if east { (pw - x0) / w0 } else { x1 / w0 },
            if south { (ph - y0) / h0 } else { y1 / h0 },
        );
        let s = jmin(most, max_of(&[min / w0, min / h0, sx, sy]));
        let (w, h) = (w0 * s, h0 * s);
        return vec![
            if east { x0 } else { x1 - w },
            if south { y0 } else { y1 - h },
            if east { x0 + w } else { x1 },
            if south { y0 + h } else { y1 },
        ];
    }
    let mut rect = initial.to_vec();
    if west {
        rect[0] = jmax(lx, jmin(x1 - min, x0 + dx));
    }
    if east {
        rect[2] = jmin(rx, jmax(x0 + min, x1 + dx));
    }
    if north {
        rect[1] = jmax(ly, jmin(y1 - min, y0 + dy));
    }
    if south {
        rect[3] = jmin(ry, jmax(y0 + min, y1 + dy));
    }
    rect
}

/// R:3104 `snapRect()`: alignment guides (view space, y down). Per axis the listed moving edges
/// meet every candidate (each other rect's edges and centre, then the page's edges and centre);
/// the smallest |delta| ≤ tolerance wins, a tie keeps the first. At most one translation per axis.
pub fn snap_rect(
    rect: &[f64],
    others: &[Vec<f64>],
    page: [f64; 2],
    tolerance: f64,
    edges: &[SnapEdge],
) -> (Vec<f64>, Vec<SnapGuide>) {
    let mut out = rect.to_vec();
    let mut guides = Vec::new();
    for (axis, lo, extent) in [(Axis::X, 0usize, page[0]), (Axis::Y, 1usize, page[1])] {
        let (e0, e1, ec) = match axis {
            Axis::X => (SnapEdge::X0, SnapEdge::X1, SnapEdge::Cx),
            Axis::Y => (SnapEdge::Y0, SnapEdge::Y1, SnapEdge::Cy),
        };
        let values = [
            (e0, rect[lo]),
            (e1, rect[lo + 2]),
            (ec, (rect[lo] + rect[lo + 2]) / 2.0),
        ];
        let mut candidates: Vec<f64> = others
            .iter()
            .flat_map(|o| [o[lo], o[lo + 2], (o[lo] + o[lo + 2]) / 2.0])
            .collect();
        candidates.extend([0.0, extent, extent / 2.0]);
        let mut best: Option<(f64, f64)> = None;
        for (edge, value) in values {
            if !edges.contains(&edge) {
                continue;
            }
            for &at in &candidates {
                let delta = at - value;
                if delta.abs() <= tolerance && best.is_none_or(|(d, _)| delta.abs() < d.abs()) {
                    best = Some((delta, at));
                }
            }
        }
        if let Some((delta, at)) = best {
            out[lo] += delta;
            out[lo + 2] += delta;
            guides.push(SnapGuide { axis, at });
        }
    }
    (out, guides)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(id: i64, name: &str, kind: FieldKind, rect: [f64; 4]) -> FieldItem {
        FieldItem {
            id,
            name: name.into(),
            kind,
            rect: rect.to_vec(),
            ..FieldItem::default()
        }
    }

    #[test]
    fn reading_order_rows_then_columns() {
        let items = vec![
            field(1, "b", FieldKind::Text, [200.0, 100.0, 300.0, 120.0]),
            field(2, "a", FieldKind::Text, [10.0, 105.0, 100.0, 125.0]),
            field(3, "c", FieldKind::Text, [10.0, 200.0, 100.0, 220.0]),
            field(4, "s", FieldKind::Signature, [10.0, 0.0, 100.0, 20.0]),
        ];
        let names: Vec<_> = reading_order(&items).into_iter().map(|i| i.name).collect();
        assert_eq!(names, ["a", "b", "c"]);
        let (order, used) = tab_order(&items, Some("C"));
        assert_eq!(used, "C");
        assert_eq!(order.iter().map(|i| i.id).collect::<Vec<_>>(), [2, 3, 1]);
        assert_eq!(tab_order(&items, Some("S")).1, "fallback");
    }

    #[test]
    fn radio_groups_collapse_to_the_checked_kid() {
        let mut on = field(2, "r", FieldKind::Radio, [100.0, 10.0, 110.0, 20.0]);
        on.value = FieldValue::Bool(true);
        let items = vec![
            field(1, "r", FieldKind::Radio, [10.0, 10.0, 20.0, 20.0]),
            on,
        ];
        let order = reading_order(&items);
        assert_eq!(order.len(), 1);
        assert_eq!(order[0].id, 2);
    }

    #[test]
    fn boxes_round_trip() {
        let m = [1.0, 0.0, 0.0, -1.0, 0.0, 792.0];
        let view = view_box(&[72.0, 700.0, 172.0, 720.0], &m);
        assert_eq!(view, vec![72.0, 72.0, 172.0, 92.0]);
        assert_eq!(pdf_box(&view, &m).unwrap(), vec![72.0, 700.0, 172.0, 720.0]);
        assert!(pdf_box(&view, &[0.0; 6]).is_err());
        let b = editor_box(&[10.0, 20.0, 10.0, 60.0], 2.0);
        assert_eq!(
            (b.left, b.top, b.width, b.height, b.font_px),
            (20.0, 40.0, 1.0, 80.0, 48.0)
        );
    }

    #[test]
    fn gesture_rect_rules() {
        let options = GestureOptions {
            min: 8.0,
            lock_aspect: false,
            page: [600.0, 800.0],
            within: None,
        };
        assert_eq!(
            gesture_rect(&[10.0, 10.0, 110.0, 60.0], "grip", -50.0, 5.0, &options),
            vec![0.0, 15.0, 100.0, 65.0]
        );
        assert_eq!(
            gesture_rect(&[10.0, 10.0, 110.0, 60.0], "e", -200.0, 0.0, &options),
            vec![10.0, 10.0, 18.0, 60.0]
        );
        let locked = GestureOptions {
            lock_aspect: true,
            ..options.clone()
        };
        assert_eq!(
            gesture_rect(&[0.0, 0.0, 100.0, 50.0], "se", 100.0, 0.0, &locked),
            vec![0.0, 0.0, 200.0, 100.0]
        );
        let crop = GestureOptions {
            within: Some(vec![5.0, 5.0, 50.0, 50.0]),
            ..options
        };
        assert_eq!(
            gesture_rect(&[10.0, 10.0, 20.0, 20.0], "grip", -20.0, 100.0, &crop),
            vec![5.0, 40.0, 15.0, 50.0]
        );
    }

    #[test]
    fn snap_keeps_the_nearest_and_first() {
        let (rect, guides) = snap_rect(
            &[98.0, 10.0, 148.0, 30.0],
            &[vec![100.0, 200.0, 150.0, 300.0]],
            [600.0, 800.0],
            4.0,
            &[
                SnapEdge::X0,
                SnapEdge::X1,
                SnapEdge::Cx,
                SnapEdge::Y0,
                SnapEdge::Y1,
                SnapEdge::Cy,
            ],
        );
        assert_eq!(rect, vec![100.0, 10.0, 150.0, 30.0]);
        assert_eq!(
            guides,
            vec![SnapGuide {
                axis: Axis::X,
                at: 100.0
            }]
        );
        let (rect, guides) = snap_rect(
            &[3.0, 400.0, 13.0, 410.0],
            &[],
            [600.0, 800.0],
            4.0,
            &[SnapEdge::X0],
        );
        assert_eq!(rect[0], 0.0);
        assert_eq!(guides.len(), 1);
    }
}

/// JavaScript's `Number#toFixed(digits)` for |x| < 1e21: an exact tie rounds away from zero
/// (Rust's formatting rounds an exact tie to even).
pub fn js_to_fixed(x: f64, digits: usize) -> String {
    if !x.is_finite() || x.abs() >= 1e21 {
        return js_num(x);
    }
    let exact = format!("{:.80}", x.abs());
    let (int, frac) = exact.split_once('.').unwrap_or((&exact, ""));
    let tie = frac.len() > digits
        && frac.as_bytes()[digits] == b'5'
        && frac[digits + 1..].bytes().all(|b| b == b'0');
    let text = if tie {
        // Round up the kept digits by one unit in the last place.
        let kept = format!("{int}{}", &frac[..digits]);
        let mut digits_vec: Vec<u8> = kept.bytes().collect();
        let mut i = digits_vec.len();
        loop {
            if i == 0 {
                digits_vec.insert(0, b'1');
                break;
            }
            i -= 1;
            if digits_vec[i] == b'9' {
                digits_vec[i] = b'0';
            } else {
                digits_vec[i] += 1;
                break;
            }
        }
        let s = String::from_utf8(digits_vec).unwrap_or_default();
        if digits == 0 {
            s
        } else {
            format!("{}.{}", &s[..s.len() - digits], &s[s.len() - digits..])
        }
    } else {
        format!("{:.*}", digits, x.abs())
    };
    if x < 0.0 { format!("-{text}") } else { text }
}

#[cfg(test)]
mod fixed_tests {
    use super::js_to_fixed;

    #[test]
    fn to_fixed_matches_js() {
        assert_eq!(js_to_fixed(0.25, 1), "0.3");
        assert_eq!(js_to_fixed(1.005, 2), "1.00");
        assert_eq!(js_to_fixed(72.0, 1), "72.0");
        assert_eq!(js_to_fixed(9.95, 1), "9.9");
        assert_eq!(js_to_fixed(99.95, 1), "100.0");
        assert_eq!(js_to_fixed(-0.25, 1), "-0.3");
        assert_eq!(js_to_fixed(0.5, 0), "1");
    }
}
