//! Security (renderer.ts R:10590-10770): the passwords and permissions dialog, the pending
//! `security-set`, the smoke's encrypt / encrypt-keep / password paths. The dialog mirrors the
//! engine's security state and never keeps a password: the two fields are cleared once the
//! change is accepted and on close (a refusal keeps them to correct), and only the request
//! carries them.

use super::Session;
use super::dom_ids::*;
use super::types::*;
use serde_json::{Map, Value, json};
use tandem_protocol::engine::{
    DocumentAlgorithm, DocumentPermissions, PermissionName, SecurityMode,
};

/// What the dialog says when "Require a password" is chosen with no password to open.
pub(crate) const NEEDS_OPEN_PASSWORD: &str =
    "Type a password in \"Password to open\". The password to change these settings is optional.";

/// Round 2 (pages#7): an encrypted file's own `/P` flags (None for a file without
/// `/Encrypt`, or one that cannot be read here). Read without a password: `/P` is not
/// encrypted. Only a file that names `/Encrypt` is parsed a second time.
pub(crate) fn file_permissions_of(bytes: &[u8]) -> Option<DocumentPermissions> {
    if !bytes.windows(8).any(|w| w == b"/Encrypt") {
        return None;
    }
    let doc = tandem_pdf_engine::document::Document::open(
        bytes,
        tandem_pdf_engine::document::DocumentLimits::default(),
    )
    .ok()?;
    let bits = tandem_pdf_engine::app::info::permission_bits(&doc)?;
    let has = |bit: u32| bits & bit == bit;
    Some(DocumentPermissions {
        print: has(4),
        edit: has(8),
        copy: has(16),
        annotate: has(32),
        form: has(256),
    })
}

/// What people may not do with a protected file: "print or copy text".
fn withheld(granted: &[PermissionName]) -> Vec<&'static str> {
    [
        (PermissionName::Print, "print"),
        (PermissionName::Copy, "copy text"),
        (PermissionName::Edit, "change pages"),
        (PermissionName::Annotate, "add comments"),
        (PermissionName::Form, "fill in forms"),
    ]
    .into_iter()
    .filter(|(name, _)| !granted.contains(name))
    .map(|(_, words)| words)
    .collect()
}

/// The security dialog's state line (walkthrough pages#7): how the file is protected now,
/// read from the file, and what a pending change will do when the document is saved.
/// `password_to_open`: opening it asks for a password (review fix 5: a file encrypted with an
/// empty open password opens without one, though it may still withhold printing or copying).
pub(crate) fn security_words(
    info: &tandem_protocol::engine::DocumentInfo,
    password_to_open: bool,
    owner: bool,
    pending: Option<&tandem_protocol::engine::SecurityState>,
    file: Option<&DocumentPermissions>,
) -> String {
    let mut text = if !info.encrypted {
        "Right now anyone can open this file.".to_string()
    } else {
        let weak = matches!(
            info.algorithm,
            DocumentAlgorithm::Rc440 | DocumentAlgorithm::Rc4128
        );
        let weak = if weak {
            ", with old, weak protection"
        } else {
            ""
        };
        let mut t = if password_to_open {
            format!("Right now this file needs a password to open{weak}.")
        } else {
            format!(
                "Right now anyone can open this file without a password, but it is encrypted{weak}."
            )
        };
        let p = file.unwrap_or(&info.permissions);
        let names: Vec<PermissionName> = [
            (p.print, PermissionName::Print),
            (p.copy, PermissionName::Copy),
            (p.edit, PermissionName::Edit),
            (p.annotate, PermissionName::Annotate),
            (p.form, PermissionName::Form),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, name)| name)
        .collect();
        let off = withheld(&names);
        if !off.is_empty() {
            t.push_str(&format!(
                " People who open it can't {}.",
                crate::messages::listing(&off).replace(" and ", " or ")
            ));
        }
        if owner {
            t.push_str(" You opened it with the password that can change these settings.");
        }
        t
    };
    if let Some(next) = pending {
        match next.mode {
            SecurityMode::None => text.push_str(" When you save, it will open without a password."),
            SecurityMode::Aes256 => {
                let off = withheld(&next.permissions);
                text.push_str(" When you save, it will need a password to open");
                if !off.is_empty() {
                    text.push_str(&format!(
                        ", and people who open it won't be able to {}",
                        crate::messages::listing(&off).replace(" and ", " or ")
                    ));
                }
                text.push('.');
            }
            _ => {}
        }
    }
    text
}

