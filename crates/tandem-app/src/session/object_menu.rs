//! The object context menu (P6 U5 §2.4) and object editing on the page (R:3992-4618): the
//! menu's items and keys, focus restore after a write, gesture work, object targets (move,
//! resize, ⌥-duplicate, group drag and resize, rotate/flip, crop) and redaction mark targets.
#![allow(dead_code)]

use super::Session;
use super::content::ObjectEntry;
use super::direct::{
    FIELD_HANDLES, Place, RectCommit, RectGesture, has_op, map_rect, rect_distance, shift_rect,
    turn_rect, union_rect,
};
use super::dom_ids as ids;
use super::hit::{Focus, HitNode, KeyEv, NodeKey, NodeKind, Part, PtrEvent, cmd_key};
use super::members::is_object_member;
use super::nav::{GestureOptions, gesture_rect, pdf_box, round4, view_box};
use super::types::*;
use crate::engine::Refusal;
use serde_json::{Map, Value, json};
use tandem_protocol::engine::{EditObjectAction, EditTargetKind};

/// The menu's items in DOM order (`objectMenuButtons`' keys).
pub const MENU_KEYS: [&str; 12] = [
    "cut",
    "copy",
    "paste",
    "replace",
    "duplicate",
    "rotateCw",
    "rotateCcw",
    "flipH",
    "flipV",
    "crop",
    "delete",
    "redactPage",
];

/// `Record<ObjectMenuItem, boolean>`, in DOM order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuItems(pub Vec<(&'static str, bool)>);

impl MenuItems {
    fn new() -> Self {
        MenuItems(MENU_KEYS.iter().map(|k| (*k, false)).collect())
    }
    pub fn get(&self, name: &str) -> bool {
        self.0.iter().any(|(k, v)| *k == name && *v)
    }
    fn set(&mut self, name: &str, on: bool) {
        if let Some(entry) = self.0.iter_mut().find(|(k, _)| *k == name) {
            entry.1 = on;
        }
    }
    /// `{cut, copy, …, redactPage: bool}` for STATE.
    pub fn to_json(&self) -> Value {
        Value::Object(
            self.0
                .iter()
                .map(|(k, v)| (k.to_string(), Value::Bool(*v)))
                .collect(),
        )
    }
}

/// A `json!({...})` object as the request map `apply_object_edit` takes.
pub(crate) fn as_map(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// The menu button id of an item key.
pub fn menu_button(name: &str) -> &'static str {
    ids::OBJECT_MENU_BUTTONS
        .iter()
        .find(|(k, _)| *k == name)
        .map_or("", |(_, id)| id)
}

/// R:4217 `nudgeRect()`: ⌥-arrow 1 pt, ⌥⇧ 10 pt, ⌥⌘←/→ width 1 pt, in y-down page pt.
pub fn nudge_rect(
    rect: &[f64],
    key: &str,
    shift: bool,
    cmd: bool,
    min: f64,
    size: &PageSize,
) -> Vec<f64> {
    let amount = if !cmd && shift { 10.0 } else { 1.0 };
    let dx = match key {
        "ArrowLeft" => -amount,
        "ArrowRight" => amount,
        _ => 0.0,
    };
    let dy = match key {
        "ArrowUp" => -amount,
        "ArrowDown" => amount,
        _ => 0.0,
    };
    gesture_rect(
        rect,
        if cmd { "e" } else { "grip" },
        dx,
        if cmd { 0.0 } else { dy },
        &GestureOptions {
            min,
            lock_aspect: false,
            page: [size.width, size.height],
            within: None,
        },
    )
}

/// R:4071 `menuDuplicateRect()`: +10 pt right and down, −10 on an axis with no room, kept on the page.
pub fn menu_duplicate_rect(union: &[f64], page: [f64; 2]) -> Vec<f64> {
    let dx = if union[2] + 10.0 <= page[0] {
        10.0
    } else {
        -10.0
    };
    let dy = if union[3] + 10.0 <= page[1] {
        10.0
    } else {
        -10.0
    };
    gesture_rect(
        union,
        "grip",
        dx,
        dy,
        &GestureOptions {
            min: 1.0,
            lock_aspect: false,
            page,
            within: None,
        },
    )
}

