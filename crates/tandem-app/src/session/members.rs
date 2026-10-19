//! P6 U7 selection members (R:3613-3837): every member kind (objects, runs, fields, links,
//! marks) in its own mode, Shift-click toggling, re-resolving after a write, and the mixed
//! selection's move and delete (one `edit-batch`).
#![allow(dead_code)]

use super::Session;
use super::direct::{MemberEntry, Place, RectCommit, RectGesture, shift_rect, union_rect};
use super::hit::{NodeKey, NodeKind, PtrEvent, link_annot_id};
use super::nav::view_box;
use super::types::*;
use crate::engine::Refusal;
use serde_json::{Map, Value, json};
use tandem_protocol::engine::{EditTargetKind, EditTargetSupport, FieldKind};

/// R:3615 `isObjectMember()`.
pub fn is_object_member(member: &ObjectFocus) -> bool {
    member.kind == "image" || member.kind == "form"
}

/// R:3617 `memberMode()`: the one mode whose layer paints a member kind.
pub fn member_mode(kind: &str) -> UiMode {
    match kind {
        "field" => UiMode::Forms,
        "link" => UiMode::Annotate,
        "mark" => UiMode::Redact,
        _ => UiMode::Edit,
    }
}

/// R:3618 `textMember()`: a run; its name is the first 12 characters of its text.
pub fn text_member(item: &EditTarget) -> ObjectFocus {
    let text = item.text.clone().unwrap_or_default();
    // `slice(0, 12)` counts UTF-16 units.
    let units: Vec<u16> = text.encode_utf16().take(12).collect();
    ObjectFocus {
        kind: "text".into(),
        name: String::from_utf16_lossy(&units),
        target_id: item.target_id.clone(),
        rect: item.bounds.clone().unwrap_or_default(),
        text: Some(text),
    }
}

/// R:3619 `fieldMember()`.
pub fn field_member(item: &FieldItem) -> ObjectFocus {
    ObjectFocus {
        kind: "field".into(),
        name: item.name.clone(),
        target_id: item.id.to_string(),
        rect: item.rect.clone(),
        text: None,
    }
}

/// R:3620 `linkMember()`.
pub fn link_member(item: &LinkEntry) -> ObjectFocus {
    let id = link_annot_id(item);
    ObjectFocus {
        kind: "link".into(),
        name: format!("link {id}"),
        target_id: id.to_string(),
        rect: item.rect.clone(),
        text: None,
    }
}

/// R:3621 `markMember()`.
pub fn mark_member(item: &AnnotItem) -> ObjectFocus {
    ObjectFocus {
        kind: "mark".into(),
        name: format!("mark {}", item.id),
        target_id: item.id.to_string(),
        rect: item.rect.clone(),
        text: None,
    }
}

/// R:3622 `sameMember()`.
pub fn same_member(a: &ObjectFocus, b: &ObjectFocus) -> bool {
    a.kind == b.kind && a.target_id == b.target_id
}

/// JS `Number(text)` of a member's target id (NaN → never equal).
fn member_id(member: &ObjectFocus) -> Option<i64> {
    member.target_id.trim().parse::<i64>().ok()
}

impl Session {
    /// R:3623 `selectionMixed()`.
    pub fn selection_mixed(&self) -> bool {
        self.direct_selection.iter().any(|m| !is_object_member(m))
    }

    /// R:3624 `memberNode()`: the member's painted node.
    pub fn member_node(&self, member: &ObjectFocus) -> Option<usize> {
        if is_object_member(member) {
            return self.object_node(&member.target_id);
        }
        let layer = &self.lane2.hit;
        match member.kind.as_str() {
            "text" => layer.find(&NodeKey::Text(member.target_id.clone())),
            "field" => member_id(member).and_then(|id| layer.find(&NodeKey::Field(id))),
            "link" => member_id(member).and_then(|id| {
                layer.nodes.iter().position(
                    |n| matches!(&n.kind, NodeKind::Link { item, .. } if link_annot_id(item) == id),
                )
            }),
            _ => member_id(member).and_then(|id| layer.find(&NodeKey::Mark(id))),
        }
    }

