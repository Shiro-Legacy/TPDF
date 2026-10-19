//! The hit layer as data, focus, and DOM event routing for the on-page direct manipulation
//! (renderer.ts R:2768-5347 and the hit-layer listeners R:11017-11080).
//!
//! The Electron renderer built `#hit-layer` out of buttons, handle spans and inline editors and
//! let the browser hit-test, focus and propagate events. Here [`HitLayer`] keeps the same nodes
//! as data (`paint_hits` rebuilds it), positioned in CSS px exactly as `placeBox` positioned the
//! elements (page points × `shownZoom`), so STATE `box` values and the drive's pointer events at
//! page points reach the same targets: the same z-order (`.selected` z 2, `.active` z 3, handles
//! z 3, the inline editor z 2), the same handle geometry (8 px squares at −4 px, the 14 px grip
//! at −15 px, inside a 1 px border) and the same `pointer-events` rules (`#hit-layer.live`,
//! `.active`, the 6 px ring of a wrapping form).
//!
//! Client coordinates are CSS px from the canvas's top-left corner (the canvas box is
//! `round(width × zoom)` by `round(height × zoom)`, as `paint` sizes it).
//!
//! New entry points for the UI (not TS names): [`Session::pointer`], [`Session::key`],
//! [`Session::context_menu`], [`Session::double_click`], [`Session::editor_input`].
#![allow(dead_code)]

use super::Session;
use super::dom_ids as ids;
use super::nav::{EditorBox, editor_box, is_pressed};
use super::types::*;
use crate::engine::{EngineResult, Refusal};
use serde_json::Value;
use tandem_protocol::engine::LinkKind;

// ------------------------------------------------------------------ data

/// `{left, top, width, height}` CSS px (an element's `style`, read back by `cssBox`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CssBox {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

impl From<EditorBox> for CssBox {
    fn from(b: EditorBox) -> Self {
        CssBox {
            left: b.left,
            top: b.top,
            width: b.width,
            height: b.height,
        }
    }
}

impl CssBox {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.left && x < self.left + self.width && y >= self.top && y < self.top + self.height
    }
}

/// What a painted node stands for, with what its listeners captured at paint time.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// R:8294 `paintLinkTarget` (`.link-target`); `editing` = Annotate, may annotate, annotId > 0.
    Link {
        item: LinkEntry,
        page: i64,
        editing: bool,
    },
    /// R:4620 `paintTextBoxTarget` (`.textbox-target`); `can` = canAnnotate() at paint.
    TextBox {
        item: AnnotItem,
        page: i64,
        can: bool,
    },
    /// R:4962 `paintSignatureTarget` (`.signature-target`).
    Signature {
        item: AnnotItem,
        page: i64,
        can: bool,
    },
    /// R:3423 a form field widget (`.hit-target[data-field-id]`); `handles` = editable + edit.
    Field { item: FieldItem },
    /// R:3448 a signature field (`.sig-target`).
    SigField {
        item: FieldItem,
        signed: bool,
        movable: bool,
    },
    /// R:3481 `.para-outline` (hidden until its paragraph target is hovered).
    ParaOutline { paragraph_id: String },
    /// R:4222 `paintObjectTarget` (`.object-target`); `rect` is the view rect at paint.
    Object {
        item: EditTarget,
        page: i64,
        rect: Vec<f64>,
        wrapping: bool,
        can: bool,
    },
    /// R:3484 a text run or paragraph (`.text-target`).
    Text { item: EditTarget },
    /// R:4573 `paintMarkTarget` (`.redact-target`).
    Mark {
        item: AnnotItem,
        page: i64,
        can: bool,
    },
    /// R:3559 `paintGroupBox` (`.group-box`, never a hit target itself).
    GroupBox,
    /// R:4489 `paintCrop`'s `.crop-full` frame.
    CropFull,
}

/// A node's identity across repaints (its class plus its `data-*` id).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKey {
    /// `data-link-id`.
    Link(i64),
    TextBox(i64),
    Signature(i64),
    Field(i64),
    SigField(i64),
    ParaOutline(String),
    Object(String),
    Text(String),
    Mark(i64),
    GroupBox,
    CropFull,
}

/// One element of `#hit-layer`.
#[derive(Clone, Debug, PartialEq)]
pub struct HitNode {
    pub kind: NodeKind,
    /// Document order (append order).
    pub seq: u64,
    /// The y-down page rect the node was last placed at, and its CSS box.
    pub rect: Vec<f64>,
    pub css: CssBox,
    pub selected: bool,
    pub multi: bool,
    pub active: bool,
    pub single_run_active: bool,
    pub cropping: bool,
    /// `[data-handle]` children in DOM order ("grip" first when present).
    pub handles: Vec<&'static str>,
    /// Other classes, for the UI (`highlighted`, `required`, `readonly`, `paragraph`, …).
    pub classes: Vec<&'static str>,
    pub label: String,
    pub title: String,
    pub role: &'static str,
    /// `tabIndex` when the renderer set one (radio roving focus).
    pub tab_index: Option<i64>,
    pub hidden: bool,
}

impl HitNode {
    pub fn new(kind: NodeKind, rect: &[f64], zoom: f64) -> Self {
        HitNode {
            kind,
            seq: 0,
            rect: rect.to_vec(),
            css: editor_box(rect, zoom).into(),
            selected: false,
            multi: false,
            active: false,
            single_run_active: false,
            cropping: false,
            handles: Vec::new(),
            classes: Vec::new(),
            label: String::new(),
            title: String::new(),
            role: "button",
            tab_index: None,
            hidden: false,
        }
    }

    pub fn key(&self) -> NodeKey {
        match &self.kind {
            NodeKind::Link { item, .. } => NodeKey::Link(item.id),
            NodeKind::TextBox { item, .. } => NodeKey::TextBox(item.id),
            NodeKind::Signature { item, .. } => NodeKey::Signature(item.id),
            NodeKind::Field { item } => NodeKey::Field(item.id),
            NodeKind::SigField { item, .. } => NodeKey::SigField(item.id),
            NodeKind::ParaOutline { paragraph_id } => NodeKey::ParaOutline(paragraph_id.clone()),
            NodeKind::Object { item, .. } => NodeKey::Object(item.target_id.clone()),
            NodeKind::Text { item } => NodeKey::Text(item.target_id.clone()),
            NodeKind::Mark { item, .. } => NodeKey::Mark(item.id),
            NodeKind::GroupBox => NodeKey::GroupBox,
            NodeKind::CropFull => NodeKey::CropFull,
        }
    }

    /// `.hit-target` (group boxes, outlines and crop frames are not).
    pub fn is_target(&self) -> bool {
        !matches!(
            self.kind,
            NodeKind::ParaOutline { .. } | NodeKind::GroupBox | NodeKind::CropFull
        )
    }

    /// `data-annot-id`.
    pub fn annot_id(&self) -> Option<i64> {
        match &self.kind {
            NodeKind::Link { item, .. } => Some(link_annot_id(item)),
            NodeKind::TextBox { item, .. }
            | NodeKind::Signature { item, .. }
            | NodeKind::Mark { item, .. } => Some(item.id),
            _ => None,
        }
    }

    /// `data-field-id` (fields and signature fields).
    pub fn field_id(&self) -> Option<i64> {
        match &self.kind {
            NodeKind::Field { item } | NodeKind::SigField { item, .. } => Some(item.id),
            _ => None,
        }
    }

