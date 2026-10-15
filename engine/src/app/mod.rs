// SPDX-License-Identifier: AGPL-3.0-or-later
//! The application engine: the worker's `SessionRequest` contract
//! (`src/renderer/engine.ts`) as JSON requests and replies. Binary fields
//! travel as `{"$buffer": n}` placeholders indexing a side list of buffers,
//! in both directions; `request` lists the other wire markers.
//!
//! On wasm32 a panic aborts and poisons the whole `RustApp`, every session
//! with it, so op code must not panic on any input.
pub mod annots;
pub mod document;
pub mod edit;
pub mod forms;
pub mod info;
pub mod journal;
pub mod labels;
pub mod layers;
pub mod ops;
mod overlay;
pub mod pagetree;
pub mod password;
pub mod request;
mod view_depth;
mod view_fixes;

use serde_json::{Map, Value, json};
use std::fmt;
use std::sync::Arc;

pub use document::{AppDocument, LoadError, Transaction};
pub use info::{DocumentInfo, PageState, Permissions};

use crate::view::ViewDocument;

/// engine.ts `MAX_PDF_BYTES`: the open ceiling.
pub const MAX_PDF_BYTES: usize = 120 * 1024 * 1024;
/// engine.ts `MAX_SAVE_BYTES`: the save and validate ceiling.
pub const MAX_SAVE_BYTES: usize = 50 * 1024 * 1024;
/// engine.ts `MAX_PASSWORD_CHARS`, in UTF-16 code units.
pub const MAX_PASSWORD_CHARS: usize = 1024;

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

/// Placeholder for the worker clipboard.
#[derive(Clone, Debug, Default)]
pub struct Clipboard;

/// The view-only compare document.
pub use ops::compare::CompareDocument;

/// What an op needs `/P` to grant (engine-core.ts `OP_PERMISSION` values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    Print,
    Copy,
    Edit,
    Annotate,
    Form,
    /// annotate, edit or form.
    Any,
    /// edit and annotate.
    EditAnnotate,
}

/// engine-core.ts `OP_PERMISSION`; every other op needs `Edit`.
pub const OP_PERMISSION: [(&str, Need); 31] = [
    ("field-set", Need::Form),
    ("text-add", Need::Edit),
    ("field-add", Need::EditAnnotate),
    ("field-props", Need::EditAnnotate),
    ("field-delete", Need::EditAnnotate),
    ("annot-add", Need::Annotate),
    ("annot-set", Need::Annotate),
    ("annot-move", Need::Annotate),
    ("annot-delete", Need::Annotate),
    ("redact-add", Need::Annotate),
    ("redact-search", Need::Annotate),
    ("redact-set", Need::Annotate),
    ("redact-apply", Need::Edit),
    ("link-add", Need::Annotate),
    ("link-delete", Need::Annotate),
    ("link-set", Need::Annotate),
    ("outline-add", Need::Edit),
    ("outline-set", Need::Edit),
    ("outline-delete", Need::Edit),
    ("insert-pages", Need::Edit),
    ("stamp", Need::Edit),
    ("labels-set", Need::Edit),
    ("labels-clear", Need::Edit),
    ("page-rotate", Need::Edit),
    ("page-delete", Need::Edit),
    ("page-move", Need::Edit),
    ("clip-copy", Need::Copy),
    ("clip-paste", Need::Any),
    ("edit-batch", Need::Any),
    ("undo", Need::Any),
    ("redo", Need::Any),
];

/// The permission `op` needs.
pub fn op_permission(op: &str) -> Need {
    OP_PERMISSION
        .iter()
        .find(|(name, _)| *name == op)
        .map_or(Need::Edit, |(_, need)| *need)
}

impl Need {
    /// engine-core.ts `allowed`: an unencrypted document permits everything.
    pub fn allowed(self, info: &DocumentInfo) -> bool {
        let p = info.permissions;
        !info.encrypted
            || match self {
                Self::Print => p.print,
                Self::Copy => p.copy,
                Self::Edit => p.edit,
                Self::Annotate => p.annotate,
                Self::Form => p.form,
                Self::Any => p.annotate || p.edit || p.form,
                Self::EditAnnotate => p.edit && p.annotate,
            }
    }

