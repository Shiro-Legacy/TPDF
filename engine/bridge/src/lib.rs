// SPDX-License-Identifier: AGPL-3.0-or-later
#![forbid(unsafe_code)]
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tandem_pdf_engine::app::{AppEngine, AppError, AppSession, error_reply, request};
use tandem_pdf_engine::session::{SaveToken, Session, SessionLimits, TextTarget};
use wasm_bindgen::prelude::*;
fn error(value: impl ToString) -> JsValue {
    JsValue::from_str(&value.to_string())
}

#[wasm_bindgen]
pub struct RustEngine {
    session: Session,
    targets: Vec<TextTarget>,
    pending_save: Option<SaveToken>,
    width: u32,
    height: u32,
}
#[wasm_bindgen]
impl RustEngine {
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<RustEngine, JsValue> {
        Self::with_password(bytes, "")
    }
    /// Open an encrypted document; failure reports `needs-password`.
    pub fn with_password(bytes: &[u8], password: &str) -> Result<RustEngine, JsValue> {
        if bytes.len() > SessionLimits::default().max_pdf_bytes {
            return Err(error("too-large: input exceeds 16 MiB"));
        }
        Ok(Self {
            session: Session::open_view_with_password(
                bytes.to_vec(),
                SessionLimits::default(),
                password.as_bytes(),
            )
            .map_err(error)?,
            targets: Vec::new(),
            pending_save: None,
            width: 0,
            height: 0,
        })
    }
    pub fn info(&self) -> Result<String, JsValue> {
        let state = self.session.state();
        let pages = self.session.pages().map_err(error)?;
        let capabilities = self.session.capabilities();
        let reason = |error: &Option<tandem_pdf_engine::session::SessionError>| {
            error
                .as_ref()
                .map(|e| json!({"reason":e.reason,"detail":e.detail}))
        };
        Ok(json!({"pageCount":pages.len(), "pages":pages.iter().map(|p|p.display_bounds()).collect::<Vec<_>>(),
            "capabilities":{"canSave":capabilities.save_error.is_none(), "saveError":reason(&capabilities.save_error),
                "pages":capabilities.pages.iter().map(|p|json!({"canRender":p.render_error.is_none(),
                    "canEditText":p.edit_error.is_none() && p.text_target_count>0,
                    "textTargetCount":p.text_target_count,
                    "renderError":reason(&p.render_error),"editError":reason(&p.edit_error)})).collect::<Vec<_>>()},
            "revision":state.revision,"generation":state.generation,"dirty":state.dirty,
            "canUndo":state.can_undo,"canRedo":state.can_redo,"width":self.width,"height":self.height}).to_string())
    }
    pub fn targets(&mut self, page: usize) -> Result<String, JsValue> {
        // A failed scan cannot leave old numeric target indices available.
        self.targets.clear();
        self.targets = self.session.text_targets(page).map_err(error)?;
        Ok(json!(
            self.targets
                .iter()
                .enumerate()
                .map(|(id, t)| json!({"targetId":id,"page":t.page,"text":t.text,
            "bounds":t.bounds,"repertoire":t.repertoire}))
                .collect::<Vec<_>>()
        )
        .to_string())
    }
    pub fn replace(&mut self, target: usize, text: &str) -> Result<String, JsValue> {
        let target = self
            .targets
            .get(target)
            .ok_or_else(|| error("stale-target: rescan text"))?;
        self.session.replace_text(target, text).map_err(error)?;
        self.targets.clear();
        self.info()
    }
    pub fn undo(&mut self) -> Result<String, JsValue> {
        self.session.undo().map_err(error)?;
        self.targets.clear();
        self.info()
    }
    pub fn redo(&mut self) -> Result<String, JsValue> {
        self.session.redo().map_err(error)?;
        self.targets.clear();
        self.info()
    }
    pub fn render(&mut self, page: usize, dpi: f64) -> Result<Vec<u8>, JsValue> {
        let image = self.session.render(page, dpi).map_err(error)?;
        self.width = image.width();
        self.height = image.height();
        Ok(image.pixels().to_vec())
    }
    pub fn save(&mut self) -> Result<Vec<u8>, JsValue> {
        if self.pending_save.is_some() {
            return Err(error("save-busy: a save awaits acknowledgement"));
        }
        let saved = self.session.save().map_err(error)?;
        self.pending_save = Some(saved.token);
        Ok(saved.bytes)
    }
    pub fn saved(&mut self) -> Result<String, JsValue> {
        let token = self
            .pending_save
            .take()
            .ok_or_else(|| error("stale-save: no pending save"))?;
        self.session.acknowledge_saved(token).map_err(error)?;
        self.info()
    }
    pub fn cancel_save(&mut self) {
        self.pending_save = None;
    }
    /// Validation for a write must retain the strict workflow gate even though
    /// ordinary opening now admits documents with unavailable operations.
    pub fn validate(bytes: &[u8]) -> Result<usize, JsValue> {
        if bytes.len() > SessionLimits::default().max_pdf_bytes {
            return Err(error("too-large: input exceeds 16 MiB"));
        }
        let session = Session::open(bytes.to_vec(), SessionLimits::default()).map_err(error)?;
        Ok(session.pages().map_err(error)?.len())
    }
}

