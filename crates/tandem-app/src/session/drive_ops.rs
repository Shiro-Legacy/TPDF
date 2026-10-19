//! Lane 1: renderer.ts 1734–2350, "A2 drive ops": the drive's own steps and
//! `runEditRequest`, the central op dispatcher (spec-drive-state §3.0).
//!
//! The gesture steps (`click`, `menu`, `drag`/`handle`, `key`, `type`) validate, go to the
//! page and draw it here; what the Electron drive then did with synthetic DOM events (hit-test
//! the topmost element at the point and dispatch pointer, mouse, keyboard or clipboard events
//! to it) is the hit layer's: it is called through the `drive_hit_*` / `*_keydown` seams.
#![allow(dead_code)]

use super::Session;
use super::bookmarks_links::{LinkChange, LinkTarget};
use super::consts::{DIRECT_MODES, DRAW_LINK_PROMPT, DRAW_SIGFIELD_PROMPT, PAGE_OPS, ZOOM_STEPS};
use super::dom_ids as ids;
use super::shell::{js_integer, js_slice16, js_string, js_string_or_empty, parse_page_range};
use super::types::{
    AnnotLast, DecorateLast, DirectItem, DirectKind, EditorTag, FieldItem, FieldValue, LastAction,
    LinkMode, MarkupLast, MarkupSource, NavLast, UiMode, ZoomMode,
};
use crate::js::js_num;
use serde_json::{Map, Value};
use tandem_protocol::engine::FieldKind;
use tandem_protocol::limits::{MAX_BOOKMARK_CHARS, MAX_NOTE_CHARS};

/// The `#labels-style` options (index.html).
const LABELS_STYLE_OPTIONS: [&str; 6] = ["D", "r", "R", "a", "A", ""];

/// R:2037 `MENU_ITEMS`: drive item names → object-menu keys.
const MENU_ITEMS: [(&str, &str); 12] = [
    ("cut", "cut"),
    ("copy", "copy"),
    ("paste", "paste"),
    ("replace", "replace"),
    ("duplicate", "duplicate"),
    ("rotate-cw", "rotateCw"),
    ("rotate-ccw", "rotateCcw"),
    ("flip-h", "flipH"),
    ("flip-v", "flipV"),
    ("crop", "crop"),
    ("delete", "delete"),
    ("redact-page", "redactPage"),
];

/// R:2246 `driveArm`'s tool buttons.
const ARM_TOOLS: [(&str, &str); 9] = [
    ("note", ids::NOTE_ADD_BUTTON),
    ("freetext", ids::FREE_TEXT_ADD_BUTTON),
    ("ink", ids::INK_DRAW_BUTTON),
    ("redact", ids::REDACT_AREA_BUTTON),
    ("text", ids::CONTENT_ADD_ARM_BUTTON),
    ("link", ids::LINK_DRAW_BUTTON),
    ("sigfield", ids::SIGFIELD_DRAW_BUTTON),
    ("add-text", ids::TEXT_ADD_TOOL_BUTTON),
    ("textbox", ids::TEXTBOX_TOOL_BUTTON),
];

fn lookup<'a>(table: &[(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn number(map: &Map<String, Value>, key: &str) -> Option<f64> {
    map.get(key).and_then(Value::as_f64)
}

fn flag(map: &Map<String, Value>, key: &str) -> bool {
    map.get(key) == Some(&Value::Bool(true))
}

/// `typeof x === "number" || typeof x === "string" ? String(x) : ""`.
fn number_or_string(value: Option<&Value>) -> String {
    match value {
        Some(v @ (Value::Number(_) | Value::String(_))) => js_string(v),
        _ => String::new(),
    }
}

/// A drive key with its `Alt+` / `Meta+` / `Shift+` prefixes (stripped once each, in that order).
fn parse_key(raw: &str) -> (String, bool, bool, bool) {
    let mut key = raw.to_string();
    let (mut alt, mut meta, mut shift) = (false, false, false);
    for (prefix, which) in [("Alt+", 0), ("Meta+", 1), ("Shift+", 2)] {
        if let Some(rest) = key.strip_prefix(prefix) {
            match which {
                0 => alt = true,
                1 => meta = true,
                _ => shift = true,
            }
            key = rest.to_string();
        }
    }
    (key, alt, meta, shift)
}

/// UTF-16 offset → byte offset in `text` (clamped).
fn byte_at(text: &str, units: usize) -> usize {
    let mut count = 0;
    for (index, c) in text.char_indices() {
        if count >= units {
            return index;
        }
        count += c.len_utf16();
    }
    text.len()
}

impl Session {
    fn page_count(&self) -> i64 {
        self.doc.as_ref().map_or(0, |doc| doc.page_count)
    }

    fn unlocked_doc(&self) -> bool {
        self.doc
            .as_ref()
            .is_some_and(|doc| !doc.info.needs_password)
    }

    /// R:1737 `goto {page}`: 0-based, refused before `goTo` (which clamps silently).
    pub(crate) fn drive_goto(&mut self, contents: Option<&str>) {
        self.clear_error();
        let Some(wanted) = self.drive_wanted(contents) else {
            return;
        };
        if !self.unlocked_doc() {
            return;
        }
        let count = self.page_count();
        let page = js_integer(wanted.get("page")).filter(|p| *p >= 0 && *p < count);
        let Some(page) = page else {
            self.show_error(&format!(
                "goto: page must be an integer between 0 and {}",
                count - 1
            ));
            return;
        };
        self.go_to(page);
        self.refresh_notes();
        self.refresh_fields();
        self.refresh_links();
        if self.ui_mode == UiMode::Edit {
            self.refresh_content_targets(true);
        }
    }

