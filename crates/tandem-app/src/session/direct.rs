//! Direct manipulation on the page (R:3121-3612): editor placement, the one move/resize gesture
//! engine (`startRectGesture`, 3 px threshold, snapping, Escape), the text-run gesture, the
//! field handles and nudges, and `paintHits`, which builds the hit layer.
#![allow(dead_code)]

use super::Session;
use super::dom_ids as ids;
use super::hit::{
    CssBox, EvTarget, Focus, HitNode, KeyEv, NodeKey, NodeKind, PtrEvent, link_annot_id,
};
use super::nav::{
    GestureOptions, editor_box, gesture_rect, is_pressed, reading_order, round4, snap_rect,
    view_box,
};
use super::types::*;
use crate::engine::Refusal;
use crate::js::js_num;
use serde_json::{Value, json};
use tandem_protocol::engine::{
    EditObjectAction, EditTargetKind, EditTargetSupport, FieldKind, LinkKind,
};

/// R:3256 `RectGesture`: one move/resize gesture's options (the TS closures become data).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RectGesture {
    pub rect: Vec<f64>,
    pub id: TargetRef,
    pub handle: String,
    pub min: f64,
    pub lock_aspect: bool,
    /// `can()` evaluated at the press (the TS calls it once, there).
    pub can: bool,
    pub commit: RectCommit,
    /// P6 U5 §9.14: unset = the pressed button follows the rect.
    pub place: Place,
    pub duplicate: bool,
    /// `done` removes the duplicate's ghosts.
    pub ghosts: bool,
    /// P6 U6 §9.13: the button to focus when the pressed element is not in a `.hit-target`.
    pub focus: Option<NodeKey>,
    /// §2.3: crop mode's clamp box (view full ∩ page).
    pub within: Option<Vec<f64>>,
    pub crop: bool,
    /// §2.4/§9.20: snap to these view rects and the page (computed once at press).
    pub snap: Option<Vec<Vec<f64>>>,
    /// draw-signature D6: the aspect stays locked under Shift.
    pub keep_aspect: bool,
}

/// The commit callback of a [`RectGesture`].
#[derive(Clone, Debug, Default, PartialEq)]
pub enum RectCommit {
    #[default]
    None,
    /// R:3227 `commitFieldRect`.
    Field { item: FieldItem, page: i64 },
    /// R:8273 `commitLinkRect` (lane 4).
    Link { item: LinkEntry, page: i64 },
    /// R:4607 `commitMarkRect`.
    Mark { annot_id: i64, page: i64 },
    /// R:4717 `commitBoxMove`.
    BoxMove { annot_id: i64, page: i64 },
    /// R:4937 `commitSignatureRect`.
    Signature {
        annot_id: i64,
        page: i64,
        initial: Vec<f64>,
        resize: bool,
    },
    /// R:4332 `commitObjectRect`.
    ObjectRect {
        item: EditTarget,
        page: i64,
        initial: Vec<f64>,
    },
    /// R:4458 `commitObjectCrop`.
    ObjectCrop {
        item: EditTarget,
        page: i64,
        initial: Vec<f64>,
    },
    /// R:4370 `commitObjectDuplicate`.
    ObjectDuplicate {
        item: EditTarget,
        page: i64,
        initial: Vec<f64>,
    },
    /// R:4394 `commitObjectGroup` (a group drag; `duplicate` = ⌥-drag).
    ObjectGroup {
        targets: Vec<ObjectTarget>,
        union: Vec<f64>,
        duplicate: bool,
    },
    /// R:4428 `commitGroupResize`.
    GroupResize {
        targets: Vec<ObjectTarget>,
        union: Vec<f64>,
    },
    /// R:3769 `commitSelectionMove`.
    SelectionMove {
        members: Vec<MemberEntry>,
        union: Vec<f64>,
    },
}

/// How a gesture places what it moves.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Place {
    /// `placeBox(button, rect, shownZoom)`.
    #[default]
    Button,
    /// Every member shifted by the union's delta, and the group box (R:3754, R:4388).
    Shift {
        members: Vec<(NodeKey, Vec<f64>)>,
        union: Vec<f64>,
    },
    /// Every member mapped by the union's affine, and the group box (R:4421).
    Map {
        members: Vec<(NodeKey, Vec<f64>)>,
        union: Vec<f64>,
    },
}

/// R:3647 `selectionMembers()`'s entry: a painted member, its view rect and its node.
#[derive(Clone, Debug, PartialEq)]
pub struct MemberEntry {
    pub member: ObjectFocus,
    pub rect: Vec<f64>,
    pub node: NodeKey,
}

/// A live gesture's window listeners and closure state.
#[derive(Clone, Debug, PartialEq)]
pub enum LiveGesture {
    Rect(RectLive),
    Text(TextLive),
}

/// `startRectGesture`'s closure state.
#[derive(Clone, Debug, PartialEq)]
pub struct RectLive {
    pub options: RectGesture,
    pub page_index: i64,
    pub initial: Vec<f64>,
    pub start: [f64; 2],
    pub pt: [f64; 2],
    pub button: Option<NodeKey>,
    pub moved: bool,
}

/// `startTextGesture`'s closure state.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLive {
    pub item: EditTarget,
    pub handle: String,
    pub scan: ContentScan,
    pub page_index: i64,
    pub initial: Vec<f64>,
    pub paragraph: Option<ParagraphItem>,
    pub initial_size: f64,
    pub initial_width: f64,
    pub start: [f64; 2],
    pub button: NodeKey,
    pub moved: bool,
    pub size: f64,
    pub width: f64,
}

/// `{ width?, move?: {dx, dy}, size? }` of `edit-paragraph` / `edit-text`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextTransform {
    pub width: Option<f64>,
    pub mv: Option<(f64, f64)>,
    pub size: Option<f64>,
}

impl TextTransform {
    fn extend(&self, message: &mut Value) {
        if let Some(width) = self.width {
            message["width"] = json!(width);
        }
        if let Some((dx, dy)) = self.mv {
            message["move"] = json!({"dx": dx, "dy": dy});
        }
        if let Some(size) = self.size {
            message["size"] = json!(size);
        }
    }
}

/// JS `Number(text)`: trimmed, "" → 0, anything unparsable → NaN.
pub fn js_number(text: &str) -> f64 {
    let t = crate::js::js_trim(text);
    if t.is_empty() {
        return 0.0;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// `Number(x) || fallback`.
fn or_number(text: Option<&str>, fallback: f64) -> f64 {
    let n = text.map_or(f64::NAN, js_number);
    if n.is_nan() || n == 0.0 { fallback } else { n }
}

/// R:3574 `unionRect()`.
pub fn union_rect(rects: &[Vec<f64>]) -> Vec<f64> {
    use super::nav::{max_of, min_of};
    let col = |i: usize| rects.iter().map(|r| r[i]).collect::<Vec<f64>>();
    vec![
        min_of(&col(0)),
        min_of(&col(1)),
        max_of(&col(2)),
        max_of(&col(3)),
    ]
}

/// R:3612 `shiftRect()`.
pub fn shift_rect(rect: &[f64], dx: f64, dy: f64) -> Vec<f64> {
    vec![rect[0] + dx, rect[1] + dy, rect[2] + dx, rect[3] + dy]
}

/// R:3527 `rectDistance()`.
pub fn rect_distance(a: &[f64], b: &[f64]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1]) + (a[2] - b[2]).hypot(a[3] - b[3])
}

/// R:3576 `mapRect()`: `rect` (inside `from`) under the axis-aligned affine that maps `from` onto `to`.
pub fn map_rect(rect: &[f64], from: &[f64], to: &[f64]) -> Vec<f64> {
    let or1 = |v: f64| if v == 0.0 || v.is_nan() { 1.0 } else { v };
    let sx = (to[2] - to[0]) / or1(from[2] - from[0]);
    let sy = (to[3] - to[1]) / or1(from[3] - from[1]);
    vec![
        to[0] + (rect[0] - from[0]) * sx,
        to[1] + (rect[1] - from[1]) * sy,
        to[0] + (rect[2] - from[0]) * sx,
        to[1] + (rect[3] - from[1]) * sy,
    ]
}

/// R:3581 `turnRect()`: turned 90° or flipped about (cx, cy) in view space (y down).
pub fn turn_rect(rect: &[f64], which: &str, cx: f64, cy: f64) -> Vec<f64> {
    let (x0, y0, x1, y1) = (rect[0], rect[1], rect[2], rect[3]);
    match which {
        "rotateCw" => vec![
            cx - (y1 - cy),
            cy + (x0 - cx),
            cx - (y0 - cy),
            cy + (x1 - cx),
        ],
        "rotateCcw" => vec![
            cx + (y0 - cy),
            cy - (x1 - cx),
            cx + (y1 - cy),
            cy - (x0 - cx),
        ],
        "flipH" => vec![2.0 * cx - x1, y0, 2.0 * cx - x0, y1],
        _ => vec![x0, 2.0 * cy - y1, x1, 2.0 * cy - y0],
    }
}

