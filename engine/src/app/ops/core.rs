// SPDX-License-Identifier: AGPL-3.0-or-later
//! Document lifecycle and session ops: open, authenticate, close, undo,
//! redo, saved, security-set, save, validate and page-ids (engine.ts).
//!
//! Timing fields (`openMs`, `saveMs`) are 0 here: `std::time::Instant`
//! panics on wasm32-unknown-unknown, so the TypeScript adapter measures the
//! dispatch and fills them in.
use std::sync::Arc;

use serde_json::{Map, Value, json};

use crate::app::document::{limits, open_options, parse_limit};
use crate::app::info::{self, DocumentInfo, PERMISSION_BITS, PageState, Permissions};
use crate::app::save_tree;
use crate::app::{
    AppDocument, AppEngine, AppError, AppSession, Auth, Buffers, MAX_PASSWORD_CHARS, MAX_PDF_BYTES,
    MAX_SAVE_BYTES, SecuritySetting, overlay, password, reply, request,
};
use crate::document::{Document, DocumentErrorKind};
use crate::serialize::SerializeError;
use crate::writer::{DocumentWriteError, Encryption, Entropy, WriteOptions, write_document};

fn plain(message: impl Into<String>) -> AppError {
    AppError::plain(message)
}

fn tagged(reason: &str, message: impl Into<String>) -> AppError {
    AppError::reason(reason, message)
}

/// JavaScript `string.length`.
fn js_length(text: &str) -> usize {
    text.encode_utf16().count()
}

/// engine.ts `open`: a failed open leaves the previous document in place.
pub fn open(
    engine: &mut AppEngine,
    session: &mut AppSession,
    request: &Value,
    buffers: &mut Buffers,
) -> Result<Value, AppError> {
    let name = match request::string(request, "name") {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => return Err(plain("open: missing document name")),
    };
    let field = request.get("pdf");
    let bytes = buffers.take(field);
    if bytes.is_none() && !request::truthy(field) {
        return Err(plain("open: no bytes"));
    }
    // Compare §9.3: the pair never exceeds MAX_PDF_BYTES; the compare slot goes first.
    let mut compare_closed = false;
    if let (Some(bytes), Some(compare)) = (&bytes, &session.compare) {
        if bytes.len().saturating_add(compare.source_bytes()) > MAX_PDF_BYTES {
            session.compare = None;
            compare_closed = true;
        }
    }
    let Opened {
        doc,
        source,
        info,
        pages,
    } = open_document(bytes).map_err(|error| match compare_closed {
        true => error.with_extra(json!({"compareClosed": true})),
        false => error,
    })?;
    session.name = Some(name);
    session.source_bytes = source.len();
    session.locked = info.needs_password;
    session.signed_source =
        (info.signed() && source.len() <= MAX_SAVE_BYTES).then(|| source.to_vec());
    session.locked_bytes = info.needs_password.then_some(source);
    session.auth = Auth::None;
    session.password = None;
    session.security = None;
    session.security_dirty = false;
    session.revision = 0;
    session.generation = engine.next_generation();
    session.saved_position = doc.position();
    session.doc = Some(doc);
    let mut fields = pages.json();
    fields.insert("info".into(), info.json());
    fields.insert("openMs".into(), 0.into());
    fields.insert("revision".into(), 0.into());
    fields.insert("auth".into(), Auth::None.as_str().into());
    if compare_closed {
        fields.insert("compareClosed".into(), true.into());
    }
    Ok(reply(request, Value::Object(fields)))
}

struct Opened {
    doc: AppDocument,
    source: Arc<Vec<u8>>,
    info: DocumentInfo,
    pages: PageState,
}

