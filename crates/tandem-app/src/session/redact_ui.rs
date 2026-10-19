//! The redaction tool's own surfaces and words (redact-ui.ts, "draw a box, everything inside is
//! gone"): the chip and "Show what's underneath" toggle, the "Redactions (N)" side panel, the
//! "Find text to redact…" helper, the confirmation dialog, the pending-boxes save prompt, the
//! flatten/unsigned questions and the banner over a redacted copy. `createRedactUi`'s closure
//! state is `Session::redact_ui`; its methods are `redact_ui_*` here. Its `RedactHost`
//! (R:995-1022, lane 1) is `redact_ui_pages`, `redact_ui_boxes_on`, `redact_ui_delete_box`,
//! `redact_ui_can_edit` and `redact_ui_can_annotate`; the rest of the host is direct calls. The in-page questions (`askPending`,
//! `askYesNo`, `askThree`) are native message boxes on `self.host.platform`; a headless run
//! never reaches them (every caller answers from the drive's pre-answers first).
#![allow(dead_code)]

use super::Session;
use super::dom_ids::redact_ui as ids;
use super::types::{
    AnnotItem, PageSize, PendingAction, PendingChoice, RedactBannerState, RedactBox,
    RedactInspectRegion, RedactPageGroup, RedactPreview, RedactTint, TintKind, TintShape,
};
use crate::messages::redact as words;
use crate::platform::{BoxKind, MessageBox, Question as Asked};
use serde_json::{Map, Value, json};
use tandem_protocol::engine::{RedactInventory, RedactSanitize};

// ------------------------------------------------------------------ pure helpers

/// U:17.
pub const SANITIZE_KEYS: [&str; 9] = [
    "info",
    "xmp",
    "attachments",
    "outlines",
    "annots",
    "links",
    "fields",
    "layers",
    "struct",
];

/// U:51: whether main asks about unsaved boxes before a file replaces the window's document.
/// D6's signed copy skips the question for marks (the copy keeps them), but not for a signed
/// file's boxes in memory: the copy does not have them.
pub fn asks_boxes_on_open(skip_discard: bool, pending: Option<(i64, bool)>) -> bool {
    match pending {
        Some((count, memory)) if count >= 1 => !skip_discard || memory,
        _ => false,
    }
}

/// U:60: the side panel's order: pages ascending, boxes top to bottom then left to right.
pub fn group_by_page(boxes: &[RedactBox]) -> Vec<RedactPageGroup> {
    let mut pages: Vec<RedactPageGroup> = Vec::new();
    for b in boxes {
        match pages.iter_mut().find(|g| g.page == b.page) {
            Some(group) => group.boxes.push(b.clone()),
            None => pages.push(RedactPageGroup {
                page: b.page,
                boxes: vec![b.clone()],
            }),
        }
    }
    pages.sort_by_key(|g| g.page);
    for group in &mut pages {
        let at = |b: &RedactBox, i: usize| b.rect.get(i).copied().unwrap_or(0.0);
        // A stable sort, as Array#sort is (a total order, so a NaN cannot upset it).
        group.boxes.sort_by(|a, b| {
            at(a, 1)
                .total_cmp(&at(b, 1))
                .then(at(a, 0).total_cmp(&at(b, 0)))
        });
    }
    pages
}

/// U:74: "Redact whole page": the page's full box in y-down page points.
pub fn whole_page_rect(size: &PageSize) -> Vec<f64> {
    vec![
        0.0,
        0.0,
        super::content::round2(size.width),
        super::content::round2(size.height),
    ]
}

/// U:79: all nine switches on or off (the "Also remove hidden information" checkbox).
pub fn sanitize_all(on: bool) -> RedactSanitize {
    RedactSanitize {
        info: on,
        xmp: on,
        attachments: on,
        outlines: on,
        annots: on,
        links: on,
        fields: on,
        layers: on,
        r#struct: on,
    }
}