/// R:3570 `groupMin()`: the union's smallest side keeps every member at 1 pt or more.
pub fn group_min(targets: &[ObjectTarget], union: &[f64]) -> f64 {
    let mut values = vec![1.0];
    for t in targets {
        let a = (union[2] - union[0]) / (t.rect[2] - t.rect[0]).max(1e-6);
        let b = (union[3] - union[1]) / (t.rect[3] - t.rect[1]).max(1e-6);
        values.push(a.max(b));
    }
    super::nav::max_of(&values)
}

/// R:4748 `isDrawnSignature` / `isSignature`.
pub fn is_drawn_signature(item: &AnnotItem) -> bool {
    item.r#type == "Ink" && item.subject == tandem_protocol::limits::DRAWN_SIGNATURE_SUBJECT
}
pub fn is_signature(item: &AnnotItem) -> bool {
    is_drawn_signature(item)
        || (item.r#type == "Stamp" && item.subject == tandem_protocol::limits::SIGNATURE_SUBJECT)
}

/// R:3510 `sigSigned()`: the engine's `signed` flag (the widget's own `/V`).
pub fn sig_signed(item: &FieldItem) -> bool {
    item.signed == Some(true)
}

/// Whether a `{op}` list holds `op`.
pub fn has_op(item: &EditTarget, op: EditObjectAction) -> bool {
    item.object_ops
        .as_ref()
        .is_some_and(|ops| ops.contains(&op))
}

impl Session {
    // ------------------------------------------------------------------ editor placement

    /// R:3121 `placeDirectActive(zoom)`.
    pub fn place_direct_active(&mut self, zoom: f64) {
        let Some(active) = self.direct_active.as_ref() else {
            return;
        };
        let rect = match (&active.paragraph, &active.item) {
            (Some(p), _) => p.bounds.clone(),
            (None, DirectItem::Field(f)) => f.rect.clone(),
            (None, DirectItem::Link(l)) => l.rect.clone(),
            (None, DirectItem::Target(t)) => t.bounds.clone().unwrap_or_default(),
        };
        let paragraph = active.paragraph.clone();
        let kind = active.kind;
        if let Some(active) = self.direct_active.as_mut() {
            place_editor(&mut active.editor, &rect, zoom);
            if let DirectItem::Field(f) = &active.item {
                active.editor.font_px =
                    super::editors::field_font_px(f, active.editor.font_px, zoom);
            }
            if let Some(p) = &paragraph {
                active.editor.width = p.width * zoom;
                active.editor.line_height = Some(p.leading * zoom);
                grow_paragraph_editor(&mut active.editor, Some((p.bounds[3] - p.bounds[1]) * zoom));
            }
        }
        if kind == DirectKind::Add || kind == DirectKind::Textbox {
            self.grow_add_editor();
        }
    }

    /// R:3128 `growAddEditor()`: the add editor keeps its own font size (no 10 px floor), as wide
    /// as its box. (The browser also widened it to a longer draft, up to the page's right edge;
    /// text is not measured here, so the box keeps its initial width.)
    pub fn grow_add_editor(&mut self) {
        let zoom = self.shown_zoom;
        let Some(active) = self.direct_active.as_ref() else {
            return;
        };
        let rect = match &active.item {
            DirectItem::Target(t) => t.bounds.clone().unwrap_or_default(),
            _ => Vec::new(),
        };
        if rect.len() != 4 || self.page_size(active.page).is_none() {
            return;
        }
        let Some(active) = self.direct_active.as_mut() else {
            return;
        };
        let size = active
            .editor
            .font_size
            .as_deref()
            .map_or(f64::NAN, js_number);
        active.editor.font_px = (size * zoom).min(400.0);
        active.editor.width = (rect[2] - rect[0]) * zoom;
        grow_paragraph_editor(&mut active.editor, Some((rect[3] - rect[1]) * zoom));
    }

    // ------------------------------------------------------------------ fill bookkeeping

    /// R:3158 `fillOp()`.
    pub fn fill_op(op: &str, id: i64, rect: Option<Vec<f64>>) -> FillOp {
        FillOp {
            op: op.to_string(),
            id,
            rect,
        }
    }

    /// R:3159 `noteFilled()`: the first Save after a fill writes a "(filled)" copy.
    pub fn note_filled(&mut self) {
        let filled_name = self
            .doc
            .as_ref()
            .is_some_and(|d| d.name.to_lowercase().ends_with(" (filled).pdf"));
        if self.fill_save == FillSave::None && self.doc.is_some() && !filled_name {
            self.fill_save = FillSave::NeedsCopy;
        }
    }

    // ------------------------------------------------------------------ focus state

    /// R:3178 `clearDirectFocus()`.
    pub fn clear_direct_focus(&mut self) {
        self.direct_focused_box = None;
        self.direct_focused_sig = None;
        self.direct_focused_object = None;
        self.direct_selection.clear();
        self.direct_focused_mark = None;
        self.direct_focused_link = None;
        self.close_object_menu(false);
        self.leave_crop();
        self.lane2
            .hit
            .nodes
            .retain(|n| n.kind != NodeKind::GroupBox);
    }

    /// R:3184 `#field-highlight` change.
    pub fn on_field_highlight_change(&mut self, checked: bool) {
        self.highlight_fields = checked;
        self.dom.set_checked(ids::FIELD_HIGHLIGHT, checked);
        self.host_store_toggle("field-highlight", checked);
        let (w, h) = self.canvas_client();
        let page = self.shown_page;
        self.paint_hits(w, h, page);
    }

    /// R:3185 `#field-next` click.
    pub fn on_field_next_click(&mut self) {
        let (page, index) = (self.wanted_page, self.direct_field_index);
        self.focus_next_field(false, page, index);
    }

    /// R:3187 `addTextBoxAtCentre()`: the keyboard way to the text-box editor, at the page centre.
    pub fn add_text_box_at_centre(&mut self) {
        let page = self.wanted_page;
        let Some(size) = self.page_size(page) else {
            return;
        };
        if self.doc.is_none() || !self.can_annotate() {
            return;
        }
        if self.rotation(page) % 360 != 0 {
            self.show_error(super::consts::TEXTBOX_ROTATED);
            return;
        }
        if self.ui_mode != UiMode::Forms {
            self.set_mode(UiMode::Forms);
        }
        self.open_text_box(page, [size.width / 2.0, size.height / 2.0], None);
    }

    /// `doc.rotations?.[page] ?? 0`.
    pub fn rotation(&self, page: i64) -> i64 {
        self.doc
            .as_ref()
            .and_then(|d| d.rotations.get(page.max(0) as usize).copied())
            .unwrap_or(0)
    }

    /// R:3198 `setOutlines(on)`.
    pub fn set_outlines(&mut self, on: bool) {
        self.show_outlines = on;
        self.dom.set_checked(ids::EDIT_OUTLINES_INPUT, on);
        self.host_store_toggle("edit-outlines", on);
        let (w, h) = self.canvas_client();
        let page = self.shown_page;
        self.paint_hits(w, h, page);
    }

    /// R:3203 `layerLive()`: the layer takes pointer input only with no placing tool armed.
    pub fn layer_live(&self) -> bool {
        let pressed = |id: &str| is_pressed(&self.dom, id);
        matches!(
            self.ui_mode,
            UiMode::Read | UiMode::Annotate | UiMode::Forms | UiMode::Edit | UiMode::Redact
        ) && !pressed(ids::NOTE_ADD_BUTTON)
            && !pressed(ids::FREE_TEXT_ADD_BUTTON)
            && !pressed(ids::INK_DRAW_BUTTON)
            && !pressed(ids::REDACT_AREA_BUTTON)
            && !pressed(ids::LINK_DRAW_BUTTON)
            && !pressed(ids::SIGFIELD_DRAW_BUTTON)
            && self.ink_stroke.is_none()
            && !(self.ui_mode == UiMode::Edit && pressed(ids::CONTENT_ADD_ARM_BUTTON))
            && !self.sign_armed
    }

    /// Whether a drag owns Escape (`escapeOwners.size > 0`).
    pub fn escape_owned(&self) -> bool {
        self.lane2.gesture.is_some()
    }

    // ------------------------------------------------------------------ fields

    /// R:3227 `commitFieldRect()`: the `field-props` a field gesture (or nudge) ends with.
    pub fn commit_field_rect(
        &mut self,
        item: &FieldItem,
        page_index: i64,
        rect: &[f64],
    ) -> Result<bool, Refusal> {
        let reply = self.rq(
            json!({"op": "field-props", "page": page_index, "widgetId": item.id, "rect": rect}),
        )?;
        self.apply_edit(&super::modes::journal_of(&reply.value), "");
        self.refresh_fields();
        Ok(true)
    }

    /// R:3231 `fieldGesture(item)(event, handle)`.
    pub fn field_gesture(
        &mut self,
        item: &FieldItem,
        ev: &mut PtrEvent,
        handle: &'static str,
        key: NodeKey,
    ) {
        let page_index = self.wanted_page;
        let can = self.can_edit();
        let options = RectGesture {
            rect: item.rect.clone(),
            id: TargetRef::Number(item.id),
            handle: handle.to_string(),
            min: 8.0,
            can,
            commit: RectCommit::Field {
                item: item.clone(),
                page: page_index,
            },
            ..RectGesture::default()
        };
        self.start_rect_gesture(ev, Some(key), options);
    }

    /// R:3234 `nudgeField(item, event)`: ⌥-arrow 1 pt (Shift 10); with ⌘ the arrow resizes.
    pub fn nudge_field(&mut self, item: &FieldItem, key: &str, shift: bool, cmd: bool) {
        if self.doc.is_none() || !self.editable() || !self.permitted("edit") {
            return;
        }
        let mut rect = item.rect.clone();
        let amount = if !cmd && shift { 10.0 } else { 1.0 };
        let page_index = self.wanted_page;
        let Some(page) = self.page_size(page_index) else {
            return;
        };
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
        if cmd {
            rect[2] = (rect[0] + 8.0).max(page.width.min(rect[2] + dx));
            rect[3] = (rect[1] + 8.0).max(page.height.min(rect[3] + dy));
        } else {
            let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
            rect[0] = 0.0f64.max((page.width - w).min(rect[0] + dx));
            rect[1] = 0.0f64.max((page.height - h).min(rect[1] + dy));
            rect[2] = rect[0] + w;
            rect[3] = rect[1] + h;
        }
        match self
            .rq(json!({"op": "field-props", "page": page_index, "widgetId": item.id, "rect": rect}))
        {
            Ok(reply) => {
                self.apply_edit(&super::modes::journal_of(&reply.value), "");
                self.refresh_fields();
                if let Some(active) = self.direct_active.as_mut()
                    && active.kind == DirectKind::Field
                    && let DirectItem::Field(field) = &mut active.item
                    && field.id == item.id
                {
                    *field = FieldItem {
                        rect,
                        ..item.clone()
                    };
                }
            }
            Err(err) => self.show_error(&err.message),
        }
    }

    /// R:3437 a field target's keys: ⌥-arrow nudges; Tab moves on; radio ←/→ moves the
    /// selection; a checkbox/radio in Forms nudges on other plain arrows; Space toggles.
    pub(crate) fn field_target_keydown(&mut self, item: &FieldItem, ev: &mut KeyEv) {
        let arrow = ev.is_arrow();
        let plain = arrow && !ev.alt && !ev.meta && !ev.ctrl;
        let toggle = item.kind == FieldKind::Radio || item.kind == FieldKind::Checkbox;
        if arrow && ev.alt {
            ev.prevent();
            self.run_field_nudge(item, ev);
        } else if ev.key == "Tab" {
            ev.prevent();
            let (page, index) = (self.wanted_page, self.direct_field_index);
            self.focus_next_field(ev.shift, page, index);
        } else if item.kind == FieldKind::Radio
            && plain
            && (ev.key == "ArrowLeft" || ev.key == "ArrowRight")
        {
            ev.prevent();
            if !self.lane2.gesture_work {
                let (item, delta) = (item.clone(), if ev.key == "ArrowRight" { 1 } else { -1 });
                self.run_gesture_work(move |s| {
                    s.move_radio(&item, delta);
                    Ok(())
                });
            }
        } else if plain && self.ui_mode == UiMode::Forms && toggle {
            ev.prevent();
            self.run_field_nudge(item, ev);
        } else if ev.key == " " && toggle {
            ev.prevent();
            let value = if item.kind == FieldKind::Checkbox {
                FieldValue::Bool(item.value != FieldValue::Bool(true))
            } else {
                FieldValue::Bool(true)
            };
            let page = self.wanted_page;
            self.commit_field(item, value, page as usize);
        }
    }

    /// `if (!directGestureWork) runGestureWork(() => nudgeField(item, event))`.
    fn run_field_nudge(&mut self, item: &FieldItem, ev: &KeyEv) {
        if self.lane2.gesture_work {
            return;
        }
        let (item, key, shift, cmd) = (item.clone(), ev.key.clone(), ev.shift, ev.cmd());
        self.run_gesture_work(move |s| {
            s.nudge_field(&item, &key, shift, cmd);
            Ok(())
        });
    }

    /// R:3471 a signature field's keys: an unsigned one nudges; Enter or Space signs or verifies.
    pub(crate) fn sig_target_keydown(&mut self, item: &FieldItem, signed: bool, ev: &mut KeyEv) {
        let arrow = ev.is_arrow();
        let plain = arrow && !ev.alt && !ev.meta && !ev.ctrl;
        if !signed && ((arrow && ev.alt) || (plain && self.ui_mode == UiMode::Forms)) {
            ev.prevent();
            self.run_field_nudge(item, ev);
        } else if ev.key == "Enter" || ev.key == " " {
            ev.prevent();
            if !self.lane2.gesture_work {
                let item = item.clone();
                self.run_gesture_work(move |s| {
                    s.activate_field(&item, true);
                    Ok(())
                });
            }
        }
    }

    // ------------------------------------------------------------------ the gesture engine

    /// R:3271 `startRectGesture()`: one move/resize gesture for fields, objects, marks, links,
    /// text boxes and signatures; the commit says whether it landed.
    pub fn start_rect_gesture(
        &mut self,
        ev: &mut PtrEvent,
        button: Option<NodeKey>,
        options: RectGesture,
    ) {
        if self.doc.is_none() || !options.can {
            self.gesture_done(&options);
            return;
        }
        ev.prevented = true;
        ev.stopped = true;
        let page_index = self.wanted_page;
        let initial = options.rect.clone();
        let button = options.focus.clone().or(button);
        // CSS px → page pt through the canvas box, as `pagePoint` maps a click.
        let Some(page_size) = self.page_size(page_index) else {
            return;
        };
        let [cw, ch] = self.lane2.canvas_css;
        let pt_x = if cw >= 1.0 {
            page_size.width / cw
        } else {
            1.0 / self.shown_zoom
        };
        let pt_y = if ch >= 1.0 {
            page_size.height / ch
        } else {
            1.0 / self.shown_zoom
        };
        if let Some(key) = &button {
            self.focus_node(key);
        }
        self.lane2.gesture = Some(LiveGesture::Rect(RectLive {
            options,
            page_index,
            initial,
            start: [ev.client_x, ev.client_y],
            pt: [pt_x, pt_y],
            button,
            moved: false,
        }));
    }

    /// `done` of a rect gesture: guides go with every exit (snap), then the ghosts.
    fn gesture_done(&mut self, options: &RectGesture) {
        if options.snap.is_some() {
            self.lane2.guides.clear();
        }
        if options.ghosts {
            self.lane2.ghosts.clear();
        }
    }

    /// The rect gesture's `update(x, y, shift, meta)`.
    fn rect_update(&mut self, live: &mut RectLive, x: f64, y: f64, shift: bool, meta: bool) {
        if !live.moved && (x - live.start[0]).hypot(y - live.start[1]) < 3.0 {
            return;
        }
        live.moved = true;
        let Some(page) = self.page_size(live.page_index) else {
            return;
        };
        let options = &live.options;
        let handle = options.handle.as_str();
        let initial = &live.initial;
        let mut dx = (x - live.start[0]) * live.pt[0];
        let mut dy = (y - live.start[1]) * live.pt[1];
        let mut guides: Option<Vec<SnapGuide>> = None;
        if let Some(snap) = &options.snap {
            // §9.19: snap the moved edges (grip: all; a handle: its own), then clamp with the snapped delta.
            let all = handle == "grip";
            let (west, east, north, south) = (
                handle.contains('w'),
                handle.contains('e'),
                handle.contains('n'),
                handle.contains('s'),
            );
            let edges: Vec<SnapEdge> = if all {
                vec![
                    SnapEdge::X0,
                    SnapEdge::X1,
                    SnapEdge::Cx,
                    SnapEdge::Y0,
                    SnapEdge::Y1,
                    SnapEdge::Cy,
                ]
            } else {
                [
                    (west, SnapEdge::X0),
                    (east, SnapEdge::X1),
                    (north, SnapEdge::Y0),
                    (south, SnapEdge::Y1),
                ]
                .into_iter()
                .filter(|(on, _)| *on)
                .map(|(_, e)| e)
                .collect()
            };
            let moving = vec![
                initial[0] + if all || west { dx } else { 0.0 },
                initial[1] + if all || north { dy } else { 0.0 },
                initial[2] + if all || east { dx } else { 0.0 },
                initial[3] + if all || south { dy } else { 0.0 },
            ];
            let (snapped, found) = if meta {
                (moving.clone(), Vec::new())
            } else {
                snap_rect(&moving, snap, [page.width, page.height], 4.0, &edges)
            };
            dx += snapped[0] - moving[0];
            dy += snapped[1] - moving[1];
            guides = Some(found);
        }
        let lock =
            (options.keep_aspect || (options.lock_aspect && !shift)) && options.within.is_none();
        let rect = gesture_rect(
            initial,
            handle,
            dx,
            dy,
            &GestureOptions {
                min: options.min,
                lock_aspect: lock,
                page: [page.width, page.height],
                within: options.within.clone(),
            },
        );
        if let Some(found) = &mut guides {
            let at = |axis: Axis| match axis {
                Axis::X => [rect[0], rect[2], (rect[0] + rect[2]) / 2.0],
                Axis::Y => [rect[1], rect[3], (rect[1] + rect[3]) / 2.0],
            };
            found.retain(|g| at(g.axis).iter().any(|v| (v - g.at).abs() <= 0.01));
            self.lane2.guides = found.clone();
        }
        self.gesture_place(&live.options.place, live.button.as_ref(), &rect);
        let kind = if options.crop {
            GestureKind::Crop
        } else if options.duplicate {
            GestureKind::Duplicate
        } else if handle == "grip" {
            GestureKind::Move
        } else {
            GestureKind::Resize
        };
        self.direct_gesture = Some(DirectGesture {
            kind,
            handle: handle.to_string(),
            target_id: options.id.clone(),
            rect,
            guides,
        });
    }

    /// The gesture's `place(rect)`.
    fn gesture_place(&mut self, place: &Place, button: Option<&NodeKey>, rect: &[f64]) {
        match place {
            Place::Button => {
                if let Some(key) = button {
                    self.place_node(key, rect);
                }
            }
            Place::Shift { members, union } => {
                for (key, member) in members {
                    self.place_node(
                        key,
                        &shift_rect(member, rect[0] - union[0], rect[1] - union[1]),
                    );
                }
                self.place_node(&NodeKey::GroupBox, rect);
            }
            Place::Map { members, union } => {
                for (key, member) in members {
                    self.place_node(key, &map_rect(member, union, rect));
                }
                self.place_node(&NodeKey::GroupBox, rect);
            }
        }
    }

    /// `placeBox(node, rect, shownZoom)` for a painted node.
    pub fn place_node(&mut self, key: &NodeKey, rect: &[f64]) {
        let zoom = self.shown_zoom;
        if let Some(i) = self.lane2.hit.find(key) {
            let node = &mut self.lane2.hit.nodes[i];
            node.rect = rect.to_vec();
            node.css = editor_box(rect, zoom).into();
        }
    }

    /// The window `pointermove` of a live gesture.
    pub(crate) fn gesture_pointermove(&mut self, ev: &PtrEvent) {
        match self.lane2.gesture.take() {
            Some(LiveGesture::Rect(mut live)) => {
                self.rect_update(
                    &mut live,
                    ev.client_x,
                    ev.client_y,
                    ev.shift,
                    super::hit::cmd_key(ev.meta, ev.ctrl),
                );
                if self.lane2.gesture.is_none() {
                    self.lane2.gesture = Some(LiveGesture::Rect(live));
                }
            }
            Some(LiveGesture::Text(mut live)) => {
                self.text_update(&mut live, ev.client_x, ev.client_y);
                if self.lane2.gesture.is_none() {
                    self.lane2.gesture = Some(LiveGesture::Text(live));
                }
            }
            None => {}
        }
    }

    /// The window `pointerup` of a live gesture (its `end`).
    pub(crate) fn gesture_pointerup(&mut self, ev: &PtrEvent) {
        match self.lane2.gesture.take() {
            Some(LiveGesture::Rect(live)) => self.rect_end(live, ev),
            Some(LiveGesture::Text(live)) => self.text_end(live, ev),
            None => {}
        }
    }

    /// R:3311 the rect gesture's `end`.
    fn rect_end(&mut self, mut live: RectLive, ev: &PtrEvent) {
        self.rect_update(
            &mut live,
            ev.client_x,
            ev.client_y,
            ev.shift,
            super::hit::cmd_key(ev.meta, ev.ctrl),
        );
        let gesture = self.direct_gesture.take();
        if live.moved {
            self.lane2.swallow_click = true;
        }
        let place = live.options.place.clone();
        let button = live.button.clone();
        let unchanged = gesture
            .as_ref()
            .is_none_or(|g| g.rect.iter().zip(&live.initial).all(|(a, b)| a == b));
        if !live.moved || unchanged {
            self.gesture_place(&place, button.as_ref(), &live.initial);
            self.gesture_done(&live.options);
            return;
        }
        let Some(gesture) = gesture else { return };
        self.clear_error();
        self.lane2.gesture_work = true;
        match self.commit_rect(&live.options.commit, &gesture.rect) {
            Ok(true) => {}
            Ok(false) => self.gesture_place(&place, button.as_ref(), &live.initial),
            Err(err) => {
                self.show_error(&err.message);
                self.gesture_place(&place, button.as_ref(), &live.initial);
            }
        }
        self.gesture_done(&live.options);
        self.lane2.gesture_work = false;
    }

    /// A gesture's commit callback.
    fn commit_rect(&mut self, commit: &RectCommit, rect: &[f64]) -> Result<bool, Refusal> {
        match commit {
            RectCommit::None => Ok(false),
            RectCommit::Field { item, page } => self.commit_field_rect(item, *page, rect),
            RectCommit::Link { item, page } => Ok(self.commit_link_rect(item, *page, rect)),
            RectCommit::Mark { annot_id, page } => {
                Ok(self.commit_mark_rect(*annot_id, *page, rect))
            }
            RectCommit::BoxMove { annot_id, page } => {
                Ok(self.commit_box_move(*annot_id, *page, rect))
            }
            RectCommit::Signature {
                annot_id,
                page,
                initial,
                resize,
            } => Ok(self.commit_signature_rect(*annot_id, *page, initial, rect, *resize)),
            RectCommit::ObjectRect {
                item,
                page,
                initial,
            } => self.commit_object_rect(item, *page, initial, rect),
            RectCommit::ObjectCrop {
                item,
                page,
                initial,
            } => self.commit_object_crop(item, *page, initial, rect),
            RectCommit::ObjectDuplicate {
                item,
                page,
                initial,
            } => self.commit_object_duplicate(item, *page, initial, rect),
            RectCommit::ObjectGroup {
                targets,
                union,
                duplicate,
            } => {
                let action = if *duplicate { "duplicate" } else { "move" };
                self.commit_object_group(targets, rect[0] - union[0], rect[1] - union[1], action)
            }
            RectCommit::GroupResize { targets, union } => {
                self.commit_group_resize(targets, union, rect)
            }
            RectCommit::SelectionMove { members, union } => {
                self.commit_selection_move(members, rect[0] - union[0], rect[1] - union[1])
            }
        }
    }

    /// The window `keydown` of a live gesture: Escape cancels it (R:3325, R:3401).
    pub(crate) fn gesture_keydown(&mut self, ev: &mut KeyEv) {
        if ev.key != "Escape" || self.lane2.gesture.is_none() {
            return;
        }
        ev.prevent();
        self.lane2.swallow_after_release = true;
        match self.lane2.gesture.take() {
            Some(LiveGesture::Rect(live)) => {
                self.direct_gesture = None;
                self.add_blocked = true;
                self.gesture_place(&live.options.place, live.button.as_ref(), &live.initial);
                self.gesture_done(&live.options);
            }
            Some(LiveGesture::Text(live)) => self.restore_text_gesture(&live.button, &live.initial),
            None => {}
        }
    }

    // ------------------------------------------------------------------ text gestures

    /// R:3328 `applyParagraphEdit()`.
    pub fn apply_paragraph_edit(
        &mut self,
        scan: &ContentScan,
        paragraph: &ParagraphItem,
        text: &str,
        transform: TextTransform,
    ) -> bool {
        let mut message = json!({"op": "edit-paragraph", "page": scan.page, "paragraphId": paragraph.paragraph_id, "generation": scan.generation, "revision": scan.revision, "text": text});
        transform.extend(&mut message);
        let reply = match self.rq(message) {
            Ok(reply) => reply,
            Err(err) => {
                self.show_error(&err.message);
                return false;
            }
        };
        self.apply_edit(&super::modes::journal_of(&reply.value), "");
        self.refresh_content_targets(true);
        let has_paragraph = self
            .direct_active
            .as_ref()
            .is_some_and(|a| a.paragraph.is_some());
        if has_paragraph && let Some(content) = self.content_scan.clone() {
            let (dx, dy) = transform.mv.unwrap_or((0.0, 0.0));
            let distance = |p: &ParagraphItem| {
                (p.bounds[0] - paragraph.bounds[0] - dx)
                    .hypot(p.bounds[1] - paragraph.bounds[1] - dy)
            };
            let mut next: Option<&ParagraphItem> = None;
            for p in &content.paragraphs {
                if next.is_none_or(|n| distance(p) < distance(n)) {
                    next = Some(p);
                }
            }
            if let Some(next) = next.cloned() {
                if let Some(active) = self.direct_active.as_mut() {
                    if let Some(item) = content
                        .items
                        .iter()
                        .find(|i| next.target_ids.first() == Some(&i.target_id))
                    {
                        active.item = DirectItem::Target(item.clone());
                    }
                    active.editor.font_size = Some(js_num(next.font_size));
                    active.paragraph = Some(next);
                    active.scan = Some(content);
                }
                let zoom = self.shown_zoom;
                self.place_direct_active(zoom);
                self.mark_active_text_target();
            } else {
                self.close_direct();
            }
        }
        true
    }

    /// R:3341 `applyTextTransform()`.
    pub fn apply_text_transform(
        &mut self,
        scan: &ContentScan,
        item: &EditTarget,
        transform: TextTransform,
    ) -> bool {
        let mut message = json!({"op": "edit-text", "page": scan.page, "targetId": item.target_id, "generation": scan.generation, "revision": scan.revision, "text": item.text.clone().unwrap_or_default()});
        transform.extend(&mut message);
        let reply = match self.rq(message) {
            Ok(reply) => reply,
            Err(err) => {
                self.show_error(&err.message);
                return false;
            }
        };
        self.apply_edit(&super::modes::journal_of(&reply.value), "");
        self.refresh_content_targets(true);
        let editing_text = self
            .direct_active
            .as_ref()
            .is_some_and(|a| a.kind == DirectKind::Text);
        if editing_text && let Some(content) = self.content_scan.clone() {
            let old = match self.direct_active.as_ref().map(|a| &a.item) {
                Some(DirectItem::Target(t)) => t.clone(),
                _ => EditTarget::default(),
            };
            let (dx, dy) = transform.mv.unwrap_or((0.0, 0.0));
            let ob = old.bounds.clone().unwrap_or_else(|| vec![f64::NAN; 4]);
            let distance = |i: &EditTarget| {
                let b = i.bounds.as_ref().unwrap();
                (b[0] - ob[0] - dx).hypot(b[1] - ob[1] - dy)
            };
            let mut next: Option<&EditTarget> = None;
            for i in content.items.iter().filter(|i| {
                i.kind == EditTargetKind::Text
                    && i.support == EditTargetSupport::Candidate
                    && i.text == old.text
                    && i.bounds.is_some()
            }) {
                if next.is_none_or(|n| distance(i) < distance(n)) {
                    next = Some(i);
                }
            }
            if let Some(next) = next.cloned() {
                let size = next.font_size.or(transform.size).unwrap_or(12.0);
                if let Some(active) = self.direct_active.as_mut() {
                    active.item = DirectItem::Target(next);
                    active.scan = Some(content);
                    active.editor.font_size = Some(js_num(size));
                }
                let zoom = self.shown_zoom;
                self.place_direct_active(zoom);
                self.mark_active_text_target();
            }
        }
        true
    }

    /// R:3354 `restoreTextGesture()`.
    fn restore_text_gesture(&mut self, button: &NodeKey, rect: &[f64]) {
        self.place_node(button, rect);
        if self
            .direct_active
            .as_ref()
            .is_some_and(|a| a.kind == DirectKind::Text)
        {
            let zoom = self.shown_zoom;
            self.place_direct_active(zoom);
        }
    }

    /// R:3358 `startTextGesture()` (candidate runs only).
    pub fn start_text_gesture(
        &mut self,
        ev: &mut PtrEvent,
        item: &EditTarget,
        handle: &'static str,
        button: NodeKey,
    ) {
        let Some(scan) = self.content_scan.clone() else {
            return;
        };
        if self.doc.is_none() || item.support != EditTargetSupport::Candidate {
            return;
        }
        let Some(initial) = item.bounds.clone() else {
            return;
        };
        ev.prevented = true;
        ev.stopped = true;
        let page_index = self.wanted_page;
        let paragraph = item.paragraph_id.as_ref().and_then(|id| {
            scan.paragraphs
                .iter()
                .find(|p| &p.paragraph_id == id)
                .cloned()
        });
        let initial_size = paragraph
            .as_ref()
            .map(|p| p.font_size)
            .or(item.font_size)
            .unwrap_or(0.0);
        let initial_width = paragraph.as_ref().map_or(0.0, |p| p.width);
        let editing_this = self.direct_active.as_ref().is_some_and(|a| {
            a.kind == DirectKind::Text
                && match &paragraph {
                    Some(p) => a
                        .paragraph
                        .as_ref()
                        .is_some_and(|ap| ap.paragraph_id == p.paragraph_id),
                    None => {
                        matches!(&a.item, DirectItem::Target(t) if t.target_id == item.target_id)
                    }
                }
        });
        if !editing_this {
            self.focus_node(&button);
        }
        self.lane2.gesture = Some(LiveGesture::Text(TextLive {
            item: item.clone(),
            handle: handle.to_string(),
            scan,
            page_index,
            initial,
            paragraph,
            initial_size,
            initial_width,
            start: [ev.client_x, ev.client_y],
            button,
            moved: false,
            size: initial_size,
            width: initial_width,
        }));
    }

    /// The text gesture's `update(x, y)`.
    fn text_update(&mut self, live: &mut TextLive, x: f64, y: f64) {
        if !live.moved && (x - live.start[0]).hypot(y - live.start[1]) < 3.0 {
            return;
        }
        live.moved = true;
        let zoom = self.shown_zoom;
        let dx = (x - live.start[0]) / zoom;
        let dy = (y - live.start[1]) / zoom;
        let initial = &live.initial;
        let mut rect = initial.clone();
        let handle = live.handle.as_str();
        if handle == "grip" {
            rect = shift_rect(&rect, dx, dy);
        } else if (handle == "e" || handle == "w") && live.paragraph.is_some() {
            live.width = (live.initial_width + if handle == "e" { dx } else { -dx }).max(8.0);
            if handle == "w" {
                rect[0] = initial[2] - live.width;
                rect[2] = initial[2];
            } else {
                rect[2] = rect[0] + live.width;
            }
        } else {
            let start_height = initial[3] - initial[1];
            let change = if handle.contains('n') { -dy } else { dy };
            let ratio = (1.0f64 / 400.0).max(400.0f64.min((start_height + change) / start_height));
            live.size = 1.0f64
                .max(400.0f64.min(crate::js::js_round(live.initial_size * ratio * 2.0) / 2.0));
            let height = start_height * live.size / live.initial_size.max(1.0);
            if handle.contains('n') {
                rect[1] = initial[3] - height;
            } else {
                rect[3] = initial[1] + height;
            }
        }
        self.place_node(&live.button, &rect);
        let paragraph = live.paragraph.is_some();
        let (size, width) = (live.size, live.width);
        if let Some(active) = self.direct_active.as_mut()
            && active.kind == DirectKind::Text
        {
            place_editor(&mut active.editor, &rect, zoom);
            active.editor.font_px = 10.0f64.max(400.0f64.min(size * zoom));
            if paragraph {
                active.editor.width = width * zoom;
                grow_paragraph_editor(&mut active.editor, Some((rect[3] - rect[1]) * zoom));
            }
        }
    }

    /// R:3382 the text gesture's `end`.
    fn text_end(&mut self, mut live: TextLive, ev: &PtrEvent) {
        self.text_update(&mut live, ev.client_x, ev.client_y);
        let zoom = self.shown_zoom;
        let dx = (ev.client_x - live.start[0]) / zoom;
        let dy = (ev.client_y - live.start[1]) / zoom;
        if !live.moved {
            return;
        }
        let current_scan = match &self.content_scan {
            Some(s) if s.page == live.page_index => s.clone(),
            _ => live.scan.clone(),
        };
        let current_paragraph = live.paragraph.as_ref().map(|p| {
            current_scan
                .paragraphs
                .iter()
                .find(|v| v.paragraph_id == p.paragraph_id)
                .cloned()
                .unwrap_or_else(|| p.clone())
        });
        let current_item = current_scan
            .items
            .iter()
            .find(|v| v.target_id == live.item.target_id)
            .cloned()
            .unwrap_or_else(|| live.item.clone());
        self.lane2.gesture_work = true;
        let handle = live.handle.as_str();
        let mut accepted = true;
        if let Some(paragraph) = &current_paragraph {
            let editing = self.direct_active.as_ref().and_then(|a| {
                a.paragraph
                    .as_ref()
                    .map(|p| (p.paragraph_id.clone(), a.editor.value.clone()))
            });
            let text = match editing {
                Some((id, value)) if id == paragraph.paragraph_id => value,
                _ => paragraph_text(paragraph, Some(&current_scan)),
            };
            if handle == "grip" {
                accepted = self.apply_paragraph_edit(
                    &current_scan,
                    paragraph,
                    &text,
                    TextTransform {
                        mv: Some((dx, dy)),
                        ..TextTransform::default()
                    },
                );
            } else if handle == "e" || handle == "w" {
                accepted = self.apply_paragraph_edit(
                    &current_scan,
                    paragraph,
                    &text,
                    TextTransform {
                        width: Some(live.width),
                        ..TextTransform::default()
                    },
                );
            } else if live.size != live.initial_size {
                accepted = self.apply_paragraph_edit(
                    &current_scan,
                    paragraph,
                    &text,
                    TextTransform {
                        size: Some(live.size),
                        ..TextTransform::default()
                    },
                );
            } else {
                self.restore_text_gesture(&live.button, &live.initial);
            }
        } else if handle == "grip" {
            accepted = self.apply_text_transform(
                &current_scan,
                &current_item,
                TextTransform {
                    mv: Some((dx, dy)),
                    ..TextTransform::default()
                },
            );
        } else if live.size != live.initial_size {
            accepted = self.apply_text_transform(
                &current_scan,
                &current_item,
                TextTransform {
                    size: Some(live.size),
                    ..TextTransform::default()
                },
            );
        } else {
            self.restore_text_gesture(&live.button, &live.initial);
        }
        if !accepted {
            self.restore_text_gesture(&live.button, &live.initial);
        }
        self.lane2.gesture_work = false;
    }

    // ------------------------------------------------------------------ paintHits

    /// R:3403 `paintHits(width, height, page)`: rebuilds the hit layer for the shown page.
    pub fn paint_hits(&mut self, width: f64, height: f64, page: i64) {
        let focused_field = match &self.lane2.focus {
            Focus::Node(key) => self
                .lane2
                .hit
                .find(key)
                .and_then(|i| self.lane2.hit.nodes[i].field_id()),
            _ => None,
        };
        let refocus = self.lane2.focus == Focus::Body || self.focus_in_hit_layer();
        let live_layer = self.layer_live();
        let zoom = self.shown_zoom;
        {
            let layer = &mut self.lane2.hit;
            layer.width = width;
            layer.height = height;
            layer.live = live_layer;
            layer.outlined = self.ui_mode == UiMode::Edit && self.show_outlines;
            layer.nodes.clear();
        }
        self.lane2.guides.clear();
        // The removed nodes lose focus to the body (the editor is not removed).
        if matches!(self.lane2.focus, Focus::Node(_)) {
            self.lane2.focus = Focus::Body;
        }
        let wanted = self.wanted_page;
        let live: Vec<FieldItem> = if page == wanted
            && matches!(
                self.ui_mode,
                UiMode::Read | UiMode::Annotate | UiMode::Forms
            ) {
            self.fields.clone()
        } else {
            Vec::new()
        };
        if self.direct_active.as_ref().is_some_and(|a| a.page == page) {
            self.place_direct_active(zoom);
        }
        self.dom
            .set_hidden(ids::FIELD_BAR, self.doc.is_none() || self.fields.is_empty());
        let n = self.fields.len();
        self.dom.set_text(
            ids::FIELD_COUNT,
            format!(
                "This page has {n} form field{}.",
                if n == 1 { "" } else { "s" }
            ),
        );
        let radio_focus: Vec<(String, i64)> = reading_order(&live)
            .into_iter()
            .filter(|i| i.kind == FieldKind::Radio)
            .map(|i| (i.name, i.id))
            .collect();
        // P6 U4 §1.4/§9.5: links before fields, so a field over a link stays on top.
        if matches!(self.ui_mode, UiMode::Read | UiMode::Annotate)
            && self.links_page == page
            && page == wanted
        {
            for item in self.links.clone() {
                if item.rect.len() == 4 {
                    let node = self.paint_link_node(&item, page);
                    self.lane2.hit.append(node);
                }
            }
        }
        // fill-anywhere D4: text boxes before fields, so a field over a box stays on top.
        if self.ui_mode == UiMode::Forms
            && page == wanted
            && self.doc.is_some()
            && self.rotation(page) % 360 == 0
        {
            let notes = self.notes.clone();
            for (index, item) in notes
                .iter()
                .filter(|i| i.r#type == "FreeText" && i.editable && i.rect.len() == 4)
                .enumerate()
            {
                let node = self.paint_text_box_target(item, page, index);
                self.lane2.hit.append(node);
            }
            for (sign_index, item) in notes
                .iter()
                .filter(|i| is_signature(i) && i.editable && i.rect.len() == 4)
                .enumerate()
            {
                let node = self.paint_signature_target(item, page, sign_index);
                self.lane2.hit.append(node);
            }
        }
        let mut sorted = live.clone();
        sorted.sort_by(|a, b| {
            a.rect[1]
                .partial_cmp(&b.rect[1])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(
                    a.rect[0]
                        .partial_cmp(&b.rect[0])
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
        });
        let can_move = self.editable() && self.permitted("edit");
        for item in sorted {
            if item.kind == FieldKind::Signature || item.rect.len() != 4 {
                continue;
            }
            let mut node = HitNode::new(NodeKind::Field { item: item.clone() }, &item.rect, zoom);
            if self.highlight_fields {
                node.classes.push("highlighted");
            }
            if item.required {
                node.classes.push("required");
            }
            if item.read_only {
                node.classes.push("readonly");
            }
            node.label = super::form_fields::field_label(&item);
            node.role = if item.kind == FieldKind::Text {
                "textbox"
            } else {
                "button"
            };
            if !can_move {
                let title = self.permission_title(false);
                node.title = if title.is_empty() {
                    "This document is read-only".into()
                } else {
                    title
                };
            } else if let Some(tu) = item.tu.as_ref().filter(|t| !t.is_empty()) {
                node.title = tu.clone();
            }
            if item.kind == FieldKind::Radio {
                let stop = radio_focus
                    .iter()
                    .find(|(name, _)| *name == item.name)
                    .map(|(_, id)| *id);
                node.tab_index = Some(if stop == Some(item.id) { 0 } else { -1 });
            }
            if can_move {
                node.handles = FIELD_HANDLES.to_vec();
            }
            self.lane2.hit.append(node);
        }
        for item in live
            .iter()
            .filter(|i| i.kind == FieldKind::Signature && i.rect.len() == 4)
        {
            // P5 U3 §2.1: an empty field is a "Sign" target, a signed one carries its badge.
            let signed = sig_signed(item);
            let status = self.sig_status(item);
            let label = format!("Signature field {}, {status}", item.name);
            let movable = can_move && !signed;
            let mut node = HitNode::new(
                NodeKind::SigField {
                    item: item.clone(),
                    signed,
                    movable,
                },
                &item.rect,
                zoom,
            );
            node.classes.push("sig-target");
            match status {
                "valid" => node.classes.push("valid"),
                "invalid" => node.classes.push("invalid"),
                "unsupported" => node.classes.push("unsupported"),
                _ => {}
            }
            if status == "valid" && !self.sig_trusted(&item.name) {
                node.classes.push("untrusted");
            }
            node.label = label.clone();
            node.title = label;
            if movable {
                node.handles = FIELD_HANDLES.to_vec();
            }
            self.lane2.hit.append(node);
        }
        // A repaint keeps focus where it was without scrolling the view back to it.
        if let Some(id) = focused_field
            && let Some(i) = self
                .lane2
                .hit
                .nodes
                .iter()
                .position(|n| n.field_id() == Some(id))
        {
            let key = self.lane2.hit.nodes[i].key();
            self.set_focus(Focus::Node(key));
        }
        if self.ui_mode == UiMode::Edit
            && let Some(scan) = self.content_scan.clone()
            && scan.page == page
            && scan.doc_generation.is_none_or(|g| g == self.generation)
        {
            for paragraph in &scan.paragraphs {
                let mut outline = HitNode::new(
                    NodeKind::ParaOutline {
                        paragraph_id: paragraph.paragraph_id.clone(),
                    },
                    &paragraph.bounds,
                    zoom,
                );
                outline.hidden = true;
                self.lane2.hit.append(outline);
            }
            if page == wanted {
                for item in self.object_target_items(&scan) {
                    let node = self.paint_object_target(&item, &scan);
                    self.lane2.hit.append(node);
                }
            }
            let mut painted: Vec<String> = Vec::new();
            for item in &scan.items {
                if item.kind != EditTargetKind::Text
                    || item.bounds.as_ref().is_none_or(|b| b.len() != 4)
                {
                    continue;
                }
                if let Some(id) = &item.paragraph_id {
                    if painted.contains(id) {
                        continue;
                    }
                    painted.push(id.clone());
                }
                let paragraph = item
                    .paragraph_id
                    .as_ref()
                    .and_then(|id| scan.paragraphs.iter().find(|p| &p.paragraph_id == id));
                let bounds = paragraph
                    .map(|p| p.bounds.clone())
                    .unwrap_or_else(|| item.bounds.clone().unwrap_or_default());
                let mut node = HitNode::new(NodeKind::Text { item: item.clone() }, &bounds, zoom);
                if item.paragraph_id.is_some() {
                    node.classes.push("paragraph");
                }
                let candidate = item.support == EditTargetSupport::Candidate;
                if !candidate {
                    node.classes.push("unsupported");
                }
                node.active = self.direct_active.as_ref().is_some_and(|a| {
                    a.kind == DirectKind::Text
                        && match &a.paragraph {
                            Some(p) => Some(&p.paragraph_id) == item.paragraph_id.as_ref(),
                            None => matches!(&a.item, DirectItem::Target(t) if t.target_id == item.target_id),
                        }
                });
                node.label = match paragraph {
                    Some(p) => format!("Paragraph, {} lines", p.lines),
                    None => item
                        .text
                        .clone()
                        .filter(|t| !t.is_empty())
                        .or_else(|| item.reason.clone().filter(|r| !r.is_empty()))
                        .unwrap_or_else(|| "Text block".into()),
                };
                node.title = if candidate {
                    "Edit text".into()
                } else {
                    super::consts::lookup(
                        &super::consts::CONTENT_REASON_TEXT,
                        item.reason.as_deref().unwrap_or(""),
                    )
                    .unwrap_or("Not editable")
                    .to_string()
                };
                if candidate {
                    node.handles = text_handles(item);
                    if item.moveable != Some(true) && item.paragraph_id.is_none() {
                        node.title = match super::consts::lookup(
                            &super::consts::CONTENT_REASON_TEXT,
                            item.move_reason.as_deref().unwrap_or(""),
                        ) {
                            Some(why) => format!("This text can't be moved: {why}"),
                            None => "This text can't be moved".to_string(),
                        };
                    }
                }
                self.lane2.hit.append(node);
            }
        }
        if self.ui_mode == UiMode::Redact && page == wanted {
            let notes = self.notes.clone();
            for (index, item) in notes.iter().enumerate() {
                if item.r#type == "Redact" && item.rect.len() == 4 {
                    let node = self.paint_mark_target(item, page, index);
                    self.lane2.hit.append(node);
                }
            }
        }
        if page == wanted {
            self.restore_direct_focus(refocus);
        }
        if page == wanted
            && let Some(id) = self.direct_focused_box
            && refocus
            && self.direct_active.is_none()
        {
            self.focus_node(&NodeKey::TextBox(id));
        }
        if page == wanted
            && let Some(id) = self.direct_focused_sig
            && refocus
            && self.direct_active.is_none()
        {
            self.focus_node(&NodeKey::Signature(id));
        }
        // §9.16: a paint of another page or zoom, or a rescan that lost the anchor, closes the menu.
        if !self.dom.hidden(ids::OBJECT_MENU) {
            let anchor = self.direct_selection.last().cloned();
            let lost = self.object_menu_kind == ObjectMenuKind::Selection
                && anchor
                    .as_ref()
                    .is_none_or(|a| self.member_node(a).is_none());
            let moved = self
                .object_menu_at
                .is_none_or(|at| at.page != page || at.zoom != self.shown_zoom);
            if moved || lost {
                self.close_object_menu(false);
            } else {
                self.sync_object_menu();
            }
        }
    }

    /// R:8294 `paintLinkTarget(item, page)`'s node (its listeners are in hit.rs). Lane 4 owns
    /// the TS function; the node is built here because the hit layer is this lane's.
    pub fn paint_link_node(&mut self, item: &LinkEntry, page: i64) -> HitNode {
        let annot_id = link_annot_id(item);
        let editing = self.ui_mode == UiMode::Annotate && self.can_annotate() && annot_id > 0;
        let mut node = HitNode::new(
            NodeKind::Link {
                item: item.clone(),
                page,
                editing,
            },
            &item.rect,
            self.shown_zoom,
        );
        node.classes.push("link-target");
        if item.kind == LinkKind::Blocked {
            node.classes.push("blocked");
        }
        if editing {
            node.classes.push("editable");
            node.handles = FIELD_HANDLES.to_vec();
        }
        node.role = "link";
        node.label = super::bookmarks_links::link_label(item);
        node.title = match item.kind {
            LinkKind::Page => format!("Page {}", item.page.unwrap_or(0) + 1),
            LinkKind::Uri => item.uri.clone().unwrap_or_default(),
            _ => format!(
                "Blocked scheme {}",
                super::bookmarks_links::scheme_name(item.uri.as_deref().unwrap_or(""))
            ),
        };
        node
    }

    /// R:3511 `sigStatus()`: empty → signed → the last Verify row status for this field.
    pub fn sig_status(&self, item: &FieldItem) -> &'static str {
        if !sig_signed(item) {
            return "empty";
        }
        match self.verify_by_field.get(&item.name).map(String::as_str) {
            Some("valid") => "valid",
            Some("invalid") => "invalid",
            Some("unsupported") => "unsupported",
            _ => "signed",
        }
    }

    /// R:3517 `sigTrusted()`: the last Verify row's trust for that field.
    pub fn sig_trusted(&self, name: &str) -> bool {
        let rows = self
            .verify_last
            .as_ref()
            .and_then(|v| v.get("signatures"))
            .and_then(Value::as_array);
        rows.is_some_and(|rows| {
            rows.iter().any(|row| {
                row.get("field").and_then(Value::as_str) == Some(name)
                    && row.get("trust").and_then(Value::as_str) == Some("trusted")
            })
        })
    }

    /// R:3523 `signaturesSummary()`: STATE `signatures[]`.
    pub fn signatures_summary(&self) -> Vec<Value> {
        if self.doc.is_none() {
            return Vec::new();
        }
        self.fields
            .iter()
            .filter(|i| i.kind == FieldKind::Signature)
            .map(|i| json!({"field": i.name, "rect": i.rect, "status": self.sig_status(i)}))
            .collect()
    }

    /// R:3529 `objectTargetItems()`: movable placements; one covering ≥ 90 % of the page stays panel-only.
    pub fn object_target_items(&self, scan: &ContentScan) -> Vec<EditTarget> {
        let (Some(size), Some(m)) = (self.page_size(scan.page), scan.transform.as_ref()) else {
            return Vec::new();
        };
        scan.items
            .iter()
            .filter(|item| {
                if item.kind == EditTargetKind::Text
                    || item.support != EditTargetSupport::Candidate
                    || item.bounds.is_none()
                    || !has_op(item, EditObjectAction::Move)
                {
                    return false;
                }
                let Some(placement) = &item.placement else {
                    return false;
                };
                if placement.r#box.len() != 4 {
                    return false;
                }
                let r = view_box(&placement.r#box, m);
                let area = (r[2].min(size.width) - r[0].max(0.0)).max(0.0)
                    * (r[3].min(size.height) - r[1].max(0.0)).max(0.0);
                area < 0.9 * size.width * size.height
            })
            .cloned()
            .collect()
    }

    /// R:3540 `resolveObject()`: the same object after a commit (same id, else the nearest of that kind + name).
    pub fn resolve_object(
        &self,
        scan: &ContentScan,
        item: &EditTarget,
        rect: &[f64],
    ) -> Option<EditTarget> {
        let same: Vec<&EditTarget> = scan
            .items
            .iter()
            .filter(|v| {
                v.kind == item.kind
                    && v.object_name.clone().unwrap_or_default()
                        == item.object_name.clone().unwrap_or_default()
                    && v.placement.is_some()
            })
            .collect();
        let exact = same.iter().find(|v| v.target_id == item.target_id);
        if exact.is_some() || scan.transform.is_none() {
            return exact.map(|v| (*v).clone());
        }
        let m = scan.transform.as_ref().unwrap();
        let mut best: Option<(&EditTarget, f64)> = None;
        for v in same {
            let d = rect_distance(&view_box(&v.placement.as_ref().unwrap().r#box, m), rect);
            if best.is_none_or(|(_, bd)| d < bd) {
                best = Some((v, d));
            }
        }
        best.map(|(v, _)| v.clone())
    }

    /// R:3547 `markSelected(button)`.
    pub fn mark_selected(&mut self, key: Option<&NodeKey>) {
        for n in &mut self.lane2.hit.nodes {
            n.selected = false;
        }
        self.lane2
            .hit
            .nodes
            .retain(|n| !matches!(n.kind, NodeKind::GroupBox | NodeKind::CropFull));
        if let Some(key) = key
            && let Some(i) = self.lane2.hit.find(key)
        {
            self.lane2.hit.nodes[i].selected = true;
        }
    }

    /// R:3553 `markSelection()`: every member is `.selected`; two or more are `.multi` with a group box.
    pub fn mark_selection(&mut self) {
        for n in &mut self.lane2.hit.nodes {
            n.selected = false;
            n.multi = false;
        }
        self.lane2
            .hit
            .nodes
            .retain(|n| n.kind != NodeKind::GroupBox);
        let keys: Vec<NodeKey> = self
            .selection_members()
            .into_iter()
            .map(|e| e.node)
            .collect();
        let multi = keys.len() >= 2;
        for key in &keys {
            if let Some(i) = self.lane2.hit.find(key) {
                self.lane2.hit.nodes[i].selected = true;
                self.lane2.hit.nodes[i].multi = multi;
            }
        }
        if multi {
            self.paint_group_box();
        }
    }

    /// R:3561 `paintGroupBox()`: one frame on the union with 8 handles (no grip) for an all-object group.
    pub fn paint_group_box(&mut self) {
        let targets = self.selection_targets();
        let members = self.selection_members();
        if members.len() < 2 {
            return;
        }
        let union = union_rect(&members.iter().map(|e| e.rect.clone()).collect::<Vec<_>>());
        let mut node = HitNode::new(NodeKind::GroupBox, &union, self.shown_zoom);
        if targets.is_some_and(|t| t.len() >= 2) && !self.selection_mixed() && self.can_edit() {
            node.handles = FIELD_HANDLES[1..].to_vec();
        }
        self.lane2.hit.append(node);
    }

    /// R:3589 `snapOthers(exclude)`: every painted object's view rect except the gesture's own.
    pub fn snap_others(&self, exclude: &[String]) -> Vec<Vec<f64>> {
        let Some(scan) = &self.content_scan else {
            return Vec::new();
        };
        let Some(m) = scan.transform.as_ref() else {
            return Vec::new();
        };
        if scan.page != self.wanted_page {
            return Vec::new();
        }
        self.object_target_items(scan)
            .into_iter()
            .filter(|i| !exclude.contains(&i.target_id))
            .filter_map(|i| i.placement.map(|p| view_box(&p.r#box, m)))
            .collect()
    }

    /// R:3595 `objectNode(targetId)`.
    pub fn object_node(&self, target_id: &str) -> Option<usize> {
        self.lane2.hit.find(&NodeKey::Object(target_id.to_string()))
    }

    /// R:3598 `selectionTargets()`: the painted object members with their items and view rects;
    /// None when any member is not painted.
    pub fn selection_targets(&self) -> Option<Vec<ObjectTarget>> {
        let scan = self.content_scan.as_ref()?;
        let m = scan.transform.as_ref()?;
        if scan.page != self.wanted_page || self.direct_selection.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        for member in &self.direct_selection {
            let item = scan
                .items
                .iter()
                .find(|v| v.target_id == member.target_id)?;
            let placement = item.placement.as_ref()?;
            self.object_node(&member.target_id)?;
            out.push(ObjectTarget {
                member: member.clone(),
                item: item.clone(),
                rect: view_box(&placement.r#box, m),
            });
        }
        Some(out)
    }

    /// `try { localStorage.setItem(key, String(on)); } catch {}`.
    fn host_store_toggle(&mut self, key: &str, on: bool) {
        self.local_storage_set(key, if on { "true" } else { "false" });
    }
}

/// The handles of `addFieldHandles` (grip first, then the 8 sides).
pub const FIELD_HANDLES: [&str; 9] = ["grip", "n", "ne", "e", "se", "s", "sw", "w", "nw"];

/// R:3209 `addTextHandles()`: a grip for a moveable run or a paragraph, e/w for a paragraph,
/// corners when the font size is known.
pub fn text_handles(item: &EditTarget) -> Vec<&'static str> {
    let mut handles = Vec::new();
    if item.moveable == Some(true) || item.paragraph_id.is_some() {
        handles.push("grip");
    }
    if item.paragraph_id.is_some() {
        handles.extend(["e", "w"]);
    }
    if item.font_size.is_some() {
        handles.extend(["ne", "se", "sw", "nw"]);
    }
    handles
}

/// R:3142 `placeBox()` for an inline editor: its box and font size.
pub fn place_editor(editor: &mut EditorState, rect: &[f64], zoom: f64) {
    let b = editor_box(rect, zoom);
    editor.left = b.left;
    editor.top = b.top;
    editor.width = b.width;
    editor.height = b.height;
    if editor.class_name.contains("note-inline-editor") {
        // Item 6: a note is read on screen, not printed at page size.
        editor.font_px = 13.0;
    } else if editor.class_name.contains("inline-editor") {
        editor.font_px = if editor.class_name.contains("text-inline-editor") {
            let size = match editor.font_size.as_deref() {
                Some(text) => js_number(text),
                None => b.font_px / zoom,
            };
            10.0f64.max(400.0f64.min(size * zoom))
        } else {
            b.font_px
        };
    }
}

/// R:3136 `growParagraphEditor()`: at least the box's height (the browser grew it to a wrapped
/// draft; text is not measured here).
pub fn grow_paragraph_editor(editor: &mut EditorState, min_px: Option<f64>) {
    if let Some(min) = min_px {
        editor.min_height = Some(js_num(min));
    }
    editor.height = or_number(editor.min_height.as_deref(), 0.0);
}

/// The CSS box of an editor (`cssBox(editor)`).
pub fn editor_css(editor: &EditorState) -> CssBox {
    CssBox {
        left: editor.left,
        top: editor.top,
        width: editor.width,
        height: editor.height,
    }
}

/// R:5118 `paragraphText(paragraph, scan)`.
pub fn paragraph_text(paragraph: &ParagraphItem, scan: Option<&ContentScan>) -> String {
    let Some(scan) = scan else {
        return String::new();
    };
    scan.items
        .iter()
        .filter(|i| paragraph.target_ids.contains(&i.target_id))
        .map(|i| i.text.clone().unwrap_or_default())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `EvTarget` of a node for a synthetic dispatch.
pub fn body_of(index: usize) -> EvTarget {
    EvTarget::Node {
        index,
        part: super::hit::Part::Body,
    }
}

/// Round a rect to 4 decimals (`rect.map(v => Math.round(v * 1e4) / 1e4)`, no `+ 0`).
pub fn round_rect4(rect: &[f64]) -> Vec<f64> {
    rect.iter()
        .map(|v| crate::js::js_round(v * 1e4) / 1e4)
        .collect()
}

/// `round4` without the `-0` fix, kept apart for callers that print through JSON (where -0 → 0 anyway).
pub fn r4(v: f64) -> f64 {
    round4(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_helpers() {
        assert_eq!(
            map_rect(
                &[10.0, 10.0, 20.0, 20.0],
                &[0.0, 0.0, 100.0, 100.0],
                &[0.0, 0.0, 200.0, 50.0]
            ),
            vec![20.0, 5.0, 40.0, 10.0]
        );
        assert_eq!(
            turn_rect(&[0.0, 0.0, 20.0, 10.0], "rotateCw", 10.0, 5.0),
            vec![5.0, -5.0, 15.0, 15.0]
        );
        assert_eq!(
            turn_rect(&[0.0, 0.0, 20.0, 10.0], "flipH", 30.0, 5.0),
            vec![40.0, 0.0, 60.0, 10.0]
        );
        assert_eq!(
            union_rect(&[vec![0.0, 5.0, 10.0, 10.0], vec![5.0, 0.0, 20.0, 8.0]]),
            vec![0.0, 0.0, 20.0, 10.0]
        );
        assert_eq!(js_number(" 12 "), 12.0);
        assert_eq!(js_number(""), 0.0);
        assert!(js_number("x").is_nan());
    }

    #[test]
    fn text_handles_follow_the_run() {
        let mut item = EditTarget {
            moveable: Some(true),
            font_size: Some(12.0),
            ..EditTarget::default()
        };
        assert_eq!(text_handles(&item), vec!["grip", "ne", "se", "sw", "nw"]);
        item.paragraph_id = Some("p1".into());
        item.font_size = None;
        assert_eq!(text_handles(&item), vec!["grip", "e", "w"]);
    }

    #[test]
    fn editors_place_like_place_box() {
        let mut editor = EditorState {
            class_name: "inline-editor text-inline-editor".into(),
            font_size: Some("12".into()),
            ..EditorState::default()
        };
        place_editor(&mut editor, &[10.0, 20.0, 110.0, 40.0], 1.5);
        assert_eq!(
            (
                editor.left,
                editor.top,
                editor.width,
                editor.height,
                editor.font_px
            ),
            (15.0, 30.0, 150.0, 30.0, 18.0)
        );
        let mut note = EditorState {
            class_name: "inline-editor note-inline-editor".into(),
            ..EditorState::default()
        };
        place_editor(&mut note, &[0.0, 0.0, 200.0, 72.0], 2.0);
        assert_eq!(note.font_px, 13.0);
        grow_paragraph_editor(&mut note, Some(50.5));
        assert_eq!(note.height, 50.5);
    }

    #[test]
    fn a_gesture_needs_three_pixels_then_commits() {
        let mut s = Session::new();
        s.doc = Some(OpenDoc {
            page_count: 1,
            pages: vec![PageSize {
                width: 600.0,
                height: 800.0,
            }],
            rotations: vec![0],
            ..OpenDoc::default()
        });
        s.shown_zoom = 1.0;
        s.lane2.canvas_css = [600.0, 800.0];
        let options = RectGesture {
            rect: vec![10.0, 10.0, 60.0, 60.0],
            id: TargetRef::Number(5),
            handle: "grip".into(),
            min: 1.0,
            can: true,
            ..RectGesture::default()
        };
        let mut ev = PtrEvent {
            client_x: 20.0,
            client_y: 20.0,
            ..PtrEvent::default()
        };
        s.start_rect_gesture(&mut ev, None, options);
        assert!(ev.stopped && ev.prevented);
        s.gesture_pointermove(&PtrEvent {
            client_x: 21.0,
            client_y: 21.0,
            ..PtrEvent::default()
        });
        assert!(s.direct_gesture.is_none());
        s.gesture_pointermove(&PtrEvent {
            client_x: 30.0,
            client_y: 25.0,
            ..PtrEvent::default()
        });
        let g = s.direct_gesture.clone().unwrap();
        assert_eq!(
            (g.kind, g.rect),
            (GestureKind::Move, vec![20.0, 15.0, 70.0, 65.0])
        );
        // Escape cancels: nothing commits, the release's click is swallowed.
        let mut esc = KeyEv::new("Escape", false, false, false);
        s.gesture_keydown(&mut esc);
        assert!(s.direct_gesture.is_none() && s.lane2.gesture.is_none() && s.add_blocked);
        assert!(s.lane2.swallow_after_release);
    }
}
