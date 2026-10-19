//! Security (renderer.ts R:10590-10770): the passwords and permissions dialog, the pending
//! `security-set`, the smoke's encrypt / encrypt-keep / password paths. The dialog mirrors the
//! engine's security state and never keeps a password: the two fields are cleared on submit
//! and on close, and only the request carries them.

use super::Session;
use super::consts::{ALGORITHM_LABEL, lookup};
use super::dom_ids::*;
use super::types::*;
use serde_json::{Map, Value, json};
use tandem_protocol::engine::{
    DocumentAlgorithm, DocumentPermissions, PermissionName, SecurityMode,
};

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
        let text = if !info.encrypted {
            "Currently: no encryption".to_string()
        } else {
            let algorithm = info.algorithm.as_str();
            format!(
                "Currently: {}, opened with the {} password",
                lookup(&ALGORITHM_LABEL, algorithm).unwrap_or(algorithm),
                if self.auth.as_str() == "owner" {
                    "owner"
                } else {
                    "user"
                }
            )
        };
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
            _ => permission_names(&info.permissions)
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
        self.dom.set_text(SECURITY_STATUS_EL, status);
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
            message["userPassword"] = json!(self.dom.value(SECURITY_USER_INPUT));
            message["ownerPassword"] = json!(self.dom.value(SECURITY_OWNER_INPUT));
        }
        // The request is the only place the passwords live from here on.
        self.dom.set_value(SECURITY_USER_INPUT, "");
        self.dom.set_value(SECURITY_OWNER_INPUT, "");
        let reply = match self.request(message, vec![]) {
            Ok(reply) => reply,
            Err(err) => {
                let text = self.message_of(&err);
                self.dom.set_text(SECURITY_STATUS_EL, text);
                return;
            }
        };
        self.apply_security_reply(&reply.value);
        self.close_security_dialog();
        self.announce("Security will change when you save");
        self.log_edit();
    }

    /// `securityDialog.close()` with its `close` listener (R:10985).
    pub fn close_security_dialog(&mut self) {
        self.dom.set_open(SECURITY_DIALOG, false);
        self.dom.set_value(SECURITY_USER_INPUT, "");
        self.dom.set_value(SECURITY_OWNER_INPUT, "");
        self.dom.set_text(SECURITY_STATUS_EL, "");
    }

    /// R:10659: the edit-state slice a `security-set` reply carries, mirrored like a mutation.
    pub fn apply_security_reply(&mut self, reply: &Value) {
        if let Some(security) = reply
            .get("security")
            .and_then(|s| serde_json::from_value(s.clone()).ok())
        {
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
        self.apply_unlocked(serde_json::from_value(reply.value).unwrap_or_default());
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
    fn encrypt_keep_parse_error() {
        let mut s = Session::new();
        s.smoke_encrypt_keep(Some("{oops"));
        assert!(s.notice_raw.starts_with("encrypt-keep: "));
    }
}