/// engine-core.ts `permissionNames`: the granted flags in the fixed `PERMISSION_BITS` order.
pub(crate) fn permission_names(permissions: &DocumentPermissions) -> Vec<&'static str> {
    [
        ("print", permissions.print),
        ("edit", permissions.edit),
        ("copy", permissions.copy),
        ("annotate", permissions.annotate),
        ("form", permissions.form),
    ]
    .into_iter()
    .filter_map(|(name, granted)| granted.then_some(name))
    .collect()
}

/// The JS TypeError a property read on `null` throws (`parsed.user` of `JSON.parse("null")`).
fn null_read(key: &str) -> String {
    format!("Cannot read properties of null (reading '{key}')")
}

/// R:10696 `parseSecurityPayload`: the smoke's `--edit=encrypt` payload
/// `{"user":…,"owner":…,"permissions":[…]}` as the `security-set` fields; Err is the
/// exception's message.
pub(crate) fn parse_security_payload(
    contents: Option<&str>,
) -> Result<Option<Map<String, Value>>, String> {
    let Some(contents) = contents.filter(|c| !c.is_empty()) else {
        return Ok(None);
    };
    let parsed: Value = serde_json::from_str(contents).map_err(|e| e.to_string())?;
    if parsed.is_null() {
        return Err(null_read("user"));
    }
    let mut out = Map::new();
    out.insert("mode".into(), json!("aes-256"));
    // `undefined` members are dropped by the worker's structured clone, so they are absent here.
    if let Some(user) = parsed.get("user") {
        out.insert("userPassword".into(), user.clone());
    }
    if let Some(owner) = parsed.get("owner") {
        out.insert("ownerPassword".into(), owner.clone());
    }
    let mut permissions = Map::new();
    if let Some(Value::Array(names)) = parsed.get("permissions") {
        for name in names {
            permissions.insert(super::cert_sign::js_string(Some(name)), json!(true));
        }
    }
    out.insert("permissions".into(), Value::Object(permissions));
    Ok(Some(out))
}

impl Session {
    /// R:10596: Organize ▸ "Security…" and File ▸ Security…. Opened anyway when the document
    /// cannot change its security, with every control off and the reason in its status.
    pub fn open_security_dialog(&mut self) {
        let Some(doc) = &self.doc else {
            return;
        };
        let info = doc.info.clone();
        let allowed = self.can_change_security();
        let current = self.security_state.clone();
        // Walkthrough pages#7: the state read from the file, and what a pending Apply will do
        // at the next save, in plain words.
        // Opening asked for a password (the user or owner one), or a save here gave the file
        // one: Apply always sets a non-empty open password.
        let saved_aes = current
            .as_ref()
            .is_some_and(|c| c.mode == SecurityMode::Aes256 && !c.pending);
        let password_to_open = self.auth.as_str() != "none" || info.needs_password || saved_aes;
        let text = security_words(
            &info,
            password_to_open,
            self.auth.as_str() == "owner",
            current.as_ref().filter(|c| c.pending),
            self.file_permissions.as_ref().filter(|_| info.encrypted),
        );
        self.dom.set_text(SECURITY_CURRENT_EL, text);
        let none = current
            .as_ref()
            .is_some_and(|c| c.mode == SecurityMode::None);
        self.dom.set_checked(SECURITY_MODE_NONE_INPUT, none);
        self.dom.set_checked(SECURITY_MODE_AES_INPUT, !none);
        self.dom.set_value(SECURITY_USER_INPUT, "");
        self.dom.set_value(SECURITY_OWNER_INPUT, "");
        let granted: Vec<String> = match &current {
            Some(c) if c.mode == SecurityMode::Aes256 => c
                .permissions
                .iter()
                .map(|p| p.as_str().to_string())
                .collect(),
            // The boxes show what the file itself allows, not what the owner password allows.
            _ => permission_names(
                self.file_permissions
                    .as_ref()
                    .filter(|_| info.encrypted)
                    .unwrap_or(&info.permissions),
            )
            .into_iter()
            .map(str::to_string)
            .collect(),
        };
        for (name, id) in PERMISSION_BOXES {
            self.dom.set_checked(id, granted.iter().any(|g| g == name));
        }
        let status = if allowed {
            String::new()
        } else {
            self.security_refusal()
        };
        self.set_status_line(
            SECURITY_STATUS_EL,
            status,
            if allowed { "" } else { "info" },
        );
        self.dom.set_disabled(SECURITY_APPLY_BUTTON, !allowed);
        self.sync_security_mode();
        self.dom.set_open(SECURITY_DIALOG, true);
    }

