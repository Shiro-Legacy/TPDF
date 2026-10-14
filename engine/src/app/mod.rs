// SPDX-License-Identifier: AGPL-3.0-or-later
//! The application engine: the worker's `SessionRequest` contract
//! (`src/renderer/engine.ts`) as JSON requests and replies. Binary fields
//! travel as `{"$buffer": n}` placeholders indexing a side list of buffers,
//! in both directions.
pub mod ops;
pub mod request;

use serde_json::{Map, Value, json};
use std::fmt;

/// Engine-wide state shared by every session.
#[derive(Debug, Default)]
pub struct AppEngine {
    generation: u64,
}

impl AppEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocates the generation of a session that opened or closed a document.
    pub fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    /// Turns one request into one reply and its output buffers. Never fails:
    /// an error becomes an `ok: false` reply with the session's journal state,
    /// and any outputs the failed op produced are dropped.
    pub fn dispatch(
        &mut self,
        session: &mut AppSession,
        request: Value,
        buffers: Vec<Vec<u8>>,
    ) -> (Value, Vec<Vec<u8>>) {
        let mut buffers = Buffers::new(buffers);
        match ops::route(self, session, &request, &mut buffers) {
            Ok(reply) => (reply, buffers.outputs),
            Err(error) => (error_reply(&request, error, session), Vec::new()),
        }
    }
}

/// How the user proved access to the document; `Owner` implies every permission.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Auth {
    #[default]
    None,
    User,
    Owner,
}

impl Auth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::User => "user",
            Self::Owner => "owner",
        }
    }
}

/// engine.ts `Session.batch`: `false`, `true` (run) or `"plan"`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Batch {
    #[default]
    Off,
    Run,
    Plan,
}

/// A security change waiting for the next save; not journaled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecuritySetting {
    Keep,
    None,
    Aes256 {
        user_password: String,
        owner_password: String,
        /// Granted permission bits (print 4, edit 8, copy 16, annotate 32, form 256).
        permissions: u32,
    },
}

/// The open document. Placeholder until the journaled document lands; its
/// methods are the interface the session needs from it.
#[derive(Debug, Default)]
pub struct AppDocument;

impl AppDocument {
    pub fn position(&self) -> i64 {
        0
    }
    pub fn can_undo(&self) -> bool {
        false
    }
    pub fn can_redo(&self) -> bool {
        false
    }
    /// Current bytes without save side effects and without renumbering.
    pub fn snapshot(&self) -> Option<Vec<u8>> {
        None
    }
}

/// Placeholder for the worker clipboard.
#[derive(Debug, Default)]
pub struct Clipboard;

/// Placeholder for the view-only compare document.
#[derive(Debug, Default)]
pub struct CompareDocument;

/// One worker session, mirroring engine.ts `Session`.
#[derive(Debug, Default)]
pub struct AppSession {
    pub doc: Option<AppDocument>,
    pub name: Option<String>,
    pub source_bytes: usize,
    /// Still needs a password; recorded, never re-derived from the document.
    pub locked: bool,
    pub auth: Auth,
    pub password: Option<String>,
    /// The source bytes, kept only while the document is locked.
    pub locked_bytes: Option<Vec<u8>>,
    /// Source bytes kept for an incremental second signature.
    pub signed_source: Option<Vec<u8>>,
    pub security: Option<SecuritySetting>,
    /// A trailer-only change the journal cannot see.
    pub security_dirty: bool,
    /// Journal position matching the bytes on disk; -1 means unreachable.
    pub saved_position: i64,
    pub revision: u64,
    pub generation: u64,
    pub clipboard: Option<Clipboard>,
    pub compare: Option<CompareDocument>,
    pub batch: Batch,
}

/// The `{revision, dirty, canUndo, canRedo}` slice replies carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalState {
    pub revision: u64,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

impl JournalState {
    pub fn json(self) -> Map<String, Value> {
        let mut fields = Map::new();
        fields.insert("revision".into(), self.revision.into());
        fields.insert("dirty".into(), self.dirty.into());
        fields.insert("canUndo".into(), self.can_undo.into());
        fields.insert("canRedo".into(), self.can_redo.into());
        fields
    }
}