    /// R:3631 `memberRect()`: the member's current view rect from the data its layer paints.
    pub fn member_rect(&self, member: &ObjectFocus) -> Option<Vec<f64>> {
        if is_object_member(member) || member.kind == "text" {
            let scan = self
                .content_scan
                .as_ref()
                .filter(|s| s.page == self.wanted_page)?;
            let item = scan
                .items
                .iter()
                .find(|v| v.target_id == member.target_id)?;
            if member.kind == "text" {
                return item.bounds.clone().filter(|b| b.len() == 4);
            }
            let (placement, m) = (item.placement.as_ref()?, scan.transform.as_ref()?);
            return Some(view_box(&placement.r#box, m));
        }
        let id = member_id(member)?;
        match member.kind.as_str() {
            "field" => self
                .fields
                .iter()
                .find(|i| i.id == id)
                .map(|i| i.rect.clone()),
            "link" => self
                .links
                .iter()
                .find(|i| link_annot_id(i) == id)
                .map(|i| i.rect.clone()),
            _ => self
                .notes
                .iter()
                .find(|i| i.id == id && i.r#type == "Redact")
                .map(|i| i.rect.clone()),
        }
    }

    /// R:3647 `selectionMembers()`: every painted member with its view rect and node, in click order.
    pub fn selection_members(&self) -> Vec<MemberEntry> {
        let mut out = Vec::new();
        for member in &self.direct_selection {
            let Some(index) = self.member_node(member) else {
                continue;
            };
            if let Some(rect) = self.member_rect(member) {
                out.push(MemberEntry {
                    member: member.clone(),
                    rect,
                    node: self.lane2.hit.nodes[index].key(),
                });
            }
        }
        out
    }

    /// R:3651 `memberOfNode()`: the member a painted target stands for, in its own mode only;
    /// None for paragraphs, unsupported runs and signatures.
    pub fn member_of_node(&self, index: usize) -> Option<ObjectFocus> {
        let node = self.lane2.hit.nodes.get(index)?;
        match &node.kind {
            NodeKind::Text { item: painted } => {
                let scan = self
                    .content_scan
                    .as_ref()
                    .filter(|s| self.ui_mode == UiMode::Edit && s.page == self.wanted_page)?;
                let item = scan
                    .items
                    .iter()
                    .find(|v| v.target_id == painted.target_id)?;
                let ok = item.kind == EditTargetKind::Text
                    && item.paragraph_id.is_none()
                    && item.support == EditTargetSupport::Candidate
                    && item.moveable == Some(true)
                    && item.bounds.as_ref().is_some_and(|b| b.len() == 4);
                ok.then(|| text_member(item))
            }
            NodeKind::Field { item: painted } => {
                if self.ui_mode != UiMode::Forms {
                    return None;
                }
                let item = self.fields.iter().find(|v| v.id == painted.id)?;
                (item.kind != FieldKind::Signature).then(|| field_member(item))
            }
            NodeKind::Link { item: painted, .. } => {
                let annot_id = link_annot_id(painted);
                if self.ui_mode != UiMode::Annotate || annot_id <= 0 {
                    return None;
                }
                self.links
                    .iter()
                    .find(|v| link_annot_id(v) == annot_id)
                    .map(link_member)
            }
            NodeKind::Mark { item: painted, .. } => {
                if self.ui_mode != UiMode::Redact {
                    return None;
                }
                self.notes
                    .iter()
                    .find(|v| v.id == painted.id && v.r#type == "Redact")
                    .map(mark_member)
            }
            _ => None,
        }
    }

    /// R:3672 `memberDeletable()`: runs never are deleted (Cut leaves them), so they need nothing.
    pub fn member_deletable(&mut self, member: &ObjectFocus) -> bool {
        if member.kind == "text" {
            return true;
        }
        if is_object_member(member) {
            let has = self
                .content_scan
                .as_ref()
                .and_then(|s| s.items.iter().find(|v| v.target_id == member.target_id))
                .is_some_and(|i| {
                    super::direct::has_op(i, tandem_protocol::engine::EditObjectAction::Delete)
                });
            return self.can_edit() && has;
        }
        if member.kind == "field" {
            self.can_design()
        } else {
            self.can_annotate()
        }
    }

    /// R:3678 `memberCan()`: the gate a member's own move gesture checks.
    pub fn member_can(&mut self, member: &ObjectFocus) -> bool {
        if is_object_member(member) || member.kind == "text" {
            self.can_edit()
        } else if member.kind == "field" {
            self.can_design()
        } else {
            self.can_annotate()
        }
    }

    /// R:3682 `setSelection()`: the anchor (last member) drives the single focus of its kind.
    pub fn set_selection(&mut self, members: Vec<ObjectFocus>) {
        let anchor = members.last().cloned();
        self.direct_focused_object = anchor.clone().filter(is_object_member);
        self.direct_focused_link = anchor
            .as_ref()
            .filter(|a| a.kind == "link")
            .map(|a| AnnotRef {
                annot_id: member_id(a).unwrap_or(0),
            });
        self.direct_focused_mark = anchor
            .as_ref()
            .filter(|a| a.kind == "mark")
            .map(|a| AnnotRef {
                annot_id: member_id(a).unwrap_or(0),
            });
        self.direct_selection = members;
    }

    /// R:3690 `syncAnchor()`: the anchor is mirrored without opening an editor.
    pub fn sync_anchor(&mut self, anchor: &ObjectFocus) {
        if is_object_member(anchor) {
            self.sync_object_inspector(&anchor.target_id);
        } else if anchor.kind == "text" {
            let index = self
                .content_scan
                .as_ref()
                .and_then(|s| s.items.iter().position(|v| v.target_id == anchor.target_id));
            if let Some(index) = index {
                self.select_content_row(index, false);
            }
        } else if anchor.kind == "field" {
            // The field list's row of the anchor is `aria-current` (the UI reads it here).
            self.lane2.field_row_current = Some(anchor.target_id.clone());
        }
        self.sync_note_current();
        self.sync_link_current();
    }

    /// R:3697 `toggleMember()`: Shift-click adds a member (it becomes the anchor) or removes one.
    pub fn toggle_member(&mut self, member: ObjectFocus) {
        self.leave_crop();
        let mode = self.ui_mode;
        let kept: Vec<ObjectFocus> = self
            .direct_selection
            .iter()
            .filter(|v| member_mode(&v.kind) == mode)
            .cloned()
            .collect();
        let next: Vec<ObjectFocus> = if kept.iter().any(|v| same_member(v, &member)) {
            kept.into_iter()
                .filter(|v| !same_member(v, &member))
                .collect()
        } else {
            let mut next = kept;
            next.push(member);
            next
        };
        if next.is_empty() {
            self.drop_direct_focus();
            self.focus_page();
            return;
        }
        let anchor = next.last().cloned().unwrap();
        self.set_selection(next);
        self.mark_selection();
        if let Some(index) = self.member_node(&anchor) {
            let key = self.lane2.hit.nodes[index].key();
            self.focus_node(&key);
        }
        self.sync_anchor(&anchor);
    }

    /// R:3707 `shiftSelect()`: a Shift-click on a run, field, link or mark; an open run or
    /// field editor commits first and seeds the selection.
    pub fn shift_select(&mut self, member: Option<ObjectFocus>) {
        let Some(member) = member else {
            self.announce("Paragraphs cannot join a selection");
            return;
        };
        // The click's own focus change may already have blurred (and committed) the editor.
        let blurred = self.lane2.blurred.clone();
        let active = self.direct_active.clone().or(blurred.clone());
        let Some(active) = active else {
            self.toggle_member(member);
            return;
        };
        let seed = match (&active.kind, &active.item) {
            (DirectKind::Field, DirectItem::Field(item)) => Some(field_member(item)),
            (DirectKind::Text, DirectItem::Target(item)) if active.paragraph.is_none() => {
                Some(text_member(item))
            }
            _ => None,
        };
        if blurred.is_none() && self.direct_active.is_some() {
            self.finish_direct();
        }
        if let Some(seed) = seed
            && member_mode(&seed.kind) == self.ui_mode
            && !same_member(&seed, &member)
        {
            self.clear_direct_focus();
            self.set_selection(vec![seed]);
        }
        self.toggle_member(member);
    }

    /// R:3719 `resolveMembers()`: re-resolve a selection holding a non-object member after a
    /// repaint; a member whose layer is painted but has no target drops.
    pub fn resolve_members(&mut self) {
        let scan = self.content_scan.clone();
        let mut used: Vec<String> = Vec::new();
        let mut next: Vec<ObjectFocus> = Vec::new();
        // P6 U7 review F1: a scan from before the write is stale; keep the member until the new scan lands.
        let content_ready = scan.as_ref().is_some_and(|s| {
            s.page == self.wanted_page && s.revision == self.revision && s.transform.is_some()
        });
        for member in self.direct_selection.clone() {
            if member_mode(&member.kind) != self.ui_mode {
                continue;
            }
            if is_object_member(&member) || member.kind == "text" {
                let Some(scan) = scan.as_ref().filter(|_| content_ready) else {
                    next.push(member);
                    continue;
                };
                let painted = |s: &Session, item: &EditTarget, used: &Vec<String>| {
                    !used.contains(&item.target_id)
                        && s.member_node(&ObjectFocus {
                            target_id: item.target_id.clone(),
                            ..member.clone()
                        })
                        .is_some()
                };
                let found: Option<EditTarget> = if member.kind == "text" {
                    let near = |item: &EditTarget| {
                        item.kind == EditTargetKind::Text
                            && item.bounds.as_ref().is_some_and(|b| {
                                b.len() == 4
                                    && b.iter().enumerate().all(|(i, v)| {
                                        member.rect.get(i).is_some_and(|r| (v - r).abs() <= 1.01)
                                    })
                            })
                    };
                    scan.items
                        .iter()
                        .find(|i| {
                            i.target_id == member.target_id
                                && i.kind == EditTargetKind::Text
                                && painted(self, i, &used)
                        })
                        .or_else(|| {
                            scan.items.iter().find(|i| {
                                near(i) && i.text == member.text && painted(self, i, &used)
                            })
                        })
                        .cloned()
                } else {
                    let m = scan.transform.as_ref().unwrap();
                    let exact = scan.items.iter().find(|i| {
                        i.target_id == member.target_id
                            && i.placement.is_some()
                            && painted(self, i, &used)
                    });
                    match exact {
                        Some(item) => Some(item.clone()),
                        None => {
                            let mut best: Option<(&EditTarget, f64)> = None;
                            for item in scan.items.iter().filter(|i| {
                                i.kind.as_str() == member.kind
                                    && i.object_name.clone().unwrap_or_default() == member.name
                                    && i.placement.is_some()
                                    && painted(self, i, &used)
                            }) {
                                let d = super::direct::rect_distance(
                                    &view_box(&item.placement.as_ref().unwrap().r#box, m),
                                    &member.rect,
                                );
                                if best.is_none_or(|(_, bd)| d < bd) {
                                    best = Some((item, d));
                                }
                            }
                            best.map(|(i, _)| i.clone())
                        }
                    }
                };
                let Some(found) = found else { continue };
                used.push(found.target_id.clone());
                if member.kind == "text" {
                    next.push(text_member(&found));
                } else {
                    let rect = view_box(
                        &found.placement.as_ref().unwrap().r#box,
                        scan.transform.as_ref().unwrap(),
                    );
                    next.push(ObjectFocus {
                        target_id: found.target_id.clone(),
                        rect,
                        ..member.clone()
                    });
                }
                continue;
            }
            let ready = match member.kind.as_str() {
                "field" => self.fields_page == self.wanted_page,
                "link" => self.links_page == self.wanted_page,
                _ => true,
            };
            let rect = if self.member_node(&member).is_some() {
                self.member_rect(&member)
            } else {
                None
            };
            match rect {
                Some(rect) => next.push(ObjectFocus { rect, ..member }),
                None if !ready => next.push(member),
                None => {}
            }
        }
        self.set_selection(next);
    }

    /// R:3751 `startSelectionGesture()`: a body drag on any member of a selection of two or
    /// more moves every member by one clamped delta.
    pub fn start_selection_gesture(&mut self, ev: &mut PtrEvent, node: NodeKey) {
        let members = self.selection_members();
        let Some(anchor) = self.direct_selection.last().cloned() else {
            return;
        };
        if members.len() < 2 {
            return;
        }
        let union = union_rect(&members.iter().map(|e| e.rect.clone()).collect::<Vec<_>>());
        let place = Place::Shift {
            members: members
                .iter()
                .map(|e| (e.node.clone(), e.rect.clone()))
                .collect(),
            union: union.clone(),
        };
        let can = members.iter().all(|e| {
            let m = e.member.clone();
            self.member_can(&m)
        });
        let snap = self.snap_others(
            &members
                .iter()
                .map(|e| e.member.target_id.clone())
                .collect::<Vec<_>>(),
        );
        let options = RectGesture {
            rect: union.clone(),
            id: TargetRef::Text(anchor.target_id.clone()),
            handle: "grip".into(),
            min: 1.0,
            focus: Some(node),
            place,
            can,
            snap: Some(snap),
            commit: RectCommit::SelectionMove { members, union },
            ..RectGesture::default()
        };
        self.start_rect_gesture(ev, None, options);
    }

    /// R:3762 `nudgeSelection()`: arrows move the whole selection by 1 pt (Shift 10 pt), clamped as one union.
    pub fn nudge_selection(&mut self, key: &str, shift: bool) -> Result<bool, Refusal> {
        let members = self.selection_members();
        let Some(size) = self.page_size(self.wanted_page) else {
            return Ok(false);
        };
        if members.len() < 2 {
            return Ok(false);
        }
        let union = union_rect(&members.iter().map(|e| e.rect.clone()).collect::<Vec<_>>());
        let next = super::object_menu::nudge_rect(&union, key, shift, false, 1.0, &size);
        self.commit_selection_move(&members, next[0] - union[0], next[1] - union[1])
    }

    /// R:3769 `commitSelectionMove()`: one `edit-batch` moving every member by the same view delta.
    pub fn commit_selection_move(
        &mut self,
        members: &[MemberEntry],
        dx: f64,
        dy: f64,
    ) -> Result<bool, Refusal> {
        let scan = self.content_scan.clone();
        let Some(size) = self.page_size(self.wanted_page) else {
            return Ok(false);
        };
        let all_can = members.iter().all(|e| {
            let m = e.member.clone();
            self.member_can(&m)
        });
        if (dx == 0.0 && dy == 0.0) || !all_can {
            return Ok(false);
        }
        let round = super::direct::round_rect4;
        let mut items: Vec<Value> = Vec::new();
        for MemberEntry { member, rect, .. } in members {
            let id = member_id(member);
            let moved = shift_rect(rect, dx, dy);
            if is_object_member(member) {
                let Some(scan) = scan.as_ref().filter(|s| s.transform.is_some()) else {
                    return Ok(false);
                };
                let bx = self.object_box(scan, &size, &moved)?;
                items.push(json!({"op": "edit-object", "targetId": member.target_id, "action": "move", "box": bx}));
            } else if member.kind == "text" {
                items.push(json!({"op": "edit-text", "targetId": member.target_id, "move": {"dx": dx, "dy": dy}}));
            } else if member.kind == "field" {
                items.push(json!({"op": "field-props", "widgetId": id, "rect": round(&moved)}));
            } else if member.kind == "link" {
                items.push(json!({"op": "link-set", "annotId": id, "rect": round(&moved)}));
            } else {
                items.push(json!({"op": "redact-set", "annotId": id, "rect": round(&moved)}));
            }
        }
        // §9.13: every member's rect is its requested rect before the reply repaints.
        let previous = self.direct_selection.clone();
        let moved: Vec<ObjectFocus> = self
            .direct_selection
            .iter()
            .map(
                |v| match members.iter().find(|e| same_member(&e.member, v)) {
                    Some(e) => ObjectFocus {
                        rect: shift_rect(&e.rect, dx, dy),
                        ..v.clone()
                    },
                    None => v.clone(),
                },
            )
            .collect();
        self.set_selection(moved);
        let name = self
            .direct_selection
            .last()
            .map(|m| m.name.clone())
            .unwrap_or_default();
        let n = members.len() as i64;
        if self.apply_selection_batch(items, "move", &format!("Moved {n} items"), n, &name) {
            return Ok(true);
        }
        self.set_selection(previous);
        self.restore_direct_focus(false);
        Ok(false)
    }

    /// R:3791 `deleteSelection()`: Delete on a selection with a non-object member; a run
    /// refuses on the page with no request.
    pub fn delete_selection(&mut self) -> bool {
        if self.direct_selection.iter().any(|m| m.kind == "text") {
            self.show_error(super::consts::RUN_DELETE_TEXT);
            return false;
        }
        let count = self.direct_selection.len();
        let members = self.direct_selection.clone();
        self.delete_members(
            members,
            &format!("Deleted {count} item{}", if count == 1 { "" } else { "s" }),
        )
    }

    /// R:3797 `deleteMembers()`: one delete per member (a radio: every kid of its group); one
    /// item is its single op, more are one `edit-batch`.
    pub fn delete_members(&mut self, members: Vec<ObjectFocus>, announcement: &str) -> bool {
        if members.is_empty() || !members.iter().all(|m| self.member_deletable(m)) {
            return false;
        }
        let mut items: Vec<Value> = Vec::new();
        for member in &members {
            let id = member_id(member);
            if is_object_member(member) {
                items.push(
                    json!({"op": "edit-object", "targetId": member.target_id, "action": "delete"}),
                );
            } else if member.kind == "field" {
                let field = self.fields.iter().find(|v| Some(v.id) == id).cloned();
                let kids: Vec<FieldItem> = match &field {
                    Some(f) if f.kind == FieldKind::Radio => self
                        .fields
                        .iter()
                        .filter(|v| v.kind == FieldKind::Radio && v.name == f.name)
                        .cloned()
                        .collect(),
                    Some(f) => vec![f.clone()],
                    None => Vec::new(),
                };
                for kid in kids {
                    items.push(json!({"op": "field-delete", "widgetId": kid.id}));
                }
            } else if member.kind == "link" {
                items.push(json!({"op": "link-delete", "annotId": id}));
            } else if member.kind == "mark" {
                items.push(json!({"op": "annot-delete", "annotId": id}));
            }
        }
        let previous = self.direct_selection.clone();
        let name = members.last().map(|m| m.name.clone()).unwrap_or_default();
        self.set_selection(Vec::new());
        if self.apply_selection_batch(items, "delete", announcement, members.len() as i64, &name) {
            return true;
        }
        self.set_selection(previous);
        self.restore_direct_focus(false);
        false
    }

    /// R:3816 `applySelectionBatch()`: one journal op for a selection's moves or deletes;
    /// `content.edited {kind:'selection'}`; one restore after the refreshes.
    pub fn apply_selection_batch(
        &mut self,
        items: Vec<Value>,
        action: &str,
        announcement: &str,
        count: i64,
        name: &str,
    ) -> bool {
        let page = self.wanted_page;
        let scan = self.content_scan.clone();
        let content = items.iter().any(|i| {
            matches!(
                i.get("op").and_then(Value::as_str),
                Some("edit-text" | "edit-object")
            )
        });
        if items.is_empty() || (content && scan.as_ref().is_none_or(|s| s.page != page)) {
            return false;
        }
        let mut stamp = Map::new();
        match scan.as_ref().filter(|_| content) {
            Some(s) => {
                stamp.insert("generation".into(), json!(s.generation));
                stamp.insert("revision".into(), json!(s.revision));
            }
            None => {
                stamp.insert("revision".into(), json!(self.revision));
            }
        }
        let payload = if items.len() >= 2 {
            let mut p = Map::new();
            p.insert("op".into(), json!("edit-batch"));
            p.insert("page".into(), json!(page));
            p.extend(stamp);
            p.insert("items".into(), Value::Array(items));
            Value::Object(p)
        } else {
            let mut rest = items[0].as_object().cloned().unwrap_or_default();
            let op = rest.remove("op").unwrap_or(Value::Null);
            if op == json!("link-delete") {
                let annot_id = rest.get("annotId").and_then(Value::as_i64);
                let Some(entry) = self
                    .links
                    .iter()
                    .find(|v| Some(link_annot_id(v)) == annot_id)
                else {
                    return false;
                };
                json!({"op": op, "page": page, "linkId": entry.id})
            } else {
                let mut p = Map::new();
                p.insert("op".into(), op.clone());
                p.insert("page".into(), json!(page));
                if op == json!("edit-object") {
                    p.extend(stamp);
                }
                p.extend(rest);
                Value::Object(p)
            }
        };
        let reply = match self.rq(payload) {
            Ok(reply) => reply,
            Err(err) => {
                self.show_clip_refusal(&err, name);
                let (w, h) = self.canvas_client();
                let page = self.shown_page;
                self.paint_hits(w, h, page);
                return false;
            }
        };
        if reply.get("noop") == Some(&Value::Bool(true)) {
            return true;
        }
        self.content_edited = Some(ContentEdited::Object {
            kind: "selection".into(),
            action: action.into(),
            name: name.into(),
            count: Some(count),
            skipped: None,
        });
        self.apply_edit(reply.value, announcement);
        if self.ui_mode != UiMode::Edit {
            self.refresh_content_targets(true);
        }
        self.restore_direct_focus(true);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_name_and_mode() {
        let run = EditTarget {
            target_id: "0.12".into(),
            text: Some("Hello, wonderful world".into()),
            bounds: Some(vec![1.0, 2.0, 3.0, 4.0]),
            ..EditTarget::default()
        };
        let m = text_member(&run);
        assert_eq!((m.name.as_str(), m.kind.as_str()), ("Hello, wonde", "text"));
        assert_eq!(member_mode("field"), UiMode::Forms);
        assert_eq!(member_mode("image"), UiMode::Edit);
        let link = LinkEntry {
            id: 3,
            annot_id: 17,
            rect: vec![0.0; 4],
            ..LinkEntry::default()
        };
        assert_eq!(link_member(&link).name, "link 17");
        assert!(same_member(
            &mark_member(&AnnotItem {
                id: 4,
                ..AnnotItem::default()
            }),
            &ObjectFocus {
                kind: "mark".into(),
                target_id: "4".into(),
                ..ObjectFocus::default()
            }
        ));
    }
}