    /// R:10619: AES-256 needs a password pair; "No encryption" does not (also the radios'
    /// `change` listener).
    pub fn sync_security_mode(&mut self) {
        let allowed = self.can_change_security();
        let aes = self.dom.checked(SECURITY_MODE_AES_INPUT);
        for id in [SECURITY_MODE_NONE_INPUT, SECURITY_MODE_AES_INPUT] {
            self.dom.set_disabled(id, !allowed);
        }
        for (_, id) in PERMISSION_BOXES {
            self.dom.set_disabled(id, !allowed);
        }
        self.dom.set_disabled(SECURITY_USER_INPUT, !allowed || !aes);
        self.dom
            .set_disabled(SECURITY_OWNER_INPUT, !allowed || !aes);
        self.dom.set_disabled(SECURITY_APPLY_BUTTON, !allowed);
    }

    /// R:10633 "Apply": one `security-set`; the engine stores the pending setting and marks
    /// the document dirty. The change lands in the file on the next save; it is not journaled.
    pub fn apply_security_dialog(&mut self) {
        if self.doc.is_none() {
            return;
        }
        let aes = self.dom.checked(SECURITY_MODE_AES_INPUT);
        let mut permissions = Map::new();
        for (name, id) in PERMISSION_BOXES {
            permissions.insert(name.into(), json!(self.dom.checked(id)));
        }
        let mut message = json!({ "op": "security-set", "mode": if aes { "aes-256" } else { "none" }, "permissions": permissions });
        if aes {
            // Re-check 10m: the engine refuses an empty open password; say so in plain words
            // before asking, and keep what was typed.
            if self.dom.value(SECURITY_USER_INPUT).is_empty() {
                self.set_status_line(SECURITY_STATUS_EL, NEEDS_OPEN_PASSWORD, "error");
                return;
            }
            message["userPassword"] = json!(self.dom.value(SECURITY_USER_INPUT));
            message["ownerPassword"] = json!(self.dom.value(SECURITY_OWNER_INPUT));
        }
        let reply = match self.request(message, vec![]) {
            Ok(reply) => reply,
            Err(err) => {
                // A refusal keeps the fields so the person can correct them; the dialog shows
                // the catalogue's plain words, never the engine's.
                let text = crate::messages::user_text(&self.message_of(&err));
                self.set_status_line(SECURITY_STATUS_EL, text, "error");
                return;
            }
        };
        // The request is the only place the passwords live from here on.
        self.dom.set_value(SECURITY_USER_INPUT, "");
        self.dom.set_value(SECURITY_OWNER_INPUT, "");
        self.apply_security_reply(&reply.value);
        self.close_security_dialog();
        self.announce("The password protection changes when you save");
        self.log_edit();
    }