/// R:4497 `wrapsPageContent()`: a Form contains (±0.5 pt) an item the page itself draws.
pub fn wraps_page_content(form: &EditTarget, scan: &ContentScan) -> bool {
    let Some(bx) = form.placement.as_ref().map(|p| &p.r#box) else {
        return false;
    };
    scan.items.iter().any(|other| {
        if other.target_id == form.target_id
            || other
                .target_id
                .split('.')
                .nth(1)
                .unwrap_or("/")
                .contains('/')
        {
            return false;
        }
        let bounds = other
            .placement
            .as_ref()
            .map(|p| p.r#box.clone())
            .or_else(|| other.bounds.clone());
        bounds.is_some_and(|b| {
            b.len() == 4
                && b[0] >= bx[0] - 0.5
                && b[1] >= bx[1] - 0.5
                && b[2] <= bx[2] + 0.5
                && b[3] <= bx[3] + 0.5
        })
    })
}

/// R:4467 `viewFull()`: the uncropped view box (the visible box when uncropped).
pub fn view_full(item: &EditTarget, m: &[f64]) -> Vec<f64> {
    let placement = item.placement.clone().unwrap_or_default();
    view_box(placement.full.as_ref().unwrap_or(&placement.r#box), m)
}

impl Session {
    // ------------------------------------------------------------------ the menu

    /// R:4001 `objectMenuItems()`.
    pub fn object_menu_items(&mut self) -> MenuItems {
        // P6 U7: the page menu offers Paste only; a selection with a non-object member offers Cut/Copy/Paste/Delete.
        if self.object_menu_kind == ObjectMenuKind::Page {
            let mut items = self.object_items_for(None);
            let redact = self.ui_mode == UiMode::Redact && self.can_redact_draw();
            items.set("redactPage", redact);
            return items;
        }
        if !self.selection_mixed() {
            let targets = self.selection_targets();
            return self.object_items_for(targets.as_deref());
        }
        let copy = !self.direct_selection.is_empty() && self.permitted("copy");
        let members = self.direct_selection.clone();
        let remove = !members.is_empty() && members.iter().all(|m| self.member_deletable(m));
        let mut items = self.object_items_for(None);
        items.set("cut", copy && remove);
        items.set("copy", copy);
        items.set("delete", remove);
        items
    }

    /// R:4009 `objectItemsFor()`: rotate/flip need `transform` on every member (rotate also
    /// needs the turned union to fit the page); crop needs one member listing `crop`.
    pub fn object_items_for(&mut self, targets: Option<&[ObjectTarget]>) -> MenuItems {
        let list = targets.unwrap_or(&[]);
        let edit = targets.is_some() && !list.is_empty() && self.can_edit();
        let size = self.page_size(self.wanted_page);
        let every = |op: EditObjectAction| edit && list.iter().all(|t| has_op(&t.item, op));
        let union = if list.is_empty() {
            None
        } else {
            Some(union_rect(
                &list.iter().map(|t| t.rect.clone()).collect::<Vec<_>>(),
            ))
        };
        let turns = match (&union, &size) {
            (Some(u), Some(s)) => u[3] - u[1] <= s.width + 1e-6 && u[2] - u[0] <= s.height + 1e-6,
            _ => false,
        };
        let copy = !list.is_empty() && self.permitted("copy");
        let paste = self.clipboard_info.is_some() && self.editable();
        let mut items = MenuItems::new();
        items.set("cut", copy && every(EditObjectAction::Delete));
        items.set("copy", copy);
        items.set("paste", paste);
        items.set(
            "replace",
            edit && list.len() == 1
                && list[0].item.kind == EditTargetKind::Image
                && has_op(&list[0].item, EditObjectAction::Replace),
        );
        items.set("duplicate", every(EditObjectAction::Duplicate));
        items.set("rotateCw", every(EditObjectAction::Transform) && turns);
        items.set("rotateCcw", every(EditObjectAction::Transform) && turns);
        items.set("flipH", every(EditObjectAction::Transform));
        items.set("flipV", every(EditObjectAction::Transform));
        items.set(
            "crop",
            edit && list.len() == 1 && has_op(&list[0].item, EditObjectAction::Crop),
        );
        items.set("delete", every(EditObjectAction::Delete));
        items
    }

    /// R:4031 `syncObjectMenu()`.
    pub fn sync_object_menu(&mut self) {
        let items = self.object_menu_items();
        for (name, on) in &items.0 {
            let id = menu_button(name);
            self.dom.set_disabled(id, !on);
            self.dom
                .set_attr(id, "aria-disabled", if *on { "false" } else { "true" });
        }
        let crop = self.direct_crop.is_some();
        self.dom.set_attr(
            menu_button("crop"),
            "aria-checked",
            if crop { "true" } else { "false" },
        );
        self.dom.set_checked(menu_button("crop"), crop);
        let shown = self.object_menu_kind == ObjectMenuKind::Page && self.ui_mode == UiMode::Redact;
        self.dom.set_hidden(menu_button("redactPage"), !shown);
    }

    /// R:4039 `objectMenuState()`: STATE `direct.menu`.
    pub fn object_menu_state(&self) -> Option<Value> {
        if self.dom.hidden(ids::OBJECT_MENU) {
            return None;
        }
        let items = Value::Object(
            MENU_KEYS
                .iter()
                .map(|k| {
                    (
                        k.to_string(),
                        Value::Bool(!self.dom.disabled(menu_button(k))),
                    )
                })
                .collect(),
        );
        if self.object_menu_kind == ObjectMenuKind::Page {
            return Some(json!({"name": "", "kind": "page", "count": 0, "items": items}));
        }
        let anchor = self.direct_selection.last()?;
        Some(
            json!({"name": anchor.name, "kind": anchor.kind, "count": self.direct_selection.len(), "items": items}),
        )
    }

    /// R:4047 `openObjectMenu()`: at a client point, focus on the first enabled U5/U6 item.
    pub fn open_object_menu(
        &mut self,
        client_x: f64,
        client_y: f64,
        kind: ObjectMenuKind,
        point: Option<Vec<f64>>,
    ) {
        if self.doc.is_none()
            || (kind == ObjectMenuKind::Selection && self.direct_selection.is_empty())
        {
            return;
        }
        self.object_menu_kind = kind;
        self.object_menu_point = point;
        self.object_menu_at = Some(ObjectMenuAt {
            page: self.wanted_page,
            zoom: self.shown_zoom,
        });
        self.sync_object_menu();
        self.dom.set_hidden(ids::OBJECT_MENU, false);
        self.lane2.menu_at = [client_x, client_y];
        let enabled = |s: &Session, k: &str| !s.dom.disabled(menu_button(k));
        let clip = ["cut", "copy", "paste"];
        let first = MENU_KEYS
            .iter()
            .find(|k| !clip.contains(k) && enabled(self, k))
            .or_else(|| MENU_KEYS.iter().find(|k| enabled(self, k)));
        match first {
            Some(k) => self.set_focus(Focus::MenuItem(k)),
            None => self.set_focus(Focus::Menu),
        };
    }

    /// R:4064 `closeObjectMenu(refocus)`: every close but an outside press puts focus back on the anchor.
    pub fn close_object_menu(&mut self, refocus: bool) {
        if self.dom.hidden(ids::OBJECT_MENU) {
            return;
        }
        self.dom.set_hidden(ids::OBJECT_MENU, true);
        self.object_menu_at = None;
        if matches!(self.lane2.focus, Focus::Menu | Focus::MenuItem(_)) {
            self.lane2.focus = Focus::Body;
        }
        if !refocus {
            return;
        }
        let anchor = self.direct_selection.last().cloned();
        if self.object_menu_kind == ObjectMenuKind::Page {
            self.focus_page();
        } else if let Some(anchor) = anchor
            && let Some(index) = self.member_node(&anchor)
        {
            let key = self.lane2.hit.nodes[index].key();
            self.focus_node(&key);
        }
    }

    /// R:4076 `objectMenuWork()`: Replace, duplicate, rotate/flip or delete the selection.
    pub fn object_menu_work(
        &mut self,
        which: &str,
        point: Option<Vec<f64>>,
    ) -> Result<(), Refusal> {
        // P6 U7: the clipboard items (and the page menu) never need object targets; a mixed selection only deletes.
        if matches!(which, "cut" | "copy" | "paste") {
            self.clipboard_work(which, point)?;
            return Ok(());
        }
        if which == "redactPage" {
            let page = self.object_menu_at.map_or(self.wanted_page, |at| at.page);
            self.add_redact_page(page as usize);
            return Ok(());
        }
        if self.selection_mixed() {
            if which == "delete" {
                self.delete_selection();
            }
            return Ok(());
        }
        let (Some(targets), Some(size)) =
            (self.selection_targets(), self.page_size(self.wanted_page))
        else {
            return Ok(());
        };
        if matches!(which, "rotateCw" | "rotateCcw" | "flipH" | "flipV") {
            self.transform_objects(&targets, which, None)?;
            return Ok(());
        }
        if which == "crop" {
            return Ok(());
        }
        if which == "delete" && targets.len() >= 2 {
            self.group_delete();
            return Ok(());
        }
        let union = union_rect(&targets.iter().map(|t| t.rect.clone()).collect::<Vec<_>>());
        let moved = menu_duplicate_rect(&union, [size.width, size.height]);
        if which == "duplicate" && targets.len() >= 2 {
            self.commit_object_group(
                &targets,
                moved[0] - union[0],
                moved[1] - union[1],
                "duplicate",
            )?;
            return Ok(());
        }
        let (item, rect) = (targets[0].item.clone(), targets[0].rect.clone());
        let page = self.wanted_page;
        if which == "duplicate" {
            self.commit_object_duplicate(
                &item,
                page,
                &rect,
                &shift_rect(&rect, moved[0] - union[0], moved[1] - union[1]),
            )?;
            return Ok(());
        }
        self.object_action(&item, page, &rect, which)?;
        Ok(())
    }

    /// R:4092 `activateObjectMenu()`: the menu closes (focus back to the anchor), then the item runs.
    pub fn activate_object_menu(&mut self, which: &str) {
        if self.dom.hidden(ids::OBJECT_MENU) || self.dom.disabled(menu_button(which)) {
            return;
        }
        let point = if self.object_menu_kind == ObjectMenuKind::Page {
            self.object_menu_point.clone()
        } else {
            None
        };
        self.close_object_menu(true);
        // §9.15: Crop toggles a mode; it is not gesture work.
        if which == "crop" {
            self.toggle_crop();
            return;
        }
        if !self.lane2.gesture_work {
            let which = which.to_string();
            self.run_gesture_work(move |s| s.object_menu_work(&which, point));
        }
    }

    /// R:4107 the menu's keydown: ↑/↓ cycle the enabled items, Home/End, Enter/Space activate, Esc/Tab close.
    pub(crate) fn on_object_menu_keydown(&mut self, ev: &mut KeyEv) {
        let enabled: Vec<&'static str> = MENU_KEYS
            .iter()
            .copied()
            .filter(|k| !self.dom.disabled(menu_button(k)))
            .collect();
        let at = match &self.lane2.focus {
            Focus::MenuItem(name) => enabled
                .iter()
                .position(|k| k == name)
                .map_or(-1, |i| i as i64),
            _ => -1,
        };
        let len = enabled.len() as i64;
        match ev.key.as_str() {
            "ArrowDown" | "ArrowUp" => {
                if len > 0 {
                    let step = if ev.key == "ArrowDown" {
                        1
                    } else {
                        len - if at < 0 { 0 } else { 1 }
                    };
                    let next = enabled[((at + step) % len) as usize];
                    self.set_focus(Focus::MenuItem(next));
                }
            }
            "Home" | "End" => {
                let pick = if ev.key == "Home" {
                    enabled.first()
                } else {
                    enabled.last()
                };
                if let Some(k) = pick {
                    self.set_focus(Focus::MenuItem(k));
                }
            }
            "Enter" | " " => {
                if let Focus::MenuItem(name) = self.lane2.focus {
                    self.activate_object_menu(name);
                }
            }
            "Escape" | "Tab" => self.close_object_menu(true),
            _ => return,
        }
        ev.prevent();
        ev.stopped = true;
    }

    /// R:4118: the menu closes on any press outside it (capture phase, no refocus).
    pub(crate) fn on_document_pointerdown_menu(&mut self, ev: &PtrEvent) {
        if self.dom.hidden(ids::OBJECT_MENU) || ev.target.in_menu() {
            return;
        }
        self.close_object_menu(false);
        self.object_menu_dismiss = true;
    }

    /// R:4122: a viewport scroll closes the menu (refocus).
    pub fn on_viewport_scroll(&mut self) {
        self.close_object_menu(true);
    }

    // ------------------------------------------------------------------ focus after a write

    /// R:4130 `syncNoteCurrent()`: the Annotations row of the focused mark is the current one.
    pub fn sync_note_current(&mut self) {
        self.lane2.note_row_current = self.direct_focused_mark.map(|m| m.annot_id);
    }

    /// R:4136 `dropDirectFocus()`.
    pub fn drop_direct_focus(&mut self) {
        self.clear_direct_focus();
        self.mark_selected(None);
        self.sync_note_current();
        self.sync_link_current();
    }

    /// R:4137 `restoreDirectFocus(refocus)`: after a repaint, the focused object (nearest of the
    /// same kind + name), mark or link and the selection's members are found again.
    pub fn restore_direct_focus(&mut self, refocus: bool) {
        // P6 U7 §10.39: a selection holding a run, field, link or mark re-resolves each member by its own layer.
        if self.selection_mixed() {
            let link = self.direct_focused_link;
            self.resolve_members();
            if self.direct_focused_link.is_none()
                && self
                    .direct_active
                    .as_ref()
                    .is_some_and(|a| a.kind == DirectKind::Link)
            {
                self.direct_focused_link = link;
            }
            let target = self
                .direct_selection
                .last()
                .cloned()
                .and_then(|a| self.member_node(&a))
                .map(|i| self.lane2.hit.nodes[i].key());
            if self.direct_selection.is_empty() {
                self.mark_selected(None);
            } else {
                self.mark_selection();
            }
            if let Some(key) = target.filter(|_| refocus && self.direct_active.is_none()) {
                self.focus_node(&key);
            }
            self.sync_note_current();
            self.sync_link_current();
            return;
        }
        let mut target: Option<NodeKey> = None;
        if let Some(focus) = self.direct_focused_object.clone() {
            let scan = self.content_scan.clone();
            if self.ui_mode != UiMode::Edit {
                self.direct_focused_object = None;
            } else if let Some(scan) =
                scan.filter(|s| s.page == self.wanted_page && s.transform.is_some())
            {
                // Nearest painted target of the same kind + name to the last rect (ids change on a rewrite);
                // a replace renames the XObject (Im2 → Im2r1): the same kind in the same place (≤ 2 px) keeps focus.
                let transform = scan.transform.clone().unwrap();
                let mut used: Vec<usize> = Vec::new();
                let resolve = |s: &Session,
                               member: &ObjectFocus,
                               rename: bool,
                               used: &mut Vec<usize>|
                 -> Option<(usize, ObjectFocus)> {
                    let mut best = f64::INFINITY;
                    let mut found: Option<usize> = None;
                    let mut renamed: Option<EditTarget> = None;
                    let passes: &[bool] = if rename { &[true, false] } else { &[true] };
                    for &pass in passes {
                        for (index, node) in s.lane2.hit.nodes.iter().enumerate() {
                            let NodeKind::Object { item: painted, .. } = &node.kind else {
                                continue;
                            };
                            if used.contains(&index) {
                                continue;
                            }
                            let Some(item) =
                                scan.items.iter().find(|v| v.target_id == painted.target_id)
                            else {
                                continue;
                            };
                            let Some(placement) = &item.placement else {
                                continue;
                            };
                            if item.kind.as_str() != member.kind
                                || (item.object_name.clone().unwrap_or_default() == member.name)
                                    != pass
                            {
                                continue;
                            }
                            let distance = rect_distance(
                                &view_box(&placement.r#box, &transform),
                                &member.rect,
                            );
                            if distance < best && (pass || distance <= 2.0) {
                                best = distance;
                                found = Some(index);
                                renamed = if pass { None } else { Some(item.clone()) };
                            }
                        }
                        if found.is_some() {
                            break;
                        }
                    }
                    let index = found?;
                    used.push(index);
                    let target_id = s.lane2.hit.nodes[index]
                        .target_id()
                        .unwrap_or(&member.target_id)
                        .to_string();
                    let name = match &renamed {
                        Some(r) => r.object_name.clone().unwrap_or_default(),
                        None => member.name.clone(),
                    };
                    Some((
                        index,
                        ObjectFocus {
                            name,
                            target_id,
                            ..member.clone()
                        },
                    ))
                };
                let anchor = resolve(self, &focus, true, &mut used);
                target = anchor.as_ref().map(|(i, _)| self.lane2.hit.nodes[*i].key());
                self.direct_focused_object = anchor.map(|(_, m)| m);
                let selection = self.direct_selection.clone();
                let others: Vec<ObjectFocus> = selection
                    .iter()
                    .take(selection.len().saturating_sub(1))
                    .filter_map(|m| resolve(self, m, false, &mut used).map(|(_, m)| m))
                    .collect();
                self.direct_selection = match &self.direct_focused_object {
                    Some(anchor) => {
                        let mut next = others;
                        next.push(anchor.clone());
                        next
                    }
                    None => Vec::new(),
                };
            }
        }
        if self.direct_focused_object.is_none() {
            self.direct_selection.clear();
        }
        if let Some(mark) = self.direct_focused_mark {
            target = self
                .lane2
                .hit
                .find(&NodeKey::Mark(mark.annot_id))
                .map(|_| NodeKey::Mark(mark.annot_id));
            if target.is_none() {
                self.direct_focused_mark = None;
            }
        }
        if let Some(link) = self.direct_focused_link {
            let found = self.lane2.hit.nodes.iter().find(|n| matches!(&n.kind, NodeKind::Link { item, .. } if super::hit::link_annot_id(item) == link.annot_id)).map(HitNode::key);
            target = found.clone();
            // An open link editor keeps its link focused while the target is repainted under it.
            if found.is_none()
                && self
                    .direct_active
                    .as_ref()
                    .is_none_or(|a| a.kind != DirectKind::Link)
            {
                self.direct_focused_link = None;
            }
        }
        if self.direct_selection.is_empty() {
            self.mark_selected(target.as_ref());
        } else {
            self.mark_selection();
        }
        // P6 U6 §9.18: crop mode survives a repaint on the re-resolved anchor.
        if self.direct_crop.is_some() {
            let anchor = self
                .direct_focused_object
                .clone()
                .filter(|_| self.direct_selection.len() == 1);
            let node = anchor.as_ref().and_then(|a| self.object_node(&a.target_id));
            let item = node.and_then(|_| {
                self.content_scan.as_ref().and_then(|s| {
                    s.items
                        .iter()
                        .find(|v| Some(&v.target_id) == anchor.as_ref().map(|a| &a.target_id))
                        .cloned()
                })
            });
            let transform = self.content_scan.as_ref().and_then(|s| s.transform.clone());
            match (node, item, transform) {
                (Some(_), Some(item), Some(m)) if item.placement.is_some() => {
                    self.direct_crop = Some(DirectCrop {
                        target_id: item.target_id.clone(),
                        full: view_full(&item, &m),
                    });
                    self.paint_crop(Some(&NodeKey::Object(item.target_id.clone())));
                }
                _ => self.leave_crop(),
            }
        }
        // §9.6: a repaint while an editor is open must not steal its focus (the blur would commit it).
        if let Some(key) = target.filter(|_| refocus && self.direct_active.is_none()) {
            self.focus_node(&key);
        }
        // P6 U6 review R4 (§9.26): the Inspector follows the re-resolved anchor.
        if let Some(anchor) = self.direct_focused_object.clone()
            && self.content_object_target.as_ref().map(|t| &t.target_id) != Some(&anchor.target_id)
        {
            self.sync_object_inspector(&anchor.target_id);
        }
        self.sync_note_current();
        self.sync_link_current();
    }

    /// R:4208 `runGestureWork()`: one keyboard/menu/double-click commit as the pending gesture
    /// work; a refusal it throws is shown.
    pub fn run_gesture_work<F: FnOnce(&mut Session) -> Result<(), Refusal>>(&mut self, work: F) {
        self.clear_error();
        let was = std::mem::replace(&mut self.lane2.gesture_work, true);
        if let Err(err) = work(self) {
            self.show_error(&err.message);
        }
        self.lane2.gesture_work = was;
    }

    // ------------------------------------------------------------------ object targets

    /// R:4222 `paintObjectTarget()`: an image or Form placement's target.
    pub fn paint_object_target(&mut self, item: &EditTarget, scan: &ContentScan) -> HitNode {
        let m = scan.transform.clone().unwrap_or_default();
        let rect = view_box(
            &item
                .placement
                .as_ref()
                .map(|p| p.r#box.clone())
                .unwrap_or_default(),
            &m,
        );
        let page = scan.page;
        // P6 U6 §2.5/§9.24: a form around page content takes presses on a 6 px border ring only.
        let wrapping = item.kind == EditTargetKind::Form && wraps_page_content(item, scan);
        let can = self.can_edit();
        let mut node = HitNode::new(
            NodeKind::Object {
                item: item.clone(),
                page,
                rect: rect.clone(),
                wrapping,
                can,
            },
            &rect,
            self.shown_zoom,
        );
        node.classes.push("object-target");
        node.classes.push(if item.kind == EditTargetKind::Image {
            "image"
        } else {
            "form"
        });
        if wrapping {
            node.classes.push("wrapping");
        }
        node.label = super::content::object_row_label(item);
        node.title = super::content::object_row_state(item);
        if can {
            node.handles = FIELD_HANDLES.to_vec();
        }
        node
    }

    /// The object node's captured data.
    fn object_data(&self, index: usize) -> Option<(EditTarget, i64, Vec<f64>, bool, NodeKey)> {
        let node = self.lane2.hit.nodes.get(index)?;
        match &node.kind {
            NodeKind::Object {
                item,
                page,
                rect,
                can,
                ..
            } => Some((item.clone(), *page, rect.clone(), *can, node.key())),
            _ => None,
        }
    }

    fn is_selected_object(&self, target_id: &str) -> bool {
        self.direct_selection
            .iter()
            .any(|m| m.target_id == target_id)
    }

    fn cropping(&self, target_id: &str) -> bool {
        self.direct_crop
            .as_ref()
            .is_some_and(|c| c.target_id == target_id)
    }

    /// The object target's `start(event, handle)`.
    fn object_start(
        &mut self,
        item: &EditTarget,
        page: i64,
        rect: &[f64],
        ev: &mut PtrEvent,
        handle: &str,
        key: NodeKey,
    ) {
        let can = self.can_edit();
        let snap = Some(self.snap_others(std::slice::from_ref(&item.target_id)));
        // §2.3/§9.18: in crop mode the handles and the grip move the crop window inside the uncropped box (∩ page).
        if self.cropping(&item.target_id)
            && let Some(crop) = self.direct_crop.clone()
        {
            let Some(size) = self.page_size(page) else {
                return;
            };
            let full = crop.full;
            let within = vec![
                full[0].max(0.0),
                full[1].max(0.0),
                size.width.min(full[2]),
                size.height.min(full[3]),
            ];
            let options = RectGesture {
                rect: rect.to_vec(),
                id: TargetRef::Text(item.target_id.clone()),
                handle: handle.into(),
                min: 1.0,
                can,
                within: Some(within),
                crop: true,
                snap,
                commit: RectCommit::ObjectCrop {
                    item: item.clone(),
                    page,
                    initial: rect.to_vec(),
                },
                ..RectGesture::default()
            };
            self.start_rect_gesture(ev, Some(key), options);
            return;
        }
        let options = RectGesture {
            rect: rect.to_vec(),
            id: TargetRef::Text(item.target_id.clone()),
            handle: handle.into(),
            min: 1.0,
            lock_aspect: item.kind == EditTargetKind::Image,
            can,
            snap,
            commit: RectCommit::ObjectRect {
                item: item.clone(),
                page,
                initial: rect.to_vec(),
            },
            ..RectGesture::default()
        };
        self.start_rect_gesture(ev, Some(key), options);
    }

    /// R:4237-4261 the object target's handle and body pointerdown.
    pub(crate) fn object_pointerdown(&mut self, index: usize, part: Part, ev: &mut PtrEvent) {
        let Some((item, page, rect, can, key)) = self.object_data(index) else {
            return;
        };
        if !can {
            return;
        }
        if let Part::Handle(handle) = part {
            self.object_start(&item, page, &rect, ev, handle, key.clone());
            if ev.stopped {
                return;
            }
        }
        // §9.27: a Control-click is the macOS right-click; it opens the menu, never a drag.
        if ev.button != 0 || ev.ctrl || ev.target.on_handle() {
            return;
        }
        self.lane2.press_at = Some((key.clone(), [ev.client_x, ev.client_y]));
        // §9.18: a body press on the cropping object moves the crop window.
        if self.cropping(&item.target_id) && !ev.shift {
            self.object_start(&item, page, &rect, ev, "grip", key);
            return;
        }
        // §9.23: a plain press on a non-member of a group makes it the single selection first.
        if self.direct_selection.len() >= 2
            && !self.is_selected_object(&item.target_id)
            && !ev.shift
        {
            self.focus_object(&item, &rect, &key);
        }
        if self.direct_selection.len() >= 2 && self.is_selected_object(&item.target_id) {
            if self.selection_mixed() {
                self.start_selection_gesture(ev, key);
            } else {
                self.start_group_gesture(ev, ev.alt, key);
            }
            return;
        }
        if !ev.alt {
            self.object_start(&item, page, &rect, ev, "grip", key);
            return;
        }
        // §2.3: ⌥-drag duplicates; the original keeps an outline where it was.
        self.lane2.ghosts = vec![rect.clone()];
        let can = self.can_edit();
        let snap = Some(self.snap_others(std::slice::from_ref(&item.target_id)));
        let options = RectGesture {
            rect: rect.clone(),
            id: TargetRef::Text(item.target_id.clone()),
            handle: "grip".into(),
            min: 1.0,
            can,
            duplicate: true,
            ghosts: true,
            snap,
            commit: RectCommit::ObjectDuplicate {
                item: item.clone(),
                page,
                initial: rect.clone(),
            },
            ..RectGesture::default()
        };
        self.start_rect_gesture(ev, Some(key), options);
    }

    /// R:4267 the object target's click: a click that stayed put selects (Shift toggles).
    pub(crate) fn object_click(&mut self, index: usize, part: Part, ev: &mut PtrEvent) {
        let Some((item, _, rect, _, key)) = self.object_data(index) else {
            return;
        };
        ev.prevented = true;
        let press = self
            .lane2
            .press_at
            .take()
            .filter(|(k, _)| *k == key)
            .map(|(_, p)| p);
        if matches!(part, Part::Handle(_)) || ev.ctrl {
            return;
        }
        // A real drag ends in a click on the moved button; only a click that stayed put selects.
        if let Some(p) = press
            && (ev.client_x - p[0]).hypot(ev.client_y - p[1]) >= 3.0
        {
            return;
        }
        if ev.shift {
            self.toggle_object(&item, &rect, &key);
        } else {
            self.focus_object(&item, &rect, &key);
        }
    }

    /// R:4275 double-click an image with `replace` → Replace.
    pub(crate) fn object_dblclick(&mut self, index: usize, ev: &mut PtrEvent) {
        let Some((item, page, rect, _, _)) = self.object_data(index) else {
            return;
        };
        ev.prevented = true;
        if self.direct_selection.len() < 2
            && self.direct_crop.is_none()
            && !self.lane2.gesture_work
            && self.can_edit()
            && item.kind == EditTargetKind::Image
            && has_op(&item, EditObjectAction::Replace)
        {
            self.run_gesture_work(move |s| {
                s.object_action(&item, page, &rect, "replace").map(|_| ())
            });
        }
    }

    /// R:4262 the object target's contextmenu: select it unless a member, then the menu at the pointer.
    pub(crate) fn object_contextmenu(&mut self, index: usize, ev: &mut PtrEvent) {
        let Some((item, _, rect, _, key)) = self.object_data(index) else {
            return;
        };
        ev.prevented = true;
        ev.stopped = true;
        if !self.is_selected_object(&item.target_id) {
            self.focus_object(&item, &rect, &key);
        }
        self.open_object_menu(ev.client_x, ev.client_y, ObjectMenuKind::Selection, None);
    }

    /// R:4276 the object target's keys.
    pub(crate) fn object_keydown(&mut self, index: usize, ev: &mut KeyEv) {
        let Some((item, page, rect, _, key)) = self.object_data(index) else {
            return;
        };
        // §9.18: crop mode owns Escape/Enter; a gesture's Escape is the window's.
        if ev.key == "Escape" && self.direct_gesture.is_some() {
            return;
        }
        if self.cropping(&item.target_id) {
            if ev.key == "Escape" || ev.key == "Enter" {
                ev.prevent();
                self.leave_crop();
                return;
            }
            if ev.is_arrow() || ev.key == "Delete" || ev.key == "Backspace" {
                ev.prevent();
                return;
            }
        }
        if ev.key == "Escape" {
            ev.prevent();
            self.drop_direct_focus();
            return;
        }
        if (ev.key == "F10" && ev.shift) || ev.key == "ContextMenu" {
            ev.prevent();
            if !self.is_selected_object(&item.target_id) {
                self.focus_object(&item, &rect, &key);
            }
            let css = self
                .lane2
                .hit
                .find(&key)
                .map(|i| self.lane2.hit.nodes[i].css)
                .unwrap_or_default();
            self.open_object_menu(css.left, css.top, ObjectMenuKind::Selection, None);
            return;
        }
        let size = self.page_size(page);
        let cmd = ev.cmd();
        type Work = Box<dyn FnOnce(&mut Session) -> Result<(), Refusal>>;
        let mut work: Option<Work> = None;
        if self.direct_selection.len() >= 2 && self.is_selected_object(&item.target_id) {
            // §2.2: a focused member moves or deletes the whole group; Enter and ⌥⌘ widths are single-only.
            if ev.is_arrow() && (ev.alt || (!ev.meta && !ev.ctrl)) {
                // P6 U6 §9.21: ⌥⌘←/→ resize the union's width by 1 pt (⌥⌘↑/↓ is Move Page).
                if cmd && (ev.key == "ArrowUp" || ev.key == "ArrowDown") {
                    return;
                }
                let targets = self.selection_targets();
                if let (Some(targets), Some(size)) = (targets, size)
                    && self.can_edit()
                {
                    let union =
                        union_rect(&targets.iter().map(|t| t.rect.clone()).collect::<Vec<_>>());
                    let min = if cmd {
                        super::direct::group_min(&targets, &union)
                    } else {
                        1.0
                    };
                    let next = nudge_rect(&union, &ev.key, ev.shift, cmd, min, &size);
                    work = Some(if cmd {
                        Box::new(move |s: &mut Session| {
                            s.commit_group_resize(&targets, &union, &next).map(|_| ())
                        })
                    } else {
                        Box::new(move |s: &mut Session| {
                            s.commit_object_group(
                                &targets,
                                next[0] - union[0],
                                next[1] - union[1],
                                "move",
                            )
                            .map(|_| ())
                        })
                    });
                }
            } else if ev.key == "Delete" || ev.key == "Backspace" {
                work = Some(Box::new(|s: &mut Session| {
                    s.group_delete();
                    Ok(())
                }));
            } else if ev.key != "Enter" && ev.key != " " {
                return;
            }
            ev.prevent();
            if let Some(work) = work
                && !self.lane2.gesture_work
            {
                self.run_gesture_work(work);
            }
            return;
        }
        if ev.is_arrow() && (ev.alt || (!ev.meta && !ev.ctrl)) {
            if cmd && (ev.key == "ArrowUp" || ev.key == "ArrowDown") {
                return;
            }
            if let Some(size) = size
                && self.can_edit()
            {
                let next = nudge_rect(&rect, &ev.key, ev.shift, cmd, 1.0, &size);
                let (item, rect) = (item.clone(), rect.clone());
                work = Some(Box::new(move |s: &mut Session| {
                    s.commit_object_rect(&item, page, &rect, &next).map(|_| ())
                }));
            }
        } else if ev.key == "Delete" || ev.key == "Backspace" {
            if self.can_edit() && has_op(&item, EditObjectAction::Delete) {
                let (item, rect) = (item.clone(), rect.clone());
                work = Some(Box::new(move |s: &mut Session| {
                    s.object_action(&item, page, &rect, "delete").map(|_| ())
                }));
            }
        } else if ev.key == "Enter" {
            if self.can_edit()
                && item.kind == EditTargetKind::Image
                && has_op(&item, EditObjectAction::Replace)
            {
                let (item, rect) = (item.clone(), rect.clone());
                work = Some(Box::new(move |s: &mut Session| {
                    s.object_action(&item, page, &rect, "replace").map(|_| ())
                }));
            }
        } else if ev.key != " " {
            return;
        }
        ev.prevent();
        if let Some(work) = work
            && !self.lane2.gesture_work
        {
            self.run_gesture_work(work);
        }
    }

    /// R:4320 `focusObject()`: click focuses the object and mirrors it in the Inspector.
    pub fn focus_object(&mut self, item: &EditTarget, rect: &[f64], key: &NodeKey) {
        let focus = ObjectFocus {
            kind: item.kind.as_str().into(),
            name: item.object_name.clone().unwrap_or_default(),
            target_id: item.target_id.clone(),
            rect: rect.to_vec(),
            text: None,
        };
        self.direct_focused_object = Some(focus.clone());
        self.direct_focused_mark = None;
        self.direct_selection = vec![focus];
        self.mark_selection();
        self.focus_node(key);
        self.sync_note_current();
        self.sync_object_inspector(&item.target_id);
    }

    /// R:4327 `syncObjectInspector()`: the Inspector row and form follow the anchor.
    pub fn sync_object_inspector(&mut self, target_id: &str) {
        let Some(scan) = self.content_scan.clone() else {
            return;
        };
        if let Some(index) = scan.items.iter().position(|v| v.target_id == target_id) {
            self.select_content_row(index as i64, false);
            self.open_content_object_form(scan.items[index].clone(), false);
        }
    }

    /// The focused-object record of a commit.
    fn object_focus_of(item: &EditTarget, rect: &[f64]) -> ObjectFocus {
        ObjectFocus {
            kind: item.kind.as_str().into(),
            name: item.object_name.clone().unwrap_or_default(),
            target_id: item.target_id.clone(),
            rect: rect.to_vec(),
            text: None,
        }
    }

    /// R:4332 `commitObjectRect()`: one `edit-object` move/resize with the view rect mapped back to PDF space.
    pub fn commit_object_rect(
        &mut self,
        item: &EditTarget,
        page: i64,
        initial: &[f64],
        rect: &[f64],
    ) -> Result<bool, Refusal> {
        let scan = self.content_scan.clone();
        let (Some(scan), Some(size)) = (
            scan.filter(|s| s.page == page && s.transform.is_some()),
            self.page_size(page),
        ) else {
            return Ok(false);
        };
        let Some(current) = self.resolve_object(&scan, item, initial) else {
            return Ok(false);
        };
        let Some(old) = current.placement.as_ref().map(|p| p.r#box.clone()) else {
            return Ok(false);
        };
        let bx = self.object_box(&scan, &size, rect)?;
        let same = |a: f64, b: f64| (a - b).abs() < 0.05;
        let action = if same(bx[2] - bx[0], old[2] - old[0]) && same(bx[3] - bx[1], old[3] - old[1])
        {
            "move"
        } else {
            "resize"
        };
        let (previous, selection) = (
            self.direct_focused_object.clone(),
            self.direct_selection.clone(),
        );
        let focus = Self::object_focus_of(&current, rect);
        self.direct_focused_object = Some(focus.clone());
        self.direct_focused_mark = None;
        self.direct_selection = vec![focus];
        if self.apply_object_edit(
            &scan,
            &current,
            action,
            as_map(json!({"box": bx})),
            Vec::new(),
        ) {
            return Ok(true);
        }
        self.direct_focused_object = previous;
        self.direct_selection = selection;
        Ok(false)
    }

    /// R:4349 `objectBox()`: a view rect → the PDF box an edit sends (snapped to the page clip within 0.01).
    pub fn object_box(
        &self,
        scan: &ContentScan,
        size: &PageSize,
        rect: &[f64],
    ) -> Result<Vec<f64>, Refusal> {
        let m = scan.transform.clone().unwrap_or_default();
        let clip = pdf_box(&[0.0, 0.0, size.width, size.height], &m)?;
        Ok(pdf_box(rect, &m)?
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                if (v - clip[i]).abs() <= 0.01 {
                    clip[i]
                } else {
                    v
                }
            })
            .collect())
    }

    /// R:4353 `selectCopies()`: after a duplicate the copies are the selection.
    fn select_copies(&mut self, before: &[String], wanted: &[ObjectFocus]) {
        let Some(scan) = self.content_scan.clone() else {
            return;
        };
        let Some(m) = scan.transform.clone() else {
            return;
        };
        let mut used: Vec<String> = Vec::new();
        let mut picks: Vec<ObjectFocus> = Vec::new();
        for want in wanted {
            let mut best: Option<(&EditTarget, f64)> = None;
            for value in scan.items.iter().filter(|v| {
                !before.contains(&v.target_id)
                    && !used.contains(&v.target_id)
                    && v.kind.as_str() == want.kind
                    && v.object_name.clone().unwrap_or_default() == want.name
                    && v.placement.is_some()
            }) {
                let d = rect_distance(
                    &view_box(&value.placement.as_ref().unwrap().r#box, &m),
                    &want.rect,
                );
                if best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((value, d));
                }
            }
            if let Some((copy, _)) = best {
                used.push(copy.target_id.clone());
                picks.push(ObjectFocus {
                    target_id: copy.target_id.clone(),
                    ..want.clone()
                });
            }
        }
        let Some(anchor) = picks.last().cloned() else {
            return;
        };
        self.direct_selection = picks;
        self.direct_focused_object = Some(anchor.clone());
        self.mark_selection();
        self.focus_node(&NodeKey::Object(anchor.target_id.clone()));
        self.sync_object_inspector(&anchor.target_id);
    }

    /// R:4370 `commitObjectDuplicate()`: one `edit-object {action:'duplicate', box}`; the copy becomes the selection.
    pub fn commit_object_duplicate(
        &mut self,
        item: &EditTarget,
        page: i64,
        initial: &[f64],
        rect: &[f64],
    ) -> Result<bool, Refusal> {
        let scan = self.content_scan.clone();
        let (Some(scan), Some(size)) = (
            scan.filter(|s| s.page == page && s.transform.is_some()),
            self.page_size(page),
        ) else {
            return Ok(false);
        };
        let Some(current) = self
            .resolve_object(&scan, item, initial)
            .filter(|c| c.placement.is_some())
        else {
            return Ok(false);
        };
        let before: Vec<String> = scan.items.iter().map(|v| v.target_id.clone()).collect();
        let (previous, selection) = (
            self.direct_focused_object.clone(),
            self.direct_selection.clone(),
        );
        let focus = Self::object_focus_of(&current, rect);
        self.direct_focused_object = Some(focus.clone());
        self.direct_focused_mark = None;
        self.direct_selection = vec![focus.clone()];
        let bx = self.object_box(&scan, &size, rect)?;
        if !self.apply_object_edit(
            &scan,
            &current,
            "duplicate",
            as_map(json!({"box": bx})),
            Vec::new(),
        ) {
            self.direct_focused_object = previous;
            self.direct_selection = selection;
            return Ok(false);
        }
        self.select_copies(&before, &[focus]);
        Ok(true)
    }

    /// R:4384 `startGroupGesture()`: a group drag on the union; every member follows.
    pub fn start_group_gesture(&mut self, ev: &mut PtrEvent, duplicate: bool, key: NodeKey) {
        let (Some(targets), Some(anchor)) =
            (self.selection_targets(), self.direct_focused_object.clone())
        else {
            return;
        };
        let union = union_rect(&targets.iter().map(|t| t.rect.clone()).collect::<Vec<_>>());
        let place = Place::Shift {
            members: targets
                .iter()
                .map(|t| (NodeKey::Object(t.item.target_id.clone()), t.rect.clone()))
                .collect(),
            union: union.clone(),
        };
        if duplicate {
            self.lane2.ghosts = targets.iter().map(|t| t.rect.clone()).collect();
        }
        let can = self.can_edit();
        let snap = Some(
            self.snap_others(
                &targets
                    .iter()
                    .map(|t| t.item.target_id.clone())
                    .collect::<Vec<_>>(),
            ),
        );
        let options = RectGesture {
            rect: union.clone(),
            id: TargetRef::Text(anchor.target_id.clone()),
            handle: "grip".into(),
            min: 1.0,
            can,
            place,
            duplicate,
            ghosts: duplicate,
            snap,
            commit: RectCommit::ObjectGroup {
                targets,
                union,
                duplicate,
            },
            ..RectGesture::default()
        };
        self.start_rect_gesture(ev, Some(key), options);
    }

    /// R:4394 `commitObjectGroup()`: every member moved or duplicated by the same view delta, as one `edit-objects`.
    pub fn commit_object_group(
        &mut self,
        targets: &[ObjectTarget],
        dx: f64,
        dy: f64,
        action: &str,
    ) -> Result<bool, Refusal> {
        let scan = self.content_scan.clone();
        let (Some(scan), Some(size)) = (
            scan.filter(|s| s.page == self.wanted_page && s.transform.is_some()),
            self.page_size(self.wanted_page),
        ) else {
            return Ok(false);
        };
        if dx == 0.0 && dy == 0.0 {
            return Ok(false);
        }
        let before: Vec<String> = scan.items.iter().map(|v| v.target_id.clone()).collect();
        let wanted: Vec<ObjectFocus> = targets
            .iter()
            .map(|t| ObjectFocus {
                target_id: t.item.target_id.clone(),
                rect: shift_rect(&t.rect, dx, dy),
                ..t.member.clone()
            })
            .collect();
        let (previous, selection) = (
            self.direct_focused_object.clone(),
            self.direct_selection.clone(),
        );
        // §9.13: every member's rect is its requested rect before the reply repaints.
        self.direct_selection = wanted.clone();
        self.direct_focused_object = wanted.last().cloned();
        self.direct_focused_mark = None;
        let mut entries = Vec::new();
        for (t, w) in targets.iter().zip(&wanted) {
            entries.push(ObjectEntry {
                item: t.item.clone(),
                r#box: Some(self.object_box(&scan, &size, &w.rect)?),
            });
        }
        if !self.apply_objects_edit(&scan, &entries, action, None) {
            self.direct_focused_object = previous;
            self.direct_selection = selection;
            self.mark_selection();
            return Ok(false);
        }
        if action == "duplicate" {
            self.select_copies(&before, &wanted);
        }
        Ok(true)
    }

    /// R:4407 `groupDelete()`: Delete on a group; the selection is cleared.
    pub fn group_delete(&mut self) -> bool {
        let (Some(scan), Some(targets)) = (self.content_scan.clone(), self.selection_targets())
        else {
            return false;
        };
        if !self.can_edit()
            || !targets
                .iter()
                .all(|t| has_op(&t.item, EditObjectAction::Delete))
        {
            return false;
        }
        let (previous, selection) = (
            self.direct_focused_object.clone(),
            self.direct_selection.clone(),
        );
        self.direct_focused_object = None;
        self.direct_selection.clear();
        if self.apply_objects_edit(
            &scan,
            &targets
                .iter()
                .map(|t| ObjectEntry {
                    item: t.item.clone(),
                    r#box: None,
                })
                .collect::<Vec<_>>(),
            "delete",
            None,
        ) {
            return true;
        }
        self.direct_focused_object = previous;
        self.direct_selection = selection;
        self.mark_selection();
        false
    }

    /// R:4418 `startGroupResize()`: a `.group-box` handle resizes the union by one affine.
    pub fn start_group_resize(&mut self, ev: &mut PtrEvent, handle: &str) {
        let (Some(targets), Some(anchor)) =
            (self.selection_targets(), self.direct_focused_object.clone())
        else {
            return;
        };
        if targets.len() < 2 {
            return;
        }
        let union = union_rect(&targets.iter().map(|t| t.rect.clone()).collect::<Vec<_>>());
        let focus = targets
            .iter()
            .find(|t| t.member.target_id == anchor.target_id)
            .or(targets.last())
            .map(|t| NodeKey::Object(t.item.target_id.clone()));
        let place = Place::Map {
            members: targets
                .iter()
                .map(|t| (NodeKey::Object(t.item.target_id.clone()), t.rect.clone()))
                .collect(),
            union: union.clone(),
        };
        let can = self.can_edit();
        let snap = Some(
            self.snap_others(
                &targets
                    .iter()
                    .map(|t| t.item.target_id.clone())
                    .collect::<Vec<_>>(),
            ),
        );
        let min = super::direct::group_min(&targets, &union);
        let lock = targets.iter().all(|t| t.item.kind == EditTargetKind::Image);
        let options = RectGesture {
            rect: union.clone(),
            id: TargetRef::Text(anchor.target_id.clone()),
            handle: handle.into(),
            min,
            lock_aspect: lock,
            can,
            place,
            focus,
            snap,
            commit: RectCommit::GroupResize { targets, union },
            ..RectGesture::default()
        };
        self.start_rect_gesture(ev, None, options);
    }

    /// R:4428 `commitGroupResize()`.
    pub fn commit_group_resize(
        &mut self,
        targets: &[ObjectTarget],
        union: &[f64],
        rect: &[f64],
    ) -> Result<bool, Refusal> {
        let scan = self.content_scan.clone();
        let (Some(scan), Some(size)) = (
            scan.filter(|s| s.page == self.wanted_page && s.transform.is_some()),
            self.page_size(self.wanted_page),
        ) else {
            return Ok(false);
        };
        if rect.iter().zip(union).all(|(a, b)| a == b) {
            return Ok(false);
        }
        let wanted: Vec<ObjectFocus> = targets
            .iter()
            .map(|t| ObjectFocus {
                target_id: t.item.target_id.clone(),
                rect: map_rect(&t.rect, union, rect),
                ..t.member.clone()
            })
            .collect();
        let (previous, selection) = (
            self.direct_focused_object.clone(),
            self.direct_selection.clone(),
        );
        self.direct_selection = wanted.clone();
        self.direct_focused_object = wanted.last().cloned();
        self.direct_focused_mark = None;
        let mut entries = Vec::new();
        for (t, w) in targets.iter().zip(&wanted) {
            entries.push(ObjectEntry {
                item: t.item.clone(),
                r#box: Some(self.object_box(&scan, &size, &w.rect)?),
            });
        }
        if !self.apply_objects_edit(&scan, &entries, "resize", None) {
            self.direct_focused_object = previous;
            self.direct_selection = selection;
            self.mark_selection();
            return Ok(false);
        }
        Ok(true)
    }

    /// R:4438 `transformObjects()`: rotate 90° or flip the targets about their union's centre,
    /// kept on the page; one request. `from` is the Inspector button that acted.
    pub fn transform_objects(
        &mut self,
        targets: &[ObjectTarget],
        which: &str,
        from: Option<&str>,
    ) -> Result<bool, Refusal> {
        let scan = self.content_scan.clone();
        let (Some(scan), Some(size)) = (
            scan.filter(|s| s.page == self.wanted_page && s.transform.is_some()),
            self.page_size(self.wanted_page),
        ) else {
            return Ok(false);
        };
        if targets.is_empty() {
            return Ok(false);
        }
        let m = scan.transform.clone().unwrap();
        let union = union_rect(&targets.iter().map(|t| t.rect.clone()).collect::<Vec<_>>());
        let (cx, cy) = ((union[0] + union[2]) / 2.0, (union[1] + union[3]) / 2.0);
        let turned: Vec<Vec<f64>> = targets
            .iter()
            .map(|t| turn_rect(&t.rect, which, cx, cy))
            .collect();
        let next = union_rect(&turned);
        let placed = gesture_rect(
            &next,
            "grip",
            0.0,
            0.0,
            &GestureOptions {
                min: 1.0,
                lock_aspect: false,
                page: [size.width, size.height],
                within: None,
            },
        );
        let rects: Vec<Vec<f64>> = turned
            .iter()
            .map(|r| shift_rect(r, placed[0] - next[0], placed[1] - next[1]))
            .collect();
        // §9.23: on an axis-swapping page (/Rotate 90/270) a view flip is the other PDF flip.
        let swaps = m[1].abs() > m[0].abs();
        let transform = match which {
            "rotateCw" => "rotate-cw",
            "rotateCcw" => "rotate-ccw",
            _ if (which == "flipH") != swaps => "flip-h",
            _ => "flip-v",
        };
        let wanted: Vec<ObjectFocus> = targets
            .iter()
            .zip(&rects)
            .map(|(t, r)| ObjectFocus {
                target_id: t.item.target_id.clone(),
                rect: r.clone(),
                ..t.member.clone()
            })
            .collect();
        let (previous, selection) = (
            self.direct_focused_object.clone(),
            self.direct_selection.clone(),
        );
        self.direct_selection = wanted.clone();
        self.direct_focused_object = wanted.last().cloned();
        self.direct_focused_mark = None;
        let ok = if targets.len() == 1 {
            let bx = self.object_box(&scan, &size, &rects[0])?;
            self.apply_object_edit(
                &scan,
                &targets[0].item,
                "transform",
                as_map(json!({"transform": transform, "box": bx})),
                Vec::new(),
            )
        } else {
            let mut entries = Vec::new();
            for (t, r) in targets.iter().zip(&rects) {
                entries.push(ObjectEntry {
                    item: t.item.clone(),
                    r#box: Some(self.object_box(&scan, &size, r)?),
                });
            }
            self.apply_objects_edit(&scan, &entries, "transform", Some(transform))
        };
        if !ok {
            self.direct_focused_object = previous;
            self.direct_selection = selection;
            self.mark_selection();
            return Ok(false);
        }
        self.after_object_edit(from);
        Ok(true)
    }

    /// R:4458 `afterObjectEdit()`: the Inspector shows the anchor's new box; the button that acted gets focus back.
    fn after_object_edit(&mut self, from: Option<&str>) {
        if let Some(anchor) = self.direct_focused_object.clone() {
            self.sync_object_inspector(&anchor.target_id);
        }
        if let Some(id) = from {
            self.set_focus(Focus::Element(id.to_string()));
        }
    }

    /// R:4463 `commitObjectCrop()`: one `edit-object {action:'crop', box}`.
    pub fn commit_object_crop(
        &mut self,
        item: &EditTarget,
        page: i64,
        initial: &[f64],
        rect: &[f64],
    ) -> Result<bool, Refusal> {
        let scan = self.content_scan.clone();
        let (Some(scan), Some(size)) = (
            scan.filter(|s| s.page == page && s.transform.is_some()),
            self.page_size(page),
        ) else {
            return Ok(false);
        };
        let Some(current) = self
            .resolve_object(&scan, item, initial)
            .filter(|c| c.placement.is_some())
        else {
            return Ok(false);
        };
        let (previous, selection) = (
            self.direct_focused_object.clone(),
            self.direct_selection.clone(),
        );
        let focus = Self::object_focus_of(&current, rect);
        self.direct_focused_object = Some(focus.clone());
        self.direct_focused_mark = None;
        self.direct_selection = vec![focus];
        let bx = self.object_box(&scan, &size, rect)?;
        if self.apply_object_edit(
            &scan,
            &current,
            "crop",
            as_map(json!({"box": bx})),
            Vec::new(),
        ) {
            self.after_object_edit(None);
            return Ok(true);
        }
        self.direct_focused_object = previous;
        self.direct_selection = selection;
        Ok(false)
    }

    /// R:4468 `toggleCrop()` / `enterCrop()` / `leaveCrop()`.
    pub fn toggle_crop(&mut self) {
        if self.direct_crop.is_some() {
            self.leave_crop()
        } else {
            self.enter_crop()
        }
    }

    pub fn enter_crop(&mut self) {
        let (Some(targets), Some(m)) = (
            self.selection_targets(),
            self.content_scan.as_ref().and_then(|s| s.transform.clone()),
        ) else {
            return;
        };
        if targets.len() != 1 || !self.object_items_for(Some(&targets)).get("crop") {
            return;
        }
        let id = targets[0].item.target_id.clone();
        self.direct_crop = Some(DirectCrop {
            target_id: id.clone(),
            full: view_full(&targets[0].item, &m),
        });
        self.paint_crop(Some(&NodeKey::Object(id)));
    }

    pub fn leave_crop(&mut self) {
        self.direct_crop = None;
        self.paint_crop(None);
    }

    /// R:4481 `paintCrop()`: `.cropping` on the member and one dashed `.crop-full` under it.
    pub fn paint_crop(&mut self, node: Option<&NodeKey>) {
        self.lane2
            .hit
            .nodes
            .retain(|n| n.kind != NodeKind::CropFull);
        for n in &mut self.lane2.hit.nodes {
            n.cropping = false;
        }
        if let (Some(crop), Some(key)) = (self.direct_crop.clone(), node)
            && let Some(i) = self.lane2.hit.find(key)
        {
            self.lane2.hit.nodes[i].cropping = true;
            let mut frame = HitNode::new(NodeKind::CropFull, &crop.full, self.shown_zoom);
            frame.seq = self.lane2.hit.nodes[i].seq;
            self.lane2.hit.nodes.insert(i, frame);
        }
        let on = self.direct_crop.is_some();
        let flag = if on { "true" } else { "false" };
        self.dom.set_attr(menu_button("crop"), "aria-checked", flag);
        self.dom.set_checked(menu_button("crop"), on);
        let inspector = transform_button("crop");
        self.dom.set_attr(inspector, "aria-pressed", flag);
        self.dom.set_checked(inspector, on);
    }

    /// R:4497: a press anywhere but the cropping object (Shift-click included), the menu or the
    /// Inspector leaves crop mode.
    pub(crate) fn on_document_pointerdown_crop(&mut self, ev: &PtrEvent) {
        let Some(crop) = self.direct_crop.clone() else {
            return;
        };
        if ev.target.in_menu() {
            return;
        }
        if !ev.shift
            && let super::hit::EvTarget::Node { index, .. } = ev.target
            && self.object_node(&crop.target_id) == Some(index)
        {
            return;
        }
        self.leave_crop();
        self.crop_dismiss = true;
    }

    /// R:4516 `inspectorTargets()`: the selection when the Inspector's object is a member, else that object alone.
    pub fn inspector_targets(&self) -> Option<Vec<ObjectTarget>> {
        let item = self.content_object_target.as_ref()?;
        let scan = self.content_scan.as_ref()?;
        let m = scan.transform.as_ref()?;
        if scan.page != self.wanted_page {
            return None;
        }
        if self
            .direct_selection
            .iter()
            .any(|v| v.target_id == item.target_id)
        {
            return self.selection_targets();
        }
        let current = scan.items.iter().find(|v| v.target_id == item.target_id)?;
        let rect = view_box(&current.placement.as_ref()?.r#box, m);
        Some(vec![ObjectTarget {
            member: Self::object_focus_of(current, &rect),
            item: current.clone(),
            rect,
        }])
    }

    /// R:4525 `syncInspectorTransform()`.
    pub fn sync_inspector_transform(&mut self) {
        let targets = self.inspector_targets();
        let items = self.object_items_for(targets.as_deref());
        for (name, id) in ids::CONTENT_TRANSFORM_BUTTONS {
            self.dom.set_disabled(id, !items.get(name));
        }
        let on = self.direct_crop.is_some();
        self.dom.set_attr(
            transform_button("crop"),
            "aria-pressed",
            if on { "true" } else { "false" },
        );
        self.dom.set_checked(transform_button("crop"), on);
    }

    /// R:4530 `inspectorTransform()`: an Inspector rotate/flip/crop button.
    pub fn inspector_transform(&mut self, which: &str) {
        let Some(targets) = self.inspector_targets() else {
            return;
        };
        if !self.object_items_for(Some(&targets)).get(which) {
            return;
        }
        let button = transform_button(which);
        if which != "crop" {
            if !self.lane2.gesture_work {
                let which = which.to_string();
                self.run_gesture_work(move |s| {
                    s.transform_objects(&targets, &which, Some(button))
                        .map(|_| ())
                });
            }
            return;
        }
        if self.direct_crop.is_some() {
            self.leave_crop();
            return;
        }
        let target = &targets[0];
        let key = NodeKey::Object(target.item.target_id.clone());
        if self.lane2.hit.find(&key).is_none() {
            return;
        }
        if !self.is_selected_object(&target.item.target_id) {
            self.focus_object(&target.item.clone(), &target.rect.clone(), &key);
        }
        self.enter_crop();
        self.set_focus(Focus::Element(button.to_string()));
    }

    /// R:4541 `toggleObject()`: Shift-click adds a non-member (it becomes the anchor) or removes a member.
    pub fn toggle_object(&mut self, item: &EditTarget, rect: &[f64], key: &NodeKey) {
        if self.direct_selection.is_empty() {
            self.focus_object(item, rect, key);
            return;
        }
        match self
            .direct_selection
            .iter()
            .position(|m| m.target_id == item.target_id)
        {
            None => {
                let member = Self::object_focus_of(item, rect);
                self.direct_selection.push(member.clone());
                self.direct_focused_object = Some(member);
                self.direct_focused_mark = None;
                self.direct_focused_link = None;
                self.mark_selection();
                self.focus_node(key);
                self.sync_note_current();
                self.sync_object_inspector(&item.target_id);
            }
            Some(index) => {
                self.direct_selection.remove(index);
                // §9.23: focus follows the anchor, so the next key never reaches the removed button.
                let Some(anchor) = self.direct_selection.last().cloned() else {
                    self.drop_direct_focus();
                    self.focus_page();
                    return;
                };
                self.direct_focused_object = Some(anchor.clone()).filter(is_object_member);
                self.mark_selection();
                if let Some(i) = self.member_node(&anchor) {
                    let key = self.lane2.hit.nodes[i].key();
                    self.focus_node(&key);
                }
                match self.direct_focused_object.clone() {
                    Some(o) => self.sync_object_inspector(&o.target_id),
                    None => self.sync_anchor(&anchor),
                }
            }
        }
    }

    /// R:4558 `objectAction()`: delete, or replace through the image picker.
    pub fn object_action(
        &mut self,
        item: &EditTarget,
        page: i64,
        rect: &[f64],
        action: &str,
    ) -> Result<bool, Refusal> {
        let Some(scan) = self.content_scan.clone().filter(|s| s.page == page) else {
            return Ok(false);
        };
        if action == "delete" {
            let Some(current) = self.resolve_object(&scan, item, rect) else {
                return Ok(false);
            };
            self.direct_focused_object = None;
            self.direct_selection.clear();
            return Ok(self.apply_object_edit(&scan, &current, "delete", Map::new(), Vec::new()));
        }
        let Some((name, bytes)) = self.pick_one_image() else {
            return Ok(false);
        };
        let scan = self.content_scan.clone().filter(|s| s.page == page);
        let current = scan
            .as_ref()
            .and_then(|s| self.resolve_object(s, item, rect));
        let (Some(scan), Some(current)) = (scan, current) else {
            return Ok(false);
        };
        let focus = Self::object_focus_of(&current, rect);
        self.direct_focused_object = Some(focus.clone());
        self.direct_selection = vec![focus];
        Ok(self.apply_object_edit(
            &scan,
            &current,
            "replace",
            as_map(json!({"image": {"name": name, "bytes": {"$buffer": 0}}})),
            vec![bytes],
        ))
    }

    // ------------------------------------------------------------------ redaction marks

    /// R:4573 `paintMarkTarget()`.
    pub fn paint_mark_target(&mut self, item: &AnnotItem, page: i64, index: usize) -> HitNode {
        let can = self.can_annotate();
        let mut node = HitNode::new(
            NodeKind::Mark {
                item: item.clone(),
                page,
                can,
            },
            &item.rect,
            self.shown_zoom,
        );
        node.classes.push("redact-target");
        node.label = format!(
            "Redaction box {}{}",
            index + 1,
            if item.search == Some(true) {
                " (search)"
            } else {
                ""
            }
        );
        node.title = if self.redact_memory_mode() {
            // Walkthrough redact#11: boxes on a signed file live in memory and stay put.
            "Redaction box: on a signed file it can't be moved; Delete removes it".into()
        } else if can {
            "Redaction box: drag to move, Delete removes it".into()
        } else {
            "This document is read-only".into()
        };
        if can {
            node.handles = FIELD_HANDLES.to_vec();
        }
        node
    }

    /// The mark target's `start(event, handle)`.
    pub(crate) fn mark_start(
        &mut self,
        item: &AnnotItem,
        page: i64,
        ev: &mut PtrEvent,
        handle: &str,
        key: NodeKey,
    ) {
        let can = self.can_annotate();
        let options = RectGesture {
            rect: item.rect.clone(),
            id: TargetRef::Number(item.id),
            handle: handle.into(),
            min: 1.0,
            can,
            commit: RectCommit::Mark {
                annot_id: item.id,
                page,
            },
            ..RectGesture::default()
        };
        self.start_rect_gesture(ev, Some(key), options);
    }

    /// R:4590 the mark target's keys: the layered Escape, nudges, Delete.
    pub(crate) fn mark_keydown(&mut self, item: &AnnotItem, page: i64, ev: &mut KeyEv) {
        // X6 §2.3 step 1: a selected box takes the first Escape; once nothing is selected the key goes on to the tool.
        if ev.key == "Escape" {
            if !self.has_direct_focus() {
                return;
            }
            ev.prevent();
            self.drop_direct_focus();
            return;
        }
        let size = self.page_size(page);
        let cmd = ev.cmd();
        let (id, rect) = (item.id, item.rect.clone());
        type Work = Box<dyn FnOnce(&mut Session) -> Result<(), Refusal>>;
        let mut work: Option<Work> = None;
        if ev.is_arrow() && (ev.alt || (!ev.meta && !ev.ctrl)) {
            if cmd && (ev.key == "ArrowUp" || ev.key == "ArrowDown") {
                return;
            }
            if let Some(size) = size
                && self.can_annotate()
            {
                let next = nudge_rect(&rect, &ev.key, ev.shift, cmd, 1.0, &size);
                work = Some(Box::new(move |s: &mut Session| {
                    s.commit_mark_rect(id, page, &next);
                    Ok(())
                }));
            }
        } else if ev.key == "Delete" || ev.key == "Backspace" {
            if self.can_annotate() {
                work = Some(Box::new(move |s: &mut Session| {
                    s.direct_focused_mark = None;
                    s.delete_note(id, "Deleted redaction box");
                    Ok(())
                }));
            }
        } else if ev.key != "Enter" && ev.key != " " {
            return;
        }
        ev.prevent();
        if let Some(work) = work
            && !self.lane2.gesture_work
        {
            self.run_gesture_work(work);
        }
    }

    /// R:4607 `commitMarkRect()`: one `redact-set`; a no-op reply writes nothing.
    pub fn commit_mark_rect(&mut self, annot_id: i64, page: i64, rect: &[f64]) -> bool {
        self.direct_focused_mark = Some(AnnotRef { annot_id });
        self.direct_focused_object = None;
        match self.rq(json!({"op": "redact-set", "page": page, "annotId": annot_id, "rect": super::direct::round_rect4(rect)})) {
            Ok(reply) => {
                if reply.get("noop") != Some(&Value::Bool(true)) {
                    self.apply_edit(&super::modes::journal_of(&reply.value), "Moved redaction box");
                }
                true
            }
            Err(err) => {
                let text = err.reply.get("error").and_then(Value::as_str).map(str::to_string).unwrap_or(err.message);
                self.show_error(&text);
                false
            }
        }
    }
}

/// An Inspector transform button's id.
pub fn transform_button(name: &str) -> &'static str {
    ids::CONTENT_TRANSFORM_BUTTONS
        .iter()
        .find(|(k, _)| *k == name)
        .map_or("", |(_, id)| id)
}

/// Whether a key event is the drive's `Space`.
pub fn is_space(ev: &KeyEv) -> bool {
    ev.key == " "
}

/// `cmdKey` of a pointer event.
pub fn ptr_cmd(ev: &PtrEvent) -> bool {
    cmd_key(ev.meta, ev.ctrl)
}

/// `round4` on a point.
pub fn round_point(p: &[f64]) -> Vec<f64> {
    p.iter().map(|v| round4(*v)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nudges_and_duplicates() {
        let size = PageSize {
            width: 600.0,
            height: 800.0,
        };
        assert_eq!(
            nudge_rect(
                &[10.0, 10.0, 20.0, 20.0],
                "ArrowLeft",
                true,
                false,
                1.0,
                &size
            ),
            vec![0.0, 10.0, 10.0, 20.0]
        );
        assert_eq!(
            nudge_rect(
                &[10.0, 10.0, 20.0, 20.0],
                "ArrowRight",
                false,
                true,
                1.0,
                &size
            ),
            vec![10.0, 10.0, 21.0, 20.0]
        );
        assert_eq!(
            nudge_rect(
                &[10.0, 10.0, 20.0, 20.0],
                "ArrowUp",
                false,
                true,
                1.0,
                &size
            ),
            vec![10.0, 10.0, 20.0, 20.0]
        );
        assert_eq!(
            menu_duplicate_rect(&[580.0, 10.0, 595.0, 20.0], [600.0, 800.0]),
            vec![570.0, 20.0, 585.0, 30.0]
        );
    }

    #[test]
    fn menu_items_keep_dom_order() {
        let mut items = MenuItems::new();
        items.set("crop", true);
        assert!(items.get("crop") && !items.get("cut"));
        assert_eq!(items.0[0].0, "cut");
        assert_eq!(menu_button("rotateCw"), "object-menu-rotate-cw");
        assert_eq!(transform_button("flipH"), "content-flip-h");
    }

    #[test]
    fn wrapping_needs_page_content_inside() {
        let form = EditTarget {
            target_id: "0.5".into(),
            placement: Some(tandem_protocol::engine::Placement {
                r#box: vec![0.0, 0.0, 100.0, 100.0],
                ..Default::default()
            }),
            ..EditTarget::default()
        };
        let inner = EditTarget {
            target_id: "0.6".into(),
            bounds: Some(vec![10.0, 10.0, 20.0, 20.0]),
            ..EditTarget::default()
        };
        let nested = EditTarget {
            target_id: "0.5/1".into(),
            bounds: Some(vec![10.0, 10.0, 20.0, 20.0]),
            ..EditTarget::default()
        };
        let scan = ContentScan {
            items: vec![form.clone(), nested],
            ..ContentScan::default()
        };
        assert!(!wraps_page_content(&form, &scan));
        let scan = ContentScan {
            items: vec![form.clone(), inner],
            ..ContentScan::default()
        };
        assert!(wraps_page_content(&form, &scan));
    }
}
