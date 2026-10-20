// SPDX-License-Identifier: MIT
//! `unsigned-copy`: the open document's bytes without its signatures, so a
//! signed (read-only) file can be edited as a copy. Nothing else changes:
//! form values, annotations and pages stay as they are.
//!
//! Every signed signature field (and the widgets under it) loses its value
//! (/V, the signature dictionary), its /Lock and its appearance, and stays
//! as an empty signature field the copy can be signed in again. The catalog
//! loses /Perms (DocMDP certification, usage rights) and /DSS (validation
//! data). What that in-place pass cannot reach (a signed field written
//! directly into an array) goes as redaction removes it
//! (`sanitize::remove_signatures`). The copy is one full revision, written
//! with the save's security, and is checked to read as unsigned.
//!
//! The session's document and journal are not changed.
use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::app::edit::{Builder, Edit, Place};
use crate::app::ops::core::save_encryption;
use crate::app::ops::redact::{holds_signature, reopen};
use crate::app::redact::sanitize::{remove_signatures, signed_widget};
use crate::app::{
    AppError, AppSession, Buffers, MAX_SAVE_BYTES, SecuritySetting, info, overlay, reply, save_tree,
};
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};
use crate::serialize::SerializeError;
use crate::writer::{DocumentWriteError, Entropy, WriteOptions, write_document};

const OP: &str = "unsigned-copy";

fn tagged(reason: &str, message: impl Into<String>) -> AppError {
    AppError::reason(reason, message)
}

/// A field or widget dictionary (not a signature value, which has neither).
fn field_like(doc: &Document<'_>, dict: &Dictionary) -> bool {
    dict.get(b"FT".as_slice()).is_some()
        || dict.get(b"T".as_slice()).is_some()
        || doc.dict_get(dict, b"Subtype").as_name() == Some(b"Widget")
}

/// What a signed field or its widget sheds: the signature, the lock it
/// applied and the look of being signed.
fn unsign(dict: &mut Dictionary) {
    for key in [b"V".as_slice(), b"Lock", b"AP"] {
        dict.remove(key);
    }
}

/// The indirect fields and widgets of signed signature fields: every
/// object whose own or inherited /FT is /Sig with a /V on the way up.
fn signed_nodes(doc: &Document<'_>) -> BTreeSet<ObjectId> {
    (doc.objects())
        .filter(|o| {
            o.value
                .as_dict()
                .is_some_and(|d| field_like(doc, d) && signed_widget(doc, d))
        })
        .map(|o| o.id)
        .collect()
}

/// Clears the signatures of `target` in place (see the module comment);
/// returns how many signed fields there were.
pub fn unsign_document(target: &mut Builder) -> Result<usize, AppError> {
    let failed = |e: crate::document::DocumentError| AppError::plain(format!("{OP}: {e}"));
    let nodes = signed_nodes(target.document());
    let fields = info::detect(target.document(), false, false).signatures;
    for id in nodes {
        target
            .update(id, |value| {
                if let Some(dict) = value.as_dict_mut() {
                    unsign(dict);
                    // Direct widgets under the field.
                    if let Some(Object::Array(kids)) = dict.get_mut(b"Kids".as_slice()) {
                        for kid in kids.iter_mut() {
                            if let Some(kid) = kid.as_dict_mut() {
                                unsign(kid);
                            }
                        }
                    }
                }
            })
            .map_err(failed)?;
    }
    let root = target.document().root();
    Place::object(root)
        .update(target, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.remove(b"Perms".as_slice());
                dict.remove(b"DSS".as_slice());
            }
        })
        .map_err(failed)?;
    // Whatever is still signed (and /SigFlags) goes as redaction removes it.
    remove_signatures(target, OP)?;
    Ok(fields)
}

/// engine op `unsigned-copy`: `{bytes, signatures}`, the copy and how many
/// signed fields it no longer has.
pub fn unsigned_copy(
    session: &mut AppSession,
    request: &Value,
    buffers: &mut Buffers,
) -> Result<Value, AppError> {
    let Some(app) = &session.doc else {
        return Err(tagged("no-document", format!("{OP}: no document open")));
    };
    if session.locked {
        return Err(tagged(
            "encrypted",
            format!("{OP}: this document is encrypted; enter its password first"),
        ));
    }
    if !(app.info(false).signed() || holds_signature(app.document())) {
        return Err(tagged(
            "not-signed",
            format!("{OP}: the document has no signatures"),
        ));
    }
    if session.source_bytes > MAX_SAVE_BYTES {
        return Err(tagged(
            "too-large",
            format!(
                "{OP}: {} bytes is over the {MAX_SAVE_BYTES} byte save ceiling",
                session.source_bytes
            ),
        ));
    }
    let revision = app.document().security().map(|s| s.revision);
    let mut builder = Builder::from_document(app.document().clone());
    let signatures = unsign_document(&mut builder)?;
    let unsigned = builder.into_document();
    let mut limits = overlay::limits();
    limits.syntax.max_output_bytes = MAX_SAVE_BYTES;
    let entropy = (buffers.input(request.get("entropy")))
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .map(Entropy);
    let options = WriteOptions {
        encryption: save_encryption(session),
        entropy,
        ..WriteOptions::APP
    };
    let written = save_tree::for_save(&unsigned)
        .map_err(|error| tagged("invalid", format!("{OP}: {error}")))?;
    let bytes = match write_document(&written, limits, options) {
        Ok(bytes) if bytes.len() <= MAX_SAVE_BYTES => bytes,
        Ok(_) | Err(DocumentWriteError::Serialize(SerializeError::OutputLimit)) => {
            return Err(tagged(
                "too-large",
                format!("{OP}: the copy would exceed the {MAX_SAVE_BYTES} byte save ceiling"),
            ));
        }
        Err(error) => return Err(tagged("invalid", format!("{OP}: {error}"))),
    };
    // The copy must open and read as unsigned, with no signature dictionary,
    // time stamp, certification or validation data left.
    let password = match &session.security {
        Some(SecuritySetting::Aes256 { user_password, .. }) => Some(user_password.clone()),
        _ => session.password.clone(),
    };
    let copy = reopen(&bytes, password.as_deref(), revision)
        .ok_or_else(|| tagged("invalid", format!("{OP}: the copy could not be reopened")))?;
    if info::detect(&copy, false, false).signed() || holds_signature(&copy) {
        return Err(tagged(
            "invalid",
            format!("{OP}: the copy still holds a signature"),
        ));
    }
    Ok(reply(
        request,
        json!({"bytes": buffers.put(bytes), "signatures": signatures}),
    ))
}
