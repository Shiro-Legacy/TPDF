//! Save, Save As and Save a copy (renderer.ts R:11501-11756; spec §3.C.4): the engine's
//! `save` then `validate` (the bytes must read back), then `self.host` writes the file. The
//! worker gate and the latest-guards collapse: the engine answers at once; the `superseded`
//! checks stay (a host call can open another document).

use super::Session;
use super::dom_ids::*;
use super::types::*;
use crate::engine::Refusal;
use crate::host::SaveOptions;
use crate::messages::{self, MessageKind};
use serde_json::{Map, Value, json};

/// R:11505 `SAVE_REASON_TEXT[reason]`.
fn save_reason_text(reason: &str) -> Option<String> {
    messages::save::reason(reason)
}

/// The validated bytes, the save's `saveMs` and its ticket.
type Validated = (Vec<u8>, Value, Option<String>);

/// What a save flow did when it did not land: the TS's fall-through to its last lines
/// (`lastSave = {ok:false, reason}` and `P1_SAVE_ERROR`), or an early return.
enum Ended {
    Returned,
    Failed(String),
}

impl Session {
    /// R:11508: a new file's write failure (export, new PDF, signed copy) as text.
    pub fn new_file_reason_text(&self, reason: &str) -> Option<String> {
        messages::save::new_file(reason)
    }

    /// R:11680 (review fix 8): Save, Save As and Save a copy first commit what an open
    /// editor holds, as a click elsewhere would.
    pub fn commit_open_editor(&mut self) {
        if self.direct_active.is_some() {
            self.finish_direct();
        }
    }

    fn save_superseded(&self, generation: i64, url: &str) -> bool {
        generation != self.generation
            || self.doc.as_ref().and_then(|d| d.url.as_deref()) != Some(url)
    }

    /// The engine's `save` then `validate`: the validated bytes and `saveMs`.
    fn save_and_validate(
        &mut self,
        generation: i64,
        url: &str,
    ) -> Result<Option<Validated>, Refusal> {
        let save = self.request(json!({ "op": "save" }), vec![])?;
        if self.save_superseded(generation, url) {
            return Ok(None);
        }
        let saved = save.bytes("bytes").map(<[u8]>::to_vec).unwrap_or_default();
        let validated = self.request(
            json!({ "op": "validate", "pdf": { "$buffer": 0 } }),
            vec![saved],
        )?;
        if self.save_superseded(generation, url) {
            return Ok(None);
        }
        let bytes = validated
            .bytes("bytes")
            .map(<[u8]>::to_vec)
            .unwrap_or_default();
        if bytes.len() < 5 {
            return Err(Refusal::plain("validate: no bytes"));
        }
        let save_ms = save.get("saveMs").cloned().unwrap_or(Value::Null);
        let ticket = save
            .get("ticket")
            .and_then(Value::as_str)
            .map(str::to_string);
        Ok(Some((bytes, save_ms, ticket)))
    }

    /// R:11519. Plain Save: the same engine transaction as Save a copy, but main rewrites the
    /// source file in place (source-changed blocks instead of warning) and the engine only
    /// becomes clean once main reports the write landed. UX item 7: `mode` "save-as" is Save
    /// As…: main asks for a new file, writes it and binds the window to it.
    pub fn save_in_place(&mut self, mode: &str) {
        let save_as_file = mode == "save-as";
        self.commit_open_editor();
        let Some(doc) = &self.doc else {
            return;
        };
        let Some(url) = doc.url.clone() else {
            self.show_error(messages::open::SAMPLE);
            return;
        };
        let name = doc.name.clone();
        if !save_as_file && !self.editable() {
            self.show_error(messages::save::READ_ONLY);
            return;
        }
        if !save_as_file && !self.dirty {
            self.set_status(messages::save::NOTHING_TO_SAVE);
            return;
        }
        if self.saving {
            self.set_status("A save is already running…");
            return;
        }
        if !self.confirm_pending_save(PendingAction::Save) {
            return;
        }
        let generation = self.generation;
        self.saving = true;
        self.clear_error();
        self.clear_warning();
        self.sync_controls();
        self.set_status(&format!("Saving {name}…"));
        let mut landed = false;
        let mut disk_copy: Option<Vec<u8>> = None;
        let ended = self.save_in_place_run(
            save_as_file,
            &name,
            &url,
            generation,
            &mut landed,
            &mut disk_copy,
        );
        // A write that landed stays the disk's state until it is marked saved or another document opens.
        if !landed
            && let Some(copy) = &disk_copy
            && self.disk_write.as_ref().is_some_and(|w| w.bytes == *copy)
        {
            self.disk_write = None;
        }
        self.saving = false;
        self.sync_controls();
        if let Ended::Failed(reason) = ended {
            self.save_failed(&reason);
        }
    }