/// engine.ts `openDocument` and the checks `open` makes on the result.
fn open_document(bytes: Option<Vec<u8>>) -> Result<Opened, AppError> {
    let Some(bytes) = bytes else {
        return Err(plain("open: expected an ArrayBuffer"));
    };
    if bytes.len() < 5 {
        return Err(plain("open: not a PDF (too few bytes)"));
    }
    if bytes.len() > MAX_PDF_BYTES {
        return Err(plain(format!(
            "open: PDF exceeds the {MAX_PDF_BYTES} byte ceiling"
        )));
    }
    let source = Arc::new(bytes);
    let doc = AppDocument::load(source.clone(), b"")
        .map_err(|error| plain(format!("open: the file could not be read ({error})")))?;
    let info = doc.info(doc.is_locked());
    let pages = doc.page_state();
    if pages.count() < 1 && !info.needs_password {
        return Err(plain("open: PDF has no pages"));
    }
    Ok(Opened {
        doc,
        source,
        info,
        pages,
    })
}

/// engine.ts `authenticate`: a wrong password is an answer, not an error.
/// The document is reopened from its source with the password; the
/// generation and revision stay.
pub fn authenticate(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let Some(doc) = &session.doc else {
        return Err(plain("authenticate: no document open"));
    };
    if !session.locked {
        return Err(plain("authenticate: document is not locked"));
    }
    let password = match request::string(request, "password") {
        Some(password) if js_length(password) <= MAX_PASSWORD_CHARS => password.to_string(),
        _ => {
            return Err(plain(format!(
                "authenticate: password must be a string of at most {MAX_PASSWORD_CHARS} characters"
            )));
        }
    };
    let source = (session.locked_bytes.clone()).unwrap_or_else(|| doc.input().clone());
    let candidates = password::candidates(&password, password::revision(doc.document()));
    let candidates: Vec<&[u8]> = candidates.iter().map(Vec::as_slice).collect();
    // A wrong password for a file the strict reader opened is known without
    // reading the file again: the reload would ask for a password too.
    if doc.opened_strictly() && password::all_wrong(doc.document(), &candidates) {
        return Ok(reply(request, json!({"authenticated": false})));
    }
    let fresh = AppDocument::load_any(source, &candidates)
        .map_err(|_| plain("authenticate: the password did not unlock a fresh copy"))?;
    if fresh.is_locked() {
        return Ok(reply(request, json!({"authenticated": false})));
    }
    let owner = fresh.document().security().is_some_and(|s| s.owner);
    let mut fields = Map::new();
    fields.insert("authenticated".into(), true.into());
    fields.extend(fresh.page_state().json());
    fields.insert("info".into(), fresh.info(false).json());
    session.saved_position = fresh.position();
    session.doc = Some(fresh);
    session.locked_bytes = None;
    session.locked = false;
    session.auth = if owner { Auth::Owner } else { Auth::User };
    session.password = Some(password);
    fields.insert("auth".into(), session.auth.as_str().into());
    Ok(reply(request, Value::Object(fields)))
}

/// engine.ts `close`: never fails, allocates a new generation and keeps the
/// clipboard, compare slot and batch mode.
pub fn close(engine: &mut AppEngine, session: &mut AppSession, request: &Value) -> Value {
    let AppSession {
        clipboard,
        compare,
        batch,
        ..
    } = std::mem::take(session);
    *session = AppSession {
        clipboard,
        compare,
        batch,
        generation: engine.next_generation(),
        ..AppSession::default()
    };
    reply(request, json!({}))
}

/// engine.ts `undo` / `redo`: the reply names the step moved over.
pub fn undo_redo(session: &mut AppSession, request: &Value, op: &str) -> Result<Value, AppError> {
    session.require_mutable(op)?;
    let Some(doc) = session.doc.as_mut() else {
        return Err(tagged("no-document", format!("{op}: no document open")));
    };
    let moved = if op == "undo" { doc.undo() } else { doc.redo() };
    let step = moved
        .map_err(|error| plain(format!("{op}: {error}")))?
        .ok_or_else(|| plain(format!("{op}: there is nothing to {op}")))?;
    session.revision += 1;
    let mut fields = session.edit_state();
    if !step.is_empty() {
        fields.insert("step".into(), step.into());
    }
    Ok(reply(request, Value::Object(fields)))
}

/// A `save` reply's ticket: the generation, the journal state and the
/// security setting the bytes were written from.
fn ticket(session: &AppSession, doc: &AppDocument) -> String {
    format!(
        "{}:{}:{}",
        session.generation,
        doc.journal().state_serial(),
        session.security_version
    )
}