    /// `securityDialog.close()` with its `close` listener (R:10985).
    pub fn close_security_dialog(&mut self) {
        self.dom.set_open(SECURITY_DIALOG, false);
        self.dom.set_value(SECURITY_USER_INPUT, "");
        self.dom.set_value(SECURITY_OWNER_INPUT, "");
        self.set_status_line(SECURITY_STATUS_EL, "", "");
    }

    /// R:10659: the edit-state slice a `security-set` reply carries, mirrored like a mutation.
    pub fn apply_security_reply(&mut self, reply: &Value) {
        if let Some(security) = reply.get("security").and_then(super::parse_reply) {
            self.security_state = Some(security);
        }
        if let Some(revision) = reply.get("revision").and_then(Value::as_i64) {
            self.revision = revision;
        }
        if let Some(dirty) = reply.get("dirty").and_then(Value::as_bool) {
            self.dirty = dirty;
        }
        if let Some(can_undo) = reply.get("canUndo").and_then(Value::as_bool) {
            self.can_undo = can_undo;
        }
        if let Some(can_redo) = reply.get("canRedo").and_then(Value::as_bool) {
            self.can_redo = can_redo;
        }
        self.sync_dirty();
        self.sync_controls();
        if let Some(info) = self.doc.as_ref().map(|doc| doc.info.clone()) {
            self.render_banner(&info);
        }
        // security-set moves the revision without a journal step: rescan as `applyEdit` does,
        // so text edits and replace previews name the new one.
        self.refresh_content_targets(self.ui_mode == UiMode::Edit);
    }

    /// R:10678: after a write in place landed, the file on disk has the pending security and
    /// the banner must stop saying it will change. Save a copy leaves the change pending.
    pub fn mark_security_saved(&mut self) {
        let Some(state) = self.security_state.as_mut() else {
            return;
        };
        state.pending = false;
        let state = state.clone();
        let Some(doc) = self.doc.as_mut() else {
            return;
        };
        match state.mode {
            SecurityMode::None => {
                self.file_permissions = None;
                doc.info.encrypted = false;
                doc.info.needs_password = false;
                doc.info.algorithm = DocumentAlgorithm::None;
                doc.info.permissions = DocumentPermissions {
                    print: true,
                    copy: true,
                    edit: true,
                    annotate: true,
                    form: true,
                };
            }
            SecurityMode::Aes256 => {
                let has = |name: PermissionName| state.permissions.contains(&name);
                doc.info.permissions = DocumentPermissions {
                    print: has(PermissionName::Print),
                    copy: has(PermissionName::Copy),
                    edit: has(PermissionName::Edit),
                    annotate: has(PermissionName::Annotate),
                    form: has(PermissionName::Form),
                };
                doc.info.encrypted = true;
                doc.info.needs_password = false;
                doc.info.algorithm = DocumentAlgorithm::Aes256;
                self.file_permissions = Some(doc.info.permissions.clone());
            }
            _ => {}
        }
        let info = doc.info.clone();
        self.render_banner(&info);
    }

    /// R:10710 `--edit=encrypt` with `--new-passwords`: apply AES-256, then save a copy.
    pub fn smoke_encrypt(&mut self, contents: Option<&str>) {
        let setting = match parse_security_payload(contents) {
            Ok(Some(setting)) => setting,
            Ok(None) => return,
            Err(message) => {
                self.show_error(&format!("encrypt: {message}"));
                return;
            }
        };
        let mut message = json!({ "op": "security-set" });
        for (key, value) in setting {
            message[key] = value;
        }
        match self.request(message, vec![]) {
            Ok(reply) => self.apply_security_reply(&reply.value),
            Err(err) => {
                let text = self.message_of(&err);
                self.show_error(&text);
                return;
            }
        }
        self.log_edit();
        self.save_as();
    }