    /// `data-target-id` (objects and text targets).
    pub fn target_id(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::Object { item, .. } | NodeKind::Text { item } => Some(&item.target_id),
            _ => None,
        }
    }

    /// The padding box (absolutely positioned children are placed inside the 1 px border).
    fn padding_box(&self) -> CssBox {
        let b = if self.is_target() { 1.0 } else { 0.0 };
        CssBox {
            left: self.css.left + b,
            top: self.css.top + b,
            width: self.css.width - 2.0 * b,
            height: self.css.height - 2.0 * b,
        }
    }

    /// The CSS box of a `[data-handle]` child (styles.css:963-975).
    pub fn handle_box(&self, name: &str) -> CssBox {
        let p = self.padding_box();
        if name == "grip" {
            return CssBox {
                left: p.left - 15.0,
                top: p.top - 15.0,
                width: 14.0,
                height: 14.0,
            };
        }
        let x = if name.contains('w') {
            p.left - 4.0
        } else if name.contains('e') {
            p.left + p.width - 4.0
        } else {
            p.left + p.width / 2.0 - 4.0
        };
        let y = if name.contains('n') {
            p.top - 4.0
        } else if name.contains('s') {
            p.top + p.height - 4.0
        } else {
            p.top + p.height / 2.0 - 4.0
        };
        CssBox {
            left: x,
            top: y,
            width: 8.0,
            height: 8.0,
        }
    }

    /// The 6 px border ring of a wrapping form (`.ring`'s clip-path).
    fn ring_contains(&self, x: f64, y: f64) -> bool {
        let p = self.padding_box();
        let inner = CssBox {
            left: p.left + 6.0,
            top: p.top + 6.0,
            width: p.width - 12.0,
            height: p.height - 12.0,
        };
        p.contains(x, y) && !inner.contains(x, y)
    }
}

/// `#hit-layer`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HitLayer {
    /// `.live` (`layerLive()` at the last paint).
    pub live: bool,
    /// `.outlined` (Edit mode with "Outline objects" on).
    pub outlined: bool,
    pub width: f64,
    pub height: f64,
    pub nodes: Vec<HitNode>,
    /// The next document-order number.
    pub next_seq: u64,
    /// The open inline editor's document-order number.
    pub editor_seq: u64,
}

impl HitLayer {
    pub fn append(&mut self, mut node: HitNode) -> usize {
        self.next_seq += 1;
        node.seq = self.next_seq;
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    pub fn find(&self, key: &NodeKey) -> Option<usize> {
        self.nodes.iter().position(|n| &n.key() == key)
    }

    pub fn group_box(&self) -> Option<usize> {
        self.nodes.iter().position(|n| n.kind == NodeKind::GroupBox)
    }
}

/// `document.activeElement`, as far as the logic reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Focus {
    /// `document.body` (nothing focused).
    #[default]
    Body,
    /// `#page` (`pageRegion`).
    Page,
    /// A hit-layer target.
    Node(NodeKey),
    /// The open inline editor (by its identity, see `Lane2State::editor_id`).
    Editor(u64),
    /// `#object-menu` itself, or one of its items (by menu key).
    Menu,
    MenuItem(&'static str),
    /// A text entry outside the page (an `<input>` of a text type, a `<textarea>`, a `<select>`).
    TextEntry(String),
    /// Any other element (a button, a checkbox), by its index.html id.
    Element(String),
}

/// One part of a hit-layer node an event lands on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Part {
    #[default]
    Body,
    Ring,
    Handle(&'static str),
}

/// An event's target.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum EvTarget {
    /// Outside the page region (the body, a toolbar).
    #[default]
    Outside,
    /// `#page` or anything in it that is not in the hit layer (canvas, text layer).
    Page,
    /// `window` itself (the drive dispatches a drag's move, release and Escape there).
    Window,
    /// `#hit-layer` itself.
    HitLayer,
    /// A node (index into `HitLayer::nodes` at dispatch time).
    Node {
        index: usize,
        part: Part,
    },
    /// The open inline editor.
    Editor,
    /// `#object-menu` or one of its items.
    Menu,
    MenuItem(&'static str),
}

impl EvTarget {
    /// `hitLayer.contains(event.target)`.
    pub fn in_hit_layer(&self) -> bool {
        matches!(
            self,
            EvTarget::HitLayer | EvTarget::Node { .. } | EvTarget::Editor
        )
    }
    /// `pageRegion.contains(event.target)` (the object menu lives in `#page`).
    pub fn in_page(&self) -> bool {
        !matches!(self, EvTarget::Outside | EvTarget::Window)
    }
    /// `objectMenu.contains(event.target)`.
    pub fn in_menu(&self) -> bool {
        matches!(self, EvTarget::Menu | EvTarget::MenuItem(_))
    }
    /// `event.target.closest(".hit-handle")`.
    pub fn on_handle(&self) -> bool {
        matches!(
            self,
            EvTarget::Node {
                part: Part::Handle(_),
                ..
            }
        )
    }
}

/// A pointer or mouse event (`PointerEvent` / `MouseEvent`): client CSS px, modifiers, and the
/// propagation flags the listeners set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PtrEvent {
    pub client_x: f64,
    pub client_y: f64,
    /// 0 primary, 2 secondary.
    pub button: i64,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
    pub ctrl: bool,
    pub target: EvTarget,
    /// `event.detail` (0 for a keyboard-made click).
    pub detail: i64,
    pub stopped: bool,
    pub prevented: bool,
}

impl PtrEvent {
    /// `event.target.closest(".hit-target, .hit-handle")` (R:11133).
    pub fn on_target_or_handle(&self, layer: &HitLayer) -> bool {
        match &self.target {
            EvTarget::Node { index, part } => {
                matches!(part, Part::Handle(_))
                    || layer.nodes.get(*index).is_some_and(HitNode::is_target)
            }
            _ => false,
        }
    }
}

/// A `KeyboardEvent`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyEv {
    pub key: String,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
    pub ctrl: bool,
    pub composing: bool,
    pub cancelable: bool,
    pub prevented: bool,
    pub stopped: bool,
    pub target: EvTarget,
}

impl KeyEv {
    pub fn new(key: &str, shift: bool, alt: bool, meta: bool) -> Self {
        KeyEv {
            key: key.to_string(),
            shift,
            alt,
            meta,
            cancelable: true,
            ..KeyEv::default()
        }
    }
    /// `event.preventDefault()`.
    pub fn prevent(&mut self) {
        if self.cancelable {
            self.prevented = true;
        }
    }
    /// `cmdKey(event)`.
    pub fn cmd(&self) -> bool {
        cmd_key(self.meta, self.ctrl)
    }
    pub fn is_arrow(&self) -> bool {
        super::consts::ARROW_KEYS.contains(&self.key.as_str())
    }
}

/// R:202 `cmdKey`: ⌘ on macOS, Ctrl elsewhere (a drive `Meta+` key sets metaKey everywhere).
pub fn cmd_key(meta: bool, ctrl: bool) -> bool {
    meta || (!super::consts::IS_MAC && ctrl)
}

/// R:8251 `linkAnnotId(item)`.
pub fn link_annot_id(item: &LinkEntry) -> i64 {
    item.annot_id
}