/// The fields of a ticket this engine issued.
fn parse_ticket(ticket: Option<&Value>) -> Option<[u64; 3]> {
    let mut parts = ticket?.as_str()?.split(':').map(|part| part.parse().ok());
    let fields = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some(fields)
}

/// engine.ts `saved`: the write landed. With the `ticket` of the save that
/// wrote it, the saved state is the one that save serialized, so a late
/// acknowledgement cannot mark newer edits clean; without one, the current
/// state, as engine.ts does.
pub fn saved(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let Some(doc) = &session.doc else {
        return Err(tagged("no-document", "saved: no document open"));
    };
    let ticket = request.get("ticket");
    let (position, security) = if request::nullish(ticket) {
        (doc.position(), true)
    } else {
        let Some([generation, state, security]) = parse_ticket(ticket) else {
            return Err(tagged(
                "invalid",
                "saved: the save ticket is not one this engine issued",
            ));
        };
        if generation != session.generation {
            return Err(tagged(
                "invalid",
                "saved: the save ticket belongs to another document",
            ));
        }
        let position = doc.journal().position_of(state);
        (
            position.map_or(-1, |p| i64::try_from(p).unwrap_or(-1)),
            security == session.security_version,
        )
    };
    session.saved_position = position;
    if security {
        session.security_dirty = false;
        if session.security == Some(SecuritySetting::None) {
            session.password = None;
        }
        if let (
            Some(setting @ (SecuritySetting::None | SecuritySetting::Aes256 { .. })),
            Some(doc),
        ) = (&session.security, session.doc.as_mut())
        {
            doc.set_written_security(setting.clone());
        }
    }
    Ok(reply(
        request,
        Value::Object(session.journal_state().json()),
    ))
}

/// engine.ts `securitySetting`: the request as a pending setting.
fn security_setting(request: &Value) -> Result<SecuritySetting, AppError> {
    match request::string(request, "mode") {
        Some("keep") => return Ok(SecuritySetting::Keep),
        Some("none") => return Ok(SecuritySetting::None),
        Some("aes-256") => {}
        _ => {
            return Err(plain(
                "security-set: mode must be one of keep, none, aes-256",
            ));
        }
    }
    let check = |value: Option<&Value>, label: &str| -> Result<String, AppError> {
        let Some(text) = value.and_then(Value::as_str) else {
            return Err(plain(format!("security-set: {label} must be a string")));
        };
        if js_length(text) > MAX_PASSWORD_CHARS {
            return Err(plain(format!(
                "security-set: {label} must be at most {MAX_PASSWORD_CHARS} characters"
            )));
        }
        if text.contains(',') {
            return Err(plain(format!(
                "security-set: {label} cannot contain a comma"
            )));
        }
        // Readers (MuPDF's, and `password::candidates`) end a password at
        // its first NUL; the writer hashes every byte, so the saved copy
        // would never open with it.
        if text.contains('\0') {
            return Err(plain(format!(
                "security-set: {label} cannot contain a NUL character"
            )));
        }
        Ok(text.to_string())
    };
    let user_password = check(request.get("userPassword"), "user password")?;
    if user_password.is_empty() {
        return Err(plain(
            "security-set: AES-256 needs a non-empty user password",
        ));
    }
    let owner_password = match request.get("ownerPassword") {
        None => user_password.clone(),
        Some(Value::String(text)) if text.is_empty() => user_password.clone(),
        owner => check(owner, "owner password")?,
    };
    // `value?.[name]`: only an object's own keys are flags.
    let flags = request.get("permissions").and_then(Value::as_object);
    let flag = |name: &str| match flags.and_then(|flags| flags.get(name)) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(plain(format!("security-set: {name} must be true or false"))),
    };
    let permissions = Permissions {
        print: flag("print")?,
        copy: flag("copy")?,
        edit: flag("edit")?,
        annotate: flag("annotate")?,
        form: flag("form")?,
    };
    Ok(SecuritySetting::Aes256 {
        user_password,
        owner_password,
        permissions: permissions.bits(),
    })
}