    fn save_failed(&mut self, reason: &str) {
        let mut last = Map::new();
        last.insert("ok".into(), json!(false));
        last.insert("reason".into(), json!(reason));
        self.last_save = Some(last);
        self.console_line("P1_SAVE_ERROR", json!({ "reason": reason }));
    }

    fn save_in_place_run(
        &mut self,
        save_as_file: bool,
        name: &str,
        url: &str,
        generation: i64,
        landed: &mut bool,
        disk_copy: &mut Option<Vec<u8>>,
    ) -> Ended {
        let (bytes, save_ms, ticket) = match self.save_and_validate(generation, url) {
            Ok(Some(saved)) => saved,
            Ok(None) => return Ended::Returned,
            Err(err) => return self.save_in_place_caught(&err, false, name),
        };
        let byte_length = bytes.len();
        let filled_copy = !save_as_file && self.fill_save == FillSave::NeedsCopy;
        // M9: what lands on disk is what a failed restore reopens. Until `saved` is recorded
        // (or another document opens), a restore that falls back opens these bytes.
        let dialog = save_as_file
            || filled_copy
            || self.doc.as_ref().is_some_and(|d| d.untitled == Some(true));
        *disk_copy = Some(bytes.clone());
        self.disk_write = Some(DiskWrite {
            done: None,
            bytes: bytes.clone(),
            dialog,
        });
        let options = SaveOptions {
            in_place: !save_as_file,
            filled: filled_copy,
            rebind: save_as_file,
            suggested_name: None,
        };
        let result = self.host.save_file(&bytes, url, &options);
        if let Some(write) = self.disk_write.as_mut().filter(|w| w.bytes == bytes) {
            write.done = Some(result.is_ok());
        }
        *landed = result.is_ok();
        // M9: a Save As (Untitled or filled) that landed after a restore reopened the last
        // saved state: main now binds the window to the written file, which the window shows.
        if let Ok(saved) = &result
            && dialog
            && self.generation == self.fallback_generation
            && self.doc.as_ref().and_then(|d| d.url.as_deref()) == Some(url)
        {
            self.show_late_save(&saved.name, bytes, url);
            return Ended::Returned;
        }
        if self.save_superseded(generation, url) {
            return Ended::Returned;
        }
        let saved = match result {
            Ok(saved) => saved,
            Err(fail) => {
                let reason = fail.as_str();
                if reason == "cancelled" {
                    self.set_status("Save cancelled");
                    // Bugfix 1 D3b: STATE save shows the cancel; a smoke --save-in-place ends on it.
                    let mut last = Map::new();
                    last.insert("ok".into(), json!(false));
                    last.insert("reason".into(), json!("cancelled"));
                    self.last_save = Some(last);
                    self.console_line("P1_SAVE_ERROR", json!({ "reason": "cancelled" }));
                    return Ended::Returned;
                }
                self.set_status("Not saved");
                let text = save_reason_text(reason)
                    .unwrap_or_else(|| "The document could not be saved.".into());
                self.show_error(&text);
                return Ended::Failed(reason.to_string());
            }
        };
        // The write landed: the state the save captured is the clean baseline now. The ticket
        // keeps an edit made while main was writing unsaved, so the window stays dirty.
        let mut message = json!({ "op": "saved" });
        if let Some(ticket) = ticket {
            message["ticket"] = json!(ticket);
        }
        let marked = match self.request(message, vec![]) {
            Ok(marked) => marked,
            Err(err) => return self.save_in_place_caught(&err, true, name),
        };
        if self.save_superseded(generation, url) {
            return Ended::Returned;
        }
        let kept = self.recovery_fits_saved(byte_length).then(|| bytes.clone());
        self.recovery_saved_copy(kept);
        if self.disk_write.as_ref().is_some_and(|w| w.bytes == bytes) {
            self.disk_write = None;
        }
        let journal = super::decorate::journal_of(&marked.value);
        self.revision = journal.revision;
        self.dirty = journal.dirty;
        self.can_undo = journal.can_undo;
        self.can_redo = journal.can_redo;
        self.mark_security_saved();
        // The window was rebound to another file (Save As, or the filled form's new file): it
        // is no longer the redacted copy the banner names (main forgot its original too).
        if (save_as_file && !saved.same_file) || (filled_copy && saved.name != name) {
            self.hide_banner();
        }
        // Bugfix 1 D3: Save on an untitled document wrote a new file and main rebound the window to it.
        if let Some(doc) = self.doc.as_mut() {
            if saved.name != doc.name {
                doc.name = saved.name.clone();
                self.dom.set_text(DOCUMENT_TITLE_EL, saved.name.clone());
            }
            doc.untitled = Some(false);
        }
        // fill-anywhere D7 / item 8: the filled form was saved (over itself or as a new file): asked once.
        if filled_copy || (save_as_file && self.fill_save == FillSave::NeedsCopy) {
            self.fill_save = FillSave::Copied;
        }
        self.sync_dirty();
        self.sync_controls();
        let status = if save_as_file && saved.name != name {
            messages::save::saved_as(&saved.name)
        } else {
            format!("Saved {}", saved.name)
        };
        self.set_status(&status);
        // Review fix 7: Save As onto the open file (under any spelling) was a plain Save.
        let wrote_in_place = !save_as_file || saved.same_file;
        let mut last = Map::new();
        last.insert("ok".into(), json!(true));
        last.insert("name".into(), json!(saved.name));
        last.insert("source".into(), json!(name));
        last.insert("bytes".into(), json!(byte_length));
        last.insert("saveMs".into(), save_ms);
        last.insert("validated".into(), json!(true));
        last.insert("inPlace".into(), json!(wrote_in_place));
        if save_as_file {
            last.insert("as".into(), json!(true));
        }
        self.console_line("P1_SAVE", Value::Object(last.clone()));
        self.last_save = Some(last);
        Ended::Returned
    }