/// One switch by its key.
pub fn sanitize_get(values: &RedactSanitize, key: &str) -> bool {
    match key {
        "info" => values.info,
        "xmp" => values.xmp,
        "attachments" => values.attachments,
        "outlines" => values.outlines,
        "annots" => values.annots,
        "links" => values.links,
        "fields" => values.fields,
        "layers" => values.layers,
        "struct" => values.r#struct,
        _ => false,
    }
}

fn sanitize_set(values: &mut RedactSanitize, key: &str, on: bool) {
    match key {
        "info" => values.info = on,
        "xmp" => values.xmp = on,
        "attachments" => values.attachments = on,
        "outlines" => values.outlines = on,
        "annots" => values.annots = on,
        "links" => values.links = on,
        "fields" => values.fields = on,
        "layers" => values.layers = on,
        "struct" => values.r#struct = on,
        _ => {}
    }
}

/// Whether the preview's inventory has something under `key` (U:374).
fn inventory_has(inventory: &RedactInventory, key: &str) -> bool {
    match key {
        "xmp" => inventory.xmp,
        "info" => inventory.info > 0,
        "attachments" => inventory.attachments > 0,
        "outlines" => inventory.outlines > 0,
        "annots" => inventory.annots > 0,
        "links" => inventory.links > 0,
        "fields" => inventory.fields > 0,
        "layers" => inventory.layers > 0,
        "struct" => inventory.r#struct > 0,
        _ => false,
    }
}

/// U:84: a drive's `{hidden?, details?}` into the nine switches: `hidden` sets all (anything
/// but `false` is on), `details` overrides single ones.
pub fn sanitize_from(hidden: Option<&Value>, details: Option<&Value>) -> RedactSanitize {
    let mut out = sanitize_all(hidden != Some(&Value::Bool(false)));
    if let Some(Value::Object(map)) = details {
        for key in SANITIZE_KEYS {
            if let Some(Value::Bool(on)) = map.get(key) {
                sanitize_set(&mut out, key, *on);
            }
        }
    }
    out
}

/// U:105: what `redact-inspect` says goes, flattened into tint shapes.
pub fn tint_shapes(regions: &[RedactInspectRegion]) -> Vec<TintShape> {
    let mut out = Vec::new();
    for region in regions {
        // Over the engine's bound the reply is coarse: tint the whole box rather than a partial picture.
        if (region.truncated || region.needs_flatten) && region.rect.len() == 4 {
            out.push(TintShape {
                kind: TintKind::Shape,
                points: region.rect.clone(),
            });
            if !region.needs_flatten {
                continue;
            }
        }
        for quad in &region.glyphs {
            if quad.len() == 8 {
                out.push(TintShape {
                    kind: TintKind::Glyph,
                    points: quad.clone(),
                });
            }
        }
        for image in &region.images {
            if image.rect.len() == 4 {
                out.push(TintShape {
                    kind: TintKind::Image,
                    points: image.rect.clone(),
                });
            }
        }
        for shape in &region.shapes {
            if shape.rect.len() == 4 {
                out.push(TintShape {
                    kind: TintKind::Shape,
                    points: shape.rect.clone(),
                });
            }
        }
        for annot in &region.annots {
            if annot.rect.len() == 4 {
                out.push(TintShape {
                    kind: TintKind::Annot,
                    points: annot.rect.clone(),
                });
            }
        }
    }
    out
}

/// U:470: `${page}:${JSON.stringify(rects)}`.
pub fn inspect_key(page: i64, rects: &[Vec<f64>]) -> String {
    let rects: Vec<Value> = rects.iter().map(|r| json!(r)).collect();
    format!("{page}:{}", super::content::json_text(&Value::Array(rects)))
}

/// R:6435: the Redact items of an `annots` list as panel boxes.
pub fn boxes_of(items: &[AnnotItem], page: i64) -> Vec<RedactBox> {
    items
        .iter()
        .filter(|item| item.r#type == "Redact" && item.rect.len() == 4)
        .map(|item| RedactBox {
            page,
            id: item.id,
            rect: item.rect.clone(),
            search: item.search == Some(true),
        })
        .collect()
}

/// One polygon of the overlay (U:451-469): a mark's box (`class: "redact-mark"`) or a tint
/// shape (`"redact-tint redact-tint-{kind}"`), in CSS px at `zoom`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OverlayShape {
    pub class: String,
    /// x0,y0,x1,y1 for a mark; four corners (8 numbers) for a tint.
    pub points: Vec<f64>,
}

