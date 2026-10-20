//! Shapes and stamps, unit 9 (renderer.ts 8648–8954): where a Square/Circle/Line/Ink/stamp or
//! a signature appearance goes, the adds themselves, the stamp icon dialog, the `shapes` smoke,
//! note edits and deletes, undo and redo, and the note/highlight/free-text smokes.
#![allow(dead_code)]

use super::Session;
use super::consts;
use super::dom_ids as ids;
use super::field_designer::round2;
use super::form_fields::{journal_of, nums};
use super::markup::r2;
use super::types::*;
use crate::host::PickedFile;
use serde_json::{Value, json};
use tandem_protocol::engine::PageReply;
use tandem_protocol::limits::{NOTE_SIZE, SHAPE_SIZE, SIGNATURE_SIZE};

/// R:8699: the stand-in Ink the keyboard path draws: a 4-point zigzag along the box's diagonal.
pub fn ink_zigzag(rect: &[f64]) -> Vec<Point> {
    let [x0, y0, x1, y1] = [rect[0], rect[1], rect[2], rect[3]];
    vec![
        [x0, y0],
        [x0 + (x1 - x0) / 3.0, y0 + ((y1 - y0) * 2.0) / 3.0],
        [x0 + ((x1 - x0) * 2.0) / 3.0, y0 + (y1 - y0) / 3.0],
        [x1, y1],
    ]
}

/// The stamp dialog's choices (index.html `stamp-icon` radios), Draft first.
pub const STAMP_ICONS: [&str; 6] = [
    "Draft",
    "Approved",
    "Confidential",
    "Final",
    "NotApproved",
    "Void",
];
impl Session {
    /// The wanted page's size.
    pub(crate) fn wanted_size(&self) -> Option<PageSize> {
        self.doc
            .as_ref()?
            .pages
            .get(usize::try_from(self.wanted_page).ok()?)
            .cloned()
    }

    /// R:8651: a `width`x`height` box with its top-left at `point`, kept inside the page box.
    pub fn box_at(&self, point: Point, width: f64, height: f64) -> Option<Vec<f64>> {
        let size = self.wanted_size()?;
        let w = round2(width).min(size.width);
        let h = round2(height).min(size.height);
        let x = round2(point[0]).min(size.width - w).max(0.0);
        let y = round2(point[1]).min(size.height - h).max(0.0);
        // The far corner is clamped to the page too: a hit at the edge must not round past it.
        Some(vec![
            x,
            y,
            (x + w).min(size.width),
            (y + h).min(size.height),
        ])
    }

    /// R:8663: the current hit's quad union padded 4 pt, kept inside the page box.
    pub fn hit_box(&self) -> Option<Vec<f64>> {
        let hit = self.current_hit()?;
        let rect = super::bookmarks_links::hit_rect(&hit)?;
        self.box_at(
            [rect[0] - 4.0, rect[1] - 4.0],
            rect[2] - rect[0] + 8.0,
            rect[3] - rect[1] + 8.0,
        )
    }

    /// R:8671: the hit box, else a SHAPE_SIZE box at the page centre.
    pub fn shape_box(&self) -> Option<Vec<f64>> {
        if let Some(anchored) = self.hit_box() {
            return Some(anchored);
        }
        let size = self.wanted_size()?;
        self.box_at(
            [size.width / 2.0, size.height / 2.0],
            SHAPE_SIZE[0],
            SHAPE_SIZE[1],
        )
    }

    /// R:8679: the signature's own box: SIGNATURE_SIZE centred on the hit, else on the page centre.
    pub fn signature_box(&self) -> Option<Vec<f64>> {
        let size = self.wanted_size()?;
        let centre = match self.hit_box() {
            Some(hit) => [(hit[0] + hit[2]) / 2.0, (hit[1] + hit[3]) / 2.0],
            None => [size.width / 2.0, size.height / 2.0],
        };
        let [width, height] = SIGNATURE_SIZE;
        self.box_at(
            [centre[0] - width / 2.0, centre[1] - height / 2.0],
            width,
            height,
        )
    }

    /// R:8691: a drag point (page pt, from the UI) kept inside the page box.
    pub fn ink_point(&self, point: Point) -> Option<Point> {
        let size = self.wanted_size()?;
        Some([
            point[0].min(size.width).max(0.0),
            point[1].min(size.height).max(0.0),
        ])
    }

    /// R:8710: true (and announced) when the open document cannot take an annotation.
    pub fn refuses_add(&mut self) -> bool {
        if self.doc.is_some() && self.editable() {
            return false;
        }
        self.show_error("This document is read-only, so annotations cannot be added.");
        true
    }

