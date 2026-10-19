//! The drive STATE (renderer.ts R:9851-10042): `stateSummary`, `editSummary` (also the
//! `P2_EDIT` payload), `directStateActive`, `driveWanted` and the small summaries they use.
//! spec-drive-state.md §4 is the authority for every key, and for when a key is absent.
//! No document text beyond the capped fields, no paths (except `batch`), no passwords.

use super::Session;
use super::decorate::slice_utf16;
use super::dom_ids::*;
use super::types::*;
use crate::host::util::js_number;
use serde_json::{Map, Value, json};

/// Lane 5's state that `Session` has no field for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lane5State {
    /// `signLast.warning` (R:10577; `SignLast` has no such field): the record it was
    /// written with and the warning; STATE adds it while `sign_last` is still that record.
    pub sign_warning: Option<(SignLast, String)>,
    /// The Verify table (`#verify-list` rows, R:10384-10399).
    pub verify_rows: Vec<super::verify::VerifyRow>,
    /// The Verify dialog's trusted list (`#verify-trusted`, one CN per item).
    pub verify_trusted: Vec<String>,
}

/// R:7600 `round2`.
pub(crate) fn round2(value: f64) -> f64 {
    crate::js::js_round(value * 100.0) / 100.0
}

/// R:7601 `round3`.
pub(crate) fn round3(value: f64) -> f64 {
    crate::js::js_round(value * 1000.0) / 1000.0
}

/// Numbers as JS prints them (whole values as integers, `-0` as 0).
fn nums(values: &[f64]) -> Value {
    Value::Array(values.iter().map(|&v| js_number(v)).collect())
}

fn nums2(values: &[f64]) -> Value {
    Value::Array(values.iter().map(|&v| js_number(round2(v))).collect())
}

/// `value.length` of a JS string (UTF-16 units).
fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `String(value)` of a field value.
fn field_value_text(value: &FieldValue) -> String {
    match value {
        FieldValue::Bool(b) => b.to_string(),
        FieldValue::Text(t) => t.clone(),
    }
}

/// R:9887 `cssBox(editor)`: `{left, top, width, height}` CSS px of an inline editor.
pub(crate) fn css_box(editor: &EditorState) -> Value {
    json!({
        "left": js_number(editor.left), "top": js_number(editor.top),
        "width": js_number(editor.width), "height": js_number(editor.height),
    })
}

/// Serde value of a serialisable record (the derived shapes match the TS objects).
fn to_value<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn opt_value<T: serde::Serialize>(value: Option<&T>) -> Value {
    value.map_or(Value::Null, to_value)
}

impl Session {
    /// R:9855: the selected inspector tab id, read live from its tablist (A2 §8.13).
    pub fn active_inspector_tab_id(&self) -> String {
        INSPECTOR_TABS
            .iter()
            .find(|tab| self.dom.attr(tab, "aria-selected").as_deref() == Some("true"))
            .map_or(String::new(), |tab| tab.to_string())
    }