    /// The `catch` of `saveInPlace`.
    fn save_in_place_caught(&mut self, err: &Refusal, landed: bool, name: &str) -> Ended {
        let reason = err.reason.clone().unwrap_or_else(|| "worker".into());
        if landed {
            // M9: the file was written but the engine stopped before it was marked saved; the
            // window stays marked unsaved.
            self.set_status(&format!(
                "Saved {name}, but the window still marks its changes unsaved"
            ));
            let mut last = Map::new();
            last.insert("ok".into(), json!(false));
            last.insert("reason".into(), json!(reason));
            last.insert("written".into(), json!(true));
            self.last_save = Some(last);
            self.console_line(
                "P1_SAVE_ERROR",
                json!({ "reason": reason, "written": true }),
            );
            return Ended::Returned;
        }
        self.set_status("Not saved");
        let text = save_reason_text(&reason).unwrap_or_else(|| self.message_of(err));
        self.show_error(&text);
        Ended::Failed(reason)
    }

    /// R:11662 (M9): a Save As that asked for a path landed after a restore reopened the last
    /// saved state. With nothing edited since, the window shows the written file; otherwise
    /// it keeps the edits and says the file holds another version.
    pub fn show_late_save(&mut self, name: &str, bytes: Vec<u8>, url: &str) {
        if !self.dirty {
            self.open_document(name, bytes, Some(url), true);
            let notice = messages::save::late_shown(name);
            self.show_message(&notice, MessageKind::Success);
            self.announce(&notice);
            return;
        }
        if let Some(doc) = self.doc.as_mut() {
            doc.name = name.to_string();
            doc.untitled = Some(false);
            self.dom.set_text(DOCUMENT_TITLE_EL, name);
            self.sync_dirty();
        }
        self.show_message(&messages::save::late_kept(name), MessageKind::Info);
    }