/// The page this lane's code keeps beside the session's own fields.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane2State {
    pub hit: HitLayer,
    pub focus: Focus,
    /// The canvas CSS size (`round(width × zoom)`); 300×150 before the first paint.
    pub canvas_css: [f64; 2],
    /// The last render was refused as `skipped` (the canvas is cleared).
    pub canvas_blank: bool,
    /// The live move/resize gesture's window listeners.
    pub gesture: Option<super::direct::LiveGesture>,
    /// R:3257 `swallowNextClick`: the next click is not a click.
    pub swallow_click: bool,
    /// R:3263 `swallowClickAfterRelease`: the next pointerup arms `swallow_click`.
    pub swallow_after_release: bool,
    /// The open editor's identity (TS compares `directActive === active`); bumped by
    /// `setup_inline_events`, which every editor-opening path calls.
    pub editor_id: u64,
    /// `setupInlineEvents(editor, multiline)`'s `multiline`.
    pub editor_multiline: bool,
    /// A body press's client point, per target (`pressAt` of the sig-field and object closures).
    pub press_at: Option<(NodeKey, [f64; 2])>,
    /// `.ghost` outlines of a duplicate drag, and `.guide` lines (for the UI).
    pub ghosts: Vec<Vec<f64>>,
    pub guides: Vec<SnapGuide>,
    /// `:hover` (only the UI's pointer sets it).
    pub hover: Option<NodeKey>,
    /// `window.getSelection().isCollapsed` (the UI reports a text selection).
    pub text_collapsed: bool,
    /// A blur committed the open editor during the last focus change.
    pub blur_committed: bool,
    /// `directGestureWork` is running.
    pub gesture_work: bool,
    /// The UI's pressed target and its node key (for the click a release makes).
    pub down: Option<(EvTarget, Option<(NodeKey, Part)>)>,
    /// The signature pad canvas's CSS size (`clientWidth || 520`, `clientHeight || 180`).
    pub pad_size: [f64; 2],
    /// The editor a focus change blurred and committed during the current event (the TS
    /// commit was still in flight when the click's handlers ran, so they saw it open).
    pub blurred: Option<DirectActive>,
    /// The Fields list row of an anchored field (`aria-current`), and the Annotations row of
    /// the focused mark (R:4130 `syncNoteCurrent`): for the UI.
    pub field_row_current: Option<String>,
    pub note_row_current: Option<i64>,
    /// The add editor's CSS font and colour.
    pub editor_font: Option<super::editors::EditorFont>,
    /// The text layer of the last paint (for the UI).
    pub text_layer: super::editors::TextLayer,
    /// The open editor's `aria-label`.
    pub editor_label: String,
    /// Where the object menu opened (client px; the UI clamps it inside the viewport).
    pub menu_at: [f64; 2],
}

impl Default for Lane2State {
    fn default() -> Self {
        Lane2State {
            hit: HitLayer::default(),
            focus: Focus::Body,
            canvas_css: [300.0, 150.0],
            canvas_blank: false,
            gesture: None,
            swallow_click: false,
            swallow_after_release: false,
            editor_id: 0,
            editor_multiline: false,
            press_at: None,
            ghosts: Vec::new(),
            guides: Vec::new(),
            hover: None,
            text_collapsed: true,
            blur_committed: false,
            gesture_work: false,
            down: None,
            pad_size: [520.0, 180.0],
            blurred: None,
            field_row_current: None,
            note_row_current: None,
            menu_at: [0.0, 0.0],
            editor_label: String::new(),
            editor_font: None,
            text_layer: super::editors::TextLayer::default(),
        }
    }
}

impl Session {
    // ------------------------------------------------------------------ requests

    /// `request(message)`: every engine call of this lane goes through here.
    pub(crate) fn rq(&mut self, message: Value) -> EngineResult {
        self.engine.request(message, Vec::new())
    }

    /// `request(message, transfer)` with binary inputs (`{"$buffer": i}` placeholders).
    pub(crate) fn rq_with(&mut self, message: Value, inputs: Vec<Vec<u8>>) -> EngineResult {
        self.engine.request(message, inputs)
    }

    // ------------------------------------------------------------------ geometry

    /// The canvas box's page-pt-per-CSS-px for the shown page (`size.width / box.width`).
    pub fn css_per_pt(&self, page: i64) -> Option<(f64, f64)> {
        let size = self.doc.as_ref()?.pages.get(page as usize)?;
        let [w, h] = self.lane2.canvas_css;
        if w < 1.0 || h < 1.0 {
            return None;
        }
        Some((w / size.width, h / size.height))
    }

    /// Page points → client CSS px through the canvas box (unrounded).
    pub fn client_of(&self, page: i64, x: f64, y: f64) -> Option<(f64, f64)> {
        let (sx, sy) = self.css_per_pt(page)?;
        Some((x * sx, y * sy))
    }

    /// R:10045 `pagePoint(event)`: client px → page points of the shown page.
    pub fn page_point_of(&self, ev: &PtrEvent) -> Option<[f64; 2]> {
        let size = self
            .doc
            .as_ref()?
            .pages
            .get(self.shown_page.max(0) as usize)?;
        let [w, h] = self.lane2.canvas_css;
        if w < 1.0 || h < 1.0 {
            return None;
        }
        Some([
            ev.client_x * (size.width / w),
            ev.client_y * (size.height / h),
        ])
    }

    /// Whether a node's handles are displayed (styles.css:934-966).
    fn handles_shown(&self, node: &HitNode) -> bool {
        if node.kind == NodeKind::GroupBox {
            return self.lane2.hit.live;
        }
        if node.selected && node.multi {
            return false;
        }
        let key = node.key();
        let focused = self.lane2.focus == Focus::Node(key.clone());
        node.selected || node.active || focused || self.lane2.hover.as_ref() == Some(&key)
    }

    /// `document.elementsFromPoint(x, y)` restricted to the hit layer, topmost first.
    pub fn hit_test_all(&self, x: f64, y: f64) -> Vec<EvTarget> {
        let layer = &self.lane2.hit;
        // (z, seq, sub) → topmost is the largest.
        let mut hits: Vec<((i64, u64, u64), EvTarget)> = Vec::new();
        for (index, node) in layer.nodes.iter().enumerate() {
            if node.hidden {
                continue;
            }
            let z = if node.active {
                3
            } else if node.selected {
                2
            } else {
                0
            };
            let is_target = node.is_target();
            let wrapping = matches!(node.kind, NodeKind::Object { wrapping: true, .. });
            // `#hit-layer:not(.live) .hit-target` and `.active` take no pointer; outlines, frames and
            // the group box never do.
            let body_events = is_target && layer.live && !node.active && !wrapping;
            if body_events && node.css.contains(x, y) {
                hits.push((
                    (z, node.seq, 0),
                    EvTarget::Node {
                        index,
                        part: Part::Body,
                    },
                ));
            }
            if wrapping && layer.live && node.ring_contains(x, y) {
                hits.push((
                    (z, node.seq, 1),
                    EvTarget::Node {
                        index,
                        part: Part::Ring,
                    },
                ));
            }
            let handle_events = if node.kind == NodeKind::GroupBox {
                layer.live
            } else {
                is_target && (layer.live || node.active)
            };
            if handle_events && self.handles_shown(node) {
                let handle_z = if z == 0 { 3 } else { z };
                for (order, name) in node.handles.iter().enumerate() {
                    if node.single_run_active && (*name == "e" || *name == "w") {
                        continue;
                    }
                    if node.handle_box(name).contains(x, y) {
                        hits.push((
                            (handle_z, node.seq, 2 + order as u64),
                            EvTarget::Node {
                                index,
                                part: Part::Handle(name),
                            },
                        ));
                    }
                }
            }
        }
        if let Some(active) = &self.direct_active {
            let e = &active.editor;
            let b = CssBox {
                left: e.left,
                top: e.top,
                width: e.width,
                height: e.height,
            };
            if b.contains(x, y) {
                let z = if e.class_name.contains("link-editor") {
                    4
                } else {
                    2
                };
                hits.push(((z, layer.editor_seq, 0), EvTarget::Editor));
            }
        }
        hits.sort_by(|a, b| b.0.cmp(&a.0));
        hits.into_iter().map(|(_, t)| t).collect()
    }

    /// The topmost element at a client point: a hit-layer element, else the page.
    pub fn hit_test(&self, x: f64, y: f64) -> EvTarget {
        self.hit_test_all(x, y)
            .into_iter()
            .next()
            .unwrap_or(EvTarget::Page)
    }

    // ------------------------------------------------------------------ focus