    /// R:9861 (item 6): each Comments row's words, in `comments` order.
    pub fn drive_comment_labels(&mut self) -> Vec<String> {
        let notes = self.notes.clone();
        notes
            .iter()
            .map(|item| {
                let name = super::pages::annot_name(item)
                    .map_or_else(|| item.r#type.clone(), str::to_string);
                format!("{name}: {}", self.annot_summary(item))
            })
            .collect()
    }

    /// R:9865 (A2 §8.4): the shown page's annotations, text capped at 200 UTF-16 units.
    pub fn drive_comments(&self) -> Value {
        Value::Array(
            self.notes
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    json!({
                        "index": index, "type": item.r#type,
                        "contents": slice_utf16(&item.contents, 200), "editable": item.editable,
                    })
                })
                .collect(),
        )
    }

    /// R:9872: the `contents` of a parameterized drive op: `{}` when absent, None when it is
    /// not a JSON object (§1: the refusal lands in STATE `error`, never silently).
    pub fn drive_wanted(&mut self, contents: Option<&str>) -> Option<Map<String, Value>> {
        let Some(contents) = contents else {
            return Some(Map::new());
        };
        if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(contents) {
            return Some(map);
        }
        self.show_error("The drive contents must be a JSON object");
        None
    }

    /// R:9890: the open inline editor, or the focused object / link / mark / field, as STATE
    /// `direct.active` (spec §4.9). Null while the hit layer is not live.
    pub fn direct_state_active(&self) -> Value {
        if !self.layer_live() {
            return Value::Null;
        }
        let page = self.wanted_page;
        if let Some(active) = &self.direct_active {
            let point = active.point.map_or(Value::Null, |p| nums(&p));
            match active.kind {
                DirectKind::Textbox => {
                    return json!({
                        "kind": "textbox", "annotId": active.annot_id, "page": active.page, "point": point,
                        "box": css_box(&active.editor), "draft": active.editor.value,
                    });
                }
                DirectKind::Note => {
                    return json!({
                        "kind": "note", "page": active.page, "point": point,
                        "box": css_box(&active.editor), "draft": active.editor.value,
                    });
                }
                DirectKind::Add => {
                    // `Number(dataset.fontSize) || 0`.
                    let size = active
                        .editor
                        .font_size
                        .as_deref()
                        .map(super::decorate::js_number_of)
                        .filter(|v| !v.is_nan())
                        .unwrap_or(0.0);
                    return json!({
                        "kind": "add", "page": active.page, "point": point, "fontSize": js_number(size),
                        "box": css_box(&active.editor), "draft": active.editor.value,
                    });
                }
                _ => {}
            }
        }
        if self.direct_active.is_none()
            && let Some(focus) = &self.direct_focused_object
        {
            let node = self.hit_layer_box(".object-target", &focus.target_id);
            let item = self.content_scan.as_ref().and_then(|scan| {
                scan.items
                    .iter()
                    .find(|value| value.target_id == focus.target_id)
            });
            if let (Some(node), Some(item)) = (node, item) {
                return json!({
                    "kind": "object", "name": focus.name, "targetId": focus.target_id, "page": page,
                    "box": node,
                    "placement": item.placement.as_ref().map_or(Value::Null, |p| nums(&p.r#box)),
                    "ops": item.object_ops.as_ref().map_or(json!([]), to_value),
                });
            }
        }
        if let Some(active) = self
            .direct_active
            .as_ref()
            .filter(|a| a.kind == DirectKind::Link)
            && let DirectItem::Link(item) = &active.item
        {
            let annot_id = super::bookmarks_links::link_annot_id(item);
            let current = self
                .links
                .iter()
                .find(|entry| super::bookmarks_links::link_annot_id(entry) == annot_id)
                .unwrap_or(item);
            return self.link_state(
                current,
                css_box(&active.editor),
                true,
                Some(&active.editor.value),
            );
        }
        if self.direct_active.is_none()
            && let Some(focus) = self.direct_focused_link
        {
            let node = self.hit_layer_box(".link-target", &focus.annot_id.to_string());
            let item = self
                .links
                .iter()
                .find(|entry| super::bookmarks_links::link_annot_id(entry) == focus.annot_id);
            if let (Some(node), Some(item)) = (node, item) {
                return self.link_state(item, node, false, None);
            }
        }
        if self.direct_active.is_none()
            && let Some(focus) = self.direct_focused_mark
        {
            let node = self.hit_layer_box(".redact-target", &focus.annot_id.to_string());
            let item = self.notes.iter().find(|value| value.id == focus.annot_id);
            if let (Some(node), Some(item)) = (node, item) {
                return json!({
                    "kind": "redact", "annotId": focus.annot_id, "page": page, "box": node, "rect": nums(&item.rect),
                });
            }
        }
        if let Some(active) = &self.direct_active {
            let mut out = Map::new();
            let kind = if active.paragraph.is_some() {
                "paragraph"
            } else {
                active.kind.as_str()
            };
            out.insert("kind".into(), json!(kind));
            let (rect, value) = match &active.item {
                DirectItem::Field(field) => {
                    out.insert("name".into(), json!(field.name));
                    (nums(&field.rect), field_value_text(&field.value))
                }
                DirectItem::Target(target) => {
                    out.insert("targetId".into(), json!(target.target_id));
                    if let Some(paragraph) = &active.paragraph {
                        out.insert("paragraphId".into(), json!(paragraph.paragraph_id));
                        out.insert("lines".into(), json!(paragraph.lines));
                        out.insert("width".into(), js_number(paragraph.width));
                    } else {
                        if let Some(size) = target.font_size {
                            out.insert("fontSize".into(), js_number(size));
                        }
                        if let Some(moveable) = target.moveable {
                            out.insert("moveable".into(), json!(moveable));
                        }
                    }
                    let rect = match &active.paragraph {
                        Some(paragraph) => nums(&paragraph.bounds),
                        None => target.bounds.as_ref().map_or(Value::Null, |b| nums(b)),
                    };
                    let value = match &active.paragraph {
                        Some(paragraph) => self.paragraph_text(paragraph, active.scan.as_ref()),
                        None => target.text.clone().unwrap_or_default(),
                    };
                    (rect, value)
                }
                DirectItem::Link(item) => {
                    // Unreachable in the TS (a link editor returned above); kept total.
                    (nums(&item.rect), String::new())
                }
            };
            out.insert("page".into(), json!(active.page));
            out.insert("rect".into(), rect);
            out.insert("box".into(), css_box(&active.editor));
            out.insert("fontPx".into(), js_number(active.editor.font_px));
            out.insert("value".into(), json!(value));
            out.insert("draft".into(), json!(active.editor.value));
            return Value::Object(out);
        }
        let item = if self.direct_focused_page == self.wanted_page {
            self.direct_focused_field.as_ref()
        } else {
            None
        };
        match item {
            Some(item) => {
                let value = field_value_text(&item.value);
                json!({ "kind": "field", "name": item.name, "page": page, "value": value, "draft": value })
            }
            None => Value::Null,
        }
    }

    /// R:9926 `stateSummary()`: the `STATE` line (spec §4.1, keys in emission order).
    pub fn state_summary(&mut self) -> Value {
        let mut out = Map::new();
        let doc = self.doc.as_ref();
        out.insert(
            "doc".into(),
            doc.map_or(Value::Null, |d| {
                json!({
                    "name": d.name, "pageCount": d.page_count,
                    "readOnly": super::opening::is_read_only(&d.info), "needsPassword": d.info.needs_password,
                })
            }),
        );
        let cert_open = self.dom.is_open(CERT_SIGN_DIALOG);
        out.insert(
            "sign".into(),
            json!({
                "last": self.sign_last_value(),
                "dialog": match (&self.cert_sign_field, cert_open) {
                    (Some(field), true) => json!({ "field": field.name, "page": field.page }),
                    _ => Value::Null,
                },
                // Item 9: whether the certificate dialog is open and its "Sign and save as…" can be pressed.
                "certificateDialog": if cert_open {
                    json!({ "ready": !self.dom.disabled(CERT_SIGN_GO), "label": self.dom.text(CERT_SIGN_GO) })
                } else {
                    Value::Null
                },
            }),
        );
        out.insert("signatures".into(), self.signatures_summary());
        out.insert(
            "verify".into(),
            json!({ "last": opt_value(self.verify_last.as_ref()) }),
        );
        out.insert("mode".into(), json!(self.ui_mode.as_str()));
        // Item 5: the last markup and the page text selected now.
        let selection = self
            .selection_markup()
            .map_or(Value::Null, |s| json!(s.text));
        out.insert(
            "markup".into(),
            json!({ "last": opt_value(self.markup_last.as_ref()), "selection": selection }),
        );
        // X6: the armed tool as its chip names it, or null.
        out.insert(
            "tool".into(),
            if doc.is_some() {
                json!(self.armed_tool())
            } else {
                Value::Null
            },
        );
        // A2 §2: page/zoom/search/tab/comments exist only while a document is open; the fixed
        // zoom carries a rounded `scale`, the fit modes only their mode (§8.16).
        if doc.is_some() {
            out.insert("page".into(), json!(self.wanted_page));
            let zoom = if self.zoom_mode == ZoomMode::Fixed {
                json!({ "mode": "fixed", "scale": js_number(round3(self.zoom_now(self.wanted_page))) })
            } else {
                json!({ "mode": self.zoom_mode.as_str() })
            };
            out.insert("zoom".into(), zoom);
            out.insert(
                "search".into(),
                json!({ "query": self.search_query, "count": self.search_hits.len(), "index": self.search_index }),
            );
            out.insert("tab".into(), json!(self.active_inspector_tab_id()));
            out.insert("comments".into(), self.drive_comments());
            out.insert("commentLabels".into(), json!(self.drive_comment_labels()));
        }
        if let Some(last) = &self.annot_last {
            out.insert("annotLast".into(), to_value(last));
        }
        out.insert("redact".into(), self.redact_state_summary());
        out.insert("status".into(), json!(self.dom.text(STATUS_EL)));
        let hidden = self.dom.hidden(ERROR_EL);
        let quiet = matches!(self.notice_kind, MessageKind::Success | MessageKind::Info);
        out.insert(
            "error".into(),
            json!(if hidden || quiet {
                ""
            } else {
                self.notice_raw.as_str()
            }),
        );
        // X7/X8: what the notice bar shows the person, in plain words, and its kind.
        out.insert(
            "message".into(),
            if hidden {
                Value::Null
            } else {
                json!({ "kind": self.notice_kind.as_str(), "text": self.dom.text(ERROR_EL) })
            },
        );
        out.insert("recovery".into(), self.recovery_state_summary());
        let edit = self.dom.text(EDIT_STATUS_EL);
        let announcement = if edit.is_empty() {
            self.dom.text(NAV_STATUS_EL)
        } else {
            edit
        };
        out.insert("announcement".into(), json!(announcement));
        out.insert("canRedo".into(), json!(self.can_redo));
        out.insert(
            "help".into(),
            json!({ "open": self.dom.is_open(HELP_DIALOG), "rows": self.help_rows() }),
        );
        if self.batch_state.is_some() {
            out.insert("batch".into(), self.batch_summary());
        }
        out.insert("compare".into(), self.compare_summary());
        // `...editSummary()`: its `sign` (a signed stamp on the shown page) replaces the
        // object above in place.
        if let Value::Object(edit) = self.edit_summary() {
            for (key, value) in edit {
                out.insert(key, value);
            }
        }
        out.insert("fill".into(), self.fill_state_summary());
        out.insert("content".into(), self.content_summary());
        if let Some(save) = &self.last_save {
            out.insert("save".into(), Value::Object(save.clone()));
        }
        if let Some(print) = &self.last_print {
            out.insert("print".into(), Value::Object(print.clone()));
        }
        Value::Object(out)
    }

    /// R:9985 `logState()`: the `STATE` line.
    pub fn log_state(&mut self) {
        let state = self.state_summary();
        self.console_line("STATE", state);
    }

    /// `signLast`, with the `warning` the SignLast type has no field for (see `Lane5State`).
    fn sign_last_value(&self) -> Value {
        let Some(last) = &self.sign_last else {
            return Value::Null;
        };
        let mut value = to_value(last);
        if let Some((owner, warning)) = &self.lane5.sign_warning
            && owner == last
        {
            value["warning"] = json!(warning);
        }
        value
    }

    /// STATE `redact` (spec §4.4, R:9946-9964).
    fn redact_state_summary(&self) -> Value {
        use super::dom_ids::redact_ui as ui;
        let state = self.redact_state.as_ref();
        let rects: Vec<Value> = self
            .notes
            .iter()
            .filter(|item| item.r#type == "Redact")
            .map(|item| json!({ "id": item.id, "rect": nums(&item.rect), "search": item.search == Some(true) }))
            .collect();
        let flatten_note = if self.dom.hidden(ui::FLATTEN_NOTE_EL) {
            String::new()
        } else {
            self.dom.text(ui::FLATTEN_NOTE_EL)
        };
        json!({
            "marks": state.map_or(0, |s| s.marks),
            "pages": state.map_or(json!([]), |s| json!(s.pages)),
            "rects": rects,
            "preview": state.map_or(Value::Null, |s| json!({
                "shared": s.shared.len(), "sanitize": to_value(&s.sanitize), "inlineAlt": s.inline_alt,
            })),
            "last": opt_value(self.redact_last.as_ref()),
            // Redaction redesign §2.3: the tool, its panel, the dialog, the save prompt and the copy's banner.
            "tool": self.redact_ui.active,
            "memory": self.redact_memory.len(),
            // Review M1: what main's close and open prompts would name ("count:memory").
            "unsaved": self.redact_pending_sent,
            "tint": self.redact_ui.tint.as_ref().map_or(0, |t| t.shapes.len()),
            "flattenNote": flatten_note,
            "underneath": self.dom.checked(ui::UNDERNEATH_INPUT),
            "panel": { "boxes": self.redact_ui.boxes.len(), "button": self.dom.text(ui::APPLY_OPEN) },
            "dialog": self.dom.is_open(ui::CONFIRM_DIALOG),
            "prompt": self.redact_prompt.as_ref().map_or(Value::Null, |p| json!({
                "marks": p.marks, "choice": p.choice.as_str(), "action": p.action.as_str(),
            })),
            "banner": self.redact_ui.banner_state.as_ref().map_or(Value::Null, |b| json!({
                "name": b.name, "areas": b.areas, "original": b.original, "overRedacted": b.over_redacted,
            })),
        })
    }

    /// STATE `recovery` (spec §4.5, R:9970, the RecoveryLog getters of recovery.ts:265-272).
    fn recovery_state_summary(&self) -> Value {
        let log = &self.recovery;
        let bytes = log.base.as_ref().map_or(0, Vec::len)
            + log.saved.as_ref().map_or(0, Vec::len)
            + log.entry_bytes;
        let last = self.restore_last.as_ref().map_or(Value::Null, |last| {
            let mut value = json!({
                "outcome": last.outcome.as_str(), "dropped": last.dropped, "cause": last.cause.as_str(),
                "lost": last.lost, "steps": last.steps,
            });
            if let Some(over) = last.over_budget {
                value["overBudget"] = json!(over);
            }
            value
        });
        json!({
            "available": log.base.is_some(),
            "unavailable": log.unavailable.map(Unavailable::as_str),
            "entries": log.entries.len(),
            "bytes": bytes,
            "saved": log.saved.is_some(),
            "budget": log.budget,
            "restoring": self.restore_running,
            "dirtyReported": self.reported_dirty,
            "progress": self.restore_progress.map_or(Value::Null, |p| json!({ "done": p.done, "total": p.total })),
            "paused": self.paused_reads,
            "last": last,
        })
    }

    /// STATE `fill` (spec §4.7; fill-anywhere D6/D7).
    fn fill_state_summary(&self) -> Value {
        let boxes: Vec<Value> = self
            .notes
            .iter()
            .filter(|item| item.r#type == "FreeText")
            .map(|item| json!({ "id": item.id, "rect": nums2(&item.rect), "chars": js_len(&item.contents) }))
            .collect();
        let (kind, strokes, points) = match &self.saved_signature {
            None => (Value::Null, 0, 0),
            Some(SavedSignature::Image(image)) => (
                json!(if image.kind == "image" {
                    "image"
                } else {
                    "drawn"
                }),
                0,
                0,
            ),
            Some(SavedSignature::Drawn(drawn)) => (
                json!("drawn"),
                drawn.strokes.len(),
                drawn.strokes.iter().map(Vec::len).sum::<usize>(),
            ),
        };
        let signatures: Vec<Value> = self
            .notes
            .iter()
            .filter(|item| self.is_signature(item))
            .map(|item| {
                json!({
                    "id": item.id, "rect": nums2(&item.rect),
                    "kind": if self.is_drawn_signature(item) { "drawn" } else { "image" },
                })
            })
            .collect();
        json!({
            "filled": self.fill_save != FillSave::None,
            "saveAsCopy": self.fill_save == FillSave::NeedsCopy,
            "boxes": boxes,
            "last": opt_value(self.fill_last.as_ref()),
            "signature": {
                "saved": self.saved_signature.is_some(), "kind": kind, "strokes": strokes, "points": points,
                "armed": self.sign_armed, "pad": self.dom.is_open(SIGPAD_DIALOG),
            },
            "signatures": signatures,
            "lastSignature": opt_value(self.fill_sign_last.as_ref()),
        })
    }

    /// R:9991 `editSummary()`: the edit slice of STATE and the `P2_EDIT` payload (spec §4.3).
    pub fn edit_summary(&self) -> Value {
        const TYPES: [&str; 11] = [
            "Text",
            "Highlight",
            "Underline",
            "StrikeOut",
            "FreeText",
            "Square",
            "Circle",
            "Line",
            "Ink",
            "Stamp",
            "Redact",
        ];
        let mut annots = Map::new();
        for kind in TYPES {
            annots.insert(
                kind.into(),
                json!(self.notes.iter().filter(|item| item.r#type == kind).count()),
            );
        }
        let doc = self.doc.as_ref();
        let mut out = Map::new();
        out.insert("revision".into(), json!(self.revision));
        out.insert("dirty".into(), json!(self.dirty));
        out.insert("canUndo".into(), json!(self.can_undo));
        out.insert("notes".into(), annots["Text"].clone());
        out.insert("annots".into(), Value::Object(annots));
        let mut pages = json!({
            "count": doc.map_or(0, |d| d.page_count),
            "rotate": doc.map_or(json!([]), |d| json!(d.rotations)),
        });
        if let Some(ids) = &self.page_ids {
            pages["ids"] = json!(ids);
        }
        out.insert("pages".into(), pages);
        out.insert("fields".into(), self.field_summary());
        out.insert("direct".into(), self.direct_summary());
        let outline: Vec<Value> = self
            .outline
            .iter()
            .map(|entry| match (&entry.page, &entry.uri) {
                (None, Some(uri)) if !uri.is_empty() => {
                    json!({ "title": entry.title, "page": null, "uri": uri })
                }
                _ => json!({ "title": entry.title, "page": entry.page }),
            })
            .collect();
        let nav_links: Map<String, Value> = self
            .nav_links
            .iter()
            .map(|(page, items)| (page.to_string(), to_value(items)))
            .collect();
        let mut nav = json!({
            "outline": outline,
            "links": nav_links,
            "linkDraft": self.nav_link_draft.as_ref().map_or(Value::Null, |d| json!({ "page": d.page, "rect": nums(&d.rect) })),
        });
        if let Some(last) = &self.nav_last {
            nav["last"] = to_value(last);
        }
        out.insert("nav".into(), nav);
        // P6 U4 §1.8: the shown page's links with their stable ids; `nav.links` keeps its pinned shape.
        let links: Vec<Value> = if doc.is_some() {
            self.links
                .iter()
                .map(|item| {
                    let mut value = json!({
                        "id": item.id, "annotId": super::bookmarks_links::link_annot_id(item), "rect": nums2(&item.rect),
                        "kind": item.kind.as_str(),
                    });
                    if item.kind.as_str() == "page" {
                        if let Some(page) = item.page {
                            value["page"] = json!(page);
                        }
                    } else if let Some(uri) = &item.uri {
                        value["uri"] = json!(uri);
                    }
                    value
                })
                .collect()
        } else {
            Vec::new()
        };
        out.insert("links".into(), Value::Array(links));
        if let Some(summary) = &self.assemble_summary {
            out.insert("assemble".into(), to_value(summary));
        }
        if let Some(summary) = &self.extract_summary {
            out.insert("extract".into(), to_value(summary));
        }
        if let Some(summary) = &self.export_summary {
            out.insert("export".into(), to_value(summary));
        }
        if let Some(summary) = &self.decorate_summary {
            let mut value = to_value(summary);
            value["bates"] = json!({
                "next": self.bates_next, "pad": self.bates_pad, "last": opt_value(self.bates_last.as_ref()),
            });
            out.insert("decorate".into(), value);
        }
        if let Some((name, subj)) = self.sign_from_notes() {
            out.insert("sign".into(), json!({ "name": name, "subj": subj }));
        }
        if let Some(security) = &self.security_state {
            out.insert(
                "security".into(),
                json!({
                    "mode": security.mode.as_str(), "algorithm": security.algorithm.as_str(),
                    "permissions": security.permissions.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
                    "auth": security.auth.as_str(),
                }),
            );
        }
        Value::Object(out)
    }

    /// `editSummary().direct` (spec §4.9, R:10005-10024).
    fn direct_summary(&self) -> Value {
        let live = self.layer_live();
        let mode = self.ui_mode;
        let doc = self.doc.is_some();
        let gesture = self.direct_gesture.as_ref().map_or(Value::Null, |g| {
            let mut value = json!({
                "kind": g.kind.as_str(), "handle": g.handle, "targetId": to_value(&g.target_id), "rect": nums(&g.rect),
            });
            if let Some(guides) = &g.guides {
                value["guides"] = Value::Array(
                    guides.iter().map(|guide| json!({ "axis": guide.axis.as_str(), "at": js_number(guide.at) })).collect(),
                );
            }
            value
        });
        // P6 U5 §2.5 / U7 §10.29: the object selection (click order, anchor last), every member kind.
        let selection: Vec<Value> = if doc {
            self.direct_selection
                .iter()
                .map(|member| json!({ "targetId": member.target_id, "kind": member.kind, "name": member.name }))
                .collect()
        } else {
            Vec::new()
        };
        // P6 U6 §2.3: crop mode (view space; `full` is the visible box when uncropped).
        let crop = match (&self.direct_crop, &self.direct_focused_object) {
            (Some(crop), Some(focus)) if doc && mode == UiMode::Edit => {
                json!({ "name": focus.name, "full": nums2(&crop.full) })
            }
            _ => Value::Null,
        };
        let targets = if !live || mode == UiMode::Redact {
            0
        } else if mode == UiMode::Edit {
            self.content_scan.as_ref().map_or(0, |scan| {
                scan.items
                    .iter()
                    .filter(|item| {
                        item.bounds.as_ref().is_some_and(|b| b.len() == 4)
                            && item.kind.as_str() == "text"
                    })
                    .count()
            })
        } else {
            self.fields
                .iter()
                .filter(|item| item.kind.as_str() != "signature")
                .count()
        };
        json!({
            "mode": mode.as_str(),
            "active": self.direct_state_active(),
            "gesture": gesture,
            "selection": selection,
            "menu": self.object_menu_state().unwrap_or(Value::Null),
            // P6 U7 §2.5: this window's clipboard (count and kinds only).
            "clipboard": self.clipboard_info.as_ref().map_or(Value::Null, |c| json!({ "count": c.count, "kinds": c.kinds })),
            "crop": crop,
            "highlight": self.highlight_fields,
            "targets": targets,
            "objects": if mode == UiMode::Edit && live { self.hit_layer_count(".object-target") } else { 0 },
            "marks": if mode == UiMode::Redact && live { self.hit_layer_count(".redact-target") } else { 0 },
            "links": if live { self.hit_layer_count(".link-target") } else { 0 },
            "outlines": self.show_outlines,
            "status": self.dom.text(FIELDS_STATUS_EL),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_matches_js() {
        assert_eq!(round2(1.005), 1.0);
        assert_eq!(round2(2.345), 2.35);
        assert_eq!(round3(1.2345), 1.235);
        assert_eq!(nums(&[1.0, -0.0, 2.5]), json!([1, 0, 2.5]));
    }

    #[test]
    fn drive_wanted_parses_objects_only() {
        let mut s = Session::new();
        assert_eq!(s.drive_wanted(None), Some(Map::new()));
        assert_eq!(s.drive_wanted(Some(r#"{"a":1}"#)).unwrap()["a"], json!(1));
        assert!(s.drive_wanted(Some("[1]")).is_none());
        assert_eq!(s.notice_raw, "The drive contents must be a JSON object");
        assert!(s.drive_wanted(Some("nope")).is_none());
    }

    #[test]
    fn inspector_tab_reads_aria_selected() {
        let s = Session::new();
        assert_eq!(s.active_inspector_tab_id(), "tab-links");
    }

    #[test]
    fn css_box_prints_integers() {
        let editor = EditorState {
            left: 10.0,
            top: 2.5,
            width: 100.0,
            height: 20.0,
            ..EditorState::default()
        };
        assert_eq!(
            css_box(&editor),
            json!({"left": 10, "top": 2.5, "width": 100, "height": 20})
        );
    }
}