    /// The `<what>` of requireMutable's refusal.
    fn what(self) -> &'static str {
        match self {
            Self::Any => "changes",
            Self::Form => "filling form fields",
            Self::Annotate => "annotations",
            Self::EditAnnotate => "authoring form fields",
            _ => "changes to its content",
        }
    }
}

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
    pub locked_bytes: Option<Arc<Vec<u8>>>,
    /// Source bytes kept for an incremental second signature.
    pub signed_source: Option<Vec<u8>>,
    pub security: Option<SecuritySetting>,
    /// A trailer-only change the journal cannot see.
    pub security_dirty: bool,
    /// Counts `security-set` requests, so a save ticket knows which setting
    /// its bytes were written with.
    pub security_version: u64,
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

    /// engine.ts `detect(session.doc, session.locked)`.
    pub fn info(&self) -> Option<DocumentInfo> {
        Some(self.doc.as_ref()?.info(self.locked))
    }

    /// engine.ts `pageState`: `{pageCount, pages, rotations}`, empty without
    /// a document.
    pub fn page_state(&self) -> PageState {
        self.doc
            .as_ref()
            .map_or_else(PageState::default, AppDocument::page_state)
    }

    /// engine.ts `editState`: the journal slice and the page state.
    pub fn edit_state(&self) -> Map<String, Value> {
        let mut fields = self.journal_state().json();
        fields.extend(self.page_state().json());
        fields
    }

    /// engine.ts `requireMutable`: the open document, when it is unlocked,
    /// unsigned and its permissions allow `op`.
    pub fn require_mutable(&self, op: &str) -> Result<&AppDocument, AppError> {
        let Some(doc) = &self.doc else {
            return Err(AppError::reason(
                "no-document",
                format!("{op}: no document open"),
            ));
        };
        if self.locked {
            return Err(AppError::reason(
                "read-only",
                format!("{op}: this document is read-only until it is unlocked"),
            ));
        }
        let info = doc.info(self.locked);
        if info.signed() {
            return Err(AppError::reason(
                "read-only",
                format!("{op}: this document is read-only"),
            ));
        }
        let need = op_permission(op);
        if !need.allowed(&info) {
            return Err(AppError::reason(
                "read-only",
                format!("{op}: this document does not permit {}", need.what()),
            ));
        }
        Ok(doc)
    }

    /// engine.ts `checkPage`: an integer page index in range, else a tagged
    /// `invalid` refusal.
    pub fn check_page(&self, page: Option<&Value>, op: &str) -> Result<usize, AppError> {
        let count = self.doc.as_ref().map_or(0, AppDocument::page_count);
        page.and_then(request::as_int)
            .and_then(|page| usize::try_from(page).ok())
            .filter(|&page| page < count)
            .ok_or_else(|| {
                AppError::reason(
                    "invalid",
                    format!(
                        "{op}: page must be an integer in 0..{}",
                        count.saturating_sub(1)
                    ),
                )
            })
    }

    /// engine.ts `mutate`: runs `work` as one journal operation named
    /// `step` (the name `undo` and `redo` report). On success the revision
    /// goes up; on an error everything `work` wrote is restored. In batch
    /// mode `Run`, `work` joins the batch's operation with no step or
    /// revision of its own; in `Plan` it does not run (`Ok(None)`). A call
    /// inside another operation joins it, as MuPDF nests operations.
    ///
    /// `work` gets the session; it writes through `AppSession::tx`.
    pub fn mutate<T>(
        &mut self,
        step: &str,
        work: impl FnOnce(&mut AppSession) -> Result<T, AppError>,
    ) -> Result<Option<T>, AppError> {
        match self.batch {
            Batch::Plan => return Ok(None),
            Batch::Run => return work(self).map(Some),
            Batch::Off => {}
        }
        let Some(doc) = self.doc.as_mut() else {
            return Err(AppError::reason(
                "invalid",
                format!("{step}: the open document is not a PDF"),
            ));
        };
        if doc.journal().is_open() {
            return work(self).map(Some);
        }
        // Both are read before the operation opens, as engine.ts must.
        let branched = doc.can_redo();
        let before = doc.position();
        if !doc.begin(step) {
            return Err(AppError::reason(
                "read-only",
                format!("{step}: this document is read-only until it is unlocked"),
            ));
        }
        let result = work(self);
        let Some(doc) = self.doc.as_mut().filter(|doc| doc.journal().is_open()) else {
            return Err(AppError::plain(format!(
                "{step}: the document was replaced during the operation"
            )));
        };
        let outcome = match result {
            Ok(value) => {
                doc.end();
                Ok(Some(value))
            }
            Err(error) => match doc.abandon() {
                Ok(()) => Err(error),
                Err(restore) => Err(AppError::plain(format!("{step}: {restore}"))),
            },
        };
        // A write after an undo cut the redo branch; if the saved state was
        // on it, the same position no longer means the same bytes.
        if branched && !doc.can_redo() && self.saved_position > before {
            self.saved_position = -1;
        }
        if outcome.is_ok() {
            self.revision += 1;
        }
        outcome
    }

    /// Write access during `mutate`'s work; a plain error outside it.
    pub fn tx(&mut self) -> Result<Transaction<'_>, AppError> {
        self.doc
            .as_mut()
            .and_then(AppDocument::transaction)
            .ok_or_else(|| AppError::plain("no journal operation is open"))
    }

    /// The current document as hayro draws it, cached until it changes.
    /// Plain errors; view ops check for a document and a lock first with
    /// their own messages.
    pub fn view(&mut self) -> Result<&ViewDocument, AppError> {
        if self.locked {
            return Err(AppError::plain(
                "document is encrypted, a password is required",
            ));
        }
        let doc = (self.doc.as_mut()).ok_or_else(|| AppError::plain("no document open"))?;
        doc.view()
            .map_err(|error| AppError::plain(error.to_string()))
    }

    /// The security the next save writes: a pending `none` or `aes-256`,
    /// else (`keep`, or nothing pending) what the last acknowledged save
    /// wrote. `None` means the source's own.
    pub fn effective_security(&self) -> Option<&SecuritySetting> {
        match &self.security {
            Some(setting @ (SecuritySetting::None | SecuritySetting::Aes256 { .. })) => {
                Some(setting)
            }
            _ => self.doc.as_ref()?.written_security(),
        }
    }

    /// engine.ts `securityState`: never a password. `keep` reports what the
    /// file on disk has (`effective_security`).
    pub fn security_state(&self) -> Value {
        let auth = self.auth.as_str();
        let pending = self.security_dirty;
        let mode = match &self.security {
            Some(SecuritySetting::Aes256 { .. }) => "aes-256",
            Some(SecuritySetting::None) => "none",
            _ => "keep",
        };
        let (algorithm, names) = match self.effective_security() {
            Some(SecuritySetting::Aes256 { permissions, .. }) => {
                ("aes-256", Permissions::from_bits(*permissions).names())
            }
            Some(SecuritySetting::None) => ("none", Permissions::ALL.names()),
            Some(SecuritySetting::Keep) | None => match self.info() {
                Some(info) => (info.algorithm, info.permissions.names()),
                None => ("none", Vec::new()),
            },
        };
        json!({"mode": mode, "algorithm": algorithm, "permissions": names, "auth": auth,
            "pending": pending})
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

    /// The current document bytes, for inspection by an independent reader:
    /// the source while locked, otherwise `AppDocument::snapshot`.
    pub fn snapshot_bytes(&self) -> Option<Vec<u8>> {
        let doc = self.doc.as_ref()?;
        match &self.locked_bytes {
            Some(bytes) if self.locked => Some(bytes.to_vec()),
            _ => doc.snapshot().ok(),
        }
    }

    /// Test inspection: `state_json` plus the fields tests read directly on an
    /// engine.ts session, passwords included. The adapter never sends it.
    pub fn debug_json(&self) -> Value {
        let security = self.security.as_ref().map(|setting| match setting {
            SecuritySetting::Keep => json!({"mode": "keep"}),
            SecuritySetting::None => json!({"mode": "none"}),
            SecuritySetting::Aes256 {
                user_password,
                owner_password,
                permissions,
            } => json!({"mode": "aes-256", "userPassword": user_password,
                "ownerPassword": owner_password, "permissions": permissions}),
        });
        let batch = match self.batch {
            Batch::Off => json!(false),
            Batch::Run => json!(true),
            Batch::Plan => json!("plan"),
        };
        let journal = (self.doc.as_ref())
            .map(|doc| json!({"position": doc.position(), "steps": doc.steps()}));
        let extra = json!({
            "password": self.password,
            "security": security,
            "securityDirty": self.security_dirty,
            "lockedBytes": self.locked_bytes.as_ref().map(|bytes| bytes.len()),
            "signedSource": self.signed_source.as_ref().map(Vec::len),
            "batch": batch,
            "clipboard": self.clipboard.is_some(),
            "compare": self.compare.as_ref().map(CompareDocument::debug_json),
            "journal": journal,
        });
        let mut state = self.state_json();
        if let (Value::Object(fields), Value::Object(extra)) = (&mut state, extra) {
            fields.extend(extra);
        }
        state
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

/// A plain error with the document error's text; ops that owe a specific
/// message map the error themselves.
impl From<crate::document::DocumentError> for AppError {
    fn from(error: crate::document::DocumentError) -> Self {
        Self::plain(error.to_string())
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

    /// The input a placeholder names, still in place.
    pub fn input(&self, value: Option<&Value>) -> Option<&[u8]> {
        let index = request::buffer_index(value?)?;
        self.inputs.get(index)?.as_deref()
    }

    /// `{"$input": n}`: the reply hands the request's own buffer back, as
    /// engine.ts `validate` does; the adapter answers with the caller's
    /// original `ArrayBuffer`. Valid after `take` too.
    pub fn echo(&self, value: Option<&Value>) -> Option<Value> {
        let index = request::buffer_index(value?)?;
        (index < self.inputs.len()).then(|| json!({ "$input": index }))
    }

    /// Stores an output buffer and returns its placeholder.
    pub fn put(&mut self, bytes: Vec<u8>) -> Value {
        self.outputs.push(bytes);
        json!({ "$buffer": self.outputs.len() - 1 })
    }
}
