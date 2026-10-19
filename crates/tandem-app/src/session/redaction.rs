//! Redaction (renderer.ts R:6429-6874; rust-and-ux-plan.md §2.3): the requests, the journal,
//! the copy that opens and the drive. The tool's panel, dialogs and words are redact_ui.rs.
//!
//! Kept exactly as reviewed: the copy is checked by the engine before anything is written
//! (fail closed), main never lets the copy replace its original file, the copy opens in this
//! window with its banner, and every way out (save, export, print, sign, close, open) asks
//! about boxes that were never redacted.
#![allow(dead_code)]

use super::Session;
use super::consts::{OPS, REDACT_LOG_OPS};
use super::content::{json_text, round2};
use super::dom_ids::{OBJECT_MENU, redact_ui as ids};
use super::modes::journal_of;
use super::redact_ui::{sanitize_from, whole_page_rect};
use super::types::{
    MessageKind, OpKind, OthersAnswer, PendingAction, PendingChoice, PromptChoice, RedactAskAnswer,
    RedactLast, RedactPreview, RedactPrompt, RedactRegion, RedactVerifiedCounts, UiMode,
};
use crate::host::SaveKind;
use crate::messages::redact as words;
use crate::safe_write::SaveFail;
use serde_json::{Map, Value, json};
use tandem_protocol::engine::{RedactApplyReply, RedactInspectRegion, RedactSanitize};

/// `applyRedact`'s `extra` (R:6639): the flatten and unsign answers, boxes for this apply
/// only (a drive's `regions`, passed to the engine as given), and whether a `signed` refusal
/// was about Adobe Reader usage rights.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RedactExtra {
    pub flatten: Option<Vec<i64>>,
    pub unsign: bool,
    pub regions: Option<Vec<Value>>,
    pub usage_rights: bool,
}

/// `RedactLast` with only `op`, `ok` and maybe `reason`.
fn last(op: &str, ok: bool, reason: Option<&str>) -> RedactLast {
    RedactLast {
        op: op.into(),
        ok,
        reason: reason.map(str::to_string),
        ..RedactLast::default()
    }
}

/// The integers of a refusal's `pages` (`Number.isInteger`), or None when it has no array.
fn integer_pages(value: Option<&Value>) -> Option<Vec<i64>> {
    let list = value?.as_array()?;
    Some(
        list.iter()
            .filter_map(Value::as_f64)
            .filter(|v| v.fract() == 0.0 && v.is_finite())
            .map(|v| v as i64)
            .collect(),
    )
}

/// `` `${value}` `` of a logged message field.
fn template(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => json_text(other),
    }
}

impl Session {
    /// D5 (R:6440): a signed or certified file takes no marks; its boxes live in
    /// `redact_memory` and go as `regions`.
    pub fn redact_memory_mode(&self) -> bool {
        self.doc
            .as_ref()
            .is_some_and(|d| !d.info.needs_password && (d.info.signatures > 0 || d.info.certified))
            && self.permitted("edit")
    }

    /// R:6443.
    pub fn can_redact_draw(&self) -> bool {
        self.can_annotate() || self.redact_memory_mode()
    }

    /// R:6448: the preview the panel and dialog show: the document's marks plus the boxes
    /// held in memory.
    pub fn redact_view(&self) -> Option<RedactPreview> {
        if self.redact_memory.is_empty() {
            return self.redact_state.clone();
        }
        let base = self.redact_state.clone().unwrap_or_default();
        let mut counts: std::collections::BTreeMap<i64, i64> = base
            .page_counts
            .iter()
            .map(|item| (item.page, item.marks))
            .collect();
        for b in &self.redact_memory {
            *counts.entry(b.page).or_insert(0) += 1;
        }
        let pages: Vec<i64> = counts.keys().copied().collect();
        Some(RedactPreview {
            marks: base.marks + self.redact_memory.len() as i64,
            page_counts: pages
                .iter()
                .map(|page| tandem_protocol::engine::RedactPageCount {
                    page: *page,
                    marks: counts[page],
                })
                .collect(),
            pages,
            ..base
        })
    }

