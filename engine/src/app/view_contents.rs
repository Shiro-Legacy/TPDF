// SPDX-License-Identifier: AGPL-3.0-or-later
//! Page content streams the view leaves out: those encoded with an image
//! filter (JBIG2, JPX, DCT, CCITT fax). They decode to samples, not content
//! (MuPDF parses the samples and finds no operators), and hayro decodes each
//! whole before anything is drawn: three JBIG2 streams declaring 100,000 ×
//! 100,000 pixel pages (pdf.js bomb_giant.pdf, 123 KB) held about 1.5 GB.
//! The view's private update writes each such page without them; the
//! content walk refuses any other content stream so encoded
//! (`view::walk`).
//!
//! Entries of a /Contents array that are not streams (a name, a number, a
//! reference to a missing or non-stream object) are left out too: MuPDF
//! skips them, while hayro stops reading the array at the first one and
//! drew nothing of the page after it (M10 fuzz, `design/m10-fuzz.md`).
use std::collections::BTreeMap;

use crate::document::Document;
use crate::object::{Object, ObjectId};

use super::info;
use crate::view::is_image_filter;

/// Redefines in `fixes` every page written in its own object whose /Contents
/// names a stream encoded with an image filter, or lists an entry that is
/// no stream, without those entries.
pub(crate) fn drop_sample_streams(doc: &Document<'_>, fixes: &mut BTreeMap<ObjectId, Object>) {
    for page in info::pages(doc) {
        let Some(id) = page.id else {
            continue;
        };
        let current = fixes
            .get(&id)
            .and_then(Object::as_dict)
            .unwrap_or(page.dict);
        let Some(contents) = current.get(b"Contents".as_slice()) else {
            continue;
        };
        let items: Vec<Object> = match doc.resolve(contents) {
            Object::Array(items) => items.clone(),
            _ => vec![contents.clone()],
        };
        let kept: Vec<Object> = items
            .iter()
            .filter(|item| is_stream(doc, item) && !encodes_samples(doc, item))
            .cloned()
            .collect();
        if kept.len() == items.len() {
            continue;
        }
        let mut dict = current.clone();
        dict.insert(b"Contents".to_vec(), Object::Array(kept));
        fixes.insert(id, Object::Dictionary(dict));
    }
}

/// Whether `item` is a reference to a stream.
fn is_stream(doc: &Document<'_>, item: &Object) -> bool {
    matches!(item, Object::Reference(id) if doc.stream_dict(*id).is_ok())
}

/// Whether `item` is a reference to a stream whose filters include one that
/// decodes image samples.
pub(crate) fn encodes_samples(doc: &Document<'_>, item: &Object) -> bool {
    let Object::Reference(id) = item else {
        return false;
    };
    let Ok(dict) = doc.stream_dict(*id) else {
        return false;
    };
    let filters = doc.dict_get(dict, b"Filter");
    let names: Vec<&[u8]> = match filters {
        Object::Array(items) => items
            .iter()
            .filter_map(|f| doc.resolve(f).as_name())
            .collect(),
        other => other.as_name().into_iter().collect(),
    };
    names.iter().any(|name| is_image_filter(name))
}