    /// R:10733 `--edit=encrypt-keep` [`{password, note}` or just the password]: unlock if
    /// needed, add one note, then save a copy with the encryption preserved.
    pub fn smoke_encrypt_keep(&mut self, contents: Option<&str>) {
        let mut password: Option<Value> = None;
        let mut note: Option<Value> = None;
        match contents {
            Some(text) if crate::js::js_trim(text).starts_with('{') => {
                match serde_json::from_str::<Value>(text) {
                    Ok(parsed) if parsed.is_null() => {
                        self.show_error(&format!("encrypt-keep: {}", null_read("password")));
                        return;
                    }
                    Ok(parsed) => {
                        password = parsed.get("password").cloned();
                        note = parsed.get("note").cloned();
                    }
                    Err(err) => {
                        self.show_error(&format!("encrypt-keep: {err}"));
                        return;
                    }
                }
            }
            Some(text) if !text.is_empty() => password = Some(Value::String(text.to_string())),
            _ => {}
        }
        // `if (password)`: a truthy value; smokePassword itself takes strings only.
        if let Some(Value::String(password)) = password.as_ref().filter(|p| truthy(p)) {
            self.smoke_password(Some(password));
        }
        if let Some(note) = note.filter(truthy)
            && let Some(doc) = &self.doc
        {
            let page = self.wanted_page.max(0) as usize;
            if let Some(size) = doc.pages.get(page).cloned() {
                let half = tandem_protocol::limits::NOTE_SIZE / 2.0;
                let text = super::cert_sign::js_string(Some(&note));
                self.add_note_at(
                    page as i64,
                    [size.width / 2.0 - half, size.height / 2.0 - half],
                    &text,
                    "Added note",
                );
            }
        }
        self.log_edit();
        self.save_as();
    }

    /// R:10756: the smoke's unlock path: the CLI password goes to the engine and nowhere else.
    pub fn smoke_password(&mut self, value: Option<&str>) {
        let Some(value) = value.filter(|v| !v.is_empty()) else {
            return;
        };
        if !self.doc.as_ref().is_some_and(|doc| doc.info.needs_password) {
            return;
        }
        let reply = match self.request(json!({ "op": "authenticate", "password": value }), vec![]) {
            Ok(reply) => reply,
            Err(err) => {
                let text = self.message_of(&err);
                self.show_error(&text);
                return;
            }
        };
        if reply.get("authenticated") != Some(&Value::Bool(true)) {
            self.show_error("The document password was not accepted.");
            return;
        }
        self.apply_unlocked(super::parse_reply_or_default(&reply.value));
    }
}