    /// One `annot-add`, then applyEdit; a refusal shows its message.
    pub(crate) fn annot_add(&mut self, request: Value, announcement: &str) -> bool {
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

    /// R:8717: one Square/Circle (its rect) or Line (left-mid → right-mid of the box).
    pub fn add_shape(&mut self, kind: &str, rect: &[f64], announcement: &str) -> bool {
        if self.refuses_add() {
            return false;
        }
        let page = self.wanted_page;
        let request = if kind == "Line" {
            let middle = (rect[1] + rect[3]) / 2.0;
            json!({"op": "annot-add", "page": page, "type": kind, "points": [nums(&[rect[0], middle]), nums(&[rect[2], middle])], "contents": ""})
        } else {
            json!({"op": "annot-add", "page": page, "type": kind, "rect": nums(rect), "contents": ""})
        };
        self.annot_add(request, announcement)
    }

    /// R:8734: one Ink with the strokes a pointer drag (or the keyboard stand-in) recorded.
    pub fn add_ink(&mut self, strokes: &[Vec<Point>], announcement: &str) -> bool {
        if self.refuses_add() {
            return false;
        }
        let strokes: Vec<Value> = strokes
            .iter()
            .map(|stroke| Value::Array(stroke.iter().map(|p| nums(p)).collect()))
            .collect();
        let request = json!({"op": "annot-add", "page": self.wanted_page, "type": "Ink", "strokes": strokes, "contents": ""});
        self.annot_add(request, announcement)
    }

    /// R:8747: one icon stamp; the engine synthesises the appearance for the six icons.
    pub fn add_icon_stamp(&mut self, rect: &[f64], icon: &str) -> bool {
        if self.refuses_add() {
            return false;
        }
        let request = json!({"op": "annot-add", "page": self.wanted_page, "type": "Stamp", "rect": nums(rect), "icon": icon, "contents": ""});
        self.annot_add(request, &format!("Added stamp {icon}"))
    }

    /// R:8762: one signature appearance: an image stamp whose `/Subj` says it is a picture, not
    /// a signature. The engine decodes the image.
    pub fn add_signature(&mut self, rect: &[f64], name: &str, image: &PickedFile) -> bool {
        if self.refuses_add() {
            return false;
        }
        let request = json!({
            "op": "annot-add", "page": self.wanted_page, "type": "Stamp", "rect": nums(rect),
            "image": {"name": image.name, "bytes": {"$buffer": 0}}, "sign": {"name": name}, "contents": "",
        });
        match self.request(request, vec![image.bytes.clone()]) {
            Ok(reply) => {
                self.apply_edit(
                    &journal_of(&reply),
                    "Added signature appearance (a picture, not a certificate signature)",
                );
                true
            }
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
                false
            }
        }
    }

    /// R:8777: the Insert menu's four geometric shapes: no dialog, one add each.
    pub fn add_shape_op(&mut self, op: &str) {
        if self.doc.is_none() || !self.can_annotate() {
            return;
        }
        let Some(rect) = self.shape_box() else {
            return;
        };
        match op {
            "ink" => {
                self.add_ink(&[ink_zigzag(&rect)], "Added ink (4 points)");
            }
            "line" => {
                self.add_shape("Line", &rect, "Added line");
            }
            "circle" => {
                self.add_shape("Circle", &rect, "Added circle");
            }
            _ => {
                self.add_shape("Square", &rect, "Added square");
            }
        }
    }

    /// R:8787.
    pub fn open_stamp_icon_dialog(&mut self) {
        if self.doc.is_none() || !self.can_annotate() {
            return;
        }
        self.dom.set_open(ids::STAMP_ICON_DIALOG, true);
        let icon = self.lane4.stamp_icon.clone();
        self.check_stamp_icon(&icon);
    }

    /// UI entry: one of the dialog's `stamp-icon` radios was checked (the window reports it
    /// as `radio:stamp-icon:<value>`; the value is the stamp's icon name).
    pub fn on_stamp_icon_choose(&mut self, icon: &str) {
        if STAMP_ICONS.contains(&icon) {
            self.lane4.stamp_icon = icon.to_string();
            self.check_stamp_icon(icon);
        }
    }

    /// The dialog's radios show `icon` chosen (Draft when the dialog first opens).
    pub(crate) fn check_stamp_icon(&mut self, icon: &str) {
        for name in STAMP_ICONS {
            self.dom
                .set_checked(&format!("radio:stamp-icon:{name}"), name == icon);
        }
    }

