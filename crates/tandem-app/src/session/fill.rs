//! Fill anywhere and picture signatures (R:4619-5060): FreeText text boxes in Fill forms, the
//! note editor, the signature pad, placing and moving drawn/image signatures, and the drive's
//! `fill-text`, `sign-draw` and `sign-place`.
#![allow(dead_code)]

use super::Session;
use super::consts::{
    MAX_PAD_POINTS, MAX_PAD_STROKES, PAD_FULL, SIGN_IMAGE_LARGE, SIGN_IMAGE_NOT, SIGN_IMAGE_ONE,
    SIGN_IMAGE_UNREADABLE, SIGN_MIN_WIDTH, SIGN_ROTATED, TEXTBOX_HALF_LINE, TEXTBOX_ROTATED,
};
use super::direct::{
    FIELD_HANDLES, RectCommit, RectGesture, grow_paragraph_editor, is_signature, place_editor,
    round_rect4,
};
use super::dom_ids as ids;
use super::hit::{HitNode, KeyEv, NodeKey, NodeKind, PtrEvent};
use super::nav::{js_to_fixed, round2, set_pressed};
use super::types::*;
use crate::engine::Refusal;
use serde_json::{Map, Value, json};
use tandem_protocol::limits::{
    MAX_INK_POINTS, MAX_NOTE_CHARS, MAX_STAMP_IMAGE_BYTES, NOTE_SIZE, SIGNATURE_SIZE,
};