impl Session {
    /// R:6435 `redactBoxes(items, page)` (lane 1's `boxesOn` calls it).
    pub fn redact_boxes(&self, items: Vec<AnnotItem>, page: usize) -> Vec<RedactBox> {
        boxes_of(&items, page as i64)
    }

    // -------------------------------------------------------------- tool on/off

    /// U:232.
    pub fn redact_ui_set_active(&mut self, on: bool) {
        self.redact_ui.active = on;
        self.dom.set_hidden(ids::PANEL, !on);
        if !on {
            self.redact_ui.peek = false;
            self.dom.set_open(ids::FIND_DIALOG, false);
        }
        self.redact_ui_update();
    }

    /// U:243.
    pub fn redact_ui_set_underneath(&mut self, on: bool) {
        self.dom.set_checked(ids::UNDERNEATH_INPUT, on);
        self.redact_ui.underneath = on;
        self.repaint();
    }

    /// The "Show what's underneath" checkbox changed (U:262).
    pub fn on_redact_underneath_change(&mut self, checked: bool) {
        self.redact_ui_set_underneath(checked);
    }

    /// §2.3 step 2 (U:250): while the pointer is down on the page the boxes turn translucent.
    pub fn on_redact_page_pointer_down(&mut self, button: i64, ctrl: bool) {
        if self.redact_ui.active && button == 0 && !ctrl {
            self.redact_ui.peek = true;
        }
    }

    /// `pointerup` / `pointercancel` anywhere (U:253).
    pub fn on_redact_pointer_end(&mut self) {
        self.redact_ui.peek = false;
    }

    // -------------------------------------------------------------- panel

    /// U:266.
    pub fn redact_ui_render(&mut self) {
        let preview = self.redact_view();
        let marks = preview.as_ref().map_or(0, |p| p.marks);
        let pages = preview.as_ref().map_or(0, |p| p.pages.len() as i64);
        self.dom.set_text(ids::HEADING, words::panel_heading(marks));
        self.dom
            .set_text(ids::APPLY_OPEN, words::redact_button_text(marks));
        let can_edit = self.redact_ui_can_edit();
        self.dom
            .set_disabled(ids::APPLY_OPEN, !can_edit || marks == 0);
        self.dom
            .set_text(ids::COUNT_EL, words::count_text(marks, pages));
        let can_annotate = self.redact_ui_can_annotate();
        let has_pages = self.redact_ui_pages().is_some();
        self.dom
            .set_disabled(ids::FIND_OPEN, !can_annotate || !has_pages);
        let empty = self.dom.value(ids::SEARCH_TEXT).is_empty();
        self.dom
            .set_disabled(ids::SEARCH_GO, !can_annotate || empty);
        self.report_redact_pending();
    }

    /// U:279: after the boxes changed: the counts now, the list once its pages are read.
    pub fn redact_ui_update(&mut self) {
        self.redact_ui_render();
        if self.redact_ui.active {
            self.redact_ui_refresh_list();
        } else {
            self.redact_ui.list_seq += 1;
            self.redact_ui.boxes.clear();
        }
    }

    /// U:285.
    pub fn redact_ui_refresh_list(&mut self) {
        self.redact_ui.list_seq += 1;
        let pages = self.redact_view().map(|p| p.pages).unwrap_or_default();
        let mut found = Vec::new();
        for page in pages {
            found.extend(self.redact_ui_boxes_on(page).unwrap_or_default());
        }
        self.redact_ui.boxes = found;
        self.redact_ui_paint_list();
    }

    /// U:299 (the rows are [`Session::redact_ui_groups`]).
    pub fn redact_ui_paint_list(&mut self) {
        let empty = self.redact_ui.boxes.is_empty();
        self.dom.set_hidden(ids::EMPTY_EL, !empty);
    }