    /// R:1752 `zoom {mode}` or `{scale}`; a scale must be one of `ZOOM_STEPS`.
    pub(crate) fn drive_zoom(&mut self, contents: Option<&str>) {
        self.clear_error();
        let Some(wanted) = self.drive_wanted(contents) else {
            return;
        };
        if !self.unlocked_doc() {
            return;
        }
        if let Some(mode) = wanted.get("mode").and_then(Value::as_str) {
            let mode = match mode {
                "fit-width" => ZoomMode::FitWidth,
                "fit-page" => ZoomMode::FitPage,
                _ => {
                    self.show_error("zoom: mode must be fit-width or fit-page");
                    return;
                }
            };
            let value = self.fixed_zoom;
            self.set_zoom(mode, value);
            self.drain();
            return;
        }
        let scale = number(&wanted, "scale").filter(|s| ZOOM_STEPS.contains(s));
        let Some(scale) = scale else {
            let steps: Vec<String> = ZOOM_STEPS.iter().map(|s| js_num(*s)).collect();
            self.show_error(&format!("zoom: scale must be one of {}", steps.join(", ")));
            return;
        };
        self.set_zoom(ZoomMode::Fixed, scale);
        self.drain();
    }

    /// R:1775 `find {query}` or `{dir: next|prev}`: the toolbar search handlers.
    pub(crate) fn drive_find(&mut self, contents: Option<&str>) {
        self.clear_error();
        let Some(wanted) = self.drive_wanted(contents) else {
            return;
        };
        if !self.unlocked_doc() {
            return;
        }
        if let Some(query) = wanted.get("query").and_then(Value::as_str) {
            self.dom.set_value(ids::SEARCH_INPUT, query);
            self.start_search();
            return;
        }
        match wanted.get("dir").and_then(Value::as_str) {
            Some("next") => self.move_hit(1),
            Some("prev") => self.move_hit(-1),
            _ => self.show_error("find: send a query, or a dir of next/prev"),
        }
    }

    /// R:1790 `tab {id}`: the inspector tablist's own click handler; unknown ids refused.
    pub(crate) fn drive_tab(&mut self, contents: Option<&str>) {
        self.clear_error();
        let Some(wanted) = self.drive_wanted(contents) else {
            return;
        };
        let id = wanted.get("id").and_then(Value::as_str);
        let index = id.and_then(|id| ids::INSPECTOR_TABS.iter().position(|tab| *tab == id));
        let Some(index) = index else {
            self.show_error(&format!("tab: no inspector tab named {}", id.unwrap_or("")));
            return;
        };
        self.on_tab_click(ids::INSPECTOR_TABS[index]);
    }