/// JS truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|v| v != 0.0 && !v.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dialog_reads_the_files_own_flags() {
        use tandem_pdf_engine::document::{Document, DocumentLimits};
        use tandem_pdf_engine::writer::{
            DocumentWriteLimits, Encryption, Entropy, WriteOptions, write_document,
        };
        let path = format!("{}/../../fixtures/hello.pdf", env!("CARGO_MANIFEST_DIR"));
        let plain = std::fs::read(path).unwrap();
        assert_eq!(file_permissions_of(&plain), None);
        let doc = Document::open(&plain, DocumentLimits::default()).unwrap();
        let options = WriteOptions {
            encryption: Encryption::Aes256 {
                user_password: b"",
                owner_password: b"o",
                permissions: 16 | 32,
            },
            entropy: Some(Entropy([7; 32])),
            ..WriteOptions::APP
        };
        let bytes = write_document(&doc, DocumentWriteLimits::default(), options).unwrap();
        let p = file_permissions_of(&bytes).unwrap();
        assert!(!p.print && p.copy && p.annotate && !p.edit && !p.form);
        // Opened with the owner password the engine grants everything; the boxes do not.
        let mut s = Session::new();
        s.open_document("locked.pdf", bytes, None, false);
        s.auth = AuthLevel::Owner;
        if let Some(doc) = s.doc.as_mut() {
            doc.info.permissions = DocumentPermissions {
                print: true,
                copy: true,
                edit: true,
                annotate: true,
                form: true,
            };
        }
        s.open_security_dialog();
        assert!(!s.dom.checked(PERMISSION_BOXES[0].1), "print stays off");
        let words = s.dom.text(SECURITY_CURRENT_EL);
        assert!(words.contains("can't print"), "{words}");
    }

    #[test]
    fn security_line_for_people() {
        use tandem_protocol::engine::{DocumentInfo, SecurityState};
        let mut info = DocumentInfo::default();
        assert_eq!(
            security_words(&info, false, false, None, None),
            "Right now anyone can open this file."
        );
        let next = SecurityState {
            mode: SecurityMode::Aes256,
            permissions: vec![PermissionName::Copy, PermissionName::Form],
            pending: true,
            ..SecurityState::default()
        };
        assert_eq!(
            security_words(&info, false, false, Some(&next), None),
            "Right now anyone can open this file. When you save, it will need a password to open, and people who open it won't be able to print, change pages or add comments."
        );
        info.encrypted = true;
        info.algorithm = DocumentAlgorithm::Aes256;
        info.permissions = DocumentPermissions {
            print: false,
            copy: true,
            edit: true,
            annotate: true,
            form: true,
        };
        assert_eq!(
            security_words(&info, true, true, None, None),
            "Right now this file needs a password to open. People who open it can't print. You opened it with the password that can change these settings."
        );
        // An empty open password: no prompt, but printing is still withheld.
        assert_eq!(
            security_words(&info, false, false, None, None),
            "Right now anyone can open this file without a password, but it is encrypted. People who open it can't print."
        );
    }

    #[test]
    fn payload_parsing() {
        assert_eq!(parse_security_payload(None), Ok(None));
        assert_eq!(parse_security_payload(Some("")), Ok(None));
        let parsed = parse_security_payload(Some(
            r#"{"user":"u","owner":"o","permissions":["print","copy"]}"#,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(parsed["mode"], json!("aes-256"));
        assert_eq!(parsed["userPassword"], json!("u"));
        assert_eq!(parsed["permissions"], json!({"print": true, "copy": true}));
        assert_eq!(parse_security_payload(Some("null")), Err(null_read("user")));
        assert!(parse_security_payload(Some("{")).is_err());
        let bare = parse_security_payload(Some("5")).unwrap().unwrap();
        assert!(bare.get("userPassword").is_none());
    }

    #[test]
    fn permission_order() {
        let all = DocumentPermissions {
            print: true,
            copy: true,
            edit: true,
            annotate: true,
            form: true,
        };
        assert_eq!(
            permission_names(&all),
            ["print", "edit", "copy", "annotate", "form"]
        );
    }

    #[test]
    fn require_a_password_without_one_says_what_to_fill_in() {
        // Re-check 10m: only the owner password filled: plain words, nothing cleared, no request.
        let path = format!("{}/../../fixtures/hello.pdf", env!("CARGO_MANIFEST_DIR"));
        let mut s = Session::new();
        s.open_document("hello.pdf", std::fs::read(path).unwrap(), None, false);
        s.open_security_dialog();
        s.dom.set_checked(SECURITY_MODE_NONE_INPUT, false);
        s.dom.set_checked(SECURITY_MODE_AES_INPUT, true);
        s.dom.set_value(SECURITY_OWNER_INPUT, "owner");
        s.apply_security_dialog();
        assert_eq!(s.dom.text(SECURITY_STATUS_EL), NEEDS_OPEN_PASSWORD);
        assert_eq!(s.dom.value(SECURITY_OWNER_INPUT), "owner");
        assert!(s.dom.is_open(SECURITY_DIALOG) && !s.dirty);
        // With an open password it is accepted and the fields are cleared.
        s.dom.set_value(SECURITY_USER_INPUT, "user");
        s.apply_security_dialog();
        assert!(!s.dom.is_open(SECURITY_DIALOG) && s.dirty);
        assert_eq!(s.dom.value(SECURITY_USER_INPUT), "");
        assert_eq!(s.dom.value(SECURITY_OWNER_INPUT), "");
    }

    #[test]
    fn encrypt_keep_parse_error() {
        let mut s = Session::new();
        s.smoke_encrypt_keep(Some("{oops"));
        assert!(s.notice_raw.starts_with("encrypt-keep: "));
    }
}