/// The text a refusal of an annotation write shows: the missing characters of an
/// `unsupported-encoding`, else the engine's `error`, else the message.
fn annot_refusal_text(err: &Refusal) -> String {
    let refused = &err.reply;
    let missing: Vec<String> = refused
        .get("missing")
        .and_then(Value::as_array)
        .map(|m| {
            m.iter()
                .filter_map(|c| c.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if refused.get("detail").and_then(Value::as_str) == Some("unsupported-encoding")
        && !missing.is_empty()
    {
        return format!("Cannot draw: {} (not in this font)", missing.join(" "));
    }
    match refused.get("error") {
        Some(Value::String(e)) => e.clone(),
        _ => err.message.clone(),
    }
}

/// `reply.error` when the refusal has one, else the message (`refusalOf(err).reply?.error ?? messageOf(err)`).
fn error_text(err: &Refusal) -> String {
    err.reply
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| err.message.clone())
}

/// A JSON number that is an integer (`Number.isInteger`).
fn int_of(v: Option<&Value>) -> Option<i64> {
    let n = v?.as_f64()?;
    (n.is_finite() && n.fract() == 0.0).then_some(n as i64)
}

/// A finite JSON number.
fn finite_of(v: Option<&Value>) -> Option<f64> {
    v?.as_f64().filter(|n| n.is_finite())
}

impl Session {
    // ------------------------------------------------------------------ text boxes

    /// R:4620 `paintTextBoxTarget()`: a FreeText in Fill forms — drag moves it, arrows nudge,
    /// Delete removes it, a click or Enter edits it.
    pub fn paint_text_box_target(&mut self, item: &AnnotItem, page: i64, index: usize) -> HitNode {
        let can = self.can_annotate();
        let mut node = HitNode::new(
            NodeKind::TextBox {
                item: item.clone(),
                page,
                can,
            },
            &item.rect,
            self.shown_zoom,
        );
        node.classes.push("textbox-target");
        node.label = format!("Text box {}", index + 1);
        node.title = if can {
            "Text box: click to edit, drag to move, Delete removes it".into()
        } else {
            "This document is read-only".into()
        };
        node
    }

    /// R:4628 the text box's body press (no handles): a move.
    pub(crate) fn text_box_start(
        &mut self,
        item: &AnnotItem,
        page: i64,
        ev: &mut PtrEvent,
        key: NodeKey,
    ) {
        let can = self.can_annotate();
        let options = RectGesture {
            rect: item.rect.clone(),
            id: TargetRef::Number(item.id),
            handle: "grip".into(),
            min: 1.0,
            can,
            commit: RectCommit::BoxMove {
                annot_id: item.id,
                page,
            },
            ..RectGesture::default()
        };
        self.start_rect_gesture(ev, Some(key), options);
    }

    /// R:4633 the text box's keys.
    pub(crate) fn text_box_keydown(&mut self, item: &AnnotItem, page: i64, ev: &mut KeyEv) {
        let point = [item.rect[0], item.rect[1] + TEXTBOX_HALF_LINE];
        if ev.key == "Escape" {
            ev.prevent();
            self.drop_direct_focus();
            return;
        }
        let size = self.page_size(page);
        let (id, rect) = (item.id, item.rect.clone());
        type Work = Box<dyn FnOnce(&mut Session) -> Result<(), Refusal>>;
        let mut work: Option<Work> = None;
        if ev.is_arrow() && !ev.meta && !ev.ctrl {
            if let Some(size) = size
                && self.can_annotate()
            {
                let next =
                    super::object_menu::nudge_rect(&rect, &ev.key, ev.shift, false, 1.0, &size);
                work = Some(Box::new(move |s: &mut Session| {
                    s.commit_box_move(id, page, &next);
                    Ok(())
                }));
            }
        } else if ev.key == "Delete" || ev.key == "Backspace" {
            if self.can_annotate() {
                work = Some(Box::new(move |s: &mut Session| {
                    s.commit_text_box(page, Some(id), &point, "");
                    Ok(())
                }));
            }
        } else if ev.key == "Enter" {
            ev.prevent();
            if self.can_annotate() {
                self.open_text_box(page, point, Some(item.clone()));
            }
            return;
        } else {
            return;
        }
        ev.prevent();
        if let Some(work) = work
            && !self.lane2.gesture_work
        {
            self.run_gesture_work(work);
        }
    }

    /// R:4648 `openTextBox()`: the text-box editor in Helvetica 12 black; Enter, Tab and blur commit.
    pub fn open_text_box(&mut self, page: i64, point: [f64; 2], bx: Option<AnnotItem>) {
        self.clear_direct_focus();
        self.close_direct();
        self.clear_error();
        let rect = vec![
            point[0],
            point[1] - TEXTBOX_HALF_LINE,
            point[0] + 96.0,
            point[1] + TEXTBOX_HALF_LINE,
        ];
        let value = bx.as_ref().map(|b| b.contents.clone()).unwrap_or_default();
        let mut editor = EditorState {
            tag: EditorTag::Textarea,
            class_name: "inline-editor text-inline-editor add-text".into(),
            max_length: MAX_NOTE_CHARS as i64,
            font_size: Some("12".into()),
            initial: Some(value.clone()),
            selection_start: Some(utf16_len(&value)),
            selection_end: Some(utf16_len(&value)),
            value,
            ..EditorState::default()
        };
        self.lane2.editor_label = if bx.is_some() {
            "Edit text box".into()
        } else {
            format!(
                "New text box at {}, {} pt",
                js_to_fixed(point[0], 1),
                js_to_fixed(point[1], 1)
            )
        };
        place_editor(&mut editor, &rect, self.shown_zoom);
        let item = EditTarget {
            kind: tandem_protocol::engine::EditTargetKind::Text,
            bounds: Some(rect),
            support: tandem_protocol::engine::EditTargetSupport::Candidate,
            text: Some(String::new()),
            ..EditTarget::default()
        };
        let select = bx.is_some();
        self.direct_active = Some(DirectActive {
            kind: DirectKind::Textbox,
            item: DirectItem::Target(item),
            editor,
            page,
            point: Some(point),
            annot_id: bx.map(|b| b.id),
            ..DirectActive::default()
        });
        self.grow_add_editor();
        self.setup_inline_events(false);
        self.focus_editor();
        if select {
            self.select_editor_text();
        }
    }

    /// R:4667 `openNoteEditor()`: placing a note opens its editor at once, beside the note's spot.
    pub fn open_note_editor(&mut self, page: i64, point: [f64; 2]) {
        self.clear_direct_focus();
        self.close_direct();
        self.clear_error();
        let width = self.page_size(page).map_or(612.0, |s| s.width);
        let left = (point[0] + NOTE_SIZE).min((width - 200.0).max(0.0));
        let rect = vec![left, point[1], left + 200.0, point[1] + 72.0];
        let mut editor = EditorState {
            tag: EditorTag::Textarea,
            class_name: "inline-editor note-inline-editor".into(),
            max_length: MAX_NOTE_CHARS as i64,
            placeholder: "Type your note… (⌘Enter to finish, Esc to discard)".into(),
            selection_start: Some(0),
            selection_end: Some(0),
            ..EditorState::default()
        };
        self.lane2.editor_label = "New note".into();
        place_editor(&mut editor, &rect, self.shown_zoom);
        let item = EditTarget {
            kind: tandem_protocol::engine::EditTargetKind::Text,
            bounds: Some(rect),
            support: tandem_protocol::engine::EditTargetSupport::Candidate,
            text: Some(String::new()),
            ..EditTarget::default()
        };
        self.direct_active = Some(DirectActive {
            kind: DirectKind::Note,
            item: DirectItem::Target(item),
            editor,
            page,
            point: Some(point),
            ..DirectActive::default()
        });
        self.setup_inline_events(true);
        self.focus_editor();
        self.announce("Type the note, then press ⌘Enter or click elsewhere");
    }

    /// R:4683 `commitTextBox()`: the one commit the editor, Delete and `fill-text` share
    /// (new → add, emptied → delete, else re-fit).
    pub fn commit_text_box(
        &mut self,
        page: i64,
        annot_id: Option<i64>,
        point: &[f64],
        text: &str,
    ) -> bool {
        if self.doc.is_none() || !self.can_annotate() {
            return false;
        }
        if self.rotation(page) % 360 != 0 {
            self.show_error(TEXTBOX_ROTATED);
            return false;
        }
        if annot_id.is_none() && text.is_empty() {
            return true;
        }
        self.clear_error();
        let result = (|| -> Result<(), Refusal> {
            match annot_id {
                None => {
                    let before = self.annot_ids(page)?;
                    let reply = self.rq(json!({"op": "annot-add", "page": page, "type": "FreeText", "point": point, "contents": text, "fit": true}))?;
                    self.note_filled();
                    self.apply_edit(reply.value, "Added text box");
                    let id = self
                        .notes
                        .iter()
                        .find(|i| i.r#type == "FreeText" && !before.contains(&i.id))
                        .map_or(-1, |i| i.id);
                    self.fill_last = Some(Self::fill_op("add", id, self.note_rect2(id)));
                    self.direct_focused_box = (id >= 0).then_some(id);
                }
                Some(annot_id) if text.is_empty() => {
                    let reply =
                        self.rq(json!({"op": "annot-delete", "page": page, "annotId": annot_id}))?;
                    self.note_filled();
                    self.direct_focused_box = None;
                    self.fill_last = Some(Self::fill_op("delete", annot_id, None));
                    self.apply_edit(reply.value, "Deleted text box");
                }
                Some(annot_id) => {
                    let reply = self.rq(json!({"op": "annot-set", "page": page, "annotId": annot_id, "contents": text, "fit": true}))?;
                    self.note_filled();
                    self.apply_edit(reply.value, "Edited text box");
                    self.fill_last =
                        Some(Self::fill_op("edit", annot_id, self.note_rect2(annot_id)));
                    self.direct_focused_box = Some(annot_id);
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => true,
            Err(err) => {
                let message = annot_refusal_text(&err);
                self.announce(&message);
                self.show_error(&message);
                false
            }
        }
    }

    /// The ids `annots {page}` lists (to find the one an add made).
    fn annot_ids(&mut self, page: i64) -> Result<Vec<i64>, Refusal> {
        let reply = self.rq(json!({"op": "annots", "page": page}))?;
        Ok(reply
            .get("items")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.get("id").and_then(Value::as_i64))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// `notes.find(item => item.id === id)?.rect.map(round2) ?? null` (R:4691 `rectOf`, R:4750 `sigRect`).
    fn note_rect2(&self, id: i64) -> Option<Vec<f64>> {
        self.notes
            .iter()
            .find(|i| i.id == id)
            .map(|i| i.rect.iter().map(|v| round2(*v)).collect())
    }

    /// R:4717 `commitBoxMove()`: one `annot-move` to the rect's top-left; a no-op reply writes nothing.
    pub fn commit_box_move(&mut self, annot_id: i64, page: i64, rect: &[f64]) -> bool {
        self.direct_focused_box = Some(annot_id);
        match self.rq(json!({"op": "annot-move", "page": page, "annotId": annot_id, "point": round_rect4(&rect[..2])})) {
            Ok(reply) => {
                if reply.get("noop") == Some(&Value::Bool(true)) {
                    return true;
                }
                self.note_filled();
                self.apply_edit(reply.value, "Moved text box");
                self.fill_last = Some(Self::fill_op("move", annot_id, self.note_rect2(annot_id)));
                true
            }
            Err(err) => {
                self.show_error(&error_text(&err));
                false
            }
        }
    }

    /// R:4734 `driveFillText()`: `fill-text {page, x, y, text}`.
    pub fn drive_fill_text(&mut self, contents: Option<&str>) {
        let Some(value) = self.drive_wanted(contents) else {
            return;
        };
        let Some(doc) = &self.doc else { return };
        let page_count = doc.page_count;
        let (page, x, y, text) = (
            int_of(value.get("page")),
            finite_of(value.get("x")),
            finite_of(value.get("y")),
            value.get("text").and_then(Value::as_str),
        );
        let (Some(page), Some(x), Some(y), Some(text)) = (page, x, y, text) else {
            self.show_error("fill-text: page, x, y and text are required");
            return;
        };
        if page < 0 || page >= page_count {
            self.show_error(&format!(
                "fill-text: page must be an integer between 0 and {}",
                page_count - 1
            ));
            return;
        }
        // An empty text or a document that forbids annotations is refused in `error`, never a silent no-op.
        let draft = add_draft(text);
        if draft.is_empty() {
            self.show_error("fill-text: text is empty");
            return;
        }
        if !self.can_annotate() {
            self.show_error("fill-text: this document does not permit annotations");
            return;
        }
        if self.direct_active.is_some() {
            self.finish_direct();
        }
        self.go_to(page);
        self.drain();
        self.commit_text_box(page, None, &[x, y], &draft);
    }

    // ------------------------------------------------------------------ the signature pad

    /// R:4761 `padBegin()`: a 65th stroke is refused; the drawing stops at 4096 points.
    pub fn pad_begin(&mut self, point: Point) -> bool {
        self.drop_pad_image(); // sign-merge D1: drawing discards a chosen image
        if self.pad_strokes.len() >= MAX_PAD_STROKES {
            self.dom.set_text(
                ids::SIGPAD_STATUS,
                "A signature can have at most 64 strokes",
            );
            return false;
        }
        if self.pad_count() + 2 > MAX_PAD_POINTS {
            self.pad_full = true;
            self.dom.set_text(ids::SIGPAD_STATUS, PAD_FULL);
            return false;
        }
        self.pad_stroke = Some(vec![point]);
        true
    }

    /// R:4768 `padPoint()`: a sample under 2 px from the last is skipped; 2048 points stop a stroke.
    pub fn pad_point(&mut self, point: Point) {
        let full = self.pad_count() >= MAX_PAD_POINTS;
        let Some(stroke) = self.pad_stroke.as_mut() else {
            return;
        };
        if stroke.len() >= MAX_INK_POINTS {
            return;
        }
        let last = stroke[stroke.len() - 1];
        if (point[0] - last[0]).hypot(point[1] - last[1]) < 2.0 {
            return;
        }
        if full {
            self.pad_full = true;
            self.dom.set_text(ids::SIGPAD_STATUS, PAD_FULL);
            return;
        }
        stroke.push(point);
    }

    /// R:4774 `padEnd()`: a tap (1 point) is kept as a dot.
    pub fn pad_end(&mut self) {
        if let Some(mut stroke) = self.pad_stroke.take() {
            if stroke.len() == 1 {
                stroke.push([stroke[0][0] + 0.5, stroke[0][1]]);
            }
            self.pad_strokes.push(stroke);
        }
        self.sync_pad();
    }

    /// R:4781 `padCount`.
    pub fn pad_count(&self) -> usize {
        self.pad_strokes.iter().map(Vec::len).sum::<usize>()
            + self.pad_stroke.as_ref().map_or(0, Vec::len)
    }

    /// R:4782 `padBounds()`.
    pub fn pad_bounds(&self) -> Option<[f64; 4]> {
        let mut points = self.pad_strokes.iter().flatten().peekable();
        points.peek()?;
        Some(points.fold(
            [
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ],
            |b, p| {
                [
                    b[0].min(p[0]),
                    b[1].min(p[1]),
                    b[2].max(p[0]),
                    b[3].max(p[1]),
                ]
            },
        ))
    }

    /// R:4787 `syncPad()`: Save needs an image or a drawing at least 8 px wide.
    pub fn sync_pad(&mut self) {
        let bounds = self.pad_bounds();
        let save_off = self.pad_image.is_none() && bounds.is_none_or(|b| b[2] - b[0] < 8.0);
        self.dom.set_disabled(ids::SIGPAD_SAVE_BUTTON, save_off);
        self.dom
            .set_disabled(ids::SIGPAD_REMOVE_BUTTON, self.saved_signature.is_none());
    }

    /// R:4804 `clearPad()`.
    pub fn clear_pad(&mut self) {
        self.drop_pad_image();
        self.pad_strokes.clear();
        self.pad_stroke = None;
        self.pad_full = false;
        self.dom.set_text(ids::SIGPAD_STATUS, "");
        self.sync_pad();
    }

    /// R:4805 `dropPadImage()`.
    pub fn drop_pad_image(&mut self) {
        self.pad_image = None;
    }

    /// R:4815 `padUseImage()`: "Use an image…" — exactly one PNG or JPEG through the picker,
    /// decoded upright and kept as a fresh PNG (long side ≤ 1000 px). `None` = chosen,
    /// `Some("")` = cancelled, else the refusal (nothing changed).
    pub fn pad_use_image(&mut self) -> Option<String> {
        let picked = match self.host.pick_images() {
            Ok(images) => images,
            Err("cancelled") => return Some(String::new()),
            Err("too-many") => return Some(SIGN_IMAGE_ONE.into()),
            Err("too-large") => return Some(SIGN_IMAGE_LARGE.into()),
            Err(_) => return Some(SIGN_IMAGE_UNREADABLE.into()),
        };
        if picked.len() > 1 {
            return Some(SIGN_IMAGE_ONE.into());
        }
        let Some(bytes) = picked.into_iter().next().map(|p| p.bytes) else {
            return Some(String::new());
        };
        if bytes.len() > MAX_STAMP_IMAGE_BYTES {
            return Some(SIGN_IMAGE_LARGE.into());
        }
        let png_magic = bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
        let jpeg_magic = bytes.starts_with(&[0xff, 0xd8, 0xff]);
        if !png_magic && !jpeg_magic {
            return Some(SIGN_IMAGE_NOT.into());
        }
        // Review F1: decoded upright (EXIF applied), re-encoded as PNG with no metadata; long side capped at 1000 px.
        let (png, width, height) = match self.decode_signature_image(&bytes) {
            Ok(decoded) => decoded,
            Err(refusal) => return Some(refusal),
        };
        if png.len() > MAX_STAMP_IMAGE_BYTES {
            return Some(SIGN_IMAGE_LARGE.into());
        }
        self.drop_pad_image();
        self.pad_strokes.clear();
        self.pad_stroke = None;
        self.pad_full = false;
        self.pad_image = Some(PadImage {
            width,
            height,
            data: crate::host::util::base64_encode(&png),
        });
        self.dom.set_text(ids::SIGPAD_STATUS, "");
        self.sync_pad();
        None
    }

    /// R:4848 `openSigpad(thenArm)`: the pad always opens empty.
    pub fn open_sigpad(&mut self, then_arm: bool) {
        if !self.dom.is_open(ids::SIGPAD_DIALOG) {
            self.dom.set_open(ids::SIGPAD_DIALOG, true);
        }
        self.clear_pad();
        self.pad_then_arm = then_arm;
    }

    /// R:4849 `loadSavedSignature()` (also run once at startup, R:5060).
    pub fn load_saved_signature(&mut self) {
        self.saved_signature = self
            .host
            .signature
            .get()
            .and_then(|v| serde_json::from_value(v).ok());
        self.sync_pad();
    }

    /// `sigpadDialog.close()` and its `close` listener (R:5046): only Save wrote anything.
    pub fn close_sigpad(&mut self) {
        if !self.dom.is_open(ids::SIGPAD_DIALOG) {
            return;
        }
        self.dom.set_open(ids::SIGPAD_DIALOG, false);
        self.pad_then_arm = false;
        self.pad_strokes.clear();
        self.pad_stroke = None;
        self.drop_pad_image();
    }

    /// R:4854 `saveDrawnSignature()`: translate to the origin, round, store through main, re-read,
    /// close, announce; or the chosen image.
    pub fn save_drawn_signature(&mut self) -> bool {
        let bounds = self.pad_bounds();
        if self.pad_image.is_none() && bounds.is_none_or(|b| b[2] - b[0] < 8.0) {
            return false;
        }
        let value: SavedSignature = match &self.pad_image {
            Some(image) => SavedSignature::Image(ImageSig {
                v: 1,
                kind: "image".into(),
                width: image.width,
                height: image.height,
                data: image.data.clone(),
            }),
            None => {
                let b = bounds.unwrap();
                let strokes = self
                    .pad_strokes
                    .iter()
                    .map(|s| {
                        s.iter()
                            .map(|p| [round2(p[0] - b[0]), round2(p[1] - b[1])])
                            .collect()
                    })
                    .collect();
                SavedSignature::Drawn(DrawnSig {
                    v: 1,
                    width: round2(b[2] - b[0]),
                    height: round2(b[3] - b[1]),
                    strokes,
                })
            }
        };
        let ok = serde_json::to_value(&value).is_ok_and(|v| self.host.signature.set(Some(&v)));
        if !ok {
            self.dom
                .set_text(ids::SIGPAD_STATUS, "The signature could not be saved");
            return false;
        }
        self.load_saved_signature();
        let then_arm = std::mem::take(&mut self.pad_then_arm);
        self.close_sigpad();
        self.announce("Signature saved");
        if then_arm {
            self.arm_sign();
        }
        true
    }

    /// R:4869 `removeDrawnSignature()`.
    pub fn remove_drawn_signature(&mut self) -> bool {
        if !self.host.signature.set(None) {
            return false;
        }
        self.load_saved_signature();
        self.dom
            .set_text(ids::SIGPAD_STATUS, "Saved signature removed");
        self.announce("Saved signature removed");
        true
    }

    /// R:4878 `armSign()`: the next plain click on the page places, or Enter at the page centre.
    pub fn arm_sign(&mut self) {
        if self.doc.is_none() || !self.can_annotate() {
            return;
        }
        if self.ui_mode != UiMode::Forms {
            self.set_mode(UiMode::Forms);
        }
        self.sign_armed = true;
        set_pressed(&mut self.dom, ids::FILL_SIGN_BUTTON, true);
        let (w, h) = self.canvas_client();
        let page = self.shown_page;
        self.paint_hits(w, h, page);
        self.focus_page();
        self.announce("Click the page to place your signature");
    }

    /// R:4886 `disarmSign()`.
    pub fn disarm_sign(&mut self) {
        if !self.sign_armed {
            return;
        }
        self.sign_armed = false;
        set_pressed(&mut self.dom, ids::FILL_SIGN_BUTTON, false);
        let (w, h) = self.canvas_client();
        let page = self.shown_page;
        self.paint_hits(w, h, page);
    }

    /// R:4892 `pressSign()`: a second press disarms; nothing saved opens the pad first.
    pub fn press_sign(&mut self) {
        if self.sign_armed {
            self.disarm_sign();
            return;
        }
        if self.doc.is_none() || !self.can_annotate() {
            return;
        }
        if self.saved_signature.is_none() {
            self.open_sigpad(true);
            return;
        }
        self.arm_sign();
    }

    /// R:4898 `placeSignatureAtCentre()`.
    pub fn place_signature_at_centre(&mut self) {
        let page = self.wanted_page;
        match self.page_size(page) {
            Some(size) => {
                self.place_signature(page, [size.width / 2.0, size.height / 2.0], None);
            }
            None => self.disarm_sign(),
        }
    }

    /// R:4903 `placeSignature()`: fit inside SIGNATURE_SIZE (or `width`), centred on the point, inside the page.
    pub fn place_signature(&mut self, page: i64, point: [f64; 2], width: Option<f64>) -> bool {
        self.disarm_sign();
        let Some(size) = self.page_size(page).filter(|_| self.doc.is_some()) else {
            return false;
        };
        if self.rotation(page) % 360 != 0 {
            return self.refuse_signature(SIGN_ROTATED);
        }
        let Some(sig) = self.saved_signature.clone() else {
            return self.refuse_signature("Draw your signature first");
        };
        if !self.can_annotate() {
            return self.refuse_signature("This document does not permit annotations");
        }
        let [box_w, box_h] = SIGNATURE_SIZE;
        let (w0, h0) = match &sig {
            SavedSignature::Image(i) => (i.width, i.height),
            SavedSignature::Drawn(d) => (d.width, d.height),
        };
        let scale = match width {
            // Review: the default fit is also inside the page.
            None => (box_w / w0).min(size.width / w0).min(if h0 > 0.0 {
                box_h.min(size.height) / h0
            } else {
                f64::INFINITY
            }),
            Some(width) => {
                width.max(SIGN_MIN_WIDTH).min(size.width).min(if h0 > 0.0 {
                    size.height * w0 / h0
                } else {
                    f64::INFINITY
                }) / w0
            }
        };
        let (w, h) = (w0 * scale, h0 * scale);
        let left = 0.0f64.max((size.width - w).min(point[0] - w / 2.0));
        let top = 0.0f64.max((size.height - h).min(point[1] - h / 2.0));
        let at =
            |value: f64, most: f64| 0.0f64.max(most.min(crate::js::js_round(value * 1e4) / 1e4));
        self.clear_error();
        let result = (|| -> Result<(), Refusal> {
            let before = self.annot_ids(page)?;
            // sign-merge D4: an image is one signature Stamp over the fitted rect.
            let reply = match &sig {
                SavedSignature::Image(image) => {
                    let bytes = crate::host::util::base64_decode_lenient(&image.data);
                    let rect = [
                        at(left, size.width),
                        at(top, size.height),
                        at(left + w, size.width),
                        at(top + h, size.height),
                    ];
                    self.rq_with(json!({"op": "annot-add", "page": page, "type": "Stamp", "rect": rect, "image": {"name": "signature", "bytes": {"$buffer": 0}}, "signature": true, "contents": ""}), vec![bytes])?
                }
                SavedSignature::Drawn(drawn) => {
                    let strokes: Vec<Vec<[f64; 2]>> = drawn
                        .strokes
                        .iter()
                        .map(|s| {
                            s.iter()
                                .map(|p| {
                                    [
                                        at(left + p[0] * scale, size.width),
                                        at(top + p[1] * scale, size.height),
                                    ]
                                })
                                .collect()
                        })
                        .collect();
                    self.rq(json!({"op": "annot-add", "page": page, "type": "Ink", "strokes": strokes, "contents": "", "signature": true}))?
                }
            };
            self.note_filled();
            self.apply_edit(reply.value, "Added signature");
            let id = self
                .notes
                .iter()
                .find(|i| is_signature(i) && !before.contains(&i.id))
                .map_or(-1, |i| i.id);
            self.fill_sign_last = Some(Self::fill_op("add", id, self.note_rect2(id)));
            self.direct_focused_sig = (id >= 0).then_some(id);
            if id >= 0 {
                self.focus_node(&NodeKey::Signature(id));
            }
            Ok(())
        })();
        match result {
            Ok(()) => true,
            Err(err) => {
                let text = error_text(&err);
                self.refuse_signature(&text)
            }
        }
    }

    /// `placeSignature`'s `refuse`: announced and shown.
    fn refuse_signature(&mut self, message: &str) -> bool {
        self.announce(message);
        self.show_error(message);
        false
    }

    /// R:4937 `commitSignatureRect()`: one `annot-move` (point = new top-left; `width` on a resize).
    pub fn commit_signature_rect(
        &mut self,
        annot_id: i64,
        page: i64,
        initial: &[f64],
        rect: &[f64],
        resize: bool,
    ) -> bool {
        self.direct_focused_sig = Some(annot_id);
        let (w0, h0) = (initial[2] - initial[0], initial[3] - initial[1]);
        let width = if !resize {
            None
        } else if (rect[2] - rect[0] - w0).abs() <= 0.01 && h0 > 0.0 {
            Some(w0 * (rect[3] - rect[1]) / h0)
        } else {
            Some(rect[2] - rect[0])
        };
        let mut message = Map::new();
        message.insert("op".into(), json!("annot-move"));
        message.insert("page".into(), json!(page));
        message.insert("annotId".into(), json!(annot_id));
        message.insert("point".into(), json!(round_rect4(&rect[..2])));
        if let Some(width) = width {
            message.insert(
                "width".into(),
                json!(crate::js::js_round(width * 1e4) / 1e4),
            );
        }
        match self.rq(Value::Object(message)) {
            Ok(reply) => {
                if reply.get("noop") == Some(&Value::Bool(true)) {
                    return true;
                }
                let resized = width.is_some_and(|w| (w - w0).abs() > 0.01);
                self.note_filled();
                self.apply_edit(
                    reply.value,
                    if resized {
                        "Resized signature"
                    } else {
                        "Moved signature"
                    },
                );
                self.fill_sign_last = Some(Self::fill_op(
                    if resized { "resize" } else { "move" },
                    annot_id,
                    self.note_rect2(annot_id),
                ));
                true
            }
            Err(err) => {
                self.show_error(&error_text(&err));
                false
            }
        }
    }

    /// R:4955 `deleteSignature()`.
    pub fn delete_signature(&mut self, annot_id: i64) {
        self.direct_focused_sig = None;
        self.delete_note(annot_id, "Deleted signature");
        if self.notes.iter().any(|i| i.id == annot_id) {
            return;
        }
        self.note_filled();
        self.fill_sign_last = Some(Self::fill_op("delete", annot_id, None));
    }

    /// R:4962 `paintSignatureTarget()`: drag moves, handles resize (aspect kept, Shift too),
    /// arrows nudge, +/- resize 10 %, Delete removes; click and Enter only focus.
    pub fn paint_signature_target(&mut self, item: &AnnotItem, page: i64, index: usize) -> HitNode {
        let can = self.can_annotate();
        let mut node = HitNode::new(
            NodeKind::Signature {
                item: item.clone(),
                page,
                can,
            },
            &item.rect,
            self.shown_zoom,
        );
        node.classes.push("signature-target");
        node.label = format!("Signature {}", index + 1);
        node.title = if can {
            "Signature: drag to move, handles or +/- resize, Delete removes it".into()
        } else {
            "This document is read-only".into()
        };
        if can {
            node.handles = FIELD_HANDLES.to_vec();
        }
        node
    }

    /// R:4971 the signature's `start(event, handle)`.
    pub(crate) fn signature_start(
        &mut self,
        item: &AnnotItem,
        page: i64,
        ev: &mut PtrEvent,
        handle: &str,
        key: NodeKey,
    ) {
        let rect = item.rect.clone();
        let (w0, h0) = (rect[2] - rect[0], rect[3] - rect[1]);
        // gestureRect's corner lock keeps both sides ≥ min; this min keeps the width ≥ 24 pt.
        let min = if h0 > 0.0 && w0 > 0.0 {
            SIGN_MIN_WIDTH * 1.0f64.min(h0 / w0)
        } else {
            SIGN_MIN_WIDTH
        };
        self.direct_focused_sig = Some(item.id);
        let can = self.can_annotate();
        let options = RectGesture {
            rect: rect.clone(),
            id: TargetRef::Number(item.id),
            handle: handle.into(),
            min,
            lock_aspect: true,
            keep_aspect: true,
            can,
            commit: RectCommit::Signature {
                annot_id: item.id,
                page,
                initial: rect,
                resize: handle != "grip",
            },
            ..RectGesture::default()
        };
        self.start_rect_gesture(ev, Some(key), options);
    }

    /// R:4980 the signature's keys.
    pub(crate) fn signature_keydown(&mut self, item: &AnnotItem, page: i64, ev: &mut KeyEv) {
        if ev.key == "Escape" {
            ev.prevent();
            self.drop_direct_focus();
            return;
        }
        let size = self.page_size(page);
        let (id, rect) = (item.id, item.rect.clone());
        let (w0, h0) = (rect[2] - rect[0], rect[3] - rect[1]);
        type Work = Box<dyn FnOnce(&mut Session) -> Result<(), Refusal>>;
        let mut work: Option<Work> = None;
        if ev.is_arrow() && !ev.meta && !ev.ctrl {
            if let Some(size) = size
                && self.can_annotate()
            {
                let next =
                    super::object_menu::nudge_rect(&rect, &ev.key, ev.shift, false, 1.0, &size);
                work = Some(Box::new(move |s: &mut Session| {
                    s.commit_signature_rect(id, page, &rect, &next, false);
                    Ok(())
                }));
            }
        } else if ev.key == "+" || ev.key == "=" || ev.key == "-" {
            if self.can_annotate() {
                let factor = if ev.key == "-" { 0.9 } else { 1.1 };
                let next = vec![
                    rect[0],
                    rect[1],
                    rect[0] + w0 * factor,
                    rect[1] + h0 * factor,
                ];
                work = Some(Box::new(move |s: &mut Session| {
                    s.commit_signature_rect(id, page, &rect, &next, true);
                    Ok(())
                }));
            }
        } else if ev.key == "Delete" || ev.key == "Backspace" {
            if self.can_annotate() {
                work = Some(Box::new(move |s: &mut Session| {
                    s.delete_signature(id);
                    Ok(())
                }));
            }
        } else if ev.key == "Enter" || ev.key == " " {
            ev.prevent();
            self.direct_focused_sig = Some(id);
            return;
        } else {
            return;
        }
        ev.prevent();
        if let Some(work) = work
            && !self.lane2.gesture_work
        {
            self.run_gesture_work(work);
        }
    }

    /// R:4996 `driveSignDraw()`: `sign-draw {strokes}|{image:true}|{clear:true}` runs the pad's
    /// own recorder and Save. No document, no journal, never dirty.
    pub fn drive_sign_draw(&mut self, contents: Option<&str>) {
        self.clear_error();
        // Review: preload sends an over-cap (> 4 MiB) sign-draw as "" so it is refused here, not dropped.
        if contents == Some("") {
            self.show_error("sign-draw: contents are empty or larger than 4 MiB");
            return;
        }
        let Some(value) = self.drive_wanted(contents) else {
            return;
        };
        if ["strokes", "image", "clear"]
            .iter()
            .filter(|k| value.contains_key(**k))
            .count()
            > 1
        {
            self.show_error("sign-draw: give one of strokes, image or clear");
            return;
        }
        if value.get("image") == Some(&Value::Bool(true)) {
            self.pad_then_arm = false;
            if let Some(refused) = self.pad_use_image() {
                let text = if refused.is_empty() {
                    "no image was given".to_string()
                } else {
                    refused
                };
                self.show_error(&format!("sign-draw: {text}"));
                return;
            }
            if !self.save_drawn_signature() {
                self.show_error("sign-draw: the signature could not be saved");
            }
            return;
        }
        if value.get("clear") == Some(&Value::Bool(true)) {
            if !self.remove_drawn_signature() {
                self.show_error("sign-draw: the saved signature could not be removed");
            }
            return;
        }
        let strokes = value
            .get("strokes")
            .and_then(Value::as_array)
            .and_then(|strokes| {
                strokes
                    .iter()
                    .map(|stroke| {
                        let points = stroke.as_array().filter(|p| !p.is_empty())?;
                        points
                            .iter()
                            .map(|p| {
                                let p = p.as_array().filter(|p| p.len() == 2)?;
                                let (x, y) = (finite_of(p.first())?, finite_of(p.get(1))?);
                                Some([x, y])
                            })
                            .collect::<Option<Vec<Point>>>()
                    })
                    .collect::<Option<Vec<Vec<Point>>>>()
            });
        let Some(strokes) = strokes else {
            self.show_error("sign-draw: strokes must be a list of strokes of [x, y] points, or clear must be true");
            return;
        };
        if strokes.len() > MAX_PAD_STROKES {
            self.show_error("sign-draw: a signature can have at most 64 strokes");
            return;
        }
        self.pad_then_arm = false;
        self.clear_pad();
        for stroke in &strokes {
            self.pad_begin(stroke[0]);
            for point in &stroke[1..] {
                self.pad_point(*point);
            }
            self.pad_end();
        }
        if self.pad_full {
            self.clear_pad();
            self.show_error("sign-draw: a signature can have at most 4096 points (points under 2 px apart are skipped)");
            return;
        }
        if self.dom.disabled(ids::SIGPAD_SAVE_BUTTON) {
            self.show_error("sign-draw: the drawing needs a stroke and must be at least 8 px wide");
            return;
        }
        if !self.save_drawn_signature() {
            self.show_error("sign-draw: the signature could not be saved");
        }
    }

    /// R:5021 `driveSignPlace()`: `sign-place {page, x, y, width?}`; no contents arms (the menu path).
    pub fn drive_sign_place(&mut self, contents: Option<&str>) {
        self.clear_error();
        if contents.is_none() {
            self.press_sign();
            return;
        }
        let Some(value) = self.drive_wanted(contents) else {
            return;
        };
        let Some(page_count) = self.doc.as_ref().map(|d| d.page_count) else {
            return;
        };
        let (Some(page), Some(x), Some(y)) = (
            int_of(value.get("page")),
            finite_of(value.get("x")),
            finite_of(value.get("y")),
        ) else {
            self.show_error("sign-place: page, x and y are required");
            return;
        };
        let width = match value.get("width") {
            None => None,
            Some(w) => match finite_of(Some(w)) {
                Some(w) => Some(w),
                None => {
                    self.show_error("sign-place: width must be a number");
                    return;
                }
            },
        };
        if page < 0 || page >= page_count {
            self.show_error(&format!(
                "sign-place: page must be an integer between 0 and {}",
                page_count - 1
            ));
            return;
        }
        if self.saved_signature.is_none() {
            self.show_error("sign-place: no signature has been drawn");
            return;
        }
        if self.rotation(page) % 360 != 0 {
            self.announce(SIGN_ROTATED);
            self.show_error(SIGN_ROTATED);
            return;
        }
        if !self.can_annotate() {
            self.show_error("sign-place: this document does not permit annotations");
            return;
        }
        if self.direct_active.is_some() {
            self.finish_direct();
        }
        self.go_to(page);
        self.drain();
        self.place_signature(page, [x, y], width);
    }

    // ------------------------------------------------------------------ buttons (R:5038-5059)

    /// R:5038 `#fill-sign` click: a press while armed disarms, else the Sign choice.
    pub fn on_fill_sign_click(&mut self) {
        if self.sign_armed {
            self.disarm_sign();
        } else {
            self.open_sign_choice();
        }
    }

    /// R:5039 `#fill-sign-draw` click.
    pub fn on_fill_sign_draw_click(&mut self) {
        if self.doc.is_some() && self.can_annotate() {
            self.open_sigpad(false);
        }
    }

    /// R:5040-5045 the pad's buttons: "clear", "image", "remove", "save", "cancel".
    pub fn on_sigpad_button(&mut self, which: &str) {
        match which {
            "clear" => self.clear_pad(),
            "image" => {
                if let Some(refused) = self.pad_use_image().filter(|r| !r.is_empty()) {
                    self.dom.set_text(ids::SIGPAD_STATUS, refused);
                }
            }
            "remove" => {
                self.remove_drawn_signature();
            }
            "save" => {
                self.save_drawn_signature();
            }
            "cancel" => self.close_sigpad(),
            _ => {}
        }
    }

    /// R:5047-5059 the pad canvas's pointer, in pad CSS px: "down" (primary), "move", "up", "cancel".
    pub fn on_sigpad_pointer(&mut self, phase: &str, x: f64, y: f64) {
        match phase {
            "down" => {
                self.pad_begin([x, y]);
            }
            "move" => {
                if self.pad_stroke.is_some() {
                    self.pad_point([x, y]);
                }
            }
            "up" | "cancel" => self.pad_end(),
            _ => {}
        }
    }
}

/// The UTF-16 length of a string (`value.length`).
pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// R:5209 `addDraft()`: leading and trailing blank lines are dropped; whitespace only is empty.
pub fn add_draft(value: &str) -> String {
    let mut lines: Vec<&str> = value.split('\n').collect();
    while lines
        .first()
        .is_some_and(|l| crate::js::js_trim(l).is_empty())
    {
        lines.remove(0);
    }
    while lines
        .last()
        .is_some_and(|l| crate::js::js_trim(l).is_empty())
    {
        lines.pop();
    }
    lines.join("\n")
}

/// Editor heights follow `growParagraphEditor` (kept here for the note/text-box editors' tests).
pub fn grow(editor: &mut EditorState, min: f64) {
    grow_paragraph_editor(editor, Some(min));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drafts_drop_blank_edges() {
        assert_eq!(add_draft("\n  \nHello\n world \n\n"), "Hello\n world ");
        assert_eq!(add_draft(" \n\t"), "");
    }

    #[test]
    fn the_pad_records_like_the_canvas() {
        let mut s = Session::new();
        assert!(s.pad_begin([0.0, 0.0]));
        s.pad_point([1.0, 1.0]); // under 2 px: skipped
        s.pad_point([10.0, 0.0]);
        s.pad_end();
        assert_eq!(s.pad_strokes, vec![vec![[0.0, 0.0], [10.0, 0.0]]]);
        assert!(!s.dom.disabled(ids::SIGPAD_SAVE_BUTTON));
        s.pad_begin([50.0, 50.0]);
        s.pad_end(); // a tap is a dot
        assert_eq!(s.pad_strokes[1], vec![[50.0, 50.0], [50.5, 50.0]]);
        s.clear_pad();
        assert!(s.pad_strokes.is_empty() && s.dom.disabled(ids::SIGPAD_SAVE_BUTTON));
    }
}