    /// R:6458: the shown page's boxes, document marks first, as the overlay draws them.
    pub fn redact_page_rects(&self, page: i64) -> Vec<Vec<f64>> {
        self.notes
            .iter()
            .filter(|item| item.r#type == "Redact" && item.rect.len() == 4)
            .map(|item| item.rect.clone())
            .chain(
                self.redact_memory
                    .iter()
                    .filter(|b| b.page == page)
                    .map(|b| b.rect.clone()),
            )
            .collect()
    }

    /// R:6469 (review M1/L5): main's close and open prompts name only boxes that would be
    /// lost: a signed file's boxes in memory always, the document's marks only while unsaved.
    pub fn report_redact_pending(&mut self) {
        let marks = if self.dirty {
            self.redact_state.as_ref().map_or(0, |s| s.marks)
        } else {
            0
        };
        let count = self.redact_memory.len() as i64 + marks;
        let memory = !self.redact_memory.is_empty() && marks == 0;
        let key = format!("{count}:{memory}");
        if key == self.redact_pending_sent {
            return;
        }
        self.redact_pending_sent = key;
        self.host.set_redact_pending(count.max(0) as u32, memory);
    }

    /// R:6478 (review L6): "Box N" in the panel goes to the page and selects the box there
    /// (keys then move or resize it).
    pub fn show_redact_box(&mut self, page: usize, id: i64) {
        self.go_to(page as i64);
        if id < 0 {
            return;
        }
        self.drain();
        if self.shown_page == page as i64 && self.notes.iter().any(|item| item.id == id) {
            self.on_redact_target_click(id);
        }
    }

    /// R:6491 (review M2): unsaved changes other than redaction boxes since the last save (or
    /// open). The recovery log names every mutation; without it any unsaved change counts.
    pub fn other_unsaved_changes(&self) -> bool {
        if !self.dirty {
            return false;
        }
        if self.recovery.base.is_none() {
            return true;
        }
        let entries = &self.recovery.entries;
        let start = entries
            .iter()
            .rposition(|entry| entry.op == "saved")
            .map_or(0, |i| i + 1);
        entries[start..].iter().any(|entry| {
            let mutates = OPS
                .iter()
                .any(|(op, kind, _, _)| *op == entry.op && *kind == OpKind::Mutate);
            if REDACT_LOG_OPS.contains(&entry.op.as_str()) || !mutates {
                return false;
            }
            if entry.op == "annot-delete" {
                let key = format!(
                    "{}:{}",
                    template(entry.message.get("page")),
                    template(entry.message.get("annotId"))
                );
                return !self.redact_deletes.contains(&key);
            }
            true
        })
    }

    /// R:6506.
    pub fn after_redact_memory(&mut self, announcement: &str) {
        self.announce(announcement);
        self.render_redact_state();
        self.repaint();
    }

    /// R:6516 (§2.3 step 2): ask the engine what the shown page's boxes take
    /// (`redact-inspect`), once per change of boxes while the tool is on; the overlay tints it
    /// while the pointer is down or with "Show what's underneath". Called while painting.
    pub fn schedule_redact_inspect(&mut self, page: usize, rects: Vec<Vec<f64>>) {
        let page = page as i64;
        let unlocked = self.doc.as_ref().is_some_and(|d| !d.info.needs_password);
        if !self.redact_ui.active || !unlocked || rects.is_empty() || page != self.wanted_page {
            if !self.redact_ui_tint_key().is_empty() {
                self.redact_ui_set_tint(page, "", None);
            }
            self.redact_inspect_key.clear();
            return;
        }
        let key = super::redact_ui::inspect_key(page, &rects);
        if key == self.redact_inspect_key {
            return;
        }
        self.redact_inspect_key = key.clone();
        // M4: a document mark is named by its annotId (the engine reads its own rect, quads and
        // rule, so a find-helper box gets the tight rule); a memory box goes as its rect.
        let regions: Vec<Value> = self
            .notes
            .iter()
            .filter(|item| item.r#type == "Redact" && item.rect.len() == 4)
            .map(|item| json!({ "page": page, "annotId": item.id }))
            .chain(
                self.redact_memory
                    .iter()
                    .filter(|b| b.page == page)
                    .map(|b| json!({ "page": page, "rect": b.rect })),
            )
            .collect();
        match self.request(
            json!({ "op": "redact-inspect", "regions": regions }),
            Vec::new(),
        ) {
            Ok(reply) => {
                if self.redact_inspect_key != key {
                    return;
                }
                let regions: Vec<RedactInspectRegion> = reply
                    .get("regions")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                self.redact_ui_set_tint(page, &key, Some(&regions));
                self.repaint();
            }
            Err(_) => {
                if self.redact_inspect_key == key {
                    self.redact_ui_set_tint(page, &key, Some(&[]));
                }
            }
        }
    }

    /// R:6541 (X6 with §2.3 step 1): whether Escape's last layer is the redaction tool. The
    /// object menu, a drag or resize, and a box being drawn take Escape before it.
    pub fn redact_escape_layer(&self) -> bool {
        self.ui_mode == UiMode::Redact
            && self.redact_ui.active
            && self.dom.hidden(OBJECT_MENU)
            && self.direct_gesture.is_none()
            && !self.area_draft_open()
    }

    /// R:6545: Escape (or a drive's) leaves the tool for the mode it was chosen from.
    pub fn exit_redact_tool(&mut self) {
        if self.ui_mode != UiMode::Redact {
            return;
        }
        let back = if self.redact_return_mode == UiMode::Redact {
            UiMode::Read
        } else {
            self.redact_return_mode
        };
        self.set_mode(back);
        self.announce(words::STOPPED);
    }

    /// R:6551.
    pub fn render_redact_state(&mut self) {
        self.redact_ui_update();
    }

    /// R:6555. Never fails: a refusal is shown (outside memory mode) and leaves no preview.
    pub fn refresh_redact_preview(&mut self) -> Option<RedactPreview> {
        if self.doc.as_ref().is_none_or(|d| d.info.needs_password) {
            self.redact_state = None;
            self.render_redact_state();
            return None;
        }
        match self.request(json!({ "op": "redact-preview" }), Vec::new()) {
            Ok(reply) => self.redact_state = serde_json::from_value(reply.value).ok(),
            Err(err) => {
                if self.ui_mode == UiMode::Redact && !self.redact_memory_mode() {
                    self.show_error(&err.message);
                }
                self.redact_state = None;
            }
        }
        self.render_redact_state();
        self.redact_state.clone()
    }

    /// R:6571: one drawn box (`announcement` is `words::ADDED` unless a caller says otherwise).
    pub fn add_redact_area(&mut self, page: usize, rect: Vec<f64>, announcement: &str) -> bool {
        if self.doc.is_none() {
            return false;
        }
        if self.redact_memory_mode() {
            self.redact_memory.push(RedactRegion {
                page: page as i64,
                rect: rect.iter().map(|v| round2(*v)).collect(),
            });
            self.redact_last = Some(last("redact", true, None));
            self.after_redact_memory(announcement);
            return true;
        }
        if !self.can_annotate() {
            return false;
        }
        match self.request(
            json!({ "op": "redact-add", "page": page, "rect": rect }),
            Vec::new(),
        ) {
            Ok(reply) => {
                self.redact_last = Some(last("redact", true, None));
                self.apply_edit(&journal_of(&reply.value), announcement);
                true
            }
            Err(err) => {
                let reason = err.reason.clone().unwrap_or_else(|| "invalid".into());
                self.redact_last = Some(last("redact", false, Some(&reason)));
                let text =
                    words::refusal_text(Some(&reason), &[], None, false).unwrap_or(err.message);
                self.show_error(&text);
                false
            }
        }
    }

    /// R:6594 (§2.3 step 3): "Redact whole page" (the page menu in Redact, or the drive's
    /// `redact-page`); the TS default page is `wantedPage`.
    pub fn add_redact_page(&mut self, page: usize) -> bool {
        let Some(size) = self.doc.as_ref().and_then(|d| d.pages.get(page)).cloned() else {
            return false;
        };
        self.add_redact_area(
            page,
            whole_page_rect(&size),
            &words::added_page(page as i64 + 1),
        )
    }

    /// R:6600: "Find text to redact…" (never in memory mode: it needs annotate permission).
    pub fn add_redact_search(&mut self, text: &str) -> bool {
        if self.doc.is_none() || !self.can_annotate() {
            return false;
        }
        match self.request(json!({ "op": "redact-search", "text": text }), Vec::new()) {
            Ok(reply) => {
                self.redact_last = Some(last("redact", true, None));
                let added = reply
                    .get("added")
                    .and_then(Value::as_f64)
                    .map_or(0, |a| a as i64);
                self.apply_edit(&journal_of(&reply.value), &words::added_search(added, text));
                true
            }
            Err(err) => {
                let reason = err.reason.clone().unwrap_or_else(|| "invalid".into());
                self.redact_last = Some(last("redact", false, Some(&reason)));
                let text =
                    words::refusal_text(Some(&reason), &[], None, false).unwrap_or(err.message);
                self.show_error(&text);
                false
            }
        }
    }

    /// R:6616: the panel's "Redact N areas…", the menu, and "Redact now…".
    pub fn open_redact_dialog(&mut self) {
        if self.ui_mode != UiMode::Redact {
            self.set_mode(UiMode::Redact);
        }
        self.refresh_redact_preview();
        let Some(preview) = self.redact_view() else {
            return;
        };
        if preview.marks == 0 {
            self.announce(words::DRAW_FIRST);
            self.set_status(words::DRAW_FIRST);
            return;
        }
        self.redact_ui_show_confirm(&preview);
    }

    /// R:6639: one apply. `open` is the §2.3 flow: the copy is saved, opens in this window and
    /// the original closes (the dialog and `redact-confirm`); without it the legacy drive op
    /// keeps the original open. A refusal can ask to flatten pages or to drop the signatures.
    pub fn apply_redact(
        &mut self,
        sanitize: &RedactSanitize,
        open: bool,
        extra: RedactExtra,
    ) -> bool {
        let Some(source) = self.doc.as_ref().map(|d| d.name.clone()) else {
            return false;
        };
        if !(self.can_edit() || self.redact_memory_mode()) {
            return false;
        }
        let mut regions: Vec<Value> = self
            .redact_memory
            .iter()
            .map(|b| json!({ "page": b.page, "rect": b.rect }))
            .collect();
        regions.extend(extra.regions.clone().unwrap_or_default());
        let mut request = Map::new();
        request.insert("op".into(), json!("redact-apply"));
        request.insert(
            "sanitize".into(),
            serde_json::to_value(sanitize).unwrap_or(Value::Null),
        );
        if !regions.is_empty() {
            request.insert("regions".into(), Value::Array(regions));
        }
        if let Some(flatten) = extra.flatten.as_ref().filter(|f| !f.is_empty()) {
            request.insert("flatten".into(), json!(flatten));
        }
        if extra.unsign {
            request.insert("unsign".into(), json!(true));
        }
        let mut reply = match self.request(Value::Object(request), Vec::new()) {
            Ok(reply) => reply,
            Err(err) => {
                let refused = &err.reply;
                let code = err.reason.clone().or_else(|| {
                    refused
                        .get("reason")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                });
                let refused_pages = integer_pages(refused.get("pages"));
                if open && code.as_deref() == Some("needs-flatten") && refused_pages.is_some() {
                    let pages = refused_pages.unwrap_or_default();
                    let agreed = extra.flatten.clone().unwrap_or_default();
                    // Review L1: a page already agreed to that still refuses ends the retries.
                    let fresh: Vec<i64> = pages
                        .iter()
                        .copied()
                        .filter(|p| !agreed.contains(p))
                        .collect();
                    if !fresh.is_empty() && self.redact_ask("flatten", &fresh) {
                        let mut union = agreed;
                        for page in pages {
                            if !union.contains(&page) {
                                union.push(page);
                            }
                        }
                        return self.apply_redact(
                            sanitize,
                            open,
                            RedactExtra {
                                flatten: Some(union),
                                ..extra
                            },
                        );
                    }
                    self.redact_last = Some(last("redact-apply", false, Some("cancelled")));
                    return false;
                }
                if open && code.as_deref() == Some("signed") && !extra.unsign {
                    // A reader-extended file (usage rights) is refused as signed too; it is
                    // worded apart when it can be told apart.
                    let usage_rights = self.redact_usage_rights(refused);
                    let kind = if usage_rights {
                        "usage-rights"
                    } else {
                        "signed"
                    };
                    if self.redact_ask(kind, &[]) {
                        return self.apply_redact(
                            sanitize,
                            open,
                            RedactExtra {
                                unsign: true,
                                usage_rights,
                                ..extra
                            },
                        );
                    }
                    self.redact_last = Some(RedactLast {
                        question: Some(kind.into()),
                        ..last("redact-apply", false, Some("cancelled"))
                    });
                    return false;
                }
                let reason = err.reason.clone();
                self.redact_last = Some(last(
                    "redact-apply",
                    false,
                    Some(reason.as_deref().unwrap_or("invalid")),
                ));
                if open {
                    self.redact_ui_close_confirm(); // the error bar must not sit behind the modal dialog
                }
                let pages = refused_pages.unwrap_or_default();
                let detail = refused.get("detail").and_then(Value::as_str);
                let hidden_kept = super::redact_ui::SANITIZE_KEYS
                    .iter()
                    .any(|key| !super::redact_ui::sanitize_get(sanitize, key));
                let text = words::refusal_text(reason.as_deref(), &pages, detail, hidden_kept)
                    .unwrap_or(err.message.clone());
                self.show_error(&text);
                return false;
            }
        };
        let bytes = reply.take_bytes("bytes").unwrap_or_default();
        let applied: RedactApplyReply = serde_json::from_value(reply.value).unwrap_or_default();
        let verified = RedactVerifiedCounts {
            text: applied.verified.text,
            pixels: applied.verified.pixels,
            raw: applied.verified.raw,
        };
        let over = applied.over_redacted;
        let suggested = format!("{} redacted.pdf", self.file_stem(&source));
        if !open {
            let Some(name) = self.save_new_file(&bytes, &suggested, "pdf", false) else {
                return false;
            };
            self.redact_last = Some(RedactLast {
                verified: Some(verified),
                flattened: Some(applied.flattened.clone()),
                unsigned: Some(applied.unsigned),
                over_redacted: Some(over),
                ..last("redact-apply", true, None)
            });
            let text = words::saved_kept(&name, over);
            self.set_status(&text);
            // X8: amber when more than the boxes was removed (worth checking), green otherwise; never red.
            self.show_message(
                &text,
                if over {
                    MessageKind::Warning
                } else {
                    MessageKind::Success
                },
            );
            self.redact_ui_close_confirm();
            return true;
        }
        let mut saved = self
            .host
            .save_new(&bytes, Some(&suggested), SaveKind::Pdf, false, true);
        // Review H1: main refuses to write the copy over the original; the save panel is offered again.
        while saved == Err(SaveFail::SameFile)
            && !self.headless
            && self.redact_ui_ask_yes_no(&words::SAME_FILE_QUESTION.into())
        {
            saved = self
                .host
                .save_new(&bytes, Some(&suggested), SaveKind::Pdf, false, true);
        }
        let saved = match saved {
            Ok(saved) => saved,
            Err(SaveFail::SameFile) => {
                self.redact_last = Some(last("redact-apply", false, Some("same-file")));
                self.redact_ui_close_confirm();
                self.show_error(&words::same_file());
                return false;
            }
            Err(SaveFail::Cancelled) => {
                self.redact_last = Some(last("redact-apply", false, Some("cancelled")));
                return false;
            }
            Err(fail) => {
                self.redact_last = Some(last("redact-apply", false, Some(fail.as_str())));
                self.show_error(&words::not_saved(fail.as_str()));
                return false;
            }
        };
        self.redact_ui_close_confirm();
        let areas = applied.marks;
        // §2.3 step 6: the copy replaces the original in this window; the tool's job is done.
        if self.ui_mode == UiMode::Redact {
            let back = if self.redact_return_mode == UiMode::Redact {
                UiMode::Read
            } else {
                self.redact_return_mode
            };
            self.set_mode(back);
        }
        if let Some(url) = saved.url.clone() {
            self.open_bound(&saved.name, &url);
        }
        let opened = self.doc.as_ref().is_some_and(|d| d.name == saved.name);
        if opened {
            let banner = words::BannerExtra {
                flattened: Some(applied.flattened.clone()),
                unsigned: applied.unsigned,
                usage_rights: extra.usage_rights,
                over_redacted: over,
            };
            self.redact_ui_show_banner(&saved.name, areas, saved.original == Some(true), &banner);
            self.set_status(&words::saved_opened(&saved.name));
        } else {
            let text = words::saved(&saved.name, over);
            self.set_status(&text);
            // X8: no banner says it here, so the notice does, in green.
            self.show_message(
                &text,
                if over {
                    MessageKind::Warning
                } else {
                    MessageKind::Success
                },
            );
        }
        self.redact_last = Some(RedactLast {
            verified: Some(verified),
            opened: Some(if opened {
                saved.name.clone()
            } else {
                String::new()
            }),
            areas: Some(areas),
            flattened: Some(applied.flattened),
            unsigned: Some(applied.unsigned),
            over_redacted: Some(over),
            ..last("redact-apply", true, None)
        });
        true
    }

    /// R:6717: the two follow-up questions (§2.4 R4, D5); a drive answers them with
    /// `redact-confirm {flatten, unsign}`.
    pub fn redact_ask(&mut self, kind: &str, pages: &[i64]) -> bool {
        if self.headless {
            let answer = self.redact_ask_answer.unwrap_or_default();
            return if kind == "flatten" {
                answer.flatten == Some(true)
            } else {
                answer.unsign == Some(true)
            };
        }
        let question = match kind {
            "flatten" => words::flatten_question(pages),
            "usage-rights" => words::USAGE_RIGHTS_QUESTION.into(),
            _ => words::SIGNED_QUESTION.into(),
        };
        self.redact_ui_ask_yes_no(&question)
    }

    /// R:6728: whether a `signed` refusal is about Adobe Reader usage rights only: true only
    /// when the engine says so (a refusal `detail` of "usage-rights"; the open reply's info
    /// carries no `usageRights` in this engine), and the signed wording stays otherwise.
    pub fn redact_usage_rights(&self, refused: &Value) -> bool {
        refused.get("detail").and_then(Value::as_str) == Some("usage-rights")
    }

    /// R:6733: opens a URL main issued for the copy or the original, then lets main bind the
    /// window to it (review L3).
    pub fn open_bound(&mut self, name: &str, url: &str) {
        self.open_from_url(name, url, false);
        if self.doc.as_ref().and_then(|d| d.url.as_deref()) == Some(url) {
            self.host.bind_open(url);
        }
    }

    /// R:6743: the dialog's "Redact and save as…". Review M2: other unsaved changes go into the
    /// copy but not the original, so they can be saved to the original first. Review L2: one
    /// apply at a time.
    pub fn confirm_redact(&mut self) -> bool {
        if self.redact_applying {
            return false;
        }
        self.redact_applying = true;
        self.dom.set_disabled(ids::CONFIRM_BUTTON, true);
        let result = self.confirm_redact_inner();
        self.redact_applying = false;
        self.dom.set_disabled(ids::CONFIRM_BUTTON, false);
        result
    }

    fn confirm_redact_inner(&mut self) -> bool {
        if self.other_unsaved_changes() {
            let answer = if self.headless {
                self.redact_others_answer.unwrap_or(OthersAnswer::Continue)
            } else {
                let q = words::OTHERS_QUESTION;
                match self.redact_ui_ask_three(
                    q.heading,
                    q.text,
                    "Save them to the original first",
                    "Continue",
                ) {
                    0 => OthersAnswer::Save,
                    1 => OthersAnswer::Continue,
                    _ => OthersAnswer::Cancel,
                }
            };
            if answer == OthersAnswer::Cancel {
                self.redact_last = Some(last("redact-apply", false, Some("cancelled")));
                return false;
            }
            if answer == OthersAnswer::Save {
                // The boxes are saved as marks, without asking again.
                self.redact_marks_acknowledged = self.redact_state.as_ref().map_or(0, |s| s.marks);
                self.save_in_place("in-place");
                if self.dirty {
                    self.redact_last = Some(last("redact-apply", false, Some("not-saved")));
                    self.set_status(words::ORIGINAL_NOT_SAVED);
                    return false;
                }
            }
        }
        let sanitize = self.redact_ui_sanitize();
        self.apply_redact(&sanitize, true, RedactExtra::default())
    }

    /// R:6766 (§2.3 step 6): "Open original" in the banner.
    pub fn open_redact_original(&mut self) {
        match self.host.open_original() {
            Err(reason) => {
                if reason != "cancelled" {
                    self.show_error(words::ORIGINAL_UNREADABLE);
                }
            }
            Ok(issued) => {
                self.redact_ui_hide_banner();
                self.open_bound(&issued.name, &issued.url);
            }
        }
    }

    /// R:6784 (§2.3 step 7): every path that sends the document out (Save, Save As, Export,
    /// Print, signing) with boxes that were never redacted asks first: an export or print
    /// still shows what is under them. "Save marks only" is not asked again until the boxes
    /// change; exports and prints always ask. A drive answers with `redact-pending {choice}`;
    /// unanswered it goes on and STATE `redact.prompt` records `choice: "unasked"`.
    pub fn confirm_pending_save(&mut self, action: PendingAction) -> bool {
        // Review M1: a signed file's boxes (in memory) count too; a saved copy does not carry them.
        let marks = self.redact_view().map_or(0, |p| p.marks);
        if marks == 0 || (action == PendingAction::Save && marks == self.redact_marks_acknowledged)
        {
            return true;
        }
        let choice = if self.headless {
            match self.redact_prompt_answer.take() {
                None => {
                    self.redact_prompt = Some(RedactPrompt {
                        marks,
                        choice: PromptChoice::Unasked,
                        action,
                    });
                    return true;
                }
                Some(choice) => choice,
            }
        } else {
            let memory = !self.redact_memory.is_empty();
            self.redact_ui_ask_pending(marks, memory, action)
        };
        let prompt_choice = match choice {
            PendingChoice::Redact => PromptChoice::Redact,
            PendingChoice::Marks => PromptChoice::Marks,
            PendingChoice::Cancel => PromptChoice::Cancel,
        };
        self.redact_prompt = Some(RedactPrompt {
            marks,
            choice: prompt_choice,
            action,
        });
        match choice {
            PendingChoice::Marks => {
                if action == PendingAction::Save {
                    self.redact_marks_acknowledged = marks;
                }
                return true;
            }
            PendingChoice::Redact => self.open_redact_dialog(),
            PendingChoice::Cancel => self.set_status(&words::cancelled(action)),
        }
        false
    }

    /// R:6801: the drive's `redact {rect, page?}` or `redact {search}`.
    pub fn drive_redact(&mut self, contents: Option<&str>) {
        self.clear_error();
        let Some(wanted) = self.drive_wanted(contents) else {
            return;
        };
        if let Some(rect) = wanted.get("rect").and_then(Value::as_array) {
            let page = match wanted.get("page").and_then(Value::as_f64) {
                // A page the engine must refuse stays out of range rather than becoming another page.
                Some(p) if p >= 0.0 && p.fract() == 0.0 => p as usize,
                Some(_) => usize::MAX,
                None => self.wanted_page.max(0) as usize,
            };
            let rect: Vec<f64> = rect
                .iter()
                .map(|v| v.as_f64().unwrap_or(f64::NAN))
                .collect();
            self.add_redact_area(page, rect, words::ADDED);
        } else if let Some(text) = wanted.get("search").and_then(Value::as_str) {
            let text = text.to_string();
            self.add_redact_search(&text);
        } else {
            self.redact_last = Some(last("redact", false, Some("invalid")));
            self.show_error("redact: send rect and page, or search text");
        }
    }

    /// R:6815. `refresh_redact_preview` never fails, so the step always records `ok: true`.
    pub fn drive_redact_preview(&mut self) {
        self.clear_error();
        self.refresh_redact_preview();
        self.redact_last = Some(last("redact-preview", true, None));
    }

    /// R:6826: the legacy op: the copy is saved and the original stays open (scripts and the
    /// MCP agent rely on it).
    pub fn drive_redact_apply(&mut self, contents: Option<&str>) {
        let Some(wanted) = self.drive_wanted(contents) else {
            return;
        };
        let sanitize = sanitize_from(Some(&json!(true)), wanted.get("sanitize"));
        // R6: `regions [{page, rect}]` are boxes held in memory for this apply only (the engine checks them).
        let regions = wanted.get("regions").and_then(Value::as_array).cloned();
        // R4 / D5: `flatten [page]` and `unsign: true` are the answers an agent gives up front.
        let flatten = integer_pages(wanted.get("flatten"));
        let unsign = wanted.get("unsign") == Some(&Value::Bool(true));
        self.apply_redact(
            &sanitize,
            false,
            RedactExtra {
                flatten,
                unsign,
                regions,
                usage_rights: false,
            },
        );
    }

    /// R:6842: `redact-confirm {hidden?, details?, flatten?, unsign?, others?}`: the dialog's
    /// "Redact and save as…" (§2.3 steps 5-6). The dialog opens first if it is not open; the
    /// line's `to` answers the save picker.
    pub fn drive_redact_confirm(&mut self, contents: Option<&str>) {
        self.clear_error();
        let Some(wanted) = self.drive_wanted(contents) else {
            return;
        };
        if self.doc.is_none() {
            return;
        }
        if !self.redact_ui_confirm_open() {
            self.open_redact_dialog();
        }
        if !self.redact_ui_confirm_open() {
            self.redact_last = Some(last("redact-apply", false, Some("no-boxes")));
            return;
        }
        self.redact_ui_set_sanitize(&sanitize_from(wanted.get("hidden"), wanted.get("details")));
        self.redact_ask_answer = Some(RedactAskAnswer {
            flatten: Some(wanted.get("flatten") == Some(&Value::Bool(true))),
            unsign: Some(wanted.get("unsign") == Some(&Value::Bool(true))),
        });
        self.redact_others_answer = wanted
            .get("others")
            .and_then(Value::as_str)
            .and_then(OthersAnswer::parse);
        if self.dom.disabled(ids::CONFIRM_BUTTON) {
            self.redact_last = Some(last("redact-apply", false, Some("no-boxes")));
        } else {
            self.confirm_redact();
        }
        self.redact_ask_answer = None;
        self.redact_others_answer = None;
    }

    /// R:6858: `redact-search {text}`: the "Find text to redact…" helper, filled and run (the
    /// menu opens it empty).
    pub fn drive_redact_find(&mut self, contents: Option<&str>) {
        self.clear_error();
        let text = self
            .drive_wanted(contents)
            .and_then(|w| w.get("text").and_then(Value::as_str).map(str::to_string))
            .filter(|t| !t.is_empty());
        let Some(text) = text else {
            self.show_error("redact-search: give the text to find");
            return;
        };
        self.redact_ui_open_find();
        self.redact_ui_find_text(&text);
        self.redact_ui_run_find();
        self.redact_ui_close_find();
    }

    /// R:6869: `redact-pending {choice: "redact"|"marks"|"cancel"}`: answers the next
    /// pending-boxes save prompt.
    pub fn drive_redact_pending(&mut self, contents: Option<&str>) {
        let choice = self.drive_wanted(contents).and_then(|w| {
            w.get("choice")
                .and_then(Value::as_str)
                .and_then(PendingChoice::parse)
        });
        match choice {
            Some(choice) => self.redact_prompt_answer = Some(choice),
            None => self.show_error("redact-pending: choice must be redact, marks or cancel"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_pages_are_integers() {
        assert_eq!(
            integer_pages(Some(&json!([0, 1.5, 2, "3"]))),
            Some(vec![0, 2])
        );
        assert_eq!(integer_pages(Some(&json!("x"))), None);
        assert_eq!(template(Some(&json!(3))), "3");
        assert_eq!(template(None), "undefined");
    }

    #[test]
    fn memory_boxes_join_the_preview() {
        let mut session = Session::new();
        assert_eq!(session.redact_view(), None);
        session.redact_memory.push(RedactRegion {
            page: 2,
            rect: vec![0.0, 0.0, 1.0, 1.0],
        });
        session.redact_memory.push(RedactRegion {
            page: 0,
            rect: vec![0.0, 0.0, 1.0, 1.0],
        });
        session.redact_memory.push(RedactRegion {
            page: 2,
            rect: vec![0.0, 0.0, 1.0, 1.0],
        });
        let view = session.redact_view().unwrap();
        assert_eq!(view.marks, 3);
        assert_eq!(view.pages, vec![0, 2]);
        assert_eq!(
            view.page_counts.iter().map(|c| c.marks).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(session.redact_page_rects(2).len(), 2);
    }

    #[test]
    fn other_changes_skip_redaction_ops() {
        use super::super::types::RecoveryEntry;
        let mut session = Session::new();
        assert!(!session.other_unsaved_changes());
        session.dirty = true;
        assert!(session.other_unsaved_changes()); // no recovery log: any change counts
        session.recovery.base = Some(vec![1]);
        let entry = |op: &str, message: Value| RecoveryEntry {
            op: op.into(),
            message: message.as_object().cloned().unwrap_or_default(),
            ..RecoveryEntry::default()
        };
        session.recovery.entries = vec![
            entry("annot-add", json!({})),
            entry("saved", json!({})),
            entry("redact-add", json!({})),
        ];
        assert!(!session.other_unsaved_changes());
        session
            .recovery
            .entries
            .push(entry("annot-delete", json!({"page": 0, "annotId": 7})));
        assert!(session.other_unsaved_changes());
        session.redact_deletes.insert("0:7".into());
        assert!(!session.other_unsaved_changes());
    }
}