    /// `element.focus()`: moves focus; leaving the open editor is its blur, which commits it
    /// (R:5147 `directFinishWork = finishDirect()`). Returns whether that blur committed.
    pub fn set_focus(&mut self, focus: Focus) -> bool {
        if self.lane2.focus == focus {
            return false;
        }
        let was = std::mem::replace(&mut self.lane2.focus, focus);
        if let Focus::Editor(id) = was
            && self.direct_active.is_some()
            && self.lane2.editor_id == id
        {
            self.lane2.blur_committed = true;
            self.lane2.blurred = self.direct_active.clone();
            self.finish_direct();
            return true;
        }
        false
    }

    /// `pageRegion.focus({preventScroll: true})`.
    pub fn focus_page(&mut self) {
        self.set_focus(Focus::Page);
    }

    /// `node.focus()` for the node at `key`, when it is painted.
    pub fn focus_node(&mut self, key: &NodeKey) -> bool {
        if self.lane2.hit.find(key).is_none() {
            return false;
        }
        self.set_focus(Focus::Node(key.clone()));
        true
    }

    /// `directActive.editor.focus()`.
    pub fn focus_editor(&mut self) {
        let id = self.lane2.editor_id;
        self.set_focus(Focus::Editor(id));
    }

    /// The focused hit-layer node, when focus is on one that is painted.
    pub fn focused_node(&self) -> Option<usize> {
        match &self.lane2.focus {
            Focus::Node(key) => self.lane2.hit.find(key),
            _ => None,
        }
    }

    /// `hitLayer.contains(document.activeElement)` (the editor is in the layer too).
    pub fn focus_in_hit_layer(&self) -> bool {
        match &self.lane2.focus {
            Focus::Node(key) => self.lane2.hit.find(key).is_some(),
            Focus::Editor(id) => self.direct_active.is_some() && *id == self.lane2.editor_id,
            _ => false,
        }
    }

    /// Where a key goes: the focused element.
    fn focus_target(&self) -> EvTarget {
        match &self.lane2.focus {
            Focus::Node(key) => match self.lane2.hit.find(key) {
                Some(index) => EvTarget::Node {
                    index,
                    part: Part::Body,
                },
                None => EvTarget::Outside,
            },
            Focus::Editor(id) if self.direct_active.is_some() && *id == self.lane2.editor_id => {
                EvTarget::Editor
            }
            Focus::Page => EvTarget::Page,
            Focus::Menu => EvTarget::Menu,
            Focus::MenuItem(name) => EvTarget::MenuItem(name),
            _ => EvTarget::Outside,
        }
    }

    // ------------------------------------------------------------------ dispatch

    /// The node an event target stands for, by key (a node index is only good until the next
    /// paint; a listener may repaint while the event is still being dispatched).
    pub fn target_key(&self, target: &EvTarget) -> Option<(NodeKey, Part)> {
        match target {
            EvTarget::Node { index, part } => {
                self.lane2.hit.nodes.get(*index).map(|n| (n.key(), *part))
            }
            _ => None,
        }
    }

    /// The target again after a repaint: the node with the same key, else the layer itself (in
    /// the TS the detached element kept its listeners, but it had not been repainted yet).
    pub fn refresh_target(&self, target: &mut EvTarget, saved: &Option<(NodeKey, Part)>) {
        if let Some((key, part)) = saved {
            *target = match self.lane2.hit.find(key) {
                Some(index) => EvTarget::Node { index, part: *part },
                None => EvTarget::HitLayer,
            };
        }
    }

    /// A `pointerdown` at `ev.target`, through the capture listeners, the target and the bubble.
    pub(crate) fn dispatch_pointerdown(&mut self, ev: &mut PtrEvent) {
        let saved = self.target_key(&ev.target);
        // Document capture: the object menu's outside close (R:4118), crop's outside press (R:4497).
        self.on_document_pointerdown_menu(ev);
        self.on_document_pointerdown_crop(ev);
        self.refresh_target(&mut ev.target, &saved);
        if ev.target.in_menu() {
            // R:4106: the menu's own pointerdown stops there.
            ev.stopped = true;
        }
        if !ev.stopped && ev.target.in_hit_layer() {
            self.on_hit_layer_pointerdown_capture(ev);
        }
        self.refresh_target(&mut ev.target, &saved);
        if !ev.stopped
            && let EvTarget::Node { index, part } = ev.target.clone()
        {
            self.node_pointerdown(index, part, ev);
        }
        if !ev.stopped && ev.target.in_page() && !ev.target.in_menu() {
            self.on_page_pointerdown(ev);
        }
        // `event === objectMenuDismiss` / `cropDismiss` hold only for this one press.
        self.object_menu_dismiss = false;
        self.crop_dismiss = false;
    }

    /// A `pointermove`: `on_window` = dispatched on window (the drive), else at the target
    /// (bubbling through `#page` to window).
    pub(crate) fn dispatch_pointermove(&mut self, ev: &mut PtrEvent, on_window: bool) {
        if !on_window && ev.target.in_page() {
            self.on_page_pointermove(ev);
        }
        self.gesture_pointermove(ev);
        self.area_drag_move(ev);
    }

    /// A `pointerup` (see `dispatch_pointermove`); `cancel` is a `pointercancel`.
    pub(crate) fn dispatch_pointerup(&mut self, ev: &mut PtrEvent, on_window: bool, cancel: bool) {
        // Window capture: a cancelled drag's release arms the click swallow (R:3263).
        if !cancel && self.lane2.swallow_after_release {
            self.lane2.swallow_after_release = false;
            self.lane2.swallow_click = true;
        }
        if !on_window && ev.target.in_page() {
            self.on_page_pointerup(ev, cancel);
        }
        if !cancel {
            self.gesture_pointerup(ev);
        }
        self.area_drag_end(ev, cancel);
    }

    /// A `click`.
    pub(crate) fn dispatch_click(&mut self, ev: &mut PtrEvent) {
        // R:3257: the one-shot capture listener on window stops the click a drag's release makes.
        if self.lane2.swallow_click {
            self.lane2.swallow_click = false;
            ev.stopped = true;
            ev.prevented = true;
            return;
        }
        if let EvTarget::MenuItem(name) = ev.target {
            // R:4100: the menu's click listener (stops there).
            ev.stopped = true;
            self.activate_object_menu(name);
            return;
        }
        if ev.target == EvTarget::Menu {
            ev.stopped = true;
            return;
        }
        let saved = self.target_key(&ev.target);
        if let EvTarget::Node { index, part } = ev.target.clone() {
            self.node_click(index, part, ev);
        }
        self.refresh_target(&mut ev.target, &saved);
        if !ev.stopped && ev.target.in_page() {
            self.on_page_click(ev);
        }
    }

    /// A `contextmenu`.
    pub(crate) fn dispatch_contextmenu(&mut self, ev: &mut PtrEvent) {
        let saved = self.target_key(&ev.target);
        if let EvTarget::Node { index, part } = ev.target.clone() {
            self.node_contextmenu(index, part, ev);
            self.refresh_target(&mut ev.target, &saved);
            if !ev.stopped {
                self.on_hit_layer_contextmenu(ev);
            }
            self.refresh_target(&mut ev.target, &saved);
        }
        if !ev.stopped && ev.target.in_page() {
            self.on_page_contextmenu(ev);
        }
    }

    /// A `dblclick`.
    pub(crate) fn dispatch_dblclick(&mut self, ev: &mut PtrEvent) {
        if let EvTarget::Node { index, part } = ev.target.clone() {
            self.node_dblclick(index, part, ev);
        }
    }