    /// R:11687 Save a copy: engine save → validate → main writes a new file; the window stays
    /// on the original and a pending security change stays pending.
    pub fn save_as(&mut self) {
        self.commit_open_editor();
        let Some(doc) = &self.doc else {
            return;
        };
        let Some(url) = doc.url.clone() else {
            self.show_error(messages::open::SAMPLE);
            return;
        };
        let name = doc.name.clone();
        if self.saving {
            self.set_status("A save is already running…");
            return;
        }
        if !self.confirm_pending_save(PendingAction::Save) {
            return;
        }
        let generation = self.generation;
        self.saving = true;
        self.clear_error();
        self.clear_warning();
        // The rewrite plus the validated copy is the memory peak: drop the page cache first.
        self.cache.entries.clear();
        self.cache.bytes = 0;
        self.sync_controls();
        self.set_status(&format!("Saving a copy of {name}…"));
        let ended = self.save_as_run(&name, &url, generation);
        self.saving = false;
        self.sync_controls();
        if let Ended::Failed(reason) = ended {
            self.save_failed(&reason);
        }
    }

    fn save_as_run(&mut self, name: &str, url: &str, generation: i64) -> Ended {
        let (bytes, save_ms, _) = match self.save_and_validate(generation, url) {
            Ok(Some(saved)) => saved,
            Ok(None) => return Ended::Returned,
            Err(err) => {
                let reason = err.reason.clone().unwrap_or_else(|| "worker".into());
                self.set_status("Not saved");
                let text = save_reason_text(&reason).unwrap_or_else(|| self.message_of(&err));
                self.show_error(&text);
                return Ended::Failed(reason);
            }
        };
        let byte_length = bytes.len();
        let result = self.host.save_file(&bytes, url, &SaveOptions::default());
        if self.save_superseded(generation, url) {
            return Ended::Returned;
        }
        match result {
            Ok(saved) => {
                self.set_status(&messages::save::saved_copy(&saved.name));
                self.show_warning(saved.warning);
                // `name` is the copy main wrote; `source` the document that was opened. No paths.
                let mut last = Map::new();
                last.insert("ok".into(), json!(true));
                last.insert("name".into(), json!(saved.name));
                last.insert("source".into(), json!(name));
                last.insert("bytes".into(), json!(byte_length));
                last.insert("saveMs".into(), save_ms);
                last.insert("validated".into(), json!(true));
                if let Some(warning) = saved.warning {
                    last.insert("warning".into(), json!(warning));
                }
                self.console_line("P1_SAVE", Value::Object(last.clone()));
                self.last_save = Some(last);
                Ended::Returned
            }
            Err(fail) => {
                let reason = fail.as_str();
                if reason == "cancelled" {
                    // No `save` record and no P1_SAVE_ERROR for a cancelled copy.
                    self.set_status("Save cancelled");
                    return Ended::Returned;
                }
                self.set_status("Not saved");
                let text = save_reason_text(reason)
                    .unwrap_or_else(|| "The copy could not be saved.".into());
                self.show_error(&text);
                Ended::Failed(reason.to_string())
            }
        }
    }

    /// R:11433 "Open…" (and Cmd/Ctrl+O): main shows the open sheet; the document arrives as
    /// a host event.
    pub fn request_open(&mut self) {
        self.clear_error();
        self.host.open_file();
    }
}