    /// The panel's list: pages, then their boxes, in panel order (U:300-336). A box row is
    /// "Box {i}" plus " (found text)" for a search mark; its Remove is disabled without
    /// annotate permission (`redact_ui_can_annotate`).
    pub fn redact_ui_groups(&self) -> Vec<RedactPageGroup> {
        group_by_page(&self.redact_ui.boxes)
    }

    /// A page row's "Page N" (U:310).
    pub fn on_redact_panel_page(&mut self, page: i64) {
        self.go_to(page);
    }

    /// A box row's "Box N" (U:323).
    pub fn on_redact_panel_box(&mut self, page: i64, id: i64) {
        self.show_redact_box(page.max(0) as usize, id);
    }

    /// A box row's "Remove" (U:330).
    pub fn on_redact_panel_remove(&mut self, page: i64, id: i64) {
        self.redact_ui_delete_box(page, id);
    }

    // -------------------------------------------------------------- find helper

    /// U:341 (focusing and selecting the field is the UI's).
    pub fn redact_ui_open_find(&mut self) {
        self.dom.set_open(ids::FIND_DIALOG, true);
        let disabled =
            !self.redact_ui_can_annotate() || self.dom.value(ids::SEARCH_TEXT).is_empty();
        self.dom.set_disabled(ids::SEARCH_GO, disabled);
    }

    /// U:347.
    pub fn redact_ui_run_find(&mut self) -> bool {
        let text = self.dom.value(ids::SEARCH_TEXT);
        if text.is_empty() {
            return false;
        }
        let ok = self.add_redact_search(&text);
        if ok {
            self.dom.set_open(ids::FIND_DIALOG, false);
        }
        ok
    }

    /// U:503 `findText(text)`.
    pub fn redact_ui_find_text(&mut self, text: &str) {
        self.dom.set_value(ids::SEARCH_TEXT, text);
    }

    /// U:505 `closeFind()`.
    pub fn redact_ui_close_find(&mut self) {
        self.dom.set_open(ids::FIND_DIALOG, false);
    }

    /// "Find text to redact…" (U:354).
    pub fn on_redact_find_open(&mut self) {
        self.redact_ui_open_find();
    }

    /// Typing in the find field (U:355).
    pub fn on_redact_search_input(&mut self, text: &str) {
        self.dom.set_value(ids::SEARCH_TEXT, text);
        let disabled = !self.redact_ui_can_annotate() || text.is_empty();
        self.dom.set_disabled(ids::SEARCH_GO, disabled);
    }

    /// A key in the find field (U:356): Enter with text runs the search. Returns whether the
    /// key was taken (`preventDefault`).
    pub fn on_redact_search_key(&mut self, key: &str) -> bool {
        if key != "Enter" || self.dom.value(ids::SEARCH_TEXT).is_empty() {
            return false;
        }
        self.redact_ui_run_find();
        true
    }

    /// "Add boxes" (U:361).
    pub fn on_redact_search_go(&mut self) {
        self.redact_ui_run_find();
    }

    /// The find dialog's "Cancel" (U:362).
    pub fn on_redact_find_cancel(&mut self) {
        self.dom.set_open(ids::FIND_DIALOG, false);
    }

    // -------------------------------------------------------------- confirmation

    /// U:366: the nine switches as the dialog shows them.
    pub fn redact_ui_sanitize(&self) -> RedactSanitize {
        let mut out = sanitize_all(false);
        for (key, id) in ids::SANITIZE_BOXES {
            sanitize_set(&mut out, key, self.dom.checked(id));
        }
        out
    }