    /// R:8793.
    pub fn save_stamp_icon_dialog(&mut self) {
        let icon = self.lane4.stamp_icon.clone();
        let rect = self.shape_box();
        self.dom.set_open(ids::STAMP_ICON_DIALOG, false);
        if let Some(rect) = rect
            && !icon.is_empty()
        {
            self.add_icon_stamp(&rect, &icon);
        }
    }

    /// R:8805: `--edit=shapes`: search `one`, then one of each new type over the hit, a Draft
    /// stamp, and a signature appearance from `--images` (last, so one undo leaves one Stamp).
    pub fn smoke_shapes(&mut self, contents: Option<&str>) {
        if self.doc.is_none() {
            self.log_edit();
            return;
        }
        let name = contents.unwrap_or("PDF4All P2 shapes").to_string();
        self.dom.set_value(ids::SEARCH_INPUT, "one");
        self.start_search();
        let rect = self.shape_box();
        if let Some(rect) = &rect {
            self.add_shape("Square", rect, "Added square");
            self.add_shape("Circle", rect, "Added circle");
            self.add_shape("Line", rect, "Added line");
            self.add_ink(&[ink_zigzag(rect)], "Added ink (4 points)");
            self.add_icon_stamp(rect, "Draft");
        }
        let image = self
            .host
            .pick_images()
            .ok()
            .and_then(|images| images.into_iter().next());
        let signature = self.signature_box();
        if let (Some(image), Some(signature)) = (image, signature) {
            self.add_signature(&signature, &name, &image);
        }
        self.log_edit();
    }