    /// A `keydown` at `ev.target`: window capture, the object menu, the hit layer's capture
    /// listener, the target, `#page`, the document, and window's bubble (a live drag's Escape).
    pub(crate) fn dispatch_keydown(&mut self, ev: &mut KeyEv) {
        let saved = self.target_key(&ev.target);
        self.on_window_keydown_capture(ev);
        if ev.stopped {
            return;
        }
        self.refresh_target(&mut ev.target, &saved);
        if ev.target.in_menu() {
            self.on_object_menu_keydown(ev);
        }
        if !ev.stopped && ev.target.in_hit_layer() {
            self.on_hit_layer_keydown_capture(ev);
        }
        self.refresh_target(&mut ev.target, &saved);
        if !ev.stopped {
            match ev.target.clone() {
                EvTarget::Editor => self.on_editor_keydown(ev),
                EvTarget::Node { index, .. } => self.node_keydown(index, ev),
                _ => {}
            }
            self.refresh_target(&mut ev.target, &saved);
        }
        if !ev.stopped && ev.target.in_page() {
            self.on_page_keydown(ev);
        }
        if !ev.stopped && ev.target != EvTarget::Window {
            self.on_document_keydown(ev);
        }
        if !ev.stopped {
            self.gesture_keydown(ev);
        }
    }

    /// The `input` event of the open editor: the drive's `type` and the UI's typing.
    pub(crate) fn dispatch_editor_input(&mut self) {
        self.on_editor_input();
    }

    // ------------------------------------------------------------------ per-node listeners