    /// U:369.
    pub fn redact_ui_sync_hidden(&mut self) {
        let values: Vec<bool> = ids::SANITIZE_BOXES
            .iter()
            .map(|(_, id)| self.dom.checked(id))
            .collect();
        let all = values.iter().all(|v| *v);
        let some = values.iter().any(|v| *v);
        self.dom.set_checked(ids::HIDDEN_INPUT, all);
        self.dom.set_attr(
            ids::HIDDEN_INPUT,
            "indeterminate",
            if some && !all { "true" } else { "false" },
        );
        let preview = self.redact_view();
        let kept: Vec<String> = ids::SANITIZE_BOXES
            .iter()
            .filter(|(key, id)| {
                !self.dom.checked(id)
                    && preview
                        .as_ref()
                        .is_none_or(|p| inventory_has(&p.sanitize, key))
            })
            .map(|(key, _)| words::sanitize_label(key).unwrap_or("").to_lowercase())
            .collect();
        let text = if kept.is_empty() {
            String::new()
        } else {
            format!("Kept in the copy: {}.", kept.join("; "))
        };
        self.dom.set_text(ids::KEPT_EL, text);
    }

    /// U:377.
    pub fn redact_ui_set_sanitize(&mut self, values: &RedactSanitize) {
        for (key, id) in ids::SANITIZE_BOXES {
            self.dom.set_checked(id, sanitize_get(values, key));
        }
        self.redact_ui_sync_hidden();
    }

    /// "Also remove hidden information…" changed (U:381).
    pub fn on_redact_hidden_change(&mut self, checked: bool) {
        self.redact_ui_set_sanitize(&sanitize_all(checked));
    }

    /// One of the nine switches changed (U:382).
    pub fn on_redact_sanitize_change(&mut self, key: &str, checked: bool) {
        if let Some((_, id)) = ids::SANITIZE_BOXES.iter().find(|(k, _)| *k == key) {
            self.dom.set_checked(id, checked);
        }
        self.redact_ui_sync_hidden();
    }

    /// U:385: fills and shows the dialog (its primary button runs `confirm_redact`). The
    /// "Boxes per page" rows are `lane3.redact_confirm_rows`; focus is the UI's.
    pub fn redact_ui_show_confirm(&mut self, preview: &RedactPreview) {
        self.dom.set_text(
            ids::CONFIRM_HEADING_EL,
            words::confirm_heading(preview.marks),
        );
        self.lane3.redact_confirm_rows = preview.page_counts.clone();
        // Shared content under a box is redacted everywhere it is used; the copy's banner says so (overRedacted).
        self.dom.set_text(ids::SHARED_EL, "");
        self.dom.set_open(ids::DETAILS_EL, false);
        self.redact_ui_set_sanitize(&sanitize_all(true));
        let disabled = preview.marks == 0 || !self.redact_ui_can_edit();
        self.dom.set_disabled(ids::CONFIRM_BUTTON, disabled);
        self.dom.set_open(ids::CONFIRM_DIALOG, true);
    }

    /// U:508 `closeConfirm()`.
    pub fn redact_ui_close_confirm(&mut self) {
        self.dom.set_open(ids::CONFIRM_DIALOG, false);
    }

    /// The confirm dialog's "Cancel" (U:403).
    pub fn on_redact_confirm_cancel(&mut self) {
        self.redact_ui_close_confirm();
    }

    // -------------------------------------------------------------- questions

    /// One modal question: its words go into the dialog's elements (as the in-page dialog had
    /// them), the platform asks it, and the dialog is closed again.
    fn redact_ask_box(
        &mut self,
        dialog: &str,
        message: String,
        detail: String,
        buttons: Vec<String>,
        default: usize,
        cancel: usize,
    ) -> usize {
        self.dom.set_open(dialog, true);
        // ponytail: platform::Question has no redaction-question variant yet; PendingBoxes tags them all.
        let question = MessageBox {
            question: Asked::PendingBoxes,
            kind: BoxKind::Question,
            message,
            detail: Some(detail),
            buttons,
            default,
            cancel,
        };
        let choice = self.host.platform.ask(&question);
        self.dom.set_open(dialog, false);
        choice
    }

