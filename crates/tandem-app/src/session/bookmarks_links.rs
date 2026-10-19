//! Bookmarks and links (renderer.ts 7810–8506): the outline and link panels, their dialogs,
//! link drawing (the shared page-area drag), the on-page link targets and their inline target
//! editor, the `nav` smoke; then the Comments list words and note adds that close the section.
#![allow(dead_code)]

use super::Session;
use super::dom_ids as ids;
use super::form_fields::{journal_of, nums};
use super::markup::{AreaDrag, AreaDragKind, char_box_at, collapse_spaces};
use super::types::*;
use crate::engine::Refusal;
use crate::js::js_trim;
use crate::messages;
use serde_json::{Map, Value, json};
use tandem_protocol::engine::LinkKind;
use tandem_protocol::limits::{MAX_BOOKMARK_CHARS, MAX_URI_CHARS};

/// A new link's destination (`{ page } | { uri }`).
#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    Page(i64),
    Uri(String),
}

impl LinkTarget {
    fn to_json(&self) -> Value {
        match self {
            LinkTarget::Page(page) => json!({"page": page}),
            LinkTarget::Uri(uri) => json!({"uri": uri}),
        }
    }
}

/// `link-set`'s change: a rect, a target, or both.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LinkChange {
    pub rect: Option<Vec<f64>>,
    pub target: Option<LinkTarget>,
}

/// One bookmark row (R:7887): the entry and the list depth it is drawn at.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineRow {
    pub entry: OutlineEntry,
    /// The nested `<ul>` the row lands in (an entry deeper than its parent's list sits in the
    /// deepest list there is).
    pub depth: usize,
    pub go_label: String,
    /// Rename and Delete are disabled without the edit permission.
    pub can_edit: bool,
}

/// R:7864: the `P2_EDIT.nav.links` entry for one page: page links keep their index, the rest their URI.
pub fn link_summary(items: &[LinkEntry]) -> Vec<NavLinkItem> {
    items
        .iter()
        .map(|item| match item.kind {
            LinkKind::Page => NavLinkItem {
                kind: "page".into(),
                page: item.page,
                uri: None,
            },
            kind => NavLinkItem {
                kind: kind.as_str().into(),
                page: None,
                uri: item.uri.clone(),
            },
        })
        .collect()
}

/// R:7934: "Page 3", the URI, or for a link the app does not follow what it is, in plain
/// words (walkthrough view#10: "Link to a file on this computer (not opened, for safety)").
pub fn link_label(item: &LinkEntry) -> String {
    match item.kind {
        LinkKind::Page => format!("Page {}", item.page.unwrap_or(0) + 1),
        LinkKind::Uri => item.uri.clone().unwrap_or_default(),
        _ => blocked_link_words(&scheme_name(item.uri.as_deref().unwrap_or(""))).to_string(),
    }
}

/// R:7934's "Blocked link (file:)": the announcement STATE keeps.
pub fn blocked_link_raw(item: &LinkEntry) -> String {
    format!(
        "Blocked link ({})",
        scheme_name(item.uri.as_deref().unwrap_or(""))
    )
}

/// What a link the app does not follow is, for people.
pub fn blocked_link_words(scheme: &str) -> &'static str {
    match scheme {
        "file:" => "Link to a file on this computer (not opened, for safety)",
        "javascript:" => "Link that runs a script (not run, for safety)",
        "unknown" | "" => "Link with no usable address",
        _ => "Link of a kind this app doesn't open (not opened, for safety)",
    }
}

/// R:7941: the scheme as the UI names it, with a colon when there is one
/// (`/^([A-Za-z][A-Za-z0-9+.-]*):/` on the trimmed URI).
pub fn scheme_name(uri: &str) -> String {
    let uri = js_trim(uri);
    let mut chars = uri.char_indices();
    if !matches!(chars.next(), Some((_, c)) if c.is_ascii_alphabetic()) {
        return "unknown".into();
    }
    for (at, c) in chars {
        if c == ':' {
            return format!("{}:", uri[..at].to_ascii_lowercase());
        }
        if !(c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')) {
            break;
        }
    }
    "unknown".into()
}

/// R:8021: the union of a search hit's quads, the anchor a new link takes (like a markup).
pub fn hit_rect(hit: &SearchHit) -> Option<Vec<f64>> {
    let quads: Vec<&Vec<f64>> = hit.quads.iter().filter(|quad| quad.len() == 8).collect();
    if quads.is_empty() {
        return None;
    }
    let xs = quads.iter().flat_map(|q| [q[0], q[2], q[4], q[6]]);
    let ys = quads.iter().flat_map(|q| [q[1], q[3], q[5], q[7]]);
    let (x0, x1) = xs.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
        (lo.min(v), hi.max(v))
    });
    let (y0, y1) = ys.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
        (lo.min(v), hi.max(v))
    });
    Some(vec![x0, y0, x1, y1])
}

/// R:8251: the link's `/Annots` object number (0 for an inline dict or an unpaired entry).
pub fn link_annot_id(item: &LinkEntry) -> i64 {
    item.annot_id
}