    /// R:8833.
    pub fn edit_note(&mut self, annot_id: i64, contents: &str) {
        let request = json!({"op": "annot-set", "page": self.wanted_page, "annotId": annot_id, "contents": contents});
        match self.request(request, vec![]) {
            Ok(reply) => self.apply_edit(&journal_of(&reply), "Edited note"),
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
            }
        }
    }

    /// R:8842 (TS default announcement: "Deleted note").
    pub fn delete_note(&mut self, annot_id: i64, announcement: &str) {
        if self
            .notes
            .iter()
            .find(|item| item.id == annot_id)
            .is_some_and(|item| item.r#type == "Redact")
        {
            self.redact_deletes
                .insert(format!("{}:{annot_id}", self.wanted_page));
        }
        let request = json!({"op": "annot-delete", "page": self.wanted_page, "annotId": annot_id});
        match self.request(request, vec![]) {
            Ok(reply) => self.apply_edit(&journal_of(&reply), announcement),
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
            }
        }
    }

    /// The shared tail of undo and redo (R:8859-8871): a page step refreshes like a page op.
    fn undo_redo(&mut self, op: &str, verb: &str, plain: &str) {
        self.clear_error(); // Bugfix 1 D5: a performed undo clears the error bar; a failing one shows its own.
        let reply = match self.request(json!({"op": op}), vec![]) {
            Ok(reply) => reply,
            Err(err) => {
                let message = self.message_of(&err);
                self.show_error(&message);
                return;
            }
        };
        let reply: PageReply = super::parse_reply_or_default(&reply.value);
        let step = reply.step.clone().unwrap_or_default();
        // A reverted page op moved the page tree, so the same full refresh applies.
        if consts::PAGE_STEPS.contains(&step.as_str()) {
            let page = self.wanted_page.max(0) as usize;
            self.apply_page_edit(&reply, &format!("{verb} {step}"), page);
            self.refresh_labels();
        } else {
            let journal = tandem_protocol::engine::JournalState {
                revision: reply.revision,
                dirty: reply.dirty,
                can_undo: reply.can_undo,
                can_redo: reply.can_redo,
            };
            self.apply_edit(&journal, plain);
            if consts::LABEL_STEPS.contains(&step.as_str()) {
                self.refresh_labels();
            }
        }
        self.refresh_decorate_summary();
    }

    /// R:8852.
    pub fn undo_edit(&mut self) {
        if self.direct_active.is_some() {
            self.close_direct();
            self.direct_focused_field = None;
            return;
        }
        // P6 U5 §9.28: the selection survives; the reply's repaint re-resolves each member.
        self.direct_focused_field = None;
        self.direct_focused_mark = None;
        self.direct_focused_link = None;
        self.close_object_menu(false);
        if !self.can_undo {
            return;
        }
        self.undo_redo("undo", "Undid", "Undid the last change");
    }

    /// R:8874.
    pub fn redo_edit(&mut self) {
        if self.direct_active.is_some() {
            self.close_direct();
            self.direct_focused_field = None;
            return;
        }
        self.direct_focused_field = None;
        self.direct_focused_mark = None;
        self.direct_focused_link = None;
        self.close_object_menu(false);
        if !self.can_redo {
            return;
        }
        self.undo_redo("redo", "Redid", "Redid the last change");
    }

    /// R:8895: the menu/smoke path for a note: the current page centre, with the payload's text.
    pub fn smoke_note(&mut self, contents: &str) {
        let Some(size) = self.wanted_size() else {
            return;
        };
        let page = self.wanted_page;
        let point = [
            size.width / 2.0 - NOTE_SIZE / 2.0,
            size.height / 2.0 - NOTE_SIZE / 2.0,
        ];
        self.add_note_at(page, point, contents, "Added note");
        self.log_edit();
    }

    /// R:8907: the menu/smoke path for a highlight. The smoke sends `contents`, so a cold run
    /// first finds hello.pdf's only word; the menu sends none and only acts on a hit.
    pub fn smoke_highlight(&mut self, contents: Option<&str>) {
        if self.doc.is_none() {
            return;
        }
        if self.selection_markup().is_some() {
            // Item 5: a selection comes first.
            self.mark_selection_or_match("Highlight");
            self.log_edit();
            return;
        }
        let mut hit = self.current_hit();
        if hit.is_none() && contents.is_some() {
            self.dom.set_value(ids::SEARCH_INPUT, "PDF4All");
            self.start_search();
            hit = self.current_hit();
        }
        let Some(hit) = hit else {
            self.announce("Find something first");
            return;
        };
        if self.add_markup(hit.page, "Highlight", &hit.quads, "Highlighted match") {
            self.markup_last = Some(MarkupLast {
                kind: "Highlight".into(),
                page: hit.page,
                source: MarkupSource::Search,
                quads: hit
                    .quads
                    .iter()
                    .map(|quad| quad.iter().map(|v| r2(*v)).collect())
                    .collect(),
                text: None,
            });
        }
        self.log_edit();
    }

    /// R:8928: the menu opens the dialog for a typed box; the smoke supplies the text and lands
    /// a box at the page centre.
    pub fn smoke_free_text(&mut self, contents: Option<&str>) {
        let Some(size) = self.wanted_size() else {
            return;
        };
        let point = [size.width / 2.0, size.height / 2.0];
        let Some(contents) = contents else {
            self.open_text_box_dialog(point);
            return;
        };
        let page = self.wanted_page;
        self.add_text_box_at(page, point, contents);
        self.log_edit();
    }

    /// R:8941: `edit-request {op:'undo'}` from main's menu or the smoke driver.
    pub fn smoke_undo(&mut self) {
        if self.direct_active.is_some() {
            // §8.7: editor open — Esc, journal untouched.
            self.close_direct();
            self.direct_focused_field = None;
            return;
        }
        self.undo_edit();
        self.log_edit();
        if self.content_smoke_active {
            self.log_content_smoke();
        }
    }

    /// R:8948.
    pub fn smoke_redo(&mut self) {
        if self.direct_active.is_some() {
            self.close_direct();
            self.direct_focused_field = None;
            return;
        }
        self.redo_edit();
        self.log_edit();
        if self.content_smoke_active {
            self.log_content_smoke();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zigzag_runs_corner_to_corner() {
        assert_eq!(
            ink_zigzag(&[0.0, 0.0, 30.0, 60.0]),
            vec![[0.0, 0.0], [10.0, 40.0], [20.0, 20.0], [30.0, 60.0]]
        );
    }

    #[test]
    fn boxes_stay_inside_the_page() {
        let mut session = Session::new();
        assert_eq!(session.box_at([0.0, 0.0], 10.0, 10.0), None);
        session.doc = Some(OpenDoc {
            page_count: 1,
            pages: vec![PageSize {
                width: 100.0,
                height: 50.0,
            }],
            ..OpenDoc::default()
        });
        session.wanted_page = 0;
        assert_eq!(
            session.box_at([95.0, -5.0], 20.0, 10.0),
            Some(vec![80.0, 0.0, 100.0, 10.0])
        );
        assert_eq!(
            session.box_at([10.0, 10.0], 500.0, 10.0),
            Some(vec![0.0, 10.0, 100.0, 20.0])
        );
        // No hit: the shape box sits at the page centre, pushed back inside.
        assert_eq!(session.shape_box(), Some(vec![0.0, 0.0, 100.0, 50.0]));
        assert_eq!(session.signature_box(), Some(vec![0.0, 0.0, 100.0, 50.0]));
        assert_eq!(session.ink_point([120.0, -3.0]), Some([100.0, 0.0]));
    }
}