    /// U:408: the pending-boxes question. `memory`: a signed file's boxes, which a saved copy
    /// does not carry. [Redact now…] is first and the default; Escape is Cancel.
    pub fn redact_ui_ask_pending(
        &mut self,
        count: i64,
        memory: bool,
        action: PendingAction,
    ) -> PendingChoice {
        let w = words::pending_words(action, memory);
        let heading = words::pending_text(count);
        self.dom.set_text(ids::PENDING_HEADING, heading.clone());
        self.dom.set_text(ids::PENDING_TEXT_EL, w.detail);
        self.dom.set_text(ids::PENDING_MARKS, w.go);
        let buttons = vec![
            self.dom.text(ids::PENDING_NOW),
            w.go.to_string(),
            self.dom.text(ids::PENDING_CANCEL),
        ];
        match self.redact_ask_box(
            ids::PENDING_DIALOG,
            heading,
            w.detail.to_string(),
            buttons,
            0,
            2,
        ) {
            0 => PendingChoice::Redact,
            1 => PendingChoice::Marks,
            _ => PendingChoice::Cancel,
        }
    }

    /// U:415: a yes/no question; Cancel is the default (focused) answer.
    pub fn redact_ui_ask_yes_no(&mut self, question: &words::Question) -> bool {
        self.redact_ui_ask(question, 1)
    }

    /// Round 2: "That would replace the original": the way forward ("Choose another name…")
    /// is the default; Cancel stays the Escape answer.
    pub fn redact_ui_ask_retry(&mut self, question: &words::Question) -> bool {
        self.redact_ui_ask(question, 0)
    }

    fn redact_ui_ask(&mut self, question: &words::Question, default: usize) -> bool {
        self.dom
            .set_text(ids::ASK_HEADING, question.heading.clone());
        self.dom.set_text(ids::ASK_TEXT, question.text.clone());
        self.dom.set_text(ids::ASK_YES, question.yes.clone());
        self.dom.set_hidden(ids::ASK_ALT, true);
        let buttons = vec![question.yes.clone(), self.dom.text(ids::ASK_CANCEL)];
        self.redact_ask_box(
            ids::ASK_DIALOG,
            question.heading.clone(),
            question.text.clone(),
            buttons,
            default,
            1,
        ) == 0
    }

    /// U:423: three answers: `yes` (0), the `alt` button (1), or Cancel (2); `default` is the
    /// answer Return gives.
    pub fn redact_ui_ask_three(
        &mut self,
        heading: &str,
        text: &str,
        yes: &str,
        alt: &str,
        default: usize,
    ) -> usize {
        self.dom.set_text(ids::ASK_HEADING, heading);
        self.dom.set_text(ids::ASK_TEXT, text);
        self.dom.set_text(ids::ASK_YES, yes);
        self.dom.set_text(ids::ASK_ALT, alt);
        self.dom.set_hidden(ids::ASK_ALT, false);
        let buttons = vec![
            yes.to_string(),
            alt.to_string(),
            self.dom.text(ids::ASK_CANCEL),
        ];
        let choice = self.redact_ask_box(
            ids::ASK_DIALOG,
            heading.to_string(),
            text.to_string(),
            buttons,
            default.min(2),
            2,
        );
        self.dom.set_hidden(ids::ASK_ALT, true);
        choice.min(2)
    }

    // -------------------------------------------------------------- banner (§2.3 step 6)

    /// U:435.
    pub fn redact_ui_show_banner(
        &mut self,
        name: &str,
        areas: i64,
        original: bool,
        extra: &words::BannerExtra,
    ) {
        self.redact_ui.banner_state = Some(RedactBannerState {
            name: name.to_string(),
            areas,
            original,
            over_redacted: extra.over_redacted,
        });
        self.dom
            .set_text(ids::BANNER_TEXT_EL, words::banner_text(name, areas, extra));
        // Round 2: content went outside the boxes: the banner is a warning, not information.
        self.set_status_kind(
            ids::BANNER_TEXT_EL,
            if extra.over_redacted {
                "warning"
            } else {
                "info"
            },
        );
        self.dom.set_hidden(ids::BANNER_ORIGINAL, !original);
        self.dom.set_hidden(ids::BANNER, false);
    }