    fn node_pointerdown(&mut self, index: usize, part: Part, ev: &mut PtrEvent) {
        let Some(node) = self.lane2.hit.nodes.get(index).cloned() else {
            return;
        };
        let key = node.key();
        match (&node.kind, part) {
            (NodeKind::GroupBox, Part::Handle(handle)) => self.start_group_resize(ev, handle),
            (NodeKind::Field { item }, Part::Handle(handle)) => {
                self.field_gesture(item, ev, handle, key)
            }
            (NodeKind::SigField { item, movable, .. }, part) => {
                if let Part::Handle(handle) = part {
                    if *movable {
                        self.field_gesture(item, ev, handle, key.clone());
                    }
                    if ev.stopped {
                        return;
                    }
                }
                // R:3460: the body press (a drag in Forms moves the field; the click after it is not a sign).
                ev.prevented = true;
                self.lane2.press_at = None;
                if !*movable
                    || self.ui_mode != UiMode::Forms
                    || ev.button != 0
                    || ev.target.on_handle()
                {
                    return;
                }
                self.lane2.press_at = Some((key.clone(), [ev.client_x, ev.client_y]));
                self.field_gesture(item, ev, "grip", key);
            }
            (
                NodeKind::Link {
                    item,
                    page,
                    editing,
                },
                part,
            ) => {
                if *editing {
                    match part {
                        Part::Handle(handle) => {
                            self.link_target_start(item, *page, ev, handle, key)
                        }
                        _ if ev.button == 0 && !ev.ctrl => {
                            self.link_target_start(item, *page, ev, "grip", key)
                        }
                        _ => {}
                    }
                }
            }
            (NodeKind::TextBox { item, page, can }, _) => {
                if *can && ev.button == 0 && !ev.ctrl {
                    self.text_box_start(item, *page, ev, key);
                }
            }
            (NodeKind::Signature { item, page, can }, part) => {
                if *can {
                    match part {
                        Part::Handle(handle) => self.signature_start(item, *page, ev, handle, key),
                        _ if ev.button == 0 && !ev.ctrl => {
                            self.signature_start(item, *page, ev, "grip", key)
                        }
                        _ => {}
                    }
                }
            }
            (NodeKind::Text { item }, Part::Handle(handle)) => {
                self.start_text_gesture(ev, item, handle, key)
            }
            (NodeKind::Object { .. }, _) => self.object_pointerdown(index, part, ev),
            (NodeKind::Mark { item, page, can }, part) => {
                if *can {
                    match part {
                        Part::Handle(handle) => self.mark_start(item, *page, ev, handle, key),
                        _ if ev.button == 0 && !ev.ctrl => {
                            self.mark_start(item, *page, ev, "grip", key)
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn node_click(&mut self, index: usize, part: Part, ev: &mut PtrEvent) {
        let Some(node) = self.lane2.hit.nodes.get(index).cloned() else {
            return;
        };
        let key = node.key();
        let on_handle = matches!(part, Part::Handle(_));
        match &node.kind {
            NodeKind::Field { item } => {
                ev.prevented = true;
                if on_handle || ev.ctrl {
                    return;
                }
                // P6 U7 §9.21: in Fill forms a Shift-click adds or removes the field (no editor, no toggle).
                if ev.shift && self.ui_mode == UiMode::Forms {
                    let member = self.member_of_node(index);
                    self.shift_select(member);
                    return;
                }
                if !self.direct_selection.is_empty() {
                    self.drop_direct_focus();
                }
                self.focus_node(&key);
                self.activate_field(item, true);
            }
            NodeKind::SigField { item, .. } => {
                ev.prevented = true;
                let press = self
                    .lane2
                    .press_at
                    .take()
                    .filter(|(k, _)| *k == key)
                    .map(|(_, p)| p);
                if on_handle {
                    return;
                }
                if let Some(p) = press
                    && (ev.client_x - p[0]).hypot(ev.client_y - p[1]) >= 3.0
                {
                    return;
                }
                self.focus_node(&key);
                self.activate_field(item, true);
            }
            NodeKind::Link { item, .. } => self.link_target_click(item, index, ev, on_handle),
            NodeKind::TextBox { item, page, .. } => {
                ev.prevented = true;
                if ev.ctrl || ev.shift || !self.can_annotate() {
                    return;
                }
                let point = [
                    item.rect[0],
                    item.rect[1] + super::consts::TEXTBOX_HALF_LINE,
                ];
                self.open_text_box(*page, point, Some(item.clone()));
            }
            NodeKind::Signature { item, .. } => {
                ev.prevented = true;
                if on_handle {
                    return;
                }
                self.direct_focused_sig = Some(item.id);
                self.focus_node(&key);
            }
            NodeKind::Text { item } => {
                ev.prevented = true;
                if on_handle || ev.ctrl {
                    return;
                }
                if ev.shift {
                    let member = self.member_of_node(index);
                    self.shift_select(member);
                    return;
                }
                self.focus_node(&key);
                if item.support == tandem_protocol::engine::EditTargetSupport::Candidate {
                    self.open_inline_text(item, ev.alt);
                } else {
                    let found = self
                        .content_scan
                        .as_ref()
                        .and_then(|s| s.items.iter().position(|i| i == item));
                    if let Some(i) = found {
                        self.select_content_row(i, true);
                    }
                    let title = node.title.clone();
                    self.announce(&title);
                }
            }
            NodeKind::Object { .. } => self.object_click(index, part, ev),
            NodeKind::Mark { item, .. } => {
                ev.prevented = true;
                if on_handle || ev.ctrl {
                    return;
                }
                // P6 U7 §9.21: a mark is a selection member; Shift-click adds or removes it.
                if ev.shift {
                    self.shift_select(Some(super::members::mark_member(item)));
                    return;
                }
                self.direct_focused_object = None;
                self.set_selection(vec![super::members::mark_member(item)]);
                self.mark_selection();
                self.focus_node(&key);
                self.sync_note_current();
            }
            _ => {}
        }
    }

    fn node_keydown(&mut self, index: usize, ev: &mut KeyEv) {
        let Some(node) = self.lane2.hit.nodes.get(index).cloned() else {
            return;
        };
        match &node.kind {
            NodeKind::Field { item } => self.field_target_keydown(item, ev),
            NodeKind::SigField { item, signed, .. } => self.sig_target_keydown(item, *signed, ev),
            NodeKind::Link {
                item,
                page,
                editing,
            } => self.link_target_keydown(item, *page, *editing, ev),
            NodeKind::TextBox { item, page, .. } => self.text_box_keydown(item, *page, ev),
            NodeKind::Signature { item, page, .. } => self.signature_keydown(item, *page, ev),
            NodeKind::Object { .. } => self.object_keydown(index, ev),
            NodeKind::Mark { item, page, .. } => self.mark_keydown(item, *page, ev),
            _ => {}
        }
    }

    fn node_contextmenu(&mut self, index: usize, _part: Part, ev: &mut PtrEvent) {
        if matches!(
            self.lane2.hit.nodes.get(index).map(|n| &n.kind),
            Some(NodeKind::Object { .. })
        ) {
            self.object_contextmenu(index, ev);
        }
    }

    fn node_dblclick(&mut self, index: usize, _part: Part, ev: &mut PtrEvent) {
        let Some(node) = self.lane2.hit.nodes.get(index).cloned() else {
            return;
        };
        match &node.kind {
            NodeKind::Object { .. } => self.object_dblclick(index, ev),
            NodeKind::Link {
                item,
                page,
                editing: true,
            } => {
                ev.prevented = true;
                if !self.lane2.gesture_work {
                    self.open_link_editor(item, *page as usize);
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------ link targets (R:8294 closures)

    /// R:8302: a link's drag or handle (Annotate, editable links only).
    fn link_target_start(
        &mut self,
        item: &LinkEntry,
        page: i64,
        ev: &mut PtrEvent,
        handle: &'static str,
        key: NodeKey,
    ) {
        let annot_id = link_annot_id(item);
        let can = self.can_annotate();
        let options = super::direct::RectGesture {
            rect: item.rect.clone(),
            id: TargetRef::Number(annot_id),
            handle: handle.to_string(),
            min: 4.0,
            can,
            commit: super::direct::RectCommit::Link {
                item: item.clone(),
                page,
            },
            ..super::direct::RectGesture::default()
        };
        self.start_rect_gesture(ev, Some(key), options);
    }

    /// R:8307: Read follows a link; Annotate focuses it (Shift-click adds or removes it).
    fn link_target_click(
        &mut self,
        item: &LinkEntry,
        index: usize,
        ev: &mut PtrEvent,
        on_handle: bool,
    ) {
        ev.prevented = true;
        if on_handle || ev.ctrl {
            return;
        }
        let annot_id = link_annot_id(item);
        if self.ui_mode == UiMode::Annotate && annot_id > 0 && ev.shift {
            self.shift_select(Some(super::members::link_member(item)));
            return;
        }
        if self.ui_mode == UiMode::Read {
            self.follow_link(item);
        } else if self.ui_mode == UiMode::Annotate {
            let key = self.lane2.hit.nodes.get(index).map(HitNode::key);
            self.focus_link(item, key);
            if annot_id > 0 {
                self.set_selection(vec![super::members::link_member(item)]);
                self.mark_selection();
            }
        }
    }

    /// R:8317: the link target's keys (Annotate only).
    fn link_target_keydown(&mut self, item: &LinkEntry, page: i64, editing: bool, ev: &mut KeyEv) {
        if self.ui_mode != UiMode::Annotate {
            return;
        }
        if ev.key == "Escape" {
            ev.prevent();
            self.drop_direct_focus();
            return;
        }
        let size = self.page_size(page);
        type Work = Box<dyn FnOnce(&mut Session) -> Result<(), Refusal>>;
        let mut work: Option<Work> = None;
        if ev.is_arrow() && (ev.alt || (!ev.meta && !ev.ctrl)) {
            if ev.cmd() && (ev.key == "ArrowUp" || ev.key == "ArrowDown") {
                return;
            }
            if let Some(size) = size
                && editing
            {
                let next = super::object_menu::nudge_rect(
                    &item.rect,
                    &ev.key,
                    ev.shift,
                    ev.cmd(),
                    4.0,
                    &size,
                );
                let item = item.clone();
                work = Some(Box::new(move |s: &mut Session| {
                    s.commit_link_rect(&item, page as usize, next);
                    Ok(())
                }));
            }
        } else if ev.key == "Delete" || ev.key == "Backspace" {
            if self.can_annotate() {
                let id = item.id;
                work = Some(Box::new(move |s: &mut Session| {
                    s.delete_link(id);
                    Ok(())
                }));
            }
        } else if ev.key == "Enter" {
            ev.prevent();
            if !self.lane2.gesture_work && editing {
                self.open_link_editor(item, page as usize);
            }
            return;
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

    /// `doc.pages[page]`.
    pub fn page_size(&self, page: i64) -> Option<PageSize> {
        if page < 0 {
            return None;
        }
        self.doc.as_ref()?.pages.get(page as usize).cloned()
    }

    // ------------------------------------------------------------------ hit layer listeners (R:11017-11080)

    /// The member-node test of R:11021 (`directSelection.some(member => memberNode(member) === node)`).
    fn is_member_node(&self, index: usize) -> bool {
        let selection = self.direct_selection.clone();
        selection.iter().any(|m| self.member_node(m) == Some(index))
    }

    /// R:11023: the hit layer's capture keydown: Shift+F10 / ContextMenu on a run, field, link or
    /// mark opens the menu; a mixed selection of two or more moves and deletes as one.
    pub(crate) fn on_hit_layer_keydown_capture(&mut self, ev: &mut KeyEv) {
        if self.direct_active.is_some() || self.direct_gesture.is_some() {
            return;
        }
        let EvTarget::Node { index, .. } = ev.target else {
            return;
        };
        if !self
            .lane2
            .hit
            .nodes
            .get(index)
            .is_some_and(HitNode::is_target)
        {
            return;
        }
        let is_object = matches!(self.lane2.hit.nodes[index].kind, NodeKind::Object { .. });
        if ((ev.key == "F10" && ev.shift) || ev.key == "ContextMenu") && !is_object {
            let Some(member) = self.member_of_node(index) else {
                return;
            };
            ev.prevent();
            ev.stopped = true;
            if !self.is_member_node(index) {
                self.clear_direct_focus();
                self.toggle_member(member);
            }
            let css = self.node_css_after(index);
            self.open_object_menu(css.left, css.top, ObjectMenuKind::Selection, None);
            return;
        }
        if self.direct_selection.len() < 2 || !self.selection_mixed() || !self.is_member_node(index)
        {
            return;
        }
        if ev.key == "Escape" {
            ev.prevent();
            ev.stopped = true;
            self.drop_direct_focus();
            return;
        }
        let nudge = ev.is_arrow() && !ev.meta && !ev.ctrl;
        if !nudge && ev.key != "Delete" && ev.key != "Backspace" {
            return;
        }
        ev.prevent();
        ev.stopped = true;
        if !self.lane2.gesture_work {
            let (key, shift) = (ev.key.clone(), ev.shift);
            if nudge {
                self.run_gesture_work(move |s| s.nudge_selection(&key, shift).map(|_| ()));
            } else {
                self.run_gesture_work(|s| {
                    s.delete_selection();
                    Ok(())
                });
            }
        }
    }

    /// The CSS box of the node that was at `index` (the node may have been repainted since).
    fn node_css_after(&self, index: usize) -> CssBox {
        self.lane2
            .hit
            .nodes
            .get(index)
            .map(|n| n.css)
            .unwrap_or_default()
    }

    /// R:11049: the hit layer's capture pointerdown: a body press on any member of a mixed
    /// selection of two or more moves every member.
    pub(crate) fn on_hit_layer_pointerdown_capture(&mut self, ev: &mut PtrEvent) {
        if ev.button != 0
            || ev.ctrl
            || ev.shift
            || self.direct_active.is_some()
            || ev.target.on_handle()
        {
            return;
        }
        let EvTarget::Node { index, .. } = ev.target else {
            return;
        };
        if !self
            .lane2
            .hit
            .nodes
            .get(index)
            .is_some_and(HitNode::is_target)
        {
            return;
        }
        if self.direct_selection.len() < 2 || !self.selection_mixed() || !self.is_member_node(index)
        {
            return;
        }
        let key = self.lane2.hit.nodes[index].key();
        self.start_selection_gesture(ev, key);
    }

    /// R:11055: a right-click (or Control-click) on a run, field, link or mark selects it alone
    /// unless it is a member, then opens the menu.
    pub(crate) fn on_hit_layer_contextmenu(&mut self, ev: &mut PtrEvent) {
        let EvTarget::Node { index, .. } = ev.target else {
            return;
        };
        let Some(node) = self.lane2.hit.nodes.get(index) else {
            return;
        };
        if !node.is_target()
            || matches!(node.kind, NodeKind::Object { .. })
            || self.direct_active.is_some()
        {
            return;
        }
        let Some(member) = self.member_of_node(index) else {
            return;
        };
        ev.prevented = true;
        ev.stopped = true;
        if !self.is_member_node(index) {
            self.clear_direct_focus();
            self.toggle_member(member);
        }
        self.open_object_menu(ev.client_x, ev.client_y, ObjectMenuKind::Selection, None);
    }

    /// R:11061: right-click on empty page space opens the page menu (only Paste can be
    /// enabled); it pastes there.
    pub(crate) fn on_page_contextmenu(&mut self, ev: &mut PtrEvent) {
        if ev.target.in_menu() || matches!(ev.target, EvTarget::Node { .. } | EvTarget::Editor) {
            return;
        }
        let live_mode = matches!(
            self.ui_mode,
            UiMode::Edit | UiMode::Forms | UiMode::Annotate | UiMode::Redact
        );
        if self.doc.is_none() || !live_mode || !self.layer_live() {
            return;
        }
        ev.prevented = true;
        self.drop_direct_focus();
        self.add_blocked = true;
        let point = match &self.drive_menu_point {
            Some(p) => Some(p.clone()),
            None => self.page_point_of(ev).map(|p| p.to_vec()),
        };
        self.open_object_menu(ev.client_x, ev.client_y, ObjectMenuKind::Page, point);
    }

    /// R:11070: the Edit menu's Cut/Copy/Paste roles and the chords reach the page selection
    /// as DOM clipboard events at the focused element.
    pub fn on_clipboard_event(&mut self, kind: &str) -> bool {
        let editor_focused = matches!(self.lane2.focus, Focus::Editor(_));
        let active = match &self.lane2.focus {
            Focus::Body => None,
            Focus::Editor(_) => Some(super::clipboard::ActiveElement {
                tag: "INPUT".into(),
                kind: None,
                content_editable: false,
                inline_editor: true,
            }),
            Focus::TextEntry(_) => Some(super::clipboard::ActiveElement {
                tag: "TEXTAREA".into(),
                kind: None,
                content_editable: false,
                inline_editor: false,
            }),
            _ => Some(super::clipboard::ActiveElement {
                tag: "BUTTON".into(),
                kind: Some("button".into()),
                content_editable: false,
                inline_editor: false,
            }),
        };
        let ctx = super::clipboard::ChordContext {
            mode: self.ui_mode,
            layer_live: self.layer_live(),
            dialog_open: self.any_dialog_open(),
            menu_open: !self.dom.hidden(ids::OBJECT_MENU),
            editor_open: self.direct_active.is_some() || editor_focused,
            active,
            collapsed: self.lane2.text_collapsed,
            selection: self.direct_selection.len(),
            clipboard: self.clipboard_info.is_some(),
            editable: self.editable(),
        };
        if !super::clipboard::clipboard_chord_ok(kind, &ctx) {
            return false;
        }
        if !self.lane2.gesture_work {
            let which = kind.to_string();
            self.run_gesture_work(move |s| s.clipboard_work(&which, None).map(|_| ()));
        }
        true
    }

    /// `document.querySelector("dialog[open]")`.
    pub fn any_dialog_open(&self) -> bool {
        DIALOG_IDS.iter().any(|id| self.dom.is_open(id))
    }

    // ------------------------------------------------------------------ UI entry points

    /// The UI's pointer: `phase` is "down", "move", "up" or "cancel", at page points of `page`
    /// (the shown page). A press is routed as the browser would: hit test, the capture
    /// listeners, the target, `#page`, then the default focus; a release makes the click.
    pub fn pointer(
        &mut self,
        phase: &str,
        page: usize,
        x: f64,
        y: f64,
        shift: bool,
        alt: bool,
        meta: bool,
    ) {
        self.begin_event();
        let Some((cx, cy)) = self.client_of(page as i64, x, y) else {
            return;
        };
        let target = self.hit_test(cx, cy);
        let mut ev = PtrEvent {
            client_x: cx,
            client_y: cy,
            button: 0,
            shift,
            alt,
            meta,
            ctrl: false,
            target: target.clone(),
            detail: 1,
            ..PtrEvent::default()
        };
        self.lane2.hover = match &target {
            EvTarget::Node { index, .. } => self.lane2.hit.nodes.get(*index).map(HitNode::key),
            _ => None,
        };
        match phase {
            "down" => {
                self.lane2.swallow_click = false;
                self.lane2.down = Some((target.clone(), self.target_key(&target)));
                self.dispatch_pointerdown(&mut ev);
                if !ev.prevented {
                    // The press's default action: focus the focusable element under it.
                    match &target {
                        EvTarget::Node { index, .. } => {
                            if let Some(key) = self
                                .lane2
                                .hit
                                .nodes
                                .get(*index)
                                .filter(|n| n.is_target())
                                .map(HitNode::key)
                            {
                                self.set_focus(Focus::Node(key));
                            }
                        }
                        EvTarget::Editor => self.focus_editor(),
                        EvTarget::Page | EvTarget::HitLayer => self.focus_page(),
                        _ => {}
                    }
                }
            }
            "move" => self.dispatch_pointermove(&mut ev, false),
            "up" | "cancel" => {
                let cancel = phase == "cancel";
                self.dispatch_pointerup(&mut ev, false, cancel);
                let down = self.lane2.down.take();
                if cancel {
                    return;
                }
                // The click goes to the nearest common ancestor of the press and the release.
                let release = self.target_key(&target);
                let click_target = match down {
                    Some((_, Some((key, part))))
                        if release.as_ref().is_some_and(|(k, _)| *k == key) =>
                    {
                        let same_part = release.as_ref().is_some_and(|(_, p)| *p == part);
                        self.lane2.hit.find(&key).map(|index| EvTarget::Node {
                            index,
                            part: if same_part { part } else { Part::Body },
                        })
                    }
                    Some((down, _)) if down == target && !matches!(down, EvTarget::Node { .. }) => {
                        Some(target)
                    }
                    Some((down, _)) if down.in_hit_layer() && target.in_hit_layer() => {
                        Some(EvTarget::HitLayer)
                    }
                    Some((down, _)) if down.in_page() && target.in_page() => Some(EvTarget::Page),
                    _ => None,
                };
                if let Some(click_target) = click_target {
                    let mut click = PtrEvent {
                        target: click_target,
                        ..ev
                    };
                    self.dispatch_click(&mut click);
                }
            }
            _ => {}
        }
    }

    /// The UI's right-click (or Control-click) at page points: the `contextmenu` event.
    pub fn context_menu(&mut self, page: usize, x: f64, y: f64) {
        self.begin_event();
        let Some((cx, cy)) = self.client_of(page as i64, x, y) else {
            return;
        };
        let target = self.hit_test(cx, cy);
        let mut press = PtrEvent {
            client_x: cx,
            client_y: cy,
            button: 2,
            target: target.clone(),
            ..PtrEvent::default()
        };
        self.dispatch_pointerdown(&mut press);
        let mut ev = PtrEvent {
            client_x: cx,
            client_y: cy,
            button: 2,
            target,
            ..PtrEvent::default()
        };
        self.dispatch_contextmenu(&mut ev);
    }

    /// The UI's double click at page points (after its two clicks): the `dblclick` event.
    pub fn double_click(&mut self, page: usize, x: f64, y: f64) {
        let Some((cx, cy)) = self.client_of(page as i64, x, y) else {
            return;
        };
        let target = self.hit_test(cx, cy);
        let mut ev = PtrEvent {
            client_x: cx,
            client_y: cy,
            target,
            detail: 2,
            ..PtrEvent::default()
        };
        self.dispatch_dblclick(&mut ev);
    }

    /// The UI's key press at the focused element (`key` is the DOM `KeyboardEvent.key`, " " for
    /// Space). An unhandled Enter or Space on a focused target is its click, as on a button.
    pub fn key(&mut self, key: &str, shift: bool, alt: bool, meta: bool) {
        self.begin_event();
        let target = self.focus_target();
        let mut ev = KeyEv {
            target: target.clone(),
            ..KeyEv::new(key, shift, alt, meta)
        };
        self.dispatch_keydown(&mut ev);
        if ev.prevented {
            return;
        }
        if (key == "Enter" || key == " ")
            && let EvTarget::Node { index, .. } = target
            && self
                .lane2
                .hit
                .nodes
                .get(index)
                .is_some_and(HitNode::is_target)
        {
            let mut click = PtrEvent {
                target: EvTarget::Node {
                    index,
                    part: Part::Body,
                },
                detail: 0,
                ..PtrEvent::default()
            };
            self.dispatch_click(&mut click);
        } else if let EvTarget::MenuItem(name) = target
            && (key == "Enter" || key == " ")
        {
            self.activate_object_menu(name);
        }
    }

    /// The UI's typing in the open inline editor: its new value and caret.
    pub fn editor_input(&mut self, value: &str, selection: Option<(usize, usize)>) {
        let Some(active) = self.direct_active.as_mut() else {
            return;
        };
        active.editor.value = value.to_string();
        if let Some((start, end)) = selection {
            active.editor.selection_start = Some(start);
            active.editor.selection_end = Some(end);
        }
        self.dispatch_editor_input();
    }

    /// `hitLayer.querySelectorAll(selector).length` for STATE: `.object-target`, `.redact-target`, `.link-target`.
    pub fn hit_count(&self, kind: &str) -> usize {
        self.lane2
            .hit
            .nodes
            .iter()
            .filter(|n| match (&n.kind, kind) {
                (NodeKind::Object { .. }, "object") => true,
                (NodeKind::Mark { .. }, "mark") => true,
                (NodeKind::Link { .. }, "link") => true,
                _ => false,
            })
            .count()
    }

    /// `cssBox(element)` of the painted node a STATE entry reads (objects by target id,
    /// links and marks by annot id).
    pub fn node_css_box(&self, kind: &str, id: &str) -> Option<CssBox> {
        self.lane2.hit.nodes.iter().find_map(|n| {
            let hit = match (&n.kind, kind) {
                (NodeKind::Object { item, .. }, "object") => item.target_id == id,
                (NodeKind::Link { item, .. }, "link") => link_annot_id(item).to_string() == id,
                (NodeKind::Mark { item, .. }, "mark") => item.id.to_string() == id,
                _ => false,
            };
            hit.then_some(n.css)
        })
    }

    /// Whether the link kind is a page link (the UI's cursor).
    pub fn link_is_page(item: &LinkEntry) -> bool {
        item.kind == LinkKind::Page
    }

    /// Whether a tool button is armed.
    pub fn tool_pressed(&self, id: &str) -> bool {
        is_pressed(&self.dom, id)
    }
}

/// Every `<dialog>` of index.html (`document.querySelector("dialog[open]")`).
pub const DIALOG_IDS: [&str; 21] = [
    "note-dialog",
    "help-dialog",
    "redact-dialog",
    "redact-find-dialog",
    "redact-pending-dialog",
    "redact-ask-dialog",
    "stamp-icon-dialog",
    "sigpad-dialog",
    "stamp-dialog",
    "labels-dialog",
    "export-dialog",
    "insert-dialog",
    "extract-dialog",
    "bookmark-dialog",
    "link-dialog",
    "print-dialog",
    "verify-dialog",
    "batch-dialog",
    "sign-choice-dialog",
    "cert-sign-dialog",
    "security-dialog",
];

/// A plain refusal of this lane's own (`throw new Error(text)`).
pub fn refusal(text: &str) -> Refusal {
    Refusal::plain(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(kind: NodeKind, rect: [f64; 4]) -> HitNode {
        HitNode::new(kind, &rect, 1.0)
    }

    #[test]
    fn handles_sit_on_the_padding_box() {
        let mut n = node(
            NodeKind::Mark {
                item: AnnotItem::default(),
                page: 0,
                can: true,
            },
            [100.0, 100.0, 200.0, 150.0],
        );
        n.handles = vec!["grip", "n", "ne", "e", "se", "s", "sw", "w", "nw"];
        assert_eq!(
            n.handle_box("grip"),
            CssBox {
                left: 86.0,
                top: 86.0,
                width: 14.0,
                height: 14.0
            }
        );
        assert_eq!(
            n.handle_box("nw"),
            CssBox {
                left: 97.0,
                top: 97.0,
                width: 8.0,
                height: 8.0
            }
        );
        assert_eq!(
            n.handle_box("se"),
            CssBox {
                left: 195.0,
                top: 145.0,
                width: 8.0,
                height: 8.0
            }
        );
        assert_eq!(n.handle_box("n").left, 146.0);
    }

    #[test]
    fn hit_order_follows_z_and_document_order() {
        let mut s = Session::new();
        s.lane2.hit.live = true;
        let mut a = node(
            NodeKind::Mark {
                item: AnnotItem {
                    id: 1,
                    ..AnnotItem::default()
                },
                page: 0,
                can: true,
            },
            [0.0, 0.0, 100.0, 100.0],
        );
        a.handles = vec!["grip", "n", "ne", "e", "se", "s", "sw", "w", "nw"];
        let mut b = node(
            NodeKind::Mark {
                item: AnnotItem {
                    id: 2,
                    ..AnnotItem::default()
                },
                page: 0,
                can: true,
            },
            [50.0, 50.0, 150.0, 150.0],
        );
        b.handles = vec!["grip", "n", "ne", "e", "se", "s", "sw", "w", "nw"];
        let ia = s.lane2.hit.append(a);
        let ib = s.lane2.hit.append(b);
        assert_eq!(
            s.hit_test(75.0, 75.0),
            EvTarget::Node {
                index: ib,
                part: Part::Body
            }
        );
        assert_eq!(
            s.hit_test(25.0, 25.0),
            EvTarget::Node {
                index: ia,
                part: Part::Body
            }
        );
        // a selected node is above a later unselected one, and its handles show.
        s.lane2.hit.nodes[ia].selected = true;
        assert_eq!(
            s.hit_test(75.0, 75.0),
            EvTarget::Node {
                index: ia,
                part: Part::Body
            }
        );
        assert_eq!(
            s.hit_test(100.0, 100.0),
            EvTarget::Node {
                index: ia,
                part: Part::Handle("se")
            }
        );
        s.lane2.hit.live = false;
        assert_eq!(s.hit_test(25.0, 25.0), EvTarget::Page);
    }

    #[test]
    fn wrapping_forms_take_the_ring_only() {
        let mut s = Session::new();
        s.lane2.hit.live = true;
        let n = node(
            NodeKind::Object {
                item: EditTarget::default(),
                page: 0,
                rect: vec![0.0, 0.0, 100.0, 100.0],
                wrapping: true,
                can: true,
            },
            [0.0, 0.0, 100.0, 100.0],
        );
        let i = s.lane2.hit.append(n);
        assert_eq!(
            s.hit_test(3.0, 50.0),
            EvTarget::Node {
                index: i,
                part: Part::Ring
            }
        );
        assert_eq!(s.hit_test(50.0, 50.0), EvTarget::Page);
    }
}