/// The application engine behind the worker's `SessionRequest` protocol.
/// Requests and replies are JSON; binary fields are `{"$buffer": n}`
/// placeholders into the inputs pushed before a dispatch and the outputs it
/// leaves behind. A panic poisons the object: wasm-bindgen refuses every
/// later call, so the adapter treats any throw as fatal.
#[wasm_bindgen]
#[derive(Default)]
pub struct RustApp {
    engine: AppEngine,
    sessions: BTreeMap<u32, AppSession>,
    next_session: u32,
    inputs: Vec<Vec<u8>>,
    outputs: Vec<Vec<u8>>,
}
#[wasm_bindgen]
impl RustApp {
    #[wasm_bindgen(constructor)]
    pub fn new() -> RustApp {
        Self::default()
    }
    pub fn create_session(&mut self) -> u32 {
        self.next_session = self.next_session.wrapping_add(1);
        self.sessions
            .insert(self.next_session, AppSession::default());
        self.next_session
    }
    pub fn drop_session(&mut self, session: u32) {
        self.sessions.remove(&session);
    }
    /// Queues an input buffer; the next dispatch takes every queued input.
    pub fn push_input(&mut self, bytes: Vec<u8>) {
        self.inputs.push(bytes);
    }
    /// Drops queued inputs, so a request abandoned before its dispatch leaves
    /// nothing for the next one's placeholders to bind.
    pub fn clear_inputs(&mut self) {
        self.inputs.clear();
    }
    /// One request in, one reply out. Errors are `ok: false` replies, never throws.
    pub fn dispatch(&mut self, session: u32, request: &str) -> String {
        let inputs = std::mem::take(&mut self.inputs);
        self.outputs.clear();
        let request = match serde_json::from_str::<Value>(request) {
            Ok(request) => request,
            Err(error) => {
                let error = AppError::plain(format!("invalid request: {error}"));
                let fallback = AppSession::default();
                let session = self.sessions.get(&session).unwrap_or(&fallback);
                return error_reply(&Value::Null, error, session).to_string();
            }
        };
        let Some(state) = self.sessions.get_mut(&session) else {
            let error = AppError::plain(format!("no session {session}"));
            return error_reply(&request, error, &AppSession::default()).to_string();
        };
        let (reply, outputs) = self.engine.dispatch(state, request, inputs);
        self.outputs = outputs;
        reply.to_string()
    }
    pub fn output_count(&self) -> u32 {
        self.outputs.len() as u32
    }
    /// Moves an output out; a second take of the same index is empty.
    pub fn take_output(&mut self, index: u32) -> Vec<u8> {
        self.outputs
            .get_mut(index as usize)
            .map(std::mem::take)
            .unwrap_or_default()
    }
    /// `AppSession::state_json`, or `null` for an unknown session.
    pub fn session_state(&self, session: u32) -> String {
        self.sessions
            .get(&session)
            .map_or(Value::Null, AppSession::state_json)
            .to_string()
    }
    /// The session's current document bytes, for test inspection.
    pub fn snapshot_bytes(&self, session: u32) -> Option<Vec<u8>> {
        self.sessions.get(&session)?.snapshot_bytes()
    }
    /// The bytes of the session's compare document, for test inspection.
    pub fn compare_bytes(&self, session: u32) -> Option<Vec<u8>> {
        let compare = self.sessions.get(&session)?.compare.as_ref()?;
        Some(compare.bytes().to_vec())
    }
    /// Test-only: `AppSession::debug_json`, or `null` for an unknown session.
    pub fn session_debug(&self, session: u32) -> String {
        self.sessions
            .get(&session)
            .map_or(Value::Null, AppSession::debug_json)
            .to_string()
    }
    /// Test-only: the fields tests assign directly on an engine.ts session,
    /// `{"sourceBytes": n}`, `{"clipboardFrom": session | null}` (a copy of
    /// another session's clipboard) and `{"signedSource": null}` (drops the
    /// retained signed source). Anything else changes nothing and is false.
    pub fn session_set(&mut self, session: u32, fields: &str) -> bool {
        let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(fields) else {
            return false;
        };
        let mut source_bytes = None;
        let mut clipboard = None;
        let mut drop_signed = false;
        for (key, value) in &fields {
            match key.as_str() {
                "sourceBytes" => {
                    let bytes = request::as_int(value).and_then(|n| usize::try_from(n).ok());
                    let Some(bytes) = bytes else { return false };
                    source_bytes = Some(bytes);
                }
                "clipboardFrom" if value.is_null() => clipboard = Some(None),
                "clipboardFrom" => {
                    let from = request::as_int(value).and_then(|n| u32::try_from(n).ok());
                    let Some(from) = from.and_then(|from| self.sessions.get(&from)) else {
                        return false;
                    };
                    clipboard = Some(from.clipboard.clone());
                }
                "signedSource" if value.is_null() => drop_signed = true,
                _ => return false,
            }
        }
        let Some(state) = self.sessions.get_mut(&session) else {
            return false;
        };
        if let Some(bytes) = source_bytes {
            state.source_bytes = bytes;
        }
        if let Some(clipboard) = clipboard {
            state.clipboard = clipboard;
        }
        if drop_signed {
            state.signed_source = None;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(app: &mut RustApp, session: u32, request: &str) -> Value {
        serde_json::from_str(&app.dispatch(session, request)).unwrap()
    }

    #[test]
    fn cleared_inputs_never_reach_the_next_request() {
        let mut app = RustApp::new();
        let session = app.create_session();
        app.push_input(vec![0xaa]);
        app.clear_inputs();
        app.push_input(vec![0xbb]);
        assert_eq!(app.inputs, [vec![0xbb]]);
        let close = reply(&mut app, session, r#"{"id":1,"op":"close"}"#);
        assert_eq!(close, serde_json::json!({"id": 1, "ok": true}));
        assert!(app.inputs.is_empty(), "a dispatch takes every queued input");
    }

    #[test]
    fn unparsable_requests_and_unknown_sessions_are_plain_errors() {
        let mut app = RustApp::new();
        let session = app.create_session();
        let invalid = reply(&mut app, session, "{");
        assert_eq!(invalid["ok"], false);
        assert!(invalid.get("id").is_none() && invalid.get("reason").is_none());
        let unknown = reply(&mut app, 99, r#"{"id":2,"op":"close"}"#);
        assert_eq!(
            (&unknown["id"], &unknown["error"]),
            (&2.into(), &"no session 99".into())
        );
        assert_eq!(app.session_state(99), "null");
        assert_eq!(app.session_debug(99), "null");
    }

    #[test]
    fn test_setters_change_only_what_they_name() {
        let mut app = RustApp::new();
        let a = app.create_session();
        let b = app.create_session();
        assert!(app.session_set(a, r#"{"sourceBytes":52428801}"#));
        let state: Value = serde_json::from_str(&app.session_state(a)).unwrap();
        assert_eq!(state["sourceBytes"], 52_428_801);
        app.sessions.get_mut(&a).unwrap().clipboard = Some(Default::default());
        assert!(app.session_set(b, &format!(r#"{{"clipboardFrom":{a}}}"#)));
        assert!(app.sessions[&b].clipboard.is_some());
        assert!(app.session_set(b, r#"{"clipboardFrom":null}"#));
        assert!(app.sessions[&b].clipboard.is_none());
        app.sessions.get_mut(&a).unwrap().signed_source = Some(vec![1]);
        assert!(app.session_set(a, r#"{"signedSource":null}"#));
        assert!(app.sessions[&a].signed_source.is_none());
        for refused in [
            r#"{"sourceBytes":-1}"#,
            r#"{"sourceBytes":1.5}"#,
            r#"{"clipboardFrom":99}"#,
            r#"{"name":"x"}"#,
            r#"{"sourceBytes":1,"other":2}"#,
            r#"{"signedSource":[1]}"#,
            "[]",
            "{",
        ] {
            assert!(!app.session_set(a, refused), "{refused}");
        }
        assert_eq!(app.sessions[&a].source_bytes, 52_428_801);
        assert!(!app.session_set(99, r#"{"sourceBytes":1}"#));
    }
}