    /// U:441.
    pub fn redact_ui_hide_banner(&mut self) {
        self.redact_ui.banner_state = None;
        self.dom.set_hidden(ids::BANNER, true);
    }

    /// The banner's "Open original" (U:445).
    pub fn on_redact_banner_original(&mut self) {
        self.open_redact_original();
    }

    /// The banner's "×" (U:446).
    pub fn on_redact_banner_dismiss(&mut self) {
        self.redact_ui_hide_banner();
    }

    // -------------------------------------------------------------- overlay

    /// U:451: the page's boxes (black, red outline) and, under them, the tint of what goes
    /// (shown while [`Session::redact_ui_peeking`]), in CSS px at `zoom`.
    pub fn redact_ui_paint_boxes(
        &self,
        page: i64,
        zoom: f64,
        rects: &[Vec<f64>],
    ) -> Vec<OverlayShape> {
        let mut out: Vec<OverlayShape> = rects
            .iter()
            .filter(|b| b.len() >= 4)
            .map(|b| OverlayShape {
                class: "redact-mark".into(),
                points: vec![
                    b[0] * zoom,
                    b[1] * zoom,
                    (b[2] - b[0]) * zoom,
                    (b[3] - b[1]) * zoom,
                ],
            })
            .collect();
        let Some(tint) = &self.redact_ui.tint else {
            return out;
        };
        if tint.page != page || tint.key != inspect_key(page, rects) {
            return out;
        }
        for shape in &tint.shapes {
            let p = &shape.points;
            let corners: Vec<f64> = if p.len() == 8 {
                p.clone()
            } else if p.len() >= 4 {
                vec![p[0], p[1], p[2], p[1], p[2], p[3], p[0], p[3]]
            } else {
                continue;
            };
            out.push(OverlayShape {
                class: format!("redact-tint redact-tint-{}", shape.kind.as_str()),
                points: corners.iter().map(|v| v * zoom).collect(),
            });
        }
        out
    }

    /// U:475: the underneath view: the toggle is on or the pointer is down.
    pub fn redact_ui_peeking(&self) -> bool {
        self.redact_ui.underneath || self.redact_ui.peek
    }

    /// U:484 `tintKey`.
    pub fn redact_ui_tint_key(&self) -> String {
        self.redact_ui
            .tint
            .as_ref()
            .map(|t| t.key.clone())
            .unwrap_or_default()
    }

    /// U:485 `setTint(page, key, regions)`.
    pub fn redact_ui_set_tint(
        &mut self,
        page: i64,
        key: &str,
        regions: Option<&[RedactInspectRegion]>,
    ) {
        self.redact_ui.tint = regions.map(|r| RedactTint {
            page,
            key: key.to_string(),
            shapes: tint_shapes(r),
        });
        // R4: say before the question that this page will become an image.
        let flat = regions.is_some_and(|r| r.iter().any(|region| region.needs_flatten));
        self.dom.set_hidden(ids::FLATTEN_NOTE_EL, !flat);
        self.dom.set_text(
            ids::FLATTEN_NOTE_EL,
            if flat {
                words::flatten_note(page)
            } else {
                String::new()
            },
        );
    }

    // -------------------------------------------------------------- getters (STATE `redact`)