/// R:8335: the value the inline target editor opens with: a 1-based page, or the URI.
pub fn link_editor_value(item: &LinkEntry) -> String {
    match item.kind {
        LinkKind::Page => (item.page.unwrap_or(0) + 1).to_string(),
        _ => item.uri.clone().unwrap_or_default(),
    }
}

/// R:7893-7930: the nested list each bookmark lands in. A `<ul>` per depth is created lazily,
/// so an item with children owns one; an entry deeper than the lists so far sits in the
/// deepest one there is.
pub fn outline_depths(outline: &[OutlineEntry]) -> Vec<usize> {
    let mut lists = 1usize;
    let mut depths = Vec::with_capacity(outline.len());
    for (index, entry) in outline.iter().enumerate() {
        let depth = (entry.depth.max(0) as usize).min(lists - 1);
        if outline
            .get(index + 1)
            .is_some_and(|next| next.depth == depth as i64 + 1)
        {
            lists = lists.max(depth + 2);
        }
        depths.push(depth);
    }
    depths
}

/// `/^\d+$/`.
fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// `String#slice(0, n)` over characters.
fn take_chars(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

impl Session {
    /// R:7816: the document's bookmarks; the reply's `id` is the preorder index.
    pub fn refresh_outline(&mut self) {
        self.outline_seq += 1;
        if self.doc.as_ref().is_none_or(|doc| doc.info.needs_password) {
            self.outline = Vec::new();
            self.render_outline();
            return;
        }
        // A document that changed under us owns the panel now.
        let Ok(reply) = self.request(json!({"op": "outline"}), vec![]) else {
            return;
        };
        let Some(items) = super::parse_reply(reply.get("items").unwrap_or(&Value::Null)) else {
            return;
        };
        self.outline = items;
        self.render_outline();
    }

    /// R:7837: the shown page's links, plus the smoke's `P2_EDIT.nav.links` slice for that page.
    pub fn refresh_links(&mut self) {
        self.links_seq += 1;
        if self.doc.as_ref().is_none_or(|doc| doc.info.needs_password) {
            self.links = Vec::new();
            self.links_page = self.wanted_page;
            self.render_links();
            return;
        }
        let page = self.wanted_page;
        let items = self
            .engine
            .request(json!({"op": "links", "page": page}), vec![])
            .ok()
            .and_then(|reply| {
                super::parse_reply::<Vec<LinkEntry>>(reply.get("items").unwrap_or(&Value::Null))
            });
        let Some(items) = items else {
            // §9.5: a current failure empties the page's links (so the drive's wait passes).
            self.links = Vec::new();
            self.links_page = page;
            self.render_links();
            self.repaint_hits();
            return;
        };
        self.nav_links.insert(page, link_summary(&items));
        self.links = items;
        self.links_page = page;
        self.render_links();
        if page == self.shown_page {
            self.repaint_hits();
        }
    }

    /// R:7871: the nav smoke reads every page's links; every other path reads only the shown one.
    /// An engine refusal ends it (the TS threw out of the drive step).
    pub fn load_all_links(&mut self) -> Result<(), Refusal> {
        let Some(count) = self.doc.as_ref().map(|doc| doc.page_count) else {
            return Ok(());
        };
        for page in 0..count {
            let reply = self
                .engine
                .request(json!({"op": "links", "page": page}), vec![])?;
            let items: Vec<LinkEntry> =
                serde_json::from_value(reply.get("items").cloned().unwrap_or(Value::Null))
                    .map_err(|e| Refusal::plain(e.to_string()))?;
            self.nav_links.insert(page, link_summary(&items));
            if page == self.wanted_page {
                self.links = items;
                self.links_page = page;
                self.render_links();
            }
        }
        Ok(())
    }

    /// R:7887: the outline panel's empty line; the rows are [`Self::outline_rows`].
    pub fn render_outline(&mut self) {
        self.set_empty(
            ids::OUTLINE_EMPTY_EL,
            self.outline.len(),
            "No bookmarks in this document.",
            "No document open yet.",
        );
    }

    /// R:7893-7930: one row per bookmark, at the depth of the nested list it is drawn in.
    pub fn outline_rows(&self) -> Vec<OutlineRow> {
        let can_edit = self.can_edit();
        self.outline
            .iter()
            .zip(outline_depths(&self.outline))
            .map(|(entry, depth)| {
                let go_label = match entry.page {
                    Some(page) => format!("Go to bookmark {}, page {}", entry.title, page + 1),
                    None => format!("Bookmark {}", entry.title),
                };
                OutlineRow {
                    entry: entry.clone(),
                    depth,
                    go_label,
                    can_edit,
                }
            })
            .collect()
    }

    /// R:7946: the links panel's heading and empty line (the rows come from `links`).
    pub fn render_links(&mut self) {
        self.dom.set_text(
            ids::LINKS_HEADING,
            format!("Links on page {}", self.wanted_page + 1),
        );
        self.set_empty(
            ids::LINKS_EMPTY_EL,
            self.links.len(),
            "No links on this page.",
            "No document open yet.",
        );
        self.sync_link_current();
    }

    /// R:7989: following a bookmark: a page destination navigates, a web one goes to main.
    pub fn follow_outline(&mut self, entry: &OutlineEntry) {
        if let Some(page) = entry.page {
            self.go_to(page);
            return;
        }
        let Some(uri) = entry.uri.as_deref().filter(|uri| !uri.is_empty()) else {
            self.announce_nav("That bookmark has no destination");
            return;
        };
        let scheme = scheme_name(uri);
        if matches!(scheme.as_str(), "http:" | "https:" | "mailto:") {
            self.host.open_link(uri);
            return;
        }
        self.announce_nav(&format!("Blocked link ({scheme})"));
        if !self.headless {
            self.show_message(
                &format!("{}.", blocked_link_words(&scheme)),
                crate::messages::MessageKind::Info,
            );
        }
    }

    /// R:8007: following a link: only a page or an allowed URI has an action.
    pub fn follow_link(&mut self, item: &LinkEntry) {
        match (item.kind, item.uri.as_deref()) {
            (LinkKind::Page, _) => {
                let page = item.page.unwrap_or(0);
                self.nav_last = Some(NavLast {
                    action: Some("link-follow".into()),
                    kind: Some("page".into()),
                    page: Some(page),
                    ..NavLast::default()
                });
                self.go_to(page);
            }
            (LinkKind::Uri, Some(uri)) if !uri.is_empty() => {
                self.nav_last = Some(NavLast {
                    action: Some("link-follow".into()),
                    kind: Some("uri".into()),
                    uri: Some(uri.into()),
                    ..NavLast::default()
                });
                self.host.open_link(uri);
            }
            _ => {
                self.nav_last = Some(NavLast {
                    action: Some("link-follow".into()),
                    kind: Some("blocked".into()),
                    uri: Some(item.uri.clone().unwrap_or_default()),
                    ..NavLast::default()
                });
                // The same words as a blocked bookmark; the window also says why (view#10).
                self.announce_nav(&blocked_link_raw(item));
                if !self.headless {
                    self.show_message(
                        &format!("{}.", link_label(item)),
                        crate::messages::MessageKind::Info,
                    );
                }
            }
        }
    }

    /// R:8029: `#nav-status` shows the text as given; the edit live region is cleared.
    pub fn announce_nav(&mut self, text: &str) {
        self.dom.set_text(ids::NAV_STATUS_EL, text);
        self.dom.set_text(ids::EDIT_STATUS_EL, "");
    }

    /// R:8035: "Add bookmark" and a bookmark's Rename share this dialog.
    pub fn open_bookmark_dialog(&mut self, outline_id: Option<i64>) {
        if self.doc.is_none() || !self.can_edit() {
            return;
        }
        self.editing_bookmark = outline_id;
        let entry =
            outline_id.and_then(|id| self.outline.iter().find(|item| item.id == id).cloned());
        self.dom.set_text(
            ids::BOOKMARK_DIALOG_HEADING,
            if entry.is_some() {
                "Rename bookmark"
            } else {
                "Add bookmark"
            },
        );
        let title = entry.map_or_else(
            || format!("Page {}", self.wanted_page + 1),
            |entry| entry.title,
        );
        self.dom.set_value(ids::BOOKMARK_TITLE_INPUT, title);
        self.dom.set_open(ids::BOOKMARK_DIALOG, true);
    }

    /// R:8046.
    pub fn save_bookmark_dialog(&mut self) -> bool {
        let outline_id = self.editing_bookmark;
        let title = take_chars(
            &self.dom.value(ids::BOOKMARK_TITLE_INPUT),
            MAX_BOOKMARK_CHARS,
        );
        if js_trim(&title).is_empty() {
            // The engine refuses it too; keeping the dialog open is the keyboard-friendly path.
            self.show_error("A bookmark needs a title");
            return false;
        }
        self.dom.set_open(ids::BOOKMARK_DIALOG, false);
        self.editing_bookmark = None; // the dialog's close listener (R:10909)
        match outline_id {
            None => {
                let page = self.wanted_page;
                self.add_bookmark(&title, page)
            }
            Some(id) => self.rename_bookmark(id, &title),
        }
    }

    /// R:8060.
    pub fn add_bookmark(&mut self, title: &str, page: i64) -> bool {
        match self.request(
            json!({"op": "outline-add", "title": title, "page": page}),
            vec![],
        ) {
            Ok(reply) => {
                self.apply_edit(&journal_of(&reply), "");
                self.announce_nav(&format!("Added bookmark Page {}", page + 1));
                true
            }
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
                false
            }
        }
    }

    /// R:8072.
    pub fn rename_bookmark(&mut self, outline_id: i64, title: &str) -> bool {
        match self.request(
            json!({"op": "outline-set", "outlineId": outline_id, "title": title}),
            vec![],
        ) {
            Ok(reply) => {
                self.apply_edit(&journal_of(&reply), "");
                self.announce_nav("Renamed bookmark");
                true
            }
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
                false
            }
        }
    }

    /// R:8084.
    pub fn delete_bookmark(&mut self, outline_id: i64) {
        match self.request(
            json!({"op": "outline-delete", "outlineId": outline_id}),
            vec![],
        ) {
            Ok(reply) => {
                self.apply_edit(&journal_of(&reply), "");
                self.announce_nav("Deleted bookmark");
            }
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
            }
        }
    }

    /// R:8098: "Link to page…" and "Web link…" share one dialog; both anchor on the current
    /// search match, whose rect is captured when the dialog opens.
    pub fn open_link_dialog(&mut self, mode: LinkMode) {
        let rect = self.current_hit().and_then(|hit| hit_rect(&hit));
        if self.doc.is_none() || !self.can_annotate() {
            return;
        }
        let Some(rect) = rect else {
            self.announce_nav("Find something first");
            return;
        };
        self.open_link_dialog_at(rect, mode);
    }

    /// R:8109.
    pub fn open_link_dialog_at(&mut self, rect: Vec<f64>, mode: LinkMode) {
        self.link_anchor = Some(rect);
        self.set_link_mode(mode, false);
        self.dom.set_value(
            ids::LINK_TARGET_INPUT,
            if mode == LinkMode::Page {
                ""
            } else {
                "https://"
            },
        );
        self.dom.set_open(ids::LINK_DIALOG, true);
    }

    /// R:8119: the dialog's Page / Web choice; switching keeps a typed value and swaps only the
    /// other kind's default.
    pub fn set_link_mode(&mut self, mode: LinkMode, swap_default: bool) {
        self.link_mode = mode;
        self.dom.set_value(ids::LINK_KIND_SELECT, mode.as_str());
        let page = mode == LinkMode::Page;
        self.dom.set_text(
            ids::LINK_DIALOG_HEADING,
            if page { "Link to page" } else { "Web link" },
        );
        self.dom.set_text(
            ids::LINK_TARGET_LABEL,
            if page { "Page number" } else { "Web address" },
        );
        let placeholder = if page {
            format!("1–{}", self.doc.as_ref().map_or(1, |doc| doc.page_count))
        } else {
            "https://".into()
        };
        self.dom
            .set_attr(ids::LINK_TARGET_INPUT, "placeholder", placeholder);
        let value = self.dom.value(ids::LINK_TARGET_INPUT);
        if swap_default && mode == LinkMode::Uri && value.is_empty() {
            self.dom.set_value(ids::LINK_TARGET_INPUT, "https://");
        }
        if swap_default && mode == LinkMode::Page && value == "https://" {
            self.dom.set_value(ids::LINK_TARGET_INPUT, "");
        }
    }

    /// UI entry: `#link-kind` changed (R:10915).
    pub fn on_link_kind(&mut self, value: &str) {
        self.set_link_mode(
            if value == "uri" {
                LinkMode::Uri
            } else {
                LinkMode::Page
            },
            true,
        );
    }

    /// R:8133: P6 U4 §9.8: one page-level area drag (Draw link, Mark area, unarmed Redact).
    /// `point` is the press in page points. Under 3 CSS px it is a click.
    pub fn start_area_drag(&mut self, page: i64, point: Point, kind: AreaDragKind) {
        self.lane4.area_drag = Some(AreaDrag {
            page,
            start: point,
            kind,
            draft: None,
        });
    }

    /// Whether `#hit-layer .area-draft` exists: startAreaDrag appends it at the press and
    /// removes it at the release (R:8133).
    pub fn area_draft_open(&self) -> bool {
        self.lane4.area_drag.is_some()
    }

    /// `toPt` (R:8137): a page point kept inside the page and rounded to 2 decimals.
    fn area_point(&self, page: i64, point: Point) -> Option<Point> {
        let size = self.doc.as_ref()?.pages.get(usize::try_from(page).ok()?)?;
        if self.shown_zoom <= 0.0 {
            return None;
        }
        let round = |v: f64| super::markup::r2(v);
        Some([
            round(point[0].clamp(0.0, size.width)),
            round(point[1].clamp(0.0, size.height)),
        ])
    }

    fn area_rect(&self, drag: &AreaDrag, point: Point) -> Option<Vec<f64>> {
        let a = self.area_point(drag.page, drag.start)?;
        let b = self.area_point(drag.page, point)?;
        Some(vec![
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[0].max(b[0]),
            a[1].max(b[1]),
        ])
    }

    /// The drag's length in CSS px.
    fn area_distance(&self, drag: &AreaDrag, point: Point) -> f64 {
        (point[0] - drag.start[0]).hypot(point[1] - drag.start[1]) * self.shown_zoom
    }

    /// UI entry: the pointer moved during an area drag: the draft shows from 3 CSS px.
    pub fn on_area_drag_move(&mut self, point: Point) {
        let Some(drag) = self.lane4.area_drag.clone() else {
            return;
        };
        let rect = if self.area_distance(&drag, point) >= 3.0 {
            self.area_rect(&drag, point)
        } else {
            None
        };
        if let Some(drag) = self.lane4.area_drag.as_mut() {
            drag.draft = rect;
        }
    }

    /// UI entry: the pointer went up (or was cancelled) during an area drag.
    pub fn on_area_drag_end(&mut self, point: Point, cancelled: bool) {
        let Some(drag) = self.lane4.area_drag.take() else {
            return;
        };
        // The trailing click never reaches click-to-add.
        self.add_blocked = true;
        // `onEnd`: the tool that started the drag disarms (R:11122-11135).
        match drag.kind {
            AreaDragKind::RedactArmed => {
                self.set_tool_pressed(ids::REDACT_AREA_BUTTON, false);
                self.repaint_hits();
            }
            AreaDragKind::Link => {
                self.set_tool_pressed(ids::LINK_DRAW_BUTTON, false);
                self.announce("");
                self.repaint_hits();
            }
            AreaDragKind::SignatureField => {
                self.set_tool_pressed(ids::SIGFIELD_DRAW_BUTTON, false);
                self.announce("");
                self.repaint_hits();
            }
            AreaDragKind::RedactUnarmed => {}
        }
        if cancelled || self.area_distance(&drag, point) < 3.0 {
            return;
        }
        let Some(rect) = self.area_rect(&drag, point) else {
            return;
        };
        // runGestureWork: the error bar is cleared before the work.
        self.clear_error();
        let page = drag.page.max(0) as usize;
        match drag.kind {
            AreaDragKind::Link => self.drawn_link(drag.page, rect),
            AreaDragKind::SignatureField => self.add_signature_field(page, rect),
            AreaDragKind::RedactArmed | AreaDragKind::RedactUnarmed => {
                self.add_redact_area(page, rect, messages::redact::ADDED);
            }
        }
    }

    /// R:8184: §1.6: a drawn rect opens the link dialog on it; under 4 × 4 pt is refused.
    pub fn drawn_link(&mut self, page: i64, rect: Vec<f64>) {
        if self.doc.is_none() || !self.can_annotate() {
            return;
        }
        if rect.len() < 4 || rect[2] - rect[0] < 4.0 || rect[3] - rect[1] < 4.0 {
            self.show_error("Draw a bigger link");
            return;
        }
        let mode = self.link_mode;
        self.open_link_dialog_at(rect.clone(), mode);
        self.nav_link_draft = Some(NavLinkDraft {
            page,
            rect: rect.clone(),
        });
        self.nav_last = Some(NavLast {
            action: Some("link-draw".into()),
            page: Some(page),
            rect: Some(rect),
            ..NavLast::default()
        });
    }

    /// R:8193: the anchor and the draft go with the dialog.
    pub fn close_link_dialog(&mut self) {
        self.dom.set_open(ids::LINK_DIALOG, false);
        self.link_anchor = None;
        self.nav_link_draft = None;
    }

    /// R:8198.
    pub fn save_link_dialog(&mut self) -> bool {
        let anchor = self.link_anchor.clone();
        let mode = self.link_mode;
        let text = js_trim(&self.dom.value(ids::LINK_TARGET_INPUT)).to_string();
        let (Some(count), Some(anchor)) = (self.doc.as_ref().map(|doc| doc.page_count), anchor)
        else {
            self.close_link_dialog();
            return false;
        };
        if mode == LinkMode::Page {
            let page = super::field_designer::js_to_number(&text);
            if page.fract() != 0.0 || !page.is_finite() || page < 1.0 || page > count as f64 {
                self.show_error(&format!("Enter a page number between 1 and {count}"));
                return false;
            }
            self.close_link_dialog();
            let at = self.wanted_page;
            return self.add_link(at, anchor, LinkTarget::Page(page as i64 - 1));
        }
        if text.is_empty() {
            // Keep the refusal in the dialog rather than round-tripping a URI the engine will reject.
            self.show_error("Enter a web address");
            return false;
        }
        self.close_link_dialog();
        let at = self.wanted_page;
        self.add_link(
            at,
            anchor,
            LinkTarget::Uri(take_chars(&text, MAX_URI_CHARS)),
        )
    }

    /// R:8225.
    pub fn add_link(&mut self, page: i64, rect: Vec<f64>, target: LinkTarget) -> bool {
        let request = json!({"op": "link-add", "page": page, "rect": nums(&rect), "target": target.to_json()});
        match self.request(request, vec![]) {
            Ok(reply) => {
                self.apply_edit(&journal_of(&reply), "");
                let words = match target {
                    LinkTarget::Page(page) => format!("Added link to page {}", page + 1),
                    LinkTarget::Uri(_) => "Added web link".into(),
                };
                self.announce_nav(&words);
                true
            }
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
                false
            }
        }
    }

    /// R:8237: a deleted link takes its on-page focus with it.
    pub fn delete_link(&mut self, link_id: i64) {
        let gone = self
            .links
            .iter()
            .find(|entry| entry.id == link_id)
            .map(link_annot_id);
        if gone.is_some() && self.direct_focused_link.map(|f| f.annot_id) == gone {
            // (markSelected(null) only repainted the selection outline.)
            self.direct_focused_link = None;
            self.sync_link_current();
        }
        let request = json!({"op": "link-delete", "page": self.wanted_page, "linkId": link_id});
        match self.request(request, vec![]) {
            Ok(reply) => {
                self.apply_edit(&journal_of(&reply), "");
                self.announce_nav("Deleted link");
            }
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
            }
        }
    }

    /// R:8257: P6 U4 §9.7: every link-set records `nav.last`; a noop writes nothing else.
    pub fn link_set(
        &mut self,
        page: i64,
        item: &LinkEntry,
        change: LinkChange,
        announcement: &str,
        silent: bool,
    ) -> bool {
        let index = item.id;
        let mut request = Map::new();
        request.insert("op".into(), json!("link-set"));
        request.insert("page".into(), json!(page));
        request.insert("annotId".into(), json!(link_annot_id(item)));
        if let Some(rect) = &change.rect {
            request.insert("rect".into(), nums(rect));
        }
        if let Some(target) = &change.target {
            request.insert("target".into(), target.to_json());
        }
        match self.request(Value::Object(request), vec![]) {
            Ok(reply) => {
                self.nav_last = Some(NavLast {
                    action: Some("link-set".into()),
                    index: Some(index),
                    ..NavLast::default()
                });
                if reply.get("noop") != Some(&json!(true)) {
                    self.apply_edit(&journal_of(&reply), announcement);
                }
                true
            }
            Err(err) => {
                let detail = err
                    .reply
                    .get("detail")
                    .and_then(Value::as_str)
                    .unwrap_or("invalid")
                    .to_string();
                self.nav_last = Some(NavLast {
                    action: Some("link-set".into()),
                    index: Some(index),
                    detail: Some(detail),
                    ..NavLast::default()
                });
                if !silent {
                    let text = match err.reply.get("error").and_then(Value::as_str) {
                        Some(text) => text.to_string(),
                        None => self.message_of(&err),
                    };
                    self.show_error(&text);
                }
                false
            }
        }
    }

    /// R:8273: §1.4: one `link-set {rect}` from a drag, resize or nudge (rounded like
    /// `commitMarkRect`); a refusal snaps back. The rect gesture's commit for a link target.
    pub fn commit_link_rect(&mut self, item: &LinkEntry, page: i64, rect: &[f64]) -> bool {
        self.direct_focused_link = Some(AnnotRef {
            annot_id: link_annot_id(item),
        });
        self.direct_focused_object = None;
        self.direct_focused_mark = None;
        let rect: Vec<f64> = rect
            .iter()
            .map(|v| crate::js::js_round(v * 1e4) / 1e4)
            .collect();
        let ok = self.link_set(
            page,
            item,
            LinkChange {
                rect: Some(rect),
                target: None,
            },
            "Moved link",
            false,
        );
        if !ok {
            self.repaint_hits();
        }
        ok
    }

    /// R:8281: §1.7: the Links row of the focused link is the current one. The UI draws
    /// `aria-current` from `direct_focused_link`; there is nothing else to sync.
    pub fn sync_link_current(&mut self) {}

    /// R:8288: `button` is the link target's hit-layer node (`markSelected` + focus).
    pub fn focus_link(&mut self, item: &LinkEntry, button: Option<super::hit::NodeKey>) {
        self.direct_focused_link = Some(AnnotRef {
            annot_id: link_annot_id(item),
        });
        self.direct_focused_object = None;
        self.direct_focused_mark = None;
        self.direct_focused_field = None;
        self.mark_selected(button.as_ref());
        if let Some(key) = &button {
            self.focus_node(key);
        }
        self.sync_note_current();
        self.sync_link_current();
    }

    /// R:8339: the inline target editor over a link (Annotate mode).
    pub fn open_link_editor(&mut self, item: &LinkEntry, page: i64) {
        if self.doc.is_none() || !self.can_annotate() || link_annot_id(item) <= 0 {
            return;
        }
        self.clear_error();
        self.close_direct();
        let value = link_editor_value(item);
        let mut editor = EditorState {
            tag: EditorTag::Input,
            class_name: "inline-editor link-editor".into(),
            value: value.clone(),
            max_length: MAX_URI_CHARS as i64,
            title: "Enter a page number or a web address; Enter saves, Escape cancels".into(),
            initial: Some(value),
            ..EditorState::default()
        };
        super::direct::place_editor(&mut editor, &item.rect, self.shown_zoom);
        self.lane2.editor_label = format!(
            "Link target (page number or web address): {}",
            link_label(item)
        );
        self.direct_focused_link = Some(AnnotRef {
            annot_id: link_annot_id(item),
        });
        self.direct_focused_object = None;
        self.direct_focused_mark = None;
        self.direct_active = Some(DirectActive {
            kind: DirectKind::Link,
            item: DirectItem::Link(item.clone()),
            editor,
            page,
            ..DirectActive::default()
        });
        self.setup_inline_events(false);
        self.focus_editor();
        self.select_editor_text();
        self.sync_link_current();
    }

    /// R:8356: §1.5: digits → a page (checked like the dialog), anything else → a URI; a
    /// refusal keeps the editor and its draft. `silent` (§9.6, a mode or page change): a
    /// refusal drops the draft without #error.
    pub fn commit_link_editor(&mut self, active: &DirectActive, silent: bool) -> bool {
        let DirectItem::Link(edited) = &active.item else {
            return false;
        };
        let item = self
            .links
            .iter()
            .find(|entry| link_annot_id(entry) == link_annot_id(edited))
            .cloned()
            .unwrap_or_else(|| edited.clone());
        let value = js_trim(&active.editor.value).to_string();
        let Some(count) = self.doc.as_ref().map(|doc| doc.page_count) else {
            return false;
        };
        if all_digits(&value) {
            let number = value.parse::<f64>().unwrap_or(f64::INFINITY);
            if number < 1.0 || number > count as f64 {
                if !silent {
                    self.show_error(&format!("Enter a page number between 1 and {count}"));
                }
                return false;
            }
            let change = LinkChange {
                rect: None,
                target: Some(LinkTarget::Page(number as i64 - 1)),
            };
            return self.link_set(
                active.page,
                &item,
                change,
                &format!("Link now goes to page {}", number as i64),
                silent,
            );
        }
        let change = LinkChange {
            rect: None,
            target: Some(LinkTarget::Uri(take_chars(&value, MAX_URI_CHARS))),
        };
        self.link_set(
            active.page,
            &item,
            change,
            "Link now opens a web address",
            silent,
        )
    }

    /// R:8368: §1.8: STATE `direct.active` for a focused or edited link. `css_box` is the
    /// target's or editor's `{left, top, width, height}`.
    pub fn link_state(
        &self,
        item: &LinkEntry,
        css_box: Value,
        editing: bool,
        draft: Option<&str>,
    ) -> Value {
        let target = match item.kind {
            LinkKind::Page => json!({"kind": "page", "page": item.page.unwrap_or(0)}),
            kind => json!({"kind": kind.as_str(), "uri": item.uri.clone().unwrap_or_default()}),
        };
        let mut out = Map::new();
        out.insert("kind".into(), json!("link"));
        out.insert("annotId".into(), json!(link_annot_id(item)));
        out.insert("id".into(), json!(item.id));
        out.insert("page".into(), json!(self.wanted_page));
        out.insert("box".into(), css_box);
        out.insert("rect".into(), nums(&item.rect));
        out.insert("target".into(), target);
        out.insert("editing".into(), json!(editing));
        if let Some(draft) = draft {
            out.insert("draft".into(), json!(draft));
        }
        Value::Object(out)
    }

    /// R:8380: `--edit=nav`: bookmark the last page, link the search hit "one" (page 1) to
    /// page 3, delete page 2, then log.
    pub fn smoke_nav(&mut self, contents: Option<&str>) {
        let Some(count) = self.doc.as_ref().map(|doc| doc.page_count) else {
            self.log_edit();
            return;
        };
        let title = contents.unwrap_or("PDF4All P2 nav").to_string();
        self.add_bookmark(&title, count - 1);
        self.dom.set_value(ids::SEARCH_INPUT, "one");
        self.start_search();
        let hit = self
            .search_hits
            .iter()
            .find(|item| item.page == 0)
            .or(self.search_hits.first())
            .cloned();
        if let Some(hit) = hit
            && let Some(rect) = hit_rect(&hit)
        {
            self.add_link(hit.page, rect, LinkTarget::Page(2));
        }
        self.delete_page(1);
        if self.load_all_links().is_err() {
            return;
        }
        self.log_edit();
        // The blocked link's own activation: main re-checks the allowlist and logs P2_LINK
        // with allowed:false rather than opening anything.
        let blocked = self
            .links
            .iter()
            .find(|item| item.kind == LinkKind::Blocked)
            .and_then(|item| item.uri.clone());
        if let Some(uri) = blocked.filter(|uri| !uri.is_empty()) {
            self.host.open_link(&uri);
        }
    }

    /// R:8401: a Comments row's words: the marked text of a markup, the comment, or a stand-in.
    pub fn annot_summary(&mut self, item: &AnnotItem) -> String {
        let text = js_trim(&collapse_spaces(&item.contents)).to_string();
        // Item 6: a markup is named by the words it marks ("Strikethrough on page 2: 'quarterly'").
        let marked = if matches!(
            item.r#type.as_str(),
            "Highlight" | "Underline" | "StrikeOut"
        ) {
            self.marked_text(&item.rect)
        } else {
            String::new()
        };
        let quoted = if marked.is_empty() {
            String::new()
        } else if marked.chars().count() > 50 {
            format!("‘{}…’", take_chars(&marked, 50))
        } else {
            format!("‘{marked}’")
        };
        let comment = if text.chars().count() > 60 {
            format!("{}…", take_chars(&text, 60))
        } else {
            text
        };
        if !quoted.is_empty() && !comment.is_empty() {
            return format!("{quoted} — {comment}");
        }
        if !quoted.is_empty() || !comment.is_empty() {
            return if quoted.is_empty() { comment } else { quoted };
        }
        match item.r#type.as_str() {
            "Text" => "(empty note)",
            "FreeText" => "(empty text box)",
            _ => "(no comment)",
        }
        .into()
    }

    /// R:8415: the shown page's text inside `rect` (page points, y down), from the text layer's
    /// lines; "" when unknown (and the list is drawn again once the text arrives).
    pub fn marked_text(&mut self, rect: &[f64]) -> String {
        let lines: Vec<tandem_protocol::engine::Line> = if self.shown_page == self.wanted_page {
            self.text_layer().map(<[_]>::to_vec).unwrap_or_default()
        } else {
            Vec::new()
        };
        if lines.is_empty() {
            self.notes_need_text = true;
        }
        let [x0, y0, x1, y1] = match rect {
            [a, b, c, d] if !lines.is_empty() => [*a, *b, *c, *d],
            _ => return String::new(),
        };
        let mut words: Vec<String> = Vec::new();
        let scale = self.text_layer_scale();
        // Each drawn character's own box on the text layer: a character is marked when its centre is inside.
        for line in &lines {
            let chars: Vec<char> = line.text.chars().collect();
            let mut picked = String::new();
            for (i, c) in chars.iter().enumerate() {
                let [x, y, w, h] = char_box_at(line, i, chars.len(), scale);
                let (cx, cy) = (x + w / 2.0, y + h / 2.0);
                if cx >= x0 && cx <= x1 && cy >= y0 && cy <= y1 {
                    picked.push(*c);
                }
            }
            let picked = js_trim(&picked);
            if !picked.is_empty() {
                words.push(picked.to_string());
            }
        }
        collapse_spaces(&words.join(" "))
    }

    /// R:8438: the Comments panel's heading and empty line (the rows are drawn by the UI from
    /// `notes`, without Redact marks, with `annot_name` and `annot_summary`).
    pub fn render_notes(&mut self) {
        self.dom.set_text(
            ids::NOTES_HEADING,
            format!("Comments on page {}", self.wanted_page + 1),
        );
        // Redaction redesign §2.4: redaction boxes are listed in the Redactions panel.
        let listed = self
            .notes
            .iter()
            .filter(|item| item.r#type != "Redact")
            .count();
        self.sync_note_current();
        self.set_empty(
            ids::NOTES_EMPTY_EL,
            listed,
            "No annotations on this page yet.",
            "No document open yet.",
        );
    }

    /// R:8483: adds a 20x20 Text note whose top-left is `point` (PDF points, y down).
    pub fn add_note_at(
        &mut self,
        page: i64,
        point: Point,
        contents: &str,
        announcement: &str,
    ) -> bool {
        if self.doc.is_none() || !self.can_annotate() {
            self.show_error("This document does not permit annotations.");
            return false;
        }
        let request = json!({"op": "annot-add", "page": page, "type": "Text", "point": nums(&point), "contents": contents});
        match self.request(request, vec![]) {
            Ok(reply) => {
                self.apply_edit(&journal_of(&reply), announcement);
                true
            }
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
                false
            }
        }
    }

    /// R:8501: the current search match, but only when it is on the page being shown.
    pub fn current_hit(&self) -> Option<SearchHit> {
        let index = usize::try_from(self.search_index).ok()?;
        let hit = self.search_hits.get(index)?;
        (hit.page == self.wanted_page).then(|| hit.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemes_and_labels() {
        assert_eq!(scheme_name(" HTTPS://x"), "https:");
        assert_eq!(scheme_name("file:///etc"), "file:");
        assert_eq!(scheme_name("no scheme"), "unknown");
        assert_eq!(scheme_name("1a:b"), "unknown");
        let page = LinkEntry {
            kind: LinkKind::Page,
            page: Some(2),
            ..LinkEntry::default()
        };
        assert_eq!(link_label(&page), "Page 3");
        assert_eq!(link_editor_value(&page), "3");
        let blocked = LinkEntry {
            kind: LinkKind::Blocked,
            uri: Some("javascript:alert(1)".into()),
            ..LinkEntry::default()
        };
        assert_eq!(
            link_label(&blocked),
            "Link that runs a script (not run, for safety)"
        );
        assert_eq!(blocked_link_raw(&blocked), "Blocked link (javascript:)");
        let summary = link_summary(&[page, blocked]);
        assert_eq!(
            serde_json::to_value(&summary).unwrap(),
            json!([{"kind": "page", "page": 2}, {"kind": "blocked", "uri": "javascript:alert(1)"}])
        );
    }

    #[test]
    fn hit_rect_is_the_quad_union() {
        let hit = SearchHit {
            page: 0,
            quads: vec![
                vec![1.0, 2.0, 5.0, 2.0, 1.0, 9.0, 5.0, 9.0],
                vec![0.0; 4],
                vec![6.0, 1.0, 8.0, 1.0, 6.0, 3.0, 8.0, 3.0],
            ],
        };
        assert_eq!(hit_rect(&hit), Some(vec![1.0, 1.0, 8.0, 9.0]));
        assert_eq!(hit_rect(&SearchHit::default()), None);
    }

    #[test]
    fn outline_rows_nest_lazily() {
        let entry = |id: i64, depth: i64| OutlineEntry {
            id,
            title: format!("b{id}"),
            depth,
            page: Some(0),
            uri: None,
        };
        // A child deeper than any list so far sits in the deepest list there is.
        let outline = vec![
            entry(0, 0),
            entry(1, 2),
            entry(2, 0),
            entry(3, 1),
            entry(4, 2),
        ];
        assert_eq!(outline_depths(&outline), vec![0, 0, 0, 1, 2]);
    }

    #[test]
    fn current_hit_is_on_the_wanted_page_only() {
        let mut session = Session::new();
        session.search_hits = vec![SearchHit {
            page: 1,
            quads: vec![],
        }];
        session.search_index = 0;
        assert!(session.current_hit().is_none());
        session.wanted_page = 1;
        assert!(session.current_hit().is_some());
    }
}