impl AppSession {
    /// engine.ts `journalState`: everything false without a document.
    pub fn journal_state(&self) -> JournalState {
        let doc = self.doc.as_ref();
        JournalState {
            revision: self.revision,
            dirty: doc
                .is_some_and(|doc| doc.position() != self.saved_position || self.security_dirty),
            can_undo: doc.is_some_and(AppDocument::can_undo),
            can_redo: doc.is_some_and(AppDocument::can_redo),
        }
    }

    /// The fields the TypeScript adapter exposes as session getters.
    pub fn state_json(&self) -> Value {
        let mut state = json!({
            "generation": self.generation,
            "savedPosition": self.saved_position,
            "locked": self.locked,
            "auth": self.auth.as_str(),
            "name": self.name,
            "sourceBytes": self.source_bytes,
        });
        if let Value::Object(fields) = &mut state {
            fields.extend(self.journal_state().json());
        }
        state
    }

    /// The current document bytes, for inspection by an independent reader.
    pub fn snapshot_bytes(&self) -> Option<Vec<u8>> {
        self.doc.as_ref()?.snapshot()
    }
}

/// A refusal. Plain errors carry no `reason`; tagged ones do, which is how
/// the renderer tells a refusal token from a failure.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppError {
    pub message: String,
    pub reason: Option<String>,
    pub detail: Option<String>,
    pub extra: Map<String, Value>,
}

impl AppError {
    pub fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ..Self::default()
        }
    }

    /// engine-core.ts `failReason`.
    pub fn reason(reason: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            reason: Some(reason.into()),
            ..Self::plain(message)
        }
    }

    pub fn detail(
        reason: impl Into<String>,
        detail: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            detail: Some(detail.into()),
            ..Self::reason(reason, message)
        }
    }

    /// Adds reply fields; a non-object adds nothing, as a JS spread would.
    pub fn with_extra(mut self, extra: Value) -> Self {
        if let Value::Object(fields) = extra {
            self.extra.extend(fields);
        }
        self
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

/// The engine.ts dispatch catch: `{id, ok: false, error, reason?, detail?,
/// ...extra, ...journalState}`, later fields overriding earlier ones.
pub fn error_reply(request: &Value, error: AppError, session: &AppSession) -> Value {
    let mut reply = Map::new();
    if let Some(id) = request.get("id") {
        reply.insert("id".into(), id.clone());
    }
    reply.insert("ok".into(), false.into());
    reply.insert("error".into(), error.message.into());
    for (key, value) in [("reason", error.reason), ("detail", error.detail)] {
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            reply.insert(key.into(), value.into());
        }
    }
    reply.extend(error.extra);
    reply.extend(session.journal_state().json());
    Value::Object(reply)
}

/// A successful reply: `{id, ok: true, ...fields}`.
pub fn reply(request: &Value, fields: Value) -> Value {
    let mut reply = Map::new();
    if let Some(id) = request.get("id") {
        reply.insert("id".into(), id.clone());
    }
    reply.insert("ok".into(), true.into());
    if let Value::Object(fields) = fields {
        reply.extend(fields);
    }
    Value::Object(reply)
}

/// The input and output buffers of one dispatch.
#[derive(Debug, Default)]
pub struct Buffers {
    inputs: Vec<Option<Vec<u8>>>,
    outputs: Vec<Vec<u8>>,
}

impl Buffers {
    pub fn new(inputs: Vec<Vec<u8>>) -> Self {
        Self {
            inputs: inputs.into_iter().map(Some).collect(),
            outputs: Vec::new(),
        }
    }

    /// The input a placeholder names, taken at most once. `None` for anything
    /// else, which is where engine.ts refuses a non-`ArrayBuffer`.
    pub fn take(&mut self, value: Option<&Value>) -> Option<Vec<u8>> {
        let index = request::buffer_index(value?)?;
        self.inputs.get_mut(index)?.take()
    }

    /// Stores an output buffer and returns its placeholder.
    pub fn put(&mut self, bytes: Vec<u8>) -> Value {
        self.outputs.push(bytes);
        json!({ "$buffer": self.outputs.len() - 1 })
    }
}