    /// R:1804 `annot {action: set|delete, index}`: the Comments row's own edit/delete calls.
    pub(crate) fn drive_annot(&mut self, contents: Option<&str>) {
        self.clear_error();
        let Some(wanted) = self.drive_wanted(contents) else {
            return;
        };
        if !self.unlocked_doc() {
            return;
        }
        let action = match wanted.get("action").and_then(Value::as_str) {
            Some("set") => Some("set"),
            Some("delete") => Some("delete"),
            _ => None,
        };
        let at = js_integer(wanted.get("index")).unwrap_or(-1);
        let Some(action) = action else {
            self.annot_last = Some(AnnotLast {
                action: js_string_or_empty(&wanted, "action"),
                index: at,
                detail: Some("invalid-action".into()),
            });
            return;
        };
        // §8.5: the list is refreshed before the index is resolved.
        self.refresh_notes();
        let item = usize::try_from(at)
            .ok()
            .and_then(|i| self.notes.get(i))
            .cloned();
        let Some(item) = item else {
            self.annot_last = Some(AnnotLast {
                action: action.into(),
                index: at,
                detail: Some("not-found".into()),
            });
            return;
        };
        if !item.editable && !(action == "delete" && item.r#type == "Redact") {
            self.annot_last = Some(AnnotLast {
                action: action.into(),
                index: at,
                detail: Some("not-editable".into()),
            });
            return;
        }
        if action == "set" {
            let text = js_slice16(&js_string_or_empty(&wanted, "contents"), MAX_NOTE_CHARS);
            self.edit_note(item.id, &text);
        } else {
            let announcement = if item.r#type == "Redact" {
                "Deleted redaction mark"
            } else {
                "Deleted note"
            };
            self.delete_note(item.id, announcement);
        }
        self.annot_last = Some(AnnotLast {
            action: action.into(),
            index: at,
            detail: None,
        });
        self.refresh_notes();
    }

    /// R:1833 `underline` / `strikeout`: markup on the current search match (a text
    /// selection comes first), no implicit query.
    pub(crate) fn drive_markup(&mut self, kind: &str) {
        self.clear_error();
        if self.selection_markup().is_some() {
            self.mark_selection_or_match(kind);
            return;
        }
        let Some(hit) = self.current_hit() else {
            self.annot_last = Some(AnnotLast {
                action: kind.to_lowercase(),
                index: -1,
                detail: Some("no-anchor".into()),
            });
            self.announce("Find something first");
            return;
        };
        let announcement = if kind == "Underline" {
            "Underlined match"
        } else {
            "Struck out match"
        };
        if self.add_markup(hit.page, kind, &hit.quads, announcement) {
            self.markup_last = Some(MarkupLast {
                kind: kind.into(),
                page: hit.page,
                source: MarkupSource::Search,
                quads: hit
                    .quads
                    .iter()
                    .map(|quad| {
                        quad.iter()
                            .map(|v| crate::js::js_round(v * 100.0) / 100.0)
                            .collect()
                    })
                    .collect(),
                text: None,
            });
        }
    }

    /// R:1846 `insert {after, range}` with the line's `from`: the insert dialog's own save path.
    pub(crate) fn drive_insert(&mut self, contents: Option<&str>) {
        self.clear_error();
        let Some(wanted) = self.drive_wanted(contents) else {
            return;
        };
        if self.doc.is_none() || !self.can_edit() {
            return;
        }
        let Some(picked) = self.pick_insert_source("insert") else {
            let mut summary = self.assemble_summary.clone().unwrap_or_default();
            summary.last = Some(LastAction {
                action: "insert".into(),
                ok: false,
                detail: Some("unreadable".into()),
            });
            self.assemble_summary = Some(summary);
            return;
        };
        self.open_insert_dialog(picked);
        self.dom.set_value(
            ids::INSERT_AFTER_INPUT,
            number_or_string(wanted.get("after")),
        );
        let range = wanted
            .get("range")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        self.dom.set_value(ids::INSERT_RANGE_INPUT, range);
        self.save_insert_dialog();
        self.close_dialog(ids::INSERT_DIALOG);
    }

    /// R:1864 `watermark` / `header-footer` with a payload: the stamp dialog's own save path.
    pub(crate) fn drive_stamp(&mut self, contents: Option<&str>) {
        self.clear_error();
        let Some(wanted) = self.drive_wanted(contents) else {
            return;
        };
        if self.doc.is_none() || !self.can_edit() {
            return;
        }
        let kind = match wanted.get("kind").and_then(Value::as_str) {
            Some(kind @ ("watermark" | "header" | "footer")) => kind.to_string(),
            _ => {
                self.decorate_last(DecorateLast {
                    action: "stamp".into(),
                    ok: false,
                    detail: Some("invalid-kind".into()),
                    ..DecorateLast::default()
                });
                return;
            }
        };
        self.open_stamp_dialog(Some(&kind));
        if let Some(text) = wanted.get("text").and_then(Value::as_str) {
            self.dom.set_value(ids::STAMP_TEXT_INPUT, text);
        }
        let pages_text = wanted
            .get("pages")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        self.dom.set_value(ids::STAMP_PAGES_INPUT, pages_text);
        // §9.11: present keys go into the fields verbatim, so the dialog's check is the only check.
        if let Some(start) = wanted.get("batesStart") {
            self.dom
                .set_value(ids::STAMP_BATES_START_INPUT, js_string(start));
        }
        if let Some(pad) = wanted.get("batesPad") {
            self.dom
                .set_value(ids::STAMP_BATES_PAD_INPUT, js_string(pad));
        }
        let pages = parse_page_range(&self.dom.value(ids::STAMP_PAGES_INPUT), self.page_count());
        self.stamp_bates = None;
        let mut ok = false;
        let align = wanted.get("align");
        let align_ok = matches!(align.and_then(Value::as_str), Some("left" | "right"));
        if align.is_some() && !align_ok {
            self.show_error("Align must be left or right");
        } else {
            if let Some(align) = align.and_then(Value::as_str) {
                self.dom.set_value(ids::STAMP_ALIGN_SELECT, align);
            }
            ok = self.save_stamp_dialog();
        }
        self.close_dialog(ids::STAMP_DIALOG);
        let bates = if ok { self.stamp_bates } else { None };
        self.decorate_last(DecorateLast {
            action: "stamp".into(),
            kind: Some(kind),
            pages,
            ok,
            detail: None,
            bates,
        });
    }

    /// R:1893 `labels {from, style, prefix, start}` or `{clear, from}`: the dialog's own save.
    pub(crate) fn drive_labels(&mut self, contents: Option<&str>) {
        self.clear_error();
        let Some(wanted) = self.drive_wanted(contents) else {
            return;
        };
        if self.doc.is_none() || !self.can_edit() {
            return;
        }
        self.open_labels_dialog();
        if matches!(
            wanted.get("from"),
            Some(Value::Number(_) | Value::String(_))
        ) {
            self.dom
                .set_value(ids::LABELS_FROM_INPUT, number_or_string(wanted.get("from")));
        }
        if flag(&wanted, "clear") {
            let ok = self.clear_labels();
            self.close_dialog(ids::LABELS_DIALOG);
            self.refresh_labels();
            self.decorate_last(DecorateLast {
                action: "labels-clear".into(),
                ok,
                ..DecorateLast::default()
            });
            return;
        }
        let style = wanted
            .get("style")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if !LABELS_STYLE_OPTIONS.contains(&style.as_str()) {
            // §8.9: checked against the select before anything is written.
            self.close_dialog(ids::LABELS_DIALOG);
            self.decorate_last(DecorateLast {
                action: "labels-set".into(),
                ok: false,
                detail: Some("invalid-style".into()),
                ..DecorateLast::default()
            });
            return;
        }
        self.dom.set_value(ids::LABELS_STYLE_SELECT, style);
        if let Some(prefix) = wanted.get("prefix").and_then(Value::as_str) {
            self.dom.set_value(ids::LABELS_PREFIX_INPUT, prefix);
        }
        if let Some(start) = wanted.get("start") {
            self.dom
                .set_value(ids::LABELS_START_INPUT, js_string(start));
        }
        let ok = self.save_labels_dialog();
        self.close_dialog(ids::LABELS_DIALOG);
        self.refresh_labels();
        self.decorate_last(DecorateLast {
            action: "labels-set".into(),
            ok,
            ..DecorateLast::default()
        });
    }

    /// R:1923 `link {kind, page|uri}` / `{action: delete|set, index}`: a drawn rect or a find
    /// hit is the anchor.
    pub(crate) fn drive_link(&mut self, contents: Option<&str>) {
        self.clear_error();
        let Some(wanted) = self.drive_wanted(contents) else {
            return;
        };
        if self.doc.is_none() || !self.can_annotate() {
            return;
        }
        let at = js_integer(wanted.get("index")).unwrap_or(-1);
        let action = wanted.get("action").and_then(Value::as_str);
        if action == Some("delete") {
            self.refresh_links();
            let item = usize::try_from(at)
                .ok()
                .and_then(|i| self.links.get(i))
                .cloned();
            let Some(item) = item else {
                self.nav_last = Some(NavLast {
                    action: Some("link-delete".into()),
                    index: Some(at),
                    detail: Some("no-such-link".into()),
                    ..NavLast::default()
                });
                return;
            };
            self.delete_link(item.id);
            self.refresh_links();
            self.nav_last = Some(NavLast {
                action: Some("link-delete".into()),
                index: Some(at),
                ..NavLast::default()
            });
            return;
        }
        if action == Some("set") {
            // P6 U4 §9.10: by the row id; the request goes by that row's annotId; page is 1-based.
            self.refresh_links();
            let item = self.links.iter().find(|entry| entry.id == at).cloned();
            let Some(item) = item else {
                self.nav_last = Some(NavLast {
                    action: Some("link-set".into()),
                    index: Some(at),
                    detail: Some("no-such-link".into()),
                    ..NavLast::default()
                });
                self.show_error(&format!("link: no link {at} on this page"));
                return;
            };
            // `change.rect = wanted.rect` (unknown in the TS): a rect that is not a number list
            // cannot be carried by `LinkChange` and is left out.
            let mut change = LinkChange {
                rect: wanted
                    .get("rect")
                    .and_then(|rect| serde_json::from_value(rect.clone()).ok()),
                target: None,
            };
            if let Some(page) = number(&wanted, "page") {
                change.target = Some(LinkTarget::Page((page - 1.0) as i64));
            } else if let Some(uri) = wanted.get("uri").and_then(Value::as_str) {
                change.target = Some(LinkTarget::Uri(uri.to_string()));
            }
            let page = self.wanted_page;
            self.link_set(page, &item, change, "Changed link", false);
            return;
        }
        let kind = wanted
            .get("kind")
            .and_then(Value::as_str)
            .and_then(LinkMode::parse);
        let target_text = |kind: LinkMode| {
            if kind == LinkMode::Page {
                js_string_or_empty(&wanted, "page")
            } else {
                js_string_or_empty(&wanted, "uri")
            }
        };
        // P6 U4 §1.6: a drawn rect (its dialog still open) is the anchor; otherwise the find hit.
        if self.nav_link_draft.is_some() && self.dom.is_open(ids::LINK_DIALOG) {
            let Some(kind) = kind else {
                self.nav_last = Some(NavLast {
                    action: Some("link-add".into()),
                    detail: Some("invalid-kind".into()),
                    ..NavLast::default()
                });
                return;
            };
            self.set_link_mode(kind, false);
            self.dom
                .set_value(ids::LINK_TARGET_INPUT, target_text(kind));
            let ok = self.save_link_dialog();
            self.close_link_dialog();
            self.nav_last = Some(NavLast {
                action: Some("link-add".into()),
                detail: (!ok).then(|| "invalid".to_string()),
                ..NavLast::default()
            });
            return;
        }
        let hit = self.current_hit();
        let rect = hit.as_ref().and_then(super::bookmarks_links::hit_rect);
        if hit.is_none() || rect.is_none() {
            self.nav_last = Some(NavLast {
                action: Some("link-add".into()),
                detail: Some("no-anchor".into()),
                ..NavLast::default()
            });
            self.announce_nav("Find something first");
            return;
        }
        let Some(kind) = kind else {
            self.nav_last = Some(NavLast {
                action: Some("link-add".into()),
                detail: Some("invalid-kind".into()),
                ..NavLast::default()
            });
            return;
        };
        self.open_link_dialog(kind);
        self.dom
            .set_value(ids::LINK_TARGET_INPUT, target_text(kind));
        let ok = self.save_link_dialog();
        self.close_link_dialog();
        self.nav_last = Some(NavLast {
            action: Some("link-add".into()),
            detail: (!ok).then(|| "invalid".to_string()),
            ..NavLast::default()
        });
    }

    /// The common start of `click` and `menu` (R:1989-1993, R:2032-2034): `{page, x, y}`
    /// required, the page an integer in range. Returns (page, x, y).
    fn drive_point(&mut self, op: &str, wanted: &Map<String, Value>) -> Option<(i64, f64, f64)> {
        let (Some(page), Some(x), Some(y)) = (
            number(wanted, "page"),
            number(wanted, "x"),
            number(wanted, "y"),
        ) else {
            self.show_error(&format!("{op}: page, x and y are required"));
            return None;
        };
        let count = self.page_count();
        if page != page.trunc() || page < 0.0 || page >= count as f64 {
            self.show_error(&format!(
                "{op}: page must be an integer between 0 and {}",
                count - 1
            ));
            return None;
        }
        Some((page as i64, x, y))
    }

    /// The page drawn and sized, as the drive waited for it (R:1994-1996).
    fn drive_page_ready(&mut self, page: i64) -> bool {
        self.go_to(page);
        self.drain();
        let shown = self.shown_page;
        self.doc
            .as_ref()
            .and_then(|doc| usize::try_from(shown).ok().and_then(|p| doc.pages.get(p)))
            .is_some_and(|size| size.height >= 1.0)
    }

    /// R:1988 `click {page, x, y, alt?, shift?}` in page points (y down): the press, release
    /// and click on whatever is topmost at the point (`drive_hit_click`).
    pub(crate) fn drive_direct_click(&mut self, contents: Option<&str>) {
        let Some(wanted) = self.drive_wanted(contents) else {
            return;
        };
        if self.doc.is_none() {
            return;
        }
        let Some((page, x, y)) = self.drive_point("click", &wanted) else {
            return;
        };
        if !self.drive_page_ready(page) {
            return;
        }
        self.drive_hit_click(
            page as usize,
            x,
            y,
            flag(&wanted, "alt"),
            flag(&wanted, "shift"),
        );
    }

    /// R:2030 `menu {page, x, y, item?}`: a right-click on the target under the point, then
    /// optionally one item.
    pub(crate) fn drive_object_menu(&mut self, contents: Option<&str>) {
        let Some(wanted) = self.drive_wanted(contents) else {
            return;
        };
        if self.doc.is_none() {
            return;
        }
        let Some((page, x, y)) = self.drive_point("menu", &wanted) else {
            return;
        };
        let item = match wanted.get("item") {
            None => None,
            Some(item) => match item
                .as_str()
                .and_then(|name| lookup(&MENU_ITEMS, name).map(|key| (name, key)))
            {
                Some(found) => Some(found),
                None => {
                    self.show_error("menu: item must be cut, copy, paste, replace, duplicate, rotate-cw, rotate-ccw, flip-h, flip-v, crop, delete or redact-page");
                    return;
                }
            },
        };
        self.close_object_menu(false); // as the outside press of a real right-click would
        if !self.drive_page_ready(page) {
            return;
        }
        // P6 U7 §9.28: empty page space opens the page menu; the drive hands it the exact page point.
        self.drive_menu_point = Some(vec![x, y]);
        self.drive_context_menu(page as usize, x, y);
        self.drive_menu_point = None;
        let Some((name, key)) = item else { return };
        let button = lookup(&ids::OBJECT_MENU_BUTTONS, key).unwrap_or("");
        if self.dom.hidden(ids::OBJECT_MENU) || self.dom.disabled(button) {
            self.show_error(&format!("menu: {name} is not available"));
            return;
        }
        self.on_object_menu_item(key);
    }

    /// R:2061 `drag` / `handle {page, x, y, dx?, dy?, handle?, shift?, alt?, meta?, escape?}`.
    pub(crate) fn drive_direct_gesture(&mut self, op: &str, contents: Option<&str>) {
        let Some(wanted) = self.drive_wanted(contents) else {
            return;
        };
        if self.doc.is_none() {
            return;
        }
        let (Some(page), Some(x), Some(y)) = (
            number(&wanted, "page"),
            number(&wanted, "x"),
            number(&wanted, "y"),
        ) else {
            return;
        };
        // `goTo` truncates and clamps; the page box is read at the asked index.
        self.go_to(page.trunc() as i64);
        self.drain();
        let size_ok = (page == page.trunc() && page >= 0.0)
            .then(|| {
                self.doc
                    .as_ref()
                    .and_then(|doc| doc.pages.get(page as usize))
            })
            .flatten()
            .is_some_and(|size| size.height >= 1.0);
        if !size_ok {
            return;
        }
        let handle = wanted
            .get("handle")
            .map_or_else(|| "grip".to_string(), js_string);
        self.drive_hit_gesture(
            op,
            page as usize,
            x,
            y,
            number(&wanted, "dx"),
            number(&wanted, "dy"),
            &handle,
            flag(&wanted, "shift"),
            flag(&wanted, "alt"),
            flag(&wanted, "meta"),
            flag(&wanted, "escape"),
        );
    }

    /// R:2103 `type {text}`: into the open inline editor (a select takes a known option; a
    /// text editor's `maxlength` truncates), then its `input` event.
    pub(crate) fn drive_direct_type(&mut self, contents: Option<&str>) {
        let wanted = self.drive_wanted(contents);
        let Some(text) = wanted
            .as_ref()
            .and_then(|w| w.get("text"))
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return;
        };
        let Some(active) = self.direct_active.as_mut() else {
            return;
        };
        let editor = &mut active.editor;
        if editor.tag == EditorTag::Select {
            let option = editor
                .options
                .iter()
                .find(|(value, label)| *label == text || *value == text)
                .cloned();
            let Some((value, _)) = option else {
                self.dom.set_text(ids::FIELDS_STATUS_EL, "not an option");
                return;
            };
            editor.value = value;
        } else {
            // setRangeText(text, start, end, "end") in UTF-16 offsets.
            let length = editor.value.encode_utf16().count();
            let start = editor.selection_start.unwrap_or(length).min(length);
            let end = editor.selection_end.unwrap_or(start).clamp(start, length);
            let (a, b) = (byte_at(&editor.value, start), byte_at(&editor.value, end));
            editor.value.replace_range(a..b, &text);
            let caret = start + text.encode_utf16().count();
            editor.selection_start = Some(caret);
            editor.selection_end = Some(caret);
            // maxlength does not stop setRangeText; §8.14 truncates and commits that one value.
            if editor.max_length > 0
                && editor.value.encode_utf16().count() > editor.max_length as usize
            {
                editor.value = js_slice16(&editor.value, editor.max_length as usize);
                let end = editor.value.encode_utf16().count();
                editor.selection_start = Some(end);
                editor.selection_end = Some(end);
            }
        }
        self.on_direct_editor_input(&text);
    }

    /// R:2114 `key {key}` with `Alt+`/`Meta+`/`Shift+` prefixes; the precedence mirrors the
    /// real keyboard model (spec §A.7).
    pub(crate) fn drive_direct_key(&mut self, contents: Option<&str>) {
        let wanted = self.drive_wanted(contents);
        let Some(raw) = wanted
            .as_ref()
            .and_then(|w| w.get("key"))
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            return;
        };
        let has_editor = self.direct_active.is_some();
        let (key, alt, meta, shift) = parse_key(&raw);
        let translated = if key == "Space" {
            " ".to_string()
        } else {
            key.clone()
        };
        // P5 U3 §9.12: Escape first closes an open sign or verify dialog.
        if key == "Escape"
            && (self.dom.is_open(ids::CERT_SIGN_DIALOG) || self.dom.is_open(ids::VERIFY_DIALOG))
        {
            self.close_dialog(ids::CERT_SIGN_DIALOG);
            self.close_dialog(ids::VERIFY_DIALOG);
            return;
        }
        // P6 U7 §10.35: ⌘C/⌘X/⌘V are the Edit-menu roles: a clipboard event on the focused element.
        let clipboard = match key.as_str() {
            "c" => Some("copy"),
            "x" => Some("cut"),
            "v" => Some("paste"),
            _ => None,
        };
        if let (true, Some(kind)) = (meta && !alt && !shift, clipboard) {
            // The focused element is a page target or the body (an open inline editor already
            // refuses the chord as `editorOpen`), and the drive has no text selection.
            self.on_clipboard(kind, None, true);
            return;
        }
        // P6 U5 §9.17: an open object menu owns every key; Escape closes it.
        if !self.dom.hidden(ids::OBJECT_MENU) {
            if key == "Escape" {
                self.close_object_menu(true);
            } else {
                self.object_menu_keydown(&translated, meta, alt, shift);
            }
            return;
        }
        let active_kind = self.direct_active.as_ref().map(|a| a.kind);
        // draw-signature D5: while armed, Enter places at the page centre and Escape disarms.
        if self.sign_armed && active_kind.is_none() && (key == "Enter" || key == "Escape") {
            if key == "Escape" {
                self.disarm_sign();
            } else {
                self.place_signature_at_centre();
            }
            return;
        }
        // P6 U6 §9.18: Escape or Enter leaves crop mode first (the selection stays).
        if self.direct_crop.is_some() && (key == "Escape" || key == "Enter") {
            self.leave_crop();
            return;
        }
        // §9.21: ⌥⌘←/→ on a group reaches the anchor's keydown wherever focus is.
        if alt
            && meta
            && (key == "ArrowLeft" || key == "ArrowRight")
            && active_kind.is_none()
            && self.direct_selection.len() >= 2
            && let Some(anchor) = self
                .direct_focused_object
                .as_ref()
                .map(|o| o.target_id.clone())
            && self.object_keydown(&anchor, &key, meta, alt, shift)
        {
            return;
        }
        let focused_field = self.direct_focused_field.clone();
        if key == "Space"
            && let Some(item) = focused_field
                .as_ref()
                .filter(|f| matches!(f.kind, FieldKind::Checkbox | FieldKind::Radio))
        {
            let value = toggled(item);
            let page = self.wanted_page.max(0) as usize;
            self.commit_field(item, value, page);
            return;
        }
        if (key == "ArrowLeft" || key == "ArrowRight")
            && !alt
            && let Some(item) = focused_field
                .as_ref()
                .filter(|f| f.kind == FieldKind::Radio)
        {
            self.move_radio(item.clone(), if key == "ArrowRight" { 1 } else { -1 });
            return;
        }
        if key == "Tab" {
            let page = self
                .direct_active
                .as_ref()
                .map_or(self.wanted_page, |a| a.page);
            let index = self.direct_field_index;
            if matches!(
                active_kind,
                Some(DirectKind::Add | DirectKind::Link | DirectKind::Textbox | DirectKind::Note)
            ) {
                self.finish_direct();
                return;
            }
            if active_kind.is_some() {
                self.finish_direct();
            }
            self.focus_next_field(shift, page.max(0) as usize, usize::try_from(index).ok());
            return;
        }
        // §9.7: a focused object or redaction mark owns the key (Delete, Enter, ⌥-arrows).
        if active_kind.is_none() && key != "Escape" {
            let group = self.direct_selection.len() >= 2;
            if self.focused_target_keydown(&translated, meta, alt, shift, group) {
                return;
            }
        }
        if alt && super::consts::ARROW_KEYS.contains(&key.as_str()) {
            let item = match self.direct_active.as_ref() {
                Some(active) if active.kind == DirectKind::Field => match &active.item {
                    DirectItem::Field(field) => Some(field.clone()),
                    _ => None,
                },
                _ => focused_field.clone(),
            };
            if let Some(item) = item {
                self.nudge_field(item, &key, shift, meta);
                return;
            }
        }
        let paragraph = self
            .direct_active
            .as_ref()
            .is_some_and(|a| a.paragraph.is_some());
        let add_or_note = matches!(active_kind, Some(DirectKind::Add | DirectKind::Note));
        // §9.7 plain Enter is a newline in the add and paragraph editors.
        if key == "Enter" && !meta && has_editor && (add_or_note || paragraph) {
            self.drive_direct_type(Some(r#"{"text":"\n"}"#));
            return;
        }
        if key == "Enter" && (meta || !(paragraph || add_or_note)) {
            if active_kind.is_some() {
                self.finish_direct();
            }
            return;
        }
        if key == "Enter" && active_kind.is_none() {
            return;
        }
        // X6: Escape peels one layer at a time: an open editor, then a selection, then the armed tool.
        if key == "Escape"
            && active_kind.is_none()
            && !self.has_direct_focus()
            && self.armed_tool().is_some()
        {
            self.disarm_tools();
            return;
        }
        // Review fix 8: Escape keeps a note that has text (an empty one is discarded).
        if key == "Escape"
            && active_kind == Some(DirectKind::Note)
            && let Some(active) = self.direct_active.clone()
            && !self.direct_unchanged(active)
        {
            self.finish_direct();
            return;
        }
        if key == "Escape" {
            if active_kind == Some(DirectKind::Add) {
                self.clear_error();
            }
            if active_kind.is_some() {
                self.close_direct();
            }
            self.direct_focused_field = None;
            self.drop_direct_focus();
            return;
        }
        self.drive_focused_keydown(&translated, meta, alt, shift);
        if key == "Space" {
            let field = match self.direct_active.as_ref() {
                Some(active) if active.kind == DirectKind::Field => match &active.item {
                    DirectItem::Field(field) => Some(field.clone()),
                    _ => None,
                },
                _ => None,
            };
            if let Some(item) =
                field.filter(|f| matches!(f.kind, FieldKind::Checkbox | FieldKind::Radio))
            {
                let value = toggled(&item);
                let page = self.wanted_page.max(0) as usize;
                self.commit_field(&item, value, page);
            }
        }
    }

    /// R:2182 `bookmark {action: add|rename|delete|go, index?, title?}`.
    pub(crate) fn drive_bookmark(&mut self, contents: Option<&str>) {
        self.clear_error();
        let Some(wanted) = self.drive_wanted(contents) else {
            return;
        };
        if self.doc.is_none() || !self.can_edit() {
            return;
        }
        let action = wanted
            .get("action")
            .and_then(Value::as_str)
            .map(str::to_string);
        let at = js_integer(wanted.get("index")).unwrap_or(-1);
        let title = js_slice16(&js_string_or_empty(&wanted, "title"), MAX_BOOKMARK_CHARS);
        if action.as_deref() == Some("add") {
            self.open_bookmark_dialog(None);
            self.dom.set_value(ids::BOOKMARK_TITLE_INPUT, title);
            let ok = self.save_bookmark_dialog();
            self.close_dialog(ids::BOOKMARK_DIALOG);
            self.refresh_outline();
            self.nav_last = Some(NavLast {
                action: Some("bookmark-add".into()),
                detail: (!ok).then(|| "invalid".to_string()),
                ..NavLast::default()
            });
            return;
        }
        self.refresh_outline();
        let entry = usize::try_from(at)
            .ok()
            .and_then(|i| self.outline.get(i))
            .cloned();
        let Some(entry) = entry else {
            self.nav_last = Some(NavLast {
                action: Some(format!(
                    "bookmark-{}",
                    js_string_or_empty(&wanted, "action")
                )),
                index: Some(at),
                detail: Some("no-such-bookmark".into()),
                ..NavLast::default()
            });
            return;
        };
        match action.as_deref() {
            Some("rename") => {
                self.open_bookmark_dialog(Some(entry.id));
                self.dom.set_value(ids::BOOKMARK_TITLE_INPUT, title);
                let ok = self.save_bookmark_dialog();
                self.close_dialog(ids::BOOKMARK_DIALOG);
                self.nav_last = Some(NavLast {
                    action: Some("bookmark-rename".into()),
                    index: Some(at),
                    detail: (!ok).then(|| "invalid".to_string()),
                    ..NavLast::default()
                });
            }
            Some("delete") => {
                self.delete_bookmark(entry.id);
                self.nav_last = Some(NavLast {
                    action: Some("bookmark-delete".into()),
                    index: Some(at),
                    ..NavLast::default()
                });
            }
            Some("go") => {
                self.follow_outline(&entry);
                self.nav_last = Some(NavLast {
                    action: Some("bookmark-go".into()),
                    index: Some(at),
                    ..NavLast::default()
                });
            }
            _ => {
                self.nav_last = Some(NavLast {
                    action: Some(js_string_or_empty(&wanted, "action")),
                    index: Some(at),
                    detail: Some("invalid-action".into()),
                    ..NavLast::default()
                });
                return;
            }
        }
        self.refresh_outline();
    }

    /// R:2225 `mode {mode, tool?}`: the chip the drive script cannot click.
    pub(crate) fn drive_mode(&mut self, contents: Option<&str>) {
        let wanted = self.drive_wanted(contents);
        let mode = wanted
            .as_ref()
            .and_then(|w| w.get("mode"))
            .and_then(Value::as_str)
            .and_then(UiMode::parse)
            .filter(|mode| DIRECT_MODES.contains(mode));
        let Some(mode) = mode else {
            self.show_error("mode: expected read, annotate, organize, forms, edit or redact");
            return;
        };
        self.set_mode(mode);
        // X6: `tool` arms that mode's explicit tool in the same step.
        if let Some(tool) = wanted.as_ref().and_then(|w| w.get("tool")) {
            let name = js_string(tool);
            let button = match name.as_str() {
                "add-text" => Some(ids::TEXT_ADD_TOOL_BUTTON),
                "textbox" => Some(ids::TEXTBOX_TOOL_BUTTON),
                _ => None,
            };
            match button {
                None => self.show_error(&format!("mode: unknown tool {name}")),
                Some(button) if !self.dom.disabled(button) => self.set_pressed(button, true),
                Some(_) => {}
            }
        }
        if mode == UiMode::Edit {
            self.refresh_content_targets(true);
        }
        let (width, height) = self.canvas_client_size();
        self.paint_hits(width, height, self.shown_page);
    }

    /// R:2244 `arm {tool}`: press the tool the click must pass through.
    pub(crate) fn drive_arm(&mut self, contents: Option<&str>) {
        let wanted = self.drive_wanted(contents);
        let tool = wanted
            .as_ref()
            .and_then(|w| w.get("tool"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        self.cancel_pending_tools();
        let button = lookup(&ARM_TOOLS, &tool);
        if tool != "off" && button.is_none() {
            self.show_error(&format!("arm: unknown tool {tool}"));
            return;
        }
        if let Some(button) = button {
            self.set_pressed(button, true);
        }
        let prompt = match tool.as_str() {
            "link" => Some(DRAW_LINK_PROMPT),
            "sigfield" => Some(DRAW_SIGFIELD_PROMPT),
            _ => None,
        };
        if let Some(prompt) = prompt {
            self.announce(prompt); // the same prompt the toolbar button announces
        }
        let (width, height) = self.canvas_client_size();
        self.paint_hits(width, height, self.shown_page);
    }

    /// R:2258 `runEditRequest(op, contents)`: main's `edit-request`. `contents` None is a menu
    /// press (open the dialog or arm the tool); present, a drive or smoke step's payload. The
    /// chain below is the list of renderer ops; anything else is refused by name.
    pub fn run_edit_request(&mut self, op: &str, contents: Option<&str>) {
        match op {
            "state" => {}
            // `pausedRetry.hidden` is never set (only its bar is hidden), so this refusal cannot fire.
            "page-retry" => {
                if self.dom.hidden(ids::PAUSED_RETRY) {
                    self.show_error("page-retry: nothing is paused");
                } else {
                    self.retry_paused();
                }
            }
            "click" => self.drive_direct_click(contents),
            "drag" | "handle" => self.drive_direct_gesture(op, contents),
            "menu" => self.drive_object_menu(contents),
            "type" => self.drive_direct_type(contents),
            "key" => self.drive_direct_key(contents),
            "mode" => self.drive_mode(contents),
            "arm" => self.drive_arm(contents),
            "outlines" => match self
                .drive_wanted(contents)
                .and_then(|w| w.get("on").and_then(Value::as_bool))
            {
                Some(on) => self.set_outlines(on),
                None => self.show_error("outlines: on must be true or false"),
            },
            // §8.8: the help dialog needs no document.
            "help" => self.help_op(contents),
            // Batch needs no document either: the menu opens the dialog, a drive runs it.
            "batch" => match contents {
                None => self.open_batch_dialog(),
                Some(contents) => self.drive_batch(contents),
            },
            // Compare: close works without a main document; everything else needs one.
            "compare" => {
                let close = contents.is_some()
                    && self
                        .drive_wanted(contents)
                        .is_some_and(|w| w.get("close") == Some(&Value::Bool(true)));
                if close {
                    self.drive_compare(contents.unwrap_or(""));
                    return;
                }
                if !self.unlocked_doc() {
                    self.show_error("Open a document first, then choose Compare with…");
                    return;
                }
                match contents {
                    None => self.request_compare(),
                    Some(contents) => self.drive_compare(contents),
                }
            }
            "undo" => self.smoke_undo(),
            "redo" => self.smoke_redo(),
            "note" => self.smoke_note(contents.unwrap_or("")),
            "highlight" => self.smoke_highlight(contents),
            "highlight-all" => {
                self.highlight_all_matches();
            }
            "select" => self.drive_select(contents),
            "underline" => self.drive_markup("Underline"),
            "strikeout" => self.drive_markup("StrikeOut"),
            "annot" => self.drive_annot(contents),
            "goto" => self.drive_goto(contents),
            "zoom" => self.drive_zoom(contents),
            "find" => self.drive_find(contents),
            "tab" => self.drive_tab(contents),
            "freetext" => self.smoke_free_text(contents),
            "pages" => self.smoke_pages(),
            "form" => self.smoke_form(contents),
            "fill-text" => self.drive_fill_text(contents),
            "sign-draw" => self.drive_sign_draw(contents),
            "sign-place" => self.drive_sign_place(contents),
            "fields" => self.smoke_field_design(contents),
            "nav" => self.smoke_nav(contents),
            "assemble" => self.smoke_assemble(),
            // The menu opens the dialogs; a `contents` payload marks the smoke path.
            "extract" => match contents {
                None => self.request_extract(),
                Some(_) => self.smoke_extract(),
            },
            "insert" => match contents {
                None => self.request_insert(),
                Some(_) => self.drive_insert(contents),
            },
            "new-from-images" => self.request_new_from_images(),
            "export" => self.request_export(),
            "watermark" => match contents {
                None => self.open_stamp_dialog(Some("watermark")),
                Some(_) => self.drive_stamp(contents),
            },
            "header-footer" => match contents {
                None => self.open_stamp_dialog(Some("header")),
                Some(_) => self.drive_stamp(contents),
            },
            "labels" => match contents {
                None => self.open_labels_dialog(),
                Some(_) => self.drive_labels(contents),
            },
            "decorate" => self.smoke_decorate(contents),
            "security" => self.open_security_dialog(),
            "cert-sign-open" => self.open_cert_sign_dialog(None),
            "verify-open" | "verify" => match contents {
                None => self.open_verify_dialog(),
                Some(_) => self.drive_verify(contents),
            },
            "encrypt" => self.smoke_encrypt(contents),
            "encrypt-keep" => self.smoke_encrypt_keep(contents),
            "password" => self.smoke_password(contents),
            "square" | "circle" | "line" | "ink" => self.add_shape_op(op),
            "stamp" => self.open_stamp_icon_dialog(),
            "sign" => match contents {
                None => self.drive_sign_place(None),
                Some(_) => self.drive_sign(contents),
            },
            "shapes" => self.smoke_shapes(contents),
            "export-png" | "export-jpeg" | "export-txt" => self.smoke_export(&op[7..]),
            "bookmark" => match contents {
                None => self.open_bookmark_dialog(None),
                Some(_) => self.drive_bookmark(contents),
            },
            "link" => match contents {
                None => self.open_link_dialog(LinkMode::Page),
                Some(_) => self.drive_link(contents),
            },
            "content-edit" => self.open_content_editor(),
            "text-add-open" => {
                self.select_tab_by_id(&ids::INSPECTOR_TABS, &ids::INSPECTOR_PANELS, "tab-text");
                self.set_inspector_open(true);
            }
            "text-add" => self.drive_text_add(contents),
            "content" => self.smoke_content(contents),
            "redact" => self.drive_redact(contents),
            "redact-preview" => self.drive_redact_preview(),
            "redact-apply" => self.drive_redact_apply(contents),
            // `redactAreaButton.click()`: a programmatic click (`event.detail === 0`), as the keyboard's.
            "redact-area" => self.on_redact_area_click(true),
            "redact-search" => match contents {
                None => self.redact_ui_open_find(),
                Some(_) => self.drive_redact_find(contents),
            },
            "redact-apply-open" => self.open_redact_dialog(),
            // Redaction redesign §2.3 (docs/agent.md "Redaction"): the tool's own drive steps.
            "redact-confirm" => self.drive_redact_confirm(contents),
            "redact-page" => {
                let page = js_integer(
                    self.drive_wanted(contents)
                        .as_ref()
                        .and_then(|w| w.get("page")),
                )
                .unwrap_or(self.wanted_page);
                self.add_redact_page(page.max(0) as usize);
            }
            "redact-pending" => self.drive_redact_pending(contents),
            "redact-original" => self.open_redact_original(),
            // §2.3 step 1: a real Escape keydown on the focused element.
            "redact-escape" => self.drive_escape_keydown(),
            "redact-underneath" => match self
                .drive_wanted(contents)
                .and_then(|w| w.get("on").and_then(Value::as_bool))
            {
                Some(on) => self.redact_ui_set_underneath(on),
                None => self.show_error("redact-underneath: on must be true or false"),
            },
            _ if PAGE_OPS.contains(&op) => self.run_page_op(op),
            // A typo must not read as a green step.
            _ => self.show_error(&format!("Unknown command \"{op}\"")),
        }
    }
}

/// `item.kind === "checkbox" ? item.value !== true : true`.
fn toggled(item: &FieldItem) -> FieldValue {
    if item.kind == FieldKind::Checkbox {
        FieldValue::Bool(item.value != FieldValue::Bool(true))
    } else {
        FieldValue::Bool(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_prefixes() {
        assert_eq!(
            parse_key("Alt+Meta+ArrowLeft"),
            ("ArrowLeft".into(), true, true, false)
        );
        assert_eq!(
            parse_key("Meta+Alt+K"),
            ("Alt+K".into(), false, true, false)
        );
        assert_eq!(parse_key("Shift+Tab"), ("Tab".into(), false, false, true));
    }

    #[test]
    fn utf16_offsets() {
        assert_eq!(byte_at("a😀b", 3), 5);
        assert_eq!(byte_at("ab", 9), 2);
        assert_eq!(number_or_string(Some(&json!(2.0))), "2");
        assert_eq!(number_or_string(Some(&json!(true))), "");
    }
}