    /// `redactUi.active`.
    pub fn redact_ui_active(&self) -> bool {
        self.redact_ui.active
    }
    /// `redactUi.flattenNote`.
    pub fn redact_ui_flatten_note(&self) -> String {
        if self.dom.hidden(ids::FLATTEN_NOTE_EL) {
            String::new()
        } else {
            self.dom.text(ids::FLATTEN_NOTE_EL)
        }
    }
    /// `redactUi.tintCount`.
    pub fn redact_ui_tint_count(&self) -> usize {
        self.redact_ui.tint.as_ref().map_or(0, |t| t.shapes.len())
    }
    /// `redactUi.underneath` (the checkbox).
    pub fn redact_ui_underneath(&self) -> bool {
        self.dom.checked(ids::UNDERNEATH_INPUT)
    }
    /// `redactUi.boxes`.
    pub fn redact_ui_boxes(&self) -> &[RedactBox] {
        &self.redact_ui.boxes
    }
    /// `redactUi.applyOpen.textContent`.
    pub fn redact_ui_apply_open_text(&self) -> String {
        self.dom.text(ids::APPLY_OPEN)
    }
    /// `redactUi.findOpen`.
    pub fn redact_ui_find_open(&self) -> bool {
        self.dom.is_open(ids::FIND_DIALOG)
    }
    /// `redactUi.confirmOpen`.
    pub fn redact_ui_confirm_open(&self) -> bool {
        self.dom.is_open(ids::CONFIRM_DIALOG)
    }
    /// `redactUi.pendingOpen`.
    pub fn redact_ui_pending_open(&self) -> bool {
        self.dom.is_open(ids::PENDING_DIALOG)
    }
    /// `redactUi.banner` as STATE writes it: `{name, areas, original, overRedacted}` or null.
    pub fn redact_ui_banner(&self) -> Value {
        match &self.redact_ui.banner_state {
            None => Value::Null,
            Some(b) => {
                let mut map = Map::new();
                map.insert("name".into(), json!(b.name));
                map.insert("areas".into(), json!(b.areas));
                map.insert("original".into(), json!(b.original));
                map.insert("overRedacted".into(), json!(b.over_redacted));
                Value::Object(map)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rbox(page: i64, id: i64, x: f64, y: f64) -> RedactBox {
        RedactBox {
            page,
            id,
            rect: vec![x, y, x + 10.0, y + 10.0],
            search: false,
        }
    }

    #[test]
    fn groups_pages_then_top_to_bottom() {
        let boxes = [
            rbox(2, 1, 50.0, 10.0),
            rbox(0, 2, 30.0, 40.0),
            rbox(0, 3, 10.0, 40.0),
            rbox(0, 4, 90.0, 5.0),
        ];
        let groups = group_by_page(&boxes);
        assert_eq!(
            groups.iter().map(|g| g.page).collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(
            groups[0].boxes.iter().map(|b| b.id).collect::<Vec<_>>(),
            vec![4, 3, 2]
        );
    }

    #[test]
    fn sanitize_switches() {
        let all = sanitize_from(None, None);
        assert!(all.info && all.r#struct);
        let none = sanitize_from(
            Some(&json!(false)),
            Some(&json!({"xmp": true, "info": "yes"})),
        );
        assert!(!none.info && none.xmp && !none.links);
        let one_off = sanitize_from(Some(&json!(null)), Some(&json!({"links": false})));
        assert!(one_off.info && !one_off.links);
    }

    #[test]
    fn tint_and_keys() {
        let region = RedactInspectRegion {
            rect: vec![0.0, 0.0, 10.0, 10.0],
            glyphs: vec![vec![0.0; 8], vec![0.0; 6]],
            needs_flatten: true,
            ..RedactInspectRegion::default()
        };
        let shapes = tint_shapes(std::slice::from_ref(&region));
        assert_eq!(shapes.len(), 2);
        assert_eq!(shapes[0].kind, TintKind::Shape);
        let coarse = RedactInspectRegion {
            needs_flatten: false,
            truncated: true,
            ..region
        };
        assert_eq!(tint_shapes(&[coarse]).len(), 1);
        assert_eq!(
            inspect_key(0, &[vec![1.0, 2.5, 3.0, 4.0]]),
            "0:[[1,2.5,3,4]]"
        );
        assert_eq!(
            whole_page_rect(&PageSize {
                width: 612.0,
                height: 792.004
            }),
            vec![0.0, 0.0, 612.0, 792.0]
        );
    }

    #[test]
    fn boxes_on_open() {
        assert!(!asks_boxes_on_open(false, None));
        assert!(asks_boxes_on_open(false, Some((1, false))));
        assert!(!asks_boxes_on_open(true, Some((2, false))));
        assert!(asks_boxes_on_open(true, Some((2, true))));
    }
}