/// engine.ts `security-set`: a pending change, written by the next save.
pub fn security_set(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let Some(info) = session.info() else {
        return Err(plain("security-set: no document open"));
    };
    if session.locked {
        return Err(plain("security-set: the document is locked"));
    }
    if info.signed() {
        return Err(plain(
            "security-set: signed or certified documents keep their security",
        ));
    }
    if info.encrypted && session.auth != Auth::Owner {
        return Err(plain(
            "security-set: open the document with its owner password to change security",
        ));
    }
    session.security = Some(security_setting(request)?);
    session.security_dirty = true;
    session.security_version += 1;
    session.revision += 1;
    let mut fields = session.journal_state().json();
    fields.insert("security".into(), session.security_state());
    Ok(reply(request, Value::Object(fields)))
}

/// How the next save encrypts: the one place a save chooses encryption.
/// `keep` keeps what the file on disk has (`effective_security`): the
/// source's own handler, or plaintext for a plain document.
pub fn save_encryption(session: &AppSession) -> Encryption<'_> {
    match session.effective_security() {
        Some(SecuritySetting::None) => Encryption::Plaintext,
        Some(SecuritySetting::Aes256 {
            user_password,
            owner_password,
            permissions,
        }) => Encryption::Aes256 {
            user_password: user_password.as_bytes(),
            owner_password: owner_password.as_bytes(),
            permissions: *permissions,
        },
        _ => Encryption::Keep,
    }
}

fn too_large_copy() -> AppError {
    tagged(
        "too-large",
        format!("save: the copy would exceed the {MAX_SAVE_BYTES} byte save ceiling"),
    )
}

/// engine.ts `save` (`serializeDocument`): a full rewrite of the reachable
/// objects without renumbering. Edit permissions are not checked. The reply
/// adds a `ticket` for `saved`.
pub fn save(
    session: &mut AppSession,
    request: &Value,
    buffers: &mut Buffers,
) -> Result<Value, AppError> {
    let Some(doc) = &session.doc else {
        return Err(tagged("no-document", "save: no document open"));
    };
    if session.source_bytes > MAX_SAVE_BYTES {
        return Err(tagged(
            "too-large",
            format!(
                "save: {} bytes is over the {MAX_SAVE_BYTES} byte save ceiling",
                session.source_bytes
            ),
        ));
    }
    if session.locked {
        return Err(tagged(
            "encrypted",
            "save: this document is encrypted; enter its password before saving",
        ));
    }
    if doc.info(session.locked).signed() {
        return Err(tagged(
            "read-only",
            "save: signed or certified documents are read-only",
        ));
    }
    let mut limits = overlay::limits();
    limits.syntax.max_output_bytes = MAX_SAVE_BYTES;
    // 32 fresh random bytes from the adapter (crypto.getRandomValues): the
    // file key, IVs and a new /ID[1]. Without them encrypted output is refused.
    let entropy = (buffers.input(request.get("entropy")))
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .map(Entropy);
    let options = WriteOptions {
        encryption: save_encryption(session),
        entropy,
        ..WriteOptions::APP
    };
    // A repaired or fallback page list is written as a tree MuPDF reads
    // the same way (`save_tree`); an intact tree is written as it is.
    let written = save_tree::for_save(doc.document())
        .map_err(|error| tagged("invalid", format!("save: {error}")))?;
    let bytes = match write_document(&written, limits, options) {
        Ok(bytes) if bytes.len() <= MAX_SAVE_BYTES => bytes,
        Ok(_) | Err(DocumentWriteError::Serialize(SerializeError::OutputLimit)) => {
            return Err(too_large_copy());
        }
        Err(error) => return Err(tagged("invalid", format!("save: {error}"))),
    };
    let ticket = ticket(session, doc);
    let state = session.journal_state();
    let mut fields = Map::new();
    fields.insert("bytes".into(), buffers.put(bytes));
    fields.insert("saveMs".into(), 0.into());
    fields.insert("dirty".into(), state.dirty.into());
    fields.insert("revision".into(), state.revision.into());
    fields.insert("security".into(), session.security_state());
    fields.insert("ticket".into(), ticket.into());
    Ok(reply(request, Value::Object(fields)))
}

/// The pages and declared features `validate` compares.
fn same_shape(before: &DocumentInfo, after: &DocumentInfo, encrypted: bool) -> bool {
    after.encrypted == encrypted
        && after.signatures == before.signatures
        && after.certified == before.certified
        && after.xfa == before.xfa
        && after.attachments == before.attachments
        && after.open_action == before.open_action
        && after.tagged == before.tagged
}

/// Why `validate`'s strict reopen of a saved copy failed: a size or count
/// limit the copy passes (an edit pushed the document past the values or
/// xref-entry budget the engine opens; nothing is damaged), else damage.
/// Both are `invalid`, as engine.ts tags a copy that does not reopen.
fn reopen_refusal(kind: &DocumentErrorKind) -> AppError {
    let limit = match kind {
        DocumentErrorKind::LimitExceeded(limit) => Some(*limit),
        kind => parse_limit(kind),
    };
    match limit {
        Some(limit) => tagged(
            "invalid",
            format!(
                "validate: the saved copy passes the engine's {limit} limit, so it would not reopen"
            ),
        ),
        None => tagged("invalid", "validate: the saved copy needed repair"),
    }
}

/// engine.ts `validate` (`validateDocument`): the saved bytes reopened
/// strictly by the Rust parser with the password they were written with,
/// then compared with the session document and the security the save wrote
/// (`effective_security`). The caller's own buffer is handed back.
pub fn validate(
    session: &mut AppSession,
    request: &Value,
    buffers: &mut Buffers,
) -> Result<Value, AppError> {
    let field = request.get("pdf");
    let input = buffers.input(field);
    if input.is_none() && !request::truthy(field) {
        return Err(tagged("no-document", "validate: no bytes"));
    }
    let Some(doc) = &session.doc else {
        return Err(tagged("no-document", "validate: no document open"));
    };
    let Some(bytes) = input else {
        return Err(tagged("invalid", "validate: expected an ArrayBuffer"));
    };
    if bytes.len() < 5 {
        return Err(tagged("invalid", "validate: not a PDF (too few bytes)"));
    }
    if bytes.len() > MAX_SAVE_BYTES {
        return Err(tagged(
            "too-large",
            format!("validate: PDF exceeds the {MAX_SAVE_BYTES} byte save ceiling"),
        ));
    }
    let wrong_password = || {
        tagged(
            "invalid",
            "validate: the saved copy did not open with its own password",
        )
    };
    let setting = session.effective_security();
    let check = match Document::open_with(bytes, limits(), b"", open_options()) {
        Ok(check) => check,
        Err(error) if matches!(error.kind, DocumentErrorKind::PasswordRequired) => {
            let (password, revision) = match setting {
                Some(SecuritySetting::Aes256 { user_password, .. }) => {
                    (Some(user_password.as_str()), Some(6))
                }
                _ => (
                    session.password.as_deref(),
                    doc.document().security().map(|security| security.revision),
                ),
            };
            let Some(password) = password.filter(|password| !password.is_empty()) else {
                return Err(wrong_password());
            };
            let mut opened = None;
            for candidate in password::candidates(password, revision) {
                match Document::open_with(bytes, limits(), &candidate, open_options()) {
                    Ok(check) => {
                        opened = Some(check);
                        break;
                    }
                    Err(error) if matches!(error.kind, DocumentErrorKind::PasswordRequired) => {}
                    Err(error) => return Err(reopen_refusal(&error.kind)),
                }
            }
            opened.ok_or_else(wrong_password)?
        }
        Err(error) => return Err(reopen_refusal(&error.kind)),
    };
    let page_count = info::pages(&check).len();
    let expected = doc.page_count();
    if page_count != expected {
        return Err(tagged(
            "invalid",
            format!("validate: the saved copy has {page_count} pages, expected {expected}"),
        ));
    }
    let before = doc.info(session.locked);
    let after = info::detect(&check, false, false);
    // `keep` writes the handler the session authenticated; an /Encrypt
    // that names none (not a dictionary) is not written.
    let encrypted = match setting {
        Some(SecuritySetting::None) => false,
        Some(SecuritySetting::Aes256 { .. }) => true,
        _ => before.encrypted && doc.document().security().is_some(),
    };
    if !same_shape(&before, &after, encrypted) {
        return Err(tagged(
            "invalid",
            "validate: the saved copy changed the document's declared features",
        ));
    }
    match setting {
        Some(SecuritySetting::Aes256 { permissions, .. }) => {
            let bits = info::permission_bits(&check);
            for (name, bit) in PERMISSION_BITS {
                let written = bits.is_some_and(|bits| bits & bit == bit);
                if written != (permissions & bit == bit) {
                    return Err(tagged(
                        "invalid",
                        format!(
                            "validate: the saved copy's {name} permission does not match what was requested"
                        ),
                    ));
                }
            }
        }
        Some(SecuritySetting::None) => {
            if after.permissions != Permissions::ALL {
                return Err(tagged(
                    "invalid",
                    "validate: the saved copy is still restricted after encryption was removed",
                ));
            }
        }
        _ => {
            if after.permissions != before.permissions {
                return Err(tagged(
                    "invalid",
                    "validate: the saved copy has different permissions than the source",
                ));
            }
        }
    }
    let state = session.journal_state();
    let mut fields = Map::new();
    fields.insert("pageCount".into(), page_count.into());
    fields.insert("bytes".into(), buffers.echo(field).unwrap_or(Value::Null));
    fields.insert("dirty".into(), state.dirty.into());
    fields.insert("revision".into(), state.revision.into());
    Ok(reply(request, Value::Object(fields)))
}

/// engine.ts `page-ids`: each page's object number, -1 for a page that is
/// not an indirect object; `[]` without a document. No refusals.
pub fn page_ids(session: &AppSession, request: &Value) -> Value {
    let ids: Vec<i64> = session.doc.as_ref().map_or_else(Vec::new, |doc| {
        doc.page_ids()
            .into_iter()
            .map(|id| id.map_or(-1, |id| i64::from(id.number())))
            .collect()
    });
    reply(request, json!({ "ids": ids }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pdf(values: usize) -> Vec<u8> {
        let array = vec!["1"; values].join(" ");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R /Big 4 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".into(),
            format!("[{array}]"),
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        let trailer = format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n");
        bytes.extend_from_slice(trailer.as_bytes());
        bytes
    }

    /// Round 3 (design/app-accept-journal.md): a saved copy past a parse
    /// budget is reported as that limit, not as a copy that needed repair;
    /// damage still is. Small limits stand in for the 8 million values.
    #[test]
    fn a_saved_copy_past_a_budget_is_refused_as_the_limit() {
        let mut small = limits();
        small.parse.max_objects = 30;
        let refusal = |bytes: &[u8]| {
            let error = Document::open_with(bytes, small, b"", open_options()).err();
            let error = error.expect("the strict open fails");
            let refusal = reopen_refusal(&error.kind);
            (refusal.reason.expect("a tagged refusal"), refusal.message)
        };
        assert!(Document::open_with(&pdf(10), small, b"", open_options()).is_ok());
        let (reason, message) = refusal(&pdf(40));
        assert_eq!(reason, "invalid");
        assert_eq!(
            message,
            "validate: the saved copy passes the engine's values limit, so it would not reopen"
        );
        let mut entries = limits();
        entries.max_xref_entries = 3;
        let error = Document::open_with(&pdf(10), entries, b"", open_options()).err();
        let message = reopen_refusal(&error.expect("refused").kind).message;
        assert!(message.contains("the engine's"), "{message}");
        let mut damaged = pdf(10);
        let at = damaged.windows(7).position(|w| w == b"endobj\n").unwrap();
        damaged[at..at + 6].copy_from_slice(b"xxxxxx");
        let (reason, message) = refusal(&damaged);
        assert_eq!(
            (reason.as_str(), message.as_str()),
            ("invalid", "validate: the saved copy needed repair")
        );
    }
}
